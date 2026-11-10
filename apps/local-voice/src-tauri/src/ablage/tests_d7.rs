//! D7 (Spec §8): verknüpfte Ordner und lokale OCR. Nur synthetische Daten. Der Unterprozess ist wie in D2
//! das Testprogramm selbst ([`super::tests_d2::kind`]); die OCR liefert in Tests ein Test-Leser
//! (`LVA_EXTRAKT_TEST=ocr`, Kennung [`ocr::TEST_KENNUNG`]).

use super::eintrag;
use super::freigabe;
use super::freigabe::KiStufe;
use super::index;
use super::ki::{self, KiBereich, Urteil, ZielAnbieter};
use super::ocr;
use super::semantik::AblageEinbettung;
use super::speicher::{Ablage, AblageEinstellungen};
use super::tests_d2::{kind, pdf_aus_objekten, TEST_GRENZEN};
use super::textfassung;
use super::verknuepft::{self, Art, Auftrag, Ausgang, Entpreller};
use super::zusammenfuehren::Klasse;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const KONTROLLE: &str = "Zebrafink";

struct Env {
    d: tempfile::TempDir,
    a: Ablage,
    /// Der Ordner „draußen“, der verknüpft wird (nicht im Datenordner der App).
    quelle: PathBuf,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let a = Ablage::neu(d.path().join("daten").join("ablage"));
    assert_eq!(a.wartung(), None);
    let quelle = d.path().join("Dokumente");
    fs::create_dir_all(&quelle).unwrap();
    // Kanonisch (macOS: /var → /private/var), damit der Pfad keine Verknüpfung enthält.
    let quelle = fs::canonicalize(quelle).unwrap();
    Env { d, a, quelle }
}

impl Env {
    fn schreiben(&self, rel: &str, inhalt: &[u8]) -> PathBuf {
        let p = self.quelle.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, inhalt).unwrap();
        p
    }

    fn verknuepfen(&self) -> String {
        let id = verknuepft::verknuepfen(&self.a, &self.quelle).unwrap();
        verknuepft::abtasten(&self.a, &id).unwrap();
        id
    }

    fn lauf(&self, ocr: Option<ocr::Kennung>, test: Option<&str>) -> verknuepft::Lauf {
        verknuepft::rueckstellungen_vergessen();
        let l = verknuepft::lauf(
            &self.a,
            &kind(test),
            TEST_GRENZEN,
            &AtomicBool::new(false),
            &|| {},
            ocr,
        )
        .unwrap();
        index::abgleichen(&self.a).unwrap();
        l
    }

    fn rel_dat(&self, id: &str, rel: &str) -> String {
        verknuepft::bestand(self.a.dir(), id).dateien[rel]
            .dat
            .clone()
    }

    fn treffer(&self, q: &str) -> Vec<index::AblageSuchtreffer> {
        index::suchen(&self.a, q).unwrap()
    }

    fn zwischen_leer(&self) -> bool {
        fs::read_dir(self.a.dir().join("verknuepft").join("zwischen"))
            .map(|r| r.count() == 0)
            .unwrap_or(true)
    }
}

fn code(e: super::Fehler) -> String {
    e.code
}

// ---------------------------------------------------------------------------
// Verknüpfen
// ---------------------------------------------------------------------------

#[test]
fn verknuepfen_prueft_den_ordner() {
    let e = env();
    let datei = e.schreiben("brief.txt", b"x");
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, &datei).unwrap_err()),
        "ablage.kein_ordner"
    );
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, &e.d.path().join("fehlt")).unwrap_err()),
        "ablage.nicht_erreichbar"
    );
    // Datenordner der App (und alles darin) wird nie verknüpft.
    fs::create_dir_all(e.a.dir()).unwrap();
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, e.a.dir()).unwrap_err()),
        "ablage.verknuepft_datenordner"
    );
    #[cfg(unix)]
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, Path::new("/")).unwrap_err()),
        "ablage.verknuepft_wurzel"
    );
    let id = verknuepft::verknuepfen(&e.a, &e.quelle).unwrap();
    assert!(super::id_gueltig(super::PRAEFIX_ORDNER, &id));
    let l = verknuepft::liste(e.a.dir()).unwrap();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].klasse, "confidential", "Vorgabe der Klasse");
    assert_eq!(l[0].name, "Dokumente");
    // Doppelt, darin, darüber: abgewiesen.
    fs::create_dir_all(e.quelle.join("Unter")).unwrap();
    for p in [
        e.quelle.clone(),
        e.quelle.join("Unter"),
        e.d.path().to_path_buf(),
    ] {
        let p = fs::canonicalize(p).unwrap();
        let c = code(verknuepft::verknuepfen(&e.a, &p).unwrap_err());
        assert!(
            c == "ablage.verknuepft_doppelt" || c == "ablage.verknuepft_datenordner",
            "{c}"
        );
    }
}

#[cfg(unix)]
#[test]
fn verknuepfung_als_quelle_oder_im_pfad_wird_abgewiesen() {
    let e = env();
    let link = e.d.path().join("Link");
    std::os::unix::fs::symlink(&e.quelle, &link).unwrap();
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, &link).unwrap_err()),
        "ablage.verknuepfung"
    );
    fs::create_dir_all(e.quelle.join("Unter")).unwrap();
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, &link.join("Unter")).unwrap_err()),
        "ablage.verknuepfung",
        "Verknüpfung weiter oben im Pfad"
    );
    // Später durch eine Verknüpfung ersetzt: nicht mehr erreichbar, nichts gelesen.
    let id = e.verknuepfen();
    let woanders = e.d.path().join("Woanders");
    fs::create_dir_all(&woanders).unwrap();
    fs::write(woanders.join("fremd.txt"), KONTROLLE).unwrap();
    fs::rename(&e.quelle, e.d.path().join("weg")).unwrap();
    std::os::unix::fs::symlink(&woanders, &e.quelle).unwrap();
    verknuepft::abtasten(&e.a, &id).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(b.nicht_erreichbar);
    assert!(b.dateien.is_empty());
}

// ---------------------------------------------------------------------------
// Abtasten
// ---------------------------------------------------------------------------

#[test]
fn abtasten_mit_ausschluessen() {
    let e = env();
    e.schreiben("Bericht.txt", b"a");
    e.schreiben("Akte/Notiz.md", b"b");
    e.schreiben("~$Bericht.docx", b"Sperre");
    e.schreiben("export.TMP", b"t");
    e.schreiben(".versteckt", b"v");
    e.schreiben(".git/config", b"g");
    e.schreiben("Thumbs.db", b"t");
    e.schreiben("desktop.ini", b"d");
    e.schreiben("$RECYCLE.BIN/alt.txt", b"r");
    e.schreiben("System Volume Information/x.txt", b"s");
    let id = e.verknuepfen();
    let b = verknuepft::bestand(e.a.dir(), &id);
    let rels: Vec<&str> = b.dateien.keys().map(String::as_str).collect();
    assert_eq!(rels, ["Akte/Notiz.md", "Bericht.txt"]);
    assert_eq!(b.ausgelassen, 0);
    assert_eq!(b.dateien["Bericht.txt"].stand, "ausstehend");
    assert!(verknuepft::ausgeschlossen("~$x.xlsx", false));
    assert!(verknuepft::ausgeschlossen("a.tmp", false));
    assert!(!verknuepft::ausgeschlossen("tmp.txt", false));
}

#[cfg(unix)]
#[test]
fn abtasten_folgt_keinen_verknuepfungen_und_weist_hartlinks_ab() {
    let e = env();
    let draussen = e.d.path().join("draussen");
    fs::create_dir_all(&draussen).unwrap();
    fs::write(draussen.join("geheim.txt"), KONTROLLE).unwrap();
    e.schreiben("echt.txt", b"echt");
    std::os::unix::fs::symlink(draussen.join("geheim.txt"), e.quelle.join("datei-link.txt"))
        .unwrap();
    std::os::unix::fs::symlink(&draussen, e.quelle.join("ordner-link")).unwrap();
    fs::hard_link(draussen.join("geheim.txt"), e.quelle.join("hart.txt")).unwrap();
    let id = e.verknuepfen();
    let b = verknuepft::bestand(e.a.dir(), &id);
    let rels: Vec<&str> = b.dateien.keys().map(String::as_str).collect();
    assert_eq!(rels, ["echt.txt"]);
    assert_eq!(b.ausgelassen, 3);
    e.lauf(None, None);
    assert!(e.treffer(KONTROLLE).is_empty());
}

#[test]
fn grenzen_tiefe() {
    let e = env();
    let mut rel = String::new();
    for i in 0..verknuepft::MAX_TIEFE + 2 {
        rel.push_str(&format!("t{i}/"));
    }
    e.schreiben(&format!("{rel}tief.txt"), b"x");
    e.schreiben("flach.txt", b"x");
    let id = e.verknuepfen();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!(b.dateien.len(), 1);
    assert_eq!(b.ausgelassen, 1);
}

