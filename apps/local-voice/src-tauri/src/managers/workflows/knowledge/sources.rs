//! Wo der Abgleich nachschaut (B6): die Wissensbasis (MCP `wissen_suchen`) und der Vault.
//!
//! Beide liefern [`Evidence`]: Titel, Pfad, ein Textausschnitt und eine Wertung. Alles darin ist FREMDER
//! Text (Notizen koennen eingefuegte Texte aus dem Netz enthalten): bereinigt, gekuerzt und im Prompt nur
//! als Daten verwendet.
//!
//! # Wissensbasis
//!
//! `integrations::wissen::search` (Streamable HTTP, Bearer-Schluessel nur im Geheimnisspeicher, keine
//! Weiterleitungen, Antwort hoechstens 2 MiB, hoechstens 20 Treffer, Zeitlimit je Anfrage). Das Recht
//! `knowledge.search` prueft die Engine am Tor; hier wird nur gesucht. Fehler: nicht erreichbar, Zeit,
//! 429/5xx -> `Transient`; Schluessel, Scope, Adresse, Werkzeug -> `Permanent`.
//!
//! # Vault (woertlich, ohne Modell)
//!
//! Die Notizen des Vaults, die der Index der Wissensbasis noch nicht kennt (auch die eben geschriebene Notiz
//! zum vorigen Video), findet eine einfache Wortsuche: EIN Durchlauf je Schritt baut aus jeder Markdown-Datei
//! die Menge ihrer Woerter (als Pruefsummen, nicht als Text), danach kostet jede Aussage nur einen Vergleich.
//! Grenzen: hoechstens [`VaultLimits::max_files`] Dateien, [`VaultLimits::max_bytes_per_file`] je Datei,
//! [`VaultLimits::max_total_bytes`] zusammen und [`VaultLimits::max_millis`] Wanduhr; ist eine erreicht,
//! meldet der Index `incomplete` (der Aufrufer sagt es im Ergebnis, es wird nie so getan, als sei alles
//! durchsucht). Es folgt keiner Verknuepfung und keinem Ordner `.obsidian`, `.git`, `.trash`,
//! `node_modules` oder mit Punkt am Anfang. Dateien, deren Frontmatter eine der Ausschlusszeilen enthaelt
//! (die Notiz zu DIESEM Video), zaehlen nicht.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::agent::extract::clean_text;
use crate::managers::integrations::folder::Sandbox;
use crate::managers::integrations::model::Integration;
use crate::managers::integrations::wissen::{self, HttpOpts, WissenConfig, WissenError};

use super::{md_inline, neutralize_markup};

/// Hoechstzahl Treffer je Quelle und Aussage.
pub const MAX_LIMIT: usize = 10;
const SNIPPET_CHARS: usize = 300;
const TITLE_CHARS: usize = 120;
const PATH_CHARS: usize = 200;
const SNIPPET_READ_BYTES: u64 = 32 * 1024;
const MAX_TOKENS_PER_FILE: usize = 600;

/// Ein Beleg aus der Wissensbasis oder dem Vault.
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    /// `wissen` oder `vault`.
    pub source: &'static str,
    pub title: String,
    /// Pfad in der Wissensbasis bzw. relativ zum Vault, mit `/`.
    pub path: String,
    pub snippet: String,
    /// 0..=1.
    pub score: f64,
}

/// Warum eine Suche nichts lieferte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// Nichts ist passiert, spaeter erneut versuchen.
    Transient(String),
    /// Wird so nicht gelingen (Schluessel, Einstellung).
    Permanent(String),
}

impl SearchError {
    pub fn message(&self) -> &str {
        match self {
            SearchError::Transient(m) | SearchError::Permanent(m) => m,
        }
    }
}

fn map_wissen(e: WissenError) -> SearchError {
    let text = e.to_string();
    match e {
        WissenError::Config(_)
        | WissenError::TokenMissing
        | WissenError::Unauthorized
        | WissenError::ScopeMissing(_)
        | WissenError::EndpointNotFound
        | WissenError::ToolMissing(_) => SearchError::Permanent(text),
        _ => SearchError::Transient(text),
    }
}

// ---------------------------------------------------------------------------
// Wissensbasis
// ---------------------------------------------------------------------------

pub struct WissenSource {
    cfg: WissenConfig,
    token: Zeroizing<String>,
    opts: HttpOpts,
}

