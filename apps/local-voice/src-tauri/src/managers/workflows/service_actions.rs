//! Bausteine fuer Dienste (Welle 1, Spec 2026-10-06-verbindungen-aktionen):
//!
//! | Baustein          | Recht (Tor)    | Dienste                                                      |
//! |-------------------|----------------|--------------------------------------------------------------|
//! | `chat.post`       | `chat.post`    | Slack, Teams, Discord                                        |
//! | `task.create`     | `task.create`  | Asana, ClickUp, Jira, Trello, Todoist, monday, Linear, GitHub, HubSpot, Pipedrive |
//! | `task.create_from`| `task.create`  | dieselben: eine Aufgabe je To-do/Frist aus `agent.extract`   |
//! | `page.create`     | `page.write`   | Notion, Confluence                                           |
//! | `page.append`     | `page.write`   | Notion, Confluence                                           |
//! | `crm.note`        | `crm.write`    | HubSpot, Pipedrive (Kontakt per E-Mail verknuepft)           |
//! | `record.append`   | `record.write` | Airtable                                                     |
//!
//! Es gelten die Regeln von `integration_actions`: das Tor entscheidet VOR `run`; die Freigabe
//! zeigt Ziel und vollstaendigen Inhalt und ist an genau diese Fassung gebunden (`gate_view` und
//! `run` bilden die Eingabe mit derselben Funktion); was nach dem Senden scheitert, ist `Unknown`
//! und wird nie von selbst wiederholt; nach einem Absturz belegt `confirm` die Wirkung aus dem
//! Provenienz-Eintrag. `task.create_from` haelt jede angelegte Aufgabe einzeln fest: ein zweiter
//! Durchgang legt nur an, was noch fehlt.
//!
//! Der Ausfuehrer (`Exec`) ist austauschbar: im Betrieb `services::http::execute`, in Tests eine
//! Attrappe ohne Netz.

use std::sync::Arc;

use chrono::NaiveDate;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::managers::integrations::model::{Capability, Integration, Kind};
use crate::managers::integrations::services::http::{
    self, ApiReply, ApiRequest, Class, ServiceError,
};
use crate::managers::integrations::services::ops::{self, Account, Created, TaskInput};
use crate::managers::integrations::services::registry::{def, ServiceId};
use crate::managers::integrations::webhook::HttpOpts;
use crate::managers::meetings::mail::valid_address;
use crate::managers::provenance::SubjectKind;

use super::action::{
    Action, EffectKind, GateEnv, GateView, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::agent_notes::deadlines::from_of;
use super::agent_notes::items::{self, clean, Input, Kind as ItemKind};
use super::app_actions::{previous_result, record, text_param, AppServices};
use super::catalog::{self, ActionSpec};
use super::integration_actions::{db_err, integration_of};

/// Fuehrt eine Anfrage an einen Dienst aus.
pub type Exec = Arc<dyn Fn(&ApiRequest) -> Result<ApiReply, ServiceError> + Send + Sync>;

/// Der Ausfuehrer im Betrieb: HTTPS, keine Umleitungen, Zeit- und Groessengrenzen.
pub fn default_exec() -> Exec {
    Arc::new(|r: &ApiRequest| http::execute(r, &HttpOpts::default()))
}

const MAX_TEXT_CHARS: usize = 20_000;
const MAX_PAGE_CHARS: usize = 100_000;
const MAX_DESCRIPTION_CHARS: usize = 10_000;
const MAX_TITLE_CHARS: usize = 250;
/// Hoechstens so viele Aufgaben legt `task.create_from` in einem Schritt an.
pub const MAX_TASKS_FROM: usize = 30;
const MAX_RECORD_FIELDS: usize = 50;
const MAX_RECORD_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    ChatPost,
    TaskCreate,
    TaskCreateFrom,
    PageCreate,
    PageAppend,
    CrmNote,
    RecordAppend,
}

impl Op {
    const ALL: [Op; 7] = [
        Op::ChatPost,
        Op::TaskCreate,
        Op::TaskCreateFrom,
        Op::PageCreate,
        Op::PageAppend,
        Op::CrmNote,
        Op::RecordAppend,
    ];

