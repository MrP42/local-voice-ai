//! M8 meetings: file import. Two paths that share only the meeting shell:
//!
//! - VTT/SRT: parsed directly into segments (`subtitle::parse_subtitles`),
//!   no transcription, meeting goes straight to `ready`.
//! - Everything else (`media::MEDIA_EXTENSIONS`): decoded to WAV via
//!   `media::ensure_wav`, then run through the *same* chunk -> transcribe ->
//!   append_delta pipeline the live recorder uses (`recorder.rs`), except as
//!   one mixed channel instead of separate mic/system channels — there is
//!   only one audio track to import.
//!
//! Errors on the audio path always land the meeting on `failed` with an
//! `Error` event; nothing is silently dropped (recording an import that
//! looked like it worked but has no segments would be worse than an obvious
//! failure).
//!
//! P8a: jede Verarbeitung ist ein Auftrag (`job.rs`): Fortschritt als Anteil
//! der Audiodauer, Pause und Stopp zwischen zwei Bloecken. Ein Stopp in der
//! Vorbereitung oder Transkription beendet die Besprechung als `cancelled`
//! (die fertigen Bloecke bleiben, die WAV bleibt fuer "Fortsetzen"); ein Stopp
//! erst in der Sprechertrennung laesst das vollstaendige Transkript stehen und
//! die Besprechung wird `ready`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::{error, info};
use tauri_specta::Event;

use super::chunker::{ChannelChunker, Chunk};
use super::empty::{coded_error, EmptyFill};
use super::job::{self, Gate, JobHandle, JobPhase};
use super::recorder::MeetingEvent;
use super::retention::MeetingAudioRetention;
use super::speakers::{self, ApplyOutcome};
use super::store::{MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta};
use super::subtitle::parse_subtitles;
use crate::managers::transcription::{TimedSegment, TranscriptionManager, WordTime};
use crate::media;

/// `StoredSegment::channel` for a single imported track — there is no
/// separate mic/system split for an imported file (mirrors the doc comment
/// on `StoredSegment::channel`: 2 = MixedCapture).
const CHANNEL_MIXED: u8 = 2;
/// Import runs off the UI thread already (`spawn_blocking`), so nobody is
/// waiting on any one chunk the way the live pipeline's user is; a coarser
/// block keeps the segment/model-call overhead down. `Segments` events after
/// each chunk are still frequent enough to serve as progress feedback.
const IMPORT_CHUNK_TARGET_MS: u64 = 60_000;
/// `chunk_all` feeds the chunker this many samples (~1 s at 16 kHz) at a
/// time instead of the whole file in one `push`. `ChannelChunker::push`
/// re-scans its buffer for a cut point every call
/// (`ChannelChunker::cut_point`), so one giant push followed by draining via
/// `push(&[])` would make every scan O(remaining samples) — quadratic over a
/// long import. Feeding in small slices keeps each scan bounded by the
/// target window, matching how the live recorder feeds it from real-time
/// audio callbacks.
const CHUNK_FEED_SLICE_SAMPLES: usize = 16_000;
const SUBTITLE_EXTENSIONS: [&str; 2] = ["vtt", "srt"];

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Import")
        .to_string()
}

/// Imports `path` as a new meeting — VTT/SRT directly, audio/video through
/// the chunked transcription pipeline. Returns the new meeting's id.
///
/// Consent (#15): `consent_confirmed = false` is NOT refused here, on purpose.
/// The import has a UI gate only (the app and the headless `--import-meeting`
/// both pass `true`); the backend merely records the moment
/// (`consent_confirmed_at`) when the caller says it was confirmed. The live
/// recording is different: `recorder::consent_gate` refuses it in the backend.
/// `the_backend_import_does_not_require_a_consent_confirmation_on_purpose` pins
/// this, so it cannot turn into a half-way gate by accident.
pub async fn import_media_file(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<TranscriptionManager>,
    path: PathBuf,
    consent_confirmed: bool,
) -> Result<String, String> {
    let consent_confirmed_at = consent_confirmed.then(|| chrono::Utc::now().timestamp());
    let title = title_from_path(&path);

    if SUBTITLE_EXTENSIONS.contains(&extension_of(&path).as_str()) {
        return import_subtitle_file(&store, &title, &path, consent_confirmed_at);
    }

    import_audio_file(app, store, tm, title, path, consent_confirmed_at).await
}

/// Subtitle path: synchronous — no transcription, so no reason to leave the
/// command task.
fn import_subtitle_file(
    store: &Arc<MeetingStore>,
    title: &str,
    path: &Path,
    consent_confirmed_at: Option<i64>,
) -> Result<String, String> {
    import_subtitle_file_into(store, title, path, consent_confirmed_at, None)
}

/// G1 (#70): Untertitel-Import, auf Wunsch in einen vorhandenen LEEREN Eintrag
/// (`target`) statt in eine neue Besprechung. Die Datei wird zuerst gelesen und
/// geprueft: eine kaputte Datei laesst das Ziel unberuehrt leer. Fehler des
/// Ziels: `target_not_empty`, `meeting_not_found`.
pub fn import_subtitle_file_into(
    store: &Arc<MeetingStore>,
    title: &str,
    path: &Path,
    consent_confirmed_at: Option<i64>,
    target: Option<&str>,
) -> Result<String, String> {
    // Fehler als Code (#15, `meetingErrors.ts`): `subtitle_unreadable`, `subtitle_invalid`.
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("subtitle_unreadable: {e}"))?;
    let segments = parse_subtitles(&content)?;
    let segment_count = segments.len();

    let meeting = match target {
        Some(target_id) => {
            let mut fill = EmptyFill::new(MeetingSource::Subtitle, MeetingStatus::Processing);
            fill.consent_confirmed_at = consent_confirmed_at;
            fill.title = Some(title);
            fill.only_if_default = true;
            store
                .fill_empty_meeting(target_id, &fill)
                .map_err(|e| coded_error("meeting_create_failed", &e))?
        }
        None => store
            .create_meeting(title, MeetingSource::Subtitle, consent_confirmed_at)
            .map_err(|e| format!("meeting_create_failed: {e}"))?,
    };
    if let Some(source) = path.to_str() {
        if let Err(e) = store.set_source_path(&meeting.id, source) {
            log::warn!("meetings: source_path not stored for subtitle import: {e}");
        }
    }

    store
        .append_delta(
            &meeting.id,
            &TranscriptDelta {
                new_segments: segments,
            },
        )
        .map_err(|e| format!("segments_store_failed: {e}"))?;
    store
        .set_status(&meeting.id, MeetingStatus::Ready)
        .map_err(|e| format!("status_ready_failed: {e}"))?;

    // A1: Herkunft des Transkripts (Quelle: die Untertiteldatei, kein Modell).
    crate::managers::provenance::generation::record_subtitle_import(
        store,
        &meeting.id,
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Untertitel"),
        segment_count,
    );
    info!("meetings: subtitle import ready ({})", meeting.id);
    Ok(meeting.id)
}

