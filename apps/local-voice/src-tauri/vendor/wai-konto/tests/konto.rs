use std::sync::mpsc;
use std::time::Duration;
use wai_konto::{
    Anmeldung, BILD_MAX_BYTES, BILD_MAX_KANTE, KontoFehler, KontoSpeicher, ProfilAenderung,
    ProfilEingabe,
};

fn png_mit(breite: u32, hoehe: u32, n: usize) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend_from_slice(&13u32.to_be_bytes());
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&breite.to_be_bytes());
    v.extend_from_slice(&hoehe.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    v.resize(n.max(v.len()), 7);
    v
}

fn png(n: usize) -> Vec<u8> {
    png_mit(16, 16, n)
}

fn speicher() -> (tempfile::TempDir, KontoSpeicher) {
    let t = tempfile::tempdir().unwrap();
    let s = KontoSpeicher::oeffnen(t.path().join("konto")).unwrap();
    (t, s)
}

fn eingabe(name: &str) -> ProfilEingabe {
    ProfilEingabe {
        anzeigename: name.into(),
        ..Default::default()
    }
}

#[test]
fn erstanlage_und_wiederoeffnen() {
    let (t, s) = speicher();
    let d = s.lesen().unwrap();
    assert_eq!(d.version, 1);
    assert_eq!(d.profile.len(), 1);
    assert_eq!(s.aktiv().unwrap().anzeigename, "Lokales Profil");
    assert_eq!(d.profile[0].id.len(), 26);
    let s2 = KontoSpeicher::oeffnen(t.path().join("konto")).unwrap();
    assert_eq!(s2.lesen().unwrap(), d);
}

