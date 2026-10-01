//! D5 (Goal issues-abschluss #70, M7 = #69): Folientext in die Prompts von
//! Protokoll, KI-Notizen und Chat einweben. Rein, ohne I/O.
//!
//! Grundlage ist `koordination/bild-video/spike/spike-bericht.md`, Abschnitt
//! "Nutzung im Protokoll":
//!
//! - Jede nicht ausgeblendete Folie MIT Text wird zu einer Zeile an der Stelle des
//!   Transkripts, an der sie erstmals zu sehen war (`[Folie 7 · 04:12] <Text>`);
//!   kehrt der Vortrag spaeter zu ihr zurueck, steht dort eine kurze Marke statt
//!   des ganzen Textes. Die Bildbeschreibung (D3) steht nur gekennzeichnet dahinter
//!   (`{Bild: ...}`): sie hat Ablesefehler (R2) und ist kein Zahlenlieferant.
//! - Die Folien duerfen das Transkript nicht verdraengen: ihr Anteil ist begrenzt
//!   ([`slide_budget_chars`]); passt er nicht, gilt eine feste Kuerzungsleiter
//!   ([`LEVELS`]): zuerst die Beschreibungen, dann die Rueckkehr-Marken, dann der
//!   Folientext (300, 120 Zeichen), zuletzt Folien vom Ende her. Der Folientext
//!   faellt als Letztes.
//! - Belege: im Protokoll steht `[F7]` hinter der Aussage, in den KI-Notizen
//!   `"sources": ["S12", "F7"]`. Beide werden gegen die Folien geprueft, die wirklich
//!   im Prompt standen ([`normalize_tags`], [`SlideContext::numbers`]); unbekannte
//!   Belege werden entfernt und gezaehlt, nie still uebernommen.
//!
//! Ohne Folien (keine, alle ausgeblendet, keine mit Text) liefert [`prepare`]
//! `None`, und die Aufrufer gehen den alten Weg: Prompts BYTEGLEICH wie vor D5.
//!
//! # Fehlerfaelle und Absicherung
//!
//! | Fall | Verhalten | Test |
//! |---|---|---|
//! | Keine Folie / nur ausgeblendete / ohne Text | kein Einweben, Prompts wie vorher | `without_usable_slides_nothing_changes` |
//! | Zu viel Folientext | Kuerzungsleiter, Anteil begrenzt, Zaehler im Bericht | `the_budget_ladder_shortens_in_the_documented_order`, `slides_never_exceed_their_share_of_the_budget` |
//! | Bildbeschreibung | nur als `{Bild: ...}`, faellt zuerst | `a_description_is_marked_and_dropped_first` |
//! | Folientext enthaelt `[F3]`, `S12` oder `{Bild:` | eckige und geschweifte Klammern werden zu runden: kein gefaelschter Beleg | `slide_text_cannot_forge_a_tag_or_a_marker` |
//! | Das Modell belegt eine Folie, die nicht im Prompt stand | Beleg entfernt und gezaehlt | `unknown_tags_are_dropped_and_counted` |
//! | Folien aendern sich waehrend des Laufs | der Lauf liest EINEN Schnappschuss und prueft gegen ihn | (Aufrufer: `minutes`, `enhance`) |

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;
use regex::Regex;

use super::store::MeetingSlide;
use crate::managers::meetings::llm_call::mm_ss;
use crate::managers::meetings::notes::budget::slide_budget_chars;

/// Hoechstzeichen des Folientextes einer Zeile (Spike: 600).
pub const TEXT_MAX_CHARS: usize = 600;
/// Hoechstzeichen der Bildbeschreibung einer Zeile.
pub const DESCRIPTION_MAX_CHARS: usize = 240;
/// Hoechstzahl Folien-Quellen, die ein Dokument in seiner Herkunft nennt.
pub const MAX_SOURCE_REFS: usize = 150;
/// Eine Folie mit dieser Kennung (`kind`) gilt als ohne Text (D2: weniger als drei
/// Woerter, Sprecherbild, Videokachel) und wird nicht eingewoben.
pub const KIND_NO_TEXT: &str = "ohne_text";

