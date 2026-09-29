//! Personen (M5, P5d; `entwurf/m5-m6-kalender-export.md` F17): `humans` wird die
//! Personentabelle. Quellen sind die Teilnehmenden eines Kalendertermins, die
//! benannten Sprecher (`speakers.display_name`, M3) und die manuelle Pflege.
//!
//! Abgleich, in dieser Reihenfolge: E-Mail exakt -> sonst Alias-Name exakt
//! (normalisiert, siehe [`normalize`]) -> sonst neue Person. Keine unscharfe
//! Automatik: eine Fehlzusammenfuehrung waere still. „Zusammenfuehren“ ist eine
//! manuelle Handlung ([`MeetingStore::merge_people`]).
//!
//! `meeting_participants` haelt die Zuordnung Besprechung <-> Person dauerhaft
//! (auch wenn der Termin aus dem Kalender-Cache faellt). Sie geht mit der
//! Besprechung (`soft_delete_meeting`); die Person bleibt.
//!
//! Alles hier ist Store-Logik ohne Tauri und ohne Einstellungen: „Ich“
//! (`meeting_self_emails`) kommt als Parameter herein. Kein Name, keine Adresse
//! landet im Log oder in einer Fehlermeldung.

pub mod brief;
pub mod normalize;

use std::collections::HashSet;

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use ulid::Ulid;

use crate::managers::calendar::model::CalEvent;
use crate::managers::meetings::store::MeetingStore;

use normalize::{
    company_from_email, display_name, name_from_email, normalize_email, normalize_name,
};

pub use brief::{brief_scope, Brief, MAX_BRIEF_MEETINGS};

/// Rollen in `meeting_participants.role`.
pub const ROLES: [&str; 3] = ["organizer", "attendee", "speaker"];
/// Quellen in `meeting_participants.source`.
pub const SOURCES: [&str; 3] = ["calendar", "speaker", "manual"];

/// Die letzten Besprechungen im Detail einer Person.
const RECENT_MEETINGS: u32 = 10;
/// Groesste Personenliste, die ein Aufruf liefert.
const MAX_LIST: usize = 1_000;

/// Eine Person in der Liste („Personen verwalten“, Filter).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PersonSummary {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub company: Option<String>,
    /// „Ich“: Kennzeichen an der Person oder Adresse aus „Meine E-Mail-Adressen“.
    pub is_self: bool,
    /// Lebende Besprechungen, an denen die Person teilnahm.
    pub meeting_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PersonMeeting {
    pub id: String,
    pub title: String,
    pub started_at: Option<i64>,
}

/// Eine Person mit allem fuer das Popover.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PersonDetail {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub company: Option<String>,
    pub is_self: bool,
    pub meeting_count: u32,
    /// Weitere Adressen, die nach einem Zusammenfuehren zur Person gehoeren.
    pub other_emails: Vec<String>,
    /// Die juengsten Besprechungen mit der Person, neueste zuerst.
    pub recent_meetings: Vec<PersonMeeting>,
}

/// Eine teilnehmende Person einer Besprechung.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Participant {
    pub human_id: String,
    pub name: String,
    pub email: Option<String>,
    pub company: Option<String>,
    /// `organizer`, `attendee` oder `speaker`.
    pub role: String,
    /// `calendar`, `speaker` oder `manual`.
    pub source: String,
    pub is_self: bool,
    pub meeting_count: u32,
}

/// Eine gefundene Person (lebend, nicht zusammengefuehrt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundPerson {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub is_self: bool,
}

impl FoundPerson {
    /// „Ich“ nach Kennzeichen oder nach den eigenen Adressen.
    pub fn is_me(&self, own: &HashSet<String>) -> bool {
        self.is_self || self.email.as_ref().is_some_and(|e| own.contains(e))
    }
}

/// Die eigenen Adressen in Vergleichsform (normalisiert, ohne Unbrauchbares).
pub fn own_set(self_emails: &[String]) -> HashSet<String> {
    self_emails
        .iter()
        .filter_map(|e| normalize_email(e))
        .collect()
}

fn now_secs() -> i64 {
    Utc::now().timestamp()
}

// SQL-Rangfolge: ein staerkerer Eintrag ueberschreibt einen schwaecheren, nie umgekehrt.
const ROLE_RANK: &str = "(CASE {c} WHEN 'organizer' THEN 3 WHEN 'attendee' THEN 2 ELSE 1 END)";
const SOURCE_RANK: &str = "(CASE {c} WHEN 'manual' THEN 3 WHEN 'calendar' THEN 2 ELSE 1 END)";

fn upsert_participant_sql() -> String {
    format!(
        "INSERT INTO meeting_participants (meeting_id, human_id, role, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(meeting_id, human_id) DO UPDATE SET
           role = CASE WHEN {er} > {rr} THEN excluded.role ELSE role END,
           source = CASE WHEN {es} > {rs} THEN excluded.source ELSE source END",
        er = ROLE_RANK.replace("{c}", "excluded.role"),
        rr = ROLE_RANK.replace("{c}", "role"),
        es = SOURCE_RANK.replace("{c}", "excluded.source"),
        rs = SOURCE_RANK.replace("{c}", "source"),
    )
}

