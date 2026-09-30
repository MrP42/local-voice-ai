//! Vorschau einer Freigabe-Anfrage fuer den Nutzer (A1n).
//!
//! Der Nutzer genehmigt, was er sieht. Eine Vorschau, die den Gesamtstring nach N
//! Zeichen abschneidet, laesst Empfaenger hinter einem langen Text verschwinden.
//! Deshalb ist die Vorschau gegliedert:
//!
//! 1. `Ziel: ...` (die Zielangabe der Anfrage), hoechstens `MAX_TARGET_CHARS`.
//! 2. Alle **sicherheitsrelevanten Felder** (Empfaenger `to`/`cc`/`bcc`, Pfade,
//!    Adressen, Anhaenge, Ziele ...) in der Tiefe der Argumente gefunden, jedes
//!    VOLLSTAENDIG. Ist eines laenger als `MAX_SAFETY_FIELD_CHARS` oder sprengen
//!    alle zusammen `MAX_SAFETY_TOTAL_CHARS`, gibt es KEINE Vorschau: `build`
//!    meldet einen Fehler und der Aufrufer lehnt die Anfrage ab, statt sie
//!    gekuerzt zur Freigabe vorzulegen.
//! 3. Alle uebrigen Felder, je Feld auf `MAX_OTHER_FIELD_CHARS` gekuerzt mit sichtbarer
//!    Marke (`… [gekürzt, insgesamt N Zeichen]`); passen nicht alle, steht am Ende
//!    `… [+N weitere Felder nicht angezeigt]`.
//!
//! Eine Struktur, die sich nicht zuverlaessig einordnen laesst (mehr als
//! `MAX_NODES` Knoten oder tiefer als `MAX_DEPTH`), wird ebenfalls abgelehnt.
//!
//! Darstellung: Jedes Feld steht auf genau EINER Zeile (`• name: wert`), das Ziel auf
//! `Ziel: ...`. Zeilenumbrueche, Steuerzeichen und unsichtbare Zeichen im Inhalt werden
//! sichtbar ersetzt (`⏎`, `⟦U+202E⟧`): ein Wert kann keine Zeile und kein Feld
//! vortaeuschen. Geheimnisse sind geschwaerzt (`audit::redact_text`, geheime
//! Schluessel `***`). Adressen bleiben vollstaendig: der Nutzer muss das Ziel sehen.

use std::fmt;

use serde_json::Value;

use super::audit::{is_secret_key, key_segments, redact_text};

/// Laengste Zielangabe (nach Schwaerzung und Ersetzen der Steuerzeichen).
pub const MAX_TARGET_CHARS: usize = 300;
/// Laengster Wert eines sicherheitsrelevanten Feldes.
pub const MAX_SAFETY_FIELD_CHARS: usize = 300;
/// Summe aller Zeilen der sicherheitsrelevanten Felder.
pub const MAX_SAFETY_TOTAL_CHARS: usize = 1800;
/// Laengster angezeigter Wert eines uebrigen Feldes (danach gekuerzt mit Marke).
pub const MAX_OTHER_FIELD_CHARS: usize = 160;
/// Summe der Zeilen der uebrigen Felder.
pub const MAX_OTHER_TOTAL_CHARS: usize = 1200;
/// Laengster angezeigter Feldname.
pub const MAX_NAME_CHARS: usize = 80;
/// Hoechstzahl Knoten (Felder, Objekte, Listen) in den Argumenten.
pub const MAX_NODES: usize = 400;
/// Tiefste Verschachtelung der Argumente.
pub const MAX_DEPTH: usize = 8;

/// Warum es keine Vorschau gibt: die Anfrage ist abzulehnen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewError {
    TargetTooLong { chars: usize },
    FieldTooLong { name: String, chars: usize },
    SafetyFieldsTooLarge,
    TooComplex,
}

impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreviewError::TargetTooLong { chars } => write!(
                f,
                "Das Ziel ist zu lang für die Freigabe ({chars} Zeichen, höchstens {MAX_TARGET_CHARS}). Die Anfrage wurde abgelehnt."
            ),
            PreviewError::FieldTooLong { name, chars } => write!(
                f,
                "Das Feld „{name}“ ist zu lang, um es vollständig zur Freigabe anzuzeigen ({chars} Zeichen, höchstens {MAX_SAFETY_FIELD_CHARS}). Die Anfrage wurde abgelehnt."
            ),
            PreviewError::SafetyFieldsTooLarge => write!(
                f,
                "Empfänger, Pfade und Ziele der Anfrage sind zusammen zu umfangreich, um sie vollständig zur Freigabe anzuzeigen. Die Anfrage wurde abgelehnt."
            ),
            PreviewError::TooComplex => write!(
                f,
                "Die Argumente der Anfrage sind zu umfangreich oder zu tief verschachtelt, um sie zur Freigabe anzuzeigen. Die Anfrage wurde abgelehnt."
            ),
        }
    }
}

impl std::error::Error for PreviewError {}

/// Ganze, normalisierte Schluesselnamen, die Empfaenger, Orte oder Anhaenge tragen.
const SAFETY_EXACT: &[&str] = &[
    "to",
    "cc",
    "bcc",
    "from",
    "sender",
    "replyto",
    "email",
    "emails",
    "mail",
    "address",
    "addresses",
    "adresse",
    "path",
    "paths",
    "pfad",
    "pfade",
    "file",
    "files",
    "datei",
    "dateien",
    "folder",
    "ordner",
    "dir",
    "directory",
    "dest",
    "target",
    "targets",
    "ziel",
    "url",
    "urls",
    "uri",
    "link",
    "links",
    "host",
    "invitees",
    "attendees",
    "an",
    "von",
    "kopie",
];

/// Letztes Wortsegment eines Namens, das den Inhalt als Empfaenger oder Ort ausweist
/// (`reply_to`, `target_url`, `source_path`, `user_email`); `mail_body` und
/// `file_content` zaehlen bewusst NICHT, sonst waere jeder lange Text ein Fehler.
const SAFETY_LAST_SEGMENT: &[&str] = &[
    "to",
    "cc",
    "bcc",
    "path",
    "paths",
    "pfad",
    "url",
    "urls",
    "uri",
    "dest",
    "target",
    "ziel",
    "link",
    "links",
    "file",
    "files",
    "folder",
    "ordner",
    "dir",
    "directory",
    "address",
    "addresses",
    "adresse",
    "email",
    "emails",
    "host",
    "datei",
];

/// Erstes Wortsegment: `to_list`, `cc_addresses`.
const SAFETY_FIRST_SEGMENT: &[&str] = &["to", "cc", "bcc"];

/// Teile eines normalisierten Namens, die genuegen (laengere Woerter, bei denen ein
/// Teilstring-Treffer kein Zufall ist).
const SAFETY_HINTS: &[&str] = &[
    "recipient",
    "empfaenger",
    "teilnehmer",
    "attach",
    "anhang",
    "anhaeng",
    "filename",
    "filepath",
    "dateiname",
    "pathname",
    "destination",
    "mailto",
    "webhook",
    "endpoint",
];

/// Kleinbuchstaben, Umlaute aufgeloest, nur ASCII-Buchstaben und -Ziffern.
fn normalize_key(key: &str) -> String {
    key.to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// Deutet der Schluessel auf Empfaenger, Orte oder Anhaenge?
fn is_safety_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    if SAFETY_HINTS.iter().any(|h| normalized.contains(h))
        || SAFETY_EXACT.contains(&normalized.as_str())
    {
        return true;
    }
    let words: Vec<String> = key_segments(key)
        .into_iter()
        .filter(|s| s.bytes().any(|b| b.is_ascii_alphabetic()))
        .collect();
    words
        .first()
        .is_some_and(|w| SAFETY_FIRST_SEGMENT.contains(&w.as_str()))
        || words
            .last()
            .is_some_and(|w| SAFETY_LAST_SEGMENT.contains(&w.as_str()))
}

fn is_invisible(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x200B..=0x200F
            | 0x2028..=0x2029
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x2069
            | 0xFEFF
    )
}

