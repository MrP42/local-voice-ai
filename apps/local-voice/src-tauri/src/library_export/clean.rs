//! Text einer Fassung für den Export aufbereiten (lva#236).
//!
//! Der Seitentext trägt Steuerzeichen des Vorlesens, die im Lesemodus
//! unsichtbar sind und in einer weitergegebenen Datei nichts verloren haben:
//! Seitenmarken `{{S. 12}}` aus dem PDF-Import, Stil-Tags `[flüstert]` und
//! Sprecher-Marker `<Mara>` am Zeilenanfang. Die Regel ist dieselbe wie im
//! Lesemodus (`readerDoc.parseInline`): jedes `[…]` innerhalb einer Zeile ist
//! ein Tag. Markdown (`#`, `**`, `*`) bleibt stehen; daraus entstehen
//! Überschriften und Hervorhebungen im Zielformat.
//!
//! Rein (ohne Tauri), damit es ohne App-Build prüfbar ist.

/// Exporttext ohne Seitenmarken, Tags und Sprecher-Marker.
pub fn for_export(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").split('\n') {
        let line = strip_inline(raw);
        let line = strip_speaker(&line);
        // Eine Zeile, die nur aus Steuerzeichen bestand, verschwindet ganz
        // (statt eine Leerzeile mitten in einen Absatz zu setzen).
        if line.trim().is_empty() && !raw.trim().is_empty() {
            continue;
        }
        out.push(line.trim_end().to_string());
    }
    collapse_blank_lines(&out.join("\n")).trim().to_string()
}

/// Seitenmarken und `[…]`-Tags einer Zeile entfernen, Leerzeichen glätten.
fn strip_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && chars.get(i + 1) == Some(&'{') {
            if let Some(len) = page_mark_len(&chars[i..]) {
                i += len;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == ']') {
                if close > 0 {
                    i += close + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    squeeze_spaces(&out)
}

/// Länge einer Seitenmarke `{{S. 12}}` / `{{S.12}}` am Anfang, sonst `None`.
fn page_mark_len(chars: &[char]) -> Option<usize> {
    let s: String = chars.iter().take(16).collect();
    let rest = s.strip_prefix("{{S.")?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 5 {
        return None;
    }
    if !rest[digits..].starts_with("}}") {
        return None;
    }
    Some(s.len() - rest.len() + digits + 2)
}

/// Mehrfache Leerzeichen (aus entfernten Marken) zu einem; Einrückung bleibt.
fn squeeze_spaces(line: &str) -> String {
    let lead = line.len() - line.trim_start().len();
    let mut out = String::from(&line[..lead]);
    let mut prev_space = false;
    for c in line[lead..].chars() {
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            // Kein Leerzeichen vor Satzzeichen, das eine Marke hinterlassen hat.
            if prev_space && matches!(c, '.' | ',' | ';' | ':' | '!' | '?') {
                out.pop();
            }
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// Sprecher-Marker `<Name>` am Zeilenanfang (bis 60 Zeichen) entfernen.
fn strip_speaker(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('<') {
        if let Some(end) = rest.find('>') {
            let name = &rest[..end];
            if !name.is_empty()
                && name.chars().count() <= 60
                && !name.contains('<')
                && !name.contains('\n')
            {
                let lead = &line[..line.len() - trimmed.len()];
                return format!("{lead}{}", rest[end + 1..].trim_start());
            }
        }
    }
    line.to_string()
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.split('\n') {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Dateiname ohne Zeichen, die Windows oder macOS nicht erlauben.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let squeezed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = squeezed.trim_matches(|c: char| c == '.' || c == ' ');
    let short: String = trimmed.chars().take(120).collect();
    if short.is_empty() {
        "Export".to_string()
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_page_marks_tags_and_speakers() {
        let text = "# Kapitel 1\n\n<Mara> [flüstert] Hallo {{S. 12}} Welt.\nZweite {{S.13}}Zeile .\n{{S. 14}}\n\nEnde [lacht]";
        assert_eq!(
            for_export(text),
            "# Kapitel 1\n\nHallo Welt.\nZweite Zeile.\n\nEnde"
        );
    }

    #[test]
    fn keeps_markdown_and_text_without_marks() {
        let text = "## Teil\n\nEin **fetter** und *kursiver* Satz.\n\n- Punkt";
        assert_eq!(for_export(text), text);
    }

    #[test]
    fn keeps_unclosed_brackets_and_angle_text() {
        assert_eq!(for_export("a [b\nc"), "a [b\nc");
        assert_eq!(for_export("x < y > z"), "x < y > z");
    }

    #[test]
    fn collapses_blank_lines() {
        assert_eq!(for_export("a\n\n\n\nb\r\n"), "a\n\nb");
    }

    #[test]
    fn safe_names() {
        assert_eq!(
            safe_file_name("Alles Idioten?! – Kurz: DE"),
            "Alles Idioten ! – Kurz DE"
        );
        assert_eq!(safe_file_name("..."), "Export");
        assert_eq!(safe_file_name("a/b\\c"), "a b c");
    }
}
