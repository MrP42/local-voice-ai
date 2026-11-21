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
//!
//! A3 / B17: das alte Transkript geht nie mehr verloren. Vor dem ersten neuen Block
//! sichert `variants::begin_rerun` es als Fassung; am Ende wird das Ergebnis die
//! NEUE aktive Fassung (`finish_rerun`), bei Stopp oder Fehler stellt `abort_rerun`
//! die alte wieder her (der vorherige Status kommt zurueck, auch nach einem
//! Stopp mitten im Lauf). Dieselbe Pipeline traegt die eigene Transkription eines
//! YouTube-Videos (`RerunSource::Youtube`): Audio per selbst installiertem yt-dlp
//! in einen Temp-Ordner, dekodiert, transkribiert, Temp-Datei geloescht.
//!
//! lva#523: ein STOPP durch den Nutzer wirft das neu Transkribierte nicht mehr weg.
//! Gibt es schon neue Abschnitte DIESES Laufs, werden sie eine eigene aktive Fassung,
//! die alte bleibt als Fassung erhalten, und die Besprechung endet als `cancelled`;
//! „Fortsetzen“ holt den Rest mit dem Modell des Laufs nach. Stopp, Fehler und
//! Absturz laufen ueber dieselbe Zustandsmaschine (`rerun_state`), immer unter dem
//! Auftrag der Besprechung. YouTube hat keine gespeicherte Spur: dort bleibt das
//! Teilergebnis als Fassung, aber ohne Fortsetzen (vorheriger Status).

use std::path::Path;
use std::sync::Arc;

use log::{error, info};
use tauri_specta::Event;

use super::import::{read_wav_i16_mono_16k, run_speaker_step, transcribe_and_store};
use super::job::{self, JobHandle, JobPhase};
use super::language_run::{self, RunRequest};
use super::recorder::MeetingEvent;
use super::rerun_state::{self, Ends, Settled};
use super::store::{MeetingStatus, MeetingStore};
use super::trim::{self, TimeRange};
use super::variants;
use crate::managers::transcription::TranscriptionManager;
use crate::managers::youtube::fetch as yt_fetch;

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
///
/// G5: `language` ist die Wahl im Dialog (`auto` oder leer: erkennen). Ein Sprachcode gilt
/// vor der Einstellung und vor jeder Erkennung; ohne ausdrueckliches `model_id` waehlt die
/// App das passende Modell der Sprache (`language_run::prepare`).
///
/// lva#522: mit `range` wird nur dieser Bereich der Aufnahme transkribiert (jede Spur
/// auf `[start_ms, end_ms)` geschnitten); die Segmente behalten die ABSOLUTEN Zeiten der
/// Aufnahme. Das bisherige Transkript bleibt wie sonst als Fassung erhalten. Ein
/// ungueltiger Bereich (`start_ms >= end_ms`, Anfang hinter dem Ende) endet mit `invalid_range`.
pub async fn retranscribe_meeting(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<TranscriptionManager>,
    meeting_id: String,
    model_id: Option<String>,
    language: Option<String>,
    range: Option<TimeRange>,
) -> Result<(), String> {
    rerun_meeting(
        app,
        store,
        tm,
        meeting_id,
        model_id,
        language,
        RerunSource::Stored,
        range,
    )
    .await
}

/// Woher das Audio eines Laufs kommt.
pub enum RerunSource {
    /// Die gespeicherte Aufnahme oder Importdatei der Besprechung.
    Stored,
    /// A3: das Video einer YouTube-Besprechung ueber ein selbst installiertes yt-dlp.
    Youtube {
        exe: std::path::PathBuf,
        video_id: String,
        tool_version: Option<String>,
    },
}

/// Die Eingabe des blockierenden Laufs.
enum Input {
    Files(Vec<(String, u8)>),
    Youtube {
        exe: std::path::PathBuf,
        video_id: String,
        tool_version: Option<String>,
        guard: yt_fetch::Guard,
    },
}

