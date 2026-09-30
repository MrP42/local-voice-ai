use super::*;
use crate::managers::integrations::test_support::{add_calendar_source, folder, of_kind, Fx};
use serde_json::json;

#[test]
fn create_get_and_list_round_trip_with_the_defaults_of_the_kind() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut n = NewIntegration::new(Kind::Obsidian, "  Mein Vault  ");
    n.config = json!({ "path": "C:/Vault", "subfolder": "Besprechungen" });
    n.account_hint = Some("Vault lokal".into());
    n.data_class = Some("confidential".into());
    let created = create(&conn, &n, 5_000).unwrap();
    assert_eq!(created.label, "Mein Vault", "Name getrimmt");
    assert_eq!(created.kind, Kind::Obsidian);
    assert_eq!(created.direction, Direction::Both, "Vorgabe der Art");
    assert!(created.enabled);
    assert_eq!((created.created_at, created.updated_at), (5_000, 5_000));
    assert_eq!(created.data_class.as_deref(), Some("confidential"));
    assert_eq!(created.id.len(), 26, "ULID");
    let cfg: Value = serde_json::from_str(&created.config_json).unwrap();
    assert_eq!(cfg["subfolder"], "Besprechungen");

    assert_eq!(get(&conn, &created.id).unwrap().unwrap(), created);
    assert!(get(&conn, "gibt-es-nicht").unwrap().is_none());
    let second = folder(&conn, "Ablage");
    let all = list(&conn).unwrap();
    // Nach Anlagezeit, aelteste zuerst: `folder` legt bei 1000 an, die Vault bei 5000.
    assert_eq!(
        all.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        vec![second.id, created.id]
    );
}

#[test]
fn calendar_kinds_are_created_by_the_calendar_not_the_register() {
    let fx = Fx::new();
    for kind in [Kind::Ics, Kind::Graph] {
        let err = create(&fx.conn(), &NewIntegration::new(kind, "Kalender"), 1).unwrap_err();
        assert!(matches!(err, IntegrationError::Managed(_)), "{kind:?}");
    }
    assert!(list(&fx.conn()).unwrap().is_empty());
}

#[test]
fn a_direction_the_kind_does_not_know_is_refused() {
    let fx = Fx::new();
    let conn = fx.conn();
    // SMTP kann nur senden, YouTube nur lesen.
    let mut n = NewIntegration::new(Kind::Smtp, "Postfach");
    n.direction = Some(Direction::Read);
    assert!(matches!(
        create(&conn, &n, 1),
        Err(IntegrationError::Invalid(_))
    ));
    let mut n = NewIntegration::new(Kind::Youtube, "YouTube");
    n.direction = Some(Direction::Both);
    assert!(matches!(
        create(&conn, &n, 1),
        Err(IntegrationError::Invalid(_))
    ));
    assert_eq!(
        create(&conn, &NewIntegration::new(Kind::Smtp, "Postfach"), 1)
            .unwrap()
            .direction,
        Direction::Write
    );
}

