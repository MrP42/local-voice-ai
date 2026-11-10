//! D1: sichere Ablage, Papierkorb, Journal, Wiederanlauf, Freigaben. Nur synthetische Daten.

use super::freigabe::{self, KiStufe, KiUnklar};
use super::speicher::{
    Ablage, AblageArt, AblageEinstellungen, AblageImportStand, AblageKollisionswahl, AblageLage,
    AblageSicht, Schritt, ABBRUCH, JOURNAL, TEST_ABBRUCH,
};
use std::fs;
use std::path::{Path, PathBuf};

const GEHEIM: &str = "KONTROLLTEXT-D1-7f3a";

struct Env {
    _d: tempfile::TempDir,
    wurzel: PathBuf,
    quellen: PathBuf,
    a: Ablage,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let wurzel = d.path().join("ablage");
    let quellen = d.path().join("quellen");
    fs::create_dir_all(&quellen).unwrap();
    let a = Ablage::neu(&wurzel);
    assert_eq!(a.wartung(), None);
    Env {
        _d: d,
        wurzel,
        quellen,
        a,
    }
}

impl Env {
    fn quelle(&self, name: &str, inhalt: &str) -> PathBuf {
        let p = self.quellen.join(name);
        fs::write(&p, inhalt).unwrap();
        p
    }

    fn sicht(&self) -> AblageSicht {
        self.a.ansicht().unwrap()
    }

    fn standard(&self) -> String {
        self.sicht().ordner[0].id.clone()
    }

    fn ablegen(&self, name: &str, inhalt: &str, ordner: &str) -> String {
        let q = self.quelle(name, inhalt);
        let z = self.a.importieren(&q, ordner, None).unwrap();
        assert_eq!(z.len(), 1);
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        z[0].dat.clone().unwrap()
    }

    fn original(&self, dat: &str) -> PathBuf {
        self.a.original_pfad(dat).unwrap()
    }

    fn dateien_in(&self, dat: &str) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.wurzel.join("dateien").join(dat))
            .map(|r| {
                r.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    fn neustart(&self) {
        self.a.prozess_neustart();
    }
}

fn abbruch(s: Option<Schritt>) {
    ABBRUCH.with(|a| a.set(s));
}

fn namen(s: &AblageSicht) -> Vec<String> {
    s.dateien.iter().map(|d| d.name.clone()).collect()
}

// --- erster Start, Ablegen ----------------------------------------------------------

#[test]
fn erster_start_legt_standardordner_an() {
    let e = env();
    let s = e.sicht();
    assert_eq!(s.lage, None);
    assert_eq!(s.ordner.len(), 1);
    assert_eq!(s.ordner[0].name, "Allgemein");
    assert!(e.wurzel.join(JOURNAL).exists());
    // zweiter Aufruf legt nichts doppelt an
    assert_eq!(e.sicht().ordner.len(), 1);
}

#[test]
fn datei_wird_sicher_abgelegt_und_im_baum_gefuehrt() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("Rahmenvertrag.pdf", GEHEIM, &o);
    let pfad = e.original(&dat);
    assert_eq!(fs::read_to_string(&pfad).unwrap(), GEHEIM);
    let n = pfad.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        n.starts_with("fas_") && n.ends_with(".pdf"),
        "Original heißt nach Kennung: {n}"
    );
    let s = e.sicht();
    let d = &s.dateien[0];
    assert_eq!(
        (d.name.as_str(), d.typ.as_str(), d.groesse),
        ("Rahmenvertrag.pdf", "pdf", GEHEIM.len() as u64)
    );
    assert_eq!(d.fassungen, 1);
    assert_eq!(d.ki.zustand, "aus", "fehlender Eintrag = aus");
    assert_eq!(d.klasse, "internal");
    assert!(
        e.dateien_in(&dat).iter().all(|f| !f.starts_with(".teil-")),
        "keine Zwischendatei"
    );
    // Journal ist nach dem Baumeintrag leer, keine Namen im Dateisystem
    let roh = fs::read_to_string(e.wurzel.join(JOURNAL)).unwrap();
    assert!(roh.contains("\"journal\": []"));
}

#[test]
fn unbekannte_endung_und_ohne_endung() {
    let e = env();
    let o = e.standard();
    let d1 = e.ablegen("ohneendung", "x", &o);
    let d2 = e.ablegen("Archiv.tar.gz", "y", &o);
    assert!(e
        .original(&d1)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .chars()
        .all(|c| c != '.'));
    assert!(e.original(&d2).to_string_lossy().ends_with(".gz"));
}

// --- Namen (P4 §4.4) -------------------------------------------------------------------

#[test]
fn reservierte_und_gefaehrliche_namen() {
    let e = env();
    // Unter Windows lassen sich solche Quelldateien nicht anlegen (CON ist ein Gerät, `\\` trennt
    // Pfade, `:*?` sind verboten): dort wird die Bereinigung direkt geprüft, sonst zusätzlich über
    // einen echten Import.
    #[cfg(not(windows))]
    {
        let o = e.standard();
        e.ablegen("CON.txt", "a", &o);
        e.ablegen("..\\x", "b", &o);
        e.ablegen("a:b*c?.txt", "c", &o);
        let s = e.sicht();
        let n = namen(&s);
        assert!(n.contains(&"_CON.txt".to_string()), "{n:?}");
        assert!(n.contains(&".._x".to_string()), "{n:?}");
        assert!(n.contains(&"a_b_c_.txt".to_string()), "{n:?}");
        // Nichts außerhalb von ablage/ entstanden
        assert!(!e._d.path().join("x").exists());
    }
    for (roh, erwartet) in [
        ("CON.txt", "_CON.txt"),
        ("..\\x", ".._x"),
        ("a:b*c?.txt", "a_b_c_.txt"),
    ] {
        assert_eq!(super::speicher::bereinigen(roh).unwrap(), erwartet, "{roh}");
    }
    assert!(e.a.ordner_anlegen(None, "..").is_err());
    assert!(e.a.ordner_anlegen(None, "").is_err());
    assert!(e.a.ordner_anlegen(Some("../etc"), "x").is_err());
    let id = e.a.ordner_anlegen(None, "CON").unwrap();
    assert_eq!(
        e.sicht().ordner.iter().find(|x| x.id == id).unwrap().name,
        "_CON"
    );
}

#[test]
fn gross_klein_kollision_wird_gemeldet_nichts_ueberschrieben() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("Bericht.pdf", "eins", &o);
    let q = e.quelle("bericht.PDF", "zwei");
    let z = e.a.importieren(&q, &o, None).unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Kollision);
    assert_eq!(z[0].dat.as_deref(), Some(dat.as_str()));
    let s = e.sicht();
    assert_eq!(s.dateien.len(), 1);
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), "eins");
    assert_eq!(
        e.dateien_in(&dat).len(),
        1,
        "bei Kollision wird nichts geschrieben"
    );
}

#[test]
fn kollision_neue_fassung_haengt_an_den_kopf() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("Plan.docx", "v1", &o);
    let q = e.quelle("Plan.docx", "v2 länger");
    let z =
        e.a.importieren(&q, &o, Some(AblageKollisionswahl::NeueFassung))
            .unwrap();
    assert_eq!(z[0].stand, AblageImportStand::NeueFassung);
    assert_eq!(z[0].dat.as_deref(), Some(dat.as_str()));
    let f = e.a.fassungen(&dat).unwrap();
    assert_eq!(f.len(), 2);
    assert!(f[0].aktuell && f[0].kopf && f[0].nummer == 2, "{f:?}");
    assert_eq!(f[0].vorgaenger.as_deref(), Some(f[1].fas.as_str()));
    assert!(!f[1].aktuell);
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), "v2 länger");
    assert_eq!(
        e.dateien_in(&dat).len(),
        2,
        "die alte Fassung bleibt unverändert liegen"
    );
    assert_eq!(e.sicht().dateien[0].fassungen, 2);
    assert!(!e.sicht().dateien[0].konflikt);
}

#[test]
fn kollision_beide_behalten_ergibt_neue_datei_mit_zusatz() {
    let e = env();
    let o = e.standard();
    e.ablegen("Plan.docx", "v1", &o);
    let q = e.quelle("Plan.docx", "v2");
    let z =
        e.a.importieren(&q, &o, Some(AblageKollisionswahl::BeideBehalten))
            .unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Abgelegt);
    assert_eq!(z[0].name.as_deref(), Some("Plan (2).docx"));
    let q3 = e.quelle("Plan.docx", "v3");
    e.a.importieren(&q3, &o, Some(AblageKollisionswahl::BeideBehalten))
        .unwrap();
    let n = namen(&e.sicht());
    assert_eq!(n.len(), 3);
    assert!(n.contains(&"Plan (3).docx".to_string()));
}