/// Audio/video path: create the (already `processing`) meeting row up front
/// so the caller has an id to show immediately, then do the actual decode +
/// transcribe work off the async runtime.
async fn import_audio_file(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<TranscriptionManager>,
    title: String,
    path: PathBuf,
    consent_confirmed_at: Option<i64>,
) -> Result<String, String> {
    let meeting = store
        .create_meeting(&title, MeetingSource::Import, consent_confirmed_at)
        .map_err(|e| format!("meeting_create_failed: {e}"))?;
    let meeting_id = meeting.id.clone();
    // The title starts out as the file stem but is user-editable from here on,
    // so the file it came from is recorded separately (store.rs `source_path`).
    if let Some(source) = path.to_str() {
        if let Err(e) = store.set_source_path(&meeting_id, source) {
            log::warn!("meetings: source_path not stored for import: {e}");
        }
    }
    // P8a: Fortschritt, Pause und Stopp fuer diese Verarbeitung. Fuer eine
    // frische Besprechung gibt es nie schon einen Auftrag; scheitert es doch,
    // darf die Zeile nicht auf `processing` stehen bleiben.
    let job = match job::global().try_start(&meeting_id, job::app_emit(app)) {
        Ok(job) => job,
        Err(e) => {
            let _ = store.set_status(&meeting_id, MeetingStatus::Failed);
            return Err(e.to_string());
        }
    };
    emit_state(app, &meeting_id, "processing");

    let app_owned = app.clone();
    let status_store = Arc::clone(&store);
    let blocking_meeting_id = meeting_id.clone();
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        run_import(
            &app_owned,
            &store,
            &tm,
            &blocking_meeting_id,
            &path,
            job.handle(),
        )
    })
    .await;

    match join_result {
        Ok(Ok(())) => Ok(meeting_id),
        Ok(Err(e)) => Err(e),
        Err(join_err) => {
            let _ = status_store.set_status(&meeting_id, MeetingStatus::Failed);
            emit_error(app, &meeting_id, "import_panicked");
            Err(format!("meetings_import panicked: {join_err}"))
        }
    }
}

/// #15: kopiert die dekodierte WAV nach `dst`. Scheitert die Kopie (Platte voll,
/// Datei gesperrt), bleibt keine halbe `import.wav` zurueck: der Import steht dann
/// auf `failed`, und die Datei wuerde niemand mehr kennen (die DB-Pfade stehen
/// erst nach der Kopie). `copy` ist die Kopierfunktion, damit der Fehlerweg ohne
/// volle Platte pruefbar ist.
fn copy_import_wav_with(
    copy: impl FnOnce(&Path, &Path) -> std::io::Result<u64>,
    src: &Path,
    dst: &Path,
) -> Result<(), String> {
    copy(src, dst).map(|_| ()).map_err(|e| {
        // Best effort: eine halbe Datei, die niemand kennt, soll nicht liegen bleiben.
        let _ = std::fs::remove_file(dst);
        format!("import_wav_copy_failed: {e}")
    })
}

/// #15: traegt die Audiodatei eines Imports in die Besprechung ein (Pfad und Dauer
/// stehen VOR der Transkription, P8a). Gelingt das nicht, kennt keine Zeile die
/// Datei: sie wird entfernt, statt als Debris zu bleiben. Steht der Pfad dagegen
/// in der Datenbank (ein spaeterer Fehler in der Transkription), bleibt die Datei:
/// "Neu transkribieren" einer fehlgeschlagenen Besprechung liest sie.
fn register_import_audio(
    store: &MeetingStore,
    meeting_id: &str,
    wav: &Path,
    duration_ms: u64,
) -> Result<(), String> {
    store
        .set_audio_paths(meeting_id, wav.to_str(), None, Some(duration_ms))
        .map_err(|e| {
            let _ = std::fs::remove_file(wav);
            format!("audio_paths_failed: {e}")
        })
}

/// Wie eine Import-Verarbeitung endete, wenn sie nicht scheiterte.
enum ImportEnd {
    /// Das Transkript ist vollstaendig (die Sprechertrennung darf gestoppt sein).
    Done { duration_ms: u64 },
    /// Der Nutzer hat vor dem Ende der Transkription gestoppt.
    Stopped,
}

/// The blocking body: decode, copy the WAV, transcribe in chunks, finish with
/// `ready` or `failed` (or, when the user stopped, `cancelled`) — always one
/// of them, plus the matching event. U7: `pub(super)`, die Warteschlange
/// (`queue.rs`) ruft sie fuer jede wartende Datei.
pub(super) fn run_import(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    path: &Path,
    job: &Arc<JobHandle>,
) -> Result<(), String> {
    let import_started = std::time::Instant::now();
    let outcome = (|| -> Result<ImportEnd, String> {
        job.begin_phase(JobPhase::Prepare, 0);
        // Kick the model load FIRST (non-blocking) so it warms up while ffmpeg
        // decodes; `transcribe_segments` then waits on the load condvar instead
        // of failing with "Model is not loaded" — the live recorder does the
        // same in start() (recorder.rs). Without this, any import after the
        // idle unload (default 5 min) failed immediately. Meetings may use
        // their own model (`meeting_model`, dictation model as fallback).
        tm.initiate_meeting_model_load(&crate::settings::get_settings(app));
        // P8a: ein Stopp beendet ffmpeg (ueber sein Handle, nie ueber den Namen).
        let (wav_path, _tmp_guard) =
            match media::ensure_wav_cancellable(path, 16_000, &job.stop_flag()) {
                Ok(decoded) => decoded,
                Err(e) if e == media::DECODE_CANCELLED => return Ok(ImportEnd::Stopped),
                Err(e) => return Err(e),
            };
        if job.is_stopped() {
            return Ok(ImportEnd::Stopped);
        }
        let samples = read_wav_i16_mono_16k(&wav_path)?;

        let dir = super::meetings_data_dir(app)
            .map_err(|e| format!("app_data_dir_failed: {e}"))?
            .join(meeting_id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("meeting_dir_failed: {e}"))?;
        let import_wav_path = dir.join("import.wav");
        copy_import_wav_with(|from, to| std::fs::copy(from, to), &wav_path, &import_wav_path)?;

        let duration_ms = (samples.len() as u64 * 1_000) / 16_000;
        // P8a: die Pfade stehen VOR der Transkription. Ein Absturz oder Stopp
        // mittendrin laesst sonst eine Besprechung ohne Audio zurueck, die
        // weder nachgeholt noch fortgesetzt werden kann.
        register_import_audio(store, meeting_id, &import_wav_path, duration_ms)?;

        job.begin_phase(JobPhase::Transcription, duration_ms);
        let run = transcribe_and_store(
            app,
            store,
            tm,
            meeting_id,
            &samples,
            CHANNEL_MIXED,
            0,
            job,
            0,
            &mut || tm.initiate_meeting_model_load(&crate::settings::get_settings(app)),
        )?;
        // Der Puffer (i16) wird nicht mehr gebraucht; die Sprechertrennung liest
        // die Spur selbst (f32) und soll nicht zwei Kopien nebeneinander halten.
        drop(samples);
        if run.stopped {
            return Ok(ImportEnd::Stopped);
        }
        // M3-P3b: Sprecher (Einstellung `meeting_diarization`). Ein Fehler hier
        // macht den Import nicht kaputt: das Transkript steht schon.
        run_speaker_step(app, store, meeting_id, Some(job));
        Ok(ImportEnd::Done { duration_ms })
    })();

    let result = match outcome {
        Ok(ImportEnd::Done { duration_ms }) => {
            mark_import_ready(store, meeting_id)?;
            // A1: Herkunft des Transkripts (Modell, Dauer, Quelldatei). Das
            // tatsaechlich geladene Modell zaehlt (ein Ausweichmodell eingeschlossen).
            record_import_provenance(app, store, tm, meeting_id, path, duration_ms, import_started);
            // Imports have no separate "recording ended" moment, so `now`
            // stands in for `ended_at` here too (mirrors the live recorder's
            // `stop()`) — and is persisted so later recomputations (minutes
            // generated after a delay) stay anchored to it. No minutes
            // document exists yet.
            let now = chrono::Utc::now().timestamp();
            if let Err(e) = store.set_ended_at(meeting_id, now) {
                log::warn!("meetings: ended_at not stored after import: {e}");
            }
            let policy = crate::settings::get_meeting_audio_retention(app);
            let until = super::retention::retention_until(&policy, now, now, false);
            if let Err(e) = store.set_retention_until(meeting_id, until) {
                log::warn!("meetings: retention_until not stored after import: {e}");
            }
            emit_state(app, meeting_id, "ready");
            info!("meetings: import ready ({meeting_id}, {duration_ms} ms)");
            Ok(())
        }
        Ok(ImportEnd::Stopped) => {
            let policy = crate::settings::get_meeting_audio_retention(app);
            match mark_stopped(
                store,
                meeting_id,
                chrono::Utc::now().timestamp(),
                &policy,
            ) {
                Ok(()) => {
                    emit_state(app, meeting_id, "cancelled");
                    info!("meetings: import stopped by the user ({meeting_id})");
                    Ok(())
                }
                // Der Endzustand liess sich nicht schreiben (Platte voll, Datei
                // gesperrt): wie jeder andere Fehler `failed` statt `processing`,
                // und das Diktatmodell kommt trotzdem zurueck.
                Err(e) => {
                    error!("meetings: stop not stored ({meeting_id}): {e}");
                    mark_import_failed(store, meeting_id);
                    emit_state(app, meeting_id, "failed");
                    emit_error(app, meeting_id, "import_failed");
                    Err(e)
                }
            }
        }
        Err(e) => {
            error!("meetings: import failed ({meeting_id}): {e}");
            mark_import_failed(store, meeting_id);
            // Same branch as the status write right above — see
            // `mark_import_failed`'s doc comment for why this call itself
            // isn't covered by a test. The state event keeps the list's
            // status badge honest (it refreshes on state events; without
            // this, a failed import kept showing "processing" until reload).
            emit_state(app, meeting_id, "failed");
            emit_error(app, meeting_id, "import_failed");
            Err(e)
        }
    };

    // Restore the dictation model (no-op when meeting and dictation model
    // are the same) — mirrors the live recorder's stop(). U7: wartet schon die
    // naechste Datei der Warteschlange, bleibt das Besprechungsmodell geladen
    // (sonst kostete jede Datei zwei Modellwechsel); das Diktatmodell kommt mit
    // der letzten zurueck.
    if !super::queue::more_waiting() {
        let dictation_model = crate::settings::get_settings(app).selected_model;
        tm.initiate_model_load_target(&dictation_model);
    }

    result
}