#[test]
fn anlegen_aendern_entfernen_aktivieren() {
    let (_t, s) = speicher();
    let erst = s.aktiv().unwrap();
    let neu = s
        .profil_anlegen(ProfilEingabe {
            email: Some("  a@example.org ".into()),
            ..eingabe(" Anna ")
        })
        .unwrap();
    assert_eq!(neu.anzeigename, "Anna");
    assert_eq!(neu.email.as_deref(), Some("a@example.org"));
    let g = s
        .profil_aendern(
            &neu.id,
            ProfilAenderung {
                telefon: Some("123".into()),
                email: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(g.telefon.as_deref(), Some("123"));
    assert_eq!(g.email, None);
    assert!(matches!(
        s.profil_entfernen(&erst.id),
        Err(KontoFehler::NichtEntfernbar(_))
    ));
    let rev = s.sitzungsrevision().unwrap();
    s.aktivieren(&neu.id).unwrap();
    assert_eq!(s.aktiv().unwrap().id, neu.id);
    assert!(!s.pruefe_revision(rev).unwrap());
    assert!(s.pruefe_revision(rev + 1).unwrap());
    s.profil_entfernen(&erst.id).unwrap();
    assert!(matches!(
        s.profil_entfernen(&neu.id),
        Err(KontoFehler::NichtEntfernbar(_))
    ));
    assert!(matches!(
        s.profil_entfernen("X"),
        Err(KontoFehler::NichtGefunden(_))
    ));
}

#[test]
fn validierung() {
    let (_t, s) = speicher();
    assert!(s.profil_anlegen(eingabe("  ")).is_err());
    assert!(s.profil_anlegen(eingabe("a\u{0007}b")).is_err());
    assert!(s.profil_anlegen(eingabe(&"x".repeat(101))).is_err());
    for mail in ["ohne", "a@b@c", "@x.de", "a@", "a b@x.de"] {
        let e = ProfilEingabe {
            email: Some(mail.into()),
            ..eingabe("A")
        };
        assert!(s.profil_anlegen(e).is_err(), "{mail}");
    }
    let e = ProfilEingabe {
        telefon: Some("1".repeat(41)),
        ..eingabe("A")
    };
    assert!(s.profil_anlegen(e).is_err());
    let e = ProfilEingabe {
        farbe: Some(16),
        ..eingabe("A")
    };
    assert!(s.profil_anlegen(e).is_err());
    assert_eq!(s.lesen().unwrap().profile.len(), 1);
}

#[test]
fn schreibschutz_idp_felder() {
    let (t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.profil_aendern(
        &id,
        ProfilAenderung {
            email: Some("a@example.org".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut d = s.lesen().unwrap();
    d.profile[0].anmeldung = Some(Anmeldung {
        issuer: "https://idp.example.org".into(),
        subject: "s1".into(),
        seit_ms: 1,
        idp_felder: vec!["email".into()],
    });
    std::fs::write(
        t.path().join("konto/profile.json"),
        serde_json::to_vec(&d).unwrap(),
    )
    .unwrap();
    let r = s.profil_aendern(
        &id,
        ProfilAenderung {
            email: Some("b@example.org".into()),
            ..Default::default()
        },
    );
    assert!(matches!(r, Err(KontoFehler::Schreibgeschuetzt(f)) if f == "email"));
    s.profil_aendern(
        &id,
        ProfilAenderung {
            email: Some("a@example.org".into()),
            anzeigename: Some("Neu".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.aktiv().unwrap().anzeigename, "Neu");
}

#[test]
fn bild_grenzen() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
    assert!(matches!(
        s.bild_setzen(&id, b"GIF89a....."),
        Err(KontoFehler::Bild(_))
    ));
    assert!(matches!(
        s.bild_setzen(&id, &png(8)[..8]),
        Err(KontoFehler::Bild(_))
    ));
    assert!(matches!(
        s.bild_setzen(&id, &png(BILD_MAX_BYTES + 1)),
        Err(KontoFehler::Bild(_))
    ));
    s.bild_setzen(&id, &png(BILD_MAX_BYTES)).unwrap();
    assert!(s.aktiv().unwrap().hat_bild);
    assert_eq!(s.bild_lesen(&id).unwrap().unwrap().len(), BILD_MAX_BYTES);
    assert_eq!(s.bild_lesen("../x").unwrap(), None);
    s.bild_entfernen(&id).unwrap();
    assert!(!s.aktiv().unwrap().hat_bild);
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
    assert!(matches!(
        s.bild_setzen("../../x", &png(20)),
        Err(KontoFehler::NichtGefunden(_))
    ));
}

#[test]
fn generation_steigt() {
    let (_t, s) = speicher();
    let g0 = s.lesen().unwrap().generation;
    let id = s.aktiv().unwrap().id;
    s.profil_anlegen(eingabe("B")).unwrap();
    s.bild_setzen(&id, &png(20)).unwrap();
    assert_eq!(s.lesen().unwrap().generation, g0 + 2);
}

#[test]
fn zwei_instanzen_sehen_aenderungen() {
    let (t, a) = speicher();
    let b = KontoSpeicher::oeffnen(t.path().join("konto")).unwrap();
    let p = a.profil_anlegen(eingabe("Zweit")).unwrap();
    assert_eq!(b.lesen().unwrap().profile.len(), 2);
    b.aktivieren(&p.id).unwrap();
    assert_eq!(a.aktiv().unwrap().id, p.id);
}

#[test]
fn parallele_schreiber_verlieren_nichts() {
    let (t, _s) = speicher();
    let wurzel = t.path().join("konto");
    let hand: Vec<_> = (0..8)
        .map(|i| {
            let w = wurzel.clone();
            std::thread::spawn(move || {
                let s = KontoSpeicher::oeffnen(w).unwrap();
                for j in 0..10 {
                    s.profil_anlegen(eingabe(&format!("P{i}-{j}"))).unwrap();
                }
            })
        })
        .collect();
    hand.into_iter().for_each(|h| h.join().unwrap());
    let d = KontoSpeicher::oeffnen(wurzel).unwrap().lesen().unwrap();
    assert_eq!(d.profile.len(), 81);
    assert_eq!(d.generation, 81);
}

#[test]
fn beobachten_meldet_aenderung() {
    let (t, s) = speicher();
    let (tx, rx) = mpsc::channel();
    let _h = s
        .beobachten_mit(Duration::from_millis(20), move |d| {
            let _ = tx.send(d.generation);
        })
        .unwrap();
    let andere = KontoSpeicher::oeffnen(t.path().join("konto")).unwrap();
    andere.profil_anlegen(eingabe("N")).unwrap();
    let g = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(g, 2);
}

#[test]
fn beobachter_stoppt_beim_drop() {
    let (_t, s) = speicher();
    let h = s.beobachten(|_| {}).unwrap();
    let start = std::time::Instant::now();
    drop(h);
    assert!(start.elapsed() < Duration::from_millis(500));
}

#[test]
fn formatfehler_ohne_inhalt() {
    let (t, s) = speicher();
    std::fs::write(
        t.path().join("konto/profile.json"),
        "{\"geheim@example.org\": }",
    )
    .unwrap();
    let text = s.lesen().unwrap_err().to_string();
    assert!(text.contains("Zeile 1"), "{text}");
    assert!(!text.contains("geheim"), "{text}");
}

#[test]
fn oeffnen_ueberschreibt_unlesbare_datei_nicht() {
    let t = tempfile::tempdir().unwrap();
    let w = t.path().join("konto");
    std::fs::create_dir_all(w.join("profile.json")).unwrap();
    assert!(KontoSpeicher::oeffnen(w.clone()).is_err() || w.join("profile.json").is_dir());
    assert!(w.join("profile.json").is_dir());
}

#[test]
fn ungueltige_ids_in_datei_werden_abgelehnt() {
    let (t, s) = speicher();
    let pfad = t.path().join("konto/profile.json");
    let mut d = s.lesen().unwrap();
    d.profile[0].id = "../../x".into();
    std::fs::write(&pfad, serde_json::to_vec(&d).unwrap()).unwrap();
    assert!(matches!(
        s.lesen(),
        Err(KontoFehler::Ungueltig { feld: "id", .. })
    ));
}

#[test]
fn version_wird_vor_dem_rest_geprueft() {
    let (t, s) = speicher();
    std::fs::write(
        t.path().join("konto/profile.json"),
        r#"{"version":2,"neuesFeld":true}"#,
    )
    .unwrap();
    assert!(matches!(s.lesen(), Err(KontoFehler::Version(2))));
}

#[test]
fn ihdr_wird_geprueft() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    let fehler = |b: Vec<u8>| matches!(s.bild_setzen(&id, &b), Err(KontoFehler::Bild(_)));
    assert!(fehler(png_mit(0, 16, 40)));
    assert!(fehler(png_mit(16, 0, 40)));
    assert!(fehler(png_mit(BILD_MAX_KANTE + 1, 16, 40)));
    assert!(fehler(png_mit(16, BILD_MAX_KANTE + 1, 40)));
    assert!(fehler(png_mit(u32::MAX, u32::MAX, 40)));
    let mut falsche_laenge = png_mit(16, 16, 40);
    falsche_laenge[11] = 12;
    assert!(fehler(falsche_laenge));
    let mut falscher_typ = png_mit(16, 16, 40);
    falscher_typ[12..16].copy_from_slice(b"IDAT");
    assert!(fehler(falscher_typ));
    assert!(fehler(png_mit(16, 16, 40)[..32].to_vec()));
    assert!(!s.aktiv().unwrap().hat_bild);
    s.bild_setzen(&id, &png_mit(1, 1, 40)).unwrap();
    s.bild_setzen(&id, &png_mit(BILD_MAX_KANTE, BILD_MAX_KANTE, 40))
        .unwrap();
}

#[test]
fn bild_lesen_begrenzt_und_prueft() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.bild_setzen(&id, &png(100)).unwrap();
    let pfad = s.wurzel().join("bilder").join(format!("{id}.png"));
    std::fs::write(&pfad, png(BILD_MAX_BYTES + 50_000)).unwrap();
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
    std::fs::write(&pfad, png_mit(0, 5, 100)).unwrap();
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
    std::fs::write(&pfad, b"kein png").unwrap();
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
    std::fs::write(&pfad, png(BILD_MAX_BYTES)).unwrap();
    assert_eq!(s.bild_lesen(&id).unwrap().unwrap().len(), BILD_MAX_BYTES);
}

/// Blockiert das Schreiben von `profile.json`: Die Temp-Datei des atomaren Schreibens ist
/// ein Ordner, `File::create` scheitert.
fn profil_schreiben_blockieren(s: &KontoSpeicher) -> std::path::PathBuf {
    let sperre = s
        .wurzel()
        .join(format!(".profile.json.{}.tmp", std::process::id()));
    std::fs::create_dir(&sperre).unwrap();
    sperre
}

fn bild_dateien(s: &KontoSpeicher) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(s.wurzel().join("bilder")) else {
        return vec![];
    };
    let mut namen: Vec<String> = rd
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    namen.sort();
    namen
}

#[test]
fn bild_setzen_scheitert_ohne_flag_und_ohne_rest() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    let sperre = profil_schreiben_blockieren(&s);
    assert!(s.bild_setzen(&id, &png(100)).is_err());
    std::fs::remove_dir(&sperre).unwrap();
    assert!(!s.aktiv().unwrap().hat_bild);
    assert!(bild_dateien(&s).is_empty());
}

#[test]
fn bild_ersetzen_scheitert_mit_gueltigem_bild() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.bild_setzen(&id, &png(100)).unwrap();
    let sperre = profil_schreiben_blockieren(&s);
    assert!(s.bild_setzen(&id, &png(200)).is_err());
    std::fs::remove_dir(&sperre).unwrap();
    assert!(s.aktiv().unwrap().hat_bild);
    let bild = s
        .bild_lesen(&id)
        .unwrap()
        .expect("Bild muss lesbar bleiben");
    assert_eq!(bild, png(100));
    assert_eq!(bild_dateien(&s).len(), 1);
}

#[test]
fn bild_entfernen_scheitert_ohne_verlust_des_flags() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.bild_setzen(&id, &png(100)).unwrap();
    let sperre = profil_schreiben_blockieren(&s);
    assert!(s.bild_entfernen(&id).is_err());
    std::fs::remove_dir(&sperre).unwrap();
    assert!(s.aktiv().unwrap().hat_bild);
    assert!(s.bild_lesen(&id).unwrap().is_some());
    s.bild_entfernen(&id).unwrap();
    assert!(bild_dateien(&s).is_empty());
}

#[test]
fn bild_entfernen_toleriert_fehlende_datei() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.bild_setzen(&id, &png(100)).unwrap();
    std::fs::remove_file(s.wurzel().join("bilder").join(format!("{id}.png"))).unwrap();
    s.bild_entfernen(&id).unwrap();
    assert!(!s.aktiv().unwrap().hat_bild);
}

