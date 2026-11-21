//! Eine eingelöste Freigabe ist verbraucht: das zweite Einlösen ist ein Zugriff nach dem Move.

use crate::llm_versand::pruefer::freigabe_uebergang;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;

fn einloesen() {
    let ziel = Ziel::http("anbieter", "https://api.example.invalid/v1", "modell");
    let quellen = Quellen::keine();
    let freigabe = freigabe_uebergang(&ziel).freigabe;
    let erstes = freigabe.einloesen(&ziel, &quellen);
    #[cfg(not(lva_sonde = "kontrolle"))]
    let zweites = freigabe.einloesen(&ziel, &quellen);
    #[cfg(lva_sonde = "kontrolle")]
    let zweites = freigabe_uebergang(&ziel)
        .freigabe
        .einloesen(&ziel, &quellen);
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
