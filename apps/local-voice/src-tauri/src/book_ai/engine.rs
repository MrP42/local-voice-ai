//! Ablauf einer Buchzusammenfassung nach Vorlage (lva#182).
//!
//! 1. Passt der gewählte Text in eine Anfrage, bekommt das Modell ihn ganz
//!    zusammen mit der Vorlage.
//! 2. Sonst entsteht je Kapitel (lange Kapitel blockweise) eine
//!    **Zwischenfassung**. Sie ist neutral und dicht und hängt nicht von der
//!    Vorlage ab. Gecacht wird sie unter einem Hash aus Prompt-Version, Modell
//!    und Kapiteltext. Eine zweite Vorlage für dasselbe Buch braucht deshalb
//!    nur noch den letzten Schritt.
//! 3. Sind die Zwischenfassungen zusammen noch zu lang, werden sie in Gruppen
//!    verdichtet.
//! 4. Die Vorlage formt aus den Zwischenfassungen das Ergebnis.
//!
//! Das Sprachmodell wird als Funktion übergeben, damit der Ablauf ohne
//! Netz testbar ist. Im Log stehen nur Zähler, nie Text.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use super::source::{word_count, Section};
use super::templates::{BookTemplate, TemplateForm};

/// Ändert sich der Zwischenfassungs-Prompt, gilt der alte Cache nicht mehr.
const DIGEST_VERSION: &str = "book-digest-v1";

pub const CANCELLED: &str = "book_ai:cancelled";
/// Auch nach dem Verdichten passt die Quelle nicht in eine Anfrage.
pub const TOO_LONG: &str = "book_ai:too_long";
/// Mindestplatz für Quelltext je Anfrage, auch bei sehr großem Vorlagenrahmen.
const MIN_SOURCE_CHARS: usize = 4_000;
const MAX_CONDENSE_ROUNDS: usize = 4;

/// Ablage der Zwischenfassungen. `None` schaltet den Cache ab (Tests).
pub struct DigestCache {
    pub dir: Option<PathBuf>,
}

