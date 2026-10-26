//! Generischer Adapter für `POST {base}/audio/speech` (Paket P2, Goal tts-provider).
//!
//! Deckt jeden Server mit der OpenAI-Sprachschnittstelle ab: `ptts-openai-server`
//! (Phonon), Kokoro-FastAPI und — über [`super::ollama_speech`] — Ollama. Die
//! Schnittstelle und die Abweichungen der Server stehen in
//! `docs/tts/provider-research.md`, Abschnitt D.
//!
//! * Anfrage: JSON `{model, input, voice?, response_format}`. `speed` wird nie
//!   gesendet (alle drei Server kennen nur 1.0; das Tempo regelt der Player),
//!   `voice` nur, wenn eine Stimme gewählt ist. `response_format` ist `pcm`
//!   (16 Bit mono, Rate aus der Konfiguration) oder `wav` (Rate steht im
//!   Kopf); `opts.format` steuert das: `Pcm16Le` → `pcm`, `Wav` → `wav`.
//! * Antwort: echtes Streaming. Die Antwort wird stückweise gelesen, auf
//!   Frame-Grenzen (2 Byte je Kanal) ausgerichtet — ein Rest wandert in den
//!   nächsten Teil, nie ein halbes Sample — und als Chunks mit lückenlosem
//!   `seq` weitergereicht. Damit genau der LETZTE Chunk `is_last` trägt,
//!   hält der Adapter das letzte Frame zurück und sendet es am Ende der
//!   Antwort (kein zusätzlicher Wartetakt: alles davor geht sofort raus; der
//!   letzte Chunk ist dafür nur ein Frame lang). `synthesize` sammelt die Teile
//!   und verpackt sie mit `wav_from_pcm` zu EINEM sauberen WAV. WAV-Antworten
//!   mit Streaming-Kopf (`0xFFFFFFFF`) verträgt der Dekoder ([`PcmDecoder`]).
//! * Datenschutz im Adapter (zusätzlich zur Sperre in `commands`, P3): ein
//!   Host außerhalb von Loopback (`127.0.0.0/8`, `::1`, `localhost`) braucht
//!   `allow_remote`, sonst `PrivacyBlocked`. Umleitungen werden nicht
//!   verfolgt (ein Loopback-Server könnte sonst Text an einen Fremden
//!   weiterreichen); für Loopback wird der System-Proxy umgangen.
//! * Der API-Schlüssel steht nie in der Konfiguration, nur der NAME einer
//!   Umgebungsvariablen (`api_key_env`). Er wird als `Bearer` gesendet (als
//!   sensibel markiert), taucht in keiner Fehlermeldung auf und wird in
//!   Server-Auszügen geschwärzt.
//!
//! ## Fehlerklassen
//!
//! | Ursache | Klasse |
//! |---|---|
//! | Verbindung/Zeitüberschreitung/abgerissener Strom | `Network`, wiederholbar |
//! | 5xx, 429 | `Backend`, wiederholbar |
//! | 408 | `Network`, wiederholbar |
//! | 400/404/422 und übrige 4xx | `BadRequest` (Auszug höchstens 300 Zeichen), nicht wiederholbar |
//! | 401/403, 3xx, fehlender Schlüssel | `InvalidConfig` |
//! | Nicht-Loopback ohne `allow_remote` | `PrivacyBlocked` |
//! | JSON/Text statt Audio, falsches WAV, halbes Sample, zu groß | `Backend` |
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Gleichzeitige Aufträge | keine gemeinsamen veränderlichen Daten außer dem (nebenläufigen) HTTP-Client; Test `parallele_auftraege_…` |
//! | Abbruch/Frist mitten im Strom | `guarded` lässt das Future (samt Antwort) fallen, die Verbindung wird geschlossen; Test `abbruch_schliesst_die_verbindung…` (Server sieht den Abriss) |
//! | Abgerissene Verbindung mitten im Strom | `Network`, KEIN `is_last`-Chunk; Test `abgerissener_strom_…` |
//! | Voller Speicher | Antwortdeckel 256 MiB (Dekoder und `synthesize`), Stream-Kanal begrenzt (`send_chunk`); Test `zu_grosse_antwort_…` |
//! | Fehlender Server | Health `unavailable` mit Grund und Hinweis, `synthesize` → `Network` (wiederholbar), Stimmenliste fällt auf die konfigurierte Stimme zurück; Test `server_nicht_erreichbar_…` |
//! | Hänger des Servers | Frist (`opts.timeout`) → `Timeout`; Health- und Stimmenfrist 3 s |
//! | Kaputte Antwort | Dekoder-Tests mit ungeraden Grenzen, halbem Sample, Float-WAV, Müll |
//! | Datenabfluss | Loopback-Pflicht, keine Umleitungen, kein Proxy für Loopback |

use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use tokio::sync::mpsc;

use super::contract::{
    guarded, is_valid_provider_id, language_matches, send_chunk, AudioChunk, AudioClip,
    AudioEncoding, AudioFormat, CancelToken, Device, LicenseInfo, ProviderCaps, ProviderDescriptor,
    ProviderHealth, ProviderTier, StreamingMode, SynthesisOptions, TtsError, TtsErrorKind,
    TtsProvider, VoiceDescriptor,
};
use super::wav::{parse_wav, wav_from_pcm};

pub const DEFAULT_ID: &str = "openai-speech";
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8880/v1";
/// `input` darf höchstens 4096 Zeichen haben (OpenAI-Spezifikation, Ollama,
/// ptts-openai-server; provider-research.md, D).
const MAX_INPUT_CHARS: usize = 4096;
/// Obergrenze der gesamten Audioantwort.
const MAX_AUDIO_BYTES: u64 = 256 * 1024 * 1024;
/// Frist für Health und Stimmenliste.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
/// Obergrenze für JSON-Antworten (Stimmenlisten, Modelle).
const JSON_BODY_CAP: usize = 1024 * 1024;
/// So viel Antworttext erscheint in Fehlermeldungen.
const EXCERPT_CHARS: usize = 300;
/// Ein WAV-Kopf ist nie größer; darüber ist es kein WAV.
const MAX_WAV_HEADER: usize = 64 * 1024;
const DEFAULT_LICENSE_URL: &str = "https://github.com/openai/openai-openapi";

// ---- Konfiguration ---------------------------------------------------------------

/// Vom Nutzer angegebene Lizenzdaten des Servers/Modells. Sie gelten immer
/// als UNGEPRÜFT (`verified = false`): die Angabe ersetzt keine Prüfung.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct LicenseConfig {
    pub code: String,
    pub weights: String,
    pub non_commercial: bool,
    pub source_url: String,
    pub note: String,
}

/// Einstellungen des generischen Sprachserver-Adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct OpenAiSpeechConfig {
    /// Kennung des Anbieters (`[a-z0-9][a-z0-9_-]*`).
    pub id: String,
    pub label: String,
    /// Basis ohne `/audio/speech`, z. B. `http://127.0.0.1:8880/v1`.
    pub base_url: String,
    /// Wird immer gesendet (OpenAI-Pflichtfeld); manche Server ignorieren es.
    pub model: String,
    /// Stimme; `None` = nicht senden (Servervorgabe).
    pub voice: Option<String>,
    /// Abtastrate der Rohdaten (`pcm`); bei `wav` zählt der Kopf.
    pub sample_rate: u32,
    /// Sprachkürzel, die der Server spricht (`und` = alle).
    pub languages: Vec<String>,
    /// Server außerhalb von Loopback erlauben (Standard: nein).
    pub allow_remote: bool,
    /// NAME einer Umgebungsvariablen mit dem API-Schlüssel (nie der Schlüssel selbst).
    pub api_key_env: Option<String>,
    /// Lizenzangaben des Nutzers (immer ungeprüft).
    pub license: Option<LicenseConfig>,
}

impl Default for OpenAiSpeechConfig {
    fn default() -> Self {
        Self {
            id: DEFAULT_ID.to_string(),
            label: "OpenAI-kompatibler Sprachserver".to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: String::new(),
            voice: None,
            sample_rate: 24_000,
            languages: vec!["und".to_string()],
            allow_remote: false,
            api_key_env: None,
            license: None,
        }
    }
}

/// Gültig geprüfte Basis.
struct Parsed {
    /// Ohne abschließenden Schrägstrich.
    base: String,
    loopback: bool,
}

/// Ist der Host der URL Loopback (`127.0.0.0/8`, `::1`, IPv4-gemapptes
/// `::ffff:127.x`, genau `localhost`)? Unparsbare URLs: nein.
pub fn is_loopback_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url.trim()) else {
        return false;
    };
    match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        None => false,
    }
}

fn invalid(msg: impl Into<String>) -> TtsError {
    TtsError::invalid_config(msg)
}

