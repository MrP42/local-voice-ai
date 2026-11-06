//! Gemeinsamer Kern fuer OpenAI-kompatible HTTP-Endpunkte (llama-server, Ollama `/v1`,
//! OpenAI, Mistral, OpenRouter): Chat Completions, Modellliste, SSE-Streaming, Abbruch.
//! Schluessel werden erst beim Senden aus dem `SecretStore` gelesen und nie gespeichert.

use crate::contract::{
    ContentPart, DeltaEvent, DoneEvent, ErrorEvent, FinishReason, InferenceRequest,
    InferenceResponse, OutputPart, OutputText, OutputToolCall, Role, SCHEMA_VERSION, StreamEvent,
    ToolEvent, ToolPhase, Usage, UsageEvent,
};
use crate::error::{ProviderError, Result, safe_detail};
use crate::provider::InferenceStream;
use crate::util::{new_id, now_rfc3339};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, HeaderValue, RETRY_AFTER};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wai_security::{SecretHandle, SecretStore};

pub type SharedStore = Arc<dyn SecretStore + Send + Sync>;

#[derive(Clone)]
pub(crate) enum Auth {
    None,
    Bearer {
        handle: SecretHandle,
        store: SharedStore,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    /// Gesamtdauer fuer nicht streamende Aufrufe und Modellliste.
    pub request: Duration,
    /// Maximale Stille zwischen zwei Stream-Chunks.
    pub stream_idle: Duration,
}
impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(300),
            stream_idle: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteModel {
    pub id: String,
    pub size_bytes: Option<u64>,
    pub n_ctx_train: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct OpenAiClient {
    http: reqwest::Client,
    /// Basis inkl. `/v1`, ohne Schraegstrich am Ende.
    base: String,
    auth: Auth,
    timeouts: Timeouts,
}

impl std::fmt::Debug for OpenAiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let auth = match self.auth {
            Auth::None => "none",
            Auth::Bearer { .. } => "bearer(secret-handle)",
        };
        f.debug_struct("OpenAiClient")
            .field("base", &self.base)
            .field("auth", &auth)
            .finish()
    }
}

/// Kontext fuer erzeugte Vertragsobjekte.
#[derive(Clone)]
pub(crate) struct CallCtx {
    pub provider_id: String,
    pub policy_decision_id: String,
}

