//! Eval fuer AK3 (M1, P1e): Wie gut sind die KI-Notizen wirklich?
//!
//! Zwei Soll-Werte, unabhaengig von der Konstruktion in `assemble`
//! nachgeprueft (das Eval vertraut weder `EnhanceStats` noch `EntryFlags`):
//! - `user_preserved_ratio` = 1,0: jeder nicht leere Nutzerstichpunkt steht
//!   woertlich (byte-genau) und als Nutzertext (`origin = user`, `note_id`)
//!   in den KI-Notizen;
//! - `ai_sourced_ratio` >= 0,95: KI-Eintraege mit mindestens einer Quelle,
//!   deren Segmente es alle gibt.
//!
//! Dazu als Info `lexical_support_ratio`: Anteil der KI-Eintraege, die mit
//! einer ihrer Quellen mindestens ein Inhaltswort (>= 4 Zeichen, kein
//! Fuellwort) teilen -- ein grober Hinweis, ob die Quelle zum Satz passt.
//!
//! Der headless Lauf (`--eval-notes <dir>`, lib.rs) spielt synthetische
//! Fixtures (`tests/fixtures/notes/*.json`) in einen Sandbox-Store im
//! Temp-Verzeichnis und laesst den echten Motor (`enhance_meeting`) mit dem
//! eingestellten Modell laufen. Die produktive meetings.db wird nie geoeffnet.
//!
//! Kein `get_settings(&AppHandle)` hier: Einstellungen kommen als Parameter.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::enhance::{
    enhance_meeting, parse_enhanced, render_notes_for_prompt, render_segments_for_prompt,
    single_pass_budget_chars_for,
};
use super::model::{EnhancedNotes, NoteBlock, Origin};
use super::templates::builtin_id;
use crate::managers::meetings::llm_call::{resolve_provider_coded, sorted_segments};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta,
};
use crate::settings::AppSettings;

/// Soll-Werte aus AK3.
pub const USER_PRESERVED_TARGET: f64 = 1.0;
pub const AI_SOURCED_TARGET: f64 = 0.95;

/// Exit-Codes des headless Laufs.
pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_MISSED: i32 = 3;

// ---------------------------------------------------------------------------
// Metriken
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EvalMetrics {
    pub user_notes_total: u32,
    pub user_notes_preserved: u32,
    pub user_preserved_ratio: f64,
    pub ai_entries: u32,
    pub ai_entries_sourced: u32,
    pub ai_sourced_ratio: f64,
    pub ai_entries_lexical: u32,
    pub lexical_support_ratio: f64,
}

/// Anteil; ohne Nenner 1,0 (nichts zu pruefen). Dass „keine KI-Eintraege"
/// trotzdem kein Bestehen ist, entscheidet `targets_met`.
fn ratio(part: u32, total: u32) -> f64 {
    if total == 0 {
        1.0
    } else {
        f64::from(part) / f64::from(total)
    }
}

impl EvalMetrics {
    fn from_counts(
        user_total: u32,
        user_preserved: u32,
        ai: u32,
        ai_sourced: u32,
        ai_lexical: u32,
    ) -> Self {
        Self {
            user_notes_total: user_total,
            user_notes_preserved: user_preserved,
            user_preserved_ratio: ratio(user_preserved, user_total),
            ai_entries: ai,
            ai_entries_sourced: ai_sourced,
            ai_sourced_ratio: ratio(ai_sourced, ai),
            ai_entries_lexical: ai_lexical,
            lexical_support_ratio: ratio(ai_lexical, ai),
        }
    }
}

/// Deutsche Fuellwoerter mit mindestens vier Zeichen: sie kommen in fast
/// jedem Satz vor und wuerden die lexikalische Stuetze beliebig machen.
const STOPWORDS: &[&str] = &[
    "aber", "alle", "allen", "alles", "also", "auch", "beim", "bitte", "damit", "dann", "dass",
    "dazu", "dafür", "denn", "diese", "diesem", "diesen", "dieser", "dieses", "doch", "dort",
    "durch", "eine", "einem", "einen", "einer", "eines", "etwa", "ganz", "gern", "gerne", "gibt",
    "haben", "hatte", "hier", "immer", "jetzt", "kann", "können", "mehr", "muss", "müssen", "nach",
    "nicht", "noch", "oder", "ohne", "schon", "sehr", "sein", "seine", "sich", "sind", "soll",
    "sollen", "sowie", "über", "unter", "weil", "wenn", "werden", "wird", "wurde", "zwischen",
];

