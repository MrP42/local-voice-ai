use super::*;
use serde_json::json;

fn schreibe(data: &Path, rel: &str, inhalt: &[u8]) {
    let p = data.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, inhalt).unwrap();
}

fn auftrag(einst: &Value) -> Auftrag<'_> {
    Auftrag {
        anlass: "anmeldung",
        konto: Some("b@example.org"),
        einstellungen: einst,
        platz_frei: None,
    }
}

fn beispiel() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let d = t.path();
    schreibe(
        d,
        "projects/index.json",
        br#"{"pages":[{"id":"page_1","title":"A"},{"id":"page_2","title":"B","buch":"book_1"}]}"#,
    );
    schreibe(d, "projects/page_1/state.json", br#"{"text":"Hallo"}"#);
    schreibe(d, "projects/page_1/audio.wav", b"RIFF");
    schreibe(d, "projects/_geloescht/page_9/state.json", b"alt");
    schreibe(d, "library/book_1/buch.json", b"{}");
    schreibe(d, "library/book_1/audio/hoerbuch.mp3", b"0123456789abcdef");
    schreibe(d, "library/book_1/fassungen/page_2/state.json", b"{}");
    schreibe(
        d,
        "library/fortschritt.json",
        br#"{"progress.audio:book_1":"a","progress.audio:book_12":"b","progress.text:page_1:original":"c","tts.resume.page_1:original":"d","tts.voice.lang.de":"e"}"#,
    );
    t
}

