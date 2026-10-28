//! Tauri-Befehle für eigene Phonon-Stimmen: importieren, mit Gradium erstellen, entfernen.
//!
//! Die Regeln stehen in `managers::tts::{phonon_voices, phonon_clone, gradium, phonon_clip}`
//! (rein und dort getestet); die Befehle hier sind dünne Hüllen.
//!
//! ## NUR für die Oberfläche
//!
//! **`phonon_voice_clone`, `phonon_voice_clone_cancel`, `phonon_gradium_key_*` und
//! `phonon_voice_record_*` sind ein Klick des Nutzers und dürfen NIE in die Agentenbrücke, in
//! eine CLI-Automatik, in einen MCP-Server, in den Start oder in eine Gesundheitsprüfung
//! aufgenommen werden.** `phonon_voice_clone` sendet eine Aufnahme der Stimme einer Person an
//! einen Dienst im Netz (Gradium) und legt sie dort in einem Konto ab; das geschieht nur nach
//! Hinweis und bestätigter Einwilligung der sprechenden Person (`consent`), und ein Agent darf
//! diese Entscheidung nie für den Nutzer treffen. Der Test
//! `gradium_ist_nur_ueber_die_befehle_der_oberflaeche_erreichbar` hält das fest.
//!
//! Der API-Schlüssel geht ausschließlich in den Geheimnisspeicher (DPAPI) und nie durch die
//! Einstellungen; die Oberfläche bekommt von ihm nur [`KeyStatus`] zurück.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};

use crate::managers::calendar::secret;
use crate::managers::tts::gradium::{self, Phase};
use crate::managers::tts::models::TtsModelManager;
use crate::managers::tts::phonon_clip::MIN_CLIP_SECONDS;
use crate::managers::tts::phonon_clone::{
    self, CloneEnv, CloneRequest, JobSlot, KeyStatus, PhononVoiceError, PhononVoiceErrorKind,
    Recording, PENDING_RECORDING,
};
use crate::managers::tts::phonon_models;
use crate::managers::tts::phonon_voices::{ModelTarget, OwnVoice, VoiceStore};
use crate::managers::tts::providers::samples;
use crate::managers::tts::TtsManager;

/// Bindung der Mikrofonaufnahme im `AudioRecordingManager`.
const RECORD_BINDING: &str = "phonon_voice";
/// Längere Aufnahmen werden auf so viele Sekunden gekürzt (Speicher).
const MAX_RECORDING_SECS: usize = 60;
/// Sample-Rate der Aufnahmen des `AudioRecordingManager`.
const RECORDING_RATE: u32 = 16_000;
/// Ereignis mit der Stufe des laufenden Auftrags (`uploading`, `waiting`, `downloading`).
pub const PHASE_EVENT: &str = "phonon-voice-clone-phase";

/// Der eine Auftrag, der gleichzeitig laufen darf.
static CLONE_JOB: JobSlot = JobSlot::new();

/// Eigene Stimmen und das, was die Oberfläche zum Anzeigen braucht.
#[derive(Debug, Clone, Serialize, Type)]
pub struct OwnVoicesOverview {
    pub voices: Vec<OwnVoice>,
    /// Kennungen der installierten Phonon-Modelle (dorthin wird kopiert).
    pub models: Vec<String>,
    /// Sprachen, die Gradium beim Erstellen annimmt.
    pub languages: Vec<String>,
    /// Es läuft gerade eine Erstellung.
    pub busy: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RecordedInfo {
    pub seconds: f32,
}

struct Ctx {
    root: PathBuf,
    targets: Vec<ModelTarget>,
    models: Arc<TtsModelManager>,
}

impl Ctx {
    fn new(app: &AppHandle) -> Self {
        let models = app.state::<Arc<TtsModelManager>>().inner().clone();
        let dirs = models.phonon_dirs();
        Self {
            root: dirs.root().to_path_buf(),
            targets: phonon_models::installed_model_dirs(dirs)
                .into_iter()
                .map(|(id, dir)| ModelTarget { id, dir })
                .collect(),
            models,
        }
    }

    fn store(&self) -> VoiceStore<'_> {
        VoiceStore {
            root: &self.root,
            targets: &self.targets,
            lock: self.models.phonon_voices_lock(),
        }
    }
}

