//! Regelweise Vorschläge mit Byte-Bereichen in der unveränderten Basis.

use super::format::{formatieren, FormatOptionen};
use crate::aufbereitung::{pruefe_aenderungen, Aenderung, AenderungArt, AenderungStatus};

fn regeln(opt: &FormatOptionen) -> [(&'static str, bool); 8] {
    [
        ("nfc", opt.nfc),
        ("ligaturen", opt.ligaturen),
        ("weiche_trennstriche", opt.weiche_trennstriche),
        ("anfuehrungszeichen", opt.anfuehrungszeichen),
        ("gedankenstrich", opt.gedankenstrich),
        ("auslassung", opt.auslassung),
        ("leerzeichen", opt.leerzeichen),
        ("listen", opt.listen),
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
        listen: regel == "listen",
    }
}

fn zeilen_hunks(alt: &str, neu: &str, position: usize) -> Vec<([usize; 2], String, String)> {
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
    let vorher = &alt[praefix..alt.len() - suffix];
    let nachher = &neu[praefix..neu.len() - suffix];
    let position = position + praefix;
    let a: Vec<char> = vorher.chars().collect();
    let b: Vec<char> = nachher.chars().collect();
    // Myers speichert je Distanz nur die erreichbaren Diagonalen, keine N×M-Tabelle.
    let mut spur: Vec<Vec<usize>> = Vec::new();
    let mut gefunden = false;
    for d in 0..=4000.min(a.len().saturating_add(b.len())) {
        let mut zeile = Vec::with_capacity(d + 1);
        for schritt in 0..=d {
            let k = 2 * schritt as isize - d as isize;
            let mut x = if d == 0 {
                0
            } else if schritt == 0 {
                spur[d - 1][0]
            } else if schritt == d {
                spur[d - 1][schritt - 1] + 1
            } else {
                let links = spur[d - 1][schritt - 1];
                let rechts = spur[d - 1][schritt];
                if links >= rechts {
                    links + 1
                } else {
                    rechts
                }
            };
            let mut y = (x as isize - k) as usize;
            while x < a.len() && y < b.len() && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            zeile.push(x);
            if x == a.len() && y == b.len() {
                gefunden = true;
                break;
            }
        }
        spur.push(zeile);
        if gefunden {
            break;
        }
    }
    if !gefunden {
        // Bei zu großer Editierdistanz bleibt das gemeinsame Präfix/Suffix ausgespart.
        return vec![(
            [position, position + vorher.len()],
            vorher.to_owned(),
            nachher.to_owned(),
        )];
    }
    let mut gleich = Vec::new();
    let (mut x, mut y) = (a.len(), b.len());
    for d in (1..spur.len()).rev() {
        let k = x as isize - y as isize;
        let schritt = ((k + d as isize) / 2) as usize;
        let vorher = &spur[d - 1];
        let einfuegen = schritt == 0 || (schritt < d && vorher[schritt - 1] < vorher[schritt]);
        let vor_schritt = if einfuegen { schritt } else { schritt - 1 };
        let vor_x = vorher[vor_schritt];
        let vor_k = 2 * vor_schritt as isize - (d - 1) as isize;
        let vor_y = (vor_x as isize - vor_k) as usize;
        while x > vor_x && y > vor_y {
            x -= 1;
            y -= 1;
            gleich.push((x, y));
        }
        x = vor_x;
        y = vor_y;
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        gleich.push((x, y));
    }
    gleich.reverse();
    let grenzen: Vec<_> = vorher
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(vorher.len()))
        .collect();
    let mut hunks = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (ende_i, ende_j) in gleich
        .into_iter()
        .chain(std::iter::once((a.len(), b.len())))
    {
        if i != ende_i || j != ende_j {
            hunks.push((
                [position + grenzen[i], position + grenzen[ende_i]],
                vorher[grenzen[i]..grenzen[ende_i]].to_owned(),
                b[j..ende_j].iter().collect(),
            ));
        }
        i = ende_i + 1;
        j = ende_j + 1;
    }
    hunks
}

fn hunk_regeln(alt: &str, neu: &str, opt: &FormatOptionen) -> u8 {
    let mut beteiligt = 0;
    let mut einzeln = None;
    for (index, (regel, aktiv)) in regeln(opt).into_iter().enumerate() {
        if !aktiv {
            continue;
        }
        let probe = formatieren(alt, &nur_regel(regel)).0;
        // Ein einzelner Schließer hat in der Einzelprobe keinen Absatzkontext.
        let zitat = regel == "anfuehrungszeichen" && alt.contains('"') && neu.contains(['„', '“']);
        if probe != alt || zitat {
            beteiligt |= 1 << index;
        }
        if einzeln.is_none()
            && (probe == neu
                || (regel == "anfuehrungszeichen" && alt == "\"" && matches!(neu, "„" | "“"))
                // Der Zeilen-Diff spart gemeinsame Ziffern und Abstände aus.
                || (regel == "listen"
                    && ((alt == ")" && neu == ".")
                        || (alt.trim_end_matches([' ', '\t', '\u{a0}']).chars().count() == 1
                            && alt.starts_with(['•', '·', '‣', '◦', '▪', '●'])
                            && matches!(neu, "-" | "- ")))))
        {
            einzeln = Some(1 << index);
        }
    }
    einzeln.unwrap_or(beteiligt)
}

fn wurzel(eltern: &[usize], mut index: usize) -> usize {
    while eltern[index] != index {
        index = eltern[index];
    }
    index
}

