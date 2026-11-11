//! Duenne Huelle um die Terminvorbereitung (G4, #499). Die Logik steht in
//! `managers/people/prep.rs` (rein, ohne Tauri); hier stehen nur die
//! Einstellungen, die Wahl zwischen Modell und Regeln, der Modellaufruf und die
//! Erzeugung vorab.
//!
//! Systemschutz: kein neuer Kindprozess. Ein lokales Modell startet nur ueber den
//! bestehenden Weg (`llm_client`, damit RAM-Start-Gate und Job-Objekt); bei knappem
//! RAM, laufender Aufnahme auf der CPU oder laufenden KI-Notizen wird das Modell
//! gar nicht erst gefragt, es gilt der regelbasierte Rueckfall. Der Prompt ist klein
//! (hoechstens zwoelf Zeilen), ein Modellaufruf hat ein Zeitlimit. Die Sperre
//! (`MODEL_SLOT`) haelt nur den einzelnen Modellaufruf; ein Oeffnen durch den
//! Nutzer hat Vorrang vor der Erzeugung vorab, und ist das Modell besetzt, zeigt
//! die Karte sofort das Regel-Ergebnis.
//!
//! Datenschutz (ADR-0014): nichts wird gespeichert; das Ergebnis haengt am Termin
//! (Arbeitsspeicher, `prep::cache_get`, begrenzte Lebensdauer, geleert beim Loeschen
//! einer Besprechung). Ein nicht-lokales Modell bekommt nur auf Klick des Nutzers
//! etwas (`prep::decide`), vorab nie. Logzeilen nennen nur Zahlen und feste Codes.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};

use crate::managers::compliance::{self, Verdict};
use crate::managers::meetings::llm_call::{resolve_provider_coded, ResolvedProvider};
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::people::prep::{
    self, Decision, ModelChoice, OneAtATime, OpenCount, PrefetchDay, Prep, PrepMode, PrepTrigger,
    Trigger,
};
use crate::managers::usage::Purpose;
use crate::settings::{self, AppSettings};

/// Zeitlimit fuer den Modellaufruf; danach gilt der Rueckfall.
const MODEL_TIMEOUT: Duration = Duration::from_secs(60);
/// So viele Termine des Tages werden vorab vorbereitet.
const PREFETCH_MAX_EVENTS: usize = 12;

/// Ein Modellaufruf gleichzeitig (ein lokales Modell wird nicht doppelt belastet).
/// Gehalten wird die Sperre nur um den Aufruf selbst, nicht um den ganzen Lauf.
static MODEL_SLOT: OneAtATime = OneAtATime::new();
/// Laufende Oeffnungen durch den Nutzer; sie haben Vorrang vor der Erzeugung vorab.
static OPENINGS: OpenCount = OpenCount::new();
/// Ein Vorab-Lauf gleichzeitig.
static PREFETCH_RUN: OneAtATime = OneAtATime::new();
/// Der Tag, fuer den die Erzeugung vorab vollstaendig durchlief.
static PREFETCH_DAY: PrefetchDay = PrefetchDay::new();

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
    // Dieselbe Regel wie im Gateway und im Verbrauchsbuch: auch Ollama oder vLLM
    // auf localhost zaehlen als lokal.
    let local = compliance::is_local(provider);
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

/// Setzt, ob die Karte „Mit KI zusammenfassen“ anbietet: nur beim Regel-Ergebnis,
/// ohne Fehlschlag, mit Punkten und wenn ein nicht-lokales Modell bereitstuende.
fn finish(mut prep: Prep, decision: Decision, failed: bool) -> Prep {
    prep.summarizable =
        decision.summarizable && prep.mode == PrepMode::Rules && !failed && !prep.points.is_empty();
    prep
}

