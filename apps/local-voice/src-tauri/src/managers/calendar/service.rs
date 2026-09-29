//! Sync-Dienst des Kalenders (M5, P5b; `entwurf/m5-m6-kalender-export.md` §3 F15,
//! §5, §7): holt die ICS-Quellen alle 15 Minuten (und auf Knopfdruck), schreibt
//! sie in den Cache und erinnert 1 min vor Terminen.
//!
//! Aufbau: die Entscheidungen stehen in freien, testbaren Funktionen
//! (`apply_parsed`, `pending_reminders`, `plan_start`, `finish_start`); der
//! Dienst selbst (`CalendarService`) ist nur die Schleife darum.
//!
//! Fehlerfaelle und ihre Absicherung:
//! - Abruf scheitert (kein Netz, 401/404, HTML, zu gross): der Cache bleibt, die
//!   Quelle bekommt den Klartext (`SyncMark::Failed`), der naechste Versuch kommt im
//!   Intervall; kein Hinweisfenster (Tests `failed_parse_keeps_the_cache`).
//! - Parser stuerzt ab (Panik in der Bibliothek): eigener Thread mit
//!   `catch_unwind`, die App laeuft weiter, die Quelle meldet einen Fehler (Test
//!   `run_guarded_turns_a_panic_into_an_error`).
//! - Zwei Laeufe gleichzeitig (Zeitgeber + „Jetzt aktualisieren“ + Hinzufuegen):
//!   EIN Mutex (`gate`) serialisiert sie; zusaetzlich ist jeder Schreibweg des
//!   Stores eine `IMMEDIATE`-Transaktion.
//! - Ruhezustand/Uhrzeitwechsel: die Erinnerung ist zeitgeberbasiert und holt
//!   Termine, die vor mehr als 2 min begannen, nicht nach (`reminder.rs`).
//! - Hinweisfenster schon offen: der Tick uebergeht die Erinnerung, bis es zu ist;
//!   der Termin bleibt bis 2 min nach Beginn faellig.
//! - Geheimnis fehlt oder gehoert einem anderen Benutzer: „Adresse neu eingeben“
//!   (`CalendarError::Auth`), kein Absturz.
//!
//! Was bei vollem RAM passiert: vor jedem Abruf prueft `process_guard::
//! check_ram_for_start` den freien Speicher; ist er knapp, wird dieser Lauf mit
//! Klartext uebersprungen (naechster Versuch in 15 min). Der Abruf ist auf 20 MB
//! begrenzt, das Parsen erzeugt hoechstens 2 Mio. Instanzen (`ics.rs`), gespeichert
//! werden hoechstens 20 000 Termine je Quelle. Der Dienst startet keinen Prozess.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::fetch::{self, FetchOutcome};
use super::ics::{self, IcsResult};
use super::model::{CalEvent, CalendarError, CalendarKind, CalendarSource};
use super::reminder::{self, ReminderCtx};
use super::secret;
use crate::managers::meetings::recorder::MeetingRecorderManager;
use crate::managers::meetings::store::{MeetingStore, SyncMark, SyncMeta};

/// Abstand der Abrufe.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Erster Abruf nach dem Start (die Oberflaeche soll zuerst kommen).
const FIRST_SYNC_DELAY: Duration = Duration::from_secs(10);
/// Takt der Erinnerung.
pub const TICK_INTERVAL: Duration = Duration::from_secs(15);
/// Fenster der gespeicherten Termine um „jetzt“.
const WINDOW_DAYS: i64 = 30;
const DAY_MS: i64 = 86_400_000;
/// Der Parser rekursiert bei verschachtelten Komponenten; eigener, grosszuegiger Stapel.
const PARSE_STACK_BYTES: usize = 16 * 1024 * 1024;
/// Freier Arbeitsspeicher, den ein Abruf samt Parsen hoechstens braucht (MB).
const SYNC_NEED_MB: u64 = 400;
/// Titel, wenn weder der Nutzer noch ein Termin einen liefert.
pub const DEFAULT_TITLE: &str = "Besprechung";

