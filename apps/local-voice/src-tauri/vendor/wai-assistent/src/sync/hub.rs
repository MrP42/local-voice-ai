//! Abstraktion des Sync-Hubs (wai-portal). Der Kern kennt keinen Transport; Tests nutzen [`super::memhub::MemHub`].
//!
//! Vertrag des Hubs (ADR-0028 E3.0 bis E3.3): Er sieht nur die Steuerfelder, vergibt `revision`, nutzt sie
//! ausschliesslich fuer das atomare Vergleichsschreiben, ordnet Schreibvorgaenge ueber die
//! Geraete-Authentifizierung (`AuthGeraet`, nicht aus dem Umschlag) dem Geraet zu, weist widerrufene Geraete und alte
//! Epochen ab und weist nach einem Grabstein jedes spaetere Objekt der Unterhaltung ab.

use std::sync::Arc;

use thiserror::Error;

use super::kennung::Hex128;
use super::umschlag::{Sammlung, Umschlag};

/// Vom Transport authentifizierte Geraeteidentitaet (z. B. aus dem Geraete-Token des Hubs).
///
/// Sie stammt nie aus dem Umschlag: ein Gast mit eigenem Zugang kann sich nicht als anderes Geraet
/// ausgeben, weil der Hub `umschlag.geraet_id == auth` erzwingt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthGeraet(String);

impl AuthGeraet {
    /// Nur der Transport (oder ein Test, der ihn nachstellt) erzeugt die Identitaet.
    pub fn new(geraet_id: &str) -> Self {
        Self(geraet_id.to_owned())
    }

    pub fn id(&self) -> &str {
        &self.0
    }
}

/// Ablehnungsgruende des Hubs (ohne Inhalte).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HubFehler {
    #[error("Geraet wurde widerrufen")]
    Widerrufen,
    #[error("Geraet ist am Hub nicht zugelassen")]
    NichtZugelassen,
    #[error("geraet_id im Umschlag passt nicht zur authentifizierten Identitaet")]
    GeraetStimmtNichtUeberein,
    #[error("Schluesselepoche veraltet (Mindestepoche {mindest})")]
    EpocheVeraltet { mindest: u64 },
    #[error("Schluesselepoche gibt es noch nicht (aktuell {aktuell})")]
    EpocheVoraus { aktuell: u64 },
    #[error("Epochenwechsel verloren: Hub steht auf Epoche {aktuell}")]
    EpocheKonflikt { aktuell: u64 },
    #[error("durch Grabstein geloescht, Objekt abgewiesen")]
    GrabsteinAbgewiesen,
    #[error("Revision stimmt nicht (aktuell: {aktuell:?})")]
    RevisionKonflikt { aktuell: Option<u64> },
    #[error("Sammlung ist unveraenderlich")]
    NichtVeraenderlich,
    #[error("Objekt verletzt den Vertrag: {0}")]
    Vertrag(&'static str),
    #[error("Hub nicht erreichbar")]
    Nichtverfuegbar,
}

/// Ergebnis eines Erstschreibens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schreiben {
    /// Neu gespeichert mit dieser Revision.
    Neu(u64),
    /// Objekt-ID war schon vorhanden (idempotent, nichts ueberschrieben).
    Vorhanden(u64),
}

/// Pull-Filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeseFilter {
    /// Nur Objekte mit groesserer Revision.
    pub nach_revision: u64,
    pub unterhaltung_ref: Option<Hex128>,
    pub sammlung: Option<Sammlung>,
}

/// Speicher des Hubs. Jede Methode bekommt die authentifizierte Identitaet des Aufrufers.
///
/// Hubregeln: Nur zugelassene, nicht widerrufene Geraete schreiben und lesen; beim Schreiben muss
/// `umschlag.geraet_id == auth` gelten. Objekte mit `key_epoch` unter der Mindestepoche oder ueber der
/// aktuellen Epoche werden abgewiesen. Die aktuelle Epoche vergibt der Hub per Vergleichsschreiben.
pub trait HubSpeicher {
    /// Erstschreiben (unveraenderliche Sammlungen; bei veraenderlichen nur, solange das Objekt fehlt).
    fn schreiben(&self, auth: &AuthGeraet, umschlag: &Umschlag) -> Result<Schreiben, HubFehler>;

