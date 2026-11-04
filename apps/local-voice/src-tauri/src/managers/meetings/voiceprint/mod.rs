//! Stimmabdrücke innerhalb einer Besprechung (Konzept Stimmprofile, Schritt 2a).
//!
//! Ein kleines Sprecher-Embedding-Modell (3D-Speaker CAM++, Apache-2.0) rechnet
//! je Sprecher einen Schwerpunkt aus seinen längsten Sätzen. Ähnliche Sprecher
//! derselben Spur ergeben einen Vorschlag „klingt wie …, zusammenführen?“.
//!
//! Datenschutz: Die Vektoren bleiben im Speicher dieses Prozesses und werden nie
//! geschrieben, geloggt oder mit anderen Besprechungen verglichen. Gespeichert
//! wird nur, welche Vorschläge der Nutzer verworfen hat (Sprechernummern).

pub mod fbank;
pub mod similar;

use crate::managers::meetings::diarize::Turn;
use similar::{Clip, Voice};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Katalog-ID des Modells.
pub const MODEL_ID: &str = "spk-campplus-voxceleb-16k";
/// Unterordner unter `<app_data>/models` (nicht oben, sonst hielte der
/// Modell-Scan die Datei für ein Transkriptionsmodell).
pub const MODEL_DIR: &str = "speaker-embedding";
/// Das Modell mittelt in Abschnitten zu 100 Zeitschritten (`AveragePool` mit
/// `ceil_mode`), nach der Halbierung am Eingang also 200 Rahmen. Einen
/// angebrochenen Abschnitt teilen ONNX Runtime bis 1.24 durch 100, ab 1.29 wie
/// PyTorch durch die echte Länge. Mit ganzen Abschnitten rechnen alle Versionen
/// gleich (am Modell geprüft), deshalb nutzt [`Embedder::embed`] nur Vielfache.
pub const SEGMENT_FRAMES: usize = 200;
/// Arbeitsspeicher, den ein Lauf ungefähr braucht (Modell + ein Kanal Audio).
pub const RAM_NEED_MB: u64 = 400;

/// Katalogeintrag des Modells.
pub fn catalog_entry() -> Option<crate::catalog::TtsCatalogEntry> {
    crate::catalog::tts_entries(crate::catalog::Purpose::SpeakerEmbedding)
        .into_iter()
        .find(|e| e.id == MODEL_ID)
}

/// Pfad der Modelldatei unter `models_root` (`<app_data>/models`).
pub fn model_path(models_root: &Path) -> Option<PathBuf> {
    let entry = catalog_entry()?;
    let file = entry.files.first()?;
    Some(models_root.join(MODEL_DIR).join(&file.filename))
}

/// Das geladene Modell.
pub struct Embedder {
    session: ort::session::Session,
}

impl Embedder {
    pub fn load(path: &Path) -> Result<Self, String> {
        use ort::session::builder::GraphOptimizationLevel;
        if !path.is_file() {
            return Err("model_missing".into());
        }
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().clamp(1, 4))
            .unwrap_or(2);
        let err = |e: &dyn std::fmt::Display| format!("voiceprint model: {e}");
        let session = ort::session::Session::builder()
            .map_err(|e| err(&e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| err(&e))?
            .with_intra_threads(threads)
            .map_err(|e| err(&e))?
            .with_inter_threads(1)
            .map_err(|e| err(&e))?
            .commit_from_file(path)
            .map_err(|e| err(&e))?;
        Ok(Self { session })
    }

    /// Stimmabdruck eines Ausschnitts (16 kHz, [-1, 1]). Genutzt werden die
    /// ersten ganzen [`SEGMENT_FRAMES`], der Rest fällt weg.
    pub fn embed(&mut self, pcm: &[f32]) -> Result<Vec<f32>, String> {
        let mut feats = fbank::compute(pcm);
        let frames = feats.len() / fbank::NUM_BINS / SEGMENT_FRAMES * SEGMENT_FRAMES;
        if frames == 0 {
            return Err("clip_too_short".into());
        }
        feats.truncate(frames * fbank::NUM_BINS);
        fbank::subtract_mean(&mut feats);
        let input = ort::value::Tensor::from_array(([1usize, frames, fbank::NUM_BINS], feats))
            .map_err(|e| format!("voiceprint input: {e}"))?;
        let outputs = self
            .session
            .run(ort::inputs!["x" => input])
            .map_err(|e| format!("voiceprint run: {e}"))?;
        let (_, data) = outputs["embedding"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("voiceprint output: {e}"))?;
        Ok(data.to_vec())
    }
}

/// Der Ausschnitt `clip` aus `pcm`, wenn das Audio ihn ganz enthält
/// (mindestens [`similar::MIN_CLIP_MS`]); sonst `None`.
pub fn clip_pcm(pcm: &[f32], clip: Clip) -> Option<&[f32]> {
    let at = |ms: u64| ((ms as usize) * fbank::SAMPLE_RATE / 1_000).min(pcm.len());
    let part = &pcm[at(clip.start_ms)..at(clip.end_ms)];
    (part.len() >= (similar::MIN_CLIP_MS as usize) * fbank::SAMPLE_RATE / 1_000).then_some(part)
}

