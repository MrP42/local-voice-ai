//! Tauri-Commands des Vorlesen-Bereichs (TP1).

use crate::managers::tts::builder;
use crate::managers::tts::models::{TtsDownloadInfo, TtsModelManager};
use crate::managers::tts::registry::{ReferenceAnalysis, VoiceInfo, VoiceMeta};
use crate::managers::tts::{ReadingInfo, TtsManager, TtsStatus};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[tauri::command]
#[specta::specta]
pub async fn tts_speak_text(app: AppHandle, text: String) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    // speak_text sichert selbst: Cache-Offline-Pfad oder Serverstart.
    tts.speak_text(&text).await.map(|_| ())
}

#[tauri::command]
#[specta::specta]
pub async fn tts_speak_clipboard(app: AppHandle) -> Result<(), String> {
    let text = app
        .clipboard()
        .read_text()
        .map_err(|e| format!("clipboard read failed: {e}"))?;
    tts_speak_text(app, text).await
}

#[tauri::command]
#[specta::specta]
pub fn tts_cancel(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().cancel();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn tts_server_start(app: AppHandle) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.refresh_from_settings();
    tts.ensure_server().await
}

#[tauri::command]
#[specta::specta]
pub async fn tts_server_stop(app: AppHandle) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.stop_server_any().await
}

#[tauri::command]
#[specta::specta]
pub fn tts_server_status(app: AppHandle) -> Result<TtsStatus, String> {
    Ok(app.state::<Arc<TtsManager>>().status())
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ImportedVoice {
    pub id: String,
    pub transcript: String,
}

#[tauri::command]
#[specta::specta]
pub fn tts_list_voices(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(app.state::<Arc<TtsManager>>().list_voice_ids())
}

/// Hoerprobe einer Stimme: derselbe Demotext, mit dieser Stimme erzeugt.
#[derive(serde::Serialize, serde::Deserialize, specta::Type)]
pub struct VoiceSample {
    /// Absoluter Pfad zur WAV — die Oberflaeche spielt sie ueber das
    /// asset-Protokoll ab, ohne sie zu kopieren.
    pub wav_path: String,
    /// Der gesprochene Satz. Fuer alle Stimmen derselbe, sonst vergleicht man
    /// Aufnahmen statt Stimmen.
    pub transcript: String,
}

/// Erzeugt die Hoerprobe beim ersten Aufruf (und erneut, wenn die Stimme
/// neu aufgenommen wurde); danach kommt sie aus dem Cache. Braucht den
/// Fish-Speech-Server, der bei Bedarf gestartet wird — der erste Aufruf kann
/// deshalb dauern.
#[tauri::command]
#[specta::specta]
pub async fn tts_voice_demo(app: AppHandle, voice_id: String) -> Result<VoiceSample, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    let wav = tts.synthesize_voice_demo(&voice_id).await?;
    Ok(VoiceSample {
        wav_path: wav.to_string_lossy().into_owned(),
        transcript: TtsManager::DEMO_TEXT.to_string(),
    })
}

#[tauri::command]
#[specta::specta]
pub fn tts_record_reference_start(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().record_reference_start()
}

#[tauri::command]
#[specta::specta]
pub fn tts_record_reference_stop(app: AppHandle) -> Result<String, String> {
    app.state::<Arc<TtsManager>>().record_reference_stop()
}

#[tauri::command]
#[specta::specta]
pub fn tts_save_voice(app: AppHandle, name: String, transcript: String) -> Result<String, String> {
    app.state::<Arc<TtsManager>>()
        .save_pending_voice(&name, &transcript)
}

#[tauri::command]
#[specta::specta]
pub fn tts_import_voice(
    app: AppHandle,
    name: String,
    wav_path: String,
    transcript: Option<String>,
) -> Result<ImportedVoice, String> {
    let (id, transcript) = app
        .state::<Arc<TtsManager>>()
        .import_voice_file(&name, &wav_path, transcript)?;
    Ok(ImportedVoice { id, transcript })
}

#[tauri::command]
#[specta::specta]
pub fn tts_delete_voice(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().delete_voice_id(&id)
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct TranslateOutcome {
    pub transcript: String,
    pub translation: String,
}

#[tauri::command]
#[specta::specta]
pub async fn tts_translate_speak(
    app: AppHandle,
    text: String,
    target_lang: String,
) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.translate_and_speak(&text, &target_lang).await
}