impl OpenAiSpeechConfig {
    fn parse(&self) -> Result<Parsed, TtsError> {
        if !is_valid_provider_id(&self.id) {
            return Err(invalid(format!(
                "id '{}' ist ungültig: erlaubt sind Kleinbuchstaben, Ziffern, '-' und '_' (nicht am Anfang)",
                self.id
            )));
        }
        let raw = self.base_url.trim();
        if raw.is_empty() {
            return Err(invalid("base_url fehlt (z. B. http://127.0.0.1:8880/v1)"));
        }
        let url = url::Url::parse(raw)
            .map_err(|e| invalid(format!("base_url '{raw}' ist keine gültige URL: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(invalid(format!(
                "base_url muss mit http:// oder https:// beginnen (war: {}://)",
                url.scheme()
            )));
        }
        if url.host().is_none() {
            return Err(invalid("base_url enthält keinen Host"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(invalid(
                "base_url darf keine Zugangsdaten enthalten; den Schlüssel über api_key_env angeben",
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(invalid(
                "base_url darf keine Query und kein Fragment enthalten",
            ));
        }
        let loopback = is_loopback_url(raw);
        if !loopback && !self.allow_remote {
            return Err(TtsError::new(
                TtsErrorKind::PrivacyBlocked,
                format!(
                    "Der Server {} liegt nicht auf diesem Rechner (nicht Loopback); der Text würde das Gerät verlassen. Nur mit ausdrücklicher Freigabe (allow_remote) möglich.",
                    url.host_str().unwrap_or("?")
                ),
            ));
        }
        if self.model.trim().is_empty() {
            return Err(invalid("model fehlt (wird dem Server immer mitgesendet)"));
        }
        if self.model.chars().any(char::is_control) {
            return Err(invalid("model enthält Steuerzeichen"));
        }
        if let Some(v) = self.voice.as_deref() {
            if v.chars().any(char::is_control) {
                return Err(invalid("voice enthält Steuerzeichen"));
            }
        }
        if !(8_000..=192_000).contains(&self.sample_rate) {
            return Err(invalid(format!(
                "sample_rate {} liegt nicht in 8000 bis 192000",
                self.sample_rate
            )));
        }
        if self.languages.is_empty()
            || self
                .languages
                .iter()
                .any(|l| l.trim().is_empty() || l.trim().chars().count() > 16)
        {
            return Err(invalid(
                "languages braucht mindestens ein Sprachkürzel (z. B. de, en oder und)",
            ));
        }
        if let Some(name) = self.api_key_env.as_deref() {
            let name = name.trim();
            let ok = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit());
            if !ok {
                return Err(invalid(format!(
                    "api_key_env '{name}' ist kein Name einer Umgebungsvariablen (nur Buchstaben, Ziffern, '_')"
                )));
            }
        }
        let mut base = url.as_str().trim_end_matches('/').to_string();
        if base.is_empty() {
            base = raw.trim_end_matches('/').to_string();
        }
        Ok(Parsed { base, loopback })
    }

    /// Prüft die Einstellungen; die Meldungen sind für den Nutzer gedacht.
    /// Ein Host außerhalb von Loopback ohne `allow_remote` ist `PrivacyBlocked`.
    pub fn validate(&self) -> Result<(), TtsError> {
        self.parse().map(|_| ())
    }
}

// ---- Audio-Dekoder -------------------------------------------------------------------

/// Format der Antwort auf dem Draht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wire {
    /// Rohes PCM, 16 Bit little endian, mono, Rate aus der Konfiguration.
    Pcm,
    /// WAV-Container (Rate und Kanäle im Kopf, evtl. Streaming-Kopf).
    Wav,
}

impl Wire {
    fn name(self) -> &'static str {
        match self {
            Self::Pcm => "pcm",
            Self::Wav => "wav",
        }
    }
}

/// Setzt beliebig zerhackte Antwortstücke zu frame-genauem PCM zusammen.
///
/// * `Pcm`: sofort bereit, Kanäle = 1.
/// * `Wav`: sammelt den Kopf, bis der `data`-Chunk beginnt, prüft 16-Bit-PCM
///   und übernimmt Rate und Kanäle; bei endlicher Längenangabe zählt nur
///   diese Menge (Zusatzchunks hinter den Daten werden nicht als Audio
///   gelesen), bei `0`/`0xFFFFFFFF` alles bis zum Ende.
/// * Ein Rest unterhalb eines Frames (2 Byte je Kanal) bleibt für den nächsten
///   Aufruf liegen — ausgegeben wird nur, was ganze Frames sind.
pub(crate) struct PcmDecoder {
    wire: Wire,
    rate: u32,
    channels: u16,
    header: Vec<u8>,
    header_done: bool,
    remaining: Option<u64>,
    pending: Vec<u8>,
    total: u64,
    max_total: u64,
}

impl PcmDecoder {
    pub fn new(wire: Wire, pcm_rate: u32, max_total: u64) -> Self {
        Self {
            wire,
            rate: pcm_rate,
            channels: 1,
            header: Vec::new(),
            header_done: wire == Wire::Pcm,
            remaining: None,
            pending: Vec::new(),
            total: 0,
            max_total,
        }
    }

    /// Rate und Kanäle, sobald bekannt (bei `Pcm` von Anfang an).
    pub fn format(&self) -> Option<(u32, u16)> {
        self.header_done.then_some((self.rate, self.channels))
    }

    fn frame(&self) -> usize {
        2 * usize::from(self.channels.max(1))
    }

    /// Ein Stück der Antwort einspeisen; liefert die daraus entstandenen
    /// ganzen Frames (kann leer sein).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<u8>, TtsError> {
        if self.header_done {
            return self.feed(bytes);
        }
        self.header.extend_from_slice(bytes);
        if !self.try_header()? {
            return Ok(Vec::new());
        }
        let rest = std::mem::take(&mut self.header);
        self.feed(&rest)
    }

    /// `true`, sobald der Kopf vollständig gelesen ist; `self.header` enthält
    /// dann nur noch die Nutzdaten hinter dem Kopf.
    fn try_header(&mut self) -> Result<bool, TtsError> {
        let buf = &self.header;
        if buf.len() >= 4 && &buf[..4] != b"RIFF" || buf.len() >= 12 && &buf[8..12] != b"WAVE" {
            return Err(TtsError::backend(
                "Der Server liefert kein WAV, obwohl response_format=wav angefordert war",
            ));
        }
        match parse_wav(buf) {
            Ok(info) => {
                if info.format_tag != 1 || info.bits_per_sample != 16 {
                    return Err(TtsError::backend(format!(
                        "Der Server liefert WAV mit Format {} und {} Bit; erwartet wird 16-Bit-PCM",
                        info.format_tag, info.bits_per_sample
                    )));
                }
                self.rate = info.sample_rate;
                self.channels = info.channels;
                let at = info.data_offset;
                let declared =
                    u32::from_le_bytes([buf[at - 4], buf[at - 3], buf[at - 2], buf[at - 1]]);
                self.remaining = if declared == 0 || declared == u32::MAX {
                    None
                } else {
                    Some(u64::from(declared))
                };
                self.header.drain(..at);
                self.header_done = true;
                Ok(true)
            }
            // Noch nicht genug Bytes: weiter sammeln — aber nicht endlos.
            Err(_) if self.header.len() <= MAX_WAV_HEADER => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<u8>, TtsError> {
        if let Some(rem) = self.remaining.as_mut() {
            let take = (*rem).min(bytes.len() as u64) as usize;
            bytes = &bytes[..take];
            *rem -= take as u64;
        }
        self.total += bytes.len() as u64;
        if self.total > self.max_total {
            let limit = if self.max_total >= 1024 * 1024 {
                format!("{} MiB", self.max_total / (1024 * 1024))
            } else {
                format!("{} Byte", self.max_total)
            };
            return Err(TtsError::backend(format!(
                "Die Antwort überschreitet {limit} und wurde abgebrochen"
            )));
        }
        self.pending.extend_from_slice(bytes);
        let frame = self.frame();
        let whole = self.pending.len() - self.pending.len() % frame;
        Ok(self.pending.drain(..whole).collect())
    }

    /// Ende der Antwort: prüft, dass nichts Halbes übrig ist.
    pub fn finish(&mut self) -> Result<(), TtsError> {
        if !self.header_done {
            return Err(TtsError::backend(if self.header.is_empty() {
                "Der Server lieferte eine leere Antwort"
            } else {
                "Die Antwort endete vor dem Ende des WAV-Kopfes"
            }));
        }
        if let Some(r) = self.remaining {
            if r > 0 {
                return Err(TtsError::backend(format!(
                    "Die WAV-Antwort endet {r} Byte(s) vor der angekündigten Länge (abgeschnitten?)"
                )));
            }
        }
        if !self.pending.is_empty() {
            return Err(TtsError::backend(format!(
                "Die Antwort endet mitten in einem Sample ({} Byte(s) übrig; abgeschnittener Strom?)",
                self.pending.len()
            )));
        }
        Ok(())
    }
}

/// Liest die Antwort stückweise und liefert ausgerichtetes PCM.
struct PcmReader {
    resp: reqwest::Response,
    dec: PcmDecoder,
    done: bool,
}

impl PcmReader {
    /// Nächstes nicht leeres, frame-ausgerichtetes PCM-Stück; `None` am Ende.
    async fn next(&mut self) -> Result<Option<Vec<u8>>, TtsError> {
        loop {
            if self.done {
                return Ok(None);
            }
            match self.resp.chunk().await {
                Ok(Some(bytes)) => {
                    let pcm = self.dec.push(&bytes)?;
                    if !pcm.is_empty() {
                        return Ok(Some(pcm));
                    }
                }
                Ok(None) => {
                    self.done = true;
                    self.dec.finish()?;
                    return Ok(None);
                }
                Err(e) => {
                    self.done = true;
                    return Err(TtsError::network(format!(
                        "Die Verbindung zum Sprachserver brach während der Antwort ab: {e}"
                    )));
                }
            }
        }
    }

    fn format(&self) -> Option<(u32, u16)> {
        self.dec.format()
    }
}

// ---- Hilfen für Antworttexte ----------------------------------------------------------------

/// Antworttext auf eine Zeile und höchstens [`EXCERPT_CHARS`] Zeichen kürzen.
fn excerpt(raw: &str) -> String {
    let flat = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= EXCERPT_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(EXCERPT_CHARS).collect();
    format!("{cut}…")
}

/// Meldung aus einem Fehlerkörper: bei JSON `error.message`, `message`,
/// `detail` oder `error` (als Text), sonst der Rohtext.
fn body_message(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        let found = v["error"]["message"]
            .as_str()
            .or_else(|| v["message"].as_str())
            .or_else(|| v["detail"].as_str())
            .or_else(|| v["error"].as_str());
        if let Some(m) = found {
            return excerpt(m);
        }
    }
    excerpt(&text)
}

/// Sprachangabe eines Servers auf das Hauptkürzel bringen (`de-DE` → `de`).
fn normalize_language(raw: &str, default: &str) -> String {
    let primary = raw
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_alphabetic()) {
        primary
    } else {
        default.to_string()
    }
}

