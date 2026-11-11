//! Duenne Huelle um die Terminvorbereitung (G4, #499). Die Logik steht in
//! `managers/people/prep.rs` (rein, ohne Tauri); hier stehen nur die
//! Einstellungen, die Wahl zwischen Modell und Regeln, der Modellaufruf und die
//! Erzeugung vorab.
//!
//! Systemschutz: kein neuer Kindprozess. Ein lokales Modell startet nur ueber den
//! bestehenden Weg (`llm_client`, damit RAM-Start-Gate und Job-Objekt); bei knappem
//! RAM, laufender Aufnahme auf der CPU oder laufenden KI-Notizen wird das Modell
//! gar nicht erst gefragt, es gilt der regelbasierte Rueckfall. Der Prompt ist klein
//! (hoechstens zwoelf Zeilen), ein Modellaufruf hat ein Zeitlimit.
//!
//! Datenschutz (ADR-0014): nichts wird gespeichert; das Ergebnis haengt am Termin
//! (Arbeitsspeicher, `prep::cache_get`). Logzeilen nennen nur Zahlen und Codes.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Manager, State};

use crate::managers::compliance::{self, Verdict};
use crate::managers::meetings::llm_call::{resolve_provider_coded, ResolvedProvider};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::people::prep::{self, ModelChoice, Prep, PrepMode, Trigger};
use crate::managers::usage::Purpose;
use crate::settings::{self, AppSettings};

/// Zeitlimit fuer den Modellaufruf; danach gilt der Rueckfall.
const MODEL_TIMEOUT: Duration = Duration::from_secs(60);
/// So viele Termine des Tages werden vorab vorbereitet.
const PREFETCH_MAX_EVENTS: usize = 12;

/// Ein Lauf gleichzeitig (Aufruf beim Oeffnen und Erzeugung vorab greifen nicht
/// ineinander, ein lokales Modell wird nicht doppelt belastet).
fn gate() -> &'static tauri::async_runtime::Mutex<()> {
    static GATE: OnceLock<tauri::async_runtime::Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| tauri::async_runtime::Mutex::new(()))
}

fn is_recording(app: &AppHandle) -> bool {
    app.try_state::<Arc<MeetingRecorderManager>>()
        .is_some_and(|r| r.is_recording())
}

/// Was ueber das gewaehlte Modell feststeht, samt dem aufloesten Anbieter.
async fn model_choice(
    app: &AppHandle,
    settings: &AppSettings,
) -> (ModelChoice, Option<ResolvedProvider>) {
    let Ok(resolved) = resolve_provider_coded(settings) else {
        let none = ModelChoice {
            configured: false,
            local: false,
            allowed: false,
            busy: false,
        };
        return (none, None);
    };
    let provider = &resolved.0;
    let local = crate::managers::llm::is_local(provider);
    let allowed = compliance::assess(
        settings.compliance_profile,
        provider,
        compliance::training_opt_out_for(settings, provider),
        &compliance::today(),
    )
    .verdict
        != Verdict::Blocked;
    let ram_low = local && {
        let free = crate::process_guard::available_ram_mb();
        free != 0 && free < crate::process_guard::ram_reserve_mb()
    };
    let mut busy = ram_low || crate::commands::meeting_chat::enhance_running();
    if !busy && local && is_recording(app) {
        busy = crate::commands::meeting_chat::local_backend_is_cpu(app).await;
    }
    let choice = ModelChoice {
        configured: true,
        local,
        allowed,
        busy,
    };
    (choice, Some(resolved))
}

