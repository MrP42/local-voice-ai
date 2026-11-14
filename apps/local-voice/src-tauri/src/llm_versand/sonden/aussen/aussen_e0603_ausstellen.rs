//! `freigabe_ausstellen` (die einzige Ausstellung) ist nur innerhalb von `llm_versand`
//! erreichbar. Öffentlich im Prüfer ist z. B. die Quelle `AppFreigabeQuelle`.

#[cfg(not(lva_sonde = "kontrolle"))]
use crate::llm_versand::pruefer::freigabe_ausstellen;
#[cfg(lva_sonde = "kontrolle")]
use crate::llm_versand::pruefer::AppFreigabeQuelle;

#[cfg(lva_sonde = "kontrolle")]
const _: () = super::kontrollmarke();