    fn id(self) -> &'static str {
        match self {
            Op::ChatPost => "chat.post",
            Op::TaskCreate => "task.create",
            Op::TaskCreateFrom => "task.create_from",
            Op::PageCreate => "page.create",
            Op::PageAppend => "page.append",
            Op::CrmNote => "crm.note",
            Op::RecordAppend => "record.append",
        }
    }

    fn capability(self) -> Capability {
        match self {
            Op::ChatPost => Capability::ChatPost,
            Op::TaskCreate | Op::TaskCreateFrom => Capability::TaskCreate,
            Op::PageCreate | Op::PageAppend => Capability::PageWrite,
            Op::CrmNote => Capability::CrmWrite,
            Op::RecordAppend => Capability::RecordWrite,
        }
    }

    /// Was die Faehigkeit fuer Menschen heisst (Fehlermeldung).
    fn verb(self) -> &'static str {
        match self {
            Op::ChatPost => "in einen Kanal posten",
            Op::TaskCreate | Op::TaskCreateFrom => "Aufgaben anlegen",
            Op::PageCreate | Op::PageAppend => "Seiten schreiben",
            Op::CrmNote => "CRM-Notizen anlegen",
            Op::RecordAppend => "Datensätze anlegen",
        }
    }

    /// Operation im Provenienz-Register (nur Kleinbuchstaben, Ziffern, `_`).
    fn record_op(self) -> &'static str {
        match self {
            Op::ChatPost => "service_chat",
            Op::TaskCreate => "service_task",
            Op::TaskCreateFrom => "service_task_from",
            Op::PageCreate => "service_page",
            Op::PageAppend => "service_page_append",
            Op::CrmNote => "service_crm_note",
            Op::RecordAppend => "service_record",
        }
    }
}

/// Was ein Schritt tun wird, gebildet aus Parametern und Laufkontext (gleich fuer Tor und Lauf).
#[derive(Clone, Debug, PartialEq)]
enum Work {
    Chat(String),
    Task(TaskInput),
    Tasks(Vec<(String, TaskInput)>),
    /// `task.create_from` ohne Eingabe (Vorschritt uebersprungen): nichts zu tun, mit Grund.
    Nothing(String),
    Page {
        title: String,
        content: String,
    },
    Append {
        page: Option<String>,
        content: String,
    },
    Note {
        text: String,
        contact: Option<String>,
    },
    Record(Map<String, Value>),
}

pub struct ServiceAction {
    op: Op,
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
    exec: Exec,
}

/// Alle Dienst-Bausteine.
pub fn actions(services: Arc<dyn AppServices>, exec: Exec) -> Vec<Arc<dyn Action>> {
    Op::ALL
        .iter()
        .map(|op| {
            Arc::new(ServiceAction {
                op: *op,
                spec: catalog::action_spec(op.id())
                    .unwrap_or_else(|| panic!("Katalogeintrag {} fehlt", op.id())),
                services: services.clone(),
                exec: exec.clone(),
            }) as Arc<dyn Action>
        })
        .collect()
}

fn step_err(e: ServiceError) -> StepError {
    let text = e.to_string();
    match e.class() {
        Class::NotSent => StepError::Transient(text),
        Class::Rejected => StepError::Permanent(text),
        Class::Unknown => StepError::Unknown(text),
    }
}

fn permanent(s: impl Into<String>) -> StepError {
    StepError::Permanent(s.into())
}

