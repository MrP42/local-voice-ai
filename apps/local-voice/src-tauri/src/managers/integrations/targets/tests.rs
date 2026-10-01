use std::cell::RefCell;

use super::*;
use crate::managers::integrations::approvals;
use crate::managers::integrations::model::GrantMode;
use crate::managers::integrations::test_support::Fx;
use crate::managers::integrations::{audit, secrets};

fn tmp_dir() -> std::path::PathBuf {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().to_path_buf();
    std::mem::forget(d);
    p
}

fn smtp_settings() -> TargetSettings {
    TargetSettings {
        host: Some("smtp.example.de".into()),
        port: Some(587),
        security: Some(smtp::Security::Starttls),
        username: Some("patrick".into()),
        from_address: Some("patrick@example.de".into()),
        from_name: Some("Patrick".into()),
        secret: Some("App-Passwort-123".into()),
        ..Default::default()
    }
}

fn recording_put(
    store_in: &RefCell<Vec<(String, String, String)>>,
) -> impl Fn(&Integration, &str, &str) -> Result<(), String> + '_ {
    move |i, slot, value| {
        store_in
            .borrow_mut()
            .push((i.id.clone(), slot.to_string(), value.to_string()));
        Ok(())
    }
}

#[test]
fn smtp_settings_become_a_checked_config_without_the_password() {
    let fx = Fx::new();
    let conn = fx.conn();
    let saved = RefCell::new(Vec::new());
    let i = create_with_secret(
        &conn,
        Kind::Smtp,
        "Mein Postfach",
        None,
        &smtp_settings(),
        &recording_put(&saved),
        1_000,
    )
    .unwrap();
    assert_eq!(i.kind, Kind::Smtp);
    assert_eq!(i.direction, Direction::Write);
    assert_eq!(i.account_hint.as_deref(), Some("smtp.example.de"));
    assert!(!i.config_json.contains("App-Passwort-123"));
    let cfg: Value = serde_json::from_str(&i.config_json).unwrap();
    assert_eq!(cfg["port"], 587);
    assert_eq!(cfg["security"], "starttls");
    let saved = saved.borrow();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0],
        (
            i.id.clone(),
            "password".to_string(),
            "App-Passwort-123".to_string()
        )
    );
}

#[test]
fn the_default_port_follows_the_security_choice() {
    let mut s = smtp_settings();
    s.port = None;
    s.security = Some(smtp::Security::Tls);
    let raw = config_from_settings(Kind::Smtp, &s, None);
    assert_eq!(raw["port"], 465);
    s.security = None;
    assert_eq!(config_from_settings(Kind::Smtp, &s, None)["port"], 587);
}

#[test]
fn a_missing_secret_stops_the_creation_and_leaves_no_entry() {
    let fx = Fx::new();
    let conn = fx.conn();
    let saved = RefCell::new(Vec::new());
    let mut s = smtp_settings();
    s.secret = Some(String::new());
    let err = create_with_secret(
        &conn,
        Kind::Smtp,
        "X",
        None,
        &s,
        &recording_put(&saved),
        1_000,
    )
    .unwrap_err();
    assert!(err.contains("Passwort fehlt"), "{err}");
    let w = TargetSettings {
        endpoint: Some("https://os.example.de/mcp".into()),
        ..Default::default()
    };
    let err = create_with_secret(
        &conn,
        Kind::Wissen,
        "W",
        None,
        &w,
        &recording_put(&saved),
        1_000,
    )
    .unwrap_err();
    assert!(err.contains("Zugangsschlüssel fehlt"), "{err}");
    assert!(store::list(&conn).unwrap().is_empty());
    assert!(saved.borrow().is_empty());
}

#[test]
fn invalid_settings_never_create_an_entry() {
    let fx = Fx::new();
    let conn = fx.conn();
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    let mut s = smtp_settings();
    s.host = Some(String::new());
    assert!(create_with_secret(&conn, Kind::Smtp, "X", None, &s, &put, 1).is_err());
    let mut s = smtp_settings();
    s.security = Some(smtp::Security::Plain);
    let err = create_with_secret(&conn, Kind::Smtp, "X", None, &s, &put, 1).unwrap_err();
    assert!(err.contains("nur zu diesem Rechner"), "{err}");
    let w = TargetSettings {
        endpoint: Some("http://os.example.de/mcp".into()),
        secret: Some("wai_abc".into()),
        ..Default::default()
    };
    assert!(create_with_secret(&conn, Kind::Wissen, "W", None, &w, &put, 1).is_err());
    assert!(store::list(&conn).unwrap().is_empty());
}

