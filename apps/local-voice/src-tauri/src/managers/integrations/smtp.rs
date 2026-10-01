//! SMTP-Postfach (A6, Goal „Integrationen“): Mail ueber ein eigenes Konto senden
//! (App-Passwort, STARTTLS oder TLS), freigabepflichtig ueber das Tor
//! (`targets::send_mail` -> `gate`).
//!
//! Blockierend (`std::net`), weil das Tor synchron ist: der Aufrufer fuehrt
//! `send` auf einem Arbeitsthread aus (Tauri: `spawn_blocking`).
//!
//! Sicherheitsannahmen:
//! - **Das Passwort steht nur im Geheimnisspeicher** (`secrets`, Fach `password`).
//!   `send`/`test` bekommen es als Argument und geben es nie zurueck, nie in einen
//!   Fehlertext, nie ins Protokoll. Es geht ausschliesslich als `AUTH PLAIN`/`LOGIN`
//!   ueber eine bereits verschluesselte Verbindung.
//! - **Kein Herabstufen**: `starttls` ohne `STARTTLS` in der Antwort des Servers ->
//!   Fehler, nie ein Login im Klartext. Klartext (`plain`) gibt es nur gegen
//!   Loopback (lokaler Testserver). Zertifikat und Hostname werden immer geprueft
//!   (Schannel), TLS mindestens 1.2.
//! - **STARTTLS-Einschleusung**: Bytes, die der Server vor dem Handshake mitschickt,
//!   brechen die Verbindung ab (CVE-Klasse „response injection“).
//! - **Kopfzeilen-Einschleusung**: Adressen und Betreff duerfen weder Zeilenumbruch
//!   noch Steuerzeichen enthalten; Adressen sind ASCII und streng geprueft
//!   (`meetings::mail::valid_address`). Sonderzeichen im Betreff und im Namen
//!   kodiert `mail-builder`.
//! - **Grenzen**: hoechstens `MAX_RECIPIENTS` Empfaenger, Nachricht hoechstens
//!   `MAX_MESSAGE_BYTES`, Zeilen der Antwort `MAX_LINE`, Zeitlimits fuer Verbindung
//!   und jeden Lese-/Schreibschritt. Kein Hintergrundprozess, kein Modell: bei vollem
//!   Arbeitsspeicher aendert sich nichts, die Nachricht ist hoechstens 5 MiB gross.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use mail_builder::headers::{address::Address, date::Date};
use mail_builder::MessageBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use zeroize::Zeroizing;

use crate::managers::meetings::mail::valid_address;

pub const MAX_RECIPIENTS: usize = 20;
pub const MAX_MESSAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_SUBJECT_CHARS: usize = 200;
pub const MAX_LINE: usize = 4096;
/// Hoechstzahl Zeilen einer mehrzeiligen Antwort.
const MAX_REPLY_LINES: usize = 100;

/// Verbindungsart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// Klartext verbinden, dann `STARTTLS` (Port 587).
    Starttls,
    /// Sofort TLS (Port 465).
    Tls,
    /// Klartext: nur gegen Loopback (lokaler Testserver).
    Plain,
}

impl Security {
    pub fn default_port(self) -> u16 {
        match self {
            Security::Starttls => 587,
            Security::Tls => 465,
            Security::Plain => 25,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Security::Starttls => "starttls",
            Security::Tls => "tls",
            Security::Plain => "plain",
        }
    }
}

/// Einstellungen eines SMTP-Postfachs (`config_json`; das Passwort steht NICHT hier).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// Benutzername fuer die Anmeldung (leer: ohne Anmeldung, nur Loopback).
    #[serde(default)]
    pub username: String,
    pub from_address: String,
    #[serde(default)]
    pub from_name: String,
}

pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

