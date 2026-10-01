//! Tests der Bausteine `agent.note`, `deadline.remind` und `deadline.calendar` (C4): durch die
//! Engine, mit fester Uhr, einem Wegwerf-Vault, einer Graph-Attrappe auf Loopback und (fuer die
//! Ende-zu-Ende-Faelle) der llama-server-Attrappe als Modell. Kein Netz, keine Fenster, keine
//! echten Modellstarts, keine produktiven Daten.
//!
//! Die Untergruppen: `notes` (Notiz, Frontmatter, Dublettenschutz, feindliche Texte), `reminders`
//! (Erinnerung mit fester Uhr), `calendar` (Termin nur nach Freigabe), `end_to_end` (Besprechung ->
//! extrahieren -> Notiz + Erinnerung mit echtem `agent.extract`).

mod calendar;
mod end_to_end;
mod notes;
mod reminders;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::TimeZone;
use serde_json::{json, Value};

use super::*;
use crate::agent::runtime::Target;
use crate::managers::calendar::graph::Endpoints;
use crate::managers::integrations::m365::account::{StoredAccount, Vault};
use crate::managers::integrations::m365::config::M365Config;
use crate::managers::integrations::m365::test_server::{serve, Req, Resp, Seen};
use crate::managers::integrations::m365::{FilesMode, M365Service};
use crate::managers::integrations::model::{Capability, GrantMode, Kind as IKind, NewIntegration};
use crate::managers::integrations::store as integrations_store;
use crate::managers::meetings::store::{
    MeetingDocument, MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta,
};
use crate::managers::workflows::app_actions::{self, GenRequest, ServiceError};
use crate::managers::workflows::engine::{Clock, Engine, EnqueueRequest, RunOutcome};
use crate::managers::workflows::model::{Origin, StepState};
use crate::managers::workflows::store::StepRow;
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine, set_grant, step, FakeClock, Fx, Scripted, T0,
};

pub(crate) const TITLE: &str = "Jour fixe Projekt Nordlicht";
const CLIENT: &str = "11111111-2222-3333-4444-555555555555";

pub(crate) fn block<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

/// Millisekunden UTC zu einer Ortszeit.
pub(crate) fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::Local
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
        .timestamp_millis()
}

// ---------------------------------------------------------------------------
// Dienste der „App“
// ---------------------------------------------------------------------------

pub(crate) struct TestServices {
    store: Arc<MeetingStore>,
    pub(crate) notified: Mutex<Vec<(String, String)>>,
    pub(crate) notify_fails: AtomicBool,
    target: Mutex<Option<Target>>,
    m365: Mutex<Option<Arc<M365Service>>>,
}

fn nope<T>() -> Result<T, ServiceError> {
    Err(ServiceError::NotAvailable("nicht Teil dieses Tests".into()))
}

