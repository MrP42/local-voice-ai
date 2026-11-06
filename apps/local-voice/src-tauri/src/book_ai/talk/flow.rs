//! Ablauf einer Antwort: Sammeln des Modellstroms mit satzweiser Prüfung,
//! Rückfälle ohne Modell (Fundstellen, Hinweis) und die feldübergreifenden
//! Regeln des Vertrags (Plan §7a). Gleiches Verhalten wie der Swift-Kern
//! (Referenz, `BookTalkSession.swift`).

use super::cite::{
    best_quote, evaluate, plain, reference_sentence, speech_tail, spoken_quote, BookTalkCitation,
    DeltaFilter, Grounding, SentenceKind, QUOTE_MAX_CHARS,
};
use super::index::{sentence_spans, BookIndex};
use super::intent::{BookTalkIntent, Lang};
use super::retrieve::Retrieval;
use super::{
    BookTalkAnswer, BookTalkEvent, BookTalkRequest, BookTalkScope, BookTalkSentence,
    CODE_NO_FRONTIER, REASON_NO_HITS,
};

/// Alles, was der Ablauf einer Frage über die Anfrage weiß.
pub struct Ctx<'a> {
    pub request: &'a BookTalkRequest,
    pub intent: BookTalkIntent,
    pub lang: Lang,
    pub reading_page: Option<u32>,
    pub index: &'a BookIndex,
}

impl Ctx<'_> {
    pub fn scope(&self) -> BookTalkScope {
        self.request.scope
    }

    pub fn frontier(&self) -> u32 {
        self.request.frontier.unwrap_or(0)
    }
}

/// Sammelt den Modellstrom, prüft Satz für Satz und liefert Ereignisse.
pub struct Collector<'a> {
    grounding: Grounding<'a>,
    lang: Lang,
    request_id: String,
    filter: DeltaFilter,
    text: String,
    done: usize,
    kept: Vec<BookTalkSentence>,
    held: Vec<(String, Vec<BookTalkCitation>, bool)>,
    general_kept: usize,
    cited_kept: usize,
    pub dropped: u32,
    pub has_anchor: bool,
}

impl<'a> Collector<'a> {
    pub fn new(grounding: Grounding<'a>, lang: Lang, request_id: &str) -> Self {
        Self {
            grounding,
            lang,
            request_id: request_id.to_string(),
            filter: DeltaFilter::new(),
            text: String::new(),
            done: 0,
            kept: Vec::new(),
            held: Vec::new(),
            general_kept: 0,
            cited_kept: 0,
            dropped: 0,
            has_anchor: false,
        }
    }

    pub fn feed(&mut self, piece: &str) -> Vec<BookTalkEvent> {
        let t = self.filter.feed(piece);
        self.text.push_str(&t);
        self.handle(false)
    }

    pub fn finish(&mut self) -> Vec<BookTalkEvent> {
        let t = self.filter.finish();
        self.text.push_str(&t);
        self.handle(true)
    }

    fn handle(&mut self, last: bool) -> Vec<BookTalkEvent> {
        let units: Vec<u16> = self.text.encode_utf16().collect();
        let sentences = sentence_spans(&units);
        let limit = if last {
            sentences.len()
        } else {
            sentences.len().saturating_sub(1)
        };
        let mut events = Vec::new();
        while self.done < limit {
            let (s0, s1) = sentences[self.done];
            self.done += 1;
            let sentence = String::from_utf16_lossy(&units[s0..s1])
                .trim_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r')
                .to_string();
            let ids: Vec<usize> = self
                .filter
                .markers
                .iter()
                .filter(|m| m.offset > s0 && m.offset <= s1)
                .flat_map(|m| m.ids.clone())
                .collect();
            let (kind, cites) = evaluate(&sentence, &ids, &self.grounding, self.general_kept == 0);
            match kind {
                SentenceKind::Dropped => self.dropped += 1,
                SentenceKind::General => {
                    self.general_kept += 1;
                    if self.has_anchor {
                        events.push(self.emit(&sentence, Vec::new(), true));
                    } else {
                        self.held.push((sentence, Vec::new(), true));
                    }
                }
                SentenceKind::Cited | SentenceKind::Absent => {
                    self.has_anchor = true;
                    for (text, citations, general) in std::mem::take(&mut self.held) {
                        events.push(self.emit(&text, citations, general));
                    }
                    events.push(self.emit(&sentence, cites, false));
                }
            }
        }
        events
    }

