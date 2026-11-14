//! Der Versandablauf `llm_versand::versand` ist privat; öffentlich ist nur der Reexport
//! `versenden`.

#[cfg(not(lva_sonde = "kontrolle"))]
use crate::llm_versand::versand::versenden;
#[cfg(lva_sonde = "kontrolle")]
use crate::llm_versand::versenden;

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
