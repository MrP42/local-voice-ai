//! lva#365 P4a: Texte, Projektdateien und KI-Kontext über den Dienst. Echter `MeetingStore` und
//! echte Projektablage in Temp-Ordnern; nur synthetische Daten.

use super::dateien;
use super::dienst::ProjektDienst;
use super::kontext::{self, Form, KontextForm};
use super::lokaler_principal;
use super::quellen::{Inhalt, ProdQuellen, SeiteKurz};
use super::recherche::RechercheStore;
use crate::managers::meetings::search::index::tests::tmp_store;
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
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
    let k = e.projekte.kontext_fuer_test(&p).await.unwrap();
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
    let k2 = e.projekte.kontext_fuer_test(&p).await.unwrap();
    assert!(!kontext::als_text(&k2).contains(GEHEIM));
    assert_eq!(k2.uebersicht.count, 1);
    // Erneutes Zuordnen schaltet ein bewusstes „aus“ nicht wieder ein.
    e.projekte.seite_zuordnen(&p, "s2").await.unwrap();
    assert!(!kontext::als_text(&e.projekte.kontext_fuer_test(&p).await.unwrap()).contains(GEHEIM));
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
    let k = e.projekte.kontext_fuer_test(&p).await.unwrap();
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
    assert!(!kontext::als_text(&e.projekte.kontext_fuer_test(&p).await.unwrap()).contains(GEHEIM));
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
    let k = e.projekte.kontext_fuer_test(&p).await.unwrap();
    assert!(k.texte.is_empty());
}

/// Freischaltung (P4c-4): Die Sperre ist aufgehoben, und trotzdem gibt es Kontext nur über das Versandtor.
/// Was der Dienst sonst nach außen gibt (Übersicht, Einträge, Projekte), trägt keinen Inhalt; den Text der
/// Einträge liefert allein `kontext_fuer_ki`, und das ruft nur das Tor (Quelltest `ohne_tor_kein_kontext_im_code`).
/// Die Abnahme setzt die Freischaltung voraus; bei gesetztem Rückbau (`PROJEKT_KI_FREIGEGEBEN = false`)
/// gilt die Gegenprobe: `kontext_fuer_ki` liefert dann nur einen Fehler.
#[tokio::test]
async fn ohne_tor_kein_kontext() {
    let e = env().await;
    let p = projekt(&e).await;
    seite(&e, "s1", GEHEIM);
    e.projekte.seite_zuordnen(&p, "s1").await.unwrap();
    // Die Übersicht zeigt den Eintrag (ohne Inhalt) …
    let u = e.projekte.kontext_uebersicht(&p).await.unwrap();
    assert_eq!(u.count, 1);
    // … ebenso Einträge und Projektliste: alles, was nach außen geht, ist frei von Inhalten.
    let nach_aussen = serde_json::to_string(&(
        &u,
        e.projekte.eintraege(&p).await.unwrap(),
        e.projekte.projekte().await.unwrap(),
    ))
    .unwrap();
    assert!(!nach_aussen.contains(GEHEIM), "Inhalt ohne Tor sichtbar");
    let k = e.projekte.kontext_fuer_ki(&p).await;
    if super::dienst::ki_freigegeben() {
        // Mit dem Tor (und nur so) kommt der Text an: Gegenprobe, dass der Test etwas sehen könnte.
        assert!(kontext::als_text(&k.unwrap()).contains(GEHEIM));
    } else {
        assert!(k.is_err(), "Rückbau: kein Kontext, auch nicht für das Tor");
    }
    // Ein Fehler trägt nie Inhalt.
    let f = e.projekte.kontext_fuer_ki("kein-projekt").await;
    assert!(f.is_err());
    assert!(!format!("{:?}", f.err()).contains(GEHEIM));
}

/// Der Rückbau bleibt ein Wert: Die Sperre steht als eine Konstante (`bool`) in `dienst.rs`, und
/// `kontext_fuer_ki` prüft sie vor dem Lesen jedes Inhalts. Der Test hängt nicht am Wert der Konstante.
#[test]
fn rueckbau_ist_eine_konstante_vor_dem_lesen() {
    let q = include_str!("dienst.rs").replace("\r\n", "\n");
    let konstante = Regex::new(r"const PROJEKT_KI_FREIGEGEBEN: bool = (true|false);").unwrap();
    assert_eq!(konstante.find_iter(&q).count(), 1, "genau eine Konstante");
    assert_eq!(q.matches("const PROJEKT_KI_FREIGEGEBEN").count(), 1);
    let f = &q[q.find("pub(super) async fn kontext_fuer_ki(").unwrap()..];
    let f = &f[..f.find("\n    }\n").unwrap()];
    assert!(f.find("PROJEKT_KI_FREIGEGEBEN").unwrap() < f.find("kontext_bauen(").unwrap());
}