/// Text uebersetzen, ohne ihn abzuspielen. Zwischengespeichert je Text und
/// Sprache; laeuft der Fish-Server, rechnet die Uebersetzung auf der CPU.
#[tauri::command]
#[specta::specta]
pub async fn tts_translate(
    app: AppHandle,
    text: String,
    target_lang: String,
) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.translate_text(&text, &target_lang).await
}

/// Liegt fuer diesen Text in dieser Sprache schon eine Uebersetzung bereit?
/// Damit die Oberflaeche beim Umschalten zwischen Sprachen sofort zeigen
/// kann, was da ist, ohne eine Uebersetzung anzustossen.
#[tauri::command]
#[specta::specta]
pub fn tts_cached_translation(app: AppHandle, text: String, target_lang: String) -> Option<String> {
    app.state::<Arc<TtsManager>>()
        .cached_translation(text.trim(), &target_lang)
}

/// Welche Modelle das lokale Ollama gerade geladen hat.
///
/// Grundlage der Sprachmodell-Anzeige neben dem Serversymbol. Kein lokales
/// Ollama konfiguriert oder nicht erreichbar → leere Liste: fuer die Anzeige
/// ist "nichts geladen" und "nichts da" derselbe graue Zustand.
#[tauri::command]
#[specta::specta]
pub async fn llm_ps(app: AppHandle) -> Vec<String> {
    let settings = crate::settings::get_settings(&app);
    let Some(provider) = settings.active_post_process_provider() else {
        return Vec::new();
    };
    let Some(chat) = crate::llm_client::ollama_native_url(&provider.base_url) else {
        return Vec::new();
    };
    let url = chat.replace("/api/chat", "/api/ps");
    let Ok(resp) = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    else {
        return Vec::new();
    };
    let Ok(value) = resp.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    value
        .get("models")
        .and_then(|m| m.as_array())
        .map(|models| {
            models
                .iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                .map(|n| n.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Alle geladenen Ollama-Modelle sofort entladen. Rueckgabe: wie viele.
#[tauri::command]
#[specta::specta]
pub async fn llm_unload(app: AppHandle) -> Result<u32, String> {
    let settings = crate::settings::get_settings(&app);
    let Some(provider) = settings.active_post_process_provider().cloned() else {
        return Ok(0);
    };
    let loaded = llm_ps(app).await;
    let count = loaded.len() as u32;
    for model in loaded {
        crate::llm_client::ollama_unload(&provider.base_url, &model).await;
    }
    Ok(count)
}

/// Das konfigurierte Modell vorwaermen: laden, ohne etwas zu erzeugen.
///
/// Fuer den Fall "gleich uebersetze ich mehrmals": das Laden passiert jetzt,
/// nicht mitten im ersten Auftrag. keep_alive bewusst begrenzt — wer es
/// laenger halten will, waermt erneut oder laesst die Uebersetzung selbst
/// laden.
#[tauri::command]
#[specta::specta]
pub async fn llm_warm(app: AppHandle) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or("Kein Post-Processing-Provider konfiguriert")?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err("Kein Modell eingetragen (Einstellungen → Nachbearbeitung)".to_string());
    }
    let chat = crate::llm_client::ollama_native_url(&provider.base_url)
        .ok_or("Vorwaermen geht nur mit einem lokalen Ollama")?;
    let url = chat.replace("/api/chat", "/api/generate");
    let body = serde_json::json!({ "model": model, "keep_alive": "10m" });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| format!("Ollama nicht erreichbar: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ollama antwortete {}", resp.status()));
    }
    Ok(model)
}

/// Den aktuellen Stimm-Seed unter einem Namen als Stimme sichern.
/// Rueckgabe: die bereinigte Stimmen-Kennung.
#[tauri::command]
#[specta::specta]
pub async fn tts_save_seed_voice(app: AppHandle, name: String) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.save_seed_voice(&name).await
}

/// Diktat fuer das Vorlesefeld: Aufnahme starten.
#[tauri::command]
#[specta::specta]
pub fn tts_dictate_start(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().record_dictate_start()
}

/// Diktat beenden und den erkannten Text zurueckgeben.
#[tauri::command]
#[specta::specta]
pub async fn tts_dictate_stop(app: AppHandle) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.record_dictate_stop().await
}

