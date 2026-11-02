//! Durchsetzung des Policy-Gates (`wai_security::decide_egress`) vor jedem Netzaufruf
//! zu einem Cloud-Ziel. Fail-closed: ohne Opt-in, ohne Freigabe der Policy oder ohne
//! Nachweise wird kein Byte gesendet; es gibt keinen stillen Cloud-Fallback.

use crate::contract::{BillingMode, DataClass, InferenceRequest};
use crate::error::{ProviderError, Result};
use crate::provider::{CallOptions, payload_bytes};
use crate::util::new_id;
use std::sync::Arc;
use wai_security::{ApprovalLedger, Decision, EgressConfig, EgressRequest, Route};

/// Wohin ein Provider sendet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Loopback (127.0.0.0/8, ::1, localhost): kein Egress.
    Loopback,
    /// Alles andere: Cloud-Route im Sinne der Policy.
    Remote { endpoint: String, region: String },
}

/// Tatsaechliches Ziel einer Cloud-Route: kanonischer Endpunkt `scheme://host:port`
/// (Host lowercase, Port explizit) plus Region. Zugleich der Freigabe-Empfaenger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressTarget {
    pub endpoint: String,
    pub region: String,
}

/// Kanonischer Freigabe-Empfaenger `<scheme://host:port>|region=<region>|policy=<version>`.
/// Einzige Bildungsstelle: Ausstellung der Freigabe (Empfaenger) und Gate-Verbrauch nutzen
/// ausschliesslich diesen String, damit das Ledger Region und Policy-Version serverseitig
/// vergleicht.
pub fn approval_scope(target: &EgressTarget, policy_version: u32) -> String {
    format!(
        "{}|region={}|policy={}",
        target.endpoint, target.region, policy_version
    )
}

type Clock = fn() -> u64;

#[derive(Clone, Default)]
pub struct EgressGate {
    cfg: EgressConfig,
    approvals: Option<(Arc<ApprovalLedger>, Clock)>,
}

impl std::fmt::Debug for EgressGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EgressGate")
            .field("cfg", &self.cfg)
            .field("approvals", &self.approvals.is_some())
            .finish()
    }
}

impl EgressGate {
    pub fn new(cfg: EgressConfig) -> Self {
        Self {
            cfg,
            approvals: None,
        }
    }
    /// Ledger, in dem das Gate Freigaben beim Senden atomar verbraucht; `now` liefert
    /// die Zeit in der Einheit von `expires_at`. Ohne Ledger gibt es keine Freigaben.
    pub fn with_approvals(mut self, ledger: Arc<ApprovalLedger>, now: fn() -> u64) -> Self {
        self.approvals = Some((ledger, now));
        self
    }
    /// Keine Cloud-Ziele freigegeben.
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Prueft eine Anfrage und liefert die ID der Entscheidung (fuer `policy_decision_id`).
    pub(crate) fn check(
        &self,
        provider_id: &str,
        target: &Target,
        opted_in: bool,
        req: &InferenceRequest,
        opts: &CallOptions,
    ) -> Result<String> {
        let Target::Remote { endpoint, region } = target else {
            return Ok(new_id("pol"));
        };
        if !opted_in {
            return Err(ProviderError::CloudNotOptedIn(provider_id.to_string()));
        }
        let egress_target = EgressTarget {
            endpoint: endpoint.clone(),
            region: region.clone(),
        };
        let payload = payload_bytes(req);
        // Nachweis nur gueltig, wenn an Tenant, Payload, Ziel und Policy-Version gebunden.
        let evidence = opts.evidence().filter(|e| {
            e.binds(
                opts.tenant(),
                &payload,
                &egress_target,
                self.cfg.policy_version,
            )
        });
        let egress = EgressRequest {
            class: Some(req.data_class.into()),
            route: Route::Cloud {
                provider: provider_id.to_string(),
                region: region.clone(),
            },
            purpose: req.purpose.clone(),
            // Kein vertrauenswuerdiger Redaktionspruefer: nie verifiziert.
            redacted_verified: false,
        };
        match wai_security::decide_egress(&self.cfg, &egress) {
            Decision::Allow => Ok(new_id("pol")),
            Decision::NeedsApproval => {
                let (Some(e), Some((ledger, now)), Some(tenant)) =
                    (evidence, self.approvals.as_ref(), opts.tenant())
                else {
                    return Err(ProviderError::ApprovalRequired);
                };
                // Atomarer Einmalverbrauch beim Senden.
                ledger
                    .verify_and_consume(
                        e.record(),
                        tenant,
                        &payload,
                        &approval_scope(&egress_target, self.cfg.policy_version),
                        now(),
                    )
                    .map_err(|_| ProviderError::ApprovalRequired)?;
                Ok(new_id("pol"))
            }
            Decision::Deny(reason) => Err(ProviderError::PolicyDenied(reason.to_string())),
        }
    }

    /// Gate fuer Netzoperationen ohne Nutzdaten (Discovery/Modellabfrage): wie eine
    /// Public-Anfrage; ohne Opt-in oder Freigabe des Ziels wird nichts gesendet.
    pub(crate) fn check_discovery(
        &self,
        provider_id: &str,
        target: &Target,
        opted_in: bool,
    ) -> Result<()> {
        let Target::Remote { region, .. } = target else {
            return Ok(());
        };
        if !opted_in {
            return Err(ProviderError::CloudNotOptedIn(provider_id.to_string()));
        }
        let egress = EgressRequest {
            class: Some(DataClass::Public.into()),
            route: Route::Cloud {
                provider: provider_id.to_string(),
                region: region.clone(),
            },
            purpose: "discovery".to_string(),
            redacted_verified: false,
        };
        match wai_security::decide_egress(&self.cfg, &egress) {
            Decision::Allow => Ok(()),
            Decision::NeedsApproval => Err(ProviderError::ApprovalRequired),
            Decision::Deny(reason) => Err(ProviderError::PolicyDenied(reason.to_string())),
        }
    }
}

/// Billing-Modus der Anfrage muss exakt dem des Providers entsprechen.
pub(crate) fn check_billing(provider: BillingMode, requested: BillingMode) -> Result<()> {
    if provider == requested {
        Ok(())
    } else {
        Err(ProviderError::PolicyDenied("billing-mode-mismatch".into()))
    }
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    h.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::is_loopback_host;

    #[test]
    fn loopback_detection_is_strict() {
        for h in [
            "localhost",
            "LocalHost",
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "[::1]",
        ] {
            assert!(is_loopback_host(h), "{h}");
        }
        for h in [
            "127.attacker.example",
            "127.0.0.1.nip.io",
            "localhost.attacker.example",
            "example.com",
            "::2",
            "128.0.0.1",
        ] {
            assert!(!is_loopback_host(h), "{h}");
        }
    }
}
