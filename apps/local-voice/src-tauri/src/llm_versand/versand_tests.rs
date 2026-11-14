//! Tests von `versenden` (lva#604, Teilpaket 3): Prüfung bei jedem Versuch, Streamingpuffer,
//! Fallbacks, Fehlerkontrakt, Buchung und der Rahmen für Wiederholungen.
//!
//! Aufbau: Die Cloud ist ein lokaler Testserver, an den ein Cloud-Ziel über den Testhaken
//! [`testhaken::setze_cloud_proxy`] geht (kein TLS, keine Umgebungsvariable). Compliance und
//! Budget zählt der Regelwerk-Haken; die Besprechungsprüfung läuft gegen die Testquelle des
//! Prüfers. Deren Lesungen sind im ersten Versuch R1 = 1, R2 = 2, R3 = 3, im Folgeversuch
//! 4, 5, 6. Der Haken der Quelle schiebt an genau einer Stelle einen Widerruf ein oder
//! beobachtet, was bis dahin ausgegeben wurde. Kein Schlaf als Mittel der Synchronisation.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::llm_client::{mit_denksenke, ChatReply};
use crate::llm_versand::fehler::{cli_code, CODE_BESPRECHUNG_CLOUD_GESPERRT};
use crate::llm_versand::pruefer::TestQuelle;
use crate::llm_versand::testhilfe::{
    anbieter as provider, mock, sse_denken, sse_text, Mock, MockAnfrage, RohAntwort,
};
use crate::managers::meetings::ki_freigabe::KiSperre;

const CLOUD_BASIS: &str = "http://cloud.example/v1";
const GEHEIM: &str = "GEHEIM-ANBIETERTEXT-4711";

// -- Aufbau -------------------------------------------------------------------------------------

fn cloud() -> (Ziel, PostProcessProvider) {
    (
        Ziel::http("custom", CLOUD_BASIS, "m"),
        provider("custom", CLOUD_BASIS),
    )
}

fn lokal_bei(server: &Mock) -> (Ziel, PostProcessProvider) {
    (
        Ziel::http("custom", &server.basis(), "m"),
        provider("custom", &server.basis()),
    )
}

fn cli() -> (Ziel, PostProcessProvider) {
    (
        Ziel::cli("codex-cli", "codex", "m"),
        provider("codex-cli", "cli://codex"),
    )
}

fn richtlinie(nur_lokal: bool) -> KiSperre {
    KiSperre::Pruefen {
        kundenprofil_nur_lokal: nur_lokal,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into()],
    }
}

/// Eine Quelle, in der die Besprechung `m1` freigegeben ist.
fn quelle_mit_m1() -> TestQuelle {
    let quelle = TestQuelle::neu(1, richtlinie(false));
    quelle.freigeben("m1");
    quelle
}

fn besprechung() -> Quellen {
    Quellen::besprechung("m1")
}

fn gesperrt(grund: &'static str) -> VersandFehler {
    VersandFehler::gesperrt(grund)
}

/// Ein Testserver, an den Cloud-Ziele dieses Tests gehen.
async fn cloud_server(
    handler: impl Fn(&MockAnfrage) -> RohAntwort + Send + Sync + 'static,
) -> Mock {
    let server = mock(handler).await;
    testhaken::setze_cloud_proxy(&server.adresse());
    server
}

/// Alles, was Rückrufe eines Aufrufs ausgeben, in der Reihenfolge des Eintreffens
/// (`T:` Textstück, `D:` Denktext).
#[derive(Clone, Default)]
struct Ausgaben(Arc<Mutex<Vec<String>>>);

impl Ausgaben {
    fn text(&self) -> impl Fn(&str) + Send + Sync + 'static {
        let ziel = Arc::clone(&self.0);
        move |stueck: &str| ziel.lock().unwrap().push(format!("T:{stueck}"))
    }

    fn denken(&self) -> DenkSenke {
        let ziel = Arc::clone(&self.0);
        Arc::new(move |stueck: &str| ziel.lock().unwrap().push(format!("D:{stueck}")))
    }

    fn log(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn anzahl(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// Zählt, wie oft Compliance und Budget geprüft wurden.
#[derive(Clone, Default)]
struct Pruefungen {
    compliance: Rc<Cell<usize>>,
    budget: Rc<Cell<usize>>,
}

impl Pruefungen {
    fn anzahl(&self) -> (usize, usize) {
        (self.compliance.get(), self.budget.get())
    }
}

/// Setzt den Regelwerk-Haken: er zählt beide Prüfungen und verweigert höchstens eine davon.
/// Das Budget wird nur geprüft, wenn Compliance bestanden ist (wie im echten Ablauf).
fn regelwerk(verweigert: Option<&'static str>) -> Pruefungen {
    let zaehler = Pruefungen::default();
    let gemerkt = zaehler.clone();
    testhaken::setze_regelwerk(move |art, _anbieter, _modell| {
        let zelle = if art == "compliance" {
            &gemerkt.compliance
        } else {
            &gemerkt.budget
        };
        zelle.set(zelle.get() + 1);
        if verweigert == Some(art) {
            Err(format!("{art} verweigert"))
        } else {
            Ok(())
        }
    });
    zaehler
}

/// Der Haken der Quelle: bei der Lesung `nr_ziel` wird die Freigabe von `m1` widerrufen.
fn widerrufen_bei(quelle: &TestQuelle, nr_ziel: usize) {
    quelle.bei_lesung(move |quelle, nr| {
        if nr == nr_ziel {
            quelle.widerrufen("m1");
        }
    });
}

/// Der Haken der Quelle: bei der Lesung `nr_ziel` wird festgehalten, wie viele Ausgaben schon
/// hinausgegangen sind.
fn ausgaben_bei(quelle: &TestQuelle, nr_ziel: usize, aus: &Ausgaben) -> Arc<Mutex<Option<usize>>> {
    let gesehen = Arc::new(Mutex::new(None));
    let (ziel, aus) = (Arc::clone(&gesehen), aus.clone());
    quelle.bei_lesung(move |_, nr| {
        if nr == nr_ziel {
            *ziel.lock().unwrap() = Some(aus.anzahl());
        }
    });
    gesehen
}

fn chat_anfrage(anbieter: &PostProcessProvider) -> Anfrage<'_> {
    Anfrage {
        zweck: Purpose::Chat,
        anbieter,
        api_key: String::new(),
        inhalt: Inhalt::Chat(ChatInhalt {
            nutzer: "Frage".into(),
            system: None,
            schema: None,
            reasoning_effort: None,
            reasoning: None,
        }),
    }
}

fn strom_anfrage<'a>(
    anbieter: &'a PostProcessProvider,
    frage: &str,
    on_delta: &'a (dyn Fn(&str) + Send + Sync),
) -> Anfrage<'a> {
    Anfrage {
        zweck: Purpose::Chat,
        anbieter,
        api_key: String::new(),
        inhalt: Inhalt::Strom {
            nachrichten: vec![StreamMessage::new("user", frage)],
            grenzen: StreamLimits::default(),
            on_delta,
        },
    }
}

