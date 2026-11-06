//! Fachbegriffe im Buch erkennen (lva#184).
//!
//! Der Lauf startet auf Knopfdruck, je Buch. Codex-Entscheid vom 01.11.2026:
//! - Ein LLM-Aufruf je Kapitel liefert Begriffe als JSON.
//! - Das Ergebnis jedes Kapitels wird sofort gespeichert. Ein abgebrochener
//!   Lauf setzt deshalb beim nächsten Kapitel fort.
//! - Ein späterer Lauf bearbeitet nur neue oder geänderte Kapitel. Maßgeblich
//!   ist der Hash des gesäuberten Kapiteltexts.
//! - Korrekturen der Nutzer bleiben erhalten: Ein geänderter oder
//!   ausgeblendeter Begriff wird nie überschrieben.
//!
//! Die Erklärung hat zwei getrennte Teile:
//! - `from_book`: was das Buch selbst sagt;
//! - `general`: allgemeines Modellwissen. Es ist so gekennzeichnet, wird nie
//!   als geprüft ausgegeben und auch beim Vorlesen und im Export getrennt
//!   genannt.
//!
//! Fundstellen erfindet das Modell nicht. Sie entstehen durch Suche im Text,
//! und die Seite liefert die Seitenmarke `{{S. n}}` davor.
//!
//! Ablage: `projects/<page>/ki/begriffe.json`.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use super::source::Section;
use crate::commands::pages::write_atomic;

