//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::error;
#[cfg(not(target_os = "windows"))]
use log::warn;
use tauri::AppHandle;
#[cfg(not(target_os = "windows"))]
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(not(target_os = "linux"))]
use crate::settings::get_settings;
use crate::settings::{self, ShortcutBinding};

#[cfg(not(target_os = "windows"))]
use super::handler::handle_shortcut_event;
#[cfg(target_os = "windows")]
use super::windows_hotkey;

/// Initialize shortcuts using Tauri's global-shortcut plugin
pub fn init_shortcuts(app: &AppHandle) -> Result<(), String> {
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    let mut failures = Vec::new();
    #[cfg(target_os = "windows")]
    let mut active = Vec::new();
    // Register all default shortcuts, applying user customizations
    for (id, default_binding) in default_bindings {
        if id == "cancel" {
            continue; // Skip cancel shortcut, it will be registered dynamically
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        match register_shortcut(app, binding.clone()) {
            Err(e) => {
                error!("Failed to register shortcut {} during init", id);
                failures.push((id, e));
            }
            Ok(()) => {
                #[cfg(target_os = "windows")]
                active.push(binding);
            }
        }
    }

    #[cfg(target_os = "windows")]
    if user_settings.keyboard_hooks_migration_pending {
        let mut restored = user_settings.clone();
        let fallback = restore_migrated_hooks(&mut restored, &failures);
        if fallback {
            // No mixed backends: release only registrations acquired above.
            for binding in active {
                unregister_shortcut(app, binding)?;
            }
        }
        settings::try_write_settings(app, restored)?;
        // Bestandsschutz must be durable before the hook backend can start.
        use tauri_plugin_store::StoreExt;
        app.store(crate::portable::store_path(settings::SETTINGS_STORE_PATH))
            .and_then(|store| store.save())
            .map_err(|_| String::from("Keyboard migration result could not be saved"))?;
        if fallback {
            super::handy_keys::init_shortcuts(app)?;
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        // Returned through initialize_shortcuts; the UI uses its existing
        // localized shortcut error text. No shortcut contents in the log.
        Err(failures.into_iter().map(|(id, error)| format!("{id}: {error}"))
            .collect::<Vec<_>>().join("; "))
    }
}

#[cfg(target_os = "windows")]
fn restore_migrated_hooks(settings: &mut settings::AppSettings, failures: &[(String, String)]) -> bool {
    let defaults = settings::get_default_settings().bindings;
    let restore = settings.keyboard_hooks_migration_pending
        && settings.keyboard_implementation == settings::KeyboardImplementation::Tauri
        && failures.iter().any(|(id, _)| {
            id != "cancel" && defaults.get(id).zip(settings.bindings.get(id))
                .is_some_and(|(default, binding)| {
                    !binding.current_binding.trim().is_empty()
                        && binding.current_binding != default.current_binding
                })
        });
    settings.keyboard_hooks_migration_pending = false;
    if restore {
        settings.keyboard_implementation = settings::KeyboardImplementation::HandyKeys;
        settings.keyboard_hooks_explicitly_enabled = true;
    }
    restore
}

#[cfg(all(test, target_os = "windows"))]
mod migration_tests {
    use super::*;
    use settings::KeyboardImplementation;

    #[test]
    fn only_a_failed_migrated_custom_binding_restores_the_previous_backend() {
        for pending in [false, true] {
            for custom in [false, true] {
                for failed in [false, true] {
                    let mut settings = settings::get_default_settings();
                    settings.keyboard_hooks_migration_pending = pending;
                    if custom {
                        settings.bindings.get_mut("transcribe").unwrap().current_binding = "ctrl+alt+f9".into();
                    }
                    let before = serde_json::to_value(&settings.bindings).unwrap();
                    let failures = if failed { vec![("transcribe".into(), "synthetic conflict".into())] } else { vec![] };
                    let expected = pending && custom && failed;
                    assert_eq!(restore_migrated_hooks(&mut settings, &failures), expected);
                    assert_eq!(settings.keyboard_hooks_explicitly_enabled, expected);
                    assert_eq!(settings.keyboard_implementation, if expected {
                        KeyboardImplementation::HandyKeys
                    } else { KeyboardImplementation::Tauri });
                    assert!(!settings.keyboard_hooks_migration_pending);
                    assert_eq!(serde_json::to_value(&settings.bindings).unwrap(), before);
                    assert!(!restore_migrated_hooks(&mut settings, &failures));
                }
            }
        }
    }

    #[test]
    fn failure_of_a_default_does_not_grandfather_another_custom_binding() {
        let mut settings = settings::get_default_settings();
        settings.keyboard_hooks_migration_pending = true;
        settings.bindings.get_mut("transcribe").unwrap().current_binding = "ctrl+alt+f9".into();
        assert!(!restore_migrated_hooks(&mut settings, &[("voice_call".into(), "synthetic conflict".into())]));
        assert!(!settings.keyboard_hooks_explicitly_enabled);
    }
}

/// Validate a shortcut string for the Tauri global-shortcut implementation.
/// Tauri requires at least one non-modifier key and doesn't support the fn key.
#[cfg(target_os = "windows")]
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    windows_hotkey::parse_shortcut(raw).map(|_| ())
}

#[cfg(not(target_os = "windows"))]
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }

    let modifiers = [
        "ctrl", "control", "shift", "alt", "option", "meta", "command", "cmd", "super", "win",
        "windows",
    ];

    // Check for fn key which Tauri doesn't support
    let parts: Vec<String> = raw.split('+').map(|p| p.trim().to_lowercase()).collect();
    for part in &parts {
        if part == "fn" || part == "function" {
            return Err("The 'fn' key is not supported by Tauri global shortcuts".into());
        }
    }

    // Check for at least one non-modifier key
    let has_non_modifier = parts.iter().any(|part| !modifiers.contains(&part.as_str()));

    if has_non_modifier {
        Ok(())
    } else {
        Err("Tauri shortcuts must include a main key (letter, number, F-key, etc.) in addition to modifiers".into())
    }
}

