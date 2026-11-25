//! Rein synthetische Graph-Antworten, keine Sockets.
use super::polls::*;
use serde_json::json;

#[test]
fn schedule_partial_error_and_missing_person_are_unknown() {
    let window = ScheduleWindow { start: "2026-12-01T09:00:00".into(), end: "2026-12-01T10:00:00".into(), time_zone: "Europe/Berlin".into() };
    let addresses = vec!["one@example.invalid".into(), "two@example.invalid".into(), "three@example.invalid".into()];
    let out = parse_schedule(&json!({"value":[
        {"scheduleId":"one@example.invalid","availabilityView":"02","subject":"NEVER RETAIN"},
        {"scheduleId":"two@example.invalid","availabilityView":"00","error":{"message":"PRIVATE"}}
    ]}), &addresses, &window).unwrap();
    assert_eq!(out[0].intervals, vec![Availability::Free, Availability::Busy]);
    assert_eq!(out[1].intervals, vec![Availability::Unknown; 2]);
    assert_eq!(out[2].intervals, vec![Availability::Unknown; 2]);
    assert!(!serde_json::to_string(&out).unwrap().contains("PRIVATE"));
}

#[test]
fn delta_page_requires_one_valid_cursor_and_rejects_foreign_links() {
    let base = "https://graph.microsoft.com/v1.0";
    let link = format!("{base}/me/mailFolders/inbox/messages/delta?$skiptoken=SECRET");
    let page = parse_delta(json!({"value":[],"@odata.nextLink":link}), base).unwrap();
    assert!(!page.finished);
    let page = parse_delta(json!({"value":[],"@odata.deltaLink":link}), base).unwrap();
    assert!(page.finished);
    for bad in ["https://foreign.invalid/v1.0/me/mailFolders/inbox/messages/delta?x=y", "https://graph.microsoft.com/v1.0/me/sendMail?x=y", "https://user@graph.microsoft.com/v1.0/me/mailFolders/inbox/messages/delta?x=y"] {
        assert!(validate_delta_link(base, bad).is_err());
    }
    assert!(parse_delta(json!({"value":[]}), base).is_err());
}

#[test]
fn new_scopes_are_opt_in_and_independently_removable() {
    use super::config::*;
    use crate::managers::integrations::model::Capability;
    let mut cfg = M365Config::from_json("{}").unwrap();
    assert_eq!(cfg.required_scopes(), BASE_SCOPES.map(str::to_string).to_vec());
    cfg.capabilities.push(Capability::Verfuegbarkeit);
    assert!(cfg.required_scopes().contains(&"Calendars.ReadBasic".into()));
    assert!(!cfg.required_scopes().contains(&"Mail.Read".into()));
    cfg.capabilities = vec![Capability::AntwortenLesen];
    assert!(!cfg.required_scopes().contains(&"Calendars.ReadBasic".into()));
    assert!(cfg.required_scopes().contains(&"Mail.Read".into()));
}
