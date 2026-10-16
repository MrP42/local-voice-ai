//! Ganztaegiger Termin am Kalender (C4, Scope `Calendars.ReadWrite`): ein Eintrag „Frist“.
//!
//! Gegenstueck zu `event` (Notiz an einem vorhandenen Termin): hier entsteht ein NEUER Termin.
//! Er ist ganztaegig, blockiert die Verfuegbarkeit nicht (`showAs: free`) und hat nur Betreff
//! und einen Klartext-Text, keine Teilnehmenden (es geht keine Einladung an Dritte).
//!
//! - **Keine Dublette**: jede Anfrage traegt eine `transactionId`, die der Aufrufer aus Lauf,
//!   Schritt und Frist bildet. Wiederholt ein Aufrufer eine Anfrage, deren Antwort verloren ging,
//!   erkennt Graph sie daran und legt den Termin nicht noch einmal an. Das ist die zweite
//!   Sicherung hinter dem Eintrag im Herkunftsregister des Bausteins.
//! - **Unklarer Ausgang**: reisst die Verbindung nach dem Senden, ist es `Uncertain` (nie von
//!   selbst wiederholen), wie bei `sendMail`.
//! - Betreff und Text sind Klartext (kein HTML), der Betreff auf eine Zeile und 255 Zeichen
//!   gekuerzt: ein Termintext aus einem Transkript kann kein Markup und keine Einladung enthalten.

use chrono::{Duration as ChronoDuration, NaiveDate};
use serde_json::{json, Value};

use super::error::M365Error;
use super::event::MAX_NOTE_CHARS;
use super::service::{error_of, Acct, M365Service};
use crate::managers::integrations::model::Capability;

pub const MAX_SUBJECT_CHARS: usize = 255;
/// Laengste `transactionId`, die Graph annimmt (255); wir bilden sie aus 64 Hex-Zeichen.
const MAX_TRANSACTION_CHARS: usize = 120;

/// Was der neue Termin traegt.
#[derive(Clone, Debug)]
pub struct NewAllDayEvent<'a> {
    pub subject: &'a str,
    /// Der Tag der Frist.
    pub date: NaiveDate,
    /// Klartext, hoechstens [`MAX_NOTE_CHARS`] Zeichen.
    pub body: &'a str,
    /// Stabile Kennung dieser einen Anfrage (nur Buchstaben, Ziffern, `-`, `_`).
    pub transaction_id: &'a str,
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

fn midnight(date: NaiveDate) -> String {
    format!("{}T00:00:00", date.format("%Y-%m-%d"))
}

/// Der JSON-Rumpf der Anfrage (ohne Netz pruefbar).
pub fn request_body(ev: &NewAllDayEvent<'_>) -> Result<Value, M365Error> {
    let subject = one_line(ev.subject, MAX_SUBJECT_CHARS);
    if subject.is_empty() {
        return Err(M365Error::Invalid(
            "Der Termin braucht einen Betreff.".to_string(),
        ));
    }
    if ev.body.chars().count() > MAX_NOTE_CHARS {
        return Err(M365Error::Invalid(format!(
            "Der Text des Termins ist zu lang (höchstens {MAX_NOTE_CHARS} Zeichen)."
        )));
    }
    let tx = ev.transaction_id;
    if tx.is_empty()
        || tx.len() > MAX_TRANSACTION_CHARS
        || !tx
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(M365Error::Invalid(
            "Die Kennung der Anfrage ist ungültig.".to_string(),
        ));
    }
    let next = ev
        .date
        .checked_add_signed(ChronoDuration::days(1))
        .ok_or_else(|| M365Error::Invalid("Das Datum ist ungültig.".to_string()))?;
    Ok(json!({
        "subject": subject,
        "isAllDay": true,
        "start": { "dateTime": midnight(ev.date), "timeZone": "UTC" },
        "end": { "dateTime": midnight(next), "timeZone": "UTC" },
        "showAs": "free",
        "body": { "contentType": "text", "content": ev.body },
        "transactionId": tx,
    }))
}

/// Kuerzeste und laengste Dauer eines Folgetermins (Minuten).
pub const MIN_MINUTES: u32 = 5;
pub const MAX_MINUTES: u32 = 8 * 60;

