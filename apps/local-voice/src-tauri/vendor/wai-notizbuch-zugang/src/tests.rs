use super::*;
use serde_json::json;

/// Legt eine Ablage im Schema des Notizbuch-Hosts an (Migrationen 1 und 2 aus `notizbuch-store`).
struct Ablage {
    _dir: tempfile::TempDir,
    pfad: std::path::PathBuf,
    conn: Connection,
}

impl Ablage {
    fn neu() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("notizbuch.db");
        let conn = Connection::open(&pfad).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE _wai_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
            INSERT INTO _wai_migrations (version, name) VALUES (1, 'notizbuch_init'), (2, 'notizbuch_fts_rowid');
            CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, daten TEXT NOT NULL, PRIMARY KEY (art, id)) WITHOUT ROWID;
            CREATE TABLE dateien (id TEXT PRIMARY KEY, mime TEXT NOT NULL, daten BLOB NOT NULL);
            CREATE TABLE meta (schluessel TEXT PRIMARY KEY, wert TEXT NOT NULL);
            CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram');
            CREATE TABLE fts_nr (nr INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE);
            CREATE TABLE intern (schluessel TEXT PRIMARY KEY, wert INTEGER NOT NULL);",
        )
        .unwrap();
        let a = Self {
            _dir: dir,
            pfad,
            conn,
        };
        let weg = json!("2026-10-01T00:00:00Z");
        a.satz("notizbuecher", json!({"id": "nb1", "name": "Arbeit"}));
        a.satz("notizbuecher", json!({"id": "nb2", "name": "Privat"}));
        a.satz(
            "notizbuecher",
            json!({"id": "nbx", "name": "Alt", "geloescht": weg}),
        );
        a.satz(
            "gruppen",
            json!({"id": "g1", "name": "Projekte", "notizbuchId": "nb1"}),
        );
        a.satz(
            "abschnitte",
            json!({"id": "a1", "name": "Besprechungen", "notizbuchId": "nb1"}),
        );
        a.satz(
            "abschnitte",
            json!({"id": "a2", "name": "Haushalt", "notizbuchId": "nb1", "gruppeId": "g1"}),
        );
        a.satz(
            "abschnitte",
            json!({"id": "a3", "name": "Geheim", "notizbuchId": "nb2"}),
        );
        a.satz(
            "abschnitte",
            json!({"id": "ax", "name": "Weg", "notizbuchId": "nbx"}),
        );
        a.seite(
            "s1",
            "a1",
            "Jour fixe",
            "Haushaltsplan besprechen, Budget kürzen",
            "2026-11-08T09:00:00Z",
            "2026-11-08T10:30:00Z",
        );
        a.seite(
            "s2",
            "a2",
            "Haushaltsplan 2027",
            "Entwurf Haushaltsplan mit Straßenbau",
            "2026-11-01T08:00:00Z",
            "2026-11-02T08:00:00Z",
        );
        a.seite(
            "s3",
            "a3",
            "Haushaltsplan privat",
            "Haushaltsplan geheim",
            "2026-11-08T09:30:00Z",
            "2026-11-08T09:40:00Z",
        );
        a.seite(
            "sx",
            "ax",
            "Haushaltsplan alt",
            "Haushaltsplan alt",
            "2026-11-08T09:30:00Z",
            "2026-11-08T09:40:00Z",
        );
        a
    }

    fn satz(&self, art: &str, v: Value) {
        let id = v["id"].as_str().unwrap().to_string();
        self.conn
            .execute(
                "INSERT OR REPLACE INTO saetze (art, id, daten) VALUES (?1, ?2, ?3)",
                params![art, id, v.to_string()],
            )
            .unwrap();
    }

    fn seite(
        &self,
        id: &str,
        abschnitt: &str,
        titel: &str,
        text: &str,
        erstellt: &str,
        geaendert: &str,
    ) {
        let inhalt = json!({"type": "doc", "content": [
            {"type": "paragraph", "attrs": {"bid": format!("{id}-b1"), "zeit": erstellt}, "content": [{"type": "text", "text": text}]}
        ]});
        self.satz(
            "seiten",
            json!({"id": id, "abschnittId": abschnitt, "titel": titel, "text": text, "inhalt": inhalt,
                   "erstellt": erstellt, "geaendert": geaendert, "version": 1, "schema": 1, "reihenfolge": 0}),
        );
        self.conn
            .execute("INSERT OR IGNORE INTO fts_nr (id) VALUES (?1)", params![id])
            .unwrap();
        let nr: i64 = self
            .conn
            .query_row("SELECT nr FROM fts_nr WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .unwrap();
        self.conn
            .execute("DELETE FROM seiten_fts WHERE rowid = ?1", params![nr])
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO seiten_fts (rowid, id, titel, text) VALUES (?1, ?2, ?3, ?4)",
                params![nr, id, falte(titel), falte(text)],
            )
            .unwrap();
    }

    fn zugang(&self) -> Zugang {
        Zugang::oeffnen(&self.pfad).unwrap()
    }
}

