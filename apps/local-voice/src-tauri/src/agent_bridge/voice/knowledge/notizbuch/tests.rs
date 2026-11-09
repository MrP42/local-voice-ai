use std::cell::Cell;
use std::path::PathBuf;

use chrono::{Local, TimeZone};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use wai_notizbuch_zugang::falte;

use super::super::{
    run as knowledge_run, Context, ConversationInfo, ConversationView, Dictation, EntryInfo,
    EventInfo, KnowledgeSource, MeetingInfo, PageInfo,
};
use super::*;
use crate::agent_bridge::voice::areas::Areas;

/// Synthetische Ablage im Schema des Notizbuch-Hosts (Migrationen 1 bis 3).
struct Ablage {
    _dir: tempfile::TempDir,
    pfad: PathBuf,
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
            INSERT INTO _wai_migrations (version, name) VALUES (1, 'notizbuch_init'), (2, 'notizbuch_fts_rowid'), (3, 'assistent_anfragen');
            CREATE TABLE saetze (art TEXT NOT NULL, id TEXT NOT NULL, daten TEXT NOT NULL, PRIMARY KEY (art, id)) WITHOUT ROWID;
            CREATE VIRTUAL TABLE seiten_fts USING fts5(id UNINDEXED, titel, text, tokenize = 'trigram');
            CREATE TABLE fts_nr (nr INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE);",
        )
        .unwrap();
        let a = Self {
            _dir: dir,
            pfad,
            conn,
        };
        a.satz("notizbuecher", json!({"id": "nb1", "name": "Arbeit"}));
        a.satz("notizbuecher", json!({"id": "nb2", "name": "Privat"}));
        a.satz(
            "abschnitte",
            json!({"id": "a1", "name": "Besprechungen", "notizbuchId": "nb1"}),
        );
        a.satz(
            "abschnitte",
            json!({"id": "a2", "name": "Ideen", "notizbuchId": "nb1"}),
        );
        a.satz(
            "abschnitte",
            json!({"id": "a3", "name": "Geheim", "notizbuchId": "nb2"}),
        );
        a.seite(
            "s1",
            "a1",
            "Jour fixe",
            "Haushaltsplan besprechen",
            "2026-11-05T09:00:00Z",
            "2026-11-05T10:30:00Z",
        );
        a.seite(
            "s2",
            "a2",
            "Radwege",
            "Haushaltsplan Radwege ergänzen",
            "2026-11-04T08:00:00Z",
            "2026-11-04T08:00:00Z",
        );
        a.seite(
            "s3",
            "a3",
            "Haushaltsplan privat",
            "Haushaltsplan geheim",
            "2026-11-05T09:30:00Z",
            "2026-11-05T09:40:00Z",
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
            {"type": "paragraph", "attrs": {"bid": format!("{id}b1"), "zeit": erstellt, "zeit_geaendert": geaendert},
             "content": [{"type": "text", "text": text}]}
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
            .execute(
                "INSERT INTO seiten_fts (rowid, id, titel, text) VALUES (?1, ?2, ?3, ?4)",
                params![nr, id, falte(titel), falte(text)],
            )
            .unwrap();
    }
}

/// Quelle mit Notizbuch; zählt, wie oft die Ablage geöffnet wurde.
struct Quelle {
    pfad: PathBuf,
    freigabe: Vec<String>,
    geoeffnet: Cell<usize>,
}

impl Quelle {
    fn neu(a: &Ablage, freigabe: &[&str]) -> Self {
        Self {
            pfad: a.pfad.clone(),
            freigabe: freigabe.iter().map(|s| s.to_string()).collect(),
            geoeffnet: Cell::new(0),
        }
    }
}

