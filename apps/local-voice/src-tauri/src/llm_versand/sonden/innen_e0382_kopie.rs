//! `Versandfreigabe` ist nicht `Copy`: wer sie weiterreicht, hat sie nicht mehr.

use crate::llm_versand::pruefer::{freigabe_uebergang, Versandfreigabe};
use crate::llm_versand::ziel::Ziel;

fn behalten(freigabe: Versandfreigabe) {}

fn weiterreichen() {
    let ziel = Ziel::http("anbieter", "https://api.example.invalid/v1", "modell");
    let freigabe = freigabe_uebergang(&ziel).freigabe;
    behalten(freigabe);
    #[cfg(not(lva_sonde = "kontrolle"))]
    behalten(freigabe);
    #[cfg(lva_sonde = "kontrolle")]
    behalten(freigabe_uebergang(&ziel).freigabe);
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
