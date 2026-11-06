//! Piper-Engine (Paket B-E2): winziges VITS-TTS als CPU-Subprozess.
//!
//! Piper wird wie ffmpeg (`crate::media::decode_media_to_wav`) als externes
//! Binary gestartet — KEIN Linking, keine neue Rust-Dependency; das
//! GPL-lizenzierte espeak-ng bleibt im Subprozess. Je Syntheseauftrag läuft
//! genau ein Prozess: Text auf stdin, WAV in eine Temp-Datei, fertig. Das
//! ist bewusst der einfache Weg; ein langlebiger `--json-input`-Prozess ist
//! als Optimierung denkbar (Follow-up), aber erst, wenn er gebraucht wird.
//!
//! Binary und Stimmen legt Paket E3 in das App-Datenverzeichnis; die
//! Pfad-Funktionen hier sind der Ablage-VERTRAG zwischen beiden Paketen:
//!
//! ```text
//! <app_data>/tts/piper/<plattform>/piper(.exe)   das Binary
//! <app_data>/tts/piper/voices/<id>.onnx          eine Stimme
//! <app_data>/tts/piper/voices/<id>.onnx.json     ihre Konfiguration
//! ```
//!
//! Fehlt etwas, bleibt die Engine gewählt, aber nicht einsatzbereit:
//! `ensure_ready`/`synthesize` liefern eine KONSTANTE Fehler-ID (die UI
//! übersetzt) — es gibt keinen stillen Rückfall auf den Fish-GPU-Server.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::engine::{EngineCaps, SynthesisRequest, TtsEngine, TtsEngineKind};
use super::protocol;

/// Fähigkeiten von Piper: eine feste Stimme je Modell, kein Klonen, keine
/// Stil-Tags — dafür CPU-only und in unter einer Sekunde beim ersten Ton.
pub const PIPER_CAPS: EngineCaps = EngineCaps {
    style_tags: false,
    pace_tags: false,
    cloning: false,
    voice_switching: false,
    streaming: false,
    needs_gpu: false,
    export_formats: &["wav"],
};

/// Härter als das Fish-Timeout (300 s): Piper spricht einen Satz auf CPU in
/// wenigen Sekunden — wer länger als eine Minute braucht, hängt.
pub const PIPER_TIMEOUT: Duration = Duration::from_secs(60);

/// Konstante Fehler-IDs (Vertrag mit der UI: an den Texten wird verglichen
/// und übersetzt — NICHT umformulieren).
pub const ERR_BINARY_MISSING: &str = "Piper-Programm fehlt — unter Modelle > Vorlesestimmen laden";
pub const ERR_VOICE_MISSING: &str = "Piper-Stimme fehlt — unter Modelle > Vorlesestimmen laden";
/// Programm vorhanden, aber eine Bibliothek oder die Sprachdaten fehlen: der
/// Start scheiterte sonst erst beim Vorlesen (`dyld: Library not loaded`).
pub const ERR_BINARY_INCOMPLETE: &str =
    "Piper-Laufzeit unvollständig — unter Modelle > Vorlesestimmen erneut laden";

/// Aufgelöste, existierende Pfade eines startklaren Piper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiperPaths {
    pub binary: PathBuf,
    pub model: PathBuf,
    pub config: PathBuf,
}

/// Plattform-Unterverzeichnis des Binaries — Teil des Ablage-Vertrags mit
/// Paket E3 (Downloads): dieselbe Kennung muss dort beim Entpacken entstehen.
pub fn platform_subdir() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64"
        } else {
            "macos-x64"
        }
    } else {
        "linux-x64"
    }
}

fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "piper.exe"
    } else {
        "piper"
    }
}

/// Wo das Piper-Binary liegt (ob es liegt, prüft [`piper_paths`]).
pub fn piper_binary_path(app_data: &Path) -> PathBuf {
    app_data
        .join("tts")
        .join("piper")
        .join(platform_subdir())
        .join(binary_name())
}

/// Ist die Piper-Laufzeit startfaehig? Ok = Pfad des Programms; sonst die
/// konstante Fehler-ID: `ERR_BINARY_MISSING` (kein Programm) oder
/// `ERR_BINARY_INCOMPLETE` (Programm da, Bibliotheken/Sprachdaten fehlen).
fn runtime_check(app_data: &Path) -> Result<PathBuf, &'static str> {
    let binary = piper_binary_path(app_data);
    if !binary.is_file() {
        return Err(ERR_BINARY_MISSING);
    }
    let dir = binary.parent().unwrap_or(app_data);
    if !super::availability::piper_ready(dir, platform_subdir()) {
        return Err(ERR_BINARY_INCOMPLETE);
    }
    Ok(binary)
}

/// Wo Modell und Konfiguration einer Stimme liegen.
pub fn piper_voice_paths(app_data: &Path, voice_id: &str) -> (PathBuf, PathBuf) {
    let dir = app_data.join("tts").join("piper").join("voices");
    (
        dir.join(format!("{voice_id}.onnx")),
        dir.join(format!("{voice_id}.onnx.json")),
    )
}

/// Pfad-Auflösung: alles vorhanden → die drei Pfade; sonst die konstante
/// Fehler-ID dessen, was fehlt. Kein Setting-Override (`tts_piper_dir` o. Ä.)
/// — der Katalog (E3) ist der einzige Weg, Dateien hierher zu bekommen.
pub fn piper_paths(app_data: &Path, voice_id: &str) -> Result<PiperPaths, &'static str> {
    let binary = runtime_check(app_data)?;
    let (model, config) = piper_voice_paths(app_data, voice_id);
    if !model.is_file() || !config.is_file() {
        return Err(ERR_VOICE_MISSING);
    }
    Ok(PiperPaths {
        binary,
        model,
        config,
    })
}

/// `--length_scale` aus dem Wiedergabetempo: Kehrwert (doppeltes Tempo =
/// halbe Länge), geklemmt auf 0.5–2.0 — außerhalb klingt VITS hörbar
/// artefaktig. Unsinnige Werte (0, negativ, NaN) fallen auf 1.0 zurück.
pub fn length_scale_for_speed(speed: f32) -> f32 {
    if !speed.is_finite() || speed <= 0.0 {
        return 1.0;
    }
    (1.0 / speed).clamp(0.5, 2.0)
}

