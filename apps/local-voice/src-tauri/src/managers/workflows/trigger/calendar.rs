//! Ausloeser „Termin beginnt“ und „Termin endet“ (B2, AK3).
//!
//! **Derselbe Takt wie die Erinnerung.** `CalendarService::remind_tick` (alle 15 s) ruft
//! nach dem Zusammenstellen seiner Daten `WorkflowHub::on_tick`, und der ruft `on_tick`
//! dieses Moduls. Es gibt hier keinen Thread und keinen Zeitgeber. Auch die ZEITREGEL ist
//! keine Kopie: `reminder::in_start_window` (Vorlauf, 2 min Nachlauf, nie ganztaegig, nie
//! abgesagt) und `reminder::meeting_like` (Granola-Regel „Besprechung“) sind die Funktionen,
//! die die Erinnerung selbst benutzt.
//!
//! **Genau einmal je Termin und Ablauf.** Der Schluessel ist `calendar_start:<termin-key>`
//! (`<quelle>:<uid>:<beginn-ms>`, stabil ueber Abrufe; eine Serie hat je Termin einen).
//! Die Engine macht daraus hoechstens einen Lauf je Ablauf, auch ueber Neustart, zwei Takte
//! und zwei Threads (`UNIQUE (workflow_id, trigger_key)`). Die Merker der Erinnerung
//! (`reminded_at`, `dismissed_at`) werden NICHT angefasst: eine Erinnerung verbraucht den
//! Ausloeser nicht, ein Ausloeser nicht die Erinnerung.
//!
//! **Nicht waehrend einer laufenden Aufnahme.** Ist beim Takt eine Aufnahme aktiv, startet
//! „Termin beginnt“ nicht, und der Termin wird fuer diesen Ablauf im Speicher vermerkt:
//! endet die Aufnahme innerhalb des Nachlaufs (der Nutzer hat den Termin von Hand
//! aufgenommen und stoppt zwei Minuten nach dem Beginn), feuert der Ablauf nicht nachtraeglich
//! und bittet nicht um eine zweite Aufnahme desselben Termins. Der Vermerk ist bewusst
//! fluechtig und begrenzt (256): nach einem Neustart gilt hoechstens noch der Nachlauf.
//! „Termin endet“ kennt die Ausnahme nicht: am Ende laeuft die Aufnahme ja gerade.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::managers::calendar::model::CalEvent;
use crate::managers::calendar::reminder::{self, distinct_attendees};
use crate::managers::meetings::store::MeetingStore;

use super::super::model::TriggerDef;
use super::{enabled_with, fire, iso, RunSink, TickReport};

pub const KIND_START: &str = "calendar.event_starting";
pub const KIND_END: &str = "calendar.event_ended";
/// Vorlauf, wenn `lead_min` fehlt: wie die Erinnerung (1 min), damit die Bitte um die
/// Einwilligung vor dem Beginn auf dem Bildschirm steht.
pub const DEFAULT_LEAD_MIN: i64 = 1;
/// Hoechstzahl Teilnehmende in `trigger.attendees` (die Ausloeserdaten sind auf 64 KiB
/// begrenzt; ein Termin mit 500 Eingeladenen darf keinen Lauf verhindern).
pub const MAX_ATTENDEES: usize = 100;
const MAX_TITLE_CHARS: usize = 500;
/// So viele „uebersprungen“-Vermerke (Ablauf, Termin) merkt sich der Takt.
const MAX_SKIPPED: usize = 256;

/// Die Filter eines Kalender-Ausloesers aus der Definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// Kennung der Kalender-Integration = Kennung der Kalenderquelle.
    pub integration: String,
    /// Vorlauf in ms (nur „beginnt“).
    pub lead_ms: i64,
    /// Nur Besprechungen (Beitritts-Adresse, mindestens zwei Personen, Quelle ohne
    /// Teilnehmerdaten) und keine Einzeltermine; Vorgabe ja.
    pub only_meetings: bool,
    /// Kleingeschrieben; leer = kein Filter.
    pub title_contains: Option<String>,
    /// Mindestens so viele verschiedene Teilnehmende (0 = kein Filter).
    pub min_attendees: usize,
}