/// Ein Strom mit Textstücken und Denktext, mit Denksenke und allen Ausgaben in `aus`.
async fn strom_mit_denken(
    quelle: &TestQuelle,
    ziel: &Ziel,
    quellen: &Quellen,
    anbieter: &PostProcessProvider,
    aus: &Ausgaben,
) -> Result<Antwort, VersandFehler> {
    let on_delta = aus.text();
    mit_denksenke(
        Some(aus.denken()),
        versenden(
            quelle,
            ziel,
            quellen,
            strom_anfrage(anbieter, "Frage", &on_delta),
        ),
    )
    .await
}

/// Denken `d1`, Text `a`, Denken `d2`, Text `b`, Ende.
fn strom_gemischt() -> RohAntwort {
    let mut stuecke = sse_denken(&["d1"]);
    stuecke.extend(sse_text(&["a"]));
    stuecke.extend(sse_denken(&["d2"]));
    stuecke.extend(sse_text(&["b"]));
    stuecke.push(b"data: [DONE]\n\n".to_vec());
    RohAntwort::neu(200, "text/event-stream", stuecke)
}

/// Führt `fut` in einem Erfassungsbereich aus und liefert, was dabei gebucht wurde.
async fn gebucht<F: Future>(fut: F) -> (F::Output, Vec<usage::CapturedCall>) {
    usage::with_capture(async {
        let ergebnis = fut.await;
        (ergebnis, usage::captured_calls().0)
    })
    .await
}

fn antwort_cli(inhalt: &str) -> ChatReply {
    ChatReply {
        content: Some(inhalt.into()),
        truncated: false,
    }
}

fn verbrauch(prompt: u64, completion: u64) -> TokenUsage {
    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
    }
}

// -- Streamingpuffer ----------------------------------------------------------------------------

#[tokio::test]
async fn cloud_mit_besprechung_haelt_alles_bis_zum_abschlussvergleich_zurueck() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| strom_gemischt()).await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let aus = Ausgaben::default();
    let bei_r3 = ausgaben_bei(&quelle, 3, &aus);

    let antwort = strom_mit_denken(&quelle, &ziel, &besprechung(), &anbieter, &aus)
        .await
        .expect("freigegeben");

    assert_eq!(
        antwort.text.as_deref(),
        Some("ab"),
        "Denktext ist nicht Teil der Antwort"
    );
    assert_eq!(
        *bei_r3.lock().unwrap(),
        Some(0),
        "vor dem Abschlussvergleich ging weder Text noch Denktext hinaus"
    );
    assert_eq!(
        aus.log(),
        ["D:d1", "T:a", "D:d2", "T:b"],
        "danach alles, in der Reihenfolge des Eintreffens"
    );
    assert_eq!(quelle.lesungen(), 3);
    assert_eq!(server.treffer(), 1);
    assert_eq!(pruefungen.anzahl(), (1, 1));
}

#[tokio::test]
async fn ein_widerruf_vor_dem_abschlussvergleich_verwirft_die_ganze_ausgabe() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| strom_gemischt()).await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 3);
    let aus = Ausgaben::default();

    let ergebnis = strom_mit_denken(&quelle, &ziel, &besprechung(), &anbieter, &aus).await;

    assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
    assert!(aus.log().is_empty(), "nichts darf hinaus: {:?}", aus.log());
    assert_eq!(
        server.treffer(),
        1,
        "der Request ging hinaus, sein Ergebnis nicht"
    );
    assert_eq!(pruefungen.anzahl(), (1, 1));
}

#[tokio::test]
async fn ohne_freigabe_geht_nichts_hinaus_und_nichts_wird_geprueft() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| strom_gemischt()).await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = TestQuelle::neu(1, richtlinie(false));
    let aus = Ausgaben::default();

    let (ergebnis, gebucht) = gebucht(strom_mit_denken(
        &quelle,
        &ziel,
        &besprechung(),
        &anbieter,
        &aus,
    ))
    .await;

    assert_eq!(ergebnis, Err(gesperrt(grund::NICHT_SICHTBAR)));
    assert_eq!(server.treffer(), 0, "kein Request");
    assert_eq!(
        pruefungen.anzahl(),
        (0, 0),
        "Gesperrt hat Vorrang vor Compliance und Budget"
    );
    assert!(aus.log().is_empty());
    assert!(gebucht.is_empty(), "es wurde nichts verbraucht");
}

const STUECKE: usize = 12;

/// Was ein Strom aus [`STUECKE`] Textstücken und `[DONE]` über seine Ausgabe verrät.
struct Beobachtung {
    /// Wie viele Stücke der Server geschrieben hatte (oder gerade schrieb), als das erste
    /// Textstück beim Rückruf ankam. Live: klein. Gepuffert: alle.
    erste_bei: usize,
    pruefungen: (usize, usize),
    lesungen: usize,
    text: String,
}

async fn beobachten(
    lokal: bool,
    quelle: &TestQuelle,
    quellen: &Quellen,
    uebergang: bool,
) -> Beobachtung {
    let _wache = testhaken::wache();
    let gesendet = Arc::new(AtomicUsize::new(0));
    let server = {
        let gesendet = Arc::clone(&gesendet);
        mock(move |_| {
            let gesendet = Arc::clone(&gesendet);
            let teile: Vec<String> = (0..STUECKE).map(|i| format!("t{i} ")).collect();
            let teile: Vec<&str> = teile.iter().map(String::as_str).collect();
            RohAntwort::strom(&teile)
                .mit_vor_stueck(move |nr| gesendet.store(nr + 1, Ordering::SeqCst))
        })
        .await
    };
    let (ziel, anbieter) = if lokal {
        lokal_bei(&server)
    } else {
        testhaken::setze_cloud_proxy(&server.adresse());
        cloud()
    };
    let pruefungen = regelwerk(None);
    let erste = Arc::new(Mutex::new(None));
    let on_delta = {
        let (erste, gesendet) = (Arc::clone(&erste), Arc::clone(&gesendet));
        move |_: &str| {
            let mut erste = erste.lock().unwrap();
            if erste.is_none() {
                *erste = Some(gesendet.load(Ordering::SeqCst));
            }
        }
    };
    let anfrage = strom_anfrage(&anbieter, "Frage", &on_delta);
    let antwort = if uebergang {
        ausfuehren(Pruefung::Uebergang, &ziel, quellen, anfrage).await
    } else {
        versenden(quelle, &ziel, quellen, anfrage).await
    }
    .expect("Antwort");
    let erste_bei = erste.lock().unwrap().expect("kein Textstück angekommen");
    Beobachtung {
        erste_bei,
        pruefungen: pruefungen.anzahl(),
        lesungen: quelle.lesungen(),
        text: antwort.text.unwrap_or_default(),
    }
}

