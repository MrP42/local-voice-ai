//! Die Quelle der App-Richtlinie liegt im privaten Untermodul `pruefer::app`; nach außen führt
//! nur der Reexport `pruefer::AppFreigabeQuelle`.

#[cfg(not(lva_sonde = "kontrolle"))]
use crate::llm_versand::pruefer::app;
#[cfg(lva_sonde = "kontrolle")]
use crate::llm_versand::pruefer::AppFreigabeQuelle;

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