// --- Verknüpfungen --------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn verknuepfung_als_quelle_wird_abgewiesen() {
    let e = env();
    let o = e.standard();
    let echt = e.quelle("echt.txt", "geheim");
    let link = e.quellen.join("link.txt");
    std::os::unix::fs::symlink(&echt, &link).unwrap();
    let z = e.a.importieren(&link, &o, None).unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Uebersprungen);
    assert_eq!(z[0].code.as_deref(), Some("ablage.verknuepfung"));
    assert!(e.sicht().dateien.is_empty());
    // Verknüpfungen in einem Ordner werden beim Ordnerimport übersprungen
    let dir = e.quellen.join("Ordner");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a.txt"), "a").unwrap();
    std::os::unix::fs::symlink(&echt, dir.join("l.txt")).unwrap();
    let z = e.a.importieren(&dir, &o, None).unwrap();
    assert_eq!(
        z.iter()
            .filter(|x| x.stand == AblageImportStand::Abgelegt)
            .count(),
        1
    );
    assert_eq!(
        z.iter()
            .filter(|x| x.stand == AblageImportStand::Uebersprungen)
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn verknuepfung_als_ziel_wird_abgewiesen() {
    let e = env();
    let o = e.standard();
    let aussen = e._d.path().join("aussen");
    fs::create_dir(&aussen).unwrap();
    // `dateien` wird durch eine Verknüpfung nach außen ersetzt.
    fs::remove_dir_all(e.wurzel.join("dateien")).unwrap();
    std::os::unix::fs::symlink(&aussen, e.wurzel.join("dateien")).unwrap();
    let q = e.quelle("x.txt", "inhalt");
    let z = e.a.importieren(&q, &o, None).unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Fehler);
    assert_eq!(z[0].code.as_deref(), Some("ablage.verknuepfung"));
    assert_eq!(
        fs::read_dir(&aussen).unwrap().count(),
        0,
        "nichts außerhalb von ablage/ geschrieben"
    );
}

#[cfg(unix)]
#[test]
fn verknuepfung_des_dat_ordners_wird_abgewiesen() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    let aussen = e._d.path().join("aussen2");
    fs::create_dir(&aussen).unwrap();
    let dir_dat = e.wurzel.join("dateien").join(&dat);
    fs::remove_dir_all(&dir_dat).unwrap();
    std::os::unix::fs::symlink(&aussen, &dir_dat).unwrap();
    let q = e.quelle("a.txt", "neu");
    let z =
        e.a.importieren(&q, &o, Some(AblageKollisionswahl::NeueFassung))
            .unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Fehler);
    assert_eq!(fs::read_dir(&aussen).unwrap().count(), 0);
    assert!(
        e.a.original_pfad(&dat).is_err(),
        "Öffnen folgt keiner Verknüpfung"
    );
}

// --- Abbruch und Wiederanlauf -------------------------------------------------------

fn abbruch_import(e: &Env, schritt: Schritt) -> String {
    let o = e.standard();
    let q = e.quelle("Angebot.pdf", GEHEIM);
    abbruch(Some(schritt));
    let z = e.a.importieren(&q, &o, None).unwrap();
    abbruch(None);
    assert_eq!(z[0].stand, AblageImportStand::Fehler);
    assert_eq!(z[0].code.as_deref(), Some(TEST_ABBRUCH));
    o
}

fn alle_dateien_unter(p: &Path) -> Vec<String> {
    fn rek(p: &Path, aus: &mut Vec<String>) {
        if let Ok(r) = fs::read_dir(p) {
            for e in r.flatten() {
                if e.path().is_dir() {
                    rek(&e.path(), aus);
                } else {
                    aus.push(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    let mut v = Vec::new();
    rek(p, &mut v);
    v
}

#[test]
fn abbruch_nach_zwischendatei_kein_eintrag() {
    let e = env();
    abbruch_import(&e, Schritt::NachZwischendatei);
    e.neustart();
    assert_eq!(e.a.wartung(), None);
    let s = e.sicht();
    assert!(s.dateien.is_empty() && s.gefunden.is_empty());
    assert!(
        alle_dateien_unter(&e.wurzel.join("dateien")).is_empty(),
        "keine Zwischendatei im Baum"
    );
    assert_eq!(
        alle_dateien_unter(&e.wurzel.join("papierkorb")).len(),
        1,
        "Zwischendatei im Papierkorb"
    );
}

#[test]
fn abbruch_nach_hash_kein_eintrag() {
    let e = env();
    abbruch_import(&e, Schritt::NachHash);
    e.neustart();
    let s = e.sicht();
    assert!(s.dateien.is_empty() && s.gefunden.is_empty());
    assert!(alle_dateien_unter(&e.wurzel.join("dateien")).is_empty());
}

#[test]
fn abbruch_nach_umbenennen_wird_als_fund_angeboten_nie_still_aufgenommen() {
    let e = env();
    let o = abbruch_import(&e, Schritt::NachUmbenennen);
    e.neustart();
    let s = e.sicht();
    assert!(s.dateien.is_empty(), "nicht still aufgenommen");
    assert_eq!(s.gefunden.len(), 1);
    let dat = s.gefunden[0].dat.clone();
    e.a.gefundene_aufnehmen(&dat, &o).unwrap();
    let s = e.sicht();
    assert_eq!(s.dateien.len(), 1);
    assert!(s.dateien[0].name.starts_with("Gefundene Datei "));
    assert!(s.dateien[0].name.ends_with(".pdf"));
    assert!(s.gefunden.is_empty());
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), GEHEIM);
}

#[test]
fn abbruch_nach_journal_wird_nachgetragen() {
    let e = env();
    abbruch_import(&e, Schritt::NachJournal);
    // Ohne Neustart des Prozesses: das Journal wird trotzdem bei der Wartung zu Ende geführt.
    let s = e.sicht();
    assert_eq!(s.dateien.len(), 1);
    assert_eq!(s.dateien[0].name, "Angebot.pdf");
    assert!(s.gefunden.is_empty());
    assert_eq!(
        fs::read_to_string(e.original(&s.dateien[0].id)).unwrap(),
        GEHEIM
    );
    let roh = fs::read_to_string(e.wurzel.join(JOURNAL)).unwrap();
    assert!(roh.contains("\"journal\": []"));
}

#[test]
fn abbruch_nach_journal_bei_neuer_fassung_haengt_an() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("A.pdf", "eins", &o);
    let q = e.quelle("A.pdf", "zwei");
    abbruch(Some(Schritt::NachJournal));
    let _ =
        e.a.importieren(&q, &o, Some(AblageKollisionswahl::NeueFassung))
            .unwrap();
    abbruch(None);
    assert_eq!(e.a.fassungen(&dat).unwrap().len(), 1);
    e.sicht();
    let f = e.a.fassungen(&dat).unwrap();
    assert_eq!(f.len(), 2);
    assert!(f[0].aktuell);
}

#[test]
fn wiederanlauf_prueft_das_original_vor_dem_nachtragen() {
    let e = env();
    abbruch_import(&e, Schritt::NachJournal);
    // Original wird zwischen Journal und Wiederanlauf verändert: kein Eintrag, kein stilles Aufnehmen.
    let dat = alle_dateien_unter_dirs(&e.wurzel.join("dateien"));
    let p = e.wurzel.join("dateien").join(&dat);
    let f = fs::read_dir(&p).unwrap().flatten().next().unwrap().path();
    fs::write(&f, "VERÄNDERT").unwrap();
    let s = e.sicht();
    assert!(s.dateien.is_empty());
    assert_eq!(
        s.gefunden.len(),
        1,
        "das Original bleibt auffindbar, der Eintrag geht nicht still verloren"
    );
}

fn alle_dateien_unter_dirs(p: &Path) -> String {
    fs::read_dir(p)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned()
}

// --- Journal defekt, Lesefehler ≠ leer, Neuaufbau --------------------------------------

#[test]
fn journal_defekt_blockiert_bis_bestand_neu_aufbauen() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("A.pdf", "inhalt", &o);
    fs::write(e.wurzel.join(JOURNAL), b"{kaputt").unwrap();
    let s = e.sicht();
    assert_eq!(s.lage, Some(AblageLage::Defekt));
    assert!(s.dateien.is_empty());
    let q = e.quelle("B.pdf", "b");
    let z = e.a.importieren(&q, &o, None).unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Fehler, "blockiert");
    assert_eq!(z[0].code.as_deref(), Some("ablage.journal_defekt"));
    assert!(e.a.ordner_anlegen(None, "X").is_err());
    let n = e.a.neu_aufbauen().unwrap();
    assert_eq!(n, 1);
    let defekt_da = fs::read_dir(&e.wurzel).unwrap().flatten().any(|x| {
        x.file_name()
            .to_string_lossy()
            .starts_with("ablage.json.defekt-")
    });
    assert!(
        defekt_da,
        "die defekte Datei wird beiseitegelegt, nie gelöscht"
    );
    let s = e.sicht();
    assert_eq!(s.lage, None);
    assert_eq!(s.gefunden.len(), 1, "angeboten, nicht still aufgenommen");
    assert_eq!(s.gefunden[0].dat, dat);
    assert!(s.dateien.is_empty());
    assert!(e.a.neu_aufbauen().is_err(), "lesbar: nichts neu aufzubauen");
    let o2 = s.ordner[0].id.clone();
    e.a.gefundene_aufnehmen(&dat, &o2).unwrap();
    assert_eq!(e.sicht().dateien.len(), 1);
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), "inhalt");
}

