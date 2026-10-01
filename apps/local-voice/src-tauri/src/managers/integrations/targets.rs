//! Kleber der Ziele und Konten von A6 (SMTP, Ordner, Obsidian, Wissensbasis):
//! Einstellungen der Oberflaeche -> geprueftes `config_json` plus Geheimnis,
//! „Verbindung testen“ und die Aktionen durch das Tor (`gate`).
//!
//! Jede Aktion eines Aufrufers ausser dem Nutzer geht durch `gate::run` bzw.
//! `gate::run_approved` (Rechte, Freigabe, Audit VOR der Ausfuehrung). Der Nutzer
//! in der Oberflaeche braucht keine Freigabe; seine Aktionen schreibt dieses Modul
//! selbst ins Audit (Aufrufer `user`), damit AK11 („alle Aktionen im Audit“) gilt.
//! Im Audit stehen nur Ziel und Mengen, nie Inhalt, Passwort oder Schluessel.
//!
//! Geheimnisse: `SecretLookup` liefert sie aus `secrets` (Produktion) oder aus einem
//! Testordner; sie verlassen dieses Modul nur als Argument an den Adapter.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use zeroize::Zeroizing;

use super::folder::{self, FolderConfig, PlacedFile, Sandbox};
use super::gate::{self, GateOutcome, Request};
use super::model::{
    AuditOutcome, Caller, Capability, Direction, Integration, IntegrationError, Kind, NewAudit,
};
use super::obsidian::{self, NoteInput, ObsidianConfig, ObsidianError, SaveResult};
use super::smtp::{self, ConnectOpts, MailMessage, SendReceipt, SmtpConfig};
use super::view::{self, TestResult};
use super::wissen::{self, HttpOpts, WissenConfig, WissenHit};
use super::{audit, store};

/// Liefert ein Geheimnis (Fach) einer Integration; `Ok(None)`: fehlt.
pub type SecretLookup<'a> =
    &'a dyn Fn(&Integration, &str) -> Result<Option<Zeroizing<String>>, String>;

/// Legt ein Geheimnis ab (Fach, Wert).
pub type SecretPut<'a> = &'a dyn Fn(&Integration, &str, &str) -> Result<(), String>;

/// Das Fach des Geheimnisses je Art.
pub fn secret_slot(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Smtp => Some("password"),
        Kind::Wissen => Some("token"),
        _ => None,
    }
}

/// Einstellungen, wie die Oberflaeche sie schickt. Felder, die eine Art nicht kennt,
/// werden ignoriert. `secret` ist das Passwort (SMTP) oder der Schluessel (Wissen);
/// leer oder fehlend bedeutet beim Aendern „unveraendert“.
#[derive(Clone, Default, Serialize, Deserialize, Type)]
pub struct TargetSettings {
    /// Ordner oder Vault (absoluter Pfad).
    pub path: Option<String>,
    /// Unterordner (Ordner: fuer Exporte; Vault: fuer neue Notizen).
    pub subfolder: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub security: Option<smtp::Security>,
    pub username: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub context_area: Option<String>,
    pub tier: Option<String>,
    pub endpoint: Option<String>,
    pub search_tool: Option<String>,
    pub area: Option<String>,
    pub secret: Option<String>,
}

fn opt(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|v| v.trim().to_string())
}

