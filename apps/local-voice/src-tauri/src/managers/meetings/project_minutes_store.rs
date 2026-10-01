//! G3 (#70, U9 aus #64): Ablage der Projekt-Protokolle.
//!
//! Ein Projekt-Protokoll (oder eine Projekt-Zusammenfassung) fasst mehrere
//! Aufnahmen eines Projekts (= Ordner) zusammen und gehoert dem PROJEKT, nicht
//! einer Besprechung: es steht in `project_minutes`, nicht in
//! `meeting_documents` (deren `meeting_id` ist Pflicht). Die Zeile traegt alles,
//! was die Anzeige und der Export brauchen:
//!
//! - `body`: das fertige Markdown (Export, Kopieren),
//! - `sections_json`: die Abschnitte MIT Quellen je Eintrag (Aufnahme, Segment,
//!   Zeitstempel), daraus baut die Oberflaeche die anklickbaren Belege,
//! - `recordings_json`: die Quellaufnahmen zum Zeitpunkt der Erzeugung (Titel,
//!   Datum, Dauer): auch nach dem Umbenennen oder Loeschen einer Aufnahme bleibt
//!   lesbar, woraus das Protokoll entstand,
//! - `metadata_json`: Herkunft (Modell, Anbieter, Vorlage, Bloecke, Luecken).
//!
//! Fehlerfaelle und ihre Absicherung:
//! - Abbruch mitten im Vorgang (Stopp, Zeitlimit, Absturz): geschrieben wird erst
//!   am Ende, mit EINEM `INSERT` in einer Transaktion. Es gibt kein halbes
//!   Dokument (`an_insert_into_a_missing_project_writes_nothing`,
//!   `a_failed_insert_leaves_the_stored_documents_as_they_were`).
//! - Projekt waehrend des Laufs geloescht: der `INSERT` prueft den Ordner in
//!   derselben Transaktion (`folder_not_found`, nichts geschrieben).
//! - Voller Datentraeger / gesperrte Datenbank: der `INSERT` scheitert als Ganzes
//!   (`a_read_only_store_refuses_the_write`).
//! - Migration: nur `CREATE`, keine Rueckfuellung; vorhandene Zeilen bleiben
//!   byte-gleich, ein Abbruch rollt sie vollstaendig zurueck
//!   (`the_migration_keeps_every_existing_row`,
//!   `an_aborted_migration_leaves_the_old_database_untouched`); die Sicherung
//!   vor dem Schritt macht `store::backup_before_migration`.
//! - Geloeschte Aufnahme oder geloeschtes Projekt: Aufnahmen stehen nur als Text
//!   in `recordings_json` (kein Fremdschluessel); ein geloeschtes Projekt blendet
//!   seine Protokolle aus, die Zeilen bleiben (kein stiller Datenverlust).

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::notes::classify::AutoTemplateInfo;
use super::notes::model::SectionKind;
use super::store::{Meeting, MeetingStore};

/// Migration G3: nur `CREATE` (vorhandene Zeilen bleiben unveraendert). Der
/// SQL-Text steht hier, damit der Eintrag in `store::MIGRATIONS` beim
/// Zusammenfuehren mit anderen Zweigen nur aus einer Zeile besteht.
///
/// `kind`: `minutes` | `summary`. `deleted_at`: weiches Loeschen. Es gibt keinen
/// Fremdschluessel auf `meeting_folders`: wie dort ueblich prueft der Code.
pub(super) const PROJECT_MINUTES_MIGRATION: &str = "CREATE TABLE project_minutes (
      id TEXT PRIMARY KEY, folder_id TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
      body TEXT NOT NULL, sections_json TEXT NOT NULL, recordings_json TEXT NOT NULL,
      template_id TEXT, metadata_json TEXT NOT NULL,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE INDEX idx_project_minutes_folder
      ON project_minutes(folder_id, deleted_at, created_at);";