fn content_words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 4 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Die Metriken einer KI-Notizen-Ausgabe gegen die Eingabe (Notizblock und
/// Segmente). Rein, ohne I/O; liest bewusst nicht `stats`/`flags`.
pub fn metrics(
    out: &EnhancedNotes,
    notes: &[NoteBlock],
    segments: &[StoredSegment],
) -> EvalMetrics {
    let entries: Vec<_> = out.sections.iter().flat_map(|s| s.entries.iter()).collect();

    // Nutzerstichpunkte: jeder nicht leere Block muss mindestens einmal als
    // Nutzereintrag mit seiner ID auftauchen, und jeder dieser Eintraege
    // traegt den Text byte-genau.
    let mut user_total = 0u32;
    let mut user_preserved = 0u32;
    for block in notes.iter().filter(|b| !b.text.trim().is_empty()) {
        user_total += 1;
        let mine: Vec<_> = entries
            .iter()
            .filter(|e| e.origin == Origin::User && e.note_id.as_deref() == Some(block.id.as_str()))
            .collect();
        if !mine.is_empty() && mine.iter().all(|e| e.text == block.text) {
            user_preserved += 1;
        }
    }

    // KI-Eintraege: belegt = mindestens eine Quelle, und alle Quellen gibt es.
    let by_index: std::collections::HashMap<u32, &StoredSegment> =
        segments.iter().map(|s| (s.segment_index, s)).collect();
    let (mut ai, mut ai_sourced, mut ai_lexical) = (0u32, 0u32, 0u32);
    for entry in entries.iter().filter(|e| e.origin == Origin::Ai) {
        ai += 1;
        let ids = &entry.source_segment_ids;
        if ids.is_empty() || !ids.iter().all(|id| by_index.contains_key(id)) {
            continue;
        }
        ai_sourced += 1;
        let words = content_words(&entry.text);
        if ids
            .iter()
            .any(|id| !words.is_disjoint(&content_words(&by_index[id].text)))
        {
            ai_lexical += 1;
        }
    }
    EvalMetrics::from_counts(user_total, user_preserved, ai, ai_sourced, ai_lexical)
}

/// Summe ueber mehrere Laeufe (Mikro-Mittel: Zaehler addieren, dann teilen).
pub fn aggregate(items: &[EvalMetrics]) -> EvalMetrics {
    let sum = |f: fn(&EvalMetrics) -> u32| items.iter().map(f).sum::<u32>();
    EvalMetrics::from_counts(
        sum(|m| m.user_notes_total),
        sum(|m| m.user_notes_preserved),
        sum(|m| m.ai_entries),
        sum(|m| m.ai_entries_sourced),
        sum(|m| m.ai_entries_lexical),
    )
}

/// Soll AK3 erfuellt? Ohne einen einzigen KI-Eintrag nein: dann hat das
/// Modell nichts ergaenzt, und 100 % „belegt" waeren eine leere Aussage.
pub fn targets_met(m: &EvalMetrics) -> bool {
    m.user_preserved_ratio >= USER_PRESERVED_TARGET
        && m.ai_entries > 0
        && m.ai_sourced_ratio >= AI_SOURCED_TARGET
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Eine synthetische Besprechung (`tests/fixtures/notes/*.json`).
#[derive(Clone, Debug, Deserialize)]
pub struct NotesFixture {
    pub title: String,
    /// Schluessel einer mitgelieferten Vorlage (`builtin:<key>`).
    pub template_key: String,
    pub segments: Vec<StoredSegment>,
    pub notes: Vec<NoteBlock>,
}

/// Alle `*.json` eines Verzeichnisses, nach Dateiname sortiert. Name = Datei
/// ohne Endung.
pub fn load_fixtures(dir: &Path) -> Result<Vec<(String, NotesFixture)>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Fixture-Verzeichnis {} nicht lesbar: {e}", dir.display()))?;
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        })
        .collect();
    paths.sort();
    let mut fixtures = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("{} nicht lesbar: {e}", path.display()))?;
        let fixture: NotesFixture = serde_json::from_str(&text)
            .map_err(|e| format!("{} ist kein Fixture: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        fixtures.push((name, fixture));
    }
    if fixtures.is_empty() {
        return Err(format!("keine Fixtures (*.json) in {}", dir.display()));
    }
    Ok(fixtures)
}

/// Der Teil des Prompts, den das Einzeldurchlauf-Budget zaehlt: gerenderte
/// Notizen und gerendertes Transkript (wie `run_enhance`).
pub fn payload_text(fixture: &NotesFixture) -> String {
    format!(
        "{}\n{}",
        render_notes_for_prompt(&fixture.notes),
        render_segments_for_prompt(&sorted_segments(&fixture.segments))
    )
}

// ---------------------------------------------------------------------------
// Lauf
// ---------------------------------------------------------------------------

pub struct FixtureOutcome {
    pub template_id: String,
    pub notes: EnhancedNotes,
    pub metrics: EvalMetrics,
    pub elapsed_ms: u64,
}