/// Die Einstellungen als rohes `config_json`-Objekt; fehlende Felder kommen aus
/// `existing` (Aendern) oder aus den Vorgaben der Art.
pub fn config_from_settings(kind: Kind, s: &TargetSettings, existing: Option<&Value>) -> Value {
    let old = |key: &str| existing.and_then(|e| e.get(key)).cloned();
    let text = |new: Option<String>, key: &str| -> Value {
        match new {
            Some(v) => json!(v),
            None => old(key).unwrap_or_else(|| json!("")),
        }
    };
    // Leer heisst „Vorgabe“ (Obsidian, Wissen): nie ein leerer Wert im Register.
    let text_or = |new: Option<String>, key: &str, default: &str| -> Value {
        match new {
            Some(v) if !v.is_empty() => json!(v),
            _ => old(key).unwrap_or_else(|| json!(default)),
        }
    };
    match kind {
        Kind::Folder => json!({
            "path": text(opt(&s.path), "path"),
            "subfolder": text(opt(&s.subfolder), "subfolder"),
        }),
        Kind::Smtp => {
            let security = s
                .security
                .map(|x| json!(x.as_str()))
                .or_else(|| old("security"))
                .unwrap_or_else(|| json!("starttls"));
            let parsed: smtp::Security =
                serde_json::from_value(security.clone()).unwrap_or(smtp::Security::Starttls);
            let port = s
                .port
                .map(|p| json!(p))
                .or_else(|| old("port"))
                .unwrap_or_else(|| json!(parsed.default_port()));
            json!({
                "host": text(opt(&s.host), "host"),
                "port": port,
                "security": security,
                "username": text(opt(&s.username), "username"),
                "from_address": text(opt(&s.from_address), "from_address"),
                "from_name": text(opt(&s.from_name), "from_name"),
            })
        }
        Kind::Obsidian => json!({
            "path": text(opt(&s.path), "path"),
            "subfolder": text_or(opt(&s.subfolder), "subfolder", obsidian::DEFAULT_SUBFOLDER),
            "context_area": text_or(opt(&s.context_area), "context_area", obsidian::DEFAULT_AREA),
            "tier": text_or(opt(&s.tier), "tier", obsidian::DEFAULT_TIER),
        }),
        Kind::Wissen => json!({
            "endpoint": text(opt(&s.endpoint), "endpoint"),
            "search_tool": text_or(opt(&s.search_tool), "search_tool", wissen::DEFAULT_TOOL),
            "area": text(opt(&s.area), "area"),
        }),
        _ => existing.cloned().unwrap_or_else(|| json!({})),
    }
}

/// Prueft ein rohes `config_json` und liefert die bereinigte Fassung samt Anzeige
/// (`account_hint`: Server, nie Adresse oder Schluessel). Fehler: Code oder Klartext.
pub fn normalize_config(kind: Kind, raw: &Value) -> Result<(Value, Option<String>), String> {
    let text = raw.to_string();
    match kind {
        Kind::Folder => {
            let cfg = FolderConfig::from_config_json(&text).map_err(|e| e.code().to_string())?;
            folder::check_root_text(&cfg.path).map_err(|e| e.code().to_string())?;
            folder::check_relative(&cfg.subfolder).map_err(|e| e.code().to_string())?;
            Ok((
                json!({ "path": cfg.path, "subfolder": cfg.subfolder }),
                None,
            ))
        }
        Kind::Smtp => {
            let cfg = SmtpConfig::from_config_json(&text).map_err(|e| e.to_string())?;
            Ok((cfg.to_json(), Some(cfg.host.trim().to_string())))
        }
        Kind::Obsidian => {
            let cfg = ObsidianConfig::from_config_json(&text).map_err(|e| e.to_string())?;
            folder::check_root_text(&cfg.path)
                .map_err(|e| ObsidianError::Folder(e).code().to_string())?;
            folder::check_relative(&cfg.subfolder).map_err(|e| e.code().to_string())?;
            Ok((cfg.to_json(), None))
        }
        Kind::Wissen => {
            let cfg = WissenConfig::from_config_json(&text).map_err(|e| e.to_string())?;
            let host = wissen::parse_endpoint(&cfg.endpoint)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string));
            Ok((cfg.to_json(), host))
        }
        _ => Err(view::ERR_KIND_NOT_AVAILABLE.to_string()),
    }
}

/// Braucht diese Konfiguration ein Geheimnis? SMTP nur mit Benutzername (ohne
/// Anmeldung gibt es kein Passwort), die Wissensbasis immer.
fn needs_secret(kind: Kind, config: &Value) -> bool {
    match kind {
        Kind::Smtp => config
            .get("username")
            .and_then(Value::as_str)
            .is_some_and(|u| !u.is_empty()),
        Kind::Wissen => true,
        _ => false,
    }
}

fn missing_secret_text(kind: Kind) -> String {
    match kind {
        Kind::Smtp => smtp::SmtpError::PasswordMissing.to_string(),
        _ => wissen::WissenError::TokenMissing.to_string(),
    }
}

/// Legt eine Integration an und das Geheimnis dazu. Scheitert das Geheimnis, wird die
/// Integration wieder entfernt (nie eine halbe Einrichtung).
pub fn create_with_secret(
    conn: &Connection,
    kind: Kind,
    label: &str,
    direction: Option<Direction>,
    settings: &TargetSettings,
    put: SecretPut<'_>,
    now_ms: i64,
) -> Result<Integration, String> {
    let secret = settings.secret.as_deref().filter(|s| !s.is_empty());
    let raw = config_from_settings(kind, settings, None);
    if needs_secret(kind, &raw) && secret.is_none() {
        return Err(missing_secret_text(kind));
    }
    let created = view::create_from_ui(conn, kind, label, direction, raw, now_ms)?;
    if let (Some(slot), Some(value)) = (secret_slot(kind), secret) {
        if let Err(e) = put(&created, slot, value) {
            let _ = store::delete(conn, &created.id);
            return Err(e);
        }
    }
    Ok(created)
}

