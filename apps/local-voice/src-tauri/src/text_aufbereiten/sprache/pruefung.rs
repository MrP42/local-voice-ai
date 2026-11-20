//! Konservative Inhaltsprüfung entmaskierter Modellvorschläge.
//! Auch Korrekturen direkter Nachbarwörter einer Negation oder eines Modalworts
//! werden bewusst konservativ verworfen.
//! Auch eingefügte oder entfernte Satz-/Teilsatztrenner vor einer Negation oder
//! einem Modalwort führen bewusst konservativ zum Verwerfen.

use std::sync::LazyLock;

use regex::Regex;

use super::fehler::GrundCode;

pub const LAENGE_ABS_MIN: usize = 8;
pub const LAENGE_REL_PROZENT: usize = 30;

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
    adressen: Vec<String>,
    zahlen: Vec<String>,
    zitate: Vec<String>,
    negationen: Vec<(usize, Option<String>, String, Option<String>)>,
    anfuehrung_offen: bool,
}

fn ist_zahlwort(wort: &str) -> bool {
    if ZAHLWOERTER.contains(&wort) {
        return true;
    }
    if wort.chars().count() > 64 {
        return false;
    }
    const MORPHEME: &[&str] = &[
        "ein", "eins", "eine", "einen", "einem", "einer", "zwei", "drei", "vier", "fuenf", "fünf",
        "sechs", "sech", "sieben", "sieb", "acht", "neun", "zehn", "elf", "zwoelf", "zwölf",
        "zwanzig", "dreissig", "dreißig", "vierzig", "fuenfzig", "fünfzig", "sechzig", "siebzig",
        "achtzig", "neunzig", "hundert", "tausend", "und", "zig",
    ];
    // Erreichbare Bytepositionen mit und ohne bereits gefundenes Zahlmorphem.
    let mut erreichbar = vec![[false; 2]; wort.len() + 1];
    erreichbar[0][0] = true;
    for (position, _) in wort.char_indices() {
        for (mit_zahl, gefunden) in erreichbar[position].into_iter().enumerate() {
            if !gefunden {
                continue;
            }
            for morphem in MORPHEME {
                if wort[position..].starts_with(morphem) {
                    let zahl = usize::from(mit_zahl == 1 || *morphem != "und");
                    erreichbar[position + morphem.len()][zahl] = true;
                }
            }
        }
    }
    erreichbar[wort.len()][1]
}

fn wortzeichen(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\'' || c == '’'
}

fn sammle_tokens(
    text: &str,
    von: usize,
    bis: usize,
    ergebnis: &mut Merkmale,
    woerter: &mut Vec<(usize, String)>,
) {
    for token in TOKEN.find_iter(&text[von..bis]) {
        let start = von + token.start();
        let ende = von + token.end();
        let wert = token.as_str();
        if wert.chars().next().is_some_and(char::is_numeric) {
            let mut zahl_start = start;
            if let Some((position, zeichen)) = text[..start].char_indices().next_back() {
                if matches!(zeichen, '-' | '\u{2212}' | '+')
                    && (position == 0
                        || text[..position].chars().next_back().is_some_and(|c| {
                            !c.is_alphanumeric() && !matches!(c, '-' | '\u{2212}' | '+')
                        }))
                {
                    zahl_start = position;
                }
            }
            ergebnis.zahlen.push(text[zahl_start..ende].to_owned());
        } else {
            let links = text[..start].chars().next_back();
            let rechts = text[ende..].chars().next();
            if links.is_some_and(wortzeichen) || rechts.is_some_and(wortzeichen) {
                continue;
            }
            let wort = wert.to_lowercase();
            if ist_zahlwort(&wort) {
                ergebnis.zahlen.push(wort.clone());
            }
            woerter.push((start, wort));
        }
    }
}

