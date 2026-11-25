//! lva#898: Bedienung sieht nur Dokumente; der gespeicherte Stand bleibt vollständig.
use super::*;

struct Probe {
    _temp: tempfile::TempDir,
    a: Ablage,
    ord: String,
    dat: String,
    fas: String,
    quelle: PathBuf,
}

fn probe(system: u8) -> Probe {
    let temp = tempfile::tempdir().unwrap();
    let a = Ablage::neu(temp.path().join("ablage"));
    assert_eq!(a.wartung(), None);
    let ord = a.ansicht().unwrap().ordner[0].id.clone();
    let quelle = temp.path().join("probe.txt");
    fs::write(&quelle, b"synthetische Probe").unwrap();
    let zeile = a.importieren(&quelle, &ord, None).unwrap().remove(0);
    let dat = zeile.dat.unwrap();
    let fas = zeile.fas.unwrap();
    let mut st = a.laden_intern().unwrap();
    if system != 0 {
        let root = super::super::system::kennung(&[7; 32], true, 0, "");
        let mut o = st.ordner[&ord].clone();
        o.id = root.clone();
        st.ordner.insert(root.clone(), o);
        // Auch eine gewöhnliche Kennung unter dem Systemordner gehört zum privaten Baum.
        st.ordner.get_mut(&ord).unwrap().eltern.wert = Some(root.clone());
        if system == 2 {
            st.ordner.remove(&root);
        } else if system == 3 {
            st.ordner.get_mut(&root).unwrap().eltern.wert = Some(ord.clone());
        }
    }
    a.sichern(&mut st).unwrap();
    Probe { _temp: temp, a, ord, dat, fas, quelle }
}

#[test]
fn laden_laesst_offene_auftraege_und_epoche_unveraendert() {
    let p = probe(0);
    let mut st = p.a.laden_intern().unwrap();
    st.journal.push(Eintrag::Loeschen {
        dat: vec![p.dat.clone()], ordner: vec![], aufheben: false,
    });
    p.a.sichern(&mut st).unwrap();
    let vorher = fs::read(p.a.journal_pfad()).unwrap();
    let geladen = p.a.laden().unwrap();
    assert_eq!(geladen.journal.len(), 1);
    assert!(geladen.dateien.contains_key(&p.dat));
    assert_eq!(fs::read(p.a.journal_pfad()).unwrap(), vorher);
    assert!(p.a.dir.join(DATEIEN).join(&p.dat).exists());
}

#[test]
fn lesende_wege_filtern_systemnachfahren_auch_ohne_systemwurzel() {
    for system in 0..=3 {
        let p = probe(system);
        let sichtbar = system == 0;
        let s = p.a.ansicht().unwrap();
        assert_eq!(s.ordner.iter().any(|o| o.id == p.ord), sichtbar);
        assert_eq!(s.dateien.iter().any(|d| d.id == p.dat), sichtbar);
        assert_eq!(s.belegt > 0, sichtbar);
        assert_eq!(p.a.fassungen(&p.dat).is_ok(), sichtbar);
        assert_eq!(p.a.original_pfad(&p.dat).is_ok(), sichtbar);
        assert_eq!(p.a.aufloesen(&p.dat).is_ok(), sichtbar);
        assert_eq!(p.a.dokument_kennungen().unwrap().contains(&p.dat), sichtbar);
        assert_eq!(p.a.dokument_kennungen().unwrap().contains(&p.ord), sichtbar);
        assert_eq!(!p.a.treffer_infos(&[p.dat.clone()]).unwrap().is_empty(), sichtbar);
        assert_eq!(!p.a.index_quellen().unwrap().is_empty(), sichtbar);
        assert_eq!(!p.a.text_auftraege().unwrap().is_empty(), sichtbar);
        let ki = p.a.ki_bestand().unwrap();
        assert_eq!(ki.dateien.contains_key(&p.dat), sichtbar);
        assert_eq!(ki.ordner.contains_key(&p.ord), sichtbar);
        assert_eq!(p.a.verweise().unwrap().iter().any(|(id, _, _)| id == &p.dat), sichtbar);
        // Papierkorb ist eine eigene Sicht, auch für bereits gelöschte Nachfahren.
        let mut st = p.a.laden_intern().unwrap();
        st.dateien.get_mut(&p.dat).unwrap().papierkorb.wert = Some(jetzt_ms());
        p.a.sichern(&mut st).unwrap();
        assert_eq!(p.a.ansicht().unwrap().papierkorb.iter().any(|d| d.id == p.dat), sichtbar);
    }
}

