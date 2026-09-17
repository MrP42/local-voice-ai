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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::{error, info};
use tauri_specta::Event;

use super::chunker::{ChannelChunker, Chunk};
use super::recorder::MeetingEvent;
use super::store::{MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta};
use super::subtitle::parse_subtitles;
use crate::managers::transcription::TranscriptionManager;
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
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Untertiteldatei nicht lesbar: {e}"))?;
    let segments = parse_subtitles(&content)?;

    let meeting = store
        .create_meeting(title, MeetingSource::Subtitle, consent_confirmed_at)
        .map_err(|e| format!("meeting_create_failed: {e}"))?;
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
    emit_state(app, &meeting_id, "processing");

    let app_owned = app.clone();
    let status_store = Arc::clone(&store);
    let blocking_meeting_id = meeting_id.clone();
    let join_result = tauri::async_runtime::spawn_blocking(move || {
        run_import(&app_owned, &store, &tm, &blocking_meeting_id, &path)
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

/// The blocking body: decode, copy the WAV, transcribe in chunks, finish with
/// `ready` or `failed` — always one or the other, plus the matching event.
fn run_import(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    path: &Path,
) -> Result<(), String> {
    let outcome = (|| -> Result<u64, String> {
        // Kick the model load FIRST (non-blocking) so it warms up while ffmpeg
        // decodes; `transcribe_segments` then waits on the load condvar instead
        // of failing with "Model is not loaded" — the live recorder does the
        // same in start() (recorder.rs). Without this, any import after the
        // idle unload (default 5 min) failed immediately. Meetings may use
        // their own model (`meeting_model`, dictation model as fallback).
        let target = crate::managers::transcription::TranscriptionManager::meeting_model_target(
            &crate::settings::get_settings(app),
        );
        tm.initiate_model_load_target(&target);
        let (wav_path, _tmp_guard) = media::ensure_wav(path, 16_000)?;
        let samples = read_wav_i16_mono_16k(&wav_path)?;

        let dir = super::meetings_data_dir(app)
            .map_err(|e| format!("app_data_dir_failed: {e}"))?
            .join(meeting_id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("meeting_dir_failed: {e}"))?;
        let import_wav_path = dir.join("import.wav");
        std::fs::copy(&wav_path, &import_wav_path)
            .map_err(|e| format!("import_wav_copy_failed: {e}"))?;

        transcribe_and_store(app, store, tm, meeting_id, &samples, CHANNEL_MIXED, 0)?;

        let duration_ms = (samples.len() as u64 * 1_000) / 16_000;
        store
            .set_audio_paths(
                meeting_id,
                import_wav_path.to_str(),
                None,
                Some(duration_ms),
            )
            .map_err(|e| format!("audio_paths_failed: {e}"))?;
        Ok(duration_ms)
    })();

    let result = match outcome {
        Ok(duration_ms) => {
            mark_import_ready(store, meeting_id)?;
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
    // are the same) — mirrors the live recorder's stop().
    let dictation_model = crate::settings::get_settings(app).selected_model;
    tm.initiate_model_load_target(&dictation_model);

    result
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
                    let target = TranscriptionManager::meeting_model_target(
                        &crate::settings::get_settings(app),
                    );
                    tm.initiate_model_load_target(&target);
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
    }]
}

/// Transcribes and stores each chunk in turn, same as the live worker in
/// `recorder.rs` — chunking itself happens incrementally in `chunk_all`.
pub(super) fn transcribe_and_store(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    samples: &[i16],
    channel: u8,
    first_index: u32,
) -> Result<u32, String> {
    let mut next_index: u32 = first_index;
    for chunk in chunk_all(samples, IMPORT_CHUNK_TARGET_MS) {
        let offset_ms = chunk.offset_ms;
        let timed = transcribe_chunk_resilient(app, tm, &chunk);

        let appended: Vec<StoredSegment> = timed
            .into_iter()
            .filter(|s| !s.text.trim().is_empty())
            .map(|s| {
                let segment = StoredSegment {
                    segment_index: next_index,
                    text: s.text,
                    start_ms: offset_ms + s.start_ms,
                    end_ms: offset_ms + s.end_ms,
                    channel,
                    speaker_index: None,
                };
                next_index += 1;
                segment
            })
            .collect();
        if appended.is_empty() {
            continue;
        }

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
    }
    Ok(next_index)
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

    #[test]
    fn gap_placeholder_names_the_time_span() {
        assert_eq!(
            gap_placeholder(72_000, 132_000),
            "[Nicht transkribiert 1:12–2:12 — bitte anhören und ergänzen]"
        );
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
}