#[test]
fn anlegen_und_aktivieren_atomar() {
    let (_t, s) = speicher();
    let alt = s.aktiv().unwrap().id;
    let d0 = s.lesen().unwrap();
    let neu = s.profil_anlegen_und_aktivieren(eingabe("Neu")).unwrap();
    let d1 = s.lesen().unwrap();
    assert_eq!(d1.aktiv, neu.id);
    assert_ne!(neu.id, alt);
    assert_eq!(d1.profile.len(), 2);
    assert_eq!(d1.generation, d0.generation + 1);
    assert_eq!(d1.sitzungsrevision, d0.sitzungsrevision + 1);
    // Ungültige Eingabe: nichts ändert sich.
    assert!(s.profil_anlegen_und_aktivieren(eingabe("  ")).is_err());
    let d2 = s.lesen().unwrap();
    assert_eq!(d2.aktiv, neu.id);
    assert_eq!(d2.generation, d1.generation);
    assert_eq!(d2.profile.len(), 2);
    // `profil_anlegen` bleibt ohne Aktivierung.
    let dritt = s.profil_anlegen(eingabe("Dritt")).unwrap();
    assert_ne!(s.lesen().unwrap().aktiv, dritt.id);
}

#[test]
fn bild_entfernen_mit_haengender_datei_ist_kein_fehler() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.bild_setzen(&id, &png(100)).unwrap();
    // Löschen unmöglich: an der Stelle der Datei liegt ein nichtleerer Ordner.
    let pfad = s.wurzel().join("bilder").join(format!("{id}.png"));
    std::fs::remove_file(&pfad).unwrap();
    std::fs::create_dir(&pfad).unwrap();
    std::fs::write(pfad.join("x"), b"x").unwrap();
    s.bild_entfernen(&id).unwrap();
    assert!(!s.aktiv().unwrap().hat_bild);
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
}