#[test]
fn labels_must_be_present_and_bounded() {
    let fx = Fx::new();
    let conn = fx.conn();
    for bad in ["", "   ", &"x".repeat(MAX_LABEL_CHARS + 1)] {
        assert!(
            matches!(
                create(&conn, &NewIntegration::new(Kind::Folder, bad), 1),
                Err(IntegrationError::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    create(
        &conn,
        &NewIntegration::new(Kind::Folder, &"ä".repeat(MAX_LABEL_CHARS)),
        1,
    )
    .unwrap();
}

#[test]
fn a_configuration_with_a_secret_is_refused() {
    let fx = Fx::new();
    let conn = fx.conn();
    let bad = [
        json!({ "host": "smtp.example.invalid", "password": "hunter2" }),
        json!({ "account": { "apiKey": "k" } }),
        json!({ "list": [ { "refresh_token": "t" } ] }),
        json!({ "url": "https://anna:geheim@mail.example/" }),
        json!({ "header": "Bearer abcdef0123456789" }),
        json!({ "jwt": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig" }),
        json!({ "Authorization": "x" }),
    ];
    for config in bad {
        let mut n = NewIntegration::new(Kind::Folder, "x");
        n.config = config.clone();
        let err = create(&conn, &n, 1).unwrap_err();
        assert!(
            matches!(&err, IntegrationError::Invalid(m) if m.contains("Geheimnis")),
            "{config} -> {err}"
        );
    }
    assert!(list(&conn).unwrap().is_empty(), "nichts gespeichert");
}

#[test]
fn a_clean_configuration_is_accepted_but_not_a_non_object_or_a_huge_one() {
    let fx = Fx::new();
    let conn = fx.conn();
    let mut n = NewIntegration::new(Kind::Folder, "ok");
    n.config = json!({ "path": "C:/Users/x/OneDrive/Besprechungen", "url": "https://example.invalid/pfad?seite=2", "limit": 3 });
    create(&conn, &n, 1).unwrap();
    for config in [json!([1, 2]), json!("text"), json!(null)] {
        let mut n = NewIntegration::new(Kind::Folder, "x");
        n.config = config;
        assert!(matches!(
            create(&conn, &n, 1),
            Err(IntegrationError::Invalid(_))
        ));
    }
    let mut n = NewIntegration::new(Kind::Folder, "gross");
    n.config = json!({ "blob": "x".repeat(MAX_CONFIG_BYTES) });
    assert!(matches!(
        create(&conn, &n, 1),
        Err(IntegrationError::Invalid(_))
    ));
    // Tief verschachtelt ist auch abgelehnt.
    let mut deep = json!("blatt");
    for _ in 0..12 {
        deep = json!({ "a": deep });
    }
    assert!(validate_config(&deep).is_err());
}

#[test]
fn ids_must_be_safe_and_unique() {
    let fx = Fx::new();
    let conn = fx.conn();
    for bad in ["", "a b", "../x", "a/b", "ä", &"x".repeat(MAX_ID_LEN + 1)] {
        assert!(!valid_id(bad), "{bad:?}");
        let mut n = NewIntegration::new(Kind::Folder, "x");
        n.id = Some(bad.to_string());
        assert!(create(&conn, &n, 1).is_err(), "{bad:?}");
    }
    let mut n = NewIntegration::new(Kind::Folder, "eins");
    n.id = Some("mein-ordner_1".into());
    create(&conn, &n, 1).unwrap();
    let err = create(&conn, &n, 2).unwrap_err();
    assert!(err.to_string().contains("schon vergeben"), "{err}");
    assert_eq!(list(&conn).unwrap().len(), 1);
}

#[test]
fn update_changes_only_what_is_given_and_validates() {
    let fx = Fx::new();
    let conn = fx.conn();
    let i = folder(&conn, "Ablage");
    let patched = update(
        &conn,
        &i.id,
        &IntegrationPatch {
            label: Some("Neuer Name".into()),
            enabled: Some(false),
            direction: Some(Direction::Read),
            config: Some(json!({ "path": "D:/Neu" })),
            data_class: Some(Some("internal".into())),
        },
        9_000,
    )
    .unwrap();
    assert_eq!(patched.label, "Neuer Name");
    assert!(!patched.enabled);
    assert_eq!(patched.direction, Direction::Read);
    assert_eq!(patched.updated_at, 9_000);
    assert_eq!(patched.created_at, i.created_at);
    assert_eq!(patched.data_class.as_deref(), Some("internal"));
    // Leerer Patch aendert nichts ausser der Zeit.
    let same = update(&conn, &i.id, &IntegrationPatch::default(), 9_500).unwrap();
    assert_eq!(
        (same.label.as_str(), same.enabled, same.direction),
        ("Neuer Name", false, Direction::Read)
    );
    // Ungueltiges wird abgelehnt und aendert nichts.
    assert!(update(
        &conn,
        &i.id,
        &IntegrationPatch {
            label: Some(String::new()),
            ..Default::default()
        },
        1
    )
    .is_err());
    assert!(update(
        &conn,
        &i.id,
        &IntegrationPatch {
            config: Some(json!({ "token": "x" })),
            ..Default::default()
        },
        1
    )
    .is_err());
    assert_eq!(get(&conn, &i.id).unwrap().unwrap().label, "Neuer Name");
    assert!(matches!(
        update(&conn, "nope", &IntegrationPatch::default(), 1),
        Err(IntegrationError::NotFound(_))
    ));
    // data_class wieder leeren.
    let cleared = update(
        &conn,
        &i.id,
        &IntegrationPatch {
            data_class: Some(None),
            ..Default::default()
        },
        2,
    )
    .unwrap();
    assert_eq!(cleared.data_class, None);
}

#[test]
fn calendar_managed_integrations_keep_name_and_switch_in_the_calendar() {
    let fx = Fx::new();
    let conn = fx.conn();
    add_calendar_source(&conn, "src-1", "ics", "Outlook", 100);
    // Das Register hat sie gespiegelt.
    let i = get(&conn, "src-1").unwrap().unwrap();
    assert_eq!(i.kind, Kind::Ics);
    for patch in [
        IntegrationPatch {
            label: Some("Anders".into()),
            ..Default::default()
        },
        IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            update(&conn, "src-1", &patch, 1),
            Err(IntegrationError::Managed(_))
        ));
    }
    assert!(matches!(
        delete(&conn, "src-1"),
        Err(IntegrationError::Managed(_))
    ));
    // Richtung und Konfiguration gehoeren dem Register.
    let ok = update(
        &conn,
        "src-1",
        &IntegrationPatch {
            config: Some(json!({ "zone": "Europe/Berlin" })),
            ..Default::default()
        },
        2,
    )
    .unwrap();
    assert!(ok.config_json.contains("Europe/Berlin"));
    // ICS kennt nur „lesen“.
    assert!(update(
        &conn,
        "src-1",
        &IntegrationPatch {
            direction: Some(Direction::Both),
            ..Default::default()
        },
        3
    )
    .is_err());
}

#[test]
fn delete_removes_the_integration_and_its_grants_together() {
    let fx = Fx::new();
    let conn = fx.conn();
    let i = folder(&conn, "Ablage");
    let keep = folder(&conn, "Andere");
    set_grant(
        &conn,
        &i.id,
        Capability::FilesWrite,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    set_grant(
        &conn,
        &keep.id,
        Capability::FilesWrite,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();
    delete(&conn, &i.id).unwrap();
    assert!(get(&conn, &i.id).unwrap().is_none());
    assert!(list_grants(&conn, &i.id).unwrap().is_empty());
    assert_eq!(
        list_grants(&conn, &keep.id).unwrap().len(),
        1,
        "fremde Rechte bleiben"
    );
    assert!(matches!(
        delete(&conn, &i.id),
        Err(IntegrationError::NotFound(_))
    ));
}

#[test]
fn set_grant_validates_caller_capability_and_recording() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let agent = of_kind(&conn, Kind::Agent, "Claude Code");
    // Nutzer hat keine Rechte.
    assert!(set_grant(
        &conn,
        &f.id,
        Capability::FilesRead,
        Caller::User,
        GrantMode::Off
    )
    .is_err());
    // Die Art bietet die Faehigkeit nicht an.
    assert!(set_grant(
        &conn,
        &f.id,
        Capability::MailSend,
        Caller::Workflow,
        GrantMode::Allow
    )
    .is_err());
    // Unbekannte Integration.
    assert!(matches!(
        set_grant(
            &conn,
            "nope",
            Capability::FilesRead,
            Caller::Workflow,
            GrantMode::Allow
        ),
        Err(IntegrationError::NotFound(_))
    ));
    // Aufnahme nie erlaubt, aber fragen/aus gehen.
    let err = set_grant(
        &conn,
        &agent.id,
        Capability::RecordingStart,
        Caller::AgentExternal,
        GrantMode::Allow,
    )
    .unwrap_err();
    assert!(err.to_string().contains("Einwilligungsdialog"), "{err}");
    set_grant(
        &conn,
        &agent.id,
        Capability::RecordingStart,
        Caller::AgentExternal,
        GrantMode::Ask,
    )
    .unwrap();
    set_grant(
        &conn,
        &agent.id,
        Capability::RecordingStart,
        Caller::AgentLocal,
        GrantMode::Off,
    )
    .unwrap();
}

#[test]
fn grants_upsert_clear_and_feed_effective() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    let eff = |caller| effective(&conn, &f.id, Capability::FilesWrite, caller, None).unwrap();
    assert_eq!(eff(Caller::Workflow), GrantMode::Ask, "Vorgabe");
    set_grant(
        &conn,
        &f.id,
        Capability::FilesWrite,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    assert_eq!(eff(Caller::Workflow), GrantMode::Allow);
    set_grant(
        &conn,
        &f.id,
        Capability::FilesWrite,
        Caller::Workflow,
        GrantMode::Off,
    )
    .unwrap();
    assert_eq!(eff(Caller::Workflow), GrantMode::Off);
    assert_eq!(
        list_grants(&conn, &f.id).unwrap().len(),
        1,
        "Upsert, keine Dublette"
    );
    assert!(clear_grant(&conn, &f.id, Capability::FilesWrite, Caller::Workflow).unwrap());
    assert!(!clear_grant(&conn, &f.id, Capability::FilesWrite, Caller::Workflow).unwrap());
    assert_eq!(
        eff(Caller::Workflow),
        GrantMode::Ask,
        "zurueck auf die Vorgabe"
    );
    // Eine unbekannte Integration ist aus, nicht ein Fehler.
    assert_eq!(
        effective(&conn, "nope", Capability::FilesRead, Caller::User, None).unwrap(),
        GrantMode::Off
    );
    // Ausgeschaltet schlaegt jedes Recht.
    set_grant(
        &conn,
        &f.id,
        Capability::FilesRead,
        Caller::Workflow,
        GrantMode::Allow,
    )
    .unwrap();
    update(
        &conn,
        &f.id,
        &IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        2,
    )
    .unwrap();
    assert_eq!(
        effective(&conn, &f.id, Capability::FilesRead, Caller::Workflow, None).unwrap(),
        GrantMode::Off
    );
}

#[test]
fn unknown_values_in_a_grant_row_are_skipped_and_fall_back_to_the_strict_default() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    // Eine Zeile aus einer neueren Version mit unbekannter Faehigkeit.
    conn.execute(
        "INSERT INTO integration_grants VALUES (?1, 'files.teleport', 'agent_external', 'allow')",
        params![f.id],
    )
    .unwrap();
    assert!(list_grants(&conn, &f.id).unwrap().is_empty());
    assert_eq!(
        effective(
            &conn,
            &f.id,
            Capability::FilesWrite,
            Caller::AgentExternal,
            None
        )
        .unwrap(),
        GrantMode::Off
    );
}

#[test]
fn the_database_itself_refuses_bad_modes_callers_and_directions() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    for sql in [
        "INSERT INTO integration_grants VALUES (?1, 'files.read', 'workflow', 'maybe')",
        "INSERT INTO integration_grants VALUES (?1, 'files.read', 'user', 'allow')",
        "UPDATE integrations SET direction = 'sideways' WHERE id = ?1",
    ] {
        assert!(conn.execute(sql, params![f.id]).is_err(), "{sql}");
    }
}

#[test]
fn mark_ok_and_mark_error_keep_the_status_without_secrets() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    mark_error(
        &conn,
        &f.id,
        "Anmeldung fehlgeschlagen: https://anna:geheim@mail.example password=hunter2",
        4_000,
    )
    .unwrap();
    let broken = get(&conn, &f.id).unwrap().unwrap();
    let err = broken.last_error.unwrap();
    assert!(!err.contains("geheim") && !err.contains("hunter2"), "{err}");
    assert_eq!(broken.updated_at, 4_000);
    mark_ok(&conn, &f.id, 5_000).unwrap();
    let ok = get(&conn, &f.id).unwrap().unwrap();
    assert_eq!((ok.last_ok_at, ok.last_error), (Some(5_000), None));
    // Lange Fehler werden gekuerzt.
    mark_error(&conn, &f.id, &"x".repeat(5_000), 6_000).unwrap();
    assert!(
        get(&conn, &f.id)
            .unwrap()
            .unwrap()
            .last_error
            .unwrap()
            .chars()
            .count()
            <= MAX_ERROR_CHARS + 1
    );
}

// -- A1n: Haertung nach dem Sicherheits-Review -----------------------------------

#[test]
fn list_skips_a_row_of_an_unknown_kind_instead_of_failing_as_a_whole() {
    let fx = Fx::new();
    let conn = fx.conn();
    let a = folder(&conn, "Erste");
    // Eine Zeile aus einer neueren Version: die Spalte `kind` hat keine CHECK-Regel.
    conn.execute(
        "INSERT INTO integrations (id, kind, label, enabled, direction, config_json,
                                   created_at, updated_at)
         VALUES ('zukunft-1', 'hologramm', 'Neu', 1, 'read', '{}', 5000, 5000)",
        [],
    )
    .unwrap();
    let b = folder(&conn, "Zweite");
    let listed = list(&conn).unwrap();
    let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec![a.id.as_str(), b.id.as_str()], "{ids:?}");
    // Die einzelne Zeile selbst wird nie geraten: `get` meldet den Fehler.
    assert!(get(&conn, "zukunft-1").is_err());
    // Auch eine unbekannte Richtung zieht nur ihre Zeile heraus.
    conn.execute(
        "UPDATE integrations SET kind = 'folder' WHERE id = 'zukunft-1'",
        [],
    )
    .unwrap();
    assert_eq!(list(&conn).unwrap().len(), 3);
}

