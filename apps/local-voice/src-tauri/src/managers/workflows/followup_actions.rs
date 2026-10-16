//! Folge-Bausteine fuer Outlook (Welle 2, Spec 2026-10-06-verbindungen-aktionen):
//!
//! | Baustein            | Recht (Tor)      | Ziel                                                     |
//! |---------------------|------------------|----------------------------------------------------------|
//! | `mail.draft`        | `mail.draft`     | Microsoft 365: Entwurf im Ordner „Entwürfe“ (`POST /me/messages`) |
//! | `calendar.followup` | `calendar.write` | Microsoft 365: Folgetermin mit Uhrzeit (`POST /me/events`) |
//!
//! `mail.draft` bildet Empfaenger, Betreff, Text und Anhaenge genau wie `mail.send`
//! (`MailSend::build`): feste Regel, keine Adresse aus Daten, Anhaenge nur aus den Ordnern des
//! Nutzers und an die Freigabe gebunden. Ein Entwurf erreicht niemanden; E3 („an Dritte nur nach
//! Freigabe“) greift deshalb nicht, gesendet wird erst in Outlook von Hand.
//!
//! `calendar.followup` legt einen Termin im eigenen Kalender an. Mit `invite` (Regel wie bei der
//! Mail) laedt Outlook die Teilnehmenden ein: dann ist hoechstens „fragen“ moeglich (E3), die
//! Freigabe zeigt alle Eingeladenen. Gegen Dubletten traegt jede Anfrage eine `transactionId`
//! aus Lauf und Schritt; dazu der Provenienz-Eintrag wie bei allen Bausteinen.

use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::managers::integrations::m365::event_create::{NewTimedEvent, MAX_MINUTES, MIN_MINUTES};
use crate::managers::integrations::m365::mail::{
    MailAttachment, MailBody, MailMessage as GraphMail,
};
use crate::managers::integrations::m365::Acct;
use crate::managers::integrations::model::{GrantMode, Integration, Kind};
use crate::managers::integrations::services::caldav::{self, NewCalEvent};
use crate::managers::integrations::services::http::ApiRequest;
use crate::managers::integrations::services::ops::Account;
use crate::managers::integrations::services::registry::ServiceId;
use crate::managers::provenance::SubjectKind;

use super::action::{
    Action, EffectKind, GateEnv, GateView, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::app_actions::{previous_result, record, text_param, AppServices};
use super::catalog::{self, ActionSpec};
use super::integration_actions::recipients::{self, Rule, Sources};
use super::integration_actions::{db_err, integration_of, m365_error, m365_service, MailSend};

const MAX_BODY_CHARS: usize = 10_000;
const DEFAULT_MINUTES: u32 = 30;

fn spec_of(id: &str) -> &'static ActionSpec {
    catalog::action_spec(id).unwrap_or_else(|| panic!("Katalogeintrag {id} fehlt"))
}

fn permanent(s: impl Into<String>) -> StepError {
    StepError::Permanent(s.into())
}

/// Stabile Kennung einer Anfrage aus Lauf und Schritt (Graph `transactionId`).
fn transaction_id(key: &str, salt: &str) -> String {
    Sha256::digest(format!("{salt}|{key}").as_bytes())
        .iter()
        .take(24)
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// mail.draft
// ---------------------------------------------------------------------------

pub struct MailDraftAction {
    spec: &'static ActionSpec,
    send: MailSend,
    services: Arc<dyn AppServices>,
}

impl MailDraftAction {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("mail.draft"),
            send: MailSend::new(services.clone()),
            services,
        }
    }
}

fn earlier_draft(ctx: &RunCtx<'_>) -> Option<StepOutput> {
    let (_, prev) = previous_result(ctx, SubjectKind::Export, "mail_draft")?;
    let mut data = prev.unwrap_or_else(|| json!({}));
    data["draft"] = json!(true);
    data["reused"] = json!(true);
    Some(StepOutput::with_data(data).summary("Der Entwurf lag schon in Outlook (Wiederaufnahme)."))
}

