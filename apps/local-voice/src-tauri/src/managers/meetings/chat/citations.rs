//! Zitate per Konstruktion (M4 D5): Nachbearbeitung der Modellantwort.
//!
//! Das Modell soll nur Auszugs-IDs zitieren (`[Q3]`, `[Q3, Q7]`, `(Q3)`).
//! Kleine Modelle haengen gern die Segmentnummer der Auszugszeile an
//! (`[Q2:S44]`, `[Q1, S8]`, `(Q3:S12)`) oder zitieren nur sie (`[S8]`). Alle
//! Formen werden angenommen: `Sn` hinter einem `Qk` ist ein Hinweis, welche
//! Zeile des Auszugs gemeint ist (gilt nur, wenn `Sn` im Auszug steht); ein
//! reines `[Sn]` wird nur aufgeloest, wenn `Sn` genau ein Segment bezeichnet
//! (in allen gelesenen Auszuegen, die es enthalten, dieselbe Besprechung und
//! Epoche), sonst verworfen und gezaehlt. Danach wird jede ID auf ihren
//! Auszug abgebildet, je Satz auf das beste Segment verfeinert und fuer die
//! Anzeige in Reihenfolge des ersten Auftretens umnummeriert (`[1]`, `[2]`,
//! ...; gleiche Quelle = gleiche Nummer). Unbekannte IDs fallen weg und werden
//! gezaehlt; nackte `[3]` des Modells werden entfernt (die UI liest `[n]` als
//! Zitat). Im Antworttext bleibt nie ein roher Marker stehen. Rein, keine I/O.

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;
use regex::Regex;

use super::context::{clip_chars, content_terms, Excerpt};
use super::{ChunkSource, Citation};

/// Zitat-Marker. `q`: beginnt mit einer Auszugs-ID (`[Q3]`, `(Q3)`,
/// `[Q3, Q7]`, `[Q2:S44]`, `[Q1, S8]`, `(Q3:S12)`, `[Q1,2]`); `s`: nur
/// Segmente in eckigen Klammern (`[S8]`, `[S8, S9]`); `bare`: nackte Nummern
/// (`[3]`); `junk`: ein Marker aus Q/S/Ziffern/Trennern, der keine der
/// Formen trifft (`[Q1:S8-9]`), wird entfernt statt roh angezeigt. Bewusst
/// nur Q, S, Ziffern und Trenner: "(Q3 2026)" und "[Q3 Ergebnisse]" bleiben Text.
static MARKERS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"(?i)",
        r"(?P<q>[\[(]\s*Q\s*\d+(?:\s*[,;/:\-–]\s*[QS]\s*\d+|\s*[,;/]\s*\d+|\s+[QS]\s*\d+)*\s*[\])])",
        r"|(?P<s>\[\s*S\s*\d+(?:\s*[,;/:\-–]\s*[QS]\s*\d+|\s+[QS]\s*\d+)*\s*\])",
        r"|(?P<bare>\[\s*\d{1,3}(?:\s*,\s*\d{1,3})*\s*\])",
        r"|(?P<junk>\[\s*[QS]\s*\d[\s,;:/\-–\dQS.]{0,40}\])",
    ))
    .expect("Marker-Regex")
});
/// Bestandteile eines Markers: `Q3`, `S12` oder eine nackte Nummer.
static TOKEN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?P<kind>[QS])\s*(?P<n>\d+)|(?P<bare>\d+)").expect("Token-Regex")
});
static DISPLAY_MARK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[\d+\]").expect("Anzeige-Regex"));
static KEIN_BELEG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)kein[_ ]beleg").expect("KB-Regex"));
static THINK_BLOCK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<think>.*?</think>").expect("Think-Regex"));
/// Rest nach `[` oder `(`, der noch zu einem Marker werden kann.
static MARKER_TAIL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:[QS]\s*)?\d*(?:\s*[,;/:\-–]?\s*(?:[QS]\s*)?\d*)*\s*$")
        .expect("Tail-Regex")
});

const QUOTE_CHARS: usize = 200;
/// Laengster Satz, der fuer die Zeilenwahl vor einem Marker betrachtet wird.
const SENTENCE_MAX_CHARS: usize = 600;
const NOT_FOUND_TOKEN: &str = "kein_beleg";

/// Entfernt Denk-Bloecke (`<think>...</think>`), einen Rest vor einem
/// einzelnen `</think>` und alles ab einem nicht geschlossenen `<think>`.
pub fn strip_think(raw: &str) -> String {
    let mut s = THINK_BLOCK.replace_all(raw, "").into_owned();
    if let Some(i) = s.rfind("</think>") {
        s = s[i + "</think>".len()..].to_string();
    }
    if let Some(i) = s.find("<think>") {
        s.truncate(i);
    }
    s
}

/// Index der Zeile, die am besten zum Satz passt: groesste Ueberlappung der
/// Inhaltswoerter, Gleichstand -> fruehere Zeile (Zeilen sind zeitlich
/// geordnet). Transkript: nur Zeilen mit Segment; Notizen: nur Zeilen mit
/// Schluessel (gibt es keine, alle).
pub fn refine_line(sentence: &str, excerpt: &Excerpt) -> Option<usize> {
    let wanted: HashSet<String> = content_terms(sentence).into_iter().collect();
    let anchored: Vec<usize> = excerpt
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.segment_index.is_some() || l.ref_key.is_some())
        .map(|(i, _)| i)
        .collect();
    let candidates: Vec<usize> = if anchored.is_empty() {
        (0..excerpt.lines.len()).collect()
    } else {
        anchored
    };
    let mut best: Option<(usize, usize)> = None;
    for i in candidates {
        let have: HashSet<String> = content_terms(&excerpt.lines[i].content)
            .into_iter()
            .collect();
        let overlap = wanted.intersection(&have).count();
        if best.is_none_or(|(_, b)| overlap > b) {
            best = Some((i, overlap));
        }
    }
    best.map(|(i, _)| i)
}

