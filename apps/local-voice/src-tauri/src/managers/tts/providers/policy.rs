//! Einstellungen, Konfigurationsvorrang und Guardrails der Anbieterschicht
//! (Goal tts-provider, Paket P3a).
//!
//! Alles hier ist REIN: keine `AppHandle`, kein Dateizugriff, keine
//! Prozessumgebung — Einstellungen, Umgebung und Überschreibungen werden
//! hereingereicht. Damit ist jede Regel ohne App testbar (und die Test-Exe
//! startet, weil nie `settings::get_settings(&AppHandle)` gerufen wird).
//!
//! ## Konfigurationsvorrang (deterministisch)
//!
//! `Override` (Argument; die CLI aus P3b füllt [`ProviderOverrides`])
//! `>` Umgebungsvariable `LVA_TTS_<ANBIETER>_<FELD>`
//! `>` Einstellungen (`tts_providers`)
//! `>` „installiert“ (was die Modellseite eingerichtet hat, [`InstalledPaths`]; nur
//!   `exe_path` und `model` von Phonon, nur wenn die Einstellung leer ist)
//! `>` Vorgabe des Adapters (`Default` der Konfiguration; wer nichts
//! gespeichert hat, bekommt sie über `#[serde(default)]`).
//!
//! * Umgebungsvariablen und Overrides gibt es NUR für `exe_path`, `model`,
//!   `base_url` und `voice`. `<ANBIETER>` ist die Kennung in Großbuchstaben,
//!   `-` wird `_`: `LVA_TTS_PHONON_EXE_PATH`, `LVA_TTS_OPENAI_SPEECH_BASE_URL`,
//!   `LVA_TTS_OLLAMA_SPEECH_MODEL`, `LVA_TTS_OPENAI_SPEECH_VOICE`, … Felder,
//!   die ein Anbieter nicht kennt (`LVA_TTS_PHONON_BASE_URL`), werden
//!   ignoriert. Leere Werte (nach `trim`) zählen als nicht gesetzt.
//! * Stimme: Override > Umgebung > `voices[<id>]` (die letzte Auswahl) >
//!   `voice` der Anbieter-Konfiguration > keine.
//! * Freigaben (`enabled`, `license_acks`, `allow_remote`) lassen sich auf
//!   KEINEM dieser Wege setzen — weder per Umgebung noch per Override noch
//!   über `tts_provider_set_config`. Die Adapter-Schalter `allow_remote`
//!   (openai-speech, ollama-speech) und `allow_network` (phonon) werden beim
//!   Auflösen aus `allow_remote` der Einstellungen ABGELEITET.
//!
//! ## Guardrails
//!
//! [`check_use`] ist die EINE Stelle, die entscheidet, ob ein Anbieter
//! benutzt werden darf. Sie gilt für jede Nutzung (Vorlesen, Probestimme,
//! später CLI-Benchmark und Agentenbrücke) und steht deshalb im Rechenweg
//! jeder Nutzung (`pipeline::ProviderEngine`, `preview::PreviewService`),
//! nicht erst im Tauri-Befehl. Regeln siehe dort.
//!
//! **Die Freigabe-Befehle `tts_provider_set_enabled`,
//! `tts_provider_acknowledge_license` und `tts_provider_set_allow_remote`
//! sind AUSSCHLIESSLICH für die Oberfläche (ein Klick des Nutzers). Sie dürfen
//! nie in die Agentenbrücke, in CLI-Automatik oder einen anderen
//! unbeaufsichtigten Weg aufgenommen werden** — sonst könnte ein Agent die
//! Lizenz- und Datenschutzsperre selbst aufheben.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use super::contract::{is_valid_provider_id, ProviderDescriptor, TtsError, TtsErrorKind};
use super::ollama_speech::{OllamaSpeechConfig, OLLAMA_ID};
use super::openai_speech::{is_loopback_url, OpenAiSpeechConfig, DEFAULT_ID as OPENAI_DEFAULT_ID};
use super::phonon::{PhononConfig, PhononModelRef, PHONON_ID};

pub const FISH_ID: &str = "fish";
pub const PIPER_ID: &str = "piper";
pub const MOCK_ID: &str = "mock";

/// Kennungen, die der openai-speech-Adapter nie tragen darf (sonst ersetzte
/// er per `register` einen eingebauten Anbieter).
pub const RESERVED_IDS: [&str; 5] = [FISH_ID, PIPER_ID, PHONON_ID, OLLAMA_ID, MOCK_ID];

/// Felder, die Umgebung und Override setzen dürfen.
pub const OVERRIDABLE_FIELDS: [&str; 4] = ["exe_path", "model", "base_url", "voice"];

// ---- Einstellungen ----------------------------------------------------------------

/// Zustimmung des Nutzers zu den Lizenzangaben eines Anbieters. Gilt nur,
/// solange `fingerprint` dem aktuellen Stand entspricht (siehe
/// [`license_fingerprint`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LicenseAck {
    pub provider_id: String,
    pub fingerprint: String,
    /// RFC 3339, UTC.
    pub acked_at: String,
}

/// Einstellungsfeld `tts_providers` (optional: alte Dateien ohne das Feld
/// bleiben gültig, alles ist aus bzw. leer).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct TtsProvidersSettings {
    pub phonon: PhononConfig,
    pub openai_speech: OpenAiSpeechConfig,
    pub ollama_speech: OllamaSpeechConfig,
    /// Opt-in je Anbieter-Kennung. Nur über `tts_provider_set_enabled`.
    pub enabled: Vec<String>,
    /// Lizenz-Zustimmungen. Nur über `tts_provider_acknowledge_license`.
    pub license_acks: Vec<LicenseAck>,
    /// Anbieter, deren Server/Netzzugriff außerhalb von Loopback liegen darf.
    /// Nur über `tts_provider_set_allow_remote`.
    pub allow_remote: Vec<String>,
    /// Letzte Stimmenauswahl je Anbieter-Kennung (`tts_provider_select`).
    pub voices: BTreeMap<String, String>,
    /// Welche Stimmen eines Anbieters im Vorlesen-Dropdown stehen (nur
    /// `tts_provider_set_visible_voices`). Kein Eintrag oder eine leere Liste: alle sichtbar.
    /// Die Liste „keine“ ist `["default"]` (die Standardstimme steht immer da). Die gerade gewählte
    /// Stimme bleibt in jedem Fall sichtbar ([`visible_voice`]).
    pub visible_voices: BTreeMap<String, Vec<String>>,
}

/// Kennung der Standardstimme eines Anbieters (immer sichtbar, nie abwählbar).
pub const DEFAULT_VOICE_ID: &str = "default";

/// Steht die Stimme `voice` im Vorlesen-Dropdown? `visible`: die gespeicherte Liste des Anbieters
/// (`None`/leer = alle), `selected`: die gerade gewählte Stimme des Anbieters (bleibt sichtbar).
pub fn visible_voice(visible: Option<&Vec<String>>, selected: Option<&str>, voice: &str) -> bool {
    voice == DEFAULT_VOICE_ID
        || selected == Some(voice)
        || visible.map_or(true, |list| {
            list.is_empty() || list.iter().any(|v| v == voice)
        })
}

impl TtsProvidersSettings {
    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.iter().any(|e| e == id)
    }

    pub fn is_remote_allowed(&self, id: &str) -> bool {
        self.allow_remote.iter().any(|e| e == id)
    }

    pub fn license_ack(&self, id: &str) -> Option<&LicenseAck> {
        self.license_acks.iter().find(|a| a.provider_id == id)
    }

    /// Opt-in setzen oder zurücknehmen (ohne Doppelte, Reihenfolge bleibt).
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        set_membership(&mut self.enabled, id, on);
    }

    pub fn set_remote_allowed(&mut self, id: &str, on: bool) {
        set_membership(&mut self.allow_remote, id, on);
    }

    /// Zustimmung eintragen; eine frühere desselben Anbieters wird ersetzt.
    pub fn record_ack(&mut self, id: &str, fingerprint: &str, acked_at: &str) {
        self.license_acks.retain(|a| a.provider_id != id);
        self.license_acks.push(LicenseAck {
            provider_id: id.to_string(),
            fingerprint: fingerprint.to_string(),
            acked_at: acked_at.to_string(),
        });
    }
}

fn set_membership(list: &mut Vec<String>, id: &str, on: bool) {
    let present = list.iter().any(|e| e == id);
    match (on, present) {
        (true, false) => list.push(id.to_string()),
        (false, true) => list.retain(|e| e != id),
        _ => {}
    }
}

