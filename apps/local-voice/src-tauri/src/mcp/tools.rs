//! Die vier lesenden Werkzeuge des lokalen MCP-Servers: `list_meetings`,
//! `search_meetings`, `get_meeting`, `get_transcript`.
//!
//! Sicherheitsgrenzen (Entwurf `m5-m6-kalender-export.md` F21, E13):
//! - Nur lesen: der Store ist `SQLITE_OPEN_READ_ONLY` geoeffnet
//!   (`MeetingStore::open_read_only`); kein Werkzeug ruft eine Schreibfunktion.
//! - Sichtbar sind nur lebende (`deleted_at IS NULL`) UND fertige (`ready`)
//!   Besprechungen. Eine laufende Aufnahme oder ein halber Import ist kein
//!   Inhalt, den ein KI-Client sehen soll.
//! - Nie im Ergebnis: Audiopfade, Quelldateipfade, Rohdatenbank, Einstellungen.
//! - Ist das Transkript nicht freigegeben, gibt es KEINEN Weg dorthin: kein
//!   `get_transcript`, keine Treffer, deren bester Chunk aus dem Transkript
//!   stammt (`search_meetings`), kein `person`-Filter (der wuerde ueber den
//!   Volltext im Transkript suchen und verriete Inhalte durch Ja/Nein).
//! - Jede Antwort ist begrenzt (Listen 50, Suche 20, Transkriptseite 40 000
//!   Zeichen, Besprechungstext 120 000 Zeichen).
//! - Ein Fehler beschreibt die Ursache und den naechsten Schritt, enthaelt aber
//!   nie Inhalte (keine Texte aus Transkript oder Notizen).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, SecondsFormat, TimeZone};
use serde_json::{json, Value};

use crate::managers::meetings::export::{
    build_bundle, bundle_to_markdown, ExportParticipant, ExportParts,
};
use crate::managers::meetings::llm_call::duration_label;
use crate::managers::meetings::notes::model::{ActionItem, STATUS_DONE};
use crate::managers::meetings::search::chunking::{query_terms, ChunkSource};
use crate::managers::meetings::search::index::{MeetingFilter, ScopeFilter};
use crate::managers::meetings::speakers::SpeakerDirectory;
use crate::managers::meetings::store::{Meeting, MeetingStore, ReadOnlyOpenError};
use crate::managers::meetings::subtitle::speaker_label;

/// Zeichen je Seite von `get_transcript`.
pub const TRANSCRIPT_PAGE_CHARS: usize = 40_000;
/// Obergrenze fuer den Text von `get_meeting`.
pub const MEETING_TEXT_MAX_CHARS: usize = 120_000;
pub const LIST_LIMIT_DEFAULT: u32 = 20;
pub const LIST_LIMIT_MAX: u32 = 50;
pub const SEARCH_LIMIT_DEFAULT: u32 = 10;
pub const SEARCH_LIMIT_MAX: u32 = 20;
/// So viele Treffer holt die Suche, bevor sie nach Status/Freigabe filtert.
const SEARCH_FETCH: u32 = 100;
const MAX_QUERY_CHARS: usize = 500;
const MAX_ID_CHARS: usize = 64;

/// Ergebnis eines Werkzeugaufrufs: ein Text und ob er ein Fehler ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
}

impl ToolOutcome {
    fn ok(text: String) -> Self {
        Self {
            text,
            is_error: false,
        }
    }

