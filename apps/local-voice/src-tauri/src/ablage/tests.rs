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
