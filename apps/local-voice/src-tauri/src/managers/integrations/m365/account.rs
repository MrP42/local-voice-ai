//! Das Geheimnis des Kontos: Erneuerungs-Token samt Client-ID, Verzeichnis und den
//! Scopes, die bei der Anmeldung zugestimmt wurden (A5).
//!
//! Es liegt DPAPI-verschluesselt im Geheimnis-Namensraum der Integrationen
//! (`int-<id>-token.bin`, siehe `secrets.rs`), nie in `config_json`, nie im Audit,
//! nie im Dump (dort nur der Zustand), nie in einer Fehlermeldung. Das
//! Zugriffstoken lebt nur im Arbeitsspeicher (`TokenCache` im Dienst).
//!
//! Schreiben ist atomar (Temp-Datei, dann umbenennen, siehe `calendar::secret`):
//! ein Abbruch oder eine volle Platte laesst das alte Token stehen.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::managers::calendar::secret::{self, SecretStatus};
use crate::managers::integrations::model::Integration;
use crate::managers::integrations::secrets;

/// Fach des Erneuerungs-Tokens (`secrets::required_slots(Kind::M365)`).
pub const SLOT: &str = "token";

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    v: u32,
    pub client_id: String,
    pub tenant: String,
    /// Scopes der Anmeldung, durch Leerzeichen getrennt.
    pub scope: String,
    refresh_token: String,
    /// Adresse und Name des Kontos, nur zur Anzeige in der Oberflaeche.
    pub address: Option<String>,
    pub name: Option<String>,
}

impl StoredAccount {
    pub fn new(
        client_id: String,
        tenant: String,
        scope: String,
        refresh_token: String,
        address: Option<String>,
        name: Option<String>,
    ) -> Self {
        Self {
            v: 1,
            client_id,
            tenant,
            scope,
            refresh_token,
            address,
            name,
        }
    }

    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    /// Dasselbe Konto mit einem neuen Erneuerungs-Token (Microsoft rotiert es).
    pub fn with_refresh_token(&self, token: String) -> Self {
        let mut next = self.clone();
        next.refresh_token = token;
        next
    }

    /// Die zugestimmten Scopes als Liste.
    pub fn granted(&self) -> Vec<String> {
        self.scope.split_whitespace().map(str::to_string).collect()
    }

    /// Welche der verlangten Scopes fehlen (Gross-/Kleinschreibung egal)?
    pub fn missing(&self, required: &[String]) -> Vec<String> {
        let granted: Vec<String> = self.granted().iter().map(|s| s.to_lowercase()).collect();
        required
            .iter()
            .filter(|r| !granted.contains(&r.to_lowercase()))
            .cloned()
            .collect()
    }
}

impl Drop for StoredAccount {
    fn drop(&mut self) {
        self.refresh_token.zeroize();
    }
}

impl std::fmt::Debug for StoredAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StoredAccount({}, {})", self.client_id, self.tenant)
    }
}

/// Zugriff auf den Geheimnisspeicher: im Betrieb der globale Ordner dieser
/// App-Instanz, in Tests ein eigener.
#[derive(Clone, Debug, Default)]
pub struct Vault {
    dir: Option<PathBuf>,
    /// Tests: Schreiben schlaegt fehl (volle Platte).
    #[cfg(test)]
    pub fail_writes: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Vault {
    /// Der Speicher der laufenden App (Ordner wird beim Start festgelegt).
    pub fn system() -> Self {
        Self {
            dir: None,
            #[cfg(test)]
            fail_writes: Default::default(),
        }
    }

    /// Ein bestimmter Ordner (Tests, Dump).
    pub fn in_dir(dir: PathBuf) -> Self {
        Self {
            dir: Some(dir),
            #[cfg(test)]
            fail_writes: Default::default(),
        }
    }

    /// Zustand des Faches, ohne den Inhalt zu lesen.
    pub fn status(&self, i: &Integration) -> SecretStatus {
        match &self.dir {
            Some(dir) => secrets::status_in(dir, i, SLOT),
            None => secrets::status(i, SLOT),
        }
    }

    /// `Ok(None)`: kein Token (nie angemeldet oder abgemeldet). `Err`: Datei da, aber
    /// nicht lesbar oder kein gueltiges Konto (anderer Benutzer, beschaedigt): auch
    /// dann muss der Nutzer sich neu anmelden.
    pub fn load(&self, i: &Integration) -> Result<Option<StoredAccount>, String> {
        let r = secrets::secret_ref(i, SLOT);
        let bytes: Option<Zeroizing<Vec<u8>>> = match &self.dir {
            Some(dir) => secret::get_ref_in(dir, &r)?,
            None => secret::secret_get_ref(&r)?,
        };
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let account: StoredAccount = serde_json::from_slice(&bytes)
            .map_err(|_| "Das gespeicherte Konto ist unlesbar.".to_string())?;
        if account.v != 1 || account.refresh_token.is_empty() {
            return Err("Das gespeicherte Konto ist ungültig.".to_string());
        }
        Ok(Some(account))
    }

    pub fn save(&self, i: &Integration, account: &StoredAccount) -> Result<(), String> {
        #[cfg(test)]
        if self.fail_writes.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Geheimnis nicht speicherbar: Der Datenträger ist voll.".to_string());
        }
        let bytes = Zeroizing::new(
            serde_json::to_vec(account)
                .map_err(|_| "Das Konto ist nicht speicherbar.".to_string())?,
        );
        match &self.dir {
            Some(dir) => secrets::put_in(dir, i, SLOT, &bytes),
            None => secrets::put(i, SLOT, &bytes),
        }
    }

    /// Loescht das Token (Abmelden, oder Microsoft hat es widerrufen).
    pub fn clear(&self, i: &Integration) {
        let r = secrets::secret_ref(i, SLOT);
        match &self.dir {
            Some(dir) => secret::delete_ref_in(dir, &r),
            None => secret::secret_delete_ref(&r),
        }
    }
}
