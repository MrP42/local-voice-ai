//! G9a-2 (lva#578, wai-sdk ADR-0031 E2.6–E2.8): Freigabe einer Besprechung für KI-Clients.
//!
//! Der lokale MCP-Server und die Agentenbrücke geben Besprechungen an Clients weiter, deren
//! Verarbeitung die App nicht kennt („MCP ist kein Nachweis lokaler Verarbeitung“). Darum
//! sieht ein Client eine Besprechung nur, wenn [`besprechungsplanung::assistent::pruefen_mcp_ausgabe`]
//! sie freigibt: Liste aus dem Kalendertermin, eingestuft nach den eigenen Domänen, vom Nutzer
//! für GENAU diese Besprechung und diese Liste bestätigt. Das gilt unabhängig vom Schalter
//! „Assistent in der Besprechung“ (Auflage B1) und auch für Altbestand (B2).
//!
//! Nachweis (ADR-0031 E2 Nr. 7, Codex #583): Eine Bestätigung setzt voraus, dass der Nutzer
//! erklärt, die Teilnehmenden informiert zu haben, und die geprüfte Rechtsgrundlage wählt; bei
//! Einwilligung zusätzlich, dass sie von allen vorliegt (sonst `ki_freigabe_ohne_information`,
//! `ki_freigabe_ohne_rechtsgrundlage`, `ki_freigabe_ohne_einwilligung`). Die rechtliche Prüfung
//! selbst bleibt beim Nutzer; die App hält Erklärung und Grundlage je Besprechung fest.
//!
//! Linearisierung (Codex #583 Fixrunde 2): Eine monotone Revision (`meeting_ai_revision`, per
//! Trigger) und ein Spiegel der Richtlinie (`meeting_ai_policy`) erlauben `mcp::tools::call`,
//! am Ende eines Aufrufs zu prüfen, dass sich seit dem Anfang nichts an der Freigabegrundlage
//! geändert hat.
//!
//! Speicherung (Auflage B3): eine Zeile je Besprechung in `meeting_ai_confirmations`. Keine
//! Zeile heißt unbestätigt, also gesperrt; es gibt keine Rückfüllung. Gespeichert werden
//! Einstufung, Fingerabdruck, dessen Schemaversion und Zeitpunkt, aber keine Kopie der
//! Teilnehmenden (der Hash ist bei erratbaren Listen nicht anonym, darum nur hier lokal).
//! Eine Zeile mit anderer Schemaversion zählt nicht. Die Einstufung wird bei jedem Abruf neu
//! berechnet und nur zusammen mit einem passenden Fingerabdruck genutzt. Die Zeile geht mit
//! der Besprechung (Löschen in `store.rs`; die Aufbewahrung löscht nur Audio, Transkript und
//! Bestätigung bleiben zusammen).
//!
//! Teilnehmerstand (Codex-Blocker 1 zu #582): Die Liste gilt nur als aktuell und vollständig,
//! wenn die Kalenderquelle NACH dem Ende des Termins erfolgreich abgeglichen hat und die
//! Liste nicht an der Importgrenze gekürzt sein kann. Sonst sieht kein Client die Besprechung.

use anyhow::{anyhow, Result};
use besprechungsplanung::assistent::{
    einstufen, pruefen_mcp_ausgabe, Bestaetigung, Einstufung, EinstufungsErgebnis, Entscheidung,
    McpAnfrage, Teilnehmerinfo, FINGERABDRUCK_VERSION,
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
      fp_version INTEGER NOT NULL,
      informiert INTEGER NOT NULL CHECK (informiert = 1),
      confirmed_at INTEGER NOT NULL);";