#[test]
fn schreibende_wege_wehren_systemnachfahren_ohne_schreiben_ab() {
    let wege: &[(&str, fn(&Probe) -> Res<()>)] = &[
        ("Ordner anlegen", |p| p.a.ordner_anlegen(Some(&p.ord), "Kind").map(|_| ())),
        ("Ordner umbenennen", |p| p.a.ordner_umbenennen(&p.ord, "Neu")),
        ("Ordner verschieben", |p| p.a.ordner_verschieben(&p.ord, None)),
        ("Ordner einstellen", |p| p.a.ordner_einstellungen(&p.ord, &AblageEinstellungen {
            datenklasse: None, nur_geraet: Some(true), zielordner_ki: None,
        })),
        ("KI setzen", |p| p.a.ki_setzen(&p.ord, Some(KiStufe::Aus))),
        ("Datei umbenennen", |p| p.a.datei_umbenennen(&p.dat, "Neu.txt")),
        ("Datei verschieben", |p| p.a.datei_verschieben(&p.dat, &p.ord)),
        ("Import", |p| p.a.datei_ablegen(&p.quelle, &p.ord, None, None).map(|_| ())),
        ("Datei Papierkorb", |p| p.a.in_papierkorb(AblageArt::Datei, &p.dat)),
        ("Ordner Papierkorb", |p| p.a.in_papierkorb(AblageArt::Ordner, &p.ord)),
        ("Fassung wählen", |p| p.a.fassung_waehlen(&p.dat, &p.fas)),
        ("Fassungen aufräumen", |p| p.a.fassungen_aufraeumen(&p.dat, jetzt_ms()).map(|_| ())),
        ("Text schreiben", |p| p.a.text_schreiben(&p.dat, &"a".repeat(64), b"{}").map(|_| ())),
        ("Cloud-Ausnahme", |p| p.a.cloud_ausnahme_setzen(&p.ord, "anbieter-x", false)),
    ];
    for (name, weg) in wege {
        for system in 0..=3 {
            let p = probe(system);
            let vorher = fs::read(p.a.journal_pfad()).unwrap();
            let erg = weg(&p);
            assert_eq!(erg.is_ok(), system == 0, "{name}, Fall {system}");
            if system != 0 {
                assert_eq!(erg.unwrap_err().code, "ablage.nicht_gefunden", "{name}");
                assert_eq!(fs::read(p.a.journal_pfad()).unwrap(), vorher, "{name}");
            }
        }
    }
}

#[test]
fn papierkorb_befehle_pruefen_auch_geloeschte_systemnachfahren() {
    for art in [AblageArt::Datei, AblageArt::Ordner] {
        for loeschen in [false, true] {
            for system in 0..=3 {
                let p = probe(system);
                let id = match art { AblageArt::Datei => &p.dat, AblageArt::Ordner => &p.ord };
                let mut st = p.a.laden_intern().unwrap();
                match art {
                    AblageArt::Datei => st.dateien.get_mut(id).unwrap().papierkorb.wert = Some(jetzt_ms()),
                    AblageArt::Ordner => st.ordner.get_mut(id).unwrap().papierkorb.wert = Some(jetzt_ms()),
                }
                p.a.sichern(&mut st).unwrap();
                let vorher = fs::read(p.a.journal_pfad()).unwrap();
                let erg = if loeschen {
                    p.a.endgueltig_loeschen(art, id).map(|_| ())
                } else {
                    p.a.wiederherstellen(art, id)
                };
                assert_eq!(erg.is_ok(), system == 0);
                if system != 0 {
                    assert_eq!(erg.unwrap_err().code, "ablage.nicht_gefunden");
                    assert_eq!(fs::read(p.a.journal_pfad()).unwrap(), vorher);
                }
            }
        }
    }
}

#[test]
fn sammelbefehle_lassen_systemobjekte_im_vollstaendigen_stand() {
    for system in 0..=3 {
        let p = probe(system);
        let mut st = p.a.laden_intern().unwrap();
        st.dateien.get_mut(&p.dat).unwrap().papierkorb.wert = Some(jetzt_ms());
        p.a.sichern(&mut st).unwrap();
        let b = p.a.papierkorb_leeren().unwrap();
        assert_eq!(b.dateien, u32::from(system == 0));
        assert_eq!(p.a.laden_intern().unwrap().dateien.contains_key(&p.dat), system != 0);
        p.a.freigaben_neu_festlegen().unwrap();
        let freigabe::Lage::Gueltig(f) = freigabe::laden(p.a.dir()) else { panic!("Freigabe fehlt") };
        assert_eq!(f.ordner.contains_key(&p.ord), system == 0);
    }
}

