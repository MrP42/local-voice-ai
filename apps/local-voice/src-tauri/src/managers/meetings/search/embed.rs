//! Embeddings fuer den Such-Index (M4 §4, P4b): der Vertrag `Embedder` und
//! seine Umsetzung `LlamaEmbedder` ueber den zweiten `llama-server` im
//! Embedding-Modus (BGE-M3 Q8_0, `--pooling cls`, M4 D3).
//!
//! BGE-M3 braucht keine Anweisung vor der Abfrage: Abfrage und Dokument werden
//! gleich eingebettet (`EmbedKind` bleibt fuer ein spaeteres Modell, das sie
//! unterscheidet). Kein Text aus Besprechungen landet im Log oder in einer
//! Fehlermeldung; geloggt werden Codes, Anzahlen und Laengen.

use futures_util::future::BoxFuture;
use std::time::Duration;

use crate::managers::llm;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbedKind {
    Query,
    Document,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbedError {
    /// Modell nicht heruntergeladen (oder keine Laufzeit).
    NoModel,
    /// RAM-Gate hat den Start verweigert.
    MemoryLow,
    /// Server gerade nicht bereit (laedt, ueberlastet).
    Busy,
    /// Alles andere (Absturz, HTTP-Fehler, kaputte Antwort). Der Text ist
    /// technisch, nie Besprechungstext.
    Failed(String),
}

impl EmbedError {
    /// Kurzcode fuer Log und Anzeige (nie der Text der Anfrage).
    pub fn code(&self) -> &'static str {
        match self {
            EmbedError::NoModel => "no_model",
            EmbedError::MemoryLow => "memory_low",
            EmbedError::Busy => "busy",
            EmbedError::Failed(_) => "failed",
        }
    }
}

/// Liefert Vektoren fuer Texte. Die Methoden mit Voreinstellung steuern den
/// Server-Lebenszyklus; ein Fake (Tests, P4c) braucht nur `embed` und `model_id`.
pub trait Embedder: Send + Sync {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
        kind: EmbedKind,
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>>;

    fn model_id(&self) -> &str;

    /// Ist das Modell vorhanden? Ohne Modell startet der Indexer keinen
    /// Server und laesst die Besprechungen bei `lexical`.
    fn available(&self) -> bool {
        true
    }

    /// Laeuft der Server gerade?
    fn server_running(&self) -> bool {
        false
    }

    /// Beendet den Server, wenn er `idle` lang ungenutzt war.
    fn release_if_idle(&self, _idle: Duration) {}

    /// Beendet den Server sofort (Gate geschlossen, Ende des Headless-Laufs).
    fn release_now(&self) {}
}

/// Bildet einen Fehler von `llm::ensure_embedding` (`<code>: <text>`) ab.
pub fn embed_error_from(message: &str) -> EmbedError {
    let code = message.split(':').next().unwrap_or("").trim();
    match code {
        "no_model" | "no_runtime" => EmbedError::NoModel,
        "memory_low" => EmbedError::MemoryLow,
        "busy" => EmbedError::Busy,
        _ => EmbedError::Failed(message.chars().take(300).collect()),
    }
}

/// Liest die Antwort von `/v1/embeddings` (OpenAI-Form `{data: [{index,
/// embedding}]}`) in Eingabereihenfolge. Anzahl, Indizes und Laengen muessen
/// passen, sonst `Failed` -- eine halbe Antwort wird nie gespeichert.
pub fn parse_embeddings(json: &serde_json::Value, expected: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let bad = |what: &str| EmbedError::Failed(format!("embedding_response_invalid: {what}"));
    let data = json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| bad("data fehlt"))?;
    if data.len() != expected {
        return Err(bad("Anzahl"));
    }
    let mut out: Vec<Option<Vec<f32>>> = vec![None; expected];
    for (pos, item) in data.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(|i| i.as_u64())
            .map(|i| i as usize)
            .unwrap_or(pos);
        if index >= expected || out[index].is_some() {
            return Err(bad("Index"));
        }
        let values = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| bad("embedding fehlt"))?;
        let vec: Vec<f32> = values
            .iter()
            .map(|v| v.as_f64().map(|f| f as f32))
            .collect::<Option<Vec<f32>>>()
            .ok_or_else(|| bad("Zahl"))?;
        if vec.is_empty() || vec.iter().any(|x| !x.is_finite()) {
            return Err(bad("Werte"));
        }
        out[index] = Some(vec);
    }
    let vectors: Vec<Vec<f32>> = out.into_iter().collect::<Option<_>>().ok_or_else(|| bad("Luecke"))?;
    let dim = vectors.first().map(Vec::len).unwrap_or(0);
    if vectors.iter().any(|v| v.len() != dim) {
        return Err(bad("Dimension"));
    }
    Ok(vectors)
}