fn arbeit() -> Freigabe {
    Freigabe::default().notizbuch("nb1")
}

#[test]
fn oeffnen_nur_lesend_und_versionspruefung() {
    let a = Ablage::neu();
    let z = a.zugang();
    assert!(
        z.conn.execute("DELETE FROM saetze", []).is_err(),
        "Zugang darf nicht schreiben"
    );
    a.conn
        .execute(
            "INSERT INTO _wai_migrations (version, name) VALUES (4, 'neu')",
            [],
        )
        .unwrap();
    assert!(matches!(Zugang::oeffnen(&a.pfad), Err(Fehler::Version)));
    assert!(matches!(
        Zugang::oeffnen(a.pfad.with_file_name("fehlt.db")),
        Err(Fehler::Fehlt)
    ));
}

#[test]
fn baum_zeigt_nur_freigegebenes() {
    let a = Ablage::neu();
    let z = a.zugang();
    assert!(z.baum(&Freigabe::default()).unwrap().is_empty());
    let b = z.baum(&arbeit()).unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].name, "Arbeit");
    let namen: Vec<_> = b[0]
        .abschnitte
        .iter()
        .map(|x| (x.name.as_str(), x.seiten))
        .collect();
    assert_eq!(namen, [("Besprechungen", 1), ("Haushalt", 1)]);
    let einzeln = z.baum(&Freigabe::default().abschnitt("a2")).unwrap();
    assert_eq!(einzeln[0].abschnitte.len(), 1);
    // Papierkorb-Notizbuch erscheint auch in der Einstellung nicht.
    let alle = z.gliederung_fuer_einstellung().unwrap();
    assert_eq!(
        alle.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ["nb1", "nb2"]
    );
}

#[test]
fn seite_gleiche_antwort_fuer_verborgen_und_fehlend() {
    let a = Ablage::neu();
    let z = a.zugang();
    let s = z.seite("s2", &arbeit()).unwrap().unwrap();
    assert_eq!(s.kopf.pfad, "Arbeit › Projekte › Haushalt");
    assert_eq!(s.bloecke[0].bid.as_deref(), Some("s2-b1"));
    assert!(!s.gekuerzt && !s.unvollstaendig);
    assert!(z.seite("s3", &arbeit()).unwrap().is_none());
    assert!(
        z.seite("sx", &Freigabe::default().notizbuch("nbx"))
            .unwrap()
            .is_none()
    );
    assert!(z.seite("gibtsnicht", &arbeit()).unwrap().is_none());
    assert!(matches!(z.seite("../x", &arbeit()), Err(Fehler::Eingabe)));
}

#[test]
fn suche_filtert_vor_dem_zaehlen() {
    let a = Ablage::neu();
    let z = a.zugang();
    let e = z.suchen("haushaltsplan", 20, &arbeit()).unwrap();
    assert!(e.genutzt);
    let ids: Vec<_> = e.treffer.iter().map(|t| t.kopf.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"s1") && ids.contains(&"s2"));
    // Titeltreffer zuerst.
    assert_eq!(ids[0], "s2");
    assert!(e.treffer[1].auszug.contains("Haushaltsplan"));
    assert!(
        z.suchen("straße", 20, &arbeit()).unwrap().treffer[0]
            .kopf
            .id
            == "s2"
    );
    assert!(
        z.suchen("haushaltsplan", 20, &Freigabe::default())
            .unwrap()
            .treffer
            .is_empty()
    );
    assert!(!z.suchen("ab", 20, &arbeit()).unwrap().genutzt);
    assert!(matches!(
        z.suchen(&"x ".repeat(11), 5, &arbeit()),
        Err(Fehler::Eingabe)
    ));
}

#[test]
fn zeitraum() {
    let a = Ablage::neu();
    let z = a.zugang();
    let k = z
        .zeitraum(
            "2026-11-08T00:00:00Z",
            "2026-11-09T00:00:00Z",
            50,
            &arbeit(),
        )
        .unwrap();
    assert_eq!(k.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["s1"]);
    assert!(matches!(
        z.zeitraum("2026-11-09", "2026-11-08", 5, &arbeit()),
        Err(Fehler::Eingabe)
    ));
    assert!(matches!(
        z.zeitraum("aaaaaaaaaa", "2026-11-09T00:00:00Z", 5, &arbeit()),
        Err(Fehler::Eingabe)
    ));
    // Sekundengrenze: Millisekunden-Form der Ablage gegen Eingabe ohne Bruchteil.
    a.seite(
        "s4",
        "a1",
        "Grenze",
        "x",
        "2026-11-07T00:00:00.000Z",
        "2026-11-07T00:00:00.000Z",
    );
    let k = z
        .zeitraum("2026-11-07T00:00:00Z", "2026-11-07T00:00:01Z", 5, &arbeit())
        .unwrap();
    assert_eq!(k.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), ["s4"]);
}