/// A1: schreibt die Provenienz eines fertigen Imports. Nie ein Fehler nach
/// aussen: das Transkript steht schon.
fn record_import_provenance(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    path: &Path,
    audio_ms: u64,
    started: std::time::Instant,
) {
    let model = tm.get_current_model().unwrap_or_else(|| {
        let settings = crate::settings::get_settings(app);
        settings.meeting_model.unwrap_or(settings.selected_model)
    });
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Import")
        .to_string();
    crate::managers::provenance::generation::record_stt(
        store,
        crate::managers::provenance::generation::SttRun {
            meeting_id,
            operation: "import",
            actor_kind: crate::managers::provenance::ActorKind::User,
            model_id: &model,
            revision: None,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            sources: vec![crate::managers::provenance::SourceRef::new(
                "import",
                &file,
                Some(&file),
            )],
            params: serde_json::json!({ "audio_ms": audio_ms }),
        },
    );
}

/// P8a: Endzustand nach einem Stopp durch den Nutzer: `cancelled`, mit
/// Abschlusszeit und Aufbewahrung wie bei einem normalen Ende (sonst bliebe
/// die WAV eines abgebrochenen Imports fuer immer liegen). Das bis dahin
/// gespeicherte Transkript wird nicht angefasst.
pub(super) fn mark_stopped(
    store: &MeetingStore,
    meeting_id: &str,
    now: i64,
    policy: &MeetingAudioRetention,
) -> Result<(), String> {
    store
        .set_status(meeting_id, MeetingStatus::Cancelled)
        .map_err(|e| format!("status_cancelled_failed: {e}"))?;
    if let Err(e) = store.set_ended_at(meeting_id, now) {
        log::warn!("meetings: ended_at not stored after stop: {e}");
    }
    let until = super::retention::retention_until(policy, now, now, false);
    if let Err(e) = store.set_retention_until(meeting_id, until) {
        log::warn!("meetings: retention_until not stored after stop: {e}");
    }
    Ok(())
}

/// M3-P3b: Sprechertrennung fuer das gespeicherte Transkript einer
/// Besprechung (Import, Neu-Transkription): gespeicherte Turns zuerst, sonst
/// Sortformer (Einstellung `meeting_diarization`, RAM-Tor, ein Modell zur
/// Zeit). Danach laedt die Anzeige das Transkript neu (`Reset` + `Segments`),
/// weil sich Sprecher und Segmentgrenzen geaendert haben. Nie ein Fehler und
/// keine Panik nach aussen: das Transkript bleibt, wie es ist, der Bericht
/// steht in `metadata_json.diarize`.
pub(super) fn run_speaker_step(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    meeting_id: &str,
    job: Option<&Arc<JobHandle>>,
) {
    let mut diarizer = super::final_pass::AppDiarizer::from_app(
        app,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    // P8a: die Phase "Sprecher" mit Fortschritt je Kanal; ein Stopp beendet den
    // laufenden Modelllauf, das Transkript bleibt, wie es ist.
    if let Some(job) = job {
        job.begin_phase(JobPhase::Speakers, 0);
        diarizer.attach_job(Arc::clone(job));
    }
    let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        speakers::step_on_stored(store, meeting_id, &mut diarizer, true)
    }));
    let (report, outcome) = match step {
        Ok(done) => done,
        Err(_) => {
            error!("meetings: speaker step panicked ({meeting_id}) - transcript without speakers");
            return;
        }
    };
    match outcome {
        ApplyOutcome::Applied {
            epoch,
            epoch_bumped,
        } => {
            info!(
                "meetings: speakers applied ({meeting_id}): {} assigned, {} split, epoch {epoch}{}",
                report.assigned,
                report.split_added,
                if epoch_bumped { " (new)" } else { "" }
            );
            if let Ok(all) = store.get_segments(meeting_id) {
                let _ = (MeetingEvent::Reset {
                    meeting_id: meeting_id.to_string(),
                })
                .emit(app);
                let _ = (MeetingEvent::Segments {
                    meeting_id: meeting_id.to_string(),
                    appended: all,
                })
                .emit(app);
            }
        }
        ApplyOutcome::Unchanged => {}
        ApplyOutcome::Conflict => {
            log::warn!("meetings: speakers not applied ({meeting_id}): transcript kept changing")
        }
        ApplyOutcome::Failed(e) => log::warn!("meetings: speakers not applied ({meeting_id}): {e}"),
    }
}

/// Success half of the "always reach a terminal status" contract.
fn mark_import_ready(store: &Arc<MeetingStore>, meeting_id: &str) -> Result<(), String> {
    store
        .set_status(meeting_id, MeetingStatus::Ready)
        .map_err(|e| format!("status_ready_failed: {e}"))
}