impl app_actions::AppServices for TestServices {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError> {
        Ok(self.store.clone())
    }
    fn agent_target(&self) -> Result<Target, ServiceError> {
        self.target
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ServiceError::NotAvailable("kein Modell in diesem Test".into()))
    }
    fn m365(&self) -> Option<Arc<M365Service>> {
        self.m365.lock().unwrap().clone()
    }
    fn generate_notes(
        &self,
        _: &GenRequest,
        _: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        nope()
    }
    fn generate_minutes(
        &self,
        _: &GenRequest,
        _: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        nope()
    }
    fn summarize(
        &self,
        _: &str,
        _: &crate::summarizer::SummaryOptions,
        _: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError> {
        nope()
    }
    fn audio_dir(&self, _: Option<&str>) -> Result<PathBuf, ServiceError> {
        nope()
    }
    fn render_speech(
        &self,
        _: &str,
        _: &Path,
        _: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ServiceError> {
        nope()
    }
    fn notify(&self, title: &str, body: &str) -> Result<(), ServiceError> {
        if self.notify_fails.load(Ordering::SeqCst) {
            return Err(ServiceError::Transient(
                "Mitteilungsdienst nicht erreichbar".into(),
            ));
        }
        self.notified
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Graph-Attrappe (nur Termine anlegen)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GraphMode {
    Ok,
    Forbidden,
    Status500,
    Drop,
}

pub(crate) struct Graph {
    pub(crate) mode: Arc<Mutex<GraphMode>>,
    pub(crate) seen: Seen,
}

impl Graph {
    pub(crate) fn created(&self) -> Vec<Req> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST" && r.path() == "/v1.0/me/events")
            .cloned()
            .collect()
    }
}

fn start_graph() -> (String, Graph) {
    let mode = Arc::new(Mutex::new(GraphMode::Ok));
    let counter = Arc::new(AtomicUsize::new(0));
    let (m2, c2) = (mode.clone(), counter.clone());
    let handler = move |req: &Req| -> Resp {
        if req.method == "POST" && req.path().ends_with("/oauth2/v2.0/token") {
            return Resp::json(
                200,
                json!({"access_token": "AT-1", "expires_in": 3600, "token_type": "Bearer"}),
            );
        }
        match (req.method.as_str(), req.path()) {
            ("POST", "/v1.0/me/events") => match *m2.lock().unwrap() {
                GraphMode::Ok => {
                    let n = c2.fetch_add(1, Ordering::SeqCst) + 1;
                    Resp::json(201, json!({"id": format!("EVT-{n}")}))
                }
                GraphMode::Forbidden => Resp::json(
                    403,
                    json!({"error": {"code": "ErrorAccessDenied", "message": "Zugriff verweigert"}}),
                ),
                GraphMode::Status500 => Resp::json(
                    500,
                    json!({"error": {"code": "InternalServerError", "message": "kaputt"}}),
                ),
                GraphMode::Drop => Resp::Drop,
            },
            _ => Resp::empty(404),
        }
    };
    let (base, seen) = block(serve(handler));
    (base, Graph { mode, seen })
}

// ---------------------------------------------------------------------------
// Die Welt
// ---------------------------------------------------------------------------

pub(crate) struct World {
    pub(crate) fx: Fx,
    pub(crate) clock: Arc<FakeClock>,
    pub(crate) engine: Engine,
    pub(crate) svc: Arc<TestServices>,
    pub(crate) meeting_id: String,
    pub(crate) vault: PathBuf,
    pub(crate) extract: Arc<Scripted>,
    pub(crate) graph: Option<Graph>,
    seq: AtomicUsize,
}

pub(crate) fn temp_dir() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().to_path_buf();
    std::mem::forget(dir);
    p
}

pub(crate) fn segments() -> Vec<StoredSegment> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../agent/fixtures/jourfixe-nordlicht.json"
    ))
    .unwrap();
    fixture["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| StoredSegment {
            segment_index: s["i"].as_u64().unwrap() as u32,
            text: s["text"].as_str().unwrap().to_string(),
            start_ms: s["start_ms"].as_u64().unwrap(),
            end_ms: s["start_ms"].as_u64().unwrap() + 5_000,
            channel: 1,
            speaker_index: None,
            words: None,
        })
        .collect()
}

