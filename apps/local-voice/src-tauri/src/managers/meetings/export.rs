//! Protokolle als Datei: Markdown, reiner Text oder Word (.docx).
//!
//! Warum im Backend und nicht per `writeTextFile` im Frontend: das
//! fs-Plugin prüft jeden Schreibzugriff gegen die Capability-Liste, und die
//! ist bewusst auf `$APPDATA` begrenzt. Ein Speicherziel in „Dokumente" —
//! also genau das, was der Speichern-Dialog anbietet — scheiterte deshalb
//! mit „not allowed by ACL". Den Pfad hat der Nutzer im Systemdialog selbst
//! gewählt; ihn danach noch gegen eine Positivliste zu prüfen, schützt
//! niemanden. Der Umweg über Rust erlaubt außerdem Binärformate: eine
//! .docx-Datei ist ein ZIP-Archiv und kein Text.
//!
//! M6-P6a: dazu der Export einer ganzen Besprechung (`ExportBundle`) als
//! Markdown, Text, Word, HTML, SRT/VTT und JSON sowie als formatierte
//! Zwischenablage. Word und HTML teilen sich den Block-/Span-Parser unten.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::llm_call::duration_label;
use super::notes::enhance::{enhanced_to_markdown_with_done, parse_enhanced, DOC_KIND};
use super::notes::model::{
    ActionItem, EnhancedNotes, NoteBlock, NoteBlockKind, Origin, SectionKind, STATUS_DONE,
};
use super::store::{Meeting, MeetingStore, StoredSegment};
use super::subtitle::{segments_to_srt, segments_to_vtt, speaker_label};

/// Zielformate des Protokoll-Exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Markdown, unverändert wie erzeugt.
    Markdown,
    /// Reiner Text: Auszeichnungen aufgelöst, Struktur über Leerzeilen.
    PlainText,
    /// Word-Dokument mit echten Überschriften und Fettungen.
    Docx,
    /// Eigenständige HTML-Seite mit eigenem CSS.
    Html,
    /// Untertitel (nur Transkript), Sprecher als Präfix.
    Srt,
    /// WebVTT (nur Transkript), Sprecher als Präfix.
    Vtt,
    /// `lva-meeting-export@1`.
    Json,
}

impl ExportFormat {
    /// Format aus der Dateiendung. Der Nutzer wählt im Speichern-Dialog eine
    /// Endung — die ist die Absichtserklärung, nicht ein zweites Auswahlfeld.
    /// Unbekanntes wird Markdown: Rohtext ist nie falsch, nur unschöner.
    pub fn from_path(path: &Path) -> Self {
        Self::from_extension(
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default(),
        )
        .unwrap_or(Self::Markdown)
    }

    /// Format aus einer Endung oder einem Formatnamen (`--format`); `None`
    /// bei Unbekanntem, damit die Kommandozeile es melden kann.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim().trim_start_matches('.').to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" => Some(Self::PlainText),
            "docx" | "doc" => Some(Self::Docx),
            "html" | "htm" => Some(Self::Html),
            "srt" => Some(Self::Srt),
            "vtt" => Some(Self::Vtt),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Protokoll schreiben. Format ergibt sich aus der Endung von `path`.
/// SRT/VTT/JSON brauchen eine Besprechung, kein Markdown: dafür ist
/// `write_export` da.
pub fn write_document(path: &Path, markdown: &str) -> Result<(), String> {
    let write = |text: &str| {
        std::fs::write(path, text.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    };
    match ExportFormat::from_path(path) {
        ExportFormat::Markdown => write(markdown),
        ExportFormat::PlainText => write(&markdown_to_text(markdown)),
        ExportFormat::Docx => write_docx(path, markdown),
        ExportFormat::Html => {
            let title = first_heading(markdown).unwrap_or_else(|| "Protokoll".to_string());
            write(&html_page(&title, &markdown_to_html(markdown)))
        }
        ExportFormat::Srt | ExportFormat::Vtt | ExportFormat::Json => Err(format!(
            "{} kann nur aus einer Besprechung exportiert werden",
            path.display()
        )),
    }
}

// ---------------------------------------------------------------- Markdown --

/// Eine Zeile des Protokolls, so weit ausgewertet, wie beide Zielformate es
/// brauchen. Mehr Markdown als das erzeugt `minutes.rs` nicht.
#[derive(Debug, PartialEq)]
enum Block {
    Heading(u8, String),
    Bullet(String),
    Paragraph(String),
    Blank,
}

fn parse_blocks(markdown: &str) -> Vec<Block> {
    markdown
        .lines()
        .map(|raw| {
            let line = raw.trim_end();
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                return Block::Blank;
            }
            if let Some(rest) = trimmed.strip_prefix("### ") {
                return Block::Heading(3, rest.trim().to_string());
            }
            if let Some(rest) = trimmed.strip_prefix("## ") {
                return Block::Heading(2, rest.trim().to_string());
            }
            if let Some(rest) = trimmed.strip_prefix("# ") {
                return Block::Heading(1, rest.trim().to_string());
            }
            if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                return Block::Bullet(rest.trim().to_string());
            }
            Block::Paragraph(trimmed.to_string())
        })
        .collect()
}

/// Ein Textabschnitt mit seiner Auszeichnung.
#[derive(Debug, PartialEq, Clone)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
}

/// `**fett**` und `*kursiv*` / `_kursiv_` in Abschnitte zerlegen.
///
/// Bewusst ein einfacher Zustandsautomat statt einer Markdown-Bibliothek:
/// die Quelle ist unser eigener Protokoll-Generator, nicht beliebiges
/// Markdown aus dem Netz. Unpaarige Zeichen bleiben stehen, statt den Rest
/// der Zeile zu verschlucken.
fn parse_spans(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut i = 0;

    let flush = |current: &mut String, bold: bool, italic: bool, spans: &mut Vec<Span>| {
        if !current.is_empty() {
            spans.push(Span {
                text: std::mem::take(current),
                bold,
                italic,
            });
        }
    };

    while i < chars.len() {
        // `\*` und `\_` sind Zeichen, keine Auszeichnung (Rohtext wie ein
        // Transkript, das Unterstriche enthält, wird so maskiert).
        if chars[i] == '\\' && i + 1 < chars.len() && (chars[i + 1] == '*' || chars[i + 1] == '_') {
            current.push(chars[i + 1]);
            i += 2;
            continue;
        }
        let two = i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*';
        if two {
            // Öffnen nur, wenn es weiter hinten auch wieder zugeht — sonst
            // wäre ein einzelnes "**" mitten im Text eine unsichtbare
            // Fettung bis zum Zeilenende.
            let closes = bold;
            let opens = !bold
                && chars
                    .get(i + 2..)
                    .unwrap_or(&[])
                    .windows(2)
                    .any(|w| w == ['*', '*']);
            if closes || opens {
                flush(&mut current, bold, italic, &mut spans);
                bold = !bold;
                i += 2;
                continue;
            }
        }
        let one = chars[i] == '*' || chars[i] == '_';
        if one {
            flush(&mut current, bold, italic, &mut spans);
            italic = !italic;
            i += 1;
            continue;
        }
        current.push(chars[i]);
        i += 1;
    }
    flush(&mut current, bold, italic, &mut spans);
    spans
}

/// Auszeichnungen entfernen — die Textfassung soll gelesen, nicht geparst
/// werden.
fn strip_marks(line: &str) -> String {
    parse_spans(line)
        .into_iter()
        .map(|s| s.text)
        .collect::<String>()
}

/// Protokoll als reiner Text.
///
/// Überschriften bleiben als eigene Zeile mit Leerzeile davor stehen und
/// Aufzählungen behalten ihr Zeichen — ohne diese Struktur wäre die
/// Textfassung eine Wand aus Sätzen.
pub fn markdown_to_text(markdown: &str) -> String {
    let mut out = String::new();
    for block in parse_blocks(markdown) {
        match block {
            Block::Heading(_, text) => {
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(&strip_marks(&text));
                out.push('\n');
            }
            Block::Bullet(text) => {
                out.push_str("  \u{2022} ");
                out.push_str(&strip_marks(&text));
                out.push('\n');
            }
            Block::Paragraph(text) => {
                out.push_str(&strip_marks(&text));
                out.push('\n');
            }
            Block::Blank => out.push('\n'),
        }
    }
    // Windows-Zeilenenden: die Textfassung landet regelmäßig im Editor.
    out.trim_end().replace('\n', "\r\n") + "\r\n"
}

// ------------------------------------------------------------------- HTML --

