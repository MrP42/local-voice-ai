use super::*;
use crate::managers::integrations::model::{Capability, GrantMode, Kind, NewIntegration};
use crate::managers::integrations::test_support::{add_calendar_source, folder, Fx};

#[test]
fn the_dump_lists_adopted_calendar_sources_with_the_effective_modes() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-1", "ics", "Outlook privat", 100);
    add_calendar_source(&conn, "graph-1", "graph", "Arbeit", 200);
    let dump = build(&conn, None, Some(&fx.db_path)).unwrap();

    let list = dump["integrations"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"], "src-1");
    assert_eq!(list[0]["kind"], "ics");
    assert_eq!(list[0]["direction"], "read");
    assert_eq!(list[1]["id"], "graph-1");
    // Wirksame Modi, Vorgaben eingerechnet: extern aus, Workflow liest frei.
    let read = &list[0]["effective"]["calendar.read"];
    assert_eq!(read["agent_external"], "off");
    assert_eq!(read["workflow"], "allow");
    assert_eq!(read["user"], "allow");
    // Graph ist standardmaessig nur lesend: Schreiben ist durch die Richtung aus.
    let write = &list[1]["effective"]["calendar.write"];
    assert_eq!(write["workflow"], "off");
    assert_eq!(write["user"], "off");
    assert_eq!(dump["calendar_sources"].as_array().unwrap().len(), 2);
    assert_eq!(dump["counts"]["integrations"], 2);
    assert!(dump["db"].as_str().unwrap().ends_with("meetings.db"));
    assert!(dump["schema_version"].as_i64().unwrap() >= 6);
}

#[test]
fn stored_grants_show_up_twice_as_rows_and_as_effective_mode() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    crate::managers::integrations::store::set_grant(
        &conn,
        &f.id,
        Capability::FilesWrite,
        Caller::AgentExternal,
        GrantMode::Ask,
    )
    .unwrap();
    let dump = build(&conn, None, None).unwrap();
    let i = &dump["integrations"][0];
    assert_eq!(i["grants"][0]["capability"], "files.write");
    assert_eq!(i["grants"][0]["caller"], "agent_external");
    assert_eq!(i["grants"][0]["mode"], "ask");
    assert_eq!(i["effective"]["files.write"]["agent_external"], "ask");
    assert_eq!(dump["counts"]["grants"], 1);
}

