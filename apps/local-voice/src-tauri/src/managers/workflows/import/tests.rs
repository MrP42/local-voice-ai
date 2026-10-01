//! Die Bausteine `meeting.import` und `youtube.add_source` (B3), durch die echte Engine mit
//! Attrappen fuer Warteschlange und YouTube-Weg.

use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::*;
use crate::managers::integrations::model::{Capability, GrantMode, NewIntegration};
use crate::managers::integrations::store as integrations;
use crate::managers::workflows::engine::{EnqueueRequest, RunOutcome};
use crate::managers::workflows::model::{RunState, StepState};
use crate::managers::workflows::store::{self, RunRow, StepRow};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine_with, register, roomy_gate, set_grant, step, FakeClock, Fx,
    Scripted, T0,
};

// ---------------------------------------------------------------------------
// Attrappen
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeImport {
    calls: Mutex<Vec<ImportRequest>>,
    finds: Mutex<Vec<(String, i64)>>,
    existing: Mutex<Option<Imported>>,
    errors: Mutex<VecDeque<String>>,
    subtitles_ready: Mutex<bool>,
}

impl ImportControl for FakeImport {
    fn enqueue(&self, req: &ImportRequest) -> Result<Imported, String> {
        if let Some(e) = self.errors.lock().unwrap().pop_front() {
            return Err(e);
        }
        self.calls.lock().unwrap().push(req.clone());
        let ready = *self.subtitles_ready.lock().unwrap();
        Ok(Imported {
            meeting_id: "m-1".to_string(),
            title: req.title.clone(),
            state: if ready {
                ImportState::Ready
            } else {
                ImportState::Queued
            },
        })
    }

    fn find_since(&self, source_path: &str, since_ms: i64) -> Option<Imported> {
        self.finds
            .lock()
            .unwrap()
            .push((source_path.to_string(), since_ms));
        self.existing.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct FakeYoutube {
    existing: Mutex<Option<MeetingRef>>,
    adds: Mutex<Vec<(String, Option<String>)>>,
    errors: Mutex<VecDeque<YoutubeError>>,
}

impl YoutubeControl for FakeYoutube {
    fn find_video(&self, _video_id: &str) -> Option<MeetingRef> {
        self.existing.lock().unwrap().clone()
    }

    fn add(&self, url: &str, project: Option<&str>) -> Result<MeetingRef, YoutubeError> {
        if let Some(e) = self.errors.lock().unwrap().pop_front() {
            return Err(e);
        }
        self.adds
            .lock()
            .unwrap()
            .push((url.to_string(), project.map(str::to_string)));
        Ok(MeetingRef {
            id: "yt-1".to_string(),
            title: "Testvideo".to_string(),
            created_at_ms: T0,
        })
    }
}

/// Meldet jede Datei mit diesem Namen als „nur in der Cloud“.
struct CloudProbe(&'static str);

impl FileProbe for CloudProbe {
    fn cloud_only(&self, path: &Path, _meta: &fs::Metadata) -> bool {
        path.file_name().is_some_and(|n| n == self.0)
    }
    fn locked(&self, _path: &Path) -> bool {
        false
    }
}

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    inbox: tempfile::TempDir,
    import: Arc<FakeImport>,
    youtube: Arc<FakeYoutube>,
    notify: Arc<Scripted>,
}

fn world_with_probe(probe: Arc<dyn FileProbe>) -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine_with(&fx, &clock, roomy_gate());
    let inbox = tempfile::tempdir().unwrap();
    {
        let conn = fx.conn();
        let mut n = NewIntegration::new(Kind::Folder, "Eingang");
        n.id = Some("eingang".to_string());
        n.config = json!({"path": inbox.path().to_string_lossy(), "subfolder": ""});
        integrations::create(&conn, &n, T0).unwrap();
        register(&conn, Kind::Agent, "app-automation");
        set_grant(&conn, "app-automation", Capability::YoutubeAdd, GrantMode::Allow);
    }
    let import = Arc::new(FakeImport::default());
    let youtube = Arc::new(FakeYoutube::default());
    engine.register_action(Arc::new(MeetingImport::with_probe(import.clone(), probe)));
    engine.register_action(Arc::new(YoutubeAddSource::new(youtube.clone())));
    let notify = Scripted::new("notify.local", EffectKind::Idempotent);
    engine.register_action(notify.clone());
    World {
        fx,
        clock,
        engine,
        inbox,
        import,
        youtube,
        notify,
    }
}

fn world() -> World {
    world_with_probe(Arc::new(SystemProbe))
}

impl World {
    fn put(&self, rel: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = self.inbox.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, bytes).unwrap();
        p
    }