/// Stimmabdrücke aller Sprecher eines Kanals mit genug Sprache. `embed`
/// liefert den Abdruck eines Ausschnitts, `None`, wenn das Audio ihn nicht
/// enthält (Modell, Zwischenspeicher, in Tests eine Attrappe).
pub fn voices_of_channel(
    channel: u8,
    turns: &[Turn],
    embed: &mut dyn FnMut(Clip) -> Result<Option<Vec<f32>>, String>,
) -> Result<Vec<Voice>, String> {
    let speakers: BTreeMap<u32, ()> = turns.iter().map(|t| (t.speaker, ())).collect();
    let mut out = Vec::new();
    for &speaker in speakers.keys() {
        if speaker == 0 {
            continue;
        }
        let mut parts = Vec::new();
        for clip in similar::clips_for_speaker(turns, speaker) {
            if let Some(v) = embed(clip)? {
                parts.push((v, clip.len_ms()));
            }
        }
        let speech_ms = parts.iter().map(|p| p.1).sum();
        if let Some(vector) = similar::centroid(&parts) {
            out.push(Voice {
                channel,
                speaker,
                vector,
                speech_ms,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_in_the_catalog_with_a_license() {
        let entry = catalog_entry().expect("Katalogeintrag");
        assert_eq!(entry.license.as_deref(), Some("Apache-2.0"));
        let file = entry.files.first().unwrap();
        assert!(file.sha256.as_deref().is_some_and(|s| s.len() == 64));
        let path = model_path(Path::new("/m")).unwrap();
        assert!(path.starts_with(Path::new("/m").join(MODEL_DIR)));
    }

    #[test]
    fn voices_use_only_speakers_with_enough_solo_speech() {
        let turns = [
            Turn {
                start_ms: 0,
                end_ms: 6_000,
                speaker: 1,
            },
            Turn {
                start_ms: 6_000,
                end_ms: 7_000,
                speaker: 2,
            },
            Turn {
                start_ms: 7_000,
                end_ms: 15_000,
                speaker: 3,
            },
        ];
        let pcm = vec![0.1f32; 16 * 15_000];
        let mut calls = 0;
        let mut embed = |c: Clip| {
            calls += 1;
            Ok(clip_pcm(&pcm, c).map(|p| vec![p.len() as f32, 1.0]))
        };
        let voices = voices_of_channel(1, &turns, &mut embed).unwrap();
        assert_eq!(voices.iter().map(|v| v.speaker).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(calls, 2);
        assert_eq!(voices[0].speech_ms, 5_600);
    }

    /// 2,1 s: das Testsignal der Filterbank, vier Mal und ein Stück.
    fn long_signal() -> Vec<f32> {
        let one = fbank::tests::test_signal();
        one.iter().cycle().take(33_600).copied().collect()
    }

    #[test]
    fn clips_below_one_segment_are_refused_before_the_model() {
        let f = fbank::num_frames(16 * similar::MIN_CLIP_MS as usize);
        assert!(f >= SEGMENT_FRAMES, "{f}");
        let f = fbank::num_frames(16 * similar::MAX_CLIP_MS as usize);
        assert_eq!(f / SEGMENT_FRAMES * SEGMENT_FRAMES, 1_000, "{f}");
    }

    /// Mit echtem Modell (`VOICEPRINT_MODEL=<pfad>.onnx`): gleiche Werte wie
    /// `kaldi_native_fbank` + `onnxruntime` 1.24 und 1.29 in Python (2,1 s Testsignal).
    #[test]
    #[ignore = "braucht die Modelldatei"]
    fn the_real_model_matches_the_python_reference() {
        let path = std::env::var("VOICEPRINT_MODEL").expect("VOICEPRINT_MODEL");
        let mut model = Embedder::load(Path::new(&path)).unwrap();
        let e = model.embed(&long_signal()).unwrap();
        assert_eq!(e.len(), 512);
        let want = [-3.25160f32, 0.94728, -0.71560, 1.01669, 1.83247, -0.46177];
        for (g, w) in e.iter().zip(want) {
            assert!((g - w).abs() < 0.02, "{g} statt {w}");
        }
        let norm = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 34.406).abs() < 0.1, "{norm}");
    }

    #[test]
    fn audio_shorter_than_the_turns_is_skipped() {
        let turns = [Turn {
            start_ms: 0,
            end_ms: 20_000,
            speaker: 1,
        }];
        let pcm = vec![0.1f32; 16 * 1_500];
        let mut embed = |c: Clip| Ok(clip_pcm(&pcm, c).map(|_| vec![1.0, 0.0]));
        let voices = voices_of_channel(0, &turns, &mut embed).unwrap();
        assert!(voices.is_empty());
        // 3 s Audio: vom ersten Ausschnitt sind 2,8 s da (genug), vom zweiten nichts.
        let pcm = vec![0.1f32; 16 * 3_000];
        assert_eq!(
            clip_pcm(
                &pcm,
                Clip {
                    start_ms: 200,
                    end_ms: 10_220
                }
            )
            .map(<[f32]>::len),
            Some(16 * 2_800)
        );
        assert!(clip_pcm(
            &pcm,
            Clip {
                start_ms: 10_220,
                end_ms: 20_000
            }
        )
        .is_none());
    }
}
