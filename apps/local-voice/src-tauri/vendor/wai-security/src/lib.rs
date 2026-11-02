//! `wai-security` (SDK-07): Principal/Tenant, Policy, Egress-Gate (fail-closed),
//! Freigaben, SecretStore (keyring + Envelope) und Log-Redaktion.
//! Grundsatz: Principal und Tenant ermittelt ausschliesslich der Server.

pub mod approval;
pub mod model;
pub mod policy;
pub mod redact;
pub mod secret;

pub use approval::{ApprovalError, ApprovalLedger, ApprovalRecord, ApprovalToken, payload_hash};
pub use model::{
    ActorId, AuthError, ClientClaims, Principal, Role, ServerSession, TenantId, resolve_principal,
};
pub use policy::{
    AccessRequest, Action, DataClass, Decision, DecisionDenied, EgressConfig, EgressRequest, Route,
    authorize, authorize_decision, decide_egress, strictest, strictest_art9,
};
pub use redact::{Redacted, redact, redact_debug};
pub use secret::{
    BlobStore, EnvelopeStore, FileBlobs, KeyBackend, KeyringBackend, MemoryBackend, MemoryBlobs,
    SecretError, SecretHandle, SecretStore, SecretValue,
};
