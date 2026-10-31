//! Die lesenden Besprechungs-Werkzeuge der Agentenbruecke (Sprachgespraech Stufe 2): Besprechungen
//! auflisten, durchsuchen und das Transkript lesen. Rechte, Freigaben und Audit erledigt das Tor
//! VOR dem Aufruf (Recht `meeting.read`); die Handler hier pruefen ihre Argumente selbst und lesen
//! ueber dieselbe Logik wie der lokale MCP-Server (`mcp::tools`), also mit denselben Grenzen.
//!
//! | Werkzeug | Recht | Wirkung |
//! |---|---|---|
//! | `meetings_list` | `meeting.read` | fertige Besprechungen, neueste zuerst (Zeitraum, höchstens 50) |
//! | `meetings_search` | `meeting.read` | Volltextsuche; Treffer aus dem Transkript nur mit Transkript-Freigabe |
//! | `meeting_transcript` | `meeting.read` | Transkript seitenweise; nur mit Transkript-Freigabe |
//!
//! # Warum eigene Namen
//! Der lokale MCP-Server kennt `list_meetings`, `search_meetings` und `get_transcript` selbst. Die
//! Bruecke bekommt andere Namen (`meetings_*`), damit der MCP-Proxy nie ein eigenes Werkzeug
//! ueber die Bruecke umleitet (`mcp::link::is_bridge_tool`).
//!
//! # Fehlerfaelle und ihre Absicherung (Tests in `meetings/tests.rs`)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Unvertraute Argumente (Personenfilter, unbekanntes Feld, falscher Typ, Pfad statt Kennung) | strenge Pruefung, nichts gelesen; der Personenfilter fehlt absichtlich (er durchsucht das Transkript) | `arguments_are_checked_strictly` |
//! | 2 | Transkript nicht freigegeben | `meeting_transcript` verweigert mit Klartext, die Suche zeigt keine Transkript-Treffer | `the_transcript_needs_the_users_release` |
//! | 3 | Datenbank gesperrt, Platte voll, noch keine Datenbank | die Logik des MCP-Servers: kurze Wartezeit (2 s), Klartext ohne Inhalte, leere Liste ohne Datenbank | `mcp::tools` |
//! | 4 | Riesige Antworten | Seiten von 40 000 Zeichen, Listen 50, Suche 20; die Bruecke ersetzt Antworten ueber 1 MiB | `mcp::tools`, `bridge::run_handler` |
//! | 5 | Laufende Aufnahme, halber Import | nur fertige (`ready`) und lebende Besprechungen sind sichtbar | `mcp::tools` |
//!
//! Schreiben: kein Werkzeug hier schreibt (der Store wird nur lesend geoeffnet). Audio-Echtzeitpfad
//! und Kindprozesse: nicht beruehrt.

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::{spec, string_prop, AppTools, Args};
use crate::agent_bridge::catalog::ToolSpec;
use crate::mcp::tools::{self as mcp_tools, Backend};

/// Wartezeit auf eine gesperrte Datenbank (wie der lokale MCP-Server; die Stimme wartet nicht 30 s).
pub const READ_BUSY_TIMEOUT: Duration = Duration::from_secs(2);
/// Laengste Datumsangabe (`YYYY-MM-DD` oder RFC 3339).
const MAX_DATE_CHARS: usize = 40;
/// Laengste Suchanfrage.
const MAX_QUERY_CHARS: usize = 500;
/// Obergrenzen der Listen (wie `mcp::tools`).
const LIST_LIMIT_MAX: i64 = 50;
const SEARCH_LIMIT_MAX: i64 = 20;
/// Hoechster Zeichenversatz einer Transkriptseite.
const MAX_CURSOR: i64 = 100_000_000;

