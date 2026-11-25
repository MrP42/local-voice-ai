//! Tests zu P5 (lva#736): Protokollvorlage aus der Agenda, "Ziel erreicht", Freigabe und Entwurf an
//! die Teilnehmenden. Nur synthetische Daten (`example.*`); die Datenbank liegt im Tempverzeichnis.

use super::protokoll::*;
use super::*;
use crate::managers::meetings::minutes::DOC_KIND;
use crate::managers::meetings::notes::templates::validate_spec;
use crate::managers::meetings::store::{MeetingSource, MeetingStatus};
use besprechungsplanung::protokoll::{self as pk, Erreicht};

fn fix() -> (tempfile::TempDir, MeetingStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
    (dir, store)
}

/// Besprechung mit Planung (Termin `uid`, Teilnehmende `ANNA`/`BERT`, alle zugestimmt).
fn geplant(store: &MeetingStore, uid: &str) -> (besprechungsplanung::model::Plan, String) {
    let m = store
        .create_meeting("Abstimmung", MeetingSource::Empty, Some(1))
        .unwrap();
    store.set_status(&m.id, MeetingStatus::Ready).unwrap();
    let (plan, _) = testhilfe::plan_fuer_besprechung(store, &m.id, uid, true);
    (plan, m.id)
}

fn protokoll_text(erreicht: &str) -> String {
    format!(
        "# Protokoll: Abstimmung\n\n**Datum:** 2026-11-12\n**Ziel (Entscheiden):** Wir entscheiden\n\
         **Ziel erreicht:** {erreicht}\n**Klassifizierung:** intern\n\n## 1. Optionen\n\n\
         Option A gewinnt. [1]\n\n## Sprecher & Redeanteile\n\n| Sprecher | Anteil |\n|---|---|\n\
         | Anna | 60 % |\n"
    )
}

fn dokument_anlegen(store: &MeetingStore, meeting_id: &str, text: &str) -> String {
    store
        .insert_document(meeting_id, DOC_KIND, "markdown@1", text, None, None)
        .unwrap()
}

fn tabellen_stand(store: &MeetingStore) -> Vec<(String, i64)> {
    let conn = verbindung(store).unwrap();
    let mut st = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap();
    let namen = st
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    namen
        .into_iter()
        .map(|n| {
            let n_zeilen = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{n}\""), [], |r| r.get(0))
                .unwrap();
            (n, n_zeilen)
        })
        .collect()
}

#[test]
fn migration_legt_die_tabelle_an_und_ist_wiederholbar() {
    let (_dir, store) = fix();
    let conn = verbindung(&store).unwrap();
    conn.execute_batch(PROTOKOLL_MIGRATION).unwrap();
    conn.execute_batch(PROTOKOLL_MIGRATION).unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM meeting_plan_protocols", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn vorlage_aus_der_agenda_passt_ins_vorlagensystem() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-vorlage");
    let erg = vorlage_anlegen(&store, &plan.id, 10).unwrap();
    assert_eq!(erg.meeting_id, meeting_id);
    assert_eq!(erg.abschnitte, 4); // 1 Agendapunkt + Ergebnisse, Entscheidungen, Aufgaben
    let info = store.get_template_info(&erg.template_id).unwrap().unwrap();
    assert!(!info.builtin);
    assert!(info.title.starts_with("Agenda: "));
    validate_spec(&info.spec).unwrap();
    assert_eq!(info.spec.sections[0].title, "1. Optionen");
    // Der Besprechung zugeordnet: dort wirkt sie als Nutzerwahl des vorhandenen Weges.
    assert_eq!(
        store.meeting_template_id(&meeting_id).unwrap().as_deref(),
        Some(erg.template_id.as_str())
    );
    assert_eq!(
        stand(&store, &plan.id).unwrap().vorlage_id,
        Some(erg.template_id.clone())
    );
}

#[test]
fn vorlage_nochmal_aktualisiert_dieselbe_vorlage() {
    let (_dir, store) = fix();
    let (plan, _) = geplant(&store, "uid-vorlage-2");
    let erste = vorlage_anlegen(&store, &plan.id, 10).unwrap();
    let vorher = tabellen_stand(&store);
    let zweite = vorlage_anlegen(&store, &plan.id, 11).unwrap();
    assert_eq!(erste.template_id, zweite.template_id);
    assert_eq!(
        tabellen_stand(&store),
        vorher,
        "keine zweite Vorlage, keine zweite Zeile"
    );
}

