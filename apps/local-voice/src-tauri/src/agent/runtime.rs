//! C2 (Goal Lokaler Agent, AK3): die Laufzeit des lokalen Agenten.
//!
//! Eine Anfrage an den llama-server im Schema-Modus: `response_format` json_schema
//! (Grammatik-Sampling), `chat_template_kwargs.enable_thinking = false`, Temperatur 0,
//! fester Startwert, Token-Obergrenze (`max_tokens`). Die Antwort wird vom Aufrufer
//! geparst und fachlich geprueft (`parse`); scheitert das (kein gueltiges JSON, nicht
//! zum Schema passend, abgeschnitten), folgt GENAU EIN Wiederholversuch mit einem
//! Hinweis im Prompt, danach `AgentError::SchemaInvalid` -- die Aufrufer fallen darauf
//! auf `no_action` bzw. eine leere Extraktion zurueck ([`AgentRuntime::choose`],
//! `extract::extract`). Nichts in diesem Modul loest eine Panik aus; jede Antwort des
//! Servers ist Eingabe, nie Vertrauensgrundlage.
//!
//! Warum nicht `llm_client::send_chat_completion*`: dort fehlen `max_tokens`, die Token-
//! Zaehler der Antwort (Provenienz) und die Unterscheidung von "Server weg", "Server
//! belegt" und "Zeit ueberschritten". Der Server selbst wird trotzdem nur ueber den
//! vorhandenen Manager gestartet (`managers::llm::ensure_local`: RAM-Start-Tor,
//! Job-Objekt von `process_guard`, Absturzsperre), und jeder Aufruf wird im
//! Verbrauchs-Ledger gebucht (`usage::record_call`, Zweck `Extract`/`AgentRoute`).
//!
//! # Fehlerfaelle und ihre Absicherung
//!
//! | Fall | Verhalten | Test |
//! |------|-----------|------|
//! | ungueltiges / abgeschnittenes JSON | ein Wiederholversuch mit Hinweis, dann `SchemaInvalid` -> `no_action` | `invalid_json_*`, `truncated_*` |
//! | Server nicht erreichbar / bricht ab | `Unavailable` (nichts passiert, wiederholbar), kein interner Retry | `a_closed_port_*`, `a_dropped_connection_*` |
//! | Server belegt (503/429), Speicher knapp | `Busy` mit Wartezeit (der Schritt wartet, scheitert nicht) | `a_busy_server_*`, `memory_*` |
//! | Server haengt | harte Zeitgrenze je Anfrage -> `Timeout` | `a_hanging_server_*` |
//! | Prompt zu gross fuer den Kontext | `ContextExceeded` (Aufrufer teilt) | `context_overflow_*` |
//! | Modell fehlt / nicht eingerichtet | `NotConfigured` (dauerhaft) | `manager_errors_*` |
//! | Server stirbt mitten im Aufruf | Verbindungsabbruch -> `Unavailable` | `a_dropped_connection_*` |
//! | feindliche Antwort (riesig, Muell) | Parser des Aufrufers; hier nur `max_tokens` und Zeitgrenze | `max_tokens_*` |
//! | zwei Aufrufe gleichzeitig | zustandslos: nichts geteilt ausser dem Server (der serialisiert) | `parallel_asks_*` |
//!
//! Der Echtzeit-Audiopfad ist nicht beteiligt; es gibt hier keinen Audio-Callback.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::schema::{self, ToolChoice, ToolSpec, NO_ACTION};
use crate::managers::usage::{self, Purpose, TokenUsage};
use crate::settings::PostProcessProvider;

