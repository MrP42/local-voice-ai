//! Hörbuch ↔ Buchtext abgleichen (Hörbuch-Sync, Stufe 1).
//!
//! Eingang: der Buchtext und die Wörter, die eine Spracherkennung in einem
//! Stück Hörbuch gehört hat (mit Zeit). Ausgang: Stützpunkte `Zeichen ↔ ms`
//! für die Hervorhebung und Einschübe (gehörter Text, der im Buch fehlt).
//!
//! Vorgehen:
//! 1. Wörter normalisieren (Kleinschrift, nur Buchstaben/Ziffern, ß → ss).
//! 2. Anker: Wortdreier, die im Suchfenster des Buchs genau einmal vorkommen;
//!    davon die längste in beiden Achsen steigende Kette.
//! 3. Zwischen den Ankern: Wortabgleich per dynamischer Programmierung
//!    (ähnliche Wörter zählen als Treffer, Auslassungen kosten).
//! 4. Läufe von mindestens [`MIN_INSERT_WORDS`] gehörten Wörtern ohne Treffer
//!    werden Einschübe an der Buchstelle dazwischen.
//!
//! Keine Abhängigkeiten, kein Tauri: dieselbe Logik soll später in der
//! iPhone-App laufen. Gespeichert wird nur das Ergebnis, nie ein Transkript.

use std::collections::HashMap;

/// Ab so vielen gehörten Wörtern ohne Gegenstück im Buch gilt ein Lauf als Einschub.
pub const MIN_INSERT_WORDS: usize = 4;

/// Ein Wort des Buchtexts mit seiner Lage (Zeichen, nicht Bytes: wie im Frontend).
#[derive(Debug, Clone, PartialEq)]
pub struct BookWord {
    pub norm: String,
    pub start: u32,
    pub end: u32,
}

/// Ein gehörtes Wort mit Zeit (ms ab Hörbuchbeginn).
#[derive(Debug, Clone, PartialEq)]
pub struct HeardWord {
    pub text: String,
    pub norm: String,
    pub start_ms: f64,
    pub end_ms: f64,
}

/// Ein Erkennungssegment, wie es die Spracherkennung liefert.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub start_ms: f64,
    pub end_ms: f64,
}

/// Stützpunkt: an Zeichen `chars` des Buchs ist das Hörbuch bei `ms`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncPoint {
    pub chars: u32,
    pub ms: f64,
}

