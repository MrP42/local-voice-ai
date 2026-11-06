//! Wort-Diff für das Lernen aus Korrekturen (Stufe 3, ADR-0021).
//!
//! Reine Logik ohne Speicher und ohne Tauri: Aus „erkannt“ und „korrigiert“ (oder
//! Vorlesetext und Erkennung) werden kurze Paare „gehört → korrigiert“, aber nur
//! dann, wenn die Änderung ein Verhörer ist und keine Umformulierung.
//!
//! Verfahren: Beide Texte werden in Wörter zerlegt (Satzzeichen am Rand und Groß-/
//! Kleinschreibung zählen nicht). Ein LCS auf den Wörtern trennt Gleiches von
//! Geändertem; zusammenhängende Änderungen bilden einen Block. Nur Blöcke, in denen
//! auf beiden Seiten 1 bis 3 Wörter stehen, sind Ersetzungen und kommen als Paar
//! infrage. Zu viele Blöcke, zu wenig Gemeinsames oder eine zu geringe Ähnlichkeit
//! der Schreibweisen sind eine Umformulierung und ergeben nichts.
//!
//! Inhalte stehen nie im Log und nie in Fehlermeldungen.

/// So viele Zeichen darf ein Text für `feedback` und `setup_evaluate` höchstens haben.
pub const MAX_TEXT_CHARS: usize = 2000;
/// Höchstens so viele Wörter je Seite eines Paares.
pub const MAX_PAIR_WORDS: usize = 3;
/// Höchstens so viele Zeichen je Seite eines Paares.
pub const MAX_PAIR_CHARS: usize = 64;
/// Ab dieser Ähnlichkeit (0 bis 1, ohne Leerzeichen und Groß-/Kleinschreibung) ist ein
/// Block ein Verhörer; darunter ist es ein anderes Wort, also eine Umformulierung.
pub const MIN_SIMILARITY: f64 = 0.5;
/// Eine Korrektur darf höchstens so viele Änderungsblöcke haben.
pub const MAX_CORRECTION_HUNKS: usize = 3;
/// Mindestens so viele Buchstaben muss die korrigierte Seite haben (kein „ein“ → „er“).
pub const MIN_CORRECTED_LETTERS: usize = 3;

/// Wofür der Diff läuft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Eine einzelne Äußerung wurde vom Nutzer verbessert: streng.
    Correction,
    /// Vorlesetext gegen Erkennung: viele Verhörer sind normal, unlesbare Blöcke werden übersprungen.
    Reading,
}

/// Ergebnis des Vergleichs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Mindestens ein lernbares Paar.
    Pairs,
    /// Nichts zu lernen: gleicher Text, nur Zeichensetzung oder Großschreibung, nur Einfügungen,
    /// Löschungen oder Zahlen.
    Nothing,
    /// Umformulierung (oder ein anderer Text): wird verworfen.
    Rewrite,
}

/// Ein gelerntes Paar in der Schreibweise des Textes (Rand-Satzzeichen entfernt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub heard: String,
    pub corrected: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub verdict: Verdict,
    pub pairs: Vec<Pair>,
    /// Wörter, die in beiden Texten übereinstimmen.
    pub matched: usize,
    /// Wörter des längeren Textes.
    pub total: usize,
}

impl Analysis {
    fn of(verdict: Verdict, matched: usize, total: usize) -> Self {
        Self {
            verdict,
            pairs: Vec::new(),
            matched,
            total,
        }
    }
}

/// Prüft die Länge. Die Meldung nennt keine Inhalte.
pub fn check_text(text: &str) -> Result<(), String> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!(
            "Der Text ist zu lang (höchstens {MAX_TEXT_CHARS} Zeichen)."
        ));
    }
    Ok(())
}

/// Bereich: 1 bis 32 Zeichen aus `a-z`, `0-9`, `_`, `-`.
pub fn check_bereich(bereich: &str) -> Result<(), String> {
    let ok = !bereich.is_empty()
        && bereich.len() <= 32
        && bereich
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err("Der Bereich ist ungültig (a-z, 0-9, _ und -, höchstens 32 Zeichen).".to_string())
    }
}

struct Tok {
    /// Schreibweise ohne Satzzeichen am Rand.
    shown: String,
    /// Vergleichsschlüssel: kleingeschrieben.
    key: String,
}