#[test]
fn an_id_that_belongs_to_a_calendar_source_cannot_be_taken_by_the_register() {
    let fx = Fx::new();
    let conn = fx.conn();
    crate::managers::integrations::test_support::add_calendar_source(
        &conn, "kal-1", "ics", "Kalender", 1_000,
    );
    let mut n = NewIntegration::new(Kind::Folder, "Ordner");
    n.id = Some("kal-1".into());
    // Solange die Quelle lebt, sperrt schon der Spiegel die Kennung ...
    assert!(create(&conn, &n, 2_000).is_err());
    // ... und die Kennung einer entfernten Quelle bleibt ebenfalls tabu: die Zeile
    // steht in `calendar_sources` weiter, der Spiegel ist aber weg.
    conn.execute(
        "UPDATE calendar_sources SET deleted_at = 1500 WHERE id = 'kal-1'",
        [],
    )
    .unwrap();
    assert!(get(&conn, "kal-1").unwrap().is_none(), "Spiegel entfernt");
    let err = create(&conn, &n, 2_000).unwrap_err();
    assert!(err.to_string().contains("schon vergeben"), "{err}");
    assert!(get(&conn, "kal-1").unwrap().is_none());
    // Eine freie Kennung geht weiterhin.
    n.id = Some("ordner-frei".into());
    create(&conn, &n, 2_000).unwrap();
}