/// Kommandozeile eines Piper-Aufrufs — pur und damit testbar.
///
/// Die Reihenfolge ist FEST (model, config, output_file, dann optional
/// length_scale): die Fake-Binaries der Prozesstests greifen den
/// Ausgabepfad über seine Argumentposition (Nr. 6) ab.
pub fn piper_args(
    paths: &PiperPaths,
    out_wav: &Path,
    speed: Option<f32>,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--model".into(),
        paths.model.clone().into(),
        "--config".into(),
        paths.config.clone().into(),
        "--output_file".into(),
        out_wav.to_path_buf().into(),
    ];
    if let Some(speed) = speed {
        args.push("--length_scale".into());
        args.push(length_scale_for_speed(speed).to_string().into());
    }
    args
}

/// Die letzten `max_lines` nicht leeren Zeilen — eine Fehlermeldung ist kein
/// Protokollfenster, aber die letzten Zeilen von Pipers stderr sagen fast
/// immer, WAS fehlte (Modell unlesbar, Phonemizer-Daten weg, …).
fn stderr_tail(stderr: &str, max_lines: usize) -> String {
    let mut lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    lines.join(" | ")
}

/// Ein Piper-Lauf, blockierend: Prozess starten, Text auf stdin, auf die
/// WAV-Datei warten (mit Timeout), Bytes validiert zurückgeben. Die
/// Temp-Datei räumt sich über ihren Drop auf — auch auf jedem Fehlerpfad.
pub fn run_piper_blocking(
    paths: &PiperPaths,
    text: &str,
    speed: Option<f32>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    run_piper_cancellable(paths, text, speed, timeout, None)
}

