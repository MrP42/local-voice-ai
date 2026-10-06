//! `--integrations-dump` (A1): der Stand des Registers als JSON, fuer Abnahme
//! und Fehlersuche ohne sqlite3.
//!
//! Enthaelt nie ein Geheimnis: je Fach nur `present`/`missing`/`broken`, die
//! Konfiguration laeuft zur Sicherheit durch die Schwaerzung, die Rechte stehen
//! als gespeicherte Zeilen UND als wirksamer Modus (Vorgaben eingerechnet).
//!
//! Der Aufruf in `lib.rs` verlangt `LVA_MEETINGS_DIR` (Sandbox): der Dump
//! oeffnet den Store und migriert ihn, er liest nie die produktive Datenbank.

use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use super::audit::{self, redact_value};
use super::grants::effective_mode;
use super::model::{Caller, Integration, IntegrationError};
use super::{secrets, store};

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

fn integration_json(
    conn: &Connection,
    i: &Integration,
    secrets_dir: Option<&Path>,
) -> Result<Value, IntegrationError> {
    let grants = store::grants_for(conn, &i.id)?;
    let stored: Vec<Value> = store::list_grants(conn, &i.id)?
        .into_iter()
        .map(|g| {
            json!({
                "capability": g.capability.as_str(),
                "caller": g.caller.as_str(),
                "mode": g.mode.as_str(),
            })
        })
        .collect();
    let mut effective = Map::new();
    for cap in i.capabilities() {
        let mut by_caller = Map::new();
        for caller in Caller::GRANTABLE {
            by_caller.insert(
                caller.as_str().to_string(),
                json!(effective_mode(i, *cap, caller, &grants, None).as_str()),
            );
        }
        by_caller.insert(
            Caller::User.as_str().to_string(),
            json!(effective_mode(i, *cap, Caller::User, &grants, None).as_str()),
        );
        effective.insert(cap.as_str().to_string(), Value::Object(by_caller));
    }
    let secrets_json: Vec<Value> = match secrets_dir {
        Some(dir) => secrets::statuses_in(dir, i)
            .into_iter()
            .map(|(slot, status)| {
                json!({
                    "slot": slot,
                    "status": secrets::status_label(&status),
                })
            })
            .collect(),
        None => Vec::new(),
    };
    let config: Value = serde_json::from_str(&i.config_json).unwrap_or(Value::Null);
    Ok(json!({
        "id": i.id,
        "kind": i.kind.as_str(),
        "label": i.label,
        "enabled": i.enabled,
        "direction": i.direction.as_str(),
        "account_hint": i.account_hint,
        "data_class": i.data_class,
        "created_at": i.created_at,
        "updated_at": i.updated_at,
        "last_ok_at": i.last_ok_at,
        "last_error": i.last_error,
        "config": redact_value(&config),
        "secrets": secrets_json,
        "grants": stored,
        "effective": Value::Object(effective),
    }))
}

