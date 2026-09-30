//! M9: re-running the transcription of a meeting that already has audio.
//!
//! The point is model choice: a meeting first transcribed with the streaming
//! dictation model can be redone with a batch model (or the other way round)
//! without re-importing the file. The audio the app kept is the input — so
//! this only works while the recording still exists on disk (retention
//! policy, see `retention.rs`); once purged, the transcript is all there is.
//!
//! Everything except the audio source is the import pipeline verbatim
//! (`import::transcribe_and_store`), so a re-transcribed meeting is
//! byte-for-byte the same shape as a freshly imported one.
//!
//! P8a: ein Auftrag mit Fortschritt, Pause und Stopp (`job.rs`). Stoppt der
//! Nutzer, bevor das alte Transkript ersetzt wurde, bleibt alles wie es war
//! (der vorherige Status kommt zurueck); danach endet die Besprechung als
//! `cancelled` mit den bis dahin neu transkribierten Bloecken.

use std::path::Path;
use std::sync::Arc;

use log::{error, info};
use tauri_specta::Event;

use super::import::{
    mark_stopped, read_wav_i16_mono_16k, run_speaker_step, transcribe_and_store,
};
use super::job::{self, JobHandle, JobPhase};
use super::recorder::MeetingEvent;
use super::store::{MeetingStatus, MeetingStore};
use crate::managers::transcription::TranscriptionManager;

/// `StoredSegment::channel` values, mirroring `store.rs`.
const CHANNEL_MIC: u8 = 0;
const CHANNEL_SYSTEM: u8 = 1;
const CHANNEL_MIXED: u8 = 2;