/// Allowlist: den Kontext mit Inhalten baut nur `kontext_bauen` (privat in `dienst.rs`), und das
/// rufen nur die erlaubten Zugänge auf.
#[test]
fn kontext_mit_inhalten_nur_ueber_erlaubte_zugaenge() {
    const ERLAUBT: [&str; 4] = [
        "kontext_bauen",
        "kontext_uebersicht",
        "kontext_fuer_ki",
        "kontext_fuer_test",
    ];
    let quelle = include_str!("dienst.rs");
    let mut treffer = 0;
    for (pos, _) in quelle.match_indices("kontext_bauen(") {
        let davor = &quelle[..pos];
        let start = davor.rfind("fn ").expect("umschließende Funktion") + 3;
        let name: String = quelle[start..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        assert!(ERLAUBT.contains(&name.as_str()), "kontext_bauen aus {name}");
        treffer += 1;
    }
    assert_eq!(treffer, 4, "Definition und drei erlaubte Aufrufe");
    assert!(
        quelle.contains("    async fn kontext_bauen("),
        "kontext_bauen bleibt privat"
    );
    // Der volle Kontext ist sonst nirgends öffentlich.
    assert!(!quelle.contains("pub async fn kontext("));
    assert!(!quelle.contains("pub(super) async fn kontext("));
    assert!(!quelle.contains("pub(crate) async fn kontext("));
}

// ---------------------------------------------------------------- Quelltests über alle Wege (P4c-4)
//
// Die Wege: Chat und Sprache (W1, W2: `voice_call/projekt.rs`), Recherche (`recherche.rs`), KI-Dokument
// (`dokument.rs`) und Stapel (`tor.rs`, `einzellauf`). Jeder Modellaufruf im Projekt steht hinter
// `vor_versand`; jeder andere Modellaufruf der App kennt den Projektkontext nicht.

/// Ist die Datei eine Testdatei? `tests.rs`, `tests_*.rs`, `*_tests.rs` und alles unter `tests/`, dazu jede
/// Datei, die ihr Elternmodul unter `#[cfg(test)]` einbindet (`testkit.rs`, `test_support.rs`, Quelltests).
/// Bewusst nicht jede Datei mit „test“ im Namen: `selftest.rs` und `agent_bridge/test_tools.rs` sind
/// Produktivcode und gehören geprüft.
fn ist_testdatei(rel: &str, alle: &BTreeMap<String, String>) -> bool {
    let teile: Vec<&str> = rel.split('/').collect();
    let (datei, ordner) = teile.split_last().unwrap();
    let stamm = datei.trim_end_matches(".rs");
    if ordner.contains(&"tests")
        || stamm == "tests"
        || stamm.starts_with("tests_")
        || stamm.ends_with("_tests")
    {
        return true;
    }
    let eltern: Vec<String> = if ordner.is_empty() {
        vec!["lib.rs".into(), "main.rs".into()]
    } else {
        let dir = ordner.join("/");
        vec![format!("{dir}.rs"), format!("{dir}/mod.rs")]
    };
    let muster = Regex::new(&format!(
        r"#\[cfg\(test\)\]\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+{}\s*;",
        regex::escape(stamm)
    ))
    .unwrap();
    eltern
        .iter()
        .filter_map(|e| alle.get(e))
        .any(|src| muster.is_match(src))
}

/// Alle Quelldateien der App (ohne Testdateien) als `(Pfad, Code)`. Der Code ist ohne den eingebetteten
/// Testteil und ohne Kommentarzeilen; Zeilenenden sind vereinheitlicht (ein Windows-Checkout liefert `\r\n`).
fn quelldateien() -> Vec<(String, String)> {
    fn sammeln(dir: &std::path::Path, aus: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                sammeln(&p, aus);
            } else if p.extension().is_some_and(|x| x == "rs") {
                aus.push(p);
            }
        }
    }
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pfade = Vec::new();
    sammeln(&src, &mut pfade);
    let alle: BTreeMap<String, String> = pfade
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let roh = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("{rel}: {e}"))
                .replace("\r\n", "\n");
            (rel, roh)
        })
        .collect();
    alle.iter()
        .filter(|(rel, _)| !ist_testdatei(rel, &alle))
        .map(|(rel, roh)| {
            let ohne_tests = roh
                .split("\n#[cfg(test)]\nmod tests {")
                .next()
                .unwrap_or(roh);
            let code = ohne_tests
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            (rel.clone(), code)
        })
        .collect()
}

