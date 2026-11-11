//! G2 (#497): Befehle zum Hinweis an Teilnehmende einer Besprechung.
//!
//! Duenne Schale ueber `managers::compliance::notice` (Regeln, Textbausteine,
//! Datenformat) und `managers::meetings::notice` (Ablage in der Besprechung).
//! Die App sendet nie selbst: Kopieren legt nur Text in die Zwischenablage,
//! Einfuegen drueckt nur Ctrl+V (Cmd+V) in das Fenster, das der Nutzer nach
//! einer Frist im Vordergrund hat (`clipboard::paste_notice_guarded`), nie
//! Enter und nie "Senden".
//!
//! Ohne `meeting_id` entsteht der Nachweis vor dem Start (Startdialog) und wird
//! vom Recorder nach erfolgreichem Start an die neue Besprechung gehaengt.
//!
//! Fehlercodes (Konvention aus `commands::meetings`): `notice_disabled`,
//! `notice_cancelled`, `notice_busy`, `clipboard_failed`, `store_failed`,
//! `meetings_unavailable`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::clipboard::{self, GuardedPasteOutcome};
use crate::commands::meetings::require_managed;
use crate::managers::compliance::notice::{
    self as nc, MeetingNoticeSettings, NoticeApp, NoticeFallback, NoticeLang, NoticeRecord,
    NoticeWay,
};
use crate::managers::meetings::notice as store_notice;
use crate::managers::meetings::store::MeetingStore;
use crate::paste_guard::{self, PasteFallback};
use crate::settings;

/// Eine Frist laeuft: abbrechen erlaubt (`meeting_notice_cancel_paste`).
static CANCEL: AtomicBool = AtomicBool::new(false);
/// Hoechstens ein Einfuegen zugleich.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Setzt `RUNNING` bei jedem Ausgang zurueck.
struct RunGuard;