/// Beendet `child` samt Kindprozessen — auch dann, wenn `child` selbst
/// schon beendet ist und nur noch Kinder mit geerbten Pipes übrig sind.
/// Windows: das Job-Objekt mit KILL_ON_JOB_CLOSE schließen (ohne Job
/// `taskkill /T`, solange der Elternprozess läuft). Unix: SIGKILL an die
/// eigene Prozessgruppe (gestartet mit `process_group(0)`); die Gruppen-ID
/// bleibt gültig, solange ein Mitglied lebt, und wird bis dahin nicht als
/// PID neu vergeben. Ist die Gruppe leer, scheitert das Signal folgenlos.
fn kill_tree(child: &mut std::process::Child, job: Option<crate::process_guard::ProcessGuard>) {
    #[cfg(windows)]
    if job.is_none() && matches!(child.try_wait(), Ok(None)) {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
    }
    drop(job);
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Fehlertext, wenn Piper unter Windows nicht in ein Job-Objekt kommt:
/// der Prozess wird dann gar nicht erst fortgesetzt.
#[cfg(windows)]
pub const ERR_NO_PROCESS_GUARD: &str = "Piper: Prozessschutz (Job-Objekt) nicht verfügbar";

#[cfg(all(test, windows))]
thread_local! {
    /// Test-Schalter: überspringt die Job-Zuordnung (wie ein gescheitertes
    /// `attach`), ohne dass je ein Job-Objekt entsteht.
    static FORCE_NO_JOB: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Höchstwartezeit auf die I/O-Threads, nachdem der Prozessbaum beendet
/// ist. Hält ein unerreichbarer Prozess eine Pipe trotzdem offen, endet der
/// Lauf nach dieser Frist; der Thread endet dann mit dem Schließen der Pipe.
const IO_GRACE: Duration = Duration::from_secs(2);

/// Startet `f` auf einem eigenen Thread; das Ergebnis kommt über den Kanal,
/// damit das Warten darauf eine Frist haben kann (`join` hat keine).
fn spawn_io<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> std::sync::mpsc::Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

/// Wie [`run_piper_blocking`]; ein gesetztes `cancel` beendet den Prozess
/// sofort (Prüfung alle 25 ms), statt bis zum Ende oder Timeout zu warten.
pub fn run_piper_cancellable(
    paths: &PiperPaths,
    text: &str,
    speed: Option<f32>,
    timeout: Duration,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u8>, String> {
    let out = tempfile::Builder::new()
        .prefix("lva-piper-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| format!("Piper: keine Temp-Datei: {e}"))?
        .into_temp_path();

    // Nie eine Platzhalter-/kaputte .exe starten (Windows zeigte sonst einen modalen Dialog).
    crate::process_guard::check_launchable(&paths.binary)
        .map_err(|e| format!("{ERR_BINARY_MISSING} ({e})"))?;
    let mut cmd = std::process::Command::new(&paths.binary);
    cmd.args(piper_args(paths, &out, speed))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW (Muster: media.rs) und angehalten starten: erst
        // nach der Job-Zuordnung läuft der Prozess an (siehe unten).
        cmd.creation_flags(0x0800_0000 | crate::process_guard::CREATE_SUSPENDED);
    }
    // Eigene Prozessgruppe: Abbruch und Timeout beenden auch Kindprozesse.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{ERR_BINARY_MISSING} ({e})"))?;
    // Job-Objekt mit KILL_ON_JOB_CLOSE: endet die App mitten in einer
    // Synthese (Beenden, Absturz), stirbt Piper mit, statt als Waise zu
    // laufen. Kein Speicherdeckel, CPU-Deckel wie bei ffmpeg.
    // Test-Schalter (nur Windows-Tests): attach wird gar nicht erst
    // versucht, der Prozess bleibt wirklich ohne Job-Zuordnung.
    #[cfg(all(test, windows))]
    let skip_attach = FORCE_NO_JOB.with(|f| f.get());
    #[cfg(not(all(test, windows)))]
    let skip_attach = false;
    let mut job = if skip_attach {
        None
    } else {
        crate::process_guard::ProcessGuard::attach(
            &child,
            None,
            crate::process_guard::CPU_CAP_PERCENT,
        )
    };
    // Windows: Der Prozess wurde angehalten gestartet. Erst nach erfolgreicher
    // Job-Zuordnung wird er fortgesetzt — vorher kann er weder Kinder starten
    // noch sich beenden, also entgeht kein Kind dem Job. Ohne Job (oder wenn
    // das Fortsetzen scheitert) wird er beendet, bevor er je gelaufen ist:
    // kein Lauf ohne Prozessbaum-Schutz.
    #[cfg(windows)]
    {
        let resumed = match job {
            Some(_) => crate::process_guard::resume_suspended(&child),
            None => Err("kein Job-Objekt".to_string()),
        };
        if let Err(e) = resumed {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{ERR_NO_PROCESS_GUARD} ({e})"));
        }
    }

    // stderr nebenläufig leeren, BEVOR geschrieben wird: ein gesprächiges
    // Piper bleibt so nicht am vollen Puffer hängen.
    let mut stderr_pipe = child.stderr.take();
    let stderr_reader = spawn_io(move || {
        let mut buf = String::new();
        if let Some(e) = stderr_pipe.as_mut() {
            let _ = e.read_to_string(&mut buf);
        }
        buf
    });

    // Der Text als GENAU EINE Zeile: Piper spricht je stdin-Zeile eine
    // Äußerung — ein Satz mit hartem Umbruch käme sonst zweigeteilt heraus.
    // Geschrieben wird in einem eigenen Thread: liest Piper stdin nicht, hängt
    // nur dieser Thread an der vollen Pipe, Deadline und Abbruch greifen
    // trotzdem (das Beenden schließt die Pipe, der Schreiber endet). Ein
    // Schreibfehler wird bewusst verschluckt: die Diagnose liefern dann
    // Exit-Status und stderr. Drop schließt stdin: Piper sieht EOF.
    let stdin_pipe = child.stdin.take();
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let stdin_writer = spawn_io(move || {
        if let Some(mut stdin) = stdin_pipe {
            use std::io::Write as _;
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
    });

    let cancelled = || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Acquire));
    // Erst den ganzen Prozessbaum beenden — auch nach einem regulären Ende
    // des Elternprozesses, denn ein Kind kann die geerbten Pipes offen
    // halten —, dann die I/O-Threads mit Frist einsammeln. Während des
    // Wartens bleiben Abbruch und Deadline wirksam.
    let deadline = std::time::Instant::now() + timeout;
    let finish = |child: &mut std::process::Child,
                  job: &mut Option<crate::process_guard::ProcessGuard>|
     -> Result<String, String> {
        kill_tree(child, job.take());
        let until = std::time::Instant::now() + IO_GRACE;
        let mut stderr = None;
        let mut written = false;
        while stderr.is_none() || !written {
            if stderr.is_none() {
                stderr = stderr_reader.try_recv().ok();
            }
            if !written {
                written = stdin_writer.try_recv().is_ok();
            }
            if stderr.is_some() && written {
                break;
            }
            if cancelled() {
                return Err(super::providers::pipeline::CANCELLED_MESSAGE.to_string());
            }
            if std::time::Instant::now() >= until {
                log::warn!("Piper: I/O-Threads nach Prozessende nicht beendet, Frist abgelaufen");
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(stderr.unwrap_or_default())
    };

    // Warten mit Timeout — `std::process` kennt keins, also try_wait-Schleife.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if cancelled() {
                    let _ = finish(&mut child, &mut job);
                    return Err(super::providers::pipeline::CANCELLED_MESSAGE.to_string());
                }
                if std::time::Instant::now() >= deadline {
                    let _ = finish(&mut child, &mut job);
                    return Err(format!(
                        "Piper hat nach {} s nicht geantwortet und wurde beendet",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = finish(&mut child, &mut job);
                return Err(format!("Piper-Prozess nicht abfragbar: {e}"));
            }
        }
    };
    let stderr = finish(&mut child, &mut job)?;
    if !status.success() {
        return Err(format!(
            "Piper endete mit {status}: {}",
            stderr_tail(&stderr, 10)
        ));
    }
    let bytes = std::fs::read(&out).map_err(|e| format!("Piper-Ausgabe nicht lesbar: {e}"))?;
    if !protocol::looks_like_wav(&bytes) {
        return Err(format!(
            "Piper lieferte kein WAV: {}",
            stderr_tail(&stderr, 10)
        ));
    }
    Ok(bytes)
}

/// Die Piper-Engine des Kerns: hält die aufgelösten Pfade ODER den Grund,
/// warum sie nicht einsatzbereit ist. Aufgelöst wird bei jedem
/// Settings-Refresh (also vor jedem Auftrag) neu — nachgeladene Dateien
/// wirken damit ohne App-Neustart.
#[derive(Clone, Debug)]
pub struct PiperEngine {
    /// Gewählte Stimme (`tts_piper_voice`); geht in den Cache-Tag ein.
    voice_id: Option<String>,
    /// Ok: startklar. Err: konstante Fehler-ID, was fehlt.
    resolved: Result<PiperPaths, &'static str>,
    /// Sprache je Satz erkennen und die passende geladene Stimme nehmen
    /// (`tts_piper_auto_language`). Eine deutsche Stimme liest englischen
    /// Text sonst mit deutscher Aussprache — unbrauchbar.
    auto_language: bool,
    /// Alle geladenen Stimmen mit ihrer Sprache (`de_DE-thorsten-high` → `de`),
    /// sortiert: die gewählte zuerst, dann nach Kennung.
    alternatives: Vec<(String, String, PiperPaths)>,
}

/// Sprachkürzel aus der Stimmen-Kennung (`de_DE-thorsten-high` → `de`).
pub fn language_of_voice_id(voice_id: &str) -> Option<String> {
    let locale = voice_id.split('-').next()?;
    let lang = locale.split('_').next()?;
    (!lang.is_empty()).then(|| lang.to_lowercase())
}

/// Alle vollständigen Stimmen (`.onnx` + `.onnx.json`) im Stimmenordner.
pub fn installed_voices(app_data: &Path) -> Vec<(String, String, PiperPaths)> {
    let dir = app_data.join("tts").join("piper").join("voices");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, PiperPaths)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".onnx")?.to_string();
            let lang = language_of_voice_id(&id)?;
            let paths = piper_paths(app_data, &id).ok()?;
            Some((id, lang, paths))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

impl PiperEngine {
    /// Aus App-Datenverzeichnis und Settings-Stimme auflösen. Ohne
    /// Datenverzeichnis kann kein Binary liegen; ohne gewählte Stimme ist
    /// die Meldung „Stimme fehlt" nur dann dran, wenn das Binary schon da
    /// ist — sonst ist das Binary die erste Baustelle.
    pub fn resolve(app_data: Option<&Path>, voice_id: Option<&str>) -> Self {
        let resolved = match (app_data, voice_id) {
            (None, _) => Err(ERR_BINARY_MISSING),
            (Some(data), None) => match runtime_check(data) {
                Ok(_) => Err(ERR_VOICE_MISSING),
                Err(reason) => Err(reason),
            },
            (Some(data), Some(voice)) => piper_paths(data, voice),
        };
        Self {
            voice_id: voice_id.map(str::to_string),
            resolved,
            auto_language: false,
            alternatives: Vec::new(),
        }
    }

    /// Wie `resolve`, zusätzlich mit allen geladenen Stimmen als Kandidaten
    /// für die automatische Sprachwahl.
    pub fn resolve_auto(
        app_data: Option<&Path>,
        voice_id: Option<&str>,
        auto_language: bool,
    ) -> Self {
        let mut engine = Self::resolve(app_data, voice_id);
        engine.auto_language = auto_language;
        if let Some(data) = app_data {
            let mut all = installed_voices(data);
            if let Some(chosen) = voice_id {
                all.sort_by_key(|(id, _, _)| id != chosen);
            }
            engine.alternatives = all;
        }
        // Ohne gewählte Stimme, aber mit geladenen: die erste ist gut genug —
        // besser als „Stimme fehlt", wenn eine im Ordner liegt. NUR ohne Wahl:
        // wurde die gewählte Stimme entfernt oder ist defekt, bleibt es beim
        // Fehler — still eine andere Stimme zu nehmen hiesse, dem Nutzer eine
        // Stimme unterzuschieben, die er nicht gewählt hat (Issue #29).
        if engine.resolved.is_err()
            && voice_id.is_none()
            && app_data.is_some_and(|d| runtime_check(d).is_ok())
        {
            if let Some((id, _, paths)) = engine.alternatives.first().cloned() {
                engine.voice_id = Some(id);
                engine.resolved = Ok(paths);
            }
        }
        engine
    }

    /// Die Pfade für diesen Satz: bei Auto-Sprache die geladene Stimme, deren
    /// Sprache zum Text passt; sonst (oder ohne Treffer) die gewählte.
    pub fn paths_for(&self, text: &str) -> Result<PiperPaths, &'static str> {
        let chosen = self.resolved.clone()?;
        if !self.auto_language {
            return Ok(chosen);
        }
        let Some(lang) = super::lang::detect_language(text) else {
            return Ok(chosen);
        };
        let chosen_lang = self.voice_id.as_deref().and_then(language_of_voice_id);
        if chosen_lang.as_deref() == Some(lang) {
            return Ok(chosen);
        }
        Ok(self
            .alternatives
            .iter()
            .find(|(_, l, _)| l == lang)
            .map(|(_, _, p)| p.clone())
            .unwrap_or(chosen))
    }

    /// Die Stimme, die diesen Text WIRKLICH liest, wenn es nicht die gewaehlte
    /// ist: bei Auto-Sprache die geladene Stimme der erkannten Sprache. `None`:
    /// die gewaehlte liest (Auto-Sprache aus, Sprache unklar, gleiche Sprache,
    /// keine passende Stimme geladen).
    pub fn replacement_voice_id(&self, text: &str) -> Option<String> {
        if !self.auto_language {
            return None;
        }
        let lang = super::lang::detect_language(text)?;
        let chosen_lang = self.voice_id.as_deref().and_then(language_of_voice_id);
        if chosen_lang.as_deref() == Some(lang) {
            return None;
        }
        self.alternatives
            .iter()
            .find(|(_, l, _)| l == lang)
            .map(|(id, _, _)| id.clone())
    }

    /// Warum die Engine nicht einsatzbereit ist (None = bereit) — Grundlage
    /// der Warnzeile beim Engine-Wechsel in `TtsCore::set_engine`.
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        self.resolved.as_ref().err().copied()
    }
}

