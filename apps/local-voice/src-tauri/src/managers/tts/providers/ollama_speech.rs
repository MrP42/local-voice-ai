//! Sprachausgabe über Ollama (Paket P2, Goal tts-provider).
//!
//! Ollama kann laut Release-Notes v0.40.11 (24.10.2026) Sprache erzeugen —
//! „Ollama can now generate speech on macOS“ — und liefert sie über den
//! OpenAI-kompatiblen Endpunkt `POST /v1/audio/speech` (provider-research.md,
//! B). Der Adapter baut auf [`OpenAiSpeechProvider`] auf (Anfrage, Streaming,
//! Dekoder, Fehlerklassen, Datenschutz sind DORT, nicht kopiert) und fügt
//! hinzu, was nur Ollama betrifft: Versions-/Modell-/Plattformprüfung.
//!
//! ## Nur lesende Aufrufe
//!
//! Der Adapter ändert an Ollama nichts: kein `ollama pull`, kein Anlegen oder
//! Löschen von Modellen, keine Eingriffe in LLM-Einstellungen. Er ruft nur
//! `GET /api/version`, `GET /api/tags`, `POST /api/show` (liest die Modellangaben,
//! verändert nichts) und für die eigentliche Sprachausgabe
//! `POST /v1/audio/speech`. Einzige Ausnahme mit Nebenwirkung: die
//! Probe-Synthese bei `probe_speech = true` — sie lädt das Sprachmodell in
//! Ollama (das kann ein geladenes LLM aus dem Speicher drängen). Darum ist sie
//! standardmäßig aus, braucht die ausdrückliche Einstellung und wird für
//! 5 Minuten zwischengespeichert, damit ein oft aktualisierter Status nicht
//! jedes Mal rechnet.
//!
//! ## Health in Stufen (jede mit eigenem Grund)
//!
//! 1. Dienst erreichbar? `GET /api/version` (Frist 2 s).
//! 2. Version ≥ [`MIN_OLLAMA_VERSION`] (0.40.11, siehe dort)?
//! 3. Modell vorhanden? `GET /api/tags` (Name exakt oder `name:latest`).
//! 4. Plattform: Upstream nennt nur macOS. Anderswo ist der Anbieter „nicht
//!    verfügbar“ mit Grund — es sei denn `probe_speech = true`: dann
//!    entscheidet eine Mini-Probe gegen `/v1/audio/speech` und nur echtes Audio
//!    zählt als verfügbar.
//!
//! Ollama ist laut Briefing kein Anbieter mit festen Modellnamen: nur der
//! Standard `phonon` ist hinterlegt, alles andere (Tag, Stimme) kommt aus der
//! Konfiguration oder aus Ollama selbst (`/api/show`).
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Gleichzeitige Aufträge | wie `openai_speech`; der Probe-Zwischenspeicher liegt hinter einem `Mutex`, nie über ein `await` gehalten |
//! | Abbruch mitten im Vorgang | `guarded` im Unterbau (Verbindung wird geschlossen); Vertragstest mit hängendem Server |
//! | Voller Speicher | Antwortdeckel im Unterbau; die Probe lädt das Modell nur bei ausdrücklicher Freigabe und einmal je 5 Minuten |
//! | Fehlender Dienst/Version/Modell/Plattform | Health in Stufen mit Grund und Hinweis; `synthesize` meldet die Plattformsperre ohne Anfrage als `Unavailable`; ein 404 des Dienstes heißt „Modell fehlt oder Ollama zu alt“ (`Unavailable`) |
//! | Hänger des Dienstes | Fristen 2 s (Version), 3 s (Modelle, Stimmen), 10 s (Probe), `opts.timeout` (Synthese) |
//! | Datenabfluss | Loopback-Pflicht (`allow_remote`), keine Umleitungen, kein Proxy für Loopback (Unterbau) |

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use tokio::sync::mpsc;

use super::contract::{
    language_matches, AudioChunk, AudioClip, CancelToken, Device, LicenseInfo, ProviderDescriptor,
    ProviderHealth, SynthesisOptions, TtsError, TtsProvider, VoiceDescriptor,
};
use super::openai_speech::{parse_voices, OpenAiSpeechConfig, OpenAiSpeechProvider};

pub const OLLAMA_ID: &str = "ollama-speech";
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
/// Einziger hinterlegter Modellname (überschreibbar).
pub const DEFAULT_MODEL: &str = "phonon";
/// Kleinste Ollama-Version mit Sprachausgabe.
///
/// Quelle: Release-Notes v0.40.11 vom 24.10.2026, „Ollama can now generate
/// speech on macOS“ (https://github.com/ollama/ollama/releases/tag/v0.40.11,
/// abgerufen 26.10.2026). Die Doku nennt widersprüchlich v0.40.8 (OpenAI-
/// Kompatibilität) und v0.40.9 (Speech); v0.40.11 ist die einzige durch die
/// Release-Notes belegte Version (`docs/tts/provider-research.md`, B).
pub const MIN_OLLAMA_VERSION: (u32, u32, u32) = (0, 40, 11);

const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TTL: Duration = Duration::from_secs(300);
/// Wenige Zeichen genügen: es geht nur darum, ob überhaupt Audio entsteht.
const PROBE_TEXT: &str = "Hallo.";
const SOURCE_URL: &str = "https://ollama.com/library/phonon";
const VERIFIED_ON: &str = "2026-10-26";

// ---- Konfiguration ---------------------------------------------------------------

/// Einstellungen des Ollama-Sprachanbieters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct OllamaSpeechConfig {
    /// Adresse des Ollama-Dienstes (ohne `/v1`; ein angehängtes `/v1` wird toleriert).
    pub base_url: String,
    /// Modell in Ollama (Standard `phonon`; z. B. `phonon:de`).
    pub model: String,
    /// Stimme; `None` = Vorgabe des Modells.
    pub voice: Option<String>,
    /// Statt der Plattformsperre eine Mini-Probe gegen den Dienst machen
    /// (lädt das Sprachmodell in Ollama; Standard: aus).
    pub probe_speech: bool,
    /// Dienst außerhalb von Loopback erlauben (Standard: nein).
    pub allow_remote: bool,
    /// Sprachen des Modells (`und`, wenn leer: Ollama nennt sie nicht).
    pub languages: Vec<String>,
}

