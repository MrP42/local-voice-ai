//! `--provision`: Modelle fuer das portable Offline-Paket vorinstallieren.
//!
//! Der Bau des Offline-Pakets (`scripts/build-portable-zip.ps1 -Offline`) legt
//! neben die EXE die Marke `portable` und ruft `local-voice-ai.exe --provision
//! <ids>` auf. Jede Kennung laeuft ueber denselben Manager wie ein Klick auf der
//! Modellseite (Download, Pruefsumme, Marke), nur ohne Oberflaeche. So ist das
//! Ergebnis unter `Data\` genau das, was die App selbst als installiert erkennt;
//! eine zweite, nachgebaute Ablage-Logik im Bauskript gibt es nicht.
//!
//! Zuordnung der Kennung zum Manager:
//! * `llm-…`, `emb-…` → [`LlmRuntimeManager`] (Laufzeit und Sprachmodelle),
//! * Phonon/Piper (`phonon-runtime`, `phonon-40m-de`, Stimmen, `piper-runtime`)
//!   → [`TtsModelManager`],
//! * alles andere → [`ModelManager`] (Spracherkennung, Kennung wie `--list-models`).

use std::sync::Arc;

use tauri::AppHandle;

use crate::managers::llm::LlmRuntimeManager;
use crate::managers::model::ModelManager;
use crate::managers::tts::models::{TtsModelManager, RUNTIME_ID as PIPER_RUNTIME_ID};
use crate::managers::tts::phonon_models;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Llm,
    Tts,
    Asr,
}

fn kind_of(id: &str) -> Kind {
    if id.starts_with("llm-") || id.starts_with("emb-") {
        Kind::Llm
    } else if id == PIPER_RUNTIME_ID || phonon_models::is_phonon_id(id) {
        Kind::Tts
    } else {
        Kind::Asr
    }
}

/// Exit-Code wie in `cli.rs` beschrieben.
pub fn run(app: &AppHandle, ids: &[String]) -> i32 {
    crate::selftest::begin_headless_run();
    let ids: Vec<&str> = ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    match crate::portable::data_dir() {
        Some(dir) => eprintln!("[provision] Ziel (portabel): {}", dir.display()),
        None => eprintln!("[provision] Ziel: Datenverzeichnis des Benutzers (nicht portabel)"),
    }

    let mut llm: Option<LlmRuntimeManager> = None;
    let mut tts: Option<TtsModelManager> = None;
    let mut asr: Option<Arc<ModelManager>> = None;
    let mut failed = 0usize;
    let mut selected_asr: Option<String> = None;

    for &id in &ids {
        let kind = kind_of(id);
        eprintln!("[provision] {id} ({kind:?}) …");
        let result: Result<bool, String> = match kind {
            Kind::Llm => {
                if llm.is_none() {
                    match LlmRuntimeManager::new(app) {
                        Ok(m) => llm = Some(m),
                        Err(e) => return fail_init("Sprachmodell-Laufzeit", &e.to_string()),
                    }
                }
                let m = llm.as_ref().expect("gesetzt");
                if !m.list_downloads().iter().any(|d| d.id == id) {
                    return unknown(id);
                }
                tauri::async_runtime::block_on(m.download(id)).map(|()| {
                    m.list_downloads()
                        .iter()
                        .any(|d| d.id == id && d.is_downloaded)
                })
            }
            Kind::Tts => {
                if tts.is_none() {
                    match TtsModelManager::new(app) {
                        Ok(m) => tts = Some(m),
                        Err(e) => return fail_init("Sprachausgabe", &e.to_string()),
                    }
                }
                let m = tts.as_ref().expect("gesetzt");
                tauri::async_runtime::block_on(m.download(id)).map(|()| {
                    m.list_downloads()
                        .iter()
                        .any(|d| d.id == id && d.is_downloaded)
                })
            }
            Kind::Asr => {
                if asr.is_none() {
                    match ModelManager::new(app) {
                        Ok(m) => asr = Some(Arc::new(m)),
                        Err(e) => return fail_init("Spracherkennung", &e.to_string()),
                    }
                }
                let m = asr.as_ref().expect("gesetzt");
                let Some(full) = resolve_asr(m, id) else {
                    return unknown(id);
                };
                if full != id {
                    eprintln!("[provision] {id} -> {full}");
                }
                selected_asr.get_or_insert_with(|| full.clone());
                tauri::async_runtime::block_on(m.download_model(&full))
                    .map_err(|e| e.to_string())
                    .map(|()| {
                        let _ = m.rescan_local_models();
                        m.get_model_info(&full).is_some_and(|i| i.is_downloaded)
                    })
            }
        };
        match result {
            Ok(true) => eprintln!("[provision] {id}: installiert"),
            Ok(false) => {
                failed += 1;
                eprintln!(
                    "[provision] {id}: Download gemeldet, aber nicht als installiert erkannt"
                );
            }
            Err(e) => {
                failed += 1;
                eprintln!("[provision] {id}: FEHLER {e}");
            }
        }
    }
    if failed > 0 {
        return 1;
    }
    preselect(app, selected_asr.as_deref(), &ids);
    0
}