    fn emit(
        &mut self,
        sentence: &str,
        citations: Vec<BookTalkCitation>,
        general: bool,
    ) -> BookTalkEvent {
        let de = self.lang == Lang::De;
        let mut speech = plain(sentence, self.lang);
        if general {
            let prefix = if de { "Allgemein:" } else { "General:" };
            if let Some(rest) = speech.strip_prefix(prefix) {
                speech = format!(
                    "{}{rest}",
                    if de { "Allgemein gilt:" } else { "In general:" }
                );
            }
        } else if !citations.is_empty() {
            self.cited_kept += 1;
            if self.cited_kept == 1 && self.grounding.intent != BookTalkIntent::Recap {
                if let Some(c) = citations.first() {
                    speech.push(' ');
                    speech.push_str(&reference_sentence(c, self.lang));
                }
            }
        }
        let s = BookTalkSentence {
            index: self.kept.len() as u32,
            text: sentence.to_string(),
            speech,
            citations,
            general,
        };
        self.kept.push(s.clone());
        BookTalkEvent::Sentence {
            request_id: self.request_id.clone(),
            sentence: s,
        }
    }

    pub fn answer(&self, ctx: &Ctx<'_>) -> BookTalkAnswer {
        let cites: Vec<BookTalkCitation> =
            self.kept.iter().flat_map(|s| s.citations.clone()).collect();
        let mut speech = self
            .kept
            .iter()
            .map(|s| s.speech.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let tail = speech_tail(ctx.intent, !cites.is_empty(), ctx.lang, ctx.reading_page);
        if !tail.is_empty() {
            if !speech.is_empty() {
                speech.push(' ');
            }
            speech.push_str(&tail);
        }
        BookTalkAnswer {
            intent: ctx.intent,
            text: self
                .kept
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            sentences: self.kept.clone(),
            speech,
            citations: cites,
            found_only: false,
            reason: None,
            dropped: self.dropped,
            scope: ctx.scope(),
            frontier: ctx.frontier(),
            preview: false,
        }
    }
}

/// Antwort ohne Modell: Ereignisse plus die endgültige Antwort.
pub struct Fallback {
    pub events: Vec<BookTalkEvent>,
    pub answer: BookTalkAnswer,
}

impl Fallback {
    /// Ein Hinweis in einem Satz; im Bereich „bis hier“ mit dem Angebot, im
    /// ganzen Buch zu suchen.
    pub fn notice(ctx: &Ctx<'_>, reason: &str, text: &str) -> Fallback {
        let mut speech = plain(text, ctx.lang);
        let mut shown = text.to_string();
        if ctx.scope() == BookTalkScope::UntilPosition {
            let offer = if ctx.lang == Lang::De {
                "Soll ich im ganzen Buch suchen?"
            } else {
                "Shall I search the whole book?"
            };
            shown.push(' ');
            shown.push_str(offer);
            speech.push(' ');
            speech.push_str(offer);
        }
        let s = BookTalkSentence {
            index: 0,
            text: shown.clone(),
            speech: speech.clone(),
            citations: Vec::new(),
            general: false,
        };
        let answer = BookTalkAnswer {
            intent: ctx.intent,
            sentences: vec![s.clone()],
            text: shown,
            speech,
            citations: Vec::new(),
            found_only: true,
            reason: Some(reason.to_string()),
            dropped: 0,
            scope: ctx.scope(),
            frontier: ctx.frontier(),
            preview: false,
        };
        Fallback {
            events: vec![BookTalkEvent::Sentence {
                request_id: ctx.request.request_id.clone(),
                sentence: s,
            }],
            answer,
        }
    }

