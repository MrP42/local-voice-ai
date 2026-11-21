//! Typen und Hilfen für Modellaufrufe, **ohne Netz** (lva#604, Teilpaket 3).
//!
//! Hier liegen die Drahttypen der OpenAI-kompatiblen Schnittstelle, der SSE-Dekoder, die
//! Aufruf-Kontexte der CLI (`CLI_*`, `DENK_SENKE`) und kleine reine Helfer. Der gesamte Weg ins
//! Netz und zu den CLIs (Chat, Strom, Bild, Ollama, Modellliste, Prozessstart) ist nach
//! [`crate::llm_versand`] verschoben; die bestehenden Aufrufer gehen bis lva#601 über dessen
//! Übergangsfassade (Untermodul `uebergang` dort). Die Drahttypen sind `pub(crate)` (früher privat),
//! weil der Transport sie jetzt aus einem anderen Modul benutzt; sie gehören nicht zur
//! öffentlichen Schnittstelle der Crate. Eine Sperre gegen eigene Netzwege anderer Module ist das
//! nicht; sie liefern erst `versenden` und der Architekturtest aus lva#601.

use crate::managers::usage::{Purpose, TokenUsage};
use crate::settings::PostProcessProvider;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct ChatMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonSchema {
    pub(crate) name: String,
    pub(crate) strict: bool,
    pub(crate) schema: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponseFormat {
    #[serde(rename = "type")]
    pub(crate) format_type: String,
    pub(crate) json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<ReasoningConfig>,
    /// Nur lokal und nur fuer Denkmodelle: schaltet das Denken ab (P1g).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) chat_template_kwargs: Option<Value>,
    /// Nur lokal und nur fuer KI-Notizen und Follow-up-Mail: deterministisch (siehe
    /// `deterministic_sampling`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) seed: Option<u32>,
}

/// Temperatur und Startwert fuer einen nicht-streamenden Aufruf: die KI-Notizen
/// (Erzeugen, Map/Reduce, Anweisung anwenden) und seit P1k das Protokoll samt
/// Vorlagenwahl laufen lokal deterministisch, wie der Chat seit P4g (A3: auch die
/// KI-Zusammenfuehrung von Transkript-Fassungen). Sonst sampelt das Modell mal ohne, mal mit
/// verworfener Quell-ID, und die Notizen-Eval (Soll ai_sourced >= 0,95)
/// besteht nur zufaellig. Entfernte Anbieter und alle anderen Zwecke bekommen
/// die Felder nicht: manche lehnen sie mit 400 ab (Denkmodelle).
pub(crate) fn deterministic_sampling(local: bool, purpose: Purpose) -> (Option<f32>, Option<u32>) {
    if local
        && matches!(
            purpose,
            Purpose::EnhancedNotes
                | Purpose::Followup
                | Purpose::Minutes
                | Purpose::Document
                | Purpose::TranscriptMerge
                | Purpose::TranscriptTranslation
        )
    {
        (Some(0.0), Some(CHAT_SEED))
    } else {
        (None, None)
    }
}

/// Denkt dieses lokale Modell vor der Antwort, sofern man es nicht abschaltet?
/// Qwen3 und Qwen3.5 (Katalog-Kennungen `llm-qwen3-...`, `llm-qwen3.5-...`)
/// tun das: ein Prompt "2+3" kostet 623 Token, und bei einem langen Prompt ist
/// der Kontext voll, bevor das JSON beginnt (P1e, Befund B3). Andere Modelle
/// (Gemma, Llama, ...) bekommen nichts.
pub fn is_thinking_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("qwen3")
}

/// `chat_template_kwargs` fuer einen nicht-streamenden Aufruf: nur beim
/// lokalen Server und nur fuer Denkmodelle. Entfernte Anbieter lehnen
/// unbekannte Felder teils mit 400 ab.
pub(crate) fn thinking_off_kwargs(local: bool, model: &str) -> Option<Value> {
    (local && is_thinking_model(model)).then(|| serde_json::json!({ "enable_thinking": false }))
}