// ---------------------------------------------------------------------------
// Typen (gehen als JSON in die Spalten und als Typen an die Oberflaeche)
// ---------------------------------------------------------------------------

/// Protokoll oder Zusammenfassung.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Minutes,
    Summary,
}

impl ProjectKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Minutes => "minutes",
            ProjectKind::Summary => "summary",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "minutes" => Some(ProjectKind::Minutes),
            "summary" => Some(ProjectKind::Summary),
            _ => None,
        }
    }
}

/// Eine Aufnahme, die in das Projekt-Protokoll einging. `index` zaehlt ab 1 in
/// chronologischer Reihenfolge (`R1`, `R2` im Prompt, `A1`, `A2` im Markdown).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SourceRecording {
    pub index: u32,
    pub meeting_id: String,
    pub title: String,
    /// Startzeit der Aufnahme (Sekunden); Importe ohne Start: Anlagezeit.
    pub started_at: i64,
    pub duration_ms: Option<u64>,
    /// Zahl der ausgewerteten Transkriptsegmente.
    pub segments: u32,
}

/// Wo ein Eintrag herkommt: Aufnahme und Stelle darin. Ein Klick darauf oeffnet
/// die Aufnahme und springt zur Audiostelle (`segment_index`, `start_ms`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EntrySource {
    /// `SourceRecording::index`.
    pub recording: u32,
    pub meeting_id: String,
    pub segment_index: u32,
    pub start_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProjectEntry {
    pub text: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
    /// Gueltige Belege, chronologisch. Leer = `unsupported`.
    pub sources: Vec<EntrySource>,
    /// Das Modell hat keinen gueltigen Beleg genannt (der Eintrag bleibt, ist
    /// aber markiert; sonst waere die Belegquote geschoent).
    pub unsupported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProjectSection {
    pub id: String,
    pub title: String,
    pub kind: SectionKind,
    pub entries: Vec<ProjectEntry>,
}

/// Herkunft eines Projekt-Protokolls (wie bei Einzelprotokollen): Modell,
/// Anbieter, Vorlage, Verfahren, Luecken.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProjectMinutesMeta {
    pub model: String,
    pub provider: String,
    pub template_id: String,
    pub template_title: String,
    /// Die automatische Wahl, wenn "Automatisch" gewaehlt war.
    pub auto: Option<AutoTemplateInfo>,
    pub single_pass: bool,
    pub chunks_total: u32,
    pub chunks_split: u32,
    /// Teile des Transkripts konnten nicht ausgewertet werden.
    pub incomplete: bool,
    /// Die fehlenden Stellen (`Aufnahme 2, 03:15-07:40`).
    pub gaps: Vec<String>,
    /// Quellen-IDs des Modells, die es im Transkript nicht gab.
    pub dropped_sources: u32,
    /// Eintraege ohne gueltigen Beleg.
    pub unsupported_entries: u32,
}

/// Ein gespeichertes Projekt-Protokoll samt allem, was die Anzeige braucht.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProjectMinutes {
    pub id: String,
    pub folder_id: String,
    pub kind: ProjectKind,
    pub title: String,
    /// Das Markdown (Export, Kopieren).
    pub body: String,
    pub sections: Vec<ProjectSection>,
    pub recordings: Vec<SourceRecording>,
    pub meta: ProjectMinutesMeta,
    /// Erzeugungszeitpunkt (Sekunden).
    pub created_at: i64,
}

/// Eine Zeile der Liste im Projekt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProjectMinutesSummary {
    pub id: String,
    pub folder_id: String,
    pub kind: ProjectKind,
    pub title: String,
    pub created_at: i64,
    pub recordings: u32,
    pub template_title: String,
    pub incomplete: bool,
}

