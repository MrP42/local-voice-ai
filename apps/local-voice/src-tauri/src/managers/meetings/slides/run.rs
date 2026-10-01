//! D1: der Auftrag "Folien erkennen" (Job-Phase `Slides`).
//!
//! [`run`] fuehrt Abtastung, Segmentierung, Zusammenfassung, Bild-Extraktion und
//! Ablage nacheinander aus und meldet Fortschritt, Pause und Stopp ueber den
//! uebergebenen [`JobHandle`] (`job.rs`). Wer ihn aufruft (der Befehl
//! `detect_meeting_slides`, spaeter der Import hinter seiner Warteschlange), haelt
//! den Auftrag im Verzeichnis (`try_start`): ein Auftrag je Besprechung.
//!
//! Fortschritt: `done`/`total` zaehlen ms POSITION IM VIDEO, in beiden Teilen der
//! Phase (Abtastung, danach Bilder), jeweils von vorn. Pausierbar ist die Phase
//! durchgehend: in der Abtastung als Gegendruck auf die Pipe (ffmpeg schlaeft,
//! nichts wird eingefroren), bei den Bildern zwischen zwei Folien.
//!
//! D2 haengt die Texterkennung ein ([`super::ocr`]): zwischen Zusammenfassen nach Hash
//! und dem Ablegen wird je Hash-Gruppe EIN Bild gelesen (Windows-OCR, ~70 ms),
//! danach fuehrt der Textabgleich Folien mit weit entferntem Hash, aber gleichem Text
//! (Webcam, Ueberblendung) zusammen. Ohne Texterkennung (`SlideRun::ocr == None`,
//! andere Plattform, keine OCR-Sprache) bleibt der Lauf der von D1. Die Phase hat
//! damit drei Teile (Abtastung, Lesen, Bilder), jeder mit Fortschritt von vorn und
//! Kontrollpunkt je Bild: pausierbar und stoppbar. Ein Fehler beim Lesen EINER
//! Folie ist kein Fehler des Laufs (fail-open): die Folie bleibt ohne Text und ohne
//! Art, die naechste Wiederholung liest sie nach. Die Bilder der gelesenen Folien
//! liegen bis zum Ablegen in `slides/.cand/` (wird am Ende, auch bei Stopp und
//! Fehler, geraeumt); ein Text, der schon gespeichert ist, wird nicht noch einmal
//! gelesen.
//!
//! Wiederholung: derselbe Lauf noch einmal (nach einem Stopp, Absturz oder auf
//! Wunsch) fuegt nichts doppelt ein. Eine erkannte Folie wird einer vorhandenen
//! zugeordnet, wenn der Hash hoechstens die Schwelle abweicht; dann wachsen nur
//! ihre Zeitbereiche, Bild, Text und "ausgeblendet" bleiben.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use log::{info, warn};

use super::ffmpeg::{self, Flow, SampleHook, SampleTick};
use super::ocr::{self, OcrBackend};
use super::store::{NewSlide, SlideRecord, ORIGIN_VIDEO};
use super::{
    clamp_last_end, group_by_hash, hamming, merge_occurrences, segment, SlideDetectConfig,
    SlideError, SlideGroup, SLIDES_DIR,
};
use crate::managers::meetings::job::{Gate, JobHandle, JobPhase};
use crate::managers::meetings::store::MeetingStore;

/// Was ein Lauf braucht.
pub struct SlideRun<'a> {
    pub store: &'a MeetingStore,
    pub meeting_id: &'a str,
    pub video: &'a Path,
    /// Der Besprechungsordner; die Bilder kommen nach `<meeting_dir>/slides/`.
    pub meeting_dir: &'a Path,
    pub cfg: &'a SlideDetectConfig,
    /// Texterkennung (D2); `None`: Folien ohne Text (andere Plattform, keine
    /// OCR-Sprache, Tests der Erkennung allein).
    pub ocr: Option<&'a dyn OcrBackend>,
    /// Bekommt die PID des Abtast-ffmpeg (Diagnose, Tests); in der App `None`.
    pub pid_out: Option<&'a AtomicU32>,
}

/// Wie der Lauf endete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlideOutcome {
    Done,
    /// Der Nutzer hat gestoppt; was fertig war, bleibt.
    Stopped,
    /// Es gab nichts zu tun (kein Videobild, ffmpeg fehlt): der Code sagt warum.
    Skipped(&'static str),
}

/// Zaehler eines Laufs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideSummary {
    pub outcome: SlideOutcome,
    /// Erkannte Abschnitte (ohne Schwarzbilder).
    pub segments: u32,
    /// Verschiedene Folien dieses Laufs.
    pub groups: u32,
    /// Neu angelegte Folien.
    pub added: u32,
    /// Vorhandene Folien, deren Zeitbereiche gewachsen sind oder deren Bild
    /// nachgeholt wurde.
    pub updated: u32,
    pub duration_ms: Option<u64>,
    /// Kennung der Texterkennung dieses Laufs (`windows-ocr`); `None`: es gab keine.
    pub ocr_engine: Option<&'static str>,
    /// Folien dieses Laufs mit lesbarem Text (Art `text`).
    pub text_slides: u32,
    /// Folien dieses Laufs ohne Text (Art `ohne_text`).
    pub ohne_text: u32,
    /// Hash-Gruppen, deren Text nicht gelesen werden konnte (bleiben ohne Art).
    pub ocr_failed: u32,
}

impl SlideSummary {
    fn empty(outcome: SlideOutcome) -> Self {
        Self {
            outcome,
            segments: 0,
            groups: 0,
            added: 0,
            updated: 0,
            duration_ms: None,
            ocr_engine: None,
            text_slides: 0,
            ohne_text: 0,
            ocr_failed: 0,
        }
    }
}

fn store_error(e: &anyhow::Error) -> SlideError {
    if let Some(rusqlite::Error::SqliteFailure(f, _)) = e.downcast_ref::<rusqlite::Error>() {
        if f.code == rusqlite::ErrorCode::DiskFull {
            return SlideError::DiskFull;
        }
    }
    SlideError::Store(e.to_string())
}

/// Was ein Fehler der Abtastung fuer den Lauf heisst: ein Stopp ist kein Fehler,
/// "keine Videospur" und "ffmpeg fehlt" ueberspringen die Phase (der Besprechung
/// geschieht nichts), alles andere ist ein Fehler des Laufs.
fn sampling_outcome(e: SlideError) -> Result<SlideSummary, SlideError> {
    match e {
        SlideError::Cancelled => Ok(SlideSummary::empty(SlideOutcome::Stopped)),
        SlideError::NoVideoStream | SlideError::FfmpegMissing => {
            info!("slides: Phase uebersprungen ({})", e.code());
            Ok(SlideSummary::empty(SlideOutcome::Skipped(e.code())))
        }
        other => Err(other),
    }
}