impl DigestCache {
    pub fn key(model: &str, label: &str, text: &str, words: u32) -> String {
        let mut h = Sha256::new();
        for part in [DIGEST_VERSION, model, label, &words.to_string(), text] {
            h.update(part.as_bytes());
            h.update([0u8]);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    fn get(&self, key: &str) -> Option<String> {
        let path = self.dir.as_ref()?.join(format!("{key}.md"));
        std::fs::read_to_string(path)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }

    fn put(&self, key: &str, text: &str) {
        let Some(dir) = &self.dir else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        // Ein Cache-Fehler ist kein Fehler der Zusammenfassung.
        let _ =
            crate::commands::pages::write_atomic(&dir.join(format!("{key}.md")), text.as_bytes());
    }
}

/// Zielumfang einer Zwischenfassung: etwa ein Zehntel, 120 bis 500 Wörter.
pub fn digest_words(text: &str) -> u32 {
    (word_count(text) / 10).clamp(120, 500)
}

pub fn digest_prompt(label: &str, text: &str, words: u32, language: &str) -> String {
    format!(
        "Write a dense, factual digest of the following part of a book, in the SAME \
         language as the text{language}. About {words} words. Keep everything a later \
         summary may need: main events or arguments in order, names of people and \
         places, key terms with their meaning, numbers, conclusions, and at most two \
         short verbatim quotes that matter. No commentary, no evaluation, nothing that \
         is not in the text. Reply with ONLY the digest.\n\nPart: {label}\n\n{text}"
    )
}

pub fn condense_prompt(parts: &[(String, String)], language: &str) -> String {
    format!(
        "The following are digests of consecutive parts of one book, in order. \
         Condense them into ONE digest in the same language{language}, about half as \
         long, keeping the order, names, key terms and the part labels as short \
         headings. Nothing that is not in the digests. Reply with ONLY the digest.\n\n{}",
        join_parts(parts)
    )
}

fn join_parts(parts: &[(String, String)]) -> String {
    parts
        .iter()
        .map(|(label, text)| format!("### {label}\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn audience_instruction(audience: &str) -> &'static str {
    match audience {
        "fachpublikum" => "Write for an expert audience; keep technical terms precise.",
        "management" => "Write for decision makers: lead with outcomes, implications and recommendations.",
        "einfache_sprache" => "Write in plain language: short sentences, common words, explain every necessary technical term.",
        "lernende" => "Write for learners: clear, structured, easy to memorise.",
        _ => "Write for a general audience without assuming prior knowledge.",
    }
}

fn form_instruction(form: TemplateForm) -> &'static str {
    match form {
        TemplateForm::Prose => {
            "Form: flowing prose that reads well aloud. No headings, no lists, no tables."
        }
        TemplateForm::Structured => {
            "Form: Markdown with a '## ' heading per section and prose or short lists below."
        }
        TemplateForm::List => "Form: a Markdown list; every item is a complete sentence.",
        TemplateForm::Table => "Form: one Markdown table, optionally one sentence before it.",
    }
}

/// Prompt des letzten Schritts. `full_text`: Die Quelle ist der Buchtext
/// selbst und keine Zwischenfassung.
pub fn final_prompt(
    template: &BookTemplate,
    parts: &[(String, String)],
    full_text: bool,
    language: &str,
) -> String {
    let mut sections = String::new();
    if !template.sections.is_empty() {
        sections.push_str("\nSections, in this order (use the titles as headings):\n");
        for s in &template.sections {
            sections.push_str(&format!("- {}: {}\n", s.title.trim(), s.instruction.trim()));
        }
    }
    let source = if full_text {
        "the full text of the selected part of the book"
    } else {
        "digests of consecutive parts of the book, in order"
    };
    format!(
        "You write a document based on a book. The book is the ONLY source for \
         statements about its content: do not invent events, quotes, page numbers or \
         facts. If the source does not cover something the task asks for, say so in \
         one short sentence.\n\
         Task: {context}\n\
         {audience}\n\
         Target length: about {words} words.\n\
         {form}{sections}\n\
         Write in the SAME language as the source{language}. Reply with ONLY the \
         document, without a title line and without meta commentary.\n\n\
         Source ({source}):\n\n{body}",
        context = template.context.trim(),
        audience = audience_instruction(&template.audience),
        words = template.target_words,
        form = form_instruction(template.form),
        body = join_parts(parts),
    )
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

/// Teile in Gruppen, die jeweils höchstens `limit` Zeichen haben.
fn groups(parts: Vec<(String, String)>, limit: usize) -> Vec<Vec<(String, String)>> {
    let mut out: Vec<Vec<(String, String)>> = Vec::new();
    let mut size = 0;
    for p in parts {
        let len = p.0.chars().count() + p.1.chars().count();
        if out.is_empty() || (size + len > limit && !out.last().is_some_and(|g| g.is_empty())) {
            out.push(Vec::new());
            size = 0;
        }
        size += len;
        if let Some(g) = out.last_mut() {
            g.push(p);
        }
    }
    out
}

pub struct RunInput<'a> {
    pub sections: &'a [Section],
    pub template: &'a BookTemplate,
    /// Größte Eingabe je Anfrage in Zeichen (lokal klein, Cloud groß).
    pub limit: usize,
    /// Anbieter und Modell: Teil des Cache-Schlüssels.
    pub model_key: &'a str,
    /// Sprachhinweis für den Prompt, z. B. " (that is: write in German)".
    pub language: &'a str,
}

/// Die Zusammenfassung erzeugen. `progress(done, total)`; der letzte Schritt
/// ist die Vorlage.
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
        template,
        limit,
        model_key,
        language,
    } = input;
    if sections.is_empty() {
        return Err("book_ai:empty_source".into());
    }
    // Das Limit gilt für die ganze Anfrage: Vorlagenrahmen und Prompttext
    // gehen vom Platz für den Quelltext ab.
    let overhead = final_prompt(template, &[], false, language).chars().count();
    let limit = limit.saturating_sub(overhead).max(MIN_SOURCE_CHARS);
    let labelled: Vec<(String, String)> = sections
        .iter()
        .enumerate()
        .map(|(i, s)| (s.label(i), s.text.clone()))
        .collect();
    let total_chars: usize = labelled
        .iter()
        .map(|(l, t)| l.chars().count() + t.chars().count())
        .sum();

    if total_chars <= limit {
        progress(0, 1);
        let out = ask(final_prompt(template, &labelled, true, language)).await?;
        check(cancel)?;
        progress(1, 1);
        return Ok(out);
    }

    // Zwischenfassungen: je Kapitel, lange Kapitel blockweise.
    let mut jobs: Vec<(usize, String, String)> = Vec::new();
    for (i, (label, text)) in labelled.iter().enumerate() {
        let chunks = crate::summarizer::chunk_text(text, limit);
        let n = chunks.len();
        for (k, chunk) in chunks.into_iter().enumerate() {
            let l = if n > 1 {
                format!("{label}, Abschnitt {}/{n}", k + 1)
            } else {
                label.clone()
            };
            jobs.push((i, l, chunk));
        }
    }
    let mut total = jobs.len() + 1;
    let mut done = 0;
    progress(done, total);
    let mut digests: Vec<(String, String)> = labelled
        .iter()
        .map(|(l, _)| (l.clone(), String::new()))
        .collect();
    let mut cached = 0usize;
    for (i, label, chunk) in &jobs {
        check(cancel)?;
        let words = digest_words(chunk);
        let key = DigestCache::key(model_key, label, chunk, words);
        let digest = match cache.get(&key) {
            Some(hit) => {
                cached += 1;
                hit
            }
            None => {
                let d = ask(digest_prompt(label, chunk, words, language)).await?;
                check(cancel)?;
                cache.put(&key, &d);
                d
            }
        };
        let slot = &mut digests[*i].1;
        if !slot.is_empty() {
            slot.push_str("\n\n");
        }
        slot.push_str(digest.trim());
        done += 1;
        progress(done, total);
    }
    log::info!(
        "book_ai: {} Kapitel, {} Bloecke, {} aus dem Cache",
        sections.len(),
        jobs.len(),
        cached
    );

    // Zu lang für den letzten Schritt: in Gruppen verdichten. Gruppen mit
    // halbem Limit, damit jede Runde die Zahl der Teile wirklich senkt.
    let mut parts = digests;
    for _ in 0..MAX_CONDENSE_ROUNDS {
        let size: usize = parts
            .iter()
            .map(|(l, t)| l.chars().count() + t.chars().count())
            .sum();
        if size <= limit || parts.len() <= 1 {
            break;
        }
        let gs = groups(parts, limit / 2);
        if gs.iter().all(|g| g.len() == 1) {
            // Jede Zwischenfassung ist für sich schon zu groß: nichts zu bündeln.
            parts = gs.into_iter().flatten().collect();
            break;
        }
        total += gs.len();
        progress(done, total);
        let mut next = Vec::with_capacity(gs.len());
        for g in gs {
            check(cancel)?;
            let label = match (g.first(), g.last()) {
                (Some(a), Some(b)) if g.len() > 1 => format!("{} – {}", a.0, b.0),
                (Some(a), _) => a.0.clone(),
                _ => continue,
            };
            let text = if g.len() == 1 {
                g[0].1.clone()
            } else {
                ask(condense_prompt(&g, language)).await?
            };
            next.push((label, text));
            done += 1;
            progress(done, total);
        }
        parts = next;
    }

    let size: usize = parts
        .iter()
        .map(|(l, t)| l.chars().count() + t.chars().count())
        .sum();
    if size > limit {
        // Lieber ein klarer Fehler als ein still abgeschnittener Prompt.
        return Err(TOO_LONG.into());
    }
    check(cancel)?;
    let out = ask(final_prompt(template, &parts, false, language)).await?;
    check(cancel)?;
    progress(total, total);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book_ai::templates::builtins;
    use std::sync::Mutex;

    fn section(title: &str, words: usize) -> Section {
        Section {
            title: title.into(),
            text: "Wort ".repeat(words).trim().to_string(),
            first_page: Some(1),
            last_page: Some(2),
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn the_final_prompt_carries_template_form_and_sections() {
        let t = builtins()
            .into_iter()
            .find(|t| t.id == "builtin:management_summary")
            .unwrap();
        let p = final_prompt(
            &t,
            &[("Eins (S. 1–2)".into(), "Text".into())],
            true,
            " (that is: write in German)",
        );
        assert!(p.contains("about 300 words"));
        assert!(p.contains("decision makers"));
        assert!(p.contains("- Kernaussage:"));
        assert!(p.contains("'## ' heading"));
        assert!(p.contains("ONLY source"));
        assert!(p.contains("full text"));
        assert!(p.contains("write in German"));
        assert!(p.ends_with("### Eins (S. 1–2)\nText"));
    }

    #[test]
    fn short_sources_need_exactly_one_call() {
        let calls = Mutex::new(Vec::new());
        let t = &builtins()[0];
        let s = [section("Eins", 50)];
        let out = block_on(run(
            RunInput {
                sections: &s,
                template: t,
                limit: 10_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            |p| {
                calls.lock().unwrap().push(p);
                async { Ok("Ergebnis".to_string()) }
            },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(out, "Ergebnis");
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn long_sources_digest_each_chapter_then_apply_the_template() {
        let calls = Mutex::new(Vec::<String>::new());
        let steps = Mutex::new(Vec::new());
        let t = &builtins()[1];
        let s = [
            section("Eins", 300),
            section("Zwei", 300),
            section("Drei", 300),
        ];
        let out = block_on(run(
            RunInput {
                sections: &s,
                template: t,
                limit: 2_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            |p| {
                calls.lock().unwrap().push(p);
                async { Ok("kurz".to_string()) }
            },
            &|d, n| steps.lock().unwrap().push((d, n)),
            &AtomicBool::new(false),
        ))
        .unwrap();
        assert_eq!(out, "kurz");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 4, "drei Zwischenfassungen + Vorlage");
        assert!(calls[0].contains("Part: Eins (S. 1–2)"));
        assert!(calls[3].contains("digests of consecutive parts"));
        assert!(calls[3].contains("### Drei (S. 1–2)\nkurz"));
        assert_eq!(steps.lock().unwrap().last(), Some(&(4, 4)));
    }

    #[test]
    fn cached_digests_are_reused_for_a_second_template() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DigestCache {
            dir: Some(dir.path().to_path_buf()),
        };
        let s = [section("Eins", 600), section("Zwei", 600)];
        let count = Mutex::new(0usize);
        for t in &builtins()[..2] {
            block_on(run(
                RunInput {
                    sections: &s,
                    template: t,
                    limit: 1_800,
                    model_key: "m",
                    language: "",
                },
                &cache,
                |_| {
                    *count.lock().unwrap() += 1;
                    async { Ok("d".to_string()) }
                },
                &|_, _| {},
                &AtomicBool::new(false),
            ))
            .unwrap();
        }
        // Erster Lauf: 2 Zwischenfassungen + 1 Vorlage; zweiter nur die Vorlage.
        assert_eq!(*count.lock().unwrap(), 4);
        // Anderes Modell: neuer Schlüssel.
        assert_ne!(
            DigestCache::key("a", "l", "t", 120),
            DigestCache::key("b", "l", "t", 120)
        );
    }

    #[test]
    fn oversized_digests_are_condensed_in_groups() {
        let calls = Mutex::new(Vec::<String>::new());
        let t = &builtins()[0];
        let s: Vec<_> = (0..6).map(|i| section(&format!("K{i}"), 300)).collect();
        let long_digest = "x".repeat(900);
        block_on(run(
            RunInput {
                sections: &s,
                template: t,
                limit: 2_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            |p| {
                calls.lock().unwrap().push(p);
                let d = long_digest.clone();
                async move { Ok(d) }
            },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.contains("Condense them")));
        assert!(calls.last().unwrap().contains("You write a document"));
    }

    #[test]
    fn cancel_stops_between_steps() {
        let cancel = AtomicBool::new(true);
        let s = [section("Eins", 300), section("Zwei", 300)];
        let err = block_on(run(
            RunInput {
                sections: &s,
                template: &builtins()[0],
                limit: 1_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            |_| async { Ok("d".to_string()) },
            &|_, _| {},
            &cancel,
        ))
        .unwrap_err();
        assert_eq!(err, CANCELLED);
    }

    #[test]
    fn sources_that_stay_too_long_fail_clearly_instead_of_being_cut() {
        let s: Vec<_> = (0..4).map(|i| section(&format!("K{i}"), 2_000)).collect();
        let huge = "x".repeat(20_000);
        let err = block_on(run(
            RunInput {
                sections: &s,
                template: &builtins()[0],
                limit: 12_000,
                model_key: "m",
                language: "",
            },
            &DigestCache { dir: None },
            |_| {
                let d = huge.clone();
                async move { Ok(d) }
            },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap_err();
        assert_eq!(err, TOO_LONG);
    }

    #[test]
    fn digest_length_scales_with_the_chapter() {
        assert_eq!(digest_words("a b c"), 120);
        assert_eq!(digest_words(&"w ".repeat(3_000)), 300);
        assert_eq!(digest_words(&"w ".repeat(50_000)), 500);
    }
}