impl WissenSource {
    /// Aus der Integration und ihrem Schluessel. Ein fehlender Schluessel oder eine unvollstaendige
    /// Einstellung ist dauerhaft.
    pub fn open(
        integration: &Integration,
        token: Option<Zeroizing<String>>,
        opts: HttpOpts,
    ) -> Result<Self, SearchError> {
        let cfg = WissenConfig::from_config_json(&integration.config_json).map_err(map_wissen)?;
        let token = token
            .filter(|t| !t.is_empty())
            .ok_or_else(|| map_wissen(WissenError::TokenMissing))?;
        Ok(Self { cfg, token, opts })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Evidence>, SearchError> {
        let limit = limit.clamp(1, MAX_LIMIT) as u32;
        let hits = wissen::search(&self.cfg, &self.token, query, Some(limit), None, &self.opts)
            .map_err(map_wissen)?;
        Ok(hits
            .into_iter()
            .map(|h| Evidence {
                source: "wissen",
                title: md_inline(&h.title, TITLE_CHARS),
                path: md_inline(&h.path, PATH_CHARS),
                snippet: md_inline(&h.snippet, SNIPPET_CHARS),
                score: h.score.clamp(0.0, 1.0),
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Woerter
// ---------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    "aber", "auch", "auf", "aus", "bei", "bin", "bis", "das", "dass", "dem", "den", "der", "des",
    "die", "dies", "diese", "diesem", "diesen", "dieser", "doch", "dort", "durch", "ein", "eine",
    "einem", "einen", "einer", "eines", "euch", "fuer", "hat", "hier", "ich", "ihr", "ihre", "ist",
    "kann", "man", "mehr", "mit", "nach", "nicht", "noch", "nur", "oder", "ohne", "sein", "sich",
    "sie", "sind", "ueber", "und", "uns", "von", "vor", "war", "was", "wenn", "wer", "wie", "wir",
    "wird", "wurde", "zum", "zur", "the", "and", "for", "are", "was", "with", "that", "this",
    "from", "have", "has", "not", "you", "your", "can", "will", "its", "but", "all", "any", "one",
    "our", "out", "who",
];

/// Woerter eines Textes: klein, Umlaute und ss ausgeschrieben, nur Buchstaben und Ziffern; Fuellwoerter
/// und sehr kurze Woerter bleiben weg.
pub fn tokens(text: &str) -> Vec<String> {
    let mut folded = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => folded.push_str("ae"),
            'ö' => folded.push_str("oe"),
            'ü' => folded.push_str("ue"),
            'ß' => folded.push_str("ss"),
            c if c.is_alphanumeric() => folded.push(c),
            _ => folded.push(' '),
        }
    }
    folded
        .split_whitespace()
        .filter(|w| {
            let numeric = w.chars().all(|c| c.is_ascii_digit());
            let long_enough = if numeric {
                w.chars().count() >= 2
            } else {
                w.chars().count() >= 3
            };
            long_enough && !STOPWORDS.contains(w)
        })
        .map(str::to_string)
        .collect()
}

/// FNV-1a: gleiche Eingabe, gleiche Pruefsumme, ohne weitere Abhaengigkeit.
fn hash(word: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in word.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn hashed(text: &str, cap: usize) -> Vec<u64> {
    let mut v: Vec<u64> = tokens(text).iter().map(|w| hash(w)).collect();
    v.sort_unstable();
    v.dedup();
    v.truncate(cap);
    v
}

// ---------------------------------------------------------------------------
// Vault
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct VaultLimits {
    pub max_files: usize,
    pub max_bytes_per_file: usize,
    pub max_total_bytes: usize,
    pub max_millis: u64,
    pub max_depth: usize,
}

impl Default for VaultLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_bytes_per_file: 16 * 1024,
            max_total_bytes: 48 * 1024 * 1024,
            max_millis: 20_000,
            max_depth: 12,
        }
    }
}

struct Entry {
    path: PathBuf,
    rel: String,
    title: String,
    tokens: Vec<u64>,
}

pub struct VaultIndex {
    entries: Vec<Entry>,
    /// Pfade (relativ, `/`) der Dateien, die wegen der Ausschlusszeilen nicht zaehlen.
    pub excluded: Vec<String>,
    pub files_scanned: usize,
    /// Eine Grenze wurde erreicht: der Vault ist nur teilweise durchsucht.
    pub incomplete: bool,
}

fn is_skipped_dir(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with('.') || lower == "node_modules"
}

/// Titel aus dem Frontmatter (`title: "…"`), sonst der Dateiname ohne Endung.
fn title_of(head: &str, path: &Path) -> String {
    let mut lines = head.lines();
    if lines.next().map(str::trim_end) == Some("---") {
        for line in lines {
            let l = line.trim_end();
            if l == "---" {
                break;
            }
            if let Some(rest) = l.strip_prefix("title:") {
                let t = rest.trim().trim_matches('"').trim_matches('\'').trim();
                if !t.is_empty() {
                    return md_inline(t, TITLE_CHARS);
                }
            }
        }
    }
    md_inline(
        &path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        TITLE_CHARS,
    )
}

/// Der Text ohne Frontmatter.
fn body_of(text: &str) -> &str {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return text;
    };
    if first.trim_end() != "---" {
        return text;
    }
    let mut consumed = first.len();
    for line in lines {
        consumed += line.len();
        if line.trim_end() == "---" {
            return &text[consumed..];
        }
    }
    text
}