/// Funktionswörter (Artikel, Präpositionen, Pronomen, Bindewörter): sie werden nie gelernt und
/// gehören nie in den Wortschatz.
const FUNCTION_WORDS: &[&str] = &[
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "eines",
    "und", "oder", "aber", "auch", "nicht", "kein", "keine", "keinen", "keinem", "keiner", "ich",
    "du", "er", "sie", "es", "wir", "ihr", "mich", "mir", "dich", "dir", "ihn", "ihm", "uns",
    "euch", "ihnen", "mein", "meine", "dein", "deine", "sein", "seine", "unser", "unsere", "euer",
    "in", "im", "an", "am", "auf", "aus", "bei", "mit", "nach", "von", "vom", "vor", "zu", "zum",
    "zur", "für", "über", "unter", "um", "durch", "gegen", "ohne", "bis", "seit", "als", "wie",
    "wenn", "dass", "so", "da", "dann", "noch", "nur", "schon", "ja", "nein",
];

pub fn is_function_word(key: &str) -> bool {
    FUNCTION_WORDS.contains(&key)
}

/// Steht in `text` (ein Wort oder mehrere) ein Funktionswort?
pub fn has_function_word(text: &str) -> bool {
    tokenize(text).iter().any(|t| is_function_word(&t.key))
}

