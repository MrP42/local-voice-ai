//! lva#365 P4a: Texte, Projektdateien und KI-Kontext über den Dienst. Echter `MeetingStore` und
//! echte Projektablage in Temp-Ordnern; nur synthetische Daten.

use super::dateien;
use super::dienst::ProjektDienst;
use super::kontext::{self, Form, KontextForm};
use super::lokaler_principal;
use super::quellen::{Inhalt, ProdQuellen, SeiteKurz};
use super::recherche::RechercheStore;
use crate::managers::meetings::search::index::tests::tmp_store;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wai_projekte::Klasse;

const GEHEIM: &str = "KONTROLLTEXT-P4-91c2";

struct Env {
    _dir: tempfile::TempDir,
    _mdir: tempfile::TempDir,
    ablage: PathBuf,
    projekte: Arc<ProjektDienst>,
    seiten: Arc<Mutex<Option<Vec<SeiteKurz>>>>,
    texte: Arc<Mutex<HashMap<String, String>>>,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let (mdir, store) = tmp_store();
    let store = Arc::new(store);
    let seiten = Arc::new(Mutex::new(Some(Vec::<SeiteKurz>::new())));
    let texte = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    let (s, t) = (seiten.clone(), texte.clone());
    let quellen = Arc::new(
        ProdQuellen::neu(store, || Ok(Vec::<Inhalt>::new()))
            .mit_seiten(move || {
                s.lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| "nicht verfügbar".to_string())
            })
            .mit_texten(move |link, _w| match t.lock().unwrap().get(link) {
                Some(x) => Form::Text(x.clone()),
                None => Form::Fehlt(kontext::hinweis::KEIN_TEXT),
            }),
    );
    let ablage = dir.path().join("projekte");
    std::fs::create_dir_all(&ablage).unwrap();
    let rstore = RechercheStore::open(ablage.join("recherche.db"))
        .await
        .unwrap();
    let projekte = Arc::new(
        ProjektDienst::open(
            ablage.join("projekte.db"),
            quellen,
            rstore,
            lokaler_principal(),
        )
        .await
        .unwrap()
        .with_ablage(ablage.clone()),
    );
    Env {
        _dir: dir,
        _mdir: mdir,
        ablage,
        projekte,
        seiten,
        texte,
    }
}

fn seite(e: &Env, id: &str, text: &str) {
    e.seiten.lock().unwrap().as_mut().unwrap().push(SeiteKurz {
        id: id.into(),
        titel: format!("Text {id}"),
        lesbar: true,
    });
    e.texte
        .lock()
        .unwrap()
        .insert(format!("wai://lva/seite/{id}"), text.into());
}

async fn projekt(e: &Env) -> String {
    e.projekte
        .anlegen("Wärmeplanung", Klasse::Internal, false)
        .await
        .unwrap()
        .id
}

fn quelle(e: &Env, name: &str, inhalt: &str) -> PathBuf {
    let q = e._dir.path().join("quelle");
    std::fs::create_dir_all(&q).unwrap();
    let p = q.join(name);
    std::fs::write(&p, inhalt).unwrap();
    p
}

#[tokio::test]
async fn text_zuordnen_ist_kontext_und_ausschalten_nimmt_ihn_heraus() {
    let e = env().await;
    let p = projekt(&e).await;
    seite(&e, "s1", "Erlaubter Text");
    seite(&e, "s2", GEHEIM);
    e.projekte.seite_zuordnen(&p, "s1").await.unwrap();
    e.projekte.seite_zuordnen(&p, "s2").await.unwrap();
    let k = e.projekte.kontext(&p).await.unwrap();
    assert_eq!(k.uebersicht.count, 2, "bewusst zugeordnet: an");
    let s2 = e
        .projekte
        .eintraege(&p)
        .await
        .unwrap()
        .into_iter()
        .find(|x| x.link.as_deref() == Some("wai://lva/seite/s2"))
        .unwrap();
    let u = e
        .projekte
        .kontext_schalten(&p, &s2.id, false, None, None)
        .await
        .unwrap();
    assert_ne!(u.stand, k.uebersicht.stand);
    let k2 = e.projekte.kontext(&p).await.unwrap();
    assert!(!kontext::als_text(&k2).contains(GEHEIM));
    assert_eq!(k2.uebersicht.count, 1);
    // Erneutes Zuordnen schaltet ein bewusstes „aus“ nicht wieder ein.
    e.projekte.seite_zuordnen(&p, "s2").await.unwrap();
    assert!(!kontext::als_text(&e.projekte.kontext(&p).await.unwrap()).contains(GEHEIM));
    // Falsche Form wird abgewiesen.
    assert!(e
        .projekte
        .kontext_schalten(&p, &s2.id, true, Some(KontextForm::Kurzbuch), None)
        .await
        .is_err());
}

#[tokio::test]
async fn nicht_verfuegbarer_seitenbestand_entfernt_keine_mitgliedschaft() {
    let e = env().await;
    let p = projekt(&e).await;
    seite(&e, "s1", "Text");
    e.projekte.seite_zuordnen(&p, "s1").await.unwrap();
    // Bestand vorübergehend nicht lesbar: der Abgleich darf nicht entfernen.
    *e.seiten.lock().unwrap() = None;
    assert!(e.projekte.abgleichen().await.is_err());
    *e.seiten.lock().unwrap() = Some(vec![]);
    // Bestand gelesen, Seite weg: jetzt ist sie verwaist.
    assert_eq!(e.projekte.abgleichen().await.unwrap(), 1);
}

