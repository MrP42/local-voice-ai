use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};

use super::*;
use crate::managers::meetings::store::{MeetingStore, MIGRATIONS};
use crate::managers::workflows::schema::WORKFLOWS_MIGRATION;
use crate::managers::workflows::test_support::{def, note, Fx, T0};
use crate::managers::workflows::validate::parse_definition;

fn workflow(conn: &Connection, name: &str) -> WorkflowRow {
    let mut v = def(vec![note("a")]);
    v["name"] = serde_json::json!(name);
    let d = parse_definition(&v).unwrap();
    let text = serde_json::to_string(&d).unwrap();
    save_workflow(conn, None, &d, &text, T0).unwrap()
}

fn queue(conn: &Connection, wf: &WorkflowRow, key: &str, at: i64) -> InsertedRun {
    insert_run(
        conn,
        &NewRun {
            workflow: wf,
            trigger_key: key,
            origin: Origin::Trigger,
            dry_run: false,
            context_json: "{}",
        },
        at,
    )
    .unwrap()
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

#[test]
fn a_new_workflow_is_off_and_in_dry_run_and_an_edit_disarms_it() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "Erster");
    assert!(!wf.enabled, "neue Ablaeufe sind ausgeschaltet");
    assert!(
        wf.dry_run,
        "neue Ablaeufe planen nur, bis sie scharf geschaltet sind"
    );

    set_enabled(&conn, &wf.id, true, T0).unwrap();
    set_armed(&conn, &wf.id, true, T0).unwrap();
    let armed = get_workflow(&conn, &wf.id).unwrap().unwrap();
    assert!(armed.enabled && !armed.dry_run);

    // Gleiche Definition noch einmal speichern: bleibt scharf.
    let d = parse_definition(&serde_json::from_str(&armed.definition_json).unwrap()).unwrap();
    save_workflow(&conn, Some(&wf.id), &d, &armed.definition_json, T0 + 1).unwrap();
    assert!(!get_workflow(&conn, &wf.id).unwrap().unwrap().dry_run);

    // Geaenderte Definition (neuer Schritt): zurueck in den Trockenlauf.
    let mut changed = serde_json::from_str::<serde_json::Value>(&armed.definition_json).unwrap();
    changed["steps"].as_array_mut().unwrap().push(note("b"));
    let d2 = parse_definition(&changed).unwrap();
    let text2 = serde_json::to_string(&d2).unwrap();
    let after = save_workflow(&conn, Some(&wf.id), &d2, &text2, T0 + 2).unwrap();
    assert!(
        after.dry_run,
        "eine geaenderte Definition ist nicht die, die scharf geschaltet wurde"
    );
    assert!(after.enabled, "eingeschaltet bleibt sie");
    assert!(matches!(
        save_workflow(&conn, Some("gibt-es-nicht"), &d2, &text2, T0),
        Err(WorkflowError::NotFound(_))
    ));
}

#[test]
fn the_same_trigger_key_gives_one_run_and_returns_the_existing_one() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    let first = queue(&conn, &wf, "calendar:42", T0);
    let second = queue(&conn, &wf, "calendar:42", T0 + 5);
    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.run_id, second.run_id);
    let other = queue(&conn, &wf, "calendar:43", T0 + 6);
    assert!(other.created);
    // Derselbe Schluessel in einem ANDEREN Ablauf ist ein eigener Lauf.
    let wf2 = workflow(&conn, "B");
    assert!(queue(&conn, &wf2, "calendar:42", T0).created);
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM workflow_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 3);
}

#[test]
fn the_same_trigger_from_many_threads_makes_exactly_one_run() {
    let fx = Fx::new();
    let wf = workflow(&fx.conn(), "A");
    let path = fx.db_path.clone();
    let wf = std::sync::Arc::new(wf);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            let wf = wf.clone();
            std::thread::spawn(move || {
                let conn = Connection::open(&path).unwrap();
                insert_run(
                    &conn,
                    &NewRun {
                        workflow: &wf,
                        trigger_key: "file:abc",
                        origin: Origin::Trigger,
                        dry_run: false,
                        context_json: "{}",
                    },
                    T0,
                )
                .unwrap()
            })
        })
        .collect();
    let results: Vec<InsertedRun> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.created).count(), 1);
    let ids: std::collections::HashSet<_> = results.iter().map(|r| r.run_id.clone()).collect();
    assert_eq!(ids.len(), 1, "alle bekommen dieselbe Kennung");
}