impl World {
    /// Eine Welt mit fertiger Besprechung (1.10.2026, mittags), einem Vault `vault-1` und einem
    /// gescripteten `agent.extract`. Die Uhr steht auf dem 1.10.2026, 14:00 Ortszeit.
    pub(crate) fn new() -> Self {
        let fx = Fx::new();
        let clock = FakeClock::new();
        clock.set(at(2026, 10, 1, 14, 0));
        let engine = engine(&fx, &clock);
        let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
        let meeting = store
            .create_meeting(TITLE, MeetingSource::Import, None)
            .unwrap();
        store
            .append_delta(
                &meeting.id,
                &TranscriptDelta {
                    new_segments: segments(),
                },
            )
            .unwrap();
        store.set_status(&meeting.id, MeetingStatus::Ready).unwrap();
        fx.conn()
            .execute(
                "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
                rusqlite::params![at(2026, 10, 1, 12, 0) / 1000, meeting.id],
            )
            .unwrap();

        // Der Vault: ein Wegwerf-Ordner mit Inbox und Kontextordner, als Integration `vault-1`.
        let vault = temp_dir().join("vault");
        std::fs::create_dir_all(vault.join("00_inbox")).unwrap();
        std::fs::create_dir_all(vault.join("10_contexts").join("beruf")).unwrap();
        {
            let conn = fx.conn();
            let mut n = NewIntegration::new(IKind::Obsidian, "Mein Vault");
            n.id = Some("vault-1".to_string());
            n.config = ObsidianConfig {
                path: vault.to_string_lossy().to_string(),
                subfolder: obsidian::DEFAULT_SUBFOLDER.to_string(),
                context_area: "beruf".to_string(),
                tier: obsidian::DEFAULT_TIER.to_string(),
            }
            .to_json();
            integrations_store::create(&conn, &n, T0).unwrap();
        }

        let svc = Arc::new(TestServices {
            store,
            notified: Mutex::new(Vec::new()),
            notify_fails: AtomicBool::new(false),
            target: Mutex::new(None),
            m365: Mutex::new(None),
        });
        let extract = Scripted::new("agent.extract", EffectKind::Pure);
        engine.register_action(extract.clone());
        install(&engine, svc.clone());
        World {
            fx,
            clock,
            engine,
            svc,
            meeting_id: meeting.id,
            vault,
            extract,
            graph: None,
            seq: AtomicUsize::new(0),
        }
    }

    /// Ein Microsoft-365-Konto `m365-1` (Termine schreiben an) gegen die Graph-Attrappe.
    pub(crate) fn with_graph(mut self) -> Self {
        let (base, graph) = start_graph();
        let vault = Vault::in_dir(temp_dir());
        let cfg = M365Config::new(
            CLIENT,
            "common",
            &[Capability::CalendarWrite],
            FilesMode::Full,
            "Local Voice AI",
        )
        .unwrap();
        let service = M365Service::new(
            Endpoints {
                authority: base.clone(),
                graph: format!("{base}/v1.0"),
                use_env_proxy: false,
            },
            vault.clone(),
        );
        {
            let conn = self.fx.conn();
            let mut n = NewIntegration::new(IKind::M365, "Mein Kalender");
            n.id = Some("m365-1".to_string());
            n.config = cfg.to_json();
            let integ = integrations_store::create(&conn, &n, T0).unwrap();
            vault
                .save(
                    &integ,
                    &StoredAccount::new(
                        cfg.client_id.clone(),
                        cfg.tenant.clone(),
                        cfg.scope_string(),
                        "RT-geheim-0001".to_string(),
                        Some("ich@wolff.de".to_string()),
                        Some("Ich".to_string()),
                    ),
                )
                .unwrap();
        }
        *self.svc.m365.lock().unwrap() = Some(Arc::new(service));
        self.graph = Some(graph);
        self
    }

    pub(crate) fn conn(&self) -> rusqlite::Connection {
        self.fx.conn()
    }

    pub(crate) fn grant(&self, id: &str, cap: Capability, mode: GrantMode) {
        set_grant(&self.conn(), id, cap, mode);
    }

    pub(crate) fn set_now(&self, ms: i64) {
        self.clock.set(ms);
    }

    pub(crate) fn workflow(&self, steps: Vec<Value>) -> String {
        armed_workflow(&self.engine, &def(steps))
    }

    /// Reiht einen Lauf ein; der gescriptete `agent.extract` liefert `extraction`.
    pub(crate) fn start(&self, wf: &str, extraction: Value) -> String {
        self.extract.push_ok(extraction);
        self.enqueue(wf)
    }

    pub(crate) fn enqueue(&self, wf: &str) -> String {
        let n = self.seq.fetch_add(1, Ordering::SeqCst);
        self.engine
            .enqueue(&EnqueueRequest {
                workflow_id: wf.to_string(),
                trigger_key: format!("meeting:{}:{n}", self.meeting_id),
                origin: Origin::Trigger,
                trigger: json!({ "meeting_id": self.meeting_id, "stage": "transcript" }),
                vars: serde_json::Map::new(),
                force_dry_run: false,
            })
            .unwrap()
            .run_id
    }

