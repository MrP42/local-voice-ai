//! JSON-RPC 2.0 und der MCP-Rahmen fuer `--mcp` (stdio, eine Nachricht je
//! Zeile). Reine Funktionen ohne Datenbank und ohne Einstellungen: was hier
//! steht, ist mit festen Transkripten testbar.
//!
//! Regeln, die durchgehend gelten:
//! - Jede Antwort ist EIN JSON-Wert ohne Zeilenumbruch (`serde_json` maskiert
//!   Umbrueche in Zeichenketten), damit der Client sie zeilenweise liest.
//! - Notifications (Nachrichten ohne `id`) bekommen NIE eine Antwort, auch
//!   nicht bei unbekannter Methode.
//! - Batches (JSON-Array) kennt MCP seit 2025-06-18 nicht mehr: `-32600`.
//! - Eine Eingabezeile ist auf `MAX_LINE_BYTES` begrenzt; eine laengere wird
//!   verworfen (nicht gepuffert), der Server laeuft weiter.

use serde_json::{json, Value};
use std::io::{self, BufRead};

/// Protokollversionen mit `initialize`-Handshake („alt“).
pub const LEGACY_VERSIONS: [&str; 2] = ["2025-06-18", "2025-11-25"];
/// Die zustandslose Fassung (A8, R9): jede Anfrage traegt Version und Faehigkeiten des Clients in
/// `_meta`, es gibt keinen Handshake; `server/discover` nennt die Versionen des Servers.
pub const MODERN_PROTOCOL: &str = "2026-07-28";
/// Alle Versionen, die dieser Server spricht (aelteste zuerst). Er ist „dual-era“: eine Anfrage
/// mit `_meta` wird zustandslos nach 2026-07-28 bedient, ein `initialize` waehlt den alten
/// Ablauf (eine Anfrage ganz ohne beides wird wie bisher tolerant als alt behandelt).
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-11-25", "2026-07-28"];
/// Antwort auf ein `initialize` mit unbekannter Version: die neueste Handshake-Version. Ein
/// Handshake kann keine zustandslose Version waehlen.
pub const LATEST_PROTOCOL: &str = "2025-11-25";

/// Schluessel in `_meta` (Fassung 2026-07-28).
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
/// Fehlercode „Protokollversion nicht unterstuetzt“ (Fassung 2026-07-28).
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// Laengste angenommene Eingabezeile (1 MiB). Eine Anfrage dieses Servers ist
/// wenige hundert Byte lang; mehr ist ein Fehler des Clients, kein Grund, den
/// Speicher zu fuellen.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// Was eine Eingabezeile ist.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// Leerzeile oder eine Antwort des Clients (dieser Server fragt nie): still uebergehen.
    Ignore,
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// Nachricht ohne `id`: wird ausgefuehrt/uebergangen, aber nie beantwortet.
    Notification { method: String },
    /// Nicht verarbeitbar; `id` ist `null`, wenn keine brauchbare vorlag.
    Invalid {
        id: Value,
        code: i64,
        message: String,
    },
}

fn invalid(id: Value, code: i64, message: &str) -> Incoming {
    Incoming::Invalid {
        id,
        code,
        message: message.to_string(),
    }
}

/// Ordnet eine Eingabezeile ein. Wirft nie.
pub fn parse_message(line: &str) -> Incoming {
    // PowerShell und manche Clients stellen der ersten Zeile eine BOM voran.
    let line = line.trim_start_matches('\u{feff}').trim();
    if line.is_empty() {
        return Incoming::Ignore;
    }
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return invalid(Value::Null, PARSE_ERROR, "Parse error"),
    };
    let Value::Object(mut object) = value else {
        return invalid(
            Value::Null,
            INVALID_REQUEST,
            "Invalid Request: batches are not supported",
        );
    };
    // Nur eine Zahl oder Zeichenkette ist als `id` in eine Fehlerantwort zu spiegeln.
    let echo_id = match object.get("id") {
        Some(id @ (Value::String(_) | Value::Number(_))) => id.clone(),
        _ => Value::Null,
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid(
            echo_id,
            INVALID_REQUEST,
            "Invalid Request: jsonrpc must be \"2.0\"",
        );
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        if object.contains_key("result") || object.contains_key("error") {
            return Incoming::Ignore;
        }
        return invalid(echo_id, INVALID_REQUEST, "Invalid Request: method missing");
    };
    let method = method.to_string();
    let params = object.remove("params").unwrap_or(Value::Null);
    match object.get("id") {
        None => Incoming::Notification { method },
        Some(Value::String(_) | Value::Number(_)) => Incoming::Request {
            id: echo_id,
            method,
            params,
        },
        // MCP: die `id` einer Anfrage ist nie `null`.
        Some(_) => invalid(
            Value::Null,
            INVALID_REQUEST,
            "Invalid Request: id must be a string or a number",
        ),
    }
}

