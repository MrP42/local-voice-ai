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

use tauri::{AppHandle, Emitter, Manager};

use crate::managers::llm::resources::system_memory;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings};

/// Die wirksame Einstellung fuer Pruefungen ohne `AppHandle` (z. B. `process_guard`): so
/// gelten fuer Diktat, Vorlesen und Sprachmodell dieselbe Grenze und derselbe Schalter.
/// Wird beim Start und bei jeder Aenderung nachgezogen ([`sync_config`]).
static GUARD_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static GUARD_PERCENT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(98);
/// Ende der einmaligen Freigabe „Trotzdem laden“ (monotone ms, 0 = keine).
static ALLOW_ONCE_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const ALLOW_ONCE_WINDOW: Duration = Duration::from_secs(120);

fn now_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static ANCHOR: OnceLock<Instant> = OnceLock::new();
    u64::try_from(ANCHOR.get_or_init(Instant::now).elapsed().as_millis())
        .unwrap_or(u64::MAX - 1)
        .saturating_add(1)
}

/// Liest Schalter und Grenze aus den Einstellungen in die globale Konfiguration.
pub fn sync_config(app: &AppHandle) {
    let settings = get_settings(app);
    set_config(settings.memory_guard_enabled, settings.memory_guard_percent);
}

pub(crate) fn set_config(enabled: bool, percent: u8) {
    use std::sync::atomic::Ordering;
    GUARD_ENABLED.store(enabled, Ordering::Relaxed);
    GUARD_PERCENT.store(clamp_percent(percent), Ordering::Relaxed);
}

/// Aktueller Schalter und aktuelle Grenze.
pub fn current_config() -> (bool, u8) {
    use std::sync::atomic::Ordering;
    (
        GUARD_ENABLED.load(Ordering::Relaxed),
        GUARD_PERCENT.load(Ordering::Relaxed),
    )
}

/// Gilt die Einmal-Freigabe „Trotzdem laden“ noch? Verbraucht sie (genau ein Ladevorgang).
fn take_allow_once() -> bool {
    use std::sync::atomic::Ordering;
    let until = ALLOW_ONCE_UNTIL.load(Ordering::Acquire);
    until != 0
        && now_ms() <= until
        && ALLOW_ONCE_UNTIL
            .compare_exchange(until, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
}

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

/// Diktatmodelle bis zu dieser Groesse werden nie hart gesperrt (lva#654): Auf einem 16-GB-
/// Firmenrechner mit viel belegtem RAM waere sonst kein Transkriptionsmodell ladbar und das
/// Diktat faellt aus. Sie laden mit Warnung, solange sie noch in den physischen Speicher passen.
pub const SMALL_MODEL_MB: u64 = 1_100;

/// Urteil des Speicherschutzes ueber einen Ladevorgang.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Unter der Grenze (oder nicht messbar).
    Ok,
    /// Ueber der Grenze, aber ein kleines Modell, das noch in den Speicher passt: laden, einmal warnen.
    Warn,
    /// Zu gross fuer Grenze und Speicher: nicht laden (grosse Modelle: mit Knopf „Trotzdem laden“).
    Block,
}

/// Entscheidung nach Messung. Nicht messbar (`total_mb == 0`) sperrt nie.
pub fn judge(used_mb: u64, need_mb: u64, total_mb: u64, percent: u8) -> Verdict {
    if !exceeds_limit(used_mb, need_mb, total_mb, percent) {
        Verdict::Ok
    } else if need_mb <= SMALL_MODEL_MB && used_mb.saturating_add(need_mb) <= total_mb {
        Verdict::Warn
    } else {
        Verdict::Block
    }
}

/// Dezenter Hinweis (kein Fehler) fuer ein trotz Ueberschreitung geladenes kleines Modell.
fn warning(what: &str, used_mb: u64, need_mb: u64, total_mb: u64, percent: u8) -> String {
    format!(
        "Speicher knapp: „{what}“ wurde trotzdem geladen. Belegt sind {:.1} GB von {:.1} GB, das Modell braucht etwa {:.1} GB, die Grenze liegt bei {percent} % ({:.1} GB). Schließen Sie bei Bedarf andere Programme oder ändern Sie die Grenze in den Einstellungen.",
        used_mb as f64 / 1024.0,
        total_mb as f64 / 1024.0,
        need_mb as f64 / 1024.0,
        limit_mb(total_mb, percent) as f64 / 1024.0,
    )
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
    if take_allow_once() {
        return Ok(());
    }
    release_blocking();
    std::thread::sleep(Duration::from_millis(400));
    let mem = system_memory();
    match judge(used(&mem), need_mb, mem.ram_total_mb, percent) {
        Verdict::Ok => Ok(()),
        Verdict::Warn => {
            let _ = app.emit(
                "memory-guard-warning",
                warning(what, used(&mem), need_mb, mem.ram_total_mb, percent),
            );
            Ok(())
        }
        Verdict::Block => Err(refusal(
            what,
            used(&mem),
            need_mb,
            mem.ram_total_mb,
            percent,
        )),
    }
}

