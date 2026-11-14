//! Netztests des Rohtransports gegen einen lokalen HTTP-Testserver (lva#604, Teilpaket 3).
//!
//! Geprüft werden die Transportregeln (kein Redirect, lokal ohne Proxy, Cloud mit
//! Umgebungsproxy), der Fehlervertrag (kein Anbietertext in Fehler und Log, Kontextüberlauf
//! erkennbar, Zielwechsel abgewiesen) und das Verhalten der Wege (Chat, Strom, Rückfall, Bild,
//! Ollama, Modellliste). Tests, die Umgebungsvariablen oder den globalen Logger brauchen, laufen
//! in einem Kindprozess ([`kind_ausfuehren`]): dort läuft nur dieser Test.

use std::sync::{Mutex, OnceLock};

use std::time::Duration;

use super::{bild, http, json, modelle, ollama};
use super::{Ausgabe, Fehlschlag, Kontext, Versuch};
use crate::llm_client::{ChatReply, StreamLimits, StreamMessage, STREAM_MAX_CHARS};
use crate::llm_versand::anfrage::ChatInhalt;
use crate::llm_versand::fehler::{grund, VersandFehler};
use crate::llm_versand::pruefer::{freigabe_uebergang, quellen_uebergang};
use crate::llm_versand::testhilfe::{
    anbieter, ist_kind, kind_ausfuehren, mock, sammler, sse_denken, sse_text, RohAntwort,
};
use crate::llm_versand::ziel::Ziel;
use crate::managers::usage::Purpose;
use crate::settings::PostProcessProvider;

/// Text, der in keinem Fehler und in keiner Logzeile vorkommen darf.
const GEHEIM: &str = "GEHEIM-ANBIETERTEXT-4711";

fn chat_inhalt(text: &str) -> ChatInhalt {
    ChatInhalt {
        nutzer: text.into(),
        system: None,
        schema: None,
        reasoning_effort: None,
        reasoning: None,
    }
}

fn kontext(anbieter: &PostProcessProvider) -> Kontext<'_> {
    Kontext {
        anbieter,
        api_key: "",
        zweck: Purpose::Chat,
    }
}

async fn chat(ziel: &Ziel, anbieter: &PostProcessProvider) -> Versuch<ChatReply> {
    http::chat_einmal(
        freigabe_uebergang(ziel).freigabe,
        ziel,
        &quellen_uebergang(),
        &kontext(anbieter),
        &chat_inhalt("Hallo"),
    )
    .await
}

fn nachrichten() -> Vec<StreamMessage> {
    vec![StreamMessage::new("user", "x")]
}

/// Ein Stromversuch; liefert das Ergebnis und die Stücke, die der Rückruf sah.
async fn strom(
    ziel: &Ziel,
    anbieter: &PostProcessProvider,
    grenzen: &StreamLimits,
) -> (Versuch<String>, Vec<String>) {
    let (gesehen, on_delta) = sammler();
    let ausgabe = Ausgabe {
        on_delta: &on_delta,
        senke: None,
    };
    let ergebnis = http::strom(
        freigabe_uebergang(ziel).freigabe,
        ziel,
        &quellen_uebergang(),
        &kontext(anbieter),
        &nachrichten(),
        grenzen,
        &ausgabe,
    )
    .await;
    let stuecke = gesehen.lock().unwrap().clone();
    (ergebnis, stuecke)
}

fn fehler<T: std::fmt::Debug>(versuch: Versuch<T>) -> VersandFehler {
    match versuch {
        Err(Fehlschlag::Fehler(fehler)) => fehler,
        andere => panic!("kein Fehler: {andere:?}"),
    }
}

fn lokal(mock_basis: &str) -> (Ziel, PostProcessProvider) {
    (
        Ziel::http("custom", mock_basis, "m"),
        anbieter("custom", mock_basis),
    )
}

// -- Weg: Antwort am Stück ----------------------------------------------------------------------

#[tokio::test]
async fn eine_antwort_am_stueck_kommt_mit_verbrauch_und_pfad_an() {
    let server = mock(|_| RohAntwort::json("Hallo zurück", "stop", 3, 2)).await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (antwort, verbrauch) = chat(&ziel, &anbieter).await.unwrap();
    assert_eq!(antwort.content.as_deref(), Some("Hallo zurück"));
    assert!(!antwort.truncated);
    assert_eq!(
        (verbrauch.prompt_tokens, verbrauch.completion_tokens),
        (3, 2)
    );
    let anfragen = server.anfragen();
    assert_eq!(anfragen.len(), 1);
    assert!(
        anfragen[0].zeile.starts_with("POST /v1/chat/completions"),
        "{}",
        anfragen[0].zeile
    );
    assert!(anfragen[0].body.contains("\"model\":\"m\""));
}

#[tokio::test]
async fn eine_abgeschnittene_antwort_wird_gemeldet() {
    let server = mock(|_| RohAntwort::json("abc", "length", 1, 1)).await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (antwort, _) = chat(&ziel, &anbieter).await.unwrap();
    assert!(antwort.truncated);
}

