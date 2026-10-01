//! D1: Ablage der Folien (Tabelle `meeting_slides`).
//!
//! Folien sind KEIN Fassungsobjekt des Transkripts (Fassungen schliessen sich
//! gegenseitig aus, Folien kommen hinzu), sondern ein Anhang der Besprechung. Eine
//! Zeile ist eine Folie mit allen Zeitbereichen, in denen sie im Video zu sehen
//! ist (`occurrences_json`: ein Ruecksprung ergibt mehrere Eintraege, keine neue
//! Folie). Die Bilder liegen als Dateien im Besprechungsordner (`slides/0007.jpg`,
//! Pfade RELATIV dazu); die Zeile entsteht erst NACH den Dateien.
//!
//! Fehlerfaelle und Absicherung:
//! - Migration: nur `CREATE ... IF NOT EXISTS`, keine Rueckfuellung; Altdaten
//!   bleiben unberuehrt, ein Abbruch rollt vollstaendig zurueck, vor dem Schritt
//!   liegt die Sicherung `meetings.db.bak-v<n>` (`store::backup_before_migration`).
//! - Besprechung geloescht (auch waehrend eines Laufs): `slide_insert` prueft sie in
//!   derselben Transaktion (`meeting_not_found`); `soft_delete_meeting` blendet die
//!   Folien mit aus, die Dateien gehen mit dem Besprechungsordner.
//! - Zwei Schreiber: jede Aenderung in einer IMMEDIATE-Transaktion; die Nummer wird
//!   darin vergeben, eine belegte Nummer wird abgelehnt (`slide_number_taken`).

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;
use ulid::Ulid;

use super::SlideOccurrence;
use crate::managers::meetings::store::MeetingStore;

/// Migration D1: nur `CREATE` (vorhandene Zeilen bleiben unveraendert), jedes
/// `IF NOT EXISTS`: wer die Tabelle schon hat (Entwicklungsstand mit anderer
/// Schrittnummer, nach dem Zusammenfuehren mit Nachbarzweigen), scheitert nicht.
/// Der SQL-Text steht hier, damit der Eintrag in `store::MIGRATIONS` nur aus einer
/// Zeile besteht.
///
/// `dhash`: 64 Bit als `INTEGER` (vorzeichenbehaftet, bitgleich), `occurrences_json`:
/// `[{"start_ms":..,"end_ms":..}]`, `origin`: `video` | `image`, `kind`: `text` |
/// `ohne_text` | Modellklasse (D2/D3), `hidden`: der Nutzer blendet aus.
pub const SLIDES_MIGRATION: &str = "CREATE TABLE IF NOT EXISTS meeting_slides (
      id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, number INTEGER NOT NULL,
      origin TEXT NOT NULL CHECK (origin IN ('video','image')),
      image_path TEXT NOT NULL, thumb_path TEXT,
      dhash INTEGER NOT NULL,
      occurrences_json TEXT NOT NULL DEFAULT '[]',
      ocr_text TEXT, ocr_engine TEXT,
      kind TEXT,
      description TEXT, description_model TEXT,
      hidden INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
    CREATE INDEX IF NOT EXISTS idx_slides_meeting ON meeting_slides(meeting_id, number);";

pub const ORIGIN_VIDEO: &str = "video";
/// Einzelbilder als Quelle (D6).
#[allow(dead_code)]
pub const ORIGIN_IMAGE: &str = "image";

/// Eine Folie, wie die Oberflaeche sie liest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MeetingSlide {
    pub id: String,
    pub meeting_id: String,
    /// 1..n in der Reihenfolge der Anlage (bei der ersten Erkennung: nach dem
    /// ersten Auftreten im Video).
    pub number: u32,
    /// `video` | `image`.
    pub origin: String,
    /// Relativ zum Besprechungsordner: `slides/0007.jpg`.
    pub image_path: String,
    pub thumb_path: Option<String>,
    /// Wo im Video die Folie zu sehen ist; Ruecksprung = mehrere Bereiche.
    pub occurrences: Vec<SlideOccurrence>,
    pub ocr_text: Option<String>,
    pub ocr_engine: Option<String>,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub description_model: Option<String>,
    /// Vom Nutzer ausgeblendet (Sprecherbild, Dublette).
    pub hidden: bool,
}

