//! Konfiguration des Microsoft-365-Kontos und die Scopes je Faehigkeit (A5).
//!
//! Die Konfiguration liegt in `integrations.config_json` und enthaelt kein
//! Geheimnis: die Client-ID einer oeffentlichen App-Registrierung ist keins.
//!
//! ```json
//! { "client_id": "<GUID>", "tenant": "common",
//!   "enabled_capabilities": ["mail.send", "files.write", "calendar.write"],
//!   "files_mode": "full", "files_folder": "Local Voice AI" }
//! ```
//!
//! **Scopes nur fuer eingeschaltete Faehigkeiten.** `required_scopes` liefert
//! `offline_access` und `User.Read` (Anmeldung und Name des Kontos) plus je
//! eingeschalteter Faehigkeit genau einen Scope. Wer nichts einschaltet, fragt
//! nichts an; wer eine Faehigkeit ausschaltet, fragt sie beim naechsten Erneuern
//! nicht mehr an. `files_mode = app_folder` ist nur fuer private Microsoft-Konten
//! (Files.ReadWrite.AppFolder gibt es bei Arbeits- und Schulkonten nicht).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::error::M365Error;
use crate::managers::calendar::graph::{validate_client_id, validate_tenant, DEFAULT_TENANT};
use crate::managers::integrations::model::Capability;

/// Gehoert zu jeder Anmeldung: Erneuerungs-Token und eigenes Profil (Name, Adresse).
pub const BASE_SCOPES: [&str; 2] = ["offline_access", "User.Read"];

/// Faehigkeiten, die dieses Paket umsetzt und die man am Konto einschalten kann.
/// (`calendar.read` und `files.read` bietet die Art an; lesende Termine liefert die
/// Kalenderquelle „Microsoft 365“, Lesen aus OneDrive folgt mit dem ersten Nutzer.)
pub const ENABLEABLE: [Capability; 3] = [
    Capability::MailSend,
    Capability::FilesWrite,
    Capability::CalendarWrite,
];

/// Unterordner in OneDrive, wenn der Nutzer keinen anderen waehlt.
pub const DEFAULT_FILES_FOLDER: &str = "Local Voice AI";

/// Wo in OneDrive geschrieben wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FilesMode {
    /// Das ganze OneDrive (`Files.ReadWrite`): Arbeits-/Schulkonten und private Konten.
    Full,
    /// Nur der App-Ordner (`Files.ReadWrite.AppFolder`): nur private Konten.
    AppFolder,
}

impl FilesMode {
    pub fn as_str(self) -> &'static str {
        match self {
            FilesMode::Full => "full",
            FilesMode::AppFolder => "app_folder",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full" => Some(FilesMode::Full),
            "app_folder" => Some(FilesMode::AppFolder),
            _ => None,
        }
    }
}

/// Der Scope einer Faehigkeit; `None` fuer Faehigkeiten, die das Konto (noch)
/// nicht umsetzt.
pub fn scope_of(cap: Capability, files: FilesMode) -> Option<&'static str> {
    match cap {
        Capability::MailSend => Some("Mail.Send"),
        Capability::FilesWrite => Some(match files {
            FilesMode::Full => "Files.ReadWrite",
            FilesMode::AppFolder => "Files.ReadWrite.AppFolder",
        }),
        Capability::CalendarWrite => Some("Calendars.ReadWrite"),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M365Config {
    /// Leer: noch keine Client-ID (Zustand „nicht eingerichtet“).
    pub client_id: String,
    pub tenant: String,
    /// Eingeschaltete Faehigkeiten in der Reihenfolge von `ENABLEABLE`, ohne Dubletten.
    pub capabilities: Vec<Capability>,
    pub files_mode: FilesMode,
    /// Unterordner in OneDrive (relativ, ohne fuehrenden Schraegstrich); leer = Wurzel.
    pub files_folder: String,
}

impl M365Config {
    /// Prueft die Eingaben der Oberflaeche und baut die Konfiguration. Eine leere
    /// Client-ID ist erlaubt (Zustand „nicht eingerichtet“); eine gesetzte muss eine
    /// GUID sein.
    pub fn new(
        client_id: &str,
        tenant: &str,
        capabilities: &[Capability],
        files_mode: FilesMode,
        files_folder: &str,
    ) -> Result<Self, M365Error> {
        let client_id = match client_id.trim() {
            "" => String::new(),
            id => validate_client_id(id).map_err(M365Error::Invalid)?,
        };
        let tenant = validate_tenant(tenant).map_err(M365Error::Invalid)?;
        let wanted: BTreeSet<&str> = capabilities.iter().map(|c| c.as_str()).collect();
        if let Some(bad) = capabilities.iter().find(|c| !ENABLEABLE.contains(c)) {
            return Err(M365Error::Invalid(format!(
                "Die Fähigkeit „{}“ lässt sich an diesem Konto nicht einschalten.",
                bad.as_str()
            )));
        }
        let capabilities = ENABLEABLE
            .into_iter()
            .filter(|c| wanted.contains(c.as_str()))
            .collect();
        let files_folder = super::drive::clean_folder(files_folder)?;
        Ok(Self {
            client_id,
            tenant,
            capabilities,
            files_mode,
            files_folder,
        })
    }

    /// Liest `config_json`. Fehlende Felder bekommen die sichere Vorgabe (keine
    /// Faehigkeit, `common`); eine unlesbare Konfiguration ist ein Fehler.
    pub fn from_json(config_json: &str) -> Result<Self, M365Error> {
        let v: Value = serde_json::from_str(config_json)
            .map_err(|_| M365Error::Config("Die Konfiguration des Kontos ist unlesbar.".into()))?;
        let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let caps: Vec<Capability> = v
            .get("enabled_capabilities")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter_map(Capability::parse)
                    .collect()
            })
            .unwrap_or_default();
        let files_mode = v
            .get("files_mode")
            .and_then(Value::as_str)
            .and_then(FilesMode::parse)
            .unwrap_or(FilesMode::Full);
        let folder = match v.get("files_folder") {
            Some(Value::String(f)) => f.clone(),
            _ => DEFAULT_FILES_FOLDER.to_string(),
        };
        let tenant = text("tenant");
        Self::new(
            &text("client_id"),
            if tenant.is_empty() {
                DEFAULT_TENANT
            } else {
                &tenant
            },
            &caps,
            files_mode,
            &folder,
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "client_id": self.client_id,
            "tenant": self.tenant,
            "enabled_capabilities": self.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "files_mode": self.files_mode.as_str(),
            "files_folder": self.files_folder,
        })
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Alle Scopes, die die eingeschalteten Faehigkeiten brauchen (siehe Moduldoku).
    pub fn required_scopes(&self) -> Vec<String> {
        let mut scopes: Vec<String> = BASE_SCOPES.iter().map(|s| s.to_string()).collect();
        for cap in &self.capabilities {
            if let Some(scope) = scope_of(*cap, self.files_mode) {
                scopes.push(scope.to_string());
            }
        }
        scopes
    }

    /// Dieselben, durch Leerzeichen getrennt (Wert des Parameters `scope`).
    pub fn scope_string(&self) -> String {
        self.required_scopes().join(" ")
    }
}
