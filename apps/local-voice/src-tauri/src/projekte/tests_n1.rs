//! Gegenproben für Quellenraum, Auswahl und lokale Versionen (lva#622).

use super::kontext::{self, Form, KontextForm, Mitglied, Schalter, Sorte};
use super::quellenraum::{self, QuellenArt, QuellenZustand};
use super::tests_p4c::{buch, env, projekt, seite, seite_text};
use std::fs;
use wai_projekte::Klasse;

const DATEI: &str = "quellenversionen.json";
const P: &str = "prj_01HZZZZZZZZZZZZZZZZZZZZZZZ";

fn mitglied(id: &str, sorte: Sorte, pfad: &str) -> Mitglied {
    Mitglied {
        id: id.into(),
        art: "document".into(),
        sorte,
        titel: String::new(),
        link: format!("wai://lva/{pfad}/{id}"),
        lesbar: true,
        klasse: Klasse::Internal,
    }
}

#[tokio::test]
async fn quellenraum_version_folgt_aenderungen_auch_zurueck_zum_alten_text() {
    let e = env().await;
    seite(&e, "s1", "Erster Text");
    let p = projekt(&e, Klasse::Personal).await;
    e.dienst.seite_zuordnen(&p, "s1").await.unwrap();
    let a = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(a.sources[0].version, Some(1));
    let b = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(a, b);
    seite_text(&e, "s1", "Zweiter Text");
    let c = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(c.sources[0].version, Some(2));
    assert_eq!(c.sources[0].version_count, 2);
    assert_ne!(a.sources[0].version_hash, c.sources[0].version_hash);
    seite_text(&e, "s1", "Erster Text");
    let d = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(d.sources[0].version, Some(3));
    assert_eq!(d.sources[0].version_count, 3);
    assert_eq!(a.sources[0].version_hash, d.sources[0].version_hash);
}

#[tokio::test]
async fn quellenraum_auswahl_wird_erneut_gelesen_und_kontrolltext_bleibt_intern() {
    let e = env().await;
    seite(&e, "s1", "GEHEIM");
    let p = projekt(&e, Klasse::Personal).await;
    e.dienst.seite_zuordnen(&p, "s1").await.unwrap();
    let id = e.dienst.eintraege(&p).await.unwrap()[0].id.clone();
    for an in [true, false] {
        e.dienst
            .kontext_schalten(&p, &id, an, None, None)
            .await
            .unwrap();
        let sicht = e.dienst.quellenraum(&p).await.unwrap();
        assert_eq!(sicht.sources[0].selected, an);
        assert_eq!(sicht.selected_count, u32::from(an));
        assert!(!serde_json::to_string(&sicht).unwrap().contains("GEHEIM"));
    }
    let k = e.dienst.kontext_fuer_test(&p).await.unwrap();
    assert!(!kontext::als_text(&k).contains("GEHEIM"));
}

#[tokio::test]
async fn quellenraum_schalterdatei_ueberlebt_neues_laden() {
    let e = env().await;
    let m = mitglied("s1", Sorte::Sonst, "seite");
    for an in [true, false] {
        kontext::setzen(&e.ordner, P, &m, an, None, None).unwrap();
        assert!(e.ordner.join(kontext::DATEI).is_file());
        assert_eq!(Schalter::laden(&e.ordner).wahl(P, &m).an, an);
        let k = kontext::bauen(
            P,
            std::slice::from_ref(&m),
            &Schalter::laden(&e.ordner),
            &mut |_, _| Form::Text("Kontrolle".into()),
        );
        let sicht = quellenraum::sicht_bauen(
            &e.ordner,
            std::slice::from_ref(&m),
            &k.uebersicht,
            &mut |_, _| Form::Text("Kontrolle".into()),
        );
        assert_eq!(sicht.sources[0].selected, an);
        assert_eq!(sicht.selected_count, u32::from(an));
    }
}

