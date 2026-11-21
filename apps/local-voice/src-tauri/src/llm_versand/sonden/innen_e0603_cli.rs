//! Der rohe CLI-Start `transport::cli` ist auch **innerhalb** von `llm_versand` privat: erreichbar
//! ist nur der Durchreich-Einstieg `transport::cli_roh`, der eine Freigabe einlöst.

#[cfg(not(lva_sonde = "kontrolle"))]
use crate::llm_versand::transport::cli;
#[cfg(lva_sonde = "kontrolle")]
use crate::llm_versand::transport::cli_roh;

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