    /// Ein scharfer Ablauf mit den Schritten; ein Lauf mit den Ausloeserdaten; ein Takt.
    fn run(&self, steps: Vec<Value>, trigger: Value) -> (RunRow, Vec<StepRow>, RunOutcome) {
        let wf = armed_workflow(&self.engine, &def(steps));
        let mut req = EnqueueRequest::manual(&wf);
        req.trigger = trigger;
        let run_id = self.engine.enqueue(&req).unwrap().run_id;
        let report = self.engine.tick().unwrap();
        let outcome = report
            .outcomes
            .iter()
            .find(|(id, _)| *id == run_id)
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| panic!("kein Ergebnis: {report:?}"));
        let conn = self.fx.conn();
        (
            store::get_run(&conn, &run_id).unwrap().unwrap(),
            store::steps_for(&conn, &run_id).unwrap(),
            outcome,
        )
    }
}

fn import_step(params: Value) -> Value {
    step("imp", "meeting.import", params)
}

fn trigger_for(path: &Path) -> Value {
    json!({
        "path": path.to_string_lossy(),
        "name": path.file_name().unwrap().to_string_lossy(),
    })
}

fn last_error(steps: &[StepRow]) -> String {
    steps
        .iter()
        .rev()
        .find_map(|s| s.error.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// meeting.import
// ---------------------------------------------------------------------------

#[test]
fn a_file_is_enqueued_once_and_the_meeting_is_handed_to_the_next_step() {
    let w = world();
    let file = w.put("Kundentermin.WAV", b"RIFF audio");
    let (run, steps, outcome) = w.run(
        vec![
            import_step(json!({"via": "eingang", "path": "{{trigger.path}}", "title": "Kundentermin"})),
            step("n", "notify.local", json!({"title": "Besprechung {{meeting.id}}"})),
        ],
        trigger_for(&file),
    );
    assert_eq!(outcome, RunOutcome::Done, "{run:?} {steps:?}");
    let calls = w.import.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].title, "Kundentermin");
    assert!(calls[0].path.ends_with("Kundentermin.WAV"));
    assert!(
        !calls[0].path.to_string_lossy().starts_with(r"\\?\"),
        "der Pfad der Besprechung ist der, den der Nutzer kennt"
    );
    assert_eq!(calls[0].project, None);
    let out: Value = serde_json::from_str(steps[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(out["meeting_id"], "m-1");
    assert_eq!(out["status"], "queued");
    assert_eq!(out["file"], "Kundentermin.WAV");
    assert_eq!(out["reused"], false);
    assert!(
        !out.to_string()
            .contains(&file.parent().unwrap().to_string_lossy().to_string()),
        "im Ergebnis steht der Dateiname, kein ganzer Pfad"
    );
    // Der naechste Schritt sieht die Besprechung als `meeting`.
    assert_eq!(w.notify.calls()[0].params["title"], "Besprechung m-1");
}

#[test]
fn the_title_defaults_to_the_file_name_without_extension_and_the_project_is_forwarded() {
    let w = world();
    let file = w.put("Abteilungsrunde 12.mp3", b"mp3");
    let (_, steps, outcome) = w.run(
        vec![import_step(json!({
            "via": "eingang", "path": "{{trigger.path}}", "project": "projekt-1"
        }))],
        trigger_for(&file),
    );
    assert_eq!(outcome, RunOutcome::Done, "{steps:?}");
    let calls = w.import.calls.lock().unwrap().clone();
    assert_eq!(calls[0].title, "Abteilungsrunde 12");
    assert_eq!(calls[0].project.as_deref(), Some("projekt-1"));
}

#[test]
fn a_subtitle_file_is_ready_at_once() {
    let w = world();
    *w.import.subtitles_ready.lock().unwrap() = true;
    let file = w.put("Untertitel.vtt", b"WEBVTT\n\n00:00.000 --> 00:01.000\nHallo\n");
    let (_, steps, outcome) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))],
        trigger_for(&file),
    );
    assert_eq!(outcome, RunOutcome::Done);
    let out: Value = serde_json::from_str(steps[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(out["status"], "ready");
}

#[test]
fn a_relative_path_inside_the_folder_works_and_dotdot_does_not() {
    let w = world();
    w.put("Unter/ordner.wav", b"x");
    let (_, steps, outcome) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "Unter/ordner.wav"}))],
        json!({}),
    );
    assert_eq!(outcome, RunOutcome::Done, "{steps:?}");
    assert_eq!(w.import.calls.lock().unwrap().len(), 1);

    let (run, steps, _) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "../draussen.wav"}))],
        json!({}),
    );
    assert_eq!(run.state, RunState::Failed);
    assert!(last_error(&steps).contains("nicht zulässig"), "{steps:?}");
    assert_eq!(w.import.calls.lock().unwrap().len(), 1, "nichts weiter eingereiht");
}