/// Gehörter Text ohne Gegenstück im Buch, einzufügen vor Zeichen `at`.
#[derive(Debug, Clone, PartialEq)]
pub struct Insert {
    pub at: u32,
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Alignment {
    pub points: Vec<SyncPoint>,
    pub inserts: Vec<Insert>,
    /// Anteil der gehörten Wörter mit Treffer im Buch (0..1).
    pub coverage: f32,
    /// Erstes und letztes getroffenes Buchwort (Index), für den nächsten Hinweis.
    pub book_span: Option<(usize, usize)>,
}

/// Ein Wort für den Vergleich: Kleinschrift, nur Buchstaben und Ziffern.
pub fn normalize(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if c == 'ß' {
            out.push_str("ss");
        } else if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Wörter des Buchs. Markdown-Zeichen und Seitenmarken `{{S. n}}` fallen
/// durch die Normalisierung bzw. werden übersprungen.
pub fn book_words(text: &str) -> Vec<BookWord> {
    let mut out = Vec::new();
    let mut start: Option<u32> = None;
    let mut buf = String::new();
    let mut idx: u32 = 0;
    let mut chars = text.chars().peekable();
    let flush = |buf: &mut String, start: &mut Option<u32>, end: u32, out: &mut Vec<BookWord>| {
        if let Some(s) = start.take() {
            let norm = normalize(buf);
            if !norm.is_empty() {
                out.push(BookWord {
                    norm,
                    start: s,
                    end,
                });
            }
        }
        buf.clear();
    };
    while let Some(c) = chars.next() {
        // Seitenmarke {{S. 12}} überspringen: kein Lesetext.
        if c == '{' && chars.peek() == Some(&'{') {
            flush(&mut buf, &mut start, idx, &mut out);
            let mut n = 1;
            let mut prev = c;
            for d in chars.by_ref() {
                n += 1;
                if prev == '}' && d == '}' {
                    break;
                }
                prev = d;
            }
            idx += n;
            continue;
        }
        if c.is_whitespace() || c == '—' || c == '–' {
            flush(&mut buf, &mut start, idx, &mut out);
        } else {
            if start.is_none() {
                start = Some(idx);
            }
            buf.push(c);
        }
        idx += 1;
    }
    flush(&mut buf, &mut start, idx, &mut out);
    out
}

/// Segmente in Wörter mit Zeit zerlegen: innerhalb eines Segments nach
/// Zeichenzahl verteilt (Segmentzeiten sind genau, Wortzeiten geschätzt).
pub fn heard_words(segments: &[Segment]) -> Vec<HeardWord> {
    let mut out = Vec::new();
    for seg in segments {
        let words: Vec<&str> = seg.text.split_whitespace().collect();
        let total: usize = words.iter().map(|w| w.chars().count() + 1).sum();
        if total == 0 {
            continue;
        }
        let span = (seg.end_ms - seg.start_ms).max(0.0);
        let mut acc = 0usize;
        for w in words {
            let len = w.chars().count() + 1;
            let a = seg.start_ms + span * acc as f64 / total as f64;
            acc += len;
            let b = seg.start_ms + span * acc as f64 / total as f64;
            let norm = normalize(w);
            if norm.is_empty() {
                continue;
            }
            out.push(HeardWord {
                text: w.to_string(),
                norm,
                start_ms: a,
                end_ms: b,
            });
        }
    }
    out
}

/// Ähnlichkeit zweier normalisierter Wörter (0..1), Levenshtein-basiert.
pub fn similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len().max(b.len());
    if n == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    1.0 - prev[b.len()] as f32 / n as f32
}

/// Gilt als dasselbe Wort (Hörfehler, Beugung, Bindestrich).
fn same(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let short = a.chars().count().min(b.chars().count());
    if short <= 2 {
        return false;
    }
    similarity(a, b) >= if short <= 4 { 0.75 } else { 0.7 }
}

fn trigram_key(a: &str, b: &str, c: &str) -> String {
    let mut k = String::with_capacity(a.len() + b.len() + c.len() + 2);
    k.push_str(a);
    k.push('\u{1}');
    k.push_str(b);
    k.push('\u{1}');
    k.push_str(c);
    k
}

/// Anker: (gehört, Buch) für Wortdreier, die in beiden Fenstern genau einmal
/// vorkommen; davon die längste steigende Kette.
fn anchors(book: &[BookWord], from: usize, to: usize, heard: &[HeardWord]) -> Vec<(usize, usize)> {
    let mut in_book: HashMap<String, Option<usize>> = HashMap::new();
    for i in from..to.saturating_sub(2) {
        let k = trigram_key(&book[i].norm, &book[i + 1].norm, &book[i + 2].norm);
        in_book
            .entry(k)
            .and_modify(|v| *v = None)
            .or_insert(Some(i));
    }
    let mut in_heard: HashMap<String, Option<usize>> = HashMap::new();
    for i in 0..heard.len().saturating_sub(2) {
        let k = trigram_key(&heard[i].norm, &heard[i + 1].norm, &heard[i + 2].norm);
        in_heard
            .entry(k)
            .and_modify(|v| *v = None)
            .or_insert(Some(i));
    }
    let mut pairs: Vec<(usize, usize)> = in_heard
        .iter()
        .filter_map(|(k, h)| match (h, in_book.get(k)) {
            (Some(h), Some(Some(b))) => Some((*h, *b)),
            _ => None,
        })
        .collect();
    pairs.sort_unstable();
    longest_chain(&pairs)
}

/// Längste in beiden Komponenten streng steigende Teilfolge (nach .0 sortiert).
fn longest_chain(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if pairs.is_empty() {
        return Vec::new();
    }
    let n = pairs.len();
    let mut len = vec![1usize; n];
    let mut prev = vec![usize::MAX; n];
    for i in 0..n {
        for j in 0..i {
            if pairs[j].0 < pairs[i].0 && pairs[j].1 < pairs[i].1 && len[j] + 1 > len[i] {
                len[i] = len[j] + 1;
                prev[i] = j;
            }
        }
    }
    let mut best = (0..n).max_by_key(|&i| len[i]).unwrap_or(0);
    let mut out = Vec::new();
    loop {
        out.push(pairs[best]);
        if prev[best] == usize::MAX {
            break;
        }
        best = prev[best];
    }
    out.reverse();
    out
}

/// Wortabgleich zweier Abschnitte; liefert Paare (gehört, Buch).
fn dp_match(
    heard: &[HeardWord],
    h0: usize,
    h1: usize,
    book: &[BookWord],
    b0: usize,
    b1: usize,
) -> Vec<(usize, usize)> {
    let n = h1.saturating_sub(h0);
    let m = b1.saturating_sub(b0);
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // Große Lücken ohne Anker nicht quadratisch abgleichen: dann nur die
    // ersten Wörter gegeneinander (der nächste Anker folgt bald).
    const MAX_CELLS: usize = 400_000;
    let (n, m) = if n * m > MAX_CELLS {
        let s = ((MAX_CELLS as f64).sqrt()) as usize;
        (n.min(s), m.min(s))
    } else {
        (n, m)
    };
    const GAP: i32 = -1;
    let w = m + 1;
    let mut score = vec![0i32; (n + 1) * w];
    let mut step = vec![0u8; (n + 1) * w]; // 1 diag, 2 up (heard skip), 3 left (book skip)
    for i in 1..=n {
        score[i * w] = GAP * i as i32;
        step[i * w] = 2;
    }
    for j in 1..=m {
        score[j] = GAP * j as i32;
        step[j] = 3;
    }
    for i in 1..=n {
        for j in 1..=m {
            let hit = same(&heard[h0 + i - 1].norm, &book[b0 + j - 1].norm);
            let d = score[(i - 1) * w + j - 1] + if hit { 2 } else { -2 };
            let u = score[(i - 1) * w + j] + GAP;
            let l = score[i * w + j - 1] + GAP;
            let (s, t) = if d >= u && d >= l {
                (d, 1)
            } else if u >= l {
                (u, 2)
            } else {
                (l, 3)
            };
            score[i * w + j] = s;
            step[i * w + j] = t;
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        match step[i * w + j] {
            1 => {
                if same(&heard[h0 + i - 1].norm, &book[b0 + j - 1].norm) {
                    out.push((h0 + i - 1, b0 + j - 1));
                }
                i -= 1;
                j -= 1;
            }
            2 => i -= 1,
            _ => j -= 1,
        }
    }
    out.reverse();
    out
}

/// Gehörte Wörter gegen das Buch legen.
///
/// `hint`: erwarteter Index des ersten gehörten Worts im Buch (aus dem letzten
/// Abgleich oder der anteiligen Schätzung); `window`: wie viele Buchwörter um
/// den Hinweis durchsucht werden. Ohne Hinweis wird das ganze Buch durchsucht
/// (nach einem Sprung).
pub fn align(
    book: &[BookWord],
    heard: &[HeardWord],
    hint: Option<usize>,
    window: usize,
) -> Alignment {
    if book.is_empty() || heard.is_empty() {
        return Alignment::default();
    }
    let (from, to) = match hint {
        Some(h) => (h.saturating_sub(window / 2), (h + window).min(book.len())),
        None => (0, book.len()),
    };
    let chain = anchors(book, from, to, heard);
    if chain.is_empty() {
        return Alignment::default();
    }
    // Anker sind Dreier: alle drei Wörter als feste Paare, dazwischen DP.
    let mut fixed: Vec<(usize, usize)> = Vec::new();
    for &(h, b) in &chain {
        for k in 0..3 {
            let p = (h + k, b + k);
            if fixed.last().is_none_or(|l| l.0 < p.0 && l.1 < p.1) {
                fixed.push(p);
            }
        }
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    // Vor dem ersten Anker: rückwärts begrenzt auf so viele Buchwörter wie gehört.
    let (fh, fb) = fixed[0];
    pairs.extend(dp_match(heard, 0, fh, book, fb.saturating_sub(fh * 2), fb));
    for win in fixed.windows(2) {
        let (a, b) = (win[0], win[1]);
        pairs.push(a);
        pairs.extend(dp_match(heard, a.0 + 1, b.0, book, a.1 + 1, b.1));
    }
    let (lh, lb) = *fixed.last().unwrap_or(&(0, 0));
    pairs.push((lh, lb));
    let rest = heard.len() - lh - 1;
    pairs.extend(dp_match(
        heard,
        lh + 1,
        heard.len(),
        book,
        lb + 1,
        (lb + 1 + rest * 2).min(book.len()),
    ));

    let points: Vec<SyncPoint> = pairs
        .iter()
        .map(|&(h, b)| SyncPoint {
            chars: book[b].start,
            ms: heard[h].start_ms,
        })
        .collect();

    let mut inserts = Vec::new();
    let mut last: Option<(usize, usize)> = None;
    let mut push_gap = |hs: usize, he: usize, at: u32| {
        if he.saturating_sub(hs) >= MIN_INSERT_WORDS {
            let text = heard[hs..he]
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            inserts.push(Insert {
                at,
                start_ms: heard[hs].start_ms,
                end_ms: heard[he - 1].end_ms,
                text,
            });
        }
    };
    for &(h, b) in &pairs {
        let hs = last.map_or(0, |l| l.0 + 1);
        if last.is_some() {
            push_gap(hs, h, book[b].start);
        }
        last = Some((h, b));
    }
    if let Some((h, b)) = last {
        if heard.len() - h > MIN_INSERT_WORDS && b + 1 >= book.len() {
            push_gap(h + 1, heard.len(), book[b].end);
        }
    }

    Alignment {
        coverage: pairs.len() as f32 / heard.len() as f32,
        book_span: Some((pairs[0].1, pairs[pairs.len() - 1].1)),
        points,
        inserts,
    }
}

#[cfg(test)]
mod tests;
