//! Built-in Apple capabilities, checked after the UI exists (Foundation
//! Models must not be queried during early app initialization).
use crate::settings::{self, LlmConnection, LlmModelConfig};
use tauri::{AppHandle, Manager};

#[derive(serde::Serialize, specta::Type)]
pub struct AppleSystemStatus {
    pub speech_available: bool,
    pub speech_authorized: bool,
    pub llm_available: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn apple_system_status() -> AppleSystemStatus {
    AppleSystemStatus {
        speech_available: crate::apple_speech::available("auto"),
        speech_authorized: crate::apple_speech::authorized(),
        llm_available: crate::commands::check_apple_intelligence_available(),
    }
}

/// Register the usable built-in LLM without replacing a configured model.
/// No model is downloaded and no cloud provider is contacted.
#[tauri::command]
#[specta::specta]
pub async fn apple_system_initialize(app: AppHandle) -> AppleSystemStatus {
    let status = apple_system_status().await;
    let mut settings = settings::get_settings(&app);
    let mut changed = false;
    // Migrate only the unusable legacy default. Named voices and custom
    // installations are user choices and remain untouched.
    if cfg!(target_os = "macos")
        && settings.tts_engine == "fish"
        && settings.tts_voice.is_none()
        && !app
            .state::<std::sync::Arc<crate::managers::tts::TtsManager>>()
            .fish_is_installed()
        && (settings.tts_fish_dir.is_empty() || settings.tts_fish_dir == r"C:\AI\fish-speech")
    {
        settings.tts_engine = "system".into();
        changed = true;
    }
    if status.llm_available {
        changed |= register_apple_default(&mut settings);
    }
    if changed {
        settings::write_settings(&app, settings);
    }
    status
}

fn register_apple_default(s: &mut settings::AppSettings) -> bool {
    let id = settings::APPLE_INTELLIGENCE_PROVIDER_ID;
    let remote = settings::APPLE_INTELLIGENCE_DEFAULT_MODEL_ID;
    if s.llm_connections.iter().any(|c| c.id == id && !c.enabled) {
        return false;
    }
    let mut changed = false;
    if !s.llm_connections.iter().any(|c| c.id == id) {
        s.llm_connections.push(LlmConnection {
            id: id.into(),
            kind: id.into(),
            label: remote.into(),
            base_url: "apple-intelligence://local".into(),
            enabled: true,
            monthly_budget_usd: None,
            budget_enforced: false,
        });
        changed = true;
    }
    if !s.post_process_providers.iter().any(|p| p.id == id) {
        s.post_process_providers
            .push(settings::PostProcessProvider {
                id: id.into(),
                label: remote.into(),
                base_url: "apple-intelligence://local".into(),
                allow_base_url_edit: false,
                models_endpoint: None,
                supports_structured_output: true,
            });
        changed = true;
    }
    let model_id = LlmModelConfig::make_id(id, remote);
    if !s.llm_models.iter().any(|m| m.id == model_id) {
        s.llm_models.push(LlmModelConfig {
            id: model_id.clone(),
            connection_id: id.into(),
            remote_id: remote.into(),
            label: remote.into(),
            enabled: true,
            context_limit: Some(4096),
            max_input_tokens: None,
            max_output_tokens: None,
            price_input_per_mtok: Some(0.0),
            price_output_per_mtok: Some(0.0),
            tags: vec!["local".into(), "system".into()],
        });
        changed = true;
    }
    if s.llm_active_model_id.is_none() && s.llm_models.iter().any(|m| m.id == model_id && m.enabled)
    {
        s.llm_active_model_id = Some(model_id);
        s.sync_legacy_from_llm();
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_llm_is_default_only_without_an_existing_selection() {
        let mut settings = settings::get_default_settings();
        assert!(register_apple_default(&mut settings));
        assert_eq!(settings.post_process_provider_id, "apple_intelligence");
        assert!(
            !register_apple_default(&mut settings),
            "initialization is idempotent"
        );
        settings.llm_active_model_id = Some("user:model".into());
        register_apple_default(&mut settings);
        assert_eq!(settings.llm_active_model_id.as_deref(), Some("user:model"));
    }
    #[test]
    fn an_explicitly_disabled_apple_connection_stays_disabled() {
        let mut settings = settings::get_default_settings();
        register_apple_default(&mut settings);
        settings.llm_active_model_id = None;
        settings
            .llm_connections
            .iter_mut()
            .find(|c| c.id == "apple_intelligence")
            .unwrap()
            .enabled = false;
        assert!(!register_apple_default(&mut settings));
        assert!(settings.llm_active_model_id.is_none());
    }
}