#[tauri::command]
#[specta::specta]
pub fn tts_record_translate_start(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().record_translate_start()
}

#[tauri::command]
#[specta::specta]
pub async fn tts_record_translate_stop(
    app: AppHandle,
    target_lang: String,
) -> Result<TranslateOutcome, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    let (transcript, translation) = tts.record_translate_stop(&target_lang).await?;
    Ok(TranslateOutcome {
        transcript,
        translation,
    })
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_open(app: AppHandle, path: String) -> Result<ReadingInfo, String> {
    app.state::<Arc<TtsManager>>().reading_open(&path)
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_play(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>()
        .inner()
        .clone()
        .reading_play()
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_pause(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().reading_pause();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_list(app: AppHandle) -> Result<Vec<ReadingInfo>, String> {
    Ok(app.state::<Arc<TtsManager>>().reading_list())
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_seek(app: AppHandle, delta: i32) -> Result<ReadingInfo, String> {
    app.state::<Arc<TtsManager>>()
        .inner()
        .clone()
        .reading_seek(delta)
}

#[tauri::command]
#[specta::specta]
pub async fn tts_speak_resume(app: AppHandle) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.speak_resume().await.map(|_| ())
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_reset(app: AppHandle, key: String) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().reading_reset(&key)
}

#[tauri::command]
#[specta::specta]
pub fn tts_reading_remove(app: AppHandle, key: String) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().reading_remove(&key)
}

#[tauri::command]
#[specta::specta]
pub fn tts_export_format(app: AppHandle) -> Result<String, String> {
    Ok(app.state::<Arc<TtsManager>>().export_format())
}

#[tauri::command]
#[specta::specta]
pub async fn tts_summarize_text(
    app: AppHandle,
    text: String,
    options: crate::summarizer::SummaryOptions,
) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    crate::summarizer::summarize(&settings, &text, &options).await
}

#[tauri::command]
#[specta::specta]
pub fn tts_extract_document(path: String) -> Result<String, String> {
    crate::media::extract_document_text(std::path::Path::new(&path))
}

#[tauri::command]
#[specta::specta]
pub async fn tts_extract_url(url: String) -> Result<String, String> {
    crate::media::extract_url_text(&url).await
}

/// Alles beenden, was auf dem TTS-Port lauscht — ohne Gesundheitsprüfung.
/// Der Ausweg, wenn ein hängender Server die Grafikkarte festhält.
#[tauri::command]
#[specta::specta]
pub fn tts_server_kill(app: AppHandle) -> Result<String, String> {
    app.state::<Arc<TtsManager>>().kill_server_hard()
}

#[tauri::command]
#[specta::specta]
pub fn tts_voicechange_record_start(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().record_voicechange_start()
}

#[tauri::command]
#[specta::specta]
pub async fn tts_voicechange_record_stop(app: AppHandle) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.record_voicechange_stop().await
}

#[tauri::command]
#[specta::specta]
pub async fn tts_voicechange_file(app: AppHandle, wav_path: String) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.respeak_file(&wav_path).await
}

/// Den Vorlesetext samt Sprecherwechseln in eine WAV-Datei schreiben.
///
/// Kehrt SOFORT zurueck; der Lauf arbeitet im Hintergrund weiter und meldet
/// sich ueber `tts-export-progress`. Ein langer Text braucht Minuten — die
/// Oberflaeche darf solange nicht blockiert sein, und der Fortschritt gehoert
/// sichtbar auf den Schirm statt in eine wartende Zusage.
#[tauri::command]
#[specta::specta]
pub fn tts_speak_to_file(app: AppHandle, text: String, out_path: String) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = tts.speak_to_file(&text, &out_path).await {
            log::warn!("tts export failed: {e}");
            let _ = tauri::Emitter::emit(
                &tts.app_handle(),
                "tts-export-error",
                serde_json::json!({ "message": e }),
            );
        }
    });
    Ok(())
}

/// Laufenden Datei-Export abbrechen.
#[tauri::command]
#[specta::specta]
pub fn tts_export_cancel(app: AppHandle) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().cancel_export();
    Ok(())
}

/// Freitext-Vorlesen an einer bestimmten Satzposition fortsetzen — die Basis
/// fuer "vorheriger/naechster Satz" in der Transportzeile.
#[tauri::command]
#[specta::specta]
pub async fn tts_speak_seek(app: AppHandle, delta: i32) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.speak_seek(delta).await.map(|_| ())
}