impl SmtpConfig {
    pub fn from_config_json(json: &str) -> Result<Self, SmtpError> {
        let v: Value = serde_json::from_str(json)
            .map_err(|_| SmtpError::Config("Die SMTP-Einstellungen sind nicht lesbar."))?;
        let cfg: SmtpConfig = serde_json::from_value(v)
            .map_err(|_| SmtpError::Config("Die SMTP-Einstellungen sind unvollständig."))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "port": self.port,
            "security": self.security.as_str(),
            "username": self.username,
            "from_address": self.from_address,
            "from_name": self.from_name,
        })
    }

    pub fn validate(&self) -> Result<(), SmtpError> {
        let bad = |m: &'static str| Err(SmtpError::Config(m));
        let host = self.host.trim();
        if host.is_empty() {
            return bad("Der Server fehlt.");
        }
        if host.len() > 253
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "/\\@<>\"'".contains(c))
        {
            return bad("Der Servername ist ungültig.");
        }
        if self.port == 0 {
            return bad("Der Port ist ungültig.");
        }
        if self.security == Security::Plain && !is_loopback_host(host) {
            return Err(SmtpError::PlainNotAllowed);
        }
        if self.username.chars().any(char::is_control) || self.username.len() > 254 {
            return bad("Der Benutzername ist ungültig.");
        }
        if !valid_address(&self.from_address) || !self.from_address.is_ascii() {
            return bad("Die Absenderadresse ist ungültig.");
        }
        if self.from_name.chars().any(char::is_control) || self.from_name.chars().count() > 120 {
            return bad("Der Absendername ist ungültig.");
        }
        Ok(())
    }
}

/// Fehler des SMTP-Pfads. `code()` ist der Schluessel der Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmtpError {
    Config(&'static str),
    PasswordMissing,
    PlainNotAllowed,
    Connect(String),
    Timeout,
    Tls(String),
    StarttlsUnsupported,
    AuthUnsupported,
    AuthFailed,
    /// Der Server hat Absender, Empfaenger oder Inhalt abgelehnt.
    Rejected {
        stage: &'static str,
        text: String,
    },
    Protocol(String),
    InvalidMessage(&'static str),
}

impl SmtpError {
    pub fn code(&self) -> &'static str {
        match self {
            SmtpError::Config(_) => "smtp_config_invalid",
            SmtpError::PasswordMissing => "smtp_password_missing",
            SmtpError::PlainNotAllowed => "smtp_plain_not_allowed",
            SmtpError::Connect(_) => "smtp_connect_failed",
            SmtpError::Timeout => "smtp_timeout",
            SmtpError::Tls(_) => "smtp_tls_failed",
            SmtpError::StarttlsUnsupported => "smtp_starttls_unsupported",
            SmtpError::AuthUnsupported => "smtp_auth_unsupported",
            SmtpError::AuthFailed => "smtp_auth_failed",
            SmtpError::Rejected { .. } => "smtp_rejected",
            SmtpError::Protocol(_) => "smtp_protocol",
            SmtpError::InvalidMessage(_) => "smtp_message_invalid",
        }
    }
}