#[test]
fn suche_findet_freigegebenes_hinter_vielen_verborgenen() {
    let a = Ablage::neu();
    for i in 0..600 {
        a.seite(
            &format!("v{i}"),
            "a3",
            "Haushaltsplan",
            "Haushaltsplan",
            "2026-11-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
        );
    }
    let e = a.zugang().suchen("haushaltsplan", 5, &arbeit()).unwrap();
    assert_eq!(e.treffer.len(), 2);
}

#[test]
fn titel_ohne_unsichtbare_zeichen_und_stand_bei_umbenennung() {
    let a = Ablage::neu();
    let z = a.zugang();
    let f = arbeit();
    let s0 = z.stand(&f).unwrap();
    a.seite(
        "s1",
        "a1",
        "Jour\u{200B} fixe neu\u{E0041}",
        "x",
        "2026-11-08T09:00:00Z",
        "2026-11-08T10:30:00Z",
    );
    assert_ne!(s0, z.stand(&f).unwrap());
    assert_eq!(
        z.seite("s1", &f).unwrap().unwrap().kopf.titel,
        "Jour fixe neu"
    );
    let s1 = z.stand(&f).unwrap();
    a.satz("notizbuecher", json!({"id": "nb1", "name": "Dienst"}));
    assert_ne!(s1, z.stand(&f).unwrap());
}

#[test]
fn stand_wechselt_bei_papierkorb_und_verschieben() {
    let a = Ablage::neu();
    let z = a.zugang();
    let f = arbeit();
    let s0 = z.stand(&f).unwrap();
    assert_eq!(s0, z.stand(&f).unwrap());
    // Abschnitt in den Papierkorb, Seite selbst unverändert.
    a.satz("abschnitte", json!({"id": "a2", "name": "Haushalt", "notizbuchId": "nb1", "gruppeId": "g1", "geloescht": "2026-11-08T11:00:00Z"}));
    let s1 = z.stand(&f).unwrap();
    assert_ne!(s0, s1);
    assert!(
        z.seite("s2", &f).unwrap().is_none(),
        "Prüfung bei jedem Zugriff"
    );
    // Seite in einen nicht freigegebenen Abschnitt verschoben (gleicher Zeitstempel).
    a.satz(
        "seiten",
        json!({"id": "s1", "abschnittId": "a3", "titel": "Jour fixe", "text": "", "inhalt": {},
        "erstellt": "2026-11-08T09:00:00Z", "geaendert": "2026-11-08T10:30:00Z", "version": 1}),
    );
    assert_ne!(s1, z.stand(&f).unwrap());
    assert!(z.seite("s1", &f).unwrap().is_none());
    assert_ne!(z.stand(&Freigabe::default()).unwrap(), z.stand(&f).unwrap());
}

#[test]
fn faltung_wie_im_notizbuch() {
    assert_eq!(falte("Straße ÄÖÜ Œuvre"), "strasse aou oeuvre");
    assert_eq!(
        fts_anfrage("Groß \"x\"y"),
        Some("\"gross\" AND \"\"\"x\"\"y\"".into())
    );
    assert_eq!(fts_anfrage("ab cde"), None);
}

// ------------------------------------------------------------ Codex lva#414 (Härtung)

thread_local! {
    /// Wird in `mit_stand` zwischen Stand und Ergebnis ausgeführt (nur Tests): simuliert eine
    /// gleichzeitige Änderung durch das Notizbuch.
    static ZWISCHEN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn zwischenschritt() {
    if let Some(f) = ZWISCHEN.with(|z| z.borrow_mut().take()) {
        f();
    }
}

fn zwischen(f: impl FnOnce() + 'static) {
    ZWISCHEN.with(|z| *z.borrow_mut() = Some(Box::new(f)));
}

#[test]
fn schema_wird_strukturell_geprueft() {
    // Richtige Version, aber `saetze` ohne Spalte `daten`.
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("notizbuch.db");
    let c = Connection::open(&pfad).unwrap();
    c.execute_batch(
        "CREATE TABLE _wai_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL);
         INSERT INTO _wai_migrations VALUES (2, 'x');
         CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, inhalt TEXT, PRIMARY KEY (art, id));
         CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram');",
    )
    .unwrap();
    assert!(matches!(Zugang::oeffnen(&pfad), Err(Fehler::Version)));
    // `seiten_fts` als gewöhnliche Tabelle statt FTS5.
    c.execute_batch(
        "DROP TABLE saetze; CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, daten TEXT NOT NULL, PRIMARY KEY (art, id));
         DROP TABLE seiten_fts; CREATE TABLE seiten_fts (id TEXT, titel TEXT, text TEXT);",
    )
    .unwrap();
    assert!(matches!(Zugang::oeffnen(&pfad), Err(Fehler::Version)));
    // `saetze` ohne Eindeutigkeit über (art, id).
    c.execute_batch(
        "DROP TABLE seiten_fts; CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram');
         DROP TABLE saetze; CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, daten TEXT NOT NULL);",
    )
    .unwrap();
    assert!(matches!(Zugang::oeffnen(&pfad), Err(Fehler::Version)));
    // Vollständig: geht.
    c.execute_batch(
        "DROP TABLE saetze; CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, daten TEXT NOT NULL, PRIMARY KEY (art, id)) WITHOUT ROWID;",
    )
    .unwrap();
    assert!(Zugang::oeffnen(&pfad).is_ok());
    // Die Ablage des Fixture-Schemas (wie notizbuch-store) geht ebenfalls.
    assert!(Zugang::oeffnen(&Ablage::neu().pfad).is_ok());
}

