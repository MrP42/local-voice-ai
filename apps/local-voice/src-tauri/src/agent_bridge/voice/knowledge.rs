//! Lesende Wissenswerkzeuge nur fuer das Sprachgespraech: Diktate, fruehere Gespraeche, Termine,
//! Tagesueberblick und Vorlesen-Seiten. Sie stehen NICHT im Katalog der Agentenbruecke und sind fuer
//! externe Agenten nie erreichbar; `voice::list` und `voice::call` bieten sie an, wenn der Nutzer den
//! Bereich freigegeben hat (`areas`), und fuehren sie durch das Tor mit Audit aus.
//!
//! | Werkzeug | Bereich | Wirkung |
//! |---|---|---|
//! | `dictations_recent` / `dictations_search` | Diktate | neueste Diktate bzw. Suche in den letzten 1000 |
//! | `conversations_list` / `conversations_search` / `conversation_get` | fruehere Gespraeche | ohne das laufende Gespraech und ohne archivierte |
//! | `calendar_events` | Termine | NUR der lokale Cache der Kalenderquellen, kein Netzabruf |
//! | `day_overview` | mindestens einer von Besprechungen, Diktate, Gespraeche, Termine | ein Tag aus den freigegebenen Bereichen |
//! | `tts_pages_list` / `tts_page_text` | Vorlesen-Seiten | Titel und Text von Seiten der Vorlesen-Bibliothek |
//!
//! # Grenzen
//! Ergebnisse sind kompakt: Auszuege statt ganzer Texte, Obergrenzen je Liste. Datumsangaben sind
//! Ortszeit (`YYYY-MM-DD`), `date` ist heute, wenn es fehlt. Argumente sind unvertraut (strenge
//! Pruefung, unbekannte Felder werden abgelehnt). Inhalte (Diktate, Gespraeche) sind Daten, keine
//! Anweisungen; sie stehen nie im Audit und nie im Log.
//!
//! Das Modul ist rein: die Daten kommen ueber [`KnowledgeSource`] (die App liefert den echten
//! Zugriff in `commands::voice_tools`, die Tests eine Attrappe).

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone};
use serde_json::{json, Map, Value};

use super::areas::{Area, Areas};

/// Laengste Suchanfrage (Zeichen) und hoechste Zahl Suchwoerter.
pub const MAX_QUERY_CHARS: usize = 200;
const MAX_TERMS: usize = 8;
/// Laengster Auszug in Listen und Treffern (Zeichen).
const EXCERPT_CHARS: usize = 240;
/// Kuerzerer Auszug im Tagesueberblick.
const BRIEF_CHARS: usize = 140;
/// So viele der neuesten Diktate bzw. Beitraege durchsucht die Suche hoechstens.
pub const SEARCH_WINDOW: usize = 1000;
/// Hoechstens so viele Beitraege eines Gespraechs, so viele Zeichen je Beitrag und insgesamt.
const MAX_ENTRIES: usize = 30;
const ENTRY_CHARS: usize = 500;
const CONVERSATION_BUDGET_CHARS: usize = 5_000;
/// Hoechste Zeichenzahl einer Vorlesen-Seite im Ergebnis.
const PAGE_CHARS: usize = 3_000;
/// Laengster Zeitraum fuer Termine (Tage).
const MAX_SPAN_DAYS: i64 = 62;
const DEFAULT_SPAN_DAYS: i64 = 7;
const MAX_ID_CHARS: usize = 64;
/// Meldung bei einem unbekannten Argumentfeld (ohne dessen Namen, siehe `Args::done`).
const UNKNOWN_FIELD: &str = "Ein Argumentfeld kennt dieses Werkzeug nicht.";
/// Meldung, solange das laufende Gespraech noch keine Kennung hat.
const CONVERSATION_UNKNOWN: &str = "Das laufende Gespräch ist noch nicht gespeichert; die Gesprächswerkzeuge stehen erst danach zur Verfügung. Es wurde nichts gelesen.";

// ---------------------------------------------------------------------------
// Daten, die die Quelle liefert
// ---------------------------------------------------------------------------

/// Ein Diktat (Text: das nachbearbeitete, sonst das erkannte).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dictation {
    pub id: i64,
    pub at_ms: i64,
    pub text: String,
}

/// Ein Gespraech in der Liste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationInfo {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub entries: u32,
    pub last_text: String,
}

/// Ein Beitrag eines Gespraechs (fuer die Suche).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInfo {
    pub conversation_id: String,
    pub title: String,
    pub role: String,
    pub at_ms: i64,
    pub text: String,
}