// -- Transportregeln ----------------------------------------------------------------------------

#[tokio::test]
async fn auch_die_lokale_route_folgt_keinem_redirect() {
    let ziel_server = mock(|_| RohAntwort::json("zu weit", "stop", 1, 1)).await;
    let ziel_adresse = ziel_server.adresse();
    let umleiter = mock(move |_| {
        let mut antwort = RohAntwort::neu(302, "text/plain", Vec::new());
        antwort
            .kopfzeilen
            .push(format!("Location: {ziel_adresse}/v1/chat/completions"));
        antwort
    })
    .await;
    let (ziel, anbieter) = lokal(&umleiter.basis());
    let ergebnis = chat(&ziel, &anbieter).await;
    assert_eq!(fehler(ergebnis), VersandFehler::Http { status: 302 });
    assert_eq!(umleiter.treffer(), 1);
    assert_eq!(
        ziel_server.treffer(),
        0,
        "der Umleitung wurde nicht gefolgt"
    );
}

/// Die Regeln mit der **echten** Umgebung: der Kindprozess setzt `HTTP_PROXY` auf einen
/// Testserver. Kontrolle zuerst: ein Standardclient erreicht den lokalen Server über den Proxy
/// (sonst bewiese „kein Proxy“ nichts). Dann: die lokale Route bleibt direkt; ein Cloud-Ziel geht
/// über den Proxy; auch dort folgt der Transport keinem Redirect.
#[tokio::test]
async fn lokal_ohne_proxy_cloud_mit_umgebungsproxy_und_nie_ein_redirect() {
    if !ist_kind() {
        return kind_ausfuehren(
            module_path!(),
            "lokal_ohne_proxy_cloud_mit_umgebungsproxy_und_nie_ein_redirect",
        );
    }
    let proxy = mock(|anfrage| match anfrage.nr {
        0 | 1 => RohAntwort::json("über den Proxy", "stop", 1, 1),
        _ => {
            let mut antwort = RohAntwort::neu(302, "text/plain", Vec::new());
            antwort
                .kopfzeilen
                .push("Location: http://cloud.example/anderswo".into());
            antwort
        }
    })
    .await;
    let direkt = mock(|_| RohAntwort::json("direkt", "stop", 1, 1)).await;
    for var in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::set_var(var, proxy.adresse());
    }

    // Kontrolle: ein Standardclient wird vom Umgebungsproxy abgefangen.
    let antwort = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", direkt.adresse()))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(antwort.status().as_u16(), 200);
    assert_eq!(proxy.treffer(), 1, "Kontrolle: der Umgebungsproxy wirkt");
    assert_eq!(direkt.treffer(), 0);

    // Lokale Route: direkt, der Proxy bleibt unberührt.
    let (ziel, lokaler_anbieter) = lokal(&direkt.basis());
    let (lokal_antwort, _) = chat(&ziel, &lokaler_anbieter).await.unwrap();
    assert_eq!(lokal_antwort.content.as_deref(), Some("direkt"));
    assert_eq!(direkt.treffer(), 1);
    assert_eq!(
        proxy.treffer(),
        1,
        "die lokale Route ging nicht über den Proxy"
    );

    // Cloud-Ziel: über den Umgebungsproxy, mit der vollen Adresse in der Anfragezeile.
    let cloud = Ziel::http("custom", "http://cloud.example/v1", "m");
    assert!(cloud.ist_cloud());
    let cloud_anbieter = anbieter("custom", "http://cloud.example/v1");
    let (cloud_antwort, _) = chat(&cloud, &cloud_anbieter).await.unwrap();
    assert_eq!(cloud_antwort.content.as_deref(), Some("über den Proxy"));
    assert_eq!(proxy.treffer(), 2);
    assert!(proxy.anfragen()[1]
        .zeile
        .contains("http://cloud.example/v1/chat/completions"));

    // Auch beim Cloud-Ziel kein Redirect.
    let umgeleitet = chat(&cloud, &cloud_anbieter).await;
    assert_eq!(fehler(umgeleitet), VersandFehler::Http { status: 302 });
    assert_eq!(proxy.treffer(), 3, "der Umleitung wurde nicht gefolgt");
}

// -- Zielbindung --------------------------------------------------------------------------------

#[tokio::test]
async fn ein_zielwechsel_bei_gleicher_anbieter_id_wird_abgewiesen_und_nichts_gesendet() {
    let server = mock(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    let gebunden = Ziel::http("custom", &server.basis(), "m");
    let umgebogen = Ziel::http("custom", "http://127.0.0.1:9/v1", "m");
    assert_eq!(gebunden.anbieter_id(), umgebogen.anbieter_id());
    let anbieter = anbieter("custom", &server.basis());
    let ergebnis = http::chat_einmal(
        freigabe_uebergang(&gebunden).freigabe,
        &umgebogen,
        &quellen_uebergang(),
        &kontext(&anbieter),
        &chat_inhalt("x"),
    )
    .await;
    assert_eq!(
        fehler(ergebnis),
        VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT)
    );
    assert_eq!(server.treffer(), 0, "es wurde nichts gesendet");
}

