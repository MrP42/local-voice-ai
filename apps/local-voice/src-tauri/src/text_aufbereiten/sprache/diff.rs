//! Wortweise Vorschläge mit begrenztem Myers-Diff und bytegenauen Bereichen.

use super::vertrag::{SprachGruppe, Zeile};

const MAX_DISTANZ: usize = 2000;

#[derive(Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    start: usize,
}

fn tokenisiere(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut zeichen = text.char_indices().peekable();
    while let Some((start, erstes)) = zeichen.next() {
        let mut ende = start + erstes.len_utf8();
        if erstes.is_whitespace() {
            while let Some(&(position, c)) = zeichen.peek() {
                if !c.is_whitespace() {
                    break;
                }
                ende = position + c.len_utf8();
                zeichen.next();
            }
        } else if erstes.is_alphanumeric() {
            let mut letztes_alphanumerisch = true;
            while let Some(&(position, c)) = zeichen.peek() {
                if c.is_alphanumeric() {
                    ende = position + c.len_utf8();
                    letztes_alphanumerisch = true;
                    zeichen.next();
                } else if (c == '\'' || c == '’') && letztes_alphanumerisch {
                    // Ein Apostroph gehört nur zwischen zwei Wortzeichen zum Wort.
                    let mut vorschau = zeichen.clone();
                    vorschau.next();
                    if !vorschau.peek().is_some_and(|(_, n)| n.is_alphanumeric()) {
                        break;
                    }
                    ende = position + c.len_utf8();
                    letztes_alphanumerisch = false;
                    zeichen.next();
                } else {
                    break;
                }
            }
        }
        tokens.push(Token {
            text: &text[start..ende],
            start,
        });
    }
    tokens
}

#[derive(Clone, Copy)]
enum Schritt {
    Gleich,
    Loeschen,
    Einfuegen,
}

// Jede Spur enthält nur die erreichbaren Diagonalen -d, -d+2, ..., d.
fn spurwert(spur: &[usize], d: usize, k: isize) -> usize {
    spur[((k + d as isize) / 2) as usize]
}

fn rueckweg(spuren: &[Vec<usize>], mut x: usize, mut y: usize) -> Vec<Schritt> {
    let mut schritte = Vec::new();
    for d in (1..spuren.len()).rev() {
        let k = x as isize - y as isize;
        let vorher = &spuren[d - 1];
        let einfuegen = k == -(d as isize)
            || (k != d as isize && spurwert(vorher, d - 1, k - 1) < spurwert(vorher, d - 1, k + 1));
        let vorher_k = if einfuegen { k + 1 } else { k - 1 };
        let vorher_x = spurwert(vorher, d - 1, vorher_k);
        let vorher_y = (vorher_x as isize - vorher_k) as usize;
        while x > vorher_x && y > vorher_y {
            schritte.push(Schritt::Gleich);
            x -= 1;
            y -= 1;
        }
        if einfuegen {
            schritte.push(Schritt::Einfuegen);
        } else {
            schritte.push(Schritt::Loeschen);
        }
        x = vorher_x;
        y = vorher_y;
    }
    while x > 0 && y > 0 {
        schritte.push(Schritt::Gleich);
        x -= 1;
        y -= 1;
    }
    schritte.reverse();
    schritte
}

fn myers(alt: &[Token<'_>], neu: &[Token<'_>]) -> Option<Vec<Schritt>> {
    if alt.len().abs_diff(neu.len()) > MAX_DISTANZ {
        return None;
    }
    let mut spuren: Vec<Vec<usize>> = Vec::new();
    for d in 0..=MAX_DISTANZ.min(alt.len().saturating_add(neu.len())) {
        let mut spur = Vec::with_capacity(d + 1);
        for index in 0..=d {
            let k = 2 * index as isize - d as isize;
            let mut x = if d == 0 {
                0
            } else {
                let vorher = &spuren[d - 1];
                if k == -(d as isize)
                    || (k != d as isize
                        && spurwert(vorher, d - 1, k - 1) < spurwert(vorher, d - 1, k + 1))
                {
                    spurwert(vorher, d - 1, k + 1)
                } else {
                    spurwert(vorher, d - 1, k - 1) + 1
                }
            };
            let mut y = (x as isize - k) as usize;
            while x < alt.len() && y < neu.len() && alt[x].text == neu[y].text {
                x += 1;
                y += 1;
            }
            spur.push(x);
            if x == alt.len() && y == neu.len() {
                spuren.push(spur);
                return Some(rueckweg(&spuren, x, y));
            }
        }
        spuren.push(spur);
    }
    None
}

#[derive(Clone, Copy)]
struct Hunk {
    alt_von: usize,
    alt_bis: usize,
    neu_von: usize,
    neu_bis: usize,
}