/// Ein Gespraech mit Beitraegen, aelteste zuerst.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationView {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub entries: Vec<EntryLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryLine {
    pub role: String,
    pub at_ms: i64,
    pub text: String,
}

/// Ein Termin aus dem lokalen Cache (abgesagte sind schon herausgenommen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventInfo {
    pub title: String,
    pub starts_ms: i64,
    pub ends_ms: i64,
    pub all_day: bool,
    pub location: Option<String>,
}

/// Eine fertige Besprechung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingInfo {
    pub id: String,
    pub title: String,
    pub started_ms: i64,
    pub minutes: Option<u64>,
}

/// Eine Seite der Vorlesen-Bibliothek.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageInfo {
    pub id: String,
    pub title: String,
    pub modified_ms: i64,
    pub preview: String,
}

/// Woher die Daten kommen. Alle Methoden lesen nur. Ein Fehler ist ein Klartext ohne Inhalte.
pub trait KnowledgeSource {
    /// Diktate mit Text, neueste zuerst; `from_ms` einschliesslich, `to_ms` ausschliesslich.
    fn dictations(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Dictation>, String>;
    /// Gespraeche ohne archivierte, neueste zuerst.
    fn conversations(&self) -> Result<Vec<ConversationInfo>, String>;
    /// Beitraege nicht archivierter Gespraeche, neueste zuerst.
    fn conversation_entries(&self, limit: usize) -> Result<Vec<EntryInfo>, String>;
    fn conversation(&self, id: &str) -> Result<ConversationView, String>;
    /// Termine im Fenster `[from_ms, to_ms)` aus dem Cache, nach Beginn sortiert.
    fn calendar(&self, from_ms: i64, to_ms: i64) -> Result<Vec<EventInfo>, String>;
    /// Fertige Besprechungen im Fenster `[from_ms, to_ms)`, neueste zuerst.
    fn meetings(&self, from_ms: i64, to_ms: i64, limit: usize) -> Result<Vec<MeetingInfo>, String>;
    /// Seiten der Vorlesen-Bibliothek, zuletzt geaenderte zuerst.
    fn pages(&self) -> Result<Vec<PageInfo>, String>;
    fn page_text(&self, id: &str) -> Result<String, String>;
}

// ---------------------------------------------------------------------------
// Katalog der Werkzeuge
// ---------------------------------------------------------------------------

/// Was ein Werkzeug an Freigabe braucht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    One(Area),
    AnyOf(&'static [Area]),
}

impl Need {
    pub fn met(self, areas: Areas) -> bool {
        match self {
            Need::One(a) => areas.has(a),
            Need::AnyOf(list) => areas.has_any(list),
        }
    }