/// Antworttext eines lokalen Servers ohne Denk-Reste: verirrt sich trotz
/// abgeschaltetem Denken ein `<think>...</think>` in die Antwort (aeltere
/// Chat-Vorlagen, `--reasoning-format none`), ist es sonst der Anfang eines
/// kaputten JSON. Entfernte Anbieter bleiben unveraendert.
pub(crate) fn clean_local_content(local: bool, content: Option<String>) -> Option<String> {
    match content {
        Some(text) if local && text.contains("think>") => Some(
            crate::managers::meetings::chat::citations::strip_think(&text)
                .trim_start()
                .to_string(),
        ),
        other => other,
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionResponse {
    pub(crate) choices: Vec<ChatChoice>,
    /// Token-Zaehler, wie OpenAI, llama-server, Ollama und OpenRouter sie
    /// liefern. Fehlt bei manchen Anbietern -- dann wird mit null gebucht.
    #[serde(default)]
    pub(crate) usage: Option<UsageBlock>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct UsageBlock {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

impl From<Option<UsageBlock>> for TokenUsage {
    fn from(block: Option<UsageBlock>) -> Self {
        let b = block.unwrap_or_default();
        TokenUsage {
            prompt_tokens: b.prompt_tokens,
            completion_tokens: b.completion_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChoice {
    pub(crate) message: ChatMessageResponse,
    /// `length`: der Server hat die Antwort abgebrochen (Kontext oder
    /// Ausgabegrenze voll). Fehlt bei manchen Anbietern.
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatMessageResponse {
    pub(crate) content: Option<String>,
}

/// Build headers for API requests based on provider type
pub(crate) fn build_headers(
    provider: &PostProcessProvider,
    api_key: &str,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/cjpais/Handy"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Handy/1.0 (+https://github.com/cjpais/Handy)"),
    );
    headers.insert("X-Title", HeaderValue::from_static("Handy"));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Zeigt die Adresse auf diesen Rechner (127.0.0.0/8, ::1, localhost)?
pub(crate) fn is_loopback_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url.trim()) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Antwort eines Chat-Aufrufs samt dem Hinweis, ob der Server sie abgeschnitten
/// hat (`finish_reason == "length"`). Ein abgeschnittenes JSON ist nie gueltig;
/// wer es weiss, kann den Prompt verkleinern statt denselben noch einmal zu
/// senden (P1i, Befund B11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub content: Option<String>,
    pub truncated: bool,
}

/// Empfaenger fuer den Denkprozess des Modells (Reasoning-Text), nur fuer die Anzeige: nie
/// gesprochen, nie Teil der Antwort oder des Verlaufs. Gesetzt nur von Aufrufern, die das
/// zulassen (Sprachgespraech ohne Notizbuchdaten, lva#485).
pub type DenkSenke = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

tokio::task_local! {
    static DENK_SENKE: DenkSenke;
}

/// Fuehrt `fut` aus; meldet das Modell dabei Denktext (SSE `reasoning_content`/`reasoning`, Codex
/// `reasoning`-Eintraege), geht er an `senke`. Ohne Senke unveraendert.
pub async fn mit_denksenke<F: std::future::Future>(senke: Option<DenkSenke>, fut: F) -> F::Output {
    match senke {
        Some(s) => DENK_SENKE.scope(s, fut).await,
        None => fut.await,
    }
}

pub(crate) fn denksenke() -> Option<DenkSenke> {
    DENK_SENKE.try_with(|s| s.clone()).ok()
}

tokio::task_local! {
    /// Zeitgrenze fuer CLI-Aufrufe im aktuellen Task (Relay fuer das iPhone:
    /// 60 s statt der ueblichen Obergrenze). Ohne Wert gilt die Voreinstellung.
    pub static CLI_TIMEOUT: std::time::Duration;
}

/// Was ein CLI-Aufruf ueber sich verraten hat: tatsaechliches Modell und Token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliMeta {
    pub model: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

tokio::task_local! {
    /// Ablage fuer [`CliMeta`] im aktuellen Task (Relay: `model_tuple` der Antwort).
    pub static CLI_META: std::sync::Arc<std::sync::Mutex<CliMeta>>;
}

tokio::task_local! {
    /// Abbruchsignal fuer CLI-Aufrufe im aktuellen Task (Relay: Widerruf,
    /// Zeitablauf, Abbruch durch das iPhone). Gesetzt beendet es den Prozessbaum.
    pub static CLI_CANCEL: std::sync::Arc<std::sync::atomic::AtomicBool>;
}

tokio::task_local! {
    /// Obergrenze der Ausgabe-Token fuer CLI-Aufrufe im aktuellen Task (Relay).
    pub static CLI_MAX_TOKENS: u32;
}

tokio::task_local! {
    /// Verlangt eine gueltige Nutzungsangabe der CLI (fehlt sie, ist der
    /// Aufruf ein Fehler statt null Token).
    pub static CLI_REQUIRE_USAGE: bool;
}

tokio::task_local! {
    /// Abo-Vorschau des Buch-Gespraechs (lva#334): empirisches Budget,
    /// gehaertete Isolation und fail-closed-Pruefung der CLI-Antwort. Nur
    /// gesetzt, wenn Schalter und gebundenes Tupel gelten
    /// (`managers::llm::preview`); andere Aufrufer bleiben unveraendert.
    pub static CLI_PREVIEW: crate::managers::llm::preview::PreviewCheck;
}

tokio::task_local! {
    /// Der Vorschau-Schalter live (lva#334): vor, waehrend und nach dem
    /// CLI-Aufruf geprueft; „aus“ beendet den Prozessbaum und verwirft das
    /// Ergebnis. Nur zusammen mit [`CLI_PREVIEW`] gesetzt.
    pub static CLI_PREVIEW_SWITCH: crate::managers::llm::preview::PreviewSwitch;
}

/// Eine Zeile des Gespraechs, wie die CLI sie bekommt (`rolle: text`).
pub fn dialog_line(role: &str, content: &str) -> String {
    format!("{role}: {content}")
}

/// Zusaetze eines Streaming-Aufrufs fuer Aufrufer, die Ausgabegrenze und
/// Vollstaendigkeit erzwingen muessen (Buch-Gespraech). `Default` ist das
/// bisherige Verhalten: nichts gesetzt, nichts verlangt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamLimits {
    /// Obergrenze der Ausgabe-Token: `max_tokens` (OpenAI-kompatibel, auch
    /// llama-server), bei Ollama zusaetzlich `options.num_predict`, bei der
    /// CLI die Ausgabegrenze der CLI.
    pub max_tokens: Option<u32>,
    /// Kontext, mit dem Ollama rechnen soll (`options.num_ctx`); sonst gilt
    /// dessen Voreinstellung, die kleiner sein kann als der gepruefte Prompt.
    pub num_ctx: Option<u32>,
    /// Streng: ein Strom ohne Abschluss (`[DONE]`) oder ueber der Zeichen-
    /// grenze und eine CLI-Antwort ohne gueltige Nutzungsangabe sind Fehler.
    pub strict: bool,
}

/// Ist der Anbieter Ollama (Vorlage oder Standardport)?
pub fn is_ollama(provider: &PostProcessProvider) -> bool {
    provider.id == "ollama" || provider.base_url.contains(":11434")
}

/// Setzt die Grenzen in den Anfragekoerper. Ohne Grenzen bleibt er unveraendert.
pub fn apply_limits(body: &mut Value, ollama: bool, limits: &StreamLimits) {
    if let Some(n) = limits.max_tokens {
        body["max_tokens"] = n.into();
    }
    if !ollama {
        return;
    }
    let mut options = serde_json::Map::new();
    if let Some(n) = limits.max_tokens {
        options.insert("num_predict".into(), n.into());
    }
    if let Some(n) = limits.num_ctx {
        options.insert("num_ctx".into(), n.into());
    }
    if !options.is_empty() {
        body["options"] = Value::Object(options);
    }
}

// ---------------------------------------------------- Ollama, nativ, auf CPU --

/// Ollamas eigener Endpunkt aus einer OpenAI-kompatiblen Basis-URL.
///
/// Der kompatible Pfad ist `…/v1/chat/completions`, der native
/// `…/api/chat`. Nur der native nimmt `options` entgegen — und nur dort
/// lässt sich die GPU abwählen.
///
/// `None`, wenn die Basis nicht nach einem lokalen Dienst aussieht: bei
/// OpenAI oder Groq stellt sich die Frage nicht, und vLLM kennt `/api/chat`
/// gar nicht.
pub fn ollama_native_url(base_url: &str) -> Option<String> {
    let base = base_url.trim_end_matches('/');
    if !(base.contains("localhost") || base.contains("127.0.0.1")) {
        return None;
    }
    let root = base.strip_suffix("/v1").unwrap_or(base);
    Some(format!("{root}/api/chat"))
}

// ------------------------------------------------ Streaming (M4, P4c Chat) --

/// Eine Nachricht fuer `send_chat_completion_stream` (`role` = `system` |
/// `user` | `assistant`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StreamMessage {
    pub role: String,
    pub content: String,
}

impl StreamMessage {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
        }
    }
}

/// Mehr Antworttext nimmt der Stream nicht an: ein Modell in einer
/// Wiederholungsschleife soll weder den Speicher noch die Oberflaeche fluten.
/// Beim Erreichen wird die Verbindung geschlossen (der Server bricht dann ab).
pub const STREAM_MAX_CHARS: usize = 32 * 1024;
/// Laengste Pause zwischen zwei Stream-Stuecken, sobald Text fliesst. Vor dem
/// ersten Token gilt `STREAM_FIRST_TOKEN_TIMEOUT` (Prefill auf CPU dauert).
pub(crate) const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
pub(crate) const STREAM_FIRST_TOKEN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(600);
/// Eine SSE-Zeile laenger als das ist kein Token-Stream mehr.
const SSE_MAX_LINE_BYTES: usize = 1024 * 1024;

/// Fester Startwert fuer den lokalen Chat (mit `temperature` 0 ohnehin nur
/// Absicherung, falls ein Server trotzdem sampelt).
pub const CHAT_SEED: u32 = 42;

/// Anfrage-Body fuer Chat mit oder ohne Streaming. Beim lokalen Server wird
/// das Denken des Modells abgeschaltet (Qwen3/Qwen3.5: `enable_thinking=false`
/// ueber `chat_template_kwargs`) und die Antwort deterministisch gemacht
/// (`temperature` 0, fester `seed`): dieselbe Frage mit denselben Auszuegen
/// gibt dieselbe Antwort, und ein kleines Modell zitiert nicht mal so, mal so
/// (P4g: Eval-Genauigkeit 0,58-0,71 je Lauf). Entfernte Anbieter bekommen
/// diese Felder nicht, weil manche unbekannte Felder oder Werte mit 400
/// ablehnen (Denkmodelle erlauben keine Temperatur).
pub fn stream_request_body(
    model: &str,
    messages: &[StreamMessage],
    stream: bool,
    local: bool,
) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });
    if local {
        body["chat_template_kwargs"] = serde_json::json!({ "enable_thinking": false });
        body["temperature"] = serde_json::json!(0);
        body["seed"] = serde_json::json!(CHAT_SEED);
    }
    body
}