impl Default for OllamaSpeechConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            voice: None,
            probe_speech: false,
            allow_remote: false,
            languages: Vec::new(),
        }
    }
}

/// Wurzel des Dienstes ohne abschließenden Schrägstrich und ohne `/v1`.
fn ollama_root(base_url: &str) -> String {
    let t = base_url.trim().trim_end_matches('/');
    t.strip_suffix("/v1")
        .unwrap_or(t)
        .trim_end_matches('/')
        .to_string()
}

impl OllamaSpeechConfig {
    /// Die Konfiguration des Unterbaus (gleiche Prüfungen, gleiche Meldungen).
    fn inner_config(&self) -> OpenAiSpeechConfig {
        OpenAiSpeechConfig {
            id: OLLAMA_ID.to_string(),
            label: "Ollama Sprachausgabe".to_string(),
            base_url: format!("{}/v1", ollama_root(&self.base_url)),
            model: self.model.trim().to_string(),
            voice: self.voice.clone(),
            // „Audio is 16-bit, mono, 24 kHz.“ (Ollama-Speech-Doku, 26.10.2026)
            sample_rate: 24_000,
            languages: if self.languages.is_empty() {
                vec!["und".to_string()]
            } else {
                self.languages.clone()
            },
            allow_remote: self.allow_remote,
            api_key_env: None,
            license: None,
        }
    }

    /// Prüft die Einstellungen (Adresse, Modell, Loopback-Pflicht).
    pub fn validate(&self) -> Result<(), TtsError> {
        self.inner_config().validate()
    }
}

// ---- Version ---------------------------------------------------------------------

/// Eine Ollama-Version. `release = false` für Vorabversionen (`-rc1`): sie
/// sind kleiner als die gleichnamige Veröffentlichung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u32,
    minor: u32,
    patch: u32,
    release: bool,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.release {
            f.write_str("-pre")?;
        }
        Ok(())
    }
}

const MIN_VERSION: Version = Version {
    major: MIN_OLLAMA_VERSION.0,
    minor: MIN_OLLAMA_VERSION.1,
    patch: MIN_OLLAMA_VERSION.2,
    release: true,
};

/// Version tolerant lesen: `0.40.11`, `v0.40.11`, `0.40` (Patch 0),
/// `0.40.11-rc1` (Vorabversion), `0.40.11+build`; sonst `None`.
fn parse_version(raw: &str) -> Option<Version> {
    let s = raw.trim().trim_start_matches(['v', 'V']);
    let core_end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (core, rest) = s.split_at(core_end);
    let parts: Vec<&str> = core.split('.').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let num = |i: usize| -> Option<u32> {
        match parts.get(i) {
            Some(p) => p.parse().ok(),
            None => Some(0),
        }
    };
    Some(Version {
        major: num(0)?,
        minor: num(1)?,
        patch: num(2)?,
        release: !rest.starts_with('-'),
    })
}

/// Lizenz der Gewichte nach dem Ollama-Modell. `verified` nur für die
/// Tags, die die Bibliotheksseite ausdrücklich als 40M nennt
/// (`phonon:40m-de`/`phonon:de`, `phonon:multilingual`: CC BY-NC 4.0); alles
/// andere gilt vorsorglich als nichtkommerziell und ungeprüft.
fn license_for(model: &str) -> LicenseInfo {
    let code = "MIT OR Apache-2.0 (Phonon-Code, Gradium)".to_string();
    let m = model.trim().to_ascii_lowercase();
    let (name, tag) = m.split_once(':').unwrap_or((m.as_str(), ""));
    let nc_tag = name == "phonon"
        && (tag.starts_with("40m") || tag == "de" || tag.starts_with("multilingual"));
    if nc_tag {
        return LicenseInfo {
            code,
            weights: "CC BY-NC 4.0 (Phonon 40M)".to_string(),
            non_commercial: true,
            verified: true,
            source_url: SOURCE_URL.to_string(),
            verified_on: Some(VERIFIED_ON.to_string()),
            note: "Laut Ollama-Bibliothek (phonon): die 40M-Checkpoints (phonon:40m-de/phonon:de, \
                   phonon:multilingual) stehen unter CC BY-NC 4.0, keine kommerzielle Nutzung \
                   (geprüft 26.10.2026, docs/tts/provider-research.md, B)."
                .to_string(),
        };
    }
    LicenseInfo {
        code,
        weights: format!(
            "unbekannt: Lizenz des Ollama-Modells '{}' prüfen (40M-Checkpoints CC BY-NC 4.0)",
            model.trim()
        ),
        non_commercial: true,
        verified: false,
        source_url: SOURCE_URL.to_string(),
        verified_on: None,
        note: "Die Lizenz dieses Ollama-Modells ist nicht belegt. Der 31M-Checkpoint von Gradium \
               steht unter Apache-2.0, die Lizenz seiner Ollama-Fassung wurde nicht gegengeprüft; \
               vorsorglich wird das Modell als nichtkommerziell behandelt."
            .to_string(),
    }
}

/// Ein 404 auf der Sprachanfrage heißt bei Ollama: Modell nicht vorhanden
/// oder die Version kennt die Sprachausgabe nicht.
fn ollama_status(code: u16, msg: &str) -> Option<TtsError> {
    (code == 404).then(|| {
        let (a, b, c) = MIN_OLLAMA_VERSION;
        TtsError::unavailable(format!(
            "Ollama meldet für die Sprachausgabe 404: das Modell fehlt oder Ollama ist zu alt \
             (nötig ab {a}.{b}.{c}){}",
            if msg.is_empty() {
                String::new()
            } else {
                format!(": {msg}")
            }
        ))
    })
}

/// Eintrag `voices` irgendwo in einer `/api/show`-Antwort suchen (die
/// Struktur ist nicht belegt, nur dass es `voices` gibt).
fn find_voices(v: &Value, depth: u8) -> Option<&Value> {
    if depth > 3 {
        return None;
    }
    match v {
        Value::Object(o) => o
            .get("voices")
            .or_else(|| o.values().find_map(|c| find_voices(c, depth + 1))),
        Value::Array(a) => a.iter().find_map(|c| find_voices(c, depth + 1)),
        _ => None,
    }
}

