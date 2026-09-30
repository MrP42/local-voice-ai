//! Kommandos der YouTube-Quelle (A2, Goal „Integrationen“): Link pruefen,
//! Quelle anlegen, Quelle lesen, yt-dlp erkennen, zwei Einstellungen.
//! Die Logik steht in `crate::managers::youtube`; hier nur Argumente und Fehler.
//!
//! Fehler gehen als Code an die Oberflaeche (`youtube_*`, bei `youtube_network`
//! und aehnlichen mit Zusatz nach einem Doppelpunkt); die Oberflaeche uebersetzt.

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::store::{Meeting, MeetingStore};
use crate::managers::youtube::source::{self, AddOptions, YoutubeSource};
use crate::managers::youtube::tool::{self, ToolStatus};
use crate::managers::youtube::{normalize_link, YoutubeError};
use crate::settings;

/// Was ein gueltiger Link bezeichnet (fuer die Rueckmeldung im Dialog, ohne Netz).
#[derive(Clone, Debug, Serialize, Type)]
pub struct YoutubeLinkInfo {
    pub video_id: String,
    pub url: String,
    pub start_s: Option<u32>,
}

/// Prueft einen eingefuegten Link ohne Netzzugriff: die bereinigte Adresse oder
/// ein Fehlercode (`youtube_playlist`, `youtube_channel`, `youtube_not_youtube`, ...).
#[tauri::command]
#[specta::specta]
pub fn youtube_normalize_link(url: String) -> Result<YoutubeLinkInfo, String> {
    let video = normalize_link(&url).map_err(|e| YoutubeError::Link(e).to_command_error())?;
    Ok(YoutubeLinkInfo {
        url: video.canonical_url(),
        video_id: video.video_id,
        start_s: video.start_s,
    })
}

/// Legt aus einem Link eine Besprechung mit Quelle YouTube an (ein oEmbed-Abruf
/// fuer Titel und Kanal, im Audit) im gewaehlten Projekt. Der bewusste Nutzerschritt
/// „Link einfuegen“; nichts anderes verbindet sich dabei mit YouTube.
#[tauri::command]
#[specta::specta]
pub async fn youtube_add_source(
    app: tauri::AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    url: String,
    project_id: Option<String>,
) -> Result<Meeting, String> {
    let added = source::add_youtube_source(
        &store,
        &url,
        project_id.as_deref(),
        &AddOptions::production(),
    )
    .await
    .map_err(|e| e.to_command_error())?;
    indexer::submit(&app, IndexJob::Meeting(added.meeting.id.clone()));
    Ok(added.meeting)
}

/// Die YouTube-Angaben einer Besprechung, oder nichts (andere Quelle, keine Daten).
/// Kein Netzzugriff.
#[tauri::command]
#[specta::specta]
pub async fn youtube_source_get(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Option<YoutubeSource>, String> {
    source::read_source(&store, &meeting_id).map_err(|e| e.to_command_error())
}

/// Sucht ein selbst installiertes yt-dlp und zeigt seine Version. `path`: der Wert
/// aus dem Eingabefeld (noch nicht gespeichert); `None` = gespeicherter Pfad.
/// Startet hoechstens `yt-dlp --version` (im Job-Objekt, mit Zeitlimit); kein
/// Netzzugriff.
#[tauri::command]
#[specta::specta]
pub async fn youtube_tool_detect(
    app: tauri::AppHandle,
    path: Option<String>,
) -> Result<ToolStatus, String> {
    let configured = match path {
        Some(p) => Some(p),
        None => settings::get_settings(&app).meeting_youtube_tool_path,
    };
    tauri::async_runtime::spawn_blocking(move || tool::detect(configured.as_deref()))
        .await
        .map_err(|e| format!("youtube_tool_detect panicked: {e}"))
}

/// Schalter „privat/experimentell“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_youtube_private_setting(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_youtube_private = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Pfad eines selbst installierten yt-dlp. Leer = im PATH suchen.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_youtube_tool_path_setting(
    app: tauri::AppHandle,
    path: Option<String>,
) -> Result<(), String> {
    let value = normalize_tool_path(path.as_deref())?;
    let mut settings = settings::get_settings(&app);
    settings.meeting_youtube_tool_path = value;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Laengster gespeicherter Pfad (Zeichen).
const MAX_PATH_CHARS: usize = 1024;

/// Leer -> `None`; Leerraum am Rand weg; Steuerzeichen und Uebergroesse sind Fehler
/// (`youtube_path_invalid`).
fn normalize_tool_path(path: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = path.map(str::trim).filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > MAX_PATH_CHARS || text.chars().any(char::is_control) {
        return Err("youtube_path_invalid".to_string());
    }
    Ok(Some(text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_check_returns_the_clean_address_or_a_code() {
        let ok = youtube_normalize_link("https://youtu.be/dQw4w9WgXcQ?t=1m5s&si=x".into()).unwrap();
        assert_eq!(ok.video_id, "dQw4w9WgXcQ");
        assert_eq!(ok.url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(ok.start_s, Some(65));
        for (raw, code) in [
            (
                "https://www.youtube.com/playlist?list=PLx",
                "youtube_playlist",
            ),
            ("https://vimeo.com/1", "youtube_not_youtube"),
            ("kein link", "youtube_invalid_link"),
            ("", "youtube_empty"),
        ] {
            assert_eq!(
                youtube_normalize_link(raw.into()).unwrap_err(),
                code,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_tool_path_setting_is_trimmed_and_bounded() {
        assert_eq!(normalize_tool_path(None), Ok(None));
        assert_eq!(normalize_tool_path(Some("")), Ok(None));
        assert_eq!(normalize_tool_path(Some("   ")), Ok(None));
        assert_eq!(
            normalize_tool_path(Some("  C:\\Tools\\yt-dlp.exe ")),
            Ok(Some("C:\\Tools\\yt-dlp.exe".to_string()))
        );
        assert_eq!(
            normalize_tool_path(Some("C:\\x\u{0}y")),
            Err("youtube_path_invalid".to_string())
        );
        assert_eq!(
            normalize_tool_path(Some(&"a".repeat(MAX_PATH_CHARS + 1))),
            Err("youtube_path_invalid".to_string())
        );
    }

    #[test]
    fn command_errors_carry_a_stable_code_before_any_detail() {
        assert_eq!(YoutubeError::Busy.to_command_error(), "youtube_busy");
        assert_eq!(
            YoutubeError::Unavailable.to_command_error(),
            "youtube_unavailable"
        );
        assert_eq!(
            YoutubeError::Http(500).to_command_error(),
            "youtube_http: 500"
        );
        assert!(YoutubeError::Network("x".into())
            .to_command_error()
            .starts_with("youtube_network: "));
        for e in [
            YoutubeError::Busy,
            YoutubeError::Project,
            YoutubeError::Timeout,
            YoutubeError::RateLimited,
        ] {
            assert!(!e.to_command_error().contains(':'), "{e:?}");
        }
    }
}