#[test]
fn journal_fehlt_mit_dateien_ist_unklar_nicht_leer() {
    let e = env();
    let o = e.standard();
    e.ablegen("A.pdf", "x", &o);
    fs::remove_file(e.wurzel.join(JOURNAL)).unwrap();
    assert_eq!(e.sicht().lage, Some(AblageLage::Fehlt));
    assert_eq!(e.a.neu_aufbauen().unwrap(), 1);
}

#[test]
fn journal_fehlt_ohne_dateien_ist_frisch() {
    let e = env();
    fs::remove_file(e.wurzel.join(JOURNAL)).unwrap();
    assert_eq!(e.sicht().lage, None);
}

#[test]
fn lesefehler_ist_nicht_leer() {
    let e = env();
    let o = e.standard();
    e.ablegen("A.pdf", "x", &o);
    // `ablage.json` ist ein Verzeichnis: Lesefehler, kein NotFound.
    fs::remove_file(e.wurzel.join(JOURNAL)).unwrap();
    fs::create_dir(e.wurzel.join(JOURNAL)).unwrap();
    let s = e.sicht();
    assert_eq!(s.lage, Some(AblageLage::NichtLesbar));
    assert!(s.ordner.is_empty());
    assert!(
        e.a.neu_aufbauen().is_err(),
        "ein nur nicht lesbares Journal wird nie neu aufgebaut"
    );
}

#[test]
fn neuere_version_wird_nie_neu_aufgebaut() {
    let e = env();
    fs::write(
        e.wurzel.join(JOURNAL),
        br#"{"v":99,"geraet":"x","epoche":1,"neu":1}"#,
    )
    .unwrap();
    assert_eq!(e.sicht().lage, Some(AblageLage::Neuer));
    assert!(e.a.neu_aufbauen().is_err());
    assert!(e.wurzel.join(JOURNAL).exists());
}

#[test]
fn ungueltige_kennung_im_journal_ist_defekt() {
    let e = env();
    let o = e.standard();
    e.ablegen("A.pdf", "x", &o);
    let roh = fs::read_to_string(e.wurzel.join(JOURNAL)).unwrap();
    let kaputt = roh.replacen("dat_", "dat_../", 1);
    fs::write(e.wurzel.join(JOURNAL), kaputt).unwrap();
    assert_eq!(e.sicht().lage, Some(AblageLage::Defekt));
}

// --- Papierkorb -------------------------------------------------------------------------

#[test]
fn papierkorb_und_wiederherstellen() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("Vertrag.pdf", GEHEIM, &o);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    let s = e.sicht();
    assert!(s.dateien.is_empty(), "nicht mehr in der Liste");
    assert_eq!(s.papierkorb.len(), 1);
    assert_eq!(s.papierkorb[0].tage_rest, 30);
    assert_eq!(s.papierkorb[0].ursprung, "Allgemein");
    assert!(
        e.wurzel.join("dateien").join(&dat).exists(),
        "Original bleibt bis zum endgültigen Löschen"
    );
    assert!(e.a.in_papierkorb(AblageArt::Datei, &dat).is_err());
    // Name inzwischen belegt: Wiederherstellen gibt einen freien Namen, überschreibt nichts.
    let neu = e.ablegen("Vertrag.pdf", "zweite", &o);
    e.a.wiederherstellen(AblageArt::Datei, &dat).unwrap();
    let s = e.sicht();
    assert!(s.papierkorb.is_empty());
    let n = namen(&s);
    assert_eq!(n.len(), 2);
    assert!(
        n.contains(&"Vertrag.pdf".to_string()) && n.contains(&"Vertrag (2).pdf".to_string()),
        "{n:?}"
    );
    assert_eq!(fs::read_to_string(e.original(&neu)).unwrap(), "zweite");
    assert!(
        e.a.wiederherstellen(AblageArt::Datei, &dat).is_err(),
        "liegt nicht im Papierkorb"
    );
}

#[test]
fn ordner_im_papierkorb_nimmt_den_inhalt_mit() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(Some(&root), "Verträge").unwrap();
    let w = e.a.ordner_anlegen(Some(&v), "Wartung").unwrap();
    e.ablegen("a.pdf", "a", &w);
    e.a.in_papierkorb(AblageArt::Ordner, &v).unwrap();
    let s = e.sicht();
    assert!(s.dateien.is_empty());
    assert!(s.ordner.iter().all(|o| o.id != v && o.id != w));
    assert_eq!(s.papierkorb.len(), 1, "nur die oberste Ebene wird gelistet");
    assert!(
        e.a.ordner_anlegen(Some(&w), "Neu").is_err(),
        "in den Papierkorb nichts anlegen"
    );
    e.a.wiederherstellen(AblageArt::Ordner, &v).unwrap();
    let s = e.sicht();
    assert_eq!(s.dateien.len(), 1);
    assert_eq!(s.ordner.len(), 3);
}

#[test]
fn endgueltig_loeschen_entfernt_original_text_und_eintrag() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("Geheim.pdf", GEHEIM, &o);
    // Textfassung (D2) und Rest einer Zwischenablage mit abräumen
    let text = e.wurzel.join("text").join(&dat);
    fs::create_dir_all(&text).unwrap();
    fs::write(text.join("rev.json"), GEHEIM).unwrap();
    assert!(
        e.a.endgueltig_loeschen(AblageArt::Datei, &dat).is_err(),
        "nur aus dem Papierkorb"
    );
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    let b = e.a.endgueltig_loeschen(AblageArt::Datei, &dat).unwrap();
    assert_eq!((b.dateien, b.ordner), (1, 0));
    assert!(!e.wurzel.join("dateien").join(&dat).exists());
    assert!(!text.exists());
    assert!(e.sicht().papierkorb.is_empty());
    assert!(alle_dateien_unter(&e.wurzel)
        .iter()
        .all(|n| n != "rev.json"));
    let roh = fs::read_to_string(e.wurzel.join(JOURNAL)).unwrap();
    assert!(roh.contains(&dat), "Kennung steht als Grabstein im Journal");
    assert!(!roh.contains(GEHEIM) && !roh.contains("Geheim.pdf"));
    // Papierkorb enthält keine Reste des Originals
    assert!(alle_dateien_unter(&e.wurzel.join("papierkorb")).is_empty());
}

#[test]
fn endgueltig_loeschen_wird_nach_abbruch_zu_ende_gefuehrt() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("A.pdf", "x", &o);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    abbruch(Some(Schritt::LoeschenNachJournal));
    assert!(e.a.endgueltig_loeschen(AblageArt::Datei, &dat).is_err());
    abbruch(None);
    assert!(
        e.wurzel.join("dateien").join(&dat).exists(),
        "noch nichts entfernt"
    );
    let s = e.sicht();
    assert!(s.papierkorb.is_empty());
    assert!(
        !e.wurzel.join("dateien").join(&dat).exists(),
        "Wiederanlauf führt das Löschen zu Ende"
    );
}

#[test]
fn papierkorb_30_tage_dann_endgueltig_und_leeren() {
    let e = env();
    let o = e.standard();
    let a = e.ablegen("alt.pdf", "a", &o);
    let b = e.ablegen("neu.pdf", "b", &o);
    e.a.in_papierkorb(AblageArt::Datei, &a).unwrap();
    e.a.in_papierkorb(AblageArt::Datei, &b).unwrap();
    // `alt.pdf` wurde vor 31 Tagen gelöscht.
    let pfad = e.wurzel.join(JOURNAL);
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&pfad).unwrap()).unwrap();
    let alt = super::jetzt_ms() - 31 * 24 * 60 * 60 * 1000;
    v["dateien"][&a]["papierkorb"]["wert"] = serde_json::json!(alt);
    fs::write(&pfad, serde_json::to_vec(&v).unwrap()).unwrap();
    let s = e.sicht();
    assert_eq!(s.papierkorb.len(), 1, "abgelaufener Eintrag ist weg");
    assert_eq!(s.papierkorb[0].name, "neu.pdf");
    assert!(!e.wurzel.join("dateien").join(&a).exists());
    assert!(e.wurzel.join("dateien").join(&b).exists());
    let r = e.a.papierkorb_leeren().unwrap();
    assert_eq!(r.dateien, 1);
    assert!(e.sicht().papierkorb.is_empty());
    assert!(!e.wurzel.join("dateien").join(&b).exists());
}

// --- Ordner -------------------------------------------------------------------------------