/// Register a shortcut using Tauri's global-shortcut plugin
#[cfg(target_os = "windows")]
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    windows_hotkey::register_shortcut(app, binding)
}

#[cfg(not(target_os = "windows"))]
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    // Validate for Tauri requirements
    if let Err(e) = validate_shortcut(&binding.current_binding) {
        warn!(
            "register_tauri_shortcut validation error for binding '{}': {}",
            binding.current_binding, e
        );
        return Err(e);
    }

    // Parse shortcut and return error if it fails
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", binding.current_binding);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    // Clone binding.id for use in the closure
    let binding_id_for_closure = binding.id.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, scut, event| {
            if scut == &shortcut {
                let shortcut_string = scut.into_string();
                let is_pressed = event.state == ShortcutState::Pressed;
                handle_shortcut_event(
                    app_handle,
                    &binding_id_for_closure,
                    &shortcut_string,
                    is_pressed,
                );
            }
        })
        .map_err(|e| {
            let error_msg = format!(
                "Couldn't register shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut registration error: {}", error_msg);
            error_msg
        })?;

    Ok(())
}

/// Unregister a shortcut from Tauri's global-shortcut plugin
#[cfg(target_os = "windows")]
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    windows_hotkey::unregister_shortcut(app, binding)
}

#[cfg(not(target_os = "windows"))]
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}' for unregistration: {}",
                binding.current_binding, e
            );
            error!("unregister_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    app.global_shortcut().unregister(shortcut).map_err(|e| {
        let error_msg = format!(
            "Failed to unregister shortcut '{}': {}",
            binding.current_binding, e
        );
        error!("unregister_tauri_shortcut error: {}", error_msg);
        error_msg
    })?;

    Ok(())
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    // Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                if let Err(e) = register_shortcut(&app_clone, cancel_binding) {
                    error!("Failed to register cancel shortcut: {}", e);
                }
            }
        });
    }
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    // Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        return;
    }

    #[cfg(not(target_os = "linux"))]
    {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(cancel_binding) = get_settings(&app_clone).bindings.get("cancel").cloned() {
                // We ignore errors here as it might already be unregistered
                let _ = unregister_shortcut(&app_clone, cancel_binding);
            }
        });
    }
}