impl KnowledgeSource for Quelle {
    fn dictations(
        &self,
        _: Option<i64>,
        _: Option<i64>,
        _: usize,
    ) -> Result<Vec<Dictation>, String> {
        Err("nicht im Test".into())
    }
    fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
        Err("nicht im Test".into())
    }
    fn conversation_entries(&self, _: usize) -> Result<Vec<EntryInfo>, String> {
        Err("nicht im Test".into())
    }
    fn conversation(&self, _: &str) -> Result<ConversationView, String> {
        Err("nicht im Test".into())
    }
    fn calendar(&self, _: i64, _: i64) -> Result<Vec<EventInfo>, String> {
        Err("nicht im Test".into())
    }
    fn meetings(&self, _: i64, _: i64, _: usize) -> Result<Vec<MeetingInfo>, String> {
        Err("nicht im Test".into())
    }
    fn pages(&self) -> Result<Vec<PageInfo>, String> {
        Err("nicht im Test".into())
    }
    fn page_text(&self, _: &str) -> Result<String, String> {
        Err("nicht im Test".into())
    }
    fn notizbuch(&self) -> Result<Sicht, String> {
        self.geoeffnet.set(self.geoeffnet.get() + 1);
        Ok(Sicht {
            zugang: Zugang::oeffnen(&self.pfad).map_err(|e| fehlertext(&e))?,
            freigabe: freigabe_aus_einstellung(&self.freigabe),
        })
    }
}

fn jetzt() -> i64 {
    Local
        .with_ymd_and_hms(2026, 11, 6, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis()
}

fn ctx(bereiche: &[&str]) -> Context<'static> {
    let keys: Vec<String> = bereiche.iter().map(|s| s.to_string()).collect();
    Context {
        areas: Areas::from_settings(&keys, false),
        current_conversation: None,
        now_ms: jetzt(),
    }
}

fn aufruf(q: &Quelle, tool: &str, args: Value) -> Result<Value, String> {
    knowledge_run(q, &ctx(&["notebook"]), tool, &args)
}

#[test]
fn einstellung_wird_streng_gelesen_und_normalisiert() {
    let roh: Vec<String> = [
        "abschnitt:a1",
        "notizbuch:nb2",
        "abschnitt:a1",
        "alles",
        "abschnitt:",
        "x:y",
        "abschnitt:a b",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        eintraege_normalisieren(&roh),
        vec!["abschnitt:a1".to_string(), "notizbuch:nb2".to_string()]
    );
    assert!(freigabe_aus_einstellung(&[]).ist_leer());
    assert!(freigabe_aus_einstellung(&["alles".to_string()]).ist_leer());
    assert!(!freigabe_aus_einstellung(&roh).ist_leer());
}

#[test]
fn ohne_bereich_oder_ohne_abschnitt_wird_nichts_gelesen() {
    let a = Ablage::neu();
    let q = Quelle::neu(&a, &["notizbuch:nb1"]);
    let err = knowledge_run(&q, &ctx(&["meetings"]), "notizbuch_baum", &json!({})).unwrap_err();
    assert!(err.contains("nicht freigegeben"), "{err}");
    assert_eq!(q.geoeffnet.get(), 0);

    let leer = Quelle::neu(&a, &[]);
    let err = aufruf(
        &leer,
        "notizbuch_suchen",
        json!({"anfrage": "Haushaltsplan"}),
    )
    .unwrap_err();
    assert!(err.contains("noch kein Abschnitt freigegeben"), "{err}");
}

#[test]
fn suche_nur_in_freigegebenen_abschnitten_mit_quelle_und_stand() {
    let a = Ablage::neu();
    let q = Quelle::neu(&a, &["abschnitt:a1", "abschnitt:a2"]);
    let v = aufruf(&q, "notizbuch_suchen", json!({"anfrage": "Haushaltsplan"})).unwrap();
    let ids: Vec<&str> = v["treffer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["seite_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "{v}");
    assert!(
        !ids.contains(&"s3"),
        "verborgener Abschnitt bleibt verborgen"
    );
    for t in v["treffer"].as_array().unwrap() {
        let id = t["seite_id"].as_str().unwrap();
        assert_eq!(t["quelle"], json!(format!("wai://notizbuch/seite/{id}")));
        assert!(t["auszug"].as_str().unwrap().contains("Haushaltsplan"));
        assert!(t["pfad"].as_str().unwrap().starts_with("Arbeit"));
    }
    assert_eq!(v["stand"].as_str().unwrap().len(), 64);
    // Ein anderer Freigabestand ergibt einen anderen Hash.
    let nur_a1 = Quelle::neu(&a, &["abschnitt:a1"]);
    let w = aufruf(
        &nur_a1,
        "notizbuch_suchen",
        json!({"anfrage": "Haushaltsplan"}),
    )
    .unwrap();
    assert_eq!(w["treffer"].as_array().unwrap().len(), 1);
    assert_ne!(v["stand"], w["stand"]);
}