#[tokio::test]
async fn cloud_mit_besprechung_gibt_erst_nach_dem_ende_des_stroms_aus() {
    let quelle = quelle_mit_m1();
    let b = beobachten(false, &quelle, &besprechung(), false).await;
    assert_eq!(
        b.erste_bei,
        STUECKE + 1,
        "das erste Stück kam erst an, als der Server alles (samt [DONE]) geschickt hatte"
    );
    assert_eq!(b.lesungen, 3);
    assert_eq!(b.pruefungen, (1, 1));
    assert!(b.text.starts_with("t0 t1 ") && b.text.ends_with("t11 "));
}

#[tokio::test]
async fn lokale_ziele_streamen_stueck_fuer_stueck_und_ohne_lesung() {
    // Nicht freigegeben, nicht einmal sichtbar: lokal ist das gleichgültig.
    let quelle = TestQuelle::neu(1, richtlinie(true));
    let b = beobachten(true, &quelle, &besprechung(), false).await;
    assert!(
        b.erste_bei <= STUECKE / 2,
        "das erste Stück kam live an (Server bei {} von {})",
        b.erste_bei,
        STUECKE + 1
    );
    assert_eq!(b.lesungen, 0, "lokal liest nichts");
    assert_eq!(
        b.pruefungen,
        (1, 1),
        "Compliance und Budget laufen auch lokal"
    );
}

#[tokio::test]
async fn cloud_ohne_besprechungsquellen_streamt_live_und_prueft_trotzdem_regelwerk_und_budget() {
    let quelle = TestQuelle::neu(1, richtlinie(true));
    let b = beobachten(false, &quelle, &Quellen::keine(), false).await;
    assert!(
        b.erste_bei <= STUECKE / 2,
        "live, Server bei {}",
        b.erste_bei
    );
    assert_eq!(b.lesungen, 0, "keine() braucht keine Besprechungsprüfung");
    assert_eq!(b.pruefungen, (1, 1));
}

#[tokio::test]
async fn die_uebergangsfassade_streamt_unveraendert_live_ohne_puffer_und_ohne_lesung() {
    let quelle = TestQuelle::neu(1, richtlinie(false));
    let b = beobachten(false, &quelle, &Quellen::unbestimmt(), true).await;
    assert!(
        b.erste_bei <= STUECKE / 2,
        "live, Server bei {}",
        b.erste_bei
    );
    assert_eq!(b.lesungen, 0);
    assert_eq!(b.pruefungen, (1, 1));
}

#[test]
fn nur_der_volle_vertrag_bei_cloud_mit_quellen_puffert() {
    let quelle = quelle_mit_m1();
    let voll = Pruefung::Voll(&quelle);
    let lokal = Ziel::http("ollama", "http://127.0.0.1:11434/v1", "m");
    let (cloud, _) = cloud();
    assert!(voll.puffert(&cloud, &besprechung()));
    assert!(voll.puffert(&cloud, &Quellen::unbestimmt()));
    assert!(!voll.puffert(&cloud, &Quellen::keine()));
    assert!(!voll.puffert(&lokal, &besprechung()));
    assert!(!Pruefung::Uebergang.puffert(&cloud, &besprechung()));
    assert!(!Pruefung::Uebergang.puffert(&cloud, &Quellen::unbestimmt()));
}

#[tokio::test]
async fn codex_denkzeilen_werden_wie_text_bis_zum_abschlussvergleich_zurueckgehalten() {
    let _wache = testhaken::wache();
    regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    let aus = Ausgaben::default();
    let bei_r3 = ausgaben_bei(&quelle, 3, &aus);
    testhaken::setze_cli(|_, senke| {
        let senke = senke.expect("ein Empfänger für Denktext wurde gesetzt");
        senke("denke 1");
        senke("denke 2");
        Ok((antwort_cli("fertig"), verbrauch(4, 1)))
    });

    let antwort = mit_denksenke(
        Some(aus.denken()),
        versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)),
    )
    .await
    .expect("freigegeben");

    assert_eq!(antwort.text.as_deref(), Some("fertig"));
    assert_eq!(*bei_r3.lock().unwrap(), Some(0), "vor R3 nichts");
    assert_eq!(aus.log(), ["D:denke 1", "D:denke 2"]);
    assert_eq!(quelle.lesungen(), 3);
}

#[tokio::test]
async fn bei_widerruf_kommen_auch_codex_denkzeilen_nie_an() {
    let _wache = testhaken::wache();
    regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 3);
    let aus = Ausgaben::default();
    testhaken::setze_cli(|_, senke| {
        senke.expect("Empfänger")("geheimes Denken");
        Ok((antwort_cli("fertig"), verbrauch(4, 1)))
    });

    let ergebnis = mit_denksenke(
        Some(aus.denken()),
        versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)),
    )
    .await;

    assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
    assert!(aus.log().is_empty(), "{:?}", aus.log());
}

#[tokio::test]
async fn ohne_empfaenger_fuer_denktext_wird_auch_nichts_zurueckgehalten() {
    let _wache = testhaken::wache();
    regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    testhaken::setze_cli(|_, senke| {
        assert!(senke.is_none(), "ohne Empfänger gibt es keine Senke");
        Ok((antwort_cli("fertig"), verbrauch(1, 1)))
    });
    let antwort = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter))
        .await
        .unwrap();
    assert_eq!(antwort.text.as_deref(), Some("fertig"));
}

// -- Puffer als Einheit -------------------------------------------------------------------------

#[test]
fn der_denktextpuffer_ist_begrenzt_und_zaehlt_was_er_verwirft() {
    let laengen = Arc::new(Mutex::new(Vec::<usize>::new()));
    let texte = Arc::new(Mutex::new(Vec::<String>::new()));
    let senke: DenkSenke = {
        let laengen = Arc::clone(&laengen);
        Arc::new(move |s: &str| laengen.lock().unwrap().push(s.len()))
    };
    let on_delta = {
        let texte = Arc::clone(&texte);
        move |s: &str| texte.lock().unwrap().push(s.to_owned())
    };
    let ausgabe = Versuchsausgabe::neu(true, &on_delta, Some(senke));
    let denken = ausgabe.denksenke().expect("gepufferte Senke");

    denken(&"x".repeat(PUFFER_DENKEN_MAX - 1));
    denken("ab"); // würde die Grenze überschreiten: verworfen
    denken("c"); // passt genau
    denken("d"); // darüber: verworfen
    (ausgabe.ausgabe().on_delta)("Text bleibt unbehelligt");

    {
        let puffer = ausgabe.puffer.as_ref().unwrap().lock().unwrap();
        assert_eq!(puffer.denken_bytes, PUFFER_DENKEN_MAX);
        assert_eq!(
            puffer.denken_verworfen, 2,
            "verworfene Stücke werden gezählt"
        );
    }
    assert!(
        laengen.lock().unwrap().is_empty(),
        "vor der Freigabe nichts"
    );
    ausgabe.freigeben();
    assert_eq!(*laengen.lock().unwrap(), vec![PUFFER_DENKEN_MAX - 1, 1]);
    assert_eq!(*texte.lock().unwrap(), vec!["Text bleibt unbehelligt"]);
}

