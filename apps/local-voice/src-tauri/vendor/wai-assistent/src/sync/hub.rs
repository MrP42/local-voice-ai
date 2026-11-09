//! Abstraktion des Sync-Hubs (wai-portal). Der Kern kennt keinen Transport; Tests nutzen [`super::memhub::MemHub`].
//!
//! Vertrag des Hubs (ADR-0028 E3.0 bis E3.3, E3.8): Er sieht nur die Steuerfelder, vergibt `revision`, nutzt sie
//! ausschliesslich fuer das atomare Vergleichsschreiben, ordnet Schreibvorgaenge ueber die
//! Geraete-Authentifizierung (`AuthGeraet`, nicht aus dem Umschlag) dem Geraet zu, weist widerrufene Geraete und alte
//! Epochen ab und weist nach einem Grabstein jedes spaetere Objekt der Unterhaltung ab.
//!
//! **Der Kern vertraut keiner dieser Regeln** (E3.8, boesartiger Hub): Sie sind Verteidigung in der Tiefe und
//! entlasten die Geraete. Integritaet und Vertraulichkeit haengen nur an Signaturen, dem gepruften
//! Geraeteverzeichnis, Kettengliedern und lokalen Hoechststaenden; der Hub kann nur zurueckhalten.

use std::sync::Arc;

use thiserror::Error;

use super::kennung::Hex128;
use super::paket::Schluesselpaket;
use super::umschlag::{Sammlung, Umschlag};
use super::verzeichnis::Verzeichniseintrag;

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
    #[error("Verzeichnis hat schon {laenge} Eintraege")]
    VerzeichnisKonflikt { laenge: u64 },
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
/// Transportgrenzen (E3.8, Must-fix 7): Eine Netz-Implementierung liest Objekte nur ueber
/// [`Umschlag::aus_json`] bzw. [`Umschlag::liste_aus_json`], [`Verzeichniseintrag::aus_json`] und
/// [`Schluesselpaket::aus_json`]; dort ist die Rohlaenge vor jeder Deserialisierung begrenzt.
///
/// Hubregeln: Nur zugelassene, nicht widerrufene Geraete schreiben und lesen; beim Schreiben muss
/// `umschlag.geraet_id == auth` gelten. Objekte mit `key_epoch` unter der Mindestepoche oder ueber der
/// aktuellen Epoche werden abgewiesen. Zulassung, Widerruf und Epoche leitet der Hub aus dem
/// Geraeteverzeichnis ab (Aufnahme, Widerruf); die Geraete pruefen es selbst.
pub trait HubSpeicher {
    /// Erstschreiben (unveraenderliche Sammlungen; bei veraenderlichen nur, solange das Objekt fehlt).
    fn schreiben(&self, auth: &AuthGeraet, umschlag: &Umschlag) -> Result<Schreiben, HubFehler>;

    /// Vergleichsschreiben: nur wenn die gespeicherte Revision `erwartete_revision` ist
    /// (`None` = Objekt darf noch nicht existieren). Genau ein gleichzeitiger Schreiber gewinnt.
    /// Gibt die neue Revision zurueck. Die Revision hat keine Fencing- und keine Versionsbedeutung.
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

    /// Alle aufbewahrten Fassungen eines veraenderlichen Objekts (Kopf, Lease), die Belege fuer die
    /// lueckenlose Kette (E3.8, Must-fix 1). Der Hub bewahrt jede Fassung auf, die er je gespeichert hat;
    /// das Geraet prueft jede Fassung selbst (Signatur, Glied, Abstammung). Fehlende Belege fuehren nur zum
    /// Zurueckhalten, nie zum Uebernehmen. Standard: keine Belege.
    fn fassungen_lesen(
        &self,
        _auth: &AuthGeraet,
        _sammlung: Sammlung,
        _objekt_id: &Hex128,
    ) -> Result<Vec<Umschlag>, HubFehler> {
        Ok(Vec::new())
    }

    /// Objekte (inklusive Loeschmarken) nach Revision aufsteigend, jeweils mit gesetzter `revision`.
    fn lesen(&self, auth: &AuthGeraet, filter: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler>;

    /// Haengt einen signierten Verzeichniseintrag an, nur wenn das Verzeichnis genau `eintrag.nummer`
    /// Eintraege hat (Vergleichsschreiben auf die Laenge; sonst `VerzeichnisKonflikt`). Ein identischer
    /// Eintrag an derselben Stelle ist idempotent erfolgreich (verlorene Antwort). Ein Widerruf sperrt das
    /// Geraet und setzt aktuelle und Mindestepoche auf `epoche`.
    fn verzeichnis_anhaengen(
        &self,
        auth: &AuthGeraet,
        eintrag: &Verzeichniseintrag,
    ) -> Result<(), HubFehler>;

    /// Verzeichniseintraege ab Nummer `ab`, aufsteigend.
    fn verzeichnis_lesen(
        &self,
        auth: &AuthGeraet,
        ab: u64,
    ) -> Result<Vec<Verzeichniseintrag>, HubFehler>;

    /// Legt gewrappte Epochenschluessel ab (idempotent je Konto, Epoche und Empfaenger).
    fn pakete_ablegen(
        &self,
        auth: &AuthGeraet,
        pakete: &[Schluesselpaket],
    ) -> Result<(), HubFehler>;

    /// Gewrappte Epochenschluessel fuer das aufrufende Geraet (`empfaenger == auth`).
    fn pakete_lesen(&self, auth: &AuthGeraet) -> Result<Vec<Schluesselpaket>, HubFehler>;
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
        fn fassungen_lesen(
            &self,
            a: &AuthGeraet,
            s: Sammlung,
            id: &Hex128,
        ) -> Result<Vec<Umschlag>, HubFehler> {
            (**self).fassungen_lesen(a, s, id)
        }
        fn verzeichnis_anhaengen(
            &self,
            a: &AuthGeraet,
            e: &Verzeichniseintrag,
        ) -> Result<(), HubFehler> {
            (**self).verzeichnis_anhaengen(a, e)
        }
        fn verzeichnis_lesen(
            &self,
            a: &AuthGeraet,
            ab: u64,
        ) -> Result<Vec<Verzeichniseintrag>, HubFehler> {
            (**self).verzeichnis_lesen(a, ab)
        }
        fn pakete_ablegen(&self, a: &AuthGeraet, p: &[Schluesselpaket]) -> Result<(), HubFehler> {
            (**self).pakete_ablegen(a, p)
        }
        fn pakete_lesen(&self, a: &AuthGeraet) -> Result<Vec<Schluesselpaket>, HubFehler> {
            (**self).pakete_lesen(a)
        }
    };
}

impl<H: HubSpeicher + ?Sized> HubSpeicher for &H {
    hub_delegieren!();
}

impl<H: HubSpeicher + ?Sized> HubSpeicher for Arc<H> {
    hub_delegieren!();
}