#[test]
fn vorlage_ohne_besprechung_meldet_den_grund_und_legt_nichts_an() {
    let (_dir, store) = fix();
    let (plan, _) = testhilfe::plan_mit_termin(&store, "uid-ohne", true);
    let vorher = tabellen_stand(&store);
    assert_eq!(
        vorlage_anlegen(&store, &plan.id, 10).unwrap_err(),
        "besprechung_fehlt"
    );
    assert_eq!(
        vorlage_anlegen(&store, "gibt-es-nicht", 10).unwrap_err(),
        "plan_nicht_gefunden"
    );
    assert_eq!(tabellen_stand(&store), vorher);
}

#[test]
fn kopf_nur_fuer_besprechungen_mit_planung() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-kopf");
    let kopf = kopf_fuer_besprechung(&store, &meeting_id, None)
        .unwrap()
        .unwrap();
    assert!(kopf.contains("**Ziel (Entscheiden):**"));
    assert!(kopf.contains("**Ziel erreicht:** noch nicht bewertet"));
    assert!(kopf.contains("**Klassifizierung:** intern"));
    erreicht_setzen(&store, &plan.id, Some(Erreicht::Ja), 20).unwrap();
    let kopf = kopf_fuer_besprechung(&store, &meeting_id, None)
        .unwrap()
        .unwrap();
    assert!(kopf.contains("**Ziel erreicht:** ja\n"));

    let ohne = store
        .create_meeting("Ohne Planung", MeetingSource::Empty, Some(1))
        .unwrap();
    assert_eq!(kopf_fuer_besprechung(&store, &ohne.id, None).unwrap(), None);
}

#[test]
fn erreicht_wird_von_hand_gesetzt_und_ins_protokoll_nachgeschrieben() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-erreicht");
    let dok = dokument_anlegen(&store, &meeting_id, &protokoll_text("noch nicht bewertet"));
    let s = erreicht_setzen(&store, &plan.id, Some(Erreicht::Teilweise), 20).unwrap();
    assert_eq!(s.erreicht, Some(Erreicht::Teilweise));
    assert!(!s.kopf_fehlt);
    let text = store.get_document(&dok).unwrap().unwrap().body;
    assert!(text.contains("**Ziel erreicht:** teilweise\n"));
    assert!(!text.contains("noch nicht bewertet"));
    // Zuruecknehmen.
    let s = erreicht_setzen(&store, &plan.id, None, 21).unwrap();
    assert_eq!(s.erreicht, None);
    assert!(store
        .get_document(&dok)
        .unwrap()
        .unwrap()
        .body
        .contains("noch nicht bewertet"));
    assert_eq!(
        erreicht_setzen(&store, "gibt-es-nicht", None, 22).unwrap_err(),
        "plan_nicht_gefunden"
    );
}

#[test]
fn protokoll_ohne_kopfzeile_wird_gemeldet_nicht_veraendert() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-altes-protokoll");
    let alt = "# Protokoll: Alt\n\n## A\n\ntext\n";
    let dok = dokument_anlegen(&store, &meeting_id, alt);
    let s = erreicht_setzen(&store, &plan.id, Some(Erreicht::Ja), 20).unwrap();
    assert!(s.kopf_fehlt);
    assert_eq!(s.erreicht, Some(Erreicht::Ja));
    assert_eq!(store.get_document(&dok).unwrap().unwrap().body, alt);
}

#[test]
fn freigabe_haelt_die_fassung_fest_und_setzt_den_status() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-freigabe");
    assert_eq!(
        freigeben(&store, &plan.id, 30).unwrap_err(),
        "protokoll_fehlt"
    );
    let dok = dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    let s = freigeben(&store, &plan.id, 31).unwrap();
    assert!(s.freigegeben);
    assert_eq!(s.freigegeben_am, Some(31));
    assert!(!s.geaendert_seit_freigabe);
    assert_eq!(s.dokument_id.as_deref(), Some(dok.as_str()));
    assert_eq!(
        plan_laden(&store, &plan.id).unwrap().unwrap().status,
        besprechungsplanung::model::PlanStatus::Protokolliert
    );
    // Wiederholt: derselbe Stand.
    assert!(freigeben(&store, &plan.id, 32).unwrap().freigegeben);
}

