//! Pre-Meeting-Brief (M5, P5e; `entwurf/m5-m6-kalender-export.md` F18): der
//! Bereich fuer einen anstehenden Termin. Kein eigener Prompt: das eingebaute
//! Recipe `builtin:vorbereitung-termin` laeuft ueber den Chat (M4) mit einem
//! Scope, der auf fruehere Besprechungen mit gemeinsamen Teilnehmenden
//! eingegrenzt ist. Hier steht nur diese Eingrenzung.
//!
//! - nur Personen, die der Termin und frueher aufgenommene Besprechungen
//!   gemeinsam haben (bekannt = schon in `humans`; wer neu ist, hat keine
//!   gemeinsamen Besprechungen),
//! - „ich“ zaehlt nie (Kennzeichen `is_self`, Adressen aus `meeting_self_emails`),
//! - hoechstens [`MAX_BRIEF_MEETINGS`] juengste Besprechungen,
//! - Besprechungen, die zu genau diesem Termin aufgenommen wurden, zaehlen nicht.
//!
//! Ohne gemeinsame Besprechungen ist der Brief leer (`Brief::is_empty`); die
//! Oberflaeche deaktiviert dann den Knopf „Vorbereiten“.

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::managers::calendar::model::CalEvent;
use crate::managers::meetings::search::index::ScopeFilter;
use crate::managers::meetings::store::MeetingStore;

use super::normalize::{display_name, name_from_email, normalize_email, normalize_name};
use super::{own_set, shared_meetings};

/// Wie viele fruehere Besprechungen in den Brief eingehen.
pub const MAX_BRIEF_MEETINGS: u32 = 20;

/// Laenge des Namenstexts fuer `{{teilnehmende}}` (das Recipe erlaubt 200 Zeichen).
const MAX_NAMES_CHARS: usize = 180;

/// Der Zuschnitt eines Briefs.
#[derive(Clone, Debug)]
pub struct Brief {
    /// Scope fuer `meeting_chat_ask`: `meeting_ids` = die frueheren
    /// Besprechungen, `event_uid` = der Termin (Verlauf-Wiederverwendung).
    pub filter: ScopeFilter,
    /// Anzeigenamen der Teilnehmenden des Termins ohne mich (auch der noch
    /// unbekannten), ohne Duplikate.
    pub names: Vec<String>,
    /// Zeitpunkt (s) der juengsten gemeinsamen Besprechung.
    pub newest_at: Option<i64>,
}

impl Brief {
    /// Anzahl der gemeinsamen Besprechungen.
    pub fn shared(&self) -> u32 {
        self.filter
            .meeting_ids
            .as_ref()
            .map_or(0, |ids| ids.len() as u32)
    }

    /// Keine gemeinsame Besprechung: nichts vorzubereiten.
    pub fn is_empty(&self) -> bool {
        self.shared() == 0
    }

    /// Die Namen als ein Text fuer die Recipe-Variable (hoechstens
    /// [`MAX_NAMES_CHARS`] Zeichen, gekappt an einer Namensgrenze).
    pub fn names_text(&self) -> String {
        let mut out = String::new();
        let mut used = 0usize;
        for (i, name) in self.names.iter().enumerate() {
            let piece = if i == 0 {
                name.clone()
            } else {
                format!(", {name}")
            };
            let rest = self.names.len() - i;
            if used + piece.chars().count() > MAX_NAMES_CHARS {
                out.push_str(&format!(" u. a. ({rest} weitere)"));
                break;
            }
            used += piece.chars().count();
            out.push_str(&piece);
        }
        out
    }
}

