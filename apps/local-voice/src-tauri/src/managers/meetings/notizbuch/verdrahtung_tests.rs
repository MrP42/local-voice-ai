//! Tests der Verdrahtung (N4a Schritt 2): Migration `meeting_note_links`, Store-API, Dienst
//! `zuordnen` über eine synthetische Notizbuch-Ablage, Freigabe-Grenze (M1) und Verlustprüfung
//! mit beiden Notizquellen. Nur synthetische Daten (`example.*`).

use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::{params, Connection};
use rusqlite_migration::Migrations;
use serde_json::{json, Value};
use wai_notizbuch_zugang::{falte, Freigabe, Zugang};

use super::dienst::{zuordnen_mit, TOR_BESTANDEN};
use super::links::{AutoLink, LinkArt, NOTE_LINKS_MIGRATION};
use super::quelle::{
    bloecke_aus_seite, lesen, verlust_sichern, NoteSource, NotizbuchNotizen, MAX_SEITENTEXT,
    WEITERE_ID,
};
use super::zeit::Zeitachse;
use crate::managers::meetings::notes::assemble::{assemble, RawEntry};
use crate::managers::meetings::notes::model::{
    EnhancedSection, NoteBlock, NoteBlockKind, Origin, SectionKind, TemplateSection, TemplateSpec,
};
use crate::managers::meetings::store::{MeetingSource, MeetingStore, StoredSegment, MIGRATIONS};

/// Beginn der synthetischen Besprechung: 2026-11-05T09:00:00Z.
const T0: i64 = 1_793_955_600;
const DAUER_MS: i64 = 60 * 60_000;

