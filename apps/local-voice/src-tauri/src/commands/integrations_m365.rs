//! Kommandos des Microsoft-365-Kontos (A5, Goal „Integrationen“, Issue #66).
//!
//! Die Logik steht in `crate::managers::integrations::m365`; hier nur Argumente,
//! der Systembrowser (Anmelden) und die Fehlerabbildung. Jedes Kommando ist ein
//! Schritt des Nutzers in der Oberflaeche; Aktionen mit Aussenwirkung (Mail, Datei,
//! Notiz) laufen durch das Tor (`m365::actions`) mit dem Aufrufer `user` und stehen
//! im Audit. Fehler gehen als `code` oder `code|detail` an die Oberflaeche
//! (`integrations.m365.errors.<code>`), nie als Token oder Adresse.
//!
//! Ein echter Anmelde-Ablauf oeffnet den Systembrowser (`opener`); Tests der
//! Oberflaeche ersetzen das Kommando durch eine Attrappe, die Rust-Tests laufen
//! gegen einen lokalen Server (`m365::tests`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use tauri::{AppHandle, State};

use crate::managers::integrations::gate::GateOutcome;
use crate::managers::integrations::m365::actions;
use crate::managers::integrations::m365::config::DEFAULT_FILES_FOLDER;
use crate::managers::integrations::m365::drive::{Conflict, UploadRequest, UploadSource};
use crate::managers::integrations::m365::event::{EventRef, NoteOutcome};
use crate::managers::integrations::m365::mail::{message_from_draft, MailBody, MailMessage};
use crate::managers::integrations::m365::{
    status::status_of, Acct, FilesMode, M365Config, M365Error, M365Service, M365Status,
};
use crate::managers::integrations::model::{
    Caller, Capability, Integration, IntegrationPatch, Kind, NewIntegration,
};
use crate::managers::integrations::view::{self, IntegrationView};
use crate::managers::integrations::{secrets, store};
use crate::managers::meetings::mail::MailDraft;
use crate::managers::meetings::store::MeetingStore;

type Service<'a> = State<'a, Arc<M365Service>>;
type Meetings<'a> = State<'a, Arc<MeetingStore>>;

fn conn(store: &MeetingStore) -> Result<rusqlite::Connection, String> {
    store.get_connection().map_err(|e| e.to_string())
}

fn secret_state(i: &Integration, slot: &str) -> String {
    secrets::status_label(&secrets::status(i, slot)).to_string()
}

fn load(conn: &rusqlite::Connection, id: &str) -> Result<Integration, String> {
    let i = store::get(conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Integration nicht gefunden: {id}"))?;
    if i.kind != Kind::M365 {
        return Err(M365Error::Invalid("Das ist kein Microsoft-365-Konto.".into()).wire());
    }
    Ok(i)
}

fn status(svc: &M365Service, i: &Integration) -> Result<M365Status, String> {
    status_of(i, &svc.vault, svc.signing_in()).map_err(|e| e.wire())
}

/// Ergebnis einer Aktion fuer die Oberflaeche: `ok`, sonst ein Fehlercode
/// (`m365_*` oder `m365_gate|<grund>`), dazu ein Hinweis (z. B. der Ablageort).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct M365ActionResult {
    pub ok: bool,
    pub code: String,
    pub detail: Option<String>,
}

impl M365ActionResult {
    fn ok(detail: Option<String>) -> Self {
        Self {
            ok: true,
            code: "ok".to_string(),
            detail,
        }
    }

    fn failed(code: String) -> Self {
        Self {
            ok: false,
            code,
            detail: None,
        }
    }
}