/// Der Brief zum Termin. `self_emails`: die eigenen Adressen (Einstellung
/// „Meine E-Mail-Adressen“).
pub fn brief_scope(
    store: &MeetingStore,
    event: &CalEvent,
    self_emails: &[String],
) -> Result<Brief> {
    let own = own_set(self_emails);
    let mut names: Vec<String> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut ids: Vec<String> = Vec::new();
    for attendee in &event.attendees {
        let email = attendee.email.as_deref().and_then(normalize_email);
        if attendee.is_self || email.as_ref().is_some_and(|e| own.contains(e)) {
            continue;
        }
        let named = attendee.name.as_deref().and_then(display_name);
        if email.is_none() && named.is_none() {
            continue;
        }
        let found = store.find_person(email.as_deref(), named.as_deref())?;
        if found.as_ref().is_some_and(|f| f.is_me(&own)) {
            continue;
        }
        let shown = found
            .as_ref()
            .map(|f| f.name.clone())
            .or(named)
            .or_else(|| email.as_deref().and_then(name_from_email))
            .or(email);
        if let Some(shown) = shown {
            if seen_names.insert(normalize_name(&shown)) {
                names.push(shown);
            }
        }
        if let Some(found) = found {
            if !ids.contains(&found.id) {
                ids.push(found.id);
            }
        }
    }

    let shared = {
        let conn = store.get_connection()?;
        shared_meetings(&conn, &ids, MAX_BRIEF_MEETINGS, Some(&event.key))?
    };
    let newest_at = shared.iter().map(|(_, at)| *at).max();
    let meeting_ids: Vec<String> = shared.into_iter().map(|(id, _)| id).collect();
    Ok(Brief {
        filter: ScopeFilter {
            meeting_ids: Some(meeting_ids),
            event_uid: Some(event.uid.clone()),
            ..ScopeFilter::default()
        },
        names,
        newest_at,
    })
}

impl MeetingStore {
    /// Der gespeicherte Brief-Verlauf zu diesem Termin (Serien-UID): der juengste
    /// globale Verlauf, dessen Scope diese `event_uid` traegt. Er gilt nur, solange
    /// keine neuere gemeinsame Besprechung dazugekommen ist (`newest_meeting_at`,
    /// s): sonst waere der Brief veraltet und wird neu gefragt.
    pub fn brief_thread(
        &self,
        event_uid: &str,
        newest_meeting_at: Option<i64>,
    ) -> Result<Option<String>> {
        let conn = self.get_connection()?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT t.id, t.created_at FROM chat_threads t
                 WHERE t.deleted_at IS NULL AND t.meeting_id IS NULL
                   AND json_extract(t.scope_json, '$.filter.event_uid') = ?1
                   AND EXISTS (SELECT 1 FROM chat_messages cm
                               WHERE cm.thread_id = t.id AND cm.role = 'assistant')
                 ORDER BY t.updated_at DESC, t.rowid DESC LIMIT 1",
                params![event_uid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(id, created_at)| {
            newest_meeting_at
                .is_none_or(|newest| created_at >= newest)
                .then_some(id)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::Attendee;
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus};
    use crate::managers::people::participants_from_event;

    fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        (dir, s)
    }

    fn ready(s: &MeetingStore, title: &str, started_at: i64) -> String {
        let m = s
            .create_meeting(title, MeetingSource::Live, Some(1))
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
                params![started_at, m.id],
            )
            .unwrap();
        s.set_status(&m.id, MeetingStatus::Ready).unwrap();
        m.id
    }

    fn person(email: &str, name: &str) -> Attendee {
        Attendee {
            email: Some(email.into()),
            name: Some(name.into()),
            organizer: false,
            is_self: false,
            partstat: None,
        }
    }

    fn event(uid: &str, attendees: Vec<Attendee>) -> CalEvent {
        CalEvent {
            key: format!("s:{uid}:9000000"),
            source_id: "s".into(),
            uid: uid.into(),
            title: "Jour fixe".into(),
            starts_at: 9_000_000,
            ends_at: 9_100_000,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees,
        }
    }

    /// Eine fertige Besprechung mit diesen Teilnehmenden (als frueherer Termin).
    fn past_meeting(s: &MeetingStore, title: &str, at: i64, who: Vec<Attendee>) -> String {
        let id = ready(s, title, at);
        participants_from_event(s, &id, &event("alt", who)).unwrap();
        id
    }

