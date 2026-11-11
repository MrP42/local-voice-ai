//! G2 (#497): Befehle zum Hinweis an Teilnehmende einer Besprechung.
//!
//! Duenne Schale ueber `managers::compliance::notice` (Regeln, Textbausteine,
//! Datenformat) und `managers::meetings::notice` (Ablage in der Besprechung).
//! Die App sendet nie selbst: Kopieren legt nur Text in die Zwischenablage,
//! Einfuegen drueckt nur Ctrl+V (Cmd+V) in das Fenster, das der Nutzer nach
//! einer Frist im Vordergrund hat (`clipboard::paste_notice_guarded`), nie
//! Enter und nie "Senden". Eingefuegt wird nur in eine Besprechungs-App oder
//! einen Browser (`nc::paste_target_decision`), nie in ein beliebiges Fenster.
//!
//! Ohne `meeting_id` entsteht der Nachweis vor dem Start im Startdialog. Der
//! Dialog bekommt beim Oeffnen ein Token (`meeting_notice_dialog_open`); der
//! Nachweis gehoert zu diesem Token, und nur der Start mit demselben Token
//! (`meetings_start*`) sieht ihn. Schliesst der Dialog, verfaellt er
//! (`meeting_notice_dialog_close`).
//!
//! Fehlercodes (Konvention aus `commands::meetings`): `notice_disabled`,
//! `notice_cancelled`, `notice_busy`, `notice_dialog_gone`, `clipboard_failed`,
//! `store_failed`, `meetings_unavailable`.

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
    NoticeTarget, NoticeWay,
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
    /// Nachweise der Besprechung bzw. die geltenden des Startdialogs (aelteste zuerst).
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

/// Namen der Anbieter, an die fuer die Besprechung Text geht und die nicht auf
/// diesem Rechner laufen: der Standardanbieter (Chat, Folgeaufgaben, Einordnung)
/// und der Anbieter der Notizen und Protokolle (Modell der Funktion
/// `meeting_protocol`, falls eigens gewaehlt). Leer = alles bleibt lokal (auch
/// ohne aktiven Anbieter, wie in der Oberflaeche).
fn external_providers(app: &AppHandle) -> Vec<String> {
    let all = settings::get_settings(app);
    let for_notes = all.for_function(settings::FN_MEETING_PROTOCOL);
    let used: Vec<(String, bool)> = [&all, &for_notes]
        .into_iter()
        .filter_map(|s| {
            s.post_process_providers
                .iter()
                .find(|p| p.id == s.post_process_provider_id)
        })
        .map(|p| (p.label.clone(), crate::managers::compliance::is_local(p)))
        .collect();
    nc::external_names(used.iter().map(|(label, local)| (label.as_str(), *local)))
}

