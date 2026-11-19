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
        return text.find(']').map(|ende| ende + 1);
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
        if let Some(laenge) = geschuetzte_laenge(rest) {
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
            marken.push((platzhalter, rest[..laenge].to_owned()));
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
            let oeffnend = vorher.is_none_or(|z: char| z.is_whitespace() || z == '(' || z == '[');
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
                .all(|z| z.is_whitespace());
            ersetzen = !zeilenanfang
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

fn leerzeichen(text: &str) -> (String, u32) {
    let mut anzahl = 0_u32;
    let normalisiert: String = text
        .chars()
        .map(|z| {
            if z == '\u{a0}' {
                anzahl = anzahl.saturating_add(1);
                ' '
            } else {
                z
            }
        })
        .collect();
    let mut ergebnis = String::new();
    for zeile in normalisiert.split_inclusive('\n') {
        let (inhalt, ende) = if let Some(rest) = zeile.strip_suffix("\r\n") {
            (rest, "\r\n")
        } else if let Some(rest) = zeile.strip_suffix('\n') {
            (rest, "\n")
        } else {
            (zeile, "")
        };
        let mut zeichen = inhalt.char_indices().peekable();
        while let Some((position, zeichen_aktuell)) = zeichen.next() {
            if zeichen_aktuell != ' ' && zeichen_aktuell != '\t' {
                ergebnis.push(zeichen_aktuell);
                continue;
            }
            let mut ende_position = position + zeichen_aktuell.len_utf8();
            while let Some(&(index, z)) = zeichen.peek() {
                if z != ' ' && z != '\t' {
                    break;
                }
                ende_position = index + z.len_utf8();
                zeichen.next();
            }
            let original = &inhalt[position..ende_position];
            let danach = zeichen.peek().map(|&(_, z)| z);
            let ersatz = if danach.is_none() {
                ""
            } else if position == 0 {
                original
            } else if danach.is_some_and(|z| matches!(z, ',' | ';' | ':' | '!' | '?'))
                || (danach == Some('.') && !inhalt[ende_position..].starts_with(".."))
            {
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

pub fn formatieren(text: &str, opt: &FormatOptionen) -> (String, FormatBericht) {
    let (mut text, marken) = maskieren(text);
    let mut bericht = FormatBericht::default();
    if opt.nfc {
        let normalisiert: String = text.nfc().collect();
        melden(&mut bericht, "nfc", u32::from(normalisiert != text));
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
        let normalisiert: String = text.nfc().collect();
        if normalisiert != text && !bericht.regeln.iter().any(|r| r.regel == "nfc") {
            bericht.regeln.insert(
                0,
                RegelBericht {
                    regel: "nfc",
                    anzahl: 1,
                },
            );
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
            "  A B C, D E\n",
            "leerzeichen",
            5,
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
            "\"Cafe\u{301}  ﬃ\u{ad} - Ende...\"",
            &FormatOptionen::default(),
        );
        assert_eq!(text, "„Café ffi – Ende…“");
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