/// Ergebnis eines Abrufs fuer die Oberflaeche (`CalendarSyncEvent`).
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct CalendarSyncEvent {
    pub source_id: String,
    pub ok: bool,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncReport {
    /// Termine der Quelle im Cache nach dem Abruf.
    pub count: u32,
    /// Uebersprungene Komponenten o. ae. (nur zur Anzeige im Log).
    pub warnings: usize,
}

/// Fenster `[jetzt - 30 Tage, jetzt + 30 Tage]` in ms.
pub fn window_for(now_ms: i64) -> (i64, i64) {
    (now_ms - WINDOW_DAYS * DAY_MS, now_ms + WINDOW_DAYS * DAY_MS)
}

/// Fuehrt `f` in einem eigenen Thread aus und faengt eine Panik ab. `None`,
/// wenn der Thread nicht startete oder abstuerzte.
pub fn run_guarded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let handle = std::thread::Builder::new()
        .name("calendar-parse".to_string())
        .stack_size(PARSE_STACK_BYTES)
        .spawn(move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)))
        .ok()?;
    match handle.join() {
        Ok(Ok(value)) => Some(value),
        _ => None,
    }
}

/// `ics::parse_and_expand` in einem eigenen Thread mit `catch_unwind`.
pub fn parse_guarded(
    raw: String,
    source_id: String,
    from_ms: i64,
    to_ms: i64,
    tz: String,
) -> Result<IcsResult, CalendarError> {
    run_guarded(move || ics::parse_and_expand(&raw, &source_id, from_ms, to_ms, &tz))
        .unwrap_or_else(|| {
            Err(CalendarError::Parse(
                "interner Fehler beim Lesen des Kalenders".to_string(),
            ))
        })
}

/// Standardzone fuer schwebende Zeiten: die Windows-Zone, sonst UTC.
pub fn default_tz() -> String {
    ics::system_tz_name().unwrap_or_else(|| "UTC".to_string())
}

/// Schreibt ein Parse-Ergebnis in EINER Transaktion in den Cache der Quelle.
pub fn apply_parsed(
    store: &MeetingStore,
    source_id: &str,
    parsed: &IcsResult,
    etag: Option<&str>,
    last_modified: Option<&str>,
    now_ms: i64,
) -> Result<SyncReport, String> {
    let meta = SyncMeta {
        has_attendee_data: parsed.has_attendee_data,
        etag,
        last_modified,
        now_ms,
    };
    store
        .calendar_replace_events(source_id, &parsed.events, &meta)
        .map_err(|e| format!("Die Termine konnten nicht gespeichert werden: {e}"))?;
    Ok(SyncReport {
        count: parsed.events.len() as u32,
        warnings: parsed.warnings.len(),
    })
}

/// Parsen und Speichern in einem Schritt (blockierend).
pub fn apply_body(
    store: &MeetingStore,
    source_id: &str,
    text: String,
    etag: Option<&str>,
    last_modified: Option<&str>,
    now_ms: i64,
    tz: &str,
) -> Result<SyncReport, String> {
    let (from, to) = window_for(now_ms);
    let parsed = parse_guarded(text, source_id.to_string(), from, to, tz.to_string())
        .map_err(|e| e.to_string())?;
    apply_parsed(store, source_id, &parsed, etag, last_modified, now_ms)
}

// ---------------------------------------------------------------------------
// Erinnerung
// ---------------------------------------------------------------------------

/// Die Termine, zu denen jetzt erinnert werden soll (bereits nach Beginn
/// sortiert). Liest den Cache; schreibt nichts.
pub fn pending_reminders(
    store: &MeetingStore,
    now_ms: i64,
    lead_ms: i64,
    recording: bool,
    all_events: bool,
) -> Result<Vec<CalEvent>> {
    if lead_ms <= 0 || recording {
        return Ok(Vec::new());
    }
    let from = now_ms - reminder::CATCH_UP_MS - 1_000;
    let to = now_ms + lead_ms + 1_000;
    let events = store.calendar_events_between(from, to, false)?;
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let with_data: std::collections::HashSet<String> = store
        .calendar_sources()?
        .into_iter()
        .filter(|s| s.has_attendee_data)
        .map(|s| s.id)
        .collect();
    let attendee_data = |source_id: &str| with_data.contains(source_id);
    let handled = |key: &str| {
        matches!(
            store.calendar_reminder_state(key),
            Ok(Some((reminded, dismissed))) if reminded.is_some() || dismissed.is_some()
        )
    };
    let ctx = ReminderCtx {
        now_ms,
        lead_ms,
        recording,
        all_events,
        attendee_data: &attendee_data,
        handled: &handled,
    };
    Ok(reminder::due_reminders(&events, &ctx)
        .into_iter()
        .cloned()
        .collect())
}

