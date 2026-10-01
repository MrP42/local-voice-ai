//! Tests der Werkzeuge (A8): Eingaben, Wirkung gegen eine mitschreibende Attrappe der App und
//! gegen den echten Store, Zustimmung zur Aufnahme und Audit ueber die echte Bruecke.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::bridge::{ApprovalNotifier, Bridge, BridgeError, CallStep, ClientCtx};
use crate::agent_bridge::catalog::{CallContext, ToolRegistry, CATALOG};
use crate::agent_bridge::clients;
use crate::agent_bridge::protocol::code;
use crate::agent_bridge::testkit::{fast_config, NOW};
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{Caller, GrantMode};
use crate::managers::integrations::store as register;
use crate::managers::integrations::test_support::Fx;
use crate::managers::workflows::recording::{CurrentRecording, StartedRecording};
use crate::managers::youtube::oembed::FetchOpts;
use crate::managers::youtube::test_support::{delayed, json_ok, serve};

// ---------------------------------------------------------------------------
// Attrappen
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeRecording {
    running: Mutex<Option<String>>,
    starts: AtomicUsize,
    stops: AtomicUsize,
    start_error: Mutex<Option<String>>,
}

impl RecordingControl for FakeRecording {
    fn current(&self) -> Option<CurrentRecording> {
        self.running
            .lock()
            .unwrap()
            .clone()
            .map(|id| CurrentRecording {
                meeting_id: Some(id),
                started_at_ms: Some(1),
            })
    }

    fn start(&self, req: &StartRequest) -> Result<StartedRecording, String> {
        if let Some(e) = self.start_error.lock().unwrap().clone() {
            return Err(e);
        }
        let mut running = self.running.lock().unwrap();
        if running.is_some() {
            return Err("already_recording".to_string());
        }
        let n = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("REC{n}");
        *running = Some(id.clone());
        Ok(StartedRecording {
            meeting_id: id,
            title: req.title.clone(),
        })
    }

    fn stop(&self) -> Result<String, String> {
        match self.running.lock().unwrap().take() {
            Some(id) => {
                self.stops.fetch_add(1, Ordering::SeqCst);
                Ok(id)
            }
            None => Err("not_recording".to_string()),
        }
    }
}

/// Haelt einen Render an, bis der Test ihn freigibt.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
    entered: AtomicBool,
}

impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn wait_inside(&self) {
        self.entered.store(true, Ordering::SeqCst);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.cv.wait(open).unwrap();
        }
    }
}

struct FakeHost {
    recording: Option<Arc<FakeRecording>>,
    enqueued: Mutex<Vec<(String, String)>>,
    pages: Mutex<Vec<(String, String, String)>>,
    renders: Mutex<Vec<(String, String)>>,
    created: Mutex<Vec<String>>,
    stops: Mutex<Vec<(String, i64)>>,
    fail_enqueue: AtomicBool,
    fail_render: Mutex<Option<String>>,
    oembed: Mutex<String>,
    gate: Option<Arc<Gate>>,
}

impl FakeHost {
    fn new() -> Arc<Self> {
        Self::build(Some(Arc::new(FakeRecording::default())), None)
    }

    fn build(recording: Option<Arc<FakeRecording>>, gate: Option<Arc<Gate>>) -> Arc<Self> {
        Arc::new(Self {
            recording,
            enqueued: Mutex::default(),
            pages: Mutex::default(),
            renders: Mutex::default(),
            created: Mutex::default(),
            stops: Mutex::default(),
            fail_enqueue: AtomicBool::new(false),
            fail_render: Mutex::new(None),
            oembed: Mutex::new("http://127.0.0.1:9/oembed".to_string()),
            gate,
        })
    }

    fn rec(&self) -> Arc<FakeRecording> {
        self.recording.clone().expect("Recorder")
    }
}

impl Host for FakeHost {
    fn enqueue_import(&self, title: &str, source_path: &str) -> Result<QueuedImport, String> {
        if self.fail_enqueue.load(Ordering::SeqCst) {
            return Err("Datenbank gesperrt".to_string());
        }
        self.enqueued
            .lock()
            .unwrap()
            .push((title.to_string(), source_path.to_string()));
        Ok(QueuedImport {
            meeting_id: "M1".to_string(),
            title: title.to_string(),
            status: "queued".to_string(),
        })
    }

    fn create_page(&self, title: &str, text: &str) -> Result<PageRef, String> {
        let mut pages = self.pages.lock().unwrap();
        let id = format!("page_{}", pages.len() + 1);
        pages.push((id.clone(), title.to_string(), text.to_string()));
        Ok(PageRef {
            id,
            title: title.to_string(),
        })
    }

