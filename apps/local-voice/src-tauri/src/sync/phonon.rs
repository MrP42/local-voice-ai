//! S1 nutzt ausschließlich die bestehenden Ablage-Sammlungen und deren verschlüsselte Blobs.
use super::ablage::Konto;
use crate::ablage::speicher::Ablage;
use crate::managers::tts::phonon_voices::{VoiceError, VoiceStore};

pub(crate) fn abgleichen(a: &Ablage, store: &VoiceStore<'_>, konto: &Konto<'_>) -> Result<(), VoiceError> {
    store.sync_ablage(a, konto)
}

/// Nur im autorisierten Sync-Lauf aufrufen: die Konto-Sperre hält Abmeldung und Kontowechsel fern.
pub(crate) fn abgleichen_app(app: &tauri::AppHandle, a: &Ablage, konto: &Konto<'_>) {
    use crate::managers::tts::{models::TtsModelManager, phonon_models};
    use crate::managers::tts::phonon_voices::ModelTarget;
    use std::sync::Arc;
    use tauri::Manager;
    let models = app.state::<Arc<TtsModelManager>>();
    let dirs = models.phonon_dirs();
    let targets = phonon_models::installed_model_dirs(dirs).into_iter()
        .map(|(id, dir)| ModelTarget { id, dir }).collect::<Vec<_>>();
    let store = VoiceStore { root: dirs.root(), targets: &targets, lock: models.phonon_voices_lock() };
    if abgleichen(a, &store, konto).is_err() {
        log::warn!("phonon-sync: lokaler Abgleich nicht abgeschlossen");
    }
}
