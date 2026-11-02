//! Besprechungsordner <-> Suite-Projekt (ADR-0026 PR-7, Epic wai-sdk#1047).
//!
//! Ein Besprechungsordner ("Projekt" der Aufnahmen-Oberflaeche, `meeting_folders`) kann auf ein
//! Suite-Projekt zeigen: die nullable Spalte `projekt_id`. Die Spalte ist ein reiner Verweis; das
//! Suite-Projekt selbst liegt in `wai-projekte` (eigene Datei `projekte.db`, siehe `crate::projekte`).
//! Bestehende Ordner bleiben unveraendert (`NULL`).
//!
//! Die Migration ist additiv (`ADD COLUMN` ohne Vorgabe, plus ein Index) und folgt dem Muster der
//! uebrigen Schritte: EINE Transaktion, ein Abbruch rollt sie ganz zurueck. Der SQL-Text steht hier,
//! damit der Eintrag in `store.rs::MIGRATIONS` nur aus einer Zeile besteht.
//!
//! Fehlercodes (Text): `folder_not_found`, `projekt_invalid`.

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::params;

use super::store::MeetingStore;

/// Migration Index 13 (ADR-0026 PR-7): `meeting_folders.projekt_id`, nullable, mit Index.
pub const PROJEKT_MIGRATION: &str = "
    ALTER TABLE meeting_folders ADD COLUMN projekt_id TEXT;
    CREATE INDEX IF NOT EXISTS idx_meeting_folders_projekt
        ON meeting_folders(projekt_id) WHERE projekt_id IS NOT NULL;";

/// Hoechstlaenge einer Suite-Projekt-Kennung (`prj_` plus ULID sind 30 Zeichen).
const MAX_PROJEKT_ID: usize = 64;

/// Besprechungsordner mit dem Verweis auf sein Suite-Projekt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrdnerVerweis {
    pub id: String,
    pub name: String,
    pub projekt_id: Option<String>,
}