impl std::fmt::Display for SmtpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmtpError::Config(m) => write!(f, "{m}"),
            SmtpError::PasswordMissing => write!(
                f,
                "Das Passwort fehlt. Bitte in der Integration neu eintragen."
            ),
            SmtpError::PlainNotAllowed => write!(
                f,
                "Eine unverschlüsselte Verbindung ist nur zu diesem Rechner erlaubt. Bitte STARTTLS oder TLS wählen."
            ),
            SmtpError::Connect(m) => write!(f, "Der Mailserver ist nicht erreichbar: {m}"),
            SmtpError::Timeout => write!(f, "Der Mailserver antwortet nicht (Zeitüberschreitung)."),
            SmtpError::Tls(m) => write!(
                f,
                "Die verschlüsselte Verbindung kam nicht zustande (Zertifikat oder Protokoll): {m}"
            ),
            SmtpError::StarttlsUnsupported => write!(
                f,
                "Der Server bietet kein STARTTLS an. Aus Sicherheitsgründen wird kein Passwort unverschlüsselt gesendet; bitte TLS (Port 465) wählen."
            ),
            SmtpError::AuthUnsupported => write!(
                f,
                "Der Server bietet keine Anmeldung mit Benutzername und Passwort (PLAIN/LOGIN) an."
            ),
            SmtpError::AuthFailed => write!(
                f,
                "Die Anmeldung wurde abgelehnt. Benutzername oder Passwort stimmen nicht; bei Gmail und anderen ist ein App-Passwort nötig."
            ),
            SmtpError::Rejected { stage, text } => {
                let what = match *stage {
                    "greeting" => "Der Server hat die Verbindung abgelehnt",
                    "sender" => "Der Server hat den Absender abgelehnt",
                    "recipient" => "Der Server hat einen Empfänger abgelehnt",
                    _ => "Der Server hat die Nachricht abgelehnt",
                };
                write!(f, "{what}: {text}")
            }
            SmtpError::Protocol(m) => write!(f, "Unerwartete Antwort des Mailservers: {m}"),
            SmtpError::InvalidMessage(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for SmtpError {}

/// Zeitlimits (und fuer Tests ein zusaetzlicher Vertrauensanker).
#[derive(Clone, Debug)]
pub struct ConnectOpts {
    pub connect_timeout: Duration,
    pub io_timeout: Duration,
    /// Nur Tests: ein selbstsigniertes Zertifikat als Vertrauensanker.
    #[cfg(test)]
    pub extra_root_pem: Option<Vec<u8>>,
}

impl Default for ConnectOpts {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            io_timeout: Duration::from_secs(30),
            #[cfg(test)]
            extra_root_pem: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Nachricht
// ---------------------------------------------------------------------------

/// Eine Mail, wie das Tor sie sieht.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MailMessage {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
}

impl MailMessage {
    /// Alle Empfaenger (An und Kopie), ohne Doppelte.
    pub fn recipients(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.to
            .iter()
            .chain(self.cc.iter())
            .map(|a| a.trim().to_string())
            .filter(|a| seen.insert(a.to_lowercase()))
            .collect()
    }

    pub fn validate(&self) -> Result<(), SmtpError> {
        let bad = |m: &'static str| Err(SmtpError::InvalidMessage(m));
        if self.to.is_empty() {
            return bad("Es ist kein Empfänger angegeben.");
        }
        let all = self.recipients();
        if all.len() > MAX_RECIPIENTS {
            return bad("Zu viele Empfänger (höchstens 20).");
        }
        for a in &all {
            if !valid_address(a) || !a.is_ascii() {
                return bad("Eine Empfängeradresse ist ungültig.");
            }
        }
        let subject = self.subject.trim();
        if subject.is_empty() {
            return bad("Der Betreff fehlt.");
        }
        if subject.chars().count() > MAX_SUBJECT_CHARS || subject.chars().any(char::is_control) {
            return bad("Der Betreff ist ungültig (eine Zeile, höchstens 200 Zeichen).");
        }
        if self.body_text.trim().is_empty() {
            return bad("Der Text der Nachricht fehlt.");
        }
        Ok(())
    }
}

/// Anhaenge (B5): hoechstens so viele, zusammen hoechstens so gross (die Nachricht ist mit
/// base64 hoechstens `MAX_MESSAGE_BYTES` gross).
pub const MAX_ATTACHMENTS: usize = 5;
pub const MAX_ATTACHMENT_BYTES: usize = 2_560 * 1024;
const MAX_ATTACHMENT_NAME_CHARS: usize = 150;

/// Ein Anhang einer Mail (Inhalt im Speicher).
#[derive(Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nie den Inhalt in ein Protokoll.
        write!(
            f,
            "Attachment {{ name: {:?}, content_type: {:?}, bytes: {} }}",
            self.name,
            self.content_type,
            self.bytes.len()
        )
    }
}

/// Prueft die Anhaenge (Anzahl, Namen, Groesse); nichts wird gekuerzt oder weggelassen.
pub fn validate_attachments(list: &[Attachment]) -> Result<(), SmtpError> {
    let bad = |m: &'static str| Err(SmtpError::InvalidMessage(m));
    if list.len() > MAX_ATTACHMENTS {
        return bad("Zu viele Anhänge (höchstens 5).");
    }
    let mut total = 0usize;
    for a in list {
        let name = a.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_ATTACHMENT_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return bad("Der Name eines Anhangs ist ungültig.");
        }
        if a.bytes.is_empty() {
            return bad("Ein Anhang ist leer.");
        }
        total += a.bytes.len();
    }
    if total > MAX_ATTACHMENT_BYTES {
        return bad("Die Anhänge sind zu groß (höchstens 2,5 MiB zusammen).");
    }
    Ok(())
}