    fn page_text(&self, page_id: &str) -> Result<String, String> {
        self.pages
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _, _)| id == page_id)
            .map(|(_, _, t)| t.clone())
            .ok_or_else(|| "unbekannt".to_string())
    }

    fn render_audio(
        &self,
        page_id: &str,
        _text: &str,
        file_name: &str,
    ) -> Result<Rendered, String> {
        if let Some(gate) = &self.gate {
            gate.wait_inside();
        }
        if let Some(e) = self.fail_render.lock().unwrap().clone() {
            return Err(e);
        }
        self.renders
            .lock()
            .unwrap()
            .push((page_id.to_string(), file_name.to_string()));
        Ok(Rendered {
            file_name: format!("{file_name}.wav"),
            path: format!("C:/Seiten/{page_id}/{file_name}.wav"),
            bytes: 32_044,
            format: "wav".to_string(),
        })
    }

    fn recording(&self) -> Option<Arc<dyn RecordingControl>> {
        self.recording
            .clone()
            .map(|r| r as Arc<dyn RecordingControl>)
    }

    fn schedule_stop(&self, meeting_id: &str, at_ms: i64) {
        self.stops
            .lock()
            .unwrap()
            .push((meeting_id.to_string(), at_ms));
    }

    fn meeting_created(&self, meeting_id: &str) {
        self.created.lock().unwrap().push(meeting_id.to_string());
    }

    fn youtube_options(&self) -> AddOptions {
        AddOptions {
            oembed_base: self.oembed.lock().unwrap().clone(),
            fetch: FetchOpts {
                timeout: Duration::from_secs(5),
                max_bytes: 64 * 1024,
                use_env_proxy: false,
            },
        }
    }
}

fn ctx(approved: bool) -> CallContext {
    CallContext {
        client_id: "C1".to_string(),
        client_label: "Claude Code".to_string(),
        approved,
        now_ms: NOW,
    }
}

struct Rig {
    fx: Fx,
    store: Arc<MeetingStore>,
    host: Arc<FakeHost>,
    tools: Arc<AppTools>,
}

fn rig_with(host: Arc<FakeHost>) -> Rig {
    let fx = Fx::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let tools = Arc::new(AppTools::new(store.clone(), host.clone()));
    Rig {
        fx,
        store,
        host,
        tools,
    }
}

fn rig() -> Rig {
    rig_with(FakeHost::new())
}

impl Rig {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx(false), tool, &args)
    }

    fn call_approved(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx(true), tool, &args)
    }
}

fn err_text(r: Result<Value, String>) -> String {
    r.expect_err("Fehler erwartet")
}

// ---------------------------------------------------------------------------
// Katalog und Argumente
// ---------------------------------------------------------------------------

#[test]
fn the_handler_offers_exactly_the_catalog_tools_and_registers_cleanly() {
    let r = rig();
    let mut offered: Vec<String> = r.tools.specs().into_iter().map(|s| s.name).collect();
    offered.sort();
    let mut catalog: Vec<String> = CATALOG.iter().map(|e| e.name.to_string()).collect();
    catalog.sort();
    assert_eq!(offered, catalog);
    let mut registry = ToolRegistry::new();
    registry.register(r.tools.clone()).unwrap();
    for spec in r.tools.specs() {
        assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
        assert_eq!(
            spec.input_schema["additionalProperties"],
            json!(false),
            "{}",
            spec.name
        );
        assert!(spec.description.chars().count() > 20, "{}", spec.name);
    }
}

#[test]
fn arguments_are_checked_strictly() {
    let r = rig();
    let cases = [
        ("create_session", json!({})),
        ("create_session", json!({"name": 5})),
        ("create_session", json!({"name": "x", "extra": 1})),
        ("create_session", json!("text")),
        ("create_meeting", json!({"title": ["a"]})),
        ("create_meeting", json!({"session_id": "../x"})),
        ("create_meeting", json!({"session_id": "a b"})),
        ("transcribe_file", json!({})),
        ("transcribe_file", json!({"path": 7})),
        (
            "transcribe_file",
            json!({"path": "C:\\a.wav", "force": true}),
        ),
        ("tts_render_audio", json!({})),
        ("tts_render_audio", json!({"page_id": "../x"})),
        ("tts_page_create", json!({"title": "t"})),
        ("add_youtube_source", json!({})),
        ("stop_recording", json!({"x": 1})),
    ];
    for (tool, args) in cases {
        assert!(r.call(tool, args.clone()).is_err(), "{tool} {args}");
    }
    // Eine Zeichenfolge als Argumente und unbekannte Werkzeuge gibt es nicht.
    assert!(r.call("gibt_es_nicht", json!({})).is_err());
    // Fehlende Argumente (null) sind ein leeres Objekt.
    assert!(r.call("create_meeting", Value::Null).is_ok());
}

#[test]
fn text_limits_hold() {
    let r = rig();
    let long_title = "T".repeat(MAX_TITLE_CHARS + 1);
    assert!(err_text(r.call("create_meeting", json!({"title": long_title}))).contains("zu lang"));
    assert!(err_text(r.call(
        "create_session",
        json!({"name": "N".repeat(MAX_TITLE_CHARS + 1)})
    ))
    .contains("zu lang"));
    let text = "x".repeat(MAX_TTS_TEXT_CHARS + 1);
    assert!(
        err_text(r.call("tts_page_create", json!({"title": "t", "text": text})))
            .contains("zu lang")
    );
    let url = format!("https://youtu.be/{}", "a".repeat(MAX_URL_CHARS));
    assert!(err_text(r.call("add_youtube_source", json!({"url": url}))).contains("zu lang"));
    assert!(r.host.pages.lock().unwrap().is_empty(), "nichts angelegt");
    assert!(r.host.created.lock().unwrap().is_empty());
}

