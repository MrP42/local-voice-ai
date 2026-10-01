//! Notiz am Termin (A5, Scope `Calendars.ReadWrite`): haengt einen Absatz an den
//! Text eines Outlook-Termins an.
//!
//! Ablauf: den Termin im Fenster seiner Zeit suchen (der Kalender-Cache kennt die
//! `iCalUId`, nicht die Graph-Kennung), seinen Text lesen, die Notiz anhaengen,
//! mit `If-Match` zurueckschreiben (ein zwischenzeitlich geaenderter Termin wird
//! nicht ueberschrieben, sondern gemeldet).
//!
//! - **Nie den Text ersetzen**: die Einladung (Teams-Link) bleibt unveraendert; ein
//!   HTML-Text bekommt den Absatz vor `</body>`, ein Klartext-Text wird verlaengert.
//! - **Wiederholbar ohne Dublette**: die Notiz traegt eine Marke
//!   (`<!--lva-note:<Pruefsumme>-->`); steht die Marke oder der Notiztext schon im
//!   Termin, geschieht nichts (`AlreadyThere`). Das macht auch den Fall „Antwort
//!   verloren“ (`Uncertain`) harmlos: ein zweiter Versuch haengt nichts doppelt an.
//! - Ist man nicht der Organisator, antwortet Graph mit 403: das kommt als
//!   `Denied` mit Klartext, nicht als stiller Misserfolg.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::error::M365Error;
use super::service::{error_of, Acct, M365Service};
use crate::managers::calendar::graph::parse_event;
use crate::managers::integrations::model::Capability;

pub const MAX_NOTE_CHARS: usize = 20_000;
const MARKER_PREFIX: &str = "lva-note:";

/// Der Termin, wie ihn der Kalender-Cache kennt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRef {
    /// Bereinigte `iCalUId` (`CalEvent::uid`).
    pub uid: String,
    pub starts_at: i64,
    pub ends_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteOutcome {
    Added,
    AlreadyThere,
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn marker_of(note: &str) -> String {
    let digest = Sha256::digest(note.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{MARKER_PREFIX}{hex}")
}

fn note_html(note: &str) -> String {
    let body = escape_html(note.trim())
        .replace("\r\n", "\n")
        .replace('\n', "<br>");
    format!(
        "<div><hr><p><b>Notiz aus Local Voice AI</b></p><p>{body}</p><!--{}--></div>",
        marker_of(note)
    )
}

/// Haengt die Notiz an einen HTML-Text an (vor dem letzten `</body>`, sonst ans Ende).
pub fn append_html(content: &str, note: &str) -> String {
    let snippet = note_html(note);
    match content.to_ascii_lowercase().rfind("</body>") {
        Some(pos) => format!("{}{}{}", &content[..pos], snippet, &content[pos..]),
        None => format!("{content}{snippet}"),
    }
}

/// Haengt die Notiz an einen Klartext-Text an.
pub fn append_text(content: &str, note: &str) -> String {
    format!(
        "{}\n\n--- Notiz aus Local Voice AI ---\n{}\n",
        content.trim_end(),
        note.trim()
    )
}

/// Steht die Notiz schon im Termin (Marke oder Text)?
pub fn already_has(content: &str, note: &str, html: bool) -> bool {
    if content.contains(&marker_of(note)) {
        return true;
    }
    if html {
        content.contains(&escape_html(note.trim()).replace('\n', "<br>"))
    } else {
        content.contains(note.trim())
    }
}

fn valid_event_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 600
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'='))
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

impl M365Service {
    /// Die Graph-Kennung des Termins: im Fenster seiner Zeit suchen und `iCalUId` und
    /// Beginn vergleichen (dieselbe Bereinigung wie beim Kalenderabruf).
    pub async fn find_event_id(&self, a: &Acct, ev: &EventRef) -> Result<String, M365Error> {
        let end = ev.ends_at.max(ev.starts_at + 60_000);
        let url = format!(
            "{}/me/calendarView?startDateTime={}&endDateTime={}&$select=id,iCalUId,subject,start,end,isAllDay,isCancelled&$top=50",
            self.graph_base(),
            iso(ev.starts_at),
            iso(end)
        );
        let reply = self
            .send_authed(a, true, |c, token| {
                c.get(&url)
                    .bearer_auth(token)
                    .header("Prefer", "outlook.timezone=\"UTC\"")
            })
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Termin"));
        }
        let v = reply.json()?;
        let items = v.get("value").and_then(Value::as_array);
        for item in items.into_iter().flatten() {
            let Ok(found) = parse_event(item, "m365", None) else {
                continue;
            };
            if found.uid == ev.uid && found.starts_at == ev.starts_at {
                if let Some(id) = item.get("id").and_then(Value::as_str) {
                    if valid_event_id(id) {
                        return Ok(id.to_string());
                    }
                }
            }
        }
        Err(M365Error::NotFound("Termin".to_string()))
    }

    /// Haengt die Notiz an den Termin an (siehe Moduldoku).
    pub async fn add_event_note(
        &self,
        a: &Acct,
        ev: &EventRef,
        note: &str,
    ) -> Result<NoteOutcome, M365Error> {
        a.require(Capability::CalendarWrite)?;
        if note.trim().is_empty() {
            return Err(M365Error::Invalid("Die Notiz ist leer.".to_string()));
        }
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(M365Error::Invalid(format!(
                "Die Notiz ist zu lang (höchstens {MAX_NOTE_CHARS} Zeichen)."
            )));
        }
        let id = self.find_event_id(a, ev).await?;
        let enc = super::drive::encode_segment(&id);
        let url = format!("{}/me/events/{enc}", self.graph_base());
        let get_url = format!("{url}?$select=id,subject,body");
        let reply = self
            .send_authed(a, true, |c, token| c.get(&get_url).bearer_auth(token))
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Termin"));
        }
        let v = reply.json()?;
        let html = v
            .pointer("/body/contentType")
            .and_then(Value::as_str)
            .map(|t| t.eq_ignore_ascii_case("html"))
            .unwrap_or(false);
        let content = v
            .pointer("/body/content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if already_has(content, note, html) {
            return Ok(NoteOutcome::AlreadyThere);
        }
        let etag = v
            .get("@odata.etag")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (kind, merged) = if html {
            ("html", append_html(content, note))
        } else {
            ("text", append_text(content, note))
        };
        let patch = json!({ "body": { "contentType": kind, "content": merged } });
        let reply = self
            .send_authed(a, false, |c, token| {
                let mut rb = c.patch(&url).bearer_auth(token).json(&patch);
                if let Some(etag) = &etag {
                    rb = rb.header(reqwest::header::IF_MATCH, etag.as_str());
                }
                rb
            })
            .await?;
        if reply.is_success() {
            Ok(NoteOutcome::Added)
        } else if reply.status == 412 {
            Err(M365Error::Http {
                status: 412,
                code: "etagMismatch".to_string(),
            })
        } else {
            Err(error_of(&reply, "Termin"))
        }
    }
}