/// Ein Ereignis aus dem SSE-Strom eines OpenAI-kompatiblen Servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    Delta(String),
    /// Denktext des Modells (`delta.reasoning_content` bzw. `delta.reasoning`), nicht Antwort.
    Reasoning(String),
    Usage(TokenUsage),
    Done,
    /// Der Server meldet einen Fehler im Strom (`{"error": ...}`); nur der
    /// Typ/Code, nie der Text der Anfrage.
    Error(String),
}

/// Zerlegt SSE-Bytes in Ereignisse. Puffert bis zum Zeilenende, damit ein
/// UTF-8-Zeichen, das auf zwei Netzwerkpakete verteilt ist, heil bleibt (ein
/// `\n` kommt in UTF-8 nie mitten in einem Zeichen vor).
#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            events.extend(parse_sse_line(line.trim_end_matches(['\r', '\n'])));
        }
        if self.buf.len() > SSE_MAX_LINE_BYTES {
            return Err("SSE-Zeile zu lang".to_string());
        }
        Ok(events)
    }

    /// Rest ohne abschliessenden Zeilenumbruch (Server schliesst direkt nach
    /// dem letzten `data:`).
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let rest = std::mem::take(&mut self.buf);
        let line = String::from_utf8_lossy(&rest);
        parse_sse_line(line.trim_end_matches(['\r', '\n']))
    }
}