/// Eine Fixture als fertige Besprechung in den (Sandbox-)Store legen und den
/// echten Motor darueber laufen lassen.
pub async fn run_fixture(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    fixture: &NotesFixture,
) -> Result<FixtureOutcome, String> {
    let e = |err: anyhow::Error| err.to_string();
    let template_id = builtin_id(&fixture.template_key);
    if store.get_template_info(&template_id).map_err(e)?.is_none() {
        return Err(format!("unbekannte Vorlage: {}", fixture.template_key));
    }
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
    store
        .save_notes(&meeting.id, &fixture.notes, 0)
        .map_err(e)?;

    let started = Instant::now();
    let document = enhance_meeting(
        settings,
        store.clone(),
        &meeting.id,
        Some(&template_id),
        |_, _| {},
    )
    .await?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let notes = parse_enhanced(&document).map_err(String::from)?;
    let segments = store.get_segments(&meeting.id).map_err(e)?;
    let metrics = metrics(&notes, &fixture.notes, &segments);
    Ok(FixtureOutcome {
        template_id,
        notes,
        metrics,
        elapsed_ms,
    })
}

/// `--model` fuer das Eval: das lokale Modell dieser Kennung (Katalog-ID,
/// z. B. `llm-qwen3.5-9b-q4`) statt des eingestellten. Nur im Speicher.
pub fn apply_model_override(settings: &mut AppSettings, model: &str) {
    let local = crate::managers::llm::LOCAL_PROVIDER_ID;
    settings.post_process_provider_id = local.to_string();
    settings
        .post_process_models
        .insert(local.to_string(), model.to_string());
    settings.llm_active_model_id = Some(format!("{local}:{model}"));
}

