//! `NemotronOnnxEngine`: Nemotron-3-Diarization ueber ONNX Runtime (CPU),
//! Paket N1. Die Sitzung rechnet je Aufruf einen Block ([`BlockModel`]); die
//! Schleife drumherum steht in [`super::blocks`].
//!
//! Das Modell besteht aus zwei Dateien, die nebeneinander liegen muessen: dem
//! Graphen `model_quantized.onnx` und den Gewichten `model_quantized.onnx_data`
//! (ONNX "external data", der Graph nennt die Datei beim Namen). Fehlt die zweite
//! oder ist sie abgeschnitten, meldet das Laden einen Fehler, nie eine Panik.
//!
//! Wie bei Sortformer (`diarize::engine`): ein Diarisierer im Prozess (der
//! `ExclusiveSlot` im umgebenden `Diarizer`), RAM-Start-Gate davor, eine Panik im
//! Aufruf macht die Engine unbrauchbar statt die App zu beenden.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use ort::memory::Allocator;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::{Tensor, TensorRef};
use transcribe_cpp::CancelToken;

use super::blocks::{
    run_offline, turns_from_logits, BlockInput, BlockModel, BlockOutput, BlockPlan,
};
use super::cache::CacheConfig;
use super::frontend::MelFrontend;
use super::{HIDDEN, MEL_BINS};
use crate::managers::meetings::diarize::postproc::RawTurn;
use crate::managers::meetings::diarize::{DiarizeEngine, DiarizeError, Preset};

const INPUTS: [&str; 3] = ["input_features", "cached_embeds", "attention_mask"];
const OUTPUTS: [&str; 3] = ["logits", "chunk_embeds", "silence_embeds"];

/// Datei mit den Gewichten neben dem Graphen: `<name>.onnx` -> `<name>.onnx_data`.
pub fn external_data_path(model_path: &Path) -> PathBuf {
    let mut name = model_path.as_os_str().to_owned();
    name.push("_data");
    PathBuf::from(name)
}