impl OpenAiClient {
    pub fn new(base: String, auth: Auth, timeouts: Timeouts) -> Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(timeouts.connect)
            .build()
            .map_err(|_| ProviderError::NotConfigured("http client".into()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            auth,
            timeouts,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn authorize(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        match &self.auth {
            Auth::None => Ok(rb),
            Auth::Bearer { handle, store } => {
                let secret = store
                    .get(handle)
                    .map_err(|_| ProviderError::SecretUnavailable)?;
                let key = std::str::from_utf8(secret.expose())
                    .map_err(|_| ProviderError::SecretUnavailable)?;
                let mut hv = HeaderValue::from_str(&format!("Bearer {key}"))
                    .map_err(|_| ProviderError::SecretUnavailable)?;
                hv.set_sensitive(true);
                Ok(rb.header(AUTHORIZATION, hv))
            }
        }
    }

    fn transport_error(e: &reqwest::Error) -> ProviderError {
        let detail = if e.is_timeout() {
            "request timed out"
        } else if e.is_connect() {
            "connection failed"
        } else {
            "transport error"
        };
        ProviderError::Unavailable {
            detail: detail.into(),
            retryable: true,
            retry_after_ms: None,
        }
    }

    async fn checked(
        &self,
        rb: reqwest::RequestBuilder,
        provider: &str,
        model: &str,
    ) -> Result<reqwest::Response> {
        let resp = self
            .authorize(rb)?
            .send()
            .await
            .map_err(|e| Self::transport_error(&e))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let retry_after_ms = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|s| s.saturating_mul(1000));
        let body = resp.text().await.unwrap_or_default();
        Err(map_status(
            status.as_u16(),
            &body,
            retry_after_ms,
            provider,
            model,
        ))
    }

    pub async fn list_models(&self, provider: &str) -> Result<Vec<RemoteModel>> {
        let rb = self
            .http
            .get(self.url("/models"))
            .timeout(self.timeouts.request);
        let resp = self.checked(rb, provider, "").await?;
        let v: Value = resp
            .json()
            .await
            .map_err(|_| ProviderError::Protocol("models response is not JSON".into()))?;
        let data = v
            .get("data")
            .and_then(Value::as_array)
            .or_else(|| v.get("models").and_then(Value::as_array))
            .ok_or_else(|| ProviderError::Protocol("models response has no 'data' array".into()))?;
        Ok(data
            .iter()
            .filter_map(|m| {
                let id = m
                    .get("id")
                    .or_else(|| m.get("model"))
                    .and_then(Value::as_str)?
                    .to_string();
                let meta = m.get("meta");
                Some(RemoteModel {
                    id,
                    size_bytes: meta.and_then(|x| x.get("size")).and_then(Value::as_u64),
                    n_ctx_train: meta
                        .and_then(|x| x.get("n_ctx_train"))
                        .and_then(Value::as_u64),
                })
            })
            .collect())
    }

    pub async fn chat(&self, req: &InferenceRequest, ctx: &CallCtx) -> Result<InferenceResponse> {
        let body = build_body(req, false)?;
        let rb = self
            .http
            .post(self.url("/chat/completions"))
            .timeout(self.timeouts.request)
            .json(&body);
        let resp = self.checked(rb, &ctx.provider_id, &req.model).await?;
        let v: Value = resp
            .json()
            .await
            .map_err(|_| ProviderError::Protocol("response is not JSON".into()))?;
        parse_completion(&v, req, ctx)
    }

    pub async fn chat_stream(
        &self,
        req: &InferenceRequest,
        ctx: &CallCtx,
    ) -> Result<InferenceStream> {
        let body = build_body(req, true)?;
        // Kein Gesamt-Timeout: Streams duerfen lange laufen; Stille begrenzt `stream_idle`.
        let rb = self.http.post(self.url("/chat/completions")).json(&body);
        let resp = self.checked(rb, &ctx.provider_id, &req.model).await?;
        let (tx, rx) = mpsc::channel(64);
        let cancel = CancellationToken::new();
        let pump = Pump {
            em: Emitter {
                request_id: req.id.clone(),
                trace_id: req.trace_id.clone(),
                seq: 0,
            },
            tx,
            cancel: cancel.clone(),
            idle: self.timeouts.stream_idle,
        };
        tokio::spawn(pump.run(resp));
        Ok(InferenceStream::new(rx, cancel))
    }
}

// ---------------------------------------------------------------- Request

fn build_body(req: &InferenceRequest, stream: bool) -> Result<Value> {
    let mut messages = Vec::new();
    if let Some(i) = &req.instructions {
        messages.push(json!({"role": "system", "content": i}));
    }
    for m in &req.input {
        let mut texts = Vec::new();
        for p in &m.content {
            match p {
                ContentPart::Text(t) => texts.push(t.text.as_str()),
                ContentPart::ArtifactRef(_) => {
                    return Err(ProviderError::UnsupportedFeature(
                        "artifact_ref must be resolved to text before it reaches the provider"
                            .into(),
                    ));
                }
            }
        }
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Developer => "system",
        };
        messages.push(json!({"role": role, "content": texts.join("\n")}));
    }
    let mut body = json!({"model": req.model, "messages": messages, "stream": stream});
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    if let Some(m) = req.max_output_tokens {
        body["max_tokens"] = json!(m);
    }
    if let Some(tools) = &req.tools {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name,
                        "description": t.description.clone().unwrap_or_default(),
                        "parameters": Value::Object(t.parameters_schema.clone()),
                    }})
                })
                .collect(),
        );
    }
    Ok(body)
}

// ---------------------------------------------------------------- Fehler

fn upstream_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string())
}

pub(crate) fn map_status(
    status: u16,
    body: &str,
    retry_after_ms: Option<i64>,
    provider: &str,
    model: &str,
) -> ProviderError {
    let msg = upstream_message(body);
    let lower = msg.to_ascii_lowercase();
    match status {
        401 | 403 => ProviderError::AuthRejected,
        404 if lower.contains("model") => ProviderError::ModelNotFound {
            provider: provider.into(),
            model: model.into(),
        },
        400 | 413 | 422
            if lower.contains("context")
                && (lower.contains("exceed")
                    || lower.contains("size")
                    || lower.contains("length")) =>
        {
            ProviderError::ContextExceeded
        }
        400 | 413 | 422 => {
            ProviderError::InvalidRequest(format!("HTTP {status}: {}", safe_detail(&msg)))
        }
        408 | 429 | 500 | 502 | 503 | 504 => ProviderError::Unavailable {
            detail: format!("HTTP {status}: {}", safe_detail(&msg)),
            retryable: true,
            retry_after_ms,
        },
        _ => ProviderError::Unavailable {
            detail: format!("HTTP {status}: {}", safe_detail(&msg)),
            retryable: false,
            retry_after_ms: None,
        },
    }
}