/// Wie eine Folienzeile aussieht: das Protokoll liest `[Folie 7 · 04:12] Text`
/// (Zeitmarke wie die Transkriptzeilen), die KI-Notizen `F7 [04:12] Folie: Text`
/// (Kennung `F<n>` neben `S<n>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Minutes,
    Notes,
}

/// Eine Stufe der Kuerzungsleiter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    /// Hoechstzeichen des Folientextes.
    pub text_max: usize,
    /// Bildbeschreibungen behalten?
    pub descriptions: bool,
    /// Rueckkehr-Marken ("wieder gezeigt") behalten?
    pub again: bool,
}

/// Die Leiter, von voll nach knapp. Jede Stufe nimmt mehr weg als die davor; wer
/// auch die letzte nicht erreicht, verliert Folien vom Ende her ([`fit`]).
pub const LEVELS: [Level; 5] = [
    Level {
        text_max: TEXT_MAX_CHARS,
        descriptions: true,
        again: true,
    },
    Level {
        text_max: TEXT_MAX_CHARS,
        descriptions: false,
        again: true,
    },
    Level {
        text_max: TEXT_MAX_CHARS,
        descriptions: false,
        again: false,
    },
    Level {
        text_max: 300,
        descriptions: false,
        again: false,
    },
    Level {
        text_max: 120,
        descriptions: false,
        again: false,
    },
];

/// Eine Folie mit Text als Quelle einer oder mehrerer Zeilen (Prompt) oder Chunks
/// (Suchindex, Chat). Der Text ist bereinigt ([`clean_text`]), ungekuerzt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideText {
    pub id: String,
    pub number: u32,
    /// Beginn jeder Sichtung, aufsteigend; die erste traegt den Text.
    pub starts: Vec<u64>,
    pub text: String,
    pub description: Option<String>,
}

/// Eine fertige Zeile mit Zeit und Folie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedLine {
    pub number: u32,
    pub at_ms: u64,
    pub text: String,
}

/// Was die Kuerzung getan hat (nur Zaehler, nie Inhalt: Datenschutz D9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FitReport {
    /// Folien mit Text vor der Kuerzung.
    pub slides_total: usize,
    /// Folien, die als Zeile im Prompt stehen.
    pub slides_kept: usize,
    /// Index der erreichten Stufe in [`LEVELS`] (0 = ungekuerzt).
    pub level: usize,
    /// Zeichen, die alle Zeilen zusammen kosten (Zeilenumbrueche eingerechnet).
    pub chars: usize,
    /// Das Zeichenbudget, gegen das gekuerzt wurde.
    pub budget_chars: usize,
}

impl FitReport {
    /// Folien, die wegen Platzmangels GANZ fehlen.
    pub fn slides_dropped(&self) -> usize {
        self.slides_total - self.slides_kept
    }
    /// Wurde ueberhaupt gekuerzt oder weggelassen?
    pub fn reduced(&self) -> bool {
        self.level > 0 || self.slides_dropped() > 0
    }
}

/// Die Folien eines Laufs: fertige Zeilen, die belegbaren Nummern und der Bericht.
/// Entsteht einmal je Lauf aus EINEM Schnappschuss der Folien.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideContext {
    pub lines: Vec<RenderedLine>,
    /// Nummern der Folien, die im Prompt stehen: nur sie sind belegbar.
    pub numbers: HashSet<u32>,
    pub report: FitReport,
    /// Nummer -> (ID, erste Sichtung in ms), fuer Herkunft und Anzeige.
    by_number: HashMap<u32, (String, u64)>,
}

/// Ein belegte Folie fuer die Herkunft (`SourceRef kind = slide`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideRefInfo {
    pub slide_id: String,
    pub number: u32,
    pub at_ms: u64,
}