#[tokio::test]
async fn jeder_transportweg_loest_die_freigabe_gegen_das_ziel_ein() {
    let server = mock(|_| RohAntwort::json("ok", "stop", 1, 1)).await;
    let gebunden = Ziel::http("custom", &server.basis(), "m");
    let anderes = Ziel::http("custom", "http://127.0.0.1:9/v1", "m");
    let anbieter = anbieter("custom", &server.basis());
    let gesperrt = VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT);

    // Strom und Rückfall.
    let ausgabe = Ausgabe {
        on_delta: &|_: &str| {},
        senke: None,
    };
    for ohne_strom in [false, true] {
        let freigabe = freigabe_uebergang(&gebunden).freigabe;
        let nachrichten = nachrichten();
        let grenzen = StreamLimits::default();
        let ergebnis = if ohne_strom {
            http::ohne_strom(
                freigabe,
                &anderes,
                &quellen_uebergang(),
                &kontext(&anbieter),
                &nachrichten,
                &grenzen,
                &ausgabe,
            )
            .await
        } else {
            http::strom(
                freigabe,
                &anderes,
                &quellen_uebergang(),
                &kontext(&anbieter),
                &nachrichten,
                &grenzen,
                &ausgabe,
            )
            .await
        };
        assert_eq!(fehler(ergebnis), gesperrt);
    }
    // Ollama nativ.
    let ergebnis = ollama::nativ(
        freigabe_uebergang(&gebunden).freigabe,
        &anderes,
        &quellen_uebergang(),
        "x".into(),
        false,
    )
    .await;
    assert_eq!(fehler(ergebnis), gesperrt);
    // Bild und Modellliste (Texte, nicht Typen).
    let (_ordner, bild_datei) = bild_datei("a.jpg", b"x");
    let fehlertext = bild::senden(
        freigabe_uebergang(&gebunden).freigabe,
        &anderes,
        &quellen_uebergang(),
        &bild_datei,
        "p",
        10,
    )
    .await
    .unwrap_err();
    assert!(fehlertext.contains("ziel_gewechselt"), "{fehlertext}");
    let fehlertext = modelle::liste(
        freigabe_uebergang(&gebunden).freigabe,
        &anderes,
        &quellen_uebergang(),
        &anbieter,
        "",
    )
    .await
    .unwrap_err();
    assert!(fehlertext.contains("ziel_gewechselt"), "{fehlertext}");
    // Rohes JSON-POST (Agent, Einbettungen, Tokenzählung).
    let ergebnis = json::post(
        freigabe_uebergang(&gebunden).freigabe,
        &anderes,
        &quellen_uebergang(),
        "/tokenize",
        &serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(ergebnis.unwrap_err(), gesperrt);
    assert_eq!(server.treffer(), 0, "nichts ging hinaus");
}

// -- Weg: rohes JSON-POST -----------------------------------------------------------------------

async fn json_post(
    ziel: &Ziel,
    pfad: &str,
    zeit: Duration,
) -> Result<crate::llm_versand::anfrage::JsonAntwort, VersandFehler> {
    json::post(
        freigabe_uebergang(ziel).freigabe,
        ziel,
        &quellen_uebergang(),
        pfad,
        &serde_json::json!({"content": "x"}),
        zeit,
    )
    .await
}