/// Migration (Index 20, Codex #583 Fixrunde 2): Revision, Richtlinienspiegel, Rechtsgrundlage.
///
/// - `meeting_ai_revision`: EIN Zähler, den Trigger bei JEDER Änderung an der Freigabegrundlage
///   erhöhen (Bestätigungen, Richtlinie, Termine, Teilnehmende, Quellen, Verknüpfungen). Er
///   sinkt nie; darum erkennt ein Vorher-/Nachher-Vergleich auch A → B → A.
/// - `meeting_ai_policy`: Spiegel der Richtlinie aus den Einstellungen (Kundenprofil, „Cloud bei
///   Externen“, eigene Domänen). Die App schreibt ihn bei jeder Einstellungsänderung; der
///   MCP-Server gibt nur aus, wenn Spiegel und Einstellungsdatei übereinstimmen.
/// - Rechtsgrundlage je Bestätigung (ADR-0031 E2 Nr. 7). Altzeilen ohne sie zählen nicht.
pub(crate) const KI_REVISION_MIGRATION: &str = "
    CREATE TABLE IF NOT EXISTS meeting_ai_revision (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      rev INTEGER NOT NULL);
    INSERT OR IGNORE INTO meeting_ai_revision (id, rev) VALUES (1, 0);
    CREATE TABLE IF NOT EXISTS meeting_ai_policy (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      nur_lokal INTEGER NOT NULL CHECK (nur_lokal IN (0, 1)),
      cloud_bei_externen INTEGER NOT NULL CHECK (cloud_bei_externen IN (0, 1)),
      eigene_domaenen TEXT NOT NULL);
    ALTER TABLE meeting_ai_confirmations ADD COLUMN rechtsgrundlage TEXT;
    ALTER TABLE meeting_ai_confirmations ADD COLUMN einwilligungen_belegt INTEGER NOT NULL DEFAULT 0;
    CREATE TRIGGER IF NOT EXISTS ai_rev_conf_i AFTER INSERT ON meeting_ai_confirmations
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_conf_u AFTER UPDATE ON meeting_ai_confirmations
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_conf_d AFTER DELETE ON meeting_ai_confirmations
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_pol_i AFTER INSERT ON meeting_ai_policy
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_pol_u AFTER UPDATE ON meeting_ai_policy
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_pol_d AFTER DELETE ON meeting_ai_policy
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_link_i AFTER INSERT ON meeting_calendar_links
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_link_u AFTER UPDATE ON meeting_calendar_links
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_link_d AFTER DELETE ON meeting_calendar_links
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_ev_i AFTER INSERT ON calendar_events
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_ev_u AFTER UPDATE ON calendar_events
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_ev_d AFTER DELETE ON calendar_events
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_att_i AFTER INSERT ON calendar_event_attendees
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_att_u AFTER UPDATE ON calendar_event_attendees
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_att_d AFTER DELETE ON calendar_event_attendees
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_src_i AFTER INSERT ON calendar_sources
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_src_u AFTER UPDATE ON calendar_sources
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;
    CREATE TRIGGER IF NOT EXISTS ai_rev_src_d AFTER DELETE ON calendar_sources
      BEGIN UPDATE meeting_ai_revision SET rev = rev + 1 WHERE id = 1; END;";

/// Rechtsgrundlage der Weitergabe an KI-Clients (ADR-0031 E2 Nr. 7). Die App prüft sie nicht
/// rechtlich; sie hält fest, auf welche geprüfte Grundlage der Nutzer die Freigabe stützt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Rechtsgrundlage {
    /// Art. 6 Abs. 1 lit. a DSGVO; nur mit [`Freigabenachweis::einwilligungen_liegen_vor`].
    Einwilligung,
    /// Art. 6 Abs. 1 lit. b DSGVO.
    Vertrag,
    /// Art. 6 Abs. 1 lit. c DSGVO.
    RechtlichePflicht,
    /// Art. 6 Abs. 1 lit. e DSGVO (öffentliche Aufgabe).
    OeffentlicheAufgabe,
    /// Art. 6 Abs. 1 lit. f DSGVO.
    BerechtigtesInteresse,
    /// Dienst- oder Betriebsvereinbarung (Beschäftigte, LDSG BW bzw. § 26 BDSG).
    Kollektivvereinbarung,
}

impl Rechtsgrundlage {
    fn text(self) -> &'static str {
        match self {
            Rechtsgrundlage::Einwilligung => "einwilligung",
            Rechtsgrundlage::Vertrag => "vertrag",
            Rechtsgrundlage::RechtlichePflicht => "rechtliche_pflicht",
            Rechtsgrundlage::OeffentlicheAufgabe => "oeffentliche_aufgabe",
            Rechtsgrundlage::BerechtigtesInteresse => "berechtigtes_interesse",
            Rechtsgrundlage::Kollektivvereinbarung => "kollektivvereinbarung",
        }
    }

    fn aus_text(s: &str) -> Option<Self> {
        [
            Rechtsgrundlage::Einwilligung,
            Rechtsgrundlage::Vertrag,
            Rechtsgrundlage::RechtlichePflicht,
            Rechtsgrundlage::OeffentlicheAufgabe,
            Rechtsgrundlage::BerechtigtesInteresse,
            Rechtsgrundlage::Kollektivvereinbarung,
        ]
        .into_iter()
        .find(|r| r.text() == s)
    }
}

