//! M3-P3a: Sprechertrennung (Diarisierung) einer Besprechungsspur.
//!
//! `diarize(pcm, params)` -> Turns `{start_ms, end_ms, speaker}` (Sprecher
//! 1-basiert in Ankunftsreihenfolge, Ueberlappung erlaubt). Modell:
//! Sortformer 4spk-v2.1 (GGUF Q8) ueber transcribe-cpp; die Nachbearbeitung
//! (Luecken <= 1 s schliessen, 100 ms Pad) ist Pflicht und Teil des Moduls
//! (Entwurf `m3-sprecher.md` §2.2, §3.1).
//!
//! Fehlerfaelle und ihre Absicherung:
//! - zwei Aufrufe gleichzeitig: [`engine::ExclusiveSlot`], nie zwei Modelle;
//! - wenig RAM: Start-Gate vor dem Laden ([`DiarizeError::LowMemory`]);
//! - Modell fehlt/falsches Modell: [`DiarizeError::ModelMissing`] /
//!   [`DiarizeError::WrongModel`], ohne Panik;
//! - Abbruch: `cancel` wird vor und nach dem Lauf geprueft und an
//!   transcribe-cpp weitergereicht; ein abgebrochener Lauf liefert KEINE
//!   Teil-Turns ([`DiarizeError::Cancelled`]);
//! - zu kurzes Audio (< 2 s): leeres Ergebnis, das Modell wird nicht geladen;
//! - NaN/Inf im Puffer: vor dem Modell auf 0 gesetzt (Kopie nur dann).

// Die Pipeline-API (`diarize`, Abbruch, Kurz-Audio) bindet erst P3b in den
// Enddurchlauf ein; bis dahin nutzt sie nur das DER-Werkzeug und die Tests.
#![allow(dead_code)]

pub mod der;
pub mod engine;
pub mod postproc;

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use transcribe_cpp::CancelToken;

pub use engine::{DiarizeEngine, Diarizer};

/// Abtastrate, die das Modell erwartet.
pub const SAMPLE_RATE: usize = 16_000;
/// Nachbearbeitung: Luecken bis hierhin je Sprecher schliessen (ms).
pub const DEFAULT_GAP_MS: u64 = 1000;
/// Nachbearbeitung: jeden Turn um so viel verlaengern (ms).
pub const DEFAULT_PAD_MS: u64 = 100;
/// Unter dieser Audiodauer wird nicht diarisiert (es kann keine 2 s Sprache
/// enthalten; Entwurf §4 "Kanal < 2 s Sprache").
pub const MIN_AUDIO_MS: u64 = 2000;

/// Katalog-ID des Standardmodells (catalog.json, Zweck `diarization`).
pub const DEFAULT_MODEL_ID: &str = "diar-sortformer-4spk-v2.1-q8";

/// Ein Sprecher-Turn eines Kanals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    /// 1-basiert, in Ankunftsreihenfolge.
    pub speaker: u32,
}

/// Arbeitspunkt des Streaming-Sortformers (Vorausschau vs. Rechenaufwand).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// Konfiguration aus dem GGUF.
    Default,
    /// ~30,4 s Vorausschau: der Arbeitspunkt fuer ganze Dateien (gemessen).
    VeryHighLatency,
    HighLatency,
    LowLatency,
}

/// Parameter eines Diarisierungslaufs.
#[derive(Debug, Clone)]
pub struct DiarizeParams {
    pub model_path: PathBuf,
    pub gap_ms: u64,
    pub pad_ms: u64,
    pub preset: Preset,
    /// CPU-Threads des Modells (Standard: halbe Kernzahl).
    pub threads: usize,
    /// Abbruch von aussen (Beenden der App, Nutzer bricht ab).
    pub cancel: Option<CancelToken>,
}

impl DiarizeParams {
    /// Standardwerte (Entwurf §6): gap 1000 ms, pad 100 ms, "very high latency".
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            gap_ms: DEFAULT_GAP_MS,
            pad_ms: DEFAULT_PAD_MS,
            preset: Preset::VeryHighLatency,
            threads: default_threads(),
            cancel: None,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.is_cancelled())
    }
}