#[test]
fn the_queue_of_one_workflow_is_capped() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    for i in 0..MAX_QUEUED_PER_WORKFLOW {
        queue(&conn, &wf, &format!("k{i}"), T0 + i);
    }
    let over = insert_run(
        &conn,
        &NewRun {
            workflow: &wf,
            trigger_key: "ueberzaehlig",
            origin: Origin::Trigger,
            dry_run: false,
            context_json: "{}",
        },
        T0,
    );
    assert!(matches!(over, Err(WorkflowError::QueueFull(_))), "{over:?}");
    // Ein bereits bekannter Schluessel wird auch bei voller Warteschlange wiedererkannt.
    assert!(!queue(&conn, &wf, "k0", T0).created);
}

#[test]
fn a_run_goes_to_exactly_one_worker_even_when_many_ask_at_once() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    for i in 0..20 {
        queue(&conn, &wf, &format!("k{i}"), T0 + i);
    }
    let path = fx.db_path.clone();
    let handles: Vec<_> = (0..4)
        .map(|w| {
            let path = path.clone();
            std::thread::spawn(move || {
                let conn = Connection::open(&path).unwrap();
                let mut mine = Vec::new();
                while let Some(run) =
                    claim_next(&conn, &format!("worker-{w}"), T0 + 100, 60_000).unwrap()
                {
                    mine.push(run.id);
                }
                mine
            })
        })
        .collect();
    let mut all: Vec<String> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    assert_eq!(all.len(), 20, "jeder Lauf wurde geholt");
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 20, "kein Lauf an zwei Arbeiter");
}

#[test]
fn claim_follows_the_order_of_arrival_and_waits_for_next_run_at() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    let a = queue(&conn, &wf, "a", T0);
    let b = queue(&conn, &wf, "b", T0 + 10);
    let c = queue(&conn, &wf, "c", T0 + 20);
    // `a` wartet auf spaeter (Wiederholung).
    let run = claim_next(&conn, "w", T0 + 30, 60_000).unwrap().unwrap();
    assert_eq!(run.id, a.run_id, "der aelteste zuerst");
    park_run(
        &conn,
        &run.id,
        "w",
        RunState::Queued,
        Some(T0 + 1_000),
        Some("retry"),
        T0 + 31,
    )
    .unwrap();
    let next = claim_next(&conn, "w", T0 + 40, 60_000).unwrap().unwrap();
    assert_eq!(next.id, b.run_id, "a ist noch nicht faellig");
    let third = claim_next(&conn, "w", T0 + 50, 60_000).unwrap().unwrap();
    assert_eq!(third.id, c.run_id);
    assert!(claim_next(&conn, "w", T0 + 60, 60_000).unwrap().is_none());
    let again = claim_next(&conn, "w", T0 + 1_001, 60_000).unwrap().unwrap();
    assert_eq!(again.id, a.run_id, "nach next_run_at ist a wieder dran");
}

#[test]
fn a_worker_that_lost_the_lease_cannot_write() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    queue(&conn, &wf, "a", T0);
    let run = claim_next(&conn, "alt", T0 + 1, 1_000).unwrap().unwrap();
    // Der Vertrag laeuft ab, ein anderer uebernimmt.
    assert!(recover_requeue(&conn, &run.id, T0 + 5_000).unwrap());
    let run2 = claim_next(&conn, "neu", T0 + 5_001, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(run2.id, run.id);

    let row = StepRow {
        run_id: run.id.clone(),
        step_id: "a".into(),
        attempt: 1,
        ordinal: 0,
        action: "notify.local".into(),
        state: StepState::Running,
        error_class: None,
        input_json: None,
        output_json: None,
        error: None,
        approval_id: None,
        wake_at: None,
        started_at: Some(T0),
        ended_at: None,
    };
    assert_eq!(
        insert_step(&conn, "alt", &row),
        Err(WorkflowError::LeaseLost)
    );
    assert_eq!(
        finish_run(
            &conn,
            &run.id,
            "alt",
            RunState::Done,
            None,
            None,
            T0 + 6_000
        ),
        Err(WorkflowError::LeaseLost)
    );
    assert_eq!(
        park_run(
            &conn,
            &run.id,
            "alt",
            RunState::Queued,
            None,
            None,
            T0 + 6_000
        ),
        Err(WorkflowError::LeaseLost)
    );
    // Der neue Arbeiter darf.
    insert_step(&conn, "neu", &row).unwrap();
    update_step(
        &conn,
        "neu",
        &StepUpdate {
            run_id: &run.id,
            step_id: "a",
            attempt: 1,
            state: StepState::Done,
            error_class: None,
            output_json: Some("{}"),
            error: None,
            approval_id: None,
            wake_at: None,
            ended_at: Some(T0 + 7_000),
        },
    )
    .unwrap();
    // Und der alte scheitert auch beim Aendern.
    assert_eq!(
        update_step(
            &conn,
            "alt",
            &StepUpdate {
                run_id: &run.id,
                step_id: "a",
                attempt: 1,
                state: StepState::Failed,
                error_class: None,
                output_json: None,
                error: Some("zu spaet"),
                approval_id: None,
                wake_at: None,
                ended_at: None,
            }
        ),
        Err(WorkflowError::LeaseLost)
    );
    let rows = steps_for(&conn, &run.id).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, StepState::Done);
}