#[test]
fn ungepuffert_geht_alles_sofort_an_die_empfaenger_und_freigeben_tut_nichts() {
    let log = Ausgaben::default();
    let on_delta = log.text();
    let ausgabe = Versuchsausgabe::neu(false, &on_delta, Some(log.denken()));
    (ausgabe.ausgabe().on_delta)("a");
    ausgabe.denksenke().expect("Senke")("d");
    assert_eq!(log.log(), ["T:a", "D:d"]);
    ausgabe.freigeben();
    assert_eq!(log.log(), ["T:a", "D:d"], "nichts doppelt");
}

#[test]
fn eine_vergiftete_puffersperre_verwirft_die_ausgabe_statt_zu_paniken() {
    let log = Ausgaben::default();
    let on_delta = log.text();
    let ausgabe = Versuchsausgabe::neu(true, &on_delta, Some(log.denken()));
    let puffer = Arc::clone(ausgabe.puffer.as_ref().unwrap());
    let _ = std::thread::spawn(move || {
        let _gehalten = puffer.lock().unwrap();
        panic!("Absicht: die Sperre vergiften");
    })
    .join();
    assert!(ausgabe.puffer.as_ref().unwrap().is_poisoned());

    (ausgabe.ausgabe().on_delta)("a");
    ausgabe.denksenke().expect("Senke")("d");
    ausgabe.freigeben();
    assert!(log.log().is_empty(), "sicherer Weg: nichts ausgeben");
}

// -- Fallbacks ----------------------------------------------------------------------------------

/// Der erste Request (Strom) wird abgelehnt, der zweite (ohne Strom) beantwortet.
fn strom_abgelehnt_dann_antwort() -> impl Fn(&MockAnfrage) -> RohAntwort + Send + Sync + 'static {
    |anfrage| {
        if anfrage.nr == 0 {
            RohAntwort::text(400, "stream not supported")
        } else {
            RohAntwort::json("Rückfall", "stop", 5, 1)
        }
    }
}

#[tokio::test]
async fn nach_einem_widerruf_gibt_es_keinen_zweiten_request_ohne_strom() {
    let _wache = testhaken::wache();
    let server = cloud_server(strom_abgelehnt_dann_antwort()).await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 3);
    let aus = Ausgaben::default();

    let ergebnis = strom_mit_denken(&quelle, &ziel, &besprechung(), &anbieter, &aus).await;

    assert_eq!(
        ergebnis,
        Err(gesperrt(grund::WIDERRUF_NACH_START)),
        "Gesperrt ist terminal"
    );
    assert_eq!(server.treffer(), 1, "kein Fallback nach Gesperrt");
    assert_eq!(quelle.lesungen(), 3, "kein neuer Prüfversuch");
    assert_eq!(pruefungen.anzahl(), (1, 1));
    assert!(aus.log().is_empty());
}

#[tokio::test]
async fn der_rueckfall_ohne_strom_ist_ein_neuer_versuch_mit_eigener_pruefung() {
    let _wache = testhaken::wache();
    let server = cloud_server(strom_abgelehnt_dann_antwort()).await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let aus = Ausgaben::default();

    let (antwort, gebucht) = gebucht(strom_mit_denken(
        &quelle,
        &ziel,
        &besprechung(),
        &anbieter,
        &aus,
    ))
    .await;

    assert_eq!(antwort.unwrap().text.as_deref(), Some("Rückfall"));
    assert_eq!(server.treffer(), 2);
    assert_eq!(quelle.lesungen(), 6, "R1 bis R3 je Versuch");
    assert_eq!(
        pruefungen.anzahl(),
        (2, 2),
        "Compliance und Budget bei jedem Versuch"
    );
    assert_eq!(aus.log(), ["T:Rückfall"]);
    assert_eq!(gebucht.len(), 1, "ein Aufruf, eine Buchung");
    assert_eq!(
        (gebucht[0].prompt_tokens, gebucht[0].completion_tokens),
        (5, 1)
    );
    assert!(gebucht[0].ok);
}

#[tokio::test]
async fn lokal_und_ohne_quellen_prueft_der_rueckfall_regelwerk_und_budget_erneut_aber_liest_nichts()
{
    for lokal in [true, false] {
        let _wache = testhaken::wache();
        let server = mock(strom_abgelehnt_dann_antwort()).await;
        let (ziel, anbieter, quellen) = if lokal {
            let (ziel, anbieter) = lokal_bei(&server);
            (ziel, anbieter, besprechung())
        } else {
            testhaken::setze_cloud_proxy(&server.adresse());
            let (ziel, anbieter) = cloud();
            (ziel, anbieter, Quellen::keine())
        };
        let pruefungen = regelwerk(None);
        let quelle = TestQuelle::neu(1, richtlinie(true));
        let aus = Ausgaben::default();

        let antwort = strom_mit_denken(&quelle, &ziel, &quellen, &anbieter, &aus)
            .await
            .unwrap();

        assert_eq!(antwort.text.as_deref(), Some("Rückfall"), "lokal={lokal}");
        assert_eq!(server.treffer(), 2, "lokal={lokal}");
        assert_eq!(quelle.lesungen(), 0, "lokal={lokal}");
        assert_eq!(pruefungen.anzahl(), (2, 2), "lokal={lokal}");
    }
}

#[tokio::test]
async fn ein_widerruf_beim_rueckfallversuch_verwirft_dessen_ergebnis_und_bucht_den_verbrauch() {
    let _wache = testhaken::wache();
    let server = cloud_server(strom_abgelehnt_dann_antwort()).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 6);
    let aus = Ausgaben::default();

    let (ergebnis, gebucht) = gebucht(strom_mit_denken(
        &quelle,
        &ziel,
        &besprechung(),
        &anbieter,
        &aus,
    ))
    .await;

    assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
    assert_eq!(server.treffer(), 2);
    assert!(
        aus.log().is_empty(),
        "auch die Antwort des Rückfalls geht nicht hinaus"
    );
    assert_eq!(gebucht.len(), 1);
    assert!(!gebucht[0].ok);
    assert_eq!(
        (gebucht[0].prompt_tokens, gebucht[0].completion_tokens),
        (5, 1),
        "der Verbrauch des verworfenen Versuchs zählt"
    );
}