/// Ändert sich der Extraktions-Prompt, werden alle Kapitel neu bewertet.
const TERMS_VERSION: &str = "book-terms-v1";
/// Höchstens so viele Begriffe je Kapitel.
pub const MAX_TERMS_PER_CHAPTER: usize = 15;
const MAX_TERM_CHARS: usize = 80;
const MAX_EXPLANATION_WORDS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TermKind {
    Fachbegriff,
    Person,
    Ort,
    Ereignis,
    Konzept,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BookTerm {
    /// Normalisierter Begriff (klein, Leerraum zusammengezogen).
    pub id: String,
    pub term: String,
    pub variants: Vec<String>,
    pub kind: TermKind,
    /// Was das Buch über den Begriff sagt. Leer, wenn es nichts erklärt.
    pub from_book: String,
    /// Allgemeines Modellwissen, getrennt gekennzeichnet und nicht geprüft.
    pub general: String,
    /// Kapitel, in dem der Begriff zuerst gefunden wurde.
    pub chapter: String,
    /// Vom Nutzer geändert oder angelegt: Läufe überschreiben ihn nicht.
    pub user_edited: bool,
    /// Vom Nutzer ausgeblendet: wird nicht hervorgehoben und nicht neu angelegt.
    pub hidden: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TermsFile {
    /// Kapitel-Schlüssel → Hash des bewerteten Texts.
    pub chapters: BTreeMap<String, String>,
    pub terms: Vec<BookTerm>,
}

/// Fundstelle eines Begriffs im Seitentext (UTF-16-Offsets, Ende exklusiv).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct TermHit {
    pub term_id: String,
    pub start: u32,
    pub end: u32,
    /// Letzte Seitenmarke vor der Stelle.
    pub page: Option<u32>,
}

pub fn normalize(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn chapter_key(index: usize, section: &Section) -> String {
    format!("{index}:{}", section.title)
}

pub fn chapter_hash(section: &Section) -> String {
    let mut h = Sha256::new();
    h.update(TERMS_VERSION.as_bytes());
    h.update([0]);
    h.update(section.text.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Kapitel, die (neu) bewertet werden müssen.
pub fn pending<'a>(file: &TermsFile, sections: &'a [Section]) -> Vec<(usize, &'a Section)> {
    sections
        .iter()
        .enumerate()
        .filter(|(i, s)| file.chapters.get(&chapter_key(*i, s)) != Some(&chapter_hash(s)))
        .collect()
}

pub fn extract_prompt(label: &str, text: &str, language: &str, known: &[String]) -> String {
    let known = if known.is_empty() {
        String::new()
    } else {
        format!(
            "\nAlready known terms (do not repeat them): {}.",
            known.join(", ")
        )
    };
    format!(
        "From the following part of a book, pick at most {MAX_TERMS_PER_CHAPTER} terms a \
         reader may want explained: technical terms, important people, places, events \
         or concepts. Skip everyday words.{known}\n\
         Reply with ONLY a JSON array. Each element: {{\"term\": exact spelling as in the \
         text, \"variants\": other spellings or inflections found in the text, \"kind\": \
         one of \"fachbegriff\", \"person\", \"ort\", \"ereignis\", \"konzept\", \
         \"from_book\": what THIS text says about the term (empty string if it does not \
         explain it; never invent), \"general\": a short general explanation from your \
         own knowledge (empty if unsure)}}. Both explanations together at most \
         {MAX_EXPLANATION_WORDS} words, in the same language as the text{language}. \
         Only terms that literally occur in the text.\n\nPart: {label}\n\n{text}"
    )
}

/// Ein Element der Modellantwort, feldweise tolerant gelesen (`null`,
/// fehlende oder falsch typisierte Felder kosten nur dieses Feld).
fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn list_field(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn kind_of(raw: &str) -> TermKind {
    match raw.trim().to_lowercase().as_str() {
        "person" => TermKind::Person,
        "ort" | "place" => TermKind::Ort,
        "ereignis" | "event" => TermKind::Ereignis,
        "konzept" | "concept" => TermKind::Konzept,
        _ => TermKind::Fachbegriff,
    }
}

fn clip_words(text: &str, max: usize) -> String {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.len() <= max {
        words.join(" ")
    } else {
        format!("{} …", words[..max].join(" "))
    }
}

/// Das JSON-Array in der Antwort finden: jedes `[` als Anfang probieren,
/// bis ein gültiges Array bis zum letzten `]` entsteht. So stört Text in
/// eckigen Klammern vor dem Array nicht.
fn json_array(answer: &str) -> Option<Vec<serde_json::Value>> {
    let end = answer.rfind(']')?;
    answer[..end]
        .match_indices('[')
        .find_map(|(a, _)| serde_json::from_str::<Vec<serde_json::Value>>(&answer[a..=end]).ok())
}

/// Antwort des Modells lesen. Das Modell darf Text um das JSON-Array
/// schreiben. Begriffe, die im Kapiteltext nicht vorkommen, werden verworfen;
/// das schützt vor erfundenen Belegen. `None`: keine lesbare Antwort. Das
/// Kapitel gilt dann nicht als bewertet und kommt im nächsten Lauf wieder.
pub fn parse_answer(answer: &str, chapter: &str, chapter_text: &str) -> Option<Vec<BookTerm>> {
    let raw = json_array(answer)?;
    let lowered = Lowered::new(chapter_text);
    let mut seen = HashSet::new();
    let out = raw
        .iter()
        .filter_map(|r| {
            let term = str_field(r, "term")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let id = normalize(&term);
            if id.is_empty() || term.chars().count() > MAX_TERM_CHARS || !seen.insert(id.clone()) {
                return None;
            }
            let variants: Vec<String> = list_field(r, "variants")
                .into_iter()
                .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|v| {
                    !v.is_empty() && normalize(v) != id && v.chars().count() <= MAX_TERM_CHARS
                })
                .filter(|v| !find_in(&lowered, v).is_empty())
                .take(8)
                .collect();
            if find_in(&lowered, &term).is_empty() && variants.is_empty() {
                return None;
            }
            Some(BookTerm {
                id,
                term,
                variants,
                kind: kind_of(&str_field(r, "kind")),
                from_book: clip_words(&str_field(r, "from_book"), MAX_EXPLANATION_WORDS),
                general: clip_words(&str_field(r, "general"), MAX_EXPLANATION_WORDS),
                chapter: chapter.to_string(),
                user_edited: false,
                hidden: false,
            })
        })
        .take(MAX_TERMS_PER_CHAPTER)
        .collect();
    Some(out)
}

/// Ein Kapitel wurde neu bewertet: Begriffe aus seiner alten Fassung, die
/// der Nutzer nicht angefasst hat, fallen weg, bevor die neuen kommen. So
/// veralten Erklärungen nicht.
pub fn drop_chapter(file: &mut TermsFile, chapter: &str) {
    file.terms
        .retain(|t| t.chapter != chapter || t.user_edited || t.hidden);
}

/// Neue Begriffe übernehmen. Ein vorhandener Begriff wird nur ergänzt,
/// solange der Nutzer ihn nicht bearbeitet oder ausgeblendet hat.
pub fn merge(file: &mut TermsFile, new: Vec<BookTerm>) {
    for t in new {
        match file.terms.iter_mut().find(|x| x.id == t.id) {
            None => file.terms.push(t),
            Some(old) if old.user_edited || old.hidden => {}
            Some(old) => {
                for v in t.variants {
                    if !old.variants.iter().any(|x| normalize(x) == normalize(&v)) {
                        old.variants.push(v);
                    }
                }
                if old.from_book.is_empty() {
                    old.from_book = t.from_book;
                }
                if old.general.is_empty() {
                    old.general = t.general;
                }
            }
        }
    }
    file.terms.sort_by(|a, b| a.id.cmp(&b.id));
}

// ---------------------------------------------------------------------------
// Suche
// ---------------------------------------------------------------------------

fn is_word_char(c: char) -> bool {
    // Kombinierende Zeichen (Akzente, U+0307 aus „İ“) gehören zum Wort.
    c.is_alphanumeric() || c == '-' || c == '_' || ('\u{0300}'..='\u{036F}').contains(&c)
}

/// Text in Kleinschreibung mit Rückabbildung auf Byte-Positionen. Einmal je
/// Text gebaut, dann für alle Begriffe benutzt.
pub struct Lowered {
    low: Vec<char>,
    pos: Vec<usize>,
    len: usize,
}

impl Lowered {
    pub fn new(hay: &str) -> Self {
        let mut low = Vec::with_capacity(hay.len());
        let mut pos = Vec::with_capacity(hay.len());
        for (b, c) in hay.char_indices() {
            for l in c.to_lowercase() {
                low.push(l);
                pos.push(b);
            }
        }
        Lowered {
            low,
            pos,
            len: hay.len(),
        }
    }
}

/// Alle Fundstellen von `needle` in `hay` als Byte-Bereiche. Groß- und
/// Kleinschreibung zählen nicht, Wortgrenzen schon.
pub fn find_all(hay: &str, needle: &str) -> Vec<(usize, usize)> {
    find_in(&Lowered::new(hay), needle)
}

pub fn find_in(text: &Lowered, needle: &str) -> Vec<(usize, usize)> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let (low, pos) = (&text.low, &text.pos);
    let mut out = Vec::new();
    let n = needle.len();
    let mut i = 0;
    while i + n <= low.len() {
        if low[i..i + n] == needle[..] {
            let before_ok = i == 0 || !is_word_char(low[i - 1]);
            let after_ok = i + n == low.len() || !is_word_char(low[i + n]);
            // Ende mitten in einem Zeichen, dessen Kleinschreibung aus
            // mehreren Zeichen besteht (z. B. „İ“): kein Treffer.
            let whole = i + n == low.len() || pos[i + n] != pos[i + n - 1];
            if before_ok && after_ok && whole {
                let start = pos[i];
                let end = pos.get(i + n).copied().unwrap_or(text.len);
                out.push((start, end));
                i += n;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

fn mark_len(text: &str, at: usize) -> usize {
    text[at..].find("}}").map(|i| i + 2).unwrap_or(0)
}

/// Seitenmarken `{{S. n}}` mit Byte-Position.
fn page_marks(text: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("{{S. ") {
        let at = from + i;
        let rest = &text[at + 5..];
        if let Some(end) = rest.find("}}") {
            if let Ok(p) = rest[..end].trim().parse() {
                out.push((at, p));
            }
        }
        from = at + 5;
    }
    out
}

/// Fundstellen für die Hervorhebung: je Absatz (Leerzeile oder Zeilenumbruch)
/// nur die erste Stelle eines Begriffs, damit der Text ruhig bleibt.
/// Ausgeblendete Begriffe fehlen. Längere Begriffe gewinnen bei Überlappung.
pub fn hits(text: &str, terms: &[BookTerm]) -> Vec<TermHit> {
    let marks = page_marks(text);
    let lowered = Lowered::new(text);
    let newlines: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    // Start → (Ende, Begriff); belegte Bereiche überlappen sich nie.
    let mut spans: BTreeMap<usize, (usize, &str)> = BTreeMap::new();
    // Alle Kandidaten (Begriff und Varianten) global: längere Fundstellen
    // zuerst, bei gleicher Länge nach Position.
    let mut candidates: Vec<(usize, usize, &str)> = terms
        .iter()
        .filter(|t| !t.hidden)
        .flat_map(|t| {
            std::iter::once(t.term.as_str())
                .chain(t.variants.iter().map(String::as_str))
                .flat_map(|n| find_in(&lowered, n))
                .map(move |(s, e)| (s, e, t.id.as_str()))
        })
        .filter(|(s, e, _)| s < e)
        // Nicht in Seitenmarken `{{S. n}}` hervorheben.
        .filter(|(s, _, _)| {
            let k = marks.partition_point(|(at, _)| at <= s);
            k == 0 || *s >= marks[k - 1].0 + mark_len(text, marks[k - 1].0)
        })
        .collect();
    candidates.sort_by_key(|(s, e, _)| (std::cmp::Reverse(e - s), *s));
    // Absatz, in dem ein Begriff schon markiert ist.
    let mut marked: HashSet<(&str, usize)> = HashSet::new();
    for (s, e, id) in candidates {
        let para = newlines.partition_point(|&nl| nl < s);
        if marked.contains(&(id, para)) {
            continue;
        }
        // Überlappung mit einem (längeren) schon belegten Begriff?
        let overlaps = spans
            .range(..e)
            .next_back()
            .is_some_and(|(_, (end, _))| *end > s);
        if overlaps {
            continue;
        }
        marked.insert((id, para));
        spans.insert(s, (e, id));
    }
    // Byte → UTF-16 in einem Durchlauf.
    let mut out = Vec::with_capacity(spans.len());
    let mut byte = 0usize;
    let mut units = 0u32;
    for (s, (e, id)) in spans {
        units += utf16_len(&text[byte..s]);
        let start = units;
        let end = start + utf16_len(&text[s..e]);
        units = end;
        byte = e;
        let k = marks.partition_point(|(at, _)| *at <= s);
        let page = k.checked_sub(1).map(|i| marks[i].1);
        out.push(TermHit {
            term_id: id.to_string(),
            start,
            end,
            page,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Ablage
// ---------------------------------------------------------------------------

pub fn load(page_dir: &Path) -> Result<TermsFile, String> {
    let path = super::store::ki_dir(page_dir).join("begriffe.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| "book_ai:terms_broken".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TermsFile::default()),
        Err(_) => Err("book_ai:terms_unreadable".into()),
    }
}

pub fn save(page_dir: &Path, file: &TermsFile) -> Result<(), String> {
    let dir = super::store::ki_dir(page_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let raw = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    write_atomic(&dir.join("begriffe.json"), raw.as_bytes())
}

/// Nutzeränderung übernehmen (bearbeiten, ausblenden, neu anlegen).
pub fn apply_user_edit(file: &mut TermsFile, mut term: BookTerm) -> Result<BookTerm, String> {
    term.term = term.term.split_whitespace().collect::<Vec<_>>().join(" ");
    if term.term.is_empty() || term.term.chars().count() > MAX_TERM_CHARS {
        return Err("book_ai:invalid_term".into());
    }
    let new_id = normalize(&term.term);
    term.from_book = clip_words(&term.from_book, MAX_EXPLANATION_WORDS * 2);
    term.general = clip_words(&term.general, MAX_EXPLANATION_WORDS * 2);
    term.variants = term
        .variants
        .iter()
        .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|v| !v.is_empty() && v.chars().count() <= MAX_TERM_CHARS)
        .take(8)
        .collect();
    term.user_edited = true;
    let old_id = std::mem::take(&mut term.id);
    // Die Identität bestimmt das Backend: eine unbekannte alte ID ist ein Fehler,
    // kein neuer Begriff, und ein belegter neuer Name überschreibt nichts.
    if !old_id.is_empty() && !file.terms.iter().any(|t| t.id == old_id) {
        return Err("book_ai:term_missing".into());
    }
    let taken = file.terms.iter().any(|t| t.id == new_id);
    if taken && old_id != new_id {
        return Err("book_ai:duplicate_term".into());
    }
    if !old_id.is_empty() && old_id != new_id {
        file.terms.retain(|t| t.id != old_id);
    }
    term.id = new_id;
    match file.terms.iter_mut().find(|t| t.id == term.id) {
        Some(slot) => *slot = term.clone(),
        None => file.terms.push(term.clone()),
    }
    file.terms.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(term)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sec(title: &str, text: &str) -> Section {
        Section {
            title: title.into(),
            text: text.into(),
            first_page: None,
            last_page: None,
        }
    }

    #[test]
    fn answers_are_read_and_invented_terms_dropped() {
        let text = "Die Albedo sinkt. Der Permafrost taut; Permafrostböden speichern Kohlenstoff.";
        let answer = r#"Hier: [
          {"term":"Albedo","variants":[],"kind":"fachbegriff","from_book":"sinkt","general":"Rückstrahlvermögen einer Fläche."},
          {"term":"Permafrost","variants":["Permafrostböden","Dauerfrost"],"kind":"konzept","from_book":"","general":"dauerhaft gefrorener Boden"},
          {"term":"Kipppunkt","kind":"fachbegriff","from_book":"erfunden","general":""},
          {"term":"albedo","kind":"x"}
        ] Ende"#;
        let t = parse_answer(answer, "Kap 1", text).unwrap();
        assert_eq!(t.len(), 2, "{t:?}");
        assert_eq!(t[0].id, "albedo");
        assert_eq!(t[1].kind, TermKind::Konzept);
        assert_eq!(t[1].variants, vec!["Permafrostböden".to_string()]);
        assert!(parse_answer("kein json", "k", text).is_none());
        // Text in eckigen Klammern vor dem Array und null-Felder stören nicht.
        let tolerant = parse_answer(
            r#"[Hinweis] [{"term":"Albedo","kind":null,"variants":null}]"#,
            "k",
            text,
        )
        .unwrap();
        assert_eq!(tolerant.len(), 1);
        assert!(parse_answer("[{\"term\": \"Albedo\"", "k", text).is_none());
    }

    #[test]
    fn explanations_are_capped() {
        let long = "wort ".repeat(200);
        let answer = format!("[{{\"term\":\"Albedo\",\"general\":\"{long}\"}}]");
        let t = parse_answer(&answer, "k", "Albedo").unwrap();
        assert_eq!(
            t[0].general.split_whitespace().count(),
            MAX_EXPLANATION_WORDS + 1
        );
        assert!(t[0].general.ends_with('…'));
    }

    #[test]
    fn only_new_or_changed_chapters_are_pending() {
        let s = vec![sec("A", "eins"), sec("B", "zwei")];
        let mut f = TermsFile::default();
        assert_eq!(pending(&f, &s).len(), 2);
        f.chapters
            .insert(chapter_key(0, &s[0]), chapter_hash(&s[0]));
        let p = pending(&f, &s);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, 1);
        let changed = vec![sec("A", "eins geändert"), sec("B", "zwei")];
        assert_eq!(pending(&f, &changed).len(), 2);
    }

    #[test]
    fn user_corrections_survive_later_runs() {
        let mut f = TermsFile::default();
        let base =
            parse_answer(r#"[{"term":"Albedo","general":"Modell"}]"#, "k", "Albedo").unwrap();
        merge(&mut f, base.clone());
        let mut mine = f.terms[0].clone();
        mine.general = "Meine Erklärung".into();
        apply_user_edit(&mut f, mine).unwrap();
        merge(&mut f, base.clone());
        assert_eq!(f.terms[0].general, "Meine Erklärung");
        assert!(f.terms[0].user_edited);

        let mut g = TermsFile::default();
        merge(&mut g, base.clone());
        g.terms[0].hidden = true;
        merge(&mut g, base);
        assert_eq!(g.terms.len(), 1);
        assert!(g.terms[0].hidden);
    }

    #[test]
    fn renaming_a_term_replaces_the_old_entry() {
        let mut f = TermsFile::default();
        merge(
            &mut f,
            parse_answer(r#"[{"term":"Albedo"}]"#, "k", "Albedo").unwrap(),
        );
        let mut t = f.terms[0].clone();
        t.term = "Albedo-Effekt".into();
        let out = apply_user_edit(&mut f, t).unwrap();
        assert_eq!(out.id, "albedo-effekt");
        assert_eq!(f.terms.len(), 1);
        let mut empty = f.terms[0].clone();
        empty.term = "  ".into();
        assert_eq!(
            apply_user_edit(&mut f, empty).unwrap_err(),
            "book_ai:invalid_term"
        );
    }

    #[test]
    fn search_respects_word_boundaries_and_case() {
        let hay = "Ort, ORT und Ortschaft. Großer Ort.";
        let h = find_all(hay, "ort");
        assert_eq!(h.len(), 3);
        assert_eq!(&hay[h[1].0..h[1].1], "ORT");
        assert!(find_all(hay, "").is_empty());
        let umlaut = find_all("Die Ärztin und die ärztin", "ärztin");
        assert_eq!(umlaut.len(), 2);
    }

    #[test]
    fn hits_mark_the_first_occurrence_per_paragraph_with_page_and_utf16_offsets() {
        let text = "😀 {{S. 4}} Albedo und Albedo.\nNeue Zeile mit Albedo-Effekt und Albedo.";
        let mut terms =
            parse_answer(r#"[{"term":"Albedo"},{"term":"Albedo-Effekt"}]"#, "k", text).unwrap();
        let h = hits(text, &terms);
        assert_eq!(h.len(), 3, "{h:?}");
        // „😀“ zählt in UTF-16 doppelt.
        let first = &h[0];
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let s = String::from_utf16(&utf16[first.start as usize..first.end as usize]).unwrap();
        assert_eq!(s, "Albedo");
        assert_eq!(first.page, Some(4));
        assert_eq!(h[1].term_id, "albedo-effekt", "längerer Begriff gewinnt");
        assert_eq!(h[2].term_id, "albedo");
        terms[0].hidden = true;
        assert_eq!(hits(text, &terms).len(), 1);
    }

    #[test]
    fn renaming_onto_an_existing_term_or_an_unknown_id_fails() {
        let mut f = TermsFile::default();
        let found = parse_answer(
            r#"[{"term":"Albedo"},{"term":"Permafrost"}]"#,
            "k",
            "Albedo Permafrost",
        )
        .unwrap();
        merge(&mut f, found);
        let mut t = f.terms[1].clone();
        t.term = "Albedo".into();
        assert_eq!(
            apply_user_edit(&mut f, t).unwrap_err(),
            "book_ai:duplicate_term"
        );
        let mut ghost = f.terms[0].clone();
        ghost.id = "gibt-es-nicht".into();
        assert_eq!(
            apply_user_edit(&mut f, ghost).unwrap_err(),
            "book_ai:term_missing"
        );
        assert_eq!(f.terms.len(), 2);
    }

    #[test]
    fn re_evaluated_chapters_drop_stale_untouched_terms() {
        let mut f = TermsFile::default();
        let found = parse_answer(r#"[{"term":"Alt"},{"term":"Mein"}]"#, "Kap", "Alt Mein").unwrap();
        merge(&mut f, found);
        f.terms[1].user_edited = true;
        drop_chapter(&mut f, "Kap");
        assert_eq!(f.terms.len(), 1);
        assert_eq!(f.terms[0].id, "mein");
    }

    #[test]
    fn multi_char_lowercase_and_combining_marks_do_not_split_words() {
        // „İ“ wird klein zu „i“ + U+0307: Treffer dürfen es nicht halbieren.
        assert!(find_all("ALİ", "ali").is_empty());
        assert!(find_all("İstanbul", "stanbul").is_empty());
        assert_eq!(find_all("ALİ geht", "geht").len(), 1);
        // Zerlegter Akzent (NFD): „te“ ist kein Wort in „te\u{301}“.
        assert!(find_all("Cafe te\u{301} x", "te").is_empty());
    }

    #[test]
    fn page_marks_are_never_highlighted() {
        let text = "{{S. 4}} Kapitel 4 beginnt.";
        let terms = parse_answer(r#"[{"term":"4"}]"#, "k", text).unwrap();
        let h = hits(text, &terms);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].start as usize, text.find("4 beginnt").unwrap());
    }

    #[test]
    fn prompt_names_known_terms_and_the_rules() {
        let p = extract_prompt(
            "Kap 1",
            "Text",
            " (that is: write in German)",
            &["Albedo".into()],
        );
        assert!(p.contains("never invent"));
        assert!(p.contains("Already known terms (do not repeat them): Albedo."));
        assert!(p.contains("write in German"));
        assert!(p.ends_with("Part: Kap 1\n\nText"));
    }

    #[test]
    fn terms_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = TermsFile::default();
        merge(
            &mut f,
            parse_answer(r#"[{"term":"Albedo"}]"#, "k", "Albedo").unwrap(),
        );
        f.chapters.insert("0:A".into(), "h".into());
        save(dir.path(), &f).unwrap();
        let back = load(dir.path()).unwrap();
        assert_eq!(back.terms, f.terms);
        assert_eq!(back.chapters, f.chapters);
        assert!(load(tempfile::tempdir().unwrap().path())
            .unwrap()
            .terms
            .is_empty());
    }
}