#[test]
fn a_failing_secret_store_rolls_the_entry_back() {
    let fx = Fx::new();
    let conn = fx.conn();
    let put = |_: &Integration, _: &str, _: &str| Err("Speicher voll".to_string());
    let err = create_with_secret(&conn, Kind::Smtp, "X", None, &smtp_settings(), &put, 1_000)
        .unwrap_err();
    assert_eq!(err, "Speicher voll");
    assert!(store::list(&conn).unwrap().is_empty());
}

#[test]
fn smtp_without_a_login_needs_no_password_but_only_on_this_machine() {
    let fx = Fx::new();
    let conn = fx.conn();
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    let local = TargetSettings {
        host: Some("127.0.0.1".into()),
        port: Some(2525),
        security: Some(smtp::Security::Plain),
        from_address: Some("a@example.de".into()),
        ..Default::default()
    };
    assert!(create_with_secret(&conn, Kind::Smtp, "Lokal", None, &local, &put, 1).is_ok());
}

#[test]
fn updating_keeps_the_old_secret_when_none_is_given_and_replaces_it_otherwise() {
    let fx = Fx::new();
    let conn = fx.conn();
    let saved = RefCell::new(Vec::new());
    let i = create_with_secret(
        &conn,
        Kind::Smtp,
        "Postfach",
        None,
        &smtp_settings(),
        &recording_put(&saved),
        1_000,
    )
    .unwrap();
    let change = TargetSettings {
        host: Some("mail.example.de".into()),
        ..Default::default()
    };
    let u = update_settings(&conn, &i.id, &change, &recording_put(&saved), 2_000).unwrap();
    assert_eq!(u.account_hint.as_deref(), Some("mail.example.de"));
    let cfg: Value = serde_json::from_str(&u.config_json).unwrap();
    assert_eq!(cfg["host"], "mail.example.de");
    assert_eq!(cfg["username"], "patrick", "nicht gesendete Felder bleiben");
    assert_eq!(saved.borrow().len(), 1, "kein neues Geheimnis ohne Eingabe");
    let change = TargetSettings {
        secret: Some("Neues-Passwort-9".into()),
        ..Default::default()
    };
    update_settings(&conn, &i.id, &change, &recording_put(&saved), 3_000).unwrap();
    assert_eq!(saved.borrow().len(), 2);
    assert_eq!(saved.borrow()[1].2, "Neues-Passwort-9");
    // Die Aenderung steht im Audit, das Passwort nicht.
    let rows = audit::list(&conn, &audit::AuditFilter::default(), 50).unwrap();
    assert!(rows.iter().any(|r| r
        .detail_json
        .as_deref()
        .is_some_and(|d| d.contains("\"phase\":\"updated\""))));
    assert!(!format!("{rows:?}").contains("Neues-Passwort-9"));
}

#[test]
fn updating_with_invalid_values_changes_nothing() {
    let fx = Fx::new();
    let conn = fx.conn();
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    let i = create_with_secret(
        &conn,
        Kind::Smtp,
        "Postfach",
        None,
        &smtp_settings(),
        &put,
        1_000,
    )
    .unwrap();
    let bad = TargetSettings {
        port: Some(0),
        ..Default::default()
    };
    assert!(update_settings(&conn, &i.id, &bad, &put, 2_000).is_err());
    assert_eq!(
        store::get(&conn, &i.id).unwrap().unwrap().config_json,
        i.config_json
    );
}

// ---------------------------------------------------------------------------
// Ordner
// ---------------------------------------------------------------------------

fn folder_integration(conn: &Connection, root: &std::path::Path, sub: &str) -> Integration {
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    let s = TargetSettings {
        path: Some(root.display().to_string()),
        subfolder: Some(sub.to_string()),
        ..Default::default()
    };
    create_with_secret(conn, Kind::Folder, "Ablage", None, &s, &put, 1_000).unwrap()
}

#[test]
fn a_folder_with_an_escaping_subfolder_cannot_be_created() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    for sub in [
        "../draussen",
        "C:\\Windows",
        "\\\\server\\share",
        "a/../../b",
    ] {
        let s = TargetSettings {
            path: Some(root.display().to_string()),
            subfolder: Some(sub.to_string()),
            ..Default::default()
        };
        let err = create_with_secret(&conn, Kind::Folder, "X", None, &s, &put, 1).unwrap_err();
        assert_eq!(err, "folder_path_escape", "{sub}");
    }
    assert!(store::list(&conn).unwrap().is_empty());
}