/// Dateiname fuer Fehlertexte (kein voller Pfad).
fn short(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Entfernt den Ordner der Modelldatei aus einem Fehlertext von ONNX Runtime
/// (der Text nennt die Pfade ausgeschrieben): kein voller Pfad im Log.
fn without_dir(text: &str, model_path: &Path) -> String {
    let Some(dir) = model_path
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
    else {
        return text.to_string();
    };
    if dir.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    for sep in ["/", "\\"] {
        out = out.replace(&format!("{}{}", dir.trim_end_matches(['/', '\\']), sep), "");
    }
    out
}

/// Die ONNX-Sitzung des Blockmodells.
pub struct OnnxBlockModel {
    session: Session,
}

impl OnnxBlockModel {
    /// Prueft beide Dateien, laedt die Sitzung mit `threads` CPU-Threads und
    /// prueft Ein- und Ausgaenge: eine fremde ONNX-Datei ist ein Bedienfehler
    /// (`WrongModel`), kein Absturz beim ersten Block.
    pub fn load(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        if !model_path.is_file() {
            return Err(DiarizeError::ModelMissing(model_path.to_path_buf()));
        }
        let data = external_data_path(model_path);
        if !data.is_file() {
            return Err(DiarizeError::ModelMissing(data));
        }
        let load_err = |e: &dyn std::fmt::Display| {
            DiarizeError::Load(format!(
                "{}: {}",
                short(model_path),
                without_dir(&e.to_string(), model_path)
            ))
        };
        // Die Blockgroessen wechseln (zuletzt ein Rest): ein Speichermuster
        // fuer feste Formen bringt hier nichts.
        let session = Session::builder()
            .map_err(|e| load_err(&e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| load_err(&e))?
            .with_intra_threads(threads.clamp(1, 64))
            .map_err(|e| load_err(&e))?
            .with_inter_threads(1)
            .map_err(|e| load_err(&e))?
            .with_memory_pattern(false)
            .map_err(|e| load_err(&e))?
            .commit_from_file(model_path)
            .map_err(|e| load_err(&e))?;
        let have_in: Vec<&str> = session.inputs().iter().map(|o| o.name()).collect();
        let have_out: Vec<&str> = session.outputs().iter().map(|o| o.name()).collect();
        if let Some(missing) = INPUTS
            .iter()
            .find(|n| !have_in.contains(n))
            .or_else(|| OUTPUTS.iter().find(|n| !have_out.contains(n)))
        {
            return Err(DiarizeError::WrongModel(format!("onnx ohne '{missing}'")));
        }
        Ok(Self { session })
    }
}

fn run_err(what: &str, e: &dyn std::fmt::Display) -> DiarizeError {
    DiarizeError::Run(format!("{what}: {e}"))
}

impl BlockModel for OnnxBlockModel {
    fn run(&mut self, input: &BlockInput<'_>) -> Result<BlockOutput, DiarizeError> {
        let features = TensorRef::from_array_view(([1usize, input.mel_rows, MEL_BINS], input.mel))
            .map_err(|e| run_err("Eingang", &e))?;
        // Der erste Block hat noch keinen Zwischenspeicher: Form `[1, 0, 512]`.
        // `from_array_view` verlangt Dimensionen >= 1, ein leerer Tensor kommt
        // vom Zuteiler.
        let cached: SessionInputValue<'_> = if input.cached_frames == 0 {
            Tensor::<f32>::new(&Allocator::default(), [1usize, 0, HIDDEN])
                .map_err(|e| run_err("Zwischenspeicher", &e))?
                .into()
        } else {
            TensorRef::from_array_view(([1usize, input.cached_frames, HIDDEN], input.cached))
                .map_err(|e| run_err("Zwischenspeicher", &e))?
                .into()
        };
        let mask = TensorRef::from_array_view(([1usize, input.mask.len()], input.mask))
            .map_err(|e| run_err("Maske", &e))?;
        let outputs = self
            .session
            .run(ort::inputs![
                "input_features" => features,
                "cached_embeds" => cached,
                "attention_mask" => mask,
            ])
            .map_err(|e| run_err("Block", &e))?;
        let take = |name: &str| -> Result<Vec<f32>, DiarizeError> {
            let (_, data) = outputs[name]
                .try_extract_tensor::<f32>()
                .map_err(|e| run_err(name, &e))?;
            Ok(data.to_vec())
        };
        Ok(BlockOutput {
            logits: take("logits")?,
            chunk_embeds: take("chunk_embeds")?,
            silence: take("silence_embeds")?,
        })
    }
}

/// Nemotron-3-Diarization als [`DiarizeEngine`].
pub struct NemotronOnnxEngine {
    model: OnnxBlockModel,
    front: MelFrontend,
    /// Nach einer Panik im Aufruf ist der Zustand der Sitzung unbekannt.
    broken: bool,
}

impl NemotronOnnxEngine {
    /// `model_path` ist die `.onnx`-Datei; die Gewichte (`.onnx_data`) liegen daneben.
    pub fn load(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        Ok(Self {
            model: OnnxBlockModel::load(model_path, threads)?,
            front: MelFrontend::new(),
            broken: false,
        })
    }

    /// Logits aller Rahmen (`Rahmen * 8`), ohne Schwelle: fuer Messungen und den
    /// Vergleich mit der Referenz.
    pub fn logits(
        &mut self,
        pcm: &[f32],
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<f32>, DiarizeError> {
        guarded_logits(&mut self.model, &self.front, &mut self.broken, pcm, cancel)
    }
}

/// Ein Lauf mit Panikschutz: eine Panik im Aufruf (ONNX Runtime ueber FFI) wird
/// zum Fehler, und der Zustand der Sitzung gilt danach als unbekannt (`broken`),
/// statt die App zu beenden. Ein abgebrochener Lauf liefert keine Teilergebnisse.
fn guarded_logits(
    model: &mut dyn BlockModel,
    front: &MelFrontend,
    broken: &mut bool,
    pcm: &[f32],
    cancel: Option<&CancelToken>,
) -> Result<Vec<f32>, DiarizeError> {
    if *broken {
        return Err(DiarizeError::Run(
            "Diarisierer nach einem Fehler unbrauchbar".into(),
        ));
    }
    let cancelled = || cancel.is_some_and(|c| c.is_cancelled());
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_offline(
            model,
            front,
            pcm,
            CacheConfig::offline(),
            BlockPlan::OFFLINE,
            &cancelled,
        )
    }));
    match result {
        Ok(r) => r,
        Err(_) => {
            *broken = true;
            Err(DiarizeError::Run("Panik im Diarisierer".into()))
        }
    }
}

