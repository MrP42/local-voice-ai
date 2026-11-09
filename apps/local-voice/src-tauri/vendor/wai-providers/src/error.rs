//! Providerfehler. Meldungen enthalten nie Schluessel, Header oder Prompt-Inhalte;
//! Upstream-Details laufen durch `wai_security::redact` und werden gekuerzt.

use crate::contract::{ContractError, Diagnostic, RedactedTrue};
use crate::provider::Capability;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("provider is not configured: {0}")]
    NotConfigured(String),
    #[error("provider does not support capability {0:?}")]
    UnsupportedCapability(Capability),
    #[error("unsupported request feature: {0}")]
    UnsupportedFeature(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("model '{model}' is not available at provider '{provider}'")]
    ModelNotFound { provider: String, model: String },
    #[error(
        "model '{model}' is too large for this host ({size_bytes} bytes > limit {limit_bytes})"
    )]
    ModelTooLarge {
        model: String,
        size_bytes: u64,
        limit_bytes: u64,
    },
    #[error("request exceeds the context window of the model")]
    ContextExceeded,
    #[error("provider rejected the credentials")]
    AuthRejected,
    #[error("API key is not available in the secret store")]
    SecretUnavailable,
    #[error("provider unavailable: {detail}")]
    Unavailable {
        detail: String,
        retryable: bool,
        retry_after_ms: Option<i64>,
    },
    #[error("provider returned a malformed response: {0}")]
    Protocol(String),
    #[error("cloud egress denied by policy: {0}")]
    PolicyDenied(String),
    #[error("cloud egress needs a verified approval")]
    ApprovalRequired,
    #[error("cloud egress for provider '{0}' was not opted in")]
    CloudNotOptedIn(String),
}

impl ProviderError {
    /// Code aus `x-wai-error-codes` in `contracts/core/error.schema.json`.
    pub fn code(&self) -> &'static str {
        match self {
            ProviderError::NotConfigured(_) | ProviderError::SecretUnavailable => {
                "provider.not_configured"
            }
            ProviderError::UnsupportedCapability(_)
            | ProviderError::UnsupportedFeature(_)
            | ProviderError::ModelNotFound { .. } => "provider.unsupported_capability",
            ProviderError::InvalidRequest(_) => "contract.invalid_value",
            ProviderError::ModelTooLarge { .. } | ProviderError::ContextExceeded => {
                "resource.admission_denied"
            }
            ProviderError::AuthRejected
            | ProviderError::Unavailable { .. }
            | ProviderError::Protocol(_) => "provider.unavailable",
            ProviderError::PolicyDenied(_) | ProviderError::CloudNotOptedIn(_) => "policy.denied",
            ProviderError::ApprovalRequired => "policy.approval_required",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::Unavailable {
                retryable: true,
                ..
            }
        )
    }

    pub fn to_contract(&self, trace_id: Option<&str>) -> ContractError {
        let retry_after_ms = match self {
            ProviderError::Unavailable { retry_after_ms, .. } => *retry_after_ms,
            _ => None,
        };
        ContractError {
            code: self.code().to_string(),
            message: self.to_string().chars().take(1000).collect(),
            retryable: self.retryable(),
            retry_after_ms,
            diagnostic: Diagnostic {
                redacted: RedactedTrue,
                trace_id: trace_id.map(str::to_string),
                component: Some("wai-providers".to_string()),
                detail: None,
            },
        }
    }
}

/// Upstream-Text fuer Fehlermeldungen: redigiert und auf 200 Zeichen gekuerzt.
pub(crate) fn safe_detail(raw: &str) -> String {
    let red = wai_security::redact(raw);
    let one_line: String = red.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(200).collect()
}

pub type Result<T> = std::result::Result<T, ProviderError>;