/// Erster Versuch plus EIN Wiederholversuch.
pub const MAX_ATTEMPTS: u32 = 2;
/// Standard-Obergrenze der Antwort (Token). Eine Extraktion mit 3 x 12 Eintraegen und
/// Zitaten braucht rund 1 500 Token; die Grenze faengt ein Modell ab, das aus dem Tritt
/// geraet (Endlosliste), statt es bis zum Kontextende laufen zu lassen.
pub const DEFAULT_MAX_TOKENS: u32 = 2_048;
/// Wartezeit je Anfrage (hart). Ein haengender Server blockiert den schweren Platz nie
/// laenger als diese Zeit je Versuch.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// Fester Startwert (mit Temperatur 0 ohnehin nur Absicherung).
pub const SEED: u32 = 42;
/// Wartezeit, wenn der Server belegt ist (503/429).
pub const SERVER_BUSY_RETRY_MS: u64 = 10_000;
/// Wartezeit, wenn der Arbeitsspeicher fuer den Start nicht reicht.
pub const MEMORY_RETRY_MS: u64 = 30_000;
/// Wartezeit nach einem Absturz des Servers (der Manager sperrt den Neustart eine Minute).
pub const CRASH_RETRY_MS: u64 = 60_000;

/// Wohin die Anfrage geht.
#[derive(Clone, Debug)]
pub enum Target {
    /// Der lokale llama-server. Bei jedem Aufruf bestaetigt der Manager, dass er laeuft
    /// (RAM-Start-Tor, Job-Objekt, Absturzsperre) und setzt die Leerlaufuhr zurueck.
    Local { model: String },
    /// Ein laufender Endpunkt mit OpenAI-Schnittstelle (die llama-server-Attrappe der Tests;
    /// spaeter ein entfernter Anbieter, Owner-Entscheidung E4).
    #[allow(dead_code)]
    Endpoint {
        base_url: String,
        model: String,
        context_tokens: u32,
    },
}

impl Target {
    pub fn model(&self) -> &str {
        match self {
            Target::Local { model } | Target::Endpoint { model, .. } => model,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// Obergrenze der Antwort in Token (`max_tokens`).
    pub max_tokens: u32,
    /// Harte Wartezeit je Anfrage.
    pub request_timeout: Duration,
    pub seed: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            seed: SEED,
        }
    }
}

/// Token-Zaehler laut Server, ueber alle Versuche eines Aufrufs summiert.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(other.completion_tokens);
    }
}

/// Warum ein Aufruf kein Ergebnis lieferte. Keine Variante traegt Text aus dem Prompt oder
/// der Antwort (Transkripte duerfen nicht in Protokolle und Fehlermeldungen wandern).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    /// Modell/Laufzeit nicht eingerichtet oder nicht geladen: wird so nicht gelingen.
    NotConfigured(String),
    /// Server nicht erreichbar, nicht gestartet, Verbindung abgebrochen: es wurde nichts
    /// geschrieben, ein neuer Versuch ist sicher.
    Unavailable(String),
    /// Nicht jetzt: Arbeitsspeicher, Server belegt, Neustart gesperrt.
    Busy { retry_after_ms: u64, reason: String },
    /// Keine Antwort innerhalb der Zeitgrenze.
    Timeout { after_ms: u64 },
    /// Der Prompt passt nicht in den Kontext des Servers (der Aufrufer teilt ihn).
    ContextExceeded,
    /// Der Server lehnt die Anfrage ab (4xx ausser Kontext/Belegt): wird so nicht gelingen.
    Rejected { status: u16 },
    /// Auch der Wiederholversuch brachte kein gueltiges, zum Schema passendes JSON.
    SchemaInvalid {
        truncated: bool,
        /// Laenge der letzten Antwort in Zeichen (nie ihr Inhalt).
        raw_len: usize,
        /// Klassifikation des Fehlers (`Syntax-Fehler in Zeile 1, Spalte 5`), ohne Inhalt.
        detail: String,
        /// Verbrauch beider Versuche (die Antworten sind trotzdem gebucht und gezaehlt).
        usage: Usage,
        duration_ms: u64,
    },
}