/// Segment (Index in der Epoche des Auszugs), das den Satz am besten belegt.
/// Schnittstelle aus dem Entwurf (§4); `postprocess` nutzt `refine_line`
/// direkt, weil es auch Zitattext und Schluessel der Zeile braucht.
#[cfg_attr(not(test), allow(dead_code))]
pub fn refine_segment(sentence: &str, excerpt: &Excerpt) -> Option<u32> {
    refine_line(sentence, excerpt).and_then(|i| excerpt.lines[i].segment_index)
}

/// Satz, auf den sich ein Marker an Position `pos` bezieht: vom letzten
/// Satzende davor bis zum Marker (steht der Marker hinter dem Punkt, zaehlt
/// der Satz davor). Ein Punkt zwischen zwei Ziffern (`150.000`, `1.8`,
/// `22.10.`) beendet keinen Satz: "Die Reserve betraegt 150.000 Euro [Q1]"
/// war sonst der Satz "000 Euro" und fand keine Zeile (P4g, 12B: falscher
/// Notizblock zitiert). Andere Marker im Satz werden entfernt.
fn sentence_before(text: &str, pos: usize) -> String {
    let before = text[..pos].trim_end();
    let before = before
        .trim_end_matches(['.', '!', '?', ':', ';', ','])
        .trim_end();
    // Ein Riesen-"Satz" ohne Satzzeichen darf die Arbeit je Marker nicht
    // mit der Textlaenge wachsen lassen: hoechstens die letzten Zeichen.
    let start = before
        .char_indices()
        .rev()
        .nth(SENTENCE_MAX_CHARS)
        .map_or(0, |(i, _)| i);
    let window = &before[start..];
    let chars: Vec<(usize, char)> = window.char_indices().collect();
    let mut from = 0;
    for (k, &(i, c)) in chars.iter().enumerate().rev() {
        let ends = match c {
            '!' | '?' | '\n' => true,
            '.' => {
                let digit_before = k > 0 && chars[k - 1].1.is_ascii_digit();
                let digit_after = chars.get(k + 1).is_some_and(|(_, n)| n.is_ascii_digit());
                !(digit_before && digit_after)
            }
            _ => false,
        };
        if ends {
            from = i + c.len_utf8();
            break;
        }
    }
    MARKERS.replace_all(&window[from..], " ").into_owned()
}