/// Was der Nutzer mit der Bestätigung erklärt (ADR-0031 E2 Nr. 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Freigabenachweis {
    /// Die Teilnehmenden sind über Zweck, Empfänger, Cloud-Nutzung und Speicherdauer informiert.
    pub informiert: bool,
    /// Die geprüfte Rechtsgrundlage; ohne sie keine Freigabe.
    pub rechtsgrundlage: Option<Rechtsgrundlage>,
    /// Bei Einwilligung: Sie liegt von allen Teilnehmenden vor (ein Hinweis ersetzt sie nicht).
    pub einwilligungen_liegen_vor: bool,
}

impl Freigabenachweis {
    /// Die Grundlage, wenn der Nachweis vollständig ist; sonst der Fehlercode.
    fn pruefen(&self) -> Result<Rechtsgrundlage> {
        if !self.informiert {
            return Err(anyhow!("ki_freigabe_ohne_information"));
        }
        let grundlage = self
            .rechtsgrundlage
            .ok_or_else(|| anyhow!("ki_freigabe_ohne_rechtsgrundlage"))?;
        if grundlage == Rechtsgrundlage::Einwilligung && !self.einwilligungen_liegen_vor {
            return Err(anyhow!("ki_freigabe_ohne_einwilligung"));
        }
        Ok(grundlage)
    }
}

/// Richtlinie für KI-Clients, wie sie im Spiegel `meeting_ai_policy` steht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Richtlinie {
    pub nur_lokal: bool,
    pub cloud_bei_externen: bool,
    pub eigene_domaenen: Vec<String>,
}

impl Richtlinie {
    pub fn aus_einstellungen(s: &crate::settings::AppSettings) -> Self {
        Richtlinie {
            nur_lokal: s.compliance_profile
                == crate::managers::compliance::ComplianceProfile::LocalOnly,
            cloud_bei_externen: s.meeting_cloud_with_externals,
            eigene_domaenen: s.meeting_own_domains.clone(),
        }
    }
}

/// Revision und Richtlinienspiegel aus EINEM Datenbankstand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KiStand {
    pub rev: i64,
    pub richtlinie: Option<Richtlinie>,
}

/// Importgrenze der Teilnehmerliste je Termin (`calendar::ics` und `calendar::graph`). Eine
/// Liste dieser Länge kann gekürzt sein und gilt als unvollständig.
const TEILNEHMER_IMPORTGRENZE: usize = 500;

/// Teilnehmerliste einer Besprechung und ob sie aus einem Kalendertermin stammt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Teilnehmerlage {
    /// Je teilnehmender Person GENAU eine Angabe: die gelieferte Adresse, sonst `?`. Ein
    /// Anzeigename zählt NIE als Adresse, auch nicht, wenn er wie eine aussieht (Codex #583,
    /// Blocker 1); er geht darum gar nicht in die Prüfung ein.
    pub teilnehmende: Vec<String>,
    /// Verknüpfter Termin mit Teilnehmerdaten aus einer nicht entfernten Quelle.
    pub liste_aus_termin: bool,
    /// Aktualität und Vollständigkeit der Liste (ohne Termin: unbekannt).
    pub teilnehmerinfo: Teilnehmerinfo,
}

/// Alles, was die Prüfung einer Besprechung aus der Datenbank braucht, gelesen in EINER
/// Lesetransaktion (ein Datenbankstand): Teilnehmerlage und gespeicherte Bestätigung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KiLage {
    pub lage: Teilnehmerlage,
    pub bestaetigung: Option<Bestaetigung>,
}

