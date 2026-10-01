//! `--workflow-run <datei> --dry-run [--json] [--out F]` (B1, AK2): der Trockenlauf
//! einer Definitionsdatei gegen das Register einer Sandbox.
//!
//! Die Ein-/Ausgabe (Datei lesen, Sandbox verlangen, drucken, Exit-Code setzen) macht
//! `lib.rs`; hier steht die Rechnung, damit sie ohne Programmstart pruefbar ist.
//!
//! Garantie: nichts wird geschrieben, auch nicht ins Audit oder in die Freigaben, kein
//! Baustein laeuft, kein Modell startet (`plan`).
//!
//! Exit-Codes: 0 Plan erstellt, jeder Schritt erlaubt oder fragt; 3 Plan erstellt, aber
//! mindestens ein Schritt wuerde abgelehnt oder ist ungueltig; 2 die Datei ist keine
//! gueltige Definition (JSON mit allen Befunden); 1 interner Fehler (in `lib.rs`).

use rusqlite::Connection;
use serde_json::{json, Value};

use super::action::ActionRegistry;
use super::plan::{self, PLAN_SCHEMA};
use super::validate;

pub const EXIT_OK: i32 = 0;
pub const EXIT_BAD_INPUT: i32 = 2;
pub const EXIT_WOULD_BE_DENIED: i32 = 3;

pub struct DryRun {
    pub exit_code: i32,
    pub payload: Value,
}

/// Plant die Definition in `text` gegen das Register in `conn`.
pub fn dry_run(conn: &Connection, text: &str) -> DryRun {
    let def = match validate::parse_definition_str(text) {
        Ok(d) => d,
        Err(issues) => {
            return DryRun {
                exit_code: EXIT_BAD_INPUT,
                payload: json!({
                    "schema": PLAN_SCHEMA,
                    "dry_run": true,
                    "valid": false,
                    "writes": "nothing",
                    "issues": issues
                        .iter()
                        .map(|i| json!({"path": i.path, "message": i.message}))
                        .collect::<Vec<_>>(),
                }),
            };
        }
    };
    let registry = ActionRegistry::with_catalog();
    let mut payload = plan::plan_definition(conn, &registry, &def, None);
    payload["valid"] = json!(true);
    let blocked = payload["summary"]["denied"].as_u64().unwrap_or(0)
        + payload["summary"]["invalid"].as_u64().unwrap_or(0);
    DryRun {
        exit_code: if blocked > 0 {
            EXIT_WOULD_BE_DENIED
        } else {
            EXIT_OK
        },
        payload,
    }
}

/// Lesbare Fassung fuer die Konsole.
pub fn format_text(result: &DryRun) -> String {
    if result.payload["valid"].as_bool() == Some(false) {
        let mut out = String::from("Die Datei ist keine gültige Definition:\n");
        for i in result.payload["issues"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  {} {}\n",
                i["path"].as_str().unwrap_or(""),
                i["message"].as_str().unwrap_or("")
            ));
        }
        out
    } else {
        plan::format_table(&result.payload)
    }
}
