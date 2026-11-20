//! Schutzbereiche und inhaltsfreie Platzhalter der KI-Sprachstufe.

use super::fehler::GrundCode;
use super::vertrag::Zeile;
use std::fmt;

crate::text_aufbereiten::re!(URL, r#"https?://[^\s]+|\bwww\.[^\s]+"#);
crate::text_aufbereiten::re!(EMAIL, r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}");

#[derive(Clone)]
pub struct Maskiert {
    pub text: String,
    masken: Vec<String>,
    begrenzer: (&'static str, &'static str),
}

impl fmt::Debug for Maskiert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Maskiert")
            .field("laenge", &self.text.len())
            .field("anzahl", &self.masken.len())
            .finish()
    }
}

pub fn waehle_begrenzer(abschnitt_text: &str) -> Option<(&'static str, &'static str)> {
    [("⟦", "⟧"), ("⟪", "⟫"), ("⦃", "⦄")]
        .into_iter()
        .find(|(auf, zu)| !abschnitt_text.contains(auf) && !abschnitt_text.contains(zu))
}

pub fn schutzbereiche_der_zeile(
    basis: &str,
    zeile: &Zeile,
    global: &[(usize, usize)],
) -> Vec<[usize; 2]> {
    let [start, ende] = zeile.bereich;
    let Some(text) = basis.get(start..ende) else {
        return Vec::new();
    };
    let mut bereiche = Vec::new();
    for &(von, bis) in global {
        let von = von.max(start);
        let bis = bis.min(ende);
        if von < bis && basis.is_char_boundary(von) && basis.is_char_boundary(bis) {
            bereiche.push([von - start, bis - start]);
        }
    }

    let mut ticks = text.match_indices('`');
    while let Some((von, _)) = ticks.next() {
        if let Some((bis, _)) = ticks.next() {
            bereiche.push([von, bis + 1]);
        }
    }
    for fund in URL.find_iter(text) {
        let url = fund.as_str().trim_end_matches([
            '.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'', '“', '”', '«', '»', '’',
        ]);
        if !url.is_empty() {
            bereiche.push([fund.start(), fund.start() + url.len()]);
        }
    }
    for fund in EMAIL.find_iter(text) {
        bereiche.push([fund.start(), fund.end()]);
    }

    bereiche.sort_unstable();
    let mut vereinigt: Vec<[usize; 2]> = Vec::new();
    for bereich in bereiche {
        if let Some(letzter) = vereinigt.last_mut() {
            if bereich[0] <= letzter[1] {
                letzter[1] = letzter[1].max(bereich[1]);
                continue;
            }
        }
        vereinigt.push(bereich);
    }
    vereinigt
}

pub fn maskiere_zeile(
    zeile_text: &str,
    schutz: &[[usize; 2]],
    begrenzer: (&'static str, &'static str),
) -> Maskiert {
    let mut text = String::new();
    let mut masken = Vec::new();
    let mut position = 0;
    for &[von, bis] in schutz {
        if von < position || von >= bis || !zeile_text.is_char_boundary(von) {
            continue;
        }
        let Some(original) = zeile_text.get(von..bis) else {
            continue;
        };
        text.push_str(&zeile_text[position..von]);
        text.push_str(&format!("{}{}{}", begrenzer.0, masken.len(), begrenzer.1));
        masken.push(original.to_owned());
        position = bis;
    }
    text.push_str(&zeile_text[position..]);
    Maskiert {
        text,
        masken,
        begrenzer,
    }
}

impl Maskiert {
    pub fn pruefe(&self, antwort: &str) -> Result<(), GrundCode> {
        let (auf, zu) = self.begrenzer;
        let mut rest = antwort;
        for nr in 0..self.masken.len() {
            let Some(position) = rest.find(auf) else {
                return Err(GrundCode::MaskeBeschaedigt);
            };
            if rest[..position].contains(zu) {
                return Err(GrundCode::MaskeBeschaedigt);
            }
            let platzhalter = format!("{auf}{nr}{zu}");
            rest = &rest[position..];
            if !rest.starts_with(&platzhalter) {
                return Err(GrundCode::MaskeBeschaedigt);
            }
            rest = &rest[platzhalter.len()..];
        }
        if rest.contains(auf) || rest.contains(zu) {
            return Err(GrundCode::MaskeBeschaedigt);
        }
        Ok(())
    }

    pub fn entmaskieren(&self, antwort: &str) -> String {
        let (auf, zu) = self.begrenzer;
        let mut ergebnis = String::new();
        let mut rest = antwort;
        while let Some(position) = rest.find(auf) {
            ergebnis.push_str(&rest[..position]);
            rest = &rest[position + auf.len()..];
            // Nur Ziffern lesen, damit auch beschädigte Antworten linear verarbeitet werden.
            let ziffern = rest.bytes().take_while(u8::is_ascii_digit).count();
            let nummer = &rest[..ziffern];
            let original = nummer.parse::<usize>().ok().and_then(|nr| {
                if nummer == nr.to_string() && rest[ziffern..].starts_with(zu) {
                    self.masken.get(nr)
                } else {
                    None
                }
            });
            if let Some(original) = original {
                ergebnis.push_str(original);
                rest = &rest[ziffern + zu.len()..];
            } else {
                ergebnis.push_str(auf);
                ergebnis.push_str(nummer);
                rest = &rest[ziffern..];
            }
        }
        ergebnis.push_str(rest);
        ergebnis
    }