#[test]
fn quelle_ist_die_db_kennung_nicht_das_json() {
    let a = Ablage::neu();
    // Zeile `s9`, deren JSON sich als `s1` ausgibt: gilt als beschädigt und ist nicht sichtbar.
    let inhalt = json!({"type": "doc", "content": []});
    a.conn
        .execute(
            "INSERT INTO saetze (art, id, daten) VALUES ('seiten', 's9', ?1)",
            params![json!({"id": "s1", "abschnittId": "a1", "titel": "Täuschung", "text": "Haushaltsplan",
                "inhalt": inhalt, "erstellt": "2026-11-08T09:00:00Z", "geaendert": "2026-11-08T09:00:00Z"}).to_string()],
        )
        .unwrap();
    let z = a.zugang();
    let f = arbeit();
    assert!(z.seite("s9", &f).unwrap().is_none());
    let k = z
        .zeitraum("2026-11-08T00:00:00Z", "2026-11-09T00:00:00Z", 50, &f)
        .unwrap();
    assert!(k.iter().all(|k| k.titel != "Täuschung"));
    assert_eq!(
        z.baum(&f).unwrap()[0].abschnitte[0].seiten,
        1,
        "beschädigter Satz zählt nicht"
    );
    // Ohne `id` im JSON gilt die Zeilenkennung.
    a.conn
        .execute(
            "INSERT INTO saetze (art, id, daten) VALUES ('seiten', 's8', ?1)",
            params![
                json!({"abschnittId": "a1", "titel": "Ohne", "text": "", "inhalt": {},
                "erstellt": "2026-11-08T09:00:00Z", "geaendert": "2026-11-08T09:00:00Z"})
                .to_string()
            ],
        )
        .unwrap();
    assert_eq!(z.seite("s8", &f).unwrap().unwrap().kopf.id, "s8");
    // Sichtbarkeitsprüfung schon gelieferter Ergebnisse folgt derselben Regel.
    let ids: Vec<String> = ["s1", "s8", "s9", "s3"].map(String::from).to_vec();
    let s = z.sichtbare_seiten(&ids, &f).unwrap();
    assert_eq!(
        s.wert.into_iter().collect::<Vec<_>>(),
        ["s1".to_string(), "s8".to_string()]
    );
    assert_eq!(s.stand, z.stand(&f).unwrap());
}

