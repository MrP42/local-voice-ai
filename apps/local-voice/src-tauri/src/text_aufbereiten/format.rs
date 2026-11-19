//! Deterministische Formatierung mit unveränderten Strukturmarken.

use unicode_normalization::UnicodeNormalization;

/// Welche Regeln laufen. Alle standardmäßig an (`Default`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptionen {
    pub nfc: bool,
    pub ligaturen: bool,
    pub weiche_trennstriche: bool,
    pub anfuehrungszeichen: bool,
    pub gedankenstrich: bool,
    pub auslassung: bool,
    pub leerzeichen: bool,
    pub listen: bool,
}

impl Default for FormatOptionen {
    fn default() -> Self {
        Self {
            nfc: true,
            ligaturen: true,
            weiche_trennstriche: true,
            anfuehrungszeichen: true,
            gedankenstrich: true,
            auslassung: true,
            leerzeichen: true,
            listen: true,
        }
    }
}

/// Eine Regel und wie oft sie gegriffen hat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegelBericht {
    pub regel: &'static str,
    pub anzahl: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatBericht {
    pub regeln: Vec<RegelBericht>,
}

fn melden(bericht: &mut FormatBericht, regel: &'static str, anzahl: u32) {
    if anzahl > 0 {
        bericht.regeln.push(RegelBericht { regel, anzahl });
    }
}

// Alle Längen sind Bytepositionen aus gültigen Zeichengrenzen.
fn geschuetzte_laenge(text: &str) -> Option<usize> {
    if let Some(rest) = text.strip_prefix("{{S. ") {
        let ziffern = rest.bytes().take_while(u8::is_ascii_digit).count();
        if ziffern > 0 && rest[ziffern..].starts_with("}}") {
            return Some(5 + ziffern + 2);
        }
    }
    if text.starts_with('[') {
        for (anzahl, (position, zeichen)) in text[1..].char_indices().enumerate() {
            if zeichen == ']' {
                return (anzahl <= 80).then_some(position + 2);
            }
            if matches!(zeichen, '\n' | '\r') || anzahl >= 80 {
                return None;
            }
        }
        return None;
    }
    if let Some(rest) = text.strip_prefix('<') {
        let mut anzahl = 0;
        for (position, zeichen) in rest.char_indices() {
            if zeichen == '>' {
                return (anzahl > 0).then_some(position + 2);
            }
            anzahl += 1;
            if zeichen == '<' || anzahl > 40 {
                return None;
            }
        }
    }
    None
}

fn code_laenge(text: &str, zeilenanfang: bool) -> Option<usize> {
    if zeilenanfang {
        let eingerueckt = text.trim_start_matches([' ', '\t']);
        let erstes = eingerueckt.chars().next()?;
        let breite = eingerueckt.chars().take_while(|&z| z == erstes).count();
        if matches!(erstes, '`' | '~') && breite >= 3 {
            let mut zeilen = text.split_inclusive('\n');
            let mut laenge = zeilen.next()?.len();
            for zeile in zeilen {
                laenge += zeile.len();
                let inhalt = zeile.trim();
                let schluss = inhalt.chars().take_while(|&z| z == erstes).count();
                if schluss >= breite && inhalt[schluss..].trim().is_empty() {
                    return Some(laenge);
                }
            }
            return Some(text.len());
        }
    }
    if !text.starts_with('`') {
        return None;
    }
    let breite = text.bytes().take_while(|&z| z == b'`').count();
    let zeile = text.split(['\n', '\r']).next()?;
    let mut position = breite;
    while let Some(abstand) = zeile[position..].find('`') {
        position += abstand;
        let schluss = zeile[position..].bytes().take_while(|&z| z == b'`').count();
        if schluss == breite {
            return Some(position + schluss);
        }
        position += schluss;
    }
    None
}

fn maskieren(text: &str) -> (String, Vec<(String, String)>) {
    // Ein im Original fehlendes Präfix verhindert Kollisionen mit privaten Zeichen.
    let mut praefix = String::from('\u{e000}');
    while text.contains(&praefix) {
        praefix.push('\u{e000}');
    }
    let mut maskiert = String::new();
    let mut marken = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let position = text.len() - rest.len();
        let zeilenanfang = position == 0 || text[..position].ends_with('\n');
        if let Some(laenge) = code_laenge(rest, zeilenanfang).or_else(|| geschuetzte_laenge(rest)) {
            // Zeilengrenzen bleiben auch innerhalb geschützter Codeblöcke sichtbar.
            for zeile in rest[..laenge].split_inclusive('\n') {
                let inhalt = zeile.trim_end_matches(['\r', '\n']);
                // Auch der Index besteht ausschließlich aus privaten Zeichen.
                let index: String = marken
                    .len()
                    .to_string()
                    .bytes()
                    .map(|ziffer| {
                        char::from_u32(0xe010 + u32::from(ziffer - b'0')).unwrap_or('\u{e010}')
                    })
                    .collect();
                let platzhalter = format!("{praefix}{index}\u{e001}");
                maskiert.push_str(&platzhalter);
                maskiert.push_str(&zeile[inhalt.len()..]);
                marken.push((platzhalter, inhalt.to_owned()));
            }
            rest = &rest[laenge..];
        } else if let Some(zeichen) = rest.chars().next() {
            maskiert.push(zeichen);
            rest = &rest[zeichen.len_utf8()..];
        }
    }
    (maskiert, marken)
}