/// Macht Steuerzeichen und unsichtbare Zeichen sichtbar (siehe Moduldoku).
fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\r' => out.push('⏎'),
            '\t' => out.push(' '),
            c if c.is_control() || is_invisible(c) => {
                out.push_str(&format!("⟦U+{:04X}⟧", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn clip_with_marker(s: &str, max: usize) -> String {
    let chars = s.chars().count();
    if chars <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}… [gekürzt, insgesamt {chars} Zeichen]")
}

struct Field {
    name: String,
    value: String,
    safety: bool,
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => redact_text(s),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn collect(
    v: &Value,
    name: &str,
    safety: bool,
    secret: bool,
    depth: usize,
    nodes: &mut usize,
    out: &mut Vec<Field>,
) -> Result<(), PreviewError> {
    *nodes += 1;
    if *nodes > MAX_NODES || depth > MAX_DEPTH {
        return Err(PreviewError::TooComplex);
    }
    match v {
        Value::Object(map) => {
            for (key, val) in map {
                let child = if name.is_empty() {
                    key.clone()
                } else {
                    format!("{name}.{key}")
                };
                collect(
                    val,
                    &child,
                    safety || is_safety_key(key),
                    secret || is_secret_key(key),
                    depth + 1,
                    nodes,
                    out,
                )?;
            }
        }
        Value::Array(items) => {
            for (i, val) in items.iter().enumerate() {
                collect(
                    val,
                    &format!("{name}[{i}]"),
                    safety,
                    secret,
                    depth + 1,
                    nodes,
                    out,
                )?;
            }
        }
        scalar => out.push(Field {
            name: name.to_string(),
            value: if secret {
                "***".to_string()
            } else {
                scalar_text(scalar)
            },
            safety,
        }),
    }
    Ok(())
}

fn shown_name(name: &str) -> String {
    let n = visible(name);
    if n.is_empty() {
        return "(ohne Namen)".to_string();
    }
    let chars = n.chars().count();
    if chars <= MAX_NAME_CHARS {
        n
    } else {
        let head: String = n.chars().take(MAX_NAME_CHARS).collect();
        format!("{head}…")
    }
}

fn shown_value(value: &str) -> String {
    let v = visible(value);
    if v.is_empty() {
        "(leer)".to_string()
    } else {
        v
    }
}

/// Baut die Vorschau (siehe Moduldoku). `Err`: die Anfrage ist abzulehnen.
pub fn build(target: Option<&str>, args: Option<&Value>) -> Result<String, PreviewError> {
    let mut lines: Vec<String> = Vec::new();
    if let Some(t) = target.filter(|t| !t.is_empty()) {
        let shown = visible(&redact_text(t));
        let chars = shown.chars().count();
        if chars > MAX_TARGET_CHARS {
            return Err(PreviewError::TargetTooLong { chars });
        }
        lines.push(format!("Ziel: {shown}"));
    }

    let mut fields: Vec<Field> = Vec::new();
    if let Some(args) = args.filter(|a| !a.is_null()) {
        let mut nodes = 0;
        match args {
            Value::Object(_) => collect(args, "", false, false, 0, &mut nodes, &mut fields)?,
            // Ein Argument ohne Namen kann alles sein, auch ein Empfaenger.
            other => collect(other, "Argument", true, false, 0, &mut nodes, &mut fields)?,
        }
    }

    let mut safety_total = 0;
    for f in fields.iter().filter(|f| f.safety) {
        let name = shown_name(&f.name);
        let value = shown_value(&f.value);
        let chars = value.chars().count();
        if chars > MAX_SAFETY_FIELD_CHARS {
            return Err(PreviewError::FieldTooLong { name, chars });
        }
        let line = format!("• {name}: {value}");
        safety_total += line.chars().count();
        if safety_total > MAX_SAFETY_TOTAL_CHARS {
            return Err(PreviewError::SafetyFieldsTooLarge);
        }
        lines.push(line);
    }

    let mut other_total = 0;
    let mut hidden = 0;
    for f in fields.iter().filter(|f| !f.safety) {
        let line = format!(
            "• {}: {}",
            shown_name(&f.name),
            clip_with_marker(&shown_value(&f.value), MAX_OTHER_FIELD_CHARS)
        );
        let len = line.chars().count();
        if hidden > 0 || other_total + len > MAX_OTHER_TOTAL_CHARS {
            hidden += 1;
            continue;
        }
        other_total += len;
        lines.push(line);
    }
    if hidden > 0 {
        lines.push(format!("… [+{hidden} weitere Felder nicht angezeigt]"));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests;