/// Erfolgsantwort als eine Zeile (ohne Zeilenende).
pub fn result_line(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// Fehlerantwort als eine Zeile (ohne Zeilenende).
pub fn error_line(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Versionsaushandlung des alten Handshakes: die angefragte Version, wenn wir sie mit
/// Handshake sprechen, sonst die neueste Handshake-Version (der Client entscheidet dann, ob er
/// damit leben kann).
pub fn negotiate_version(requested: &str) -> &'static str {
    LEGACY_VERSIONS
        .iter()
        .copied()
        .find(|v| *v == requested)
        .unwrap_or(LATEST_PROTOCOL)
}

/// Fehlerantwort mit `data` (z. B. die unterstuetzten Versionen) als eine Zeile.
pub fn error_line_data(id: &Value, code: i64, message: &str, data: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message, "data": data } })
        .to_string()
}

/// Wie eine Anfrage sich ausgibt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    /// Ohne Versionsangabe in `_meta` (oder mit einer Handshake-Version): Verhalten wie bisher.
    Legacy,
    /// Mit `_meta` der Fassung 2026-07-28: zustandslos, Ergebnisse tragen `resultType`.
    Modern,
}

/// Warum eine Anfrage wegen ihres `_meta` abgewiesen wird.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl MetaError {
    pub fn line(&self, id: &Value) -> String {
        match &self.data {
            Some(d) => error_line_data(id, self.code, &self.message, d.clone()),
            None => error_line(id, self.code, &self.message),
        }
    }
}

/// Ordnet eine Anfrage nach ihrem `_meta` ein (Fassung 2026-07-28):
/// - keine Version in `_meta`: `Legacy` (kein Handshake noetig, wie bisher);
/// - eine Handshake-Version: `Legacy`;
/// - `2026-07-28`: `Modern`, dann ist `clientCapabilities` Pflicht (sonst `-32602`);
/// - jede andere Version: `-32022` mit `data.supported` und `data.requested`.
pub fn classify(params: &Value) -> Result<Era, MetaError> {
    let Some(requested) = params
        .get("_meta")
        .and_then(|m| m.get(META_PROTOCOL_VERSION))
    else {
        return Ok(Era::Legacy);
    };
    let Some(version) = requested.as_str() else {
        return Err(MetaError {
            code: INVALID_PARAMS,
            message: format!("Invalid params: _meta.{META_PROTOCOL_VERSION} must be a string"),
            data: None,
        });
    };
    if LEGACY_VERSIONS.contains(&version) {
        return Ok(Era::Legacy);
    }
    if version == MODERN_PROTOCOL {
        let caps = params["_meta"].get(META_CLIENT_CAPABILITIES);
        if !caps.is_some_and(Value::is_object) {
            return Err(MetaError {
                code: INVALID_PARAMS,
                message: format!("Invalid params: _meta.{META_CLIENT_CAPABILITIES} missing"),
                data: None,
            });
        }
        return Ok(Era::Modern);
    }
    let shown: String = version.chars().take(40).collect();
    Err(MetaError {
        code: UNSUPPORTED_PROTOCOL_VERSION,
        message: "Unsupported protocol version".to_string(),
        data: Some(json!({
            "supported": [MODERN_PROTOCOL, LEGACY_VERSIONS[1], LEGACY_VERSIONS[0]],
            "requested": shown,
        })),
    })
}

/// Ergebnis von [`read_line_capped`].
#[derive(Debug, PartialEq, Eq)]
pub enum LineRead {
    /// Eingabe zu Ende (der Client hat stdin geschlossen).
    Eof,
    /// Eine Zeile ohne Zeilenende.
    Line(String),
    /// Die Zeile war laenger als das Limit; sie ist verworfen, der Rest der
    /// Zeile ebenfalls, die naechste Zeile ist wieder lesbar.
    TooLong,
    /// Die Zeile war kein UTF-8.
    InvalidUtf8,
}