    #[test]
    fn only_meetings_with_shared_attendees_count() {
        let (_d, s) = tmp_store();
        let with_anna = past_meeting(
            &s,
            "Mit Anna",
            100,
            vec![
                person("anna@firma.de", "Anna Berg"),
                person("x@firma.de", "Xaver"),
            ],
        );
        let _other = past_meeting(&s, "Ohne", 200, vec![person("zed@andere.de", "Zed")]);
        let ev = event(
            "u1",
            vec![
                person("anna@firma.de", "Anna Berg"),
                person("neu@irgendwo.de", "Neu"),
            ],
        );
        let brief = brief_scope(&s, &ev, &[]).unwrap();
        assert_eq!(brief.filter.meeting_ids, Some(vec![with_anna]));
        assert_eq!(brief.filter.event_uid.as_deref(), Some("u1"));
        assert_eq!(brief.shared(), 1);
        assert!(!brief.is_empty());
        // Auch die noch unbekannte Person steht im Namenstext.
        assert_eq!(
            brief.names,
            vec!["Anna Berg".to_string(), "Neu".to_string()]
        );
        assert_eq!(brief.names_text(), "Anna Berg, Neu");
    }

    #[test]
    fn myself_is_excluded_by_flag_by_address_and_by_person() {
        let (_d, s) = tmp_store();
        // Nur „ich“ verbindet die Besprechung mit dem Termin: kein Brief.
        let _m = past_meeting(
            &s,
            "Nur ich und Zed",
            100,
            vec![
                person("ich@wolff.de", "Patrick Wolff"),
                person("zed@x.de", "Zed"),
            ],
        );
        let ev = event(
            "u1",
            vec![
                person("ich@wolff.de", "Patrick Wolff"),
                person("neu@y.de", "Neu"),
            ],
        );
        let own = vec!["Ich@Wolff.de".to_string()];
        let by_address = brief_scope(&s, &ev, &own).unwrap();
        assert!(by_address.is_empty());
        assert_eq!(
            by_address.names,
            vec!["Neu".to_string()],
            "ich fehlt im Namenstext"
        );
        // Ohne die Einstellung waere „ich“ gemeinsam.
        assert!(!brief_scope(&s, &ev, &[]).unwrap().is_empty());
        // Kennzeichen an der Person (aus dem eigenen Konto) genuegt.
        let me = s.find_person(Some("ich@wolff.de"), None).unwrap().unwrap();
        s.mark_person_self(&me.id).unwrap();
        assert!(brief_scope(&s, &ev, &[]).unwrap().is_empty());
        // Kennzeichen am Teilnehmer des Termins ebenso.
        let mut flagged = person("ich@wolff.de", "Patrick Wolff");
        flagged.is_self = true;
        let (_d2, s2) = tmp_store();
        past_meeting(&s2, "M", 100, vec![person("ich@wolff.de", "Patrick Wolff")]);
        let ev2 = event("u2", vec![flagged]);
        assert!(brief_scope(&s2, &ev2, &[]).unwrap().names.is_empty());
    }

    #[test]
    fn at_most_twenty_newest_meetings_go_in() {
        let (_d, s) = tmp_store();
        let mut ids = Vec::new();
        for i in 0..25 {
            ids.push(past_meeting(
                &s,
                &format!("M{i}"),
                1_000 + i,
                vec![person("anna@firma.de", "Anna Berg")],
            ));
        }
        let ev = event("u1", vec![person("anna@firma.de", "Anna Berg")]);
        let brief = brief_scope(&s, &ev, &[]).unwrap();
        let got = brief.filter.meeting_ids.clone().unwrap();
        assert_eq!(got.len(), MAX_BRIEF_MEETINGS as usize);
        // Neueste zuerst: M24 ... M5.
        let want: Vec<String> = ids.iter().rev().take(20).cloned().collect();
        assert_eq!(got, want);
        assert_eq!(brief.newest_at, Some(1_024));
    }

    #[test]
    fn no_shared_meetings_means_an_empty_brief() {
        let (_d, s) = tmp_store();
        let ev = event("u1", vec![person("neu@irgendwo.de", "Neu")]);
        let brief = brief_scope(&s, &ev, &[]).unwrap();
        assert!(brief.is_empty());
        assert_eq!(brief.filter.meeting_ids, Some(vec![]));
        assert_eq!(brief.newest_at, None);
        // Auch ein Termin ohne Teilnehmende oder nur mit mir.
        let empty = brief_scope(&s, &event("u2", vec![]), &[]).unwrap();
        assert!(empty.is_empty() && empty.names.is_empty());
    }

