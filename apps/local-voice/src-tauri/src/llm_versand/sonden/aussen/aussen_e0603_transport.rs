//! Der Rohtransport `llm_versand::transport` ist von außen nicht erreichbar, auch nicht aus
//! `crate::managers::llm`. Erreichbar ist der öffentliche Vertrag (hier: `Ziel`).

#[cfg(not(lva_sonde = "kontrolle"))]
use crate::llm_versand::transport;
#[cfg(lva_sonde = "kontrolle")]
use crate::llm_versand::ziel;

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
