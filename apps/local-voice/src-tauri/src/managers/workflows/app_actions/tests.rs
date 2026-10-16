use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use super::*;
use crate::managers::integrations::approvals;
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{Capability, GrantMode, NewIntegration};
use crate::managers::meetings::llm_call::test_support::{
    chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
};
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, StoredSegment, TranscriptDelta,
};
use crate::managers::workflows::action::HeavyNeed;
use crate::managers::workflows::cli;
use crate::managers::workflows::engine::{Clock, EnqueueRequest, RunOutcome};
use crate::managers::workflows::model::{Origin, RunState, StepState};
use crate::managers::workflows::templates;
use crate::managers::workflows::test_support::{
    armed_workflow, engine, set_grant, step, FakeClock, Fx, T0,
};
use crate::settings::AppSettings;

const TITLE: &str = "Jour fixe Überprüfung";

// ---------------------------------------------------------------------------
// Aufbau: Dienste mit Mock-Modell, Besprechung, Ordner
// ---------------------------------------------------------------------------

struct TestServices {
    store: Arc<MeetingStore>,
    settings: AppSettings,
    audio_dir: PathBuf,
    notified: Mutex<Vec<(String, String)>>,
    speech_calls: AtomicUsize,
    local: bool,
}

impl AppServices for TestServices {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError> {
        Ok(self.store.clone())
    }

    fn llm_is_local(&self) -> bool {
        self.local
    }

    fn generate_notes(
        &self,
        req: &GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        use crate::managers::meetings::notes::enhance;
        let fut = enhance::enhance_meeting_with_basis(
            &self.settings,
            self.store.clone(),
            &req.meeting_id,
            req.template_id.as_deref(),
            &req.basis,
            |_, _| {},
        );
        finish_generation(run_cancellable(cancel, fut))
    }

    fn generate_minutes(
        &self,
        req: &GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        use crate::managers::meetings::minutes;
        let fut = minutes::generate_minutes_with_basis(
            &self.settings,
            self.store.clone(),
            &req.meeting_id,
            req.template_id.as_deref(),
            &req.basis,
            &|_| {},
        );
        finish_generation(run_cancellable(cancel, fut))
    }

    fn summarize(
        &self,
        text: &str,
        opts: &SummaryOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError> {
        finish_generation(run_cancellable(
            cancel,
            crate::summarizer::summarize(&self.settings, text, opts),
        ))
    }

    fn audio_dir(&self, meeting_id: Option<&str>) -> Result<PathBuf, ServiceError> {
        Ok(match meeting_id {
            Some(id) => self.audio_dir.join(id),
            None => self.audio_dir.clone(),
        })
    }

    fn render_speech(
        &self,
        text: &str,
        out: &Path,
        _cancel: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ServiceError> {
        self.speech_calls.fetch_add(1, Ordering::SeqCst);
        std::fs::write(out, format!("RIFF-test {}", text.len()))
            .map_err(|e| ServiceError::Transient(e.to_string()))?;
        Ok(out.to_path_buf())
    }

    fn notify(&self, title: &str, body: &str) -> Result<(), ServiceError> {
        self.notified
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string()));
        Ok(())
    }
}

/// Antwort passend zur Standardvorlage „Allgemein“ des Protokolls (mit Umlauten).
fn minutes_reply() -> String {
    json!({
        "zusammenfassung": ["Der Go-Live wurde auf Ende Oktober gelegt; die Abnahme prüft Frau Müller."],
        "besprochene_punkte": ["Größe der Testgruppe", "Änderungen am Zeitplan"],
        "entscheidungen": ["Go-Live am 30. Oktober"],
        "aufgaben": [{ "text": "Release-Notes schreiben", "assignee": null, "due": null }],
        "offene_fragen": []
    })
    .to_string()
}

/// Antwort passend zu den KI-Notizen der Standardvorlage.
fn notes_reply() -> String {
    json!({
        "zusammenfassung": [{"ref": null, "text": "Der Kunde will im Herbst starten.", "sources": ["S1"]}],
        "besprochene_punkte": [{"ref": null, "text": "Größe des Pilotprojekts.", "sources": ["S2"]}],
        "entscheidungen": [],
        "aufgaben": [{"ref": null, "text": "Angebot senden", "sources": ["S3"], "assignee": null, "due": null}],
        "offene_fragen": []
    })
    .to_string()
}

fn segments() -> Vec<StoredSegment> {
    (0..4u32)
        .map(|i| StoredSegment {
            segment_index: i,
            text: format!(
                "Zeile {i}: der Zeitplan und die Größe der Testgruppe werden besprochen, die Änderungen folgen."
            ),
            start_ms: u64::from(i) * 60_000,
            end_ms: u64::from(i) * 60_000 + 50_000,
            channel: 1,
            speaker_index: None,
            words: None,
        })
        .collect()
}

/// Legt eine Besprechung mit Transkript an (`status`: was der Import daraus macht).
fn make_meeting(store: &MeetingStore, title: &str, status: MeetingStatus) -> String {
    let m = store
        .create_meeting(title, MeetingSource::Import, None)
        .unwrap();
    store
        .append_delta(
            &m.id,
            &TranscriptDelta {
                new_segments: segments(),
            },
        )
        .unwrap();
    store.set_status(&m.id, status).unwrap();
    m.id
}

/// Stand-in fuer den Baustein „Datei importieren“ aus B3: legt eine Besprechung an.
struct FakeImport {
    store: Arc<MeetingStore>,
    status: Mutex<MeetingStatus>,
    made: Mutex<Vec<String>>,
}

impl FakeImport {
    fn made(&self) -> Vec<String> {
        self.made.lock().unwrap().clone()
    }
}

