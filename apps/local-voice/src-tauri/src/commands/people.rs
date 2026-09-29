//! Duenne Befehlshuelle ueber die Personen (M5, P5d) und den Pre-Meeting-Brief
//! (P5e). Die Logik steht in `managers/people`; hier stehen nur Argumente, die
//! Einstellung „Meine E-Mail-Adressen“ und der Weg vom Hinweisfenster in das
//! Hauptfenster.

use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::managers::meetings::search::index::ScopeFilter;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::people::{self, Participant, PersonDetail, PersonSummary};
use crate::settings;

/// ID des eingebauten Recipes fuer den Brief (`chat/recipes.rs`).
pub const BRIEF_RECIPE_ID: &str = "builtin:vorbereitung-termin";
/// Name der Recipe-Variable mit den Teilnehmenden.
pub const BRIEF_RECIPE_VAR: &str = "teilnehmende";

fn store_err(e: anyhow::Error) -> String {
    e.to_string()
}

fn self_emails(app: &AppHandle) -> Vec<String> {
    settings::get_settings(app).meeting_self_emails
}

/// Alle Personen, die meisten Besprechungen zuerst; `query` filtert nach Name,
/// Adresse oder Firma.
#[tauri::command]
#[specta::specta]
pub async fn people_list(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    query: Option<String>,
) -> Result<Vec<PersonSummary>, String> {
    let own = self_emails(&app);
    store.list_people(query.as_deref(), &own).map_err(store_err)
}

