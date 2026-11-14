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

/// Wie die App: Einstellungsdatei UND Richtlinienspiegel der Datenbank.
fn einstellen(fx: &Fx, extra: serde_json::Value) {
    settings(fx.dir.path(), extra);
    spiegeln(fx);
}

/// Spiegel nach der Einstellungsdatei (so schreibt ihn die App).
fn spiegeln(fx: &Fx) {
    let sperre = read_ki_sperre(&config_in(fx.dir.path()).settings_path);
    fx.store
        .ki_richtlinie_setzen(&sperre.richtlinie().unwrap())
        .unwrap();
}

/// Fixture mit echter Pruefung.
fn fx_mit_sperre(extra: serde_json::Value) -> Fx {
    let fx = Fx::new();
    einstellen(&fx, extra);
    Fx {
        server: Server::new(config_in(fx.dir.path())),
        ..fx
    }
}

fn nachweis() -> ki_freigabe::Freigabenachweis {
    ki_freigabe::Freigabenachweis {
        informiert: true,
        rechtsgrundlage: Some(ki_freigabe::Rechtsgrundlage::OeffentlicheAufgabe),
        einwilligungen_liegen_vor: false,
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
                now_ms: T1 + 3_600_000, // Abgleich nach dem Ende: Teilnehmerstand aktuell
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
        ki_freigabe::bestaetigen(&fx.store, &sperre, meeting, e, &fp, &nachweis(), 4_000).unwrap();
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
    einstellen(&fx, json!({ "meeting_cloud_with_externals": true }));
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
fn unbekanntes_oder_null_kundenprofil_sperrt_fehlendes_nicht() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings_store.json");
    for (profil, gesperrt) in [
        (json!("eu"), false),
        (json!("none"), false),
        (json!("local_only"), true),
        (json!("strenger_als_lokal"), true),
        (json!("EU"), true),
        (Value::Null, true),
    ] {
        std::fs::write(
            &path,
            json!({ "settings": { KEY_COMPLIANCE_PROFILE: profil } }).to_string(),
        )
        .unwrap();
        assert_eq!(read_ki_sperre(&path).ganz_gesperrt(), gesperrt, "{profil}");
    }
    // Schlüssel fehlt (Altbestand): Standard der App, `eu`.
    std::fs::write(&path, r#"{"settings":{}}"#).unwrap();
    assert!(!read_ki_sperre(&path).ganz_gesperrt());
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

/// Setzt die Testbarriere vor dem Linearisierungspunkt und entfernt sie am Ende. Die Änderung
/// läuft nur beim ERSTEN Durchgang; gezählt werden alle Durchgänge.
struct Barriere {
    zaehler: std::rc::Rc<std::cell::Cell<usize>>,
}
impl Barriere {
    fn einmal(f: impl Fn() + 'static) -> Self {
        let zaehler = std::rc::Rc::new(std::cell::Cell::new(0));
        let z = zaehler.clone();
        ki_freigabe::VOR_NACHPRUEFUNG.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                z.set(z.get() + 1);
                if z.get() == 1 {
                    f()
                }
            }))
        });
        Barriere { zaehler }
    }
    fn durchgaenge(&self) -> usize {
        self.zaehler.get()
    }
}
impl Drop for Barriere {
    fn drop(&mut self) {
        ki_freigabe::VOR_NACHPRUEFUNG.with(|h| *h.borrow_mut() = None);
    }
}

const WERKZEUGE: [&str; 5] = [
    "get_meeting",
    "get_transcript",
    "get_provenance",
    "list_meetings",
    "search_meetings",
];

/// Aufruf eines Werkzeugs für Besprechung A; `Some(Text)`, wenn A in der Antwort steckt.
fn a_abrufen(fx: &Fx, tool: &str) -> (Option<String>, String) {
    let (text, is_error) = fx.tool(tool, json!({ "id": fx.a, "query": "Umsatz" }));
    let drin = match tool {
        "list_meetings" | "search_meetings" if !is_error => {
            let v: Value = serde_json::from_str(&text).unwrap();
            ids(&v).contains(&fx.a)
        }
        _ => !is_error,
    };
    (drin.then(|| text.clone()), text)
}