fn human_by_id(conn: &Connection, id: &str) -> Result<Option<FoundPerson>> {
    Ok(conn
        .query_row(
            "SELECT id, name, email_norm, is_self FROM humans
             WHERE id = ?1 AND deleted_at IS NULL AND merged_into IS NULL",
            params![id],
            |r| {
                Ok(FoundPerson {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    email: r.get(2)?,
                    is_self: r.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()?)
}

/// Wie eine Person gefunden wurde.
enum Match {
    ByEmail(FoundPerson),
    ByName(FoundPerson),
    None,
}

/// Abgleich ohne Schreiben: E-Mail exakt (Person oder Alias), sonst Namens-Alias.
fn lookup(conn: &Connection, email: Option<&str>, name_key: &str) -> Result<Match> {
    if let Some(email) = email {
        let by_email: Option<String> = conn
            .query_row(
                "SELECT id FROM humans WHERE email_norm = ?1
                 AND deleted_at IS NULL AND merged_into IS NULL",
                params![email],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(found) = by_email.and_then(|id| human_by_id(conn, &id).transpose()) {
            return Ok(Match::ByEmail(found?));
        }
        let alias: Option<String> = conn
            .query_row(
                "SELECT human_id FROM human_aliases WHERE kind = 'email' AND value_norm = ?1",
                params![email],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(found) = alias.and_then(|id| human_by_id(conn, &id).transpose()) {
            return Ok(Match::ByEmail(found?));
        }
    }
    if !name_key.is_empty() {
        let alias: Option<String> = conn
            .query_row(
                "SELECT human_id FROM human_aliases WHERE kind = 'name' AND value_norm = ?1",
                params![name_key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(found) = alias.and_then(|id| human_by_id(conn, &id).transpose()) {
            return Ok(Match::ByName(found?));
        }
    }
    Ok(Match::None)
}

fn add_alias(conn: &Connection, kind: &str, value_norm: &str, human_id: &str) -> Result<()> {
    if value_norm.is_empty() {
        return Ok(());
    }
    // Der Schluessel gehoert dem, der ihn zuerst hatte.
    conn.execute(
        "INSERT OR IGNORE INTO human_aliases (kind, value_norm, human_id) VALUES (?1, ?2, ?3)",
        params![kind, value_norm, human_id],
    )?;
    Ok(())
}

fn validate_source(source: &str) -> Result<()> {
    if SOURCES.contains(&source) {
        Ok(())
    } else {
        Err(anyhow!("person_invalid:source"))
    }
}

fn is_valid_role(role: &str) -> bool {
    ROLES.contains(&role)
}

/// Eine Person zu Name und/oder Adresse: gefunden oder angelegt (siehe Modul).
fn upsert_in(conn: &Connection, email: Option<&str>, name: Option<&str>) -> Result<String> {
    let email = email.and_then(normalize_email);
    let shown = name
        .and_then(display_name)
        .or_else(|| email.as_deref().and_then(name_from_email));
    let Some(shown) = shown else {
        return Err(anyhow!("person_invalid"));
    };
    let key = normalize_name(&shown);
    let company = email.as_deref().and_then(company_from_email);
    let now = now_secs();

    match lookup(conn, email.as_deref(), &key)? {
        Match::ByEmail(found) => {
            add_alias(conn, "name", &key, &found.id)?;
            if let Some(company) = &company {
                conn.execute(
                    "UPDATE humans SET company = ?1, updated_at = ?2
                     WHERE id = ?3 AND company IS NULL",
                    params![company, now, found.id],
                )?;
            }
            return Ok(found.id);
        }
        Match::ByName(found) => match (&email, &found.email) {
            // Die Person war nur mit Namen bekannt: die Adresse kommt dazu.
            (Some(email), None) => {
                conn.execute(
                    "UPDATE humans SET email_norm = ?1, company = COALESCE(company, ?2),
                            updated_at = ?3 WHERE id = ?4",
                    params![email, company, now, found.id],
                )?;
                return Ok(found.id);
            }
            // Gleicher Name, andere Adresse: keine Annahme, dass es dieselbe Person ist.
            (Some(_), Some(_)) => {}
            (None, _) => return Ok(found.id),
        },
        Match::None => {}
    }

    let id = Ulid::new().to_string();
    conn.execute(
        "INSERT INTO humans (id, name, email_norm, company, is_self, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
        params![id, shown, email, company, now],
    )?;
    add_alias(conn, "name", &key, &id)?;
    Ok(id)
}

impl MeetingStore {
    /// Die Person zu dieser Adresse und/oder diesem Namen; wird angelegt, wenn es
    /// sie noch nicht gibt. `source`: `calendar`, `speaker` oder `manual`.
    /// Fehler: `person_invalid` (weder brauchbare Adresse noch Name),
    /// `person_invalid:source`.
    pub fn upsert_person(
        &self,
        email: Option<&str>,
        name: Option<&str>,
        source: &str,
    ) -> Result<String> {
        validate_source(source)?;
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let id = upsert_in(&tx, email, name)?;
        tx.commit()?;
        Ok(id)
    }

    /// Wie `upsert_person`, aber ohne Anlegen und ohne Schreiben: die schon
    /// bekannte Person oder `None` (Brief, Filter).
    pub fn find_person(
        &self,
        email: Option<&str>,
        name: Option<&str>,
    ) -> Result<Option<FoundPerson>> {
        let conn = self.get_connection()?;
        let email = email.and_then(normalize_email);
        let key = name.map(normalize_name).unwrap_or_default();
        Ok(match lookup(&conn, email.as_deref(), &key)? {
            Match::ByEmail(f) => Some(f),
            // Mit Adresse gesucht und eine andere gefunden: nicht dieselbe Person.
            Match::ByName(f) => match (&email, &f.email) {
                (Some(_), Some(_)) => None,
                _ => Some(f),
            },
            Match::None => None,
        })
    }

    /// Kennzeichnet eine Person als „Ich“ (z. B. aus dem eigenen Konto des Kalenders).
    pub fn mark_person_self(&self, human_id: &str) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE humans SET is_self = 1, updated_at = ?1 WHERE id = ?2",
            params![now_secs(), human_id],
        )?;
        Ok(())
    }

    /// Setzt die Teilnehmenden EINER Quelle (`calendar`, `speaker`, `manual`) fuer
    /// eine Besprechung: Zeilen dieser Quelle, die in `rows` fehlen, entfallen,
    /// die anderen kommen dazu oder werden gestaerkt. Zeilen anderer Quellen
    /// bleiben; eine Person, die schon stärker eingetragen ist (Kalender vor
    /// Sprecher, Organisator vor Teilnehmer), behaelt ihre Zeile.
    /// `rows`: `(human_id, role)`.
    pub fn set_participants(
        &self,
        meeting_id: &str,
        source: &str,
        rows: &[(String, &str)],
    ) -> Result<()> {
        validate_source(source)?;
        if rows.iter().any(|(_, role)| !is_valid_role(role)) {
            return Err(anyhow!("person_invalid:role"));
        }
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::ensure_meeting_is_live(&tx, meeting_id)?;
        let keep: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
        tx.execute(
            "DELETE FROM meeting_participants
             WHERE meeting_id = ?1 AND source = ?2
               AND human_id NOT IN (SELECT value FROM json_each(?3))",
            params![meeting_id, source, serde_json::to_string(&keep)?],
        )?;
        let sql = upsert_participant_sql();
        let now = now_secs();
        for (human_id, role) in rows {
            tx.execute(&sql, params![meeting_id, human_id, role, source, now])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Teilnehmende einer Besprechung: Organisator zuerst, dann nach Rolle und
    /// Name. Zusammengefuehrte und geloeschte Personen kommen nicht vor.
    pub fn participants_of(
        &self,
        meeting_id: &str,
        self_emails: &[String],
    ) -> Result<Vec<Participant>> {
        let own = own_set(self_emails);
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT h.id, h.name, h.email_norm, h.company, h.is_self, p.role, p.source,
                    ({COUNT_SQL})
             FROM meeting_participants p
             JOIN humans h ON h.id = p.human_id
              AND h.deleted_at IS NULL AND h.merged_into IS NULL
             WHERE p.meeting_id = ?1"
        ))?;
        let mut out = stmt
            .query_map(params![meeting_id], |r| {
                let email: Option<String> = r.get(2)?;
                let is_self =
                    r.get::<_, i64>(4)? != 0 || email.as_ref().is_some_and(|e| own.contains(e));
                Ok(Participant {
                    human_id: r.get(0)?,
                    name: r.get(1)?,
                    email,
                    company: r.get(3)?,
                    is_self,
                    role: r.get(5)?,
                    source: r.get(6)?,
                    meeting_count: r.get::<_, i64>(7)?.max(0) as u32,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rank = |role: &str| match role {
            "organizer" => 0,
            "attendee" => 1,
            _ => 2,
        };
        out.sort_by(|a, b| {
            rank(&a.role)
                .cmp(&rank(&b.role))
                .then_with(|| normalize_name(&a.name).cmp(&normalize_name(&b.name)))
                .then_with(|| a.human_id.cmp(&b.human_id))
        });
        Ok(out)
    }

    /// Fertige, lebende Besprechungen mit mindestens einer der Personen, neueste
    /// zuerst, hoechstens `limit`.
    pub fn meetings_with_people(&self, human_ids: &[String], limit: u32) -> Result<Vec<String>> {
        let conn = self.get_connection()?;
        Ok(shared_meetings(&conn, human_ids, limit, None)?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
    }

    /// Alle Personen (nicht zusammengefuehrt), die meisten Besprechungen zuerst.
    /// `query` filtert nach Name (Namensschluessel), Adresse oder Firma.
    pub fn list_people(
        &self,
        query: Option<&str>,
        self_emails: &[String],
    ) -> Result<Vec<PersonSummary>> {
        let own = own_set(self_emails);
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT h.id, h.name, COALESCE(h.email_norm, h.email), h.company, h.is_self,
                    ({COUNT_SQL})
             FROM humans h WHERE h.deleted_at IS NULL AND h.merged_into IS NULL"
        ))?;
        let mut people = stmt
            .query_map([], |r| {
                let email: Option<String> = r.get(2)?;
                let is_self = r.get::<_, i64>(4)? != 0
                    || email
                        .as_deref()
                        .and_then(normalize_email)
                        .is_some_and(|e| own.contains(&e));
                Ok(PersonSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    email,
                    company: r.get(3)?,
                    is_self,
                    meeting_count: r.get::<_, i64>(5)?.max(0) as u32,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            let key = normalize_name(q);
            let plain = q.to_lowercase();
            people.retain(|p| {
                (!key.is_empty() && normalize_name(&p.name).contains(&key))
                    || p.email.as_deref().is_some_and(|e| e.contains(&plain))
                    || p.company.as_deref().is_some_and(|c| c.contains(&plain))
            });
        }
        people.sort_by(|a, b| {
            b.meeting_count
                .cmp(&a.meeting_count)
                .then_with(|| normalize_name(&a.name).cmp(&normalize_name(&b.name)))
                .then_with(|| a.id.cmp(&b.id))
        });
        people.truncate(MAX_LIST);
        Ok(people)
    }

    /// Eine Person mit den juengsten Besprechungen; `None`, wenn es sie nicht
    /// (mehr) gibt.
    pub fn get_person(&self, id: &str, self_emails: &[String]) -> Result<Option<PersonDetail>> {
        let own = own_set(self_emails);
        let conn = self.get_connection()?;
        let Some(found) = human_by_id(&conn, id)? else {
            return Ok(None);
        };
        let (company, meeting_count): (Option<String>, i64) = conn.query_row(
            &format!("SELECT h.company, ({COUNT_SQL}) FROM humans h WHERE h.id = ?1"),
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut stmt = conn.prepare(
            "SELECT value_norm FROM human_aliases WHERE kind = 'email' AND human_id = ?1
             ORDER BY value_norm",
        )?;
        let other_emails = stmt
            .query_map(params![id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.title, m.started_at FROM meeting_participants p
             JOIN meetings m ON m.id = p.meeting_id AND m.deleted_at IS NULL
             WHERE p.human_id = ?1
             ORDER BY COALESCE(m.started_at, m.created_at) DESC, m.id
             LIMIT ?2",
        )?;
        let recent_meetings = stmt
            .query_map(params![id, RECENT_MEETINGS], |r| {
                Ok(PersonMeeting {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    started_at: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let is_self = found.is_me(&own);
        Ok(Some(PersonDetail {
            id: found.id,
            name: found.name,
            email: found.email,
            company,
            is_self,
            meeting_count: meeting_count.max(0) as u32,
            other_emails,
            recent_meetings,
        }))
    }

    /// Benennt eine Person um und aendert (oder loescht) ihre Adresse.
    /// `email`: `None` = unveraendert, leer = Adresse entfernen. Der alte Name
    /// bleibt als Alias erhalten. Fehler: `person_not_found`,
    /// `person_name_invalid`, `person_email_invalid`, `person_email_taken`
    /// (die Adresse gehoert einer anderen Person: zusammenfuehren).
    pub fn update_person(&self, id: &str, name: &str, email: Option<&str>) -> Result<()> {
        let Some(shown) = display_name(name) else {
            return Err(anyhow!("person_name_invalid"));
        };
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(found) = human_by_id(&tx, id)? else {
            return Err(anyhow!("person_not_found"));
        };
        let now = now_secs();
        // (neue Adresse, neue Firma) oder unveraendert.
        let new_email: Option<(Option<String>, Option<String>)> = match email {
            None => None,
            Some(raw) if raw.trim().is_empty() => Some((None, None)),
            Some(raw) => {
                let Some(norm) = normalize_email(raw) else {
                    return Err(anyhow!("person_email_invalid"));
                };
                let taken: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM humans WHERE email_norm = ?1 AND id <> ?2
                                   AND deleted_at IS NULL AND merged_into IS NULL)",
                    params![norm, id],
                    |r| r.get(0),
                )?;
                if taken {
                    return Err(anyhow!("person_email_taken"));
                }
                let company = company_from_email(&norm);
                Some((Some(norm), company))
            }
        };
        match new_email {
            Some((email, company)) if email != found.email => {
                tx.execute(
                    "UPDATE humans SET name = ?1, email_norm = ?2, company = ?3, updated_at = ?4
                     WHERE id = ?5",
                    params![shown, email, company, now, id],
                )?;
            }
            _ => {
                tx.execute(
                    "UPDATE humans SET name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![shown, now, id],
                )?;
            }
        }
        add_alias(&tx, "name", &normalize_name(&shown), id)?;
        tx.commit()?;
        Ok(())
    }

    /// Fuehrt `gone` in `keep` zusammen (eine Transaktion): Teilnahmen,
    /// Aliasse, Sprecherzuordnungen und Aufgaben gehen an `keep`, `gone` wird als
    /// zusammengefuehrt markiert (und gibt seine Adresse frei; sie bleibt als
    /// Alias von `keep`). Fehler: `person_not_found`, `person_merge_invalid`
    /// (gleiche Person).
    pub fn merge_people(&self, keep: &str, gone: &str) -> Result<()> {
        if keep == gone {
            return Err(anyhow!("person_merge_invalid"));
        }
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (Some(kept), Some(other)) = (human_by_id(&tx, keep)?, human_by_id(&tx, gone)?) else {
            return Err(anyhow!("person_not_found"));
        };
        let now = now_secs();
        let other_company: Option<String> = tx.query_row(
            "SELECT company FROM humans WHERE id = ?1",
            params![gone],
            |r| r.get(0),
        )?;

        // Teilnahmen: dieselbe Besprechung bei beiden -> die staerkere Zeile zaehlt.
        let rows: Vec<(String, String, String, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT meeting_id, role, source, created_at FROM meeting_participants
                 WHERE human_id = ?1",
            )?;
            let rows = stmt
                .query_map(params![gone], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        let sql = upsert_participant_sql();
        for (meeting_id, role, source, created_at) in &rows {
            tx.execute(&sql, params![meeting_id, keep, role, source, created_at])?;
        }
        tx.execute(
            "DELETE FROM meeting_participants WHERE human_id = ?1",
            params![gone],
        )?;

        // Aliasse und Verweise.
        tx.execute(
            "UPDATE human_aliases SET human_id = ?1 WHERE human_id = ?2",
            params![keep, gone],
        )?;
        add_alias(&tx, "name", &normalize_name(&other.name), keep)?;
        tx.execute(
            "UPDATE speakers SET human_id = ?1 WHERE human_id = ?2",
            params![keep, gone],
        )?;
        tx.execute(
            "UPDATE action_items SET assignee_human_id = ?1 WHERE assignee_human_id = ?2",
            params![keep, gone],
        )?;
        let has_voiceprints: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'voiceprints')",
            [],
            |r| r.get(0),
        )?;
        if has_voiceprints {
            tx.execute(
                "UPDATE voiceprints SET human_id = ?1 WHERE human_id = ?2",
                params![keep, gone],
            )?;
        }
        // Wer schon in `gone` aufging, gehoert jetzt `keep`.
        tx.execute(
            "UPDATE humans SET merged_into = ?1, updated_at = ?2 WHERE merged_into = ?3",
            params![keep, now, gone],
        )?;

        // `gone` gibt die Adresse frei (eindeutiger Index) und markiert sich.
        tx.execute(
            "UPDATE humans SET email_norm = NULL, merged_into = ?1, deleted_at = ?2, updated_at = ?2
             WHERE id = ?3",
            params![keep, now, gone],
        )?;
        if let Some(email) = &other.email {
            add_alias(&tx, "email", email, keep)?;
        }
        let keep_email = kept.email.clone().or_else(|| other.email.clone());
        tx.execute(
            "UPDATE humans SET email_norm = ?1, company = COALESCE(company, ?2),
                    is_self = MAX(is_self, ?3), updated_at = ?4
             WHERE id = ?5",
            params![
                keep_email,
                other_company,
                i64::from(other.is_self),
                now,
                keep
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Zaehlt die lebenden Besprechungen einer Person (`h` = Zeile in `humans`).
const COUNT_SQL: &str = "SELECT COUNT(*) FROM meeting_participants cp
     JOIN meetings cm ON cm.id = cp.meeting_id AND cm.deleted_at IS NULL
     WHERE cp.human_id = h.id";

/// Fertige, lebende Besprechungen mit mindestens einer der Personen, neueste
/// zuerst, als `(id, Zeitpunkt in s)`. `exclude_event_key`: Besprechungen, die
/// zu genau diesem Termin verknuepft sind, zaehlen nicht (sie sind nicht
/// „frueher“).
pub(crate) fn shared_meetings(
    conn: &Connection,
    human_ids: &[String],
    limit: u32,
    exclude_event_key: Option<&str>,
) -> Result<Vec<(String, i64)>> {
    if human_ids.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let mut params: Vec<Value> = vec![
        Value::Text(serde_json::to_string(human_ids)?),
        Value::Integer(i64::from(limit)),
    ];
    let mut exclude = String::new();
    if let Some(key) = exclude_event_key {
        params.push(Value::Text(key.to_string()));
        exclude = " AND m.id NOT IN (SELECT meeting_id FROM meeting_calendar_links
                                     WHERE event_key = ?3)"
            .to_string();
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT m.id, COALESCE(m.started_at, m.created_at)
         FROM meetings m
         WHERE m.deleted_at IS NULL AND m.status = 'ready'
           AND m.id IN (SELECT meeting_id FROM meeting_participants
                        WHERE human_id IN (SELECT value FROM json_each(?1))){exclude}
         ORDER BY COALESCE(m.started_at, m.created_at) DESC, m.id DESC
         LIMIT ?2"
    ))?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Die Teilnehmenden eines Termins werden Personen und Teilnehmende der
/// Besprechung (Quelle `calendar`). Attendees ohne Adresse und ohne Namen
/// entfallen. Aufgerufen nach dem Start aus einem Termin (`finish_start`).
pub fn participants_from_event(
    store: &MeetingStore,
    meeting_id: &str,
    event: &CalEvent,
) -> Result<usize> {
    let mut rows: Vec<(String, &'static str)> = Vec::new();
    for attendee in &event.attendees {
        let email = attendee.email.as_deref().and_then(normalize_email);
        let named = attendee.name.as_deref().and_then(display_name);
        if email.is_none() && named.is_none() {
            continue;
        }
        let id = store.upsert_person(email.as_deref(), named.as_deref(), "calendar")?;
        if attendee.is_self {
            store.mark_person_self(&id)?;
        }
        let role = if attendee.organizer {
            "organizer"
        } else {
            "attendee"
        };
        match rows.iter_mut().find(|(existing, _)| *existing == id) {
            Some((_, existing_role)) => {
                if role == "organizer" {
                    *existing_role = role;
                }
            }
            None => rows.push((id, role)),
        }
    }
    store.set_participants(meeting_id, "calendar", &rows)?;
    Ok(rows.len())
}

/// Benannte Sprecher werden Personen (`speakers.human_id`) und Teilnehmende der
/// Besprechung (Quelle `speaker`). Ein entfernter Name nimmt die Person wieder
/// aus den Teilnehmenden (die Person selbst bleibt). Aufgerufen nach Benennen
/// und Zusammenfuehren von Sprechern.
pub fn sync_speaker_participants(store: &MeetingStore, meeting_id: &str) -> Result<usize> {
    let named: Vec<(String, String)> = {
        let conn = store.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, display_name FROM speakers
             WHERE meeting_id = ?1 AND deleted_at IS NULL AND display_name IS NOT NULL
               AND TRIM(display_name) <> ''
             ORDER BY channel, speaker_index, created_at",
        )?;
        let rows = stmt
            .query_map(params![meeting_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    let mut rows: Vec<(String, &'static str)> = Vec::new();
    for (speaker_id, name) in &named {
        let human_id = store.upsert_person(None, Some(name), "speaker")?;
        {
            let conn = store.get_connection()?;
            conn.execute(
                "UPDATE speakers SET human_id = ?1 WHERE id = ?2",
                params![human_id, speaker_id],
            )?;
        }
        if !rows.iter().any(|(id, _)| *id == human_id) {
            rows.push((human_id, "speaker"));
        }
    }
    {
        // Sprecher ohne Namen verweisen auf niemanden.
        let conn = store.get_connection()?;
        conn.execute(
            "UPDATE speakers SET human_id = NULL
             WHERE meeting_id = ?1 AND deleted_at IS NULL
               AND (display_name IS NULL OR TRIM(display_name) = '')
               AND human_id IS NOT NULL",
            params![meeting_id],
        )?;
    }
    store.set_participants(meeting_id, "speaker", &rows)?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::Attendee;
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus};

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

    fn attendee(email: Option<&str>, name: Option<&str>, organizer: bool) -> Attendee {
        Attendee {
            email: email.map(str::to_string),
            name: name.map(str::to_string),
            organizer,
            is_self: false,
            partstat: None,
        }
    }

    fn event(uid: &str, start: i64, attendees: Vec<Attendee>) -> CalEvent {
        CalEvent {
            key: format!("s:{uid}:{start}"),
            source_id: "s".into(),
            uid: uid.into(),
            title: "Jour fixe".into(),
            starts_at: start,
            ends_at: start + 3_600_000,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees,
        }
    }

    fn human_count(s: &MeetingStore) -> i64 {
        s.get_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM humans", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn last_comma_first_and_first_last_are_the_same_person() {
        let (_d, s) = tmp_store();
        let a = s
            .upsert_person(None, Some("Berg, Anna"), "speaker")
            .unwrap();
        let b = s.upsert_person(None, Some("Anna Berg"), "speaker").unwrap();
        let c = s
            .upsert_person(None, Some("  anna   BERG "), "manual")
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(human_count(&s), 1);
        let p = s.get_person(&a, &[]).unwrap().unwrap();
        assert_eq!(p.name, "Anna Berg", "gespeichert wird die Anzeigeform");
    }

    #[test]
    fn email_wins_over_name() {
        let (_d, s) = tmp_store();
        let first = s
            .upsert_person(Some("Anna@Firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        // Andere Schreibweise des Namens, gleiche Adresse: dieselbe Person,
        // und der neue Name wird als Alias gemerkt.
        let again = s
            .upsert_person(Some("anna@firma.de"), Some("A. Berg"), "calendar")
            .unwrap();
        assert_eq!(first, again);
        let by_alias = s.upsert_person(None, Some("A. Berg"), "speaker").unwrap();
        assert_eq!(first, by_alias);
        // Gleicher Name, ANDERE Adresse: nicht raten, eine zweite Person.
        let other = s
            .upsert_person(Some("anna.berg@andere.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        assert_ne!(first, other);
        assert_eq!(human_count(&s), 2);
        // Die Adresse allein findet die erste Person.
        assert_eq!(
            s.find_person(Some("ANNA@firma.de"), None)
                .unwrap()
                .map(|f| f.id),
            Some(first)
        );
    }

    #[test]
    fn a_person_known_by_name_gets_the_address_later() {
        let (_d, s) = tmp_store();
        let speaker = s.upsert_person(None, Some("Anna Berg"), "speaker").unwrap();
        let calendar = s
            .upsert_person(Some("anna@firma.de"), Some("Berg, Anna"), "calendar")
            .unwrap();
        assert_eq!(speaker, calendar);
        let p = s.get_person(&speaker, &[]).unwrap().unwrap();
        assert_eq!(p.email.as_deref(), Some("anna@firma.de"));
        assert_eq!(p.company.as_deref(), Some("firma.de"));
    }

    #[test]
    fn freemail_addresses_have_no_company_and_names_come_from_the_address() {
        let (_d, s) = tmp_store();
        let id = s
            .upsert_person(Some("clara.holm@gmail.com"), None, "calendar")
            .unwrap();
        let p = s.get_person(&id, &[]).unwrap().unwrap();
        assert_eq!(p.company, None);
        assert_eq!(p.name, "Clara Holm");
        let firm = s
            .upsert_person(Some("ben@kunde-ag.de"), Some("Ben Kunde"), "calendar")
            .unwrap();
        assert_eq!(
            s.get_person(&firm, &[])
                .unwrap()
                .unwrap()
                .company
                .as_deref(),
            Some("kunde-ag.de")
        );
    }

    #[test]
    fn unusable_input_is_refused_and_bad_sources_too() {
        let (_d, s) = tmp_store();
        assert_eq!(
            s.upsert_person(None, None, "manual")
                .unwrap_err()
                .to_string(),
            "person_invalid"
        );
        assert_eq!(
            s.upsert_person(Some("kaputt"), Some("  "), "manual")
                .unwrap_err()
                .to_string(),
            "person_invalid"
        );
        // Eine kaputte Adresse mit brauchbarem Namen zaehlt als Name allein.
        assert!(s
            .upsert_person(Some("kaputt"), Some("Ben Alt"), "manual")
            .is_ok());
        assert_eq!(
            s.upsert_person(None, Some("X"), "irgendwas")
                .unwrap_err()
                .to_string(),
            "person_invalid:source"
        );
        assert_eq!(human_count(&s), 1);
    }

    #[test]
    fn merging_moves_participations_aliases_and_frees_the_address() {
        let (_d, s) = tmp_store();
        let m1 = ready(&s, "Eins", 100);
        let m2 = ready(&s, "Zwei", 200);
        let m3 = ready(&s, "Drei", 300);
        let keep = s
            .upsert_person(Some("anna@firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        let gone = s
            .upsert_person(Some("a.berg@privat.de"), Some("Anni"), "calendar")
            .unwrap();
        s.set_participants(&m1, "calendar", &[(keep.clone(), "organizer")])
            .unwrap();
        s.set_participants(&m2, "calendar", &[(gone.clone(), "attendee")])
            .unwrap();
        // m3: beide sind eingetragen; nach dem Zusammenfuehren bleibt EINE Zeile.
        s.set_participants(
            &m3,
            "calendar",
            &[(keep.clone(), "attendee"), (gone.clone(), "organizer")],
        )
        .unwrap();

        s.merge_people(&keep, &gone).unwrap();

        let count = |human: &str, meeting: &str| -> i64 {
            s.get_connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM meeting_participants
                     WHERE human_id = ?1 AND meeting_id = ?2",
                    params![human, meeting],
                    |r| r.get(0),
                )
                .unwrap()
        };
        for m in [&m1, &m2, &m3] {
            assert_eq!(count(&keep, m), 1, "{m}");
            assert_eq!(count(&gone, m), 0, "{m}");
        }
        let in_m3 = s.participants_of(&m3, &[]).unwrap();
        assert_eq!(in_m3.len(), 1);
        assert_eq!(in_m3[0].role, "organizer", "die staerkere Rolle bleibt");
        // Die Person `gone` ist weg, `keep` zaehlt alle drei Besprechungen.
        let people = s.list_people(None, &[]).unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].id, keep);
        assert_eq!(people[0].meeting_count, 3);
        assert!(s.get_person(&gone, &[]).unwrap().is_none());
        // Die alten Schluessel fuehren zu `keep`: Adresse und Name.
        assert_eq!(
            s.upsert_person(Some("a.berg@privat.de"), None, "calendar")
                .unwrap(),
            keep
        );
        assert_eq!(
            s.upsert_person(None, Some("Anni"), "speaker").unwrap(),
            keep
        );
        let detail = s.get_person(&keep, &[]).unwrap().unwrap();
        assert_eq!(detail.other_emails, vec!["a.berg@privat.de".to_string()]);
        assert_eq!(
            human_count(&s),
            2,
            "die Zeile von `gone` bleibt als Verweis"
        );
    }

    #[test]
    fn merging_takes_over_speakers_and_the_address_when_the_keeper_has_none() {
        let (_d, s) = tmp_store();
        let m = ready(&s, "Rund", 100);
        let keep = s.upsert_person(None, Some("Anna"), "speaker").unwrap();
        let gone = s
            .upsert_person(Some("anna@firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "INSERT INTO speakers (id, meeting_id, channel, speaker_index, display_name,
                     human_id, created_at, updated_at) VALUES ('S1', ?1, 1, 1, 'Anna Berg', ?2, 1, 1)",
                params![m, gone],
            )
            .unwrap();
        s.merge_people(&keep, &gone).unwrap();
        let human: String = s
            .get_connection()
            .unwrap()
            .query_row("SELECT human_id FROM speakers WHERE id = 'S1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(human, keep);
        let p = s.get_person(&keep, &[]).unwrap().unwrap();
        assert_eq!(p.email.as_deref(), Some("anna@firma.de"));
        assert_eq!(p.company.as_deref(), Some("firma.de"));
        // Sich selbst oder Unbekanntes zusammenzufuehren ist ein Fehler.
        assert_eq!(
            s.merge_people(&keep, &keep).unwrap_err().to_string(),
            "person_merge_invalid"
        );
        assert_eq!(
            s.merge_people(&keep, &gone).unwrap_err().to_string(),
            "person_not_found",
            "`gone` ist schon aufgegangen"
        );
    }

    #[test]
    fn a_deleted_meeting_disappears_from_people_and_from_shared_meetings() {
        let (_d, s) = tmp_store();
        let keep_m = ready(&s, "Bleibt", 100);
        let gone_m = ready(&s, "Weg", 200);
        let anna = s
            .upsert_person(Some("anna@firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        for m in [&keep_m, &gone_m] {
            s.set_participants(m, "calendar", &[(anna.clone(), "attendee")])
                .unwrap();
        }
        assert_eq!(
            s.meetings_with_people(std::slice::from_ref(&anna), 20)
                .unwrap(),
            vec![gone_m.clone(), keep_m.clone()],
            "neueste zuerst"
        );
        s.soft_delete_meeting(&gone_m).unwrap();
        assert_eq!(
            s.meetings_with_people(std::slice::from_ref(&anna), 20)
                .unwrap(),
            vec![keep_m.clone()]
        );
        assert!(s.participants_of(&gone_m, &[]).unwrap().is_empty());
        let p = s.get_person(&anna, &[]).unwrap().unwrap();
        assert_eq!(p.meeting_count, 1);
        assert_eq!(p.recent_meetings.len(), 1);
        assert_eq!(p.recent_meetings[0].id, keep_m);
        // Die Person bleibt, auch ohne Besprechung.
        s.soft_delete_meeting(&keep_m).unwrap();
        let p = s.get_person(&anna, &[]).unwrap().unwrap();
        assert_eq!(p.meeting_count, 0);
        assert!(s.meetings_with_people(&[anna], 20).unwrap().is_empty());
    }

    #[test]
    fn calendar_participants_are_stored_with_roles_and_deduplicated() {
        let (_d, s) = tmp_store();
        let m = ready(&s, "Jour fixe", 100);
        let mut me = attendee(Some("ich@wolff.de"), Some("Patrick Wolff"), false);
        me.is_self = true;
        let ev = event(
            "u1",
            1_000_000,
            vec![
                attendee(Some("anna@firma.de"), Some("Berg, Anna"), true),
                attendee(Some("ANNA@firma.de"), None, false),
                attendee(Some("bernd@firma.de"), Some("Bernd Alt"), false),
                attendee(None, Some("Nur Name"), false),
                attendee(None, None, false),
                me,
            ],
        );
        assert_eq!(participants_from_event(&s, &m, &ev).unwrap(), 4);
        let list = s.participants_of(&m, &[]).unwrap();
        let names: Vec<(&str, &str, bool)> = list
            .iter()
            .map(|p| (p.name.as_str(), p.role.as_str(), p.is_self))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Anna Berg", "organizer", false),
                ("Bernd Alt", "attendee", false),
                ("Nur Name", "attendee", false),
                ("Patrick Wolff", "attendee", true),
            ]
        );
        // Erneut mit weniger Teilnehmenden: der Rest entfaellt.
        let ev2 = event(
            "u1",
            1_000_000,
            vec![attendee(Some("anna@firma.de"), None, true)],
        );
        assert_eq!(participants_from_event(&s, &m, &ev2).unwrap(), 1);
        assert_eq!(s.participants_of(&m, &[]).unwrap().len(), 1);
        // Eine unbekannte Besprechung ist ein Fehler, kein stilles Anlegen.
        assert!(participants_from_event(&s, "gibt-es-nicht", &ev2).is_err());
    }

    #[test]
    fn self_is_recognised_by_flag_or_by_own_address() {
        let (_d, s) = tmp_store();
        let m = ready(&s, "Jour fixe", 100);
        let ev = event(
            "u1",
            1,
            vec![
                attendee(Some("ich@wolff.de"), Some("Patrick Wolff"), false),
                attendee(Some("anna@firma.de"), Some("Anna Berg"), false),
            ],
        );
        participants_from_event(&s, &m, &ev).unwrap();
        let none = s.participants_of(&m, &[]).unwrap();
        assert!(none.iter().all(|p| !p.is_self));
        let own = vec!["ICH@wolff.de".to_string()];
        let with = s.participants_of(&m, &own).unwrap();
        assert_eq!(
            with.iter()
                .filter(|p| p.is_self)
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Patrick Wolff"]
        );
        let list = s.list_people(None, &own).unwrap();
        assert!(list.iter().any(|p| p.name == "Patrick Wolff" && p.is_self));
    }

    #[test]
    fn named_speakers_become_participants_and_unnaming_removes_them() {
        let (_d, s) = tmp_store();
        let m = ready(&s, "Gespraech", 100);
        s.set_speaker_name(&m, 1, 1, Some("Anna Berg")).unwrap();
        s.set_speaker_name(&m, 1, 2, Some("Ben Alt")).unwrap();
        s.set_speaker_name(&m, 1, 3, None).unwrap();
        assert_eq!(sync_speaker_participants(&s, &m).unwrap(), 2);
        let list = s.participants_of(&m, &[]).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list
            .iter()
            .all(|p| p.role == "speaker" && p.source == "speaker"));
        // Der Sprecher verweist auf die Person.
        let linked: Vec<Option<String>> = s
            .speaker_rows(&m)
            .unwrap()
            .into_iter()
            .map(|r| r.human_id)
            .collect();
        assert_eq!(linked.iter().filter(|h| h.is_some()).count(), 2);
        assert!(linked[2].is_none());
        // Umbenennen: die alte Person faellt aus der Besprechung.
        s.set_speaker_name(&m, 1, 2, None).unwrap();
        assert_eq!(sync_speaker_participants(&s, &m).unwrap(), 1);
        assert_eq!(s.participants_of(&m, &[]).unwrap().len(), 1);
        assert_eq!(
            s.list_people(None, &[]).unwrap().len(),
            2,
            "Personen bleiben"
        );
    }

    #[test]
    fn a_calendar_entry_outranks_the_same_person_as_speaker() {
        let (_d, s) = tmp_store();
        let m = ready(&s, "Runde", 100);
        let ev = event(
            "u",
            1,
            vec![attendee(Some("anna@firma.de"), Some("Anna Berg"), true)],
        );
        participants_from_event(&s, &m, &ev).unwrap();
        s.set_speaker_name(&m, 1, 1, Some("Anna Berg")).unwrap();
        sync_speaker_participants(&s, &m).unwrap();
        let list = s.participants_of(&m, &[]).unwrap();
        assert_eq!(list.len(), 1, "eine Person, eine Zeile");
        assert_eq!(
            (list[0].role.as_str(), list[0].source.as_str()),
            ("organizer", "calendar")
        );
        // Der Name faellt weg: die Kalenderzeile bleibt.
        s.set_speaker_name(&m, 1, 1, None).unwrap();
        sync_speaker_participants(&s, &m).unwrap();
        assert_eq!(s.participants_of(&m, &[]).unwrap().len(), 1);
    }

    #[test]
    fn scope_finds_meetings_by_participant_name_and_by_person_id() {
        use crate::managers::meetings::search::index::{MeetingFilter, ScopeFilter};
        let (_d, s) = tmp_store();
        let m1 = ready(&s, "Eins", 100);
        let m2 = ready(&s, "Zwei", 200);
        let m3 = ready(&s, "Drei", 300);
        let anna = s
            .upsert_person(Some("anna@firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        let ben = s
            .upsert_person(None, Some("Ben Müller"), "speaker")
            .unwrap();
        s.set_participants(&m1, "calendar", &[(anna.clone(), "attendee")])
            .unwrap();
        s.set_participants(&m2, "speaker", &[(ben.clone(), "speaker")])
            .unwrap();
        s.set_participants(
            &m3,
            "calendar",
            &[(anna.clone(), "attendee"), (ben.clone(), "attendee")],
        )
        .unwrap();
        let ids = |f: ScopeFilter| s.resolve_scope(&f).unwrap();
        let person = |p: &str| ScopeFilter {
            person: Some(p.into()),
            ..Default::default()
        };
        // Name in beiden Schreibweisen, ohne Beachtung von Gross-/Kleinschreibung
        // und Diakritika, obwohl der Text der Besprechungen den Namen nie nennt.
        assert_eq!(ids(person("Berg, Anna")), vec![m3.clone(), m1.clone()]);
        assert_eq!(ids(person("anna berg")), vec![m3.clone(), m1.clone()]);
        assert_eq!(ids(person("MULLER")), vec![m3.clone(), m2.clone()]);
        assert_eq!(ids(person("anna@firma")), vec![m3.clone(), m1.clone()]);
        assert!(ids(person("niemand")).is_empty());
        // Genau diese Person, nicht ein Suchtext.
        let by_id = |id: &str| ScopeFilter {
            person_id: Some(id.into()),
            ..Default::default()
        };
        assert_eq!(ids(by_id(&anna)), vec![m3.clone(), m1.clone()]);
        assert_eq!(ids(by_id(&ben)), vec![m3.clone(), m2.clone()]);
        assert!(ids(by_id("unbekannt")).is_empty());
        // Mit den uebrigen Feldern zugleich (UND).
        assert_eq!(
            ids(ScopeFilter {
                person_id: Some(anna.clone()),
                from: Some(250),
                ..Default::default()
            }),
            vec![m3.clone()]
        );
        // Die Listensuche filtert ebenso.
        let page = s
            .search_meetings(
                "",
                &MeetingFilter {
                    person_id: Some(ben.clone()),
                    ..Default::default()
                },
                0,
                50,
            )
            .unwrap();
        let mut listed: Vec<&str> = page.items.iter().map(|i| i.meeting.id.as_str()).collect();
        listed.sort_unstable();
        let mut want = vec![m3.as_str(), m2.as_str()];
        want.sort_unstable();
        assert_eq!(listed, want);
        assert_eq!(page.total, 2);
        // Gelöschte Besprechungen verschwinden auch hier.
        s.soft_delete_meeting(&m3).unwrap();
        assert_eq!(ids(by_id(&anna)), vec![m1]);
    }

    #[test]
    fn people_can_be_renamed_and_their_address_changed() {
        let (_d, s) = tmp_store();
        let anna = s
            .upsert_person(Some("anna@firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        let ben = s
            .upsert_person(Some("ben@firma.de"), Some("Ben Alt"), "calendar")
            .unwrap();
        s.update_person(&anna, "Anna Berg-Meier", Some("Anna.BM@Neu.de"))
            .unwrap();
        let p = s.get_person(&anna, &[]).unwrap().unwrap();
        assert_eq!(p.name, "Anna Berg-Meier");
        assert_eq!(p.email.as_deref(), Some("anna.bm@neu.de"));
        assert_eq!(p.company.as_deref(), Some("neu.de"));
        // Der alte Name findet weiter dieselbe Person.
        assert_eq!(
            s.upsert_person(None, Some("Anna Berg"), "speaker").unwrap(),
            anna
        );
        // `None` laesst die Adresse, leer entfernt sie.
        s.update_person(&anna, "Anna Berg-Meier", None).unwrap();
        assert!(s.get_person(&anna, &[]).unwrap().unwrap().email.is_some());
        s.update_person(&anna, "Anna Berg-Meier", Some("  "))
            .unwrap();
        let p = s.get_person(&anna, &[]).unwrap().unwrap();
        assert_eq!((p.email, p.company), (None, None));
        // Fehler: fremde Adresse, kaputte Adresse, leerer Name, unbekannte Person.
        let err = |r: Result<()>| r.unwrap_err().to_string();
        assert_eq!(
            err(s.update_person(&anna, "Anna", Some("ben@firma.de"))),
            "person_email_taken"
        );
        assert_eq!(
            err(s.update_person(&anna, "Anna", Some("kaputt"))),
            "person_email_invalid"
        );
        assert_eq!(
            err(s.update_person(&ben, "  ", None)),
            "person_name_invalid"
        );
        assert_eq!(err(s.update_person("nix", "X", None)), "person_not_found");
    }

    #[test]
    fn the_people_list_is_sorted_by_meetings_and_searchable() {
        let (_d, s) = tmp_store();
        let m1 = ready(&s, "Eins", 100);
        let m2 = ready(&s, "Zwei", 200);
        let anna = s
            .upsert_person(Some("anna@firma.de"), Some("Anna Berg"), "calendar")
            .unwrap();
        let ben = s
            .upsert_person(Some("ben@kunde.de"), Some("Ben Müller"), "calendar")
            .unwrap();
        let _cara = s.upsert_person(None, Some("Cara Zed"), "manual").unwrap();
        s.set_participants(&m1, "calendar", &[(ben.clone(), "attendee")])
            .unwrap();
        s.set_participants(
            &m2,
            "calendar",
            &[(ben.clone(), "attendee"), (anna.clone(), "attendee")],
        )
        .unwrap();
        let all = s.list_people(None, &[]).unwrap();
        assert_eq!(
            all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Ben Müller", "Anna Berg", "Cara Zed"]
        );
        assert_eq!(all[0].meeting_count, 2);
        let names = |q: &str| -> Vec<String> {
            s.list_people(Some(q), &[])
                .unwrap()
                .into_iter()
                .map(|p| p.name)
                .collect()
        };
        assert_eq!(names("berg, anna"), vec!["Anna Berg"]);
        assert_eq!(names("MULLER"), vec!["Ben Müller"], "ohne Diakritika");
        assert_eq!(names("kunde.de"), vec!["Ben Müller"], "nach Adresse/Firma");
        assert!(names("niemand").is_empty());
    }
}