#[test]
fn a_path_outside_the_folder_is_refused_for_good_and_nothing_is_enqueued() {
    let w = world();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("fremd.wav");
    fs::write(&outside, b"geheim").unwrap();
    let (run, steps, outcome) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))],
        trigger_for(&outside),
    );
    assert!(matches!(outcome, RunOutcome::Failed { .. }), "{outcome:?}");
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(steps.last().unwrap().state, StepState::Failed);
    assert!(
        last_error(&steps).contains("nicht im Ordner"),
        "{}",
        last_error(&steps)
    );
    assert!(w.import.calls.lock().unwrap().is_empty());
    assert_eq!(steps.len(), 1, "ein dauerhafter Fehler wird nicht wiederholt");
}

#[test]
fn unsuitable_sources_are_refused_with_a_sentence() {
    let w = world();
    let doc = w.put("notiz.docx", b"x");
    let empty = w.put("leer.wav", b"");
    let missing = w.inbox.path().join("weg.wav");
    let dir = w.inbox.path().join("ordner.wav");
    fs::create_dir(&dir).unwrap();
    for (path, expect) in [
        (doc, "lassen sich nicht importieren"),
        (empty, "leer"),
        (missing, "gibt es nicht"),
        (dir, "keine Datei"),
    ] {
        let (run, steps, _) = w.run(
            vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))],
            trigger_for(&path),
        );
        assert_eq!(run.state, RunState::Failed, "{path:?}");
        assert!(last_error(&steps).contains(expect), "{path:?}: {}", last_error(&steps));
    }
    assert!(w.import.calls.lock().unwrap().is_empty());
}

#[test]
fn only_a_folder_integration_is_a_source() {
    let w = world();
    register(&w.fx.conn(), Kind::Obsidian, "vault");
    let file = w.put("a.wav", b"x");
    // Die Ordner-Integration „eingang“ gibt es; „vault“ hat FilesRead, ist aber kein Ordner.
    let (run, steps, _) = w.run(
        vec![import_step(json!({"via": "vault", "path": "{{trigger.path}}"}))],
        trigger_for(&file),
    );
    assert_eq!(run.state, RunState::Failed);
    assert!(last_error(&steps).contains("keine Ordner-Integration"), "{}", last_error(&steps));
    let (run, steps, _) = w.run(
        vec![import_step(json!({"via": "gibtsnicht", "path": "{{trigger.path}}"}))],
        trigger_for(&file),
    );
    assert_eq!(run.state, RunState::Failed);
    assert!(!last_error(&steps).is_empty());
    assert!(w.import.calls.lock().unwrap().is_empty());
}

#[test]
fn a_cloud_only_file_is_not_read_and_not_enqueued() {
    let w = world_with_probe(Arc::new(CloudProbe("platzhalter.wav")));
    let file = w.put("platzhalter.wav", b"inhalt");
    let (run, steps, _) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))],
        trigger_for(&file),
    );
    assert_eq!(run.state, RunState::Failed);
    assert!(
        last_error(&steps).contains("nur in der Cloud") && last_error(&steps).contains("nicht heruntergeladen"),
        "{}",
        last_error(&steps)
    );
    assert!(w.import.calls.lock().unwrap().is_empty());
}