    fn json(value: Value) -> Self {
        Self::ok(value.to_string())
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

/// Wo und wie gelesen wird.
pub struct Backend<'a> {
    pub db_path: &'a Path,
    pub busy_timeout: Duration,
    /// Einstellung `meeting_mcp_include_transcript`.
    pub include_transcript: bool,
}

fn read_only_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

/// Inhalt von `tools/list`.
pub fn definitions() -> Vec<Value> {
    let time = |what: &str| {
        json!({
            "type": "string",
            "description": format!(
                "{what} als Datum (YYYY-MM-DD, Ortszeit) oder RFC 3339, z. B. 2026-09-01 oder 2026-09-01T09:00:00+02:00."
            )
        })
    };
    vec![
        json!({
            "name": "list_meetings",
            "title": "Besprechungen auflisten",
            "description": "Listet fertige Besprechungen aus Local Voice AI, neueste zuerst, mit ID, Titel, Datum und Dauer. \
                Optional nach Zeitraum, Person (Sprechername oder Name in Titel, Notizen und Transkript; nur mit \
                Transkript-Freigabe) oder Ordner eingrenzen. Die ID braucht get_meeting und get_transcript.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": time("Frühester Beginn"),
                    "to": time("Spätester Beginn"),
                    "person": { "type": "string", "description": "Name einer Person, z. B. \"Anna Berg\"." },
                    "folder": { "type": "string", "description": "Name oder ID eines Ordners." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": LIST_LIMIT_MAX, "default": LIST_LIMIT_DEFAULT,
                               "description": "Höchstens so viele Besprechungen (Standard 20, höchstens 50)." }
                },
                "additionalProperties": false
            },
            "annotations": read_only_annotations()
        }),
        json!({
            "name": "search_meetings",
            "title": "Besprechungen durchsuchen",
            "description": "Volltextsuche über Titel, Notizen, KI-Notizen und Transkript aller fertigen Besprechungen. \
                Liefert je Besprechung den besten Treffer als Auszug (Fundstelle in **fett**). Alle Suchwörter müssen \
                im selben Abschnitt vorkommen; Teilwörter und Zusammensetzungen werden gefunden.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS,
                               "description": "Suchwörter, z. B. \"Budget Freigabe\"." },
                    "from": time("Frühester Beginn"),
                    "to": time("Spätester Beginn"),
                    "limit": { "type": "integer", "minimum": 1, "maximum": SEARCH_LIMIT_MAX, "default": SEARCH_LIMIT_DEFAULT,
                               "description": "Höchstens so viele Besprechungen (Standard 10, höchstens 20)." }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": read_only_annotations()
        }),
        json!({
            "name": "get_meeting",
            "title": "Besprechung lesen",
            "description": "Liefert Notizen einer fertigen Besprechung als Text: KI-Notizen, eigene Notizen, Protokoll, \
                Teilnehmende und Aufgaben. Ohne parts kommt alles. Das Transkript gibt es nur über get_transcript.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "ID aus list_meetings oder search_meetings." },
                    "parts": {
                        "type": "array",
                        "items": { "type": "string", "enum": PARTS },
                        "uniqueItems": true,
                        "description": "Welche Teile; Standard: alle."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": read_only_annotations()
        }),
        json!({
            "name": "get_transcript",
            "title": "Transkript lesen",
            "description": "Liefert das Transkript einer fertigen Besprechung seitenweise (Zeitmarke, Sprecher, Text). \
                Gibt es eine weitere Seite, steht next_cursor im Ergebnis: diesen Wert als cursor erneut übergeben. \
                Nur verfügbar, wenn der Nutzer das Transkript freigegeben hat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "ID aus list_meetings oder search_meetings." },
                    "cursor": { "type": "integer", "minimum": 0, "description": "next_cursor der vorigen Seite; ohne Angabe: Anfang." }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": read_only_annotations()
        }),
    ]
}

const PARTS: [&str; 5] = [
    "ai_notes",
    "notes",
    "minutes",
    "participants",
    "action_items",
];

/// Ist `name` eines unserer Werkzeuge?
pub fn is_known(name: &str) -> bool {
    matches!(
        name,
        "list_meetings" | "search_meetings" | "get_meeting" | "get_transcript"
    )
}

/// Fuehrt ein Werkzeug aus. `name` ist vorher mit [`is_known`] geprueft.
pub fn call(name: &str, args: &Value, backend: &Backend) -> ToolOutcome {
    let outcome = match name {
        "list_meetings" => list_meetings(args, backend),
        "search_meetings" => search_meetings(args, backend),
        "get_meeting" => get_meeting(args, backend),
        "get_transcript" => get_transcript(args, backend),
        other => Err(ToolOutcome::error(format!("Unbekanntes Werkzeug: {other}"))),
    };
    outcome.unwrap_or_else(|e| e)
}

