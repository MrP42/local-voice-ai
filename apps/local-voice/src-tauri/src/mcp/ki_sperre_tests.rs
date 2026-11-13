//! G9a-2 (lva#578, ADR-0031 E2.8): der MCP-Server gibt nur freigegebene Besprechungen heraus.
//! Feste Besprechungen aus `testkit`, hier MIT der echten Pruefung (`Server::new`).

use serde_json::json;

use super::testkit::*;
use super::*;
use crate::managers::calendar::model::{event_key, Attendee, CalEvent, CalendarKind};
use crate::managers::meetings::ki_freigabe;
use crate::managers::meetings::store::SyncMeta;
use besprechungsplanung::assistent::Einstufung;

const T1: i64 = 1_790_000_000_000;

fn settings(dir: &std::path::Path, extra: serde_json::Value) {
    let mut s = json!({
        "meeting_mcp_enabled": true,
        "meeting_mcp_include_transcript": true,
        "meeting_own_domains": ["example.org"],
    });
    for (k, v) in extra.as_object().unwrap() {
        s[k] = v.clone();
    }
    std::fs::write(
        dir.join("settings_store.json"),
        json!({ "settings": s }).to_string(),
    )
    .unwrap();
}

/// Fixture mit echter Pruefung.
fn fx_mit_sperre(extra: serde_json::Value) -> Fx {
    let fx = Fx::new();
    settings(fx.dir.path(), extra);
    Fx {
        server: Server::new(config_in(fx.dir.path())),
        ..fx
    }
}

fn attendee(email: &str) -> Attendee {
    Attendee {
        email: Some(email.into()),
        name: None,
        organizer: false,
        is_self: false,
        partstat: None,
    }
}

/// Verknuepft `meeting` mit einem Termin der Teilnehmenden `emails` und bestaetigt `bestaetigt`.
fn verknuepfen(fx: &Fx, meeting: &str, uid: &str, emails: &[&str], bestaetigt: Option<Einstufung>) {
    if fx.store.calendar_source("src1").unwrap().is_none() {
        fx.store
            .calendar_source_add("src1", CalendarKind::Ics, "Kalender", None, 1_000)
            .unwrap();
    }
    let ev = CalEvent {
        key: event_key("src1", uid, T1),
        source_id: "src1".into(),
        uid: uid.into(),
        title: "Termin".into(),
        starts_at: T1,
        ends_at: T1 + 1_800_000,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: None,
        instance_start: None,
        attendees: emails.iter().map(|e| attendee(e)).collect(),
    };
    let mut events: Vec<CalEvent> = fx
        .store
        .calendar_events_between(T1 - 1, T1 + 3_600_000, true)
        .unwrap();
    events.retain(|e| e.key != ev.key);
    events.push(ev.clone());
    fx.store
        .calendar_replace_events(
            "src1",
            &events,
            &SyncMeta {
                has_attendee_data: true,
                etag: None,
                last_modified: None,
                now_ms: 2_000,
            },
        )
        .unwrap();
    fx.store
        .link_meeting_event(meeting, &ev, "manual", 3_000)
        .unwrap();
    if let Some(e) = bestaetigt {
        let sperre = read_ki_sperre(&config_in(fx.dir.path()).settings_path);
        let fp = ki_freigabe::stand(&fx.store, &sperre, meeting)
            .unwrap()
            .ergebnis
            .fingerabdruck;
        ki_freigabe::bestaetigen(&fx.store, &sperre, meeting, e, &fp, 4_000).unwrap();
    }
}

#[test]
fn altbestand_ohne_bestaetigung_ist_unsichtbar() {
    let fx = fx_mit_sperre(json!({}));
    let list = fx.tool_json("list_meetings", json!({}));
    assert_eq!(ids(&list), Vec::<String>::new());
    assert_eq!(list["total"], 0, "auch die Gesamtzahl verrät nichts");
    assert!(list["note"].as_str().unwrap().contains("freigegeben"));
    let search = fx.tool_json("search_meetings", json!({ "query": "Umsatz" }));
    assert_eq!(ids(&search), Vec::<String>::new());
    for tool in ["get_meeting", "get_transcript", "get_provenance"] {
        let (text, is_error) = fx.tool(tool, json!({ "id": fx.a }));
        assert!(is_error, "{tool}: {text}");
        assert!(text.contains("nicht gefunden oder nicht für KI-Clients freigegeben"));
        assert!(!text.contains("Jour fixe"), "kein Inhalt im Fehler: {text}");
    }
    // Gesperrt und unbekannt sehen gleich aus.
    let (unbekannt, _) = fx.tool("get_meeting", json!({ "id": "01UNBEKANNT" }));
    let (gesperrt, _) = fx.tool("get_meeting", json!({ "id": fx.a }));
    assert_eq!(
        unbekannt.replace("01UNBEKANNT", "ID"),
        gesperrt.replace(&fx.a, "ID")
    );
}

