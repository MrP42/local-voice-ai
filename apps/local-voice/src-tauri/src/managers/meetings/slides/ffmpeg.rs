//! D1: der ffmpeg-Kindprozess der Folienerkennung.
//!
//! Zwei Aufrufe, beide ohne Fenster (`CREATE_NO_WINDOW`), beide in einem
//! Job-Objekt (`process_guard`: CPU-Deckel, niedrige Prioritaet,
//! `KILL_ON_JOB_CLOSE`) und beide nur ueber ihr eigenes Handle beendbar, nie ueber
//! den Programmnamen:
//!
//! - [`sample_video`]: Abtastung mit 1 fps als Rohbilder ueber die Pipe (stdout),
//!   gehasht in einem Lesethread. Dazu liest ein zweiter Thread die Fehlerausgabe
//!   (Kopf mit Dauer und Streams, Ende fuer Fehlertexte); ohne ihn blockiert ein
//!   voller Pipe-Puffer den Prozess.
//! - [`extract_frame`]: ein Bild (Vollbild, optional Vorschau) an einer Stelle.
//!
//! Pause ist Gegendruck, kein Anhalten: haelt der Aufrufer ueber [`Flow`] das Lesen
//! an, fuellt sich die Pipe (64 kB, wenige Bilder) und ffmpeg schlaeft in `write`.
//! Der Prozess wird nie eingefroren.
//!
//! ffmpeg ist wie bisher das des Nutzers (PATH, siehe `media.rs`); der App liegt
//! keines bei.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::{dhash64, mean_luma, SlideDetectConfig, SlideError, FRAME_BYTES, SAMPLE_H, SAMPLE_W};
use crate::media::{run_child_cancellable, ChildEnd};

/// Haelt ffmpeg den Rechner nicht auf: Dekoder-Threads (Spike: 4 statt aller kostet
/// 1-4 s mehr und ist fuer den Hintergrundbetrieb richtig). In den Tests 2: die
/// Testlaeufe teilen sich den Rechner mit anderer Arbeit.
const DECODE_THREADS: &str = if cfg!(test) { "2" } else { "4" };
/// Breite der Vorschau (Hoehe aus dem Seitenverhaeltnis).
pub const THUMB_WIDTH: u32 = 320;
/// Wie viel vom Anfang der Fehlerausgabe fuer Dauer und Streams gelesen wird.
const HEAD_BYTES: usize = 32 * 1024;
/// Wie viel vom Ende fuer den Fehlertext behalten wird.
const TAIL_BYTES: usize = 4 * 1024;
/// So oft prueft der wartende Thread, ob der Prozess fertig ist oder abgebrochen wird.
const POLL: Duration = Duration::from_millis(20);
/// Windows: der Kindprozess bekommt kein Konsolenfenster (die App hat keines, ein
/// sichtbares Fenster je Aufruf waere ein Aufblitzen).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Endung der halbfertigen Bilder: erst wenn ffmpeg fertig ist, wird umbenannt.
pub const PART_MARKER: &str = ".part.jpg";

// ---------------------------------------------------------------------------
// Kopf der Fehlerausgabe (rein)
// ---------------------------------------------------------------------------

/// Was ffmpeg ueber die Eingabe verraet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaInfo {
    /// Dauer, soweit der Container sie kennt (`Duration: N/A` bei Aufnahmen ohne).
    pub duration_ms: Option<u64>,
    /// Gibt es eine echte Videospur? Ein Titelbild (`attached pic`) zaehlt nicht.
    pub has_video: bool,
}

/// `HH:MM:SS.xx` -> ms. `N/A` und alles Unlesbare -> `None`.
pub fn parse_duration_ms(text: &str) -> Option<u64> {
    let mut parts = text.trim().split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let hours: u64 = h.trim().parse().ok()?;
    let minutes: u64 = m.trim().parse().ok()?;
    let seconds: f64 = s.trim().parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 || minutes > 59 {
        return None;
    }
    Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as u64)
}

/// Dauer und Videospur aus dem Kopf der Fehlerausgabe (`-loglevel info`).
pub fn parse_media_header(text: &str) -> MediaInfo {
    let duration_ms = text.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("Duration:")?;
        parse_duration_ms(rest.split(',').next()?)
    });
    let has_video = text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("Stream #") && line.contains(": Video:") && !line.contains("attached pic")
    });
    MediaInfo {
        duration_ms,
        has_video,
    }
}

/// Spricht der Text fuer einen vollen Datentraeger?
pub fn is_disk_full_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("no space left on device")
        || lower.contains("not enough space on the disk")
        || lower.contains("disk full")
        || lower.contains("there is not enough space")
}

/// Ist der Betriebssystemfehler "Datentraeger voll"? (Windows: ERROR_HANDLE_DISK_FULL
/// 39 und ERROR_DISK_FULL 112; Unix: ENOSPC 28.)
pub fn is_disk_full_io(e: &std::io::Error) -> bool {
    if cfg!(windows) {
        matches!(e.raw_os_error(), Some(39) | Some(112))
    } else {
        e.raw_os_error() == Some(28)
    }
}

/// Die letzten (hoechstens drei) nichtleeren Zeilen als eine Zeile.
fn tail_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" | ")
}