/// Freigegebene Besprechung A: intern, oder (`extern_mit_cloud`) mit Externem und Ausnahme an.
fn freigegeben(extern_mit_cloud: bool) -> Fx {
    if extern_mit_cloud {
        let fx = fx_mit_sperre(json!({ "meeting_cloud_with_externals": true }));
        verknuepfen(
            &fx,
            &fx.a,
            "u-a",
            &["a@example.org", "x@example.com"],
            Some(Einstufung::Extern),
        );
        fx
    } else {
        let fx = fx_mit_sperre(json!({}));
        verknuepfen(
            &fx,
            &fx.a,
            "u-a",
            &["a@example.org", "b@example.org"],
            Some(Einstufung::Intern),
        );
        fx
    }
}

/// Änderungen, wie sie zwischen Prüfung und Ausgabe festgeschrieben werden können.
fn aendern(art: &str, dir: &std::path::Path, store_db: &std::path::Path) {
    let sql = rusqlite::Connection::open(store_db).unwrap();
    let store = crate::managers::meetings::store::MeetingStore::open_at(store_db).unwrap();
    let einstellen = |extra: serde_json::Value| {
        settings(dir, extra);
        let sperre = read_ki_sperre(&dir.join("settings_store.json"));
        store
            .ki_richtlinie_setzen(&sperre.richtlinie().unwrap())
            .unwrap();
    };
    match art {
        "widerruf" => {
            sql.execute("DELETE FROM meeting_ai_confirmations", [])
                .unwrap();
        }
        "kalender" => {
            sql.execute(
                "INSERT INTO calendar_event_attendees (event_key, email, name)
                 SELECT key, 'neu@example.org', '' FROM calendar_events WHERE uid = 'u-a'",
                [],
            )
            .unwrap();
        }
        "kalender_hin_und_zurueck" => {
            sql.execute(
                "INSERT INTO calendar_event_attendees (event_key, email, name)
                 SELECT key, 'neu@example.org', '' FROM calendar_events WHERE uid = 'u-a'",
                [],
            )
            .unwrap();
            sql.execute(
                "DELETE FROM calendar_event_attendees WHERE email = 'neu@example.org'",
                [],
            )
            .unwrap();
        }
        "cloud_ausnahme_aus" => einstellen(json!({ "meeting_cloud_with_externals": false })),
        "nur_lokal" => einstellen(json!({
            "meeting_cloud_with_externals": true,
            "compliance_profile": "local_only"
        })),
        "domaenen" => einstellen(json!({
            "meeting_cloud_with_externals": true,
            "meeting_own_domains": ["example.net"]
        })),
        // Nur die Datei ist umgestellt, der Spiegel noch nicht (Schreiben unterwegs/fehlgeschlagen).
        "nur_datei" => settings(dir, json!({ "meeting_cloud_with_externals": false })),
        other => panic!("{other}"),
    }
}

/// Codex #583 Fixrunde 2: Je Werkzeug und Änderungsart eine FRISCHE Fixture. Erst ein
/// erfolgreicher Abruf (A mit Inhalt), dann die Änderung an der Barriere vor dem
/// Linearisierungspunkt. Die Ausgabe des ersten Durchgangs geht NIE hinaus (zwei Durchgänge
/// gezählt); der zweite liest frisch und sieht A gesperrt. Entfernt man die abschließende
/// Prüfung in `tools::call`, wird jeder dieser Fälle rot.
#[test]
fn aenderung_vor_dem_linearisierungspunkt_verwirft_jede_ausgabe() {
    for art in [
        "widerruf",
        "kalender",
        "cloud_ausnahme_aus",
        "nur_lokal",
        "domaenen",
        "nur_datei",
    ] {
        for tool in WERKZEUGE {
            let fx = freigegeben(true);
            let (vorher, text) = a_abrufen(&fx, tool);
            assert!(vorher.is_some(), "{art}/{tool}: vorher sichtbar: {text}");
            let cfg = config_in(fx.dir.path());
            let dir = fx.dir.path().to_path_buf();
            let b = Barriere::einmal(move || aendern(art, &dir, &cfg.db_path));
            let (nachher, text) = a_abrufen(&fx, tool);
            assert!(nachher.is_none(), "{art}/{tool}: {text}");
            assert!(
                !text.contains("Millionen") && !text.contains("Jour fixe"),
                "{art}/{tool}: kein Inhalt: {text}"
            );
            let erwartet = if art == "nur_datei" || art == "nur_lokal" {
                // Der zweite Durchgang bricht schon am Anfang ab (Abgleich bzw. „nur lokal“).
                1
            } else {
                2
            };
            assert_eq!(b.durchgaenge(), erwartet, "{art}/{tool}: Durchgänge");
        }
    }
}