/// Neu-Transkription aus gespeichertem Audio oder (A3) aus einem YouTube-Video.
#[allow(clippy::too_many_arguments)]
pub async fn rerun_meeting(
    app: &tauri::AppHandle,
    store: Arc<MeetingStore>,
    tm: Arc<TranscriptionManager>,
    meeting_id: String,
    model_id: Option<String>,
    language: Option<String>,
    source: RerunSource,
    range: Option<TimeRange>,
) -> Result<(), String> {
    // lva#522: ein ungueltiger Bereich scheitert vor jeder Aenderung.
    let range = range.map(TimeRange::validate).transpose()?;
    let request = RunRequest {
        language,
        model_id: model_id.clone(),
    };
    let meeting = store
        .get_meeting(&meeting_id)
        .map_err(|e| format!("meeting_lookup_failed: {e}"))?
        .ok_or_else(|| "meeting_not_found".to_string())?;

    // P8a: was gerade aufgenommen oder verarbeitet wird, fasst niemand an
    // (ein zweiter Lauf schriebe gleichzeitig ins selbe Transkript).
    if matches!(
        meeting.status.as_str(),
        "recording" | "processing" | "queued"
    ) {
        return Err("meeting_busy".to_string());
    }

    // Subtitle imports carry no audio at all — there is nothing to redo, and
    // silently clearing their segments would destroy the only copy.
    let audio: Vec<(String, u8)> = match (&meeting.mic_audio_path, &meeting.system_audio_path) {
        _ if matches!(source, RerunSource::Youtube { .. }) => {
            if meeting.source != "youtube" {
                return Err("not_a_youtube_meeting".to_string());
            }
            Vec::new()
        }
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
    // lva#522: ein Bereich hinter dem Ende der Aufnahme ergaebe ein leeres Transkript.
    if let (Some(range), Some(duration_ms)) = (range, meeting.duration_ms) {
        if duration_ms > 0 && range.start_ms >= duration_ms {
            return Err("invalid_range".to_string());
        }
    }

    let target = match model_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => tm.meeting_model_target(&crate::settings::get_settings(app)),
    };

    // A3: Tor und Audit `media.fetch` VOR dem Auftrag und vor dem ersten Byte.
    let (input, youtube_guard_failed) = match source {
        RerunSource::Stored => (Input::Files(audio), None),
        RerunSource::Youtube {
            exe,
            video_id,
            tool_version,
        } => match yt_fetch::begin(&store, &video_id, "audio") {
            Ok(guard) => (
                Input::Youtube {
                    exe,
                    video_id,
                    tool_version,
                    guard,
                },
                None,
            ),
            Err(e) => (Input::Files(Vec::new()), Some(e)),
        },
    };
    if let Some(e) = youtube_guard_failed {
        return Err(e.to_command_error());
    }
    let has_stored_audio = meeting.mic_audio_path.is_some() || meeting.system_audio_path.is_some();
    // lva#534: Einwilligungs-Tor der Besprechungsplanung, dann Auftrag, Marke und Status unter
    // derselben Sperre (siehe `begin_run`). Erst danach laeuft Transkription.
    let (job, previous_status) = match begin_run(&store, &meeting_id, range, job::app_emit(app)) {
        Ok(started) => started,
        Err(e) => {
            if let Input::Youtube { guard, .. } = input {
                yt_fetch::end(
                    &store,
                    guard,
                    &Err(crate::managers::youtube::YoutubeError::Cancelled),
                );
            }
            return Err(e);
        }
    };
    emit_state(app, &meeting_id, "processing");

    let app_owned = app.clone();
    let blocking_store = Arc::clone(&store);
    let blocking_id = meeting_id.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_retranscribe(
                &app_owned,
                &blocking_store,
                &tm,
                &blocking_id,
                input,
                &target,
                &request,
                range,
                job.handle(),
            )
        }))
        .unwrap_or_else(|_| Err("meetings_retranscribe panicked".to_string()));
        // lva#523: der Abschluss (behalten, zuruecknehmen, Status) geschieht noch
        // unter dem Auftrag: niemand sonst schreibt dazwischen.
        let end = finish_run(
            &app_owned,
            &blocking_store,
            &blocking_id,
            run,
            previous_status,
            has_stored_audio,
        );
        drop(job);
        end
    })
    .await;

    match join {
        Ok(result) => result,
        Err(join_err) => Err(format!("meetings_retranscribe panicked: {join_err}")),
    }
}

