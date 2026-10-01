//! Zustand eines Microsoft-365-Kontos fuer die Oberflaeche (A5).
//!
//! Der Zustand ergibt sich aus Konfiguration und Geheimnis, nicht aus einem
//! gespeicherten Merker:
//!
//! | Zustand           | Wann                                                         |
//! |-------------------|--------------------------------------------------------------|
//! | `not_configured`  | keine Client-ID                                              |
//! | `no_capabilities` | keine Faehigkeit eingeschaltet                               |
//! | `needs_sign_in`   | kein Token (nie angemeldet, abgemeldet, widerrufen, kaputt)  |
//! | `needs_consent`   | Token da, aber eine eingeschaltete Faehigkeit braucht Scopes, |
//! |                   | denen noch nicht zugestimmt wurde                            |
//! | `ready`           | alles da                                                     |
//!
//! „Neu anmelden“ bei ungueltigem Erneuerungs-Token entsteht dadurch, dass der
//! Dienst das tote Token loescht (`M365Service::access_token`): danach zeigt dieser
//! Zustand `needs_sign_in`. Adresse und Name des Kontos stehen nur im verschluesselten
//! Geheimnis und erscheinen hier nur fuer die Anzeige, nie im Register oder Audit.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::account::Vault;
use super::config::{FilesMode, M365Config};
use super::error::M365Error;
use crate::managers::integrations::model::{Capability, Integration};
use crate::managers::integrations::secrets;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    NotConfigured,
    NoCapabilities,
    NeedsSignIn,
    NeedsConsent,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct M365Status {
    pub state: AccountState,
    pub client_id: String,
    pub tenant: String,
    pub account: Option<String>,
    pub display_name: Option<String>,
    pub enabled_capabilities: Vec<Capability>,
    /// Faehigkeiten, die sich an diesem Konto einschalten lassen.
    pub available_capabilities: Vec<Capability>,
    pub files_mode: FilesMode,
    pub files_folder: String,
    pub required_scopes: Vec<String>,
    pub granted_scopes: Vec<String>,
    pub missing_scopes: Vec<String>,
    /// Zustand des Geheimnisses: `present`, `missing` oder `broken`.
    pub secret: String,
    pub signing_in: bool,
}

/// Berechnet den Zustand (siehe Moduldoku). `signing_in`: laeuft gerade eine Anmeldung?
pub fn status_of(
    i: &Integration,
    vault: &Vault,
    signing_in: bool,
) -> Result<M365Status, M365Error> {
    let cfg = M365Config::from_json(&i.config_json)?;
    let required = cfg.required_scopes();
    let secret = secrets::status_label(&vault.status(i)).to_string();
    let account = vault.load(i).ok().flatten();
    let granted = account.as_ref().map(|a| a.granted()).unwrap_or_default();
    let missing = match &account {
        Some(a) => a.missing(&required),
        None => Vec::new(),
    };
    let state = if cfg.client_id.is_empty() {
        AccountState::NotConfigured
    } else if cfg.capabilities.is_empty() {
        AccountState::NoCapabilities
    } else {
        match &account {
            None => AccountState::NeedsSignIn,
            Some(a) if a.client_id != cfg.client_id || a.tenant != cfg.tenant => {
                AccountState::NeedsSignIn
            }
            Some(_) if !missing.is_empty() => AccountState::NeedsConsent,
            Some(_) => AccountState::Ready,
        }
    };
    Ok(M365Status {
        state,
        client_id: cfg.client_id.clone(),
        tenant: cfg.tenant.clone(),
        account: account.as_ref().and_then(|a| a.address.clone()),
        display_name: account.as_ref().and_then(|a| a.name.clone()),
        enabled_capabilities: cfg.capabilities.clone(),
        available_capabilities: super::config::ENABLEABLE.to_vec(),
        files_mode: cfg.files_mode,
        files_folder: cfg.files_folder.clone(),
        required_scopes: required,
        granted_scopes: granted,
        missing_scopes: missing,
        secret,
        signing_in,
    })
}