#[test]
fn seite_mit_bloecken_verborgen_und_unbekannt_sind_gleich() {
    let a = Ablage::neu();
    let q = Quelle::neu(&a, &["notizbuch:nb1"]);
    let v = aufruf(&q, "notizbuch_seite", json!({"seite_id": "s1"})).unwrap();
    assert_eq!(v["titel"], json!("Jour fixe"));
    assert_eq!(v["quelle"], json!("wai://notizbuch/seite/s1"));
    assert_eq!(v["gekuerzt"], json!(false));
    let b = &v["bloecke"][0];
    assert_eq!(b["bid"], json!("s1b1"));
    assert_eq!(b["text"], json!("Haushaltsplan besprechen"));
    assert!(b["zeit"].is_string() && b["zeit_geaendert"].is_string());

    let verborgen = aufruf(&q, "notizbuch_seite", json!({"seite_id": "s3"})).unwrap_err();
    let unbekannt = aufruf(&q, "notizbuch_seite", json!({"seite_id": "s9"})).unwrap_err();
    let ungueltig = aufruf(&q, "notizbuch_seite", json!({"seite_id": "s 9"})).unwrap_err();
    assert_eq!(verborgen, NICHT_GEFUNDEN);
    assert_eq!(unbekannt, verborgen);
    assert_eq!(ungueltig, verborgen);
}

#[test]
fn lange_seite_bleibt_unter_dem_gespraechsbudget() {
    let a = Ablage::neu();
    let bloecke: Vec<Value> = (0..200)
        .map(|i| json!({"type": "paragraph", "attrs": {"bid": format!("lb{i}")},
                         "content": [{"type": "text", "text": format!("Absatz {i} {}", "x".repeat(80))}]}))
        .collect();
    a.satz(
        "seiten",
        json!({"id": "lang", "abschnittId": "a1", "titel": "Lang", "text": "", "inhalt": {"type": "doc", "content": bloecke},
               "erstellt": "2026-11-05T09:00:00Z", "geaendert": "2026-11-05T09:00:00Z", "version": 1, "schema": 1, "reihenfolge": 1}),
    );
    let q = Quelle::neu(&a, &["abschnitt:a1"]);
    let v = aufruf(&q, "notizbuch_seite", json!({"seite_id": "lang"})).unwrap();
    assert_eq!(v["gekuerzt"], json!(true));
    assert!(v.to_string().chars().count() < crate::agent_bridge::voice::MAX_RESULT_CHARS);
}

#[test]
fn zeitraum_und_baum_nur_freigegeben() {
    let a = Ablage::neu();
    let q = Quelle::neu(&a, &["notizbuch:nb1"]);
    let v = aufruf(
        &q,
        "notizbuch_zeitraum",
        json!({"von": "2026-11-03", "bis": "2026-11-05"}),
    )
    .unwrap();
    let ids: Vec<&str> = v["seiten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["seite_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["s1", "s2"], "neueste zuerst, ohne s3");
    let leer = aufruf(
        &q,
        "notizbuch_zeitraum",
        json!({"von": "2026-10-01", "bis": "2026-10-02"}),
    )
    .unwrap();
    assert!(leer["seiten"].as_array().unwrap().is_empty());
    let err = aufruf(
        &q,
        "notizbuch_zeitraum",
        json!({"von": "2026-11-05", "bis": "2026-11-01"}),
    )
    .unwrap_err();
    assert!(err.contains("vor"), "{err}");

    let baum = aufruf(&q, "notizbuch_baum", json!({})).unwrap();
    let nbs = baum["notizbuecher"].as_array().unwrap();
    assert_eq!(nbs.len(), 1);
    assert_eq!(nbs[0]["name"], json!("Arbeit"));
    assert_eq!(nbs[0]["abschnitte"].as_array().unwrap().len(), 2);
    assert!(!baum.to_string().contains("Geheim"));
}

#[test]
fn falsche_argumente_oeffnen_die_ablage_nicht() {
    let a = Ablage::neu();
    let q = Quelle::neu(&a, &["notizbuch:nb1"]);
    for (tool, args) in [
        ("notizbuch_suchen", json!({})),
        ("notizbuch_suchen", json!({"anfrage": "Haus", "grenze": 0})),
        (
            "notizbuch_suchen",
            json!({"anfrage": "Haus", "unbekannt": 1}),
        ),
        ("notizbuch_seite", json!({})),
        ("notizbuch_zeitraum", json!({"von": "morgen früh"})),
        ("notizbuch_baum", json!({"x": 1})),
    ] {
        assert!(aufruf(&q, tool, args.clone()).is_err(), "{tool} {args}");
    }
    assert_eq!(q.geoeffnet.get(), 0);
}

