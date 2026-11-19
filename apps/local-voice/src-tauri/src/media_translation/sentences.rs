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
    ausgegeben: Vec<(u64, u64, String)>,
    satz_index: u64,
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
                        true,
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
                let erster_satz = ausgabe.len();
                self.stueck(
                    spur_id,
                    (segment.start_ms, segment.end_ms),
                    &segment.text,
                    guete,
                    false,
                    &mut ausgabe,
                );
                // Mehrere Sätze teilen ohne Wortzeiten dieselbe unbelegte Zeitspanne.
                let satzanzahl =
                    ausgabe.len() - erster_satz + usize::from(!self.rest.trim().is_empty());
                if satzanzahl > 1 {
                    for satz in &mut ausgabe[erster_satz..] {
                        satz.guete = ZeitGuete::Grob;
                        satz.synchron_erlaubt = false;
                    }
                    if !self.rest.trim().is_empty() {
                        self.guete = Some(ZeitGuete::Grob);
                    }
                }
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
        mit_wortzeiten: bool,
        ausgabe: &mut Vec<Satz>,
    ) {
        let (start_ms, end_ms) = zeiten;
        if end_ms < start_ms || !self.gesehen.insert((start_ms, end_ms, text.to_owned())) {
            return;
        }
        // Ein neuer Segmentzuschnitt darf bereits ausgegebene Sätze nicht erneut bilden.
        if self.bereits_ausgegeben(start_ms, end_ms, text) {
            return;
        }
        // Ein neu zugeschnittener Anfang des offenen Rests ist bereits enthalten.
        if self.start_ms <= start_ms && end_ms <= self.end_ms && !text.trim().is_empty() {
            if let Some(rest) = self.rest.trim().strip_prefix(text.trim()) {
                if !rest.starts_with(char::is_alphanumeric) {
                    return;
                }
            }
        }
        // Auch angeschnittene Segmente können einen bereits gesehenen Textanfang enthalten.
        // Gleicher Text ohne zeitliche Überlappung bleibt eine echte Wiederholung.
        let ueberlappung = if start_ms < self.verlauf_bis_ms {
            textueberlappung(&self.verlauf, text)
        } else {
            0
        };
        // Ohne Wortzeiten belegt die gekürzte Passage keine neue Satzgrenze.
        let guete = if ueberlappung > 0 && !mit_wortzeiten {
            ZeitGuete::Grob
        } else {
            guete
        };
        let text = &text[ueberlappung..];
        self.verlauf_bis_ms = self.verlauf_bis_ms.max(end_ms);
        if text.is_empty() {
            return;
        }
        // Satzzeichen schließen auch nach einer gekürzten Überlappung direkt an.
        let satzzeichenanschluss = text.starts_with(|zeichen| {
            matches!(
                zeichen,
                '.' | ','
                    | '!'
                    | '?'
                    | ';'
                    | ':'
                    | '…'
                    | '"'
                    | '\''
                    | '”'
                    | '’'
                    | '“'
                    | '‘'
                    | '»'
                    | '›'
                    | '«'
                    | '‹'
            )
        });
        if !self.verlauf.is_empty()
            && !self.verlauf.ends_with(char::is_whitespace)
            && !text.starts_with(char::is_whitespace)
            && !satzzeichenanschluss
        {
            self.verlauf.push(' ');
        }
        self.verlauf.push_str(text);
        if !self.rest.is_empty()
            && !self.rest.ends_with(char::is_whitespace)
            && !text.starts_with(char::is_whitespace)
            && !satzzeichenanschluss
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
            // Zusammenhängende Satzschlusszeichen gehören noch zum selben Satz.
            if zeichen == '\n'
                || ((matches!(zeichen, '!' | '?') || (zeichen == '.' && !dezimal && !abkuerzung))
                    && !danach.is_some_and(|c| matches!(c, '.' | '!' | '?')))
            {
                self.abschliessen(spur_id, ausgabe);
            }
        }
    }

    fn bereits_ausgegeben(&self, start_ms: u64, end_ms: u64, text: &str) -> bool {
        let gesucht = normalisiert(text);
        if gesucht.is_empty() {
            return false;
        }
        let mut bereiche: Vec<_> = self
            .ausgegeben
            .iter()
            .filter(|(start, ende, _)| *start < end_ms && start_ms < *ende)
            .collect();
        bereiche.sort_by_key(|(start, ende, _)| (*start, *ende));
        let mut abgedeckt_bis = start_ms;
        let mut verlauf = String::new();
        for (start, ende, text) in bereiche {
            if *start > abgedeckt_bis {
                break;
            }
            abgedeckt_bis = abgedeckt_bis.max(*ende);
            verlauf.push(' ');
            verlauf.push_str(text);
        }
        verlauf.push(' ');
        abgedeckt_bis >= end_ms && verlauf.contains(&format!(" {gesucht} "))
    }

    fn abschliessen(&mut self, spur_id: &str, ausgabe: &mut Vec<Satz>) {
        let text = self.rest.trim().to_owned();
        if !text.is_empty() {
            let guete = self.guete.unwrap_or(ZeitGuete::Grob);
            self.ausgegeben
                .push((self.start_ms, self.end_ms, normalisiert(&text)));
            ausgabe.push(Satz {
                id: format!(
                    "{}:{}:{:016x}:{}",
                    spur_id,
                    self.start_ms,
                    hash(text.as_bytes()),
                    self.satz_index
                ),
                start_ms: self.start_ms,
                end_ms: self.end_ms,
                text,
                guete,
                synchron_erlaubt: guete != ZeitGuete::Grob,
            });
            self.satz_index = self.satz_index.saturating_add(1);
        }
        self.rest.clear();
        self.end_ms = 0;
        self.guete = None;
    }
}

