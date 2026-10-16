//! Mail senden ueber Microsoft Graph: `POST /me/sendMail` (A5, Scope `Mail.Send`).
//!
//! Grundsaetze:
//! - Eingaben werden VOR dem Netz geprueft: mindestens ein Empfaenger, jede Adresse
//!   brauchbar (`meetings::mail::valid_address`, streng), hoechstens
//!   `MAX_RECIPIENTS`, Betreff eine Zeile, Text begrenzt. Eine Adresse, die nicht
//!   stimmt, bricht ab; sie wird nie still weggelassen.
//! - Kein automatisches Wiederholen: bricht die Verbindung NACH dem Senden der
//!   Anfrage ab, ist der Ausgang unklar (`Uncertain`), und eine zweite Mail waere
//!   moeglicherweise doppelt. Nur ein 401 (die Anfrage wurde nicht bearbeitet) wird
//!   nach dem Erneuern des Tokens einmal wiederholt (`send_authed`).
//! - Die Mail liegt danach im Ordner „Gesendet“ (`saveToSentItems`).

use serde_json::{json, Value};

use super::error::M365Error;
use super::service::{error_of, Acct, M365Service};
use crate::managers::integrations::model::Capability;
use crate::managers::meetings::mail::{finalize, valid_address, MailDraft};

/// Empfaenger an und Kopie zusammen.
pub const MAX_RECIPIENTS: usize = 30;
pub const MAX_SUBJECT_CHARS: usize = 255;
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Anhaenge (B5): hoechstens so viele, zusammen hoechstens so gross. Sie gehen als
/// `fileAttachment` im Koerper von `sendMail` mit (base64 macht daraus gut ein Drittel mehr,
/// Graph erlaubt dafuer rund 4 MB je Anfrage); groessere Dateien brauchen eine Upload-Sitzung
/// an einem Entwurf, die dieses Konto nicht kennt (Scope `Mail.ReadWrite`).
pub const MAX_ATTACHMENTS: usize = 5;
pub const MAX_ATTACHMENT_BYTES: usize = 2_560 * 1024;
const MAX_ATTACHMENT_NAME_CHARS: usize = 150;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailBody {
    Text(String),
    Html(String),
}

impl MailBody {
    fn content(&self) -> &str {
        match self {
            MailBody::Text(t) | MailBody::Html(t) => t,
        }
    }
}

/// Ein Anhang (Inhalt im Speicher, hoechstens [`MAX_ATTACHMENT_BYTES`] zusammen).
#[derive(Clone, PartialEq, Eq)]
pub struct MailAttachment {
    pub name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for MailAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nie den Inhalt in ein Protokoll.
        write!(
            f,
            "MailAttachment {{ name: {:?}, content_type: {:?}, bytes: {} }}",
            self.name,
            self.content_type,
            self.bytes.len()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailMessage {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: MailBody,
    pub attachments: Vec<MailAttachment>,
}

fn clean_list(list: &[String], what: &str) -> Result<Vec<String>, M365Error> {
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let a = raw.trim();
        if a.is_empty() {
            continue;
        }
        if !valid_address(a) {
            return Err(M365Error::Invalid(format!(
                "Die Adresse „{a}“ ({what}) ist nicht brauchbar."
            )));
        }
        if !out.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            out.push(a.to_string());
        }
    }
    Ok(out)
}

impl MailMessage {
    /// Prueft und bereinigt die Eingaben (siehe Moduldoku).
    pub fn new(
        to: &[String],
        cc: &[String],
        subject: &str,
        body: MailBody,
    ) -> Result<Self, M365Error> {
        let to = clean_list(to, "An")?;
        let cc: Vec<String> = clean_list(cc, "Kopie")?
            .into_iter()
            .filter(|c| !to.iter().any(|t| t.eq_ignore_ascii_case(c)))
            .collect();
        if to.is_empty() {
            return Err(M365Error::Invalid(
                "Es fehlt ein Empfänger für die Mail.".to_string(),
            ));
        }
        if to.len() + cc.len() > MAX_RECIPIENTS {
            return Err(M365Error::Invalid(format!(
                "Zu viele Empfänger (höchstens {MAX_RECIPIENTS})."
            )));
        }
        let subject: String = subject
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if subject.is_empty() {
            return Err(M365Error::Invalid(
                "Es fehlt ein Betreff für die Mail.".to_string(),
            ));
        }
        if subject.chars().count() > MAX_SUBJECT_CHARS {
            return Err(M365Error::Invalid(format!(
                "Der Betreff ist zu lang (höchstens {MAX_SUBJECT_CHARS} Zeichen)."
            )));
        }
        if body.content().trim().is_empty() {
            return Err(M365Error::Invalid("Der Mailtext ist leer.".to_string()));
        }
        if body.content().len() > MAX_BODY_BYTES {
            return Err(M365Error::Invalid(
                "Der Mailtext ist zu lang (höchstens 1 MB).".to_string(),
            ));
        }
        Ok(Self {
            to,
            cc,
            subject,
            body,
            attachments: Vec::new(),
        })
    }

    /// Haengt Anhaenge an (B5). Zu viele, zu grosse, leere oder unbenannte Anhaenge sind ein
    /// Fehler; nichts wird gekuerzt oder weggelassen.
    pub fn with_attachments(mut self, list: Vec<MailAttachment>) -> Result<Self, M365Error> {
        if list.len() > MAX_ATTACHMENTS {
            return Err(M365Error::Invalid(format!(
                "Zu viele Anhänge (höchstens {MAX_ATTACHMENTS})."
            )));
        }
        let mut total = 0usize;
        for a in &list {
            let name = a.name.trim();
            if name.is_empty()
                || name.chars().count() > MAX_ATTACHMENT_NAME_CHARS
                || name.chars().any(char::is_control)
            {
                return Err(M365Error::Invalid(
                    "Der Name eines Anhangs ist ungültig.".to_string(),
                ));
            }
            if a.bytes.is_empty() {
                return Err(M365Error::Invalid(format!("Der Anhang „{name}“ ist leer.")));
            }
            total += a.bytes.len();
        }
        if total > MAX_ATTACHMENT_BYTES {
            return Err(M365Error::Invalid(format!(
                "Die Anhänge sind zu groß (höchstens {} KiB zusammen).",
                MAX_ATTACHMENT_BYTES / 1024
            )));
        }
        self.attachments = list;
        Ok(self)
    }

