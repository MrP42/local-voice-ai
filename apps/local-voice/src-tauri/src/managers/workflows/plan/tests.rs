use rusqlite::Connection;
use serde_json::{json, Value};

use crate::managers::integrations::model::{Capability, GrantMode, Kind};
use crate::managers::workflows::cli::{self, EXIT_BAD_INPUT, EXIT_OK, EXIT_WOULD_BE_DENIED};
use crate::managers::workflows::templates::MEETING_SAMPLE;
use crate::managers::workflows::test_support::{def, register, set_grant, step, Fx};

/// Das Register, das die Beispielvorlage voraussetzt.
fn seed_register(conn: &Connection) {
    register(conn, Kind::Graph, "cal-graph-1");
    register(conn, Kind::Agent, "app-automation");
    register(conn, Kind::Folder, "folder-onedrive-protokolle");
    register(conn, Kind::M365, "m365-1");
    set_grant(
        conn,
        "folder-onedrive-protokolle",
        Capability::FilesWrite,
        GrantMode::Allow,
    );
}

fn plan_of(conn: &Connection, text: &str) -> (i32, Value) {
    let r = cli::dry_run(conn, text);
    (r.exit_code, r.payload)
}

fn step_of<'a>(plan: &'a Value, id: &str) -> &'a Value {
    plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("Schritt {id} fehlt im Plan"))
}

#[test]
fn the_meeting_template_is_planned_with_effect_and_permission_per_step() {
    let fx = Fx::new();
    let conn = fx.conn();
    seed_register(&conn);
    let (code, plan) = plan_of(&conn, MEETING_SAMPLE);
    assert_eq!(
        code,
        EXIT_OK,
        "{}",
        serde_json::to_string_pretty(&plan).unwrap()
    );
    assert_eq!(plan["schema"], "lva-workflow-plan@1");
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["writes"], "nothing");
    assert_eq!(plan["trigger_sample"], true);
    let ids: Vec<&str> = plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["rec", "min", "doc", "mail_me", "mail_all"]);

    // Aufnahme: nie "erlaubt", immer Freigabe (Einwilligung).
    let rec = step_of(&plan, "rec");
    assert_eq!(rec["permission"]["result"], "needs_approval");
    assert_eq!(rec["permission"]["capability"], "recording.start");
    assert_eq!(rec["effect_kind"], "external");
    assert!(rec["effect"].as_str().unwrap().contains("Einwilligung"));
    // Protokoll: schwer, kein Recht noetig.
    let min = step_of(&plan, "min");
    assert_eq!(min["permission"]["result"], "not_required");
    assert_eq!(min["heavy"]["label"], "Sprachmodell");
    assert_eq!(min["params"]["template"], "kunde", "Variable eingesetzt");
    // Ablage: Recht ausdruecklich erlaubt, Titel aus den Beispieldaten.
    let doc = step_of(&plan, "doc");
    assert_eq!(doc["permission"]["result"], "allowed");
    assert_eq!(doc["params"]["name"], "Beispieltermin – Protokoll");
    // Mail: Standard "fragen", Vorschau nennt die Empfaengerregel; der Anhang haengt an
    // einem Vorschritt und bleibt als Verweis stehen.
    let mail = step_of(&plan, "mail_me");
    assert_eq!(mail["permission"]["result"], "needs_approval");
    assert_eq!(mail["permission"]["mode"], "ask");
    assert!(mail["permission"]["preview"]
        .as_str()
        .unwrap()
        .contains("to: me"));
    assert_eq!(mail["unresolved"], json!(["steps.doc.path"]));
    assert_eq!(mail["params"]["subject"], "Protokoll: Beispieltermin");
    // Bedingung des zweiten Mails haengt nicht von einem Schritt ab und ist wahr.
    let all = step_of(&plan, "mail_all");
    assert_eq!(all["condition"]["result"], "true");
    assert_eq!(all["status"], "planned");

    let s = &plan["summary"];
    assert_eq!(s["steps"], 5);
    assert_eq!(s["allowed"], 1);
    assert_eq!(s["needs_approval"], 3);
    assert_eq!(s["denied"], 0);
    assert_eq!(s["heavy"], 1);
    assert_eq!(s["external_effects"], 3);
    assert_eq!(s["would_run_without_intervention"], true);
}

#[test]
fn a_dry_run_writes_nothing_not_even_audit_or_approvals() {
    let fx = Fx::new();
    let conn = fx.conn();
    seed_register(&conn);
    // Ein Plan mit abgelehnten UND fragenden Schritten: auch der schreibt nichts.
    set_grant(&conn, "m365-1", Capability::MailSend, GrantMode::Off);
    let tables: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%fts%'")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let counts = |conn: &Connection| -> Vec<i64> {
        tables
            .iter()
            .map(|t| {
                conn.query_row(&format!("SELECT COUNT(*) FROM \"{t}\""), [], |r| r.get(0))
                    .unwrap_or(-1)
            })
            .collect()
    };
    let rows_before = counts(&conn);
    let changes_before = conn.total_changes();
    let (code, plan) = plan_of(&conn, MEETING_SAMPLE);
    assert_eq!(code, EXIT_WOULD_BE_DENIED);
    assert!(plan["summary"]["needs_approval"].as_u64().unwrap() >= 1);
    assert!(plan["summary"]["denied"].as_u64().unwrap() >= 1);
    assert_eq!(
        conn.total_changes(),
        changes_before,
        "der Plan hat nichts geschrieben"
    );
    assert_eq!(counts(&conn), rows_before, "keine Zeile in keiner Tabelle");
    for t in ["audit_log", "approvals", "provenance", "workflow_runs"] {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{t} muss leer bleiben");
    }
}

