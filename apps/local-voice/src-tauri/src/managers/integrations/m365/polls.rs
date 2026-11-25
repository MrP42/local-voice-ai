//! Graph-Transport fuer TF2. Keine Mailinhalte oder Delta-Adressen in Fehlern.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::{Acct, M365Error, M365Service};
use super::service::{error_of, Reply};
use crate::managers::integrations::model::Capability;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum Availability {
    #[serde(rename = "free")]
    Free,
    #[serde(rename = "tentative")]
    Tentative,
    #[serde(rename = "busy")]
    Busy,
    #[serde(rename = "oof")]
    Oof,
    #[serde(rename = "workingElsewhere")]
    WorkingElsewhere,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PersonAvailability {
    pub address: String,
    pub intervals: Vec<Availability>,
}

/// Lokale ISO-Zeitpunkte und IANA-/Windows-Zone der App; keine implizite UTC-
/// Umdeutung. Der Aufrufer liefert die Zone wie bei der Auswahl im Raster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ScheduleWindow {
    pub start: String,
    pub end: String,
    pub time_zone: String,
}

impl ScheduleWindow {
    pub fn intervals(&self) -> Result<usize, M365Error> {
        let parse = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
            .map_err(|_| invalid("Zeitfenster"));
        let seconds = (parse(&self.end)? - parse(&self.start)?).num_seconds();
        if seconds <= 0 || seconds > 31 * 86400 || seconds % 1800 != 0
            || self.time_zone.is_empty() || self.time_zone.len() > 128
            || !self.time_zone.bytes().all(|b| b.is_ascii_alphanumeric() || b"/ ._-+".contains(&b))
        {
            return Err(invalid("Zeitfenster"));
        }
        Ok((seconds / 1800) as usize)
    }
}

fn invalid(what: &str) -> M365Error { M365Error::Invalid(what.into()) }
fn malformed() -> M365Error { M365Error::Parse("Terminfindung: ungueltige Graph-Antwort".into()) }

pub(crate) fn parse_schedule(v: &Value, addresses: &[String], window: &ScheduleWindow) -> Result<Vec<PersonAvailability>, M365Error> {
    let count = window.intervals()?;
    let rows = v.get("value").and_then(Value::as_array).ok_or_else(malformed)?;
    Ok(addresses.iter().map(|address| {
        let matching: Vec<_> = rows.iter().filter(|r| r.get("scheduleId").and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case(address))).collect();
        let mut intervals = vec![Availability::Unknown; count];
        if matching.len() == 1 && matching[0].get("error").is_none_or(Value::is_null) {
            if let Some(view) = matching[0].get("availabilityView").and_then(Value::as_str).filter(|s| s.len() == count) {
                intervals = view.bytes().map(|b| match b {
                    b'0' => Availability::Free, b'1' => Availability::Tentative,
                    b'2' => Availability::Busy, b'3' => Availability::Oof,
                    b'4' => Availability::WorkingElsewhere, _ => Availability::Unknown,
                }).collect();
                // Graph fasst workingElsewhere im availabilityView unter 0 zusammen.
                // Nur den Status und die Zeiten lesen; Betreff/Ort niemals uebernehmen.
                if let Some(items) = matching[0].get("scheduleItems").and_then(Value::as_array) {
                    let start = chrono::NaiveDateTime::parse_from_str(&window.start, "%Y-%m-%dT%H:%M:%S").ok();
                    for item in items.iter().filter(|i| i.get("status").and_then(Value::as_str) == Some("workingElsewhere")) {
                        let time = |key: &str| item.pointer(key).and_then(Value::as_str)
                            .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok());
                        if let (Some(start), Some(a), Some(b)) = (start, time("/start/dateTime"), time("/end/dateTime")) {
                            for (n, state) in intervals.iter_mut().enumerate() {
                                let left = start + chrono::Duration::minutes(n as i64 * 30);
                                if a < left + chrono::Duration::minutes(30) && b > left && *state == Availability::Free {
                                    *state = Availability::WorkingElsewhere;
                                }
                            }
                        }
                    }
                }
            }
        }
        PersonAvailability { address: address.clone(), intervals }
    }).collect())
}

/// Nicht Debug/Serialize: die Seite enthaelt fluechtige Mailtexte und ein Geheimnis.
pub struct DeltaPage {
    pub messages: Vec<Value>,
    pub cursor: String,
    pub finished: bool,
}

pub(crate) fn validate_delta_link(base: &str, link: &str) -> Result<(), M365Error> {
    let expected = reqwest::Url::parse(&format!("{}/me/mailFolders/inbox/messages/delta", base.trim_end_matches('/'))).map_err(|_| malformed())?;
    let url = reqwest::Url::parse(link).map_err(|_| malformed())?;
    if url.origin() != expected.origin() || url.path() != expected.path()
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
    { return Err(malformed()); }
    Ok(())
}

