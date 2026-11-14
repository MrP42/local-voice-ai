//! Rohes JSON-POST für Aufrufer mit eigener Antwortauswertung (lva#604, Teilpaket 3).
//!
//! Der Agent (`agent/runtime.rs`), die Einbettungen (`managers/meetings/search/embed.rs`) und die
//! Tokenzählung (`managers/llm/mod.rs`) hatten je einen eigenen Client ohne Redirect- und
//! Proxyvorgabe. Sie senden jetzt über diesen einen Weg: **ein** Request an das gebundene
//! [`Ziel`], die Freigabe wird eingelöst, die Route nicht neu aufgelöst, kein Redirect, die
//! lokale Route ohne Proxy.
//!
//! Der Transport wertet nichts fachlich aus. Er liefert Status, ob eine Fehlerantwort einen
//! Kontextüberlauf meldet, und den JSON-Körper einer erfolgreichen Antwort
//! ([`JsonAntwort`](crate::llm_versand::anfrage::JsonAntwort)). Der Text einer Fehlerantwort
//! (er kann Eingabetext zitieren) wird bis [`FEHLER_KOERPER_MAX`](super::http) Byte gelesen,
//! klassifiziert und verworfen; er gelangt weder in einen Fehler noch ins Log.

use std::time::Duration;

use super::http::{basis, client, ist_ueberlauf, koerper_anfang, sendefehler};
use crate::llm_versand::anfrage::JsonAntwort;
use crate::llm_versand::fehler::VersandFehler;
use crate::llm_versand::pruefer::Versandfreigabe;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;

/// Ein JSON-POST an `<Basis des Ziels><pfad>` (`pfad` mit führendem `/`), mit Gesamtzeit
/// `timeout`. Nur ein Verbindungsfehler oder eine abgelaufene Zeit ist ein Fehler; jeder
/// HTTP-Status ist eine [`JsonAntwort`].
pub(in crate::llm_versand) async fn post(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    pfad: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<JsonAntwort, VersandFehler> {
    freigabe.einloesen(ziel, quellen)?;
    let url = format!("{}{pfad}", basis(ziel)?.trim_end_matches('/'));
    let client = client(ziel, None, Some(timeout))?;
    let antwort = client
        .post(&url)
        .json(body)
        .send()
        .await
        .map_err(|e| sendefehler(&e))?;
    let status = antwort.status();
    if !status.is_success() {
        let koerper = koerper_anfang(antwort).await;
        let ueberlauf = ist_ueberlauf(&koerper);
        log::info!("JSON-Anfrage abgelehnt (Status {})", status.as_u16());
        return Ok(JsonAntwort {
            status: status.as_u16(),
            ueberlauf,
            json: None,
        });
    }
    let json = match antwort.json::<serde_json::Value>().await {
        Ok(json) => Some(json),
        // Die Zeit lief beim Lesen des Körpers ab: wie beim Senden ein Zeitfehler.
        Err(fehler) if fehler.is_timeout() => return Err(VersandFehler::Zeitueberschreitung),
        Err(_) => None,
    };
    Ok(JsonAntwort {
        status: status.as_u16(),
        ueberlauf: false,
        json,
    })
}