    /// Die Bereiche, die fehlen, damit das Werkzeug laeuft (fuer die Meldung).
    pub fn labels(self) -> String {
        match self {
            Need::One(a) => a.label().to_string(),
            Need::AnyOf(list) => list
                .iter()
                .map(|a| a.label())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

const DAY_MEETINGS: usize = 8;
const DAY_DICTATIONS: usize = 6;
const DAY_CONVERSATIONS: usize = 6;
const DAY_EVENTS: usize = 15;

const DAY_AREAS: &[Area] = &[
    Area::Meetings,
    Area::Dictations,
    Area::Conversations,
    Area::Calendar,
];

/// Beschreibung eines Werkzeugs, wie `voice::list` sie weitergibt.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub need: Need,
}

fn string_prop(description: &str, max: usize) -> Value {
    json!({ "type": "string", "maxLength": max, "description": description })
}

fn limit_prop(default: i64, max: i64) -> Value {
    json!({ "type": "integer", "minimum": 1, "maximum": max,
            "description": format!("Höchstens so viele Treffer (Standard {default}, höchstens {max}).") })
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required,
            "additionalProperties": false })
}

/// Alle Wissenswerkzeuge in fester Reihenfolge.
pub fn specs() -> Vec<Spec> {
    let date = |what: &str| string_prop(what, 10);
    vec![
        Spec {
            name: "dictations_recent",
            title: "Letzte Diktate lesen",
            description: "Liefert die neuesten Diktate des Nutzers (Zeit und Textauszug), neueste zuerst. Nur lesend.",
            parameters: object(json!({ "limit": limit_prop(5, 20) }), &[]),
            need: Need::One(Area::Dictations),
        },
        Spec {
            name: "dictations_search",
            title: "Diktate durchsuchen",
            description: "Sucht in den letzten Diktaten nach Wörtern (alle müssen vorkommen) und liefert Zeit und Auszug. Nur lesend.",
            parameters: object(
                json!({ "query": string_prop("Suchwörter, z. B. „Grundsteuer Bescheid“.", MAX_QUERY_CHARS),
                        "limit": limit_prop(8, 20) }),
                &["query"],
            ),
            need: Need::One(Area::Dictations),
        },
        Spec {
            name: "conversations_list",
            title: "Frühere Gespräche auflisten",
            description: "Listet frühere Sprachgespräche des Nutzers (Kennung, Titel, Zeit, Anfang des letzten Beitrags), neueste zuerst, ohne das laufende Gespräch. Die Kennung braucht conversation_get. Nur lesend.",
            parameters: object(json!({ "limit": limit_prop(8, 20) }), &[]),
            need: Need::One(Area::Conversations),
        },
        Spec {
            name: "conversations_search",
            title: "Frühere Gespräche durchsuchen",
            description: "Sucht in Titeln und Beiträgen früherer Gespräche (ohne das laufende) und liefert je Gespräch den besten Auszug. Nur lesend.",
            parameters: object(
                json!({ "query": string_prop("Suchwörter.", MAX_QUERY_CHARS),
                        "limit": limit_prop(8, 20) }),
                &["query"],
            ),
            need: Need::One(Area::Conversations),
        },
        Spec {
            name: "conversation_get",
            title: "Früheres Gespräch lesen",
            description: "Liest die letzten Beiträge eines früheren Gesprächs (Kennung aus conversations_list). Das laufende Gespräch ist ausgenommen. Nur lesend.",
            parameters: object(
                json!({ "id": string_prop("Kennung des Gesprächs.", MAX_ID_CHARS) }),
                &["id"],
            ),
            need: Need::One(Area::Conversations),
        },
        Spec {
            name: "calendar_events",
            title: "Termine lesen",
            description: "Liefert Termine aus dem lokalen Kalender-Cache der App (kein Netzabruf, Stand der letzten Aktualisierung) im Zeitraum von bis (Ortszeit, YYYY-MM-DD, bis einschließlich). Ohne Angabe: die nächsten sieben Tage ab heute. Nur lesend.",
            parameters: object(
                json!({ "from": date("Erster Tag (YYYY-MM-DD), Standard heute."),
                        "to": date("Letzter Tag einschließlich (YYYY-MM-DD), Standard sechs Tage nach „from“.") }),
                &[],
            ),
            need: Need::One(Area::Calendar),
        },
        Spec {
            name: "day_overview",
            title: "Tagesüberblick",
            description: "Fasst einen Tag zusammen: Besprechungen, Diktate, Gespräche und Termine mit Titel, Zeit und kurzem Auszug, soweit der Nutzer die Bereiche freigegeben hat (nicht_freigegeben nennt die übrigen). Jede Liste ist gekürzt; „weitere“ nennt je Liste, wie viele fehlen. Dann nachfragen oder das Spezialwerkzeug nutzen (calendar_events, conversations_list, dictations_recent, Besprechungen suchen). Standard heute. Nur lesend.",
            parameters: object(
                json!({ "date": date("Tag (YYYY-MM-DD, Ortszeit), Standard heute.") }),
                &[],
            ),
            need: Need::AnyOf(DAY_AREAS),
        },
        Spec {
            name: "tts_pages_list",
            title: "Vorlesen-Seiten auflisten",
            description: "Listet die Seiten der Vorlesen-Bibliothek (Kennung, Titel, Änderungszeit, Textanfang), zuletzt geänderte zuerst. Nur lesend.",
            parameters: object(json!({ "limit": limit_prop(10, 30) }), &[]),
            need: Need::One(Area::Tts),
        },
        Spec {
            name: "tts_page_text",
            title: "Vorlesen-Seite lesen",
            description: "Liest den Text einer Vorlesen-Seite (Kennung aus tts_pages_list), bei langen Seiten den Anfang. Nur lesend.",
            parameters: object(
                json!({ "id": string_prop("Kennung der Seite.", MAX_ID_CHARS) }),
                &["id"],
            ),
            need: Need::One(Area::Tts),
        },
    ]
}

pub fn find(name: &str) -> Option<Spec> {
    specs().into_iter().find(|s| s.name == name)
}

// ---------------------------------------------------------------------------
// Argumente
// ---------------------------------------------------------------------------

/// Reinigt eine Zeile: Steuer- und Umschaltzeichen weg, Leerraum zusammengezogen.
pub fn clean_line(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| {
            !c.is_control()
                && !matches!(c,
                    '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Die Argumente eines Aufrufs: jedes Feld wird einmal entnommen, am Ende darf nichts uebrig sein.
struct Args {
    map: Map<String, Value>,
}

impl Args {
    fn new(args: &Value) -> Result<Self, String> {
        match args {
            Value::Object(m) => Ok(Self { map: m.clone() }),
            Value::Null => Ok(Self { map: Map::new() }),
            _ => Err("Die Argumente müssen ein Objekt sein.".to_string()),
        }
    }

    fn text(&mut self, key: &str, max: usize) -> Result<Option<String>, String> {
        match self.map.remove(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => {
                let cleaned = clean_line(&s);
                if cleaned.is_empty() {
                    return Ok(None);
                }
                if cleaned.chars().count() > max {
                    return Err(format!("„{key}“ ist zu lang (höchstens {max} Zeichen)."));
                }
                Ok(Some(cleaned))
            }
            Some(_) => Err(format!("„{key}“ muss ein Text sein.")),
        }
    }

    fn required(&mut self, key: &str, max: usize) -> Result<String, String> {
        self.text(key, max)?
            .ok_or_else(|| format!("„{key}“ fehlt."))
    }

    fn limit(&mut self, default: usize, max: usize) -> Result<usize, String> {
        match self.map.remove("limit") {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Number(n)) => match n.as_i64() {
                Some(v) if v >= 1 => Ok((v as usize).min(max)),
                _ => Err(format!(
                    "„limit“ muss eine ganze Zahl von 1 bis {max} sein."
                )),
            },
            Some(_) => Err(format!(
                "„limit“ muss eine ganze Zahl von 1 bis {max} sein."
            )),
        }
    }

    fn done(self) -> Result<(), String> {
        match self.map.keys().next() {
            None => Ok(()),
            // Bewusst ohne den Feldnamen: er stammt aus dem Modelltext und landet im Audit.
            Some(_) => Err(UNKNOWN_FIELD.to_string()),
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// ---------------------------------------------------------------------------
// Zeit (Ortszeit)
// ---------------------------------------------------------------------------

fn local(ms: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&Local))
}

/// „2026-11-01 09:30“ in Ortszeit.
pub fn fmt_time(ms: i64) -> String {
    local(ms)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn fmt_day(ms: i64) -> String {
    local(ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn today(now_ms: i64) -> Result<NaiveDate, String> {
    local(now_ms)
        .map(|d| d.date_naive())
        .ok_or_else(|| "Die Uhr der App ist nicht lesbar.".to_string())
}

fn day_start(date: NaiveDate) -> Result<i64, String> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "Das Datum ist ungültig.".to_string())?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        .map(|d| d.timestamp_millis())
        .ok_or_else(|| "Das Datum ist ungültig.".to_string())
}

/// `[Beginn, Ende)` des Tages in Millisekunden.
pub fn day_bounds(date: NaiveDate) -> Result<(i64, i64), String> {
    let next = date
        .succ_opt()
        .ok_or_else(|| "Das Datum ist ungültig.".to_string())?;
    Ok((day_start(date)?, day_start(next)?))
}

fn parse_date(key: &str, text: &str, now_ms: i64) -> Result<NaiveDate, String> {
    let base = today(now_ms)?;
    match text.to_lowercase().as_str() {
        "heute" => return Ok(base),
        "gestern" => {
            return base
                .pred_opt()
                .ok_or_else(|| "Das Datum ist ungültig.".to_string())
        }
        "morgen" => {
            return base
                .succ_opt()
                .ok_or_else(|| "Das Datum ist ungültig.".to_string())
        }
        _ => {}
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| format!("„{key}“ ist kein Datum. Erwartet: YYYY-MM-DD (z. B. 2026-11-01)."))
}

// ---------------------------------------------------------------------------
// Suche und Auszuege
// ---------------------------------------------------------------------------

fn lower_chars(text: &str) -> Vec<char> {
    text.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

/// Suchwoerter: klein geschrieben, ohne Doppelte, hoechstens [`MAX_TERMS`].
pub fn query_terms(query: &str) -> Vec<Vec<char>> {
    let mut terms: Vec<Vec<char>> = Vec::new();
    for word in query.split_whitespace() {
        let term = lower_chars(word);
        if !term.is_empty() && !terms.contains(&term) {
            terms.push(term);
        }
        if terms.len() == MAX_TERMS {
            break;
        }
    }
    terms
}

fn find_from(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Stelle des ersten Treffers (Zeichenindex), wenn ALLE Woerter vorkommen.
fn match_position(text: &str, terms: &[Vec<char>]) -> Option<usize> {
    if terms.is_empty() {
        return None;
    }
    let hay = lower_chars(text);
    let mut first = usize::MAX;
    for term in terms {
        first = first.min(find_from(&hay, term, 0)?);
    }
    Some(first)
}

/// Kurzer Auszug (einzeilig) um `at` herum, mit Auslassungszeichen.
pub fn excerpt(text: &str, at: usize, max: usize) -> String {
    let chars: Vec<char> = clean_line(text).chars().collect();
    if chars.len() <= max {
        return chars.into_iter().collect();
    }
    let start = at.saturating_sub(max / 3).min(chars.len() - max);
    let end = start + max;
    let body: String = chars[start..end].iter().collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        body.trim(),
        if end < chars.len() { "…" } else { "" }
    )
}

fn head(text: &str, max: usize) -> String {
    excerpt(text, 0, max)
}

// ---------------------------------------------------------------------------
// Ausfuehrung
// ---------------------------------------------------------------------------

/// Umstaende eines Aufrufs.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub areas: Areas,
    /// Das laufende Gespraech: es steht dem Modell ohnehin vor Augen und wird ausgenommen.
    pub current_conversation: Option<&'a str>,
    pub now_ms: i64,
}

impl Context<'_> {
    fn is_current(&self, id: &str) -> bool {
        self.current_conversation == Some(id)
    }

    /// Ohne Kennung des laufenden Gespraechs ist nicht zu sagen, was ausgenommen werden muss:
    /// dann (fail closed) laeuft kein Gespraechswerkzeug.
    fn conversations_open(&self) -> bool {
        self.current_conversation.is_some_and(valid_id)
    }

    fn require_conversation(&self) -> Result<(), String> {
        if self.conversations_open() {
            Ok(())
        } else {
            Err(CONVERSATION_UNKNOWN.to_string())
        }
    }
}

/// Fuehrt ein Wissenswerkzeug aus. Die Freigabe wird hier noch einmal geprueft.
pub fn run(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let spec = find(tool).ok_or_else(|| "Dieses Wissenswerkzeug gibt es nicht.".to_string())?;
    if !spec.need.met(ctx.areas) {
        return Err(not_shared(spec.need));
    }
    let a = Args::new(args)?;
    match tool {
        "dictations_recent" => dictations_recent(source, a),
        "dictations_search" => dictations_search(source, a),
        "conversations_list" => conversations_list(source, ctx, a),
        "conversations_search" => conversations_search(source, ctx, a),
        "conversation_get" => conversation_get(source, ctx, a),
        "calendar_events" => calendar_events(source, ctx, a),
        "day_overview" => day_overview(source, ctx, a),
        "tts_pages_list" => tts_pages_list(source, a),
        "tts_page_text" => tts_page_text(source, a),
        _ => Err("Dieses Wissenswerkzeug gibt es nicht.".to_string()),
    }
}

/// Meldung, wenn ein Bereich nicht frei ist (auch fuer `voice::call`).
pub fn not_shared(need: Need) -> String {
    format!(
        "Der Bereich „{}“ ist für das Sprachgespräch nicht freigegeben. Der Nutzer kann ihn unter „Gesprächseinstellungen → Wissen“ freigeben. Es wurde nichts gelesen.",
        need.labels()
    )
}

fn dictation_json(d: &Dictation, max: usize, at: usize) -> Value {
    json!({ "id": d.id, "time": fmt_time(d.at_ms), "text": excerpt(&d.text, at, max) })
}

fn dictations_recent(source: &dyn KnowledgeSource, mut a: Args) -> Result<Value, String> {
    let limit = a.limit(5, 20)?;
    a.done()?;
    let found = source.dictations(None, None, limit)?;
    let list: Vec<Value> = found
        .iter()
        .map(|d| dictation_json(d, EXCERPT_CHARS, 0))
        .collect();
    Ok(json!({ "dictations": list, "returned": list.len() }))
}

fn dictations_search(source: &dyn KnowledgeSource, mut a: Args) -> Result<Value, String> {
    let query = a.required("query", MAX_QUERY_CHARS)?;
    let limit = a.limit(8, 20)?;
    a.done()?;
    let terms = query_terms(&query);
    let window = source.dictations(None, None, SEARCH_WINDOW)?;
    let list: Vec<Value> = window
        .iter()
        .filter_map(|d| {
            match_position(&d.text, &terms).map(|at| dictation_json(d, EXCERPT_CHARS, at))
        })
        .take(limit)
        .collect();
    Ok(json!({ "dictations": list, "returned": list.len(), "searched_latest": window.len() }))
}

fn conversation_row(c: &ConversationInfo) -> Value {
    json!({ "id": c.id, "title": head(&c.title, 100), "updated": fmt_time(c.updated_ms),
            "entries": c.entries, "last": head(&c.last_text, BRIEF_CHARS) })
}

fn conversations_list(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    mut a: Args,
) -> Result<Value, String> {
    ctx.require_conversation()?;
    let limit = a.limit(8, 20)?;
    a.done()?;
    let list: Vec<Value> = source
        .conversations()?
        .iter()
        .filter(|c| !ctx.is_current(&c.id))
        .take(limit)
        .map(conversation_row)
        .collect();
    Ok(json!({ "conversations": list, "returned": list.len() }))
}

fn conversations_search(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    mut a: Args,
) -> Result<Value, String> {
    ctx.require_conversation()?;
    let query = a.required("query", MAX_QUERY_CHARS)?;
    let limit = a.limit(8, 20)?;
    a.done()?;
    let terms = query_terms(&query);
    let mut hits: Vec<(String, Value)> = Vec::new();
    // Titel zuerst, dann Beitraege; je Gespraech zaehlt der erste (neueste) Treffer.
    for c in source.conversations()? {
        if ctx.is_current(&c.id) || hits.iter().any(|(id, _)| *id == c.id) {
            continue;
        }
        if match_position(&c.title, &terms).is_some() {
            let row = json!({ "id": c.id, "title": head(&c.title, 100),
                              "updated": fmt_time(c.updated_ms), "match": "title",
                              "excerpt": head(&c.last_text, BRIEF_CHARS) });
            hits.push((c.id.clone(), row));
        }
    }
    for e in source.conversation_entries(SEARCH_WINDOW)? {
        if hits.len() >= limit {
            break;
        }
        if ctx.is_current(&e.conversation_id) || hits.iter().any(|(id, _)| *id == e.conversation_id)
        {
            continue;
        }
        if let Some(at) = match_position(&e.text, &terms) {
            let row = json!({ "id": e.conversation_id, "title": head(&e.title, 100),
                              "updated": fmt_time(e.at_ms), "match": e.role,
                              "excerpt": excerpt(&e.text, at, EXCERPT_CHARS) });
            hits.push((e.conversation_id.clone(), row));
        }
    }
    let list: Vec<Value> = hits.into_iter().map(|(_, v)| v).take(limit).collect();
    Ok(json!({ "conversations": list, "returned": list.len() }))
}

fn conversation_get(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    mut a: Args,
) -> Result<Value, String> {
    ctx.require_conversation()?;
    let id = a.required("id", MAX_ID_CHARS)?;
    a.done()?;
    if !valid_id(&id) {
        return Err("Die Kennung ist ungültig. Sie kommt aus conversations_list.".to_string());
    }
    if ctx.is_current(&id) {
        return Err("Das ist das laufende Gespräch; seinen Verlauf kennst du bereits.".to_string());
    }
    let view = source.conversation(&id)?;
    // Die neuesten Beitraege, aelteste zuerst, innerhalb des Zeichenbudgets.
    let mut budget = CONVERSATION_BUDGET_CHARS;
    let mut kept: Vec<Value> = Vec::new();
    for e in view.entries.iter().rev().take(MAX_ENTRIES) {
        let text = head(&e.text, ENTRY_CHARS);
        let cost = text.chars().count();
        if cost > budget {
            break;
        }
        budget -= cost;
        kept.push(json!({ "role": e.role, "time": fmt_time(e.at_ms), "text": text }));
    }
    kept.reverse();
    let omitted = view.entries.len() - kept.len();
    Ok(json!({ "id": view.id, "title": head(&view.title, 100),
               "updated": fmt_time(view.updated_ms), "entries": kept,
               "earlier_entries_omitted": omitted }))
}

const DAY_MS: i64 = 86_400_000;

/// Der Kalendertag eines Ganztags-Zeitpunkts. Die Quellen speichern ihn verschieden: ICS als
/// Mitternacht der Standardzone (Ortszeit), Graph als Mitternacht UTC. Ein Wert genau auf
/// UTC-Mitternacht ist der Graph-Fall (der Tag ist sein UTC-Datum), alles andere zaehlt als
/// Ortszeit. Nur so landet ein ganztaegiger Termin an keiner Zeitzone am Vor- oder Folgetag.
fn all_day_date(ms: i64) -> Option<NaiveDate> {
    if ms.rem_euclid(DAY_MS) == 0 {
        return DateTime::from_timestamp_millis(ms).map(|d| d.date_naive());
    }
    local(ms).map(|d| d.date_naive())
}

/// Erster und letzter Tag (einschliesslich) eines ganztaegigen Termins; das Ende der Quellen
/// ist ausschliessend (Mitternacht nach dem letzten Tag).
fn all_day_span(e: &EventInfo) -> Option<(NaiveDate, NaiveDate)> {
    let first = all_day_date(e.starts_ms)?;
    let last = match all_day_date(e.ends_ms) {
        Some(end) if e.ends_ms > e.starts_ms && end > first => end.pred_opt()?,
        _ => first,
    };
    Some((first, last))
}

/// Beruehrt der Termin die Tage `from` bis `to` (einschliesslich)? Ganztaegige nach Kalendertag,
/// alle anderen nach Zeit.
fn touches(e: &EventInfo, from: NaiveDate, to: NaiveDate, start: i64, end: i64) -> bool {
    if e.all_day {
        return all_day_span(e).is_some_and(|(first, last)| first <= to && last >= from);
    }
    e.starts_ms < end && (e.ends_ms > start || e.starts_ms >= start)
}

/// Die Termine der Tage `from` bis `to`. Die Quelle bekommt je einen Tag Spielraum (ein
/// ganztaegiger Termin liegt je nach Zone neben der Tagesgrenze), gefiltert wird hier.
fn events_between(
    source: &dyn KnowledgeSource,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<EventInfo>, String> {
    let (start, _) = day_bounds(from)?;
    let (_, end) = day_bounds(to)?;
    let mut events: Vec<EventInfo> = source
        .calendar(start - DAY_MS, end + DAY_MS)?
        .into_iter()
        .filter(|e| touches(e, from, to, start, end))
        .collect();
    events.sort_by_key(|e| {
        if e.all_day {
            all_day_span(e)
                .and_then(|(first, _)| day_start(first).ok())
                .unwrap_or(e.starts_ms)
        } else {
            e.starts_ms
        }
    });
    Ok(events)
}

fn event_json(e: &EventInfo) -> Value {
    let mut v = json!({ "title": head(&e.title, 100), "all_day": e.all_day });
    let m = v.as_object_mut().expect("Objekt");
    if let (true, Some((first, last))) = (e.all_day, all_day_span(e)) {
        m.insert("day".to_string(), json!(first.to_string()));
        if last > first {
            m.insert("until".to_string(), json!(last.to_string()));
        }
    } else if e.all_day {
        m.insert("day".to_string(), json!(fmt_day(e.starts_ms)));
    } else {
        m.insert("from".to_string(), json!(fmt_time(e.starts_ms)));
        m.insert("to".to_string(), json!(fmt_time(e.ends_ms)));
    }
    if let Some(place) = e
        .location
        .as_deref()
        .map(|l| head(l, 80))
        .filter(|l| !l.is_empty())
    {
        m.insert("location".to_string(), json!(place));
    }
    v
}

fn calendar_events(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    mut a: Args,
) -> Result<Value, String> {
    let from = match a.text("from", 10)? {
        Some(t) => parse_date("from", &t, ctx.now_ms)?,
        None => today(ctx.now_ms)?,
    };
    let to = match a.text("to", 10)? {
        Some(t) => parse_date("to", &t, ctx.now_ms)?,
        None => from + Duration::days(DEFAULT_SPAN_DAYS - 1),
    };
    a.done()?;
    if to < from {
        return Err("„to“ liegt vor „from“.".to_string());
    }
    if (to - from).num_days() >= MAX_SPAN_DAYS {
        return Err(format!(
            "Der Zeitraum ist zu lang (höchstens {MAX_SPAN_DAYS} Tage)."
        ));
    }
    let events = events_between(source, from, to)?;
    let list: Vec<Value> = events.iter().take(50).map(event_json).collect();
    Ok(
        json!({ "from": from.to_string(), "to": to.to_string(), "events": list,
               "returned": list.len(), "total": events.len(),
               "note": "Stand des lokalen Kalender-Caches, kein Abruf im Netz." }),
    )
}

/// Kappt eine Liste auf `max` Eintraege; was fehlt, steht unter `more[name]`.
fn capped(
    name: &str,
    items: impl Iterator<Item = Value>,
    max: usize,
    more: &mut Map<String, Value>,
) -> Value {
    let all: Vec<Value> = items.collect();
    if all.len() > max {
        more.insert(name.to_string(), json!(all.len() - max));
    }
    Value::Array(all.into_iter().take(max).collect())
}

fn day_overview(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    mut a: Args,
) -> Result<Value, String> {
    let date = match a.text("date", 10)? {
        Some(t) => parse_date("date", &t, ctx.now_ms)?,
        None => today(ctx.now_ms)?,
    };
    a.done()?;
    let (start, end) = day_bounds(date)?;
    let mut out = Map::new();
    let mut more = Map::new();
    out.insert("date".to_string(), json!(date.to_string()));
    if ctx.areas.has(Area::Meetings) {
        let all = source.meetings(start, end, DAY_MEETINGS + 1)?;
        let list = capped(
            "meetings",
            all.iter().map(|m| {
                json!({ "id": m.id, "title": head(&m.title, 100), "time": fmt_time(m.started_ms),
                    "minutes": m.minutes })
            }),
            DAY_MEETINGS,
            &mut more,
        );
        out.insert("meetings".to_string(), list);
    }
    if ctx.areas.has(Area::Dictations) {
        let all = source.dictations(Some(start), Some(end), DAY_DICTATIONS + 1)?;
        let list = capped(
            "dictations",
            all.iter().map(|d| dictation_json(d, BRIEF_CHARS, 0)),
            DAY_DICTATIONS,
            &mut more,
        );
        out.insert("dictations".to_string(), list);
    }
    if ctx.areas.has(Area::Conversations) {
        if ctx.conversations_open() {
            let all = source.conversations()?;
            let list = capped(
                "conversations",
                all.iter()
                    .filter(|c| {
                        !ctx.is_current(&c.id) && c.updated_ms >= start && c.updated_ms < end
                    })
                    .map(conversation_row),
                DAY_CONVERSATIONS,
                &mut more,
            );
            out.insert("conversations".to_string(), list);
        } else {
            out.insert("conversations".to_string(), json!([]));
            out.insert(
                "conversations_hinweis".to_string(),
                json!(CONVERSATION_UNKNOWN),
            );
        }
    }
    if ctx.areas.has(Area::Calendar) {
        let all = events_between(source, date, date)?;
        let list = capped("events", all.iter().map(event_json), DAY_EVENTS, &mut more);
        out.insert("events".to_string(), list);
    }
    if !more.is_empty() {
        out.insert("weitere".to_string(), Value::Object(more));
    }
    let hidden: Vec<&str> = DAY_AREAS
        .iter()
        .filter(|a| !ctx.areas.has(**a))
        .map(|a| a.label())
        .collect();
    if !hidden.is_empty() {
        out.insert("nicht_freigegeben".to_string(), json!(hidden));
    }
    Ok(Value::Object(out))
}

fn tts_pages_list(source: &dyn KnowledgeSource, mut a: Args) -> Result<Value, String> {
    let limit = a.limit(10, 30)?;
    a.done()?;
    let list: Vec<Value> = source
        .pages()?
        .iter()
        .take(limit)
        .map(|p| {
            json!({ "id": p.id, "title": head(&p.title, 100), "modified": fmt_time(p.modified_ms),
                    "preview": head(&p.preview, BRIEF_CHARS) })
        })
        .collect();
    Ok(json!({ "pages": list, "returned": list.len() }))
}

fn tts_page_text(source: &dyn KnowledgeSource, mut a: Args) -> Result<Value, String> {
    let id = a.required("id", MAX_ID_CHARS)?;
    a.done()?;
    if !valid_id(&id) {
        return Err("Die Kennung ist ungültig. Sie kommt aus tts_pages_list.".to_string());
    }
    let text = source.page_text(&id)?;
    let flat = clean_line(&text);
    let total = flat.chars().count();
    let shown: String = flat.chars().take(PAGE_CHARS).collect();
    Ok(json!({ "id": id, "text": shown, "truncated": total > PAGE_CHARS, "chars": total }))
}

#[cfg(test)]
mod tests;
