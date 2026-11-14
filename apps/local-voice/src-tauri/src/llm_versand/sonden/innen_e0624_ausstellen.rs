//! `Versandfreigabe::ausstellen` ist privat im Prüfer: auch das übrige `llm_versand` kann nicht
//! selbst eine Freigabe ausstellen, nur `freigabe_ausstellen` nach bestandener Prüfung.

use crate::llm_versand::pruefer::{freigabe_uebergang, Versandfreigabe};
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;

fn ausstellen() {
    let ziel = Ziel::http("anbieter", "https://api.example.invalid/v1", "modell");
    let quellen = Quellen::keine();
    #[cfg(not(lva_sonde = "kontrolle"))]
    let freigabe = Versandfreigabe::ausstellen(&ziel, &quellen);
    #[cfg(lva_sonde = "kontrolle")]
    let freigabe = freigabe_uebergang(&ziel).freigabe;
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