/// Text für HTML maskieren (Text- und Attributkontext).
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Klartext mit Zeilenumbrüchen als HTML-Absatzinhalt.
fn html_text(text: &str) -> String {
    html_escape(text.trim_end())
        .replace("\r\n", "\n")
        .replace('\n', "<br>")
}

/// Eine Markdown-Zeile als HTML-Inhalt: dieselben Abschnitte wie beim
/// Word-Export (`parse_spans`), damit beide Formate nie auseinanderlaufen.
fn inline_html(line: &str) -> String {
    parse_spans(line)
        .into_iter()
        .map(|span| {
            let mut html = html_escape(&span.text);
            if span.italic {
                html = format!("<em>{html}</em>");
            }
            if span.bold {
                html = format!("<strong>{html}</strong>");
            }
            html
        })
        .collect()
}

/// Zellen einer Tabellenzeile `| a | b |`.
fn table_cells(row: &str) -> Vec<String> {
    let inner = row.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

/// Trennzeile `|---|:--:|`.
fn is_table_separator(row: &str) -> bool {
    let cells = table_cells(row);
    !cells.is_empty()
        && cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
}

/// Aufgabenpunkt `[ ] text` / `[x] text` → (erledigt, Text).
fn task_marker(text: &str) -> Option<(bool, &str)> {
    if let Some(rest) = text.strip_prefix("[ ] ") {
        return Some((false, rest));
    }
    text.strip_prefix("[x] ")
        .or_else(|| text.strip_prefix("[X] "))
        .map(|rest| (true, rest))
}

const BOX_OPEN: &str = "&#9744;";
const BOX_DONE: &str = "&#9745;";

/// Markdown (der Dialekt von `minutes.rs`/`enhance.rs`) als HTML-Fragment.
///
/// Auf demselben Block-/Span-Parser wie Text und Word; zusätzlich werden
/// Tabellen (Redeanteile im Protokoll) und `- [x]`-Aufgaben erkannt, die
/// Text/Word als Zeile stehen lassen.
pub fn markdown_to_html(markdown: &str) -> String {
    let blocks = parse_blocks(markdown);
    let mut out = String::new();
    let mut in_list = false;
    let mut i = 0;
    while i < blocks.len() {
        let close_list = |out: &mut String, in_list: &mut bool| {
            if *in_list {
                out.push_str("</ul>\n");
                *in_list = false;
            }
        };
        match &blocks[i] {
            Block::Heading(level, text) => {
                close_list(&mut out, &mut in_list);
                out.push_str(&format!("<h{level}>{}</h{level}>\n", inline_html(text)));
            }
            Block::Bullet(text) => {
                if !in_list {
                    out.push_str("<ul>\n");
                    in_list = true;
                }
                match task_marker(text) {
                    Some((done, rest)) => out.push_str(&format!(
                        "<li class=\"task{}\"><span class=\"box\">{}</span> {}</li>\n",
                        if done { " done" } else { "" },
                        if done { BOX_DONE } else { BOX_OPEN },
                        inline_html(rest)
                    )),
                    None => out.push_str(&format!("<li>{}</li>\n", inline_html(text))),
                }
            }
            Block::Paragraph(text) if text.starts_with('|') => {
                close_list(&mut out, &mut in_list);
                let mut rows: Vec<&str> = Vec::new();
                while let Some(Block::Paragraph(row)) = blocks.get(i) {
                    if !row.starts_with('|') {
                        break;
                    }
                    rows.push(row);
                    i += 1;
                }
                out.push_str(&table_html(&rows));
                continue;
            }
            Block::Paragraph(text) => {
                close_list(&mut out, &mut in_list);
                out.push_str(&format!("<p>{}</p>\n", inline_html(text)));
            }
            Block::Blank => close_list(&mut out, &mut in_list),
        }
        i += 1;
    }
    if in_list {
        out.push_str("</ul>\n");
    }
    out
}

fn table_html(rows: &[&str]) -> String {
    let has_header = rows.len() >= 2 && is_table_separator(rows[1]);
    let mut html = String::from("<table>\n");
    for (n, row) in rows.iter().enumerate() {
        if is_table_separator(row) {
            continue;
        }
        let tag = if has_header && n == 0 { "th" } else { "td" };
        html.push_str("<tr>");
        for cell in table_cells(row) {
            html.push_str(&format!("<{tag}>{}</{tag}>", inline_html(&cell)));
        }
        html.push_str("</tr>\n");
    }
    html.push_str("</table>\n");
    html
}

const PAGE_CSS: &str = "\
body{margin:0;background:#fff;color:#1f2933;font:16px/1.55 \"Segoe UI\",system-ui,-apple-system,sans-serif}\n\
main{max-width:760px;margin:0 auto;padding:32px 24px 64px}\n\
h1{font-size:1.7rem;margin:0 0 .2em}\n\
h2{font-size:1.25rem;margin:1.8em 0 .5em;padding-bottom:.2em;border-bottom:1px solid #e3e7eb}\n\
h3{font-size:1.05rem;margin:1.3em 0 .3em}\n\
p{margin:.4em 0}\n\
ul{margin:.3em 0;padding-left:1.4em}\n\
.meta{color:#6b7280;margin:0 0 1em}\n\
.ai{color:#6b7280}\n\
.tpl{color:#6b7280;font-style:italic}\n\
li.task{list-style:none;margin-left:-1.2em}\n\
.box{display:inline-block;width:1.2em}\n\
.done{text-decoration:line-through;color:#9aa5b1}\n\
.extra{color:#6b7280;font-size:.9em}\n\
table{border-collapse:collapse;margin:.6em 0}\n\
th,td{border:1px solid #d9dee3;padding:.3em .7em;text-align:left}\n\
.seg{margin:.3em 0}\n\
.seg .t{color:#9aa5b1;font-variant-numeric:tabular-nums}\n\
.seg .who{font-weight:600}\n\
@media print{main{max-width:none;padding:0}}\n";

/// Ein vollständiges HTML-Dokument um einen Textkörper.
fn html_page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{PAGE_CSS}</style>\n</head>\n<body>\n<main>\n{body}</main>\n</body>\n</html>\n",
        html_escape(title)
    )
}

/// Erste Überschrift eines Markdown-Textes (Titel des Dokuments).
fn first_heading(markdown: &str) -> Option<String> {
    parse_blocks(markdown).into_iter().find_map(|b| match b {
        Block::Heading(_, text) => Some(strip_marks(&text)),
        _ => None,
    })
}

// ----------------------------------------------------------------- Besprechung --

/// Welche Teile in den Export kommen (`meetings_export`, Zwischenablage).
/// Fehlende Felder gelten als „an“: ein Aufruf ohne Auswahl exportiert alles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ExportParts {
    pub ai_notes: bool,
    pub notes: bool,
    pub minutes: bool,
    pub transcript: bool,
    pub participants: bool,
}

impl Default for ExportParts {
    fn default() -> Self {
        Self::all()
    }
}

impl ExportParts {
    pub fn all() -> Self {
        Self {
            ai_notes: true,
            notes: true,
            minutes: true,
            transcript: true,
            participants: true,
        }
    }
}

/// Teilnehmende einer Besprechung. Die Tabelle dazu kommt mit dem
/// Kalenderpaket (P5d); bis dahin bleibt die Liste leer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportParticipant {
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: Option<String>,
}

impl ExportParticipant {
    fn label(&self) -> String {
        let name = self.name.as_deref().map(str::trim).unwrap_or_default();
        let email = self.email.as_deref().map(str::trim).unwrap_or_default();
        match (name.is_empty(), email.is_empty()) {
            (false, false) => format!("{name} ({email})"),
            (false, true) => name.to_string(),
            (true, false) => email.to_string(),
            (true, true) => String::new(),
        }
    }
}

/// Alles, was ein Export einer Besprechung enthalten kann. Audio gehört nie
/// dazu.
#[derive(Clone, Debug)]
pub struct ExportBundle {
    pub meeting: Meeting,
    /// Datum für die Kopfzeile („2026-09-28 14:30“), vom Aufrufer in
    /// Ortszeit gebildet; hier ein Feld, damit die Ausgabe rein bleibt.
    pub date_label: String,
    pub participants: Vec<ExportParticipant>,
    pub notes: Vec<NoteBlock>,
    /// Jüngste KI-Notizen-Version.
    pub enhanced: Option<EnhancedNotes>,
    pub minutes_md: Option<String>,
    pub segments: Vec<StoredSegment>,
    /// Aufgaben der jüngsten KI-Notizen plus manuelle.
    pub action_items: Vec<ActionItem>,
    /// Vom Nutzer vergebene Sprechernamen je `speaker_index`.
    pub speaker_names: BTreeMap<u32, String>,
}