impl Filter {
    pub fn from_def(t: &TriggerDef) -> Option<Self> {
        let integration = t.params.get("integration")?.as_str()?.to_string();
        let lead_min = t
            .params
            .get("lead_min")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_LEAD_MIN)
            .max(0);
        Some(Self {
            integration,
            lead_ms: lead_min * 60_000,
            only_meetings: t
                .params
                .get("only_meetings")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            title_contains: t
                .params
                .get("title_contains")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty()),
            min_attendees: t
                .params
                .get("min_attendees")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .max(0) as usize,
        })
    }

    /// Passt der Termin zu den Filtern (ohne Zeitfenster)?
    pub fn matches(&self, e: &CalEvent, attendee_data: &dyn Fn(&str) -> bool) -> bool {
        e.source_id == self.integration
            && (!self.only_meetings || reminder::meeting_like(e, attendee_data))
            && self
                .title_contains
                .as_deref()
                .is_none_or(|t| e.title.to_lowercase().contains(t))
            && distinct_attendees(e) >= self.min_attendees
    }
}

/// Liegt das Ende des Termins im Fenster `[Ende, Ende + Nachlauf)`? (Gegenstueck zu
/// `reminder::in_start_window`.)
pub fn in_end_window(e: &CalEvent, now_ms: i64) -> bool {
    !e.all_day && !e.cancelled && e.ends_at <= now_ms && now_ms < e.ends_at + reminder::CATCH_UP_MS
}

