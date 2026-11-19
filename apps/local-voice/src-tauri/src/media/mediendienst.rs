//! Fensterweise Medienerkennung mit absoluten Zeiten und Ueberlappungsschutz.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::zeitachse::{MedienQuelle, Segment, Wort, ZeitGuete};
use crate::media_translation::{MediaSegmentSource, QuellenFehler};

pub trait Dekoder {
    /// Liefert 16-kHz-Mono-Samples des angeforderten Fensters.
    fn fenster(
        &mut self,
        pfad: &Path,
        von_ms: u64,
        bis_ms: u64,
        abbruch: &AtomicBool,
    ) -> Result<Vec<f32>, DienstFehler>;
}

pub trait Erkenner {
    fn erkenne(&mut self, samples: Vec<f32>) -> Result<Vec<RohSegment>, DienstFehler>;
}

/// Zeiten des Erkenners sind relativ zum Fensteranfang.
pub struct RohSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub woerter: Option<Vec<RohWort>>,
}

pub struct RohWort {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DienstFehler {
    Abgebrochen,
    Beschaedigt,
    DekodierFehler,
    ErkennerFehler,
    KeinModell,
}

impl std::fmt::Display for DienstFehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nur feste Fehlerarten, keine Pfade oder erkannten Inhalte.
        f.write_str(match self {
            Self::Abgebrochen => "abgebrochen",
            Self::Beschaedigt => "datei_beschaedigt",
            Self::DekodierFehler => "dekodierfehler",
            Self::ErkennerFehler => "erkennerfehler",
            Self::KeinModell => "kein_modell",
        })
    }
}

impl std::error::Error for DienstFehler {}