/// Re-transcribes `meeting_id` from its stored audio.
///
/// `model_id` overrides the model for this run only — `None` falls back to
/// the configured meeting model (which itself falls back to the dictation
/// model). The dictation model is restored afterwards either way, exactly as
/// the import and live-recording paths do.
pub async fn retranscribe_meeting(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<TranscriptionManager>,
    meeting_id: String,
    model_id: Option<String>,
) -> Result<(), String> {
    let meeting = store
        .get_meeting(&meeting_id)
        .map_err(|e| format!("meeting_lookup_failed: {e}"))?
        .ok_or_else(|| "meeting_not_found".to_string())?;

    // P8a: was gerade aufgenommen oder verarbeitet wird, fasst niemand an
    // (ein zweiter Lauf schriebe gleichzeitig ins selbe Transkript).
    if matches!(meeting.status.as_str(), "recording" | "processing" | "queued") {
        return Err("meeting_busy".to_string());
    }

    // Subtitle imports carry no audio at all — there is nothing to redo, and
    // silently clearing their segments would destroy the only copy.
    let audio: Vec<(String, u8)> = match (&meeting.mic_audio_path, &meeting.system_audio_path) {
        (Some(mic), Some(system)) => {
            vec![(mic.clone(), CHANNEL_MIC), (system.clone(), CHANNEL_SYSTEM)]
        }
        // A single track: the import path stores it as `mic_audio_path` and
        // labels its segments "mixed", so keep that labelling for imports.
        (Some(single), None) => {
            let channel = if meeting.source == "import" {
                CHANNEL_MIXED
            } else {
                CHANNEL_MIC
            };
            vec![(single.clone(), channel)]
        }
        (None, Some(system)) => vec![(system.clone(), CHANNEL_SYSTEM)],
        (None, None) => return Err("no_audio".to_string()),
    };

    for (path, _) in &audio {
        if !Path::new(path).exists() {
            return Err("audio_missing".to_string());
        }
    }

    let target = match model_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => tm.meeting_model_target(&crate::settings::get_settings(app)),
    };

    // P8a: der Auftrag zuerst: gibt es schon einen, geschieht nichts.
    let job = job::global()
        .try_start(&meeting_id, job::app_emit(app))
        .map_err(|e| e.to_string())?;
    let previous_status = restored_status(&meeting.status);
    store
        .set_status(&meeting_id, MeetingStatus::Processing)
        .map_err(|e| format!("status_processing_failed: {e}"))?;
    emit_state(app, &meeting_id, "processing");

    let app_owned = app.clone();
    let blocking_store = Arc::clone(&store);
    let blocking_id = meeting_id.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        let end = run_retranscribe(
            &app_owned,
            &blocking_store,
            &tm,
            &blocking_id,
            &audio,
            &target,
            job.handle(),
        );
        drop(job);
        end
    })
    .await;

    let outcome = match join {
        Ok(result) => result,
        Err(join_err) => Err(format!("meetings_retranscribe panicked: {join_err}")),
    };

    match outcome {
        Ok(RetranscribeEnd::Completed) => {
            store
                .set_status(&meeting_id, MeetingStatus::Ready)
                .map_err(|e| format!("status_ready_failed: {e}"))?;
            emit_state(app, &meeting_id, "ready");
            info!("meetings: retranscribe ready ({meeting_id})");
            Ok(())
        }
        Ok(RetranscribeEnd::Stopped { transcript_replaced }) => {
            // Nichts ersetzt: alles ist wie vor dem Start (vorheriger Status).
            // Sonst `cancelled` mit Abschluss- und Aufbewahrungsdaten.
            let stored = if transcript_replaced {
                let policy = crate::settings::get_meeting_audio_retention(app);
                mark_stopped(&store, &meeting_id, chrono::Utc::now().timestamp(), &policy)
                    .map(|()| MeetingStatus::Cancelled)
            } else {
                store
                    .set_status(&meeting_id, previous_status)
                    .map(|()| previous_status)
                    .map_err(|e| format!("status_restore_failed: {e}"))
            };
            match stored {
                Ok(status) => {
                    emit_state(app, &meeting_id, status_name(status));
                    info!("meetings: retranscribe stopped by the user ({meeting_id})");
                    Ok(())
                }
                // Der Endzustand liess sich nicht schreiben: `failed` statt
                // eines haengenden `processing`.
                Err(e) => {
                    error!("meetings: stop not stored ({meeting_id}): {e}");
                    let _ = store.set_status(&meeting_id, MeetingStatus::Failed);
                    emit_state(app, &meeting_id, "failed");
                    emit_error(app, &meeting_id, "retranscribe_failed");
                    Err(e)
                }
            }
        }
        Err(e) => {
            error!("meetings: retranscribe failed ({meeting_id}): {e}");
            let _ = store.set_status(&meeting_id, MeetingStatus::Failed);
            emit_state(app, &meeting_id, "failed");
            emit_error(app, &meeting_id, "retranscribe_failed");
            Err(e)
        }
    }
}

/// Wie eine Neu-Transkription endete, wenn sie nicht scheiterte.
#[derive(Debug, PartialEq, Eq)]
enum RetranscribeEnd {
    Completed,
    /// Der Nutzer hat gestoppt; `transcript_replaced`: das alte Transkript war
    /// schon durch (Teile des) neuen ersetzt.
    Stopped { transcript_replaced: bool },
}

/// Der Status, den eine gestoppte Neu-Transkription ohne Aenderung
/// zuruecklaesst: der vorherige Endzustand (nie `processing`).
fn restored_status(previous: &str) -> MeetingStatus {
    match previous {
        "failed" => MeetingStatus::Failed,
        "cancelled" => MeetingStatus::Cancelled,
        _ => MeetingStatus::Ready,
    }
}

fn status_name(status: MeetingStatus) -> &'static str {
    match status {
        MeetingStatus::Recording => "recording",
        MeetingStatus::Processing => "processing",
        MeetingStatus::Ready => "ready",
        MeetingStatus::Failed => "failed",
        MeetingStatus::Cancelled => "cancelled",
    }
}