impl Action for FakeImport {
    fn id(&self) -> &str {
        "meeting.import"
    }
    fn effect(&self) -> EffectKind {
        EffectKind::External
    }
    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(spec_of("meeting.import"), params)
    }
    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(spec_of("meeting.import"), params)
    }
    fn run(&self, _ctx: &RunCtx<'_>, _params: &Value) -> Result<StepOutput, StepError> {
        let status = match *self.status.lock().unwrap() {
            MeetingStatus::Ready => MeetingStatus::Ready,
            MeetingStatus::Failed => MeetingStatus::Failed,
            MeetingStatus::Cancelled => MeetingStatus::Cancelled,
            MeetingStatus::Recording => MeetingStatus::Recording,
            MeetingStatus::Processing => MeetingStatus::Processing,
        };
        let id = make_meeting(&self.store, TITLE, status);
        self.made.lock().unwrap().push(id.clone());
        Ok(StepOutput::with_data(
            json!({"meeting": {"id": id, "title": TITLE}, "meeting_id": id}),
        ))
    }
}

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    services: Arc<TestServices>,
    import: Arc<FakeImport>,
    out_dir: PathBuf,
    llm_calls: Arc<AtomicUsize>,
}

fn temp_dir() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().to_path_buf();
    std::mem::forget(dir);
    p
}

fn world_with(
    reply: impl Fn(&str) -> MockReply + Send + Sync + 'static,
    import_status: MeetingStatus,
) -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    let llm_calls = Arc::new(AtomicUsize::new(0));
    let counter = llm_calls.clone();
    let port = tauri::async_runtime::block_on(spawn_llm_mock_with(move |body| {
        counter.fetch_add(1, Ordering::SeqCst);
        reply(body)
    }));
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let services = Arc::new(TestServices {
        store: store.clone(),
        settings: settings_with_mock_provider(port),
        audio_dir: temp_dir(),
        notified: Mutex::new(Vec::new()),
        speech_calls: AtomicUsize::new(0),
        local: true,
    });
    install(&engine, services.clone());
    let import = Arc::new(FakeImport {
        store,
        status: Mutex::new(import_status),
        made: Mutex::new(Vec::new()),
    });
    engine.register_action(import.clone());

    // Ordner-Integrationen des Nutzers: Eingang (lesen) und Ziel (schreiben).
    let out_dir = temp_dir();
    let in_dir = temp_dir();
    {
        let conn = fx.conn();
        for (id, dir) in [("folder-eingang", &in_dir), ("folder-protokolle", &out_dir)] {
            let mut n = NewIntegration::new(
                crate::managers::integrations::model::Kind::Folder,
                id,
            );
            n.id = Some(id.to_string());
            n.config = json!({ "path": dir.to_string_lossy() });
            crate::managers::integrations::store::create(&conn, &n, T0).unwrap();
        }
        set_grant(&conn, "folder-eingang", Capability::FilesRead, GrantMode::Allow);
        set_grant(&conn, "folder-protokolle", Capability::FilesWrite, GrantMode::Allow);
    }
    World {
        fx,
        clock,
        engine,
        services,
        import,
        out_dir,
        llm_calls,
    }
}

fn world() -> World {
    world_with(|_| MockReply::Body(chat_body(&minutes_reply())), MeetingStatus::Ready)
}

impl World {
    fn conn(&self) -> rusqlite::Connection {
        self.fx.conn()
    }

    /// Der Ablauf der Vorlage „Eingangsordner -> Word“, scharf geschaltet.
    fn shipped(&self) -> String {
        armed_workflow(&self.engine, &shipped_definition())
    }

    /// Simulierter Ausloeser (B3 liefert den echten): eine Datei ist im Eingangsordner gelandet.
    fn file_arrives(&self, workflow: &str, name: &str) -> String {
        let req = EnqueueRequest {
            workflow_id: workflow.to_string(),
            trigger_key: format!("folder:folder-eingang:{name}"),
            origin: Origin::Trigger,
            trigger: json!({
                "integration": "folder-eingang",
                "path": format!("C:/Eingang/{name}"),
                "name": name,
                "extension": "wav",
                "size": 1_048_576
            }),
            vars: serde_json::Map::new(),
            force_dry_run: false,
        };
        let queued = self.engine.enqueue(&req).unwrap();
        assert!(!queued.dry_run);
        queued.run_id
    }

    fn detail(&self, run: &str) -> crate::managers::workflows::engine::RunDetail {
        self.engine.run_detail(run).unwrap()
    }

    fn step_state(&self, run: &str, step_id: &str) -> StepState {
        self.detail(run)
            .steps
            .iter()
            .rfind(|s| s.step_id == step_id)
            .map(|s| s.state)
            .unwrap_or_else(|| panic!("Schritt {step_id} fehlt"))
    }

    fn files_in_target(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.out_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort();
        v
    }

    fn audit(&self) -> Vec<crate::managers::integrations::model::AuditEntry> {
        audit::list(&self.conn(), &AuditFilter::default(), 200).unwrap()
    }
}

fn shipped_definition() -> Value {
    serde_json::from_str(templates::FOLDER_TO_WORD).unwrap()
}

fn docx_xml(path: &Path) -> String {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).expect("gültiges ZIP");
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .expect("word/document.xml")
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

/// Ein RunCtx fuer den direkten Aufruf eines Bausteins (ohne Engine).
struct Direct {
    cancel: AtomicBool,
    context: Value,
}

impl Direct {
    fn new(meeting_id: Option<&str>) -> Self {
        let context = match meeting_id {
            Some(id) => json!({"meeting": {"id": id, "title": TITLE}, "trigger": {}, "steps": {}}),
            None => json!({"trigger": {}, "steps": {}}),
        };
        Self {
            cancel: AtomicBool::new(false),
            context,
        }
    }