#[test]
fn the_added_secret_key_names_are_refused_in_a_configuration() {
    for key in [
        "pwd",
        "pass",
        "cookie",
        "session",
        "auth",
        "sig",
        "signature",
        "accessKey",
        "sas",
        "key",
    ] {
        let cfg = json!({ "verschachtelt": { key: "wert" } });
        let err = validate_config(&cfg).unwrap_err();
        assert!(err.to_string().contains(key), "{key}: {err}");
        assert!(validate_config(&json!({ key: "wert" })).is_err(), "{key}");
    }
    // Aehnlich klingende, harmlose Namen bleiben erlaubt.
    validate_config(
        &json!({ "author": "a", "keyboard": "de", "auth_mode": "device", "design": 1 }),
    )
    .unwrap();
}

#[test]
fn mark_error_keeps_the_host_of_an_address_but_not_its_path() {
    let fx = Fx::new();
    let conn = fx.conn();
    let f = folder(&conn, "Ablage");
    mark_error(
        &conn,
        &f.id,
        "Abruf https://outlook.office365.com/owa/calendar/x/SCHLUESSEL123/calendar.ics: 403",
        5,
    )
    .unwrap();
    let err = get(&conn, &f.id).unwrap().unwrap().last_error.unwrap();
    assert!(!err.contains("SCHLUESSEL123"), "{err}");
    assert!(err.contains("outlook.office365.com"), "{err}");
}
