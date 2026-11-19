use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Aufbereitung {
    pub id: String,
    pub dokument: Dokument,
    pub basis: Basis,
    pub vorgaenger_aktiv: Option<String>,
    pub schritte: Vec<Schritt>,
    pub modell: Option<ModellArt>,
    pub status: Status,
    pub fortschritt: Fortschritt,
    pub aenderungen: Vec<Aenderung>,
    pub erstellt: u64,
    pub geaendert: u64,
}

impl std::fmt::Debug for Aufbereitung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aufbereitung")
            .field("status", &self.status)
            .field("fortschritt", &self.fortschritt)
            .field("aenderungen_anzahl", &self.aenderungen.len())
            .finish_non_exhaustive()
    }
}

impl Aufbereitung {
    pub fn neu(dokument: Dokument, basis_text: &str, jetzt_ms: u64) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            dokument,
            basis: Basis::aus_text(basis_text),
            vorgaenger_aktiv: None,
            schritte: Vec::new(),
            modell: None,
            status: Status::Entwurf,
            fortschritt: Fortschritt::default(),
            aenderungen: Vec::new(),
            erstellt: jetzt_ms,
            geaendert: jetzt_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Dokument {
    pub art: DokumentArt,
    pub id: String,
    pub fassung_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum DokumentArt {
    Buch,
    Seite,
    Gespraech,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Basis {
    pub sha256: String,
    pub laenge: u64,
}

impl Basis {
    pub fn aus_text(text: &str) -> Self {
        Self {
            sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
            laenge: text.len() as u64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Schritt {
    Struktur,
    Format,
    Sprache,
    Glaetten,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ModellArt {
    pub funktion: Schritt,
    pub ziel_art: ZielArt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ZielArt {
    Lokal,
    Cloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
#[specta(rename = "AufbereitungStatus")] // sonst Namenskollision in bindings.ts
pub enum Status {
    Laeuft,
    Entwurf,
    Aktiv,
    Abgeloest,
    Verworfen,
    Veraltet,
    Abgebrochen,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Fortschritt {
    pub abschnitte_fertig: u64,
    pub abschnitte_gesamt: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Aenderung {
    pub id: String,
    pub art: AenderungArt,
    /// Halboffener Byte-Bereich in der eingefrorenen Basis.
    pub bereich: [usize; 2],
    pub alt: String,
    pub neu: String,
    pub grund_code: String,
    pub zaehler: Option<u64>,
    pub gruppe: Option<String>,
    pub status: AenderungStatus,
}

impl std::fmt::Debug for Aenderung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aenderung")
            .field("bereich", &self.bereich)
            .field("alt_laenge", &self.alt.len())
            .field("neu_laenge", &self.neu.len())
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AenderungArt {
    Regel(String),
    Sprache,
    Glaetten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AenderungStatus {
    Offen,
    Angenommen,
    Abgelehnt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum ErgebnisFehler {
    Status,
    Bereich,
    Ueberlappung,
    Geschuetzt,
    Alt,
    Gruppe,
    Konflikt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum SpeicherFehler {
    Id,
    OriginalFest,
    Lesen,
    Schreiben,
    Schema,
    Basis,
    Aenderungen,
}

impl std::fmt::Display for ErgebnisFehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ungültige Aufbereitung: {self:?}")
    }
}

impl std::error::Error for ErgebnisFehler {}

impl std::fmt::Display for SpeicherFehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Aufbereitungsspeicher: {self:?}")
    }
}

impl std::error::Error for SpeicherFehler {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Struktur {
    pub ueberschriften: Vec<String>,
    pub seitenmarken: Vec<String>,
    pub absaetze: Vec<String>,
    pub tabellen: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StrukturVergleich {
    pub vorher: Struktur,
    pub nachher: Struktur,
}