/// Die Termine, fuer die dieser Ausloeser jetzt faellig ist (rein: keine Uhr, keine
/// Datenbank). `start`: „beginnt“, sonst „endet“.
pub fn due<'e>(
    events: &'e [CalEvent],
    filter: &Filter,
    start: bool,
    now_ms: i64,
    attendee_data: &dyn Fn(&str) -> bool,
) -> Vec<&'e CalEvent> {
    let mut out: Vec<&CalEvent> = events
        .iter()
        .filter(|e| {
            let in_window = if start {
                reminder::in_start_window(e, filter.lead_ms, now_ms)
            } else {
                in_end_window(e, now_ms)
            };
            in_window && filter.matches(e, attendee_data)
        })
        .collect();
    out.sort_by(|a, b| {
        a.starts_at
            .cmp(&b.starts_at)
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

fn domain_of(email: &str) -> Option<String> {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
}

/// Die Daten des Ausloesers (`trigger.*`). Keine Beitritts-Adresse und keine Beschreibung:
/// beides kann einen Zugang enthalten, und der Laufkontext steht im Laufprotokoll.
///
/// `external_attendees`: verschiedene Teilnehmende mit Adresse, die weder man selbst
/// (`self_emails`, Einstellung „Meine E-Mail-Adressen“) sind noch zu einer Domaene der
/// eigenen Adressen gehoeren. Ohne eigene Adressen ist jede andere Person extern.
pub fn trigger_data(e: &CalEvent, self_emails: &[String]) -> Value {
    let own: BTreeSet<String> = self_emails
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect();
    let own_domains: BTreeSet<String> = own.iter().filter_map(|m| domain_of(m)).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut external: BTreeSet<String> = BTreeSet::new();
    let mut attendees: Vec<Value> = Vec::new();
    for a in &e.attendees {
        let email = a
            .email
            .as_deref()
            .map(|m| m.trim().to_lowercase())
            .filter(|m| !m.is_empty());
        let is_self = a.is_self || email.as_ref().is_some_and(|m| own.contains(m));
        if let Some(m) = &email {
            if !seen.insert(m.clone()) {
                continue;
            }
            if !is_self && !domain_of(m).is_some_and(|d| own_domains.contains(&d)) {
                external.insert(m.clone());
            }
        }
        if attendees.len() < MAX_ATTENDEES {
            attendees.push(json!({
                "email": email,
                "name": a.name,
                "is_self": is_self,
            }));
        }
    }
    let title: String = e.title.chars().take(MAX_TITLE_CHARS).collect();
    json!({
        "calendar": e.source_id,
        "event_id": e.key,
        "title": title,
        "start": iso(e.starts_at),
        "end": iso(e.ends_at),
        "attendees": attendees,
        "external_attendees": external.len(),
        "online": e.join_url.as_deref().is_some_and(|u| !u.trim().is_empty()),
        "meeting": {"title": title},
    })
}

/// Merker der Takte (fluechtig, begrenzt): Termine, die wegen einer laufenden Aufnahme
/// fuer einen Ablauf uebersprungen wurden.
#[derive(Default)]
pub struct State {
    skipped: Mutex<VecDeque<(String, String)>>,
}

impl State {
    fn is_skipped(&self, workflow_id: &str, event_key: &str) -> bool {
        self.skipped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(w, k)| w == workflow_id && k == event_key)
    }

    fn skip(&self, workflow_id: &str, event_key: &str) {
        let mut q = self.skipped.lock().unwrap_or_else(|e| e.into_inner());
        if q.iter().any(|(w, k)| w == workflow_id && k == event_key) {
            return;
        }
        if q.len() >= MAX_SKIPPED {
            q.pop_front();
        }
        q.push_back((workflow_id.to_string(), event_key.to_string()));
    }

    pub fn skipped_len(&self) -> usize {
        self.skipped.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Was der Takt ueber die Welt weiss.
pub struct TickInput<'a> {
    pub now_ms: i64,
    /// Laeuft eine Aufnahme?
    pub recording: bool,
    /// Einstellung „Meine E-Mail-Adressen“.
    pub self_emails: &'a [String],
}

/// Ein Takt: prueft die Termine des Zeitfensters gegen alle eingeschalteten Ablaeufe mit
/// einem Kalender-Ausloeser und reiht die faelligen Laeufe ein.
pub fn on_tick(
    sink: &dyn RunSink,
    store: &MeetingStore,
    state: &State,
    input: &TickInput<'_>,
) -> TickReport {
    let mut report = TickReport::default();
    let armed = match enabled_with(sink, &[KIND_START, KIND_END]) {
        Ok(a) => a,
        Err(e) => {
            report.errors.push(format!("Ablaeufe: {e}"));
            return report;
        }
    };
    let jobs: Vec<_> = armed
        .iter()
        .filter_map(|a| {
            Filter::from_def(&a.def.trigger).map(|f| (a, f, a.def.trigger.kind == KIND_START))
        })
        .collect();
    if jobs.is_empty() {
        return report;
    }
    let max_lead = jobs
        .iter()
        .filter(|(_, _, start)| *start)
        .map(|(_, f, _)| f.lead_ms)
        .max()
        .unwrap_or(0);
    let from = input.now_ms - reminder::CATCH_UP_MS - 1_000;
    let to = input.now_ms + max_lead + 1_000;
    let events = match store.calendar_events_between(from, to, false) {
        Ok(e) => e,
        Err(e) => {
            report.errors.push(format!("Kalender: {e}"));
            return report;
        }
    };
    if events.is_empty() {
        return report;
    }
    let with_data: BTreeSet<String> = match store.calendar_sources() {
        Ok(s) => s
            .into_iter()
            .filter(|s| s.has_attendee_data)
            .map(|s| s.id)
            .collect(),
        Err(e) => {
            report.errors.push(format!("Kalenderquellen: {e}"));
            return report;
        }
    };
    let attendee_data = |source_id: &str| with_data.contains(source_id);

    for (armed, filter, start) in jobs {
        let wf = armed.row.id.as_str();
        for e in due(&events, &filter, start, input.now_ms, &attendee_data) {
            if start {
                if state.is_skipped(wf, &e.key) {
                    continue;
                }
                if input.recording {
                    state.skip(wf, &e.key);
                    log::info!(
                        "workflows: Termin {} startet Ablauf {wf} nicht: es laeuft schon eine Aufnahme",
                        e.key
                    );
                    report
                        .skipped
                        .push(format!("{wf}: Aufnahme läuft ({})", e.key));
                    continue;
                }
            }
            let prefix = if start {
                "calendar_start"
            } else {
                "calendar_end"
            };
            fire(
                sink,
                &mut report,
                wf,
                format!("{prefix}:{}", e.key),
                trigger_data(e, input.self_emails),
            );
        }
    }
    report
}

#[cfg(test)]
mod tests;
