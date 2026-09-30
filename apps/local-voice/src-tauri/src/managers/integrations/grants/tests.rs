use super::*;
use crate::managers::integrations::model::{Access, Direction, Kind};

fn integration(kind: Kind, direction: Direction, enabled: bool) -> Integration {
    Integration {
        id: "i-1".into(),
        kind,
        label: "Test".into(),
        enabled,
        direction,
        config_json: "{}".into(),
        account_hint: None,
        data_class: None,
        created_at: 0,
        updated_at: 0,
        last_ok_at: None,
        last_error: None,
    }
}

fn set(entries: &[(Capability, Caller, GrantMode)]) -> GrantSet {
    entries.iter().map(|(c, k, m)| ((*c, *k), *m)).collect()
}

#[test]
fn the_order_off_ask_allow_makes_the_strictest_win_as_min() {
    assert!(GrantMode::Off < GrantMode::Ask && GrantMode::Ask < GrantMode::Allow);
    assert_eq!(GrantMode::Allow.min(GrantMode::Ask), GrantMode::Ask);
    assert_eq!(GrantMode::Ask.min(GrantMode::Off), GrantMode::Off);
}

#[test]
fn a_disabled_integration_is_off_for_everyone_including_the_user() {
    let i = integration(Kind::Folder, Direction::Both, false);
    for caller in [
        Caller::User,
        Caller::Workflow,
        Caller::AgentExternal,
        Caller::AgentLocal,
    ] {
        let (mode, why) = explain(&i, Capability::FilesRead, caller, &GrantSet::new(), None);
        assert_eq!(mode, GrantMode::Off, "{caller:?}");
        assert_eq!(why, Some(OffReason::IntegrationDisabled));
    }
}

#[test]
fn a_capability_the_kind_does_not_offer_is_off() {
    // Ein SMTP-Postfach kann keine Dateien schreiben.
    let i = integration(Kind::Smtp, Direction::Write, true);
    let (mode, why) = explain(
        &i,
        Capability::FilesWrite,
        Caller::User,
        &GrantSet::new(),
        None,
    );
    assert_eq!(
        (mode, why),
        (GrantMode::Off, Some(OffReason::CapabilityNotOffered))
    );
}

#[test]
fn the_direction_must_permit_the_access_kind() {
    let read_only = integration(Kind::Folder, Direction::Read, true);
    let write_only = integration(Kind::Folder, Direction::Write, true);
    let grants = set(&[
        (Capability::FilesWrite, Caller::Workflow, GrantMode::Allow),
        (Capability::FilesRead, Caller::Workflow, GrantMode::Allow),
    ]);
    // Schreiben bei nur lesender Richtung: aus, trotz „erlaubt“.
    assert_eq!(
        explain(
            &read_only,
            Capability::FilesWrite,
            Caller::Workflow,
            &grants,
            None
        ),
        (GrantMode::Off, Some(OffReason::DirectionBlocks))
    );
    assert_eq!(
        effective_mode(
            &read_only,
            Capability::FilesRead,
            Caller::Workflow,
            &grants,
            None
        ),
        GrantMode::Allow
    );
    // Lesen bei nur schreibender Richtung: aus.
    assert_eq!(
        effective_mode(
            &write_only,
            Capability::FilesRead,
            Caller::Workflow,
            &grants,
            None
        ),
        GrantMode::Off
    );
    assert!(Direction::Both.permits(Access::Read) && Direction::Both.permits(Access::Write));
}

#[test]
fn the_user_in_the_ui_needs_no_grant() {
    let i = integration(Kind::M365, Direction::Both, true);
    for cap in Kind::M365.capabilities() {
        assert_eq!(
            effective_mode(&i, *cap, Caller::User, &GrantSet::new(), None),
            GrantMode::Allow,
            "{cap:?}"
        );
    }
}

#[test]
fn external_agents_default_to_off_for_everything() {
    let i = integration(Kind::M365, Direction::Both, true);
    for cap in Kind::M365.capabilities() {
        let (mode, why) = explain(&i, *cap, Caller::AgentExternal, &GrantSet::new(), None);
        assert_eq!(mode, GrantMode::Off, "{cap:?}");
        assert_eq!(why, Some(OffReason::GrantOff));
    }
}

#[test]
fn workflow_and_local_agent_read_freely_and_ask_before_writing() {
    let i = integration(Kind::M365, Direction::Both, true);
    for caller in [Caller::Workflow, Caller::AgentLocal] {
        for cap in Kind::M365.capabilities() {
            let expected = match cap.access() {
                Access::Read => GrantMode::Allow,
                Access::Write => GrantMode::Ask,
            };
            assert_eq!(
                effective_mode(&i, *cap, caller, &GrantSet::new(), None),
                expected,
                "{caller:?} {cap:?}"
            );
        }
    }
}

#[test]
fn fetching_a_youtube_file_is_never_on_by_default() {
    let i = integration(Kind::Youtube, Direction::Read, true);
    for caller in [Caller::Workflow, Caller::AgentExternal, Caller::AgentLocal] {
        assert_eq!(
            effective_mode(&i, Capability::MediaFetch, caller, &GrantSet::new(), None),
            GrantMode::Off,
            "{caller:?}"
        );
    }
    // Der Nutzer darf; und wer es einschaltet, bekommt es.
    assert_eq!(
        effective_mode(
            &i,
            Capability::MediaFetch,
            Caller::User,
            &GrantSet::new(),
            None
        ),
        GrantMode::Allow
    );
    let on = set(&[(Capability::MediaFetch, Caller::Workflow, GrantMode::Ask)]);
    assert_eq!(
        effective_mode(&i, Capability::MediaFetch, Caller::Workflow, &on, None),
        GrantMode::Ask
    );
}