// ---------------------------------------------------------------------------
// Argumente
// ---------------------------------------------------------------------------

type Res<T> = Result<T, ToolOutcome>;

fn arg_error(text: impl Into<String>) -> ToolOutcome {
    ToolOutcome::error(text)
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Res<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = s.trim();
            Ok((!s.is_empty()).then_some(s))
        }
        Some(_) => Err(arg_error(format!("„{key}“ muss ein Text sein."))),
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Res<&'a str> {
    opt_str(args, key)?.ok_or_else(|| arg_error(format!("„{key}“ fehlt.")))
}

fn req_id(args: &Value) -> Res<&str> {
    let id = req_str(args, "id")?;
    if id.chars().count() > MAX_ID_CHARS || id.chars().any(char::is_control) {
        return Err(arg_error("„id“ ist keine Besprechungs-ID."));
    }
    Ok(id)
}

fn limit_arg(args: &Value, default: u32, max: u32) -> Res<u32> {
    let raw = match args.get("limit") {
        None | Some(Value::Null) => return Ok(default),
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        Some(_) => None,
    };
    match raw {
        Some(n) if n >= 1 => Ok((n as u64).min(u64::from(max)) as u32),
        _ => Err(arg_error(format!(
            "„limit“ muss eine ganze Zahl von 1 bis {max} sein."
        ))),
    }
}

/// `YYYY-MM-DD` (Ortszeit; bei `end_of_day` bis 23:59:59) oder RFC 3339, als
/// Unix-Sekunden.
fn time_arg(args: &Value, key: &str, end_of_day: bool) -> Res<Option<i64>> {
    let Some(text) = opt_str(args, key)? else {
        return Ok(None);
    };
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let naive = if end_of_day {
            date.and_hms_opt(23, 59, 59)
        } else {
            date.and_hms_opt(0, 0, 0)
        };
        let local = naive.and_then(|n| {
            Local
                .from_local_datetime(&n)
                .earliest()
                .or_else(|| Local.from_local_datetime(&n).latest())
        });
        if let Some(local) = local {
            return Ok(Some(local.timestamp()));
        }
    } else if let Ok(stamp) = DateTime::parse_from_rfc3339(text) {
        return Ok(Some(stamp.timestamp()));
    }
    Err(arg_error(format!(
        "„{key}“ ist kein Datum. Erwartet: YYYY-MM-DD oder RFC 3339 (z. B. 2026-09-01T09:00:00+02:00)."
    )))
}

// ---------------------------------------------------------------------------
// Datenbank
// ---------------------------------------------------------------------------

fn looks_busy(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("locked") || m.contains("busy")
}

const BUSY_HINT: &str = "Die Besprechungsdatenbank ist gerade gesperrt (Local Voice AI schreibt). \
    Bitte in ein paar Sekunden erneut versuchen.";

/// Fehler aus dem Store als Text ohne Inhalte (SQLite-Meldungen nennen nie Nutzertext).
fn db_error(message: &str) -> ToolOutcome {
    if looks_busy(message) {
        return ToolOutcome::error(BUSY_HINT);
    }
    ToolOutcome::error(format!(
        "Die Besprechungsdatenbank konnte nicht gelesen werden ({message})."
    ))
}

fn store_error(e: &anyhow::Error) -> ToolOutcome {
    db_error(&e.to_string())
}

/// `Ok(None)`: es gibt noch keine Datenbank (leere Liste statt Fehler).
fn open(backend: &Backend) -> Res<Option<MeetingStore>> {
    match MeetingStore::open_read_only(backend.db_path, backend.busy_timeout) {
        Ok(store) => Ok(Some(store)),
        Err(ReadOnlyOpenError::Missing) => Ok(None),
        Err(ReadOnlyOpenError::SchemaNewer { .. }) => Err(ToolOutcome::error(
            "Local Voice AI wurde aktualisiert und hat die Datenbank umgestellt. \
             Bitte den MCP-Server neu starten (im KI-Client die Verbindung „local-voice“ erneuern).",
        )),
        Err(ReadOnlyOpenError::SchemaOlder { .. }) => Err(ToolOutcome::error(
            "Die Besprechungsdatenbank stammt von einer älteren Version. Bitte Local Voice AI \
             einmal starten, damit sie aktualisiert wird, und dann erneut fragen.",
        )),
        Err(ReadOnlyOpenError::Unreadable(message)) => Err(db_error(&message)),
    }
}