/// Ein Modellaufruf mit Zeitlimit. `None` bei Fehler oder Zeitlimit; im Log steht
/// nur ein fester Code, nie der Fehlertext (er kann Teile der Anfrage enthalten).
async fn call_model(resolved: ResolvedProvider, input: &prep::PrepInput) -> Option<String> {
    let (provider, model, api_key) = resolved;
    let cands = prep::model_candidates(&input.ranked);
    let call = crate::llm_client::send_chat_completion_checked(
        Purpose::Extract,
        &provider,
        api_key,
        &model,
        prep::user_prompt(&cands),
        Some(prep::system_prompt()),
        None,
        None,
        None,
    );
    match tokio::time::timeout(MODEL_TIMEOUT, call).await {
        Ok(Ok(reply)) => reply.content,
        Ok(Err(e)) => {
            log::warn!(
                "Terminvorbereitung: Modell fehlgeschlagen ({}), Regeln",
                prep::error_code(&e)
            );
            None
        }
        Err(_) => {
            log::warn!("Terminvorbereitung: Zeitlimit, Regeln");
            None
        }
    }
}

/// Erzeugt die Vorbereitung zu einem Termin. Fehler nur bei unbekanntem Termin
/// oder Speicherfehler; jede Schwierigkeit mit dem Modell endet im Rueckfall.
/// `Ok(None)` nur bei [`Trigger::Prefetch`]: der Lauf gibt auf, weil das Modell
/// besetzt ist oder der Nutzer gerade etwas oeffnet.
async fn run(
    app: &AppHandle,
    store: Arc<MeetingStore>,
    event_key: &str,
    trigger: Trigger,
) -> Result<Option<Prep>, String> {
    let settings = settings::get_settings(app);
    if !settings.meeting_prep_enabled {
        return Ok(Some(Prep::empty()));
    }
    let _opening = (trigger != Trigger::Prefetch).then(|| OPENINGS.enter());

    let own = settings.meeting_self_emails.clone();
    let now = chrono::Utc::now().timestamp();
    let input = {
        let store = Arc::clone(&store);
        let key = event_key.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let event = store
                .calendar_event(&key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "calendar_event_not_found".to_string())?;
            prep::prepare_input(&store, &event, &own, now).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??
    };
    // Kein frueherer Stoff: nichts anzeigen, kein Modell, kein Fuelltext.
    if input.is_empty() {
        return Ok(Some(Prep {
            points: Vec::new(),
            mode: PrepMode::Rules,
            meetings: input.meetings,
            summarizable: false,
        }));
    }

    let (choice, resolved) = model_choice(app, &settings).await;
    let decision = prep::decide(trigger, choice);
    // Schluessel: Termin, Fingerabdruck der Quellen, ANGEFRAGTER Modus. Beim
    // Anzeigen mit nicht-lokalem Modell gilt zusaetzlich ein frueheres Ergebnis
    // des Modells (nach Klick): es wird ohne neuen Versand wieder gezeigt.
    let hit = prep::cache_get(event_key, &input.fingerprint, decision.mode).or_else(|| {
        (decision.mode == PrepMode::Rules && decision.summarizable)
            .then(|| prep::cache_get(event_key, &input.fingerprint, PrepMode::Model))
            .flatten()
            .filter(|h| !h.failed)
    });
    if let Some(hit) = hit {
        return Ok(Some(finish(hit.prep, decision, hit.failed)));
    }

    let (result, failed) = match (decision.mode, resolved) {
        (PrepMode::Model, Some(resolved)) => {
            if trigger == Trigger::Prefetch && OPENINGS.waiting() {
                return Ok(None);
            }
            let Some(_slot) = MODEL_SLOT.try_acquire() else {
                // Besetzt: vorab geben wir auf, der Nutzer sieht sofort die Regeln
                // (nicht gemerkt, der naechste Versuch darf das Modell fragen).
                if trigger == Trigger::Prefetch {
                    return Ok(None);
                }
                return Ok(Some(finish(prep::rules_prep(&input), decision, false)));
            };
            let reply = call_model(resolved, &input).await;
            let outcome = prep::model_prep(&input, reply.as_deref());
            (outcome.prep, outcome.failed)
        }
        _ => (prep::rules_prep(&input), false),
    };
    log::info!(
        "Terminvorbereitung: {} Punkte aus {} Besprechungen ({:?})",
        result.points.len(),
        result.meetings,
        result.mode
    );
    prep::cache_put(
        event_key,
        &input.fingerprint,
        decision.mode,
        &result,
        failed,
    );
    Ok(Some(finish(result, decision, failed)))
}