impl ExportBundle {
    fn label(&self, segment: &StoredSegment) -> String {
        speaker_label(segment, &self.speaker_names)
    }

    fn sorted_segments(&self) -> Vec<&StoredSegment> {
        let mut list: Vec<&StoredSegment> = self.segments.iter().collect();
        list.sort_by_key(|s| (s.start_ms, s.segment_index));
        list
    }

    /// Eintrags-IDs erledigter Aufgaben (für die Häkchen der KI-Notizen).
    fn done_entries(&self) -> HashSet<String> {
        self.action_items
            .iter()
            .filter(|a| a.status == STATUS_DONE)
            .filter_map(|a| a.entry_id.clone())
            .collect()
    }

    /// Aufgaben ohne Eintrag in den KI-Notizen (von Hand angelegt).
    fn manual_tasks(&self) -> Vec<&ActionItem> {
        self.action_items
            .iter()
            .filter(|a| a.entry_id.is_none())
            .collect()
    }

    fn duration_text(&self) -> Option<String> {
        self.meeting.duration_ms.map(duration_label)
    }
}

/// Besprechung aus dem Speicher zusammenstellen. Jede Quelle, die fehlt
/// (noch keine KI-Notizen, kein Protokoll), bleibt leer statt den Export
/// scheitern zu lassen.
pub fn build_bundle(store: &MeetingStore, meeting_id: &str) -> Result<ExportBundle, String> {
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| e.to_string())?
        .filter(|m| m.deleted_at.is_none())
        .ok_or_else(|| format!("meeting_not_found: {meeting_id}"))?;
    let segments = store.get_segments(meeting_id).map_err(|e| e.to_string())?;
    let notes = store
        .get_notes(meeting_id)
        .map_err(|e| e.to_string())?
        .blocks;
    let documents = store.get_documents(meeting_id).map_err(|e| e.to_string())?;
    let latest = |kind: &str| {
        documents
            .iter()
            .filter(|d| d.kind == kind)
            .max_by_key(|d| d.version)
    };
    let enhanced = latest(DOC_KIND).and_then(|d| parse_enhanced(d).ok());
    let minutes_md = latest("minutes")
        .map(|d| d.body.clone())
        .filter(|b| !b.trim().is_empty());
    let action_items = store
        .list_action_items(meeting_id)
        .map_err(|e| e.to_string())?;
    let stamp = meeting.started_at.unwrap_or(meeting.created_at);
    let date_label = chrono::DateTime::from_timestamp(stamp, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    Ok(ExportBundle {
        meeting,
        date_label,
        participants: Vec::new(),
        notes,
        enhanced,
        minutes_md,
        segments,
        action_items,
        speaker_names: BTreeMap::new(),
    })
}

/// Markdown-Sonderzeichen des Parsers (`*`, `_`) in Rohtext maskieren, damit
/// ein Unterstrich im Transkript nicht zur Kursivschrift wird.
fn md_escape(text: &str) -> String {
    text.replace('*', "\\*").replace('_', "\\_")
}

/// Überschriften eines eingebetteten Dokuments um eine Ebene tiefer setzen;
/// die erste H1 (Dokumenttitel) entfällt, der Abschnitt bekommt eine eigene.
fn demote_headings(markdown: &str) -> String {
    let mut out = String::new();
    let mut dropped_title = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if !dropped_title && trimmed.starts_with("# ") {
            dropped_title = true;
            continue;
        }
        if trimmed.starts_with("## ") || trimmed.starts_with("# ") {
            out.push('#');
            out.push_str(trimmed);
        } else {
            out.push_str(line);
        }
        out.push_str("\n");
    }
    out
}

fn notes_markdown(blocks: &[NoteBlock]) -> String {
    let mut md = String::new();
    let mut last_was_list = false;
    for block in blocks.iter().filter(|b| !b.text.trim().is_empty()) {
        let text = md_escape(block.text.trim());
        // Ein Absatz direkt unter einem Listenpunkt wäre in Markdown dessen
        // Folgezeile; die Leerzeile trennt ihn ab.
        if block.kind == NoteBlockKind::Paragraph && last_was_list {
            md.push('\n');
        }
        last_was_list = matches!(block.kind, NoteBlockKind::Bullet | NoteBlockKind::Todo);
        match block.kind {
            NoteBlockKind::Heading => {
                md.push_str(&format!("\n### {}\n\n", text.replace('\n', " ")))
            }
            NoteBlockKind::Bullet => md.push_str(&format!("- {}\n", text.replace('\n', " "))),
            NoteBlockKind::Todo => md.push_str(&format!(
                "- [{}] {}\n",
                if block.checked { "x" } else { " " },
                text.replace('\n', " ")
            )),
            NoteBlockKind::Paragraph => {
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    md.push_str(line.trim());
                    md.push('\n');
                }
            }
        }
    }
    md
}

/// Alle gewählten Teile als ein Markdown-Text: die Quelle für `.md`, `.txt`
/// und `.docx`.
pub fn bundle_to_markdown(b: &ExportBundle, p: &ExportParts) -> String {
    let mut md = format!("# {}\n\n", b.meeting.title.trim());
    let mut head = Vec::new();
    if !b.date_label.is_empty() {
        head.push(format!("**Datum:** {}", b.date_label));
    }
    if let Some(duration) = b.duration_text() {
        head.push(format!("**Dauer:** {duration}"));
    }
    if !head.is_empty() {
        md.push_str(&head.join(" · "));
        md.push('\n');
    }

    let people: Vec<String> = b
        .participants
        .iter()
        .map(ExportParticipant::label)
        .filter(|l| !l.is_empty())
        .collect();
    if p.participants && !people.is_empty() {
        md.push_str("\n## Teilnehmende\n\n");
        for person in people {
            md.push_str(&format!("- {}\n", md_escape(&person)));
        }
    }

    if p.ai_notes {
        if let Some(enhanced) = &b.enhanced {
            md.push_str("\n## KI-Notizen\n\n");
            let notes = demote_headings(&enhanced_to_markdown_with_done(
                &b.meeting.title,
                enhanced,
                &b.done_entries(),
            ));
            md.push_str(notes.trim_start_matches('\n'));
        }
        let manual = b.manual_tasks();
        if !manual.is_empty() {
            md.push_str("\n## Weitere Aufgaben\n\n");
            for task in manual {
                let mark = if task.status == STATUS_DONE { "x" } else { " " };
                md.push_str(&format!("- [{mark}] {}\n", md_escape(task.text.trim())));
            }
        }
    }

    if p.notes {
        let notes = notes_markdown(&b.notes);
        if !notes.trim().is_empty() {
            md.push_str("\n## Meine Notizen\n\n");
            md.push_str(notes.trim_start_matches('\n'));
        }
    }

    if p.minutes {
        if let Some(minutes) = &b.minutes_md {
            md.push_str("\n## Protokoll\n\n");
            md.push_str(demote_headings(minutes).trim_start_matches('\n'));
        }
    }

    if p.transcript && b.segments.iter().any(|s| !s.text.trim().is_empty()) {
        md.push_str("\n## Transkript\n\n");
        for segment in b.sorted_segments() {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            let label = b.label(segment);
            let who = if label.is_empty() {
                String::new()
            } else {
                format!(" {}:", md_escape(&label))
            };
            md.push_str(&format!(
                // Zwei Leerzeichen am Ende = harter Umbruch in Markdown; der
                // Parser schneidet sie ab, Text und Word bleiben unberührt.
                "**[{}]{who}** {}  \n",
                duration_label(segment.start_ms),
                md_escape(&text.replace('\n', " "))
            ));
        }
    }
    md
}