#[test]
fn ordner_anlegen_umbenennen_verschieben() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(None, "Verträge").unwrap();
    let w = e.a.ordner_anlegen(Some(&v), "Wartung").unwrap();
    assert!(
        e.a.ordner_anlegen(Some(&v), "wartung").is_err(),
        "gleicher Name ohne Groß/Klein"
    );
    e.a.ordner_umbenennen(&w, "Wartung IT").unwrap();
    assert!(e.a.ordner_umbenennen(&w, "").is_err());
    e.a.ordner_verschieben(&w, Some(&root)).unwrap();
    let s = e.sicht();
    let ow = s.ordner.iter().find(|o| o.id == w).unwrap();
    assert_eq!(
        (ow.name.as_str(), ow.eltern.as_deref()),
        ("Wartung IT", Some(root.as_str()))
    );
    // Zyklus wird abgewiesen
    assert!(e.a.ordner_verschieben(&v, Some(&v)).is_err());
    e.a.ordner_verschieben(&v, Some(&w)).unwrap();
    assert!(
        e.a.ordner_verschieben(&w, Some(&v)).is_err(),
        "unter den eigenen Unterordner"
    );
    assert!(e.a.ordner_verschieben(&root, Some(&v)).is_err());
    // Datei verschieben und umbenennen
    let dat = e.ablegen("a.txt", "a", &v);
    e.ablegen("b.txt", "b", &w);
    assert!(e.a.datei_verschieben(&dat, &w).is_ok());
    let dat2 = e.ablegen("a.txt", "a2", &v);
    assert!(
        e.a.datei_verschieben(&dat2, &w).is_err(),
        "Name im Ziel vergeben"
    );
    assert!(e.a.datei_umbenennen(&dat2, "B.TXT").is_ok());
    assert!(e.a.datei_verschieben(&dat2, &w).is_err());
}

#[test]
fn verzeichnis_import_behaelt_die_struktur() {
    let e = env();
    let o = e.standard();
    let dir = e.quellen.join("Projekt");
    fs::create_dir_all(dir.join("Unter/Tief")).unwrap();
    fs::write(dir.join("a.txt"), "a").unwrap();
    fs::write(dir.join("Unter/b.txt"), "b").unwrap();
    fs::write(dir.join("Unter/Tief/c.txt"), "c").unwrap();
    let z = e.a.importieren(&dir, &o, None).unwrap();
    assert_eq!(
        z.iter()
            .filter(|x| x.stand == AblageImportStand::Abgelegt)
            .count(),
        3
    );
    let s = e.sicht();
    let id = |n: &str| s.ordner.iter().find(|x| x.name == n).unwrap().id.clone();
    let (p, u, t) = (id("Projekt"), id("Unter"), id("Tief"));
    assert_eq!(
        s.ordner
            .iter()
            .find(|x| x.id == p)
            .unwrap()
            .eltern
            .as_deref(),
        Some(o.as_str())
    );
    assert_eq!(
        s.ordner
            .iter()
            .find(|x| x.id == u)
            .unwrap()
            .eltern
            .as_deref(),
        Some(p.as_str())
    );
    assert_eq!(
        s.ordner
            .iter()
            .find(|x| x.id == t)
            .unwrap()
            .eltern
            .as_deref(),
        Some(u.as_str())
    );
    assert_eq!(
        s.dateien.iter().find(|d| d.name == "c.txt").unwrap().ordner,
        t
    );
    // Zweiter Import: Kollisionen werden gemeldet, vorhandene Ordner wiederverwendet.
    let z = e.a.importieren(&dir, &o, None).unwrap();
    assert_eq!(
        z.iter()
            .filter(|x| x.stand == AblageImportStand::Kollision)
            .count(),
        3
    );
    assert_eq!(
        e.sicht()
            .ordner
            .iter()
            .filter(|x| x.name == "Unter")
            .count(),
        1
    );
}

#[test]
fn einstellungen_klasse_wird_angehoben_nur_geraet_ist_unterbaumregel() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(Some(&root), "Verträge").unwrap();
    let w = e.a.ordner_anlegen(Some(&v), "Wartung").unwrap();
    let set = |klasse: Option<&str>, nur: Option<bool>, ki: Option<bool>| AblageEinstellungen {
        datenklasse: klasse.map(Into::into),
        nur_geraet: nur,
        zielordner_ki: ki,
    };
    e.a.ordner_einstellungen(&v, &set(Some("confidential"), Some(true), Some(true)))
        .unwrap();
    e.a.ordner_einstellungen(&w, &set(Some("public"), None, None))
        .unwrap();
    let s = e.sicht();
    let ow = s.ordner.iter().find(|o| o.id == w).unwrap();
    assert_eq!(ow.datenklasse, "public", "gespeichert bleibt der Wert");
    assert_eq!(ow.klasse, "confidential", "wirksam angehoben");
    assert!(ow.nur_geraet_wirksam && !ow.nur_geraet);
    assert!(s.ordner.iter().find(|o| o.id == v).unwrap().zielordner_ki);
    assert!(
        e.a.ordner_einstellungen(&w, &set(None, Some(false), None))
            .is_err(),
        "nicht aufhebbar"
    );
    assert!(e
        .a
        .ordner_einstellungen(&w, &set(Some("geheim"), None, None))
        .is_err());
    let dat = e.ablegen("x.pdf", "x", &w);
    let d = e.sicht().dateien.into_iter().find(|d| d.id == dat).unwrap();
    assert_eq!(d.klasse, "confidential");
    assert!(d.nur_geraet);
}

// --- KI-Freigabe über die Ablage -------------------------------------------------------

#[test]
fn ki_freigabe_vererbt_gesperrt_gewinnt_und_ist_gerätelokal() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(Some(&root), "Verträge").unwrap();
    let w = e.a.ordner_anlegen(Some(&v), "Wartung").unwrap();
    let dat = e.ablegen("x.pdf", "x", &w);
    let ki = |e: &Env, id: &str| {
        e.sicht()
            .ordner
            .iter()
            .find(|o| o.id == id)
            .unwrap()
            .ki
            .clone()
    };
    assert_eq!(ki(&e, &w).zustand, "aus");
    e.a.ki_setzen(&v, Some(KiStufe::An)).unwrap();
    let k = ki(&e, &w);
    assert_eq!(
        (k.zustand.as_str(), k.von.as_deref(), k.von_name.as_deref()),
        ("an", Some(v.as_str()), Some("Verträge"))
    );
    assert_eq!(
        e.sicht()
            .dateien
            .iter()
            .find(|d| d.id == dat)
            .unwrap()
            .ki
            .zustand,
        "an"
    );
    e.a.ki_setzen(&root, Some(KiStufe::Gesperrt)).unwrap();
    assert_eq!(
        ki(&e, &w).zustand,
        "gesperrt",
        "gesperrt gewinnt auch über ein näheres an"
    );
    e.a.ki_setzen(&root, None).unwrap();
    assert_eq!(ki(&e, &w).zustand, "an");
    // gerätelokal: der synchronisierte Ordnerdatensatz kennt kein ki-Feld
    let roh = fs::read_to_string(e.wurzel.join(JOURNAL)).unwrap();
    assert!(!roh.contains("\"ki\""));
    assert!(e.wurzel.join(freigabe::DATEI).exists());
}

#[test]
fn defekte_freigabe_sperrt_alles_und_stellt_aus_vorher_wieder_her() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(Some(&root), "Verträge").unwrap();
    e.a.ki_setzen(&v, Some(KiStufe::Gesperrt)).unwrap();
    e.a.ki_setzen(&root, Some(KiStufe::An)).unwrap();
    // Defekt, gültige .vorher (mit gesperrt): automatische Wiederherstellung
    fs::write(e.wurzel.join(freigabe::DATEI), b"\xff\xfe nicht json").unwrap();
    let s = e.sicht();
    assert_eq!(s.freigabe_unklar, None);
    assert_eq!(
        s.ordner.iter().find(|o| o.id == v).unwrap().ki.zustand,
        "gesperrt",
        "gesperrt bleibt"
    );
    // Defekt ohne brauchbare .vorher: unklar, alles gesperrt, nur „neu festlegen“
    fs::write(e.wurzel.join(freigabe::DATEI), b"kaputt").unwrap();
    fs::write(e.wurzel.join(freigabe::VORHER), b"auch kaputt").unwrap();
    let s = e.sicht();
    assert_eq!(s.freigabe_unklar, Some(KiUnklar::Defekt));
    assert!(s.ordner.iter().all(|o| o.ki.zustand == "unklar"));
    assert!(s.dateien.iter().all(|d| d.ki.zustand == "unklar"));
    assert!(e.a.ki_setzen(&v, Some(KiStufe::An)).is_err());
    e.a.freigaben_neu_festlegen().unwrap();
    let s = e.sicht();
    assert_eq!(s.freigabe_unklar, None);
    assert!(s.ordner.iter().all(|o| o.ki.zustand == "aus"));
    assert!(fs::read_dir(&e.wurzel).unwrap().flatten().any(|x| x
        .file_name()
        .to_string_lossy()
        .starts_with("freigabe.json.defekt-")));
}

#[test]
fn endgueltig_geloeschter_ordner_verliert_seine_freigabe() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(Some(&root), "Weg").unwrap();
    e.a.ki_setzen(&v, Some(KiStufe::An)).unwrap();
    e.a.in_papierkorb(AblageArt::Ordner, &v).unwrap();
    e.a.endgueltig_loeschen(AblageArt::Ordner, &v).unwrap();
    let f = fs::read_to_string(e.wurzel.join(freigabe::DATEI)).unwrap();
    assert!(!f.contains(&v));
}

// --- Fassungen: Aufbewahrung ---------------------------------------------------------------

