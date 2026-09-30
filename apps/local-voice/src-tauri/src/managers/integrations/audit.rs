//! Audit-Log: jede Aktion eines Nicht-Nutzers ueber das Register (A1, AK11).
//!
//! - Geheimnisse gelangen nie hinein: Schluessel wie `password`/`token` werden
//!   geschwaerzt, Texte von Zugangsdaten in Adressen und Bearer-Tokens befreit
//!   (`redact_text`, `redact_value`).
//! - Begrenzt: Zielangabe 300 Zeichen, Detail 4 KiB, und hoechstens `MAX_ROWS`
//!   Zeilen -- beim Schreiben werden die aeltesten in derselben Transaktion
//!   entfernt. Ein Agent, der in der Schleife ruft, kann die Platte also nicht
//!   fuellen.
//! - Jede Schreibstelle ist EINE `IMMEDIATE`-Transaktion (Einfuegen + Kuerzen):
//!   Abbruch und volle Platte lassen den alten Stand.

use std::sync::OnceLock;

use regex::Regex;
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde_json::Value;

use super::model::{AuditEntry, AuditOutcome, IntegrationError, NewAudit};

/// Aufbewahrung: so viele Zeilen bleiben hoechstens stehen.
pub const MAX_ROWS: i64 = 20_000;
pub const MAX_TARGET_CHARS: usize = 300;
pub const MAX_DETAIL_BYTES: usize = 4096;
const MAX_DEPTH: usize = 6;
const MAX_ARRAY: usize = 50;
/// Hoechstzahl Zeilen je Abfrage.
pub const MAX_LIST: u32 = 500;

/// Schluesselnamen, deren Wert nie im Audit oder in Vorschauen landet.
const SECRET_KEY_HINTS: [&str; 10] = [
    "password",
    "passwort",
    "passwd",
    "secret",
    "token",
    "apikey",
    "authorization",
    "credential",
    "privatekey",
    "refresh",
];

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Sieht der Schluessel nach einem Geheimnis aus?
pub fn is_secret_key(key: &str) -> bool {
    let k = normalized_key(key);
    SECRET_KEY_HINTS.iter().any(|hint| k.contains(hint))
}

fn userinfo_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)([a-z][a-z0-9+.\-]*://)[^/\s@]+@").expect("userinfo"))
}

fn bearer_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=\-]{8,}").expect("bearer")
    })
}

fn assignment_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(password|passwort|passwd|secret|token|api[_-]?key|authorization|access_token|refresh_token)(["']?\s*[=:]\s*)("[^"]*"|'[^']*'|[^\s,;&"']+)"#,
        )
        .expect("assignment")
    })
}

fn jwt_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]*").expect("jwt")
    })
}

/// Entfernt aus einem Text Zugangsdaten in Adressen (`https://user:pw@host`),
/// Bearer-/Basic-Tokens, JWTs und Zuweisungen wie `password=...`.
pub fn redact_text(s: &str) -> String {
    let s = userinfo_pattern().replace_all(s, "${1}***@");
    let s = bearer_pattern().replace_all(&s, "${1} ***");
    let s = jwt_pattern().replace_all(&s, "***");
    let s = assignment_pattern().replace_all(&s, "${1}${2}***");
    s.into_owned()
}

fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Text fuer Audit/Vorschau: geschwaerzt und gekuerzt.
pub fn sanitize_text(s: &str, max: usize) -> String {
    clip_chars(&redact_text(s), max)
}

/// Schwaerzt ein JSON-Wert rekursiv: Werte unter Geheimnis-Schluesseln werden
/// `"***"`, Strings durchlaufen `redact_text`, Tiefe und Listenlaenge sind
/// begrenzt.
pub fn redact_value(v: &Value) -> Value {
    redact_depth(v, 0)
}

