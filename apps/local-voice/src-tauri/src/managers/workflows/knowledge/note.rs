//! Die Notiz im Vault (B6): planen, rendern, schreiben, ohne Dublette.
//!
//! Aufbauend auf `integrations::obsidian` (A6): derselbe Frontmatter-Vertrag des AI-OS (`title`, `tags`,
//! `context_area`, `data_class`, `sensitivity`, `tier`, `lva_id`), dieselbe Pfad-Sandbox, dieselbe Suche
//! nach der Kennung (`find_by_id`), dieselben Marken `<!-- lva:begin -->`/`<!-- lva:end -->`.
//!
//! # Zwei Formen
//!
//! - **Einzelne Notiz** (`entry` leer): Frontmatter, dann EIN verwalteter Block zwischen den Marken. Gibt es
//!   die Notiz schon (gleicher Schluessel), wird nur dieser Block ersetzt und das Frontmatter nachgezogen
//!   (`updated`, die Felder aus `meta`, eine weitere Quelle unter `quellen`); alles ausserhalb der Marken
//!   (Handarbeit, geaendertes `context_area` oder `tier` nach der Triage) bleibt stehen.
//! - **Sammelnotiz** (`entry` gesetzt, z. B. Kanal): Frontmatter, Titel, die Marke `<!-- lva:entries -->`
//!   und darunter je Eintrag ein Block mit eigenen Marken `<!-- lva:entry:<ID> -->` … `<!-- lva:entry-end:<ID> -->`.
//!   Derselbe Eintrag wird ersetzt, ein neuer kommt direkt unter die Marke (neueste oben).
//!
//! # Zusagen
//!
//! - **Keine Dublette**: [`plan`] sucht im ganzen Vault nach `lva_id: "<Schluessel>"`; gefunden -> Aendern,
//!   sonst Anlegen. Ist der Vault zu gross, um sicher zu pruefen, wird NICHT angelegt (`ScanLimit`).
//! - **Reine Rechnung**: [`plan`] aendert nichts und haengt nur vom Vault und der Spezifikation ab (kein
//!   Uhrzeit-Wert im Text); dieselbe Spezifikation ergibt denselben Text und dieselbe Pruefsumme. Darauf
//!   beruhen Freigabe (Bindung an die Pruefsumme), Wiederholung und die Wiederaufnahme nach einem Absturz
//!   (`Unchanged`).
//! - **Handarbeit bleibt**; Marken und Frontmatter sind nicht faelschbar: aller Text aus Modell und fremden
//!   Quellen wird vor dem Einfuegen entschaerft (`md_block`, `md_inline`), Schluessel und Eintragskennung
//!   bestehen nur aus `A-Za-z0-9_-`.
//! - **Atomar**: Anlegen ueber `Sandbox::write_bytes_new` (neue Datei, bei Namenskollision `Name (2).md`,
//!   nie ein Ueberschreiben einer fremden Datei), Aendern ueber `Sandbox::replace_bytes` (Nachbardatei,
//!   dann umbenennen).

use std::path::PathBuf;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::managers::integrations::folder::{FolderError, Sandbox};
use crate::managers::integrations::obsidian::{
    self, ObsidianConfig, ObsidianError, BEGIN_MARK, END_MARK,
};

use super::md_inline;

/// Ein Schloss um „planen und schreiben“: zwei Schritte in diesem Prozess schreiben nie zugleich in
/// denselben Vault (die Engine hat zwar nur einen Arbeiter; Tests und kuenftige Aufrufer nicht).
pub static VAULT_LOCK: Mutex<()> = Mutex::new(());

pub const ENTRIES_MARK: &str = "<!-- lva:entries -->";
pub const MAX_KEY_CHARS: usize = 80;
pub const MAX_CONTENT_CHARS: usize = 100_000;

pub fn entry_begin(id: &str) -> String {
    format!("<!-- lva:entry:{id} -->")
}

pub fn entry_end(id: &str) -> String {
    format!("<!-- lva:entry-end:{id} -->")
}

/// Schluessel und Eintragskennung: nur so entstehen Marken, die niemand faelschen kann.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_KEY_CHARS
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Felder, die der Vault-Vertrag oder die App belegt: `meta` darf sie nicht setzen.
pub const RESERVED_META: &[&str] = &[
    "title",
    "tags",
    "context_area",
    "data_class",
    "sensitivity",
    "tier",
    "status",
    "updated",
    "lva_id",
    "lva_quelle",
    "quelle_url",
    "quellen",
];

pub fn valid_meta_key(s: &str) -> bool {
    let mut chars = s.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    first_ok
        && s.len() <= 30
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !RESERVED_META.contains(&s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataClass {
    Internal,
    Confidential,
}

impl DataClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DataClass::Internal => "internal",
            DataClass::Confidential => "confidential",
        }
    }
}

