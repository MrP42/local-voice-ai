use std::collections::HashSet;

use super::*;
use crate::agent_bridge::catalog;
use crate::managers::integrations::audit::AuditFilter;
use crate::managers::integrations::model::Caller;
use crate::managers::integrations::store;
use crate::managers::integrations::test_support::Fx;

const NOW: i64 = 1_790_000_000_000;

fn avail(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn tool<'a>(v: &'a AgentClientView, name: &str) -> &'a ToolView {
    v.tools.iter().find(|t| t.name == name).unwrap()
}

#[test]
fn a_new_client_lists_every_catalog_tool_switched_off() {
    let fx = Fx::new();
    let conn = fx.conn();
    let made = create_client(&conn, "Claude Code", None, NOW).unwrap();
    assert!(made.token.starts_with("lvat_"));
    let views = client_views(&conn, &avail(&["transcribe_file"])).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].tools.len(), catalog::CATALOG.len());
    for t in &views[0].tools {
        assert_eq!(t.client_mode, GrantMode::Off, "{}", t.name);
        assert_eq!(t.effective_mode, GrantMode::Off, "{}", t.name);
        assert!(t.off_reason.is_some(), "{}", t.name);
    }
    assert!(tool(&views[0], "transcribe_file").available);
    assert!(!tool(&views[0], "create_meeting").available);
}

#[test]
fn the_view_shows_the_effective_mode_and_why_a_tool_is_still_off() {
    let fx = Fx::new();
    let conn = fx.conn();
    let made = create_client(&conn, "Claude Code", None, NOW).unwrap();
    let id = made.client.id.clone();
    let none = avail(&[]);
    // Das Recht des Zugangs allein reicht nicht: die Obergrenze der Integration steht auf „aus“.
    let v = set_tool_mode(&conn, &id, "transcribe_file", GrantMode::Ask, &none, NOW).unwrap();
    let t = tool(&v, "transcribe_file");
    assert_eq!(t.client_mode, GrantMode::Ask);
    assert_eq!(t.effective_mode, GrantMode::Off);
    assert_eq!(t.off_reason.as_deref(), Some("grant_off"));
    // Obergrenze auf „erlaubt“: jetzt gilt das Recht des Zugangs.
    store::set_grant(
        &conn,
        &made.client.integration_id,
        Capability::TranscribeFile,
        Caller::AgentExternal,
        GrantMode::Allow,
    )
    .unwrap();
    let v = view_of(&conn, &made.client, &none).unwrap();
    let t = tool(&v, "transcribe_file");
    assert_eq!((t.client_mode, t.effective_mode), (GrantMode::Ask, GrantMode::Ask));
    assert_eq!(t.off_reason, None);
    // Aufnahme: nie erlaubt, auch wenn alles auf „erlaubt“ steht, hoechstens „fragen“.
    assert!(set_tool_mode(&conn, &id, "start_recording", GrantMode::Allow, &none, NOW).is_err());
}

#[test]
fn creating_revoking_and_rights_changes_are_audited_as_user_actions() {
    let fx = Fx::new();
    let conn = fx.conn();
    let made = create_client(&conn, "Claude Code", None, NOW).unwrap();
    set_tool_mode(&conn, &made.client.id, "transcribe_file", GrantMode::Ask, &avail(&[]), NOW + 1).unwrap();
    revoke_client(&conn, &made.client.id, NOW + 2).unwrap();
    // Zweites Zurueckziehen: keine zweite Zeile.
    revoke_client(&conn, &made.client.id, NOW + 3).unwrap();
    let rows = audit::list(&conn, &AuditFilter::default(), 100).unwrap();
    let phases: Vec<String> = rows
        .iter()
        .rev()
        .map(|r| {
            let d: serde_json::Value = serde_json::from_str(r.detail_json.as_deref().unwrap()).unwrap();
            assert_eq!(r.caller, "user");
            assert!(r.target.as_deref().unwrap().contains(&made.client.id));
            d["phase"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(phases, vec!["agent_client_created", "agent_tool_mode", "agent_client_revoked"]);
    // Das Token steht nirgends im Audit.
    for r in &rows {
        assert!(!format!("{r:?}").contains(&made.token));
    }
}

#[test]
fn revoked_clients_stay_in_the_list_marked_as_revoked_and_deleting_removes_them() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = create_client(&conn, "Alt", None, NOW).unwrap().client;
    let b = create_client(&conn, "Neu", None, NOW + 1).unwrap().client;
    revoke_client(&conn, &a.id, NOW + 2).unwrap();
    let views = client_views(&conn, &avail(&[])).unwrap();
    assert_eq!(views.len(), 2);
    assert!(views[0].client.revoked_at.is_some());
    assert!(views[1].client.revoked_at.is_none());
    delete_client(&conn, &a.id, NOW + 3).unwrap();
    let views = client_views(&conn, &avail(&[])).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].client.id, b.id);
    assert!(matches!(delete_client(&conn, &a.id, NOW + 4), Err(IntegrationError::NotFound(_))));
    assert!(matches!(revoke_client(&conn, "gibt-es-nicht", NOW), Err(IntegrationError::NotFound(_))));
}