/// Halbe Kernzahl, mindestens 1 (Entwurf §3.7): Diktat bleibt bedienbar.
pub fn default_threads() -> usize {
    (crate::process_guard::logical_cpus() / 2).max(1)
}

#[derive(Debug)]
pub enum DiarizeError {
    ModelMissing(PathBuf),
    WrongModel(String),
    LowMemory(String),
    Load(String),
    Run(String),
    Cancelled,
}

impl fmt::Display for DiarizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelMissing(p) => write!(
                f,
                "Sprechertrennungs-Modell nicht installiert: {}",
                p.display()
            ),
            Self::WrongModel(arch) => write!(
                f,
                "Kein Sprechertrennungs-Modell (Architektur '{arch}', erwartet sortformer)"
            ),
            Self::LowMemory(msg) => write!(f, "{msg}"),
            Self::Load(msg) => write!(f, "Sprechertrennung konnte nicht laden: {msg}"),
            Self::Run(msg) => write!(f, "Sprechertrennung fehlgeschlagen: {msg}"),
            Self::Cancelled => write!(f, "Sprechertrennung abgebrochen"),
        }
    }
}

impl std::error::Error for DiarizeError {}

/// Diarisiert einen Kanal: laedt das Modell (ein Diarisierer im Prozess,
/// RAM-Gate), rechnet, bearbeitet nach und entlaedt es wieder.
pub fn diarize(pcm: &[f32], p: &DiarizeParams) -> Result<Vec<Turn>, DiarizeError> {
    if p.cancelled() {
        return Err(DiarizeError::Cancelled);
    }
    if audio_ms(pcm) < MIN_AUDIO_MS {
        return Ok(Vec::new());
    }
    let mut d = Diarizer::load(&p.model_path, p.threads)?;
    d.diarize(pcm, p)
}

/// Die Pipeline mit beliebigem Motor (Tests, geladener [`Diarizer`]).
pub fn diarize_with<E: DiarizeEngine + ?Sized>(
    engine: &mut E,
    pcm: &[f32],
    p: &DiarizeParams,
) -> Result<Vec<Turn>, DiarizeError> {
    if p.cancelled() {
        return Err(DiarizeError::Cancelled);
    }
    let dur = audio_ms(pcm);
    if dur < MIN_AUDIO_MS {
        return Ok(Vec::new());
    }
    let clean = finite_pcm(pcm);
    let raw = engine.raw_turns(&clean, p.preset, p.cancel.as_ref())?;
    if p.cancelled() {
        return Err(DiarizeError::Cancelled);
    }
    Ok(postproc::apply(&raw, p.gap_ms, p.pad_ms, dur))
}

/// Dauer eines 16-kHz-Puffers in ms.
pub fn audio_ms(pcm: &[f32]) -> u64 {
    (pcm.len() as u64 * 1000) / SAMPLE_RATE as u64
}

/// NaN/Inf auf 0 setzen; ohne solche Werte keine Kopie.
fn finite_pcm(pcm: &[f32]) -> Cow<'_, [f32]> {
    if pcm.iter().all(|x| x.is_finite()) {
        Cow::Borrowed(pcm)
    } else {
        Cow::Owned(
            pcm.iter()
                .map(|&x| if x.is_finite() { x } else { 0.0 })
                .collect(),
        )
    }
}

/// Modell fuer einen Aufruf: `spec` ist ein Dateipfad oder eine Katalog-ID,
/// ohne Angabe das Standardmodell. Unbekannte IDs gelten als Pfad (das Laden
/// meldet dann `ModelMissing`).
pub fn resolve_model_path(models_root: &Path, spec: Option<&str>) -> PathBuf {
    let spec = spec.map(str::trim).filter(|s| !s.is_empty());
    match spec {
        Some(s) if Path::new(s).is_file() => PathBuf::from(s),
        Some(s) => catalog_model_path(models_root, s).unwrap_or_else(|| PathBuf::from(s)),
        None => catalog_model_path(models_root, DEFAULT_MODEL_ID)
            .unwrap_or_else(|| models_root.join("diarization")),
    }
}

