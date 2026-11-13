//! G9a-2 (lva#578, wai-sdk ADR-0031 E2.6–E2.8): Freigabe einer Besprechung für KI-Clients.
//!
//! Der lokale MCP-Server und die Agentenbrücke geben Besprechungen an Clients weiter, deren
//! Verarbeitung die App nicht kennt („MCP ist kein Nachweis lokaler Verarbeitung“). Darum
//! sieht ein Client eine Besprechung nur, wenn [`besprechungsplanung::assistent::pruefen_mcp_ausgabe`]
//! sie freigibt: Liste aus dem Kalendertermin, eingestuft nach den eigenen Domänen, vom Nutzer
//! für GENAU diese Besprechung und diese Liste bestätigt. Das gilt unabhängig vom Schalter
//! „Assistent in der Besprechung“ (Auflage B1) und auch für Altbestand (B2).
//!
//! Speicherung (Auflage B3): eine Zeile je Besprechung in `meeting_ai_confirmations`. Keine
//! Zeile heißt unbestätigt, also gesperrt; es gibt keine Rückfüllung. Gespeichert werden
//! Einstufung, Fingerabdruck (mit Versionspräfix im Hash) und Zeitpunkt; die Einstufung wird
//! bei jedem Abruf neu berechnet und nur zusammen mit einem passenden Fingerabdruck genutzt.
//! Die Zeile geht mit der Besprechung (hartes Löschen in `store.rs`).

use anyhow::{anyhow, Result};
use besprechungsplanung::assistent::{
    einstufen, pruefen_mcp_ausgabe, Bestaetigung, Einstufung, EinstufungsErgebnis, Entscheidung,
    McpAnfrage,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::store::MeetingStore;

/// Migration (Index 19): Bestätigungen je Besprechung. Nur `CREATE ... IF NOT EXISTS`.
pub(crate) const KI_FREIGABE_MIGRATION: &str =
    "CREATE TABLE IF NOT EXISTS meeting_ai_confirmations (
      meeting_id TEXT PRIMARY KEY,
      einstufung TEXT NOT NULL CHECK (einstufung IN ('intern', 'unbekannt', 'extern')),
      fingerabdruck TEXT NOT NULL,
      confirmed_at INTEGER NOT NULL);";

/// Teilnehmerliste einer Besprechung und ob sie aus einem Kalendertermin stammt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Teilnehmerlage {
    /// Je teilnehmender Person GENAU eine Angabe: die Adresse, sonst der Name, sonst `?`.
    /// Name und Adresse werden nie zu einem Text zusammengefügt (sonst könnte ein Name eine
    /// zweite Adresse einschmuggeln).
    pub teilnehmende: Vec<String>,
    /// Verknüpfter Termin mit Teilnehmerdaten aus einer nicht entfernten Quelle.
    pub liste_aus_termin: bool,
}

fn einstufung_text(e: Einstufung) -> &'static str {
    match e {
        Einstufung::Intern => "intern",
        Einstufung::Unbekannt => "unbekannt",
        Einstufung::Extern => "extern",
    }
}

fn einstufung_aus_text(s: &str) -> Option<Einstufung> {
    match s {
        "intern" => Some(Einstufung::Intern),
        "unbekannt" => Some(Einstufung::Unbekannt),
        "extern" => Some(Einstufung::Extern),
        _ => None,
    }
}