type SourceKey = (String, &'static str, u32, Option<u32>, Option<String>, u32);

/// `hint`: Segment aus `[Qk:Sn]`; gilt nur, wenn die Zeile im Auszug steht,
/// sonst entscheidet der Satz.
fn citation_for(excerpt: &Excerpt, sentence: &str, hint: Option<u32>) -> (SourceKey, Citation) {
    let line = hint
        .and_then(|seg| {
            excerpt
                .lines
                .iter()
                .position(|l| l.segment_index == Some(seg))
        })
        .or_else(|| refine_line(sentence, excerpt))
        .map(|i| &excerpt.lines[i]);
    let transcript = excerpt.source == ChunkSource::Transcript;
    let segment_index = if transcript {
        line.and_then(|l| l.segment_index)
    } else {
        None
    };
    let start_ms = if transcript {
        line.and_then(|l| l.start_ms).or(excerpt.start_ms)
    } else {
        None
    };
    let ref_key = if transcript {
        None
    } else {
        line.and_then(|l| l.ref_key.clone())
            .or_else(|| excerpt.ref_keys.first().cloned())
    };
    let quote_src = line
        .map(|l| l.content.clone())
        .unwrap_or_else(|| excerpt.body());
    // Ohne Anker (kein Segment, kein Schluessel) ist der Auszug selbst die Quelle.
    let anchorless = if segment_index.is_none() && ref_key.is_none() {
        excerpt.qid
    } else {
        0
    };
    let key = (
        excerpt.meeting_id.clone(),
        excerpt.source.as_str(),
        excerpt.epoch,
        segment_index,
        ref_key.clone(),
        anchorless,
    );
    let citation = Citation {
        n: 0,
        meeting_id: excerpt.meeting_id.clone(),
        meeting_title: excerpt.meeting_title.clone(),
        started_at: excerpt.started_at,
        source: excerpt.source,
        epoch: excerpt.epoch,
        segment_index,
        start_ms,
        ref_key,
        quote: clip_chars(quote_src.trim(), QUOTE_CHARS),
    };
    (key, citation)
}

/// Eine aufgeloeste Zitatangabe eines Markers: der Auszug und, falls das
/// Modell eine Zeile genannt hat, deren Segment.
#[derive(Debug, PartialEq)]
struct MarkerRef {
    qid: u32,
    hint: Option<u32>,
}

/// Zerlegt einen Marker in seine Angaben.
///
/// - `Qk` -> Auszug k; eine nackte Nummer direkt hinter `Qk` (`[Q1,2]`) ist
///   ebenfalls eine Auszugs-ID.
/// - `Sn` hinter einem `Qk` desselben Markers: Hinweis fuer den naechsten
///   davorstehenden Auszug, der das Segment enthaelt (`[Q1, Q2:S8]`); steht
///   `Sn` in keinem, entfaellt der Hinweis (das Zitat bleibt).
/// - `Sn` ohne `Qk`: aufgeloest, wenn alle Auszuege, die `Sn` enthalten, zu
///   derselben Besprechung und Epoche gehoeren (dann ist es genau ein
///   Segment); sonst verworfen.
///
/// Unbekannte Auszuege und mehrdeutige `Sn` erhoehen `dropped`.
fn resolve_marker(
    marker: &str,
    by_qid: &HashMap<u32, &Excerpt>,
    by_segment: &HashMap<u32, Vec<&Excerpt>>,
    dropped: &mut u32,
) -> Vec<MarkerRef> {
    let mut refs: Vec<MarkerRef> = Vec::new();
    // Auszuege dieses Markers (Indizes in `refs`), naechster zuletzt.
    let mut quoted: Vec<usize> = Vec::new();
    // Letzte Angabe war ein (bekanntes oder unbekanntes) Qk.
    let mut after_q = false;
    let mut last_q_known = false;
    let mut seen_q = false;
    for caps in TOKEN.captures_iter(marker) {
        let number: Option<u32> = caps
            .name("n")
            .or_else(|| caps.name("bare"))
            .and_then(|n| n.as_str().parse().ok());
        let Some(number) = number else {
            *dropped += 1;
            continue;
        };
        let kind = caps
            .name("kind")
            .map(|k| k.as_str().to_ascii_uppercase())
            .unwrap_or_default();
        match kind.as_str() {
            "S" => {
                after_q = false;
                if seen_q {
                    // Hinweis fuer ein unbekanntes Qk: schon als verworfen gezaehlt.
                    if !last_q_known {
                        continue;
                    }
                    let target = quoted.iter().rev().copied().find(|&i| {
                        by_qid[&refs[i].qid]
                            .lines
                            .iter()
                            .any(|l| l.segment_index == Some(number))
                    });
                    if let Some(i) = target {
                        if refs[i].hint.is_none() {
                            refs[i].hint = Some(number);
                        } else {
                            let qid = refs[i].qid;
                            refs.push(MarkerRef {
                                qid,
                                hint: Some(number),
                            });
                            quoted.push(refs.len() - 1);
                        }
                    }
                } else {
                    let owners = by_segment.get(&number).map(Vec::as_slice).unwrap_or(&[]);
                    let same = owners.first().is_some_and(|first| {
                        owners
                            .iter()
                            .all(|o| o.meeting_id == first.meeting_id && o.epoch == first.epoch)
                    });
                    match owners.first() {
                        Some(first) if same => refs.push(MarkerRef {
                            qid: first.qid,
                            hint: Some(number),
                        }),
                        _ => *dropped += 1,
                    }
                }
            }
            "Q" | "" => {
                // Eine nackte Nummer zaehlt nur direkt hinter einem Qk.
                if kind.is_empty() && !after_q {
                    *dropped += 1;
                    continue;
                }
                seen_q = true;
                after_q = true;
                if by_qid.contains_key(&number) {
                    last_q_known = true;
                    refs.push(MarkerRef {
                        qid: number,
                        hint: None,
                    });
                    quoted.push(refs.len() - 1);
                } else {
                    last_q_known = false;
                    *dropped += 1;
                }
            }
            _ => {}
        }
    }
    refs
}

/// Nachbearbeitung einer Antwort gegen die Auszuege, die das Modell gesehen
/// hat. Liefert (Text mit `[n]`, Zitate nach `n`, verworfene Zitate,
/// `not_found`). Bei `not_found` sind Text und Zitate leer (die UI zeigt den
/// uebersetzten Hinweis).
pub fn postprocess(raw: &str, excerpts: &[Excerpt]) -> (String, Vec<Citation>, u32, bool) {
    let text = strip_think(raw);
    let text = text.trim();
    let by_qid: HashMap<u32, &Excerpt> = excerpts.iter().map(|e| (e.qid, e)).collect();
    let mut by_segment: HashMap<u32, Vec<&Excerpt>> = HashMap::new();
    for ex in excerpts
        .iter()
        .filter(|e| e.source == ChunkSource::Transcript)
    {
        for seg in ex.lines.iter().filter_map(|l| l.segment_index) {
            let owners = by_segment.entry(seg).or_default();
            if !owners.iter().any(|o| o.qid == ex.qid) {
                owners.push(ex);
            }
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut dropped = 0u32;
    let mut numbers: HashMap<SourceKey, u32> = HashMap::new();
    let mut citations: Vec<Citation> = Vec::new();
    // Ende des vorigen Q-Markers, sein Satz und die dort gezeigten Nummern:
    // direkt folgende Marker (`[Q1][Q2]`) gehoeren zum selben Satz, und eine
    // Nummer erscheint dort nur einmal.
    let mut prev: Option<(usize, String, Vec<u32>)> = None;

    for caps in MARKERS.captures_iter(text) {
        let m = caps.get(0).expect("ganzer Treffer");
        out.push_str(&text[last..m.start()]);
        let mut shown: Vec<u32> = Vec::new();
        if caps.name("bare").is_some() {
            dropped += TOKEN.find_iter(m.as_str()).count() as u32;
        } else if caps.name("junk").is_some() {
            dropped += 1;
        } else {
            let refs = resolve_marker(m.as_str(), &by_qid, &by_segment, &mut dropped);
            let (sentence, mut group) = match prev.take() {
                Some((end, s, shown_before)) if text[end..m.start()].trim().is_empty() => {
                    (s, shown_before)
                }
                _ => (sentence_before(text, m.start()), Vec::new()),
            };
            let mut seen_refs = HashSet::new();
            for r in refs {
                if !seen_refs.insert((r.qid, r.hint)) {
                    continue;
                }
                let excerpt = by_qid[&r.qid];
                let (key, mut citation) = citation_for(excerpt, &sentence, r.hint);
                let next = numbers.len() as u32 + 1;
                let n = *numbers.entry(key).or_insert_with(|| {
                    citation.n = next;
                    citations.push(citation);
                    next
                });
                if !group.contains(&n) {
                    group.push(n);
                    shown.push(n);
                }
            }
            prev = Some((m.end(), sentence, group));
        }
        if shown.is_empty() {
            // Weggefallener Marker: kein doppelter Leerraum, kein " ." zurueck.
            let next = text[m.end()..].chars().next();
            if next.is_none_or(|c| c.is_whitespace() || ".,;:!?)".contains(c)) {
                let trimmed = out.trim_end_matches([' ', '\t']).len();
                out.truncate(trimmed);
            }
        } else {
            for n in shown {
                out.push_str(&format!("[{n}]"));
            }
        }
        last = m.end();
    }
    out.push_str(&text[last..]);

    let core: String = DISPLAY_MARK
        .replace_all(&out, " ")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == ' ')
        .collect();
    let core = core
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase();
    let says_not_found = KEIN_BELEG.is_match(&out);
    if core == NOT_FOUND_TOKEN || (says_not_found && citations.is_empty()) {
        return (String::new(), Vec::new(), dropped, true);
    }
    let out = if says_not_found {
        KEIN_BELEG.replace_all(&out, "–").into_owned()
    } else {
        out
    };
    (out.trim().to_string(), citations, dropped, false)
}

/// Filter fuer die gestreamten Stuecke: zeigt nur Text, der sich nicht mehr
/// aendert. Denk-Bloecke und Marker (`[Q3]`, `[3]`) bleiben verborgen, ein
/// angefangener Marker wird zurueckgehalten, ebenso eine Antwort, die mit
/// `KEIN_BELEG` beginnt (die UI zeigt dafuer den Hinweis der Endantwort).
#[derive(Default)]
pub struct DeltaFilter {
    raw: String,
    emitted: String,
}

impl DeltaFilter {
    pub fn push(&mut self, delta: &str) -> Option<String> {
        self.raw.push_str(delta);
        let visible = stream_visible(&self.raw);
        if visible.len() > self.emitted.len() && visible.starts_with(&self.emitted) {
            let out = visible[self.emitted.len()..].to_string();
            self.emitted = visible;
            Some(out)
        } else {
            None
        }
    }
}

fn stream_visible(raw: &str) -> String {
    let mut s = strip_think(raw);
    // Ein angefangenes `<think>` am Ende zurueckhalten.
    for k in (1.."<think>".len()).rev() {
        if s.ends_with(&"<think>"[..k]) {
            s.truncate(s.len() - k);
            break;
        }
    }
    let mut s = MARKERS.replace_all(&s, "").into_owned();
    if let Some(open) = s.rfind(['[', '(']) {
        let tail = &s[open + 1..];
        if tail.len() < 48 && MARKER_TAIL.is_match(tail) {
            s.truncate(open);
        }
    }
    let visible = s.trim_start();
    let lower = visible.to_lowercase();
    let token = "kein_beleg";
    if lower.starts_with(token) || (!lower.is_empty() && token.starts_with(lower.trim_end())) {
        return String::new();
    }
    visible.to_string()
}

#[cfg(test)]
mod tests {
    use super::super::context::tests::meeting;
    use super::super::context::{transcript_blocks, ExcerptLine};
    use super::*;
    use crate::managers::meetings::store::StoredSegment;

    fn seg(i: u32, start_s: u64, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: i,
            text: text.into(),
            start_ms: start_s * 1_000,
            end_ms: start_s * 1_000 + 3_000,
            channel: 0,
            speaker_index: None,
            words: None,
        }
    }

    /// Ein Auszug je Eintrag aus `(meeting, [(Segment, Text)])`, IDs ab 1.
    fn excerpts(spec: &[(&str, &[(u32, &str)])]) -> Vec<Excerpt> {
        spec.iter()
            .enumerate()
            .map(|(i, (m, segs))| {
                let segs: Vec<StoredSegment> = segs
                    .iter()
                    .map(|(ix, t)| seg(*ix, *ix as u64 * 10, t))
                    .collect();
                let mut ex = transcript_blocks(&meeting(m), &segs, 3, 100_000)
                    .pop()
                    .unwrap();
                ex.qid = i as u32 + 1;
                ex
            })
            .collect()
    }

    fn numbers(cites: &[Citation]) -> Vec<(u32, Option<u32>)> {
        cites.iter().map(|c| (c.n, c.segment_index)).collect()
    }

    #[test]
    fn unknown_ids_dropped_and_counted() {
        let ex = excerpts(&[
            ("a", &[(1, "Budget genehmigt")]),
            ("a", &[(2, "Termin Freitag")]),
        ]);
        let (text, cites, dropped, not_found) = postprocess(
            "Das Budget ist genehmigt [Q1]. Es regnet [Q9]. Termin am Freitag [Q2, Q7].",
            &ex,
        );
        assert!(!not_found);
        assert_eq!(dropped, 2, "Q9 und Q7 gibt es nicht");
        assert_eq!(numbers(&cites), vec![(1, Some(1)), (2, Some(2))]);
        assert_eq!(
            text,
            "Das Budget ist genehmigt [1]. Es regnet. Termin am Freitag [2]."
        );
        // Nur unbekannte IDs: kein Zitat, alles gezaehlt.
        let (text, cites, dropped, not_found) = postprocess("Satz [Q5][Q6].", &ex);
        assert_eq!(
            (text.as_str(), cites.len(), dropped, not_found),
            ("Satz.", 0, 2, false)
        );
    }

    #[test]
    fn renumbers_in_order_of_first_use() {
        let ex = excerpts(&[
            ("a", &[(1, "Kosten steigen")]),
            ("b", &[(5, "Anna uebernimmt")]),
            ("a", &[(9, "Termin verschoben")]),
        ]);
        let (text, cites, dropped, _) = postprocess(
            "Der Termin ist verschoben [Q3]. Die Kosten steigen [Q1]. Nochmal der Termin [Q3]. Anna [Q2].",
            &ex,
        );
        assert_eq!(dropped, 0);
        assert_eq!(
            text,
            "Der Termin ist verschoben [1]. Die Kosten steigen [2]. Nochmal der Termin [1]. Anna [3]."
        );
        let got: Vec<(u32, &str, Option<u32>)> = cites
            .iter()
            .map(|c| (c.n, c.meeting_id.as_str(), c.segment_index))
            .collect();
        assert_eq!(
            got,
            vec![(1, "a", Some(9)), (2, "a", Some(1)), (3, "b", Some(5))]
        );
        assert_eq!(cites[0].epoch, 3);
        assert_eq!(cites[0].start_ms, Some(90_000));
        assert_eq!(cites[0].quote, "Termin verschoben");
        assert_eq!(cites[0].meeting_title, "Besprechung a");
    }

    #[test]
    fn the_same_segment_from_two_excerpts_gets_one_number() {
        // Ueberlappende Auszuege (Nachbar-Chunks) enthalten dasselbe Segment 4.
        let ex = excerpts(&[
            ("a", &[(3, "Wetter"), (4, "Budget genehmigt")]),
            ("a", &[(4, "Budget genehmigt"), (5, "Urlaub")]),
        ]);
        let (text, cites, _, _) = postprocess("Budget genehmigt [Q1][Q2].", &ex);
        assert_eq!(text, "Budget genehmigt [1].");
        assert_eq!(numbers(&cites), vec![(1, Some(4))]);
    }

    #[test]
    fn marker_forms_are_recognised() {
        let ex = excerpts(&[("a", &[(1, "eins")]), ("b", &[(2, "zwei")])]);
        for raw in [
            "Satz [Q1, Q2].",
            "Satz [Q1][Q2].",
            "Satz (Q1) (Q2).",
            "Satz [q1; Q2].",
            "Satz [Q1,2].",
            "Satz [ Q1 / Q2 ].",
        ] {
            let (text, cites, dropped, _) = postprocess(raw, &ex);
            assert_eq!(cites.len(), 2, "{raw}");
            assert_eq!(dropped, 0, "{raw}");
            assert!(text.starts_with("Satz [1]"), "{raw} -> {text}");
            assert!(text.contains("[2]"), "{raw} -> {text}");
        }
    }

    #[test]
    fn bare_numbers_of_the_model_are_removed_and_counted() {
        let ex = excerpts(&[("a", &[(1, "eins")])]);
        let (text, cites, dropped, _) =
            postprocess("Laut Quelle [3] gilt das [Q1]. Siehe [4, 5].", &ex);
        assert_eq!(text, "Laut Quelle gilt das [1]. Siehe.");
        assert_eq!(cites.len(), 1);
        assert_eq!(dropped, 3);
        // Jahreszahlen in Klammern bleiben.
        let (text, _, dropped, _) = postprocess("Seit [2024] so [Q1].", &ex);
        assert_eq!(text, "Seit [2024] so [1].");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn kein_beleg_sets_not_found() {
        let ex = excerpts(&[("a", &[(1, "eins")])]);
        for raw in [
            "KEIN_BELEG",
            "  kein_beleg. \n",
            "KEIN_BELEG.",
            "Kein Beleg!",
            "<think>\n\n</think>\n\nKEIN_BELEG",
            "Dazu steht in den Auszuegen nichts. KEIN_BELEG",
            "KEIN_BELEG [Q9]",
        ] {
            let (text, cites, _, not_found) = postprocess(raw, &ex);
            assert!(not_found, "{raw:?}");
            assert!(text.is_empty() && cites.is_empty(), "{raw:?}");
        }
        // Teilantwort mit Beleg und einer Luecke: bleibt eine Antwort.
        let (text, cites, _, not_found) =
            postprocess("Das Budget steht [Q1]. Zum Termin: KEIN_BELEG", &ex);
        assert!(!not_found);
        assert_eq!(cites.len(), 1);
        assert_eq!(text, "Das Budget steht [1]. Zum Termin: –");
    }

    #[test]
    fn an_answer_without_citations_is_not_not_found() {
        let ex = excerpts(&[("a", &[(1, "eins")])]);
        let (text, cites, dropped, not_found) = postprocess("Eine Behauptung ohne Beleg.", &ex);
        assert_eq!(text, "Eine Behauptung ohne Beleg.");
        assert!(cites.is_empty());
        assert_eq!(dropped, 0);
        assert!(!not_found, "ohne Zitat ist 'uncited', nicht 'not_found'");
    }

    #[test]
    fn refine_picks_overlapping_segment() {
        let ex = &excerpts(&[(
            "a",
            &[
                (1, "Wir reden kurz über das Wetter."),
                (2, "Das Budget beträgt 5000 Euro für das Projekt."),
                (3, "Das Wetter bleibt schön."),
            ],
        )])[0];
        assert_eq!(
            refine_segment("Das Budget liegt bei 5000 Euro", ex),
            Some(2)
        );
        assert_eq!(
            refine_segment("Wie wird das Wetter? Es bleibt schön", ex),
            Some(3)
        );
        // Gleichstand (hier: keine Ueberlappung) -> fruehestes Segment.
        assert_eq!(refine_segment("Ganz anderes Thema", ex), Some(1));
        // Nur Segmente des Auszugs kommen in Frage.
        assert!(ex
            .lines
            .iter()
            .any(|l| l.segment_index == refine_segment("Budget", ex)));
        // Im Text: der Satz vor dem Marker entscheidet, nicht der ganze Text.
        let mut single = ex.clone();
        single.qid = 1;
        let (_, cites, _, _) = postprocess(
            "Das Wetter bleibt schön [Q1]. Das Budget beträgt 5000 Euro [Q1].",
            &[single],
        );
        assert_eq!(numbers(&cites), vec![(1, Some(3)), (2, Some(2))]);
    }

    #[test]
    fn a_dot_between_digits_does_not_end_the_sentence_that_picks_the_line() {
        // 12B (P4g): "150.000" machte aus dem Satz "000 Euro"; ohne Ueberlappung
        // gewann die fruehste Zeile ("Status") statt der Zeile mit der Reserve.
        let ex = excerpts(&[(
            "a",
            &[
                (1, "Status"),
                (2, "Fertigung hat drei Wochen Verzug"),
                (3, "Budget: 1,1 von 1,8 Mio. verbraucht, Reserve 150 T€"),
                (4, "Nur 19 von 32 Key-Usern geschult"),
            ],
        )]);
        for raw in [
            "Die Budgetreserve beträgt 150.000 Euro [Q1].",
            "Die Budgetreserve beträgt 150.000 Euro. [Q1]",
            "Gut so. Die Budgetreserve beträgt 1.500 Euro [Q1].",
            "Das Budget: 1.8 Mio. und die Reserve [Q1]",
        ] {
            let (_, cites, _, _) = postprocess(raw, &ex);
            assert_eq!(numbers(&cites), vec![(1, Some(3))], "{raw}");
        }
        // Ein Punkt am Satzende trennt weiter (auch hinter einer Zahl mit Leerzeichen).
        let (_, cites, _, _) = postprocess(
            "Fertigung hat drei Wochen Verzug [Q1]. Nur 19 von 32 Key-Usern geschult [Q1].",
            &ex,
        );
        assert_eq!(numbers(&cites), vec![(1, Some(2)), (2, Some(4))]);
        assert_eq!(
            sentence_before("Er nennt 150.000 Euro [Q1]", 21),
            "Er nennt 150.000 Euro"
        );
        assert_eq!(
            sentence_before("Ende. Dann 22.10.2026 folgt", 21),
            " Dann 22.10.2026"
        );
    }

    #[test]
    fn notes_citations_carry_the_block_key() {
        let mut ex = excerpts(&[("a", &[(1, "x")])]).pop().unwrap();
        ex.source = ChunkSource::UserNotes;
        ex.lines = vec![
            ExcerptLine {
                segment_index: None,
                start_ms: Some(1_000),
                ref_key: Some("n1".into()),
                text: "# Budget".into(),
                content: "Budget".into(),
            },
            ExcerptLine {
                segment_index: None,
                start_ms: None,
                ref_key: Some("n2".into()),
                text: "- Angebot an Nordlicht schicken".into(),
                content: "Angebot an Nordlicht schicken".into(),
            },
        ];
        ex.ref_keys = vec!["n1".into(), "n2".into()];
        let (_, cites, _, _) = postprocess("Das Angebot geht an Nordlicht [Q1].", &[ex]);
        assert_eq!(cites[0].ref_key.as_deref(), Some("n2"));
        assert_eq!(cites[0].segment_index, None);
        assert_eq!(cites[0].start_ms, None);
        assert_eq!(cites[0].quote, "Angebot an Nordlicht schicken");
    }

    #[test]
    fn a_long_quote_is_clipped() {
        let long = "Budget ".repeat(80);
        let ex = excerpts(&[("a", &[(1, long.as_str())])]);
        let (_, cites, _, _) = postprocess("Budget [Q1].", &ex);
        assert_eq!(cites[0].quote.chars().count(), 200);
        assert!(cites[0].quote.ends_with('…'));
    }

    #[test]
    fn think_blocks_are_removed() {
        assert_eq!(strip_think("<think>abc</think>Antwort"), "Antwort");
        assert_eq!(strip_think("Vorlauf</think>Antwort"), "Antwort");
        assert_eq!(strip_think("Antwort<think>abgeschnitten"), "Antwort");
        assert_eq!(strip_think("nur Text"), "nur Text");
    }

    #[test]
    fn the_stream_filter_hides_markers_and_thinking_across_chunks() {
        let mut f = DeltaFilter::default();
        let mut shown = String::new();
        for piece in [
            "<thi",
            "nk>\n\n</think>\n\nDas Bud",
            "get steht [",
            "Q1",
            "2]. Weiter",
            " [3] hier",
        ] {
            if let Some(d) = f.push(piece) {
                assert!(!d.contains('['), "Marker sichtbar: {d:?}");
                assert!(!d.contains("think"), "Denken sichtbar: {d:?}");
                shown.push_str(&d);
            }
        }
        assert_eq!(shown, "Das Budget steht . Weiter  hier");
    }

    #[test]
    fn the_stream_filter_holds_back_kein_beleg() {
        let mut f = DeltaFilter::default();
        let mut shown = String::new();
        for piece in ["KEIN", "_BEL", "EG"] {
            if let Some(d) = f.push(piece) {
                shown.push_str(&d);
            }
        }
        assert_eq!(shown, "", "kein Aufblitzen des Codes");
        let mut f = DeltaFilter::default();
        assert_eq!(f.push("Kei"), None);
        assert_eq!(f.push("ne Ahnung"), Some("Keine Ahnung".into()));
    }

    // ---- P4g: Segment-Formen kleiner Modelle ([Q2:S44], [Q1, S8], [S8]) ----

    /// Auszug 1 mit den Segmenten 7 (Wetter), 8 (Budget), 9 (Urlaub).
    fn weather_budget() -> Vec<Excerpt> {
        excerpts(&[(
            "a",
            &[
                (7, "Das Wetter ist schön"),
                (8, "Das Budget beträgt 5000 Euro"),
                (9, "Der Urlaub beginnt im August"),
            ],
        )])
    }

    fn raw_marker_left(text: &str) -> bool {
        Regex::new(r"(?i)[\[(]\s*[QS]\s*\d").unwrap().is_match(text)
    }

    #[test]
    fn a_segment_after_the_q_id_picks_that_line_in_every_form() {
        let ex = weather_budget();
        for marker in [
            "[Q1:S8]",
            "[Q1: S8]",
            "[Q1, S8]",
            "[Q1 : S8]",
            "[q1;s8]",
            "[Q1 S8]",
            "[Q1/S8]",
            "[Q1-S8]",
            "(Q1:S8)",
            "(Q1, S8)",
        ] {
            // Ohne Hinweis waehlt der Satz das Wetter-Segment 7.
            let raw = format!("Das Wetter wird schön {marker}.");
            let (text, cites, dropped, not_found) = postprocess(&raw, &ex);
            assert!(!not_found, "{marker}");
            assert_eq!(dropped, 0, "{marker}");
            assert_eq!(numbers(&cites), vec![(1, Some(8))], "{marker}");
            assert_eq!(text, "Das Wetter wird schön [1].", "{marker}");
        }
        // Der Hinweis der Kontrolle: derselbe Satz ohne Hinweis landet bei 7.
        let (_, cites, _, _) = postprocess("Das Wetter wird schön [Q1].", &ex);
        assert_eq!(numbers(&cites), vec![(1, Some(7))]);
    }

    #[test]
    fn a_segment_hint_outside_the_excerpt_is_ignored_but_the_citation_stays() {
        let ex = weather_budget();
        let (text, cites, dropped, _) = postprocess("Das Wetter wird schön [Q1:S99].", &ex);
        assert_eq!(dropped, 0);
        assert_eq!(numbers(&cites), vec![(1, Some(7))], "der Satz entscheidet");
        assert_eq!(text, "Das Wetter wird schön [1].");
    }

    #[test]
    fn a_hint_for_an_unknown_excerpt_is_dropped_with_it() {
        let ex = weather_budget();
        for raw in [
            "Das Budget steht [Q9:S8].",
            "Das Budget steht [Q9, S8].",
            "Das Budget steht (Q9:S8).",
        ] {
            let (text, cites, dropped, _) = postprocess(raw, &ex);
            assert_eq!(dropped, 1, "{raw}: S8 ist der Hinweis von Q9, kein Zitat");
            assert!(cites.is_empty(), "{raw}");
            assert_eq!(text, "Das Budget steht.", "{raw}");
        }
        // Ein bekanntes und ein unbekanntes Qk: der Hinweis gehoert zum letzten.
        let (text, cites, dropped, _) = postprocess("Das Budget steht [Q1, Q9:S8].", &ex);
        assert_eq!(dropped, 1);
        assert_eq!(
            numbers(&cites),
            vec![(1, Some(8))],
            "Q1 ohne Hinweis, der Satz waehlt"
        );
        assert_eq!(text, "Das Budget steht [1].");
    }

    #[test]
    fn several_ids_and_segments_in_one_marker_are_paired_up() {
        let ex = excerpts(&[
            ("a", &[(7, "Wetter"), (8, "Budget genehmigt")]),
            ("b", &[(3, "Termin Freitag"), (4, "Anna uebernimmt")]),
        ]);
        let (text, cites, dropped, _) = postprocess("Satz [Q1:S8, Q2:S4].", &ex);
        assert_eq!(dropped, 0);
        assert_eq!(numbers(&cites), vec![(1, Some(8)), (2, Some(4))]);
        assert_eq!(text, "Satz [1][2].");
        // Das Segment gehoert zum davorstehenden Auszug, der es enthaelt.
        let (_, cites, dropped, _) = postprocess("Satz [Q1, Q2, S8].", &ex);
        assert_eq!(dropped, 0);
        assert_eq!(
            cites[0].segment_index,
            Some(8),
            "S8 steht in Q1, nicht in Q2"
        );
        assert_eq!(cites.len(), 2);
        // Zwei Segmente desselben Auszugs: zwei Zitate.
        let (text, cites, _, _) = postprocess("Satz [Q1:S7, S8].", &ex);
        assert_eq!(numbers(&cites), vec![(1, Some(7)), (2, Some(8))]);
        assert_eq!(text, "Satz [1][2].");
    }

    #[test]
    fn a_bare_segment_is_resolved_when_it_names_exactly_one_segment() {
        let ex = weather_budget();
        for raw in [
            "Das Budget steht [S8].",
            "Das Budget steht [ S8 ].",
            "Das Budget steht [s8].",
        ] {
            let (text, cites, dropped, _) = postprocess(raw, &ex);
            assert_eq!(dropped, 0, "{raw}");
            assert_eq!(numbers(&cites), vec![(1, Some(8))], "{raw}");
            assert_eq!(text, "Das Budget steht [1].", "{raw}");
        }
        // Eine Liste von Segmenten: je eines.
        let (text, cites, _, _) = postprocess("Budget und Urlaub [S8, S9].", &ex);
        assert_eq!(numbers(&cites), vec![(1, Some(8)), (2, Some(9))]);
        assert_eq!(text, "Budget und Urlaub [1][2].");
        // Dasselbe Segment in zwei ueberlappenden Auszuegen einer Besprechung
        // ist eindeutig: eine Nummer.
        let overlap = excerpts(&[
            ("a", &[(3, "Wetter"), (4, "Budget genehmigt")]),
            ("a", &[(4, "Budget genehmigt"), (5, "Urlaub")]),
        ]);
        let (text, cites, dropped, _) = postprocess("Budget genehmigt [S4].", &overlap);
        assert_eq!((dropped, numbers(&cites)), (0, vec![(1, Some(4))]));
        assert_eq!(text, "Budget genehmigt [1].");
    }

    #[test]
    fn an_ambiguous_or_unknown_bare_segment_is_dropped_and_counted() {
        // S8 kommt in zwei Besprechungen vor: nicht eindeutig.
        let two = excerpts(&[("a", &[(8, "Budget")]), ("b", &[(8, "Termin")])]);
        let (text, cites, dropped, not_found) = postprocess("Es geht um Geld [S8].", &two);
        assert!(!not_found);
        assert!(cites.is_empty());
        assert_eq!(dropped, 1);
        assert_eq!(text, "Es geht um Geld.");
        // Unbekanntes Segment.
        let (text, cites, dropped, _) = postprocess("Es geht um Geld [S99] und [Q1].", &two);
        assert_eq!(dropped, 1);
        assert_eq!(cites.len(), 1);
        assert_eq!(text, "Es geht um Geld und [1].");
        // Gleiche Nummer in verschiedenen Epochen derselben Besprechung ist ebenfalls mehrdeutig.
        let mut epochs = excerpts(&[("a", &[(8, "alt")]), ("a", &[(8, "neu")])]);
        epochs[1].epoch = 5;
        let (_, cites, dropped, _) = postprocess("Satz [S8].", &epochs);
        assert_eq!((cites.len(), dropped), (0, 1));
        // Notizen-Auszuege haben keine Segmente: `[S8]` findet dort nichts.
        let mut notes = weather_budget();
        notes[0].source = ChunkSource::UserNotes;
        let (_, cites, dropped, _) = postprocess("Satz [S8].", &notes);
        assert_eq!((cites.len(), dropped), (0, 1));
    }

    #[test]
    fn no_raw_marker_survives_in_the_answer_text() {
        let ex = weather_budget();
        for raw in [
            "Satz [Q1:S8-9].",
            "Satz [Q1-Q2].",
            "Satz [S 8].",
            "Satz [Q 1 : S 8].",
            "Satz [Q1;S8;S9].",
            "Satz [S8] und [Q1:S9] und (Q1, S7).",
            "Satz [Q1:S8, 12].",
            "Satz [Q12:S8].",
            "Satz [S8-S9].",
            "Satz [Q1 : S 8 – S9].",
        ] {
            let (text, _, _, _) = postprocess(raw, &ex);
            assert!(!raw_marker_left(&text), "{raw} -> {text}");
            assert!(text.starts_with("Satz"), "{raw} -> {text}");
        }
        // Text, der nur so aussieht, bleibt.
        let (text, cites, dropped, _) = postprocess(
            "Im Plan (Q3 2026) und [Q3 Ergebnisse] und [S-Bahn] [Q1].",
            &ex,
        );
        assert_eq!(
            text,
            "Im Plan (Q3 2026) und [Q3 Ergebnisse] und [S-Bahn] [1]."
        );
        assert_eq!((cites.len(), dropped), (1, 0));
    }

    #[test]
    fn the_stream_filter_hides_the_segment_forms_across_chunks() {
        for pieces in [
            vec!["Das Budget steht [", "Q2", ":S4", "4]. Weiter"],
            vec!["Das Budget steht [Q1", ", S", "8]. Weiter"],
            vec!["Das Budget steht [S", "8]. Weiter"],
            vec!["Das Budget steht (Q3:", "S12). Weiter"],
            vec!["Das Budget steht [Q1:S8-", "9]. Weiter"],
        ] {
            let mut f = DeltaFilter::default();
            let mut shown = String::new();
            for piece in &pieces {
                if let Some(d) = f.push(piece) {
                    assert!(!raw_marker_left(&d), "Marker sichtbar: {d:?} ({pieces:?})");
                    shown.push_str(&d);
                }
            }
            assert_eq!(shown, "Das Budget steht . Weiter", "{pieces:?}");
        }
        // Ein Text in Klammern, der mit S anfaengt, blitzt nur kurz nicht auf.
        let mut f = DeltaFilter::default();
        assert_eq!(f.push("Er kommt (S"), Some("Er kommt ".into()));
        assert_eq!(f.push("amstag)."), Some("(Samstag).".into()));
    }

    #[test]
    fn a_huge_answer_full_of_markers_is_processed_at_once() {
        let ex = weather_budget();
        let raw = "Das Budget steht [Q1:S8] und [S9], [Q9] ".repeat(400);
        assert!(raw.len() > 15_000);
        let started = std::time::Instant::now();
        let (text, cites, dropped, _) = postprocess(&raw, &ex);
        assert!(started.elapsed().as_secs() < 5, "lineare Laufzeit");
        assert_eq!(cites.len(), 2, "gleiche Quelle = gleiche Nummer");
        assert_eq!(dropped, 400);
        assert!(!raw_marker_left(&text));
    }
}