/// Setzt zerlegte Umlaute (Grundbuchstabe plus U+0308) zu einem Zeichen zusammen; mehr braucht
/// deutscher Text nicht, und eine Abhängigkeit für die volle Normalisierung wäre übertrieben.
pub fn compose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if chars.peek() == Some(&'\u{0308}') {
            let composed = match c {
                'a' => Some('ä'),
                'o' => Some('ö'),
                'u' => Some('ü'),
                'A' => Some('Ä'),
                'O' => Some('Ö'),
                'U' => Some('Ü'),
                _ => None,
            };
            if let Some(m) = composed {
                chars.next();
                out.push(m);
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn tokenize(text: &str) -> Vec<Tok> {
    let text = compose(text);
    text.split_whitespace()
        .filter_map(|raw| {
            let shown = raw.trim_matches(|c: char| !c.is_alphanumeric());
            if shown.is_empty() {
                return None;
            }
            Some(Tok {
                shown: shown.to_string(),
                key: shown.to_lowercase(),
            })
        })
        .collect()
}

/// Schlüssel eines Paares-Teils für den Vergleich im Speicher: klein, Wörter mit einem Leerzeichen.
pub fn normalize_key(text: &str) -> String {
    tokenize(text)
        .iter()
        .map(|t| t.key.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Ähnlichkeit zweier Schreibweisen von 0 bis 1, ohne Leerzeichen.
pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().filter(|c| !c.is_whitespace()).collect();
    let b: Vec<char> = b.chars().filter(|c| !c.is_whitespace()).collect();
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / longest as f64
}

struct Hunk {
    dels: Vec<usize>,
    ins: Vec<usize>,
}

/// Zeichen, die der eigene Wortschatz nicht annimmt (wie `CustomWords.tsx`).
pub fn has_forbidden_chars(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '<' | '>' | '"' | '\''))
}

fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

/// Ob ein Block als Paar taugt, wenn er eine Ersetzung ist.
enum Learnable {
    Yes(Pair),
    /// Gehört nicht in den Lernspeicher (Zahl, Sonderzeichen, zu kurz), ist aber keine Umformulierung.
    Skip,
    /// Zu lang oder zu verschieden: Umformulierung.
    Rewrite,
}

fn judge(a: &[Tok], b: &[Tok], hunk: &Hunk) -> Learnable {
    if hunk.dels.len() > MAX_PAIR_WORDS || hunk.ins.len() > MAX_PAIR_WORDS {
        return Learnable::Rewrite;
    }
    // Funktionswörter am Rand gehören nicht zum Paar: auf die tatsächlich geänderten Wörter kürzen.
    let trim = |toks: &[Tok], idx: &[usize]| -> Vec<usize> {
        let start = idx.iter().position(|&i| !is_function_word(&toks[i].key));
        let end = idx.iter().rposition(|&i| !is_function_word(&toks[i].key));
        match (start, end) {
            (Some(s), Some(e)) => idx[s..=e].to_vec(),
            _ => Vec::new(),
        }
    };
    let dels = trim(a, &hunk.dels);
    let ins = trim(b, &hunk.ins);
    if dels.is_empty() || ins.is_empty() {
        return Learnable::Skip;
    }
    let side = |toks: &[Tok], idx: &[usize]| -> (String, String) {
        let shown = idx
            .iter()
            .map(|&i| toks[i].shown.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let key = idx
            .iter()
            .map(|&i| toks[i].key.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        (shown, key)
    };
    let (heard, heard_key) = side(a, &dels);
    let (corrected, corrected_key) = side(b, &ins);
    if heard.chars().count() > MAX_PAIR_CHARS || corrected.chars().count() > MAX_PAIR_CHARS {
        return Learnable::Rewrite;
    }
    if letters(&heard) == 0 || letters(&corrected) == 0 {
        return Learnable::Skip;
    }
    if similarity(&heard_key, &corrected_key) < MIN_SIMILARITY {
        return Learnable::Rewrite;
    }
    if letters(&corrected) < MIN_CORRECTED_LETTERS
        || has_forbidden_chars(&corrected)
        || corrected_key.split(' ').any(is_function_word)
    {
        return Learnable::Skip;
    }
    Learnable::Yes(Pair { heard, corrected })
}

/// Zerlegt die Unterschiede in Blöcke; liefert auch die Zahl der gemeinsamen Wörter.
fn hunks(a: &[Tok], b: &[Tok]) -> (Vec<Hunk>, usize) {
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = Länge des LCS von a[i..] und b[j..]
    let width = m + 1;
    let mut lcs = vec![0u16; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * width + j] = if a[i].key == b[j].key {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut cur = Hunk {
        dels: Vec::new(),
        ins: Vec::new(),
    };
    let mut matched = 0;
    let (mut i, mut j) = (0, 0);
    let flush = |cur: &mut Hunk, out: &mut Vec<Hunk>| {
        if !cur.dels.is_empty() || !cur.ins.is_empty() {
            out.push(std::mem::replace(
                cur,
                Hunk {
                    dels: Vec::new(),
                    ins: Vec::new(),
                },
            ));
        }
    };
    while i < n || j < m {
        if i < n && j < m && a[i].key == b[j].key {
            flush(&mut cur, &mut out);
            matched += 1;
            i += 1;
            j += 1;
        } else if j >= m || (i < n && lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
            cur.dels.push(i);
            i += 1;
        } else {
            cur.ins.push(j);
            j += 1;
        }
    }
    flush(&mut cur, &mut out);
    (out, matched)
}

/// Vergleicht `heard` (was die Erkennung geliefert hat) mit `corrected` (was gemeint war:
/// die Korrektur des Nutzers bzw. der Vorlesetext). Das Paar heißt „gehört → korrigiert“.
pub fn analyze(heard: &str, corrected: &str, mode: Mode) -> Analysis {
    let a = tokenize(heard);
    let b = tokenize(corrected);
    let total = a.len().max(b.len());
    let (blocks, matched) = hunks(&a, &b);
    if blocks.is_empty() {
        return Analysis::of(Verdict::Nothing, matched, total);
    }
    if total >= 4 && matched * 2 < total {
        return Analysis::of(Verdict::Rewrite, matched, total);
    }
    if mode == Mode::Correction && blocks.len() > MAX_CORRECTION_HUNKS {
        return Analysis::of(Verdict::Rewrite, matched, total);
    }
    let mut pairs: Vec<Pair> = Vec::new();
    for hunk in &blocks {
        if hunk.dels.is_empty() || hunk.ins.is_empty() {
            continue;
        }
        match judge(&a, &b, hunk) {
            Learnable::Yes(pair) => {
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
            Learnable::Skip => {}
            Learnable::Rewrite if mode == Mode::Correction => {
                return Analysis::of(Verdict::Rewrite, matched, total)
            }
            Learnable::Rewrite => {}
        }
    }
    let verdict = if pairs.is_empty() {
        Verdict::Nothing
    } else {
        Verdict::Pairs
    };
    Analysis {
        verdict,
        pairs,
        matched,
        total,
    }
}
