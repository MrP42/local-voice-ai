//! Korrekturlesen des Hörbuch-Transkripts (lva#208 Teil B).
//!
//! Ein Transkript aus der Spracherkennung hat Fehler in Rechtschreibung,
//! Zeichensetzung und Groß-/Kleinschreibung. Das Sprachmodell des aktiven
//! Post-Process-Providers liest den Text absatzweise Korrektur. Es darf nichts
//! umstellen, kürzen oder ergänzen: Die Absatzzahl bleibt gleich (damit die
//! Zeitachse Text ↔ Audio gültig bleibt), und jeder Absatz geht durch
//! [`validate_proofread`] (tolerantere Stufe als `refinement::validator`).
//! Fällt er durch, bleibt das Original stehen.
//!
//! Datenschutz: kein Buch-, Prompt- oder Antworttext im Log oder in
//! Fehlermeldungen; nur Zähler.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::translator::{complete_on, translate_block};

/// Ändert sich der Prompt oder die Prüfung, gilt der alte Cache nicht mehr.
pub const PROMPT_VERSION: &str = "lektorat-v2";
/// Zielgröße eines Blocks (Zeichen), nie mitten in einem Absatz getrennt.
pub const BLOCK_CHARS: usize = 1500;
/// Gleichzeitige Blöcke bei Cloud-, API- und Abo-Modellen.
pub const WORKERS_CLOUD: usize = 4;
/// Gleichzeitige Blöcke bei einem Modell auf diesem Rechner. Ein lokales
/// Modell rechnet die Anfragen ohnehin zum großen Teil nacheinander, und jede
/// weitere parallele Anfrage belegt zusätzlichen Grafikspeicher (Kontext je
/// Anfrage) — bei wenig VRAM kippt das Modell dann auf die CPU. Zwei Anfragen
/// halten die GPU ausgelastet, ohne den Speicher zu sprengen.
pub const WORKERS_LOCAL: usize = 2;
/// Fehlercode, wenn der Nutzer abgebrochen hat.
pub const CANCELLED: &str = "proofread:cancelled";
/// Fehlercode, wenn schon ein Lauf aktiv ist.
pub const BUSY: &str = "proofread:busy";

/// Abbruch des laufenden Laufs (`library_proofread_cancel`).
static CANCEL: AtomicBool = AtomicBool::new(false);
/// Es läuft höchstens ein Lauf zur Zeit.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Setzt `RUNNING` beim Verlassen zurück, auch bei Fehlern.
struct RunGuard;
impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

// ------------------------------------------------------------------ Text --

/// Absätze: durch eine Leerzeile getrennt. `join("\n\n")` ergibt wieder den
/// Ausgangstext.
pub fn split_paragraphs(text: &str) -> Vec<&str> {
    text.split("\n\n").collect()
}

/// Überschriften und Leerstücke gehen nicht an das Modell.
fn is_fixed(paragraph: &str) -> bool {
    let t = paragraph.trim_start();
    t.is_empty() || t.starts_with('#')
}

/// Ein Block: aufeinanderfolgende Absätze (Index in der Absatzliste).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub indices: Vec<usize>,
}

/// Absätze zu Blöcken von etwa `max` Zeichen. Ein Absatz wird nie geteilt (ein
/// längerer steht allein); eine Überschrift beendet den Block.
pub fn plan_blocks(paragraphs: &[&str], max: usize) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut size = 0usize;
    for (i, p) in paragraphs.iter().enumerate() {
        if is_fixed(p) {
            if !current.is_empty() {
                blocks.push(Block {
                    indices: std::mem::take(&mut current),
                });
                size = 0;
            }
            continue;
        }
        let len = p.chars().count() + 2;
        if !current.is_empty() && size + len > max {
            blocks.push(Block {
                indices: std::mem::take(&mut current),
            });
            size = 0;
        }
        current.push(i);
        size += len;
    }
    if !current.is_empty() {
        blocks.push(Block { indices: current });
    }
    blocks
}

fn language_name(code: &str) -> &'static str {
    let base = code
        .trim()
        .to_ascii_lowercase()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_string();
    match base.as_str() {
        "en" | "eng" => "English",
        "fr" | "fra" | "fre" => "French",
        "es" | "spa" => "Spanish",
        "it" | "ita" => "Italian",
        "nl" | "nld" | "dut" => "Dutch",
        "pt" | "por" => "Portuguese",
        "pl" | "pol" => "Polish",
        "sv" | "swe" => "Swedish",
        _ => "German",
    }
}

/// Sprache des Buchs: Angabe der Metadaten, sonst erkannt, sonst Deutsch.
pub fn book_language(declared: Option<&str>, text: &str) -> &'static str {
    if let Some(code) = declared.filter(|c| !c.trim().is_empty()) {
        return language_name(code);
    }
    match crate::managers::tts::lang::detect_language(text) {
        Some(code) => language_name(code),
        None => "German",
    }
}