fn text_probe(p: &Probe) -> zf::Text {
    let st = p.a.laden_intern().unwrap();
    let f = &st.dateien[&p.dat].fassungen[0];
    zf::Text {
        textrev: erwartete_textrev(f).unwrap(), blob: String::new(), size: 0,
        extraktor: "probe".into(), extraktor_version: "1".into(), marken_version: 1,
        stand: "ok".into(), code: None, versuche: 0, versuche_zeit: 0,
    }
}

#[test]
fn text_und_entwurf_befehle_pruefen_quellen_und_ziele() {
    for system in 0..=3 {
        let p = probe(system);
        let text = text_probe(&p);
        let erg = p.a.text_ablegen(&p.dat, &p.fas, Some(b"{}"), text.clone());
        assert_eq!(erg.is_ok(), system == 0);
        assert_eq!(p.a.text_quelle(&p.dat, &text.textrev).is_some(), system == 0);

        let mut st = p.a.laden_intern().unwrap();
        st.ordner.get_mut(&p.ord).unwrap().zielordner_ki.wert = true;
        p.a.sichern(&mut st).unwrap();
        assert_eq!(p.a.entwurf_anlegen(Some(&p.ord), "Ziel", "Probe", &[], None).is_ok(), system == 0);
        assert_eq!(p.a.entwurf_anlegen(None, "Quelle", "Probe", &[p.dat.clone()], None).is_ok(), system == 0);
    }
}

#[test]
fn text_einlegen_prueft_nachfahren_vor_dem_schreiben() {
    for system in 0..=3 {
        let p = probe(system);
        let mut text = text_probe(&p);
        let st = p.a.laden_intern().unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "textrev": text.textrev,
            "original_sha256": st.dateien[&p.dat].fassungen[0].original_sha256,
        })).unwrap();
        text.blob = super::super::extrakt::hex(&sha2_digest(&bytes));
        text.size = bytes.len() as u64;
        let mut st = p.a.laden_intern().unwrap();
        st.dateien.get_mut(&p.dat).unwrap().fassungen[0].texte.push(text.clone());
        p.a.sichern(&mut st).unwrap();
        assert_eq!(p.a.text_einlegen(&p.dat, &p.fas, &text.textrev, &bytes).is_ok(), system == 0);
        assert_eq!(p.a.text_pfad(&p.dat, &text.textrev).exists(), system == 0);
    }
}

#[test]
fn gefundene_nachfahren_werden_nicht_angeboten_oder_aufgenommen() {
    for system in 0..=3 {
        let p = probe(system);
        let mut st = p.a.laden_intern().unwrap();
        let f = &st.dateien[&p.dat].fassungen[0];
        let fas = neue_id(PRAEFIX_FASSUNG);
        fs::copy(
            p.a.dir.join(DATEIEN).join(&p.dat).join(original_name(&f.fas, &f.typ)),
            p.a.dir.join(DATEIEN).join(&p.dat).join(original_name(&fas, &f.typ)),
        ).unwrap();
        st.gefunden.push(Gefunden {
            dat: p.dat.clone(), fas, typ: f.typ.clone(),
            groesse: f.size, ms: jetzt_ms(), vorgaenger: None,
        });
        p.a.sichern(&mut st).unwrap();
        assert_eq!(p.a.ansicht().unwrap().gefunden.iter().any(|g| g.dat == p.dat), system == 0);
        assert_eq!(p.a.gefundene_aufnehmen(&p.dat, &p.ord).is_ok(), system == 0);
    }
}

#[test]
fn reservierte_kennungen_bleiben_auch_in_gewoehnlichem_ordner_privat() {
    let mut p = probe(0);
    let mut st = p.a.laden_intern().unwrap();
    let mut d = st.dateien.remove(&p.dat).unwrap();
    let alt = p.dat.clone();
    p.dat = super::super::system::kennung(&[7; 32], false, 0, "probe");
    d.id = p.dat.clone();
    st.dateien.insert(p.dat.clone(), d);
    fs::rename(p.a.dir.join(DATEIEN).join(alt), p.a.dir.join(DATEIEN).join(&p.dat)).unwrap();
    p.a.sichern(&mut st).unwrap();
    assert!(p.a.ansicht().unwrap().dateien.is_empty());
    assert_eq!(p.a.ansicht().unwrap().ordner[0].dateien, 0);
    assert!(p.a.fassungen(&p.dat).is_err());
    assert!(p.a.datei_umbenennen(&p.dat, "Neu").is_err());
    // Eine gleichnamige gewöhnliche Datei darf entstehen, ohne die private Fassung zu ändern.
    let z = p.a.importieren(&p.quelle, &p.ord, Some(AblageKollisionswahl::NeueFassung)).unwrap();
    assert_ne!(z[0].dat.as_deref(), Some(p.dat.as_str()));
    assert_eq!(p.a.laden_intern().unwrap().dateien[&p.dat].fassungen.len(), 1);
    p.a.in_papierkorb(AblageArt::Ordner, &p.ord).unwrap();
    p.a.endgueltig_loeschen(AblageArt::Ordner, &p.ord).unwrap();
    assert!(p.a.laden_intern().unwrap().dateien.contains_key(&p.dat));
    assert!(p.a.dir.join(DATEIEN).join(&p.dat).exists());
}