fn anfuehrungen(absatz: &str) -> (String, u32) {
    let mut vorher = None;
    let mut offen = false;
    let mut anzahl = 0_u32;
    let mut ergebnis = String::new();
    for zeichen in absatz.chars() {
        if zeichen == '"' {
            // Markdown-Betonung öffnet Zitate; nach ':' bleibt die Zuordnung unsicher.
            let oeffnend = vorher
                .is_none_or(|z: char| z.is_whitespace() || matches!(z, '(' | '[' | '*' | '_'));
            // Auch eine gerade Anzahl kann falsch verschachtelt sein.
            if oeffnend == offen {
                return (absatz.to_owned(), 0);
            }
            ergebnis.push(if oeffnend { '„' } else { '“' });
            offen = oeffnend;
            anzahl = anzahl.saturating_add(1);
        } else {
            ergebnis.push(zeichen);
        }
        vorher = Some(zeichen);
    }
    if offen {
        (absatz.to_owned(), 0)
    } else {
        (ergebnis, anzahl)
    }
}

fn anfuehrungszeichen(text: &str) -> (String, u32) {
    let mut ergebnis = String::new();
    let mut absatz = String::new();
    let mut anzahl = 0_u32;
    for zeile in text.split_inclusive('\n') {
        if zeile.trim().is_empty() {
            let (formatiert, ersetzt) = anfuehrungen(&absatz);
            ergebnis.push_str(&formatiert);
            ergebnis.push_str(zeile);
            anzahl = anzahl.saturating_add(ersetzt);
            absatz.clear();
        } else {
            absatz.push_str(zeile);
        }
    }
    let (formatiert, ersetzt) = anfuehrungen(&absatz);
    ergebnis.push_str(&formatiert);
    (ergebnis, anzahl.saturating_add(ersetzt))
}

fn gedankenstriche(text: &str) -> (String, u32) {
    let zeichen: Vec<char> = text.chars().collect();
    let mut ergebnis = String::new();
    let mut anzahl = 0_u32;
    for (index, &aktuell) in zeichen.iter().enumerate() {
        let mut ersetzen = aktuell == '-'
            && index > 0
            && zeichen[index - 1] == ' '
            && zeichen.get(index + 1) == Some(&' ');
        if ersetzen {
            let links = zeichen[..index].iter().rev().find(|z| !z.is_whitespace());
            let rechts = zeichen[index + 1..].iter().find(|z| !z.is_whitespace());
            let zeilenanfang = zeichen[..index]
                .iter()
                .rev()
                .take_while(|&&z| z != '\n' && z != '\r')
                .all(|z| z.is_whitespace() || matches!(*z, '>' | '|'));
            let anfang = zeichen[..index]
                .iter()
                .rposition(|&z| matches!(z, '\n' | '\r'))
                .map_or(0, |p| p + 1);
            let ende = zeichen[index..]
                .iter()
                .position(|&z| matches!(z, '\n' | '\r'))
                .map_or(zeichen.len(), |p| index + p);
            let zeile: String = zeichen[anfang..ende].iter().collect();
            let trennlinie = zeile
                .chars()
                .all(|z| z.is_whitespace() || matches!(z, '-' | '*' | '_'));
            ersetzen = !zeilenanfang
                && !zeile.trim_start().starts_with('>')
                && !tabellenzeile(&zeile)
                && !trennlinie
                && !(links.is_some_and(|z| z.is_ascii_digit())
                    && rechts.is_some_and(|z| z.is_ascii_digit()));
        }
        ergebnis.push(if ersetzen { '–' } else { aktuell });
        if ersetzen {
            anzahl = anzahl.saturating_add(1);
        }
    }
    (ergebnis, anzahl)
}