/// Prompt für einen Block aus `count` Absätzen.
pub fn proofread_prompt(language: &str, count: usize, text: &str) -> String {
    format!(
        "You are a careful proofreader. The text below is an automatic transcript \
         of a spoken audiobook, written in {language}. Correct spelling, grammar, \
         punctuation, capitalization and typical speech-recognition errors (wrong \
         word boundaries, homophones that are wrong in context). Start every \
         sentence with a capital letter and end it with the right punctuation \
         mark. Hard rules: do NOT change the wording, the order or the content \
         beyond these corrections; add nothing, remove nothing, do not summarize, \
         translate or comment. Keep names and numbers unchanged. The text has \
         exactly {count} paragraph(s), separated by one blank line. Return \
         exactly {count} paragraph(s), separated by one blank line; never merge \
         or split paragraphs. Tokens like ⟦0⟧ stand for speaker names, sound tags \
         and page marks: copy every such token unchanged, exactly once, at the \
         matching place. Reply with ONLY the corrected text, without explanations \
         and without quotation marks around it.\n\n{text}"
    )
}

// ---------------------------------------------------------------- Prüfung --

/// Warum ein korrigierter Absatz verworfen wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    Numbers,
    Negations,
    WordCount,
    InventedContent,
    Order,
}

/// Mindestanteil der Originalwörter, die in gleicher Reihenfolge wiederkehren.
const MIN_COVERAGE: f64 = 0.85;
/// Größte normierte Levenshtein-Distanz zweier Wörter, die noch „dasselbe“ sind.
const MAX_WORD_DISTANCE: f64 = 0.34;

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase().replace('ß', "ss"))
        .collect()
}

fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Gleiches Wort oder ein ähnliches (Rechtschreibung, Homophon).
fn similar(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let longest = a.chars().count().max(b.chars().count());
    longest > 0 && distance(a, b) as f64 / longest as f64 <= MAX_WORD_DISTANCE
}

fn negation_of(word: &str) -> Option<&'static str> {
    const EXACT: [(&str, &str); 12] = [
        ("nicht", "nicht"),
        ("nie", "nie"),
        ("niemals", "niemals"),
        ("nirgends", "nirgends"),
        ("niemand", "niemand"),
        ("nichts", "nichts"),
        ("ohne", "ohne"),
        ("not", "nicht"),
        ("never", "nie"),
        ("nobody", "niemand"),
        ("nothing", "nichts"),
        ("no", "kein"),
    ];
    if let Some((_, n)) = EXACT.iter().find(|(w, _)| *w == word) {
        return Some(n);
    }
    // „garnicht“ zusammengeschrieben; „keine“, „keinen“ …
    if word.len() > 5 && word.ends_with("nicht") {
        return Some("nicht");
    }
    word.starts_with("kein").then_some("kein")
}

fn negations(tokens: &[String]) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = tokens.iter().filter_map(|t| negation_of(t)).collect();
    found.sort_unstable();
    found
}

fn digits(tokens: &[String]) -> Vec<&str> {
    let mut found: Vec<&str> = tokens
        .iter()
        .map(String::as_str)
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .collect();
    found.sort_unstable();
    found
}

const NUMBER_WORDS: [&str; 24] = [
    "null",
    "eins",
    "zwei",
    "drei",
    "vier",
    "fuenf",
    "fünf",
    "sechs",
    "sieben",
    "acht",
    "neun",
    "zehn",
    "elf",
    "zwoelf",
    "zwölf",
    "hundert",
    "tausend",
    "million",
    "millionen",
    "one",
    "two",
    "three",
    "four",
    "five",
];

fn number_words(tokens: &[String]) -> Vec<&str> {
    let mut found: Vec<&str> = tokens
        .iter()
        .map(String::as_str)
        .filter(|t| NUMBER_WORDS.contains(t))
        .collect();
    found.sort_unstable();
    found
}

/// Kleine Wörter, die ein Lektorat einfügen darf (Artikel, Bindewörter).
fn is_function_word(word: &str) -> bool {
    word.chars().count() <= 2
        || matches!(
            word,
            "und"
                | "der"
                | "die"
                | "das"
                | "dass"
                | "den"
                | "dem"
                | "des"
                | "ein"
                | "eine"
                | "ist"
                | "war"
                | "zu"
                | "zum"
                | "zur"
                | "mit"
                | "von"
                | "the"
                | "and"
                | "was"
        )
}

