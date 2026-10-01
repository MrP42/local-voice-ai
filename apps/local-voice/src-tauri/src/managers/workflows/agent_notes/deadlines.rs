//! Fristen aus dem Ergebnis von `agent.extract`: die Erinnerung (`deadline.remind`) und der
//! Kalendereintrag (`deadline.calendar`) (C4).
//!
//! Beide arbeiten auf derselben Auswahl: alle Fristen (und auf Wunsch die To-dos mit
//! Faelligkeitsdatum) des Extraktionsschritts, nach Tag und Text sortiert, ohne Doppelte, ohne
//! Eintraege, deren Tag schon vorbei ist, und auf Wunsch ohne Daten, die das Sprachmodell nur
//! geschaetzt hat. **Daten, die das Modell geschaetzt hat, werden nie still behandelt wie
//! gesicherte:** jede Mitteilung, jeder Termin und jede Vorschau nennt es.
//!
//! # `deadline.remind`: Zeitplan ueber den vorhandenen Takt
//!
//! Der Baustein hat kein eigenes Planungswerk. Er rechnet je Frist den Zeitpunkt
//! (`days_before` Tage vor dem Tag der Frist, `at` Uhr Ortszeit) und tut zweierlei:
//! Faelliges zeigt er (Windows-Mitteilung, `AppServices::notify`), fuer den naechsten Zeitpunkt
//! meldet er `Defer` bis dahin (hoechstens 6 Stunden am Stueck, damit Zeitumstellung und
//! Uhrenspruenge nie weit danebenliegen). Die Engine parkt den Lauf (`next_run_at`) und weckt ihn
//! mit ihrem Takt (`Engine::tick`): kein zweiter Poller, kein Zeitgeber im Baustein. Die Uhr ist
//! die der Engine (`RunCtx::now_ms`), im Test also eine feste Uhr.
//!
//! - **Verpasst**: war der Zeitpunkt schon vorbei, als der Baustein drankommt (App war aus,
//!   Besprechung spaet fertig), die Frist selbst aber noch nicht, kommt die Mitteilung sofort.
//!   Liegt der Tag der Frist hinter uns, gibt es keine Mitteilung, der Eintrag steht in
//!   `skipped` mit Grund.
//! - **Hoechstens einmal**: nach jeder gezeigten Mitteilung steht ein Eintrag im Herkunftsregister
//!   (`SubjectKind::RunOutput`, Operation `deadline_reminder_shown`, Gegenstand
//!   `deadline:<besprechung>:<kennung>`). Vor jeder Mitteilung wird nachgeschaut: dieselbe Frist
//!   derselben Besprechung erscheint nie ein zweites Mal, auch nicht aus einem zweiten Lauf oder
//!   nach einem Neustart.
//! - **Steht im Lauf ganz hinten**: er kann Tage warten. Gatepflichtige Schritte (Vault-Notiz,
//!   Kalender) gehoeren davor, sonst blockierte das Warten sie.
//!
//! # `deadline.calendar`: nur nach Freigabe
//!
//! Der Baustein wirkt nach aussen (`External`) und braucht `calendar.write` am Konto. Er begrenzt
//! das Recht ueber `GateView::max_mode` auf „fragen“: **jeder** Lauf legt die Liste der Eintraege
//! (Tag und Betreff, mit Kennzeichnung geschaetzter Daten) zur Freigabe vor, auch wenn der Nutzer
//! das Recht auf „erlaubt“ gestellt hat. Die Freigabe ist an genau diese Liste gebunden. Je Frist
//! entsteht ein ganztaegiger Termin ohne Teilnehmende (es geht keine Einladung hinaus), mit einer
//! `transactionId` aus Lauf, Schritt und Frist; dazu steht nach jedem Termin ein Eintrag im
//! Herkunftsregister (`deadline-cal:<konto>:<besprechung>:<kennung>`), der einen zweiten Termin
//! fuer dieselbe Frist verhindert. Hoechstens [`MAX_CALENDAR_EVENTS`] je Schritt.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, LocalResult, NaiveDate, TimeZone};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::managers::integrations::m365::event_create::NewAllDayEvent;
use crate::managers::integrations::m365::Acct;
use crate::managers::integrations::model::{GrantMode, Kind as IntegrationKind};
use crate::managers::provenance::{ActorKind, NewProvenance, SourceRef, SubjectKind};
use crate::managers::workflows::action::{
    Action, EffectKind, GateEnv, GateView, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use crate::managers::workflows::app_actions::{has_template, svc, text_param, AppServices};
use crate::managers::workflows::catalog::{self, ActionSpec};
use crate::managers::workflows::integration_actions::{
    db_err, integration_of, m365_error, m365_service,
};

use super::items::{self, clean, Extracted, Input, Item, Kind};
use super::render::{de_date, weekday_de};

/// Laengster Schlaf ohne neue Pruefung (Zeitumstellung, Uhrensprung).
pub const MAX_SLEEP_MS: u64 = 6 * 3_600_000;
pub const MIN_SLEEP_MS: u64 = 1_000;
pub const DEFAULT_AT: &str = "09:00";
pub const DEFAULT_DAYS_BEFORE: i64 = 1;
/// Hoechstzahl Kalendereintraege je Schritt (die Freigabe zeigt sie alle).
pub const MAX_CALENDAR_EVENTS: usize = 10;
const MAX_NOTIFY_TITLE_CHARS: usize = 80;
const MAX_NOTIFY_BODY_CHARS: usize = 240;
const MAX_EVENT_SOURCES: usize = 40;

// Nicht `deadline_remind` und `deadline_calendar`: so nennt die Engine ihren allgemeinen Eintrag je
// Schritt (Aktionskennung mit `_`), und die Zaehlung hier darf ihn nicht mitzaehlen.
pub const OP_REMIND: &str = "deadline_reminder_shown";
pub const OP_CALENDAR: &str = "deadline_event_created";

fn spec_of(id: &str) -> &'static ActionSpec {
    catalog::action_spec(id).unwrap_or_else(|| panic!("Katalogeintrag {id} fehlt"))
}

