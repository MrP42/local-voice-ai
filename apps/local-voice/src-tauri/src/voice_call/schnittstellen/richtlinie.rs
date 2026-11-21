//! Reine Freigabepolitik; eine freie Souveraenitaetsstufe allein gibt keine Daten frei.
//! B2.6 muss vor dem Modellversand zusaetzlich `sovereignty_allows` aus `turn.rs`
//! anwenden. Die Richtlinie ersetzt diese Pruefung nicht.
use super::aufrufkern::Aufrufergebnis;
use super::rahmung;

pub(crate) trait Richtlinie {
    fn netzabfluss_erlaubt(&self, kennung: &str) -> bool;
    fn widerrufen(&self, kennung: &str) -> bool;
    fn cloud_werkzeugdefinitionen_erlaubt(&self) -> bool;
    fn cloud_ergebnis_erlaubt(&self, kennung: &str) -> bool;
    fn geschuetzt(&self) -> bool;
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