/// Baut die Nachricht (RFC 5322, MIME). Rein: Datum und Kennung kommen von aussen.
pub fn build_message(
    cfg: &SmtpConfig,
    msg: &MailMessage,
    date: DateTime<Utc>,
    id_seed: u128,
) -> Result<(Vec<u8>, String), SmtpError> {
    build_message_with(cfg, msg, &[], date, id_seed)
}

/// Wie [`build_message`], mit Anhaengen.
pub fn build_message_with(
    cfg: &SmtpConfig,
    msg: &MailMessage,
    attachments: &[Attachment],
    date: DateTime<Utc>,
    id_seed: u128,
) -> Result<(Vec<u8>, String), SmtpError> {
    msg.validate()?;
    validate_attachments(attachments)?;
    let domain = cfg
        .from_address
        .rsplit('@')
        .next()
        .unwrap_or("local-voice-ai.invalid");
    let message_id = format!("lva-{id_seed:032x}@{domain}");
    let from_name: Option<String> =
        (!cfg.from_name.trim().is_empty()).then(|| cfg.from_name.trim().to_string());
    let to: Vec<Address<'_>> = msg
        .to
        .iter()
        .map(|a| Address::new_address(None::<&str>, a.trim().to_string()))
        .collect();
    let mut b = MessageBuilder::new()
        .message_id(message_id.clone())
        .date(Date::new(date.timestamp()))
        .from(Address::new_address(from_name, cfg.from_address.clone()))
        .to(Address::new_list(to))
        .subject(msg.subject.trim().to_string())
        .text_body(msg.body_text.clone());
    if !msg.cc.is_empty() {
        let cc: Vec<Address<'_>> = msg
            .cc
            .iter()
            .map(|a| Address::new_address(None::<&str>, a.trim().to_string()))
            .collect();
        b = b.cc(Address::new_list(cc));
    }
    if let Some(html) = msg.body_html.as_deref().filter(|h| !h.trim().is_empty()) {
        b = b.html_body(html.to_string());
    }
    for a in attachments {
        b = b.attachment(
            a.content_type.clone(),
            a.name.trim().to_string(),
            a.bytes.clone(),
        );
    }
    let bytes = b
        .write_to_vec()
        .map_err(|_| SmtpError::InvalidMessage("Die Nachricht ließ sich nicht aufbauen."))?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(SmtpError::InvalidMessage(
            "Die Nachricht ist zu groß (höchstens 5 MiB).",
        ));
    }
    Ok((bytes, message_id))
}

/// SMTP-Daten: Zeilenenden CRLF, Zeilen mit fuehrendem Punkt verdoppeln.
fn dot_stuff(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 64);
    let mut at_line_start = true;
    let mut prev = 0u8;
    for &b in message {
        // Alleinstehendes LF zu CRLF.
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        out.push(b);
        at_line_start = b == b'\n';
        prev = b;
    }
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Verbindung
// ---------------------------------------------------------------------------

enum Transport {
    Plain(TcpStream),
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.read(buf),
            Transport::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.write(buf),
            Transport::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Transport::Plain(s) => s.flush(),
            Transport::Tls(s) => s.flush(),
        }
    }
}

#[derive(Debug)]
struct Reply {
    code: u16,
    lines: Vec<String>,
}

impl Reply {
    fn text(&self) -> String {
        self.lines.join(" ")
    }

    /// Faehigkeiten aus der EHLO-Antwort (grossgeschrieben).
    fn capabilities(&self) -> Vec<String> {
        self.lines
            .iter()
            .skip(1)
            .map(|l| l.trim().to_uppercase())
            .collect()
    }
}

struct Conn {
    reader: BufReader<Transport>,
    host: String,
    opts: ConnectOpts,
}

fn map_io(e: std::io::Error) -> SmtpError {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => SmtpError::Timeout,
        _ => SmtpError::Connect(e.to_string()),
    }
}

