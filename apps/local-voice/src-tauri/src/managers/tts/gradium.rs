//! Gradium-Client für „Phonon-Stimme erstellen“ — der EINZIGE Netzzugriff dieser Funktion.
//!
//! Nachbau von `scripts/clone-voice.py` (github.com/gradium-ai/xn-ptts, ohne Python):
//!
//! 1. `POST {API}/voices/` (multipart: `name`, `language`, `start_s`, `audio_file`; Kopf
//!    `x-api-key`) → JSON mit `uid`;
//! 2. `GET {API}/voices/embeddings/{uid}/default` bis `found` (bis zu 60 Mal, alle 2 s);
//! 3. `GET {API}/voices/embeddings/{uid}/default/blob?as_safetensors=1` → die Phonon-Stimmdatei.
//!
//! Danach ist die Stimme offline nutzbar; in Gradiums Konto bleibt sie gespeichert (die
//! Oberfläche sagt das vor dem Senden). `API` = `https://api.gradium.ai/api`, `default` ist der
//! Modellname, den die API bei jedem Phonon-Checkpoint annimmt (Skript: „any model name the API
//! knows will do“). Sprachen: `en`, `fr`, `de`, `es`, `pt`.
//!
//! ## Nur aufgerufen, wenn der Nutzer es will
//!
//! Dieses Modul wird ausschließlich vom Befehl `phonon_voice_clone` (commands/phonon_voices.rs)
//! benutzt, nach Hinweis und Einwilligung; nie beim Start, nie in einer Gesundheitsprüfung, nie
//! aus CLI oder Agentenbrücke (ein Test prüft die Verwendungsstellen).
//!
//! ## Schlüssel und Daten
//!
//! * Der Schlüssel geht nur an den EINEN Ursprung `https://api.gradium.ai` (`Origin`, wie beim
//!   Hugging-Face-Token in `managers/model/download/auth.rs`): Weiterleitungen folgt dieser
//!   Client selbst, bei jedem Schritt wird neu entschieden. Ein anderer Ursprung (z. B. ein CDN
//!   für den Blob) bekommt den Schlüssel nie; ein `POST` wird nie weitergeleitet (die Aufnahme
//!   geht an keinen anderen Ort).
//! * Der Schlüssel steht in keiner Meldung, keinem `Debug` und keinem Log: `Secret` zeigt nur
//!   `***`, der Kopf ist als sensibel markiert, Fremdtext aus Antworten wird vor der Ausgabe
//!   um den Schlüssel bereinigt, Meldungen tragen keine Adresse.
//! * Die Kennung (`uid`) kommt vom Server und wird in Adressen gesetzt: nur `[A-Za-z0-9_-]`,
//!   höchstens 64 Zeichen.
//! * Antworten sind in der Größe begrenzt (JSON 64 KiB, Blob 2 MiB).
//! * Abbruch: ein gesetztes [`CancelFlag`] beendet laufende Anfragen und das Warten binnen
//!   ~50 ms (die Verbindung wird geschlossen).

use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde_json::Value;
use specta::Type;

use crate::managers::model::download::{Origin, Secret};

pub const API_BASE: &str = "https://api.gradium.ai/api";
/// Modellname in den Adressen der Einbettung (siehe Moduldoku).
pub const MODEL: &str = "default";
/// Sprachen, die `clone-voice.py` erlaubt.
pub const LANGUAGES: [&str; 5] = ["en", "fr", "de", "es", "pt"];
/// Obergrenzen der Antworten.
pub const MAX_BLOB_BYTES: usize = 2 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_REDIRECTS: usize = 3;
const MAX_DETAIL_CHARS: usize = 200;

