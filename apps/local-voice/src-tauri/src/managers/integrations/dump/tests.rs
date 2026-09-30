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