#[test]
fn geaenderter_text_oder_neue_fassung_ist_nicht_mehr_freigegeben() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-aenderung");
    let dok = dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    assert!(freigeben(&store, &plan.id, 31).unwrap().freigegeben);

    // Von Hand bearbeitet.
    let d = store.get_document(&dok).unwrap().unwrap();
    store
        .update_markdown_document(&dok, &format!("{}\nNachtrag\n", d.body), d.updated_at)
        .unwrap();
    let s = stand(&store, &plan.id).unwrap();
    assert!(!s.freigegeben && s.geaendert_seit_freigabe);
    assert_eq!(
        entwurf(&store, &plan.id, &[]).unwrap_err(),
        "protokoll_nicht_freigegeben"
    );
    assert!(freigeben(&store, &plan.id, 40).unwrap().freigegeben);

    // Neu erzeugt (neue Version): die freigegebene gilt nicht fuer die neue.
    dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    let s = stand(&store, &plan.id).unwrap();
    assert!(!s.freigegeben && s.geaendert_seit_freigabe);
    assert_eq!(s.dokument_version, Some(2));
}

#[test]
fn ziel_erreicht_nach_der_freigabe_macht_eine_neue_freigabe_noetig() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-erreicht-nach");
    dokument_anlegen(&store, &meeting_id, &protokoll_text("noch nicht bewertet"));
    assert!(freigeben(&store, &plan.id, 31).unwrap().freigegeben);
    let s = erreicht_setzen(&store, &plan.id, Some(Erreicht::Nein), 32).unwrap();
    assert!(!s.freigegeben && s.geaendert_seit_freigabe);
}

#[test]
fn entwurf_nur_aus_freigegebenem_protokoll() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-entwurf-sperre");
    assert_eq!(
        entwurf(&store, &plan.id, &[]).unwrap_err(),
        "protokoll_nicht_freigegeben"
    );
    dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    assert_eq!(
        entwurf(&store, &plan.id, &[]).unwrap_err(),
        "protokoll_nicht_freigegeben"
    );
}

#[test]
fn entwurf_hat_empfaenger_betreff_klassifizierung_und_kein_transkript() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-entwurf");
    // Ein Transkript, das nie im Entwurf auftauchen darf.
    store
        .append_delta(
            &meeting_id,
            &crate::managers::meetings::store::TranscriptDelta {
                new_segments: vec![crate::managers::meetings::store::StoredSegment {
                    segment_index: 0,
                    text: "GEHEIMES-TRANSKRIPT-4711".into(),
                    start_ms: 0,
                    end_ms: 1000,
                    channel: 0,
                    speaker_index: None,
                    words: None,
                }],
            },
        )
        .unwrap();
    dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    freigeben(&store, &plan.id, 31).unwrap();

    let e = entwurf(&store, &plan.id, &[testhilfe::BERT.to_uppercase()]).unwrap();
    assert!(e.termin_gefunden);
    assert_eq!(e.ohne_adresse, 0);
    assert_eq!(
        e.entwurf.to,
        [testhilfe::ANNA.to_string()],
        "eigene Adresse fehlt, klein geschrieben"
    );
    assert!(e
        .entwurf
        .subject
        .starts_with("Protokoll: Abstimmung Posteingang ("));
    assert!(e.entwurf.subject.ends_with(')'));
    let body = &e.entwurf.body_text;
    // Die Zeile steht genau einmal da (der Kopf des Protokolls traegt sie schon), wie in der Einladung.
    assert!(
        body.lines().any(|l| l == "Klassifizierung: intern"),
        "war: {body}"
    );
    assert_eq!(body.matches("Klassifizierung: intern").count(), 1);
    assert!(body.contains("Option A gewinnt."));
    assert!(!body.contains("GEHEIMES-TRANSKRIPT-4711"));
    assert!(!body.contains("Redeanteile") && !body.contains("60 %"));
    assert!(!body.contains("[1]"), "Zitatmarken gehen nicht in die Mail");
    assert!(!body.contains('\r'));
}

#[test]
fn entwurf_ohne_eigene_adressen_enthaelt_alle_teilnehmenden() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-entwurf-alle");
    dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    freigeben(&store, &plan.id, 31).unwrap();
    let e = entwurf(&store, &plan.id, &[]).unwrap();
    assert_eq!(
        e.entwurf.to,
        [testhilfe::ANNA.to_string(), testhilfe::BERT.to_string()]
    );
}

#[test]
fn entwurf_aendert_nichts_in_der_datenbank() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-entwurf-lesend");
    dokument_anlegen(&store, &meeting_id, &protokoll_text("ja"));
    freigeben(&store, &plan.id, 31).unwrap();
    let vorher = tabellen_stand(&store);
    entwurf(&store, &plan.id, &[]).unwrap();
    assert_eq!(tabellen_stand(&store), vorher);
    // Versendet wird nie automatisch: der Status bleibt "protokolliert".
    assert_eq!(
        plan_laden(&store, &plan.id).unwrap().unwrap().status,
        besprechungsplanung::model::PlanStatus::Protokolliert
    );
}

