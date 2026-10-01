//! Obsidian-Vault (A6, Goal „Integrationen“): eine Besprechung als Notiz mit
//! AI-OS-Frontmatter in den Vault schreiben. Der AI-OS-Index uebernimmt die Datei
//! beim naechsten Rescan (`Vault-README`: Markdown mit gueltigem Frontmatter);
//! es gibt keinen Schreib-Endpunkt und kein Obsidian-Plugin.
//!
//! Frontmatter-Vertrag (Vault-README, `90_templates/note.md`): `title`, `tags`,
//! `context_area`, `data_class`, `sensitivity`, `tier`; fehlt `data_class`, gilt
//! `restricted` (fail-closed). Dazu die Felder dieser App:
//! - `lva_id`: stabile Kennung der Notiz (`meeting-<besprechungs-id>`). Damit gibt es
//!   **nie eine Dublette**: vor dem Anlegen sucht `find_by_id` im ganzen Vault nach
//!   dieser Kennung (auch, wenn die Notiz inzwischen aus `00_inbox/` in einen
//!   Kontextordner verschoben wurde). Gefunden -> aktualisieren, sonst anlegen.
//! - `lva_meeting_id` und `quellen`: der Rueckverweis auf die Besprechung.
//! - Datenklasse (E7): Besprechungen `confidential`, YouTube-Videos `internal`.
//!
//! Aktualisieren ueberschreibt keine Handarbeit: Der von der App geschriebene Teil
//! liegt zwischen `<!-- lva:begin -->` und `<!-- lva:end -->`; nur er wird ersetzt
//! (und `updated` im Frontmatter). Alles ausserhalb (Ergaenzungen in Obsidian, ein
//! geaendertes `context_area` oder `tier` nach der Triage) bleibt stehen. Fehlen die
//! Marken, wird der Block am Ende angehaengt.
//!
//! Sicherheitsannahmen: Schreiben nur ueber die Pfad-Sandbox (`folder::Sandbox`) in
//! der Vault-Wurzel; die Suche folgt keinen Verknuepfungen/Junctions und ueberspringt
//! `.obsidian`, `.git`, `.trash`; begrenzt auf `MAX_SCAN_FILES` Dateien, `MAX_SCAN_DEPTH`
//! Ebenen und die ersten 4 KiB je Datei. Ist der Vault groesser und die Notiz nicht
//! gefunden, wird NICHT angelegt (Fehler statt Dublette).

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::folder::{FolderError, Sandbox};

pub const MAX_SCAN_FILES: usize = 50_000;
pub const MAX_SCAN_DEPTH: usize = 12;
const HEAD_BYTES: u64 = 4096;
pub const DEFAULT_SUBFOLDER: &str = "00_inbox";
pub const DEFAULT_AREA: &str = "beruf";
pub const DEFAULT_TIER: &str = "propose";
pub const BEGIN_MARK: &str = "<!-- lva:begin -->";
pub const END_MARK: &str = "<!-- lva:end -->";

/// Die zehn Kontextbereiche des AI-OS (`shared/enums.py`).
pub const CONTEXT_AREAS: [&str; 10] = [
    "privat",
    "familie",
    "beruf",
    "wai",
    "schule_uni",
    "finanzen",
    "gesundheit",
    "behoerden",
    "projekte",
    "kunden",
];
/// Schreib-Autonomie (Vault-Vorlage).
pub const TIERS: [&str; 4] = ["auto", "logged", "propose", "untouchable"];

/// Einstellungen eines Vaults (`config_json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObsidianConfig {
    /// Wurzel des Vaults (absoluter Pfad).
    pub path: String,
    /// Unterordner fuer neue Notizen (Vorgabe `00_inbox`: Triage).
    #[serde(default = "default_subfolder")]
    pub subfolder: String,
    #[serde(default = "default_area")]
    pub context_area: String,
    #[serde(default = "default_tier")]
    pub tier: String,
}

fn default_subfolder() -> String {
    DEFAULT_SUBFOLDER.to_string()
}
fn default_area() -> String {
    DEFAULT_AREA.to_string()
}
fn default_tier() -> String {
    DEFAULT_TIER.to_string()
}