/// Token-Zaehler der Aufrufe eines Fixture-Laufs (aus dem Verbrauchs-Ledger,
/// also aus `usage` der Antworten).
#[derive(Clone, Debug, Default, Serialize)]
pub struct UsageTotals {
    pub calls: u32,
    pub failed_calls: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Prompt-Token des ersten erfolgreichen Aufrufs (Einzeldurchlauf: der
    /// ganze Prompt).
    pub first_prompt_tokens: Option<u64>,
    /// Summe der Aufrufdauern (der erste Aufruf enthaelt den Serverstart).
    pub llm_ms: u64,
}

fn ledger_last_id() -> i64 {
    crate::managers::usage::ledger()
        .and_then(|l| l.events(1, 0).ok())
        .and_then(|events| events.first().map(|e| e.id))
        .unwrap_or(0)
}

/// Die Buchungen nach `after_id`. `record_call` schreibt im Hintergrund:
/// warten, bis die Zahl 300 ms lang stabil ist (hoechstens 3 s).
async fn usage_since(after_id: i64) -> UsageTotals {
    let Some(ledger) = crate::managers::usage::ledger() else {
        return UsageTotals::default();
    };
    let purpose = crate::managers::usage::Purpose::EnhancedNotes.as_str();
    let mut events = Vec::new();
    let (mut last, mut stable) = (usize::MAX, 0);
    for _ in 0..30 {
        events = ledger
            .events(10_000, 0)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.id > after_id && e.purpose == purpose)
            .collect();
        if events.len() == last {
            stable += 1;
            if stable >= 3 {
                break;
            }
        } else {
            (last, stable) = (events.len(), 0);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    events.sort_by_key(|e| e.id);
    UsageTotals {
        calls: events.len() as u32,
        failed_calls: events.iter().filter(|e| !e.ok).count() as u32,
        prompt_tokens: events.iter().map(|e| u64::from(e.prompt_tokens)).sum(),
        completion_tokens: events.iter().map(|e| u64::from(e.completion_tokens)).sum(),
        first_prompt_tokens: events
            .iter()
            .find(|e| e.ok)
            .map(|e| u64::from(e.prompt_tokens)),
        llm_ms: events.iter().map(|e| u64::from(e.duration_ms)).sum(),
    }
}

/// Token eines Texts ueber `/tokenize` des laufenden lokalen Servers: misst
/// genau die Groesse, die `single_pass_budget_chars_for` in Zeichen schaetzt.
async fn count_tokens(model: &str, text: &str) -> Option<u64> {
    let root = crate::managers::llm::local_server_root(model).await?;
    crate::managers::llm::tokenize_count(&root, text)
        .await
        .map(|count| count as u64)
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Der headless Lauf `--eval-notes <dir>`: alle Fixtures seriell gegen das
/// eingestellte Modell. `sandbox` ist ein frisches Temp-Verzeichnis (der
/// Aufrufer raeumt es weg). Liefert Exit-Code und Bericht.
///
/// Installiert einen Verbrauchs-Ledger im Speicher, damit die Token-Zaehler
/// der Antworten lesbar sind -- nur fuer headless Prozesse gedacht (in der
/// App ist der produktive Ledger schon gesetzt und bleibt es).
pub async fn run_cli(settings: AppSettings, dir: &Path, sandbox: &Path) -> (i32, Value) {
    let fail = |message: String| {
        (
            EXIT_ERROR,
            json!({ "mode": "eval-notes", "error": message }),
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
        Err(e) => log::warn!("eval-notes: kein Ledger, Token-Zaehler fehlen ({e})"),
    }

    // Kontext wie ihn ein Serverstart jetzt waehlt (je freiem VRAM, P1g); das
    // Budget richtet sich danach.
    let context_tokens = if local {
        Some(crate::managers::llm::context_for_model(&model).await)
    } else {
        None
    };
    let budget = single_pass_budget_chars_for(
        &model,
        context_tokens.unwrap_or(crate::managers::llm::DEFAULT_CONTEXT_TOKENS),
        local,
    );
    let started = Instant::now();
    let mut reports = Vec::new();
    let mut all = Vec::new();
    let mut had_error = false;
    let (mut sum_chars, mut sum_tokens) = (0u64, 0u64);
    for (name, fixture) in &fixtures {
        eprintln!("eval-notes: {name} ...");
        let before = ledger_last_id();
        let result = run_fixture(&settings, store.clone(), fixture).await;
        let usage = usage_since(before).await;
        let payload = payload_text(fixture);
        let payload_chars = payload.chars().count() as u64;
        let payload_tokens = match (&result, local) {
            (Ok(_), true) => count_tokens(&model, &payload).await,
            _ => None,
        };
        if let Some(tokens) = payload_tokens.filter(|t| *t > 0) {
            sum_chars += payload_chars;
            sum_tokens += tokens;
        }
        let chars_per_token = payload_tokens
            .filter(|t| *t > 0)
            .map(|t| round3(payload_chars as f64 / t as f64));
        let mut report = json!({
            "name": name,
            "title": fixture.title,
            "template_key": fixture.template_key,
            "segments": fixture.segments.len(),
            "notes": fixture.notes.iter().filter(|b| !b.text.trim().is_empty()).count(),
            "notes_without_at_ms": fixture.notes.iter().filter(|b| b.at_ms.is_none()).count(),
            "payload_chars": payload_chars,
            "payload_tokens": payload_tokens,
            "chars_per_token": chars_per_token,
            "usage": usage,
        });
        match result {
            Ok(outcome) => {
                let stats = &outcome.notes.stats;
                // Prompt-Overhead (System, Kopf, Vorlage, Chat-Vorlage) nur im
                // Einzeldurchlauf mit genau einem Aufruf eindeutig.
                let overhead = match (
                    stats.single_pass,
                    usage.calls,
                    usage.first_prompt_tokens,
                    payload_tokens,
                ) {
                    (true, 1, Some(prompt), Some(payload)) => Some(prompt as i64 - payload as i64),
                    _ => None,
                };
                let unsourced: Vec<Value> = outcome
                    .notes
                    .sections
                    .iter()
                    .flat_map(|s| s.entries.iter().map(move |e| (s, e)))
                    .filter(|(_, e)| {
                        e.origin == Origin::Ai
                            && (e.source_segment_ids.is_empty() || e.flags.unsupported)
                    })
                    .map(|(s, e)| json!({ "section": s.id, "text": e.text }))
                    .collect();
                let obj = report.as_object_mut().expect("json object");
                obj.insert("template_id".into(), json!(outcome.template_id));
                obj.insert("elapsed_ms".into(), json!(outcome.elapsed_ms));
                obj.insert("single_pass".into(), json!(stats.single_pass));
                obj.insert("chunks_total".into(), json!(stats.chunks_total));
                obj.insert("chunks_failed".into(), json!(stats.chunks_failed));
                obj.insert("prompt_overhead_tokens".into(), json!(overhead));
                obj.insert("metrics".into(), json!(outcome.metrics));
                obj.insert("targets_met".into(), json!(targets_met(&outcome.metrics)));
                obj.insert("stats".into(), json!(stats));
                obj.insert("unsourced_ai_entries".into(), json!(unsourced));
                obj.insert("enhanced".into(), json!(outcome.notes));
                all.push(outcome.metrics);
            }
            Err(error) => {
                had_error = true;
                eprintln!("eval-notes: {name} fehlgeschlagen: {error}");
                report
                    .as_object_mut()
                    .expect("json object")
                    .insert("error".into(), json!(error));
            }
        }
        reports.push(report);
    }

    let aggregate = aggregate(&all);
    let passed = !had_error && targets_met(&aggregate);
    let code = if had_error {
        EXIT_ERROR
    } else if passed {
        EXIT_OK
    } else {
        EXIT_MISSED
    };
    let mut aggregate_json = json!(aggregate);
    let obj = aggregate_json.as_object_mut().expect("json object");
    obj.insert("fixtures".into(), json!(reports.len()));
    obj.insert("failed_fixtures".into(), json!(reports.len() - all.len()));
    obj.insert(
        "elapsed_ms".into(),
        json!(started.elapsed().as_millis() as u64),
    );
    obj.insert(
        "chars_per_token".into(),
        json!((sum_tokens > 0).then(|| round3(sum_chars as f64 / sum_tokens as f64))),
    );
    let payload = json!({
        "mode": "eval-notes",
        "provider": provider.id,
        "model": model,
        "local": local,
        "context_tokens": context_tokens,
        "single_pass_budget_chars": budget,
        "chars_per_token_assumed": super::budget::chars_per_token_x100(&model) as f64 / 100.0,
        "targets": {
            "user_preserved_ratio": USER_PRESERVED_TARGET,
            "ai_sourced_ratio": AI_SOURCED_TARGET,
        },
        "fixtures": reports,
        "aggregate": aggregate_json,
        "passed": passed,
        "exit_code": code,
    });
    drop(store);
    (code, payload)
}

/// Kurzfassung fuer die Konsole (ohne `--json`).
pub fn summary_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = payload.get("error").and_then(Value::as_str) {
        lines.push(format!("eval-notes: Fehler: {error}"));
        return lines;
    }
    lines.push(format!(
        "eval-notes: Modell {} ({}), Budget Einzeldurchlauf {} Zeichen",
        payload["model"].as_str().unwrap_or("?"),
        payload["provider"].as_str().unwrap_or("?"),
        payload["single_pass_budget_chars"]
    ));
    for f in payload["fixtures"].as_array().into_iter().flatten() {
        if let Some(error) = f.get("error").and_then(Value::as_str) {
            lines.push(format!(
                "  {:<20} FEHLER {error}",
                f["name"].as_str().unwrap_or("?")
            ));
            continue;
        }
        let m = &f["metrics"];
        lines.push(format!(
            "  {:<20} Nutzer {}/{} ({:.3})  KI belegt {}/{} ({:.3})  lexikalisch {:.3}  {}  {:.1} s  {} Zeichen/Token",
            f["name"].as_str().unwrap_or("?"),
            m["user_notes_preserved"],
            m["user_notes_total"],
            m["user_preserved_ratio"].as_f64().unwrap_or(0.0),
            m["ai_entries_sourced"],
            m["ai_entries"],
            m["ai_sourced_ratio"].as_f64().unwrap_or(0.0),
            m["lexical_support_ratio"].as_f64().unwrap_or(0.0),
            if f["single_pass"].as_bool() == Some(true) {
                "Einzeldurchlauf".to_string()
            } else {
                format!("Map-Reduce ({} Bloecke)", f["chunks_total"])
            },
            f["elapsed_ms"].as_f64().unwrap_or(0.0) / 1000.0,
            f["chars_per_token"],
        ));
    }
    let a = &payload["aggregate"];
    let failed = a["failed_fixtures"].as_u64().unwrap_or(0);
    lines.push(format!(
        "  gesamt: Nutzer {:.3} (Soll 1,0)  KI belegt {:.3} (Soll 0,95){}  -> {}",
        a["user_preserved_ratio"].as_f64().unwrap_or(0.0),
        a["ai_sourced_ratio"].as_f64().unwrap_or(0.0),
        if failed > 0 {
            format!("  [{failed} Fixture(s) fehlgeschlagen, nur der Rest gemessen]")
        } else {
            String::new()
        },
        if payload["passed"].as_bool() == Some(true) {
            "SOLL ERFUELLT"
        } else {
            "SOLL VERFEHLT"
        }
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::notes::model::{
        EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags, NoteBlockKind, SectionKind,
    };

    fn note(id: &str, kind: NoteBlockKind, text: &str) -> NoteBlock {
        NoteBlock {
            id: id.into(),
            kind,
            text: text.into(),
            at_ms: Some(1_000),
            checked: false,
        }
    }

    fn seg(index: u32, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.into(),
            start_ms: u64::from(index) * 5_000,
            end_ms: u64::from(index) * 5_000 + 4_000,
            channel: 0,
            speaker_index: None,
            words: None,
        }
    }

    fn user(note_id: &str, text: &str) -> EnhancedEntry {
        EnhancedEntry {
            id: String::new(),
            origin: Origin::User,
            text: text.into(),
            note_id: Some(note_id.into()),
            source_segment_ids: Vec::new(),
            assignee: None,
            due: None,
            flags: EntryFlags::default(),
        }
    }

    fn ai(text: &str, sources: &[u32]) -> EnhancedEntry {
        EnhancedEntry {
            id: String::new(),
            origin: Origin::Ai,
            text: text.into(),
            note_id: None,
            source_segment_ids: sources.to_vec(),
            assignee: None,
            due: None,
            flags: EntryFlags::default(),
        }
    }

    fn enhanced(entries: Vec<EnhancedEntry>) -> EnhancedNotes {
        EnhancedNotes {
            format: "enhanced@1".into(),
            template_id: None,
            template_title: "Test".into(),
            segment_epoch: 0,
            sections: vec![EnhancedSection {
                id: "a".into(),
                title: "A".into(),
                kind: SectionKind::Text,
                entries,
            }],
            stats: EnhanceStats::default(),
        }
    }

    fn base() -> (Vec<NoteBlock>, Vec<StoredSegment>) {
        (
            vec![
                note("B1", NoteBlockKind::Bullet, "Preis zu hoch?!"),
                note("B2", NoteBlockKind::Bullet, "   "),
                note("B3", NoteBlockKind::Todo, "Angebot bis Freitag"),
            ],
            vec![
                seg(0, "Der Preis pro Monteur muss runter."),
                seg(1, "Ich schicke das Angebot bis Freitag."),
            ],
        )
    }

    #[test]
    fn verbatim_user_notes_and_sourced_ai_entries_reach_the_targets() {
        let (notes, segments) = base();
        let out = enhanced(vec![
            user("B1", "Preis zu hoch?!"),
            ai("Der Preis pro Monteur soll sinken.", &[0]),
            user("B3", "Angebot bis Freitag"),
            ai("Das Angebot kommt bis Freitag.", &[1]),
        ]);
        let m = metrics(&out, &notes, &segments);
        // Der leere Block B2 zaehlt nicht.
        assert_eq!((m.user_notes_total, m.user_notes_preserved), (2, 2));
        assert_eq!(m.user_preserved_ratio, 1.0);
        assert_eq!((m.ai_entries, m.ai_entries_sourced), (2, 2));
        assert_eq!(m.ai_sourced_ratio, 1.0);
        assert_eq!(m.ai_entries_lexical, 2);
    }

    #[test]
    fn a_changed_or_missing_user_text_drops_user_preserved_below_one() {
        let (notes, segments) = base();
        // Geaenderter Nutzertext (ein Zeichen anders).
        let changed = enhanced(vec![
            user("B1", "Preis zu hoch?"),
            user("B3", "Angebot bis Freitag"),
        ]);
        let m = metrics(&changed, &notes, &segments);
        assert_eq!(m.user_notes_preserved, 1);
        assert!(m.user_preserved_ratio < 1.0);

        // Richtiger Text, aber als KI-Eintrag: nicht als Nutzertext erhalten.
        let as_ai = enhanced(vec![
            ai("Preis zu hoch?!", &[0]),
            user("B3", "Angebot bis Freitag"),
        ]);
        assert!(metrics(&as_ai, &notes, &segments).user_preserved_ratio < 1.0);

        // Fehlende Notiz.
        let missing = enhanced(vec![user("B1", "Preis zu hoch?!")]);
        assert_eq!(
            metrics(&missing, &notes, &segments).user_preserved_ratio,
            0.5
        );

        // Doppelt, einmal davon veraendert: nicht erhalten.
        let twice = enhanced(vec![
            user("B1", "Preis zu hoch?!"),
            user("B1", "Preis zu hoch"),
            user("B3", "Angebot bis Freitag"),
        ]);
        assert!(metrics(&twice, &notes, &segments).user_preserved_ratio < 1.0);
    }

    #[test]
    fn ai_entries_without_a_valid_source_are_unsourced() {
        let (notes, segments) = base();
        let out = enhanced(vec![
            user("B1", "Preis zu hoch?!"),
            user("B3", "Angebot bis Freitag"),
            ai("Belegt.", &[1]),
            ai("Ohne Quelle.", &[]),
            ai("Quelle gibt es nicht.", &[99]),
            ai("Eine gute, eine erfundene Quelle.", &[0, 99]),
        ]);
        let m = metrics(&out, &notes, &segments);
        assert_eq!((m.ai_entries, m.ai_entries_sourced), (4, 1));
        assert_eq!(m.ai_sourced_ratio, 0.25);
    }

    #[test]
    fn lexical_support_needs_a_shared_content_word() {
        let (notes, segments) = base();
        let out = enhance_lexical_case();
        let m = metrics(&out, &notes, &segments);
        assert_eq!(m.ai_entries, 3);
        // "Monteur" teilt ein Inhaltswort mit S0; "dass wird nicht" sind nur
        // Fuellwoerter; der dritte Eintrag hat keine gueltige Quelle.
        assert_eq!(m.ai_entries_lexical, 1);
        assert!((m.lexical_support_ratio - 1.0 / 3.0).abs() < 1e-9);
    }

    fn enhance_lexical_case() -> EnhancedNotes {
        enhanced(vec![
            ai("Kosten je MONTEUR senken.", &[0]),
            ai("Dass wird nicht.", &[0]),
            ai("Angebot bis Freitag.", &[99]),
        ])
    }

    #[test]
    fn no_ai_entries_is_not_a_pass() {
        let (notes, segments) = base();
        let out = enhanced(vec![
            user("B1", "Preis zu hoch?!"),
            user("B3", "Angebot bis Freitag"),
        ]);
        let m = metrics(&out, &notes, &segments);
        assert_eq!(m.ai_entries, 0);
        assert!(
            !targets_met(&m),
            "ohne KI-Eintraege gibt es nichts zu belegen"
        );
    }

    #[test]
    fn the_aggregate_adds_counts_before_dividing() {
        let a = EvalMetrics::from_counts(8, 8, 10, 10, 9);
        let b = EvalMetrics::from_counts(12, 12, 30, 27, 20);
        let sum = aggregate(&[a, b]);
        assert_eq!(
            (sum.user_notes_total, sum.ai_entries, sum.ai_entries_sourced),
            (20, 40, 37)
        );
        assert!((sum.ai_sourced_ratio - 37.0 / 40.0).abs() < 1e-9);
        assert!(!targets_met(&sum), "0,925 < 0,95");
        assert!(targets_met(&aggregate(&[a_full()])));
    }

    fn a_full() -> EvalMetrics {
        EvalMetrics::from_counts(8, 8, 20, 19, 15)
    }

    #[test]
    fn the_model_override_selects_the_local_server() {
        let mut settings = crate::settings::get_default_settings();
        apply_model_override(&mut settings, "llm-qwen3.5-9b-q4");
        let (provider, model, _) = resolve_provider_coded(&settings).unwrap();
        assert!(crate::managers::llm::is_local(&provider));
        assert_eq!(model, "llm-qwen3.5-9b-q4");
        assert_eq!(
            settings.llm_active_model_id.as_deref(),
            Some("local:llm-qwen3.5-9b-q4")
        );
    }

    // -- Fixtures -------------------------------------------------------------

    fn fixture_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes")
    }

    fn fixture(name: &str) -> NotesFixture {
        load_fixtures(&fixture_dir())
            .unwrap()
            .into_iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("Fixture {name} fehlt"))
            .1
    }

    #[test]
    fn the_three_fixtures_have_the_agreed_shape() {
        use crate::managers::meetings::notes::model::NoteBlockKind as K;
        let all = load_fixtures(&fixture_dir()).unwrap();
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["jour_fixe", "kundengespraech", "lenkungskreis_lang"]
        );
        let templates: HashSet<String> =
            crate::managers::meetings::notes::templates::builtin_templates()
                .into_iter()
                .map(|(key, _, _)| key.to_string())
                .collect();
        for (name, fx) in &all {
            assert!(templates.contains(&fx.template_key), "{name}: Vorlage");
            let ids: HashSet<&str> = fx.notes.iter().map(|b| b.id.as_str()).collect();
            assert_eq!(ids.len(), fx.notes.len(), "{name}: Notiz-IDs eindeutig");
            let indices: HashSet<u32> = fx.segments.iter().map(|s| s.segment_index).collect();
            assert_eq!(
                indices.len(),
                fx.segments.len(),
                "{name}: Segmentindizes eindeutig"
            );
            assert!(
                fx.segments
                    .windows(2)
                    .all(|w| w[0].start_ms < w[1].start_ms),
                "{name}: Segmente zeitlich geordnet"
            );
        }
        let minutes = |fx: &NotesFixture| fx.segments.last().unwrap().end_ms as f64 / 60_000.0;

        let kunde = fixture("kundengespraech");
        assert_eq!(kunde.notes.len(), 8);
        assert!((9.0..=11.0).contains(&minutes(&kunde)));
        let channels: HashSet<u8> = kunde.segments.iter().map(|s| s.channel).collect();
        assert_eq!(channels, HashSet::from([0, 1]));

        let jf = fixture("jour_fixe");
        assert_eq!(jf.notes.len(), 10);
        assert!((18.0..=22.0).contains(&minutes(&jf)));
        assert!(jf.notes.iter().any(|b| b.kind == K::Heading));
        assert!(jf.notes.iter().any(|b| b.kind == K::Todo));

        let lk = fixture("lenkungskreis_lang");
        assert_eq!(lk.notes.len(), 12);
        assert_eq!(lk.notes.iter().filter(|b| b.at_ms.is_none()).count(), 2);
        // Erzwingt lokal Map-Reduce, mit Abstand -- bei dem Standard-Kontext.
        // Waehlt der Serverstart je VRAM einen groesseren (P1g), laeuft die
        // Fixture live im Einzeldurchlauf; Map-Reduce decken die Stub-Tests ab
        // (oder `LVA_LLM_CONTEXT_TOKENS=8192` beim Eval-Lauf).
        let budget = single_pass_budget_chars_for(
            "llm-gemma4-e4b-q4",
            crate::managers::llm::DEFAULT_CONTEXT_TOKENS,
            true,
        );
        let payload = render_notes_for_prompt(&lk.notes).chars().count()
            + render_segments_for_prompt(&sorted_segments(&lk.segments))
                .chars()
                .count();
        assert!(
            payload * 100 > budget * 115,
            "{payload} Zeichen vs. Budget {budget}"
        );
    }

    // -- Stub-LLM gegen alle Fixtures -----------------------------------------

    /// Antwort eines Stub-Modells: liest aus dem Prompt die Abschnitte, die
    /// Notiz-IDs und die Segmente und antwortet regelkonform -- jede Notiz
    /// einmal als Verweis, dazu je drittes Segment ein belegter KI-Satz, der
    /// den Segmenttext wiederholt.
    fn stub_reply(request: &str) -> String {
        let body: Value = serde_json::from_str(request).unwrap();
        let prompt = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_str())
            .unwrap()
            .to_string();
        let mut sections: Vec<String> = Vec::new();
        let mut in_sections = false;
        let mut refs: Vec<String> = Vec::new();
        let mut segments: Vec<(String, String)> = Vec::new();
        for line in prompt.lines() {
            if line.starts_with("# ") {
                in_sections = line.starts_with("# Template sections");
                continue;
            }
            if in_sections {
                if let Some(id) = line.strip_prefix("- ").and_then(|l| l.split(" (").next()) {
                    sections.push(id.to_string());
                }
                continue;
            }
            let digits = |rest: &str| rest.chars().take_while(char::is_ascii_digit).count();
            if let Some(rest) = line.strip_prefix('N') {
                let n = digits(rest);
                if n > 0 && rest[n..].starts_with(' ') {
                    refs.push(format!("N{}", &rest[..n]));
                }
            } else if let Some(rest) = line.strip_prefix('S') {
                let n = digits(rest);
                if n > 0 && rest[n..].starts_with(" [") {
                    let text = rest
                        .split_once("]: ")
                        .map(|(_, t)| t)
                        .unwrap_or_else(|| rest.split_once(": ").map(|(_, t)| t).unwrap_or(""));
                    segments.push((format!("S{}", &rest[..n]), text.to_string()));
                }
            }
        }
        assert!(!sections.is_empty(), "Stub: keine Abschnitte im Prompt");
        let mut first: Vec<Value> = refs
            .iter()
            .map(|r| json!({"ref": r, "text": "", "sources": []}))
            .collect();
        for (id, text) in segments.iter().step_by(3) {
            first.push(
                json!({"ref": null, "text": text, "sources": [id], "assignee": null, "due": null}),
            );
        }
        let mut answer = serde_json::Map::new();
        for (i, id) in sections.iter().enumerate() {
            answer.insert(id.clone(), if i == 0 { json!(first) } else { json!([]) });
        }
        crate::managers::meetings::llm_call::test_support::chat_body(
            &Value::Object(answer).to_string(),
        )
    }

