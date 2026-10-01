use super::*;
use crate::managers::integrations::gate::{self, Decision, Request};
use crate::managers::integrations::test_support::{add_calendar_source, of_kind, Fx};
use crate::managers::meetings::store::MeetingStore;
use serde_json::json;

fn no_secrets(_: &Integration, _: &str) -> String {
    "present".to_string()
}

/// Ein echter, leerer Ordner fuer die Pruefung des Pfads.
fn real_folder() -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);
    path
}

fn make_folder(conn: &Connection, label: &str) -> (Integration, std::path::PathBuf) {
    let path = real_folder();
    let i = create_from_ui(
        conn,
        Kind::Folder,
        label,
        None,
        json!({ "path": path.display().to_string() }),
        1_000,
    )
    .unwrap();
    (i, path)
}

fn mode_of(v: &IntegrationView, cap: Capability, caller: Caller) -> CallerMode {
    v.capabilities
        .iter()
        .find(|c| c.capability == cap)
        .unwrap()
        .modes
        .iter()
        .find(|m| m.caller == caller)
        .unwrap()
        .clone()
}

// ---------------------------------------------------------------------------
// Anlegen
// ---------------------------------------------------------------------------

#[test]
fn a_folder_is_created_with_a_checked_path_and_the_default_direction() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, path) = make_folder(&conn, "  Ablage  ");
    assert_eq!(i.kind, Kind::Folder);
    assert_eq!(i.label, "Ablage");
    assert_eq!(i.direction, Direction::Both);
    let cfg: Value = serde_json::from_str(&i.config_json).unwrap();
    assert_eq!(cfg["path"], path.display().to_string());
}

#[test]
fn a_folder_path_is_refused_when_it_is_empty_relative_missing_or_a_file() {
    let fx = Fx::new();
    let conn = fx.conn();
    let file = real_folder().join("datei.txt");
    std::fs::write(&file, b"x").unwrap();
    let missing = real_folder().join("gibt-es-nicht");
    let cases = [
        ("", ERR_PATH_MISSING),
        ("   ", ERR_PATH_MISSING),
        ("Ablage/unten", ERR_PATH_RELATIVE),
        ("..\\hoch", ERR_PATH_RELATIVE),
        (missing.to_str().unwrap(), ERR_PATH_NOT_FOUND),
        (file.to_str().unwrap(), ERR_PATH_NOT_A_FOLDER),
    ];
    for (path, code) in cases {
        let err = create_from_ui(
            &conn,
            Kind::Folder,
            "Ablage",
            None,
            json!({ "path": path }),
            1,
        )
        .unwrap_err();
        assert_eq!(err, code, "{path:?}");
    }
    assert!(store::list(&conn).unwrap().is_empty(), "nichts angelegt");
}

#[test]
fn a_missing_path_field_is_the_same_as_an_empty_path() {
    let fx = Fx::new();
    let err = create_from_ui(&fx.conn(), Kind::Folder, "X", None, json!({}), 1).unwrap_err();
    assert_eq!(err, ERR_PATH_MISSING);
}

#[test]
fn kinds_that_need_an_account_cannot_be_created_from_the_ui_yet() {
    let fx = Fx::new();
    let conn = fx.conn();
    for kind in [
        Kind::M365,
        Kind::Smtp,
        Kind::Obsidian,
        Kind::Wissen,
        Kind::Agent,
        Kind::Youtube,
        Kind::Ics,
        Kind::Graph,
    ] {
        let err = create_from_ui(&conn, kind, "X", None, json!({}), 1).unwrap_err();
        assert_eq!(err, ERR_KIND_NOT_AVAILABLE, "{kind:?}");
    }
    assert!(store::list(&conn).unwrap().is_empty());
}

#[test]
fn a_config_with_a_secret_looking_extra_is_not_smuggled_in() {
    // Aus dem Feld `path` wird NUR der Pfad uebernommen; andere Felder fallen weg.
    let fx = Fx::new();
    let conn = fx.conn();
    let path = real_folder();
    let i = create_from_ui(
        &conn,
        Kind::Folder,
        "Ablage",
        None,
        json!({ "path": path.display().to_string(), "password": "geheim" }),
        1,
    )
    .unwrap();
    assert!(!i.config_json.contains("geheim"));
    assert!(!i.config_json.contains("password"));
}

