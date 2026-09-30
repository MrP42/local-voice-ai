//! Audit-Log: jede Aktion eines Nicht-Nutzers ueber das Register (A1, AK11).
//!
//! - Geheimnisse gelangen nie hinein: Schluessel wie `password`/`token` werden
//!   geschwaerzt, Texte von Zugangsdaten in Adressen und Bearer-Tokens befreit
//!   (`redact_text`, `redact_value`). Adressen behalten im Audit nur den Host: der
//!   Pfad kann einen Zugangsschluessel tragen (Outlook-ICS), siehe `clip_url_paths`.
//! - Begrenzt: Zielangabe 300 Zeichen, Detail 4 KiB, Integration/Faehigkeit 64
//!   Zeichen, und hoechstens `MAX_ROWS` Zeilen -- beim Schreiben werden die
//!   aeltesten in derselben Transaktion entfernt, und zwar ZUERST Verweigerungen,
//!   dann erst ok/error/pending. Ein Agent, der in der Schleife ruft, kann die
//!   Platte also nicht fuellen und verdraengt keine echten Aktionen.
//! - Verweigerungen werden zusammengefasst: gleiche (Aufrufer, Integration,
//!   Faehigkeit, Grund) innerhalb von `DENY_WINDOW_MS` zaehlen eine Zeile hoch
//!   (`detail.count`, `detail.first_ts`), statt je Aufruf eine neue zu schreiben.
//! - Jede Schreibstelle ist EINE `IMMEDIATE`-Transaktion (Einfuegen/Zusammenfassen +
//!   Kuerzen): Abbruch und volle Platte lassen den alten Stand.

use std::sync::OnceLock;

use regex::Regex;
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde_json::{json, Value};

use super::model::{AuditEntry, AuditOutcome, IntegrationError, NewAudit};

/// Aufbewahrung: so viele Zeilen bleiben hoechstens stehen.
pub const MAX_ROWS: i64 = 20_000;
pub const MAX_TARGET_CHARS: usize = 300;
pub const MAX_DETAIL_BYTES: usize = 4096;
/// Laengste gespeicherte Integrations-Kennung bzw. Faehigkeit (Zeichen).
pub const MAX_ID_CHARS: usize = 64;
/// Zeitfenster, in dem gleiche Verweigerungen zu einer Zeile zusammengefasst werden.
pub const DENY_WINDOW_MS: i64 = 60_000;
const MAX_DEPTH: usize = 6;
const MAX_ARRAY: usize = 50;
/// Hoechstzahl Zeilen je Abfrage.
pub const MAX_LIST: u32 = 500;

/// Teile von Schluesselnamen, deren Wert nie im Audit oder in Vorschauen landet
/// (Teilstring des normalisierten Namens, also auch `smtp_password`, `sessionId`).
const SECRET_KEY_HINTS: &[&str] = &[
    "password",
    "passwort",
    "passwd",
    "pwd",
    "passphrase",
    "passcode",
    "secret",
    "token",
    "apikey",
    "accesskey",
    "authorization",
    "credential",
    "privatekey",
    "refresh",
    "cookie",
    "session",
    "signature",
];

/// Kurze Namen, die nur als GANZER normalisierter Name gelten (`key`, nicht
/// `monkey`/`keyboard`).
const SECRET_KEY_EXACT: &[&str] = &["key", "pass", "auth", "sig", "sas"];

/// Kurze Woerter, die als LETZTES Wortsegment genuegen (`x-auth`, `x_sig`), aber
/// nicht mitten im Namen (`auth_mode`, `author`). `pass` fehlt bewusst: `single_pass`
/// und `final_pass` sind Verarbeitungsstufen, keine Kennwoerter (`pass` als ganzer
/// Name und `passwd`/`passphrase` sind dennoch erfasst).
const SECRET_KEY_LAST_SEGMENT: &[&str] = &["auth", "sig", "sas", "pwd"];

fn normalized_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Zerlegt einen Schluesselnamen in Woerter: an allen Zeichen ausser Buchstaben
/// und Ziffern, an Klein-zu-Gross-Wechseln (`userPass`) und zwischen Buchstaben
/// und Ziffern. Alles in Kleinbuchstaben.
pub(super) fn key_segments(key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in key.chars() {
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur).to_ascii_lowercase());
            }
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let boundary = (p.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (p.is_ascii_alphabetic() != c.is_ascii_alphabetic());
            if boundary && !cur.is_empty() {
                out.push(std::mem::take(&mut cur).to_ascii_lowercase());
            }
        }
        cur.push(c);
        prev = Some(c);
    }
    if !cur.is_empty() {
        out.push(cur.to_ascii_lowercase());
    }
    out
}

