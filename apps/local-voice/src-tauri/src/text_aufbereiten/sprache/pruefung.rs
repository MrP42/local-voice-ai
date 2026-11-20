//! Konservative Inhaltsprüfung entmaskierter Modellvorschläge.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::fehler::GrundCode;

pub const LAENGE_ABS_MIN: usize = 8;
pub const LAENGE_REL_PROZENT: usize = 30;

type Multimenge = BTreeMap<String, usize>;

// Adressen werden als ganze Spannen erkannt und nicht erneut als Wörter durchsucht.
static ADRESSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"https?://[^\s]+|www\.[^\s]+|",
        r"[\p{L}\p{N}._%+\-]+@[\p{L}\p{N}\-]+",
        r"(?:\.[\p{L}\p{N}\-]+)+"
    ))
    .expect("Festes Adressmuster der Sprachprüfung")
});

static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\p{Nd}+(?:[.,:/\-]\p{Nd}+)*|",
        r"[\p{L}\p{M}]+(?:['’][\p{L}\p{M}]+)*"
    ))
    .expect("Festes Muster der Sprachprüfung")
});

const ZAHLWOERTER: &[&str] = &[
    "null",
    "ein",
    "eins",
    "eine",
    "einen",
    "einem",
    "einer",
    "zwei",
    "drei",
    "vier",
    "fünf",
    "fuenf",
    "sechs",
    "sieben",
    "acht",
    "neun",
    "zehn",
    "elf",
    "zwölf",
    "zwoelf",
    "dreizehn",
    "vierzehn",
    "fünfzehn",
    "fuenfzehn",
    "sechzehn",
    "siebzehn",
    "achtzehn",
    "neunzehn",
    "zwanzig",
    "dreißig",
    "dreissig",
    "vierzig",
    "fünfzig",
    "fuenfzig",
    "sechzig",
    "siebzig",
    "achtzig",
    "neunzig",
    "hundert",
    "tausend",
    "million",
    "millionen",
    "milliarde",
    "milliarden",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "hundred",
    "thousand",
    "million",
];

const NEGATIONEN: &[&str] = &[
    "nicht",
    "nichts",
    "kein",
    "keine",
    "keinen",
    "keinem",
    "keiner",
    "keines",
    "nie",
    "niemals",
    "niemand",
    "niemandem",
    "niemanden",
    "ohne",
    "weder",
    "noch",
    "nein",
    "muss",
    "musst",
    "müssen",
    "muessen",
    "mussten",
    "darf",
    "dürfen",
    "duerfen",
    "kann",
    "kannst",
    "können",
    "koennen",
    "soll",
    "sollst",
    "sollen",
    "will",
    "willst",
    "wollen",
    "mag",
    "möchte",
    "moechte",
    "möchten",
    "moechten",
    "not",
    "no",
    "never",
    "none",
    "without",
    "nor",
    "cannot",
    "can't",
    "don't",
    "doesn't",
    "won't",
    "isn't",
    "aren't",
    "must",
    "shall",
    "should",
    "can",
    "may",
];

#[derive(Default)]
struct Merkmale {
    adressen: Multimenge,
    zahlen: Multimenge,
    zitate: Multimenge,
    negationen: Multimenge,
}

fn zaehle(menge: &mut Multimenge, text: &str) {
    *menge.entry(text.to_owned()).or_default() += 1;
}

fn wortzeichen(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\'' || c == '’'
}

fn sammle_tokens(
    text: &str,
    von: usize,
    bis: usize,
    ergebnis: &mut Merkmale,
    letztes_nicht: &mut Option<usize>,
) {
    for token in TOKEN.find_iter(&text[von..bis]) {
        let start = von + token.start();
        let ende = von + token.end();
        let wert = token.as_str();
        if wert.chars().next().is_some_and(char::is_numeric) {
            zaehle(&mut ergebnis.zahlen, wert);
            *letztes_nicht = None;
        } else {
            let links = text[..start].chars().next_back();
            let rechts = text[ende..].chars().next();
            if links.is_some_and(wortzeichen) || rechts.is_some_and(wortzeichen) {
                *letztes_nicht = None;
                continue;
            }
            let wort = wert.to_lowercase();
            if ZAHLWOERTER.contains(&wort.as_str()) {
                zaehle(&mut ergebnis.zahlen, &wort);
            }
            if NEGATIONEN.contains(&wort.as_str()) {
                zaehle(&mut ergebnis.negationen, &wort);
            }
            if wort == "mehr"
                && letztes_nicht.is_some_and(|ende| {
                    let zwischen: &str = &text[ende..start];
                    !zwischen.is_empty() && zwischen.chars().all(char::is_whitespace)
                })
            {
                zaehle(&mut ergebnis.negationen, "nicht mehr");
            }
            *letztes_nicht = (wort == "nicht").then_some(ende);
        }
    }
}