/// Der Beginn eines Neu-Transkriptions-Laufs, unter dem Einwilligungs-Tor der
/// Besprechungsplanung (lva#534): Ist die Besprechung einer Planung zugeordnet
/// (`meeting_calendar_links`, `meeting_plans.meeting_id`), die nicht `MitTranskription` ergibt
/// (nicht geplant, Ablehnung, fehlende Zustimmung, unbekannte Person), endet der Aufruf mit
/// `nur_notizen: <grund>`, bevor ein Auftrag, eine Marke oder ein Status entstehen; Speech-to-Text
/// laeuft dann nie. Tor, Auftrag, Marke und Status `processing` stehen unter einer Sperre: ein
/// Widerruf wartet bis nach dem Beginn und gilt fuer den naechsten Lauf. Ohne zugeordnete
/// Planung aendert sich nichts.
pub(crate) fn begin_run(
    store: &MeetingStore,
    meeting_id: &str,
    range: Option<TimeRange>,
    emit: job::EmitFn,
) -> Result<(job::JobGuard, MeetingStatus), String> {
    // Ein Auftrag, der unter der Sperre entstand und dann doch nicht beginnt, endet erst NACH
    // ihr: sein Ende meldet sich an die Oberflaeche (`JobEnded`), und unter der Tor-Sperre laeuft
    // nur Datenbankarbeit (Sperrordnung, `managers::planning`).
    let mut verworfen: Option<job::JobGuard> = None;
    let begonnen = crate::managers::planning::mit_tor_besprechung(store, meeting_id, |_tor| {
        // P8a: der Auftrag zuerst: gibt es schon einen, geschieht nichts. (Das Verzeichnis der
        // Auftraege ist eine Blatt-Sperre: kurz, ohne Warten, nie ueber die Tor-Sperre gehalten.)
        let job = job::global()
            .try_start(meeting_id, emit)
            .map_err(|e| e.to_string())?;
        // lva#523 / A3: ein offener frueherer Lauf (Absturz, gescheiterter Abschluss) wird
        // unter DIESEM Auftrag abgeschlossen, bevor etwas Neues beginnt; dann die Marke
        // `start` mit dem Stand davor (fuer das Zuruecknehmen). Der Vorstatus kommt aus dem
        // Zustand NACH diesem Abschluss, unter dem Auftrag gelesen.
        let markiert = rerun_state::mark_start(store, meeting_id, range).and_then(|previous| {
            store
                .set_status(meeting_id, MeetingStatus::Processing)
                .map_err(|e| format!("status_processing_failed: {e}"))?;
            Ok(previous)
        });
        match markiert {
            Ok(previous_status) => Ok((job, previous_status)),
            Err(e) => {
                verworfen = Some(job);
                Err(e)
            }
        }
    });
    drop(verworfen);
    begonnen
}

/// lva#523: Endzustand eines Laufs, unter seinem Auftrag.
fn finish_run(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    meeting_id: &str,
    outcome: Result<RetranscribeEnd, String>,
    previous_status: MeetingStatus,
    has_stored_audio: bool,
) -> Result<(), String> {
    match outcome {
        // Fassung, Marke und `ready` stehen schon (`rerun_state::complete`).
        Ok(RetranscribeEnd::Completed) => {
            emit_state(app, meeting_id, "ready");
            info!("meetings: retranscribe ready ({meeting_id})");
            Ok(())
        }
        Ok(RetranscribeEnd::Stopped) => {
            // lva#523: behalten (mit Spur fortsetzbar, `cancelled`) oder zuruecknehmen
            // (Vorstatus), jeweils mit Marke und Status in EINER Transaktion.
            let kept_status = if has_stored_audio {
                MeetingStatus::Cancelled
            } else {
                previous_status
            };
            let ends = Ends {
                kept: Some(kept_status),
                resumable: has_stored_audio,
                fallback: Some(previous_status),
            };
            let status = match rerun_state::settle(store, meeting_id, false, ends) {
                Ok(Settled::Kept(_)) => kept_status,
                Ok(Settled::RolledBack(status)) => status.unwrap_or(previous_status),
                Ok(Settled::Nothing) => {
                    store
                        .set_status(meeting_id, previous_status)
                        .map_err(|e| format!("status_restore_failed: {e}"))?;
                    previous_status
                }
                // Nichts veraendert: Arbeitskopie und alte Fassung bleiben beide; ein
                // spaeterer Abschluss (Fortsetzen, neuer Lauf, Fassungsliste) holt es nach.
                Err(e) => {
                    error!("meetings: stop not settled ({meeting_id}): {e}");
                    let _ = store.set_status(meeting_id, MeetingStatus::Cancelled);
                    emit_reset(app, meeting_id);
                    emit_state(app, meeting_id, "cancelled");
                    emit_error(app, meeting_id, "retranscribe_failed");
                    return Err(e);
                }
            };
            emit_reset(app, meeting_id);
            emit_state(app, meeting_id, status_name(status));
            info!("meetings: retranscribe stopped by the user ({meeting_id})");
            Ok(())
        }
        Err(e) => {
            error!("meetings: retranscribe failed ({meeting_id}): {e}");
            // B17 / lva#523: das alte Transkript zurueck, aber erst nach dauerhaft
            // gespeicherter Absicht (`rerun_state::fail_run`). Ohne sie beginnt kein
            // Zuruecknehmen; der Stand bleibt, wie er ist.
            let status = match rerun_state::fail_run(store, meeting_id, previous_status) {
                Ok(status) => status,
                Err(err) => {
                    error!("meetings: rollback not started ({meeting_id}): {err}");
                    MeetingStatus::Failed
                }
            };
            emit_reset(app, meeting_id);
            emit_state(app, meeting_id, status_name(status));
            emit_error(app, meeting_id, "retranscribe_failed");
            Err(e)
        }
    }
}