/// Die Auswahl der Quelldateien trifft Produktivcode mit „test“ im Namen und lässt die Testdateien weg.
#[test]
fn quelldateien_nehmen_produktivcode_mit_test_im_namen_und_keine_testdateien() {
    let namen: Vec<String> = quelldateien().into_iter().map(|(r, _)| r).collect();
    for da in [
        "selftest.rs",
        "agent_bridge/test_tools.rs",
        "projekte/tor.rs",
    ] {
        assert!(namen.iter().any(|n| n == da), "{da} fehlt");
    }
    for weg in [
        "projekte/tests_p4.rs",
        "projekte/tests.rs",
        "voice_call/projekt_quelltests.rs",
        "voice_call/projekt/tests.rs",
        "agent_bridge/testkit.rs",
        "commands/voice_tools/projekt_tests.rs",
    ] {
        assert!(!namen.iter().any(|n| n == weg), "{weg} ist Testcode");
    }
}

fn code_von<'a>(dateien: &'a [(String, String)], rel: &str) -> &'a str {
    dateien
        .iter()
        .find(|(r, _)| r == rel)
        .unwrap_or_else(|| panic!("{rel} fehlt"))
        .1
        .as_str()
}

/// Der Rumpf der Funktion `name`: von `fn name(` bis zur schließenden Klammer in derselben Einrückung.
fn rumpf<'a>(code: &'a str, name: &str) -> &'a str {
    let start = code
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("fn {name} fehlt"));
    let zeile = code[..start].rfind('\n').map_or(0, |i| i + 1);
    let einzug = " ".repeat(code[zeile..start].len() - code[zeile..start].trim_start().len());
    let ende = code[start..]
        .find(&format!("\n{einzug}}}\n"))
        .map_or(code.len(), |e| start + e + 1);
    &code[zeile..ende]
}

/// Ohne Tor kein Kontext, im Code: Den Kontexttext baut und liefert nur das Tor, und kein öffentlicher Zugang
/// des Dienstes gibt einen Kontext mit Inhalten heraus (`kontext_fuer_ki` ist `pub(super)`). Die Prüfung
/// geht über Wörter statt über Pfade (`kontext::als_text(`), damit eine Umbenennung beim Import sie nicht
/// umgeht: jeder Aufruf von `als_text(`, jeder `Kontext>` oder `Kontext,` in einer Signatur, jedes
/// `KiBereich::Projekt(`.
#[test]
fn ohne_tor_kein_kontext_im_code() {
    let dateien = quelldateien();
    let aufruf = Regex::new(r"\bals_text\s*\(").unwrap();
    let alias = Regex::new(r"\bals_text\s+as\b").unwrap();
    // Aufrufe, nicht die Definitionen (`fn als_text(`).
    let mit_text: Vec<&str> = dateien
        .iter()
        .filter(|(_, c)| {
            aufruf
                .find_iter(c)
                .any(|m| !c[..m.start()].trim_end().ends_with("fn"))
        })
        .map(|(r, _)| r.as_str())
        .collect();
    assert_eq!(mit_text, vec!["projekte/tor.rs"], "Kontexttext nur im Tor");
    for (rel, code) in &dateien {
        assert!(
            !alias.is_match(code),
            "{rel}: `als_text` unter anderem Namen"
        );
    }
    // Der Kontexttyp mit Inhalten (nicht der gleichnamige des Sprachdienstes) steht nur im Dienst, im
    // Tor und in seinem eigenen Modul, und nie in einer öffentlichen Signatur von Dienst oder Tor.
    let typ = Regex::new(r"\bKontext\s*[>,]|\bKontext\s+as\b").unwrap();
    for (rel, code) in &dateien {
        let betrifft_projekte = rel.starts_with("projekte/") || code.contains("projekte::");
        for zeile in code.lines().filter(|z| typ.is_match(z)) {
            if !betrifft_projekte {
                continue;
            }
            assert!(
                [
                    "projekte/dienst.rs",
                    "projekte/tor.rs",
                    "projekte/kontext.rs"
                ]
                .contains(&rel.as_str()),
                "{rel}: Kontext mit Inhalten nur in Dienst, Tor und kontext.rs: {zeile}"
            );
            let z = zeile.trim_start();
            if rel == "projekte/kontext.rs" {
                continue;
            }
            assert!(
                !z.starts_with("pub async fn")
                    && !z.starts_with("pub fn")
                    && !z.starts_with("pub(crate)"),
                "{rel}: Kontext mit Inhalten darf nicht öffentlich sein: {z}"
            );
        }
    }
    // Der Projektbereich der Ablage-Auszüge wird nur im Tor aufgebaut (in `ablage/ki.rs` steht der Typ selbst).
    let bereich: Vec<&str> = dateien
        .iter()
        .filter(|(r, c)| r != "ablage/ki.rs" && c.contains("KiBereich::Projekt("))
        .map(|(r, _)| r.as_str())
        .collect();
    assert_eq!(
        bereich,
        vec!["projekte/tor.rs"],
        "KiBereich::Projekt nur im Tor"
    );
}