#[test]
fn fassungen_aufraeumen_entfernt_alte_und_behaelt_kopf() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("A.pdf", "v0", &o);
    for i in 1..=12 {
        let q = e.quelle("A.pdf", &format!("v{i}"));
        e.a.importieren(&q, &o, Some(AblageKollisionswahl::NeueFassung))
            .unwrap();
    }
    assert_eq!(e.a.fassungen(&dat).unwrap().len(), 13);
    // 200 Tage später: nur noch die 10 jüngsten bleiben
    let spaeter = super::jetzt_ms() + 200 * 24 * 60 * 60 * 1000;
    let n = e.a.fassungen_aufraeumen(&dat, spaeter).unwrap();
    assert_eq!(n, 3);
    let f = e.a.fassungen(&dat).unwrap();
    assert_eq!(f.len(), 10);
    assert!(f[0].aktuell);
    assert_eq!(
        e.dateien_in(&dat).len(),
        10,
        "Originale der entfernten Fassungen sind weg"
    );
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), "v12");
}

// --- Keine Inhalte im Fehlertext ----------------------------------------------------------

#[test]
fn fehlertexte_enthalten_keine_namen() {
    let e = env();
    let o = e.standard();
    let q = e.quelle("Streng Geheim.pdf", GEHEIM);
    fs::write(e.wurzel.join(JOURNAL), b"kaputt").unwrap();
    let z = e.a.importieren(&q, &o, None).unwrap();
    let text = format!("{:?}", z);
    // Die Zeile trägt den Quellpfad für die Zuordnung in der Oberfläche, aber nur einen Code.
    assert_eq!(z[0].code.as_deref(), Some("ablage.journal_defekt"));
    assert!(!text.contains(GEHEIM));
    let f = super::Fehler::neu("ablage.x", "Allgemeiner Text.");
    assert_eq!(String::from(f), "ablage.x: Allgemeiner Text.");
}

// --- Review-Korrekturen ---------------------------------------------------------------------

fn journal_json(e: &Env) -> serde_json::Value {
    serde_json::from_slice(&fs::read(e.wurzel.join(JOURNAL)).unwrap()).unwrap()
}

fn journal_schreiben(e: &Env, v: &serde_json::Value) {
    fs::write(e.wurzel.join(JOURNAL), serde_json::to_vec(v).unwrap()).unwrap();
}

#[test]
fn unterbaum_folgt_der_wirksamen_zuordnung_nicht_der_gespeicherten() {
    let e = env();
    let root = e.standard();
    let a = e.a.ordner_anlegen(Some(&root), "A").unwrap();
    let b = e.a.ordner_anlegen(Some(&root), "B").unwrap();
    let dat = e.ablegen("b.txt", "b", &b);
    // Zyklus A <-> B: A (höherer Rang) liegt unter B, B unter „Wiederhergestellt“.
    let mut v = journal_json(&e);
    v["ordner"][&a]["eltern"] = serde_json::json!({"wert": b, "zaehler": 1000, "geraet": "g_t"});
    v["ordner"][&b]["eltern"] = serde_json::json!({"wert": a, "zaehler": 900, "geraet": "g_t"});
    journal_schreiben(&e, &v);
    let s = e.sicht();
    assert!(
        s.ordner
            .iter()
            .find(|o| o.id == b)
            .unwrap()
            .wiederhergestellt
    );
    e.a.in_papierkorb(AblageArt::Ordner, &a).unwrap();
    e.a.endgueltig_loeschen(AblageArt::Ordner, &a).unwrap();
    let s = e.sicht();
    assert!(s.ordner.iter().any(|o| o.id == b), "B bleibt erhalten");
    assert!(s.dateien.iter().any(|d| d.id == dat), "Datei in B bleibt");
    assert!(e.original(&dat).exists());
}

#[test]
fn journal_geht_bei_fehler_nicht_verloren() {
    let e = env();
    let o = e.standard();
    let d1 = e.ablegen("a.txt", "a", &o);
    let d2 = e.ablegen("b.txt", "b", &o);
    // Löschen von d1 scheitert (ein Verzeichnis ist durch eine Datei ersetzt).
    fs::remove_dir_all(e.wurzel.join("dateien").join(&d1)).unwrap();
    fs::write(e.wurzel.join("dateien").join(&d1), "x").unwrap();
    let mut v = journal_json(&e);
    v["journal"] = serde_json::json!([
        {"art": "loeschen", "dat": [d1], "ordner": []},
        {"art": "loeschen", "dat": [d2], "ordner": []}
    ]);
    journal_schreiben(&e, &v);
    e.sicht();
    let j = journal_json(&e);
    assert_eq!(
        j["journal"].as_array().unwrap().len(),
        2,
        "fehlgeschlagener und nachfolgender Eintrag bleiben im Journal"
    );
    assert!(e.wurzel.join("dateien").join(&d2).exists());
    e.sicht();
    assert!(journal_json(&e)["journal"].as_array().unwrap().is_empty());
    assert!(!e.wurzel.join("dateien").join(&d2).exists());
}

#[test]
fn teildateien_im_papierkorb_werden_geleert_und_laufen_ab() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    let teil = |n: &str| {
        let fas = super::neue_id(super::PRAEFIX_FASSUNG);
        let p = e
            .wurzel
            .join("dateien")
            .join(&dat)
            .join(format!(".teil-{fas}"));
        fs::write(&p, n).unwrap();
        fas
    };
    let alt = teil("alt");
    e.sicht();
    let ziel_alt = e.wurzel.join("papierkorb").join(format!("teil-{alt}"));
    assert!(
        ziel_alt.exists(),
        "liegengebliebene Zwischendatei im Papierkorb"
    );
    let spaet = std::time::SystemTime::now() - std::time::Duration::from_secs(31 * 24 * 3600);
    fs::OpenOptions::new()
        .write(true)
        .open(&ziel_alt)
        .unwrap()
        .set_modified(spaet)
        .unwrap();
    let neu = teil("neu");
    e.sicht();
    assert!(!ziel_alt.exists(), "nach 30 Tagen entfernt");
    let ziel_neu = e.wurzel.join("papierkorb").join(format!("teil-{neu}"));
    assert!(ziel_neu.exists(), "jüngere bleibt");
    e.a.papierkorb_leeren().unwrap();
    assert!(
        !ziel_neu.exists(),
        "Papierkorb leeren entfernt auch Zwischendateien"
    );
}

#[test]
fn ki_sperre_der_gespeicherten_elternkette_gilt_unter_wiederhergestellt() {
    let e = env();
    let root = e.standard();
    let a = e.a.ordner_anlegen(Some(&root), "A").unwrap();
    let b = e.a.ordner_anlegen(Some(&root), "B").unwrap();
    e.a.ordner_einstellungen(
        &a,
        &AblageEinstellungen {
            nur_geraet: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    e.a.ki_setzen(&a, Some(KiStufe::Gesperrt)).unwrap();
    // B: gespeicherter Elternordner A, aber wirksam unter „Wiederhergestellt“ (Zyklus).
    let mut v = journal_json(&e);
    v["ordner"][&a]["eltern"] = serde_json::json!({"wert": b, "zaehler": 1000, "geraet": "g_t"});
    v["ordner"][&b]["eltern"] = serde_json::json!({"wert": a, "zaehler": 900, "geraet": "g_t"});
    journal_schreiben(&e, &v);
    let s = e.sicht();
    let ob = s.ordner.iter().find(|o| o.id == b).unwrap();
    assert!(ob.wiederhergestellt);
    assert_eq!(ob.ki.zustand, "gesperrt");
    assert!(ob.nur_geraet_wirksam);
}

// --- Review-Korrekturen, Runde 2 (Codex) -----------------------------------------------------

fn fund_schreiben(e: &Env, v: &mut serde_json::Value, dat: &str, inhalt: &str) -> String {
    let fas = super::neue_id(super::PRAEFIX_FASSUNG);
    let dir = e.wurzel.join("dateien").join(dat);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{fas}.txt")), inhalt).unwrap();
    v["gefunden"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"dat": dat, "fas": fas, "typ": "txt", "groesse": 1, "ms": 5}));
    fas
}

#[test]
fn gefundene_aufnehmen_erfindet_keine_abstammung() {
    let e = env();
    let o = e.standard();
    // Bestehende Datei mit einem Kopf, dazu zwei unabhängige Funde derselben `dat`.
    let dat = e.ablegen("a.txt", "a", &o);
    let kopf = e.a.fassungen(&dat).unwrap()[0].fas.clone();
    let mut v = journal_json(&e);
    v["gefunden"] = serde_json::json!([]);
    let f1 = fund_schreiben(&e, &mut v, &dat, "b");
    let f2 = fund_schreiben(&e, &mut v, &dat, "c");
    journal_schreiben(&e, &v);
    e.a.gefundene_aufnehmen(&dat, &o).unwrap();
    let f = e.a.fassungen(&dat).unwrap();
    assert_eq!(f.len(), 3);
    for id in [&f1, &f2] {
        let x = f.iter().find(|x| &x.fas == id).unwrap();
        assert_eq!(x.vorgaenger, None, "keine erfundene Kante");
        assert!(x.kopf, "eigener Kopf zur ausdrücklichen Wahl");
    }
    assert!(f.iter().find(|x| x.fas == kopf).unwrap().kopf);
    assert!(
        f.iter().all(|x| !x.aktuell),
        "Konflikt: niemand ist still aktuell"
    );
    // Neue Datei aus zwei unabhängigen Funden: zwei Köpfe, keine Kante.
    let neu = super::neue_id(super::PRAEFIX_DATEI);
    let mut v = journal_json(&e);
    let g1 = fund_schreiben(&e, &mut v, &neu, "x");
    let g2 = fund_schreiben(&e, &mut v, &neu, "y");
    journal_schreiben(&e, &v);
    e.a.gefundene_aufnehmen(&neu, &o).unwrap();
    let f = e.a.fassungen(&neu).unwrap();
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|x| x.vorgaenger.is_none() && x.kopf));
    assert!([g1, g2].iter().all(|g| f.iter().any(|x| &x.fas == g)));
    let s = e.sicht();
    assert!(s.dateien.iter().find(|d| d.id == neu).unwrap().konflikt);
}