/// Eine Zeile Text: Steuerzeichen weg, Leerraum zusammengefasst, gekuerzt.
fn one_line(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// Text mit Zeilenumbruechen: andere Steuerzeichen weg, `\r` weg.
fn multi_line(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\r')
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn long_text(params: &Value, key: &str, max: usize, what: &str) -> Result<String, StepError> {
    let raw = params.get(key).and_then(Value::as_str).unwrap_or("");
    let text = multi_line(raw);
    if text.is_empty() {
        return Err(permanent(format!("{what} ist leer (Parameter {key}).")));
    }
    if text.chars().count() > max {
        return Err(permanent(format!(
            "{what} ist zu lang ({} Zeichen, höchstens {max}).",
            text.chars().count()
        )));
    }
    Ok(text)
}

fn due_of(params: &Value) -> Result<Option<NaiveDate>, StepError> {
    match text_param(params, "due") {
        None => Ok(None),
        Some(d) => NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| {
                permanent(format!(
                    "Das Fälligkeitsdatum „{d}“ ist kein Datum (JJJJ-MM-TT)."
                ))
            }),
    }
}

/// Aufgaben aus dem Ergebnis von `agent.extract`: jedes To-do, mit `deadlines` (Vorgabe ja)
/// auch jede Frist. Kennung je Eintrag fuer den Dublettenschutz (`Item::key`).
fn tasks_from(context: &Value, params: &Value) -> Result<Work, StepError> {
    let from = from_of(params)?;
    let ex = match items::read(context, &from)? {
        Input::Data(ex) => ex,
        Input::Nothing(why) => return Ok(Work::Nothing(why)),
    };
    let with_deadlines = params
        .get("deadlines")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let tasks: Vec<(String, TaskInput)> = ex
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Todo || (with_deadlines && i.kind == ItemKind::Deadline))
        .take(MAX_TASKS_FROM)
        .map(|i| {
            let mut desc = Vec::new();
            if let Some(a) = &i.assignee {
                desc.push(format!("Zuständig: {}", clean(a, 80)));
            }
            if i.kind == ItemKind::Deadline {
                desc.push("Frist aus der Besprechung.".to_string());
            }
            if i.unverified_date && i.due.is_some() {
                desc.push("Datum vom Sprachmodell geschätzt, bitte prüfen.".to_string());
            }
            if !i.quote.is_empty() {
                desc.push(format!("Beleg: „{}“", clean(&i.quote, 200)));
            }
            desc.push("Angelegt von Local Voice AI.".to_string());
            (
                i.key(),
                TaskInput {
                    title: one_line(&i.text, MAX_TITLE_CHARS),
                    description: desc.join("\n"),
                    due: i.due,
                },
            )
        })
        .collect();
    if tasks.is_empty() {
        return Ok(Work::Nothing(
            "Der Extraktionsschritt hat keine To-dos oder Fristen geliefert.".to_string(),
        ));
    }
    Ok(Work::Tasks(tasks))
}

fn record_fields(params: &Value) -> Result<Map<String, Value>, StepError> {
    let Some(Value::Object(fields)) = params.get("fields") else {
        return Err(permanent(
            "Die Felder des Datensatzes fehlen (Parameter fields, ein Objekt).",
        ));
    };
    if fields.is_empty() || fields.len() > MAX_RECORD_FIELDS {
        return Err(permanent(format!(
            "Ein Datensatz braucht 1 bis {MAX_RECORD_FIELDS} Felder."
        )));
    }
    let mut out = Map::new();
    for (k, v) in fields {
        let key = one_line(k, 100);
        if key.is_empty() {
            return Err(permanent("Ein Feldname ist leer."));
        }
        let ok = match v {
            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => true,
            Value::Array(a) => a.iter().all(Value::is_string),
            Value::Object(_) => false,
        };
        if !ok {
            return Err(permanent(format!(
                "Das Feld „{key}“ hat keinen einfachen Wert (Text, Zahl, Ja/Nein oder Liste von Texten)."
            )));
        }
        let v = match v {
            Value::String(s) => Value::String(multi_line(s)),
            other => other.clone(),
        };
        out.insert(key, v);
    }
    if Value::Object(out.clone()).to_string().len() > MAX_RECORD_BYTES {
        return Err(permanent("Der Datensatz ist zu groß (höchstens 64 KiB)."));
    }
    Ok(out)
}