fn merkmale(text: &str) -> Merkmale {
    let mut ergebnis = Merkmale::default();
    let mut letztes_nicht = None;
    let mut von = 0;
    // Tokenisierung in Dokumentreihenfolge: jede Lücke wird genau einmal zerlegt.
    // Getrennte Suchmuster vermeiden wiederholte erfolglose Adresssuchen je Wort.
    for adresse in ADRESSE.find_iter(text) {
        sammle_tokens(
            text,
            von,
            adresse.start(),
            &mut ergebnis,
            &mut letztes_nicht,
        );
        zaehle(
            &mut ergebnis.adressen,
            adresse
                .as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?', ')']),
        );
        letztes_nicht = None;
        von = adresse.end();
    }
    sammle_tokens(text, von, text.len(), &mut ergebnis, &mut letztes_nicht);

    // Ein schließendes Zeichen öffnet nicht zugleich ein umgekehrtes Klammerpaar.
    let mut offen: Option<(usize, usize, char)> = None;
    for (zeichen_nr, (position, c)) in text.char_indices().enumerate() {
        if let Some((start, nummer, zu)) = offen {
            if c == zu {
                if zeichen_nr - nummer >= 2 {
                    zaehle(&mut ergebnis.zitate, &text[start..position]);
                }
                offen = None;
            }
        } else {
            let zu = match c {
                '„' => Some('“'),
                '"' => Some('"'),
                '“' => Some('”'),
                '»' => Some('«'),
                '«' => Some('»'),
                '‘' => Some('’'),
                _ => None,
            };
            if let Some(zu) = zu {
                offen = Some((position + c.len_utf8(), zeichen_nr + 1, zu));
            }
        }
    }
    ergebnis
}