/// Der Termin, dem eine jetzt beginnende Aufnahme gehoert (Beginn +-15 min,
/// genau einer), sonst `None`.
pub fn suggest_event(store: &MeetingStore, now_ms: i64) -> Result<Option<CalEvent>> {
    let window = reminder::START_WINDOW_MS;
    let events = store.calendar_events_between(now_ms - window, now_ms + window + 1, false)?;
    Ok(reminder::event_for_start(&events, now_ms, window).cloned())
}

// ---------------------------------------------------------------------------
// Start aus Termin
// ---------------------------------------------------------------------------

/// Was `meetings_start_from_event` vor dem Start festlegt.
#[derive(Clone, Debug, PartialEq)]
pub struct StartPlan {
    pub title: String,
    pub event: Option<CalEvent>,
    /// Vorlage der letzten Besprechung derselben Serie (UID); `None` = Standard.
    pub template_id: Option<String>,
}

/// Titel: was der Nutzer eingegeben hat, sonst der Termintitel, sonst
/// `DEFAULT_TITLE`. Unbekannter Termin = Fehler (kein Start mit falschem Bezug).
pub fn plan_start(
    store: &MeetingStore,
    event_key: Option<&str>,
    title: Option<&str>,
) -> Result<StartPlan, String> {
    let event = match event_key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => Some(
            store
                .calendar_event(key)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "calendar_event_not_found".to_string())?,
        ),
        None => None,
    };
    let typed = title.map(str::trim).filter(|t| !t.is_empty());
    let title = typed
        .map(str::to_string)
        .or_else(|| event.as_ref().map(|e| e.title.clone()))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TITLE.to_string());
    let template_id = match &event {
        Some(e) => series_template(store, &e.uid).map_err(|e| e.to_string())?,
        None => None,
    };
    Ok(StartPlan {
        title,
        event,
        template_id,
    })
}

/// Vorlage der juengsten lebenden Besprechung derselben Serie, die eine hat.
fn series_template(store: &MeetingStore, uid: &str) -> Result<Option<String>> {
    for meeting_id in store.meetings_by_event_uid(uid)? {
        if let Some(t) = store.meeting_template_id(&meeting_id)? {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

/// Nach `recorder.start`: Verknuepfung, Vorlage und Teilnehmer-Schnappschuss.
/// Fehler hier duerfen die laufende Aufnahme nie kippen; sie werden nur
/// gemeldet. `linked_by`: `prompt` oder `auto`.
pub fn finish_start(
    store: &MeetingStore,
    meeting_id: &str,
    plan: &StartPlan,
    linked_by: &str,
    now_ms: i64,
) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(event) = &plan.event {
        if let Err(e) = store.link_meeting_event(meeting_id, event, linked_by, now_ms) {
            problems.push(format!("Verknüpfung mit dem Termin: {e}"));
        }
        let attendees: Vec<serde_json::Value> = event
            .attendees
            .iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name,
                    "email": a.email,
                    "organizer": a.organizer,
                })
            })
            .collect();
        let snapshot = serde_json::json!({
            "event_key": event.key,
            "uid": event.uid,
            "title": event.title,
            "starts_at": event.starts_at,
            "attendees": attendees,
        });
        if let Err(e) = store.set_metadata_key(meeting_id, "calendar", snapshot) {
            problems.push(format!("Teilnehmende des Termins: {e}"));
        }
        // M5-P5d: die Teilnehmenden werden Personen und Teilnehmende der Besprechung.
        if let Err(e) = crate::managers::people::participants_from_event(store, meeting_id, event) {
            problems.push(format!("Personen des Termins: {e}"));
        }
    }
    if let Some(template) = &plan.template_id {
        if let Err(e) = store.set_meeting_template(meeting_id, Some(template)) {
            problems.push(format!("Vorlage der Serie: {e}"));
        }
    }
    problems
}

// ---------------------------------------------------------------------------
// Dienst
// ---------------------------------------------------------------------------