/// Ein Folgetermin mit Uhrzeit (Workflow-Baustein `calendar.followup`). Mit Teilnehmenden
/// verschickt Outlook Einladungen: das entscheidet der Baustein (Freigabe bei Dritten, E3).
#[derive(Clone, Debug)]
pub struct NewTimedEvent<'a> {
    pub subject: &'a str,
    /// Beginn in UTC.
    pub start: chrono::DateTime<chrono::Utc>,
    pub minutes: u32,
    /// Klartext, hoechstens [`MAX_NOTE_CHARS`] Zeichen.
    pub body: &'a str,
    /// Adressen der Eingeladenen; leer: nur im eigenen Kalender.
    pub attendees: &'a [String],
    pub transaction_id: &'a str,
}

fn check_transaction(tx: &str) -> Result<(), M365Error> {
    if tx.is_empty()
        || tx.len() > MAX_TRANSACTION_CHARS
        || !tx
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(M365Error::Invalid(
            "Die Kennung der Anfrage ist ungültig.".to_string(),
        ));
    }
    Ok(())
}

/// Der JSON-Rumpf eines Folgetermins (ohne Netz pruefbar).
pub fn timed_request_body(ev: &NewTimedEvent<'_>) -> Result<Value, M365Error> {
    let subject = one_line(ev.subject, MAX_SUBJECT_CHARS);
    if subject.is_empty() {
        return Err(M365Error::Invalid(
            "Der Termin braucht einen Betreff.".to_string(),
        ));
    }
    if !(MIN_MINUTES..=MAX_MINUTES).contains(&ev.minutes) {
        return Err(M365Error::Invalid(format!(
            "Die Dauer muss zwischen {MIN_MINUTES} und {MAX_MINUTES} Minuten liegen."
        )));
    }
    if ev.body.chars().count() > MAX_NOTE_CHARS {
        return Err(M365Error::Invalid(format!(
            "Der Text des Termins ist zu lang (höchstens {MAX_NOTE_CHARS} Zeichen)."
        )));
    }
    check_transaction(ev.transaction_id)?;
    let end = ev.start + ChronoDuration::minutes(i64::from(ev.minutes));
    let fmt = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut body = json!({
        "subject": subject,
        "start": { "dateTime": fmt(ev.start), "timeZone": "UTC" },
        "end": { "dateTime": fmt(end), "timeZone": "UTC" },
        "body": { "contentType": "text", "content": ev.body },
        "isReminderOn": true,
        "reminderMinutesBeforeStart": 15,
        "transactionId": ev.transaction_id,
    });
    if !ev.attendees.is_empty() {
        body["attendees"] = Value::Array(
            ev.attendees
                .iter()
                .map(|a| json!({"emailAddress": {"address": a}, "type": "required"}))
                .collect(),
        );
    }
    Ok(body)
}

impl M365Service {
    /// Legt einen Folgetermin mit Uhrzeit an (Scope `Calendars.ReadWrite`); mit Teilnehmenden
    /// verschickt Outlook die Einladungen. Gibt Kennung und Link zurueck.
    pub async fn create_event(
        &self,
        a: &Acct,
        ev: &NewTimedEvent<'_>,
    ) -> Result<(String, Option<String>), M365Error> {
        a.require(Capability::CalendarWrite)?;
        let body = timed_request_body(ev)?;
        let url = format!("{}/me/events", self.graph_base());
        let reply = self
            .send_authed(a, false, |c, token| {
                c.post(&url).bearer_auth(token).json(&body)
            })
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Kalender"));
        }
        let v = reply.json()?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| M365Error::Parse("Antwort ohne Termin-Kennung".to_string()))?;
        Ok((
            id,
            v.get("webLink").and_then(Value::as_str).map(str::to_string),
        ))
    }

    /// Legt den ganztaegigen Termin an und gibt die Kennung zurueck, die Graph vergab.
    pub async fn create_all_day_event(
        &self,
        a: &Acct,
        ev: &NewAllDayEvent<'_>,
    ) -> Result<String, M365Error> {
        a.require(Capability::CalendarWrite)?;
        let body = request_body(ev)?;
        let url = format!("{}/me/events", self.graph_base());
        // Nicht wiederholbar: bei unklarem Ausgang `Uncertain`, nie stilles Wiederholen.
        let reply = self
            .send_authed(a, false, |c, token| {
                c.post(&url).bearer_auth(token).json(&body)
            })
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Kalender"));
        }
        let v = reply.json()?;
        v.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| M365Error::Parse("Antwort ohne Termin-Kennung".to_string()))
    }
}
