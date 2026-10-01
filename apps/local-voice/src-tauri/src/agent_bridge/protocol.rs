//! Zeilenprotokoll der Agentenbruecke: je Zeile ein JSON-Objekt, Anfrage und Antwort im
//! Wechsel (eine Anfrage zur Zeit je Verbindung). Angelehnt an JSON-RPC 2.0, aber mit
//! Text-Fehlercodes, die Skripte und `ctl` direkt auswerten koennen.
//!
//! Anfrage:  `{"id": <Text|Zahl|null>, "method": "<name>", "params": { ... }}`
//! Antwort:  `{"id": <wie in der Anfrage>, "result": { ... }}`
//!           `{"id": ..., "error": {"code": "<code>", "message": "<Klartext>", "data": {...}}}`
//!
//! Methoden:
//! - `hello`          `{token?, name?}` meldet die Verbindung an (ohne Token: nur `status`).
//! - `status`         Lebenszeichen und Stand; ohne Anmeldung nur Name, Version, Protokoll.
//! - `tools/list`     die Werkzeuge, die dieser Zugang jetzt benutzen darf (Modus `ask`/`allow`).
//! - `tools/call`     `{name, arguments?, approval_id?}`; Antwort `{status: "done"|"pending", ...}`.
//! - `approval/status` `{approval_id}` Stand einer eigenen Freigabe.
//!
//! Es gibt KEINE Methode, die eine Freigabe entscheidet, ein Recht aendert oder einen Zugang
//! anlegt: das tut nur der Nutzer in der Oberflaeche.

use std::io;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Laengste Kennung (`id`), die eine Anfrage tragen darf, serialisiert in Bytes. Die Antwort
/// traegt sie zurueck: ohne Grenze liesse sich mit einer 1-MiB-Kennung jede Antwort aufblasen (B23).
pub const MAX_ID_BYTES: usize = 128;

/// Fehlercodes (stabil, von `ctl` und dem MCP-Proxy ausgewertet).
pub mod code {
    pub const BAD_REQUEST: &str = "bad_request";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const LINE_TOO_LONG: &str = "line_too_long";
    pub const UNAUTHENTICATED: &str = "unauthenticated";
    pub const TOKEN_INVALID: &str = "token_invalid";
    pub const TOKEN_REVOKED: &str = "token_revoked";
    pub const TOOL_OFF: &str = "tool_off";
    pub const UNKNOWN_TOOL: &str = "unknown_tool";
    pub const TOOL_UNAVAILABLE: &str = "tool_unavailable";
    pub const DENIED: &str = "denied";
    pub const APPROVAL_DENIED: &str = "approval_denied";
    pub const APPROVAL_EXPIRED: &str = "approval_expired";
    pub const APPROVAL_NOT_FOUND: &str = "approval_not_found";
    pub const APPROVAL_USED: &str = "approval_used";
    pub const APPROVAL_MISMATCH: &str = "approval_mismatch";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const TOO_MANY_CONNECTIONS: &str = "too_many_connections";
    pub const STORE_UNAVAILABLE: &str = "store_unavailable";
    pub const FAILED: &str = "failed";
    pub const SHUTTING_DOWN: &str = "shutting_down";
    pub const REPLY_TOO_LARGE: &str = "reply_too_large";
}

/// Eine gelesene Anfrage oder der Grund, warum die Zeile keine ist.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Invalid {
        id: Value,
        message: String,
    },
}

/// Zerlegt eine Zeile. `params` ist immer ein Objekt (fehlt es, `{}`).
pub fn parse_request(line: &str) -> Incoming {
    let invalid = |id: Value, message: &str| Incoming::Invalid {
        id,
        message: message.to_string(),
    };
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return invalid(Value::Null, "Die Zeile ist kein gültiges JSON.");
    };
    let Value::Object(mut map) = value else {
        return invalid(Value::Null, "Die Anfrage muss ein JSON-Objekt sein.");
    };
    let id = map.remove("id").unwrap_or(Value::Null);
    if !matches!(id, Value::Null | Value::String(_) | Value::Number(_)) {
        return invalid(Value::Null, "Die Kennung (id) muss Text, Zahl oder null sein.");
    }
    // Die Kennung kommt in jeder Antwort zurueck: ohne Grenze liesse sich eine Antwort mit
    // bis zu 1 MiB aufblasen (B23). Zu lang: abgelehnt, ohne sie zu spiegeln.
    if id.to_string().len() > MAX_ID_BYTES {
        return invalid(
            Value::Null,
            &format!("Die Kennung (id) ist zu lang (höchstens {MAX_ID_BYTES} Zeichen)."),
        );
    }
    let Some(Value::String(method)) = map.remove("method") else {
        return invalid(id, "Die Methode (method) fehlt oder ist kein Text.");
    };
    if method.is_empty() || method.len() > 64 {
        return invalid(id, "Die Methode ist leer oder zu lang.");
    }
    let params = match map.remove("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p,
        Some(_) => return invalid(id, "params muss ein Objekt sein."),
    };
    Incoming::Request { id, method, params }
}

/// Erfolgsantwort als Zeile (ohne Zeilenende).
pub fn ok_line(id: &Value, result: Value) -> String {
    json!({ "id": id, "result": result }).to_string()
}

/// Fehlerantwort als Zeile (ohne Zeilenende).
pub fn err_line(id: &Value, code: &str, message: &str, data: Option<&Value>) -> String {
    let mut error = json!({ "code": code, "message": message });
    if let Some(d) = data {
        error["data"] = d.clone();
    }
    json!({ "id": id, "error": error }).to_string()
}

/// Begrenzt eine Antwortzeile auf `max` Bytes; zu lange wird durch einen kurzen Fehler mit
/// derselben Kennung ersetzt (B23). Die Kennung ist durch `MAX_ID_BYTES` klein, der Ersatz also
/// immer kurz.
pub fn cap_reply(id: &Value, line: String, max: usize) -> String {
    if line.len() <= max {
        return line;
    }
    err_line(
        id,
        code::REPLY_TOO_LARGE,
        "Die Antwort ist zu groß und wird nicht übertragen.",
        None,
    )
}

/// Was `read_line_capped` gelesen hat.
#[derive(Debug, PartialEq, Eq)]
pub enum LineRead {
    Line(String),
    Eof,
    /// Die Zeile war laenger als erlaubt; der Rest wurde nicht gelesen (die Verbindung wird geschlossen).
    TooLong,
    InvalidUtf8,
}

/// Liest eine Zeile, hoechstens `max` Bytes (ohne Zeilenende). Nie mehr als `max` plus
/// ein Lesepuffer im Speicher.
pub async fn read_line_capped<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> io::Result<LineRead> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            // Gegenstelle zu; eine unvollstaendige letzte Zeile ohne Zeilenende zaehlt als Zeile.
            return Ok(if buf.is_empty() {
                LineRead::Eof
            } else {
                finish(buf)
            });
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                if buf.len() + pos > max {
                    return Ok(LineRead::TooLong);
                }
                buf.extend_from_slice(&chunk[..pos]);
                reader.consume(pos + 1);
                return Ok(finish(buf));
            }
            None => {
                let n = chunk.len();
                if buf.len() + n > max {
                    return Ok(LineRead::TooLong);
                }
                buf.extend_from_slice(chunk);
                reader.consume(n);
            }
        }
    }
}

fn finish(mut buf: Vec<u8>) -> LineRead {
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    match String::from_utf8(buf) {
        Ok(s) => LineRead::Line(s),
        Err(_) => LineRead::InvalidUtf8,
    }
}

#[cfg(test)]
mod tests;