#[test]
fn the_dump_reports_the_state_of_secrets_but_never_their_content() {
    let fx = Fx::new();
    let conn = fx.conn();
    let smtp = crate::managers::integrations::store::create(
        &conn,
        &NewIntegration::new(Kind::Smtp, "Postfach"),
        1,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dump = build(&conn, Some(dir.path()), None).unwrap();
    assert_eq!(dump["integrations"][0]["secrets"][0]["slot"], "password");
    assert_eq!(dump["integrations"][0]["secrets"][0]["status"], "missing");

    // Eine kaputte Datei: Zustand `broken`, aber kein Wort ihres Inhalts im Dump.
    std::fs::write(
        dir.path().join(format!("int-{}-password.bin", smtp.id)),
        b"GEHEIMER-INHALT",
    )
    .unwrap();
    let dump = build(&conn, Some(dir.path()), None).unwrap();
    assert_eq!(dump["integrations"][0]["secrets"][0]["status"], "broken");
    assert!(!dump.to_string().contains("GEHEIMER"));
    // Ohne Ordner: keine Angaben statt geratener.
    let dump = build(&conn, None, None).unwrap();
    assert!(dump["integrations"][0]["secrets"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[cfg(windows)]
#[test]
fn a_present_secret_shows_as_present_and_its_content_stays_out() {
    let fx = Fx::new();
    let conn = fx.conn();
    let smtp = crate::managers::integrations::store::create(
        &conn,
        &NewIntegration::new(Kind::Smtp, "Postfach"),
        1,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    crate::managers::integrations::secrets::put_in(
        dir.path(),
        &smtp,
        "password",
        b"GEHEIMES-PASSWORT",
    )
    .unwrap();
    let dump = build(&conn, Some(dir.path()), None).unwrap();
    assert_eq!(dump["integrations"][0]["secrets"][0]["status"], "present");
    assert!(!dump.to_string().contains("GEHEIMES"));
}

#[test]
fn a_secret_that_slipped_into_a_config_is_masked_in_the_dump() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    // Am Register vorbei geschrieben (alte Version, Handarbeit): der Dump schwaerzt trotzdem.
    conn.execute(
        "UPDATE integrations SET config_json = ?2 WHERE id = ?1",
        rusqlite::params![
            f.id,
            r#"{"path":"C:/A","password":"hunter2","url":"https://u:pw@h/"}"#
        ],
    )
    .unwrap();
    let dump = build(&conn, None, None).unwrap();
    let text = dump.to_string();
    assert!(
        !text.contains("hunter2") && !text.contains("u:pw@"),
        "{text}"
    );
    assert_eq!(dump["integrations"][0]["config"]["path"], "C:/A");
    assert_eq!(dump["integrations"][0]["config"]["password"], "***");
}

#[test]
fn the_dump_counts_audit_approvals_and_provenance_and_lists_recent_audit() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let request = crate::managers::integrations::gate::Request {
        caller: Caller::AgentExternal,
        integration_id: &f.id,
        capability: Capability::FilesRead,
        target: Some("x.md"),
        args: None,
        tool_mode: None,
    };
    crate::managers::integrations::gate::check(&conn, &request, 1_000).unwrap();
    let ask = crate::managers::integrations::gate::Request {
        caller: Caller::Workflow,
        capability: Capability::FilesWrite,
        ..request.clone()
    };
    crate::managers::integrations::gate::check(&conn, &ask, 2_000).unwrap();
    let dump = build(&conn, None, None).unwrap();
    assert_eq!(dump["counts"]["audit"], 2);
    assert_eq!(dump["counts"]["approvals_pending"], 1);
    assert_eq!(dump["counts"]["provenance"], 0);
    let recent = dump["recent_audit"].as_array().unwrap();
    assert_eq!(recent[0]["outcome"], "pending");
    assert_eq!(recent[1]["outcome"], "denied");
    assert_eq!(recent[1]["capability"], "files.read");
}

#[test]
fn the_table_form_lists_every_integration() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-1", "ics", "Outlook privat", 100);
    folder(&conn, "Ablage");
    let text = format_table(&build(&conn, None, None).unwrap());
    assert!(text.starts_with("integrations: 2"), "{text}");
    assert!(text.contains("src-1") && text.contains("Outlook privat") && text.contains("Ablage"));
    assert!(text.contains("audit: 0, approvals pending: 0, provenance: 0"));
}

// ---------------------------------------------------------------------------
// Audit-Dump (A8, AK11)
// ---------------------------------------------------------------------------

use crate::managers::integrations::audit;
use crate::managers::integrations::model::{AuditOutcome, NewAudit};

fn put(
    conn: &Connection,
    caller: &str,
    capability: &str,
    outcome: AuditOutcome,
    target: &str,
    detail: Option<Value>,
    now_ms: i64,
) -> i64 {
    audit::record_at(
        conn,
        &NewAudit {
            caller: caller.to_string(),
            integration_id: Some("agents".to_string()),
            capability: Some(capability.to_string()),
            target: Some(target.to_string()),
            outcome,
            detail,
        },
        now_ms,
    )
    .unwrap()
}

#[test]
fn the_audit_dump_lists_every_entry_oldest_first_with_parsed_detail_and_totals() {
    let fx = Fx::new();
    let conn = fx.conn();
    put(&conn, "agent_external", "transcribe.file", AuditOutcome::Ok, "transcribe_file · Claude Code (C1)", Some(json!({"phase": "done"})), 1_000);
    put(&conn, "agent_external", "recording.start", AuditOutcome::Pending, "start_recording · Claude Code (C1)", Some(json!({"phase": "approval"})), 2_000);
    put(&conn, "user", "media.fetch", AuditOutcome::Ok, "youtube:dQw4w9WgXcQ", None, 3_000);
    put(&conn, "agent_external", "tts.render", AuditOutcome::Denied, "tts_render_audio · Claude Code (C1)", Some(json!({"reason": "tool_off"})), 4_000);
    let dump = build_audit(&conn, Some(&fx.db_path), None).unwrap();

    assert_eq!(dump["returned"], 4);
    assert_eq!(dump["truncated"], false);
    assert_eq!(dump["retention"]["max_rows"], audit::MAX_ROWS);
    assert_eq!(dump["retention"]["rows"], 4);
    assert!(dump["db"].as_str().unwrap().ends_with("meetings.db"));
    let entries = dump["entries"].as_array().unwrap();
    let caps: Vec<&str> = entries.iter().map(|e| e["capability"].as_str().unwrap()).collect();
    assert_eq!(caps, ["transcribe.file", "recording.start", "media.fetch", "tts.render"], "aelteste zuerst");
    assert_eq!(entries[0]["detail"]["phase"], "done", "das Detail ist ein Objekt, kein Text");
    assert_eq!(entries[2]["detail"], Value::Null);
    assert_eq!(entries[3]["detail"]["reason"], "tool_off");
    assert_eq!(entries[1]["target"], "start_recording · Claude Code (C1)");
    assert_eq!(dump["by_outcome"], json!({"denied": 1, "ok": 2, "pending": 1}));
    assert_eq!(dump["by_caller"], json!({"agent_external": 3, "user": 1}));
    assert_eq!(dump["by_capability"]["recording.start"], 1);
    assert_eq!(dump["by_capability"]["media.fetch"], 1);
}

#[test]
fn the_audit_dump_is_capped_by_the_limit_and_says_so() {
    let fx = Fx::new();
    let conn = fx.conn();
    for i in 0..30 {
        put(&conn, "agent_external", "meeting.create", AuditOutcome::Ok, &format!("create_meeting #{i}"), None, 1_000 + i);
    }
    let dump = build_audit(&conn, None, Some(10)).unwrap();
    assert_eq!(dump["returned"], 10);
    assert_eq!(dump["truncated"], true);
    assert_eq!(dump["retention"]["rows"], 30, "die Summen gelten fuer die ganze Tabelle");
    assert_eq!(dump["by_outcome"]["ok"], 30);
    let entries = dump["entries"].as_array().unwrap();
    assert_eq!(entries[0]["target"], "create_meeting #20", "die neuesten zehn, aelteste davon zuerst");
    assert_eq!(entries[9]["target"], "create_meeting #29");
    // Null wird auf eins angehoben; ohne Angabe gilt die Vorgabe.
    assert_eq!(build_audit(&conn, None, Some(0)).unwrap()["returned"], 1);
    assert_eq!(build_audit(&conn, None, None).unwrap()["limit"], AUDIT_DUMP_DEFAULT_LIMIT);
}

#[test]
fn the_audit_retention_is_capped_and_the_dump_reports_the_cap() {
    let fx = Fx::new();
    let conn = fx.conn();
    let cap = audit::MAX_ROWS;
    // Fast bis zur Grenze in einem Schritt (schnell), danach ueber den echten Schreibweg.
    conn.execute_batch(&format!(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {})
         INSERT INTO audit_log (ts, caller, integration_id, capability, target, outcome, detail_json)
         SELECT i, 'agent_external', 'agents', 'tts.render', 'bulk', 'ok', NULL FROM n",
        cap - 5
    ))
    .unwrap();
    for i in 0..20 {
        put(&conn, "agent_external", "youtube.add", AuditOutcome::Ok, &format!("neu #{i}"), None, cap + i);
    }
    let dump = build_audit(&conn, None, Some(u32::MAX)).unwrap();
    assert_eq!(dump["retention"]["rows"], cap, "die Tabelle haelt hoechstens MAX_ROWS Zeilen");
    assert_eq!(dump["retention"]["max_rows"], cap);
    assert_eq!(dump["limit"], cap, "der Dump liefert nie mehr als die Aufbewahrung");
    assert_eq!(dump["returned"], cap);
    assert_eq!(dump["truncated"], false);
    let entries = dump["entries"].as_array().unwrap();
    assert!(entries[0]["id"].as_i64().unwrap() > 15, "die aeltesten Zeilen sind gefallen");
    assert_eq!(entries.last().unwrap()["target"], "neu #19", "die neueste bleibt");
    // Nochmal schreiben: es bleibt bei der Grenze.
    put(&conn, "agent_external", "youtube.add", AuditOutcome::Ok, "noch eine", None, cap + 100);
    assert_eq!(build_audit(&conn, None, Some(1)).unwrap()["retention"]["rows"], cap);
}

#[test]
fn the_audit_dump_of_an_empty_log_is_valid_and_secrets_stay_redacted() {
    let fx = Fx::new();
    let conn = fx.conn();
    let empty = build_audit(&conn, None, None).unwrap();
    assert_eq!(empty["returned"], 0);
    assert_eq!(empty["truncated"], false);
    assert_eq!(empty["entries"], json!([]));
    assert_eq!(empty["by_outcome"], json!({}));
    put(&conn, "agent_external", "mail.send", AuditOutcome::Error, "x", Some(json!({"password": "geheim123", "error": "Anmeldung bei https://u:pw@host/p fehlgeschlagen"})), 1);
    let text = build_audit(&conn, None, None).unwrap().to_string();
    assert!(!text.contains("geheim123") && !text.contains("u:pw"), "{text}");
}

#[test]
fn the_audit_table_format_names_every_row_and_marks_truncation() {
    let fx = Fx::new();
    let conn = fx.conn();
    for i in 0..3 {
        put(&conn, "workflow", "files.write", AuditOutcome::Ok, &format!("Ablage #{i}"), None, i);
    }
    let text = format_audit_table(&build_audit(&conn, None, Some(2)).unwrap());
    assert!(text.starts_with("audit: 2 of 3 rows"), "{text}");
    assert!(text.contains("truncated") && text.contains("Ablage #2") && text.contains("files.write"));
    assert!(!text.contains("Ablage #0"));
}
