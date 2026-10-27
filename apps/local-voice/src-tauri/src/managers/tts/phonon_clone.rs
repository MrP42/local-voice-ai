//! „Phonon-Stimme erstellen“: der Ablauf rund um den Gradium-Aufruf (Einwilligung, Quelle,
//! Schlüssel, Auftragssperre, Ablage) — ohne Tauri, damit er gegen einen Fake-Server testbar ist.
//!
//! Die Befehle in `commands/phonon_voices.rs` sind dünne Hüllen darüber.
//!
//! ## Reihenfolge und Sperren (jede vor dem ersten Netzbyte)
//!
//! 1. **Einwilligung**: ohne gesetztes Häkchen (`consent`) geschieht nichts — es wird weder der
//!    Schlüssel gelesen noch eine Datei geöffnet noch eine Verbindung aufgebaut.
//! 2. **Eingaben**: Sprache, Name (frei/vergeben?), Startsekunde.
//! 3. **Auftragssperre**: höchstens EIN Auftrag gleichzeitig (Doppelklick).
//! 4. **Quelle → 10-Sekunden-Stück** (`phonon_clip`): Datei, Fish-Referenz oder Mikrofon; Stille
//!    und zu kurze Stücke scheitern hier.
//! 5. **Schlüssel** aus dem Geheimnisspeicher (nie aus den Einstellungen); fehlt er: Fehler.
//! 6. erst dann **Gradium** ([`super::gradium`]) und danach die Ablage über
//!    [`super::phonon_voices::VoiceStore`].
//!
//! ## Schlüsselspeicher
//!
//! Der Schlüssel liegt DPAPI-verschlüsselt im Geheimnisspeicher der App
//! (`managers::calendar::secret`, Integrationsraum `int-tts-gradium-apikey.bin`), nie in
//! `settings_store.json`. Die Oberfläche bekommt ihn nie zurück, nur [`KeyStatus`]. Unter
//! Systemen ohne DPAPI wird er nicht gespeichert (kein Klartext-Ersatz).
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Doppelklick / zweiter Auftrag | `JobSlot::claim` (atomar) → `busy`; Test `ein_zweiter_auftrag_…` |
//! | Abbruch durch den Nutzer | `JobSlot::cancel` setzt die Marke, Anfragen enden binnen ~50 ms, nichts wird gespeichert, der Platz ist frei; Test `abbruch_…` |
//! | Absturz im Auftrag (Panic) | `JobGuard` gibt den Platz beim Aufräumen frei; Test `der_platz_wird_auch_nach_einem_panic_frei` |
//! | Datei weg / nicht dekodierbar / ffmpeg fehlt | Meldung vor dem Netz, Temp-Datei fällt per Drop weg; Test `unbrauchbare_quellen_…` |
//! | Gradium-Fehler, Netz weg | typisierte Meldung, lokal bleibt alles, wie es war; Tests `fehler_von_gradium_…` |
//! | Ungültige Datei von Gradium | wird vor der Ablage geprüft und verworfen; Test `eine_ungueltige_stimmdatei_…` |
//! | Name in der Zwischenzeit vergeben | `VoiceStore::add` prüft unter der Sperre erneut; die Stimme bleibt in Gradiums Konto |

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::gradium::{self, CancelFlag, Client, CloneJob, GradiumError, GradiumErrorKind, Phase};
use super::phonon_clip::{self, Clip, ClipError};
use super::phonon_voices::{self, NewVoice, OwnVoice, VoiceError, VoiceStore, ORIGIN_GRADIUM};
use super::{registry, voices};
use crate::managers::calendar::secret::{self, SecretRef, SecretStatus};
use crate::managers::model::download::Secret;

