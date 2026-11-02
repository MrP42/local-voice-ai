//! Kurzbuch (lva#205): die Kernaussagen eines Buchs als kurze, vorlesbare
//! Abschnitte. Aufbau nach dem Vorbild von Kernaussagen-Diensten, eigene
//! Texte und Gestaltung (Analyse:
//! `docs/analyse/2026-11-02-kurzbuch-funktionsanalyse.md`).
//!
//! Ablauf, sparsam für lokale Modelle mit kleinem Kontext:
//! 1. Zwischenfassungen je Kapitel aus dem gemeinsamen Cache (`engine`).
//! 2. Gliederung als JSON: worum es geht, für wen, Kernaussagen mit den
//!    Teilen, aus denen sie stammen.
//! 3. Je Kernaussage ein kurzer Schreibschritt nur mit diesen Teilen.
//! 4. Fazit aus den fertigen Kernaussagen.
//!
//! Gliederung und Kernaussagen liegen einzeln im Cache des Buchs: Nach einem
//! Abbruch wiederholt der nächste Lauf nur die fehlenden Schritte.
//!
//! Das Ergebnis ist Markdown mit fester Gliederung ([`assemble`]); die
//! Oberfläche erkennt Kurzbücher an Vorlagen-ID und Überschriften.

use std::future::Future;
use std::sync::atomic::AtomicBool;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::engine::{
    chapter_parts, check, condense, digest_chapters, digest_jobs, size, DigestCache, Part,
    RunInput, Steps, TOO_LONG,
};
use super::source::word_count;

pub const TEMPLATE_ID: &str = "builtin:kurzbuch";
pub const HEAD_ABOUT: &str = "Worum es geht";
pub const HEAD_AUDIENCE: &str = "Für wen";
pub const HEAD_CONCLUSION: &str = "Fazit";
/// Gliederung auch nach zwei Versuchen nicht lesbar.
pub const OUTLINE_UNREADABLE: &str = "book_ai:outline_unreadable";

