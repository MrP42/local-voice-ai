//! `Versandfreigabe` hat kein `Clone`: eine Freigabe lässt sich nicht vervielfältigen.

use crate::llm_versand::pruefer::freigabe_uebergang;
use crate::llm_versand::ziel::Ziel;

fn zwei_freigaben() {
    let ziel = Ziel::http("anbieter", "https://api.example.invalid/v1", "modell");
    let freigabe = freigabe_uebergang(&ziel).freigabe;
    #[cfg(not(lva_sonde = "kontrolle"))]
    let zweite = freigabe.clone();
    #[cfg(lva_sonde = "kontrolle")]
    let zweite = freigabe_uebergang(&ziel).freigabe;
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