/// Der erste Treffer gewinnt; Texte werden weder ausgegeben noch protokolliert.
pub fn pruefe_zeile(alt: &str, neu: &str) -> Result<(), GrundCode> {
    let alt_laenge = alt.chars().count();
    let neu_laenge = neu.chars().count();
    let toleranz = LAENGE_ABS_MIN.max(alt_laenge.saturating_mul(LAENGE_REL_PROZENT) / 100);
    if alt_laenge.abs_diff(neu_laenge) > toleranz {
        return Err(GrundCode::LaengeAbweichung);
    }
    let alt = merkmale(alt);
    let neu = merkmale(neu);
    for (verschieden, grund) in [
        (alt.adressen != neu.adressen, GrundCode::UrlGeaendert),
        (alt.zahlen != neu.zahlen, GrundCode::ZahlGeaendert),
        (alt.zitate != neu.zitate, GrundCode::AnfuehrungGeaendert),
        (
            alt.negationen != neu.negationen,
            GrundCode::NegationGeaendert,
        ),
    ] {
        if verschieden {
            return Err(grund);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laengengrenzen_sind_inklusiv_und_zaehlen_zeichen() {
        assert_eq!(pruefe_zeile("abc", "abcdefghijk"), Ok(()));
        assert_eq!(
            pruefe_zeile("abc", "abcdefghijkl"),
            Err(GrundCode::LaengeAbweichung)
        );
        let alt = "ä".repeat(100);
        assert_eq!(pruefe_zeile(&alt, &"ä".repeat(130)), Ok(()));
        assert_eq!(pruefe_zeile(&alt, &"ä".repeat(70)), Ok(()));
        for laenge in [69, 131] {
            assert_eq!(
                pruefe_zeile(&alt, &"ä".repeat(laenge)),
                Err(GrundCode::LaengeAbweichung)
            );
        }
    }

    #[test]
    fn zahlen_und_zahlwoerter_bleiben_als_multimenge_erhalten() {
        for (alt, neu) in [
            ("1.234,56", "1.234,57"),
            ("12.03.2024", "12.03.2025"),
            ("12:30", "12:31"),
            ("3/4", "3/5"),
            ("zwei", "drei"),
            ("fünf", "fuenf"),
            ("dreißig", "dreissig"),
            ("zwei zwei", "zwei"),
            ("abc12", "abc13"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::ZahlGeaendert));
        }
        for (alt, neu) in [
            ("Eins", "eins"),
            ("zwei 3/4", "3/4 zwei"),
            ("verein", "Verein"),
            ("einsatz", "Einsatz"),
            ("eins2", "zwei2"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Ok(()));
        }
    }

    #[test]
    fn adressen_ignorieren_nur_abschliessende_satzzeichen() {
        for (alt, neu) in [
            ("https://a.de/1", "https://b.de/1"),
            ("www.a.de", "www.b.de"),
            ("x@a.de", "y@a.de"),
            ("x@a.de x@a.de", "x@a.de"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::UrlGeaendert));
        }
        assert_eq!(pruefe_zeile("https://a.de.", "https://a.de!"), Ok(()));
        assert_eq!(pruefe_zeile("x@a.de.", "x@a.de!"), Ok(()));
    }

    #[test]
    fn zitate_schuetzen_inhalte_aber_nicht_den_umgebenden_text() {
        let paare = [
            ('„', '“'),
            ('"', '"'),
            ('“', '”'),
            ('»', '«'),
            ('«', '»'),
            ('‘', '’'),
        ];
        for (auf, zu) in paare {
            let alt = format!("Tippfeler {auf}Müller{zu}");
            let neu = format!("Tippfeler {auf}Mueller{zu}");
            assert_eq!(
                pruefe_zeile(&alt, &neu),
                Err(GrundCode::AnfuehrungGeaendert)
            );
            assert_eq!(
                pruefe_zeile(&alt, &alt.replace("Tippfeler", "Tippfehler")),
                Ok(())
            );
        }
        assert_eq!(
            pruefe_zeile("\"ab\" und \"cd\"", "\"ab\" und \"ce\""),
            Err(GrundCode::AnfuehrungGeaendert)
        );
        assert_eq!(pruefe_zeile("\"a\"", "\"b\""), Ok(()));
        assert_eq!(
            pruefe_zeile("»ab« Tippfeler »cd«", "»ab« Tippfehler »cd«"),
            Ok(())
        );
        assert_eq!(
            pruefe_zeile("„ab“ Tippfeler „cd“", "„ab“ Tippfehler „cd“"),
            Ok(())
        );
    }

    #[test]
    fn negationen_modalwoerter_und_nicht_mehr_sind_konservativ() {
        for (alt, neu) in [
            ("Das geht nicht.", "Das geht."),
            ("Das ist Brot.", "Das ist kein Brot."),
            ("Er kann gehen.", "Er muss gehen."),
            ("Wir muessen gehen.", "Wir müssen gehen."),
            ("nicht mehr", "nicht sehr"),
            ("not not", "not"),
            ("I can't go.", "I can go."),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::NegationGeaendert));
        }
        assert_eq!(
            pruefe_zeile("Ich sehe das er geht.", "Ich sehe, dass er geht."),
            Ok(())
        );
        assert_eq!(pruefe_zeile("Nicht mehr!", "nicht mehr."), Ok(()));
        assert_eq!(merkmale("nicht mehr").negationen.get("nicht"), Some(&1));
        assert_eq!(
            merkmale("nicht mehr").negationen.get("nicht mehr"),
            Some(&1)
        );
        assert_eq!(pruefe_zeile("niemandsland", "Niemandsland"), Ok(()));
    }

    #[test]
    fn erster_fehler_gewinnt_bei_mehreren_verletzungen() {
        assert_eq!(
            pruefe_zeile("1", "https://b.de 2 nicht"),
            Err(GrundCode::LaengeAbweichung)
        );
        assert_eq!(
            pruefe_zeile("https://a.de 1", "https://b.de 2"),
            Err(GrundCode::UrlGeaendert)
        );
        assert_eq!(
            pruefe_zeile("„ab“ 1", "„cd“ 2"),
            Err(GrundCode::ZahlGeaendert)
        );
        assert_eq!(
            pruefe_zeile("„ab“ kann", "„cd“ muss"),
            Err(GrundCode::AnfuehrungGeaendert)
        );
    }

    #[test]
    fn identitaet_und_unicode_tippfehler_sind_erlaubt() {
        for text in [
            "",
            "ä🙂ß",
            "e\u{301}",
            "1.234,56",
            "12.03.2024",
            "12:30",
            "3/4",
            "Eins zwei",
            "https://a.de/1",
            "x@y.de",
            "„Müller“",
            "\"ab\" \"cd\"",
            "nicht mehr",
            "can't",
        ] {
            assert_eq!(pruefe_zeile(text, text), Ok(()));
        }
        assert_eq!(
            pruefe_zeile("Grüße🙂 Tippfeler e\u{301}", "Grüße🙂 Tippfehler é"),
            Ok(())
        );
    }
}