/// Rangfolge nur aus dem sichtbaren Bestand (Codex lva#414 M3): zwei Ablagen mit gleichem sichtbarem
/// Inhalt, die zweite mit vielen verborgenen Seiten (anderer Abschnitt, Papierkorb). `bm25()` des
/// Index kehrt die Reihenfolge dort um; die Suche des Zugangs nicht.
#[test]
fn rangfolge_unabhaengig_von_verborgenem() {
    let fuellen = |a: &Ablage| {
        a.seite(
            "d1",
            "a1",
            "Notiz eins",
            "alpha alpha alpha alpha alpha bravo",
            "2026-11-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
        );
        a.seite(
            "d2",
            "a1",
            "Notiz zwei",
            "alpha bravo bravo bravo bravo",
            "2026-11-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
        );
    };
    let ohne = Ablage::neu();
    fuellen(&ohne);
    let mit = Ablage::neu();
    fuellen(&mit);
    for i in 0..40 {
        mit.seite(
            &format!("v{i}"),
            if i % 2 == 0 { "a3" } else { "ax" },
            "verborgen",
            "bravo",
            "2026-11-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
        );
    }
    let roh = |a: &Ablage| -> Vec<String> {
        let mut st = a
            .conn
            .prepare(
                "SELECT id FROM seiten_fts WHERE seiten_fts MATCH '\"alpha\" AND \"bravo\"' \
                 AND id IN ('d1', 'd2') ORDER BY bm25(seiten_fts, 0.0, 10.0, 1.0)",
            )
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_ne!(
        roh(&ohne),
        roh(&mit),
        "Vorbedingung: verborgene Seiten verschieben bm25()"
    );
    let ids = |a: &Ablage| -> Vec<String> {
        a.zugang()
            .suchen("alpha bravo", 10, &arbeit())
            .unwrap()
            .treffer
            .into_iter()
            .map(|t| t.kopf.id)
            .collect()
    };
    assert_eq!(ids(&ohne).len(), 2);
    assert_eq!(ids(&ohne), ids(&mit));
}

#[test]
fn zeitraum_nur_angelegt_oder_geaendert_im_fenster() {
    let a = Ablage::neu();
    // Vor dem Fenster angelegt, erst danach geändert: nicht im Fenster.
    a.seite(
        "lang",
        "a1",
        "Lang",
        "x",
        "2026-11-01T00:00:00Z",
        "2026-11-20T00:00:00Z",
    );
    // Gemischte Zeitformen: sortiert nach normalisierten Werten.
    a.seite(
        "frueh",
        "a1",
        "Früh",
        "x",
        "2026-11-08T08:00:00Z",
        "2026-11-08T08:00:00.5Z",
    );
    a.seite(
        "spaet",
        "a1",
        "Spät",
        "x",
        "2026-11-08T08:00:00Z",
        "2026-11-08T08:00:00.100Z",
    );
    let k = a
        .zugang()
        .zeitraum(
            "2026-11-08T00:00:00Z",
            "2026-11-09T00:00:00Z",
            50,
            &arbeit(),
        )
        .unwrap();
    assert_eq!(
        k.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
        ["s1", "frueh", "spaet"]
    );
}

#[test]
fn vorfahren_im_papierkorb_verbergen() {
    let a = Ablage::neu();
    let z = a.zugang();
    let f = arbeit();
    assert!(z.seite("s2", &f).unwrap().is_some());
    // Gruppe im Papierkorb: Abschnitt a2 samt Seite verschwindet, a1 bleibt.
    a.satz(
        "gruppen",
        json!({"id": "g1", "name": "Projekte", "notizbuchId": "nb1", "geloescht": "2026-11-08T12:00:00Z"}),
    );
    assert!(z.seite("s2", &f).unwrap().is_none());
    assert!(z.seite("s1", &f).unwrap().is_some());
    assert!(
        z.suchen("haushaltsplan", 20, &f)
            .unwrap()
            .treffer
            .iter()
            .all(|t| t.kopf.id != "s2")
    );
    // Notizbuch im Papierkorb: alles darunter weg, auch bei Einzelfreigabe des Abschnitts.
    a.satz(
        "notizbuecher",
        json!({"id": "nb1", "name": "Arbeit", "geloescht": "2026-11-08T12:00:00Z"}),
    );
    assert!(
        z.seite("s1", &Freigabe::default().abschnitt("a1"))
            .unwrap()
            .is_none()
    );
    assert!(z.baum(&f).unwrap().is_empty());
    // Fehlende Gruppe (gelöscht statt Papierkorb) verbirgt ebenso.
    a.satz("notizbuecher", json!({"id": "nb1", "name": "Arbeit"}));
    a.conn
        .execute("DELETE FROM saetze WHERE art = 'gruppen' AND id = 'g1'", [])
        .unwrap();
    assert!(z.seite("s2", &f).unwrap().is_none());
}

#[test]
fn verschieben_innerhalb_und_aus_der_freigabe() {
    let a = Ablage::neu();
    let z = a.zugang();
    let f = arbeit();
    let s0 = z.stand(&f).unwrap();
    // Innerhalb der Freigabe verschoben: sichtbar, neuer Pfad, neuer Stand.
    a.seite(
        "s1",
        "a2",
        "Jour fixe",
        "Haushaltsplan besprechen, Budget kürzen",
        "2026-11-08T09:00:00Z",
        "2026-11-08T10:30:00Z",
    );
    let s = z.seite("s1", &f).unwrap().unwrap();
    assert_eq!(s.kopf.pfad, "Arbeit › Projekte › Haushalt");
    let s1 = z.stand(&f).unwrap();
    assert_ne!(s0, s1);
    // Abschnitt in ein anderes Notizbuch verschoben: bei Notizbuch-Freigabe nicht mehr sichtbar.
    a.satz(
        "abschnitte",
        json!({"id": "a2", "name": "Haushalt", "notizbuchId": "nb2"}),
    );
    assert!(z.seite("s1", &f).unwrap().is_none());
    assert!(z.seite("s2", &f).unwrap().is_none());
    assert_ne!(s1, z.stand(&f).unwrap());
    // Mit Einzelfreigabe des Abschnitts bleibt er sichtbar (Freigabe hängt am Abschnitt).
    assert!(
        z.seite("s1", &Freigabe::default().abschnitt("a2"))
            .unwrap()
            .is_some()
    );
}

/// Laufender Widerruf (Codex lva#414 M2): Zwischen Stand und Lesen verschiebt das Notizbuch die Seite
/// aus der Freigabe. Ergebnis und Stand stammen trotzdem aus demselben Schnappschuss; der nächste
/// Aufruf sieht beides neu.
#[test]
fn laufender_widerruf_trifft_stand_und_ergebnis_gemeinsam() {
    let a = Ablage::neu();
    // Der Test setzt WAL voraus: nur dann darf der Schreiber während der Lesetransaktion schreiben,
    // und der Leser behält seinen Schnappschuss.
    let modus: String = a
        .conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(modus, "wal");
    let z = a.zugang();
    let f = arbeit();
    let vorher = z.stand(&f).unwrap();
    let pfad = a.pfad.clone();
    zwischen(move || {
        let w = Connection::open(&pfad).unwrap();
        w.busy_timeout(std::time::Duration::from_secs(2)).unwrap();
        let alt: String = w
            .query_row(
                "SELECT daten FROM saetze WHERE art = 'seiten' AND id = 's1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut v: Value = serde_json::from_str(&alt).unwrap();
        v["abschnittId"] = json!("a3");
        w.execute(
            "UPDATE saetze SET daten = ?1 WHERE art = 'seiten' AND id = 's1'",
            params![v.to_string()],
        )
        .unwrap();
    });
    let r = z.seite_mit_stand("s1", &f).unwrap();
    assert!(r.wert.is_some(), "Schnappschuss vor dem Verschieben");
    assert_eq!(r.stand, vorher, "Stand passt zum gelieferten Ergebnis");
    // Danach: Seite weg, Stand neu, und wieder beides aus einem Schnappschuss.
    let n = z.seite_mit_stand("s1", &f).unwrap();
    assert!(n.wert.is_none());
    assert_ne!(n.stand, vorher);
    assert_eq!(n.stand, z.stand(&f).unwrap());
    // Gleiches für Suche und Zeitraum.
    let s = z.suchen_mit_stand("haushaltsplan", 20, &f).unwrap();
    assert_eq!(s.stand, n.stand);
    assert!(s.wert.treffer.iter().all(|t| t.kopf.id != "s1"));
    let k = z
        .zeitraum_mit_stand("2026-11-01T00:00:00Z", "2026-12-01T00:00:00Z", 50, &f)
        .unwrap();
    assert_eq!(k.stand, n.stand);
    let b = z.baum_mit_stand(&f).unwrap();
    assert_eq!(b.stand, n.stand);
}

/// Eine Seitenzeile mit frei wählbarer DB-Kennung und JSON-`id` samt Indexzeile.
fn seite_mit_kennung(a: &Ablage, db_id: &str, json_id: Value, abschnitt: &str, text: &str) {
    let mut v = json!({"abschnittId": abschnitt, "titel": "x", "text": text, "inhalt": {},
        "erstellt": "2026-11-01T00:00:00Z", "geaendert": "2026-11-01T00:00:00Z"});
    if !json_id.is_null() || db_id.starts_with("null") {
        v["id"] = json_id;
    }
    a.conn
        .execute(
            "INSERT OR REPLACE INTO saetze (art, id, daten) VALUES ('seiten', ?1, ?2)",
            params![db_id, v.to_string()],
        )
        .unwrap();
    a.conn
        .execute(
            "INSERT INTO seiten_fts (id, titel, text) VALUES (?1, 'x', ?2)",
            params![db_id, falte(text)],
        )
        .unwrap();
}

/// Codex lva#414 (P1): Seiten mit ungültiger DB-Kennung oder abweichender JSON-`id` im freigegebenen
/// Abschnitt zählen weder in Seitenzahl, mittlere Länge noch Dokumenthäufigkeit. Kontrolle: dieselben
/// Seiten mit gültiger Identität verschieben die Rangfolge, der Test misst also etwas.
#[test]
fn identitaet_vor_allen_aggregaten() {
    let fuellen = |a: &Ablage| {
        a.seite(
            "d1",
            "a1",
            "Notiz eins",
            "alpha alpha alpha alpha alpha bravo",
            "2026-11-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
        );
        a.seite(
            "d2",
            "a1",
            "Notiz zwei",
            "alpha bravo bravo bravo bravo",
            "2026-11-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
        );
    };
    let ids = |a: &Ablage| -> Vec<String> {
        a.zugang()
            .suchen("alpha bravo", 10, &arbeit())
            .unwrap()
            .treffer
            .into_iter()
            .map(|t| t.kopf.id)
            .collect()
    };
    let ohne = Ablage::neu();
    fuellen(&ohne);
    let kaputt = Ablage::neu();
    fuellen(&kaputt);
    let kontrolle = Ablage::neu();
    fuellen(&kontrolle);
    for i in 0..40 {
        let (db, js) = match i % 5 {
            0 => (format!("k{i}"), json!("d1")),
            1 => (format!("k{i}"), json!(i)),
            2 => (format!("böse {i}"), Value::Null),
            3 => (format!("k{i}\u{0}x"), Value::Null),
            _ => (format!("k{i}.x"), Value::Null),
        };
        seite_mit_kennung(&kaputt, &db, js, "a1", "bravo");
        seite_mit_kennung(&kontrolle, &format!("k{i}"), Value::Null, "a1", "bravo");
    }
    assert_eq!(ids(&ohne).len(), 2);
    assert_ne!(
        ids(&ohne),
        ids(&kontrolle),
        "Vorbedingung: gültige Seiten verschieben die Rangfolge"
    );
    assert_eq!(ids(&ohne), ids(&kaputt));
    // Auch einzeln gesucht tauchen die beschädigten Sätze nicht auf.
    assert!(
        kaputt
            .zugang()
            .suchen("bravo", 20, &arbeit())
            .unwrap()
            .treffer
            .iter()
            .all(|t| t.kopf.id == "d1" || t.kopf.id == "d2")
    );
}

/// Codex lva#414 (P2): der beste Treffer steht nach Kennung hinter 5 000 anderen Kandidaten und wird
/// trotzdem gefunden (kein LIMIT vor dem Ranking).
#[test]
fn bester_treffer_hinter_vielen_kandidaten() {
    let a = Ablage::neu();
    a.conn.execute_batch("BEGIN").unwrap();
    for i in 0..5_001 {
        seite_mit_kennung(
            &a,
            &format!("m{i:05}"),
            Value::Null,
            "a1",
            "ein langer text mit dem wort zielwort und viel fuelltext drumherum",
        );
    }
    seite_mit_kennung(&a, "zz-bester", Value::Null, "a1", "zielwort zielwort");
    a.conn.execute_batch("COMMIT").unwrap();
    let t = a.zugang().suchen("zielwort", 3, &arbeit()).unwrap().treffer;
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].kopf.id, "zz-bester");
    // Gleichstand: Kennung aufsteigend.
    assert_eq!(t[1].kopf.id, "m00000");
    assert_eq!(t[2].kopf.id, "m00001");
}

