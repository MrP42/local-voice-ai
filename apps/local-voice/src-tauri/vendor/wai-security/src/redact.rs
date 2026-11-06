use regex::Regex;
use std::fmt;
use std::sync::OnceLock;

const MASK: &str = "[REDACTED]";

/// Huelle, die ihren Inhalt nie in Debug/Display preisgibt.
pub struct Redacted<T>(pub T);
impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MASK)
    }
}
impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(MASK)
    }
}

fn rules() -> &'static Vec<(Regex, &'static str)> {
    static R: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    R.get_or_init(|| {
        let r = |p: &str| Regex::new(p).expect("static regex");
        vec![
            // Mail-Inhalte: body/subject/... : "..." (auch Debug-Format mit Escapes)
            (
                r(r#"(?i)\b(body|subject|mail_body|html_body|text_body|snippet)("?\s*[:=]\s*)"(?:[^"\\]|\\.)*""#),
                "$1$2\"[REDACTED]\"",
            ),
            // Authorization / Bearer
            (r(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}"), "$1 [REDACTED]"),
            // Schluessel=Wert, auch JSON/Debug-Format mit Anfuehrungszeichen
            (
                r(r#"(?i)\b([A-Za-z_]*(?:token|secret|password|passwd|api[_-]?key|apikey|authorization|credential|private[_-]?key)[A-Za-z_]*)("?\s*[:=]\s*)(?:"(?:[^"\\]|\\.)*"|[^\s,;)}\]]+)"#),
                "$1$2[REDACTED]",
            ),
            // bekannte Praefixe
            (r(r"\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}"), MASK),
            (r(r"\b(?:ghp|gho|ghs|ghu|github_pat)_[A-Za-z0-9_]{16,}"), MASK),
            (r(r"\bxox[abprs]-[A-Za-z0-9-]{10,}"), MASK),
            (r(r"\bAKIA[0-9A-Z]{16}\b"), MASK),
            // JWT
            (r(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*"), MASK),
            // E-Mail-Adressen
            (r(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"), "[EMAIL]"),
        ]
    })
}

/// Maskiert Tokens, Keys, Passwoerter, E-Mail-Adressen und Mail-Inhalte.
pub fn redact(input: &str) -> String {
    let mut s = input.to_string();
    for (re, rep) in rules() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    s
}

/// Debug-Ausgabe eines Werts, redigiert.
pub fn redact_debug<T: fmt::Debug>(v: &T) -> String {
    redact(&format!("{v:?}"))
}
