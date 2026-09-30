//! Eval fuer P1k: Vorlagenwahl und Protokoll auf synthetischen Fixtures.
//!
//! Der headless Lauf (`--eval-minutes <dir>`, lib.rs) spielt Fixtures
//! (`tests/fixtures/notes/*.json` fuer die Wahl, `tests/fixtures/minutes/*.json`
//! fuer lange Transkripte) in einen Sandbox-Store im Temp-Verzeichnis und laesst
//! den echten Motor (`generate_minutes_with_settings`) mit dem eingestellten
//! Modell laufen. Die produktive meetings.db wird nie geoeffnet.
//!
//! Je Fixture wird berichtet: welche Vorlage die Automatik gewaehlt hat (gegen
//! `template_key` der Fixture; leer = keine Erwartung), mit Begruendung, und wie
//! das Protokoll entstand (Einzeldurchlauf oder Bloecke, halbiert, Luecken,
//! Token). "Ohne verworfenen Block" heisst: keine Luecke und kein
//! `chunks_failed`.
//!
//! Kein `get_settings(&AppHandle)` hier: Einstellungen kommen als Parameter.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use super::classify;
use super::eval::{
    ledger_last_id, load_fixtures, usage_since_for, EXIT_ERROR, EXIT_MISSED, EXIT_OK,
};
use super::templates::{builtin_id, AUTO_TEMPLATE_ID};
use crate::managers::meetings::llm_call::resolve_provider_coded;
use crate::managers::meetings::minutes::{self, MinutesProgress};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, MeetingStore, TranscriptDelta,
};
use crate::managers::usage::Purpose;
use crate::settings::AppSettings;