/// Failure half: whatever went wrong on the audio path, the meeting must
/// never stay stuck on `processing` — it always lands on `failed`. Split out
/// of `run_import`'s `Err` arm so it can be exercised directly against a
/// real `MeetingStore`, forced by an actually-unreadable input file, without
/// needing a live `AppHandle` (which `run_import` itself requires, for
/// `app_data_dir` and the `MeetingEvent` emits). `run_import` calls this and
/// then `emit_error(app, meeting_id, "import_failed")` on the very next line
/// — that emit isn't separately covered by a test since it needs a real
/// `AppHandle`, but it sits in the same match arm as this call, so a test
/// proving this function runs on a genuine failing outcome proves that arm —
/// and therefore the emit — is reached.
fn mark_import_failed(store: &Arc<MeetingStore>, meeting_id: &str) {
    let _ = store.set_status(meeting_id, MeetingStatus::Failed);
}

/// Feeds `samples` into a fresh `ChannelChunker` in bounded
/// `CHUNK_FEED_SLICE_SAMPLES` slices (see its doc comment for why: a single
/// whole-file `push` makes every cut-point scan quadratic), then `flush`es
/// the tail. Pure — no I/O, no transcription — so the "every sample is
/// covered exactly once, offsets strictly increase" invariant is directly
/// testable without a model or a store.
fn chunk_all(samples: &[i16], target_ms: u64) -> Vec<Chunk> {
    let mut chunker = ChannelChunker::new(target_ms);
    let mut chunks = Vec::new();
    for slice in samples.chunks(CHUNK_FEED_SLICE_SAMPLES) {
        if let Some(chunk) = chunker.push(slice) {
            chunks.push(chunk);
        }
    }
    if let Some(chunk) = chunker.flush() {
        chunks.push(chunk);
    }
    chunks
}

/// Wie oft ein Block versucht wird, bevor er als Luecke gespeichert wird.
const CHUNK_ATTEMPTS: u32 = 3;

/// Zeitstempel `m:ss` fuer die Lueckenmarkierung im Transkript.
fn mm_ss(ms: u64) -> String {
    let secs = ms / 1_000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Text des Platzhalters, der eine nicht transkribierte Stelle im Transkript
/// markiert. Steht im Transkript selbst, damit die Luecke beim Lesen sichtbar
/// ist und ueber den Stift von Hand ergaenzt werden kann.
pub(super) fn gap_placeholder(start_ms: u64, end_ms: u64) -> String {
    format!(
        "[Nicht transkribiert {}–{} — bitte anhören und ergänzen]",
        mm_ss(start_ms),
        mm_ss(end_ms)
    )
}

/// Ist `text` ein Luecken-Platzhalter von [`gap_placeholder`]? Ein solcher
/// Text gehoert keiner Person (Sprecherzuordnung laesst ihn aus).
pub(super) fn is_gap_placeholder(text: &str) -> bool {
    text.starts_with("[Nicht transkribiert ")
}

/// Ein Block: erst mit Wiederholung transkribieren, sonst als Luecke
/// speichern — die Aufnahme laeuft in jedem Fall weiter.
///
/// Am 17.09.2026 hat ein Diktat per Tastenkuerzel waehrend eines Imports das
/// Besprechungsmodell verdraengt; der naechste Block bekam "Model is not
/// loaded", und der ganze Import stand auf "fehlgeschlagen", obwohl nur ein
/// Block fehlte. Deshalb: das Besprechungsmodell erneut anfordern (die
/// Transkription wartet auf dessen Ladevorgang), bis zu drei Versuche, und
/// wenn es dann immer noch nicht geht, ein Platzhalter mit Zeitraum statt
/// eines Abbruchs. Vollstaendigkeit hat Vorrang: lieber eine markierte
/// Luecke als ein halbes Transkript.
pub(super) fn transcribe_chunk_resilient(
    app: &tauri::AppHandle,
    tm: &Arc<TranscriptionManager>,
    chunk: &Chunk,
) -> Vec<crate::managers::transcription::TimedSegment> {
    let mut last_error = String::new();
    for attempt in 1..=CHUNK_ATTEMPTS {
        match tm.transcribe_segments(chunk.samples.clone()) {
            Ok(timed) => return timed,
            Err(e) => {
                last_error = e.to_string();
                log::warn!(
                    "meetings: Block bei {} ms nicht transkribiert (Versuch {attempt} von {CHUNK_ATTEMPTS}): {last_error}",
                    chunk.offset_ms
                );
                if attempt < CHUNK_ATTEMPTS {
                    tm.initiate_meeting_model_load(&crate::settings::get_settings(app));
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
    }
    let chunk_ms = (chunk.samples.len() as u64 * 1_000) / 16_000;
    let (start_ms, end_ms) = (chunk.offset_ms, chunk.offset_ms + chunk_ms);
    error!(
        "meetings: Block {}–{} endgültig nicht transkribiert ({last_error}) — als Lücke gespeichert",
        mm_ss(start_ms),
        mm_ss(end_ms)
    );
    vec![crate::managers::transcription::TimedSegment {
        text: gap_placeholder(start_ms, end_ms),
        start_ms: 0,
        end_ms: chunk_ms,
        words: None,
    }]
}

/// STT-Ergebnis eines Blocks -> zu speichernde Segmente: Zeiten auf die
/// Kanal-Achse (`offset_ms` = Beginn des Blocks), leere Texte entfallen,
/// `segment_index` laeuft ab `next_index` weiter. M3-P3b: die Wortzeiten
/// bleiben erhalten (ebenfalls auf der Kanal-Achse, wie in `live_segments`),
/// damit Segmente an Sprecherwechseln geteilt werden koennen.
pub(super) fn stored_segments(
    timed: Vec<crate::managers::transcription::TimedSegment>,
    offset_ms: u64,
    channel: u8,
    next_index: &mut u32,
) -> Vec<StoredSegment> {
    timed
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| {
            let segment = StoredSegment {
                segment_index: *next_index,
                text: s.text,
                start_ms: offset_ms + s.start_ms,
                end_ms: offset_ms + s.end_ms,
                channel,
                speaker_index: None,
                words: s.words.map(|ws| {
                    ws.into_iter()
                        .map(|w| WordTime {
                            text: w.text,
                            start_ms: offset_ms + w.start_ms,
                            end_ms: offset_ms + w.end_ms,
                        })
                        .collect()
                }),
            };
            *next_index += 1;
            segment
        })
        .collect()
}

/// Was ein Durchlauf ueber Bloecke ergab.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TranscribeRun {
    /// Naechster freier `segment_index`.
    pub next_index: u32,
    /// Der Nutzer hat gestoppt: jeder Block bis dahin ist gespeichert, der
    /// Rest nicht transkribiert.
    pub stopped: bool,
}

/// Ein Durchlauf ueber `chunks` mit einem Kontrollpunkt vor jedem Block (Pause
/// haelt hier an, Stopp beendet den Lauf) und dem Fortschritt danach. Ohne
/// Tauri, Modell und Speicher: Transkription, Ablage und "nach der Pause"
/// stehen als Closures, damit Pause, Stopp und Fortschritt direkt testbar
/// sind. Ein Block, der schon laeuft, wird fertig und gespeichert.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_chunks(
    chunks: Vec<Chunk>,
    job: &JobHandle,
    base_ms: u64,
    channel: u8,
    first_index: u32,
    transcribe: &mut dyn FnMut(&Chunk) -> Vec<TimedSegment>,
    store_batch: &mut dyn FnMut(Vec<StoredSegment>) -> Result<(), String>,
    on_resume: &mut dyn FnMut(),
) -> Result<TranscribeRun, String> {
    let mut next_index = first_index;
    for chunk in chunks {
        match job.checkpoint(&|| false) {
            Gate::Go { resumed } => {
                if resumed {
                    // Nach einer langen Pause ist das Modell evtl. wegen
                    // Leerlauf entladen: gleich wieder anfordern, statt den
                    // ersten Versuch scheitern zu lassen.
                    on_resume();
                }
            }
            Gate::Stopped | Gate::Cancelled => {
                return Ok(TranscribeRun {
                    next_index,
                    stopped: true,
                })
            }
        }
        let offset_ms = chunk.offset_ms;
        let chunk_ms = chunk.samples.len() as u64 / 16;
        let timed = transcribe(&chunk);
        let appended = stored_segments(timed, offset_ms, channel, &mut next_index);
        if !appended.is_empty() {
            store_batch(appended)?;
        }
        job.advance(base_ms + offset_ms + chunk_ms);
    }
    Ok(TranscribeRun {
        next_index,
        stopped: false,
    })
}

/// Transcribes and stores each chunk in turn, same as the live worker in
/// `recorder.rs` — chunking itself happens incrementally in `chunk_all`.
/// `base_ms`: Beginn dieser Spur auf der Achse des Auftrags (mehrere Spuren
/// zaehlen in einen Fortschritt); `on_resume`: Modell nach einer Pause wieder
/// anfordern.
#[allow(clippy::too_many_arguments)]
pub(super) fn transcribe_and_store(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    samples: &[i16],
    channel: u8,
    first_index: u32,
    job: &JobHandle,
    base_ms: u64,
    on_resume: &mut dyn FnMut(),
) -> Result<TranscribeRun, String> {
    drive_chunks(
        chunk_all(samples, IMPORT_CHUNK_TARGET_MS),
        job,
        base_ms,
        channel,
        first_index,
        &mut |chunk| transcribe_chunk_resilient(app, tm, chunk),
        &mut |appended| {
            store
                .append_delta(
                    meeting_id,
                    &TranscriptDelta {
                        new_segments: appended.clone(),
                    },
                )
                .map_err(|e| format!("delta_store_failed: {e}"))?;
            let _ = (MeetingEvent::Segments {
                meeting_id: meeting_id.to_string(),
                appended,
            })
            .emit(app);
            Ok(())
        },
        on_resume,
    )
}

/// Reads a WAV file as 16 kHz mono i16 PCM, downmixing/resampling as needed.
/// `ensure_wav` already guarantees this for anything it transcodes via
/// ffmpeg, but a `.wav` input passes straight through unchanged, so this
/// stays tolerant of arbitrary channel counts and sample rates (same
/// downmix/linear-resample approach as
/// `managers::tts::voices::load_wav_mono_16k`, i16 output instead of f32).
pub(super) fn read_wav_i16_mono_16k(path: &Path) -> Result<Vec<i16>, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| format!("WAV nicht lesbar: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        (hound::SampleFormat::Int, bits) => {
            let scale = (1i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
    };

    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

    let resampled = if spec.sample_rate == 16_000 || mono.is_empty() {
        mono
    } else {
        let ratio = spec.sample_rate as f64 / 16_000.0;
        let out_len = ((mono.len() as f64) / ratio).floor() as usize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = i as f64 * ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = mono[idx.min(mono.len() - 1)];
            let b = mono[(idx + 1).min(mono.len() - 1)];
            out.push(a + (b - a) * frac);
        }
        out
    };

    Ok(resampled
        .into_iter()
        .map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect())
}