fn index_ids(d: &Path) -> Vec<String> {
    let v: Value =
        serde_json::from_slice(&fs::read(d.join("projects/index.json")).unwrap()).unwrap();
    v["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

fn fortschritt(d: &Path) -> BTreeMap<String, String> {
    serde_json::from_slice(&fs::read(d.join("library/fortschritt.json")).unwrap()).unwrap()
}

#[test]
fn bezug_ist_strukturiert() {
    assert_eq!(bezug("progress.audio:book_1"), Some("book_1"));
    assert_eq!(bezug("progress.text:page_1:original"), Some("page_1"));
    assert_eq!(bezug("progress.doc:book_7:zusammenfassung"), Some("book_7"));
    assert_eq!(bezug("tts.resume.page_1:original"), Some("page_1"));
    assert_eq!(bezug("tts.bookmarks.page_1"), Some("page_1"));
    assert_eq!(bezug("tts.voice.book.book_1_ab.de"), Some("book_1_ab"));
    assert_eq!(bezug("tts.voice.lang.de"), None);
    assert_eq!(bezug("progress.anders:book_1"), None);
}

#[test]
fn nur_lokal_ueber_einheit_und_exakte_schluessel() {
    let n = NurLokal {
        schluessel: ["settings/app".to_string()].into(),
        einheiten: ["page_1".to_string(), "book_1".to_string()].into(),
    };
    assert!(n.betrifft("page", "page_1", &Value::Null));
    assert!(n.betrifft("book", "book_1", &Value::Null));
    assert!(n.betrifft("book_file", "x", &json!({"book": "book_1"})));
    assert!(n.betrifft("progress", "p", &json!({"key": "audio:book_1"})));
    assert!(n.betrifft(
        "bookmark",
        "b",
        &json!({"key": "tts.resume.page_1:original"})
    ));
    assert!(n.betrifft("settings", "app", &Value::Null));
    assert!(!n.betrifft("progress", "p", &json!({"key": "audio:book_12"})));
    assert!(!n.betrifft("bookmark", "b", &json!({"key": "tts.voice.lang.de"})));
    assert!(n.sperrt_eingang("page", "page_1"));
    assert!(n.sperrt_eingang("settings", "app"));
    assert!(!n.sperrt_eingang("progress", "p"));
    let mut m = n.clone();
    m.einheit_freigeben("page_1");
    assert!(!m.betrifft("page", "page_1", &Value::Null));
    assert!(m.betrifft("book", "book_1", &Value::Null));
}

#[test]
fn bestand_nur_identische_hashes_desselben_kontos_sind_bekannt() {
    let o = |s: &str, id: &str, h: Option<&str>, e: Option<&str>| Objekt {
        sammlung: s.into(),
        id: id.into(),
        hash: h.map(str::to_owned),
        einheit: e.map(str::to_owned),
        map_schluessel: None,
    };
    let objekte = vec![
        o("page", "page_1", Some("h1"), Some("page_1")),
        o("page", "page_2", Some("neu"), Some("page_2")),
        o("book", "book_1", None, Some("book_1")),
        o("settings", "app", Some("s"), None),
        o("page", "page_3", Some("h3"), Some("page_3")),
    ];
    let h = Herkunft {
        user_id: 7,
        hub: "https://hub.example.org".into(),
        email: "a@example.org".into(),
        eintraege: [
            ("page/page_1".to_string(), "h1".to_string()),
            ("page/page_2".to_string(), "alt".to_string()),
            ("book/book_1".to_string(), "x".to_string()),
            ("settings/app".to_string(), "s".to_string()),
        ]
        .into(),
    };
    let nur = NurLokal {
        einheiten: ["page_3".to_string()].into(),
        ..Default::default()
    };
    let b: Vec<&str> = bestand(&objekte, Some(&h), &nur)
        .iter()
        .map(|o| o.id.as_str())
        .collect();
    // geändert nach Abmeldung, unfertiger Hash → Bestand; gleich → bekannt; nur lokal → nicht.
    assert_eq!(b, vec!["page_2", "book_1"]);
    // Ohne Herkunft desselben Kontos ist alles Bestand.
    assert_eq!(bestand(&objekte, None, &NurLokal::default()).len(), 5);
}

#[test]
fn sicherung_kopiert_prueft_und_listet() {
    let t = beispiel();
    let d = t.path();
    nur_lokal_speichern(
        d,
        &NurLokal {
            einheiten: ["page_7".to_string()].into(),
            ..Default::default()
        },
    )
    .unwrap();
    let einst = json!({"theme": "dark"});
    let info = sichern(d, &auftrag(&einst)).unwrap();
    assert_eq!((info.seiten, info.buecher), (1, 1));
    let dir = d.join(SICHERUNGEN).join(&info.name);
    let m = pruefen(&dir).unwrap();
    let pfade: Vec<&str> = m.dateien.iter().map(|x| x.pfad.as_str()).collect();
    for p in [
        "projects/index.json",
        "projects/page_1/audio.wav",
        "library/book_1/audio/hoerbuch.mp3",
        "library/book_1/fassungen/page_2/state.json",
        "library/fortschritt.json",
        NUR_LOKAL,
    ] {
        assert!(pfade.contains(&p), "{p} fehlt");
    }
    assert!(!pfade.iter().any(|p| p.contains("_geloescht")));
    assert_eq!(einstellungen_der_sicherung(d, &info.name).unwrap(), einst);
    assert_eq!(auflisten(d)[0].name, info.name);
    // Kopie, kein Link: Änderung am Original ändert die Sicherung nicht.
    fs::write(d.join("library/book_1/audio/hoerbuch.mp3"), b"anders").unwrap();
    pruefen(&dir).unwrap();
}

#[test]
fn zu_wenig_platz_aendert_nichts() {
    let t = beispiel();
    let einst = json!({});
    let mut a = auftrag(&einst);
    a.platz_frei = Some(10);
    assert!(sichern(t.path(), &a).is_err());
    assert!(auflisten(t.path()).is_empty());
    assert!(!t.path().join(SICHERUNGEN).exists());
}

#[test]
fn beschaedigte_sicherung_wird_erkannt() {
    let t = beispiel();
    let einst = json!({});
    let info = sichern(t.path(), &auftrag(&einst)).unwrap();
    let dir = t.path().join(SICHERUNGEN).join(&info.name);
    fs::write(dir.join("projects/page_1/state.json"), b"anders").unwrap();
    assert!(pruefen(&dir).is_err());
    assert!(umsetzen(t.path(), &info.name, &Plan::default()).is_err());
}

#[test]
fn entfernen_nur_was_unveraendert_gesichert_ist_und_wiederholbar() {
    let t = beispiel();
    let d = t.path();
    let einst = json!({});
    let info = sichern(d, &auftrag(&einst)).unwrap();
    let plan = Plan {
        entfernen: ["page_1".to_string(), "book_1".to_string()].into(),
        ..Default::default()
    };
    // Nach der Sicherung geändert: nichts wird entfernt.
    schreibe(d, "projects/page_1/neu.txt", b"neu");
    assert!(umsetzen(d, &info.name, &plan).is_err());
    assert!(d.join("library/book_1/buch.json").is_file());
    assert!(d.join("projects/page_1/state.json").is_file());
    fs::remove_file(d.join("projects/page_1/neu.txt")).unwrap();

    let u = umsetzen(d, &info.name, &plan).unwrap();
    assert_eq!(u.entfernte_einheiten, 2);
    assert!(!d.join("projects/page_1").exists());
    assert!(!d.join("library/book_1").exists());
    assert!(index_ids(d).is_empty());
    let f = fortschritt(d);
    assert_eq!(
        f.keys().cloned().collect::<Vec<_>>(),
        vec!["progress.audio:book_12", "tts.voice.lang.de"]
    );
    // Wiederanlauf: gleicher Endstand, kein Fehler.
    let u2 = umsetzen(d, &info.name, &plan).unwrap();
    assert_eq!(u2.entfernte_einheiten, 0);
    assert_eq!(fortschritt(d).len(), 2);
}

#[test]
fn kollision_kopiert_seite_unter_neuer_kennung_und_gibt_alte_frei() {
    let t = beispiel();
    let d = t.path();
    nur_lokal_speichern(
        d,
        &NurLokal {
            einheiten: ["page_1".to_string()].into(),
            ..Default::default()
        },
    )
    .unwrap();
    let einst = json!({});
    let info = sichern(d, &auftrag(&einst)).unwrap();
    let plan = Plan {
        kopien: [("page_1".to_string(), "page_77".to_string())].into(),
        entfernen: ["page_1".to_string()].into(),
        nur_lokal: NurLokal {
            einheiten: ["page_77".to_string()].into(),
            ..Default::default()
        },
        freigeben: ["page_1".to_string()].into(),
        ..Default::default()
    };
    umsetzen(d, &info.name, &plan).unwrap();
    assert!(!d.join("projects/page_1").exists());
    assert_eq!(
        fs::read(d.join("projects/page_77/audio.wav")).unwrap(),
        b"RIFF"
    );
    assert_eq!(index_ids(d), vec!["page_2", "page_77"]);
    let f = fortschritt(d);
    assert!(f.contains_key("progress.text:page_77:original"));
    assert!(f.contains_key("tts.resume.page_77:original"));
    assert!(!f.contains_key("progress.text:page_1:original"));
    let n = nur_lokal_laden(d);
    assert!(n.einheiten.contains("page_77") && !n.einheiten.contains("page_1"));
    // Wiederholbar: keine zweite Kopie.
    umsetzen(d, &info.name, &plan).unwrap();
    assert_eq!(index_ids(d), vec!["page_2", "page_77"]);
}

#[test]
fn journal_rundlauf() {
    let t = tempfile::tempdir().unwrap();
    let j = Journal {
        art: Art::Abmeldung,
        hub: "https://hub.example.org".into(),
        user_id: 3,
        email: "a@example.org".into(),
        wahl: Some(Wahl::Entfernen),
        phase: Phase::Gesichert,
        sicherung: Some("konto-1".into()),
        plan: Plan::default(),
    };
    journal_speichern(t.path(), &j).unwrap();
    assert_eq!(journal_laden(t.path()).unwrap(), j);
    journal_loeschen(t.path()).unwrap();
    assert!(journal_laden(t.path()).is_none());
}

#[test]
fn sicherung_eines_offenen_uebergangs_ist_nicht_loeschbar() {
    let t = beispiel();
    let einst = json!({});
    let info = sichern(t.path(), &auftrag(&einst)).unwrap();
    let j = Journal {
        art: Art::Anmeldung,
        hub: "h".into(),
        user_id: 1,
        email: "e".into(),
        wahl: None,
        phase: Phase::Gesichert,
        sicherung: Some(info.name.clone()),
        plan: Plan::default(),
    };
    journal_speichern(t.path(), &j).unwrap();
    assert!(loeschen(t.path(), &info.name).is_err());
    journal_loeschen(t.path()).unwrap();
    loeschen(t.path(), &info.name).unwrap();
    assert!(auflisten(t.path()).is_empty());
}

#[test]
fn wiederherstellen_ueberschreibt_nie_und_legt_abweichendes_daneben() {
    let t = beispiel();
    let d = t.path();
    let einst = json!({});
    let info = sichern(d, &auftrag(&einst)).unwrap();
    let plan = Plan {
        entfernen: ["book_1".to_string()].into(),
        ..Default::default()
    };
    umsetzen(d, &info.name, &plan).unwrap();
    // page_1 lokal geändert, ein Fortschritt geändert.
    fs::write(d.join("projects/page_1/state.json"), b"neu").unwrap();
    let mut f = fortschritt(d);
    f.insert("progress.audio:book_12".into(), "geaendert".into());
    fs::write(
        d.join("library/fortschritt.json"),
        serde_json::to_vec(&f).unwrap(),
    )
    .unwrap();

    let w = wiederherstellen(d, &info.name).unwrap();
    assert_eq!(w.zurueck, vec!["book_1"]);
    assert_eq!(w.zusaetzlich.len(), 1);
    assert_eq!(
        fs::read(d.join("projects/page_1/state.json")).unwrap(),
        b"neu"
    );
    let kopie = &w.zusaetzlich[0];
    assert_eq!(
        fs::read(d.join("projects").join(kopie).join("state.json")).unwrap(),
        br#"{"text":"Hallo"}"#
    );
    assert_eq!(
        fs::read(d.join("library/book_1/audio/hoerbuch.mp3")).unwrap(),
        b"0123456789abcdef"
    );
    assert!(index_ids(d).contains(&"page_2".to_string()));
    assert!(index_ids(d).contains(kopie));
    assert_eq!(w.fortschritte_ergaenzt, 1);
    assert_eq!(w.fortschritte_abweichend, 1);
    assert_eq!(fortschritt(d)["progress.audio:book_12"], "geaendert");
    // Zweiter Lauf: alles gleich oder schon daneben, nichts überschrieben.
    let w2 = wiederherstellen(d, &info.name).unwrap();
    assert!(w2.zurueck.is_empty());
    assert!(w2.gleich.contains(&"book_1".to_string()));
}