/// Liest eine Zeile, ohne je mehr als `max` Byte zu halten.
pub fn read_line_capped<R: BufRead>(reader: &mut R, max: usize) -> io::Result<LineRead> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    let mut got_any = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            if !got_any {
                return Ok(LineRead::Eof);
            }
            break; // letzte Zeile ohne Zeilenende
        }
        got_any = true;
        let newline = available.iter().position(|b| *b == b'\n');
        let chunk_end = newline.unwrap_or(available.len());
        if !too_long {
            if buf.len() + chunk_end > max {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&available[..chunk_end]);
            }
        }
        let consumed = newline.map_or(available.len(), |i| i + 1);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if too_long {
        return Ok(LineRead::TooLong);
    }
    Ok(match String::from_utf8(buf) {
        Ok(line) => LineRead::Line(line),
        Err(_) => LineRead::InvalidUtf8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn the_requested_version_wins_when_known_else_the_latest() {
        assert_eq!(negotiate_version("2025-06-18"), "2025-06-18");
        assert_eq!(negotiate_version("2025-11-25"), "2025-11-25");
        // Ein Handshake kann keine zustandslose Version waehlen.
        assert_eq!(negotiate_version("2026-07-28"), LATEST_PROTOCOL);
        assert_eq!(negotiate_version("2024-11-05"), LATEST_PROTOCOL);
        assert_eq!(negotiate_version("gibt-es-nicht"), LATEST_PROTOCOL);
        assert_eq!(negotiate_version(""), LATEST_PROTOCOL);
    }

    #[test]
    fn the_versions_list_matches_its_parts() {
        let mut all: Vec<&str> = LEGACY_VERSIONS.to_vec();
        all.push(MODERN_PROTOCOL);
        assert_eq!(all, PROTOCOL_VERSIONS);
        assert!(LEGACY_VERSIONS.contains(&LATEST_PROTOCOL));
    }

    fn with_meta(version: Option<Value>, caps: Option<Value>) -> Value {
        let mut meta = serde_json::Map::new();
        if let Some(v) = version {
            meta.insert(META_PROTOCOL_VERSION.into(), v);
        }
        if let Some(c) = caps {
            meta.insert(META_CLIENT_CAPABILITIES.into(), c);
        }
        json!({ "_meta": meta, "name": "x" })
    }

    #[test]
    fn a_request_is_modern_only_with_the_new_version_and_capabilities() {
        assert_eq!(classify(&Value::Null), Ok(Era::Legacy));
        assert_eq!(classify(&json!({})), Ok(Era::Legacy));
        assert_eq!(classify(&json!({"_meta": {}})), Ok(Era::Legacy), "kein Handshake noetig, wie bisher");
        assert_eq!(classify(&json!({"_meta": {"progressToken": 1}})), Ok(Era::Legacy));
        for old in ["2025-06-18", "2025-11-25"] {
            assert_eq!(classify(&with_meta(Some(json!(old)), None)), Ok(Era::Legacy), "{old}");
        }
        assert_eq!(
            classify(&with_meta(Some(json!("2026-07-28")), Some(json!({})))),
            Ok(Era::Modern)
        );
        assert_eq!(
            classify(&with_meta(Some(json!("2026-07-28")), Some(json!({"roots": {}})))),
            Ok(Era::Modern)
        );
    }

    #[test]
    fn a_modern_request_without_capabilities_is_invalid_params() {
        for caps in [None, Some(json!(null)), Some(json!("x")), Some(json!([]))] {
            let e = classify(&with_meta(Some(json!("2026-07-28")), caps)).unwrap_err();
            assert_eq!(e.code, INVALID_PARAMS);
            assert!(e.message.contains("clientCapabilities"), "{}", e.message);
            assert!(e.data.is_none());
        }
        let e = classify(&with_meta(Some(json!(20260728)), Some(json!({})))).unwrap_err();
        assert_eq!(e.code, INVALID_PARAMS);
    }

    #[test]
    fn an_unknown_version_is_refused_with_the_supported_list() {
        for v in ["1900-01-01", "2026-07-29", "gibt-es-nicht", "2024-11-05"] {
            let e = classify(&with_meta(Some(json!(v)), Some(json!({})))).unwrap_err();
            assert_eq!(e.code, UNSUPPORTED_PROTOCOL_VERSION, "{v}");
            let data = e.data.as_ref().unwrap();
            assert_eq!(data["requested"], v);
            assert_eq!(data["supported"], json!(["2026-07-28", "2025-11-25", "2025-06-18"]));
            let line: Value = serde_json::from_str(&e.line(&json!(3))).unwrap();
            assert_eq!(line["error"]["code"], -32022);
            assert_eq!(line["error"]["data"]["supported"][0], "2026-07-28");
            assert_eq!(line["id"], 3);
        }
        // Eine riesige „Version“ wird nicht zurueckgespiegelt.
        let big = "x".repeat(10_000);
        let e = classify(&with_meta(Some(json!(big)), Some(json!({})))).unwrap_err();
        assert!(e.data.unwrap()["requested"].as_str().unwrap().len() <= 40);
    }

    #[test]
    fn parse_classifies_every_kind_of_line() {
        assert_eq!(parse_message("   "), Incoming::Ignore);
        assert_eq!(
            parse_message(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#),
            Incoming::Request {
                id: json!(7),
                method: "ping".into(),
                params: Value::Null
            }
        );
        // Zeichenketten-ID und BOM
        assert!(matches!(
            parse_message("\u{feff}{\"jsonrpc\":\"2.0\",\"id\":\"a-1\",\"method\":\"ping\"}"),
            Incoming::Request { id, .. } if id == json!("a-1")
        ));
        assert_eq!(
            parse_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Incoming::Notification {
                method: "notifications/initialized".into()
            }
        );
        // Antwort des Clients: ueberhoeren
        assert_eq!(
            parse_message(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#),
            Incoming::Ignore
        );
    }

    #[test]
    fn broken_lines_are_reported_with_the_right_code() {
        let code = |line: &str| match parse_message(line) {
            Incoming::Invalid { code, .. } => code,
            other => panic!("erwartet Invalid, war {other:?}"),
        };
        assert_eq!(code("{kaputt"), PARSE_ERROR);
        assert_eq!(code(r#"{"jsonrpc":"2.0","id":1,"meth"#), PARSE_ERROR);
        assert_eq!(
            code(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#),
            INVALID_REQUEST
        );
        assert_eq!(code("42"), INVALID_REQUEST);
        assert_eq!(
            code(r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#),
            INVALID_REQUEST
        );
        assert_eq!(code(r#"{"jsonrpc":"2.0","id":1}"#), INVALID_REQUEST);
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#),
            INVALID_REQUEST
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#),
            INVALID_REQUEST
        );
    }

    #[test]
    fn an_invalid_request_still_echoes_a_usable_id() {
        match parse_message(r#"{"jsonrpc":"1.0","id":9,"method":"ping"}"#) {
            Incoming::Invalid { id, .. } => assert_eq!(id, json!(9)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn responses_are_single_lines_even_with_line_breaks_in_the_text() {
        let line = result_line(&json!(1), json!({ "text": "a\nb\r\nc" }));
        assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["result"]["text"], "a\nb\r\nc");
        let line = error_line(&Value::Null, PARSE_ERROR, "Zeile 1\nZeile 2");
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["id"],
            Value::Null
        );
    }

    #[test]
    fn the_capped_reader_reads_lines_and_survives_an_overlong_one() {
        let mut input = Vec::new();
        input.extend_from_slice(b"erste\r\n");
        input.extend_from_slice(&vec![b'x'; 5_000]);
        input.extend_from_slice(b"\nzweite\n");
        input.extend_from_slice(&[0xff, 0xfe, b'\n']);
        input.extend_from_slice(b"ohne Ende");
        // Kleiner Puffer: die lange Zeile geht ueber viele fill_buf-Runden.
        let mut reader = io::BufReader::with_capacity(16, Cursor::new(input));
        let mut next = || read_line_capped(&mut reader, 100).unwrap();
        assert_eq!(next(), LineRead::Line("erste\r".into()));
        assert_eq!(next(), LineRead::TooLong);
        assert_eq!(next(), LineRead::Line("zweite".into()));
        assert_eq!(next(), LineRead::InvalidUtf8);
        assert_eq!(next(), LineRead::Line("ohne Ende".into()));
        assert_eq!(next(), LineRead::Eof);
    }
}