/// Pfad des installierten Katalogmodells `model_id` unter `models_root`
/// (`<app_data>/models`): `<models_root>/diarization/<datei>`. `None`, wenn
/// die ID nicht im Katalog steht. Eigener Unterordner, weil der Modell-Scan
/// des `ModelManager` nur die oberste Ebene liest: so erscheint der
/// Diarisierer nie als Transkriptionsmodell.
pub fn catalog_model_path(models_root: &Path, model_id: &str) -> Option<PathBuf> {
    let entry = crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
        .into_iter()
        .find(|e| e.id == model_id)?;
    let file = entry.files.first()?;
    Some(models_root.join("diarization").join(&file.filename))
}

#[cfg(test)]
mod tests {
    use super::postproc::RawTurn;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Attrappe: liefert feste Turns, zaehlt Aufrufe, prueft die Eingabe.
    struct Fake {
        turns: Vec<RawTurn>,
        calls: AtomicUsize,
        saw_non_finite: bool,
        cancel_during_run: bool,
    }

    impl Fake {
        fn new(turns: Vec<RawTurn>) -> Self {
            Self {
                turns,
                calls: AtomicUsize::new(0),
                saw_non_finite: false,
                cancel_during_run: false,
            }
        }
    }

    impl DiarizeEngine for Fake {
        fn raw_turns(
            &mut self,
            pcm: &[f32],
            _preset: Preset,
            cancel: Option<&CancelToken>,
        ) -> Result<Vec<RawTurn>, DiarizeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.saw_non_finite |= pcm.iter().any(|x| !x.is_finite());
            if self.cancel_during_run {
                if let Some(c) = cancel {
                    c.cancel();
                }
            }
            Ok(self.turns.clone())
        }
    }

    fn secs(s: usize) -> Vec<f32> {
        vec![0.01; s * SAMPLE_RATE]
    }

    fn params() -> DiarizeParams {
        let mut p = DiarizeParams::new("unbenutzt.gguf");
        p.threads = 1;
        p
    }

    fn raw(t0_ms: i64, t1_ms: i64, speaker_id: i32) -> RawTurn {
        RawTurn {
            t0_ms,
            t1_ms,
            speaker_id,
        }
    }

    #[test]
    fn defaults_follow_the_design() {
        let p = DiarizeParams::new("m.gguf");
        assert_eq!((p.gap_ms, p.pad_ms), (1000, 100));
        assert_eq!(p.preset, Preset::VeryHighLatency);
        assert!(p.threads >= 1);
    }

    #[test]
    fn pipeline_postprocesses_and_numbers_by_arrival() {
        let mut e = Fake::new(vec![
            raw(5000, 6000, 1),
            raw(1000, 2000, 2),
            raw(2500, 3000, 2),
        ]);
        let got = diarize_with(&mut e, &secs(10), &params()).unwrap();
        assert_eq!(
            got,
            vec![
                Turn {
                    start_ms: 900,
                    end_ms: 3100,
                    speaker: 1
                },
                Turn {
                    start_ms: 4900,
                    end_ms: 6100,
                    speaker: 2
                },
            ]
        );
    }

    #[test]
    fn short_audio_is_not_sent_to_the_model() {
        let mut e = Fake::new(vec![raw(0, 500, 1)]);
        let got = diarize_with(&mut e, &vec![0.1; SAMPLE_RATE], &params()).unwrap();
        assert!(got.is_empty());
        assert_eq!(e.calls.load(Ordering::SeqCst), 0);
        // Auch die oeffentliche Funktion laedt fuer kurzes Audio kein Modell
        // (der Pfad existiert nicht; ein Ladeversuch waere ModelMissing).
        assert!(diarize(&[0.0; 100], &params()).unwrap().is_empty());
    }

    #[test]
    fn cancel_before_the_run_calls_nothing() {
        let mut e = Fake::new(vec![raw(0, 5000, 1)]);
        let mut p = params();
        let token = CancelToken::new();
        token.cancel();
        p.cancel = Some(token);
        let err = diarize_with(&mut e, &secs(5), &p).unwrap_err();
        assert!(matches!(err, DiarizeError::Cancelled));
        assert_eq!(e.calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            diarize(&secs(5), &p),
            Err(DiarizeError::Cancelled)
        ));
    }

    #[test]
    fn cancel_during_the_run_returns_no_partial_turns() {
        let mut e = Fake::new(vec![raw(0, 5000, 1)]);
        e.cancel_during_run = true;
        let mut p = params();
        p.cancel = Some(CancelToken::new());
        let err = diarize_with(&mut e, &secs(5), &p).unwrap_err();
        assert!(matches!(err, DiarizeError::Cancelled));
    }

    #[test]
    fn non_finite_samples_never_reach_the_model() {
        let mut e = Fake::new(vec![]);
        let mut pcm = secs(3);
        pcm[10] = f32::NAN;
        pcm[20] = f32::INFINITY;
        diarize_with(&mut e, &pcm, &params()).unwrap();
        assert!(!e.saw_non_finite);
        assert!(matches!(finite_pcm(&secs(1)), Cow::Borrowed(_)));
    }

    #[test]
    fn a_missing_model_is_an_error_not_a_panic() {
        let p = DiarizeParams::new("Z:/gibt/es/nicht/diar.gguf");
        match diarize(&secs(3), &p) {
            Err(DiarizeError::ModelMissing(path)) => assert!(path.ends_with("diar.gguf")),
            other => panic!("erwartet ModelMissing, bekam {other:?}"),
        }
    }

    #[test]
    fn engine_errors_pass_through() {
        struct Broken;
        impl DiarizeEngine for Broken {
            fn raw_turns(
                &mut self,
                _: &[f32],
                _: Preset,
                _: Option<&CancelToken>,
            ) -> Result<Vec<RawTurn>, DiarizeError> {
                Err(DiarizeError::Run("kaputt".into()))
            }
        }
        let err = diarize_with(&mut Broken, &secs(3), &params()).unwrap_err();
        assert!(err.to_string().contains("kaputt"));
    }

    #[test]
    fn the_model_is_a_path_a_catalog_id_or_the_default() {
        let root = Path::new("C:/data/models");
        let default = resolve_model_path(root, None);
        assert_eq!(default, catalog_model_path(root, DEFAULT_MODEL_ID).unwrap());
        assert_eq!(resolve_model_path(root, Some("  ")), default);
        assert_eq!(resolve_model_path(root, Some(DEFAULT_MODEL_ID)), default);
        let file = tempfile::NamedTempFile::new().unwrap();
        let spec = file.path().to_str().unwrap();
        assert_eq!(resolve_model_path(root, Some(spec)), file.path());
        assert_eq!(
            resolve_model_path(root, Some("D:/fehlt.gguf")),
            PathBuf::from("D:/fehlt.gguf")
        );
    }

    #[test]
    fn the_catalog_knows_the_default_model() {
        let p = catalog_model_path(Path::new("C:/data/models"), DEFAULT_MODEL_ID)
            .expect("Katalogeintrag fehlt");
        assert!(p.ends_with("diarization/diar_streaming_sortformer_4spk-v2.1-Q8_0.gguf"));
        assert!(catalog_model_path(Path::new("C:/x"), "gibt-es-nicht").is_none());
        // Download verifizierbar (Pruefsumme, fester Stand) und nie im ASR-Katalog.
        let entry = crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
            .into_iter()
            .find(|e| e.id == DEFAULT_MODEL_ID)
            .unwrap();
        let file = &entry.files[0];
        assert_eq!(file.sha256.as_deref().map(str::len), Some(64));
        assert!(file
            .url
            .starts_with("https://huggingface.co/handy-computer/"));
        assert!(file
            .url
            .contains("ae4afbb5c3d33b71cf2dbf600022b655ee706dd0"));
        assert!(!crate::catalog::CATALOG
            .iter()
            .any(|d| d.id.contains("sortformer")));
    }
}
