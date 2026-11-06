//! Index über den Buchtext (Plan §3.1): Stücke von 500 bis 900 Zeichen, nie
//! über Kapitelgrenzen, mit Seite, Kapitel und UTF-16-Offsets im
//! ORIGINALTEXT (Sprung im Reader). Gleiche Regeln wie der Swift-Kern
//! (Referenz, `BookTalkIndex.swift`): Absätze, Zusammenlegen, Grenzen,
//! Seitenzuordnung, Kapitelindex (0-basiert), Säubern.
//!
//! Gesäubert werden Sprechermarken `<Name>`, Stilmarken `[calm]` und
//! Seitenmarken `{{S. n}}`; `<<<` und `>>>` sind keine Sprechermarken.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::lex::fnv1a;

pub const MIN_CHARS: usize = 500;
pub const MAX_CHARS: usize = 900;

/// Ein Stück Buchtext.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: usize,
    /// 0-basiert nach Überschriften (Text vor der ersten Überschrift zählt zu 0).
    pub chapter_index: usize,
    pub chapter_title: String,
    /// Letzte Seitenmarke vor dem Stück; `None`, wenn noch keine kam.
    pub page: Option<u32>,
    pub start: usize,
    pub end: usize,
    /// Gesäubert, Leerraum gefaltet.
    pub text: String,
    /// UTF-16-Einheiten von `text`.
    pub units: Vec<u16>,
    /// Originaloffset je UTF-16-Einheit von `text` (plus ein Eintrag fürs Ende).
    pub offset_map: Vec<usize>,
}

impl Chunk {
    pub fn text_len(&self) -> usize {
        self.units.len()
    }

    pub fn slice(&self, from: usize, to: usize) -> String {
        String::from_utf16_lossy(&self.units[from..to])
    }