    /// Der Koerper der Anfrage an `sendMail`.
    pub fn to_graph_json(&self) -> Value {
        let recipients = |list: &[String]| -> Vec<Value> {
            list.iter()
                .map(|a| json!({ "emailAddress": { "address": a } }))
                .collect()
        };
        let (kind, content) = match &self.body {
            MailBody::Text(t) => ("Text", t),
            MailBody::Html(h) => ("HTML", h),
        };
        let mut message = json!({
            "subject": self.subject,
            "body": { "contentType": kind, "content": content },
            "toRecipients": recipients(&self.to),
        });
        if !self.cc.is_empty() {
            message["ccRecipients"] = Value::Array(recipients(&self.cc));
        }
        if !self.attachments.is_empty() {
            use base64::Engine as _;
            let files: Vec<Value> = self
                .attachments
                .iter()
                .map(|a| {
                    json!({
                        "@odata.type": "#microsoft.graph.fileAttachment",
                        "name": a.name.trim(),
                        "contentType": a.content_type,
                        "contentBytes": base64::engine::general_purpose::STANDARD.encode(&a.bytes),
                    })
                })
                .collect();
            message["attachments"] = Value::Array(files);
        }
        json!({ "message": message, "saveToSentItems": true })
    }

    /// Kurzes Ziel fuer Audit und Freigabe: erster Empfaenger, dazu die Anzahl.
    pub fn gate_target(&self) -> String {
        let total = self.to.len() + self.cc.len();
        if total <= 1 {
            self.to[0].clone()
        } else {
            format!("{} (+{})", self.to[0], total - 1)
        }
    }

    /// Die Argumente fuer Freigabe-Vorschau und Bindung (Empfaenger vollstaendig,
    /// Text als `body` und damit nur gekuerzt in der Vorschau).
    pub fn gate_args(&self) -> Value {
        let mut args = json!({
            "to": self.to,
            "subject": self.subject,
            "body": self.body.content(),
        });
        if !self.cc.is_empty() {
            args["cc"] = json!(self.cc);
        }
        if !self.attachments.is_empty() {
            args["attachments"] = json!(self
                .attachments
                .iter()
                .map(|a| a.name.trim().to_string())
                .collect::<Vec<_>>());
        }
        args
    }
}

/// Die Follow-up-Mail der Besprechung als Nachricht: HTML aus dem bearbeiteten Text
/// (`finalize`, wie beim Kopieren). Eine unbrauchbare Adresse bricht ab, statt dass
/// die Mail an weniger Leute geht, als der Nutzer eingetragen hat.
pub fn message_from_draft(draft: &MailDraft) -> Result<MailMessage, M365Error> {
    if let Some(bad) = draft
        .to
        .iter()
        .map(|a| a.trim())
        .find(|a| !a.is_empty() && !valid_address(a))
    {
        return Err(M365Error::Invalid(format!(
            "Die Adresse „{bad}“ ist nicht brauchbar."
        )));
    }
    let d = finalize(draft);
    MailMessage::new(&d.to, &[], &d.subject, MailBody::Html(d.body_html))
}

/// Ein angelegter Entwurf: Kennung und Link zum Oeffnen in Outlook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftCreated {
    pub id: String,
    pub web_link: Option<String>,
}

impl M365Service {
    /// Legt die Mail als Entwurf im Ordner „Entwürfe“ ab (Scope `Mail.ReadWrite`); sie geht an
    /// niemanden. Nicht wiederholbar: bei unklarem Ausgang `Uncertain`.
    pub async fn create_draft(
        &self,
        a: &Acct,
        msg: &MailMessage,
    ) -> Result<DraftCreated, M365Error> {
        a.require(Capability::MailDraft)?;
        let body = msg
            .to_graph_json()
            .get("message")
            .cloned()
            .unwrap_or(Value::Null);
        let url = format!("{}/me/messages", self.graph_base());
        let reply = self
            .send_authed(a, false, |c, token| {
                c.post(&url)
                    .bearer_auth(token)
                    .header(reqwest::header::ACCEPT, "application/json")
                    .json(&body)
            })
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Entwurf"));
        }
        let v = reply.json()?;
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| M365Error::Parse("Antwort ohne Kennung des Entwurfs".to_string()))?;
        Ok(DraftCreated {
            id,
            web_link: v.get("webLink").and_then(Value::as_str).map(str::to_string),
        })
    }

    /// Sendet die Mail (Scope `Mail.Send`). Siehe Moduldoku.
    pub async fn send_mail(&self, a: &Acct, msg: &MailMessage) -> Result<(), M365Error> {
        a.require(Capability::MailSend)?;
        let body = msg.to_graph_json();
        let url = format!("{}/me/sendMail", self.graph_base());
        let reply = self
            .send_authed(a, false, |c, token| {
                c.post(&url)
                    .bearer_auth(token)
                    .header(reqwest::header::ACCEPT, "application/json")
                    .json(&body)
            })
            .await?;
        if reply.is_success() {
            Ok(())
        } else {
            Err(error_of(&reply, "sendMail"))
        }
    }
}