// ---- Konfigurationsvorrang ----------------------------------------------------------

/// Woher ein aufgelöster Wert stammt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    /// Vorgabe des Adapters (nichts gespeichert).
    Default,
    /// Über die Modellseite installiert; nichts eingetragen (Anzeige: „Installiert“).
    Installed,
    Settings,
    Env,
    Override,
}

/// Überschreibungen EINES Anbieters (Argumente der CLI). Bewusst nur die vier
/// überschreibbaren Felder — Freigaben gibt es hier nicht.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderOverride {
    pub exe_path: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub voice: Option<String>,
}

impl ProviderOverride {
    fn get(&self, field: &str) -> Option<&str> {
        match field {
            "exe_path" => self.exe_path.as_deref(),
            "model" => self.model.as_deref(),
            "base_url" => self.base_url.as_deref(),
            "voice" => self.voice.as_deref(),
            _ => None,
        }
    }
}

/// Überschreibungen aller konfigurierbaren Anbieter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderOverrides {
    pub phonon: ProviderOverride,
    pub openai_speech: ProviderOverride,
    pub ollama_speech: ProviderOverride,
}

/// Pfade der Teile von Phonon, die die Modellseite installiert hat (leer = nichts oder
/// unvollständig). Rein: der Aufrufer ermittelt sie am Dateisystem
/// (`managers::tts::phonon_models::installed_paths`) und reicht sie herein.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledPhonon {
    pub exe_path: Option<String>,
    pub model: Option<String>,
    /// Weitere vollstaendig installierte Modelle anderer Sprachen (z. B. das englische neben dem
    /// deutschen). Der Anbieter bietet ihre Stimmen als `<sprache>:<stimme>` an.
    pub others: Vec<PhononModelRef>,
}

/// Was die Modellseite installiert hat, für die Auflösungsstufe „installiert“.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstalledPaths {
    pub phonon: InstalledPhonon,
}

/// Name der Umgebungsvariablen für `<slot_id>` und `<field>`:
/// `env_var_name("openai-speech", "base_url") == "LVA_TTS_OPENAI_SPEECH_BASE_URL"`.
pub fn env_var_name(slot_id: &str, field: &str) -> String {
    format!(
        "LVA_TTS_{}_{}",
        slot_id.to_ascii_uppercase().replace('-', "_"),
        field.to_ascii_uppercase()
    )
}

/// Umgebung des Prozesses (Produktion). Tests reichen eine eigene Funktion herein.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Ergebnis von [`resolve_provider_configs`]: die Konfigurationen, mit denen
/// die Adapter gebaut werden, und woher jeder der vier überschreibbaren Werte kam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfigs {
    pub phonon: PhononConfig,
    pub openai_speech: OpenAiSpeechConfig,
    pub ollama_speech: OllamaSpeechConfig,
    sources: BTreeMap<(String, String), ConfigSource>,
    /// Der Stand „installiert“, mit dem aufgelöst wurde (wer neu auflöst, nimmt denselben).
    installed: InstalledPaths,
}

impl ResolvedConfigs {
    /// Die Installation, die in diese Auflösung eingegangen ist.
    pub fn installed(&self) -> &InstalledPaths {
        &self.installed
    }

    /// Herkunft eines Feldes (`slot_id` = Kennung des Anbieters, bei
    /// openai-speech immer `"openai-speech"`, auch wenn die Konfiguration
    /// eine andere Kennung trägt). Unbekannte Felder: `Default`.
    pub fn source(&self, slot_id: &str, field: &str) -> ConfigSource {
        self.sources
            .get(&(slot_id.to_string(), field.to_string()))
            .copied()
            .unwrap_or(ConfigSource::Default)
    }

    /// Was die Guardrails über die WIRKSAME Konfiguration eines Anbieters
    /// wissen müssen (Modell für den Fingerabdruck, Adresse für die
    /// Datenschutzsperre). Fish/Piper und unbekannte Kennungen: leer.
    pub fn subject(&self, provider_id: &str) -> UseSubject {
        if provider_id == PHONON_ID {
            UseSubject {
                model: self.phonon.model.trim().to_string(),
                base_url: None,
            }
        } else if provider_id == OLLAMA_ID {
            UseSubject {
                model: self.ollama_speech.model.trim().to_string(),
                base_url: Some(self.ollama_speech.base_url.trim().to_string()),
            }
        } else if provider_id == self.openai_speech.id {
            UseSubject {
                model: self.openai_speech.model.trim().to_string(),
                base_url: Some(self.openai_speech.base_url.trim().to_string()),
            }
        } else {
            UseSubject::default()
        }
    }

    /// Die gewählte Stimme eines Anbieters (Vorrang siehe Moduldoku).
    pub fn voice_for(&self, provider_id: &str) -> Option<String> {
        let voice = if provider_id == PHONON_ID {
            self.phonon.voice.as_deref()
        } else if provider_id == OLLAMA_ID {
            self.ollama_speech.voice.as_deref()
        } else if provider_id == self.openai_speech.id {
            self.openai_speech.voice.as_deref()
        } else {
            None
        };
        voice
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from)
    }
}

/// Leerer Wert (nach `trim`) = nicht gesetzt.
fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(String::from)
}

/// Wendet Override und Umgebung auf EIN Feld an.
struct Layers<'a> {
    slot: &'a str,
    env: &'a dyn Fn(&str) -> Option<String>,
    ov: &'a ProviderOverride,
    sources: &'a mut BTreeMap<(String, String), ConfigSource>,
}

impl Layers<'_> {
    fn apply(
        &mut self,
        field: &str,
        current: Option<String>,
        base: ConfigSource,
    ) -> Option<String> {
        let (value, source) = if let Some(v) = clean(self.ov.get(field)) {
            (Some(v), ConfigSource::Override)
        } else if let Some(v) = clean((self.env)(&env_var_name(self.slot, field)).as_deref()) {
            (Some(v), ConfigSource::Env)
        } else {
            (current, base)
        };
        self.sources
            .insert((self.slot.to_string(), field.to_string()), source);
        value
    }
}

fn base_source(differs_from_default: bool) -> ConfigSource {
    if differs_from_default {
        ConfigSource::Settings
    } else {
        ConfigSource::Default
    }
}

/// Einstellung vor „installiert“ vor Vorgabe, für ein Feld, das leer bleiben darf. Leer (nach
/// `trim`) oder gleich der Vorgabe zählt als nicht eingetragen.
fn with_installed(current: &str, default: &str, installed: Option<&str>) -> (String, ConfigSource) {
    if current.trim().is_empty() || current == default {
        if let Some(path) = clean(installed) {
            return (path, ConfigSource::Installed);
        }
        return (current.to_string(), ConfigSource::Default);
    }
    (current.to_string(), ConfigSource::Settings)
}

/// Löst die Konfigurationen aller Anbieter auf — rein und deterministisch
/// (Vorrang siehe Moduldoku). `env` liefert Umgebungsvariablen (Produktion:
/// [`process_env`]). Ohne die Stufe „installiert“ (siehe
/// [`resolve_provider_configs_with`]).
pub fn resolve_provider_configs(
    settings: &TtsProvidersSettings,
    env: &dyn Fn(&str) -> Option<String>,
    overrides: &ProviderOverrides,
) -> ResolvedConfigs {
    resolve_provider_configs_with(settings, env, overrides, &InstalledPaths::default())
}

