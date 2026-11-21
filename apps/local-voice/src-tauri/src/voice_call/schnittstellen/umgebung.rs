//! Aufrufzeitliche Konfiguration; fehlende Profile bleiben geschlossen.
use std::sync::Arc;

use super::anbindung::{Abbruch, Antwortrunde};
use super::aufrufkern::{fuehre_aus, Aufrufergebnis};
use super::bestaetigung::Freigabe;
use super::kodierung::Wert;
use super::richtlinie::{Richtlinie, Verweigernd};
use super::transport::{SystemDns, TcpTlsAufbau};

pub(crate) trait Aufrufumgebung: Send + Sync {
    fn eintraege(&self) -> Vec<Wert> {
        Vec::new()
    }
    fn aktive_ids(&self) -> Vec<String> {
        Vec::new()
    }
    fn assistent(&self) -> String {
        "lokal".into()
    }
    fn sprache(&self) -> String {
        "de".into()
    }
    fn zeit(&self) -> String {
        super::anbindung::zeit_jetzt_utc()
    }
    fn richtlinie(&self) -> Arc<dyn Richtlinie + Send + Sync> {
        Arc::new(Verweigernd)
    }
    // Die Methode kapselt konkrete Adapter, damit Testadapter keinen Netzweg brauchen.
    #[allow(clippy::too_many_arguments)]
    fn fuehre(
        &self,
        eintraege: &[Wert],
        ids: &[String],
        name: &str,
        argumente: &str,
        runde: &mut Antwortrunde,
        abbruch: &Abbruch,
        frage: Option<(&Freigabe, u64)>,
    ) -> Aufrufergebnis;
}

pub(crate) struct VerweigerndeUmgebung;

impl Aufrufumgebung for VerweigerndeUmgebung {
    fn fuehre(
        &self,
        eintraege: &[Wert],
        ids: &[String],
        name: &str,
        argumente: &str,
        runde: &mut Antwortrunde,
        abbruch: &Abbruch,
        frage: Option<(&Freigabe, u64)>,
    ) -> Aufrufergebnis {
        fuehre_aus(
            eintraege,
            ids,
            name,
            argumente,
            runde,
            abbruch,
            frage,
            &Verweigernd,
            &mut SystemDns,
            &mut TcpTlsAufbau,
            &self.assistent(),
            &self.sprache(),
            &self.zeit(),
        )
    }
}
