//! Speicherschutz: kein Modell laden, das die RAM-Grenze ueberschreiten wuerde,
//! und vorher ungenutzte Modelle entladen. Dazu der Knopf "Speicher freigeben".
//!
//! Gesichert sind die zwei Haupteinstiege: das Diktatmodell
//! (`TranscriptionManager::load_model_with_device`) und der lokale Chat-Server
//! (`llm::ensure_local_with`). Nicht gesichert: Vorlesen (Fish/Phonon haben ihr
//! eigenes RAM-Tor in `process_guard`), der Embedding-Server und die zusaetzlichen
//! Engines der Import-Warteschlange (U7).

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::managers::llm::resources::system_memory;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings};

pub const MIN_PERCENT: u8 = 60;
pub const MAX_PERCENT: u8 = 98;

/// Mindestzeit ohne Nutzung, bevor der Speicherschutz einen Chat-Server
/// beendet, den er nicht auf `/slots` pruefen kann (blockierender Pfad).
const BLOCKING_MIN_IDLE: Duration = Duration::from_secs(60);

pub fn clamp_percent(percent: u8) -> u8 {
    percent.clamp(MIN_PERCENT, MAX_PERCENT)
}

/// Grenze in MB bei `total_mb` Gesamtspeicher.
pub fn limit_mb(total_mb: u64, percent: u8) -> u64 {
    total_mb * u64::from(clamp_percent(percent)) / 100
}

/// Wuerde `used_mb + need_mb` die Grenze ueberschreiten? Nicht messbar
/// (`total_mb == 0`) blockiert nie.
pub fn exceeds_limit(used_mb: u64, need_mb: u64, total_mb: u64, percent: u8) -> bool {
    total_mb != 0 && used_mb.saturating_add(need_mb) > limit_mb(total_mb, percent)
}

fn refusal(what: &str, used_mb: u64, need_mb: u64, total_mb: u64, percent: u8) -> String {
    format!(
        "Speicherschutz: „{what}“ wird nicht geladen. Belegt sind {:.1} GB von {:.1} GB, das Modell braucht etwa {:.1} GB, die Grenze liegt bei {percent} % ({:.1} GB). Schließen Sie andere Programme, geben Sie Speicher frei oder erhöhen Sie die Grenze.",
        used_mb as f64 / 1024.0,
        total_mb as f64 / 1024.0,
        need_mb as f64 / 1024.0,
        limit_mb(total_mb, percent) as f64 / 1024.0,
    )
}

/// Gibt im blockierenden Pfad nur frei, was einen Zaehler fuer laufende
/// Anfragen hat (Embedding-Server). Der Chat-Server hat keinen: ihn ohne
/// `/slots`-Pruefung zu beenden koennte eine laufende Anfrage abbrechen, darum
/// bleibt er hier unberuehrt (der async-Pfad prueft `/slots`).
fn release_blocking() {
    crate::managers::llm::stop_embedding_if_idle(BLOCKING_MIN_IDLE);
}

/// Prueft vor dem Laden des Diktatmodells (blockierend, nie auf einem
/// Tokio-Worker). `credit_mb`: Groesse des Modells, das beim Wechsel ohnehin
/// entladen wird; es zaehlt nicht als belegt. `Ok`, wenn der Schutz aus ist,
/// nichts messbar ist oder es nach dem Entladen ungenutzter Modelle passt.
pub fn check_before_load_blocking(
    app: &AppHandle,
    need_mb: u64,
    credit_mb: u64,
    what: &str,
) -> Result<(), String> {
    let settings = get_settings(app);
    if !settings.memory_guard_enabled {
        return Ok(());
    }
    let percent = clamp_percent(settings.memory_guard_percent);
    let mem = system_memory();
    let used = |m: &crate::managers::llm::SystemMemory| m.ram_used_mb.saturating_sub(credit_mb);
    if !exceeds_limit(used(&mem), need_mb, mem.ram_total_mb, percent) {
        return Ok(());
    }
    release_blocking();
    std::thread::sleep(Duration::from_millis(400));
    let mem = system_memory();
    if exceeds_limit(used(&mem), need_mb, mem.ram_total_mb, percent) {
        return Err(refusal(
            what,
            used(&mem),
            need_mb,
            mem.ram_total_mb,
            percent,
        ));
    }
    Ok(())
}