    fn run(
        &self,
        w: &World,
        action: &dyn Action,
        run_id: &str,
        step_id: &str,
        params: &Value,
    ) -> Result<StepOutput, StepError> {
        let clock: &dyn Clock = &*w.clock;
        let ctx = RunCtx {
            workflow_id: "wf-test",
            run_id,
            step_id,
            attempt: 1,
            idempotency_key: format!("{run_id}:{step_id}"),
            context: &self.context,
            step_started_at: T0,
            approved: false,
            gate_args: None,
            cancel: &self.cancel,
            clock,
            db_path: &w.fx.db_path,
        };
        action.run(&ctx, params)
    }
}

fn action_of(w: &World, id: &str) -> Arc<dyn Action> {
    w.engine.registry().get(id).unwrap().clone()
}

// ---------------------------------------------------------------------------
// AK6: Eingangsordner -> Word
// ---------------------------------------------------------------------------

#[test]
fn a_file_in_the_inbox_becomes_a_word_protocol_in_the_target_folder() {
    let w = world();
    let wf = w.shipped();
    let run = w.file_arrives(&wf, "Kundentermin.wav");
    let report = w.engine.tick().unwrap();
    assert_eq!(
        report.outcomes,
        vec![(run.clone(), RunOutcome::Done)],
        "{report:?}"
    );

    // Laufprotokoll: alle drei Schritte „ok“.
    let detail = w.detail(&run);
    assert_eq!(detail.run.state, RunState::Done);
    for id in ["import", "protokoll", "word"] {
        assert_eq!(w.step_state(&run, id), StepState::Done, "Schritt {id}");
    }

    // Die Datei: ZIP mit word/document.xml, Titel und Umlaute.
    let files = w.files_in_target();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().to_string();
    assert_eq!(name, format!("{TITLE} – Protokoll.docx"));
    let xml = docx_xml(&files[0]);
    assert!(xml.contains("Jour fixe Überprüfung"), "Titel fehlt");
    assert!(xml.contains("Größe der Testgruppe"), "Umlaut Ö fehlt");
    assert!(xml.contains("Änderungen am Zeitplan"));
    assert!(xml.contains("Frau Müller"));
    assert!(xml.contains("Go-Live am 30. Oktober"));

    // Der Ausgang des Exportschritts nennt den Pfad (fuer die folgenden Schritte).
    let out: Value = serde_json::from_str(
        detail
            .steps
            .iter()
            .find(|s| s.step_id == "word")
            .unwrap()
            .output_json
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert!(out["path"].as_str().unwrap().ends_with("Protokoll.docx"));
    assert!(!out["path"].as_str().unwrap().starts_with(r"\\?\"));
    assert_eq!(out["format"], "docx");

    // Das Tor hat den Export als Aktion des Ablaufs gebucht (ok), mit dem Ziel.
    let audit = w.audit();
    assert!(
        audit.iter().any(|a| a.caller == "workflow"
            && a.capability.as_deref() == Some("files.write")
            && a.integration_id.as_deref() == Some("folder-protokolle")
            && a.outcome == "ok"),
        "{audit:?}"
    );
    // Der Weg des Protokolls: ein Modellaufruf.
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn the_same_file_twice_makes_one_run_and_one_document() {
    let w = world();
    let wf = w.shipped();
    let first = w.file_arrives(&wf, "A.wav");
    w.engine.tick().unwrap();
    // Derselbe Ausloeser noch einmal (zweiter Takt, Neustart): derselbe Lauf, nichts Neues.
    let again = w
        .engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf.clone(),
            trigger_key: "folder:folder-eingang:A.wav".to_string(),
            origin: Origin::Trigger,
            trigger: json!({"integration": "folder-eingang", "path": "C:/Eingang/A.wav"}),
            vars: serde_json::Map::new(),
            force_dry_run: false,
        })
        .unwrap();
    assert!(!again.created);
    assert_eq!(again.run_id, first);
    assert!(w.engine.tick().unwrap().outcomes.is_empty());
    assert_eq!(w.files_in_target().len(), 1);
    assert_eq!(w.import.made().len(), 1);
}

// ---------------------------------------------------------------------------
// AK8: Rechte
// ---------------------------------------------------------------------------

#[test]
fn an_export_target_that_is_off_denies_the_step_and_audits_it() {
    let w = world();
    set_grant(
        &w.conn(),
        "folder-protokolle",
        Capability::FilesWrite,
        GrantMode::Off,
    );
    let wf = w.shipped();
    let run = w.file_arrives(&wf, "Kundentermin.wav");
    let report = w.engine.tick().unwrap();
    assert!(
        matches!(report.outcomes[0].1, RunOutcome::Failed { .. }),
        "{report:?}"
    );
    assert_eq!(w.step_state(&run, "protokoll"), StepState::Done);
    assert_eq!(w.step_state(&run, "word"), StepState::Denied);
    assert_eq!(w.detail(&run).run.state, RunState::Failed);
    assert!(w.files_in_target().is_empty(), "nichts darf geschrieben sein");
    let denied: Vec<_> = w
        .audit()
        .into_iter()
        .filter(|a| a.outcome == "denied" && a.capability.as_deref() == Some("files.write"))
        .collect();
    assert_eq!(denied.len(), 1, "genau ein Audit-Eintrag „abgelehnt“");
    assert_eq!(denied[0].caller, "workflow");
    assert_eq!(denied[0].integration_id.as_deref(), Some("folder-protokolle"));
}

#[test]
fn an_export_target_that_asks_waits_for_the_approval_and_writes_after_it() {
    let w = world();
    set_grant(
        &w.conn(),
        "folder-protokolle",
        Capability::FilesWrite,
        GrantMode::Ask,
    );
    let wf = w.shipped();
    let run = w.file_arrives(&wf, "Kundentermin.wav");
    let report = w.engine.tick().unwrap();
    assert_eq!(report.outcomes[0].1, RunOutcome::AwaitingApproval, "{report:?}");
    assert!(w.files_in_target().is_empty(), "vor der Freigabe nichts schreiben");
    let pending = approvals::list_pending(&w.conn(), T0).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool_or_capability, "files.write");

    approvals::decide(&w.conn(), &pending[0].id, true, T0).unwrap();
    let report = w.engine.tick().unwrap();
    assert_eq!(report.outcomes[0].1, RunOutcome::Done, "{report:?}");
    assert_eq!(w.files_in_target().len(), 1);
    // Genau EINE Freigabe, nicht zwei (kein zweiter Torgang im Baustein).
    assert_eq!(w.detail(&run).run.state, RunState::Done);
}

// ---------------------------------------------------------------------------
// Warten, bis die Besprechung fertig ist
// ---------------------------------------------------------------------------

#[test]
fn minutes_wait_for_a_meeting_that_is_still_processing_without_calling_the_model() {
    let w = world_with(
        |_| MockReply::Body(chat_body(&minutes_reply())),
        MeetingStatus::Processing,
    );
    let wf = w.shipped();
    let run = w.file_arrives(&wf, "Lang.wav");
    let report = w.engine.tick().unwrap();
    assert!(
        matches!(report.outcomes[0].1, RunOutcome::Parked { .. }),
        "{report:?}"
    );
    assert_eq!(w.step_state(&run, "protokoll"), StepState::Waiting);
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 0);
    assert_eq!(w.detail(&run).run.next_run_at, Some(T0 + READY_POLL_MS as i64));

    // Die Verarbeitung ist fertig: der Lauf geht weiter, ein Versuch, ein Modellaufruf.
    let id = w.import.made()[0].clone();
    w.services
        .store
        .set_status(&id, MeetingStatus::Ready)
        .unwrap();
    w.clock.advance(READY_POLL_MS as i64);
    let report = w.engine.tick().unwrap();
    assert_eq!(report.outcomes[0].1, RunOutcome::Done, "{report:?}");
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 1);
    assert_eq!(w.files_in_target().len(), 1);
}