/// Häufigkeitszählung und Titelgewicht des gewichteten Substring-BM25, getrennt geprüft.
#[test]
fn substring_bm25_zaehlung_und_titelgewicht() {
    let w = |s: &str| vec![s.to_string()];
    let nah = |x: f64, y: f64| (x - y).abs() < 1e-9;
    // Nicht überlappende Vorkommen: "aaaa" enthält "aa" zweimal, "aaa" einmal.
    // Mittlere Länge = eigene Länge, also Normierung 1: Wert = tf·2,2 / (tf + 1,2).
    assert!(nah(
        substring_bm25(&w("aa"), &[1.0], "", "aaaa", 4.0),
        2.0 * 2.2 / 3.2
    ));
    assert!(nah(substring_bm25(&w("aa"), &[1.0], "", "aaa", 3.0), 1.0));
    assert!(nah(substring_bm25(&w("xyz"), &[1.0], "", "abc", 3.0), 0.0));
    // Ein Titeltreffer zählt zehnfach: tf = 10.
    assert!(nah(
        substring_bm25(&w("abc"), &[1.0], "abc", "", 3.0),
        10.0 * 2.2 / 11.2
    ));
    // Gleich lang, gleiches Wort: Titel vor Text.
    assert!(
        substring_bm25(&w("abc"), &[1.0], "abc", "def", 6.0)
            > substring_bm25(&w("abc"), &[1.0], "def", "abc", 6.0)
    );
    // IDF skaliert linear, Wörter addieren sich.
    let zwei = substring_bm25(
        &["abc".to_string(), "def".to_string()],
        &[2.0, 1.0],
        "",
        "abc def",
        7.0,
    );
    let einzeln = substring_bm25(&w("abc"), &[2.0], "", "abc def", 7.0)
        + substring_bm25(&w("def"), &[1.0], "", "abc def", 7.0);
    assert!(nah(zwei, einzeln));
}

