//! Der Baustein `obsidian.note` (B6): eine Notiz im Vault schreiben, ohne Dublette.
//!
//! Das Recht (`vault.write` am Vault `via`) prueft die Engine vor `run`. Dazu bildet der Baustein ueber
//! `Action::gate_view` die Ansicht, die das Tor sieht und die Freigabe bindet: Schluessel, Zielpfad, ob
//! die Notiz NEU entsteht oder GEAENDERT wird, Groesse, Pruefsumme des endgueltigen Textes und eine kurze
//! Vorschau. Wird eine vorhandene Notiz geaendert, gilt hoechstens „fragen“ (`GateView::max_mode`), auch
//! wenn der Nutzer „erlaubt“ eingestellt hat (R8/E9); `auto: true` im Schritt hebt das fuer diesen Ablauf
//! auf. Aendern heisst hier immer nur: den Block der App zwischen den Marken ersetzen bzw. einen Eintrag
//! ersetzen oder oben einfuegen, nie das Umschreiben fremden Textes.
//!
//! Die Freigabe gilt fuer genau diesen Text: aendert sich die Notiz oder der Inhalt bis zur Entscheidung,
//! passt die Pruefsumme nicht mehr, und der Schritt wird abgelehnt.
//!
//! # Wiederholung und Absturz
//!
//! Der Baustein ist `Idempotent`: derselbe Schluessel mit demselben Inhalt ergibt dieselbe Datei und beim
//! zweiten Mal `Unchanged` (keine zweite Datei, keine zweite Aenderung, kein zweiter Provenienz-Eintrag).
//! Stirbt die App nach dem Schreiben und vor dem Journal, belegt `confirm` (und die Wiederholung selbst)
//! das ueber diesen Vergleich. Entstand die Notiz zwischen Tor und Schreiben anderswo (sie wurde zur
//! Aenderung), schreibt der Baustein NICHT ohne Freigabe weiter, sondern meldet `Transient`: der naechste
//! Versuch geht mit der richtigen Ansicht durchs Tor.
//!
//! # Ergebnis
//!
//! `steps.<id>.path` (relativ zum Vault, nie absolut), `.mode` (`created`, `updated`, `unchanged`),
//! `.key`, `.entry`, `.bytes`, `.reused` (Wiederholung).

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::managers::integrations::folder::{self, FolderError, Sandbox};
use crate::managers::integrations::model::{GrantMode, Integration, Kind as IntegrationKind};
use crate::managers::integrations::obsidian::{ObsidianConfig, ObsidianError};
use crate::managers::integrations::store as integrations_store;
use crate::managers::provenance::{SourceRef, SubjectKind};

use super::super::action::{
    Action, EffectKind, GateEnv, GateView, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::super::app_actions::{has_template, previous_result, record, text_param};
use super::super::catalog::{self, ActionSpec};
use super::note::{
    self, valid_id, valid_meta_key, DataClass, Kind, MetaValue, NoteSpec, Plan, MAX_CONTENT_CHARS,
    VAULT_LOCK,
};
use super::{db_err, md_block, md_inline};

const OPERATION: &str = "vault_note";

fn spec_of() -> &'static ActionSpec {
    catalog::action_spec("obsidian.note")
        .unwrap_or_else(|| panic!("Katalogeintrag obsidian.note fehlt"))
}

fn permanent(m: impl Into<String>) -> StepError {
    StepError::Permanent(m.into())
}

// ---------------------------------------------------------------------------
// Parameter -> Spezifikation
// ---------------------------------------------------------------------------

fn opt_text(params: &Value, key: &str, max: usize) -> Option<String> {
    text_param(params, key)
        .map(|t| md_inline(t, max))
        .filter(|t| !t.is_empty())
}

/// `JJJJ-MM-TT`?
fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Das Datum der Notiz: Parameter, sonst Veroeffentlichung des Ausloesers, sonst der Beginn des Laufs.
/// Ohne Uhr: dieselbe Spezifikation ergibt immer denselben Text.
fn date_of(params: &Value, context: &Value) -> Result<String, String> {
    if let Some(d) = text_param(params, "date") {
        return if valid_date(d) {
            Ok(d.to_string())
        } else {
            Err(format!(
                "Das Datum „{d}“ ist nicht von der Form JJJJ-MM-TT."
            ))
        };
    }
    if let Some(p) = context
        .pointer("/trigger/published")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if let Some(day) = p.get(..10).filter(|d| valid_date(d)) {
            return Ok(day.to_string());
        }
    }
    let ms = context.pointer("/run/started_at").and_then(Value::as_i64);
    Ok(ms
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string()))
}