#[tokio::test]
async fn datei_hinzufuegen_kontext_und_entfernen_in_den_papierkorb() {
    let e = env().await;
    let p = projekt(&e).await;
    let ids = e
        .projekte
        .dateien_hinzufuegen(
            &p,
            &[
                quelle(&e, "notiz.md", GEHEIM),
                quelle(&e, "plan.pdf", "%PDF"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    let liste = e.projekte.eintraege(&p).await.unwrap();
    assert_eq!(liste.len(), 2);
    let k = e.projekte.kontext(&p).await.unwrap();
    assert_eq!(k.uebersicht.count, 1);
    assert!(kontext::als_text(&k).contains(GEHEIM));
    let pdf = k
        .uebersicht
        .rows
        .iter()
        .find(|r| r.hint.as_deref() == Some(kontext::hinweis::FORMAT));
    assert!(pdf.is_some(), "PDF nur mit Formathinweis");
    assert!(e.projekte.datei_entfernen(&p, &ids[0]).await.unwrap());
    let liste = e.projekte.eintraege(&p).await.unwrap();
    assert_eq!(liste.len(), 1);
    let korb = e.ablage.join(&p).join(dateien::PAPIERKORB);
    assert_eq!(std::fs::read_dir(korb).unwrap().count(), 1);
    assert!(!kontext::als_text(&e.projekte.kontext(&p).await.unwrap()).contains(GEHEIM));
}

#[tokio::test]
async fn absturz_vor_dem_eintragen_wird_beim_wiederanlauf_eingetragen() {
    let e = env().await;
    let p = projekt(&e).await;
    // Schritte 1–4 ohne den Dienst: Datei abgelegt, keine Mitgliedschaft.
    dateien::hinzufuegen(&e.ablage, &p, &quelle(&e, "a.txt", "a")).unwrap();
    // Absturz zwischen Ablegen und Journal: Datei ohne Eintrag.
    std::fs::write(e.ablage.join(&p).join(dateien::DATEIEN).join("b.txt"), "b").unwrap();
    assert!(e.projekte.eintraege(&p).await.unwrap().is_empty());
    assert_eq!(e.projekte.dateien_wiederanlauf().await.unwrap(), 1);
    let liste = e.projekte.eintraege(&p).await.unwrap();
    assert_eq!(liste.len(), 1, "nur die abgelegte, nicht die gefundene");
    let gefunden = e.projekte.gefundene_dateien(&p).await.unwrap();
    assert_eq!(gefunden.len(), 1);
    e.projekte
        .gefundene_datei_aufnehmen(&p, &gefunden[0].id)
        .await
        .unwrap();
    assert_eq!(e.projekte.eintraege(&p).await.unwrap().len(), 2);
    // Zweiter Lauf: nichts zu tun.
    assert_eq!(e.projekte.dateien_wiederanlauf().await.unwrap(), 0);
}

#[tokio::test]
async fn projekt_loeschen_legt_den_ordner_in_den_papierkorb() {
    let e = env().await;
    let p = projekt(&e).await;
    e.projekte
        .dateien_hinzufuegen(&p, &[quelle(&e, "a.txt", "a")])
        .await
        .unwrap();
    e.projekte.loeschen(&p).await.unwrap();
    assert!(!e.ablage.join(&p).exists());
    assert_eq!(
        std::fs::read_dir(e.ablage.join(dateien::PROJEKT_PAPIERKORB))
            .unwrap()
            .count(),
        1
    );
    // Verwaister Ordner (Absturz nach dem Löschen in der Datenbank) wird nachgeräumt.
    let q = projekt(&e).await;
    e.projekte
        .dateien_hinzufuegen(&q, &[quelle(&e, "b.txt", "b")])
        .await
        .unwrap();
    e.projekte
        .verz_fuer_test()
        .projekt_loeschen(&lokaler_principal(), &q)
        .await
        .unwrap();
    assert_eq!(
        e.projekte.verwaiste_projektordner_raeumen().await.unwrap(),
        1
    );
    assert!(!e.ablage.join(&q).exists());
}

#[tokio::test]
async fn fremde_projektdatei_liefert_keinen_text() {
    let e = env().await;
    let p = projekt(&e).await;
    let q = projekt(&e).await;
    let ids = e
        .projekte
        .dateien_hinzufuegen(&q, &[quelle(&e, "g.md", GEHEIM)])
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    let k = e.projekte.kontext(&p).await.unwrap();
    assert!(k.texte.is_empty());
}

#[tokio::test]
async fn projekt_ki_pfade_liefern_ohne_p4c_freigabe_nichts() {
    let e = env().await;
    let p = projekt(&e).await;
    seite(&e, "s1", GEHEIM);
    e.projekte.seite_zuordnen(&p, "s1").await.unwrap();
    // Die Übersicht zeigt den Eintrag (ohne Inhalt) …
    let u = e.projekte.kontext_uebersicht(&p).await.unwrap();
    assert_eq!(u.count, 1);
    assert!(!serde_json::to_string(&u).unwrap().contains(GEHEIM));
    // … der KI-Weg liefert vor P4c nichts.
    let f = e.projekte.kontext_fuer_ki(&p).await;
    assert!(f.is_err());
    assert!(!format!("{:?}", f.err()).contains(GEHEIM));
}