#[test]
fn a_meeting_that_never_gets_ready_ends_the_step_after_six_hours() {
    let w = world_with(
        |_| MockReply::Body(chat_body(&minutes_reply())),
        MeetingStatus::Processing,
    );
    let wf = w.shipped();
    let run = w.file_arrives(&wf, "Haengt.wav");
    w.engine.tick().unwrap();
    w.clock.advance(MAX_WAIT_FOR_MEETING_MS + 1_000);
    let report = w.engine.tick().unwrap();
    assert!(
        matches!(report.outcomes[0].1, RunOutcome::Failed { .. }),
        "{report:?}"
    );
    assert_eq!(w.step_state(&run, "protokoll"), StepState::Failed);
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failed_or_stopped_meeting_is_a_permanent_failure_not_a_wait() {
    for status in [MeetingStatus::Failed, MeetingStatus::Cancelled] {
        let w = world_with(|_| MockReply::Status(500), status);
        let id = make_meeting(&w.services.store, "X", status);
        let direct = Direct::new(Some(&id));
        let err = direct
            .run(&w, &*action_of(&w, "meeting.minutes"), "R", "min", &json!({}))
            .unwrap_err();
        assert!(matches!(err, StepError::Permanent(_)), "{status:?}: {err:?}");
    }
}

#[test]
fn a_step_without_any_meeting_in_the_run_says_so() {
    let w = world();
    let direct = Direct::new(None);
    for id in ["meeting.minutes", "meeting.notes", "export.document"] {
        let err = direct
            .run(
                &w,
                &*action_of(&w, id),
                "R",
                "s",
                &json!({"target": "folder-protokolle", "format": "docx"}),
            )
            .unwrap_err();
        match err {
            StepError::Permanent(m) => assert!(m.contains("Besprechung"), "{id}: {m}"),
            other => panic!("{id}: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Idempotenz und Provenienz
// ---------------------------------------------------------------------------

#[test]
fn a_repeated_minutes_step_returns_the_document_it_made_before() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let action = action_of(&w, "meeting.minutes");
    let first = direct
        .run(&w, &*action, "R1", "protokoll", &json!({}))
        .unwrap();
    // Absturz vor dem Journal: dieselbe Kennung ruft den Baustein erneut.
    let second = direct
        .run(&w, &*action, "R1", "protokoll", &json!({}))
        .unwrap();
    assert_eq!(first.data["document_id"], second.data["document_id"]);
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 1, "kein zweiter Modellaufruf");
    let docs = w.services.store.get_documents(&id).unwrap();
    assert_eq!(docs.iter().filter(|d| d.kind == "minutes").count(), 1);
    // Ein anderer Lauf (neue Kennung) erzeugt bewusst eine neue Fassung.
    let third = direct
        .run(&w, &*action, "R2", "protokoll", &json!({}))
        .unwrap();
    assert_ne!(first.data["document_id"], third.data["document_id"]);

    // Provenienz: Akteur Ablauf, Kennung `<workflow>/<lauf>/<schritt>`, Quelle = Besprechung.
    let doc_id = first.data["document_id"].as_str().unwrap();
    let entries =
        crate::managers::provenance::list(&w.conn(), SubjectKind::Document, doc_id).unwrap();
    let mine = entries
        .iter()
        .find(|e| e.actor_ref.as_deref() == Some("wf-test/R1/protokoll"))
        .expect("Eintrag des Ablaufs");
    assert_eq!(
        mine.actor_kind,
        Some(crate::managers::provenance::ActorKind::Workflow)
    );
    assert_eq!(mine.operation, "minutes");
    assert!(mine.sources.iter().any(|s| s.kind == "meeting" && s.reference == id));
    assert!(mine
        .params_json
        .as_deref()
        .unwrap()
        .contains("R1:protokoll"));
}

#[test]
fn a_repeated_export_step_does_not_make_a_second_file() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    direct
        .run(&w, &*action_of(&w, "meeting.minutes"), "R1", "min", &json!({}))
        .unwrap();
    let export = action_of(&w, "export.document");
    let params = json!({"format": "md", "target": "folder-protokolle", "name": "Protokoll"});
    let first = direct.run(&w, &*export, "R1", "ablage", &params).unwrap();
    let second = direct.run(&w, &*export, "R1", "ablage", &params).unwrap();
    assert_eq!(first.data["path"], second.data["path"]);
    assert_eq!(w.files_in_target().len(), 1, "{:?}", w.files_in_target());
    // Ein anderer Schritt mit demselben Namen legt NIE etwas darueber, sondern daneben.
    let third = direct.run(&w, &*export, "R1", "ablage2", &params).unwrap();
    assert_eq!(third.data["file_name"], "Protokoll (2).md");
    assert_eq!(w.files_in_target().len(), 2);
}

// ---------------------------------------------------------------------------
// export.document: Inhalt, Formate, Fehler
// ---------------------------------------------------------------------------

#[test]
fn export_writes_markdown_with_the_protocol_and_the_meeting_title() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    direct
        .run(&w, &*action_of(&w, "meeting.minutes"), "R", "min", &json!({}))
        .unwrap();
    let out = direct
        .run(
            &w,
            &*action_of(&w, "export.document"),
            "R",
            "ab",
            &json!({"format": "md", "target": "folder-protokolle", "subfolder": "2026/Oktober"}),
        )
        .unwrap();
    assert_eq!(out.data["rel"], format!("2026/Oktober/{TITLE}.md"));
    let text = std::fs::read_to_string(out.data["path"].as_str().unwrap()).unwrap();
    assert!(text.starts_with(&format!("# {TITLE}")), "{text}");
    assert!(text.contains("Go-Live am 30. Oktober"));
    assert!(text.contains("Größe der Testgruppe"));
}

#[test]
fn export_of_a_missing_protocol_is_permanent_and_writes_nothing() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let err = direct
        .run(
            &w,
            &*action_of(&w, "export.document"),
            "R",
            "ab",
            &json!({"format": "docx", "target": "folder-protokolle"}),
        )
        .unwrap_err();
    match err {
        StepError::Permanent(m) => assert!(m.contains("noch kein"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(w.files_in_target().is_empty());
}

#[test]
fn export_refuses_a_target_that_is_not_a_folder_or_does_not_exist() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    {
        let conn = w.conn();
        let mut n = NewIntegration::new(crate::managers::integrations::model::Kind::Smtp, "Post");
        n.id = Some("smtp-1".into());
        crate::managers::integrations::store::create(&conn, &n, T0).unwrap();
    }
    let direct = Direct::new(Some(&id));
    let export = action_of(&w, "export.document");
    for (target, needle) in [("smtp-1", "weder ein Ordner"), ("folder-nirgends", "gibt es nicht")] {
        let err = direct
            .run(
                &w,
                &*export,
                "R",
                "ab",
                &json!({"format": "md", "target": target}),
            )
            .unwrap_err();
        match err {
            StepError::Permanent(m) => assert!(m.contains(needle), "{target}: {m}"),
            other => panic!("{target}: {other:?}"),
        }
    }
}

#[test]
fn a_target_folder_that_vanished_is_transient_and_an_escaping_subfolder_permanent() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    direct
        .run(&w, &*action_of(&w, "meeting.minutes"), "R", "min", &json!({}))
        .unwrap();
    let export = action_of(&w, "export.document");
    let escape = direct
        .run(
            &w,
            &*export,
            "R",
            "ab",
            &json!({"format": "md", "target": "folder-protokolle", "subfolder": "../draussen"}),
        )
        .unwrap_err();
    assert!(matches!(escape, StepError::Permanent(_)), "{escape:?}");
    assert!(w.out_dir.parent().unwrap().join("draussen").exists() == false);

    std::fs::remove_dir_all(&w.out_dir).unwrap();
    let gone = direct
        .run(
            &w,
            &*export,
            "R",
            "ab2",
            &json!({"format": "md", "target": "folder-protokolle"}),
        )
        .unwrap_err();
    assert!(matches!(gone, StepError::Transient(_)), "{gone:?}");
}

#[test]
fn export_of_notes_needs_notes_and_writes_them() {
    let w = world_with(
        |_| MockReply::Body(chat_body(&notes_reply())),
        MeetingStatus::Ready,
    );
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let export = action_of(&w, "export.document");
    let params = json!({"format": "md", "target": "folder-protokolle", "content": "notes"});
    assert!(matches!(
        direct.run(&w, &*export, "R", "ab0", &params).unwrap_err(),
        StepError::Permanent(_)
    ));
    let notes = direct
        .run(&w, &*action_of(&w, "meeting.notes"), "R", "n", &json!({}))
        .unwrap();
    assert!(notes.data["document_id"].is_string());
    let docs = w.services.store.get_documents(&id).unwrap();
    assert!(docs.iter().any(|d| d.kind == "enhanced_notes"));
    let out = direct.run(&w, &*export, "R", "ab", &params).unwrap();
    let text = std::fs::read_to_string(out.data["path"].as_str().unwrap()).unwrap();
    assert!(text.contains("Der Kunde will im Herbst starten."), "{text}");
    assert!(text.contains("Angebot senden"));
}

// ---------------------------------------------------------------------------
// meeting.minutes: Vorlage, Sprache, Fehler
// ---------------------------------------------------------------------------

#[test]
fn a_template_can_be_named_by_title_or_id_and_unknown_ones_are_refused() {
    let w = world();
    let store = &w.services.store;
    assert_eq!(resolve_template(store, None).unwrap(), None);
    assert_eq!(
        resolve_template(store, Some("auto")).unwrap().as_deref(),
        Some("auto")
    );
    assert_eq!(
        resolve_template(store, Some("builtin:allgemein"))
            .unwrap()
            .as_deref(),
        Some("builtin:allgemein")
    );
    // Der Schluessel ohne Praefix und der Titel gehen auch.
    assert_eq!(
        resolve_template(store, Some("Allgemein")).unwrap().as_deref(),
        Some("builtin:allgemein")
    );
    let err = resolve_template(store, Some("gibt-es-nicht")).unwrap_err();
    assert!(err.contains("gibt es nicht"), "{err}");
}

#[test]
fn an_unknown_template_or_language_is_permanent_and_never_reaches_the_model() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let action = action_of(&w, "meeting.minutes");
    for params in [
        json!({"template": "gibt-es-nicht"}),
        json!({"output_language": "Deutsch bitte"}),
    ] {
        let err = direct.run(&w, &*action, "R", "m", &params).unwrap_err();
        assert!(matches!(err, StepError::Permanent(_)), "{params}: {err:?}");
    }
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn the_output_language_reaches_the_document_metadata() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let out = direct
        .run(
            &w,
            &*action_of(&w, "meeting.minutes"),
            "R",
            "m",
            &json!({"output_language": "EN"}),
        )
        .unwrap();
    let doc_id = out.data["document_id"].as_str().unwrap();
    let meta = w
        .services
        .store
        .document_generation_metadata(doc_id)
        .unwrap()
        .unwrap();
    assert!(meta.contains("\"output_language\":\"en\""), "{meta}");
}