fn merkmale(text: &str) -> Merkmale {
    let mut ergebnis = Merkmale::default();
    let mut woerter = Vec::new();
    let mut von = 0;
    // Tokenisierung in Dokumentreihenfolge: jede Lücke wird genau einmal zerlegt.
    // Getrennte Suchmuster vermeiden wiederholte erfolglose Adresssuchen je Wort.
    for adresse in ADRESSE.find_iter(text) {
        sammle_tokens(text, von, adresse.start(), &mut ergebnis, &mut woerter);
        ergebnis.adressen.push(
            adresse
                .as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?', ')'])
                .to_owned(),
        );
        von = adresse.end();
    }
    sammle_tokens(text, von, text.len(), &mut ergebnis, &mut woerter);
    let mut trenner = text
        .char_indices()
        .filter(|(_, c)| matches!(c, ',' | ';' | ':' | '.' | '!' | '?' | '–' | '—'))
        .map(|(position, _)| position)
        .peekable();
    let mut trenner_vorher = 0;
    for (position, (start, wort)) in woerter.iter().enumerate() {
        while trenner.peek().is_some_and(|index| *index < *start) {
            trenner.next();
            trenner_vorher += 1;
        }
        if NEGATIONEN.contains(&wort.as_str()) {
            ergebnis.negationen.push((
                trenner_vorher,
                position.checked_sub(1).map(|i| woerter[i].1.clone()),
                wort.clone(),
                woerter.get(position + 1).map(|(_, wort)| wort.clone()),
            ));
        }
    }

    // Ein schließendes Zeichen öffnet nicht zugleich ein umgekehrtes Klammerpaar.
    let mut offen: Option<(usize, usize, char)> = None;
    for (zeichen_nr, (position, c)) in text.char_indices().enumerate() {
        if let Some((start, nummer, zu)) = offen {
            if c == zu {
                if zeichen_nr > nummer {
                    ergebnis.zitate.push(text[start..position].to_owned());
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
    ergebnis.anfuehrung_offen = offen.is_some() || text.matches('"').count() % 2 != 0;
    ergebnis
}

/// Der erste Treffer gewinnt; Texte werden weder ausgegeben noch protokolliert.
pub fn pruefe_zeile(alt: &str, neu: &str) -> Result<(), GrundCode> {
    let alt_merkmale = merkmale(alt);
    let neu_merkmale = merkmale(neu);
    if alt != neu && (alt_merkmale.anfuehrung_offen || neu_merkmale.anfuehrung_offen) {
        return Err(GrundCode::AnfuehrungGeaendert);
    }
    let alt_laenge = alt.chars().count();
    let neu_laenge = neu.chars().count();
    let toleranz = LAENGE_ABS_MIN.max(alt_laenge.saturating_mul(LAENGE_REL_PROZENT) / 100);
    if alt_laenge.abs_diff(neu_laenge) > toleranz {
        return Err(GrundCode::LaengeAbweichung);
    }
    let alt = alt_merkmale;
    let neu = neu_merkmale;
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
    fn zahlen_und_zahlwoerter_bleiben_in_reihenfolge_erhalten() {
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
            ("zwei 3/4", "3/4 zwei"),
            (
                "Anna erhaelt 10 Euro, Ben 20 Euro.",
                "Anna erhaelt 20 Euro, Ben 10 Euro.",
            ),
            ("zwei drei", "drei zwei"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::ZahlGeaendert));
        }
        for (alt, neu) in [
            ("Eins", "eins"),
            ("Eine", "eine"),
            ("verein", "Verein"),
            ("einsatz", "Einsatz"),
            ("eins2", "zwei2"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Ok(()));
        }
    }

    #[test]
    fn vorzeichen_und_zahlwortkomposita_bleiben_erhalten() {
        for (alt, neu) in [
            (
                "Die Temperatur betraegt -5 Grad.",
                "Die Temperatur betraegt 5 Grad.",
            ),
            ("−5", "5"),
            ("+5", "5"),
            ("(-5)", "(5)"),
            ("Wert -5", "Wert +5"),
            ("einundzwanzig", "zweiundzwanzig"),
            ("einhundertfünf", "einhundertsechs"),
            ("zweitausendzwölf", "zweitausendelf"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::ZahlGeaendert));
        }
        for wort in [
            "einundzwanzig",
            "einenundeinem",
            "einerundeine",
            "einsundelf",
            "sechzehn",
            "siebzehn",
            "fuenfunddreissig",
            "fünfunddreißig",
            "siebenundvierzig",
            "achtundfuenfzig",
            "neunundfünfzig",
            "sechsundsechzig",
            "zweiundsiebzig",
            "dreiundachtzig",
            "vierundneunzig",
            "tausendhundertzig",
        ] {
            assert!(ist_zahlwort(wort));
        }
        for wort in ["", "und", "undund", "verein", "einsatz", "einundzwanzigmal"] {
            assert!(!ist_zahlwort(wort));
        }
        assert!(!ist_zahlwort(&"ein".repeat(22)));
        assert_eq!(pruefe_zeile("Wort-5", "Wort+5"), Ok(()));
        assert_eq!(pruefe_zeile("a-b 5", "a b 5"), Ok(()));
        assert_eq!(pruefe_zeile("Einundzwanzig", "einundzwanzig"), Ok(()));
    }

    #[test]
    fn vorzeichen_nach_satzzeichen_bleiben_erhalten() {
        for (alt, neu) in [
            ("Wert:-5 Grad.", "Wert:5 Grad."),
            ("Wert:-5 Grad.", "Wert:+5 Grad."),
            ("Temperatur:−5 °C", "Temperatur:5 °C"),
            ("Wert:+5", "Wert:5"),
            ("Wert;-5", "Wert;5"),
            ("[-5]", "[5]"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::ZahlGeaendert));
        }
        assert_eq!(pruefe_zeile("A-5", "A+5"), Ok(()));
        assert_eq!(pruefe_zeile("A-5", "A5"), Ok(()));
        assert_eq!(pruefe_zeile("--5", "-+5"), Ok(()));
        assert_eq!(pruefe_zeile("12-5", "12-6"), Err(GrundCode::ZahlGeaendert));
        assert_eq!(merkmale("12-5").zahlen, ["12-5"]);
    }

    #[test]
    fn adressen_ignorieren_nur_abschliessende_satzzeichen() {
        for (alt, neu) in [
            ("https://a.de/1", "https://b.de/1"),
            ("www.a.de", "www.b.de"),
            ("x@a.de", "y@a.de"),
            ("x@a.de x@a.de", "x@a.de"),
            ("https://a.de https://b.de", "https://b.de https://a.de"),
            ("x@a.de y@a.de", "y@a.de x@a.de"),
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
        assert_eq!(
            pruefe_zeile("\"a\"", "\"b\""),
            Err(GrundCode::AnfuehrungGeaendert)
        );
        assert_eq!(
            pruefe_zeile("Sie sagte „a“.", "Sie sagte „b“."),
            Err(GrundCode::AnfuehrungGeaendert)
        );
        assert_eq!(
            pruefe_zeile("„ab“ und „cd“", "„cd“ und „ab“"),
            Err(GrundCode::AnfuehrungGeaendert)
        );
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
    fn offene_anfuehrungen_sperren_jede_aenderung() {
        for (auf, zu) in [
            ('„', '“'),
            ('"', '"'),
            ('“', '”'),
            ('»', '«'),
            ('«', '»'),
            ('‘', '’'),
        ] {
            let alt = format!("Er sagte: {auf}Ein Feler.");
            assert_eq!(pruefe_zeile(&alt, &alt), Ok(()));
            for neu in [
                alt.replace("Feler", "Fehler"),
                alt.replace("Er", "Sie"),
                format!("{alt}{zu}"),
                "x".to_owned(),
            ] {
                assert_eq!(
                    pruefe_zeile(&alt, &neu),
                    Err(GrundCode::AnfuehrungGeaendert)
                );
            }
            let geschlossen = format!("{alt}{zu}");
            assert_eq!(
                pruefe_zeile(&geschlossen, &alt),
                Err(GrundCode::AnfuehrungGeaendert)
            );
        }
        assert_eq!(
            pruefe_zeile("„a“", "„a\"“"),
            Err(GrundCode::AnfuehrungGeaendert)
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
            ("Er kommt nicht, sie kommt.", "Er kommt, sie kommt nicht."),
            ("Er kann gehen, sie geht.", "Er geht, sie kann gehen."),
            ("Er kommt nicht.", "Er geht nicht."),
            ("Er kann gehn.", "Er kann gehen."),
            ("nicht er", "er nicht"),
            ("nicht er, nie sie", "nie sie, nicht er"),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::NegationGeaendert));
        }
        assert_eq!(
            pruefe_zeile("Ich sehe das er geht.", "Ich sehe, dass er geht."),
            Ok(())
        );
        assert_eq!(pruefe_zeile("Nicht mehr!", "nicht mehr."), Ok(()));
        assert_eq!(
            merkmale("nicht mehr").negationen,
            vec![(0, None, "nicht".to_owned(), Some("mehr".to_owned()))]
        );
        assert_eq!(
            merkmale("Er, NICHT; sie! kann.").negationen,
            vec![
                (
                    1,
                    Some("er".to_owned()),
                    "nicht".to_owned(),
                    Some("sie".to_owned())
                ),
                (3, Some("sie".to_owned()), "kann".to_owned(), None),
            ]
        );
        assert_eq!(pruefe_zeile("Er kann, gehen.", "er KANN gehen!"), Ok(()));
        assert_eq!(pruefe_zeile("niemandsland", "Niemandsland"), Ok(()));
    }

    #[test]
    fn negationsbezug_bleibt_im_selben_teilsatz() {
        for (alt, neu) in [
            (
                "Er kommt heute nicht mehr, sie kommt heute wieder.",
                "Er kommt heute wieder, sie kommt heute nicht mehr.",
            ),
            (
                "Anna kommt heute nicht nach Hause; Ben kommt heute nach Hause.",
                "Anna kommt heute nach Hause; Ben kommt heute nicht nach Hause.",
            ),
            ("Heute kommt er nicht mehr.", "Heute, kommt er nicht mehr."),
        ] {
            assert_eq!(pruefe_zeile(alt, neu), Err(GrundCode::NegationGeaendert));
        }
        for trenner in [',', ';', ':', '.', '!', '?', '–', '—'] {
            let alt = format!("Heute{trenner} kommt er nicht mehr.");
            assert_eq!(
                pruefe_zeile(&alt, "Heute kommt er nicht mehr."),
                Err(GrundCode::NegationGeaendert)
            );
        }
        assert_eq!(
            pruefe_zeile(
                "Ein Feler, er kommt nicht mehr.",
                "Ein Fehler, er kommt nicht mehr."
            ),
            Ok(())
        );
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