/// Wie [`resolve_provider_configs`], mit der Stufe „installiert“ zwischen Einstellungen und
/// Vorgabe: ist `exe_path` oder `model` von Phonon leer und hat die Modellseite etwas
/// installiert, gilt der installierte Pfad (`source()` meldet [`ConfigSource::Installed`]).
pub fn resolve_provider_configs_with(
    settings: &TtsProvidersSettings,
    env: &dyn Fn(&str) -> Option<String>,
    overrides: &ProviderOverrides,
    installed: &InstalledPaths,
) -> ResolvedConfigs {
    let mut sources = BTreeMap::new();

    // ---- phonon
    let mut phonon = settings.phonon.clone();
    {
        let d = PhononConfig::default();
        let mut l = Layers {
            slot: PHONON_ID,
            env,
            ov: &overrides.phonon,
            sources: &mut sources,
        };
        let (exe, exe_base) = with_installed(
            &phonon.exe_path,
            &d.exe_path,
            installed.phonon.exe_path.as_deref(),
        );
        phonon.exe_path = l.apply("exe_path", Some(exe), exe_base).unwrap_or_default();
        let (model, model_base) =
            with_installed(&phonon.model, &d.model, installed.phonon.model.as_deref());
        phonon.model = l
            .apply("model", Some(model), model_base)
            .unwrap_or_default();
        let (voice, base) = stored_voice(settings, PHONON_ID, phonon.voice.as_deref());
        phonon.voice = l.apply("voice", voice, base);
        // base_url gibt es bei phonon nicht.
        l.sources.insert(
            (PHONON_ID.to_string(), "base_url".to_string()),
            ConfigSource::Default,
        );
    }
    // Netzzugriff (Modell-Downloads) nur mit der ausdrücklichen Freigabe.
    phonon.allow_network = phonon.allow_network && settings.is_remote_allowed(PHONON_ID);

    // ---- openai-speech
    let mut openai = settings.openai_speech.clone();
    // Eine ungültige oder eingebaute Kennung würde einen anderen Anbieter ersetzen.
    if !is_valid_provider_id(&openai.id) || RESERVED_IDS.contains(&openai.id.as_str()) {
        openai.id = OPENAI_DEFAULT_ID.to_string();
    }
    {
        let d = OpenAiSpeechConfig::default();
        let mut l = Layers {
            slot: OPENAI_DEFAULT_ID,
            env,
            ov: &overrides.openai_speech,
            sources: &mut sources,
        };
        openai.base_url = l
            .apply(
                "base_url",
                Some(openai.base_url.clone()),
                base_source(openai.base_url != d.base_url),
            )
            .unwrap_or_default();
        openai.model = l
            .apply(
                "model",
                Some(openai.model.clone()),
                base_source(openai.model != d.model),
            )
            .unwrap_or_default();
        let (voice, base) = stored_voice(settings, &openai.id, openai.voice.as_deref());
        openai.voice = l.apply("voice", voice, base);
        l.sources.insert(
            (OPENAI_DEFAULT_ID.to_string(), "exe_path".to_string()),
            ConfigSource::Default,
        );
    }
    openai.allow_remote = settings.is_remote_allowed(&openai.id);

    // ---- ollama-speech
    let mut ollama = settings.ollama_speech.clone();
    {
        let d = OllamaSpeechConfig::default();
        let mut l = Layers {
            slot: OLLAMA_ID,
            env,
            ov: &overrides.ollama_speech,
            sources: &mut sources,
        };
        ollama.base_url = l
            .apply(
                "base_url",
                Some(ollama.base_url.clone()),
                base_source(ollama.base_url != d.base_url),
            )
            .unwrap_or_default();
        ollama.model = l
            .apply(
                "model",
                Some(ollama.model.clone()),
                base_source(ollama.model != d.model),
            )
            .unwrap_or_default();
        let (voice, base) = stored_voice(settings, OLLAMA_ID, ollama.voice.as_deref());
        ollama.voice = l.apply("voice", voice, base);
        l.sources.insert(
            (OLLAMA_ID.to_string(), "exe_path".to_string()),
            ConfigSource::Default,
        );
    }
    ollama.allow_remote = settings.is_remote_allowed(OLLAMA_ID);

    ResolvedConfigs {
        phonon,
        openai_speech: openai,
        ollama_speech: ollama,
        sources,
        installed: installed.clone(),
    }
}

/// Gespeicherte Stimme: die letzte Auswahl (`voices`) vor `voice` der Konfiguration.
fn stored_voice(
    settings: &TtsProvidersSettings,
    id: &str,
    configured: Option<&str>,
) -> (Option<String>, ConfigSource) {
    if let Some(v) = clean(settings.voices.get(id).map(String::as_str)) {
        return (Some(v), ConfigSource::Settings);
    }
    match clean(configured) {
        Some(v) => (Some(v), ConfigSource::Settings),
        None => (None, ConfigSource::Default),
    }
}

// ---- Guardrails -----------------------------------------------------------------------

/// Die wirksame Konfiguration, soweit die Guardrails sie brauchen (Typ in
/// `contract`, weil ihn die Instanz liefert: [`TtsProvider::use_subject`]).
pub use super::contract::UseSubject;

/// Gemeinsamer, jederzeit AKTUELLER Stand der Freigaben (Opt-in, Zustimmungen,
/// Fernzugriff). Der `TtsManager` hält ihn, die Befehle der Oberfläche
/// aktualisieren ihn nach dem Speichern, und jede laufende Nutzung (Engine,
/// Probestimme, Probe des Ollama-Anbieters) prüft [`check_use`] je Satz bzw.
/// je Teil dagegen — nicht gegen einen Stand, der beim Start des Auftrags
/// eingefroren wurde. Ein Widerruf wirkt damit auch auf laufende Aufträge.
///
/// Lesen kostet einen kurzen Lesezugriff ohne Wartezeit (kein I/O, keine
/// Allokation außer dem `Arc`-Zähler); der Audio-Echtzeitpfad ruft das nicht.
/// `Default` = nichts freigegeben (sicherer Anfangszustand).
#[derive(Clone, Default)]
pub struct LiveApprovals(Arc<RwLock<Arc<TtsProvidersSettings>>>);

impl LiveApprovals {
    pub fn new(settings: TtsProvidersSettings) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(settings))))
    }

    /// Der aktuelle Stand.
    pub fn get(&self) -> Arc<TtsProvidersSettings> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Neuen Stand setzen (nach dem Speichern der Einstellungen).
    pub fn set(&self, settings: TtsProvidersSettings) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(settings);
    }

    /// [`check_use`] gegen den AKTUELLEN Stand.
    pub fn check(&self, desc: &ProviderDescriptor, subject: &UseSubject) -> Result<(), TtsError> {
        check_use(desc, subject, &self.get())
    }

    /// Dieselbe Prüfung als weitergebbare Funktion (für Anbieter mit
    /// Nebenwirkung, z. B. die Probe des Ollama-Anbieters).
    pub fn gate(&self) -> UseGate {
        let live = self.clone();
        Arc::new(move |desc, subject| live.check(desc, subject))
    }
}

impl std::fmt::Debug for LiveApprovals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveApprovals(..)")
    }
}

/// Eine Prüffunktion ([`check_use`] gegen einen bestimmten Stand der
/// Freigaben), die ein Anbieter vor Nebenwirkungen fragen kann.
pub type UseGate =
    Arc<dyn Fn(&ProviderDescriptor, &UseSubject) -> Result<(), TtsError> + Send + Sync>;