/// Kennung wie in `--list-models`; ein eindeutiger Teil davon reicht
/// (`nemotron-3.5-asr-streaming-0.6b-Q8_0`). Mehrdeutig oder unbekannt: `None`
/// und die Kandidaten im Log.
fn resolve_asr(m: &ModelManager, id: &str) -> Option<String> {
    if m.get_model_info(id).is_some() {
        return Some(id.to_string());
    }
    let hits: Vec<String> = m
        .get_available_models()
        .into_iter()
        .map(|i| i.id)
        .filter(|full| full.contains(id))
        .collect();
    match hits.as_slice() {
        [one] => Some(one.clone()),
        [] => None,
        many => {
            eprintln!("[provision] {id} ist mehrdeutig: {}", many.join(", "));
            None
        }
    }
}

/// Die vorinstallierten Modelle gleich einstellen: das erste Spracherkennungs-
/// modell fuer Diktat und Besprechung, das erste lokale Sprachmodell fuer
/// Zusammenfassung und Protokoll. Phonon wird bewusst NICHT eingeschaltet:
/// Opt-in und Lizenzhinweis (CC BY-NC beim deutschen Modell) bestaetigt die
/// Person selbst in der App.
fn preselect(app: &AppHandle, asr: Option<&str>, ids: &[&str]) {
    let llm = ids
        .iter()
        .copied()
        .find(|id| id.starts_with("llm-") && !id.starts_with("llm-runtime-"));
    if asr.is_none() && llm.is_none() {
        return;
    }
    let mut settings = crate::settings::get_settings(app);
    if let Some(id) = asr {
        settings.selected_model = id.to_string();
    }
    if let Some(id) = llm {
        let local = crate::managers::llm::LOCAL_PROVIDER_ID;
        settings.post_process_provider_id = local.to_string();
        settings
            .post_process_models
            .insert(local.to_string(), id.to_string());
        settings.llm_active_model_id = Some(format!("{local}:{id}"));
    }
    crate::settings::write_settings(app, settings);
    eprintln!("[provision] voreingestellt: Spracherkennung {asr:?}, Sprachmodell {llm:?}");
}

fn fail_init(what: &str, e: &str) -> i32 {
    eprintln!("[provision] {what} nicht verfuegbar: {e}");
    1
}

fn unknown(id: &str) -> i32 {
    eprintln!("[provision] unbekannte Kennung: {id} (siehe --list-models bzw. Katalog)");
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_go_to_the_right_manager() {
        assert_eq!(kind_of("llm-qwen3.5-4b-q4"), Kind::Llm);
        assert_eq!(kind_of("llm-runtime-windows-x64-vulkan"), Kind::Llm);
        assert_eq!(kind_of("emb-bge-m3-q8"), Kind::Llm);
        assert_eq!(kind_of("phonon-runtime"), Kind::Tts);
        assert_eq!(kind_of("phonon-40m-de"), Kind::Tts);
        assert_eq!(kind_of("phonon-31m-en"), Kind::Tts);
        assert_eq!(kind_of("piper-runtime"), Kind::Tts);
        assert_eq!(
            kind_of("handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf"),
            Kind::Asr
        );
    }
}