#[test]
fn creating_survives_reopening_the_store() {
    let fx = Fx::new();
    let (i, _) = make_folder(&fx.conn(), "Ablage");
    drop(fx.store);
    let reopened = MeetingStore::open_at(&fx.db_path).unwrap();
    let conn = reopened.get_connection().unwrap();
    let again = store::get(&conn, &i.id).unwrap().unwrap();
    assert_eq!(again.label, "Ablage");
    assert_eq!(again.direction, Direction::Both);
}

// ---------------------------------------------------------------------------
// Ansicht und Rechte-Matrix
// ---------------------------------------------------------------------------

#[test]
fn the_view_lists_every_capability_of_the_kind_for_the_three_callers() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let v = get_view(&conn, &i.id, &no_secrets, 2_000).unwrap();
    let caps: Vec<Capability> = v.capabilities.iter().map(|c| c.capability).collect();
    assert_eq!(caps, vec![Capability::FilesRead, Capability::FilesWrite]);
    for c in &v.capabilities {
        let callers: Vec<Caller> = c.modes.iter().map(|m| m.caller).collect();
        assert_eq!(callers, MATRIX_CALLERS.to_vec());
    }
    assert_eq!(
        v.directions,
        vec![Direction::Read, Direction::Write, Direction::Both]
    );
    assert!(!v.calendar_managed);
}

#[test]
fn the_view_shows_the_defaults_of_e3_when_nothing_is_stored() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let v = get_view(&conn, &i.id, &no_secrets, 2_000).unwrap();
    // Externe Agenten: alles aus. Workflow/lokal: Lesen erlaubt, Schreiben fragen.
    let read_ext = mode_of(&v, Capability::FilesRead, Caller::AgentExternal);
    assert_eq!(read_ext.effective, GrantMode::Off);
    assert_eq!(read_ext.stored, None);
    assert_eq!(read_ext.off_reason.as_deref(), Some("grant_off"));
    assert_eq!(
        mode_of(&v, Capability::FilesRead, Caller::Workflow).effective,
        GrantMode::Allow
    );
    assert_eq!(
        mode_of(&v, Capability::FilesWrite, Caller::Workflow).effective,
        GrantMode::Ask
    );
    assert_eq!(
        mode_of(&v, Capability::FilesWrite, Caller::AgentLocal).effective,
        GrantMode::Ask
    );
}

#[test]
fn the_view_reports_the_default_next_to_the_stored_mode_even_when_blocked() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    update_from_ui(&conn, &i.id, None, None, Some(Direction::Read), 2_000).unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 3_000).unwrap();
    // Schreiben ist durch die Richtung gesperrt: wirksam aus, die Vorgabe bleibt sichtbar.
    let m = mode_of(&v, Capability::FilesWrite, Caller::Workflow);
    assert_eq!((m.effective, m.default_mode), (GrantMode::Off, GrantMode::Ask));
    let m = mode_of(&v, Capability::FilesRead, Caller::AgentExternal);
    assert_eq!(m.default_mode, GrantMode::Off);
    let m = mode_of(&v, Capability::FilesRead, Caller::AgentLocal);
    assert_eq!(m.default_mode, GrantMode::Allow);
}

#[test]
fn the_view_uses_the_same_rule_as_the_gate_for_the_effective_mode() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesWrite,
        Caller::AgentExternal,
        Some(GrantMode::Allow),
        2_000,
    )
    .unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 3_000).unwrap();
    let m = mode_of(&v, Capability::FilesWrite, Caller::AgentExternal);
    assert_eq!((m.stored, m.effective), (Some(GrantMode::Allow), GrantMode::Allow));
    // Das Tor entscheidet gleich.
    let req = Request {
        caller: Caller::AgentExternal,
        integration_id: &i.id,
        capability: Capability::FilesWrite,
        target: Some("Notiz.md"),
        args: None,
        tool_mode: None,
    };
    assert_eq!(
        gate::check(&conn, &req, 3_000).unwrap(),
        Decision::Allowed,
        "Ansicht und Tor stimmen ueberein"
    );
}