pub fn specs() -> Vec<ToolSpec> {
    let date = |what: &str| {
        string_prop(
            &format!("{what} als Datum (YYYY-MM-DD, Ortszeit) oder RFC 3339."),
            MAX_DATE_CHARS,
        )
    };
    vec![
        spec(
            "meetings_list",
            "Listet fertige Besprechungen aus Local Voice AI, neueste zuerst, mit Kennung, Titel, Datum und Dauer. \
             Optional nach Zeitraum eingrenzen. Die Kennung braucht meeting_transcript. Nur lesend.",
            json!({
                "type": "object",
                "properties": {
                    "from": date("Frühester Beginn"),
                    "to": date("Spätester Beginn"),
                    "limit": { "type": "integer", "minimum": 1, "maximum": LIST_LIMIT_MAX,
                               "description": "Höchstens so viele Besprechungen (Standard 20, höchstens 50)." }
                },
                "additionalProperties": false
            }),
        ),
        spec(
            "meetings_search",
            "Volltextsuche über Titel, Notizen und Transkript aller fertigen Besprechungen. Liefert je Besprechung \
             den besten Treffer als Auszug. Treffer aus dem Transkript gibt es nur, wenn der Nutzer das Transkript \
             freigegeben hat. Nur lesend.",
            json!({
                "type": "object",
                "properties": {
                    "query": string_prop("Suchwörter, z. B. „Budget Freigabe“.", MAX_QUERY_CHARS),
                    "from": date("Frühester Beginn"),
                    "to": date("Spätester Beginn"),
                    "limit": { "type": "integer", "minimum": 1, "maximum": SEARCH_LIMIT_MAX,
                               "description": "Höchstens so viele Besprechungen (Standard 10, höchstens 20)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        spec(
            "meeting_transcript",
            "Liest das Transkript einer fertigen Besprechung seitenweise (Zeitmarke, Sprecher, Text). Gibt es eine \
             weitere Seite, steht next_cursor im Ergebnis: diesen Wert als cursor erneut übergeben. Nur verfügbar, \
             wenn der Nutzer das Transkript freigegeben hat. Nur lesend.",
            json!({
                "type": "object",
                "properties": {
                    "meeting_id": string_prop("Kennung aus meetings_list oder meetings_search.", 64),
                    "cursor": { "type": "integer", "minimum": 0, "maximum": MAX_CURSOR,
                                "description": "next_cursor der vorigen Seite; ohne Angabe: Anfang." }
                },
                "required": ["meeting_id"],
                "additionalProperties": false
            }),
        ),
    ]
}

/// Uebernimmt `key` als Text in die weitergereichten Argumente.
fn forward_text(
    a: &mut Args,
    forwarded: &mut Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<(), String> {
    if let Some(v) = a.text(key, max, false)? {
        forwarded.insert(key.to_string(), Value::String(v));
    }
    Ok(())
}

/// Uebernimmt `key` als ganze Zahl in die weitergereichten Argumente.
fn forward_integer(
    a: &mut Args,
    forwarded: &mut Map<String, Value>,
    key: &str,
    min: i64,
    max: i64,
) -> Result<(), String> {
    if let Some(v) = a.integer(key, min, max)? {
        forwarded.insert(key.to_string(), json!(v));
    }
    Ok(())
}

impl AppTools {
    /// Liest ueber die Logik des lokalen MCP-Servers; ein Fehlertext wird zum Werkzeugfehler.
    fn read_meetings(&self, mcp_tool: &str, args: Map<String, Value>) -> Result<Value, String> {
        let backend = Backend {
            db_path: self.store.db_path(),
            busy_timeout: READ_BUSY_TIMEOUT,
            include_transcript: self.host.transcript_shared(),
        };
        let outcome = mcp_tools::call(mcp_tool, &Value::Object(args), &backend);
        if outcome.is_error {
            return Err(outcome.text);
        }
        serde_json::from_str(&outcome.text)
            .map_err(|_| "Die Antwort der Besprechungsdatenbank war nicht lesbar.".to_string())
    }

    pub(super) fn meetings_list(&self, mut a: Args) -> Result<Value, String> {
        let mut forwarded = Map::new();
        forward_text(&mut a, &mut forwarded, "from", MAX_DATE_CHARS)?;
        forward_text(&mut a, &mut forwarded, "to", MAX_DATE_CHARS)?;
        forward_integer(&mut a, &mut forwarded, "limit", 1, LIST_LIMIT_MAX)?;
        a.done()?;
        self.read_meetings("list_meetings", forwarded)
    }

    pub(super) fn meetings_search(&self, mut a: Args) -> Result<Value, String> {
        let mut forwarded = Map::new();
        let query = a.required("query", MAX_QUERY_CHARS, false)?;
        forwarded.insert("query".to_string(), Value::String(query));
        forward_text(&mut a, &mut forwarded, "from", MAX_DATE_CHARS)?;
        forward_text(&mut a, &mut forwarded, "to", MAX_DATE_CHARS)?;
        forward_integer(&mut a, &mut forwarded, "limit", 1, SEARCH_LIMIT_MAX)?;
        a.done()?;
        self.read_meetings("search_meetings", forwarded)
    }

    pub(super) fn meeting_transcript(&self, mut a: Args) -> Result<Value, String> {
        let id = a
            .id("meeting_id")?
            .ok_or_else(|| "„meeting_id“ fehlt.".to_string())?;
        let mut forwarded = Map::new();
        forwarded.insert("id".to_string(), Value::String(id));
        forward_integer(&mut a, &mut forwarded, "cursor", 0, MAX_CURSOR)?;
        a.done()?;
        self.read_meetings("get_transcript", forwarded)
    }
}

#[cfg(test)]
mod tests;
