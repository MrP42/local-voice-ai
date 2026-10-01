//! Zugaenge der Agentenbruecke: Token anlegen, pruefen, zurueckziehen, und das Recht je
//! Werkzeug (A7).
//!
//! - **Token**: `lvat_` + 43 Zeichen Base64-URL aus 32 Zufallsbytes des Betriebssystems
//!   (256 Bit). Gespeichert wird NUR der SHA-256 (hex); das Token selbst sieht der Nutzer genau
//!   einmal beim Anlegen. Bei so viel Zufall braucht es kein langsames Passwort-Hashing; die
//!   Suche laeuft ueber den Hash (eindeutiger Index), danach wird der Hash zeitkonstant
//!   verglichen.
//! - **Zurueckziehen** setzt `revoked_at`; die Zeile bleibt, damit ein altes Token als
//!   „zurueckgezogen“ erkannt und im Audit so benannt wird. Es gilt sofort (jede Anfrage prueft
//!   den Zugang neu, siehe `bridge`).
//! - **Werkzeugrecht**: Zeile in `agent_tool_grants`; fehlt sie, gilt „aus“ (ein neuer Zugang hat
//!   keine Werkzeuge). Nur Werkzeuge des Katalogs; „Aufnahme starten“ nie `allow`.
//! - **Begrenzt**: hoechstens `MAX_ACTIVE_CLIENTS` aktive Zugaenge, Name hoechstens
//!   `MAX_LABEL_CHARS` Zeichen ohne Steuerzeichen.

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use ulid::Ulid;

use super::catalog;
use crate::managers::integrations::model::{
    Direction, GrantMode, Integration, IntegrationError, Kind, NewIntegration,
};
use crate::managers::integrations::store;

/// Praefix jedes Tokens (erkennbar in Logs und Geheimnis-Scannern).
pub const TOKEN_PREFIX: &str = "lvat_";
/// Gesamtlaenge eines Tokens: Praefix + 43 Zeichen (32 Bytes Base64-URL ohne Auffuellung).
pub const TOKEN_LEN: usize = 5 + 43;
pub const MAX_ACTIVE_CLIENTS: i64 = 20;
pub const MAX_LABEL_CHARS: usize = 60;
/// Kennung der Agent-Integration, die beim ersten Zugang angelegt wird.
pub const DEFAULT_INTEGRATION_ID: &str = "agents";
pub const DEFAULT_INTEGRATION_LABEL: &str = "Externe Agenten";
/// `last_used_at` wird hoechstens so oft geschrieben (eine Zeile je Minute, nicht je Aufruf).
pub const TOUCH_INTERVAL_MS: i64 = 60_000;
/// Zuordnungen von Freigaben werden nach so langer Zeit entfernt (doppelte Lebensdauer einer Freigabe).
pub const APPROVAL_LINK_TTL_MS: i64 = 2 * 60 * 60 * 1000;

/// Ein Zugang (ohne Token).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AgentClient {
    pub id: String,
    pub label: String,
    /// Agent-Integration, deren Rechte als Obergrenze gelten.
    pub integration_id: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

impl AgentClient {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// Ergebnis der Anmeldung mit einem Token.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    /// Kein Zugang gehoert zu diesem Token (oder es ist keine gueltige Form).
    Invalid,
    /// Der Zugang wurde zurueckgezogen.
    Revoked(AgentClient),
    Store(String),
}

impl From<rusqlite::Error> for AuthError {
    fn from(e: rusqlite::Error) -> Self {
        AuthError::Store(e.to_string())
    }
}