#[tokio::test]
async fn ein_regelwerk_das_den_rueckfall_sperrt_beendet_den_aufruf_ohne_zweiten_request() {
    let _wache = testhaken::wache();
    let server = cloud_server(strom_abgelehnt_dann_antwort()).await;
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    // Compliance besteht beim ersten Versuch und verweigert beim zweiten.
    let aufrufe = Rc::new(Cell::new(0usize));
    {
        let aufrufe = Rc::clone(&aufrufe);
        testhaken::setze_regelwerk(move |art, _, _| {
            if art != "compliance" {
                return Ok(());
            }
            aufrufe.set(aufrufe.get() + 1);
            if aufrufe.get() >= 2 {
                Err("Profil gewechselt".into())
            } else {
                Ok(())
            }
        });
    }
    let aus = Ausgaben::default();

    let ergebnis = strom_mit_denken(&quelle, &ziel, &besprechung(), &anbieter, &aus).await;

    assert!(
        matches!(ergebnis, Err(VersandFehler::Compliance { .. })),
        "{ergebnis:?}"
    );
    assert_eq!(server.treffer(), 1, "der zweite Versuch ging nicht hinaus");
}

// -- CLI: Update und Neuversuch -----------------------------------------------------------------

/// Setzt Haken: die CLI meldet beim ersten Versuch `CliVeraltet`; das Update liefert `update`.
/// Gibt zurück: Zähler der Updates.
fn cli_veraltet_dann_antwort(update: bool) -> Rc<Cell<usize>> {
    let updates = Rc::new(Cell::new(0usize));
    testhaken::setze_cli(|nr, _| {
        if nr == 0 {
            Err(Fehlschlag::CliVeraltet)
        } else {
            Ok((antwort_cli("nach dem Update"), verbrauch(3, 2)))
        }
    });
    let gezaehlt = Rc::clone(&updates);
    testhaken::setze_aktualisierer(move || {
        gezaehlt.set(gezaehlt.get() + 1);
        update
    });
    updates
}

#[tokio::test]
async fn nach_einem_widerruf_gibt_es_kein_update_und_keinen_neuen_cli_versuch() {
    let _wache = testhaken::wache();
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 3);
    let updates = cli_veraltet_dann_antwort(true);

    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;

    assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
    assert_eq!(updates.get(), 0, "kein Update nach Gesperrt");
    assert_eq!(testhaken::cli_versuche(), 1, "kein zweiter Versuch");
    assert_eq!(quelle.lesungen(), 3);
    assert_eq!(pruefungen.anzahl(), (1, 1));
}

#[tokio::test]
async fn nach_dem_update_ist_der_neuversuch_ein_neuer_versuch_mit_eigener_pruefung() {
    let _wache = testhaken::wache();
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    let updates = cli_veraltet_dann_antwort(true);

    let (antwort, gebucht) = gebucht(versenden(
        &quelle,
        &ziel,
        &besprechung(),
        chat_anfrage(&anbieter),
    ))
    .await;

    assert_eq!(antwort.unwrap().text.as_deref(), Some("nach dem Update"));
    assert_eq!(updates.get(), 1);
    assert_eq!(testhaken::cli_versuche(), 2);
    assert_eq!(quelle.lesungen(), 6, "R1 bis R3 je Versuch");
    assert_eq!(pruefungen.anzahl(), (2, 2));
    assert_eq!(gebucht.len(), 1);
    assert_eq!(
        (gebucht[0].prompt_tokens, gebucht[0].completion_tokens),
        (3, 2)
    );
}

#[tokio::test]
async fn scheitert_das_update_bleibt_es_bei_einem_versuch_und_cli_update() {
    let _wache = testhaken::wache();
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    let updates = cli_veraltet_dann_antwort(false);

    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;

    assert_eq!(ergebnis, Err(VersandFehler::CliUpdate));
    assert_eq!(updates.get(), 1);
    assert_eq!(testhaken::cli_versuche(), 1);
    assert_eq!(pruefungen.anzahl(), (1, 1));
}

#[tokio::test]
async fn eine_cli_die_auch_nach_dem_update_zu_alt_ist_wird_nur_einmal_aktualisiert() {
    let _wache = testhaken::wache();
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    testhaken::setze_cli(|_, _| Err(Fehlschlag::CliVeraltet));
    let updates = Rc::new(Cell::new(0usize));
    let gezaehlt = Rc::clone(&updates);
    testhaken::setze_aktualisierer(move || {
        gezaehlt.set(gezaehlt.get() + 1);
        true
    });

    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;

    assert_eq!(ergebnis, Err(VersandFehler::CliUpdate));
    assert_eq!(updates.get(), 1, "höchstens ein Update je Aufruf");
    assert_eq!(testhaken::cli_versuche(), 2);
    assert_eq!(quelle.lesungen(), 6);
    assert_eq!(pruefungen.anzahl(), (2, 2));
}

#[tokio::test]
async fn ein_regelwerk_das_den_neuversuch_sperrt_hindert_den_zweiten_cli_start() {
    let _wache = testhaken::wache();
    let (ziel, anbieter) = cli();
    let quelle = quelle_mit_m1();
    let aufrufe = Rc::new(Cell::new(0usize));
    {
        let aufrufe = Rc::clone(&aufrufe);
        testhaken::setze_regelwerk(move |art, _, _| {
            if art == "budget" {
                aufrufe.set(aufrufe.get() + 1);
                if aufrufe.get() >= 2 {
                    return Err("Budget aufgebraucht".into());
                }
            }
            Ok(())
        });
    }
    cli_veraltet_dann_antwort(true);

    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;

    assert!(
        matches!(ergebnis, Err(VersandFehler::Budget { .. })),
        "{ergebnis:?}"
    );
    assert_eq!(
        testhaken::cli_versuche(),
        1,
        "der Neuversuch startete die CLI nicht"
    );
}

#[tokio::test]
async fn auch_nach_einem_fehler_der_cli_laeuft_der_abschlussvergleich_und_die_buchung() {
    let fehlerarten = [
        VersandFehler::Cli {
            code: cli_code::FEHLT,
            hinweis: None,
        },
        VersandFehler::Absturz,
        VersandFehler::Transport,
        VersandFehler::Zeitueberschreitung,
    ];
    for fehler in fehlerarten {
        // Ohne Widerruf: der Fehler des Transports bleibt, R3 lief, gebucht ist er auch.
        let _wache = testhaken::wache();
        regelwerk(None);
        let (ziel, anbieter) = cli();
        let quelle = quelle_mit_m1();
        let gemeldet = fehler.clone();
        testhaken::setze_cli(move |_, _| Err(Fehlschlag::Fehler(gemeldet.clone())));
        let (ergebnis, gebucht) = gebucht(versenden(
            &quelle,
            &ziel,
            &besprechung(),
            chat_anfrage(&anbieter),
        ))
        .await;
        assert_eq!(ergebnis, Err(fehler.clone()));
        assert_eq!(quelle.lesungen(), 3, "R3 auch nach {fehler:?}");
        assert_eq!(gebucht.len(), 1, "{fehler:?}");
        assert!(!gebucht[0].ok);

        // Mit Widerruf: nicht der Fehler des Transports, sondern Gesperrt.
        let quelle = quelle_mit_m1();
        widerrufen_bei(&quelle, 3);
        let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;
        assert_eq!(
            ergebnis,
            Err(gesperrt(grund::WIDERRUF_NACH_START)),
            "{fehler:?}"
        );
    }
}