#[test]
fn only_a_lease_that_has_expired_can_be_recovered() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    queue(&conn, &wf, "a", T0);
    let run = claim_next(&conn, "w", T0, 10_000).unwrap().unwrap();
    assert!(list_expired(&conn, T0 + 9_999).unwrap().is_empty());
    assert!(
        !recover_requeue(&conn, &run.id, T0 + 9_999).unwrap(),
        "Vertrag gilt noch"
    );
    assert_eq!(list_expired(&conn, T0 + 10_001).unwrap().len(), 1);
    // Verlaengern schiebt das Ende hinaus.
    assert_eq!(
        renew_leases(
            &conn,
            "w",
            std::slice::from_ref(&run.id),
            T0 + 9_000,
            10_000
        )
        .unwrap(),
        1
    );
    assert!(list_expired(&conn, T0 + 10_001).unwrap().is_empty());
    // Ein fremder Arbeiter verlaengert nichts.
    assert_eq!(
        renew_leases(&conn, "anderer", std::slice::from_ref(&run.id), T0, 10_000).unwrap(),
        0
    );
}

#[test]
fn old_finished_runs_are_pruned_but_open_runs_never() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    let total = MAX_RUNS_PER_WORKFLOW + 25;
    let mut ids = Vec::new();
    for i in 0..total {
        ids.push(queue(&conn, &wf, &format!("k{i}"), T0 + i).run_id);
    }
    // Alle bis auf die letzten drei sind beendet; die letzten drei bleiben offen.
    for id in &ids[..(total - 3) as usize] {
        conn.execute(
            "UPDATE workflow_runs SET state = 'done', ended_at = ?2 WHERE id = ?1",
            params![id, T0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workflow_run_steps (run_id, step_id, attempt, ordinal, action, state)
             VALUES (?1, 'a', 1, 0, 'notify.local', 'done')",
            params![id],
        )
        .unwrap();
    }
    let removed = prune(&conn).unwrap();
    assert_eq!(removed as i64, total - 3 - MAX_RUNS_PER_WORKFLOW);
    let finished: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workflow_runs WHERE state = 'done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        finished, MAX_RUNS_PER_WORKFLOW,
        "die juengsten beendeten bleiben"
    );
    let open: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workflow_runs WHERE state = 'queued'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open, 3, "offene Laeufe werden nie geloescht");
    // Die Schrittzeilen der geloeschten Laeufe sind mit weg.
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workflow_run_steps WHERE run_id NOT IN (SELECT id FROM workflow_runs)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0);
    // Die aeltesten sind weg, nicht die neuesten.
    assert!(get_run(&conn, &ids[0]).unwrap().is_none());
    assert!(get_run(&conn, &ids[(total - 4) as usize])
        .unwrap()
        .is_some());
}