#[test]
fn model_trouble_maps_to_the_right_step_error_class() {
    let busy = |e: ServiceError| matches!(e, ServiceError::Busy { .. });
    assert!(busy(classify_generation_error("meeting_not_finished")));
    assert!(busy(classify_generation_error("minutes_busy")));
    assert!(busy(classify_generation_error("enhance_busy")));
    assert!(busy(classify_generation_error("memory_low")));
    assert!(busy(classify_generation_error("recording_active")));
    assert!(busy(classify_generation_error(
        "Zu wenig freier Arbeitsspeicher: 2.0 GB frei"
    )));
    for code in ["no_provider", "no_model", "no_transcript", "template_not_found", "meeting_not_found"] {
        assert!(
            matches!(classify_generation_error(code), ServiceError::Permanent(_)),
            "{code}"
        );
    }
    assert!(matches!(
        classify_generation_error("Kein LLM-Provider konfiguriert (Einstellungen → Nachbearbeitung)"),
        ServiceError::Permanent(_)
    ));
    assert_eq!(
        classify_generation_error("minutes_cancelled"),
        ServiceError::Cancelled
    );
    assert!(matches!(
        classify_generation_error("stopped"),
        ServiceError::Permanent(_)
    ));
    // Ein Modellfehler: nichts gespeichert, neuer Versuch sicher; der Text nennt nur den Code.
    match classify_generation_error("llm_failed: Zeitlimit überschritten") {
        ServiceError::Transient(m) => assert!(m.contains("llm_failed") && !m.contains("Zeitlimit")),
        other => panic!("{other:?}"),
    }
    // Und die Abbildung in die Fehlerklassen der Engine.
    assert!(matches!(
        ServiceError::Busy {
            retry_after_ms: 5,
            reason: "x".into()
        }
        .into_step_error(),
        StepError::Defer { retry_after_ms: 5, .. }
    ));
    assert!(matches!(
        ServiceError::Cancelled.into_step_error(),
        StepError::Transient(_)
    ));
}