/// Fingerabdruck der Lizenzlage: Kennung + Gewichts-Lizenz (Text und
/// „nichtkommerziell“) + Modell. Ändert sich eines davon, gilt eine frühere
/// Zustimmung nicht mehr. Wird gespeichert — der Algorithmus (SHA-256 über
/// längenpräfixierte Teile, erste 32 Hexzeichen) darf sich nicht ändern, ein
/// Test hält einen Referenzwert fest.
pub fn license_fingerprint(desc: &ProviderDescriptor, subject: &UseSubject) -> String {
    let mut h = Sha256::new();
    let nc = if desc.license.non_commercial {
        "nc"
    } else {
        "c"
    };
    for part in [
        desc.id.as_str(),
        desc.license.weights.as_str(),
        nc,
        subject.model.trim(),
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let digest = h.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Braucht dieser Anbieter eine Lizenz-Zustimmung? Nur Opt-in-Anbieter, und
/// nur wenn die Gewichte nichtkommerziell oder ungeprüft sind. Fish und Piper
/// (`opt_in = false`) sind NIE gesperrt, auch wenn ihre Lizenz ungeprüft ist.
pub fn license_needs_ack(desc: &ProviderDescriptor) -> bool {
    desc.opt_in && (desc.license.non_commercial || !desc.license.verified)
}

fn is_remote(subject: &UseSubject) -> Result<bool, TtsError> {
    let Some(url) = subject.base_url.as_deref() else {
        return Ok(false);
    };
    if url::Url::parse(url.trim()).is_err() {
        return Err(TtsError::invalid_config(format!(
            "base_url '{}' ist keine gültige Adresse",
            url.trim()
        )));
    }
    Ok(!is_loopback_url(url))
}

/// DIE Prüfung vor jeder Nutzung eines Anbieters (siehe Moduldoku).
///
/// 1. `opt_in`: die Kennung muss in `enabled` stehen — sonst
///    [`TtsErrorKind::LicenseBlocked`] („nicht freigeschaltet“; die Klasse
///    steht für „der Nutzer muss erst zustimmen“).
/// 2. `opt_in && (non_commercial || !verified)`: ein `license_ack` mit dem
///    AKTUELLEN Fingerabdruck ist nötig — sonst `LicenseBlocked`.
/// 3. Adresse außerhalb von Loopback: die Kennung muss in `allow_remote`
///    stehen — sonst [`TtsErrorKind::PrivacyBlocked`]. Gilt unabhängig von
///    `opt_in`. Eine unparsbare Adresse ist `InvalidConfig`.
///
/// Fish und Piper haben `opt_in = false` und keine Adresse und bleiben frei.
pub fn check_use(
    desc: &ProviderDescriptor,
    subject: &UseSubject,
    settings: &TtsProvidersSettings,
) -> Result<(), TtsError> {
    if desc.opt_in {
        if !settings.is_enabled(&desc.id) {
            return Err(TtsError::new(
                TtsErrorKind::LicenseBlocked,
                format!(
                    "{} ist nicht freigeschaltet (Opt-in fehlt). In den Einstellungen unter \
                     Sprachausgabe-Anbieter einschalten.",
                    desc.label
                ),
            ));
        }
        if license_needs_ack(desc) {
            let current = license_fingerprint(desc, subject);
            match settings.license_ack(&desc.id) {
                Some(ack) if ack.fingerprint == current => {}
                Some(_) => {
                    return Err(TtsError::new(
                        TtsErrorKind::LicenseBlocked,
                        format!(
                            "{}: Modell oder Lizenzangaben haben sich geändert, die frühere \
                             Zustimmung gilt nicht mehr. Lizenz erneut bestätigen.",
                            desc.label
                        ),
                    ))
                }
                None => {
                    return Err(TtsError::new(
                        TtsErrorKind::LicenseBlocked,
                        format!(
                            "{}: Die Lizenz der Gewichte ist nichtkommerziell oder ungeprüft. \
                             Vor der Nutzung muss sie bestätigt werden.",
                            desc.label
                        ),
                    ))
                }
            }
        }
    }
    if is_remote(subject)? && !settings.is_remote_allowed(&desc.id) {
        return Err(TtsError::new(
            TtsErrorKind::PrivacyBlocked,
            format!(
                "{} spricht eine Adresse außerhalb dieses Rechners an. Der Text würde das \
                 Gerät verlassen — erst ausdrücklich erlauben.",
                desc.label
            ),
        ));
    }
    Ok(())
}

/// Freigabezustand eines Anbieters für die Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UseState {
    /// Opt-in erteilt (Anbieter ohne Opt-in: immer `true`).
    pub enabled: bool,
    pub license_needs_ack: bool,
    /// Gültige Zustimmung mit aktuellem Fingerabdruck liegt vor.
    pub license_acked: bool,
    /// Fernzugriff ausdrücklich erlaubt.
    pub allow_remote: bool,
    /// Die wirksame Adresse liegt außerhalb von Loopback.
    pub remote: bool,
    /// Aktueller Fingerabdruck — die Oberfläche reicht ihn beim Bestätigen zurück.
    pub fingerprint: String,
    /// Warum die Nutzung jetzt gesperrt ist (`None` = erlaubt).
    pub blocked: Option<TtsError>,
}

pub fn use_state(
    desc: &ProviderDescriptor,
    subject: &UseSubject,
    settings: &TtsProvidersSettings,
) -> UseState {
    let fingerprint = license_fingerprint(desc, subject);
    UseState {
        enabled: !desc.opt_in || settings.is_enabled(&desc.id),
        license_needs_ack: license_needs_ack(desc),
        license_acked: settings
            .license_ack(&desc.id)
            .is_some_and(|a| a.fingerprint == fingerprint),
        allow_remote: settings.is_remote_allowed(&desc.id),
        remote: is_remote(subject).unwrap_or(false),
        fingerprint,
        blocked: check_use(desc, subject, settings).err(),
    }
}

// ---- Konfiguration lesen und schreiben ---------------------------------------------------

/// Welche Konfiguration hinter einer Anbieter-Kennung liegt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSlot {
    Phonon,
    OpenAiSpeech,
    OllamaSpeech,
}

/// Kennung → Konfigurationsplatz. openai-speech trägt die Kennung seiner
/// (gespeicherten) Konfiguration; Fish/Piper/Mock haben keine Konfiguration.
pub fn config_slot(settings: &TtsProvidersSettings, id: &str) -> Option<ConfigSlot> {
    if id == PHONON_ID {
        Some(ConfigSlot::Phonon)
    } else if id == OLLAMA_ID {
        Some(ConfigSlot::OllamaSpeech)
    } else if id == OPENAI_DEFAULT_ID || id == settings.openai_speech.id {
        Some(ConfigSlot::OpenAiSpeech)
    } else {
        None
    }
}

fn bad_json(e: serde_json::Error) -> TtsError {
    TtsError::invalid_config(format!("Konfiguration ist kein gültiges JSON: {e}"))
}

/// Gespeicherte Konfiguration als JSON-Text.
pub fn config_json(settings: &TtsProvidersSettings, id: &str) -> Result<String, TtsError> {
    let slot = config_slot(settings, id)
        .ok_or_else(|| TtsError::bad_request(format!("Anbieter '{id}' hat keine Konfiguration")))?;
    let json = match slot {
        ConfigSlot::Phonon => serde_json::to_string(&settings.phonon),
        ConfigSlot::OpenAiSpeech => serde_json::to_string(&settings.openai_speech),
        ConfigSlot::OllamaSpeech => serde_json::to_string(&settings.ollama_speech),
    };
    json.map_err(bad_json)
}