fn emit_state(app: &tauri::AppHandle, meeting_id: &str, status: &str) {
    let _ = (MeetingEvent::State {
        meeting_id: meeting_id.to_string(),
        status: status.to_string(),
        paused: false,
    })
    .emit(app);
}

fn emit_error(app: &tauri::AppHandle, meeting_id: &str, message: &str) {
    let _ = (MeetingEvent::Error {
        meeting_id: meeting_id.to_string(),
        message: message.to_string(),
    })
    .emit(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A1: ein Untertitel-Import legt die Herkunft des Transkripts an (Quelle:
    /// die Datei, kein Modell) und bleibt ohne sie gueltig.
    #[test]
    fn a_subtitle_import_records_where_the_transcript_came_from() {
        use crate::managers::provenance::{self, ActorKind, SubjectKind};
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let vtt = dir.path().join("interview.vtt");
        std::fs::write(
            &vtt,
            "WEBVTT

00:00:00.000 --> 00:00:02.000
Guten Tag.

00:00:02.500 --> 00:00:05.000
Willkommen zum Gespräch.
",
        )
        .unwrap();
        let id = import_subtitle_file(&store, "interview", &vtt, None).unwrap();

        let conn = store.get_connection().unwrap();
        let entries = provenance::list(&conn, SubjectKind::Transcript, &id).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation, "subtitles_import");
        assert_eq!(entries[0].actor_kind, Some(ActorKind::User));
        assert_eq!(entries[0].model_id, None, "kein Modell: die Untertitel kommen fertig");
        assert_eq!(entries[0].sources[0].kind, "subtitle");
        assert_eq!(entries[0].sources[0].reference, "interview.vtt");
        assert!(entries[0].params_json.as_deref().unwrap().contains("\"segments\":2"));

        // Ist die Tabelle unbrauchbar, scheitert der Import nicht.
        conn.execute_batch("DROP TABLE provenance").unwrap();
        let second = import_subtitle_file(&store, "noch einmal", &vtt, None).unwrap();
        assert_eq!(store.get_segments(&second).unwrap().len(), 2);
    }

    /// G1 (#70): Untertitel in einen leeren Eintrag: dieselbe Besprechung wird
    /// gefuellt (Projekt, Titel des Nutzers bleiben), eine kaputte Datei oder ein
    /// nicht leeres Ziel lassen das Ziel unberuehrt.
    #[test]
    fn a_subtitle_file_fills_an_empty_entry_and_refuses_anything_else() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let folder = store.folder_save(None, "Kunde", None).unwrap().id;
        let target = store.create_empty_meeting("Neue Besprechung", Some(&folder)).unwrap();
        store.set_title(&target.id, "Mein Titel").unwrap();
        let vtt = dir.path().join("interview.vtt");
        std::fs::write(&vtt, "WEBVTT

00:00:00.000 --> 00:00:02.000
Guten Tag.
").unwrap();
        let bad = dir.path().join("kaputt.vtt");
        std::fs::write(&bad, "").unwrap();

        // Kaputte Datei: das Ziel bleibt ein leerer Eintrag.
        assert!(import_subtitle_file_into(&store, "kaputt", &bad, None, Some(&target.id)).is_err());
        assert!(store.is_empty_meeting(&target.id).unwrap());

        let id = import_subtitle_file_into(&store, "interview", &vtt, Some(3), Some(&target.id))
            .unwrap();
        assert_eq!(id, target.id, "dieselbe Besprechung");
        let m = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!((m.source.as_str(), m.status.as_str()), ("subtitle", "ready"));
        assert_eq!(m.title, "Mein Titel", "umbenannter Titel bleibt");
        assert_eq!(m.consent_confirmed_at, Some(3));
        assert_eq!(store.get_segments(&id).unwrap().len(), 1);
        assert_eq!(store.meeting_folder_ids(&id).unwrap(), vec![folder]);
        assert_eq!(store.list_meetings(0, 10).unwrap().len(), 1, "keine zweite Besprechung");

        // Zweiter Versuch: das Ziel ist nicht mehr leer.
        let err = import_subtitle_file_into(&store, "noch", &vtt, None, Some(&id)).unwrap_err();
        assert_eq!(err, "target_not_empty");
        assert_eq!(store.get_segments(&id).unwrap().len(), 1);
    }

    #[test]
    fn gap_placeholder_names_the_time_span() {
        assert_eq!(
            gap_placeholder(72_000, 132_000),
            "[Nicht transkribiert 1:12–2:12 — bitte anhören und ergänzen]"
        );
    }

    #[test]
    fn imported_segments_keep_their_word_times_on_the_channel_axis() {
        use crate::managers::transcription::TimedSegment;
        let timed = vec![
            TimedSegment {
                text: "Guten Tag zusammen".into(),
                start_ms: 0,
                end_ms: 1_500,
                words: Some(vec![
                    WordTime {
                        text: "Guten".into(),
                        start_ms: 100,
                        end_ms: 400,
                    },
                    WordTime {
                        text: "Tag".into(),
                        start_ms: 450,
                        end_ms: 700,
                    },
                ]),
            },
            TimedSegment {
                text: "   ".into(),
                start_ms: 1_500,
                end_ms: 1_700,
                words: None,
            },
            TimedSegment {
                text: "Ohne Woerter".into(),
                start_ms: 1_700,
                end_ms: 2_500,
                words: None,
            },
        ];
        let mut next = 7;
        let out = stored_segments(timed, 60_000, CHANNEL_MIXED, &mut next);
        assert_eq!(out.len(), 2, "der leere Text entfaellt");
        assert_eq!(next, 9, "Indizes laufen weiter");
        assert_eq!((out[0].segment_index, out[1].segment_index), (7, 8));
        assert_eq!((out[0].start_ms, out[0].end_ms), (60_000, 61_500));
        assert_eq!(out[0].channel, CHANNEL_MIXED);
        assert_eq!(out[0].speaker_index, None);
        let words = out[0].words.as_ref().expect("Woerter bleiben");
        assert_eq!((words[0].start_ms, words[0].end_ms), (60_100, 60_400));
        assert_eq!((words[1].start_ms, words[1].end_ms), (60_450, 60_700));
        assert!(out[1].words.is_none(), "ohne Wortzeiten keine erfinden");
    }

    #[test]
    fn a_gap_placeholder_is_recognised_and_ordinary_text_is_not() {
        assert!(is_gap_placeholder(&gap_placeholder(0, 60_000)));
        assert!(!is_gap_placeholder("Nicht transkribiert wurde gestern"));
        assert!(!is_gap_placeholder("[Applaus]"));
        assert!(!is_gap_placeholder(""));
    }

    #[test]
    fn title_from_path_uses_the_file_stem() {
        assert_eq!(
            title_from_path(Path::new("C:/rec/Jour Fixe.mp3")),
            "Jour Fixe"
        );
        assert_eq!(title_from_path(Path::new("C:/rec/notes.vtt")), "notes");
    }

    #[test]
    fn title_from_path_falls_back_when_there_is_no_usable_stem() {
        assert_eq!(title_from_path(Path::new("C:/rec/   .mp3")), "Import");
    }

    #[test]
    fn subtitle_extensions_are_recognized_case_insensitively() {
        assert!(SUBTITLE_EXTENSIONS.contains(&extension_of(Path::new("a.VTT")).as_str()));
        assert!(SUBTITLE_EXTENSIONS.contains(&extension_of(Path::new("a.srt")).as_str()));
        assert!(!SUBTITLE_EXTENSIONS.contains(&extension_of(Path::new("a.mp3")).as_str()));
    }

    // -- Finding 2: incremental chunk feeding -----------------------------

    #[test]
    fn chunk_all_covers_every_sample_exactly_once_with_increasing_offsets() {
        // 130 s of audio at 16 kHz, well past two 60 s target chunks, fed in
        // 1 s slices by `chunk_all` (not one whole-file push).
        let samples = vec![7_000i16; 16_000 * 130];
        let chunks = chunk_all(&samples, IMPORT_CHUNK_TARGET_MS);

        assert!(
            chunks.len() >= 2,
            "130 s of audio at a 60 s target should yield at least two chunks"
        );

        let total: usize = chunks.iter().map(|c| c.samples.len()).sum();
        assert_eq!(
            total,
            samples.len(),
            "every input sample must be covered exactly once"
        );

        let mut last_offset: Option<u64> = None;
        for chunk in &chunks {
            if let Some(prev) = last_offset {
                assert!(
                    chunk.offset_ms > prev,
                    "chunk offsets must strictly increase (got {} after {prev})",
                    chunk.offset_ms
                );
            }
            last_offset = Some(chunk.offset_ms);
        }
    }

    #[test]
    fn chunk_all_of_empty_input_yields_no_chunks() {
        assert!(chunk_all(&[], IMPORT_CHUNK_TARGET_MS).is_empty());
    }

    #[test]
    fn chunk_all_flushes_a_short_tail_below_the_target() {
        let samples = vec![7_000i16; 16_000 * 5]; // 5 s, well under any target
        let chunks = chunk_all(&samples, IMPORT_CHUNK_TARGET_MS);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), samples.len());
        assert_eq!(chunks[0].offset_ms, 0);
    }

    // -- Finding 1: the "never silent" failure contract --------------------

    fn temp_store() -> Arc<MeetingStore> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&path).unwrap();
        std::mem::forget(dir); // keep the tempdir alive for the store's lifetime
        Arc::new(store)
    }

    #[test]
    fn garbage_input_is_a_real_pipeline_failure() {
        // Same first two steps `run_import`'s outcome closure runs
        // (import.rs: `ensure_wav` then `read_wav_i16_mono_16k`): a `.wav`
        // extension makes `ensure_wav` pass the file through unchanged, so
        // garbage bytes must fail at the hound read, not silently produce
        // empty audio.
        let dir = tempfile::tempdir().unwrap();
        let garbage_path = dir.path().join("not-actually-audio.wav");
        std::fs::write(&garbage_path, b"this is not a wav file at all, just text").unwrap();

        let outcome: Result<Vec<i16>, String> = (|| {
            let (wav_path, _tmp) = media::ensure_wav(&garbage_path, 16_000)?;
            read_wav_i16_mono_16k(&wav_path)
        })();

        assert!(
            outcome.is_err(),
            "garbage bytes must not silently parse as audio"
        );
    }

    #[test]
    fn a_failing_outcome_marks_the_meeting_failed_never_leaving_it_stuck() {
        let store = temp_store();
        let meeting = store
            .create_meeting("Kaputter Import", MeetingSource::Import, None)
            .unwrap();
        assert_eq!(meeting.status, "processing");

        // `mark_import_failed` is exactly what `run_import`'s `Err` arm
        // calls (import.rs, right before `emit_error`) when the pipeline —
        // proven failing above — reports an error.
        mark_import_failed(&store, &meeting.id);

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            after.status, "failed",
            "a failed import must never leave the meeting stuck on 'processing'"
        );
    }

    #[test]
    fn a_successful_outcome_marks_the_meeting_ready() {
        let store = temp_store();
        let meeting = store
            .create_meeting("Sauberer Import", MeetingSource::Import, None)
            .unwrap();

        mark_import_ready(&store, &meeting.id).unwrap();

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(after.status, "ready");
    }

    // -- P8a: Fortschritt, Pause und Stopp zwischen den Bloecken --------------

    use super::super::job::JobRunState;
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;

    fn job_with_events() -> (Arc<JobHandle>, Arc<Mutex<Vec<MeetingEvent>>>) {
        let events: Arc<Mutex<Vec<MeetingEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let job = JobHandle::new("m1", Arc::new(move |e| sink.lock().unwrap().push(e)));
        (job, events)
    }

    /// Ein Block von `ms` Millisekunden Stille bei `offset_ms`.
    fn block(offset_ms: u64, ms: u64) -> Chunk {
        Chunk {
            samples: vec![0.1; (ms * 16) as usize],
            offset_ms,
        }
    }

    fn one_segment(text: &str) -> Vec<TimedSegment> {
        vec![TimedSegment {
            text: text.to_string(),
            start_ms: 0,
            end_ms: 500,
            words: None,
        }]
    }

    #[test]
    fn every_block_is_stored_in_order_and_the_progress_reaches_the_end_of_the_audio() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 180_000);
        let chunks = vec![block(0, 60_000), block(60_000, 60_000), block(120_000, 60_000)];
        let mut stored: Vec<Vec<StoredSegment>> = Vec::new();
        let run = drive_chunks(
            chunks,
            &job,
            0,
            CHANNEL_MIXED,
            5,
            &mut |c| one_segment(&format!("ab {}", c.offset_ms)),
            &mut |batch| {
                stored.push(batch);
                Ok(())
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(run, TranscribeRun { next_index: 8, stopped: false });
        let starts: Vec<u64> = stored.iter().map(|b| b[0].start_ms).collect();
        assert_eq!(starts, vec![0, 60_000, 120_000]);
        assert_eq!(stored[2][0].segment_index, 7, "Indizes laufen ab first_index weiter");
        let snap = job.snapshot();
        assert_eq!((snap.done, snap.total), (180_000, 180_000));
    }

    #[test]
    fn a_stop_lets_the_running_block_finish_and_skips_the_rest() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 240_000);
        let chunks = (0..4).map(|i| block(i * 60_000, 60_000)).collect();
        let mut stored: Vec<u64> = Vec::new();
        let mut transcribed: Vec<u64> = Vec::new();
        let job_for_stop = Arc::clone(&job);
        let run = drive_chunks(
            chunks,
            &job,
            0,
            CHANNEL_MIXED,
            0,
            &mut |c| {
                transcribed.push(c.offset_ms);
                if c.offset_ms == 60_000 {
                    // Der Nutzer drueckt "Stoppen", waehrend Block 2 rechnet.
                    job_for_stop.stop().unwrap();
                }
                one_segment("Wort")
            },
            &mut |batch| {
                stored.push(batch[0].start_ms);
                Ok(())
            },
            &mut || {},
        )
        .unwrap();
        assert!(run.stopped);
        assert_eq!(run.next_index, 2);
        assert_eq!(transcribed, vec![0, 60_000], "Block 3 und 4 nie transkribiert");
        assert_eq!(stored, vec![0, 60_000], "der laufende Block ist gespeichert");
        assert_eq!(job.snapshot().done, 120_000);
    }

    #[test]
    fn a_stop_before_the_first_block_transcribes_nothing() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 120_000);
        job.stop().unwrap();
        let mut calls = 0;
        let run = drive_chunks(
            vec![block(0, 60_000), block(60_000, 60_000)],
            &job,
            0,
            CHANNEL_MIXED,
            0,
            &mut |_| {
                calls += 1;
                one_segment("x")
            },
            &mut |_| Ok(()),
            &mut || {},
        )
        .unwrap();
        assert!(run.stopped);
        assert_eq!((run.next_index, calls), (0, 0));
    }

    #[test]
    fn a_pause_holds_before_the_next_block_and_resume_asks_for_the_model_once() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 240_000);
        let (entered_tx, entered_rx) = mpsc::channel::<u64>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let resumed = Arc::new(Mutex::new(0u32));
        let worker = {
            let job = Arc::clone(&job);
            let resumed = Arc::clone(&resumed);
            std::thread::spawn(move || {
                let chunks = (0..4).map(|i| block(i * 60_000, 60_000)).collect();
                drive_chunks(
                    chunks,
                    &job,
                    0,
                    CHANNEL_MIXED,
                    0,
                    &mut |c| {
                        entered_tx.send(c.offset_ms).unwrap();
                        release_rx.recv().unwrap();
                        one_segment("Wort")
                    },
                    &mut |_| Ok(()),
                    &mut || *resumed.lock().unwrap() += 1,
                )
            })
        };
        // Block 1 rechnet; waehrenddessen wird pausiert.
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(2)), Ok(0));
        job.pause().unwrap();
        release_tx.send(()).unwrap();
        // Block 2 darf NICHT beginnen: der Kontrollpunkt haelt.
        assert!(entered_rx.recv_timeout(Duration::from_millis(400)).is_err());
        assert_eq!(job.snapshot().state, JobRunState::Paused);
        assert_eq!(job.snapshot().done, 60_000, "Block 1 ist verbucht");
        job.resume().unwrap();
        for expected in [60_000, 120_000, 180_000] {
            assert_eq!(entered_rx.recv_timeout(Duration::from_secs(2)), Ok(expected));
            release_tx.send(()).unwrap();
        }
        let run = worker.join().unwrap().unwrap();
        assert!(!run.stopped);
        assert_eq!(run.next_index, 4, "kein Block verloren, keiner doppelt");
        assert_eq!(*resumed.lock().unwrap(), 1, "Modell einmal nach der Pause angefordert");
    }

    #[test]
    fn stopping_a_paused_run_ends_it_without_another_block() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 180_000);
        let (entered_tx, entered_rx) = mpsc::channel::<u64>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let worker = {
            let job = Arc::clone(&job);
            std::thread::spawn(move || {
                let chunks = (0..3).map(|i| block(i * 60_000, 60_000)).collect();
                drive_chunks(
                    chunks,
                    &job,
                    0,
                    CHANNEL_MIXED,
                    0,
                    &mut |c| {
                        entered_tx.send(c.offset_ms).unwrap();
                        release_rx.recv().unwrap();
                        one_segment("Wort")
                    },
                    &mut |_| Ok(()),
                    &mut || {},
                )
            })
        };
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(2)), Ok(0));
        job.pause().unwrap();
        release_tx.send(()).unwrap();
        assert!(entered_rx.recv_timeout(Duration::from_millis(300)).is_err());
        job.stop().unwrap();
        let run = worker.join().unwrap().unwrap();
        assert!(run.stopped);
        assert_eq!(run.next_index, 1, "nur der erste Block");
        assert!(entered_rx.try_recv().is_err());
    }

    #[test]
    fn a_storage_error_ends_the_run_with_that_error_and_no_further_block() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 120_000);
        let mut transcribed = 0;
        let result = drive_chunks(
            vec![block(0, 60_000), block(60_000, 60_000)],
            &job,
            0,
            CHANNEL_MIXED,
            0,
            &mut |_| {
                transcribed += 1;
                one_segment("Wort")
            },
            &mut |_| Err("delta_store_failed: Platte voll".to_string()),
            &mut || {},
        );
        assert_eq!(result, Err("delta_store_failed: Platte voll".to_string()));
        assert_eq!(transcribed, 1);
        assert_eq!(job.snapshot().done, 0, "ein nicht gespeicherter Block zaehlt nicht");
    }

    #[test]
    fn blocks_without_text_still_advance_the_progress() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 120_000);
        let mut stored = 0;
        let run = drive_chunks(
            vec![block(0, 60_000), block(60_000, 60_000)],
            &job,
            0,
            CHANNEL_MIXED,
            0,
            &mut |_| Vec::new(),
            &mut |_| {
                stored += 1;
                Ok(())
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!((run.next_index, stored), (0, 0));
        assert_eq!(job.snapshot().done, 120_000, "Stille ist trotzdem verarbeitet");
    }

    #[test]
    fn several_tracks_share_one_progress_axis() {
        let (job, _events) = job_with_events();
        job.begin_phase(JobPhase::Transcription, 120_000);
        let a = drive_chunks(
            vec![block(0, 60_000)],
            &job,
            0,
            0,
            0,
            &mut |_| one_segment("Wort"),
            &mut |_| Ok(()),
            &mut || {},
        )
        .unwrap();
        assert_eq!(job.snapshot().done, 60_000);
        let b = drive_chunks(
            vec![block(0, 60_000)],
            &job,
            60_000, // die zweite Spur beginnt bei 60 s der Achse des Auftrags
            1,
            a.next_index,
            &mut |_| one_segment("Wort"),
            &mut |_| Ok(()),
            &mut || {},
        )
        .unwrap();
        assert_eq!(b.next_index, 2);
        assert_eq!(job.snapshot().done, 120_000);
    }

    #[test]
    fn a_stopped_import_ends_as_cancelled_keeps_its_segments_and_gets_a_retention_date() {
        let store = temp_store();
        let meeting = store
            .create_meeting("Abgebrochener Import", MeetingSource::Import, None)
            .unwrap();
        let segment = |i: u32, text: &str| StoredSegment {
            segment_index: i,
            text: text.to_string(),
            start_ms: u64::from(i) * 1_000,
            end_ms: u64::from(i) * 1_000 + 900,
            channel: CHANNEL_MIXED,
            speaker_index: None,
            words: None,
        };
        store
            .append_delta(
                &meeting.id,
                &TranscriptDelta {
                    new_segments: vec![segment(0, "Guten Tag"), segment(1, "zusammen")],
                },
            )
            .unwrap();

        let now = 1_800_000_000;
        mark_stopped(&store, &meeting.id, now, &MeetingAudioRetention::Days(7)).unwrap();

        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(after.status, "cancelled");
        assert_eq!(after.ended_at, Some(now));
        assert_eq!(after.audio_retention_until, Some(now + 7 * 86_400));
        assert_eq!(
            store.get_segments(&meeting.id).unwrap().len(),
            2,
            "bereits transkribierte Segmente bleiben erhalten"
        );

        // Doppelt aufgerufen (Stopp waehrend des Stopps): derselbe Endzustand.
        mark_stopped(&store, &meeting.id, now + 5, &MeetingAudioRetention::Days(7)).unwrap();
        let again = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(again.status, "cancelled");
        assert_eq!(store.get_segments(&meeting.id).unwrap().len(), 2);
    }

    #[test]
    fn a_stopped_import_under_the_after_minutes_policy_keeps_its_audio_for_now() {
        let store = temp_store();
        let meeting = store
            .create_meeting("Abgebrochen", MeetingSource::Import, None)
            .unwrap();
        mark_stopped(&store, &meeting.id, 1_800_000_000, &MeetingAudioRetention::AfterMinutes)
            .unwrap();
        let after = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(after.audio_retention_until, None, "kein Protokoll, also bleibt die WAV");
        assert_eq!(after.status, "cancelled");
    }

    // ---- #15: ein gescheiterter Import hinterlaesst keine unbekannte import.wav ----

    #[test]
    fn a_failed_copy_leaves_no_half_written_import_wav_behind() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("decoded.wav");
        std::fs::write(&src, vec![1u8; 1_000]).unwrap();
        let dst = dir.path().join("import.wav");

        // Platte voll nach 10 Bytes: die Kopierfunktion hat schon geschrieben.
        let err = copy_import_wav_with(
            |_, to| {
                std::fs::write(to, [0u8; 10])?;
                Err(std::io::Error::other("disk full"))
            },
            &src,
            &dst,
        )
        .unwrap_err();
        assert!(err.starts_with("import_wav_copy_failed: "), "war: {err}");
        assert!(!dst.exists(), "keine halbe Datei");
        assert!(src.exists(), "die Quelle bleibt");

        // Und der Normalfall kopiert wirklich.
        copy_import_wav_with(|a, b| std::fs::copy(a, b), &src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap().len(), 1_000);
    }

    #[test]
    fn audio_that_could_not_be_registered_is_removed_but_registered_audio_stays() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let wav = dir.path().join("import.wav");
        std::fs::write(&wav, b"RIFF").unwrap();

        // Unbekannte Besprechung: die Datenbank kennt die Datei nicht -> weg.
        let err = register_import_audio(&store, "gibt-es-nicht", &wav, 1_000).unwrap_err();
        assert!(err.starts_with("audio_paths_failed: "), "war: {err}");
        assert!(!wav.exists(), "Debris ohne Besitzer wird entfernt");

        // Eingetragen: die Datei bleibt (spaeter ist "Neu transkribieren" moeglich).
        let meeting = store
            .create_meeting("Import", MeetingSource::Import, None)
            .unwrap();
        std::fs::write(&wav, b"RIFF").unwrap();
        register_import_audio(&store, &meeting.id, &wav, 6_000).unwrap();
        assert!(wav.exists());
        let stored = store.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(stored.mic_audio_path.as_deref(), wav.to_str());
        assert_eq!(stored.duration_ms, Some(6_000));
    }

    /// #15: das Backend lehnt einen Import ohne bestaetigte Einwilligung NICHT ab.
    /// Das ist beabsichtigt: das Gate ist ein Oberflaechen-Gate (Oberflaeche und
    /// Headless-Aufruf melden `true`); nur die Live-Aufnahme prueft `consent_gate`
    /// im Recorder. Ohne Haken bleibt `consent_confirmed_at` leer. Der Test haelt
    /// das fest, damit es niemand unbemerkt zur halben Sperre macht.
    #[test]
    fn the_backend_import_does_not_require_a_consent_confirmation_on_purpose() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let vtt = dir.path().join("ohne-einwilligung.vtt");
        std::fs::write(&vtt, "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nGuten Tag.\n").unwrap();
        let id = import_subtitle_file(&store, "ohne", &vtt, None).unwrap();
        let meeting = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(meeting.status, "ready");
        assert_eq!(meeting.consent_confirmed_at, None, "kein Haken, kein Zeitstempel");
    }

    /// #15: Fehler des Untertitel-Imports sind Codes (`code` oder `code: Detail`), keine
    /// deutschen Saetze: die Oberflaeche zeigt sie in der Sprache der App.
    #[test]
    fn subtitle_import_errors_are_codes_not_prose() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let missing = import_subtitle_file(&store, "x", &dir.path().join("gibt-es-nicht.vtt"), None)
            .unwrap_err();
        assert!(missing.starts_with("subtitle_unreadable: "), "war: {missing}");
        let bad = dir.path().join("kaputt.vtt");
        std::fs::write(&bad, "das ist kein Untertitel").unwrap();
        assert_eq!(
            import_subtitle_file(&store, "x", &bad, None).unwrap_err(),
            "subtitle_invalid"
        );
    }
}