/// Was ein Lauf zum Speichern mitbringt.
#[derive(Clone, Debug)]
pub struct NewProjectMinutes {
    pub id: String,
    pub folder_id: String,
    pub kind: ProjectKind,
    pub title: String,
    pub body: String,
    pub sections: Vec<ProjectSection>,
    pub recordings: Vec<SourceRecording>,
    pub meta: ProjectMinutesMeta,
}

fn parse_kind(text: &str) -> ProjectKind {
    ProjectKind::parse(text).unwrap_or(ProjectKind::Minutes)
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
);

const COLUMNS: &str = "p.id, p.folder_id, p.kind, p.title, p.body, p.sections_json,
    p.recordings_json, p.metadata_json, p.created_at";

fn from_row(row: Row) -> Result<ProjectMinutes> {
    let (id, folder_id, kind, title, body, sections, recordings, metadata, created_at) = row;
    Ok(ProjectMinutes {
        kind: parse_kind(&kind),
        sections: serde_json::from_str(&sections)
            .map_err(|e| anyhow!("project minutes {id}: sections unreadable: {e}"))?,
        recordings: serde_json::from_str(&recordings)
            .map_err(|e| anyhow!("project minutes {id}: recordings unreadable: {e}"))?,
        meta: serde_json::from_str(&metadata)
            .map_err(|e| anyhow!("project minutes {id}: metadata unreadable: {e}"))?,
        id,
        folder_id,
        title,
        body,
        created_at,
    })
}