/// Abbruchmarke; von der Oberfläche über `phonon_voice_clone_cancel` gesetzt.
pub type CancelFlag = Arc<AtomicBool>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GradiumErrorKind {
    /// 401/403: Schlüssel fehlt, ist falsch oder widerrufen.
    Unauthorized,
    /// 402/429: Kontingent erschöpft oder zu viele Anfragen.
    QuotaOrRate,
    /// Anderer 4xx beim Anlegen: Aufnahme oder Angaben abgelehnt.
    BadAudio,
    /// 5xx.
    Server,
    /// Keine Antwort in der Frist, oder die Stimme wurde nicht rechtzeitig fertig.
    Timeout,
    /// Nicht erreichbar, Verbindung abgerissen.
    Network,
    /// Antwort nicht wie beschrieben (kein JSON, keine Kennung, zu groß, Weiterleitung).
    BadResponse,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct GradiumError {
    pub kind: GradiumErrorKind,
    /// Deutsche Meldung für den Nutzer: ohne Schlüssel, ohne Adresse.
    pub message: String,
}

impl fmt::Display for GradiumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GradiumError {}

/// Stufen des Ablaufs (für die Anzeige).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Uploading,
    Waiting,
    Downloading,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Wurzel der API (ohne Schrägstrich am Ende).
    pub base: Url,
    /// Der Ursprung, an den der Schlüssel gehen darf.
    pub origin: Origin,
    /// Frist je Anfrage.
    pub request_timeout: Duration,
    pub poll_interval: Duration,
    pub max_polls: u32,
    pub https_only: bool,
}

impl Config {
    /// Wie `clone-voice.py`: 60 s je Anfrage, bis zu 60 Abfragen im Abstand von 2 s.
    pub fn production() -> Self {
        Self {
            base: Url::parse(API_BASE).expect("fester Wert"),
            origin: Origin::gradium(),
            request_timeout: Duration::from_secs(60),
            poll_interval: Duration::from_secs(2),
            max_polls: 60,
            https_only: true,
        }
    }

    /// Fake-Server auf Loopback (nur Tests).
    #[cfg(test)]
    pub(crate) fn for_fake(fake: &crate::managers::tts::providers::test_http::FakeServer) -> Self {
        Self {
            base: Url::parse(&format!("{}/api", fake.root)).expect("Loopback-Adresse"),
            origin: Origin::plain_http("127.0.0.1", fake.port),
            request_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(10),
            max_polls: 20,
            https_only: false,
        }
    }
}

/// Was an Gradium geht: Name, Sprache und das ausgeschnittene Stück als WAV.
pub struct CloneJob {
    pub name: String,
    pub language: String,
    pub wav: Vec<u8>,
}

impl fmt::Debug for CloneJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloneJob")
            .field("name", &self.name)
            .field("language", &self.language)
            .field("wav_len", &self.wav.len())
            .finish()
    }
}

/// Die fertige Stimme.
pub struct Cloned {
    /// Kennung in Gradiums Konto.
    pub voice_id: String,
    /// Die Phonon-Stimmdatei (noch ungeprüft: `phonon_voices::validate_voice_bytes`).
    pub safetensors: Vec<u8>,
}

impl fmt::Debug for Cloned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cloned")
            .field("voice_id", &self.voice_id)
            .field("bytes", &self.safetensors.len())
            .finish()
    }
}

pub struct Client {
    http: reqwest::Client,
    cfg: Config,
    key: Secret,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Client(***)")
    }
}

#[derive(Clone, Copy)]
enum Step {
    Create,
    Poll,
    Blob,
}

/// `uid` nur aus unschädlichen Zeichen: sie kommt vom Server und landet in Adressen.
pub fn valid_voice_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Multipart-Körper von Hand (kein zusätzliches reqwest-Feature nötig): Felder in der
/// Reihenfolge von `requests` (erst Daten, dann Datei).
pub(crate) fn build_multipart(
    boundary: &str,
    fields: &[(&str, &str)],
    file_name: &str,
    content_type: &str,
    bytes: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 512);
    for (name, value) in fields {
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"audio_file\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(bytes);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn cancelled() -> GradiumError {
    GradiumError {
        kind: GradiumErrorKind::Cancelled,
        message: "Abgebrochen.".to_string(),
    }
}

/// `fut` laufen lassen, bis es fertig ist oder das Flag gesetzt wird (Prüfung alle 50 ms). Das
/// Verwerfen des Futures schließt die Verbindung.
async fn cancellable<T>(
    cancel: &CancelFlag,
    fut: impl Future<Output = T>,
) -> Result<T, GradiumError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled());
    }
    tokio::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return Ok(out),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancel.load(Ordering::SeqCst) {
                    return Err(cancelled());
                }
            }
        }
    }
}