#[test]
fn a_repeated_step_reuses_the_meeting_it_already_created() {
    let w = world();
    let file = w.put("zweimal.wav", b"x");
    // Der erste Versuch hat eingereiht, das Journal kam nicht mehr: die Besprechung gibt es schon.
    *w.import.existing.lock().unwrap() = Some(Imported {
        meeting_id: "m-alt".to_string(),
        title: "zweimal".to_string(),
        state: ImportState::Queued,
    });
    let (run, steps, outcome) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))],
        trigger_for(&file),
    );
    assert_eq!(outcome, RunOutcome::Done, "{run:?}");
    assert!(w.import.calls.lock().unwrap().is_empty(), "kein zweites Einreihen");
    let out: Value = serde_json::from_str(steps[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(out["meeting_id"], "m-alt");
    assert_eq!(out["reused"], true);
    // Gesucht wird nach dem Pfad der Datei und der Zeit des Schrittbeginns.
    let finds = w.import.finds.lock().unwrap().clone();
    assert!(finds[0].0.ends_with("zweimal.wav"));
    assert!(finds[0].1 <= T0 && finds[0].1 > T0 - 10_000, "{finds:?}");
}

#[test]
fn queue_errors_are_classified_so_that_retries_are_safe() {
    // Nichts entstanden: wiederholen.
    for code in ["queue_enqueue_failed: database is locked", "meeting_create_failed: disk full", "meetings_unavailable"] {
        assert!(matches!(map_import_error(code), StepError::Transient(_)), "{code}");
    }
    // Wird nie gelingen.
    for code in ["folder_not_found", "subtitle_invalid", "subtitle_unreadable: x", "import_path_invalid"] {
        assert!(matches!(map_import_error(code), StepError::Permanent(_)), "{code}");
    }
    // Unklar, ob etwas zurueckblieb: nie von selbst wiederholen.
    for code in ["segments_store_failed: x", "status_ready_failed", "etwas anderes"] {
        assert!(matches!(map_import_error(code), StepError::Unknown(_)), "{code}");
    }
}

#[test]
fn a_transient_enqueue_error_is_retried_and_then_succeeds_without_a_second_meeting() {
    let w = world();
    let file = w.put("retry.wav", b"x");
    w.import
        .errors
        .lock()
        .unwrap()
        .push_back("queue_enqueue_failed: database is locked".to_string());
    let (run, steps, outcome) = w.run(
        vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))],
        trigger_for(&file),
    );
    assert!(matches!(outcome, RunOutcome::Parked { .. }), "{outcome:?} {run:?}");
    assert_eq!(steps.last().unwrap().state, StepState::Retrying);
    // Nach der Wartezeit gelingt der zweite Versuch.
    w.clock.advance(10 * 60_000);
    let report = w.engine.tick().unwrap();
    assert!(report.outcomes.iter().any(|(_, o)| *o == RunOutcome::Done), "{report:?}");
    assert_eq!(w.import.calls.lock().unwrap().len(), 1);
}

#[test]
fn the_right_files_read_is_checked_before_anything_is_enqueued() {
    let w = world();
    let file = w.put("recht.wav", b"x");
    let steps_def = vec![import_step(json!({"via": "eingang", "path": "{{trigger.path}}"}))];
    // „aus“: der Schritt wird abgelehnt.
    set_grant(&w.fx.conn(), "eingang", Capability::FilesRead, GrantMode::Off);
    let (run, steps, _) = w.run(steps_def.clone(), trigger_for(&file));
    assert_eq!(run.state, RunState::Failed);
    assert_eq!(steps.last().unwrap().state, StepState::Denied, "{steps:?}");
    // „fragen“: der Lauf wartet auf die Freigabe.
    set_grant(&w.fx.conn(), "eingang", Capability::FilesRead, GrantMode::Ask);
    let (_, _, outcome) = w.run(steps_def, trigger_for(&file));
    assert_eq!(outcome, RunOutcome::AwaitingApproval);
    assert!(w.import.calls.lock().unwrap().is_empty());
}

#[test]
fn the_import_step_is_a_heavy_transcription_step_with_an_external_effect() {
    let a = MeetingImport::new(Arc::new(FakeImport::default()));
    assert_eq!(a.effect(), EffectKind::External);
    let need = a.heavy(&json!({})).expect("schwer");
    assert_eq!(need.label, "Transkription");
    let needs = a
        .needs(&json!({"via": "eingang", "path": "C:/Eingang/a.wav"}))
        .unwrap()
        .unwrap();
    assert_eq!(needs.capability, Capability::FilesRead);
    assert_eq!(needs.integration_id, "eingang");
    assert_eq!(needs.target.as_deref(), Some("C:/Eingang/a.wav"));
    assert!(a
        .describe(&json!({"via": "eingang", "path": "a.wav"}))
        .contains("a.wav"));
}