/// Der headless Lauf `--eval-minutes <dir>`: alle Fixtures seriell gegen das
/// eingestellte Modell. `template`: `auto` (Standard) oder eine Vorlagen-ID.
/// `sandbox` ist ein frisches Temp-Verzeichnis (der Aufrufer raeumt es weg).
/// Liefert Exit-Code (0 alles gut, 3 falsche Vorlage oder Luecke, 1 Fehler) und
/// Bericht.
pub async fn run_cli(
    settings: AppSettings,
    dir: &Path,
    sandbox: &Path,
    template: &str,
) -> (i32, Value) {
    let fail = |message: String| {
        (
            EXIT_ERROR,
            json!({ "mode": "eval-minutes", "error": message }),
        )
    };
    let fixtures = match load_fixtures(dir) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    let (provider, model, _) = match resolve_provider_coded(&settings) {
        Ok(p) => p,
        Err(e) => return fail(format!("{}: {}", e.code, e.message)),
    };
    let local = crate::managers::llm::is_local(&provider);
    let store = match MeetingStore::open_at(&sandbox.join("meetings.db")) {
        Ok(s) => Arc::new(s),
        Err(e) => return fail(format!("Sandbox-Store nicht anlegbar: {e}")),
    };
    match crate::managers::usage::UsageLedger::open(Path::new(":memory:")) {
        Ok(ledger) => {
            let source = settings.clone();
            crate::managers::usage::install_globals(
                Arc::new(ledger),
                Arc::new(move || source.clone()),
            );
        }
        Err(e) => log::warn!("eval-minutes: kein Ledger, Token-Zaehler fehlen ({e})"),
    }
    let context_tokens = if local {
        Some(crate::managers::llm::context_for_model(&model).await)
    } else {
        None
    };

    let started = Instant::now();
    let mut reports = Vec::new();
    let (mut had_error, mut mismatches, mut gaps) = (false, 0usize, 0usize);
    for (name, fixture) in &fixtures {
        eprintln!("eval-minutes: {name} ...");
        let expected = (!fixture.template_key.trim().is_empty())
            .then(|| builtin_id(fixture.template_key.trim()));
        let created = (|| -> Result<String, String> {
            let e = |err: anyhow::Error| err.to_string();
            let meeting = store
                .create_meeting(
                    &fixture.title,
                    MeetingSource::Live,
                    Some(chrono::Utc::now().timestamp()),
                )
                .map_err(e)?;
            store
                .append_delta(
                    &meeting.id,
                    &TranscriptDelta {
                        new_segments: fixture.segments.clone(),
                    },
                )
                .map_err(e)?;
            store
                .set_status(&meeting.id, MeetingStatus::Ready)
                .map_err(e)?;
            Ok(meeting.id)
        })();
        let mut report = json!({
            "name": name,
            "title": fixture.title,
            "segments": fixture.segments.len(),
            "expected_template": expected,
            "requested_template": template,
        });
        let meeting_id = match created {
            Ok(id) => id,
            Err(error) => {
                had_error = true;
                report["error"] = json!(error);
                reports.push(report);
                continue;
            }
        };
        let before = ledger_last_id();
        let began = Instant::now();
        let last_line = std::sync::Mutex::new(String::new());
        let result = minutes::generate_minutes_with_settings(
            &settings,
            store.clone(),
            &meeting_id,
            Some(template),
            &|p: &MinutesProgress| {
                let line = format!("{:?} {}/{}", p.phase, p.done, p.total);
                let mut last = last_line.lock().unwrap_or_else(|e| e.into_inner());
                if *last != line {
                    eprintln!("eval-minutes: {name} {line}");
                    *last = line;
                }
            },
        )
        .await;
        let elapsed_ms = began.elapsed().as_millis() as u64;
        let usage = usage_since_for(before, Purpose::Minutes).await;
        report["elapsed_ms"] = json!(elapsed_ms);
        report["usage"] = json!(usage);
        match result {
            Ok(document) => {
                let meta = minutes::latest_meta(&store, &meeting_id);
                let chosen = meta.as_ref().and_then(|m| m.template_id.clone());
                let auto = meta.as_ref().and_then(|m| m.auto.clone());
                let matched = match (&expected, template == AUTO_TEMPLATE_ID) {
                    (Some(want), true) => Some(chosen.as_deref() == Some(want.as_str())),
                    _ => None,
                };
                if matched == Some(false) {
                    mismatches += 1;
                }
                let incomplete = meta.as_ref().is_some_and(|m| m.incomplete);
                if incomplete {
                    gaps += 1;
                }
                let stored = classify::stored_choice(&store, &meeting_id);
                report["chosen_template"] = json!(chosen);
                report["chosen_title"] =
                    json!(meta.as_ref().and_then(|m| m.template_title.clone()));
                report["auto"] = json!(auto);
                report["stored_choice"] = json!(stored);
                report["template_matches"] = json!(matched);
                report["meta"] = json!(meta);
                report["minutes_chars"] = json!(document.body.chars().count());
                report["minutes_markdown"] = json!(document.body);
                report["dropped_blocks"] = json!(incomplete);
            }
            Err(error) => {
                had_error = true;
                eprintln!("eval-minutes: {name} fehlgeschlagen: {error}");
                report["error"] = json!(error);
            }
        }
        reports.push(report);
    }

    let code = if had_error {
        EXIT_ERROR
    } else if mismatches > 0 || gaps > 0 {
        EXIT_MISSED
    } else {
        EXIT_OK
    };
    let payload = json!({
        "mode": "eval-minutes",
        "provider": provider.id,
        "model": model,
        "local": local,
        "context_tokens": context_tokens,
        "requested_template": template,
        "fixtures": reports,
        "aggregate": {
            "fixtures": fixtures.len(),
            "template_mismatches": mismatches,
            "with_gaps": gaps,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        },
        "passed": code == EXIT_OK,
        "exit_code": code,
    });
    drop(store);
    (code, payload)
}

/// Kurzfassung fuer die Konsole (ohne `--json`).
pub fn summary_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = payload.get("error").and_then(Value::as_str) {
        lines.push(format!("eval-minutes: Fehler: {error}"));
        return lines;
    }
    lines.push(format!(
        "eval-minutes: Modell {} ({}), Vorlage {}",
        payload["model"].as_str().unwrap_or("?"),
        payload["provider"].as_str().unwrap_or("?"),
        payload["requested_template"].as_str().unwrap_or("?")
    ));
    for f in payload["fixtures"].as_array().into_iter().flatten() {
        let name = f["name"].as_str().unwrap_or("?");
        if let Some(error) = f.get("error").and_then(Value::as_str) {
            lines.push(format!("  {name:<22} FEHLER {error}"));
            continue;
        }
        lines.push(format!(
            "  {name:<22} erwartet {:<22} gewaehlt {:<22} {}  {:.1} s  Bloecke {} (halbiert {}), Luecken {}",
            f["expected_template"].as_str().unwrap_or("-"),
            f["chosen_template"].as_str().unwrap_or("?"),
            match f["template_matches"].as_bool() {
                Some(true) => "OK",
                Some(false) => "FALSCH",
                None => "-",
            },
            f["elapsed_ms"].as_f64().unwrap_or(0.0) / 1000.0,
            f["meta"]["chunks_total"],
            f["meta"]["chunks_split"],
            f["meta"]["gaps"].as_array().map_or(0, Vec::len),
        ));
    }
    lines
}