/// Uebersetzt den Ausgang des Tors in ein Ergebnis fuer die Oberflaeche.
fn result_of<T>(
    out: Result<GateOutcome<T>, crate::managers::integrations::model::IntegrationError>,
    on_done: impl FnOnce(T) -> Option<String>,
) -> Result<M365ActionResult, String> {
    match out.map_err(|e| e.to_string())? {
        GateOutcome::Done(v) => Ok(M365ActionResult::ok(on_done(v))),
        GateOutcome::Failed(wire) => Ok(M365ActionResult::failed(wire)),
        GateOutcome::Denied { code, .. } => {
            Ok(M365ActionResult::failed(format!("m365_gate|{code}")))
        }
        GateOutcome::Pending { .. } => Ok(M365ActionResult::failed("m365_gate|pending".into())),
    }
}

/// Zustand des Kontos: Einstellungen, Scopes und was noch fehlt.
#[tauri::command]
#[specta::specta]
pub async fn m365_status(
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
) -> Result<M365Status, String> {
    let conn = conn(&store)?;
    let i = load(&conn, &id)?;
    status(&svc, &i)
}

/// Legt ein Microsoft-365-Konto an (ohne Anmeldung). Leere Client-ID und leeres
/// Verzeichnis uebernehmen die Einstellungen des Kalenders (E14), wenn dort etwas
/// steht; sonst bleibt das Konto „nicht eingerichtet“, bis eine Client-ID folgt.
/// Fehler: `m365_invalid|<Text>` oder der Klartext des Registers.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn m365_create(
    app: AppHandle,
    store: Meetings<'_>,
    label: String,
    client_id: Option<String>,
    tenant: Option<String>,
    capabilities: Vec<Capability>,
    files_mode: Option<FilesMode>,
    files_folder: Option<String>,
) -> Result<IntegrationView, String> {
    let settings = crate::settings::get_settings(&app);
    let pick = |own: Option<String>, fallback: Option<String>| {
        own.filter(|v| !v.trim().is_empty())
            .or(fallback)
            .unwrap_or_default()
    };
    let cfg = M365Config::new(
        &pick(client_id, settings.calendar_graph_client_id.clone()),
        &pick(tenant, settings.calendar_graph_tenant.clone()),
        &capabilities,
        files_mode.unwrap_or(FilesMode::Full),
        &files_folder.unwrap_or_else(|| DEFAULT_FILES_FOLDER.to_string()),
    )
    .map_err(|e| e.wire())?;
    let conn = conn(&store)?;
    let now = view::now_ms();
    let mut n = NewIntegration::new(Kind::M365, &label);
    n.config = cfg.to_json();
    n.account_hint = Some("graph.microsoft.com".to_string());
    let created = store::create(&conn, &n, now).map_err(|e| e.to_string())?;
    actions::audit_user_event(
        &conn,
        &created.id,
        json!({
            "phase": "created",
            "kind": "m365",
            "capabilities": cfg.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
        }),
    );
    view::get_view(&conn, &created.id, &secret_state, now).map_err(|e| e.to_string())
}

/// Aendert Client-ID, Verzeichnis, eingeschaltete Faehigkeiten oder Ablageort
/// (`None` = unveraendert). Eine andere Client-ID oder ein anderes Verzeichnis
/// verwirft das Token (neu anmelden); eine zusaetzliche Faehigkeit verlangt beim
/// naechsten Anmelden die Zustimmung zu ihrem Scope (`needs_consent`).
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn m365_update_settings(
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
    client_id: Option<String>,
    tenant: Option<String>,
    capabilities: Option<Vec<Capability>>,
    files_mode: Option<FilesMode>,
    files_folder: Option<String>,
) -> Result<M365Status, String> {
    let conn = conn(&store)?;
    let i = load(&conn, &id)?;
    let old = M365Config::from_json(&i.config_json).map_err(|e| e.wire())?;
    let cfg = M365Config::new(
        client_id.as_deref().unwrap_or(&old.client_id),
        tenant.as_deref().unwrap_or(&old.tenant),
        capabilities.as_deref().unwrap_or(&old.capabilities),
        files_mode.unwrap_or(old.files_mode),
        files_folder.as_deref().unwrap_or(&old.files_folder),
    )
    .map_err(|e| e.wire())?;
    let now = view::now_ms();
    let patch = IntegrationPatch {
        config: Some(cfg.to_json()),
        ..Default::default()
    };
    let updated = store::update(&conn, &id, &patch, now).map_err(|e| e.to_string())?;
    if cfg.client_id != old.client_id || cfg.tenant != old.tenant {
        svc.sign_out(&updated);
    }
    actions::audit_user_event(
        &conn,
        &id,
        json!({
            "phase": "updated",
            "capabilities": cfg.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "files_mode": cfg.files_mode.as_str(),
            "client_changed": cfg.client_id != old.client_id,
        }),
    );
    status(&svc, &updated)
}