/// A → B → A zwischen Prüfung und Ausgabe: gleicher Wert, aber höhere Revision. Die erste
/// Ausgabe wird verworfen (zwei Durchgänge), die zweite liest den Stand nach der Rückkehr.
#[test]
fn hin_und_zurueck_wird_erkannt() {
    for tool in WERKZEUGE {
        let fx = freigegeben(false);
        assert!(a_abrufen(&fx, tool).0.is_some(), "{tool}");
        let cfg = config_in(fx.dir.path());
        let dir = fx.dir.path().to_path_buf();
        let b = Barriere::einmal(move || aendern("kalender_hin_und_zurueck", &dir, &cfg.db_path));
        assert!(a_abrufen(&fx, tool).0.is_some(), "{tool}");
        assert_eq!(b.durchgaenge(), 2, "{tool}: erste Ausgabe verworfen");
    }
}

/// Widerruf eines bereits geprüften Listeneintrags, während weitere Einträge verarbeitet werden:
/// Die GANZE Liste wird verworfen und neu gelesen.
#[test]
fn widerruf_eines_listeneintrags_verwirft_die_ganze_liste() {
    for tool in ["list_meetings", "search_meetings"] {
        let fx = freigegeben(false);
        verknuepfen(
            &fx,
            &fx.d,
            "u-d",
            &["a@example.org"],
            Some(Einstufung::Intern),
        );
        let alle = fx.tool_json("list_meetings", json!({}));
        assert_eq!(ids(&alle).len(), 2);
        let db = config_in(fx.dir.path()).db_path;
        let a = fx.a.clone();
        let b = Barriere::einmal(move || {
            let sql = rusqlite::Connection::open(&db).unwrap();
            sql.execute(
                "DELETE FROM meeting_ai_confirmations WHERE meeting_id = ?1",
                [&a],
            )
            .unwrap();
        });
        let v = fx.tool_json(tool, json!({ "query": "Jour fixe Workshop" }));
        assert!(!ids(&v).contains(&fx.a), "{tool}: {v}");
        assert_eq!(b.durchgaenge(), 2, "{tool}");
        if tool == "list_meetings" {
            assert_eq!(ids(&v), [fx.d.clone()]);
            assert_eq!(v["total"], 1);
        }
    }
}

/// Ständige Änderung: nach drei verworfenen Durchgängen eine Fehlermeldung ohne Inhalt.
#[test]
fn staendige_aenderung_endet_mit_hinweis_ohne_inhalt() {
    let fx = freigegeben(false);
    let db = config_in(fx.dir.path()).db_path;
    ki_freigabe::VOR_NACHPRUEFUNG.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            rusqlite::Connection::open(&db)
                .unwrap()
                .execute(
                    "UPDATE calendar_sources SET last_ok_at = last_ok_at + 1",
                    [],
                )
                .unwrap();
        }))
    });
    let (text, is_error) = fx.tool("get_meeting", json!({ "id": fx.a }));
    ki_freigabe::VOR_NACHPRUEFUNG.with(|h| *h.borrow_mut() = None);
    assert!(is_error && text.contains("geändert"), "{text}");
    assert!(!text.contains("Umsatz"));
}

/// Ein Widerruf NACH dem Linearisierungspunkt gilt als nach der Antwort geschehen: Diese
/// Antwort geht noch hinaus (sie war beim Linearisierungspunkt freigegeben), der nächste
/// Aufruf ist gesperrt. Dokumentiert die Grenze (ADR-0031: Versandtes lässt sich nicht
/// zurückholen).
#[test]
fn widerruf_nach_dem_linearisierungspunkt_wirkt_ab_dem_naechsten_aufruf() {
    let fx = freigegeben(false);
    let (text, is_error) = fx.tool("get_meeting", json!({ "id": fx.a }));
    assert!(!is_error, "{text}");
    fx.store.ki_bestaetigung_entfernen(&fx.a).unwrap();
    assert!(fx.tool("get_meeting", json!({ "id": fx.a })).1);
}

/// Spiegel fehlt (Datenbank vor dem ersten Start nach dem Update): nichts geht hinaus.
#[test]
fn ohne_spiegel_gibt_es_keine_ausgabe() {
    let fx = freigegeben(false);
    rusqlite::Connection::open(config_in(fx.dir.path()).db_path)
        .unwrap()
        .execute("DELETE FROM meeting_ai_policy", [])
        .unwrap();
    for tool in WERKZEUGE {
        let (text, is_error) = fx.tool(tool, json!({ "id": fx.a, "query": "Umsatz" }));
        assert!(
            is_error && text.contains("Freigabe-Einstellungen"),
            "{tool}: {text}"
        );
    }
}