fn has_excluded_line(head: &str, exclude: &[String]) -> bool {
    if exclude.is_empty() {
        return false;
    }
    let mut lines = head.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return false;
    }
    for line in lines.take(60) {
        let l = line.trim_end();
        if l == "---" {
            return false;
        }
        if exclude.iter().any(|e| e == l) {
            return true;
        }
    }
    false
}

impl VaultIndex {
    /// Ein Durchlauf ueber den Vault (siehe Moduldoku). `exclude`: Zeilen des Frontmatters, die eine Datei
    /// ausschliessen (`lva_id: "video-<ID>"`). `cancel` wird zwischen den Dateien gefragt.
    pub fn build(
        sandbox: &Sandbox,
        exclude: &[String],
        limits: &VaultLimits,
        cancel: &dyn Fn() -> bool,
    ) -> VaultIndex {
        let started = Instant::now();
        let budget = Duration::from_millis(limits.max_millis);
        let root = sandbox.root().to_path_buf();
        let mut index = VaultIndex {
            entries: Vec::new(),
            excluded: Vec::new(),
            files_scanned: 0,
            incomplete: false,
        };
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        queue.push_back((root, 0));
        let mut total_bytes = 0usize;
        'walk: while let Some((dir, depth)) = queue.pop_front() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut children: Vec<_> = read.flatten().collect();
            // Feste Reihenfolge: bei einer Grenze ist das Ergebnis dasselbe.
            children.sort_by_key(|e| e.file_name());
            for entry in children {
                if cancel() || started.elapsed() > budget {
                    index.incomplete = true;
                    break 'walk;
                }
                let Ok(ft) = entry.file_type() else { continue };
                // Verknuepfungen und Junctions werden nie verfolgt.
                if ft.is_symlink() {
                    continue;
                }
                let path = entry.path();
                if ft.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if is_skipped_dir(&name) {
                        continue;
                    }
                    if depth < limits.max_depth {
                        queue.push_back((path, depth + 1));
                    } else {
                        index.incomplete = true;
                    }
                    continue;
                }
                let is_md = ft.is_file()
                    && path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
                if !is_md {
                    continue;
                }
                if index.files_scanned >= limits.max_files || total_bytes >= limits.max_total_bytes
                {
                    index.incomplete = true;
                    break 'walk;
                }
                let mut buf = Vec::new();
                if let Ok(f) = std::fs::File::open(&path) {
                    let _ = f
                        .take(limits.max_bytes_per_file as u64)
                        .read_to_end(&mut buf);
                }
                total_bytes += buf.len();
                index.files_scanned += 1;
                let text = String::from_utf8_lossy(&buf).into_owned();
                let rel = sandbox.rel_of(&path.canonicalize().unwrap_or_else(|_| path.clone()));
                if has_excluded_line(&text, exclude) {
                    index.excluded.push(rel);
                    continue;
                }
                let title = title_of(&text, &path);
                let tokens = hashed(&format!("{title}\n{}", body_of(&text)), MAX_TOKENS_PER_FILE);
                if tokens.is_empty() {
                    continue;
                }
                index.entries.push(Entry {
                    path,
                    rel,
                    title,
                    tokens,
                });
            }
        }
        index
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Die besten Treffer fuer `query` (Anteil der Woerter der Anfrage, die in der Notiz vorkommen).
    pub fn search(&self, query: &str, limit: usize) -> Vec<Evidence> {
        let q_words: Vec<String> = {
            let mut w = tokens(query);
            w.sort();
            w.dedup();
            w
        };
        if q_words.is_empty() {
            return Vec::new();
        }
        let q_hashes: Vec<u64> = q_words.iter().map(|w| hash(w)).collect();
        let need = q_hashes.len().min(3);
        let mut scored: Vec<(f64, usize, &Entry)> = self
            .entries
            .iter()
            .filter_map(|e| {
                let shared = q_hashes
                    .iter()
                    .filter(|h| e.tokens.binary_search(h).is_ok())
                    .count();
                let coverage = shared as f64 / q_hashes.len() as f64;
                (shared >= need && coverage >= 0.4).then_some((coverage, shared, e))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.cmp(&a.1))
                .then(a.2.rel.cmp(&b.2.rel))
        });
        scored
            .into_iter()
            .take(limit.clamp(1, MAX_LIMIT))
            .map(|(score, _, e)| Evidence {
                source: "vault",
                title: e.title.clone(),
                path: md_inline(&e.rel, PATH_CHARS),
                snippet: snippet_for(&e.path, &q_words),
                score,
            })
            .collect()
    }
}