impl DiarizeEngine for NemotronOnnxEngine {
    /// Der Arbeitspunkt ist immer der Offline-Block des Modells; `preset` gilt nur fuer Sortformer.
    fn raw_turns(
        &mut self,
        pcm: &[f32],
        _preset: Preset,
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<RawTurn>, DiarizeError> {
        Ok(turns_from_logits(&self.logits(pcm, cancel)?))
    }

    fn describe(&self) -> String {
        "onnxruntime-cpu".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in files {
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        dir
    }

    #[test]
    fn the_weights_file_sits_next_to_the_graph_under_a_fixed_name() {
        assert_eq!(
            external_data_path(Path::new("/m/model_quantized.onnx")),
            PathBuf::from("/m/model_quantized.onnx_data")
        );
    }

    #[test]
    fn a_missing_model_is_model_missing_not_a_panic() {
        let dir = dir_with(&[]);
        let path = dir.path().join("model_quantized.onnx");
        match OnnxBlockModel::load(&path, 1) {
            Err(DiarizeError::ModelMissing(p)) => assert_eq!(p, path),
            other => panic!("erwartet ModelMissing, bekam {:?}", other.err()),
        }
    }

    #[test]
    fn missing_onnx_data_is_reported_with_the_data_file() {
        let dir = dir_with(&[("model_quantized.onnx", b"graph")]);
        let path = dir.path().join("model_quantized.onnx");
        match OnnxBlockModel::load(&path, 1) {
            Err(DiarizeError::ModelMissing(p)) => {
                assert!(p.ends_with("model_quantized.onnx_data"), "{}", p.display())
            }
            other => panic!("erwartet ModelMissing, bekam {:?}", other.err()),
        }
        assert!(NemotronOnnxEngine::load(&path, 1).is_err());
    }

    /// Modell, das im Aufruf panikt (stellvertretend fuer einen Fehler in der FFI-Schicht).
    struct Panicker {
        calls: usize,
    }

    impl BlockModel for Panicker {
        fn hidden(&self) -> usize {
            HIDDEN
        }

        fn run(&mut self, _: &BlockInput<'_>) -> Result<BlockOutput, DiarizeError> {
            self.calls += 1;
            panic!("Absturz in der Modellschicht");
        }
    }

    #[test]
    fn a_panic_in_the_model_call_becomes_an_error_and_the_engine_stays_unusable() {
        let (mut model, front, mut broken) = (Panicker { calls: 0 }, MelFrontend::new(), false);
        let pcm = vec![0.05f32; 16_000];
        match guarded_logits(&mut model, &front, &mut broken, &pcm, None) {
            Err(DiarizeError::Run(msg)) => assert!(msg.contains("Panik"), "{msg}"),
            other => panic!("erwartet Run, bekam {:?}", other.err()),
        }
        assert!(broken);
        assert_eq!(model.calls, 1);
        // Danach kein weiterer Aufruf des Modells, sondern ein klarer Fehler.
        match guarded_logits(&mut model, &front, &mut broken, &pcm, None) {
            Err(DiarizeError::Run(msg)) => assert!(msg.contains("unbrauchbar"), "{msg}"),
            other => panic!("erwartet Run, bekam {:?}", other.err()),
        }
        assert_eq!(model.calls, 1, "Modell nach der Panik erneut gerufen");
    }

    #[test]
    fn a_cancelled_run_does_not_break_the_engine() {
        let (mut model, front, mut broken) = (Panicker { calls: 0 }, MelFrontend::new(), false);
        let token = CancelToken::new();
        token.cancel();
        let err = guarded_logits(
            &mut model,
            &front,
            &mut broken,
            &vec![0.05; 16_000],
            Some(&token),
        );
        assert!(matches!(err, Err(DiarizeError::Cancelled)));
        assert!(!broken);
        assert_eq!(model.calls, 0);
    }

    #[test]
    fn error_texts_lose_the_folder_but_keep_the_file_names() {
        let p = Path::new("/tmp/x7/model_quantized.onnx");
        assert_eq!(
            without_dir("Load /tmp/x7/model_quantized.onnx_data failed", p),
            "Load model_quantized.onnx_data failed"
        );
        #[cfg(windows)]
        assert_eq!(
            without_dir(
                r"Load C:\Daten\m\a.onnx failed",
                Path::new(r"C:\Daten\m\a.onnx")
            ),
            "Load a.onnx failed"
        );
        assert_eq!(without_dir("kein Pfad", Path::new("a.onnx")), "kein Pfad");
    }

    /// Ein gueltiges ONNX (Identity, Ein-/Ausgang "x"/"y"), von Hand als
    /// Protobuf geschrieben: ein fremdes Modell neben einer Gewichte-Datei.
    fn identity_graph() -> Vec<u8> {
        fn varint(mut v: u64) -> Vec<u8> {
            let mut out = Vec::new();
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(b);
                    return out;
                }
                out.push(b | 0x80);
            }
        }
        fn field(tag: u64, payload: &[u8]) -> Vec<u8> {
            let mut out = varint(tag << 3 | 2);
            out.extend(varint(payload.len() as u64));
            out.extend_from_slice(payload);
            out
        }
        fn number(tag: u64, v: u64) -> Vec<u8> {
            let mut out = varint(tag << 3);
            out.extend(varint(v));
            out
        }
        let tensor = |name: &str| {
            let dim = field(1, &number(1, 1)); // Dimension { dim_value: 1 }
            let shape = field(2, &dim);
            let mut tensor_type = number(1, 1); // elem_type FLOAT
            tensor_type.extend(shape);
            let type_proto = field(1, &tensor_type);
            let mut info = field(1, name.as_bytes());
            info.extend(field(2, &type_proto));
            info
        };
        let mut node = field(1, b"x");
        node.extend(field(2, b"y"));
        node.extend(field(4, b"Identity"));
        let mut graph = field(1, &node);
        graph.extend(field(2, b"g"));
        graph.extend(field(11, &tensor("x")));
        graph.extend(field(12, &tensor("y")));
        let mut model = number(1, 8); // ir_version
        model.extend(field(7, &graph));
        let mut opset = field(1, b"");
        opset.extend(number(2, 13));
        model.extend(field(8, &opset));
        model
    }