#[test]
fn a_model_server_error_is_transient_and_leaves_no_document() {
    let w = world_with(|_| MockReply::Status(500), MeetingStatus::Ready);
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let err = direct
        .run(&w, &*action_of(&w, "meeting.minutes"), "R", "m", &json!({}))
        .unwrap_err();
    assert!(matches!(err, StepError::Transient(_)), "{err:?}");
    assert!(w.services.store.get_documents(&id).unwrap().is_empty());
}

#[test]
fn cancelling_the_run_stops_the_model_call() {
    // Der Server antwortet nie; der Abbruch beendet den Schritt trotzdem.
    let w = world_with(|_| MockReply::Hang, MeetingStatus::Ready);
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    direct.cancel.store(true, Ordering::SeqCst);
    let started = std::time::Instant::now();
    let err = direct
        .run(&w, &*action_of(&w, "meeting.minutes"), "R", "m", &json!({}))
        .unwrap_err();
    assert!(matches!(err, StepError::Transient(m) if m.contains("abgebrochen")));
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(w.services.store.get_documents(&id).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// text.summarize
// ---------------------------------------------------------------------------

#[test]
fn a_summary_of_the_protocol_comes_back_as_text_and_leaves_a_provenance_entry() {
    let w = world_with(
        |body| {
            if body.contains("Summarize the following text") {
                MockReply::Body(chat_body("Kurz gesagt: Der Go-Live ist auf Ende Oktober gelegt."))
            } else {
                MockReply::Body(chat_body(&minutes_reply()))
            }
        },
        MeetingStatus::Ready,
    );
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    // Ohne Protokoll: Permanent mit Hinweis.
    let sum = action_of(&w, "text.summarize");
    let err = direct
        .run(&w, &*sum, "R", "z0", &json!({"source": "minutes"}))
        .unwrap_err();
    assert!(matches!(err, StepError::Permanent(m) if m.contains("kein Protokoll")));
    direct
        .run(&w, &*action_of(&w, "meeting.minutes"), "R", "min", &json!({}))
        .unwrap();
    let out = direct
        .run(&w, &*sum, "R", "z", &json!({"source": "minutes", "style": "management"}))
        .unwrap();
    assert_eq!(
        out.data["text"],
        "Kurz gesagt: Der Go-Live ist auf Ende Oktober gelegt."
    );
    assert_eq!(out.data["truncated"], false);
    assert_eq!(out.data["meeting_id"], id);
    let entries = crate::managers::provenance::list(&w.conn(), SubjectKind::Summary, "R:z").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].actor_ref.as_deref(), Some("wf-test/R/z"));
    assert_eq!(out.sources.len(), 1);
}