/// Codex lva#414 (P2): Schemaumgehungen per Teilindex, abweichender Sortierfolge und vorgetäuschter
/// FTS5-Tabelle werden abgewiesen.
#[test]
fn schemaumgehungen_werden_abgewiesen() {
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("notizbuch.db");
    let c = Connection::open(&pfad).unwrap();
    let fts = "CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram');";
    c.execute_batch(&format!(
        "CREATE TABLE _wai_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL);
         INSERT INTO _wai_migrations VALUES (2, 'x');
         CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, daten TEXT NOT NULL);
         {fts}"
    ))
    .unwrap();
    let abgewiesen = |was: &str| {
        assert!(
            matches!(Zugang::oeffnen(&pfad), Err(Fehler::Version)),
            "nicht abgewiesen: {was}"
        );
    };
    // Eindeutiger Teilindex, der nichts erfasst.
    c.execute_batch("CREATE UNIQUE INDEX u ON saetze(art, id) WHERE 0;")
        .unwrap();
    abgewiesen("Teilindex WHERE 0");
    // Eindeutig nur ohne Groß-/Kleinschreibung.
    c.execute_batch("DROP INDEX u; CREATE UNIQUE INDEX u ON saetze(art, id COLLATE NOCASE);")
        .unwrap();
    abgewiesen("NOCASE-Index");
    // Richtiger Index: geht.
    c.execute_batch("DROP INDEX u; CREATE UNIQUE INDEX u ON saetze(art, id);")
        .unwrap();
    assert!(Zugang::oeffnen(&pfad).is_ok());
    let ersetzen = |neu: &str, was: &str| {
        c.execute_batch(&format!("DROP TABLE seiten_fts; {neu}"))
            .unwrap();
        abgewiesen(was);
    };
    ersetzen(
        "CREATE TABLE seiten_fts (id TEXT, titel TEXT, text TEXT /* using fts5 */);",
        "gewöhnliche Tabelle mit Kommentar",
    );
    ersetzen(
        "CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text);",
        "FTS5 ohne Trigramm",
    );
    ersetzen(
        "CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram', content = '');",
        "inhaltslose FTS5",
    );
    ersetzen(
        "CREATE VIRTUAL TABLE seiten_fts USING fts5(id, titel, text, tokenize = 'trigram');",
        "id indiziert",
    );
    ersetzen(
        "CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram' /* x */);",
        "Kommentar im Vertrag",
    );
    // Gleicher Vertrag, andere Schreibweise (Leerraum, Großschreibung): geht.
    c.execute_batch(
        "DROP TABLE seiten_fts; create virtual table seiten_fts using FTS5(
            id unindexed,  titel,
            text, tokenize='trigram');",
    )
    .unwrap();
    assert!(Zugang::oeffnen(&pfad).is_ok());
}

