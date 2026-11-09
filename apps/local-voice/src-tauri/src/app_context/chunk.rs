//! Zerlegen von Nicht-Buch-Texten in Stücke (Bücher: `book_ai::talk::index`).
//!
//! Regeln: Absätze (Leerzeile oder Zeilenumbruch) werden zu Stücken von
//! [`MIN_CHARS`] bis [`MAX_CHARS`] Zeichen zusammengelegt; ein zu langer Absatz
//! wird an Satzenden, notfalls hart geteilt. Offsets sind Zeichenoffsets im
//! Original (für den Sprung).

pub const MIN_CHARS: usize = 400;
pub const MAX_CHARS: usize = 900;

/// Ein Stück Text mit Zeichenoffset im Original.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub start: usize,
    pub text: String,
}

/// Absätze mit Startoffset (Zeichen), Leerraum gefaltet, leere übersprungen.
fn paragraphs(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_start = 0usize;
    for (i, line) in split_lines(text) {
        let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            if !cur.is_empty() {
                out.push((cur_start, std::mem::take(&mut cur)));
            }
            continue;
        }
        if cur.is_empty() {
            cur_start = i;
            cur = t;
        } else {
            cur.push(' ');
            cur.push_str(&t);
        }
        // Einzelne Zeilen (Diktate, Gesprächsbeiträge) sind eigene Absätze,
        // wenn sie mit einem Satzende schließen.
        if cur.ends_with(['.', '!', '?', ':']) {
            out.push((cur_start, std::mem::take(&mut cur)));
        }
    }
    if !cur.is_empty() {
        out.push((cur_start, cur));
    }
    out
}

/// Zeilen mit ihrem Zeichenoffset.
fn split_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut chars = 0usize;
    for line in text.split('\n') {
        out.push((chars, line));
        chars += line.chars().count() + 1;
    }
    out
}

/// Teilt einen zu langen Absatz an Satzenden (notfalls hart bei `MAX_CHARS`).
fn split_long(start: usize, text: &str) -> Vec<Piece> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut from = 0usize;
    while from < chars.len() {
        let limit = (from + MAX_CHARS).min(chars.len());
        let mut cut = limit;
        if limit < chars.len() {
            if let Some(p) = (from + MIN_CHARS / 2..limit)
                .rev()
                .find(|&i| matches!(chars[i], '.' | '!' | '?') && chars.get(i + 1) == Some(&' '))
            {
                cut = p + 1;
            }
        }
        let piece: String = chars[from..cut].iter().collect();
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            let lead = piece.chars().take_while(|c| c.is_whitespace()).count();
            out.push(Piece {
                start: start + from + lead,
                text: trimmed.to_string(),
            });
        }
        from = cut;
    }
    out
}

/// Zerlegt einen Text in Stücke.
pub fn pieces(text: &str) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut cur: Option<Piece> = None;
    for (start, para) in paragraphs(text) {
        let len = para.chars().count();
        if len > MAX_CHARS {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            out.extend(split_long(start, &para));
            continue;
        }
        match cur.as_mut() {
            Some(c) if c.text.chars().count() + 1 + len <= MAX_CHARS => {
                c.text.push(' ');
                c.text.push_str(&para);
            }
            _ => {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                cur = Some(Piece { start, text: para });
            }
        }
        if cur
            .as_ref()
            .is_some_and(|c| c.text.chars().count() >= MIN_CHARS)
        {
            out.push(cur.take().unwrap());
        }
    }
    if let Some(c) = cur {
        out.push(c);
    }
    out
}