/// Die Zeile der Notiz, die der Anfrage am naechsten kommt (zusammen mit der vorigen, wenn sie kurz ist).
fn snippet_for(path: &Path, q_words: &[String]) -> String {
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(SNIPPET_READ_BYTES).read_to_end(&mut buf);
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let body = body_of(&text);
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("<!--"))
        .collect();
    let mut best: Option<(usize, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        let lw = tokens(line);
        let hits = q_words.iter().filter(|q| lw.contains(q)).count();
        if hits > 0 && best.is_none_or(|(_, h)| hits > h) {
            best = Some((i, hits));
        }
    }
    let pick = best.map(|(i, _)| i).unwrap_or(0);
    let mut out = String::new();
    if pick > 0 && lines[pick].chars().count() < 80 {
        out.push_str(lines[pick - 1]);
        out.push(' ');
    }
    if let Some(l) = lines.get(pick) {
        out.push_str(l);
    }
    md_inline(&out, SNIPPET_CHARS)
}

// ---------------------------------------------------------------------------
// Beides zusammen
// ---------------------------------------------------------------------------

/// Notizen zu diesem Video zaehlen nicht als Beleg.
#[derive(Clone, Debug, Default)]
pub struct Exclude {
    pub video_id: Option<String>,
    /// Pfade (relativ, `/`) von Vault-Dateien, die ausgeschlossen wurden.
    pub rels: Vec<String>,
}

fn norm_path(p: &str) -> String {
    p.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_lowercase()
}

impl Exclude {
    fn hits(&self, e: &Evidence) -> bool {
        if let Some(id) = &self.video_id {
            if e.path.contains(id.as_str())
                || e.title.contains(id.as_str())
                || e.snippet.contains(id.as_str())
            {
                return true;
            }
        }
        let p = norm_path(&e.path);
        self.rels.iter().any(|r| {
            let r = norm_path(r);
            !r.is_empty()
                && (p == r || p.ends_with(&format!("/{r}")) || r.ends_with(&format!("/{p}")))
        })
    }
}

/// Die Quellen eines Abgleichs.
pub struct SourceSet {
    pub wissen: Option<WissenSource>,
    pub vault: Option<VaultIndex>,
    pub exclude: Exclude,
}

impl SourceSet {
    /// Sucht in allen Quellen; Treffer nach Wertung, doppelte Pfade nur einmal, hoechstens `limit`.
    /// Ein Fehler der Wissensbasis ist ein Fehler der Suche (nie ein stilles „nichts gefunden“).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Evidence>, SearchError> {
        let q = clean_text(query, wissen::MAX_QUERY_CHARS);
        let mut all: Vec<Evidence> = Vec::new();
        if let Some(w) = &self.wissen {
            all.extend(w.search(&q, limit)?);
        }
        if let Some(v) = &self.vault {
            all.extend(v.search(&q, limit));
        }
        all.retain(|e| !self.exclude.hits(e));
        all.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut seen: Vec<String> = Vec::new();
        all.retain(|e| {
            let key = norm_path(&e.path);
            if key.is_empty() {
                return true;
            }
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });
        all.truncate(limit.clamp(1, MAX_LIMIT));
        Ok(all)
    }

    pub fn names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.wissen.is_some() {
            v.push("wissen");
        }
        if self.vault.is_some() {
            v.push("vault");
        }
        v
    }

    pub fn vault_incomplete(&self) -> bool {
        self.vault.as_ref().is_some_and(|v| v.incomplete)
    }
}

/// Text fuer eine Anzeige (kein Prompt): Marken entschaerft, eine Zeile.
pub fn display(s: &str, max: usize) -> String {
    clean_text(&neutralize_markup(s), max)
}

#[cfg(test)]
mod tests;
