use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{hash, Segment, ZeitGuete};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Satz {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub guete: ZeitGuete,
    pub synchron_erlaubt: bool,
}

/// Nur für quelleneigene, relative Daten vor der Übergabe an den Satzbildner.
/// Der Satzbildner selbst akzeptiert ausschließlich absolute Medienzeiten.
pub fn mit_offset(mut segment: Segment, offset_ms: u64) -> Segment {
    segment.start_ms = segment.start_ms.saturating_add(offset_ms);
    segment.end_ms = segment.end_ms.saturating_add(offset_ms);
    if let Some(woerter) = &mut segment.woerter {
        for wort in woerter {
            wort.start_ms = wort.start_ms.saturating_add(offset_ms);
            wort.end_ms = wort.end_ms.saturating_add(offset_ms);
        }
    }
    segment
}

#[derive(Default)]
pub struct Satzbildner {
    gesehen: BTreeSet<(u64, u64, String)>,
    verlauf: String,
    verlauf_bis_ms: u64,
    rest: String,
    start_ms: u64,
    end_ms: u64,
    guete: Option<ZeitGuete>,
}

impl Satzbildner {
    /// Bei Seek einen neuen Bildner verwenden; Überlappungen derselben Spur bleiben erhalten.
    pub fn fenster(
        &mut self,
        spur_id: &str,
        mut segmente: Vec<Segment>,
        guete: ZeitGuete,
        dateiende: bool,
    ) -> Vec<Satz> {
        segmente.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
        let mut ausgabe = Vec::new();
        for segment in segmente {
            if let Some(mut woerter) = segment.woerter.filter(|w| !w.is_empty()) {
                woerter.sort_by_key(|wort| (wort.start_ms, wort.end_ms));
                for wort in woerter {
                    self.stueck(
                        spur_id,
                        (wort.start_ms, wort.end_ms),
                        &wort.text,
                        guete,
                        &mut ausgabe,
                    );
                }
            } else {
                // Ohne Wortzeiten ist eine feinere zeitliche Zuordnung nicht belegbar.
                let guete = if guete == ZeitGuete::Wort {
                    ZeitGuete::Grob
                } else {
                    guete
                };
                self.stueck(
                    spur_id,
                    (segment.start_ms, segment.end_ms),
                    &segment.text,
                    guete,
                    &mut ausgabe,
                );
            }
        }
        if dateiende {
            self.abschliessen(spur_id, &mut ausgabe);
        }
        ausgabe
    }

    fn stueck(
        &mut self,
        spur_id: &str,
        zeiten: (u64, u64),
        text: &str,
        guete: ZeitGuete,
        ausgabe: &mut Vec<Satz>,
    ) {
        let (start_ms, end_ms) = zeiten;
        if end_ms < start_ms || !self.gesehen.insert((start_ms, end_ms, text.to_owned())) {
            return;
        }
        // Auch angeschnittene Segmente können einen bereits gesehenen Textanfang enthalten.
        // Gleicher Text ohne zeitliche Überlappung bleibt eine echte Wiederholung.
        let ueberlappung = if start_ms < self.verlauf_bis_ms {
            textueberlappung(&self.verlauf, text)
        } else {
            0
        };
        let text = &text[ueberlappung..];
        self.verlauf_bis_ms = self.verlauf_bis_ms.max(end_ms);
        if text.is_empty() {
            return;
        }
        if !self.verlauf.is_empty()
            && !self.verlauf.ends_with(char::is_whitespace)
            && !text.starts_with(char::is_whitespace)
        {
            self.verlauf.push(' ');
        }
        self.verlauf.push_str(text);
        if !self.rest.is_empty()
            && !self.rest.ends_with(char::is_whitespace)
            && !text.starts_with(char::is_whitespace)
        {
            self.rest.push(' ');
        }
        for (index, zeichen) in text.char_indices() {
            if self.rest.trim().is_empty() {
                self.start_ms = start_ms;
            }
            self.end_ms = self.end_ms.max(end_ms);
            self.guete = Some(schlechter(self.guete, guete));
            self.rest.push(zeichen);
            let davor = text[..index].chars().next_back();
            let danach = text[index + zeichen.len_utf8()..].chars().next();
            let dezimal = davor.is_some_and(|c| c.is_ascii_digit())
                && danach.is_some_and(|c| c.is_ascii_digit());
            let wort = self.rest.split_whitespace().next_back().unwrap_or("");
            let abkuerzung = matches!(
                wort.to_lowercase().as_str(),
                "z." | "b." | "d." | "h." | "dr." | "prof." | "bzw." | "usw."
            );
            if zeichen == '\n'
                || matches!(zeichen, '!' | '?')
                || (zeichen == '.' && !dezimal && !abkuerzung)
            {
                self.abschliessen(spur_id, ausgabe);
            }
        }
    }