fn secrets_dir(app: &AppHandle) -> Result<PathBuf, PhononVoiceError> {
    secret::secrets_dir_for(app).map_err(|_| {
        PhononVoiceError::new(
            PhononVoiceErrorKind::KeyStore,
            "Der Ordner für Geheimnisse ist nicht erreichbar.",
        )
    })
}

/// Blockierende Arbeit (Dateien) außerhalb des Oberflächen-Threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, PhononVoiceError> + Send + 'static,
) -> Result<T, PhononVoiceError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| {
            PhononVoiceError::new(PhononVoiceErrorKind::Io, "Der Vorgang wurde unterbrochen.")
        })?
}

// ---- Stimmen ---------------------------------------------------------------------------------------------

/// Eigene Stimmen anzeigen. Gleicht vorher den eigenen Speicher mit den installierten Modellen ab
/// (fehlende Dateien und Zeilen werden ergänzt, z. B. nach einem neu installierten Modell).
#[tauri::command]
#[specta::specta]
pub async fn phonon_own_voices_list(app: AppHandle) -> Result<OwnVoicesOverview, PhononVoiceError> {
    blocking(move || {
        let ctx = Ctx::new(&app);
        let store = ctx.store();
        let report = store.sync();
        if !report.conflicts.is_empty() {
            log::warn!(
                "eigene Phonon-Stimmen: Namenskonflikte mit Modellstimmen: {:?}",
                report.conflicts
            );
        }
        Ok(OwnVoicesOverview {
            voices: store.list(),
            models: ctx.targets.iter().map(|t| t.id.clone()).collect(),
            languages: gradium::LANGUAGES.iter().map(|l| l.to_string()).collect(),
            busy: CLONE_JOB.is_running(),
        })
    })
    .await
}

/// Eine fertige Stimmdatei (`.safetensors`) importieren: prüfen, kopieren, eintragen. Kein
/// Netzzugriff.
#[tauri::command]
#[specta::specta]
pub async fn phonon_own_voice_import(
    app: AppHandle,
    path: String,
    name: String,
    overwrite: bool,
) -> Result<OwnVoice, PhononVoiceError> {
    let handle = app.clone();
    let voice = blocking(move || {
        let ctx = Ctx::new(&app);
        Ok(ctx
            .store()
            .import_file(std::path::Path::new(&path), &name, overwrite)?)
    })
    .await?;
    samples::refresh_in_background(&handle, "phonon");
    Ok(voice)
}

/// Eine eigene Stimme entfernen (nur eigene; mitgelieferte Stimmen nie).
#[tauri::command]
#[specta::specta]
pub async fn phonon_own_voice_remove(app: AppHandle, id: String) -> Result<(), PhononVoiceError> {
    blocking(move || {
        let ctx = Ctx::new(&app);
        Ok(ctx.store().remove(&id)?)
    })
    .await
}

// ---- Erstellen über Gradium -------------------------------------------------------------------------------------

/// Eine Stimme über Gradium erstellen. **Netzzugriff** (nur dieser Befehl): sendet ein Stück von
/// etwa zehn Sekunden Sprache an api.gradium.ai. Ohne `consent` geschieht nichts.
#[tauri::command]
#[specta::specta]
pub async fn phonon_voice_clone(
    app: AppHandle,
    request: CloneRequest,
) -> Result<OwnVoice, PhononVoiceError> {
    // Ohne Einwilligung nicht einmal Ordner und Schlüsselspeicher anfassen.
    if !request.consent {
        return Err(PhononVoiceError::new(
            PhononVoiceErrorKind::ConsentRequired,
            "Ohne die Einwilligung der sprechenden Person wird nichts gesendet.",
        ));
    }
    let ctx = Ctx::new(&app);
    let secrets = secrets_dir(&app)?;
    let fish_dir = app.state::<Arc<TtsManager>>().fish_dir_public();
    let key = move || phonon_clone::load_key(&secrets);
    let env = CloneEnv {
        store: ctx.store(),
        fish_dir: &fish_dir,
        gradium: gradium::Config::production(),
        key: &key,
        recording: &PENDING_RECORDING,
    };
    let emitter = app.clone();
    let voice = phonon_clone::create_voice(&env, &CLONE_JOB, request, &move |phase: Phase| {
        let _ = emitter.emit(PHASE_EVENT, phase);
    })
    .await?;
    samples::refresh_in_background(&app, "phonon");
    Ok(voice)
}