/// Wie eine Neu-Transkription endete, wenn sie nicht scheiterte.
#[derive(Debug, PartialEq, Eq)]
enum RetranscribeEnd {
    Completed,
    /// Der Nutzer hat gestoppt. Was davon bleibt, entscheidet `rerun_state::settle`
    /// anhand der Marke (lva#523).
    Stopped,
}

/// Der Status, den eine gestoppte Neu-Transkription ohne Aenderung
/// zuruecklaesst: der vorherige Endzustand (nie `processing`).
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
#[allow(clippy::too_many_arguments)]
fn run_retranscribe(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    tm: &Arc<TranscriptionManager>,
    meeting_id: &str,
    input: Input,
    target: &str,
    request: &RunRequest,
    range: Option<TimeRange>,
    job: &Arc<JobHandle>,
) -> Result<RetranscribeEnd, String> {
    // lva#544: dieser Thread schickt ab hier Ton der Besprechung durch Modelle und speichert die
    // Ergebnisse: jede Aenderung am Transkript prueft das Einwilligungs-Tor beim Schreiben.
    let _arbeit = crate::managers::planning::Modellarbeit::beginnen(meeting_id);
    job.begin_phase(JobPhase::Prepare, 0);
    tm.initiate_model_load_target(target);

    let (audio, youtube) = match input {
        Input::Files(audio) => (audio, None),
        Input::Youtube {
            exe,
            video_id,
            tool_version,
            guard,
        } => (Vec::new(), Some((exe, video_id, tool_version, guard))),
    };
    let mut tracks: Vec<(Vec<i16>, u8)> = Vec::with_capacity(audio.len());
    let mut stopped_early = false;
    let mut youtube_ref: Option<(String, Option<String>)> = None;
    if let Some((exe, video_id, tool_version, guard)) = youtube {
        // Audio ins Temp, dekodieren, in den Speicher lesen; die Datei verschwindet
        // sofort danach (nicht erst nach der langen Transkription).
        let cancel = job.stop_flag();
        let fetched = yt_fetch::download_audio(
            &yt_fetch::ToolRun {
                exe: &exe,
                temp_base: None,
            },
            &video_id,
            &cancel,
        );
        yt_fetch::end(
            store,
            guard,
            &fetched.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        let downloaded = match fetched {
            Ok(d) => d,
            Err(crate::managers::youtube::YoutubeError::Cancelled) => {
                restore_dictation_model(app, tm);
                return Ok(RetranscribeEnd::Stopped);
            }
            Err(e) => {
                restore_dictation_model(app, tm);
                return Err(e.to_command_error());
            }
        };
        let decoded = crate::media::ensure_wav_cancellable(&downloaded.file, 16_000, &cancel);
        let samples = match decoded {
            Ok((wav, _tmp)) => read_wav_i16_mono_16k(&wav).map(|mut samples| {
                if let Some(range) = range {
                    trim::cut_in_place(&mut samples, range);
                }
                samples
            }),
            Err(e) if e == crate::media::DECODE_CANCELLED => {
                restore_dictation_model(app, tm);
                return Ok(RetranscribeEnd::Stopped);
            }
            Err(e) => Err(e),
        };
        drop(downloaded);
        match samples {
            Ok(samples) => tracks.push((samples, CHANNEL_MIXED)),
            Err(e) => {
                restore_dictation_model(app, tm);
                return Err(e);
            }
        }
        youtube_ref = Some((video_id, tool_version));
    }
    for (path, channel) in &audio {
        if job.is_stopped() {
            stopped_early = true;
            break;
        }
        // lva#522: mit Bereich wird nur dieser Abschnitt gelesen (nicht die ganze Aufnahme).
        let read = match range {
            Some(range) => trim::read_track_range(Path::new(path), range),
            None => read_wav_i16_mono_16k(Path::new(path)),
        };
        match read {
            Ok(samples) => tracks.push((samples, *channel)),
            Err(e) => {
                restore_dictation_model(app, tm);
                return Err(e);
            }
        }
    }
    if stopped_early || job.is_stopped() {
        restore_dictation_model(app, tm);
        return Ok(RetranscribeEnd::Stopped);
    }
    // lva#522: eine Spur, die im Bereich nichts hat (kuerzer als der Anfang), entfaellt;
    // bleibt keine, ist der Bereich ungueltig. Noch ist nichts veraendert.
    if range.is_some() {
        tracks.retain(|(samples, _)| !samples.is_empty());
        if tracks.is_empty() {
            restore_dictation_model(app, tm);
            return Err("invalid_range".to_string());
        }
    }
    // Beginn der geschnittenen Spuren auf der Achse der Aufnahme.
    let time_offset_ms = range.map_or(0, |r| r.start_ms);
    let total_ms: u64 = tracks.iter().map(|(s, _)| s.len() as u64 / 16).sum();
    let started = std::time::Instant::now();
    // G5: Sprache und Modell dieses Laufs (Dialog, Einstellung, sonst eine Probe aus dem
    // laengsten Spur). Ab hier rechnet der Lauf mit `plan.model_id`; die Vorgabe gilt, solange
    // `plan` lebt.
    let plan = {
        let longest = tracks
            .iter()
            .max_by_key(|(s, _)| s.len())
            .map(|(s, _)| s.as_slice());
        // lva#534: die Probe rechnet ein Modell ueber das Audio: Einwilligungs-Tor je Fenster.
        // Eine Ablehnung hier aendert noch nichts (das alte Transkript steht).
        let probe = super::language_run::prepare(
            app,
            tm,
            store,
            meeting_id,
            longest.unwrap_or(&[]),
            request,
            Some(job),
        );
        match probe {
            Ok(plan) => plan,
            Err(e) => {
                restore_dictation_model(app, tm);
                return Err(e);
            }
        }
    };
    let target_owned = plan.model_id.clone();
    let target = target_owned.as_str();
    // Ein Stopp waehrend der Probe: noch nichts ist veraendert, das alte Transkript bleibt.
    if job.is_stopped() {
        restore_dictation_model(app, tm);
        return Ok(RetranscribeEnd::Stopped);
    }

    // A3: eigene Transkription eines Videos oder Neu-Transkription.
    let kind = if youtube_ref.is_some() {
        variants::KIND_OWN
    } else {
        variants::KIND_RETRANSCRIBED
    };
    let result = (move || -> Result<RetranscribeEnd, String> {
        // lva#544: Sichern, Leeren und die Marke `running` sind EIN Schritt unter dem
        // Einwilligungs-Tor. Lehnt es ab (Widerruf seit dem Beginn), geschieht keiner davon:
        // Fassungen, Transkript und Marke bleiben, wie sie sind.
        crate::managers::planning::mit_tor_besprechung(store, meeting_id, |_tor| {
            // B17: das alte Transkript als Fassung sichern, bevor der Lauf in
            // `transcripts` schreibt.
            {
                let mut conn = store
                    .get_connection()
                    .map_err(|e| format!("variants_begin_failed: {e}"))?;
                variants::begin_rerun(&mut conn, meeting_id)
                    .map_err(|e| format!("variants_begin_failed: {e}"))?;
            }
            store
                .clear_segments(meeting_id)
                .map_err(|e| format!("clear_segments_failed: {e}"))?;
            // lva#523: ab hier gehoert die Arbeitskopie (diese Epoche) DIESEM Lauf; erst
            // jetzt darf ein Stopp oder Absturz sie als neue Abschnitte behalten.
            rerun_state::mark_running(store, meeting_id, target, kind, range)
        })?;
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
                time_offset_ms,
                &mut || tm.initiate_model_load_target(target),
            )?;
            if run.stopped {
                return Ok(RetranscribeEnd::Stopped);
            }
            next_index = run.next_index;
            base_ms += samples.len() as u64 / 16;
        }
        // M3-P3b: Sprecher. Die Turns stehen in `speaker_hints_json` und ueber-
        // stehen `clear_segments` (samt Namen in `speakers`): die neuen Segmente
        // bekommen dieselben Sprecher, nur fehlende Turns werden neu berechnet.
        drop(tracks);
        run_speaker_step(app, store, meeting_id, Some(job));
        // A1: Herkunft des neuen Transkripts (Modell, Dauer, Quelle).
        let (operation, sources, params) = match &youtube_ref {
            Some((video_id, tool_version)) => {
                let mut source =
                    crate::managers::provenance::SourceRef::new("youtube", video_id, None);
                source.url = Some(crate::managers::youtube::subtitles::watch_url(video_id));
                (
                    "youtube_audio",
                    vec![source],
                    trim::params_with_range(
                        serde_json::json!({
                            "audio_ms": total_ms,
                            "tool": "yt-dlp",
                            "tool_version": tool_version,
                        }),
                        range,
                    ),
                )
            }
            None => (
                "retranscribe",
                vec![crate::managers::provenance::SourceRef::new(
                    "audio", meeting_id, None,
                )],
                trim::params_with_range(serde_json::json!({ "audio_ms": total_ms }), range),
            ),
        };
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let run = || crate::managers::provenance::generation::SttRun {
            meeting_id,
            operation,
            actor_kind: crate::managers::provenance::ActorKind::User,
            model_id: target,
            revision: None,
            duration_ms,
            sources: sources.clone(),
            params: params.clone(),
        };
        crate::managers::provenance::generation::record_stt(store, run());
        // B17: das Ergebnis wird erst JETZT die neue aktive Fassung.
        // lva#523: Fassung, Marke weg und `ready` in einer Transaktion.
        // lva#544: unter dem Einwilligungs-Tor: nach einem Widerruf wird das Ergebnis nie die
        // aktive Fassung (der Lauf endet als Fehler und wird zurueckgenommen).
        let variant_id =
            crate::managers::planning::mit_tor_besprechung(store, meeting_id, |_tor| {
                rerun_state::complete(store, meeting_id, kind)
            })?;
        crate::managers::provenance::generation::record_stt_variant(store, &variant_id, run());
        // G5: die gueltige Sprache (Gegenprobe am Text) und das Modell an Besprechung,
        // Transkript und die neue aktive Fassung.
        language_run::finalize(store, meeting_id, &plan, tm.get_current_model());
        Ok(RetranscribeEnd::Completed)
    })();

    // Restore the dictation model, win or lose (mirrors import.rs).
    restore_dictation_model(app, tm);

    result
}

fn emit_reset(app: &tauri::AppHandle, meeting_id: &str) {
    let _ = (MeetingEvent::Reset {
        meeting_id: meeting_id.to_string(),
    })
    .emit(app);
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
        assert_eq!(
            rerun_state::restorable_status("ready"),
            MeetingStatus::Ready
        );
        assert_eq!(
            rerun_state::restorable_status("failed"),
            MeetingStatus::Failed
        );
        assert_eq!(
            rerun_state::restorable_status("cancelled"),
            MeetingStatus::Cancelled
        );
        // Nie `processing` oder `recording`: ein haengender Zustand waere der Fehler.
        assert_eq!(
            rerun_state::restorable_status("processing"),
            MeetingStatus::Ready
        );
        assert_eq!(
            rerun_state::restorable_status("recording"),
            MeetingStatus::Ready
        );
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
