//! Plattformneutraler Kern von `lva.schnittstellen/1` (lva#558).
//!
//! Noch nicht in das Gespraech verdrahtet: deshalb ist `dead_code` fuer diesen
//! gesamten, intern sichtbaren Kern erlaubt. Keine Commands und keine Oberflaeche.
//! Paket A der Verdrahtung (lva#737) liefert die echten Adapter (`transport`: Namensaufloesung,
//! TCP/TLS nur an die gepruefte IP, HTTP/1.1 ohne Proxy/Umleitung/Alt-Svc/Upgrade), den
//! Verzeichnis-Speicher je Geraet (`speicher`), die Uhr in der Freigabe, die Grenzen in
//! `versende` (Zeitlimit, Abbruch, Antwortgroesse, Aufruflimit) und das Protokoll ohne Inhalte.
//!
//! Offen: Verdrahtung ins Gespraech (Befehle, Orchestrierung, Oberflaeche).
//! App/MCP-Ausfuehrung bleibt ausserhalb dieses HTTP/Web-Kerns.
#![allow(dead_code)]

pub(crate) mod adressen;
pub(crate) mod aeusserung;
pub(crate) mod anbindung;
pub(crate) mod anfrage;
pub(crate) mod aufrufkern;
pub(crate) mod bestaetigung;
pub(crate) mod bindung;
pub(crate) mod gespraeche;
pub(crate) mod http1;
pub(crate) mod kodierung;
pub(crate) mod rahmung;
pub(crate) mod richtlinie;
pub(crate) mod speicher;
pub(crate) mod transport;
pub(crate) mod verzeichnis;
pub(crate) mod werkzeug;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_aufrufkern {
    include!("tests_aufrufkern.rs");

    mod tests_richtlinie {
        include!("tests_richtlinie.rs");
    }

    mod tests_aeusserung {
        include!("tests_aeusserung.rs");
    }
}

#[cfg(test)]
mod tests_http1;

#[cfg(test)]
mod tests_gespraeche;

#[cfg(test)]
mod tests_speicher;

#[cfg(test)]
mod tests_transport;

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