#[test]
fn gefundene_aufnehmen_behaelt_vermerkte_vorgaengerin() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    let kopf = e.a.fassungen(&dat).unwrap()[0].fas.clone();
    let mut v = journal_json(&e);
    let f1 = fund_schreiben(&e, &mut v, &dat, "b");
    v["gefunden"][0]["vorgaenger"] = serde_json::json!(kopf);
    journal_schreiben(&e, &v);
    e.a.gefundene_aufnehmen(&dat, &o).unwrap();
    let f = e.a.fassungen(&dat).unwrap();
    let x = f.iter().find(|x| x.fas == f1).unwrap();
    assert_eq!(x.vorgaenger.as_deref(), Some(kopf.as_str()));
    assert!(x.aktuell);
}

#[test]
fn wiederhergestellte_freigabe_sperrt_ordner_ohne_sicherung() {
    let e = env();
    let root = e.standard();
    e.a.ki_setzen(&root, Some(KiStufe::Aus)).unwrap();
    e.a.ki_setzen(&root, Some(KiStufe::Aus)).unwrap();
    // Neuer Ordner oberster Ebene: in der `.vorher` gibt es keinen Eintrag für ihn.
    let neu = e.a.ordner_anlegen(None, "Neu").unwrap();
    fs::write(e.wurzel.join(freigabe::DATEI), b"kaputt").unwrap();
    let s = e.sicht();
    assert!(s.freigabe_wiederhergestellt);
    assert_eq!(s.freigabe_unklar, None);
    let ki = |s: &AblageSicht, id: &str| s.ordner.iter().find(|o| o.id == id).unwrap().ki.clone();
    assert_eq!(ki(&s, &neu).zustand, "gesperrt");
    assert_eq!(ki(&s, &root).zustand, "gesperrt");
    let l = freigabe::laden(&e.wurzel);
    let w = freigabe::wirksam(&l, std::slice::from_ref(&neu));
    assert!(!freigabe::global_erlaubt(&w) && !freigabe::projekt_erlaubt(&w));
    // Ausdrücklich neu gesetzt: gilt; Bestätigen hebt die Sperre der Wiederherstellung auf.
    e.a.ki_setzen(&neu, Some(KiStufe::An)).unwrap();
    assert_eq!(ki(&e.sicht(), &neu).zustand, "an");
    e.a.freigaben_neu_festlegen().unwrap();
    let s = e.sicht();
    assert!(!s.freigabe_wiederhergestellt);
    assert_eq!(
        ki(&s, &root).zustand,
        "gesperrt",
        "gesperrt bleibt erhalten"
    );
    assert_eq!(ki(&s, &neu).zustand, "an");
}

#[cfg(unix)]
#[test]
fn verknuepftes_dateien_wird_beim_loeschen_nie_gefolgt() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    // `dateien/` wird durch eine Verknüpfung auf einen fremden Ordner ersetzt, der eine gleichnamige
    // `dat` und eine liegengebliebene Zwischendatei enthält.
    let aussen = e._d.path().join("aussen-dateien");
    fs::rename(e.wurzel.join("dateien"), &aussen).unwrap();
    let teil = aussen
        .join(&dat)
        .join(format!(".teil-{}", super::neue_id(super::PRAEFIX_FASSUNG)));
    fs::write(&teil, "fremd").unwrap();
    std::os::unix::fs::symlink(&aussen, e.wurzel.join("dateien")).unwrap();
    let r = e.a.endgueltig_loeschen(AblageArt::Datei, &dat);
    assert_eq!(r.unwrap_err().code, "ablage.verknuepfung");
    assert!(aussen.join(&dat).exists(), "nichts außerhalb gelöscht");
    e.neustart();
    let s = e.sicht();
    assert_eq!(s.papierkorb.len(), 1, "Eintrag bleibt");
    assert!(teil.exists(), "Wartung folgt der Verknüpfung nicht");
    assert!(aussen.join(&dat).exists());
    assert_eq!(journal_json(&e)["journal"].as_array().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn verknuepfter_papierkorb_wird_beim_loeschen_nie_gefolgt() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    let aussen = e._d.path().join("aussen-korb");
    let rest = aussen.join("dat_x.loeschen-y");
    fs::create_dir_all(&rest).unwrap();
    fs::write(rest.join("f"), "fremd").unwrap();
    fs::remove_dir_all(e.wurzel.join("papierkorb")).unwrap();
    std::os::unix::fs::symlink(&aussen, e.wurzel.join("papierkorb")).unwrap();
    let r = e.a.endgueltig_loeschen(AblageArt::Datei, &dat);
    assert_eq!(r.unwrap_err().code, "ablage.verknuepfung");
    assert!(rest.join("f").exists());
    assert!(
        e.wurzel.join("dateien").join(&dat).exists(),
        "Original bleibt"
    );
}

/// Windows: `dateien/` als Junction (Reparse-Point). Läuft nur unter Windows (`mklink /J` braucht
/// keine Adminrechte); unter Unix deckt der Symlink-Test denselben Pfad ab.
#[cfg(windows)]
#[test]
fn junction_dateien_wird_beim_loeschen_nie_gefolgt() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    let aussen = e._d.path().join("aussen-dateien");
    fs::rename(e.wurzel.join("dateien"), &aussen).unwrap();
    let ok = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(e.wurzel.join("dateien"))
        .arg(&aussen)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        // Ohne `mklink` (z. B. eingeschränkte Umgebung) lässt sich keine Junction anlegen.
        return;
    }
    let r = e.a.endgueltig_loeschen(AblageArt::Datei, &dat);
    assert_eq!(r.unwrap_err().code, "ablage.verknuepfung");
    assert!(aussen.join(&dat).exists(), "nichts außerhalb gelöscht");
}

#[test]
fn lesefehler_beim_endgueltigen_loeschen_behaelt_auftrag_und_datei() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    let original = e.original(&dat);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    abbruch(Some(Schritt::LoeschenLesefehler));
    assert!(e.a.endgueltig_loeschen(AblageArt::Datei, &dat).is_err());
    // Neustart, Lesefehler besteht weiter: Auftrag offen, Eintrag und Original bleiben.
    e.neustart();
    let s = e.sicht();
    abbruch(None);
    assert_eq!(s.papierkorb.len(), 1, "Eintrag verschwindet nicht");
    assert!(original.exists(), "Original noch da");
    let j = journal_json(&e);
    assert_eq!(j["journal"].as_array().unwrap().len(), 1, "Auftrag offen");
    assert!(j["dateien"].get(&dat).is_some());
    // Lesefehler behoben: der Wiederanlauf führt das Löschen zu Ende.
    let s = e.sicht();
    assert!(s.papierkorb.is_empty());
    assert!(!original.exists());
    assert!(journal_json(&e)["journal"].as_array().unwrap().is_empty());
}

#[test]
fn offener_loeschauftrag_sperrt_verschieben_aus_dem_geloeschten_ordner() {
    let e = env();
    let root = e.standard();
    let v = e.a.ordner_anlegen(Some(&root), "Weg").unwrap();
    let ziel = e.a.ordner_anlegen(Some(&root), "Bleibt").unwrap();
    let dat = e.ablegen("a.txt", "a", &v);
    e.a.in_papierkorb(AblageArt::Ordner, &v).unwrap();
    abbruch(Some(Schritt::LoeschenNachJournal));
    assert!(e.a.endgueltig_loeschen(AblageArt::Ordner, &v).is_err());
    abbruch(None);
    // Ohne Wartung dazwischen: der Auftrag ist offen.
    let r = e.a.datei_verschieben(&dat, &ziel);
    assert_eq!(r.unwrap_err().code, "ablage.loeschen_offen");
    assert_eq!(
        e.a.datei_umbenennen(&dat, "b.txt").unwrap_err().code,
        "ablage.loeschen_offen"
    );
    assert_eq!(
        e.a.wiederherstellen(AblageArt::Ordner, &v)
            .unwrap_err()
            .code,
        "ablage.loeschen_offen"
    );
    assert_eq!(
        e.a.ordner_anlegen(Some(&v), "X").unwrap_err().code,
        "ablage.loeschen_offen"
    );
    let q = e.quelle("neu.txt", "n");
    let z = e.a.importieren(&q, &v, None).unwrap();
    assert_eq!(z[0].code.as_deref(), Some("ablage.loeschen_offen"));
    // Ohne offenen Auftrag: eine Datei im Papierkorb wird nicht verschoben.
    let d2 = e.ablegen("c.txt", "c", &ziel);
    e.a.in_papierkorb(AblageArt::Datei, &d2).unwrap();
    assert_eq!(
        e.a.datei_verschieben(&d2, &root).unwrap_err().code,
        "ablage.datei_im_papierkorb"
    );
    // Die Wartung führt den Auftrag zu Ende; im lebenden Ordner wurde nichts mitgelöscht.
    let s = e.sicht();
    assert!(s.ordner.iter().all(|o| o.id != v));
    assert!(s.ordner.iter().any(|o| o.id == ziel));
    assert!(!e.wurzel.join("dateien").join(&dat).exists());
    assert!(e.wurzel.join("dateien").join(&d2).exists());
}

