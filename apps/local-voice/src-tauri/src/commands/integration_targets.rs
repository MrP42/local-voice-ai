//! Kommandos der Ziele und Konten von A6: SMTP-Postfach, Ordner („ablegen in“),
//! Obsidian-Vault, WAI-Wissensbasis (Goal „Integrationen“, Issue #66).
//!
//! Die Logik steht in `crate::managers::integrations::{targets, smtp, folder,
//! obsidian, wissen}`; hier nur Argumente, Geheimnisse (`secrets`) und die Abbildung
//! auf Fehlertexte. Jedes Kommando ist ein Schritt des Nutzers in der Oberflaeche
//! (`Caller::User`): es braucht keine Freigabe, steht aber im Protokoll. Agenten und
//! Workflows rufen dieselben Funktionen mit ihrem Aufrufer durch das Tor
//! (`targets::*`), nie ueber diese Kommandos.
//!
//! Netzwerk und Dateisystem laufen auf Arbeitsthreads (`spawn_blocking`), nie auf dem
//! Thread der Oberflaeche. Fehler: ein Code (`folder_*`, `vault_*`, `smtp_*`,
//! `wissen_*`) oder Klartext; die Oberflaeche uebersetzt, was sie kennt.

use std::sync::Arc;

use tauri::State;

use crate::managers::integrations::folder::PlacedFile;
use crate::managers::integrations::gate::GateOutcome;
use crate::managers::integrations::model::{
    Caller, Direction, Integration, IntegrationError, Kind,
};
use crate::managers::integrations::obsidian::{self, NoteInput, SaveKind};
use crate::managers::integrations::secrets;
use crate::managers::integrations::smtp::{ConnectOpts, MailMessage};
use crate::managers::integrations::targets::{self, ExportPlacement, TargetSettings};
use crate::managers::integrations::view::{self, IntegrationView};
use crate::managers::integrations::wissen::{HttpOpts, WissenHit};
use crate::managers::meetings::export::{
    build_bundle, bundle_to_markdown, write_export, ExportFormat, ExportParts,
};
use crate::managers::meetings::store::MeetingStore;

fn conn(store: &MeetingStore) -> Result<rusqlite::Connection, String> {
    store.get_connection().map_err(|e| e.to_string())
}

fn secret_state(i: &Integration, slot: &str) -> String {
    secrets::status_label(&secrets::status(i, slot)).to_string()
}

fn put_secret(i: &Integration, slot: &str, value: &str) -> Result<(), String> {
    secrets::put(i, slot, value.as_bytes())
}

fn lookup_secret(
    i: &Integration,
    slot: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    secrets::get_text(i, slot)
}

fn gate_err(e: IntegrationError) -> String {
    e.to_string()
}

/// Ergebnis des Tors fuer den Nutzer: Erfolg, Fehlertext oder Ablehnungsgrund.
fn user_result<T>(out: GateOutcome<T>) -> Result<T, String> {
    match out {
        GateOutcome::Done(v) => Ok(v),
        GateOutcome::Failed(text) => Err(text),
        GateOutcome::Denied { reason, .. } => Err(reason),
        GateOutcome::Pending { .. } => Err("approval_required".to_string()),
    }
}

/// Aendert Einstellungen (und optional das Geheimnis: Passwort bzw. Schluessel) einer
/// SMTP-, Ordner-, Vault- oder Wissens-Integration. Nicht gesendete Felder bleiben,
/// ein leeres `secret` laesst das Geheimnis unveraendert.
#[tauri::command]
#[specta::specta]
pub async fn integration_update_settings(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    settings: TargetSettings,
) -> Result<IntegrationView, String> {
    let conn = conn(&store)?;
    let now = view::now_ms();
    targets::update_settings(&conn, &id, &settings, &put_secret, now)?;
    view::get_view(&conn, &id, &secret_state, now).map_err(gate_err)
}

/// Legt eine Integration mit Einstellungen an (SMTP, Ordner, Vault, Wissensbasis); das
/// Geheimnis geht in den Geheimnisspeicher, nie in die Datenbank. Fehler: Codes
/// (`folder_*`, `vault_path_*`) oder Klartext der Pruefung.
#[tauri::command]
#[specta::specta]
pub async fn integration_create_with_settings(
    store: State<'_, Arc<MeetingStore>>,
    kind: Kind,
    label: String,
    direction: Option<Direction>,
    settings: TargetSettings,
) -> Result<IntegrationView, String> {
    let conn = conn(&store)?;
    let now = view::now_ms();
    let created =
        targets::create_with_secret(&conn, kind, &label, direction, &settings, &put_secret, now)?;
    view::get_view(&conn, &created.id, &secret_state, now).map_err(gate_err)
}

/// Sendet eine Testmail vom SMTP-Postfach an dessen eigene Absenderadresse.
#[tauri::command]
#[specta::specta]
pub async fn integration_send_test_mail(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
) -> Result<(), String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn(&store)?;
        let i = crate::managers::integrations::store::get(&conn, &id)
            .map_err(gate_err)?
            .ok_or_else(|| IntegrationError::NotFound(id.clone()).to_string())?;
        let cfg = crate::managers::integrations::smtp::SmtpConfig::from_config_json(&i.config_json)
            .map_err(|e| e.to_string())?;
        let msg = MailMessage {
            to: vec![cfg.from_address.clone()],
            cc: vec![],
            subject: "Testmail von Local Voice AI".to_string(),
            body_text: "Diese Nachricht bestätigt, dass Local Voice AI über dieses Postfach senden kann.\n\nSie wurde auf deinen Wunsch in den Einstellungen unter „Integrationen“ gesendet.".to_string(),
            body_html: None,
        };
        let out = targets::send_mail(
            &conn,
            Caller::User,
            &id,
            &msg,
            None,
            &lookup_secret,
            &ConnectOpts::default(),
            view::now_ms(),
        )
        .map_err(gate_err)?;
        user_result(out).map(|_| ())
    })
    .await
    .map_err(|e| format!("integration_send_test_mail panicked: {e}"))?
}