    #[test]
    fn a_valid_onnx_with_other_inputs_is_wrong_model_not_a_crash_at_the_first_block() {
        let dir = dir_with(&[
            ("model_quantized.onnx", &identity_graph()),
            ("model_quantized.onnx_data", b"xx"),
        ]);
        let path = dir.path().join("model_quantized.onnx");
        match OnnxBlockModel::load(&path, 1) {
            Err(DiarizeError::WrongModel(what)) => {
                assert!(
                    what.contains("input_features") || what.contains("cached_embeds"),
                    "{what}"
                )
            }
            other => panic!("erwartet WrongModel, bekam {:?}", other.err()),
        }
    }

    #[test]
    fn a_file_that_is_no_onnx_graph_is_a_load_error_not_a_panic() {
        let dir = dir_with(&[
            ("model_quantized.onnx", b"das ist kein ONNX"),
            ("model_quantized.onnx_data", b"xx"),
        ]);
        let path = dir.path().join("model_quantized.onnx");
        match OnnxBlockModel::load(&path, 1) {
            Err(DiarizeError::Load(msg)) => {
                assert!(msg.contains("model_quantized.onnx"), "{msg}");
                // Kein voller Pfad im Fehlertext.
                assert!(!msg.contains(dir.path().to_str().unwrap()), "{msg}");
            }
            other => panic!("erwartet Load, bekam {:?}", other.err()),
        }
    }
}

