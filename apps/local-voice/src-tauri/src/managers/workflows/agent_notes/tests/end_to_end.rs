//! Ende zu Ende mit dem echten `agent.extract` (llama-server-Attrappe als Modell): die Vorlage
//! „Besprechung fertig -> extrahieren -> Vault-Notiz + Frist-Erinnerungen“ (AK6, AK7).

use super::*;
use crate::agent::test_support::{mock, ok, Mock};
use crate::managers::workflows::agent_actions;
use crate::managers::workflows::engine::RunOutcome::{Done, Parked};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../agent/fixtures/jourfixe-nordlicht.json"
    ))
    .unwrap()
}

/// Die Welt mit dem echten Extraktionsbaustein hinter der Modell-Attrappe.
fn world(m: &Mock) -> World {
    let w = World::new();
    *w.svc.target.lock().unwrap() = Some(Target::Endpoint {
        base_url: m.base_url.clone(),
        model: "llm-test".into(),
        context_tokens: 8192,
    });
    agent_actions::install(&w.engine, w.svc.clone());
    w
}

fn template(w: &World) -> String {
    let text = crate::managers::workflows::templates::BESPRECHUNG_ERGEBNIS;
    let definition: Value = serde_json::from_str(text).unwrap();
    armed_workflow(&w.engine, &definition)
}

fn fire(w: &World, wf: &str, n: u32) -> String {
    w.engine
        .enqueue(&EnqueueRequest {
            workflow_id: wf.to_string(),
            trigger_key: format!("meeting:{}:{n}", w.meeting_id),
            origin: Origin::Trigger,
            trigger: json!({ "meeting_id": w.meeting_id, "stage": "transcript", "title": TITLE, "meeting": {"id": w.meeting_id, "title": TITLE} }),
            vars: serde_json::Map::new(),
            force_dry_run: false,
        })
        .unwrap()
        .run_id
}

#[test]
fn ak6_meeting_finished_extract_vault_note_and_reminders_with_the_shipped_template() {
    let reply = fixture()["responses"][0].to_string();
    let m = block(mock(vec![ok(&reply)]));
    let w = world(&m);
    w.grant("vault-1", Capability::VaultWrite, GrantMode::Allow);
    let wf = template(&w);

    let run = fire(&w, &wf, 1);
    // Notiz geschrieben, der Lauf wartet auf die erste Erinnerung (14.10. 09:00).
    assert_eq!(
        w.tick(),
        vec![(
            run.clone(),
            Parked {
                reason: "defer".into()
            }
        )]
    );
    assert_eq!(m.count(), 1);
    assert_eq!(w.state(&run, "extract"), StepState::Done);
    assert_eq!(w.state(&run, "note"), StepState::Done);
    assert_eq!(
        w.state(&run, "kalender"),
        StepState::Skipped,
        "Kalender nur auf Wunsch (Variable)"
    );
    assert_eq!(w.state(&run, "erinnerung"), StepState::Waiting);

    let notes = w.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    let note = w.read_note(&notes[0]);
    let (f, body) = front(&note);
    assert_ai_os_frontmatter(&f);
    assert_eq!(f["data_class"], "confidential");
    assert_eq!(f["lva_meeting_id"], format!("\"{}\"", w.meeting_id));
    assert!(
        body.contains("- **15.10.2026** – Die Abnahme findet statt."),
        "{body}"
    );
    assert!(
        body.contains("- **31.10.2026** – Die Septemberrechnung muss raus."),
        "{body}"
    );
    assert!(
        body.contains("- Das Wochenmeeting findet dauerhaft dienstags um 10 Uhr statt."),
        "{body}"
    );
    assert!(
        body.contains("## To-dos") && body.contains("zuständig: Frau Berg"),
        "{body}"
    );
    assert!(body.contains("Modell llm-test (lokal)"), "{body}");
    // Das falsche Datum des Modells wurde im Code ersetzt (Freitag nächster Woche = 9.10.).
    assert!(body.contains("fällig: Freitag, 09.10.2026"), "{body}");

    // Zweiter Lauf auf derselben Besprechung: keine Dublette, dieselbe Datei.
    let second = fire(&w, &wf, 2);
    w.tick();
    assert_eq!(m.count(), 2);
    assert_eq!(w.notes(), notes, "keine zweite Notiz");
    assert!(matches!(
        w.output(&second, "note")["outcome"].as_str(),
        Some("unchanged") | Some("updated")
    ));

    // Herkunft der Notiz (AK8-Grundlage): Modell, Token, Quellen.
    let prov = w.provenance(
        crate::managers::provenance::SubjectKind::KnowledgeNote,
        &format!("ergebnis-{}", w.meeting_id),
    );
    assert!(!prov.is_empty());
    assert_eq!(prov[0].model_id.as_deref(), Some("llm-test"));
    assert_eq!(prov[0].prompt_tokens, Some(100));
    assert!(prov[0].sources.iter().any(|s| s.kind == "segment"));

    // Die Erinnerungen kommen zu ihrer Zeit, je eine Frist, hoechstens einmal.
    assert!(w.toasts().is_empty());
    w.set_now(at(2026, 10, 14, 9, 0));
    w.tick();
    assert_eq!(w.toasts().len(), 1, "{:?}", w.toasts());
    assert_eq!(w.toasts()[0].0, "Frist morgen: Die Abnahme findet statt.");
    w.set_now(at(2026, 10, 30, 9, 0));
    let outcomes = w.tick();
    assert!(outcomes.iter().all(|(_, o)| *o == Done), "{outcomes:?}");
    assert_eq!(
        w.toasts().len(),
        2,
        "zwei Laeufe, aber jede Frist nur einmal: {:?}",
        w.toasts()
    );
    assert_eq!(
        w.toasts()[1].0,
        "Frist morgen: Die Septemberrechnung muss raus."
    );
}