impl MeetingStore {
    /// Teilnehmerliste laut verknüpftem Termin. Ohne Verknüpfung, ohne Termin im Cache, aus
    /// einer entfernten Quelle oder einer Quelle ohne Teilnehmerdaten: leer und nicht aus
    /// einem Termin (zählt wie Ad-hoc).
    pub fn ki_teilnehmerlage(&self, meeting_id: &str) -> Result<Teilnehmerlage> {
        let conn = self.get_connection()?;
        let event_key: Option<String> = conn
            .query_row(
                "SELECT e.key FROM meeting_calendar_links l
                   JOIN calendar_events e ON e.key = l.event_key
                   JOIN calendar_sources s ON s.id = e.source_id
                  WHERE l.meeting_id = ?1 AND s.deleted_at IS NULL AND s.has_attendee_data = 1",
                params![meeting_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(key) = event_key else {
            return Ok(Teilnehmerlage {
                teilnehmende: Vec::new(),
                liste_aus_termin: false,
            });
        };
        let mut stmt = conn.prepare(
            "SELECT email, name FROM calendar_event_attendees WHERE event_key = ?1 ORDER BY rowid",
        )?;
        let teilnehmende = stmt
            .query_map(params![key], |r| {
                let email: Option<String> = r.get(0)?;
                let name: Option<String> = r.get(1)?;
                let email = email
                    .map(|e| e.trim().to_string())
                    .filter(|e| !e.is_empty());
                let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
                Ok(email.or(name).unwrap_or_else(|| "?".to_string()))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Teilnehmerlage {
            teilnehmende,
            liste_aus_termin: true,
        })
    }

    /// Gespeicherte Bestätigung der Besprechung; ein unlesbarer Wert zählt als keine.
    pub fn ki_bestaetigung(&self, meeting_id: &str) -> Result<Option<Bestaetigung>> {
        let conn = self.get_connection()?;
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT einstufung, fingerabdruck FROM meeting_ai_confirmations WHERE meeting_id = ?1",
                params![meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(e, f)| {
            einstufung_aus_text(&e).map(|einstufung| Bestaetigung {
                einstufung,
                fingerabdruck: f,
            })
        }))
    }

    /// Speichert die Bestätigung sofort (ersetzt eine vorhandene). Nur für lebende Besprechungen.
    pub fn ki_bestaetigung_setzen(
        &self,
        meeting_id: &str,
        bestaetigung: &Bestaetigung,
        now_ms: i64,
    ) -> Result<()> {
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;
        tx.execute(
            "INSERT OR REPLACE INTO meeting_ai_confirmations
               (meeting_id, einstufung, fingerabdruck, confirmed_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                meeting_id,
                einstufung_text(bestaetigung.einstufung),
                bestaetigung.fingerabdruck,
                now_ms
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Nimmt die Bestätigung zurück (Besprechung wieder gesperrt).
    pub fn ki_bestaetigung_entfernen(&self, meeting_id: &str) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "DELETE FROM meeting_ai_confirmations WHERE meeting_id = ?1",
            params![meeting_id],
        )?;
        Ok(())
    }
}

/// Was aus den Einstellungen in die Prüfung eingeht. Fehlt eine Angabe, gilt der strenge Wert
/// (keine eigenen Domänen, „Cloud bei Externen“ aus).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KiSperre {
    Pruefen {
        /// Kundenprofil „nur lokal“: Ausgabe an KI-Clients ganz gesperrt.
        kundenprofil_nur_lokal: bool,
        cloud_bei_externen: bool,
        eigene_domaenen: Vec<String>,
    },
    /// Nur für Tests der Werkzeuge, die mit der Freigabe nichts zu tun haben.
    #[cfg(test)]
    Offen,
}

impl KiSperre {
    /// Aus den Einstellungen der App (gleiche Bedeutung wie `mcp::read_ki_sperre`).
    pub fn aus_einstellungen(s: &crate::settings::AppSettings) -> Self {
        KiSperre::Pruefen {
            kundenprofil_nur_lokal: s.compliance_profile
                == crate::managers::compliance::ComplianceProfile::LocalOnly,
            cloud_bei_externen: s.meeting_cloud_with_externals,
            eigene_domaenen: s.meeting_own_domains.clone(),
        }
    }

    /// Wird überhaupt geprüft? (Nur die Testvariante prüft nicht.)
    pub fn prueft(&self) -> bool {
        matches!(self, KiSperre::Pruefen { .. })
    }

    /// Ist die Ausgabe an KI-Clients insgesamt gesperrt (Kundenprofil)?
    pub fn ganz_gesperrt(&self) -> bool {
        match self {
            KiSperre::Pruefen {
                kundenprofil_nur_lokal,
                ..
            } => *kundenprofil_nur_lokal,
            #[cfg(test)]
            KiSperre::Offen => false,
        }
    }

    /// Prüft eine Besprechung bei JEDEM Abruf neu (Liste, Domänen, Bestätigung).
    pub fn pruefen(&self, store: &MeetingStore, meeting_id: &str) -> Result<Entscheidung> {
        match self {
            KiSperre::Pruefen {
                kundenprofil_nur_lokal,
                cloud_bei_externen,
                eigene_domaenen,
            } => {
                let lage = store.ki_teilnehmerlage(meeting_id)?;
                Ok(pruefen_mcp_ausgabe(&McpAnfrage {
                    kundenprofil_nur_lokal: *kundenprofil_nur_lokal,
                    cloud_bei_externen: *cloud_bei_externen,
                    teilnehmende: lage.teilnehmende,
                    liste_aus_termin: lage.liste_aus_termin,
                    eigene_domaenen: eigene_domaenen.clone(),
                    bestaetigung: store.ki_bestaetigung(meeting_id)?,
                }))
            }
            #[cfg(test)]
            KiSperre::Offen => Ok(Entscheidung::Erlaubt {
                einstufung: Einstufung::Intern,
            }),
        }
    }

    /// `true`, wenn ein KI-Client die Besprechung sehen darf; ein Lesefehler sperrt.
    pub fn sichtbar(&self, store: &MeetingStore, meeting_id: &str) -> bool {
        self.pruefen(store, meeting_id)
            .map(|e| e.erlaubt())
            .unwrap_or(false)
    }
}

/// Stand der Freigabe für die Oberfläche.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct KiFreigabeStand {
    pub ergebnis: EinstufungsErgebnis,
    pub bestaetigung: Option<Bestaetigung>,
    /// Entscheidung für KI-Clients mit den aktuellen Einstellungen.
    pub entscheidung: Entscheidung,
}

/// Stand einer Besprechung für die Oberfläche.
pub fn stand(store: &MeetingStore, sperre: &KiSperre, meeting_id: &str) -> Result<KiFreigabeStand> {
    let lage = store.ki_teilnehmerlage(meeting_id)?;
    let eigene = match sperre {
        KiSperre::Pruefen {
            eigene_domaenen, ..
        } => eigene_domaenen.clone(),
        #[cfg(test)]
        KiSperre::Offen => Vec::new(),
    };
    Ok(KiFreigabeStand {
        ergebnis: einstufen(&lage.teilnehmende, &eigene, lage.liste_aus_termin),
        bestaetigung: store.ki_bestaetigung(meeting_id)?,
        entscheidung: sperre.pruefen(store, meeting_id)?,
    })
}

/// Bestätigt die Einstufung. Sie muss zur AKTUELLEN Liste passen (Fingerabdruck) und darf
/// nicht lockerer sein als die berechnete: „intern“ nur, wenn die Liste intern ist.
pub fn bestaetigen(
    store: &MeetingStore,
    sperre: &KiSperre,
    meeting_id: &str,
    einstufung: Einstufung,
    fingerabdruck: &str,
    now_ms: i64,
) -> Result<KiFreigabeStand> {
    let aktuell = stand(store, sperre, meeting_id)?;
    if aktuell.ergebnis.fingerabdruck != fingerabdruck {
        return Err(anyhow!("ki_freigabe_veraltet"));
    }
    if einstufung == Einstufung::Intern && aktuell.ergebnis.einstufung != Einstufung::Intern {
        return Err(anyhow!("ki_freigabe_nicht_intern"));
    }
    store.ki_bestaetigung_setzen(
        meeting_id,
        &Bestaetigung {
            einstufung,
            fingerabdruck: fingerabdruck.to_string(),
        },
        now_ms,
    )?;
    stand(store, sperre, meeting_id)
}

#[cfg(test)]
#[path = "ki_freigabe_tests.rs"]
mod tests;
