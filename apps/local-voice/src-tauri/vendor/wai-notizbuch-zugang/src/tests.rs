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