pub struct CalendarService {
    app: AppHandle,
    store: Arc<MeetingStore>,
    /// Ein Abruf zur Zeit (Zeitgeber, „Jetzt aktualisieren“, Hinzufuegen).
    gate: tauri::async_runtime::Mutex<()>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl CalendarService {
    /// Legt den Geheimnisordner fest und startet Abruf- und Erinnerungsschleife.
    pub fn spawn(app: AppHandle, store: Arc<MeetingStore>) -> Arc<Self> {
        match secret::secrets_dir_for(&app) {
            Ok(dir) => secret::init_dir(dir),
            Err(e) => log::error!("calendar: secrets directory unavailable: {e}"),
        }
        let service = Arc::new(Self {
            app,
            store,
            gate: tauri::async_runtime::Mutex::new(()),
        });

        let syncer = service.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_SYNC_DELAY).await;
            loop {
                syncer.sync_now(None).await;
                tokio::time::sleep(SYNC_INTERVAL).await;
            }
        });

        let reminder = service.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(TICK_INTERVAL).await;
                let tick = reminder.clone();
                // SQLite und Fensteraufbau: nicht auf dem Async-Arbeiter.
                let _ =
                    tauri::async_runtime::spawn_blocking(move || tick.remind_tick(now_ms())).await;
            }
        });
        service
    }

    /// Ein Zeitgebertakt der Erinnerung; zeigt hoechstens EINEN Hinweis.
    pub fn remind_tick(&self, now_ms: i64) {
        let settings = crate::settings::get_settings(&self.app);
        let lead_ms = i64::from(settings.meeting_reminder_lead_s) * 1_000;
        if lead_ms <= 0 || crate::meeting_prompt::is_open(&self.app) {
            return;
        }
        let recording = self
            .app
            .try_state::<Arc<MeetingRecorderManager>>()
            .is_some_and(|r| r.is_recording());
        let due = match pending_reminders(
            &self.store,
            now_ms,
            lead_ms,
            recording,
            settings.meeting_reminder_all_events,
        ) {
            Ok(due) => due,
            Err(e) => {
                log::warn!("calendar: reminder check failed: {e}");
                return;
            }
        };
        let Some(event) = due.into_iter().next() else {
            return;
        };
        if let Err(e) = self.store.calendar_mark_reminded(&event.key, now_ms) {
            log::warn!("calendar: could not mark reminder: {e}");
            return; // ohne Merker wuerde der Hinweis alle 15 s wiederkommen
        }
        crate::meeting_prompt::show_event(&self.app, event);
    }

    /// Ruft eine oder alle aktiven Quellen ab und liefert den Stand danach.
    pub async fn sync_now(&self, only: Option<String>) -> Vec<CalendarSource> {
        let _gate = self.gate.lock().await;
        let sources = match self.store.calendar_sources() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("calendar: sources unreadable: {e}");
                return Vec::new();
            }
        };
        for source in sources.iter().filter(|s| {
            s.enabled && s.kind == CalendarKind::Ics && only.as_deref().is_none_or(|id| id == s.id)
        }) {
            let outcome = self.sync_source(source).await;
            let now = now_ms();
            let (ok, count) = match outcome {
                Ok(report) => (true, report.count),
                Err(msg) => {
                    log::warn!("calendar: sync of {} failed: {msg}", source.id);
                    if let Err(e) = self.store.calendar_source_mark_sync(
                        &source.id,
                        now,
                        SyncMark::Failed(&msg),
                    ) {
                        log::warn!("calendar: could not record the failure: {e}");
                    }
                    (false, source.event_count)
                }
            };
            let _ = CalendarSyncEvent {
                source_id: source.id.clone(),
                ok,
                count,
            }
            .emit(&self.app);
        }
        self.store.calendar_sources().unwrap_or_default()
    }

    /// Ein Abruf: Geheimnis lesen, Adresse abrufen, parsen, speichern. Bei
    /// Erfolg hat der Store `last_ok_at` gesetzt; Fehler meldet der Aufrufer.
    async fn sync_source(&self, source: &CalendarSource) -> Result<SyncReport, String> {
        crate::process_guard::check_ram_for_start(SYNC_NEED_MB)?;
        let url = read_url(&source.id)?;
        let validators = self
            .store
            .calendar_source_validators(&source.id)
            .map_err(|e| e.to_string())?
            .unwrap_or((None, None));
        let outcome = fetch::fetch_ics(&url, validators.0.as_deref(), validators.1.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        drop(url);
        let now = now_ms();
        match outcome {
            FetchOutcome::NotModified => {
                self.store
                    .calendar_source_mark_sync(&source.id, now, SyncMark::NotModified)
                    .map_err(|e| e.to_string())?;
                Ok(SyncReport {
                    count: source.event_count,
                    warnings: 0,
                })
            }
            FetchOutcome::Body {
                text,
                etag,
                last_modified,
            } => {
                let store = self.store.clone();
                let id = source.id.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    apply_body(
                        &store,
                        &id,
                        text,
                        etag.as_deref(),
                        last_modified.as_deref(),
                        now,
                        &default_tz(),
                    )
                })
                .await
                .map_err(|e| format!("Lesevorgang abgebrochen: {e}"))?
            }
        }
    }

    /// Neue ICS-Quelle: Probeabruf und Lesetest VOR dem Speichern. Erst wenn die
    /// Adresse einen lesbaren Kalender liefert, entstehen Geheimnis und Quelle.
    pub async fn add_ics_source(&self, label: &str, url: &str) -> Result<CalendarSource, String> {
        let _gate = self.gate.lock().await;
        crate::process_guard::check_ram_for_start(SYNC_NEED_MB)?;
        let url = url.trim().to_string();
        let normalized = fetch::normalize_url(&url).map_err(|e| e.to_string())?;
        let hint = fetch::host_hint(&normalized);
        let label = label.trim();
        let label = if label.is_empty() {
            hint.clone().unwrap_or_else(|| "Kalender".to_string())
        } else {
            label.to_string()
        };

        let outcome = fetch::fetch_ics(&normalized, None, None)
            .await
            .map_err(|e| e.to_string())?;
        let FetchOutcome::Body {
            text,
            etag,
            last_modified,
        } = outcome
        else {
            return Err(CalendarError::NotCalendar.to_string());
        };
        let now = now_ms();
        let (from, to) = window_for(now);
        let tz = default_tz();
        let parsed = tauri::async_runtime::spawn_blocking(move || {
            parse_guarded(text, "probe".to_string(), from, to, tz)
        })
        .await
        .map_err(|e| format!("Lesevorgang abgebrochen: {e}"))?
        .map_err(|e| e.to_string())?;

        let id = new_source_id();
        secret::secret_put(&id, normalized.as_bytes())?;
        let created =
            self.store
                .calendar_source_add(&id, CalendarKind::Ics, &label, hint.as_deref(), now);
        if let Err(e) = created {
            secret::secret_delete(&id);
            return Err(format!("Die Quelle konnte nicht gespeichert werden: {e}"));
        }
        // Die Termine tragen die Quelle im Schluessel: fuer die echte ID neu
        // aufloesen statt die der Probe umzuschreiben.
        let store = self.store.clone();
        let stored_id = id.clone();
        let report = tauri::async_runtime::spawn_blocking(move || {
            rekey_and_store(&store, &stored_id, parsed, etag, last_modified, now)
        })
        .await
        .map_err(|e| format!("Speichern abgebrochen: {e}"))?;
        if let Err(msg) = report {
            let _ = self.store.calendar_source_remove(&id, now_ms());
            secret::secret_delete(&id);
            return Err(msg);
        }
        let source = self
            .store
            .calendar_source(&id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Die Quelle ist nicht mehr da.".to_string())?;
        let _ = CalendarSyncEvent {
            source_id: id,
            ok: true,
            count: source.event_count,
        }
        .emit(&self.app);
        Ok(source)
    }

    /// Entfernt Quelle, Cache und Geheimnis.
    pub fn remove_source(&self, id: &str) -> Result<(), String> {
        self.store
            .calendar_source_remove(id, now_ms())
            .map_err(|e| e.to_string())?;
        secret::secret_delete(id);
        Ok(())
    }
}