/// Allowlist der Modellaufrufe (`send_chat_completion*`): Jede Datei der App mit einem direkten Aufruf ist
/// hier eingeordnet, entweder als Projektweg (hinter dem Tor) oder als Aufrufer ohne Projektkontext (der das
/// Projektmodul nicht kennt). Ein neuer Aufruf in einer unbekannten Datei lässt den Test scheitern.
#[test]
fn allowlist_modellaufrufe_alle_wege_ueber_das_tor() {
    const PROJEKTWEGE: [&str; 4] = [
        "voice_call/mod.rs",
        "voice_call/turn.rs",
        "voice_call/projekt.rs",
        "projekte/zustand.rs",
    ];
    const OHNE_PROJEKTKONTEXT: [&str; 10] = [
        "actions.rs",
        "book_ai/talk/mod.rs",
        "books.rs",
        "managers/meetings/chat/controller.rs",
        "managers/meetings/followup.rs",
        "managers/meetings/llm_call.rs",
        "summarizer.rs",
        "sync/booktalk_relay.rs",
        "tagging.rs",
        "translator.rs",
    ];
    let dateien = quelldateien();
    let mit_aufruf: Vec<&str> = dateien
        .iter()
        // `llm_versand/` ist der Transport und die Übergangsfassade selbst (lva#604), kein Aufrufer.
        .filter(|(r, c)| {
            r != "llm_client.rs"
                && !r.starts_with("llm_versand/")
                && c.contains("send_chat_completion")
        })
        .map(|(r, _)| r.as_str())
        .collect();
    for rel in &mit_aufruf {
        assert!(
            PROJEKTWEGE.contains(rel) || OHNE_PROJEKTKONTEXT.contains(rel),
            "{rel}: neuer Modellaufruf. Hier einordnen: Projektweg (hinter vor_versand) oder ohne Projektkontext"
        );
    }
    for rel in PROJEKTWEGE.iter().chain(OHNE_PROJEKTKONTEXT.iter()) {
        assert!(
            mit_aufruf.contains(rel),
            "{rel} ruft kein send_chat_completion* mehr: die Liste pflegen"
        );
    }
    // Aufrufer ohne Projektkontext kennen weder das Tor noch den Kontext.
    for rel in OHNE_PROJEKTKONTEXT {
        let code = code_von(&dateien, rel);
        for verboten in [
            "projekte::",
            "wai_projekte",
            "ProjektTor",
            "kontext_fuer_ki",
            "KontextQuelle",
        ] {
            assert!(!code.contains(verboten), "{rel}: `{verboten}` ohne Tor");
        }
    }
}

