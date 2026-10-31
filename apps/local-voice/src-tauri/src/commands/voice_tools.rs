//! Werkzeuge fuer den Orchestrator des Sprachgespraechs (Stufe 2): `voice_tools_list` und
//! `voice_tool_call`. Die Logik steht in `crate::agent_bridge::voice` (nur lesende Werkzeuge des
//! Katalogs, jeder Aufruf durch das Tor mit Audit); hier nur Zugriff auf Zustand und Blockier-Thread.
//!
//! Fehlermeldungen und Antworten enthalten keine Inhalte aus Besprechungen ausser dem Ergebnis des
//! Werkzeugs selbst, das an das Sprachmodell geht; nichts davon wird geloggt.

use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager};

use crate::agent_bridge::runtime::AppBridge;
use crate::agent_bridge::voice::{self, RateLimiter, VoiceOptions, VoiceToolInfo};
use crate::managers::meetings::store::MeetingStore;

/// Aufrufe je Minute (alle Gespräche): eine Schleife des Modells darf die Datenbank nicht fluten.
static LIMITER: Lazy<Mutex<RateLimiter>> = Lazy::new(Default::default);

fn options(app: &AppHandle) -> VoiceOptions {
    VoiceOptions {
        transcripts: crate::settings::get_settings(app).voice_call_read_transcripts,
    }
}

/// „Transkripte im Sprachgespräch lesen“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_voice_call_read_transcripts_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.voice_call_read_transcripts = enabled;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Bruecke und Datenbank des Laufs; fehlt eines, ist das ein Klartext (die App startet noch).
fn parts(app: &AppHandle) -> Result<(Arc<AppBridge>, Arc<MeetingStore>), String> {
    let bridge = app
        .try_state::<Arc<AppBridge>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Die Werkzeuge sind noch nicht bereit.".to_string())?;
    let store = app
        .try_state::<Arc<MeetingStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Die Besprechungsdatenbank ist noch nicht bereit.".to_string())?;
    Ok((bridge, store))
}

/// Die lesenden Werkzeuge, die das Sprachgespräch jetzt benutzen darf.
#[tauri::command]
#[specta::specta]
pub async fn voice_tools_list(app: AppHandle) -> Result<Vec<VoiceToolInfo>, String> {
    let (bridge, store) = parts(&app)?;
    let opts = options(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store.get_connection().map_err(|e| e.to_string())?;
        voice::list(&conn, bridge.server().bridge().registry(), opts, now_ms())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ruft ein lesendes Werkzeug über das Tor der Agentenbrücke (Recht, Audit). `args` ist ein
/// JSON-Objekt als Text. Die Antwort ist der JSON-Text des Ergebnisses; eine zu lange Antwort wird nie still gekürzt, sondern abgelehnt.
#[tauri::command]
#[specta::specta]
pub async fn voice_tool_call(app: AppHandle, name: String, args: String) -> Result<String, String> {
    let (bridge, store) = parts(&app)?;
    let opts = options(&app);
    let allowed = LIMITER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allow(now_ms());
    if !allowed {
        return Err(format!(
            "Zu viele Werkzeugaufrufe in kurzer Zeit (höchstens {} pro Minute). Es wurde nichts ausgeführt.",
            RateLimiter::MAX_CALLS
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store.get_connection().map_err(|e| e.to_string())?;
        voice::call(
            &conn,
            bridge.server().bridge().registry(),
            opts,
            &name,
            &args,
            now_ms(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