const NO_DATABASE_NOTE: &str =
    "Es gibt noch keine Besprechungsdatenbank. Sie entsteht mit der ersten Besprechung in Local Voice AI.";

fn not_found(id: &str) -> ToolOutcome {
    ToolOutcome::error(format!(
        "Besprechung nicht gefunden (ID: {id}). IDs liefern list_meetings und search_meetings."
    ))
}

/// Die lebende, FERTIGE Besprechung oder ein Fehlertext.
fn ready_meeting(store: &MeetingStore, id: &str) -> Res<Meeting> {
    match store.get_meeting(id) {
        Ok(Some(m)) if m.status == "ready" => Ok(m),
        Ok(Some(m)) => Err(ToolOutcome::error(format!(
            "Die Besprechung ist noch nicht fertig (Status: {}). Bitte später erneut fragen.",
            m.status
        ))),
        Ok(None) => Err(not_found(id)),
        Err(e) => Err(store_error(&e)),
    }
}

fn iso(ts: i64) -> Option<String> {
    DateTime::from_timestamp(ts, 0).map(|d| {
        d.with_timezone(&Local)
            .to_rfc3339_opts(SecondsFormat::Secs, false)
    })
}

fn meeting_json(m: &Meeting) -> Value {
    json!({
        "id": m.id,
        "title": m.title,
        "date": iso(m.started_at.unwrap_or(m.created_at)),
        "duration_seconds": m.duration_ms.map(|ms| ms / 1_000),
        "source": m.source,
        "language": m.language,
        "status": m.status,
    })
}

fn resolve_folder(store: &MeetingStore, wanted: &str) -> Res<String> {
    let folders = store.folders_list().map_err(|e| store_error(&e))?;
    let needle = wanted.trim().to_lowercase();
    if let Some(folder) = folders
        .iter()
        .find(|f| f.id == wanted.trim() || f.name.trim().to_lowercase() == needle)
    {
        return Ok(folder.id.clone());
    }
    let names: Vec<String> = folders.iter().map(|f| format!("„{}“", f.name)).collect();
    Err(ToolOutcome::error(if names.is_empty() {
        format!("Ordner nicht gefunden: „{wanted}“. Es gibt noch keine Ordner.")
    } else {
        format!(
            "Ordner nicht gefunden: „{wanted}“. Vorhandene Ordner: {}.",
            names.join(", ")
        )
    }))
}

// ---------------------------------------------------------------------------
// list_meetings
// ---------------------------------------------------------------------------

fn list_meetings(args: &Value, backend: &Backend) -> Res<ToolOutcome> {
    let from = time_arg(args, "from", false)?;
    let to = time_arg(args, "to", true)?;
    let person = opt_str(args, "person")?;
    let folder = opt_str(args, "folder")?;
    let limit = limit_arg(args, LIST_LIMIT_DEFAULT, LIST_LIMIT_MAX)?;
    if person.is_some() && !backend.include_transcript {
        return Err(ToolOutcome::error(
            "Der Personenfilter durchsucht auch das Transkript und ist deshalb nur mit \
             Transkript-Freigabe verfügbar (Einstellungen > Besprechungen > Lokaler MCP-Server). \
             Ohne „person“ funktioniert die Liste.",
        ));
    }
    let Some(store) = open(backend)? else {
        return Ok(ToolOutcome::json(json!({
            "meetings": [], "returned": 0, "total": 0, "note": NO_DATABASE_NOTE
        })));
    };
    let folder_id = match folder {
        Some(name) => Some(resolve_folder(&store, name)?),
        None => None,
    };
    let ids = store
        .resolve_scope(&ScopeFilter {
            meeting_ids: None,
            folder_id,
            person: person.map(str::to_string),
            from,
            to,
            person_id: None,
            event_uid: None,
        })
        .map_err(|e| store_error(&e))?;
    let total = ids.len();
    let mut meetings = Vec::new();
    for id in ids.iter().take(limit as usize) {
        if let Some(meeting) = store.get_meeting(id).map_err(|e| store_error(&e))? {
            meetings.push(meeting_json(&meeting));
        }
    }
    Ok(ToolOutcome::json(json!({
        "meetings": meetings, "returned": meetings.len(), "total": total
    })))
}