/// Nächstes Ziel einer Weiterleitung: https, oder http nur aus http (Tests auf Loopback); keine
/// Zugangsdaten in der Adresse.
fn redirect_target(current: &Url, location: &str) -> Option<Url> {
    let next = current.join(location).ok()?;
    let scheme_ok =
        next.scheme() == "https" || (next.scheme() == "http" && current.scheme() == "http");
    (scheme_ok && next.username().is_empty() && next.password().is_none()).then_some(next)
}

impl Client {
    pub fn new(cfg: Config, key: Secret) -> Result<Self, GradiumError> {
        let http = reqwest::Client::builder()
            // Weiterleitungen folgt `send` selbst: bei jedem Schritt wird neu entschieden, ob der
            // Schlüssel mitgeht.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(cfg.request_timeout)
            .connect_timeout(Duration::from_secs(15).min(cfg.request_timeout))
            .https_only(cfg.https_only)
            .user_agent(concat!("local-voice-ai/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| GradiumError {
                kind: GradiumErrorKind::Network,
                message: "Der Netzwerkzugriff lässt sich nicht vorbereiten.".to_string(),
            })?;
        Ok(Self { http, cfg, key })
    }

    /// Fehler mit bereinigter Meldung: der Schlüssel kann in keiner Meldung stehen.
    fn fail(&self, kind: GradiumErrorKind, message: impl AsRef<str>) -> GradiumError {
        GradiumError {
            kind,
            message: self.key.scrub(message.as_ref()),
        }
    }

    fn url(&self, path: &str) -> Result<Url, GradiumError> {
        Url::parse(&format!(
            "{}/{path}",
            self.cfg.base.as_str().trim_end_matches('/')
        ))
        .map_err(|_| self.fail(GradiumErrorKind::BadResponse, "Ungültige Adresse."))
    }

    fn net_err(&self, e: reqwest::Error) -> GradiumError {
        // Ohne Adresse: eine Adresse in einer Meldung ist selbst ein Datenschutzproblem.
        let e = e.without_url();
        if e.is_timeout() {
            self.fail(
                GradiumErrorKind::Timeout,
                format!(
                    "Gradium hat nicht innerhalb von {} s geantwortet (Zeitüberschreitung).",
                    self.cfg.request_timeout.as_secs().max(1)
                ),
            )
        } else if e.is_connect() {
            self.fail(
                GradiumErrorKind::Network,
                "api.gradium.ai ist nicht erreichbar. Internetverbindung, Proxy und Firewall prüfen.",
            )
        } else {
            self.fail(
                GradiumErrorKind::Network,
                "Die Verbindung zu Gradium wurde unterbrochen. Bitte erneut versuchen.",
            )
        }
    }