#[test]
fn the_user_places_an_export_in_the_subfolder_and_it_is_audited() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = folder_integration(&conn, &root, "Protokolle");
    let what = ExportPlacement {
        meeting_id: "m1",
        file_name: "Wochenmeeting: Q3.md",
        format: "md",
    };
    let out = place_export(&conn, Caller::User, &i.id, &what, None, 5_000, |p| {
        std::fs::write(p, "# Protokoll\n").map_err(|e| e.to_string())
    })
    .unwrap();
    let GateOutcome::Done(placed) = out else {
        panic!("{out:?}")
    };
    assert_eq!(placed.rel, "Protokolle/Wochenmeeting_ Q3.md");
    assert_eq!(
        std::fs::read_to_string(root.join("Protokolle").join("Wochenmeeting_ Q3.md")).unwrap(),
        "# Protokoll\n"
    );
    let rows = audit::list(&conn, &audit::AuditFilter::default(), 50).unwrap();
    let row = rows
        .iter()
        .find(|r| r.capability.as_deref() == Some("files.write"))
        .unwrap();
    assert_eq!(row.caller, "user");
    assert_eq!(row.outcome, "ok");
    assert_eq!(
        row.target.as_deref(),
        Some("Protokolle/Wochenmeeting_ Q3.md")
    );
    // Kein absoluter Pfad im Protokoll.
    assert!(!format!("{rows:?}").contains(&root.display().to_string().replace('\\', "\\\\")));
}

#[test]
fn a_workflow_export_waits_for_approval_then_lands_once() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = folder_integration(&conn, &root, "");
    let what = ExportPlacement {
        meeting_id: "m1",
        file_name: "Bericht.md",
        format: "md",
    };
    let write = |p: &std::path::Path| std::fs::write(p, "x").map_err(|e| e.to_string());
    let pending = place_export(&conn, Caller::Workflow, &i.id, &what, None, 5_000, write).unwrap();
    let GateOutcome::Pending { approval_id } = pending else {
        panic!("{pending:?}")
    };
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        0,
        "vor der Freigabe wird nichts geschrieben"
    );
    approvals::decide(&conn, &approval_id, true, 6_000).unwrap();
    let done = place_export(
        &conn,
        Caller::Workflow,
        &i.id,
        &what,
        Some(&approval_id),
        7_000,
        write,
    )
    .unwrap();
    assert!(matches!(done, GateOutcome::Done(_)), "{done:?}");
    let again = place_export(
        &conn,
        Caller::Workflow,
        &i.id,
        &what,
        Some(&approval_id),
        8_000,
        write,
    )
    .unwrap();
    assert!(matches!(again, GateOutcome::Denied { .. }), "{again:?}");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn a_read_only_folder_refuses_writing_even_for_the_user() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = folder_integration(&conn, &root, "");
    store::update(
        &conn,
        &i.id,
        &crate::managers::integrations::model::IntegrationPatch {
            direction: Some(Direction::Read),
            ..Default::default()
        },
        2_000,
    )
    .unwrap();
    let what = ExportPlacement {
        meeting_id: "m1",
        file_name: "B.md",
        format: "md",
    };
    let out = place_export(&conn, Caller::User, &i.id, &what, None, 5_000, |_| Ok(())).unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "direction_blocks",
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn a_hostile_agent_cannot_pick_a_path_only_a_name() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = folder_integration(&conn, &root, "");
    store::set_grant(
        &conn,
        &i.id,
        Capability::FilesWrite,
        Caller::AgentExternal,
        GrantMode::Allow,
    )
    .unwrap();
    let what = ExportPlacement {
        meeting_id: "m1",
        file_name: "..\\..\\Windows\\boese.md",
        format: "md",
    };
    let out = place_export(
        &conn,
        Caller::AgentExternal,
        &i.id,
        &what,
        None,
        5_000,
        |p| std::fs::write(p, "x").map_err(|e| e.to_string()),
    )
    .unwrap();
    let GateOutcome::Done(placed) = out else {
        panic!("{out:?}")
    };
    assert!(
        !placed.rel.contains('/') && !placed.rel.contains('\\'),
        "{}",
        placed.rel
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert!(!root.parent().unwrap().join("Windows").exists());
}

// ---------------------------------------------------------------------------
// Vault und Test
// ---------------------------------------------------------------------------

fn vault_integration(conn: &Connection, root: &std::path::Path) -> Integration {
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    let s = TargetSettings {
        path: Some(root.display().to_string()),
        ..Default::default()
    };
    create_with_secret(conn, Kind::Obsidian, "AI-OS Vault", None, &s, &put, 1_000).unwrap()
}

fn note() -> NoteInput {
    NoteInput {
        meeting_id: "m-123".into(),
        title: "Planung".into(),
        date_label: "01.10.2026 09:00".into(),
        date_iso: "2026-10-01".into(),
        source: "besprechung".into(),
        source_url: None,
        body_md: "# Planung\n\nText\n".into(),
    }
}