fn projekt_id_gueltig(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_PROJEKT_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl MeetingStore {
    /// Lebende Ordner samt Verweis auf ihr Suite-Projekt.
    pub fn ordner_verweise(&self) -> Result<Vec<OrdnerVerweis>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, projekt_id FROM meeting_folders WHERE deleted_at IS NULL
             ORDER BY sort, name COLLATE NOCASE, id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(OrdnerVerweis {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    projekt_id: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Setzt (`Some`) oder loest (`None`) den Verweis eines Ordners auf sein Suite-Projekt.
    /// `folder_not_found` bei unbekanntem oder geloeschtem Ordner, `projekt_invalid` bei einer
    /// Kennung mit unzulaessigen Zeichen.
    pub fn ordner_projekt_setzen(&self, ordner_id: &str, projekt_id: Option<&str>) -> Result<()> {
        if projekt_id.is_some_and(|p| !projekt_id_gueltig(p)) {
            return Err(anyhow!("projekt_invalid"));
        }
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let changed = tx.execute(
            "UPDATE meeting_folders SET projekt_id = ?1, updated_at = ?2
             WHERE id = ?3 AND deleted_at IS NULL",
            params![projekt_id, Utc::now().timestamp(), ordner_id],
        )?;
        if changed == 0 {
            return Err(anyhow!("folder_not_found"));
        }
        tx.commit()?;
        Ok(())
    }

    /// Loest den Verweis aller Ordner auf `projekt_id` (das Suite-Projekt wurde geloescht).
    /// Liefert die Zahl der geloesten Ordner. Die Besprechungen bleiben unberuehrt.
    pub fn ordner_projekt_loesen_alle(&self, projekt_id: &str) -> Result<u32> {
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let n = tx.execute(
            "UPDATE meeting_folders SET projekt_id = NULL, updated_at = ?1
             WHERE projekt_id = ?2",
            params![Utc::now().timestamp(), projekt_id],
        )?;
        tx.commit()?;
        Ok(n as u32)
    }

    /// Lebende Besprechungen (Kennung, Titel); geloeschte (Papierkorb) zaehlen nicht.
    pub fn besprechungen_kurz(&self) -> Result<Vec<(String, String)>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, title FROM meetings WHERE deleted_at IS NULL ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Kennungen der lebenden Besprechungen in einem Ordner.
    pub fn besprechungen_im_ordner(&self, ordner_id: &str) -> Result<Vec<String>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT fi.meeting_id FROM meeting_folder_items fi
             JOIN meetings m ON m.id = fi.meeting_id AND m.deleted_at IS NULL
             WHERE fi.folder_id = ?1 ORDER BY m.created_at, m.id",
        )?;
        let rows = stmt
            .query_map(params![ordner_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Suite-Projekte, auf die die lebenden Ordner dieser Besprechung zeigen (ohne Doppelte).
    /// Das Chat-Werkzeug "recherche" waehlt daraus das Projekt; mehrere Treffer sind mehrdeutig.
    pub fn projekte_der_besprechung(&self, meeting_id: &str) -> Result<Vec<String>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT f.projekt_id FROM meeting_folder_items fi
             JOIN meeting_folders f ON f.id = fi.folder_id
              AND f.deleted_at IS NULL AND f.projekt_id IS NOT NULL
             WHERE fi.meeting_id = ?1 ORDER BY f.projekt_id",
        )?;
        let rows = stmt
            .query_map(params![meeting_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};

    fn ordner(s: &MeetingStore, name: &str) -> String {
        s.folder_save(None, name, None).unwrap().id
    }

    #[test]
    fn die_spalte_ist_nullable_und_bestehende_ordner_bleiben_ohne_verweis() {
        let (_dir, s) = tmp_store();
        let f = ordner(&s, "Waermewende");
        let liste = s.ordner_verweise().unwrap();
        assert_eq!(liste.len(), 1);
        assert_eq!(liste[0].id, f);
        assert_eq!(liste[0].projekt_id, None);
    }

    #[test]
    fn verweis_setzen_loesen_und_unbekannter_ordner() {
        let (_dir, s) = tmp_store();
        let f = ordner(&s, "A");
        s.ordner_projekt_setzen(&f, Some("prj_01HZZZZZZZZZZZZZZZZZZZZZZZ"))
            .unwrap();
        assert_eq!(
            s.ordner_verweise().unwrap()[0].projekt_id.as_deref(),
            Some("prj_01HZZZZZZZZZZZZZZZZZZZZZZZ")
        );
        s.ordner_projekt_setzen(&f, None).unwrap();
        assert_eq!(s.ordner_verweise().unwrap()[0].projekt_id, None);
        let err = s.ordner_projekt_setzen("gibt-es-nicht", Some("prj_x")).unwrap_err();
        assert_eq!(err.to_string(), "folder_not_found");
        let err = s.ordner_projekt_setzen(&f, Some("prj x/..")).unwrap_err();
        assert_eq!(err.to_string(), "projekt_invalid");
    }

    #[test]
    fn die_ordnerliste_der_oberflaeche_traegt_den_verweis() {
        let (_dir, s) = tmp_store();
        let f = ordner(&s, "A");
        assert_eq!(s.folders_list().unwrap()[0].projekt_id, None);
        s.ordner_projekt_setzen(&f, Some("prj_abc")).unwrap();
        assert_eq!(
            s.folders_list().unwrap()[0].projekt_id.as_deref(),
            Some("prj_abc")
        );
    }

    #[test]
    fn besprechungen_im_ordner_und_projekt_der_besprechung() {
        let (_dir, s) = tmp_store();
        let m1 = ready_meeting(&s, "Eins", 1_750_000_000).id;
        let m2 = ready_meeting(&s, "Zwei", 1_750_000_100).id;
        let a = ordner(&s, "A");
        let b = ordner(&s, "B");
        s.set_meeting_folders(&m1, &[a.clone(), b.clone()]).unwrap();
        s.set_meeting_folders(&m2, std::slice::from_ref(&b)).unwrap();
        s.ordner_projekt_setzen(&a, Some("prj_a")).unwrap();
        s.ordner_projekt_setzen(&b, Some("prj_b")).unwrap();
        assert_eq!(s.besprechungen_im_ordner(&a).unwrap(), vec![m1.clone()]);
        assert_eq!(s.besprechungen_im_ordner(&b).unwrap().len(), 2);
        assert_eq!(
            s.projekte_der_besprechung(&m1).unwrap(),
            vec!["prj_a".to_string(), "prj_b".to_string()],
            "zwei Projekte sind mehrdeutig"
        );
        assert_eq!(
            s.projekte_der_besprechung(&m2).unwrap(),
            vec!["prj_b".to_string()]
        );
    }

    #[test]
    fn geloeschte_besprechungen_zaehlen_nicht_und_loesen_alle_trifft_nur_das_projekt() {
        let (_dir, s) = tmp_store();
        let m1 = ready_meeting(&s, "Eins", 1_750_000_000).id;
        let a = ordner(&s, "A");
        let b = ordner(&s, "B");
        s.set_meeting_folders(&m1, std::slice::from_ref(&a)).unwrap();
        s.ordner_projekt_setzen(&a, Some("prj_a")).unwrap();
        s.ordner_projekt_setzen(&b, Some("prj_b")).unwrap();
        assert_eq!(s.besprechungen_kurz().unwrap().len(), 1);
        assert_eq!(s.ordner_projekt_loesen_alle("prj_a").unwrap(), 1);
        let liste = s.ordner_verweise().unwrap();
        assert_eq!(liste.iter().find(|o| o.id == a).unwrap().projekt_id, None);
        assert_eq!(
            liste.iter().find(|o| o.id == b).unwrap().projekt_id.as_deref(),
            Some("prj_b")
        );
    }
}