/// The blocking body. Clears the old segments only once the first audio file
/// has actually been read: a meeting whose WAV turns out to be unreadable
/// keeps the transcript it had.
fn run_retranscribe(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    audio: &[(String, u8)],
    target: &str,
    job: &Arc<JobHandle>,
) -> Result<RetranscribeEnd, String> {
    job.begin_phase(JobPhase::Prepare, 0);
    tm.initiate_model_load_target(target);

    let mut tracks: Vec<(Vec<i16>, u8)> = Vec::with_capacity(audio.len());
    let mut stopped_early = false;
    for (path, channel) in audio {
        if job.is_stopped() {
            stopped_early = true;
            break;
        }
        match read_wav_i16_mono_16k(Path::new(path)) {
            Ok(samples) => tracks.push((samples, *channel)),
            Err(e) => {
                restore_dictation_model(app, tm);
                return Err(e);
            }
        }
    }
    if stopped_early || job.is_stopped() {
        restore_dictation_model(app, tm);
        return Ok(RetranscribeEnd::Stopped {
            transcript_replaced: false,
        });
    }
    let total_ms: u64 = tracks.iter().map(|(s, _)| s.len() as u64 / 16).sum();

    let result = (move || -> Result<RetranscribeEnd, String> {
        store
            .clear_segments(meeting_id)
            .map_err(|e| format!("clear_segments_failed: {e}"))?;
        let _ = (MeetingEvent::Reset {
            meeting_id: meeting_id.to_string(),
        })
        .emit(app);

        job.begin_phase(JobPhase::Transcription, total_ms);
        let mut next_index = 0u32;
        let mut base_ms = 0u64;
        for (samples, channel) in &tracks {
            let run = transcribe_and_store(
                app,
                store,
                tm,
                meeting_id,
                samples,
                *channel,
                next_index,
                job,
                base_ms,
                &mut || tm.initiate_model_load_target(target),
            )?;
            if run.stopped {
                return Ok(RetranscribeEnd::Stopped {
                    transcript_replaced: true,
                });
            }
            next_index = run.next_index;
            base_ms += samples.len() as u64 / 16;
        }
        // M3-P3b: Sprecher. Die Turns stehen in `speaker_hints_json` und ueber-
        // stehen `clear_segments` (samt Namen in `speakers`): die neuen Segmente
        // bekommen dieselben Sprecher, nur fehlende Turns werden neu berechnet.
        drop(tracks);
        run_speaker_step(app, store, meeting_id, Some(job));
        Ok(RetranscribeEnd::Completed)
    })();

    // Restore the dictation model, win or lose (mirrors import.rs).
    restore_dictation_model(app, tm);

    result
}

fn restore_dictation_model(app: &tauri::AppHandle, tm: &Arc<TranscriptionManager>) {
    let dictation_model = crate::settings::get_settings(app).selected_model;
    tm.initiate_model_load_target(&dictation_model);
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
    fn a_stop_without_any_change_restores_the_previous_final_status() {
        assert_eq!(restored_status("ready"), MeetingStatus::Ready);
        assert_eq!(restored_status("failed"), MeetingStatus::Failed);
        assert_eq!(restored_status("cancelled"), MeetingStatus::Cancelled);
        // Nie `processing` oder `recording`: ein haengender Zustand waere der Fehler.
        assert_eq!(restored_status("processing"), MeetingStatus::Ready);
        assert_eq!(restored_status("recording"), MeetingStatus::Ready);
    }

    #[test]
    fn status_names_match_the_wire_values_of_the_store() {
        for status in [
            MeetingStatus::Recording,
            MeetingStatus::Processing,
            MeetingStatus::Ready,
            MeetingStatus::Failed,
            MeetingStatus::Cancelled,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
            let meeting = store
                .create_meeting("t", super::super::store::MeetingSource::Import, None)
                .unwrap();
            store.set_status(&meeting.id, status).unwrap();
            let stored = store.get_meeting(&meeting.id).unwrap().unwrap().status;
            assert_eq!(stored, status_name(status));
        }
    }
}