#[test]
fn ohne_notizbuch_auf_dem_geraet_eine_klare_meldung() {
    let a = Ablage::neu();
    let mut q = Quelle::neu(&a, &["notizbuch:nb1"]);
    q.pfad = a.pfad.with_file_name("fehlt.db");
    let err = aufruf(&q, "notizbuch_baum", json!({})).unwrap_err();
    assert_eq!(err, "Auf diesem Gerät gibt es noch kein Notizbuch.");
}

/// Codex lva#414 M5: Argumente und Ergebnisse eines Notizbuchaufrufs fließen weder ins Audit noch
/// ins Log. Weg wie im Gespräch: `voice::call` mit echtem Tor und echtem Audit.
#[test]
fn argumente_und_ergebnisse_nicht_in_audit_und_log() {
    use crate::agent_bridge::catalog::ToolRegistry;
    use crate::agent_bridge::voice::{call, Session, VoiceOptions};
    use crate::managers::integrations::audit::{self, AuditFilter};
    use crate::managers::integrations::test_support::Fx;

    crate::testlog::installieren();
    const GEHEIM_INHALT: &str = "NBXVERTRAULICHINHALT";
    const GEHEIM_FRAGE: &str = "NBXVERTRAULICHFRAGE";
    let a = Ablage::neu();
    a.seite(
        "s9",
        "a1",
        &format!("Titel {GEHEIM_INHALT}"),
        &format!("Text {GEHEIM_INHALT} und {GEHEIM_FRAGE}"),
        "2026-11-05T09:00:00Z",
        "2026-11-05T09:00:00Z",
    );
    let q = Quelle::neu(&a, &["notizbuch:nb1"]);
    let fx = Fx::new();
    let conn = fx.conn();
    let registry = ToolRegistry::new();
    let session = Session {
        registry: &registry,
        source: &q,
        conversation: None,
    };
    let opts = VoiceOptions {
        areas: Areas::from_settings(&["notebook".to_string()], false),
    };
    let jetzt_ms = jetzt();
    let s = call(
        &conn,
        &session,
        opts,
        "notizbuch_suchen",
        &json!({ "anfrage": GEHEIM_FRAGE }).to_string(),
        jetzt_ms,
    )
    .unwrap();
    assert!(s.contains(GEHEIM_INHALT), "das Ergebnis geht an das Modell");
    let s = call(
        &conn,
        &session,
        opts,
        "notizbuch_seite",
        &json!({ "seite_id": "s9" }).to_string(),
        jetzt_ms,
    )
    .unwrap();
    assert!(s.contains(GEHEIM_INHALT));
    // Fehlerpfad (verborgene Seite) ebenso.
    assert!(call(
        &conn,
        &session,
        opts,
        "notizbuch_seite",
        &json!({ "seite_id": "s3" }).to_string(),
        jetzt_ms,
    )
    .is_err());

    let rows = audit::list(&conn, &AuditFilter::default(), 100).unwrap();
    assert!(rows
        .iter()
        .any(|r| r.target.as_deref() == Some("notizbuch_suchen · voice_orchestrator")));
    for r in &rows {
        let alles = format!("{r:?}");
        assert!(
            !alles.contains("NBXVERTRAULICH") && !alles.contains("s9"),
            "Audit ohne Argumente und Inhalte: {alles}"
        );
    }
    // Auch keine Rohspalte der Audit-Tabelle (falls die Darstellung etwas auslässt).
    let mut st = conn.prepare("SELECT * FROM audit_log").unwrap();
    let spalten = st.column_count();
    let roh: Vec<String> = st
        .query_map([], |r| {
            Ok((0..spalten)
                .map(|i| {
                    format!(
                        "{:?}",
                        r.get_ref(i).map(|v| v.as_str().unwrap_or("")).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!roh.is_empty());
    assert!(roh.iter().all(|z| !z.contains("NBXVERTRAULICH")), "{roh:?}");
    // Log (alle Stufen): kein Inhalt, keine Anfrage.
    let zeilen = crate::testlog::zeilen();
    assert!(
        zeilen.iter().all(|l| !l.contains("NBXVERTRAULICH")),
        "Log ohne Inhalte: {zeilen:?}"
    );
}