#[cfg(test)]
thread_local! {
    /// Testbarriere: läuft in `mcp::tools::call` nach dem Lesen der Nutzlast und vor der
    /// abschließenden Prüfung (dem Linearisierungspunkt).
    pub(crate) static VOR_NACHPRUEFUNG: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn barriere_ausloesen() {
    VOR_NACHPRUEFUNG.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f()
        }
    });
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
        Ok(self.ki_lage(meeting_id)?.lage)
    }

    /// Teilnehmerlage und Bestätigung aus EINEM Datenbankstand (Lesetransaktion), damit die
    /// Prüfung nie eine Liste mit der Bestätigung eines anderen Stands mischt.
    pub fn ki_lage(&self, meeting_id: &str) -> Result<KiLage> {
        let conn = self.get_connection()?;
        let tx = conn.unchecked_transaction()?;
        let lage = Self::ki_teilnehmerlage_in(&tx, meeting_id)?;
        let bestaetigung = Self::ki_bestaetigung_in(&tx, meeting_id)?;
        tx.commit()?;
        Ok(KiLage { lage, bestaetigung })
    }

    fn ki_teilnehmerlage_in(
        conn: &rusqlite::Connection,
        meeting_id: &str,
    ) -> Result<Teilnehmerlage> {
        let termin: Option<(String, i64, Option<i64>)> = conn
            .query_row(
                "SELECT e.key, e.ends_at, s.last_ok_at FROM meeting_calendar_links l
                   JOIN calendar_events e ON e.key = l.event_key
                   JOIN calendar_sources s ON s.id = e.source_id
                  WHERE l.meeting_id = ?1 AND s.deleted_at IS NULL AND s.has_attendee_data = 1",
                params![meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((key, ends_at, last_ok_at)) = termin else {
            return Ok(Teilnehmerlage {
                teilnehmende: Vec::new(),
                liste_aus_termin: false,
                teilnehmerinfo: Teilnehmerinfo::Unbekannt,
            });
        };
        let mut stmt = conn.prepare(
            "SELECT email FROM calendar_event_attendees WHERE event_key = ?1 ORDER BY rowid",
        )?;
        let teilnehmende = stmt
            .query_map(params![key], |r| {
                let email: Option<String> = r.get(0)?;
                Ok(email
                    .map(|e| e.trim().to_string())
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| "?".to_string()))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let teilnehmerinfo = if last_ok_at.is_none_or(|ok| ok < ends_at) {
            Teilnehmerinfo::Veraltet
        } else if teilnehmende.len() >= TEILNEHMER_IMPORTGRENZE {
            Teilnehmerinfo::Unvollstaendig
        } else {
            Teilnehmerinfo::AktuellVollstaendig
        };
        Ok(Teilnehmerlage {
            teilnehmende,
            liste_aus_termin: true,
            teilnehmerinfo,
        })
    }

    /// Gespeicherte Bestätigung der Besprechung; ein unlesbarer Wert oder eine andere
    /// Schemaversion des Fingerabdrucks zählt als keine.
    pub fn ki_bestaetigung(&self, meeting_id: &str) -> Result<Option<Bestaetigung>> {
        Ok(self.ki_lage(meeting_id)?.bestaetigung)
    }

    fn ki_bestaetigung_in(
        conn: &rusqlite::Connection,
        meeting_id: &str,
    ) -> Result<Option<Bestaetigung>> {
        let row: Option<(String, String, i64, Option<String>, i64)> = conn
            .query_row(
                "SELECT einstufung, fingerabdruck, fp_version, rechtsgrundlage, einwilligungen_belegt
                   FROM meeting_ai_confirmations WHERE meeting_id = ?1",
                params![meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        // Zählt nur mit passender Schemaversion und vollständigem Nachweis (Rechtsgrundlage,
        // bei Einwilligung deren Vorliegen); eine Altzeile ohne Rechtsgrundlage zählt nicht.
        let row = row.filter(|(_, _, v, grund, belegt)| {
            *v == i64::from(FINGERABDRUCK_VERSION)
                && match grund.as_deref().and_then(Rechtsgrundlage::aus_text) {
                    Some(Rechtsgrundlage::Einwilligung) => *belegt == 1,
                    Some(_) => true,
                    None => false,
                }
        });
        Ok(row.and_then(|(e, f, ..)| {
            einstufung_aus_text(&e).map(|einstufung| Bestaetigung {
                einstufung,
                fingerabdruck: f,
            })
        }))
    }

    /// Revision und Richtlinienspiegel aus einem Datenbankstand (Lesetransaktion).
    pub fn ki_stand(&self) -> Result<KiStand> {
        let conn = self.get_connection()?;
        let tx = conn.unchecked_transaction()?;
        let rev: i64 = tx.query_row(
            "SELECT rev FROM meeting_ai_revision WHERE id = 1",
            [],
            |r| r.get(0),
        )?;
        let richtlinie = Self::ki_richtlinie_in(&tx)?;
        tx.commit()?;
        Ok(KiStand { rev, richtlinie })
    }

    fn ki_richtlinie_in(conn: &rusqlite::Connection) -> Result<Option<Richtlinie>> {
        let row: Option<(bool, bool, String)> = conn
            .query_row(
                "SELECT nur_lokal, cloud_bei_externen, eigene_domaenen FROM meeting_ai_policy
                  WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        // Ein unlesbarer Spiegel zählt wie keiner (gesperrt).
        Ok(row.and_then(|(nur_lokal, cloud_bei_externen, domaenen)| {
            serde_json::from_str::<Vec<String>>(&domaenen)
                .ok()
                .map(|eigene_domaenen| Richtlinie {
                    nur_lokal,
                    cloud_bei_externen,
                    eigene_domaenen,
                })
        }))
    }

    /// Schreibt den Richtlinienspiegel in EINER Schreibtransaktion. Ändern sich dabei die eigenen
    /// Domänen, werden im selben Zug ALLE Bestätigungen gelöscht (eine Rückkehr zu alten Domänen
    /// reaktiviert nichts). Unverändert: kein Schreiben, keine neue Revision. `true`, wenn sich
    /// der Spiegel geändert hat.
    pub fn ki_richtlinie_setzen(&self, neu: &Richtlinie) -> Result<bool> {
        let mut conn = self.get_connection()?;
        let tx = Self::write_tx(&mut conn)?;
        let alt = Self::ki_richtlinie_in(&tx)?;
        if alt.as_ref() == Some(neu) {
            return Ok(false);
        }
        if alt.as_ref().map(|a| &a.eigene_domaenen) != Some(&neu.eigene_domaenen) {
            tx.execute("DELETE FROM meeting_ai_confirmations", [])?;
        }
        tx.execute(
            "INSERT INTO meeting_ai_policy (id, nur_lokal, cloud_bei_externen, eigene_domaenen)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET nur_lokal = excluded.nur_lokal,
               cloud_bei_externen = excluded.cloud_bei_externen,
               eigene_domaenen = excluded.eigene_domaenen",
            params![
                neu.nur_lokal,
                neu.cloud_bei_externen,
                serde_json::to_string(&neu.eigene_domaenen)?
            ],
        )?;
        tx.commit()?;
        Ok(true)
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

    /// Entscheidung über einen gelesenen Stand (rein, ohne Datenbank).
    pub fn entscheiden(&self, l: &KiLage) -> Entscheidung {
        match self {
            KiSperre::Pruefen {
                kundenprofil_nur_lokal,
                cloud_bei_externen,
                eigene_domaenen,
            } => pruefen_mcp_ausgabe(&McpAnfrage {
                kundenprofil_nur_lokal: *kundenprofil_nur_lokal,
                cloud_bei_externen: *cloud_bei_externen,
                teilnehmende: l.lage.teilnehmende.clone(),
                liste_aus_termin: l.lage.liste_aus_termin,
                teilnehmerinfo: l.lage.teilnehmerinfo,
                eigene_domaenen: eigene_domaenen.clone(),
                bestaetigung: l.bestaetigung.clone(),
            }),
            #[cfg(test)]
            KiSperre::Offen => Entscheidung::Erlaubt {
                einstufung: Einstufung::Intern,
            },
        }
    }

    /// Prüft eine Besprechung bei JEDEM Abruf neu (Liste, Domänen, Bestätigung).
    pub fn pruefen(&self, store: &MeetingStore, meeting_id: &str) -> Result<Entscheidung> {
        Ok(self.entscheiden(&store.ki_lage(meeting_id)?))
    }

    /// `true`, wenn ein KI-Client die Besprechung sehen darf; ein Lesefehler sperrt.
    pub fn sichtbar(&self, store: &MeetingStore, meeting_id: &str) -> bool {
        store
            .ki_lage(meeting_id)
            .is_ok_and(|l| self.entscheiden(&l).erlaubt())
    }

    /// Die Richtlinie dieser Sperre (die Testvariante hat keine).
    pub fn richtlinie(&self) -> Option<Richtlinie> {
        match self {
            KiSperre::Pruefen {
                kundenprofil_nur_lokal,
                cloud_bei_externen,
                eigene_domaenen,
            } => Some(Richtlinie {
                nur_lokal: *kundenprofil_nur_lokal,
                cloud_bei_externen: *cloud_bei_externen,
                eigene_domaenen: eigene_domaenen.clone(),
            }),
            #[cfg(test)]
            KiSperre::Offen => None,
        }
    }

    /// Abgleich mit dem Spiegel in der Datenbank (Codex #583 Fixrunde 2): Nur wenn beide
    /// übereinstimmen, gilt die Sperre; fehlt der Spiegel oder weicht er ab (eine Änderung ist
    /// gerade unterwegs, oder sein Schreiben schlug fehl), gibt es KEINE Ausgabe (`None`).
    pub fn abgleichen(&self, spiegel: Option<&Richtlinie>) -> Option<KiSperre> {
        match self.richtlinie() {
            None => Some(self.clone()),
            Some(r) => (spiegel == Some(&r)).then(|| self.clone()),
        }
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
    let l = store.ki_lage(meeting_id)?;
    let lage = &l.lage;
    let eigene = match sperre {
        KiSperre::Pruefen {
            eigene_domaenen, ..
        } => eigene_domaenen.clone(),
        #[cfg(test)]
        KiSperre::Offen => Vec::new(),
    };
    Ok(KiFreigabeStand {
        ergebnis: einstufen(&lage.teilnehmende, &eigene, lage.liste_aus_termin),
        entscheidung: sperre.entscheiden(&l),
        bestaetigung: l.bestaetigung,
    })
}

/// Bestätigt die Einstufung. Prüfung und Speichern laufen in EINER Schreibtransaktion gegen den
/// Richtlinienspiegel der Datenbank (Codex #583 Fixrunde 2): Ein gleichzeitiger Wechsel der
/// eigenen Domänen ist entweder vorher festgeschrieben (dann gilt er hier) oder er löscht diese
/// Bestätigung danach. Die Einstufung muss zur AKTUELLEN Liste passen (Fingerabdruck) und darf
/// nicht lockerer sein als die berechnete: „intern“ nur, wenn die Liste intern ist. Der
/// Nachweis (Information, Rechtsgrundlage, bei Einwilligung deren Vorliegen) ist Pflicht.
pub fn bestaetigen(
    store: &MeetingStore,
    sperre: &KiSperre,
    meeting_id: &str,
    einstufung: Einstufung,
    fingerabdruck: &str,
    nachweis: &Freigabenachweis,
    now_ms: i64,
) -> Result<KiFreigabeStand> {
    let grundlage = nachweis.pruefen()?;
    {
        let mut conn = store.get_connection()?;
        let tx = MeetingStore::write_tx(&mut conn)?;
        MeetingStore::ensure_meeting_is_live(&tx, meeting_id)?;
        let richtlinie = MeetingStore::ki_richtlinie_in(&tx)?
            .ok_or_else(|| anyhow!("ki_freigabe_richtlinie_fehlt"))?;
        let lage = MeetingStore::ki_teilnehmerlage_in(&tx, meeting_id)?;
        let ergebnis = einstufen(
            &lage.teilnehmende,
            &richtlinie.eigene_domaenen,
            lage.liste_aus_termin,
        );
        if ergebnis.fingerabdruck != fingerabdruck {
            return Err(anyhow!("ki_freigabe_veraltet"));
        }
        if einstufung == Einstufung::Intern && ergebnis.einstufung != Einstufung::Intern {
            return Err(anyhow!("ki_freigabe_nicht_intern"));
        }
        tx.execute(
            "INSERT OR REPLACE INTO meeting_ai_confirmations
               (meeting_id, einstufung, fingerabdruck, fp_version, informiert, confirmed_at,
                rechtsgrundlage, einwilligungen_belegt)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7)",
            params![
                meeting_id,
                einstufung_text(einstufung),
                fingerabdruck,
                i64::from(FINGERABDRUCK_VERSION),
                now_ms,
                grundlage.text(),
                nachweis.einwilligungen_liegen_vor && grundlage == Rechtsgrundlage::Einwilligung,
            ],
        )?;
        tx.commit()?;
    }
    stand(store, sperre, meeting_id)
}

#[cfg(test)]
#[path = "ki_freigabe_tests.rs"]
mod tests;
