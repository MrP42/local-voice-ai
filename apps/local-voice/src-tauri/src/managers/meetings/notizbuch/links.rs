//! Speicher der Zuordnungen Notizbuch-Seite ↔ Besprechung (Spec §5.2, lva#396 N4a Schritt 2).
//!
//! Tabelle `meeting_note_links` im Besprechungs-Store, je `(meeting_id, seite_id)` eine Zeile:
//!
//! - `art`: `auto` (automatisch zugeordnet, erst nach bestandenem Eval-Tor), `vorschlag`
//!   (zwischen Schwelle B und A), `bestaetigt` (vom Nutzer zugeordnet), `geloest` (vom Nutzer
//!   gelöst; wird **nie** automatisch überschrieben oder wieder zugeordnet).
//! - `stand`: Freigabestand des Notizbuchs beim Schreiben (Hash, kein Inhalt).
//! - `punkte`, `gruende_json`: Bewertung und Zuordnungsgrund für die Anzeige.
//! - `aktualisiert`: Unix-Millisekunden.
//!
//! Automatik schreibt nur über [`MeetingStore::note_links_automatisch_ersetzen`]: sie fasst
//! ausschließlich Zeilen mit `auto`/`vorschlag` an. Nutzerentscheidungen gehen über
//! [`MeetingStore::note_link_setzen`] und [`MeetingStore::note_link_loesen`].
//!
//! Die Migration ist additiv (`CREATE … IF NOT EXISTS`) und damit idempotent; wie jeder Schritt
//! läuft sie in einer Transaktion. Der SQL-Text steht hier, damit der Eintrag in
//! `store.rs::MIGRATIONS` nur aus einer Zeile besteht.

use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};

use crate::managers::meetings::store::MeetingStore;

/// Migration Index 16 (lva#396 N4a): `meeting_note_links`. Nur CREATE … IF NOT EXISTS.
pub const NOTE_LINKS_MIGRATION: &str = "CREATE TABLE IF NOT EXISTS meeting_note_links (
      meeting_id TEXT NOT NULL,
      seite_id TEXT NOT NULL,
      art TEXT NOT NULL CHECK (art IN ('auto', 'bestaetigt', 'geloest', 'vorschlag')),
      stand TEXT NOT NULL DEFAULT '',
      punkte INTEGER NOT NULL DEFAULT 0,
      gruende_json TEXT NOT NULL DEFAULT '[]',
      aktualisiert INTEGER NOT NULL,
      PRIMARY KEY (meeting_id, seite_id));
    CREATE INDEX IF NOT EXISTS idx_meeting_note_links_seite ON meeting_note_links(seite_id);";

/// Art einer Zuordnung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkArt {
    Auto,
    Bestaetigt,
    Geloest,
    Vorschlag,
}

impl LinkArt {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkArt::Auto => "auto",
            LinkArt::Bestaetigt => "bestaetigt",
            LinkArt::Geloest => "geloest",
            LinkArt::Vorschlag => "vorschlag",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(LinkArt::Auto),
            "bestaetigt" => Some(LinkArt::Bestaetigt),
            "geloest" => Some(LinkArt::Geloest),
            "vorschlag" => Some(LinkArt::Vorschlag),
            _ => None,
        }
    }

    /// Zugeordnet im Sinne der Zusammenführung (zweite Notizquelle).
    pub fn ist_zugeordnet(self) -> bool {
        matches!(self, LinkArt::Auto | LinkArt::Bestaetigt)
    }

    /// Von der Automatik geschrieben (darf von ihr ersetzt oder entfernt werden).
    pub fn ist_automatisch(self) -> bool {
        matches!(self, LinkArt::Auto | LinkArt::Vorschlag)
    }
}

/// Eine gespeicherte Zuordnung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLink {
    pub meeting_id: String,
    pub seite_id: String,
    pub art: LinkArt,
    pub stand: String,
    pub punkte: u32,
    pub gruende_json: String,
    pub aktualisiert: i64,
}

/// Ergebnis der Automatik für eine Seite (nur `Auto` oder `Vorschlag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoLink {
    pub seite_id: String,
    pub art: LinkArt,
    pub punkte: u32,
    pub gruende_json: String,
}

fn seite_id_pruefen(id: &str) -> Result<()> {
    if wai_notizbuch_zugang::gueltige_id(id) {
        Ok(())
    } else {
        Err(anyhow!("seite_invalid"))
    }
}