    #[tokio::test]
    async fn a_stub_model_over_all_fixtures_meets_the_targets_and_manipulation_is_caught() {
        use crate::managers::meetings::llm_call::test_support::{
            settings_with_mock_provider, spawn_llm_mock_with, MockReply,
        };
        let port = spawn_llm_mock_with(|request| MockReply::Body(stub_reply(request))).await;
        let settings = settings_with_mock_provider(port);
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());

        let fixtures = load_fixtures(&fixture_dir()).unwrap();
        assert_eq!(fixtures.len(), 3);
        let mut all = Vec::new();
        for (name, fx) in &fixtures {
            let outcome = run_fixture(&settings, store.clone(), fx)
                .await
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let m = &outcome.metrics;
            assert_eq!(m.user_preserved_ratio, 1.0, "{name}: {m:?}");
            assert_eq!(m.user_notes_total as usize, fx.notes.len(), "{name}");
            assert!(m.ai_entries > 0, "{name}");
            assert_eq!(m.ai_sourced_ratio, 1.0, "{name}: {m:?}");
            assert_eq!(
                m.lexical_support_ratio, 1.0,
                "{name}: Stub wiederholt die Quelle"
            );
            assert!(targets_met(m), "{name}");

            // Manipulierte Ausgabe: ein Nutzertext veraendert.
            let mut manipulated = outcome.notes.clone();
            let entry = manipulated
                .sections
                .iter_mut()
                .flat_map(|s| s.entries.iter_mut())
                .find(|e| e.origin == Origin::User)
                .unwrap();
            entry.text.push_str(" (umformuliert)");
            let segments = sorted_segments(&fx.segments);
            let broken = metrics(&manipulated, &fx.notes, &segments);
            assert!(broken.user_preserved_ratio < 1.0, "{name}: {broken:?}");
            assert!(!targets_met(&broken), "{name}");
            all.push(outcome.metrics);
        }
        assert!(targets_met(&aggregate(&all)));
        // Die Sandbox ist die einzige Datenbank, die der Lauf angefasst hat.
        assert_eq!(store.db_path(), dir.path().join("meetings.db"));
    }
}