impl MeetingStore {
    /// Speichert ein Projekt-Protokoll: EIN `INSERT` in einer Transaktion, die den
    /// Ordner mitprueft. `folder_not_found`, wenn das Projekt fehlt oder
    /// inzwischen geloescht wurde; dann ist nichts geschrieben.
    pub fn project_minutes_insert(&self, new: &NewProjectMinutes) -> Result<()> {
        let sections = serde_json::to_string(&new.sections)?;
        let recordings = serde_json::to_string(&new.recordings)?;
        let metadata = serde_json::to_string(&new.meta)?;
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let folder_alive: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
            params![new.folder_id],
            |r| r.get(0),
        )?;
        if !folder_alive {
            return Err(anyhow!("folder_not_found"));
        }
        tx.execute(
            "INSERT INTO project_minutes
               (id, folder_id, kind, title, body, sections_json, recordings_json,
                template_id, metadata_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                new.id,
                new.folder_id,
                new.kind.as_str(),
                new.title,
                new.body,
                sections,
                recordings,
                new.meta.template_id,
                metadata,
                now
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Die Protokolle eines Projekts, das juengste zuerst. Ein geloeschtes
    /// Projekt hat keine.
    pub fn project_minutes_list(&self, folder_id: &str) -> Result<Vec<ProjectMinutesSummary>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM project_minutes p
             JOIN meeting_folders f ON f.id = p.folder_id AND f.deleted_at IS NULL
             WHERE p.folder_id = ?1 AND p.deleted_at IS NULL
             ORDER BY p.created_at DESC, p.id DESC"
        ))?;
        let rows = stmt
            .query_map(params![folder_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<Row>, _>>()?;
        // Ein unlesbarer Eintrag blendet nicht die ganze Liste aus: er faellt
        // mit einer Warnung (ohne Inhalt) weg.
        Ok(rows
            .into_iter()
            .filter_map(|row| match from_row(row) {
                Ok(m) => Some(ProjectMinutesSummary {
                    recordings: m.recordings.len() as u32,
                    template_title: m.meta.template_title.clone(),
                    incomplete: m.meta.incomplete,
                    id: m.id,
                    folder_id: m.folder_id,
                    kind: m.kind,
                    title: m.title,
                    created_at: m.created_at,
                }),
                Err(e) => {
                    log::warn!("project minutes: Eintrag nicht lesbar: {e}");
                    None
                }
            })
            .collect())
    }

    /// Ein Projekt-Protokoll; `None`, wenn es fehlt, geloescht ist oder sein
    /// Projekt geloescht wurde.
    pub fn project_minutes_get(&self, id: &str) -> Result<Option<ProjectMinutes>> {
        let conn = self.get_connection()?;
        let row: Option<Row> = conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM project_minutes p
                     JOIN meeting_folders f ON f.id = p.folder_id AND f.deleted_at IS NULL
                     WHERE p.id = ?1 AND p.deleted_at IS NULL"
                ),
                params![id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .optional()?;
        row.map(from_row).transpose()
    }

    /// Loescht ein Projekt-Protokoll (weich). `false`, wenn es nichts zu loeschen
    /// gab. Die Quellaufnahmen bleiben unberuehrt.
    pub fn project_minutes_delete(&self, id: &str) -> Result<bool> {
        let now = Utc::now().timestamp();
        let conn = self.get_connection()?;
        let changed = conn.execute(
            "UPDATE project_minutes SET deleted_at = ?1, updated_at = ?1
             WHERE id = ?2 AND deleted_at IS NULL",
            params![now, id],
        )?;
        Ok(changed > 0)
    }

    /// Lebende Besprechungen eines lebenden Projekts, chronologisch (Start, sonst
    /// Anlage). `folder_not_found`, wenn das Projekt fehlt.
    pub fn project_meetings(&self, folder_id: &str) -> Result<Vec<Meeting>> {
        let ids: Vec<String> = {
            let conn = self.get_connection()?;
            let alive: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
                params![folder_id],
                |r| r.get(0),
            )?;
            if !alive {
                return Err(anyhow!("folder_not_found"));
            }
            let mut stmt = conn.prepare(
                "SELECT m.id FROM meeting_folder_items fi
                 JOIN meetings m ON m.id = fi.meeting_id AND m.deleted_at IS NULL
                 WHERE fi.folder_id = ?1
                 ORDER BY COALESCE(m.started_at, m.created_at), m.created_at, m.id",
            )?;
            let ids = stmt
                .query_map(params![folder_id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ids
        };
        let mut meetings = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(meeting) = self.get_meeting(&id)? {
                meetings.push(meeting);
            }
        }
        Ok(meetings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};
    use crate::managers::meetings::store::MIGRATIONS;
    use rusqlite::Connection;
    use rusqlite_migration::{Migrations, M};

    fn recording(index: u32, meeting_id: &str, title: &str) -> SourceRecording {
        SourceRecording {
            index,
            meeting_id: meeting_id.to_string(),
            title: title.to_string(),
            started_at: 1_790_000_000 + i64::from(index) * 86_400,
            duration_ms: Some(1_800_000),
            segments: 12,
        }
    }

    pub(crate) fn meta() -> ProjectMinutesMeta {
        ProjectMinutesMeta {
            model: "test-model".into(),
            provider: "custom".into(),
            template_id: "builtin:allgemein".into(),
            template_title: "Allgemein".into(),
            auto: None,
            single_pass: true,
            chunks_total: 1,
            chunks_split: 0,
            incomplete: false,
            gaps: vec![],
            dropped_sources: 0,
            unsupported_entries: 0,
        }
    }

    fn new_minutes(id: &str, folder_id: &str, title: &str) -> NewProjectMinutes {
        NewProjectMinutes {
            id: id.to_string(),
            folder_id: folder_id.to_string(),
            kind: ProjectKind::Minutes,
            title: title.to_string(),
            body: format!("# {title}\n\nÄnderung für Größe & Maß 🚀\n"),
            sections: vec![ProjectSection {
                id: "zusammenfassung".into(),
                title: "Zusammenfassung".into(),
                kind: SectionKind::Text,
                entries: vec![ProjectEntry {
                    text: "Das Budget steht.".into(),
                    assignee: None,
                    due: None,
                    sources: vec![EntrySource {
                        recording: 1,
                        meeting_id: "M1".into(),
                        segment_index: 3,
                        start_ms: 65_000,
                    }],
                    unsupported: false,
                }],
            }],
            recordings: vec![recording(1, "M1", "Kick-off")],
            meta: meta(),
        }
    }

    fn folder(store: &MeetingStore, name: &str) -> String {
        store.folder_save(None, name, None).unwrap().id
    }

    #[test]
    fn a_stored_minutes_document_round_trips_with_sources_and_provenance() {
        let (_dir, store) = tmp_store();
        let f = folder(&store, "Kunde Stadtwerke");
        let new = new_minutes("PM1", &f, "Projekt-Protokoll: Kunde Stadtwerke");
        store.project_minutes_insert(&new).unwrap();

        let got = store.project_minutes_get("PM1").unwrap().unwrap();
        assert_eq!(got.id, "PM1");
        assert_eq!(got.folder_id, f);
        assert_eq!(got.kind, ProjectKind::Minutes);
        assert_eq!(got.body, new.body, "Umlaute und Emoji unveraendert");
        assert_eq!(got.sections, new.sections);
        assert_eq!(got.sections[0].entries[0].sources[0].start_ms, 65_000);
        assert_eq!(got.recordings, new.recordings);
        assert_eq!(got.meta, new.meta);
        assert!(got.created_at > 0);
        assert!(store
            .project_minutes_get("gibt-es-nicht")
            .unwrap()
            .is_none());
    }

    #[test]
    fn the_list_is_per_project_newest_first() {
        let (_dir, store) = tmp_store();
        let a = folder(&store, "A");
        let b = folder(&store, "B");
        for (id, f) in [("PM1", &a), ("PM2", &a), ("PM3", &b)] {
            store
                .project_minutes_insert(&new_minutes(id, f, id))
                .unwrap();
        }
        // Gleiche Sekunde: die ID ordnet (ULID, spaeter = groesser).
        let in_a: Vec<_> = store
            .project_minutes_list(&a)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(in_a, vec!["PM2", "PM1"]);
        let in_b = store.project_minutes_list(&b).unwrap();
        assert_eq!(in_b.len(), 1);
        assert_eq!(in_b[0].recordings, 1);
        assert_eq!(in_b[0].template_title, "Allgemein");
        assert!(store.project_minutes_list("nirgends").unwrap().is_empty());
    }

    #[test]
    fn an_insert_into_a_missing_project_writes_nothing() {
        let (_dir, store) = tmp_store();
        let gone = folder(&store, "Weg");
        store.folder_delete(&gone).unwrap();
        for target in [gone.as_str(), "gibt-es-nicht"] {
            let err = store
                .project_minutes_insert(&new_minutes("PMX", target, "x"))
                .unwrap_err();
            assert_eq!(err.to_string(), "folder_not_found");
        }
        let count: i64 = store
            .get_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM project_minutes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "weder eine Zeile noch ein Rest");
    }

    #[test]
    fn a_failed_insert_leaves_the_stored_documents_as_they_were() {
        let (_dir, store) = tmp_store();
        let f = folder(&store, "A");
        store
            .project_minutes_insert(&new_minutes("PM1", &f, "Erstes"))
            .unwrap();
        // Doppelte ID: der INSERT scheitert als Ganzes.
        assert!(store
            .project_minutes_insert(&new_minutes("PM1", &f, "Zweites"))
            .is_err());
        let list = store.project_minutes_list(&f).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Erstes");
        // Und die Datenbank nimmt danach wieder Schreibzugriffe an (keine
        // haengende Transaktion).
        store
            .project_minutes_insert(&new_minutes("PM2", &f, "Drittes"))
            .unwrap();
        assert_eq!(store.project_minutes_list(&f).unwrap().len(), 2);
    }

    #[test]
    fn deleting_hides_the_document_softly_and_only_once() {
        let (_dir, store) = tmp_store();
        let f = folder(&store, "A");
        store
            .project_minutes_insert(&new_minutes("PM1", &f, "Erstes"))
            .unwrap();
        assert!(store.project_minutes_delete("PM1").unwrap());
        assert!(!store.project_minutes_delete("PM1").unwrap());
        assert!(store.project_minutes_get("PM1").unwrap().is_none());
        assert!(store.project_minutes_list(&f).unwrap().is_empty());
        let rows: i64 = store
            .get_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM project_minutes WHERE deleted_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "die Zeile bleibt (weich geloescht)");
    }