fn meta_value(key: &str, v: &Value) -> Result<MetaValue, String> {
    match v {
        Value::String(s) => Ok(MetaValue(note::yq(&md_inline(s, 200)))),
        Value::Number(n) => Ok(MetaValue(n.to_string())),
        Value::Bool(b) => Ok(MetaValue(b.to_string())),
        _ => Err(format!("meta.{key}: erlaubt sind Text, Zahl und Ja/Nein.")),
    }
}

/// Die Parameter (eingesetzt) als Spezifikation; Fehler sind Saetze fuer den Nutzer.
pub fn spec_from(params: &Value, context: &Value) -> Result<NoteSpec, String> {
    let key = text_param(params, "key")
        .ok_or_else(|| "Es fehlt der Schlüssel der Notiz (key).".to_string())?;
    if !valid_id(key) {
        return Err(format!(
            "Der Schlüssel „{key}“ ist ungültig: erlaubt sind Buchstaben, Ziffern, - und _ (höchstens {}).",
            note::MAX_KEY_CHARS
        ));
    }
    let title = opt_text(params, "title", 160)
        .ok_or_else(|| "Es fehlt der Titel der Notiz.".to_string())?;
    let content = text_param(params, "content")
        .map(|c| md_block(c, MAX_CONTENT_CHARS))
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "Es fehlt der Inhalt der Notiz.".to_string())?;
    let entry = match text_param(params, "entry") {
        None => None,
        Some(e) if valid_id(e) => Some(e.to_string()),
        Some(e) => {
            return Err(format!(
                "Die Eintragskennung „{e}“ ist ungültig: erlaubt sind Buchstaben, Ziffern, - und _."
            ))
        }
    };
    let folder = match text_param(params, "folder") {
        None => None,
        Some(f) => {
            folder::check_relative(f)
                .map_err(|_| format!("Der Unterordner „{f}“ liegt nicht innerhalb des Vaults."))?;
            Some(f.to_string())
        }
    };
    let mut tags = Vec::new();
    if let Some(Value::Array(items)) = params.get("tags") {
        for t in items.iter().filter_map(Value::as_str) {
            let t = t.trim().to_lowercase();
            if t.is_empty() {
                continue;
            }
            let ok = t.chars().count() <= 30
                && t.chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
            if !ok {
                return Err(format!(
                    "Der Tag „{t}“ ist ungültig: erlaubt sind Buchstaben, Ziffern, - und _."
                ));
            }
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
    }
    let mut meta = Vec::new();
    match params.get("meta") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if !valid_meta_key(k) {
                    return Err(format!(
                        "meta.{k}: der Name ist ungültig oder von der App belegt (klein, Ziffern, _; höchstens 30)."
                    ));
                }
                meta.push((k.clone(), meta_value(k, v)?));
            }
        }
        Some(_) => return Err("meta: erwartet wird ein Objekt aus Namen und Werten.".to_string()),
    }
    let source_url = text_param(params, "source_url").map(str::to_string);
    if let Some(u) = &source_url {
        if !(u.starts_with("https://") || u.starts_with("http://"))
            || u.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(
                "source_url: erwartet wird ein Link mit http:// oder https://.".to_string(),
            );
        }
    }
    let data_class = match text_param(params, "data_class") {
        None | Some("internal") => DataClass::Internal,
        Some("confidential") => DataClass::Confidential,
        Some(other) => {
            return Err(format!(
                "data_class: „{other}“ gibt es nicht (internal, confidential)."
            ))
        }
    };
    Ok(NoteSpec {
        key: key.to_string(),
        title,
        content,
        name: opt_text(params, "name", 120),
        folder,
        date: date_of(params, context)?,
        entry,
        source_title: opt_text(params, "source_title", 160),
        source_url,
        origin: opt_text(params, "origin", 240),
        tags,
        meta,
        data_class,
        auto: params.get("auto").and_then(Value::as_bool).unwrap_or(false),
    })
}

// ---------------------------------------------------------------------------
// Vault oeffnen
// ---------------------------------------------------------------------------

fn obsidian_error(e: ObsidianError) -> StepError {
    let text = e.to_string();
    match e {
        // Der Vault kann zurueckkommen (Laufwerk, OneDrive noch nicht gestartet).
        ObsidianError::Folder(FolderError::RootNotFound) | ObsidianError::Io(_) => {
            StepError::Transient(text)
        }
        other => match other {
            ObsidianError::Folder(FolderError::Io(_)) => StepError::Transient(text),
            _ => StepError::Permanent(text),
        },
    }
}

fn folder_error(e: FolderError) -> StepError {
    match e {
        FolderError::RootNotFound | FolderError::Io(_) => StepError::Transient(e.to_string()),
        other => StepError::Permanent(other.to_string()),
    }
}