// ---- Provider ----------------------------------------------------------------------

/// Ollama-Sprachanbieter (siehe Moduldoku).
pub struct OllamaSpeechProvider {
    cfg: OllamaSpeechConfig,
    root: String,
    inner: OpenAiSpeechProvider,
    host_os: String,
    probe_ttl: Duration,
    probe_cache: Mutex<Option<(Instant, Result<(), String>)>>,
}

impl OllamaSpeechProvider {
    /// Anbieter aus einer Konfiguration. Eine ungültige Konfiguration zeigt
    /// sich erst bei der Benutzung (Health mit Grund, Synthese mit
    /// `InvalidConfig`/`PrivacyBlocked`); vorher prüfen: [`OllamaSpeechConfig::validate`].
    pub fn new(cfg: OllamaSpeechConfig) -> Self {
        let inner = OpenAiSpeechProvider::new(cfg.inner_config())
            .with_license_override(license_for(&cfg.model))
            .with_status_mapper(ollama_status);
        Self {
            root: ollama_root(&cfg.base_url),
            cfg,
            inner,
            host_os: std::env::consts::OS.to_string(),
            probe_ttl: PROBE_TTL,
            probe_cache: Mutex::new(None),
        }
    }

    /// Wie [`new`](Self::new), aber mit sofortiger Prüfung.
    pub fn try_new(cfg: OllamaSpeechConfig) -> Result<Self, TtsError> {
        cfg.validate()?;
        Ok(Self::new(cfg))
    }

    pub fn config(&self) -> &OllamaSpeechConfig {
        &self.cfg
    }

    #[cfg(test)]
    pub(crate) fn with_host_os(mut self, os: &str) -> Self {
        self.host_os = os.to_string();
        self
    }

    #[cfg(test)]
    pub(crate) fn with_probe_ttl(mut self, ttl: Duration) -> Self {
        self.probe_ttl = ttl;
        self
    }

    fn platform_supported(&self) -> bool {
        self.host_os == "macos"
    }

    fn platform_reason(&self) -> String {
        format!(
            "Ollama erzeugt Sprache laut Upstream nur auf macOS (Release-Notes v0.40.11); \
             für {} ist es nicht belegt",
            self.host_os
        )
    }

    /// Plattformsperre ohne Anfrage: nicht macOS und keine Probe erlaubt.
    fn platform_block(&self) -> Option<TtsError> {
        (!self.platform_supported() && !self.cfg.probe_speech)
            .then(|| TtsError::unavailable(self.platform_reason()))
    }

    fn fail(&self, reason: String, hint: &str, version: Option<String>) -> ProviderHealth {
        let mut h = ProviderHealth::unavailable(reason, Some(hint.to_string()));
        h.device = Some(Device::Remote);
        h.version = version;
        h
    }

