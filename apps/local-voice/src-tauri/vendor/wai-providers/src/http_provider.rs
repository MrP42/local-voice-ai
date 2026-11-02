//! Konkrete Provider ueber OpenAI-kompatibles HTTP:
//! - `local_llama`: verwalteter llama-server auf 127.0.0.1 mit `--api-key` (ADR-0002 §5),
//! - `ollama_byo`: vorhandene Ollama-Instanz, nur verbinden (nie starten/stoppen/loeschen),
//! - `api`: optionaler BYOK-Anbieter, Schluessel nur als `SecretHandle`, Cloud nur mit Opt-in
//!   und Policy-Entscheidung (ADR-0002 §6).

use crate::contract::{
    Adapter, BillingMode, DataClass, Egress, Facts, InferenceRequest, InferenceResponse,
    InputModality, ModelCapability, OutputModality, ProviderCapability, SCHEMA_VERSION, TriState,
};
use crate::error::{ProviderError, Result};
use crate::gate::{EgressGate, EgressTarget, Target, check_billing, is_loopback_host};
use crate::openai::{Auth, CallCtx, OpenAiClient, RemoteModel, SharedStore, Timeouts};
use crate::provider::{CallOptions, Capability, InferenceProvider, InferenceStream, Provider};
use crate::util::{new_id, now_rfc3339};
use async_trait::async_trait;
use wai_security::SecretHandle;

pub struct LocalLlamaConfig {
    pub provider_id: String,
    /// Vom Supervisor vergebener OS-Port auf 127.0.0.1.
    pub port: u16,
    /// Zufaelliger `--api-key` je Start, nur als Handle.
    pub api_key: SecretHandle,
    /// Obergrenze der Modellgroesse (aus der Admission); groessere Modelle werden abgelehnt.
    pub max_model_bytes: Option<u64>,
    pub timeouts: Timeouts,
}

pub struct OllamaByoConfig {
    pub provider_id: String,
    /// z. B. `http://127.0.0.1:11434`; die Instanz gehoert dem Nutzer.
    pub base_url: String,
    pub api_key: Option<SecretHandle>,
    /// Nur relevant, wenn die Instanz nicht auf Loopback liegt.
    pub remote_opt_in: bool,
    pub max_model_bytes: Option<u64>,
    pub timeouts: Timeouts,
}

pub struct ApiProviderConfig {
    pub provider_id: String,
    /// Basis inkl. Versionspfad, z. B. `https://api.mistral.ai/v1`.
    pub base_url: String,
    pub region: String,
    pub api_key: SecretHandle,
    /// Bewusstes Opt-in fuer Cloud-Egress. Standard: `false`.
    pub cloud_opt_in: bool,
    /// Klartext-HTTP nur fuer Tests und Kunden-Gateways im eigenen Netz.
    pub allow_insecure_http: bool,
    pub facts: Facts,
    pub max_data_class: DataClass,
    pub timeouts: Timeouts,
}

impl ApiProviderConfig {
    pub fn new(provider_id: &str, base_url: &str, region: &str, api_key: SecretHandle) -> Self {
        Self {
            provider_id: provider_id.into(),
            base_url: base_url.into(),
            region: region.into(),
            api_key,
            cloud_opt_in: false,
            allow_insecure_http: false,
            facts: Facts {
                dpf: TriState::Unknown,
                no_training: TriState::Unknown,
                eu_region: TriState::Unknown,
            },
            max_data_class: DataClass::Internal,
            timeouts: Timeouts::default(),
        }
    }
}

pub struct HttpProvider {
    provider_id: String,
    adapter: Adapter,
    billing: BillingMode,
    client: OpenAiClient,
    target: Target,
    host: String,
    opted_in: bool,
    gate: EgressGate,
    max_class: DataClass,
    facts: Facts,
    max_model_bytes: Option<u64>,
    /// Modellliste vor der Inferenz pruefen (lokal/BYO); Cloud meldet fehlende Modelle selbst.
    check_models: bool,
}

impl std::fmt::Debug for HttpProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpProvider")
            .field("provider_id", &self.provider_id)
            .field("adapter", &self.adapter)
            .field("client", &self.client)
            .field("target", &self.target)
            .field("opted_in", &self.opted_in)
            .finish()
    }
}