/// Aendert Einstellungen (und optional das Geheimnis) einer Integration. Das alte
/// Geheimnis bleibt, wenn `settings.secret` leer ist.
pub fn update_settings(
    conn: &Connection,
    id: &str,
    settings: &TargetSettings,
    put: SecretPut<'_>,
    now_ms: i64,
) -> Result<Integration, String> {
    let current = store::get(conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| IntegrationError::NotFound(id.to_string()).to_string())?;
    if !view::UI_CREATABLE.contains(&current.kind) {
        return Err(view::ERR_KIND_NOT_AVAILABLE.to_string());
    }
    let existing: Value = serde_json::from_str(&current.config_json).unwrap_or(Value::Null);
    let raw = config_from_settings(current.kind, settings, Some(&existing));
    let (config, hint) = normalize_config(current.kind, &raw)?;
    let secret = settings.secret.as_deref().filter(|s| !s.is_empty());
    if let (Some(slot), Some(value)) = (secret_slot(current.kind), secret) {
        put(&current, slot, value)?;
    }
    let patch = super::model::IntegrationPatch {
        config: Some(config),
        ..Default::default()
    };
    let updated = store::update(conn, id, &patch, now_ms).map_err(|e| e.to_string())?;
    // Anzeige (Server) nachziehen: `store::update` kennt kein `account_hint`.
    if updated.account_hint != hint {
        conn.execute(
            "UPDATE integrations SET account_hint = ?2 WHERE id = ?1",
            rusqlite::params![id, hint],
        )
        .map_err(|e| e.to_string())?;
    }
    view::audit_user(
        conn,
        id,
        None,
        None,
        json!({ "phase": "updated", "changes": { "settings": true, "secret": secret.is_some() } }),
        now_ms,
    );
    store::get(conn, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| IntegrationError::NotFound(id.to_string()).to_string())
}

// ---------------------------------------------------------------------------
// Verbindung testen
// ---------------------------------------------------------------------------

fn result(ok: bool, code: &str, detail: Option<String>) -> TestResult {
    TestResult {
        ok,
        code: code.to_string(),
        detail,
    }
}

/// Probiert die Verbindung einer Integration aus und haelt das Ergebnis am Eintrag
/// fest (`last_ok_at`/`last_error`). Netzwerk und Dateisystem: auf einem
/// Arbeitsthread aufrufen.
pub fn test_target(
    conn: &Connection,
    id: &str,
    secret: SecretLookup<'_>,
    smtp_opts: &ConnectOpts,
    http_opts: &HttpOpts,
    now_ms: i64,
) -> Result<TestResult, IntegrationError> {
    let i = store::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    let lookup = |slot: &str| -> Result<Zeroizing<String>, String> {
        Ok(secret(&i, slot)?.unwrap_or_default())
    };
    let outcome: Result<(), (String, String)> = match i.kind {
        Kind::Folder => return view::test_integration(conn, id, now_ms),
        Kind::Smtp => SmtpConfig::from_config_json(&i.config_json)
            .map_err(|e| (e.code().to_string(), e.to_string()))
            .and_then(|cfg| {
                let pw = lookup("password").map_err(|e| ("smtp_password_broken".to_string(), e))?;
                smtp::test(&cfg, &pw, smtp_opts).map_err(|e| (e.code().to_string(), e.to_string()))
            })
            .map(|_| ()),
        Kind::Obsidian => ObsidianConfig::from_config_json(&i.config_json)
            .and_then(|cfg| obsidian::test(&cfg))
            .map_err(|e| (e.code().to_string(), e.to_string())),
        Kind::Wissen => WissenConfig::from_config_json(&i.config_json)
            .map_err(|e| (e.code().to_string(), e.to_string()))
            .and_then(|cfg| {
                let token = lookup("token").map_err(|e| ("wissen_token_broken".to_string(), e))?;
                wissen::test(&cfg, &token, http_opts)
                    .map_err(|e| (e.code().to_string(), e.to_string()))
            })
            .map(|_| ()),
        _ => return Ok(result(false, "test_not_available", None)),
    };
    let ok_code = match i.kind {
        Kind::Smtp => "smtp_ok",
        Kind::Obsidian => "vault_ok",
        _ => "wissen_ok",
    };
    Ok(match outcome {
        Ok(()) => {
            store::mark_ok(conn, id, now_ms)?;
            result(true, ok_code, None)
        }
        Err((code, text)) => {
            store::mark_error(conn, id, &text, now_ms)?;
            result(false, &code, Some(audit::sanitize_audit_text(&text, 400)))
        }
    })
}