#[tokio::test]
async fn ein_json_post_liefert_status_und_koerper_und_haengt_den_pfad_an_die_basis() {
    let server = mock(|anfrage| {
        if anfrage.zeile.contains("/tokenize") {
            RohAntwort::neu(
                200,
                "application/json",
                vec![br#"{"tokens":[1,2,3]}"#.to_vec()],
            )
        } else {
            RohAntwort::text(200, "kein JSON")
        }
    })
    .await;
    let ziel = Ziel::http("local", &format!("{}/", server.adresse()), "");
    let antwort = json_post(&ziel, "/tokenize", Duration::from_secs(5))
        .await
        .unwrap();
    assert!(antwort.erfolgreich());
    assert_eq!(antwort.json, Some(serde_json::json!({"tokens": [1, 2, 3]})));
    assert!(server.anfragen()[0].zeile.starts_with("POST /tokenize "));
    assert!(server.anfragen()[0].body.contains("\"content\":\"x\""));

    // Erfolg ohne gültiges JSON: der Status bleibt, der Körper fehlt.
    let anders = json_post(&ziel, "/ping", Duration::from_secs(5))
        .await
        .unwrap();
    assert!(anders.erfolgreich());
    assert_eq!(anders.json, None);
}

#[tokio::test]
async fn ein_json_post_meldet_fehlerstatus_und_ueberlauf_aber_nie_den_text() {
    let server = mock(|anfrage| match anfrage.nr {
        0 => RohAntwort::text(503, GEHEIM),
        1 => RohAntwort::text(400, &format!("{GEHEIM} exceeds the available context size")),
        _ => RohAntwort::text(400, &format!("{GEHEIM} anderer Fehler")),
    })
    .await;
    let ziel = Ziel::http("local", &server.basis(), "");
    let busy = json_post(&ziel, "/x", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        (busy.status, busy.ueberlauf, busy.erfolgreich()),
        (503, false, false)
    );
    let ueberlauf = json_post(&ziel, "/x", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!((ueberlauf.status, ueberlauf.ueberlauf), (400, true));
    let sonst = json_post(&ziel, "/x", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        (sonst.status, sonst.ueberlauf, sonst.json.clone()),
        (400, false, None)
    );
    for antwort in [busy, ueberlauf, sonst] {
        assert!(!format!("{antwort:?}").contains(GEHEIM));
    }
}

#[tokio::test]
async fn ein_json_post_folgt_keinem_redirect() {
    let ziel_server = mock(|_| RohAntwort::json("zu weit", "stop", 1, 1)).await;
    let adresse = ziel_server.adresse();
    let umleiter = mock(move |_| {
        let mut antwort = RohAntwort::neu(307, "text/plain", Vec::new());
        antwort
            .kopfzeilen
            .push(format!("Location: {adresse}/tokenize"));
        antwort
    })
    .await;
    let ziel = Ziel::http("local", &umleiter.basis(), "");
    let antwort = json_post(&ziel, "/tokenize", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(antwort.status, 307);
    assert!(!antwort.erfolgreich());
    assert_eq!(ziel_server.treffer(), 0, "dem Redirect wurde nicht gefolgt");
}

#[tokio::test]
async fn ein_json_post_ist_bei_abgelaufener_zeit_ein_zeitfehler_auch_beim_lesen_des_koerpers() {
    // Der Kopf kommt sofort, der Körper stockt: die Gesamtzeit läuft beim Lesen ab.
    let server = mock(|_| {
        RohAntwort::neu(
            200,
            "application/json",
            vec![b"{\"a\":".to_vec(), b"1}".to_vec()],
        )
        .mit_pause(Duration::from_millis(600))
    })
    .await;
    let ziel = Ziel::http("local", &server.basis(), "");
    let fehler = json_post(&ziel, "/x", Duration::from_millis(150))
        .await
        .unwrap_err();
    assert_eq!(fehler, VersandFehler::Zeitueberschreitung);
}

#[tokio::test]
async fn ein_json_post_ohne_server_ist_ein_transportfehler() {
    let ziel = Ziel::http("local", "http://127.0.0.1:1", "");
    let fehler = json_post(&ziel, "/x", Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(fehler, VersandFehler::Transport);
}

// -- Fehlervertrag ------------------------------------------------------------------------------

#[tokio::test]
async fn ein_kontextueberlauf_wird_erkannt_und_sein_text_verworfen() {
    let server = mock(|_| {
        RohAntwort::neu(
            400,
            "application/json",
            vec![format!(
                r#"{{"error":{{"type":"exceed_context_size_error","message":"{GEHEIM}: request (9000 tokens) exceeds the available context size"}}}}"#
            )
            .into_bytes()],
        )
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let f = fehler(chat(&ziel, &anbieter).await);
    assert_eq!(f, VersandFehler::Kontextueberlauf);
    for text in [f.to_string(), format!("{f:?}"), f.alte_textform()] {
        assert!(!text.contains(GEHEIM), "{text}");
    }
    // Die Aufrufer erkennen den Überlauf wie bisher am Text (`meetings/llm_call.rs`).
    assert!(crate::managers::meetings::llm_call::is_truncation_error(
        &f.alte_textform()
    ));
    assert!(crate::managers::meetings::llm_call::is_splittable_error(
        &f.alte_textform()
    ));
}

#[tokio::test]
async fn jeder_andere_fehlerstatus_traegt_nur_die_zahl() {
    let server = mock(|_| RohAntwort::text(503, &format!("{GEHEIM} overloaded"))).await;
    let (ziel, anbieter) = lokal(&server.basis());
    let f = fehler(chat(&ziel, &anbieter).await);
    assert_eq!(f, VersandFehler::Http { status: 503 });
    assert!(f.ist_wiederholbar());
    assert!(f
        .alte_textform()
        .contains("API request failed with status 503"));
    assert!(!f.alte_textform().contains(GEHEIM));
}

#[tokio::test]
async fn ohne_server_ist_es_ein_transportfehler_ohne_text() {
    let (ziel, anbieter) = lokal("http://127.0.0.1:1/v1");
    let f = fehler(chat(&ziel, &anbieter).await);
    assert_eq!(f, VersandFehler::Transport);
    // Die Aufrufer unterscheiden „keine Verbindung“ am Präfix (`meetings/chat/controller.rs`).
    assert!(f.alte_textform().starts_with("HTTP request failed"));
}

/// Der Marker steht in Fehlerantworten, Stromfehlern, kaputtem JSON, Ollama-, Bild- und
/// Listenantworten; er darf weder in einem Fehler (alle Textformen) noch im Log erscheinen. Der
/// Kindprozess setzt den Logger (es gibt nur einen je Prozess) und stellt zuerst sicher, dass
/// überhaupt Zeilen ankommen.
#[tokio::test]
async fn anbietertext_gelangt_weder_in_einen_fehler_noch_ins_log() {
    if !ist_kind() {
        return kind_ausfuehren(
            module_path!(),
            "anbietertext_gelangt_weder_in_einen_fehler_noch_ins_log",
        );
    }
    struct Erfasser(Mutex<Vec<String>>);
    impl log::Log for Erfasser {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, satz: &log::Record) {
            self.0.lock().unwrap().push(format!("{}", satz.args()));
        }
        fn flush(&self) {}
    }
    static ERFASSER: OnceLock<Erfasser> = OnceLock::new();
    let erfasser = ERFASSER.get_or_init(|| Erfasser(Mutex::new(Vec::new())));
    log::set_logger(erfasser).expect("ein Logger je Prozess");
    log::set_max_level(log::LevelFilter::Trace);

    let server = mock(|anfrage| match anfrage.zeile.as_str() {
        z if z.contains("/models") => RohAntwort::text(500, GEHEIM),
        z if z.contains("/api/chat") => RohAntwort::text(500, GEHEIM),
        _ if anfrage.body.contains("\"stream\":true") => {
            // Der Strom wird abgelehnt (Rückfall erlaubt) ...
            RohAntwort::text(500, GEHEIM)
        }
        _ if anfrage.body.contains("kaputt") => RohAntwort::neu(
            200,
            "application/json",
            vec![format!("{{\"choices\": {GEHEIM}").into_bytes()],
        ),
        _ if anfrage.body.contains("ereignis") => RohAntwort::neu(
            200,
            "text/event-stream",
            vec![format!("data: {{\"error\":{{\"type\":\"{GEHEIM}\"}}}}\n\n").into_bytes()],
        ),
        _ => RohAntwort::text(500, GEHEIM),
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let mut fehlerliste: Vec<VersandFehler> = Vec::new();

    // Antwort am Stück: Fehlerstatus.
    fehlerliste.push(fehler(chat(&ziel, &anbieter).await));
    // Antwort am Stück: kaputtes JSON.
    let ergebnis = http::chat_einmal(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        &kontext(&anbieter),
        &chat_inhalt("kaputt"),
    )
    .await;
    fehlerliste.push(fehler(ergebnis));
    // Strom abgelehnt, dann der Rückfall (ebenfalls Fehlerstatus).
    let (abgelehnt, _) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    assert!(matches!(abgelehnt, Err(Fehlschlag::OhneStromNeu)));
    let ausgabe = Ausgabe {
        on_delta: &|_: &str| {},
        senke: None,
    };
    let rueckfall = http::ohne_strom(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        &kontext(&anbieter),
        &nachrichten(),
        &StreamLimits::default(),
        &ausgabe,
    )
    .await;
    fehlerliste.push(fehler(rueckfall));
    // Fehlerereignis im Strom (Marker im Typ).
    let (ergebnis, _) = {
        let (gesehen, on_delta) = sammler();
        let ausgabe = Ausgabe {
            on_delta: &on_delta,
            senke: None,
        };
        let nachrichten = vec![StreamMessage::new("user", "ereignis")];
        // Der Haupt-Testserver lehnt `"stream":true` ab; das Fehlerereignis liefert ein zweiter.
        let server2 = mock(|_| {
            RohAntwort::neu(
                200,
                "text/event-stream",
                vec![format!("data: {{\"error\":{{\"type\":\"{GEHEIM}\"}}}}\n\n").into_bytes()],
            )
        })
        .await;
        let ziel2 = Ziel::http("custom", &server2.basis(), "m");
        let ergebnis = http::strom(
            freigabe_uebergang(&ziel2).freigabe,
            &ziel2,
            &quellen_uebergang(),
            &kontext(&anbieter),
            &nachrichten,
            &StreamLimits::default(),
            &ausgabe,
        )
        .await;
        (ergebnis, gesehen)
    };
    fehlerliste.push(fehler(ergebnis));
    // Ollama, Modellliste, Bild.
    let ollama_ziel = Ziel::http("ollama", &format!("{}/api/chat", server.adresse()), "m");
    let nativ = ollama::nativ(
        freigabe_uebergang(&ollama_ziel).freigabe,
        &ollama_ziel,
        &quellen_uebergang(),
        "x".into(),
        false,
    )
    .await;
    fehlerliste.push(fehler(nativ));
    let mut texte: Vec<String> = Vec::new();
    texte.push(
        modelle::liste(
            freigabe_uebergang(&ziel).freigabe,
            &ziel,
            &quellen_uebergang(),
            &anbieter,
            "",
        )
        .await
        .unwrap_err(),
    );
    let (_ordner, bild_datei) = bild_datei("a.jpg", b"x");
    texte.push(
        bild::senden(
            freigabe_uebergang(&ziel).freigabe,
            &ziel,
            &quellen_uebergang(),
            &bild_datei,
            "p",
            10,
        )
        .await
        .unwrap_err(),
    );

    assert_eq!(
        fehlerliste.len(),
        5,
        "fünf Wege mit Fehler, dazu Liste und Bild in `texte`"
    );
    for f in &fehlerliste {
        for text in [f.to_string(), format!("{f:?}"), f.alte_textform()] {
            assert!(!text.contains(GEHEIM), "Fehler trägt Anbietertext: {text}");
        }
    }
    for text in &texte {
        assert!(
            !text.contains(GEHEIM),
            "Fehlertext trägt Anbietertext: {text}"
        );
    }
    let zeilen = erfasser.0.lock().unwrap().clone();
    assert!(
        !zeilen.is_empty(),
        "der Logger sah nichts: die Prüfung wäre leer"
    );
    for zeile in &zeilen {
        assert!(
            !zeile.contains(GEHEIM),
            "Logzeile trägt Anbietertext: {zeile}"
        );
    }
}

// -- Weg: Strom ---------------------------------------------------------------------------------

#[tokio::test]
async fn ein_strom_liefert_die_stuecke_in_reihenfolge_und_den_ganzen_text() {
    let server = mock(|anfrage| {
        assert!(
            anfrage.body.contains("\"stream\":true"),
            "Streaming angefragt"
        );
        RohAntwort::strom(&["Das ", "Budget ", "steht [Q1]."])
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (ergebnis, stuecke) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    let (text, _) = ergebnis.unwrap();
    assert_eq!(text, "Das Budget steht [Q1].");
    assert_eq!(stuecke, vec!["Das ", "Budget ", "steht [Q1]."]);
}

#[tokio::test]
async fn ein_strom_ohne_done_endet_mit_der_verbindung_und_streng_ist_das_ein_fehler() {
    let server =
        mock(|_| RohAntwort::neu(200, "text/event-stream", sse_text(&["Ende ", "ohne DONE"])))
            .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (ergebnis, _) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    assert_eq!(ergebnis.unwrap().0, "Ende ohne DONE");
    let streng = StreamLimits {
        strict: true,
        ..StreamLimits::default()
    };
    let (ergebnis, _) = strom(&ziel, &anbieter, &streng).await;
    assert_eq!(fehler(ergebnis), VersandFehler::Protokoll);
}

#[tokio::test]
async fn eine_ganze_json_antwort_zaehlt_als_ein_stueck() {
    let server = mock(|_| RohAntwort::json("Alles auf einmal", "stop", 1, 1)).await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (ergebnis, stuecke) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    assert_eq!(ergebnis.unwrap().0, "Alles auf einmal");
    assert_eq!(stuecke, vec!["Alles auf einmal"]);
}

#[tokio::test]
async fn ein_abgelehnter_strom_erlaubt_einen_neuen_versuch_ohne_strom() {
    let server = mock(|anfrage| {
        if anfrage.nr == 0 {
            assert!(anfrage.body.contains("\"stream\":true"));
            RohAntwort::text(400, "stream not supported")
        } else {
            assert!(anfrage.body.contains("\"stream\":false"), "ohne Streaming");
            RohAntwort::json("Rückfall", "stop", 5, 1)
        }
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    // Der Transport macht genau EINEN Request und meldet, dass ein neuer Versuch erlaubt wäre.
    let (ergebnis, stuecke) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    assert!(
        matches!(ergebnis, Err(Fehlschlag::OhneStromNeu)),
        "{ergebnis:?}"
    );
    assert!(stuecke.is_empty());
    assert_eq!(server.treffer(), 1, "kein heimlicher zweiter Request");

    // Der neue Versuch ist ein eigener Aufruf.
    let (gesehen, on_delta) = sammler();
    let ausgabe = Ausgabe {
        on_delta: &on_delta,
        senke: None,
    };
    let (text, verbrauch) = http::ohne_strom(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        &kontext(&anbieter),
        &nachrichten(),
        &StreamLimits::default(),
        &ausgabe,
    )
    .await
    .unwrap();
    assert_eq!(text, "Rückfall");
    assert_eq!(verbrauch.prompt_tokens, 5);
    assert_eq!(*gesehen.lock().unwrap(), vec!["Rückfall"]);
    assert_eq!(server.treffer(), 2);
}

#[tokio::test]
async fn ein_fehler_nach_dem_ersten_token_ist_ein_fehler_und_kein_neuer_versuch() {
    let server = mock(|_| {
        let mut stuecke = sse_text(&["Teil "]);
        stuecke.push(b"data: {\"error\":{\"type\":\"server_error\"}}\n\n".to_vec());
        RohAntwort::neu(200, "text/event-stream", stuecke)
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (ergebnis, stuecke) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    assert_eq!(fehler(ergebnis), VersandFehler::Protokoll);
    assert_eq!(stuecke, vec!["Teil "]);
    assert_eq!(server.treffer(), 1);
}

#[tokio::test]
async fn denktexte_gehen_an_die_senke_und_nie_in_den_text() {
    let server = mock(|_| {
        let mut stuecke = sse_denken(&["Ich prüfe"]);
        stuecke.extend(sse_text(&["Antwort"]));
        stuecke.push(b"data: [DONE]\n\n".to_vec());
        RohAntwort::neu(200, "text/event-stream", stuecke)
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (gedanken, senke) = sammler();
    let (texte, on_delta) = sammler();
    let senke: crate::llm_client::DenkSenke = std::sync::Arc::new(senke);
    let ausgabe = Ausgabe {
        on_delta: &on_delta,
        senke: Some(senke),
    };
    let (text, _) = http::strom(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        &kontext(&anbieter),
        &nachrichten(),
        &StreamLimits::default(),
        &ausgabe,
    )
    .await
    .unwrap();
    assert_eq!(text, "Antwort");
    assert_eq!(*gedanken.lock().unwrap(), vec!["Ich prüfe"]);
    assert_eq!(*texte.lock().unwrap(), vec!["Antwort"]);
}

#[tokio::test]
async fn eine_ausufernde_antwort_wird_an_der_grenze_abgeschnitten() {
    let server = mock(|_| {
        let stueck = "x".repeat(4096);
        let teile: Vec<&str> = (0..12).map(|_| stueck.as_str()).collect();
        RohAntwort::neu(200, "text/event-stream", sse_text(&teile))
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let (ergebnis, _) = strom(&ziel, &anbieter, &StreamLimits::default()).await;
    let (text, _) = ergebnis.unwrap();
    assert!(text.chars().count() <= STREAM_MAX_CHARS);
    assert!(text.chars().count() >= STREAM_MAX_CHARS - 4096);
    // Streng ist die Überschreitung ein Fehler.
    let streng = StreamLimits {
        strict: true,
        ..StreamLimits::default()
    };
    let (ergebnis, _) = strom(&ziel, &anbieter, &streng).await;
    assert_eq!(fehler(ergebnis), VersandFehler::Protokoll);
}

// -- Weg: Bild ----------------------------------------------------------------------------------

fn bild_datei(name: &str, inhalt: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let ordner = tempfile::tempdir().unwrap();
    let pfad = ordner.path().join(name);
    std::fs::write(&pfad, inhalt).unwrap();
    (ordner, pfad)
}

async fn bild_senden(
    basis: &str,
    datei: &std::path::Path,
) -> Result<crate::llm_client::ImageReply, String> {
    let ziel = Ziel::eingebaut("m", basis);
    bild::senden(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        datei,
        "Frage",
        200,
    )
    .await
}

#[tokio::test]
async fn eine_bildantwort_kommt_zurueck_und_der_server_sah_das_bild() {
    let server =
        mock(|_| RohAntwort::json("sprecher_raum\nEine Person am Pult.", "stop", 1, 1)).await;
    let (_ordner, datei) = bild_datei("0001.jpg", b"jpegbytes");
    let antwort = bild_senden(&server.basis(), &datei).await.unwrap();
    assert_eq!(antwort.content, "sprecher_raum\nEine Person am Pult.");
    assert!(!antwort.truncated);
    let gesendet: serde_json::Value = serde_json::from_str(&server.anfragen()[0].body).unwrap();
    assert_eq!(gesendet["messages"][0]["content"][1]["text"], "Frage");
    assert!(gesendet["messages"][0]["content"][0]["image_url"]["url"]
        .as_str()
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));
}

#[tokio::test]
async fn abgeschnittene_leere_und_leerzeichen_antworten_werden_gemeldet_nicht_versteckt() {
    let server = mock(|anfrage| match anfrage.nr {
        0 => RohAntwort::json("abc", "length", 1, 1),
        1 => RohAntwort::neu(
            200,
            "application/json",
            vec![br#"{"choices":[{"message":{"content":null},"finish_reason":"stop"}]}"#.to_vec()],
        ),
        _ => RohAntwort::json("   ", "stop", 1, 1),
    })
    .await;
    let (_ordner, datei) = bild_datei("a.png", b"png");
    let erste = bild_senden(&server.basis(), &datei).await.unwrap();
    assert_eq!((erste.content.as_str(), erste.truncated), ("abc", true));
    let zweite = bild_senden(&server.basis(), &datei).await.unwrap();
    assert_eq!((zweite.content.as_str(), zweite.truncated), ("", false));
    let dritte = bild_senden(&server.basis(), &datei).await.unwrap();
    assert_eq!(dritte.content, "   ");
}

#[tokio::test]
async fn schlechte_bilder_und_serverfehler_tragen_einen_code_aber_keinen_servertext() {
    let server = mock(|_| RohAntwort::text(500, GEHEIM)).await;
    let basis = server.basis();
    let (_ordner, datei) = bild_datei("a.jpg", b"x");
    let fehlertext = bild_senden(&basis, &datei).await.unwrap_err();
    assert!(
        fehlertext.starts_with("request_failed: HTTP 500"),
        "{fehlertext}"
    );
    assert!(!fehlertext.contains(GEHEIM), "{fehlertext}");

    let (_o2, leer) = bild_datei("e.jpg", b"");
    let fehlertext = bild_senden(&basis, &leer).await.unwrap_err();
    assert!(fehlertext.starts_with("image_empty"), "{fehlertext}");
    let (_o3, text) = bild_datei("a.txt", b"x");
    let fehlertext = bild_senden(&basis, &text).await.unwrap_err();
    assert!(fehlertext.starts_with("image_unsupported"), "{fehlertext}");
    let fehlend = std::path::Path::new("/gibt/es/nicht.jpg");
    let fehlertext = bild_senden(&basis, fehlend).await.unwrap_err();
    assert!(fehlertext.starts_with("image_unreadable"), "{fehlertext}");
    // Kein Server: Verbindungsfehler, kein Hänger.
    let fehlertext = bild_senden("http://127.0.0.1:1/v1", &datei)
        .await
        .unwrap_err();
    assert!(fehlertext.starts_with("request_failed"), "{fehlertext}");
    assert_eq!(
        server.treffer(),
        1,
        "nur der erste Aufruf erreichte den Server"
    );
}

// -- Weg: Ollama und Modellliste ----------------------------------------------------------------

#[tokio::test]
async fn natives_ollama_liefert_text_und_eigene_zaehler_und_laedt_ohne_gpu_wenn_verlangt() {
    let server = mock(|_| {
        RohAntwort::neu(
            200,
            "application/json",
            vec![
                br#"{"message":{"content":"Hallo"},"prompt_eval_count":7,"eval_count":3}"#.to_vec(),
            ],
        )
    })
    .await;
    let ziel = Ziel::http("ollama", &format!("{}/api/chat", server.adresse()), "m");
    let (text, verbrauch) = ollama::nativ(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        "Prompt".into(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(text.as_deref(), Some("Hallo"));
    assert_eq!(
        (verbrauch.prompt_tokens, verbrauch.completion_tokens),
        (7, 3)
    );
    let gesendet: serde_json::Value = serde_json::from_str(&server.anfragen()[0].body).unwrap();
    assert_eq!(gesendet["options"]["num_gpu"], 0);
    assert_eq!(gesendet["keep_alive"], 0);
    assert_eq!(gesendet["stream"], false);
}

#[tokio::test]
async fn das_entladen_schickt_keep_alive_null_und_scheitert_still() {
    let server = mock(|_| RohAntwort::text(200, "{}")).await;
    let ziel = Ziel::http("ollama", &format!("{}/api/generate", server.adresse()), "m");
    ollama::entladen(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
    )
    .await;
    assert_eq!(server.treffer(), 1);
    assert!(server.anfragen()[0].zeile.starts_with("POST /api/generate"));
    // Ohne Server: kein Fehler, kein Hänger.
    let tot = Ziel::http("ollama", "http://127.0.0.1:1/api/generate", "m");
    ollama::entladen(
        freigabe_uebergang(&tot).freigabe,
        &tot,
        &quellen_uebergang(),
    )
    .await;
}

#[tokio::test]
async fn die_modellliste_kennt_beide_formate_und_meldet_fehler_ohne_text() {
    let server = mock(|anfrage| match anfrage.nr {
        0 => RohAntwort::neu(
            200,
            "application/json",
            vec![br#"{"data":[{"id":"a"},{"name":"b"},{"x":1}]}"#.to_vec()],
        ),
        1 => RohAntwort::neu(200, "application/json", vec![br#"["c","d"]"#.to_vec()]),
        _ => RohAntwort::text(401, GEHEIM),
    })
    .await;
    let (ziel, anbieter) = lokal(&server.basis());
    let quellen = quellen_uebergang();
    let liste = || {
        modelle::liste(
            freigabe_uebergang(&ziel).freigabe,
            &ziel,
            &quellen,
            &anbieter,
            "",
        )
    };
    assert_eq!(liste().await.unwrap(), vec!["a", "b"]);
    assert_eq!(liste().await.unwrap(), vec!["c", "d"]);
    let fehlertext = liste().await.unwrap_err();
    assert!(fehlertext.contains("401"), "{fehlertext}");
    assert!(!fehlertext.contains(GEHEIM), "{fehlertext}");
    assert!(server.anfragen()[0].zeile.starts_with("GET /v1/models"));
}