impl ObsidianConfig {
    pub fn from_config_json(json: &str) -> Result<Self, ObsidianError> {
        let cfg: ObsidianConfig = serde_json::from_str(json)
            .map_err(|_| ObsidianError::Config("Die Vault-Einstellungen sind unvollständig."))?;
        cfg.validate_fields()?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "subfolder": self.subfolder,
            "context_area": self.context_area,
            "tier": self.tier,
        })
    }

    /// Prueft die Felder ohne Dateisystem (der Pfad wird in `Sandbox::open` geprueft).
    pub fn validate_fields(&self) -> Result<(), ObsidianError> {
        if !CONTEXT_AREAS.contains(&self.context_area.as_str()) {
            return Err(ObsidianError::Config("Der Kontextbereich ist unbekannt."));
        }
        if !TIERS.contains(&self.tier.as_str()) {
            return Err(ObsidianError::Config(
                "Die Schreib-Autonomie ist unbekannt.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObsidianError {
    Config(&'static str),
    Folder(FolderError),
    /// Der Vault ist zu gross, um sicher auf eine vorhandene Notiz zu pruefen.
    ScanLimit,
    Io(String),
}

impl ObsidianError {
    pub fn code(&self) -> &'static str {
        match self {
            ObsidianError::Config(_) => "obsidian_config_invalid",
            ObsidianError::Folder(e) => match e {
                FolderError::RootMissing => "vault_path_missing",
                FolderError::RootRelative => "vault_path_relative",
                FolderError::RootNotFound => "vault_path_not_found",
                FolderError::RootNotAFolder => "vault_path_not_a_folder",
                other => other.code(),
            },
            ObsidianError::ScanLimit => "obsidian_scan_limit",
            ObsidianError::Io(_) => "obsidian_io",
        }
    }
}

impl std::fmt::Display for ObsidianError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObsidianError::Config(m) => write!(f, "{m}"),
            ObsidianError::Folder(FolderError::RootNotFound) => {
                write!(f, "Der Vault wurde nicht gefunden.")
            }
            ObsidianError::Folder(FolderError::RootNotAFolder) => {
                write!(f, "Der Pfad ist kein Ordner.")
            }
            ObsidianError::Folder(e) => write!(f, "{e}"),
            ObsidianError::ScanLimit => write!(
                f,
                "Der Vault ist zu groß, um sicher zu prüfen, ob die Notiz schon existiert. Es wurde nichts geschrieben."
            ),
            ObsidianError::Io(m) => write!(f, "Schreiben nicht möglich: {m}"),
        }
    }
}

impl std::error::Error for ObsidianError {}

impl From<FolderError> for ObsidianError {
    fn from(e: FolderError) -> Self {
        ObsidianError::Folder(e)
    }
}

/// Alles, was in die Notiz eingeht.
#[derive(Clone, Debug)]
pub struct NoteInput {
    pub meeting_id: String,
    pub title: String,
    /// Anzeigedatum der Besprechung („30.09.2026 14:30“), fuer Rueckverweis.
    pub date_label: String,
    /// Datum fuer Dateiname und `updated` („2026-09-30“).
    pub date_iso: String,
    /// `besprechung` oder `youtube`.
    pub source: String,
    /// Adresse des Videos (nur `http`/`https`).
    pub source_url: Option<String>,
    /// Markdown der Besprechung (Protokoll, KI-Notizen ...).
    pub body_md: String,
}

/// Datenklasse nach E7: Besprechungen vertraulich, oeffentliche Videos intern.
pub fn data_class_for(source: &str) -> &'static str {
    if source == "youtube" {
        "internal"
    } else {
        "confidential"
    }
}

pub fn note_id(meeting_id: &str) -> String {
    format!("meeting-{meeting_id}")
}

/// YAML-String in doppelten Anfuehrungszeichen (Zeilenumbrueche und Steuerzeichen
/// werden zu Leerzeichen, `\` und `"` maskiert).
fn yq(s: &str) -> String {
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

/// Ein Titel in einer Zeile (fuer Fliesstext): Steuerzeichen werden Leerzeichen.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn safe_url(url: &Option<String>) -> Option<&str> {
    url.as_deref().map(str::trim).filter(|u| {
        (u.starts_with("https://") || u.starts_with("http://"))
            && !u.chars().any(char::is_whitespace)
    })
}

