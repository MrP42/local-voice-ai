//! Was Local Voice AI an Besprechungen, Ordnern und Buechern fuer die Projekte bereitstellt.
//!
//! Der Dienst kennt nur dieses Trait; die App liefert [`ProdQuellen`] (Besprechungsspeicher und
//! Bibliothek), Tests liefern echte Speicher in einem Tempverzeichnis. Die Buecher werden nur
//! gelesen (Bibliothek, lva#147 bis #152 gehoeren einem anderen Faden).

use crate::managers::meetings::store::MeetingStore;
use std::sync::Arc;

/// Kennung und Titel eines Inhalts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inhalt {
    pub id: String,
    pub titel: String,
}

/// Besprechungsordner mit dem Verweis auf sein Suite-Projekt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdnerKurz {
    pub id: String,
    pub name: String,
    pub projekt_id: Option<String>,
}

pub trait LvaQuellen: Send + Sync {
    /// Lebende Besprechungen.
    fn besprechungen(&self) -> Result<Vec<Inhalt>, String>;
    /// Buecher der Bibliothek.
    fn buecher(&self) -> Result<Vec<Inhalt>, String>;
    /// Lebende Besprechungsordner.
    fn ordner(&self) -> Result<Vec<OrdnerKurz>, String>;
    /// Kennungen der lebenden Besprechungen eines Ordners.
    fn besprechungen_im_ordner(&self, ordner_id: &str) -> Result<Vec<String>, String>;
    /// Setzt oder loest den Verweis des Ordners auf sein Suite-Projekt.
    fn ordner_projekt_setzen(&self, ordner_id: &str, projekt_id: Option<&str>) -> Result<(), String>;
    /// Loest den Verweis aller Ordner auf dieses Suite-Projekt; liefert deren Zahl.
    fn ordner_projekt_loesen_alle(&self, projekt_id: &str) -> Result<u32, String>;
    /// Suite-Projekte, auf die die Ordner einer Besprechung zeigen.
    fn projekte_der_besprechung(&self, meeting_id: &str) -> Result<Vec<String>, String>;
}

type BuecherFn = dyn Fn() -> Result<Vec<Inhalt>, String> + Send + Sync;

/// Produktion: Besprechungsspeicher der App; die Buecher liefert die Bibliothek ueber `buecher`.
pub struct ProdQuellen {
    meetings: Arc<MeetingStore>,
    buecher: Arc<BuecherFn>,
}

impl ProdQuellen {
    pub fn neu(
        meetings: Arc<MeetingStore>,
        buecher: impl Fn() -> Result<Vec<Inhalt>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            meetings,
            buecher: Arc::new(buecher),
        }
    }
}

fn text(e: anyhow::Error) -> String {
    e.to_string()
}

impl LvaQuellen for ProdQuellen {
    fn besprechungen(&self) -> Result<Vec<Inhalt>, String> {
        Ok(self
            .meetings
            .besprechungen_kurz()
            .map_err(text)?
            .into_iter()
            .map(|(id, titel)| Inhalt { id, titel })
            .collect())
    }

    fn buecher(&self) -> Result<Vec<Inhalt>, String> {
        (self.buecher)()
    }

    fn ordner(&self) -> Result<Vec<OrdnerKurz>, String> {
        Ok(self
            .meetings
            .ordner_verweise()
            .map_err(text)?
            .into_iter()
            .map(|o| OrdnerKurz {
                id: o.id,
                name: o.name,
                projekt_id: o.projekt_id,
            })
            .collect())
    }

    fn besprechungen_im_ordner(&self, ordner_id: &str) -> Result<Vec<String>, String> {
        self.meetings.besprechungen_im_ordner(ordner_id).map_err(text)
    }

    fn ordner_projekt_setzen(&self, ordner_id: &str, projekt_id: Option<&str>) -> Result<(), String> {
        self.meetings
            .ordner_projekt_setzen(ordner_id, projekt_id)
            .map_err(text)
    }

    fn ordner_projekt_loesen_alle(&self, projekt_id: &str) -> Result<u32, String> {
        self.meetings.ordner_projekt_loesen_alle(projekt_id).map_err(text)
    }

    fn projekte_der_besprechung(&self, meeting_id: &str) -> Result<Vec<String>, String> {
        self.meetings.projekte_der_besprechung(meeting_id).map_err(text)
    }
}
