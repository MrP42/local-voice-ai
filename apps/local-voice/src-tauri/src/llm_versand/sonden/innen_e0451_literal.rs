//! Die Felder von `Versandfreigabe` sind privat: auch innerhalb von `llm_versand` lässt sich
//! keine Freigabe als Struktur-Literal bauen.

use crate::llm_versand::pruefer::{freigabe_uebergang, Versandfreigabe};
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;

fn literal() {
    let ziel = Ziel::http("anbieter", "https://api.example.invalid/v1", "modell");
    #[cfg(not(lva_sonde = "kontrolle"))]
    let freigabe = Versandfreigabe {
        ziel: ziel.clone(),
        quellen: Quellen::keine().fingerabdruck(),
    };
    #[cfg(lva_sonde = "kontrolle")]
    let freigabe = freigabe_uebergang(&ziel).freigabe;
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
