use serde::{Deserialize, Serialize};

/// Inhalt von `profile.json` (Schema `contracts/core/profil.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfilDatei {
    pub version: u32,
    pub generation: u64,
    /// Steigt bei Profilwechsel und (ab Paket 2) bei Anmelden, Erneuern und Abmelden;
    /// ein Ergebnis, das mit älterer Revision begann, darf keine Sitzung wiederherstellen.
    pub sitzungsrevision: u64,
    pub aktiv: String,
    pub profile: Vec<Profil>,
}

/// Ein Profil: die Person, die an diesem Gerät arbeitet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profil {
    pub id: String,
    pub anzeigename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vorname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nachname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telefon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobil: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abteilung: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organisation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standort: Option<String>,
    /// Index der Initialenfarbe, 0 bis 15.
    pub farbe: u8,
    /// Profilbild liegt unter `bilder/<id>.png`.
    pub hat_bild: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anmeldung: Option<Anmeldung>,
    /// PIN-Sperre aktiv (Paket 4, hier nur Feld).
    #[serde(default)]
    pub gesperrt: bool,
    pub erstellt_ms: u64,
    pub geaendert_ms: u64,
}

/// Verbindung eines Profils mit einem Identitätsanbieter (ab Paket 2 befüllt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Anmeldung {
    pub issuer: String,
    pub subject: String,
    pub seit_ms: u64,
    /// Schreibgeschützte Feldnamen (camelCase, etwa `email`).
    pub idp_felder: Vec<String>,
}

/// Eingabe für ein neues Profil; nur der Anzeigename ist Pflicht.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfilEingabe {
    pub anzeigename: String,
    pub vorname: Option<String>,
    pub nachname: Option<String>,
    pub email: Option<String>,
    pub telefon: Option<String>,
    pub mobil: Option<String>,
    pub position: Option<String>,
    pub abteilung: Option<String>,
    pub organisation: Option<String>,
    pub standort: Option<String>,
    /// Ohne Angabe wird die Farbe reihum vergeben.
    pub farbe: Option<u8>,
}

/// Teiländerung eines Profils: `None` lässt das Feld unverändert, `Some("")` leert
/// ein optionales Feld (der Anzeigename darf nicht leer werden).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfilAenderung {
    pub anzeigename: Option<String>,
    pub vorname: Option<String>,
    pub nachname: Option<String>,
    pub email: Option<String>,
    pub telefon: Option<String>,
    pub mobil: Option<String>,
    pub position: Option<String>,
    pub abteilung: Option<String>,
    pub organisation: Option<String>,
    pub standort: Option<String>,
    pub farbe: Option<u8>,
}