#[test]
fn a_summary_of_a_given_text_or_the_transcript_works_and_a_wrong_style_is_refused() {
    let w = world_with(
        |_| MockReply::Body(chat_body("Zusammenfassung ohne Besprechung.")),
        MeetingStatus::Ready,
    );
    let sum = action_of(&w, "text.summarize");
    // Ein Text ohne Besprechung im Lauf.
    let direct = Direct::new(None);
    let out = direct
        .run(&w, &*sum, "R", "z", &json!({"source": "Ein langer Text über Größe und Änderung."}))
        .unwrap();
    assert_eq!(out.data["text"], "Zusammenfassung ohne Besprechung.");
    assert!(out.data.get("meeting_id").is_none());
    // Das Transkript der Besprechung.
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let with_meeting = Direct::new(Some(&id));
    let out = with_meeting
        .run(&w, &*sum, "R", "t", &json!({"source": "transcript"}))
        .unwrap();
    assert_eq!(out.data["meeting_id"], id);
    // Unbekannte Art: beim Speichern UND beim Lauf abgelehnt.
    let params = json!({"source": "minutes", "style": "poetisch"});
    assert!(sum.validate(params.as_object().unwrap()).is_err());
    assert!(matches!(
        with_meeting.run(&w, &*sum, "R", "x", &params).unwrap_err(),
        StepError::Permanent(_)
    ));
    // Eine Vorlage `{{...}}` wird erst beim Lauf geprueft.
    let dynamic = json!({"source": "minutes", "style": "{{vars.art}}"});
    assert!(sum.validate(dynamic.as_object().unwrap()).is_ok());
}

// ---------------------------------------------------------------------------
// tts.render
// ---------------------------------------------------------------------------