/// Ausrichtung der Wörter: 1:1 ähnlich, ein Originalwort ↔ zwei Wörter (und
/// umgekehrt), wenn die Buchstaben zusammengesetzt gleich oder ähnlich sind.
/// Liefert (zugeordnete Originalwörter, nicht zugeordnete Wörter des Kandidaten).
fn align(orig: &[String], cand: &[String]) -> (usize, Vec<usize>) {
    let (n, m) = (orig.len(), cand.len());
    let mut best = vec![vec![0usize; m + 2]; n + 2];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let mut v = best[i + 1][j].max(best[i][j + 1]);
            if similar(&orig[i], &cand[j]) {
                v = v.max(2 + best[i + 1][j + 1]);
            }
            if j + 1 < m && similar(&orig[i], &format!("{}{}", cand[j], cand[j + 1])) {
                v = v.max(3 + best[i + 1][j + 2]);
            }
            if i + 1 < n && similar(&format!("{}{}", orig[i], orig[i + 1]), &cand[j]) {
                v = v.max(3 + best[i + 2][j + 1]);
            }
            best[i][j] = v;
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut matched, mut free) = (0usize, Vec::new());
    while i < n && j < m {
        let here = best[i][j];
        if similar(&orig[i], &cand[j]) && here == 2 + best[i + 1][j + 1] {
            matched += 1;
            i += 1;
            j += 1;
        } else if j + 1 < m
            && similar(&orig[i], &format!("{}{}", cand[j], cand[j + 1]))
            && here == 3 + best[i + 1][j + 2]
        {
            matched += 1;
            i += 1;
            j += 2;
        } else if i + 1 < n
            && similar(&format!("{}{}", orig[i], orig[i + 1]), &cand[j])
            && here == 3 + best[i + 2][j + 1]
        {
            matched += 2;
            i += 2;
            j += 1;
        } else if here == best[i + 1][j] {
            i += 1;
        } else {
            free.push(j);
            j += 1;
        }
    }
    free.extend(j..m);
    (matched, free)
}

/// Prüfung eines korrigierten Absatzes. Erlaubt sind Zeichensetzung,
/// Groß-/Kleinschreibung, Wortgrenzen („garnicht“ ↔ „gar nicht“) und ähnliche
/// Wörter (Rechtschreibung, Homophone). Streng bleiben Zahlen, Verneinungen,
/// Wortzahl (±10 %, mindestens 2 Wörter), Reihenfolge und neue Inhaltswörter.
pub fn validate_proofread(original: &str, candidate: &str) -> Result<(), Rejection> {
    let (o, c) = (words(original), words(candidate));
    if digits(&o) != digits(&c) || number_words(&o) != number_words(&c) {
        return Err(Rejection::Numbers);
    }
    if negations(&o) != negations(&c) {
        return Err(Rejection::Negations);
    }
    let room = (o.len() / 10).max(2);
    if o.len().abs_diff(c.len()) > room {
        return Err(Rejection::WordCount);
    }
    if o.is_empty() {
        return if c.is_empty() {
            Ok(())
        } else {
            Err(Rejection::InventedContent)
        };
    }
    let (matched, free) = align(&o, &c);
    // Neu ist ein Wort nur, wenn das Original nirgends ein ähnliches hat;
    // verschobene Wörter zeigt die Reihenfolgeprüfung an.
    let is_new = |j: usize| !is_function_word(&c[j]) && !o.iter().any(|w| similar(w, &c[j]));
    if free.iter().any(|&j| is_new(j)) {
        return Err(Rejection::InventedContent);
    }
    if (matched as f64) / (o.len() as f64) < MIN_COVERAGE {
        return Err(Rejection::Order);
    }
    Ok(())
}

// ------------------------------------------------------------------ Block --

/// Ergebnis eines Blocks: korrigierte Absätze samt Zählern.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockResult {
    pub paragraphs: Vec<String>,
    /// Absätze, die das Modell geändert hat und die die Prüfung bestanden.
    pub changed: u32,
    /// Absätze, die die Prüfung (oder die Absatzzahl) verworfen hat.
    pub rejected: u32,
}

fn with_padding(original: &str, corrected: &str) -> String {
    let lead = &original[..original.len() - original.trim_start().len()];
    let trail = &original[original.trim_end().len()..];
    format!("{lead}{}{trail}", corrected.trim())
}

/// Absatz für Absatz prüfen: unverändert, übernommen oder verworfen.
fn check_paragraphs(originals: &[&str], corrected: &[&str]) -> BlockResult {
    let mut out = BlockResult::default();
    for (orig, cand) in originals.iter().zip(corrected) {
        let cand = cand.trim();
        if cand == orig.trim() {
            out.paragraphs.push((*orig).to_string());
            continue;
        }
        if cand.is_empty() || validate_proofread(orig, cand).is_err() {
            out.rejected += 1;
            out.paragraphs.push((*orig).to_string());
            continue;
        }
        out.changed += 1;
        out.paragraphs.push(with_padding(orig, cand));
    }
    out
}