struct Url {
    https: bool,
    host: String,
    port: u16,
}
impl Url {
    /// Kanonischer Endpunkt `scheme://host:port` (Host lowercase, Port explizit).
    fn endpoint(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        if self.host.contains(':') {
            format!("{scheme}://[{}]:{}", self.host, self.port)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }
}
fn parse_url(u: &str) -> Result<Url> {
    let (https, rest) = if let Some(r) = u.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = u.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(ProviderError::NotConfigured(
            "base_url must be http(s)".into(),
        ));
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') || authority.is_empty() {
        return Err(ProviderError::NotConfigured(
            "base_url must not contain credentials".into(),
        ));
    }
    let (host, port_str) = if let Some(stripped) = authority.strip_prefix('[') {
        let (h, after) = stripped.split_once(']').unwrap_or((stripped, ""));
        (h.to_string(), after.strip_prefix(':'))
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    let port = match port_str {
        None => {
            if https {
                443
            } else {
                80
            }
        }
        Some(p) => p
            .parse::<u16>()
            .map_err(|_| ProviderError::NotConfigured("base_url has an invalid port".into()))?,
    };
    Ok(Url {
        https,
        host: host.to_ascii_lowercase(),
        port,
    })
}

impl HttpProvider {
    /// Lokaler llama-server. Host ist fest 127.0.0.1, der Schluessel Pflicht.
    pub fn local_llama(cfg: LocalLlamaConfig, store: SharedStore) -> Result<Self> {
        let client = OpenAiClient::new(
            format!("http://127.0.0.1:{}/v1", cfg.port),
            Auth::Bearer {
                handle: cfg.api_key,
                store,
            },
            cfg.timeouts,
        )?;
        Ok(Self {
            provider_id: cfg.provider_id,
            adapter: Adapter::LocalLlamaServer,
            billing: BillingMode::Local,
            client,
            target: Target::Loopback,
            host: "127.0.0.1".into(),
            opted_in: true,
            gate: EgressGate::deny_all(),
            max_class: DataClass::Restricted,
            facts: Facts {
                dpf: TriState::Unknown,
                no_training: TriState::Yes,
                eu_region: TriState::Yes,
            },
            max_model_bytes: cfg.max_model_bytes,
            check_models: true,
        })
    }

    /// BYO-Ollama: nur Verbindung zu einer vorhandenen Instanz.
    pub fn ollama_byo(cfg: OllamaByoConfig, store: SharedStore, gate: EgressGate) -> Result<Self> {
        let url = parse_url(&cfg.base_url)?;
        let loopback = is_loopback_host(&url.host);
        let auth = match cfg.api_key {
            Some(handle) => Auth::Bearer { handle, store },
            None => Auth::None,
        };
        let client = OpenAiClient::new(
            format!("{}/v1", cfg.base_url.trim_end_matches('/')),
            auth,
            cfg.timeouts,
        )?;
        Ok(Self {
            provider_id: cfg.provider_id,
            adapter: Adapter::OllamaByo,
            billing: BillingMode::Local,
            client,
            target: if loopback {
                Target::Loopback
            } else {
                Target::Remote {
                    endpoint: url.endpoint(),
                    region: "unknown".into(),
                }
            },
            host: url.host,
            opted_in: loopback || cfg.remote_opt_in,
            gate,
            max_class: if loopback {
                DataClass::Restricted
            } else {
                DataClass::Internal
            },
            facts: Facts {
                dpf: TriState::Unknown,
                no_training: TriState::Unknown,
                eu_region: TriState::Unknown,
            },
            max_model_bytes: cfg.max_model_bytes,
            check_models: true,
        })
    }

    /// Optionaler BYOK-Cloud-Provider (OpenAI-kompatibel).
    pub fn api(cfg: ApiProviderConfig, store: SharedStore, gate: EgressGate) -> Result<Self> {
        let url = parse_url(&cfg.base_url)?;
        if !url.https && !cfg.allow_insecure_http {
            return Err(ProviderError::NotConfigured(
                "cloud providers require https".into(),
            ));
        }
        let client = OpenAiClient::new(
            cfg.base_url,
            Auth::Bearer {
                handle: cfg.api_key,
                store,
            },
            cfg.timeouts,
        )?;
        Ok(Self {
            provider_id: cfg.provider_id,
            adapter: Adapter::OpenaiCompatible,
            billing: BillingMode::Api,
            client,
            // BYOK ist immer eine Cloud-Route, auch wenn ein Test auf Loopback zeigt.
            target: Target::Remote {
                endpoint: url.endpoint(),
                region: cfg.region,
            },
            host: url.host,
            opted_in: cfg.cloud_opt_in,
            gate,
            max_class: cfg.max_data_class,
            facts: cfg.facts,
            max_model_bytes: None,
            check_models: false,
        })
    }

