use std::collections::HashMap;

use super::{
    Aenderung, AenderungStatus, Aufbereitung, Basis, ErgebnisFehler, Status, Struktur,
    StrukturVergleich,
};

pub const AUFBEWAHRUNG_MS: u64 = 30 * 24 * 60 * 60 * 1000;

pub fn konflikt(basis: &Basis, aktueller_text: &str) -> bool {
    *basis != Basis::aus_text(aktueller_text)
}

pub fn abgelaufen(aufbereitung: &Aufbereitung, jetzt_ms: u64) -> bool {
    matches!(aufbereitung.status, Status::Verworfen | Status::Veraltet)
        && jetzt_ms.saturating_sub(aufbereitung.geaendert) >= AUFBEWAHRUNG_MS
}

/// Außen: aktive ID gefunden; innen: Vorgänger-ID oder Original (None).
pub fn rueckgaengig(kette: &[Aufbereitung], aktiv: &str) -> Option<Option<String>> {
    kette
        .iter()
        .find(|fassung| fassung.id == aktiv)
        .map(|fassung| fassung.vorgaenger_aktiv.clone())
}

/// Prüft auch offene und abgelehnte Vorschläge, damit Teilannahmen sicher bleiben.
pub fn pruefe_aenderungen(
    basis_text: &str,
    aenderungen: &[Aenderung],
) -> Result<(), ErgebnisFehler> {
    let marken = geschuetzte_bereiche(basis_text);
    let mut gruppen = HashMap::new();
    let mut sortiert: Vec<_> = aenderungen.iter().collect();
    sortiert.sort_by_key(|a| a.bereich);
    for (index, aenderung) in sortiert.iter().enumerate() {
        let [start, ende] = aenderung.bereich;
        let alt = basis_text.get(start..ende).ok_or(ErgebnisFehler::Bereich)?;
        if alt != aenderung.alt {
            return Err(ErgebnisFehler::Alt);
        }
        if index > 0 {
            let [vor_start, vor_ende] = sortiert[index - 1].bereich;
            if start < vor_ende || (start == vor_start && start == vor_ende) {
                return Err(ErgebnisFehler::Ueberlappung);
            }
        }
        if marken.iter().any(|&(marke_start, marke_ende)| {
            if start == ende {
                marke_start < start && start < marke_ende
            } else {
                start < marke_ende && marke_start < ende
            }
        }) {
            return Err(ErgebnisFehler::Geschuetzt);
        }
        if let Some(gruppe) = &aenderung.gruppe {
            if let Some(status) = gruppen.insert(gruppe, aenderung.status) {
                if status != aenderung.status {
                    return Err(ErgebnisFehler::Gruppe);
                }
            }
        }
    }
    Ok(())
}

pub fn ergebnis_text(
    basis_text: &str,
    aenderungen: &[Aenderung],
) -> Result<String, ErgebnisFehler> {
    pruefe_aenderungen(basis_text, aenderungen)?;
    let mut angenommen: Vec<_> = aenderungen
        .iter()
        .filter(|a| a.status == AenderungStatus::Angenommen)
        .collect();
    angenommen.sort_by_key(|a| a.bereich);
    let mut text = String::new();
    let mut position = 0;
    for aenderung in angenommen {
        let [start, ende] = aenderung.bereich;
        text.push_str(&basis_text[position..start]);
        text.push_str(&aenderung.neu);
        position = ende;
    }
    text.push_str(&basis_text[position..]);
    Ok(text)
}

/// Der Aufrufer veröffentlicht erst den erfolgreich zurückgegebenen Text.
pub fn uebernehmen(
    aufbereitung: &mut Aufbereitung,
    aktueller_text: &str,
    jetzt_ms: u64,
) -> Result<String, ErgebnisFehler> {
    if aufbereitung.status != Status::Entwurf {
        return Err(ErgebnisFehler::Status);
    }
    if konflikt(&aufbereitung.basis, aktueller_text) {
        aufbereitung.status = Status::Veraltet;
        aufbereitung.geaendert = jetzt_ms;
        return Err(ErgebnisFehler::Konflikt);
    }
    let text = ergebnis_text(aktueller_text, &aufbereitung.aenderungen)?;
    aufbereitung.status = Status::Aktiv;
    aufbereitung.geaendert = jetzt_ms;
    Ok(text)
}