/// Im Projektzweig steht `vor_versand` vor jedem Modellaufruf: Chat und Sprache (die Zweige kehren vor dem
/// gewöhnlichen Versand zurück, der Sendeweg des Projekts steht hinter `vor_versand`), KI-Dokument und
/// Stapel (`einzellauf`), Recherche (die Modellquelle der Einstellungen gilt nur ohne Projektkontext).
#[test]
fn projektzweig_vor_versand_steht_vor_jedem_modellaufruf() {
    let dateien = quelldateien();
    // W1: Chat mit Texteingabe und Sprachgespräch Stufe 1.
    let r = rumpf(code_von(&dateien, "voice_call/mod.rs"), "voice_call_chat");
    let zweig = r.find("return projekt::chat(").expect("Projektzweig fehlt");
    assert!(zweig < r.find("send_chat_completion_stream_routed(").unwrap());
    // W2: Sprachgespräch Stufe 2 (Sprecher und Denker).
    let r = rumpf(code_von(&dateien, "voice_call/turn.rs"), "voice_call_turn");
    let zweig = r.find("projekt::durchgang(").expect("Projektzweig fehlt");
    assert!(zweig < r.find("send_chat_completion_stream(").unwrap());
    // Der Kern beider: erst das Tor, dann der Versand; der einzige Sendeweg des Projekts ist `LlmWeg`.
    let projekt = code_von(&dateien, "voice_call/projekt.rs");
    let kern = rumpf(projekt, "durchlauf_kern");
    assert!(kern.find(".vor_versand(").unwrap() < kern.find("weg.senden(").unwrap());
    assert_eq!(projekt.matches("send_chat_completion").count(), 1);
    assert!(
        projekt.find("impl Modellweg for LlmWeg").unwrap()
            < projekt.find("send_chat_completion").unwrap()
    );
    // KI-Dokument und Stapel: derselbe Einzellauf, der Versand steht hinter dem Tor.
    let tor = code_von(&dateien, "projekte/tor.rs");
    let lauf = rumpf(tor, "einzellauf");
    assert!(lauf.find("self.vor_versand(").unwrap() < lauf.find("versand.senden(").unwrap());
    assert!(rumpf(tor, "stapel").contains("self.einzellauf("));
    let dokument = code_von(&dateien, "projekte/dokument.rs");
    assert!(dokument.contains(".einzellauf("));
    assert!(!dokument.contains("send_chat_completion"));
    // Recherche: mit Projektkontext das Ziel aus dem Tor, die Modellquelle der Einstellungen nur ohne.
    // Die Reihenfolge sichert der Typ: `weg_modell` nimmt ein `FreigegebenesZiel`, das nur eine Freigabe
    // oder Fortsetzung des Tors herausgibt (Feld privat in `tor.rs`). Ein Test auf Dateipositionen wäre
    // hier falsch-grün (`weg_modell` steht ohnehin hinter dem Halt); geprüft wird darum die Form.
    let rech = code_von(&dateien, "projekte/recherche.rs");
    assert!(!rech.contains("send_chat_completion"));
    assert!(
        rech.contains("fn weg_modell(weg: Arc<dyn Modellweg>, ziel: FreigegebenesZiel)"),
        "weg_modell nimmt nur ein vom Tor freigegebenes Ziel"
    );
    assert_eq!(rech.matches("weg.senden(").count(), 1, "ein Sendeweg");
    assert!(rumpf(rech, "weg_modell").contains("weg.senden("));
    assert!(
        !rech.contains("Ziel {"),
        "kein Ziel-Literal in der Recherche"
    );
    // Beide Aufrufer (Lauf mit der Freigabe `f`, Fortsetzen mit der `fortsetzung`) holen das Ziel aus dem Tor
    // und reichen genau dieses an `weg_modell`.
    let beleg = Regex::new(r"let (\w+) = (f|fortsetzung)\.freigegebenes_ziel\(\);").unwrap();
    let belege: Vec<(String, String)> = beleg
        .captures_iter(rech)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let woher: Vec<&str> = belege.iter().map(|(_, q)| q.as_str()).collect();
    assert_eq!(woher, vec!["f", "fortsetzung"], "Freigabe und Fortsetzung");
    assert_eq!(
        rech.matches("weg_modell(").count(),
        3,
        "Definition und zwei Aufrufe"
    );
    for (var, _) in &belege {
        assert!(
            rech.contains(&format!("weg_modell(ki.weg.clone(), {var})")),
            "weg_modell bekommt das Ziel aus dem Tor ({var})"
        );
    }
    let ohne = rech
        .match_indices("self.modelle.modell(")
        .collect::<Vec<_>>();
    assert_eq!(ohne.len(), 2, "Lauf und Fortsetzen");
    for (pos, _) in ohne {
        assert!(
            rech[pos.saturating_sub(60)..pos].contains("None =>"),
            "die Modellquelle der Einstellungen gilt nur ohne Projektkontext"
        );
    }
    // Jeder Weg des Tors hat genau eine Stelle im Code, die ihn eröffnet (Chat und Sprache teilen sich
    // `Weg::Sprache`).
    for (weg, rel) in [
        ("Weg::Sprache", "voice_call/projekt.rs"),
        ("Weg::Recherche", "projekte/recherche.rs"),
        ("Weg::Dokument", "projekte/dokument.rs"),
        ("Weg::Stapel", "projekte/tor.rs"),
    ] {
        let stellen: Vec<&str> = dateien
            .iter()
            .filter(|(_, c)| c.contains(&format!("weg: {weg},")))
            .map(|(r, _)| r.as_str())
            .collect();
        assert_eq!(stellen, vec![rel], "{weg}");
    }
}