/// Eine Antwort der Suche fuer die Oberflaeche.
#[tauri::command]
#[specta::specta]
pub async fn wissen_suchen(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    query: String,
    limit: Option<u32>,
    area: Option<String>,
) -> Result<Vec<WissenHit>, String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn(&store)?;
        let out = targets::search_wissen(
            &conn,
            Caller::User,
            &id,
            &query,
            limit,
            area.as_deref(),
            None,
            &lookup_secret,
            &HttpOpts::default(),
            view::now_ms(),
        )
        .map_err(gate_err)?;
        user_result(out)
    })
    .await
    .map_err(|e| format!("wissen_suchen panicked: {e}"))?
}

/// Wohin der Export kam (Pfad relativ zum Ordner).
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct PlacedInfo {
    pub rel: String,
    pub bytes: f64,
}

impl From<PlacedFile> for PlacedInfo {
    fn from(p: PlacedFile) -> Self {
        Self {
            rel: p.rel,
            bytes: p.bytes as f64,
        }
    }
}

/// Dateiname eines Exports: Titel ohne Windows-Sonderzeichen plus Endung.
fn export_file_name(title: &str, ext: &str) -> String {
    let base = title.trim();
    let base = if base.is_empty() { "besprechung" } else { base };
    format!("{base}.{ext}")
}

/// Legt den Export einer Besprechung in einer Ordner-Integration ab („ablegen in“).
/// Format: `md`, `txt`, `docx`, `html`, `pdf` (die vorhandenen Exporte); Audio nie.
/// Eine vorhandene Datei wird nie ueberschrieben (`Name (2).md`).
#[tauri::command]
#[specta::specta]
pub async fn integration_export_to_folder(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    meeting_id: String,
    format: String,
    parts: ExportParts,
) -> Result<PlacedInfo, String> {
    let fmt = ExportFormat::from_extension(&format)
        .filter(|f| {
            matches!(
                f,
                ExportFormat::Markdown
                    | ExportFormat::PlainText
                    | ExportFormat::Docx
                    | ExportFormat::Html
                    | ExportFormat::Pdf
            )
        })
        .ok_or_else(|| "export_format_unsupported".to_string())?;
    let ext = format.trim().trim_start_matches('.').to_lowercase();
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn(&store)?;
        let bundle = build_bundle(&store, &meeting_id)?;
        let file_name = export_file_name(&bundle.meeting.title, &ext);
        let what = ExportPlacement {
            meeting_id: &meeting_id,
            file_name: &file_name,
            format: &ext,
        };
        let out = targets::place_export(
            &conn,
            Caller::User,
            &id,
            &what,
            None,
            view::now_ms(),
            |path| write_export(path, fmt, &bundle, &parts),
        )
        .map_err(gate_err)?;
        user_result(out).map(PlacedInfo::from)
    })
    .await
    .map_err(|e| format!("integration_export_to_folder panicked: {e}"))?
}

/// Ergebnis von „in Obsidian ablegen“.
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct VaultSaveInfo {
    /// Pfad der Notiz relativ zum Vault.
    pub rel: String,
    /// `created`, `updated` oder `unchanged`.
    pub result: String,
}

/// Schreibt die Besprechung als Notiz mit Frontmatter in den Vault (oder
/// aktualisiert die vorhandene, ohne Dublette). `parts` waehlt die Teile.
#[tauri::command]
#[specta::specta]
pub async fn integration_save_to_vault(
    store: State<'_, Arc<MeetingStore>>,
    id: String,
    meeting_id: String,
    parts: ExportParts,
) -> Result<VaultSaveInfo, String> {
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = conn(&store)?;
        let bundle = build_bundle(&store, &meeting_id)?;
        let is_video = bundle.meeting.source == "youtube";
        let source_url = if is_video {
            crate::managers::youtube::source::read_source(&store, &meeting_id)
                .ok()
                .flatten()
                .map(|s| s.url)
        } else {
            None
        };
        let date_iso = bundle
            .date_label
            .get(..10)
            .filter(|d| {
                d.len() == 10
                    && d.chars().enumerate().all(|(i, c)| {
                        if i == 4 || i == 7 {
                            c == '-'
                        } else {
                            c.is_ascii_digit()
                        }
                    })
            })
            .map(str::to_string)
            .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
        let note = NoteInput {
            meeting_id: meeting_id.clone(),
            title: bundle.meeting.title.clone(),
            date_label: bundle.date_label.clone(),
            date_iso,
            source: if is_video { "youtube" } else { "besprechung" }.to_string(),
            source_url,
            body_md: bundle_to_markdown(&bundle, &parts),
        };
        let out = targets::save_note(&conn, Caller::User, &id, &note, None, view::now_ms())
            .map_err(gate_err)?;
        let saved = user_result(out)?;
        Ok(VaultSaveInfo {
            rel: obsidian::display_rel(&saved.rel),
            result: match saved.kind {
                SaveKind::Created => "created",
                SaveKind::Updated => "updated",
                SaveKind::Unchanged => "unchanged",
            }
            .to_string(),
        })
    })
    .await
    .map_err(|e| format!("integration_save_to_vault panicked: {e}"))?
}