impl Action for MailDraftAction {
    fn id(&self) -> &str {
        "mail.draft"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        self.send.validate(params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let built = self
            .send
            .build(env.conn, env.context, params, env.planning)?;
        if built.integration.kind != Kind::M365 {
            return Err(permanent(
                "Entwürfe gibt es nur mit einem Microsoft-365-Konto (Outlook).",
            ));
        }
        let mut view = built.view();
        // Ein Entwurf geht an niemanden: E3 greift nicht. Ein Anhang aus einem Ordner, dessen
        // Lesen „fragen“ verlangt, bleibt eine Freigabe wert.
        view.max_mode = built
            .attachments
            .iter()
            .any(|a| a.read_mode == GrantMode::Ask)
            .then_some(GrantMode::Ask);
        view.target = Some(format!("Entwurf an {}", recipients::target_of(&built.to)));
        Ok(Some(view))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if let Some(out) = earlier_draft(ctx) {
            return Ok(out);
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor der Entwurf angelegt wurde.".to_string(),
            ));
        }
        let conn = ctx.conn().map_err(db_err)?;
        let built = self.send.build(&conn, ctx.context, params, false)?;
        if built.integration.kind != Kind::M365 {
            return Err(permanent(
                "Entwürfe gibt es nur mit einem Microsoft-365-Konto (Outlook).",
            ));
        }
        built.check_bound(&conn, ctx)?;
        drop(conn);
        let svc = m365_service(&*self.services)?;
        let acct = Acct::from_integration(built.integration.clone())
            .map_err(|e| permanent(e.to_string()))?;
        let msg = GraphMail::new(
            &built.to,
            &[],
            &built.subject,
            MailBody::Text(built.body.clone()),
        )
        .and_then(|m| {
            m.with_attachments(
                built
                    .attachments
                    .iter()
                    .map(|a| MailAttachment {
                        name: a.name.clone(),
                        content_type: a.content_type.to_string(),
                        bytes: a.bytes.clone(),
                    })
                    .collect(),
            )
        })
        .map_err(|e| permanent(e.to_string()))?;
        let draft =
            tauri::async_runtime::block_on(svc.create_draft(&acct, &msg)).map_err(m365_error)?;
        let data = json!({
            "draft": true,
            "id": draft.id,
            "web_link": draft.web_link,
            "via": built.integration.id,
            "recipients": built.to.len(),
            "subject": built.subject,
            "attachments": built.attachments.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        });
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            "mail_draft",
            Vec::new(),
            data.clone(),
        );
        Ok(StepOutput::with_data(data).summary(&format!(
            "Entwurf „{}“ in Outlook angelegt (an {}); gesendet wird von Hand.",
            built.subject,
            recipients::target_of(&built.to)
        )))
    }

    fn confirm(&self, ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        earlier_draft(ctx)
    }
}

// ---------------------------------------------------------------------------
// calendar.followup
// ---------------------------------------------------------------------------

pub struct CalendarFollowup {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl CalendarFollowup {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("calendar.followup"),
            services,
        }
    }
}

/// Ein fertig gebildeter Folgetermin (gleich fuer Tor und Lauf).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Followup {
    pub(super) title: String,
    /// Ortszeit, wie der Nutzer sie eingab.
    pub(super) local: NaiveDateTime,
    pub(super) start_utc: chrono::DateTime<Utc>,
    pub(super) minutes: u32,
    pub(super) body: String,
    pub(super) invite: Option<Rule>,
    pub(super) attendees: Vec<String>,
}

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

/// Ortszeit -> UTC (Zeitzone des Rechners). Eine Zeit, die es wegen der Zeitumstellung nicht
/// gibt, ist ein Fehler; eine doppelte nimmt die fruehere.
pub(super) fn to_utc(local: NaiveDateTime) -> Result<chrono::DateTime<Utc>, StepError> {
    chrono::Local
        .from_local_datetime(&local)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| {
            permanent(format!(
                "Die Uhrzeit {} gibt es wegen der Zeitumstellung nicht.",
                local.format("%d.%m.%Y %H:%M")
            ))
        })
}