/// Die Besprechung als eigenständige HTML-Seite. KI-Text steht grau, was der
/// Nutzer selbst geschrieben oder bearbeitet hat, in der Textfarbe — wie in
/// der App. Die Farbe steht zusätzlich inline, damit sie auch beim Einfügen
/// in Word/Outlook ohne das Stylesheet erhalten bleibt.
pub fn render_meeting_html(b: &ExportBundle, p: &ExportParts) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h1>{}</h1>\n",
        html_escape(b.meeting.title.trim())
    ));
    let mut meta = Vec::new();
    if !b.date_label.is_empty() {
        meta.push(html_escape(&b.date_label));
    }
    if let Some(duration) = b.duration_text() {
        meta.push(format!("Dauer {}", html_escape(&duration)));
    }
    if !meta.is_empty() {
        body.push_str(&format!("<p class=\"meta\">{}</p>\n", meta.join(" · ")));
    }

    let people: Vec<String> = b
        .participants
        .iter()
        .map(ExportParticipant::label)
        .filter(|l| !l.is_empty())
        .collect();
    if p.participants && !people.is_empty() {
        body.push_str("<section class=\"participants\">\n<h2>Teilnehmende</h2>\n<ul>\n");
        for person in people {
            body.push_str(&format!("<li>{}</li>\n", html_escape(&person)));
        }
        body.push_str("</ul>\n</section>\n");
    }

    if p.ai_notes {
        body.push_str(&ai_notes_html(b));
    }

    if p.notes {
        let blocks: Vec<&NoteBlock> = b
            .notes
            .iter()
            .filter(|n| !n.text.trim().is_empty())
            .collect();
        if !blocks.is_empty() {
            body.push_str("<section class=\"my-notes\">\n<h2>Meine Notizen</h2>\n");
            let mut in_list = false;
            for block in blocks {
                let text = html_text(&block.text);
                let item = match block.kind {
                    NoteBlockKind::Bullet => Some(format!("<li>{text}</li>\n")),
                    NoteBlockKind::Todo => Some(format!(
                        "<li class=\"task{}\"><span class=\"box\">{}</span> {text}</li>\n",
                        if block.checked { " done" } else { "" },
                        if block.checked { BOX_DONE } else { BOX_OPEN }
                    )),
                    _ => None,
                };
                match item {
                    Some(item) => {
                        if !in_list {
                            body.push_str("<ul>\n");
                            in_list = true;
                        }
                        body.push_str(&item);
                    }
                    None => {
                        if in_list {
                            body.push_str("</ul>\n");
                            in_list = false;
                        }
                        if block.kind == NoteBlockKind::Heading {
                            body.push_str(&format!("<h3>{text}</h3>\n"));
                        } else {
                            body.push_str(&format!("<p>{text}</p>\n"));
                        }
                    }
                }
            }
            if in_list {
                body.push_str("</ul>\n");
            }
            body.push_str("</section>\n");
        }
    }

    if p.minutes {
        if let Some(minutes) = &b.minutes_md {
            body.push_str("<section class=\"minutes\">\n<h2>Protokoll</h2>\n");
            body.push_str(&markdown_to_html(&demote_headings(minutes)));
            body.push_str("</section>\n");
        }
    }

    if p.transcript && b.segments.iter().any(|s| !s.text.trim().is_empty()) {
        body.push_str("<section class=\"transcript\">\n<h2>Transkript</h2>\n");
        for segment in b.sorted_segments() {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            let label = b.label(segment);
            let who = if label.is_empty() {
                String::new()
            } else {
                format!(" <span class=\"who\">{}:</span>", html_escape(&label))
            };
            body.push_str(&format!(
                "<p class=\"seg\"><span class=\"t\">[{}]</span>{who} {}</p>\n",
                duration_label(segment.start_ms),
                html_text(text)
            ));
        }
        body.push_str("</section>\n");
    }

    html_page(b.meeting.title.trim(), &body)
}

const AI_STYLE: &str = "color:#6b7280";

fn ai_notes_html(b: &ExportBundle) -> String {
    let mut html = String::new();
    if let Some(enhanced) = &b.enhanced {
        let done = b.done_entries();
        let sections: Vec<_> = enhanced
            .sections
            .iter()
            .filter(|s| !s.entries.is_empty())
            .collect();
        html.push_str("<section class=\"ai-notes\">\n<h2>KI-Notizen</h2>\n");
        if !enhanced.template_title.trim().is_empty() {
            html.push_str(&format!(
                "<p class=\"tpl\">Vorlage: {}</p>\n",
                html_escape(enhanced.template_title.trim())
            ));
        }
        if sections.is_empty() {
            html.push_str("<p class=\"tpl\">Keine Einträge.</p>\n");
        }
        for section in sections {
            html.push_str(&format!(
                "<h3>{}</h3>\n<ul>\n",
                html_escape(section.title.trim())
            ));
            for entry in &section.entries {
                let ai = entry.origin == Origin::Ai && !entry.flags.edited;
                let text = html_text(&entry.text);
                let (class, style) = if ai {
                    ("ai", format!(" style=\"{AI_STYLE}\""))
                } else {
                    ("user", String::new())
                };
                if section.kind == SectionKind::Tasks {
                    let is_done = done.contains(&entry.id);
                    let mut extras = Vec::new();
                    if let Some(a) = entry.assignee.as_deref().filter(|a| !a.trim().is_empty()) {
                        extras.push(format!("Wer: {}", html_escape(a.trim())));
                    }
                    if let Some(d) = entry.due.as_deref().filter(|d| !d.trim().is_empty()) {
                        extras.push(format!("Bis: {}", html_escape(d.trim())));
                    }
                    let extra = if extras.is_empty() {
                        String::new()
                    } else {
                        format!(" <span class=\"extra\">({})</span>", extras.join(", "))
                    };
                    html.push_str(&format!(
                        "<li class=\"task {class}{}\"{style}><span class=\"box\">{}</span> {text}{extra}</li>\n",
                        if is_done { " done" } else { "" },
                        if is_done { BOX_DONE } else { BOX_OPEN }
                    ));
                } else {
                    html.push_str(&format!("<li class=\"{class}\"{style}>{text}</li>\n"));
                }
            }
            html.push_str("</ul>\n");
        }
        html.push_str("</section>\n");
    }
    let manual = b.manual_tasks();
    if !manual.is_empty() {
        html.push_str("<section class=\"more-tasks\">\n<h2>Weitere Aufgaben</h2>\n<ul>\n");
        for task in manual {
            let done = task.status == STATUS_DONE;
            html.push_str(&format!(
                "<li class=\"task{}\"><span class=\"box\">{}</span> {}</li>\n",
                if done { " done" } else { "" },
                if done { BOX_DONE } else { BOX_OPEN },
                html_text(&task.text)
            ));
        }
        html.push_str("</ul>\n</section>\n");
    }
    html
}

/// Kennung des JSON-Formats; steigt bei jeder inkompatiblen Änderung.
pub const JSON_FORMAT: &str = "lva-meeting-export@1";