/// Anmelden: oeffnet den Systembrowser (Microsoft-Anmeldung mit PKCE, Umleitung auf
/// einen Listener auf `127.0.0.1`), wartet bis zu 5 Minuten und legt das Konto
/// verschluesselt ab. Es werden nur die Scopes der eingeschalteten Faehigkeiten
/// angefragt. Fehler: `m365_*` (siehe `M365Error::code`).
#[tauri::command]
#[specta::specta]
pub async fn m365_sign_in(
    app: AppHandle,
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
) -> Result<M365Status, String> {
    use tauri_plugin_opener::OpenerExt;
    let (acct, store) = {
        let c = conn(&store)?;
        (
            Acct::from_integration(load(&c, &id)?).map_err(|e| e.wire())?,
            Arc::clone(&store),
        )
    };
    let opener = app.clone();
    let signed = svc
        .sign_in(
            &acct,
            move |url| {
                opener
                    .opener()
                    .open_url(url, None::<&str>)
                    .map_err(|e| e.to_string())
            },
            crate::managers::integrations::m365::service::SIGN_IN_TIMEOUT,
        )
        .await;
    let conn = conn(&store)?;
    let now = view::now_ms();
    match signed {
        Ok(_) => {
            let _ = store::mark_ok(&conn, &id, now);
            actions::audit_user_event(&conn, &id, json!({ "phase": "signed_in" }));
            status(&svc, &acct.integ)
        }
        Err(e) => {
            let _ = store::mark_error(&conn, &id, &e.to_string(), now);
            Err(e.wire())
        }
    }
}

/// Bricht eine laufende Anmeldung ab; `false`, wenn keine laeuft.
#[tauri::command]
#[specta::specta]
pub async fn m365_cancel_sign_in(svc: Service<'_>) -> Result<bool, String> {
    Ok(svc.cancel_sign_in())
}

/// Abmelden: das Token wird geloescht, die Integration bleibt.
#[tauri::command]
#[specta::specta]
pub async fn m365_sign_out(
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
) -> Result<M365Status, String> {
    let conn = conn(&store)?;
    let i = load(&conn, &id)?;
    svc.sign_out(&i);
    actions::audit_user_event(&conn, &id, json!({ "phase": "signed_out" }));
    status(&svc, &i)
}

/// Verbindung testen: fragt das eigene Profil ab (`GET /me`). Das Ergebnis steht
/// auch am Eintrag (`last_ok_at`/`last_error`).
#[tauri::command]
#[specta::specta]
pub async fn m365_test(
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
) -> Result<M365ActionResult, String> {
    let (acct, store) = {
        let c = conn(&store)?;
        (
            Acct::from_integration(load(&c, &id)?).map_err(|e| e.wire())?,
            Arc::clone(&store),
        )
    };
    let result = svc.me(&acct).await;
    let conn = conn(&store)?;
    let now = view::now_ms();
    match result {
        Ok(me) => {
            let _ = store::mark_ok(&conn, &id, now);
            Ok(M365ActionResult::ok(Some(me.address)))
        }
        Err(e) => {
            let _ = store::mark_error(&conn, &id, &e.to_string(), now);
            Ok(M365ActionResult::failed(e.wire()))
        }
    }
}

