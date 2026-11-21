//! Plattformneutraler Kern von `lva.schnittstellen/1` (lva#558).
//!
//! Noch nicht in das Gespraech verdrahtet: deshalb ist `dead_code` fuer diesen
//! gesamten, intern sichtbaren Kern erlaubt. Keine Commands, Speicherung oder Netzclients.
//! Die Transport-Traits beschreiben die Sicherheitsgrenze fuer eine spaetere Anbindung.
//!
//! Offen: Ausfuehrung von cargo test/clippy/rustfmt erfolgt durch den Auftraggeber.
//! App/MCP-Ausfuehrung und konkrete Transportadapter sind bewusst nicht Teil von G4.
#![allow(dead_code)]

pub(crate) mod adressen;
pub(crate) mod anbindung;
pub(crate) mod anfrage;
pub(crate) mod bestaetigung;
pub(crate) mod bindung;
pub(crate) mod kodierung;
pub(crate) mod rahmung;
pub(crate) mod verzeichnis;
pub(crate) mod werkzeug;

#[cfg(test)]
mod tests;

use kodierung::Wert;

fn objekt(felder: impl IntoIterator<Item = (impl Into<String>, Wert)>) -> Wert {
    Wert::Objekt(felder.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

fn fehler(code: &str, parameter: Option<&str>) -> Wert {
    objekt([
        ("code", Wert::from(code)),
        ("parameter", parameter.map(Wert::from).unwrap_or(Wert::Null)),
    ])
}

fn ergebnis(erg: Result<Wert, Wert>) -> Wert {
    match erg {
        Ok(w) => objekt([("ok", w)]),
        Err(w) => objekt([("fehler", w)]),
    }
}