/// Sieht der Schluessel nach einem Geheimnis aus?
pub fn is_secret_key(key: &str) -> bool {
    let k = normalized_key(key);
    if SECRET_KEY_HINTS.iter().any(|hint| k.contains(hint))
        || SECRET_KEY_EXACT.contains(&k.as_str())
    {
        return true;
    }
    key_segments(key)
        .iter()
        .rfind(|s| s.bytes().any(|b| b.is_ascii_alphabetic()))
        .is_some_and(|last| SECRET_KEY_LAST_SEGMENT.contains(&last.as_str()))
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
            r#"(?i)\b(password|passwort|passwd|pwd|secret|token|api[_-]?key|authorization|access_token|refresh_token|sig|signature|code|key)(["']?\s*[=:]\s*)("[^"]*"|'[^']*'|[^\s,;&"']+)"#,
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

/// Adresse mit Host: Schema, Host (mit Port), danach Pfad/Abfrage/Fragment.
fn url_with_path_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\b([a-z][a-z0-9+.\-]*)(://[^/\s?#"'<>]+)[/?#][^\s"'<>]*"#).expect("url")
    })
}

/// Entfernt aus einem Text Zugangsdaten in Adressen (`https://user:pw@host`),
/// Bearer-/Basic-Tokens, JWTs und Zuweisungen wie `password=...` oder `sig=...`.
pub fn redact_text(s: &str) -> String {
    let s = userinfo_pattern().replace_all(s, "${1}***@");
    let s = bearer_pattern().replace_all(&s, "${1} ***");
    let s = jwt_pattern().replace_all(&s, "***");
    let s = assignment_pattern().replace_all(&s, "${1}${2}***");
    s.into_owned()
}

/// Kuerzt jede Adresse auf Schema und Host (`https://host/…`): ein Zugangsschluessel
/// steckt oft im Pfad (Outlook-ICS-Adresse), und der Pfad ist nicht zu erkennen.
/// `file://` bleibt stehen -- lokale Pfade sind keine Zugangsschluessel.
pub fn clip_url_paths(s: &str) -> String {
    url_with_path_pattern()
        .replace_all(s, |caps: &regex::Captures<'_>| {
            if caps[1].eq_ignore_ascii_case("file") {
                caps[0].to_string()
            } else {
                format!("{}{}/…", &caps[1], &caps[2])
            }
        })
        .into_owned()
}

fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Text fuer Anzeige und Vorschau: geschwaerzt und gekuerzt.
pub fn sanitize_text(s: &str, max: usize) -> String {
    clip_chars(&redact_text(s), max)
}

/// Text fuer Audit, Fehlermeldungen und Status: wie `sanitize_text`, und Adressen
/// behalten nur den Host (`clip_url_paths`).
pub fn sanitize_audit_text(s: &str, max: usize) -> String {
    clip_chars(&clip_url_paths(&redact_text(s)), max)
}

/// Grenzen und Art der Schwaerzung eines JSON-Werts.
#[derive(Clone, Copy)]
struct Shape {
    max_depth: usize,
    max_array: usize,
    max_string: usize,
    clip_urls: bool,
}

/// Anzeige und Dump: begrenzt, Adressen unveraendert.
const SHAPE_DISPLAY: Shape = Shape {
    max_depth: MAX_DEPTH,
    max_array: MAX_ARRAY,
    max_string: 1000,
    clip_urls: false,
};

/// Audit-Detail: begrenzt, Adressen auf den Host gekuerzt.
const SHAPE_AUDIT: Shape = Shape {
    max_depth: MAX_DEPTH,
    max_array: MAX_ARRAY,
    max_string: 1000,
    clip_urls: true,
};

/// Angaben, deren Form erhalten bleiben muss (Provenienz-`params`: Listen von
/// Ereignisnummern, Segmente): nur Geheimnisse fallen weg, nichts wird gekuerzt.
const SHAPE_KEEP: Shape = Shape {
    max_depth: 16,
    max_array: usize::MAX,
    max_string: usize::MAX,
    clip_urls: false,
};

/// Schwaerzt ein JSON-Wert rekursiv: Werte unter Geheimnis-Schluesseln werden
/// `"***"`, Strings durchlaufen `redact_text`, Tiefe und Listenlaenge sind
/// begrenzt.
pub fn redact_value(v: &Value) -> Value {
    redact_depth(v, 0, SHAPE_DISPLAY)
}

/// Wie `redact_value`, aber ohne Kuerzen von Listen, Texten und Tiefe (bis 16
/// Ebenen): fuer Angaben, die als Ganzes erhalten bleiben sollen (Provenienz).
pub fn redact_params(v: &Value) -> Value {
    redact_depth(v, 0, SHAPE_KEEP)
}