#[test]
fn text_hash_der_freigabe_ist_der_des_dokuments() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-hash");
    let text = protokoll_text("ja");
    dokument_anlegen(&store, &meeting_id, &text);
    freigeben(&store, &plan.id, 31).unwrap();
    let conn = verbindung(&store).unwrap();
    let hash: String = conn
        .query_row(
            "SELECT approved_hash FROM meeting_plan_protocols WHERE plan_id = ?1",
            [&plan.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash, pk::text_hash(&text));
}

#[test]
fn entwurf_ergaenzt_die_klassifizierung_bei_protokoll_ohne_kopf() {
    let (_dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-ohne-kopf");
    dokument_anlegen(
        &store,
        &meeting_id,
        "# Protokoll: Alt\n\n## A\n\nBeschluss steht.\n",
    );
    freigeben(&store, &plan.id, 31).unwrap();
    let e = entwurf(&store, &plan.id, &[]).unwrap();
    assert!(e.entwurf.body_text.starts_with("Klassifizierung: intern\n"));
    assert_eq!(
        e.entwurf
            .body_text
            .matches("Klassifizierung: intern")
            .count(),
        1
    );
    assert!(e.entwurf.body_text.contains("Beschluss steht."));
}

// ---------------------------------------------------------------------------
// Fixrunde lva#738 (HOCH 2): Tor-Pruefung und Einfuegen des Protokolls in EINER Schreib-Transaktion
// ---------------------------------------------------------------------------

/// Raeumt die Testhaken des Threads auf, auch wenn der Test vorher scheitert.
struct LueckeAus;

impl Drop for LueckeAus {
    fn drop(&mut self) {
        ZWISCHEN_PRUEFEN_UND_EINFUEGEN.with(|h| *h.borrow_mut() = None);
        VOR_COMMIT.with(|h| *h.borrow_mut() = None);
    }
}

/// Ein Widerruf "eines anderen Prozesses" genau zwischen der letzten Pruefung des Tors und der
/// Schreib-Transaktion: eine EIGENE Verbindung (ohne Tor-Sperre) schreibt ihn und committet.
/// Gibt die Zahl der Aufrufe des Hakens zurueck.
fn widerruf_in_der_luecke(
    dir: &tempfile::TempDir,
    plan_id: &str,
) -> std::rc::Rc<std::cell::Cell<u32>> {
    let aufrufe = std::rc::Rc::new(std::cell::Cell::new(0));
    let zaehler = std::rc::Rc::clone(&aufrufe);
    let pfad = dir.path().join("meetings.db");
    let plan_id = plan_id.to_string();
    ZWISCHEN_PRUEFEN_UND_EINFUEGEN.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            zaehler.set(zaehler.get() + 1);
            let mut conn = crate::managers::meetings::store::open_connection(&pfad).unwrap();
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            plan_store::einwilligung_setzen(
                &tx,
                &testhilfe::widerruf(&plan_id, testhilfe::ANNA),
                testhilfe::NOW + 5,
            )
            .unwrap();
            tx.commit().unwrap();
        }));
    });
    aufrufe
}

fn dokument_im_tor_anlegen(
    store: &MeetingStore,
    meeting_id: &str,
) -> Result<String, SchreibFehler> {
    unter_tor_schreiben(store, meeting_id, |tx| {
        assert!(
            unter_tor_sperre(),
            "Sperrordnung: Tor-Sperre, dann Datenbank"
        );
        MeetingStore::insert_document_in(
            tx,
            meeting_id,
            DOC_KIND,
            "markdown@1",
            "# Protokoll\n",
            None,
            None,
        )
        .map_err(|e| e.to_string())
    })
}

#[test]
fn unter_dem_tor_wird_das_dokument_bei_offenem_tor_geschrieben() {
    let (_dir, store) = fix();
    let (_plan, meeting_id) = geplant(&store, "uid-tor-schreiben-offen");
    let id = dokument_im_tor_anlegen(&store, &meeting_id).expect("alle haben zugestimmt");
    let docs = store.get_documents(&meeting_id).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].id, id);
    assert!(!unter_tor_sperre(), "Sperre wieder frei");
}

#[test]
fn ohne_planung_wird_das_dokument_geschrieben() {
    let (_dir, store) = fix();
    let m = store
        .create_meeting("Ohne Planung", MeetingSource::Empty, Some(1))
        .unwrap();
    store.set_status(&m.id, MeetingStatus::Ready).unwrap();
    dokument_im_tor_anlegen(&store, &m.id).expect("das Tor gilt nur fuer geplante Besprechungen");
    assert_eq!(store.get_documents(&m.id).unwrap().len(), 1);
}

