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

// ---------------------------------------------------------------------------
// G9a-2 (lva#578, ADR-0031 E2.6–E2.8): Freigabe einer Besprechung fuer KI-Clients
// ---------------------------------------------------------------------------

use std::sync::Arc;

use besprechungsplanung::assistent::{
    eigene_domaenen_bereinigen, eigene_domaenen_vorschlag, EigeneDomaenenBereinigt, Einstufung,
};
use tauri::Manager;

use crate::managers::meetings::ki_freigabe::{
    self, Freigabenachweis, KiFreigabeStand, KiSperre, Richtlinie,
};
use crate::managers::meetings::store::MeetingStore;

fn store_of(app: &tauri::AppHandle) -> Result<Arc<MeetingStore>, String> {
    app.try_state::<Arc<MeetingStore>>()
        .map(|state| Arc::clone(&state))
        .ok_or_else(|| "meetings_unavailable".to_string())
}

/// Richtlinienspiegel vor dem Schreiben der Einstellungen (Fehler an die Oberflaeche).
fn spiegeln(app: &tauri::AppHandle, s: &settings::AppSettings) -> Result<(), String> {
    store_of(app)?
        .ki_richtlinie_setzen(&Richtlinie::aus_einstellungen(s))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn sperre_of(app: &tauri::AppHandle) -> KiSperre {
    KiSperre::aus_einstellungen(&settings::get_settings(app))
}

/// Stand der Freigabe einer Besprechung fuer KI-Clients (Einstufung, Bestaetigung, Entscheidung).
#[tauri::command]
#[specta::specta]
pub fn meeting_ai_release_status(
    app: tauri::AppHandle,
    meeting_id: String,
) -> Result<KiFreigabeStand, String> {
    let store = store_of(&app)?;
    ki_freigabe::stand(&store, &sperre_of(&app), &meeting_id).map_err(|e| e.to_string())
}

/// Bestaetigt die Einstufung fuer GENAU diese Besprechung und diese Teilnehmerliste. Fehler
/// `ki_freigabe_veraltet` (Liste oder Domaenen haben sich geaendert, neu laden),
/// `ki_freigabe_nicht_intern` (die Liste ist nicht intern), `ki_freigabe_ohne_information`,
/// `ki_freigabe_ohne_rechtsgrundlage`, `ki_freigabe_ohne_einwilligung` (Nachweis nach ADR-0031
/// E2 Nr. 7 unvollstaendig) oder `ki_freigabe_richtlinie_fehlt`. Wird sofort gespeichert.
#[tauri::command]
#[specta::specta]
pub fn meeting_ai_release_confirm(
    app: tauri::AppHandle,
    meeting_id: String,
    einstufung: Einstufung,
    fingerabdruck: String,
    nachweis: Freigabenachweis,
) -> Result<KiFreigabeStand, String> {
    let store = store_of(&app)?;
    ki_freigabe::bestaetigen(
        &store,
        &sperre_of(&app),
        &meeting_id,
        einstufung,
        &fingerabdruck,
        &nachweis,
        chrono::Utc::now().timestamp_millis(),
    )
    .map_err(|e| e.to_string())
}

/// Nimmt die Freigabe zurueck; KI-Clients sehen die Besprechung ab dem naechsten Abruf nicht mehr.
#[tauri::command]
#[specta::specta]
pub fn meeting_ai_release_revoke(
    app: tauri::AppHandle,
    meeting_id: String,
) -> Result<KiFreigabeStand, String> {
    let store = store_of(&app)?;
    store
        .ki_bestaetigung_entfernen(&meeting_id)
        .map_err(|e| e.to_string())?;
    ki_freigabe::stand(&store, &sperre_of(&app), &meeting_id).map_err(|e| e.to_string())
}

/// „Eigene Domänen“ speichern: nur die wirksamen Eintraege; die abgelehnten gehen zur Anzeige
/// zurueck. Erst der Richtlinienspiegel (eine Schreibtransaktion, die bei geaenderten Domaenen
/// ALLE Bestaetigungen loescht), dann die Einstellungsdatei. Schlaegt der Spiegel fehl, bleibt
/// alles beim Alten (Fehler an die Oberflaeche); dazwischen gibt der MCP-Server nichts aus.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_own_domains_setting(
    app: tauri::AppHandle,
    domains: Vec<String>,
) -> Result<EigeneDomaenenBereinigt, String> {
    let bereinigt = eigene_domaenen_bereinigen(&domains);
    let mut s = settings::get_settings(&app);
    s.meeting_own_domains = bereinigt.gueltig.clone();
    spiegeln(&app, &s)?;
    settings::write_settings(&app, s);
    Ok(bereinigt)
}

/// Vorschlag aus „Meine E-Mail-Adressen“; wird nie ohne Bestaetigung des Nutzers uebernommen.
#[tauri::command]
#[specta::specta]
pub fn meeting_own_domains_suggestion(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    Ok(eigene_domaenen_vorschlag(
        &settings::get_settings(&app).meeting_self_emails,
    ))
}

/// „Cloud-Modelle in Besprechungen mit Externen“ (Standard aus).
#[tauri::command]
#[specta::specta]
pub fn change_meeting_cloud_with_externals_setting(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.meeting_cloud_with_externals = enabled;
    spiegeln(&app, &s)?;
    settings::write_settings(&app, s);
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