fn jetzt_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl MeetingStore {
    /// Alle Zuordnungen einer Besprechung, nach Seite sortiert. Zeilen mit unbekannter Art
    /// (künftige Fassung) werden übergangen.
    pub fn note_links(&self, meeting_id: &str) -> Result<Vec<NoteLink>> {
        let conn = self.get_connection()?;
        let mut st = conn.prepare(
            "SELECT meeting_id, seite_id, art, stand, punkte, gruende_json, aktualisiert
             FROM meeting_note_links WHERE meeting_id = ?1 ORDER BY seite_id",
        )?;
        let zeilen = st
            .query_map(params![meeting_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(zeilen
            .into_iter()
            .filter_map(
                |(meeting_id, seite_id, art, stand, punkte, gruende_json, aktualisiert)| {
                    Some(NoteLink {
                        meeting_id,
                        seite_id,
                        art: LinkArt::parse(&art)?,
                        stand,
                        punkte: punkte.clamp(0, i64::from(u32::MAX)) as u32,
                        gruende_json,
                        aktualisiert,
                    })
                },
            )
            .collect())
    }

    /// Eine Zuordnung, falls vorhanden.
    pub fn note_link(&self, meeting_id: &str, seite_id: &str) -> Result<Option<NoteLink>> {
        Ok(self
            .note_links(meeting_id)?
            .into_iter()
            .find(|l| l.seite_id == seite_id))
    }

    /// Nutzerentscheidung: setzt die Art (`bestaetigt` oder `geloest`) und überschreibt jede
    /// vorhandene Zeile. Automatische Arten sind hier nicht erlaubt (`link_art_invalid`).
    pub fn note_link_setzen(
        &self,
        meeting_id: &str,
        seite_id: &str,
        art: LinkArt,
        stand: &str,
    ) -> Result<()> {
        if art.ist_automatisch() {
            return Err(anyhow!("link_art_invalid"));
        }
        seite_id_pruefen(seite_id)?;
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;
        // Grund und Punkte einer früheren Bewertung bleiben bei der Bestätigung erhalten.
        tx.execute(
            "INSERT INTO meeting_note_links (meeting_id, seite_id, art, stand, aktualisiert)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (meeting_id, seite_id) DO UPDATE SET
               art = excluded.art, stand = excluded.stand, aktualisiert = excluded.aktualisiert",
            params![meeting_id, seite_id, art.as_str(), stand, jetzt_ms()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// „Lösen“: die Seite gehört nicht zu dieser Besprechung; wird nie wieder automatisch
    /// zugeordnet.
    pub fn note_link_loesen(&self, meeting_id: &str, seite_id: &str) -> Result<()> {
        self.note_link_setzen(meeting_id, seite_id, LinkArt::Geloest, "")
    }

    /// Ergebnis eines Automatiklaufs übernehmen: ersetzt alle automatischen Zeilen (`auto`,
    /// `vorschlag`) der Besprechung durch `neu`. Zeilen des Nutzers (`bestaetigt`, `geloest`)
    /// bleiben unverändert; ein Eintrag in `neu` für eine solche Seite wird übergangen.
    /// Nicht automatische Arten in `neu` werden abgewiesen (`link_art_invalid`).
    pub fn note_links_automatisch_ersetzen(
        &self,
        meeting_id: &str,
        stand: &str,
        neu: &[AutoLink],
    ) -> Result<()> {
        if neu.iter().any(|l| !l.art.ist_automatisch()) {
            return Err(anyhow!("link_art_invalid"));
        }
        for l in neu {
            seite_id_pruefen(&l.seite_id)?;
        }
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        tx.execute(
            "DELETE FROM meeting_note_links
             WHERE meeting_id = ?1 AND art IN ('auto', 'vorschlag')",
            params![meeting_id],
        )?;
        let jetzt = jetzt_ms();
        for l in neu {
            // `DO NOTHING`: eine Nutzerzeile (nach dem DELETE die einzige mögliche) gewinnt.
            tx.execute(
                "INSERT INTO meeting_note_links
                   (meeting_id, seite_id, art, stand, punkte, gruende_json, aktualisiert)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (meeting_id, seite_id) DO NOTHING",
                params![
                    meeting_id,
                    l.seite_id,
                    l.art.as_str(),
                    stand,
                    i64::from(l.punkte),
                    l.gruende_json,
                    jetzt
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Art einer Zuordnung, ohne die ganze Liste zu laden.
    pub fn note_link_art(&self, meeting_id: &str, seite_id: &str) -> Result<Option<LinkArt>> {
        let conn = self.get_connection()?;
        let art: Option<String> = conn
            .query_row(
                "SELECT art FROM meeting_note_links WHERE meeting_id = ?1 AND seite_id = ?2",
                params![meeting_id, seite_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(art.as_deref().and_then(LinkArt::parse))
    }
}