fn frontmatter(cfg: &ObsidianConfig, n: &NoteInput) -> String {
    let tags = if n.source == "youtube" {
        "[video, youtube, local-voice-ai]"
    } else {
        "[besprechung, local-voice-ai]"
    };
    let what = if n.source == "youtube" {
        "Video"
    } else {
        "Besprechung"
    };
    let mut quelle = format!(
        "Local Voice AI – {what} „{}“ ({})",
        n.title.trim(),
        n.meeting_id
    );
    if !n.date_label.trim().is_empty() {
        quelle.push_str(&format!(", {}", n.date_label.trim()));
    }
    let mut fm = String::from("---\n");
    fm.push_str(&format!("title: {}\n", yq(n.title.trim())));
    fm.push_str(&format!("tags: {tags}\n"));
    fm.push_str(&format!("context_area: {}\n", cfg.context_area));
    fm.push_str(&format!("data_class: {}\n", data_class_for(&n.source)));
    fm.push_str("sensitivity: \"normal\"\n");
    fm.push_str(&format!("tier: {}\n", cfg.tier));
    fm.push_str("status: entwurf\n");
    fm.push_str(&format!("updated: {}\n", yq(&n.date_iso)));
    fm.push_str(&format!("lva_id: {}\n", yq(&note_id(&n.meeting_id))));
    fm.push_str(&format!("lva_meeting_id: {}\n", yq(&n.meeting_id)));
    fm.push_str(&format!(
        "lva_quelle: {}\n",
        if n.source == "youtube" {
            "youtube"
        } else {
            "besprechung"
        }
    ));
    if let Some(url) = safe_url(&n.source_url) {
        fm.push_str(&format!("quelle_url: {}\n", yq(url)));
    }
    fm.push_str("quellen:\n");
    fm.push_str(&format!("  - {}\n", yq(&quelle)));
    fm.push_str("---\n");
    fm
}

/// Der von der App verwaltete Block (mit Marken).
fn managed_block(n: &NoteInput) -> String {
    let what = if n.source == "youtube" {
        "Video"
    } else {
        "Besprechung"
    };
    let mut b = String::new();
    b.push_str(BEGIN_MARK);
    b.push('\n');
    b.push_str(n.body_md.trim_end());
    b.push_str(&format!(
        "\n\n---\n*Quelle: Local Voice AI, {what} „{}“ (Kennung `{}`).*\n",
        one_line(&n.title),
        n.meeting_id
    ));
    b.push_str(END_MARK);
    b.push('\n');
    b
}

/// Eine neue Notiz, Byte fuer Byte reproduzierbar (Golden-Test).
pub fn render_note(cfg: &ObsidianConfig, n: &NoteInput) -> String {
    format!("{}\n{}", frontmatter(cfg, n), managed_block(n))
}