/// Die Besprechung als JSON (`lva-meeting-export@1`): alles, was die App
/// kennt, mit Segment-IDs und den Quellen der KI-Einträge. Nicht gewählte
/// Teile stehen als `null`, damit das Schema stabil bleibt. Audio nie —
/// weder Pfade noch Inhalte.
pub fn export_json(b: &ExportBundle, p: &ExportParts) -> serde_json::Value {
    use serde_json::{json, Value};
    let done = b.done_entries();
    let m = &b.meeting;

    let participants: Value = if p.participants {
        Value::Array(
            b.participants
                .iter()
                .map(|x| json!({"name": x.name, "email": x.email, "role": x.role}))
                .collect(),
        )
    } else {
        Value::Null
    };

    let ai_notes: Value = match (&b.enhanced, p.ai_notes) {
        (Some(e), true) => json!({
            "template_id": e.template_id,
            "template_title": e.template_title,
            "segment_epoch": e.segment_epoch,
            "sections": e.sections.iter().map(|s| json!({
                "id": s.id,
                "title": s.title,
                "kind": s.kind,
                "entries": s.entries.iter().map(|x| json!({
                    "id": x.id,
                    "origin": x.origin,
                    "text": x.text,
                    "assignee": x.assignee,
                    "due": x.due,
                    "source_segment_ids": x.source_segment_ids,
                    "unsupported": x.flags.unsupported,
                    "edited": x.flags.edited,
                    "done": s.kind == SectionKind::Tasks && done.contains(&x.id),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }),
        _ => Value::Null,
    };

    let my_notes: Value = if p.notes {
        Value::Array(
            b.notes
                .iter()
                .map(|n| {
                    json!({"id": n.id, "kind": n.kind, "text": n.text, "at_ms": n.at_ms, "checked": n.checked})
                })
                .collect(),
        )
    } else {
        Value::Null
    };

    let action_items: Value = if p.ai_notes || p.notes {
        Value::Array(
            b.action_items
                .iter()
                .map(|a| {
                    json!({
                        "text": a.text,
                        "status": a.status,
                        "assignee": a.assignee_label,
                        "entry_id": a.entry_id,
                        "source_segment_ids": a.source_segment_ids,
                        "source": a.source,
                    })
                })
                .collect(),
        )
    } else {
        Value::Null
    };

    let transcript: Value = if p.transcript {
        Value::Array(
            b.sorted_segments()
                .into_iter()
                .map(|s| {
                    json!({
                        "segment_id": s.segment_index,
                        "start_ms": s.start_ms,
                        "end_ms": s.end_ms,
                        "channel": s.channel,
                        "speaker_index": s.speaker_index,
                        "speaker": b.label(s),
                        "text": s.text,
                    })
                })
                .collect(),
        )
    } else {
        Value::Null
    };

    let minutes_markdown: Value = if p.minutes {
        json!(b.minutes_md)
    } else {
        Value::Null
    };

    json!({
        "format": JSON_FORMAT,
        "meeting": {
            "id": m.id,
            "title": m.title,
            "status": m.status,
            "source": m.source,
            "started_at": m.started_at,
            "ended_at": m.ended_at,
            "created_at": m.created_at,
            "duration_ms": m.duration_ms,
            "language": m.language,
        },
        "parts": {
            "ai_notes": p.ai_notes,
            "notes": p.notes,
            "minutes": p.minutes,
            "transcript": p.transcript,
            "participants": p.participants,
        },
        "participants": participants,
        "ai_notes": ai_notes,
        "my_notes": my_notes,
        "minutes_markdown": minutes_markdown,
        "action_items": action_items,
        "transcript": transcript,
    })
}

/// Die formatierte Zwischenablage: (HTML, Klartext).
pub fn clipboard_payload(b: &ExportBundle, p: &ExportParts) -> (String, String) {
    (
        render_meeting_html(b, p),
        markdown_to_text(&bundle_to_markdown(b, p)),
    )
}

/// Besprechung im gewählten Format schreiben (UTF-8 ohne BOM). SRT/VTT
/// enthalten immer nur das Transkript, unabhängig von `parts`.
pub fn write_export(
    path: &Path,
    format: ExportFormat,
    b: &ExportBundle,
    p: &ExportParts,
) -> Result<(), String> {
    let write = |text: String| {
        std::fs::write(path, text.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    };
    match format {
        ExportFormat::Markdown => write(bundle_to_markdown(b, p)),
        ExportFormat::PlainText => write(markdown_to_text(&bundle_to_markdown(b, p))),
        ExportFormat::Docx => write_docx(path, &bundle_to_markdown(b, p)),
        ExportFormat::Html => write(render_meeting_html(b, p)),
        ExportFormat::Srt => write(segments_to_srt(&b.segments, &|s| b.label(s))),
        ExportFormat::Vtt => write(segments_to_vtt(&b.segments, &|s| b.label(s))),
        ExportFormat::Json => write(
            serde_json::to_string_pretty(&export_json(b, p)).map_err(|e| e.to_string())? + "\n",
        ),
    }
}

// -------------------------------------------------------------------- DOCX --

/// XML-Sonderzeichen maskieren. Ein Protokoll enthält Namen und Zitate —
/// ein `&` oder `<` darin darf die Datei nicht unlesbar machen.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn runs_xml(line: &str) -> String {
    parse_spans(line)
        .into_iter()
        .map(|span| {
            let mut props = String::new();
            if span.bold {
                props.push_str("<w:b/>");
            }
            if span.italic {
                props.push_str("<w:i/>");
            }
            let props = if props.is_empty() {
                String::new()
            } else {
                format!("<w:rPr>{props}</w:rPr>")
            };
            format!(
                "<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                xml_escape(&span.text)
            )
        })
        .collect()
}

fn document_xml(markdown: &str) -> String {
    let mut body = String::new();
    for block in parse_blocks(markdown) {
        match block {
            Block::Heading(level, text) => {
                body.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"Heading{level}\"/></w:pPr>{}</w:p>",
                    runs_xml(&text)
                ));
            }
            Block::Bullet(text) => {
                // Aufzählungszeichen als Text statt über numbering.xml: das
                // spart einen weiteren Archivteil samt Nummerierungs-
                // definition, sieht in Word identisch aus und kann nicht
                // dadurch kaputtgehen, dass eine Listen-Id nicht aufgelöst wird.
                body.push_str(&format!(
                    "<w:p><w:pPr><w:ind w:left=\"360\" w:hanging=\"180\"/></w:pPr>\
                     <w:r><w:t xml:space=\"preserve\">\u{2022} </w:t></w:r>{}</w:p>",
                    runs_xml(&text)
                ));
            }
            Block::Paragraph(text) => {
                body.push_str(&format!("<w:p>{}</w:p>", runs_xml(&text)));
            }
            // Leerzeilen im Markdown trennen Absätze, die in Word ohnehin
            // Abstand haben — ein leerer Absatz je Leerzeile ergäbe Lücken.
            Block::Blank => {}
        }
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" w:bottom="1134" w:left="1417"/></w:sectPr></w:body></w:document>"#
    )
}

/// Formatvorlagen für Normal und Überschrift 1–3.
///
/// Ohne diesen Teil würde Word `pStyle w:val="Heading1"` ins Leere zeigen
/// lassen und alles gleich groß setzen. Die Größen sind in halben Punkt
/// angegeben (`w:sz`), so will es das Format.
const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:spacing w:after="120"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/><w:spacing w:before="240" w:after="120"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="1"/><w:spacing w:before="200" w:after="100"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="2"/><w:spacing w:before="160" w:after="80"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style>
</w:styles>"#;

const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#;

const ROOT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const DOCUMENT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

/// Protokoll als .docx schreiben — ein ZIP mit vier XML-Teilen, das ist das
/// ganze Format.
pub fn write_docx(path: &Path, markdown: &str) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let parts: [(&str, String); 4] = [
        ("[Content_Types].xml", CONTENT_TYPES_XML.to_string()),
        ("_rels/.rels", ROOT_RELS_XML.to_string()),
        (
            "word/_rels/document.xml.rels",
            DOCUMENT_RELS_XML.to_string(),
        ),
        ("word/styles.xml", STYLES_XML.to_string()),
    ];
    for (name, content) in parts {
        zip.start_file(name, options)
            .map_err(|e| format!("docx part {name} failed: {e}"))?;
        zip.write_all(content.as_bytes())
            .map_err(|e| format!("docx part {name} failed: {e}"))?;
    }
    zip.start_file("word/document.xml", options)
        .map_err(|e| format!("docx body failed: {e}"))?;
    zip.write_all(document_xml(markdown).as_bytes())
        .map_err(|e| format!("docx body failed: {e}"))?;
    zip.finish()
        .map_err(|e| format!("could not finish {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Protokoll: Test\n\n**Datum:** 2026-08-20\n\n## Aufgaben\n\n- Rückmeldung geben (*Wer: Herr Wolf*)\n- Vertrag & Frist prüfen\n";

    #[test]
    fn die_endung_bestimmt_das_format() {
        assert_eq!(
            ExportFormat::from_path(Path::new("a.docx")),
            ExportFormat::Docx
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("a.DOCX")),
            ExportFormat::Docx
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("a.txt")),
            ExportFormat::PlainText
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("a.md")),
            ExportFormat::Markdown
        );
        // Ohne Endung lieber Rohtext als ein kaputtes Word-Dokument.
        assert_eq!(
            ExportFormat::from_path(Path::new("protokoll")),
            ExportFormat::Markdown
        );
    }

    #[test]
    fn die_textfassung_traegt_keine_auszeichnungszeichen_mehr() {
        let text = markdown_to_text(SAMPLE);
        assert!(!text.contains('#'), "Rauten übrig:\n{text}");
        assert!(!text.contains('*'), "Sterne übrig:\n{text}");
        assert!(text.contains("Protokoll: Test"));
        assert!(text.contains("Datum: 2026-08-20"));
        assert!(text.contains("\u{2022} Rückmeldung geben (Wer: Herr Wolf)"));
        assert!(text.contains("\r\n"), "Windows-Zeilenenden fehlen");
    }

    #[test]
    fn fett_und_kursiv_werden_zu_eigenen_abschnitten() {
        let spans = parse_spans("**Datum:** 20.08. (*Wer: X*)");
        assert_eq!(spans[0].text, "Datum:");
        assert!(spans[0].bold);
        assert!(!spans[1].bold);
        let kursiv = spans.iter().find(|s| s.italic).expect("kursiver Abschnitt");
        assert_eq!(kursiv.text, "Wer: X");
    }

    #[test]
    fn ueberschriften_und_aufzaehlungen_werden_erkannt() {
        let blocks = parse_blocks(SAMPLE);
        assert!(blocks.contains(&Block::Heading(1, "Protokoll: Test".into())));
        assert!(blocks.contains(&Block::Heading(2, "Aufgaben".into())));
        assert!(blocks.contains(&Block::Bullet("Vertrag & Frist prüfen".into())));
    }

    #[test]
    fn xml_sonderzeichen_werden_maskiert() {
        let xml = document_xml("Vertrag & Frist <wichtig>");
        assert!(xml.contains("Vertrag &amp; Frist &lt;wichtig&gt;"), "{xml}");
    }

    /// Ein .docx ist erst dann eines, wenn die vier Pflichtteile im Archiv
    /// liegen — Word öffnet sonst gar nicht erst.
    #[test]
    fn das_word_dokument_enthaelt_alle_pflichtteile() {
        let dir = std::env::temp_dir().join(format!("lv-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("protokoll.docx");
        write_document(&path, SAMPLE).unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let mut zip = zip::ZipArchive::new(file).expect("gültiges ZIP");
        let names: Vec<String> = zip.file_names().map(|n| n.to_string()).collect();
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
            "word/document.xml",
        ] {
            assert!(names.contains(&required.to_string()), "{required} fehlt");
        }

        let mut body = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut body)
            .unwrap();
        assert!(body.contains("<w:pStyle w:val=\"Heading1\"/>"));
        assert!(body.contains("<w:b/>"), "Fettung fehlt");
        assert!(body.contains("Rückmeldung geben"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn markdown_wird_unveraendert_geschrieben() {
        let dir = std::env::temp_dir().join(format!("lv-export-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("protokoll.md");
        write_document(&path, SAMPLE).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SAMPLE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------ M6-P6a --

    use crate::managers::meetings::notes::model::{
        EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags,
    };
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};

    fn segment(
        index: u32,
        start: u64,
        end: u64,
        channel: u8,
        speaker: Option<u32>,
        text: &str,
    ) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.to_string(),
            start_ms: start,
            end_ms: end,
            channel,
            speaker_index: speaker,
            words: None,
        }
    }

    fn entry(id: &str, origin: Origin, text: &str, sources: &[u32]) -> EnhancedEntry {
        EnhancedEntry {
            id: id.to_string(),
            origin,
            text: text.to_string(),
            note_id: None,
            source_segment_ids: sources.to_vec(),
            assignee: None,
            due: None,
            flags: EntryFlags::default(),
        }
    }

    /// Feste Besprechung für die Golden-Files: nichts hängt von Uhr, Zeitzone
    /// oder Datenbank ab.
    fn nordlicht_bundle() -> ExportBundle {
        let mut edited = entry("E3", Origin::Ai, "Budget von Frau Berg freigegeben", &[3]);
        edited.flags.edited = true;
        let mut task = entry("E4", Origin::Ai, "Angebot an Nordlicht senden", &[2, 3]);
        task.assignee = Some("Anna Berg".to_string());
        task.due = Some("2026-10-02".to_string());
        let user_task = entry("E5", Origin::User, "Termin für den Workshop finden", &[]);
        let enhanced = EnhancedNotes {
            format: "enhanced@1".to_string(),
            template_id: Some("builtin:jour_fixe".to_string()),
            template_title: "Jour fixe".to_string(),
            segment_epoch: 1,
            sections: vec![
                EnhancedSection {
                    id: "summary".to_string(),
                    title: "Zusammenfassung".to_string(),
                    kind: SectionKind::Text,
                    entries: vec![
                        entry(
                            "E1",
                            Origin::Ai,
                            "Kickoff mit Nordlicht: Umfang & Zeitplan besprochen",
                            &[0, 1],
                        ),
                        entry(
                            "E2",
                            Origin::User,
                            "Größe des Pilotprojekts: 3 Standorte",
                            &[],
                        ),
                        edited,
                    ],
                },
                EnhancedSection {
                    id: "tasks".to_string(),
                    title: "Aufgaben".to_string(),
                    kind: SectionKind::Tasks,
                    entries: vec![task, user_task],
                },
                EnhancedSection {
                    id: "leer".to_string(),
                    title: "Risiken".to_string(),
                    kind: SectionKind::Text,
                    entries: vec![],
                },
            ],
            stats: EnhanceStats::default(),
        };
        let action =
            |entry_id: &str, text: &str, status: &str, assignee: Option<&str>| ActionItem {
                id: format!("A-{entry_id}"),
                meeting_id: "01NORDLICHT".to_string(),
                text: text.to_string(),
                status: status.to_string(),
                assignee_label: assignee.map(str::to_string),
                document_id: Some("D1".to_string()),
                entry_id: Some(entry_id.to_string()),
                source_segment_ids: vec![],
                source: "ai".to_string(),
            };
        let mut manual = action("", "Rechnung prüfen", "todo", None);
        manual.entry_id = None;
        manual.document_id = None;
        manual.source = "manual".to_string();
        manual.id = "A-manual".to_string();
        ExportBundle {
            meeting: Meeting {
                id: "01NORDLICHT".to_string(),
                title: "Nordlicht: Projekt-Kickoff".to_string(),
                status: "ready".to_string(),
                source: "live".to_string(),
                started_at: Some(1_790_000_000),
                ended_at: Some(1_790_003_725),
                language: Some("de".to_string()),
                mic_audio_path: Some("C:\\meetings\\01NORDLICHT\\mic.wav".to_string()),
                system_audio_path: Some("C:\\meetings\\01NORDLICHT\\system.wav".to_string()),
                duration_ms: Some(3_725_000),
                consent_confirmed_at: Some(1_790_000_000),
                audio_retention_until: None,
                source_path: None,
                created_at: 1_790_000_000,
                deleted_at: None,
            },
            date_label: "2026-09-28 14:30".to_string(),
            participants: vec![
                ExportParticipant {
                    name: Some("Anna Berg".to_string()),
                    email: Some("anna.berg@nordlicht.example".to_string()),
                    role: Some("organizer".to_string()),
                },
                ExportParticipant {
                    name: None,
                    email: Some("jonas@nordlicht.example".to_string()),
                    role: None,
                },
            ],
            notes: vec![
                NoteBlock {
                    id: "N1".to_string(),
                    kind: NoteBlockKind::Heading,
                    text: "Vorab".to_string(),
                    at_ms: None,
                    checked: false,
                },
                NoteBlock {
                    id: "N2".to_string(),
                    kind: NoteBlockKind::Bullet,
                    text: "Pilot mit <3 Standorten> prüfen".to_string(),
                    at_ms: Some(12_000),
                    checked: false,
                },
                NoteBlock {
                    id: "N3".to_string(),
                    kind: NoteBlockKind::Todo,
                    text: "Angebot senden".to_string(),
                    at_ms: Some(40_000),
                    checked: true,
                },
                NoteBlock {
                    id: "N4".to_string(),
                    kind: NoteBlockKind::Paragraph,
                    text: "Kunde mag knappe Mails.".to_string(),
                    at_ms: None,
                    checked: false,
                },
            ],
            enhanced: Some(enhanced),
            minutes_md: Some(
                "# Protokoll: Nordlicht: Projekt-Kickoff\n\n**Datum:** 2026-09-28 · **Dauer:** 1:02:05\n\n## Zusammenfassung\n\nDer Umfang des **Piloten** wurde festgelegt.\n\n## Sprecher & Redeanteile\n\n| Sprecher | Redezeit | Anteil |\n|---|---|---|\n| Ich | 10:00 | 60,0 % |\n| Gegenseite | 06:40 | 40,0 % |\n\n## Entscheidungen\n\n- Start im Oktober\n"
                    .to_string(),
            ),
            segments: vec![
                segment(1, 5_000, 9_000, 1, None, "Wir starten mit drei Standorten."),
                segment(0, 1_000, 4_000, 0, None, "Guten Morgen zusammen."),
                segment(2, 12_000, 15_500, 1, Some(1), "Das Budget & der Zeitplan stehen (snake_case_wort)."),
                segment(3, 3_723_000, 3_725_000, 2, None, "Danke, bis nächste Woche."),
                segment(4, 3_724_000, 3_724_500, 0, None, "   "),
            ],
            action_items: vec![
                action("E4", "Angebot an Nordlicht senden", "todo", Some("Anna Berg")),
                action("E5", "Termin für den Workshop finden", "done", None),
                manual,
            ],
            speaker_names: BTreeMap::from([(1, "Anna Berg".to_string())]),
        }
    }

    // -- Golden-Files ------------------------------------------------------

    fn golden_path(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("export")
            .join(name)
    }

    /// Vergleicht `actual` mit `tests/fixtures/export/<name>`; mit
    /// `LVA_UPDATE_GOLDEN=1` wird die Datei stattdessen neu geschrieben.
    /// Zeilenenden werden angeglichen (Windows-Checkout).
    fn assert_golden(name: &str, actual: &str) {
        let path = golden_path(name);
        if std::env::var("LVA_UPDATE_GOLDEN").as_deref() == Ok("1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, actual.as_bytes()).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "Golden-File {} fehlt ({e}); mit LVA_UPDATE_GOLDEN=1 erzeugen",
                path.display()
            )
        });
        assert_eq!(
            expected.replace("\r\n", "\n"),
            actual.replace("\r\n", "\n"),
            "Golden-File {name} weicht ab; bei gewollter Änderung LVA_UPDATE_GOLDEN=1"
        );
    }

    #[test]
    fn golden_nordlicht_md() {
        assert_golden(
            "nordlicht.md",
            &bundle_to_markdown(&nordlicht_bundle(), &ExportParts::all()),
        );
    }

    #[test]
    fn golden_nordlicht_html() {
        assert_golden(
            "nordlicht.html",
            &render_meeting_html(&nordlicht_bundle(), &ExportParts::all()),
        );
    }

    #[test]
    fn golden_nordlicht_srt() {
        let b = nordlicht_bundle();
        assert_golden(
            "nordlicht.srt",
            &segments_to_srt(&b.segments, &|s| b.label(s)),
        );
    }

    #[test]
    fn golden_nordlicht_vtt() {
        let b = nordlicht_bundle();
        assert_golden(
            "nordlicht.vtt",
            &segments_to_vtt(&b.segments, &|s| b.label(s)),
        );
    }

    #[test]
    fn golden_nordlicht_json() {
        let json =
            serde_json::to_string_pretty(&export_json(&nordlicht_bundle(), &ExportParts::all()))
                .unwrap();
        assert_golden("nordlicht.json", &(json + "\n"));
    }

    // -- Formate und HTML-Bausteine ---------------------------------------

    #[test]
    fn neue_endungen_bestimmen_das_format() {
        for (name, format) in [
            ("a.html", ExportFormat::Html),
            ("a.HTM", ExportFormat::Html),
            ("a.srt", ExportFormat::Srt),
            ("a.vtt", ExportFormat::Vtt),
            ("a.json", ExportFormat::Json),
        ] {
            assert_eq!(ExportFormat::from_path(Path::new(name)), format, "{name}");
        }
        assert_eq!(
            ExportFormat::from_extension("md"),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(
            ExportFormat::from_extension(".docx"),
            Some(ExportFormat::Docx)
        );
        // PDF kommt mit P6b; unbekannt heißt hier: melden, nicht raten.
        assert_eq!(ExportFormat::from_extension("pdf"), None);
    }

    #[test]
    fn markdown_wird_zu_html_mit_ueberschriften_listen_und_auszeichnung() {
        let html = markdown_to_html("# Titel\n\n**Datum:** heute (*wichtig*)\n\n## Punkte\n\n- eins & zwei\n- drei\n\nSchluss\n");
        assert!(html.contains("<h1>Titel</h1>"), "{html}");
        assert!(
            html.contains("<p><strong>Datum:</strong> heute (<em>wichtig</em>)</p>"),
            "{html}"
        );
        assert!(
            html.contains("<ul>\n<li>eins &amp; zwei</li>\n<li>drei</li>\n</ul>"),
            "{html}"
        );
        assert!(html.contains("<p>Schluss</p>"));
    }

    #[test]
    fn html_maskiert_eingebettetes_html() {
        let html = markdown_to_html("Text <script>alert(1)</script> & \"Zitat\"");
        assert!(!html.contains("<script>"), "{html}");
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;Zitat&quot;"),
            "{html}"
        );
    }

    #[test]
    fn tabellen_und_aufgabenpunkte_werden_zu_html() {
        let html = markdown_to_html(
            "| Sprecher | Anteil |\n|---|---|\n| Ich | 60,0 % |\n\n- [x] erledigt\n- [ ] offen\n",
        );
        assert!(
            html.contains("<tr><th>Sprecher</th><th>Anteil</th></tr>"),
            "{html}"
        );
        assert!(
            html.contains("<tr><td>Ich</td><td>60,0 %</td></tr>"),
            "{html}"
        );
        assert!(!html.contains("---"), "Trennzeile ausgegeben: {html}");
        assert!(
            html.contains(
                "<li class=\"task done\"><span class=\"box\">&#9745;</span> erledigt</li>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<li class=\"task\"><span class=\"box\">&#9744;</span> offen</li>"),
            "{html}"
        );
    }

    #[test]
    fn maskierte_unterstriche_bleiben_im_text_und_in_word() {
        let spans = parse_spans("snake\\_case\\_wort und *kursiv*");
        assert_eq!(spans[0].text, "snake_case_wort und ");
        assert!(spans[1].italic);
        let text = markdown_to_text("**[00:01]** snake\\_case\\_wort");
        assert!(text.contains("snake_case_wort"), "{text}");
        assert!(document_xml("a\\_b\\_c").contains("a_b_c"));
    }

    // -- Besprechung -------------------------------------------------------

    #[test]
    fn markdown_enthaelt_nur_die_gewaehlten_teile() {
        let b = nordlicht_bundle();
        let only_transcript = ExportParts {
            ai_notes: false,
            notes: false,
            minutes: false,
            transcript: true,
            participants: false,
        };
        let md = bundle_to_markdown(&b, &only_transcript);
        assert!(md.contains("## Transkript"));
        assert!(
            !md.contains("## KI-Notizen")
                && !md.contains("## Meine Notizen")
                && !md.contains("## Protokoll")
                && !md.contains("## Teilnehmende"),
            "{md}"
        );
        assert!(!md.contains("## Weitere Aufgaben"));
        let no_transcript = ExportParts {
            transcript: false,
            ..ExportParts::all()
        };
        let md = bundle_to_markdown(&b, &no_transcript);
        assert!(
            md.contains("## KI-Notizen")
                && md.contains("## Meine Notizen")
                && md.contains("## Protokoll")
                && md.contains("## Teilnehmende")
        );
        assert!(!md.contains("## Transkript"));
    }

    #[test]
    fn eingebettete_dokumente_verlieren_ihre_titelzeile() {
        let md = bundle_to_markdown(&nordlicht_bundle(), &ExportParts::all());
        assert_eq!(
            md.lines().filter(|l| l.starts_with("# ")).count(),
            1,
            "{md}"
        );
        assert!(
            md.contains("### Zusammenfassung"),
            "Abschnitte der KI-Notizen eine Ebene tiefer:\n{md}"
        );
        assert!(
            md.contains("- [x] Termin für den Workshop finden"),
            "Häkchen aus den Aufgaben:\n{md}"
        );
        assert!(
            md.contains("- [ ] Rechnung prüfen"),
            "manuelle Aufgabe fehlt:\n{md}"
        );
        assert!(!md.contains("## Risiken"), "leerer Abschnitt ausgegeben");
    }

    #[test]
    fn das_transkript_steht_in_zeitfolge_mit_sprechern_und_ohne_leere_zeilen() {
        let md = bundle_to_markdown(&nordlicht_bundle(), &ExportParts::all());
        let lines: Vec<&str> = md.lines().filter(|l| l.starts_with("**[")).collect();
        assert_eq!(lines.len(), 4, "{md}");
        assert!(lines[0].starts_with("**[00:01] Ich:** Guten Morgen"));
        assert!(lines[1].starts_with("**[00:05] Gegenseite:**"));
        assert!(lines[2].starts_with("**[00:12] Anna Berg:**"));
        assert!(
            lines[2].contains("snake\\_case\\_wort"),
            "Unterstriche müssen maskiert sein: {}",
            lines[2]
        );
        assert!(
            lines[3].starts_with("**[1:02:03]** Danke"),
            "Mischkanal ohne Präfix: {}",
            lines[3]
        );
        // Als Text (und damit Word) kommen die Unterstriche unverändert an.
        assert!(markdown_to_text(&md).contains("snake_case_wort"));
    }

    #[test]
    fn ki_text_ist_grau_nutzertext_und_bearbeitetes_nicht() {
        let html = render_meeting_html(&nordlicht_bundle(), &ExportParts::all());
        assert!(html.contains("<li class=\"ai\" style=\"color:#6b7280\">Kickoff mit Nordlicht: Umfang &amp; Zeitplan besprochen</li>"), "{html}");
        assert!(
            html.contains("<li class=\"user\">Größe des Pilotprojekts: 3 Standorte</li>"),
            "{html}"
        );
        assert!(
            html.contains("<li class=\"user\">Budget von Frau Berg freigegeben</li>"),
            "bearbeitet = Nutzertext"
        );
        assert!(html.contains(".ai{color:#6b7280}"), "eigenes CSS");
        assert!(html.starts_with("<!DOCTYPE html>") && html.contains("<meta charset=\"utf-8\">"));
    }

    #[test]
    fn aufgaben_zeigen_haken_wer_und_bis() {
        let html = render_meeting_html(&nordlicht_bundle(), &ExportParts::all());
        assert!(html.contains("<span class=\"box\">&#9744;</span> Angebot an Nordlicht senden <span class=\"extra\">(Wer: Anna Berg, Bis: 2026-10-02)</span>"), "{html}");
        assert!(html.contains("<li class=\"task user done\"><span class=\"box\">&#9745;</span> Termin für den Workshop finden</li>"), "{html}");
        assert!(html.contains("<h2>Weitere Aufgaben</h2>"));
    }

    #[test]
    fn rohtext_in_html_ist_maskiert_und_tabellen_kommen_aus_dem_protokoll() {
        let html = render_meeting_html(&nordlicht_bundle(), &ExportParts::all());
        assert!(
            html.contains("Pilot mit &lt;3 Standorten&gt; prüfen"),
            "{html}"
        );
        assert!(!html.contains("<3 Standorten>"));
        assert!(
            html.contains("<tr><th>Sprecher</th><th>Redezeit</th><th>Anteil</th></tr>"),
            "{html}"
        );
        assert!(
            html.contains("snake_case_wort"),
            "Unterstriche im HTML-Transkript unverändert"
        );
        assert!(html.contains("<span class=\"who\">Anna Berg:</span>"));
    }

    #[test]
    fn json_hat_kennung_segment_ids_quellen_und_nie_audio() {
        let json = export_json(&nordlicht_bundle(), &ExportParts::all());
        assert_eq!(json["format"], "lva-meeting-export@1");
        assert_eq!(json["transcript"][0]["segment_id"], 0);
        assert_eq!(json["transcript"][0]["speaker"], "Ich");
        assert_eq!(json["transcript"][2]["speaker"], "Anna Berg");
        assert_eq!(
            json["ai_notes"]["sections"][1]["entries"][0]["source_segment_ids"],
            serde_json::json!([2, 3])
        );
        assert_eq!(json["ai_notes"]["sections"][1]["entries"][1]["done"], true);
        assert_eq!(
            json["participants"][0]["email"],
            "anna.berg@nordlicht.example"
        );
        assert_eq!(json["action_items"].as_array().unwrap().len(), 3);
        let text = json.to_string();
        assert!(
            !text.contains("mic_audio") && !text.contains(".wav") && !text.contains("system_audio"),
            "Audio im JSON: {text}"
        );
    }

    #[test]
    fn json_fuehrt_nicht_gewaehlte_teile_als_null() {
        let p = ExportParts {
            ai_notes: false,
            notes: false,
            minutes: false,
            transcript: true,
            participants: false,
        };
        let json = export_json(&nordlicht_bundle(), &p);
        for key in [
            "participants",
            "ai_notes",
            "my_notes",
            "minutes_markdown",
            "action_items",
        ] {
            assert!(json[key].is_null(), "{key} sollte null sein");
        }
        assert_eq!(json["transcript"].as_array().unwrap().len(), 5);
        assert_eq!(json["parts"]["transcript"], true);
    }

    #[test]
    fn die_zwischenablage_liefert_html_und_lesbaren_text() {
        let (html, text) = clipboard_payload(&nordlicht_bundle(), &ExportParts::all());
        assert!(html.contains("<h1>Nordlicht: Projekt-Kickoff</h1>"));
        assert!(text.contains("Nordlicht: Projekt-Kickoff") && text.contains("Ich: Guten Morgen"));
        assert!(
            !text.contains("**") && !text.contains("## "),
            "Auszeichnungen im Klartext:\n{text}"
        );
    }

    #[test]
    fn alle_formate_werden_geschrieben() {
        let dir = tempfile::tempdir().unwrap();
        let b = nordlicht_bundle();
        for (file, needle) in [
            ("m.md", "# Nordlicht: Projekt-Kickoff"),
            ("m.txt", "Nordlicht: Projekt-Kickoff"),
            ("m.html", "<!DOCTYPE html>"),
            ("m.srt", "-->"),
            ("m.vtt", "WEBVTT"),
            ("m.json", "lva-meeting-export@1"),
        ] {
            let path = dir.path().join(file);
            write_export(
                &path,
                ExportFormat::from_path(&path),
                &b,
                &ExportParts::all(),
            )
            .unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(needle), "{file}: {text}");
            assert!(!text.starts_with('\u{feff}'), "{file} ohne BOM");
        }
        let docx = dir.path().join("m.docx");
        write_export(&docx, ExportFormat::Docx, &b, &ExportParts::all()).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&docx).unwrap()).unwrap();
        let mut body = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut body)
            .unwrap();
        assert!(body.contains("Nordlicht: Projekt-Kickoff") && body.contains("snake_case_wort"));
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("m.json")).unwrap())
                .unwrap();
        assert_eq!(parsed["format"], "lva-meeting-export@1");
    }

    #[test]
    fn write_document_kennt_html_und_verweigert_untertitel_aus_markdown() {
        let dir = tempfile::tempdir().unwrap();
        let html = dir.path().join("p.html");
        write_document(&html, SAMPLE).unwrap();
        let text = std::fs::read_to_string(&html).unwrap();
        assert!(
            text.contains("<title>Protokoll: Test</title>") && text.contains("<h2>Aufgaben</h2>"),
            "{text}"
        );
        assert!(write_document(&dir.path().join("p.srt"), SAMPLE).is_err());
        assert!(write_document(&dir.path().join("p.json"), SAMPLE).is_err());
    }

    // -- Aus dem Speicher --------------------------------------------------

    fn stored_meeting() -> (tempfile::TempDir, MeetingStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let meeting = store
            .create_meeting(
                "Aus dem Speicher",
                MeetingSource::Import,
                Some(1_790_000_000),
            )
            .unwrap();
        store
            .append_delta(
                &meeting.id,
                &TranscriptDelta {
                    new_segments: vec![
                        segment(0, 0, 2_000, 0, None, "Hallo."),
                        segment(1, 2_500, 4_000, 1, None, "Guten Tag."),
                    ],
                },
            )
            .unwrap();
        store
            .save_notes(
                &meeting.id,
                &[NoteBlock {
                    id: "N1".to_string(),
                    kind: NoteBlockKind::Bullet,
                    text: "Notiz".to_string(),
                    at_ms: None,
                    checked: false,
                }],
                0,
            )
            .unwrap();
        store
            .upsert_document(
                &meeting.id,
                "minutes",
                "markdown@1",
                "# Protokoll: X\n\n## Zusammenfassung\n\nKurz.\n",
                None,
            )
            .unwrap();
        store.set_status(&meeting.id, MeetingStatus::Ready).unwrap();
        (dir, store, meeting.id)
    }

    #[test]
    fn das_buendel_wird_aus_dem_speicher_zusammengestellt() {
        let (_dir, store, id) = stored_meeting();
        let b = build_bundle(&store, &id).unwrap();
        assert_eq!(b.meeting.title, "Aus dem Speicher");
        assert_eq!(b.segments.len(), 2);
        assert_eq!(b.notes.len(), 1);
        assert!(b.enhanced.is_none(), "noch keine KI-Notizen");
        assert!(b.minutes_md.as_deref().unwrap().contains("Kurz."));
        assert!(b.participants.is_empty());
        let md = bundle_to_markdown(&b, &ExportParts::all());
        assert!(
            md.contains("## Meine Notizen")
                && md.contains("## Protokoll")
                && !md.contains("## KI-Notizen"),
            "{md}"
        );
    }

    #[test]
    fn eine_unbekannte_oder_geloeschte_besprechung_ist_ein_klarer_fehler() {
        let (_dir, store, id) = stored_meeting();
        assert!(build_bundle(&store, "gibt-es-nicht")
            .unwrap_err()
            .starts_with("meeting_not_found"));
        store.soft_delete_meeting(&id).unwrap();
        assert!(build_bundle(&store, &id)
            .unwrap_err()
            .starts_with("meeting_not_found"));
    }
}