/// Stimmenliste tolerant lesen: `{"voices":[…]}` (auch `data`/`items`) oder
/// ein Array; Elemente als Text oder als Objekt (`id`/`voice_id`/`name`/`voice`,
/// `language`/`lang`/`locale`, `gender`).
pub(crate) fn parse_voices(value: &Value, default_lang: &str) -> Vec<VoiceDescriptor> {
    let items = match value {
        Value::Array(a) => Some(a),
        Value::Object(o) => ["voices", "data", "items"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_array)),
        _ => None,
    };
    let Some(items) = items else {
        return Vec::new();
    };
    let first_str = |o: &serde_json::Map<String, Value>, keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| match o.get(*k) {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Array(a)) => a
                .first()
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string()),
            _ => None,
        })
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let voice = match item {
            Value::String(s) if !s.trim().is_empty() => VoiceDescriptor {
                id: s.trim().to_string(),
                label: s.trim().to_string(),
                language: default_lang.to_string(),
                gender: None,
                license: None,
            },
            Value::Object(o) => {
                let Some(id) = first_str(o, &["id", "voice_id", "voiceId", "name", "voice"]) else {
                    continue;
                };
                VoiceDescriptor {
                    label: first_str(o, &["name", "label", "display_name", "title"])
                        .unwrap_or_else(|| id.clone()),
                    language: first_str(o, &["language", "lang", "locale"])
                        .map(|l| normalize_language(&l, default_lang))
                        .unwrap_or_else(|| default_lang.to_string()),
                    gender: first_str(o, &["gender"]),
                    license: None,
                    id,
                }
            }
            _ => continue,
        };
        if seen.insert(voice.id.clone()) {
            out.push(voice);
        }
    }
    out
}

/// Ergebnis einer JSON-Abfrage.
pub(crate) struct HttpReply {
    pub status: u16,
    pub json: Value,
    /// Gekürzter Antworttext (für Meldungen).
    pub text: String,
}

/// Wandelt einen Fehlerstatus (und den Auszug des Körpers) in einen Fehler,
/// bevor die allgemeine Zuordnung greift (Ollama: 404 heißt dort „Modell
/// oder Version fehlt“).
pub(crate) type StatusMapper = fn(u16, &str) -> Option<TtsError>;

struct Setup {
    base: String,
    client: reqwest::Client,
}

// ---- Provider ----------------------------------------------------------------------

/// Generischer Sprachserver-Adapter (siehe Moduldoku).
pub struct OpenAiSpeechProvider {
    cfg: OpenAiSpeechConfig,
    setup: Result<Setup, TtsError>,
    license_override: Option<LicenseInfo>,
    status_mapper: Option<StatusMapper>,
    health_timeout: Duration,
    max_audio_bytes: u64,
}

impl OpenAiSpeechProvider {
    /// Anbieter aus einer Konfiguration. Eine ungültige Konfiguration führt
    /// nicht zu einem Fehler hier, sondern bei jeder Benutzung (Health:
    /// „nicht verfügbar“ mit Grund; Synthese: `InvalidConfig` bzw.
    /// `PrivacyBlocked`). Wer früher prüfen will: [`OpenAiSpeechConfig::validate`].
    pub fn new(cfg: OpenAiSpeechConfig) -> Self {
        let setup = Self::build_setup(&cfg);
        Self {
            cfg,
            setup,
            license_override: None,
            status_mapper: None,
            health_timeout: HEALTH_TIMEOUT,
            max_audio_bytes: MAX_AUDIO_BYTES,
        }
    }

    /// Wie [`new`](Self::new), aber mit sofortiger Prüfung.
    pub fn try_new(cfg: OpenAiSpeechConfig) -> Result<Self, TtsError> {
        let p = Self::new(cfg);
        p.setup()?;
        Ok(p)
    }

    pub fn config(&self) -> &OpenAiSpeechConfig {
        &self.cfg
    }

    fn build_setup(cfg: &OpenAiSpeechConfig) -> Result<Setup, TtsError> {
        let parsed = cfg.parse()?;
        let mut builder = reqwest::Client::builder()
            // Keine Umleitungen: der Text darf nur dorthin, wo die Konfiguration hinzeigt.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3));
        if parsed.loopback {
            // Lokale Server nie über einen Proxy ansprechen (der sähe den Text).
            builder = builder.no_proxy();
        }
        let client = builder
            .build()
            .map_err(|e| TtsError::backend(format!("HTTP-Client nicht erstellbar: {e}")))?;
        Ok(Setup {
            base: parsed.base,
            client,
        })
    }

    fn setup(&self) -> Result<&Setup, TtsError> {
        self.setup.as_ref().map_err(Clone::clone)
    }

    /// Ist die Konfiguration benutzbar (gültig, Datenschutz-Sperre offen)?
    pub(crate) fn check_config(&self) -> Result<(), TtsError> {
        self.setup().map(|_| ())
    }

    /// Lizenz des Anbieters festlegen (nur für fest eingebaute Ableitungen
    /// wie Ollama; Nutzereingaben bleiben unverifiziert).
    pub(crate) fn with_license_override(mut self, license: LicenseInfo) -> Self {
        self.license_override = Some(license);
        self
    }

