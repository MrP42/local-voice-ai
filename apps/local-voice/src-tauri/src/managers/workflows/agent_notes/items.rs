//! Das Ergebnis von `agent.extract` (`steps.<id>`), gelesen und BEREINIGT, bevor etwas daraus
//! entsteht (C4).
//!
//! Der Schritt davor hat Texte, Zitate und Daten schon geprueft (`agent::extract`). Hier wird
//! trotzdem nichts blind uebernommen: das Ergebnis steht im Laufkontext (Journal, Vorlagen,
//! ein Mensch kann es bearbeitet haben), und alles darin geht auf ein Transkript zurueck, also
//! auf Text, den ein Gespraechspartner oder ein Video gesprochen hat. Deshalb gilt:
//!
//! - **Texte sind Daten.** Steuerzeichen, Umkehrzeichen der Schreibrichtung, Nullbreiten-Zeichen
//!   und Zeilenumbrueche fallen weg, die Laenge ist gedeckelt. Wie sie in eine Markdown-Notiz
//!   kommen, regelt `render` (Maskierung); in den Kopf (Frontmatter) gelangt nie ein Text aus
//!   dem Transkript ausser dem Titel der Besprechung, und der steht in Anfuehrungszeichen.
//! - **Daten sind ISO und echte Tage.** Ein Datum, das nicht als `JJJJ-MM-TT` lesbar ist, ist
//!   kein Datum: bei einer Frist faellt der Eintrag weg, bei einem To-do nur das Datum.
//! - **Unbelegte Daten sind gekennzeichnet.** Nur was der Code selbst aus einer Zeitangabe oder
//!   dem Zitat aufgeloest hat (`due_source` beginnt mit `angabe:` oder `zitat:`), gilt als
//!   gesichert. Stammt das Datum vom Sprachmodell (`modell`) oder fehlt die Herkunft ganz,
//!   steht `unverified_date` und jede Ausgabe nennt es.

use chrono::NaiveDate;
use serde_json::Value;

use crate::managers::workflows::action::StepError;

/// Hoechstzahl Eintraege, die aus dem Ergebnis uebernommen werden (die Engine begrenzt
/// jede Liste ohnehin auf 25).
pub const MAX_ITEMS_PER_KIND: usize = 30;
pub const TEXT_CHARS: usize = 300;
pub const QUOTE_CHARS: usize = 200;
pub const NAME_CHARS: usize = 80;
pub const PHRASE_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Todo,
    Deadline,
    Decision,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Todo => "todo",
            Kind::Deadline => "deadline",
            Kind::Decision => "decision",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub text: String,
    pub assignee: Option<String>,
    pub due: Option<NaiveDate>,
    pub due_phrase: Option<String>,
    /// Das Datum hat das Sprachmodell geschaetzt oder seine Herkunft ist unbekannt.
    pub unverified_date: bool,
    /// Segmentnummern (`S5` -> 5), aufsteigend, ohne Doppelte.
    pub segments: Vec<u32>,
    pub quote: String,
    pub confidence: Option<f64>,
}

impl Item {
    /// Stabile Kennung dieses Eintrags fuer Dublettenschutz und Herkunftsregister: Art, Tag und
    /// Text (ohne Gross-/Kleinschreibung und Leerraum). Dieselbe Frist in einem zweiten Lauf
    /// ergibt dieselbe Kennung.
    pub fn key(&self) -> String {
        use sha2::{Digest, Sha256};
        let text: String = self
            .text
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let day = self
            .due
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let digest = Sha256::digest(format!("{}|{day}|{text}", self.kind.as_str()).as_bytes());
        digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }
}

/// Die Herkunft des Ergebnisses (Modell, Aufwand), fuer Notiz und Herkunftsregister.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Origin {
    pub model: Option<String>,
    pub local: bool,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Extracted {
    pub meeting_id: String,
    pub meeting_date: Option<NaiveDate>,
    pub items: Vec<Item>,
    /// Eintraege, die hier (nicht erst im Schritt davor) wegfielen: kein lesbares Datum bei einer Frist.
    pub dropped_here: usize,
    /// Verworfene Eintraege laut Schritt davor (ohne Beleg, ungueltiges Datum ...).
    pub dropped_before: u64,
    pub origin: Origin,
    /// Alle belegten Segmente (Quellen fuer das Herkunftsregister), aufsteigend.
    pub segments: Vec<u32>,
}

impl Extracted {
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.kind == kind)
    }

    pub fn count(&self, kind: Kind) -> usize {
        self.of_kind(kind).count()
    }
}

/// Was der Vorschritt geliefert hat.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Data(Box<Extracted>),
    /// Kein Ergebnis, aber kein Fehler dieses Schritts (Vorschritt uebersprungen oder
    /// gescheitert mit `on_error: continue`, `no_action`): nichts zu tun, mit Grund.
    Nothing(String),
}

/// Ein Text aus dem Transkript als EINE Zeile: Steuerzeichen, Schreibrichtungs- und
/// Nullbreiten-Zeichen weg, Leerraum zusammengefasst, gekuerzt (mit `…`).
pub fn clean(s: &str, max: usize) -> String {
    let mut flat = String::with_capacity(s.len());
    for c in s.chars() {
        if is_invisible(c) {
            continue;
        }
        flat.push(if c.is_control() { ' ' } else { c });
    }
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        flat
    }
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' // Nullbreite, Schreibrichtung
            | '\u{202A}'..='\u{202E}' // Umkehrzeichen
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}' // Isolate
            | '\u{FEFF}'
    )
}