/// Welcher Fehler steckt hinter einem ffmpeg, das mit Fehler endete?
pub fn classify_failure(info: &MediaInfo, head: &str, tail: &str) -> SlideError {
    if !info.has_video && !head.is_empty() && head.contains("Input #0") {
        return SlideError::NoVideoStream;
    }
    if is_disk_full_text(tail) || is_disk_full_text(head) {
        return SlideError::DiskFull;
    }
    SlideError::Ffmpeg(tail_lines(if tail.trim().is_empty() { head } else { tail }))
}

fn spawn_error(e: std::io::Error) -> SlideError {
    if e.kind() == std::io::ErrorKind::NotFound {
        SlideError::FfmpegMissing
    } else {
        SlideError::Io(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Rohbilder lesen (ohne Prozess testbar)
// ---------------------------------------------------------------------------

/// Liest Graustufen-Rohbilder zu je [`FRAME_BYTES`] aus `reader` und uebergibt jedes
/// an `on_frame`; ein Puffer, egal wie lang der Strom ist. `on_frame` liefert
/// `false`, um aufzuhoeren. Ein angebrochenes letztes Bild (Prozess mitten im
/// Bild beendet) zaehlt nicht. Rueckgabe: Zahl der gelieferten Bilder.
pub fn read_frames<R: Read>(
    mut reader: R,
    mut on_frame: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<u64> {
    let mut buf = vec![0u8; FRAME_BYTES];
    let mut frames = 0u64;
    loop {
        let mut filled = 0;
        while filled < FRAME_BYTES {
            match reader.read(&mut buf[filled..]) {
                Ok(0) => return Ok(frames),
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        frames += 1;
        if !on_frame(&buf) {
            return Ok(frames);
        }
    }
}

// ---------------------------------------------------------------------------
// Abtastung
// ---------------------------------------------------------------------------

/// Was der Lesethread nach jedem Bild meldet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleTick {
    /// Laufende Nummer des Bildes (ab 0).
    pub index: u64,
    /// Zeit im Video (ms), aus Nummer und Abtastrate.
    pub ms: u64,
    /// Dauer des Videos, sobald ffmpeg sie genannt hat.
    pub duration_ms: Option<u64>,
}

/// Antwort des Aufrufers nach einem Bild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// Aufhoeren: der Prozess wird beendet, das Ergebnis ist [`SlideError::Cancelled`].
    Stop,
}

/// Ergebnis einer vollstaendigen Abtastung.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleResult {
    /// Hash und mittlere Helligkeit je Abtastung, zeitlich geordnet.
    pub samples: Vec<(u64, f32)>,
    pub info: MediaInfo,
}

/// Der Befehl der Abtastung: Rohbilder 160x90 Graustufen, `fps` je Sekunde, auf stdout.
pub fn sample_command(video: &Path, cfg: &SlideDetectConfig) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"])
        .args(["-threads", DECODE_THREADS])
        .arg("-i")
        .arg(video)
        .args(["-an", "-sn", "-dn", "-vf"])
        .arg(format!(
            "fps={},scale={SAMPLE_W}:{SAMPLE_H}:flags=area,format=gray",
            cfg.sample_fps
        ))
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"]);
    cmd
}

/// Der Haken, den der Lesethread nach jedem Bild ruft. `'static`, weil der Thread
/// den Aufruf ueberleben darf (siehe [`wait_for_streams`]).
pub type SampleHook = Arc<dyn Fn(SampleTick) -> Flow + Send + Sync>;

/// So lange darf der Prozess zu Ende sein, ohne dass seine Pipes enden, bevor
/// [`run_sampler`] mit dem Gelesenen weitermacht (ein fremder Prozess, der beim
/// Start versehentlich ein Pipe-Ende geerbt hat, haelt sie sonst offen).
const STREAM_GRACE: Duration = Duration::from_secs(3);

/// Tastet `video` ab. Siehe [`run_sampler`] fuer Abbruch und Pause.
pub fn sample_video(
    video: &Path,
    cfg: &SlideDetectConfig,
    cancel: &AtomicBool,
    hook: SampleHook,
) -> Result<SampleResult, SlideError> {
    run_sampler(sample_command(video, cfg), cfg, cancel, hook, None)
}

/// Zustand, den Lesethreads und wartender Thread teilen. Alles hinter Sperren
/// oder Atomics, damit die Threads den Aufruf ueberleben koennen.
#[derive(Default)]
struct Shared {
    samples: Mutex<Vec<(u64, f32)>>,
    head: Mutex<String>,
    tail: Mutex<String>,
    io_error: Mutex<Option<String>>,
    /// Dauer aus dem Kopf der Fehlerausgabe in ms, 0 = unbekannt.
    duration_ms: AtomicU64,
    reader_done: AtomicBool,
    stderr_done: AtomicBool,
    /// Der Lesethread steckt gerade im Haken (Pause): dann ist "keine Bewegung" kein Leck.
    in_hook: AtomicBool,
    /// Der Haken hat Stopp verlangt.
    stopped: AtomicBool,
    too_long: AtomicBool,
    /// Der Prozess soll beendet werden (Stopp im Haken oder zu viele Abtastungen).
    halt: AtomicBool,
    activity: Mutex<Option<Instant>>,
}

impl Shared {
    fn touch(&self) {
        *lock(&self.activity) = Some(Instant::now());
    }

    fn idle_for(&self) -> Duration {
        lock(&self.activity).map_or(Duration::ZERO, |t| t.elapsed())
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Setzt beim Verlassen (auch bei einer Panik im Haken) ein Merkzeichen: der
/// wartende Thread haengt nie an einem Thread, der schon weg ist.
struct DoneOnDrop<'a>(&'a AtomicBool);

impl Drop for DoneOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Liest die Fehlerausgabe bis zum Ende: Kopf (Dauer, Streams) und Ende. Meldet
/// die Dauer sofort, sobald sie im Kopf steht, damit der Fortschritt eine Groesse
/// bekommt, bevor das erste Bild kommt.
fn collect_stderr(mut pipe: impl Read, shared: &Shared) {
    let _done = DoneOnDrop(&shared.stderr_done);
    let mut head: Vec<u8> = Vec::new();
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = [0u8; 2048];
    let mut duration_known = false;
    while let Ok(n) = pipe.read(&mut buf) {
        if n == 0 {
            break;
        }
        shared.touch();
        if head.len() < HEAD_BYTES {
            let take = n.min(HEAD_BYTES - head.len());
            head.extend_from_slice(&buf[..take]);
            let text = String::from_utf8_lossy(&head).into_owned();
            if !duration_known {
                if let Some(ms) = parse_media_header(&text).duration_ms {
                    shared.duration_ms.store(ms, Ordering::Release);
                    duration_known = true;
                }
            }
            *lock(&shared.head) = text;
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > TAIL_BYTES {
            tail.drain(..tail.len() - TAIL_BYTES);
        }
        *lock(&shared.tail) = String::from_utf8_lossy(&tail).into_owned();
    }
}

/// Hasht jedes Bild aus `stdout`, legt es ab und ruft den Haken.
fn consume_frames(
    stdout: impl Read,
    shared: &Shared,
    hook: &SampleHook,
    fps: f64,
    max_samples: usize,
) {
    let _done = DoneOnDrop(&shared.reader_done);
    let result = read_frames(stdout, |frame| {
        let count = {
            let mut samples = lock(&shared.samples);
            samples.push((dhash64(frame, SAMPLE_W, SAMPLE_H), mean_luma(frame)));
            samples.len()
        };
        shared.touch();
        if count > max_samples {
            shared.too_long.store(true, Ordering::Release);
            shared.halt.store(true, Ordering::Release);
            return false;
        }
        let index = count as u64 - 1;
        let known = shared.duration_ms.load(Ordering::Acquire);
        let tick = SampleTick {
            index,
            ms: (index as f64 * 1000.0 / fps).round() as u64,
            duration_ms: (known > 0).then_some(known),
        };
        shared.in_hook.store(true, Ordering::Release);
        let flow = hook(tick);
        shared.in_hook.store(false, Ordering::Release);
        shared.touch();
        match flow {
            Flow::Continue => true,
            Flow::Stop => {
                shared.stopped.store(true, Ordering::Release);
                shared.halt.store(true, Ordering::Release);
                false
            }
        }
    });
    if let Err(e) = result {
        *lock(&shared.io_error) = Some(e.to_string());
    }
}

/// Wartet, bis beide Lesethreads fertig sind. Fertig sind sie, sobald ihre Pipes
/// enden. Haelt ein fremder Prozess ein Pipe-Ende offen (unter Windows erbt ein
/// gleichzeitig gestarteter Prozess gelegentlich fremde Handles), kaeme das Ende
/// nie: steht der Lesethread dann `grace` lang still und steckt nicht im Haken
/// (Pause), geht es mit dem Gelesenen weiter. `false` = so abgebrochen.
fn wait_for_streams(shared: &Shared, grace: Duration) -> bool {
    shared.touch();
    loop {
        if shared.reader_done.load(Ordering::Acquire) && shared.stderr_done.load(Ordering::Acquire)
        {
            return true;
        }
        if !shared.in_hook.load(Ordering::Acquire) && shared.idle_for() > grace {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Fuehrt `cmd` aus (ein ffmpeg, das Rohbilder 160x90 auf stdout schreibt), hasht
/// jedes Bild und ruft danach `hook`. Der Lesethread blockiert im `hook` (Pause):
/// dann liest niemand die Pipe, und ffmpeg wartet. `cancel` und ein `Stop` des
/// Hooks beenden den Prozess (Handle, nie Name; Job-Objekt) und ergeben
/// [`SlideError::Cancelled`]; ein gesetztes `cancel` VOR dem Start startet gar
/// nichts. `pid_out` bekommt die PID (Tests, Diagnose).
///
/// Nach dem Ende des Prozesses wird sein Job-Objekt geschlossen (alles, was er
/// gestartet hat, stirbt mit) und dann auf das Ende der Pipes gewartet, hoechstens
/// [`STREAM_GRACE`] ohne Bewegung (siehe [`wait_for_streams`]).
pub(crate) fn run_sampler(
    mut cmd: Command,
    cfg: &SlideDetectConfig,
    cancel: &AtomicBool,
    hook: SampleHook,
    pid_out: Option<&AtomicU32>,
) -> Result<SampleResult, SlideError> {
    run_sampler_with(
        cmd_prepare(&mut cmd),
        cfg,
        cancel,
        hook,
        pid_out,
        STREAM_GRACE,
    )
}

fn cmd_prepare(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn run_sampler_with(
    cmd: &mut Command,
    cfg: &SlideDetectConfig,
    cancel: &AtomicBool,
    hook: SampleHook,
    pid_out: Option<&AtomicU32>,
    grace: Duration,
) -> Result<SampleResult, SlideError> {
    if cancel.load(Ordering::Acquire) {
        return Err(SlideError::Cancelled);
    }
    let mut child = cmd.spawn().map_err(spawn_error)?;
    if let Some(out) = pid_out {
        out.store(child.id(), Ordering::Release);
    }
    // Ohne Speicherdeckel (Dekodieren braucht wenig), mit CPU-Deckel, niedriger
    // Prioritaet und KILL_ON_JOB_CLOSE: stirbt die App, stirbt ffmpeg mit.
    #[cfg(windows)]
    let job_guard = crate::process_guard::ProcessGuard::attach(
        &child,
        None,
        crate::process_guard::CPU_CAP_PERCENT,
    );
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let fps = if cfg.sample_fps.is_finite() && cfg.sample_fps > 0.0 {
        f64::from(cfg.sample_fps)
    } else {
        1.0
    };
    let max_samples = cfg.max_samples;

    let shared = Arc::new(Shared::default());
    shared.touch();
    {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("slides-stderr".into())
            .spawn(move || collect_stderr(stderr, &shared))
            .map_err(|e| SlideError::Io(e.to_string()))?;
    }
    {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("slides-reader".into())
            .spawn(move || consume_frames(stdout, &shared, &hook, fps, max_samples))
            .map_err(|e| SlideError::Io(e.to_string()))?;
    }

    // Warten, bis ffmpeg fertig ist; bei Abbruch oder Halt beenden (nur DIESER Prozess).
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if cancel.load(Ordering::Acquire) || shared.halt.load(Ordering::Acquire) {
                    let _ = child.kill();
                    break child.wait();
                }
                std::thread::sleep(POLL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e);
            }
        }
    };
    // Was ffmpeg gestartet hat, stirbt mit dem Job-Objekt; so bleibt kein Pipe-Ende offen.
    #[cfg(windows)]
    drop(job_guard);
    if !wait_for_streams(&shared, grace) {
        log::warn!("slides: Pipes von ffmpeg enden nicht (fremdes Handle?), mache mit dem Gelesenen weiter");
    }

    if cancel.load(Ordering::Acquire) || shared.stopped.load(Ordering::Acquire) {
        return Err(SlideError::Cancelled);
    }
    if shared.too_long.load(Ordering::Acquire) {
        return Err(SlideError::TooLong);
    }
    if let Some(e) = lock(&shared.io_error).take() {
        return Err(SlideError::Io(e));
    }
    let head = lock(&shared.head).clone();
    let tail = lock(&shared.tail).clone();
    let info = parse_media_header(&head);
    let status = status.map_err(|e| SlideError::Io(e.to_string()))?;
    if !status.success() {
        return Err(classify_failure(&info, &head, &tail));
    }
    let samples = std::mem::take(&mut *lock(&shared.samples));
    Ok(SampleResult { samples, info })
}

// ---------------------------------------------------------------------------
// Einzelbild
// ---------------------------------------------------------------------------

/// Das halbfertige Gegenstueck zu `final_path` (`0007.jpg` -> `0007.part.jpg`).
pub fn part_path(final_path: &Path) -> PathBuf {
    let stem = final_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "frame".to_string());
    final_path.with_file_name(format!("{stem}{PART_MARKER}"))
}

/// Der Befehl: ein Bild bei `at_ms` als JPEG in `full_part`, optional eine
/// Vorschau in `thumb_part`. `-ss` vor `-i` (schnelles, mit Neukodierung
/// bildgenaues Suchen).
pub fn extract_command(
    video: &Path,
    at_ms: u64,
    full_part: &Path,
    thumb_part: Option<&Path>,
) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
        .args(["-threads", DECODE_THREADS])
        .arg("-ss")
        .arg(format!("{:.3}", at_ms as f64 / 1000.0))
        .arg("-i")
        .arg(video)
        .args(["-an", "-sn", "-dn"])
        .args([
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-q:v",
            "3",
            "-f",
            "image2",
            "-update",
            "1",
        ])
        .arg(full_part);
    if let Some(thumb) = thumb_part {
        cmd.args(["-map", "0:v:0", "-frames:v", "1", "-vf"])
            .arg(format!("scale={THUMB_WIDTH}:-2"))
            .args(["-q:v", "5", "-f", "image2", "-update", "1"])
            .arg(thumb);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn remove_quietly(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Schreibt das Bild bei `at_ms` nach `full` (und die Vorschau nach `thumb`).
/// Zuerst unter `*.part.jpg`, erst wenn ffmpeg fertig ist und die Datei nicht
/// leer ist, wird umbenannt (atomar auf demselben Datentraeger): nach einem
/// Abbruch oder Absturz gibt es nie ein halbes `0007.jpg`. Bei jedem Fehler sind
/// die `*.part.jpg` weg.
pub fn extract_frame(
    video: &Path,
    at_ms: u64,
    full: &Path,
    thumb: Option<&Path>,
    cancel: &AtomicBool,
) -> Result<(), SlideError> {
    let full_part = part_path(full);
    let thumb_part = thumb.map(part_path);
    remove_quietly(&full_part);
    if let Some(p) = &thumb_part {
        remove_quietly(p);
    }
    let cleanup = || {
        remove_quietly(&full_part);
        if let Some(p) = &thumb_part {
            remove_quietly(p);
        }
    };
    let cmd = extract_command(video, at_ms, &full_part, thumb_part.as_deref());
    let end = match run_child_cancellable(cmd, cancel, None) {
        Ok(end) => end,
        Err(e) => {
            cleanup();
            return Err(spawn_error(e));
        }
    };
    match end {
        ChildEnd::Cancelled => {
            cleanup();
            return Err(SlideError::Cancelled);
        }
        ChildEnd::Finished {
            success: false,
            stderr_tail,
        } => {
            cleanup();
            return Err(if is_disk_full_text(&stderr_tail) {
                SlideError::DiskFull
            } else {
                SlideError::Ffmpeg(tail_lines(&stderr_tail))
            });
        }
        ChildEnd::Finished { success: true, .. } => {}
    }
    let non_empty = |p: &Path| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false);
    let outputs_ok = non_empty(&full_part) && thumb_part.as_deref().is_none_or(non_empty);
    if !outputs_ok {
        cleanup();
        // Hinter dem Ende des Videos (rep_ms nach der letzten Abtastung) liefert ffmpeg ohne Fehler nichts.
        return Err(SlideError::Ffmpeg("kein Bild an dieser Stelle".to_string()));
    }
    // Erst die Vorschau, dann das Vollbild: wer das Vollbild sieht, findet die Vorschau.
    if let (Some(from), Some(to)) = (&thumb_part, thumb) {
        if let Err(e) = std::fs::rename(from, to) {
            cleanup();
            return Err(e.into());
        }
    }
    if let Err(e) = std::fs::rename(&full_part, full) {
        cleanup();
        if let Some(to) = thumb {
            remove_quietly(to);
        }
        return Err(e.into());
    }
    Ok(())
}

/// Raeumt liegengebliebene `*.part.jpg` in `dir` (Absturz oder Abbruch beim Schreiben).
/// Rueckgabe: Zahl der entfernten Dateien. Ein fehlender Ordner ist kein Fehler.
pub fn clean_partial_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(PART_MARKER)
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use crate::managers::meetings::slides::test_support::process_alive;
    use crate::managers::meetings::slides::test_support::{ffmpeg_available, serial};
    use std::sync::atomic::AtomicUsize;

    // -- Kopf der Fehlerausgabe -------------------------------------------------

    const VIDEO_HEADER: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 't3.mp4':\n  Metadata:\n    major_brand     : isom\n  Duration: 00:10:00.04, start: 0.000000, bitrate: 367 kb/s\n  Stream #0:0[0x1](und): Video: h264 (Constrained Baseline) (avc1 / 0x31637661), yuv420p(progressive), 640x360, 25 fps\n  Stream #0:1[0x2](und): Audio: aac (LC), 48000 Hz, stereo\nStream mapping:\n";

    #[test]
    fn duration_and_video_stream_are_read_from_the_header() {
        let info = parse_media_header(VIDEO_HEADER);
        assert_eq!(info.duration_ms, Some(600_040));
        assert!(info.has_video);
        assert_eq!(parse_duration_ms("01:02:03.5"), Some(3_723_500));
        assert_eq!(parse_duration_ms("N/A"), None);
        assert_eq!(parse_duration_ms("00:61:00.00"), None);
        assert_eq!(parse_duration_ms("1:2"), None);
        assert_eq!(parse_duration_ms("00:00:00.00:00"), None);
    }

    #[test]
    fn an_audio_header_has_no_video() {
        let audio = "Input #0, mp3, from 'a.mp3':\n  Duration: 00:00:03.00, start: 0.025057, bitrate: 65 kb/s\n  Stream #0:0: Audio: mp3 (mp3float), 44100 Hz, mono, fltp, 64 kb/s\n";
        let info = parse_media_header(audio);
        assert!(!info.has_video);
        assert_eq!(info.duration_ms, Some(3_000));
        assert_eq!(
            parse_media_header("Duration: N/A, bitrate: N/A").duration_ms,
            None
        );
    }

    #[test]
    fn a_cover_art_stream_is_no_video() {
        let mp3 = "Input #0, mp3, from 'song.mp3':\n  Duration: 00:03:30.00, start: 0.0\n  Stream #0:0: Audio: mp3, 44100 Hz\n  Stream #0:1: Video: mjpeg (Baseline), yuvj420p, 500x500 [SAR 1:1 DAR 1:1], 90k tbr (attached pic)\n";
        assert!(
            !parse_media_header(mp3).has_video,
            "ein Titelbild ist kein Video"
        );
    }

    #[test]
    fn failures_are_classified_by_their_cause() {
        let audio = MediaInfo {
            duration_ms: Some(3_000),
            has_video: false,
        };
        let video = MediaInfo {
            duration_ms: Some(3_000),
            has_video: true,
        };
        let head = "Input #0, mp3, from 'a.mp3':\n  Stream #0:0: Audio: mp3\n";
        assert_eq!(
            classify_failure(&audio, head, "Error opening output files"),
            SlideError::NoVideoStream
        );
        assert_eq!(
            classify_failure(
                &video,
                VIDEO_HEADER,
                "av_interleaved_write_frame(): No space left on device"
            ),
            SlideError::DiskFull
        );
        assert_eq!(
            classify_failure(&video, VIDEO_HEADER, "a\n\nb\nc\nd\n"),
            SlideError::Ffmpeg("b | c | d".to_string())
        );
        // Ohne lesbaren Kopf (Datei nicht gefunden) ist es kein "ohne Video".
        assert!(matches!(
            classify_failure(
                &MediaInfo::default(),
                "",
                "x.mp4: No such file or directory"
            ),
            SlideError::Ffmpeg(_)
        ));
    }

    #[test]
    fn a_full_disk_is_recognised_in_text_and_error_code() {
        assert!(is_disk_full_text("Error writing: No space left on device"));
        assert!(is_disk_full_text("There is not enough space on the disk."));
        assert!(!is_disk_full_text(
            "Invalid data found when processing input"
        ));
        let full: &[i32] = if cfg!(windows) { &[39, 112] } else { &[28] };
        for &code in full {
            assert!(
                is_disk_full_io(&std::io::Error::from_raw_os_error(code)),
                "{code}"
            );
        }
        assert!(!is_disk_full_io(&std::io::Error::from_raw_os_error(5)));
        assert_eq!(
            SlideError::from(std::io::Error::from_raw_os_error(full[0])),
            SlideError::DiskFull
        );
        assert!(matches!(
            SlideError::from(std::io::Error::from_raw_os_error(5)),
            SlideError::Io(_)
        ));
    }

    // -- Befehle ------------------------------------------------------------------

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_sample_command_asks_for_small_gray_raw_frames_on_stdout() {
        let cfg = SlideDetectConfig {
            sample_fps: 0.5,
            ..SlideDetectConfig::default()
        };
        let a = args(&sample_command(
            Path::new("C:/Aufnahmen/Vortrag Größe.mp4"),
            &cfg,
        ));
        assert!(
            a.contains(&"-nostdin".to_string()),
            "ffmpeg darf nie auf Eingaben warten"
        );
        assert!(a.windows(2).any(|w| w == ["-threads", DECODE_THREADS]));
        assert!(a.contains(&"fps=0.5,scale=160:90:flags=area,format=gray".to_string()));
        assert!(a.windows(2).any(|w| w == ["-f", "rawvideo"]));
        assert_eq!(a.last().map(String::as_str), Some("pipe:1"));
        assert!(
            a.contains(&"C:/Aufnahmen/Vortrag Größe.mp4".to_string()),
            "Pfad als EIN Argument"
        );
        assert!(a.contains(&"-an".to_string()));
    }

    #[test]
    fn the_extract_command_seeks_before_the_input_and_writes_to_part_files() {
        let full = Path::new("m/slides/0007.jpg");
        assert_eq!(part_path(full), Path::new("m/slides/0007.part.jpg"));
        let a = args(&extract_command(
            Path::new("v.mp4"),
            65_432,
            &part_path(full),
            Some(&part_path(Path::new("m/slides/0007_t.jpg"))),
        ));
        assert!(a.windows(2).any(|w| w == ["-threads", DECODE_THREADS]));
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        let input = a.iter().position(|x| x == "-i").unwrap();
        assert!(ss < input, "-ss vor -i");
        assert_eq!(a[ss + 1], "65.432");
        assert!(a.contains(&"scale=320:-2".to_string()));
        assert_eq!(a.iter().filter(|x| x.ends_with(".part.jpg")).count(), 2);
        let without = args(&extract_command(
            Path::new("v.mp4"),
            0,
            Path::new("x.part.jpg"),
            None,
        ));
        assert!(!without.iter().any(|x| x.starts_with("scale")));
    }

    // -- Bilder lesen ----------------------------------------------------------------

    /// Ein `Read`, das `frames` Bilder liefert, ohne sie je zu speichern.
    struct Endless {
        frames: u64,
        pos: usize,
        extra: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let total = self.frames as usize * FRAME_BYTES + self.extra;
            let left = total.saturating_sub(self.pos);
            let n = left.min(buf.len()).min(8_192);
            buf[..n].fill((self.pos / FRAME_BYTES) as u8);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn the_reader_streams_a_very_long_video_in_constant_memory() {
        // 3 h bei 1 fps = 10 800 Bilder; hier 20 000 (288 MB Rohdaten), nie mehr als ein Puffer.
        let seen = AtomicUsize::new(0);
        let n = read_frames(
            Endless {
                frames: 20_000,
                pos: 0,
                extra: 0,
            },
            |frame| {
                assert_eq!(frame.len(), FRAME_BYTES);
                seen.fetch_add(1, Ordering::Relaxed);
                true
            },
        )
        .unwrap();
        assert_eq!(n, 20_000);
        assert_eq!(seen.load(Ordering::Relaxed), 20_000);
    }

    #[test]
    fn a_partial_last_frame_and_a_stop_are_handled() {
        let n = read_frames(
            Endless {
                frames: 3,
                pos: 0,
                extra: 5_000,
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(n, 3, "das angebrochene vierte Bild zaehlt nicht");
        let mut calls = 0;
        let n = read_frames(
            Endless {
                frames: 10,
                pos: 0,
                extra: 0,
            },
            |_| {
                calls += 1;
                calls < 4
            },
        )
        .unwrap();
        assert_eq!((n, calls), (4, 4), "false beendet das Lesen sofort");
        assert_eq!(read_frames(std::io::empty(), |_| true).unwrap(), 0);
    }

    // -- Prozesse (brauchen ffmpeg; ohne wird uebersprungen) ---------------------

    /// Ein ffmpeg, das in Echtzeit endlos Rohbilder (1 je Sekunde) liefert.
    fn endless_realtime_sampler() -> Command {
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "info",
            "-re",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("testsrc2=s=160x90:r=25")
        .args([
            "-an",
            "-vf",
            "fps=1,format=gray",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ]);
        cmd
    }

    fn never() -> SampleHook {
        Arc::new(|_| Flow::Continue)
    }

    /// Wartet hoechstens `limit`, bis `pid` gesetzt ist (0 = noch nicht gestartet).
    #[cfg(windows)]
    fn wait_for_pid(pid: &AtomicU32, limit: Duration) -> bool {
        let started = Instant::now();
        while pid.load(Ordering::Acquire) == 0 {
            if started.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    #[test]
    fn a_missing_ffmpeg_is_reported_not_a_hang() {
        let cmd = Command::new("lva-ffmpeg-gibt-es-nicht");
        let err = run_sampler(
            cmd,
            &SlideDetectConfig::default(),
            &AtomicBool::new(false),
            never(),
            None,
        )
        .unwrap_err();
        assert_eq!(err, SlideError::FfmpegMissing);
    }

    #[test]
    fn a_cancel_flag_set_before_the_start_spawns_nothing() {
        let pid = AtomicU32::new(0);
        let err = run_sampler(
            endless_realtime_sampler(),
            &SlideDetectConfig::default(),
            &AtomicBool::new(true),
            never(),
            Some(&pid),
        )
        .unwrap_err();
        assert_eq!(err, SlideError::Cancelled);
        assert_eq!(pid.load(Ordering::Acquire), 0, "kein Prozess gestartet");
    }

    #[cfg(windows)]
    #[test]
    fn stopping_the_sampler_leaves_no_ffmpeg_process() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg fehlt - Test uebersprungen");
            return;
        }
        let _serial = serial();
        // (1) Abbruch von aussen (Stopp-Flag) waehrend ffmpeg laeuft. Der Helfer setzt das
        // Flag in jedem Fall (auch ohne PID), damit der Test nie an ihm haengen bleibt.
        let cancel = Arc::new(AtomicBool::new(false));
        let pid = Arc::new(AtomicU32::new(0));
        let stopper = {
            let (cancel, pid) = (Arc::clone(&cancel), Arc::clone(&pid));
            std::thread::spawn(move || {
                let started = wait_for_pid(&pid, Duration::from_secs(60));
                if started {
                    std::thread::sleep(Duration::from_millis(1_500));
                }
                cancel.store(true, Ordering::Release);
            })
        };
        let started = Instant::now();
        let err = run_sampler(
            endless_realtime_sampler(),
            &SlideDetectConfig::default(),
            &cancel,
            never(),
            Some(&pid),
        )
        .unwrap_err();
        stopper.join().unwrap();
        assert_eq!(err, SlideError::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "kein Warten auf ein natuerliches Ende"
        );
        let first = pid.load(Ordering::Acquire);
        assert_ne!(first, 0);
        assert!(
            !process_alive(first),
            "ffmpeg {first} ist beendet (Abbruch von aussen)"
        );

        // (2) Der Hook verlangt das Ende (Stopp aus dem Auftrag).
        let pid2 = AtomicU32::new(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let hook: SampleHook = {
            let calls = Arc::clone(&calls);
            Arc::new(move |_| {
                if calls.fetch_add(1, Ordering::AcqRel) >= 1 {
                    Flow::Stop
                } else {
                    Flow::Continue
                }
            })
        };
        let err = run_sampler(
            endless_realtime_sampler(),
            &SlideDetectConfig::default(),
            &AtomicBool::new(false),
            hook,
            Some(&pid2),
        )
        .unwrap_err();
        assert_eq!(err, SlideError::Cancelled);
        let second = pid2.load(Ordering::Acquire);
        assert_ne!(second, 0);
        assert!(
            !process_alive(second),
            "ffmpeg {second} ist beendet (Stopp im Hook)"
        );
    }

    #[cfg(windows)]
    #[test]
    fn too_many_samples_stop_the_decoder_and_report_it() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg fehlt - Test uebersprungen");
            return;
        }
        let _serial = serial();
        let pid = AtomicU32::new(0);
        let cfg = SlideDetectConfig {
            max_samples: 1,
            ..SlideDetectConfig::default()
        };
        let err = run_sampler(
            endless_realtime_sampler(),
            &cfg,
            &AtomicBool::new(false),
            never(),
            Some(&pid),
        )
        .unwrap_err();
        assert_eq!(err, SlideError::TooLong);
        assert!(
            !process_alive(pid.load(Ordering::Acquire)),
            "der Dekoder laeuft nicht weiter"
        );
    }

    /// Ein Enkelprozess haelt die Pipe offen, nachdem der Kindprozess (hier `cmd`) laengst
    /// fertig ist: das Job-Objekt beendet ihn, die Abtastung wartet nicht auf ihn.
    #[cfg(windows)]
    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hold_up_the_run() {
        let _serial = serial();
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start /B ping -n 60 127.0.0.1 >nul & exit /B 0"]);
        let started = Instant::now();
        let result = run_sampler(
            cmd,
            &SlideDetectConfig::default(),
            &AtomicBool::new(false),
            never(),
            None,
        );
        assert!(
            started.elapsed() < Duration::from_secs(45),
            "kein Warten auf den 60-s-Enkel ({:?})",
            started.elapsed()
        );
        // cmd endet mit Erfolg und ohne Bild: kein Fehler, keine Abtastung.
        assert!(result.is_ok_and(|r| r.samples.is_empty()));
    }

    #[test]
    fn waiting_for_the_pipes_ends_when_they_end_or_when_nothing_moves_any_more() {
        // Beide Lesethreads fertig: sofort.
        let done = Shared::default();
        done.reader_done.store(true, Ordering::Release);
        done.stderr_done.store(true, Ordering::Release);
        assert!(wait_for_streams(&done, Duration::from_millis(50)));
        // Ein Pipe-Ende bleibt offen (fremdes Handle), nichts bewegt sich: nach der Karenz weiter.
        let stuck = Shared::default();
        stuck.stderr_done.store(true, Ordering::Release);
        let started = Instant::now();
        assert!(!wait_for_streams(&stuck, Duration::from_millis(60)));
        assert!(started.elapsed() < Duration::from_secs(2));
        // Steckt der Lesethread im Haken (Pause), wird NICHT abgebrochen, sondern gewartet,
        // bis er fertig ist.
        let paused = Arc::new(Shared::default());
        paused.stderr_done.store(true, Ordering::Release);
        paused.in_hook.store(true, Ordering::Release);
        let releaser = {
            let paused = Arc::clone(&paused);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(400));
                paused.reader_done.store(true, Ordering::Release);
            })
        };
        assert!(
            wait_for_streams(&paused, Duration::from_millis(50)),
            "in der Pause wird gewartet"
        );
        releaser.join().unwrap();
    }

    #[test]
    fn a_panicking_hook_still_ends_the_reader() {
        let shared = Shared::default();
        let hook: SampleHook = Arc::new(|_| panic!("Haken kaputt"));
        let frame = vec![7u8; FRAME_BYTES];
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            consume_frames(std::io::Cursor::new(frame), &shared, &hook, 1.0, 10)
        }));
        assert!(caught.is_err());
        assert!(
            shared.reader_done.load(Ordering::Acquire),
            "der wartende Thread haengt nicht"
        );
    }

    #[test]
    fn a_corrupt_file_fails_with_the_ffmpeg_tail() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg fehlt - Test uebersprungen");
            return;
        }
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("kaputt.mp4");
        std::fs::write(&junk, b"das ist kein Video").unwrap();
        let err = sample_video(
            &junk,
            &SlideDetectConfig::default(),
            &AtomicBool::new(false),
            never(),
        )
        .unwrap_err();
        assert!(
            matches!(err, SlideError::Ffmpeg(ref t) if !t.is_empty()),
            "{err:?}"
        );
        // Eine Datei, die es nicht gibt, ist ebenfalls ein Fehler mit Text, kein Haenger.
        let err = sample_video(
            &dir.path().join("gibt-es-nicht.mp4"),
            &SlideDetectConfig::default(),
            &AtomicBool::new(false),
            never(),
        )
        .unwrap_err();
        assert!(matches!(err, SlideError::Ffmpeg(_)), "{err:?}");
    }

    #[test]
    fn a_failed_extraction_leaves_no_partial_file() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg fehlt - Test uebersprungen");
            return;
        }
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        // Ausgabeordner existiert nicht: ffmpeg kann nicht schreiben.
        let full = dir.path().join("fehlt").join("0001.jpg");
        let thumb = dir.path().join("fehlt").join("0001_t.jpg");
        let junk = dir.path().join("kaputt.mp4");
        std::fs::write(&junk, b"kein Video").unwrap();
        let err =
            extract_frame(&junk, 0, &full, Some(&thumb), &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(err, SlideError::Ffmpeg(_)), "{err:?}");
        assert!(!full.exists() && !thumb.exists());
        assert!(!part_path(&full).exists() && !part_path(&thumb).exists());
        // Mit gesetztem Abbruch wird nichts gestartet und nichts hinterlassen.
        std::fs::create_dir_all(dir.path().join("fehlt")).unwrap();
        let err = extract_frame(&junk, 0, &full, Some(&thumb), &AtomicBool::new(true)).unwrap_err();
        assert_eq!(err, SlideError::Cancelled);
        assert_eq!(
            std::fs::read_dir(dir.path().join("fehlt")).unwrap().count(),
            0
        );
    }

    #[test]
    fn leftover_partial_files_are_cleaned_before_a_run() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["0001.jpg", "0001_t.jpg", "0002.part.jpg", "0002_t.part.jpg"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        assert_eq!(clean_partial_files(dir.path()), 2);
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec!["0001.jpg", "0001_t.jpg"],
            "fertige Bilder bleiben"
        );
        assert_eq!(clean_partial_files(&dir.path().join("gibt-es-nicht")), 0);
    }
}