// -- Fehlerkontrakt -----------------------------------------------------------------------------

#[tokio::test]
async fn ein_kontextueberlauf_bleibt_durch_versenden_erkennbar() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| {
        RohAntwort::neu(
            400,
            "application/json",
            vec![format!(
                r#"{{"error":{{"type":"exceed_context_size_error","message":"{GEHEIM} exceeds the available context size"}}}}"#
            )
            .into_bytes()],
        )
    })
    .await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();

    let fehler = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter))
        .await
        .unwrap_err();

    assert_eq!(fehler, VersandFehler::Kontextueberlauf);
    let text = fehler.alte_textform();
    assert!(
        crate::managers::meetings::llm_call::is_truncation_error(&text),
        "{text}"
    );
    assert!(
        crate::managers::meetings::llm_call::is_splittable_error(&text),
        "{text}"
    );
    assert!(!text.contains(GEHEIM));
    assert_eq!(server.treffer(), 1, "kein Fallback, kein zweiter Request");
    assert_eq!(quelle.lesungen(), 3);
}

#[tokio::test]
async fn anbietertext_gelangt_auch_durch_versenden_in_keinen_fehler() {
    let _wache = testhaken::wache();
    cloud_server(|_| RohAntwort::text(503, GEHEIM)).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();

    let fehler = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter))
        .await
        .unwrap_err();

    assert_eq!(fehler, VersandFehler::Http { status: 503 });
    let alles = format!("{fehler:?} | {fehler} | {}", fehler.alte_textform());
    assert!(!alles.contains(GEHEIM), "{alles}");
    assert!(alles.contains("503"));
}

// -- Regelwerk und Budget -----------------------------------------------------------------------

#[tokio::test]
async fn compliance_und_budget_laufen_bei_jedem_ziel_und_jeder_quellenart() {
    let _wache = testhaken::wache();
    let server = mock(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    testhaken::setze_cloud_proxy(&server.adresse());
    let pruefungen = regelwerk(None);
    let quelle = quelle_mit_m1();
    let (lokal, lokaler_anbieter) = lokal_bei(&server);
    let (cloud_ziel, cloud_anbieter) = cloud();

    let faelle: [(&Ziel, &PostProcessProvider, Quellen, usize); 4] = [
        (&lokal, &lokaler_anbieter, besprechung(), 0),
        (&lokal, &lokaler_anbieter, Quellen::keine(), 0),
        (&cloud_ziel, &cloud_anbieter, Quellen::keine(), 0),
        (&cloud_ziel, &cloud_anbieter, besprechung(), 3),
    ];
    for (nr, (ziel, anbieter, quellen, lesungen)) in faelle.into_iter().enumerate() {
        let vorher = quelle.lesungen();
        versenden(&quelle, ziel, &quellen, chat_anfrage(anbieter))
            .await
            .unwrap();
        assert_eq!(quelle.lesungen() - vorher, lesungen, "Fall {nr}");
        assert_eq!(pruefungen.anzahl(), (nr + 1, nr + 1), "Fall {nr}");
    }
    assert_eq!(server.treffer(), 4);
}

#[tokio::test]
async fn ein_verweigertes_regelwerk_oder_budget_verhindert_den_transport_und_bleibt_ungebucht() {
    for (art, erwartet_anzahl) in [("compliance", (1, 0)), ("budget", (1, 1))] {
        let _wache = testhaken::wache();
        let server = cloud_server(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
        let pruefungen = regelwerk(Some(art));
        let (ziel, anbieter) = cloud();
        let quelle = quelle_mit_m1();

        let (ergebnis, gebucht) = gebucht(versenden(
            &quelle,
            &ziel,
            &besprechung(),
            chat_anfrage(&anbieter),
        ))
        .await;

        match (art, &ergebnis) {
            ("compliance", Err(VersandFehler::Compliance { meldung })) => {
                assert_eq!(meldung, "compliance verweigert")
            }
            ("budget", Err(VersandFehler::Budget { meldung })) => {
                assert_eq!(meldung, "budget verweigert")
            }
            (_, andere) => panic!("{art}: {andere:?}"),
        }
        assert_eq!(server.treffer(), 0, "{art}: nichts ging hinaus");
        assert_eq!(pruefungen.anzahl(), erwartet_anzahl, "{art}");
        assert!(
            gebucht.is_empty(),
            "{art}: nichts verbraucht, nichts gebucht"
        );
    }
}

#[tokio::test]
async fn gesperrt_hat_vorrang_vor_einem_verweigernden_regelwerk() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    let pruefungen = regelwerk(Some("compliance"));
    let (ziel, anbieter) = cloud();
    // Kundenprofil „nur lokal“: Besprechungsinhalt darf nicht in die Cloud.
    let quelle = TestQuelle::neu(1, richtlinie(true));
    quelle.freigeben("m1");

    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;

    let fehler = ergebnis.unwrap_err();
    assert_eq!(fehler, gesperrt(grund::NUR_LOKAL));
    assert!(
        matches!(&fehler, VersandFehler::Gesperrt { code, .. } if *code == "besprechung_cloud_gesperrt"),
        "{fehler:?}"
    );
    assert_eq!(
        pruefungen.anzahl(),
        (0, 0),
        "Compliance wurde nicht einmal gefragt"
    );
    assert_eq!(server.treffer(), 0);
}

#[tokio::test]
async fn gesperrt_hat_vorrang_auch_vor_dem_echten_regelwerk() {
    let _wache = testhaken::wache();
    // Kein Haken: die echte Prüfung. Ein unbekannter Cloud-Anbieter ist im Standardprofil
    // gesperrt; ohne diese Voraussetzung bewiese der Test nichts (und er ginge ins Netz).
    let basis = "https://api.example.com/v1";
    let anbieter = provider("custom", basis);
    assert!(
        crate::managers::compliance::check_call(&anbieter).is_err(),
        "Voraussetzung: das echte Regelwerk sperrt diesen Anbieter"
    );
    let ziel = Ziel::http("custom", basis, "m");

    // Ohne Freigabe: der Fehler ist der Sperrcode der Besprechung, nicht der des Regelwerks.
    let quelle = TestQuelle::neu(1, richtlinie(false));
    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;
    assert_eq!(ergebnis, Err(gesperrt(grund::NICHT_SICHTBAR)));

    // Mit Freigabe: nun spricht das Regelwerk, und es geht trotzdem nichts hinaus.
    let quelle = quelle_mit_m1();
    let ergebnis = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter)).await;
    assert!(
        matches!(ergebnis, Err(VersandFehler::Compliance { .. })),
        "{ergebnis:?}"
    );
    assert_eq!(
        quelle.lesungen(),
        2,
        "R1 und R2; Transport und R3 entfielen"
    );
}

