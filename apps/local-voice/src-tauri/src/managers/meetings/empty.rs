//! G1 (Goal Issues-Abschluss, #70): der leere Eintrag.
//!
//! Eine Besprechung ohne Audio, ohne Transkript und ohne Quelle: ein Notizblock
//! in einem Projekt, in den der Nutzer spaeter aufnimmt, eine Datei importiert
//! oder einen YouTube-Link legt. Sie ist KEINE neue Tabelle und kein neuer
//! Status: `source = 'empty'`, `status = 'ready'` (wie eine YouTube-Besprechung
//! vor dem Transkript). Damit gelten Liste, Suche, MCP, Export, Loeschen und
//! die Notizen ohne Sonderweg; KI-Notizen und Protokoll melden wie bei jeder
//! fertigen Besprechung ohne Transkript `no_transcript`. Keine Migration.
//!
//! Das FUELLEN (`fill_empty_*`) wandelt dieselbe Zeile in eine Aufnahme, einen
//! Import oder eine YouTube-Quelle um: Id, Notizen, Projekte und Teilnehmende
//! bleiben. Nur ein Eintrag mit `source = 'empty'` laesst sich fuellen, die
//! Pruefung steht im UPDATE selbst (zwei gleichzeitige Fuellungen: eine gewinnt,
//! die andere bekommt `target_not_empty`).
//!
//! Der Titel: beim Anlegen merkt sich der Eintrag den vorgeschlagenen Titel
//! (`metadata_json.empty_default_title`). Der Import und die YouTube-Quelle
//! ersetzen ihn nur, solange er unveraendert so dasteht; hat der Nutzer ihn
//! umbenannt, bleibt sein Titel.
//!
//! Fehlercodes (Text des `anyhow`-Fehlers): `title_empty`, `title_too_long`,
//! `folder_not_found`, `meeting_not_found`, `target_not_empty`.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use ulid::Ulid;

use super::metadata::TITLE_MAX_CHARS;
use super::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore};

/// `meetings.source` des leeren Eintrags.
pub const SOURCE_EMPTY: &str = MeetingSource::Empty.as_str();
/// Fehlercode: das Ziel ist kein leerer Eintrag (mehr).
pub const TARGET_NOT_EMPTY: &str = "target_not_empty";
/// Schluessel in `metadata_json`: der Titel, den der Eintrag beim Anlegen bekam.
const DEFAULT_TITLE_KEY: &str = "empty_default_title";

/// Fehlertext fuer die Oberflaeche: ein Code des leeren Eintrags geht unveraendert
/// durch (`target_not_empty`, `meeting_not_found`), alles andere bekommt `prefix`.
pub fn coded_error(prefix: &str, e: &anyhow::Error) -> String {
    let text = e.to_string();
    if matches!(
        text.as_str(),
        "target_not_empty" | "meeting_not_found" | "title_empty" | "title_too_long"
    ) {
        text
    } else {
        format!("{prefix}: {e}")
    }
}

/// Wozu ein leerer Eintrag wird.
pub struct EmptyFill<'a> {
    pub source: MeetingSource,
    pub status: MeetingStatus,
    pub consent_confirmed_at: Option<i64>,
    pub source_path: Option<&'a str>,
    /// Neuer Titel; `None` laesst den Titel stehen.
    pub title: Option<&'a str>,
    /// `true`: den Titel nur ersetzen, solange er noch der vorgeschlagene ist
    /// (Import, YouTube). `false`: der Nutzer hat ihn eben im Startdialog gesehen
    /// und bestaetigt (Aufnahme).
    pub only_if_default: bool,
    /// Ein Schluessel, der in `metadata_json` dazukommt (YouTube-Angaben).
    pub metadata: Option<(&'a str, Value)>,
}

impl<'a> EmptyFill<'a> {
    pub fn new(source: MeetingSource, status: MeetingStatus) -> Self {
        Self {
            source,
            status,
            consent_confirmed_at: None,
            source_path: None,
            title: None,
            only_if_default: false,
            metadata: None,
        }
    }
}

fn clean_title(title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(anyhow!("title_empty"));
    }
    if title.chars().count() > TITLE_MAX_CHARS {
        return Err(anyhow!("title_too_long"));
    }
    Ok(title.to_string())
}

