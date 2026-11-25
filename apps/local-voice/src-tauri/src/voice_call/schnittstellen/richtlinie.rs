//! Reine Freigabepolitik; eine freie Souveraenitaetsstufe allein gibt keine Daten frei.
//! B2.6 muss vor dem Modellversand zusaetzlich `sovereignty_allows` aus `turn.rs`
//! anwenden. Die Richtlinie ersetzt diese Pruefung nicht.
use std::cell::Cell;
use std::net::IpAddr;

use super::anbindung::{Antwort, Lauf, Verbindung, Verbindungsaufbau, Ziel};
use super::aufrufkern::Aufrufergebnis;
use super::kodierung::Wert;
use super::rahmung;

pub(crate) trait Richtlinie {
    fn netzabfluss_erlaubt(&self, kennung: &str) -> bool;
    fn widerrufen(&self, kennung: &str) -> bool;
    fn cloud_werkzeugdefinitionen_erlaubt(&self) -> bool;
    fn cloud_ergebnis_erlaubt(&self, kennung: &str) -> bool;
    fn geschuetzt(&self) -> bool;
}

pub(crate) struct Widerrufspruefung<'a, V, R> {
    pub verbindung: &'a mut V,
    pub richtlinie: &'a R,
    pub kennung: &'a str,
    pub blockiert: &'a Cell<bool>,
}

pub(crate) struct GepruefteVerbindung<'a, V, R> {
    verbindung: V,
    richtlinie: &'a R,
    kennung: &'a str,
    blockiert: &'a Cell<bool>,
}

impl<'a, V: Verbindungsaufbau, R: Richtlinie> Verbindungsaufbau for Widerrufspruefung<'a, V, R> {
    type Verbunden = GepruefteVerbindung<'a, V::Verbunden, R>;

    fn verbinden(
        &mut self,
        ziel: &Ziel,
        ip: IpAddr,
        lauf: &Lauf,
    ) -> Result<Self::Verbunden, &'static str> {
        Ok(GepruefteVerbindung {
            verbindung: self.verbindung.verbinden(ziel, ip, lauf)?,
            richtlinie: self.richtlinie,
            kennung: self.kennung,
            blockiert: self.blockiert,
        })
    }
}

impl<V: Verbindung, R: Richtlinie> Verbindung for GepruefteVerbindung<'_, V, R> {
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str> {
        // Auch Widerrufe waehrend DNS und Verbindungsaufbau unmittelbar vor Versand beachten.
        if self.richtlinie.widerrufen(self.kennung) {
            self.blockiert.set(true);
            return Err("abgebrochen");
        }
        self.verbindung.senden(anfrage)
    }

    fn antwort(&mut self, lauf: &Lauf, max_bytes: usize) -> Result<Antwort, &'static str> {
        self.verbindung.antwort(lauf, max_bytes)
    }

    fn gesendet_bytes(&self) -> usize {
        self.verbindung.gesendet_bytes()
    }

    fn empfangen_bytes(&self) -> usize {
        self.verbindung.empfangen_bytes()
    }
}

/// Fehlende Konfiguration erlaubt weder Aufrufe noch einen Cloud-Abfluss.
pub(crate) struct Verweigernd;
impl Richtlinie for Verweigernd {
    fn netzabfluss_erlaubt(&self, _: &str) -> bool {
        false
    }
    fn widerrufen(&self, _: &str) -> bool {
        true
    }
    fn cloud_werkzeugdefinitionen_erlaubt(&self) -> bool {
        false
    }
    fn cloud_ergebnis_erlaubt(&self, _: &str) -> bool {
        false
    }
    fn geschuetzt(&self) -> bool {
        true
    }
}

pub(crate) fn fuer_modell(
    mut erg: Aufrufergebnis,
    cloud_modell: bool,
    kennung: &str,
    richtlinie: &impl Richtlinie,
    sprache: &str,
) -> Aufrufergebnis {
    if cloud_modell && (richtlinie.geschuetzt() || !richtlinie.cloud_ergebnis_erlaubt(kennung)) {
        erg.daten = None;
        erg.meldung = Some(
            if rahmung::englisch(sprache) {
                "The result must not be given to a cloud model and will not be stated."
            } else {
                "Das Ergebnis darf nicht an ein Cloud-Modell gegeben werden, es wird nicht genannt."
            }
            .into(),
        );
    }
    erg
}

/// Filtert bereits mit `werkzeug::werkzeug` gebaute Definitionen ohne Neubau.
pub(crate) fn werkzeugdefinitionen_fuer_modell<'a>(
    definitionen: &'a [String],
    cloud_modell: bool,
    richtlinie: &impl Richtlinie,
) -> &'a [String] {
    if cloud_modell && (richtlinie.geschuetzt() || !richtlinie.cloud_werkzeugdefinitionen_erlaubt())
    {
        &[]
    } else {
        definitionen
    }
}
