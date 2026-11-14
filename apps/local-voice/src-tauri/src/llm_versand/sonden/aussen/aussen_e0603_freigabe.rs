//! `Versandfreigabe` ist nur innerhalb von `llm_versand` benennbar (`pub(super)`): von außen
//! kann niemand eine annehmen, halten oder ausstellen. Öffentlich im Prüfer ist z. B. `Lesung`.

#[cfg(lva_sonde = "kontrolle")]
use crate::llm_versand::pruefer::Lesung;
#[cfg(not(lva_sonde = "kontrolle"))]
use crate::llm_versand::pruefer::Versandfreigabe;

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
