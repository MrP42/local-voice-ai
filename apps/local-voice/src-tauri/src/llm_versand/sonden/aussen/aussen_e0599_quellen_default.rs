//! `Quellen` hat kein `Default`: die Herkunft muss ausdrücklich gewählt werden (`keine()`,
//! `unbestimmt()`, `besprechung(..)`), ein stilles „leer“ gibt es nicht.

use crate::llm_versand::Quellen;

fn quellen() -> Quellen {
    #[cfg(not(lva_sonde = "kontrolle"))]
    return Quellen::default();
    #[cfg(lva_sonde = "kontrolle")]
    return Quellen::keine();
}

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