/// Die Vorbereitung zu einem Termin. `view`: die Karte wird angezeigt (nur Regeln
/// oder ein lokales Modell); `click`: der Nutzer hat „Mit KI zusammenfassen“
/// gedrueckt (erst dann geht etwas an ein nicht-lokales Modell). Leer (`points`
/// leer), wenn die Funktion aus ist oder es keinen frueheren Stoff gibt: die
/// Oberflaeche zeigt dann nichts. `calendar_event_not_found` bei unbekanntem Termin.
#[tauri::command]
#[specta::specta]
pub async fn meeting_prep_get(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    event_key: String,
    trigger: PrepTrigger,
) -> Result<Prep, String> {
    run(&app, Arc::clone(&store), &event_key, trigger.into())
        .await
        .map(|prep| prep.unwrap_or_else(Prep::empty))
}

/// Einstellung „Vorbereitung vor dem Termin“ (Standard an).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_prep_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_prep_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Einstellung „Vorab fuer die Termine des Tages erzeugen“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_prep_prefetch_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
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
/// vorab geht nie etwas an einen Cloud-Anbieter (`prep::decide`). Der Tag gilt erst
/// nach vollstaendigem Durchlauf als erledigt: bei Abbruch (Aufnahme laeuft, Modell
/// besetzt, der Nutzer oeffnet gerade etwas, Kalender nicht lesbar) bleibt er offen
/// und der naechste Kalenderabruf versucht es erneut; ein leerer Kalender zaehlt als
/// vollstaendig. Schon Erzeugtes steht im Zwischenspeicher und kostet nichts.
pub async fn prefetch_today(app: AppHandle, store: Arc<MeetingStore>) {
    let settings = settings::get_settings(&app);
    if !settings.meeting_prep_enabled || !settings.meeting_prep_prefetch || is_recording(&app) {
        return;
    }
    let day = {
        use chrono::Datelike;
        i64::from(chrono::Local::now().date_naive().num_days_from_ce())
    };
    if PREFETCH_DAY.is_done(day) {
        return;
    }
    let Some(_run) = PREFETCH_RUN.try_acquire() else {
        return;
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    let events = {
        let store = Arc::clone(&store);
        let to = end_of_today_ms(now_ms);
        tauri::async_runtime::spawn_blocking(move || {
            store.calendar_events_between(now_ms, to, false)
        })
        .await
    };
    let Ok(Ok(events)) = events else {
        log::warn!("Terminvorbereitung vorab: Termine nicht lesbar");
        return;
    };
    let mut done = 0usize;
    let mut complete = true;
    for event in events
        .into_iter()
        .filter(|e| !e.all_day)
        .take(PREFETCH_MAX_EVENTS)
    {
        if !prep::prefetch_may_continue(is_recording(&app), OPENINGS.waiting()) {
            complete = false;
            break;
        }
        match run(&app, Arc::clone(&store), &event.key, Trigger::Prefetch).await {
            Ok(Some(_)) => done += 1,
            Ok(None) => {
                complete = false;
                break;
            }
            // Ein verschwundener oder unlesbarer Termin haelt den Tag nicht auf.
            Err(_) => {}
        }
    }
    PREFETCH_DAY.finish(day, complete);
    log::info!("Terminvorbereitung vorab: {done} Termine (vollstaendig: {complete})");
}