// ---------------------------------------------------------------------------
// search_meetings
// ---------------------------------------------------------------------------

/// Der Auszug der Suche (HTML mit `<mark>`) als Klartext mit **Fundstelle**.
fn plain_snippet(html: &str) -> String {
    html.replace("<mark>", "**")
        .replace("</mark>", "**")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn search_meetings(args: &Value, backend: &Backend) -> Res<ToolOutcome> {
    let query = req_str(args, "query")?;
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(arg_error(format!(
            "„query“ ist zu lang (höchstens {MAX_QUERY_CHARS} Zeichen)."
        )));
    }
    if query_terms(query).is_empty() {
        return Err(arg_error(
            "„query“ enthält keine Suchwörter (nur Buchstaben und Ziffern zählen).",
        ));
    }
    let from = time_arg(args, "from", false)?;
    let to = time_arg(args, "to", true)?;
    let limit = limit_arg(args, SEARCH_LIMIT_DEFAULT, SEARCH_LIMIT_MAX)? as usize;
    let Some(store) = open(backend)? else {
        return Ok(ToolOutcome::json(json!({
            "meetings": [], "returned": 0, "note": NO_DATABASE_NOTE
        })));
    };
    let page = store
        .search_meetings(
            query,
            &MeetingFilter {
                from,
                to,
                ..MeetingFilter::default()
            },
            0,
            SEARCH_FETCH,
        )
        .map_err(|e| store_error(&e))?;
    let visible: Vec<_> = page
        .items
        .iter()
        .filter(|item| item.meeting.status == "ready")
        // Ohne Transkript-Freigabe kein Treffer, dessen bester Chunk das Transkript ist.
        .filter(|item| {
            backend.include_transcript || item.hit_source != Some(ChunkSource::Transcript)
        })
        .collect();
    let more = visible.len() > limit;
    let meetings: Vec<Value> = visible
        .iter()
        .take(limit)
        .map(|item| {
            let mut value = meeting_json(&item.meeting);
            value["snippet"] = json!(item.snippet.as_deref().map(plain_snippet));
            value["hit_source"] = json!(item.hit_source.map(ChunkSource::as_str));
            value
        })
        .collect();
    let mut notes: Vec<String> = Vec::new();
    if !backend.include_transcript {
        notes.push(
            "Transkript-Freigabe aus: Treffer im Transkript werden nicht angezeigt.".to_string(),
        );
    }
    if page.truncated {
        notes.push("Sehr viele Treffer; die Liste ist gekürzt. Bitte genauer suchen.".to_string());
    }
    if let Ok(counts) = store.index_counts() {
        if counts.pending > 0 {
            notes.push(format!(
                "{} Besprechung(en) sind noch nicht durchsuchbar (Local Voice AI baut den Index im Hintergrund auf).",
                counts.pending
            ));
        }
    }
    let mut out = json!({
        "query": query, "meetings": meetings, "returned": meetings.len(), "more_available": more
    });
    if !notes.is_empty() {
        out["note"] = json!(notes.join(" "));
    }
    Ok(ToolOutcome::json(out))
}

// ---------------------------------------------------------------------------
// get_meeting
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct PartSel {
    ai_notes: bool,
    notes: bool,
    minutes: bool,
    participants: bool,
    action_items: bool,
}

