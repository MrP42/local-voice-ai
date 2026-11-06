//! Serde-Typen fuer `contracts/core` (ProviderCapability, InferenceRequest,
//! InferenceResponse, StreamEvent, Error). Strikt: unbekannte Felder und Varianten
//! scheitern beim Decode; Muster und Grenzen prueft `validate()` (Stufe `validate`).

use regex::Regex;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::OnceLock;

pub const SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);
impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "contract.invalid_value: {}", self.0)
    }
}
impl std::error::Error for ValidationError {}

fn re(cell: &'static OnceLock<Regex>, p: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(p).expect("static regex"))
}
fn check(ok: bool, what: &str) -> Result<(), ValidationError> {
    if ok {
        Ok(())
    } else {
        Err(ValidationError(what.to_string()))
    }
}
fn schema_version(s: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(
        re(&R, r"^0\.1\.(0|[1-9][0-9]*)$").is_match(s),
        "schema_version",
    )
}
fn id(s: &str, what: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(
        re(&R, r"^[a-z]{2,12}_[0-9A-HJKMNP-TV-Z]{26}$").is_match(s),
        what,
    )
}
fn timestamp(s: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(
        re(
            &R,
            r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]{1,9})?Z$",
        )
        .is_match(s),
        "ts",
    )
}
fn trace_id(s: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(re(&R, r"^[0-9a-f]{32}$").is_match(s), "trace_id")
}
fn slug(s: &str, what: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(
        s.len() <= 64 && re(&R, r"^[a-z][a-z0-9]*([._-][a-z0-9]+)*$").is_match(s),
        what,
    )
}
fn host(s: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(
        s.len() <= 253
            && re(
                &R,
                r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$",
            )
            .is_match(s),
        "host",
    )
}
fn sha256(s: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(re(&R, r"^sha256:[0-9a-f]{64}$").is_match(s), "sha256")
}
fn len_between(s: &str, min: usize, max: usize, what: &str) -> Result<(), ValidationError> {
    let n = s.chars().count();
    check(n >= min && n <= max, what)
}
fn tool_name(s: &str) -> Result<(), ValidationError> {
    static R: OnceLock<Regex> = OnceLock::new();
    check(re(&R, r"^[a-zA-Z0-9_.-]{1,64}$").is_match(s), "tool name")
}
fn non_negative(v: i64, what: &str) -> Result<(), ValidationError> {
    check(v >= 0, what)
}