/// Zeitraum (Codex lva#414): Kalenderprüfung, volle Nachkommastellen, Fenstergrenzen.
#[test]
fn zeitraum_kalender_und_grenzen() {
    for falsch in [
        "2026-02-29T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-10T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-11-00T00:00:00Z",
        "2026-11-08T24:00:00Z",
        "2026-11-08T23:60:00Z",
        "2026-11-08T23:59:60Z",
        "2026-11-08T12:00:00.Z",
        "2026-11-08T12:00:00.1234567890Z",
        "2026-11-08T12:00:00+01:00",
    ] {
        assert_eq!(iso_norm(falsch), None, "{falsch}");
    }
    assert!(iso_norm("2028-02-29T00:00:00Z").is_some());
    assert!(iso_norm("2000-02-29T00:00:00Z").is_some());
    assert_eq!(iso_norm("1900-02-29T00:00:00Z"), None);
    assert_eq!(
        iso_norm("2026-11-08T12:00:00Z").unwrap(),
        "2026-11-08T12:00:00.000000000Z"
    );
    // Kein Abschneiden: 0,5 ms liegt nach 0 ms.
    assert!(iso_norm("2026-11-08T12:00:00.0005Z") > iso_norm("2026-11-08T12:00:00.000Z"));

    let a = Ablage::neu();
    let z = a.zugang();
    let f = arbeit();
    assert!(matches!(
        z.zeitraum("2026-02-30T00:00:00Z", "2026-12-01T00:00:00Z", 10, &f),
        Err(Fehler::Eingabe)
    ));
    // Leeres Fenster ist ein Eingabefehler.
    assert!(matches!(
        z.zeitraum("2026-11-08T10:30:00Z", "2026-11-08T10:30:00Z", 10, &f),
        Err(Fehler::Eingabe)
    ));
    // s1: erstellt 09:00, geändert 10:30. Anfang einschließlich, Ende ausschließlich.
    let ids = |von: &str, bis: &str| -> Vec<String> {
        z.zeitraum(von, bis, 50, &f)
            .unwrap()
            .into_iter()
            .map(|k| k.id)
            .collect()
    };
    assert!(ids("2026-11-08T10:30:00Z", "2026-11-08T11:00:00Z").contains(&"s1".to_string()));
    assert!(!ids("2026-11-08T09:30:00Z", "2026-11-08T10:30:00Z").contains(&"s1".to_string()));
    assert!(
        ids("2026-11-08T09:00:00Z", "2026-11-08T09:00:00.000000001Z").contains(&"s1".to_string())
    );
    assert!(!ids("2026-11-08T10:30:00.0001Z", "2026-11-08T11:00:00Z").contains(&"s1".to_string()));
}

/// Codex #1089: eine DB-Kennung mit NUL (`abc\0x`) besteht `length`/`GLOB` in SQL, nicht aber
/// `gueltige_id`. Sie darf weder zählen noch einen Platz im Top-k-Heap belegen.
#[test]
fn nul_in_der_kennung_belegt_keinen_platz() {
    let a = Ablage::neu();
    seite_mit_kennung(
        &a,
        "abc\u{0}x",
        Value::Null,
        "a1",
        "zielwort zielwort zielwort",
    );
    seite_mit_kennung(&a, "gut", Value::Null, "a1", "zielwort und mehr text");
    let t = a.zugang().suchen("zielwort", 1, &arbeit()).unwrap().treffer;
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].kopf.id, "gut");
    // Aggregat: dieselbe Seitenzahl wie ohne die NUL-Zeile.
    let n: i64 = a
        .conn
        .query_row(
            &format!("SELECT COUNT(*) FROM seiten_fts {SICHTBAR}"),
            params![r#"["a1"]"#],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2, "s1 und gut, nicht abc\\0x");
}