/// Der Dump. `secrets_dir`: Ordner der Geheimnisse (nur ihr Zustand wird gemeldet).
pub fn build(
    conn: &Connection,
    secrets_dir: Option<&Path>,
    db_path: Option<&Path>,
) -> Result<Value, IntegrationError> {
    let mut integrations = Vec::new();
    for i in store::list(conn)? {
        integrations.push(integration_json(conn, &i, secrets_dir)?);
    }
    let calendar_sources: Vec<Value> = {
        let mut stmt = conn.prepare(
            "SELECT id, kind, deleted_at IS NOT NULL FROM calendar_sources ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "deleted": r.get::<_, bool>(2)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let recent_audit: Vec<Value> = audit::list(conn, &Default::default(), 20)?
        .into_iter()
        .map(|a| {
            json!({
                "id": a.id, "ts": a.ts, "caller": a.caller,
                "integration_id": a.integration_id, "capability": a.capability,
                "target": a.target, "outcome": a.outcome,
            })
        })
        .collect();
    Ok(json!({
        "schema_version": count(conn, "PRAGMA user_version"),
        "db": db_path.map(|p| p.display().to_string()),
        "integrations": integrations,
        "calendar_sources": calendar_sources,
        "counts": {
            "integrations": count(conn, "SELECT COUNT(*) FROM integrations"),
            "grants": count(conn, "SELECT COUNT(*) FROM integration_grants"),
            "audit": count(conn, "SELECT COUNT(*) FROM audit_log"),
            "approvals_pending": count(conn, "SELECT COUNT(*) FROM approvals WHERE state = 'pending'"),
            "provenance": count(conn, "SELECT COUNT(*) FROM provenance"),
        },
        "recent_audit": recent_audit,
    }))
}

/// Standardzahl der Eintraege im Audit-Dump (die neuesten).
pub const AUDIT_DUMP_DEFAULT_LIMIT: u32 = 1000;

/// `--audit-dump` (A8, AK11): das Protokoll aller Aktionen als JSON, aelteste zuerst.
///
/// - **Alle Aktionen**: jeder Eintrag mit Zeit, Aufrufer, Integration, Faehigkeit, Ziel, Ergebnis
///   und dem Detail als Objekt; dazu Summen je Ergebnis, Aufrufer und Faehigkeit ueber die GANZE
///   Tabelle (nicht nur ueber die gelieferten Zeilen).
/// - **Gedeckelt**: die Tabelle haelt hoechstens `audit::MAX_ROWS` Zeilen (beim Schreiben fallen
///   die aeltesten, zuerst Verweigerungen); der Dump liefert hoechstens `limit` (nie mehr als
///   `MAX_ROWS`) und sagt mit `truncated`, ob mehr da ist. `retention` nennt die Grenze.
/// - Das Audit enthaelt nie ein Geheimnis (siehe `audit`); der Dump gibt es unveraendert wieder.
pub fn build_audit(
    conn: &Connection,
    db_path: Option<&Path>,
    limit: Option<u32>,
) -> Result<Value, IntegrationError> {
    let cap = audit::MAX_ROWS as u32;
    let limit = limit.unwrap_or(AUDIT_DUMP_DEFAULT_LIMIT).clamp(1, cap);
    let total = count(conn, "SELECT COUNT(*) FROM audit_log");
    let mut stmt = conn.prepare(
        "SELECT id, ts, caller, integration_id, capability, target, outcome, detail_json
         FROM audit_log ORDER BY id DESC LIMIT ?1",
    )?;
    let mut rows = stmt
        .query_map([i64::from(limit)], |r| {
            let detail: Option<String> = r.get(7)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "ts": r.get::<_, i64>(1)?,
                "caller": r.get::<_, String>(2)?,
                "integration_id": r.get::<_, Option<String>>(3)?,
                "capability": r.get::<_, Option<String>>(4)?,
                "target": r.get::<_, Option<String>>(5)?,
                "outcome": r.get::<_, String>(6)?,
                "detail": detail
                    .as_deref()
                    .and_then(|d| serde_json::from_str::<Value>(d).ok())
                    .unwrap_or(Value::Null),
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Neueste zuerst gelesen (LIMIT), aelteste zuerst ausgegeben.
    rows.reverse();
    let group = |column: &str| -> Result<Value, IntegrationError> {
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE({column}, '-'), COUNT(*) FROM audit_log GROUP BY 1 ORDER BY 1"
        ))?;
        let pairs = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Value::Object(pairs.into_iter().map(|(k, n)| (k, json!(n))).collect()))
    };
    Ok(json!({
        "schema_version": count(conn, "PRAGMA user_version"),
        "db": db_path.map(|p| p.display().to_string()),
        "retention": { "max_rows": audit::MAX_ROWS, "rows": total },
        "limit": limit,
        "returned": rows.len(),
        "truncated": (rows.len() as i64) < total,
        "by_outcome": group("outcome")?,
        "by_caller": group("caller")?,
        "by_capability": group("capability")?,
        "entries": rows,
    }))
}

/// Kurzform des Audit-Dumps fuer die Konsole (ohne `--json`).
pub fn format_audit_table(dump: &Value) -> String {
    let mut out = format!(
        "audit: {} of {} rows (retention cap {}){}\n",
        dump["returned"],
        dump["retention"]["rows"],
        dump["retention"]["max_rows"],
        if dump["truncated"].as_bool().unwrap_or(false) { ", truncated" } else { "" },
    );
    for e in dump["entries"].as_array().cloned().unwrap_or_default() {
        out.push_str(&format!(
            "  {:>6} {:<14} {:<18} {:<8} {}\n",
            e["id"],
            e["caller"].as_str().unwrap_or("?"),
            e["capability"].as_str().unwrap_or("-"),
            e["outcome"].as_str().unwrap_or("?"),
            e["target"].as_str().unwrap_or(""),
        ));
    }
    out
}

/// Kurzform fuer die Konsole (ohne `--json`).
pub fn format_table(dump: &Value) -> String {
    let mut out = String::new();
    let arr = dump["integrations"].as_array().cloned().unwrap_or_default();
    out.push_str(&format!("integrations: {}\n", arr.len()));
    for i in &arr {
        out.push_str(&format!(
            "  {:<28} {:<9} {:<5} {} ({})\n",
            i["id"].as_str().unwrap_or("?"),
            i["kind"].as_str().unwrap_or("?"),
            i["direction"].as_str().unwrap_or("?"),
            i["label"].as_str().unwrap_or("?"),
            if i["enabled"].as_bool().unwrap_or(false) {
                "on"
            } else {
                "off"
            },
        ));
    }
    out.push_str(&format!(
        "audit: {}, approvals pending: {}, provenance: {}\n",
        dump["counts"]["audit"], dump["counts"]["approvals_pending"], dump["counts"]["provenance"]
    ));
    out
}

#[cfg(test)]
mod tests;