/// Tests mit dem echten Modell. Sie laufen, wenn `LVA_NEMOTRON_MODEL` auf die
/// `.onnx`-Datei zeigt (die `.onnx_data` liegt daneben), und melden sonst
/// "uebersprungen". Die Referenz stammt aus
/// `koordination/nvidia-speech/tools/make_reference.py`.
#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::managers::meetings::diarize::nemotron::blocks::turns_from_logits;
    use crate::managers::meetings::diarize::nemotron::NUM_SPEAKERS;
    use crate::managers::meetings::diarize::DiarizeError;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::time::Instant;

    const PIPELINE_REF: &str =
        include_str!("../../../../../tests/fixtures/nemotron/pipeline_ref.json");
    const CLIP: &[u8] = include_bytes!("../../../../../tests/fixtures/nemotron/mel_clip.i16");

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn model_path() -> Option<PathBuf> {
        let path = PathBuf::from(std::env::var_os("LVA_NEMOTRON_MODEL")?);
        (path.is_file() && external_data_path(&path).is_file()).then_some(path)
    }

    fn skip(why: &str) {
        eprintln!("uebersprungen: {why}");
    }

    fn read_wav(path: &Path) -> Option<Vec<f32>> {
        let mut reader = hound::WavReader::open(path).ok()?;
        let spec = reader.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 16_000, 16)
        );
        Some(
            reader
                .samples::<i16>()
                .map(|s| f32::from(s.unwrap()) / 32768.0)
                .collect(),
        )
    }

    fn clip() -> Vec<f32> {
        CLIP.chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect()
    }

    fn sha256(path: &Path) -> String {
        format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
    }

    fn threads() -> usize {
        std::env::var("LVA_NEMOTRON_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4)
    }

    /// Aktivitaet je Rahmen (10 ms) als Bitmaske ueber die 8 Kanaele.
    fn masks_from_turns(turns: impl Iterator<Item = (u64, u64, u32)>, frames: usize) -> Vec<u8> {
        let mut m = vec![0u8; frames];
        for (t0, t1, speaker) in turns {
            for f in (t0 / 10) as usize..((t1 / 10) as usize).min(frames) {
                m[f] |= 1 << (speaker - 1);
            }
        }
        m
    }

    #[test]
    fn the_model_files_are_the_pinned_catalog_files() {
        let Some(path) = model_path() else {
            return skip("LVA_NEMOTRON_MODEL nicht gesetzt");
        };
        let entry = crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
            .into_iter()
            .find(|e| e.id == crate::managers::meetings::diarize::nemotron::MODEL_ID)
            .unwrap();
        let dir = path.parent().unwrap();
        for f in &entry.files {
            assert_eq!(
                sha256(&dir.join(&f.filename)),
                f.sha256.clone().unwrap(),
                "{}",
                f.filename
            );
        }
    }

    /// Pruefaudio nach der Vorschrift der Referenz (`make_reference.py::render`):
    /// Teile hintereinander, Ebenen eines Teils abtastwertweise addiert.
    /// `None`, wenn eine der WAVs fehlt (sie sind nicht eingecheckt).
    fn render(recipe: &Value) -> Option<Vec<f32>> {
        let mut out = Vec::new();
        for part in recipe.as_array().unwrap() {
            let mut layers: Vec<(usize, Vec<f32>)> = Vec::new();
            for layer in part["layers"].as_array().unwrap() {
                let mut x = read_wav(&fixture(layer["file"].as_str().unwrap()))?;
                if layer["reverse"].as_bool().unwrap_or(false) {
                    x.reverse();
                }
                if let Some(take) = layer["take"].as_u64() {
                    x.truncate(take as usize);
                }
                let gain = layer["gain"].as_f64().unwrap_or(1.0) as f32;
                x.iter_mut().for_each(|v| *v *= gain);
                layers.push((layer["offset"].as_u64().unwrap_or(0) as usize, x));
            }
            let len = layers.iter().map(|(o, x)| o + x.len()).max().unwrap();
            let mut mix = vec![0.0f32; len];
            for (offset, x) in layers {
                for (m, v) in mix[offset..].iter_mut().zip(x) {
                    *m += v;
                }
            }
            out.extend(mix);
        }
        Some(out)
    }

    /// Die Hauptpruefung: 73 s und 101 s Sprache (hintereinander bzw. ueberlagert
    /// und umgekehrt, drei und vier Bloecke, zwei bzw. vier Sprecher) gegen die
    /// Turns der Referenz, Rahmen fuer Rahmen.
    #[test]
    fn turns_agree_with_the_reference_frame_by_frame_on_long_audio() {
        let Some(path) = model_path() else {
            return skip("LVA_NEMOTRON_MODEL nicht gesetzt");
        };
        let reference: Value = serde_json::from_str(PIPELINE_REF).unwrap();
        let load_start = Instant::now();
        let mut engine = NemotronOnnxEngine::load(&path, threads()).expect("Modell laedt");
        eprintln!(
            "Laden des Modells (Sitzung, Graph und Gewichte): {:.2} s",
            load_start.elapsed().as_secs_f64()
        );
        let mut ran = 0;
        for case in reference["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let Some(pcm) = render(&case["recipe"]) else {
                skip(&format!(
                    "{name}: Audio fehlt (nicht eingecheckt, siehe make-fixtures)"
                ));
                continue;
            };
            assert_eq!(
                pcm.len() as u64,
                case["audio"]["samples"].as_u64().unwrap(),
                "{name}"
            );
            let frames = case["audio"]["frames"].as_u64().unwrap() as usize;

            let start = Instant::now();
            let logits = engine.logits(&pcm, None).expect("Lauf");
            let took = start.elapsed().as_secs_f64();
            let seconds = pcm.len() as f64 / 16_000.0;
            eprintln!(
                "[{name}] Laufzeit: {took:.2} s fuer {seconds:.1} s Audio mit {} Threads = {:.2} s je Minute Audio",
                threads(),
                took / (seconds / 60.0)
            );
            assert_eq!(logits.len(), frames * NUM_SPEAKERS, "{name}");
            let mel_start = Instant::now();
            let mel = MelFrontend::new().compute(&pcm);
            eprintln!(
                "[{name}] davon Mel-Merkmale allein: {:.2} s ({} Werte)",
                mel_start.elapsed().as_secs_f64(),
                mel.len()
            );
            eprintln!(
                "[{name}] zum Vergleich: Referenz mit der Standardauswahl von torch.topk weicht ab: {}",
                case["torch_default_topk"]
            );

            if let Some(dir) = std::env::var_os("LVA_NEMOTRON_DUMP") {
                let bytes: Vec<u8> = logits.iter().flat_map(|v| v.to_le_bytes()).collect();
                std::fs::write(
                    Path::new(&dir).join(format!("rust_logits_{name}.f32")),
                    bytes,
                )
                .unwrap();
            }

            let got = masks_from_turns(
                turns_from_logits(&logits)
                    .into_iter()
                    .map(|t| (t.t0_ms as u64, t.t1_ms as u64, t.speaker_id as u32)),
                frames,
            );
            let want = masks_from_turns(
                case["turns"].as_array().unwrap().iter().map(|t| {
                    (
                        t["start_ms"].as_u64().unwrap(),
                        t["end_ms"].as_u64().unwrap(),
                        t["speaker"].as_u64().unwrap() as u32,
                    )
                }),
                frames,
            );
            let equal = got.iter().zip(&want).filter(|(a, b)| a == b).count();
            let cells = got
                .iter()
                .zip(&want)
                .map(|(a, b)| 8 - (a ^ b).count_ones() as usize)
                .sum::<usize>();
            let agreement = equal as f64 / frames as f64;
            eprintln!(
                "[{name}] Rahmen gleich: {equal} von {frames} = {:.3} %, Zellen (Rahmen x Sprecher) gleich: {:.3} %",
                agreement * 100.0,
                cells as f64 / (frames * 8) as f64 * 100.0
            );
            for (c, active) in case["active_frames_per_speaker"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let mine = got.iter().filter(|m| **m & (1 << c) != 0).count();
                if mine > 0 || active.as_u64() != Some(0) {
                    eprintln!(
                        "[{name}] Sprecher {}: {mine} aktive Rahmen, Referenz {active}",
                        c + 1
                    );
                }
            }
            assert!(
                agreement >= 0.99,
                "{name}: Rahmenuebereinstimmung {agreement:.4}"
            );
            ran += 1;
        }
        eprintln!("{ran} Pruefaudios gegen die Referenz verglichen");
    }

    #[test]
    fn a_truncated_weights_file_is_a_load_error_not_a_crash() {
        let Some(path) = model_path() else {
            return skip("LVA_NEMOTRON_MODEL nicht gesetzt");
        };
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("model_quantized.onnx");
        std::fs::copy(&path, &graph).unwrap();
        let data = std::fs::read(external_data_path(&path)).unwrap();
        std::fs::write(external_data_path(&graph), &data[..data.len() / 2]).unwrap();
        match OnnxBlockModel::load(&graph, 1) {
            Err(DiarizeError::Load(msg)) => {
                assert!(!msg.contains(dir.path().to_str().unwrap()), "{msg}");
            }
            other => panic!("erwartet Load, bekam {:?}", other.err()),
        }
    }

    #[test]
    fn cancel_in_the_middle_of_a_long_run_returns_cancelled_and_the_engine_survives() {
        let Some(path) = model_path() else {
            return skip("LVA_NEMOTRON_MODEL nicht gesetzt");
        };
        // 100 s Wiederholung des Ausschnitts: vier Bloecke, mit einem Thread langsam genug.
        let one = clip();
        let pcm: Vec<f32> = one.iter().copied().cycle().take(16_000 * 100).collect();
        let mut engine = NemotronOnnxEngine::load(&path, 1).expect("Modell laedt");
        let token = CancelToken::new();
        let canceller = token.clone();
        let start = Instant::now();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                canceller.cancel();
            });
            engine.logits(&pcm, Some(&token))
        });
        let took = start.elapsed();
        assert!(
            matches!(result, Err(DiarizeError::Cancelled)),
            "{:?}",
            result.err()
        );
        eprintln!(
            "Abbruch nach {:.2} s (Ende des laufenden Blocks)",
            took.as_secs_f64()
        );
        // Kein Teilergebnis, und die Engine bleibt benutzbar.
        let again = engine
            .logits(&one, None)
            .expect("nach Abbruch weiter nutzbar");
        assert_eq!(again.len(), (one.len() / 160) * NUM_SPEAKERS);
    }

    #[test]
    fn very_short_and_empty_audio_never_panic_and_a_non_finite_sample_is_an_error() {
        let Some(path) = model_path() else {
            return skip("LVA_NEMOTRON_MODEL nicht gesetzt");
        };
        let mut engine = NemotronOnnxEngine::load(&path, 1).expect("Modell laedt");
        assert!(engine.logits(&[], None).unwrap().is_empty());
        let one = clip();
        for samples in [1usize, 160, 1_000, 16_000] {
            let logits = engine.logits(&one[..samples], None).expect("kurzes Audio");
            assert_eq!(logits.len(), (samples / 160) * NUM_SPEAKERS, "{samples}");
            assert!(logits.iter().all(|v| v.is_finite()));
        }
        let mut bad = one.clone();
        bad[4_000] = f32::NAN;
        let err = engine.logits(&bad, None);
        assert!(
            matches!(err, Err(DiarizeError::Run(_))),
            "NaN im Eingang muss ein Fehler sein, kein Ergebnis: {:?}",
            err.map(|l| l.len())
        );
        // Kein Absturz, nicht kaputt: der naechste saubere Lauf geht.
        assert!(engine.logits(&one, None).is_ok());
    }
}
