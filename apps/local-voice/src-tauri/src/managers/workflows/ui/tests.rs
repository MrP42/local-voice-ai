use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::*;
use crate::managers::workflows::action::{EffectKind, StepError};
use crate::managers::workflows::model::{RunState, StepState};
use crate::managers::workflows::test_support::{
    def, engine, note, register, set_grant, step, FakeClock, Fx, Scripted,
};

struct World {
    fx: Fx,
    engine: Engine,
    notify: Arc<Scripted>,
}

fn world() -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine(&fx, &clock);
    let notify = Scripted::new("notify.local", EffectKind::Idempotent);
    engine.register_action(notify.clone());
    World { fx, engine, notify }
}

fn text(v: &Value) -> String {
    serde_json::to_string(v).unwrap()
}

fn plain() -> String {
    text(&def(vec![note("a"), note("b")]))
}

// ---------------------------------------------------------------------------
// Katalog und Vorlagen
// ---------------------------------------------------------------------------

#[test]
fn the_catalog_view_describes_every_trigger_and_action_with_its_fields() {
    let c = catalog_view();
    assert_eq!(c.schema, "lva-workflow@1");
    assert_eq!(c.max_steps as usize, MAX_STEPS);
    assert_eq!(c.triggers.len(), catalog::triggers().len());
    assert_eq!(c.actions.len(), catalog::actions().len());

    let mail = c.actions.iter().find(|a| a.id == "mail.send").unwrap();
    assert_eq!(mail.effect, "external");
    assert_eq!(mail.capability.as_deref(), Some("mail.send"));
    let to = mail.fields.iter().find(|f| f.name == "to").unwrap();
    assert_eq!(to.kind, "choice");
    assert!(to.literal, "Empfaengerregel ist ein fester Wert");
    assert_eq!(to.options, vec!["me", "participants", "all", "list"]);
    // `via` traegt die noetige Faehigkeit fuer die Auswahl der Integration.
    let via = mail.fields.iter().find(|f| f.name == "via").unwrap();
    assert_eq!(via.kind, "id");
    assert_eq!(via.capability.as_deref(), Some("mail.send"));
    // Ein Feld ohne Integration traegt keine.
    let subject = mail.fields.iter().find(|f| f.name == "subject").unwrap();
    assert_eq!(subject.capability, None);

    let lead = c
        .triggers
        .iter()
        .find(|t| t.id == "calendar.event_starting")
        .unwrap()
        .fields
        .iter()
        .find(|f| f.name == "lead_min")
        .unwrap();
    assert_eq!(
        (lead.kind.as_str(), lead.min, lead.max),
        ("int", Some(0), Some(120))
    );

    let heavy = c
        .actions
        .iter()
        .find(|a| a.id == "meeting.minutes")
        .unwrap();
    assert_eq!(heavy.heavy_label.as_deref(), Some("Sprachmodell"));
    let manual = c.triggers.iter().find(|t| t.id == "manual").unwrap();
    assert!(!manual.automatic);
    let folder = c
        .triggers
        .iter()
        .find(|t| t.id == "folder.file_added")
        .unwrap();
    assert!(folder.automatic);
    assert_eq!(
        folder
            .fields
            .iter()
            .find(|f| f.name == "integration")
            .unwrap()
            .capability
            .as_deref(),
        Some("files.read")
    );
}

#[test]
fn every_shipped_template_is_offered_and_can_be_saved_as_it_is() {
    let w = world();
    let list = template_list();
    assert_eq!(list.len(), templates::all().len());
    assert!(!list.is_empty());
    for t in &list {
        assert!(!t.name.is_empty(), "{}", t.id);
        assert!(!t.description.is_empty(), "{}", t.id);
        let r = save(&w.engine, None, &t.definition_json).unwrap();
        assert!(r.issues.is_empty(), "{}: {:?}", t.id, r.issues);
        let item = r.workflow.unwrap();
        assert_eq!(item.name, t.name);
        // Ein neuer Ablauf ist ausgeschaltet und im Trockenlauf.
        assert!(!item.enabled && item.dry_run, "{}", t.id);
    }
}