/// Eine Person mit den juengsten Besprechungen (`person_not_found`, wenn es sie
/// nicht gibt).
#[tauri::command]
#[specta::specta]
pub async fn people_get(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<PersonDetail, String> {
    let own = self_emails(&app);
    store
        .get_person(&id, &own)
        .map_err(store_err)?
        .ok_or_else(|| "person_not_found".to_string())
}

/// Fuehrt `gone` in `keep` zusammen.
#[tauri::command]
#[specta::specta]
pub async fn people_merge(
    store: State<'_, Arc<MeetingStore>>,
    keep: String,
    gone: String,
) -> Result<(), String> {
    store.merge_people(&keep, &gone).map_err(store_err)
}

/// Benennt eine Person um; `email`: `None` = unveraendert, leer = entfernen.
#[tauri::command]
#[specta::specta]
pub async fn people_update(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    name: String,
    email: Option<String>,
) -> Result<(), String> {
    store
        .update_person(&id, &name, email.as_deref())
        .map_err(store_err)
}

/// Teilnehmende einer Besprechung (Kopfzeile des Details).
#[tauri::command]
#[specta::specta]
pub async fn meeting_participants(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<Participant>, String> {
    let own = self_emails(&app);
    store.participants_of(&meeting_id, &own).map_err(store_err)
}

// ---------------------------------------------------------------------------
// Pre-Meeting-Brief (P5e)
// ---------------------------------------------------------------------------

/// Was die Oberflaeche fuer den Knopf „Vorbereiten“ braucht.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct BriefInfo {
    pub event_key: String,
    pub event_title: String,
    /// Teilnehmende des Termins ohne mich (Anzeige).
    pub names: Vec<String>,
    /// Anzahl der fruehere Besprechungen mit gemeinsamen Teilnehmenden (0 =
    /// Knopf deaktiviert).
    pub shared_meetings: u32,
    /// Scope fuer `meeting_chat_ask`.
    pub filter: ScopeFilter,
    pub recipe_id: String,
    /// Wert der Recipe-Variable `teilnehmende` (Namen als ein Text).
    pub recipe_var: String,
    pub recipe_value: String,
    /// Der gespeicherte Brief-Verlauf dieses Termins (zweiter Klick).
    pub thread_id: Option<String>,
}

/// Brief-Zuschnitt zu einem Termin aus dem Kalender-Cache.
pub fn brief_info(
    store: &MeetingStore,
    event_key: &str,
    own: &[String],
) -> Result<BriefInfo, String> {
    let event = store
        .calendar_event(event_key)
        .map_err(store_err)?
        .ok_or_else(|| "calendar_event_not_found".to_string())?;
    let brief = people::brief_scope(store, &event, own).map_err(store_err)?;
    let thread_id = if brief.is_empty() {
        None
    } else {
        store
            .brief_thread(&event.uid, brief.newest_at)
            .map_err(store_err)?
    };
    Ok(BriefInfo {
        event_key: event.key,
        event_title: event.title,
        shared_meetings: brief.shared(),
        recipe_value: brief.names_text(),
        names: brief.names,
        filter: brief.filter,
        recipe_id: BRIEF_RECIPE_ID.to_string(),
        recipe_var: BRIEF_RECIPE_VAR.to_string(),
        thread_id,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn people_brief_info(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    event_key: String,
) -> Result<BriefInfo, String> {
    let own = self_emails(&app);
    brief_info(&store, &event_key, &own)
}

/// Das Hauptfenster soll den Brief zu diesem Termin oeffnen.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct BriefRequestEvent {
    pub event_key: String,
}

fn pending() -> &'static Mutex<Option<String>> {
    static PENDING: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

/// „Vorbereiten“ im Hinweisfenster: merkt den Termin, holt das Hauptfenster nach
/// vorn und meldet es ihm. Der Wunsch bleibt gemerkt, bis das Hauptfenster ihn
/// abholt (`people_brief_pending`): es kann gerade in einem anderen Bereich
/// stehen oder die Besprechungsseite noch aufbauen. Das Hinweisfenster schliesst
/// sich.
#[tauri::command]
#[specta::specta]
pub async fn people_brief_open(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    event_key: String,
) -> Result<(), String> {
    store
        .calendar_event(&event_key)
        .map_err(store_err)?
        .ok_or_else(|| "calendar_event_not_found".to_string())?;
    *pending().lock().unwrap_or_else(|e| e.into_inner()) = Some(event_key.clone());
    crate::show_main_window(&app);
    let _ = BriefRequestEvent { event_key }.emit(&app);
    crate::meeting_prompt::close(&app);
    Ok(())
}

/// Der gemerkte Wunsch „Brief oeffnen“, einmalig.
#[tauri::command]
#[specta::specta]
pub fn people_brief_pending() -> Option<String> {
    pending().lock().unwrap_or_else(|e| e.into_inner()).take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::CalendarKind;
    use crate::managers::calendar::model::{Attendee, CalEvent};
    use crate::managers::meetings::store::SyncMeta;
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus};

    fn attendee(email: &str, name: &str) -> Attendee {
        Attendee {
            email: Some(email.into()),
            name: Some(name.into()),
            organizer: false,
            is_self: false,
            partstat: None,
        }
    }

    fn event(uid: &str, start: i64, attendees: Vec<Attendee>) -> CalEvent {
        CalEvent {
            key: format!("src:{uid}:{start}"),
            source_id: "src".into(),
            uid: uid.into(),
            title: "Jour fixe Vertrieb".into(),
            starts_at: start,
            ends_at: start + 1_800_000,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees,
        }
    }

    fn store_with_source() -> (tempfile::TempDir, MeetingStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        s.calendar_source_add("src", CalendarKind::Ics, "K", None, 1)
            .unwrap();
        (dir, s)
    }

    fn put(s: &MeetingStore, events: &[CalEvent]) {
        s.calendar_replace_events(
            "src",
            events,
            &SyncMeta {
                has_attendee_data: true,
                etag: None,
                last_modified: None,
                now_ms: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn brief_info_is_empty_without_shared_meetings_and_names_the_others() {
        let (_d, s) = store_with_source();
        let ev = event(
            "u1",
            9_000_000,
            vec![
                attendee("anna@firma.de", "Anna Berg"),
                attendee("ich@wolff.de", "Ich"),
            ],
        );
        put(&s, &[ev.clone()]);
        let own = vec!["ich@wolff.de".to_string()];
        let info = brief_info(&s, &ev.key, &own).unwrap();
        assert_eq!(info.shared_meetings, 0);
        assert_eq!(info.names, vec!["Anna Berg".to_string()]);
        assert_eq!(info.thread_id, None);
        assert_eq!(info.recipe_id, "builtin:vorbereitung-termin");
        assert_eq!(info.recipe_value, "Anna Berg");
        assert_eq!(
            brief_info(&s, "src:gibt-es-nicht:1", &own).unwrap_err(),
            "calendar_event_not_found"
        );
    }

    #[test]
    fn brief_info_carries_the_scope_and_the_saved_thread() {
        let (_d, s) = store_with_source();
        let past = s
            .create_meeting("Frueher", MeetingSource::Live, Some(1))
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET started_at = 100 WHERE id = ?1",
                [&past.id],
            )
            .unwrap();
        s.set_status(&past.id, MeetingStatus::Ready).unwrap();
        people::participants_from_event(
            &s,
            &past.id,
            &event("alt", 1, vec![attendee("anna@firma.de", "Anna Berg")]),
        )
        .unwrap();
        let ev = event(
            "u1",
            9_000_000,
            vec![attendee("anna@firma.de", "Anna Berg")],
        );
        put(&s, &[ev.clone()]);
        let info = brief_info(&s, &ev.key, &[]).unwrap();
        assert_eq!(info.shared_meetings, 1);
        assert_eq!(info.filter.meeting_ids, Some(vec![past.id.clone()]));
        assert_eq!(info.filter.event_uid.as_deref(), Some("u1"));
        assert_eq!(info.thread_id, None, "noch kein Brief gefragt");
        // Nach einer Antwort im Verlauf mit der event_uid findet der zweite Klick sie.
        let scope = serde_json::json!({"kind": "global", "filter": info.filter}).to_string();
        let thread = s.thread_create(&scope, None, None).unwrap();
        s.thread_append(&thread.id, "assistant", "Brief", None, None)
            .unwrap();
        // Der Verlauf ist juenger als die Besprechung (Zeit heute >> 100 s).
        let again = brief_info(&s, &ev.key, &[]).unwrap();
        assert_eq!(again.thread_id.as_deref(), Some(thread.id.as_str()));
    }

    #[test]
    fn the_open_request_is_taken_once() {
        *pending().lock().unwrap() = Some("k".into());
        assert_eq!(people_brief_pending().as_deref(), Some("k"));
        assert_eq!(people_brief_pending(), None);
    }
}