fn normalisiert(text: &str) -> String {
    text.split(|zeichen: char| !zeichen.is_alphanumeric())
        .filter(|wort| !wort.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
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
    fn satzzeichenfragmente_schliessen_ohne_leerzeichen_an() {
        for zeichen in [
            '.', ',', '!', '?', ';', ':', '…', '"', '\'', '”', '’', '“', '‘', '»', '›', '«', '‹',
        ] {
            let mut bildner = Satzbildner::default();
            bildner.fenster("a", vec![segment(0, "Hallo")], ZeitGuete::Satz, false);
            let saetze = bildner.fenster(
                "a",
                vec![segment(100, &zeichen.to_string())],
                ZeitGuete::Satz,
                true,
            );
            // Gegenprobe: Bedingungsloses Trennen fügt vor jedem Satzzeichen ein Leerzeichen ein.
            assert_eq!(saetze.len(), 1);
            assert_eq!(saetze[0].text, format!("Hallo{zeichen}"));
            assert_eq!((saetze[0].start_ms, saetze[0].end_ms), (0, 200));
        }
    }

    #[test]
    fn mehrere_satzschlusszeichen_bleiben_ein_satz() {
        let saetze =
            Satzbildner::default().fenster("a", vec![segment(0, "Hallo?!")], ZeitGuete::Satz, true);
        // Gegenprobe: Sofortiger Abschluss am Fragezeichen gibt das Ausrufezeichen separat aus.
        assert_eq!(saetze.len(), 1);
        assert_eq!(saetze[0].text, "Hallo?!");
        assert_eq!(saetze[0].guete, ZeitGuete::Satz);
        assert!(saetze[0].synchron_erlaubt);
    }

    #[test]
    fn geaenderter_segmentzuschnitt_verdoppelt_offenen_rest_nicht() {
        let mut bildner = Satzbildner::default();
        let mut ganz = segment(0, "Hallo Welt");
        ganz.end_ms = 200;
        assert!(bildner
            .fenster("a", vec![ganz], ZeitGuete::Satz, false)
            .is_empty());
        let saetze = bildner.fenster(
            "a",
            vec![segment(0, "Hallo"), segment(100, "Welt.")],
            ZeitGuete::Satz,
            true,
        );
        // Gegenprobe: Der Suffixvergleich allein hängt den offenen Anfang erneut an.
        assert_eq!(saetze.len(), 1);
        assert_eq!(saetze[0].text, "Hallo Welt.");
        assert_eq!((saetze[0].start_ms, saetze[0].end_ms), (0, 200));
    }

    #[test]
    fn gekuerzte_ueberlappung_ohne_wortzeiten_bleibt_grob() {
        let mut bildner = Satzbildner::default();
        bildner.fenster("a", vec![segment(0, "Hallo Welt.")], ZeitGuete::Satz, false);
        let mut weiter = segment(50, "Welt. Weiter.");
        weiter.end_ms = 200;
        let saetze = bildner.fenster("a", vec![weiter], ZeitGuete::Satz, true);
        assert_eq!(saetze.len(), 1);
        assert_eq!(saetze[0].text, "Weiter.");
        // Gegenprobe: Nach der Kürzung bleibt nur ein Satz mit unbelegter Startzeit.
        assert_eq!(saetze[0].guete, ZeitGuete::Grob);
        assert!(!saetze[0].synchron_erlaubt);
    }

    #[test]
    fn geaenderter_segmentzuschnitt_wird_nicht_erneut_ausgegeben() {
        let mut bildner = Satzbildner::default();
        let mut ganz = segment(0, "Hallo Welt.");
        ganz.end_ms = 200;
        let zuerst = bildner.fenster("a", vec![ganz], ZeitGuete::Satz, false);
        assert_eq!(zuerst.len(), 1);
        // Gegenprobe: Tupel- und Suffixvergleich erkennen den neuen Zuschnitt nicht.
        assert!(bildner
            .fenster(
                "a",
                vec![segment(0, "Hallo"), segment(100, "Welt.")],
                ZeitGuete::Satz,
                true,
            )
            .is_empty());
        // Normalisierung berücksichtigt Großschreibung und Satzzeichen.
        assert!(bildner
            .fenster("a", vec![segment(0, "HALLO!")], ZeitGuete::Satz, true)
            .is_empty());
        // Gegenprobe: Textgleichheit allein würde diese echte Wiederholung verwerfen.
        let wiederholung = bildner.fenster(
            "a",
            vec![segment(200, "Hallo Welt.")],
            ZeitGuete::Satz,
            true,
        );
        assert_eq!(wiederholung.len(), 1);
        assert_ne!(zuerst[0].id, wiederholung[0].id);
    }

    #[test]
    fn mehrsatz_segment_ohne_wortzeiten_ist_grob_und_hat_eindeutige_ids() {
        let mut daten = segment(0, "Ja. Ja.");
        daten.end_ms = 10_000;
        let saetze = Satzbildner::default().fenster("a", vec![daten], ZeitGuete::Satz, true);
        assert_eq!(saetze.len(), 2);
        // Gegenprobe: Ohne Satzindex kollidieren gleicher Text und gleiche Startzeit.
        assert_ne!(saetze[0].id, saetze[1].id);
        for satz in saetze {
            assert_eq!((satz.start_ms, satz.end_ms), (0, 10_000));
            // Gegenprobe: Das Segmentlabel Satz belegt keine einzelnen Satzzeiten.
            assert_eq!(satz.guete, ZeitGuete::Grob);
            assert!(!satz.synchron_erlaubt);
        }
        let einzeln =
            Satzbildner::default().fenster("a", vec![segment(0, "Ja.")], ZeitGuete::Satz, true);
        assert_eq!(einzeln[0].guete, ZeitGuete::Satz);
        assert!(einzeln[0].synchron_erlaubt);
    }

    #[test]
    fn mehrsatz_segment_mit_wortzeiten_behaelt_seine_guete() {
        let mut daten = segment(0, "Ja. Ja.");
        daten.end_ms = 200;
        daten.woerter = Some(vec![
            super::super::Wort {
                start_ms: 0,
                end_ms: 100,
                text: "Ja.".to_owned(),
            },
            super::super::Wort {
                start_ms: 100,
                end_ms: 200,
                text: "Ja.".to_owned(),
            },
        ]);
        let saetze = Satzbildner::default().fenster("a", vec![daten], ZeitGuete::Wort, true);
        assert_eq!(saetze.len(), 2);
        // Gegenprobe: Pauschales Herabsetzen ignoriert belegte einzelne Wortzeiten.
        assert_eq!((saetze[0].start_ms, saetze[0].end_ms), (0, 100));
        assert_eq!((saetze[1].start_ms, saetze[1].end_ms), (100, 200));
        for satz in saetze {
            assert_eq!(satz.guete, ZeitGuete::Wort);
            assert!(satz.synchron_erlaubt);
        }
    }

    #[test]
    fn mehrsatz_segment_mit_offenem_rest_bleibt_grob() {
        let mut bildner = Satzbildner::default();
        let anfang = bildner.fenster("a", vec![segment(0, "Ja. Weiter")], ZeitGuete::Satz, false);
        assert_eq!(anfang.len(), 1);
        assert_eq!(anfang[0].guete, ZeitGuete::Grob);
        let ende = bildner.fenster("a", vec![segment(100, "geht es.")], ZeitGuete::Satz, true);
        // Gegenprobe: Nur fertige Sätze herabzustufen lässt den Rest synchronisierbar.
        assert_eq!(ende[0].text, "Weiter geht es.");
        assert_eq!(ende[0].guete, ZeitGuete::Grob);
        assert!(!ende[0].synchron_erlaubt);
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