/// Größte Audiodatei, die als Quelle gewählt werden darf (Dekodieren dauert sonst Minuten).
const MAX_SOURCE_FILE_BYTES: u64 = 200 * 1024 * 1024;
const AUDIO_EXTENSIONS: [&str; 8] = ["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "wma"];
/// Abtastrate, mit der ffmpeg Nicht-WAV-Quellen dekodiert (Phonon rechnet mit 24 kHz).
const DECODE_RATE: u32 = 24_000;
/// Kürzere Schlüssel sind Tippfehler, und das Bereinigen von Meldungen würde sie zerhacken.
const MIN_KEY_LEN: usize = 8;

// ---- Fehler, Anfrage -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PhononVoiceErrorKind {
    ConsentRequired,
    NoKey,
    KeyStore,
    Busy,
    InvalidName,
    InvalidLanguage,
    Exists,
    TakenByModel,
    InvalidFile,
    BadSource,
    NotOwn,
    TooMany,
    Io,
    // wie `GradiumErrorKind`
    Unauthorized,
    QuotaOrRate,
    BadAudio,
    Server,
    Timeout,
    Network,
    BadResponse,
    Cancelled,
}

/// Fehler der Stimmenverwaltung für die Oberfläche: `kind` steuert die Anzeige, `message` ist
/// deutsch, ohne Schlüssel und ohne Adresse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct PhononVoiceError {
    pub kind: PhononVoiceErrorKind,
    pub message: String,
}

impl PhononVoiceError {
    pub fn new(kind: PhononVoiceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PhononVoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PhononVoiceError {}

impl From<VoiceError> for PhononVoiceError {
    fn from(e: VoiceError) -> Self {
        use PhononVoiceErrorKind as K;
        let kind = match &e {
            VoiceError::InvalidName(_) => K::InvalidName,
            VoiceError::Exists { .. } => K::Exists,
            VoiceError::TakenByModel { .. } => K::TakenByModel,
            VoiceError::InvalidFile(_) => K::InvalidFile,
            VoiceError::NotOwn(_) => K::NotOwn,
            VoiceError::TooMany => K::TooMany,
            VoiceError::Io(_) => K::Io,
        };
        Self::new(kind, e.to_string())
    }
}

impl From<GradiumError> for PhononVoiceError {
    fn from(e: GradiumError) -> Self {
        use PhononVoiceErrorKind as K;
        let kind = match e.kind {
            GradiumErrorKind::Unauthorized => K::Unauthorized,
            GradiumErrorKind::QuotaOrRate => K::QuotaOrRate,
            GradiumErrorKind::BadAudio => K::BadAudio,
            GradiumErrorKind::Server => K::Server,
            GradiumErrorKind::Timeout => K::Timeout,
            GradiumErrorKind::Network => K::Network,
            GradiumErrorKind::BadResponse => K::BadResponse,
            GradiumErrorKind::Cancelled => K::Cancelled,
        };
        Self::new(kind, e.message)
    }
}

impl From<ClipError> for PhononVoiceError {
    fn from(e: ClipError) -> Self {
        Self::new(PhononVoiceErrorKind::BadSource, e.to_string())
    }
}

/// Woher die Stimme kommt.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CloneSource {
    /// Audiodatei (wav, mp3, m4a, flac, …).
    File { path: String },
    /// Referenzaufnahme einer vorhandenen Fish-Stimme.
    Fish { voice_id: String },
    /// Die letzte Mikrofonaufnahme (`phonon_voice_record_*`).
    Recording,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct CloneRequest {
    pub source: CloneSource,
    /// Anzeigename der neuen Stimme.
    pub name: String,
    /// Sprache der Aufnahme: `en`, `fr`, `de`, `es`, `pt`.
    pub language: String,
    /// Sekunde, ab der die Sprache beginnt (aus Datei oder Fish-Referenz; bei Mikrofon 0).
    pub start_s: f32,
    /// Die sprechende Person hat eingewilligt (Häkchen). Ohne `true` geschieht nichts.
    pub consent: bool,
    /// Eine vorhandene eigene Stimme dieses Namens ersetzen.
    pub overwrite: bool,
}

/// Eine Mikrofonaufnahme, die auf ihre Verwendung wartet (16 kHz mono).
#[derive(Clone)]
pub struct Recording {
    pub samples: Vec<f32>,
    pub rate: u32,
}

/// Die wartende Aufnahme: von `phonon_voice_record_stop` gefüllt, von der Erstellung gelesen.
pub static PENDING_RECORDING: Mutex<Option<Recording>> = Mutex::new(None);

// ---- Auftragssperre ---------------------------------------------------------------------------------------

/// Höchstens ein Auftrag gleichzeitig; hält die Abbruchmarke des laufenden.
pub struct JobSlot {
    running: AtomicBool,
    cancel: Mutex<Option<CancelFlag>>,
}

impl JobSlot {
    pub const fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            cancel: Mutex::new(None),
        }
    }