#[test]
fn a_narrower_direction_switches_the_capability_off_with_a_reason_and_keeps_the_stored_row() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesWrite,
        Caller::Workflow,
        Some(GrantMode::Allow),
        2_000,
    )
    .unwrap();
    update_from_ui(&conn, &i.id, None, None, Some(Direction::Read), 3_000).unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 4_000).unwrap();
    let cap = v
        .capabilities
        .iter()
        .find(|c| c.capability == Capability::FilesWrite)
        .unwrap();
    assert!(!cap.direction_allows);
    let m = mode_of(&v, Capability::FilesWrite, Caller::Workflow);
    assert_eq!(m.effective, GrantMode::Off);
    assert_eq!(m.off_reason.as_deref(), Some("direction_blocks"));
    assert_eq!(m.stored, Some(GrantMode::Allow), "die Zeile bleibt");
    // Wieder auf „beides“: das Recht wirkt wieder.
    update_from_ui(&conn, &i.id, None, None, Some(Direction::Both), 5_000).unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 6_000).unwrap();
    assert_eq!(
        mode_of(&v, Capability::FilesWrite, Caller::Workflow).effective,
        GrantMode::Allow
    );
}

#[test]
fn a_disabled_integration_is_off_for_everyone_with_the_reason() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    update_from_ui(&conn, &i.id, None, Some(false), None, 2_000).unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 3_000).unwrap();
    for c in &v.capabilities {
        for m in &c.modes {
            assert_eq!(m.effective, GrantMode::Off);
            assert_eq!(m.off_reason.as_deref(), Some("integration_disabled"));
        }
    }
}

#[test]
fn recording_start_is_marked_never_allow_and_cannot_be_set_to_allow() {
    let fx = Fx::new();
    let conn = fx.conn();
    let agent = of_kind(&conn, Kind::Agent, "Claude Code");
    let v = get_view(&conn, &agent.id, &no_secrets, 2_000).unwrap();
    let rec = v
        .capabilities
        .iter()
        .find(|c| c.capability == Capability::RecordingStart)
        .unwrap();
    assert!(rec.never_allow && rec.writes);
    assert!(
        v.capabilities
            .iter()
            .filter(|c| c.capability != Capability::RecordingStart)
            .all(|c| !c.never_allow)
    );
    let err = set_grant_from_ui(
        &conn,
        &agent.id,
        Capability::RecordingStart,
        Caller::AgentExternal,
        Some(GrantMode::Allow),
        3_000,
    )
    .unwrap_err();
    assert!(err.to_string().contains("Einwilligungsdialog"));
}

#[test]
fn a_calendar_source_appears_as_a_managed_read_only_integration() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "cal-1", "ics", "Outlook Arbeit", 1_000);
    crate::managers::integrations::adopt::reconcile(&conn).unwrap();
    let all = list_views(&conn, &no_secrets, 2_000).unwrap();
    let cal = all.iter().find(|v| v.integration.id == "cal-1").unwrap();
    assert!(cal.calendar_managed);
    assert_eq!(cal.directions, vec![Direction::Read]);
    assert_eq!(cal.integration.kind, Kind::Ics);
    // Name und Schalter gehoeren dem Kalender.
    let err = update_from_ui(&conn, "cal-1", Some("Neu".into()), None, None, 3_000).unwrap_err();
    assert!(matches!(err, IntegrationError::Managed(_)));
    let err = delete_from_ui(&conn, "cal-1", 3_000).unwrap_err();
    assert!(matches!(err, IntegrationError::Managed(_)));
}

#[test]
fn secret_slots_are_reported_by_the_callback_and_never_with_content() {
    let fx = Fx::new();
    let conn = fx.conn();
    let smtp = of_kind(&conn, Kind::Smtp, "Postfach");
    let v = get_view(&conn, &smtp.id, &|_, _| "missing".to_string(), 2_000).unwrap();
    assert_eq!(
        v.secrets,
        vec![SecretSlotView {
            slot: "password".into(),
            status: "missing".into()
        }]
    );
    let folder = get_view(
        &conn,
        &make_folder(&conn, "F").0.id,
        &|_, _| unreachable!("ein Ordner hat kein Geheimnis"),
        2_000,
    )
    .unwrap();
    assert!(folder.secrets.is_empty());
}