/// Ereignisse einer SSE-Zeile. Ein Delta kann Denktext UND Antworttext tragen: dann beide, der
/// Denktext zuerst. Nutzung nur aus einer Zeile ohne Antworttext (wie bisher).
fn parse_sse_line(line: &str) -> Vec<SseEvent> {
    let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
        return Vec::new();
    };
    if payload.is_empty() {
        return Vec::new();
    }
    if payload == "[DONE]" {
        return vec![SseEvent::Done];
    }
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Vec::new();
    };
    if let Some(error) = value.get("error") {
        let code = error
            .get("type")
            .or_else(|| error.get("code"))
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unbekannt".to_string());
        return vec![SseEvent::Error(code)];
    }
    let mut events = Vec::new();
    // Erster NICHT LEERER Alias: ein leeres `reasoning_content` verdeckt kein `reasoning`.
    if let Some(gedanke) = [
        "/choices/0/delta/reasoning_content",
        "/choices/0/delta/reasoning",
    ]
    .iter()
    .filter_map(|p| value.pointer(p).and_then(|c| c.as_str()))
    .find(|g| !g.is_empty())
    {
        events.push(SseEvent::Reasoning(gedanke.to_string()));
    }
    let content = value
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty());
    if let Some(content) = content {
        events.push(SseEvent::Delta(content.to_string()));
    } else if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
        let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        events.push(SseEvent::Usage(TokenUsage {
            prompt_tokens: count("prompt_tokens"),
            completion_tokens: count("completion_tokens"),
        }));
    }
    events
}