impl SlideRefInfo {
    /// `Folie 7 · 04:12`
    pub fn title(&self) -> String {
        slide_title(self.number, self.at_ms)
    }
}

/// `Folie 7 · 04:12`: Titel eines Beleges in Herkunft und Anzeige.
pub fn slide_title(number: u32, at_ms: u64) -> String {
    format!("Folie {number} · {}", mm_ss(at_ms))
}

impl SlideContext {
    /// Die Folien zu den belegten Nummern (aufsteigend, ohne Doppelte, hoechstens
    /// [`MAX_SOURCE_REFS`]: die Herkunft nimmt nicht mehr als 200 Quellen je Eintrag).
    pub fn refs(&self, used: &[u32]) -> Vec<SlideRefInfo> {
        let mut numbers: Vec<u32> = used.to_vec();
        numbers.sort_unstable();
        numbers.dedup();
        numbers
            .into_iter()
            .filter_map(|number| {
                self.by_number
                    .get(&number)
                    .map(|(slide_id, at_ms)| SlideRefInfo {
                        slide_id: slide_id.clone(),
                        number,
                        at_ms: *at_ms,
                    })
            })
            .take(MAX_SOURCE_REFS)
            .collect()
    }

    /// Zeichen aller Zeilen (mit Zeilenumbruch).
    #[cfg(test)]
    pub fn chars(&self) -> usize {
        self.report.chars
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Der Text einer Folie fuer den Prompt: eine Zeile, Leerraum zusammengefasst und
/// eckige wie geschweifte Klammern zu runden. Folientext ist DATEN: er darf weder
/// einen Beleg (`[F3]`, `[S12]`) noch die Kennzeichnung `{Bild: ...}` faelschen.
pub fn clean_text(raw: &str) -> String {
    let swapped: String = raw
        .chars()
        .map(|c| match c {
            '[' | '{' => '(',
            ']' | '}' => ')',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    swapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Kuerzt auf hoechstens `max` Zeichen, bevorzugt an einer Wortgrenze, mit `...`
/// am Ende (zaehlt mit). Ein Text, der passt, bleibt unveraendert.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(3);
    let head: String = text.chars().take(keep).collect();
    let cut = match head.rfind(' ') {
        // Eine Wortgrenze nur, wenn sie nicht fast alles abschneidet.
        Some(i) if head[..i].chars().count() * 2 >= keep => &head[..i],
        _ => head.as_str(),
    };
    format!("{}...", cut.trim_end())
}

// ---------------------------------------------------------------------------
// Aufbau
// ---------------------------------------------------------------------------

/// Die Folien, die in einen Prompt, den Suchindex und den Chat gehoeren: nicht
/// ausgeblendet, mit Text, nicht als "ohne Text" gekennzeichnet; nach erster Sichtung
/// geordnet.
pub fn slide_texts(slides: &[MeetingSlide]) -> Vec<SlideText> {
    let mut items: Vec<SlideText> = slides
        .iter()
        .filter(|s| !s.hidden && s.kind.as_deref() != Some(KIND_NO_TEXT))
        .filter_map(|s| {
            let text = clean_text(s.ocr_text.as_deref().unwrap_or(""));
            if text.is_empty() {
                return None;
            }
            let mut starts: Vec<u64> = s.occurrences.iter().map(|o| o.start_ms).collect();
            if starts.is_empty() {
                starts.push(0);
            }
            starts.sort_unstable();
            starts.dedup();
            let description = s
                .description
                .as_deref()
                .map(clean_text)
                .filter(|d| !d.is_empty());
            Some(SlideText {
                id: s.id.clone(),
                number: s.number,
                starts,
                text,
                description,
            })
        })
        .collect();
    items.sort_by_key(|i| (i.starts[0], i.number));
    items
}

fn render_first(item: &SlideText, style: LineStyle, level: Level) -> String {
    let text = clip(&item.text, level.text_max);
    let image = match (&item.description, level.descriptions) {
        (Some(d), true) => format!(" {{Bild: {}}}", clip(d, DESCRIPTION_MAX_CHARS)),
        _ => String::new(),
    };
    let at = mm_ss(item.starts[0]);
    match style {
        LineStyle::Minutes => format!("[Folie {} · {at}] {text}{image}", item.number),
        LineStyle::Notes => format!("F{} [{at}] Folie: {text}{image}", item.number),
    }
}

fn render_again(item: &SlideText, style: LineStyle, at_ms: u64) -> String {
    let at = mm_ss(at_ms);
    match style {
        LineStyle::Minutes => format!("[Folie {} · {at}] (wieder gezeigt)", item.number),
        LineStyle::Notes => format!("F{} [{at}] Folie: (wieder gezeigt)", item.number),
    }
}

/// Alle Zeilen einer Stufe, nach Zeit geordnet.
fn render_level(items: &[SlideText], style: LineStyle, level: Level) -> Vec<RenderedLine> {
    let mut out: Vec<RenderedLine> = Vec::new();
    for item in items {
        out.push(RenderedLine {
            number: item.number,
            at_ms: item.starts[0],
            text: render_first(item, style, level),
        });
        if level.again {
            for &at_ms in &item.starts[1..] {
                out.push(RenderedLine {
                    number: item.number,
                    at_ms,
                    text: render_again(item, style, at_ms),
                });
            }
        }
    }
    out.sort_by_key(|l| (l.at_ms, l.number));
    out
}

fn cost(lines: &[RenderedLine]) -> usize {
    lines.iter().map(|l| l.text.chars().count() + 1).sum()
}

/// Kuerzt die Folien auf `budget_chars`: die erste Stufe der Leiter, bei der alle
/// Zeilen passen. Passt auch die knappste nicht, bleiben so viele Folien (in
/// Zeitreihenfolge, knappste Stufe), wie hineingehen; die uebrigen vom Ende her
/// fehlen und werden gezaehlt.
fn fit(items: &[SlideText], style: LineStyle, budget_chars: usize) -> (Vec<RenderedLine>, FitReport) {
    let mut report = FitReport {
        slides_total: items.len(),
        budget_chars,
        ..FitReport::default()
    };
    for (index, level) in LEVELS.iter().enumerate() {
        let lines = render_level(items, style, *level);
        let chars = cost(&lines);
        if chars <= budget_chars {
            report.level = index;
            report.slides_kept = items.len();
            report.chars = chars;
            return (lines, report);
        }
    }
    // Auch die knappste Stufe ist zu gross: Folien vom Ende her weglassen.
    let last = LEVELS[LEVELS.len() - 1];
    let mut kept: Vec<SlideText> = Vec::new();
    let mut used = 0usize;
    for item in items {
        let line = render_first(item, style, last).chars().count() + 1;
        if used + line > budget_chars {
            break;
        }
        used += line;
        kept.push(item.clone());
    }
    let lines = render_level(&kept, style, last);
    report.level = LEVELS.len() - 1;
    report.slides_kept = kept.len();
    report.chars = cost(&lines);
    (lines, report)
}

/// Bereitet die Folien eines Laufs vor: nur nicht ausgeblendete mit Text, gekuerzt
/// auf ihren Anteil am Budget (`transcript_chars` = Zeichen des Transkripts). `None`,
/// wenn es keine gibt: dann bleibt jeder Prompt byteweise wie ohne Folien.
pub fn prepare(
    slides: &[MeetingSlide],
    style: LineStyle,
    transcript_chars: usize,
) -> Option<SlideContext> {
    let items = slide_texts(slides);
    if items.is_empty() {
        return None;
    }
    let budget = slide_budget_chars(transcript_chars);
    let (lines, report) = fit(&items, style, budget);
    if lines.is_empty() {
        return None;
    }
    let numbers: HashSet<u32> = lines.iter().map(|l| l.number).collect();
    let by_number: HashMap<u32, (String, u64)> = items
        .iter()
        .filter(|i| numbers.contains(&i.number))
        .map(|i| (i.number, (i.id.clone(), i.starts[0])))
        .collect();
    Some(SlideContext {
        lines,
        numbers,
        report,
        by_number,
    })
}

// ---------------------------------------------------------------------------
// Einweben
// ---------------------------------------------------------------------------

/// Setzt die Folienzeilen in die Zeilen des Transkripts. `starts[i]` ist der Beginn
/// des Segments `i` (aufsteigend), `lines[i]` seine Zeile. Eine Folie steht VOR dem
/// ersten Segment, das nicht vor ihr beginnt (sie war zu sehen, als der Satz
/// gesprochen wurde); nach dem letzten Segment: hinter der letzten Zeile. Das
/// Ergebnis hat genau so viele Elemente wie `lines`: eine Zeile mit Folie davor ist
/// ein Element mit Zeilenumbruch, so bleiben Bloecke, Zeichenzaehlung und
/// Zeitbereiche der Aufrufer unveraendert.
pub fn weave(starts: &[u64], mut lines: Vec<String>, slides: &[RenderedLine]) -> Vec<String> {
    debug_assert_eq!(starts.len(), lines.len());
    if lines.is_empty() || slides.is_empty() {
        return lines;
    }
    let mut before: Vec<Vec<&str>> = vec![Vec::new(); lines.len()];
    let mut tail: Vec<&str> = Vec::new();
    for slide in slides {
        let at = starts.partition_point(|&s| s < slide.at_ms);
        match before.get_mut(at) {
            Some(slot) => slot.push(&slide.text),
            None => tail.push(&slide.text),
        }
    }
    for (line, prefix) in lines.iter_mut().zip(before) {
        if !prefix.is_empty() {
            *line = format!("{}\n{line}", prefix.join("\n"));
        }
    }
    if !tail.is_empty() {
        if let Some(last) = lines.last_mut() {
            *last = format!("{last}\n{}", tail.join("\n"));
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Belege im Text (Protokoll)
// ---------------------------------------------------------------------------

/// Eine eckige Klammer, die mit `F<Zahl>` beginnt: `[F7]`, `[F7, F9]`, `[F7;9]`.
static TAG_GROUP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\s*[Ff]\s*\d[^\]\[]{0,60}\]").expect("Tag-Regex"));
static TAG_NUMBER: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)F?\s*(\d{1,6})").expect("Nummern-Regex"));
/// Der Inhalt einer Klammer darf nur aus `F<n>` und Trennern bestehen.
static TAG_INNER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\[\s*F\s*\d{1,6}(?:\s*[,;/]\s*(?:F\s*)?\d{1,6})*\s*\]$").expect("Inhalt-Regex")
});

/// Ergebnis von [`normalize_tags`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagOutcome {
    pub text: String,
    /// Belege auf Folien, die nicht im Prompt standen (entfernt).
    pub dropped: u32,
    /// Die gueltigen belegten Folien, in Reihenfolge des ersten Auftretens.
    pub used: Vec<u32>,
}

/// Prueft die Folienbelege (`[F7]`) im Text eines Eintrags: gueltige werden in die
/// Schreibweise `[F7]` gebracht (`[F7, F9]` wird `[F7][F9]`), Belege auf Folien,
/// die nicht im Prompt standen, entfallen und werden gezaehlt. Eine Klammer, die
/// kein reiner Beleg ist (`[F-Plan]`, `[F7 Entwurf]`), bleibt Text.
pub fn normalize_tags(text: &str, valid: &HashSet<u32>) -> TagOutcome {
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut outcome = TagOutcome::default();
    for m in TAG_GROUP.find_iter(text) {
        if !TAG_INNER.is_match(m.as_str()) {
            continue;
        }
        out.push_str(&text[last..m.start()]);
        let mut shown = String::new();
        for caps in TAG_NUMBER.captures_iter(m.as_str()) {
            let number: Option<u32> = caps.get(1).and_then(|n| n.as_str().parse().ok());
            match number {
                Some(n) if valid.contains(&n) => {
                    if !outcome.used.contains(&n) {
                        outcome.used.push(n);
                    }
                    let tag = format!("[F{n}]");
                    if !shown.contains(&tag) {
                        shown.push_str(&tag);
                    }
                }
                _ => outcome.dropped += 1,
            }
        }
        if shown.is_empty() {
            // Weggefallener Beleg: kein doppelter Leerraum zuruecklassen.
            let trimmed = out.trim_end_matches([' ', '\t']).len();
            let next = text[m.end()..].chars().next();
            if next.is_none_or(|c| c.is_whitespace() || ".,;:!?)".contains(c)) {
                out.truncate(trimmed);
            }
        } else {
            out.push_str(&shown);
        }
        last = m.end();
    }
    out.push_str(&text[last..]);
    outcome.text = out;
    outcome
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::slides::SlideOccurrence;

    fn slide(number: u32, start_ms: u64, text: &str) -> MeetingSlide {
        MeetingSlide {
            id: format!("sl-{number}"),
            meeting_id: "m1".into(),
            number,
            origin: "video".into(),
            image_path: format!("slides/{number:04}.jpg"),
            thumb_path: None,
            occurrences: vec![SlideOccurrence {
                start_ms,
                end_ms: start_ms + 20_000,
            }],
            ocr_text: Some(text.into()),
            ocr_engine: Some("windows-ocr".into()),
            kind: Some("text".into()),
            description: None,
            description_model: None,
            hidden: false,
        }
    }

    fn big(chars: usize) -> String {
        "Umsatz Region Sued ".repeat(chars / 19 + 1)[..chars].to_string()
    }

    #[test]
    fn a_minutes_line_reads_folie_number_and_time_before_the_text() {
        let ctx = prepare(
            &[slide(7, 252_000, "Umsatz 13,1 Mio. EUR")],
            LineStyle::Minutes,
            10_000,
        )
        .unwrap();
        assert_eq!(ctx.lines.len(), 1);
        assert_eq!(ctx.lines[0].text, "[Folie 7 · 04:12] Umsatz 13,1 Mio. EUR");
        assert_eq!(ctx.lines[0].at_ms, 252_000);
        assert!(ctx.numbers.contains(&7));
        assert_eq!(ctx.report.slides_total, 1);
        assert!(!ctx.report.reduced());
    }

    #[test]
    fn a_notes_line_carries_the_f_id_like_a_segment_id() {
        let ctx = prepare(&[slide(7, 252_000, "Umsatz")], LineStyle::Notes, 10_000).unwrap();
        assert_eq!(ctx.lines[0].text, "F7 [04:12] Folie: Umsatz");
    }

    #[test]
    fn without_usable_slides_nothing_changes() {
        assert!(prepare(&[], LineStyle::Minutes, 1_000).is_none());
        let mut hidden = slide(1, 0, "Text");
        hidden.hidden = true;
        let mut empty = slide(2, 0, "   ");
        empty.ocr_text = Some("   ".into());
        let mut none = slide(3, 0, "x");
        none.ocr_text = None;
        let mut no_text = slide(4, 0, "Danke");
        no_text.kind = Some(KIND_NO_TEXT.into());
        assert!(
            prepare(&[hidden, empty, none, no_text], LineStyle::Minutes, 1_000).is_none(),
            "ausgeblendet, ohne Text, ohne OCR und `ohne_text` werden nicht eingewoben"
        );
    }

    #[test]
    fn a_return_is_a_short_marker_not_the_whole_text_again() {
        let mut s = slide(3, 60_000, "Fahrplan 2027");
        s.occurrences.push(SlideOccurrence {
            start_ms: 600_000,
            end_ms: 640_000,
        });
        let ctx = prepare(&[s], LineStyle::Minutes, 10_000).unwrap();
        assert_eq!(ctx.lines.len(), 2);
        assert_eq!(ctx.lines[0].text, "[Folie 3 · 01:00] Fahrplan 2027");
        assert_eq!(ctx.lines[1].text, "[Folie 3 · 10:00] (wieder gezeigt)");
    }

    #[test]
    fn a_description_is_marked_and_dropped_first() {
        let mut s = slide(2, 0, "Balkendiagramm");
        s.description = Some("Region Sued hoechster Umsatz".into());
        let full = prepare(&[s.clone()], LineStyle::Minutes, 10_000).unwrap();
        assert_eq!(
            full.lines[0].text,
            "[Folie 2 · 00:00] Balkendiagramm {Bild: Region Sued hoechster Umsatz}"
        );
        // Das Budget reicht nur ohne die Beschreibung: Stufe 1.
        let (lines, report) = fit(&slide_texts(&[s]), LineStyle::Minutes, 40);
        assert_eq!(lines[0].text, "[Folie 2 · 00:00] Balkendiagramm");
        assert_eq!(report.level, 1);
    }

    #[test]
    fn the_budget_ladder_shortens_in_the_documented_order() {
        let mut slides: Vec<MeetingSlide> = (1..=4)
            .map(|n| slide(n, u64::from(n) * 60_000, &big(500)))
            .collect();
        slides[0].description = Some("Beschreibung ".repeat(10));
        slides[1].occurrences.push(SlideOccurrence {
            start_ms: 900_000,
            end_ms: 920_000,
        });
        let items = slide_texts(&slides);
        let level_for = |budget: usize| fit(&items, LineStyle::Minutes, budget).1;

        let full = level_for(1_000_000);
        assert_eq!((full.level, full.slides_dropped()), (0, 0));
        // Ohne Beschreibung gerade noch passend: Stufe 1.
        let no_desc = cost(&render_level(&items, LineStyle::Minutes, LEVELS[1]));
        assert_eq!(level_for(no_desc).level, 1);
        // Ohne Rueckkehr-Marken: Stufe 2.
        let no_again = cost(&render_level(&items, LineStyle::Minutes, LEVELS[2]));
        assert_eq!(level_for(no_again).level, 2);
        // Text auf 300, dann 120 Zeichen.
        let t300 = cost(&render_level(&items, LineStyle::Minutes, LEVELS[3]));
        assert_eq!(level_for(t300).level, 3);
        let t120 = cost(&render_level(&items, LineStyle::Minutes, LEVELS[4]));
        let report = level_for(t120);
        assert_eq!((report.level, report.slides_dropped()), (4, 0));
        assert!(report.reduced());
        // Noch knapper: Folien fallen weg, nie der Text einer behaltenen Folie weiter.
        let tiny = level_for(t120 / 2);
        assert!(tiny.slides_dropped() > 0 && tiny.slides_kept > 0, "{tiny:?}");
        assert!(tiny.chars <= t120 / 2);
    }

    #[test]
    fn slides_never_exceed_their_share_of_the_budget() {
        let slides: Vec<MeetingSlide> = (1..=40)
            .map(|n| slide(n, u64::from(n) * 30_000, &big(600)))
            .collect();
        // Transkript von 20 000 Zeichen: Anteil 25 % = 5 000 Zeichen.
        let ctx = prepare(&slides, LineStyle::Minutes, 20_000).unwrap();
        assert!(ctx.report.chars <= slide_budget_chars(20_000), "{:?}", ctx.report);
        assert!(ctx.report.slides_dropped() > 0 || ctx.report.level > 0);
        // Ein kurzes Transkript bekommt trotzdem Platz fuer einige Folien (Untergrenze).
        let few = prepare(&slides[..3], LineStyle::Minutes, 500).unwrap();
        assert!(few.report.slides_kept >= 3, "{:?}", few.report);
    }

    #[test]
    fn slide_text_cannot_forge_a_tag_or_a_marker() {
        let ctx = prepare(
            &[slide(
                1,
                0,
                "Siehe [F9] und [S12] {Bild: Zahl 5 Mio.} Ende",
            )],
            LineStyle::Minutes,
            10_000,
        )
        .unwrap();
        let line = &ctx.lines[0].text;
        assert_eq!(
            line,
            "[Folie 1 · 00:00] Siehe (F9) und (S12) (Bild: Zahl 5 Mio.) Ende"
        );
        assert_eq!(line.matches('[').count(), 1, "nur die Marke der Zeile");
        assert!(!line.contains('{'));
    }

    #[test]
    fn a_long_slide_text_is_clipped_with_an_ellipsis_on_a_word_boundary() {
        let ctx = prepare(&[slide(1, 0, &big(2_000))], LineStyle::Minutes, 100_000).unwrap();
        let body = ctx.lines[0].text.strip_prefix("[Folie 1 · 00:00] ").unwrap();
        assert!(body.chars().count() <= TEXT_MAX_CHARS);
        assert!(body.ends_with("..."));
    }

    #[test]
    fn slides_are_woven_before_the_first_segment_that_does_not_start_before_them() {
        let starts = [0u64, 10_000, 20_000, 30_000];
        let lines: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let slides = [
            RenderedLine { number: 1, at_ms: 0, text: "F1".into() },
            RenderedLine { number: 2, at_ms: 12_000, text: "F2".into() },
            RenderedLine { number: 3, at_ms: 20_000, text: "F3".into() },
            RenderedLine { number: 4, at_ms: 99_000, text: "F4".into() },
        ];
        let woven = weave(&starts, lines, &slides);
        assert_eq!(woven, vec!["F1\na", "b", "F2\nF3\nc", "d\nF4"]);
        assert_eq!(woven.len(), 4, "gleich viele Elemente wie Segmente");
    }

    #[test]
    fn weaving_without_slides_returns_the_lines_untouched() {
        let lines: Vec<String> = vec!["x".into(), "y".into()];
        assert_eq!(weave(&[0, 1], lines.clone(), &[]), lines);
        assert!(weave(&[], Vec::new(), &[]).is_empty());
    }

    fn valid(numbers: &[u32]) -> HashSet<u32> {
        numbers.iter().copied().collect()
    }

    #[test]
    fn valid_tags_stay_and_groups_are_normalised() {
        let out = normalize_tags("Umsatz 13,1 Mio. [F4] und Plan [F7, F9].", &valid(&[4, 7, 9]));
        assert_eq!(out.text, "Umsatz 13,1 Mio. [F4] und Plan [F7][F9].");
        assert_eq!(out.used, vec![4, 7, 9]);
        assert_eq!(out.dropped, 0);
    }

    #[test]
    fn unknown_tags_are_dropped_and_counted() {
        let out = normalize_tags("Budget steht [F99] fest. Zweitens [F4][F98].", &valid(&[4]));
        assert_eq!(out.text, "Budget steht fest. Zweitens [F4].");
        assert_eq!(out.used, vec![4]);
        assert_eq!(out.dropped, 2);
    }

    #[test]
    fn a_bracket_that_is_not_a_pure_tag_stays_text() {
        let text = "Der [F-Plan] und [F7 Entwurf] und [Q3].";
        let out = normalize_tags(text, &valid(&[7]));
        assert_eq!(out.text, text);
        assert_eq!((out.dropped, out.used.len()), (0, 0));
    }

    #[test]
    fn refs_name_the_slide_and_its_first_sighting() {
        let ctx = prepare(
            &[slide(4, 100_000, "Umsatz"), slide(7, 252_000, "Plan")],
            LineStyle::Minutes,
            10_000,
        )
        .unwrap();
        let refs = ctx.refs(&[7, 4, 7, 99]);
        assert_eq!(refs.len(), 2, "unbekannte Nummern entfallen, Doppelte auch");
        assert_eq!(refs[0].slide_id, "sl-4");
        assert_eq!(refs[0].title(), "Folie 4 · 01:40");
        assert_eq!(refs[1].title(), "Folie 7 · 04:12");
    }
}