/// Fuellt die Zeile `id` im Rahmen von `tx` (die Aufrufer haengen weitere
/// Schritte an dieselbe Transaktion: Warteschlange, YouTube-Herkunft).
pub(crate) fn fill_empty_tx(tx: &Transaction<'_>, id: &str, fill: &EmptyFill<'_>) -> Result<()> {
    let row: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT title, source, metadata_json FROM meetings
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((current_title, source, metadata_json)) = row else {
        return Err(anyhow!("meeting_not_found"));
    };
    if source != SOURCE_EMPTY {
        return Err(anyhow!(TARGET_NOT_EMPTY));
    }

    // metadata_json: Marker raus, ggf. ein Schluessel dazu. Was kein JSON-Objekt
    // ist, bleibt unberuehrt (Fehler statt Datenverlust, wie `set_metadata_key`).
    let mut metadata: serde_json::Map<String, Value> = match metadata_json.as_deref() {
        None | Some("") => serde_json::Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            _ => return Err(anyhow!("metadata_not_an_object")),
        },
    };
    let default_title = metadata
        .remove(DEFAULT_TITLE_KEY)
        .and_then(|v| v.as_str().map(str::to_string));
    if let Some((key, value)) = &fill.metadata {
        metadata.insert((*key).to_string(), value.clone());
    }
    let metadata_text = (!metadata.is_empty()).then(|| Value::Object(metadata).to_string());

    let title = match fill.title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(new)
            if !fill.only_if_default || default_title.as_deref() == Some(current_title.as_str()) =>
        {
            clean_title(new)?
        }
        _ => current_title,
    };

    let now = Utc::now().timestamp();
    let changed = tx.execute(
        "UPDATE meetings
            SET source = ?1, status = ?2, title = ?3, consent_confirmed_at = ?4,
                source_path = ?5, metadata_json = ?6, updated_at = ?7
          WHERE id = ?8 AND source = 'empty' AND deleted_at IS NULL",
        params![
            fill.source.as_str(),
            fill.status.as_str(),
            title,
            fill.consent_confirmed_at,
            fill.source_path,
            metadata_text,
            now,
            id
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!(TARGET_NOT_EMPTY));
    }
    Ok(())
}