    fn abschliessen(&mut self, spur_id: &str, ausgabe: &mut Vec<Satz>) {
        let text = self.rest.trim().to_owned();
        if !text.is_empty() {
            let guete = self.guete.unwrap_or(ZeitGuete::Grob);
            ausgabe.push(Satz {
                id: format!(
                    "{}:{}:{:016x}",
                    spur_id,
                    self.start_ms,
                    hash(text.as_bytes())
                ),
                start_ms: self.start_ms,
                end_ms: self.end_ms,
                text,
                guete,
                synchron_erlaubt: guete != ZeitGuete::Grob,
            });
        }
        self.rest.clear();
        self.end_ms = 0;
        self.guete = None;
    }
}

fn textueberlappung(verlauf: &str, text: &str) -> usize {
    for anzahl in std::iter::once(text.len()).chain(text.char_indices().rev().map(|(i, _)| i)) {
        if anzahl == 0 || !verlauf.ends_with(&text[..anzahl]) {
            continue;
        }
        let davor = verlauf[..verlauf.len() - anzahl].chars().next_back();
        let danach = text[anzahl..].chars().next();
        if !davor.is_some_and(char::is_alphanumeric) && !danach.is_some_and(char::is_alphanumeric) {
            return anzahl;
        }
    }
    0
}