/// `slides/0007.jpg` und `slides/0007_t.jpg` (relativ zum Besprechungsordner).
fn relative_paths(number: u32) -> (String, String) {
    (
        format!("{SLIDES_DIR}/{number:04}.jpg"),
        format!("{SLIDES_DIR}/{number:04}_t.jpg"),
    )
}

/// Der naechstliegende, in diesem Lauf noch nicht vergebene vorhandene Eintrag zu `hash`.
fn nearest_existing(
    existing: &[SlideRecord],
    claimed: &[bool],
    hash: u64,
    threshold: u32,
) -> Option<usize> {
    existing
        .iter()
        .enumerate()
        .filter(|(i, _)| !claimed[*i])
        .map(|(i, r)| (hamming(r.dhash, hash), i))
        .filter(|(d, _)| *d <= threshold)
        .min()
        .map(|(_, i)| i)
}

/// Was beim Lesen einer Hash-Gruppe herauskam.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TextRead {
    text: String,
    /// Engine, die gerade gelesen hat; `None`: der Text stammt aus der Datenbank
    /// (nichts zu schreiben).
    engine: Option<&'static str>,
}

/// Ordner fuer die Bilder, die zum Lesen gebraucht werden, bevor feststeht, welche
/// Folien bleiben (`slides/.cand/`). Wird beim Anlegen (Reste eines Absturzes) und
/// beim Verlassen des Laufs geraeumt, auch bei Stopp und Fehler.
struct CandidateDir {
    dir: PathBuf,
}

impl CandidateDir {
    fn new(slides_dir: &Path) -> Self {
        let dir = slides_dir.join(".cand");
        let _ = std::fs::remove_dir_all(&dir);
        Self { dir }
    }

    fn full(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{index:04}.jpg"))
    }

    fn thumb(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{index:04}_t.jpg"))
    }

    fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)
    }

    /// Verschiebt das Kandidatenbild samt Vorschau an den endgueltigen Platz.
    /// `false`, wenn es keines gibt oder das Verschieben scheitert (dann nichts
    /// Halbes am Ziel): der Aufrufer holt das Bild aus dem Video.
    fn take(&self, index: usize, full: &Path, thumb: &Path) -> bool {
        let (from_full, from_thumb) = (self.full(index), self.thumb(index));
        if !from_full.is_file() || !from_thumb.is_file() {
            return false;
        }
        // Erst die Vorschau, dann das Vollbild (wie `extract_frame`).
        if std::fs::rename(&from_thumb, thumb).is_err() {
            return false;
        }
        if std::fs::rename(&from_full, full).is_err() {
            let _ = std::fs::remove_file(thumb);
            return false;
        }
        true
    }
}