/// Wie [`check_before_load_blocking`], async (Start des Chat-Servers). Hier
/// kann der Chat-Server mit `/slots`-Pruefung und das Diktatmodell im Leerlauf
/// entladen werden.
pub async fn check_before_load(app: &AppHandle, need_mb: u64, what: &str) -> Result<(), String> {
    let settings = get_settings(app);
    if !settings.memory_guard_enabled {
        return Ok(());
    }
    let percent = clamp_percent(settings.memory_guard_percent);
    let measure = || async {
        tokio::task::spawn_blocking(system_memory)
            .await
            .map_err(|e| e.to_string())
    };
    let mem = measure().await?;
    if !exceeds_limit(mem.ram_used_mb, need_mb, mem.ram_total_mb, percent) {
        return Ok(());
    }
    release_idle(app).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mem = measure().await?;
    if exceeds_limit(mem.ram_used_mb, need_mb, mem.ram_total_mb, percent) {
        return Err(refusal(
            what,
            mem.ram_used_mb,
            need_mb,
            mem.ram_total_mb,
            percent,
        ));
    }
    Ok(())
}

/// Entlaedt alles, was gerade nicht arbeitet: Diktatmodell (Leerlauf),
/// Chat-Server (nicht beschaeftigt) und Embedding-Server.
async fn release_idle(app: &AppHandle) {
    if let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() {
        let tm = tm.inner().clone();
        let _ = tokio::task::spawn_blocking(move || tm.unload_if_idle(Duration::ZERO)).await;
    }
    crate::managers::llm::stop_local_if_idle(Some(Duration::ZERO)).await;
    let _ = tokio::task::spawn_blocking(|| {
        crate::managers::llm::stop_embedding_if_idle(Duration::ZERO)
    })
    .await;
}

/// "Speicher freigeben": entlaedt nicht arbeitende Modelle und liefert die
/// freigegebenen MB (RAM vorher minus nachher, gemessen; nie negativ).
#[tauri::command]
#[specta::specta]
pub async fn release_memory(app: AppHandle) -> Result<u64, String> {
    let measure = || async {
        tokio::task::spawn_blocking(system_memory)
            .await
            .map_err(|e| e.to_string())
    };
    let before = measure().await?;
    release_idle(&app).await;
    // Das Betriebssystem braucht einen Moment, bis die Seiten frei sind.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let after = measure().await?;
    Ok(before.ram_used_mb.saturating_sub(after.ram_used_mb))
}

#[tauri::command]
#[specta::specta]
pub fn change_memory_guard_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.memory_guard_enabled = enabled;
    write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_memory_guard_percent_setting(app: AppHandle, percent: u8) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.memory_guard_percent = clamp_percent(percent);
    write_settings(&app, settings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_clamped_to_range() {
        assert_eq!(clamp_percent(10), 60);
        assert_eq!(clamp_percent(90), 90);
        assert_eq!(clamp_percent(255), 98);
    }

    #[test]
    fn limit_blocks_only_above_threshold() {
        // 16 GB, 90 % = 14.745 MB Grenze
        assert!(!exceeds_limit(10_000, 4_000, 16_384, 90));
        assert!(exceeds_limit(10_000, 5_000, 16_384, 90));
        assert!(!exceeds_limit(10_000, 5_000, 16_384, 98));
    }

    #[test]
    fn unmeasurable_total_never_blocks() {
        assert!(!exceeds_limit(u64::MAX, u64::MAX, 0, 60));
    }

    #[test]
    fn refusal_message_is_german_and_names_the_model() {
        let m = refusal("Nemotron", 12_000, 4_000, 16_384, 90);
        assert!(m.contains("Nemotron") && m.contains("Speicherschutz") && m.contains("90 %"));
    }
}
