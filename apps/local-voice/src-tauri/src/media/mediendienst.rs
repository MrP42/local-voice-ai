//! Fensterweise Medienerkennung mit absoluten Zeiten und Ueberlappungsschutz.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::zeitachse::{MedienQuelle, Segment, Wort, ZeitGuete};
use crate::media_translation::{MediaSegmentSource, QuellenFehler};

const VORLAUF_MS: u64 = 2_000;
const NACHLAUF_MS: u64 = 2_000;
const NAHT_MS: u64 = 300;

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
    schnitt_ms: u64,
    letzter_eigen_von: u64,
    letzte_woerter: Vec<(String, u64)>,
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
            schnitt_ms: 0,
            letzter_eigen_von: 0,
            letzte_woerter: Vec::new(),
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

fn normalisiere_wort(text: &str) -> String {
    text.chars()
        .filter(|zeichen| zeichen.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn mitte(start_ms: u64, end_ms: u64) -> u64 {
    start_ms + (end_ms - start_ms) / 2
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
        // Erst nach erfolgreicher Erkennung wird der Lieferstand veraendert.
        let rueckwaerts = von_ms < self.letzter_eigen_von;
        let schnitt_ms = if rueckwaerts { von_ms } else { self.schnitt_ms };
        let eigen_von = von_ms.max(schnitt_ms);
        let eigen_bis = bis_ms;
        let dekodier_von = eigen_von.saturating_sub(VORLAUF_MS);
        let dekodier_bis = eigen_bis
            .saturating_add(NACHLAUF_MS)
            .min(self.quelle.dauer_ms);
        let samples = self
            .dekoder
            .fenster(&self.pfad, dekodier_von, dekodier_bis, &self.abbruch);
        self.pruefe_abbruch()?;
        let roh = self.erkenner.erkenne(samples?);
        self.pruefe_abbruch()?;
        let roh = roh?;

        let absolut = |zeit: u64| dekodier_von.saturating_add(zeit).min(dekodier_bis);
        let gehoert = |mitte: u64| eigen_von <= mitte && mitte < eigen_bis;
        let mut letzte_woerter = Vec::new();
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
                if !gehoert(mitte(segment.start_ms, segment.end_ms)) {
                    continue;
                }
            } else {
                let woerter: Vec<Wort> = woerter
                    .into_iter()
                    .filter(|wort| {
                        let zeit = mitte(wort.start_ms, wort.end_ms);
                        if !gehoert(zeit) {
                            return false;
                        }
                        // Nur die direkte Naht mit belegten Wortmitten absichern.
                        rueckwaerts
                            || zeit >= eigen_von.saturating_add(NAHT_MS)
                            || !self.letzte_woerter.iter().any(|(text, alt)| {
                                *alt >= schnitt_ms.saturating_sub(NAHT_MS)
                                    && *alt < schnitt_ms
                                    && zeit.abs_diff(*alt) <= NAHT_MS
                                    && !text.is_empty()
                                    && *text == normalisiere_wort(&wort.text)
                            })
                    })
                    .collect();
                let Some(erstes) = woerter.first() else {
                    continue;
                };
                segment.start_ms = erstes.start_ms;
                segment.end_ms = woerter.last().unwrap().end_ms.max(segment.start_ms);
                segment.text = woerter
                    .iter()
                    .map(|wort| wort.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                for wort in &woerter {
                    let zeit = mitte(wort.start_ms, wort.end_ms);
                    if zeit >= eigen_bis.saturating_sub(600) && zeit < eigen_bis {
                        let text = normalisiere_wort(&wort.text);
                        if !text.is_empty() {
                            letzte_woerter.push((text, zeit));
                        }
                    }
                }
                segment.woerter = Some(woerter);
            }
            segmente.push(segment);
        }
        let guete = if segmente.is_empty() {
            ZeitGuete::Grob
        } else if segmente.iter().all(|segment| {
            segment.end_ms > segment.start_ms
                && segment
                    .woerter
                    .as_ref()
                    .is_some_and(|woerter| woerter.iter().all(|wort| wort.end_ms > wort.start_ms))
        }) {
            ZeitGuete::Wort
        } else if segmente
            .iter()
            .all(|segment| segment.end_ms > segment.start_ms)
            && !segmente
                .iter()
                .all(|segment| segment.start_ms == dekodier_von && segment.end_ms == dekodier_bis)
        {
            ZeitGuete::Satz
        } else {
            ZeitGuete::Grob
        };
        self.schnitt_ms = eigen_bis;
        self.letzter_eigen_von = eigen_von;
        self.letzte_woerter = letzte_woerter;
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
        let mut roh = mit_woertern(vec![wort("Hallo", 2100, 2400)]);
        roh.end_ms = 4000;
        let mut d = dienst(vec![vec![roh]]);
        assert_eq!(d.grund(), None);
        let (segmente, guete) = d.segmente(60_000, 62_000).unwrap();
        assert_eq!(guete, ZeitGuete::Wort);
        assert_eq!(segmente[0].start_ms, 60_100);
        // Gegenprobe: Der Segmentrahmen darf nicht statt des letzten Wortes gelten.
        assert_eq!(segmente[0].end_ms, 60_400);
        assert_eq!(
            segmente[0].woerter,
            Some(vec![Wort {
                start_ms: 60_100,
                end_ms: 60_400,
                text: "Hallo".into(),
            }])
        );
        assert_eq!(d.schnitt_ms, 62_000);
        assert_eq!(d.dekoder.aufrufe, vec![(58_000, 64_000)]);
    }

    #[test]
    fn eigene_segmentspannen_ergeben_satzguete() {
        let mut d = dienst(vec![vec![
            satz("Hallo.", 2100, 2400),
            satz("Welt.", 2600, 2900),
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
            vec![satz("Hallo", 0, 6000), satz("Welt", 0, 6000)],
            vec![satz("Hallo", 2400, 2100)],
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
        let mut d = dienst(vec![vec![satz("Ende", 2100, 5000)]]);
        let (segmente, _) = d.segmente(119_000, 130_000).unwrap();
        assert_eq!(d.dekoder.aufrufe, vec![(117_000, 120_000)]);
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
            assert_eq!(d.schnitt_ms, 0);
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

    // Die Testausgaben enthalten auch Kontext, wie ein echter Erkenner.
    // Ab dem dritten Fenster sind die Rohzeiten relativ zum neuen Dekodieranfang.
    fn drei_wortfenster() -> Vec<Vec<RohSegment>> {
        vec![
            vec![mit_woertern(vec![
                wort("eins", 200, 500),
                wort("zwei", 1850, 2050),
                wort("drei", 2700, 3000),
            ])],
            vec![
                mit_woertern(vec![wort("eins", 240, 540)]),
                mit_woertern(vec![
                    wort("zwei", 1890, 2090),
                    wort("drei", 2740, 3040),
                    wort("vier", 3850, 4050),
                ]),
            ],
            vec![mit_woertern(vec![
                wort("drei", 660, 960),
                wort("vier", 1890, 2090),
                wort("fuenf", 2700, 3000),
            ])],
        ]
    }

    #[test]
    fn drei_fenster_liefern_jedes_wort_in_reihenfolge_einmal() {
        let mut d = dienst(drei_wortfenster());
        let mut texte = Vec::new();
        for (von, bis) in [(0, 2000), (2000, 4000), (4000, 6000)] {
            let (segmente, guete) = d.segmente(von, bis).unwrap();
            assert_eq!(guete, ZeitGuete::Wort);
            for segment in segmente {
                let woerter = segment.woerter.unwrap();
                assert_eq!(segment.start_ms, woerter.first().unwrap().start_ms);
                assert_eq!(segment.end_ms, woerter.last().unwrap().end_ms);
                texte.extend(woerter.into_iter().map(|wort| wort.text));
            }
        }
        // Gegenprobe: Kontext ohne Eigentum liefert die Grenzwoerter mehrfach.
        assert_eq!(texte, ["eins", "zwei", "drei", "vier", "fuenf"]);
        assert_eq!(d.dekoder.aufrufe, [(0, 4000), (0, 6000), (2000, 8000)]);
        assert_eq!(d.erkenner.aufrufe, 3);
    }

    #[test]
    fn drei_fenster_ohne_wortzeiten_behalten_ganze_segmentinhalte() {
        let mut d = dienst(vec![
            vec![
                satz("Erster Satz.", 1700, 2200),
                satz("Zweiter Satz.", 2500, 3300),
            ],
            vec![
                satz("Erster", 1740, 1940),
                satz("Satz.", 1860, 2100),
                satz("Zweiter Satz.", 2540, 3340),
                satz("Dritter Satz.", 4500, 5300),
            ],
            vec![
                satz("Zweiter", 540, 840),
                satz("Satz.", 880, 1300),
                satz("Dritter Satz.", 2460, 3260),
            ],
        ]);
        let mut texte = Vec::new();
        for (von, bis) in [(0, 2000), (2000, 4000), (4000, 6000)] {
            let (segmente, guete) = d.segmente(von, bis).unwrap();
            assert_eq!(guete, ZeitGuete::Satz);
            assert_eq!(segmente.len(), 1);
            if von == 0 {
                // Gegenprobe: Schneiden an der Grenze veraendert die belegte Satzspanne.
                assert_eq!(segmente[0].end_ms, 2200);
            }
            texte.extend(segmente.into_iter().map(|segment| segment.text));
        }
        // Gegenprobe: Textabgleich statt Mittenzuordnung laesst Teilsegmente erneut durch.
        assert_eq!(texte, ["Erster Satz.", "Zweiter Satz.", "Dritter Satz."]);
    }

    #[test]
    fn nahtschutz_verwirft_nur_direkte_jitterduplikate() {
        for (start, ende) in [(1910, 2020), (1990, 2100)] {
            let mut d = dienst(vec![
                vec![mit_woertern(vec![wort("Alt,", start, ende)])],
                vec![mit_woertern(vec![wort("ALT!", 2010, 2120)])],
            ]);
            let (mut segmente, _) = d.segmente(0, 2000).unwrap();
            segmente.extend(d.segmente(2000, 4000).unwrap().0);
            // Gegenprobe: Ohne Nahtschutz wird die ueber den Schnitt gerutschte Mitte doppelt.
            assert_eq!(segmente.len(), 1);
        }
    }

    #[test]
    fn echte_wiederholung_bleibt_auch_direkt_an_der_naht() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![
                wort("ja", 1000, 1300),
                wort("ja", 1700, 1900),
            ])],
            vec![mit_woertern(vec![
                wort("ja", 1040, 1340),
                wort("ja", 1700, 1900),
            ])],
        ]);
        let (mut segmente, _) = d.segmente(0, 1500).unwrap();
        segmente.extend(d.segmente(1500, 3000).unwrap().0);
        // Gegenprobe: Textgleichheit ohne begrenzten Mittenabstand verschluckt die Wiederholung.
        let texte: Vec<_> = segmente
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(texte, ["ja", "ja"]);
        assert_eq!(segmente[1].start_ms, 1700);
    }

    #[test]
    fn nahtabstand_ist_auf_300_ms_begrenzt() {
        for (mitte, anzahl) in [(2050, 1), (2051, 2)] {
            let mut d = dienst(vec![
                vec![mit_woertern(vec![wort("ja", 1700, 1800)])],
                vec![mit_woertern(vec![wort("JA!", mitte - 50, mitte + 50)])],
            ]);
            let (mut segmente, _) = d.segmente(0, 2000).unwrap();
            segmente.extend(d.segmente(2000, 4000).unwrap().0);
            // Gegenprobe: Ohne Abstandsgrenze entfaellt auch die Wiederholung nach 301 ms.
            assert_eq!(segmente.len(), anzahl);
        }
    }

    #[test]
    fn eigentumsgrenze_ist_links_inklusiv_und_rechts_exklusiv() {
        for mit_wortzeiten in [false, true] {
            let ergebnis = || {
                if mit_wortzeiten {
                    mit_woertern(vec![wort("Grenze", 1900, 2100)])
                } else {
                    satz("Grenze", 1900, 2100)
                }
            };
            let mut d = dienst(vec![vec![ergebnis()], vec![ergebnis()]]);
            // Gegenprobe: Eine inklusive rechte Grenze liefert die Mitte 2000 zweimal.
            assert!(d.segmente(0, 2000).unwrap().0.is_empty());
            let (segmente, _) = d.segmente(2000, 4000).unwrap();
            assert_eq!(segmente.len(), 1);
            assert_eq!(segmente[0].text, "Grenze");
            assert_eq!((segmente[0].start_ms, segmente[0].end_ms), (1900, 2100));
        }
    }

    #[test]
    fn ueberlappende_planerfenster_erben_nur_den_neuen_bereich() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![
                wort("eins", 100, 400),
                wort("zwei", 2400, 2700),
            ])],
            vec![mit_woertern(vec![
                wort("eins", 100, 400),
                wort("zwei", 2440, 2740),
            ])],
        ]);
        let (erstes, _) = d.segmente(0, 2000).unwrap();
        let (zweites, _) = d.segmente(1000, 3000).unwrap();
        // Gegenprobe: Die alte Liefergrenze bei 400 ms liefert den Kontext erneut.
        assert_eq!(erstes[0].text, "eins");
        assert_eq!(zweites[0].text, "zwei");
        assert_eq!(d.letzter_eigen_von, 2000);
        assert_eq!(d.schnitt_ms, 3000);
        assert_eq!(d.dekoder.aufrufe, [(0, 4000), (0, 5000)]);
    }

    #[test]
    fn seek_vergleicht_mit_dem_letzten_eigenen_anfang() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![wort("ja", 1700, 1900)])],
            vec![mit_woertern(vec![wort("ja", 2010, 2120)])],
            vec![mit_woertern(vec![wort("ja", 2050, 2150)])],
        ]);
        d.segmente(0, 2000).unwrap();
        d.segmente(1000, 3000).unwrap();
        let (segmente, _) = d.segmente(1500, 2500).unwrap();
        // Gegenprobe: 1500 ist groesser als der letzte Planeranfang 1000, aber ein Seek.
        assert_eq!(segmente[0].text, "ja");
        assert_eq!(d.letzter_eigen_von, 1500);
        assert_eq!(d.schnitt_ms, 2500);
    }

    #[test]
    fn nahtspeicher_enthaelt_nur_gelieferte_woerter_der_letzten_600_ms() {
        let mut d = dienst(vec![vec![mit_woertern(vec![
            wort("alt", 100, 400),
            wort("davor", 1300, 1500),
            wort("naht", 1800, 2000),
            wort("danach", 2200, 2400),
        ])]]);
        d.segmente(0, 2000).unwrap();
        // Gegenprobe: Ein Textpuffer oder Kontextwoerter speichern unbelegte Nahtkandidaten.
        assert_eq!(
            d.letzte_woerter,
            [("davor".to_string(), 1400), ("naht".to_string(), 1900)]
        );
    }

    #[test]
    fn drei_fenster_gehen_ohne_textverlust_durch_den_satzbildner() {
        use crate::media_translation::sentences::Satzbildner;

        for mit_wortzeiten in [false, true] {
            for teile in [
                vec![
                    ("Hallo", 200, 500),
                    ("Welt", 2200, 2500),
                    ("heute.", 4200, 4500),
                ],
                vec![
                    ("eins", 200, 500),
                    ("zwei drei", 2200, 2700),
                    ("vier", 4200, 4500),
                ],
            ] {
                let erwartet = teile
                    .iter()
                    .map(|(text, _, _)| *text)
                    .collect::<Vec<_>>()
                    .join(" ");
                let ergebnisse = [0, 0, 2000]
                    .into_iter()
                    .map(|offset| {
                        teile
                            .iter()
                            .filter(|(_, start, _)| *start >= offset)
                            .map(|(text, start, ende)| {
                                if mit_wortzeiten {
                                    let woerter = text
                                        .split_whitespace()
                                        .enumerate()
                                        .map(|(index, text)| {
                                            let anfang = start - offset + index as u64 * 200;
                                            wort(text, anfang, anfang + 150)
                                        })
                                        .collect();
                                    mit_woertern(woerter)
                                } else {
                                    satz(text, start - offset, ende - offset)
                                }
                            })
                            .collect()
                    })
                    .collect();
                let mut d = dienst(ergebnisse);
                let mut bildner = Satzbildner::default();
                let mut saetze = Vec::new();
                for (von, bis) in [(0, 2000), (2000, 4000), (4000, 6000)] {
                    let (segmente, guete) = d.segmente(von, bis).unwrap();
                    saetze.extend(bildner.fenster("test", segmente, guete, bis == 6000));
                }
                // Gegenprobe: Gelieferter Kontext verdoppelt die dreigeteilten Satzinhalte.
                assert_eq!(saetze.len(), 1);
                assert_eq!(saetze[0].text, erwartet);
            }
        }
    }

    #[test]
    fn null_dauer_ergibt_keine_wortguete() {
        let mut d = dienst(vec![vec![mit_woertern(vec![wort("alt", 1000, 1000)])]]);
        assert_eq!(d.segmente(0, 2000).unwrap().1, ZeitGuete::Grob);
    }

    #[test]
    fn rueckwaertssprung_setzt_die_liefergrenze_zurueck() {
        let mut d = dienst(vec![
            vec![mit_woertern(vec![wort("spaet", 2100, 3500)])],
            vec![mit_woertern(vec![wort("frueh", 1100, 1500)])],
        ]);
        d.segmente(60_000, 62_000).unwrap();
        let (segmente, _) = d.segmente(1000, 3000).unwrap();
        assert_eq!(segmente.len(), 1);
        assert_eq!(segmente[0].start_ms, 1100);
        assert_eq!(d.schnitt_ms, 3000);
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
            satz("vertauscht", 2500, 1),
            mit_woertern(vec![wort("Wort", u64::MAX, 0)]),
        ]]);
        let (segmente, _) = d.segmente(60_000, 62_000).unwrap();
        // Gegenprobe: Ohne Klemmung laeuft die Addition ueber oder die Mitte liegt ausserhalb.
        assert_eq!(segmente.len(), 2);
        assert_eq!(segmente[0].start_ms, 58_000);
        assert_eq!(segmente[0].end_ms, 64_000);
        for segment in &segmente {
            assert!((58_000..=64_000).contains(&segment.start_ms));
            assert!((segment.start_ms..=64_000).contains(&segment.end_ms));
        }
        assert_eq!((segmente[1].start_ms, segmente[1].end_ms), (60_500, 60_500));
    }

    #[test]
    fn jedes_gueltige_fenster_ruft_beide_adapter_genau_einmal_auf() {
        let mut d = dienst(vec![vec![], vec![satz("Hallo", 100, 400)]]);
        assert_eq!(d.segmente(0, 2000), Ok((vec![], ZeitGuete::Grob)));
        d.segmente(2000, 4000).unwrap();
        assert_eq!(d.dekoder.aufrufe, vec![(0, 4000), (0, 6000)]);
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
        assert_eq!(d.schnitt_ms, 2000);
    }
}