    /// Vergleichsschreiben: nur wenn die gespeicherte Revision `erwartete_revision` ist
    /// (`None` = Objekt darf noch nicht existieren). Genau ein gleichzeitiger Schreiber gewinnt.
    /// Gibt die neue Revision zurueck. Die Revision hat keine Fencing-Bedeutung.
    fn cas(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
        erwartete_revision: Option<u64>,
        neuer_umschlag: &Umschlag,
    ) -> Result<u64, HubFehler>;

    /// Einzelnes Objekt samt Revision.
    fn holen(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
    ) -> Result<Option<Umschlag>, HubFehler>;

    /// Objekte (inklusive Loeschmarken) nach Revision aufsteigend, jeweils mit gesetzter `revision`.
    fn lesen(&self, auth: &AuthGeraet, filter: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler>;

    /// Gerätewiderruf als Vergleichsschreiben auf die Epoche: nur wenn der Hub auf
    /// `erwartete_epoche` steht, wird `widerrufenes_geraet` gesperrt, die Epoche um 1 erhoeht und als
    /// Mindestepoche gesetzt. Gibt die neue Epoche zurueck. Zwei gleichzeitige Widerrufe erhalten nie
    /// dieselbe Epochennummer (der zweite bekommt `EpocheKonflikt`). Nur durch ein zugelassenes Geraet.
    ///
    /// `vorgang` ist die vom Aufrufer vergebene Vorgangskennung. Der Hub speichert sie zur neuen Epoche;
    /// eine Wiederholung mit derselben Kennung und demselben Geraet ist idempotent und liefert Erfolg mit
    /// derselben Epoche (verlorene Antwort), auch wenn `erwartete_epoche` inzwischen veraltet ist.
    fn widerrufen(
        &self,
        auth: &AuthGeraet,
        widerrufenes_geraet: &str,
        erwartete_epoche: u64,
        vorgang: &Hex128,
    ) -> Result<u64, HubFehler>;

    /// Aktuelle Schluesselepoche des Kontos (Quelle der Wahrheit).
    fn epoche(&self, auth: &AuthGeraet) -> Result<u64, HubFehler>;

    /// Zugelassene, nicht widerrufene Geraete: nur an diese Liste geht ein neuer Epochenschluessel.
    fn zugelassene(&self, auth: &AuthGeraet) -> Result<Vec<String>, HubFehler>;
}

macro_rules! hub_delegieren {
    () => {
        fn schreiben(&self, a: &AuthGeraet, u: &Umschlag) -> Result<Schreiben, HubFehler> {
            (**self).schreiben(a, u)
        }
        fn cas(
            &self,
            a: &AuthGeraet,
            s: Sammlung,
            id: &Hex128,
            e: Option<u64>,
            u: &Umschlag,
        ) -> Result<u64, HubFehler> {
            (**self).cas(a, s, id, e, u)
        }
        fn holen(
            &self,
            a: &AuthGeraet,
            s: Sammlung,
            id: &Hex128,
        ) -> Result<Option<Umschlag>, HubFehler> {
            (**self).holen(a, s, id)
        }
        fn lesen(&self, a: &AuthGeraet, f: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
            (**self).lesen(a, f)
        }
        fn widerrufen(
            &self,
            a: &AuthGeraet,
            w: &str,
            e: u64,
            v: &Hex128,
        ) -> Result<u64, HubFehler> {
            (**self).widerrufen(a, w, e, v)
        }
        fn epoche(&self, a: &AuthGeraet) -> Result<u64, HubFehler> {
            (**self).epoche(a)
        }
        fn zugelassene(&self, a: &AuthGeraet) -> Result<Vec<String>, HubFehler> {
            (**self).zugelassene(a)
        }
    };
}

impl<H: HubSpeicher + ?Sized> HubSpeicher for &H {
    hub_delegieren!();
}

impl<H: HubSpeicher + ?Sized> HubSpeicher for Arc<H> {
    hub_delegieren!();
}