impl AgentError {
    /// Ein Satz fuer das Laufprotokoll.
    pub fn describe(&self) -> String {
        match self {
            AgentError::NotConfigured(m) | AgentError::Unavailable(m) => m.clone(),
            AgentError::Busy { reason, .. } => reason.clone(),
            AgentError::Timeout { after_ms } => format!(
                "Das Sprachmodell hat nach {} Sekunden nicht geantwortet.",
                after_ms / 1000
            ),
            AgentError::ContextExceeded => {
                "Der Text ist für den Kontext des Sprachmodells zu lang.".to_string()
            }
            AgentError::Rejected { status } => {
                format!("Der Modellserver hat die Anfrage abgelehnt (HTTP {status}).")
            }
            AgentError::SchemaInvalid {
                truncated: true, ..
            } => "Die Antwort des Sprachmodells wurde auch im zweiten Versuch abgeschnitten.".to_string(),
            AgentError::SchemaInvalid { .. } => {
                "Die Antwort des Sprachmodells war auch im zweiten Versuch kein gültiges JSON nach dem Schema.".to_string()
            }
        }
    }

    /// Maschinenlesbare Kurzform (`reason` im Ergebnis, `no_action`-Grund).
    pub fn code(&self) -> &'static str {
        match self {
            AgentError::NotConfigured(_) => "not_configured",
            AgentError::Unavailable(_) => "unavailable",
            AgentError::Busy { .. } => "busy",
            AgentError::Timeout { .. } => "timeout",
            AgentError::ContextExceeded => "context_exceeded",
            AgentError::Rejected { .. } => "rejected",
            AgentError::SchemaInvalid {
                truncated: true, ..
            } => "truncated",
            AgentError::SchemaInvalid { .. } => "schema_invalid",
        }
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

impl std::error::Error for AgentError {}

/// Eine strukturierte Anfrage.
pub struct Request<'a> {
    /// Fuer das Verbrauchs-Ledger.
    pub purpose: Purpose,
    pub system: &'a str,
    pub user: &'a str,
    /// Name des Schemas im `response_format`.
    pub schema_name: &'a str,
    pub schema: &'a Value,
    /// Hinweis im Wiederholversuch nach ungueltigem JSON (der Fehler wird angehaengt).
    pub hint_invalid: &'a str,
    /// Hinweis im Wiederholversuch nach einer abgeschnittenen Antwort.
    pub hint_truncated: &'a str,
}

pub const HINT_INVALID: &str = "Deine vorige Antwort war kein gültiges JSON nach dem Schema. \
Antworte NUR mit dem JSON-Objekt, ohne weiteren Text.";
pub const HINT_TRUNCATED: &str = "Deine vorige Antwort war zu lang und wurde abgeschnitten. \
Antworte kürzer: nur das Wichtigste, kurze Texte, höchstens halb so viele Einträge.";

/// Das gueltige Ergebnis eines Aufrufs mit seinen Messwerten.
#[derive(Clone, Debug)]
pub struct Answer<T> {
    pub value: T,
    /// 1 oder 2.
    pub attempts: u32,
    pub usage: Usage,
    /// Wanduhr ueber alle Versuche.
    pub duration_ms: u64,
}

/// Ein Versuch, der kein Ergebnis lieferte (fuer Hinweis und Fehler).
enum Failure {
    Truncated { raw_len: usize },
    Invalid { raw_len: usize, detail: String },
}

/// Die rohe Antwort des Servers.
#[derive(Clone, Debug, Default, PartialEq)]
struct RawReply {
    content: Option<String>,
    truncated: bool,
    usage: Usage,
}

/// Der Koerper der Anfrage: Schema-gebunden, Denken aus, Temperatur 0, fester Startwert,
/// Token-Obergrenze.
pub fn request_body(
    model: &str,
    system: &str,
    user: &str,
    schema_name: &str,
    schema: &Value,
    config: &AgentConfig,
) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": schema_name, "strict": true, "schema": schema },
        },
        "chat_template_kwargs": { "enable_thinking": false },
        "temperature": 0,
        "seed": config.seed,
        "max_tokens": config.max_tokens,
    })
}