    /// Spoilergrenze: am `frontier` (UTF-16-Offset im Original) an der
    /// Satzgrenze davor kürzen. `None`, wenn nichts davor übrig bleibt.
    pub fn truncated(&self, frontier: usize) -> Option<Chunk> {
        if self.end <= frontier {
            return Some(self.clone());
        }
        if self.start >= frontier {
            return None;
        }
        let mut cut = 0;
        for (_, e) in sentence_spans(&self.units) {
            if self.offset_map[e - 1] + 1 <= frontier {
                cut = e;
            }
        }
        if cut == 0 {
            return None;
        }
        Some(Chunk {
            id: self.id,
            chapter_index: self.chapter_index,
            chapter_title: self.chapter_title.clone(),
            page: self.page,
            start: self.start,
            end: self.offset_map[cut - 1] + 1,
            text: self.slice(0, cut),
            units: self.units[..cut].to_vec(),
            offset_map: self.offset_map[..=cut].to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMark {
    pub offset: usize,
    pub page: u32,
}

#[derive(Debug, Clone)]
pub struct BookIndex {
    pub title: String,
    pub chunks: Vec<Chunk>,
    /// Aufsteigend nach Offset.
    pub page_marks: Vec<PageMark>,
    /// FNV-1a des Quelltextes: Neuaufbau, wenn sich der Text ändert.
    pub hash: u64,
    pub source_length: usize,
    /// Stämme aller Inhaltswörter des Buchs (träge berechnet, siehe `cite`).
    pub stems: OnceLock<HashSet<String>>,
}

pub fn text_hash(text: &str) -> u64 {
    fnv1a(text)
}

pub fn build(title: &str, text: &str) -> BookIndex {
    let u: Vec<u16> = text.encode_utf16().collect();
    let marks = scan_marks(&u);
    let chunks = pack(parse(&u), &marks);
    BookIndex {
        title: title.to_string(),
        chunks,
        page_marks: marks,
        hash: fnv1a(text),
        source_length: u.len(),
        stems: OnceLock::new(),
    }
}

impl BookIndex {
    /// Seite an einem Originaloffset (letzte Marke bei oder vor `offset`).
    pub fn page_at(&self, offset: usize) -> Option<u32> {
        page_in(&self.page_marks, offset)
    }

    /// Leseposition am Ende der Seite `page` (= Beginn der Marke der Folgeseite).
    pub fn position_end_of_page(&self, page: u32) -> usize {
        self.page_marks
            .iter()
            .find(|m| m.page > page)
            .map(|m| m.offset)
            .unwrap_or(self.source_length)
    }
}

fn page_in(marks: &[PageMark], offset: usize) -> Option<u32> {
    let (mut lo, mut hi) = (0, marks.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if marks[mid].offset <= offset {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    (lo > 0).then(|| marks[lo - 1].page)
}

fn scan_marks(u: &[u16]) -> Vec<PageMark> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < u.len() {
        if u[i] == 123 {
            if let Some((page, end)) = page_marker(u, i, u.len()) {
                out.push(PageMark { offset: i, page });
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------- Parsen --

#[derive(Debug, Clone)]
struct Paragraph {
    chapter_index: usize,
    chapter_title: String,
    start: usize,
    end: usize,
    text: String,
    units: Vec<u16>,
    map: Vec<usize>,
}

fn flush(
    out: &mut Vec<Paragraph>,
    open: &mut Vec<(usize, usize)>,
    u: &[u16],
    chapter_index: usize,
    chapter_title: &str,
) {
    let lines = std::mem::take(open);
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
        return;
    };
    let mut cleaner = Cleaner::default();
    for (n, r) in lines.iter().enumerate() {
        if n > 0 {
            cleaner.separator(r.0);
        }
        cleaner.append(u, r.0, r.1);
    }
    let (units, map) = cleaner.finish();
    if units.is_empty() {
        return;
    }
    out.push(Paragraph {
        chapter_index,
        chapter_title: chapter_title.to_string(),
        start: map.first().copied().unwrap_or(first.0),
        end: last.1,
        text: String::from_utf16_lossy(&units),
        units,
        map,
    });
}

fn parse(u: &[u16]) -> Vec<Paragraph> {
    let mut paragraphs = Vec::new();
    let mut chapter_index = 0;
    let mut chapter_title = String::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i <= u.len() {
        let mut line_end = i;
        while line_end < u.len() && u[line_end] != 10 {
            line_end += 1;
        }
        let (mut s, mut e) = (i, line_end);
        while s < e && is_space(u[s]) {
            s += 1;
        }
        while e > s && is_space(u[e - 1]) {
            e -= 1;
        }
        i = line_end + 1;
        if s == e {
            flush(&mut paragraphs, &mut open, u, chapter_index, &chapter_title);
            continue;
        }
        if u[s] == 35 {
            let mut h = s;
            while h < e && u[h] == 35 {
                h += 1;
            }
            if h < e && is_space(u[h]) {
                flush(&mut paragraphs, &mut open, u, chapter_index, &chapter_title);
                chapter_index += 1;
                let mut t = h;
                while t < e && is_space(u[t]) {
                    t += 1;
                }
                chapter_title = String::from_utf16_lossy(&u[t..e]);
                continue;
            }
        }
        while let Some((_, end)) = page_marker(u, s, e) {
            flush(&mut paragraphs, &mut open, u, chapter_index, &chapter_title);
            s = end;
            while s < e && is_space(u[s]) {
                s += 1;
            }
        }
        if s >= e {
            continue;
        }
        open.push((s, e));
    }
    flush(&mut paragraphs, &mut open, u, chapter_index, &chapter_title);
    paragraphs
}

// ---------------------------------------------------------- Stücke bilden --

fn emit(group: &mut Vec<Paragraph>, out: &mut Vec<Chunk>, marks: &[PageMark]) {
    let g = std::mem::take(group);
    let (Some(first), Some(last)) = (g.first(), g.last()) else {
        return;
    };
    let mut units: Vec<u16> = Vec::new();
    let mut map: Vec<usize> = Vec::new();
    for (n, p) in g.iter().enumerate() {
        if n > 0 {
            units.push(32);
            map.push(p.start);
        }
        units.extend_from_slice(&p.units);
        map.extend_from_slice(&p.map[..p.map.len() - 1]);
    }
    map.push(last.map.last().copied().unwrap_or(last.end));
    out.push(Chunk {
        id: out.len(),
        chapter_index: first.chapter_index.saturating_sub(1),
        chapter_title: first.chapter_title.clone(),
        page: page_in(marks, first.start),
        start: first.start,
        end: map.last().copied().unwrap_or(last.end),
        text: String::from_utf16_lossy(&units),
        units,
        offset_map: map,
    });
}

fn pack(paragraphs: Vec<Paragraph>, marks: &[PageMark]) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut group: Vec<Paragraph> = Vec::new();
    let mut group_len = 0;
    for p in &paragraphs {
        if let Some(g) = group.first() {
            if g.chapter_index != p.chapter_index {
                emit(&mut group, &mut out, marks);
                group_len = 0;
            }
        }
        for piece in split(p) {
            let len = piece.units.len();
            if !group.is_empty() && (group_len >= MIN_CHARS || group_len + 1 + len > MAX_CHARS) {
                emit(&mut group, &mut out, marks);
                group_len = 0;
            }
            group_len += if group.is_empty() { 0 } else { 1 } + len;
            group.push(piece);
        }
    }
    emit(&mut group, &mut out, marks);
    out
}

/// Absätze über `MAX_CHARS` werden an Satzgrenzen (notfalls Wortgrenzen) geteilt.
fn split(p: &Paragraph) -> Vec<Paragraph> {
    let u = &p.units;
    if u.len() <= MAX_CHARS {
        return vec![p.clone()];
    }
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let (mut from, mut to) = (0usize, 0usize);
    for (s_start, s_end) in sentence_spans(u) {
        if s_end - from > MAX_CHARS && to > from {
            ranges.push((from, to));
            from = s_start;
        }
        while s_end - from > MAX_CHARS {
            let mut e = from + MAX_CHARS;
            while e > from + MIN_CHARS && u[e - 1] != 32 {
                e -= 1;
            }
            ranges.push((from, e));
            from = e;
            while from < s_end && u[from] == 32 {
                from += 1;
            }
        }
        to = s_end;
    }
    if to > from {
        ranges.push((from, to));
    }
    ranges
        .into_iter()
        .filter_map(|(mut a, mut b)| {
            while a < b && u[a] == 32 {
                a += 1;
            }
            while b > a && u[b - 1] == 32 {
                b -= 1;
            }
            if b <= a {
                return None;
            }
            let map = p.map[a..=b].to_vec();
            Some(Paragraph {
                chapter_index: p.chapter_index,
                chapter_title: p.chapter_title.clone(),
                start: map[0],
                end: map[map.len() - 1],
                text: String::from_utf16_lossy(&u[a..b]),
                units: u[a..b].to_vec(),
                map,
            })
        })
        .collect()
}

// --------------------------------------------------------- Textwerkzeuge --

pub fn is_space(c: u16) -> bool {
    matches!(c, 32 | 9 | 10 | 13 | 0xA0)
}

/// `{{S. 12}}` ab `at`: Seite und Offset hinter der Marke.
pub fn page_marker(u: &[u16], at: usize, limit: usize) -> Option<(u32, usize)> {
    const HEAD: [u16; 4] = [123, 123, 83, 46];
    if at + HEAD.len() >= limit || u[at..at + HEAD.len()] != HEAD {
        return None;
    }
    let mut i = at + HEAD.len();
    while i < limit && is_space(u[i]) {
        i += 1;
    }
    let (mut n, mut digits) = (0u32, 0);
    while i < limit && (48..=57).contains(&u[i]) && digits < 7 {
        n = n * 10 + u32::from(u[i] - 48);
        i += 1;
        digits += 1;
    }
    if digits == 0 {
        return None;
    }
    while i < limit && is_space(u[i]) {
        i += 1;
    }
    (i + 1 < limit && u[i] == 125 && u[i + 1] == 125).then_some((n, i + 2))
}

/// Säubert (Sprecher `<Name>`, Stilmarken `[calm]`, Seitenmarken, Leerraum
/// falten) und führt die Offset-Abbildung mit.
#[derive(Default)]
struct Cleaner {
    units: Vec<u16>,
    map: Vec<usize>,
    pending_space: Option<usize>,
}

impl Cleaner {
    fn separator(&mut self, offset: usize) {
        if self.pending_space.is_none() && !self.units.is_empty() {
            self.pending_space = Some(offset);
        }
    }

    fn append(&mut self, u: &[u16], from: usize, to: usize) {
        let mut i = from;
        while i < to {
            let c = u[i];
            if is_space(c) {
                if self.pending_space.is_none() && !self.units.is_empty() {
                    self.pending_space = Some(i);
                }
                i += 1;
                continue;
            }
            if c == 123 {
                if let Some((_, end)) = page_marker(u, i, to) {
                    i = end;
                    continue;
                }
            }
            if c == 60 && !(i > from && u[i - 1] == 60) {
                if let Some(j) = closing(u, i, to, 40) {
                    if !(j + 1 < to && u[j + 1] == 62) {
                        i = j + 1;
                        continue;
                    }
                }
            }
            if c == 91 {
                if let Some(j) = closing(u, i, to, 24) {
                    i = j + 1;
                    continue;
                }
            }
            if let Some(p) = self.pending_space.take() {
                self.units.push(32);
                self.map.push(p);
            }
            self.units.push(c);
            self.map.push(i);
            i += 1;
        }
    }

    fn finish(mut self) -> (Vec<u16>, Vec<usize>) {
        let end = self.map.last().map_or(0, |m| m + 1);
        self.map.push(end);
        (self.units, self.map)
    }
}

/// `<Name>` (Buchstaben, Ziffern, Leerzeichen, Punkt, Bindestrich) oder
/// `[calm]` (a–z, -, _): Position der schließenden Klammer.
fn closing(u: &[u16], open: usize, to: usize, max_len: usize) -> Option<usize> {
    let angle = u[open] == 60;
    let close: u16 = if angle { 62 } else { 93 };
    let mut j = open + 1;
    while j < to && j - open <= max_len {
        let c = u[j];
        if c == close {
            return (j > open + 1).then_some(j);
        }
        let ok = if angle {
            matches!(c, 32 | 46 | 45 | 95 | 39)
                || char::from_u32(u32::from(c))
                    .is_some_and(|ch| ch.is_alphabetic() || ch.is_ascii_digit())
        } else {
            (97..=122).contains(&c) || c == 45 || c == 95
        };
        if !ok {
            return None;
        }
        j += 1;
    }
    None
}

// ----------------------------------------------------------------- Sätze --

fn abbreviations() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        [
            "z", "b", "d", "h", "u", "a", "o", "bzw", "usw", "ca", "dr", "nr", "s", "vgl", "etc",
            "evtl", "ggf", "inkl", "prof", "str", "sog", "bspw", "e", "g", "i", "vs", "mr", "mrs",
            "ms", "st", "no", "p", "pp", "ff", "f",
        ]
        .into_iter()
        .collect()
    })
}

const CLOSERS: [u16; 10] = [0x201C, 0x201D, 0x2018, 0x2019, 34, 39, 0xBB, 0xAB, 41, 93];
const OPENERS: [u16; 9] = [0x201E, 0x201C, 0x201A, 0x2018, 34, 39, 0xBB, 0xAB, 40];

/// Wörter, auf die eine Ordnungszahl („14.“) eindeutig folgt: Monatsnamen
/// (deutsch, englisch, Kurzformen) und wenige Substantive („3. Kapitel“).
fn ordinal_followers() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        [
            "januar",
            "jänner",
            "februar",
            "märz",
            "april",
            "mai",
            "juni",
            "juli",
            "august",
            "september",
            "oktober",
            "november",
            "dezember",
            "jan",
            "feb",
            "mär",
            "mrz",
            "apr",
            "jun",
            "jul",
            "aug",
            "sep",
            "sept",
            "okt",
            "nov",
            "dez",
            "january",
            "february",
            "march",
            "may",
            "june",
            "july",
            "october",
            "december",
            "mar",
            "oct",
            "dec",
            "kapitel",
            "stock",
            "auflage",
            "jahrhundert",
            "weltkrieg",
        ]
        .into_iter()
        .collect()
    })
}

/// „14. November“, „3. Kapitel“: Zahl mit 1–2 Ziffern vor dem Punkt und ein
/// eindeutiger Folgewort danach ist keine Satzgrenze. Vierstellige Zahlen
/// („… 1998. November …“) bleiben eine Grenze.
fn is_ordinal(u: &[u16], start: usize, dot: usize, next: usize) -> bool {
    let mut d = dot;
    while d > start && (48..=57).contains(&u[d - 1]) {
        d -= 1;
    }
    let digits = dot - d;
    if !(1..=2).contains(&digits) || (d > start && alphabetic(u[d - 1])) {
        return false;
    }
    let mut e = next;
    while e < u.len() && alphabetic(u[e]) {
        e += 1;
    }
    let word = String::from_utf16_lossy(&u[next..e]).to_lowercase();
    ordinal_followers().contains(word.as_str())
}

fn upper_or_digit(c: u16) -> bool {
    char::from_u32(u32::from(c)).is_some_and(|ch| ch.is_uppercase()) || (48..=57).contains(&c)
}

fn alphabetic(c: u16) -> bool {
    char::from_u32(u32::from(c)).is_some_and(char::is_alphabetic)
}

/// Satzspannen `[start, end)` in UTF-16-Einheiten des untersuchten Textes.
pub fn sentence_spans(u: &[u16]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if !(c == 46 || c == 33 || c == 63 || c == 0x2026) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < u.len() && (u[j] == 46 || u[j] == 33 || u[j] == 63) {
            j += 1;
        }
        while j < u.len() && CLOSERS.contains(&u[j]) {
            j += 1;
        }
        if !(j < u.len() && u[j] == 32) {
            i = j.max(i + 1);
            continue;
        }
        let mut k = j;
        while k < u.len() && u[k] == 32 {
            k += 1;
        }
        let next = if k < u.len() { u[k] } else { 0 };
        let mut boundary = upper_or_digit(next) || OPENERS.contains(&next);
        if boundary && c == 46 && j == i + 1 {
            let mut w = i;
            while w > start && alphabetic(u[w - 1]) {
                w -= 1;
            }
            let word = String::from_utf16_lossy(&u[w..i]).to_lowercase();
            if abbreviations().contains(word.as_str()) && word.encode_utf16().count() <= 4 {
                boundary = false;
            }
            if boundary && is_ordinal(u, start, i, k) {
                boundary = false;
            }
        }
        if boundary {
            out.push((start, j));
            start = k;
        }
        i = k.max(i + 1);
    }
    let mut e = u.len();
    while e > start && u[e - 1] == 32 {
        e -= 1;
    }
    if e > start {
        out.push((start, e));
    }
    out
}