#[tauri::command]
#[specta::specta]
pub async fn tts_synthesize_to_file(
    app: AppHandle,
    text: String,
    out_path: String,
) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.synthesize_to_file(&text, &out_path).await.map(|_| ())
}

// ── Piper-Katalog: Laufzeit + Stimmen (Paket B-E3) ──────────────────────────
//
// Fortschritt laeuft ueber die BESTEHENDEN Download-Events der ASR-Modelle
// (`model-download-progress`/`model-verification-*`/`model-download-complete`/
// `model-deleted`/`model-download-failed`) — siehe `TtsModelManager::run_download`.
// Eigene Events waeren hier reine Verdopplung gewesen.

#[tauri::command]
#[specta::specta]
pub fn tts_list_downloads(app: AppHandle) -> Result<Vec<TtsDownloadInfo>, String> {
    Ok(app.state::<Arc<TtsModelManager>>().list_downloads())
}

#[tauri::command]
#[specta::specta]
pub async fn tts_download_model(app: AppHandle, id: String) -> Result<(), String> {
    let manager = app.state::<Arc<TtsModelManager>>().inner().clone();
    let result = manager.download(&id).await;
    if let Err(ref error) = result {
        log::error!("Piper download failed for {}: {}", id, error);
        let _ = app.emit(
            "model-download-failed",
            serde_json::json!({ "model_id": &id, "error": error }),
        );
    }
    result
}

#[tauri::command]
#[specta::specta]
pub fn tts_cancel_download(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Arc<TtsModelManager>>().cancel(&id);
    Ok(())
}

// ---- Sprecher-Registry (Paket B-S1) ---------------------------------------

#[tauri::command]
#[specta::specta]
pub fn tts_list_voice_infos(app: AppHandle) -> Vec<VoiceInfo> {
    app.state::<Arc<TtsManager>>().list_voice_infos()
}

#[tauri::command]
#[specta::specta]
pub fn tts_get_voice_meta(app: AppHandle, id: String) -> Result<VoiceMeta, String> {
    app.state::<Arc<TtsManager>>().get_voice_meta(&id)
}

#[tauri::command]
#[specta::specta]
pub fn tts_set_voice_meta(app: AppHandle, id: String, meta: VoiceMeta) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().set_voice_meta(&id, meta)
}

/// Avatar-Bytes kommen roh als `Vec<u8>`, nicht als Base64-String: das
/// Projekt hat kein direktes base64-Crate, und eines nur fuer den
/// Avatar-Upload wollte der Auftrag ausdruecklich vermeiden (siehe
/// `voices::save_avatar`).
#[tauri::command]
#[specta::specta]
pub fn tts_set_voice_avatar(
    app: AppHandle,
    id: String,
    bytes: Vec<u8>,
    ext: String,
) -> Result<(), String> {
    app.state::<Arc<TtsManager>>()
        .set_voice_avatar(&id, bytes, &ext)
}

#[tauri::command]
#[specta::specta]
pub fn tts_delete_model(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Arc<TtsModelManager>>().delete(&id)
}

#[tauri::command]
#[specta::specta]
pub fn tts_clear_voice_avatar(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Arc<TtsManager>>().clear_voice_avatar(&id)
}

#[tauri::command]
#[specta::specta]
pub fn tts_save_style_reference(
    app: AppHandle,
    voice: String,
    style_id: String,
    name: String,
) -> Result<(), String> {
    app.state::<Arc<TtsManager>>()
        .save_style_reference(&voice, &style_id, &name)
}

#[tauri::command]
#[specta::specta]
pub fn tts_delete_style(app: AppHandle, voice: String, style_id: String) -> Result<(), String> {
    app.state::<Arc<TtsManager>>()
        .delete_style(&voice, &style_id)
}

#[tauri::command]
#[specta::specta]
pub fn tts_analyze_reference(app: AppHandle, voice: String) -> Result<ReferenceAnalysis, String> {
    app.state::<Arc<TtsManager>>().analyze_reference(&voice)
}

#[tauri::command]
#[specta::specta]
pub fn tts_analyze_pending_reference(app: AppHandle) -> Result<ReferenceAnalysis, String> {
    app.state::<Arc<TtsManager>>().analyze_pending_reference()
}

