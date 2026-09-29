//! Einstellungen und Konfig-Angaben des lokalen MCP-Servers (M6-P6e, F21).
//! Der Server selbst steht in `crate::mcp`; hier nur die zwei Schalter der
//! Einstellungszeile und der Pfad der EXE fuer die Kopier-Schnipsel.

use serde::Serialize;
use specta::Type;

use crate::settings;

/// Angaben fuer die Konfig-Schnipsel (`claude mcp add ...`, Claude Desktop, Codex).
#[derive(Clone, Debug, Serialize, Type)]
pub struct McpInfo {
    /// Voller Pfad der laufenden EXE; der Client startet sie mit `--mcp`.
    pub exe_path: String,
}

/// Pfad ohne den `\\?\`-Vorsatz, den manche Clients nicht als Programm finden.
fn plain_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(text)
}

#[tauri::command]
#[specta::specta]
pub fn meeting_mcp_info() -> Result<McpInfo, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(McpInfo {
        exe_path: plain_path(&exe),
    })
}

/// „Lokaler MCP-Server (nur lesend)“ ein- oder ausschalten. Wirkt sofort auch in
/// einer laufenden Sitzung: der Server liest die Einstellung bei jedem Aufruf.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_mcp_enabled_setting(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_mcp_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

/// „Transkript freigeben“ fuer den MCP-Server.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_mcp_include_transcript_setting(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.meeting_mcp_include_transcript = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_verbatim_prefix_is_dropped_from_the_exe_path() {
        assert_eq!(
            plain_path(Path::new(
                r"\\?\C:\Programme\Local Voice AI\local-voice-ai.exe"
            )),
            r"C:\Programme\Local Voice AI\local-voice-ai.exe"
        );
        assert_eq!(plain_path(Path::new(r"C:\x\a.exe")), r"C:\x\a.exe");
    }

    #[test]
    fn the_exe_path_of_the_running_program_is_reported() {
        let info = meeting_mcp_info().unwrap();
        assert!(!info.exe_path.is_empty());
        assert!(Path::new(&info.exe_path).is_absolute());
    }
}