#[test]
fn an_unknown_id_is_not_found() {
    let fx = Fx::new();
    let conn = fx.conn();
    assert!(matches!(
        get_view(&conn, "nope", &no_secrets, 1),
        Err(IntegrationError::NotFound(_))
    ));
    assert!(matches!(
        update_from_ui(&conn, "nope", None, Some(true), None, 1),
        Err(IntegrationError::NotFound(_))
    ));
    assert!(matches!(
        delete_from_ui(&conn, "nope", 1),
        Err(IntegrationError::NotFound(_))
    ));
    assert!(matches!(
        set_grant_from_ui(
            &conn,
            "nope",
            Capability::FilesRead,
            Caller::Workflow,
            None,
            1
        ),
        Err(IntegrationError::NotFound(_))
    ));
}

// ---------------------------------------------------------------------------
// Rechte setzen
// ---------------------------------------------------------------------------

#[test]
fn setting_and_clearing_a_grant_round_trips_and_survives_reopening() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesRead,
        Caller::AgentExternal,
        Some(GrantMode::Ask),
        2_000,
    )
    .unwrap();
    drop(conn);
    drop(fx.store);
    let reopened = MeetingStore::open_at(&fx.db_path).unwrap();
    let conn = reopened.get_connection().unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 3_000).unwrap();
    let m = mode_of(&v, Capability::FilesRead, Caller::AgentExternal);
    assert_eq!((m.stored, m.effective), (Some(GrantMode::Ask), GrantMode::Ask));
    // Zurueck auf die Vorgabe.
    set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesRead,
        Caller::AgentExternal,
        None,
        4_000,
    )
    .unwrap();
    let v = get_view(&conn, &i.id, &no_secrets, 5_000).unwrap();
    let m = mode_of(&v, Capability::FilesRead, Caller::AgentExternal);
    assert_eq!((m.stored, m.effective), (None, GrantMode::Off));
}

#[test]
fn a_grant_for_a_capability_the_kind_does_not_offer_or_for_the_user_is_refused() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    assert!(set_grant_from_ui(
        &conn,
        &i.id,
        Capability::MailSend,
        Caller::Workflow,
        Some(GrantMode::Allow),
        1
    )
    .is_err());
    assert!(set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesRead,
        Caller::User,
        Some(GrantMode::Off),
        1
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// Audit der Nutzeraktionen
// ---------------------------------------------------------------------------

#[test]
fn the_users_changes_stand_in_the_audit_with_caller_user() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesWrite,
        Caller::AgentExternal,
        Some(GrantMode::Ask),
        2_000,
    )
    .unwrap();
    update_from_ui(&conn, &i.id, None, None, Some(Direction::Read), 3_000).unwrap();
    delete_from_ui(&conn, &i.id, 4_000).unwrap();
    let rows = audit_entries(&conn, Some(i.id.clone()), None, None, 50).unwrap();
    let phases: Vec<String> = rows
        .iter()
        .map(|r| {
            let d: Value = serde_json::from_str(r.detail_json.as_deref().unwrap()).unwrap();
            d["phase"].as_str().unwrap().to_string()
        })
        .collect();
    // Neueste zuerst.
    assert_eq!(phases, vec!["deleted", "updated", "grant_changed", "created"]);
    assert!(rows.iter().all(|r| r.caller == "user" && r.outcome == "ok"));
    let grant = &rows[2];
    assert_eq!(grant.capability.as_deref(), Some("files.write"));
    let d: Value = serde_json::from_str(grant.detail_json.as_deref().unwrap()).unwrap();
    assert_eq!((d["for"].as_str(), d["mode"].as_str()), (Some("agent_external"), Some("ask")));
}

#[test]
fn a_rename_that_changes_nothing_writes_no_audit_row() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let before = audit_entries(&conn, None, None, None, 50).unwrap().len();
    update_from_ui(&conn, &i.id, None, None, None, 2_000).unwrap();
    update_from_ui(&conn, &i.id, None, Some(true), Some(Direction::Both), 2_100).unwrap();
    assert_eq!(audit_entries(&conn, None, None, None, 50).unwrap().len(), before);
}