fn text_of(v: &Value, key: &str, max: usize) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(|s| clean(s, max))
        .filter(|s| !s.is_empty())
}

fn date_of(v: &Value, key: &str) -> Option<NaiveDate> {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
}

/// `[5, 6]` oder `["S5", "S6"]` -> 5, 6 (nur Zahlen bis 1 000 000, aufsteigend, ohne Doppelte).
fn segments_of(v: &Value) -> Vec<u32> {
    let mut out: Vec<u32> = v
        .get("segments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| match s {
                    Value::Number(n) => n.as_u64(),
                    Value::String(t) => t.trim().trim_start_matches(['S', 's']).parse().ok(),
                    _ => None,
                })
                .filter(|n| *n <= 1_000_000)
                .map(|n| n as u32)
                .collect()
        })
        .unwrap_or_default();
    out.sort_unstable();
    out.dedup();
    out.truncate(40);
    out
}

fn confidence_of(v: &Value, key: &str) -> Option<f64> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite() && (0.0..=1.0).contains(c))
}

fn item_of(kind: Kind, v: &Value) -> Result<Item, ()> {
    let text = text_of(v, "text", TEXT_CHARS).ok_or(())?;
    let due = date_of(v, "due");
    let (assignee, due_phrase, unverified_date) = match kind {
        Kind::Decision => (None, None, false),
        _ => {
            let source = v.get("due_source").and_then(Value::as_str).unwrap_or("");
            let verified = source.starts_with("angabe:") || source.starts_with("zitat:");
            (
                if kind == Kind::Todo {
                    text_of(v, "assignee", NAME_CHARS)
                } else {
                    None
                },
                text_of(v, "due_phrase", PHRASE_CHARS),
                due.is_some() && !verified,
            )
        }
    };
    // Eine Frist ohne Tag ist keine Frist.
    if kind == Kind::Deadline && due.is_none() {
        return Err(());
    }
    Ok(Item {
        kind,
        text,
        assignee,
        due: if kind == Kind::Decision { None } else { due },
        due_phrase,
        unverified_date,
        segments: segments_of(v),
        quote: text_of(v, "quote", QUOTE_CHARS).unwrap_or_default(),
        confidence: confidence_of(v, "confidence"),
    })
}

/// Liest `steps.<from>` aus dem Laufkontext.
///
/// `Err(Permanent)`: es gibt keinen solchen Schritt (Tippfehler, falsche Reihenfolge): das
/// wird nie besser. `Ok(Nothing)`: der Schritt lief nicht oder lieferte nichts.
pub fn read(context: &Value, from: &str) -> Result<Input, StepError> {
    let Some(step) = context.pointer(&format!("/steps/{from}")) else {
        return Err(StepError::Permanent(format!(
            "Der Schritt „{from}“ gibt es vor diesem Schritt nicht: davor muss „Aufgaben, Fristen und Entscheidungen extrahieren“ stehen."
        )));
    };
    if !step.is_object() {
        return Err(StepError::Permanent(format!(
            "Der Schritt „{from}“ hat kein Ergebnis."
        )));
    }
    if step.get("ok").and_then(Value::as_bool) == Some(false) {
        return Ok(Input::Nothing(format!(
            "Der Schritt „{from}“ hat nichts geliefert."
        )));
    }
    match step.get("outcome").and_then(Value::as_str) {
        Some("extracted") => {}
        Some(_) => {
            let why = text_of(step, "reason_text", 200)
                .unwrap_or_else(|| "Es wurde nichts extrahiert.".to_string());
            return Ok(Input::Nothing(why));
        }
        None => {
            return Err(StepError::Permanent(format!(
                "Der Schritt „{from}“ ist kein Extraktionsschritt (kein Ergebnis mit „outcome“)."
            )))
        }
    }
    let meeting_id = step
        .get("meeting_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let mut items = Vec::new();
    let mut dropped_here = 0usize;
    for (key, kind) in [
        ("todos", Kind::Todo),
        ("deadlines", Kind::Deadline),
        ("decisions", Kind::Decision),
    ] {
        let list = step.get(key).and_then(Value::as_array);
        for raw in list.into_iter().flatten().take(MAX_ITEMS_PER_KIND) {
            match item_of(kind, raw) {
                Ok(item) => items.push(item),
                Err(()) => dropped_here += 1,
            }
        }
    }
    let prov = step.get("provenance").cloned().unwrap_or(Value::Null);
    let origin = Origin {
        model: text_of(&prov, "model", 120),
        local: prov.get("locality").and_then(Value::as_str) == Some("local"),
        prompt_tokens: prov.get("prompt_tokens").and_then(Value::as_u64),
        completion_tokens: prov.get("completion_tokens").and_then(Value::as_u64),
        duration_ms: prov.get("duration_ms").and_then(Value::as_u64),
        confidence: confidence_of(&prov, "confidence"),
    };
    let mut segments: Vec<u32> = items.iter().flat_map(|i| i.segments.clone()).collect();
    segments.sort_unstable();
    segments.dedup();
    Ok(Input::Data(Box::new(Extracted {
        meeting_id,
        meeting_date: date_of(step, "meeting_date"),
        items,
        dropped_here,
        dropped_before: step
            .pointer("/counts/dropped")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        origin,
        segments,
    })))
}