#[tokio::test]
async fn quellenraum_entfernen_behaelt_original_und_version() {
    let e = env().await;
    buch(&e, "b1", Some("Kurzbuch"), "Volltext");
    let p = projekt(&e, Klasse::Personal).await;
    e.dienst.buch_zuordnen(&p, "b1").await.unwrap();
    let vorher = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(vorher.sources[0].version, Some(1));
    e.dienst
        .kontext_schalten(
            &p,
            &vorher.sources[0].id,
            true,
            Some(KontextForm::Volltext),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        e.dienst.quellenraum(&p).await.unwrap().sources[0].version,
        Some(2)
    );
    assert!(e.dienst.buch_entfernen(&p, "b1").await.unwrap());
    assert_eq!(e.dienst.quellenraum(&p).await.unwrap().total, 0);
    e.dienst.buch_zuordnen(&p, "b1").await.unwrap();
    let nachher = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(nachher.sources[0].version, Some(3));
    assert_eq!(nachher.sources[0].version_count, 3);
}

#[tokio::test]
async fn quellenraum_gemischter_dienst_buch_und_text() {
    let e = env().await;
    buch(&e, "b1", Some("Kurzbuch"), "Volltext");
    seite(&e, "s1", "Text");
    let p = projekt(&e, Klasse::Personal).await;
    e.dienst.buch_zuordnen(&p, "b1").await.unwrap();
    e.dienst.seite_zuordnen(&p, "s1").await.unwrap();
    let sicht = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(sicht.total, 2);
    assert!(sicht.sources.iter().any(|z| z.art == QuellenArt::Buch));
    assert!(sicht.sources.iter().any(|z| z.art == QuellenArt::Text));
}

#[tokio::test]
async fn quellenraum_buch_ohne_kurzbuch_hat_keine_version() {
    let e = env().await;
    buch(&e, "b2", None, "Volltext");
    let p = projekt(&e, Klasse::Personal).await;
    e.dienst.buch_zuordnen(&p, "b2").await.unwrap();
    let sicht = e.dienst.quellenraum(&p).await.unwrap();
    assert_eq!(sicht.sources[0].state, QuellenZustand::KeinText);
    assert_eq!(sicht.sources[0].version, None);
}

#[tokio::test]
async fn quellenraum_gemischte_arten_und_nicht_lesbare_quelle() {
    let e = env().await;
    let mut ms = vec![
        mitglied("b1", Sorte::Buch, "buch"),
        mitglied("s1", Sorte::Sonst, "seite"),
        mitglied("d1", Sorte::Ablage, "ablage"),
    ];
    let schalter = Schalter::mit(P, &[(&ms[0], true, None), (&ms[1], true, None)]);
    let k = kontext::bauen(P, &ms, &schalter, &mut |_, _| Form::Text("Text".into()));
    let sicht = quellenraum::sicht_bauen(&e.ordner, &ms, &k.uebersicht, &mut |m, w| {
        assert!(w.an);
        assert_eq!(w.fassung, None);
        assert_ne!(m.sorte, Sorte::Ablage, "Auszüge brauchen keinen Leser");
        Form::Text("Text".into())
    });
    assert_eq!(sicht.total, 3);
    assert_eq!(sicht.sources[0].art, QuellenArt::Buch);
    assert_eq!(sicht.sources[1].art, QuellenArt::Text);
    assert_eq!(sicht.sources[2].art, QuellenArt::Ablage);
    assert_eq!(sicht.sources[2].state, QuellenZustand::Bereit);
    assert_eq!(sicht.sources[2].version, None);
    assert_eq!(sicht.sources[0].form, Some(KontextForm::Kurzbuch));
    ms[0].lesbar = false;
    let sicht = quellenraum::sicht_bauen(&e.ordner, &ms, &k.uebersicht, &mut |m, _| {
        assert_ne!(m.id, "b1", "Unlesbare Quellen werden nicht gelesen");
        Form::Auszuege
    });
    assert_eq!(sicht.sources[0].state, QuellenZustand::NichtVerfuegbar);
    assert_eq!(sicht.sources[0].title, None);
    assert_eq!(sicht.sources[0].version, None);
    assert!(!sicht.sources[0].selected);
    assert_eq!(sicht.selected_count, 1);
}