    pub(crate) fn tick(&self) -> Vec<(String, RunOutcome)> {
        self.engine.tick().unwrap().outcomes
    }

    pub(crate) fn rows(&self, run: &str) -> Vec<StepRow> {
        self.engine.run_detail(run).unwrap().steps
    }

    pub(crate) fn row(&self, run: &str, id: &str) -> StepRow {
        self.rows(run)
            .into_iter()
            .rfind(|s| s.step_id == id)
            .unwrap_or_else(|| panic!("Schritt {id} fehlt"))
    }

    pub(crate) fn state(&self, run: &str, id: &str) -> StepState {
        self.row(run, id).state
    }

    pub(crate) fn output(&self, run: &str, id: &str) -> Value {
        serde_json::from_str(self.row(run, id).output_json.as_deref().unwrap_or("{}")).unwrap()
    }

    pub(crate) fn run_state(&self, run: &str) -> crate::managers::workflows::model::RunState {
        self.engine.run_detail(run).unwrap().run.state
    }

    pub(crate) fn next_run_at(&self, run: &str) -> Option<i64> {
        self.engine.run_detail(run).unwrap().run.next_run_at
    }

    pub(crate) fn toasts(&self) -> Vec<(String, String)> {
        self.svc.notified.lock().unwrap().clone()
    }

    /// Alle Markdown-Dateien im Vault (relativ, mit `/`), sortiert.
    pub(crate) fn notes(&self) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else if p.extension().is_some_and(|x| x == "md") {
                    out.push(
                        p.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.vault, &self.vault, &mut out);
        out.sort();
        out
    }

    pub(crate) fn read_note(&self, rel: &str) -> String {
        std::fs::read_to_string(self.vault.join(rel))
            .unwrap()
            .replace("\r\n", "\n")
    }

    pub(crate) fn pending(&self) -> Vec<crate::managers::integrations::model::Approval> {
        crate::managers::integrations::approvals::list_pending(&self.conn(), self.clock.now_ms())
            .unwrap()
    }

    pub(crate) fn approve(&self, id: &str) {
        crate::managers::integrations::approvals::decide(
            &self.conn(),
            id,
            true,
            self.clock.now_ms(),
        )
        .unwrap();
    }

    pub(crate) fn provenance(
        &self,
        kind: crate::managers::provenance::SubjectKind,
        subject: &str,
    ) -> Vec<crate::managers::provenance::ProvenanceEntry> {
        crate::managers::provenance::list(&self.conn(), kind, subject).unwrap()
    }
}

// ---------------------------------------------------------------------------
// Ergebnis von agent.extract (so, wie `Extraction::to_json` es liefert)
// ---------------------------------------------------------------------------

pub(crate) fn deadline(text: &str, due: &str, source: &str, segs: &[u32], quote: &str) -> Value {
    json!({
        "text": text, "due": due, "due_phrase": "bis übermorgen", "due_source": source,
        "segments": segs, "quote": quote, "confidence": 0.9
    })
}

pub(crate) fn todo(text: &str, assignee: &str, due: Option<&str>, source: Option<&str>) -> Value {
    json!({
        "text": text, "assignee": assignee, "due": due, "due_phrase": due.map(|_| "bis Freitag"),
        "due_source": source, "segments": [2, 3], "quote": "Ich schicke es bis übermorgen an den Kunden.",
        "confidence": 0.8
    })
}

pub(crate) fn decision(text: &str, segs: &[u32], quote: &str) -> Value {
    json!({"text": text, "segments": segs, "quote": quote, "confidence": 0.95})
}