    /// Belegt den Platz, wenn er frei ist (atomar); der Wächter gibt ihn beim Verlassen frei.
    pub fn claim(&self) -> Option<JobGuard<'_>> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        let flag: CancelFlag = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some(flag.clone());
        Some(JobGuard {
            slot: self,
            cancel: flag,
        })
    }

    /// Setzt die Abbruchmarke des laufenden Auftrags (kein Auftrag: nichts).
    pub fn cancel(&self) {
        if let Some(flag) = self
            .cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Default for JobSlot {
    fn default() -> Self {
        Self::new()
    }
}

pub struct JobGuard<'a> {
    slot: &'a JobSlot,
    pub cancel: CancelFlag,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        *self.slot.cancel.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.slot.running.store(false, Ordering::SeqCst);
    }
}

// ---- Schlüsselspeicher -----------------------------------------------------------------------------------------

fn key_ref() -> SecretRef {
    SecretRef::integration("tts-gradium", "apikey")
}

/// Zustand des Schlüssels für die Oberfläche; nie der Schlüssel selbst.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct KeyStatus {
    /// Dieses System kann Geheimnisse verschlüsselt ablegen (Windows/DPAPI).
    pub supported: bool,
    pub present: bool,
    /// Ein Schlüssel liegt vor, ist aber nicht lesbar (anderes Konto, beschädigt).
    pub problem: Option<String>,
}

pub fn key_status(dir: &Path) -> KeyStatus {
    let (present, problem) = match secret::status_ref_in(dir, &key_ref()) {
        SecretStatus::Present => (true, None),
        SecretStatus::Missing => (false, None),
        SecretStatus::Broken(why) => (false, Some(why)),
    };
    KeyStatus {
        supported: cfg!(windows),
        present,
        problem,
    }
}

/// Schlüssel verschlüsselt ablegen. Ungültige Werte (Steuerzeichen, Leerzeichen, zu kurz oder zu
/// lang) werden abgelehnt, ohne etwas zu schreiben; ohne DPAPI gibt es einen Fehler, nie Klartext.
pub fn save_key(dir: &Path, raw: &str) -> Result<(), PhononVoiceError> {
    use PhononVoiceErrorKind as K;
    let trimmed = raw.trim();
    let valid = trimmed.len() >= MIN_KEY_LEN && Secret::new(trimmed).is_some();
    if !valid {
        return Err(PhononVoiceError::new(
            K::KeyStore,
            "Der Schlüssel sieht nicht richtig aus (mindestens 8 sichtbare Zeichen, keine Leerzeichen).",
        ));
    }
    std::fs::create_dir_all(dir).map_err(|_| {
        PhononVoiceError::new(
            K::KeyStore,
            "Der Ordner für Geheimnisse lässt sich nicht anlegen.",
        )
    })?;
    secret::put_ref_in(dir, &key_ref(), trimmed.as_bytes())
        .map_err(|e| PhononVoiceError::new(K::KeyStore, e))
}