#[tokio::test]
async fn cloud_mit_kundenprofil_nur_lokal_meldet_besprechung_cloud_gesperrt() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = TestQuelle::neu(1, richtlinie(true));
    quelle.freigeben("m1");

    let fehler = versenden(&quelle, &ziel, &besprechung(), chat_anfrage(&anbieter))
        .await
        .unwrap_err();
    assert_eq!(
        fehler,
        VersandFehler::Gesperrt {
            code: CODE_BESPRECHUNG_CLOUD_GESPERRT,
            grund: grund::NUR_LOKAL
        }
    );
    assert!(fehler.is_terminal());
    assert_eq!(server.treffer(), 0);

    // Dieselbe Quelle, lokal: erlaubt.
    let lokaler_server = mock(|_| RohAntwort::json("lokal", "stop", 1, 1)).await;
    let (lokal, lokaler_anbieter) = lokal_bei(&lokaler_server);
    let antwort = versenden(
        &quelle,
        &lokal,
        &besprechung(),
        chat_anfrage(&lokaler_anbieter),
    )
    .await
    .unwrap();
    assert_eq!(antwort.text.as_deref(), Some("lokal"));
}

#[tokio::test]
async fn unbestimmte_herkunft_geht_nicht_in_die_cloud_aber_lokal() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let ergebnis = versenden(
        &quelle,
        &ziel,
        &Quellen::unbestimmt(),
        chat_anfrage(&anbieter),
    )
    .await;
    assert_eq!(ergebnis, Err(gesperrt(grund::UNBESTIMMT)));
    assert_eq!(server.treffer(), 0);

    let (lokal, lokaler_anbieter) = lokal_bei(&server);
    let antwort = versenden(
        &quelle,
        &lokal,
        &Quellen::unbestimmt(),
        chat_anfrage(&lokaler_anbieter),
    )
    .await
    .unwrap();
    assert_eq!(antwort.text.as_deref(), Some("ok"));
}

#[tokio::test]
async fn ein_anderer_anbieter_als_der_des_ziels_wird_gesperrt_bevor_etwas_geprueft_wird() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    let pruefungen = regelwerk(None);
    let (ziel, _) = cloud();
    let fremder = provider("anderer", CLOUD_BASIS);
    let quelle = quelle_mit_m1();

    let (ergebnis, gebucht) = gebucht(versenden(
        &quelle,
        &ziel,
        &Quellen::keine(),
        chat_anfrage(&fremder),
    ))
    .await;

    assert_eq!(ergebnis, Err(gesperrt(grund::ZIEL_GEWECHSELT)));
    assert_eq!(server.treffer(), 0);
    assert_eq!(quelle.lesungen(), 0);
    assert_eq!(pruefungen.anzahl(), (0, 0));
    assert!(gebucht.is_empty());
}

// -- Buchung ------------------------------------------------------------------------------------

#[tokio::test]
async fn ein_erfolgreicher_aufruf_wird_einmal_mit_seinem_verbrauch_gebucht() {
    let _wache = testhaken::wache();
    cloud_server(|_| RohAntwort::json("hallo", "stop", 3, 2)).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();

    let (antwort, gebucht) = gebucht(versenden(
        &quelle,
        &ziel,
        &besprechung(),
        chat_anfrage(&anbieter),
    ))
    .await;

    assert_eq!(antwort.unwrap().text.as_deref(), Some("hallo"));
    assert_eq!(gebucht.len(), 1);
    assert_eq!(gebucht[0].purpose, Purpose::Chat.as_str());
    assert_eq!(gebucht[0].provider_id, "custom");
    assert_eq!(gebucht[0].model_id, "m");
    assert_eq!(
        (gebucht[0].prompt_tokens, gebucht[0].completion_tokens),
        (3, 2)
    );
    assert!(gebucht[0].ok);
}

#[tokio::test]
async fn ein_verworfenes_ergebnis_bucht_seinen_verbrauch_als_fehlgeschlagen() {
    let _wache = testhaken::wache();
    cloud_server(|_| RohAntwort::json("hallo", "stop", 3, 2)).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 3);

    let (ergebnis, gebucht) = gebucht(versenden(
        &quelle,
        &ziel,
        &besprechung(),
        chat_anfrage(&anbieter),
    ))
    .await;

    assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
    assert_eq!(
        gebucht.len(),
        1,
        "der Aufruf war, auch wenn sein Ergebnis verworfen wurde"
    );
    assert_eq!(
        (gebucht[0].prompt_tokens, gebucht[0].completion_tokens),
        (3, 2)
    );
    assert!(!gebucht[0].ok);
}

#[tokio::test]
async fn ein_transportfehler_ohne_server_ist_gebucht_und_hat_r3_durchlaufen() {
    let _wache = testhaken::wache();
    // Der „Proxy“ ist eine Adresse, an der nichts lauscht.
    let frei = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = frei.local_addr().unwrap().port();
    drop(frei);
    testhaken::setze_cloud_proxy(&format!("http://127.0.0.1:{port}"));
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();

    let (ergebnis, gebucht) = gebucht(versenden(
        &quelle,
        &ziel,
        &besprechung(),
        chat_anfrage(&anbieter),
    ))
    .await;

    assert_eq!(ergebnis, Err(VersandFehler::Transport));
    assert_eq!(quelle.lesungen(), 3, "R3 auch nach einem Transportfehler");
    assert_eq!(gebucht.len(), 1);
    assert!(!gebucht[0].ok);
}

// -- Nebenläufigkeit und Abbruch ----------------------------------------------------------------

#[tokio::test]
async fn zwei_gleichzeitige_aufrufe_vermischen_ihre_ausgaben_nicht() {
    let _wache = testhaken::wache();
    let server = cloud_server(|anfrage| {
        if anfrage.body.contains("Frage-A") {
            RohAntwort::strom(&["a1 ", "a2"])
        } else {
            RohAntwort::strom(&["b1 ", "b2"])
        }
    })
    .await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let quellen = besprechung();
    let (a, b) = (Ausgaben::default(), Ausgaben::default());
    let (text_a, text_b) = (a.text(), b.text());

    let (ra, rb) = tokio::join!(
        versenden(
            &quelle,
            &ziel,
            &quellen,
            strom_anfrage(&anbieter, "Frage-A", &text_a)
        ),
        versenden(
            &quelle,
            &ziel,
            &quellen,
            strom_anfrage(&anbieter, "Frage-B", &text_b)
        ),
    );

    assert_eq!(ra.unwrap().text.as_deref(), Some("a1 a2"));
    assert_eq!(rb.unwrap().text.as_deref(), Some("b1 b2"));
    assert_eq!(a.log(), ["T:a1 ", "T:a2"]);
    assert_eq!(b.log(), ["T:b1 ", "T:b2"]);
    assert_eq!(server.treffer(), 2);
    assert_eq!(quelle.lesungen(), 6, "jeder Aufruf prüft für sich");
}