#[test]
fn bestaetigte_interne_besprechung_ist_sichtbar_die_anderen_nicht() {
    let fx = fx_mit_sperre(json!({}));
    verknuepfen(
        &fx,
        &fx.a,
        "u-a",
        &["a@example.org", "b@example.org"],
        Some(Einstufung::Intern),
    );
    verknuepfen(&fx, &fx.d, "u-d", &["a@example.org"], None);
    let list = fx.tool_json("list_meetings", json!({}));
    assert_eq!(ids(&list), [fx.a.clone()]);
    assert_eq!(list["total"], 1);
    assert!(!fx.tool("get_meeting", json!({ "id": fx.a })).1);
    assert!(fx.tool("get_meeting", json!({ "id": fx.d })).1);
    let search = fx.tool_json("search_meetings", json!({ "query": "Umsatz" }));
    assert_eq!(ids(&search), [fx.a.clone()]);
}

#[test]
fn externe_nur_mit_einstellung() {
    let fx = fx_mit_sperre(json!({}));
    verknuepfen(
        &fx,
        &fx.a,
        "u-a",
        &["a@example.org", "x@example.com"],
        Some(Einstufung::Extern),
    );
    assert!(fx.tool("get_meeting", json!({ "id": fx.a })).1);
    settings(
        fx.dir.path(),
        json!({ "meeting_cloud_with_externals": true }),
    );
    assert!(
        !fx.tool("get_meeting", json!({ "id": fx.a })).1,
        "Einstellung wirkt ohne Neustart"
    );
}

#[test]
fn nachzuegler_im_termin_sperrt_sofort() {
    let fx = fx_mit_sperre(json!({}));
    verknuepfen(
        &fx,
        &fx.a,
        "u-a",
        &["a@example.org"],
        Some(Einstufung::Intern),
    );
    assert!(!fx.tool("get_meeting", json!({ "id": fx.a })).1);
    verknuepfen(&fx, &fx.a, "u-a", &["a@example.org", "c@example.org"], None);
    assert!(fx.tool("get_meeting", json!({ "id": fx.a })).1);
}

#[test]
fn kundenprofil_nur_lokal_sperrt_alle_werkzeuge() {
    let fx = fx_mit_sperre(json!({ "compliance_profile": "local_only" }));
    verknuepfen(
        &fx,
        &fx.a,
        "u-a",
        &["a@example.org"],
        Some(Einstufung::Intern),
    );
    for tool in ["list_meetings", "search_meetings", "get_meeting"] {
        let (text, is_error) = fx.tool(tool, json!({ "id": fx.a, "query": "Umsatz" }));
        assert!(is_error && text.contains("nur lokal"), "{tool}: {text}");
    }
}

#[test]
fn einstellungen_im_zweifel_streng() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings_store.json");
    assert!(read_ki_sperre(&path).ganz_gesperrt(), "Datei fehlt");
    std::fs::write(&path, "{kaputt").unwrap();
    assert!(read_ki_sperre(&path).ganz_gesperrt(), "unlesbar");
    std::fs::write(
        &path,
        r#"{"settings":{"compliance_profile":7,"meeting_own_domains":"example.org","meeting_cloud_with_externals":"true"}}"#,
    )
    .unwrap();
    assert_eq!(
        read_ki_sperre(&path),
        KiSperre::Pruefen {
            kundenprofil_nur_lokal: true,
            cloud_bei_externen: false,
            eigene_domaenen: Vec::new(),
        }
    );
}

#[test]
fn die_namen_der_freigabe_einstellungen_passen_zur_app() {
    let mut defaults = serde_json::to_value(crate::settings::get_default_settings()).unwrap();
    assert_eq!(defaults[KEY_OWN_DOMAINS], json!([]));
    assert_eq!(defaults[KEY_CLOUD_WITH_EXTERNALS], json!(false));
    assert_eq!(defaults[KEY_COMPLIANCE_PROFILE], json!("eu"));
    defaults[KEY_OWN_DOMAINS] = json!(["example.org"]);
    defaults[KEY_CLOUD_WITH_EXTERNALS] = json!(true);
    let mut s = crate::settings::get_default_settings();
    s.compliance_profile = crate::managers::compliance::ComplianceProfile::LocalOnly;
    defaults[KEY_COMPLIANCE_PROFILE] = serde_json::to_value(s.compliance_profile).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings_store.json");
    std::fs::write(&path, json!({ "settings": defaults }).to_string()).unwrap();
    assert_eq!(
        read_ki_sperre(&path),
        KiSperre::Pruefen {
            kundenprofil_nur_lokal: true,
            cloud_bei_externen: true,
            eigene_domaenen: vec!["example.org".into()],
        }
    );
}
