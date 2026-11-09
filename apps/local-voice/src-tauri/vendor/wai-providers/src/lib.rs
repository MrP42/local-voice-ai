//! SDK-05: Provider-Schicht. Capabilities getrennt nach Inferenz, Embeddings, STT, TTS,
//! OCR; lokaler llama-server als Standard, Ollama nur BYO, Cloud nur BYOK mit Opt-in und
//! Policy-Gate (`wai-security`). Siehe docs/adr/ADR-0002-plattformkern.md §5 und §6.

pub mod contract;
pub mod error;
pub mod gate;
pub mod http_provider;
mod openai;
pub mod provider;
pub mod util;

// Fuer Adapter ausserhalb des Crates (Apps, z. B. die Abo-Route), die die Provider-Traits implementieren.
pub use async_trait::async_trait;
pub use error::{ProviderError, Result};
pub use gate::{EgressGate, EgressTarget, Target, approval_scope};
pub use http_provider::{ApiProviderConfig, HttpProvider, LocalLlamaConfig, OllamaByoConfig};
pub use openai::{SharedStore, Timeouts};
pub use provider::{
    CallOptions, Capability, EgressEvidence, EmbeddingProvider, InferenceProvider, InferenceStream,
    OcrProvider, Provider, SttProvider, TtsProvider,
};
