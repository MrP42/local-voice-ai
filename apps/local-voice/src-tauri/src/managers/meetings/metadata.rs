//! U7 (Issue #64): Metadaten einer Besprechung bearbeiten.
//!
//! Titel, Beschreibung (neu, mehrzeilig), Datum/Uhrzeit, Teilnehmende und
//! Projekte (Ordner, n:m) aendern sich in EINER Transaktion: entweder steht
//! alles wie gewuenscht, oder nichts hat sich veraendert (ein unbekanntes
//! Projekt ruft den Titel nicht halb geaendert zurueck). Dateiname und Quelle
//! (`source_path`) gehoeren nicht dazu und bleiben unberuehrt.
//!
//! Fehlercodes (Text, die Oberflaeche uebersetzt): `meeting_not_found`,
//! `title_empty`, `title_too_long`, `description_too_long`, `date_invalid`,
//! `person_not_found`, `folder_not_found`.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::store::{Meeting, MeetingStore};

/// Laengster Titel (Zeichen).
pub const TITLE_MAX_CHARS: usize = 300;
/// Laengste Beschreibung (Zeichen): genug fuer eine Tagesordnung, klein genug,
/// dass sie in jeden Prompt passt.
pub const DESCRIPTION_MAX_CHARS: usize = 4_000;
/// Spaetestes Datum (1. Januar 2100, Unix-Sekunden).
pub const MAX_STARTED_AT: i64 = 4_102_444_800;

/// Was geaendert werden soll. `None` = unveraendert; eine leere Beschreibung
/// loescht sie; `participant_ids` und `folder_ids` ersetzen die Menge.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MetadataEdit {
    pub title: Option<String>,
    pub description: Option<String>,
    /// Beginn in Unix-Sekunden.
    pub started_at: Option<i64>,
    /// Personen (`humans.id`), die teilgenommen haben.
    pub participant_ids: Option<Vec<String>>,
    /// Projekte (Ordner-IDs).
    pub folder_ids: Option<Vec<String>>,
}