#[test]
fn entfernfehler_am_zwischenort_behaelt_auftrag_bis_er_weg_ist() {
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    // rename gelingt, remove_dir_all scheitert: Original liegt nur noch im Zwischenort.
    abbruch(Some(Schritt::LoeschenEntfernfehler));
    assert!(e.a.endgueltig_loeschen(AblageArt::Datei, &dat).is_err());
    let zwischenorte = || -> Vec<String> {
        fs::read_dir(e.wurzel.join("papierkorb"))
            .map(|r| {
                r.flatten()
                    .map(|x| x.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(&dat))
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(!zwischenorte().is_empty(), "Zwischenort vorhanden");
    assert!(
        !e.original(&dat).exists(),
        "Quelle verschoben (NotFound beim Wiederanlauf)"
    );
    // Wiederanlauf, Entfernen scheitert erneut: Auftrag und Eintrag bleiben.
    e.neustart();
    let s = e.sicht();
    abbruch(None);
    assert_eq!(s.papierkorb.len(), 1, "Eintrag verschwindet nicht");
    assert_eq!(
        journal_json(&e)["journal"].as_array().unwrap().len(),
        1,
        "Auftrag offen"
    );
    assert!(!zwischenorte().is_empty(), "Zwischenort noch da");
    // Fehler behoben: der nächste Lauf räumt den Zwischenort und trägt erst dann aus.
    let s = e.sicht();
    assert!(s.papierkorb.is_empty());
    assert!(zwischenorte().is_empty());
    assert!(journal_json(&e)["journal"].as_array().unwrap().is_empty());
}

#[test]
fn geraetenamen_werden_erkannt() {
    use super::speicher::ist_geraetename;
    for n in [
        "CON",
        "con.txt",
        "NUL",
        "nul.tar.gz",
        "AUX.",
        "PRN ",
        "COM1",
        "lpt9.log",
        "CONIN$",
        "conout$.x",
    ] {
        assert!(ist_geraetename(n), "{n}");
    }
    for n in [
        "aux_.txt",
        "CONX",
        "COM0",
        "COM10",
        "LPT",
        "Konto.txt",
        "nulla.txt",
        "",
    ] {
        assert!(!ist_geraetename(n), "{n}");
    }
}

#[test]
fn nfc_und_nfd_quellen_kollidieren_ohne_ueberschreiben() {
    let e = env();
    let o = e.standard();
    let nfc = "\u{00c4}rger.txt";
    let nfd = "A\u{0308}rger.txt";
    let dat = e.ablegen(nfc, "erste", &o);
    let q = e.quelle(nfd, "zweite");
    let z = e.a.importieren(&q, &o, None).unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Kollision, "{:?}", z[0]);
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), "erste");
    assert_eq!(
        fs::read_to_string(&q).unwrap(),
        "zweite",
        "Quelle unverändert"
    );
}

/// Echte Windows-Dateisystemfälle (Codex, Windows-Tor): Dateien werden verbatim angelegt, der Import
/// läuft über `importieren`; geprüft werden Originalbytes, Anzeigename, Ziel `<fas>.<typ>` und die
/// unveränderte Quelle.
#[cfg(windows)]
mod windows_import {
    use super::*;

    fn verbatim(p: &Path) -> PathBuf {
        let abs = std::path::absolute(p).unwrap();
        let s = abs.to_string_lossy().into_owned();
        if s.starts_with(r"\\?\") {
            abs
        } else if let Some(rest) = s.strip_prefix(r"\\") {
            PathBuf::from(format!(r"\\?\UNC\{rest}"))
        } else {
            PathBuf::from(format!(r"\\?\{s}"))
        }
    }

    fn pruefen(e: &Env, o: &str, quelle: &Path, inhalt: &[u8], anzeige: &str) {
        let z = e.a.importieren(quelle, o, None).unwrap();
        assert_eq!(z.len(), 1, "{z:?}");
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        let dat = z[0].dat.clone().unwrap();
        let ziel = e.original(&dat);
        let zname = ziel.file_name().unwrap().to_string_lossy().into_owned();
        assert!(zname.starts_with("fas_"), "Ziel {zname}");
        assert_eq!(fs::read(&ziel).unwrap(), inhalt, "Originalbytes");
        assert_eq!(z[0].name.as_deref(), Some(anzeige), "Anzeigename");
        assert_eq!(fs::read(quelle).unwrap(), inhalt, "Quelle unverändert");
    }

    #[test]
    fn echte_dateien_mit_punkt_leerzeichen_geraetenamen_und_langem_pfad() {
        let e = env();
        let o = e.standard();
        let basis = verbatim(&e.quellen);

        let aux = basis.join("aux_.txt");
        fs::write(&aux, b"aux").unwrap();
        pruefen(&e, &o, &aux, b"aux", "aux_.txt");

        // Nachweislich echte Fixtures: nur verbatim anlegbar, per read_dir wiedergefunden.
        let punkt = basis.join("Datei.");
        fs::write(&punkt, b"punkt").unwrap();
        let leer = basis.join("Datei ");
        fs::write(&leer, b"leer").unwrap();
        let namen: Vec<String> = fs::read_dir(&basis)
            .unwrap()
            .flatten()
            .map(|x| x.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            namen.contains(&"Datei.".to_string()) && namen.contains(&"Datei ".to_string()),
            "{namen:?}"
        );
        let zp = e.a.importieren(&punkt, &o, None).unwrap();
        assert_eq!(zp[0].stand, AblageImportStand::Abgelegt, "{:?}", zp[0]);
        assert_eq!(
            fs::read(e.original(zp[0].dat.as_ref().unwrap())).unwrap(),
            b"punkt"
        );
        let zl = e.a.importieren(&leer, &o, None).unwrap();
        assert!(
            matches!(
                zl[0].stand,
                AblageImportStand::Abgelegt | AblageImportStand::Kollision
            ),
            "{:?}",
            zl[0]
        );
        if zl[0].stand == AblageImportStand::Abgelegt {
            assert_eq!(
                fs::read(e.original(zl[0].dat.as_ref().unwrap())).unwrap(),
                b"leer"
            );
        }
        assert_eq!(fs::read(&punkt).unwrap(), b"punkt");
        assert_eq!(fs::read(&leer).unwrap(), b"leer");

        // Echte Datei CON.txt (verbatim): wird übernommen, Anzeigename bereinigt.
        let con = basis.join("CON.txt");
        fs::write(&con, b"con").unwrap();
        pruefen(&e, &o, &con, b"con", "_CON.txt");

        // Langer, legaler Name mit Gesamtpfad > 260 Zeichen.
        let tief = basis.join("u".repeat(120));
        fs::create_dir_all(&tief).unwrap();
        let lang = tief.join(format!("{}.txt", "l".repeat(150)));
        assert!(lang.as_os_str().len() > 260);
        fs::write(&lang, b"lang").unwrap();
        let anzeige = format!("{}.txt", "l".repeat(150));
        let z = e.a.importieren(&lang, &o, None).unwrap();
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        assert_eq!(
            fs::read(e.original(z[0].dat.as_ref().unwrap())).unwrap(),
            b"lang"
        );
        assert!(z[0].name.as_deref().unwrap().starts_with(&anzeige[..100]));
        assert_eq!(fs::read(&lang).unwrap(), b"lang");

        // Ordnerimport: Kinder bleiben verbatim adressiert.
        let ord = basis.join("Ordner");
        fs::create_dir_all(&ord).unwrap();
        fs::write(ord.join("Bericht."), b"kind").unwrap();
        let z = e.a.importieren(&ord, &o, None).unwrap();
        assert!(
            z.iter().any(|r| r.stand == AblageImportStand::Abgelegt),
            "{z:?}"
        );
    }