#[test]
fn widerruf_zwischen_pruefung_und_einfuegen_verhindert_das_dokument() {
    let (dir, store) = fix();
    let (plan, meeting_id) = geplant(&store, "uid-tor-schreiben-widerruf");
    let _aus = LueckeAus;
    let aufrufe = widerruf_in_der_luecke(&dir, &plan.id);
    let aufgerufen = std::cell::Cell::new(false);
    let fehler = unter_tor_schreiben(&store, &meeting_id, |tx| {
        aufgerufen.set(true);
        MeetingStore::insert_document_in(tx, &meeting_id, DOC_KIND, "markdown@1", "x", None, None)
            .map_err(|e| e.to_string())
    })
    .expect_err("der Widerruf in der Luecke gewinnt");
    assert_eq!(
        aufrufe.get(),
        1,
        "der Haken lief genau einmal in der Luecke"
    );
    assert_eq!(
        fehler,
        SchreibFehler::Tor(TorFehler::Abgelehnt("abgelehnt"))
    );
    assert_eq!(
        match &fehler {
            SchreibFehler::Tor(tor) => tor.code(),
            other => panic!("{other:?}"),
        },
        "nur_notizen: abgelehnt"
    );
    assert!(!aufgerufen.get(), "geschrieben wird nie");
    assert!(
        store.get_documents(&meeting_id).unwrap().is_empty(),
        "kein Dokument"
    );
}

#[test]
fn ein_fehler_beim_schreiben_nimmt_das_dokument_zurueck() {
    let (_dir, store) = fix();
    let (_plan, meeting_id) = geplant(&store, "uid-tor-schreiben-fehler");
    let fehler = unter_tor_schreiben(&store, &meeting_id, |tx| {
        MeetingStore::insert_document_in(tx, &meeting_id, DOC_KIND, "markdown@1", "x", None, None)
            .map_err(|e| e.to_string())?;
        Err::<(), _>("abbruch_mitten_im_schreiben".to_string())
    })
    .expect_err("Fehler des Aufrufers");
    assert_eq!(
        fehler,
        SchreibFehler::Schreiben("abbruch_mitten_im_schreiben".into())
    );
    assert!(store.get_documents(&meeting_id).unwrap().is_empty());
    // Danach ist alles beim Alten: ein zweiter Versuch schreibt.
    dokument_im_tor_anlegen(&store, &meeting_id).unwrap();
    assert_eq!(store.get_documents(&meeting_id).unwrap().len(), 1);
}

#[test]
fn ein_commitfehler_laesst_kein_dokument_zurueck() {
    let (_dir, store) = fix();
    let (_plan, meeting_id) = geplant(&store, "uid-tor-schreiben-commit");
    // Erst binden: sonst liefe der Haken schon im Commit der Bindung der Vorpruefung.
    tor_pruefen_besprechung(&store, &meeting_id).unwrap();
    let _aus = LueckeAus;
    let commits = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let z = std::rc::Rc::clone(&commits);
    VOR_COMMIT.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |tx: &Transaction<'_>| {
            z.set(z.get() + 1);
            // Aufgeschobener Fremdschluessel: erst beim COMMIT abgewiesen, alles davor ist geschrieben.
            tx.execute_batch(
                "PRAGMA defer_foreign_keys = ON;
                 INSERT INTO meeting_plan_bindings (meeting_id, plan_id, attendees_json, created_at)
                 VALUES ('fremd', 'plan-gibt-es-nicht', '[]', 1);",
            )
            .unwrap();
        }));
    });
    let fehler = dokument_im_tor_anlegen(&store, &meeting_id).expect_err("Commit scheitert");
    assert!(matches!(fehler, SchreibFehler::Schreiben(_)), "{fehler:?}");
    assert_eq!(
        commits.get(),
        1,
        "genau der Commit des Dokuments scheiterte"
    );
    assert!(store.get_documents(&meeting_id).unwrap().is_empty());
}

#[test]
fn ein_gesperrtes_tor_schreibt_nichts_und_ist_nicht_pruefbar() {
    let (_dir, store) = fix();
    let (_plan, meeting_id) = geplant(&store, "uid-tor-schreiben-stoerung");
    TOR_STOERUNG.with(|c| c.set(1));
    let fehler = dokument_im_tor_anlegen(&store, &meeting_id).expect_err("Tor nicht pruefbar");
    assert_eq!(fehler, SchreibFehler::Tor(TorFehler::NichtPruefbar));
    assert!(store.get_documents(&meeting_id).unwrap().is_empty());
}