/// Gespeicherten Schlüssel laden (`Ok(None)`: keiner gespeichert).
pub fn load_key(dir: &Path) -> Result<Option<Secret>, String> {
    let Some(bytes) = secret::get_ref_in(dir, &key_ref())? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| "Der gespeicherte Schlüssel ist beschädigt.".to_string())?;
    Secret::new(&text)
        .map(Some)
        .ok_or_else(|| "Der gespeicherte Schlüssel ist beschädigt.".to_string())
}

pub fn delete_key(dir: &Path) {
    secret::delete_ref_in(dir, &key_ref());
}

// ---- Ablauf ------------------------------------------------------------------------------------------------------------

/// Alles, was der Ablauf von außen braucht.
pub struct CloneEnv<'a> {
    pub store: VoiceStore<'a>,
    pub fish_dir: &'a Path,
    pub gradium: gradium::Config,
    /// Liefert den Schlüssel erst, wenn alle Sperren durch sind.
    pub key: &'a (dyn Fn() -> Result<Option<Secret>, String> + Send + Sync),
    pub recording: &'a Mutex<Option<Recording>>,
}

fn decode_message(e: &str) -> String {
    if e.contains("ffmpeg nicht gefunden") {
        "Für diese Audiodatei wird ffmpeg gebraucht (Installation z. B.: winget install ffmpeg). WAV-Dateien gehen ohne.".to_string()
    } else {
        "Die Audiodatei lässt sich nicht dekodieren (beschädigt oder kein Audio).".to_string()
    }
}

fn bad_source(message: impl Into<String>) -> PhononVoiceError {
    PhononVoiceError::new(PhononVoiceErrorKind::BadSource, message)
}

/// Quelle → das 10-Sekunden-Stück. Blockiert (ffmpeg, Dateizugriff): im Blocking-Thread rufen.
fn resolve_clip(
    env: &CloneEnv<'_>,
    source: &CloneSource,
    start_s: f32,
    cancel: &AtomicBool,
) -> Result<Clip, PhononVoiceError> {
    match source {
        CloneSource::File { path } => {
            let p = Path::new(path);
            let meta = std::fs::metadata(p)
                .map_err(|_| bad_source("Die Audiodatei lässt sich nicht öffnen."))?;
            if !meta.is_file() {
                return Err(bad_source("Das ist keine Datei."));
            }
            if meta.len() > MAX_SOURCE_FILE_BYTES {
                return Err(bad_source("Die Audiodatei ist zu groß (höchstens 200 MB)."));
            }
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                return Err(bad_source(format!(
                    "Dieses Dateiformat wird nicht unterstützt (erlaubt: {}).",
                    AUDIO_EXTENSIONS.join(", ")
                )));
            }
            // WAV geht direkt durch; alles andere dekodiert ffmpeg in eine Temp-Datei, die beim
            // Verlassen (auch im Fehlerfall) gelöscht wird.
            // Der Abbruch des Nutzers erreicht auch ffmpeg (`DECODE_CANCELLED`).
            let (wav, _tmp) = crate::media::ensure_wav_cancellable(p, DECODE_RATE, cancel)
                .map_err(|e| {
                    if e == crate::media::DECODE_CANCELLED {
                        PhononVoiceError::new(PhononVoiceErrorKind::Cancelled, "Abgebrochen.")
                    } else {
                        bad_source(decode_message(&e))
                    }
                })?;
            Ok(phonon_clip::clip_from_wav_file(&wav, start_s)?)
        }
        CloneSource::Fish { voice_id } => {
            let id = registry::require_known_voice(env.fish_dir, voice_id)
                .map_err(|_| bad_source("Diese Fish-Stimme gibt es nicht (mehr)."))?;
            let (wav, _transcript) = voices::voice_sample(env.fish_dir, &id)
                .ok_or_else(|| bad_source("Die Fish-Stimme hat keine Referenzaufnahme."))?;
            Ok(phonon_clip::clip_from_wav_file(&wav, start_s)?)
        }
        CloneSource::Recording => {
            let rec = env
                .recording
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .ok_or_else(|| {
                    bad_source("Es liegt keine Aufnahme vor. Bitte zuerst aufnehmen.")
                })?;
            Ok(phonon_clip::clip_from_samples(
                &rec.samples,
                rec.rate,
                start_s,
            )?)
        }
    }
}