fn work_of(op: Op, context: &Value, params: &Value) -> Result<Work, StepError> {
    Ok(match op {
        Op::ChatPost => Work::Chat(long_text(params, "text", MAX_TEXT_CHARS, "Der Text")?),
        Op::TaskCreate => {
            let title = one_line(
                params.get("title").and_then(Value::as_str).unwrap_or(""),
                MAX_TITLE_CHARS,
            );
            if title.is_empty() {
                return Err(permanent("Die Aufgabe hat keinen Titel (Parameter title)."));
            }
            let description = multi_line(
                params
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
            .chars()
            .take(MAX_DESCRIPTION_CHARS)
            .collect();
            Work::Task(TaskInput {
                title,
                description,
                due: due_of(params)?,
            })
        }
        Op::TaskCreateFrom => tasks_from(context, params)?,
        Op::PageCreate => {
            let title = one_line(
                params.get("title").and_then(Value::as_str).unwrap_or(""),
                200,
            );
            if title.is_empty() {
                return Err(permanent("Die Seite hat keinen Titel (Parameter title)."));
            }
            Work::Page {
                title,
                content: long_text(params, "content", MAX_PAGE_CHARS, "Der Inhalt")?,
            }
        }
        Op::PageAppend => Work::Append {
            page: text_param(params, "page").map(str::to_string),
            content: long_text(params, "content", MAX_PAGE_CHARS, "Der Inhalt")?,
        },
        Op::CrmNote => {
            let contact = match text_param(params, "contact_email") {
                None => None,
                Some(e) if valid_address(e) => Some(e.to_ascii_lowercase()),
                Some(e) => {
                    return Err(permanent(format!(
                        "„{}“ ist keine gültige E-Mail-Adresse (Parameter contact_email).",
                        one_line(e, 80)
                    )))
                }
            };
            Work::Note {
                text: long_text(params, "text", MAX_TEXT_CHARS, "Die Notiz")?,
                contact,
            }
        }
        Op::RecordAppend => Work::Record(record_fields(params)?),
    })
}

/// Die Dienst-Integration `via`; sie muss die Faehigkeit des Bausteins anbieten.
fn service_integration(
    conn: &rusqlite::Connection,
    params: &Value,
    op: Op,
) -> Result<(Integration, ServiceId), StepError> {
    let i = integration_of(
        conn,
        params,
        &[Kind::Service],
        "kein Dienst (Slack, Jira, Notion …)",
    )?;
    let id = i.service().ok_or_else(|| {
        permanent(format!(
            "Die Integration „{}“ hat keinen bekannten Dienst.",
            i.id
        ))
    })?;
    if !i.capabilities().contains(&op.capability()) {
        return Err(permanent(format!(
            "{} kann keine {} (Integration „{}“).",
            def(id).label,
            op.verb(),
            one_line(&i.label, 60)
        )));
    }
    Ok((i, id))
}

/// Ziel fuer Freigabe und Audit: Dienst, Name, Site bzw. Server; nie ein Geheimnis.
fn target_of(i: &Integration, id: ServiceId) -> String {
    let label = one_line(&i.label, 60);
    match &i.account_hint {
        Some(h) if !h.is_empty() => format!("{} „{label}“ ({h})", def(id).label),
        _ => format!("{} „{label}“", def(id).label),
    }
}

fn task_json(t: &TaskInput) -> Value {
    json!({
        "titel": t.title,
        "beschreibung": t.description,
        "faellig": t.due.map(|d| d.format("%Y-%m-%d").to_string()),
    })
}

fn work_args(work: &Work) -> Value {
    match work {
        Work::Chat(text) => json!({ "text": text }),
        Work::Task(t) => task_json(t),
        Work::Tasks(list) => json!({
            "anzahl": list.len(),
            "aufgaben": list.iter().map(|(_, t)| task_json(t)).collect::<Vec<_>>(),
        }),
        Work::Nothing(why) => json!({ "hinweis": why }),
        Work::Page { title, content } => json!({ "titel": title, "inhalt": content }),
        Work::Append { page, content } => json!({ "seite": page, "inhalt": content }),
        Work::Note { text, contact } => json!({ "notiz": text, "kontakt": contact }),
        Work::Record(fields) => json!({ "felder": fields }),
    }
}

fn created_json(c: &Created) -> Value {
    json!({ "id": c.id, "url": c.url })
}

/// Operation je angelegter Aufgabe (`Item::key` ist hexadezimal, 16 Zeichen).
fn item_op(key: &str) -> String {
    format!("svc_task_{key}")
}

/// Kurze, stabile Kennung eines Textes fuer das Provenienz-Register (kein Inhalt).
fn digest(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl ServiceAction {
    fn token(&self, i: &Integration) -> Result<Zeroizing<String>, StepError> {
        match self.services.secret(i, "token") {
            Ok(Some(t)) if !t.trim().is_empty() => Ok(t),
            Ok(_) => Err(permanent(format!(
                "Für die Integration „{}“ fehlt der Schlüssel bzw. die Webhook-Adresse. Bitte in den Integrationen neu eintragen.",
                one_line(&i.label, 60)
            ))),
            Err(e) => Err(permanent(e)),
        }
    }

    /// Ergebnis eines frueheren Durchgangs desselben Schritts (Wiederaufnahme).
    fn earlier(&self, ctx: &RunCtx<'_>) -> Option<StepOutput> {
        let (_, prev) = previous_result(ctx, SubjectKind::Export, self.op.record_op())?;
        let mut data = prev.unwrap_or_else(|| json!({}));
        data["ok"] = json!(true);
        data["reused"] = json!(true);
        Some(StepOutput::with_data(data).summary("Schon erledigt (Wiederaufnahme)."))
    }

    fn run_tasks(
        &self,
        ctx: &RunCtx<'_>,
        acc: &Account<'_>,
        i: &Integration,
        tasks: &[(String, TaskInput)],
    ) -> Result<StepOutput, StepError> {
        let mut created: Vec<Value> = Vec::new();
        let mut reused = 0usize;
        let mut exec = |r: ApiRequest| (self.exec)(&r);
        for (key, task) in tasks {
            if previous_result(ctx, SubjectKind::Export, &item_op(key)).is_some() {
                reused += 1;
                continue;
            }
            if ctx.cancelled() {
                // Was angelegt ist, steht im Register; ein neuer Durchgang legt nur den Rest an.
                return Err(StepError::Transient(format!(
                    "Der Lauf wurde abgebrochen ({} von {} Aufgaben angelegt).",
                    created.len() + reused,
                    tasks.len()
                )));
            }
            let c = ops::task_create(acc, task, &mut exec).map_err(|e| {
                let done = created.len() + reused;
                let inner = step_err(e);
                if done == 0 {
                    return inner;
                }
                let more = format!(
                    " ({done} von {} Aufgaben waren schon angelegt.)",
                    tasks.len()
                );
                match inner {
                    StepError::Transient(t) => StepError::Transient(t + &more),
                    StepError::Permanent(t) => StepError::Permanent(t + &more),
                    StepError::Unknown(t) => StepError::Unknown(t + &more),
                    other => other,
                }
            })?;
            record(
                ctx,
                SubjectKind::Export,
                &format!("{}:{key}", ctx.idempotency_key),
                &item_op(key),
                Vec::new(),
                json!({ "via": i.id, "id": c.id, "url": c.url }),
            );
            let mut v = created_json(&c);
            v["titel"] = json!(task.title);
            created.push(v);
        }
        let n = created.len();
        let summary = match (n, reused) {
            (0, r) => format!("Alle {r} Aufgaben waren schon angelegt (Wiederaufnahme)."),
            (n, 0) => format!("{n} Aufgaben in {} angelegt.", def(acc.service).label),
            (n, r) => format!(
                "{n} Aufgaben in {} angelegt, {r} waren schon da.",
                def(acc.service).label
            ),
        };
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            self.op.record_op(),
            Vec::new(),
            json!({ "via": i.id, "count": n + reused }),
        );
        Ok(StepOutput::with_data(json!({
            "ok": true, "created": created, "count": n, "reused_count": reused,
        }))
        .summary(&summary))
    }
}

impl Action for ServiceAction {
    fn id(&self) -> &str {
        self.op.id()
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let (i, id) = service_integration(env.conn, params, self.op)?;
        let target = target_of(&i, id);
        // Trockenlauf: das Ergebnis des Extraktionsschritts gibt es noch nicht.
        if self.op == Op::TaskCreateFrom && env.planning {
            let from = from_of(params)?;
            if env
                .context
                .pointer(&format!("/steps/{from}/outcome"))
                .is_none()
            {
                return Ok(Some(GateView {
                    target: Some(target),
                    args: json!({
                        "via": i.id,
                        "hinweis": format!("Die Aufgaben stehen erst nach Schritt „{from}“ fest und werden dann zur Freigabe vorgelegt."),
                    }),
                    max_mode: None,
                }));
            }
        }
        let work = match work_of(self.op, env.context, params) {
            Ok(w) => w,
            // Im Trockenlauf stehen Inhalte oft noch als `{{...}}` da: kein Fehler.
            Err(_) if env.planning => {
                Work::Nothing("Der Inhalt steht erst beim Lauf fest.".to_string())
            }
            Err(e) => return Err(e),
        };
        let mut args = work_args(&work);
        args["via"] = json!(i.id);
        args["dienst"] = json!(def(id).label);
        Ok(Some(GateView {
            target: Some(target),
            args,
            max_mode: None,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if self.op != Op::TaskCreateFrom {
            if let Some(out) = self.earlier(ctx) {
                return Ok(out);
            }
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor der Dienst gerufen wurde.".to_string(),
            ));
        }
        let work = work_of(self.op, ctx.context, params)?;
        if let Work::Nothing(why) = &work {
            return Ok(StepOutput::with_data(
                json!({"ok": true, "created": [], "count": 0, "reason": why}),
            )
            .summary(&format!("Nichts angelegt: {why}")));
        }
        let conn = ctx.conn().map_err(db_err)?;
        let (i, id) = service_integration(&conn, params, self.op)?;
        drop(conn);
        let token = self.token(&i)?;
        let settings: Map<String, Value> = serde_json::from_str(&i.config_json).unwrap_or_default();
        let acc = Account {
            service: id,
            token: &token,
            settings: &settings,
        };
        let label = def(id).label;
        let mut exec = |r: ApiRequest| (self.exec)(&r);
        let (data, summary) = match &work {
            Work::Tasks(tasks) => return self.run_tasks(ctx, &acc, &i, tasks),
            Work::Nothing(_) => unreachable!("oben behandelt"),
            Work::Chat(text) => {
                let n = ops::chat_post(&acc, text, &mut exec).map_err(step_err)?;
                (
                    json!({ "messages": n, "chars": text.chars().count(), "digest": digest(text) }),
                    if n == 1 {
                        format!("In {label} gepostet.")
                    } else {
                        format!("In {label} gepostet ({n} Nachrichten).")
                    },
                )
            }
            Work::Task(t) => {
                let c = ops::task_create(&acc, t, &mut exec).map_err(step_err)?;
                (created_json(&c), format!("Aufgabe in {label} angelegt."))
            }
            Work::Page { title, content } => {
                let c = ops::page_create(&acc, title, content, &mut exec).map_err(step_err)?;
                (created_json(&c), format!("Seite in {label} angelegt."))
            }
            Work::Append { page, content } => {
                let c = ops::page_append(&acc, page.as_deref(), content, &mut exec)
                    .map_err(step_err)?;
                (
                    created_json(&c),
                    format!("An die Seite in {label} angehängt."),
                )
            }
            Work::Note { text, contact } => {
                let (c, linked) =
                    ops::crm_note(&acc, text, contact.as_deref(), &mut exec).map_err(step_err)?;
                let mut d = created_json(&c);
                d["linked"] = json!(linked);
                (
                    d,
                    match (contact, linked) {
                        (Some(_), false) => format!(
                            "Notiz in {label} angelegt; der Kontakt wurde nicht gefunden, die Notiz ist nicht verknüpft."
                        ),
                        _ => format!("Notiz in {label} angelegt."),
                    },
                )
            }
            Work::Record(fields) => {
                let c = ops::record_append(&acc, fields, &mut exec).map_err(step_err)?;
                (created_json(&c), format!("Datensatz in {label} angelegt."))
            }
        };
        let mut prov = data.clone();
        prov["via"] = json!(i.id);
        prov["service"] = json!(id.as_str());
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            self.op.record_op(),
            Vec::new(),
            prov,
        );
        let mut out = data;
        out["ok"] = json!(true);
        out["service"] = json!(id.as_str());
        Ok(StepOutput::with_data(out).summary(&summary))
    }

    fn confirm(&self, ctx: &RunCtx<'_>, params: &Value) -> Option<StepOutput> {
        if self.op != Op::TaskCreateFrom {
            return self.earlier(ctx);
        }
        // Belegt nur, wenn JEDE Aufgabe festgehalten ist; sonst bleibt die Wirkung unklar.
        let Ok(Work::Tasks(tasks)) = work_of(self.op, ctx.context, params) else {
            return None;
        };
        let all = tasks
            .iter()
            .all(|(k, _)| previous_result(ctx, SubjectKind::Export, &item_op(k)).is_some());
        all.then(|| {
            StepOutput::with_data(json!({"ok": true, "count": 0, "reused_count": tasks.len()}))
                .summary("Alle Aufgaben waren schon angelegt (Wiederaufnahme).")
        })
    }
}

#[cfg(test)]
mod tests;