/// Die Adresse einer Quelle aus dem Geheimnisspeicher; jeder Fehler heisst
/// fuer den Nutzer „Adresse neu eingeben“ und nennt nie die Adresse.
fn read_url(source_id: &str) -> Result<zeroize::Zeroizing<String>, String> {
    let bytes = match secret::secret_get(source_id) {
        Ok(Some(b)) => b,
        Ok(None) | Err(_) => return Err(CalendarError::Auth.to_string()),
    };
    String::from_utf8(bytes.to_vec())
        .map(zeroize::Zeroizing::new)
        .map_err(|_| CalendarError::Auth.to_string())
}

/// Die Probe hat die Termine unter der Kennung `probe` geparst; fuer die echte
/// Quelle bekommen sie deren ID im Schluessel.
fn rekey_and_store(
    store: &MeetingStore,
    source_id: &str,
    mut parsed: IcsResult,
    etag: Option<String>,
    last_modified: Option<String>,
    now_ms: i64,
) -> Result<SyncReport, String> {
    for e in &mut parsed.events {
        e.source_id = source_id.to_string();
        e.key = super::model::event_key(source_id, &e.uid, e.starts_at);
    }
    apply_parsed(
        store,
        source_id,
        &parsed,
        etag.as_deref(),
        last_modified.as_deref(),
        now_ms,
    )
}