/// Antwort des Modells in Absätze; überzählige Leerzeilen zählen nicht.
fn reply_paragraphs(reply: &str) -> Vec<&str> {
    reply
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// Einen Block korrigieren. Stimmt die Absatzzahl nicht, gibt es einen zweiten
/// Versuch; danach bleibt der ganze Block im Original. `call` ist der
/// Modellaufruf (Prompt hinein, Antwort heraus).
pub async fn proofread_block<F, Fut>(
    originals: &[&str],
    language: &str,
    call: &F,
) -> Result<BlockResult, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let joined = originals.join("\n\n");
    let count = originals.len();
    for attempt in 0..2 {
        let reply = translate_block(&joined, |masked| {
            call(proofread_prompt(language, count, &masked))
        })
        .await?;
        let got = reply_paragraphs(&reply);
        if got.len() == count {
            return Ok(check_paragraphs(originals, &got));
        }
        log::warn!(
            "Lektorat: Absatzzahl stimmt nicht ({} statt {count}), Versuch {}",
            got.len(),
            attempt + 1
        );
    }
    Ok(BlockResult {
        paragraphs: originals.iter().map(|p| (*p).to_string()).collect(),
        changed: 0,
        rejected: count as u32,
    })
}

// ------------------------------------------------------------------ Cache --

/// Fertige Blöcke auf Platte: eine Datei je Block, benannt nach dem Streuwert
/// aus Modell, Prompt-Version, Sprache und Blocktext. Damit setzt ein Lauf
/// nach Abbruch oder Neustart dort fort, wo er war.
pub struct BlockCache {
    pub dir: Option<PathBuf>,
}

impl BlockCache {
    pub fn key(model: &str, language: &str, block: &str) -> String {
        let mut h = Sha256::new();
        for part in [PROMPT_VERSION, model, language, block] {
            h.update(part.as_bytes());
            h.update(b"\0");
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{key}.json")))
    }

    pub fn get(&self, key: &str) -> Option<BlockResult> {
        let raw = std::fs::read_to_string(self.path(key)?).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn put(&self, key: &str, result: &BlockResult) {
        let Some(path) = self.path(key) else { return };
        let Ok(raw) = serde_json::to_vec(result) else {
            return;
        };
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
        }
        // Ein Cache-Fehler ist kein Fehler des Korrekturlesens.
        if let Err(e) = crate::commands::pages::write_atomic(&path, &raw) {
            log::warn!("Lektorat-Cache nicht beschrieben: {e}");
        }
    }
}

// -------------------------------------------------------------------- Lauf --

pub struct RunInput<'a> {
    pub text: &'a str,
    pub language: &'a str,
    pub model_key: &'a str,
    pub workers: usize,
    pub block_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    /// Absätze, die korrigiert wurden.
    pub changed: u32,
    /// Absätze, die die Prüfung im Original gelassen hat.
    pub rejected: u32,
    pub blocks: u32,
    /// Blöcke, die aus dem Cache kamen.
    pub cached: u32,
}

/// Ganzen Text korrigieren: Blöcke bilden, bis zu `workers` gleichzeitig,
/// Cache, Fortschritt (`progress(erledigt, gesamt)`) und Abbruch.
pub async fn run<F, Fut>(
    input: &RunInput<'_>,
    cache: &BlockCache,
    call: F,
    progress: &(dyn Fn(usize, usize) + Send + Sync),
    cancel: &AtomicBool,
) -> Result<Outcome, String>
where
    F: Fn(String) -> Fut + Sync,
    Fut: Future<Output = Result<String, String>> + Send,
{
    let paragraphs = split_paragraphs(input.text);
    let blocks = plan_blocks(&paragraphs, input.block_chars);
    let total = blocks.len();
    progress(0, total);

    let call = &call;
    // Besitzende Eingaben: ein Verweis als Stromelement bräuchte eine Lebensdauer,
    // die der Compiler für die Kommandofutures nicht allgemein genug findet.
    let owned: Vec<(usize, Vec<String>)> = blocks
        .iter()
        .enumerate()
        .map(|(n, b)| {
            (
                n,
                b.indices
                    .iter()
                    .map(|&i| paragraphs[i].to_string())
                    .collect(),
            )
        })
        .collect();
    let jobs = owned.into_iter().map(|(n, originals)| async move {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        let refs: Vec<&str> = originals.iter().map(String::as_str).collect();
        let key = BlockCache::key(input.model_key, input.language, &refs.join("\n\n"));
        if let Some(hit) = cache.get(&key).filter(|h| h.paragraphs.len() == refs.len()) {
            return Ok((n, hit, true));
        }
        let result = proofread_block(&refs, input.language, call).await?;
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        cache.put(&key, &result);
        Ok((n, result, false))
    });

    let mut stream = stream::iter(jobs).buffer_unordered(input.workers.max(1));
    let mut results: Vec<Option<BlockResult>> = vec![None; total];
    let (mut done, mut cached) = (0usize, 0u32);
    while let Some(item) = stream.next().await {
        let (n, result, hit) = item?;
        results[n] = Some(result);
        cached += u32::from(hit);
        done += 1;
        progress(done, total);
    }
    drop(stream);

    let mut out: Vec<String> = paragraphs.iter().map(|p| (*p).to_string()).collect();
    let (mut changed, mut rejected) = (0u32, 0u32);
    for (block, result) in blocks.iter().zip(results) {
        let Some(result) = result else { continue };
        changed += result.changed;
        rejected += result.rejected;
        for (&i, text) in block.indices.iter().zip(result.paragraphs) {
            out[i] = text;
        }
    }
    Ok(Outcome {
        text: out.join("\n\n"),
        changed,
        rejected,
        blocks: total as u32,
        cached,
    })
}