// ---------------------------------------------------------------------------
// Text, Index, Suche, Klasse, Freigabe
// ---------------------------------------------------------------------------

#[test]
fn text_index_suche_klasse_und_freigabe() {
    let e = env();
    e.schreiben(
        "Verträge/Wartung.txt",
        format!("Laufzeit drei Jahre. Kontrollwort {KONTROLLE}.").as_bytes(),
    );
    e.schreiben("Bild.png", b"\x89PNG ohne OCR");
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "Verträge/Wartung.txt");
    assert!(verknuepft::offen(&e.a, None));
    let l = e.lauf(None, None);
    assert_eq!(l.ok, 1);
    assert!(e.zwischen_leer(), "Momentkopie entfernt");
    assert!(
        !verknuepft::offen(&e.a, None),
        "Bild ohne OCR: kein Auftrag"
    );
    let b = verknuepft::bestand(e.a.dir(), &id);
    let ein = &b.dateien["Verträge/Wartung.txt"];
    assert_eq!(ein.stand, "ok");
    assert!(verknuepft::text_pfad(e.a.dir(), &dat, ein.textrev.as_ref().unwrap()).is_file());
    // Suche: Treffer mit Pfad im verknüpften Ordner als Name, Ordner = verknüpfter Ordner.
    let t = e.treffer(KONTROLLE);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].dat, dat);
    assert_eq!(t[0].name, "Verträge/Wartung.txt");
    assert_eq!(t[0].ordner, id);
    assert_eq!(t[0].stellen[0].anker, "zeile=1");
    // Index-Felder: Klasse confidential, nur dieses Gerät, KI aus (Vorgabe).
    let q: Vec<_> = verknuepft::index_quellen(&e.a).unwrap();
    let q = q.iter().find(|q| q.dat == dat).unwrap();
    assert_eq!(
        (q.klasse.as_str(), q.nur_geraet, q.ki.as_str(), q.verknuepft),
        ("confidential", true, "aus", true)
    );
    assert_eq!(q.ordnerkette, vec![id.clone()]);
    // Sicht: eigener Bereich, Dateien nicht unter den eigenen.
    let s = e.a.ansicht().unwrap();
    assert!(s.dateien.is_empty());
    assert_eq!(s.verknuepft.len(), 1);
    let v = &s.verknuepft[0];
    assert!(v.ordner.verknuepft && v.erreichbar && v.ordner.nur_geraet_wirksam);
    assert_eq!(v.ordner.dateien, 2);
    let d = s.verknuepft_dateien.iter().find(|d| d.id == dat).unwrap();
    assert!(d.verknuepft && d.nur_geraet);
    assert_eq!(d.text.as_deref(), Some("ok"));
    let bild = s
        .verknuepft_dateien
        .iter()
        .find(|d| d.name == "Bild.png")
        .unwrap();
    assert_eq!(bild.text, None, "ohne OCR kein Text für Bilder");
    // Freigabe und Klasse wie bei Ordnern.
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    e.a.ordner_einstellungen(
        &id,
        &AblageEinstellungen {
            datenklasse: Some("personal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let s = e.a.ansicht().unwrap();
    assert_eq!(s.verknuepft[0].ordner.ki.zustand, "an");
    assert_eq!(s.verknuepft[0].ordner.klasse, "personal");
    let q = verknuepft::index_quellen(&e.a).unwrap();
    assert!(q.iter().all(|q| q.klasse == "personal" && q.ki == "an"));
    // Nur lesen: „nur dieses Gerät“ lässt sich nicht abschalten, keine KI-Entwürfe, kein Ablegen.
    for e2 in [
        AblageEinstellungen {
            nur_geraet: Some(false),
            ..Default::default()
        },
        AblageEinstellungen {
            zielordner_ki: Some(true),
            ..Default::default()
        },
    ] {
        assert_eq!(
            code(e.a.ordner_einstellungen(&id, &e2).unwrap_err()),
            "ablage.verknuepft_nur_lesen"
        );
    }
    let neu = e.d.path().join("neu.txt");
    fs::write(&neu, b"neu").unwrap();
    let z = e.a.importieren(&neu, &id, None).unwrap();
    assert_eq!(z[0].stand, super::speicher::AblageImportStand::Fehler);
    assert!(e
        .a
        .in_papierkorb(super::speicher::AblageArt::Datei, &dat)
        .is_err());
    // Die Quelle ist unverändert.
    assert!(fs::read_to_string(e.quelle.join("Verträge/Wartung.txt"))
        .unwrap()
        .contains(KONTROLLE));
}

#[test]
fn aenderung_loeschen_und_loesen() {
    let e = env();
    let p = e.schreiben("a.txt", format!("alt {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "a.txt");
    e.lauf(None, None);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    // Geändert: gleiche Kennung, neuer Text.
    fs::write(&p, b"neu Eisvogel und noch mehr").unwrap();
    assert!(verknuepft::abtasten(&e.a, &id).unwrap(), "Größe geändert");
    assert_eq!(e.rel_dat(&id, "a.txt"), dat);
    e.lauf(None, None);
    assert!(e.treffer(KONTROLLE).is_empty());
    assert_eq!(e.treffer("Eisvogel").len(), 1);
    // Gelöscht: Eintrag, Text und Treffer weg.
    fs::remove_file(&p).unwrap();
    assert!(verknuepft::abtasten(&e.a, &id).unwrap());
    index::abgleichen(&e.a).unwrap();
    assert!(e.treffer("Eisvogel").is_empty());
    assert!(!e.a.dir().join("verknuepft/text").join(&dat).exists());
    // Lösen: Bestand, Texte, Index weg; die Quelle bleibt.
    e.schreiben("b.txt", format!("bleibt {KONTROLLE}").as_bytes());
    verknuepft::abtasten(&e.a, &id).unwrap();
    e.lauf(None, None);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    verknuepft::loesen(&e.a, &id).unwrap();
    index::abgleichen(&e.a).unwrap();
    assert!(e.treffer(KONTROLLE).is_empty());
    assert!(verknuepft::liste(e.a.dir()).unwrap().is_empty());
    assert!(!e.a.dir().join("verknuepft").join(&id).exists());
    assert!(fs::read_dir(e.a.dir().join("verknuepft/text"))
        .map(|r| r.count() == 0)
        .unwrap_or(true));
    assert!(e.quelle.join("b.txt").is_file());
    let f = fs::read_to_string(e.a.dir().join("freigabe.json")).unwrap();
    assert!(!f.contains(&id), "Freigabe-Eintrag vergessen");
}

#[test]
fn momentkopie_bricht_bei_aenderung_waehrend_des_lesens_ab() {
    let e = env();
    e.schreiben(
        "laufend.txt",
        format!("erster Stand {KONTROLLE}").as_bytes(),
    );
    let id = e.verknuepfen();
    let auf: Auftrag = verknuepft::auftraege(&e.a, None)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(auf.art, Art::Text);
    verknuepft::tests_haken::NACH_KOPIE.with(|h| {
        *h.borrow_mut() = Some(Box::new(|p: &Path| {
            let mut f = fs::OpenOptions::new().append(true).open(p).unwrap();
            std::io::Write::write_all(&mut f, b" und mehr").unwrap();
        }));
    });
    let r = verknuepft::bearbeiten(
        &e.a,
        &auf,
        &kind(None),
        TEST_GRENZEN,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    verknuepft::tests_haken::NACH_KOPIE.with(|h| *h.borrow_mut() = None);
    assert_eq!(r, Ausgang::Geaendert);
    assert!(e.zwischen_leer());
    assert_eq!(
        verknuepft::bestand(e.a.dir(), &id).dateien["laufend.txt"].stand,
        "ausstehend"
    );
    // Neu abgetastet: der neue Stand wird gelesen.
    verknuepft::abtasten(&e.a, &id).unwrap();
    e.lauf(None, None);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
}

// ---------------------------------------------------------------------------
// OCR
// ---------------------------------------------------------------------------

/// PDF mit einer leeren Seite (Bild-PDF ohne Textschicht); der Test-Leser „sieht“ `OCRTEXT:` in einem
/// unbenutzten Objekt.
fn bild_pdf(text: &str) -> Vec<u8> {
    pdf_aus_objekten(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Length 0 >>\nstream\n\nendstream".to_vec(),
        format!("(OCRTEXT: {text})").into_bytes(),
    ])
}

#[test]
fn ocr_fuer_bild_pdf_und_bild() {
    let e = env();
    e.schreiben(
        "Scan.pdf",
        &bild_pdf(&format!("Gescannter Vertrag {KONTROLLE}")),
    );
    e.schreiben("Tafel.png", b"\x89PNG OCRTEXT: Whiteboard Eisvogel)");
    let id = e.verknuepfen();
    // Ohne OCR auf der Plattform: das PDF bleibt ohne Text, das Bild ohne Auftrag.
    let l = e.lauf(None, None);
    assert_eq!((l.ok, l.kein_text), (0, 1));
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!(b.dateien["Scan.pdf"].stand, "kein_text");
    assert_eq!(b.dateien["Tafel.png"].stand, "ausstehend");
    assert!(e.treffer(KONTROLLE).is_empty());
    // Mit OCR: Nachlauf für das Bild-PDF und das Bild.
    let k = Some(ocr::TEST_KENNUNG);
    let auf = verknuepft::auftraege(&e.a, k).unwrap();
    assert_eq!(auf.len(), 2);
    assert!(auf.iter().all(|a| a.art == Art::Ocr));
    let l = e.lauf(k, Some("ocr"));
    assert_eq!(l.ok, 2, "{l:?}");
    let t = e.treffer(KONTROLLE);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].name, "Scan.pdf");
    assert_eq!(t[0].stellen[0].anker, "seite=1");
    assert_eq!(e.treffer("Eisvogel")[0].name, "Tafel.png");
    let b = verknuepft::bestand(e.a.dir(), &id);
    let scan = &b.dateien["Scan.pdf"];
    assert_eq!(scan.ocr.as_deref(), Some("test-ocr@1"));
    assert_eq!(
        scan.textrev.as_deref(),
        Some(ocr::textrev(scan.sha256.as_ref().unwrap(), ocr::TEST_KENNUNG).as_str())
    );
    // Gelesen: kein weiterer Auftrag derselben OCR.
    assert!(verknuepft::auftraege(&e.a, k).unwrap().is_empty());
    assert!(e.zwischen_leer());
}

#[test]
fn ocr_im_unterprozess_prueft_typ_und_pruefsumme() {
    use super::textlauf;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("bild.png");
    fs::write(&p, b"OCRTEXT: Hallo Welt)").unwrap();
    let sha = {
        use sha2::Digest;
        super::extrakt::hex(&sha2::Sha256::digest(fs::read(&p).unwrap()))
    };
    let ab = AtomicBool::new(false);
    let t = textlauf::ausfuehren(
        &kind(Some("ocr")),
        d.path(),
        &p,
        &sha,
        "ocr:png",
        TEST_GRENZEN,
        &ab,
    )
    .unwrap();
    assert_eq!(t.extraktor, "test-ocr");
    assert_eq!(t.abschnitte[0].text, "Hallo Welt");
    assert_eq!(
        textlauf::ausfuehren(
            &kind(Some("ocr")),
            d.path(),
            &p,
            &"0".repeat(64),
            "ocr:png",
            TEST_GRENZEN,
            &ab
        )
        .unwrap_err(),
        "extrakt.pruefsumme"
    );
    assert_eq!(
        textlauf::ausfuehren(
            &kind(Some("ocr")),
            d.path(),
            &p,
            &sha,
            "ocr:docx",
            TEST_GRENZEN,
            &ab
        )
        .unwrap_err(),
        "extrakt.format"
    );
    // Ohne OCR auf dieser Plattform (Linux/macOS, kein Test-Leser): sauber „nicht verfügbar“.
    #[cfg(not(windows))]
    assert_eq!(
        textlauf::ausfuehren(
            &kind(None),
            d.path(),
            &p,
            &sha,
            "ocr:png",
            TEST_GRENZEN,
            &ab
        )
        .unwrap_err(),
        "extrakt.ocr_nicht_verfuegbar"
    );
    // Test-Naht: in Tests meldet die Plattform nie eine OCR (auch unter Windows), sonst hinge die Sicht
    // („ausstehend“ für Bilder) vom Betriebssystem ab.
    assert_eq!(ocr::extraktor(), None);
}

// ---------------------------------------------------------------------------
// Dateibeobachter und Entprellung
// ---------------------------------------------------------------------------

#[test]
fn entpreller_uebernimmt_erst_nach_der_ruhezeit() {
    let mut e = Entpreller::neu(Some("a".into()), Duration::from_secs(3));
    assert!(!e.melden("a", 0), "unverändert");
    assert!(!e.melden("b", 0), "neu: erst warten");
    assert!(e.wartet());
    assert!(!e.melden("b", 2_000), "Ruhezeit nicht vorbei");
    assert!(!e.melden("c", 2_500), "wieder geändert: neu warten");
    assert!(!e.melden("c", 5_000));
    assert!(e.melden("c", 5_500));
    assert!(!e.wartet());
    assert!(!e.melden("c", 9_000), "schon übernommen");
    // Zurück auf den alten Stand während des Wartens: nichts zu tun.
    assert!(!e.melden("d", 10_000));
    assert!(!e.melden("c", 11_000));
    assert!(!e.wartet());
}

#[test]
fn beobachter_entprellt_und_uebernimmt() {
    let e = env();
    e.schreiben("eins.txt", b"1");
    let id = e.verknuepfen();
    let ruhe = Duration::from_secs(3);
    let mut ep = std::collections::BTreeMap::new();
    assert_eq!(
        verknuepft::beobachten(&e.a, &mut ep, 0, ruhe),
        (false, false)
    );
    e.schreiben("zwei.txt", b"2");
    assert_eq!(
        verknuepft::beobachten(&e.a, &mut ep, 1_000, ruhe),
        (false, true),
        "Änderung gesehen, wartet"
    );
    assert_eq!(verknuepft::bestand(e.a.dir(), &id).dateien.len(), 1);
    e.schreiben("drei.txt", b"3");
    assert_eq!(
        verknuepft::beobachten(&e.a, &mut ep, 3_000, ruhe),
        (false, true),
        "weitere Änderung: Ruhezeit beginnt neu"
    );
    assert_eq!(
        verknuepft::beobachten(&e.a, &mut ep, 6_500, ruhe),
        (true, false)
    );
    assert_eq!(verknuepft::bestand(e.a.dir(), &id).dateien.len(), 3);
    // Ordner weg: nicht erreichbar, Bestand bleibt bis zur Rückkehr.
    let weg = e.d.path().join("weg");
    fs::rename(&e.quelle, &weg).unwrap();
    assert!(verknuepft::beobachten(&e.a, &mut ep, 7_000, ruhe).0);
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(b.nicht_erreichbar && b.dateien.len() == 3);
    assert!(verknuepft::auftraege(&e.a, None).unwrap().is_empty());
    assert!(!e.a.ansicht().unwrap().verknuepft[0].erreichbar);
}

// ---------------------------------------------------------------------------
// Getrennt von der eigenen Ablage
// ---------------------------------------------------------------------------

#[test]
fn eigene_ablage_wartung_und_sync_sehen_nichts() {
    let e = env();
    e.schreiben("geheim.txt", format!("nur hier {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    e.lauf(None, None);
    let dat = e.rel_dat(&id, "geheim.txt");
    // Wartung räumt nichts weg und meldet keine Funde.
    e.a.prozess_neustart();
    assert_eq!(e.a.wartung(), None);
    let s = e.a.ansicht().unwrap();
    assert!(s.gefunden.is_empty());
    assert_eq!(s.verknuepft_dateien.len(), 1);
    // Der Bestand des Abgleichs kennt weder Ordner noch Datei noch Text.
    let b = e.a.sync_bestand().unwrap();
    assert!(!b.ordner.contains_key(&id));
    assert!(!b.dateien.contains_key(&dat));
    assert!(b.texte.iter().all(|(d, _)| d != &dat));
    assert!(b.originale.iter().all(|(d, _)| d != &dat));
    // Der vollständige Textlauf der Ablage nimmt die verknüpften Ordner mit (zweite Phase).
    e.schreiben("neu.txt", b"Kiebitz im Feld");
    verknuepft::abtasten(&e.a, &id).unwrap();
    verknuepft::rueckstellungen_vergessen();
    let l = textfassung::lauf(
        &e.a,
        &kind(None),
        TEST_GRENZEN,
        &AtomicBool::new(false),
        &|| {},
    )
    .unwrap();
    assert_eq!(l.ok, 1);
    assert_eq!(e.treffer("Kiebitz").len(), 1);
    // „Freigaben neu festlegen“ nimmt den verknüpften Ordner mit.
    fs::write(e.a.dir().join("freigabe.json"), b"kaputt").unwrap();
    let _ = fs::remove_file(e.a.dir().join("freigabe.json.vorher"));
    e.a.freigaben_neu_festlegen().unwrap();
    let f = fs::read_to_string(e.a.dir().join("freigabe.json")).unwrap();
    assert!(f.contains(&id));
}

// ---------------------------------------------------------------------------
// Fix-Runde: Pfadglieder, Zwischenordner, OCR-Wiederholung, Unterordner, Reparse-Tags
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn momentkopie_prueft_jedes_pfadglied_nicht_nur_das_letzte() {
    let e = env();
    e.schreiben(
        "sub/innen/brief.txt",
        format!("vertraulich {KONTROLLE}").as_bytes(),
    );
    e.verknuepfen();
    let auf = verknuepft::auftraege(&e.a, None)
        .unwrap()
        .into_iter()
        .find(|a| a.rel == "sub/innen/brief.txt")
        .unwrap();
    // Nach dem Abtasten: das Zwischenverzeichnis wird durch eine Verknüpfung auf einen anderen Ordner
    // ersetzt, in dem eine gleich große Datei mit gleicher Änderungszeit liegt.
    let draussen = e.d.path().join("draussen");
    fs::create_dir_all(draussen.join("innen")).unwrap();
    fs::copy(
        e.quelle.join("sub/innen/brief.txt"),
        draussen.join("innen/brief.txt"),
    )
    .unwrap();
    fs::remove_dir_all(e.quelle.join("sub")).unwrap();
    std::os::unix::fs::symlink(&draussen, e.quelle.join("sub")).unwrap();
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "ablage.verknuepfung"
    );
    assert!(e.zwischen_leer(), "nichts gelesen, keine Kopie");
    // Der Auftrag wird abgewiesen eingetragen (nie gelesen).
    let r = verknuepft::bearbeiten(
        &e.a,
        &auf,
        &kind(None),
        TEST_GRENZEN,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert_eq!(r, Ausgang::Eingetragen("abgewiesen".into()));
    assert!(e.treffer(KONTROLLE).is_empty());
}

#[cfg(unix)]
#[test]
fn momentkopie_weist_verknuepfte_wurzel_ab() {
    let e = env();
    e.schreiben("a.txt", b"inhalt");
    e.verknuepfen();
    let auf = verknuepft::auftraege(&e.a, None).unwrap().remove(0);
    let anders = e.d.path().join("anders");
    fs::create_dir_all(&anders).unwrap();
    fs::copy(e.quelle.join("a.txt"), anders.join("a.txt")).unwrap();
    fs::remove_dir_all(&e.quelle).unwrap();
    std::os::unix::fs::symlink(&anders, &e.quelle).unwrap();
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "ablage.verknuepfung"
    );
    assert!(e.zwischen_leer());
}

#[test]
fn auftraege_weisen_unsaubere_relative_pfade_ab() {
    let e = env();
    e.schreiben("gut.txt", b"gut");
    let id = e.verknuepfen();
    let p = e.a.dir().join("verknuepft").join(&id).join("bestand.json");
    let mut b: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    let gut = b["dateien"]["gut.txt"].clone();
    let schlecht = [
        "../aussen.txt",
        "a/../b.txt",
        "./x.txt",
        "/abs.txt",
        "",
        "a//b.txt",
        "a/",
    ];
    for rel in schlecht {
        b["dateien"][rel] = gut.clone();
    }
    fs::write(&p, serde_json::to_vec(&b).unwrap()).unwrap();
    let auf = verknuepft::auftraege(&e.a, None).unwrap();
    let rels: Vec<&str> = auf.iter().map(|a| a.rel.as_str()).collect();
    assert_eq!(rels, vec!["gut.txt"], "nur der saubere Pfad bleibt");
    assert!(auf[0].quelle.starts_with(&e.quelle));
}

#[test]
fn zwischen_wird_nach_absturz_geraeumt_beim_start_und_beim_loesen() {
    let e = env();
    e.schreiben("a.txt", b"x");
    let id = e.verknuepfen();
    let zw = e.a.dir().join("verknuepft").join("zwischen");
    fs::create_dir_all(&zw).unwrap();
    fs::write(zw.join("liegengeblieben.pdf"), b"Klartext vertraulich").unwrap();
    assert!(!e.zwischen_leer());
    // Start (Beobachter und Textarbeiter rufen das auf).
    verknuepft::zwischen_raeumen(e.a.dir());
    assert!(e.zwischen_leer());
    // Lösen räumt ebenfalls.
    fs::write(zw.join("noch_eine.txt"), b"Klartext").unwrap();
    verknuepft::loesen(&e.a, &id).unwrap();
    assert!(e.zwischen_leer());
    // Ein Ordner ohne `zwischen/` ist kein Fehler.
    verknuepft::zwischen_raeumen(&e.d.path().join("nichts"));
}

#[test]
fn voruebergehende_ocr_fehler_bleiben_offen_und_neu_einlesen_setzt_zurueck() {
    let e = env();
    e.schreiben("Scan.pdf", &bild_pdf(&format!("Vertrag {KONTROLLE}")));
    e.schreiben("Tafel.png", b"\x89PNG OCRTEXT: Whiteboard Eisvogel)");
    let id = e.verknuepfen();
    let k = Some(ocr::TEST_KENNUNG);
    // Die OCR scheitert an der Zeitgrenze. Nicht endgültig: weiter offen, bis die Versuche aufgebraucht sind.
    for versuch in 1..=3u32 {
        let l = e.lauf(k, Some("ocr_zeit"));
        assert_eq!(l.ok, 0, "{l:?}");
        let b = verknuepft::bestand(e.a.dir(), &id);
        let scan = &b.dateien["Scan.pdf"];
        assert_eq!(scan.stand, "kein_text");
        assert_eq!(scan.code.as_deref(), Some("extrakt.zeit"));
        assert_eq!(scan.versuche, versuch);
        let bild = &b.dateien["Tafel.png"];
        assert_eq!(bild.stand, "fehler");
        assert_eq!(bild.code.as_deref(), Some("extrakt.zeit"));
        assert_eq!(bild.versuche, versuch);
        let offen = verknuepft::auftraege(&e.a, k).unwrap().len();
        assert_eq!(
            offen,
            if versuch < 3 { 2 } else { 0 },
            "nach Versuch {versuch}"
        );
    }
    // Ein neuer Lauf tut nichts mehr (Versuche aufgebraucht).
    let l = e.lauf(k, Some("ocr"));
    assert_eq!(l.ok, 0);
    // „Neu einlesen“ setzt die Marken zurück: beides wird wieder gelesen.
    verknuepft::marken_zuruecksetzen(&e.a, Some(&id)).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    for rel in ["Scan.pdf", "Tafel.png"] {
        assert_eq!(b.dateien[rel].versuche, 0);
        assert_eq!(b.dateien[rel].code, None);
        assert!(b.dateien[rel].ocr.is_none());
    }
    assert_eq!(verknuepft::auftraege(&e.a, k).unwrap().len(), 2);
    let l = e.lauf(k, Some("ocr"));
    assert_eq!(l.ok, 2, "{l:?}");
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    assert_eq!(e.treffer("Eisvogel").len(), 1);
    // Unbekannte Kennung: nicht gefunden.
    let fremd = super::neue_id(super::PRAEFIX_ORDNER);
    assert_eq!(
        code(verknuepft::marken_zuruecksetzen(&e.a, Some(&fremd)).unwrap_err()),
        "ablage.nicht_gefunden"
    );
}

#[test]
fn unlesbarer_unterordner_laesst_dateien_nicht_als_geloescht_gelten() {
    let e = env();
    e.schreiben("oben.txt", b"oben");
    e.schreiben("sub/tief/a.txt", format!("alt {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    e.lauf(None, None);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    let dat = e.rel_dat(&id, "sub/tief/a.txt");
    verknuepft::tests_haken::DIR_FEHLER.with(|d| *d.borrow_mut() = vec![e.quelle.join("sub")]);
    let ab = verknuepft::abtasten_roh(&verknuepft::liste(e.a.dir()).unwrap()[0]).unwrap();
    assert_eq!(ab.unlesbar, vec!["sub".to_string()]);
    assert!(ab.dateien.contains_key("oben.txt"));
    assert!(!ab.dateien.contains_key("sub/tief/a.txt"));
    verknuepft::abtasten(&e.a, &id).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(!b.nicht_erreichbar);
    assert_eq!(
        b.dateien["sub/tief/a.txt"].dat, dat,
        "Eintrag und Kennung bleiben"
    );
    assert_eq!(b.dateien["sub/tief/a.txt"].stand, "ok");
    assert_eq!(e.treffer(KONTROLLE).len(), 1, "Text und Index bleiben");
    // Die Wurzel selbst nicht lesbar: die ganze Abtastung scheitert, der Bestand bleibt.
    verknuepft::tests_haken::DIR_FEHLER.with(|d| *d.borrow_mut() = vec![e.quelle.clone()]);
    assert!(verknuepft::abtasten_roh(&verknuepft::liste(e.a.dir()).unwrap()[0]).is_err());
    verknuepft::abtasten(&e.a, &id).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(b.nicht_erreichbar);
    assert_eq!(b.dateien.len(), 2);
    verknuepft::tests_haken::DIR_FEHLER.with(|d| d.borrow_mut().clear());
    // Wieder lesbar und wirklich gelöscht: jetzt fällt die Datei weg.
    fs::remove_dir_all(e.quelle.join("sub")).unwrap();
    verknuepft::abtasten(&e.a, &id).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(!b.nicht_erreichbar);
    assert!(!b.dateien.contains_key("sub/tief/a.txt"));
}

/// Echter Berechtigungsfehler (als Administrator wirkungslos: dort liest `read_dir` trotz `chmod 000`).
#[cfg(unix)]
#[test]
fn unterordner_ohne_leserecht_bleibt_im_bestand() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: `geteuid` hat keine Voraussetzungen.
    if unsafe { libc::geteuid() } == 0 {
        // Der Pfad ist über den Testhaken (`DIR_FEHLER`) im Test
        // `unlesbarer_unterordner_laesst_dateien_nicht_als_geloescht_gelten` abgedeckt.
        return;
    }
    let e = env();
    e.schreiben("sub/a.txt", b"alt");
    let id = e.verknuepfen();
    let sub = e.quelle.join("sub");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o000)).unwrap();
    let r = verknuepft::abtasten(&e.a, &id);
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
    r.unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(b.dateien.contains_key("sub/a.txt"));
}

#[test]
fn abgetastet_wird_bei_jedem_abtasten_aktualisiert() {
    let e = env();
    e.schreiben("a.txt", b"x");
    let id = e.verknuepfen();
    let t1 = verknuepft::bestand(e.a.dir(), &id).abgetastet;
    assert!(t1 > 0);
    std::thread::sleep(Duration::from_millis(20));
    // Nichts geändert: der Zeitpunkt zählt trotzdem.
    assert!(!verknuepft::abtasten(&e.a, &id).unwrap());
    assert!(verknuepft::bestand(e.a.dir(), &id).abgetastet > t1);
}

#[test]
fn reparse_urteil_wertet_echte_masken_und_tags() {
    use super::eintrag::{online_urteil, reparse_tag_cloud, reparse_urteil, Urteil};
    const REPARSE: u32 = 0x400;
    const OFFLINE: u32 = 0x1000;
    const RECALL_OPEN: u32 = 0x4_0000;
    const RECALL_DATA: u32 = 0x40_0000;
    const PINNED: u32 = 0x8_0000;
    const ORDNER: u32 = 0x10;
    const ARCHIV: u32 = 0x20;
    const SYMLINK: u32 = 0xA000_000C;
    const MOUNT: u32 = 0xA000_0003;
    const CLOUD: u32 = 0x9000_001A;
    // OneDrive: Tag `IO_REPARSE_TAG_CLOUD_7`.
    const ONEDRIVE: u32 = 0x9000_701A;

    // Echte Maske einer Datei, die nur online liegt (Get-Item und Find-Daten: REPARSE_POINT, SPARSE,
    // OFFLINE, UNPINNED, RECALL_ON_DATA_ACCESS).
    assert_eq!(reparse_urteil(ONEDRIVE, 0x50_1620), Urteil::NurOnline);
    // Was `fs::symlink_metadata` unter Rust meldet (ohne 0x400 und ohne OFFLINE, Tag 0): ebenfalls nur online.
    assert_eq!(reparse_urteil(0, 0x50_0020), Urteil::NurOnline);
    // Hydrierte Cloud-Datei (Tag Cloud, ohne RECALL): lesbar, auch angeheftet.
    assert_eq!(
        reparse_urteil(CLOUD, REPARSE | ARCHIV),
        Urteil::Lesbar,
        "hydriert"
    );
    assert_eq!(
        reparse_urteil(ONEDRIVE, REPARSE | ARCHIV | PINNED),
        Urteil::Lesbar
    );
    // Jedes der drei Online-Bits genügt, mit und ohne Reparse-Bit, bei Tag 0 und bei fremdem Tag.
    for bit in [OFFLINE, RECALL_OPEN, RECALL_DATA] {
        assert!(online_urteil(bit));
        for tag in [0, CLOUD, ONEDRIVE, SYMLINK] {
            for reparse in [0, REPARSE] {
                assert_eq!(
                    reparse_urteil(tag, ARCHIV | bit | reparse),
                    Urteil::NurOnline,
                    "{tag:#x} {bit:#x}"
                );
            }
        }
    }
    assert!(!online_urteil(
        ARCHIV | REPARSE | 0x20_0000 | PINNED | 0x10_0000
    ));
    // Symlink, Mount-Point (Junction) und Unbekanntes: abgewiesen, auch als Ordner.
    for tag in [
        SYMLINK,
        MOUNT,
        0x8000_001B,
        0x8000_0013,
        0xA000_001D,
        0,
        0x9000_001B,
    ] {
        for attribute in [REPARSE | ARCHIV, REPARSE | ORDNER] {
            assert_eq!(reparse_urteil(tag, attribute), Urteil::Umweg, "{tag:#x}");
        }
    }
    // Ohne Reparse-Attribut ist der Tag belanglos.
    assert_eq!(reparse_urteil(SYMLINK, ARCHIV), Urteil::Lesbar);
    assert_eq!(reparse_urteil(0, ORDNER), Urteil::Lesbar);
    // Cloud-Tags samt Untertags `_1` bis `_F`.
    assert!(reparse_tag_cloud(CLOUD));
    for n in 1..=0xFu32 {
        let t = CLOUD | (n << 12);
        assert!(reparse_tag_cloud(t), "{t:#x}");
        assert_eq!(reparse_urteil(t, REPARSE | ARCHIV), Urteil::Lesbar);
    }
    assert!(!reparse_tag_cloud(SYMLINK));
    assert!(!reparse_tag_cloud(0x9000_001B));
    assert!(!reparse_tag_cloud(0x8000_001A));
    // Ordner: mit Cloud-Tag betretbar, mit RECALL (auch nur eines der drei Bits) nicht; Junction nie.
    assert_eq!(reparse_urteil(ONEDRIVE, REPARSE | ORDNER), Urteil::Lesbar);
    for bit in [OFFLINE, RECALL_OPEN, RECALL_DATA] {
        assert_eq!(
            reparse_urteil(ONEDRIVE, REPARSE | ORDNER | bit),
            Urteil::NurOnline
        );
        assert_eq!(reparse_urteil(0, ORDNER | bit), Urteil::NurOnline);
    }
    assert_eq!(reparse_urteil(MOUNT, REPARSE | ORDNER), Urteil::Umweg);
}

/// Rückt die Test-Naht beim Verlassen des Tests wieder gerade (Tests teilen sich unter
/// `--test-threads=1` den Faden).
struct NahtWaechter;

impl Drop for NahtWaechter {
    fn drop(&mut self) {
        eintrag::naht::leeren();
    }
}

fn naht_sauber() -> NahtWaechter {
    eintrag::naht::leeren();
    NahtWaechter
}

/// Find-Daten einer Datei, die nur online liegt (echte Maske aus der Windows-Probe).
const NUR_ONLINE_DATEI: u32 = 0x50_1620;
/// Find-Daten eines Ordners, der nur online liegt (RECALL_ON_DATA_ACCESS, Cloud-Tag).
const NUR_ONLINE_ORDNER: u32 = 0x50_0410;
const ONEDRIVE_TAG: u32 = 0x9000_701A;

/// Ein Programm, das nicht startet: jeder Aufruf des Unterprozesses endete als Fehlerstand, nie als Erfolg.
fn totes_programm() -> super::textlauf::Programm {
    super::textlauf::Programm {
        pfad: PathBuf::from("/nonexistent/lva-extrakt"),
        vorweg: vec![],
        umgebung: vec![],
    }
}

fn lauf_mit(e: &Env, prog: &super::textlauf::Programm) -> verknuepft::Lauf {
    verknuepft::rueckstellungen_vergessen();
    verknuepft::lauf(
        &e.a,
        prog,
        TEST_GRENZEN,
        &AtomicBool::new(false),
        &|| {},
        None,
    )
    .unwrap()
}

#[test]
fn nur_online_dateien_sind_sichtbar_aber_nie_gelesen_oder_indexiert() {
    let _n = naht_sauber();
    let e = env();
    e.schreiben("Lokal.txt", format!("lokaler Text {KONTROLLE}").as_bytes());
    let wolke = e.schreiben("Wolke.docx", b"niemals lesen");
    let wolke_txt = e.schreiben("Akte/Nur online.txt", b"Wolkenwort Kuckuck");
    e.schreiben("Offen/innen.txt", b"darin");
    // Windows liefert diese Maske (Find-Daten); `fs::symlink_metadata` hätte 0x500020 gemeldet.
    eintrag::naht::setzen(&wolke, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    eintrag::naht::setzen(&wolke_txt, 0x50_0020, 0);
    eintrag::naht::setzen(&e.quelle.join("Offen"), NUR_ONLINE_ORDNER, ONEDRIVE_TAG);
    let id = e.verknuepfen();
    let b = verknuepft::bestand(e.a.dir(), &id);
    let rels: Vec<&str> = b.dateien.keys().map(String::as_str).collect();
    assert_eq!(
        rels,
        ["Akte/Nur online.txt", "Lokal.txt", "Wolke.docx"],
        "der Ordner „Offen“ wird nicht betreten"
    );
    assert_eq!(b.nur_online_ordner, 1);
    assert_eq!(b.ausgelassen, 0);
    for rel in ["Akte/Nur online.txt", "Wolke.docx"] {
        let ein = &b.dateien[rel];
        assert_eq!(ein.stand, verknuepft::NUR_ONLINE, "{rel}");
        assert_eq!(
            ein.groesse,
            fs::metadata(e.quelle.join(rel)).unwrap().len(),
            "Größe aus den Find-Daten"
        );
        assert_eq!(
            (ein.sha256.as_deref(), ein.textrev.as_deref()),
            (None, None)
        );
    }
    assert_eq!(b.dateien["Lokal.txt"].stand, "ausstehend");
    // Aufträge: nur die lokale Datei, nie die Wolkendateien.
    let auf: Vec<String> = verknuepft::auftraege(&e.a, None)
        .unwrap()
        .into_iter()
        .map(|a| a.rel)
        .collect();
    assert_eq!(auf, ["Lokal.txt"]);
    // Lauf mit dem echten Unterprozess: nur die lokale Datei wird gelesen; Zwischenordner bleibt leer.
    let l = e.lauf(None, None);
    assert_eq!((l.ok, l.kein_text, l.fehler), (1, 0, 0));
    assert!(e.zwischen_leer());
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    assert!(e.treffer("Kuckuck").is_empty(), "kein Text, kein Index");
    let q = verknuepft::index_quellen(&e.a).unwrap();
    let wolke_dat = e.rel_dat(&id, "Wolke.docx");
    assert!(q.iter().all(|q| q.dat != wolke_dat), "nicht im Index");
    assert_eq!(q.len(), 1);
    // Sicht: Liste mit Name, Größe und Stand `nur_online`; Zählung (zwei Dateien, ein Ordner).
    let s = e.a.ansicht().unwrap();
    assert_eq!(s.verknuepft[0].nur_online, 3);
    assert_eq!(s.verknuepft[0].ordner.dateien, 3);
    let d = s
        .verknuepft_dateien
        .iter()
        .find(|d| d.name == "Wolke.docx")
        .unwrap();
    assert_eq!(d.text.as_deref(), Some("nur_online"));
    assert_eq!(d.groesse, 13);
    assert!(d.verknuepft && d.nur_geraet);
}

#[test]
fn nur_online_datei_erreicht_nie_momentkopie_oder_unterprozess() {
    let _n = naht_sauber();
    let e = env();
    let a = e.schreiben("a.txt", b"Wolkenwort");
    let b = e.schreiben("sub/b.pdf", b"%PDF-1.4");
    eintrag::naht::setzen(&a, 0x50_0020, 0);
    eintrag::naht::setzen(&b, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    let id = e.verknuepfen();
    // Alles liegt nur online: kein Auftrag, ein Lauf mit einem Programm, das nicht startet, bleibt leer
    // (jeder Aufruf des Unterprozesses wäre als Fehlerstand eingetragen worden).
    assert!(verknuepft::auftraege(&e.a, None).unwrap().is_empty());
    assert!(!verknuepft::offen(&e.a, None));
    assert_eq!(lauf_mit(&e, &totes_programm()), verknuepft::Lauf::default());
    assert!(!e.a.dir().join("verknuepft").join("zwischen").exists());
    // Ein veralteter Auftrag (die Datei war beim Abtasten noch hydriert) wird nicht kopiert.
    let bestand = verknuepft::bestand(e.a.dir(), &id);
    let ein = &bestand.dateien["a.txt"];
    let auf = Auftrag {
        ordner: id.clone(),
        rel: "a.txt".into(),
        dat: ein.dat.clone(),
        typ: ein.typ.clone(),
        groesse: ein.groesse,
        zeit: ein.zeit,
        wurzel: e.quelle.clone(),
        quelle: a.clone(),
        art: Art::Text,
        versuche: 0,
    };
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    assert!(!e.a.dir().join("verknuepft").join("zwischen").exists());
    assert_eq!(
        verknuepft::bearbeiten(
            &e.a,
            &auf,
            &totes_programm(),
            TEST_GRENZEN,
            &AtomicBool::new(false),
            None
        )
        .unwrap(),
        Ausgang::Uebersprungen,
        "nichts eingetragen, kein Unterprozess"
    );
    assert_eq!(
        verknuepft::bestand(e.a.dir(), &id).dateien["a.txt"].stand,
        verknuepft::NUR_ONLINE
    );
    assert!(e.treffer("Wolkenwort").is_empty());
}

#[test]
fn momentkopie_prueft_am_handle_und_in_jedem_pfadglied_auf_nur_online() {
    let _n = naht_sauber();
    let e = env();
    let datei = e.schreiben(
        "Akte/brief.txt",
        format!("vertraulich {KONTROLLE}").as_bytes(),
    );
    e.verknuepfen();
    let auf = verknuepft::auftraege(&e.a, None).unwrap().remove(0);
    // Die Find-Daten sehen eine hydrierte Datei, das Handle meldet RECALL: Wettlauf mit „Speicher
    // freigeben“. Das Handle wird geschlossen, ohne zu lesen.
    eintrag::naht::handle_setzen(&datei, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    assert!(!e.a.dir().join("verknuepft").join("zwischen").exists());
    // Ein Handle mit unbekanntem Tag: abgewiesen, nie gelesen.
    eintrag::naht::handle_setzen(&datei, 0x420, 0x8000_001B);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "ablage.verknuepfung"
    );
    assert!(!e.a.dir().join("verknuepft").join("zwischen").exists());
    // Hydriert und unauffällig: wird gelesen (die Kopie ist danach wieder weg).
    eintrag::naht::handle_setzen(&datei, 0x420, 0x9000_001A);
    let (kopie, _sha) = verknuepft::momentkopie(&e.a, &auf).unwrap();
    assert!(kopie.is_file());
    fs::remove_file(&kopie).unwrap();
    eintrag::naht::leeren();
    // Das Zwischenverzeichnis liegt inzwischen nur online: auch das ist ein Pfadglied.
    eintrag::naht::setzen(&e.quelle.join("Akte"), NUR_ONLINE_ORDNER, ONEDRIVE_TAG);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    // Ebenso die Wurzel selbst.
    eintrag::naht::leeren();
    eintrag::naht::setzen(&e.quelle, NUR_ONLINE_ORDNER, ONEDRIVE_TAG);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    assert!(e.zwischen_leer());
}

#[test]
fn hydrierte_datei_wird_beim_naechsten_lauf_aufgenommen_und_online_wieder_ausgeblendet() {
    let _n = naht_sauber();
    let e = env();
    let p = e.schreiben("Vertrag.txt", format!("Laufzeit {KONTROLLE}").as_bytes());
    eintrag::naht::setzen(&p, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "Vertrag.txt");
    let online = verknuepft::abtasten_roh(&verknuepft::liste(e.a.dir()).unwrap()[0]).unwrap();
    assert!(online.online.contains("Vertrag.txt"));
    assert_eq!(e.lauf(None, None).ok, 0);
    assert!(e.treffer(KONTROLLE).is_empty());
    // Der Nutzer wählt „Immer auf diesem Gerät behalten“: hydriert, die Kennung bleibt.
    eintrag::naht::leeren();
    let hydriert = verknuepft::abtasten_roh(&verknuepft::liste(e.a.dir()).unwrap()[0]).unwrap();
    assert_ne!(
        online.fingerabdruck(),
        hydriert.fingerabdruck(),
        "die Entprellung sieht den Wechsel"
    );
    assert!(verknuepft::abtasten(&e.a, &id).unwrap());
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!(b.dateien["Vertrag.txt"].stand, "ausstehend");
    assert_eq!(b.dateien["Vertrag.txt"].dat, dat);
    assert_eq!(e.lauf(None, None).ok, 1);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    // „Speicher freigeben“: wieder nur online, Text und Indexeintrag verschwinden.
    eintrag::naht::setzen(&p, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    assert!(verknuepft::abtasten(&e.a, &id).unwrap());
    index::abgleichen(&e.a).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!(b.dateien["Vertrag.txt"].stand, verknuepft::NUR_ONLINE);
    assert_eq!(b.dateien["Vertrag.txt"].textrev, None);
    assert!(e.treffer(KONTROLLE).is_empty());
    assert!(!e
        .a
        .dir()
        .join("verknuepft")
        .join("text")
        .join(&dat)
        .exists());
}

#[test]
fn rueckstellung_einer_online_gewordenen_datei_entfaellt_bei_der_hydrierung() {
    let _n = naht_sauber();
    let e = env();
    let p = e.schreiben("a.txt", format!("Text {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    // Wettlauf: nach dem Abtasten wird die Datei wieder online (nur das Handle sieht es). Der Lauf liest
    // nichts, stellt die Datei in diesem Prozess zurück und trägt nichts ein.
    eintrag::naht::handle_setzen(&p, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    verknuepft::rueckstellungen_vergessen();
    let nochmal = || {
        verknuepft::lauf(
            &e.a,
            &kind(None),
            TEST_GRENZEN,
            &AtomicBool::new(false),
            &|| {},
            None,
        )
        .unwrap()
    };
    let l = nochmal();
    assert_eq!((l.ok, l.fehler), (0, 1));
    assert!(e.zwischen_leer());
    assert!(e.treffer(KONTROLLE).is_empty());
    assert_eq!(nochmal().fehler, 0, "zurückgestellt");
    // Der nächste Abtastlauf sieht sie auch in den Find-Daten als online.
    eintrag::naht::leeren();
    eintrag::naht::setzen(&p, NUR_ONLINE_DATEI, ONEDRIVE_TAG);
    verknuepft::abtasten(&e.a, &id).unwrap();
    assert_eq!(
        verknuepft::bestand(e.a.dir(), &id).dateien["a.txt"].stand,
        verknuepft::NUR_ONLINE
    );
    // Später hydriert: ohne Neustart gelesen (die Rückstellung des früheren Stands ist entfallen).
    eintrag::naht::leeren();
    verknuepft::abtasten(&e.a, &id).unwrap();
    assert_eq!(nochmal().ok, 1);
    index::abgleichen(&e.a).unwrap();
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
}

#[test]
fn nur_online_ordner_wird_nicht_betreten_bekannte_dateien_bleiben() {
    let _n = naht_sauber();
    let e = env();
    e.schreiben("Akte/alt.txt", format!("alt {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    e.lauf(None, None);
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    // Der Ordner wird „nur online“: nicht betreten; die bekannten Dateien fallen nicht aus dem Bestand.
    eintrag::naht::setzen(&e.quelle.join("Akte"), NUR_ONLINE_ORDNER, ONEDRIVE_TAG);
    verknuepft::abtasten(&e.a, &id).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!(b.nur_online_ordner, 1);
    assert!(b.dateien.contains_key("Akte/alt.txt"));
    // Der Fehler beim Auflisten gilt für den ganzen Ordner, nie für eine Teilliste.
    eintrag::naht::leeren();
    verknuepft::tests_haken::DIR_FEHLER.with(|d| d.borrow_mut().push(e.quelle.join("Akte")));
    verknuepft::abtasten(&e.a, &id).unwrap();
    verknuepft::tests_haken::DIR_FEHLER.with(|d| d.borrow_mut().clear());
    assert!(verknuepft::bestand(e.a.dir(), &id)
        .dateien
        .contains_key("Akte/alt.txt"));
}

#[test]
fn nur_online_wurzel_wird_nicht_verknuepft_und_nicht_abgetastet() {
    let _n = naht_sauber();
    let e = env();
    e.schreiben("a.txt", b"x");
    eintrag::naht::setzen(&e.quelle, NUR_ONLINE_ORDNER, ONEDRIVE_TAG);
    assert_eq!(
        code(verknuepft::verknuepfen(&e.a, &e.quelle).unwrap_err()),
        "ablage.nur_online"
    );
    eintrag::naht::leeren();
    let id = e.verknuepfen();
    eintrag::naht::setzen(&e.quelle, NUR_ONLINE_ORDNER, ONEDRIVE_TAG);
    verknuepft::abtasten(&e.a, &id).unwrap();
    let b = verknuepft::bestand(e.a.dir(), &id);
    assert!(b.nicht_erreichbar);
    assert!(b.dateien.contains_key("a.txt"), "der alte Bestand bleibt");
}

#[test]
fn altbestand_ohne_nur_online_feld_wird_gelesen_und_vorwaerts_uebernommen() {
    let _n = naht_sauber();
    let e = env();
    e.schreiben("a.txt", format!("alt {KONTROLLE}").as_bytes());
    e.schreiben("b.txt", b"b");
    let id = e.verknuepfen();
    e.lauf(None, None);
    let dat = e.rel_dat(&id, "a.txt");
    // Bestand der Vorversion: ohne `nur_online_ordner`, `ausgelassen` zählte die Cloud-Dateien mit.
    let p = e.a.dir().join("verknuepft").join(&id).join("bestand.json");
    let mut b: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    assert!(b
        .as_object_mut()
        .unwrap()
        .remove("nur_online_ordner")
        .is_some());
    b["ausgelassen"] = serde_json::json!(3);
    fs::write(&p, serde_json::to_vec(&b).unwrap()).unwrap();
    let alt = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!((alt.nur_online_ordner, alt.ausgelassen), (0, 3));
    assert_eq!(alt.dateien["a.txt"].stand, "ok", "Text bleibt");
    // Der nächste Abtastlauf übernimmt vorwärts: gleiche Kennungen, Text und Index bleiben, die alte
    // Zählung der Ausgelassenen wird durch die echte ersetzt.
    assert!(verknuepft::abtasten(&e.a, &id).unwrap());
    let neu = verknuepft::bestand(e.a.dir(), &id);
    assert_eq!((neu.nur_online_ordner, neu.ausgelassen), (0, 0));
    assert_eq!(e.rel_dat(&id, "a.txt"), dat);
    assert_eq!(neu.dateien["a.txt"].stand, "ok");
    assert_eq!(e.treffer(KONTROLLE).len(), 1);
    // Ein Bestand der neuen Version enthält das neue Feld und bleibt les- und abtastbar.
    assert!(!verknuepft::abtasten(&e.a, &id).unwrap());
    let roh: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    assert_eq!(roh["nur_online_ordner"], 0);
}

// ---------------------------------------------------------------------------
// D3 x D7: KI-Zugriff und verknüpfte Ordner (fail-closed)
// ---------------------------------------------------------------------------

fn lokal() -> ZielAnbieter {
    ZielAnbieter {
        id: "lokal".into(),
        lokal: true,
        datenklasse_max: Klasse::Personal,
    }
}

fn cloud(id: &str, max: Klasse) -> ZielAnbieter {
    ZielAnbieter {
        id: id.into(),
        lokal: false,
        datenklasse_max: max,
    }
}

fn ziele() -> Vec<ZielAnbieter> {
    vec![
        lokal(),
        cloud("anbieter-x", Klasse::Public),
        cloud("anbieter-x", Klasse::Internal),
        cloud("anbieter-x", Klasse::Confidential),
    ]
}

impl Env {
    fn ki(&self, q: &str, b: &KiBereich, z: &ZielAnbieter) -> ki::AuszugAntwort {
        ki::auszuege_mit(&self.a, q, b, z, ki::TOP_K, &AblageEinbettung::Keine).unwrap()
    }

    /// Alle Bereiche, die eine Datei oder einen Ordner der verknüpften Seite ansprechen können.
    fn bereiche(&self, id: &str, dat: &str) -> Vec<KiBereich> {
        vec![
            KiBereich::Global,
            KiBereich::Ordner(id.to_string()),
            KiBereich::Projekt(vec![id.to_string()]),
            KiBereich::Projekt(vec![format!("{}{dat}", ki::LINK)]),
            KiBereich::Projekt(vec![format!("{}{id}", ki::LINK), dat.to_string()]),
        ]
    }

    fn null_auszuege(&self, q: &str, id: &str, dat: &str) {
        for b in self.bereiche(id, dat) {
            for z in ziele() {
                let r = self.ki(q, &b, &z);
                assert!(r.auszuege.is_empty(), "{b:?} {z:?}");
                assert!(r.kein_beleg, "{b:?} {z:?}");
                assert_eq!(r.abdeckung.durchsucht, 0, "{b:?} {z:?}");
                assert!(!ki::als_text(&r).contains(q), "{b:?} {z:?}");
            }
        }
    }

    /// Eine Datei in der eigenen Ablage (Kontrolle: die KI-Seite funktioniert, nur Verknüpftes fehlt).
    fn eigene_datei(&self, name: &str, text: &str) -> (String, String) {
        let w = self.a.ansicht().unwrap().ordner[0].id.clone();
        let q = self.d.path().join("eigen").join(name);
        fs::create_dir_all(q.parent().unwrap()).unwrap();
        fs::write(&q, text).unwrap();
        let z = self.a.importieren(&q, &w, None).unwrap();
        textfassung::lauf(
            &self.a,
            &kind(None),
            TEST_GRENZEN,
            &AtomicBool::new(false),
            &|| {},
        )
        .unwrap();
        index::abgleichen_mit(&self.a, &AblageEinbettung::Keine).unwrap();
        (w, z[0].dat.clone().unwrap())
    }
}

#[test]
fn verknuepfte_dateien_sind_fuer_die_ki_nie_erreichbar() {
    let e = env();
    e.schreiben(
        "Verträge/Wartung.txt",
        format!("Laufzeit {KONTROLLE} drei Jahre.").as_bytes(),
    );
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "Verträge/Wartung.txt");
    e.lauf(None, None);
    assert_eq!(
        e.treffer(KONTROLLE).len(),
        1,
        "die Suche in der Ablage sieht sie"
    );
    // Jede Freigabe, jede Klasse, jeder Weg, jedes Ziel: 0 Auszüge.
    for stufe in [
        None,
        Some(KiStufe::Aus),
        Some(KiStufe::An),
        Some(KiStufe::Gesperrt),
    ] {
        e.a.ki_setzen(&id, stufe).unwrap();
        for k in ["public", "internal", "confidential", "personal"] {
            e.a.ordner_einstellungen(
                &id,
                &AblageEinstellungen {
                    datenklasse: Some(k.into()),
                    ..Default::default()
                },
            )
            .unwrap();
            index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
            e.null_auszuege(KONTROLLE, &id, &dat);
        }
    }
    // Auch im Index selbst hat sie keinen KI-Leser: in SQL vor dem Ranking ausgeschlossen, selbst bei
    // `an` und `public`, auf beiden Wegen und für jedes Profil.
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    e.a.ordner_einstellungen(
        &id,
        &AblageEinstellungen {
            datenklasse: Some("public".into()),
            ..Default::default()
        },
    )
    .unwrap();
    index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    for weg in [index::KiWeg::Global, index::KiWeg::Projekt] {
        for p in index::KiProfil::ALLE {
            let s = index::ki_suchen(
                &e.a,
                KONTROLLE,
                &AblageEinbettung::Keine,
                &index::ki_leser(weg, p),
                50,
            )
            .unwrap();
            assert!(s.stellen.is_empty(), "{weg:?} {p:?}");
        }
    }
    // Kontrolle: dieselbe Frage trifft eine eigene, freigegebene Datei (der Test ist nicht leer).
    let (w, eigen) = e.eigene_datei("Eigen.txt", &format!("Eigen {KONTROLLE} Text"));
    e.a.ki_setzen(&w, Some(KiStufe::An)).unwrap();
    let r = e.ki(KONTROLLE, &KiBereich::Global, &lokal());
    assert_eq!(r.auszuege.len(), 1);
    assert_eq!(r.auszuege[0].dat, eigen);
    assert!(!r.auszuege.iter().any(|x| x.dat == dat));
    assert_eq!(
        r.abdeckung.im_bereich, 1,
        "verknüpfte Dateien zählen nicht zum Bereich"
    );
    // Auch im Projektweg mit dem verknüpften Ordner als Mitgliedschaft: weiter nichts.
    let r = e.ki(
        KONTROLLE,
        &KiBereich::Projekt(vec![id.clone(), eigen.clone()]),
        &lokal(),
    );
    assert_eq!(
        r.auszuege
            .iter()
            .map(|x| x.dat.as_str())
            .collect::<Vec<_>>(),
        [eigen.as_str()]
    );
}

#[test]
fn verknuepfte_dateien_im_index_ohne_ki_leser_stempel_wechselt() {
    let e = env();
    e.schreiben("a.txt", format!("x {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    e.lauf(None, None);
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    for q in verknuepft::index_quellen(&e.a).unwrap() {
        assert!(q.verknuepft);
        assert_eq!(
            index::leser_fuer(&q),
            vec![index::LESER_SUCHE.to_string()],
            "nur die Suche in der Ablage, nie ein KI-Weg"
        );
    }
    // Der Stempel unterscheidet sie von einer eigenen Quelle sonst gleicher Felder.
    let mut q = verknuepft::index_quellen(&e.a).unwrap().remove(0);
    let vk = index::stempel(&q);
    q.verknuepft = false;
    assert_ne!(vk, index::stempel(&q));
}

#[test]
fn herkunftsmarke_einer_verknuepften_datei_ist_entzogen_vor_und_nach_dem_loesen() {
    let e = env();
    e.schreiben("a.txt", format!("x {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "a.txt");
    e.lauf(None, None);
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    // Eine Marke, wie sie aus dieser Datei stammen müsste (Fassung ist eine Prüfsumme, keine `fas_`-
    // Kennung; auch mit einer wohlgeformten Fassung gäbe es keine Berechtigung).
    let rev = "ab".repeat(32);
    let fas = super::neue_id(super::PRAEFIX_FASSUNG);
    let m = ki::marke(&dat, &fas, &rev);
    let roh = ki::marke(
        &dat,
        &verknuepft::bestand(e.a.dir(), &id).dateien["a.txt"]
            .sha256
            .clone()
            .unwrap(),
        &rev,
    );
    for z in ziele() {
        for b in e.bereiche(&id, &dat) {
            let p = ki::herkunft_pruefen(&e.a, &[m.clone(), roh.clone()], &b, &z);
            assert!(
                p.iter().all(|x| x.urteil == Urteil::Entzogen),
                "{b:?} {z:?}"
            );
        }
    }
    verknuepft::loesen(&e.a, &id).unwrap();
    for z in ziele() {
        for b in e.bereiche(&id, &dat) {
            let p = ki::herkunft_pruefen(&e.a, &[m.clone(), roh.clone()], &b, &z);
            assert!(
                p.iter().all(|x| x.urteil == Urteil::Entzogen),
                "{b:?} {z:?}"
            );
        }
    }
    // Ein Text mit der Marke wird bereinigt: Auszug ersetzt.
    let text = format!("<<<auszug Q1 | {m}>>>\nInhalt\n<<<ende auszug Q1>>>\n");
    let p = ki::herkunft_pruefen(&e.a, std::slice::from_ref(&m), &KiBereich::Global, &lokal());
    let b = ki::bereinigen(&text, &p);
    assert_eq!(b.entzogen, 1);
    assert!(!b.text.contains("Inhalt"));
}

#[test]
fn cloud_ausnahme_fuer_verknuepfte_ordner_wird_abgewiesen_und_verfaellt() {
    let e = env();
    e.schreiben("a.txt", format!("x {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "a.txt");
    e.lauf(None, None);
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    // Standardklasse confidential: Erteilen trotzdem abgewiesen (nie erreichbar), nichts gespeichert.
    let f =
        e.a.cloud_ausnahme_setzen(&id, "anbieter-x", true)
            .unwrap_err();
    assert_eq!(code(f), "ablage.verknuepft_nur_lesen");
    let lage = freigabe::laden(e.a.dir());
    assert!(!freigabe::ausnahme_gilt(
        &lage,
        std::slice::from_ref(&id),
        "anbieter-x",
        |_| true
    ));
    // Entziehen bleibt erlaubt.
    e.a.cloud_ausnahme_setzen(&id, "anbieter-x", false).unwrap();
    // Eine (hypothetisch, etwa aus älterem Stand) vorhandene Ausnahme öffnet nichts: weder bei
    // confidential noch bei personal.
    freigabe::ausnahme_setzen(e.a.dir(), &id, "anbieter-x", true).unwrap();
    let c = cloud("anbieter-x", Klasse::Confidential);
    for k in ["confidential", "personal"] {
        e.a.ordner_einstellungen(
            &id,
            &AblageEinstellungen {
                datenklasse: Some(k.into()),
                ..Default::default()
            },
        )
        .unwrap();
        for b in e.bereiche(&id, &dat) {
            assert!(e.ki(KONTROLLE, &b, &c).auszuege.is_empty(), "{k} {b:?}");
            assert!(
                e.ki(KONTROLLE, &b, &lokal()).auszuege.is_empty(),
                "{k} {b:?}"
            );
        }
    }
    // Klassenwechsel verwirft die Ausnahme (wie bei Ordnern).
    freigabe::ausnahme_setzen(e.a.dir(), &id, "anbieter-x", true).unwrap();
    e.a.ordner_einstellungen(
        &id,
        &AblageEinstellungen {
            datenklasse: Some("internal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let lage = freigabe::laden(e.a.dir());
    assert!(!freigabe::ausnahme_gilt(
        &lage,
        std::slice::from_ref(&id),
        "anbieter-x",
        |_| true
    ));
    // Lösen verwirft Ausnahme und Freigabe-Eintrag des Ordners.
    freigabe::ausnahme_setzen(e.a.dir(), &id, "anbieter-x", true).unwrap();
    verknuepft::loesen(&e.a, &id).unwrap();
    let f = fs::read_to_string(e.a.dir().join(freigabe::DATEI)).unwrap();
    assert!(
        !f.contains(&id),
        "weder Freigabe noch Ausnahme bleiben stehen"
    );
}

#[test]
fn freigaben_neu_festlegen_schliesst_verknuepfte_ordner_ein_und_verwirft_ausnahmen() {
    let e = env();
    e.schreiben("a.txt", format!("x {KONTROLLE}").as_bytes());
    let id = e.verknuepfen();
    let dat = e.rel_dat(&id, "a.txt");
    e.lauf(None, None);
    e.a.ki_setzen(&id, Some(KiStufe::An)).unwrap();
    freigabe::ausnahme_setzen(e.a.dir(), &id, "anbieter-x", true).unwrap();
    // Freigabe defekt, ohne brauchbare Sicherung: alles gesperrt, auch das Verknüpfte.
    fs::write(e.a.dir().join(freigabe::DATEI), b"kaputt").unwrap();
    fs::write(e.a.dir().join(freigabe::VORHER), b"auch kaputt").unwrap();
    let s = e.a.ansicht().unwrap();
    assert_eq!(s.verknuepft[0].ordner.ki.zustand, "unklar");
    e.null_auszuege(KONTROLLE, &id, &dat);
    e.a.freigaben_neu_festlegen().unwrap();
    let s = e.a.ansicht().unwrap();
    assert_eq!(s.verknuepft[0].ordner.ki.zustand, "aus");
    let lage = freigabe::laden(e.a.dir());
    assert!(!freigabe::ausnahme_gilt(
        &lage,
        std::slice::from_ref(&id),
        "anbieter-x",
        |_| true
    ));
    e.null_auszuege(KONTROLLE, &id, &dat);
}
