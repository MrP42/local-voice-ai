//! Provider-Trait mit Capabilities, getrennt nach Inferenz, Embeddings, STT, TTS, OCR.
//! Ein Provider meldet ueber `capabilities()`, was er kann; die Fachtraits sind
//! getrennt, damit ein Adapter nur das implementiert, was er wirklich liefert.

use crate::contract::{
    Adapter, BillingMode, InferenceRequest, InferenceResponse, ProviderCapability, StreamEvent,
};
use crate::error::Result;
use crate::gate::EgressTarget;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wai_security::{ApprovalToken, Principal, TenantId, payload_hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Inference,
    Embeddings,
    Stt,
    Tts,
    Ocr,
}

/// Nachweis fuer das Egress-Gate. Nicht von aussen konstruierbar (private Felder), NICHT
/// `Clone`: er entsteht nur ueber `from_approval` und traegt die Freigabe-Referenz plus
/// Bindungsdaten (Tenant, Payload-Hash, kanonischer Ziel-Endpunkt, Region, Policy-Version).
/// Die Freigabe wird nicht hier, sondern atomar im Gate beim Senden ueber
/// `ApprovalLedger::verify_and_consume` verbraucht; ein zweiter Aufruf mit derselben
/// Freigabe scheitert dort (`ApprovalRequired`).
///
/// Es gibt bewusst keinen Redaktionsnachweis: es existiert kein vertrauenswuerdiger
/// Redaktionspruefer. Ein spaeterer Redaktionsdienst kann einen nicht konstruierbaren
/// Attestationstyp nachruesten; bis dahin gehen PERSONAL-Daten nie in die Cloud.
pub struct EgressEvidence {
    tenant: TenantId,
    payload_hash: [u8; 32],
    endpoint: String,
    region: String,
    policy_version: u32,
    record: ApprovalToken,
}

impl std::fmt::Debug for EgressEvidence {
    // Approval-ID ist eine Bearer-Capability, Payload-Hash ein Inhaltsabdruck: nie ausgeben.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EgressEvidence")
            .field("tenant", &self.tenant)
            .field("payload_hash", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("policy_version", &self.policy_version)
            .field("approval_id", &"<redacted>")
            .finish()
    }
}

/// Kanonische Payload-Bytes einer Anfrage (Grundlage des Payload-Hashes).
pub(crate) fn payload_bytes(req: &InferenceRequest) -> Vec<u8> {
    serde_json::to_vec(req).unwrap_or_default()
}

impl EgressEvidence {
    /// Bindet eine ausgestellte Freigabe an Principal, Anfrage, Ziel und Policy-Version.
    /// `target` kommt vom Provider (`HttpProvider::egress_target`). Der Verbrauch
    /// erfolgt erst im Gate.
    pub fn from_approval(
        record: ApprovalToken,
        principal: &Principal,
        req: &InferenceRequest,
        target: &EgressTarget,
        policy_version: u32,
    ) -> Self {
        Self {
            tenant: principal.tenant().clone(),
            payload_hash: payload_hash(&payload_bytes(req)),
            endpoint: target.endpoint.clone(),
            region: target.region.clone(),
            policy_version,
            record,
        }
    }

    /// Bindung an Tenant, Payload, Ziel (Endpunkt + Region) und Policy-Version.
    pub(crate) fn binds(
        &self,
        tenant: Option<&TenantId>,
        payload: &[u8],
        target: &EgressTarget,
        policy_version: u32,
    ) -> bool {
        tenant == Some(&self.tenant)
            && self.payload_hash == payload_hash(payload)
            && self.endpoint == target.endpoint
            && self.region == target.region
            && self.policy_version == policy_version
    }
    pub(crate) fn record(&self) -> &ApprovalToken {
        &self.record
    }
}

/// Pro Aufruf: Tenant (aus dem authentisierten Principal) und Nachweis fuer das
/// Egress-Gate (nur fuer Cloud-Routen relevant).
#[derive(Debug, Default)]
pub struct CallOptions {
    tenant: Option<TenantId>,
    evidence: Option<EgressEvidence>,
}