// ---------------------------------------------------------------------------
// Parameter
// ---------------------------------------------------------------------------

/// Gueltige Kennung eines Schritts (wie die Definition sie verlangt).
pub fn valid_step_id(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn from_of(params: &Value) -> Result<String, StepError> {
    let from = text_param(params, "from").unwrap_or("extract");
    if !valid_step_id(from) {
        return Err(StepError::Permanent(
            "„from“ muss die Kennung eines Schritts sein (Kleinbuchstaben, Ziffern, _)."
                .to_string(),
        ));
    }
    Ok(from.to_string())
}

fn flag(params: &Value, key: &str) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Zeitpunkt der Erinnerung relativ zur Frist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lead {
    pub days_before: i64,
    pub hour: u32,
    pub minute: u32,
}

/// `HH:MM` oder `H:MM` -> (Stunde, Minute).
pub fn parse_at(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

pub fn lead_of(params: &Value) -> Result<Lead, StepError> {
    let days_before = match params.get("days_before") {
        None | Some(Value::Null) => DEFAULT_DAYS_BEFORE,
        Some(v) => v.as_i64().filter(|d| (0..=30).contains(d)).ok_or_else(|| {
            StepError::Permanent("„days_before“ muss eine Zahl von 0 bis 30 sein.".to_string())
        })?,
    };
    let at = text_param(params, "at").unwrap_or(DEFAULT_AT);
    let (hour, minute) = parse_at(at).ok_or_else(|| {
        StepError::Permanent("„at“ muss eine Uhrzeit wie 09:00 sein.".to_string())
    })?;
    Ok(Lead {
        days_before,
        hour,
        minute,
    })
}

// ---------------------------------------------------------------------------
// Zeit
// ---------------------------------------------------------------------------

/// Der Tag in Ortszeit zu einem Zeitpunkt der Uhr der Engine.
pub fn local_day(now_ms: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp_millis(now_ms)
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// Wann die Erinnerung zu einer Frist faellig ist (Millisekunden UTC): `days_before` Tage vor
/// dem Tag der Frist, `hour:minute` Ortszeit. Faellt die Uhrzeit in eine Luecke der Zeitumstellung,
/// gilt die Stunde danach; ist sie doppelt, die erste.
pub fn fire_at_ms(due: NaiveDate, lead: &Lead) -> Option<i64> {
    let day = due.checked_sub_signed(ChronoDuration::days(lead.days_before))?;
    let naive = day.and_hms_opt(lead.hour, lead.minute, 0)?;
    match chrono::Local.from_local_datetime(&naive) {
        LocalResult::Single(t) => Some(t.timestamp_millis()),
        LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
        LocalResult::None => {
            match chrono::Local.from_local_datetime(&(naive + ChronoDuration::hours(1))) {
                LocalResult::Single(t) => Some(t.timestamp_millis()),
                LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
                LocalResult::None => None,
            }
        }
    }
}

fn when_label(days_left: i64) -> String {
    match days_left {
        0 => "heute".to_string(),
        1 => "morgen".to_string(),
        n => format!("in {n} Tagen"),
    }
}

fn local_time_label(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Auswahl und Herkunftsregister
// ---------------------------------------------------------------------------

/// Besprechung des Laufs: laut Extraktion, sonst `meeting.id` oder `trigger.meeting_id`.
pub fn meeting_id_in(context: &Value, ex: &Extracted) -> Result<String, StepError> {
    let id = if ex.meeting_id.is_empty() {
        context
            .pointer("/meeting/id")
            .and_then(Value::as_str)
            .or_else(|| {
                context
                    .pointer("/trigger/meeting_id")
                    .and_then(Value::as_str)
            })
            .unwrap_or("")
            .trim()
            .to_string()
    } else {
        ex.meeting_id.clone()
    };
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(StepError::Permanent(
            "Die Kennung der Besprechung fehlt oder ist ungültig.".to_string(),
        ));
    }
    Ok(id)
}

/// Die Eintraege mit Tag, die erinnert bzw. eingetragen werden sollen (siehe Moduldoku),
/// ohne die, deren Tag vor `today` liegt (die zaehlt der Aufrufer als verpasst).
pub fn dated_items(ex: &Extracted, todos: bool, skip_model_dates: bool) -> Vec<&Item> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<&Item> = ex
        .items
        .iter()
        .filter(|i| i.due.is_some())
        .filter(|i| i.kind == Kind::Deadline || (todos && i.kind == Kind::Todo))
        .filter(|i| !(skip_model_dates && i.unverified_date))
        .filter(|i| seen.insert(i.key()))
        .collect();
    out.sort_by(|a, b| {
        (a.due, a.text.as_str(), a.kind.as_str()).cmp(&(b.due, b.text.as_str(), b.kind.as_str()))
    });
    out
}

fn remind_subject(meeting: &str, item: &Item) -> String {
    format!("deadline:{meeting}:{}", item.key())
}

fn calendar_subject(via: &str, meeting: &str, item: &Item) -> String {
    format!("deadline-cal:{via}:{meeting}:{}", item.key())
}

fn ledger_has(conn: &Connection, subject: &str, operation: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM provenance WHERE subject_kind = ?1 AND subject_id = ?2 AND operation = ?3 LIMIT 1",
        rusqlite::params![SubjectKind::RunOutput.as_str(), subject, operation],
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
    .map_err(|e| e.to_string())
}

/// Wie viele Eintraege dieses Schritts (Akteur `workflow/lauf/schritt`) es fuer `operation` gibt.
fn ledger_count_of_step(ctx: &RunCtx<'_>, conn: &Connection, operation: &str) -> usize {
    let actor =
        crate::managers::workflows::action::actor_ref(ctx.workflow_id, ctx.run_id, ctx.step_id);
    conn.query_row(
        "SELECT COUNT(*) FROM provenance WHERE actor_ref = ?1 AND subject_kind = ?2 AND operation = ?3",
        rusqlite::params![actor, SubjectKind::RunOutput.as_str(), operation],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n.max(0) as usize)
    .unwrap_or(0)
}

/// Quellen eines Eintrags im Herkunftsregister: die Besprechung und die belegten Segmente.
fn sources_for(meeting_id: &str, title: Option<&str>, item: &Item) -> Vec<SourceRef> {
    let mut sources = vec![SourceRef::new("meeting", meeting_id, title)];
    sources.extend(
        item.segments
            .iter()
            .take(MAX_EVENT_SOURCES)
            .map(|n| SourceRef::new("segment", &format!("{meeting_id}:S{n}"), None)),
    );
    sources
}

/// Der Tag, an dem der Lauf begann (Ortszeit): fuer Freigabe und Ausfuehrung dieselbe Rechnung.
fn run_day(context: &Value, fallback_now_ms: i64) -> NaiveDate {
    let started = context
        .pointer("/run/started_at")
        .and_then(Value::as_i64)
        .filter(|t| *t > 0)
        .unwrap_or(fallback_now_ms);
    local_day(started)
}

fn record_entry(
    ctx: &RunCtx<'_>,
    subject: &str,
    operation: &str,
    kind: SubjectKind,
    item: &Item,
    sources: Vec<SourceRef>,
    params: Value,
) -> Result<(), StepError> {
    let mut entry = NewProvenance::new(kind, subject, operation, ActorKind::Workflow);
    entry.sources = sources;
    entry.confidence = item.confidence;
    entry.params = Some(params);
    ctx.record_provenance(entry).map(|_| ()).map_err(|e| {
        StepError::Transient(format!(
            "Der Vermerk im Herkunftsregister ließ sich nicht schreiben ({e}). Die Wirkung ist eingetreten, ein neuer Versuch ist sicher."
        ))
    })
}

fn input_of(context: &Value, from: &str) -> Result<Option<Box<Extracted>>, StepError> {
    match items::read(context, from)? {
        Input::Data(ex) => Ok(Some(ex)),
        Input::Nothing(_) => Ok(None),
    }
}

fn item_noun(i: &Item) -> &'static str {
    if i.kind == Kind::Todo {
        "To-do"
    } else {
        "Frist"
    }
}

// ---------------------------------------------------------------------------
// deadline.remind
// ---------------------------------------------------------------------------

pub struct DeadlineRemind {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl DeadlineRemind {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("deadline.remind"),
            services,
        }
    }

    fn meeting_title(&self, meeting_id: &str) -> Option<String> {
        let store = self.services.store().ok()?;
        let m = store.get_meeting(meeting_id).ok().flatten()?;
        Some(clean(&m.title, 80)).filter(|t| !t.is_empty())
    }
}