/// Bringt eine vorhandene Notiz auf den neuen Stand: der verwaltete Block und
/// `updated` werden ersetzt, alles andere bleibt. `None`: nichts zu aendern.
pub fn merge_existing(existing: &str, n: &NoteInput) -> Option<String> {
    let block = managed_block(n);
    let mut text = match (existing.find(BEGIN_MARK), existing.find(END_MARK)) {
        (Some(b), Some(e)) if b < e => {
            let end = e + END_MARK.len();
            // Den Zeilenumbruch hinter der Endemarke gehoert zum Block.
            let end = if existing[end..].starts_with("\r\n") {
                end + 2
            } else if existing[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            format!("{}{}{}", &existing[..b], block, &existing[end..])
        }
        _ => {
            let mut t = existing.trim_end().to_string();
            t.push_str("\n\n");
            t.push_str(&block);
            t
        }
    };
    // `updated` im Frontmatter nachziehen.
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let (head, tail) = rest.split_at(end);
            let new_head: String = head
                .split('\n')
                .map(|l| {
                    if l.trim_end().starts_with("updated:") {
                        let cr = if l.ends_with('\r') { "\r" } else { "" };
                        format!("updated: {}{cr}", yq(&n.date_iso))
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            text = format!("---{new_head}{tail}");
        }
    }
    (text != existing).then_some(text)
}

/// Steht im Kopf der Datei (Frontmatter) die Zeile `lva_id: "<id>"`?
fn head_has_id(head: &str, id: &str) -> bool {
    let mut lines = head.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return false;
    }
    let want = format!("lva_id: {}", yq(id));
    for line in lines {
        let l = line.trim_end();
        if l == "---" {
            return false;
        }
        if l == want {
            return true;
        }
    }
    false
}

/// Sucht die Notiz mit `lva_id` im ganzen Vault (siehe Moduldoku).
pub fn find_by_id(sandbox: &Sandbox, id: &str) -> Result<Option<PathBuf>, ObsidianError> {
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((sandbox.root().to_path_buf(), 0));
    let mut seen_files = 0usize;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            // Verknuepfungen und Junctions werden nie verfolgt.
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if matches!(
                    name.as_str(),
                    ".obsidian" | ".git" | ".trash" | "node_modules"
                ) {
                    continue;
                }
                if depth < MAX_SCAN_DEPTH {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let is_md = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if !is_md {
                continue;
            }
            seen_files += 1;
            if seen_files > MAX_SCAN_FILES {
                return Err(ObsidianError::ScanLimit);
            }
            let mut head = Vec::new();
            if let Ok(f) = std::fs::File::open(&path) {
                let _ = f.take(HEAD_BYTES).read_to_end(&mut head);
            }
            if head_has_id(&String::from_utf8_lossy(&head), id) {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

/// Was `save_note` getan hat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveKind {
    Created,
    Updated,
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveResult {
    pub kind: SaveKind,
    /// Pfad relativ zum Vault, mit `/`.
    pub rel: String,
    pub bytes: u64,
}

/// Schreibt oder aktualisiert die Notiz (siehe Moduldoku).
pub fn save_note(cfg: &ObsidianConfig, n: &NoteInput) -> Result<SaveResult, ObsidianError> {
    cfg.validate_fields()?;
    if n.meeting_id.trim().is_empty() || n.title.trim().is_empty() {
        return Err(ObsidianError::Config("Die Besprechung hat keinen Titel."));
    }
    if n.meeting_id.len() > 64
        || !n
            .meeting_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ObsidianError::Config(
            "Die Kennung der Besprechung ist ungültig.",
        ));
    }
    let sandbox = Sandbox::open(&cfg.path)?;
    let id = note_id(&n.meeting_id);
    if let Some(found) = find_by_id(&sandbox, &id)? {
        let existing = std::fs::read(&found).map_err(|e| ObsidianError::Io(e.to_string()))?;
        let existing = String::from_utf8_lossy(&existing).into_owned();
        let rel = sandbox.rel_of(&found.canonicalize().unwrap_or(found.clone()));
        return match merge_existing(&existing, n) {
            None => Ok(SaveResult {
                kind: SaveKind::Unchanged,
                rel,
                bytes: existing.len() as u64,
            }),
            Some(updated) => {
                let placed = sandbox.replace_bytes(&found, updated.as_bytes())?;
                Ok(SaveResult {
                    kind: SaveKind::Updated,
                    rel: placed.rel,
                    bytes: placed.bytes,
                })
            }
        };
    }
    let content = render_note(cfg, n);
    let file_name = format!("{} {}.md", n.date_iso.trim(), n.title.trim());
    let placed = sandbox.write_bytes_new(cfg.subfolder.trim(), &file_name, content.as_bytes())?;
    Ok(SaveResult {
        kind: SaveKind::Created,
        rel: placed.rel,
        bytes: placed.bytes,
    })
}

/// Probiert den Vault aus: vorhanden, lesbar, beschreibbar (eine Probedatei, die sofort
/// wieder entfernt wird, nur im Unterordner fuer neue Notizen).
pub fn test(cfg: &ObsidianConfig) -> Result<(), ObsidianError> {
    cfg.validate_fields()?;
    let sandbox = Sandbox::open(&cfg.path)?;
    std::fs::read_dir(sandbox.root()).map_err(|e| ObsidianError::Io(e.to_string()))?;
    let probe = sandbox.write_bytes_new(cfg.subfolder.trim(), ".lva-probe.tmp", b"probe");
    match probe {
        Ok(p) => {
            let _ = std::fs::remove_file(&p.path);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Der Pfad im Vault fuer die Anzeige in Meldungen (nie der absolute Pfad).
pub fn display_rel(rel: &str) -> String {
    Path::new(rel).to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests;