fn map_finish(s: &str) -> FinishReason {
    match s {
        "length" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCalls,
        _ => FinishReason::Stop,
    }
}

fn parse_usage(v: &Value) -> Usage {
    let n = |p: &str| v.pointer(p).and_then(Value::as_i64);
    let mut u = Usage::new(
        n("/prompt_tokens").unwrap_or(0).max(0),
        n("/completion_tokens").unwrap_or(0).max(0),
    );
    u.cached_input_tokens = n("/prompt_tokens_details/cached_tokens").filter(|c| *c >= 0);
    // B20: Kosten nur, wenn die Antwort sie nennt (`usage.cost_micros` als ganze Mikro-Einheiten oder
    // `usage.cost` als Dezimalbetrag). Keine Preistabellen, keine Schaetzung; lokale Modelle: None.
    u.cost_micros = n("/cost_micros").filter(|c| *c >= 0).or_else(|| {
        let c = v.pointer("/cost").and_then(Value::as_f64)?;
        (c.is_finite() && (0.0..=9.0e12).contains(&c)).then(|| (c * 1_000_000.0).round() as i64)
    });
    // Waehrung nur in der Vertragsform (drei Grossbuchstaben), sonst bleibt sie unbekannt.
    u.currency = v
        .pointer("/currency")
        .and_then(Value::as_str)
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))
        .filter(|_| u.cost_micros.is_some());
    u
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn reads_cost_and_currency_from_the_response() {
        let v = json!({"prompt_tokens": 10, "completion_tokens": 5, "cost_micros": 1234, "currency": "eur"});
        let u = parse_usage(&v);
        assert_eq!(
            (u.cost_micros, u.currency.as_deref()),
            (Some(1234), Some("EUR"))
        );
        let v =
            json!({"prompt_tokens": 1, "completion_tokens": 1, "cost": 0.0021, "currency": "USD"});
        let u = parse_usage(&v);
        assert_eq!(
            (u.cost_micros, u.currency.as_deref()),
            (Some(2100), Some("USD"))
        );
    }

    #[test]
    fn cost_without_currency_stays_unknown_and_missing_cost_stays_none() {
        let u = parse_usage(&json!({"prompt_tokens": 3, "completion_tokens": 4, "cost": 0.5}));
        assert_eq!((u.cost_micros, u.currency), (Some(500_000), None));
        // Lokales Modell: nur Tokens, keine Kosten erfunden; Waehrung ohne Betrag wird verworfen.
        let u =
            parse_usage(&json!({"prompt_tokens": 3, "completion_tokens": 4, "currency": "EUR"}));
        assert_eq!(
            (u.input_tokens, u.output_tokens, u.cost_micros, u.currency),
            (3, 4, None, None)
        );
        let u = parse_usage(
            &json!({"prompt_tokens": 3, "completion_tokens": 4, "cost": -1.0, "currency": "EUR/h"}),
        );
        assert_eq!((u.cost_micros, u.currency), (None, None));
    }
}

// ---------------------------------------------------------------- Non-Stream