impl Conn {
    fn open(cfg: &SmtpConfig, opts: &ConnectOpts) -> Result<Self, SmtpError> {
        let host = cfg.host.trim().to_string();
        let addrs: Vec<_> = (host.as_str(), cfg.port)
            .to_socket_addrs()
            .map_err(|e| SmtpError::Connect(format!("{host}: {e}")))?
            .collect();
        let mut last: Option<std::io::Error> = None;
        let mut stream = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, opts.connect_timeout) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        let stream = match stream {
            Some(s) => s,
            None => {
                return Err(last.map(map_io).unwrap_or_else(|| {
                    SmtpError::Connect(format!("{host}: keine Adresse gefunden"))
                }))
            }
        };
        stream
            .set_read_timeout(Some(opts.io_timeout))
            .map_err(map_io)?;
        stream
            .set_write_timeout(Some(opts.io_timeout))
            .map_err(map_io)?;
        let _ = stream.set_nodelay(true);
        let transport = if cfg.security == Security::Tls {
            Transport::Tls(Box::new(tls_handshake(&host, stream, opts)?))
        } else {
            Transport::Plain(stream)
        };
        Ok(Self {
            reader: BufReader::new(transport),
            host,
            opts: opts.clone(),
        })
    }

    fn read_reply(&mut self) -> Result<Reply, SmtpError> {
        let mut lines = Vec::new();
        loop {
            let mut buf = Vec::new();
            let n = self
                .reader
                .by_ref()
                .take(MAX_LINE as u64)
                .read_until(b'\n', &mut buf)
                .map_err(map_io)?;
            if n == 0 {
                return Err(SmtpError::Protocol(
                    "Die Verbindung wurde vom Server beendet.".to_string(),
                ));
            }
            if !buf.ends_with(b"\n") {
                return Err(SmtpError::Protocol("Antwortzeile zu lang.".to_string()));
            }
            let line = String::from_utf8_lossy(&buf).trim_end().to_string();
            if line.len() < 3 || !line.is_char_boundary(3) {
                return Err(SmtpError::Protocol(
                    crate::managers::integrations::audit::sanitize_audit_text(&line, 80),
                ));
            }
            let code: u16 = line[..3].parse().map_err(|_| {
                SmtpError::Protocol(crate::managers::integrations::audit::sanitize_audit_text(
                    &line, 80,
                ))
            })?;
            let more = line.as_bytes().get(3) == Some(&b'-');
            let rest = line.get(4..).unwrap_or("").to_string();
            lines.push(rest);
            if lines.len() > MAX_REPLY_LINES {
                return Err(SmtpError::Protocol("Antwort zu lang.".to_string()));
            }
            if !more {
                return Ok(Reply { code, lines });
            }
        }
    }

    fn send_raw(&mut self, bytes: &[u8]) -> Result<(), SmtpError> {
        let w = self.reader.get_mut();
        w.write_all(bytes).map_err(map_io)?;
        w.flush().map_err(map_io)
    }

    fn command(&mut self, line: &str) -> Result<Reply, SmtpError> {
        debug_assert!(!line.contains(['\r', '\n']));
        self.send_raw(format!("{line}\r\n").as_bytes())?;
        self.read_reply()
    }

    fn expect(&mut self, line: &str, ok: &[u16], stage: &'static str) -> Result<Reply, SmtpError> {
        let reply = self.command(line)?;
        self.check(reply, ok, stage)
    }

    fn check(&self, reply: Reply, ok: &[u16], stage: &'static str) -> Result<Reply, SmtpError> {
        if ok.contains(&reply.code) {
            return Ok(reply);
        }
        let text = crate::managers::integrations::audit::sanitize_audit_text(
            &format!("{} {}", reply.code, reply.text()),
            160,
        );
        if reply.code >= 400 && reply.code < 600 {
            Err(SmtpError::Rejected { stage, text })
        } else {
            Err(SmtpError::Protocol(text))
        }
    }

    fn ehlo(&mut self) -> Result<Reply, SmtpError> {
        let r = self.command("EHLO localhost")?;
        self.check(r, &[250], "greeting")
    }

    /// Stuft die Verbindung auf TLS hoch (nur nach `220` auf `STARTTLS`).
    fn upgrade(self) -> Result<Self, SmtpError> {
        if !self.reader.buffer().is_empty() {
            // Der Server hat vor dem Handshake Daten nachgeschoben: Einschleusung.
            return Err(SmtpError::Tls(
                "Der Server sendete Daten vor dem Verbindungsaufbau.".to_string(),
            ));
        }
        let Conn { reader, host, opts } = self;
        let Transport::Plain(stream) = reader.into_inner() else {
            return Err(SmtpError::Tls(
                "Die Verbindung ist schon verschlüsselt.".to_string(),
            ));
        };
        let tls = tls_handshake(&host, stream, &opts)?;
        Ok(Conn {
            reader: BufReader::new(Transport::Tls(Box::new(tls))),
            host,
            opts,
        })
    }

    fn is_encrypted(&self) -> bool {
        matches!(self.reader.get_ref(), Transport::Tls(_))
    }
}