#[test]
fn standardumzug_weist_systemziele_ab_und_laesst_systemquellen_stehen() {
    for system in 0..=3 {
        let p = probe(system);
        assert_eq!(p.a.standard_umziehen(&p.ord, &|_| false).is_ok(), system == 0);
        let ziel = neue_id(PRAEFIX_ORDNER);
        assert_eq!(p.a.standard_umziehen(&ziel, &|_| false).unwrap(), system == 0);
        let st = p.a.laden_intern().unwrap();
        assert_eq!(st.dateien[&p.dat].ordner.wert, if system == 0 { ziel } else { p.ord });
    }
}

#[test]
fn verschieben_prueft_systemziel_auch_bei_gewoehnlicher_quelle() {
    for system in 0..=3 {
        let p = probe(system);
        let quelle = p.a.ordner_anlegen(None, "Quelle").unwrap();
        let dat = p.a.importieren(&p.quelle, &quelle, None).unwrap()[0].dat.clone().unwrap();
        p.a.datei_umbenennen(&dat, "Quelle.txt").unwrap();
        assert_eq!(p.a.datei_verschieben(&dat, &p.ord).is_ok(), system == 0);
        assert_eq!(p.a.ordner_verschieben(&quelle, Some(&p.ord)).is_ok(), system == 0);
    }
}

#[test]
fn rueckweg_vergibt_systemnachfahren_keine_dokumentkennung() {
    struct Verweise;
    impl uebergang::UebergangVerweise for Verweise {
        fn umschreiben(&self, _: &BTreeMap<String, String>) -> Result<(), String> { Ok(()) }
    }
    for system in 0..=3 {
        let p = probe(system);
        let mut st = p.a.laden_intern().unwrap();
        st.lokal.push(p.dat.clone());
        p.a.sichern(&mut st).unwrap();
        assert_eq!(p.a.rueckweg(&Verweise).unwrap(), u32::from(system == 0));
        let st = p.a.laden_intern().unwrap();
        assert_eq!(st.dateien.contains_key(&p.dat), system != 0);
    }
}

#[test]
fn suche_und_kontext_verwerfen_systemnachfahren_auch_bei_altem_index() {
    use super::super::{index, ki, semantik::AblageEinbettung, tests_d2, textfassung};
    let p = probe(0);
    p.a.ki_setzen(&p.ord, Some(KiStufe::An)).unwrap();
    textfassung::lauf(
        &p.a, &tests_d2::kind(None), tests_d2::TEST_GRENZEN,
        &std::sync::atomic::AtomicBool::new(false), &|| {},
    ).unwrap();
    let emb = AblageEinbettung::Keine;
    index::abgleichen_mit(&p.a, &emb).unwrap();
    let bereich = ki::KiBereich::Global;
    let ziel = ki::ZielAnbieter { id: "lokal".into(), lokal: true, datenklasse_max: Klasse::Personal };
    let rev = p.a.ki_bestand().unwrap().dateien[&p.dat].ki_textrev.clone().unwrap();
    let marke = ki::marke(&p.dat, &p.fas, &rev);
    assert!(!index::suchen_mit(&p.a, "synthetische", &emb).unwrap().is_empty());
    assert!(!ki::auszuege_mit(&p.a, "synthetische", &bereich, &ziel, 5, &emb).unwrap().auszuege.is_empty());
    assert_eq!(ki::herkunft_pruefen(&p.a, &[marke.clone()], &bereich, &ziel)[0].urteil, ki::Urteil::Berechtigt);
    let mut st = p.a.laden_intern().unwrap();
    st.ordner.get_mut(&p.ord).unwrap().eltern.wert =
        Some(super::super::system::kennung(&[7; 32], true, 0, ""));
    p.a.sichern(&mut st).unwrap();
    // Vor dem Indexabgleich: alte SQL-Treffer dürfen keine Inhalte mehr liefern.
    assert!(index::suchen_mit(&p.a, "synthetische", &emb).unwrap().is_empty());
    assert!(ki::auszuege_mit(&p.a, "synthetische", &bereich, &ziel, 5, &emb).unwrap().auszuege.is_empty());
    assert_ne!(ki::herkunft_pruefen(&p.a, &[marke], &bereich, &ziel)[0].urteil, ki::Urteil::Berechtigt);
    assert!(p.a.index_quellen().unwrap().is_empty());
}