pub(crate) fn extraction(
    meeting_id: &str,
    todos: Vec<Value>,
    deadlines: Vec<Value>,
    decisions: Vec<Value>,
) -> Value {
    json!({
        "outcome": "extracted", "reason": null, "reason_text": null,
        "meeting_id": meeting_id, "meeting_date": "2026-10-01",
        "todos": todos, "deadlines": deadlines, "decisions": decisions,
        "counts": {
            "todos": todos.len(), "deadlines": deadlines.len(), "decisions": decisions.len(),
            "items": todos.len() + deadlines.len() + decisions.len(), "dropped": 0, "notes": 0
        },
        "dropped": [], "notes": [],
        "provenance": {
            "model": "llm-test", "locality": "local", "prompt_tokens": 100,
            "completion_tokens": 20, "duration_ms": 1500, "requests": 1,
            "chunks": 1, "chunks_total": 1, "confidence": 0.93, "segments": [2, 3, 5, 6]
        }
    })
}

/// Eine Besprechung mit einer Frist am 3.10. (in zwei Tagen), einer Entscheidung und einem To-do.
pub(crate) fn standard_extraction(meeting_id: &str) -> Value {
    extraction(
        meeting_id,
        vec![todo(
            "Frau Berg schickt das Angebot.",
            "Frau Berg",
            None,
            None,
        )],
        vec![deadline(
            "Das Angebot geht an die Stadtwerke.",
            "2026-10-03",
            "angabe:uebermorgen",
            &[2],
            "Ich schicke es bis übermorgen an den Kunden.",
        )],
        vec![decision(
            "Die Abnahme findet am 15. Oktober statt.",
            &[5, 6],
            "der 15. Oktober steht, das ist beschlossen",
        )],
    )
}

// ---------------------------------------------------------------------------
// Frontmatter pruefen (kein YAML-Parser im Baum: der Vertrag zeilenweise)
// ---------------------------------------------------------------------------

/// Zerlegt eine Notiz in (Kopf-Felder, Rest). Fehlt der Kopf oder ist er nicht geschlossen,
/// schlaegt der Test fehl.
pub(crate) fn front(note: &str) -> (HashMap<String, String>, String) {
    let rest = note
        .strip_prefix("---\n")
        .expect("Notiz beginnt nicht mit ---");
    let end = rest.find("\n---\n").expect("Kopf nicht geschlossen");
    let (head, body) = rest.split_at(end);
    let mut map = HashMap::new();
    for line in head.lines() {
        if line.starts_with("  - ") || line.trim().is_empty() {
            continue;
        }
        let (k, v) = line
            .split_once(": ")
            .unwrap_or((line.trim_end_matches(':'), ""));
        map.insert(k.to_string(), v.to_string());
    }
    (map, body[5..].to_string())
}

/// Ein YAML-Text in doppelten Anfuehrungszeichen: gleiche Anzahl unmaskierter Zeichen am Rand,
/// innen kein unmaskiertes `"`.
pub(crate) fn is_quoted_yaml(v: &str) -> bool {
    if v.len() < 2 || !v.starts_with('"') || !v.ends_with('"') {
        return false;
    }
    let inner = &v[1..v.len() - 1];
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return false;
        }
    }
    !escaped
}

/// Der Vertrag des Vaults (`obsidian`-Moduldoku, `90_templates/note.md`).
pub(crate) fn assert_ai_os_frontmatter(f: &HashMap<String, String>) {
    for key in [
        "title",
        "sensitivity",
        "updated",
        "lva_id",
        "lva_meeting_id",
    ] {
        assert!(
            is_quoted_yaml(f.get(key).unwrap_or_else(|| panic!("{key} fehlt"))),
            "{key}: {:?}",
            f[key]
        );
    }
    let tags = &f["tags"];
    assert!(tags.starts_with('[') && tags.ends_with(']'), "tags: {tags}");
    assert!(
        obsidian::CONTEXT_AREAS.contains(&f["context_area"].as_str()),
        "{:?}",
        f["context_area"]
    );
    assert!(
        ["public", "internal", "confidential", "restricted"].contains(&f["data_class"].as_str()),
        "{:?}",
        f["data_class"]
    );
    assert!(
        obsidian::TIERS.contains(&f["tier"].as_str()),
        "{:?}",
        f["tier"]
    );
}

pub(crate) fn mark_count(note: &str, mark: &str) -> usize {
    note.matches(mark).count()
}