fn redact_depth(v: &Value, depth: usize) -> Value {
    if depth >= MAX_DEPTH {
        return Value::String("…".to_string());
    }
    match v {
        Value::String(s) => Value::String(sanitize_text(s, 1000)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(MAX_ARRAY)
                .map(|x| redact_depth(x, depth + 1))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, val)| {
                    let shown = if is_secret_key(k) {
                        Value::String("***".to_string())
                    } else {
                        redact_depth(val, depth + 1)
                    };
                    (k.clone(), shown)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn detail_text(detail: &Option<Value>) -> Option<String> {
    let value = detail.as_ref()?;
    let text = redact_value(value).to_string();
    if text.len() > MAX_DETAIL_BYTES {
        return Some(r#"{"truncated":true}"#.to_string());
    }
    Some(text)
}

/// Schreibt einen Eintrag und kuerzt ueber `MAX_ROWS`. Gibt die ID zurueck.
pub fn record(conn: &Connection, e: &NewAudit) -> Result<i64, IntegrationError> {
    record_at(conn, e, chrono::Utc::now().timestamp_millis())
}

pub fn record_at(conn: &Connection, e: &NewAudit, now_ms: i64) -> Result<i64, IntegrationError> {
    if e.caller.trim().is_empty() {
        return Err(IntegrationError::Invalid(
            "Audit: Aufrufer fehlt".to_string(),
        ));
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO audit_log (ts, caller, integration_id, capability, target, outcome, detail_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            now_ms,
            e.caller,
            e.integration_id,
            e.capability,
            e.target.as_deref().map(|t| sanitize_text(t, MAX_TARGET_CHARS)),
            e.outcome.as_str(),
            detail_text(&e.detail),
        ],
    )?;
    let id = tx.last_insert_rowid();
    if id > MAX_ROWS {
        tx.execute(
            "DELETE FROM audit_log WHERE id <= ?1",
            params![id - MAX_ROWS],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

/// Setzt das Ergebnis eines zuvor als `pending` geschriebenen Eintrags (Aktion
/// lief: `ok` oder `error`) und ergaenzt das Detail.
pub fn set_outcome(
    conn: &Connection,
    id: i64,
    outcome: AuditOutcome,
    detail: Option<Value>,
) -> Result<(), IntegrationError> {
    conn.execute(
        "UPDATE audit_log SET outcome = ?2, detail_json = COALESCE(?3, detail_json) WHERE id = ?1",
        params![id, outcome.as_str(), detail_text(&detail)],
    )?;
    Ok(())
}

/// Filter fuer `list`.
#[derive(Clone, Debug, Default)]
pub struct AuditFilter {
    pub integration_id: Option<String>,
    pub caller: Option<String>,
    pub outcome: Option<String>,
    pub since_ms: Option<i64>,
}

/// Die neuesten Eintraege zuerst; `limit` liegt zwischen 1 und `MAX_LIST`.
pub fn list(
    conn: &Connection,
    filter: &AuditFilter,
    limit: u32,
) -> Result<Vec<AuditEntry>, IntegrationError> {
    let limit = i64::from(limit.clamp(1, MAX_LIST));
    let mut stmt = conn.prepare(
        "SELECT id, ts, caller, integration_id, capability, target, outcome, detail_json
         FROM audit_log
         WHERE (?1 IS NULL OR integration_id = ?1)
           AND (?2 IS NULL OR caller = ?2)
           AND (?3 IS NULL OR outcome = ?3)
           AND (?4 IS NULL OR ts >= ?4)
         ORDER BY id DESC LIMIT ?5",
    )?;
    let rows = stmt
        .query_map(
            params![
                filter.integration_id,
                filter.caller,
                filter.outcome,
                filter.since_ms,
                limit
            ],
            |r| {
                Ok(AuditEntry {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    caller: r.get(2)?,
                    integration_id: r.get(3)?,
                    capability: r.get(4)?,
                    target: r.get(5)?,
                    outcome: r.get(6)?,
                    detail_json: r.get(7)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn count(conn: &Connection) -> Result<i64, IntegrationError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))?)
}

#[cfg(test)]
mod tests;