#[test]
fn the_audit_filter_narrows_by_outcome_and_caller() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    // Ein Agent wird verweigert: externer Agent, Standard aus.
    let req = Request {
        caller: Caller::AgentExternal,
        integration_id: &i.id,
        capability: Capability::FilesRead,
        target: Some("a.txt"),
        args: None,
        tool_mode: None,
    };
    gate::check(&conn, &req, 5_000).unwrap();
    let denied = audit_entries(&conn, None, Some("denied".into()), None, 50).unwrap();
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].caller, "agent_external");
    let users = audit_entries(&conn, None, None, Some("user".into()), 50).unwrap();
    assert!(users.iter().all(|r| r.caller == "user"));
    assert!(!users.is_empty());
    // Limit wird eingehalten (1..=500).
    assert_eq!(audit_entries(&conn, None, None, None, 1).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Freigaben
// ---------------------------------------------------------------------------

fn ask_for_write(conn: &Connection, i: &Integration, now_ms: i64) -> String {
    set_grant_from_ui(
        conn,
        &i.id,
        Capability::FilesWrite,
        Caller::AgentExternal,
        Some(GrantMode::Ask),
        now_ms,
    )
    .unwrap();
    let args = json!({ "path": "Notiz.md", "bytes": 12 });
    let req = Request {
        caller: Caller::AgentExternal,
        integration_id: &i.id,
        capability: Capability::FilesWrite,
        target: Some("Notiz.md"),
        args: Some(&args),
        tool_mode: None,
    };
    match gate::check(conn, &req, now_ms).unwrap() {
        Decision::NeedsApproval { approval_id } => approval_id,
        other => panic!("erwartet Freigabe, bekam {other:?}"),
    }
}

#[test]
fn a_pending_approval_is_listed_with_the_integration_name_and_counted_on_its_view() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let id = ask_for_write(&conn, &i, 10_000);
    let pending = pending_approvals(&conn, 11_000).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].approval.id, id);
    assert_eq!(pending[0].integration_label.as_deref(), Some("Ablage"));
    assert_eq!(pending[0].integration_kind, Some(Kind::Folder));
    assert_eq!(pending[0].approval.tool_or_capability, "files.write");
    assert!(pending[0]
        .approval
        .args_preview
        .as_deref()
        .unwrap()
        .contains("Notiz.md"));
    let v = get_view(&conn, &i.id, &no_secrets, 11_000).unwrap();
    assert_eq!(v.pending_approvals, 1);
}

#[test]
fn approving_lets_the_gate_run_the_action_once_and_the_decision_is_audited() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let id = ask_for_write(&conn, &i, 10_000);
    let decided = decide_approval(&conn, &id, true, 12_000).unwrap();
    assert_eq!(decided.state, crate::managers::integrations::model::ApprovalState::Approved);
    assert!(pending_approvals(&conn, 13_000).unwrap().is_empty());
    let args = json!({ "path": "Notiz.md", "bytes": 12 });
    let req = Request {
        caller: Caller::AgentExternal,
        integration_id: &i.id,
        capability: Capability::FilesWrite,
        target: Some("Notiz.md"),
        args: Some(&args),
        tool_mode: None,
    };
    let out = gate::run_approved(&conn, &id, &req, 14_000, || Ok("geschrieben")).unwrap();
    assert_eq!(out, gate::GateOutcome::Done("geschrieben"));
    // Einmalig.
    let again = gate::run_approved(&conn, &id, &req, 15_000, || Ok("noch mal")).unwrap();
    assert!(matches!(again, gate::GateOutcome::Denied { .. }));
    let rows = audit_entries(&conn, Some(i.id.clone()), None, Some("user".into()), 50).unwrap();
    let decision = rows
        .iter()
        .find(|r| r.detail_json.as_deref().is_some_and(|d| d.contains("approval_decided")))
        .expect("Entscheidung im Audit");
    let d: Value = serde_json::from_str(decision.detail_json.as_deref().unwrap()).unwrap();
    assert_eq!(d["decision"], "approved");
    assert_eq!(d["approval_id"], id);
    assert_eq!(d["requested_by"], "agent_external");
}

#[test]
fn denying_blocks_the_action_and_is_audited() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let id = ask_for_write(&conn, &i, 10_000);
    decide_approval(&conn, &id, false, 12_000).unwrap();
    let args = json!({ "path": "Notiz.md", "bytes": 12 });
    let req = Request {
        caller: Caller::AgentExternal,
        integration_id: &i.id,
        capability: Capability::FilesWrite,
        target: Some("Notiz.md"),
        args: Some(&args),
        tool_mode: None,
    };
    let mut ran = false;
    let out = gate::run_approved(&conn, &id, &req, 13_000, || {
        ran = true;
        Ok(())
    })
    .unwrap();
    assert!(!ran, "die Aktion lief trotz Ablehnung");
    assert!(matches!(out, gate::GateOutcome::Denied { .. }));
    let rows = audit_entries(&conn, None, None, Some("user".into()), 50).unwrap();
    assert!(rows.iter().any(|r| r
        .detail_json
        .as_deref()
        .is_some_and(|d| d.contains("\"decision\":\"denied\""))));
}