/// Ein Wert des Frontmatters: schon als YAML geschrieben (Text in Anfuehrungszeichen, Zahl, Ja/Nein).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaValue(pub String);

#[derive(Clone, Debug)]
pub struct NoteSpec {
    pub key: String,
    pub title: String,
    pub content: String,
    /// Dateiname ohne `.md`; Vorgabe: Datum und Titel.
    pub name: Option<String>,
    /// Unterordner im Vault; Vorgabe: der der Integration.
    pub folder: Option<String>,
    /// `JJJJ-MM-TT`.
    pub date: String,
    pub entry: Option<String>,
    pub source_title: Option<String>,
    pub source_url: Option<String>,
    pub origin: Option<String>,
    pub tags: Vec<String>,
    pub meta: Vec<(String, MetaValue)>,
    pub data_class: DataClass,
    pub auto: bool,
}

/// YAML-Text in doppelten Anfuehrungszeichen (Zeilenumbrueche und Steuerzeichen werden Leerzeichen).
pub fn yq(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn safe_url(url: &str) -> Option<&str> {
    let u = url.trim();
    ((u.starts_with("https://") || u.starts_with("http://"))
        && !u.chars().any(|c| c.is_whitespace() || c.is_control())
        && u.len() <= 500)
        .then_some(u)
}

/// Text fuer die eckigen Klammern eines Markdown-Links.
fn link_text(s: &str) -> String {
    s.replace('[', "(").replace(']', ")")
}

/// Die Quelle als Zeile fuer `quellen` im Frontmatter.
fn source_line(spec: &NoteSpec) -> Option<String> {
    let title = spec
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let url = spec.source_url.as_deref().and_then(safe_url);
    match (title, url) {
        (Some(t), Some(u)) => Some(format!("{t} ({u})")),
        (Some(t), None) => Some(t.to_string()),
        (None, Some(u)) => Some(u.to_string()),
        (None, None) => None,
    }
}

/// Die Fusszeilen des verwalteten Teils: Quelle und Herkunft.
fn footer(spec: &NoteSpec) -> String {
    let mut out = String::new();
    let title = spec
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let url = spec.source_url.as_deref().and_then(safe_url);
    match (title, url) {
        (Some(t), Some(u)) => out.push_str(&format!("*Quelle: [{}]({u}).*\n", link_text(t))),
        (Some(t), None) => out.push_str(&format!("*Quelle: {t}.*\n")),
        (None, Some(u)) => out.push_str(&format!("*Quelle: <{u}>.*\n")),
        (None, None) => {}
    }
    if let Some(o) = spec
        .origin
        .as_deref()
        .map(str::trim)
        .filter(|o| !o.is_empty())
    {
        out.push_str(&format!("*Herkunft: {o}.*\n"));
    }
    out
}

fn tags_line(spec: &NoteSpec) -> String {
    let mut tags: Vec<String> = spec.tags.clone();
    if !tags.iter().any(|t| t == "local-voice-ai") {
        tags.push("local-voice-ai".to_string());
    }
    format!("[{}]", tags.join(", "))
}

fn meta_lines(spec: &NoteSpec) -> String {
    spec.meta
        .iter()
        .map(|(k, v)| format!("{k}: {}\n", v.0))
        .collect()
}

fn frontmatter(cfg: &ObsidianConfig, spec: &NoteSpec) -> String {
    let mut fm = String::from("---\n");
    fm.push_str(&format!("title: {}\n", yq(spec.title.trim())));
    fm.push_str(&format!("tags: {}\n", tags_line(spec)));
    fm.push_str(&format!("context_area: {}\n", cfg.context_area));
    fm.push_str(&format!("data_class: {}\n", spec.data_class.as_str()));
    fm.push_str("sensitivity: \"normal\"\n");
    fm.push_str(&format!("tier: {}\n", cfg.tier));
    fm.push_str("status: entwurf\n");
    fm.push_str(&format!("updated: {}\n", yq(&spec.date)));
    fm.push_str(&format!("lva_id: {}\n", yq(&spec.key)));
    fm.push_str("lva_quelle: workflow\n");
    fm.push_str(&meta_lines(spec));
    if let Some(u) = spec.source_url.as_deref().and_then(safe_url) {
        fm.push_str(&format!("quelle_url: {}\n", yq(u)));
    }
    if let Some(line) = source_line(spec) {
        fm.push_str("quellen:\n");
        fm.push_str(&format!("  - {}\n", yq(&line)));
    }
    fm.push_str("---\n");
    fm
}

/// Der verwaltete Block einer einzelnen Notiz (mit Marken).
fn single_block(spec: &NoteSpec) -> String {
    let mut b = String::new();
    b.push_str(BEGIN_MARK);
    b.push('\n');
    b.push_str(spec.content.trim_end());
    b.push('\n');
    let f = footer(spec);
    if !f.is_empty() {
        b.push_str("\n---\n");
        b.push_str(&f);
    }
    b.push_str(END_MARK);
    b.push('\n');
    b
}

/// Der Block eines Eintrags der Sammelnotiz (mit Marken).
fn entry_block(spec: &NoteSpec, id: &str) -> String {
    let mut b = String::new();
    b.push_str(&entry_begin(id));
    b.push('\n');
    b.push_str(spec.content.trim_end());
    b.push('\n');
    let f = footer(spec);
    if !f.is_empty() {
        b.push('\n');
        b.push_str(&f);
    }
    b.push_str(&entry_end(id));
    b.push('\n');
    b
}

/// Eine neue Notiz, Byte fuer Byte reproduzierbar.
pub fn render_new(cfg: &ObsidianConfig, spec: &NoteSpec) -> String {
    let mut out = frontmatter(cfg, spec);
    out.push('\n');
    match &spec.entry {
        None => out.push_str(&single_block(spec)),
        Some(id) => {
            out.push_str(&format!("# {}\n\n", spec.title.trim()));
            out.push_str(ENTRIES_MARK);
            out.push_str("\n\n");
            out.push_str(&entry_block(spec, id));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Vorhandene Notiz nachziehen
// ---------------------------------------------------------------------------

/// Ersetzt den Bereich von `begin` bis `end` (samt Zeilenumbruch dahinter) durch `block`. `None`: Marken fehlen.
fn replace_region(text: &str, begin: &str, end: &str, block: &str) -> Option<String> {
    let b = text.find(begin)?;
    let e_rel = text[b..].find(end)?;
    let e = b + e_rel + end.len();
    let e = if text[e..].starts_with("\r\n") {
        e + 2
    } else if text[e..].starts_with('\n') {
        e + 1
    } else {
        e
    };
    Some(format!("{}{}{}", &text[..b], block, &text[e..]))
}

fn merge_body(existing: &str, spec: &NoteSpec) -> String {
    match &spec.entry {
        None => {
            let block = single_block(spec);
            replace_region(existing, BEGIN_MARK, END_MARK, &block).unwrap_or_else(|| {
                let mut t = existing.trim_end().to_string();
                t.push_str("\n\n");
                t.push_str(&block);
                t
            })
        }
        Some(id) => {
            let block = entry_block(spec, id);
            if let Some(replaced) =
                replace_region(existing, &entry_begin(id), &entry_end(id), &block)
            {
                return replaced;
            }
            // Neuer Eintrag: direkt unter die Marke, sonst ans Ende.
            if let Some(at) = existing.find(ENTRIES_MARK) {
                let mut line_end = at + ENTRIES_MARK.len();
                if existing[line_end..].starts_with("\r\n") {
                    line_end += 2;
                } else if existing[line_end..].starts_with('\n') {
                    line_end += 1;
                }
                let tail = existing[line_end..].trim_start_matches(['\r', '\n']);
                let mut out = format!("{}\n{}", &existing[..line_end], block);
                if !tail.is_empty() {
                    out.push('\n');
                    out.push_str(tail);
                }
                return out;
            }
            let mut t = existing.trim_end().to_string();
            t.push_str("\n\n");
            t.push_str(&block);
            t
        }
    }
}

/// Setzt `key: value` im Frontmatter (nur oberste Ebene); fehlt der Schluessel, kommt die Zeile ans Ende.
fn set_scalar(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}:");
    match lines.iter().position(|l| l.trim_end().starts_with(&prefix)) {
        Some(i) => lines[i] = format!("{key}: {value}"),
        None => lines.push(format!("{key}: {value}")),
    }
}

/// Haengt `item` an die Liste `key` (nur, wenn es dort noch nicht steht).
fn add_list_item(lines: &mut Vec<String>, key: &str, item: &str) {
    let head = format!("{key}:");
    let line = format!("  - {item}");
    match lines.iter().position(|l| l.trim_end() == head) {
        None => {
            lines.push(head);
            lines.push(line);
        }
        Some(i) => {
            let mut j = i + 1;
            while j < lines.len() {
                let is_item = lines[j].trim_start().starts_with("- ");
                if !is_item {
                    break;
                }
                if lines[j].trim() == line.trim() {
                    return;
                }
                j += 1;
            }
            lines.insert(j, line);
        }
    }
}

/// Zieht das Frontmatter einer vorhandenen Notiz nach (siehe Moduldoku); ohne gueltiges Frontmatter
/// bleibt der Text, wie er ist.
fn merge_frontmatter(text: &str, spec: &NoteSpec) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut rows = text.split('\n');
    let Some(first) = rows.next() else {
        return text.to_string();
    };
    if first.trim_end() != "---" {
        return text.to_string();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut closed = false;
    let mut consumed = first.len() + 1;
    for row in rows {
        consumed += row.len() + 1;
        if row.trim_end() == "---" {
            closed = true;
            break;
        }
        lines.push(row.trim_end_matches('\r').to_string());
    }
    if !closed {
        return text.to_string();
    }
    set_scalar(&mut lines, "updated", &yq(&spec.date));
    for (k, v) in &spec.meta {
        set_scalar(&mut lines, k, &v.0);
    }
    if let Some(line) = source_line(spec) {
        add_list_item(&mut lines, "quellen", &yq(&line));
    }
    let rest = text.get(consumed..).unwrap_or("");
    format!("---{nl}{}{nl}---{nl}{rest}", lines.join(nl))
}

/// Die vorhandene Notiz auf den neuen Stand bringen.
pub fn merge_existing(existing: &str, spec: &NoteSpec) -> String {
    merge_frontmatter(&merge_body(existing, spec), spec)
}

// ---------------------------------------------------------------------------
// Planen und schreiben
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Create,
    /// Eine vorhandene Notiz wird geaendert.
    Modify,
    /// Die vorhandene Notiz enthaelt schon genau das.
    Unchanged,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Create => "create",
            Kind::Modify => "modify",
            Kind::Unchanged => "unchanged",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub kind: Kind,
    /// Pfad relativ zum Vault (bei `Create` der geplante, bei Kollision mit einem fremden Namen entsteht
    /// `Name (2).md`).
    pub rel: String,
    pub existing: Option<PathBuf>,
    pub text: String,
    pub dir: String,
    pub file_name: String,
    /// Erste 16 Hexzeichen von SHA-256 des Textes (zur Bindung der Freigabe).
    pub sha: String,
}

pub fn sha_of(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Das Ergebnis eines Schreibens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub kind: Kind,
    pub rel: String,
    pub bytes: u64,
}

pub fn file_name_for(spec: &NoteSpec) -> String {
    let base = match &spec.name {
        Some(n) => n.clone(),
        None => format!("{} {}", spec.date, spec.title.trim()),
    };
    let base = md_inline(&base, 120);
    if base.to_lowercase().ends_with(".md") {
        base
    } else {
        format!("{base}.md")
    }
}

/// Plant das Schreiben (siehe Moduldoku); aendert nichts.
pub fn plan(
    sandbox: &Sandbox,
    cfg: &ObsidianConfig,
    spec: &NoteSpec,
) -> Result<Plan, ObsidianError> {
    cfg.validate_fields()?;
    let dir = spec
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or(cfg.subfolder.trim())
        .to_string();
    if let Some(found) = obsidian::find_by_id(sandbox, &spec.key)? {
        let bytes = std::fs::read(&found).map_err(|e| ObsidianError::Io(e.to_string()))?;
        let existing = String::from_utf8_lossy(&bytes).into_owned();
        let rel = sandbox.rel_of(&found.canonicalize().unwrap_or_else(|_| found.clone()));
        let merged = merge_existing(&existing, spec);
        let kind = if merged == existing {
            Kind::Unchanged
        } else {
            Kind::Modify
        };
        let file_name = found
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        return Ok(Plan {
            kind,
            rel,
            existing: Some(found),
            sha: sha_of(&merged),
            text: merged,
            dir,
            file_name,
        });
    }
    let text = render_new(cfg, spec);
    let file_name = file_name_for(spec);
    let rel = if dir.is_empty() {
        file_name.clone()
    } else {
        format!("{}/{}", dir.trim_matches('/'), file_name)
    };
    Ok(Plan {
        kind: Kind::Create,
        rel,
        existing: None,
        sha: sha_of(&text),
        text,
        dir,
        file_name,
    })
}

/// Schreibt, was `plan` ergab. `Unchanged` schreibt nichts.
pub fn apply(sandbox: &Sandbox, plan: &Plan) -> Result<Written, FolderError> {
    match (plan.kind, &plan.existing) {
        (Kind::Unchanged, _) => Ok(Written {
            kind: Kind::Unchanged,
            rel: plan.rel.clone(),
            bytes: plan.text.len() as u64,
        }),
        (Kind::Modify, Some(path)) => {
            let placed = sandbox.replace_bytes(path, plan.text.as_bytes())?;
            Ok(Written {
                kind: Kind::Modify,
                rel: placed.rel,
                bytes: placed.bytes,
            })
        }
        _ => {
            let placed =
                sandbox.write_bytes_new(&plan.dir, &plan.file_name, plan.text.as_bytes())?;
            Ok(Written {
                kind: Kind::Create,
                rel: placed.rel,
                bytes: placed.bytes,
            })
        }
    }
}

#[cfg(test)]
mod tests;