impl CallOptions {
    /// Aufruf ohne Nachweise (Public-Daten; sonst lehnt das Gate ab).
    pub fn new(principal: &Principal) -> Self {
        Self {
            tenant: Some(principal.tenant().clone()),
            evidence: None,
        }
    }
    pub fn with_evidence(mut self, evidence: EgressEvidence) -> Self {
        self.evidence = Some(evidence);
        self
    }
    pub(crate) fn tenant(&self) -> Option<&TenantId> {
        self.tenant.as_ref()
    }
    pub(crate) fn evidence(&self) -> Option<&EgressEvidence> {
        self.evidence.as_ref()
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn adapter(&self) -> Adapter;
    fn billing_mode(&self) -> BillingMode;
    fn capabilities(&self) -> &[Capability];
    fn supports(&self, c: Capability) -> bool {
        self.capabilities().contains(&c)
    }
    /// Capability-Manifest (`contracts/core/provider-capability`), Modelle live abgefragt.
    async fn manifest(&self, trace_id: &str) -> Result<ProviderCapability>;
}

#[async_trait]
pub trait InferenceProvider: Provider {
    /// Nicht streamende Inferenz; die Anfrage darf `stream: false` oder `true` tragen.
    async fn infer(&self, req: &InferenceRequest, opts: &CallOptions) -> Result<InferenceResponse>;
    /// Streaming per SSE. Fehler vor dem ersten Byte kommen als `Err`; spaetere Fehler
    /// als `StreamEvent::Error` gefolgt von genau einem `Done`.
    async fn stream(&self, req: &InferenceRequest, opts: &CallOptions) -> Result<InferenceStream>;
}

/// Laufender Stream mit Abbruch. `cancel()` oder Drop schliesst die Verbindung zum
/// Provider; danach endet der Stream mit `Done { finish_reason: cancelled }`.
pub struct InferenceStream {
    rx: mpsc::Receiver<StreamEvent>,
    cancel: CancellationToken,
}

impl InferenceStream {
    pub(crate) fn new(rx: mpsc::Receiver<StreamEvent>, cancel: CancellationToken) -> Self {
        Self { rx, cancel }
    }
    /// Kanal fuer Adapter ausserhalb des Crates: der Adapter schreibt Ereignisse in den Sender und
    /// beobachtet `cancel_handle()`; Abbruch oder Drop des Streams schliesst den Kanal.
    pub fn channel(buffer: usize) -> (mpsc::Sender<StreamEvent>, Self) {
        let (tx, rx) = mpsc::channel(buffer);
        (tx, Self::new(rx, CancellationToken::new()))
    }
    pub async fn next(&mut self) -> Option<StreamEvent> {
        self.rx.recv().await
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn cancel_handle(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

impl Drop for InferenceStream {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl std::fmt::Debug for InferenceStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InferenceStream")
    }
}

// ---------------------------------------------------------------- Weitere Capabilities

#[derive(Debug, Clone)]
pub struct EmbeddingRequest {
    pub model: String,
    pub inputs: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct EmbeddingResponse {
    pub model: String,
    pub vectors: Vec<Vec<f32>>,
}
#[async_trait]
pub trait EmbeddingProvider: Provider {
    async fn embed(&self, req: &EmbeddingRequest, opts: &CallOptions) -> Result<EmbeddingResponse>;
}

#[derive(Debug, Clone)]
pub struct SttRequest {
    pub model: String,
    pub audio: Vec<u8>,
    pub mime_type: String,
    pub language: Option<String>,
}
#[derive(Debug, Clone)]
pub struct SttResponse {
    pub text: String,
    pub language: Option<String>,
}
#[async_trait]
pub trait SttProvider: Provider {
    async fn transcribe(&self, req: &SttRequest, opts: &CallOptions) -> Result<SttResponse>;
}

#[derive(Debug, Clone)]
pub struct TtsRequest {
    pub model: String,
    pub text: String,
    pub voice: Option<String>,
}
#[derive(Debug, Clone)]
pub struct TtsResponse {
    pub audio: Vec<u8>,
    pub mime_type: String,
}
#[async_trait]
pub trait TtsProvider: Provider {
    async fn synthesize(&self, req: &TtsRequest, opts: &CallOptions) -> Result<TtsResponse>;
}

#[derive(Debug, Clone)]
pub struct OcrRequest {
    pub model: String,
    pub document: Vec<u8>,
    pub mime_type: String,
}
#[derive(Debug, Clone)]
pub struct OcrPage {
    pub index: u32,
    pub text: String,
}
#[derive(Debug, Clone)]
pub struct OcrResponse {
    pub pages: Vec<OcrPage>,
}
#[async_trait]
pub trait OcrProvider: Provider {
    async fn recognize(&self, req: &OcrRequest, opts: &CallOptions) -> Result<OcrResponse>;
}