impl RunGuard {
    fn enter() -> Result<Self, String> {
        if RUNNING.swap(true, Ordering::AcqRel) {
            return Err("notice_busy".to_string());
        }
        Ok(Self)
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Stand fuer die Oberflaeche.
#[derive(Serialize, Debug, Clone, Type)]
pub struct NoticeStatus {
    /// Nachweise der Besprechung bzw. die geltenden vor dem Start (aelteste zuerst).
    pub records: Vec<NoticeRecord>,
    /// Kann diese Plattform in ein fremdes Fenster einfuegen? (Nur Windows kann
    /// pruefen, dass der Fokus im Zielfenster blieb.)
    pub paste_available: bool,
}

/// Ergebnis von Kopieren, Einfuegen und "muendlich informiert".
#[derive(Serialize, Debug, Clone, Type)]
pub struct NoticeOutcome {
    pub record: NoticeRecord,
    /// Gesetzt, wenn aus dem Einfuegen nur ein Kopieren wurde.
    pub fallback: Option<NoticeFallback>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn enabled_settings(app: &AppHandle) -> Result<MeetingNoticeSettings, String> {
    let s = settings::get_settings(app).meeting_notice;
    if s.enabled {
        Ok(s)
    } else {
        Err("notice_disabled".to_string())
    }
}

/// Name des Anbieters, an den der Text der Notizen geht; `None`, wenn alles
/// lokal bleibt (auch ohne aktiven Anbieter, wie in der Oberflaeche).
fn external_provider(app: &AppHandle) -> Option<String> {
    let s = settings::get_settings(app);
    s.post_process_providers
        .iter()
        .find(|p| p.id == s.post_process_provider_id)
        .filter(|p| !crate::managers::compliance::is_local(p))
        .map(|p| p.label.clone())
}

fn text_now(
    app: &AppHandle,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<(MeetingNoticeSettings, String), String> {
    let s = enabled_settings(app)?;
    let text = nc::text_for(&s, notice_app, lang, external_provider(app).as_deref());
    Ok((s, text))
}

/// Legt den Nachweis ab: an der Besprechung `meeting_id`, sonst vor dem Start.
fn store_record(
    app: &AppHandle,
    meeting_id: Option<&str>,
    record: &NoticeRecord,
) -> Result<(), String> {
    match meeting_id {
        Some(id) => {
            let store = require_managed(
                app.try_state::<Arc<MeetingStore>>()
                    .map(|state| Arc::clone(&state)),
            )?;
            store_notice::record_on_meeting(&store, id, record)
        }
        None => {
            nc::PENDING.add(record.clone());
            Ok(())
        }
    }
}

fn map_fallback(reason: PasteFallback) -> Result<NoticeFallback, String> {
    match reason {
        PasteFallback::NoTarget => Ok(NoticeFallback::NoTarget),
        PasteFallback::FocusChanged | PasteFallback::FocusChangedDuringPaste => {
            Ok(NoticeFallback::FocusChanged)
        }
        PasteFallback::TargetElevated => Ok(NoticeFallback::TargetElevated),
        PasteFallback::InjectionFailed => Ok(NoticeFallback::InjectionFailed),
        PasteFallback::AccessibilityDenied => Ok(NoticeFallback::AccessibilityDenied),
        PasteFallback::ClipboardUnverified => Err("clipboard_failed".to_string()),
    }
}

/// Der fertige Text fuer die Vorschau und zum Kopieren.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_preview(
    app: AppHandle,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<String, String> {
    text_now(&app, notice_app, lang).map(|(_, text)| text)
}

/// Der Textbaustein mit seinen Platzhaltern (die Anpassung des Nutzers, sonst der
/// mitgelieferte), zum Bearbeiten in den Einstellungen.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_template(
    app: AppHandle,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<String, String> {
    Ok(nc::template_for(
        &settings::get_settings(&app).meeting_notice,
        notice_app,
        lang,
    ))
}

/// Nachweise der Besprechung `meeting_id`, ohne sie die geltenden vor dem Start.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_status(
    app: AppHandle,
    meeting_id: Option<String>,
) -> Result<NoticeStatus, String> {
    let records = match meeting_id {
        Some(id) => {
            let store = require_managed(
                app.try_state::<Arc<MeetingStore>>()
                    .map(|state| Arc::clone(&state)),
            )?;
            store_notice::records_of(&store, &id)
        }
        None => nc::PENDING.fresh(now()),
    };
    Ok(NoticeStatus {
        records,
        paste_available: cfg!(target_os = "windows"),
    })
}

/// Verwirft die Nachweise vor dem Start (der Startdialog beginnt von vorn).
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_reset_pending() -> Result<(), String> {
    nc::PENDING.clear();
    Ok(())
}

/// Legt den Hinweistext in die Zwischenablage und haelt das als Nachweis fest.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_copy(
    app: AppHandle,
    meeting_id: Option<String>,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<NoticeOutcome, String> {
    let (s, text) = text_now(&app, notice_app, lang)?;
    if !clipboard::copy_text_verified(&app, &text) {
        return Err("clipboard_failed".to_string());
    }
    let record = NoticeRecord {
        at: now(),
        way: NoticeWay::Clipboard,
        app: notice_app,
        lang,
        text,
        in_export: s.export_line,
    };
    store_record(&app, meeting_id.as_deref(), &record)?;
    Ok(NoticeOutcome {
        record,
        fallback: None,
    })
}

/// Der Nutzer bestaetigt, die Teilnehmenden muendlich informiert zu haben.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_oral(
    app: AppHandle,
    meeting_id: Option<String>,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<NoticeOutcome, String> {
    let s = enabled_settings(&app)?;
    let record = NoticeRecord {
        at: now(),
        way: NoticeWay::Oral,
        app: notice_app,
        lang,
        text: String::new(),
        in_export: s.export_line,
    };
    store_record(&app, meeting_id.as_deref(), &record)?;
    Ok(NoticeOutcome {
        record,
        fallback: None,
    })
}

/// Fuegt den Hinweistext nach einer Frist in das Chatfeld des Fensters ein, das
/// dann im Vordergrund steht (die Frist nutzt der Nutzer, um in das Chatfeld
/// der Besprechung zu klicken). Gesendet wird nie. Steht dann kein fremdes
/// Fenster im Vordergrund oder scheitert eine Sicherung, bleibt der Text in
/// der Zwischenablage (`fallback`), und der Nachweis lautet "kopiert".
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_paste(
    app: AppHandle,
    meeting_id: Option<String>,
    notice_app: NoticeApp,
    lang: NoticeLang,
    countdown_secs: u32,
) -> Result<NoticeOutcome, String> {
    let (s, text) = text_now(&app, notice_app, lang)?;
    let _running = RunGuard::enter()?;
    CANCEL.store(false, Ordering::Release);
    let secs = nc::clamp_countdown(countdown_secs);
    let paste_app = app.clone();
    let (way, fallback) = tauri::async_runtime::spawn_blocking(move || {
        for _ in 0..secs {
            for _ in 0..10 {
                if CANCEL.load(Ordering::Acquire) {
                    return Err("notice_cancelled".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if CANCEL.load(Ordering::Acquire) {
            return Err("notice_cancelled".to_string());
        }
        let target = paste_guard::capture_paste_target();
        let outcome = match nc::paste_target_check(target.map(|t| t.pid), std::process::id()) {
            Ok(_) => clipboard::paste_notice_guarded(&text, paste_app, target),
            Err(reason) => {
                if !clipboard::copy_text_verified(&paste_app, &text) {
                    return Err("clipboard_failed".to_string());
                }
                return Ok((NoticeWay::Clipboard, Some(reason)));
            }
        };
        match outcome {
            GuardedPasteOutcome::Pasted => Ok((NoticeWay::Paste, None)),
            GuardedPasteOutcome::NothingToDo => Err("notice_cancelled".to_string()),
            GuardedPasteOutcome::Fallback(reason) => {
                Ok((NoticeWay::Clipboard, Some(map_fallback(reason)?)))
            }
        }
    })
    .await
    .map_err(|e| format!("meeting_notice_paste panicked: {e}"))??;
    let text = nc::text_for(&s, notice_app, lang, external_provider(&app).as_deref());
    let record = NoticeRecord {
        at: now(),
        way,
        app: notice_app,
        lang,
        text,
        in_export: s.export_line,
    };
    store_record(&app, meeting_id.as_deref(), &record)?;
    Ok(NoticeOutcome { record, fallback })
}

/// Bricht die laufende Frist des Einfuegens ab.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_cancel_paste() -> Result<(), String> {
    CANCEL.store(true, Ordering::Release);
    Ok(())
}

/// Einstellungen des Hinweises speichern (Texte werden begrenzt und bereinigt).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_notice_setting(
    app: AppHandle,
    value: MeetingNoticeSettings,
) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.meeting_notice = value.sanitized();
    settings::write_settings(&app, s);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_paste_fallback_maps_to_a_notice_fallback_except_an_unverified_clipboard() {
        assert_eq!(
            map_fallback(PasteFallback::NoTarget),
            Ok(NoticeFallback::NoTarget)
        );
        assert_eq!(
            map_fallback(PasteFallback::FocusChangedDuringPaste),
            Ok(NoticeFallback::FocusChanged)
        );
        assert_eq!(
            map_fallback(PasteFallback::TargetElevated),
            Ok(NoticeFallback::TargetElevated)
        );
        assert_eq!(
            map_fallback(PasteFallback::ClipboardUnverified),
            Err("clipboard_failed".to_string())
        );
    }

    #[test]
    fn a_second_paste_is_refused_while_one_runs_and_the_flag_is_released() {
        let first = RunGuard::enter();
        assert!(first.is_ok());
        assert_eq!(RunGuard::enter().err(), Some("notice_busy".to_string()));
        drop(first);
        assert!(RunGuard::enter().is_ok());
    }

    /// Der Einfuegepfad fuer Hinweise darf nie Enter senden: die Funktion kennt
    /// weder `send_return_key` noch `auto_submit`.
    #[test]
    fn the_notice_paste_never_submits() {
        let source = include_str!("../clipboard.rs").replace("\r\n", "\n");
        let body = source
            .split("pub fn paste_notice_guarded(")
            .nth(1)
            .and_then(|rest| rest.split("/// Paste a finished dictation").next())
            .expect("paste_notice_guarded gefunden");
        assert!(!body.contains("send_return_key"));
        assert!(!body.contains("auto_submit"));
        assert!(body.contains("send_paste_ctrl_v"));
    }
}