/// `ics-` plus 16 Hexziffern; zugleich Dateiname des Geheimnisses.
fn new_source_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("ics-{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::Attendee;
    use crate::managers::meetings::store::MeetingSource;
    use chrono::TimeZone;

    fn store() -> MeetingStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let s = MeetingStore::open_at(&path).unwrap();
        std::mem::forget(dir);
        s
    }

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/calendar")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    const MIN: i64 = 60_000;
    const T0: i64 = 1_790_589_600_000; // 29.09.2026 10:00 UTC

    fn add_source(s: &MeetingStore, id: &str) {
        s.calendar_source_add(id, CalendarKind::Ics, "Test", Some("example.org"), 1)
            .unwrap();
    }

    fn cal_event(source: &str, uid: &str, start: i64, attendees: usize) -> CalEvent {
        CalEvent {
            key: super::super::model::event_key(source, uid, start),
            source_id: source.to_string(),
            uid: uid.to_string(),
            title: format!("Termin {uid}"),
            starts_at: start,
            ends_at: start + 30 * MIN,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees: (0..attendees)
                .map(|i| Attendee {
                    email: Some(format!("p{i}@example.org")),
                    name: None,
                    organizer: i == 0,
                    is_self: false,
                    partstat: None,
                })
                .collect(),
        }
    }

    fn put_events(s: &MeetingStore, source: &str, events: &[CalEvent], has_attendees: bool) {
        s.calendar_replace_events(
            source,
            events,
            &SyncMeta {
                has_attendee_data: has_attendees,
                etag: None,
                last_modified: None,
                now_ms: 1,
            },
        )
        .unwrap();
    }

    // ---- Parser-Wache und Abruf-Ergebnis -----------------------------------

    #[test]
    fn run_guarded_turns_a_panic_into_an_error() {
        let ok = run_guarded(|| 7);
        assert_eq!(ok, Some(7));
        let boom: Option<i32> = run_guarded(|| panic!("Bibliothek abgestuerzt"));
        assert_eq!(
            boom, None,
            "die Panik bleibt im Thread, die App laeuft weiter"
        );
    }

    #[test]
    fn parse_guarded_reads_the_outlook_fixture() {
        let raw = fixture("outlook_series.ics");
        let from = ms(2026, 9, 1, 0, 0);
        let to = ms(2026, 12, 1, 0, 0);
        let parsed = parse_guarded(
            raw,
            "src".into(),
            from,
            to,
            "W. Europe Standard Time".into(),
        )
        .expect("Fixture ist lesbar");
        assert!(!parsed.events.is_empty());
        assert!(parsed.has_attendee_data);
    }

    #[test]
    fn parse_guarded_rejects_a_login_page() {
        let err = parse_guarded(
            "<html><body>Anmelden</body></html>".into(),
            "src".into(),
            0,
            1,
            "UTC".into(),
        )
        .unwrap_err();
        assert_eq!(err, CalendarError::NotCalendar);
    }

    #[test]
    fn apply_body_fills_the_cache_and_the_source_counts() {
        let s = store();
        add_source(&s, "src");
        let now = ms(2026, 9, 29, 8, 0);
        let report = apply_body(
            &s,
            "src",
            fixture("outlook_series.ics"),
            Some("\"v1\""),
            None,
            now,
            "W. Europe Standard Time",
        )
        .unwrap();
        assert!(report.count > 0);
        let source = s.calendar_source("src").unwrap().unwrap();
        assert_eq!(source.event_count, report.count);
        assert!(source.has_attendee_data);
        assert_eq!(source.last_ok_at, Some(now));
        assert_eq!(source.last_error, None);
        let (etag, _) = s.calendar_source_validators("src").unwrap().unwrap();
        assert_eq!(etag.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn failed_parse_keeps_the_cache() {
        let s = store();
        add_source(&s, "src");
        let now = ms(2026, 9, 29, 8, 0);
        apply_body(
            &s,
            "src",
            fixture("outlook_series.ics"),
            None,
            None,
            now,
            "UTC",
        )
        .unwrap();
        let before = s.calendar_source("src").unwrap().unwrap().event_count;
        assert!(before > 0);
        let err = apply_body(
            &s,
            "src",
            "<html>Login</html>".into(),
            None,
            None,
            now + 1,
            "UTC",
        )
        .unwrap_err();
        assert!(err.contains("keinen Kalender"), "Klartext: {err}");
        assert_eq!(
            s.calendar_source("src").unwrap().unwrap().event_count,
            before,
            "der alte Cache bleibt"
        );
    }

    // ---- Erinnerung ---------------------------------------------------------

    #[test]
    fn pending_reminders_returns_the_event_until_it_is_marked() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 3);
        put_events(&s, "src", &[ev.clone()], true);
        let due = pending_reminders(&s, T0 - MIN, MIN, false, false).unwrap();
        assert_eq!(
            due.iter().map(|e| e.key.clone()).collect::<Vec<_>>(),
            vec![ev.key.clone()]
        );
        s.calendar_mark_reminded(&ev.key, T0 - MIN).unwrap();
        assert!(pending_reminders(&s, T0 - MIN + 15_000, MIN, false, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pending_reminders_skips_dismissed_events_and_running_recordings() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 3);
        put_events(&s, "src", &[ev.clone()], true);
        assert!(pending_reminders(&s, T0 - MIN, MIN, true, false)
            .unwrap()
            .is_empty());
        s.calendar_mark_dismissed(&ev.key, T0 - 90_000).unwrap();
        assert!(pending_reminders(&s, T0 - MIN, MIN, false, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn pending_reminders_needs_two_attendees_only_for_sources_with_attendee_data() {
        let s = store();
        add_source(&s, "mit");
        add_source(&s, "ohne");
        put_events(&s, "mit", &[cal_event("mit", "a", T0, 1)], true);
        put_events(&s, "ohne", &[cal_event("ohne", "b", T0, 0)], false);
        let due = pending_reminders(&s, T0 - MIN, MIN, false, false).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].source_id, "ohne",
            "die Quelle ohne Teilnehmerdaten erinnert"
        );
        let all = pending_reminders(&s, T0 - MIN, MIN, false, true).unwrap();
        assert_eq!(all.len(), 2, "„alle Termine“ hebt die Regel auf");
    }

    #[test]
    fn pending_reminders_ignores_disabled_sources() {
        let s = store();
        add_source(&s, "src");
        put_events(&s, "src", &[cal_event("src", "a", T0, 3)], true);
        s.calendar_source_set_enabled("src", false, 2).unwrap();
        assert!(pending_reminders(&s, T0 - MIN, MIN, false, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn suggest_event_finds_exactly_one_event_around_now() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 2);
        put_events(&s, "src", &[ev.clone()], true);
        assert_eq!(
            suggest_event(&s, T0 - 10 * MIN).unwrap().map(|e| e.key),
            Some(ev.key.clone())
        );
        assert_eq!(
            suggest_event(&s, T0 + 10 * MIN).unwrap().map(|e| e.key),
            Some(ev.key)
        );
        assert!(suggest_event(&s, T0 + 40 * MIN).unwrap().is_none());
        put_events(
            &s,
            "src",
            &[
                cal_event("src", "a", T0, 2),
                cal_event("src", "b", T0 + 5 * MIN, 2),
            ],
            true,
        );
        assert!(
            suggest_event(&s, T0).unwrap().is_none(),
            "zwei Termine: nicht raten"
        );
    }

    // ---- Start aus Termin ---------------------------------------------------

    #[test]
    fn plan_start_takes_the_event_title_only_without_a_typed_title() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 2);
        put_events(&s, "src", &[ev.clone()], true);
        let plan = plan_start(&s, Some(&ev.key), None).unwrap();
        assert_eq!(plan.title, "Termin a");
        assert_eq!(
            plan.event.as_ref().map(|e| e.key.clone()),
            Some(ev.key.clone())
        );
        let typed = plan_start(&s, Some(&ev.key), Some("  Mein Titel ")).unwrap();
        assert_eq!(typed.title, "Mein Titel");
        let blank = plan_start(&s, Some(&ev.key), Some("   ")).unwrap();
        assert_eq!(
            blank.title, "Termin a",
            "nur Leerraum zaehlt nicht als Eingabe"
        );
    }

    #[test]
    fn plan_start_without_event_uses_typed_or_default_title() {
        let s = store();
        assert_eq!(
            plan_start(&s, None, Some("Ad hoc")).unwrap().title,
            "Ad hoc"
        );
        let plan = plan_start(&s, None, None).unwrap();
        assert_eq!(plan.title, DEFAULT_TITLE);
        assert!(plan.event.is_none() && plan.template_id.is_none());
    }

    #[test]
    fn plan_start_rejects_an_unknown_event() {
        let s = store();
        assert_eq!(
            plan_start(&s, Some("src:weg:1"), None).unwrap_err(),
            "calendar_event_not_found"
        );
    }

    #[test]
    fn plan_start_inherits_the_template_of_the_last_meeting_in_the_series() {
        let s = store();
        add_source(&s, "src");
        let old = cal_event("src", "serie", T0 - 7 * 24 * 60 * MIN, 2);
        let next = cal_event("src", "serie", T0, 2);
        put_events(&s, "src", &[old.clone(), next.clone()], true);
        let m = s
            .create_meeting("Jour fixe", MeetingSource::Live, Some(1))
            .unwrap();
        s.link_meeting_event(&m.id, &old, "prompt", 5).unwrap();
        s.set_meeting_template(&m.id, Some("builtin:vertrieb"))
            .unwrap();
        let plan = plan_start(&s, Some(&next.key), None).unwrap();
        assert_eq!(plan.template_id.as_deref(), Some("builtin:vertrieb"));
        let other = cal_event("src", "andere", T0, 2);
        put_events(&s, "src", &[old, next, other.clone()], true);
        assert_eq!(
            plan_start(&s, Some(&other.key), None).unwrap().template_id,
            None
        );
    }

    #[test]
    fn finish_start_links_the_event_sets_the_template_and_keeps_attendees() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 2);
        put_events(&s, "src", &[ev.clone()], true);
        let m = s
            .create_meeting("Termin a", MeetingSource::Live, Some(1))
            .unwrap();
        let plan = StartPlan {
            title: "Termin a".into(),
            event: Some(ev.clone()),
            template_id: Some("builtin:vertrieb".into()),
        };
        let problems = finish_start(&s, &m.id, &plan, "prompt", 42);
        assert!(problems.is_empty(), "{problems:?}");
        let link = s.meeting_calendar_link(&m.id).unwrap().unwrap();
        assert_eq!(link.uid, "a");
        assert_eq!(link.linked_by, "prompt");
        assert_eq!(
            s.meeting_template_id(&m.id).unwrap().as_deref(),
            Some("builtin:vertrieb")
        );
        let meta = s.metadata_json(&m.id).unwrap().unwrap();
        assert_eq!(
            meta["calendar"]["attendees"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(meta["calendar"]["event_key"], ev.key);
    }

    #[test]
    fn finish_start_stores_the_attendees_as_people_of_the_meeting() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 3);
        put_events(&s, "src", &[ev.clone()], true);
        let m = s
            .create_meeting("Termin a", MeetingSource::Live, Some(1))
            .unwrap();
        let plan = StartPlan {
            title: "Termin a".into(),
            event: Some(ev),
            template_id: None,
        };
        assert!(finish_start(&s, &m.id, &plan, "prompt", 42).is_empty());
        let people = s.participants_of(&m.id, &[]).unwrap();
        assert_eq!(people.len(), 3);
        assert_eq!(people[0].role, "organizer", "der Organisator steht vorn");
        assert_eq!(people[0].email.as_deref(), Some("p0@example.org"));
        assert!(people.iter().all(|p| p.source == "calendar"));
        // Ein zweiter Start (gleiche Personen) legt niemanden doppelt an.
        let again = s
            .create_meeting("Termin a, wieder", MeetingSource::Live, Some(1))
            .unwrap();
        assert!(finish_start(&s, &again.id, &plan, "prompt", 43).is_empty());
        assert_eq!(s.list_people(None, &[]).unwrap().len(), 3);
    }

    #[test]
    fn finish_start_reports_problems_instead_of_failing() {
        let s = store();
        add_source(&s, "src");
        let ev = cal_event("src", "a", T0, 2);
        put_events(&s, "src", &[ev.clone()], true);
        let plan = StartPlan {
            title: "x".into(),
            event: Some(ev),
            template_id: None,
        };
        let problems = finish_start(&s, "gibt-es-nicht", &plan, "auto", 1);
        assert!(!problems.is_empty(), "die Meldung ersetzt einen Abbruch");
    }

    #[test]
    fn source_ids_are_valid_secret_names_and_unique() {
        let a = new_source_id();
        let b = new_source_id();
        assert_ne!(a, b);
        assert!(a.starts_with("ics-") && a.len() == 20);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'));
    }

    #[test]
    fn window_spans_thirty_days_each_way() {
        let (from, to) = window_for(T0);
        assert_eq!(T0 - from, 30 * DAY_MS);
        assert_eq!(to - T0, 30 * DAY_MS);
    }
}