#[test]
fn meeting_readiness_follows_the_status_of_the_meeting() {
    for (status, expect) in [
        ("ready", MeetingReadiness::Ready),
        ("queued", MeetingReadiness::Pending),
        ("processing", MeetingReadiness::Pending),
        ("recording", MeetingReadiness::Pending),
        ("failed", MeetingReadiness::Never),
        ("cancelled", MeetingReadiness::Never),
        ("unbekannt", MeetingReadiness::Never),
    ] {
        assert_eq!(meeting_state(status), expect, "{status}");
    }
}

// ---------------------------------------------------------------------------
// youtube.add_source
// ---------------------------------------------------------------------------

fn add_step(params: Value) -> Value {
    step("yt", "youtube.add_source", params)
}

#[test]
fn a_video_becomes_a_source_with_its_meeting_and_the_project() {
    let w = world();
    let (run, steps, outcome) = w.run(
        vec![
            add_step(json!({"via": "app-automation", "url": "{{trigger.url}}", "project": "p-1"})),
            step("n", "notify.local", json!({"title": "Quelle {{meeting.id}}"})),
        ],
        json!({"url": "https://www.youtube.com/watch?v=Vid00000001&t=30&si=TRACK"}),
    );
    assert_eq!(outcome, RunOutcome::Done, "{run:?} {steps:?}");
    let adds = w.youtube.adds.lock().unwrap().clone();
    assert_eq!(adds.len(), 1);
    assert_eq!(adds[0].1.as_deref(), Some("p-1"));
    let out: Value = serde_json::from_str(steps[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(out["meeting_id"], "yt-1");
    assert_eq!(out["video_id"], "Vid00000001");
    assert_eq!(out["url"], "https://www.youtube.com/watch?v=Vid00000001", "die bereinigte Adresse");
    assert_eq!(out["reused"], false);
    assert_eq!(w.notify.calls()[0].params["title"], "Quelle yt-1");
}

#[test]
fn the_same_video_is_never_added_twice() {
    let w = world();
    *w.youtube.existing.lock().unwrap() = Some(MeetingRef {
        id: "yt-alt".to_string(),
        title: "Schon da".to_string(),
        created_at_ms: T0 - 86_400_000,
    });
    let (_, steps, outcome) = w.run(
        vec![add_step(json!({"via": "app-automation", "url": "https://youtu.be/Vid00000001"}))],
        json!({}),
    );
    assert_eq!(outcome, RunOutcome::Done);
    assert!(w.youtube.adds.lock().unwrap().is_empty());
    let out: Value = serde_json::from_str(steps[0].output_json.as_deref().unwrap()).unwrap();
    assert_eq!(out["meeting_id"], "yt-alt");
    assert_eq!(out["reused"], true);
}

#[test]
fn a_bad_link_and_a_missing_right_stop_the_step_before_any_network_call() {
    let w = world();
    for url in ["https://www.youtube.com/playlist?list=PLx", "kein link", "https://vimeo.com/1"] {
        let (run, steps, _) = w.run(
            vec![add_step(json!({"via": "app-automation", "url": url}))],
            json!({}),
        );
        assert_eq!(run.state, RunState::Failed, "{url}");
        assert_eq!(steps.last().unwrap().state, StepState::Failed, "{url}");
    }
    set_grant(&w.fx.conn(), "app-automation", Capability::YoutubeAdd, GrantMode::Off);
    let (_, steps, _) = w.run(
        vec![add_step(json!({"via": "app-automation", "url": "https://youtu.be/Vid00000001"}))],
        json!({}),
    );
    assert_eq!(steps.last().unwrap().state, StepState::Denied);
    assert!(w.youtube.adds.lock().unwrap().is_empty());
}

#[test]
fn youtube_errors_are_classified_by_what_may_have_happened() {
    use YoutubeError as E;
    for e in [
        E::Busy,
        E::RateLimited,
        E::Timeout,
        E::Network("x".into()),
        E::Http(503),
        E::BadResponse("x".into()),
        E::Store("x".into()),
    ] {
        assert!(matches!(map_youtube_error(&e), StepError::Transient(_)), "{e:?}");
    }
    assert!(matches!(
        map_youtube_error(&E::Disabled("aus".into())),
        StepError::Denied(_)
    ));
    for e in [E::Unavailable, E::Project, E::Link(crate::managers::youtube::link::LinkError::Playlist)] {
        assert!(matches!(map_youtube_error(&e), StepError::Permanent(_)), "{e:?}");
    }
}

#[test]
fn a_network_error_while_adding_is_retried_and_creates_one_source() {
    let w = world();
    w.youtube.errors.lock().unwrap().push_back(YoutubeError::Timeout);
    let (_, _, outcome) = w.run(
        vec![add_step(json!({"via": "app-automation", "url": "https://youtu.be/Vid00000001"}))],
        json!({}),
    );
    assert!(matches!(outcome, RunOutcome::Parked { .. }), "{outcome:?}");
    w.clock.advance(10 * 60_000);
    let report = w.engine.tick().unwrap();
    assert!(report.outcomes.iter().any(|(_, o)| *o == RunOutcome::Done), "{report:?}");
    assert_eq!(w.youtube.adds.lock().unwrap().len(), 1);
}

#[test]
fn the_effect_of_add_source_is_external_and_needs_the_youtube_add_right() {
    let a = YoutubeAddSource::new(Arc::new(FakeYoutube::default()));
    assert_eq!(a.effect(), EffectKind::External);
    assert!(a.heavy(&json!({})).is_none(), "leicht: ein oEmbed-Abruf");
    let n = a
        .needs(&json!({"via": "app-automation", "url": "https://youtu.be/Vid00000001"}))
        .unwrap()
        .unwrap();
    assert_eq!(n.capability, Capability::YoutubeAdd);
    assert_eq!(n.integration_id, "app-automation");
}

// ---------------------------------------------------------------------------
// Ausloeser -> Baustein, Ende zu Ende
// ---------------------------------------------------------------------------

#[test]
fn a_file_in_the_inbox_runs_through_trigger_and_import_exactly_once() {
    use crate::managers::workflows::trigger::folder;
    let w = world();
    let mut d = def(vec![import_step(json!({
        "via": "eingang", "path": "{{trigger.path}}", "title": "Kundentermin"
    }))]);
    d["trigger"] =
        json!({"type": "folder.file_added", "integration": "eingang", "stable_seconds": 2});
    armed_workflow(&w.engine, &d);
    w.put("Kunde.wav", b"RIFF audio");
    let state = folder::State::default();
    let mut now = T0;
    for _ in 0..6 {
        now += 15_000;
        w.clock.set(now);
        folder::on_tick(&w.engine, &state, &w.fx.db_path, &SystemProbe, now);
        w.engine.tick().unwrap();
    }
    let calls = w.import.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "genau eine Besprechung fuer die Datei");
    assert_eq!(calls[0].title, "Kundentermin");
    assert!(calls[0].path.ends_with("Kunde.wav"));
}

#[test]
fn a_new_channel_video_runs_through_trigger_and_add_source_exactly_once() {
    use crate::managers::workflows::trigger::youtube_channel as channel;
    struct Feed(String);
    impl channel::FeedFetcher for Feed {
        fn fetch(&self, _c: &str) -> Result<String, channel::FeedError> {
            Ok(self.0.clone())
        }
    }
    let w = world();
    let ch = "UCabcdefghijklmnopqrstuv";
    let mut d = def(vec![add_step(
        json!({"via": "app-automation", "url": "{{trigger.url}}"}),
    )]);
    d["trigger"] = json!({"type": "youtube.channel_new_video", "channel_id": ch, "backfill": 1});
    armed_workflow(&w.engine, &d);
    let xml = format!(
        "<feed><yt:channelId>{ch}</yt:channelId><title>K</title><entry><yt:videoId>Vid00000001</yt:videoId><title>T</title></entry></feed>"
    );
    let state = channel::State::default();
    let mut now = T0;
    for _ in 0..3 {
        now += 61 * 60_000;
        w.clock.set(now);
        channel::on_tick(&w.engine, &state, &Feed(xml.clone()), &w.fx.db_path, now);
        w.engine.tick().unwrap();
    }
    let adds = w.youtube.adds.lock().unwrap().clone();
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0].0, "https://www.youtube.com/watch?v=Vid00000001");
}