    /// Ergebnis der Probe, höchstens alle `probe_ttl` neu gerechnet.
    async fn probe_cached(&self) -> Result<(), String> {
        let cached = {
            let guard = self.probe_cache.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .as_ref()
                .filter(|(at, _)| at.elapsed() < self.probe_ttl)
                .map(|(_, result)| result.clone())
        };
        if let Some(result) = cached {
            return result;
        }
        let result = self
            .inner
            .probe(PROBE_TEXT, PROBE_TIMEOUT)
            .await
            .map(|_| ())
            .map_err(|e| e.message);
        *self.probe_cache.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), result.clone()));
        result
    }

    async fn staged_health(&self) -> ProviderHealth {
        if let Err(e) = self.inner.check_config() {
            return self.fail(
                e.message,
                "Einstellungen des Ollama-Sprachanbieters prüfen (Adresse, Modell).",
                None,
            );
        }
        let root = &self.root;

        // Stufe 1: Dienst erreichbar?
        let reply = match self
            .inner
            .http_json(
                Method::GET,
                &format!("{root}/api/version"),
                None,
                VERSION_TIMEOUT,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return self.fail(
                    format!("Ollama ist nicht erreichbar: {}", e.message),
                    "Ollama starten oder die Adresse prüfen.",
                    None,
                )
            }
        };
        if !(200..300).contains(&reply.status) {
            return self.fail(
                format!(
                    "Ollama antwortet unter {root}/api/version mit HTTP {}",
                    reply.status
                ),
                "Läuft unter dieser Adresse wirklich Ollama? Adresse prüfen.",
                None,
            );
        }
        let raw_version = reply.json["version"].as_str().unwrap_or("").to_string();
        let Some(version) = parse_version(&raw_version) else {
            return self.fail(
                format!("Die Ollama-Version ist nicht lesbar (Antwort: {raw_version:?})"),
                "Ollama aktualisieren oder die Adresse prüfen.",
                None,
            );
        };

        // Stufe 2: Version.
        if version < MIN_VERSION {
            let (a, b, c) = MIN_OLLAMA_VERSION;
            return self.fail(
                format!("Ollama {raw_version} ist für Sprachausgabe zu alt (nötig ab {a}.{b}.{c})"),
                "Ollama aktualisieren (Sprachausgabe gibt es seit v0.40.11, Release-Notes vom 24.10.2026).",
                Some(raw_version),
            );
        }

        // Stufe 3: Modell vorhanden?
        let model = self.cfg.model.trim().to_string();
        let tags = match self
            .inner
            .http_json(
                Method::GET,
                &format!("{root}/api/tags"),
                None,
                QUERY_TIMEOUT,
            )
            .await
        {
            Ok(r) if (200..300).contains(&r.status) => r.json,
            Ok(r) => {
                return self.fail(
                    format!(
                        "Ollama antwortet unter {root}/api/tags mit HTTP {}",
                        r.status
                    ),
                    "Ollama-Protokoll prüfen.",
                    Some(raw_version),
                )
            }
            Err(e) => {
                return self.fail(
                    format!("Die Modellliste von Ollama ist nicht lesbar: {}", e.message),
                    "Ollama-Protokoll prüfen.",
                    Some(raw_version),
                )
            }
        };
        let names: Vec<String> = tags["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["name"].as_str().or_else(|| m["model"].as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let wanted = model.to_ascii_lowercase();
        let wanted_latest = format!("{wanted}:latest");
        let present = names.iter().any(|n| {
            let n = n.to_ascii_lowercase();
            n == wanted || (!wanted.contains(':') && n == wanted_latest)
        });
        if !present {
            let similar: Vec<&str> = names
                .iter()
                .filter(|n| {
                    let n = n.to_ascii_lowercase();
                    n.starts_with(&format!("{}:", wanted.split(':').next().unwrap_or("")))
                })
                .map(String::as_str)
                .collect();
            let also = if similar.is_empty() {
                String::new()
            } else {
                format!(" (vorhanden: {})", similar.join(", "))
            };
            return self.fail(
                format!("Das Modell '{model}' ist in Ollama nicht vorhanden{also}"),
                &format!(
                    "In Ollama laden: `ollama pull {model}` (die App lädt selbst nichts). Hinweis: \
                     die 40M-Modelle sind CC BY-NC 4.0 (nicht kommerziell)."
                ),
                Some(raw_version),
            );
        }

        // Stufe 4: Plattform.
        if !self.platform_supported() && !self.cfg.probe_speech {
            return self.fail(
                self.platform_reason(),
                "Zum Ausprobieren probe_speech aktivieren (lädt das Sprachmodell in Ollama); \
                 sonst Phonon (ptts) oder einen OpenAI-kompatiblen Sprachserver nutzen.",
                Some(raw_version),
            );
        }
        if self.cfg.probe_speech {
            if let Err(why) = self.probe_cached().await {
                return self.fail(
                    format!("Probe der Sprachausgabe gescheitert: {why}"),
                    "Ollama-Protokoll prüfen; auf dieser Plattform ist Sprachausgabe laut Upstream nicht zugesagt.",
                    Some(raw_version),
                );
            }
        }
        let mut h = ProviderHealth::ok();
        h.device = Some(Device::Remote);
        h.version = Some(raw_version);
        h
    }

    async fn voices(&self, language: Option<&str>) -> Result<Vec<VoiceDescriptor>, TtsError> {
        self.inner.check_config()?;
        let lang = if self.cfg.languages.len() == 1 {
            self.cfg.languages[0].trim().to_ascii_lowercase()
        } else {
            "und".to_string()
        };
        let reply = self
            .inner
            .http_json(
                Method::POST,
                &format!("{}/api/show", self.root),
                Some(&json!({ "model": self.cfg.model.trim() })),
                QUERY_TIMEOUT,
            )
            .await;
        let mut voices = match reply {
            Ok(r) if (200..300).contains(&r.status) => match find_voices(&r.json, 0) {
                // Eine Zuordnung Name → Angaben liefert ihre Namen.
                Some(Value::Object(map)) => {
                    let names = Value::Array(map.keys().map(|k| json!(k)).collect());
                    parse_voices(&names, &lang)
                }
                Some(v) => parse_voices(v, &lang),
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
        if voices.is_empty() {
            if let Some(v) = self
                .cfg
                .voice
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                voices.push(VoiceDescriptor {
                    id: v.to_string(),
                    label: v.to_string(),
                    language: lang,
                    gender: None,
                    license: None,
                });
            }
        }
        voices.retain(|v| language_matches(&v.language, language));
        Ok(voices)
    }
}

