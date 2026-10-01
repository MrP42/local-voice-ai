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
//! Wiederholung: derselbe Lauf noch einmal (nach einem Stopp, Absturz oder auf
//! Wunsch) fuegt nichts doppelt ein. Eine erkannte Folie wird einer vorhandenen
//! zugeordnet, wenn der Hash hoechstens die Schwelle abweicht; dann wachsen nur
//! ihre Zeitbereiche, Bild, Text und "ausgeblendet" bleiben.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use log::{info, warn};

use super::ffmpeg::{self, Flow, SampleHook, SampleTick};
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

    // 2./3. Segmentieren und zusammenfassen (rein, schnell).
    let mut segments = segment(&sampled.samples, ctx.cfg.sample_fps, ctx.cfg);
    clamp_last_end(&mut segments, duration_ms);
    let kept = segments.iter().filter(|s| !s.black).count() as u32;
    let groups: Vec<SlideGroup> = group_by_hash(&segments, ctx.cfg);
    let mut summary = SlideSummary {
        outcome: SlideOutcome::Done,
        segments: kept,
        groups: groups.len() as u32,
        added: 0,
        updated: 0,
        duration_ms,
    };

    // 4. Bilder und Zeilen.
    let slides_dir = ctx.meeting_dir.join(SLIDES_DIR);
    std::fs::create_dir_all(&slides_dir)?;
    let leftovers = ffmpeg::clean_partial_files(&slides_dir);
    if leftovers > 0 {
        warn!("slides: {leftovers} halbfertige Bilder eines frueheren Laufs entfernt");
    }
    let total_ms = duration_ms.unwrap_or_else(|| groups.last().map_or(0, |g| g.rep_ms));
    job.begin_phase_ex(JobPhase::Slides, total_ms, true);

    let existing: Vec<SlideRecord> = ctx
        .store
        .slide_records(ctx.meeting_id)
        .map_err(|e| store_error(&e))?
        .into_iter()
        .filter(|r| r.slide.origin == ORIGIN_VIDEO)
        .collect();
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
                match ffmpeg::extract_frame(ctx.video, group.rep_ms, &image, Some(&thumb), &stop) {
                    Ok(()) => {}
                    Err(SlideError::Cancelled) => {
                        summary.outcome = SlideOutcome::Stopped;
                        return Ok(summary);
                    }
                    Err(e) => return Err(e),
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
                if let Err(e) = inserted {
                    // Keine Zeile ohne Datei und keine Datei ohne Zeile.
                    let _ = std::fs::remove_file(&image);
                    let _ = std::fs::remove_file(&thumb);
                    return Err(store_error(&e));
                }
                summary.added += 1;
            }
        }
        job.advance(group.rep_ms);
        after_slide(index);
    }
    job.advance(total_ms);
    info!(
        "slides: {} Folien erkannt ({} neu, {} aktualisiert)",
        summary.groups, summary.added, summary.updated
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