#[test]
fn deciding_twice_or_after_expiry_or_for_an_unknown_id_returns_a_code() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    let id = ask_for_write(&conn, &i, 10_000);
    decide_approval(&conn, &id, true, 11_000).unwrap();
    assert_eq!(
        decide_approval(&conn, &id, false, 12_000).unwrap_err(),
        "approval_already_decided"
    );
    assert_eq!(
        decide_approval(&conn, "nope", true, 12_000).unwrap_err(),
        "approval_not_found"
    );
    // Verfallen: nach der Frist (eine Stunde) nicht mehr entscheidbar.
    let late = ask_for_write(&conn, &i, 20_000);
    let after_ttl = 20_000 + approvals::TTL_MS + 1;
    assert_eq!(
        decide_approval(&conn, &late, true, after_ttl).unwrap_err(),
        "approval_expired"
    );
    assert!(pending_approvals(&conn, after_ttl).unwrap().is_empty());
}

#[test]
fn pending_approvals_of_a_deleted_integration_still_list_without_a_name() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    ask_for_write(&conn, &i, 10_000);
    delete_from_ui(&conn, &i.id, 11_000).unwrap();
    let pending = pending_approvals(&conn, 12_000).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].integration_label, None);
    assert_eq!(pending[0].integration_kind, None);
}

// ---------------------------------------------------------------------------
// Test der Verbindung und Entfernen
// ---------------------------------------------------------------------------

#[test]
fn testing_a_folder_marks_ok_or_the_error_on_the_entry() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, path) = make_folder(&conn, "Ablage");
    let r = test_integration(&conn, &i.id, 5_000).unwrap();
    assert_eq!((r.ok, r.code.as_str()), (true, "folder_ok"));
    let after = store::get(&conn, &i.id).unwrap().unwrap();
    assert_eq!((after.last_ok_at, after.last_error), (Some(5_000), None));
    // Der Ordner verschwindet.
    std::fs::remove_dir_all(&path).unwrap();
    let r = test_integration(&conn, &i.id, 6_000).unwrap();
    assert_eq!((r.ok, r.code.as_str()), (false, ERR_PATH_NOT_FOUND));
    let after = store::get(&conn, &i.id).unwrap().unwrap();
    assert_eq!(after.last_error.as_deref(), Some("Der Ordner wurde nicht gefunden."));
    // Und kommt wieder: der Fehler verschwindet.
    std::fs::create_dir_all(&path).unwrap();
    assert!(test_integration(&conn, &i.id, 7_000).unwrap().ok);
    assert_eq!(store::get(&conn, &i.id).unwrap().unwrap().last_error, None);
}

#[test]
fn testing_a_kind_without_a_test_says_so_and_changes_nothing() {
    let fx = Fx::new();
    let conn = fx.conn();
    let smtp = of_kind(&conn, Kind::Smtp, "Postfach");
    let r = test_integration(&conn, &smtp.id, 5_000).unwrap();
    assert_eq!((r.ok, r.code.as_str()), (false, "test_not_available"));
    let after = store::get(&conn, &smtp.id).unwrap().unwrap();
    assert_eq!((after.last_ok_at, after.last_error), (None, None));
}

#[test]
fn deleting_removes_the_integration_and_its_grants_but_keeps_the_audit() {
    let fx = Fx::new();
    let conn = fx.conn();
    let (i, _) = make_folder(&conn, "Ablage");
    set_grant_from_ui(
        &conn,
        &i.id,
        Capability::FilesRead,
        Caller::Workflow,
        Some(GrantMode::Ask),
        2_000,
    )
    .unwrap();
    let gone = delete_from_ui(&conn, &i.id, 3_000).unwrap();
    assert_eq!(gone.id, i.id);
    assert!(list_views(&conn, &no_secrets, 4_000).unwrap().is_empty());
    assert!(store::list_grants(&conn, &i.id).unwrap().is_empty());
    assert!(!audit_entries(&conn, Some(i.id), None, None, 50).unwrap().is_empty());
}