// ---------------------------------------------------------------------------
// D3: Bildanfrage (Folien) an einen OpenAI-kompatiblen Server mit Projektor
// ---------------------------------------------------------------------------
//
// Regeln aus dem Spike M7 (`koordination/bild-video/spike/spike-bericht.md`):
// - KEIN `response_format: json_schema` bei Bildanfragen: Gemma 4 E4B lief damit
//   auf echten Bildern bis `max_tokens` in eine Leerzeichen-Schleife (18 von 35
//   Antworten unbrauchbar). Der Aufrufer fragt nach einfachem Zeilenformat und
//   wertet selbst aus.
// - `max_tokens` klein halten (Beschreibung <= 200); ein Bild kostet ~1 000
//   Eingabe-Token, die der Server dank `-b/-ub 2048` in einem Batch sieht.
// - Denken aus (`enable_thinking: false`), wie bei den Textaufrufen.
// Kein Verbrauchs-Ledger: der Aufruf geht nur an den lokalen Server und kostet kein Geld.

/// Hoechste Bilddatei, die gesendet wird (ein 1080p-JPEG hat ~100 kB).
pub const IMAGE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Obergrenze fuer `max_tokens` einer Bildanfrage.
pub const IMAGE_MAX_TOKENS: u32 = 1024;
/// Wartezeit einer Bildanfrage: 2 s auf der GPU, ~100 s auf der CPU.
pub(crate) const IMAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// MIME-Typ einer Bilddatei nach Endung; `None` fuer alles, was kein Bild ist.
pub fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Der Body einer Bildanfrage: eine Nutzernachricht aus Bild (`image_url` mit
/// base64-Data-URL) und Text. Rein, damit der Aufbau testbar ist.
pub fn image_request_body(
    model: &str,
    prompt: &str,
    mime: &str,
    image: &[u8],
    max_tokens: u32,
) -> Value {
    use base64::Engine as _;
    let data_url = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(image)
    );
    serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": data_url } },
                { "type": "text", "text": prompt },
            ],
        }],
        "temperature": 0.1,
        "max_tokens": max_tokens.clamp(1, IMAGE_MAX_TOKENS),
        "chat_template_kwargs": { "enable_thinking": false },
    })
}