    #[test]
    fn geraete_und_datenstroeme_werden_abgewiesen() {
        let e = env();
        let o = e.standard();
        for p in [
            PathBuf::from("NUL"),
            PathBuf::from(r"C:\NUL"),
            e.quellen.join("con.txt"),
            e.quellen.join("AUX"),
            e.quellen.join("nul.tar.gz"),
            PathBuf::from(r"\\.\NUL"),
            PathBuf::from(r"\\.\PhysicalDrive0"),
            PathBuf::from(r"\\?\GLOBALROOT\Device\Null"),
        ] {
            let z = e.a.importieren(&p, &o, None).unwrap();
            assert_eq!(z[0].stand, AblageImportStand::Uebersprungen, "{p:?}");
            assert_eq!(z[0].code.as_deref(), Some("ablage.geraet"), "{p:?}");
        }
        // Alternativer Datenstrom: abgewiesen, auch verbatim.
        let a = e.quellen.join("a.txt");
        fs::write(&a, b"haupt").unwrap();
        let strom = e.quellen.join("a.txt:geheim");
        fs::write(&strom, b"strom").unwrap();
        for p in [strom.clone(), verbatim(&e.quellen).join("a.txt:geheim")] {
            let z = e.a.importieren(&p, &o, None).unwrap();
            assert_eq!(z[0].code.as_deref(), Some("ablage.datenstrom"), "{p:?}");
        }
        assert!(e.sicht().dateien.is_empty(), "nichts übernommen");
    }
}

// --- Lamport-Zähler: Überlauf und Obergrenze (#450) ---------------------------------------------

fn zaehler_setzen(e: &Env, ordner: &str, z: u64) {
    let mut v = journal_json(e);
    v["ordner"][ordner]["name"]["zaehler"] = serde_json::json!(z);
    journal_schreiben(e, &v);
}

#[test]
fn zaehler_ueberlauf_beim_schreiben_ist_ein_fehler_und_schreibt_nichts() {
    use super::zusammenfuehren::naechster_zaehler;
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    let fas = e.a.fassungen(&dat).unwrap()[0].fas.clone();
    zaehler_setzen(&e, &o, u64::MAX);
    let b = e.a.sync_bestand().unwrap();
    assert_eq!(naechster_zaehler(&b.ordner, &b.dateien), None);
    let vorher = fs::read(e.wurzel.join(JOURNAL)).unwrap();
    let dateien_vorher = e.dateien_in(&dat);
    let code = |r: super::Res<()>| r.unwrap_err().code;
    let erschoepft = "ablage.zaehler_erschoepft";
    let neu = e.a.ordner_anlegen(Some(&o), "Neu").unwrap_err();
    assert_eq!(neu.code, erschoepft);
    assert!(!neu.message.contains("Neu") && !neu.message.contains("a.txt"));
    assert_eq!(code(e.a.datei_umbenennen(&dat, "b.txt")), erschoepft);
    assert_eq!(code(e.a.ordner_umbenennen(&o, "Anders")), erschoepft);
    assert_eq!(code(e.a.in_papierkorb(AblageArt::Datei, &dat)), erschoepft);
    assert_eq!(code(e.a.fassung_waehlen(&dat, &fas)), erschoepft);
    // Import: nichts landet auf der Platte, die Zeile trägt den Fehlercode.
    let q = e.quelle("zweite.txt", "zweite");
    let z = e.a.importieren(&q, &o, None).unwrap();
    assert_eq!(z[0].stand, AblageImportStand::Fehler);
    assert_eq!(z[0].code.as_deref(), Some(erschoepft));
    assert_eq!(fs::read(e.wurzel.join(JOURNAL)).unwrap(), vorher);
    assert_eq!(e.dateien_in(&dat), dateien_vorher);
    assert_eq!(
        fs::read_dir(e.wurzel.join("dateien")).unwrap().count(),
        1,
        "kein neues Dateiverzeichnis"
    );
}

#[test]
fn lokal_wird_nie_ueber_zaehler_max_vergeben() {
    use super::zusammenfuehren::{naechster_zaehler, ZAEHLER_MAX};
    let e = env();
    let o = e.standard();
    zaehler_setzen(&e, &o, ZAEHLER_MAX - 2);
    e.a.ordner_anlegen(Some(&o), "Eins").unwrap(); // MAX - 1
    e.a.ordner_anlegen(Some(&o), "Zwei").unwrap(); // MAX
    let b = e.a.sync_bestand().unwrap();
    assert_eq!(
        b.ordner.values().map(|o| o.name.zaehler).max(),
        Some(ZAEHLER_MAX)
    );
    assert_eq!(naechster_zaehler(&b.ordner, &b.dateien), None);
    let f = e.a.ordner_anlegen(Some(&o), "Drei").unwrap_err();
    assert_eq!(f.code, "ablage.zaehler_erschoepft");
    let namen: Vec<String> = e.sicht().ordner.iter().map(|o| o.name.clone()).collect();
    assert!(namen.contains(&"Zwei".to_string()) && !namen.contains(&"Drei".to_string()));
}

#[test]
fn wiederanlauf_haelt_den_journaleintrag_bei_erschoepftem_zaehler() {
    let e = env();
    let o = abbruch_import(&e, Schritt::NachJournal);
    assert_eq!(journal_json(&e)["journal"].as_array().unwrap().len(), 1);
    let dat = alle_dateien_unter_dirs(&e.wurzel.join("dateien"));
    zaehler_setzen(&e, &o, u64::MAX);
    // Wartung scheitert am Zähler: Eintrag bleibt im Journal, das Original bleibt liegen, kein Fund.
    let s = e.sicht();
    assert!(s.dateien.is_empty() && s.gefunden.is_empty());
    assert_eq!(journal_json(&e)["journal"].as_array().unwrap().len(), 1);
    assert_eq!(e.dateien_in(&dat).len(), 1);
    let original = e
        .wurzel
        .join("dateien")
        .join(&dat)
        .join(&e.dateien_in(&dat)[0]);
    assert_eq!(fs::read_to_string(&original).unwrap(), GEHEIM);
    // Mit freiem Zähler trägt der nächste Lauf nach.
    zaehler_setzen(&e, &o, 1);
    let s = e.sicht();
    assert_eq!(namen(&s), vec!["Angebot.pdf".to_string()]);
    assert!(journal_json(&e)["journal"].as_array().unwrap().is_empty());
    assert_eq!(fs::read_to_string(e.original(&dat)).unwrap(), GEHEIM);
}

#[test]
fn sync_nimmt_zaehler_genau_an_der_obergrenze_an_und_verwirft_darueber() {
    use super::speicher::Eingang;
    use super::zusammenfuehren::ZAEHLER_MAX;
    let e = env();
    let o = e.standard();
    let dat = e.ablegen("a.txt", "a", &o);
    let b = e.a.sync_bestand().unwrap();
    let ordner = |z: u64, name: &str| {
        let mut x = b.ordner[&o].clone();
        x.name.wert = name.into();
        x.name.zaehler = z;
        Eingang::Ordner(x)
    };
    let datei = |z: u64, name: &str| {
        let mut x = b.dateien[&dat].clone();
        x.name.wert = name.into();
        x.name.zaehler = z;
        Eingang::Datei(x)
    };
    // Über der Grenze: verworfen, nichts geändert, keine Panik.
    let r =
        e.a.sync_einspielen(&[
            ordner(ZAEHLER_MAX + 1, "Zu hoch"),
            datei(u64::MAX, "zu hoch.txt"),
        ])
        .unwrap();
    assert_eq!(r.verworfen, vec![o.clone(), dat.clone()]);
    assert!(r.geaendert.is_empty());
    let s = e.sicht();
    assert_ne!(s.ordner[0].name, "Zu hoch");
    assert_eq!(namen(&s), vec!["a.txt".to_string()]);
    // Genau auf der Grenze: gültig. Die Kette ist dann ausgeschöpft: lokal gibt es nie mehr als
    // `ZAEHLER_MAX`, Schreibvorgänge scheitern sichtbar und ändern nichts.
    let r =
        e.a.sync_einspielen(&[
            ordner(ZAEHLER_MAX, "An der Grenze"),
            datei(ZAEHLER_MAX, "grenze.txt"),
        ])
        .unwrap();
    assert!(r.verworfen.is_empty(), "{r:?}");
    assert_eq!(r.geaendert, vec![o.clone(), dat.clone()]);
    assert_eq!(e.sicht().ordner[0].name, "An der Grenze");
    let vorher = fs::read(e.wurzel.join(JOURNAL)).unwrap();
    let f = e.a.datei_umbenennen(&dat, "lokal.txt").unwrap_err();
    assert_eq!(f.code, "ablage.zaehler_erschoepft");
    assert_eq!(fs::read(e.wurzel.join(JOURNAL)).unwrap(), vorher);
    assert_eq!(namen(&e.sicht()), vec!["grenze.txt".to_string()]);
    let b = e.a.sync_bestand().unwrap();
    assert_eq!(b.dateien[&dat].name.zaehler, ZAEHLER_MAX);
    // Eine Wahl über der Grenze verwirft die Datei ebenfalls.
    let mut x = b.dateien[&dat].clone();
    x.wahlen.push(super::zusammenfuehren::Wahl {
        zaehler: ZAEHLER_MAX + 1,
        ..super::zusammenfuehren::neue_wahl(&x, "wahl_x", &x.fassungen[0].fas, 1, "g_t")
    });
    let r = e.a.sync_einspielen(&[Eingang::Datei(x)]).unwrap();
    assert_eq!(r.verworfen, vec![dat]);
}