    #[test]
    fn unfinished_deleted_and_own_event_meetings_do_not_count() {
        let (_d, s) = tmp_store();
        let who = || vec![person("anna@firma.de", "Anna Berg")];
        let done = past_meeting(&s, "Fertig", 100, who());
        // Laeuft noch: kein Stoff fuer einen Brief.
        let running = s
            .create_meeting("Laeuft", MeetingSource::Live, Some(1))
            .unwrap();
        participants_from_event(&s, &running.id, &event("alt", who())).unwrap();
        // Geloescht.
        let deleted = past_meeting(&s, "Weg", 200, who());
        s.soft_delete_meeting(&deleted).unwrap();
        // Zu genau diesem Termin aufgenommen: nicht „frueher“.
        let ev = event("u1", who());
        let own_meeting = past_meeting(&s, "Dieser Termin", 300, who());
        let link_event = ev.clone();
        s.link_meeting_event(&own_meeting, &link_event, "prompt", 1)
            .unwrap();
        let brief = brief_scope(&s, &ev, &[]).unwrap();
        assert_eq!(brief.filter.meeting_ids, Some(vec![done]));
        let _ = running;
    }

    #[test]
    fn a_long_list_of_names_is_cut_at_a_name_boundary() {
        let brief = Brief {
            filter: ScopeFilter::default(),
            names: (0..40).map(|i| format!("Teilnehmer Nummer {i}")).collect(),
            newest_at: None,
        };
        let text = brief.names_text();
        assert!(text.chars().count() <= 200, "{}", text.chars().count());
        assert!(text.starts_with("Teilnehmer Nummer 0, Teilnehmer Nummer 1"));
        assert!(text.contains(" u. a. ("), "{text}");
        assert!(!text.contains("Nummer 30"), "gekappt: {text}");
        assert!(text.ends_with(" weitere)"), "{text}");
        // Jeder genannte Name steht vollstaendig da (kein halber am Ende).
        let listed = text.split(" u. a. (").next().unwrap();
        assert!(listed
            .split(", ")
            .all(|n| n.starts_with("Teilnehmer Nummer ")));
    }

    #[test]
    fn the_stored_brief_is_reused_until_a_newer_meeting_exists() {
        let (_d, s) = tmp_store();
        let scope = |uid: &str| {
            serde_json::json!({"kind": "global", "filter": {"event_uid": uid, "meeting_ids": []}})
                .to_string()
        };
        // Ohne Antwort (nur eine Frage) gibt es nichts wiederzuoeffnen.
        let empty = s.thread_create(&scope("u1"), None, None).unwrap();
        s.thread_append(&empty.id, "user", "Bereite mich vor", None, None)
            .unwrap();
        assert_eq!(s.brief_thread("u1", None).unwrap(), None);
        s.thread_append(&empty.id, "assistant", "Der Brief.", None, None)
            .unwrap();
        assert_eq!(s.brief_thread("u1", None).unwrap(), Some(empty.id.clone()));
        // Anderer Termin, andere Serie: kein Treffer.
        assert_eq!(s.brief_thread("u2", None).unwrap(), None);
        // Eine Besprechung, die NACH dem Brief entstand: veraltet.
        let created = s.thread_get(&empty.id).unwrap().unwrap().0.created_at;
        assert_eq!(
            s.brief_thread("u1", Some(created - 1)).unwrap(),
            Some(empty.id.clone())
        );
        assert_eq!(s.brief_thread("u1", Some(created + 1)).unwrap(), None);
        // Der juengste Verlauf gewinnt.
        let newer = s.thread_create(&scope("u1"), None, None).unwrap();
        s.thread_append(&newer.id, "assistant", "Neuer Brief.", None, None)
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE chat_threads SET updated_at = updated_at + 10 WHERE id = ?1",
                params![newer.id],
            )
            .unwrap();
        assert_eq!(s.brief_thread("u1", None).unwrap(), Some(newer.id));
        // Ein geloeschter Verlauf zaehlt nicht.
        s.thread_delete(&empty.id).unwrap();
    }
}
