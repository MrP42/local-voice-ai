use std::sync::mpsc;
use std::time::Duration;
use wai_konto::{
    Anmeldung, BILD_MAX_BYTES, KontoFehler, KontoSpeicher, ProfilAenderung, ProfilEingabe,
};

fn png(n: usize) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.resize(n, 7);
    v
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
        s.bild_setzen(&id, &png(8)),
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