#[test]
fn speech_goes_into_the_meeting_audio_folder_once_per_step() {
    let w = world();
    let id = make_meeting(&w.services.store, TITLE, MeetingStatus::Ready);
    let direct = Direct::new(Some(&id));
    let tts = action_of(&w, "tts.render");
    let params = json!({"text": "Das Protokoll liegt im Ordner."});
    let first = direct.run(&w, &*tts, "R1", "vorlesen", &params).unwrap();
    let path = PathBuf::from(first.data["path"].as_str().unwrap());
    assert!(path.starts_with(w.services.audio_dir.join(&id)), "{path:?}");
    assert!(path.file_name().unwrap().to_string_lossy().contains("R1-vorlesen"));
    assert!(path.exists());
    assert_eq!(w.services.speech_calls.load(Ordering::SeqCst), 1);
    // Wiederholung nach einem Absturz: dieselbe Datei, kein zweiter Aufruf der Sprachausgabe.
    let second = direct.run(&w, &*tts, "R1", "vorlesen", &params).unwrap();
    assert_eq!(second.data["path"], first.data["path"]);
    assert_eq!(w.services.speech_calls.load(Ordering::SeqCst), 1);
    let entries =
        crate::managers::provenance::list(&w.conn(), SubjectKind::TtsAudio, "R1:vorlesen").unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn speech_refuses_a_voice_an_empty_text_and_a_huge_text() {
    let w = world();
    let direct = Direct::new(None);
    let tts = action_of(&w, "tts.render");
    assert!(tts
        .validate(json!({"text": "x", "voice": "anna"}).as_object().unwrap())
        .is_err());
    assert!(tts
        .validate(json!({"text": "x", "voice": ""}).as_object().unwrap())
        .is_ok());
    for params in [
        json!({"text": "x", "voice": "anna"}),
        json!({"text": "   "}),
        json!({"text": "a".repeat(MAX_TTS_CHARS + 1)}),
    ] {
        let err = direct.run(&w, &*tts, "R", "v", &params).unwrap_err();
        assert!(matches!(err, StepError::Permanent(_)), "{err:?}");
    }
    assert_eq!(w.services.speech_calls.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// notify.local
// ---------------------------------------------------------------------------

#[test]
fn a_notification_is_cleaned_shortened_and_forwarded() {
    let w = world();
    let direct = Direct::new(None);
    let notify = action_of(&w, "notify.local");
    let out = direct
        .run(
            &w,
            &*notify,
            "R",
            "n",
            &json!({"title": "Protokoll\n fertig\u{7}", "body": "x".repeat(500)}),
        )
        .unwrap();
    assert_eq!(out.data["shown"], true);
    let sent = w.services.notified.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "Protokoll fertig");
    assert_eq!(sent[0].1.chars().count(), MAX_NOTIFY_BODY_CHARS);
    assert!(sent[0].1.ends_with('…'));
    assert!(matches!(
        direct.run(&w, &*notify, "R", "n2", &json!({"title": "  "})).unwrap_err(),
        StepError::Permanent(_)
    ));
}

// ---------------------------------------------------------------------------
// Katalog, Schweres, Dienste fehlen, Vorlage
// ---------------------------------------------------------------------------

#[test]
fn heavy_steps_declare_their_need_and_a_remote_model_needs_no_memory() {
    let w = world();
    let params = json!({});
    let heavy = |id: &str| action_of(&w, id).heavy(&params);
    for id in ["meeting.notes", "meeting.minutes", "text.summarize"] {
        assert_eq!(
            heavy(id),
            Some(HeavyNeed {
                ram_mb: 6_144,
                label: "Sprachmodell"
            }),
            "{id}"
        );
    }
    assert_eq!(heavy("tts.render").unwrap().ram_mb, 2_048);
    assert_eq!(heavy("export.document"), None);
    assert_eq!(heavy("notify.local"), None);
    // `wait` ist nicht schwer (der Lauf parkt, er belegt keinen Platz).
    assert_eq!(heavy("wait"), None);

    let remote = Arc::new(TestServices {
        store: w.services.store.clone(),
        settings: w.services.settings.clone(),
        audio_dir: temp_dir(),
        notified: Mutex::new(Vec::new()),
        speech_calls: AtomicUsize::new(0),
        local: false,
    });
    let a = MeetingDocAction::new(DocKind::Minutes, remote);
    assert_eq!(a.heavy(&params).unwrap().ram_mb, 0);
}

#[test]
fn every_block_has_a_catalog_entry_and_the_right_effect_and_right() {
    let w = world();
    for (id, effect) in [
        ("meeting.notes", EffectKind::Idempotent),
        ("meeting.minutes", EffectKind::Idempotent),
        ("text.summarize", EffectKind::Idempotent),
        ("export.document", EffectKind::Idempotent),
        ("tts.render", EffectKind::Idempotent),
        ("notify.local", EffectKind::Idempotent),
    ] {
        let a = action_of(&w, id);
        assert_eq!(a.effect(), effect, "{id}");
        assert_eq!(a.effect(), catalog::action_spec(id).unwrap().effect, "{id}");
    }
    // Nur der Export braucht ein Recht: `files.write` auf das Ziel.
    let needs = action_of(&w, "export.document")
        .needs(&json!({"target": "folder-protokolle", "format": "docx", "name": "A"}))
        .unwrap()
        .unwrap();
    assert_eq!(needs.integration_id, "folder-protokolle");
    assert_eq!(needs.capability, Capability::FilesWrite);
    assert_eq!(needs.target.as_deref(), Some("A"));
    for id in ["meeting.notes", "meeting.minutes", "text.summarize", "tts.render", "notify.local"] {
        assert_eq!(action_of(&w, id).needs(&json!({})).unwrap(), None, "{id}");
    }
}

#[test]
fn without_the_app_the_blocks_report_that_they_are_not_built_in() {
    let w = world();
    let bare = actions(Arc::new(UnavailableServices));
    let direct = Direct::new(Some("X"));
    for a in bare {
        let err = direct
            .run(
                &w,
                &*a,
                "R",
                "s",
                &json!({"title": "t", "text": "t", "source": "transcript", "format": "md", "target": "folder-protokolle"}),
            )
            .unwrap_err();
        assert!(matches!(err, StepError::NotAvailable(_)), "{}: {err:?}", a.id());
    }
}

#[test]
fn the_shipped_folder_template_is_valid_and_the_dry_run_names_every_new_block() {
    let text = templates::FOLDER_TO_WORD;
    let def = crate::managers::workflows::validate::parse_definition_str(text).unwrap();
    assert_eq!(def.trigger.kind, "folder.file_added");
    let actions: Vec<&str> = def.steps.iter().map(|s| s.action.as_str()).collect();
    assert_eq!(actions, ["meeting.import", "meeting.minutes", "export.document"]);
    assert!(templates::all()
        .iter()
        .any(|(id, _)| *id == "eingangsordner-word"));

    let w = world();
    let result = cli::dry_run(&w.conn(), text);
    assert_eq!(result.payload["valid"], true, "{}", result.payload);
    let table = cli::format_text(&result);
    assert!(table.contains("Protokoll zur Besprechung erzeugen"), "{table}");
    assert!(table.contains("als docx in folder-protokolle ablegen"), "{table}");
}

#[test]
fn the_dry_run_shows_the_effect_text_of_all_six_blocks() {
    let w = world();
    let def = json!({
        "schema": "lva-workflow@1",
        "name": "Alle Bausteine",
        "trigger": {"type": "manual"},
        "steps": [
            step("n", "meeting.notes", json!({"template": "Allgemein"})),
            step("m", "meeting.minutes", json!({"output_language": "en"})),
            step("z", "text.summarize", json!({"source": "minutes", "style": "kurz"})),
            step("e", "export.document", json!({"format": "pdf", "target": "folder-protokolle", "content": "all"})),
            step("t", "tts.render", json!({"text": "{{steps.z.text}}"})),
            step("b", "notify.local", json!({"title": "Fertig"})),
            step("w", "wait", json!({"minutes": 2}))
        ]
    });
    let result = cli::dry_run(&w.conn(), &def.to_string());
    assert_eq!(result.payload["valid"], true, "{}", result.payload);
    let steps = result.payload["steps"].as_array().unwrap();
    let effect = |i: usize| steps[i]["effect"].as_str().unwrap().to_string();
    assert!(effect(0).contains("KI-Notizen"), "{}", effect(0));
    assert!(effect(1).contains("Protokoll"), "{}", effect(1));
    assert!(effect(2).contains("Zusammenfassung"), "{}", effect(2));
    assert!(effect(3).contains("pdf"), "{}", effect(3));
    assert!(effect(5).contains("Mitteilung"), "{}", effect(5));
    assert!(effect(6).contains("2 Minuten"), "{}", effect(6));
    // Nur der Export hat ein Rechte-Ergebnis; geschrieben wird im Trockenlauf nichts.
    assert!(w.files_in_target().is_empty());
    assert_eq!(w.llm_calls.load(Ordering::SeqCst), 0);
}