fn auslassungen(text: &str) -> (String, u32) {
    let mut ergebnis = String::new();
    let mut zeichen = text.chars().peekable();
    let mut anzahl = 0_u32;
    while let Some(zeichen_aktuell) = zeichen.next() {
        if zeichen_aktuell != '.' {
            ergebnis.push(zeichen_aktuell);
            continue;
        }
        let mut punkte = 1;
        while zeichen.peek() == Some(&'.') {
            zeichen.next();
            punkte += 1;
        }
        if punkte == 3 {
            ergebnis.push('…');
            anzahl = anzahl.saturating_add(1);
        } else {
            ergebnis.extend(std::iter::repeat_n('.', punkte));
        }
    }
    (ergebnis, anzahl)
}

fn tabellenzeile(zeile: &str) -> bool {
    zeile.contains('|')
}

fn listen_inhalt(zeile: &str) -> Option<(usize, &str)> {
    let einzug = zeile.bytes().take_while(|&z| z == b' ').count();
    let inhalt = &zeile[einzug..];
    if einzug > 3 || tabellenzeile(zeile) || inhalt.starts_with('>') {
        return None;
    }
    Some((einzug, inhalt))
}

fn listen_nummer(zeile: &str) -> Option<(&str, usize)> {
    let (einzug, inhalt) = listen_inhalt(zeile)?;
    let ziffern = inhalt.bytes().take_while(u8::is_ascii_digit).count();
    if ziffern == 0 || !inhalt[ziffern..].starts_with(") ") {
        return None;
    }
    if inhalt[ziffern + 2..].trim().is_empty() {
        return None;
    }
    Some((&inhalt[..ziffern], einzug + ziffern))
}

fn listen(text: &str) -> (String, u32) {
    let zeilen: Vec<_> = text.split_inclusive('\n').collect();
    let nummern: Vec<_> = zeilen.iter().map(|zeile| listen_nummer(zeile)).collect();
    let mut ergebnis = String::new();
    let mut anzahl = 0_u32;
    for (index, zeile) in zeilen.iter().enumerate() {
        // Beide Enden eines aufsteigenden Nachbarpaares gehören zur Liste.
        let aufsteigend = |links: usize, rechts: usize| match (nummern[links], nummern[rechts]) {
            (Some((a, _)), Some((b, _))) => {
                // Ziffernweise prüfen, damit auch lange Nummern nicht überlaufen.
                let mut vorher = a.bytes().rev();
                let mut uebertrag = 1;
                for ziffer in b.bytes().rev() {
                    let summe = vorher.next().map_or(0, |z| z - b'0') + uebertrag;
                    if ziffer - b'0' != summe % 10 {
                        return false;
                    }
                    uebertrag = summe / 10;
                }
                uebertrag == 0 && vorher.all(|z| z == b'0')
            }
            _ => false,
        };
        if let Some((_, position)) = nummern[index] {
            if (index > 0 && aufsteigend(index - 1, index))
                || (index + 1 < zeilen.len() && aufsteigend(index, index + 1))
            {
                ergebnis.push_str(&zeile[..position]);
                ergebnis.push('.');
                ergebnis.push_str(&zeile[position + 1..]);
                anzahl = anzahl.saturating_add(1);
                continue;
            }
        }
        if let Some((einzug, inhalt)) = listen_inhalt(zeile) {
            if let Some(zeichen @ ('•' | '·' | '‣' | '◦' | '▪' | '●')) = inhalt.chars().next()
            {
                let rest = &inhalt[zeichen.len_utf8()..];
                if rest.starts_with([' ', '\t']) && !rest.trim().is_empty() {
                    ergebnis.push_str(&zeile[..einzug]);
                    ergebnis.push_str("- ");
                    ergebnis.push_str(rest.trim_start_matches([' ', '\t', '\u{a0}']));
                    anzahl = anzahl.saturating_add(1);
                    continue;
                }
            }
        }
        ergebnis.push_str(zeile);
    }
    (ergebnis, anzahl)
}

