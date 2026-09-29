//! Duenne Befehlshuelle ueber `CalendarService` und `MeetingStore` (M5, P5b;
//! Muster `commands/meetings.rs`). Die Logik steht in `managers/calendar`.

use std::sync::Arc;

use tauri::State;

use crate::managers::calendar::model::{CalEvent, CalendarSource};
use crate::managers::calendar::service::{self, CalendarService};
use crate::managers::meetings::store::MeetingStore;
use crate::settings;

const HOUR_MS: i64 = 3_600_000;
/// Laenger als eine Woche im Voraus zeigt keine Liste „Naechste Termine“.
const MAX_UPCOMING_HOURS: u32 = 24 * 7;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[tauri::command]
#[specta::specta]
pub async fn calendar_sources_list(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Vec<CalendarSource>, String> {
    store.calendar_sources().map_err(|e| e.to_string())
}

/// Verbindet eine ICS-Adresse. Der Probeabruf laeuft VOR dem Speichern: eine
/// Adresse, die keinen lesbaren Kalender liefert, erzeugt weder Quelle noch
/// Geheimnis, und der Fehler nennt den Grund (nie die Adresse).
#[tauri::command]
#[specta::specta]
pub async fn calendar_source_add_ics(
    service: State<'_, Arc<CalendarService>>,
    label: String,
    url: String,
) -> Result<CalendarSource, String> {
    let service = Arc::clone(&service);
    service.add_ics_source(&label, &url).await
}

/// Entfernt die Quelle samt Terminen im Cache und ihrem Geheimnis.
#[tauri::command]
#[specta::specta]
pub async fn calendar_source_remove(
    service: State<'_, Arc<CalendarService>>,
    id: String,
) -> Result<(), String> {
    service.remove_source(&id)
}

/// „Jetzt aktualisieren“: ruft eine (`id`) oder alle Quellen ab und liefert den
/// Stand danach. Fehler je Quelle stehen in `last_error` der Quelle.
#[tauri::command]
#[specta::specta]
pub async fn calendar_sync_now(
    service: State<'_, Arc<CalendarService>>,
    id: Option<String>,
) -> Result<Vec<CalendarSource>, String> {
    let service = Arc::clone(&service);
    Ok(service.sync_now(id).await)
}

/// Termine der naechsten `hours` Stunden (laufende eingeschlossen), abgesagte nie.
#[tauri::command]
#[specta::specta]
pub async fn calendar_upcoming(
    store: State<'_, Arc<MeetingStore>>,
    hours: u32,
) -> Result<Vec<CalEvent>, String> {
    let now = now_ms();
    let to = now + i64::from(hours.clamp(1, MAX_UPCOMING_HOURS)) * HOUR_MS;
    store
        .calendar_events_between(now, to, false)
        .map_err(|e| e.to_string())
}

/// Der Termin, dem eine jetzt beginnende Aufnahme gehoert (Beginn +-15 min,
/// genau einer); Grundlage des Titelvorschlags.
#[tauri::command]
#[specta::specta]
pub async fn calendar_suggest_event(
    store: State<'_, Arc<MeetingStore>>,
) -> Result<Option<CalEvent>, String> {
    service::suggest_event(&store, now_ms()).map_err(|e| e.to_string())
}

/// Oeffnet die Beitritts-Adresse eines Termins im Browser. Die Adresse kommt aus
/// dem Cache (nicht vom Fenster) und muss mit `https://` beginnen: eine
/// Kalenderdatei ist fremde Eingabe und darf kein anderes Schema oeffnen.
#[tauri::command]
#[specta::specta]
pub async fn calendar_open_join_url(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    event_key: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let event = store
        .calendar_event(&event_key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "calendar_event_not_found".to_string())?;
    let url = join_url_of(&event).ok_or_else(|| "calendar_no_join_url".to_string())?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Die Beitritts-Adresse eines Termins, nur wenn sie ein `https://`-Link ist.
pub fn join_url_of(event: &CalEvent) -> Option<String> {
    let url = event.join_url.as_deref()?.trim();
    let is_https = url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    (is_https && url.len() > 8).then(|| url.to_string())
}

/// Vorlauf der Erinnerung in Sekunden; 0 schaltet sie aus.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_reminder_lead_setting(
    app: tauri::AppHandle,
    seconds: u32,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_reminder_lead_s = seconds.min(3_600);
    settings::write_settings(&app, settings);
    Ok(())
}

/// „Auch Termine ohne Teilnehmende erinnern.“
#[tauri::command]
#[specta::specta]
pub fn change_meeting_reminder_all_events_setting(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_reminder_all_events = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(url: Option<&str>) -> CalEvent {
        CalEvent {
            key: "s:u:1".into(),
            source_id: "s".into(),
            uid: "u".into(),
            title: "T".into(),
            starts_at: 1,
            ends_at: 2,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: url.map(str::to_string),
            description: None,
            attendees: vec![],
        }
    }

    #[test]
    fn join_url_must_be_https() {
        assert_eq!(
            join_url_of(&event(Some(
                " https://teams.microsoft.com/l/meetup-join/x "
            )))
            .as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/x")
        );
        assert_eq!(
            join_url_of(&event(Some("HTTPS://meet.google.com/abc"))).as_deref(),
            Some("HTTPS://meet.google.com/abc")
        );
        for bad in [
            "http://x.example/a",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "ms-teams://x",
            "https://",
            "",
        ] {
            assert_eq!(join_url_of(&event(Some(bad))), None, "{bad}");
        }
        assert_eq!(join_url_of(&event(None)), None);
    }
}
