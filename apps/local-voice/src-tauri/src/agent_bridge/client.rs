//! Gegenstelle der Agentenbruecke: ein schlanker, blockierender Client fuer `ctl` und fuer den
//! MCP-Proxy (A8). Eine Anfrage nach der anderen, eine JSON-Zeile je Anfrage und Antwort.
//!
//! Der Client verlaesst sich nicht blind auf den Namen der Pipe: unter Windows prueft
//! `pipe::open_client`, dass der Server-Prozess demselben Benutzer gehoert (Schutz vor einem
//! untergeschobenen Server), und oeffnet die Pipe ohne Erlaubnis, sich als der Client auszugeben.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};

use serde_json::{json, Value};

use super::protocol::code;
use super::{pipe, MAX_LINE_BYTES};

/// Warum ein Aufruf nichts geliefert hat.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientError {
    /// Es gibt keine laufende App (oder die Pipe ist nicht erreichbar).
    NotRunning(String),
    /// Am anderen Ende der Pipe sitzt nicht der erwartete Server (anderer Benutzer).
    UntrustedServer(String),
    /// Die Verbindung brach ab oder liess sich nicht lesen/schreiben.
    Transport(String),
    /// Die Antwort war kein gueltiges Protokoll.
    Protocol(String),
    /// Der Server hat die Anfrage abgelehnt.
    Remote {
        code: String,
        message: String,
        data: Option<Value>,
    },
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotRunning(m)
            | ClientError::UntrustedServer(m)
            | ClientError::Transport(m)
            | ClientError::Protocol(m) => write!(f, "{m}"),
            ClientError::Remote { code, message, .. } => write!(f, "{code}: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Eine Verbindung zur Bruecke.
pub struct Client<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    next_id: u64,
}

impl Client<BufReader<File>, File> {
    /// Verbindet sich mit der Pipe `name` (siehe `pipe::open_client`).
    pub fn connect(name: &str) -> Result<Self, ClientError> {
        let file = pipe::open_client(name)?;
        let reader = file
            .try_clone()
            .map_err(|e| ClientError::Transport(format!("Pipe nicht lesbar: {e}")))?;
        Ok(Client::new(BufReader::new(reader), file))
    }
}

impl<R: BufRead, W: Write> Client<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            next_id: 0,
        }
    }

    /// Sendet eine Anfrage und liefert `result` (oder den Fehler des Servers).
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        self.next_id += 1;
        let id = self.next_id;
        let line = json!({ "id": id, "method": method, "params": params }).to_string();
        if line.len() > MAX_LINE_BYTES {
            return Err(ClientError::Protocol("Die Anfrage ist zu groß.".to_string()));
        }
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .and_then(|()| self.writer.flush())
            .map_err(|e| ClientError::Transport(format!("Senden fehlgeschlagen: {e}")))?;
        let reply = self.read_line()?;
        let value: Value = serde_json::from_str(&reply)
            .map_err(|_| ClientError::Protocol("Die Antwort ist kein JSON.".to_string()))?;
        if let Some(err) = value.get("error") {
            return Err(ClientError::Remote {
                code: err["code"].as_str().unwrap_or("unknown").to_string(),
                message: err["message"].as_str().unwrap_or("").to_string(),
                data: err.get("data").cloned(),
            });
        }
        // Eine Zeile ohne passende Kennung (z. B. „zu viele Verbindungen“ beim Verbinden) ist nur
        // als Fehler gueltig; ein Ergebnis muss zur Anfrage gehoeren.
        if value.get("id") != Some(&json!(id)) {
            return Err(ClientError::Protocol(
                "Die Antwort gehört zu einer anderen Anfrage.".to_string(),
            ));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| ClientError::Protocol("Die Antwort hat kein Ergebnis.".to_string()))
    }

    fn read_line(&mut self) -> Result<String, ClientError> {
        let mut buf = Vec::new();
        loop {
            let chunk = self
                .reader
                .fill_buf()
                .map_err(|e| ClientError::Transport(format!("Lesen fehlgeschlagen: {e}")))?;
            if chunk.is_empty() {
                return Err(ClientError::Transport(
                    "Die Verbindung zu Local Voice AI wurde beendet.".to_string(),
                ));
            }
            match chunk.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    buf.extend_from_slice(&chunk[..pos]);
                    self.reader.consume(pos + 1);
                    break;
                }
                None => {
                    let n = chunk.len();
                    buf.extend_from_slice(chunk);
                    self.reader.consume(n);
                }
            }
            if buf.len() > 2 * MAX_LINE_BYTES {
                return Err(ClientError::Protocol("Die Antwort ist zu lang.".to_string()));
            }
        }
        String::from_utf8(buf).map_err(|_| ClientError::Protocol("Antwort nicht UTF-8.".to_string()))
    }

    /// Meldet die Verbindung an (`token`: `None` = anonym, nur `status`).
    pub fn hello(&mut self, token: Option<&str>) -> Result<Value, ClientError> {
        let params = match token {
            Some(t) => json!({ "token": t }),
            None => json!({}),
        };
        self.request("hello", params)
    }
}

/// Fehlercode eines `Remote`-Fehlers, falls es einer ist.
pub fn remote_code(e: &ClientError) -> Option<&str> {
    match e {
        ClientError::Remote { code, .. } => Some(code.as_str()),
        _ => None,
    }
}

/// Ist der Fehler ein Anmeldefehler (Token ungueltig/zurueckgezogen/fehlt)?
pub fn is_auth_error(e: &ClientError) -> bool {
    matches!(
        remote_code(e),
        Some(code::TOKEN_INVALID | code::TOKEN_REVOKED | code::UNAUTHENTICATED)
    )
}

#[cfg(test)]
mod tests;