#[tauri::command]
#[specta::specta]
pub async fn tts_seed_preview(app: AppHandle, seed: i64) -> Result<Vec<u8>, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.seed_preview(seed).await
}

#[tauri::command]
#[specta::specta]
pub async fn tts_save_seed_voice_v2(
    app: AppHandle,
    seed: i64,
    display_name: String,
    meta: VoiceMeta,
) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.save_seed_voice_v2(seed, &display_name, meta).await
}

/// Zustand des laufenden Auto-Tagging-Laufs: der watch-Sender, über den
/// `tts_auto_tag_cancel` den Lauf abbricht. `None` = kein Lauf aktiv.
/// In `lib.rs` per `.manage()` registriert.
#[derive(Default)]
pub struct AutoTagRun(pub std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>);

/// T4 Auto-Tagging: schlägt Emotions-/Vortrags-Tags fürs `text` vor. Der
/// LLM-Output erreicht die Oberfläche NIE unvalidiert — `crate::tagging`
/// prüft ihn hart gegen die Nur-Einfüge-Invariante (höchstens ein Retry,
/// danach ein verständlicher Fehler statt eines möglicherweise veränderten
/// Texts). `provider_override`: `None` = aktiver Post-Processing-Provider,
/// `Some("anthropic")` = fest Claude (Modell aus `tts_tag_model`).
///
/// Der Text wird ABSCHNITTSWEISE getaggt; je fertigem Abschnitt geht ein
/// `tts-autotag-progress`-Event `{done, total, insertions}` an die UI —
/// Tags erscheinen damit fortlaufend statt alle am Ende. Abbrechen:
/// `tts_auto_tag_cancel`; die Rückgabe ist dann das bis dahin Gesammelte.
///
/// Gerätewahl (`tts_tag_device`, nur lokales Ollama): "cpu"/"gpu" fest,
/// "auto" = GPU nur, wenn der TTS-Server sie gerade nicht braucht.
///
/// Rückgabe: `offset_in_original` ist ein BYTE-Offset in `text` (Rust-Art);
/// das Frontend arbeitet mit UTF-16-Offsets und muss `offset_chars`
/// (Unicode-Skalarwert-Zählung) selbst umrechnen — siehe die Dokumentation
/// an `tagging::TagInsertion` und die Umrechnung in `AutoTagBar.tsx`.
#[tauri::command]
#[specta::specta]
pub async fn tts_auto_tag(
    app: AppHandle,
    text: String,
    allowed_tags: Vec<String>,
    provider_override: Option<String>,
) -> Result<Vec<crate::tagging::TagInsertion>, String> {
    let settings = crate::settings::get_settings(&app);
    let cpu_only = match settings.tts_tag_device.as_str() {
        "cpu" => true,
        "gpu" => false,
        _ => app.state::<Arc<TtsManager>>().gpu_busy(),
    };

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    // Ein evtl. noch registrierter älterer Lauf wird abgebrochen — es gibt
    // genau EINEN Auto-Tagging-Lauf zugleich.
    let previous_run = app
        .state::<AutoTagRun>()
        .0
        .lock()
        .unwrap()
        .replace(cancel_tx);
    if let Some(old) = previous_run {
        let _ = old.send(true);
    }

    // Dasselbe llm-activity-Event wie die Übersetzung — treibt dasselbe
    // BrainCircuit-Symbol, ohne dass diese Seite es gesondert verdrahten muss.
    let _ = app.emit("llm-activity", serde_json::json!({ "busy": true }));
    let progress_app = app.clone();
    let outcome = crate::tagging::auto_tag(
        &settings,
        &text,
        &allowed_tags,
        provider_override.as_deref(),
        cpu_only,
        cancel_rx,
        |done, total, insertions| {
            let _ = progress_app.emit(
                "tts-autotag-progress",
                serde_json::json!({
                    "done": done,
                    "total": total,
                    "insertions": insertions,
                }),
            );
        },
    )
    .await;
    app.state::<AutoTagRun>().0.lock().unwrap().take();
    let _ = app.emit(
        "llm-activity",
        serde_json::json!({
            "busy": false,
            "error": outcome.as_ref().err().cloned(),
        }),
    );
    outcome
}