/// Beschreibung in Speicherform: Zeilenenden vereinheitlicht, Raender
/// abgeschnitten, Leerstelle = keine Beschreibung. Zeilenumbrueche im Text
/// bleiben erhalten.
pub fn normalize_description(text: &str) -> Option<String> {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = unified.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn dedupe(ids: &[String]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for id in ids {
        if !out.contains(&id.as_str()) {
            out.push(id);
        }
    }
    out
}

impl MeetingStore {
    /// Wendet `edit` atomar an und liefert die aktuelle Besprechung.
    pub fn update_metadata(&self, meeting_id: &str, edit: &MetadataEdit) -> Result<Meeting> {
        // Eingaben zuerst pruefen: vor der ersten Schreibung steht fest, ob es
        // gelingen kann.
        let title = match edit.title.as_deref() {
            Some(raw) => {
                let t = raw.trim();
                if t.is_empty() {
                    return Err(anyhow!("title_empty"));
                }
                if t.chars().count() > TITLE_MAX_CHARS {
                    return Err(anyhow!("title_too_long"));
                }
                Some(t.to_string())
            }
            None => None,
        };
        let description = match edit.description.as_deref() {
            Some(raw) => {
                let d = normalize_description(raw);
                if d.as_deref()
                    .is_some_and(|d| d.chars().count() > DESCRIPTION_MAX_CHARS)
                {
                    return Err(anyhow!("description_too_long"));
                }
                Some(d)
            }
            None => None,
        };
        if let Some(ts) = edit.started_at {
            if !(0..=MAX_STARTED_AT).contains(&ts) {
                return Err(anyhow!("date_invalid"));
            }
        }

        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1 AND deleted_at IS NULL)",
            params![meeting_id],
            |row| row.get::<_, bool>(0),
        )? {
            return Err(anyhow!("meeting_not_found"));
        }
        let now = Utc::now().timestamp();

        // Felder der Besprechung: was nicht geaendert wird, bleibt wie es ist.
        if title.is_some() || description.is_some() || edit.started_at.is_some() {
            let (cur_title, cur_description, cur_started): (String, Option<String>, Option<i64>) =
                tx.query_row(
                    "SELECT title, description, started_at FROM meetings WHERE id = ?1",
                    params![meeting_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            tx.execute(
                "UPDATE meetings SET title = ?2, description = ?3, started_at = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![
                    meeting_id,
                    title.unwrap_or(cur_title),
                    description.unwrap_or(cur_description),
                    edit.started_at.or(cur_started),
                    now
                ],
            )?;
        }

        // Teilnehmende: die Menge ersetzen. Neue Personen tragen sich als
        // `manual`/`attendee` ein; bestehende Zeilen (Rolle, Quelle) bleiben.
        if let Some(ids) = &edit.participant_ids {
            let wanted = dedupe(ids);
            for id in &wanted {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM humans
                      WHERE id = ?1 AND deleted_at IS NULL AND merged_into IS NULL)",
                    params![id],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Err(anyhow!("person_not_found"));
                }
            }
            tx.execute(
                "DELETE FROM meeting_participants
                 WHERE meeting_id = ?1
                   AND human_id NOT IN (SELECT value FROM json_each(?2))",
                params![meeting_id, serde_json::to_string(&wanted)?],
            )?;
            for id in &wanted {
                tx.execute(
                    "INSERT INTO meeting_participants (meeting_id, human_id, role, source, created_at)
                     VALUES (?1, ?2, 'attendee', 'manual', ?3)
                     ON CONFLICT(meeting_id, human_id) DO NOTHING",
                    params![meeting_id, id, now],
                )?;
            }
        }

        // Projekte (Ordner, n:m): wie `set_meeting_folders`, nur in dieser
        // Transaktion; bestehende Zuordnungen behalten ihren Zeitstempel.
        if let Some(ids) = &edit.folder_ids {
            let wanted = dedupe(ids);
            for id in &wanted {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
                    params![id],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Err(anyhow!("folder_not_found"));
                }
            }
            tx.execute(
                "DELETE FROM meeting_folder_items
                 WHERE meeting_id = ?1 AND folder_id NOT IN (SELECT value FROM json_each(?2))",
                params![meeting_id, serde_json::to_string(&wanted)?],
            )?;
            for id in &wanted {
                tx.execute(
                    "INSERT INTO meeting_folder_items (folder_id, meeting_id, added_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(folder_id, meeting_id) DO NOTHING",
                    params![id, meeting_id, now],
                )?;
            }
        }

        tx.commit()?;
        self.get_meeting(meeting_id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};

    fn person(s: &MeetingStore, name: &str) -> String {
        s.upsert_person(None, Some(name), "manual").unwrap()
    }

    fn participants(s: &MeetingStore, id: &str) -> Vec<(String, String, String)> {
        let mut rows: Vec<(String, String, String)> = s
            .participants_of(id, &[])
            .unwrap()
            .into_iter()
            .map(|p| (p.name, p.role, p.source))
            .collect();
        rows.sort();
        rows
    }

    fn folder(s: &MeetingStore, name: &str) -> String {
        s.folder_save(None, name, None).unwrap().id
    }

    #[test]
    fn title_description_and_date_are_stored_and_the_rest_stays() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "Alter Titel", 1_750_000_000);
        s.set_source_path(&m.id, "C:/in/aufnahme.m4a").unwrap();
        let updated = s
            .update_metadata(
                &m.id,
                &MetadataEdit {
                    title: Some("  Neuer Titel  ".into()),
                    description: Some("Erste Zeile\r\nzweite Zeile\n\n".into()),
                    started_at: Some(1_760_000_000),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.title, "Neuer Titel");
        assert_eq!(
            updated.description.as_deref(),
            Some("Erste Zeile\nzweite Zeile"),
            "mehrzeilig, Zeilenenden vereinheitlicht, Raender weg"
        );
        assert_eq!(updated.started_at, Some(1_760_000_000));
        assert_eq!(
            updated.source_path.as_deref(),
            Some("C:/in/aufnahme.m4a"),
            "Dateiname/Quelle bleibt"
        );
        assert_eq!(updated.status, "ready");
        // Ein leerer Edit aendert nichts.
        let same = s.update_metadata(&m.id, &MetadataEdit::default()).unwrap();
        assert_eq!(same.title, "Neuer Titel");
        assert_eq!(same.description, updated.description);
    }

    #[test]
    fn an_empty_description_clears_it_and_none_keeps_it() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "T", 1_750_000_000);
        let edit = |d: Option<&str>| MetadataEdit {
            description: d.map(str::to_string),
            ..Default::default()
        };
        s.update_metadata(&m.id, &edit(Some("Agenda"))).unwrap();
        assert_eq!(
            s.update_metadata(&m.id, &edit(None))
                .unwrap()
                .description
                .as_deref(),
            Some("Agenda")
        );
        assert_eq!(
            s.update_metadata(&m.id, &edit(Some("   \n ")))
                .unwrap()
                .description,
            None
        );
        assert_eq!(s.description_of(&m.id).unwrap(), None);
    }

    #[test]
    fn invalid_input_changes_nothing() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "Titel", 1_750_000_000);
        let f = folder(&s, "Projekt A");
        let base = MetadataEdit {
            title: Some("Geaendert".into()),
            description: Some("Beschreibung".into()),
            folder_ids: Some(vec![f.clone()]),
            ..Default::default()
        };
        // Jeder Fehler NACH dem Titel und der Beschreibung rollt beide zurueck.
        let bad_folder = MetadataEdit {
            folder_ids: Some(vec![f.clone(), "gibt-es-nicht".into()]),
            ..base.clone()
        };
        let bad_person = MetadataEdit {
            participant_ids: Some(vec!["gibt-es-nicht".into()]),
            ..base.clone()
        };
        for (edit, code) in [
            (bad_folder, "folder_not_found"),
            (bad_person, "person_not_found"),
        ] {
            let err = s.update_metadata(&m.id, &edit).unwrap_err().to_string();
            assert_eq!(err, code);
            let now = s.get_meeting(&m.id).unwrap().unwrap();
            assert_eq!(now.title, "Titel", "{code}: Titel unveraendert");
            assert_eq!(now.description, None, "{code}: Beschreibung unveraendert");
            assert!(s.meeting_folder_ids(&m.id).unwrap().is_empty());
        }
        for (edit, code) in [
            (
                MetadataEdit {
                    title: Some("  ".into()),
                    ..Default::default()
                },
                "title_empty",
            ),
            (
                MetadataEdit {
                    title: Some("x".repeat(TITLE_MAX_CHARS + 1)),
                    ..Default::default()
                },
                "title_too_long",
            ),
            (
                MetadataEdit {
                    description: Some("x".repeat(DESCRIPTION_MAX_CHARS + 1)),
                    ..Default::default()
                },
                "description_too_long",
            ),
            (
                MetadataEdit {
                    started_at: Some(-1),
                    ..Default::default()
                },
                "date_invalid",
            ),
            (
                MetadataEdit {
                    started_at: Some(MAX_STARTED_AT + 1),
                    ..Default::default()
                },
                "date_invalid",
            ),
        ] {
            assert_eq!(
                s.update_metadata(&m.id, &edit).unwrap_err().to_string(),
                code
            );
        }
        assert_eq!(
            s.update_metadata("gibt-es-nicht", &MetadataEdit::default())
                .unwrap_err()
                .to_string(),
            "meeting_not_found"
        );
    }

    #[test]
    fn a_description_of_exactly_the_limit_is_accepted() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "T", 1_750_000_000);
        let long = "a".repeat(DESCRIPTION_MAX_CHARS);
        let updated = s
            .update_metadata(
                &m.id,
                &MetadataEdit {
                    description: Some(long.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.description, Some(long));
    }

    #[test]
    fn participants_are_replaced_as_a_set() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "T", 1_750_000_000);
        let anna = person(&s, "Anna Berg");
        let ben = person(&s, "Ben Koch");
        let cem = person(&s, "Cem Aydin");
        // Anna kommt aus dem Kalender (Organisatorin), Ben hat gesprochen.
        s.set_participants(&m.id, "calendar", &[(anna.clone(), "organizer")])
            .unwrap();
        s.set_participants(&m.id, "speaker", &[(ben.clone(), "speaker")])
            .unwrap();

        let edit = |ids: Vec<String>| MetadataEdit {
            participant_ids: Some(ids),
            ..Default::default()
        };
        s.update_metadata(&m.id, &edit(vec![anna.clone(), cem.clone(), cem.clone()]))
            .unwrap();
        let rows = participants(&s, &m.id);
        assert_eq!(
            rows.len(),
            2,
            "Ben entfaellt (auch wenn er Sprecher war), Cem kommt dazu, keine Dublette"
        );
        assert!(
            rows.contains(&("Anna Berg".into(), "organizer".into(), "calendar".into())),
            "Rolle und Quelle bleiben"
        );
        assert!(rows.contains(&("Cem Aydin".into(), "attendee".into(), "manual".into())));

        s.update_metadata(&m.id, &edit(vec![])).unwrap();
        assert!(
            participants(&s, &m.id).is_empty(),
            "eine leere Liste entfernt alle"
        );
        // Nicht angegeben = unveraendert.
        s.update_metadata(&m.id, &edit(vec![ben])).unwrap();
        s.update_metadata(
            &m.id,
            &MetadataEdit {
                title: Some("X".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(participants(&s, &m.id).len(), 1);
    }

    #[test]
    fn a_merged_or_deleted_person_cannot_be_added() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "T", 1_750_000_000);
        let keep = person(&s, "Anna Berg");
        let gone = person(&s, "Anna B.");
        s.merge_people(&keep, &gone).unwrap();
        let err = s
            .update_metadata(
                &m.id,
                &MetadataEdit {
                    participant_ids: Some(vec![gone]),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(err.to_string(), "person_not_found");
    }

    #[test]
    fn projects_are_replaced_as_a_set_and_keep_their_timestamps() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "T", 1_750_000_000);
        let a = folder(&s, "Projekt A");
        let b = folder(&s, "Projekt B");
        let c = folder(&s, "Projekt C");
        s.set_meeting_folders(&m.id, &[a.clone(), b.clone()])
            .unwrap();
        let added_at = |id: &str| -> i64 {
            s.get_connection()
                .unwrap()
                .query_row(
                    "SELECT added_at FROM meeting_folder_items WHERE folder_id = ?1 AND meeting_id = ?2",
                    params![id, m.id],
                    |r| r.get(0),
                )
                .unwrap()
        };
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meeting_folder_items SET added_at = 42 WHERE folder_id = ?1",
                params![a],
            )
            .unwrap();
        s.update_metadata(
            &m.id,
            &MetadataEdit {
                folder_ids: Some(vec![a.clone(), c.clone()]),
                ..Default::default()
            },
        )
        .unwrap();
        let mut ids = s.meeting_folder_ids(&m.id).unwrap();
        ids.sort();
        let mut expected = vec![a.clone(), c];
        expected.sort();
        assert_eq!(ids, expected, "B entfaellt, C kommt dazu (n:m)");
        assert_eq!(added_at(&a), 42, "A behaelt seinen Zeitstempel");
    }

    #[test]
    fn a_deleted_meeting_cannot_be_edited() {
        let (_dir, s) = tmp_store();
        let m = ready_meeting(&s, "T", 1_750_000_000);
        s.soft_delete_meeting(&m.id).unwrap();
        let err = s
            .update_metadata(
                &m.id,
                &MetadataEdit {
                    title: Some("X".into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(err.to_string(), "meeting_not_found");
    }

    #[test]
    fn normalize_keeps_inner_blank_lines_and_drops_the_edges() {
        assert_eq!(
            normalize_description("  a\n\nb \r\n"),
            Some("a\n\nb".to_string())
        );
        assert_eq!(normalize_description("\n\t "), None);
    }
}