impl From<DienstFehler> for QuellenFehler {
    fn from(fehler: DienstFehler) -> Self {
        Self(fehler.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeerGrund {
    KeineTonspur,
    Dateiende,
}

pub struct Mediendienst<D, E> {
    quelle: MedienQuelle,
    pfad: PathBuf,
    dekoder: D,
    erkenner: E,
    abbruch: Arc<AtomicBool>,
    grund: Option<LeerGrund>,
    geliefert_bis_ms: u64,
}

impl<D, E> Mediendienst<D, E> {
    pub fn neu(
        quelle: MedienQuelle,
        pfad: PathBuf,
        dekoder: D,
        erkenner: E,
        abbruch: Arc<AtomicBool>,
    ) -> Self {
        Self {
            quelle,
            pfad,
            dekoder,
            erkenner,
            abbruch,
            grund: None,
            geliefert_bis_ms: 0,
        }
    }

    pub fn grund(&self) -> Option<LeerGrund> {
        self.grund
    }

    fn pruefe_abbruch(&self) -> Result<(), QuellenFehler> {
        if self.abbruch.load(Ordering::Acquire) {
            Err(DienstFehler::Abgebrochen.into())
        } else {
            Ok(())
        }
    }
}

impl<D: Dekoder, E: Erkenner> MediaSegmentSource for Mediendienst<D, E> {
    fn segmente(
        &mut self,
        von_ms: u64,
        bis_ms: u64,
    ) -> Result<(Vec<Segment>, ZeitGuete), QuellenFehler> {
        self.grund = None;
        let bis_ms = bis_ms.min(self.quelle.dauer_ms);
        if von_ms >= self.quelle.dauer_ms || bis_ms <= von_ms {
            self.grund = Some(LeerGrund::Dateiende);
            return Ok((vec![], ZeitGuete::Grob));
        }
        if !self.quelle.hat_ton {
            self.grund = Some(LeerGrund::KeineTonspur);
            return Ok((vec![], ZeitGuete::Grob));
        }
        self.pruefe_abbruch()?;
        let samples = self
            .dekoder
            .fenster(&self.pfad, von_ms, bis_ms, &self.abbruch);
        self.pruefe_abbruch()?;
        let roh = self.erkenner.erkenne(samples?);
        self.pruefe_abbruch()?;
        let roh = roh?;

        // Ein Sprung um mehr als ein Fenster beginnt eine neue Lieferfolge.
        // Erst nach erfolgreicher Erkennung wird der Lieferstand veraendert.
        let fenster_ms = bis_ms - von_ms;
        let grenze = if von_ms < self.geliefert_bis_ms.saturating_sub(fenster_ms) {
            von_ms
        } else {
            self.geliefert_bis_ms
        };
        let absolut = |zeit: u64| von_ms.saturating_add(zeit).min(bis_ms);
        let mut geliefert_bis_ms = grenze;
        let mut segmente = Vec::new();
        for roh in roh {
            if roh.text.trim().is_empty() {
                continue;
            }
            let mut segment = Segment {
                start_ms: absolut(roh.start_ms),
                end_ms: absolut(roh.end_ms).max(absolut(roh.start_ms)),
                text: roh.text.trim().to_string(),
                woerter: None,
            };
            let woerter: Vec<Wort> = roh
                .woerter
                .unwrap_or_default()
                .into_iter()
                .filter(|wort| !wort.text.trim().is_empty())
                .map(|wort| Wort {
                    start_ms: absolut(wort.start_ms),
                    end_ms: absolut(wort.end_ms).max(absolut(wort.start_ms)),
                    text: wort.text.trim().to_string(),
                })
                .collect();
            if woerter.is_empty() {
                if segment.end_ms <= grenze {
                    continue;
                }
                geliefert_bis_ms = geliefert_bis_ms.max(segment.end_ms);
            } else {
                let anzahl = woerter.len();
                let woerter: Vec<Wort> = woerter
                    .into_iter()
                    .filter(|wort| wort.start_ms >= grenze)
                    .collect();
                let Some(erstes) = woerter.first() else {
                    continue;
                };
                if woerter.len() != anzahl {
                    segment.start_ms = erstes.start_ms;
                    segment.text = woerter
                        .iter()
                        .map(|wort| wort.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                segment.end_ms = segment.end_ms.max(segment.start_ms);
                for wort in &woerter {
                    geliefert_bis_ms = geliefert_bis_ms.max(wort.end_ms);
                }
                segment.woerter = Some(woerter);
            }
            segmente.push(segment);
        }
        let guete = if segmente.is_empty() {
            ZeitGuete::Grob
        } else if segmente.iter().all(|segment| segment.woerter.is_some()) {
            ZeitGuete::Wort
        } else if segmente
            .iter()
            .all(|segment| segment.end_ms > segment.start_ms)
            && !segmente
                .iter()
                .all(|segment| segment.start_ms == von_ms && segment.end_ms == bis_ms)
        {
            ZeitGuete::Satz
        } else {
            ZeitGuete::Grob
        };
        self.geliefert_bis_ms = geliefert_bis_ms;
        Ok((segmente, guete))
    }
}

pub struct FfmpegDekoder;

impl Dekoder for FfmpegDekoder {
    fn fenster(
        &mut self,
        pfad: &Path,
        von_ms: u64,
        bis_ms: u64,
        abbruch: &AtomicBool,
    ) -> Result<Vec<f32>, DienstFehler> {
        // TempPath schliesst das Handle vor ffmpeg und loescht auf jedem Rueckweg.
        let tmp = tempfile::Builder::new()
            .prefix("lva-mediendienst-")
            .suffix(".wav")
            .tempfile()
            .map_err(|_| DienstFehler::DekodierFehler)?
            .into_temp_path();
        super::decode_media_range_to_wav_cancellable(
            pfad,
            &tmp,
            16_000,
            Some((von_ms, Some(bis_ms))),
            abbruch,
        )
        .map_err(|fehler| {
            if fehler == super::DECODE_CANCELLED {
                DienstFehler::Abgebrochen
            } else {
                DienstFehler::DekodierFehler
            }
        })?;
        let mut reader = hound::WavReader::open(&tmp).map_err(|_| DienstFehler::Beschaedigt)?;
        let spec = reader.spec();
        if spec.channels != 1 || spec.sample_rate != 16_000 {
            return Err(DienstFehler::Beschaedigt);
        }
        match spec.sample_format {
            hound::SampleFormat::Int => {
                if !(1..=32).contains(&spec.bits_per_sample) {
                    return Err(DienstFehler::Beschaedigt);
                }
                let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|sample| sample as f32 / scale))
                    .collect::<Result<_, _>>()
                    .map_err(|_| DienstFehler::Beschaedigt)
            }
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .map_err(|_| DienstFehler::Beschaedigt),
        }
    }
}

/// Lokale Spracherkennung (nur lokale Modelle, kein Netz) fuer Medienfenster.
pub struct LokalerErkenner {
    app: tauri::AppHandle,
}

impl LokalerErkenner {
    pub fn neu(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl Erkenner for LokalerErkenner {
    fn erkenne(&mut self, samples: Vec<f32>) -> Result<Vec<RohSegment>, DienstFehler> {
        use tauri::Manager;
        // Diktat oder Aufnahme hat Vorrang: das Erkennungsmodell nicht wegnehmen.
        if self
            .app
            .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
            .is_some_and(|a| a.is_recording())
        {
            return Err(DienstFehler::ErkennerFehler);
        }
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        let settings = crate::settings::get_settings(&self.app);
        tm.initiate_meeting_model_load(&settings);
        // Je Fenster eine eigene Freigabe: Medien gehoeren keiner Besprechung.
        let frei = crate::managers::planning::Freigabe::ohne_besprechung(
            crate::managers::planning::OhneBesprechung::Medien,
        );
        let ergebnis = tm.transcribe_segments_detecting(&frei, samples);
        // Danach wieder das Diktatmodell laden.
        tm.initiate_model_load_target(&settings.selected_model);
        let (segmente, _sprache) = ergebnis.map_err(|_| DienstFehler::ErkennerFehler)?;
        Ok(segmente
            .into_iter()
            .map(|s| RohSegment {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text,
                woerter: s.words.map(|ws| {
                    ws.into_iter()
                        .map(|w| RohWort {
                            text: w.text,
                            start_ms: w.start_ms,
                            end_ms: w.end_ms,
                        })
                        .collect()
                }),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct TestDekoder {
        aufrufe: Vec<(u64, u64)>,
        fehler: Option<DienstFehler>,
        abbrechen: bool,
    }

    impl Dekoder for TestDekoder {
        fn fenster(
            &mut self,
            _pfad: &Path,
            von_ms: u64,
            bis_ms: u64,
            abbruch: &AtomicBool,
        ) -> Result<Vec<f32>, DienstFehler> {
            self.aufrufe.push((von_ms, bis_ms));
            if self.abbrechen {
                abbruch.store(true, Ordering::Release);
            }
            match &self.fehler {
                Some(fehler) => Err(fehler.clone()),
                None => Ok(vec![0.25, -0.25]),
            }
        }
    }

    struct TestErkenner {
        aufrufe: usize,
        ergebnisse: VecDeque<Vec<RohSegment>>,
        fehler: Option<DienstFehler>,
        abbruch: Arc<AtomicBool>,
        abbrechen: bool,
    }

    impl Erkenner for TestErkenner {
        fn erkenne(&mut self, samples: Vec<f32>) -> Result<Vec<RohSegment>, DienstFehler> {
            self.aufrufe += 1;
            assert_eq!(samples, vec![0.25, -0.25]);
            if self.abbrechen {
                self.abbruch.store(true, Ordering::Release);
            }
            match &self.fehler {
                Some(fehler) => Err(fehler.clone()),
                None => Ok(self.ergebnisse.pop_front().expect("Ergebnis vorbereitet")),
            }
        }
    }

    fn wort(text: &str, start_ms: u64, end_ms: u64) -> RohWort {
        RohWort {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    fn satz(text: &str, start_ms: u64, end_ms: u64) -> RohSegment {
        RohSegment {
            start_ms,
            end_ms,
            text: text.into(),
            woerter: None,
        }
    }

    fn mit_woertern(woerter: Vec<RohWort>) -> RohSegment {
        RohSegment {
            start_ms: woerter.first().unwrap().start_ms,
            end_ms: woerter.last().unwrap().end_ms,
            text: woerter
                .iter()
                .map(|wort| wort.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            woerter: Some(woerter),
        }
    }

    fn dienst(ergebnisse: Vec<Vec<RohSegment>>) -> Mediendienst<TestDekoder, TestErkenner> {
        let abbruch = Arc::new(AtomicBool::new(false));
        Mediendienst::neu(
            MedienQuelle {
                id: "test".into(),
                dauer_ms: 120_000,
                hat_ton: true,
                hat_video: false,
            },
            PathBuf::from("nicht-gelesen.wav"),
            TestDekoder {
                aufrufe: vec![],
                fehler: None,
                abbrechen: false,
            },
            TestErkenner {
                aufrufe: 0,
                ergebnisse: ergebnisse.into(),
                fehler: None,
                abbruch: Arc::clone(&abbruch),
                abbrechen: false,
            },
            abbruch,
        )
    }

    #[test]
    fn wortzeiten_werden_einmalig_absolut() {
        let mut roh = mit_woertern(vec![wort("Hallo", 100, 400)]);
        roh.end_ms = 2000;
        let mut d = dienst(vec![vec![roh]]);
        assert_eq!(d.grund(), None);
        let (segmente, guete) = d.segmente(60_000, 62_000).unwrap();
        assert_eq!(guete, ZeitGuete::Wort);
        assert_eq!(segmente[0].start_ms, 60_100);
        assert_eq!(segmente[0].end_ms, 62_000);
        assert_eq!(
            segmente[0].woerter,
            Some(vec![Wort {
                start_ms: 60_100,
                end_ms: 60_400,
                text: "Hallo".into(),
            }])
        );
        assert_eq!(d.geliefert_bis_ms, 60_400);
    }

    #[test]
    fn eigene_segmentspannen_ergeben_satzguete() {
        let mut d = dienst(vec![vec![
            satz("Hallo.", 100, 400),
            satz("Welt.", 600, 900),
        ]]);
        let (segmente, guete) = d.segmente(60_000, 62_000).unwrap();
        assert_eq!(guete, ZeitGuete::Satz);
        assert_eq!((segmente[1].start_ms, segmente[1].end_ms), (60_600, 60_900));
        assert!(segmente.iter().all(|segment| segment.woerter.is_none()));
        let mut d = dienst(vec![vec![
            mit_woertern(vec![wort("Hallo", 100, 400)]),
            satz("Welt.", 600, 900),
        ]]);
        assert_eq!(d.segmente(0, 2000).unwrap().1, ZeitGuete::Satz);
    }

    #[test]
    fn identische_fensterspannen_und_unplausible_spannen_sind_grob() {
        for segmente in [
            vec![satz("Hallo", 0, 2000), satz("Welt", 0, 2000)],
            vec![satz("Hallo", 400, 100)],
        ] {
            let mut d = dienst(vec![segmente]);
            assert_eq!(d.segmente(60_000, 62_000).unwrap().1, ZeitGuete::Grob);
        }
    }

    #[test]
    fn keine_tonspur_braucht_keinen_dekoder() {
        let mut d = dienst(vec![]);
        d.quelle.hat_ton = false;
        assert_eq!(d.segmente(0, 2000), Ok((vec![], ZeitGuete::Grob)));
        assert_eq!(d.grund(), Some(LeerGrund::KeineTonspur));
        assert!(d.dekoder.aufrufe.is_empty());
        assert_eq!(d.erkenner.aufrufe, 0);
    }

    #[test]
    fn dateiende_und_leere_fenster_brauchen_keinen_dekoder() {
        let mut d = dienst(vec![]);
        for (von, bis) in [(120_000, 121_000), (130_000, u64::MAX), (5, 5), (10, 5)] {
            assert_eq!(d.segmente(von, bis), Ok((vec![], ZeitGuete::Grob)));
            assert_eq!(d.grund(), Some(LeerGrund::Dateiende));
        }
        assert!(d.dekoder.aufrufe.is_empty());
        assert_eq!(d.erkenner.aufrufe, 0);
    }

    #[test]
    fn fenster_wird_am_dateiende_gekuerzt() {
        let mut d = dienst(vec![vec![satz("Ende", 100, 5000)]]);
        let (segmente, _) = d.segmente(119_000, 130_000).unwrap();
        assert_eq!(d.dekoder.aufrufe, vec![(119_000, 120_000)]);
        assert_eq!(segmente[0].end_ms, 120_000);
    }

    #[test]
    fn abbruch_vor_start_verhindert_beide_aufrufe() {
        let mut d = dienst(vec![]);
        d.abbruch.store(true, Ordering::Release);
        assert_eq!(
            d.segmente(0, 2000),
            Err(QuellenFehler("abgebrochen".into()))
        );
        assert!(d.dekoder.aufrufe.is_empty());
        assert_eq!(d.erkenner.aufrufe, 0);
    }

    #[test]
    fn abbruch_nach_dekodieren_verhindert_erkennung() {
        for fehler in [None, Some(DienstFehler::DekodierFehler)] {
            let mut d = dienst(vec![]);
            d.dekoder.abbrechen = true;
            d.dekoder.fehler = fehler;
            assert_eq!(
                d.segmente(0, 2000),
                Err(QuellenFehler("abgebrochen".into()))
            );
            assert_eq!(d.dekoder.aufrufe.len(), 1);
            assert_eq!(d.erkenner.aufrufe, 0);
        }
    }

    #[test]
    fn abbruch_nach_erkennung_liefert_keinen_inhalt_oder_lieferstand() {
        for fehler in [None, Some(DienstFehler::ErkennerFehler)] {
            let mut d = dienst(vec![vec![satz("Hallo", 100, 400)]]);
            d.erkenner.abbrechen = true;
            d.erkenner.fehler = fehler;
            assert_eq!(
                d.segmente(0, 2000),
                Err(QuellenFehler("abgebrochen".into()))
            );
            assert_eq!(d.erkenner.aufrufe, 1);
            assert_eq!(d.geliefert_bis_ms, 0);
        }
    }

    #[test]
    fn fehler_bleiben_feste_kurzformen() {
        for (fehler, text) in [
            (DienstFehler::Abgebrochen, "abgebrochen"),
            (DienstFehler::Beschaedigt, "datei_beschaedigt"),
            (DienstFehler::DekodierFehler, "dekodierfehler"),
            (DienstFehler::ErkennerFehler, "erkennerfehler"),
            (DienstFehler::KeinModell, "kein_modell"),
        ] {
            assert_eq!(fehler.to_string(), text);
            let mut d = dienst(vec![]);
            d.dekoder.fehler = Some(fehler.clone());
            assert_eq!(d.segmente(0, 2000), Err(QuellenFehler(text.into())));
            assert_eq!(d.erkenner.aufrufe, 0);
            let mut d = dienst(vec![]);
            d.erkenner.fehler = Some(fehler);
            assert_eq!(d.segmente(0, 2000), Err(QuellenFehler(text.into())));
        }
    }

    #[test]
    fn ueberlappende_fenster_liefern_jedes_wort_einmal() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![
                wort("eins", 100, 300),
                wort("zwei", 1000, 1300),
            ])],
            vec![mit_woertern(vec![
                wort("zwei", 0, 300),
                wort("drei", 700, 900),
            ])],
        ]);
        let (mut segmente, _) = d.segmente(0, 2000).unwrap();
        segmente.extend(d.segmente(1000, 3000).unwrap().0);
        let texte: Vec<_> = segmente
            .iter()
            .flat_map(|segment| segment.woerter.as_ref().unwrap())
            .map(|wort| wort.text.as_str())
            .collect();
        assert_eq!(texte, vec!["eins", "zwei", "drei"]);
    }

    #[test]
    fn ueberlappende_segmente_ohne_woerter_werden_verworfen() {
        let mut d = dienst(vec![
            vec![satz("alt", 100, 1300)],
            vec![satz("alt", 0, 300), satz("neu", 300, 900)],
        ]);
        d.segmente(0, 2000).unwrap();
        let (segmente, guete) = d.segmente(1000, 3000).unwrap();
        assert_eq!(segmente.len(), 1);
        assert_eq!(segmente[0].text, "neu");
        assert_eq!(segmente[0].start_ms, 1300);
        assert_eq!(guete, ZeitGuete::Satz);
    }

    #[test]
    fn teilweise_ueberlappende_wortsegmente_werden_gekuerzt() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![wort("alt", 1000, 1300)])],
            vec![mit_woertern(vec![
                wort("alt", 0, 300),
                wort(" neu ", 300, 600),
                wort("weiter", 700, 900),
            ])],
            vec![mit_woertern(vec![wort("weiter", 200, 400)])],
        ]);
        d.segmente(0, 2000).unwrap();
        let (segmente, guete) = d.segmente(1000, 3000).unwrap();
        assert_eq!(guete, ZeitGuete::Wort);
        assert_eq!(segmente[0].text, "neu weiter");
        assert_eq!(segmente[0].start_ms, 1300);
        assert_eq!(segmente[0].woerter.as_ref().unwrap().len(), 2);
        assert_eq!(d.geliefert_bis_ms, 1900);
        assert_eq!(d.segmente(1500, 3500), Ok((vec![], ZeitGuete::Grob)));
    }

    #[test]
    fn rueckwaertssprung_setzt_die_liefergrenze_zurueck() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![wort("spaet", 100, 1500)])],
            vec![mit_woertern(vec![wort("frueh", 100, 500)])],
        ]);
        d.segmente(60_000, 62_000).unwrap();
        let (segmente, _) = d.segmente(1000, 3000).unwrap();
        assert_eq!(segmente.len(), 1);
        assert_eq!(segmente[0].start_ms, 1100);
        assert_eq!(d.geliefert_bis_ms, 1500);
    }

    #[test]
    fn leere_texte_werden_verworfen_und_leere_wortlisten_sind_keine_wortguete() {
        let mut leer = satz("Satz", 600, 900);
        leer.woerter = Some(vec![wort(" \t ", 600, 900)]);
        let mut d = dienst(vec![vec![
            satz("", 0, 10),
            satz(" \n ", 10, 20),
            mit_woertern(vec![wort("Hallo", 100, 400), wort(" ", 400, 500)]),
            leer,
        ]]);
        let (segmente, guete) = d.segmente(0, 2000).unwrap();
        assert_eq!(segmente.len(), 2);
        assert_eq!(segmente[0].woerter.as_ref().unwrap().len(), 1);
        assert!(segmente[1].woerter.is_none());
        assert_eq!(guete, ZeitGuete::Satz);
    }

    #[test]
    fn zeiten_werden_geklammert_auch_bei_ueberlauf_und_vertauschten_enden() {
        let mut d = dienst(vec![vec![
            satz("vorwaerts", 0, u64::MAX),
            satz("vertauscht", 9000, 1),
            mit_woertern(vec![wort("Wort", u64::MAX, 0)]),
        ]]);
        let (segmente, _) = d.segmente(60_000, 62_000).unwrap();
        assert_eq!(segmente.len(), 3);
        assert_eq!(segmente[0].start_ms, 60_000);
        assert_eq!(segmente[0].end_ms, 62_000);
        for segment in &segmente {
            assert!((60_000..=62_000).contains(&segment.start_ms));
            assert!((segment.start_ms..=62_000).contains(&segment.end_ms));
            if let Some(woerter) = &segment.woerter {
                for wort in woerter {
                    assert_eq!((wort.start_ms, wort.end_ms), (62_000, 62_000));
                }
            }
        }
        assert_eq!((segmente[1].start_ms, segmente[1].end_ms), (62_000, 62_000));
    }

    #[test]
    fn jedes_gueltige_fenster_ruft_beide_adapter_genau_einmal_auf() {
        let mut d = dienst(vec![vec![], vec![satz("Hallo", 100, 400)]]);
        assert_eq!(d.segmente(0, 2000), Ok((vec![], ZeitGuete::Grob)));
        d.segmente(2000, 4000).unwrap();
        assert_eq!(d.dekoder.aufrufe, vec![(0, 2000), (2000, 4000)]);
        assert_eq!(d.erkenner.aufrufe, 2);
        assert_eq!(d.grund(), None);
    }

    #[test]
    fn letzter_grund_wird_geloescht_und_fehler_aendern_keinen_lieferstand() {
        let mut d = dienst(vec![vec![satz("Hallo", 100, 1000)]]);
        d.segmente(120_000, 130_000).unwrap();
        assert_eq!(d.grund(), Some(LeerGrund::Dateiende));
        d.segmente(0, 2000).unwrap();
        assert_eq!(d.grund(), None);
        d.segmente(120_000, 130_000).unwrap();
        d.erkenner.fehler = Some(DienstFehler::KeinModell);
        assert_eq!(d.segmente(0, 100), Err(QuellenFehler("kein_modell".into())));
        assert_eq!(d.grund(), None);
        assert_eq!(d.geliefert_bis_ms, 1000);
    }
}