// Satzgrenzen sind nur Schnittstellen; formatiert wird weiterhin der ganze Text.
fn satzenden(text: &str) -> Vec<usize> {
    let mut enden = Vec::new();
    let mut vorher = None;
    for (position, zeichen) in text.char_indices() {
        if zeichen.is_whitespace() && vorher.is_some_and(|z| matches!(z, '.' | '!' | '?' | '…')) {
            enden.push(position);
        }
        vorher = Some(zeichen);
    }
    enden.push(text.len());
    enden
}

fn segment_hunks(alt: &str, neu: &str, position: usize) -> Vec<([usize; 2], String, String)> {
    let links = satzenden(alt);
    let rechts = satzenden(neu);
    if links.len() != rechts.len() {
        return zeilen_hunks(alt, neu, position);
    }
    let mut hunks = Vec::new();
    let (mut a, mut b) = (0, 0);
    for (ende_a, ende_b) in links.into_iter().zip(rechts) {
        hunks.extend(zeilen_hunks(&alt[a..ende_a], &neu[b..ende_b], position + a));
        a = ende_a;
        b = ende_b;
    }
    hunks
}

fn marken_hunks(
    alt: &str,
    neu: &str,
    position: usize,
    marken: &[(usize, usize)],
) -> Vec<([usize; 2], String, String)> {
    let mut grenzen: Vec<_> = marken
        .iter()
        .filter_map(|&(start, ende)| {
            let links = start.max(position);
            let rechts = ende.min(position + alt.len());
            if links < rechts {
                Some((links - position, rechts - position))
            } else {
                None
            }
        })
        .collect();
    grenzen.sort_unstable();
    let mut vereinigt: Vec<(usize, usize)> = Vec::new();
    for (start, ende) in grenzen {
        if let Some(letzter) = vereinigt.last_mut() {
            if start <= letzter.1 {
                letzter.1 = letzter.1.max(ende);
                continue;
            }
        }
        vereinigt.push((start, ende));
    }
    let mut hunks = Vec::new();
    let (mut a, mut b) = (0, 0);
    for (start, ende) in vereinigt {
        let Some(abstand) = neu[b..].find(&alt[start..ende]) else {
            // Eine unerwartet veränderte Marke bleibt durch den Einzelfilter geschützt.
            hunks.extend(segment_hunks(&alt[a..], &neu[b..], position + a));
            return hunks;
        };
        let ziel_start = b + abstand;
        hunks.extend(segment_hunks(
            &alt[a..start],
            &neu[b..ziel_start],
            position + a,
        ));
        a = ende;
        b = ziel_start + ende - start;
    }
    hunks.extend(segment_hunks(&alt[a..], &neu[b..], position + a));
    hunks
}

fn zeichenklasse(zeichen: char) -> Option<u8> {
    match zeichen {
        ' ' | '\t' | '\u{a0}' => Some(6),
        '"' | '„' | '“' => Some(3),
        '.' | '…' => Some(5),
        _ => None,
    }
}

fn teil_hunks(
    bereich: [usize; 2],
    alt: String,
    neu: String,
    opt: &FormatOptionen,
) -> Vec<([usize; 2], String, String)> {
    let mut teile = Vec::new();
    let (mut a, mut b) = (0, 0);
    while a < alt.len() || b < neu.len() {
        let links = alt[a..].chars().next().and_then(zeichenklasse);
        let rechts = neu[b..].chars().next().and_then(zeichenklasse);
        let klasse = match (links, rechts) {
            (Some(l), Some(r)) if l == r => l,
            (Some(6), _) | (_, Some(6)) => 6,
            (Some(l), None) if b == neu.len() => l,
            (None, Some(r)) if a == alt.len() => r,
            _ => return vec![(bereich, alt, neu)],
        };
        let laenge = |text: &str| {
            text.chars()
                .take_while(|&z| zeichenklasse(z) == Some(klasse))
                .map(char::len_utf8)
                .sum::<usize>()
        };
        let ende_a = a + laenge(&alt[a..]);
        let ende_b = b + laenge(&neu[b..]);
        let vorher = &alt[a..ende_a];
        let nachher = &neu[b..ende_b];
        if vorher != nachher {
            // Nur eindeutig zuordenbare Teilhunks verlassen den ursprünglichen Hunk.
            if hunk_regeln(vorher, nachher, opt) != 1 << klasse {
                return vec![(bereich, alt, neu)];
            }
            teile.push((
                [bereich[0] + a, bereich[0] + ende_a],
                vorher.to_owned(),
                nachher.to_owned(),
            ));
        }
        a = ende_a;
        b = ende_b;
    }
    if teile
        .windows(2)
        .any(|paar| paar[0].0[0] == paar[0].0[1] && paar[0].0[0] == paar[1].0[0])
    {
        // Eine Einfügung vor einer Ersetzung am selben Punkt bleibt ungeteilt.
        vec![(bereich, alt, neu)]
    } else {
        teile
    }
}

fn beruehrt_marke([start, ende]: [usize; 2], marken: &[(usize, usize)]) -> bool {
    marken.iter().any(|&(links, rechts)| {
        if start == ende {
            links < start && start < rechts
        } else {
            start < rechts && links < ende
        }
    })
}