fn position(tokens: &[Token<'_>], index: usize, laenge: usize) -> usize {
    tokens.get(index).map_or(laenge, |t| t.start)
}

/// Ganze Token; Einfügungen besitzen einen leeren Bereich in `alt`.
pub fn wort_hunks(alt: &str, neu: &str) -> Vec<([usize; 2], String)> {
    if alt == neu {
        return Vec::new();
    }
    let a = tokenisiere(alt);
    let b = tokenisiere(neu);
    let mut praefix = 0;
    while praefix < a.len().min(b.len()) && a[praefix].text == b[praefix].text {
        praefix += 1;
    }
    let mut ende_a = a.len();
    let mut ende_b = b.len();
    while ende_a > praefix && ende_b > praefix && a[ende_a - 1].text == b[ende_b - 1].text {
        ende_a -= 1;
        ende_b -= 1;
    }
    let mut hunks = Vec::new();
    if let Some(schritte) = myers(&a[praefix..ende_a], &b[praefix..ende_b]) {
        let mut i = praefix;
        let mut j = praefix;
        let mut offen: Option<Hunk> = None;
        for schritt in schritte {
            if matches!(schritt, Schritt::Gleich) {
                if let Some(hunk) = offen.take() {
                    hunks.push(hunk);
                }
                i += 1;
                j += 1;
            } else {
                let hunk = offen.get_or_insert(Hunk {
                    alt_von: i,
                    alt_bis: i,
                    neu_von: j,
                    neu_bis: j,
                });
                match schritt {
                    Schritt::Loeschen => i += 1,
                    Schritt::Einfuegen => j += 1,
                    Schritt::Gleich => {}
                }
                hunk.alt_bis = i;
                hunk.neu_bis = j;
            }
        }
        if let Some(hunk) = offen {
            hunks.push(hunk);
        }
    } else {
        // Die unveränderten Ränder bleiben auch bei zu großer Distanz erhalten.
        hunks.push(Hunk {
            alt_von: praefix,
            alt_bis: ende_a,
            neu_von: praefix,
            neu_bis: ende_b,
        });
    }
    let mut vereinigt: Vec<Hunk> = Vec::new();
    for hunk in hunks {
        if let Some(letzter) = vereinigt.last_mut() {
            if hunk.alt_von == letzter.alt_bis + 1
                && hunk.neu_von == letzter.neu_bis + 1
                && a[letzter.alt_bis].text.chars().all(char::is_whitespace)
            {
                letzter.alt_bis = hunk.alt_bis;
                letzter.neu_bis = hunk.neu_bis;
                continue;
            }
        }
        vereinigt.push(hunk);
    }
    vereinigt
        .into_iter()
        .map(|h| {
            let bereich = [
                position(&a, h.alt_von, alt.len()),
                position(&a, h.alt_bis, alt.len()),
            ];
            let neu_von = position(&b, h.neu_von, neu.len());
            let neu_bis = position(&b, h.neu_bis, neu.len());
            let ersatz = &neu[neu_von..neu_bis];
            (bereich, ersatz.to_owned())
        })
        .collect()
}

pub fn gruppe_fuer_zeile(zeile: &Zeile, alt: &str, neu: &str) -> Option<SprachGruppe> {
    if alt == neu {
        return None;
    }
    let teile = wort_hunks(alt, neu)
        .into_iter()
        .map(|([von, bis], ersatz)| {
            (
                [zeile.bereich[0] + von, zeile.bereich[0] + bis],
                alt[von..bis].to_owned(),
                ersatz,
            )
        })
        .collect();
    Some(SprachGruppe::neu(zeile.nr, teile))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anwenden(alt: &str, neu: &str) -> Vec<([usize; 2], String)> {
        let hunks = wort_hunks(alt, neu);
        let mut ende = 0;
        for (bereich, _) in &hunks {
            assert!(bereich[0] >= ende);
            assert!(bereich[0] <= bereich[1]);
            assert!(alt.is_char_boundary(bereich[0]));
            assert!(alt.is_char_boundary(bereich[1]));
            ende = bereich[1];
        }
        let mut ergebnis = alt.to_owned();
        for ([von, bis], ersatz) in hunks.iter().rev() {
            ergebnis.replace_range(*von..*bis, ersatz);
        }
        assert_eq!(ergebnis, neu);
        hunks
    }

    #[test]
    fn ganze_woerter_und_eingefuegte_kommas() {
        assert_eq!(
            anwenden("Ein Tippfeler.", "Ein Tippfehler."),
            vec![([4, 13], "Tippfehler".into())]
        );
        // Ein Komma wird als reine Einfügung vor dem Leerraum dargestellt.
        assert_eq!(
            anwenden("Hallo Welt", "Hallo, Welt"),
            vec![([5, 5], ",".into())]
        );
        assert_eq!(
            anwenden("hallo Welt", "Hallo Welt"),
            vec![([0, 5], "Hallo".into())]
        );
        assert_eq!(anwenden("abc", "XYZ"), vec![([0, 3], "XYZ".into())]);
        assert!(anwenden("gleich", "gleich").is_empty());
    }

    #[test]
    fn entfernte_fehler_bleiben_getrennt_und_nachbarwoerter_verschmelzen() {
        assert_eq!(
            anwenden("Feler bleibt hier feler", "Fehler bleibt hier Fehler").len(),
            2
        );
        assert_eq!(anwenden("falsch wort", "richtig Wort").len(), 1);
        anwenden("altes Wort", "Wort altes");
        anwenden("a b c", "a c");
        anwenden("a c", "a b c");
    }

    #[test]
    fn apostrophe_und_unicode_haben_definierte_tokengrenzen() {
        let tokens = tokenisiere("can't O’Neil 'x' ä🙂e\u{301} ß");
        assert_eq!(
            tokens.iter().map(|t| t.text).collect::<Vec<_>>(),
            [
                "can't", " ", "O’Neil", " ", "'", "x", "'", " ", "ä", "🙂", "e", "\u{301}", " ",
                "ß"
            ]
        );
        assert_eq!(anwenden("can't", "cant"), vec![([0, 5], "cant".into())]);
        anwenden("Grüße🙂 e\u{301}", "Größe😀 é");
    }

    #[test]
    fn deterministische_paare_erfuellen_die_anwendungsinvariante() {
        let bausteine = [
            "", "a", "b", "ä", "ß", "🙂", "e\u{301}", " ", "\t", ",", "can't", "O’Neil",
        ];
        let mut zustand = 17u64;
        for _ in 0..400 {
            let mut texte = [String::new(), String::new()];
            for text in &mut texte {
                for _ in 0..12 {
                    zustand = zustand.wrapping_mul(6364136223846793005).wrapping_add(1);
                    text.push_str(bausteine[(zustand >> 32) as usize % bausteine.len()]);
                }
            }
            anwenden(&texte[0], &texte[1]);
        }
        for alt in bausteine {
            for neu in bausteine {
                anwenden(alt, neu);
            }
        }
    }

    #[test]
    fn distanzgrenze_faellt_auf_die_mitte_zurueck() {
        let grenze_alt = "!".repeat(MAX_DISTANZ / 2);
        let grenze_neu = "?".repeat(MAX_DISTANZ / 2);
        assert!(myers(&tokenisiere(&grenze_alt), &tokenisiere(&grenze_neu)).is_some());
        let alt = format!("Rand {} Ende", "a ".repeat(1001));
        let neu = format!("Rand {} Ende", "b ".repeat(1001));
        let a = tokenisiere(&alt);
        let b = tokenisiere(&neu);
        assert!(myers(&a, &b).is_none());
        let hunks = anwenden(&alt, &neu);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].0, [5, alt.len() - "  Ende".len()]);
        let kurz = tokenisiere("a");
        let lang_text = "b ".repeat(2002);
        assert!(myers(&kurz, &tokenisiere(&lang_text)).is_none());
        anwenden("a", &lang_text);
    }

    #[test]
    fn gruppe_verwendet_absolute_bereiche_und_besteht_die_gesamtpruefung() {
        let basis = "Vorlauf🙂\nEin Tippfeler bleibt hier feler.\nEnde";
        let alt = "Ein Tippfeler bleibt hier feler.";
        let neu = "Ein Tippfehler bleibt hier Fehler.";
        let start = "Vorlauf🙂\n".len();
        let zeile = Zeile {
            nr: 2,
            bereich: [start, start + alt.len()],
        };
        assert!(gruppe_fuer_zeile(&zeile, alt, alt).is_none());
        let gruppe = gruppe_fuer_zeile(&zeile, alt, neu).unwrap();
        assert_eq!(gruppe.zeile, 2);
        assert_eq!(gruppe.aenderungen.len(), 2);
        assert!(crate::aufbereitung::pruefe_aenderungen(basis, &gruppe.aenderungen).is_ok());
        let mut erhalten = basis.to_owned();
        for aenderung in gruppe.aenderungen.iter().rev() {
            let [von, bis] = aenderung.bereich;
            assert_eq!(&basis[von..bis], aenderung.alt);
            erhalten.replace_range(von..bis, &aenderung.neu);
        }
        assert_eq!(erhalten, format!("Vorlauf🙂\n{neu}\nEnde"));
    }
}