fn notification(item: &Item, days_left: i64, meeting_title: Option<&str>) -> (String, String) {
    let due = item.due.unwrap_or_default();
    let head = if item.kind == Kind::Todo {
        format!("To-do fällig {}: ", when_label(days_left))
    } else {
        format!("Frist {}: ", when_label(days_left))
    };
    let title = clean(&format!("{head}{}", item.text), MAX_NOTIFY_TITLE_CHARS);
    let mut body = format!("{}, {}.", weekday_de(due), de_date(due));
    if item.unverified_date {
        body.push_str(" Datum vom Sprachmodell geschätzt, bitte prüfen.");
    }
    if let Some(a) = &item.assignee {
        body.push_str(&format!(" Zuständig: {a}."));
    }
    if let Some(t) = meeting_title {
        body.push_str(&format!(" Besprechung „{t}“."));
    }
    (title, clean(&body, MAX_NOTIFY_BODY_CHARS))
}

impl Action for DeadlineRemind {
    fn id(&self) -> &str {
        "deadline.remind"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn validate(&self, params: &serde_json::Map<String, Value>) -> Result<(), String> {
        if let Some(Value::String(from)) = params.get("from") {
            if !has_template(from) && !valid_step_id(from) {
                return Err("from: Kennung eines Schritts erwartet".to_string());
            }
        }
        if let Some(Value::String(at)) = params.get("at") {
            if !has_template(at) && parse_at(at).is_none() {
                return Err("at: Uhrzeit wie 09:00 erwartet".to_string());
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let from = text_param(params, "from").unwrap_or("extract");
        let at = text_param(params, "at").unwrap_or(DEFAULT_AT);
        let days = params
            .get("days_before")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_DAYS_BEFORE);
        let when = match days {
            0 => format!("am Tag der Frist um {at} Uhr"),
            1 => format!("einen Tag vor der Frist um {at} Uhr"),
            n => format!("{n} Tage vor der Frist um {at} Uhr"),
        };
        let what = if flag(params, "todos") {
            "Fristen und To-dos mit Datum"
        } else {
            "Fristen"
        };
        let mut text = format!(
            "Zu den {what} aus Schritt „{from}“ {when} eine Windows-Mitteilung anzeigen; der Lauf wartet bis dahin"
        );
        if flag(params, "skip_model_dates") {
            text.push_str(" (ohne vom Sprachmodell geschätzte Daten)");
        }
        text
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let from = from_of(params)?;
        let lead = lead_of(params)?;
        let todos = flag(params, "todos");
        let skip_model = flag(params, "skip_model_dates");
        let Some(ex) = input_of(ctx.context, &from)? else {
            return Ok(StepOutput::with_data(json!({
                "reminders": 0, "sent": 0, "already_sent": 0, "skipped": []
            }))
            .summary("Keine Fristen: der Extraktionsschritt hat nichts geliefert."));
        };
        let meeting_id = meeting_id_in(ctx.context, &ex)?;
        let selected = dated_items(&ex, todos, skip_model);
        let conn = ctx.conn().map_err(db_err)?;
        let now = ctx.now_ms();
        let today = local_day(now);
        let title = self.meeting_title(&meeting_id);

        let mut skipped: Vec<Value> = Vec::new();
        let mut already_sent = 0usize;
        let mut waiting: Vec<(i64, &Item)> = Vec::new();
        for item in &selected {
            if ctx.cancelled() {
                return Err(StepError::Transient(
                    "Der Lauf wurde abgebrochen.".to_string(),
                ));
            }
            let Some(due) = item.due else { continue };
            let subject = remind_subject(&meeting_id, item);
            if ledger_has(&conn, &subject, OP_REMIND).map_err(db_err)? {
                already_sent += 1;
                continue;
            }
            if due < today {
                skipped.push(json!({
                    "text": item.text, "due": due.format("%Y-%m-%d").to_string(),
                    "reason": "Die Frist ist schon vorbei."
                }));
                continue;
            }
            match fire_at_ms(due, &lead) {
                Some(at) if at > now => waiting.push((at, item)),
                _ => {
                    // Faellig (oder verpasst, aber die Frist steht noch aus): jetzt zeigen.
                    let days_left = (due - today).num_days();
                    let (t, b) = notification(item, days_left, title.as_deref());
                    self.services.notify(&t, &b).map_err(svc)?;
                    record_entry(
                        ctx,
                        &subject,
                        OP_REMIND,
                        SubjectKind::RunOutput,
                        item,
                        sources_for(&meeting_id, title.as_deref(), item),
                        json!({
                            "kind": item.kind.as_str(),
                            "key": item.key(),
                            "due": due.format("%Y-%m-%d").to_string(),
                            "fire_at": fire_at_ms(due, &lead),
                            "days_left": days_left,
                            "unverified_date": item.unverified_date,
                        }),
                    )?;
                }
            }
        }

        if let Some((next_at, item)) = waiting.iter().min_by_key(|(at, _)| *at) {
            let wait = (*next_at - now).max(0) as u64;
            let more = waiting.len() - 1;
            return Err(StepError::Defer {
                retry_after_ms: wait.clamp(MIN_SLEEP_MS, MAX_SLEEP_MS),
                reason: format!(
                    "Wartet bis {} auf die Erinnerung an {} „{}“{}.",
                    local_time_label(*next_at),
                    item_noun(item),
                    clean(&item.text, 60),
                    if more > 0 {
                        format!(" und {more} weitere")
                    } else {
                        String::new()
                    }
                ),
            });
        }

        let sent = ledger_count_of_step(ctx, &conn, OP_REMIND);
        let summary = format!(
            "{sent} Erinnerung(en) gezeigt{}{}.",
            if already_sent > 0 {
                format!(", {already_sent} schon früher gezeigt")
            } else {
                String::new()
            },
            if skipped.is_empty() {
                String::new()
            } else {
                format!(", {} vorbei übersprungen", skipped.len())
            }
        );
        Ok(StepOutput::with_data(json!({
            "reminders": selected.len(),
            "sent": sent,
            "already_sent": already_sent,
            "skipped": skipped,
        }))
        .summary(&summary))
    }
}

// ---------------------------------------------------------------------------
// deadline.calendar
// ---------------------------------------------------------------------------

pub struct DeadlineCalendar {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl DeadlineCalendar {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("deadline.calendar"),
            services,
        }
    }
}

fn event_subject(item: &Item) -> String {
    let head = match (item.kind, item.unverified_date) {
        (Kind::Todo, false) => "To-do fällig: ",
        (Kind::Todo, true) => "To-do fällig (Datum geschätzt): ",
        (_, false) => "Frist: ",
        (_, true) => "Frist (Datum geschätzt): ",
    };
    clean(&format!("{head}{}", item.text), 200)
}

fn event_body(item: &Item, meeting_title: &str, meeting_date: &str, model: Option<&str>) -> String {
    let mut lines = vec![format!(
        "{} aus der Besprechung „{meeting_title}“{}.",
        item_noun(item),
        if meeting_date.is_empty() {
            String::new()
        } else {
            format!(" vom {meeting_date}")
        }
    )];
    if let Some(a) = &item.assignee {
        lines.push(format!("Zuständig: {a}"));
    }
    if item.unverified_date {
        lines.push("Das Datum hat das Sprachmodell geschätzt. Bitte prüfen.".to_string());
    } else if let Some(p) = &item.due_phrase {
        lines.push(format!("Datum aus der Angabe „{p}“."));
    }
    if !item.quote.is_empty() {
        lines.push(format!("Beleg: „{}“", item.quote));
    }
    lines.push(format!(
        "Automatisch erzeugt von Local Voice AI{}. Kennung: {}",
        model.map(|m| format!(" (Modell {m})")).unwrap_or_default(),
        item.key()
    ));
    lines.join("\n")
}

fn transaction_id(idempotency_key: &str, item: &Item) -> String {
    let digest = Sha256::digest(format!("{idempotency_key}|{}", item.key()).as_bytes());
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Was `deadline.calendar` heute eintragen wuerde: Eintraege mit Tag ab heute, noch ohne Termin.
fn calendar_candidates<'a>(
    ex: &'a Extracted,
    conn: &Connection,
    via: &str,
    meeting_id: &str,
    todos: bool,
    skip_model: bool,
    today: NaiveDate,
) -> Result<(Vec<&'a Item>, usize, usize), StepError> {
    let mut fresh: Vec<&Item> = Vec::new();
    let mut existing = 0usize;
    for item in dated_items(ex, todos, skip_model) {
        let Some(due) = item.due else { continue };
        if due < today {
            continue;
        }
        if ledger_has(conn, &calendar_subject(via, meeting_id, item), OP_CALENDAR)
            .map_err(db_err)?
        {
            existing += 1;
            continue;
        }
        fresh.push(item);
    }
    let overflow = fresh.len().saturating_sub(MAX_CALENDAR_EVENTS);
    fresh.truncate(MAX_CALENDAR_EVENTS);
    Ok((fresh, existing, overflow))
}

impl Action for DeadlineCalendar {
    fn id(&self) -> &str {
        "deadline.calendar"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn validate(&self, params: &serde_json::Map<String, Value>) -> Result<(), String> {
        if let Some(Value::String(from)) = params.get("from") {
            if !has_template(from) && !valid_step_id(from) {
                return Err("from: Kennung eines Schritts erwartet".to_string());
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let from = text_param(params, "from").unwrap_or("extract");
        let via = text_param(params, "via").unwrap_or("…");
        format!(
            "Fristen aus Schritt „{from}“ als ganztägige Termine über {via} in den Kalender eintragen, immer erst nach Ihrer Freigabe (mit der Liste der Einträge)"
        )
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let from = from_of(params)?;
        let via = text_param(params, "via").unwrap_or_default().to_string();
        let name = integration_of(
            env.conn,
            params,
            &[IntegrationKind::M365],
            "kein Microsoft-365-Konto",
        )
        .map(|i| clean(&i.label, 60))
        .unwrap_or_else(|_| via.clone());
        // Immer „fragen“, egal was der Nutzer eingestellt hat.
        let ask = Some(GrantMode::Ask);
        // Trockenlauf: das Ergebnis des Extraktionsschritts gibt es noch nicht.
        let planned = env.planning
            && env
                .context
                .pointer(&format!("/steps/{from}/outcome"))
                .is_none();
        if planned {
            return Ok(Some(GateView {
                target: Some(format!("Kalender „{name}“: Fristen aus Schritt „{from}“")),
                args: json!({
                    "via": via,
                    "hinweis": "Die Liste der Einträge steht erst beim Lauf fest und wird dann zur Freigabe vorgelegt."
                }),
                max_mode: ask,
            }));
        }
        let Some(ex) = input_of(env.context, &from)? else {
            return Ok(Some(GateView {
                target: Some(format!("Kalender „{name}“: keine Fristen")),
                args: json!({"via": via, "fristen": []}),
                max_mode: ask,
            }));
        };
        let meeting_id = meeting_id_in(env.context, &ex)?;
        let today = run_day(env.context, chrono::Utc::now().timestamp_millis());
        let (fresh, existing, overflow) = calendar_candidates(
            &ex,
            env.conn,
            &via,
            &meeting_id,
            flag(params, "todos"),
            flag(params, "skip_model_dates"),
            today,
        )?;
        let list: Vec<String> = fresh
            .iter()
            .map(|i| {
                format!(
                    "{}: {}",
                    de_date(i.due.unwrap_or_default()),
                    event_subject(i)
                )
            })
            .collect();
        let mut args = json!({"via": via, "fristen": list});
        if existing > 0 {
            args["schon_eingetragen"] = json!(existing);
        }
        if overflow > 0 {
            args["nicht_eingetragen_wegen_Obergrenze"] = json!(overflow);
        }
        Ok(Some(GateView {
            target: Some(format!(
                "Kalender „{name}“: {} ganztägige(r) Eintrag/Einträge",
                fresh.len()
            )),
            args,
            max_mode: ask,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let from = from_of(params)?;
        let Some(ex) = input_of(ctx.context, &from)? else {
            return Ok(StepOutput::with_data(json!({"created": 0, "events": []}))
                .summary("Keine Fristen: der Extraktionsschritt hat nichts geliefert."));
        };
        let meeting_id = meeting_id_in(ctx.context, &ex)?;
        let conn = ctx.conn().map_err(db_err)?;
        let integration = integration_of(
            &conn,
            params,
            &[IntegrationKind::M365],
            "kein Microsoft-365-Konto",
        )?;
        let now = ctx.now_ms();
        let (fresh, existing, overflow) = calendar_candidates(
            &ex,
            &conn,
            &integration.id,
            &meeting_id,
            flag(params, "todos"),
            flag(params, "skip_model_dates"),
            run_day(ctx.context, now),
        )?;
        if fresh.is_empty() {
            return Ok(StepOutput::with_data(json!({
                "created": 0, "events": [], "already_there": existing
            }))
            .summary("Alle Fristen stehen schon im Kalender oder es gibt keine."));
        }
        let store = self.services.store().map_err(svc)?;
        let meeting = store
            .get_meeting(&meeting_id)
            .map_err(|e| {
                StepError::Transient(format!("Die Besprechung ließ sich nicht lesen ({e})."))
            })?
            .filter(|m| m.deleted_at.is_none())
            .ok_or_else(|| {
                StepError::Permanent("Die Besprechung gibt es nicht mehr.".to_string())
            })?;
        let meeting_title = clean(&meeting.title, 80);
        let meeting_date = ex.meeting_date.map(de_date).unwrap_or_default();
        let svc365 = m365_service(&*self.services)?;
        let acct = Acct::from_integration(integration.clone())
            .map_err(|e| StepError::Permanent(e.to_string()))?;

        let mut created: Vec<Value> = Vec::new();
        for item in fresh {
            if ctx.cancelled() {
                return Err(StepError::Transient(
                    "Der Lauf wurde abgebrochen, bevor alle Termine angelegt waren.".to_string(),
                ));
            }
            let due = item.due.unwrap_or_default();
            let subject = event_subject(item);
            let body = event_body(
                item,
                &meeting_title,
                &meeting_date,
                ex.origin.model.as_deref(),
            );
            let tx = transaction_id(&ctx.idempotency_key, item);
            let ev = NewAllDayEvent {
                subject: &subject,
                date: due,
                body: &body,
                transaction_id: &tx,
            };
            let id = tauri::async_runtime::block_on(svc365.create_all_day_event(&acct, &ev))
                .map_err(m365_error)?;
            record_entry(
                ctx,
                &calendar_subject(&integration.id, &meeting_id, item),
                OP_CALENDAR,
                SubjectKind::RunOutput,
                item,
                sources_for(&meeting_id, Some(&meeting_title), item),
                json!({
                    "via": integration.id,
                    "kind": item.kind.as_str(),
                    "key": item.key(),
                    "due": due.format("%Y-%m-%d").to_string(),
                    "event_id": id.chars().take(120).collect::<String>(),
                    "unverified_date": item.unverified_date,
                }),
            )?;
            created.push(json!({
                "due": due.format("%Y-%m-%d").to_string(),
                "subject": subject,
                "unverified_date": item.unverified_date,
            }));
        }
        let n = created.len();
        Ok(StepOutput::with_data(json!({
            "created": n,
            "events": created,
            "already_there": existing,
            "not_entered_over_limit": overflow,
        }))
        .summary(&format!("{n} ganztägige(n) Termin(e) angelegt.")))
    }

    /// Nach einem Absturz: stehen alle Eintraege schon im Herkunftsregister, ist die Wirkung
    /// eingetreten.
    fn confirm(&self, ctx: &RunCtx<'_>, params: &Value) -> Option<StepOutput> {
        let from = from_of(params).ok()?;
        let ex = input_of(ctx.context, &from).ok()??;
        let meeting_id = meeting_id_in(ctx.context, &ex).ok()?;
        let via = text_param(params, "via")?;
        let conn = ctx.conn().ok()?;
        let all = dated_items(&ex, flag(params, "todos"), flag(params, "skip_model_dates"));
        let today = run_day(ctx.context, ctx.now_ms());
        let open: Vec<&&Item> = all
            .iter()
            .filter(|i| i.due.is_some_and(|d| d >= today))
            .take(MAX_CALENDAR_EVENTS)
            .collect();
        if open.is_empty() {
            return None;
        }
        for item in &open {
            if !ledger_has(
                &conn,
                &calendar_subject(via, &meeting_id, item),
                OP_CALENDAR,
            )
            .ok()?
            {
                return None;
            }
        }
        Some(
            StepOutput::with_data(json!({"created": 0, "events": [], "already_there": open.len()}))
                .summary("Die Termine standen schon im Kalender (Wiederaufnahme)."),
        )
    }
}