#[test]
fn a_vault_needs_an_existing_folder_and_applies_the_defaults() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = vault_integration(&conn, &root);
    let cfg: Value = serde_json::from_str(&i.config_json).unwrap();
    assert_eq!(cfg["subfolder"], "00_inbox");
    assert_eq!(cfg["context_area"], "beruf");
    assert_eq!(cfg["tier"], "propose");
    let put = |_: &Integration, _: &str, _: &str| Ok(());
    let missing = TargetSettings {
        path: Some(root.join("gibt-es-nicht").display().to_string()),
        ..Default::default()
    };
    assert_eq!(
        create_with_secret(&conn, Kind::Obsidian, "V", None, &missing, &put, 1).unwrap_err(),
        "vault_path_not_found"
    );
    let relative = TargetSettings {
        path: Some("relativ".into()),
        ..Default::default()
    };
    assert_eq!(
        create_with_secret(&conn, Kind::Obsidian, "V", None, &relative, &put, 1).unwrap_err(),
        "vault_path_relative"
    );
}

#[test]
fn a_note_goes_through_the_gate_updates_in_place_and_is_audited() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = vault_integration(&conn, &root);
    let first = save_note(&conn, Caller::User, &i.id, &note(), None, 5_000).unwrap();
    let GateOutcome::Done(r1) = first else {
        panic!("{first:?}")
    };
    assert_eq!(r1.kind, obsidian::SaveKind::Created);
    let second = save_note(&conn, Caller::User, &i.id, &note(), None, 6_000).unwrap();
    let GateOutcome::Done(r2) = second else {
        panic!("{second:?}")
    };
    assert_eq!(r2.kind, obsidian::SaveKind::Unchanged);
    assert_eq!(r2.rel, r1.rel);
    let rows = audit::list(&conn, &audit::AuditFilter::default(), 50).unwrap();
    let writes: Vec<_> = rows
        .iter()
        .filter(|r| r.capability.as_deref() == Some("vault.write"))
        .collect();
    assert_eq!(writes.len(), 2);
    assert!(writes[0]
        .target
        .as_deref()
        .unwrap()
        .starts_with("meeting-m-123"));
}

#[test]
fn an_agent_note_waits_for_approval_by_default() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = vault_integration(&conn, &root);
    let out = save_note(&conn, Caller::AgentLocal, &i.id, &note(), None, 5_000).unwrap();
    assert!(matches!(out, GateOutcome::Pending { .. }), "{out:?}");
    assert_eq!(
        std::fs::read_dir(root.join("00_inbox"))
            .map(|d| d.count())
            .unwrap_or(0),
        0
    );
}

#[test]
fn the_vault_test_reports_a_vanished_folder_with_code_and_text() {
    let fx = Fx::new();
    let conn = fx.conn();
    let root = tmp_dir();
    let i = vault_integration(&conn, &root);
    let none = |_: &Integration, _: &str| Ok(None);
    let ok = test_target(
        &conn,
        &i.id,
        &none,
        &Default::default(),
        &Default::default(),
        5_000,
    )
    .unwrap();
    assert_eq!((ok.ok, ok.code.as_str()), (true, "vault_ok"));
    std::fs::remove_dir_all(&root).unwrap();
    let bad = test_target(
        &conn,
        &i.id,
        &none,
        &Default::default(),
        &Default::default(),
        6_000,
    )
    .unwrap();
    assert!(!bad.ok);
    assert_eq!(bad.code, "vault_path_not_found");
    assert!(bad.detail.unwrap().contains("nicht gefunden"));
    assert!(store::get(&conn, &i.id)
        .unwrap()
        .unwrap()
        .last_error
        .is_some());
}

#[test]
fn the_real_secret_namespace_round_trips_a_smtp_password() {
    // Mit dem echten DPAPI-Speicher in einem Testordner: schreiben, lesen, Zustand.
    let dir = tmp_dir();
    let i = Integration {
        id: "smtp-test-1".into(),
        kind: Kind::Smtp,
        label: "Test".into(),
        enabled: true,
        direction: Direction::Write,
        config_json: "{}".into(),
        account_hint: None,
        data_class: None,
        created_at: 0,
        updated_at: 0,
        last_ok_at: None,
        last_error: None,
    };
    secrets::put_in(&dir, &i, "password", b"App-Passwort-123").unwrap();
    let back = secrets::get_text_in(&dir, &i, "password").unwrap().unwrap();
    assert_eq!(back.as_str(), "App-Passwort-123");
    // Auf der Platte steht nur Geheimtext.
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let bytes = std::fs::read(entry.path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("App-Passwort-123"));
    }
    assert!(secrets::get_text_in(&dir, &i, "other").unwrap().is_none());
}