#[test]
fn a_workflow_with_open_runs_cannot_be_deleted() {
    let fx = Fx::new();
    let conn = fx.conn();
    let wf = workflow(&conn, "A");
    let run = queue(&conn, &wf, "a", T0);
    assert!(matches!(
        delete_workflow(&conn, &wf.id),
        Err(WorkflowError::Busy(_))
    ));
    assert!(cancel_idle(&conn, &run.run_id, T0 + 1).unwrap());
    delete_workflow(&conn, &wf.id).unwrap();
    assert!(get_workflow(&conn, &wf.id).unwrap().is_none());
    assert!(
        get_run(&conn, &run.run_id).unwrap().is_none(),
        "Laeufe gehen mit"
    );
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

/// Wie viele Schritte VOR der Workflow-Migration stehen (ihr Index). Wird gemessen,
/// nicht festgeschrieben: legt ein anderer Zweig vor ihr eine Migration an, bleiben
/// diese Tests nach dem Umnummerieren richtig.
fn workflow_index() -> usize {
    for n in 1..=MIGRATIONS.len() {
        let mut conn = Connection::open_in_memory().unwrap();
        Migrations::new(MIGRATIONS[..n].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        if has_table(&conn, "workflows") {
            return n - 1;
        }
    }
    panic!("keine Migration legt die Tabelle workflows an");
}

fn seed_old_data(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
           VALUES ('M1', 'Wochenbesprechung', 'ready', 'live', 1790000000, 1790000000),
                  ('M2', 'Import Vortrag', 'ready', 'import', 1790100000, 1790100000);
         INSERT INTO integrations (id, kind, label, enabled, direction, config_json,
                                   created_at, updated_at)
           VALUES ('folder-1', 'folder', 'Ablage', 1, 'both', '{}', 1790000000, 1790000000);
         INSERT INTO integration_grants (integration_id, capability, caller, mode)
           VALUES ('folder-1', 'files.write', 'workflow', 'allow');
         INSERT INTO provenance (id, subject_kind, subject_id, created_at, operation, actor_kind)
           VALUES ('P1', 'document', 'D1', 1790000000, 'minutes', 'user');
         INSERT INTO audit_log (ts, caller, outcome) VALUES (1790000000, 'workflow', 'ok');",
    )
    .unwrap();
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn the_workflow_migration_is_an_appended_step_after_the_queue_step() {
    let idx = workflow_index();
    assert!(
        idx >= 8,
        "A1 = 5, A3 = 6, U7 = 7: die Workflow-Migration kommt danach"
    );
    let mut conn = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS[..idx].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    assert_eq!(user_version(&conn), idx as i64);
    for t in [
        "workflows",
        "workflow_runs",
        "workflow_run_steps",
        "workflow_file_ledger",
    ] {
        assert!(
            !has_table(&conn, t),
            "{t} darf vor der Workflow-Migration nicht da sein"
        );
    }
    assert!(has_table(&conn, "import_queue"), "U7 steht davor");
    assert!(has_table(&conn, "integrations"), "A1 steht davor");
}

#[test]
fn a_database_with_old_data_migrates_forward_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        Migrations::new(MIGRATIONS[..workflow_index()].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        seed_old_data(&conn);
    }
    let store = MeetingStore::open_at(&path).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    // Nichts der Altdaten ging verloren.
    assert_eq!(count(&conn, "meetings"), 2);
    assert_eq!(count(&conn, "integrations"), 1);
    assert_eq!(count(&conn, "integration_grants"), 1);
    assert_eq!(count(&conn, "provenance"), 1);
    assert_eq!(count(&conn, "audit_log"), 1);
    // Die neuen Tabellen sind da und leer.
    for t in [
        "workflows",
        "workflow_runs",
        "workflow_run_steps",
        "workflow_file_ledger",
    ] {
        assert!(has_table(&conn, t), "{t} fehlt");
        assert_eq!(count(&conn, t), 0);
    }
    // Und benutzbar.
    let wf = workflow(&conn, "Nach der Migration");
    assert!(queue(&conn, &wf, "k", T0).created);
    // Zweiter Start: nichts weiter zu tun, nichts geht verloren.
    drop(conn);
    drop(store);
    let again = MeetingStore::open_at(&path).unwrap();
    let conn = again.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    assert_eq!(count(&conn, "workflows"), 1);
    assert_eq!(count(&conn, "workflow_runs"), 1);
}

#[test]
fn an_abort_inside_the_workflow_migration_leaves_the_old_database_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        Migrations::new(MIGRATIONS[..workflow_index()].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        seed_old_data(&conn);
    }
    // Der Schritt bricht NACH dem Anlegen der Tabellen ab: alles muss zurueckrollen.
    let broken: &'static str =
        Box::leak(format!("{WORKFLOWS_MIGRATION} SELECT no_such_function();").into_boxed_str());
    let idx = workflow_index();
    let mut steps = MIGRATIONS[..idx].to_vec();
    steps.push(M::up(broken));
    let mut conn = Connection::open(&path).unwrap();
    assert!(Migrations::new(steps).to_latest(&mut conn).is_err());
    assert_eq!(user_version(&conn), idx as i64, "Version unveraendert");
    for t in [
        "workflows",
        "workflow_runs",
        "workflow_run_steps",
        "workflow_file_ledger",
    ] {
        assert!(!has_table(&conn, t), "{t} darf nicht halb angelegt sein");
    }
    assert_eq!(count(&conn, "meetings"), 2);
    assert_eq!(count(&conn, "integration_grants"), 1);
    drop(conn);
    // Mit dem richtigen Stand startet die App danach ganz normal.
    let store = MeetingStore::open_at(&path).unwrap();
    assert_eq!(
        user_version(&store.get_connection().unwrap()),
        MIGRATIONS.len() as i64
    );
}
