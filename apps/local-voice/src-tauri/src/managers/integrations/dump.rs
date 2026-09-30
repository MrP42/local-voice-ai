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
    for cap in i.kind.capabilities() {
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