// ---------------------------------------------------------------------------
// Pruefung und Speichern
// ---------------------------------------------------------------------------

#[test]
fn validation_reports_every_finding_with_its_json_pointer() {
    let w = world();
    let bad = json!({
        "schema": "lva-workflow@1",
        "name": "",
        "trigger": {"type": "manual"},
        "steps": [
            {"id": "a", "action": "gibt.es.nicht"},
            {"id": "b", "action": "wait", "params": {"minutes": 0}},
            {"id": "b", "action": "notify.local", "params": {"title": "x"}, "when": "{{"},
        ]
    });
    let issues = validate_text(&w.engine, &text(&bad));
    let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"/name"), "{paths:?}");
    assert!(paths.contains(&"/steps/0/action"), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.starts_with("/steps/1/params")),
        "{paths:?}"
    );
    assert!(issues.iter().all(|i| !i.message.is_empty()));
    // Nichts davon ist gespeichert worden.
    assert!(list(&w.engine).unwrap().is_empty());
}

#[test]
fn text_that_is_not_json_is_a_finding_not_an_error() {
    let w = world();
    let issues = validate_text(&w.engine, "{ nicht json");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "");
    assert!(issues[0].message.contains("Kein gültiges JSON"));
    let r = save(&w.engine, None, "{ nicht json").unwrap();
    assert!(r.workflow.is_none());
    assert_eq!(r.issues.len(), 1);
    assert!(list(&w.engine).unwrap().is_empty());
}

#[test]
fn an_oversized_definition_is_refused_before_it_is_parsed() {
    let w = world();
    let big = format!("{{\"x\": \"{}\"}}", "a".repeat(MAX_IMPORT_BYTES));
    let issues = validate_text(&w.engine, &big);
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("zu groß"));
}

#[test]
fn the_checks_of_the_real_actions_and_triggers_show_up_in_the_editor_too() {
    let w = world();
    // Zeitplan: Uhrzeit fehlerhaft (Pruefung in `trigger::check_definition`).
    let d = json!({
        "schema": "lva-workflow@1",
        "name": "Zeitplan",
        "trigger": {"type": "schedule", "every": "daily", "at": "25:99"},
        "steps": [note("a")]
    });
    let issues = validate_text(&w.engine, &text(&d));
    assert!(!issues.is_empty());
    assert!(
        issues.iter().any(|i| i.path.starts_with("/trigger")),
        "{issues:?}"
    );
}

#[test]
fn saving_creates_a_disabled_dry_run_workflow_and_replacing_falls_back_to_dry_run() {
    let w = world();
    let r = save(&w.engine, None, &plain()).unwrap();
    let item = r.workflow.unwrap();
    assert!(!item.enabled && item.dry_run);
    assert_eq!(item.trigger_kind, "manual");
    assert_eq!(item.step_count, 2);
    assert!(item.last_run.is_none());
    assert_eq!(item.open_runs, 0);

    // Scharf schalten und einschalten, dann die Definition aendern: zurueck in den Trockenlauf.
    let armed = set_armed(&w.engine, &item.id, true).unwrap();
    assert!(!armed.dry_run);
    let on = set_enabled(&w.engine, &item.id, true).unwrap();
    assert!(on.enabled);
    let changed = text(&def(vec![note("a")]));
    let r2 = save(&w.engine, Some(&item.id), &changed).unwrap();
    let again = r2.workflow.unwrap();
    assert_eq!(again.id, item.id);
    assert_eq!(again.step_count, 1);
    assert!(
        again.dry_run,
        "eine geaenderte Definition ist nicht mehr scharf"
    );
    assert_eq!(list(&w.engine).unwrap().len(), 1);
}