fn parse_completion(v: &Value, req: &InferenceRequest, ctx: &CallCtx) -> Result<InferenceResponse> {
    let choice = v
        .pointer("/choices/0")
        .ok_or_else(|| ProviderError::Protocol("response has no choices".into()))?;
    let mut output = Vec::new();
    if let Some(t) = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        output.push(OutputPart::Text(OutputText {
            text: t.to_string(),
        }));
    }
    if let Some(calls) = choice
        .pointer("/message/tool_calls")
        .and_then(Value::as_array)
    {
        for (i, c) in calls.iter().enumerate() {
            let name = c
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            output.push(OutputPart::ToolCall(OutputToolCall {
                tool_call_id: c
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{i}")),
                name,
                arguments_json: c
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .to_string(),
            }));
        }
    }
    let has_tools = output.iter().any(|p| matches!(p, OutputPart::ToolCall(_)));
    let finish = match choice.get("finish_reason").and_then(Value::as_str) {
        Some(s) => map_finish(s),
        None if has_tools => FinishReason::ToolCalls,
        None => FinishReason::Stop,
    };
    Ok(InferenceResponse {
        schema_version: SCHEMA_VERSION.into(),
        id: new_id("res"),
        ts: now_rfc3339(),
        trace_id: req.trace_id.clone(),
        server_context: None,
        request_id: req.id.clone(),
        provider_id: ctx.provider_id.clone(),
        model: req.model.clone(),
        billing_mode: req.billing_mode,
        output,
        finish_reason: finish,
        usage: v
            .get("usage")
            .map(parse_usage)
            .unwrap_or_else(|| Usage::new(0, 0)),
        policy_decision_id: ctx.policy_decision_id.clone(),
    })
}

// ---------------------------------------------------------------- Stream

struct Emitter {
    request_id: String,
    trace_id: String,
    seq: i64,
}

macro_rules! ev {
    ($self:ident, $variant:ident, $ty:ident { $($f:ident : $v:expr),* $(,)? }) => {{
        let seq = $self.seq;
        $self.seq += 1;
        StreamEvent::$variant($ty {
            schema_version: SCHEMA_VERSION.into(),
            id: new_id("evt"),
            ts: now_rfc3339(),
            trace_id: $self.trace_id.clone(),
            request_id: $self.request_id.clone(),
            seq,
            $($f: $v),*
        })
    }};
}

impl Emitter {
    fn delta(&mut self, text: String) -> StreamEvent {
        ev!(self, Delta, DeltaEvent { text: text })
    }
    fn tool(
        &mut self,
        id: &str,
        name: &str,
        phase: ToolPhase,
        args: Option<String>,
    ) -> StreamEvent {
        ev!(
            self,
            Tool,
            ToolEvent {
                tool_call_id: id.to_string(),
                name: name.to_string(),
                phase: phase,
                arguments_delta: args
            }
        )
    }
    fn usage(&mut self, usage: Usage) -> StreamEvent {
        ev!(self, Usage, UsageEvent { usage: usage })
    }
    fn error(&mut self, e: &ProviderError) -> StreamEvent {
        let error = e.to_contract(Some(&self.trace_id.clone()));
        ev!(self, Error, ErrorEvent { error: error })
    }
    fn done(&mut self, finish_reason: FinishReason) -> StreamEvent {
        ev!(
            self,
            Done,
            DoneEvent {
                finish_reason: finish_reason
            }
        )
    }
}

struct OpenTool {
    id: String,
    name: String,
}

struct Pump {
    em: Emitter,
    tx: mpsc::Sender<StreamEvent>,
    cancel: CancellationToken,
    idle: Duration,
}

enum Frame {
    Data(String),
    Done,
}