fn text_now(
    app: &AppHandle,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<(MeetingNoticeSettings, String), String> {
    let s = enabled_settings(app)?;
    let text = nc::text_for(&s, notice_app, lang, &external_providers(app));
    Ok((s, text))
}

/// Legt den Nachweis ab: an der Besprechung oder im offenen Startdialog.
fn store_record(
    app: &AppHandle,
    target: NoticeTarget<'_>,
    record: &NoticeRecord,
) -> Result<(), String> {
    match target {
        NoticeTarget::Meeting(id) => {
            let store = require_managed(
                app.try_state::<Arc<MeetingStore>>()
                    .map(|state| Arc::clone(&state)),
            )?;
            store_notice::record_on_meeting(&store, id, record)
        }
        NoticeTarget::Dialog(token) => nc::PENDING.add(token, record.clone()),
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

/// Name der Programmdatei zu einem Prozess (`ms-teams.exe`), nie ein
/// Fenstertitel (Titel koennen Personendaten enthalten). `None`, wenn der
/// Prozess nicht lesbar ist; der Aufrufer behandelt das als unbekanntes Ziel.
/// Laeuft im eigenen Prozess (kein Kindprozess) und liest nur diesen einen Eintrag.
fn process_name(pid: u32) -> Option<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let process = system.process(pid)?;
    let name = process.name().to_string_lossy().trim().to_string();
    if !name.is_empty() {
        return Some(name);
    }
    process
        .exe()
        .and_then(|exe| exe.file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

/// Der Startdialog oeffnet: gibt ihm ein neues, zufaelliges Token. Nachweise vor
/// dem Start tragen es, und nur der Start mit demselben Token (`meetings_start`,
/// `meetings_start_from_event`) sieht sie. Ein frueherer Dialog und sein
/// Nachweis verfallen.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_dialog_open() -> Result<String, String> {
    let token = format!("{:032x}", rand::random::<u128>());
    nc::PENDING.open(token.clone());
    Ok(token)
}

/// Der Startdialog schliesst (Abbrechen, Schliessen oder nach dem Start): der
/// Nachweis dieses Dialogs wird verworfen. Ein spaeter geoeffneter Dialog
/// bleibt unberuehrt.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_dialog_close(dialog_token: String) -> Result<(), String> {
    nc::PENDING.discard(&dialog_token);
    Ok(())
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

/// Platzhalter in `text`, die es nicht gibt (so geschrieben, wie sie dastehen).
/// Die Einstellungen zeigen sie beim Bearbeiten und Speichern des Bausteins an;
/// beim Einsetzen bleiben sie unveraendert im Text stehen.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_check_template(text: String) -> Vec<String> {
    nc::unknown_placeholders(&text)
}

/// Nachweise der Besprechung `meeting_id`, ohne sie die geltenden des
/// Startdialogs mit dem Token `dialog_token`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_status(
    app: AppHandle,
    meeting_id: Option<String>,
    dialog_token: Option<String>,
) -> Result<NoticeStatus, String> {
    let records = match meeting_id {
        Some(id) => {
            let store = require_managed(
                app.try_state::<Arc<MeetingStore>>()
                    .map(|state| Arc::clone(&state)),
            )?;
            store_notice::records_of(&store, &id)
        }
        None => nc::PENDING.fresh(dialog_token.as_deref(), now()),
    };
    Ok(NoticeStatus {
        records,
        paste_available: cfg!(target_os = "windows"),
    })
}

/// Legt den Hinweistext in die Zwischenablage und haelt das als Nachweis fest.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_copy(
    app: AppHandle,
    meeting_id: Option<String>,
    dialog_token: Option<String>,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<NoticeOutcome, String> {
    let target = nc::resolve_target(&nc::PENDING, meeting_id.as_deref(), dialog_token.as_deref())?;
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
        target: None,
    };
    store_record(&app, target, &record)?;
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
    dialog_token: Option<String>,
    notice_app: NoticeApp,
    lang: NoticeLang,
) -> Result<NoticeOutcome, String> {
    let target = nc::resolve_target(&nc::PENDING, meeting_id.as_deref(), dialog_token.as_deref())?;
    let s = enabled_settings(&app)?;
    let record = NoticeRecord {
        at: now(),
        way: NoticeWay::Oral,
        app: notice_app,
        lang,
        text: String::new(),
        in_export: s.export_line,
        target: None,
    };
    store_record(&app, target, &record)?;
    Ok(NoticeOutcome {
        record,
        fallback: None,
    })
}

/// Was nach Frist, Zielpruefung und Einfuegeversuch feststeht.
struct PasteDone {
    way: NoticeWay,
    fallback: Option<NoticeFallback>,
    /// Nur nach einem ausgeloesten Einfuegen: Name der App.
    target: Option<String>,
    /// `export_line` zum Zeitpunkt des Einfuegens (nicht des Befehlsaufrufs).
    export_line: bool,
}

/// Der blockierende Teil von `meeting_notice_paste`: Frist, Zielpruefung, ein
/// Einfuegeversuch.
///
/// Zwischenablage: ist das Ziel keine Besprechungs-App (oder fehlt es, oder
/// scheitert eine Sicherung), bleibt der Hinweistext ABSICHTLICH in der
/// Zwischenablage, damit der Nutzer selbst einfuegen kann; die Meldung der
/// Oberflaeche sagt das ("Der Text liegt in der Zwischenablage."). Den alten
/// Inhalt der Zwischenablage stellt nur ein gelungenes Einfuegen wieder her
/// (`clipboard::paste_notice_guarded`).
fn run_paste(app: &AppHandle, text: &str, secs: u32) -> Result<PasteDone, String> {
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
    // Waehrend der Frist kann die Funktion ausgeschaltet worden sein: dann wird
    // nichts mehr eingefuegt.
    let s = enabled_settings(app)?;
    // EIN Fenster wird erfasst, geprueft (fremd, Besprechungs-App oder Browser) und
    // kommt als Ziel in die Sicherung: `paste_notice_guarded` vergleicht es direkt
    // vor dem Tastendruck erneut mit dem Vordergrund. Ein zweites Erfassen koennte ein
    // anderes Fenster liefern als das gepruefte.
    let foreground = paste_guard::capture_paste_target();
    let app_name = match nc::paste_target_decision(
        foreground.map(|t| t.pid),
        std::process::id(),
        process_name,
    ) {
        Ok(name) => name,
        Err(reason) => {
            if !clipboard::copy_text_verified(app, text) {
                return Err("clipboard_failed".to_string());
            }
            return Ok(PasteDone {
                way: NoticeWay::Clipboard,
                fallback: Some(reason),
                target: None,
                export_line: s.export_line,
            });
        }
    };
    match clipboard::paste_notice_guarded(text, app.clone(), foreground) {
        GuardedPasteOutcome::Pasted => Ok(PasteDone {
            way: NoticeWay::Paste,
            fallback: None,
            target: Some(app_name),
            export_line: s.export_line,
        }),
        GuardedPasteOutcome::NothingToDo => Err("notice_cancelled".to_string()),
        GuardedPasteOutcome::Fallback(reason) => Ok(PasteDone {
            way: NoticeWay::Clipboard,
            fallback: Some(map_fallback(reason)?),
            target: None,
            export_line: s.export_line,
        }),
    }
}

