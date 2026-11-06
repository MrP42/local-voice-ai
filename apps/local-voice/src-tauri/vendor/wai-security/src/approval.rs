use crate::model::TenantId;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub fn payload_hash(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(payload).into()
}

/// Interner Ledger-Datensatz bzw. DB-Zeile (nie als Bearer an Aufrufer ausgeben, dafuer
/// gibt es [`ApprovalToken`]). Freigabe, gebunden an Payload-Hash, Empfaenger (inkl. Version) und Ablauf.
#[derive(PartialEq, Eq)]
pub struct ApprovalRecord {
    id: String,
    tenant: TenantId,
    payload_hash: [u8; 32],
    recipient: String,
    expires_at: u64,
}

impl std::fmt::Debug for ApprovalRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalRecord")
            .field("id", &"<redacted>")
            .field("tenant", &self.tenant)
            .field("recipient", &self.recipient)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl ApprovalRecord {
    /// Baut einen Datensatz aus persistierten Daten. Nur fuer vertrauenswuerdige Persistenz
    /// (eigene Datenbank des Aufrufers); Werte aus Anfragen duerfen so nie entstehen.
    pub fn from_persisted(
        id: String,
        tenant: TenantId,
        payload_hash: [u8; 32],
        recipient: String,
        expires_at: u64,
    ) -> Self {
        Self {
            id,
            tenant,
            payload_hash,
            recipient,
            expires_at,
        }
    }

    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    pub fn recipient(&self) -> &str {
        &self.recipient
    }

    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    /// Vergleicht die (geheime) ID, ohne sie herauszugeben.
    pub fn has_id(&self, id: &str) -> bool {
        self.id == id
    }

    /// Reine Pruefung aller Bindungen (Tenant, Payload-Hash, Empfaenger, Ablauf). Der
    /// Einmalverbrauch ist Sache des Speichers (Ledger oder persistente Tabelle).
    pub fn check(
        &self,
        tenant: &TenantId,
        payload: &[u8],
        recipient: &str,
        now: u64,
    ) -> Result<(), ApprovalError> {
        if self.tenant != *tenant {
            return Err(ApprovalError::TenantMismatch);
        }
        if self.payload_hash != payload_hash(payload) {
            return Err(ApprovalError::PayloadChanged);
        }
        if self.recipient != recipient {
            return Err(ApprovalError::RecipientMismatch);
        }
        if now >= self.expires_at {
            return Err(ApprovalError::Expired);
        }
        Ok(())
    }
}

/// Opake, nicht klonbare Bearer-Freigabe, die `ApprovalLedger::issue` ausgibt. Die ID ist
/// privat und taucht in `Debug` nie auf; Nicht-Geheimes liest man ueber Methoden.
pub struct ApprovalToken {
    id: String,
    tenant: TenantId,
    expires_at: u64,
}

impl ApprovalToken {
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

impl std::fmt::Debug for ApprovalToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApprovalToken(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    Unknown,
    TenantMismatch,
    PayloadChanged,
    RecipientMismatch,
    Expired,
    AlreadyConsumed,
    /// Entropiequelle ausgefallen: es wird keine Freigabe ausgestellt (fail-closed).
    Entropy,
}

#[derive(Default)]
struct Inner {
    issued: HashMap<String, ApprovalRecord>,
    consumed: HashSet<String>,
    counter: u64,
}

/// Serverseitiges Register: nur hier ausgestellte Freigaben sind gueltig, je einmal.
#[derive(Default)]
pub struct ApprovalLedger(Mutex<Inner>);

impl ApprovalLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(
        &self,
        tenant: TenantId,
        payload: &[u8],
        recipient: &str,
        expires_at: u64,
    ) -> Result<ApprovalToken, ApprovalError> {
        self.issue_with(tenant, payload, recipient, expires_at, |b| {
            getrandom::fill(b).map_err(|_| ())
        })
    }

    fn issue_with(
        &self,
        tenant: TenantId,
        payload: &[u8],
        recipient: &str,
        expires_at: u64,
        fill: impl FnOnce(&mut [u8]) -> Result<(), ()>,
    ) -> Result<ApprovalToken, ApprovalError> {
        let mut r = [0u8; 16];
        fill(&mut r).map_err(|()| ApprovalError::Entropy)?;
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.counter += 1;
        let id = format!(
            "apr-{}-{}",
            g.counter,
            r.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let rec = ApprovalRecord {
            id: id.clone(),
            tenant: tenant.clone(),
            payload_hash: payload_hash(payload),
            recipient: recipient.to_string(),
            expires_at,
        };
        g.issued.insert(id.clone(), rec);
        Ok(ApprovalToken {
            id,
            tenant,
            expires_at,
        })
    }

    /// Prueft alle Bindungen und verbraucht die Freigabe atomar (nur bei Erfolg).
    pub fn verify_and_consume(
        &self,
        presented: &ApprovalToken,
        tenant: &TenantId,
        payload: &[u8],
        recipient: &str,
        now: u64,
    ) -> Result<(), ApprovalError> {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Massgeblich ist der serverseitig gespeicherte Datensatz, nicht der vorgelegte.
        let stored = g.issued.get(&presented.id).ok_or(ApprovalError::Unknown)?;
        if g.consumed.contains(&presented.id) {
            return Err(ApprovalError::AlreadyConsumed);
        }
        stored.check(tenant, payload, recipient, now)?;
        g.consumed.insert(presented.id.clone());
        Ok(())
    }
}

/// Der Token ist nicht klonbar:
///
/// ```compile_fail
/// use wai_security::{ApprovalLedger, TenantId};
/// let l = ApprovalLedger::new();
/// let t = l.issue(TenantId("t".into()), b"p", "x@1", 10).unwrap();
/// let _c = t.clone();
/// ```
#[cfg(doctest)]
struct _TokenNotClone;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropiefehler_stellt_keine_freigabe_aus() {
        let l = ApprovalLedger::new();
        let r = l.issue_with(TenantId("t".into()), b"p", "x@1", 10, |_| Err(()));
        assert_eq!(r.unwrap_err(), ApprovalError::Entropy);
        assert!(l.0.lock().unwrap().issued.is_empty());
    }

    #[test]
    fn token_debug_enthaelt_id_nicht() {
        let l = ApprovalLedger::new();
        let t = l.issue(TenantId("t".into()), b"p", "x@1", 10).unwrap();
        let id = t.id.clone();
        let dbg = format!("{t:?}");
        assert!(!dbg.contains(&id) && !dbg.contains("apr-"));
        assert_eq!(dbg, "ApprovalToken(<redacted>)");
        assert_eq!(t.tenant(), &TenantId("t".into()));
        assert_eq!(t.expires_at(), 10);
    }

    #[test]
    fn record_debug_enthaelt_id_nicht() {
        let r = ApprovalRecord::from_persisted(
            "apr-geheim-123".into(),
            TenantId("t".into()),
            [0; 32],
            "x@1".into(),
            10,
        );
        let dbg = format!("{r:?}");
        assert!(!dbg.contains("apr-geheim-123") && dbg.contains("<redacted>"));
        assert_eq!(r.tenant(), &TenantId("t".into()));
    }

    #[test]
    fn normalfall_stellt_aus() {
        let l = ApprovalLedger::new();
        assert!(l.issue(TenantId("t".into()), b"p", "x@1", 10).is_ok());
    }
}