/// Markdown-Struktur vor/nach der Aufbereitung, ohne Darstellung oder Seiteneffekte.
pub fn struktur_vergleich(vorher: &str, nachher: &str) -> StrukturVergleich {
    StrukturVergleich {
        vorher: struktur(vorher),
        nachher: struktur(nachher),
    }
}

fn struktur(text: &str) -> Struktur {
    let mut struktur = Struktur {
        ueberschriften: Vec::new(),
        seitenmarken: geschuetzte_bereiche(text)
            .into_iter()
            .filter_map(|(start, ende)| {
                let marke = &text[start..ende];
                marke.starts_with("{{S.").then(|| marke.to_owned())
            })
            .collect(),
        absaetze: Vec::new(),
        tabellen: Vec::new(),
    };
    let mut absatz = Vec::new();
    let mut tabelle = Vec::new();
    for zeile in text.lines().chain(std::iter::once("")) {
        let zeile = zeile.trim();
        let tabellenzeile = zeile.starts_with('|') && zeile.ends_with('|') && zeile.len() > 1;
        if !tabellenzeile && !tabelle.is_empty() {
            struktur.tabellen.push(tabelle.join("\n"));
            tabelle.clear();
        }
        let ohne_rauten = zeile.trim_start_matches('#');
        let rauten = zeile.len() - ohne_rauten.len();
        let ueberschrift = (1..=6).contains(&rauten) && ohne_rauten.starts_with(' ');
        if (zeile.is_empty() || tabellenzeile || ueberschrift) && !absatz.is_empty() {
            struktur.absaetze.push(absatz.join("\n"));
            absatz.clear();
        }
        if tabellenzeile {
            tabelle.push(zeile);
        } else if ueberschrift {
            struktur.ueberschriften.push(zeile.to_owned());
        } else if !zeile.is_empty() {
            absatz.push(zeile);
        }
    }
    struktur
}