#[test]
fn a_stored_grant_overrides_the_default_in_both_directions() {
    let i = integration(Kind::M365, Direction::Both, true);
    let grants = set(&[
        (Capability::MailSend, Caller::AgentExternal, GrantMode::Ask),
        (Capability::CalendarRead, Caller::Workflow, GrantMode::Off),
        (Capability::FilesWrite, Caller::Workflow, GrantMode::Allow),
    ]);
    let mode = |cap, caller| effective_mode(&i, cap, caller, &grants, None);
    assert_eq!(
        mode(Capability::MailSend, Caller::AgentExternal),
        GrantMode::Ask
    );
    assert_eq!(
        mode(Capability::CalendarRead, Caller::Workflow),
        GrantMode::Off
    );
    assert_eq!(
        mode(Capability::FilesWrite, Caller::Workflow),
        GrantMode::Allow
    );
    // Andere Aufrufer derselben Faehigkeit bleiben bei ihrer Vorgabe.
    assert_eq!(mode(Capability::MailSend, Caller::Workflow), GrantMode::Ask);
    assert_eq!(
        mode(Capability::MailSend, Caller::AgentLocal),
        GrantMode::Ask
    );
}

#[test]
fn the_tool_level_can_only_tighten_never_loosen() {
    let i = integration(Kind::Folder, Direction::Both, true);
    let grants = set(&[(
        Capability::FilesWrite,
        Caller::AgentExternal,
        GrantMode::Allow,
    )]);
    let with = |tool| {
        effective_mode(
            &i,
            Capability::FilesWrite,
            Caller::AgentExternal,
            &grants,
            tool,
        )
    };
    assert_eq!(with(None), GrantMode::Allow);
    assert_eq!(with(Some(GrantMode::Allow)), GrantMode::Allow);
    assert_eq!(with(Some(GrantMode::Ask)), GrantMode::Ask);
    let (mode, why) = explain(
        &i,
        Capability::FilesWrite,
        Caller::AgentExternal,
        &grants,
        Some(GrantMode::Off),
    );
    assert_eq!((mode, why), (GrantMode::Off, Some(OffReason::ToolOff)));
    // Ein lockeres Werkzeugrecht hebt ein strenges Integrationsrecht nicht an.
    let strict = set(&[(
        Capability::FilesWrite,
        Caller::AgentExternal,
        GrantMode::Off,
    )]);
    assert_eq!(
        effective_mode(
            &i,
            Capability::FilesWrite,
            Caller::AgentExternal,
            &strict,
            Some(GrantMode::Allow)
        ),
        GrantMode::Off
    );
}

#[test]
fn recording_can_never_be_allowed_permanently() {
    let i = integration(Kind::Agent, Direction::Both, true);
    for caller in [Caller::Workflow, Caller::AgentExternal, Caller::AgentLocal] {
        // Selbst ein (von Hand in die Datenbank geschriebenes) `allow` wird zu `ask`.
        let grants = set(&[(Capability::RecordingStart, caller, GrantMode::Allow)]);
        assert_eq!(
            effective_mode(&i, Capability::RecordingStart, caller, &grants, None),
            GrantMode::Ask,
            "{caller:?}"
        );
    }
    assert!(Capability::RecordingStart.never_allow());
    assert!(!Capability::MailSend.never_allow());
}

#[test]
fn every_kind_offers_only_known_capabilities_and_a_permitted_default_direction() {
    for kind in Kind::ALL {
        assert!(
            kind.allowed_directions()
                .contains(&kind.default_direction()),
            "{kind:?}"
        );
        for cap in kind.capabilities() {
            assert!(Capability::ALL.contains(cap), "{kind:?} {cap:?}");
        }
        // Mindestens eine Faehigkeit ist mit der Standardrichtung nutzbar.
        assert!(
            kind.capabilities()
                .iter()
                .any(|c| kind.default_direction().permits(c.access())),
            "{kind:?} hat mit der Standardrichtung keine nutzbare Faehigkeit"
        );
    }
}

#[test]
fn names_round_trip_through_parse_for_every_enum() {
    for k in Kind::ALL {
        assert_eq!(Kind::parse(k.as_str()), Some(k));
    }
    for c in Capability::ALL {
        assert_eq!(Capability::parse(c.as_str()), Some(c));
        // Die serde-Schreibweise ist die mit Punkt, wie im Audit.
        assert_eq!(
            serde_json::to_value(c).unwrap(),
            serde_json::json!(c.as_str())
        );
    }
    for m in [GrantMode::Off, GrantMode::Ask, GrantMode::Allow] {
        assert_eq!(GrantMode::parse(m.as_str()), Some(m));
    }
    for d in [Direction::Read, Direction::Write, Direction::Both] {
        assert_eq!(Direction::parse(d.as_str()), Some(d));
    }
    assert_eq!(Kind::parse("nope"), None);
    assert_eq!(Capability::parse("files"), None);
}

#[test]
fn off_reasons_have_distinct_codes_and_german_messages() {
    let all = [
        OffReason::IntegrationDisabled,
        OffReason::CapabilityNotOffered,
        OffReason::DirectionBlocks,
        OffReason::GrantOff,
        OffReason::ToolOff,
    ];
    let mut codes: Vec<&str> = all.iter().map(|r| r.as_str()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
    assert!(all.iter().all(|r| r.message().ends_with('.')));
}
