//! Die App aktualisiert die Codex-CLI nicht selbst.
//!
//! Nimmt eine veraltete CLI ein Modell nicht an, erhaelt die Oberflaeche ueber
//! das Ereignis `cli-update` nur einen Hinweis auf den Verteilweg der IT.

use std::sync::OnceLock;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::cli::Cli;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Was die Oberflaeche erfaehrt (`state`: `manual`).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CliUpdateEvent {
    pub cli: String,
    pub state: String,
    pub version: Option<String>,
    pub message: Option<String>,
}

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Nur dieser Fehler heisst "CLI zu alt": der Server nimmt das Modell von dieser
/// Version nicht an.
pub fn is_cli_too_old(err: &str) -> bool {
    err.starts_with("cli_model_not_in_plan")
}

fn emit(state: &str, version: Option<String>, message: Option<String>) {
    if let Some(app) = APP.get() {
        let _ = app.emit(
            "cli-update",
            CliUpdateEvent {
                cli: "codex".into(),
                state: state.into(),
                version,
                message,
            },
        );
    }
}

/// Hat dieser Fehler mit einer zu alten Codex-CLI zu tun?
pub fn on_cli_error(which: Cli, err: &str) -> bool {
    which == Cli::Codex && is_cli_too_old(err)
}

/// Meldet nur den manuellen Verteilweg; die App aktualisiert die CLI nie selbst.
pub fn ensure_codex_updated() -> bool {
    emit(
        "manual",
        None,
        Some(
            "Die Codex-CLI wird von der App nicht mehr aktualisiert. Bitte ueber den Verteilweg \
             der IT aktualisieren lassen (siehe docs/FIRMENNETZ.md)."
                .into(),
        ),
    );
    false
}

/// Die Ursache des letzten gescheiterten CLI-Aufrufs als Zusatz fuer eine
/// Fehlermeldung ("" wenn keiner juengst gescheitert ist).
static LAST_ERROR: std::sync::Mutex<Option<(i64, String)>> = std::sync::Mutex::new(None);

pub fn remember_error(err: &str) {
    if let Ok(mut last) = LAST_ERROR.lock() {
        *last = Some((now(), err.to_string()));
    }
}

/// ` (Ursache: …)`, wenn in den letzten zwei Minuten ein CLI-Aufruf gescheitert ist.
pub fn recent_error_note() -> String {
    LAST_ERROR
        .lock()
        .ok()
        .and_then(|l| l.clone())
        .filter(|(at, _)| now() - at <= 120)
        .map(|(_, e)| {
            format!(
                " (Ursache: {})",
                e.split_once(": ").map_or(e.as_str(), |x| x.1)
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_model_not_accepted_error_means_too_old() {
        assert!(is_cli_too_old(
            "cli_model_not_in_plan: Dieses Modell nimmt der Server …"
        ));
        for e in [
            "cli_failed: x",
            "cli_limit_reached: x",
            "cli_not_logged_in: x",
            "cli_missing: x",
        ] {
            assert!(!is_cli_too_old(e), "{e}");
        }
    }

    #[test]
    fn ensure_codex_updated_meldet_nur_manuell_und_liefert_false() {
        assert!(!ensure_codex_updated());
        assert!(!ensure_codex_updated());
    }

    #[test]
    fn produktionscode_startet_keine_prozesse() {
        let src = include_str!("cli_update.rs");
        let produktion = src.split("#[cfg(test)]").next().unwrap();
        for fragment in [
            "Command",
            "spawn",
            "npm",
            "std::process",
            "cmd.exe",
            "powershell",
        ] {
            assert!(
                !produktion.contains(fragment),
                "Verbotenes Fragment: {fragment}"
            );
        }
    }

    #[test]
    fn the_error_note_names_the_cause_without_the_code_prefix() {
        remember_error("cli_model_not_in_plan: Codex-CLI zu alt");
        assert_eq!(recent_error_note(), " (Ursache: Codex-CLI zu alt)");
    }
}