impl TtsProvider for OllamaSpeechProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.inner.descriptor()
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(self.staged_health())
    }

    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
        Box::pin(self.voices(language))
    }

    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
        Box::pin(async move {
            if let Some(e) = self.platform_block() {
                return Err(e);
            }
            self.inner.synthesize(text, opts, cancel).await
        })
    }

    fn stream_synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a CancelToken,
        sink: mpsc::Sender<AudioChunk>,
    ) -> BoxFuture<'a, Result<(), TtsError>> {
        Box::pin(async move {
            if let Some(e) = self.platform_block() {
                return Err(e);
            }
            self.inner.stream_synthesize(text, opts, cancel, sink).await
        })
    }

    fn cache_tag(&self, voice: Option<&str>) -> String {
        self.inner.cache_tag(voice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::{StreamingMode, TtsErrorKind};
    use crate::managers::tts::providers::contract_tests::{contract_suite, ContractExpect};
    use crate::managers::tts::providers::test_http::{speech_reply, FakeServer, Reply, Req};
    use std::sync::Arc;

    /// Frei einstellbarer Ollama-Dienst.
    #[derive(Clone)]
    struct Ollama {
        version: String,
        models: Vec<&'static str>,
        /// Antwort auf `/v1/audio/speech`: `None` = Sprache, sonst fester Status.
        speech_status: Option<u16>,
        hang_speech: bool,
    }

    impl Ollama {
        fn ok() -> Self {
            Self {
                version: "0.40.11".to_string(),
                models: vec!["llama3:8b", "phonon:latest"],
                speech_status: None,
                hang_speech: false,
            }
        }

        async fn start(self) -> FakeServer {
            FakeServer::start(move |req: &Req| {
                match (req.method.as_str(), req.path.as_str()) {
                    ("GET", "/api/version") => Reply::json(200, json!({"version": self.version})),
                    ("GET", "/api/tags") => Reply::json(
                        200,
                        json!({"models": self.models.iter().map(|m| json!({"name": m, "model": m})).collect::<Vec<_>>()}),
                    ),
                    ("POST", "/api/show") => Reply::json(
                        200,
                        json!({
                            "capabilities": ["speech"],
                            "details": {"voices": [{"name": "Zoey"}, {"name": "Easton"}, "Freya"]}
                        }),
                    ),
                    ("POST", "/v1/audio/speech") => {
                        if self.hang_speech {
                            Reply::Hang
                        } else if let Some(status) = self.speech_status {
                            Reply::json(status, json!({"error": "model 'phonon' not found"}))
                        } else {
                            speech_reply(req, Duration::from_millis(1))
                        }
                    }
                    _ => Reply::json(404, json!({"error": "not found"})),
                }
            })
            .await
        }
    }

    fn cfg_for(server: &FakeServer) -> OllamaSpeechConfig {
        OllamaSpeechConfig {
            base_url: server.root.clone(),
            ..Default::default()
        }
    }

    /// Auf macOS-Verhalten festgelegt, damit Tests auf jedem System gleich laufen.
    fn mac(server: &FakeServer) -> OllamaSpeechProvider {
        OllamaSpeechProvider::new(cfg_for(server)).with_host_os("macos")
    }

    fn opts() -> SynthesisOptions {
        SynthesisOptions {
            timeout: Duration::from_secs(20),
            ..Default::default()
        }
    }

    // ---- Vertrag ------------------------------------------------------------

    #[tokio::test]
    async fn besteht_den_vertrag_gegen_den_fake_dienst_inklusive_abbruch_und_frist() {
        let server = Ollama::ok().start().await;
        let hang = Ollama {
            hang_speech: true,
            ..Ollama::ok()
        }
        .start()
        .await;
        contract_suite(
            Arc::new(mac(&server)),
            ContractExpect::available()
                .with_sample_rate(24_000)
                .with_slow(Arc::new(mac(&hang))),
        )
        .await;
        // Es wurde nur gelesen und gesprochen, nie etwas verändert.
        for r in server.requests() {
            let call = (r.method.as_str(), r.path.as_str());
            assert!(
                matches!(
                    call,
                    ("GET", "/api/version")
                        | ("GET", "/api/tags")
                        | ("POST", "/api/show")
                        | ("POST", "/v1/audio/speech")
                ),
                "unerwarteter Aufruf {call:?}"
            );
        }
    }

    #[tokio::test]
    async fn plattformsperre_besteht_den_vertrag_als_nicht_verfuegbar_ohne_sprachanfrage() {
        let server = Ollama::ok().start().await;
        let p = OllamaSpeechProvider::new(cfg_for(&server)).with_host_os("windows");
        let h = p.health_check().await;
        assert!(!h.available);
        let reason = h.reason.clone().unwrap();
        assert!(
            reason.contains("nur auf macOS") && reason.contains("windows"),
            "{reason}"
        );
        assert!(h.hint.unwrap().contains("probe_speech"));
        assert_eq!(h.version.as_deref(), Some("0.40.11"));
        contract_suite(Arc::new(p), ContractExpect::unavailable()).await;
        assert_eq!(
            server.count("/v1/audio/speech"),
            0,
            "die Sperre darf nichts anfragen"
        );
    }

    // ---- Version ------------------------------------------------------------

    #[test]
    fn version_wird_tolerant_gelesen_und_richtig_verglichen() {
        let v = |s: &str| parse_version(s);
        assert_eq!(
            MIN_OLLAMA_VERSION,
            (0, 40, 11),
            "Quelle: Release-Notes v0.40.11"
        );
        assert!(v("0.40.11").unwrap() >= MIN_VERSION);
        assert!(v("v0.40.11").unwrap() >= MIN_VERSION);
        assert!(v("0.40.12").unwrap() > MIN_VERSION);
        assert!(v("0.41.0").unwrap() > MIN_VERSION);
        assert!(v("1.0.0").unwrap() > MIN_VERSION);
        assert!(v("0.40.10").unwrap() < MIN_VERSION);
        assert!(
            v("0.34.2").unwrap() < MIN_VERSION,
            "lokaler Client 0.34.2 ist zu alt"
        );
        assert!(v("0.40.9").unwrap() < MIN_VERSION);
        assert!(
            v("0.9.99").unwrap() < MIN_VERSION,
            "numerisch, nicht als Text"
        );
        assert!(
            v("0.40.11-rc1").unwrap() < MIN_VERSION,
            "Vorabversion ist kleiner"
        );
        assert!(v("0.40.12-rc1").unwrap() >= MIN_VERSION);
        assert!(v("0.40.11+build5").unwrap() >= MIN_VERSION);
        assert_eq!(
            v("0.40").unwrap(),
            Version {
                major: 0,
                minor: 40,
                patch: 0,
                release: true
            }
        );
        assert_eq!(v(" v1.2.3 ").unwrap().to_string(), "1.2.3");
        for bad in [
            "",
            "abc",
            "1",
            "1.2.3.4",
            "1..3",
            ".1.2",
            "v",
            "0.40.x",
            "99999999999.0.0",
        ] {
            assert!(v(bad).is_none(), "{bad:?}");
        }
    }

    // ---- Health in Stufen ---------------------------------------------------------

    #[tokio::test]
    async fn health_ok_mit_version_auf_macos() {
        let server = Ollama::ok().start().await;
        let h = mac(&server).health_check().await;
        assert!(h.available, "{h:?}");
        assert_eq!(h.version.as_deref(), Some("0.40.11"));
        assert_eq!(h.device, Some(Device::Remote));
        assert!(h.reason.is_none());
        // Nur die zwei lesenden Abfragen, keine Sprachanfrage ohne probe_speech.
        let calls: Vec<_> = server.requests().iter().map(|r| r.path.clone()).collect();
        assert_eq!(calls, ["/api/version", "/api/tags"]);
    }

    #[tokio::test]
    async fn health_nennt_bei_zu_alter_version_den_grund_und_fragt_nicht_weiter() {
        for old in ["0.40.10", "0.34.2", "0.40.11-rc1"] {
            let server = Ollama {
                version: old.to_string(),
                models: vec![], // würde in Stufe 3 scheitern
                ..Ollama::ok()
            }
            .start()
            .await;
            let h = mac(&server).health_check().await;
            assert!(!h.available, "{old}");
            let reason = h.reason.unwrap();
            assert!(
                reason.contains(old) && reason.contains("zu alt"),
                "{reason}"
            );
            assert!(reason.contains("0.40.11"));
            assert!(h.hint.unwrap().contains("aktualisieren"));
            assert_eq!(h.version.as_deref(), Some(old));
            assert_eq!(server.count("/api/tags"), 0, "Stufe 3 darf nicht laufen");
        }
    }

    #[tokio::test]
    async fn health_nicht_erreichbar_unlesbare_version_und_fehlerstatus() {
        // Nichts lauscht.
        let port = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        let p = OllamaSpeechProvider::new(OllamaSpeechConfig {
            base_url: format!("http://127.0.0.1:{port}"),
            ..Default::default()
        })
        .with_host_os("macos");
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h
            .reason
            .as_deref()
            .unwrap()
            .starts_with("Ollama ist nicht erreichbar"));
        assert!(h.hint.unwrap().contains("Ollama starten"));
        assert_eq!(h.version, None);
        // Synthese gegen den toten Dienst: wiederholbarer Netzfehler.
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Network);
        assert!(e.retryable);

        // Version nicht lesbar.
        let server = FakeServer::start(|_| Reply::json(200, json!({"version": "kaputt"}))).await;
        let h = mac(&server).health_check().await;
        assert!(h.reason.unwrap().contains("nicht lesbar"));
        let server = FakeServer::start(|_| Reply::json(200, json!({"andere": 1}))).await;
        assert!(mac(&server)
            .health_check()
            .await
            .reason
            .unwrap()
            .contains("nicht lesbar"));

        // Kein Ollama (HTTP-Fehler).
        let server = FakeServer::start(|_| Reply::json(500, json!({}))).await;
        let h = mac(&server).health_check().await;
        assert!(h.reason.unwrap().contains("HTTP 500"));

        // Hängender Dienst: Frist 2 s greift, kein Hängen.
        let server = FakeServer::start(|_| Reply::Hang).await;
        let started = Instant::now();
        let h = mac(&server).health_check().await;
        assert!(!h.available);
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[tokio::test]
    async fn health_nennt_fehlendes_modell_mit_vorhandenen_varianten() {
        let server = Ollama {
            models: vec!["llama3:8b", "phonon:de", "phonon:multilingual"],
            ..Ollama::ok()
        }
        .start()
        .await;
        let h = mac(&server).health_check().await;
        assert!(!h.available);
        let reason = h.reason.unwrap();
        assert!(
            reason.contains("'phonon'") && reason.contains("nicht vorhanden"),
            "{reason}"
        );
        assert!(reason.contains("phonon:de") && reason.contains("phonon:multilingual"));
        assert!(!reason.contains("llama3"));
        let hint = h.hint.unwrap();
        assert!(
            hint.contains("ollama pull phonon") && hint.contains("selbst nichts"),
            "{hint}"
        );
        assert_eq!(h.version.as_deref(), Some("0.40.11"));
        // Kein Pull und sonst nichts Schreibendes.
        assert!(server.requests().iter().all(|r| r.method == "GET"));

        // Anderer Modellname per Konfiguration (Tag wird exakt verglichen).
        let mut cfg = cfg_for(&server);
        cfg.model = "phonon:de".to_string();
        assert!(
            OllamaSpeechProvider::new(cfg)
                .with_host_os("macos")
                .health_check()
                .await
                .available
        );
        // Ohne Tag gilt `:latest`.
        let server = Ollama {
            models: vec!["phonon"],
            ..Ollama::ok()
        }
        .start()
        .await;
        assert!(
            mac(&server).health_check().await.available,
            "Name ohne Tag exakt"
        );
        let server = Ollama {
            models: vec!["PHONON:LATEST"],
            ..Ollama::ok()
        }
        .start()
        .await;
        assert!(
            mac(&server).health_check().await.available,
            "Groß-/Kleinschreibung egal"
        );
        // Leere Modellliste.
        let server = Ollama {
            models: vec![],
            ..Ollama::ok()
        }
        .start()
        .await;
        let reason = mac(&server).health_check().await.reason.unwrap();
        assert!(!reason.contains("vorhanden:"), "{reason}");
    }

    #[tokio::test]
    async fn health_modellliste_mit_fehler_ist_ein_eigener_grund() {
        let server = FakeServer::start(|req: &Req| {
            if req.path == "/api/version" {
                Reply::json(200, json!({"version": "0.40.11"}))
            } else {
                Reply::json(500, json!({}))
            }
        })
        .await;
        let h = mac(&server).health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("/api/tags mit HTTP 500"));
    }

    #[tokio::test]
    async fn probe_speech_entscheidet_ausserhalb_von_macos_ueber_echtes_audio() {
        let server = Ollama::ok().start().await;
        let mut cfg = cfg_for(&server);
        cfg.probe_speech = true;
        let p = OllamaSpeechProvider::new(cfg.clone()).with_host_os("windows");
        let h = p.health_check().await;
        assert!(h.available, "{h:?}");
        assert!(h.reason.is_none());
        // Genau eine Mini-Probe: wenige Zeichen, Rohdaten, das konfigurierte Modell.
        let probes: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.path == "/v1/audio/speech")
            .collect();
        assert_eq!(probes.len(), 1);
        let body = probes[0].json();
        assert_eq!(body["model"], "phonon");
        assert_eq!(body["response_format"], "pcm");
        assert!(body["input"].as_str().unwrap().chars().count() <= 10);
        assert!(body.get("speed").is_none());
        // Zwischengespeichert: der zweite Health-Aufruf rechnet nicht noch einmal.
        assert!(p.health_check().await.available);
        assert_eq!(
            server.count("/v1/audio/speech"),
            1,
            "Probe muss zwischengespeichert sein"
        );
        // Ohne Zwischenspeicher (TTL 0) rechnet jeder Aufruf.
        let p0 = OllamaSpeechProvider::new(cfg.clone())
            .with_host_os("windows")
            .with_probe_ttl(Duration::ZERO);
        p0.health_check().await;
        p0.health_check().await;
        assert_eq!(server.count("/v1/audio/speech"), 3);
        // Mit Probe entfällt die Plattformsperre auch bei der Synthese.
        p.synthesize("Hallo Welt", &opts(), &CancelToken::detached())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn gescheiterte_probe_macht_den_anbieter_nicht_verfuegbar() {
        // Dienst antwortet auf Sprache mit 404 (Modell fehlt) bzw. mit JSON statt Audio.
        let server = Ollama {
            speech_status: Some(400),
            ..Ollama::ok()
        }
        .start()
        .await;
        let mut cfg = cfg_for(&server);
        cfg.probe_speech = true;
        let h = OllamaSpeechProvider::new(cfg.clone())
            .with_host_os("windows")
            .health_check()
            .await;
        assert!(!h.available);
        let reason = h.reason.unwrap();
        assert!(
            reason.contains("Probe der Sprachausgabe gescheitert"),
            "{reason}"
        );
        assert!(reason.contains("not found"), "Serverauszug fehlt: {reason}");

        let server = FakeServer::start(|req: &Req| match req.path.as_str() {
            "/api/version" => Reply::json(200, json!({"version": "0.40.11"})),
            "/api/tags" => Reply::json(200, json!({"models": [{"name": "phonon:latest"}]})),
            _ => Reply::json(200, json!({"nichts": "audio"})),
        })
        .await;
        let mut cfg = cfg_for(&server);
        cfg.probe_speech = true;
        let h = OllamaSpeechProvider::new(cfg)
            .with_host_os("windows")
            .health_check()
            .await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("statt Audio"));
    }

    #[tokio::test]
    async fn probe_hat_eine_frist_und_haengt_nicht() {
        let server = Ollama {
            hang_speech: true,
            ..Ollama::ok()
        }
        .start()
        .await;
        let mut cfg = cfg_for(&server);
        cfg.probe_speech = true;
        let started = Instant::now();
        let h = tokio::time::timeout(
            Duration::from_secs(15),
            OllamaSpeechProvider::new(cfg)
                .with_host_os("linux")
                .health_check(),
        )
        .await
        .expect("Probe hängt");
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("Keine Antwort auf die Probe"));
        assert!(started.elapsed() < Duration::from_secs(13));
    }

    // ---- Synthese -----------------------------------------------------------------

    #[tokio::test]
    async fn anfrage_nutzt_das_konfigurierte_modell_und_laesst_stimme_und_speed_weg() {
        let server = Ollama {
            models: vec!["phonon:de"],
            ..Ollama::ok()
        }
        .start()
        .await;
        let mut cfg = cfg_for(&server);
        cfg.model = "phonon:de".to_string();
        let p = OllamaSpeechProvider::new(cfg).with_host_os("macos");
        let mut o = opts();
        o.speed = Some(1.3);
        o.format = crate::managers::tts::providers::contract::AudioEncoding::Pcm16Le;
        p.synthesize("Guten Tag", &o, &CancelToken::detached())
            .await
            .unwrap();
        let req = server
            .requests()
            .into_iter()
            .find(|r| r.path == "/v1/audio/speech")
            .unwrap();
        let body = req.json();
        assert_eq!(body["model"], "phonon:de");
        assert_eq!(body["input"], "Guten Tag");
        assert_eq!(body["response_format"], "pcm");
        assert!(body.get("voice").is_none(), "keine hart kodierte Stimme");
        assert!(
            body.get("speed").is_none(),
            "Ollama erlaubt nur speed=1: nicht senden"
        );
        // Mit Stimme aus der Konfiguration bzw. aus opts.
        let mut cfg = cfg_for(&server);
        cfg.voice = Some("Zoey".to_string());
        let p = OllamaSpeechProvider::new(cfg).with_host_os("macos");
        p.synthesize("Eins", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let mut o = opts();
        o.voice = Some("Freya".to_string());
        p.synthesize("Zwei", &o, &CancelToken::detached())
            .await
            .unwrap();
        let voices: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.path == "/v1/audio/speech")
            .skip(1)
            .map(|r| r.json()["voice"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(voices, ["Zoey", "Freya"]);
    }

    #[tokio::test]
    async fn dienstfehler_auf_der_sprachanfrage_werden_eingeordnet() {
        // 404: Modell fehlt oder Version zu alt -> nicht verfügbar mit Hinweis.
        let server = Ollama {
            speech_status: Some(404),
            ..Ollama::ok()
        }
        .start()
        .await;
        let e = mac(&server)
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Unavailable, "{e:?}");
        assert!(!e.retryable);
        assert!(e.message.contains("0.40.11") && e.message.contains("not found"));
        // 400 (nicht unterstützte Option): Anfragefehler.
        let server = Ollama {
            speech_status: Some(400),
            ..Ollama::ok()
        }
        .start()
        .await;
        let e = mac(&server)
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        // 500: wiederholbarer Backend-Fehler.
        let server = Ollama {
            speech_status: Some(500),
            ..Ollama::ok()
        }
        .start()
        .await;
        let e = mac(&server)
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.retryable);
    }

    // ---- Datenschutz -------------------------------------------------------------------

    #[tokio::test]
    async fn dienst_ausserhalb_von_loopback_braucht_freigabe() {
        let cfg = OllamaSpeechConfig {
            base_url: "http://192.0.2.7:11434".to_string(),
            ..Default::default()
        };
        assert_eq!(
            cfg.validate().unwrap_err().kind,
            TtsErrorKind::PrivacyBlocked
        );
        let p = OllamaSpeechProvider::new(cfg.clone()).with_host_os("macos");
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked);
        let h = p.health_check().await;
        assert!(!h.available && h.reason.unwrap().contains("allow_remote"));
        assert_eq!(
            p.list_voices(None).await.unwrap_err().kind,
            TtsErrorKind::PrivacyBlocked
        );
        let mut allowed = cfg;
        allowed.allow_remote = true;
        allowed.validate().unwrap();
    }

    // ---- Stimmen ---------------------------------------------------------------------------

    #[tokio::test]
    async fn stimmen_kommen_aus_api_show_und_fallen_auf_die_konfigurierte_zurueck() {
        let server = Ollama::ok().start().await;
        let p = mac(&server);
        let v = p.list_voices(None).await.unwrap();
        let ids: Vec<_> = v.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["Zoey", "Easton", "Freya"]);
        let show = server
            .requests()
            .into_iter()
            .find(|r| r.path == "/api/show")
            .unwrap();
        assert_eq!(show.json()["model"], "phonon");
        assert_eq!(show.method, "POST");

        // Dienst ohne Stimmenangabe / mit Fehler: nur die konfigurierte Stimme.
        let server = FakeServer::start(|_| Reply::json(500, json!({}))).await;
        let mut cfg = cfg_for(&server);
        cfg.voice = Some("Zoey".into());
        cfg.languages = vec!["en".into()];
        let v = OllamaSpeechProvider::new(cfg)
            .list_voices(None)
            .await
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].id.as_str(), v[0].language.as_str()), ("Zoey", "en"));
        assert!(OllamaSpeechProvider::new(cfg_for(&server))
            .list_voices(None)
            .await
            .unwrap()
            .is_empty());

        // Stimmen als Zuordnung Name -> Angaben.
        let server =
            FakeServer::start(|_| Reply::json(200, json!({"voices": {"Ava": {}, "Ben": {}}})))
                .await;
        let v = OllamaSpeechProvider::new(cfg_for(&server))
            .list_voices(None)
            .await
            .unwrap();
        let mut ids: Vec<_> = v.iter().map(|v| v.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, ["Ava", "Ben"]);
    }

    // ---- Beschreibung, Konfiguration ---------------------------------------------------

    #[test]
    fn beschreibung_und_lizenz() {
        let d = OllamaSpeechProvider::new(OllamaSpeechConfig::default()).descriptor();
        assert_eq!(d.id, "ollama-speech");
        assert_eq!(d.label, "Ollama Sprachausgabe");
        assert_eq!(
            d.tier,
            crate::managers::tts::providers::contract::ProviderTier::Experimental
        );
        assert!(d.opt_in);
        assert_eq!(d.caps.streaming, StreamingMode::Chunk);
        assert_eq!(d.caps.sample_rates, vec![24_000]);
        assert_eq!(d.caps.languages, vec!["und".to_string()]);
        // Standard `phonon`: Lizenz der Ollama-Fassung nicht belegt, vorsorglich nichtkommerziell.
        assert!(d.license.non_commercial && !d.license.verified);
        assert!(d.license.verified_on.is_none() && !d.license.note.is_empty());

        for model in [
            "phonon:de",
            "phonon:40m-de",
            "phonon:multilingual",
            "PHONON:DE",
        ] {
            let l = OllamaSpeechProvider::new(OllamaSpeechConfig {
                model: model.into(),
                ..Default::default()
            })
            .descriptor()
            .license;
            assert!(l.verified && l.non_commercial, "{model}");
            assert_eq!(l.verified_on.as_deref(), Some("2026-10-26"));
            assert!(l.weights.contains("CC BY-NC 4.0"));
            assert_eq!(l.source_url, "https://ollama.com/library/phonon");
        }
        let l = OllamaSpeechProvider::new(OllamaSpeechConfig {
            model: "anderes".into(),
            ..Default::default()
        })
        .descriptor()
        .license;
        assert!(!l.verified && l.non_commercial);
        assert!(l.weights.contains("anderes"));

        let d = OllamaSpeechProvider::new(OllamaSpeechConfig {
            languages: vec!["de".into()],
            ..Default::default()
        })
        .descriptor();
        assert_eq!(d.caps.languages, vec!["de".to_string()]);
    }

    #[test]
    fn cache_tag_nennt_modell_und_stimme() {
        let p = OllamaSpeechProvider::new(OllamaSpeechConfig::default());
        assert_eq!(p.cache_tag(None), "ollama-speech:phonon:default");
        assert_eq!(p.cache_tag(Some("Zoey")), "ollama-speech:phonon:Zoey");
        let q = OllamaSpeechProvider::new(OllamaSpeechConfig {
            model: "phonon:de".into(),
            voice: Some("Freya".into()),
            ..Default::default()
        });
        assert_eq!(q.cache_tag(None), "ollama-speech:phonon:de:Freya");
    }

    #[test]
    fn konfiguration_hat_sichere_vorgaben_und_prueft_die_adresse() {
        let d = OllamaSpeechConfig::default();
        assert_eq!(d.base_url, "http://127.0.0.1:11434");
        assert_eq!(d.model, "phonon");
        assert!(d.voice.is_none() && !d.probe_speech && !d.allow_remote);
        d.validate().unwrap();
        assert_eq!(serde_json::from_str::<OllamaSpeechConfig>("{}").unwrap(), d);
        let c: OllamaSpeechConfig = serde_json::from_str(r#"{"probe_speech":true}"#).unwrap();
        assert!(c.probe_speech && c.model == "phonon");

        // `/v1` am Ende und Schrägstriche werden toleriert.
        for url in [
            "http://localhost:11434/",
            "http://localhost:11434/v1",
            "http://localhost:11434/v1/",
        ] {
            assert_eq!(ollama_root(url), "http://localhost:11434", "{url}");
        }
        let bad = |mutate: &dyn Fn(&mut OllamaSpeechConfig), needle: &str| {
            let mut c = OllamaSpeechConfig::default();
            mutate(&mut c);
            let e = c.validate().expect_err(needle);
            assert_eq!(e.kind, TtsErrorKind::InvalidConfig, "{needle}: {e:?}");
            assert!(
                e.message.contains(needle),
                "{needle} fehlt in {:?}",
                e.message
            );
        };
        bad(&|c| c.model = " ".into(), "model fehlt");
        bad(&|c| c.base_url = "".into(), "keine gültige URL");
        bad(&|c| c.base_url = "ftp://localhost".into(), "http://");
        bad(
            &|c| c.base_url = "http://u:p@localhost".into(),
            "Zugangsdaten",
        );
        bad(&|c| c.languages = vec![" ".into()], "languages");

        let mut types = specta::TypeCollection::default();
        types.register::<OllamaSpeechConfig>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .unwrap();
        for needle in [
            "OllamaSpeechConfig",
            "probe_speech: boolean",
            "model: string",
        ] {
            assert!(ts.contains(needle), "{needle} fehlt in:\n{ts}");
        }
    }

    #[test]
    fn standardplattform_ist_die_echte() {
        let p = OllamaSpeechProvider::new(OllamaSpeechConfig::default());
        assert_eq!(p.host_os, std::env::consts::OS);
        assert_eq!(p.platform_supported(), cfg!(target_os = "macos"));
    }
}