    #[test]
    fn a_deleted_project_hides_its_documents_but_keeps_the_rows() {
        let (_dir, store) = tmp_store();
        let f = folder(&store, "A");
        store
            .project_minutes_insert(&new_minutes("PM1", &f, "Erstes"))
            .unwrap();
        store.folder_delete(&f).unwrap();
        assert!(store.project_minutes_get("PM1").unwrap().is_none());
        assert!(store.project_minutes_list(&f).unwrap().is_empty());
        let rows: i64 = store
            .get_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM project_minutes WHERE deleted_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "kein stiller Datenverlust");
    }

    #[test]
    fn an_unreadable_row_drops_out_of_the_list_but_not_the_others() {
        let (_dir, store) = tmp_store();
        let f = folder(&store, "A");
        store
            .project_minutes_insert(&new_minutes("PM1", &f, "Gut"))
            .unwrap();
        store
            .project_minutes_insert(&new_minutes("PM2", &f, "Kaputt"))
            .unwrap();
        store
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE project_minutes SET sections_json = 'kein json' WHERE id = 'PM2'",
                [],
            )
            .unwrap();
        let titles: Vec<_> = store
            .project_minutes_list(&f)
            .unwrap()
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(titles, vec!["Gut"]);
        assert!(store.project_minutes_get("PM2").is_err());
    }

    #[test]
    fn a_read_only_store_refuses_the_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&path).unwrap();
        let f = folder(&store, "A");
        drop(store);
        let ro = MeetingStore::open_read_only(&path, std::time::Duration::from_millis(50))
            .map_err(|e| format!("{e:?}"))
            .unwrap();
        assert!(ro
            .project_minutes_insert(&new_minutes("PM1", &f, "x"))
            .is_err());
        assert!(ro.project_minutes_list(&f).unwrap().is_empty());
    }

    #[test]
    fn project_meetings_are_the_live_members_in_chronological_order() {
        let (_dir, store) = tmp_store();
        let f = folder(&store, "A");
        let late = ready_meeting(&store, "Spaet", 2_000);
        let early = ready_meeting(&store, "Frueh", 1_000);
        let gone = ready_meeting(&store, "Weg", 1_500);
        let outside = ready_meeting(&store, "Draussen", 500);
        for m in [&late, &early, &gone] {
            store.set_meeting_folders(&m.id, &[f.clone()]).unwrap();
        }
        store.soft_delete_meeting(&gone.id).unwrap();
        let _ = outside;
        let titles: Vec<_> = store
            .project_meetings(&f)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect();
        assert_eq!(titles, vec!["Frueh", "Spaet"]);
        assert_eq!(
            store
                .project_meetings("gibt-es-nicht")
                .unwrap_err()
                .to_string(),
            "folder_not_found"
        );
    }

    // -- Migration -------------------------------------------------------------

    /// G3 ist der Schritt mit Index 8; spaetere Schritte (G5 = Index 9) haengen dahinter.
    const G3_INDEX: usize = 8;

    /// Der Stand vor diesem Schritt: alle Migrationen vor Index 8, mit Altdaten in den
    /// Tabellen, an die der Schritt grenzt.
    fn db_before_the_step(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("meetings.db");
        let mut conn = Connection::open(&path).unwrap();
        let before = G3_INDEX;
        Migrations::new(MIGRATIONS[..before].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
               VALUES ('L1', 'Wochenbesprechung', 'ready', 'live', 1790000000, 1790003600);
             INSERT INTO meeting_documents (id, meeting_id, kind, body_format, body, created_at, updated_at)
               VALUES ('D1', 'L1', 'minutes', 'markdown@1', 'Protokoll mit Größe', 1790003700, 1790003700);
             INSERT INTO meeting_folders (id, name, sort, created_at, updated_at)
               VALUES ('F1', 'Kunden', 1, 1790000000, 1790000000);
             INSERT INTO meeting_folder_items (folder_id, meeting_id, added_at)
               VALUES ('F1', 'L1', 1790000100);",
        )
        .unwrap();
        path
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    fn scalar(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn the_migration_keeps_every_existing_row() {
        assert!(MIGRATIONS.len() > G3_INDEX, "G3 ist Index 8");
        let dir = tempfile::tempdir().unwrap();
        let path = db_before_the_step(&dir);
        {
            let conn = Connection::open(&path).unwrap();
            assert_eq!(user_version(&conn), G3_INDEX as i64);
            assert_eq!(
                scalar(
                    &conn,
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = 'project_minutes'"
                ),
                0,
                "vor dem Schritt gibt es die Tabelle nicht"
            );
        }
        let store = MeetingStore::open_at(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(
            scalar(&conn, "SELECT COUNT(*) FROM project_minutes"),
            0,
            "die neue Tabelle ist leer: keine Rueckfuellung"
        );
        let title: String = conn
            .query_row("SELECT title FROM meetings WHERE id = 'L1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let body: String = conn
            .query_row(
                "SELECT body FROM meeting_documents WHERE id = 'D1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("Wochenbesprechung", "Protokoll mit Größe")
        );
        assert_eq!(
            store.meeting_folder_ids("L1").unwrap(),
            vec!["F1".to_string()],
            "Projekte der Altdaten unveraendert"
        );
        // Und das Neue funktioniert gegen die Altdaten.
        store
            .project_minutes_insert(&new_minutes("PM1", "F1", "Neu"))
            .unwrap();
        assert_eq!(store.project_minutes_list("F1").unwrap().len(), 1);
        // Ein zweites Oeffnen aendert nichts (idempotent, kein erneuter Schritt).
        drop(store);
        let again = MeetingStore::open_at(&path).unwrap();
        assert_eq!(again.project_minutes_list("F1").unwrap().len(), 1);
    }

    #[test]
    fn opening_the_old_database_leaves_a_backup_of_the_old_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_before_the_step(&dir);
        let before = G3_INDEX as i64;
        let _store = MeetingStore::open_at(&path).unwrap();
        let backup = dir.path().join(format!("meetings.db.bak-v{before}"));
        assert!(backup.is_file(), "Sicherung des Standes vor dem Schritt");
        let conn = Connection::open(&backup).unwrap();
        assert_eq!(user_version(&conn), before);
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'project_minutes'"
            ),
            0
        );
    }

    #[test]
    fn an_aborted_migration_leaves_the_old_database_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_before_the_step(&dir);
        let before = G3_INDEX;
        // Der Schritt bricht an seinem Ende ab: alles davor rollt mit zurueck.
        let broken_sql: &'static str = Box::leak(
            format!("{PROJECT_MINUTES_MIGRATION}\nINSERT INTO gibt_es_nicht VALUES (1);")
                .into_boxed_str(),
        );
        let mut broken = MIGRATIONS[..before].to_vec();
        broken.push(M::up(broken_sql));
        let mut conn = Connection::open(&path).unwrap();
        assert!(Migrations::new(broken).to_latest(&mut conn).is_err());
        assert_eq!(user_version(&conn), before as i64, "Version unveraendert");
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%project_minutes%'"
            ),
            0,
            "weder Tabelle noch Index blieben zurueck"
        );
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meetings"), 1);
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meeting_documents"), 1);
    }
}