fn source_note(source: &CloneSource) -> String {
    match source {
        CloneSource::File { .. } => "Audiodatei".to_string(),
        CloneSource::Fish { voice_id } => format!("Fish-Stimme: {voice_id}"),
        CloneSource::Recording => "Mikrofonaufnahme".to_string(),
    }
}

/// Eine Stimme über Gradium erstellen und lokal ablegen. Siehe Moduldoku für die Reihenfolge.
pub async fn create_voice(
    env: &CloneEnv<'_>,
    slot: &JobSlot,
    req: CloneRequest,
    on_phase: &(dyn Fn(Phase) + Send + Sync),
) -> Result<OwnVoice, PhononVoiceError> {
    use PhononVoiceErrorKind as K;
    // 1. Einwilligung: vor allem anderen.
    if !req.consent {
        return Err(PhononVoiceError::new(
            K::ConsentRequired,
            "Ohne die Einwilligung der sprechenden Person wird nichts gesendet. Bitte das Häkchen setzen, wenn sie eingewilligt hat.",
        ));
    }
    // 2. Eingaben.
    if !gradium::LANGUAGES.contains(&req.language.as_str()) {
        return Err(PhononVoiceError::new(
            K::InvalidLanguage,
            format!(
                "Gradium unterstützt diese Sprachen: {}.",
                gradium::LANGUAGES.join(", ")
            ),
        ));
    }
    let name = env.store.check_name(&req.name, req.overwrite)?;
    // 3. Auftragssperre.
    let job = slot.claim().ok_or_else(|| {
        PhononVoiceError::new(
            K::Busy,
            "Es läuft schon eine Stimmenerstellung. Bitte warten oder abbrechen.",
        )
    })?;
    // 4. Das 10-Sekunden-Stück (blockierend).
    let clip =
        tokio::task::block_in_place(|| resolve_clip(env, &req.source, req.start_s, &job.cancel))?;
    // 5. Schlüssel.
    let key = (env.key)()
        .map_err(|e| PhononVoiceError::new(K::KeyStore, e))?
        .ok_or_else(|| {
            PhononVoiceError::new(
                K::NoKey,
                "Es ist noch kein Gradium-API-Schlüssel eingetragen (Einstellungen > Ausgabe > Phonon).",
            )
        })?;
    // 6. Gradium, dann ablegen.
    let client = Client::new(env.gradium.clone(), key)?;
    let cloned = client
        .clone_voice(
            &CloneJob {
                name: name.display.clone(),
                language: req.language.clone(),
                wav: clip.wav,
            },
            &job.cancel,
            on_phase,
        )
        .await?;
    if phonon_voices::validate_voice_bytes(&cloned.safetensors).is_err() {
        return Err(PhononVoiceError::new(
            K::BadResponse,
            format!(
                "Gradium hat keine gültige Phonon-Stimmdatei geliefert. Die Stimme „{}“ liegt in Ihrem Gradium-Konto.",
                cloned.voice_id
            ),
        ));
    }
    let voice = env
        .store
        .add(
            NewVoice {
                name: req.name.clone(),
                bytes: cloned.safetensors,
                origin: ORIGIN_GRADIUM,
                voice_id: cloned.voice_id.clone(),
                language: req.language.clone(),
                consent_at: Some(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                source_note: Some(source_note(&req.source)),
            },
            req.overwrite,
        )
        .map_err(|e| {
            let mut e = PhononVoiceError::from(e);
            e.message = format!(
                "{} Die Stimme „{}“ liegt in Ihrem Gradium-Konto.",
                e.message, cloned.voice_id
            );
            e
        })?;
    if matches!(req.source, CloneSource::Recording) {
        *env.recording.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    Ok(voice)
}

#[cfg(test)]
mod tests;