#[tokio::test]
async fn ein_abbruch_mitten_im_strom_gibt_nichts_aus_und_bucht_nichts() {
    let _wache = testhaken::wache();
    cloud_server(|_| {
        let teile: Vec<String> = (0..60).map(|i| format!("t{i} ")).collect();
        let teile: Vec<&str> = teile.iter().map(String::as_str).collect();
        RohAntwort::strom(&teile).mit_pause(Duration::from_millis(10))
    })
    .await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let aus = Ausgaben::default();

    let (abgebrochen, gebucht) = gebucht(async {
        tokio::time::timeout(
            Duration::from_millis(80),
            strom_mit_denken(&quelle, &ziel, &besprechung(), &anbieter, &aus),
        )
        .await
    })
    .await;

    assert!(
        abgebrochen.is_err(),
        "die Zeitgrenze, nicht das Ende des Stroms, beendete den Aufruf"
    );
    assert!(
        aus.log().is_empty(),
        "der Puffer ging mit dem Aufruf unter: {:?}",
        aus.log()
    );
    assert!(gebucht.is_empty(), "ohne Ergebnis keine Buchung");
    assert_eq!(
        quelle.lesungen(),
        2,
        "R3 kam nie, weil kein Ergebnis vorlag"
    );
    assert_eq!(pruefungen.anzahl(), (1, 1));
}

// -- Rahmen für Wiederholungen ------------------------------------------------------------------

#[tokio::test]
async fn wiederholt_wird_nur_bei_wiederholbaren_fehlern_und_nie_ueber_die_grenze() {
    let aufrufe = Cell::new(0usize);
    let ergebnis: Result<u32, VersandFehler> = mit_wiederholung(3, |nr| {
        aufrufe.set(aufrufe.get() + 1);
        async move {
            if nr < 2 {
                Err(VersandFehler::Transport)
            } else {
                Ok(7)
            }
        }
    })
    .await;
    assert_eq!(ergebnis, Ok(7));
    assert_eq!(aufrufe.get(), 3);

    aufrufe.set(0);
    let ergebnis: Result<u32, VersandFehler> = mit_wiederholung(3, |_| {
        aufrufe.set(aufrufe.get() + 1);
        async { Err(VersandFehler::Http { status: 503 }) }
    })
    .await;
    assert_eq!(ergebnis, Err(VersandFehler::Http { status: 503 }));
    assert_eq!(aufrufe.get(), 3, "höchstens `versuche` Aufrufe");

    aufrufe.set(0);
    let _: Result<u32, VersandFehler> = mit_wiederholung(0, |_| {
        aufrufe.set(aufrufe.get() + 1);
        async { Err(VersandFehler::Transport) }
    })
    .await;
    assert_eq!(aufrufe.get(), 1, "mindestens ein Aufruf");
}

#[tokio::test]
async fn terminale_und_endgueltige_fehler_beenden_die_wiederholung_sofort() {
    let endgueltig = [
        gesperrt(grund::WIDERRUF_NACH_START),
        gesperrt(grund::NUR_LOKAL),
        VersandFehler::Abgebrochen,
        VersandFehler::Kontextueberlauf,
        VersandFehler::Budget {
            meldung: "b".into(),
        },
        VersandFehler::Compliance {
            meldung: "c".into(),
        },
        VersandFehler::CliUpdate,
    ];
    for fehler in endgueltig {
        let aufrufe = Cell::new(0usize);
        let ergebnis: Result<(), VersandFehler> = mit_wiederholung(5, |_| {
            aufrufe.set(aufrufe.get() + 1);
            let fehler = fehler.clone();
            async move { Err(fehler) }
        })
        .await;
        assert_eq!(ergebnis, Err(fehler.clone()));
        assert_eq!(aufrufe.get(), 1, "{fehler:?} wird nicht wiederholt");
    }
}

#[tokio::test]
async fn jede_wiederholung_ist_ein_voller_versand_mit_eigener_pruefung() {
    let _wache = testhaken::wache();
    let server = cloud_server(|anfrage| {
        if anfrage.nr == 0 {
            RohAntwort::text(503, "überlastet")
        } else {
            RohAntwort::json("beim zweiten Mal", "stop", 1, 1)
        }
    })
    .await;
    let pruefungen = regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let quellen = besprechung();

    let antwort = mit_wiederholung(3, |_| {
        versenden(&quelle, &ziel, &quellen, chat_anfrage(&anbieter))
    })
    .await
    .unwrap();

    assert_eq!(antwort.text.as_deref(), Some("beim zweiten Mal"));
    assert_eq!(server.treffer(), 2);
    assert_eq!(quelle.lesungen(), 6, "R1 bis R3 je Aufruf");
    assert_eq!(pruefungen.anzahl(), (2, 2));
}

#[tokio::test]
async fn nach_gesperrt_repariert_oder_wiederholt_der_rahmen_nichts() {
    let _wache = testhaken::wache();
    let server = cloud_server(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    regelwerk(None);
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    widerrufen_bei(&quelle, 3);
    let quellen = besprechung();

    let ergebnis = mit_wiederholung(3, |_| {
        versenden(&quelle, &ziel, &quellen, chat_anfrage(&anbieter))
    })
    .await;

    assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
    assert_eq!(server.treffer(), 1, "kein zweiter Request nach Gesperrt");
    assert_eq!(quelle.lesungen(), 3);
}

// -- Stapel -------------------------------------------------------------------------------------

/// Die Futures von `versenden` und der Fassade tragen die großen Zustandsmaschinen von Chat,
/// Strom und CLI nicht in sich, sondern halten sie auf dem Haufen. Sonst wandert dieser Zustand
/// in das Future jedes Aufrufers (Workflows, Notizen, Agent) und füllt dort in Debug-Builds den
/// Stapel eines Threads (2 MiB): `app_actions::export_of_notes_needs_notes_and_writes_them`
/// lief damit über.
#[test]
fn das_future_von_versenden_ist_klein_weil_die_arbeit_auf_dem_haufen_liegt() {
    let (ziel, anbieter) = cloud();
    let quelle = quelle_mit_m1();
    let quellen = besprechung();
    let on_delta = |_: &str| {};
    let strom = versenden(
        &quelle,
        &ziel,
        &quellen,
        strom_anfrage(&anbieter, "x", &on_delta),
    );
    let chat = versenden(&quelle, &ziel, &quellen, chat_anfrage(&anbieter));
    let (strom, chat) = (std::mem::size_of_val(&strom), std::mem::size_of_val(&chat));
    // Gemessen: etwa 4 KiB. Ohne Haufen sind es Zehntausende Byte.
    assert!(strom <= 8192, "Strom: {strom} Byte");
    assert!(chat <= 8192, "Chat: {chat} Byte");
}