fn tls_handshake(
    host: &str,
    stream: TcpStream,
    opts: &ConnectOpts,
) -> Result<native_tls::TlsStream<TcpStream>, SmtpError> {
    let mut builder = native_tls::TlsConnector::builder();
    builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
    #[cfg(test)]
    if let Some(pem) = &opts.extra_root_pem {
        let root =
            native_tls::Certificate::from_pem(pem).map_err(|e| SmtpError::Tls(e.to_string()))?;
        builder.add_root_certificate(root);
    }
    let _ = opts;
    let connector = builder.build().map_err(|e| SmtpError::Tls(e.to_string()))?;
    connector.connect(host, stream).map_err(|e| match e {
        native_tls::HandshakeError::Failure(e) => SmtpError::Tls(e.to_string()),
        native_tls::HandshakeError::WouldBlock(_) => SmtpError::Timeout,
    })
}

/// Verbindet, handelt TLS aus, meldet sich an. Danach ist die Sitzung bereit fuer
/// `MAIL FROM`.
fn session(cfg: &SmtpConfig, password: &str, opts: &ConnectOpts) -> Result<Conn, SmtpError> {
    cfg.validate()?;
    let mut conn = Conn::open(cfg, opts)?;
    let greeting = conn.read_reply()?;
    conn.check(greeting, &[220], "greeting")?;
    let mut hello = conn.ehlo()?;
    if cfg.security == Security::Starttls {
        if !hello.capabilities().iter().any(|c| c == "STARTTLS") {
            return Err(SmtpError::StarttlsUnsupported);
        }
        conn.expect("STARTTLS", &[220], "greeting")?;
        conn = conn.upgrade()?;
        hello = conn.ehlo()?;
    }
    if cfg.username.is_empty() {
        // Ohne Anmeldung: nur gegen den eigenen Rechner sinnvoll.
        if !is_loopback_host(&cfg.host) {
            return Err(SmtpError::Config(
                "Der Benutzername fehlt (ohne Anmeldung nimmt ein fremder Server keine Mail an).",
            ));
        }
        return Ok(conn);
    }
    if password.is_empty() {
        return Err(SmtpError::PasswordMissing);
    }
    // Das Passwort geht nie im Klartext ueber ein unverschluesseltes Netz.
    if !conn.is_encrypted() && !is_loopback_host(&cfg.host) {
        return Err(SmtpError::PlainNotAllowed);
    }
    authenticate(&mut conn, &hello, &cfg.username, password)?;
    Ok(conn)
}

fn authenticate(
    conn: &mut Conn,
    hello: &Reply,
    user: &str,
    password: &str,
) -> Result<(), SmtpError> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mechs: Vec<String> = hello
        .capabilities()
        .iter()
        .filter_map(|c| {
            c.strip_prefix("AUTH ")
                .or_else(|| c.strip_prefix("AUTH="))
                .map(str::to_string)
        })
        .flat_map(|m| m.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .collect();
    let auth_failed = |r: Reply| -> SmtpError {
        if r.code == 535 || r.code == 534 || r.code == 530 || r.code == 454 {
            SmtpError::AuthFailed
        } else {
            SmtpError::Protocol(crate::managers::integrations::audit::sanitize_audit_text(
                &format!("{} {}", r.code, r.text()),
                160,
            ))
        }
    };
    if mechs.iter().any(|m| m == "PLAIN") {
        let line = Zeroizing::new(format!(
            "AUTH PLAIN {}",
            b64.encode(format!("\0{user}\0{password}"))
        ));
        let r = conn.command(&line)?;
        return if r.code == 235 {
            Ok(())
        } else {
            Err(auth_failed(r))
        };
    }
    if mechs.iter().any(|m| m == "LOGIN") {
        let r = conn.command("AUTH LOGIN")?;
        if r.code != 334 {
            return Err(auth_failed(r));
        }
        let r = conn.command(&b64.encode(user))?;
        if r.code != 334 {
            return Err(auth_failed(r));
        }
        let line = Zeroizing::new(b64.encode(password));
        let r = conn.command(&line)?;
        return if r.code == 235 {
            Ok(())
        } else {
            Err(auth_failed(r))
        };
    }
    Err(SmtpError::AuthUnsupported)
}