fn schlechter(bisher: Option<ZeitGuete>, neu: ZeitGuete) -> ZeitGuete {
    match (bisher, neu) {
        (Some(ZeitGuete::Grob), _) | (_, ZeitGuete::Grob) => ZeitGuete::Grob,
        (Some(ZeitGuete::Satz), _) | (_, ZeitGuete::Satz) => ZeitGuete::Satz,
        _ => ZeitGuete::Wort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_translation::{MediaSegmentSource, QuellenFehler, Uebersetzer};

    fn segment(start_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 100,
            text: text.to_owned(),
            woerter: None,
        }
    }

    #[test]
    fn ueberlappung_rest_und_stabile_ids() {
        // Gegenprobe: Ohne Rest oder Deduplikation fehlt der Anfang oder er steht doppelt da.
        let mut bildner = Satzbildner::default();
        let anfang = segment(0, "Das ist");
        assert!(bildner
            .fenster("a", vec![anfang.clone()], ZeitGuete::Satz, false)
            .is_empty());
        let ende = segment(100, "ein Satz.");
        let saetze = bildner.fenster(
            "a",
            vec![anfang.clone(), ende.clone()],
            ZeitGuete::Satz,
            false,
        );
        assert_eq!(saetze[0].text, "Das ist ein Satz.");
        assert_eq!((saetze[0].start_ms, saetze[0].end_ms), (0, 200));
        let erneut =
            Satzbildner::default().fenster("a", vec![anfang, ende], ZeitGuete::Satz, false);
        assert_eq!(saetze, erneut);
        assert!(bildner
            .fenster("a", vec![segment(100, "ein Satz.")], ZeitGuete::Satz, true)
            .is_empty());
    }

    #[test]
    fn angeschnittene_ueberlappung_und_echte_wiederholung() {
        let mut bildner = Satzbildner::default();
        bildner.fenster("a", vec![segment(0, "Wir sind")], ZeitGuete::Satz, false);
        let saetze = bildner.fenster("a", vec![segment(50, "sind hier.")], ZeitGuete::Satz, false);
        // Gegenprobe: Identität nur ganzer Segmente würde hier das Wort „sind“ verdoppeln.
        assert_eq!(saetze[0].text, "Wir sind hier.");
        let wiederholung = bildner.fenster(
            "a",
            vec![segment(200, "Wir sind hier.")],
            ZeitGuete::Satz,
            false,
        );
        // Gegenprobe: Reine Textdublettenprüfung würde eine echte Wiederholung verschlucken.
        assert_eq!(wiederholung[0].text, "Wir sind hier.");
        assert_ne!(saetze[0].id, wiederholung[0].id);
    }

    #[test]
    fn grenzen_guete_und_offset() {
        // Gegenprobe: Blindes Teilen an Punkten zerlegt Abkürzungen und Dezimalzahlen.
        let daten = mit_offset(segment(10, "z. B. 3.5 Euro!\nRest"), 1000);
        let saetze = Satzbildner::default().fenster("a", vec![daten], ZeitGuete::Grob, true);
        assert_eq!(saetze.len(), 2);
        assert_eq!(saetze[0].text, "z. B. 3.5 Euro!");
        // Gegenprobe: Erneute Offsetaddition oder fehlende Grobsperre verletzt diese Werte.
        assert_eq!(saetze[0].start_ms, 1010);
        assert_eq!(saetze[0].end_ms, 1110);
        assert!(!saetze[0].synchron_erlaubt);
    }

    #[test]
    fn wortoffset_und_fehlende_wortzeiten() {
        let mut daten = segment(10, "Hallo!");
        daten.woerter = Some(vec![super::super::Wort {
            start_ms: 20,
            end_ms: 50,
            text: "Hallo!".to_owned(),
        }]);
        let saetze = Satzbildner::default().fenster(
            "a",
            vec![mit_offset(daten, 1000)],
            ZeitGuete::Wort,
            true,
        );
        // Gegenprobe: Ohne Wortoffset oder mit doppelter Addition stimmen die Zeiten nicht.
        assert_eq!((saetze[0].start_ms, saetze[0].end_ms), (1020, 1050));
        assert!(saetze[0].synchron_erlaubt);
        let ohne_woerter =
            Satzbildner::default().fenster("a", vec![segment(10, "Hallo!")], ZeitGuete::Wort, true);
        // Gegenprobe: Ein bloßes Wortgüte-Label ersetzt keine tatsächlichen Wortzeitmarken.
        assert!(!ohne_woerter[0].synchron_erlaubt);
    }

    #[test]
    fn wortueberlappung_und_quellenvertrag() {
        struct Quelle;
        impl MediaSegmentSource for Quelle {
            fn segmente(
                &mut self,
                von_ms: u64,
                bis_ms: u64,
            ) -> Result<(Vec<Segment>, ZeitGuete), QuellenFehler> {
                let woerter = vec![super::super::Wort {
                    start_ms: 100,
                    end_ms: 200,
                    text: "Hallo!".to_owned(),
                }];
                Ok((
                    vec![Segment {
                        start_ms: von_ms,
                        end_ms: bis_ms,
                        text: "Hallo!".to_owned(),
                        woerter: Some(woerter),
                    }],
                    ZeitGuete::Wort,
                ))
            }
        }
        struct Lokal;
        impl Uebersetzer for Lokal {
            fn uebersetzen(
                &mut self,
                satz: &str,
            ) -> Result<String, super::super::UebersetzungsFehler> {
                Ok(satz.to_owned())
            }
        }
        let mut bildner = Satzbildner::default();
        let (daten, guete) = Quelle.segmente(0, 300).expect("Testquelle");
        let saetze = bildner.fenster("a", daten, guete, false);
        assert!(saetze[0].synchron_erlaubt);
        assert_eq!(saetze[0].start_ms, 100);
        assert_eq!(Lokal.uebersetzen(&saetze[0].text), Ok("Hallo!".to_owned()));
        // Gegenprobe: Segmentweise statt wortweiser Deduplikation liefert ein Duplikat.
        let (daten, guete) = Quelle.segmente(50, 400).expect("Testquelle");
        assert!(bildner.fenster("a", daten, guete, true).is_empty());
    }
}