#[test]
fn ak7_a_deadline_in_two_days_from_the_extraction_is_notified_at_the_configured_time() {
    let reply = json!({
        "todos": [],
        "deadlines": [{
            "text": "Das Angebot geht an die Stadtwerke.",
            "due_phrase": "bis übermorgen",
            "due_date": "2026-10-03",
            "segments": ["S2"],
            "quote": "Ich schicke es bis übermorgen an den Kunden."
        }],
        "decisions": []
    })
    .to_string();
    let m = block(mock(vec![ok(&reply)]));
    let w = world(&m);
    let wf = w.workflow(vec![
        step("extract", "agent.extract", json!({})),
        step(
            "erinnerung",
            "deadline.remind",
            json!({"days_before": 1, "at": "09:00"}),
        ),
    ]);
    let run = fire(&w, &wf, 1);
    assert_eq!(
        w.tick(),
        vec![(
            run.clone(),
            Parked {
                reason: "defer".into()
            }
        )]
    );
    let out = w.output(&run, "extract");
    assert_eq!(
        out["deadlines"][0]["due"], "2026-10-03",
        "in zwei Tagen ab dem 1.10."
    );
    assert!(out["deadlines"][0]["due_source"]
        .as_str()
        .unwrap()
        .starts_with("angabe:"));
    assert!(w.toasts().is_empty());

    w.set_now(at(2026, 10, 2, 8, 59));
    w.tick();
    assert!(w.toasts().is_empty(), "08:59 ist zu frueh");
    w.set_now(at(2026, 10, 2, 9, 0));
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert_eq!(w.toasts().len(), 1);
    assert_eq!(
        w.toasts()[0].0,
        "Frist morgen: Das Angebot geht an die Stadtwerke."
    );
    assert!(w.toasts()[0].1.starts_with("Samstag, 03.10.2026."));
    assert_eq!(
        m.count(),
        1,
        "ein Modellaufruf, die Erinnerung ruft kein Modell"
    );
}

#[test]
fn a_model_that_answers_garbage_leads_to_no_note_no_reminder_and_no_error() {
    let m = block(mock(vec![ok("{kaputt"), ok("noch kaputt")]));
    let w = world(&m);
    w.grant("vault-1", Capability::VaultWrite, GrantMode::Allow);
    let wf = template(&w);
    let run = fire(&w, &wf, 1);
    assert_eq!(w.tick(), vec![(run.clone(), Done)]);
    assert_eq!(w.state(&run, "extract"), StepState::Done);
    assert_eq!(w.output(&run, "extract")["outcome"], "no_action");
    assert_eq!(
        w.state(&run, "note"),
        StepState::Skipped,
        "keine leere Freigabe: die Bedingung haelt den Schritt zurueck"
    );
    assert!(w.notes().is_empty() && w.toasts().is_empty());
    assert!(w.pending().is_empty());
}
