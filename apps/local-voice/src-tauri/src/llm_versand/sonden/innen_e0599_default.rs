//! `Versandfreigabe` hat kein `Default`: es gibt keinen Weg zu einer leeren Freigabe.

use crate::llm_versand::pruefer::{freigabe_uebergang, Versandfreigabe};
use crate::llm_versand::ziel::Ziel;

fn freigabe() -> Versandfreigabe {
    let ziel = Ziel::http("anbieter", "https://api.example.invalid/v1", "modell");
    #[cfg(not(lva_sonde = "kontrolle"))]
    return Versandfreigabe::default();
    #[cfg(lva_sonde = "kontrolle")]
    return freigabe_uebergang(&ziel).freigabe;
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