/// Zeitlimit je Anfrage: 16 Chunks auf der CPU brauchen ~16 s (M4: 1 Chunk/s).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Der echte Embedder: startet bei Bedarf den Embedding-Server
/// (`llm::ensure_embedding`, RAM-Gate, Job-Objekt) und fragt ihn per HTTP.
pub struct LlamaEmbedder {
    model_id: String,
    http: reqwest::Client,
}

impl LlamaEmbedder {
    pub fn new(model_id: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client"),
        }
    }

    async fn embed_inner(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let _use = llm::EmbeddingUse::begin();
        let base = llm::ensure_embedding(&self.model_id)
            .await
            .map_err(|e| embed_error_from(&e))?;
        let body = serde_json::json!({
            "model": self.model_id,
            "input": texts,
            "encoding_format": "float",
        });
        let response = self
            .http
            .post(format!("{base}/embeddings"))
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbedError::Failed(format!("embedding_http: {}", e.without_url())))?;
        let status = response.status();
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            return Err(EmbedError::Busy);
        }
        if !status.is_success() {
            // Der Fehlertext des Servers kann Eingabetext zitieren: nicht weitergeben.
            return Err(EmbedError::Failed(format!("embedding_http_status: {}", status.as_u16())));
        }
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|_| EmbedError::Failed("embedding_response_invalid: json".into()))?;
        parse_embeddings(&json, texts.len())
    }
}

impl Embedder for LlamaEmbedder {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
        _kind: EmbedKind,
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(self.embed_inner(texts))
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn available(&self) -> bool {
        llm::embedding_model_ready(&self.model_id)
    }

    fn server_running(&self) -> bool {
        llm::embedding_running()
    }

    fn release_if_idle(&self, idle: Duration) {
        llm::stop_embedding_if_idle(idle);
    }

    fn release_now(&self) {
        llm::stop_embedding();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn server_errors_map_to_codes() {
        assert_eq!(embed_error_from("no_model: fehlt"), EmbedError::NoModel);
        assert_eq!(embed_error_from("no_runtime: keine"), EmbedError::NoModel);
        assert_eq!(embed_error_from("memory_low: 1 GB frei"), EmbedError::MemoryLow);
        assert_eq!(embed_error_from("busy: laedt"), EmbedError::Busy);
        assert_eq!(embed_error_from("failed: weg").code(), "failed");
        assert_eq!(embed_error_from("irgendwas").code(), "failed");
    }

    #[test]
    fn embeddings_are_returned_in_input_order() {
        let body = json!({"data": [
            {"index": 1, "embedding": [0.0, 1.0]},
            {"index": 0, "embedding": [1.0, 0.0]},
        ]});
        let v = parse_embeddings(&body, 2).unwrap();
        assert_eq!(v, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[test]
    fn a_partial_or_broken_response_is_rejected() {
        let short = json!({"data": [{"index": 0, "embedding": [1.0]}]});
        assert_eq!(parse_embeddings(&short, 2).unwrap_err().code(), "failed");
        let dup = json!({"data": [
            {"index": 0, "embedding": [1.0]}, {"index": 0, "embedding": [1.0]}]});
        assert!(parse_embeddings(&dup, 2).is_err());
        let dims = json!({"data": [
            {"index": 0, "embedding": [1.0]}, {"index": 1, "embedding": [1.0, 2.0]}]});
        assert!(parse_embeddings(&dims, 2).is_err());
        let nan = json!({"data": [{"index": 0, "embedding": ["x"]}]});
        assert!(parse_embeddings(&nan, 1).is_err());
        assert!(parse_embeddings(&json!({"error": "x"}), 1).is_err());
    }

    /// Ohne Laufzeit/Modell endet der echte Embedder mit `NoModel` und
    /// startet keinen Prozess; eine leere Eingabe braucht keinen Server.
    #[tokio::test]
    async fn the_llama_embedder_without_model_reports_no_model() {
        let _g = crate::managers::llm::tests::global_embed_lock();
        let e = LlamaEmbedder::new(llm::EMBED_MODEL_ID);
        assert!(!e.available());
        assert_eq!(e.embed(&[], EmbedKind::Query).await.unwrap(), Vec::<Vec<f32>>::new());
        let err = e
            .embed(&["Hallo".to_string()], EmbedKind::Document)
            .await
            .unwrap_err();
        assert_eq!(err, EmbedError::NoModel);
        assert!(!e.server_running());
    }
}