fn leerzeichen(text: &str) -> (String, u32) {
    // NBSP wird je Leerraumgruppe nur einmal gezählt; zwischen Ziffer und Buchstabe
    // bleibt es als geschützter Abstand für Zahl und Einheit erhalten.
    let mut anzahl = 0_u32;
    let mut ergebnis = String::new();
    let mut zeilen = text.split_inclusive('\n').peekable();
    while let Some(zeile) = zeilen.next() {
        let (inhalt, ende) = if let Some(rest) = zeile.strip_suffix("\r\n") {
            (rest, "\r\n")
        } else if let Some(rest) = zeile.strip_suffix('\n') {
            (rest, "\n")
        } else {
            (zeile, "")
        };
        if tabellenzeile(inhalt) {
            ergebnis.push_str(zeile);
            continue;
        }
        let mut zeichen = inhalt.char_indices().peekable();
        while let Some((position, zeichen_aktuell)) = zeichen.next() {
            if !matches!(zeichen_aktuell, ' ' | '\t' | '\u{a0}') {
                ergebnis.push(zeichen_aktuell);
                continue;
            }
            let mut ende_position = position + zeichen_aktuell.len_utf8();
            while let Some(&(index, z)) = zeichen.peek() {
                if !matches!(z, ' ' | '\t' | '\u{a0}') {
                    break;
                }
                ende_position = index + z.len_utf8();
                zeichen.next();
            }
            let original = &inhalt[position..ende_position];
            let danach = zeichen.peek().map(|&(_, z)| z);
            // Der Abstand hinter einem Listenzeichen gehört zur Markdown-Struktur.
            let listenabstand = listen_inhalt(&inhalt[..position])
                .is_some_and(|(_, rest)| matches!(rest, "-" | "•" | "·" | "‣" | "◦" | "▪" | "●"));
            let satzende = !listenabstand
                && danach.is_some_and(|z| matches!(z, '.' | ',' | ';' | ':' | '!' | '?'))
                && inhalt[ende_position..]
                    .chars()
                    .nth(1)
                    .is_none_or(char::is_whitespace);
            let zahlenverhaeltnis = danach == Some(':')
                && inhalt[..position]
                    .chars()
                    .next_back()
                    .is_some_and(|z| z.is_ascii_digit())
                && inhalt[ende_position + 1..]
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(|z| z.is_ascii_digit());
            let einheit = original == "\u{a0}"
                && inhalt[..position]
                    .chars()
                    .next_back()
                    .is_some_and(|z| z.is_ascii_digit())
                && danach.is_some_and(char::is_alphabetic);
            let hard_break = danach.is_none()
                && zeilen.peek().is_some()
                && !inhalt[..position].trim().is_empty()
                && original.len() >= 2
                && original.chars().all(|z| z == ' ');
            let ersatz = if original.contains('\t') || einheit || hard_break {
                original
            } else if danach.is_none() {
                ""
            } else if position == 0 {
                original
            } else if satzende && !zahlenverhaeltnis {
                ""
            } else {
                " "
            };
            ergebnis.push_str(ersatz);
            if original != ersatz {
                anzahl = anzahl.saturating_add(1);
            }
        }
        ergebnis.push_str(ende);
    }
    (ergebnis, anzahl)
}

/// Zählt veränderte Zeichengruppen, die nur gemeinsam normalisiert werden können.
/// Dazu gehören kombinierende Zeichen und zusammensetzbare Hangul-Zeichen.
fn nfc_normalisieren(text: &str) -> (String, u32) {
    let normalisiert: String = text.nfc().collect();
    if normalisiert == text {
        return (normalisiert, 0);
    }
    let mut anzahl = 0_u32;
    let mut anfang = 0;
    for (position, zeichen) in text.char_indices() {
        if position == 0 {
            continue;
        }
        let gruppe = &text[anfang..position];
        let einzelzeichen = zeichen.to_string();
        let zusammen = &text[anfang..position + zeichen.len_utf8()];
        if !gruppe.nfc().chain(einzelzeichen.nfc()).eq(zusammen.nfc()) {
            continue;
        }
        if !gruppe.chars().eq(gruppe.nfc()) {
            anzahl = anzahl.saturating_add(1);
        }
        anfang = position;
    }
    let gruppe = &text[anfang..];
    if !gruppe.chars().eq(gruppe.nfc()) {
        anzahl = anzahl.saturating_add(1);
    }
    (normalisiert, anzahl.max(1))
}

