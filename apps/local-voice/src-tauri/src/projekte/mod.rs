//! Suite-Projekte und Recherche in Local Voice AI (ADR-0026 PR-7, Epic wai-sdk#1047).
//!
//! * [`herkunft`]: Verweise `wai://lva/...` und die Momentaufnahme der Urspruenge fuer `wai-projekte`.
//! * [`quellen`]: was die App an Besprechungen, Ordnern und Buechern bereitstellt (austauschbar).
//! * [`dienst`]: Suite-Projekte, Verknuepfung Besprechungsordner und Buecher, Abgleich der Loeschung.
//! * [`recherche`]: Recherche im Modus Schnell ueber `wai-recherche` mit Halt und Freigabe.
//! * [`audit`]: Protokoll ohne Inhalte.
//! * [`befehle`]: die Tauri-Befehle.
//!
//! Das Projektverzeichnis (`projekte.db`) und die Recherche-Dokumente (`recherche.db`) liegen im
//! Datenordner der App und werden nur ueber `wai-projekte` bzw. den `RechercheStore` geoeffnet.
//! Mandant und Person kommen ausschliesslich aus dem `Principal` (hier: die lokale Einzelplatz-
//! Sitzung), nie aus Eingaben der Oberflaeche.
//!
//! Sicherheitsentscheidungen (wie Corporate LLM, PR-6):
//! - Web ist gesperrt, solange kein Suite-Projekt mit Web-Freigabe verknuepft ist (Default-Deny).
//! - Ab der Klasse `PERSONAL` haelt der Lauf an und braucht ein `FreigabeToken`; die Person darf eine
//!   Teilauswahl der Anfragen senden. Vor dem Senden werden die Regeln erneut geprueft.
//! - Die Adresse des Suchdienstes (SearXNG) darf nur ein Admin setzen und wird mit `pruefe_adresse`
//!   geprueft.
//! - Das Audit enthaelt Kennungen und Zaehler, nie Frage, Anfragen, Antworten, Titel oder Adressen.
//! - Loeschkaskade und Aufbewahrung: verschwindet die Herkunft, verschwindet der Eintrag; das
//!   Suite-Projekt loeschen entfernt auch seine Recherche-Dokumente; `retention_days` des Projekts
//!   gilt fuer seine Recherche-Dokumente.

pub mod audit;
pub mod befehle;
pub mod dateien;
pub mod dienst;
pub mod herkunft;
pub mod kontext;
pub mod quellen;
pub mod recherche;
pub mod texte;
pub mod zustand;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_p4;

use std::fmt;
use wai_security::{
    resolve_principal, ActorId, ClientClaims, DataClass, Principal, Role, ServerSession, TenantId,
};

/// Fehler mit stabilem Code (Tests, Oberflaeche) und verstaendlichem Text. Der Text enthaelt nie
/// Frage, Anfragen oder Quellen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fehler {
    pub code: &'static str,
    pub message: String,
}

impl Fehler {
    pub fn neu(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn ungueltig(message: &str) -> Self {
        Self::neu("projekte.invalid", message)
    }

    pub fn speicher(e: impl fmt::Display) -> Self {
        Self::neu("projekte.storage", e.to_string())
    }
}

impl fmt::Display for Fehler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Fehler {}

/// An die Oberflaeche geht `<code>: <text>`; sie uebersetzt den Code und zeigt sonst den Text.
impl From<Fehler> for String {
    fn from(f: Fehler) -> Self {
        format!("{}: {}", f.code, f.message)
    }
}

/// Installiert den TLS-Anbieter (ring) fuer `wai-recherche` einmal je Prozess. `reqwest` 0.13 laeuft hier
/// ohne eingebauten Anbieter (kein aws-lc: dessen Lizenz steht nicht auf der Erlaubnisliste); ohne diesen
/// Aufruf bricht der Aufbau des HTTP-Clients ab. Ist schon ein Anbieter gesetzt, bleibt er.
pub fn tls_anbieter() {
    static EINMAL: std::sync::Once = std::sync::Once::new();
    EINMAL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub type Res<T> = Result<T, Fehler>;

/// Lokale Einzelplatz-Sitzung: ein Mandant, eine Person, Rolle Admin (die Person ist die einzige
/// Betreiberin ihrer App). Das ist die Solo-Grenze: Es gibt keine Rollentrennung zwischen Nutzerin und
/// Administration (siehe docs/KNOWN-LIMITATIONS.md, "Suite-Projekte und Recherche"); mit mehreren
/// Personen an einer App-Instanz muesste die Sitzung vom Server kommen. Dieselbe Sitzung wie `local_principal` in Corporate LLM (Profil Solo);
/// die Rollenpruefung der Suchadresse bleibt bestehen und ist mit anderen Rollen getestet.
pub fn lokaler_principal() -> Principal {
    let session = ServerSession {
        tenant: TenantId("solo".into()),
        actor: ActorId("local-user".into()),
        role: Role::Admin,
        max_class: DataClass::Restricted,
        tool_rights: Default::default(),
    };
    resolve_principal(&session, &ClientClaims::default()).expect("feste Sitzung ist gueltig")
}