/// Bricht den laufenden Auto-Tagging-Lauf ab (falls einer läuft): die
/// laufende LLM-Anfrage wird gekappt, `tts_auto_tag` kehrt mit den bis
/// dahin gesammelten Vorschlägen zurück und entlädt das Ollama-Modell.
#[tauri::command]
#[specta::specta]
pub fn tts_auto_tag_cancel(app: AppHandle) -> Result<(), String> {
    if let Some(tx) = app.state::<AutoTagRun>().0.lock().unwrap().as_ref() {
        let _ = tx.send(true);
    }
    Ok(())
}

// ---- Stimmen-Baukasten (Etappe 1) -----------------------------------------

/// Zustand des laufenden Kandidaten-Laufs — wie `AutoTagRun`, damit
/// `tts_builder_cancel` ihn abbrechen kann. `None` = kein Lauf aktiv.
#[derive(Default)]
pub struct BuilderRun(pub std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>);

#[tauri::command]
#[specta::specta]
pub fn tts_builder_create_draft(
    app: AppHandle,
    display_name: String,
    description: String,
    probe_text: String,
    tags: Vec<String>,
) -> Result<builder::BuilderDraft, String> {
    let tts = app.state::<Arc<TtsManager>>();
    let now = chrono::Utc::now().timestamp();
    let draft = builder::BuilderDraft {
        id: ulid::Ulid::new().to_string(),
        display_name,
        description,
        probe_text,
        tags,
        depth: 1.0,
        candidates: Vec::new(),
        selected: None,
        created_at: now,
        updated_at: now,
    };
    builder::save_draft(&tts.fish_dir_public(), &draft)?;
    Ok(draft)
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_list_drafts(app: AppHandle) -> Vec<builder::BuilderDraft> {
    let tts = app.state::<Arc<TtsManager>>();
    builder::list_drafts(&tts.fish_dir_public())
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_update_draft(
    app: AppHandle,
    mut draft: builder::BuilderDraft,
) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>();
    // Der Regler kommt aus der Oberflaeche und wird hier hart begrenzt:
    // ueber 1,15 klingt es kuenstlich, unter 1,0 hebt es die Stimme an, was
    // der Regler nicht anbietet.
    draft.depth = draft.depth.clamp(1.0, 1.15);
    draft.updated_at = chrono::Utc::now().timestamp();
    builder::save_draft(&tts.fish_dir_public(), &draft)
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_delete_draft(app: AppHandle, id: String) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>();
    builder::delete_draft(&tts.fish_dir_public(), &id)
}

/// Kandidaten erzeugen. Je fertigem Kandidaten geht ein
/// `tts-builder-progress`-Event `{done, total, seed}` an die Oberflaeche —
/// Kandidaten erscheinen damit einzeln statt alle am Ende.
#[tauri::command]
#[specta::specta]
pub async fn tts_builder_generate(
    app: AppHandle,
    id: String,
    count: usize,
) -> Result<builder::BuilderDraft, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    let (tx, rx) = tokio::sync::watch::channel(false);
    *app.state::<BuilderRun>().0.lock().unwrap() = Some(tx);
    let progress_app = app.clone();
    let result = tts
        .builder_generate(&id, count.clamp(1, 12), rx, move |done, total, cand| {
            let _ = progress_app.emit(
                "tts-builder-progress",
                serde_json::json!({ "done": done, "total": total, "seed": cand.seed }),
            );
        })
        .await;
    *app.state::<BuilderRun>().0.lock().unwrap() = None;
    result
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_cancel(app: AppHandle) -> Result<(), String> {
    if let Some(tx) = app.state::<BuilderRun>().0.lock().unwrap().as_ref() {
        let _ = tx.send(true);
    }
    Ok(())
}

/// WAV-Bytes eines Kandidaten mit dem aktuellen Tiefe-Regler. Roh als
/// `Vec<u8>` wie beim Avatar-Upload — das Projekt hat kein base64-Crate.
#[tauri::command]
#[specta::specta]
pub fn tts_builder_candidate_wav(app: AppHandle, id: String, seed: i64) -> Result<Vec<u8>, String> {
    app.state::<Arc<TtsManager>>()
        .builder_candidate_wav(&id, seed)
}

#[tauri::command]
#[specta::specta]
pub async fn tts_builder_commit(
    app: AppHandle,
    id: String,
    meta: VoiceMeta,
) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.builder_commit(&id, meta).await
}
