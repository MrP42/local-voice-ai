//! Gegenproben am Anwendungseinstieg: gesperrte Aufrufe erreichen den Mock nie.

use super::anwendung::{chat_geprueft, testen};
use super::fehler::grund;
use super::testhilfe::{anbieter, mock, RohAntwort};
use super::{ist_gesperrt_text, testhaken, ChatInhalt, Quellen, VersandFehler};
use crate::llm_client::ChatReply;
use crate::managers::usage::Purpose;
use crate::settings::PostProcessProvider;

async fn fragen(provider: &PostProcessProvider, quellen: &Quellen) -> Result<ChatReply, String> {
    chat_geprueft(
        Purpose::Minutes,
        provider,
        String::new(),
        "m",
        ChatInhalt {
            nutzer: "Frage".into(),
            system: None,
            schema: None,
            reasoning_effort: None,
            reasoning: None,
        },
        quellen,
    )
    .await
}

#[tokio::test]
async fn cloud_ohne_freigabe_sendet_nicht() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let cloud = testen::cloud_einrichten(&server.adresse(), false, &[]);
    let fehler = fragen(&testen::cloud_anbieter(), &Quellen::besprechung("m1"))
        .await
        .err()
        .expect("Cloud ohne Freigabe muss sperren");
    assert!(ist_gesperrt_text(&fehler));
    assert_eq!(cloud.lesungen(), 1, "keine Reparatur oder Wiederholung");
    assert_eq!(server.treffer(), 0);
}

#[tokio::test]
async fn cloud_mit_freigabe_sendet_genau_einmal() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let cloud = testen::cloud_einrichten(&server.adresse(), false, &["m1"]);
    let antwort = fragen(&testen::cloud_anbieter(), &Quellen::besprechung("m1"))
        .await
        .unwrap();
    assert_eq!(antwort.content.as_deref(), Some("Antwort"));
    assert_eq!(cloud.lesungen(), 3, "R1, R2 und R3");
    assert_eq!(server.treffer(), 1);
}

#[tokio::test]
async fn nur_lokal_hat_vorrang_vor_compliance_und_budget() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let _cloud = testen::cloud_einrichten(&server.adresse(), true, &["m1"]);
    testhaken::setze_regelwerk(|_, _, _| Err("Compliance verweigert".into()));
    let fehler = fragen(&testen::cloud_anbieter(), &Quellen::besprechung("m1"))
        .await
        .err()
        .expect("nur lokal muss Cloud sperren");
    assert_eq!(
        fehler,
        VersandFehler::gesperrt(grund::NUR_LOKAL).to_string()
    );
    assert_eq!(server.treffer(), 0);
}

#[tokio::test]
async fn loopback_braucht_keine_cloud_freigabe() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let cloud = testen::cloud_einrichten(&server.adresse(), true, &[]);
    let provider = anbieter("custom", &server.basis());
    let antwort = fragen(&provider, &Quellen::besprechung("m1"))
        .await
        .unwrap();
    assert_eq!(antwort.content.as_deref(), Some("Antwort"));
    assert_eq!(cloud.lesungen(), 0);
    assert_eq!(server.treffer(), 1);
}

#[tokio::test]
async fn keine_quellen_ueberspringt_die_besprechungspruefung_auch_bei_nur_lokal() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let cloud = testen::cloud_einrichten(&server.adresse(), true, &[]);
    let antwort = fragen(&testen::cloud_anbieter(), &Quellen::keine())
        .await
        .unwrap();
    assert_eq!(antwort.content.as_deref(), Some("Antwort"));
    assert_eq!(cloud.lesungen(), 0);
    assert_eq!(server.treffer(), 1);
}

#[tokio::test]
async fn unbestimmte_quellen_sperren_cloud_ohne_transport() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let _cloud = testen::cloud_einrichten(&server.adresse(), false, &["m1"]);
    let fehler = fragen(&testen::cloud_anbieter(), &Quellen::unbestimmt())
        .await
        .err()
        .expect("unbestimmte Herkunft muss Cloud sperren");
    assert_eq!(
        fehler,
        VersandFehler::gesperrt(grund::UNBESTIMMT).to_string()
    );
    assert_eq!(server.treffer(), 0);
}

#[tokio::test]
async fn ohne_installierte_app_quelle_bleibt_cloud_gesperrt() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let _wache = testhaken::wache();
    testhaken::setze_cloud_proxy(&server.adresse());
    testhaken::setze_regelwerk(|_, _, _| Ok(()));
    let fehler = fragen(&testen::cloud_anbieter(), &Quellen::besprechung("m1"))
        .await
        .err()
        .expect("ohne App-Quelle muss Cloud sperren");
    assert_eq!(
        fehler,
        VersandFehler::gesperrt(grund::LESEFEHLER).to_string()
    );
    assert_eq!(server.treffer(), 0);
}

#[tokio::test]
async fn widerruf_wird_beim_naechsten_aufruf_frisch_geprueft() {
    let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
    let cloud = testen::cloud_einrichten(&server.adresse(), false, &[]);
    let provider = testen::cloud_anbieter();
    let quellen = Quellen::besprechung("m1");
    cloud.freigeben("m1");
    assert!(fragen(&provider, &quellen).await.is_ok());
    assert_eq!(server.treffer(), 1);
    cloud.widerrufen("m1");
    let fehler = fragen(&provider, &quellen)
        .await
        .err()
        .expect("Widerruf muss den nächsten Aufruf sperren");
    assert!(ist_gesperrt_text(&fehler));
    assert_eq!(server.treffer(), 1, "der zweite Aufruf sendet nicht");
    assert_eq!(cloud.lesungen(), 4);
}