/// Testmail an die eigene Adresse des Kontos (Owner-Pruefung: kommt die Mail an?).
#[tauri::command]
#[specta::specta]
pub async fn m365_send_test_mail(
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
) -> Result<M365ActionResult, String> {
    let own = {
        let c = conn(&store)?;
        let i = load(&c, &id)?;
        svc.vault
            .load(&i)
            .ok()
            .flatten()
            .and_then(|a| a.address.clone())
    };
    let Some(own) = own else {
        return Ok(M365ActionResult::failed(M365Error::NeedsSignIn.wire()));
    };
    let msg = match MailMessage::new(
        &[own],
        &[],
        "Testmail von Local Voice AI",
        MailBody::Text(
            "Diese Mail kommt von Local Voice AI und prüft, dass das Senden über Microsoft 365 funktioniert.\n"
                .to_string(),
        ),
    ) {
        Ok(m) => m,
        Err(e) => return Ok(M365ActionResult::failed(e.wire())),
    };
    let out = actions::send_mail(
        Arc::clone(&svc),
        Arc::clone(&store),
        Caller::User,
        &id,
        msg,
        None,
    )
    .await;
    result_of(out, |()| None)
}

/// Kleine Testdatei in den eingestellten OneDrive-Ordner legen (Owner-Pruefung).
#[tauri::command]
#[specta::specta]
pub async fn m365_upload_test_file(
    store: Meetings<'_>,
    svc: Service<'_>,
    id: String,
) -> Result<M365ActionResult, String> {
    let now = chrono::Local::now();
    let name = format!("local-voice-test-{}.txt", now.format("%Y%m%d-%H%M%S"));
    let text = format!(
        "Testdatei von Local Voice AI\nZeitpunkt: {}\n",
        now.format("%d.%m.%Y %H:%M:%S")
    );
    let req = UploadRequest {
        name: name.clone(),
        subfolder: String::new(),
        conflict: Conflict::Rename,
        source: UploadSource::Bytes(text.into_bytes()),
    };
    let out = actions::upload_file(
        Arc::clone(&svc),
        Arc::clone(&store),
        Caller::User,
        &id,
        req,
        None,
        None,
    )
    .await;
    result_of(out, |up| Some(up.name))
}

/// Follow-up-Mail einer Besprechung ueber das Microsoft-365-Konto senden („senden
/// über“). Der Entwurf kommt aus dem Dialog; eine unbrauchbare Adresse bricht ab.
#[tauri::command]
#[specta::specta]
pub async fn meeting_followup_send_m365(
    store: Meetings<'_>,
    svc: Service<'_>,
    integration_id: String,
    draft: MailDraft,
) -> Result<M365ActionResult, String> {
    let msg = match message_from_draft(&draft) {
        Ok(m) => m,
        Err(e) => return Ok(M365ActionResult::failed(e.wire())),
    };
    let out = actions::send_mail(
        Arc::clone(&svc),
        Arc::clone(&store),
        Caller::User,
        &integration_id,
        msg,
        None,
    )
    .await;
    result_of(out, |()| None)
}

/// Haengt eine Notiz an den Outlook-Termin einer Kalenderzeile (`event_key` aus dem
/// Kalender-Cache). Ist die Notiz schon im Termin, geschieht nichts (`detail`:
/// `already_there`).
#[tauri::command]
#[specta::specta]
pub async fn m365_event_note(
    store: Meetings<'_>,
    svc: Service<'_>,
    integration_id: String,
    event_key: String,
    note: String,
) -> Result<M365ActionResult, String> {
    let event = store
        .calendar_event(&event_key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "calendar_event_not_found".to_string())?;
    let ev = EventRef {
        uid: event.uid,
        starts_at: event.starts_at,
        ends_at: event.ends_at,
    };
    let out = actions::event_note(
        Arc::clone(&svc),
        Arc::clone(&store),
        Caller::User,
        &integration_id,
        ev,
        note,
        None,
    )
    .await;
    result_of(out, |o| {
        Some(match o {
            NoteOutcome::Added => "added".to_string(),
            NoteOutcome::AlreadyThere => "already_there".to_string(),
        })
    })
}