/// Export (`get_meeting` mit allen Teilen über `build_bundle`, dazu Transkript und Herkunft):
/// Die gesperrte Besprechung D trägt in JEDEM Teil eine Marke und teilt mit A dieselbe Person.
/// `build_bundle` liest ausschließlich über die angefragte ID (Besprechung, Segmente, Notizen,
/// Dokumente, Aufgaben, Sprecher); nichts von D darf in A erscheinen, weder Inhalt noch
/// Metadaten (Titel, ID).
#[test]
fn export_einer_freigegebenen_enthaelt_nichts_aus_einer_gesperrten() {
    use crate::managers::meetings::notes::model::{NoteBlock, NoteBlockKind};
    use crate::managers::meetings::store::TranscriptDelta;
    let fx = freigegeben(false);
    verknuepfen(&fx, &fx.d, "u-d", &["a@example.org"], None);
    let marke = "MARKE-D-GESPERRT";
    fx.store
        .append_delta(
            &fx.d,
            &TranscriptDelta {
                new_segments: vec![segment(0, 0, 0, &format!("{marke} Transkript"))],
            },
        )
        .unwrap();
    fx.store
        .save_notes(
            &fx.d,
            &[NoteBlock {
                id: "ND".into(),
                kind: NoteBlockKind::Bullet,
                text: format!("{marke} Notiz"),
                at_ms: None,
                checked: false,
            }],
            0,
        )
        .unwrap();
    fx.store
        .upsert_document(
            &fx.d,
            "minutes",
            "markdown@1",
            &format!("# Protokoll\n\n## Ergebnis\n\n{marke} Protokoll\n"),
            None,
        )
        .unwrap();
    let sql = rusqlite::Connection::open(config_in(fx.dir.path()).db_path).unwrap();
    sql.execute(
        "INSERT INTO action_items (id, meeting_id, text, status, source, kind, created_at, updated_at)
         VALUES ('AID', ?1, ?2, 'todo', 'manual', 'task', 1, 1)",
        rusqlite::params![fx.d, format!("{marke} Aufgabe")],
    )
    .unwrap();
    let alle = json!([
        "ai_notes",
        "notes",
        "minutes",
        "participants",
        "action_items"
    ]);
    let mut texte = Vec::new();
    for (tool, args) in [
        ("get_meeting", json!({ "id": fx.a, "parts": alle })),
        ("get_transcript", json!({ "id": fx.a })),
        ("get_provenance", json!({ "id": fx.a })),
        ("list_meetings", json!({})),
        ("search_meetings", json!({ "query": marke })),
    ] {
        let (text, is_error) = fx.tool(tool, args);
        assert!(!is_error, "{tool}: {text}");
        // Die Suche wiederholt nur die Anfrage selbst.
        let text = text.replace(&format!("\"query\":\"{marke}\""), "");
        texte.push((tool, text));
    }
    assert!(texte[0].1.contains("Jour fixe"), "Gegenprobe: A ist da");
    assert!(
        texte[1].1.contains("Millionen"),
        "Gegenprobe: Transkript von A"
    );
    for (tool, text) in &texte {
        for verboten in [marke, "Workshop Einkauf", fx.d.as_str()] {
            assert!(
                !text.contains(verboten),
                "{tool} enthält {verboten}: {text}"
            );
        }
    }
}

#[test]
fn freigegebene_besprechung_enthaelt_nichts_aus_einer_gesperrten() {
    let fx = fx_mit_sperre(json!({}));
    verknuepfen(
        &fx,
        &fx.a,
        "u-a",
        &["a@example.org"],
        Some(Einstufung::Intern),
    );
    verknuepfen(&fx, &fx.d, "u-d", &["a@example.org"], None);
    let (text, is_error) = fx.tool("get_meeting", json!({ "id": fx.a }));
    assert!(!is_error, "{text}");
    assert!(text.contains("Jour fixe"));
    assert!(!text.contains("Workshop Einkauf"), "{text}");
    let (text, _) = fx.tool("get_transcript", json!({ "id": fx.a }));
    assert!(!text.contains("Workshop Einkauf"), "{text}");
}