#[test]
fn an_invalid_replacement_leaves_the_stored_workflow_untouched() {
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let before = get(&w.engine, &id).unwrap();
    let bad = text(&def(vec![step("a", "gibt.es.nicht", json!({}))]));
    let r = save(&w.engine, Some(&id), &bad).unwrap();
    assert!(r.workflow.is_none());
    assert!(r.issues.iter().any(|i| i.path == "/steps/0/action"));
    assert_eq!(get(&w.engine, &id).unwrap(), before);
}

#[test]
fn deleting_an_unknown_workflow_is_an_error_text() {
    let w = world();
    let e = delete(&w.engine, "gibt-es-nicht").unwrap_err();
    assert!(error_text(&e).contains("gibt-es-nicht"));
}

// ---------------------------------------------------------------------------
// Export und Import (AK9: Rundlauf)
// ---------------------------------------------------------------------------

#[test]
fn export_then_import_gives_the_same_definition() {
    let w = world();
    for t in template_list() {
        let first = save(&w.engine, None, &t.definition_json)
            .unwrap()
            .workflow
            .unwrap();
        let exported = export(&w.engine, &first.id).unwrap();
        assert!(exported.ends_with('\n'));
        let second = import(&w.engine, &exported).unwrap().workflow.unwrap();
        assert_ne!(
            second.id, first.id,
            "{}: Import legt einen neuen Ablauf an",
            t.id
        );
        assert_eq!(
            second.definition_json, first.definition_json,
            "{}: Rundlauf veraendert die Definition",
            t.id
        );
        // Und ein zweiter Export ist wortgleich.
        assert_eq!(export(&w.engine, &second.id).unwrap(), exported);
        // Der Import ist ausgeschaltet und im Trockenlauf, auch wenn das Original scharf war.
        assert!(!second.enabled && second.dry_run);
    }
}