fn invalid<T>(msg: &str) -> Result<T, IntegrationError> {
    Err(IntegrationError::Invalid(msg.to_string()))
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/// Erzeugt ein neues Token (256 Bit Zufall des Betriebssystems).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Hat der Text die Form eines Tokens? Schuetzt die Datenbank vor beliebigem Muell.
pub fn token_looks_valid(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token.starts_with(TOKEN_PREFIX)
        && token[TOKEN_PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// SHA-256 des Tokens als Kleinbuchstaben-Hex.
pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Zeitkonstanter Vergleich zweier Texte gleicher Laenge (sonst `false`).
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------
// Zugaenge
// ---------------------------------------------------------------------------

fn map_client(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentClient> {
    Ok(AgentClient {
        id: r.get("id")?,
        label: r.get("label")?,
        integration_id: r.get("integration_id")?,
        created_at: r.get("created_at")?,
        last_used_at: r.get("last_used_at")?,
        revoked_at: r.get("revoked_at")?,
    })
}

const SELECT: &str =
    "SELECT id, label, integration_id, created_at, last_used_at, revoked_at FROM agent_clients";

fn validate_label(label: &str) -> Result<String, IntegrationError> {
    let t = label.trim();
    if t.is_empty() {
        return invalid("Der Name des Zugangs fehlt.");
    }
    if t.chars().count() > MAX_LABEL_CHARS {
        return invalid("Der Name des Zugangs ist zu lang (höchstens 60 Zeichen).");
    }
    if t.chars().any(char::is_control) {
        return invalid("Der Name des Zugangs enthält Steuerzeichen.");
    }
    Ok(t.to_string())
}

/// Die Agent-Integration fuer neue Zugaenge: die erste vorhandene der Art `agent`, sonst wird
/// „Externe Agenten“ (Kennung `agents`, lesend und schreibend) angelegt.
pub fn ensure_agent_integration(
    conn: &Connection,
    now_ms: i64,
) -> Result<Integration, IntegrationError> {
    let find = |conn: &Connection| -> Result<Option<Integration>, IntegrationError> {
        Ok(store::list(conn)?.into_iter().find(|i| i.kind == Kind::Agent))
    };
    if let Some(i) = find(conn)? {
        return Ok(i);
    }
    let mut n = NewIntegration::new(Kind::Agent, DEFAULT_INTEGRATION_LABEL);
    n.id = Some(DEFAULT_INTEGRATION_ID.to_string());
    n.direction = Some(Direction::Both);
    match store::create(conn, &n, now_ms) {
        Ok(i) => Ok(i),
        // Ein anderer Schreiber war schneller: dessen Eintrag gilt.
        Err(e) => find(conn)?.ok_or(e),
    }
}

/// Legt einen Zugang an und liefert das Token (nur jetzt, nie wieder lesbar). `integration_id`
/// `None`: die Standard-Agent-Integration (wird bei Bedarf angelegt).
pub fn create(
    conn: &Connection,
    label: &str,
    integration_id: Option<&str>,
    now_ms: i64,
) -> Result<(AgentClient, String), IntegrationError> {
    let label = validate_label(label)?;
    let integration = match integration_id {
        Some(id) => {
            let Some(i) = store::get(conn, id)? else {
                return Err(IntegrationError::NotFound(id.to_string()));
            };
            if i.kind != Kind::Agent {
                return invalid("Ein Zugang gehört zu einer Integration der Art „Agent“.");
            }
            i
        }
        None => ensure_agent_integration(conn, now_ms)?,
    };
    let token = generate_token();
    let id = Ulid::new().to_string();
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let active: i64 = tx.query_row(
        "SELECT COUNT(*) FROM agent_clients WHERE revoked_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    if active >= MAX_ACTIVE_CLIENTS {
        return invalid("Es gibt schon 20 aktive Zugänge. Bitte zuerst einen zurückziehen.");
    }
    tx.execute(
        "INSERT INTO agent_clients (id, label, integration_id, token_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, label, integration.id, hash_token(&token), now_ms],
    )?;
    tx.commit()?;
    let client = get(conn, &id)?.ok_or_else(|| IntegrationError::NotFound(id.clone()))?;
    Ok((client, token))
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<AgentClient>, IntegrationError> {
    Ok(conn
        .query_row(&format!("{SELECT} WHERE id = ?1"), params![id], map_client)
        .optional()?)
}

/// Alle Zugaenge, aelteste zuerst (auch zurueckgezogene).
pub fn list(conn: &Connection) -> Result<Vec<AgentClient>, IntegrationError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY created_at, id"))?;
    let rows = stmt
        .query_map([], map_client)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Meldet einen Zugang mit seinem Token an.
pub fn authenticate(conn: &Connection, token: &str) -> Result<AgentClient, AuthError> {
    if !token_looks_valid(token) {
        return Err(AuthError::Invalid);
    }
    let hash = hash_token(token);
    let row: Option<(AgentClient, String)> = conn
        .query_row(
            "SELECT id, label, integration_id, created_at, last_used_at, revoked_at, token_hash
             FROM agent_clients WHERE token_hash = ?1",
            params![hash],
            |r| Ok((map_client(r)?, r.get::<_, String>("token_hash")?)),
        )
        .optional()?;
    match row {
        Some((client, stored)) if constant_time_eq(&stored, &hash) => {
            if client.is_active() {
                Ok(client)
            } else {
                Err(AuthError::Revoked(client))
            }
        }
        _ => Err(AuthError::Invalid),
    }
}

/// Zieht einen Zugang zurueck. `true`, wenn er jetzt zurueckgezogen wurde; ein schon
/// zurueckgezogener bleibt wie er ist (`false`), ein unbekannter ist ein Fehler.
pub fn revoke(conn: &Connection, id: &str, now_ms: i64) -> Result<bool, IntegrationError> {
    let n = conn.execute(
        "UPDATE agent_clients SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
        params![id, now_ms],
    )?;
    if n == 0 && get(conn, id)?.is_none() {
        return Err(IntegrationError::NotFound(id.to_string()));
    }
    Ok(n > 0)
}

/// Entfernt einen Zugang samt Werkzeugrechten und Freigabe-Zuordnungen (eine Transaktion).
/// Sein Token ist danach „ungueltig“ (nicht mehr „zurueckgezogen“). Das Audit behaelt seine Zeilen.
pub fn delete(conn: &Connection, id: &str) -> Result<(), IntegrationError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if get(&tx, id)?.is_none() {
        return Err(IntegrationError::NotFound(id.to_string()));
    }
    tx.execute("DELETE FROM agent_tool_grants WHERE client_id = ?1", params![id])?;
    tx.execute("DELETE FROM agent_approvals WHERE client_id = ?1", params![id])?;
    tx.execute("DELETE FROM agent_clients WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(())
}

/// Vermerkt die Nutzung (`last_used_at`), hoechstens einmal je `TOUCH_INTERVAL_MS`.
pub fn touch(conn: &Connection, id: &str, now_ms: i64) -> Result<(), IntegrationError> {
    conn.execute(
        "UPDATE agent_clients SET last_used_at = ?2
         WHERE id = ?1 AND (last_used_at IS NULL OR last_used_at <= ?3)",
        params![id, now_ms, now_ms - TOUCH_INTERVAL_MS],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Werkzeugrechte
// ---------------------------------------------------------------------------

/// Gespeichertes Recht eines Zugangs fuer ein Werkzeug (`None`: keine Zeile, es gilt „aus“).
pub fn tool_mode(
    conn: &Connection,
    client_id: &str,
    tool: &str,
) -> Result<Option<GrantMode>, IntegrationError> {
    let mode: Option<String> = conn
        .query_row(
            "SELECT mode FROM agent_tool_grants WHERE client_id = ?1 AND tool = ?2",
            params![client_id, tool],
            |r| r.get(0),
        )
        .optional()?;
    // Ein unbekannter Wert (kaputte Zeile, neuere Version) gilt als „aus“, nie als mehr.
    Ok(mode.and_then(|m| GrantMode::parse(&m)))
}

/// Alle gespeicherten Rechte eines Zugangs.
pub fn tool_modes(
    conn: &Connection,
    client_id: &str,
) -> Result<HashMap<String, GrantMode>, IntegrationError> {
    let mut stmt =
        conn.prepare("SELECT tool, mode FROM agent_tool_grants WHERE client_id = ?1")?;
    let rows = stmt
        .query_map(params![client_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(t, m)| Some((t, GrantMode::parse(&m)?)))
        .collect())
}

/// Setzt das Recht eines Zugangs fuer ein Werkzeug des Katalogs. „Aufnahme starten“ nie `allow`.
pub fn set_tool_mode(
    conn: &Connection,
    client_id: &str,
    tool: &str,
    mode: GrantMode,
) -> Result<(), IntegrationError> {
    let Some(entry) = catalog::find(tool) else {
        return invalid("Dieses Werkzeug gibt es nicht.");
    };
    if entry.capability.never_allow() && mode == GrantMode::Allow {
        return invalid(
            "Aufnahme starten kann nie dauerhaft erlaubt werden: die App zeigt vor jeder Aufnahme den Einwilligungsdialog.",
        );
    }
    if get(conn, client_id)?.is_none() {
        return Err(IntegrationError::NotFound(client_id.to_string()));
    }
    conn.execute(
        "INSERT INTO agent_tool_grants (client_id, tool, mode) VALUES (?1, ?2, ?3)
         ON CONFLICT(client_id, tool) DO UPDATE SET mode = excluded.mode",
        params![client_id, tool, mode.as_str()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Freigabe-Zuordnung
// ---------------------------------------------------------------------------

/// Ordnet eine Freigabe einem Zugang und Werkzeug zu und raeumt alte Zuordnungen weg.
pub fn link_approval(
    conn: &Connection,
    approval_id: &str,
    client_id: &str,
    tool: &str,
    now_ms: i64,
) -> Result<(), IntegrationError> {
    conn.execute(
        "DELETE FROM agent_approvals WHERE created_at <= ?1",
        params![now_ms - APPROVAL_LINK_TTL_MS],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO agent_approvals (approval_id, client_id, tool, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![approval_id, client_id, tool, now_ms],
    )?;
    Ok(())
}

/// Zu welchem Zugang und Werkzeug gehoert die Freigabe? `(client_id, tool)`.
pub fn approval_link(
    conn: &Connection,
    approval_id: &str,
) -> Result<Option<(String, String)>, IntegrationError> {
    Ok(conn
        .query_row(
            "SELECT client_id, tool FROM agent_approvals WHERE approval_id = ?1",
            params![approval_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

#[cfg(test)]
mod tests;