fn parts_arg(args: &Value) -> Res<PartSel> {
    let all = PartSel {
        ai_notes: true,
        notes: true,
        minutes: true,
        participants: true,
        action_items: true,
    };
    let list = match args.get("parts") {
        None | Some(Value::Null) => return Ok(all),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(arg_error("„parts“ muss eine Liste sein.")),
    };
    let mut sel = PartSel {
        ai_notes: false,
        notes: false,
        minutes: false,
        participants: false,
        action_items: false,
    };
    for part in list {
        match part.as_str() {
            Some("ai_notes") => sel.ai_notes = true,
            Some("notes") => sel.notes = true,
            Some("minutes") => sel.minutes = true,
            Some("participants") => sel.participants = true,
            Some("action_items") => sel.action_items = true,
            _ => {
                return Err(arg_error(format!(
                    "Unbekannter Teil in „parts“. Erlaubt: {}.",
                    PARTS.join(", ")
                )))
            }
        }
    }
    Ok(sel)
}

fn actions_markdown(items: &[ActionItem]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut md = String::from("\n## Aufgaben\n\n");
    for item in items {
        let mark = if item.status == STATUS_DONE { "x" } else { " " };
        let text = item.text.split_whitespace().collect::<Vec<_>>().join(" ");
        match item.assignee_label.as_deref().map(str::trim) {
            Some(who) if !who.is_empty() => {
                md.push_str(&format!("- [{mark}] {text} (Zuständig: {who})\n"))
            }
            _ => md.push_str(&format!("- [{mark}] {text}\n")),
        }
    }
    md
}

/// Kuerzt auf `max` Zeichen an einer Zeilengrenze und sagt, dass gekuerzt wurde.
fn cap_text(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    let end = cut.rfind('\n').unwrap_or(cut.len());
    format!("{}\n\n[Text gekürzt.]", &cut[..end])
}

fn get_meeting(args: &Value, backend: &Backend) -> Res<ToolOutcome> {
    let id = req_id(args)?;
    let parts = parts_arg(args)?;
    let Some(store) = open(backend)? else {
        return Err(not_found(id));
    };
    let meeting = ready_meeting(&store, id)?;
    let mut bundle = build_bundle(&store, id).map_err(|e| db_error(&e))?;
    bundle.participants = store
        .meeting_participant_rows(id)
        .map_err(|e| store_error(&e))?
        .into_iter()
        .map(|(name, email, role)| ExportParticipant {
            name: Some(name),
            email,
            role: Some(role),
        })
        .collect();
    let all_actions = if parts.action_items {
        let md = actions_markdown(&bundle.action_items);
        // Die KI-Notizen zeigen ihre Aufgaben selbst; die Liste unten ist die
        // Gesamtsicht, darum bleiben im Export-Text nur die Aufgaben mit Eintrag
        // (fuer die Haekchen) und keine zweite "Weitere Aufgaben"-Liste.
        bundle.action_items.retain(|a| a.entry_id.is_some());
        md
    } else {
        String::new()
    };
    let mut text = format!(
        "ID: {} · Quelle: {} · Sprache: {}\n\n",
        meeting.id,
        meeting.source,
        meeting.language.as_deref().unwrap_or("unbekannt")
    );
    let markdown = bundle_to_markdown(
        &bundle,
        &ExportParts {
            ai_notes: parts.ai_notes,
            notes: parts.notes,
            minutes: parts.minutes,
            transcript: false,
            participants: parts.participants,
        },
    );
    let has_content = markdown.contains("\n## ") || !all_actions.is_empty();
    text.push_str(&markdown);
    text.push_str(&all_actions);
    if !has_content {
        text.push_str("\n(Zu den gewählten Teilen gibt es für diese Besprechung keinen Inhalt.");
        if backend.include_transcript {
            text.push_str(" Das Transkript liefert get_transcript.");
        }
        text.push_str(")\n");
    }
    Ok(ToolOutcome::ok(cap_text(text, MEETING_TEXT_MAX_CHARS)))
}

// ---------------------------------------------------------------------------
// get_transcript
// ---------------------------------------------------------------------------