    pub(crate) fn with_status_mapper(mut self, mapper: StatusMapper) -> Self {
        self.status_mapper = Some(mapper);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_health_timeout(mut self, t: Duration) -> Self {
        self.health_timeout = t;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_max_audio_bytes(mut self, n: u64) -> Self {
        self.max_audio_bytes = n;
        self
    }

    fn label(&self) -> String {
        let l = self.cfg.label.trim();
        if l.is_empty() {
            self.cfg.id.clone()
        } else {
            l.to_string()
        }
    }

    fn configured_voice(&self) -> Option<&str> {
        self.cfg
            .voice
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    /// `opts.voice` vor der Konfiguration.
    fn voice_for<'a>(&'a self, opts: &'a SynthesisOptions) -> Option<&'a str> {
        opts.voice
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .or_else(|| self.configured_voice())
    }

    fn voice_language(&self) -> String {
        if self.cfg.languages.len() == 1 {
            normalize_language(&self.cfg.languages[0], "und")
        } else {
            "und".to_string()
        }
    }

    /// Schlüssel aus der Umgebungsvariablen (nur lesen, nie loggen).
    fn api_key(&self) -> Result<Option<String>, TtsError> {
        let Some(name) = self
            .cfg
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            return Ok(None);
        };
        match std::env::var(name) {
            Ok(v) if !v.trim().is_empty() => Ok(Some(v.trim().to_string())),
            _ => Err(invalid(format!(
                "Die Umgebungsvariable {name} (api_key_env) ist nicht gesetzt oder leer"
            ))),
        }
    }

    fn with_auth(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, TtsError> {
        let Some(key) = self.api_key()? else {
            return Ok(req);
        };
        let mut value = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| invalid("Der API-Schlüssel enthält unzulässige Zeichen"))?;
        value.set_sensitive(true);
        Ok(req.header(AUTHORIZATION, value))
    }

    fn send_error(&self, e: reqwest::Error) -> TtsError {
        let base = self.setup.as_ref().map(|s| s.base.as_str()).unwrap_or("?");
        if e.is_timeout() {
            TtsError::network(format!(
                "Der Sprachserver {base} antwortet nicht innerhalb der Frist"
            ))
        } else {
            TtsError::network(format!(
                "Verbindung zum Sprachserver {base} fehlgeschlagen: {e}"
            ))
        }
    }

    /// Höchstens `cap` Bytes der Antwort lesen.
    async fn read_limited(resp: &mut reqwest::Response, cap: usize) -> Result<Vec<u8>, TtsError> {
        let mut buf = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(c)) => {
                    buf.extend_from_slice(&c);
                    if buf.len() >= cap {
                        buf.truncate(cap);
                        return Ok(buf);
                    }
                }
                Ok(None) => return Ok(buf),
                Err(e) => return Err(TtsError::network(format!("Antwort nicht lesbar: {e}"))),
            }
        }
    }

    /// Eine JSON-Anfrage mit eigener Frist (Health, Stimmen, Ollama-Abfragen).
    pub(crate) async fn http_json(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<HttpReply, TtsError> {
        let setup = self.setup()?;
        let mut req = setup.client.request(method, url).timeout(timeout);
        req = self.with_auth(req)?;
        if let Some(b) = body {
            req = req.json(b);
        }
        let mut resp = req.send().await.map_err(|e| self.send_error(e))?;
        let status = resp.status().as_u16();
        let bytes = Self::read_limited(&mut resp, JSON_BODY_CAP).await?;
        Ok(HttpReply {
            status,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            text: body_message(&bytes),
        })
    }

    /// Fehlerstatus der Sprachanfrage in einen Fehler übersetzen.
    async fn map_status(&self, mut resp: reqwest::Response) -> TtsError {
        let code = resp.status().as_u16();
        let bytes = Self::read_limited(&mut resp, 4096)
            .await
            .unwrap_or_default();
        let mut msg = body_message(&bytes);
        if let Ok(Some(key)) = self.api_key() {
            msg = msg.replace(&key, "***");
        }
        if let Some(mapper) = self.status_mapper {
            if let Some(e) = mapper(code, &msg) {
                return e;
            }
        }
        let with = |prefix: String| {
            if msg.is_empty() {
                prefix
            } else {
                format!("{prefix}: {msg}")
            }
        };
        match code {
            401 | 403 => invalid(with(format!(
                "Der Sprachserver lehnt die Anmeldung ab (HTTP {code}); api_key_env und Schlüssel prüfen"
            ))),
            408 => TtsError::network(with(format!("Der Sprachserver meldet eine Zeitüberschreitung (HTTP {code})"))),
            429 => TtsError::backend(with(format!("Der Sprachserver ist überlastet (HTTP {code})")))
                .with_retryable(true),
            300..=399 => invalid(with(format!(
                "Der Sprachserver leitet um (HTTP {code}); Basis-URL prüfen. Umleitungen werden aus Datenschutzgründen nicht verfolgt"
            ))),
            500..=599 => TtsError::backend(with(format!("Der Sprachserver meldet einen Fehler (HTTP {code})")))
                .with_retryable(true),
            _ => TtsError::bad_request(with(format!(
                "Der Sprachserver lehnt die Anfrage ab (HTTP {code})"
            ))),
        }
    }

    /// Anfrage stellen und die Antwort als [`PcmReader`] öffnen.
    async fn open(
        &self,
        text: &str,
        voice: Option<&str>,
        wire: Wire,
    ) -> Result<PcmReader, TtsError> {
        let setup = self.setup()?;
        let url = format!("{}/audio/speech", setup.base);
        // `speed` wird bewusst nie gesendet (siehe Moduldoku).
        let mut body = json!({
            "model": self.cfg.model.trim(),
            "input": text,
            "response_format": wire.name(),
        });
        if let Some(v) = voice {
            body["voice"] = json!(v);
        }
        let req = self.with_auth(setup.client.post(&url).json(&body))?;
        let resp = req.send().await.map_err(|e| self.send_error(e))?;
        if !resp.status().is_success() {
            return Err(self.map_status(resp).await);
        }
        let ct = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ct.starts_with("application/json") || ct.starts_with("text/") {
            let mut resp = resp;
            let bytes = Self::read_limited(&mut resp, 4096)
                .await
                .unwrap_or_default();
            return Err(TtsError::backend(format!(
                "Der Sprachserver lieferte {ct} statt Audio: {}",
                body_message(&bytes)
            )));
        }
        Ok(PcmReader {
            resp,
            dec: PcmDecoder::new(wire, self.cfg.sample_rate, self.max_audio_bytes),
            done: false,
        })
    }

    /// Kurzprobe: liefert der Server für `text` wirklich Audio? Gibt die
    /// Länge des ersten PCM-Stücks zurück.
    pub(crate) async fn probe(&self, text: &str, timeout: Duration) -> Result<usize, TtsError> {
        let voice = self.configured_voice().map(String::from);
        let run = async {
            let mut reader = self.open(text, voice.as_deref(), Wire::Pcm).await?;
            match reader.next().await? {
                Some(pcm) => Ok(pcm.len()),
                None => Err(TtsError::backend("Der Server lieferte kein Audio")),
            }
        };
        match tokio::time::timeout(timeout, run).await {
            Ok(r) => r,
            Err(_) => Err(TtsError::timeout(format!(
                "Keine Antwort auf die Probe nach {} s",
                timeout.as_secs()
            ))),
        }
    }

    fn check_text(text: &str) -> Result<(), TtsError> {
        if text.trim().is_empty() {
            return Err(TtsError::bad_request("Leerer Text"));
        }
        let n = text.chars().count();
        if n > MAX_INPUT_CHARS {
            return Err(TtsError::bad_request(format!(
                "Text zu lang für den Sprachserver ({n} von höchstens {MAX_INPUT_CHARS} Zeichen)"
            )));
        }
        Ok(())
    }

    fn unavailable_health(&self, reason: String, hint: Option<String>) -> ProviderHealth {
        let mut h = ProviderHealth::unavailable(reason, hint);
        h.device = Some(Device::Remote);
        h
    }

    async fn health(&self) -> ProviderHealth {
        let setup = match self.setup() {
            Ok(s) => s,
            Err(e) => {
                return self.unavailable_health(
                    e.message,
                    Some("Einstellungen des Sprachservers prüfen.".to_string()),
                )
            }
        };
        let base = setup.base.clone();
        let deadline = Instant::now() + self.health_timeout;
        for path in ["/audio/voices", "/models"] {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let reply = self
                .http_json(Method::GET, &format!("{base}{path}"), None, remaining)
                .await;
            match reply {
                Ok(r) if (200..300).contains(&r.status) => {
                    let mut h = ProviderHealth::ok();
                    h.device = Some(Device::Remote);
                    return h;
                }
                Ok(r) if r.status == 401 || r.status == 403 => {
                    return self.unavailable_health(
                        format!("Der Sprachserver {base} lehnt die Anmeldung ab (HTTP {})", r.status),
                        Some("api_key_env eintragen und die Umgebungsvariable mit dem gültigen Schlüssel setzen.".to_string()),
                    )
                }
                // Dieser Pfad ist dem Server unbekannt: den nächsten versuchen.
                Ok(r) if r.status == 404 || r.status == 405 => continue,
                Ok(r) => {
                    return self.unavailable_health(
                        format!("Der Sprachserver {base} meldet HTTP {}", r.status),
                        Some("Serverprotokoll prüfen.".to_string()),
                    )
                }
                Err(e) => {
                    return self.unavailable_health(
                        e.message,
                        Some(format!(
                            "Läuft der Sprachserver? Basis-URL und Port prüfen ({base})."
                        )),
                    )
                }
            }
        }
        if Instant::now() >= deadline {
            return self.unavailable_health(
                format!(
                    "Der Sprachserver {base} antwortet nicht innerhalb von {} s",
                    self.health_timeout.as_secs()
                ),
                Some("Läuft der Sprachserver? Basis-URL und Port prüfen.".to_string()),
            );
        }
        self.unavailable_health(
            format!("Der Server {base} ist erreichbar, kennt aber weder /audio/voices noch /models (HTTP 404)"),
            Some("Basis-URL prüfen: sie endet meist auf /v1 (z. B. http://127.0.0.1:8880/v1).".to_string()),
        )
    }

    /// Stimmen vom Server; bei jedem Problem nur die konfigurierte Stimme.
    async fn voices(&self, language: Option<&str>) -> Result<Vec<VoiceDescriptor>, TtsError> {
        let setup = self.setup()?;
        let lang = self.voice_language();
        let url = format!("{}/audio/voices", setup.base);
        let mut voices = match self
            .http_json(Method::GET, &url, None, self.health_timeout)
            .await
        {
            Ok(r) if (200..300).contains(&r.status) => parse_voices(&r.json, &lang),
            _ => Vec::new(),
        };
        if voices.is_empty() {
            if let Some(v) = self.configured_voice() {
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

    fn license(&self) -> LicenseInfo {
        if let Some(l) = &self.license_override {
            return l.clone();
        }
        let user = self.cfg.license.clone().unwrap_or_default();
        let or = |s: &str, default: &str| {
            if s.trim().is_empty() {
                default.to_string()
            } else {
                s.trim().to_string()
            }
        };
        LicenseInfo {
            code: or(&user.code, "unbekannt"),
            weights: or(&user.weights, "unbekannt (hängt am Server und Modell)"),
            non_commercial: user.non_commercial,
            // Nutzerangaben sind nie geprüft.
            verified: false,
            source_url: or(&user.source_url, DEFAULT_LICENSE_URL),
            verified_on: None,
            note: or(
                &user.note,
                "Lizenz des Servers und der Gewichte nicht geprüft: vor dem Einsatz die Lizenz des betriebenen Servers und Modells nachlesen (Recherche: docs/tts/provider-research.md, D).",
            ),
        }
    }

    /// Chunk in der angeforderten Kodierung: Rohdaten unverändert, `Wav` als
    /// kleiner eigener Container je Chunk (Rate und Kanäle aus dem Strom).
    fn make_chunk(
        encoding: AudioEncoding,
        rate: u32,
        channels: u16,
        pcm: Vec<u8>,
        meta: ChunkMeta,
    ) -> Result<AudioChunk, TtsError> {
        let data = match encoding {
            AudioEncoding::Wav => wav_from_pcm(AudioEncoding::Pcm16Le, rate, channels, &pcm)?,
            _ => pcm,
        };
        Ok(AudioChunk {
            request_id: meta.request_id,
            seq: meta.seq,
            format: AudioFormat {
                sample_rate: rate,
                channels,
                encoding,
            },
            data,
            is_last: meta.is_last,
            elapsed_ms: meta.started.elapsed().as_millis() as u64,
        })
    }
}

/// Kennzeichen eines Chunks (Reihenfolge, Auftrag, Zeit).
struct ChunkMeta {
    request_id: String,
    seq: u32,
    is_last: bool,
    started: Instant,
}

impl TtsProvider for OpenAiSpeechProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.cfg.id.clone(),
            label: self.label(),
            tier: ProviderTier::Experimental,
            opt_in: true,
            caps: ProviderCaps {
                languages: self
                    .cfg
                    .languages
                    .iter()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect(),
                streaming: StreamingMode::Chunk,
                formats: vec![AudioEncoding::Pcm16Le, AudioEncoding::Wav],
                sample_rates: vec![self.cfg.sample_rate],
                cloning: false,
                // Außerhalb des App-Prozesses (auch bei Loopback): wo der Server
                // rechnet, weiß der Adapter nicht.
                devices: vec![Device::Remote],
                platforms: ["windows-x64", "linux-x64", "macos-aarch64", "macos-x64"]
                    .map(String::from)
                    .to_vec(),
                style_tags: false,
            },
            license: self.license(),
        }
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(self.health())
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
            self.setup()?;
            Self::check_text(text)?;
            let wire = if opts.format == AudioEncoding::Wav {
                Wire::Wav
            } else {
                Wire::Pcm
            };
            let voice = self.voice_for(opts);
            guarded(cancel, opts.timeout, &self.cfg.id, async {
                let mut reader = self.open(text, voice, wire).await?;
                let mut pcm: Vec<u8> = Vec::new();
                while let Some(part) = reader.next().await? {
                    pcm.extend_from_slice(&part);
                }
                let (rate, channels) = reader
                    .format()
                    .ok_or_else(|| TtsError::backend("Der Sprachserver lieferte kein Audio"))?;
                if pcm.is_empty() {
                    return Err(TtsError::backend("Der Sprachserver lieferte kein Audio"));
                }
                AudioClip::from_pcm(AudioEncoding::Pcm16Le, rate, channels, &pcm)
            })
            .await
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
            self.setup()?;
            Self::check_text(text)?;
            let wire = match opts.format {
                AudioEncoding::Pcm16Le => Wire::Pcm,
                AudioEncoding::Wav => Wire::Wav,
                AudioEncoding::F32Le => {
                    return Err(TtsError::bad_request(
                        "Der Sprachserver liefert 16-Bit-PCM: als Stream-Format pcm16_le oder wav wählen, nicht f32_le",
                    ))
                }
            };
            let voice = self.voice_for(opts);
            let started = Instant::now();
            guarded(cancel, opts.timeout, &self.cfg.id, async {
                let mut reader = self.open(text, voice, wire).await?;
                // Das letzte Frame bleibt zurück, damit der Schluss-Chunk
                // `is_last` tragen kann, ohne dass ein Chunk wartet.
                let mut held: Vec<u8> = Vec::new();
                let mut seq = 0u32;
                while let Some(pcm) = reader.next().await? {
                    let (rate, channels) = reader.format().unwrap_or((self.cfg.sample_rate, 1));
                    let frame = 2 * usize::from(channels);
                    held.extend_from_slice(&pcm);
                    // `held` ist frame-genau und hat mindestens ein Frame.
                    let keep = held.split_off(held.len() - frame);
                    let ready = std::mem::replace(&mut held, keep);
                    if !ready.is_empty() {
                        let chunk = Self::make_chunk(
                            opts.format,
                            rate,
                            channels,
                            ready,
                            ChunkMeta {
                                request_id: cancel.request_id().to_string(),
                                seq,
                                is_last: false,
                                started,
                            },
                        )?;
                        send_chunk(&sink, chunk, cancel).await?;
                        seq += 1;
                    }
                }
                if held.is_empty() {
                    return Err(TtsError::backend("Der Sprachserver lieferte kein Audio"));
                }
                let (rate, channels) = reader.format().unwrap_or((self.cfg.sample_rate, 1));
                let chunk = Self::make_chunk(
                    opts.format,
                    rate,
                    channels,
                    held,
                    ChunkMeta {
                        request_id: cancel.request_id().to_string(),
                        seq,
                        is_last: true,
                        started,
                    },
                )?;
                send_chunk(&sink, chunk, cancel).await
            })
            .await
        })
    }

    fn cache_tag(&self, voice: Option<&str>) -> String {
        let voice = voice
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .or_else(|| self.configured_voice())
            .unwrap_or("default");
        format!("{}:{}:{}", self.cfg.id, self.cfg.model.trim(), voice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract_tests::{contract_suite, ContractExpect};
    use crate::managers::tts::providers::test_http::{
        speech_reply, split_by, tone_pcm, wav_bytes, FakeServer, Reply, Req, StreamEnd,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn cfg_for(server: &FakeServer) -> OpenAiSpeechConfig {
        OpenAiSpeechConfig {
            base_url: format!("{}/v1", server.root),
            model: "test-modell".to_string(),
            voice: Some("ava".to_string()),
            ..Default::default()
        }
    }

    fn opts() -> SynthesisOptions {
        SynthesisOptions {
            timeout: Duration::from_secs(20),
            ..Default::default()
        }
    }

    fn pcm_opts() -> SynthesisOptions {
        SynthesisOptions {
            format: AudioEncoding::Pcm16Le,
            ..opts()
        }
    }

    /// Server mit Stimmenliste und Sprachausgabe; sonst 404.
    async fn speech_server() -> FakeServer {
        FakeServer::start(|req: &Req| match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/v1/audio/voices") => Reply::json(
                200,
                json!({"voices": [
                    {"id": "ava", "name": "Ava", "language": "de-DE", "gender": "female"},
                    {"id": "ben", "name": "Ben", "language": "en"},
                    "chloe"
                ]}),
            ),
            ("POST", "/v1/audio/speech") => speech_reply(req, Duration::from_millis(1)),
            _ => Reply::json(404, json!({"detail": "Not Found"})),
        })
        .await
    }

    async fn collect(
        p: &OpenAiSpeechProvider,
        text: &str,
        o: &SynthesisOptions,
        cancel: &CancelToken,
    ) -> (Result<(), TtsError>, Vec<AudioChunk>) {
        let (tx, mut rx) = mpsc::channel(8);
        let produce = p.stream_synthesize(text, o, cancel, tx);
        let consume = async {
            let mut v = Vec::new();
            while let Some(c) = rx.recv().await {
                v.push(c);
            }
            v
        };
        futures_util::future::join(produce, consume).await
    }

    // ---- Vertrag ------------------------------------------------------------

    #[tokio::test]
    async fn besteht_den_vertrag_gegen_den_fake_server_inklusive_abbruch_und_frist() {
        let server = speech_server().await;
        let hang = FakeServer::start(|req: &Req| {
            if req.path == "/v1/audio/speech" {
                Reply::Hang
            } else {
                Reply::json(200, json!({"voices": []}))
            }
        })
        .await;
        contract_suite(
            Arc::new(OpenAiSpeechProvider::new(cfg_for(&server))),
            ContractExpect::available()
                .with_sample_rate(24_000)
                .with_slow(Arc::new(OpenAiSpeechProvider::new(cfg_for(&hang)))),
        )
        .await;
        assert!(server.count("/v1/audio/speech") > 3);
    }

    #[tokio::test]
    async fn server_nicht_erreichbar_ist_nicht_verfuegbar_mit_grund_und_wiederholbarer_fehler() {
        // Port, auf dem nichts lauscht.
        let port = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
        };
        let p = OpenAiSpeechProvider::new(OpenAiSpeechConfig {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "m".into(),
            voice: Some("ava".into()),
            ..Default::default()
        });
        let h = p.health_check().await;
        assert!(!h.available);
        assert_eq!(h.device, Some(Device::Remote));
        assert!(
            h.reason.as_deref().unwrap().contains("fehlgeschlagen"),
            "{h:?}"
        );
        assert!(h
            .hint
            .as_deref()
            .unwrap()
            .contains("Läuft der Sprachserver"));
        // Laut Briefing: Verbindungsfehler sind Network und wiederholbar.
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Network);
        assert!(e.retryable);
        // Die Stimmenliste fällt auf die konfigurierte Stimme zurück, statt zu scheitern.
        let v = p.list_voices(None).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "ava");
    }

    // ---- Datenschutz --------------------------------------------------------

    #[test]
    fn loopback_wird_genau_erkannt() {
        for ok in [
            "http://127.0.0.1:8880/v1",
            "http://127.5.6.7/v1",
            "http://localhost:11434",
            "http://LOCALHOST/v1",
            "http://[::1]:8880/v1",
            "http://[::ffff:127.0.0.1]:8880/v1",
            "https://localhost:8443/v1",
        ] {
            assert!(is_loopback_url(ok), "{ok}");
        }
        for no in [
            "http://192.0.2.1:8880/v1",
            "http://0.0.0.0:8880/v1",
            "https://api.example.com/v1",
            "http://localhost.evil.example/v1",
            "http://127.0.0.1.evil.example/v1",
            "http://[::2]/v1",
            "http://10.0.0.5/v1",
            "kein url",
            "",
        ] {
            assert!(!is_loopback_url(no), "{no}");
        }
    }

    #[tokio::test]
    async fn fremder_host_ohne_freigabe_ist_privacy_blocked_ohne_netzzugriff() {
        for url in ["http://192.0.2.1:8880/v1", "https://api.example.com/v1"] {
            let cfg = OpenAiSpeechConfig {
                base_url: url.into(),
                model: "m".into(),
                ..Default::default()
            };
            let e = cfg.validate().unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked, "{url}");
            assert!(!e.retryable);
            assert!(e.message.contains("allow_remote"));
            let p = OpenAiSpeechProvider::new(cfg.clone());
            let e = p
                .synthesize("Hallo", &opts(), &CancelToken::detached())
                .await
                .unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked);
            let (r, got) = collect(&p, "Hallo", &pcm_opts(), &CancelToken::detached()).await;
            assert_eq!(r.unwrap_err().kind, TtsErrorKind::PrivacyBlocked);
            assert!(got.is_empty());
            let e = p.list_voices(None).await.unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked);
            let h = p.health_check().await;
            assert!(!h.available && h.reason.unwrap().contains("allow_remote"));
            // Mit Freigabe ist die Konfiguration gültig (es wird nichts gesendet).
            let mut allowed = cfg;
            allowed.allow_remote = true;
            allowed.validate().unwrap();
        }
    }

    #[tokio::test]
    async fn umleitungen_werden_nicht_verfolgt() {
        let fremd = FakeServer::start(|_| Reply::audio(vec![0; 4])).await;
        let target = format!("{}/geheim", fremd.root);
        let server = FakeServer::start(move |_| Reply::redirect(&target)).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig, "{e:?}");
        assert!(e.message.contains("Umleitungen"));
        assert!(fremd.requests().is_empty(), "der Redirect wurde verfolgt");
    }

    // ---- Anfrage ------------------------------------------------------------

    #[tokio::test]
    async fn anfrage_enthaelt_modell_text_stimme_und_format_aber_kein_speed() {
        let server = speech_server().await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let mut o = opts();
        o.speed = Some(1.5);
        p.synthesize("Sag \"Hallo\" – äöü", &o, &CancelToken::detached())
            .await
            .unwrap();
        o.format = AudioEncoding::Pcm16Le;
        o.voice = Some("ben".into());
        p.synthesize("Zwei", &o, &CancelToken::detached())
            .await
            .unwrap();
        let reqs: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.path == "/v1/audio/speech")
            .collect();
        assert_eq!(reqs.len(), 2);
        let a = reqs[0].json();
        assert_eq!(a["model"], "test-modell");
        assert_eq!(a["input"], "Sag \"Hallo\" – äöü");
        assert_eq!(a["voice"], "ava");
        assert_eq!(a["response_format"], "wav", "Wav → wav");
        assert!(a.get("speed").is_none(), "speed darf nie gesendet werden");
        assert_eq!(reqs[0].headers["content-type"], "application/json");
        assert!(!reqs[0].headers.contains_key("authorization"));
        let b = reqs[1].json();
        assert_eq!(b["voice"], "ben", "opts.voice schlägt die Konfiguration");
        assert_eq!(b["response_format"], "pcm");

        // Ohne Stimme wird das Feld weggelassen.
        let mut cfg = cfg_for(&server);
        cfg.voice = None;
        let p = OpenAiSpeechProvider::new(cfg);
        p.synthesize("Drei", &opts(), &CancelToken::detached())
            .await
            .unwrap();
        let last = server.requests().pop().unwrap().json();
        assert!(last.get("voice").is_none());
    }

    #[tokio::test]
    async fn schluessel_kommt_aus_der_umgebung_und_taucht_nirgends_auf() {
        const VAR: &str = "LVA_TEST_P2_OAI_KEY_A";
        std::env::set_var(VAR, "sk-geheim-12345");
        let server = FakeServer::start(|req: &Req| {
            if req.path == "/v1/audio/speech"
                && req.headers.get("authorization").map(String::as_str)
                    == Some("Bearer sk-geheim-12345")
            {
                // Der Server schreibt den Schlüssel in seine Fehlermeldung zurück.
                Reply::json(
                    400,
                    json!({"error": {"message": "bad key sk-geheim-12345 echo"}}),
                )
            } else {
                Reply::json(401, json!({"error": "no key"}))
            }
        })
        .await;
        let mut cfg = cfg_for(&server);
        cfg.api_key_env = Some(VAR.to_string());
        let p = OpenAiSpeechProvider::new(cfg);
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest, "{e:?}");
        assert!(
            !e.message.contains("sk-geheim"),
            "Schlüssel im Fehlertext: {}",
            e.message
        );
        assert!(e.message.contains("***"));
        let req = &server.requests()[0];
        assert_eq!(req.headers["authorization"], "Bearer sk-geheim-12345");
        // Weder Konfiguration noch Deskriptor enthalten den Schlüssel.
        let all = format!("{:?}{:?}", p.config(), p.descriptor());
        assert!(!all.contains("sk-geheim"));

        // Fehlt die Variable: InvalidConfig mit dem Namen, kein Request.
        std::env::remove_var(VAR);
        let before = server.requests().len();
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
        assert!(e.message.contains(VAR));
        assert_eq!(server.requests().len(), before);
        let h = p.health_check().await;
        assert!(!h.available && h.reason.unwrap().contains(VAR));
    }

    #[tokio::test]
    async fn health_meldet_abgelehnte_anmeldung_mit_hinweis() {
        let server =
            FakeServer::start(|_| Reply::json(401, json!({"error": "unauthorized"}))).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("Anmeldung"));
        assert!(h.hint.unwrap().contains("api_key_env"));
    }

    // ---- Fehlerklassen -------------------------------------------------------

    #[tokio::test]
    async fn http_fehler_werden_nach_klasse_zugeordnet() {
        let long = "x".repeat(1000);
        for (status, body, kind, retryable) in [
            (
                400,
                json!({"error": {"message": "unsupported voice"}}),
                TtsErrorKind::BadRequest,
                false,
            ),
            (
                404,
                json!({"detail": "Model not found"}),
                TtsErrorKind::BadRequest,
                false,
            ),
            (
                422,
                json!({"detail": long.clone()}),
                TtsErrorKind::BadRequest,
                false,
            ),
            (413, json!({}), TtsErrorKind::BadRequest, false),
            (
                401,
                json!({"error": "no"}),
                TtsErrorKind::InvalidConfig,
                false,
            ),
            (
                403,
                json!({"error": "no"}),
                TtsErrorKind::InvalidConfig,
                false,
            ),
            (500, json!({"error": "boom"}), TtsErrorKind::Backend, true),
            (503, json!({}), TtsErrorKind::Backend, true),
            (429, json!({}), TtsErrorKind::Backend, true),
            (408, json!({}), TtsErrorKind::Network, true),
        ] {
            let body2 = body.clone();
            let server = FakeServer::start(move |_| Reply::json(status, body2.clone())).await;
            let p = OpenAiSpeechProvider::new(cfg_for(&server));
            let e = p
                .synthesize("Hallo", &opts(), &CancelToken::detached())
                .await
                .unwrap_err();
            assert_eq!(e.kind, kind, "HTTP {status}: {e:?}");
            assert_eq!(e.retryable, retryable, "HTTP {status}");
            assert!(e.message.contains(&status.to_string()), "{}", e.message);
            // Streams verhalten sich gleich und liefern keinen Chunk.
            let (r, got) = collect(&p, "Hallo", &pcm_opts(), &CancelToken::detached()).await;
            assert_eq!(r.unwrap_err().kind, kind);
            assert!(got.is_empty());
        }
        // Auszug: höchstens 300 Zeichen plus Auslassung, Serverwort kommt an.
        let server =
            FakeServer::start(move |_| Reply::json(422, json!({"detail": long.clone()}))).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        let xs = e.message.chars().filter(|c| *c == 'x').count();
        assert_eq!(xs, 300, "Auszug auf 300 Zeichen gekürzt");
        assert!(e.message.ends_with('…'));

        let server = FakeServer::start(|_| {
            Reply::json(400, json!({"error": {"message": "unsupported voice 'zz'"}}))
        })
        .await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert!(
            e.message.contains("unsupported voice 'zz'"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn verbindung_ohne_antwort_ist_ein_wiederholbarer_netzfehler() {
        let server = FakeServer::start(|_| Reply::Close).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Network, "{e:?}");
        assert!(e.retryable);
    }

    #[tokio::test]
    async fn json_oder_text_statt_audio_ist_ein_backend_fehler() {
        let server = FakeServer::start(|_| Reply::json(200, json!({"hello": "world"}))).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("statt Audio"));
        assert!(!e.retryable);
    }

    #[tokio::test]
    async fn leere_oder_kaputte_audioantworten_sind_backend_fehler() {
        // Leere Antwort in beiden Formaten.
        for fmt in [AudioEncoding::Pcm16Le, AudioEncoding::Wav] {
            let server = FakeServer::start(|_| Reply::audio(Vec::new())).await;
            let p = OpenAiSpeechProvider::new(cfg_for(&server));
            let mut o = opts();
            o.format = fmt;
            let e = p
                .synthesize("Hallo", &o, &CancelToken::detached())
                .await
                .unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::Backend, "{fmt:?}: {e:?}");
        }
        // Müll statt WAV.
        let server =
            FakeServer::start(|_| Reply::audio(b"das ist kein wav, sondern text".to_vec())).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("kein WAV"), "{}", e.message);
        // Halbes Sample am Ende (ungerade Byte-Zahl) in Rohdaten.
        let server = FakeServer::start(|_| Reply::audio(vec![1, 2, 3])).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let e = p
            .synthesize("Hallo", &pcm_opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("Sample"));
    }

    // ---- Streaming ------------------------------------------------------------

    #[tokio::test]
    async fn stream_liefert_ausgerichtete_chunks_die_zusammen_das_audio_ergeben() {
        let server = speech_server().await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let text = "Ein Satz mit etwas Länge.";
        let expected = tone_pcm(24 * 40 * text.chars().count());
        for fmt in [AudioEncoding::Pcm16Le, AudioEncoding::Wav] {
            let mut o = opts();
            o.format = fmt;
            let cancel = CancelToken::new("req-7");
            let (r, got) = collect(&p, text, &o, &cancel).await;
            r.unwrap();
            assert!(got.len() >= 3, "{fmt:?}: {} Chunks", got.len());
            let mut joined = Vec::new();
            for (i, c) in got.iter().enumerate() {
                assert_eq!(c.seq, i as u32);
                assert_eq!(c.request_id, "req-7");
                assert_eq!(c.is_last, i + 1 == got.len());
                assert_eq!(c.format.encoding, fmt);
                assert_eq!((c.format.sample_rate, c.format.channels), (24_000, 1));
                let pcm = if fmt == AudioEncoding::Wav {
                    let info = parse_wav(&c.data).unwrap();
                    c.data[info.data_offset..info.data_offset + info.data_len].to_vec()
                } else {
                    c.data.clone()
                };
                assert_eq!(pcm.len() % 2, 0, "Chunk {i} endet im halben Sample");
                assert!(!pcm.is_empty());
                joined.extend(pcm);
            }
            assert_eq!(joined, expected, "{fmt:?}: Chunks ergeben nicht das Audio");
        }
        // F32 gibt es nicht.
        let mut o = opts();
        o.format = AudioEncoding::F32Le;
        let (r, got) = collect(&p, text, &o, &CancelToken::detached()).await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::BadRequest);
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn synthesize_ergibt_ein_sauberes_wav_auch_bei_streaming_kopf() {
        let server = speech_server().await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let text = "Zehn Zeich";
        for fmt in [AudioEncoding::Wav, AudioEncoding::Pcm16Le] {
            let mut o = opts();
            o.format = fmt;
            let clip = p
                .synthesize(text, &o, &CancelToken::detached())
                .await
                .unwrap();
            let info = parse_wav(&clip.data).unwrap();
            // Der Kopf des Ergebnisses trägt echte Längen, nicht 0xFFFFFFFF.
            assert_eq!(
                u32::from_le_bytes(clip.data[40..44].try_into().unwrap()) as usize,
                info.data_len
            );
            assert_eq!(clip.duration_ms, 400, "{fmt:?}");
            assert_eq!(&clip.data[44..], &tone_pcm(24 * 40 * 10)[..]);
        }
    }

    #[tokio::test]
    async fn abgerissener_strom_ist_ein_netzfehler_ohne_ende_chunk() {
        let pcm = tone_pcm(5000);
        let server = FakeServer::start(move |_| {
            Reply::stream(
                split_by(&pcm, &[999, 1001]),
                Duration::from_millis(2),
                StreamEnd::Abort,
            )
        })
        .await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let (r, got) = collect(&p, "Hallo", &pcm_opts(), &CancelToken::detached()).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Network, "{e:?}");
        assert!(e.retryable);
        assert!(!got.is_empty(), "vor dem Abriss kamen Teile an");
        assert!(
            got.iter().all(|c| !c.is_last),
            "kein Ende-Chunk nach Abriss"
        );
        // synthesize liefert keinen halben Clip.
        let e = p
            .synthesize("Hallo", &pcm_opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Network);
    }

    #[tokio::test]
    async fn abbruch_schliesst_die_verbindung_und_beendet_den_stream() {
        // Server sendet ein Stück und hält dann die Verbindung offen.
        let server = FakeServer::start(|_| {
            Reply::stream(vec![vec![0u8; 2000]], Duration::ZERO, StreamEnd::Stall)
        })
        .await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let cancel = CancelToken::new("abbruch");
        let c2 = cancel.clone();
        let canceller = async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c2.cancel();
        };
        let ((r, got), ()) = tokio::time::timeout(
            Duration::from_secs(5),
            futures_util::future::join(collect(&p, "Hallo", &pcm_opts(), &cancel), canceller),
        )
        .await
        .expect("Abbruch hängt");
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert!(got.iter().all(|c| !c.is_last));
        // Der Server merkt den Abriss (Verbindung wurde geschlossen).
        for _ in 0..50 {
            if server.client_gone.load(Ordering::SeqCst) > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("die Verbindung blieb nach dem Abbruch offen");
    }

    #[tokio::test]
    async fn frist_gilt_fuer_den_ganzen_stream() {
        let server = FakeServer::start(|_| {
            Reply::stream(vec![vec![0u8; 2000]], Duration::ZERO, StreamEnd::Stall)
        })
        .await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let mut o = pcm_opts();
        o.timeout = Duration::from_millis(300);
        let started = Instant::now();
        let (r, got) = collect(&p, "Hallo", &o, &CancelToken::detached()).await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Timeout);
        assert!(!e.retryable);
        assert!(got.iter().all(|c| !c.is_last));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn zu_grosse_antwort_wird_abgebrochen() {
        let server = FakeServer::start(|_| {
            Reply::stream(
                split_by(&vec![0u8; 100_000], &[4000]),
                Duration::ZERO,
                StreamEnd::Finish,
            )
        })
        .await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server)).with_max_audio_bytes(10_000);
        let e = p
            .synthesize("Hallo", &pcm_opts(), &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("überschreitet"));
        let (r, got) = collect(&p, "Hallo", &pcm_opts(), &CancelToken::detached()).await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Backend);
        assert!(got.iter().all(|c| !c.is_last));
    }

    #[tokio::test]
    async fn parallele_auftraege_liefern_je_ihr_eigenes_audio() {
        let server = speech_server().await;
        let p = Arc::new(OpenAiSpeechProvider::new(cfg_for(&server)));
        let lens = [5usize, 17, 29, 41, 60];
        let mut tasks = Vec::new();
        for (i, n) in lens.iter().copied().enumerate() {
            let p = p.clone();
            tasks.push(tokio::spawn(async move {
                let cancel = CancelToken::new(format!("par-{i}"));
                if i % 2 == 0 {
                    p.synthesize(&"x".repeat(n), &opts(), &cancel)
                        .await
                        .map(|c| c.duration_ms)
                } else {
                    let (tx, mut rx) = mpsc::channel(8);
                    let o = pcm_opts();
                    let text = "x".repeat(n);
                    let produce = p.stream_synthesize(&text, &o, &cancel, tx);
                    let consume = async {
                        let mut bytes = 0usize;
                        while let Some(c) = rx.recv().await {
                            bytes += c.data.len();
                        }
                        bytes
                    };
                    let (r, bytes) = futures_util::future::join(produce, consume).await;
                    r.map(|()| (bytes / 2) as u64 * 1000 / 24_000)
                }
            }));
        }
        for (task, n) in tasks.into_iter().zip(lens) {
            assert_eq!(task.await.unwrap().unwrap(), n as u64 * 40, "{n} Zeichen");
        }
    }

    // ---- Gesundheit und Stimmen --------------------------------------------------

    #[tokio::test]
    async fn health_probiert_voices_dann_models_und_nennt_die_basis_url_bei_404() {
        // Nur /models vorhanden.
        let server = FakeServer::start(|req: &Req| {
            if req.path == "/v1/models" {
                Reply::json(200, json!({"data": []}))
            } else {
                Reply::json(404, json!({}))
            }
        })
        .await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let h = p.health_check().await;
        assert!(h.available, "{h:?}");
        assert_eq!(h.device, Some(Device::Remote));
        assert_eq!(server.count("/v1/audio/voices"), 1);
        assert_eq!(server.count("/v1/models"), 1);

        // Beides 404: Basis-URL ohne /v1?
        let server = FakeServer::start(|_| Reply::json(404, json!({}))).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h
            .reason
            .unwrap()
            .contains("weder /audio/voices noch /models"));
        assert!(h.hint.unwrap().contains("/v1"));

        // 500: nicht verfügbar mit Status.
        let server = FakeServer::start(|_| Reply::json(500, json!({}))).await;
        let h = OpenAiSpeechProvider::new(cfg_for(&server))
            .health_check()
            .await;
        assert!(!h.available && h.reason.unwrap().contains("HTTP 500"));
    }

    #[tokio::test]
    async fn health_beachtet_die_frist() {
        let server = FakeServer::start(|_| Reply::Hang).await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server))
            .with_health_timeout(Duration::from_millis(300));
        let started = Instant::now();
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("antwortet nicht"), "Grund fehlt");
        assert!(started.elapsed() < Duration::from_secs(2));
        // Die Stimmenliste hängt ebenfalls nicht und fällt zurück.
        let started = Instant::now();
        let v = p.list_voices(None).await.unwrap();
        assert_eq!(v.len(), 1);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn stimmen_werden_tolerant_gelesen_und_nach_sprache_gefiltert() {
        let server = speech_server().await;
        let p = OpenAiSpeechProvider::new(cfg_for(&server));
        let all = p.list_voices(None).await.unwrap();
        let ids: Vec<_> = all.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["ava", "ben", "chloe"]);
        assert_eq!(all[0].label, "Ava");
        assert_eq!(all[0].language, "de", "de-DE → de");
        assert_eq!(all[0].gender.as_deref(), Some("female"));
        assert_eq!(
            all[2].language, "und",
            "ohne Angabe und mehrere Sprachen: und"
        );
        let de: Vec<_> = p.list_voices(Some("de")).await.unwrap();
        let ids: Vec<_> = de.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["ava", "chloe"], "und passt immer");
    }

    #[test]
    fn stimmenformate_werden_verstanden() {
        let shapes = [
            json!({"voices": ["a", "b", "a", " "]}),
            json!(["a", "b"]),
            json!({"data": [{"id": "a"}, {"voice_id": "b"}, {"nichts": 1}, 5, null]}),
            json!({"voices": [{"name": "a"}, {"voice": "b", "language": ["de-AT"]}]}),
        ];
        for s in &shapes {
            let ids: Vec<_> = parse_voices(s, "und").into_iter().map(|v| v.id).collect();
            assert_eq!(ids, ["a", "b"], "{s}");
        }
        let v = parse_voices(&shapes[3], "en");
        assert_eq!(v[0].language, "en");
        assert_eq!(v[1].language, "de");
        assert!(parse_voices(&json!({"voices": "kein array"}), "und").is_empty());
        assert!(parse_voices(&json!("text"), "und").is_empty());
        assert!(parse_voices(&Value::Null, "und").is_empty());
        // Ungültige Sprachangaben fallen auf die Vorgabe zurück.
        let v = parse_voices(
            &json!([{"id": "x", "language": "Deutsch (Standard)"}]),
            "de",
        );
        assert_eq!(v[0].language, "de");
    }

    #[tokio::test]
    async fn stimmenliste_bei_serverfehler_nur_die_konfigurierte_stimme() {
        let server = FakeServer::start(|_| Reply::json(500, json!({}))).await;
        let mut cfg = cfg_for(&server);
        cfg.languages = vec!["de".into()];
        let p = OpenAiSpeechProvider::new(cfg);
        let v = p.list_voices(None).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].id.as_str(), v[0].language.as_str()), ("ava", "de"));
        let mut cfg = cfg_for(&server);
        cfg.voice = None;
        assert!(OpenAiSpeechProvider::new(cfg)
            .list_voices(None)
            .await
            .unwrap()
            .is_empty());
    }

    // ---- Beschreibung, Konfiguration ----------------------------------------------

    #[test]
    fn beschreibung_und_lizenz_sind_ehrlich() {
        let d = OpenAiSpeechProvider::new(OpenAiSpeechConfig {
            model: "m".into(),
            ..Default::default()
        })
        .descriptor();
        assert_eq!(d.id, "openai-speech");
        assert_eq!(d.tier, ProviderTier::Experimental);
        assert!(d.opt_in);
        assert_eq!(d.caps.streaming, StreamingMode::Chunk);
        assert_eq!(
            d.caps.formats,
            vec![AudioEncoding::Pcm16Le, AudioEncoding::Wav]
        );
        assert_eq!(d.caps.sample_rates, vec![24_000]);
        assert_eq!(d.caps.devices, vec![Device::Remote]);
        assert_eq!(d.caps.languages, vec!["und".to_string()]);
        assert!(!d.license.verified && d.license.verified_on.is_none());
        assert!(!d.license.note.is_empty() && !d.license.source_url.is_empty());

        // Nutzerangaben fließen ein, bleiben aber ungeprüft.
        let d = OpenAiSpeechProvider::new(OpenAiSpeechConfig {
            id: "kokoro".into(),
            label: "Kokoro".into(),
            model: "kokoro".into(),
            languages: vec!["en".into(), "fr".into()],
            license: Some(LicenseConfig {
                code: "Apache-2.0".into(),
                weights: "Apache-2.0".into(),
                non_commercial: false,
                source_url: "https://huggingface.co/hexgrad/Kokoro-82M".into(),
                note: "laut Model Card".into(),
            }),
            ..Default::default()
        })
        .descriptor();
        assert_eq!((d.id.as_str(), d.label.as_str()), ("kokoro", "Kokoro"));
        assert_eq!(d.license.code, "Apache-2.0");
        assert!(
            !d.license.verified && d.license.verified_on.is_none(),
            "Nutzerangaben sind nie verifiziert"
        );
        assert_eq!(d.caps.languages, vec!["en", "fr"]);
    }

    #[test]
    fn cache_tag_hat_die_form_id_modell_stimme() {
        let p = OpenAiSpeechProvider::new(OpenAiSpeechConfig {
            model: "kokoro".into(),
            voice: Some("af_bella".into()),
            ..Default::default()
        });
        assert_eq!(p.cache_tag(None), "openai-speech:kokoro:af_bella");
        assert_eq!(p.cache_tag(Some("x")), "openai-speech:kokoro:x");
        let q = OpenAiSpeechProvider::new(OpenAiSpeechConfig {
            model: "kokoro".into(),
            ..Default::default()
        });
        assert_eq!(q.cache_tag(None), "openai-speech:kokoro:default");
    }

    #[test]
    fn validate_nennt_den_konkreten_fehler() {
        let ok = OpenAiSpeechConfig {
            model: "m".into(),
            api_key_env: Some("MEIN_KEY".into()),
            ..Default::default()
        };
        ok.validate().unwrap();
        let bad = |mutate: &dyn Fn(&mut OpenAiSpeechConfig), needle: &str| {
            let mut c = ok.clone();
            mutate(&mut c);
            let e = c.validate().expect_err(needle);
            assert_eq!(e.kind, TtsErrorKind::InvalidConfig, "{needle}: {e:?}");
            assert!(!e.retryable);
            assert!(
                e.message.contains(needle),
                "{needle} fehlt in {:?}",
                e.message
            );
        };
        bad(&|c| c.id = "Groß".into(), "id");
        bad(&|c| c.id = "".into(), "id");
        bad(&|c| c.base_url = "".into(), "base_url fehlt");
        bad(&|c| c.base_url = "kein url".into(), "keine gültige URL");
        bad(&|c| c.base_url = "ftp://127.0.0.1/v1".into(), "http://");
        bad(
            &|c| c.base_url = "http://user:pw@127.0.0.1/v1".into(),
            "Zugangsdaten",
        );
        bad(
            &|c| c.base_url = "http://127.0.0.1/v1?key=1".into(),
            "Query",
        );
        bad(&|c| c.base_url = "http://127.0.0.1/v1#x".into(), "Fragment");
        bad(&|c| c.model = "  ".into(), "model fehlt");
        bad(&|c| c.model = "a\0b".into(), "model");
        bad(&|c| c.voice = Some("a\nb".into()), "voice");
        bad(&|c| c.sample_rate = 100, "sample_rate");
        bad(&|c| c.languages = vec![], "languages");
        bad(&|c| c.languages = vec![" ".into()], "languages");
        bad(&|c| c.api_key_env = Some("1ABC".into()), "api_key_env");
        bad(&|c| c.api_key_env = Some("A B".into()), "api_key_env");
        bad(&|c| c.api_key_env = Some("".into()), "api_key_env");
        // Slash am Ende und Leerraum schaden nicht.
        let mut c = ok.clone();
        c.base_url = "  http://localhost:8880/v1/  ".into();
        assert_eq!(c.parse().unwrap().base, "http://localhost:8880/v1");
        assert!(c.parse().unwrap().loopback);
    }

    #[test]
    fn konfiguration_hat_sichere_vorgaben_und_liest_teilweise_json() {
        let d = OpenAiSpeechConfig::default();
        assert_eq!(d.id, "openai-speech");
        assert_eq!(d.base_url, "http://127.0.0.1:8880/v1");
        assert_eq!(d.sample_rate, 24_000);
        assert!(!d.allow_remote && d.api_key_env.is_none() && d.voice.is_none());
        let c: OpenAiSpeechConfig = serde_json::from_str(r#"{"model":"m","voice":"v"}"#).unwrap();
        assert_eq!(c.model, "m");
        assert_eq!(c.base_url, d.base_url);
        assert!(!c.allow_remote);
        assert_eq!(serde_json::from_str::<OpenAiSpeechConfig>("{}").unwrap(), d);
        // Ohne Modell ist die Vorgabe ungültig (Pflichtfeld des Protokolls).
        assert_eq!(d.validate().unwrap_err().kind, TtsErrorKind::InvalidConfig);

        let mut types = specta::TypeCollection::default();
        types.register::<OpenAiSpeechConfig>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .unwrap();
        for needle in [
            "OpenAiSpeechConfig",
            "base_url: string",
            "allow_remote: boolean",
            "api_key_env: string | null",
            "LicenseConfig",
        ] {
            assert!(ts.contains(needle), "{needle} fehlt in:\n{ts}");
        }
    }

    // ---- Dekoder -------------------------------------------------------------------

    /// Antwort in Stücken der Größen `sizes` einspeisen; liefert alle PCM-Bytes.
    fn decode_all(dec: &mut PcmDecoder, data: &[u8], sizes: &[usize]) -> Result<Vec<u8>, TtsError> {
        let mut out = Vec::new();
        for piece in split_by(data, sizes) {
            let pcm = dec.push(&piece)?;
            assert_eq!(
                pcm.len() % (2 * usize::from(dec.format().map_or(1, |f| f.1))),
                0
            );
            out.extend(pcm);
        }
        dec.finish()?;
        Ok(out)
    }

    #[test]
    fn dekoder_richtet_rohdaten_bei_jeder_stueckelung_auf_ganze_samples_aus() {
        let pcm = tone_pcm(1000);
        for sizes in [
            &[1usize][..],
            &[3],
            &[1, 2],
            &[7, 5, 3],
            &[1999],
            &[2000],
            &[5000],
        ] {
            let mut dec = PcmDecoder::new(Wire::Pcm, 24_000, u64::MAX);
            assert_eq!(dec.format(), Some((24_000, 1)));
            let got = decode_all(&mut dec, &pcm, sizes).unwrap();
            assert_eq!(got, pcm, "Stückelung {sizes:?}");
        }
        // Ein einzelnes Byte allein liefert nichts, zusammen mit dem nächsten ein Sample.
        let mut dec = PcmDecoder::new(Wire::Pcm, 24_000, u64::MAX);
        assert!(dec.push(&[1]).unwrap().is_empty());
        assert_eq!(dec.push(&[2, 3]).unwrap(), vec![1, 2]);
        let e = dec.finish().unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend, "halbes Sample am Ende");
    }

    #[test]
    fn dekoder_liest_wav_mit_streaming_kopf_bei_jeder_stueckelung() {
        let pcm = tone_pcm(800);
        for streaming in [true, false] {
            let wav = wav_bytes(&pcm, 22_050, streaming);
            for sizes in [
                &[1usize][..],
                &[5],
                &[43, 1, 2],
                &[44],
                &[45],
                &[7, 11, 13],
                &[100_000],
            ] {
                let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
                assert_eq!(dec.format(), None, "Rate ist vor dem Kopf unbekannt");
                let got = decode_all(&mut dec, &wav, sizes).unwrap();
                assert_eq!(got, pcm, "streaming={streaming} {sizes:?}");
                assert_eq!(dec.format(), Some((22_050, 1)), "Rate kommt aus dem Kopf");
            }
        }
    }

    #[test]
    fn dekoder_beachtet_endliche_laengen_und_ignoriert_zusatzchunks_dahinter() {
        let pcm = tone_pcm(100);
        let mut wav = wav_bytes(&pcm, 24_000, false);
        wav.extend_from_slice(b"LIST\x04\x00\x00\x00INFO");
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        assert_eq!(decode_all(&mut dec, &wav, &[13, 29]).unwrap(), pcm);

        // Angekündigt ist mehr, als kommt: abgeschnittene Antwort.
        let mut short = wav_bytes(&pcm, 24_000, false);
        short.truncate(short.len() - 10);
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        let e = decode_all(&mut dec, &short, &[50]).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        assert!(e.message.contains("angekündigten Länge"));
    }

    #[test]
    fn dekoder_lehnt_falsche_formate_muell_und_zu_grosses_ab() {
        // Float-WAV.
        let mut wav = wav_bytes(&[0; 8], 24_000, true);
        wav[20] = 3; // Format-Code 3 = IEEE-Float
        wav[34] = 32; // 32 Bit
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        let e = dec.push(&wav).unwrap_err();
        assert!(e.message.contains("16-Bit-PCM"), "{}", e.message);

        // Kein RIFF (schon nach 4 Bytes erkannt).
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        assert!(dec.push(b"OggS").is_err());
        // Ende vor dem Kopf / leere Antwort.
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        assert!(dec
            .push(&wav_bytes(&[], 24_000, true)[..20])
            .unwrap()
            .is_empty());
        assert!(dec.finish().unwrap_err().message.contains("WAV-Kopfes"));
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        assert!(dec.finish().unwrap_err().message.contains("leere Antwort"));
        // Kopf, der nie zu Ende geht: nicht endlos sammeln.
        let mut dec = PcmDecoder::new(Wire::Wav, 24_000, u64::MAX);
        let mut junk = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0".to_vec();
        junk.extend(vec![0u8; MAX_WAV_HEADER + 10]);
        assert!(dec.push(&junk).is_err());
        // Deckel.
        let mut dec = PcmDecoder::new(Wire::Pcm, 24_000, 100);
        assert!(dec.push(&[0u8; 100]).is_ok());
        assert!(dec
            .push(&[0u8; 2])
            .unwrap_err()
            .message
            .contains("überschreitet"));
    }
}