/// Ergebnis eines Versands (fuer Protokoll und Anzeige, ohne Inhalt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendReceipt {
    pub recipients: usize,
    pub bytes: usize,
    pub message_id: String,
}

/// Warum ein Versand scheiterte, und ob die Nachricht trotzdem angekommen sein kann.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendFailure {
    pub error: SmtpError,
    /// Die Nachricht war schon unterwegs (Daten gesendet), als es scheiterte, und der Server hat
    /// sie nicht ausdruecklich abgelehnt: ob sie ankam, ist unklar. Wer sicher sein will, dass
    /// nichts geschah (`false`), darf es erneut versuchen; bei `true` nie von selbst.
    pub maybe_delivered: bool,
}

/// Sendet eine Mail. Gebaut wird zuerst (Fehler in der Nachricht kosten keine
/// Verbindung), dann Verbindung, TLS, Anmeldung, Umschlag, Daten.
pub fn send(
    cfg: &SmtpConfig,
    password: &str,
    msg: &MailMessage,
    opts: &ConnectOpts,
) -> Result<SendReceipt, SmtpError> {
    send_with(cfg, password, msg, &[], opts, None).map_err(|f| f.error)
}

/// Wie [`send`], mit Anhaengen und einer festen Kennung fuer die `Message-ID` (`id_seed`:
/// dieselbe Kennung ergibt dieselbe `Message-ID`, damit ein Wiederholungsversuch fuer Server
/// und Postfaecher als dieselbe Nachricht erkennbar ist; `None`: zufaellig).
pub fn send_with(
    cfg: &SmtpConfig,
    password: &str,
    msg: &MailMessage,
    attachments: &[Attachment],
    opts: &ConnectOpts,
    id_seed: Option<u128>,
) -> Result<SendReceipt, SendFailure> {
    let early = |error: SmtpError| SendFailure {
        error,
        maybe_delivered: false,
    };
    let seed = id_seed.unwrap_or_else(|| u128::from_le_bytes(rand::random::<[u8; 16]>()));
    let (message, message_id) =
        build_message_with(cfg, msg, attachments, Utc::now(), seed).map_err(early)?;
    let mut conn = session(cfg, password, opts).map_err(early)?;
    conn.expect(
        &format!("MAIL FROM:<{}>", cfg.from_address),
        &[250],
        "sender",
    )
    .map_err(early)?;
    let recipients = msg.recipients();
    for rcpt in &recipients {
        conn.expect(&format!("RCPT TO:<{rcpt}>"), &[250, 251], "recipient")
            .map_err(early)?;
    }
    let r = conn.command("DATA").map_err(early)?;
    conn.check(r, &[354], "data").map_err(early)?;
    let body = dot_stuff(&message);
    // Ab hier ist die Nachricht unterwegs: jeder Fehler ausser einer ausdruecklichen
    // Ablehnung der Daten laesst offen, ob sie ankam.
    let late = |error: SmtpError| SendFailure {
        maybe_delivered: !matches!(&error, SmtpError::Rejected { stage: "data", .. }),
        error,
    };
    conn.send_raw(&body).map_err(late)?;
    conn.send_raw(b".\r\n").map_err(late)?;
    let r = conn.read_reply().map_err(late)?;
    conn.check(r, &[250], "data").map_err(late)?;
    let _ = conn.command("QUIT");
    Ok(SendReceipt {
        recipients: recipients.len(),
        bytes: message.len(),
        message_id,
    })
}

/// Probiert Verbindung, Verschluesselung und Anmeldung aus, ohne eine Mail zu
/// senden.
pub fn test(cfg: &SmtpConfig, password: &str, opts: &ConnectOpts) -> Result<(), SmtpError> {
    let mut conn = session(cfg, password, opts)?;
    let _ = conn.command("QUIT");
    let _ = &conn.host;
    Ok(())
}

#[cfg(test)]
mod tests;