fn redact_depth(v: &Value, depth: usize, shape: Shape) -> Value {
    if depth >= shape.max_depth {
        return Value::String("…".to_string());
    }
    match v {
        Value::String(s) => {
            let text = redact_text(s);
            let text = if shape.clip_urls {
                clip_url_paths(&text)
            } else {
                text
            };
            Value::String(clip_chars(&text, shape.max_string))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(shape.max_array)
                .map(|x| redact_depth(x, depth + 1, shape))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, val)| {
                    let shown = if is_secret_key(k) {
                        Value::String("***".to_string())
                    } else {
                        redact_depth(val, depth + 1, shape)
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
    let text = redact_depth(value, 0, SHAPE_AUDIT).to_string();
    if text.len() > MAX_DETAIL_BYTES {
        return Some(r#"{"truncated":true}"#.to_string());
    }
    Some(text)
}

/// Schreibt einen Eintrag und kuerzt ueber `MAX_ROWS`. Gibt die ID zurueck.
pub fn record(conn: &Connection, e: &NewAudit) -> Result<i64, IntegrationError> {
    record_at(conn, e, chrono::Utc::now().timestamp_millis())
}

/// Der Grund einer Verweigerung (`detail.reason`), Schluessel der Zusammenfassung.
fn denial_reason(detail: &Option<Value>) -> Option<String> {
    detail.as_ref()?.get("reason")?.as_str().map(str::to_string)
}

/// Faellt die Verweigerung unter eine gleiche, noch junge Zeile, zaehlt diese hoch
/// und liefert ihre ID; sonst `None` (es wird eine neue Zeile geschrieben).
fn merge_denial(
    tx: &Transaction<'_>,
    e: &NewAudit,
    integration_id: Option<&str>,
    capability: Option<&str>,
    now_ms: i64,
) -> Result<Option<i64>, IntegrationError> {
    let reason = denial_reason(&e.detail);
    let candidates: Vec<(i64, i64, Option<String>)> = {
        let mut stmt = tx.prepare_cached(
            "SELECT id, ts, detail_json FROM audit_log
             WHERE outcome = 'denied' AND caller = ?1 AND integration_id IS ?2
               AND capability IS ?3 AND ts >= ?4
             ORDER BY id DESC LIMIT 20",
        )?;
        let rows = stmt
            .query_map(
                params![
                    e.caller,
                    integration_id,
                    capability,
                    now_ms - DENY_WINDOW_MS
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, ts, detail) in candidates {
        let mut stored: Value = detail
            .as_deref()
            .and_then(|d| serde_json::from_str(d).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let same_reason = stored.get("reason").and_then(Value::as_str) == reason.as_deref();
        if !same_reason {
            continue;
        }
        let count = stored
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .saturating_add(1);
        stored["count"] = json!(count);
        if stored.get("first_ts").is_none() {
            stored["first_ts"] = json!(ts);
        }
        tx.execute(
            "UPDATE audit_log SET ts = MAX(ts, ?2), detail_json = ?3 WHERE id = ?1",
            params![id, now_ms, stored.to_string()],
        )?;
        return Ok(Some(id));
    }
    Ok(None)
}

/// Haelt die Tabelle bei hoechstens `MAX_ROWS`: zuerst fallen die aeltesten
/// Verweigerungen weg, erst dann die aeltesten uebrigen Zeilen. Die eben
/// geschriebene Zeile `keep_id` bleibt immer stehen.
fn rotate(tx: &Transaction<'_>, keep_id: i64) -> Result<(), IntegrationError> {
    // Die Nummern sind fortlaufend und werden nie wiederverwendet: bis MAX_ROWS
    // kann die Tabelle nicht ueber der Grenze liegen (billige Vorpruefung).
    if keep_id <= MAX_ROWS {
        return Ok(());
    }
    let total: i64 = tx.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))?;
    let excess = total - MAX_ROWS;
    if excess <= 0 {
        return Ok(());
    }
    let dropped = tx.execute(
        "DELETE FROM audit_log WHERE id IN (
           SELECT id FROM audit_log WHERE outcome = 'denied' AND id <> ?1
           ORDER BY id LIMIT ?2)",
        params![keep_id, excess],
    )? as i64;
    let rest = excess - dropped;
    if rest > 0 {
        tx.execute(
            "DELETE FROM audit_log WHERE id IN (
               SELECT id FROM audit_log WHERE id <> ?1 ORDER BY id LIMIT ?2)",
            params![keep_id, rest],
        )?;
    }
    Ok(())
}

pub fn record_at(conn: &Connection, e: &NewAudit, now_ms: i64) -> Result<i64, IntegrationError> {
    if e.caller.trim().is_empty() {
        return Err(IntegrationError::Invalid(
            "Audit: Aufrufer fehlt".to_string(),
        ));
    }
    let integration_id = e
        .integration_id
        .as_deref()
        .map(|s| clip_chars(s, MAX_ID_CHARS));
    let capability = e.capability.as_deref().map(|s| clip_chars(s, MAX_ID_CHARS));
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if e.outcome == AuditOutcome::Denied {
        if let Some(id) = merge_denial(
            &tx,
            e,
            integration_id.as_deref(),
            capability.as_deref(),
            now_ms,
        )? {
            tx.commit()?;
            return Ok(id);
        }
    }
    tx.execute(
        "INSERT INTO audit_log (ts, caller, integration_id, capability, target, outcome, detail_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            now_ms,
            e.caller,
            integration_id,
            capability,
            e.target
                .as_deref()
                .map(|t| sanitize_audit_text(t, MAX_TARGET_CHARS)),
            e.outcome.as_str(),
            detail_text(&e.detail),
        ],
    )?;
    let id = tx.last_insert_rowid();
    rotate(&tx, id)?;
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