    async fn read_capped(
        &self,
        mut resp: reqwest::Response,
        max: usize,
        cancel: &CancelFlag,
    ) -> Result<Vec<u8>, GradiumError> {
        let too_big = || {
            self.fail(
                GradiumErrorKind::BadResponse,
                "Die Antwort von Gradium ist ungewöhnlich groß und wird verworfen.",
            )
        };
        if resp.content_length().is_some_and(|n| n > max as u64) {
            return Err(too_big());
        }
        let mut out = Vec::new();
        while let Some(chunk) = cancellable(cancel, resp.chunk())
            .await?
            .map_err(|e| self.net_err(e))?
        {
            if out.len() + chunk.len() > max {
                return Err(too_big());
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// Eine Anfrage; Weiterleitungen nur für `GET` und der Schlüssel nur an den eigenen Ursprung.
    async fn send(
        &self,
        method: Method,
        first: Url,
        body: Option<(String, Vec<u8>)>,
        max_body: usize,
        cancel: &CancelFlag,
    ) -> Result<(StatusCode, Vec<u8>), GradiumError> {
        let key = self.key.header_value().ok_or_else(|| {
            self.fail(
                GradiumErrorKind::Unauthorized,
                "Der API-Schlüssel enthält ungültige Zeichen.",
            )
        })?;
        let mut current = first;
        for _ in 0..=MAX_REDIRECTS {
            let mut req = self.http.request(method.clone(), current.clone());
            if Origin::of(&current).as_ref() == Some(&self.cfg.origin) {
                req = req.header("x-api-key", key.clone());
            }
            if let Some((content_type, bytes)) = &body {
                req = req
                    .header(CONTENT_TYPE, content_type.as_str())
                    .body(bytes.clone());
            }
            let resp = cancellable(cancel, req.send())
                .await?
                .map_err(|e| self.net_err(e))?;
            let status = resp.status();
            if status.is_redirection() {
                if method != Method::GET {
                    return Err(self.fail(
                        GradiumErrorKind::BadResponse,
                        "Gradium hat unerwartet weitergeleitet; aus Sicherheitsgründen wurde nichts gesendet.",
                    ));
                }
                let next = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|loc| redirect_target(&current, loc));
                let Some(next) = next else {
                    return Err(self.fail(
                        GradiumErrorKind::BadResponse,
                        "Gradium hat auf eine unzulässige Adresse weitergeleitet.",
                    ));
                };
                current = next;
                continue;
            }
            let bytes = self.read_capped(resp, max_body, cancel).await?;
            return Ok((status, bytes));
        }
        Err(self.fail(GradiumErrorKind::BadResponse, "Zu viele Weiterleitungen."))
    }

    /// Hinweistext des Servers: bereinigt (Schlüssel, Steuerzeichen), gekürzt, ohne HTML.
    fn detail(&self, body: &[u8]) -> Option<String> {
        let head = &body[..body.len().min(4096)];
        let raw = match serde_json::from_slice::<Value>(head) {
            Ok(v) => detail_of(&v)?,
            Err(_) => {
                let text = String::from_utf8_lossy(head).trim().to_string();
                if text.starts_with('<') {
                    return None;
                }
                text
            }
        };
        let cleaned: String = self
            .key
            .scrub(&raw)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() {
            return None;
        }
        let mut short: String = cleaned.chars().take(MAX_DETAIL_CHARS).collect();
        if cleaned.chars().count() > MAX_DETAIL_CHARS {
            short.push('…');
        }
        Some(short)
    }

    fn http_error(&self, status: StatusCode, body: &[u8], step: Step) -> GradiumError {
        let code = status.as_u16();
        let detail = self
            .detail(body)
            .map(|d| format!(": {d}"))
            .unwrap_or_default();
        match code {
            401 | 403 => self.fail(
                GradiumErrorKind::Unauthorized,
                format!("Gradium lehnt den API-Schlüssel ab (HTTP {code}). Schlüssel in den Phonon-Einstellungen prüfen oder neu eintragen."),
            ),
            402 | 429 => self.fail(
                GradiumErrorKind::QuotaOrRate,
                format!("Gradium: Kontingent erschöpft oder zu viele Anfragen (HTTP {code}){detail}. Später erneut versuchen oder das Konto prüfen."),
            ),
            500..=599 => self.fail(
                GradiumErrorKind::Server,
                format!("Der Gradium-Server meldet einen Fehler (HTTP {code}). Später erneut versuchen."),
            ),
            400..=499 if matches!(step, Step::Create) => self.fail(
                GradiumErrorKind::BadAudio,
                format!("Gradium hat die Aufnahme abgelehnt (HTTP {code}){detail}. Eine klare Aufnahme mit nur einer sprechenden Person, ohne Musik und Rauschen, wählen."),
            ),
            _ => self.fail(
                GradiumErrorKind::BadResponse,
                format!("Unerwartete Antwort von Gradium (HTTP {code}){detail}."),
            ),
        }
    }

    fn json(&self, bytes: &[u8]) -> Result<Value, GradiumError> {
        serde_json::from_slice(bytes).map_err(|_| {
            self.fail(
                GradiumErrorKind::BadResponse,
                "Gradium hat nicht wie erwartet geantwortet (kein JSON).",
            )
        })
    }

    /// Stimme anlegen, auf ihre Einbettung warten und die Phonon-Stimmdatei laden.
    pub async fn clone_voice(
        &self,
        job: &CloneJob,
        cancel: &CancelFlag,
        on_phase: &(dyn Fn(Phase) + Send + Sync),
    ) -> Result<Cloned, GradiumError> {
        if !LANGUAGES.contains(&job.language.as_str()) {
            return Err(self.fail(
                GradiumErrorKind::BadAudio,
                "Diese Sprache unterstützt Gradium nicht.",
            ));
        }
        if job.name.trim().is_empty() || job.name.chars().any(char::is_control) {
            return Err(self.fail(
                GradiumErrorKind::BadAudio,
                "Der Name der Stimme ist ungültig.",
            ));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }

        // 1. Anlegen. `start_s` ist 0: das Fenster ist schon ausgeschnitten.
        on_phase(Phase::Uploading);
        let fields = [
            ("name", job.name.as_str()),
            ("language", job.language.as_str()),
            ("start_s", "0.0"),
        ];
        let mut boundary;
        let mut body;
        loop {
            boundary = format!("lva{:032x}", rand::thread_rng().gen::<u128>());
            body = build_multipart(&boundary, &fields, "voice.wav", "audio/wav", &job.wav);
            // Der Trenner darf nirgends in der Nutzlast vorkommen (bei 128 Zufallsbit nie).
            if !find(&job.wav, boundary.as_bytes()) {
                break;
            }
        }
        let (status, bytes) = self
            .send(
                Method::POST,
                self.url("voices/")?,
                Some((format!("multipart/form-data; boundary={boundary}"), body)),
                MAX_JSON_BYTES,
                cancel,
            )
            .await?;
        if !status.is_success() {
            return Err(self.http_error(status, &bytes, Step::Create));
        }
        let created = self.json(&bytes)?;
        let voice_id = match created.get("uid").and_then(Value::as_str) {
            Some(id) if valid_voice_id(id) => id.to_string(),
            Some(id) if !id.is_empty() => {
                return Err(self.fail(
                    GradiumErrorKind::BadResponse,
                    "Gradium hat eine unbrauchbare Stimmenkennung geliefert.",
                ))
            }
            _ => {
                let why = created
                    .get("error")
                    .and_then(detail_of)
                    .and_then(|e| self.detail(e.as_bytes()))
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default();
                return Err(self.fail(
                    GradiumErrorKind::BadResponse,
                    format!("Gradium hat keine Stimme erstellt{why}."),
                ));
            }
        };

        // 2. Warten, bis die Einbettung da ist.
        on_phase(Phase::Waiting);
        let poll_url = self.url(&format!("voices/embeddings/{voice_id}/{MODEL}"))?;
        let mut found = false;
        for attempt in 0..self.cfg.max_polls {
            let (status, bytes) = self
                .send(Method::GET, poll_url.clone(), None, MAX_JSON_BYTES, cancel)
                .await?;
            if !status.is_success() {
                return Err(self.http_error(status, &bytes, Step::Poll));
            }
            if self.json(&bytes)?.get("found").and_then(Value::as_bool) == Some(true) {
                found = true;
                break;
            }
            if attempt + 1 < self.cfg.max_polls {
                cancellable(cancel, tokio::time::sleep(self.cfg.poll_interval)).await?;
            }
        }
        if !found {
            let secs =
                (self.cfg.poll_interval.as_secs_f32() * self.cfg.max_polls as f32).round() as u32;
            return Err(self.fail(
                GradiumErrorKind::Timeout,
                format!(
                    "Die Stimme „{voice_id}“ ist bei Gradium nach {secs} s noch nicht fertig. Sie ist in Ihrem Gradium-Konto gespeichert; bitte später erneut versuchen."
                ),
            ));
        }

        // 3. Die Stimmdatei laden.
        on_phase(Phase::Downloading);
        let mut blob_url = self.url(&format!("voices/embeddings/{voice_id}/{MODEL}/blob"))?;
        blob_url
            .query_pairs_mut()
            .append_pair("as_safetensors", "1");
        let (status, bytes) = self
            .send(Method::GET, blob_url, None, MAX_BLOB_BYTES, cancel)
            .await?;
        if !status.is_success() {
            return Err(self.http_error(status, &bytes, Step::Blob));
        }
        Ok(Cloned {
            voice_id,
            safetensors: bytes,
        })
    }
}

/// Der Hinweistext eines Fehlers aus der Antwort: `detail` (Text, Liste von `msg` oder Objekt),
/// sonst `message`/`error`.
fn detail_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(detail_of).collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        Value::Object(map) => ["detail", "msg", "message", "error"]
            .iter()
            .find_map(|k| map.get(*k).and_then(detail_of)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