/// Fehlertext des Modellverwalters (`ensure_local`) in einen [`AgentError`].
pub fn from_manager_error(err: &str) -> AgentError {
    use crate::managers::llm;
    use crate::managers::meetings::llm_call::is_memory_error;
    if is_memory_error(err) || err.trim_start().starts_with("memory_low") {
        AgentError::Busy {
            retry_after_ms: MEMORY_RETRY_MS,
            reason: "Wartet auf Arbeitsspeicher für das Sprachmodell.".to_string(),
        }
    } else if llm::is_server_crashed(err) {
        AgentError::Busy {
            retry_after_ms: CRASH_RETRY_MS,
            reason: "Das lokale Sprachmodell ist abgestürzt; der Neustart ist kurz gesperrt."
                .to_string(),
        }
    } else if err.contains("Modell nicht geladen") || err.contains("nicht initialisiert") {
        AgentError::NotConfigured(
            "Das lokale Sprachmodell ist nicht eingerichtet oder nicht geladen.".to_string(),
        )
    } else {
        log::warn!("agent: lokaler Server nicht bereit: {err}");
        AgentError::Unavailable("Das lokale Sprachmodell ließ sich nicht starten.".to_string())
    }
}

fn from_transport(e: &reqwest::Error, after: Duration) -> AgentError {
    if e.is_timeout() {
        AgentError::Timeout {
            after_ms: after.as_millis().min(u128::from(u64::MAX)) as u64,
        }
    } else if e.is_connect() {
        AgentError::Unavailable("Der Modellserver ist nicht erreichbar.".to_string())
    } else {
        AgentError::Unavailable("Die Verbindung zum Modellserver ist abgebrochen.".to_string())
    }
}

/// Antworttext ohne Denk-Reste einer aelteren Vorlage (`<think>...</think>`).
fn strip_think(text: &str) -> String {
    match text.rfind("</think>") {
        Some(end) => text[end + "</think>".len()..].trim().to_string(),
        None => text.to_string(),
    }
}

pub struct AgentRuntime {
    target: Target,
    config: AgentConfig,
}