/// Der Vault `via` aus dem Register: Integration, Einstellungen und Sandbox.
pub struct Vault {
    pub integration: Integration,
    pub cfg: ObsidianConfig,
    pub sandbox: Sandbox,
}

pub fn open_vault(conn: &Connection, id: &str) -> Result<Vault, StepError> {
    let integration = integrations_store::get(conn, id)
        .map_err(db_err)?
        .ok_or_else(|| permanent(format!("Die Integration „{id}“ gibt es nicht.")))?;
    if integration.kind != IntegrationKind::Obsidian {
        return Err(permanent(format!(
            "Die Integration „{id}“ ist kein Obsidian-Vault."
        )));
    }
    let cfg = ObsidianConfig::from_config_json(&integration.config_json).map_err(obsidian_error)?;
    let sandbox = Sandbox::open(&cfg.path).map_err(folder_error)?;
    Ok(Vault {
        integration,
        cfg,
        sandbox,
    })
}

// ---------------------------------------------------------------------------
// Der Baustein
// ---------------------------------------------------------------------------

pub struct ObsidianNote {
    spec: &'static ActionSpec,
}

impl ObsidianNote {
    pub fn new() -> Self {
        Self { spec: spec_of() }
    }

    fn plan_for(
        &self,
        conn: &Connection,
        params: &Value,
        context: &Value,
    ) -> Result<(Vault, NoteSpec, Plan), StepError> {
        let via = text_param(params, "via")
            .ok_or_else(|| permanent("Es ist kein Vault angegeben (Parameter via)."))?;
        let spec = spec_from(params, context).map_err(permanent)?;
        let vault = open_vault(conn, via)?;
        let plan = note::plan(&vault.sandbox, &vault.cfg, &spec).map_err(obsidian_error)?;
        Ok((vault, spec, plan))
    }
}

impl Default for ObsidianNote {
    fn default() -> Self {
        Self::new()
    }
}

fn output_of(spec: &NoteSpec, written: &note::Written, reused: bool) -> StepOutput {
    let mode = match written.kind {
        Kind::Create => "created",
        Kind::Modify => "updated",
        Kind::Unchanged => "unchanged",
    };
    let mut out = StepOutput::with_data(json!({
        "path": written.rel,
        "mode": mode,
        "key": spec.key,
        "entry": spec.entry,
        "title": spec.title,
        "bytes": written.bytes,
        "reused": reused,
    }));
    out.summary = Some(match (written.kind, reused) {
        (Kind::Create, _) => format!(
            "Notiz „{}“ im Vault angelegt ({}).",
            spec.title, written.rel
        ),
        (Kind::Modify, _) => format!("Notiz „{}“ im Vault ergänzt ({}).", spec.title, written.rel),
        (Kind::Unchanged, true) => {
            format!("Notiz „{}“ lag schon vor (Wiederaufnahme).", spec.title)
        }
        (Kind::Unchanged, false) => {
            format!(
                "Notiz „{}“ ist schon auf dem Stand, nichts geändert.",
                spec.title
            )
        }
    });
    let mut source = SourceRef::new("vault", &spec.key, Some(&spec.title));
    source.url = None;
    out.sources = vec![source];
    out
}