#[test]
fn verwaiste_datei_ohne_flag_wird_nicht_ausgeliefert() {
    let (_t, s) = speicher();
    let id = s.aktiv().unwrap().id;
    s.bild_setzen(&id, &png(100)).unwrap();
    let kopie = s.wurzel().join("kopie.png");
    std::fs::copy(s.wurzel().join("bilder").join(format!("{id}.png")), &kopie).unwrap();
    s.bild_entfernen(&id).unwrap();
    std::fs::copy(&kopie, s.wurzel().join("bilder").join(format!("{id}.png"))).unwrap();
    assert_eq!(s.bild_lesen(&id).unwrap(), None);
    assert_eq!(bild_dateien(&s).len(), 1);
}

/// Speicher mit aktivem Erstprofil (`erst`) und einem zweiten, inaktiven Profil (`anderes`).
fn zwei_profile() -> (tempfile::TempDir, KontoSpeicher, String, String) {
    let (t, s) = speicher();
    let erst = s.aktiv().unwrap().id;
    let anderes = s.profil_anlegen(eingabe("Zweit")).unwrap().id;
    (t, s, erst, anderes)
}

#[test]
fn aendern_wenn_aktiv_erfolgreich() {
    let (_t, s, erst, _) = zwei_profile();
    let g = s.lesen().unwrap().generation;
    let p = s
        .profil_aendern_wenn_aktiv(
            &erst,
            ProfilAenderung {
                anzeigename: Some("Neu".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(p.anzeigename, "Neu");
    assert_eq!(s.lesen().unwrap().generation, g + 1);
}

#[test]
fn aendern_wenn_aktiv_lehnt_inaktives_profil_ab() {
    let (_t, s, _, anderes) = zwei_profile();
    let vorher = s.lesen().unwrap();
    let r = s.profil_aendern_wenn_aktiv(
        &anderes,
        ProfilAenderung {
            anzeigename: Some("Neu".into()),
            ..Default::default()
        },
    );
    let Err(KontoFehler::NichtAktiv(id)) = r else {
        panic!("erwartet NichtAktiv, war {r:?}");
    };
    assert_eq!(id, anderes);
    assert_eq!(s.lesen().unwrap(), vorher);
    assert!(
        KontoFehler::NichtAktiv(anderes.clone())
            .to_string()
            .contains("nicht (mehr) das aktive Profil")
    );
    // Das Original schreibt weiterhin auch für inaktive Profile.
    s.profil_aendern(
        &anderes,
        ProfilAenderung {
            anzeigename: Some("Neu".into()),
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn bild_setzen_wenn_aktiv_erfolgreich_und_nicht_aktiv_ohne_wirkung() {
    let (_t, s, erst, anderes) = zwei_profile();
    s.bild_setzen_wenn_aktiv(&erst, &png(40)).unwrap();
    assert_eq!(s.bild_lesen(&erst).unwrap().unwrap(), png(40));
    assert_eq!(bild_dateien(&s), vec![format!("{erst}.png")]);

    // Anderes Profil ohne Bild: keine Bilddatei, Datei unverändert.
    let vorher = s.lesen().unwrap();
    let r = s.bild_setzen_wenn_aktiv(&anderes, &png(50));
    assert!(matches!(r, Err(KontoFehler::NichtAktiv(ref i)) if *i == anderes));
    assert_eq!(s.lesen().unwrap(), vorher);
    assert_eq!(bild_dateien(&s), vec![format!("{erst}.png")]);
    assert!(s.bild_lesen(&anderes).unwrap().is_none());

    // Anderes Profil mit Bild: altes Bild bleibt erhalten.
    s.bild_setzen(&anderes, &png(60)).unwrap();
    let vorher = s.lesen().unwrap();
    let r = s.bild_setzen_wenn_aktiv(&anderes, &png(70));
    assert!(matches!(r, Err(KontoFehler::NichtAktiv(_))));
    assert_eq!(s.lesen().unwrap(), vorher);
    assert_eq!(s.bild_lesen(&anderes).unwrap().unwrap(), png(60));
}

#[test]
fn bild_entfernen_wenn_aktiv_erfolgreich_und_nicht_aktiv_ohne_wirkung() {
    let (_t, s, erst, anderes) = zwei_profile();
    s.bild_setzen(&erst, &png(40)).unwrap();
    s.bild_setzen(&anderes, &png(60)).unwrap();

    let vorher = s.lesen().unwrap();
    let r = s.bild_entfernen_wenn_aktiv(&anderes);
    assert!(matches!(r, Err(KontoFehler::NichtAktiv(ref i)) if *i == anderes));
    assert_eq!(s.lesen().unwrap(), vorher);
    assert_eq!(s.bild_lesen(&anderes).unwrap().unwrap(), png(60));

    s.bild_entfernen_wenn_aktiv(&erst).unwrap();
    assert!(s.bild_lesen(&erst).unwrap().is_none());
    assert_eq!(bild_dateien(&s), vec![format!("{anderes}.png")]);
}

#[test]
fn wenn_aktiv_prueft_nach_profilwechsel() {
    let (_t, s, erst, anderes) = zwei_profile();
    s.aktivieren(&anderes).unwrap();
    assert!(matches!(
        s.bild_entfernen_wenn_aktiv(&erst),
        Err(KontoFehler::NichtAktiv(_))
    ));
    s.bild_setzen_wenn_aktiv(&anderes, &png(30)).unwrap();
}