/// Ändern sich die Prompts, gelten die alten Cache-Einträge nicht mehr.
const VERSION: &str = "kurzbuch-v1";
const IDEA_WORDS: u32 = 200;
const CONCLUSION_WORDS: u32 = 120;
const MIN_IDEAS: usize = 3;
const MAX_IDEAS: usize = 12;
/// Ab dieser Länge (Wörter) hat ein Kurzbuch mindestens sechs Kernaussagen.
const LONG_BOOK_WORDS: usize = 36_000;
const MAX_TITLE_CHARS: usize = 160;
const MIN_SOURCE_CHARS: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idea {
    /// „Schlagwort: ganzer Aussagesatz“.
    pub title: String,
    /// Indizes der Teile aus dem Gliederungsschritt.
    pub parts: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outline {
    pub about: String,
    pub audience: Vec<String>,
    pub ideas: Vec<Idea>,
}

#[derive(Deserialize)]
struct RawOutline {
    #[serde(default)]
    about: String,
    #[serde(default)]
    audience: Vec<String>,
    #[serde(default)]
    ideas: Vec<RawIdea>,
}

#[derive(Deserialize)]
struct RawIdea {
    #[serde(default)]
    title: String,
    #[serde(default)]
    parts: Vec<serde_json::Value>,
}

/// Empfohlene Zahl der Kernaussagen nach Buchlänge.
pub fn suggested_ideas(words: usize) -> usize {
    (words / 6_000).clamp(MIN_IDEAS, 10)
}

fn min_ideas(words: usize) -> usize {
    if words >= LONG_BOOK_WORDS {
        6
    } else {
        MIN_IDEAS
    }
}

fn key(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in std::iter::once(VERSION).chain(parts.iter().copied()) {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn numbered(parts: &[Part]) -> String {
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| format!("[{}] {}\n{}", i + 1, p.label, p.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn source_kind(full_text: bool) -> &'static str {
    if full_text {
        "the full text of the book, by chapter"
    } else {
        "digests of consecutive parts of the book, in order"
    }
}

pub fn outline_prompt(parts: &[Part], suggested: usize, full_text: bool, language: &str) -> String {
    format!(
        "You plan a short summary of a book that people read or listen to in about \
         fifteen minutes. The source is the ONLY basis: nothing that is not in it.\n\
         Reply with ONLY one JSON object, no code fence, in this shape:\n\
         {{\"about\": \"two or three sentences: what the book is about\",\n \
         \"audience\": [\"who benefits\", \"who benefits\", \"who benefits\"],\n \
         \"ideas\": [{{\"title\": \"Keyword: one complete sentence stating the idea\", \
         \"parts\": [1, 2]}}]}}\n\
         Rules: exactly three audience entries, each a short phrase. About {suggested} \
         key ideas (at least {MIN_IDEAS}, at most {MAX_IDEAS}), in the order of the book, \
         each one a distinct insight, argument or turning point. \"parts\" lists the \
         numbers of the source parts the idea draws on. Write all texts in the SAME \
         language as the source{language}.\n\n\
         Source ({kind}):\n\n{body}",
        kind = source_kind(full_text),
        body = numbered(parts),
    )
}

fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_start_matches(['#', '*', '-', ' ']).trim();
    let s = s.trim_matches('*').trim();
    if s.chars().count() <= MAX_TITLE_CHARS {
        return s.to_string();
    }
    let cut: String = s.chars().take(MAX_TITLE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

fn part_number(v: &serde_json::Value) -> Option<usize> {
    match v {
        serde_json::Value::Number(n) => n.as_u64().map(|n| n as usize),
        serde_json::Value::String(s) => s.trim().trim_matches(['[', ']']).parse().ok(),
        _ => None,
    }
}

/// Gliederung aus der Modellantwort. `None`, wenn sie nicht den Regeln
/// entspricht (dann folgt ein zweiter Versuch). `n_parts`: Zahl der Teile im
/// Prompt; ungültige Teilnummern fallen weg, fehlt die Zuordnung ganz, gilt
/// der Teil an der entsprechenden Stelle des Buchs.
pub fn parse_outline(answer: &str, n_parts: usize, min: usize) -> Option<Outline> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    let raw: RawOutline = serde_json::from_str(answer.get(start..=end)?).ok()?;
    let audience: Vec<String> = raw
        .audience
        .iter()
        .map(|a| one_line(a))
        .filter(|a| !a.is_empty())
        .take(3)
        .collect();
    let titles: Vec<(String, Vec<serde_json::Value>)> = raw
        .ideas
        .into_iter()
        .map(|i| (one_line(&i.title), i.parts))
        .filter(|(t, _)| !t.is_empty())
        .take(MAX_IDEAS)
        .collect();
    if audience.len() < 3 || titles.len() < min || n_parts == 0 {
        return None;
    }
    let count = titles.len();
    let ideas = titles
        .into_iter()
        .enumerate()
        .map(|(k, (title, raw_parts))| {
            let mut parts: Vec<usize> = raw_parts
                .iter()
                .filter_map(part_number)
                .filter(|n| (1..=n_parts).contains(n))
                .map(|n| n - 1)
                .collect();
            parts.sort_unstable();
            parts.dedup();
            if parts.is_empty() {
                parts.push(k * n_parts / count);
            }
            Idea { title, parts }
        })
        .collect();
    Some(Outline {
        about: raw.about.split_whitespace().collect::<Vec<_>>().join(" "),
        audience,
        ideas,
    })
}

/// Quelle einer Kernaussage: die genauen Zwischenfassungen der zugehörigen
/// Kapitel, wenn sie in eine Anfrage passen, sonst die Teile der Gliederung.
fn idea_sources(idea: &Idea, overview: &[Part], detail: &[Part], limit: usize) -> Vec<Part> {
    let chosen: Vec<&Part> = idea.parts.iter().filter_map(|&i| overview.get(i)).collect();
    let fine: Vec<Part> = detail
        .iter()
        .filter(|d| {
            !d.text.trim().is_empty()
                && chosen
                    .iter()
                    .any(|c| c.chapters.0 <= d.chapters.0 && d.chapters.1 <= c.chapters.1)
        })
        .cloned()
        .collect();
    if !fine.is_empty() && size(&fine) <= limit {
        return fine;
    }
    // Gröber: die Gliederungsteile, solange sie passen (mindestens einer).
    let mut out = Vec::new();
    for c in chosen {
        if !out.is_empty() && size(&out) + c.text.chars().count() + c.label.chars().count() > limit
        {
            break;
        }
        out.push(c.clone());
    }
    out
}

pub fn idea_prompt(
    about: &str,
    title: &str,
    index: usize,
    total: usize,
    sources: &[Part],
    language: &str,
) -> String {
    format!(
        "You write one key idea of a short book summary that is read and listened to. \
         The source is the ONLY basis: do not invent events, quotes, numbers or facts.\n\
         The book in brief: {about}\n\
         Key idea {index} of {total}: {title}\n\
         Write about {IDEA_WORDS} words of flowing prose in the SAME language as the \
         source{language}. Open with a question to the reader or a concrete \
         observation, then explain the idea, why it holds and one example from the \
         source. It must read well aloud: no headings, no lists, no tables, no title \
         line. Reply with ONLY the text.\n\n\
         Source (digests or text of the parts this idea draws on):\n\n{body}",
        body = sources
            .iter()
            .map(|p| format!("### {}\n{}", p.label, p.text))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

pub fn conclusion_prompt(about: &str, ideas: &[(String, String)], language: &str) -> String {
    format!(
        "Below are the key ideas of a short book summary, each with its text. Write \
         the conclusion: about {CONCLUSION_WORDS} words of flowing prose in the SAME \
         language{language}. Tie the ideas together and name the one central message \
         of the book. Nothing that is not in the key ideas. No heading, no list. Reply \
         with ONLY the text.\n\nThe book in brief: {about}\n\n{body}",
        body = ideas
            .iter()
            .map(|(t, b)| format!("### {t}\n{b}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

/// Modelltext ohne Überschriftenzeilen, damit die Gliederung des Kurzbuchs
/// eindeutig bleibt.
pub fn clean_body(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Das Kurzbuch als Markdown.
pub fn assemble(outline: &Outline, bodies: &[String], conclusion: &str) -> String {
    let mut out = String::new();
    if !outline.about.is_empty() {
        out.push_str(&format!("## {HEAD_ABOUT}\n\n{}\n\n", outline.about));
    }
    out.push_str(&format!("## {HEAD_AUDIENCE}\n\n"));
    for a in &outline.audience {
        out.push_str(&format!("- {a}\n"));
    }
    out.push('\n');
    for (idea, body) in outline.ideas.iter().zip(bodies) {
        out.push_str(&format!("## {}\n\n{}\n\n", idea.title, body));
    }
    out.push_str(&format!("## {HEAD_CONCLUSION}\n\n{conclusion}\n"));
    out
}

/// Ein Modellschritt mit Cache: vorhandene Antworten werden nicht neu erzeugt.
async fn cached<F, Fut>(
    cache: &DigestCache,
    key: &str,
    ask: &F,
    prompt: String,
) -> Result<String, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    if let Some(hit) = cache.get(key) {
        return Ok(hit);
    }
    let out = ask(prompt).await?;
    if !out.trim().is_empty() {
        cache.put(key, &out);
    }
    Ok(out)
}

/// Das Kurzbuch erzeugen. `progress(done, total)`.
pub async fn run<F, Fut>(
    input: RunInput<'_>,
    cache: &DigestCache,
    ask: F,
    progress: &(dyn Fn(usize, usize) + Send + Sync),
    cancel: &AtomicBool,
) -> Result<String, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let RunInput {
        sections,
        limit,
        model_key,
        language,
        ..
    } = input;
    if sections.is_empty() {
        return Err("book_ai:empty_source".into());
    }
    let words: usize = sections.iter().map(|s| word_count(&s.text) as usize).sum();
    let suggested = suggested_ideas(words);
    let min = min_ideas(words);
    let overhead = outline_prompt(&[], suggested, false, language)
        .chars()
        .count();
    let limit = limit.saturating_sub(overhead).max(MIN_SOURCE_CHARS);
    let chapters = chapter_parts(sections);
    let full_text = size(&chapters) <= limit;
    let jobs = if full_text {
        Vec::new()
    } else {
        digest_jobs(&chapters, limit)
    };
    let mut steps = Steps::new(jobs.len() + suggested + 2, progress);
    let detail = if full_text {
        chapters
    } else {
        digest_chapters(
            &chapters, jobs, model_key, language, cache, &ask, &mut steps, cancel,
        )
        .await?
    };
    let overview = condense(detail.clone(), limit, language, &ask, &mut steps, cancel).await?;
    if size(&overview) > limit {
        return Err(TOO_LONG.into());
    }

    // Gliederung; bei unlesbarer Antwort ein zweiter Versuch ohne Cache.
    let prompt = outline_prompt(&overview, suggested, full_text, language);
    let outline_key = key(&[model_key, "outline", &prompt]);
    let mut outline = None;
    for attempt in 0..2 {
        check(cancel)?;
        let answer = if attempt == 0 {
            cached(cache, &outline_key, &ask, prompt.clone()).await?
        } else {
            ask(prompt.clone()).await?
        };
        outline = parse_outline(&answer, overview.len(), min);
        if outline.is_some() {
            if attempt > 0 {
                cache.put(&outline_key, &answer);
            }
            break;
        }
    }
    let outline = outline.ok_or(OUTLINE_UNREADABLE)?;
    steps.step();
    steps.remaining(outline.ideas.len() + 1);

    let total = outline.ideas.len();
    let mut bodies = Vec::with_capacity(total);
    for (k, idea) in outline.ideas.iter().enumerate() {
        check(cancel)?;
        let sources = idea_sources(idea, &overview, &detail, limit);
        let prompt = idea_prompt(
            &outline.about,
            &idea.title,
            k + 1,
            total,
            &sources,
            language,
        );
        let ikey = key(&[model_key, "idea", &prompt]);
        let body = clean_body(&cached(cache, &ikey, &ask, prompt).await?);
        check(cancel)?;
        bodies.push(body);
        steps.step();
    }

    // Fazit aus den fertigen Texten; sind sie zu lang, nur aus den Titeln
    // mit dem jeweils ersten Absatz.
    let mut pairs: Vec<(String, String)> = outline
        .ideas
        .iter()
        .zip(&bodies)
        .map(|(i, b)| (i.title.clone(), b.clone()))
        .collect();
    let mut prompt = conclusion_prompt(&outline.about, &pairs, language);
    if prompt.chars().count() > limit + overhead {
        for p in &mut pairs {
            p.1 = p.1.split("\n\n").next().unwrap_or_default().to_string();
        }
        prompt = conclusion_prompt(&outline.about, &pairs, language);
    }
    check(cancel)?;
    let conclusion = clean_body(&ask(prompt).await?);
    check(cancel)?;
    steps.step();
    log::info!("book_ai: Kurzbuch mit {total} Kernaussagen");
    Ok(assemble(&outline, &bodies, &conclusion))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book_ai::source::Section;
    use crate::book_ai::templates::builtins;
    use std::sync::Mutex;

    fn section(title: &str, words: usize) -> Section {
        Section {
            title: title.into(),
            text: "Wort ".repeat(words).trim().to_string(),
            first_page: None,
            last_page: None,
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    const OUTLINE: &str = r###"Hier: {"about": "Ein Buch über Routinen.",
        "audience": ["Alle", "Teams", "Lernende"],
        "ideas": [{"title": "## Anfang: Klein beginnen.", "parts": [1]},
                  {"title": "Auslöser: Feste Momente.", "parts": ["2", 9]},
                  {"title": "Rückfall: Nie zweimal.", "parts": []}]}"###;

    #[test]
    fn the_outline_is_validated_and_cleaned() {
        let o = parse_outline(OUTLINE, 2, 3).unwrap();
        assert_eq!(o.audience.len(), 3);
        assert_eq!(o.ideas[0].title, "Anfang: Klein beginnen.");
        assert_eq!(o.ideas[1].parts, vec![1], "Teil 9 gibt es nicht");
        assert_eq!(
            o.ideas[2].parts,
            vec![1],
            "ohne Zuordnung: Teil nach Position"
        );
        assert!(
            parse_outline(OUTLINE, 2, 6).is_none(),
            "zu wenige Kernaussagen"
        );
        assert!(parse_outline(r#"{"about":"x","audience":["a"],"ideas":[]}"#, 2, 3).is_none());
        assert!(parse_outline("kein JSON", 2, 3).is_none());
    }

    #[test]
    fn the_result_has_the_fixed_structure() {
        let o = parse_outline(OUTLINE, 2, 3).unwrap();
        let md = assemble(
            &o,
            &["Eins.".into(), "Zwei.".into(), "Drei.".into()],
            "Ende.",
        );
        assert!(md.starts_with("## Worum es geht\n\nEin Buch über Routinen."));
        assert!(md.contains("## Für wen\n\n- Alle\n- Teams\n- Lernende\n"));
        assert!(md.contains("## Rückfall: Nie zweimal.\n\nDrei."));
        assert!(md.ends_with("## Fazit\n\nEnde.\n"));
        assert_eq!(clean_body("# Titel\nText\n## Zwischen\nmehr"), "Text\nmehr");
    }

    fn fake(
        calls: &Mutex<Vec<String>>,
    ) -> impl Fn(String) -> std::future::Ready<Result<String, String>> + '_ {
        move |p: String| {
            let out = if p.contains("You plan a short summary") {
                OUTLINE.to_string()
            } else {
                "Text.".to_string()
            };
            calls.lock().unwrap().push(p);
            std::future::ready(Ok(out))
        }
    }

    #[test]
    fn short_books_need_outline_ideas_and_conclusion_only() {
        let calls = Mutex::new(Vec::new());
        let t = builtins()
            .into_iter()
            .find(|t| t.id == TEMPLATE_ID)
            .unwrap();
        let s = [section("Eins", 100), section("Zwei", 100)];
        let steps = Mutex::new(Vec::new());
        let md = block_on(run(
            RunInput {
                sections: &s,
                template: &t,
                limit: 20_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            fake(&calls),
            &|d, n| steps.lock().unwrap().push((d, n)),
            &AtomicBool::new(false),
        ))
        .unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1 + 3 + 1);
        assert!(calls[1].contains("Key idea 1 of 3: Anfang: Klein beginnen."));
        assert!(calls[1].contains("### Eins"), "nur das zugehörige Kapitel");
        assert!(!calls[1].contains("### Zwei"));
        assert!(calls[4].contains("### Rückfall: Nie zweimal.\nText."));
        assert!(md.contains("## Anfang: Klein beginnen.\n\nText."));
        assert_eq!(steps.lock().unwrap().last(), Some(&(5, 5)));
    }

    #[test]
    fn a_second_run_reuses_outline_and_ideas_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DigestCache {
            dir: Some(dir.path().to_path_buf()),
        };
        let t = builtins()
            .into_iter()
            .find(|t| t.id == TEMPLATE_ID)
            .unwrap();
        let s = [section("Eins", 1_500), section("Zwei", 1_500)];
        let counts: Vec<usize> = (0..2)
            .map(|_| {
                let calls = Mutex::new(Vec::new());
                block_on(run(
                    RunInput {
                        sections: &s,
                        template: &t,
                        limit: 9_000,
                        model_key: "m",
                        language: "",
                    },
                    &cache,
                    fake(&calls),
                    &|_, _| {},
                    &AtomicBool::new(false),
                ))
                .unwrap();
                let n = calls.lock().unwrap().len();
                n
            })
            .collect();
        // Erster Lauf: 2 Zwischenfassungen + Gliederung + 3 Kernaussagen + Fazit.
        assert_eq!(counts, vec![7, 1], "zweiter Lauf: nur das Fazit");
    }

    #[test]
    fn an_unreadable_outline_is_retried_once_then_fails_clearly() {
        let calls = Mutex::new(0usize);
        let t = builtins()
            .into_iter()
            .find(|t| t.id == TEMPLATE_ID)
            .unwrap();
        let s = [section("Eins", 100)];
        let err = block_on(run(
            RunInput {
                sections: &s,
                template: &t,
                limit: 20_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            |_| {
                *calls.lock().unwrap() += 1;
                std::future::ready(Ok("Entschuldigung".to_string()))
            },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap_err();
        assert_eq!(err, OUTLINE_UNREADABLE);
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[test]
    fn long_books_ask_for_more_ideas() {
        assert_eq!(suggested_ideas(2_000), 3);
        assert_eq!(suggested_ideas(48_000), 8);
        assert_eq!(suggested_ideas(500_000), 10);
        assert_eq!(min_ideas(40_000), 6);
    }
}