/// Eine Seite Text ab Zeichenposition `cursor`, hoechstens `max` Zeichen, nach
/// Moeglichkeit an einer Zeilengrenze. Liefert (Seitentext, naechster Cursor,
/// Gesamtzeichen). Ein `cursor` hinter dem Ende ist ein Fehler.
pub fn page_text(
    text: &str,
    cursor: usize,
    max: usize,
) -> Result<(String, Option<usize>, usize), String> {
    let total = text.chars().count();
    if cursor > total {
        return Err(format!(
            "„cursor“ liegt hinter dem Ende des Transkripts ({total} Zeichen)."
        ));
    }
    let start = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(byte, _)| byte);
    let rest = &text[start..];
    if total - cursor <= max {
        return Ok((rest.to_string(), None, total));
    }
    let window_end = rest
        .char_indices()
        .nth(max)
        .map_or(rest.len(), |(byte, _)| byte);
    let window = &rest[..window_end];
    // An der letzten Zeilengrenze im Fenster schneiden; ohne eine (eine einzige
    // sehr lange Zeile) mitten im Text, damit die Seite immer vorankommt.
    let end = window.rfind('\n').map_or(window.len(), |i| i + 1);
    let page = &rest[..end];
    Ok((page.to_string(), Some(cursor + page.chars().count()), total))
}

fn cursor_arg(args: &Value) -> Res<usize> {
    match args.get("cursor") {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(n)) => n.as_u64().map(|c| c as usize).ok_or_else(|| {
            arg_error("„cursor“ muss eine Zahl ab 0 sein (next_cursor der vorigen Seite).")
        }),
        Some(Value::String(s)) => s.trim().parse::<usize>().map_err(|_| {
            arg_error("„cursor“ muss eine Zahl ab 0 sein (next_cursor der vorigen Seite).")
        }),
        Some(_) => Err(arg_error(
            "„cursor“ muss eine Zahl ab 0 sein (next_cursor der vorigen Seite).",
        )),
    }
}

/// Das ganze Transkript als Zeilen `[mm:ss] Sprecher: Text`.
fn transcript_text(store: &MeetingStore, id: &str) -> Res<String> {
    let mut segments = store.get_segments(id).map_err(|e| store_error(&e))?;
    segments.sort_by_key(|s| (s.start_ms, s.segment_index));
    let directory = SpeakerDirectory::load(store, id);
    let mut out = String::new();
    for segment in &segments {
        let text = segment
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        let label = match segment.speaker_index {
            Some(_) => directory.label(segment),
            None => speaker_label(segment, &BTreeMap::new()),
        };
        let time = duration_label(segment.start_ms);
        if label.is_empty() {
            out.push_str(&format!("[{time}] {text}\n"));
        } else {
            out.push_str(&format!("[{time}] {label}: {text}\n"));
        }
    }
    Ok(out)
}

fn get_transcript(args: &Value, backend: &Backend) -> Res<ToolOutcome> {
    let id = req_id(args)?;
    let cursor = cursor_arg(args)?;
    if !backend.include_transcript {
        return Err(ToolOutcome::error(
            "Das Transkript ist für den MCP-Server nicht freigegeben. Der Nutzer kann es unter \
             Einstellungen > Besprechungen > Lokaler MCP-Server einschalten („Transkript freigeben“). \
             Notizen und Protokoll liefert get_meeting.",
        ));
    }
    let Some(store) = open(backend)? else {
        return Err(not_found(id));
    };
    let meeting = ready_meeting(&store, id)?;
    let text = transcript_text(&store, id)?;
    let (page, next, total) =
        page_text(&text, cursor, TRANSCRIPT_PAGE_CHARS).map_err(ToolOutcome::error)?;
    let mut out = json!({
        "meeting_id": meeting.id,
        "title": meeting.title,
        "cursor": cursor,
        "next_cursor": next,
        "total_chars": total,
        "text": page,
    });
    if let Some(next) = next {
        out["hint"] = json!(format!(
            "Nächste Seite: get_transcript mit id und cursor={next}."
        ));
    }
    if total == 0 {
        out["hint"] = json!("Für diese Besprechung gibt es kein Transkript.");
    }
    Ok(ToolOutcome::json(out))
}
