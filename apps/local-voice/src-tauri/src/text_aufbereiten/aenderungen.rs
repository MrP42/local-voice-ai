//! Regelweise Vorschläge mit Byte-Bereichen in der unveränderten Basis.

use super::format::{formatieren, FormatOptionen};
use crate::aufbereitung::{geschuetzte_bereiche, Aenderung, AenderungArt, AenderungStatus};

fn regeln(opt: &FormatOptionen) -> [(&'static str, bool); 7] {
    [
        ("nfc", opt.nfc),
        ("ligaturen", opt.ligaturen),
        ("weiche_trennstriche", opt.weiche_trennstriche),
        ("anfuehrungszeichen", opt.anfuehrungszeichen),
        ("gedankenstrich", opt.gedankenstrich),
        ("auslassung", opt.auslassung),
        ("leerzeichen", opt.leerzeichen),
    ]
}

fn nur_regel(regel: &str) -> FormatOptionen {
    FormatOptionen {
        nfc: regel == "nfc",
        ligaturen: regel == "ligaturen",
        weiche_trennstriche: regel == "weiche_trennstriche",
        anfuehrungszeichen: regel == "anfuehrungszeichen",
        gedankenstrich: regel == "gedankenstrich",
        auslassung: regel == "auslassung",
        leerzeichen: regel == "leerzeichen",
    }
}

fn ueberlappt(links: [usize; 2], rechts: [usize; 2]) -> bool {
    // Dieselbe Sortierung und Grenzregel wie in pruefe_aenderungen, auch für Einfügungen.
    let (vorher, nachher) = if links <= rechts {
        (links, rechts)
    } else {
        (rechts, links)
    };
    nachher[0] < vorher[1] || (nachher[0] == vorher[0] && nachher[0] == vorher[1])
}

fn zeilen_aenderungen(basis: &str, formatiert: &str, regel: &str) -> Vec<Aenderung> {
    let vorher: Vec<_> = basis.split('\n').collect();
    let nachher: Vec<_> = formatiert.split('\n').collect();
    if vorher.len() != nachher.len() {
        return Vec::new();
    }
    let mut ergebnis = Vec::new();
    let mut position = 0;
    for (alt, neu) in vorher.into_iter().zip(nachher) {
        if alt != neu {
            let praefix: usize = alt
                .chars()
                .zip(neu.chars())
                .take_while(|(a, b)| a == b)
                .map(|(z, _)| z.len_utf8())
                .sum();
            let suffix: usize = alt[praefix..]
                .chars()
                .rev()
                .zip(neu[praefix..].chars().rev())
                .take_while(|(a, b)| a == b)
                .map(|(z, _)| z.len_utf8())
                .sum();
            ergebnis.push(Aenderung {
                id: String::new(),
                art: AenderungArt::Regel(regel.to_owned()),
                bereich: [position + praefix, position + alt.len() - suffix],
                alt: alt[praefix..alt.len() - suffix].to_owned(),
                neu: neu[praefix..neu.len() - suffix].to_owned(),
                grund_code: regel.to_owned(),
                zaehler: None,
                gruppe: Some(regel.to_owned()),
                status: AenderungStatus::Offen,
            });
        }
        position += alt.len() + 1;
    }
    ergebnis
}

/// Alle Formatregeln als Aenderungsvorschlaege (Status Offen).
/// Gruppe = Regel-ID ("ligaturen", "anfuehrungszeichen", ...).
pub fn regel_aenderungen(basis: &str, opt: &FormatOptionen) -> Vec<Aenderung> {
    let marken = geschuetzte_bereiche(basis);
    let mut ergebnis: Vec<Aenderung> = Vec::new();
    for (regel, aktiv) in regeln(opt) {
        if !aktiv {
            continue;
        }
        let (formatiert, _) = formatieren(basis, &nur_regel(regel));
        let mut nummer = 0;
        for mut aenderung in zeilen_aenderungen(basis, &formatiert, regel) {
            let [start, ende] = aenderung.bereich;
            let geschuetzt = marken.iter().any(|&(marke_start, marke_ende)| {
                if start == ende {
                    marke_start < start && start < marke_ende
                } else {
                    start < marke_ende && marke_start < ende
                }
            });
            if geschuetzt
                || ergebnis
                    .iter()
                    .any(|a| ueberlappt(a.bereich, aenderung.bereich))
            {
                continue;
            }
            nummer += 1;
            aenderung.id = format!("{regel}-{nummer}");
            ergebnis.push(aenderung);
        }
    }
    ergebnis.sort_by_key(|a| a.bereich);
    ergebnis
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aufbereitung::{ergebnis_text, pruefe_aenderungen, ErgebnisFehler};

    #[test]
    fn beispieltexte_mit_vollannahme_teilannahme_und_idempotenz() {
        // Hier sind alle Regelsegmente unabhängig und berühren keine Schutzbereiche.
        let beispiele = [
            (
                "alle Regeln",
                "Cafe\u{301}\nﬃ\nSil\u{ad}be\n\"Ja\"\nA - B\nEnde...\nA  B",
            ),
            (
                "Mischtext",
                "A\u{301} ﬃ Sil\u{ad}be Wort - Wort Ende... A  B",
            ),
            ("Buchseiten", "{{S. 1}}\nCafe\u{301}\n{{S. 2}}\nEnde..."),
            ("Gespräch", "<Anna> [lacht] 00:12\n\"Hallo\"\nA  B\nEnde..."),
            (
                "Markdown",
                "```\nﬃ  ...\n```\n`x...y`\n| A  B |\n> Wort - Wort\nA  \nEnde...",
            ),
            ("Ligaturen", "ﬀ ﬁ ﬂ ﬃ ﬄ ﬅ ﬆ"),
            ("NFC", "A\u{301} e\u{301} o\u{308}"),
            ("Hangul", "\u{1100}\u{1161} A\u{301}"),
            ("Trennstriche", "Sil\u{ad}be\nWör\u{ad}ter"),
            ("Zitate", "\"Hallo\" (\"Welt\")"),
            ("Betonung", "*\"Hallo\"* _\"Welt\"_"),
            ("Offene Zitate", "\"Offen\n\nEnde\"\n\n\"Gut\""),
            ("Gedankenstriche", "Wort - Wort\n1 - 3\n- Punkt"),
            ("Auslassung", "Warte... .... ....."),
            ("Leerzeichen", "  A  B\tC , D\u{a0}E  \n"),
            ("Einheiten", "10\u{a0}km A\u{a0}\u{a0}B"),
            ("UTF-8", "🦀 ä ﬃ\n界 Cafe\u{301}\n🦀  Öl"),
            ("CRLF", "Ä  B\r\nEnde...\r\n"),
            ("Zeilenende", "A  \n"),
            ("Leerer Text", ""),
        ];
        let opt = FormatOptionen::default();
        for (name, basis) in beispiele {
            let aenderungen = regel_aenderungen(basis, &opt);
            assert_eq!(pruefe_aenderungen(basis, &aenderungen), Ok(()), "{name}");
            assert!(aenderungen.windows(2).all(|a| a[0].bereich <= a[1].bereich));
            for a in &aenderungen {
                assert_eq!(a.status, AenderungStatus::Offen, "{name}");
                assert_eq!(a.art, AenderungArt::Regel(a.grund_code.clone()));
                assert_eq!(a.gruppe.as_deref(), Some(a.grund_code.as_str()));
                assert_eq!(a.zaehler, None);
                assert_eq!(basis.get(a.bereich[0]..a.bereich[1]), Some(a.alt.as_str()));
            }
            let mut angenommen = aenderungen.clone();
            for a in &mut angenommen {
                a.status = AenderungStatus::Angenommen;
            }
            let text = ergebnis_text(basis, &angenommen).unwrap();
            assert_eq!(text, formatieren(basis, &opt).0, "{name}");
            assert!(regel_aenderungen(&text, &opt).is_empty(), "{name}");
            // Jede vorkommende Gruppe wird unabhängig von den anderen angenommen.
            for (regel, _) in regeln(&opt) {
                if !aenderungen.iter().any(|a| a.grund_code == regel) {
                    continue;
                }
                let mut teilweise = aenderungen.clone();
                for a in &mut teilweise {
                    if a.grund_code == regel {
                        a.status = AenderungStatus::Angenommen;
                    }
                }
                assert_eq!(
                    ergebnis_text(basis, &teilweise).unwrap(),
                    formatieren(basis, &nur_regel(regel)).0,
                    "{name}: {regel}"
                );
            }
        }
    }

    #[test]
    fn kontrolltexte_liefern_keine_vorschlaege() {
        for basis in [
            "Ein sauberer Absatz mit Umlauten und Schluss.",
            "{{S. 3}} <Anna> Hallo. [lacht] 00:12",
            "Rheinland-Pfalz\n- Punkt\n  - Punkt\n1 - 3 und 1-3",
            "```rust\n\"A\u{301}  ﬃ\u{ad} - x...y\"\n```\n`a  b...`",
        ] {
            assert!(regel_aenderungen(basis, &FormatOptionen::default()).is_empty());
        }
    }

    #[test]
    fn deaktivierte_regeln_liefern_keine_vorschlaege() {
        let basis = "A\u{301}\nﬃ\nSil\u{ad}be\n\"Ja\"\nA - B\nEnde...\nA  B";
        assert!(regel_aenderungen(basis, &nur_regel("")).is_empty());
        for (regel, _) in regeln(&FormatOptionen::default()) {
            let a = regel_aenderungen(basis, &nur_regel(regel));
            assert_eq!(a.len(), 1);
            assert_eq!(a[0].grund_code, regel);
            assert_eq!(a[0].id, format!("{regel}-1"));
        }
    }

    #[test]
    fn byte_bereiche_bleiben_in_der_basis_und_ids_zaehlen_je_regel() {
        let basis = "🦀 ﬃ\nä ﬀ\nÖ...";
        let a = regel_aenderungen(basis, &FormatOptionen::default());
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].bereich, [5, 8]);
        assert_eq!(a[1].bereich, [12, 15]);
        assert_eq!(a[2].bereich, [18, 21]);
        assert_eq!(a[0].id, "ligaturen-1");
        assert_eq!(a[1].id, "ligaturen-2");
        assert_eq!(a[2].id, "auslassung-1");
    }

    #[test]
    fn ein_segment_je_zeile_und_zeichengrenzen() {
        let a = zeilen_aenderungen("ä x ö x 🦀", "ä y ö z 🦀", "test");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].bereich, [3, 9]);
        assert_eq!(a[0].alt, "x ö x");
        assert_eq!(a[0].neu, "y ö z");
        let a = zeilen_aenderungen("ä", "ö", "test");
        assert_eq!(a[0].bereich, [0, 2]);
        assert_eq!(a[0].alt, "ä");
        assert_eq!(a[0].neu, "ö");
    }

    #[test]
    fn einfuegen_loeschen_und_abweichende_zeilenzahl() {
        let a = zeilen_aenderungen("🦀ö", "🦀äö", "test");
        assert_eq!(a[0].bereich, [4, 4]);
        assert_eq!(a[0].alt, "");
        assert_eq!(a[0].neu, "ä");
        let a = regel_aenderungen("ä\u{ad}ö", &FormatOptionen::default());
        assert_eq!(a[0].bereich, [2, 4]);
        assert_eq!(a[0].alt, "\u{ad}");
        assert_eq!(a[0].neu, "");
        assert!(zeilen_aenderungen("a\nb", "ab", "test").is_empty());
        assert!(zeilen_aenderungen("ab", "a\nb", "test").is_empty());
    }

    #[test]
    fn schutz_verwirft_auch_unveraenderte_marken_im_segment() {
        for basis in [
            "\"{{S. 12}}\"",
            "\"<Anna>\"",
            "\"[lacht]\"",
            "\"00:12\"",
            "ﬃ `Code` ﬀ",
        ] {
            let a = regel_aenderungen(basis, &FormatOptionen::default());
            if basis.contains('`') {
                // Code ist im Formatter maskiert, aber kein Schutzbereich der Fassungen.
                assert_eq!(a.len(), 1);
            } else {
                assert!(a.is_empty());
            }
            assert_eq!(pruefe_aenderungen(basis, &a), Ok(()));
        }
        // Der Schutz der Fassungen erkennt auch Marken außerhalb der Formatter-Masken.
        for basis in ["{{S.12}}", "{{S.  12}}", "[ ]", "< >"] {
            assert!(regel_aenderungen(basis, &FormatOptionen::default()).is_empty());
        }
        let basis = format!("[{}ﬃ]", "ä".repeat(81));
        assert_ne!(formatieren(&basis, &FormatOptionen::default()).0, basis);
        assert!(regel_aenderungen(&basis, &FormatOptionen::default()).is_empty());
    }

    #[test]
    fn fruehere_regel_gewinnt_und_verworfene_regel_bleibt_offen() {
        let basis = "ﬃ ... ﬀ";
        let mut a = regel_aenderungen(basis, &FormatOptionen::default());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].grund_code, "ligaturen");
        assert_eq!(pruefe_aenderungen(basis, &a), Ok(()));
        a[0].status = AenderungStatus::Angenommen;
        let text = ergebnis_text(basis, &a).unwrap();
        assert_eq!(text, "ffi ... ff");
        // Nach Verzicht ist globale Idempotenz unmöglich: die Auslassung bleibt übrig.
        let rest = regel_aenderungen(&text, &FormatOptionen::default());
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].grund_code, "auslassung");
    }

    #[test]
    fn gruppenstatus_muss_einheitlich_sein() {
        let basis = "ﬃ\nﬀ";
        let mut a = regel_aenderungen(basis, &FormatOptionen::default());
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].gruppe, a[1].gruppe);
        for status in [AenderungStatus::Angenommen, AenderungStatus::Abgelehnt] {
            a[1].status = status;
            assert_eq!(pruefe_aenderungen(basis, &a), Err(ErgebnisFehler::Gruppe));
            a[1].status = AenderungStatus::Offen;
            assert_eq!(pruefe_aenderungen(basis, &a), Ok(()));
        }
    }

    #[test]
    fn ueberlappung_entspricht_der_pruefung_auch_an_einfuegegrenzen() {
        for links in [[0, 0], [0, 1], [1, 1], [1, 2], [2, 2]] {
            for rechts in [[0, 0], [0, 1], [1, 1], [1, 2], [2, 2]] {
                let mut a = zeilen_aenderungen("ab", "AB", "test").remove(0);
                a.bereich = links;
                a.alt = "ab"[links[0]..links[1]].to_owned();
                a.gruppe = None;
                let mut b = a.clone();
                b.bereich = rechts;
                b.alt = "ab"[rechts[0]..rechts[1]].to_owned();
                assert_eq!(
                    ueberlappt(links, rechts),
                    pruefe_aenderungen("ab", &[a, b]) == Err(ErgebnisFehler::Ueberlappung)
                );
            }
        }
    }

    #[test]
    fn utf8_an_grenzen_bleibt_gueltig() {
        for zeichen in ['ä', '界', '🦀', '\u{301}', '\0', '\u{e000}', '\u{10ffff}'] {
            for basis in [
                format!("{zeichen} ﬃ\n{zeichen}  Öl\nEnde..."),
                format!("[{zeichen}\n...]"),
                format!("``{zeichen}`{zeichen}``"),
                format!("~~~\n{zeichen}\n~~~\n{zeichen} ."),
                format!("\"{zeichen}\" 00:12"),
            ] {
                let mut a = regel_aenderungen(&basis, &FormatOptionen::default());
                assert_eq!(pruefe_aenderungen(&basis, &a), Ok(()));
                for vorschlag in &mut a {
                    vorschlag.status = AenderungStatus::Angenommen;
                }
                assert!(ergebnis_text(&basis, &a).is_ok());
            }
        }
    }
}