impl TtsEngine for PiperEngine {
    fn kind(&self) -> TtsEngineKind {
        TtsEngineKind::Piper
    }

    fn caps(&self) -> EngineCaps {
        PIPER_CAPS
    }

    /// `"piper/<voice_id>"` — der Parameter (die Fish-Referenzstimme) ist
    /// für Piper bedeutungslos; was den Klang bestimmt, ist das Modell.
    fn cache_tag(&self, _voice: Option<&str>) -> String {
        if !self.auto_language {
            return format!("piper/{}", self.voice_id.as_deref().unwrap_or(""));
        }
        // Mit Auto-Sprache haengt die Stimme eines Satzes von den geladenen
        // Stimmen ab: kommt eine dazu, muss derselbe Satz neu entstehen.
        let langs: Vec<&str> = self
            .alternatives
            .iter()
            .map(|(id, _, _)| id.as_str())
            .collect();
        format!(
            "piper/{}/auto:{}",
            self.voice_id.as_deref().unwrap_or(""),
            langs.join(",")
        )
    }

    async fn ensure_ready(&self) -> Result<(), String> {
        self.resolved
            .as_ref()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Ein Satz = ein Subprozess, ausgelagert auf einen Blocking-Thread —
    /// der try_wait-Timeout darf keinen Async-Worker blockieren.
    async fn synthesize(&self, req: SynthesisRequest<'_>) -> Result<Vec<u8>, String> {
        let paths = self.paths_for(req.text).map_err(str::to_string)?;
        let text = req.text.to_string();
        let speed = req.speed;
        let cancel = req.cancel.cloned();
        tokio::task::spawn_blocking(move || {
            run_piper_cancellable(&paths, &text, speed, PIPER_TIMEOUT, cancel.as_deref())
        })
        .await
        .map_err(|e| format!("Piper-Lauf abgebrochen: {e}"))?
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Pfad-Auflösung
    // ------------------------------------------------------------------

    #[test]
    fn das_binary_liegt_im_plattform_unterverzeichnis() {
        let p = piper_binary_path(Path::new("data"));
        let expected: PathBuf = ["data", "tts", "piper", platform_subdir(), binary_name()]
            .iter()
            .collect();
        assert_eq!(p, expected);
        if cfg!(windows) {
            assert!(p.ends_with("windows-x64/piper.exe"));
        }
    }

    #[test]
    fn stimmen_liegen_als_onnx_paar_im_voices_verzeichnis() {
        let (model, config) = piper_voice_paths(Path::new("data"), "eva");
        let voices: PathBuf = ["data", "tts", "piper", "voices"].iter().collect();
        assert_eq!(model, voices.join("eva.onnx"));
        assert_eq!(config, voices.join("eva.onnx.json"));
    }

    /// Eine vollstaendige Laufzeit (Programm, Sprachdaten, Bibliotheken).
    fn write_runtime(data: &Path) {
        let bin = piper_binary_path(data);
        crate::managers::tts::availability::write_piper_fixture(
            bin.parent().unwrap(),
            platform_subdir(),
        );
    }

    /// Ein tempdir-Baukasten: legt wahlweise Binary und/oder Stimme an.
    fn data_dir(binary: bool, voice: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if binary {
            write_runtime(dir.path());
        }
        if let Some(id) = voice {
            let (model, config) = piper_voice_paths(dir.path(), id);
            std::fs::create_dir_all(model.parent().unwrap()).unwrap();
            std::fs::write(&model, b"onnx").unwrap();
            std::fs::write(&config, b"{}").unwrap();
        }
        dir
    }

    #[test]
    fn fehlendes_binary_meldet_die_binary_id_auch_wenn_die_stimme_da_ist() {
        let dir = data_dir(false, Some("eva"));
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_BINARY_MISSING));
    }