fn iso(min: i64) -> String {
    chrono::DateTime::from_timestamp(T0 + min * 60, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

// ---------------------------------------------------------------------------
// Synthetische Notizbuch-Ablage (Schema des Notizbuch-Hosts, Migrationen 1 bis 3)
// ---------------------------------------------------------------------------

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
            json!({"id": "a3", "name": "Geheim", "notizbuchId": "nb2"}),
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

    /// Seite mit Blöcken `(bid, text, minute_nach_beginn)`; optional eine Aufgabe (erledigt).
    fn seite(
        &self,
        id: &str,
        abschnitt: &str,
        titel: &str,
        bloecke: &[(&str, &str, i64)],
        aufgabe: Option<(&str, &str, bool)>,
    ) {
        let mut inhalt: Vec<Value> = bloecke
            .iter()
            .map(|(bid, text, min)| {
                json!({"type": "paragraph", "attrs": {"bid": bid, "zeit": iso(*min)},
                       "content": [{"type": "text", "text": text}]})
            })
            .collect();
        if let Some((bid, text, erledigt)) = aufgabe {
            inhalt.push(json!({"type": "taskList", "content": [
                {"type": "taskItem", "attrs": {"checked": erledigt, "bid": bid, "zeit": iso(20)},
                 "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]}]}));
        }
        let text: String = bloecke
            .iter()
            .map(|(_, t, _)| *t)
            .collect::<Vec<_>>()
            .join("\n");
        let erstellt = iso(bloecke.iter().map(|b| b.2).min().unwrap_or(0));
        let geaendert = iso(bloecke.iter().map(|b| b.2).max().unwrap_or(0).max(20));
        self.satz(
            "seiten",
            json!({"id": id, "abschnittId": abschnitt, "titel": titel, "text": text,
                   "inhalt": {"type": "doc", "content": inhalt},
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
                params![nr, id, falte(titel), falte(&text)],
            )
            .unwrap();
    }

    fn zugang(&self) -> Zugang {
        Zugang::oeffnen(&self.pfad).unwrap()
    }
}

/// Seiten: s1 (freigegeben, passt), s2 (freigegeben, Tage vorher), s3 (verborgen, passt genauso).
fn ablage_mit_seiten() -> Ablage {
    let a = Ablage::neu();
    let passend: [(&str, &str, i64); 3] = [
        ("b1", "Budget Migration Frankfurt 120000 Euro", 3),
        ("b2", "Rollout Server Dezember abschalten", 12),
        ("b3", "Ansprechpartner kontakt@example.org", 25),
    ];
    a.seite(
        "s1",
        "a1",
        "Jour fixe Migration",
        &passend,
        Some(("t1", "Angebot prüfen", true)),
    );
    a.seite(
        "s2",
        "a1",
        "Alte Notiz",
        &[("c1", "Urlaubsplanung", -3 * 24 * 60)],
        None,
    );
    a.seite("s3", "a3", "Geheim Migration", &passend, None);
    a
}

fn nur_arbeit() -> Freigabe {
    Freigabe::default().notizbuch("nb1")
}

// ---------------------------------------------------------------------------
// Besprechungs-Store
// ---------------------------------------------------------------------------

fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    (dir, s)
}

fn segment(i: u32, start_min: u64, ende_min: u64, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.into(),
        start_ms: start_min * 60_000,
        end_ms: ende_min * 60_000,
        channel: 0,
        speaker_index: None,
        words: None,
    }
}

/// Fertige Besprechung mit Beginn T0, einer Stunde Dauer und Transkript.
fn besprechung(s: &MeetingStore) -> String {
    let m = s
        .create_meeting("Jour fixe", MeetingSource::Live, Some(1))
        .unwrap();
    s.get_connection()
        .unwrap()
        .execute(
            "UPDATE meetings SET started_at = ?1, duration_ms = ?2, status = 'ready' WHERE id = ?3",
            params![T0, DAUER_MS, m.id],
        )
        .unwrap();
    let segs = vec![
        segment(
            0,
            0,
            10,
            "Wir sprechen über das Budget der Migration in Frankfurt.",
        ),
        segment(
            1,
            10,
            30,
            "Der Server wird im Dezember abgeschaltet, dann Rollout.",
        ),
    ];
    let rev = s.transcript_snapshot(&m.id).unwrap().revision;
    s.replace_segments(&m.id, &segs, "test", "segment@1", rev)
        .unwrap();
    m.id
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Migration und Store-API
// ---------------------------------------------------------------------------

#[test]
fn migration_ist_angehaengt_idempotent_und_laesst_altbestand_stehen() {
    let index = MIGRATIONS.len() - 1;
    assert_eq!(index, 16, "N4a hängt als Index 16 hinter den Teilen");
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("meetings.db");
    {
        let mut conn = Connection::open(&pfad).unwrap();
        Migrations::new(MIGRATIONS[..index].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        conn.execute(
            "INSERT INTO meetings (id, title, status, source, created_at, updated_at)
             VALUES ('M1', 'Alt', 'ready', 'live', 1, 1)",
            [],
        )
        .unwrap();
        assert_eq!(user_version(&conn), index as i64);
    }
    let store = MeetingStore::open_at(&pfad).unwrap();
    let conn = store.get_connection().unwrap();
    assert_eq!(user_version(&conn), MIGRATIONS.len() as i64);
    let titel: String = conn
        .query_row("SELECT title FROM meetings WHERE id = 'M1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(titel, "Alt");
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM meeting_note_links", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    // Idempotent: der Text läuft ein zweites Mal ohne Fehler.
    conn.execute_batch(NOTE_LINKS_MIGRATION).unwrap();
    // Unbekannte Art wird von der Tabelle abgewiesen.
    assert!(conn
        .execute(
            "INSERT INTO meeting_note_links (meeting_id, seite_id, art, aktualisiert)
             VALUES ('M1', 's1', 'irgendwas', 1)",
            [],
        )
        .is_err());
    drop(conn);
    // Zweiter Start ändert nichts.
    let store = MeetingStore::open_at(&pfad).unwrap();
    assert_eq!(
        user_version(&store.get_connection().unwrap()),
        MIGRATIONS.len() as i64
    );
}

fn auto(seite: &str, art: LinkArt, punkte: u32) -> AutoLink {
    AutoLink {
        seite_id: seite.into(),
        art,
        punkte,
        gruende_json: "[]".into(),
    }
}

#[test]
fn geloest_und_bestaetigt_ueberlebt_jeden_automatiklauf() {
    let (_d, s) = tmp_store();
    let m = besprechung(&s);
    s.note_links_automatisch_ersetzen(
        &m,
        "st1",
        &[
            auto("s1", LinkArt::Vorschlag, 40),
            auto("s2", LinkArt::Vorschlag, 50),
        ],
    )
    .unwrap();
    s.note_link_loesen(&m, "s1").unwrap();
    s.note_link_setzen(&m, "s2", LinkArt::Bestaetigt, "st2")
        .unwrap();
    // Grund und Punkte der Bewertung bleiben bei der Bestätigung erhalten.
    let s2 = s.note_link(&m, "s2").unwrap().unwrap();
    assert_eq!(
        (s2.art, s2.punkte, s2.stand.as_str()),
        (LinkArt::Bestaetigt, 50, "st2")
    );

    // Automatik will beide wieder (auch als `auto`) und eine neue Seite.
    s.note_links_automatisch_ersetzen(
        &m,
        "st3",
        &[
            auto("s1", LinkArt::Auto, 90),
            auto("s2", LinkArt::Vorschlag, 40),
            auto("s4", LinkArt::Vorschlag, 36),
        ],
    )
    .unwrap();
    assert_eq!(s.note_link_art(&m, "s1").unwrap(), Some(LinkArt::Geloest));
    assert_eq!(
        s.note_link_art(&m, "s2").unwrap(),
        Some(LinkArt::Bestaetigt)
    );
    assert_eq!(s.note_link_art(&m, "s4").unwrap(), Some(LinkArt::Vorschlag));

    // Leerer Lauf entfernt nur Automatisches.
    s.note_links_automatisch_ersetzen(&m, "", &[]).unwrap();
    let arten: Vec<(String, LinkArt)> = s
        .note_links(&m)
        .unwrap()
        .into_iter()
        .map(|l| (l.seite_id, l.art))
        .collect();
    assert_eq!(
        arten,
        vec![
            ("s1".to_string(), LinkArt::Geloest),
            ("s2".to_string(), LinkArt::Bestaetigt)
        ]
    );

    // Falsche Wege werden abgewiesen.
    assert!(s
        .note_link_setzen(&m, "s1", LinkArt::Vorschlag, "")
        .is_err());
    assert!(s
        .note_links_automatisch_ersetzen(&m, "", &[auto("s1", LinkArt::Geloest, 0)])
        .is_err());
    assert!(s
        .note_link_setzen(&m, "s 1;", LinkArt::Bestaetigt, "")
        .is_err());
    assert!(s
        .note_link_setzen("gibt-es-nicht", "s1", LinkArt::Bestaetigt, "")
        .is_err());
}

// ---------------------------------------------------------------------------
// Dienst `zuordnen` und Freigabe-Grenze
// ---------------------------------------------------------------------------

#[test]
fn zuordnen_liefert_nur_vorschlaege_aus_freigegebenen_abschnitten() {
    assert!(!TOR_BESTANDEN, "Automatik erst nach dem Eval-Tor");
    let a = ablage_mit_seiten();
    let z = a.zugang();
    let (_d, s) = tmp_store();
    let m = besprechung(&s);

    let f = nur_arbeit();
    let ergebnis = zuordnen_mit(&s, &m, Some((&z, &f))).unwrap();
    let ids: Vec<&str> = ergebnis.iter().map(|r| r.seite_id.as_str()).collect();
    assert_eq!(ids, vec!["s1"], "{ergebnis:?}");
    assert_eq!(
        ergebnis[0].art,
        LinkArt::Vorschlag,
        "ohne Tor nie automatisch"
    );
    let link = s.note_link(&m, "s1").unwrap().unwrap();
    assert_eq!(link.art, LinkArt::Vorschlag);
    assert!(!link.stand.is_empty());
    assert!(
        link.gruende_json.contains("\"zeit\""),
        "{}",
        link.gruende_json
    );
    // Die verborgene Seite (gleicher Inhalt) steht nirgends.
    assert!(s.note_links(&m).unwrap().iter().all(|l| l.seite_id != "s3"));

    // Freigabe mit beiden Notizbüchern: s3 wird sichtbar (Gegenprobe, dass s3 sonst passen würde).
    let beide = Freigabe::default().notizbuch("nb1").notizbuch("nb2");
    let mit = zuordnen_mit(&s, &m, Some((&z, &beide))).unwrap();
    let ids: HashSet<&str> = mit.iter().map(|r| r.seite_id.as_str()).collect();
    assert!(ids.contains("s3") && ids.contains("s1"), "{mit:?}");

    // Widerruf: zurück auf nb1 → s3 verschwindet wieder vollständig.
    zuordnen_mit(&s, &m, Some((&z, &f))).unwrap();
    assert!(s.note_links(&m).unwrap().iter().all(|l| l.seite_id != "s3"));

    // Gelöst bleibt gelöst.
    s.note_link_loesen(&m, "s1").unwrap();
    assert!(zuordnen_mit(&s, &m, Some((&z, &f))).unwrap().is_empty());
    assert_eq!(s.note_link_art(&m, "s1").unwrap(), Some(LinkArt::Geloest));
}

#[test]
fn verborgene_abschnitte_aendern_weder_ergebnis_noch_speicher() {
    // Zwei Ablagen, die sich nur im verborgenen Abschnitt unterscheiden (M1).
    let ohne = Ablage::neu();
    let mit = ablage_mit_seiten();
    let passend: [(&str, &str, i64); 3] = [
        ("b1", "Budget Migration Frankfurt 120000 Euro", 3),
        ("b2", "Rollout Server Dezember abschalten", 12),
        ("b3", "Ansprechpartner kontakt@example.org", 25),
    ];
    ohne.seite(
        "s1",
        "a1",
        "Jour fixe Migration",
        &passend,
        Some(("t1", "Angebot prüfen", true)),
    );
    ohne.seite(
        "s2",
        "a1",
        "Alte Notiz",
        &[("c1", "Urlaubsplanung", -3 * 24 * 60)],
        None,
    );
    for i in 0..30 {
        mit.seite(&format!("v{i}"), "a3", "Geheim", &passend, None);
    }
    let f = nur_arbeit();
    let ergebnis = |a: &Ablage| {
        let (_d, s) = tmp_store();
        let m = besprechung(&s);
        let r = zuordnen_mit(&s, &m, Some((&a.zugang(), &f))).unwrap();
        let gespeichert: Vec<(String, LinkArt, u32, String)> = s
            .note_links(&m)
            .unwrap()
            .into_iter()
            .map(|l| (l.seite_id, l.art, l.punkte, l.gruende_json))
            .collect();
        (r, gespeichert)
    };
    assert_eq!(ergebnis(&ohne), ergebnis(&mit));
}

#[test]
fn bereich_aus_ergibt_nichts_und_raeumt_vorschlaege_ab() {
    let a = ablage_mit_seiten();
    let z = a.zugang();
    let (_d, s) = tmp_store();
    let m = besprechung(&s);
    zuordnen_mit(&s, &m, Some((&z, &nur_arbeit()))).unwrap();
    s.note_link_setzen(&m, "s2", LinkArt::Bestaetigt, "")
        .unwrap();
    assert!(zuordnen_mit(&s, &m, None).unwrap().is_empty());
    assert!(zuordnen_mit(&s, &m, Some((&z, &Freigabe::default())))
        .unwrap()
        .is_empty());
    let arten: Vec<LinkArt> = s
        .note_links(&m)
        .unwrap()
        .into_iter()
        .map(|l| l.art)
        .collect();
    assert_eq!(
        arten,
        vec![LinkArt::Bestaetigt],
        "nur die Nutzerzeile bleibt"
    );
}

// ---------------------------------------------------------------------------
// Zweite Notizquelle
// ---------------------------------------------------------------------------

fn achse() -> Zeitachse {
    Zeitachse::neu(T0, DAUER_MS)
}

#[test]
fn zugeordnete_seiten_vollstaendig_lesen_verborgene_still_auslassen() {
    let a = ablage_mit_seiten();
    let z = a.zugang();
    let (_d, s) = tmp_store();
    let m = besprechung(&s);
    s.note_link_setzen(&m, "s1", LinkArt::Bestaetigt, "")
        .unwrap();
    s.note_link_setzen(&m, "s3", LinkArt::Bestaetigt, "")
        .unwrap(); // verborgen
    s.note_link_setzen(&m, "s2", LinkArt::Geloest, "").unwrap(); // gelöst: nie Quelle
    let links = s.note_links(&m).unwrap();
    let nb = lesen(&z, &nur_arbeit(), &links, &achse());
    assert_eq!(nb.seiten.len(), 1);
    let seite = &nb.seiten[0];
    assert_eq!(seite.seite_id, "s1");
    assert!(!seite.unvollstaendig && !seite.stand.is_empty());
    let ids: Vec<&str> = seite.bloecke.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, vec!["nb:s1#b1", "nb:s1#b2", "nb:s1#b3", "nb:s1#t1"]);
    assert_eq!(
        seite.bloecke[0].at_ms,
        Some(3 * 60_000),
        "Güte genau → Minute"
    );
    assert_eq!(seite.bloecke[3].kind, NoteBlockKind::Todo);
    assert!(seite.bloecke[3].checked, "erledigt bleibt erledigt");
    assert_eq!(
        nb.quelle("nb:s1#b2"),
        NoteSource::Notizbuch {
            seite_id: "s1".into(),
            block_id: "b2".into(),
            stand: seite.stand.clone()
        }
    );
    assert_eq!(
        nb.quelle("01J0"),
        NoteSource::Notizblock {
            block_id: "01J0".into()
        }
    );
    // Kein Inhalt in den Metadaten.
    let meta = nb.metadaten(true, 0).to_string();
    assert!(
        !meta.contains("Budget") && !meta.contains("example.org"),
        "{meta}"
    );
}

#[test]
fn ueber_ein_megabyte_gilt_als_unvollstaendig() {
    let a = Ablage::neu();
    let gross = "x".repeat(MAX_SEITENTEXT / 2 + 10);
    a.seite(
        "s9",
        "a1",
        "Lang",
        &[("g1", &gross, 1), ("g2", &gross, 2), ("g3", "Rest", 3)],
        None,
    );
    let seite = a.zugang().seite("s9", &nur_arbeit()).unwrap().unwrap();
    let (bloecke, gekappt) = bloecke_aus_seite(&seite, &achse());
    assert!(gekappt);
    assert_eq!(bloecke.len(), 1);
}

// ---------------------------------------------------------------------------
// Verlustprüfung mit beiden Quellen
// ---------------------------------------------------------------------------

fn spec() -> TemplateSpec {
    let sec = |id: &str, kind| TemplateSection {
        id: id.into(),
        title: id.to_uppercase(),
        instruction: String::new(),
        kind,
    };
    TemplateSpec {
        version: 1,
        context: String::new(),
        sections: vec![
            sec("summary", SectionKind::Text),
            sec("tasks", SectionKind::Tasks),
        ],
    }
}

fn notizblock() -> Vec<NoteBlock> {
    let b = |id: &str, text: &str| NoteBlock {
        id: id.into(),
        kind: NoteBlockKind::Bullet,
        text: text.into(),
        at_ms: None,
        checked: false,
    };
    vec![b("N-a", "Eigene Notiz eins"), b("N-b", "Eigene Notiz zwei")]
}

fn notizbuch_notizen() -> NotizbuchNotizen {
    let a = ablage_mit_seiten();
    let (_d, s) = tmp_store();
    let m = besprechung(&s);
    s.note_link_setzen(&m, "s1", LinkArt::Bestaetigt, "")
        .unwrap();
    let mut nb = lesen(
        &a.zugang(),
        &nur_arbeit(),
        &s.note_links(&m).unwrap(),
        &achse(),
    );
    // Eine zweite, nicht vollständig lesbare Seite.
    nb.seiten.push(super::quelle::GeleseneSeite {
        seite_id: "s7".into(),
        titel: "Sehr lange Seite".into(),
        stand: String::new(),
        unvollstaendig: true,
        bloecke: vec![],
    });
    nb
}

fn note_ids(sections: &[EnhancedSection]) -> Vec<String> {
    sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .filter(|e| e.origin == Origin::User)
        .filter_map(|e| e.note_id.clone())
        .collect()
}

fn raw_ref(r: &str) -> RawEntry {
    RawEntry {
        r#ref: Some(r.into()),
        ..RawEntry::default()
    }
}

#[test]
fn jeder_block_beider_quellen_steht_genau_einmal_im_ergebnis() {
    let nb = notizbuch_notizen();
    let eigene = notizblock();
    let alle: Vec<NoteBlock> = eigene.iter().cloned().chain(nb.note_blocks()).collect();
    // Modell ordnet nur N1 (Notizblock) und N3 (erster Notizbuch-Block) zu.
    let mut raw = std::collections::BTreeMap::new();
    raw.insert("summary".to_string(), vec![raw_ref("N1"), raw_ref("N3")]);
    let (mut sections, mut stats) = assemble(raw, &spec(), &alle, &[], &[]);
    let bericht = verlust_sichern(&mut sections, &mut stats, &eigene, &nb);

    let mut ids = note_ids(&sections);
    let anzahl = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), anzahl, "kein Block doppelt");
    let erwartet: HashSet<String> = alle.iter().map(|b| b.id.clone()).collect();
    assert_eq!(
        ids.into_iter().collect::<HashSet<_>>(),
        erwartet,
        "kein Block fehlt"
    );

    let weitere = sections.iter().find(|s| s.id == WEITERE_ID).unwrap();
    assert_eq!(weitere.title, "Weitere Notizen");
    let weitere_ids: Vec<&str> = weitere
        .entries
        .iter()
        .filter_map(|e| e.note_id.as_deref())
        .collect();
    assert_eq!(weitere_ids, vec!["nb:s1#b2", "nb:s1#b3"]);
    // Aufgabe aus dem Notizbuch bleibt im Aufgaben-Abschnitt.
    let tasks = sections.iter().find(|s| s.id == "tasks").unwrap();
    assert!(tasks
        .entries
        .iter()
        .any(|e| e.note_id.as_deref() == Some("nb:s1#t1")));
    // Nutzertext byte-genau.
    let b2 = weitere
        .entries
        .iter()
        .find(|e| e.note_id.as_deref() == Some("nb:s1#b2"))
        .unwrap();
    assert_eq!(b2.text, "Rollout Server Dezember abschalten");
    // Hinweis auf die unvollständige Seite.
    assert!(weitere
        .entries
        .iter()
        .any(|e| e.text == "Notizen unvollständig: Sehr lange Seite" && e.origin == Origin::Ai));
    assert_eq!(bericht.unvollstaendig, vec!["s7".to_string()]);
    // IDs fortlaufend.
    let eids: Vec<String> = sections
        .iter()
        .flat_map(|s| s.entries.iter().map(|e| e.id.clone()))
        .collect();
    let soll: Vec<String> = (1..=eids.len()).map(|i| format!("E{i}")).collect();
    assert_eq!(eids, soll);
}

#[test]
fn ohne_notizbuch_im_prompt_landen_alle_notizbuch_bloecke_in_weitere_notizen() {
    // Entfernter Anbieter: Notizbuchinhalt geht nicht an das Modell, aber verloren geht nichts.
    let nb = notizbuch_notizen();
    let eigene = notizblock();
    let mut raw = std::collections::BTreeMap::new();
    raw.insert("summary".to_string(), vec![raw_ref("N1"), raw_ref("N2")]);
    let (mut sections, mut stats) = assemble(raw, &spec(), &eigene, &[], &[]);
    let bericht = verlust_sichern(&mut sections, &mut stats, &eigene, &nb);
    assert_eq!(
        bericht.ergaenzt,
        vec!["nb:s1#b1", "nb:s1#b2", "nb:s1#b3", "nb:s1#t1"]
    );
    let ids: HashSet<String> = note_ids(&sections).into_iter().collect();
    for b in nb.note_blocks().iter().chain(eigene.iter()) {
        assert!(ids.contains(&b.id), "{} fehlt", b.id);
    }
    let tasks = sections.iter().find(|s| s.id == "tasks").unwrap();
    assert!(tasks
        .entries
        .iter()
        .any(|e| e.note_id.as_deref() == Some("nb:s1#t1")));
}

#[test]
fn ohne_notizbuch_bleibt_das_ergebnis_wie_bisher() {
    let eigene = notizblock();
    let mut raw = std::collections::BTreeMap::new();
    raw.insert("summary".to_string(), vec![raw_ref("N1")]);
    let (vorher, stats_vorher) = assemble(raw.clone(), &spec(), &eigene, &[], &[]);
    let (mut nachher, mut stats) = assemble(raw, &spec(), &eigene, &[], &[]);
    let bericht = verlust_sichern(
        &mut nachher,
        &mut stats,
        &eigene,
        &NotizbuchNotizen::default(),
    );
    assert_eq!(vorher, nachher);
    assert_eq!(stats_vorher, stats);
    assert!(bericht.ergaenzt.is_empty() && bericht.unvollstaendig.is_empty());
}