/// Wie [`check_before_load_blocking`], async (Start des Chat-Servers). Hier
/// wird nur das ruhende Diktatmodell entladen; laufende Chat- und
/// Embedding-Aufgaben bleiben unberuehrt.
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
    if take_allow_once() {
        return Ok(());
    }
    // Vor dem Start des Chat-Servers nur das nachweislich ruhende Diktatmodell:
    // Chat- und Embedding-Server koennten gerade eine Aufgabe bearbeiten
    // (die /slots-Pruefung und der Anfragezaehler sind nicht atomar mit dem Stopp).
    release_idle_dictation(app).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mem = measure().await?;
    match judge(mem.ram_used_mb, need_mb, mem.ram_total_mb, percent) {
        Verdict::Ok => Ok(()),
        Verdict::Warn => {
            let _ = app.emit(
                "memory-guard-warning",
                warning(what, mem.ram_used_mb, need_mb, mem.ram_total_mb, percent),
            );
            Ok(())
        }
        Verdict::Block => Err(refusal(
            what,
            mem.ram_used_mb,
            need_mb,
            mem.ram_total_mb,
            percent,
        )),
    }
}

/// „Trotzdem laden“ (lva#654): erlaubt dem NAECHSTEN Ladevorgang, die Grenze zu uebergehen
/// (zwei Minuten gueltig, einmal verbraucht). Der Speicherschutz selbst bleibt eingeschaltet.
#[tauri::command]
#[specta::specta]
pub fn memory_guard_allow_once() {
    ALLOW_ONCE_UNTIL.store(
        now_ms().saturating_add(ALLOW_ONCE_WINDOW.as_millis() as u64),
        std::sync::atomic::Ordering::Release,
    );
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

/// Mindestruhe des Diktatmodells, bevor es fuer ein Sprachmodell weicht.
const DICTATION_MIN_IDLE: Duration = Duration::from_secs(30);

/// Entlaedt nur das Diktatmodell, und nur wenn es ruht (kein Engine-Zugriff,
/// keine Aufnahme, kein Laden, mindestens [`DICTATION_MIN_IDLE`] unbenutzt).
/// Liefert, ob entladen wurde. Fuer den Start des lokalen Sprachmodells bei
/// knappem RAM; Chat- und Embedding-Server bleiben unberuehrt.
pub(crate) async fn release_idle_dictation(app: &AppHandle) -> bool {
    let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() else {
        return false;
    };
    let tm = tm.inner().clone();
    tokio::task::spawn_blocking(move || tm.unload_if_idle(DICTATION_MIN_IDLE))
        .await
        .unwrap_or(false)
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
    sync_config(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_memory_guard_percent_setting(app: AppHandle, percent: u8) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.memory_guard_percent = clamp_percent(percent);
    write_settings(&app, settings);
    sync_config(&app);
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
    fn small_dictation_model_loads_with_warning_instead_of_blocking() {
        // Befund Firmen-PC 19.11.: 14,0 GB belegt von 15,5 GB, Modell 0,7 GB, Grenze 90 %.
        let (used, need, total) = (14_336, 717, 15_872);
        assert!(exceeds_limit(used, need, total, 90));
        assert_eq!(judge(used, need, total, 90), Verdict::Warn);
    }

    #[test]
    fn small_model_is_blocked_when_it_does_not_fit_physically() {
        assert_eq!(judge(15_500, 717, 15_872, 90), Verdict::Block);
    }

    #[test]
    fn large_model_stays_blocked_above_the_limit() {
        assert_eq!(judge(10_000, 5_000, 16_384, 90), Verdict::Block);
        assert_eq!(judge(10_000, SMALL_MODEL_MB + 1, 16_384, 99), Verdict::Ok);
        assert_eq!(
            judge(14_000, SMALL_MODEL_MB + 1, 16_384, 90),
            Verdict::Block
        );
    }

    #[test]
    fn under_the_limit_and_unmeasurable_are_ok() {
        assert_eq!(judge(10_000, 4_000, 16_384, 90), Verdict::Ok);
        assert_eq!(judge(u64::MAX, u64::MAX, 0, 60), Verdict::Ok);
    }

    #[test]
    fn warning_text_is_german_and_names_the_model() {
        let m = warning("Nemotron", 14_336, 717, 15_872, 90);
        assert!(m.contains("Nemotron") && m.contains("Speicher knapp") && m.contains("90 %"));
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