impl Action for ObsidianNote {
    fn id(&self) -> &str {
        "obsidian.note"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        let text = |k: &str| match params.get(k) {
            Some(Value::String(s)) if !has_template(s) => Some(s.trim().to_string()),
            _ => None,
        };
        for (field, what) in [("key", "Schlüssel"), ("entry", "Eintragskennung")] {
            if let Some(v) = text(field) {
                if !v.is_empty() && !valid_id(&v) {
                    return Err(format!(
                        "{field}: der {what} „{v}“ ist ungültig (Buchstaben, Ziffern, - und _; höchstens {}).",
                        note::MAX_KEY_CHARS
                    ));
                }
            }
        }
        if let Some(f) = text("folder") {
            if !f.is_empty() {
                folder::check_relative(&f)
                    .map_err(|_| format!("folder: „{f}“ liegt nicht innerhalb des Vaults."))?;
            }
        }
        if let Some(d) = text("date") {
            if !d.is_empty() && !valid_date(&d) {
                return Err(format!("date: „{d}“ ist nicht von der Form JJJJ-MM-TT."));
            }
        }
        if let Some(Value::Object(map)) = params.get("meta") {
            for (k, v) in map {
                if !valid_meta_key(k) {
                    return Err(format!(
                        "meta.{k}: der Name ist ungültig oder von der App belegt (klein, Ziffern, _; höchstens 30)."
                    ));
                }
                if !matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                    return Err(format!("meta.{k}: erlaubt sind Text, Zahl und Ja/Nein."));
                }
            }
        } else if let Some(v) = params.get("meta") {
            if !v.is_null() && !v.is_object() {
                return Err("meta: erwartet wird ein Objekt aus Namen und Werten.".to_string());
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let base = catalog::describe_from_spec(self.spec, params);
        match text_param(params, "entry") {
            Some(e) => format!("{base}; Eintrag {e} in einer Sammelnotiz"),
            None => base,
        }
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let title = text_param(params, "title").unwrap_or("Notiz");
        let target: String = format!(
            "{} ({})",
            text_param(params, "key").unwrap_or("?"),
            md_inline(title, 80)
        );
        // Trockenlauf: Teile der Parameter stehen noch als `{{...}}` da; nichts im Vault lesen, was
        // sich nicht sicher bestimmen laesst.
        let unresolved = ["key", "via", "title", "content", "entry", "folder"]
            .iter()
            .any(|k| text_param(params, k).is_some_and(has_template));
        if env.planning && unresolved {
            return Ok(Some(GateView {
                target: Some(target),
                args: json!({
                    "via": text_param(params, "via"),
                    "note": "(wird beim Lauf bestimmt)",
                    "mode": "(wird beim Lauf bestimmt)",
                }),
                max_mode: None,
            }));
        }
        let (_, spec, plan) = match self.plan_for(env.conn, params, env.context) {
            Ok(p) => p,
            // Im Trockenlauf ist ein nicht erreichbarer Vault kein Fehler der Ansicht.
            Err(StepError::Transient(_)) if env.planning => {
                return Ok(Some(GateView {
                    target: Some(target),
                    args: json!({"note": text_param(params, "key"), "mode": "(Vault nicht erreichbar)"}),
                    max_mode: None,
                }));
            }
            Err(e) => return Err(e),
        };
        let preview: String = md_inline(&spec.content, 140);
        let args = json!({
            "via": text_param(params, "via"),
            "note": spec.key,
            "entry": spec.entry,
            "file": plan.rel,
            "mode": plan.kind.as_str(),
            "title": spec.title,
            "bytes": plan.text.len(),
            "sha256": plan.sha,
            "vorschau": preview,
        });
        let max_mode = (plan.kind == Kind::Modify && !spec.auto).then_some(GrantMode::Ask);
        Ok(Some(GateView {
            target: Some(target),
            args,
            max_mode,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let conn = ctx.conn().map_err(db_err)?;
        let _lock = VAULT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (vault, spec, plan) = self.plan_for(&conn, params, ctx.context)?;
        match plan.kind {
            Kind::Unchanged => {
                let reused = previous_result(ctx, SubjectKind::KnowledgeNote, OPERATION).is_some();
                let written = note::Written {
                    kind: Kind::Unchanged,
                    rel: plan.rel.clone(),
                    bytes: plan.text.len() as u64,
                };
                return Ok(output_of(&spec, &written, reused));
            }
            Kind::Modify if !spec.auto && !ctx.approved => {
                // Die Notiz entstand zwischen Tor und Schreiben anderswo: die Aenderung braucht die
                // Freigabe, die dieser Versuch nicht hat. Es wurde nichts geschrieben.
                return Err(StepError::Transient(
                    "Die Notiz gibt es inzwischen schon; ändern darf der Ablauf sie nur mit Freigabe. Der nächste Versuch fragt."
                        .to_string(),
                ));
            }
            _ => {}
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen.".to_string(),
            ));
        }
        let written = note::apply(&vault.sandbox, &plan).map_err(folder_error)?;
        record(
            ctx,
            SubjectKind::KnowledgeNote,
            &spec.key,
            OPERATION,
            vec![],
            json!({
                "path": written.rel,
                "mode": plan.kind.as_str(),
                "entry": spec.entry,
                "sha256": plan.sha,
                "idempotency_key": ctx.idempotency_key,
            }),
        );
        Ok(output_of(&spec, &written, false))
    }

    /// Nach einem Absturz: enthaelt der Vault schon genau diesen Text, war die Wirkung eingetreten.
    fn confirm(&self, ctx: &RunCtx<'_>, params: &Value) -> Option<StepOutput> {
        let conn = ctx.conn().ok()?;
        let (_, spec, plan) = self.plan_for(&conn, params, ctx.context).ok()?;
        (plan.kind == Kind::Unchanged).then(|| {
            let written = note::Written {
                kind: Kind::Unchanged,
                rel: plan.rel.clone(),
                bytes: plan.text.len() as u64,
            };
            output_of(&spec, &written, true)
        })
    }
}