/// Loest nach einer Frist Ctrl+V (Cmd+V) in dem Fenster aus, das dann im
/// Vordergrund steht (die Frist nutzt der Nutzer, um in das Chatfeld der
/// Besprechung zu klicken). Gesendet wird nie, und ob der Text im Chatfeld
/// ankam, kann die App nicht sehen: der Nachweis lautet "Einfuegen ausgeloest in
/// <App>". Steht dann kein fremdes Fenster im Vordergrund, ist es keine
/// Besprechungs-App und kein Browser, oder scheitert eine Sicherung, wird nichts
/// eingefuegt; der Text bleibt in der Zwischenablage (`fallback`), und der
/// Nachweis lautet "kopiert". Der Nachweis traegt den Text, der vor der Frist
/// berechnet wurde, also den, der tatsaechlich eingefuegt wurde.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_paste(
    app: AppHandle,
    meeting_id: Option<String>,
    dialog_token: Option<String>,
    notice_app: NoticeApp,
    lang: NoticeLang,
    countdown_secs: u32,
) -> Result<NoticeOutcome, String> {
    let target = nc::resolve_target(&nc::PENDING, meeting_id.as_deref(), dialog_token.as_deref())?;
    let (_, text) = text_now(&app, notice_app, lang)?;
    let _running = RunGuard::enter()?;
    CANCEL.store(false, Ordering::Release);
    let secs = nc::clamp_countdown(countdown_secs);
    let paste_app = app.clone();
    let paste_text = text.clone();
    let done = tauri::async_runtime::spawn_blocking(move || run_paste(&paste_app, &paste_text, secs))
        .await
        .map_err(|e| format!("meeting_notice_paste panicked: {e}"))??;
    let record = NoticeRecord {
        at: now(),
        way: done.way,
        app: notice_app,
        lang,
        text,
        in_export: done.export_line,
        target: done.target,
    };
    store_record(&app, target, &record)?;
    Ok(NoticeOutcome {
        record,
        fallback: done.fallback,
    })
}

/// Bricht die laufende Frist des Einfuegens ab (die Oberflaeche ruft das auch,
/// wenn der Dialog waehrend der Frist schliesst). Ohne laufende Frist ohne
/// Wirkung: ein spaeter Abbruch trifft nicht das naechste Einfuegen.
#[tauri::command]
#[specta::specta]
pub async fn meeting_notice_cancel_paste() -> Result<(), String> {
    if RUNNING.load(Ordering::Acquire) {
        CANCEL.store(true, Ordering::Release);
    }
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

    /// Befund 2: das Ziel wird VOR dem Tastendruck geprueft und als Ziel der
    /// Sicherung weitergegeben (ein Fenster, nicht zwei Erfassungen).
    #[test]
    fn the_paste_checks_the_target_program_before_it_hands_over_to_the_guard() {
        let source = include_str!("meeting_notice.rs").replace("\r\n", "\n");
        let body = source
            .split("fn run_paste(")
            .nth(1)
            .and_then(|rest| rest.split("/// Loest nach einer Frist").next())
            .expect("run_paste gefunden");
        let capture = body.find("capture_paste_target()").expect("Erfassen");
        let decision = body.find("paste_target_decision(").expect("Pruefung");
        let paste = body.find("paste_notice_guarded(").expect("Einfuegen");
        assert!(capture < decision && decision < paste);
        assert_eq!(body.matches("capture_paste_target()").count(), 1);
        assert!(body.contains("foreground)"), "dasselbe Fenster als Ziel");
    }
}