// ---------------------------------------------------------------- Kommandos --

/// Fortschritt eines Laufs (Ereignis `library-proofread-progress`).
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ProofreadProgress {
    pub book_id: String,
    pub done: u32,
    pub total: u32,
}

/// Ergebnis des Korrekturlesens. Die Oberfläche schreibt den Text in die Seite
/// und merkt sich die Zähler an der Textfassung.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct ProofreadResult {
    pub text: String,
    pub changed: u32,
    pub rejected: u32,
    pub blocks: u32,
    pub cached: u32,
    pub model: String,
    pub at_ms: f64,
}

fn lektorat_dir(book_dir: &std::path::Path) -> PathBuf {
    book_dir.join("lektorat")
}

fn original_path(book_dir: &std::path::Path) -> PathBuf {
    lektorat_dir(book_dir).join("original.md")
}

/// Bei vorübergehenden Fehlern (ausgelastet, Mengenbegrenzung) zweimal mit
/// Pause neu; die Pause bricht bei Abbruch ab.
async fn ask_with_retry(
    settings: &crate::settings::AppSettings,
    prompt: String,
) -> Result<String, String> {
    let mut attempt = 0;
    loop {
        let reply = complete_on(
            settings,
            prompt.clone(),
            false,
            crate::managers::usage::Purpose::PostProcess,
            false,
        )
        .await;
        match reply {
            Err(e) if attempt < 2 && crate::translator::is_transient_error(&e) => {
                log::warn!("Lektorat: vorübergehender Fehler, neuer Versuch");
                let pause = if attempt == 0 { 3 } else { 8 };
                for _ in 0..pause * 4 {
                    if CANCEL.load(Ordering::SeqCst) {
                        return Err(CANCELLED.to_string());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Transkript korrekturlesen. `page_text` ist der aktuelle Text der Seite;
/// `from_original`: es gibt schon eine Korrektur, ausgegangen wird dann vom
/// gesicherten Original (ein erneuter Lauf baut nicht auf der Korrektur auf).
/// Das Original liegt als `lektorat/original.md` beim Buch.
#[tauri::command]
#[specta::specta]
pub async fn library_proofread(
    app: AppHandle,
    book_id: String,
    page_text: String,
    from_original: bool,
) -> Result<ProofreadResult, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(BUSY.into());
    }
    let _guard = RunGuard;
    CANCEL.store(false, Ordering::SeqCst);

    let dir = super::book_dir(&super::library_root(&app)?, &book_id)?;
    let book = super::read_book(&dir).ok_or("proofread:book_missing")?;
    let original = original_path(&dir);
    let base = match std::fs::read_to_string(&original) {
        Ok(saved) if from_original => saved,
        _ => {
            if page_text.trim().is_empty() {
                return Err("proofread:no_text".into());
            }
            std::fs::create_dir_all(lektorat_dir(&dir)).map_err(|e| e.to_string())?;
            crate::commands::pages::write_atomic(&original, page_text.as_bytes())?;
            page_text
        }
    };

    let settings = crate::settings::get_settings(&app);
    let provider = settings.active_post_process_provider().cloned();
    let model = provider
        .as_ref()
        .and_then(|p| settings.post_process_models.get(&p.id).cloned())
        .unwrap_or_default();
    let model_key = format!(
        "{}/{model}",
        provider.as_ref().map(|p| p.id.as_str()).unwrap_or_default()
    );
    let language = book_language(book.language.as_deref(), &base);
    let workers = if crate::translator::active_model_is_local(&settings) {
        WORKERS_LOCAL
    } else {
        WORKERS_CLOUD
    };

    let emitter = app.clone();
    let id = book_id.clone();
    let progress = move |done: usize, total: usize| {
        let _ = emitter.emit(
            "library-proofread-progress",
            ProofreadProgress {
                book_id: id.clone(),
                done: done as u32,
                total: total as u32,
            },
        );
    };
    let cache = BlockCache {
        dir: Some(lektorat_dir(&dir).join("cache")),
    };
    let outcome = run(
        &RunInput {
            text: &base,
            language,
            model_key: &model_key,
            workers,
            block_chars: BLOCK_CHARS,
        },
        &cache,
        |prompt| {
            let settings = settings.clone();
            async move { ask_with_retry(&settings, prompt).await }
        },
        &progress,
        &CANCEL,
    )
    .await;
    // Ein lokales Modell erst jetzt entladen, nicht nach jedem Block.
    if let Some(p) = &provider {
        crate::llm_client::ollama_unload(&p.base_url, &model).await;
    }
    let outcome = outcome?;
    log::info!(
        "Lektorat: {} Blöcke ({} aus dem Cache), {} korrigiert, {} verworfen",
        outcome.blocks,
        outcome.cached,
        outcome.changed,
        outcome.rejected
    );
    Ok(ProofreadResult {
        text: outcome.text,
        changed: outcome.changed,
        rejected: outcome.rejected,
        blocks: outcome.blocks,
        cached: outcome.cached,
        model: model_key,
        at_ms: super::now_ms(),
    })
}

/// Laufendes Korrekturlesen abbrechen; fertige Blöcke bleiben im Cache.
#[tauri::command]
#[specta::specta]
pub fn library_proofread_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Gesicherter Text vor dem Korrekturlesen (für „Änderungen anzeigen“ und
/// „Original wiederherstellen“); `None`, wenn es keinen gibt.
#[tauri::command]
#[specta::specta]
pub fn library_proofread_original(
    app: AppHandle,
    book_id: String,
) -> Result<Option<String>, String> {
    let dir = super::book_dir(&super::library_root(&app)?, &book_id)?;
    match std::fs::read_to_string(original_path(&dir)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("proofread:original_unreadable".into()),
    }
}

/// Gesichertes Original und Zwischenstände löschen (nach „Original
/// wiederherstellen“ und wenn der Text wächst).
#[tauri::command]
#[specta::specta]
pub fn library_proofread_discard(app: AppHandle, book_id: String) -> Result<(), String> {
    let dir = super::book_dir(&super::library_root(&app)?, &book_id)?;
    match std::fs::remove_dir_all(lektorat_dir(&dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("proofread:discard_failed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lva-proofread-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Fürs Modell: die Absätze aus dem Prompt (Text hinter der Leerzeile
    /// nach der Anweisung), hier an der letzten Doppel-Leerzeile des Vorspanns.
    fn body_of(prompt: &str) -> String {
        prompt
            .split_once("without quotation marks around it.\n\n")
            .map(|(_, b)| b.to_string())
            .unwrap()
    }

    fn fix(text: &str) -> String {
        text.replace("das haus", "Das Haus")
            .replace("ist gross", "ist groß.")
    }

    #[test]
    fn blocks_never_split_a_paragraph_and_end_at_headings() {
        let long = "a".repeat(1800);
        let paras = vec![
            "# Titel",
            "eins",
            "zwei",
            long.as_str(),
            "drei",
            "# Kapitel",
            "vier",
        ];
        let blocks = plan_blocks(&paras, 20);
        let sets: Vec<Vec<usize>> = blocks.into_iter().map(|b| b.indices).collect();
        assert_eq!(sets, vec![vec![1, 2], vec![3], vec![4], vec![6]]);
        // Überschriften und Leerstücke fehlen in jedem Block.
        let all = plan_blocks(&["x", "", "y"], 1500);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn blocks_cover_a_book_of_about_1500_chars() {
        let para = "Wort ".repeat(60);
        let paras: Vec<&str> = std::iter::repeat(para.trim()).take(20).collect();
        let blocks = plan_blocks(&paras, BLOCK_CHARS);
        assert!(blocks.iter().all(|b| b.indices.len() <= 5));
        let flat: Vec<usize> = blocks.iter().flat_map(|b| b.indices.clone()).collect();
        assert_eq!(flat, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn paragraph_count_and_text_are_kept() {
        let text = "# Buch\n\nvon A\n\ndas haus ist gross\n\nzweiter absatz";
        let cache = BlockCache { dir: None };
        let out = block_on(run(
            &RunInput {
                text,
                language: "German",
                model_key: "t/m",
                workers: 4,
                block_chars: 1500,
            },
            &cache,
            |prompt| async move { Ok(fix(&body_of(&prompt))) },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(
            out.text,
            "# Buch\n\nvon A\n\nDas Haus ist groß.\n\nzweiter absatz"
        );
        assert_eq!(out.text.split("\n\n").count(), text.split("\n\n").count());
        assert_eq!((out.changed, out.rejected), (1, 0));
    }

    #[test]
    fn a_reply_with_the_wrong_paragraph_count_keeps_the_original() {
        let text = "eins zwei\n\ndrei vier";
        let calls = AtomicUsize::new(0);
        let cache = BlockCache { dir: None };
        let out = block_on(run(
            &RunInput {
                text,
                language: "German",
                model_key: "t/m",
                workers: 1,
                block_chars: 1500,
            },
            &cache,
            |prompt| {
                calls.fetch_add(1, Ordering::SeqCst);
                // Beide Absätze zu einem verschmolzen.
                async move { Ok(body_of(&prompt).replace("\n\n", " ")) }
            },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(out.text, text);
        assert_eq!((out.changed, out.rejected), (0, 2));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "ein zweiter Versuch");
    }

    #[test]
    fn the_validator_falls_back_to_the_original_per_paragraph() {
        let text = "Es kostet 12 Euro.\n\nSie kommt nicht.";
        let cache = BlockCache { dir: None };
        let out = block_on(run(
            &RunInput {
                text,
                language: "German",
                model_key: "t/m",
                workers: 1,
                block_chars: 1500,
            },
            &cache,
            // Erster Absatz: andere Zahl, zweiter: Verneinung verloren,
            // dazu ein erfundener Satz.
            |_| async { Ok("Es kostet 15 Euro.\n\nSie kommt gern und bringt Kuchen.".to_string()) },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(out.text, text);
        assert_eq!((out.changed, out.rejected), (0, 2));
    }

    #[test]
    fn only_the_failing_paragraph_falls_back() {
        let text = "das haus ist gross\n\nEs kostet 12 Euro.";
        let cache = BlockCache { dir: None };
        let out = block_on(run(
            &RunInput {
                text,
                language: "German",
                model_key: "t/m",
                workers: 1,
                block_chars: 1500,
            },
            &cache,
            |_| async { Ok("Das Haus ist groß.\n\nEs kostet 99 Euro.".to_string()) },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(out.text, "Das Haus ist groß.\n\nEs kostet 12 Euro.");
        assert_eq!((out.changed, out.rejected), (1, 1));
    }

    #[test]
    fn page_marks_and_speakers_survive() {
        let text = "{{S. 4}} <Anna> das haus ist gross";
        let cache = BlockCache { dir: None };
        let out = block_on(run(
            &RunInput {
                text,
                language: "German",
                model_key: "t/m",
                workers: 1,
                block_chars: 1500,
            },
            &cache,
            |prompt| async move { Ok(fix(&body_of(&prompt))) },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(out.text, "{{S. 4}} <Anna> Das Haus ist groß.");
    }

    #[test]
    fn cached_blocks_are_not_asked_again() {
        let dir = temp("cache");
        let text = "das haus ist gross\n\nzweiter absatz";
        let cache = BlockCache {
            dir: Some(dir.join("cache")),
        };
        let calls = AtomicUsize::new(0);
        let go = |model: &str| {
            block_on(run(
                &RunInput {
                    text,
                    language: "German",
                    model_key: model,
                    workers: 2,
                    block_chars: 1500,
                },
                &cache,
                |prompt| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async move { Ok(fix(&body_of(&prompt))) }
                },
                &|_, _| {},
                &AtomicBool::new(false),
            ))
            .unwrap()
        };
        let first = go("a/m");
        assert_eq!((calls.load(Ordering::SeqCst), first.cached), (1, 0));
        let second = go("a/m");
        assert_eq!((calls.load(Ordering::SeqCst), second.cached), (1, 1));
        assert_eq!(first.text, second.text);
        assert_eq!((second.changed, second.rejected), (1, 0));
        // Anderes Modell: anderer Schlüssel, neue Anfrage.
        go("b/m");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_resumes_from_the_cache() {
        let dir = temp("resume");
        let para = "wort ".repeat(200);
        let text = (0..4).map(|_| para.trim()).collect::<Vec<_>>().join("\n\n");
        // Jeder Absatz hat eigene Endung, damit es vier verschiedene Blöcke sind.
        let text: String = text
            .split("\n\n")
            .enumerate()
            .map(|(i, p)| format!("{p} nr{i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let cache = BlockCache {
            dir: Some(dir.join("cache")),
        };
        let cancel = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let input = RunInput {
            text: &text,
            language: "German",
            model_key: "t/m",
            workers: 1,
            block_chars: 1500,
        };
        let first = block_on(run(
            &input,
            &cache,
            |prompt| {
                // Nach dem zweiten Block abbrechen.
                if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                    cancel.store(true, Ordering::SeqCst);
                }
                async move { Ok(body_of(&prompt)) }
            },
            &|_, _| {},
            &cancel,
        ));
        assert_eq!(first.unwrap_err(), CANCELLED);
        let seen = calls.load(Ordering::SeqCst);
        assert!(seen >= 1);
        cancel.store(false, Ordering::SeqCst);
        let second = block_on(run(
            &input,
            &cache,
            |prompt| {
                calls.fetch_add(1, Ordering::SeqCst);
                async move { Ok(body_of(&prompt)) }
            },
            &|_, _| {},
            &cancel,
        ))
        .unwrap();
        assert!(second.cached >= 1, "fertige Blöcke kommen aus dem Cache");
        assert_eq!(second.text, text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_counts_up_to_the_block_total() {
        let text = (0..6)
            .map(|i| format!("{} nr{i}", "wort ".repeat(120)))
            .collect::<Vec<_>>()
            .join("\n\n");
        let seen = Mutex::new(Vec::new());
        let cache = BlockCache { dir: None };
        let out = block_on(run(
            &RunInput {
                text: &text,
                language: "German",
                model_key: "t/m",
                workers: 3,
                block_chars: 1500,
            },
            &cache,
            |prompt| async move { Ok(body_of(&prompt)) },
            &|done, total| seen.lock().unwrap().push((done, total)),
            &AtomicBool::new(false),
        ))
        .unwrap();
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.first(), Some(&(0, out.blocks as usize)));
        assert_eq!(
            seen.last(),
            Some(&(out.blocks as usize, out.blocks as usize))
        );
        assert!(out.blocks > 1);
    }

    #[test]
    fn typical_asr_corrections_pass() {
        for (raw, fixed) in [
            (
                "ich habe das garnicht gewusst",
                "Ich habe das gar nicht gewusst.",
            ),
            ("wir gehen jetzt zu hause", "Wir gehen jetzt zu Hause."),
            (
                "er sah das mehr und die sonne",
                "Er sah das Meer und die Sonne.",
            ),
            ("seid dem tag war er weg", "Seit dem Tag war er weg."),
            ("ein auto fahrer kam vorbei", "Ein Autofahrer kam vorbei."),
            ("das haus ist gross", "Das Haus ist groß."),
            (
                "er sagte es sei spät sie nickte",
                "Er sagte: „Es sei spät.“ Sie nickte.",
            ),
            ("sie kauften 3 aepfel", "Sie kauften 3 Äpfel."),
        ] {
            assert_eq!(validate_proofread(raw, fixed), Ok(()), "{raw}");
        }
    }

    #[test]
    fn invented_changed_or_reordered_text_is_rejected() {
        let o = "der mann ging nach hause und aß 3 äpfel";
        assert_eq!(
            validate_proofread(o, "Der Mann ging nach Hause und aß 4 Äpfel."),
            Err(Rejection::Numbers)
        );
        assert_eq!(
            validate_proofread(o, "Der Mann ging nach Hause und aß drei Äpfel."),
            Err(Rejection::Numbers)
        );
        assert_eq!(
            validate_proofread("sie kommt nicht zur feier", "Sie kommt zur Feier."),
            Err(Rejection::Negations)
        );
        assert_eq!(
            validate_proofread("sie kommt zur feier", "Sie kommt nicht zur Feier."),
            Err(Rejection::Negations)
        );
        assert_eq!(
            validate_proofread(
                "er kam nach hause",
                "Er kam nach Hause und brachte Kuchen mit."
            ),
            Err(Rejection::WordCount)
        );
        let long = "der mann ging nach hause und aß äpfel dann schlief er ein und träumte vom meer";
        assert_eq!(
            validate_proofread(long, "Der Mann ging nach Hause und aß Äpfel. Dann schlief er ein und träumte vom Wetter."),
            Err(Rejection::InventedContent)
        );
        assert_eq!(
            validate_proofread(
                long,
                "Dann schlief er ein und träumte vom Meer. Der Mann ging nach Hause und aß Äpfel."
            ),
            Err(Rejection::Order)
        );
        assert_eq!(
            validate_proofread(long, "Der Mann ging."),
            Err(Rejection::WordCount)
        );
    }

    #[test]
    fn the_prompt_names_language_and_paragraph_count() {
        let p = proofread_prompt("German", 3, "x");
        assert!(p.contains("written in German"));
        assert!(p.contains("exactly 3 paragraph(s)"));
        assert_eq!(book_language(Some("en"), ""), "English");
        assert_eq!(book_language(Some("deu"), ""), "German");
        assert_eq!(book_language(None, ""), "German");
    }
}