/// Die laufende Erstellung abbrechen (Anfragen enden binnen etwa 50 ms; nichts wird gespeichert).
#[tauri::command]
#[specta::specta]
pub fn phonon_voice_clone_cancel() {
    CLONE_JOB.cancel();
}

// ---- Schlüssel ------------------------------------------------------------------------------------------------------

/// Ist ein Gradium-Schlüssel gespeichert (nie der Schlüssel selbst)?
#[tauri::command]
#[specta::specta]
pub async fn phonon_gradium_key_status(app: AppHandle) -> Result<KeyStatus, PhononVoiceError> {
    let dir = secrets_dir(&app)?;
    blocking(move || Ok(phonon_clone::key_status(&dir))).await
}

/// Schlüssel eintragen oder ändern: verschlüsselt im Geheimnisspeicher, nie in den Einstellungen.
/// Kein Netzzugriff (der Schlüssel wird nicht „zur Probe“ gesendet).
#[tauri::command]
#[specta::specta]
pub async fn phonon_gradium_key_set(
    app: AppHandle,
    key: String,
) -> Result<KeyStatus, PhononVoiceError> {
    let dir = secrets_dir(&app)?;
    blocking(move || {
        phonon_clone::save_key(&dir, &key)?;
        Ok(phonon_clone::key_status(&dir))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn phonon_gradium_key_delete(app: AppHandle) -> Result<KeyStatus, PhononVoiceError> {
    let dir = secrets_dir(&app)?;
    blocking(move || {
        phonon_clone::delete_key(&dir);
        Ok(phonon_clone::key_status(&dir))
    })
    .await
}

// ---- Mikrofon ------------------------------------------------------------------------------------------------------------

/// Aufnahme für „Stimme erstellen“ starten (mit dem Mikrofon der App; VAD aus).
#[tauri::command]
#[specta::specta]
pub fn phonon_voice_record_start(app: AppHandle) -> Result<(), PhononVoiceError> {
    *PENDING_RECORDING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let rm = app.state::<Arc<crate::managers::audio::AudioRecordingManager>>();
    rm.try_start_recording(RECORD_BINDING, crate::audio_toolkit::VadPolicy::Disabled)
        .map_err(|e| {
            PhononVoiceError::new(
                PhononVoiceErrorKind::BadSource,
                format!("Die Aufnahme lässt sich nicht starten: {e}"),
            )
        })
}

/// Aufnahme beenden und für die Erstellung bereithalten.
#[tauri::command]
#[specta::specta]
pub fn phonon_voice_record_stop(app: AppHandle) -> Result<RecordedInfo, PhononVoiceError> {
    let rm = app.state::<Arc<crate::managers::audio::AudioRecordingManager>>();
    let generation = rm.cancel_generation();
    let mut samples = rm
        .stop_recording(RECORD_BINDING, generation)
        .ok_or_else(|| {
            PhononVoiceError::new(PhononVoiceErrorKind::BadSource, "Es läuft keine Aufnahme.")
        })?;
    samples.truncate(MAX_RECORDING_SECS * RECORDING_RATE as usize);
    let seconds = samples.len() as f32 / RECORDING_RATE as f32;
    if seconds < MIN_CLIP_SECONDS {
        return Err(PhononVoiceError::new(
            PhononVoiceErrorKind::BadSource,
            format!(
                "Die Aufnahme ist zu kurz ({seconds:.1} s). Bitte mindestens {MIN_CLIP_SECONDS:.0} Sekunden, besser zehn, sprechen."
            ),
        ));
    }
    *PENDING_RECORDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Recording {
        samples,
        rate: RECORDING_RATE,
    });
    Ok(RecordedInfo { seconds })
}

/// Eine laufende oder wartende Aufnahme verwerfen (Dialog geschlossen).
#[tauri::command]
#[specta::specta]
pub fn phonon_voice_record_discard(app: AppHandle) {
    let rm = app.state::<Arc<crate::managers::audio::AudioRecordingManager>>();
    let generation = rm.cancel_generation();
    // Nur eine Aufnahme dieser Bindung wird angehalten (andere bleiben unberührt).
    let _ = rm.stop_recording(RECORD_BINDING, generation);
    *PENDING_RECORDING.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