#[test]
fn control_and_switching_characters_never_reach_titles() {
    let r = rig();
    let out = r
        .call(
            "create_meeting",
            json!({"title": "Jour\u{202E} fixe\n\u{0}\u{200B}  Vertrieb"}),
        )
        .unwrap();
    assert_eq!(out["title"], "Jour fixe Vertrieb");
    assert_eq!(clean_line("  a \t b\r\nc\u{FEFF}"), "a b c");
    assert_eq!(clean_text("a\r\nb\u{0}\u{202E}c\td"), "a\nbc\td");
}

// ---------------------------------------------------------------------------
// Datei transkribieren
// ---------------------------------------------------------------------------

#[test]
fn transcribe_file_refuses_bad_paths_before_the_queue_sees_them() {
    let r = rig();
    for path in [
        r"\\server\freigabe\a.wav",
        r"\\.\pipe\x.wav",
        r"\\?\C:\a.wav",
        "a.wav",
        r"C:\Aufnahmen\..\a.wav",
        r"C:\a.wav:strom",
        r"C:\a\NUL.wav",
        r"C:\a\programm.exe",
        r"C:\gibt\es\nicht.wav",
        "",
    ] {
        let e = err_text(r.call("transcribe_file", json!({"path": path})));
        assert!(!e.is_empty(), "{path}");
    }
    assert!(r.host.enqueued.lock().unwrap().is_empty());
    assert!(r.host.created.lock().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn transcribe_file_queues_a_local_media_file_and_returns_the_meeting_id() {
    let r = rig();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Jour fixe.m4a");
    std::fs::write(&file, b"audio").unwrap();
    let out = r
        .call("transcribe_file", json!({"path": file.to_string_lossy()}))
        .unwrap();
    assert_eq!(out["meeting_id"], "M1");
    assert_eq!(out["status"], "queued");
    let queued = r.host.enqueued.lock().unwrap().clone();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0, "Jour fixe", "Titel aus dem Dateinamen");
    assert!(
        !queued[0].1.starts_with(r"\\"),
        "kein Namensraumpraefix: {}",
        queued[0].1
    );
    assert!(queued[0].1.ends_with("Jour fixe.m4a"));
    assert_eq!(r.host.created.lock().unwrap().as_slice(), ["M1"]);
    // Ein eigener Titel gilt, bereinigt.
    let out = r
        .call(
            "transcribe_file",
            json!({"path": file.to_string_lossy(), "title": "  Kick-off\n2026 "}),
        )
        .unwrap();
    assert_eq!(out["title"], "Kick-off 2026");
}

#[cfg(windows)]
#[test]
fn a_failed_enqueue_is_a_tool_error_and_announces_no_meeting() {
    let r = rig();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.wav");
    std::fs::write(&file, b"RIFF").unwrap();
    r.host.fail_enqueue.store(true, Ordering::SeqCst);
    let e = err_text(r.call("transcribe_file", json!({"path": file.to_string_lossy()})));
    assert!(e.contains("nicht einreihen"), "{e}");
    assert!(r.host.created.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Sessions und Besprechungen (echter Store)
// ---------------------------------------------------------------------------

#[test]
fn a_session_and_an_empty_meeting_in_it_are_created() {
    let r = rig();
    let s = r
        .call("create_session", json!({"name": "  Podcast  "}))
        .unwrap();
    let session = s["session_id"].as_str().unwrap().to_string();
    assert_eq!(s["name"], "Podcast");
    let m = r
        .call(
            "create_meeting",
            json!({"title": "Folge 1", "session_id": session}),
        )
        .unwrap();
    let id = m["meeting_id"].as_str().unwrap();
    assert_eq!(r.store.meeting_folder_ids(id).unwrap(), vec![session]);
    let stored = r.store.get_meeting(id).unwrap().unwrap();
    assert_eq!(stored.source, "empty", "ein leerer Eintrag, keine Aufnahme");
    assert!(stored.mic_audio_path.is_none() && stored.source_path.is_none());
    assert_eq!(r.host.created.lock().unwrap().as_slice(), [id]);
    // Standardtitel; eine unbekannte Session gibt es nicht.
    let d = r.call("create_meeting", json!({})).unwrap();
    assert_eq!(d["title"], "Neue Besprechung");
    assert_eq!(
        err_text(r.call("create_meeting", json!({"session_id": "S-GIBT-ES-NICHT"}))),
        "Die Session gibt es nicht."
    );
}

// ---------------------------------------------------------------------------
// YouTube
// ---------------------------------------------------------------------------

const OEMBED: &str = r#"{
  "title": "Lastgang verstehen",
  "author_name": "Wolff Applied AI",
  "author_url": "https://www.youtube.com/@wolffappliedai",
  "type": "video",
  "thumbnail_url": "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"
}"#;
const LINK: &str = "https://youtu.be/dQw4w9WgXcQ?si=TRACKING";

#[test]
fn add_youtube_source_creates_a_youtube_meeting_attributed_to_the_agent() {
    let r = rig();
    let (base, seen) = tauri::async_runtime::block_on(serve(vec![json_ok(OEMBED)]));
    *r.host.oembed.lock().unwrap() = base;
    let out = r.call("add_youtube_source", json!({"url": LINK})).unwrap();
    let id = out["meeting_id"].as_str().unwrap();
    assert_eq!(out["source"], "youtube");
    assert_eq!(out["title"], "Lastgang verstehen");
    assert_eq!(out["channel"], "Wolff Applied AI");
    assert_eq!(
        out["url"], "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "bereinigt, ohne Verfolgung"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "genau ein Abruf (oEmbed), nie ein Download"
    );
    let stored = r.store.get_meeting(id).unwrap().unwrap();
    assert_eq!(stored.source, "youtube");
    assert!(stored.source_path.is_none() && stored.mic_audio_path.is_none());
    assert_eq!(r.host.created.lock().unwrap().as_slice(), [id]);

    let conn = r.fx.conn();
    let prov = provenance::list(&conn, SubjectKind::Transcript, id).unwrap();
    assert_eq!(prov.len(), 1);
    assert_eq!(prov[0].actor_kind, Some(ActorKind::AgentExternal));
    assert_eq!(prov[0].actor_ref.as_deref(), Some("C1"));
    let rows = audit::list(&conn, &AuditFilter::default(), 50).unwrap();
    let fetch = rows
        .iter()
        .find(|a| a.capability.as_deref() == Some("media.fetch"))
        .expect("Audit des Abrufs");
    assert_eq!(
        fetch.caller, "agent_external",
        "der Abruf steht nicht unter dem Nutzer"
    );
    assert_eq!(fetch.outcome, "ok");
    assert!(fetch.target.as_deref().unwrap().starts_with("youtube:"));
}

#[test]
fn add_youtube_source_refuses_bad_links_without_network() {
    let r = rig();
    let (base, seen) = tauri::async_runtime::block_on(serve(vec![json_ok(OEMBED)]));
    *r.host.oembed.lock().unwrap() = base;
    for url in [
        "https://example.com/watch?v=dQw4w9WgXcQ",
        "javascript:alert(1)",
        "file:///C:/a.wav",
        "https://www.youtube.com/playlist?list=PLxyz",
        "kein link",
    ] {
        assert!(
            r.call("add_youtube_source", json!({"url": url})).is_err(),
            "{url}"
        );
    }
    assert!(seen.lock().unwrap().is_empty(), "kein Byte ins Netz");
    assert!(r.host.created.lock().unwrap().is_empty());
}

#[test]
fn a_video_is_added_once_at_a_time() {
    let r = rig();
    let (base, _) = tauri::async_runtime::block_on(serve(vec![delayed(600, json_ok(OEMBED))]));
    *r.host.oembed.lock().unwrap() = base;
    let tools = r.tools.clone();
    let first = std::thread::spawn(move || {
        tools.call(&ctx(false), "add_youtube_source", &json!({"url": LINK}))
    });
    std::thread::sleep(Duration::from_millis(200));
    let second = r.call("add_youtube_source", json!({"url": LINK}));
    assert!(
        err_text(second).contains("gerade"),
        "zweiter Aufruf: Besetzt"
    );
    assert!(first.join().unwrap().is_ok());
    assert_eq!(r.host.created.lock().unwrap().len(), 1, "eine Besprechung");
}

// ---------------------------------------------------------------------------
// Vorlesen
// ---------------------------------------------------------------------------

#[test]
fn a_page_is_created_and_rendered_and_the_provenance_names_the_agent() {
    let r = rig();
    let p = r
        .call(
            "tts_page_create",
            json!({"title": "Begrüßung", "text": "Hallo Welt.\nZweite Zeile."}),
        )
        .unwrap();
    let page = p["page_id"].as_str().unwrap().to_string();
    assert_eq!(p["chars"], 25);
    let out = r
        .call(
            "tts_render_audio",
            json!({"page_id": page, "file_name": "begruessung"}),
        )
        .unwrap();
    assert_eq!(out["file"], "begruessung.wav");
    assert_eq!(out["format"], "wav");
    assert_eq!(out["page_id"], page.as_str());
    assert_eq!(
        r.host.renders.lock().unwrap().as_slice(),
        [(page.clone(), "begruessung".to_string())]
    );
    let conn = r.fx.conn();
    let prov = provenance::list(
        &conn,
        SubjectKind::TtsAudio,
        &format!("{page}/begruessung.wav"),
    )
    .unwrap();
    assert_eq!(prov.len(), 1);
    assert_eq!(prov[0].actor_kind, Some(ActorKind::AgentExternal));
    assert_eq!(prov[0].actor_ref.as_deref(), Some("C1"));
    assert_eq!(prov[0].operation, "tts_render");
    // Ohne Dateinamen gibt es einen Standardnamen.
    let named = r
        .call("tts_render_audio", json!({"page_id": page}))
        .unwrap();
    assert!(named["file"].as_str().unwrap().starts_with("agent-"));
}

#[test]
fn a_render_with_a_bad_file_name_an_unknown_page_or_no_text_is_refused() {
    let r = rig();
    let page = r
        .call("tts_page_create", json!({"title": "t", "text": "Text"}))
        .unwrap()["page_id"]
        .as_str()
        .unwrap()
        .to_string();
    for name in [
        "../x",
        "a/b",
        "a\\b",
        "C:x",
        "con",
        "NUL",
        "com3",
        ".versteckt",
        "x.",
        "a<b",
        "..",
        &"n".repeat(101),
    ] {
        let e = err_text(r.call(
            "tts_render_audio",
            json!({"page_id": page, "file_name": name}),
        ));
        assert!(!e.is_empty(), "{name}");
    }
    assert!(r.host.renders.lock().unwrap().is_empty());
    assert_eq!(
        err_text(r.call("tts_render_audio", json!({"page_id": "page_999"}))),
        "Die Seite gibt es nicht."
    );
    // Eine Seite ohne Text (z. B. von Hand geleert).
    r.host.pages.lock().unwrap()[0].2 = "  \n ".to_string();
    assert!(err_text(r.call("tts_render_audio", json!({"page_id": page}))).contains("keinen Text"));
    // Ein zu langer Text, der an der Aufnahmegrenze vorbei in die Seite kam.
    r.host.pages.lock().unwrap()[0].2 = "x".repeat(MAX_TTS_TEXT_CHARS + 1);
    assert!(err_text(r.call("tts_render_audio", json!({"page_id": page}))).contains("zu lang"));
    assert!(r.host.renders.lock().unwrap().is_empty());
}

#[test]
fn a_second_render_is_refused_while_one_runs() {
    let gate = Arc::new(Gate::default());
    let r = rig_with(FakeHost::build(
        Some(Arc::new(FakeRecording::default())),
        Some(gate.clone()),
    ));
    let page = r
        .call("tts_page_create", json!({"title": "t", "text": "Text"}))
        .unwrap()["page_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (tools, p2) = (r.tools.clone(), page.clone());
    let first = std::thread::spawn(move || {
        tools.call(
            &ctx(false),
            "tts_render_audio",
            &json!({"page_id": p2, "file_name": "eins"}),
        )
    });
    while !gate.entered.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let second = r.call(
        "tts_render_audio",
        json!({"page_id": page, "file_name": "zwei"}),
    );
    assert!(err_text(second).contains("schon eine Audio-Erzeugung"));
    gate.release();
    assert!(first.join().unwrap().is_ok());
    // Danach ist die Sperre wieder frei.
    assert!(r
        .call(
            "tts_render_audio",
            json!({"page_id": page, "file_name": "drei"})
        )
        .is_ok());
}

#[test]
fn a_failing_render_reports_the_error_and_frees_the_lock() {
    let r = rig();
    let page = r
        .call("tts_page_create", json!({"title": "t", "text": "Text"}))
        .unwrap()["page_id"]
        .as_str()
        .unwrap()
        .to_string();
    *r.host.fail_render.lock().unwrap() = Some("Engine nicht installiert".to_string());
    let e = err_text(r.call("tts_render_audio", json!({"page_id": page})));
    assert!(e.contains("Engine nicht installiert"), "{e}");
    *r.host.fail_render.lock().unwrap() = None;
    assert!(
        r.call("tts_render_audio", json!({"page_id": page})).is_ok(),
        "Sperre frei nach Fehler"
    );
    // Keine Herkunft fuer eine Datei, die es nicht gibt.
    let conn = r.fx.conn();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM provenance WHERE subject_kind = 'tts_audio'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn the_sandbox_host_renders_a_valid_wav_into_the_page_folder_and_never_overwrites() {
    let fx = Fx::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(sandbox::SandboxHost::new(
        store.clone(),
        dir.path().to_path_buf(),
    ));
    let tools = AppTools::new(store, host);
    let page = tools
        .call(
            &ctx(false),
            "tts_page_create",
            &json!({"title": "t", "text": "Hallo"}),
        )
        .unwrap()["page_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = tools
        .call(
            &ctx(false),
            "tts_render_audio",
            &json!({"page_id": page, "file_name": "ton"}),
        )
        .unwrap();
    let b = tools
        .call(
            &ctx(false),
            "tts_render_audio",
            &json!({"page_id": page, "file_name": "ton"}),
        )
        .unwrap();
    assert_eq!(a["file"], "ton.wav");
    assert_eq!(b["file"], "ton (2).wav", "eine vorhandene Datei bleibt");
    let path = std::path::PathBuf::from(a["path"].as_str().unwrap());
    assert!(
        path.starts_with(dir.path()),
        "nur im Seitenordner der Sandbox"
    );
    let reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(reader.spec().sample_rate, 16_000);
    assert_eq!(reader.duration(), 16_000);
    assert_eq!(a["bytes"], std::fs::metadata(&path).unwrap().len());
}

#[test]
fn a_failed_render_leaves_no_half_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("fehlt").join("x.wav");
    assert!(sandbox::write_test_tone(&target).is_err());
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "nichts zurueckgeblieben"
    );
    // Ein Erfolg hinterlaesst genau die Zieldatei, keine .part.
    let ok = dir.path().join("x.wav");
    sandbox::write_test_tone(&ok).unwrap();
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["x.wav".to_string()]);
}

// ---------------------------------------------------------------------------
// Aufnahme
// ---------------------------------------------------------------------------

#[test]
fn start_recording_without_the_users_approval_never_starts() {
    let r = rig();
    let e = err_text(r.call("start_recording", json!({"title": "Heimlich"})));
    assert!(e.contains("Ohne Ihre Bestätigung"), "{e}");
    assert_eq!(r.host.rec().starts.load(Ordering::SeqCst), 0);
    assert!(r.host.rec().current().is_none());
    assert!(r.host.stops.lock().unwrap().is_empty());
}

#[test]
fn an_approved_start_starts_once_and_a_second_is_refused() {
    let r = rig();
    let out = r
        .call_approved("start_recording", json!({"title": "Jour fixe"}))
        .unwrap();
    assert_eq!(out["meeting_id"], "REC1");
    assert_eq!(out["title"], "Jour fixe");
    assert_eq!(out["status"], "recording");
    assert_eq!(r.host.rec().starts.load(Ordering::SeqCst), 1);
    let e = err_text(r.call_approved("start_recording", json!({})));
    assert!(e.contains("schon eine Aufnahme"), "{e}");
    assert_eq!(r.host.rec().starts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_started_recording_gets_a_stop_time() {
    let r = rig();
    let out = r.call_approved("start_recording", json!({})).unwrap();
    assert_eq!(out["title"], "Aufnahme (Agent)");
    assert_eq!(out["auto_stop_after_minutes"], 480, "Vorgabe: 8 Stunden");
    let stops = r.host.stops.lock().unwrap().clone();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0].0, "REC1");
    r.host.rec().stop().unwrap();
    let out = r
        .call_approved("start_recording", json!({"max_minutes": 30}))
        .unwrap();
    assert_eq!(out["auto_stop_after_minutes"], 30);
    assert!(r
        .call_approved("start_recording", json!({"max_minutes": 721}))
        .is_err());
}

#[test]
fn two_recording_starts_start_one() {
    let r = rig();
    let barrier = Arc::new(Barrier::new(2));
    let results: Vec<Result<Value, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (tools, b) = (r.tools.clone(), barrier.clone());
                s.spawn(move || {
                    b.wait();
                    tools.call(&ctx(true), "start_recording", &json!({}))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        results.iter().filter(|x| x.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(r.host.rec().starts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_recorder_that_cannot_start_is_reported_and_nothing_runs() {
    let r = rig();
    *r.host.rec().start_error.lock().unwrap() = Some("no_input_device".to_string());
    let e = err_text(r.call_approved("start_recording", json!({})));
    assert!(e.contains("Mikrofon"), "{e}");
    assert!(r.host.rec().current().is_none());
    assert!(
        r.host.stops.lock().unwrap().is_empty(),
        "kein Ende fuer eine Aufnahme, die es nicht gibt"
    );
    *r.host.rec().start_error.lock().unwrap() = Some("loopback_start_failed".to_string());
    assert!(err_text(r.call_approved("start_recording", json!({}))).contains("Systemton"));
}

#[test]
fn an_environment_without_a_recorder_never_records() {
    let r = rig_with(FakeHost::build(None, None));
    assert!(err_text(r.call_approved("start_recording", json!({}))).contains("nicht verfügbar"));
    assert!(err_text(r.call("stop_recording", json!({}))).contains("nicht verfügbar"));
}

#[test]
fn stop_recording_stops_the_running_recording_and_is_harmless_otherwise() {
    let r = rig();
    let none = r.call("stop_recording", json!({})).unwrap();
    assert_eq!(none["stopped"], false);
    r.call_approved("start_recording", json!({})).unwrap();
    let out = r.call("stop_recording", json!({})).unwrap();
    assert_eq!(out["stopped"], true);
    assert_eq!(out["meeting_id"], "REC1");
    assert_eq!(r.host.rec().stops.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.call("stop_recording", json!({})).unwrap()["stopped"],
        false
    );
}

// ---------------------------------------------------------------------------
// Ueber die echte Bruecke: Rechte, Freigabe, Audit
// ---------------------------------------------------------------------------

struct World {
    fx: Fx,
    bridge: Bridge,
    host: Arc<FakeHost>,
    client: ClientCtx,
    notified: Arc<Mutex<Vec<String>>>,
}

fn world(host: Arc<FakeHost>) -> World {
    let fx = Fx::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(AppTools::new(store.clone(), host.clone())))
        .unwrap();
    let notified: Arc<Mutex<Vec<String>>> = Arc::default();
    let n2 = notified.clone();
    let notifier: ApprovalNotifier =
        Arc::new(move |tool| n2.lock().unwrap().push(tool.to_string()));
    let bridge = Bridge::new(store, registry, fast_config())
        .with_clock(Arc::new(|| NOW))
        .with_notifier(notifier);
    let (client, _token) = clients::create(&fx.conn(), "Claude Code", None, NOW).unwrap();
    World {
        fx,
        bridge,
        host,
        client: ClientCtx::from(&client),
        notified,
    }
}

impl World {
    fn grant(&self, tool: &str, mode: GrantMode) {
        let conn = self.fx.conn();
        let entry = catalog::find(tool).unwrap();
        let ceiling = if entry.capability.never_allow() {
            GrantMode::Ask
        } else {
            GrantMode::Allow
        };
        register::set_grant(
            &conn,
            "agents",
            entry.capability,
            Caller::AgentExternal,
            ceiling,
        )
        .unwrap();
        clients::set_tool_mode(&conn, &self.client.id, tool, mode).unwrap();
    }

    fn call(
        &self,
        tool: &str,
        args: Value,
        approval: Option<&str>,
    ) -> Result<CallStep, BridgeError> {
        self.bridge.call(&self.client, tool, &args, approval)
    }

    fn decide(&self, approval_id: &str, approve: bool) {
        crate::managers::integrations::approvals::decide(
            &self.fx.conn(),
            approval_id,
            approve,
            NOW,
        )
        .unwrap();
    }
}

#[test]
fn recording_through_the_bridge_needs_the_users_decision() {
    let w = world(FakeHost::new());
    // „Erlaubt“ laesst sich fuer die Aufnahme gar nicht einstellen ...
    {
        let conn = w.fx.conn();
        assert!(
            clients::set_tool_mode(&conn, &w.client.id, "start_recording", GrantMode::Allow)
                .is_err()
        );
    }
    w.grant("start_recording", GrantMode::Ask);
    // ... und selbst eine von Hand in die Datenbank geschriebene Zeile „allow“ (Umgehung der
    // Oberflaeche) bleibt bei „fragen“: das Tor rechnet `never_allow` immer ein.
    w.fx.conn()
        .execute(
            "UPDATE agent_tool_grants SET mode = 'allow' WHERE client_id = ?1 AND tool = 'start_recording'",
            [&w.client.id],
        )
        .unwrap();
    w.fx.conn()
        .execute(
            "UPDATE integration_grants SET mode = 'allow' WHERE capability = 'recording.start'",
            [],
        )
        .unwrap();
    let aid = match w
        .call("start_recording", json!({"title": "Jour fixe"}), None)
        .unwrap()
    {
        CallStep::Wait(id) => id,
        other => panic!("Freigabe erwartet: {other:?}"),
    };
    assert_eq!(
        w.host.rec().starts.load(Ordering::SeqCst),
        0,
        "ohne Entscheidung keine Aufnahme"
    );
    // Die App wird sofort benachrichtigt, damit sie die Bitte um Einwilligung zeigt.
    assert_eq!(w.notified.lock().unwrap().as_slice(), ["start_recording"]);
    // Dieselbe Anfrage noch einmal: dieselbe Freigabe, wieder keine Aufnahme.
    match w
        .call("start_recording", json!({"title": "Jour fixe"}), None)
        .unwrap()
    {
        CallStep::Wait(again) => assert_eq!(again, aid),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.host.rec().starts.load(Ordering::SeqCst), 0);
    // Andere Argumente brauchen eine eigene Freigabe: die alte taugt nicht.
    let e = w.call(
        "start_recording",
        json!({"title": "Etwas anderes"}),
        Some(&aid),
    );
    assert!(e.is_err() || matches!(e, Ok(CallStep::Wait(_))));
    assert_eq!(w.host.rec().starts.load(Ordering::SeqCst), 0);

    w.decide(&aid, true);
    match w
        .call("start_recording", json!({"title": "Jour fixe"}), Some(&aid))
        .unwrap()
    {
        CallStep::Done(v) => assert_eq!(v["meeting_id"], "REC1"),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.host.rec().starts.load(Ordering::SeqCst), 1);
    // Eine Freigabe gilt einmal.
    let again = w
        .call("start_recording", json!({"title": "Jour fixe"}), Some(&aid))
        .unwrap_err();
    assert_eq!(again.code, code::APPROVAL_USED);
    assert_eq!(w.host.rec().starts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_denied_recording_request_never_records() {
    let w = world(FakeHost::new());
    w.grant("start_recording", GrantMode::Ask);
    let aid = match w.call("start_recording", json!({}), None).unwrap() {
        CallStep::Wait(id) => id,
        other => panic!("{other:?}"),
    };
    w.decide(&aid, false);
    let e = w
        .call("start_recording", json!({}), Some(&aid))
        .unwrap_err();
    assert_eq!(e.code, code::APPROVAL_DENIED);
    assert_eq!(w.host.rec().starts.load(Ordering::SeqCst), 0);
    // Ohne Recht fuer das Werkzeug fehlt es ganz.
    let off = world(FakeHost::new());
    assert!(off.call("start_recording", json!({}), None).is_err());
    assert_eq!(off.host.rec().starts.load(Ordering::SeqCst), 0);
}

#[test]
fn allowed_tools_run_without_asking_and_do_not_notify() {
    let w = world(FakeHost::new());
    w.grant("create_meeting", GrantMode::Allow);
    match w
        .call("create_meeting", json!({"title": "Direkt"}), None)
        .unwrap()
    {
        CallStep::Done(v) => assert_eq!(v["title"], "Direkt"),
        other => panic!("{other:?}"),
    }
    assert!(w.notified.lock().unwrap().is_empty());
}

#[test]
fn every_tool_action_is_in_the_audit() {
    let w = world(FakeHost::new());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.wav");
    std::fs::write(&file, b"RIFF").unwrap();
    for e in CATALOG {
        w.grant(
            e.name,
            if e.capability.never_allow() {
                GrantMode::Ask
            } else {
                GrantMode::Allow
            },
        );
    }
    let page = match w
        .call(
            "tts_page_create",
            json!({"title": "t", "text": "Text"}),
            None,
        )
        .unwrap()
    {
        CallStep::Done(v) => v["page_id"].as_str().unwrap().to_string(),
        other => panic!("{other:?}"),
    };
    let (base, _) = tauri::async_runtime::block_on(serve(vec![json_ok(OEMBED)]));
    *w.host.oembed.lock().unwrap() = base;
    let mut calls: Vec<(&str, Value)> = vec![
        ("create_session", json!({"name": "S"})),
        ("create_meeting", json!({})),
        ("tts_render_audio", json!({"page_id": page})),
        ("add_youtube_source", json!({"url": LINK})),
    ];
    if cfg!(windows) {
        calls.push(("transcribe_file", json!({"path": file.to_string_lossy()})));
    }
    for (tool, args) in calls {
        match w.call(tool, args, None).unwrap() {
            CallStep::Done(_) => {}
            other => panic!("{tool}: {other:?}"),
        }
    }
    // Aufnahme: fragen -> Freigabe -> Start -> Beenden (auch das braucht eine Freigabe).
    let aid = match w.call("start_recording", json!({}), None).unwrap() {
        CallStep::Wait(id) => id,
        other => panic!("{other:?}"),
    };
    w.decide(&aid, true);
    assert!(matches!(
        w.call("start_recording", json!({}), Some(&aid)).unwrap(),
        CallStep::Done(_)
    ));
    let stop = match w.call("stop_recording", json!({}), None).unwrap() {
        CallStep::Wait(id) => id,
        other => panic!("{other:?}"),
    };
    w.decide(&stop, true);
    assert!(matches!(
        w.call("stop_recording", json!({}), Some(&stop)).unwrap(),
        CallStep::Done(_)
    ));

    let rows = audit::list(&w.fx.conn(), &AuditFilter::default(), 500).unwrap();
    let agent: Vec<_> = rows
        .iter()
        .filter(|a| a.caller == "agent_external")
        .collect();
    for cap in [
        "meeting.create",
        "tts.render",
        "youtube.add",
        "recording.start",
    ] {
        assert!(
            agent
                .iter()
                .any(|a| a.capability.as_deref() == Some(cap) && a.outcome == "ok"),
            "{cap} fehlt im Audit als ok"
        );
    }
    if cfg!(windows) {
        assert!(agent
            .iter()
            .any(|a| a.capability.as_deref() == Some("transcribe.file") && a.outcome == "ok"));
    }
    // AK11: dieselben Aktionen stehen im Audit-Dump (`--audit-dump --json`), mit Summen je Faehigkeit.
    let dump = crate::managers::integrations::dump::build_audit(&w.fx.conn(), None, None).unwrap();
    for cap in [
        "meeting.create",
        "tts.render",
        "youtube.add",
        "recording.start",
        "media.fetch",
    ] {
        assert!(
            dump["by_capability"][cap].as_i64().unwrap_or(0) >= 1,
            "{cap} fehlt im Dump: {}",
            dump["by_capability"]
        );
    }
    assert!(dump["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["caller"] == "agent_external" && e["outcome"] == "ok"));
    // Die Freigaben der Aufnahme stehen als offen/entschieden zugeordnet, jede Aktion mit dem Zugang im Ziel.
    assert!(agent.iter().all(|a| a
        .target
        .as_deref()
        .is_none_or(|t| t.contains("Claude Code") || t.starts_with("youtube:"))));
}

#[test]
fn a_failing_host_is_reported_as_a_tool_error() {
    let host = FakeHost::new();
    let w = world(host.clone());
    w.grant("tts_render_audio", GrantMode::Allow);
    w.grant("tts_page_create", GrantMode::Allow);
    let page = match w
        .call("tts_page_create", json!({"title": "t", "text": "x"}), None)
        .unwrap()
    {
        CallStep::Done(v) => v["page_id"].as_str().unwrap().to_string(),
        other => panic!("{other:?}"),
    };
    *host.fail_render.lock().unwrap() = Some("ffmpeg beendet (Code 1)".to_string());
    let e = w
        .call("tts_render_audio", json!({"page_id": page}), None)
        .unwrap_err();
    assert_eq!(e.code, code::FAILED);
    assert!(e.message.contains("ffmpeg"), "{}", e.message);
    let rows = audit::list(&w.fx.conn(), &AuditFilter::default(), 50).unwrap();
    assert!(rows
        .iter()
        .any(|a| a.capability.as_deref() == Some("tts.render") && a.outcome == "error"));
    // Die Bruecke laeuft weiter.
    *host.fail_render.lock().unwrap() = None;
    assert!(matches!(
        w.call("tts_render_audio", json!({"page_id": page}), None)
            .unwrap(),
        CallStep::Done(_)
    ));
}

#[test]
fn a_panicking_notifier_does_not_break_the_call() {
    let fx = Fx::new();
    let host = FakeHost::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(AppTools::new(store.clone(), host.clone())))
        .unwrap();
    let notifier: ApprovalNotifier = Arc::new(|_| panic!("absichtlich"));
    let bridge = Bridge::new(store, registry, fast_config())
        .with_clock(Arc::new(|| NOW))
        .with_notifier(notifier);
    let (client, _) = clients::create(&fx.conn(), "Codex", None, NOW).unwrap();
    let conn = fx.conn();
    let entry = catalog::find("start_recording").unwrap();
    register::set_grant(
        &conn,
        "agents",
        entry.capability,
        Caller::AgentExternal,
        GrantMode::Ask,
    )
    .unwrap();
    clients::set_tool_mode(&conn, &client.id, "start_recording", GrantMode::Ask).unwrap();
    let step = bridge
        .call(
            &ClientCtx::from(&client),
            "start_recording",
            &json!({}),
            None,
        )
        .unwrap();
    assert!(matches!(step, CallStep::Wait(_)));
    assert_eq!(host.rec().starts.load(Ordering::SeqCst), 0);
}