    /// Tatsaechliches Cloud-Ziel (kanonischer Endpunkt + Region) fuer
    /// `EgressEvidence::from_approval`; `None` bei Loopback (kein Egress).
    pub fn egress_target(&self) -> Option<EgressTarget> {
        match &self.target {
            Target::Loopback => None,
            Target::Remote { endpoint, region } => Some(EgressTarget {
                endpoint: endpoint.clone(),
                region: region.clone(),
            }),
        }
    }

    /// Vor jedem Netzaufruf: Vertrag, Billing, Datenklasse, Opt-in, Policy.
    fn prepare(&self, req: &InferenceRequest, opts: &CallOptions) -> Result<CallCtx> {
        req.validate()
            .map_err(|e| ProviderError::InvalidRequest(e.0))?;
        if req.provider_id != self.provider_id {
            return Err(ProviderError::InvalidRequest(
                "provider_id does not match this provider".into(),
            ));
        }
        check_billing(self.billing, req.billing_mode)?;
        if req.data_class > self.max_class {
            return Err(ProviderError::PolicyDenied(
                "data-class-above-provider-max".into(),
            ));
        }
        let decision =
            self.gate
                .check(&self.provider_id, &self.target, self.opted_in, req, opts)?;
        Ok(CallCtx {
            provider_id: self.provider_id.clone(),
            policy_decision_id: decision,
        })
    }

    async fn preflight_model(&self, model: &str) -> Result<()> {
        if !self.check_models {
            return Ok(());
        }
        let models = self.client.list_models(&self.provider_id).await?;
        let Some(m) = models.iter().find(|m| m.id == model) else {
            return Err(ProviderError::ModelNotFound {
                provider: self.provider_id.clone(),
                model: model.into(),
            });
        };
        if let (Some(limit), Some(size)) = (self.max_model_bytes, m.size_bytes)
            && size > limit
        {
            return Err(ProviderError::ModelTooLarge {
                model: model.into(),
                size_bytes: size,
                limit_bytes: limit,
            });
        }
        Ok(())
    }
}

fn model_cap(m: &RemoteModel) -> ModelCapability {
    ModelCapability {
        model_id: m.id.chars().take(128).collect(),
        display_name: None,
        input_modalities: vec![InputModality::Text],
        output_modalities: vec![OutputModality::Text],
        context_tokens: m
            .n_ctx_train
            .and_then(|n| i64::try_from(n).ok())
            .filter(|n| *n >= 1),
        supports_tools: true,
        supports_streaming: true,
    }
}

#[async_trait]
impl Provider for HttpProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }
    fn adapter(&self) -> Adapter {
        self.adapter
    }
    fn billing_mode(&self) -> BillingMode {
        self.billing
    }
    fn capabilities(&self) -> &[Capability] {
        &[Capability::Inference]
    }

    /// Fuer Cloud-Ziele ohne Opt-in: kein Netzaufruf, `configured: false`, keine Modelle.
    async fn manifest(&self, trace_id: &str) -> Result<ProviderCapability> {
        let models = if self.opted_in {
            self.gate
                .check_discovery(&self.provider_id, &self.target, self.opted_in)?;
            self.client.list_models(&self.provider_id).await?
        } else {
            Vec::new()
        };
        let (loopback, region) = match &self.target {
            Target::Loopback => (true, None),
            Target::Remote { region, .. } => (false, Some(region.chars().take(32).collect())),
        };
        Ok(ProviderCapability {
            schema_version: SCHEMA_VERSION.into(),
            id: new_id("cap"),
            ts: now_rfc3339(),
            trace_id: trace_id.into(),
            server_context: None,
            provider_id: self.provider_id.clone(),
            adapter: self.adapter,
            billing_mode: self.billing,
            configured: self.opted_in,
            egress: Egress {
                host: self.host.clone(),
                region,
                loopback,
            },
            facts: self.facts,
            max_data_class: self.max_class,
            models: models.iter().map(model_cap).collect(),
            unsupported_parameters: Some(Vec::new()),
        })
    }
}

#[async_trait]
impl InferenceProvider for HttpProvider {
    async fn infer(&self, req: &InferenceRequest, opts: &CallOptions) -> Result<InferenceResponse> {
        let ctx = self.prepare(req, opts)?;
        self.preflight_model(&req.model).await?;
        self.client.chat(req, &ctx).await
    }

    async fn stream(&self, req: &InferenceRequest, opts: &CallOptions) -> Result<InferenceStream> {
        let ctx = self.prepare(req, opts)?;
        self.preflight_model(&req.model).await?;
        self.client.chat_stream(req, &ctx).await
    }
}