/// Eine Zeile samt dem Hash, den die Oberflaeche nicht braucht (der Abgleich bei
/// einer Wiederholung des Laufs schon).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideRecord {
    pub slide: MeetingSlide,
    pub dhash: u64,
}

/// Was der Lauf zum Anlegen einer Folie mitbringt.
#[derive(Clone, Debug)]
pub struct NewSlide {
    pub meeting_id: String,
    /// `None`: die naechste freie Nummer (in der Transaktion vergeben).
    pub number: Option<u32>,
    pub origin: &'static str,
    pub image_path: String,
    pub thumb_path: Option<String>,
    pub dhash: u64,
    pub occurrences: Vec<SlideOccurrence>,
}

const COLUMNS: &str = "id, meeting_id, number, origin, image_path, thumb_path, dhash,
    occurrences_json, ocr_text, ocr_engine, kind, description, description_model, hidden";

fn from_row(row: &Row<'_>) -> rusqlite::Result<SlideRecord> {
    let occurrences_json: String = row.get(7)?;
    // Eine unlesbare Spalte blendet nicht die ganze Folie aus: leere Liste.
    let occurrences = serde_json::from_str(&occurrences_json).unwrap_or_default();
    Ok(SlideRecord {
        slide: MeetingSlide {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            number: row.get::<_, i64>(2)?.max(0) as u32,
            origin: row.get(3)?,
            image_path: row.get(4)?,
            thumb_path: row.get(5)?,
            occurrences,
            ocr_text: row.get(8)?,
            ocr_engine: row.get(9)?,
            kind: row.get(10)?,
            description: row.get(11)?,
            description_model: row.get(12)?,
            hidden: row.get::<_, i64>(13)? != 0,
        },
        dhash: row.get::<_, i64>(6)? as u64,
    })
}