    /// Die zwei besten Fundstellen oder ein Hinweis; nie ein leerer Fehler.
    pub fn found_only(
        ctx: &Ctx<'_>,
        retrieval: &Retrieval,
        reason: Option<&str>,
        dropped: u32,
    ) -> Fallback {
        let de = ctx.lang == Lang::De;
        let mut sentences: Vec<BookTalkSentence> = Vec::new();
        let mut cites: Vec<BookTalkCitation> = Vec::new();
        for (n, e) in retrieval.best_first().into_iter().take(2).enumerate() {
            let Some(q) = best_quote(&ctx.request.question, &e.chunk) else {
                continue;
            };
            let lead = match (n, de) {
                (0, true) => "Dazu steht etwas",
                (0, false) => "There is something about it",
                (_, true) => "Außerdem",
                (_, false) => "Also",
            };
            let page = ctx.index.page_at(q.start);
            let place = match (page, de) {
                (Some(p), true) => format!(" auf Seite {p}"),
                (Some(p), false) => format!(" on page {p}"),
                (None, true) => " im Buch".to_string(),
                (None, false) => " in the book".to_string(),
            };
            let c = BookTalkCitation {
                q: e.id as u32,
                page,
                chapter_index: e.chunk.chapter_index as u32,
                chapter_title: e.chunk.chapter_title.clone(),
                start: q.start as u32,
                end: q.end as u32,
                quote: q.text.clone(),
            };
            sentences.push(BookTalkSentence {
                index: sentences.len() as u32,
                text: format!("{lead}{place}: „{}“", q.text),
                speech: format!("{lead}{place}: ‚{}‘", spoken_quote(&q.text)),
                citations: vec![c.clone()],
                general: false,
            });
            cites.push(c);
        }
        if sentences.is_empty() {
            return Self::notice(
                ctx,
                reason.unwrap_or(REASON_NO_HITS),
                if de {
                    "Dazu finde ich im Buch nichts."
                } else {
                    "I find nothing about that in the book."
                },
            );
        }
        let mut speech = sentences
            .iter()
            .map(|s| s.speech.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if ctx.intent == BookTalkIntent::Locate {
            speech.push_str(if de {
                " Soll ich dir die Stelle vorlesen?"
            } else {
                " Shall I read the passage to you?"
            });
        }
        let events = sentences
            .iter()
            .map(|s| BookTalkEvent::Sentence {
                request_id: ctx.request.request_id.clone(),
                sentence: s.clone(),
            })
            .collect();
        let answer = BookTalkAnswer {
            intent: ctx.intent,
            text: sentences
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            sentences,
            speech,
            citations: cites,
            found_only: true,
            reason: reason.map(str::to_string),
            dropped,
            scope: ctx.scope(),
            frontier: ctx.frontier(),
            preview: false,
        };
        Fallback { events, answer }
    }

    /// Hinweis für „Grenze unbekannt“.
    pub fn no_frontier(ctx: &Ctx<'_>) -> Fallback {
        Self::notice(
            ctx,
            CODE_NO_FRONTIER,
            if ctx.lang == Lang::De {
                "Ich kenne deine Leseposition noch nicht."
            } else {
                "I do not know your reading position yet."
            },
        )
    }

    /// Hinweis für „nichts gefunden“.
    pub fn no_hits(ctx: &Ctx<'_>) -> Fallback {
        let until = ctx.scope() == BookTalkScope::UntilPosition;
        let text = match (ctx.lang, until) {
            (Lang::De, true) => "Dazu finde ich bis hier nichts.",
            (Lang::De, false) => "Dazu finde ich im Buch nichts.",
            (Lang::En, true) => "I find nothing about that so far.",
            (Lang::En, false) => "I find nothing about that in the book.",
        };
        Self::notice(ctx, REASON_NO_HITS, text)
    }
}

/// Feldübergreifende Regeln, die das Schema nicht ausdrücken kann (Plan §7a).
/// Leer = gültig. Im Kern führt eine Verletzung zu found_only mit Grund
/// `llm_failed`.
pub fn violations(a: &BookTalkAnswer, index: &BookIndex) -> Vec<&'static str> {
    let mut v = Vec::new();
    if a.reason.is_some() && !a.found_only {
        v.push("reason ohne found_only");
    }
    if a.found_only && a.sentences.iter().any(|s| s.general) {
        v.push("found_only mit general-Satz");
    }
    if a.sentences
        .iter()
        .any(|s| s.general && !s.citations.is_empty())
    {
        v.push("general-Satz mit Beleg");
    }
    if !a.sentences.is_empty() {
        let joined = a
            .sentences
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if joined != a.text {
            v.push("text weicht von den Sätzen ab");
        }
        let cited: Vec<&BookTalkCitation> = a
            .sentences
            .iter()
            .flat_map(|s| s.citations.iter())
            .collect();
        if cited.len() != a.citations.len() || cited.iter().zip(&a.citations).any(|(x, y)| *x != y)
        {
            v.push("citations weichen von den Sätzen ab");
        }
        if a.sentences
            .iter()
            .enumerate()
            .any(|(i, s)| s.index as usize != i)
        {
            v.push("Satzindex nicht fortlaufend");
        }
    }
    for c in &a.citations {
        let (start, end) = (c.start as usize, c.end as usize);
        if end <= start || end > index.source_length {
            v.push("Beleg außerhalb des Textes");
            continue;
        }
        if c.quote.is_empty() || c.quote.encode_utf16().count() > QUOTE_MAX_CHARS {
            v.push("Zitatlänge");
        }
        if a.scope == BookTalkScope::UntilPosition && c.end > a.frontier {
            v.push("Beleg hinter der Grenze");
        }
        let Some(chunk) = index
            .chunks
            .iter()
            .find(|k| k.start <= start && end <= k.end)
        else {
            v.push("Beleg liegt in keinem Stück");
            continue;
        };
        if !chunk.text.contains(&c.quote) {
            v.push("Zitat nicht im Stück");
        }
    }
    v
}