impl MeetingStore {
    /// Legt einen leeren Eintrag an, auf Wunsch gleich in `folder_id`. Der Titel
    /// ist der vorgeschlagene ("Neue Besprechung"); er wird als solcher gemerkt.
    pub fn create_empty_meeting(&self, title: &str, folder_id: Option<&str>) -> Result<Meeting> {
        let title = clean_title(title)?;
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(folder) = folder_id {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
                params![folder],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(anyhow!("folder_not_found"));
            }
        }
        let id = Ulid::new().to_string();
        let now = Utc::now().timestamp();
        let metadata = serde_json::json!({ DEFAULT_TITLE_KEY: title }).to_string();
        tx.execute(
            "INSERT INTO meetings (id, title, status, source, metadata_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                id,
                title,
                MeetingStatus::Ready.as_str(),
                SOURCE_EMPTY,
                metadata,
                now
            ],
        )?;
        if let Some(folder) = folder_id {
            tx.execute(
                "INSERT INTO meeting_folder_items (folder_id, meeting_id, added_at)
                 VALUES (?1, ?2, ?3)",
                params![folder, id, now],
            )?;
        }
        tx.commit()?;
        self.get_meeting(&id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))
    }

    /// Ist `id` ein lebender, noch leerer Eintrag? (Pruefhilfe der Tests.)
    #[cfg(test)]
    pub fn is_empty_meeting(&self, id: &str) -> Result<bool> {
        let conn = self.get_connection()?;
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings
                           WHERE id = ?1 AND source = 'empty' AND deleted_at IS NULL)",
            params![id],
            |r| r.get(0),
        )?)
    }

    /// Wandelt einen leeren Eintrag um und liefert ihn zurueck.
    pub fn fill_empty_meeting(&self, id: &str, fill: &EmptyFill<'_>) -> Result<Meeting> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        fill_empty_tx(&tx, id, fill)?;
        tx.commit()?;
        self.get_meeting(id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))
    }

    /// Macht aus einer gescheiterten Aufnahme wieder einen leeren Eintrag: der
    /// Start schlug fehl, bevor irgendetwas aufgenommen wurde, und der Nutzer
    /// soll seinen Notizblock behalten, nicht eine "fehlgeschlagene" Aufnahme
    /// ohne Ton. Greift nur fuer eine `live`-Zeile ohne Transkript-Saetze;
    /// `false`, wenn sie so nicht aussieht (dann bleibt alles, wie es ist).
    pub fn restore_empty_meeting(&self, id: &str) -> Result<bool> {
        let now = Utc::now().timestamp();
        let conn = self.get_connection()?;
        let changed = conn.execute(
            "UPDATE meetings
                SET source = 'empty', status = 'ready', consent_confirmed_at = NULL,
                    source_path = NULL, mic_audio_path = NULL, system_audio_path = NULL,
                    duration_ms = NULL, started_at = NULL, ended_at = NULL, updated_at = ?2
              WHERE id = ?1 AND deleted_at IS NULL AND source = 'live'
                AND status IN ('recording', 'failed')
                AND NOT EXISTS (SELECT 1 FROM transcripts
                                WHERE meeting_id = ?1 AND segments_json != '[]')",
            params![id, now],
        )?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::store::TranscriptDelta;

    fn store() -> MeetingStore {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        std::mem::forget(dir);
        s
    }

    fn folder(s: &MeetingStore, name: &str) -> String {
        s.folder_save(None, name, None).unwrap().id
    }

    fn code(e: anyhow::Error) -> String {
        e.to_string()
    }

    #[test]
    fn an_empty_entry_is_ready_without_source_audio_or_transcript() {
        let s = store();
        let m = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        assert_eq!(m.title, "Neue Besprechung");
        assert_eq!(m.source, SOURCE_EMPTY);
        assert_eq!(m.status, "ready");
        assert!(m.mic_audio_path.is_none() && m.system_audio_path.is_none());
        assert!(m.source_path.is_none() && m.consent_confirmed_at.is_none());
        assert!(s.get_segments(&m.id).unwrap().is_empty());
        assert!(s.is_empty_meeting(&m.id).unwrap());
        // Er steht in der Liste wie jede Besprechung.
        assert_eq!(s.list_meetings(0, 10).unwrap().len(), 1);
        assert!(s.meeting_folder_ids(&m.id).unwrap().is_empty());
    }

    #[test]
    fn an_empty_entry_can_be_created_inside_a_project() {
        let s = store();
        let f = folder(&s, "Kunde A");
        let m = s.create_empty_meeting("Neue Besprechung", Some(&f)).unwrap();
        assert_eq!(s.meeting_folder_ids(&m.id).unwrap(), vec![f.clone()]);
        let counts = s.folder_counts().unwrap();
        assert_eq!(counts.all, 1);
        assert_eq!(counts.unfiled, 0);
    }

    #[test]
    fn creating_refuses_a_bad_title_or_an_unknown_project_and_leaves_nothing() {
        let s = store();
        assert_eq!(code(s.create_empty_meeting("  ", None).unwrap_err()), "title_empty");
        assert_eq!(
            code(s.create_empty_meeting(&"x".repeat(TITLE_MAX_CHARS + 1), None).unwrap_err()),
            "title_too_long"
        );
        assert_eq!(
            code(s.create_empty_meeting("T", Some("gibt-es-nicht")).unwrap_err()),
            "folder_not_found"
        );
        assert!(s.list_meetings(0, 10).unwrap().is_empty());
    }

    #[test]
    fn the_title_can_be_renamed_and_the_entry_deleted_like_any_meeting() {
        let s = store();
        let m = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        s.set_title(&m.id, "Kick-off").unwrap();
        assert_eq!(s.get_meeting(&m.id).unwrap().unwrap().title, "Kick-off");
        let paths = s.soft_delete_meeting(&m.id).unwrap();
        assert!(paths.is_empty(), "kein Audio zu loeschen");
        assert!(s.get_meeting(&m.id).unwrap().is_none());
        assert!(s.list_meetings(0, 10).unwrap().is_empty());
    }

    #[test]
    fn notes_typed_into_an_empty_entry_survive_filling_it() {
        let s = store();
        let f = folder(&s, "Projekt");
        let m = s.create_empty_meeting("Neue Besprechung", Some(&f)).unwrap();
        let mut fill = EmptyFill::new(MeetingSource::Live, MeetingStatus::Recording);
        fill.consent_confirmed_at = Some(42);
        fill.title = Some("Montag");
        let filled = s.fill_empty_meeting(&m.id, &fill).unwrap();
        assert_eq!(filled.id, m.id, "dieselbe Besprechung");
        assert_eq!(filled.source, "live");
        assert_eq!(filled.status, "recording");
        assert_eq!(filled.consent_confirmed_at, Some(42));
        assert_eq!(filled.title, "Montag");
        assert_eq!(s.meeting_folder_ids(&m.id).unwrap(), vec![f], "Projekt bleibt");
        assert!(!s.is_empty_meeting(&m.id).unwrap());
    }

    #[test]
    fn an_import_keeps_a_renamed_title_but_replaces_the_suggested_one() {
        let s = store();
        let untouched = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        let renamed = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        s.set_title(&renamed.id, "Mein Titel").unwrap();
        for id in [&untouched.id, &renamed.id] {
            let mut fill = EmptyFill::new(MeetingSource::Import, MeetingStatus::Processing);
            fill.title = Some("Aufnahme vom Montag");
            fill.only_if_default = true;
            fill.source_path = Some("C:/Audio/montag.m4a");
            s.fill_empty_meeting(id, &fill).unwrap();
        }
        let a = s.get_meeting(&untouched.id).unwrap().unwrap();
        let b = s.get_meeting(&renamed.id).unwrap().unwrap();
        assert_eq!(a.title, "Aufnahme vom Montag");
        assert_eq!(b.title, "Mein Titel");
        assert_eq!(a.source_path.as_deref(), Some("C:/Audio/montag.m4a"));
        // Der Marker ist weg: nichts bleibt in metadata_json zurueck.
        assert_eq!(s.metadata_json(&untouched.id).unwrap(), None);
    }

    #[test]
    fn only_an_empty_entry_can_be_filled_and_only_once() {
        let s = store();
        let m = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        let fill = || EmptyFill::new(MeetingSource::Import, MeetingStatus::Processing);
        s.fill_empty_meeting(&m.id, &fill()).unwrap();
        // Zweites Fuellen: das Ziel ist nicht mehr leer.
        assert_eq!(code(s.fill_empty_meeting(&m.id, &fill()).unwrap_err()), TARGET_NOT_EMPTY);
        // Eine ganz normale Besprechung ist nie ein Ziel.
        let live = s.create_meeting("Live", MeetingSource::Live, Some(1)).unwrap();
        assert_eq!(code(s.fill_empty_meeting(&live.id, &fill()).unwrap_err()), TARGET_NOT_EMPTY);
        let yt = s.create_meeting("Video", MeetingSource::Youtube, None).unwrap();
        assert_eq!(code(s.fill_empty_meeting(&yt.id, &fill()).unwrap_err()), TARGET_NOT_EMPTY);
        // Unbekannt und geloescht.
        assert_eq!(
            code(s.fill_empty_meeting("gibt-es-nicht", &fill()).unwrap_err()),
            "meeting_not_found"
        );
        let gone = s.create_empty_meeting("Weg", None).unwrap();
        s.soft_delete_meeting(&gone.id).unwrap();
        assert_eq!(
            code(s.fill_empty_meeting(&gone.id, &fill()).unwrap_err()),
            "meeting_not_found"
        );
        // Die abgelehnten Versuche haben die Zeilen nicht angefasst.
        assert_eq!(s.get_meeting(&live.id).unwrap().unwrap().source, "live");
        assert_eq!(s.get_meeting(&yt.id).unwrap().unwrap().source, "youtube");
    }

    #[test]
    fn a_failed_recording_start_gives_the_empty_entry_back() {
        let s = store();
        let m = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        let mut fill = EmptyFill::new(MeetingSource::Live, MeetingStatus::Recording);
        fill.consent_confirmed_at = Some(7);
        s.fill_empty_meeting(&m.id, &fill).unwrap();
        s.set_audio_paths(&m.id, Some("C:/x/mic.wav"), None, None).unwrap();
        s.set_status(&m.id, MeetingStatus::Failed).unwrap();

        assert!(s.restore_empty_meeting(&m.id).unwrap());
        let back = s.get_meeting(&m.id).unwrap().unwrap();
        assert_eq!((back.source.as_str(), back.status.as_str()), ("empty", "ready"));
        assert!(back.mic_audio_path.is_none() && back.consent_confirmed_at.is_none());
        assert!(s.is_empty_meeting(&m.id).unwrap());
    }

    #[test]
    fn restoring_never_touches_a_recording_that_has_text() {
        let s = store();
        let m = s.create_empty_meeting("Neue Besprechung", None).unwrap();
        s.fill_empty_meeting(
            &m.id,
            &EmptyFill::new(MeetingSource::Live, MeetingStatus::Recording),
        )
        .unwrap();
        s.append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: vec![crate::managers::meetings::store::StoredSegment {
                    segment_index: 0,
                    text: "Hallo".into(),
                    start_ms: 0,
                    end_ms: 1_000,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                }],
            },
        )
        .unwrap();
        s.set_status(&m.id, MeetingStatus::Failed).unwrap();
        assert!(!s.restore_empty_meeting(&m.id).unwrap());
        assert_eq!(s.get_meeting(&m.id).unwrap().unwrap().source, "live");
        // Und eine Import-Besprechung ist nie "wiederherzustellen".
        let imp = s.create_meeting("Import", MeetingSource::Import, None).unwrap();
        assert!(!s.restore_empty_meeting(&imp.id).unwrap());
    }
}