#[test]
fn an_import_is_checked_like_a_saved_workflow() {
    let w = world();
    let r = import(&w.engine, r#"{"schema":"lva-workflow@1","name":"x","trigger":{"type":"manual"},"steps":[{"id":"a","action":"mail.send","params":{"via":"m","to":"{{trigger.x}}","subject":"s"}}]}"#).unwrap();
    assert!(r.workflow.is_none());
    assert!(
        r.issues
            .iter()
            .any(|i| i.path.contains("/steps/0/params/to")),
        "Empfaenger darf nie aus Daten entstehen: {:?}",
        r.issues
    );
    assert!(list(&w.engine).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Trockenlauf
// ---------------------------------------------------------------------------

#[test]
fn the_plan_of_an_unsaved_definition_shows_every_step_and_writes_nothing() {
    let w = world();
    let conn = w.fx.conn();
    let p: Value = serde_json::from_str(&plan_text(&w.engine, &conn, &plain(), None)).unwrap();
    assert_eq!(p["valid"], true);
    assert_eq!(p["schema"], "lva-workflow-plan@1");
    assert_eq!(p["writes"], "nothing");
    assert_eq!(p["trigger_sample"], true);
    assert_eq!(p["steps"].as_array().unwrap().len(), 2);
    assert_eq!(p["summary"]["steps"], 2);
    // Es entstand weder ein Ablauf noch ein Lauf.
    assert!(list(&w.engine).unwrap().is_empty());
    assert!(runs(&w.engine, None, false, 10).unwrap().is_empty());
}

#[test]
fn the_plan_shows_the_permission_result_of_each_step() {
    let w = world();
    let conn = w.fx.conn();
    register(
        &conn,
        crate::managers::integrations::model::Kind::Folder,
        "ordner-aus",
    );
    let d = text(&def(vec![step(
        "doc",
        "export.document",
        json!({"format": "docx", "target": "ordner-aus"}),
    )]));
    let p: Value = serde_json::from_str(&plan_text(&w.engine, &conn, &d, None)).unwrap();
    let perm = &p["steps"][0]["permission"];
    assert_eq!(perm["required"], true);
    assert_eq!(perm["integration"], "ordner-aus");
    // Ohne Recht fuer „Ablauf“ gilt die Vorgabe des Registers (fragen); wichtig: ein Ergebnis.
    assert!(perm["result"].is_string(), "{perm}");

    set_grant(
        &conn,
        "ordner-aus",
        crate::managers::integrations::model::Capability::FilesWrite,
        crate::managers::integrations::model::GrantMode::Off,
    );
    let p: Value = serde_json::from_str(&plan_text(&w.engine, &conn, &d, None)).unwrap();
    assert_eq!(p["steps"][0]["permission"]["result"], "denied");
    assert_eq!(p["summary"]["denied"], 1);
    assert_eq!(p["summary"]["would_run_without_intervention"], false);
}

#[test]
fn the_plan_of_an_invalid_definition_lists_the_findings_instead() {
    let w = world();
    let conn = w.fx.conn();
    let bad = text(&def(vec![step("a", "gibt.es.nicht", json!({}))]));
    let p: Value = serde_json::from_str(&plan_text(&w.engine, &conn, &bad, None)).unwrap();
    assert_eq!(p["valid"], false);
    assert!(p["steps"].is_null());
    assert_eq!(p["issues"][0]["path"], "/steps/0/action");
}

// ---------------------------------------------------------------------------
// Start, Laeufe, Abbrechen, Wiederholen
// ---------------------------------------------------------------------------

#[test]
fn a_manual_dry_run_plans_and_shows_up_in_the_run_log_with_its_steps() {
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let started = start(&w.engine, &id, Map::new(), true).unwrap();
    assert!(started.created && started.dry_run);

    let queued = runs(&w.engine, Some(&id), false, 10).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].state, RunState::Queued);
    assert_eq!(queued[0].origin, "manual");
    assert!(queued[0].dry_run);
    assert_eq!(get(&w.engine, &id).unwrap().open_runs, 1);

    w.engine.run_next().unwrap();
    let d = run_detail(&w.engine, &started.run_id).unwrap();
    assert_eq!(d.run.state, RunState::Done);
    assert!(d.run.dry_run);
    assert_eq!(d.steps.len(), 2);
    assert!(d.steps.iter().all(|s| s.state == StepState::Planned));
    assert_eq!(
        d.steps[0].action_title.as_deref(),
        Some("Windows-Mitteilung")
    );
    assert!(!d.can_retry && !d.can_cancel);
    // Ein Trockenlauf hat den Baustein nie gerufen.
    assert_eq!(w.notify.call_count(), 0);
    assert_eq!(get(&w.engine, &id).unwrap().open_runs, 0);
    assert_eq!(
        get(&w.engine, &id).unwrap().last_run.unwrap().state,
        RunState::Done
    );
}

#[test]
fn a_real_start_of_a_workflow_that_is_not_armed_only_plans_and_one_that_is_off_is_refused() {
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    // Nicht scharf: auch „richtig starten“ plant nur (nie scharfer als der Ablauf).
    let started = start(&w.engine, &id, Map::new(), false).unwrap();
    assert!(started.dry_run);
    // Scharf, aber ausgeschaltet: ein echter Lauf wird mit einem Satz abgewiesen.
    set_armed(&w.engine, &id, true).unwrap();
    let e = start(&w.engine, &id, Map::new(), false).unwrap_err();
    assert!(matches!(e, WorkflowError::Disabled(_)), "{e:?}");
    assert!(error_text(&e).contains("ausgeschaltet"));
    assert_eq!(runs(&w.engine, None, false, 10).unwrap().len(), 1);
}

#[test]
fn an_armed_workflow_runs_for_real_and_a_failure_can_be_retried() {
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    set_armed(&w.engine, &id, true).unwrap();
    set_enabled(&w.engine, &id, true).unwrap();
    w.notify
        .push(Err(StepError::Permanent("Nicht erreichbar".into())));

    let started = start(&w.engine, &id, Map::new(), false).unwrap();
    assert!(!started.dry_run);
    w.engine.run_next().unwrap();

    let d = run_detail(&w.engine, &started.run_id).unwrap();
    assert_eq!(d.run.state, RunState::Failed);
    assert_eq!(d.run.error_code.as_deref(), Some("permanent"));
    assert!(d.can_retry && !d.retry_needs_confirmation && !d.can_cancel);
    let failed = d
        .steps
        .iter()
        .find(|s| s.state == StepState::Failed)
        .unwrap();
    assert_eq!(failed.step_id, "a");
    assert!(failed
        .error
        .as_deref()
        .unwrap()
        .contains("Nicht erreichbar"));

    retry_run(&w.engine, &started.run_id, false).unwrap();
    assert_eq!(
        run_detail(&w.engine, &started.run_id).unwrap().run.state,
        RunState::Queued
    );
    w.engine.run_next().unwrap();
    let d = run_detail(&w.engine, &started.run_id).unwrap();
    assert_eq!(d.run.state, RunState::Done);
    assert_eq!(w.notify.call_count(), 3, "a (scheitert), a (wiederholt), b");
    // Wiederholt ist wiederholt: nur gescheiterte Laeufe gehen.
    let e = retry_run(&w.engine, &started.run_id, false).unwrap_err();
    assert!(error_text(&e).contains("gescheiterte"));
}

#[test]
fn an_unclear_effect_needs_the_explicit_confirmation_to_retry() {
    let w = world();
    // Ein Baustein mit Aussenwirkung: ein unklarer Ausgang wird nie von selbst wiederholt.
    let external = Scripted::new("notify.local", EffectKind::External);
    w.engine.register_action(external.clone());
    external.push(Err(StepError::Unknown("Zeitüberschreitung".into())));
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    set_armed(&w.engine, &id, true).unwrap();
    set_enabled(&w.engine, &id, true).unwrap();
    let started = start(&w.engine, &id, Map::new(), false).unwrap();
    w.engine.run_next().unwrap();
    let d = run_detail(&w.engine, &started.run_id).unwrap();
    assert_eq!(d.run.state, RunState::Failed);
    assert_eq!(d.run.error_code.as_deref(), Some("effect_uncertain"));
    assert!(d.can_retry && d.retry_needs_confirmation);
    let e = retry_run(&w.engine, &started.run_id, false).unwrap_err();
    assert!(error_text(&e).contains("bestätigen"), "{}", error_text(&e));
    assert_eq!(
        run_detail(&w.engine, &started.run_id).unwrap().run.state,
        RunState::Failed,
        "ohne Bestätigung bleibt der Lauf stehen"
    );
    retry_run(&w.engine, &started.run_id, true).unwrap();
    assert_eq!(
        run_detail(&w.engine, &started.run_id).unwrap().run.state,
        RunState::Queued
    );
}

#[test]
fn a_waiting_run_can_be_cancelled_and_a_finished_one_cannot() {
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let started = start(&w.engine, &id, Map::new(), true).unwrap();
    let before = run_detail(&w.engine, &started.run_id).unwrap();
    assert!(before.can_cancel && !before.can_retry);
    assert!(
        cancel_run(&w.engine, &started.run_id).unwrap(),
        "sofort abgebrochen"
    );
    let d = run_detail(&w.engine, &started.run_id).unwrap();
    assert_eq!(d.run.state, RunState::Cancelled);
    assert!(!d.can_cancel);
    let e = cancel_run(&w.engine, &started.run_id).unwrap_err();
    assert!(error_text(&e).contains("beendet"));
}

#[test]
fn the_run_list_filters_by_workflow_and_open_runs_and_is_capped() {
    let w = world();
    let a = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let b = save(&w.engine, None, &text(&def(vec![note("x")])))
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let r1 = start(&w.engine, &a, Map::new(), true).unwrap();
    start(&w.engine, &a, Map::new(), true).unwrap();
    start(&w.engine, &b, Map::new(), true).unwrap();
    assert_eq!(runs(&w.engine, None, false, 50).unwrap().len(), 3);
    assert_eq!(runs(&w.engine, Some(&a), false, 50).unwrap().len(), 2);
    assert_eq!(runs(&w.engine, Some(&b), false, 50).unwrap().len(), 1);
    assert_eq!(runs(&w.engine, None, false, 2).unwrap().len(), 2);
    cancel_run(&w.engine, &r1.run_id).unwrap();
    assert_eq!(
        runs(&w.engine, None, true, 50).unwrap().len(),
        2,
        "nur offene"
    );
    // Die Grenze gilt auch bei absurden Werten.
    assert_eq!(runs(&w.engine, None, false, i64::MAX).unwrap().len(), 3);
    assert_eq!(runs(&w.engine, None, false, -5).unwrap().len(), 1);
}

#[test]
fn an_unknown_run_is_an_error_text() {
    let w = world();
    let e = run_detail(&w.engine, "gibt-es-nicht").unwrap_err();
    assert!(error_text(&e).contains("gibt-es-nicht"));
}

// ---------------------------------------------------------------------------
// Stand der Ausloeser
// ---------------------------------------------------------------------------

#[test]
fn the_status_names_the_workflows_behind_cloud_files_and_channels() {
    use crate::managers::workflows::trigger::folder::CloudFile;
    use crate::managers::workflows::trigger::youtube_channel::ChannelStatus;
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let s = status(
        &w.engine,
        vec![CloudFile {
            workflow_id: id.clone(),
            name: "Aufnahme.wav".into(),
        }],
        vec![ChannelStatus {
            workflow_id: "weg".into(),
            channel_id: "UC0123456789012345678901".into(),
            last_ok_ms: Some(5),
            failures: 4,
            outage: true,
            last_error: Some("404".into()),
            next_fetch_ms: 9,
        }],
    );
    assert_eq!(s.cloud_only.len(), 1);
    assert_eq!(s.cloud_only[0].workflow_name, "Testablauf");
    assert_eq!(s.cloud_only[0].name, "Aufnahme.wav");
    assert_eq!(s.channels.len(), 1);
    assert!(s.channels[0].outage);
    assert_eq!(s.channels[0].failures, 4);
    // Ein geloeschter Ablauf erscheint mit seiner Kennung statt zu fehlen.
    assert_eq!(s.channels[0].workflow_name, "weg");
}

#[test]
fn files_for_export_and_import_are_written_and_read_with_a_size_cap() {
    let w = world();
    let id = save(&w.engine, None, &plain())
        .unwrap()
        .workflow
        .unwrap()
        .id;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ablauf.json");
    export_to_file(&w.engine, &id, &path).unwrap();
    let text = read_import_file(&path).unwrap();
    assert_eq!(text, export(&w.engine, &id).unwrap());
    let again = import(&w.engine, &text).unwrap().workflow.unwrap();
    assert_eq!(
        again.definition_json,
        get(&w.engine, &id).unwrap().definition_json
    );

    // Zu gross, kein UTF-8, fehlt.
    let big = dir.path().join("gross.json");
    std::fs::write(&big, vec![b'a'; MAX_IMPORT_BYTES + 1]).unwrap();
    assert!(read_import_file(&big).unwrap_err().contains("zu groß"));
    let bin = dir.path().join("bin.json");
    std::fs::write(&bin, [0xff, 0xfe, 0x00]).unwrap();
    assert!(read_import_file(&bin).unwrap_err().contains("UTF-8"));
    assert!(read_import_file(&dir.path().join("fehlt.json"))
        .unwrap_err()
        .contains("öffnen"));
    // Ein Export in einen nicht beschreibbaren Pfad ist ein Text, kein Absturz.
    let bad = dir.path().join("kein-ordner").join("x.json");
    assert!(export_to_file(&w.engine, &id, &bad)
        .unwrap_err()
        .contains("schreiben"));
}