/// Alle Formatregeln als Aenderungsvorschlaege (Status Offen).
/// Gruppen gelten lokal je Hunk bzw. Quote-Paar, mit stabiler Nummer je Regel-ID.
/// Untrennbare Mischhunks verbinden ausschließlich die betroffenen lokalen Quote-Paare.
/// `grund_code` und `art` enthalten unabhängig davon die beteiligten Regel-IDs.
/// Nicht zuordenbare Hunks bleiben in "format"; geschützte Hunks entfallen einzeln.
pub fn regel_aenderungen(basis: &str, opt: &FormatOptionen) -> Vec<Aenderung> {
    let ziel = formatieren(basis, opt).0;
    let vorher: Vec<_> = basis.split('\n').collect();
    let nachher: Vec<_> = ziel.split('\n').collect();
    if vorher.len() != nachher.len() {
        // Die aktuelle Pipeline erhält Zeilengrenzen; der Verzicht sichert Erweiterungen ab.
        return Vec::new();
    }
    let mut ergebnis = Vec::new();
    let marken = crate::aufbereitung::geschuetzte_bereiche(basis);
    let mut zuordnung = Vec::new();
    let mut position = 0;
    for (alt, neu) in vorher.into_iter().zip(nachher) {
        if alt != neu {
            let hunks = marken_hunks(alt, neu, position, &marken)
                .into_iter()
                .flat_map(|(bereich, alt, neu)| teil_hunks(bereich, alt, neu, opt));
            for (bereich, alt, neu) in hunks {
                if beruehrt_marke(bereich, &marken) {
                    continue;
                }
                let regeln = hunk_regeln(&alt, &neu, opt);
                zuordnung.push(regeln);
                ergebnis.push(Aenderung {
                    id: String::new(),
                    art: AenderungArt::Regel(String::new()),
                    bereich,
                    alt,
                    neu,
                    grund_code: String::new(),
                    zaehler: None,
                    gruppe: None,
                    status: AenderungStatus::Offen,
                });
            }
        }
        position += alt.len() + 1;
    }
    let mut eltern: Vec<_> = (0..ergebnis.len()).collect();
    if opt.anfuehrungszeichen && basis.contains('"') {
        let ohne_zitate = formatieren(
            basis,
            &FormatOptionen {
                anfuehrungszeichen: false,
                ..*opt
            },
        )
        .0;
        // Ohne Zitatregel bleiben die Zeichenanzahl und alle übrigen Regeln gleich.
        let mut bereiche = Vec::new();
        let (mut alt_position, mut neu_position) = (0, 0);
        for hunk in &ergebnis {
            neu_position += hunk.bereich[0] - alt_position;
            bereiche.push((neu_position, neu_position + hunk.neu.len()));
            neu_position += hunk.neu.len();
            alt_position = hunk.bereich[1];
        }
        let mut index = 0;
        let mut offen = None;
        for ((position, zeichen), vorher) in ziel.char_indices().zip(ohne_zitate.chars()) {
            if vorher != '"' || !matches!(zeichen, '„' | '“') {
                continue;
            }
            while index < bereiche.len() && bereiche[index].1 <= position {
                index += 1;
            }
            if index == bereiche.len() || position < bereiche[index].0 {
                continue;
            }
            if zeichen == '„' {
                offen = Some(index);
            } else if let Some(oeffner) = offen.take() {
                let links = wurzel(&eltern, oeffner);
                let rechts = wurzel(&eltern, index);
                eltern[rechts] = links;
            }
        }
    }
    let mut nummern = std::collections::HashMap::new();
    let mut gruppen = std::collections::HashMap::new();
    let mut gruppen_nummern = std::collections::HashMap::new();
    let mut gruppen_regeln = vec![0_u8; ergebnis.len()];
    for (index, &maske) in zuordnung.iter().enumerate() {
        gruppen_regeln[wurzel(&eltern, index)] |= maske;
    }
    for (index, (hunk, maske)) in ergebnis.iter_mut().zip(zuordnung).enumerate() {
        let regel = if maske == 0 {
            "format".to_owned()
        } else {
            regeln(opt)
                .into_iter()
                .enumerate()
                .filter(|(index, _)| maske & (1 << *index) != 0)
                .map(|(_, (regel, _))| regel)
                .collect::<Vec<_>>()
                .join("+")
        };
        let wurzel = wurzel(&eltern, index);
        let gruppe = gruppen.entry(wurzel).or_insert_with(|| {
            let name = if gruppen_regeln[wurzel] & (1 << 3) != 0 {
                "anfuehrungszeichen"
            } else {
                regel.as_str()
            };
            let nummer = gruppen_nummern.entry(name.to_owned()).or_insert(0_usize);
            *nummer += 1;
            format!("{name}#{nummer}")
        });
        let nummer = nummern.entry(regel.clone()).or_insert(0_usize);
        *nummer += 1;
        hunk.id = format!("{regel}-{nummer}");
        hunk.art = AenderungArt::Regel(regel.clone());
        hunk.grund_code = regel;
        hunk.gruppe = Some(gruppe.clone());
    }
    ergebnis.sort_by_key(|a| a.bereich);
    if pruefe_aenderungen(basis, &ergebnis).is_err() {
        // Andere ungültige Gesamtdaten werden weiterhin sicher verworfen.
        Vec::new()
    } else {
        ergebnis
    }
}