/// Marken enden innerhalb derselben Zeile (auch bei CR ohne LF).
/// Anders als früher schützen `<…>` höchstens 40 und `[…]` höchstens 80 Inhaltszeichen;
/// spätere oder zeilenübergreifende Schließer begründen keinen Schutzbereich mehr.
pub(crate) fn geschuetzte_bereiche(text: &str) -> Vec<(usize, usize)> {
    let mut bereiche = Vec::new();
    for (oeffner, schliesser) in [("{{S.", "}}"), ("<", ">"), ("[", "]")] {
        let mut zeilen_start = 0;
        for zeile in text.split_inclusive(['\n', '\r']) {
            // Beide Suchiteratoren laufen nur vorwärts, auch bei vielen offenen Marken.
            let mut schluesse = zeile.match_indices(schliesser).peekable();
            for (start, _) in zeile.match_indices(oeffner) {
                let inhalt_start = start + oeffner.len();
                while schluesse
                    .peek()
                    .is_some_and(|&(ende, _)| ende < inhalt_start)
                {
                    schluesse.next();
                }
                let Some(&(ende, _)) = schluesse.peek() else {
                    break;
                };
                let inhalt = &zeile[inhalt_start..ende];
                let gueltig = if oeffner == "{{S." {
                    let seitenzahl = inhalt.trim_start();
                    let ziffern = seitenzahl.bytes().take_while(u8::is_ascii_digit).count();
                    ziffern > 0 && seitenzahl[ziffern..].chars().all(char::is_whitespace)
                } else {
                    let grenze = if oeffner == "<" { 40 } else { 80 };
                    !inhalt.is_empty() && inhalt.chars().take(grenze + 1).count() <= grenze
                };
                if gueltig {
                    bereiche.push((zeilen_start + start, zeilen_start + ende + schliesser.len()));
                }
            }
            zeilen_start += zeile.len();
        }
    }
    // Zeitanker: MM:SS oder HH:MM:SS, optional mit .mmm bzw. ,mmm.
    let bytes = text.as_bytes();
    let mut position = 0;
    while position < bytes.len() {
        let start = position;
        if !bytes[position].is_ascii_digit()
            || (position > 0 && bytes[position - 1].is_ascii_digit())
        {
            position += 1;
            continue;
        }
        while position < bytes.len() && bytes[position].is_ascii_digit() {
            position += 1;
        }
        if position - start != 2 {
            continue;
        }
        let mut teile = 0;
        while teile < 2
            && bytes.get(position) == Some(&b':')
            && bytes.get(position + 1).is_some_and(u8::is_ascii_digit)
            && bytes.get(position + 2).is_some_and(u8::is_ascii_digit)
            && !bytes.get(position + 3).is_some_and(u8::is_ascii_digit)
        {
            position += 3;
            teile += 1;
        }
        if teile > 0 {
            if matches!(bytes.get(position), Some(b'.' | b','))
                && (1..=3).all(|i| bytes.get(position + i).is_some_and(u8::is_ascii_digit))
                && !bytes.get(position + 4).is_some_and(u8::is_ascii_digit)
            {
                position += 4;
            }
            bereiche.push((start, position));
        }
    }
    bereiche
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::aufbereitung::{AenderungArt, Dokument, DokumentArt};

    pub(crate) fn fassung(text: &str) -> Aufbereitung {
        let mut fassung = Aufbereitung::neu(
            Dokument {
                art: DokumentArt::Seite,
                id: "seite".into(),
                fassung_id: None,
            },
            text,
            100,
        );
        fassung.id = "00000000000000000000000001".into();
        fassung
    }

    pub(crate) fn aenderung(start: usize, alt: &str, neu: &str) -> Aenderung {
        Aenderung {
            id: format!("a{start}"),
            art: AenderungArt::Sprache,
            bereich: [start, start + alt.len()],
            alt: alt.into(),
            neu: neu.into(),
            grund_code: "test".into(),
            zaehler: None,
            gruppe: None,
            status: AenderungStatus::Angenommen,
        }
    }

    #[test]
    fn unveraendert_ist_idempotent() {
        let text = "Schon aufbereitet: schön.\n";
        let einmal = ergebnis_text(text, &[]).unwrap();
        assert_eq!(ergebnis_text(&einmal, &[]).unwrap(), text);
    }

    #[test]
    fn teilannahmen_alle_permutationen() {
        let basis = "a b c";
        let vorschlaege = [
            aenderung(0, "a", "A"),
            aenderung(2, "b", "B"),
            aenderung(4, "c", "C"),
        ];
        for reihenfolge in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut a = vorschlaege.clone();
            for vorschlag in &mut a {
                vorschlag.status = AenderungStatus::Offen;
            }
            for index in reihenfolge {
                a[index].status = AenderungStatus::Angenommen;
                let erwartet = a
                    .iter()
                    .enumerate()
                    .fold(basis.to_string(), |mut text, (i, a)| {
                        if a.status == AenderungStatus::Angenommen {
                            text.replace_range(i * 2..i * 2 + 1, &a.neu);
                        }
                        text
                    });
                let mut umgeordnet = a.to_vec();
                umgeordnet.reverse();
                assert_eq!(ergebnis_text(basis, &umgeordnet).unwrap(), erwartet);
            }
        }
    }

    #[test]
    fn ueberlappung_auch_bei_offenen_vorschlaegen() {
        let mut a = aenderung(1, "bc", "x");
        a.status = AenderungStatus::Offen;
        assert_eq!(
            ergebnis_text("abcd", &[aenderung(0, "ab", "y"), a]),
            Err(ErgebnisFehler::Ueberlappung)
        );
    }

    #[test]
    fn mehrbyte_grenze_abgelehnt() {
        let mut a = aenderung(0, "", "x");
        a.bereich = [1, 2];
        assert_eq!(ergebnis_text("ä", &[a]), Err(ErgebnisFehler::Bereich));
    }

    #[test]
    fn ungueltige_bereiche() {
        for bereich in [[3, 4], [2, 1]] {
            let mut a = aenderung(0, "", "x");
            a.bereich = bereich;
            assert_eq!(ergebnis_text("ab", &[a]), Err(ErgebnisFehler::Bereich));
        }
    }

    fn marke_pruefen(marke: &str) {
        let text = format!("x{marke}y");
        assert_eq!(
            ergebnis_text(&text, &[aenderung(1, marke, "")]),
            Err(ErgebnisFehler::Geschuetzt)
        );
        assert_eq!(
            ergebnis_text(&text, &[aenderung(2, "", "z")]),
            Err(ErgebnisFehler::Geschuetzt)
        );
        assert_eq!(
            ergebnis_text(
                &text,
                &[aenderung(0, "x", "X"), aenderung(1 + marke.len(), "y", "Y"),],
            )
            .unwrap(),
            format!("X{marke}Y")
        );
        assert!(ergebnis_text(&text, &[aenderung(1, "", "z")]).is_ok());
        assert!(ergebnis_text(&text, &[aenderung(1 + marke.len(), "", "z")]).is_ok());
    }

    #[test]
    fn seitenmarke_geschuetzt() {
        marke_pruefen("{{S. 12}}");
    }

    #[test]
    fn sprecher_geschuetzt() {
        marke_pruefen("<Sprecher>");
    }

    #[test]
    fn tags_geschuetzt() {
        marke_pruefen("[Tags]");
    }

    #[test]
    fn zeitanker_geschuetzt() {
        for marke in ["00:12", "01:02:03", "01:02:03.456", "01:02:03,456"] {
            marke_pruefen(marke);
        }
    }

    #[test]
    fn alt_muss_passen() {
        assert_eq!(
            ergebnis_text("a", &[aenderung(0, "b", "c")]),
            Err(ErgebnisFehler::Alt)
        );
    }

    #[test]
    fn gruppe_nur_gemeinsam() {
        let mut a = [aenderung(0, "a", "A"), aenderung(1, "b", "B")];
        for vorschlag in &mut a {
            vorschlag.gruppe = Some("gruppe".into());
        }
        for status in [AenderungStatus::Offen, AenderungStatus::Abgelehnt] {
            a[1].status = status;
            assert_eq!(ergebnis_text("ab", &a), Err(ErgebnisFehler::Gruppe));
            a[0].status = status;
            assert_eq!(ergebnis_text("ab", &a).unwrap(), "ab");
            a[0].status = AenderungStatus::Angenommen;
        }
        a[1].status = AenderungStatus::Angenommen;
        assert_eq!(ergebnis_text("ab", &a).unwrap(), "AB");
    }

    #[test]
    fn einfuegungen_am_selben_punkt_abgelehnt() {
        assert_eq!(
            ergebnis_text("a", &[aenderung(0, "", "x"), aenderung(0, "", "y")]),
            Err(ErgebnisFehler::Ueberlappung)
        );
    }

    #[test]
    fn benachbarte_bereiche_und_einfuegung_erlaubt() {
        assert_eq!(
            ergebnis_text(
                "ab",
                &[
                    aenderung(0, "a", "A"),
                    aenderung(1, "b", "B"),
                    aenderung(2, "", "!"),
                ],
            )
            .unwrap(),
            "AB!"
        );
    }

    #[test]
    fn konflikt_gleicher_laenge_wendet_nichts_an() {
        let mut a = fassung("ab");
        a.aenderungen.push(aenderung(0, "a", "A"));
        assert!(!konflikt(&a.basis, "ab"));
        assert!(konflikt(&a.basis, "ba"));
        assert!(konflikt(&a.basis, "abc"));
        assert_eq!(
            uebernehmen(&mut a, "ba", 200),
            Err(ErgebnisFehler::Konflikt)
        );
        assert_eq!(a.status, Status::Veraltet);
        assert_eq!(a.geaendert, 200);
        assert_eq!(a.aenderungen[0].status, AenderungStatus::Angenommen);
    }

    #[test]
    fn uebernehmen_aktiviert_nur_gueltiges_ergebnis() {
        let mut a = fassung("ab");
        a.aenderungen.push(aenderung(0, "a", "A"));
        assert_eq!(uebernehmen(&mut a, "ab", 200).unwrap(), "Ab");
        assert_eq!(a.status, Status::Aktiv);
        a.status = Status::Entwurf;
        a.aenderungen[0].alt = "x".into();
        assert!(uebernehmen(&mut a, "ab", 300).is_err());
        assert_eq!(a.status, Status::Entwurf);
    }

    #[test]
    fn uebernehmen_nur_aus_entwurf() {
        let mut a = fassung("ab");
        assert_eq!(
            uebernehmen(&mut a, "ba", 200),
            Err(ErgebnisFehler::Konflikt)
        );
        for status in [
            Status::Veraltet,
            Status::Verworfen,
            Status::Abgeloest,
            Status::Aktiv,
            Status::Laeuft,
            Status::Abgebrochen,
        ] {
            a.status = status;
            let vorher = a.clone();
            assert_eq!(uebernehmen(&mut a, "ab", 300), Err(ErgebnisFehler::Status));
            assert_eq!(a, vorher);
            assert_eq!(uebernehmen(&mut a, "ba", 400), Err(ErgebnisFehler::Status));
            assert_eq!(a, vorher);
        }
    }

    #[test]
    fn rueckgaengig_bis_original_erhaelt_kette() {
        let original = "a";
        let mut erste = fassung(original);
        erste.aenderungen.push(aenderung(0, "a", "A"));
        let erster_text = ergebnis_text(original, &erste.aenderungen).unwrap();
        let mut zweite = fassung("A");
        zweite.id = "00000000000000000000000002".into();
        zweite.vorgaenger_aktiv = Some(erste.id.clone());
        zweite.aenderungen.push(aenderung(0, "A", "B"));
        assert_eq!(
            ergebnis_text(&erster_text, &zweite.aenderungen).unwrap(),
            "B"
        );
        let kette = vec![erste.clone(), zweite.clone()];
        let aktiv = rueckgaengig(&kette, &zweite.id).unwrap().unwrap();
        assert_eq!(aktiv, erste.id);
        let wiederhergestellt = ergebnis_text(original, &kette[0].aenderungen).unwrap();
        assert_eq!(wiederhergestellt, "A");
        assert_eq!(rueckgaengig(&kette, &aktiv), Some(None));
        assert_eq!(original, "a");
        assert_eq!(rueckgaengig(&kette, "unbekannt"), None);
        assert_eq!(kette, vec![erste, zweite]);
    }

    #[test]
    fn aufbewahrung_nur_verworfen_und_veraltet() {
        let mut a = fassung("a");
        let tag = 24 * 60 * 60 * 1000;
        for status in [Status::Verworfen, Status::Veraltet] {
            a.status = status;
            assert!(!abgelaufen(&a, a.geaendert + 29 * tag));
            assert!(abgelaufen(&a, a.geaendert + 30 * tag));
            assert!(abgelaufen(&a, a.geaendert + 31 * tag));
            assert!(!abgelaufen(&a, 0));
        }
        for status in [
            Status::Aktiv,
            Status::Entwurf,
            Status::Laeuft,
            Status::Abgeloest,
            Status::Abgebrochen,
        ] {
            a.status = status;
            assert!(!abgelaufen(&a, u64::MAX));
        }
    }

    #[test]
    fn strukturvergleich_erkennt_alle_vier_arten() {
        let vorher = "# Alt\n\nAbsatz\n\n{{S. 1}}\n\n| a |\n| b |";
        let nachher = "## Neu\n\nAbsatz eins\n\nAbsatz zwei\n\n{{S. 1}}\n\n| c |";
        let vergleich = struktur_vergleich(vorher, nachher);
        assert_eq!(vergleich.vorher.ueberschriften, vec!["# Alt"]);
        assert_eq!(vergleich.nachher.ueberschriften, vec!["## Neu"]);
        assert_eq!(vergleich.vorher.seitenmarken, vec!["{{S. 1}}"]);
        assert_eq!(
            vergleich.nachher.seitenmarken,
            vergleich.vorher.seitenmarken
        );
        assert_eq!(vergleich.vorher.absaetze, vec!["Absatz", "{{S. 1}}"]);
        assert_eq!(vergleich.nachher.absaetze.len(), 3);
        assert_eq!(vergleich.vorher.tabellen, vec!["| a |\n| b |"]);
        assert_eq!(vergleich.nachher.tabellen, vec!["| c |"]);
    }

    #[test]
    fn strukturvergleich_leer_und_identisch() {
        let leer = struktur_vergleich("", "");
        assert_eq!(leer.vorher, leer.nachher);
        assert!(leer.vorher.absaetze.is_empty());
        let gleich = struktur_vergleich("Text\nFortsetzung", "Text\nFortsetzung");
        assert_eq!(gleich.vorher, gleich.nachher);
        assert_eq!(gleich.vorher.absaetze, vec!["Text\nFortsetzung"]);
    }
}