impl AgentRuntime {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            config: AgentConfig::default(),
        }
    }

    #[allow(dead_code)] // Tests; spaeter Einstellungen (Obergrenze, Zeitgrenze) je Schritt
    pub fn with_config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    pub fn model(&self) -> &str {
        self.target.model()
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Laeuft das Modell auf diesem Rechner? Der Manager-Server ja; ein fester Endpunkt nur,
    /// wenn er auf dem eigenen Rechner lauscht (Provenienz: `local`/`remote`).
    pub fn is_local(&self) -> bool {
        match &self.target {
            Target::Local { .. } => true,
            Target::Endpoint { base_url, .. } => {
                let rest = base_url.split("://").nth(1).unwrap_or(base_url);
                let host = rest.split(['/', ':']).next().unwrap_or_default();
                matches!(host, "127.0.0.1" | "localhost" | "[::1]")
            }
        }
    }

    /// Der Kontext (Token), mit dem der Server das Modell bedient.
    pub async fn context_tokens(&self) -> u32 {
        match &self.target {
            Target::Local { model } => crate::managers::llm::context_for_model(model).await,
            Target::Endpoint { context_tokens, .. } => *context_tokens,
        }
    }

    /// Fuer das Verbrauchs-Ledger.
    fn ledger_provider(&self) -> PostProcessProvider {
        let (id, base_url) = match &self.target {
            Target::Local { .. } => (
                crate::managers::llm::LOCAL_PROVIDER_ID,
                crate::managers::llm::LOCAL_PLACEHOLDER_URL.to_string(),
            ),
            Target::Endpoint { base_url, .. } => ("custom", base_url.clone()),
        };
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_string(),
            base_url,
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: true,
        }
    }

    async fn base_url(&self) -> Result<String, AgentError> {
        match &self.target {
            Target::Local { model } => crate::managers::llm::ensure_local(model)
                .await
                .map_err(|e| from_manager_error(&e)),
            Target::Endpoint { base_url, .. } => Ok(base_url.clone()),
        }
    }

    /// Ein HTTP-Aufruf, ohne Wiederholung. Bucht den Verbrauch (auch bei Fehlern).
    async fn post(&self, req: &Request<'_>, user: &str) -> Result<RawReply, AgentError> {
        let started = Instant::now();
        let result = self.post_inner(req, user).await;
        let usage = result.as_ref().map(|r| r.usage).unwrap_or_default();
        usage::record_call(
            req.purpose,
            &self.ledger_provider(),
            self.model(),
            TokenUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
            },
            started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            result.as_ref().map(|_| ()).map_err(|e| e.code().to_string()),
        )
        .await;
        result
    }

    async fn post_inner(&self, req: &Request<'_>, user: &str) -> Result<RawReply, AgentError> {
        // Regelwerk: ein fester Endpunkt ausserhalb des Rechners wird wie jeder
        // andere Anbieter geprueft, bevor etwas das Geraet verlaesst.
        crate::managers::compliance::check_call(&self.ledger_provider())
            .map_err(AgentError::NotConfigured)?;
        let base = self.base_url().await?;
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let body = request_body(
            self.model(),
            req.system,
            user,
            req.schema_name,
            req.schema,
            &self.config,
        );
        let timeout = self.config.request_timeout;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|_| AgentError::Unavailable("Der HTTP-Client ließ sich nicht erstellen.".into()))?;
        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| from_transport(&e, timeout))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                503 | 429 => AgentError::Busy {
                    retry_after_ms: SERVER_BUSY_RETRY_MS,
                    reason: "Wartet: der Modellserver ist belegt.".to_string(),
                },
                400 | 413
                    if text.contains("exceeds the available context size")
                        || text.contains("exceed_context_size_error") =>
                {
                    AgentError::ContextExceeded
                }
                code if code >= 500 => {
                    AgentError::Unavailable(format!("Der Modellserver meldet einen Fehler (HTTP {code})."))
                }
                code => AgentError::Rejected { status: code },
            });
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| from_transport(&e, timeout))?;
        let choice = &value["choices"][0];
        Ok(RawReply {
            content: choice["message"]["content"].as_str().map(strip_think),
            truncated: choice["finish_reason"].as_str() == Some("length"),
            usage: Usage {
                prompt_tokens: value["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                completion_tokens: value["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            },
        })
    }

    /// Fragt strukturiert und prueft die Antwort mit `parse` (Schema und Fachpruefung des
    /// Aufrufers). Hoechstens zwei Versuche; nur eine unbrauchbare Antwort (kein JSON, nicht
    /// zum Schema, abgeschnitten, vom Parser verworfen) loest den zweiten aus, nie ein
    /// Transportfehler.
    pub async fn ask<T>(
        &self,
        req: &Request<'_>,
        parse: &(dyn Fn(&str) -> Result<T, String> + Sync),
    ) -> Result<Answer<T>, AgentError> {
        let started = Instant::now();
        let mut usage = Usage::default();
        let mut user = req.user.to_string();
        let mut last: Option<Failure> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            let reply = self.post(req, &user).await?;
            usage.add(reply.usage);
            let raw_len = reply.content.as_deref().map_or(0, |c| c.chars().count());
            let failure = match reply.content.as_deref().map(parse) {
                Some(Ok(value)) => {
                    return Ok(Answer {
                        value,
                        attempts: attempt,
                        usage,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }
                _ if reply.truncated => Failure::Truncated { raw_len },
                Some(Err(detail)) => Failure::Invalid { raw_len, detail },
                None => Failure::Invalid {
                    raw_len,
                    detail: "leere Antwort".to_string(),
                },
            };
            if attempt < MAX_ATTEMPTS {
                let hint = match &failure {
                    Failure::Truncated { .. } => req.hint_truncated.to_string(),
                    Failure::Invalid { .. } => req.hint_invalid.to_string(),
                };
                log::warn!(
                    "agent: Antwort unbrauchbar (Versuch {attempt}) -- wiederhole mit Hinweis"
                );
                user = format!("{}\n\n{hint}", req.user);
            }
            last = Some(failure);
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        Err(match last {
            Some(Failure::Truncated { raw_len }) => AgentError::SchemaInvalid {
                truncated: true,
                raw_len,
                detail: "abgeschnitten (Token-Obergrenze oder Kontext)".to_string(),
                usage,
                duration_ms,
            },
            Some(Failure::Invalid { raw_len, detail }) => AgentError::SchemaInvalid {
                truncated: false,
                raw_len,
                detail,
                usage,
                duration_ms,
            },
            None => AgentError::SchemaInvalid {
                truncated: false,
                raw_len: 0,
                detail: "keine Antwort".to_string(),
                usage,
                duration_ms,
            },
        })
    }

    /// Wahl genau eines Werkzeugs aus `tools` (siehe `schema`). Eine unbrauchbare Antwort
    /// -- auch ein Werkzeug ausserhalb der angebotenen Liste -- ergibt nach dem einen
    /// Wiederholversuch `no_action`; nur ein Fehler des Servers selbst ist ein `Err`.
    #[allow(dead_code)] // Verbraucher ist `agent.route` (C3); hier mit Tests vorweggenommen.
    pub async fn choose(
        &self,
        system: &str,
        user: &str,
        tools: &[&ToolSpec],
    ) -> Result<Chosen, AgentError> {
        let schema = schema::choice_schema(tools);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let parse = |raw: &str| -> Result<ToolChoice, String> {
            let choice = schema::parse_choice(raw)?;
            if !names.contains(&choice.tool.as_str()) {
                Err("Werkzeug nicht in der angebotenen Liste".to_string())
            } else if !choice.arguments.is_object() {
                // `parse_choice` toleriert fehlende Argumente, das Schema verlangt sie.
                Err("Argumente fehlen".to_string())
            } else {
                Ok(choice)
            }
        };
        let req = Request {
            purpose: Purpose::AgentRoute,
            system,
            user,
            schema_name: "tool_choice",
            schema: &schema,
            hint_invalid: HINT_INVALID,
            hint_truncated: HINT_TRUNCATED,
        };
        match self.ask(&req, &parse).await {
            Ok(answer) => Ok(Chosen {
                choice: answer.value,
                fallback: None,
                attempts: answer.attempts,
                usage: answer.usage,
                duration_ms: answer.duration_ms,
            }),
            Err(e @ AgentError::SchemaInvalid { .. }) => {
                let (usage, duration_ms) = match &e {
                    AgentError::SchemaInvalid {
                        usage, duration_ms, ..
                    } => (*usage, *duration_ms),
                    _ => (Usage::default(), 0),
                };
                Ok(Chosen {
                    choice: no_action(&e.describe()),
                    fallback: Some(e),
                    attempts: MAX_ATTEMPTS,
                    usage,
                    duration_ms,
                })
            }
            Err(e) => Err(e),
        }
    }
}

/// `no_action` mit Grund: der Rueckfall, wenn keine gueltige Wahl zustande kam.
#[allow(dead_code)] // siehe `choose`
pub fn no_action(reason: &str) -> ToolChoice {
    ToolChoice {
        tool: NO_ACTION.to_string(),
        arguments: json!({ "reason": reason }),
    }
}

/// Ergebnis von [`AgentRuntime::choose`].
#[derive(Clone, Debug)]
#[allow(dead_code)] // siehe `choose`
pub struct Chosen {
    pub choice: ToolChoice,
    /// `Some`: die Wahl ist der Rueckfall `no_action`; hier steht, warum.
    pub fallback: Option<AgentError>,
    pub attempts: u32,
    /// Verbrauch aller Versuche, auch beim Rueckfall.
    pub usage: Usage,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests;
