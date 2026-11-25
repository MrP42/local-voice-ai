//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::error;
#[cfg(not(target_os = "windows"))]
use log::warn;
use tauri::AppHandle;
#[cfg(not(target_os = "windows"))]
use tauri::Manager;
#[cfg(not(target_os = "windows"))]
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(not(target_os = "linux"))]
use crate::settings::get_settings;
use crate::settings::{self, ShortcutBinding};

#[cfg(not(target_os = "windows"))]
use super::handler::handle_shortcut_event;
#[cfg(target_os = "windows")]
use super::windows_hotkey;

#[cfg(not(target_os = "windows"))]
#[derive(Default)]
struct PluginRegistrations(std::sync::Mutex<std::collections::HashMap<String, String>>);

#[cfg(not(target_os = "windows"))]
fn owns_registration(owned: &std::collections::HashMap<String, String>, binding: &ShortcutBinding) -> bool {
    owned.get(&binding.id) == Some(&binding.current_binding)
}

#[cfg(not(target_os = "windows"))]
fn plugin_transaction(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle, &mut std::collections::HashMap<String, String>) -> Result<(), String>
        + Send + 'static,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let app_clone = app.clone();
    // Plugin calls synchronously use the UI thread. Acquire ownership there
    // too, never on a worker waiting for a UI command that needs this lock.
    app.run_on_main_thread(move || {
        let app = app_clone;
        if app.try_state::<PluginRegistrations>().is_none() {
            app.manage(PluginRegistrations::default());
        }
        let registrations = app.state::<PluginRegistrations>();
        let result = registrations.0.lock()
            .map_err(|_| String::from("Shortcut registration lock poisoned"))
            .and_then(|mut owned| action(&app, &mut owned));
        let _ = tx.send(result);
    }).map_err(|_| String::from("Shortcut UI dispatch failed"))?;
    rx.recv().map_err(|_| String::from("Shortcut UI dispatch stopped"))?
}

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
    if user_settings.keyboard_hooks_migration_pending && !crate::richtlinie::aktuell().keine_tastatur_hooks {
        let mut restored = user_settings.clone();
        let fallback = restore_migrated_hooks(&mut restored, &failures);
        migration_transaction(
            |rollback| persist_migration(app, if rollback { user_settings.clone() } else { restored.clone() }),
            || {
                if fallback {
                    for binding in &active {
                        unregister_shortcut(app, binding.clone())?;
                    }
                    super::handy_keys::init_shortcuts(app)?;
                }
                Ok(())
            },
            || {
                let mut failed = false;
                for binding in &active {
                    failed |= register_shortcut(app, binding.clone()).is_err();
                }
                if failed { Err("native shortcut rollback failed".into()) } else { Ok(()) }
            },
        )?;
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
fn migration_transaction(
    mut persist: impl FnMut(bool) -> Result<(), String>,
    handoff: impl FnOnce() -> Result<(), String>,
    restore_native: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // Keep working native registrations until the choice is durable.
    let result = persist(false).and_then(|()| handoff());
    if let Err(mut error) = result {
        // Restore the in-memory choice too, including after a failed save.
        if persist(true).is_err() {
            error.push_str("; keyboard migration settings rollback failed");
        }
        if restore_native().is_err() {
            error.push_str("; native shortcut rollback failed");
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn persist_migration(app: &AppHandle, settings: settings::AppSettings) -> Result<(), String> {
    settings::try_write_settings(app, settings)?;
    // Bestandsschutz must be durable before the hook backend can start.
    use tauri_plugin_store::StoreExt;
    app.store(crate::portable::store_path(settings::SETTINGS_STORE_PATH))
        .and_then(|store| store.save())
        .map_err(|_| String::from("Keyboard migration result could not be saved"))
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
    fn migration_failures_restore_native_shortcuts_and_allow_retry() {
        use std::cell::RefCell;
        // Save, each unregister, and HandyKeys startup fail independently.
        for fail_at in 0..4 {
            let native = RefCell::new([true, true]);
            let choice = RefCell::new(false);
            let run = |fail: Option<usize>| migration_transaction(
                |rollback| {
                    *choice.borrow_mut() = !rollback;
                    if !rollback && fail == Some(0) { Err("synthetic save failure".into()) } else { Ok(()) }
                },
                || {
                    assert!(*choice.borrow(), "hooks require a durable choice");
                    for i in 0..2 {
                        if fail == Some(i + 1) { return Err("synthetic unregister failure".into()); }
                        native.borrow_mut()[i] = false;
                    }
                    assert_eq!(*native.borrow(), [false, false], "mixed backends");
                    if fail == Some(3) { Err("synthetic startup failure".into()) } else { Ok(()) }
                },
                || {
                    // Native re-registration is idempotent, including entries
                    // that were never released by the failed handoff.
                    *native.borrow_mut() = [true, true];
                    Ok(())
                },
            );
            assert!(run(Some(fail_at)).is_err());
            assert_eq!(*native.borrow(), [true, true]);
            assert!(!*choice.borrow());
            assert!(run(None).is_ok(), "retry after failure");
            assert_eq!(*native.borrow(), [false, false]);
            assert!(*choice.borrow());
        }
    }

    #[test]
    fn failed_settings_rollback_still_restores_native_shortcuts() {
        let mut restored = false;
        let error = migration_transaction(
            |_| Err("synthetic save failure".into()),
            || panic!("handoff after failed save"),
            || { restored = true; Ok(()) },
        ).unwrap_err();
        assert!(restored);
        assert!(error.contains("settings rollback failed"));
    }

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

    plugin_transaction(app, move |app, owned| {
        // Reuse only this binding's successful registration after a partial init.
        if app.global_shortcut().is_registered(shortcut) {
            if owns_registration(owned, &binding) {
                return Ok(());
            }
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

        owned.insert(binding.id, binding.current_binding);
        Ok(())
    })
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

    plugin_transaction(app, move |app, owned| {
        app.global_shortcut().unregister(shortcut).map_err(|e| {
            let error_msg = format!(
                "Failed to unregister shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("unregister_tauri_shortcut error: {}", error_msg);
            error_msg
        })?;

        owned.remove(&binding.id);
        Ok(())
    })
}

#[cfg(all(test, not(target_os = "windows")))]
mod plugin_retry_tests {
    use super::*;

    #[test]
    fn retry_does_not_claim_another_bindings_chord_or_an_old_chord() {
        let mut binding = settings::get_default_settings().bindings["transcribe"].clone();
        binding.current_binding = "ctrl+space".into();
        let mut owned = std::collections::HashMap::new();
        assert!(!owns_registration(&owned, &binding));
        owned.insert(binding.id.clone(), binding.current_binding.clone());
        assert!(owns_registration(&owned, &binding));
        let mut other = binding.clone();
        other.id = "transcribe_with_post_process".into();
        assert!(!owns_registration(&owned, &other));
        binding.current_binding = "ctrl+alt+f9".into();
        assert!(!owns_registration(&owned, &binding));
        owned.remove(&binding.id);
        assert!(!owns_registration(&owned, &binding));
    }
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