// ---------------------------------------------------------------- Enums

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    OpenaiCompatible,
    Anthropic,
    ChatgptPlan,
    LocalLlamaServer,
    OllamaByo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BillingMode {
    Subscription,
    Api,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DataClass {
    Public,
    Internal,
    Personal,
    Special,
    Restricted,
}

impl From<DataClass> for wai_security::DataClass {
    fn from(c: DataClass) -> Self {
        match c {
            DataClass::Public => wai_security::DataClass::Public,
            DataClass::Internal => wai_security::DataClass::Internal,
            DataClass::Personal => wai_security::DataClass::Personal,
            DataClass::Special => wai_security::DataClass::Special,
            DataClass::Restricted => wai_security::DataClass::Restricted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriState {
    Yes,
    No,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authn {
    Oidc,
    LocalSession,
    Service,
    ChatgptIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Developer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputModality {
    Text,
    Image,
    File,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputModality {
    Text,
    Image,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    CallStarted,
    ArgumentsDelta,
    CallCompleted,
}

// ---------------------------------------------------------------- Gemeinsam

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerContext {
    pub principal_id: String,
    pub tenant_id: String,
    pub authn: Authn,
}
impl ServerContext {
    fn validate(&self) -> Result<(), ValidationError> {
        id(&self.principal_id, "principal_id")?;
        id(&self.tenant_id, "tenant_id")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}
impl Usage {
    pub fn new(input_tokens: i64, output_tokens: i64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens: None,
            cost_micros: None,
            currency: None,
        }
    }
    fn validate(&self) -> Result<(), ValidationError> {
        non_negative(self.input_tokens, "usage.input_tokens")?;
        non_negative(self.output_tokens, "usage.output_tokens")?;
        if let Some(v) = self.cached_input_tokens {
            non_negative(v, "usage.cached_input_tokens")?;
        }
        if let Some(v) = self.cost_micros {
            non_negative(v, "usage.cost_micros")?;
        }
        if let Some(c) = &self.currency {
            static R: OnceLock<Regex> = OnceLock::new();
            check(re(&R, r"^[A-Z]{3}$").is_match(c), "usage.currency")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Error

/// `diagnostic.redacted` ist per Vertrag die Konstante `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactedTrue;
impl Serialize for RedactedTrue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(true)
    }
}
impl<'de> Deserialize<'de> for RedactedTrue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if bool::deserialize(d)? {
            Ok(RedactedTrue)
        } else {
            Err(D::Error::custom("diagnostic.redacted must be true"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub redacted: RedactedTrue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
    pub diagnostic: Diagnostic,
}
impl ContractError {
    pub fn validate(&self) -> Result<(), ValidationError> {
        static R: OnceLock<Regex> = OnceLock::new();
        check(
            re(&R, r"^[a-z]+\.[a-z_]+$").is_match(&self.code),
            "error.code",
        )?;
        len_between(&self.message, 1, 1000, "error.message")?;
        if let Some(v) = self.retry_after_ms {
            non_negative(v, "error.retry_after_ms")?;
        }
        if let Some(t) = &self.diagnostic.trace_id {
            trace_id(t)?;
        }
        if let Some(c) = &self.diagnostic.component {
            slug(c, "diagnostic.component")?;
        }
        if let Some(d) = &self.diagnostic.detail {
            len_between(d, 0, 2000, "diagnostic.detail")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- ProviderCapability

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Egress {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub loopback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    pub dpf: TriState,
    pub no_training: TriState,
    pub eu_region: TriState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapability {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub input_modalities: Vec<InputModality>,
    pub output_modalities: Vec<OutputModality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>,
    pub supports_tools: bool,
    pub supports_streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapability {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_context: Option<ServerContext>,
    pub provider_id: String,
    pub adapter: Adapter,
    pub billing_mode: BillingMode,
    pub configured: bool,
    pub egress: Egress,
    pub facts: Facts,
    pub max_data_class: DataClass,
    pub models: Vec<ModelCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported_parameters: Option<Vec<String>>,
}
impl ProviderCapability {
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema_version(&self.schema_version)?;
        id(&self.id, "id")?;
        timestamp(&self.ts)?;
        trace_id(&self.trace_id)?;
        if let Some(c) = &self.server_context {
            c.validate()?;
        }
        slug(&self.provider_id, "provider_id")?;
        host(&self.egress.host)?;
        if let Some(r) = &self.egress.region {
            len_between(r, 0, 32, "egress.region")?;
        }
        for m in &self.models {
            len_between(&m.model_id, 1, 128, "model_id")?;
            if let Some(n) = &m.display_name {
                len_between(n, 0, 128, "display_name")?;
            }
            check(
                !m.input_modalities.is_empty() && !m.output_modalities.is_empty(),
                "modalities",
            )?;
            if let Some(c) = m.context_tokens {
                check(c >= 1, "context_tokens")?;
            }
        }
        if let Some(p) = &self.unsupported_parameters {
            static R: OnceLock<Regex> = OnceLock::new();
            for s in p {
                check(re(&R, r"^[a-z_]+$").is_match(s), "unsupported_parameters")?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- InferenceRequest

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextPart {
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPart {
    pub artifact_id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text(TextPart),
    ArtifactRef(ArtifactPart),
}
impl ContentPart {
    pub fn text(t: impl Into<String>) -> Self {
        ContentPart::Text(TextPart { text: t.into() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputMessage {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters_schema: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceRequest {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    pub provider_id: String,
    pub model: String,
    pub billing_mode: BillingMode,
    pub data_class: DataClass,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<InputMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    pub stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}
impl InferenceRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema_version(&self.schema_version)?;
        id(&self.id, "id")?;
        timestamp(&self.ts)?;
        trace_id(&self.trace_id)?;
        slug(&self.provider_id, "provider_id")?;
        len_between(&self.model, 1, 128, "model")?;
        slug(&self.purpose, "purpose")?;
        if let Some(i) = &self.instructions {
            len_between(i, 0, 100_000, "instructions")?;
        }
        check(!self.input.is_empty(), "input must not be empty")?;
        for m in &self.input {
            check(!m.content.is_empty(), "content must not be empty")?;
            for p in &m.content {
                match p {
                    ContentPart::Text(t) => len_between(&t.text, 0, 1_000_000, "text")?,
                    ContentPart::ArtifactRef(a) => {
                        id(&a.artifact_id, "artifact_id")?;
                        sha256(&a.sha256)?;
                    }
                }
            }
        }
        for t in self.tools.iter().flatten() {
            tool_name(&t.name)?;
            if let Some(d) = &t.description {
                len_between(d, 0, 4000, "tool.description")?;
            }
        }
        if let Some(m) = self.max_output_tokens {
            check(m >= 1, "max_output_tokens")?;
        }
        if let Some(k) = &self.idempotency_key {
            len_between(k, 8, 128, "idempotency_key")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- InferenceResponse

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputText {
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputToolCall {
    pub tool_call_id: String,
    pub name: String,
    pub arguments_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputPart {
    Text(OutputText),
    ToolCall(OutputToolCall),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceResponse {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_context: Option<ServerContext>,
    pub request_id: String,
    pub provider_id: String,
    pub model: String,
    pub billing_mode: BillingMode,
    pub output: Vec<OutputPart>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    pub policy_decision_id: String,
}
impl InferenceResponse {
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema_version(&self.schema_version)?;
        id(&self.id, "id")?;
        timestamp(&self.ts)?;
        trace_id(&self.trace_id)?;
        if let Some(c) = &self.server_context {
            c.validate()?;
        }
        id(&self.request_id, "request_id")?;
        slug(&self.provider_id, "provider_id")?;
        len_between(&self.model, 1, 128, "model")?;
        for p in &self.output {
            if let OutputPart::ToolCall(t) = p {
                len_between(&t.tool_call_id, 1, 128, "tool_call_id")?;
                tool_name(&t.name)?;
                len_between(&t.arguments_json, 0, 1_000_000, "arguments_json")?;
            }
        }
        self.usage.validate()?;
        id(&self.policy_decision_id, "policy_decision_id")
    }
}

// ---------------------------------------------------------------- StreamEvent

macro_rules! stream_struct {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub schema_version: String,
            pub id: String,
            pub ts: String,
            pub trace_id: String,
            pub request_id: String,
            pub seq: i64,
            $($(#[$fm])* pub $f: $t,)*
        }
    };
}

stream_struct!(DeltaEvent { text: String });
stream_struct!(ToolEvent {
    tool_call_id: String,
    name: String,
    phase: ToolPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arguments_delta: Option<String>,
});
stream_struct!(UsageEvent { usage: Usage });
stream_struct!(ErrorEvent {
    error: ContractError
});
stream_struct!(DoneEvent {
    finish_reason: FinishReason
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Delta(DeltaEvent),
    Tool(ToolEvent),
    Usage(UsageEvent),
    Error(ErrorEvent),
    Done(DoneEvent),
}
impl StreamEvent {
    fn head(&self) -> (&str, &str, &str, &str, &str, i64) {
        macro_rules! h {
            ($e:expr) => {
                (
                    &$e.schema_version[..],
                    &$e.id[..],
                    &$e.ts[..],
                    &$e.trace_id[..],
                    &$e.request_id[..],
                    $e.seq,
                )
            };
        }
        match self {
            StreamEvent::Delta(e) => h!(e),
            StreamEvent::Tool(e) => h!(e),
            StreamEvent::Usage(e) => h!(e),
            StreamEvent::Error(e) => h!(e),
            StreamEvent::Done(e) => h!(e),
        }
    }
    pub fn seq(&self) -> i64 {
        self.head().5
    }
    pub fn request_id(&self) -> &str {
        self.head().4
    }
    pub fn is_done(&self) -> bool {
        matches!(self, StreamEvent::Done(_))
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        let (sv, i, ts, tr, rq, seq) = self.head();
        schema_version(sv)?;
        id(i, "id")?;
        timestamp(ts)?;
        trace_id(tr)?;
        id(rq, "request_id")?;
        non_negative(seq, "seq")?;
        match self {
            StreamEvent::Tool(t) => {
                tool_name(&t.name)?;
                len_between(&t.tool_call_id, 1, 128, "tool_call_id")
            }
            StreamEvent::Usage(u) => u.usage.validate(),
            StreamEvent::Error(e) => e.error.validate(),
            _ => Ok(()),
        }
    }
}