// ---------------------------------------------------------------------------
// Aktionen durch das Tor
// ---------------------------------------------------------------------------

/// Fuehrt `action` durch das Tor aus (mit Freigabe, falls `approval_id`), haelt Erfolg
/// und Fehler am Eintrag fest und schreibt Aktionen des Nutzers ins Audit.
#[allow(clippy::too_many_arguments)]
fn gated<T>(
    conn: &Connection,
    caller: Caller,
    integration_id: &str,
    capability: Capability,
    target: &str,
    args: &Value,
    approval_id: Option<&str>,
    now_ms: i64,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<GateOutcome<T>, IntegrationError> {
    let req = Request {
        caller,
        integration_id,
        capability,
        target: Some(target),
        args: Some(args),
        tool_mode: None,
    };
    let out = match approval_id {
        Some(a) => gate::run_approved(conn, a, &req, now_ms, action)?,
        None => gate::run(conn, &req, now_ms, action)?,
    };
    match &out {
        GateOutcome::Done(_) => {
            let _ = store::mark_ok(conn, integration_id, now_ms);
        }
        GateOutcome::Failed(e) => {
            let _ = store::mark_error(conn, integration_id, e, now_ms);
        }
        _ => {}
    }
    if caller == Caller::User {
        audit_user_outcome(conn, integration_id, capability, target, &out, now_ms);
    }
    Ok(out)
}

/// Aktion des Nutzers ins Audit (das Tor schreibt fuer den Nutzer nichts). Best
/// Effort: scheitert das Audit, geht die Aktion des Nutzers trotzdem durch.
fn audit_user_outcome<T>(
    conn: &Connection,
    integration_id: &str,
    capability: Capability,
    target: &str,
    out: &GateOutcome<T>,
    now_ms: i64,
) {
    let (outcome, detail) = match out {
        GateOutcome::Done(_) => (AuditOutcome::Ok, json!({ "phase": "done" })),
        GateOutcome::Failed(e) => (AuditOutcome::Error, json!({ "phase": "done", "error": e })),
        GateOutcome::Denied { code, .. } => (AuditOutcome::Denied, json!({ "reason": code })),
        GateOutcome::Pending { .. } => return,
    };
    let entry = NewAudit {
        caller: Caller::User.as_str().to_string(),
        integration_id: Some(integration_id.to_string()),
        capability: Some(capability.as_str().to_string()),
        target: Some(target.to_string()),
        outcome,
        detail: Some(detail),
    };
    if let Err(e) = audit::record_at(conn, &entry, now_ms) {
        log::warn!("integrations: Audit der Nutzeraktion nicht geschrieben: {e}");
    }
}

fn load(conn: &Connection, id: &str, kind: Kind) -> Result<Integration, IntegrationError> {
    let i = store::get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))?;
    if i.kind != kind {
        return Err(IntegrationError::Invalid(
            "Diese Aktion gehört zu einer anderen Art von Integration.".to_string(),
        ));
    }
    Ok(i)
}

/// Kurzes Ziel einer Mail fuer Freigabe und Audit: erster Empfaenger und Anzahl.
pub fn mail_target(msg: &MailMessage) -> String {
    let all = msg.recipients();
    match all.as_slice() {
        [] => "(kein Empfänger)".to_string(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} (+{})", rest.len()),
    }
}

/// Mail senden (`mail.send`). Die Argumente der Freigabe enthalten alle Empfaenger
/// vollstaendig und den Text; die Freigabe ist an genau diese Mail gebunden.
#[allow(clippy::too_many_arguments)]
pub fn send_mail(
    conn: &Connection,
    caller: Caller,
    integration_id: &str,
    msg: &MailMessage,
    approval_id: Option<&str>,
    secret: SecretLookup<'_>,
    opts: &ConnectOpts,
    now_ms: i64,
) -> Result<GateOutcome<SendReceipt>, IntegrationError> {
    let i = load(conn, integration_id, Kind::Smtp)?;
    let args = json!({
        "to": msg.to, "cc": msg.cc, "subject": msg.subject, "body": msg.body_text,
        "html": msg.body_html.is_some(),
    });
    gated(
        conn,
        caller,
        integration_id,
        Capability::MailSend,
        &mail_target(msg),
        &args,
        approval_id,
        now_ms,
        || {
            let cfg = SmtpConfig::from_config_json(&i.config_json).map_err(|e| e.to_string())?;
            let password = match secret(&i, "password")? {
                Some(p) => p,
                None if cfg.username.is_empty() => Zeroizing::new(String::new()),
                None => return Err(smtp::SmtpError::PasswordMissing.to_string()),
            };
            smtp::send(&cfg, &password, msg, opts).map_err(|e| e.to_string())
        },
    )
}