pub(crate) fn parse_delta(mut v: Value, base: &str) -> Result<DeltaPage, M365Error> {
    let next = v.get("@odata.nextLink").and_then(Value::as_str);
    let delta = v.get("@odata.deltaLink").and_then(Value::as_str);
    let (cursor, finished) = match (next, delta) {
        (Some(s), None) => (s.to_string(), false),
        (None, Some(s)) => (s.to_string(), true),
        _ => return Err(malformed()),
    };
    validate_delta_link(base, &cursor)?;
    let messages = v.get_mut("value").and_then(Value::as_array_mut).map(std::mem::take).ok_or_else(malformed)?;
    Ok(DeltaPage { messages, cursor, finished })
}

impl M365Service {
    fn poll_reply(&self, a: &Acct, reply: Reply) -> Result<Value, M365Error> {
        if reply.is_success() { return reply.json(); }
        // Keine Microsoft-Beschreibung uebernehmen: sie kann die Cursor-URL enthalten.
        let error = error_of(&reply, "Terminfindung");
        if let M365Error::Throttled { retry_after } = error {
            self.graph.throttle(&a.integ.id, retry_after);
            return Err(M365Error::Throttled { retry_after });
        }
        Err(match reply.status {
            401 => M365Error::NeedsSignIn,
            403 => M365Error::Denied("Terminfindung".into()),
            status => M365Error::Http { status, code: String::new() },
        })
    }

    fn poll_ready(&self, a: &Acct, cap: Capability) -> Result<(), M365Error> {
        a.require(cap)?;
        if !a.integ.enabled || !a.integ.direction.permits(cap.access()) { return Err(invalid("Integration ausgeschaltet")); }
        if let Some(retry_after) = self.graph.throttle_left(&a.integ.id) { return Err(M365Error::Throttled { retry_after }); }
        Ok(())
    }

    pub async fn poll_schedule(&self, a: &Acct, own: &str, addresses: &[String], window: &ScheduleWindow) -> Result<Vec<PersonAvailability>, M365Error> {
        self.poll_ready(a, Capability::Verfuegbarkeit)?;
        window.intervals()?;
        if addresses.is_empty() || addresses.len() > 257 { return Err(invalid("Teilnehmende")); }
        let mut out = Vec::new();
        for chunk in addresses.chunks(20) {
            let body = json!({"schedules":chunk,"startTime":{"dateTime":window.start,"timeZone":window.time_zone},
                "endTime":{"dateTime":window.end,"timeZone":window.time_zone},"availabilityViewInterval":30});
            let url = format!("{}/me/calendar/getSchedule", self.graph_base());
            let reply = self.send_authed_for_account(a, true, own, |c, token| c.post(&url).bearer_auth(token)
                .header("Prefer", format!("outlook.timezone=\"{}\"", window.time_zone)).json(&body)).await?;
            out.extend(parse_schedule(&self.poll_reply(a, reply)?, chunk, window)?);
        }
        Ok(out)
    }

    pub async fn poll_delta_page(&self, a: &Acct, own: &str, cursor: Option<&str>) -> Result<DeltaPage, M365Error> {
        self.poll_ready(a, Capability::AntwortenLesen)?;
        let initial = format!("{}/me/mailFolders/inbox/messages/delta?$select=id,subject,from,receivedDateTime,conversationId,body,internetMessageHeaders", self.graph_base());
        let url = cursor.unwrap_or(&initial);
        validate_delta_link(self.graph_base(), url)?;
        let reply = self.send_authed_for_account(a, true, own, |c, token| c.get(url).bearer_auth(token)
            .header("Prefer", "outlook.body-content-type=\"text\", odata.maxpagesize=50")).await?;
        parse_delta(self.poll_reply(a, reply)?, self.graph_base())
    }

    /// Nach sendMail nur die exakte Marke und genau einen Empfaenger zuordnen.
    /// Fehlende/mehrdeutige Treffer bleiben nachverfolgbar, loesen keinen Neuversand aus.
    pub async fn poll_sent_conversation(&self, a: &Acct, own: &str, subject: &str, address: &str, since: i64) -> Result<Option<String>, M365Error> {
        self.poll_ready(a, Capability::AntwortenLesen)?;
        let since = chrono::DateTime::from_timestamp_millis(since).ok_or_else(|| invalid("Versandzeit"))?.to_rfc3339();
        let filter = format!("sentDateTime ge {since} and subject eq '{}'", subject.replace('\'', "''"));
        let url = format!("{}/me/mailFolders/sentitems/messages", self.graph_base());
        let reply = self.send_authed_for_account(a, true, own, |c, token| c.get(&url).bearer_auth(token)
            .query(&[("$filter", filter.as_str()), ("$select", "conversationId,toRecipients"), ("$top", "2")])).await?;
        let value = self.poll_reply(a, reply)?;
        let rows = value.get("value").and_then(Value::as_array).ok_or_else(malformed)?;
        if rows.len() != 1 || value.get("@odata.nextLink").is_some() { return Ok(None); }
        let row = &rows[0];
        let recipients = row.get("toRecipients").and_then(Value::as_array).ok_or_else(malformed)?;
        if recipients.len() != 1 || !recipients[0].pointer("/emailAddress/address").and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case(address)) { return Ok(None); }
        Ok(row.get("conversationId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string))
    }
}