    pub fn anzahl(&self) -> usize {
        self.masken.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zeile(text: &str) -> Zeile {
        Zeile {
            nr: 1,
            bereich: [0, text.len()],
        }
    }

    #[test]
    fn schutz_und_rundreise_sind_bytegenau() {
        let text = "Ä🙂ß e\u{301} {{S.12}} <Anna> [pause] 12:30 `x()` https://a.de! a@b.de";
        let marken = ["{{S.12}}", "<Anna>", "[pause]", "12:30"];
        let global = marken
            .iter()
            .map(|marke| {
                let start = text.find(marke).unwrap();
                (start, start + marke.len())
            })
            .collect::<Vec<_>>();
        let schutz = schutzbereiche_der_zeile(text, &zeile(text), &global);
        let inhalte = schutz
            .iter()
            .map(|[a, b]| &text[*a..*b])
            .collect::<Vec<_>>();
        assert_eq!(
            inhalte,
            [
                "{{S.12}}",
                "<Anna>",
                "[pause]",
                "12:30",
                "`x()`",
                "https://a.de",
                "a@b.de"
            ]
        );
        let maske = maskiere_zeile(text, &schutz, waehle_begrenzer(text).unwrap());
        assert_eq!(maske.anzahl(), 7);
        assert_eq!(maske.pruefe(&maske.text), Ok(()));
        assert_eq!(maske.entmaskieren(&maske.text), text);
        assert!(!format!("{maske:?}").contains("Anna"));
        assert!(!format!("{maske:?}").contains("a@b.de"));
    }

    #[test]
    fn begrenzer_weichen_beiden_zeichen_aus() {
        assert_eq!(waehle_begrenzer("frei"), Some(("⟦", "⟧")));
        assert_eq!(waehle_begrenzer("⟦"), Some(("⟪", "⟫")));
        assert_eq!(waehle_begrenzer("⟧"), Some(("⟪", "⟫")));
        assert_eq!(waehle_begrenzer("⟦⟪⦃"), None);
        assert_eq!(waehle_begrenzer("⟧⟫⦄"), None);
    }

    #[test]
    fn beschaedigte_masken_werden_abgewiesen() {
        let maske = maskiere_zeile("ab cd", &[[0, 2], [3, 5]], ("⟦", "⟧"));
        assert_eq!(maske.pruefe("neu ⟦0⟧ und ⟦1⟧"), Ok(()));
        for antwort in [
            "⟦0⟧",
            "⟦0⟧⟦0⟧⟦1⟧",
            "⟦1⟧⟦0⟧",
            "⟦00⟧⟦1⟧",
            "⟦0⟧⟦1⟧⟦7⟧",
            "⟦0⟧⟦1⟧⟦",
            "⟧⟦0⟧⟦1⟧",
            "⟦0⟧⟦",
        ] {
            assert_eq!(maske.pruefe(antwort), Err(GrundCode::MaskeBeschaedigt));
        }
        assert_eq!(maske.entmaskieren("⟦0⟧ ⟦7⟧ ⟦"), "ab ⟦7⟧ ⟦");
        assert_eq!(maske.entmaskieren("ohne Platzhalter"), "ohne Platzhalter");
    }

    #[test]
    fn ohne_schutz_sind_begrenzer_trotzdem_verboten() {
        let maske = maskiere_zeile("Ä🙂ß e\u{301}", &[], ("⟦", "⟧"));
        assert_eq!(maske.anzahl(), 0);
        assert_eq!(maske.pruefe(&maske.text), Ok(()));
        assert_eq!(maske.entmaskieren(&maske.text), maske.text);
        for antwort in ["⟦", "⟧", "⟦0⟧"] {
            assert_eq!(maske.pruefe(antwort), Err(GrundCode::MaskeBeschaedigt));
        }
    }

    #[test]
    fn globale_bereiche_werden_geklammert_vereinigt_und_geprueft() {
        let basis = "vor\nÄ🙂abcdef\nnach";
        let zeile = Zeile {
            nr: 2,
            bereich: [4, 16],
        };
        let global = [(12, 99), (10, 13), (0, 6), (7, 9), (14, 14), (90, 99)];
        assert_eq!(
            schutzbereiche_der_zeile(basis, &zeile, &global),
            [[0, 2], [6, 12]]
        );
        assert!(schutzbereiche_der_zeile(basis, &zeile, &[(7, 9)]).is_empty());
        assert!(schutzbereiche_der_zeile(basis, &zeile, &[]).is_empty());
    }

    #[test]
    fn urls_satzzeichen_email_und_backticks() {
        let text = "www.a.de.,;:!?)]}\"'“”«»’ http://b.de https://c.de `ä🙂` `offen";
        let schutz = schutzbereiche_der_zeile(text, &zeile(text), &[]);
        let teile = schutz
            .iter()
            .map(|[a, b]| &text[*a..*b])
            .collect::<Vec<_>>();
        assert_eq!(teile, ["www.a.de", "http://b.de", "https://c.de", "`ä🙂`"]);
        let text = "xwww.a.de äwww.b.de _www.c.de www.d.de x.y+z@sub.example.de";
        let schutz = schutzbereiche_der_zeile(text, &zeile(text), &[]);
        let teile = schutz
            .iter()
            .map(|[a, b]| &text[*a..*b])
            .collect::<Vec<_>>();
        assert_eq!(teile, ["www.d.de", "x.y+z@sub.example.de"]);
        let text = "`https://a.de`";
        assert_eq!(
            schutzbereiche_der_zeile(text, &zeile(text), &[]),
            [[0, text.len()]]
        );
    }
}