/// Erzeugt die Vorbereitung zu einem Termin. Fehler nur bei unbekanntem Termin
/// oder Speicherfehler; jede Schwierigkeit mit dem Modell endet im Rueckfall.
pub async fn run_prep(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    event_key: &str,
    trigger: Trigger,
) -> Result<Prep, String> {
    let settings = settings::get_settings(app);
    if !settings.meeting_prep_enabled {
        return Ok(Prep::empty());
    }
    let _one_at_a_time = gate().lock().await;

    let own = settings.meeting_self_emails.clone();
    let now = chrono::Utc::now().timestamp();
    let (title, input) = {
        let store = Arc::clone(&store);
        let key = event_key.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let event = store
                .calendar_event(&key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "calendar_event_not_found".to_string())?;
            let input =
                prep::prepare_input(&store, &event, &own, now).map_err(|e| e.to_string())?;
            Ok::<_, String>((event.title, input))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    // Kein frueherer Stoff: nichts anzeigen, kein Modell, kein Fuelltext.
    if input.is_empty() {
        return Ok(Prep {
            points: Vec::new(),
            mode: PrepMode::Rules,
            meetings: input.meetings,
        });
    }

    let (choice, resolved) = model_choice(app, &settings).await;
    let mode = prep::decide_mode(trigger, choice);
    if let Some(hit) = prep::cache_get(event_key, &input.fingerprint, mode) {
        return Ok(hit);
    }

    let result = match (mode, resolved) {
        (PrepMode::Model, Some((provider, model, api_key))) => {
            let cands = prep::model_candidates(&input.ranked);
            let user = prep::user_prompt(&title, &cands);
            let call = crate::llm_client::send_chat_completion_checked(
                Purpose::Extract,
                &provider,
                api_key,
                &model,
                user,
                Some(prep::system_prompt()),
                None,
                None,
                None,
            );
            let reply = match tokio::time::timeout(MODEL_TIMEOUT, call).await {
                Ok(Ok(reply)) => reply.content,
                Ok(Err(e)) => {
                    // Nur die Art, nie der Text (kann Teile der Anfrage enthalten).
                    let code = e.split(':').next().unwrap_or("error");
                    log::warn!("Terminvorbereitung: Modell fehlgeschlagen ({code}), Regeln");
                    None
                }
                Err(_) => {
                    log::warn!("Terminvorbereitung: Zeitlimit, Regeln");
                    None
                }
            };
            prep::model_prep(&input, reply.as_deref())
        }
        _ => prep::rules_prep(&input),
    };
    log::info!(
        "Terminvorbereitung: {} Punkte aus {} Besprechungen ({:?})",
        result.points.len(),
        result.meetings,
        result.mode
    );
    prep::cache_put(event_key, &input.fingerprint, &result);
    Ok(result)
}

/// Die Vorbereitung zu einem Termin (Oeffnen des Termins bzw. der Aufnahme).
/// Leer (`points` leer), wenn die Funktion aus ist oder es keinen frueheren
/// Stoff gibt: die Oberflaeche zeigt dann nichts. `calendar_event_not_found` bei
/// unbekanntem Termin.
#[tauri::command]
#[specta::specta]
pub async fn meeting_prep_get(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    event_key: String,
) -> Result<Prep, String> {
    run_prep(&app, Arc::clone(&store), &event_key, Trigger::Open).await
}

/// Einstellung „Vorbereitung vor dem Termin“ (Standard an).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_prep_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_prep_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Einstellung „Vorab fuer die Termine des Tages erzeugen“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_prep_prefetch_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_prep_prefetch = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Ende des heutigen Kalendertags (lokal) in Millisekunden; im Zweifel jetzt + 24 h.
fn end_of_today_ms(now_ms: i64) -> i64 {
    use chrono::{Local, TimeZone};
    Local::now()
        .date_naive()
        .and_hms_opt(23, 59, 59)
        .and_then(|t| Local.from_local_datetime(&t).single())
        .map_or(now_ms + 24 * 3_600_000, |t| t.timestamp_millis())
}

/// Erzeugt die Vorbereitung fuer die restlichen Termine des Tages, einmal je
/// Kalendertag (nach dem ersten Kalenderabruf, den der Takt des Kalenders
/// ausloest). Nur mit eingeschalteter Funktion UND eingeschaltetem Vorab-Schalter;
/// waehrend einer Aufnahme wird nichts gestartet (der Tag bleibt dann offen), und
/// vorab geht nie etwas an einen Cloud-Anbieter (`prep::decide_mode`).
pub async fn prefetch_today(app: AppHandle, store: Arc<MeetingStore>) {
    static LAST_DAY: AtomicI64 = AtomicI64::new(0);
    let settings = settings::get_settings(&app);
    if !settings.meeting_prep_enabled || !settings.meeting_prep_prefetch || is_recording(&app) {
        return;
    }
    let day = {
        use chrono::Datelike;
        i64::from(chrono::Local::now().date_naive().num_days_from_ce())
    };
    if LAST_DAY.swap(day, Ordering::SeqCst) == day {
        return;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    let events = {
        let store = Arc::clone(&store);
        let to = end_of_today_ms(now_ms);
        tauri::async_runtime::spawn_blocking(move || store.calendar_events_between(now_ms, to, false))
            .await
    };
    let Ok(Ok(events)) = events else {
        log::warn!("Terminvorbereitung vorab: Termine nicht lesbar");
        return;
    };
    let mut done = 0usize;
    for event in events
        .into_iter()
        .filter(|e| !e.all_day)
        .take(PREFETCH_MAX_EVENTS)
    {
        if is_recording(&app) {
            break;
        }
        if run_prep(&app, Arc::clone(&store), &event.key, Trigger::Prefetch)
            .await
            .is_ok()
        {
            done += 1;
        }
    }
    log::info!("Terminvorbereitung vorab: {done} Termine");
}