#[test]
fn an_integration_that_is_off_or_missing_is_denied_with_the_reason() {
    let fx = Fx::new();
    let conn = fx.conn();
    seed_register(&conn);
    set_grant(&conn, "m365-1", Capability::MailSend, GrantMode::Off);
    let (code, plan) = plan_of(&conn, MEETING_SAMPLE);
    assert_eq!(code, EXIT_WOULD_BE_DENIED);
    let mail = step_of(&plan, "mail_me");
    assert_eq!(mail["permission"]["result"], "denied");
    assert_eq!(mail["permission"]["reason"], "grant_off");
    assert_eq!(mail["permission"]["mode"], "off");
    assert!(mail["permission"]["message"]
        .as_str()
        .unwrap()
        .contains("aus"));
    assert_eq!(plan["summary"]["denied"], 2, "beide Mail-Schritte");
    assert_eq!(plan["summary"]["would_run_without_intervention"], false);

    // Eine Integration, die es nicht gibt.
    let fx2 = Fx::new();
    let conn2 = fx2.conn();
    let (code, plan) = plan_of(&conn2, MEETING_SAMPLE);
    assert_eq!(code, EXIT_WOULD_BE_DENIED);
    assert_eq!(
        step_of(&plan, "rec")["permission"]["reason"],
        "unknown_integration"
    );
    assert_eq!(step_of(&plan, "doc")["permission"]["result"], "denied");
    assert_eq!(
        step_of(&plan, "min")["permission"]["result"],
        "not_required"
    );
}

#[test]
fn a_switched_off_integration_denies_every_step_that_uses_it() {
    let fx = Fx::new();
    let conn = fx.conn();
    seed_register(&conn);
    conn.execute(
        "UPDATE integrations SET enabled = 0 WHERE id = 'folder-onedrive-protokolle'",
        [],
    )
    .unwrap();
    let (code, plan) = plan_of(&conn, MEETING_SAMPLE);
    assert_eq!(code, EXIT_WOULD_BE_DENIED);
    let doc = step_of(&plan, "doc");
    assert_eq!(doc["permission"]["result"], "denied");
    assert_eq!(doc["permission"]["reason"], "integration_disabled");
}

#[test]
fn an_action_without_a_register_capability_is_denied_fail_closed() {
    let fx = Fx::new();
    let conn = fx.conn();
    let d = def(vec![step(
        "w",
        "webhook.post",
        json!({"url": "http://127.0.0.1:5678/webhook/test", "body": {"a": 1}}),
    )]);
    let (code, plan) = plan_of(&conn, &d.to_string());
    assert_eq!(code, EXIT_WOULD_BE_DENIED);
    let w = step_of(&plan, "w");
    assert_eq!(w["permission"]["result"], "denied");
    assert_eq!(w["permission"]["reason"], "capability_not_modeled");
    assert_eq!(w["effect_kind"], "external");
}

#[test]
fn conditions_are_decided_when_they_can_be_and_unknown_when_a_step_decides_them() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut d = def(vec![
        step("a", "notify.local", json!({"title": "eins"})),
        json!({"id": "b", "action": "notify.local", "when": "steps.a.ok",
               "params": {"title": "zwei"}}),
        json!({"id": "c", "action": "notify.local", "when": "vars.laut == false",
               "params": {"title": "drei"}}),
        json!({"id": "d", "action": "notify.local", "when": "vars.laut == true",
               "params": {"title": "vier"}}),
    ]);
    d["variables"] = json!({"laut": {"type": "bool", "default": false}});
    let (code, plan) = plan_of(&conn, &d.to_string());
    assert_eq!(code, EXIT_OK);
    assert_eq!(step_of(&plan, "b")["condition"]["result"], "unknown");
    assert_eq!(step_of(&plan, "b")["status"], "planned");
    assert_eq!(step_of(&plan, "c")["condition"]["result"], "true");
    assert_eq!(step_of(&plan, "d")["condition"]["result"], "false");
    assert_eq!(step_of(&plan, "d")["status"], "skipped");
    assert_eq!(plan["summary"]["skipped"], 1);
}

#[test]
fn a_file_that_is_not_a_valid_definition_gives_all_issues_and_exit_2() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (code, plan) = plan_of(&conn, "{kaputt");
    assert_eq!(code, EXIT_BAD_INPUT);
    assert_eq!(plan["valid"], false);
    assert!(plan["issues"][0]["message"]
        .as_str()
        .unwrap()
        .contains("Zeile"));

    let bad = json!({"schema": "lva-workflow@1", "name": "x", "trigger": {"type": "manual"},
        "steps": [{"id": "a", "action": "mail.send", "params": {"to": "{{trigger.x}}"}},
                  {"id": "b", "action": "unbekannt"}]});
    let (code, plan) = plan_of(&conn, &bad.to_string());
    assert_eq!(code, EXIT_BAD_INPUT);
    assert!(plan["issues"].as_array().unwrap().len() >= 3);
    let text = cli::format_text(&cli::dry_run(&conn, &bad.to_string()));
    assert!(text.contains("keine gültige Definition"));
}

#[test]
fn the_text_form_lists_every_step() {
    let fx = Fx::new();
    let conn = fx.conn();
    seed_register(&conn);
    let r = cli::dry_run(&conn, MEETING_SAMPLE);
    let text = cli::format_text(&r);
    for id in ["rec", "min", "doc", "mail_me", "mail_all"] {
        assert!(text.contains(id), "{id} fehlt in\n{text}");
    }
    assert!(text.contains("needs_approval"));
    assert!(text.contains("5 Schritte"));
}