#[tokio::test]
async fn quellenraum_versionsdatei_defekt_grenze_und_lesen_ohne_schreiben() {
    let e = env().await;
    let datei = e.ordner.join(DATEI);
    let erster_hash = quellenraum::inhalt_hash("Erster Text");
    for defekt in ["{kaputt", r#"{"v":2,"quellen":{}}"#] {
        fs::write(&datei, defekt).unwrap();
        assert_eq!(quellenraum::lesen(&e.ordner, "test"), None);
        assert_eq!(fs::read_to_string(&datei).unwrap(), defekt);
        assert_eq!(
            quellenraum::erfassen(&e.ordner, "test", &erster_hash).unwrap(),
            (1, 1)
        );
    }
    let sicherungen = fs::read_dir(&e.ordner)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("quellenversionen.json.defekt-")
        })
        .count();
    assert_eq!(sicherungen, 2);
    for i in 2..=55 {
        let hash = quellenraum::inhalt_hash(&format!("Text {i}"));
        assert_eq!(
            quellenraum::erfassen(&e.ordner, "test", &hash).unwrap(),
            (i, i.min(50))
        );
    }
    assert_eq!(quellenraum::lesen(&e.ordner, "test"), Some((55, 50)));
    // Nicht kanonische Leerzeichen machen selbst ein inhaltsgleiches Neuschreiben sichtbar.
    let roh = format!("{}\n\n", fs::read_to_string(&datei).unwrap());
    fs::write(&datei, &roh).unwrap();
    let hash = quellenraum::inhalt_hash("Text 55");
    assert_eq!(
        quellenraum::erfassen(&e.ordner, "test", &hash).unwrap(),
        (55, 50)
    );
    assert_eq!(fs::read_to_string(&datei).unwrap(), roh);
    assert_eq!(
        quellenraum::inhalt_hash("ä"),
        quellenraum::inhalt_hash("a\u{308}")
    );
    assert_eq!(
        quellenraum::inhalt_hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223"
    );
}

#[tokio::test]
async fn quellenraum_speicherfehler_verhindert_keine_sicht() {
    let e = env().await;
    let blockiert = e.ordner.join("kein-ordner");
    fs::write(&blockiert, "Kontrolle").unwrap();
    let m = mitglied("s1", Sorte::Sonst, "seite");
    let k = kontext::bauen(
        P,
        std::slice::from_ref(&m),
        &Schalter::laden(&e.ordner),
        &mut |_, _| Form::Text("Text".into()),
    );
    let sicht = quellenraum::sicht_bauen(&blockiert, &[m], &k.uebersicht, &mut |_, _| {
        Form::Text("Text".into())
    });
    assert_eq!(sicht.sources[0].state, QuellenZustand::Bereit);
    assert_eq!(sicht.sources[0].version, None);
    assert_eq!(sicht.stand, k.uebersicht.stand);
    assert_eq!(sicht.reset, k.uebersicht.reset);
}

#[test]
fn quellenraum_verwendet_keinen_ki_zugang() {
    let verboten = concat!("kontext_fuer", "_ki(");
    assert!(!include_str!("quellenraum.rs").contains(verboten));
    let dienst = include_str!("dienst.rs");
    let funktion = dienst
        .split("pub async fn quellenraum(")
        .nth(1)
        .expect("Dienstfunktion fehlt")
        .split("\n    /// Einziger Zugang")
        .next()
        .unwrap();
    assert!(!funktion.contains(verboten));
    assert!(funktion.contains("self.kontext_uebersicht(suite_id)"));
}