/// Antwort einer Bildanfrage: der Text (leer, wenn der Server keinen lieferte) und ob
/// er abgeschnitten wurde (`finish_reason == "length"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReply {
    pub content: String,
    pub truncated: bool,
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn sse_decoder_keeps_split_lines_and_split_utf8_intact() {
        let line = format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "Grüße"}}]})
        );
        let bytes = line.as_bytes();
        // Mitten im "ü" (2 Byte) teilen.
        let cut = line.find('ü').unwrap() + 1;
        let mut dec = SseDecoder::default();
        assert!(dec.push(&bytes[..cut]).unwrap().is_empty());
        let events = dec.push(&bytes[cut..]).unwrap();
        assert_eq!(events, vec![SseEvent::Delta("Grüße".into())]);
    }

    #[test]
    fn sse_reasoning_is_its_own_event_not_answer_text() {
        let line = |v: serde_json::Value| format!("data: {v}");
        assert_eq!(
            parse_sse_line(&line(
                serde_json::json!({"choices": [{"delta": {"reasoning_content": "Ich pruefe"}}]})
            )),
            vec![SseEvent::Reasoning("Ich pruefe".into())]
        );
        assert_eq!(
            parse_sse_line(&line(
                serde_json::json!({"choices": [{"delta": {"content": "", "reasoning": "Plan"}}]})
            )),
            vec![SseEvent::Reasoning("Plan".into())]
        );
        // Gemischtes Delta: beide Inhalte bleiben erhalten, Denktext zuerst.
        assert_eq!(
            parse_sse_line(&line(
                serde_json::json!({"choices": [{"delta": {"content": "Ja", "reasoning": "x"}}]})
            )),
            vec![
                SseEvent::Reasoning("x".into()),
                SseEvent::Delta("Ja".into())
            ]
        );
        // Leerer erster Alias verdeckt den zweiten nicht.
        assert_eq!(
            parse_sse_line(&line(
                serde_json::json!({"choices": [{"delta": {"reasoning_content": "", "reasoning": "Plan"}}]})
            )),
            vec![SseEvent::Reasoning("Plan".into())]
        );
        assert_eq!(
            parse_sse_line(&line(
                serde_json::json!({"choices": [{"delta": {"reasoning_content": ""}}]})
            )),
            vec![]
        );
    }

    #[tokio::test]
    async fn the_thought_sink_reaches_only_its_own_scope() {
        use std::sync::{Arc, Mutex};
        let got = Arc::new(Mutex::new(Vec::<String>::new()));
        let g = got.clone();
        let senke: DenkSenke = Arc::new(move |t: &str| g.lock().unwrap().push(t.to_string()));
        mit_denksenke(Some(senke), async {
            denksenke().expect("Senke im Bereich")("a");
        })
        .await;
        assert!(denksenke().is_none());
        mit_denksenke(None, async { assert!(denksenke().is_none()) }).await;
        assert_eq!(*got.lock().unwrap(), vec!["a".to_string()]);
    }

    #[test]
    fn sse_decoder_reads_usage_done_errors_and_ignores_noise() {
        let mut dec = SseDecoder::default();
        let input = concat!(
            ": keep-alive\r\n",
            "event: message\r\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\r\n\r\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
            "data: {\"error\":{\"type\":\"server_error\",\"message\":\"geheim\"}}\n",
        );
        let events = dec.push(input.as_bytes()).unwrap();
        assert_eq!(
            events,
            vec![
                SseEvent::Delta("A".into()),
                SseEvent::Usage(TokenUsage {
                    prompt_tokens: 12,
                    completion_tokens: 3
                }),
                SseEvent::Done,
                SseEvent::Error("\"server_error\"".into()),
            ]
        );
        // Letzte Zeile ohne Umbruch kommt mit `finish`.
        assert!(dec.push(b"data: [DONE]").unwrap().is_empty());
        assert_eq!(dec.finish(), vec![SseEvent::Done]);
        // Riesige Zeile ohne Umbruch: Fehler statt unbegrenztem Puffer.
        let mut dec = SseDecoder::default();
        assert!(dec.push(&vec![b'x'; SSE_MAX_LINE_BYTES + 1]).is_err());
    }

    #[test]
    fn the_local_body_turns_thinking_off_and_the_remote_body_does_not() {
        let msgs = vec![StreamMessage::new("user", "Frage")];
        let local = stream_request_body("m", &msgs, true, true);
        assert_eq!(local["stream"], true);
        assert_eq!(local["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(local["messages"][0]["role"], "user");
        // Lokal deterministisch: Temperatur 0 und fester Startwert.
        assert_eq!(local["temperature"], 0);
        assert_eq!(local["seed"], CHAT_SEED);
        let remote = stream_request_body("m", &msgs, false, false);
        assert_eq!(remote["stream"], false);
        assert!(remote.get("chat_template_kwargs").is_none());
        // Entfernte Anbieter bekommen keine Sampling-Felder.
        assert!(remote.get("temperature").is_none());
        assert!(remote.get("seed").is_none());
    }

    // ---- P1g: Denkmodus aus fuer Qwen3/Qwen3.5 ---------------------------

    fn request_json(local: bool, model: &str) -> Value {
        request_json_for(local, model, Purpose::PostProcess)
    }

    fn request_json_for(local: bool, model: &str, purpose: Purpose) -> Value {
        let sampling = deterministic_sampling(local, purpose);
        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "Frage".into(),
            }],
            response_format: None,
            reasoning_effort: None,
            reasoning: None,
            chat_template_kwargs: thinking_off_kwargs(local, model),
            temperature: sampling.0,
            seed: sampling.1,
        };
        serde_json::to_value(&request).unwrap()
    }

    #[test]
    fn qwen3_and_qwen35_get_thinking_switched_off_locally() {
        for model in ["llm-qwen3.5-9b-q4", "llm-qwen3-4b-q4", "Qwen3-8B-Q4_K_M"] {
            let body = request_json(true, model);
            assert_eq!(
                body["chat_template_kwargs"]["enable_thinking"], false,
                "{model}"
            );
        }
    }

    #[test]
    fn other_local_models_and_remote_providers_keep_the_request_unchanged() {
        for model in ["llm-gemma4-e4b-q4", "llm-gemma4-12b-q4", "llama-3.1-8b"] {
            let body = request_json(true, model);
            assert!(body.get("chat_template_kwargs").is_none(), "{model}");
        }
        // Auch ein Qwen3 bei einem entfernten Anbieter: kein unbekanntes Feld.
        let remote = request_json(false, "qwen3-235b-a22b");
        assert!(remote.get("chat_template_kwargs").is_none());
        // Sonst nur die bisherigen Felder.
        let body = request_json(true, "llm-gemma4-e4b-q4");
        let mut keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["messages", "model"]);
    }

    #[test]
    fn local_meeting_notes_calls_are_deterministic_and_nothing_else_is() {
        // Lokal + KI-Notizen: Temperatur 0 und fester Startwert (wie P4g im Chat).
        let body = request_json_for(true, "llm-gemma4-e4b-q4", Purpose::EnhancedNotes);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["seed"], CHAT_SEED);
        // P1k: das Protokoll (und mit ihm die Vorlagenwahl) ebenso.
        let body = request_json_for(true, "llm-gemma4-e4b-q4", Purpose::Minutes);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["seed"], CHAT_SEED);
        let remote = request_json_for(false, "gpt-4.1", Purpose::Minutes);
        assert!(remote.get("temperature").is_none() && remote.get("seed").is_none());
        // Auch bei einem Denkmodell zusammen mit abgeschaltetem Denken.
        let body = request_json_for(true, "llm-qwen3.5-9b-q4", Purpose::EnhancedNotes);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        // Entfernte Anbieter: keine Sampling-Felder (Denkmodelle lehnen sie ab).
        let remote = request_json_for(false, "gpt-4.1", Purpose::EnhancedNotes);
        assert!(remote.get("temperature").is_none() && remote.get("seed").is_none());
        // Follow-up-Mail (B13): ebenfalls deterministisch, nur lokal.
        let mail = request_json_for(true, "llm-gemma4-e4b-q4", Purpose::Followup);
        assert_eq!(mail["temperature"].as_f64(), Some(0.0));
        assert_eq!(mail["seed"], CHAT_SEED);
        let mail_remote = request_json_for(false, "gpt-4.1", Purpose::Followup);
        assert!(mail_remote.get("temperature").is_none() && mail_remote.get("seed").is_none());
        // G5: die Uebersetzung einer Transkript-Fassung ist ebenfalls deterministisch (nur lokal).
        let translation =
            request_json_for(true, "llm-gemma4-e4b-q4", Purpose::TranscriptTranslation);
        assert_eq!(translation["temperature"].as_f64(), Some(0.0));
        assert_eq!(translation["seed"], CHAT_SEED);
        let translation_remote = request_json_for(false, "gpt-4.1", Purpose::TranscriptTranslation);
        assert!(
            translation_remote.get("temperature").is_none()
                && translation_remote.get("seed").is_none()
        );
        // Andere lokale Zwecke (Diktat, Tagging, ...) bleiben unveraendert.
        for purpose in [
            Purpose::PostProcess,
            Purpose::Tagging,
            Purpose::Translation,
            Purpose::Summary,
        ] {
            let other = request_json_for(true, "llm-gemma4-e4b-q4", purpose);
            assert!(
                other.get("temperature").is_none() && other.get("seed").is_none(),
                "{purpose:?}"
            );
        }
    }

    #[test]
    fn think_leftovers_are_removed_from_local_answers_only() {
        let raw = Some("<think>\nerst nachdenken\n</think>\n\n{\"a\":1}".to_string());
        assert_eq!(
            clean_local_content(true, raw.clone()).as_deref(),
            Some("{\"a\":1}")
        );
        // Nur ein schliessendes Tag (Vorlage hat das oeffnende schon gesetzt).
        let tail = Some("Gedanken</think>\n{\"a\":1}".to_string());
        assert_eq!(
            clean_local_content(true, tail).as_deref(),
            Some("{\"a\":1}")
        );
        // Nicht geschlossen: nichts Brauchbares dahinter.
        let open = Some("<think>ohne Ende".to_string());
        assert_eq!(clean_local_content(true, open).as_deref(), Some(""));
        // Ohne Tags, ohne Antwort, entfernt: unveraendert.
        assert_eq!(
            clean_local_content(true, Some("{\"a\":1}".into())).as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(clean_local_content(true, None), None);
        assert_eq!(clean_local_content(false, raw.clone()), raw);
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    /// Der Body: Nachricht aus Bild (base64-Data-URL) und Text, KEIN `response_format`
    /// (Spike: Schema-Zwang laeuft bei Bildern in Leerzeichen), kleines `max_tokens`,
    /// Denken aus.
    #[test]
    fn the_request_carries_a_data_url_and_never_a_json_schema() {
        use base64::Engine as _;
        let png: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2];
        let body = image_request_body("llm-gemma4-e4b-q4", "Was steht da?", "image/png", png, 200);
        assert_eq!(body["model"], "llm-gemma4-e4b-q4");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        let content = &body["messages"][0]["content"];
        assert_eq!(content[0]["type"], "image_url");
        let expected = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        assert_eq!(
            content[0]["image_url"]["url"].as_str(),
            Some(expected.as_str())
        );
        assert_eq!(
            content[1],
            serde_json::json!({ "type": "text", "text": "Was steht da?" })
        );
        assert!(
            body.get("response_format").is_none(),
            "kein json_schema bei Bildern"
        );
        assert!(body.get("stream").is_none());
        assert_eq!(body["max_tokens"], 200);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        // Die Obergrenze gilt auch, wenn der Aufrufer mehr (oder nichts) verlangt.
        let big = image_request_body("m", "p", "image/jpeg", b"x", 100_000);
        assert_eq!(big["max_tokens"], IMAGE_MAX_TOKENS);
        let zero = image_request_body("m", "p", "image/jpeg", b"x", 0);
        assert_eq!(zero["max_tokens"], 1);
    }

    #[test]
    fn mime_follows_the_extension_and_rejects_non_images() {
        let mime = |n: &str| image_mime(std::path::Path::new(n));
        assert_eq!(mime("a.jpg"), Some("image/jpeg"));
        assert_eq!(mime("a.JPEG"), Some("image/jpeg"));
        assert_eq!(mime("a.png"), Some("image/png"));
        assert_eq!(mime("a.webp"), Some("image/webp"));
        assert_eq!(mime("a.gif"), None);
        assert_eq!(mime("a.txt"), None);
        assert_eq!(mime("a"), None);
    }
}

#[cfg(test)]
mod pinned_tests {
    use super::*;

    #[test]
    fn loopback_erkennen() {
        assert!(is_loopback_url("http://127.0.0.1:8080/v1/chat/completions"));
        assert!(is_loopback_url("http://localhost:1/x"));
        assert!(is_loopback_url("http://[::1]:1/x"));
        assert!(!is_loopback_url("http://localhost.example.com/x"));
        assert!(!is_loopback_url("https://api.example.com/v1"));
        assert!(!is_loopback_url("kein url"));
    }
}