/// Naechster vollstaendiger SSE-Frame (Trenner: Leerzeile, LF oder CRLF).
fn take_frame(buf: &mut Vec<u8>) -> Option<Frame> {
    loop {
        let (end, sep) = if let Some(p) = find(buf, b"\r\n\r\n") {
            match find(buf, b"\n\n") {
                Some(q) if q < p => (q, 2),
                _ => (p, 4),
            }
        } else {
            (find(buf, b"\n\n")?, 2)
        };
        let raw: Vec<u8> = buf.drain(..end + sep).collect();
        let text = String::from_utf8_lossy(&raw[..end]).into_owned();
        let data: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(|d| d.strip_prefix(' ').unwrap_or(d))
            .collect();
        if data.is_empty() {
            continue; // Kommentar-/Keepalive-Frame
        }
        let joined = data.join("\n");
        return Some(if joined.trim() == "[DONE]" {
            Frame::Done
        } else {
            Frame::Data(joined)
        });
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

impl Pump {
    async fn send(&self, ev: StreamEvent) -> bool {
        self.tx.send(ev).await.is_ok()
    }

    async fn run(mut self, resp: reqwest::Response) {
        let mut bytes = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut tools: BTreeMap<i64, OpenTool> = BTreeMap::new();
        let mut finish: Option<FinishReason> = None;
        let mut saw_done = false;

        'outer: loop {
            let next = tokio::select! {
                _ = self.cancel.cancelled() => {
                    drop(bytes); // schliesst die Verbindung zum Provider
                    let d = self.em.done(FinishReason::Cancelled);
                    self.send(d).await;
                    return;
                }
                n = tokio::time::timeout(self.idle, bytes.next()) => n,
            };
            match next {
                Err(_) => {
                    return self
                        .fail(ProviderError::Unavailable {
                            detail: "stream idle timeout".into(),
                            retryable: true,
                            retry_after_ms: None,
                        })
                        .await;
                }
                Ok(None) => break,
                Ok(Some(Err(_))) => {
                    return self
                        .fail(ProviderError::Unavailable {
                            detail: "stream interrupted".into(),
                            retryable: true,
                            retry_after_ms: None,
                        })
                        .await;
                }
                Ok(Some(Ok(chunk))) => buf.extend_from_slice(&chunk),
            }
            while let Some(frame) = take_frame(&mut buf) {
                match frame {
                    Frame::Done => {
                        saw_done = true;
                        break 'outer;
                    }
                    Frame::Data(d) => {
                        let Ok(v) = serde_json::from_str::<Value>(&d) else {
                            return self
                                .fail(ProviderError::Protocol("stream chunk is not JSON".into()))
                                .await;
                        };
                        if let Some(err) = v.get("error") {
                            let msg = err
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("provider error");
                            return self
                                .fail(ProviderError::Unavailable {
                                    detail: safe_detail(msg),
                                    retryable: false,
                                    retry_after_ms: None,
                                })
                                .await;
                        }
                        if !self.handle_chunk(&v, &mut tools, &mut finish).await {
                            return; // Empfaenger weg
                        }
                    }
                }
            }
        }

        if !saw_done && finish.is_none() {
            return self
                .fail(ProviderError::Unavailable {
                    detail: "stream ended before completion".into(),
                    retryable: true,
                    retry_after_ms: None,
                })
                .await;
        }
        for t in tools.values() {
            let e = self.em.tool(&t.id, &t.name, ToolPhase::CallCompleted, None);
            if !self.send(e).await {
                return;
            }
        }
        let reason = finish.unwrap_or(if tools.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolCalls
        });
        let d = self.em.done(reason);
        self.send(d).await;
    }

    /// `false`, wenn der Empfaenger verschwunden ist.
    async fn handle_chunk(
        &mut self,
        v: &Value,
        tools: &mut BTreeMap<i64, OpenTool>,
        finish: &mut Option<FinishReason>,
    ) -> bool {
        if let Some(choice) = v.pointer("/choices/0") {
            if let Some(t) = choice
                .pointer("/delta/content")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                let e = self.em.delta(t.to_string());
                if !self.send(e).await {
                    return false;
                }
            }
            if let Some(calls) = choice
                .pointer("/delta/tool_calls")
                .and_then(Value::as_array)
            {
                for c in calls {
                    let idx = c.get("index").and_then(Value::as_i64).unwrap_or(0);
                    let name = c
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty());
                    if let std::collections::btree_map::Entry::Vacant(slot) = tools.entry(idx) {
                        let id = c
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call_{idx}"));
                        let name = name.unwrap_or("unknown").to_string();
                        let e = self.em.tool(&id, &name, ToolPhase::CallStarted, None);
                        slot.insert(OpenTool { id, name });
                        if !self.send(e).await {
                            return false;
                        }
                    }
                    if let Some(a) = c
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .filter(|a| !a.is_empty())
                    {
                        let t = &tools[&idx];
                        let e = self.em.tool(
                            &t.id.clone(),
                            &t.name.clone(),
                            ToolPhase::ArgumentsDelta,
                            Some(a.to_string()),
                        );
                        if !self.send(e).await {
                            return false;
                        }
                    }
                }
            }
            if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
                *finish = Some(map_finish(f));
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let e = self.em.usage(parse_usage(u));
            if !self.send(e).await {
                return false;
            }
        }
        true
    }

    /// Fehler mitten im Stream: `error`, danach genau ein `done` (finish_reason `error`).
    async fn fail(mut self, e: ProviderError) {
        let ev = self.em.error(&e);
        if self.send(ev).await {
            let d = self.em.done(FinishReason::Error);
            self.send(d).await;
        }
    }
}