/// Konfiguration prüfen und in die Einstellungen übernehmen. Bei einem Fehler
/// bleiben die Einstellungen unverändert.
///
/// Geprüft wird mit `validate()` des Adapters — aber nur das FORMAT: eine
/// Adresse außerhalb von Loopback lässt sich speichern, benutzen darf man sie
/// erst mit `allow_remote` ([`check_use`]). Die Adapter-Schalter
/// `allow_remote` (openai-speech, ollama-speech) werden beim Speichern auf
/// `false` gesetzt: Freigaben kommen nur über `tts_provider_set_allow_remote`
/// und nie über eine Konfiguration. Eine eingebaute Kennung (`fish`, `piper`,
/// …) ist für openai-speech verboten.
pub fn apply_config_json(
    settings: &mut TtsProvidersSettings,
    id: &str,
    json: &str,
) -> Result<(), TtsError> {
    let slot = config_slot(settings, id)
        .ok_or_else(|| TtsError::bad_request(format!("Anbieter '{id}' hat keine Konfiguration")))?;
    match slot {
        ConfigSlot::Phonon => {
            let cfg: PhononConfig = serde_json::from_str(json).map_err(bad_json)?;
            // Leere Pfade = die von der App eingerichtete Installation (One-Click-Einrichtung).
            cfg.validate_stored()?;
            settings.phonon = cfg;
        }
        ConfigSlot::OpenAiSpeech => {
            let mut cfg: OpenAiSpeechConfig = serde_json::from_str(json).map_err(bad_json)?;
            if RESERVED_IDS.contains(&cfg.id.as_str()) {
                return Err(TtsError::invalid_config(format!(
                    "id '{}' ist für einen eingebauten Anbieter reserviert",
                    cfg.id
                )));
            }
            cfg.allow_remote = false;
            // Nur das Format prüfen: die Adresse darf entfernt liegen.
            OpenAiSpeechConfig {
                allow_remote: true,
                ..cfg.clone()
            }
            .validate()?;
            settings.openai_speech = cfg;
        }
        ConfigSlot::OllamaSpeech => {
            let mut cfg: OllamaSpeechConfig = serde_json::from_str(json).map_err(bad_json)?;
            cfg.allow_remote = false;
            OllamaSpeechConfig {
                allow_remote: true,
                ..cfg.clone()
            }
            .validate()?;
            settings.ollama_speech = cfg;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn sichtbar_ist_die_standardstimme_die_gewaehlte_und_die_gespeicherten() {
        let list = vec!["Nele".to_string(), "Anton".to_string()];
        // Keine Auswahl (fehlend oder leer): alle.
        assert!(visible_voice(None, None, "Mats"));
        assert!(visible_voice(Some(&Vec::new()), None, "Mats"));
        // Mit Auswahl: nur diese, dazu die Standardstimme und die gewaehlte.
        assert!(visible_voice(Some(&list), None, "Nele"));
        assert!(!visible_voice(Some(&list), None, "Mats"));
        assert!(visible_voice(Some(&list), None, DEFAULT_VOICE_ID));
        assert!(visible_voice(Some(&list), Some("Mats"), "Mats"));
        assert!(!visible_voice(Some(&list), Some("Mats"), "Jule"));
        // „Keine“: nur die Standardstimme (und die gewaehlte).
        let none = vec![DEFAULT_VOICE_ID.to_string()];
        assert!(!visible_voice(Some(&none), None, "Nele"));
        assert!(visible_voice(Some(&none), Some("Nele"), "Nele"));
    }

    use super::*;
    use crate::managers::tts::providers::builtin::{FishProvider, PiperProvider};
    use crate::managers::tts::providers::contract::TtsProvider;
    use crate::managers::tts::providers::mock::MockProvider;
    use std::collections::HashMap;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    /// Opt-in-Anbieter wie phonon mit 40m-Gewichten: nichtkommerziell.
    fn nc_desc() -> ProviderDescriptor {
        let mut d = MockProvider::new().with_id("phonon").descriptor();
        d.license.non_commercial = true;
        d.license.verified = true;
        d.license.weights = "CC BY-NC 4.0 (Phonon 40m, gated)".to_string();
        d
    }

    fn subject(model: &str) -> UseSubject {
        UseSubject {
            model: model.to_string(),
            base_url: None,
        }
    }

    fn enabled(ids: &[&str]) -> TtsProvidersSettings {
        let mut s = TtsProvidersSettings::default();
        for id in ids {
            s.set_enabled(id, true);
        }
        s
    }

    fn ack(s: &mut TtsProvidersSettings, desc: &ProviderDescriptor, subj: &UseSubject) {
        s.record_ack(
            &desc.id,
            &license_fingerprint(desc, subj),
            "2026-10-26T12:00:00Z",
        );
    }

    // ---- Einstellungen ------------------------------------------------------------

    #[test]
    fn standard_ist_alles_aus_und_leer() {
        let s = TtsProvidersSettings::default();
        assert!(s.enabled.is_empty());
        assert!(s.license_acks.is_empty());
        assert!(s.allow_remote.is_empty());
        assert!(s.voices.is_empty());
        assert!(!s.openai_speech.allow_remote);
        assert!(!s.ollama_speech.allow_remote);
        assert!(!s.phonon.allow_network);
    }

    #[test]
    fn ein_leeres_oder_teilweises_json_ergibt_standardwerte() {
        let empty: TtsProvidersSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, TtsProvidersSettings::default());
        let partial: TtsProvidersSettings =
            serde_json::from_str(r#"{"enabled":["phonon"],"phonon":{"lang":"en"}}"#).unwrap();
        assert_eq!(partial.enabled, ["phonon"]);
        assert_eq!(partial.phonon.lang, "en");
        assert_eq!(partial.openai_speech, OpenAiSpeechConfig::default());
    }

    #[test]
    fn die_einstellungen_ueberleben_hin_und_zurueck() {
        let mut s = TtsProvidersSettings::default();
        s.set_enabled("phonon", true);
        s.set_remote_allowed("openai-speech", true);
        s.record_ack("phonon", "abc", "2026-10-26T12:00:00Z");
        s.voices.insert("phonon".into(), "Zoey".into());
        s.phonon.model = "hf://gradium/phonon-40m-de".into();
        let json = serde_json::to_string(&s).unwrap();
        let back: TtsProvidersSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn freigaben_setzen_ist_idempotent_und_kann_zurueckgenommen_werden() {
        let mut s = TtsProvidersSettings::default();
        s.set_enabled("phonon", true);
        s.set_enabled("phonon", true);
        assert_eq!(s.enabled, ["phonon"]);
        s.set_enabled("phonon", false);
        s.set_enabled("phonon", false);
        assert!(s.enabled.is_empty());
        s.record_ack("a", "f1", "t1");
        s.record_ack("a", "f2", "t2");
        assert_eq!(s.license_acks.len(), 1, "eine Zustimmung je Anbieter");
        assert_eq!(s.license_ack("a").unwrap().fingerprint, "f2");
    }

    // ---- Vorrang -------------------------------------------------------------------

    #[test]
    fn env_variablen_namen_sind_dokumentiert() {
        assert_eq!(
            env_var_name("phonon", "exe_path"),
            "LVA_TTS_PHONON_EXE_PATH"
        );
        assert_eq!(
            env_var_name("openai-speech", "base_url"),
            "LVA_TTS_OPENAI_SPEECH_BASE_URL"
        );
        assert_eq!(
            env_var_name("ollama-speech", "model"),
            "LVA_TTS_OLLAMA_SPEECH_MODEL"
        );
    }

    #[test]
    fn ohne_alles_gilt_die_vorgabe_des_adapters() {
        let r = resolve_provider_configs(
            &TtsProvidersSettings::default(),
            &no_env,
            &ProviderOverrides::default(),
        );
        assert_eq!(r.phonon, PhononConfig::default());
        assert_eq!(r.openai_speech, OpenAiSpeechConfig::default());
        assert_eq!(r.ollama_speech, OllamaSpeechConfig::default());
        for (slot, field) in [("phonon", "model"), ("openai-speech", "base_url")] {
            assert_eq!(r.source(slot, field), ConfigSource::Default);
        }
    }

    #[test]
    fn einstellungen_schlagen_die_vorgabe() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "C:/modelle/phonon".into();
        s.openai_speech.base_url = "http://127.0.0.1:9000/v1".into();
        let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
        assert_eq!(r.phonon.model, "C:/modelle/phonon");
        assert_eq!(r.source("phonon", "model"), ConfigSource::Settings);
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Default);
        assert_eq!(r.openai_speech.base_url, "http://127.0.0.1:9000/v1");
        assert_eq!(
            r.source("openai-speech", "base_url"),
            ConfigSource::Settings
        );
    }

    fn installiert() -> InstalledPaths {
        InstalledPaths {
            phonon: InstalledPhonon {
                exe_path: Some("D:/app/tts/phonon/windows-x64/ptts.exe".into()),
                model: Some("D:/app/tts/phonon/models/phonon-40m-de".into()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn installiert_schlaegt_die_vorgabe_und_meldet_die_quelle() {
        let r = resolve_provider_configs_with(
            &TtsProvidersSettings::default(),
            &no_env,
            &ProviderOverrides::default(),
            &installiert(),
        );
        assert_eq!(r.phonon.exe_path, "D:/app/tts/phonon/windows-x64/ptts.exe");
        assert_eq!(r.phonon.model, "D:/app/tts/phonon/models/phonon-40m-de");
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Installed);
        assert_eq!(r.source("phonon", "model"), ConfigSource::Installed);
        // Der Gegenstand der Guardrails (Fingerabdruck) ist das installierte Modell.
        assert_eq!(
            r.subject("phonon").model,
            "D:/app/tts/phonon/models/phonon-40m-de"
        );
        assert_eq!(r.installed(), &installiert());
        assert!(ConfigSource::Default < ConfigSource::Installed);
        assert!(ConfigSource::Installed < ConfigSource::Settings);
    }

    #[test]
    fn einstellungen_schlagen_installiert_feld_fuer_feld() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "C:/eigene/modelle/phonon".into();
        let r = resolve_provider_configs_with(
            &s,
            &no_env,
            &ProviderOverrides::default(),
            &installiert(),
        );
        assert_eq!(r.phonon.model, "C:/eigene/modelle/phonon");
        assert_eq!(r.source("phonon", "model"), ConfigSource::Settings);
        assert_eq!(r.phonon.exe_path, "D:/app/tts/phonon/windows-x64/ptts.exe");
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Installed);
    }

    #[test]
    fn umgebung_und_argument_schlagen_installiert() {
        let env = env_of(&[("LVA_TTS_PHONON_MODEL", "aus-der-umgebung")]);
        let ov = ProviderOverrides {
            phonon: ProviderOverride {
                exe_path: Some("C:/arg/ptts.exe".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let r = resolve_provider_configs_with(
            &TtsProvidersSettings::default(),
            &env,
            &ov,
            &installiert(),
        );
        assert_eq!(r.phonon.model, "aus-der-umgebung");
        assert_eq!(r.source("phonon", "model"), ConfigSource::Env);
        assert_eq!(r.phonon.exe_path, "C:/arg/ptts.exe");
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Override);
    }

    #[test]
    fn eine_leere_einstellung_zaehlt_als_nicht_eingetragen() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.exe_path = "   ".into();
        let r = resolve_provider_configs_with(
            &s,
            &no_env,
            &ProviderOverrides::default(),
            &installiert(),
        );
        assert_eq!(r.phonon.exe_path, "D:/app/tts/phonon/windows-x64/ptts.exe");
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Installed);
    }

    #[test]
    fn ohne_installation_und_bei_teilinstallation_bleibt_der_rest_vorgabe() {
        let r = resolve_provider_configs_with(
            &TtsProvidersSettings::default(),
            &no_env,
            &ProviderOverrides::default(),
            &InstalledPaths::default(),
        );
        assert_eq!(r.phonon, PhononConfig::default());
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Default);

        // Nur das Programm installiert: das Modell bleibt leer (Hinweis „Modell installieren“).
        let only_exe = InstalledPaths {
            phonon: InstalledPhonon {
                exe_path: Some("D:/x/ptts.exe".into()),
                model: None,
                ..Default::default()
            },
        };
        let r = resolve_provider_configs_with(
            &TtsProvidersSettings::default(),
            &no_env,
            &ProviderOverrides::default(),
            &only_exe,
        );
        assert_eq!(r.source("phonon", "exe_path"), ConfigSource::Installed);
        assert_eq!(r.source("phonon", "model"), ConfigSource::Default);
        assert_eq!(r.phonon.model, "");
        // Die alte Funktion ist unverändert: ohne Stufe „installiert“.
        let plain = resolve_provider_configs(
            &TtsProvidersSettings::default(),
            &no_env,
            &ProviderOverrides::default(),
        );
        assert_eq!(plain.installed(), &InstalledPaths::default());
    }

    #[test]
    fn umgebung_schlaegt_einstellungen() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "aus-den-einstellungen".into();
        s.ollama_speech.model = "phonon:de".into();
        let env = env_of(&[
            ("LVA_TTS_PHONON_MODEL", "aus-der-umgebung"),
            ("LVA_TTS_PHONON_EXE_PATH", "C:/tools/ptts.exe"),
            ("LVA_TTS_OPENAI_SPEECH_BASE_URL", "http://localhost:8880/v1"),
            ("LVA_TTS_OLLAMA_SPEECH_MODEL", "phonon:en"),
        ]);
        let r = resolve_provider_configs(&s, &env, &ProviderOverrides::default());
        assert_eq!(r.phonon.model, "aus-der-umgebung");
        assert_eq!(r.source("phonon", "model"), ConfigSource::Env);
        assert_eq!(r.phonon.exe_path, "C:/tools/ptts.exe");
        assert_eq!(r.openai_speech.base_url, "http://localhost:8880/v1");
        assert_eq!(r.ollama_speech.model, "phonon:en");
    }

    #[test]
    fn override_schlaegt_umgebung_und_einstellungen() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "einstellung".into();
        let env = env_of(&[("LVA_TTS_PHONON_MODEL", "umgebung")]);
        let ov = ProviderOverrides {
            phonon: ProviderOverride {
                model: Some("argument".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let r = resolve_provider_configs(&s, &env, &ov);
        assert_eq!(r.phonon.model, "argument");
        assert_eq!(r.source("phonon", "model"), ConfigSource::Override);
        // Ein anderes Feld ohne Override fällt auf die nächste Ebene zurück.
        let env2 = env_of(&[("LVA_TTS_PHONON_EXE_PATH", "C:/x/ptts.exe")]);
        let r2 = resolve_provider_configs(&s, &env2, &ov);
        assert_eq!(r2.phonon.exe_path, "C:/x/ptts.exe");
        assert_eq!(r2.source("phonon", "exe_path"), ConfigSource::Env);
    }

    #[test]
    fn leere_werte_zaehlen_als_nicht_gesetzt() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "einstellung".into();
        let env = env_of(&[("LVA_TTS_PHONON_MODEL", "   ")]);
        let ov = ProviderOverrides {
            phonon: ProviderOverride {
                model: Some(String::new()),
                ..Default::default()
            },
            ..Default::default()
        };
        let r = resolve_provider_configs(&s, &env, &ov);
        assert_eq!(r.phonon.model, "einstellung");
        assert_eq!(r.source("phonon", "model"), ConfigSource::Settings);
    }

    #[test]
    fn nur_vier_felder_sind_ueber_die_umgebung_setzbar() {
        let env = env_of(&[
            ("LVA_TTS_PHONON_QUANT", "q8"),
            ("LVA_TTS_PHONON_ALLOW_NETWORK", "true"),
            ("LVA_TTS_PHONON_BASE_URL", "http://example.org"),
            ("LVA_TTS_OPENAI_SPEECH_ALLOW_REMOTE", "true"),
            ("LVA_TTS_OLLAMA_SPEECH_ALLOW_REMOTE", "true"),
            ("LVA_TTS_OPENAI_SPEECH_ENABLED", "true"),
        ]);
        let r = resolve_provider_configs(
            &TtsProvidersSettings::default(),
            &env,
            &ProviderOverrides::default(),
        );
        assert_eq!(r.phonon.quant, None);
        assert!(!r.phonon.allow_network);
        assert!(!r.openai_speech.allow_remote);
        assert!(!r.ollama_speech.allow_remote);
    }

    #[test]
    fn die_stimme_folgt_override_umgebung_auswahl_konfiguration() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.voice = Some("aus-der-konfiguration".into());
        let ov0 = ProviderOverrides::default();
        let r = resolve_provider_configs(&s, &no_env, &ov0);
        assert_eq!(
            r.voice_for("phonon").as_deref(),
            Some("aus-der-konfiguration")
        );

        s.voices.insert("phonon".into(), "ausgewaehlt".into());
        let r = resolve_provider_configs(&s, &no_env, &ov0);
        assert_eq!(r.voice_for("phonon").as_deref(), Some("ausgewaehlt"));
        assert_eq!(r.source("phonon", "voice"), ConfigSource::Settings);

        let env = env_of(&[("LVA_TTS_PHONON_VOICE", "umgebung")]);
        let r = resolve_provider_configs(&s, &env, &ov0);
        assert_eq!(r.voice_for("phonon").as_deref(), Some("umgebung"));
        assert_eq!(r.source("phonon", "voice"), ConfigSource::Env);

        let ov = ProviderOverrides {
            phonon: ProviderOverride {
                voice: Some("argument".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let r = resolve_provider_configs(&s, &env, &ov);
        assert_eq!(r.voice_for("phonon").as_deref(), Some("argument"));
        assert_eq!(r.source("phonon", "voice"), ConfigSource::Override);
        assert_eq!(r.voice_for("fish"), None);
    }

    #[test]
    fn adapter_schalter_folgen_nur_der_freigabeliste() {
        // Ein manipulierter Einstellungsstand trägt die Schalter selbst …
        let mut s = TtsProvidersSettings::default();
        s.openai_speech.allow_remote = true;
        s.ollama_speech.allow_remote = true;
        s.phonon.allow_network = true;
        let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
        // … wirksam wird er nur über die Liste.
        assert!(!r.openai_speech.allow_remote);
        assert!(!r.ollama_speech.allow_remote);
        assert!(!r.phonon.allow_network);

        s.set_remote_allowed("openai-speech", true);
        s.set_remote_allowed("ollama-speech", true);
        s.set_remote_allowed("phonon", true);
        let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
        assert!(r.openai_speech.allow_remote);
        assert!(r.ollama_speech.allow_remote);
        assert!(r.phonon.allow_network, "phonon: Wunsch UND Freigabe");
        // Ohne den Wunsch bleibt auch mit Freigabe alles offline.
        s.phonon.allow_network = false;
        let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
        assert!(!r.phonon.allow_network);
    }

    #[test]
    fn eine_eingebaute_kennung_wird_fuer_openai_speech_ersetzt() {
        for bad in [
            "fish",
            "piper",
            "phonon",
            "ollama-speech",
            "mock",
            "",
            "GROSS",
        ] {
            let mut s = TtsProvidersSettings::default();
            s.openai_speech.id = bad.into();
            let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
            assert_eq!(r.openai_speech.id, "openai-speech", "{bad:?}");
        }
        let mut s = TtsProvidersSettings::default();
        s.openai_speech.id = "kokoro".into();
        let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
        assert_eq!(r.openai_speech.id, "kokoro");
        // Die Umgebung heißt trotzdem nach dem Platz, nicht nach der Kennung.
        let env = env_of(&[("LVA_TTS_OPENAI_SPEECH_MODEL", "m1")]);
        let r = resolve_provider_configs(&s, &env, &ProviderOverrides::default());
        assert_eq!(r.openai_speech.model, "m1");
    }

    #[test]
    fn subject_zeigt_die_wirksame_konfiguration() {
        let mut s = TtsProvidersSettings::default();
        s.phonon.model = "einstellung".into();
        let env = env_of(&[
            ("LVA_TTS_PHONON_MODEL", "umgebung"),
            (
                "LVA_TTS_OPENAI_SPEECH_BASE_URL",
                "https://tts.example.org/v1",
            ),
        ]);
        let r = resolve_provider_configs(&s, &env, &ProviderOverrides::default());
        assert_eq!(r.subject("phonon").model, "umgebung");
        assert_eq!(r.subject("phonon").base_url, None);
        assert_eq!(
            r.subject("openai-speech").base_url.as_deref(),
            Some("https://tts.example.org/v1")
        );
        assert_eq!(r.subject("fish"), UseSubject::default());
        assert_eq!(r.subject("piper"), UseSubject::default());
    }

    // ---- Fingerabdruck -------------------------------------------------------------

    #[test]
    fn der_fingerabdruck_aendert_sich_mit_modell_lizenz_und_kennung() {
        let d = nc_desc();
        let base = license_fingerprint(&d, &subject("hf://gradium/phonon-40m-de"));
        assert_eq!(base.len(), 32);
        assert_eq!(
            base,
            license_fingerprint(&d, &subject("hf://gradium/phonon-40m-de"))
        );
        assert_eq!(
            base,
            license_fingerprint(&d, &subject("  hf://gradium/phonon-40m-de ")),
            "Leerraum zählt nicht"
        );
        assert_ne!(
            base,
            license_fingerprint(&d, &subject("hf://gradium/phonon-40m-en"))
        );
        let mut other = d.clone();
        other.license.weights = "Apache-2.0".into();
        assert_ne!(
            base,
            license_fingerprint(&other, &subject("hf://gradium/phonon-40m-de"))
        );
        let mut other = d.clone();
        other.license.non_commercial = false;
        assert_ne!(
            base,
            license_fingerprint(&other, &subject("hf://gradium/phonon-40m-de"))
        );
        let mut other = d.clone();
        other.id = "anderer".into();
        assert_ne!(
            base,
            license_fingerprint(&other, &subject("hf://gradium/phonon-40m-de"))
        );
        // Nicht in den Fingerabdruck gehen Beschriftung und Notiz ein.
        let mut other = d.clone();
        other.label = "Anderer Name".into();
        other.license.note = "neue Notiz".into();
        assert_eq!(
            base,
            license_fingerprint(&other, &subject("hf://gradium/phonon-40m-de"))
        );
    }

    /// Der Fingerabdruck wird gespeichert: dieser Referenzwert schlägt an,
    /// wenn jemand den Algorithmus ändert und damit jede Zustimmung entwertet.
    #[test]
    fn der_fingerabdruck_ist_ueber_versionen_stabil() {
        let d = nc_desc();
        assert_eq!(
            license_fingerprint(&d, &subject("hf://gradium/phonon-40m-de")),
            "8aaa9092cd71e832af91f1fe676b66a5"
        );
    }

    // ---- Guardrails ---------------------------------------------------------------

    #[test]
    fn opt_in_anbieter_ohne_freischaltung_ist_gesperrt() {
        let d = MockProvider::new().with_id("zzz").descriptor();
        assert!(d.opt_in);
        let e =
            check_use(&d, &UseSubject::default(), &TtsProvidersSettings::default()).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        assert!(e.message.contains("nicht freigeschaltet"), "{}", e.message);
        assert!(!e.retryable);
        // Freigeschaltet und lizenzrein (verified, nicht NC): erlaubt.
        assert!(check_use(&d, &UseSubject::default(), &enabled(&["zzz"])).is_ok());
    }

    #[test]
    fn nichtkommerziell_braucht_eine_zustimmung() {
        let d = nc_desc();
        let subj = subject("hf://gradium/phonon-40m-de");
        let mut s = enabled(&["phonon"]);
        let e = check_use(&d, &subj, &s).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        assert!(e.message.contains("bestätigt"), "{}", e.message);
        ack(&mut s, &d, &subj);
        assert!(check_use(&d, &subj, &s).is_ok());
    }

    #[test]
    fn ungeprueft_braucht_eine_zustimmung_auch_ohne_nc() {
        let mut d = MockProvider::new().with_id("phonon").descriptor();
        d.license.non_commercial = false;
        d.license.verified = false;
        let subj = subject("m");
        let mut s = enabled(&["phonon"]);
        assert_eq!(
            check_use(&d, &subj, &s).unwrap_err().kind,
            TtsErrorKind::LicenseBlocked
        );
        ack(&mut s, &d, &subj);
        assert!(check_use(&d, &subj, &s).is_ok());
    }

    #[test]
    fn eine_zustimmung_mit_altem_fingerabdruck_gilt_nicht_mehr() {
        let d = nc_desc();
        let old = subject("hf://gradium/phonon-40m-de");
        let new = subject("hf://gradium/phonon-40m-en");
        let mut s = enabled(&["phonon"]);
        ack(&mut s, &d, &old);
        assert!(check_use(&d, &old, &s).is_ok());
        let e = check_use(&d, &new, &s).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::LicenseBlocked);
        assert!(e.message.contains("geändert"), "{}", e.message);
        // Auch eine geänderte Gewichts-Lizenz entwertet sie.
        let mut d2 = d.clone();
        d2.license.weights = "etwas anderes".into();
        assert_eq!(
            check_use(&d2, &old, &s).unwrap_err().kind,
            TtsErrorKind::LicenseBlocked
        );
        // Neu bestätigen heilt es.
        ack(&mut s, &d, &new);
        assert!(check_use(&d, &new, &s).is_ok());
    }

    #[test]
    fn eine_zustimmung_ersetzt_nicht_das_opt_in() {
        let d = nc_desc();
        let subj = subject("m");
        let mut s = TtsProvidersSettings::default();
        ack(&mut s, &d, &subj);
        let e = check_use(&d, &subj, &s).unwrap_err();
        assert!(e.message.contains("nicht freigeschaltet"), "{}", e.message);
    }

    #[test]
    fn eine_entfernte_adresse_braucht_die_freigabe() {
        let mut d = MockProvider::new().with_id("openai-speech").descriptor();
        d.license.verified = true;
        d.license.non_commercial = false;
        let remote = UseSubject {
            model: "m".into(),
            base_url: Some("https://tts.example.org/v1".into()),
        };
        let mut s = enabled(&["openai-speech"]);
        let e = check_use(&d, &remote, &s).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::PrivacyBlocked);
        s.set_remote_allowed("openai-speech", true);
        assert!(check_use(&d, &remote, &s).is_ok());
        // Die Freigabe gilt nur für diesen Anbieter.
        let other = MockProvider::new().with_id("ollama-speech").descriptor();
        let mut s2 = enabled(&["ollama-speech"]);
        s2.set_remote_allowed("openai-speech", true);
        assert_eq!(
            check_use(&other, &remote, &s2).unwrap_err().kind,
            TtsErrorKind::PrivacyBlocked
        );
    }

    #[test]
    fn loopback_adressen_brauchen_keine_freigabe() {
        let d = MockProvider::new().with_id("openai-speech").descriptor();
        let s = enabled(&["openai-speech"]);
        for url in [
            "http://127.0.0.1:8880/v1",
            "http://localhost:11434",
            "http://[::1]:8880/v1",
            "http://127.1.2.3/v1",
        ] {
            let subj = UseSubject {
                model: "m".into(),
                base_url: Some(url.into()),
            };
            assert!(check_use(&d, &subj, &s).is_ok(), "{url}");
        }
    }

    #[test]
    fn eine_unlesbare_adresse_ist_ein_konfigurationsfehler() {
        let d = MockProvider::new().with_id("openai-speech").descriptor();
        let s = enabled(&["openai-speech"]);
        let subj = UseSubject {
            model: "m".into(),
            base_url: Some("kein url".into()),
        };
        assert_eq!(
            check_use(&d, &subj, &s).unwrap_err().kind,
            TtsErrorKind::InvalidConfig
        );
    }

    #[test]
    fn die_datenschutzsperre_gilt_auch_ohne_opt_in() {
        let mut d = MockProvider::new().with_id("zzz").descriptor();
        d.opt_in = false;
        let subj = UseSubject {
            model: String::new(),
            base_url: Some("http://192.168.1.20:8880/v1".into()),
        };
        assert_eq!(
            check_use(&d, &subj, &TtsProvidersSettings::default())
                .unwrap_err()
                .kind,
            TtsErrorKind::PrivacyBlocked
        );
    }

    #[test]
    fn fish_und_piper_sind_nie_gesperrt() {
        let fish = FishProvider::new("C:/gibt/es/nicht").descriptor();
        let piper = PiperProvider::new(None).descriptor();
        for d in [&fish, &piper] {
            assert!(!d.opt_in, "{}", d.id);
            // Fish: nichtkommerziell (Fish Audio Research License, belegt),
            // Piper: ungeprüft — beides würde bei einem Opt-in-Anbieter sperren,
            // bei den eingebauten Standards darf es das nie.
            assert!(
                d.license.non_commercial || !d.license.verified,
                "{}: die Lizenz würde einen Opt-in-Anbieter sperren, und trotzdem darf hier nichts sperren",
                d.id
            );
            assert!(!license_needs_ack(d));
            assert!(
                check_use(d, &UseSubject::default(), &TtsProvidersSettings::default()).is_ok(),
                "{}",
                d.id
            );
            let st = use_state(d, &UseSubject::default(), &TtsProvidersSettings::default());
            assert!(st.enabled && st.blocked.is_none() && !st.license_needs_ack);
        }
    }

    #[test]
    fn die_sperrmatrix_im_ueberblick() {
        // (opt_in, nc, verified, enabled, acked, remote, remote_allowed) → Ergebnis
        struct Case(
            bool,
            bool,
            bool,
            bool,
            bool,
            bool,
            bool,
            Option<TtsErrorKind>,
        );
        use TtsErrorKind::{LicenseBlocked as L, PrivacyBlocked as P};
        let cases = [
            Case(false, true, false, false, false, false, false, None),
            Case(false, false, false, false, false, true, false, Some(P)),
            Case(false, false, false, false, false, true, true, None),
            Case(true, false, true, false, false, false, false, Some(L)),
            Case(true, false, true, true, false, false, false, None),
            Case(true, true, true, true, false, false, false, Some(L)),
            Case(true, true, true, true, true, false, false, None),
            Case(true, false, false, true, false, false, false, Some(L)),
            Case(true, false, false, true, true, false, false, None),
            Case(true, true, false, true, true, true, false, Some(P)),
            Case(true, true, false, true, true, true, true, None),
            // Lizenz-Sperre kommt vor der Datenschutz-Sperre.
            Case(true, true, false, true, false, true, false, Some(L)),
        ];
        for (i, Case(opt_in, nc, verified, en, acked, remote, remote_ok, want)) in
            cases.into_iter().enumerate()
        {
            let mut d = MockProvider::new().with_id("zzz").descriptor();
            d.opt_in = opt_in;
            d.license.non_commercial = nc;
            d.license.verified = verified;
            let subj = UseSubject {
                model: "m".into(),
                base_url: remote.then(|| "https://tts.example.org/v1".to_string()),
            };
            let mut s = TtsProvidersSettings::default();
            s.set_enabled("zzz", en);
            if acked {
                ack(&mut s, &d, &subj);
            }
            s.set_remote_allowed("zzz", remote_ok);
            let got = check_use(&d, &subj, &s).err().map(|e| e.kind);
            assert_eq!(got, want, "Fall {i}");
        }
    }

    #[test]
    fn use_state_beschreibt_den_zustand_fuer_die_oberflaeche() {
        let d = nc_desc();
        let subj = subject("m1");
        let mut s = enabled(&["phonon"]);
        let st = use_state(&d, &subj, &s);
        assert!(st.enabled && st.license_needs_ack && !st.license_acked);
        assert!(!st.allow_remote && !st.remote);
        assert_eq!(
            st.blocked.as_ref().unwrap().kind,
            TtsErrorKind::LicenseBlocked
        );
        assert_eq!(st.fingerprint, license_fingerprint(&d, &subj));
        ack(&mut s, &d, &subj);
        let st = use_state(&d, &subj, &s);
        assert!(st.license_acked && st.blocked.is_none());
        // Modell geändert: die Zustimmung ist da, gilt aber nicht mehr.
        let st = use_state(&d, &subject("m2"), &s);
        assert!(!st.license_acked && st.blocked.is_some());
    }

    // ---- Konfiguration schreiben --------------------------------------------------

    fn phonon_json() -> String {
        r#"{"exe_path":"C:/tools/ptts.exe","model":"hf://gradium/phonon-40m-de@abc","lang":"de"}"#
            .to_string()
    }

    #[test]
    fn gueltige_konfiguration_wird_gespeichert_und_wieder_gelesen() {
        let mut s = TtsProvidersSettings::default();
        apply_config_json(&mut s, "phonon", &phonon_json()).unwrap();
        assert_eq!(s.phonon.exe_path, "C:/tools/ptts.exe");
        let back = config_json(&s, "phonon").unwrap();
        let cfg: PhononConfig = serde_json::from_str(&back).unwrap();
        assert_eq!(cfg, s.phonon);
    }

    #[test]
    fn leere_pfade_heissen_app_installation_und_werden_gespeichert() {
        let mut s = TtsProvidersSettings::default();
        apply_config_json(
            &mut s,
            "phonon",
            r#"{"exe_path":"","model":"","quant":"q8","lang":"de","threads":4}"#,
        )
        .unwrap();
        assert_eq!(s.phonon.exe_path, "");
        assert_eq!(s.phonon.model, "");
        assert_eq!(s.phonon.threads, Some(4));
        // Gesetzte Werte werden weiter geprüft.
        let e = apply_config_json(&mut s, "phonon", r#"{"exe_path":"","model":"-x"}"#).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
    }

    #[test]
    fn ungueltige_konfiguration_wird_abgelehnt_und_aendert_nichts() {
        let mut s = TtsProvidersSettings::default();
        let before = s.clone();
        let e = apply_config_json(
            &mut s,
            "phonon",
            r#"{"exe_path":"","model":"","threads":999}"#,
        )
        .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
        let e = apply_config_json(&mut s, "phonon", "kein json").unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
        let e = apply_config_json(
            &mut s,
            "openai-speech",
            r#"{"base_url":"ftp://x/v1","model":"m"}"#,
        )
        .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::InvalidConfig);
        assert_eq!(s, before, "ein Fehler lässt die Einstellungen unberührt");
    }

    #[test]
    fn unbekannte_anbieter_haben_keine_konfiguration() {
        let mut s = TtsProvidersSettings::default();
        for id in ["fish", "piper", "mock", "gibt-es-nicht"] {
            assert_eq!(
                apply_config_json(&mut s, id, "{}").unwrap_err().kind,
                TtsErrorKind::BadRequest,
                "{id}"
            );
            assert!(config_json(&s, id).is_err());
        }
    }

    /// Hintertür zu: eine Konfiguration kann den Fernzugriff nicht selbst freigeben.
    #[test]
    fn eine_konfiguration_kann_den_fernzugriff_nicht_freigeben() {
        let mut s = TtsProvidersSettings::default();
        apply_config_json(
            &mut s,
            "openai-speech",
            r#"{"base_url":"https://tts.example.org/v1","model":"m","allow_remote":true}"#,
        )
        .unwrap();
        assert!(!s.openai_speech.allow_remote);
        assert!(s.allow_remote.is_empty());
        apply_config_json(
            &mut s,
            "ollama-speech",
            r#"{"base_url":"http://192.168.1.5:11434","allow_remote":true}"#,
        )
        .unwrap();
        assert!(!s.ollama_speech.allow_remote);
        let r = resolve_provider_configs(&s, &no_env, &ProviderOverrides::default());
        assert!(!r.openai_speech.allow_remote && !r.ollama_speech.allow_remote);
        // Die entfernte Adresse ist gespeichert, benutzbar ist sie trotzdem nicht.
        let d = MockProvider::new().with_id("openai-speech").descriptor();
        assert_eq!(
            check_use(
                &d,
                &r.subject("openai-speech"),
                &enabled(&["openai-speech"])
            )
            .unwrap_err()
            .kind,
            TtsErrorKind::PrivacyBlocked
        );
    }

    #[test]
    fn eine_eingebaute_kennung_ist_fuer_openai_speech_verboten() {
        let mut s = TtsProvidersSettings::default();
        for id in ["fish", "piper", "phonon", "ollama-speech", "mock"] {
            let json = format!(r#"{{"id":"{id}","base_url":"http://127.0.0.1:1/v1","model":"m"}}"#);
            let e = apply_config_json(&mut s, "openai-speech", &json).unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::InvalidConfig, "{id}");
        }
        // Eine eigene Kennung ist erlaubt und macht den Platz unter dieser Kennung erreichbar.
        apply_config_json(
            &mut s,
            "openai-speech",
            r#"{"id":"kokoro","base_url":"http://127.0.0.1:8880/v1","model":"kokoro"}"#,
        )
        .unwrap();
        assert_eq!(config_slot(&s, "kokoro"), Some(ConfigSlot::OpenAiSpeech));
    }
}