/// Wohin ein Export kommt (fuer Freigabe und Audit).
pub struct ExportPlacement<'a> {
    pub meeting_id: &'a str,
    pub file_name: &'a str,
    pub format: &'a str,
}

/// Legt einen Export im Ordner ab (`files.write`). `writer` schreibt die Datei an den
/// ihm uebergebenen Pfad (die vorhandenen Exporte: Markdown, Word, PDF ...).
pub fn place_export(
    conn: &Connection,
    caller: Caller,
    integration_id: &str,
    what: &ExportPlacement<'_>,
    approval_id: Option<&str>,
    now_ms: i64,
    writer: impl FnOnce(&std::path::Path) -> Result<(), String>,
) -> Result<GateOutcome<PlacedFile>, IntegrationError> {
    let i = load(conn, integration_id, Kind::Folder)?;
    let cfg = FolderConfig::from_config_json(&i.config_json).ok();
    let name = folder::sanitize_file_name(what.file_name).unwrap_or_default();
    let sub = cfg
        .as_ref()
        .map(|c| c.subfolder.clone())
        .unwrap_or_default();
    let target = if sub.is_empty() {
        name.clone()
    } else {
        format!("{sub}/{name}")
    };
    let args = json!({ "file": target, "format": what.format, "meeting": what.meeting_id });
    gated(
        conn,
        caller,
        integration_id,
        Capability::FilesWrite,
        &target,
        &args,
        approval_id,
        now_ms,
        || {
            let cfg = FolderConfig::from_config_json(&i.config_json).map_err(|e| e.to_string())?;
            let sandbox = Sandbox::open(&cfg.path).map_err(|e| e.to_string())?;
            sandbox
                .write_new(&cfg.subfolder, what.file_name, writer)
                .map_err(|e| e.to_string())
        },
    )
}

/// Schreibt oder aktualisiert die Notiz im Vault (`vault.write`).
pub fn save_note(
    conn: &Connection,
    caller: Caller,
    integration_id: &str,
    note: &NoteInput,
    approval_id: Option<&str>,
    now_ms: i64,
) -> Result<GateOutcome<SaveResult>, IntegrationError> {
    let i = load(conn, integration_id, Kind::Obsidian)?;
    let id = obsidian::note_id(&note.meeting_id);
    let args = json!({ "note": id, "title": note.title, "meeting": note.meeting_id });
    let target = format!("{id} ({})", note.title.chars().take(80).collect::<String>());
    gated(
        conn,
        caller,
        integration_id,
        Capability::VaultWrite,
        &target,
        &args,
        approval_id,
        now_ms,
        || {
            let cfg =
                ObsidianConfig::from_config_json(&i.config_json).map_err(|e| e.to_string())?;
            obsidian::save_note(&cfg, note).map_err(|e| e.to_string())
        },
    )
}

/// Sucht in der Wissensbasis (`knowledge.search`, Lesen: fuer Workflows und den
/// lokalen Agenten standardmaessig erlaubt).
#[allow(clippy::too_many_arguments)]
pub fn search_wissen(
    conn: &Connection,
    caller: Caller,
    integration_id: &str,
    query: &str,
    limit: Option<u32>,
    area: Option<&str>,
    approval_id: Option<&str>,
    secret: SecretLookup<'_>,
    opts: &HttpOpts,
    now_ms: i64,
) -> Result<GateOutcome<Vec<WissenHit>>, IntegrationError> {
    let i = load(conn, integration_id, Kind::Wissen)?;
    let args = json!({ "q": query, "limit": limit, "bereich": area });
    let target: String = query.chars().take(80).collect();
    gated(
        conn,
        caller,
        integration_id,
        Capability::KnowledgeSearch,
        &target,
        &args,
        approval_id,
        now_ms,
        || {
            let cfg = WissenConfig::from_config_json(&i.config_json).map_err(|e| e.to_string())?;
            let token = secret(&i, "token")?
                .ok_or_else(|| wissen::WissenError::TokenMissing.to_string())?;
            wissen::search(&cfg, &token, query, limit, area, opts).map_err(|e| e.to_string())
        },
    )
}

#[cfg(test)]
mod tests;
