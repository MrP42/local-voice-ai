//! Lokales Sprachmodell: Laufzeit-Download, Modelle und der Serverprozess.
//!
//! Die App spricht mit jedem Anbieter OpenAI-kompatibel; der gebuendelte
//! `llama-server` ist einfach ein weiterer -- nur dass die App ihn selbst
//! holt, startet und beendet. Muster wie bei der Piper-Laufzeit
//! (`managers::tts::models`): Katalogeintrag je Plattform, Download mit
//! Pruefsumme, entpacken, aufloesen.

pub mod runtime;
pub mod server;

use std::sync::{Arc, OnceLock};

pub use runtime::{LlmDownloadInfo, LlmDownloadKind, LlmRuntimeManager};
pub use server::{LocalLlmServer, LocalLlmStatus, StartOptions};

use crate::settings::PostProcessProvider;

/// Kennung der Anbieter-Vorlage, hinter der der lokale Server steht.
pub const LOCAL_PROVIDER_ID: &str = "local";

/// Platzhalter-Adresse der Vorlage: Port 0 heisst "den waehlt der Server
/// beim Start". Daran erkennt `llm_client` den lokalen Anbieter -- auch bei
/// einer Verbindung mit eigener Kennung.
pub const LOCAL_PLACEHOLDER_URL: &str = "http://127.0.0.1:0/v1";

/// Standard-Kontext fuer lokale Modelle. Gross genug fuer ein Protokoll,
/// klein genug, dass der KV-Cache nicht den Speicher frisst.
pub const DEFAULT_CONTEXT_TOKENS: u32 = 8192;

static RUNTIME: OnceLock<Arc<LlmRuntimeManager>> = OnceLock::new();
static SERVER: OnceLock<Arc<LocalLlmServer>> = OnceLock::new();

/// Einmal beim App-Start gesetzt. `llm_client` hat keinen `AppHandle`, muss
/// den lokalen Server aber starten koennen, bevor es ihn anspricht.
pub fn install_globals(runtime: Arc<LlmRuntimeManager>, server: Arc<LocalLlmServer>) {
    let _ = RUNTIME.set(runtime);
    let _ = SERVER.set(server);
}

/// Ist dieser Anbieter der lokale Server? Erkannt an Vorlage oder
/// Platzhalter-Adresse, nicht am Namen der Verbindung.
pub fn is_local(provider: &PostProcessProvider) -> bool {
    provider.id == LOCAL_PROVIDER_ID || provider.base_url.trim_end_matches('/') == LOCAL_PLACEHOLDER_URL.trim_end_matches('/')
}

/// Geladene lokale Modelle -- das, was ein "Modelle laden" fuer den lokalen
/// Anbieter liefert. Kein Server noetig.
pub fn downloaded_model_ids() -> Vec<String> {
    RUNTIME
        .get()
        .map(|r| {
            r.list_downloads()
                .into_iter()
                .filter(|d| d.kind == LlmDownloadKind::Model && d.is_downloaded)
                .map(|d| d.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Die Adresse, an die eine Anfrage fuer `model` geht: fuer jeden Anbieter
/// seine eigene -- fuer den lokalen die des laufenden Servers, der dafuer
/// bei Bedarf gestartet wird.
pub async fn resolve_base_url(provider: &PostProcessProvider, model: &str) -> Result<String, String> {
    if is_local(provider) {
        ensure_local(model).await
    } else {
        Ok(provider.base_url.clone())
    }
}

/// Sorgt dafuer, dass der lokale Server das Modell `model_id` bedient, und
/// liefert seine Adresse. Startet ihn bei Bedarf -- die Laufzeit wird per
/// Selbsttest gewaehlt.
pub async fn ensure_local(model_id: &str) -> Result<String, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "Lokales Sprachmodell nicht initialisiert".to_string())?;
    let server = SERVER
        .get()
        .ok_or_else(|| "Lokales Sprachmodell nicht initialisiert".to_string())?;
    if server.is_serving(model_id) {
        if let Some(url) = server.base_url() {
            return Ok(url);
        }
    }
    let model_path = runtime
        .model_path(model_id)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("Modell nicht geladen: {model_id}"))?;
    let (_, backend, binary) = runtime.resolve_runtime().await?;
    let gpu_layers = if backend == "cpu" { 0 } else { 99 };
    let port = server
        .ensure(
            &binary,
            StartOptions {
                model_id: model_id.to_string(),
                model_path,
                backend,
                context_tokens: DEFAULT_CONTEXT_TOKENS,
                gpu_layers,
            },
            Some(runtime.log_path()),
        )
        .await?;
    Ok(format!("http://127.0.0.1:{port}/v1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, base_url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.into(),
            label: id.into(),
            base_url: base_url.into(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    #[test]
    fn the_local_provider_is_recognised_by_template_or_placeholder() {
        assert!(is_local(&provider("local", "http://127.0.0.1:0/v1")));
        // Eine Verbindung mit eigener Kennung, aber Platzhalter-Adresse.
        assert!(is_local(&provider("local-abc", "http://127.0.0.1:0/v1/")));
        assert!(!is_local(&provider("ollama", "http://127.0.0.1:11434/v1")));
        assert!(!is_local(&provider("openai", "https://api.openai.com/v1")));
    }

    #[tokio::test]
    async fn a_cloud_provider_keeps_its_own_address() {
        let p = provider("openai", "https://api.openai.com/v1");
        assert_eq!(resolve_base_url(&p, "gpt-4.1").await.unwrap(), "https://api.openai.com/v1");
    }

    /// Ohne initialisierte Laufzeit kann kein lokaler Server starten -- der
    /// Fehler sagt das, statt in einen Verbindungsversuch zu laufen.
    #[tokio::test]
    async fn the_local_provider_fails_readably_when_uninitialised() {
        let p = provider("local", "http://127.0.0.1:0/v1");
        let err = resolve_base_url(&p, "llm-qwen3-4b-q4").await.unwrap_err();
        assert!(err.contains("nicht initialisiert") || err.contains("nicht geladen"), "{err}");
    }
}