    #[test]
    fn fehlende_stimme_oder_fehlende_config_melden_die_stimmen_id() {
        let dir = data_dir(true, None);
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_VOICE_MISSING));

        // Modell da, aber die .onnx.json fehlt → ebenfalls Stimme unvollständig.
        let dir = data_dir(true, Some("eva"));
        let (_, config) = piper_voice_paths(dir.path(), "eva");
        std::fs::remove_file(config).unwrap();
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_VOICE_MISSING));
    }

    /// Issue #29: Programm und Stimme sind da, aber eine Bibliothek fehlt --
    /// das darf nicht als startklar gelten (sonst scheitert erst das Vorlesen).
    #[test]
    fn fehlende_bibliothek_meldet_unvollstaendige_laufzeit() {
        let dir = data_dir(true, Some("eva"));
        let lib = crate::managers::tts::availability::piper_libraries(platform_subdir())[0];
        let bin = piper_binary_path(dir.path());
        std::fs::remove_file(bin.parent().unwrap().join(lib)).unwrap();
        assert_eq!(piper_paths(dir.path(), "eva"), Err(ERR_BINARY_INCOMPLETE));
        let e = PiperEngine::resolve(Some(dir.path()), Some("eva"));
        assert_eq!(e.unavailable_reason(), Some(ERR_BINARY_INCOMPLETE));
        // Ohne gewaehlte Stimme meldet die unvollstaendige Laufzeit sich zuerst.
        let e = PiperEngine::resolve(Some(dir.path()), None);
        assert_eq!(e.unavailable_reason(), Some(ERR_BINARY_INCOMPLETE));
    }

    /// Issue #29: "keine stille andere Stimme" -- wurde die gewaehlte Stimme
    /// entfernt, bleibt es beim Fehler, auch wenn andere Stimmen geladen sind.
    #[test]
    fn entfernte_gewaehlte_stimme_wird_nicht_still_ersetzt() {
        let dir = data_dir(true, Some("en_US-lessac-medium"));
        let e = PiperEngine::resolve_auto(Some(dir.path()), Some("de_DE-thorsten-high"), true);
        assert_eq!(e.unavailable_reason(), Some(ERR_VOICE_MISSING));
        assert_eq!(e.paths_for("Hallo Welt."), Err(ERR_VOICE_MISSING));
        // Ohne jede Wahl springt weiterhin die erste geladene Stimme ein.
        let ohne = PiperEngine::resolve_auto(Some(dir.path()), None, true);
        assert!(ohne.unavailable_reason().is_none());
    }

    #[test]
    fn vollstaendige_ablage_liefert_die_drei_pfade() {
        let dir = data_dir(true, Some("eva"));
        let paths = piper_paths(dir.path(), "eva").unwrap();
        assert_eq!(paths.binary, piper_binary_path(dir.path()));
        assert!(paths.model.ends_with("eva.onnx"));
        assert!(paths.config.ends_with("eva.onnx.json"));
    }

    #[test]
    fn auto_sprache_nimmt_die_stimme_der_erkannten_sprache() {
        let dir = tempfile::tempdir().unwrap();
        write_runtime(dir.path());
        for id in ["de_DE-thorsten-high", "en_US-lessac-medium"] {
            let (m, c) = piper_voice_paths(dir.path(), id);
            std::fs::create_dir_all(m.parent().unwrap()).unwrap();
            std::fs::write(&m, b"onnx").unwrap();
            std::fs::write(&c, b"{}").unwrap();
        }
        let e = PiperEngine::resolve_auto(Some(dir.path()), Some("de_DE-thorsten-high"), true);
        assert!(e
            .paths_for("It was a quiet evening in the hills.")
            .unwrap()
            .model
            .ends_with("en_US-lessac-medium.onnx"));
        assert!(e
            .paths_for("Es war ein ruhiger Abend über den Hügeln.")
            .unwrap()
            .model
            .ends_with("de_DE-thorsten-high.onnx"));
        assert!(
            e.paths_for("Luminara")
                .unwrap()
                .model
                .ends_with("de_DE-thorsten-high.onnx"),
            "unklar = gewaehlte Stimme"
        );
        // Ohne Auto-Sprache bleibt es bei der Wahl, egal was der Text sagt.
        let fest = PiperEngine::resolve_auto(Some(dir.path()), Some("de_DE-thorsten-high"), false);
        assert!(fest
            .paths_for("It was a quiet evening in the hills.")
            .unwrap()
            .model
            .ends_with("de_DE-thorsten-high.onnx"));
        assert_ne!(e.cache_tag(None), fest.cache_tag(None));
        // Keine Stimme gewaehlt, aber geladen: die erste springt ein.
        let ohne = PiperEngine::resolve_auto(Some(dir.path()), None, true);
        assert!(ohne.unavailable_reason().is_none());
    }

    #[test]
    fn resolve_benennt_die_erste_baustelle() {
        // Kein Datenverzeichnis → das Binary ist die erste Baustelle.
        let e = PiperEngine::resolve(None, Some("eva"));
        assert_eq!(e.unavailable_reason(), Some(ERR_BINARY_MISSING));

        // Binary da, keine Stimme gewählt → die Stimme fehlt.
        let dir = data_dir(true, None);
        let e = PiperEngine::resolve(Some(dir.path()), None);
        assert_eq!(e.unavailable_reason(), Some(ERR_VOICE_MISSING));

        // Nichts da, nichts gewählt → erst das Binary.
        let empty = tempfile::tempdir().unwrap();
        let e = PiperEngine::resolve(Some(empty.path()), None);
        assert_eq!(e.unavailable_reason(), Some(ERR_BINARY_MISSING));

        // Alles da → bereit.
        let dir = data_dir(true, Some("eva"));
        let e = PiperEngine::resolve(Some(dir.path()), Some("eva"));
        assert_eq!(e.unavailable_reason(), None);
    }

    // ------------------------------------------------------------------
    // Kommandozeile und Tempo
    // ------------------------------------------------------------------

    #[test]
    fn das_tempo_wird_zum_gekehrten_und_geklemmten_length_scale() {
        assert_eq!(length_scale_for_speed(1.0), 1.0);
        assert_eq!(length_scale_for_speed(2.0), 0.5);
        assert_eq!(length_scale_for_speed(0.5), 2.0);
        assert_eq!(length_scale_for_speed(4.0), 0.5, "unten geklemmt");
        assert_eq!(length_scale_for_speed(0.25), 2.0, "oben geklemmt");
        assert_eq!(length_scale_for_speed(0.0), 1.0, "0 wäre unendlich");
        assert_eq!(length_scale_for_speed(-1.0), 1.0);
        assert_eq!(length_scale_for_speed(f32::NAN), 1.0);
    }

    fn dummy_paths() -> PiperPaths {
        PiperPaths {
            binary: PathBuf::from("piper.exe"),
            model: PathBuf::from("eva.onnx"),
            config: PathBuf::from("eva.onnx.json"),
        }
    }

    #[test]
    fn die_kommandozeile_traegt_model_config_und_ausgabedatei_in_fester_reihenfolge() {
        let args = piper_args(&dummy_paths(), Path::new("out.wav"), None);
        let expected: Vec<std::ffi::OsString> = vec![
            "--model".into(),
            "eva.onnx".into(),
            "--config".into(),
            "eva.onnx.json".into(),
            "--output_file".into(),
            "out.wav".into(),
        ];
        assert_eq!(args, expected, "Reihenfolge ist Vertrag (Fake-Tests: %6)");
    }

    #[test]
    fn ein_tempo_ergaenzt_den_geklemmten_length_scale() {
        let args = piper_args(&dummy_paths(), Path::new("out.wav"), Some(2.0));
        assert_eq!(args.len(), 8);
        assert_eq!(args[6], std::ffi::OsString::from("--length_scale"));
        assert_eq!(args[7], std::ffi::OsString::from("0.5"));
    }

    // ------------------------------------------------------------------
    // Caps, Cache-Tag, stderr-Kürzung
    // ------------------------------------------------------------------

    #[test]
    fn piper_ist_cpu_only_und_exportiert_nur_wav() {
        assert!(!PIPER_CAPS.needs_gpu);
        assert!(!PIPER_CAPS.cloning && !PIPER_CAPS.voice_switching && !PIPER_CAPS.style_tags);
        assert!(!PIPER_CAPS.streaming);
        assert_eq!(PIPER_CAPS.export_formats, &["wav"]);
    }

    #[test]
    fn der_cache_tag_traegt_die_piper_stimme_nicht_die_fish_referenz() {
        let dir = data_dir(true, Some("eva"));
        let e = PiperEngine::resolve(Some(dir.path()), Some("eva"));
        assert_eq!(e.cache_tag(None), "piper/eva");
        assert_eq!(
            e.cache_tag(Some("patrick")),
            "piper/eva",
            "die Fish-Referenzstimme ändert am Piper-Klang nichts"
        );
        let ohne = PiperEngine::resolve(None, None);
        assert_eq!(ohne.cache_tag(None), "piper/");
    }

    #[test]
    fn stderr_wird_auf_die_letzten_zehn_zeilen_gekuerzt() {
        let long = (1..=14)
            .map(|i| format!("zeile {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = stderr_tail(&long, 10);
        assert!(!tail.contains("zeile 4"));
        assert!(tail.starts_with("zeile 5"));
        assert!(tail.ends_with("zeile 14"));
        assert_eq!(stderr_tail("  \n\n  ", 10), "", "Leerzeilen zählen nicht");
    }

    // ------------------------------------------------------------------
    // Prozesspfad mit Fake-Binaries (Muster media.rs: echte Subprozesse)
    // ------------------------------------------------------------------

    /// Ein Fake-Piper als Skript: `Ok` kopiert eine Mini-WAV an den
    /// Ausgabepfad (Argument 6), `Broken` schreibt Müll dorthin, `Fail`
    /// endet mit Exit-Code 3 und einer stderr-Zeile.
    pub(crate) enum Fake {
        Ok,
        Broken,
        Fail,
        Heartbeat,
        Orphan,
    }

    fn write_fake_piper(dir: &Path, fake: Fake) -> PathBuf {
        let template = dir.join("vorlage.wav");
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0u8; 4096]);
        std::fs::write(&template, &wav).unwrap();

        #[cfg(windows)]
        {
            let script = dir.join("fake-piper.cmd");
            let body = match fake {
                Fake::Ok => format!(
                    "@echo off\r\ncopy /y /b \"{}\" \"%~6\" > nul\r\n",
                    template.display()
                ),
                Fake::Broken => "@echo off\r\necho kein-riff > \"%~6\"\r\n".to_string(),
                Fake::Fail => {
                    "@echo off\r\necho Modelldatei nicht lesbar 1>&2\r\nexit /b 3\r\n".to_string()
                }
                // Liest stdin nie; ein Kindprozess (`start /b`) schreibt
                // jede Sekunde einen Herzschlag, der Elternprozess wartet.
                Fake::Heartbeat => format!(
                    "@echo off\r\nif \"%~1\"==\"hb\" goto loop\r\n\
                     start /b \"\" cmd /c \"%~f0\" hb\r\n\
                     ping -n 120 127.0.0.1 > nul\r\nexit /b\r\n\
                     :loop\r\necho x>> \"{hb}\"\r\nping -n 2 127.0.0.1 > nul\r\ngoto loop\r\n",
                    hb = dir.join("hb").display()
                ),
                // Startet SOFORT ein Herzschlag-Kind mit geerbten Pipes und
                // endet, sobald der erste Herzschlag geschrieben ist. Ohne
                // Verzögerung: der angehaltene Start muss das Kind in den Job
                // bringen, nicht eine Wartezeit (Codex Runde 7).
                Fake::Orphan => format!(
                    "@echo off\r\nif \"%~1\"==\"hb\" goto loop\r\n\
                     start /b \"\" cmd /c \"%~f0\" hb\r\n\
                     :warte\r\nif exist \"{hb}\" exit /b 0\r\n\
                     ping -n 2 127.0.0.1 > nul\r\ngoto warte\r\n\
                     :loop\r\necho x>> \"{hb}\"\r\nping -n 2 127.0.0.1 > nul\r\ngoto loop\r\n",
                    hb = dir.join("hb").display()
                ),
            };
            std::fs::write(&script, body).unwrap();
            script
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.join("fake-piper.sh");
            let body = match fake {
                Fake::Ok => format!("#!/bin/sh\ncp \"{}\" \"$6\"\n", template.display()),
                Fake::Broken => "#!/bin/sh\nprintf 'kein-riff' > \"$6\"\n".to_string(),
                Fake::Fail => {
                    "#!/bin/sh\necho 'Modelldatei nicht lesbar' >&2\nexit 3\n".to_string()
                }
                // Liest stdin nie; ein Kindprozess im Hintergrund schreibt
                // alle 50 ms einen Herzschlag, der Elternprozess wartet.
                Fake::Heartbeat => format!(
                    "#!/bin/sh\n( while true; do echo x >> \"{hb}\"; sleep 0.05; done ) &\nwait\n",
                    hb = dir.join("hb").display()
                ),
                // Herzschlag-Kind mit geerbten Pipes; der Elternprozess endet,
                // sobald der erste Herzschlag geschrieben ist.
                Fake::Orphan => format!(
                    "#!/bin/sh\n( while true; do echo x >> \"{hb}\"; sleep 0.05; done ) &\n\
                     while [ ! -s \"{hb}\" ]; do sleep 0.02; done\nexit 0\n",
                    hb = dir.join("hb").display()
                ),
            };
            std::fs::write(&script, body).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        }
    }

    pub(crate) fn fake_paths(dir: &Path, fake: Fake) -> PiperPaths {
        PiperPaths {
            binary: write_fake_piper(dir, fake),
            model: dir.join("eva.onnx"),
            config: dir.join("eva.onnx.json"),
        }
    }

    #[test]
    fn der_prozesspfad_liefert_die_wav_bytes_der_ausgabedatei() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Ok);
        let bytes = run_piper_blocking(&paths, "Hallo Welt.", None, PIPER_TIMEOUT).unwrap();
        assert!(protocol::looks_like_wav(&bytes));
        assert_eq!(bytes.len(), 4 + 4096);
    }

    #[test]
    fn ein_fehlendes_binary_meldet_die_binary_id() {
        let dir = tempfile::tempdir().unwrap();
        let paths = PiperPaths {
            binary: dir.path().join("gibt-es-nicht.exe"),
            model: dir.path().join("eva.onnx"),
            config: dir.path().join("eva.onnx.json"),
        };
        let err = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT).unwrap_err();
        assert!(err.contains(ERR_BINARY_MISSING), "war: {err}");
    }

    #[test]
    fn ein_scheiternder_prozess_traegt_seinen_stderr_in_der_meldung() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Fail);
        let err = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT).unwrap_err();
        assert!(err.contains("Modelldatei nicht lesbar"), "war: {err}");
    }

    /// Codex-Auflage: Abbruch bei blockierendem stdin (Text größer als der
    /// Pipe-Puffer, Piper liest nicht), Ende des ganzen Prozessbaums und
    /// Rückkehr erst nach dem Einsammeln beider I/O-Threads. Läuft unter
    /// Windows und Unix; die Rückkehr wird mit Zeitgrenze geprüft, damit ein
    /// Hänger als Fehlschlag statt als stehender Testlauf sichtbar wird.
    #[test]
    fn abbruch_beendet_den_prozessbaum_auch_bei_blockiertem_stdin() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Heartbeat);
        let hb = dir.path().join("hb");
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                let text = "wort ".repeat(400_000); // 2 MB, weit über dem Pipe-Puffer
                let started = std::time::Instant::now();
                let res = run_piper_cancellable(&paths, &text, None, PIPER_TIMEOUT, Some(&cancel));
                let _ = tx.send((res, started.elapsed()));
            });
        }
        let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let until = std::time::Instant::now() + Duration::from_secs(15);
        while len(&hb) == 0 {
            assert!(
                std::time::Instant::now() < until,
                "Herzschlag startet nicht"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        cancel.store(true, Ordering::Release);
        let (res, elapsed) = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("Rückkehr: Prozessbaum beendet, I/O-Threads eingesammelt");
        assert_eq!(
            res.unwrap_err(),
            super::super::providers::pipeline::CANCELLED_MESSAGE
        );
        assert!(elapsed < PIPER_TIMEOUT, "nicht erst nach dem Timeout");
        // Ein überlebender Kindprozess schriebe weiter.
        let settle = Duration::from_millis(if cfg!(windows) { 2500 } else { 300 });
        std::thread::sleep(settle);
        let before = len(&hb);
        std::thread::sleep(settle);
        assert_eq!(
            len(&hb),
            before,
            "Kindprozess läuft nach dem Abbruch weiter"
        );
    }

    /// Codex-Auflage Runde 6: Der Elternprozess endet regulär, ein Kind hält
    /// die geerbten Pipes offen. Der Lauf kehrt trotzdem zurück (Baum wird vor
    /// dem Einsammeln beendet, Warten mit Frist), und das Kind läuft nicht weiter.
    #[test]
    fn elternende_mit_kind_an_den_pipes_blockiert_nicht() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Orphan);
        let hb = dir.path().join("hb");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let res = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT);
            let _ = tx.send((res, started.elapsed()));
        });
        let (res, elapsed) = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("Rückkehr trotz offener Pipes des Kindes");
        let err = res.unwrap_err();
        assert!(err.contains("kein WAV"), "war: {err}");
        assert!(elapsed < PIPER_TIMEOUT, "nicht erst nach dem Timeout");
        let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        assert!(len(&hb) > 0, "Kind hatte vor dem Elternende geschrieben");
        let settle = Duration::from_millis(if cfg!(windows) { 2500 } else { 300 });
        std::thread::sleep(settle);
        let before = len(&hb);
        std::thread::sleep(settle);
        assert_eq!(
            len(&hb),
            before,
            "Kindprozess läuft nach dem Elternende weiter"
        );
    }

    /// Codex-Auflage Runde 7/8 (nur Windows): Ohne Job-Zuordnung (attach wird
    /// übersprungen, es entsteht nie ein Job-Objekt)
    /// läuft der angehalten gestartete Prozess nie an — kein Kind, kein
    /// Herzschlag, klare Fehlermeldung statt eines ungeschützten Laufs.
    #[cfg(windows)]
    #[test]
    fn ohne_job_objekt_laeuft_piper_nie_an() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Orphan);
        let hb = dir.path().join("hb");
        super::FORCE_NO_JOB.with(|f| f.set(true));
        let res = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT);
        super::FORCE_NO_JOB.with(|f| f.set(false));
        let err = res.unwrap_err();
        assert!(err.starts_with(super::ERR_NO_PROCESS_GUARD), "war: {err}");
        std::thread::sleep(Duration::from_millis(2500));
        assert!(!hb.exists(), "Prozess ist trotz fehlendem Job angelaufen");
    }

    #[test]
    fn eine_kaputte_ausgabedatei_wird_als_kein_wav_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Broken);
        let err = run_piper_blocking(&paths, "Hallo.", None, PIPER_TIMEOUT).unwrap_err();
        assert!(err.contains("kein WAV"), "war: {err}");
    }

    /// Anzeige im Stimmen-Menue: bei englischem Text liest mit Auto-Sprache
    /// eine englische Stimme, nicht die gewaehlte deutsche (Thorsten).
    #[test]
    fn die_ersatzstimme_wird_nur_bei_abweichender_sprache_genannt() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_paths(dir.path(), Fake::Ok);
        let engine = |auto: bool| PiperEngine {
            voice_id: Some("de_DE-thorsten-high".into()),
            resolved: Ok(paths.clone()),
            auto_language: auto,
            alternatives: vec![
                ("de_DE-thorsten-high".into(), "de".into(), paths.clone()),
                ("en_US-amy-medium".into(), "en".into(), paths.clone()),
            ],
        };
        let english = "Welcome to the podcast. On Monday we meet at twelve thirty and talk about the architecture.";
        let german = "Willkommen zum Podcast. Am Montag treffen wir uns um halb eins und sprechen ueber die Architektur.";
        assert_eq!(
            engine(true).replacement_voice_id(english).as_deref(),
            Some("en_US-amy-medium")
        );
        assert_eq!(engine(true).replacement_voice_id(german), None);
        assert_eq!(
            engine(false).replacement_voice_id(english),
            None,
            "Auto aus"
        );
        // Ohne geladene englische Stimme liest die gewaehlte -- nichts zu melden.
        let mut only_german = engine(true);
        only_german.alternatives.truncate(1);
        assert_eq!(only_german.replacement_voice_id(english), None);
    }

    /// Der ganze Trait-Pfad asynchron: eine startklare Engine spricht über
    /// den Fake, eine nicht aufgelöste liefert ihre konstante Fehler-ID.
    #[tokio::test(flavor = "multi_thread")]
    async fn die_engine_synthetisiert_ueber_den_subprozess_oder_meldet_warum_nicht() {
        let dir = tempfile::tempdir().unwrap();
        let ready = PiperEngine {
            voice_id: Some("eva".into()),
            resolved: Ok(fake_paths(dir.path(), Fake::Ok)),
            auto_language: false,
            alternatives: Vec::new(),
        };
        ready.ensure_ready().await.unwrap();
        let req = SynthesisRequest {
            text: "Hallo Welt.",
            voice: None,
            seed: 42,
            speed: None,
            cancel: None,
        };
        let bytes = ready.synthesize(req).await.unwrap();
        assert!(protocol::looks_like_wav(&bytes));

        let missing = PiperEngine::resolve(None, None);
        assert_eq!(
            missing.ensure_ready().await.unwrap_err(),
            ERR_BINARY_MISSING
        );
        assert_eq!(
            missing.synthesize(req).await.unwrap_err(),
            ERR_BINARY_MISSING
        );
    }
}