impl Drop for CandidateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Liest je Hash-Gruppe EIN Bild. Gespeicherter Text einer vorhandenen Folie wird
/// wiederverwendet; hat die Folie noch keinen (Art fehlt), wird ihr Bild gelesen;
/// sonst kommt das Bild als Kandidat aus dem Video. `Ok(None)`: gestoppt.
/// Ein Fehler beim Lesen einer Folie zaehlt in `ocr_failed` und laesst sie ohne Text.
fn read_groups(
    job: &Arc<JobHandle>,
    ctx: &SlideRun<'_>,
    backend: &dyn OcrBackend,
    groups: &[SlideGroup],
    existing: &[SlideRecord],
    candidates: &CandidateDir,
    stop: &AtomicBool,
    summary: &mut SlideSummary,
) -> Result<Option<Vec<Option<TextRead>>>, SlideError> {
    let mut reads: Vec<Option<TextRead>> = vec![None; groups.len()];
    let unclaimed = vec![false; existing.len()];
    for (gi, group) in groups.iter().enumerate() {
        match job.checkpoint(&|| false) {
            Gate::Go { .. } => {}
            Gate::Stopped | Gate::Cancelled => return Ok(None),
        }
        let stored = nearest_existing(existing, &unclaimed, group.hash, ctx.cfg.hash_threshold)
            .map(|i| &existing[i]);
        if let Some(record) = stored.filter(|r| r.slide.kind.is_some()) {
            reads[gi] = Some(TextRead {
                text: record.slide.ocr_text.clone().unwrap_or_default(),
                engine: None,
            });
            job.advance(group.rep_ms);
            continue;
        }
        let stored_image = stored
            .map(|r| ctx.meeting_dir.join(&r.slide.image_path))
            .filter(|p| p.is_file());
        let source = match stored_image {
            Some(path) => path,
            None => {
                candidates.ensure()?;
                let (full, thumb) = (candidates.full(gi), candidates.thumb(gi));
                match ffmpeg::extract_frame(ctx.video, group.rep_ms, &full, Some(&thumb), stop) {
                    Ok(()) => full,
                    Err(SlideError::Cancelled) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
        };
        match backend.recognize(&source) {
            Ok(read) => {
                reads[gi] = Some(TextRead {
                    text: read.text,
                    engine: Some(read.engine),
                });
            }
            Err(e) => {
                warn!("slides: Text einer Folie nicht gelesen ({e})");
                summary.ocr_failed += 1;
            }
        }
        job.advance(group.rep_ms);
    }
    Ok(Some(reads))
}

/// Schreibt den frisch gelesenen Text einer Folie; ein Fehler beim Speichern ist
/// keiner des Laufs (die Folie bleibt ohne Art, die Wiederholung holt es nach).
fn store_text(ctx: &SlideRun<'_>, slide_id: &str, read: &TextRead) -> bool {
    let Some(engine) = read.engine else {
        return false;
    };
    match ctx.store.slide_set_text(
        slide_id,
        Some(&read.text),
        Some(engine),
        Some(ocr::kind_of(&read.text)),
    ) {
        Ok(found) => found,
        Err(e) => {
            warn!("slides: Text der Folie {slide_id} nicht gespeichert ({e})");
            false
        }
    }
}

/// Wie [`run`], aber `after_slide` wird nach jeder verarbeiteten Folie gerufen
/// (Index ab 0): in der App ein leerer Haken, in Tests ein Stopp an einer
/// bestimmten Stelle.
pub fn run_with(
    job: &Arc<JobHandle>,
    ctx: &SlideRun<'_>,
    after_slide: &dyn Fn(usize),
) -> Result<SlideSummary, SlideError> {
    if !ctx.video.is_file() {
        return Err(SlideError::VideoMissing);
    }
    let stop = job.stop_flag();
    job.begin_phase_ex(JobPhase::Slides, 0, true);

    // 1. Abtasten. Der Haken laeuft im Lesethread: Fortschritt, dann der
    // Kontrollpunkt (bei Pause blockiert er, ffmpeg wartet an der vollen Pipe).
    let hook: SampleHook = {
        let job = Arc::clone(job);
        let total_known = AtomicBool::new(false);
        Arc::new(move |tick: SampleTick| -> Flow {
            if let Some(duration) = tick.duration_ms {
                if !total_known.swap(true, Ordering::AcqRel) {
                    job.set_total(duration);
                }
            }
            job.advance(tick.ms);
            match job.checkpoint(&|| false) {
                Gate::Go { .. } => Flow::Continue,
                Gate::Stopped | Gate::Cancelled => Flow::Stop,
            }
        })
    };
    let command = ffmpeg::sample_command(ctx.video, ctx.cfg);
    let sampled = match ffmpeg::run_sampler(command, ctx.cfg, &stop, hook, ctx.pid_out) {
        Ok(sampled) => sampled,
        Err(e) => return sampling_outcome(e),
    };
    let duration_ms = sampled.info.duration_ms;

    // 2./3. Segmentieren und nach Hash zusammenfassen (rein, schnell).
    let mut segments = segment(&sampled.samples, ctx.cfg.sample_fps, ctx.cfg);
    clamp_last_end(&mut segments, duration_ms);
    let kept = segments.iter().filter(|s| !s.black).count() as u32;
    let hash_groups: Vec<SlideGroup> = group_by_hash(&segments, ctx.cfg);
    let mut summary = SlideSummary {
        outcome: SlideOutcome::Done,
        segments: kept,
        groups: hash_groups.len() as u32,
        added: 0,
        updated: 0,
        duration_ms,
        ocr_engine: ctx.ocr.map(|o| o.id()),
        text_slides: 0,
        ohne_text: 0,
        ocr_failed: 0,
    };

    // Ordner, Reste, vorhandene Folien.
    let slides_dir = ctx.meeting_dir.join(SLIDES_DIR);
    std::fs::create_dir_all(&slides_dir)?;
    let leftovers = ffmpeg::clean_partial_files(&slides_dir);
    if leftovers > 0 {
        warn!("slides: {leftovers} halbfertige Bilder eines frueheren Laufs entfernt");
    }
    let total_ms = duration_ms.unwrap_or_else(|| hash_groups.last().map_or(0, |g| g.rep_ms));
    let existing: Vec<SlideRecord> = ctx
        .store
        .slide_records(ctx.meeting_id)
        .map_err(|e| store_error(&e))?
        .into_iter()
        .filter(|r| r.slide.origin == ORIGIN_VIDEO)
        .collect();

    // 3b. Lesen (D2): je Hash-Gruppe ein Bild, danach Textabgleich der Gruppen.
    let candidates = CandidateDir::new(&slides_dir);
    let mut hash_group_of = vec![0usize; segments.len()];
    for (gi, g) in hash_groups.iter().enumerate() {
        for &i in &g.segments {
            hash_group_of[i] = gi;
        }
    }
    let mut reads: Vec<Option<TextRead>> = vec![None; hash_groups.len()];
    let groups: Vec<SlideGroup> = match ctx.ocr {
        None => hash_groups.clone(),
        Some(backend) => {
            info!(
                "slides: Texterkennung {} ({})",
                backend.id(),
                backend.language()
            );
            job.begin_phase_ex(JobPhase::Slides, total_ms, true);
            match read_groups(
                job,
                ctx,
                backend,
                &hash_groups,
                &existing,
                &candidates,
                &stop,
                &mut summary,
            )? {
                Some(done) => reads = done,
                None => {
                    summary.outcome = SlideOutcome::Stopped;
                    return Ok(summary);
                }
            }
            // Jeder Abschnitt traegt den Text seiner Hash-Gruppe; der Abgleich
            // vergleicht die ersten Abschnitte der Folien.
            let texts: Vec<Option<String>> = (0..segments.len())
                .map(|i| reads[hash_group_of[i]].as_ref().map(|r| r.text.clone()))
                .collect();
            ocr::group_with_text(&segments, &texts, ctx.cfg)
        }
    };
    summary.groups = groups.len() as u32;
    for group in &groups {
        if let Some(read) = &reads[hash_group_of[group.first]] {
            match ocr::kind_of(&read.text) {
                ocr::KIND_TEXT => summary.text_slides += 1,
                _ => summary.ohne_text += 1,
            }
        }
    }

    // 4. Bilder und Zeilen.
    job.begin_phase_ex(JobPhase::Slides, total_ms, true);
    let mut claimed = vec![false; existing.len()];

    for (index, group) in groups.iter().enumerate() {
        match job.checkpoint(&|| false) {
            Gate::Go { .. } => {}
            Gate::Stopped | Gate::Cancelled => {
                summary.outcome = SlideOutcome::Stopped;
                return Ok(summary);
            }
        }
        match nearest_existing(&existing, &claimed, group.hash, ctx.cfg.hash_threshold) {
            Some(found) => {
                claimed[found] = true;
                let record = &existing[found];
                let merged = merge_occurrences(&record.slide.occurrences, &group.occurrences);
                let mut changed = false;
                if merged != record.slide.occurrences {
                    ctx.store
                        .slide_set_occurrences(&record.slide.id, &merged)
                        .map_err(|e| store_error(&e))?;
                    changed = true;
                }
                // Bild weg (von Hand geloescht)? Aus dem Video nachholen.
                let image = ctx.meeting_dir.join(&record.slide.image_path);
                if !image.is_file() {
                    let thumb = record
                        .slide
                        .thumb_path
                        .as_ref()
                        .map(|p| ctx.meeting_dir.join(p));
                    match ffmpeg::extract_frame(
                        ctx.video,
                        group.rep_ms,
                        &image,
                        thumb.as_deref(),
                        &stop,
                    ) {
                        Ok(()) => changed = true,
                        Err(SlideError::Cancelled) => {
                            summary.outcome = SlideOutcome::Stopped;
                            return Ok(summary);
                        }
                        Err(e) => return Err(e),
                    }
                }
                // Text nachtragen: die Folie hat noch keine Art (OCR fehlte oder scheiterte).
                if record.slide.kind.is_none() {
                    if let Some(read) = &reads[hash_group_of[group.first]] {
                        changed |= store_text(ctx, &record.slide.id, read);
                    }
                }
                if changed {
                    summary.updated += 1;
                }
            }
            None => {
                let number = ctx
                    .store
                    .slide_next_number(ctx.meeting_id)
                    .map_err(|e| store_error(&e))?;
                let (image_rel, thumb_rel) = relative_paths(number);
                let (image, thumb): (PathBuf, PathBuf) = (
                    ctx.meeting_dir.join(&image_rel),
                    ctx.meeting_dir.join(&thumb_rel),
                );
                // Das Bild liegt schon als Kandidat da, wenn gelesen wurde.
                let placed = ctx.ocr.is_some()
                    && candidates.take(hash_group_of[group.first], &image, &thumb);
                if !placed {
                    match ffmpeg::extract_frame(
                        ctx.video,
                        group.rep_ms,
                        &image,
                        Some(&thumb),
                        &stop,
                    ) {
                        Ok(()) => {}
                        Err(SlideError::Cancelled) => {
                            summary.outcome = SlideOutcome::Stopped;
                            return Ok(summary);
                        }
                        Err(e) => return Err(e),
                    }
                }
                let inserted = ctx.store.slide_insert(&NewSlide {
                    meeting_id: ctx.meeting_id.to_string(),
                    number: Some(number),
                    origin: ORIGIN_VIDEO,
                    image_path: image_rel,
                    thumb_path: Some(thumb_rel),
                    dhash: group.hash,
                    occurrences: group.occurrences.clone(),
                });
                match inserted {
                    Ok(slide) => {
                        if let Some(read) = &reads[hash_group_of[group.first]] {
                            store_text(ctx, &slide.id, read);
                        }
                    }
                    Err(e) => {
                        // Keine Zeile ohne Datei und keine Datei ohne Zeile.
                        let _ = std::fs::remove_file(&image);
                        let _ = std::fs::remove_file(&thumb);
                        return Err(store_error(&e));
                    }
                }
                summary.added += 1;
            }
        }
        job.advance(group.rep_ms);
        after_slide(index);
    }
    job.advance(total_ms);
    info!(
        "slides: {} Folien erkannt ({} neu, {} aktualisiert; Text {}, ohne Text {}, nicht gelesen {})",
        summary.groups,
        summary.added,
        summary.updated,
        summary.text_slides,
        summary.ohne_text,
        summary.ocr_failed
    );
    Ok(summary)
}

/// Fuehrt die Folienerkennung als Auftrag aus (siehe Modulkopf).
pub fn run(job: &Arc<JobHandle>, ctx: &SlideRun<'_>) -> Result<SlideSummary, SlideError> {
    run_with(job, ctx, &|_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::job::{JobHandle, MeetingJobs};
    use crate::managers::meetings::recorder::MeetingEvent;
    #[cfg(windows)]
    use crate::managers::meetings::slides::test_support::process_alive;
    use crate::managers::meetings::slides::test_support::{
        ffmpeg_available, gray_hash_of_image, serial, shared_slide_video, SlideVideo,
    };
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus};
    use std::sync::{Arc, Mutex};

    struct Fx {
        /// Haelt die Reihenfolge der Tests mit Kindprozessen (siehe `test_support::serial`).
        _serial: std::sync::MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
        store: MeetingStore,
        meeting: String,
        video: SlideVideo,
    }

    impl Fx {
        fn meeting_dir(&self) -> PathBuf {
            self.dir.path().join("meeting")
        }

        fn ctx<'a>(&'a self, cfg: &'a SlideDetectConfig, meeting_dir: &'a Path) -> SlideRun<'a> {
            SlideRun {
                store: &self.store,
                meeting_id: &self.meeting,
                video: &self.video.path,
                meeting_dir,
                cfg,
                ocr: None,
                pid_out: None,
            }
        }
    }

    /// Eine Besprechung und ein 30-s-Video mit drei Folien und bewegtem Feld; `None`: ffmpeg fehlt.
    fn fixture() -> Option<Fx> {
        if !ffmpeg_available() {
            eprintln!("ffmpeg fehlt - Test uebersprungen");
            return None;
        }
        let serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("Vortrag", MeetingSource::Import, None)
            .unwrap()
            .id;
        store.set_status(&meeting, MeetingStatus::Ready).unwrap();
        let video = shared_slide_video();
        Some(Fx {
            _serial: serial,
            dir,
            store,
            meeting,
            video,
        })
    }

    type Events = Arc<Mutex<Vec<MeetingEvent>>>;

    fn job(meeting: &str) -> (Arc<JobHandle>, Events) {
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let handle = JobHandle::new(meeting, Arc::new(move |e| sink.lock().unwrap().push(e)));
        (handle, events)
    }

    fn progress(events: &Events) -> Vec<(JobPhase, u64, u64)> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                MeetingEvent::Progress {
                    phase, done, total, ..
                } => Some((*phase, *done, *total)),
                _ => None,
            })
            .collect()
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// (b) Akzeptanz: 30-s-Video, drei Folien, ein bewegtes Feld -> 3 Folien +-1 s.
    #[test]
    fn a_thirty_second_video_with_three_slides_and_a_moving_field_yields_three_slides() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, events) = job(&f.meeting);
        let summary = run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!((summary.segments, summary.groups, summary.added), (3, 3, 3));
        assert_eq!(summary.duration_ms, Some(30_000));

        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides.len(), 3);
        for (i, slide) in slides.iter().enumerate() {
            assert_eq!(slide.number, i as u32 + 1);
            assert_eq!(slide.origin, "video");
            assert_eq!(slide.image_path, format!("slides/{:04}.jpg", i + 1));
            assert_eq!(
                slide.thumb_path.as_deref(),
                Some(format!("slides/{:04}_t.jpg", i + 1).as_str())
            );
            let want_start = i as i64 * 10_000;
            let got = slide.occurrences[0];
            assert_eq!(slide.occurrences.len(), 1);
            assert!(
                (got.start_ms as i64 - want_start).abs() <= 1_000,
                "Folie {} beginnt bei {} ms, erwartet {want_start} +-1000",
                i + 1,
                got.start_ms
            );
            assert!(
                (got.end_ms as i64 - (want_start + 10_000)).abs() <= 1_000,
                "Folie {} endet bei {} ms",
                i + 1,
                got.end_ms
            );
            let full = std::fs::metadata(dir.join(&slide.image_path))
                .unwrap()
                .len();
            let thumb = std::fs::metadata(dir.join(slide.thumb_path.as_ref().unwrap()))
                .unwrap()
                .len();
            assert!(
                full > 1_000 && thumb > 300 && thumb < full,
                "Vollbild {full} B, Vorschau {thumb} B"
            );
        }
        assert_eq!(
            files(&dir.join(SLIDES_DIR)),
            vec![
                "0001.jpg",
                "0001_t.jpg",
                "0002.jpg",
                "0002_t.jpg",
                "0003.jpg",
                "0003_t.jpg"
            ],
            "keine halbfertigen Bilder"
        );

        // Fortschritt: nur die Phase Slides, die Groesse ist die Videodauer, nie rueckwaerts je Teil.
        let seen = progress(&events);
        assert!(!seen.is_empty());
        assert!(seen.iter().all(|(p, _, _)| *p == JobPhase::Slides));
        assert!(
            seen.iter().any(|(_, _, total)| *total == 30_000),
            "Groesse = Videodauer"
        );
        assert_eq!(
            seen.last().map(|(_, d, t)| (*d, *t)),
            Some((30_000, 30_000)),
            "endet bei 100 %"
        );
    }

    /// Die gespeicherten Bilder zeigen die richtige Folie: ihr Hash (auf dem Weg der
    /// Abtastung gebildet) liegt nahe an dem der Gruppe.
    #[test]
    fn the_stored_images_show_the_slides_they_belong_to() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        let records = f.store.slide_records(&f.meeting).unwrap();
        assert_eq!(records.len(), 3);
        for record in &records {
            let d = hamming(
                gray_hash_of_image(&dir.join(&record.slide.image_path)),
                record.dhash,
            );
            assert!(
                d <= 6,
                "Bild {} weicht um {d} Bit vom erkannten Hash ab",
                record.slide.number
            );
        }
        // Und die drei Bilder sind verschieden.
        let hashes: Vec<u64> = records
            .iter()
            .map(|r| gray_hash_of_image(&dir.join(&r.slide.image_path)))
            .collect();
        assert!(hamming(hashes[0], hashes[1]) > 8 && hamming(hashes[1], hashes[2]) > 8);
    }

    /// Wiederholung: nichts doppelt, Ausblenden bleibt, Bild und Text bleiben.
    #[test]
    fn a_rerun_adds_nothing_and_keeps_hidden_flags() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        let first = f.store.slides_list(&f.meeting).unwrap();
        f.store.slide_set_hidden(&first[1].id, true).unwrap();
        f.store
            .slide_set_text(
                &first[0].id,
                Some("Größe"),
                Some("windows-ocr"),
                Some("text"),
            )
            .unwrap();
        let image_stamp = std::fs::metadata(dir.join(&first[0].image_path))
            .unwrap()
            .modified()
            .unwrap();

        let (again, _) = job(&f.meeting);
        let summary = run(&again, &f.ctx(&cfg, &dir)).unwrap();
        assert_eq!(
            (summary.added, summary.updated, summary.groups),
            (0, 0, 3),
            "{summary:?}"
        );
        let second = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(second.len(), 3);
        assert_eq!(second[0].ocr_text.as_deref(), Some("Größe"));
        assert!(second[1].hidden, "Ausgeblendet bleibt");
        assert_eq!(
            second
                .iter()
                .map(|s| (&s.id, &s.occurrences))
                .collect::<Vec<_>>(),
            first
                .iter()
                .map(|s| (&s.id, &s.occurrences))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            std::fs::metadata(dir.join(&first[0].image_path))
                .unwrap()
                .modified()
                .unwrap(),
            image_stamp,
            "vorhandene Bilder werden nicht neu geschrieben"
        );
        assert_eq!(files(&dir.join(SLIDES_DIR)).len(), 6);
    }

    /// Ein von Hand geloeschtes Bild wird bei der Wiederholung nachgeholt.
    #[test]
    fn a_missing_image_is_fetched_again_on_a_rerun() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        std::fs::remove_file(dir.join("slides/0002.jpg")).unwrap();
        let (again, _) = job(&f.meeting);
        let summary = run(&again, &f.ctx(&cfg, &dir)).unwrap();
        assert_eq!((summary.added, summary.updated), (0, 1));
        assert!(dir.join("slides/0002.jpg").is_file());
        assert_eq!(f.store.slides_list(&f.meeting).unwrap().len(), 3);
    }

    /// Stopp zwischen zwei Folien: das Fertige bleibt, eine Wiederholung holt den Rest nach.
    #[test]
    fn a_stop_between_two_slides_keeps_the_finished_ones_and_a_rerun_completes_them() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        let stopper = Arc::clone(&handle);
        let summary = run_with(&handle, &f.ctx(&cfg, &dir), &move |index| {
            if index == 0 {
                stopper.stop().unwrap();
            }
        })
        .unwrap();
        assert_eq!(summary.outcome, SlideOutcome::Stopped);
        assert_eq!(summary.added, 1);
        let left = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(left.len(), 1, "nur die fertige Folie ist da");
        assert_eq!(
            files(&dir.join(SLIDES_DIR)),
            vec!["0001.jpg", "0001_t.jpg"],
            "keine Reste"
        );

        let (again, _) = job(&f.meeting);
        let summary = run(&again, &f.ctx(&cfg, &dir)).unwrap();
        assert_eq!((summary.outcome, summary.added), (SlideOutcome::Done, 2));
        let all = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, left[0].id, "die erste Folie blieb dieselbe");
        assert_eq!(
            all.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    /// Hilfsthread der Pause-Tests: wartet, bis der Lauf an einem Kontrollpunkt steht,
    /// prueft, dass er dort bleibt, und gibt ihn dann in JEDEM Fall frei (`finish`):
    /// ein gescheiterter Helfer darf den Test nie an einem angehaltenen Lauf haengen
    /// lassen. Die Feststellungen kommen als Ergebnis und werden nach dem `join` bewertet.
    fn release_when_paused(
        handle: Arc<JobHandle>,
        finish: impl FnOnce(&JobHandle) + Send + 'static,
    ) -> std::thread::JoinHandle<Result<(), String>> {
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while !handle.is_paused() && started.elapsed() < std::time::Duration::from_secs(90) {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let mut problems: Vec<&str> = Vec::new();
            if handle.is_paused() {
                std::thread::sleep(std::time::Duration::from_millis(300));
                if !handle.is_paused() {
                    problems.push("der Lauf blieb nicht angehalten");
                }
            } else {
                problems.push("der Lauf hielt nie an");
            }
            finish(&handle);
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("; "))
            }
        })
    }

    /// (d) Stopp waehrend der Abtastung (Pause, dann Stopp): kein ffmpeg bleibt zurueck, nichts geschrieben.
    #[cfg(windows)]
    #[test]
    fn a_stop_during_sampling_leaves_no_ffmpeg_process_and_nothing_on_disk() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let pid = AtomicU32::new(0);
        let (handle, _) = job(&f.meeting);
        // Pause verlangt, bevor der Lauf beginnt: er haelt am ersten Kontrollpunkt, ffmpeg wartet an der Pipe.
        handle.begin_phase_ex(JobPhase::Slides, 0, true);
        handle.pause().unwrap();
        let waiter = release_when_paused(Arc::clone(&handle), |h| {
            let _ = h.stop();
        });
        let started = std::time::Instant::now();
        let ctx = SlideRun {
            pid_out: Some(&pid),
            ..f.ctx(&cfg, &dir)
        };
        let summary = run(&handle, &ctx).unwrap();
        assert_eq!(waiter.join().unwrap(), Ok(()));
        assert_eq!(summary.outcome, SlideOutcome::Stopped);
        assert!(started.elapsed() < std::time::Duration::from_secs(90));
        assert!(f.store.slides_list(&f.meeting).unwrap().is_empty());
        assert!(
            files(&dir.join(SLIDES_DIR)).is_empty(),
            "nichts geschrieben"
        );
        let pid = pid.load(Ordering::Acquire);
        assert_ne!(pid, 0, "ffmpeg wurde gestartet");
        assert!(
            !process_alive(pid),
            "ffmpeg {pid} ist nach dem Stopp beendet"
        );
    }

    /// Pause waehrend der Abtastung haelt den Lauf an, Fortsetzen laesst ihn fertig werden.
    #[test]
    fn a_pause_holds_the_run_and_resume_lets_it_finish() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        handle.begin_phase_ex(JobPhase::Slides, 0, true);
        handle.pause().unwrap();
        let waiter = release_when_paused(Arc::clone(&handle), |h| {
            let _ = h.resume();
        });
        let summary = run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        assert_eq!(waiter.join().unwrap(), Ok(()));
        assert_eq!((summary.outcome, summary.added), (SlideOutcome::Done, 3));
    }

    /// Audio-only: uebersprungen, kein Fehler, nichts geschrieben.
    #[test]
    fn an_audio_only_file_is_skipped_without_writing_anything() {
        let Some(f) = fixture() else { return };
        let audio = f.dir.path().join("nur-ton.mp3");
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:d=3",
            ])
            .arg(&audio)
            .status()
            .unwrap()
            .success();
        assert!(ok, "Testton erzeugt");
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        let ctx = SlideRun {
            video: &audio,
            ..f.ctx(&cfg, &dir)
        };
        let summary = run(&handle, &ctx).unwrap();
        assert_eq!(summary.outcome, SlideOutcome::Skipped("slides_no_video"));
        assert!(f.store.slides_list(&f.meeting).unwrap().is_empty());
        assert!(!dir.join(SLIDES_DIR).exists(), "auch kein leerer Ordner");
    }

    #[test]
    fn a_missing_video_file_is_an_error_with_a_code() {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("x", MeetingSource::Import, None)
            .unwrap()
            .id;
        let (handle, _) = job(&meeting);
        let cfg = SlideDetectConfig::default();
        let gone = dir.path().join("weg.mp4");
        let md = dir.path().join("m");
        let ctx = SlideRun {
            store: &store,
            meeting_id: &meeting,
            video: &gone,
            meeting_dir: &md,
            cfg: &cfg,
            ocr: None,
            pid_out: None,
        };
        assert_eq!(run(&handle, &ctx).unwrap_err(), SlideError::VideoMissing);
    }

    /// Besprechung waehrend des Laufs geloescht: die Zeile wird abgelehnt, die Bilder gehen wieder.
    #[test]
    fn a_failed_insert_removes_the_images_it_just_wrote() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        f.store.soft_delete_meeting(&f.meeting).unwrap();
        let (handle, _) = job(&f.meeting);
        let err = run(&handle, &f.ctx(&cfg, &dir)).unwrap_err();
        assert!(
            matches!(err, SlideError::Store(ref m) if m == "meeting_not_found"),
            "{err:?}"
        );
        assert!(
            files(&dir.join(SLIDES_DIR)).is_empty(),
            "keine Datei ohne Zeile"
        );
    }

    /// Reste eines abgebrochenen Laufs (`*.part.jpg`) werden vor dem naechsten Lauf geraeumt.
    #[test]
    fn leftover_partial_files_do_not_survive_a_run() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        std::fs::create_dir_all(dir.join(SLIDES_DIR)).unwrap();
        std::fs::write(dir.join("slides/0001.part.jpg"), b"halb").unwrap();
        let (handle, _) = job(&f.meeting);
        run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        assert!(files(&dir.join(SLIDES_DIR))
            .iter()
            .all(|n| !n.contains(".part")));
    }

    // -- D2: Texterkennung im Lauf -------------------------------------------------

    use crate::managers::meetings::slides::ocr::{OcrError, OcrText};
    use std::sync::atomic::AtomicUsize;

    /// Ein Texterkenner nach Drehbuch: der n-te Aufruf liefert den n-ten Eintrag
    /// (danach ein leerer Text); `on_call` laeuft vor der Antwort (Stopp, Pause).
    struct ScriptedOcr {
        script: Mutex<Vec<Result<String, OcrError>>>,
        calls: AtomicUsize,
        on_call: Box<dyn Fn(usize) + Send + Sync>,
    }

    impl ScriptedOcr {
        fn new(script: Vec<Result<&str, OcrError>>) -> Self {
            Self::with_hook(script, |_| {})
        }

        fn with_hook(
            script: Vec<Result<&str, OcrError>>,
            on_call: impl Fn(usize) + Send + Sync + 'static,
        ) -> Self {
            Self {
                script: Mutex::new(script.into_iter().map(|r| r.map(str::to_string)).collect()),
                calls: AtomicUsize::new(0),
                on_call: Box::new(on_call),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }
    }

    impl OcrBackend for ScriptedOcr {
        fn id(&self) -> &'static str {
            "fake-ocr"
        }

        fn language(&self) -> String {
            "de-DE".to_string()
        }

        fn recognize(&self, image: &Path) -> Result<OcrText, OcrError> {
            assert!(
                image.is_file(),
                "das Bild muss beim Lesen da sein: {image:?}"
            );
            let n = self.calls.fetch_add(1, Ordering::AcqRel);
            (self.on_call)(n);
            let next = {
                let script = self.script.lock().unwrap();
                script.get(n).cloned().unwrap_or(Ok(String::new()))
            };
            next.map(|text| OcrText {
                text,
                engine: "fake-ocr",
                language: "de-DE".to_string(),
            })
        }
    }

    const AGENDA: &str = "Agenda Ergebnisse Kundenzufriedenheit Risiken";
    const AGENDA_PLUS: &str = "Agenda Ergebnisse Kundenzufriedenheit Risiken Ausblick";
    const OTHER: &str = "Zeitplan Einfuehrung Pilotphase Rollout";

    fn run_with_ocr(
        f: &Fx,
        ocr: &dyn OcrBackend,
        dir: &Path,
    ) -> (SlideSummary, Arc<JobHandle>, Events) {
        let cfg = SlideDetectConfig::default();
        let (handle, events) = job(&f.meeting);
        let ctx = SlideRun {
            ocr: Some(ocr),
            ..f.ctx(&cfg, dir)
        };
        let summary = run(&handle, &ctx).unwrap();
        (summary, handle, events)
    }

    /// Ohne Texterkennung ist der Lauf der von D1: keine Art, kein Text, kein Kandidatenordner.
    #[test]
    fn without_a_backend_the_run_is_the_d1_run() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        let summary = run(&handle, &f.ctx(&cfg, &dir)).unwrap();
        assert_eq!((summary.groups, summary.added), (3, 3));
        assert_eq!(
            (
                summary.ocr_engine,
                summary.text_slides,
                summary.ohne_text,
                summary.ocr_failed
            ),
            (None, 0, 0, 0)
        );
        for slide in f.store.slides_list(&f.meeting).unwrap() {
            assert_eq!(
                (slide.ocr_text, slide.ocr_engine, slide.kind),
                (None, None, None)
            );
        }
        assert!(!dir.join(SLIDES_DIR).join(".cand").exists());
    }

    /// Text-Dubletten werden zusammengefuehrt (die Hashes liegen weit auseinander),
    /// eine Folie mit kurzem Text bekommt `ohne_text`; Engine und Art stehen in der Zeile.
    #[test]
    fn equal_text_merges_slides_and_a_short_text_is_ohne_text() {
        let Some(f) = fixture() else { return };
        let dir = f.meeting_dir();
        let fake = ScriptedOcr::new(vec![Ok(AGENDA), Ok(AGENDA_PLUS), Ok("Foto")]);
        let (summary, _, _) = run_with_ocr(&f, &fake, &dir);
        assert_eq!(fake.calls(), 3, "je Hash-Gruppe ein Bild");
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!((summary.groups, summary.added), (2, 2), "{summary:?}");
        assert_eq!(
            (
                summary.ocr_engine,
                summary.text_slides,
                summary.ohne_text,
                summary.ocr_failed
            ),
            (Some("fake-ocr"), 1, 1, 0)
        );
        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].ocr_text.as_deref(), Some(AGENDA));
        assert_eq!(slides[0].ocr_engine.as_deref(), Some("fake-ocr"));
        assert_eq!(slides[0].kind.as_deref(), Some("text"));
        assert_eq!(
            slides[0].occurrences.len(),
            1,
            "beide Vorkommen aneinander: ein Bereich"
        );
        assert!(
            slides[0].occurrences[0].end_ms >= 19_000,
            "die Folie ist zu Beginn UND in der Mitte zu sehen: {:?}",
            slides[0].occurrences
        );
        assert_eq!(slides[1].ocr_text.as_deref(), Some("Foto"));
        assert_eq!(slides[1].kind.as_deref(), Some("ohne_text"));
        assert_eq!(slides[1].number, 2);
        assert_eq!(slides[1].image_path, "slides/0002.jpg");
        assert_eq!(
            files(&dir.join(SLIDES_DIR)),
            vec!["0001.jpg", "0001_t.jpg", "0002.jpg", "0002_t.jpg"],
            "die Kandidatenbilder (auch das der zusammengefuehrten Folie) sind weg"
        );
        // Das Bild der Folie ist das gelesene: es liegt gross genug da.
        assert!(
            std::fs::metadata(dir.join("slides/0002.jpg"))
                .unwrap()
                .len()
                > 1_000
        );
    }

    /// fail-open: ein Fehler an EINER Folie stoppt den Lauf nicht; die Folie bleibt ohne Art.
    /// Die Wiederholung liest nur sie nach (die anderen haben ihren Text).
    #[test]
    fn a_failing_slide_does_not_stop_the_run() {
        let Some(f) = fixture() else { return };
        let dir = f.meeting_dir();
        let fake = ScriptedOcr::new(vec![
            Ok(AGENDA),
            Err(OcrError::Image("kaputt".into())),
            Ok(OTHER),
        ]);
        let (summary, _, _) = run_with_ocr(&f, &fake, &dir);
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(
            (summary.groups, summary.added, summary.ocr_failed),
            (3, 3, 1)
        );
        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides[0].kind.as_deref(), Some("text"));
        assert_eq!(
            slides[1].kind, None,
            "ohne Art: die Wiederholung versucht es erneut"
        );
        assert_eq!(slides[1].ocr_text, None);
        assert_eq!(slides[2].kind.as_deref(), Some("text"));
        assert_eq!(files(&dir.join(SLIDES_DIR)).len(), 6, "alle drei Bilder da");
    }

    #[test]
    fn a_rerun_reads_nothing_twice_and_fills_gaps() {
        let Some(f) = fixture() else { return };
        let dir = f.meeting_dir();
        let first = ScriptedOcr::new(vec![
            Ok(AGENDA),
            Err(OcrError::Engine("kurz weg".into())),
            Ok(OTHER),
        ]);
        run_with_ocr(&f, &first, &dir);
        let before = f.store.slides_list(&f.meeting).unwrap();
        let image_stamp = std::fs::metadata(dir.join(&before[1].image_path))
            .unwrap()
            .modified()
            .unwrap();

        // Zweiter Lauf: nur die Folie ohne Art wird gelesen (aus ihrem gespeicherten Bild).
        let second = ScriptedOcr::new(vec![Ok("Nachgeholter Text der zweiten Folie")]);
        let (summary, _, _) = run_with_ocr(&f, &second, &dir);
        assert_eq!(second.calls(), 1, "nichts doppelt gelesen");
        assert_eq!(
            (summary.added, summary.updated, summary.ocr_failed),
            (0, 1, 0),
            "{summary:?}"
        );
        let after = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(
            after[0].ocr_text.as_deref(),
            Some(AGENDA),
            "gespeicherter Text bleibt"
        );
        assert_eq!(
            after[1].ocr_text.as_deref(),
            Some("Nachgeholter Text der zweiten Folie")
        );
        assert_eq!(after[1].kind.as_deref(), Some("text"));
        assert_eq!(after[1].ocr_engine.as_deref(), Some("fake-ocr"));
        assert_eq!(
            std::fs::metadata(dir.join(&after[1].image_path))
                .unwrap()
                .modified()
                .unwrap(),
            image_stamp,
            "das Bild wird nicht neu geschrieben"
        );

        // Dritter Lauf: alles hat Text, es wird nichts mehr gelesen und nichts geaendert.
        let third = ScriptedOcr::new(vec![]);
        let (summary, _, _) = run_with_ocr(&f, &third, &dir);
        assert_eq!(third.calls(), 0);
        assert_eq!((summary.added, summary.updated), (0, 0));
        assert_eq!(f.store.slides_list(&f.meeting).unwrap(), after);
    }

    /// Stopp mitten im Lesen: keine Zeile, kein Bild, kein Kandidatenordner; die Wiederholung klappt.
    #[test]
    fn a_stop_while_reading_leaves_no_candidates_and_no_rows() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        let stopper = Arc::clone(&handle);
        let fake = ScriptedOcr::with_hook(vec![Ok(AGENDA), Ok(OTHER)], move |n| {
            if n == 0 {
                stopper.stop().unwrap();
            }
        });
        let ctx = SlideRun {
            ocr: Some(&fake),
            ..f.ctx(&cfg, &dir)
        };
        let summary = run(&handle, &ctx).unwrap();
        assert_eq!(summary.outcome, SlideOutcome::Stopped);
        assert_eq!(fake.calls(), 1, "nach dem Stopp wird nichts mehr gelesen");
        assert!(f.store.slides_list(&f.meeting).unwrap().is_empty());
        assert!(
            files(&dir.join(SLIDES_DIR)).is_empty(),
            "weder Bilder noch .cand"
        );

        let again = ScriptedOcr::new(vec![Ok(AGENDA), Ok(OTHER), Ok(OTHER)]);
        let (summary, _, _) = run_with_ocr(&f, &again, &dir);
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(
            f.store.slides_list(&f.meeting).unwrap().len(),
            2,
            "OTHER zweimal: zusammengefuehrt"
        );
    }

    /// Pause mitten im Lesen haelt den Lauf an, Fortsetzen laesst ihn fertig werden.
    #[test]
    fn a_pause_while_reading_holds_the_run_and_resume_lets_it_finish() {
        let Some(f) = fixture() else { return };
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        let pauser = Arc::clone(&handle);
        let fake = ScriptedOcr::with_hook(
            vec![Ok(AGENDA), Ok(OTHER), Ok("Dritte Folie mit Text")],
            move |n| {
                if n == 0 {
                    pauser.pause().unwrap();
                }
            },
        );
        let waiter = release_when_paused(Arc::clone(&handle), |h| {
            let _ = h.resume();
        });
        let ctx = SlideRun {
            ocr: Some(&fake),
            ..f.ctx(&cfg, &dir)
        };
        let summary = run(&handle, &ctx).unwrap();
        assert_eq!(waiter.join().unwrap(), Ok(()));
        assert_eq!((summary.outcome, summary.added), (SlideOutcome::Done, 3));
        assert_eq!(fake.calls(), 3);
    }

    /// Die ganze Kette mit der ECHTEN Windows-OCR: ein Video aus der PNG-Folie
    /// (Umlaute, Euro) -> Lauf -> Text, Engine und Art in der Zeile.
    #[cfg(windows)]
    #[test]
    fn the_real_windows_ocr_reads_a_slide_out_of_a_video() {
        let Some(f) = fixture() else { return };
        let Ok(backend) = ocr::default_backend(ocr::PREFERRED_LANGUAGES) else {
            eprintln!("keine Windows-OCR-Sprache - Test uebersprungen");
            return;
        };
        let png = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/managers/meetings/slides/fixtures/ocr_slide.png");
        let video = f.dir.path().join("folie.mp4");
        let made = ["libx264", "mpeg4"].iter().any(|codec| {
            std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y", "-loop", "1"])
                .args(["-framerate", "5", "-i"])
                .arg(&png)
                .args([
                    "-t", "6", "-c:v", codec, "-threads", "2", "-pix_fmt", "yuv420p",
                ])
                .arg(&video)
                .status()
                .is_ok_and(|s| s.success())
        });
        assert!(made, "Video aus der PNG-Folie erzeugt");
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let (handle, _) = job(&f.meeting);
        let ctx = SlideRun {
            video: &video,
            ocr: Some(backend.as_ref()),
            ..f.ctx(&cfg, &dir)
        };
        let summary = run(&handle, &ctx).unwrap();
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(
            (summary.groups, summary.added, summary.text_slides),
            (1, 1, 1),
            "{summary:?}"
        );
        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides[0].ocr_engine.as_deref(), Some("windows-ocr"));
        assert_eq!(slides[0].kind.as_deref(), Some("text"));
        let text = slides[0].ocr_text.clone().unwrap();
        eprintln!("Text aus dem Video: {text}");
        assert!(text.to_lowercase().contains("70"), "{text}");
    }

    /// Das Lesen meldet Fortschritt in der Phase `Slides`, nie rueckwaerts je Teil.
    #[test]
    fn reading_reports_progress_in_the_slides_phase() {
        let Some(f) = fixture() else { return };
        let dir = f.meeting_dir();
        let fake = ScriptedOcr::new(vec![Ok(AGENDA), Ok(OTHER), Ok("Dritte Folie mit Text")]);
        let (_, _, events) = run_with_ocr(&f, &fake, &dir);
        let seen = progress(&events);
        assert!(seen.iter().all(|(p, _, _)| *p == JobPhase::Slides));
        assert_eq!(
            seen.last().map(|(_, d, t)| (*d, *t)),
            Some((30_000, 30_000))
        );
    }

    #[test]
    fn the_job_registry_allows_one_slides_job_per_meeting() {
        let jobs = MeetingJobs::new();
        let emit: crate::managers::meetings::job::EmitFn = Arc::new(|_| {});
        let first = jobs.try_start("m1", Arc::clone(&emit)).unwrap();
        assert!(
            jobs.try_start("m1", Arc::clone(&emit)).is_err(),
            "zweiter Lauf: belegt"
        );
        first.handle().begin_phase_ex(JobPhase::Slides, 0, true);
        assert_eq!(jobs.phase_of("m1"), Some(JobPhase::Slides));
        assert!(jobs.pause("m1").is_ok(), "Slides ist pausierbar");
        jobs.stop("m1").unwrap();
        drop(first);
        assert!(
            jobs.try_start("m1", emit).is_ok(),
            "nach dem Ende wieder frei"
        );
    }

    #[test]
    fn sampling_errors_map_to_outcomes() {
        let outcome = |e| sampling_outcome(e).map(|s| s.outcome);
        assert_eq!(outcome(SlideError::Cancelled), Ok(SlideOutcome::Stopped));
        assert_eq!(
            outcome(SlideError::NoVideoStream),
            Ok(SlideOutcome::Skipped("slides_no_video"))
        );
        assert_eq!(
            outcome(SlideError::FfmpegMissing),
            Ok(SlideOutcome::Skipped("slides_ffmpeg_missing"))
        );
        for failure in [
            SlideError::DiskFull,
            SlideError::TooLong,
            SlideError::Ffmpeg("x".into()),
            SlideError::Io("x".into()),
        ] {
            assert_eq!(outcome(failure.clone()), Err(failure));
        }
    }

    #[test]
    fn a_full_disk_in_the_store_is_recognised() {
        let full = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            Some("database or disk is full".into()),
        );
        assert_eq!(store_error(&anyhow::Error::new(full)), SlideError::DiskFull);
        assert_eq!(
            store_error(&anyhow::anyhow!("meeting_not_found")),
            SlideError::Store("meeting_not_found".into())
        );
    }

    #[test]
    fn nearest_existing_prefers_the_closest_unclaimed_slide() {
        use super::super::store::MeetingSlide;
        let rec = |dhash: u64| SlideRecord {
            slide: MeetingSlide {
                id: dhash.to_string(),
                meeting_id: "m".into(),
                number: 1,
                origin: "video".into(),
                image_path: "slides/0001.jpg".into(),
                thumb_path: None,
                occurrences: vec![],
                ocr_text: None,
                ocr_engine: None,
                kind: None,
                description: None,
                description_model: None,
                hidden: false,
            },
            dhash,
        };
        let existing = vec![rec(0b0000), rec(0b0011), rec(u64::MAX)];
        let mut claimed = vec![false; 3];
        assert_eq!(
            nearest_existing(&existing, &claimed, 0b0001, 4),
            Some(0),
            "gleicher Abstand: frueher"
        );
        claimed[0] = true;
        assert_eq!(
            nearest_existing(&existing, &claimed, 0b0001, 4),
            Some(1),
            "vergebene zaehlen nicht"
        );
        assert_eq!(
            nearest_existing(&existing, &claimed, 0xF0F0_F0F0_F0F0_F0F0, 4),
            None,
            "zu weit"
        );
    }
}