pub fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book_ai::talk::testkit::{buch, offsets};

    fn split(text: &str) -> Vec<String> {
        let u = units(text);
        sentence_spans(&u)
            .into_iter()
            .map(|(a, b)| String::from_utf16_lossy(&u[a..b]))
            .collect()
    }

    #[test]
    fn dates_and_ordinals_are_not_sentence_ends() {
        assert_eq!(
            split("Am 14. November 1998 strandete das Schiff. Dort stand ein Haus."),
            [
                "Am 14. November 1998 strandete das Schiff.",
                "Dort stand ein Haus."
            ]
        );
        for t in [
            "Am 3. Jänner kam er an. Dann ging er.",
            "Am 3. Dez. kam er an. Dann ging er.",
            "Am 1. Mai war es kalt. Dann ging er.",
            "On 14. March it began. Then it ended.",
            "Im 2. Stock wohnte er. Dann ging er.",
            "Im 3. Kapitel stirbt er. Dann ging er.",
        ] {
            assert_eq!(split(t).len(), 2, "{t}");
        }
    }

    #[test]
    fn a_year_at_the_end_and_real_ordinal_like_ends_stay_boundaries() {
        assert_eq!(
            split("Es geschah im Jahr 1998. November war kalt."),
            ["Es geschah im Jahr 1998.", "November war kalt."]
        );
        assert_eq!(split("Er zählte bis 14. Der Mann ging.").len(), 2);
        assert_eq!(split("Es waren 123. November kam.").len(), 2);
        assert_eq!(split("Das ist Nr. 5. Dann ging er.").len(), 2);
    }

    #[test]
    fn abbreviations_are_unchanged() {
        assert_eq!(split("Siehe z. B. Seite 4. Dann ging er.").len(), 2);
        assert_eq!(split("Dr. Meier kam. Er ging.").len(), 2);
    }

    fn idx() -> BookIndex {
        build("Der Weg", &buch())
    }

    fn slice(text: &str, s: usize, e: usize) -> String {
        String::from_utf16_lossy(&units(text)[s..e])
    }

    #[test]
    fn chunks_respect_limits_and_chapters() {
        let ix = idx();
        assert!(!ix.chunks.is_empty());
        for (n, c) in ix.chunks.iter().enumerate() {
            assert_eq!(c.id, n);
            assert!(
                c.text_len() <= MAX_CHARS,
                "Stück {n} zu lang: {}",
                c.text_len()
            );
            assert_eq!(c.offset_map.len(), c.text_len() + 1);
            assert!(c.start < c.end && c.end <= ix.source_length);
        }
        let chapters: Vec<usize> = ix.chunks.iter().map(|c| c.chapter_index).collect();
        assert!(chapters.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn every_chunk_char_maps_back_into_the_original() {
        let text = buch();
        let ix = idx();
        for c in &ix.chunks {
            for (k, &u) in c.units.iter().enumerate() {
                if u == 32 {
                    continue;
                }
                let o = c.offset_map[k];
                assert_eq!(units(&text)[o], u, "Stück {} Zeichen {k}", c.id);
            }
        }
    }

    #[test]
    fn page_marks_headings_and_speakers_are_not_text() {
        let ix = idx();
        for c in &ix.chunks {
            assert!(!c.text.contains("{{S."), "{}", c.text);
            assert!(!c.text.contains("##"));
        }
        let text = "# Kapitel\n\n<Anselm> Hallo [calm] Welt.\n\n<<<x>>> bleibt.";
        let t = build("t", text);
        assert_eq!(t.chunks[0].text, "Hallo Welt. <<<x>>> bleibt.");
        assert_eq!(
            slice(text, t.chunks[0].start, t.chunks[0].start + 5),
            "Hallo"
        );
    }

    #[test]
    fn offset_fixtures() {
        for case in offsets() {
            let ix = build("t", &case.text);
            if let Some(q) = &case.quote {
                let ut = units(&case.text);
                let uq = units(q);
                let at = ut
                    .windows(uq.len())
                    .position(|w| w == uq.as_slice())
                    .unwrap_or_else(|| panic!("{}: Zitat nicht im Text", case.id));
                if let Some(p) = case.expect_page {
                    assert_eq!(ix.page_at(at), Some(p), "{}", case.id);
                }
            }
            if let (Some(after), Some(end)) = (&case.frontier_after, &case.expect_chunk_ends_with) {
                let ut = units(&case.text);
                let ua = units(after);
                let at = ut
                    .windows(ua.len())
                    .position(|w| w == ua.as_slice())
                    .unwrap_or_else(|| panic!("{}: Grenze nicht im Text", case.id));
                let frontier = at + ua.len() + 3;
                let cut = ix
                    .chunks
                    .iter()
                    .filter(|c| c.start < frontier)
                    .filter_map(|c| c.truncated(frontier))
                    .last()
                    .expect("Stück vor der Grenze");
                assert!(
                    cut.text.ends_with(end.as_str()),
                    "{}: {}",
                    case.id,
                    cut.text
                );
                assert!(cut.end <= frontier);
            }
        }
    }

    #[test]
    fn frontier_cuts_a_chunk_at_the_sentence_before() {
        let ix = idx();
        let c = ix
            .chunks
            .iter()
            .find(|c| sentence_spans(&c.units).len() > 2)
            .expect("Stück mit mehreren Sätzen");
        let spans = sentence_spans(&c.units);
        let cut_at = c.offset_map[spans[1].1 - 1] + 1 + 1;
        let t = c.truncated(cut_at).expect("gekürzt");
        assert_eq!(t.text_len(), spans[1].1);
        assert!(t.end <= cut_at);
        assert!(c.truncated(c.start).is_none());
        assert_eq!(c.truncated(c.end).unwrap().text, c.text);
    }

    #[test]
    fn a_long_paragraph_is_split_within_the_limits() {
        let sentence = "Der Meister ging langsam durch den Hof und sah nach den Bäumen. ";
        let text = format!("# K\n\n{}", sentence.repeat(60));
        let ix = build("t", &text);
        assert!(ix.chunks.len() > 2);
        assert!(ix.chunks.iter().all(|c| c.text_len() <= MAX_CHARS));
    }

    #[test]
    fn unicode_offsets_count_utf16_units() {
        let text = "# K\n\n{{S. 1}}\n\n😀 Erster Satz. Zweiter Satz.";
        let ix = build("t", &text);
        let c = &ix.chunks[0];
        assert_eq!(slice(&text, c.start, c.start + 2), "😀");
        assert_eq!(c.page, Some(1));
    }

    #[test]
    fn sentences_split_after_closers_and_not_after_abbreviations() {
        let u = units("Er sagte: „Geh heim.“ Dann ging er, z. B. nach Hause. Fertig!");
        let s: Vec<String> = sentence_spans(&u)
            .into_iter()
            .map(|(a, b)| String::from_utf16_lossy(&u[a..b]))
            .collect();
        assert_eq!(s.len(), 3, "{s:?}");
    }
}