pub fn formatieren(text: &str, opt: &FormatOptionen) -> (String, FormatBericht) {
    let (mut text, marken) = maskieren(text);
    let mut bericht = FormatBericht::default();
    if opt.nfc {
        let (normalisiert, anzahl) = nfc_normalisieren(&text);
        melden(&mut bericht, "nfc", anzahl);
        text = normalisiert;
    }
    if opt.ligaturen {
        let mut anzahl = 0_u32;
        text = text
            .chars()
            .map(|z| {
                let ersatz = match z {
                    'ﬀ' => "ff",
                    'ﬁ' => "fi",
                    'ﬂ' => "fl",
                    'ﬃ' => "ffi",
                    'ﬄ' => "ffl",
                    'ﬅ' | 'ﬆ' => "st",
                    _ => return z.to_string(),
                };
                anzahl = anzahl.saturating_add(1);
                ersatz.to_owned()
            })
            .collect();
        melden(&mut bericht, "ligaturen", anzahl);
    }
    if opt.weiche_trennstriche {
        let mut anzahl = 0_u32;
        text.retain(|z| {
            if z == '\u{ad}' {
                anzahl = anzahl.saturating_add(1);
                false
            } else {
                true
            }
        });
        melden(&mut bericht, "weiche_trennstriche", anzahl);
    }
    // Entfernte Trennstriche können zuvor getrennte kombinierende Zeichen verbinden.
    if opt.nfc {
        let (normalisiert, anzahl) = nfc_normalisieren(&text);
        if anzahl > 0 {
            if let Some(regel) = bericht.regeln.iter_mut().find(|r| r.regel == "nfc") {
                regel.anzahl = regel.anzahl.saturating_add(anzahl);
            } else {
                bericht.regeln.insert(
                    0,
                    RegelBericht {
                        regel: "nfc",
                        anzahl,
                    },
                );
            }
        }
        text = normalisiert;
    }
    // Leerzeichen zuerst bereinigen, damit neu entstandene Muster sofort greifen.
    // Der Bericht behält dennoch die festgelegte Reihenfolge der Regeln.
    let mut leerzeichen_anzahl = 0;
    if opt.leerzeichen {
        let (formatiert, anzahl) = leerzeichen(&text);
        text = formatiert;
        leerzeichen_anzahl = anzahl;
    }
    // Neue Listenpräfixe müssen vor den übrigen Satzzeichenregeln sichtbar sein.
    let mut listen_anzahl = 0;
    if opt.listen {
        let (formatiert, anzahl) = listen(&text);
        text = formatiert;
        listen_anzahl = anzahl;
    }
    for (aktiv, regel, anwenden) in [
        (
            opt.anfuehrungszeichen,
            "anfuehrungszeichen",
            anfuehrungszeichen as fn(&str) -> (String, u32),
        ),
        (opt.gedankenstrich, "gedankenstrich", gedankenstriche),
        (opt.auslassung, "auslassung", auslassungen),
    ] {
        if aktiv {
            let (formatiert, anzahl) = anwenden(&text);
            text = formatiert;
            melden(&mut bericht, regel, anzahl);
        }
    }
    melden(&mut bericht, "leerzeichen", leerzeichen_anzahl);
    melden(&mut bericht, "listen", listen_anzahl);
    for (platzhalter, original) in marken {
        text = text.replace(&platzhalter, &original);
    }
    (text, bericht)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pruefen(vorher: &str, nachher: &str, regel: &'static str, anzahl: u32) {
        let (text, bericht) = formatieren(vorher, &FormatOptionen::default());
        assert_eq!(text, nachher);
        assert_eq!(bericht.regeln, vec![RegelBericht { regel, anzahl }]);
    }

    #[test]
    fn nfc_mit_anzahl() {
        pruefen("Cafe\u{301}", "Café", "nfc", 1);
    }

    #[test]
    fn nfc_zaehlt_mehrere_veraenderte_gruppen() {
        pruefen("A\u{301} e\u{301} o\u{308}", "Á é ö", "nfc", 3);
        pruefen("\u{1100}\u{1161} A\u{301}", "가 Á", "nfc", 2);
        let (_, bericht) = formatieren("A\u{301} e\u{ad}\u{301}", &FormatOptionen::default());
        assert_eq!(
            bericht.regeln[0],
            RegelBericht {
                regel: "nfc",
                anzahl: 2,
            }
        );
    }

    #[test]
    fn code_bleibt_vor_allen_regeln_geschuetzt() {
        for text in [
            "`\"A\u{301}  ﬃ\u{ad} - x...y\"`",
            "``\"a  b\" ` x...y``",
            "```rust\n\"A\u{301}  ﬃ\u{ad} - x...y\"\n```",
            "~~~\n\"a  b\" x...y\n~~~",
            "  ````rust\n```\n\"a  b\" x...y\n  ````\n",
            "```\n\"a  b\" x...y",
            "~~~\r\n\"a  b\" x...y\r\n",
        ] {
            let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
            assert_eq!(ergebnis, text);
            assert!(bericht.regeln.is_empty());
        }
        pruefen("`x...y` Ende...", "`x...y` Ende…", "auslassung", 1);
        pruefen(
            "```\nx...y\n```\nEnde...",
            "```\nx...y\n```\nEnde…",
            "auslassung",
            1,
        );
        pruefen("`offen\nx...y", "`offen\nx…y", "auslassung", 1);
    }

    #[test]
    fn satzzeichen_werden_nur_am_satzende_angerueckt() {
        let text = "Datei .env Pegel .5 2 : 1 :-) a ,b a ;b a !b a ?b a ./ a .)";
        assert_eq!(formatieren(text, &FormatOptionen::default()).0, text);
        pruefen(
            "A . B : C , D ; E ! F ?",
            "A. B: C, D; E! F?",
            "leerzeichen",
            6,
        );
        pruefen("A .\nB :", "A.\nB:", "leerzeichen", 2);
    }

    #[test]
    fn hard_breaks_tabs_und_tabellenabstaende_bleiben() {
        for text in [
            "A  \nB",
            "A   \r\nB",
            "A\tB\t \nC",
            "| A  B\t | C   |  \n",
            "A  B | C\t D  \n",
        ] {
            let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
            assert_eq!(ergebnis, text);
            assert!(bericht.regeln.is_empty());
        }
        pruefen("A  \n", "A\n", "leerzeichen", 1);
        pruefen("  \nB", "\nB", "leerzeichen", 1);
    }

    #[test]
    fn gedankenstriche_schonen_zitate_trennlinien_und_tabellen() {
        for text in [
            "> - Punkt",
            "> > - Punkt",
            "> Wort - Wort",
            "| - Punkt |",
            "A - B | C - D",
            "- - -",
            "* * *",
            "---",
        ] {
            let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
            assert_eq!(ergebnis, text);
            assert!(bericht.regeln.is_empty());
        }
        pruefen("Wort - Wort", "Wort – Wort", "gedankenstrich", 1);
    }

    #[test]
    fn eckige_masken_sind_auf_eine_zeile_und_80_zeichen_begrenzt() {
        pruefen("[offen\n\nEnde...]", "[offen\n\nEnde…]", "auslassung", 1);
        pruefen("[offen\rEnde...]", "[offen\rEnde…]", "auslassung", 1);
        let erlaubt = format!("[{}...]", "ä".repeat(77));
        assert_eq!(formatieren(&erlaubt, &FormatOptionen::default()).0, erlaubt);
        let zu_lang = format!("[{}...]", "ä".repeat(78));
        let erwartet = format!("[{}…]", "ä".repeat(78));
        pruefen(&zu_lang, &erwartet, "auslassung", 1);
    }

    #[test]
    fn nbsp_bei_einheiten_bleibt_und_wird_sonst_einmal_gezaehlt() {
        let text = "10\u{a0}km 5\u{a0}Äpfel";
        let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
        assert_eq!(ergebnis, text);
        assert!(bericht.regeln.is_empty());
        pruefen("A\u{a0}\u{a0}B", "A B", "leerzeichen", 1);
        pruefen("A\u{a0},", "A,", "leerzeichen", 1);
    }

    #[test]
    fn betonung_oeffnet_anfuehrungen_doppelpunkt_bleibt_unsicher() {
        pruefen(
            "*\"Hallo\"* _\"Welt\"_",
            "*„Hallo“* _„Welt“_",
            "anfuehrungszeichen",
            4,
        );
        let text = "Er sagte:\"Hallo\"";
        assert_eq!(formatieren(text, &FormatOptionen::default()).0, text);
    }

    #[test]
    fn utf8_an_maskengrenzen_panikt_nicht() {
        for zeichen in ['ä', '界', '🦀', '\u{301}', '\0', '\u{e000}', '\u{10ffff}'] {
            for text in [
                format!("[{zeichen}\n...]"),
                format!("``{zeichen}`{zeichen}``"),
                format!("~~~\n{zeichen}\n~~~\n{zeichen} ."),
                format!("{zeichen}\u{a0}km | {zeichen} - x"),
                format!("• {zeichen}\n1) {zeichen}\n2) {zeichen}"),
            ] {
                let (einmal, _) = formatieren(&text, &FormatOptionen::default());
                let (zweimal, bericht) = formatieren(&einmal, &FormatOptionen::default());
                assert_eq!(einmal, zweimal);
                assert!(bericht.regeln.is_empty());
            }
        }
    }

    #[test]
    fn ligaturen_mit_anzahl() {
        pruefen("ﬀ ﬁ ﬂ ﬃ ﬄ ﬅ ﬆ", "ff fi fl ffi ffl st st", "ligaturen", 7);
    }

    #[test]
    fn weiche_trennstriche_mit_anzahl() {
        pruefen("Sil\u{ad}be", "Silbe", "weiche_trennstriche", 1);
    }

    #[test]
    fn anfuehrungszeichen_mit_anzahl() {
        pruefen(
            "\"Hallo\" (\"Welt\")",
            "„Hallo“ („Welt“)",
            "anfuehrungszeichen",
            4,
        );
    }

    #[test]
    fn gedankenstrich_mit_anzahl() {
        pruefen("Wort - Wort", "Wort – Wort", "gedankenstrich", 1);
    }

    #[test]
    fn auslassung_mit_anzahl() {
        pruefen("Warte... .... .....", "Warte… .... .....", "auslassung", 1);
    }

    #[test]
    fn leerzeichen_mit_anzahl() {
        pruefen(
            "  A  B\tC , D\u{a0}E  \n",
            "  A B\tC, D E\n",
            "leerzeichen",
            4,
        );
    }

    #[test]
    fn listen_normalisieren_zeichen_und_aufsteigende_nachbarn() {
        // Gegenprobe: Ohne Listenregel bleiben Zeichen und Klammern unverändert.
        for zeichen in ['•', '·', '‣', '◦', '▪', '●'] {
            pruefen(&format!("{zeichen} Punkt"), "- Punkt", "listen", 1);
            pruefen(&format!("   {zeichen}\t🦀"), "   - 🦀", "listen", 1);
        }
        pruefen("1) Eins\n2) Zwei", "1. Eins\n2. Zwei", "listen", 2);
        pruefen(
            "18446744073709551615) Eins\n18446744073709551616) Zwei",
            "18446744073709551615. Eins\n18446744073709551616. Zwei",
            "listen",
            2,
        );
        pruefen(
            "  9) Neun\r\n  10) Zehn\r\n11) Elf\r\n",
            "  9. Neun\r\n  10. Zehn\r\n11. Elf\r\n",
            "listen",
            3,
        );
        pruefen(
            "• {{S. 12}} <Anna> [• x]\n1) `• x`\n2) Ende",
            "- {{S. 12}} <Anna> [• x]\n1. `• x`\n2. Ende",
            "listen",
            3,
        );
    }

    #[test]
    fn listen_schonen_kontrolltexte_und_geschuetzte_inhalte() {
        // Gegenprobe: Unbedingtes Ersetzen verletzt Einzelnummern und Schutzbereiche.
        for text in [
            "– Wohin?\n— Dorthin!",
            "1) Punkt",
            "2) Zwei\n4) Vier",
            "1) Eins\n\n2) Zwei",
            "1)\tEins\n2)\tZwei",
            "```\n• x\n1) Eins\n2) Zwei\n```",
            "~~~\n• x\n~~~",
            "`• x`\n``1) Eins``",
            "| • |\n1) Eins | A\n2) Zwei | B",
            "> • x\n> 1) Eins\n> 2) Zwei",
            "<Anna> • x\n<Anna> 1) Eins\n<Anna> 2) Zwei",
            "[• x]\n<• x>\n{{S. 12}}",
            "    • Punkt\n    1) Eins\n    2) Zwei",
            "•\n·\n‣\n◦\n▪\n●",
        ] {
            let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
            assert_eq!(ergebnis, text);
            assert!(bericht.regeln.is_empty());
        }
        let opt = FormatOptionen {
            leerzeichen: false,
            ..FormatOptionen::default()
        };
        for text in ["• ", "•\t", "1) \n2) "] {
            let (ergebnis, bericht) = formatieren(text, &opt);
            assert_eq!(ergebnis, text);
            assert!(bericht.regeln.is_empty());
        }
    }

    #[test]
    fn listen_bleiben_auch_mit_anderen_regeln_idempotent() {
        // Gegenprobe: Neu erzeugte Abstände und Striche dürfen keinen Folgelauf brauchen.
        for text in [
            "•\t.",
            "•\t- x",
            "•  Punkt\n1) Eins\n2) Zwei",
            "◦\t\t🦀  Ende...",
            "•\t\u{a0}🦀",
            "1) \"Ja\"\n2) Wort - Wort",
        ] {
            let (einmal, bericht) = formatieren(text, &FormatOptionen::default());
            assert_ne!(einmal, text);
            assert!(bericht.regeln.iter().any(|r| r.regel == "listen"));
            let (zweimal, bericht) = formatieren(&einmal, &FormatOptionen::default());
            assert_eq!(einmal, zweimal);
            assert!(bericht.regeln.is_empty());
        }
        pruefen("•\t.", "- .", "listen", 1);
        let (_, bericht) = formatieren("•  Punkt", &FormatOptionen::default());
        assert_eq!(
            bericht.regeln,
            vec![
                RegelBericht {
                    regel: "leerzeichen",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "listen",
                    anzahl: 1,
                },
            ]
        );
    }

    #[test]
    fn kontrolltexte_bleiben_bytegleich() {
        for text in [
            "Ein sauberer Absatz mit Satzzeichen, Umlauten und Schluss.",
            "{{S. 3}} <Anna> Hallo. [lacht]",
            "Rheinland-Pfalz\n- Punkt\n  - Punkt\n1 - 3 und 1-3",
            "Englische “Worte” und deutsche „Worte“.",
            "  Einzug\n\tEinzug\r\n",
        ] {
            let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
            assert_eq!(ergebnis.as_bytes(), text.as_bytes());
            assert!(bericht.regeln.is_empty());
        }
    }

    #[test]
    fn marken_sind_vor_allen_regeln_geschuetzt() {
        let text = concat!(
            "\"<A\u{301}  ﬃ\u{ad} - ...>\"  {{S. 12}} ",
            "...[\"A\u{301}\"  ﬃ\u{ad} - ...] ,"
        );
        let erwartet = concat!(
            "„<A\u{301}  ﬃ\u{ad} - ...>“ {{S. 12}} ",
            "…[\"A\u{301}\"  ﬃ\u{ad} - ...],"
        );
        let (ergebnis, bericht) = formatieren(text, &FormatOptionen::default());
        assert_eq!(ergebnis, erwartet);
        assert_eq!(
            bericht.regeln.iter().map(|r| r.regel).collect::<Vec<_>>(),
            vec!["anfuehrungszeichen", "auslassung", "leerzeichen"]
        );
    }

    #[test]
    fn unausgeglichene_anfuehrungen_bleiben_absatzweise_stehen() {
        pruefen(
            "\"Offen\n\nEnde\"\n\n\"Gut\"",
            "\"Offen\n\nEnde\"\n\n„Gut“",
            "anfuehrungszeichen",
            2,
        );
        let text = "Wort\" Wort\"";
        assert_eq!(formatieren(text, &FormatOptionen::default()).0, text);
    }

    #[test]
    fn jede_regel_ist_einzeln_abschaltbar() {
        for (index, text) in [
            "A\u{301}",
            "ﬃ",
            "Sil\u{ad}be",
            "\"Wort\"",
            "A - B",
            "Ende...",
            "A  B",
            "• Punkt\n1) Eins\n2) Zwei",
        ]
        .iter()
        .enumerate()
        {
            let mut opt = FormatOptionen::default();
            let schalter = [
                &mut opt.nfc,
                &mut opt.ligaturen,
                &mut opt.weiche_trennstriche,
                &mut opt.anfuehrungszeichen,
                &mut opt.gedankenstrich,
                &mut opt.auslassung,
                &mut opt.leerzeichen,
                &mut opt.listen,
            ];
            *schalter
                .into_iter()
                .nth(index)
                .expect("Vorhandener Regelschalter") = false;
            let (ergebnis, bericht) = formatieren(text, &opt);
            assert_eq!(ergebnis, *text);
            assert!(bericht.regeln.is_empty());
        }
    }

    #[test]
    fn zweiter_lauf_ist_ohne_aenderung() {
        for text in [
            "\"Cafe\u{301}  ﬃ\u{ad} - Ende...\"  ",
            "{{S. 3}} <Anna> [lacht]  Hallo , Welt...",
            "\"Offen\n\n\"Gut\" - Ende...",
            "  A\t B\u{a0}C !\r\n- Punkt\n1 - 3",
            ".... ..... ... ﬀ ﬆ Sil\u{ad}be",
            "\u{e000}\u{e010}\u{e001} [ﬃ  ...] \"Ja\"",
            "A  -\t B und Ende . . .",
            "A\u{ad}\u{301} ﬃ\u{301}",
            "```rust\n\"a  b\" x...y\n```\nEnde...",
            "``x...y ` a  b`` und \"Ja\"",
            "> > - Punkt\n> Wort - Wort\n- - -",
            "| A  B - C | D\t E |\nA - B | C  D",
            "A  \nB   \r\nC",
        ] {
            let (einmal, _) = formatieren(text, &FormatOptionen::default());
            let (zweimal, bericht) = formatieren(&einmal, &FormatOptionen::default());
            assert_eq!(einmal, zweimal);
            assert!(bericht.regeln.is_empty());
        }
    }

    #[test]
    fn mischtext_meldet_alle_regeln_in_fester_reihenfolge() {
        let (text, bericht) = formatieren(
            "\"Cafe\u{301}  ﬃ\u{ad} - Ende...\"\n• Punkt",
            &FormatOptionen::default(),
        );
        assert_eq!(text, "„Café ffi – Ende…“\n- Punkt");
        assert_eq!(
            bericht.regeln,
            vec![
                RegelBericht {
                    regel: "nfc",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "ligaturen",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "weiche_trennstriche",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "anfuehrungszeichen",
                    anzahl: 2,
                },
                RegelBericht {
                    regel: "gedankenstrich",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "auslassung",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "leerzeichen",
                    anzahl: 1,
                },
                RegelBericht {
                    regel: "listen",
                    anzahl: 1,
                },
            ]
        );
    }

    #[test]
    fn buchstaben_und_ziffern_gehen_nicht_verloren() {
        let text = "\"Anna  123\" - Sil\u{ad}be... {{S. 12}} <Berta> [lacht]";
        let (ergebnis, _) = formatieren(text, &FormatOptionen::default());
        let zaehlen = |s: &str| s.chars().filter(|z| z.is_alphanumeric()).count();
        assert_eq!(zaehlen(text), zaehlen(&ergebnis));
    }
}