/// Byte-Bereiche als UTF-16-Indizes für JavaScript; ungültige Bereiche ergeben MAX.
pub fn utf16_bereiche(text: &str, aenderungen: &[Aenderung]) -> Vec<[usize; 2]> {
    let mut ergebnis = vec![[usize::MAX; 2]; aenderungen.len()];
    let mut grenzen = Vec::new();
    for (index, a) in aenderungen.iter().enumerate() {
        let [start, ende] = a.bereich;
        if start <= ende && text.is_char_boundary(start) && text.is_char_boundary(ende) {
            grenzen.push((start, index, 0));
            grenzen.push((ende, index, 1));
        }
    }
    if !grenzen.windows(2).all(|paar| paar[0] <= paar[1]) {
        grenzen.sort_unstable();
    }
    let mut zeichen = text.chars();
    let (mut byte, mut utf16) = (0, 0);
    for (ziel, index, seite) in grenzen {
        while byte < ziel {
            if let Some(z) = zeichen.next() {
                byte += z.len_utf8();
                utf16 += z.len_utf16();
            } else {
                break;
            }
        }
        ergebnis[index][seite] = utf16;
    }
    ergebnis
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aufbereitung::{ergebnis_text, geschuetzte_bereiche, ErgebnisFehler};

    fn marken(text: &str) -> Vec<String> {
        geschuetzte_bereiche(text)
            .into_iter()
            .map(|(start, ende)| text[start..ende].to_owned())
            .collect()
    }

    #[test]
    fn beispieltexte_mit_vollannahme_teilannahme_und_idempotenz() {
        // Gegenproben: Ohne Pipeline-Diff scheitern die Regelkombinationen bei Vollannahme.
        // Ohne getrennte Hunks scheitern insbesondere "ﬃ ... ﬀ" und Segmente neben Marken.
        let beispiele = [
            (
                "alle Regeln",
                "Cafe\u{301}\nﬃ\nSil\u{ad}be\n\"Ja\"\nA - B\nEnde...\nA  B\n• Punkt",
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
            // Die Format-Beispiele decken auch Kontext, Masken und unveränderte Fälle ab.
            ("NFC einzeln", "Cafe\u{301}"),
            ("NFC nach Trennstrich", "A\u{301} e\u{ad}\u{301}"),
            ("Code inline", "`\"A\u{301}  ﬃ\u{ad} - x...y\"`"),
            ("Code daneben", "`x...y` Ende..."),
            ("Code offen", "`offen\nx...y"),
            (
                "Satzzeichen im Wort",
                "Datei .env Pegel .5 2 : 1 :-) a ,b a ;b a !b a ?b a ./ a .)",
            ),
            ("Satzzeichen am Ende", "A . B : C , D ; E ! F ?\nA .\nB :"),
            (
                "Abstände geschützt",
                "A  \nB\nA\tB\t \nC\n| A  B\t | C   |  \n",
            ),
            (
                "Zitat und Trennlinie",
                "> > - Punkt\n> Wort - Wort\n- - -\n* * *\n---",
            ),
            ("Offene Maske", "[offen\n\nEnde...]"),
            ("NBSP am Komma", "A\u{a0},"),
            ("Zitat unsicher", "Er sagte:\"Hallo\""),
            (
                "Private Zeichen",
                "\u{e000}\u{e010}\u{e001} [ﬃ  ...] \"Ja\"",
            ),
            ("Weicher Trennstrich einzeln", "Sil\u{ad}be"),
            ("Gedankenstrich einzeln", "Wort - Wort"),
            (
                "Kontrolltext",
                "Ein sauberer Absatz mit Umlauten und Schluss.",
            ),
            (
                "Maskierte Regeln",
                concat!(
                    "\"<A\u{301}  ﬃ\u{ad} - ...>\"  {{S. 12}} ",
                    "...[\"A\u{301}\"  ﬃ\u{ad} - ...] ,"
                ),
            ),
            ("Unausgeglichene Zitate", "Wort\" Wort\""),
            (
                "Alle Schalter",
                "A\u{301}\nﬃ\nSil\u{ad}be\n\"Wort\"\nA - B\nEnde...\nA  B",
            ),
            ("Zweiter Lauf", "A  -\t B und Ende . . ."),
            ("Berichtsreihenfolge", "\"Cafe\u{301}  ﬃ\u{ad} - Ende...\""),
            (
                "Buchstaben und Ziffern",
                "\"Anna  123\" - Sil\u{ad}be... {{S. 12}} <Berta> [lacht]",
            ),
            ("UTF-8 an Masken", "[🦀\n...]\n``界`界``\n~~~\nä\n~~~\nä ."),
            ("Ligaturen neben Sprecher", "ﬃ <Anna> ﬀ"),
            ("Leerzeichen neben Sprecher", "A  <Anna>  B"),
            ("Überlagerte Regelsegmente", "ﬃ ... ﬀ"),
            ("Zitate über Zeilengrenze", "\"A\" <Anna> \"B\nC\""),
            ("Trennstrich und NFC", "e\u{ad}\u{301}"),
            ("NBSP vor Gedankenstrich", "A\u{a0}- B"),
            ("Leerzeichen und Gedankenstrich", "a  -  b"),
            ("Seitenmarke im Segment", "ﬃ  {{S. 12}}  ﬀ"),
            ("Zeitanker und Hard-Break", "ﬃ  00:12  Ende...  \nB"),
            (
                "Listenzeichen",
                "• Eins\n· Zwei\n‣ Drei\n◦ Vier\n▪ Fünf\n● Sechs",
            ),
            ("Listen mit Einzug", "   • 🦀\n  ◦\tÖl"),
            ("Nummerierte Liste", "1) Eins\n2) Zwei\n3) Drei"),
            ("Nummern mit Übertrag", "9) Neun\r\n10) Zehn\r\n"),
            ("Einzelnummer", "1) Punkt"),
            ("Nummernlücke", "2) Zwei\n4) Vier"),
            ("Nummern mit Leerzeile", "1) Eins\n\n2) Zwei"),
            (
                "Listen neben Marken",
                "• {{S. 12}} <Anna> [lacht]\n1) `x`\n2) Öl",
            ),
            ("Listen im Code", "```\n• x\n1) Eins\n2) Zwei\n```\n`• x`"),
            ("Listen in Strukturen", "| • |\n> • x\n<Anna> • x\n– Wohin?"),
            ("Listen mit Abständen", "•  Eins\n◦\t\tZwei"),
            ("Listen vor Satzzeichen", "•\t.\n•\t- x"),
        ];
        let opt = FormatOptionen::default();
        for (name, basis) in beispiele {
            let aenderungen = regel_aenderungen(basis, &opt);
            assert_eq!(pruefe_aenderungen(basis, &aenderungen), Ok(()), "{name}");
            assert_eq!(ergebnis_text(basis, &aenderungen).unwrap(), basis, "{name}");
            assert!(aenderungen.windows(2).all(|a| a[0].bereich <= a[1].bereich));
            for a in &aenderungen {
                assert_eq!(a.status, AenderungStatus::Offen, "{name}");
                assert_eq!(a.art, AenderungArt::Regel(a.grund_code.clone()));
                assert!(a
                    .gruppe
                    .as_deref()
                    .is_some_and(|gruppe| gruppe.contains('#')));
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
            let mut abgelehnt = aenderungen.clone();
            for a in &mut abgelehnt {
                a.status = AenderungStatus::Abgelehnt;
            }
            assert_eq!(ergebnis_text(basis, &abgelehnt).unwrap(), basis, "{name}");
            let mut gruppen: Vec<_> = aenderungen
                .iter()
                .filter_map(|a| a.gruppe.as_deref())
                .collect();
            gruppen.sort_unstable();
            gruppen.dedup();
            let alle = (1_usize << gruppen.len()) - 1;
            let masken: Vec<_> = if gruppen.len() <= 6 {
                (1..=alle).collect()
            } else {
                let mut zustand = 658_u32;
                (0..20)
                    .map(|_| {
                        zustand = zustand.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        zustand as usize % alle + 1
                    })
                    .collect()
            };
            // Gegenprobe: Nur ganze Gruppen dürfen angenommen werden, auch "format".
            for maske in masken {
                let mut teilweise = aenderungen.clone();
                for a in &mut teilweise {
                    if gruppen.iter().enumerate().any(|(index, gruppe)| {
                        maske & (1 << index) != 0 && a.gruppe.as_deref() == Some(*gruppe)
                    }) {
                        a.status = AenderungStatus::Angenommen;
                    }
                }
                assert_eq!(pruefe_aenderungen(basis, &teilweise), Ok(()), "{name}");
                let text = ergebnis_text(basis, &teilweise).unwrap();
                assert_eq!(marken(&text), marken(basis), "{name}: {maske}");
                if !basis.contains(['„', '“']) {
                    assert_eq!(
                        text.matches('„').count(),
                        text.matches('“').count(),
                        "{name}"
                    );
                }
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
            "– Wohin?\n— Dorthin!\n1) Punkt",
            "2) Zwei\n4) Vier",
            "```\n• x\n```\n| • |\n> • x\n<Anna> • x",
            "[• x]\n<• x>\n{{S. 12}}",
        ] {
            assert!(regel_aenderungen(basis, &FormatOptionen::default()).is_empty());
        }
    }

    #[test]
    fn deaktivierte_regeln_liefern_keine_vorschlaege() {
        let basis = "A\u{301}\nﬃ\nSil\u{ad}be\n\"Ja\"\nA - B\nEnde...\nA  B\n• Punkt";
        assert!(regel_aenderungen(basis, &nur_regel("")).is_empty());
        for (regel, _) in regeln(&FormatOptionen::default()) {
            let a = regel_aenderungen(basis, &nur_regel(regel));
            assert_eq!(a.len(), if regel == "anfuehrungszeichen" { 2 } else { 1 });
            // Ohne Kontext ist der einzelne Gedankenstrich keiner Einzelprobe zuzuordnen.
            let gruppe = if regel == "gedankenstrich" {
                "format"
            } else {
                regel
            };
            for (index, hunk) in a.iter().enumerate() {
                assert_eq!(hunk.grund_code, gruppe);
                assert_eq!(hunk.id, format!("{gruppe}-{}", index + 1));
            }
        }
    }

    #[test]
    fn listen_hunks_erhalten_trotz_fehlendem_kontext_die_regelgruppe() {
        // Gegenprobe: Die Einzelprobe auf ")" oder "•" erkennt keine ganze Liste.
        for (basis, ziel, anzahl) in [
            ("• Eins\n◦\tZwei", "- Eins\n- Zwei", 2),
            (
                "9) Neun\n10) Zehn\n11) Elf",
                "9. Neun\n10. Zehn\n11. Elf",
                3,
            ),
            (
                "• {{S. 12}} <Anna> [lacht]",
                "- {{S. 12}} <Anna> [lacht]",
                1,
            ),
        ] {
            let opt = nur_regel("listen");
            let (text, bericht) = formatieren(basis, &opt);
            assert_eq!(text, ziel);
            assert_eq!(bericht.regeln.len(), 1);
            assert_eq!(bericht.regeln[0].regel, "listen");
            assert_eq!(bericht.regeln[0].anzahl, anzahl);
            let mut a = regel_aenderungen(basis, &opt);
            assert_eq!(a.len(), anzahl as usize);
            assert_eq!(pruefe_aenderungen(basis, &a), Ok(()));
            assert_eq!(ergebnis_text(basis, &a).unwrap(), basis);
            for (index, hunk) in a.iter_mut().enumerate() {
                assert_eq!(hunk.grund_code, "listen");
                assert_eq!(hunk.id, format!("listen-{}", index + 1));
                hunk.status = AenderungStatus::Angenommen;
            }
            assert_eq!(ergebnis_text(basis, &a).unwrap(), ziel);
            assert!(regel_aenderungen(ziel, &opt).is_empty());
            assert_eq!(marken(ziel), marken(basis));
        }
        let basis = "• Eins\n1) Eins\n2) Zwei";
        let opt = FormatOptionen {
            listen: false,
            ..FormatOptionen::default()
        };
        assert_eq!(formatieren(basis, &opt).0, basis);
        assert!(formatieren(basis, &opt).1.regeln.is_empty());
        assert!(regel_aenderungen(basis, &opt).is_empty());
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
    fn lcs_trennt_hunks_an_gleichen_zeichen_und_achtet_zeichengrenzen() {
        // Gegenprobe: Ein Segment je Zeile würde das unveränderte " ö " einschließen.
        let a = zeilen_hunks("ä x ö x 🦀", "ä y ö z 🦀", 10);
        assert_eq!(
            a,
            vec![
                ([13, 14], "x".to_owned(), "y".to_owned()),
                ([18, 19], "x".to_owned(), "z".to_owned()),
            ]
        );
        assert_eq!(
            zeilen_hunks("ä", "ö", 0),
            vec![([0, 2], "ä".to_owned(), "ö".to_owned())]
        );
    }

    #[test]
    fn einfuegen_und_loeschen_an_utf8_grenzen() {
        // Gegenprobe: Zeichenindizes als Bytebereiche würden mitten im Mehrbytezeichen liegen.
        let a = zeilen_hunks("🦀ö", "🦀äö", 0);
        assert_eq!(a, vec![([4, 4], String::new(), "ä".to_owned())]);
        let a = regel_aenderungen("ä\u{ad}ö", &FormatOptionen::default());
        assert_eq!(a[0].bereich, [2, 4]);
        assert_eq!(a[0].alt, "\u{ad}");
        assert_eq!(a[0].neu, "");
        assert_eq!(
            zeilen_hunks("🦀äö", "🦀ö", 0),
            vec![([4, 6], "ä".to_owned(), String::new())]
        );
    }

    #[test]
    fn hunks_neben_marken_bleiben_erhalten() {
        // Gegenprobe: Der alte Zeilenbereich berührt die Marke und verwirft beide Änderungen.
        for basis in [
            "\"{{S. 12}}\"",
            "\"<Anna>\"",
            "\"[lacht]\"",
            "\"00:12\"",
            "ﬃ `Code` ﬀ",
            "ﬃ <Anna> ﬀ",
            "A  <Anna>  B",
            "ﬃ  {{S. 12}}  ﬀ",
        ] {
            let mut a = regel_aenderungen(basis, &FormatOptionen::default());
            assert!(a.len() >= 2);
            assert_eq!(pruefe_aenderungen(basis, &a), Ok(()));
            for hunk in &mut a {
                hunk.status = AenderungStatus::Angenommen;
            }
            let text = ergebnis_text(basis, &a).unwrap();
            assert_eq!(text, formatieren(basis, &FormatOptionen::default()).0);
            assert_eq!(marken(&text), marken(basis));
        }
    }

    #[test]
    fn erweiterte_masken_erhalten_uebrige_hunks() {
        // Gegenprobe: Die bisherige Maske verändert diese Marken oder leert die Liste.
        for basis in ["{{S.12}}", "{{S.  12}}", "[ ]", "< >"] {
            assert!(regel_aenderungen(basis, &FormatOptionen::default()).is_empty());
        }
        for mitte in [
            format!("[{}ﬃ]", "ä".repeat(100)),
            format!("<{}ﬃ>", "ä".repeat(50)),
            "{{S.12}}".to_owned(),
            "00:12... 01:02:03.456...".to_owned(),
            format!("[{}ﬃ", "ä".repeat(100)),
        ] {
            let basis = format!("ﬃ {mitte} ﬀ Ende...");
            let mut a = regel_aenderungen(&basis, &FormatOptionen::default());
            assert!(!a.is_empty());
            assert_eq!(pruefe_aenderungen(&basis, &a), Ok(()));
            for hunk in &mut a {
                hunk.status = AenderungStatus::Angenommen;
            }
            let text = ergebnis_text(&basis, &a).unwrap();
            assert_eq!(text, formatieren(&basis, &FormatOptionen::default()).0);
            assert_eq!(marken(&text), marken(&basis));
        }
    }

    #[test]
    fn mehrere_regeln_auf_einer_zeile_gehen_nicht_verloren() {
        // Gegenprobe: Der frühere Überlappungsfilter verliert hier die Auslassung.
        let basis = "ﬃ ... ﬀ";
        let mut a = regel_aenderungen(basis, &FormatOptionen::default());
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].grund_code, "ligaturen");
        assert_eq!(a[1].grund_code, "auslassung");
        assert_eq!(a[2].grund_code, "ligaturen");
        assert_eq!(a[2].id, "ligaturen-2");
        assert_eq!(pruefe_aenderungen(basis, &a), Ok(()));
        for hunk in &mut a {
            hunk.status = AenderungStatus::Angenommen;
        }
        let text = ergebnis_text(basis, &a).unwrap();
        assert_eq!(text, "ffi … ff");
        assert!(regel_aenderungen(&text, &FormatOptionen::default()).is_empty());
    }

    #[test]
    fn gruppenstatus_muss_einheitlich_sein() {
        let basis = "\"Ja\"";
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
    fn grosse_mittelteile_bleiben_einzeln_annehmbar() {
        // Gegenprobe: Der bisherige Größenrückfall liefert nur einen großen Hunk.
        let basis = format!("äﬃ{}ﬀ🦀", "x".repeat(2001));
        let ziel = formatieren(&basis, &FormatOptionen::default()).0;
        let hunks = zeilen_hunks(&basis, &ziel, 0);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].0, [2, 5]);
        let mut a = regel_aenderungen(&basis, &FormatOptionen::default());
        assert_eq!(a.len(), 2);
        for hunk in &mut a {
            hunk.status = AenderungStatus::Angenommen;
        }
        assert_eq!(ergebnis_text(&basis, &a).unwrap(), ziel);

        // Gegenprobe: Der bisherige Größenrückfall verwirft auch die anderen Treffer.
        let basis = format!("Ende...\nﬃ{}<Anna>ﬀ", "x".repeat(2001));
        assert_eq!(
            regel_aenderungen(&basis, &FormatOptionen::default()).len(),
            3
        );
    }

    #[test]
    fn zusammenwirkende_regeln_und_zitatkontext_werden_zugeordnet() {
        // Gegenprobe: Einzelne Basisläufe ergeben weder das zusammengesetzte é noch den Strich.
        for basis in ["e\u{ad}\u{301}", "A\u{a0}- B", "a  -  b"] {
            let a = regel_aenderungen(basis, &FormatOptionen::default());
            assert!(!a.is_empty());
            let erwartet = match basis {
                "e\u{ad}\u{301}" => "weiche_trennstriche",
                "a  -  b" => "leerzeichen",
                _ => "format",
            };
            assert!(a.iter().any(|hunk| hunk.grund_code == erwartet));
        }
        // Gegenprobe: Die Einzelprobe auf einem Anführungszeichen kennt den Absatz nicht.
        let a = regel_aenderungen("\"A\" <Anna> \"B\nC\"", &FormatOptionen::default());
        assert_eq!(a.len(), 4);
        for hunk in a {
            assert_eq!(hunk.alt, "\"");
            assert!(matches!(hunk.neu.as_str(), "„" | "“"));
            assert_eq!(hunk.grund_code, "anfuehrungszeichen");
        }
    }

    #[test]
    fn misch_hunks_erhalten_lokale_zitatpaare() {
        // Gegenprobe: Globale Regel-Union verbindet verschiedene Paare miteinander.
        for basis in ["\"Warte...\"", "\"A...\" und \"B...\""] {
            let a = regel_aenderungen(basis, &FormatOptionen::default());
            let mut gruppen: Vec<_> = a.iter().filter_map(|hunk| hunk.gruppe.as_deref()).collect();
            gruppen.sort_unstable();
            gruppen.dedup();
            assert_eq!(gruppen.len(), basis.matches('"').count());
            assert!(a.iter().all(|hunk| !hunk.grund_code.contains('+')));
            for maske in 0..(1 << gruppen.len()) {
                let mut teilweise = a.clone();
                for hunk in &mut teilweise {
                    if gruppen.iter().enumerate().any(|(index, gruppe)| {
                        maske & (1 << index) != 0 && hunk.gruppe.as_deref() == Some(*gruppe)
                    }) {
                        hunk.status = AenderungStatus::Angenommen;
                    }
                    assert_eq!(hunk.art, AenderungArt::Regel(hunk.grund_code.clone()));
                }
                let text = ergebnis_text(basis, &teilweise).unwrap();
                assert_eq!(text.matches('„').count(), text.matches('“').count());
            }
        }
    }

    #[test]
    fn schutznetz_schneidet_vor_dem_diff_an_marken() {
        // Gegenprobe: Ohne Schnitt entfällt der Mittelteil mit über 4000 Editierschritten.
        let basis = format!(
            "a{}<Anna>{}b\nEnde...",
            "\u{ad}".repeat(2001),
            "\u{ad}".repeat(2001)
        );
        let mut a = regel_aenderungen(&basis, &FormatOptionen::default());
        assert_eq!(a.len(), 3);
        assert_eq!(pruefe_aenderungen(&basis, &a), Ok(()));
        for hunk in &mut a {
            hunk.status = AenderungStatus::Angenommen;
        }
        let text = ergebnis_text(&basis, &a).unwrap();
        assert_eq!(text, formatieren(&basis, &FormatOptionen::default()).0);
        assert_eq!(marken(&text), marken(&basis));
        // Einfügungen an den Markengrenzen bleiben ausdrücklich erlaubt.
        assert!(!beruehrt_marke([1, 1], &[(1, 7)]));
        assert!(!beruehrt_marke([7, 7], &[(1, 7)]));
        assert!(beruehrt_marke([2, 2], &[(1, 7)]));
    }

    #[test]
    fn ocr_text_behaelt_viele_kleine_gruppen() {
        // Gegenprobe zu #700/1: Die globale Union erzeugt eine einzige Riesengruppe.
        let basis = "\"Ja\"  Ende...  \"Warte...\"  <Anna>  \n".repeat(500);
        let opt = FormatOptionen::default();
        let a = regel_aenderungen(&basis, &opt);
        let mut gruppen = std::collections::BTreeMap::new();
        for (index, hunk) in a.iter().enumerate() {
            gruppen
                .entry(hunk.gruppe.as_deref().unwrap())
                .or_insert_with(Vec::new)
                .push(index);
            assert_eq!(hunk.art, AenderungArt::Regel(hunk.grund_code.clone()));
        }
        assert!(gruppen.len() >= 100);
        assert!(gruppen.values().all(|gruppe| gruppe.len() <= 10));
        assert_eq!(pruefe_aenderungen(&basis, &a), Ok(()));
        assert_eq!(ergebnis_text(&basis, &a).unwrap(), basis);
        for gruppe in gruppen.values() {
            // Gleiche Bilanz je Gruppe sichert jede beliebige Vereinigung von Gruppen.
            let oeffner: usize = gruppe.iter().map(|&i| a[i].neu.matches('„').count()).sum();
            let schliesser: usize = gruppe.iter().map(|&i| a[i].neu.matches('“').count()).sum();
            assert_eq!(oeffner, schliesser);
        }
        for rest in 0..7 {
            let mut teilweise = a.clone();
            for (nummer, gruppe) in gruppen.values().enumerate() {
                if nummer % 7 == rest {
                    for &index in gruppe {
                        teilweise[index].status = AenderungStatus::Angenommen;
                    }
                }
            }
            let text = ergebnis_text(&basis, &teilweise).unwrap();
            assert_eq!(marken(&text), marken(&basis));
            assert_eq!(text.matches('„').count(), text.matches('“').count());
        }
        let mut angenommen = a;
        for hunk in &mut angenommen {
            hunk.status = AenderungStatus::Angenommen;
        }
        let text = ergebnis_text(&basis, &angenommen).unwrap();
        assert_eq!(text, formatieren(&basis, &opt).0);
        assert!(regel_aenderungen(&text, &opt).is_empty());
    }

    #[test]
    fn misch_hunks_trennen_leerzeichen_auslassung_und_zitate() {
        // Gegenprobe zu #700/1: Der alte Hunk bindet den Abstand an das Zitatpaar.
        let basis = "\"Warte...\"  ";
        let opt = FormatOptionen::default();
        let a = regel_aenderungen(basis, &opt);
        for regel in ["anfuehrungszeichen", "auslassung", "leerzeichen"] {
            assert!(a.iter().any(|hunk| hunk.grund_code == regel));
        }
        assert!(a.iter().all(|hunk| !hunk.grund_code.contains('+')));
        let mut teilweise = a;
        for hunk in &mut teilweise {
            if hunk.grund_code == "leerzeichen" {
                hunk.status = AenderungStatus::Angenommen;
            }
        }
        assert_eq!(ergebnis_text(basis, &teilweise).unwrap(), "\"Warte...\"");
    }

    #[test]
    fn offene_marken_werden_linear_und_mit_gemeinsamen_grenzen_geprueft() {
        // Gegenprobe zu #700/3: Jede offene Marke durchsucht sonst den ganzen Rest.
        let basis = "<[".repeat(100_000);
        assert_eq!(basis.len(), 200_000);
        let start = std::time::Instant::now();
        assert!(geschuetzte_bereiche(&basis).is_empty());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        for (oeffner, schliesser, grenze) in [("<", ">", 40), ("[", "]", 80)] {
            let erlaubt = format!("{oeffner}{}{schliesser}", "ä".repeat(grenze));
            assert_eq!(geschuetzte_bereiche(&erlaubt), vec![(0, erlaubt.len())]);
            let zu_lang = format!("{oeffner}{}...{schliesser}", "ä".repeat(grenze));
            assert!(geschuetzte_bereiche(&zu_lang).is_empty());
            assert_eq!(
                formatieren(&zu_lang, &FormatOptionen::default()).0,
                zu_lang.replace("...", "…")
            );
            for umbruch in ["\n", "\r", "\r\n"] {
                let text = format!("{oeffner}a{umbruch}...{schliesser}");
                assert!(geschuetzte_bereiche(&text).is_empty());
                assert_eq!(
                    formatieren(&text, &FormatOptionen::default()).0,
                    text.replace("...", "…")
                );
            }
        }
        for text in ["{{S.12}}", "{{S.  12}}", "<Anna>", "[lacht]", "00:12"] {
            assert_eq!(geschuetzte_bereiche(text), vec![(0, text.len())]);
        }
        assert!(geschuetzte_bereiche("{{S.\n12}}").is_empty());
    }

    #[test]
    fn hunderttausend_zeichen_bleiben_schnell_und_fein_aufgeteilt() {
        // Gegenprobe zu 3/4: Riesen-Hunk und quadratische Strichsuche scheitern hier.
        let mut basis = format!("{} ﬃ Ende... Wort - Wort A  B ", "x".repeat(960)).repeat(100);
        let rest = 100_000 - basis.chars().count();
        basis.push_str(&"x".repeat(rest));
        assert_eq!(basis.chars().count(), 100_000);
        let start = std::time::Instant::now();
        let mut a = regel_aenderungen(&basis, &FormatOptionen::default());
        assert!(a.len() >= 100);
        for hunk in &mut a {
            hunk.status = AenderungStatus::Angenommen;
        }
        assert_eq!(
            ergebnis_text(&basis, &a).unwrap(),
            formatieren(&basis, &FormatOptionen::default()).0
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn utf16_bereiche_beachten_mehrbytezeichen_und_eingabereihenfolge() {
        // Gegenprobe: Byte-Indizes zählen Umlaute, Euro und Emoji falsch.
        let text = "ä€😀Z";
        let vorlage = regel_aenderungen("ﬃ", &FormatOptionen::default()).remove(0);
        let bereiche = [
            [5, 9],
            [0, 2],
            [2, 5],
            [10, 10],
            [1, 2],
            [0, 11],
            [5, 4],
            [usize::MAX, usize::MAX],
            [9, 10],
        ];
        let a: Vec<_> = bereiche
            .into_iter()
            .map(|bereich| Aenderung {
                bereich,
                ..vorlage.clone()
            })
            .collect();
        assert_eq!(
            utf16_bereiche(text, &a),
            vec![
                [2, 4],
                [0, 1],
                [1, 2],
                [5, 5],
                [usize::MAX; 2],
                [usize::MAX; 2],
                [usize::MAX; 2],
                [usize::MAX; 2],
                [4, 5],
            ]
        );
        assert!(utf16_bereiche(text, &[]).is_empty());
        let mut leer = vorlage;
        leer.bereich = [0, 0];
        assert_eq!(utf16_bereiche("", &[leer]), vec![[0, 0]]);
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
                format!("• {zeichen}\n1) {zeichen}\n2) {zeichen}"),
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