impl CalendarFollowup {
    fn build(&self, context: &Value, params: &Value) -> Result<Followup, StepError> {
        let title = one_line(
            params.get("title").and_then(Value::as_str).unwrap_or(""),
            255,
        );
        if title.is_empty() {
            return Err(permanent(
                "Der Folgetermin hat keinen Titel (Parameter title).",
            ));
        }
        let date_raw = text_param(params, "day")
            .ok_or_else(|| permanent("Der Tag fehlt (Parameter day, JJJJ-MM-TT)."))?;
        let date = NaiveDate::parse_from_str(date_raw, "%Y-%m-%d")
            .map_err(|_| permanent(format!("„{date_raw}“ ist kein Datum (JJJJ-MM-TT).")))?;
        let time_raw = text_param(params, "time").unwrap_or("09:00");
        let time = NaiveTime::parse_from_str(time_raw, "%H:%M")
            .map_err(|_| permanent(format!("„{time_raw}“ ist keine Uhrzeit (HH:MM).")))?;
        let minutes = match params.get("minutes") {
            None | Some(Value::Null) => DEFAULT_MINUTES,
            Some(v) => v
                .as_u64()
                .and_then(|m| u32::try_from(m).ok())
                .filter(|m| (MIN_MINUTES..=MAX_MINUTES).contains(m))
                .ok_or_else(|| {
                    permanent(format!(
                        "Die Dauer muss zwischen {MIN_MINUTES} und {MAX_MINUTES} Minuten liegen."
                    ))
                })?,
        };
        let body: String = params
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .filter(|c| *c != '\r')
            .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
            .collect::<String>()
            .trim()
            .to_string();
        if body.chars().count() > MAX_BODY_CHARS {
            return Err(permanent(format!(
                "Der Text des Termins ist zu lang (höchstens {MAX_BODY_CHARS} Zeichen)."
            )));
        }
        let invite = match text_param(params, "invite") {
            None | Some("none") => None,
            Some(r) => Some(Rule::parse(r).ok_or_else(|| {
                permanent(format!(
                    "Die Einladungsregel „{r}“ gibt es nicht (none, me, participants, all, internal, list)."
                ))
            })?),
        };
        let attendees = match invite {
            None => Vec::new(),
            Some(rule) => {
                let list: Vec<String> = params
                    .get("list")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let attendees = recipients::attendees_of(context);
                let self_emails = self.services.self_emails();
                recipients::resolve(
                    rule,
                    &Sources {
                        attendees: &attendees,
                        self_emails: &self_emails,
                        own_address: None,
                        list: &list,
                    },
                )
                .map_err(|e| permanent(e.to_string()))?
            }
        };
        let local = date.and_time(time);
        Ok(Followup {
            title,
            local,
            start_utc: to_utc(local)?,
            minutes,
            body,
            invite,
            attendees,
        })
    }
}

fn followup_view(i_id: &str, f: &Followup) -> GateView {
    let when = f.local.format("%d.%m.%Y um %H:%M").to_string();
    GateView {
        target: Some(format!("Folgetermin „{}“ am {when}", f.title)),
        args: json!({
            "via": i_id,
            "titel": f.title,
            "beginn": f.local.format("%Y-%m-%d %H:%M").to_string(),
            "minuten": f.minutes,
            "text": f.body,
            "eingeladen": f.attendees,
        }),
        // E3: Einladungen an andere als mich nur nach Freigabe.
        max_mode: f
            .invite
            .is_some_and(|r| r.reaches_others())
            .then_some(GrantMode::Ask),
    }
}

fn earlier_followup(ctx: &RunCtx<'_>) -> Option<StepOutput> {
    let (_, prev) = previous_result(ctx, SubjectKind::Export, "followup")?;
    let mut data = prev.unwrap_or_else(|| json!({}));
    data["reused"] = json!(true);
    Some(
        StepOutput::with_data(data)
            .summary("Der Folgetermin stand schon im Kalender (Wiederaufnahme)."),
    )
}