impl MeetingStore {
    /// Die naechste freie Nummer der Besprechung (hoechste vergebene + 1, auch
    /// geloeschte zaehlen mit: eine Nummer kehrt nie wieder).
    pub fn slide_next_number(&self, meeting_id: &str) -> Result<u32> {
        let conn = self.get_connection()?;
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(number), 0) FROM meeting_slides WHERE meeting_id = ?1",
            params![meeting_id],
            |r| r.get(0),
        )?;
        Ok(max as u32 + 1)
    }

    /// Legt eine Folie an: EIN `INSERT` in einer Transaktion, die die Besprechung
    /// mitprueft. `meeting_not_found`, wenn sie fehlt oder inzwischen geloescht
    /// wurde; `slide_number_taken`, wenn die verlangte Nummer schon vergeben ist.
    pub fn slide_insert(&self, new: &NewSlide) -> Result<MeetingSlide> {
        let occurrences = serde_json::to_string(&new.occurrences)?;
        let id = Ulid::new().to_string();
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let alive: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1 AND deleted_at IS NULL)",
            params![new.meeting_id],
            |r| r.get(0),
        )?;
        if !alive {
            return Err(anyhow!("meeting_not_found"));
        }
        let max: i64 = tx.query_row(
            "SELECT COALESCE(MAX(number), 0) FROM meeting_slides WHERE meeting_id = ?1",
            params![new.meeting_id],
            |r| r.get(0),
        )?;
        let number = match new.number {
            Some(n) => {
                let taken: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM meeting_slides WHERE meeting_id = ?1 AND number = ?2)",
                    params![new.meeting_id, i64::from(n)],
                    |r| r.get(0),
                )?;
                if taken {
                    return Err(anyhow!("slide_number_taken"));
                }
                n
            }
            None => max as u32 + 1,
        };
        tx.execute(
            "INSERT INTO meeting_slides
               (id, meeting_id, number, origin, image_path, thumb_path, dhash,
                occurrences_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                id,
                new.meeting_id,
                i64::from(number),
                new.origin,
                new.image_path,
                new.thumb_path,
                new.dhash as i64,
                occurrences,
                now
            ],
        )?;
        tx.commit()?;
        Ok(MeetingSlide {
            id,
            meeting_id: new.meeting_id.clone(),
            number,
            origin: new.origin.to_string(),
            image_path: new.image_path.clone(),
            thumb_path: new.thumb_path.clone(),
            occurrences: new.occurrences.clone(),
            ocr_text: None,
            ocr_engine: None,
            kind: None,
            description: None,
            description_model: None,
            hidden: false,
        })
    }

    /// Die Folien einer Besprechung samt Hash, nach Nummer.
    pub fn slide_records(&self, meeting_id: &str) -> Result<Vec<SlideRecord>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM meeting_slides
             WHERE meeting_id = ?1 AND deleted_at IS NULL ORDER BY number, id"
        ))?;
        let rows = stmt
            .query_map(params![meeting_id], from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Die Folien einer Besprechung, nach Nummer (auch ausgeblendete: die
    /// Oberflaeche filtert).
    pub fn slides_list(&self, meeting_id: &str) -> Result<Vec<MeetingSlide>> {
        Ok(self
            .slide_records(meeting_id)?
            .into_iter()
            .map(|r| r.slide)
            .collect())
    }

    /// Eine Folie; `None`, wenn es sie nicht gibt oder sie geloescht ist.
    pub fn slide_get(&self, slide_id: &str) -> Result<Option<MeetingSlide>> {
        let conn = self.get_connection()?;
        let record = conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM meeting_slides WHERE id = ?1 AND deleted_at IS NULL"
                ),
                params![slide_id],
                from_row,
            )
            .optional()?;
        Ok(record.map(|r| r.slide))
    }

    fn slide_update(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<bool> {
        let conn = self.get_connection()?;
        Ok(conn.execute(sql, args)? > 0)
    }

    /// Blendet eine Folie aus oder ein. `false`, wenn es sie nicht gibt.
    pub fn slide_set_hidden(&self, slide_id: &str, hidden: bool) -> Result<bool> {
        let now = Utc::now().timestamp();
        self.slide_update(
            "UPDATE meeting_slides SET hidden = ?1, updated_at = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            &[&i64::from(hidden), &now, &slide_id],
        )
    }

    /// Ersetzt die Zeitbereiche einer Folie (nach dem Zusammenfuehren bei einer Wiederholung).
    pub fn slide_set_occurrences(
        &self,
        slide_id: &str,
        occurrences: &[SlideOccurrence],
    ) -> Result<bool> {
        let json = serde_json::to_string(occurrences)?;
        let now = Utc::now().timestamp();
        self.slide_update(
            "UPDATE meeting_slides SET occurrences_json = ?1, updated_at = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            &[&json, &now, &slide_id],
        )
    }

    /// Folientext (Schnittstelle fuer D2): Text, Engine und Art (`text` | `ohne_text`).
    pub fn slide_set_text(
        &self,
        slide_id: &str,
        ocr_text: Option<&str>,
        ocr_engine: Option<&str>,
        kind: Option<&str>,
    ) -> Result<bool> {
        let now = Utc::now().timestamp();
        self.slide_update(
            "UPDATE meeting_slides SET ocr_text = ?1, ocr_engine = ?2, kind = ?3, updated_at = ?4
             WHERE id = ?5 AND deleted_at IS NULL",
            &[&ocr_text, &ocr_engine, &kind, &now, &slide_id],
        )
    }

    /// Bildbeschreibung (Schnittstelle fuer D3): Text und Modell.
    pub fn slide_set_description(
        &self,
        slide_id: &str,
        description: Option<&str>,
        model: Option<&str>,
    ) -> Result<bool> {
        let now = Utc::now().timestamp();
        self.slide_update(
            "UPDATE meeting_slides SET description = ?1, description_model = ?2, updated_at = ?3
             WHERE id = ?4 AND deleted_at IS NULL",
            &[&description, &model, &now, &slide_id],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus, MIGRATIONS};
    use rusqlite::Connection;
    use rusqlite_migration::{Migrations, M};

    /// Der Index von [`SLIDES_MIGRATION`] in `MIGRATIONS`. Nach dem Zusammenfuehren
    /// mit anderen Zweigen kann er sich verschieben: dann schlaegt
    /// `the_slides_migration_sits_at_its_index` mit einem klaren Hinweis fehl.
    const STEP: usize = 9;

    struct Fx {
        _dir: tempfile::TempDir,
        store: MeetingStore,
        meeting: String,
    }

    fn fixture() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("Vortrag", MeetingSource::Import, None)
            .unwrap()
            .id;
        store.set_status(&meeting, MeetingStatus::Ready).unwrap();
        Fx {
            _dir: dir,
            store,
            meeting,
        }
    }

    fn occ(start: u64, end: u64) -> SlideOccurrence {
        SlideOccurrence {
            start_ms: start,
            end_ms: end,
        }
    }

    fn new_slide(f: &Fx, hash: u64) -> NewSlide {
        NewSlide {
            meeting_id: f.meeting.clone(),
            number: None,
            origin: ORIGIN_VIDEO,
            image_path: "slides/0001.jpg".into(),
            thumb_path: Some("slides/0001_t.jpg".into()),
            dhash: hash,
            occurrences: vec![occ(0, 10_000), occ(30_000, 40_000)],
        }
    }

    #[test]
    fn a_slide_round_trips_with_a_full_width_hash_and_occurrences() {
        let f = fixture();
        // Das hoechste Bit gesetzt: als i64 negativ, als u64 wieder bitgleich.
        let hash = 0xF234_5678_9ABC_DEF0u64;
        let made = f.store.slide_insert(&new_slide(&f, hash)).unwrap();
        assert_eq!(made.number, 1);
        assert!(!made.hidden);
        let records = f.store.slide_records(&f.meeting).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].dhash, hash);
        assert_eq!(records[0].slide, made);
        assert_eq!(
            records[0].slide.occurrences,
            vec![occ(0, 10_000), occ(30_000, 40_000)]
        );
        assert_eq!(f.store.slide_get(&made.id).unwrap().unwrap(), made);
        assert!(f.store.slide_get("gibt-es-nicht").unwrap().is_none());
    }

    #[test]
    fn numbers_count_up_in_the_order_of_creation_and_never_come_back() {
        let f = fixture();
        let a = f.store.slide_insert(&new_slide(&f, 1)).unwrap();
        let b = f.store.slide_insert(&new_slide(&f, 2)).unwrap();
        assert_eq!((a.number, b.number), (1, 2));
        assert_eq!(f.store.slide_next_number(&f.meeting).unwrap(), 3);
        // Eine verlangte Nummer, die es schon gibt, wird abgelehnt (zwei Schreiber).
        let mut dup = new_slide(&f, 3);
        dup.number = Some(2);
        assert_eq!(
            f.store.slide_insert(&dup).unwrap_err().to_string(),
            "slide_number_taken"
        );
        dup.number = Some(3);
        assert_eq!(f.store.slide_insert(&dup).unwrap().number, 3);
        // Eine andere Besprechung zaehlt fuer sich.
        let other = f
            .store
            .create_meeting("Zweite", MeetingSource::Import, None)
            .unwrap()
            .id;
        let mut other_slide = new_slide(&f, 4);
        other_slide.meeting_id = other.clone();
        assert_eq!(f.store.slide_insert(&other_slide).unwrap().number, 1);
        let listed: Vec<u32> = f
            .store
            .slides_list(&f.meeting)
            .unwrap()
            .into_iter()
            .map(|s| s.number)
            .collect();
        assert_eq!(listed, vec![1, 2, 3]);
    }

    #[test]
    fn an_insert_into_a_missing_or_deleted_meeting_writes_nothing() {
        let f = fixture();
        let mut gone = new_slide(&f, 1);
        gone.meeting_id = "gibt-es-nicht".into();
        assert_eq!(
            f.store.slide_insert(&gone).unwrap_err().to_string(),
            "meeting_not_found"
        );
        f.store.soft_delete_meeting(&f.meeting).unwrap();
        assert_eq!(
            f.store
                .slide_insert(&new_slide(&f, 1))
                .unwrap_err()
                .to_string(),
            "meeting_not_found"
        );
        let count: i64 = f
            .store
            .get_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM meeting_slides", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "keine halbe Zeile");
    }

    #[test]
    fn deleting_the_meeting_hides_its_slides() {
        let f = fixture();
        let made = f.store.slide_insert(&new_slide(&f, 1)).unwrap();
        f.store.soft_delete_meeting(&f.meeting).unwrap();
        assert!(f.store.slides_list(&f.meeting).unwrap().is_empty());
        assert!(f.store.slide_get(&made.id).unwrap().is_none());
        assert!(
            !f.store.slide_set_hidden(&made.id, true).unwrap(),
            "nichts zu aendern"
        );
    }

    #[test]
    fn hiding_text_and_description_are_stored_without_touching_each_other() {
        let f = fixture();
        let made = f.store.slide_insert(&new_slide(&f, 1)).unwrap();
        assert!(f.store.slide_set_hidden(&made.id, true).unwrap());
        assert!(f
            .store
            .slide_set_text(
                &made.id,
                Some("Größe & Maße: 70 €"),
                Some("windows-ocr"),
                Some("text")
            )
            .unwrap());
        assert!(f
            .store
            .slide_set_description(&made.id, Some("Balkendiagramm"), Some("gemma-4-e4b"))
            .unwrap());
        assert!(f
            .store
            .slide_set_occurrences(&made.id, &[occ(5, 6)])
            .unwrap());
        let got = f.store.slide_get(&made.id).unwrap().unwrap();
        assert!(got.hidden);
        assert_eq!(got.ocr_text.as_deref(), Some("Größe & Maße: 70 €"));
        assert_eq!(got.ocr_engine.as_deref(), Some("windows-ocr"));
        assert_eq!(got.kind.as_deref(), Some("text"));
        assert_eq!(got.description.as_deref(), Some("Balkendiagramm"));
        assert_eq!(got.description_model.as_deref(), Some("gemma-4-e4b"));
        assert_eq!(got.occurrences, vec![occ(5, 6)]);
        assert!(f.store.slide_set_hidden(&made.id, false).unwrap());
        assert!(!f.store.slide_get(&made.id).unwrap().unwrap().hidden);
        assert!(!f.store.slide_set_hidden("gibt-es-nicht", true).unwrap());
    }

    #[test]
    fn an_unreadable_occurrences_column_does_not_hide_the_slide() {
        let f = fixture();
        let made = f.store.slide_insert(&new_slide(&f, 1)).unwrap();
        f.store
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE meeting_slides SET occurrences_json = 'kaputt' WHERE id = ?1",
                params![made.id],
            )
            .unwrap();
        let got = f.store.slide_get(&made.id).unwrap().unwrap();
        assert!(got.occurrences.is_empty());
        assert_eq!(got.image_path, "slides/0001.jpg");
    }

    // -- Migration -------------------------------------------------------------

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    fn scalar(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Der Stand vor diesem Schritt: alle Migrationen davor, mit Altdaten in den
    /// Tabellen, an die der Schritt grenzt.
    fn db_before_the_step(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("meetings.db");
        let mut conn = Connection::open(&path).unwrap();
        Migrations::new(MIGRATIONS[..STEP].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO meetings (id, title, status, source, source_path, created_at, updated_at)
               VALUES ('L1', 'Wochenbesprechung', 'ready', 'import', 'C:/Aufnahmen/Vortrag.mp4', 1790000000, 1790003600);
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

    #[test]
    fn the_slides_migration_sits_at_its_index() {
        // Schlaegt dies fehl, hat sich beim Zusammenfuehren die Reihenfolge der
        // Migrationen geaendert: `STEP` oben auf den neuen Index setzen.
        assert!(
            MIGRATIONS.len() > STEP,
            "es gibt Migrationen hinter Index {STEP}"
        );
        let dir = tempfile::tempdir().unwrap();
        let before = dir.path().join("a.db");
        let mut conn = Connection::open(&before).unwrap();
        Migrations::new(MIGRATIONS[..STEP].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        let has = |conn: &Connection| {
            scalar(
                conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'meeting_slides'",
            )
        };
        assert_eq!(has(&conn), 0, "vor dem Schritt gibt es die Tabelle nicht");
        let after = dir.path().join("b.db");
        let mut conn2 = Connection::open(&after).unwrap();
        Migrations::new(MIGRATIONS[..=STEP].to_vec())
            .to_latest(&mut conn2)
            .unwrap();
        assert_eq!(has(&conn2), 1, "mit dem Schritt gibt es sie");
    }

    #[test]
    fn the_migration_keeps_every_existing_row_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_before_the_step(&dir);
        {
            let conn = Connection::open(&path).unwrap();
            assert_eq!(user_version(&conn), STEP as i64);
        }
        let store = MeetingStore::open_at(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
        assert_eq!(
            scalar(&conn, "SELECT COUNT(*) FROM meeting_slides"),
            0,
            "keine Rueckfuellung"
        );
        let (title, body): (String, String) = (
            conn.query_row("SELECT title FROM meetings WHERE id = 'L1'", [], |r| {
                r.get(0)
            })
            .unwrap(),
            conn.query_row(
                "SELECT body FROM meeting_documents WHERE id = 'D1'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
        );
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("Wochenbesprechung", "Protokoll mit Größe")
        );
        assert_eq!(
            store.meeting_folder_ids("L1").unwrap(),
            vec!["F1".to_string()]
        );

        // Das Neue arbeitet gegen die Altdaten.
        let made = store
            .slide_insert(&NewSlide {
                meeting_id: "L1".into(),
                number: None,
                origin: ORIGIN_VIDEO,
                image_path: "slides/0001.jpg".into(),
                thumb_path: None,
                dhash: 42,
                occurrences: vec![occ(0, 1_000)],
            })
            .unwrap();
        // Ein zweites Oeffnen aendert nichts (kein erneuter Schritt).
        drop(store);
        let again = MeetingStore::open_at(&path).unwrap();
        assert_eq!(again.slides_list("L1").unwrap(), vec![made.clone()]);
        // Der SQL-Text selbst ist wiederholbar (Entwicklungsstand mit anderer Schrittnummer).
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SLIDES_MIGRATION).unwrap();
        conn.execute_batch(SLIDES_MIGRATION).unwrap();
        assert_eq!(again.slides_list("L1").unwrap(), vec![made]);
    }

    #[test]
    fn opening_the_old_database_leaves_a_backup_of_the_old_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_before_the_step(&dir);
        let _store = MeetingStore::open_at(&path).unwrap();
        let backup = dir.path().join(format!("meetings.db.bak-v{STEP}"));
        assert!(backup.is_file(), "Sicherung des Standes vor dem Schritt");
        let conn = Connection::open(&backup).unwrap();
        assert_eq!(user_version(&conn), STEP as i64);
        assert_eq!(
            scalar(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'meeting_slides'"
            ),
            0
        );
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meetings"), 1);
    }

    #[test]
    fn an_aborted_migration_leaves_the_old_database_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_before_the_step(&dir);
        // Der Schritt bricht an seinem Ende ab: alles davor rollt mit zurueck.
        let broken_sql: &'static str = Box::leak(
            format!("{SLIDES_MIGRATION}\nINSERT INTO gibt_es_nicht VALUES (1);").into_boxed_str(),
        );
        let mut broken = MIGRATIONS[..STEP].to_vec();
        broken.push(M::up(broken_sql));
        let mut conn = Connection::open(&path).unwrap();
        assert!(Migrations::new(broken).to_latest(&mut conn).is_err());
        assert_eq!(user_version(&conn), STEP as i64, "Version unveraendert");
        assert_eq!(
            scalar(&conn, "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%meeting_slides%' OR name LIKE '%idx_slides%'"),
            0,
            "weder Tabelle noch Index blieben zurueck"
        );
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meetings"), 1);
        assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM meeting_documents"), 1);
    }
}