impl Action for CalendarFollowup {
    fn id(&self) -> &str {
        "calendar.followup"
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
        let i = integration_of(
            env.conn,
            params,
            FOLLOWUP_KINDS,
            "kein Kalender mit Schreibrecht",
        )?;
        if i.kind == Kind::Service && i.service() != Some(ServiceId::Icloud) {
            return Err(permanent(format!(
                "Die Integration „{}“ ist kein Kalender (Outlook oder iCloud).",
                i.id
            )));
        }
        match self.build(env.context, params) {
            Ok(f) => Ok(Some(followup_view(&i.id, &f))),
            // Im Trockenlauf stehen Datum oder Uhrzeit oft noch als `{{...}}` da.
            Err(_) if env.planning => Ok(Some(GateView {
                target: Some("Folgetermin (Zeit steht erst beim Lauf fest)".to_string()),
                args: json!({"via": i.id, "hinweis": "Datum und Uhrzeit werden beim Lauf eingesetzt und dann zur Freigabe vorgelegt."}),
                max_mode: None,
            })),
            Err(e) => Err(e),
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if let Some(out) = earlier_followup(ctx) {
            return Ok(out);
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor der Termin angelegt wurde.".to_string(),
            ));
        }
        let conn = ctx.conn().map_err(db_err)?;
        let i = integration_of(
            &conn,
            params,
            FOLLOWUP_KINDS,
            "kein Kalender mit Schreibrecht",
        )?;
        drop(conn);
        let f = self.build(ctx.context, params)?;
        let tx = transaction_id(&ctx.idempotency_key, "followup");
        let (id, link) = match i.kind {
            Kind::M365 => {
                let svc = m365_service(&*self.services)?;
                let acct =
                    Acct::from_integration(i.clone()).map_err(|e| permanent(e.to_string()))?;
                let ev = NewTimedEvent {
                    subject: &f.title,
                    start: f.start_utc,
                    minutes: f.minutes,
                    body: &f.body,
                    attendees: &f.attendees,
                    transaction_id: &tx,
                };
                tauri::async_runtime::block_on(svc.create_event(&acct, &ev)).map_err(m365_error)?
            }
            Kind::Service if i.service() == Some(ServiceId::Icloud) => {
                if !f.attendees.is_empty() {
                    return Err(permanent(
                        "Einladungen verschickt nur Outlook; für den iCloud-Kalender „Einladen“ leer lassen.",
                    ));
                }
                let c = self.icloud_event(ctx, &i, &f, &tx)?;
                (c, None)
            }
            _ => return Err(permanent("Diese Integration kann keine Termine anlegen.")),
        };
        let data = json!({
            "id": id,
            "web_link": link,
            "via": i.id,
            "start": f.local.format("%Y-%m-%d %H:%M").to_string(),
            "minutes": f.minutes,
            "invited": f.attendees.len(),
        });
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            "followup",
            Vec::new(),
            data.clone(),
        );
        let when = f.local.format("%d.%m.%Y %H:%M");
        Ok(
            StepOutput::with_data(data).summary(&if f.attendees.is_empty() {
                format!("Folgetermin „{}“ am {when} im Kalender angelegt.", f.title)
            } else {
                format!(
                    "Folgetermin „{}“ am {when} angelegt, {} eingeladen.",
                    f.title,
                    f.attendees.len()
                )
            }),
        )
    }

    fn confirm(&self, ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        earlier_followup(ctx)
    }
}

impl CalendarFollowup {
    /// iCloud: Termin per CalDAV (`PUT` mit `If-None-Match`, UID aus Lauf und Schritt).
    fn icloud_event(
        &self,
        ctx: &RunCtx<'_>,
        i: &Integration,
        f: &Followup,
        tx: &str,
    ) -> Result<String, StepError> {
        let token = match self.services.secret(i, "token") {
            Ok(Some(t)) if !t.trim().is_empty() => t,
            Ok(_) => {
                return Err(permanent(
                    "Für den iCloud-Kalender fehlt das app-spezifische Passwort. Bitte in den Integrationen neu eintragen.",
                ))
            }
            Err(e) => return Err(permanent(e)),
        };
        let settings: Map<String, Value> = serde_json::from_str(&i.config_json).unwrap_or_default();
        let acc = Account {
            service: ServiceId::Icloud,
            token: &token,
            settings: &settings,
        };
        let uid = format!("lva-{tx}@local-voice-ai");
        let ev = NewCalEvent {
            uid: &uid,
            summary: &f.title,
            description: &f.body,
            start: f.start_utc,
            minutes: f.minutes,
            stamp: Utc
                .timestamp_millis_opt(ctx.now_ms())
                .single()
                .unwrap_or_else(Utc::now),
        };
        let exec = super::service_actions::default_exec();
        let mut run = |r: ApiRequest| exec(&r);
        caldav::create_event(&acc, &ev, &mut run)
            .map(|c| c.id)
            .map_err(super::service_actions::step_err)
    }
}

/// Kalender, in die ein Folgetermin geschrieben werden kann: Microsoft 365 und der
/// iCloud-Kalender (Dienst `icloud`); welche Dienst-Integration es ist, prueft `run`.
const FOLLOWUP_KINDS: &[Kind] = &[Kind::M365, Kind::Service];

/// Die Bausteine dieses Moduls.
pub fn actions(services: Arc<dyn AppServices>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(MailDraftAction::new(services.clone())),
        Arc::new(CalendarFollowup::new(services)),
    ]
}
