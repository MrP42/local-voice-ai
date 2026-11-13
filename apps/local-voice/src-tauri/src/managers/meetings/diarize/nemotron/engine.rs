//! `NemotronOnnxEngine`: Nemotron-3-Diarization ueber ONNX Runtime (CPU),
//! Paket N1. Die Sitzung rechnet je Aufruf einen Block ([`BlockModel`]); die
//! Schleife drumherum steht in [`super::blocks`].
//!
//! Das Modell besteht aus zwei Dateien, die nebeneinander liegen muessen: dem
//! Graphen `model_quantized.onnx` und den Gewichten `model_quantized.onnx_data`
//! (ONNX "external data", der Graph nennt die Datei beim Namen). Fehlt die zweite
//! oder ist sie abgeschnitten, meldet das Laden einen Fehler, nie eine Panik.
//! Liegt das Modell an seinem Katalogplatz, laedt es eine geschuetzte Generation:
//! beide Dateien werden je ueber EIN Handle (ein verankerter Ordner, kein Pfad)
//! vollstaendig in den Speicher gelesen und dort nach Groesse und SHA-256 gegen den
//! Katalog geprueft; die ONNX-Sitzung entsteht aus genau diesen Bytes
//! (`commit_from_memory` plus Gewichte als externe Initialisierer im Speicher). ONNX
//! Runtime oeffnet danach keinen Pfad mehr, ein Tausch der Dateien nach der Pruefung
//! aendert nichts an der Sitzung ([`OnnxBlockModel::load_verified`]). Ein Pfad von
//! Hand (kein Katalogplatz) bleibt ungeprueft und wird wie bisher von ONNX Runtime
//! selbst geoeffnet.
//!
//! Wie bei Sortformer (`diarize::engine`): ein Diarisierer im Prozess (der
//! `ExclusiveSlot` im umgebenden `Diarizer`), RAM-Start-Gate davor, eine Panik im
//! Aufruf macht die Engine unbrauchbar statt die App zu beenden.

use std::borrow::Cow;
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
use crate::managers::meetings::diarize::{
    catalog_file_sizes, catalog_place, DiarizeEngine, DiarizeError, Preset,
};
use crate::managers::model::download::anchor::{self, Anchor, Check, Point, Wanted};
use crate::managers::model::download::safe;

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

/// Liegt `model_path` an seinem Katalogplatz (`.../diarization/<id>/<graph>`),
/// muessen beide Dateien des Eintrags genau die Groesse aus dem Katalog haben.
/// Ein Pfad von Hand (anderer Ordner oder Name) ist kein Katalogmodell und wird
/// hier nicht geprueft.
///
/// Reichweite, ehrlich: das ist nur `stat` ueber Pfade, kein Hash, und es ist von
/// dem, was danach geladen wird, getrennt. Es taugt fuer eine billige, fruehe
/// Meldung (etwa vor dem Lesen von 120 MB), nicht als Schutz: der Schutz ist
/// [`OnnxBlockModel::load_verified`], das beim Laden Groesse UND SHA-256 ueber
/// dieselben Bytes prueft, aus denen die Sitzung entsteht.
pub fn check_catalog_sizes(model_path: &Path) -> Result<(), DiarizeError> {
    let Some(files) = catalog_file_sizes(model_path) else {
        return Ok(());
    };
    for (path, expected) in files {
        let actual = std::fs::metadata(&path)
            .map_err(|_| DiarizeError::ModelMissing(path.clone()))?
            .len();
        if actual != expected {
            return Err(DiarizeError::Load(size_message(&path, actual, expected)));
        }
    }
    Ok(())
}

fn size_message(path: &Path, actual: u64, expected: u64) -> String {
    format!(
        "{}: Groesse {actual} statt {expected} laut Katalog (unvollstaendig oder veraendert; im Modellmanager neu bereitstellen)",
        short(path)
    )
}

/// Der Puffer mit den geprueften Gewichten, aus dem die Sitzung entsteht (externe
/// Initialisierer im Speicher). ONNX Runtime kopiert die Daten beim Sitzungsbau. Der
/// Puffer bleibt trotzdem bis zum Ende der Sitzung liegen (vorsichtshalber, falls eine
/// Referenz bleibt) und haelt damit zusaetzlich rund 121 MB (die Gewichte von
/// Nemotron) im Speicher. Er gehoert diesem Wert allein, wird nie veraendert und wird
/// erst nach der Sitzung freigegeben (siehe die Feldreihenfolge in [`OnnxBlockModel`]).
struct Pinned {
    ptr: *mut [u8],
}

// SAFETY: der Puffer gehoert dem Wert allein, wird nur gelesen und erst im `Drop`
// freigegeben; ein Zeiger auf unveraenderliche Bytes darf zwischen Threads wandern.
unsafe impl Send for Pinned {}
unsafe impl Sync for Pinned {}

impl Pinned {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            ptr: Box::into_raw(bytes.into_boxed_slice()),
        }
    }

    /// Die Bytes fuer ONNX Runtime. `'static` ist gelogen im Typ, wahr im Ablauf: die
    /// Sitzung, die sie haelt, steht in [`OnnxBlockModel`] vor diesem Wert und wird
    /// zuerst freigegeben; scheitert der Bau der Sitzung, gibt es keine.
    fn bytes(&self) -> &'static [u8] {
        // SAFETY: `ptr` stammt aus `Box::into_raw` und bleibt bis `drop` gueltig.
        unsafe { &*self.ptr }
    }
}

impl Drop for Pinned {
    fn drop(&mut self) {
        // SAFETY: `ptr` stammt aus `Box::into_raw` in `new` und wird nur hier freigegeben.
        drop(unsafe { Box::from_raw(self.ptr) });
    }
}

/// Die ONNX-Sitzung des Blockmodells.
pub struct OnnxBlockModel {
    /// Zuerst: Felder werden in dieser Reihenfolge freigegeben, die Sitzung also vor
    /// den Gewichten, die sie referenziert.
    session: Session,
    _weights: Option<Pinned>,
}

/// Die Sitzung aus dem Graphen und den Gewichten im Speicher. Kein Pfad wird
/// geoeffnet: ONNX Runtime sieht nur diese Bytes. Der Name der Gewichte ist der
/// Name, unter dem der Graph sie als externe Daten nennt.
fn session_from_memory(
    graph: &[u8],
    weights_name: &str,
    weights: &'static [u8],
    threads: usize,
    load_err: &dyn Fn(&dyn std::fmt::Display) -> DiarizeError,
) -> Result<Session, DiarizeError> {
    let mut builder = session_builder(threads, load_err)?
        .with_external_initializer_file_in_memory(weights_name, Cow::Borrowed(weights))
        .map_err(|e| load_err(&e))?;
    builder.commit_from_memory(graph).map_err(|e| load_err(&e))
}

fn session_builder(
    threads: usize,
    load_err: &dyn Fn(&dyn std::fmt::Display) -> DiarizeError,
) -> Result<ort::session::builder::SessionBuilder, DiarizeError> {
    // Die Blockgroessen wechseln (zuletzt ein Rest): ein Speichermuster
    // fuer feste Formen bringt hier nichts.
    Session::builder()
        .map_err(|e| load_err(&e))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| load_err(&e))?
        .with_intra_threads(threads.clamp(1, 64))
        .map_err(|e| load_err(&e))?
        .with_inter_threads(1)
        .map_err(|e| load_err(&e))?
        .with_memory_pattern(false)
        .map_err(|e| load_err(&e))
}

/// Die Namen der Ein- und Ausgaenge muessen die des Nemotron-Modells sein: eine
/// fremde ONNX-Datei ist ein Bedienfehler (`WrongModel`), kein Absturz beim ersten Block.
fn check_io(session: &Session) -> Result<(), DiarizeError> {
    let have_in: Vec<&str> = session.inputs().iter().map(|o| o.name()).collect();
    let have_out: Vec<&str> = session.outputs().iter().map(|o| o.name()).collect();
    match INPUTS
        .iter()
        .find(|n| !have_in.contains(n))
        .or_else(|| OUTPUTS.iter().find(|n| !have_out.contains(n)))
    {
        Some(missing) => Err(DiarizeError::WrongModel(format!("onnx ohne '{missing}'"))),
        None => Ok(()),
    }
}

impl OnnxBlockModel {
    /// Laedt das Modell. Liegt `model_path` an seinem Katalogplatz, ist es eine
    /// geschuetzte Generation ([`Self::load_verified`]): beide Dateien werden ueber
    /// ein geoeffnetes Ordner-Handle gelesen, gegen Groesse und SHA-256 des Katalogs
    /// geprueft und die Sitzung aus genau diesen Bytes gebaut. Ein Pfad von Hand
    /// bleibt ungeprueft und wird von ONNX Runtime selbst geoeffnet.
    ///
    /// Fehlende Dateien sind `ModelMissing`, eine fremde ONNX-Datei `WrongModel`.
    pub fn load(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        if !model_path.is_file() {
            return Err(DiarizeError::ModelMissing(model_path.to_path_buf()));
        }
        let data = external_data_path(model_path);
        if !data.is_file() {
            return Err(DiarizeError::ModelMissing(data));
        }
        match catalog_place(model_path) {
            Some(place) => Self::load_catalog(model_path, &place, threads),
            None => Self::load_by_path(model_path, threads),
        }
    }

    fn load_err(model_path: &Path) -> impl Fn(&dyn std::fmt::Display) -> DiarizeError + '_ {
        move |e| {
            DiarizeError::Load(format!(
                "{}: {}",
                short(model_path),
                without_dir(&e.to_string(), model_path)
            ))
        }
    }

    /// Ein Pfad von Hand: ONNX Runtime oeffnet Graph und Gewichte selbst.
    fn load_by_path(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        let load_err = Self::load_err(model_path);
        let session = session_builder(threads, &load_err)?
            .commit_from_file(model_path)
            .map_err(|e| load_err(&e))?;
        check_io(&session)?;
        Ok(Self {
            session,
            _weights: None,
        })
    }

    /// Das Modell an seinem Katalogplatz.
    fn load_catalog(
        model_path: &Path,
        place: &crate::managers::meetings::diarize::CatalogPlace,
        threads: usize,
    ) -> Result<Self, DiarizeError> {
        let [graph, weights] = place.entry.files.as_slice() else {
            return Err(DiarizeError::Load(format!(
                "{}: Katalogeintrag ohne Graph und Gewichte",
                short(model_path)
            )));
        };
        if graph.sha256.is_none() || weights.sha256.is_none() {
            return Err(DiarizeError::Load(format!(
                "{}: Katalogeintrag ohne SHA-256 (geladen wird nur, was sich pruefen laesst)",
                short(model_path)
            )));
        }
        if short(&external_data_path(model_path)) != weights.filename {
            return Err(DiarizeError::Load(format!(
                "{}: Gewichte heissen im Katalog anders als neben dem Graphen erwartet",
                short(model_path)
            )));
        }
        let parts: Vec<&str> = place.parts.iter().map(String::as_str).collect();
        let anchor = match Anchor::open(&place.root, &parts, false) {
            Ok(Some(anchor)) => anchor,
            Ok(None) => return Err(DiarizeError::ModelMissing(model_path.to_path_buf())),
            Err(e) if safe::is_refusal(&e) => {
                return Err(DiarizeError::Load(format!(
                    "{}: Modellordner ist ein Link oder kein Ordner (im Modellmanager neu bereitstellen)",
                    short(model_path)
                )))
            }
            Err(e) => {
                return Err(DiarizeError::Load(format!("{}: {e}", short(model_path))));
            }
        };
        fn wanted(f: &crate::catalog::TtsCatalogFile) -> Wanted<'_> {
            Wanted {
                name: &f.filename,
                size: f.size_bytes,
                sha256: f.sha256.as_deref(),
            }
        }
        Self::load_verified(
            &anchor,
            [wanted(graph), wanted(weights)],
            &place.dir,
            threads,
        )
    }

    /// Die geschuetzte Generation: liest beide Dateien (Graph, Gewichte) ueber je EIN
    /// Handle des verankerten Ordners vollstaendig in den Speicher, prueft Groesse
    /// und SHA-256 gegen `wanted` ueber genau diese Bytes (`anchor::check_set`: alle
    /// Handles zugleich offen, danach Name gegen Handle verglichen) und baut die
    /// Sitzung aus dem Speicher. Eine Datei wird nie ein zweites Mal ueber einen Pfad
    /// geoeffnet; ein Tausch nach dem Lesen aendert die Sitzung nicht.
    ///
    /// `dir` dient nur den Fehlertexten (Dateinamen, kein Pfad im Text).
    pub(crate) fn load_verified(
        anchor: &Anchor,
        wanted: [Wanted<'_>; 2],
        dir: &Path,
        threads: usize,
    ) -> Result<Self, DiarizeError> {
        let [graph_want, weights_want] = &wanted;
        let graph_path = dir.join(graph_want.name);
        let load_err = Self::load_err(&graph_path);

        // Der Speicher wird VOR dem Lesen reserviert: ein Fehlschlag ist ein Ladefehler
        // (voller Arbeitsspeicher), kein Abbruch des Prozesses mitten im Lesen. Der Sink
        // schreibt nie ueber die Reserve hinaus (eine gewachsene Datei liefert ein Byte
        // mehr; sie gilt dann ohnehin als veraendert).
        let mut bytes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
        for (buffer, w) in bytes.iter_mut().zip(&wanted) {
            let reserved = usize::try_from(w.size)
                .ok()
                .and_then(|size| buffer.try_reserve_exact(size).ok());
            if reserved.is_none() {
                return Err(DiarizeError::Load(format!(
                    "{}: nicht genug Arbeitsspeicher, um die Datei zu laden ({} MB)",
                    w.name,
                    w.size.div_ceil(1_000_000)
                )));
            }
        }
        let mut sink = |i: usize, block: &[u8]| {
            let room = bytes[i].capacity() - bytes[i].len();
            bytes[i].extend_from_slice(&block[..block.len().min(room)]);
        };
        let states = anchor::check_set(anchor, &wanted, &|| false, Some(&mut sink))
            .map_err(|e| {
                if safe::is_refusal(&e) {
                    DiarizeError::Load(format!(
                        "{}: Modelldatei ist ein Link oder keine normale Datei (im Modellmanager neu bereitstellen)",
                        short(&graph_path)
                    ))
                } else {
                    DiarizeError::Load(format!("{}: {e}", short(&graph_path)))
                }
            })?
            .ok_or(DiarizeError::Cancelled)?;
        // Reihenfolge der Meldung: fehlt etwas, dann Groesse, dann Inhalt, dann Aenderung
        // waehrend des Ladens (wie vor der geschuetzten Generation: erst die Groessen).
        let rank = |c: &Check| match c {
            Check::Valid => 5,
            Check::Missing => 0,
            Check::Size(_) => 1,
            Check::Hash => 2,
            Check::Changed => 3,
        };
        if let Some((w, state)) = wanted
            .iter()
            .zip(&states)
            .filter(|(_, c)| **c != Check::Valid)
            .min_by_key(|(_, c)| rank(c))
        {
            let path = dir.join(w.name);
            return Err(match *state {
                Check::Missing => DiarizeError::ModelMissing(path),
                Check::Size(actual) => DiarizeError::Load(size_message(&path, actual, w.size)),
                Check::Hash => DiarizeError::Load(format!(
                    "{}: SHA-256 stimmt nicht mit dem Katalog ueberein (Inhalt veraendert; im Modellmanager neu bereitstellen)",
                    short(&path)
                )),
                Check::Changed | Check::Valid => DiarizeError::Load(format!(
                    "{}: waehrend des Ladens veraendert (im Modellmanager neu bereitstellen)",
                    short(&path)
                )),
            });
        }
        anchor::fire(anchor.root(), Point::Loaded);

        let [graph_bytes, weights_bytes] = bytes;
        let weights = Pinned::new(weights_bytes);
        let session = session_from_memory(
            &graph_bytes,
            weights_want.name,
            weights.bytes(),
            threads,
            &load_err,
        )?;
        check_io(&session)?;
        Ok(Self {
            session,
            _weights: Some(weights),
        })
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
    use crate::managers::model::download::linktest::swap_must_have_happened;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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

    /// Beide Dateien des Katalogeintrags an ihrem Platz unter `root`, mit den
    /// angegebenen Groessen (duenn besetzte Dateien, kein Inhalt noetig).
    fn catalog_layout(root: &Path, sizes: [u64; 2]) -> Vec<PathBuf> {
        let files =
            crate::managers::meetings::diarize::catalog_model_files(root, super::super::MODEL_ID)
                .expect("Katalogeintrag");
        std::fs::create_dir_all(files[0].parent().unwrap()).unwrap();
        for (path, size) in files.iter().zip(sizes) {
            std::fs::File::create(path).unwrap().set_len(size).unwrap();
        }
        files
    }

    fn catalog_sizes() -> [u64; 2] {
        let entry = crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
            .into_iter()
            .find(|e| e.id == super::super::MODEL_ID)
            .unwrap();
        [entry.files[0].size_bytes, entry.files[1].size_bytes]
    }

    /// Am Katalogplatz mit der falschen Groesse (abgeschnitten, ein halber Austausch):
    /// das Laden meldet es, bevor ONNX Runtime etwas oeffnet; der Text nennt den
    /// Dateinamen, keinen Pfad.
    #[test]
    fn wrong_sizes_at_the_catalog_place_fail_the_load_naming_the_file() {
        let [graph, weights] = catalog_sizes();
        for (sizes, bad) in [
            ([graph, weights - 1], "model_quantized.onnx_data"),
            ([graph + 1, weights], "model_quantized.onnx"),
            ([0, weights], "model_quantized.onnx"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let files = catalog_layout(dir.path(), sizes);
            match OnnxBlockModel::load(&files[0], 1) {
                Err(DiarizeError::Load(msg)) => {
                    assert!(msg.starts_with(bad), "{msg}");
                    assert!(msg.contains("laut Katalog"), "{msg}");
                    assert!(
                        !msg.contains(dir.path().to_str().unwrap()),
                        "Pfad im Text: {msg}"
                    );
                }
                other => panic!("erwartet Load, bekam {:?}", other.err()),
            }
        }
    }

    #[test]
    fn the_right_sizes_pass_the_check_and_a_hand_placed_model_is_not_checked() {
        let dir = tempfile::tempdir().unwrap();
        let files = catalog_layout(dir.path(), catalog_sizes());
        check_catalog_sizes(&files[0]).expect("Groessen laut Katalog");

        // Von Hand abgelegt (kein Katalogplatz): keine Groessenpruefung, auch bei
        // beliebiger Groesse.
        let manual = dir_with(&[
            ("model_quantized.onnx", b"graph"),
            ("model_quantized.onnx_data", b"daten"),
        ]);
        check_catalog_sizes(&manual.path().join("model_quantized.onnx"))
            .expect("kein Katalogmodell");
    }

    /// Die zweite Datei fehlt am Katalogplatz: weiter `ModelMissing` mit dieser
    /// Datei (die Pruefung davor), keine Groessenmeldung.
    #[test]
    fn a_missing_weights_file_at_the_catalog_place_is_still_model_missing() {
        let dir = tempfile::tempdir().unwrap();
        let files = catalog_layout(dir.path(), catalog_sizes());
        std::fs::remove_file(&files[1]).unwrap();
        match OnnxBlockModel::load(&files[0], 1) {
            Err(DiarizeError::ModelMissing(p)) => assert_eq!(p, files[1]),
            other => panic!("erwartet ModelMissing, bekam {:?}", other.err()),
        }
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

    // ---- Die geschuetzte Generation: ONNX Runtime bekommt genau die geprueften Bytes ----
    //
    // Ein winziges ONNX von Hand (Ein-/Ausgaenge wie Nemotron, `logits = input_features + w`),
    // dessen Gewicht `w` als externe Daten in `w.bin` liegt: so laesst sich zeigen, womit die
    // Sitzung rechnet, ohne das 120-MB-Modell.

    fn pb_varint(mut v: u64) -> Vec<u8> {
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

    fn pb_bytes(tag: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = pb_varint(tag << 3 | 2);
        out.extend(pb_varint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn pb_num(tag: u64, v: u64) -> Vec<u8> {
        let mut out = pb_varint(tag << 3);
        out.extend(pb_varint(v));
        out
    }

    /// Ein Ein- oder Ausgang `name`: float, Form `[1]`.
    fn pb_value_info(name: &str) -> Vec<u8> {
        let shape = pb_bytes(2, &pb_bytes(1, &pb_num(1, 1)));
        let mut tensor_type = pb_num(1, 1);
        tensor_type.extend(shape);
        let mut info = pb_bytes(1, name.as_bytes());
        info.extend(pb_bytes(2, &pb_bytes(1, &tensor_type)));
        info
    }

    fn pb_node(inputs: &[&str], output: &str, op: &str) -> Vec<u8> {
        let mut node = Vec::new();
        for input in inputs {
            node.extend(pb_bytes(1, input.as_bytes()));
        }
        node.extend(pb_bytes(2, output.as_bytes()));
        node.extend(pb_bytes(4, op.as_bytes()));
        node
    }

    const WEIGHTS_NAME: &str = "w.bin";
    const GRAPH_NAME: &str = "g.onnx";

    /// Die Nemotron-Namen, `logits = input_features + w` mit `w` (ein float) als externe
    /// Daten in `weights_name`; die beiden anderen Ausgaenge geben ihre Eingaenge zurueck.
    fn tiny_graph(weights_name: &str) -> Vec<u8> {
        let mut w = pb_num(1, 1); // dims [1]
        w.extend(pb_num(2, 1)); // FLOAT
        w.extend(pb_bytes(8, b"w"));
        for (key, value) in [("location", weights_name), ("offset", "0"), ("length", "4")] {
            let mut entry = pb_bytes(1, key.as_bytes());
            entry.extend(pb_bytes(2, value.as_bytes()));
            w.extend(pb_bytes(13, &entry));
        }
        w.extend(pb_num(14, 1)); // data_location EXTERNAL
        let mut graph = pb_bytes(1, &pb_node(&["input_features", "w"], "logits", "Add"));
        graph.extend(pb_bytes(
            1,
            &pb_node(&["cached_embeds"], "chunk_embeds", "Identity"),
        ));
        graph.extend(pb_bytes(
            1,
            &pb_node(&["attention_mask"], "silence_embeds", "Identity"),
        ));
        graph.extend(pb_bytes(2, b"g"));
        graph.extend(pb_bytes(5, &w));
        for name in INPUTS {
            graph.extend(pb_bytes(11, &pb_value_info(name)));
        }
        for name in OUTPUTS {
            graph.extend(pb_bytes(12, &pb_value_info(name)));
        }
        let mut model = pb_num(1, 8); // ir_version
        model.extend(pb_bytes(7, &graph));
        let mut opset = pb_bytes(1, b"");
        opset.extend(pb_num(2, 13));
        model.extend(pb_bytes(8, &opset));
        model
    }

    fn sha_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Ein Ordner mit `g.onnx` und `w.bin` (Gewicht `weight`), ein Anker darauf, und was
    /// der Katalog von den beiden Dateien verlangt.
    struct Tiny {
        dir: tempfile::TempDir,
        anchor: Anchor,
        graph: Vec<u8>,
        weights: Vec<u8>,
    }

    impl Tiny {
        fn new(weight: f32) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let graph = tiny_graph(WEIGHTS_NAME);
            let weights = weight.to_le_bytes().to_vec();
            std::fs::write(dir.path().join(GRAPH_NAME), &graph).unwrap();
            std::fs::write(dir.path().join(WEIGHTS_NAME), &weights).unwrap();
            let anchor = Anchor::open(dir.path(), &[], false).unwrap().unwrap();
            Self {
                dir,
                anchor,
                graph,
                weights,
            }
        }

        fn weights_path(&self) -> PathBuf {
            self.dir.path().join(WEIGHTS_NAME)
        }

        fn graph_path(&self) -> PathBuf {
            self.dir.path().join(GRAPH_NAME)
        }

        fn load(&self) -> Result<OnnxBlockModel, DiarizeError> {
            let (graph_sha, weights_sha) = (sha_hex(&self.graph), sha_hex(&self.weights));
            OnnxBlockModel::load_verified(
                &self.anchor,
                [
                    Wanted {
                        name: GRAPH_NAME,
                        size: self.graph.len() as u64,
                        sha256: Some(&graph_sha),
                    },
                    Wanted {
                        name: WEIGHTS_NAME,
                        size: self.weights.len() as u64,
                        sha256: Some(&weights_sha),
                    },
                ],
                self.dir.path(),
                1,
            )
        }
    }

    /// `logits` fuer `input_features = [1.0]`: `1 + w`.
    fn logit(model: &mut OnnxBlockModel) -> f32 {
        let (x, cached, mask) = ([1.0f32], [0.0f32], [0.0f32]);
        let outputs = model
            .session
            .run(ort::inputs![
                "input_features" => TensorRef::from_array_view(([1usize], &x[..])).unwrap(),
                "cached_embeds" => TensorRef::from_array_view(([1usize], &cached[..])).unwrap(),
                "attention_mask" => TensorRef::from_array_view(([1usize], &mask[..])).unwrap(),
            ])
            .unwrap();
        outputs["logits"].try_extract_tensor::<f32>().unwrap().1[0]
    }

    /// Ersetzt `path` durch `bytes`: einmal in die Datei hinein, einmal durch eine andere
    /// Datei unter demselben Namen. `false`, wenn das System es nicht zulaesst (Windows:
    /// eine geoeffnete Datei laesst sich so nicht tauschen).
    fn replace_file(path: &Path, bytes: &[u8], by_rename: bool) -> bool {
        if by_rename {
            let other = path.with_extension("neu");
            std::fs::write(&other, bytes).is_ok() && std::fs::rename(&other, path).is_ok()
        } else {
            std::fs::write(path, bytes).is_ok()
        }
    }

    fn load_message(result: Result<OnnxBlockModel, DiarizeError>) -> String {
        match result.err() {
            Some(DiarizeError::Load(msg)) => msg,
            other => panic!("erwartet Load, bekam {other:?}"),
        }
    }

    /// Die Sitzung rechnet mit den Gewichten aus dem Speicher: kein Pfad wird geoeffnet,
    /// und `w.bin` ist ein externer Initialisierer des Graphen.
    #[test]
    fn the_session_computes_with_the_checked_weights_from_memory() {
        let tiny = Tiny::new(10.0);
        let mut model = tiny.load().expect("geprueftes Paar laedt");
        assert_eq!(logit(&mut model), 11.0);
    }

    /// Ein Tausch NACH dem Lesen (Haken `Loaded`: alle Bytes im Speicher, noch keine
    /// Sitzung) aendert die Sitzung nicht: sie rechnet mit den geprueften Bytes, nicht mit
    /// dem, was jetzt am Pfad liegt.
    #[test]
    fn a_swap_after_the_read_has_no_effect_on_the_session() {
        for by_rename in [false, true] {
            let tiny = Tiny::new(10.0);
            let swapped = Arc::new(AtomicBool::new(false));
            let _guard = anchor::hook::set(tiny.dir.path(), {
                let (path, swapped) = (tiny.weights_path(), swapped.clone());
                move |point| {
                    if point == Point::Loaded {
                        let ok = replace_file(&path, &99.0f32.to_le_bytes(), by_rename);
                        swapped.store(ok, Ordering::SeqCst);
                    }
                }
            });

            let mut model = tiny.load().expect("das Paar war beim Lesen gueltig");

            assert!(
                swapped.load(Ordering::SeqCst),
                "der Tausch hat nicht stattgefunden (rename={by_rename}): der Test prueft nichts"
            );
            assert_eq!(
                std::fs::read(tiny.weights_path()).unwrap(),
                99.0f32.to_le_bytes(),
                "Testannahme: am Pfad liegen jetzt die anderen Bytes"
            );
            assert_eq!(logit(&mut model), 11.0, "rename={by_rename}");
        }
    }

    /// Zwischen der Pruefung des Graphen (`Checked(0)`) und dem Lesen der Gewichte werden
    /// die Gewichte gegen gleich grosse andere Bytes getauscht: einmal in die Datei hinein
    /// (der Hash sieht es), einmal durch eine andere Datei (der Name zeigt nicht mehr auf
    /// die gelesene). Beides ist ein Ladefehler, kein Modell. Unter Windows laesst sich
    /// die geoeffnete Datei nicht tauschen; dann gibt es den Tausch nicht.
    #[test]
    fn a_same_size_swap_between_the_graph_check_and_the_weights_read_is_rejected() {
        for by_rename in [false, true] {
            let tiny = Tiny::new(10.0);
            let swapped = Arc::new(AtomicBool::new(false));
            let _guard = anchor::hook::set(tiny.dir.path(), {
                let (path, swapped) = (tiny.weights_path(), swapped.clone());
                move |point| {
                    if point == Point::Checked(0) {
                        let ok = replace_file(&path, &99.0f32.to_le_bytes(), by_rename);
                        swapped.store(ok, Ordering::SeqCst);
                    }
                }
            });

            let result = tiny.load();

            if swapped.load(Ordering::SeqCst) {
                let msg = load_message(result);
                assert!(msg.starts_with(WEIGHTS_NAME), "{msg}");
                assert!(
                    msg.contains(if by_rename {
                        "waehrend des Ladens"
                    } else {
                        "SHA-256"
                    }),
                    "{msg}"
                );
                assert!(
                    !msg.contains(tiny.dir.path().to_str().unwrap()),
                    "Pfad im Text: {msg}"
                );
            } else {
                swap_must_have_happened();
                assert!(result.is_ok());
            }
        }
    }

    /// Dasselbe fuer den Graphen: getauscht, ehe er gelesen wird (`Opened`: alle Dateien
    /// offen, noch keine gelesen), ist er ein Ladefehler mit dem Namen des Graphen.
    #[test]
    fn a_same_size_swap_of_the_graph_before_it_is_read_is_rejected() {
        let tiny = Tiny::new(10.0);
        let mut other = tiny.graph.clone();
        let last = other.len() - 1;
        other[last] ^= 0xff;
        let swapped = Arc::new(AtomicBool::new(false));
        let _guard = anchor::hook::set(tiny.dir.path(), {
            let (path, swapped) = (tiny.graph_path(), swapped.clone());
            move |point| {
                if point == Point::Opened {
                    swapped.store(replace_file(&path, &other, false), Ordering::SeqCst);
                }
            }
        });

        let result = tiny.load();

        if swapped.load(Ordering::SeqCst) {
            let msg = load_message(result);
            assert!(msg.starts_with(GRAPH_NAME), "{msg}");
            assert!(msg.contains("SHA-256"), "{msg}");
        } else {
            swap_must_have_happened();
            assert!(result.is_ok());
        }
    }

    /// Das Paar liegt nach einem erfolgreichen Bereitstellen vollstaendig am Platz, und
    /// danach (zwischen Bereitstellen und Laden) werden die Gewichte oder der Graph
    /// getauscht: der Lader lehnt es ab, auch bei gleicher Groesse.
    #[test]
    fn a_pair_swapped_between_provisioning_and_loading_is_rejected() {
        let tiny = Tiny::new(10.0);
        tiny.load().expect("Ausgangslage gueltig");
        std::fs::write(tiny.weights_path(), 99.0f32.to_le_bytes()).unwrap();
        let msg = load_message(tiny.load());
        assert!(
            msg.starts_with(WEIGHTS_NAME) && msg.contains("SHA-256"),
            "{msg}"
        );

        let tiny = Tiny::new(10.0);
        let mut other = tiny.graph.clone();
        other[0] ^= 0x01;
        std::fs::write(tiny.graph_path(), &other).unwrap();
        let msg = load_message(tiny.load());
        assert!(
            msg.starts_with(GRAPH_NAME) && msg.contains("SHA-256"),
            "{msg}"
        );
    }

    /// Reihenfolge der Meldungen (wie vor der geschuetzten Generation): fehlt eine Datei,
    /// ist es `ModelMissing`; sonst kommt die Groesse vor dem Inhalt.
    #[test]
    fn a_missing_file_comes_before_a_wrong_size_and_a_wrong_size_before_a_wrong_hash() {
        let tiny = Tiny::new(10.0);
        std::fs::write(tiny.graph_path(), b"anderer graph mit anderer groesse").unwrap();
        std::fs::write(tiny.weights_path(), 99.0f32.to_le_bytes()).unwrap();
        // Graph: falsche Groesse; Gewichte: gleiche Groesse, falscher Inhalt.
        let msg = load_message(tiny.load());
        assert!(
            msg.starts_with(GRAPH_NAME) && msg.contains("laut Katalog"),
            "{msg}"
        );

        std::fs::remove_file(tiny.weights_path()).unwrap();
        match tiny.load().err() {
            Some(DiarizeError::ModelMissing(path)) => assert!(path.ends_with(WEIGHTS_NAME)),
            other => panic!("erwartet ModelMissing, bekam {other:?}"),
        }
    }

    /// Voller Arbeitsspeicher: eine Reserve, die nicht zu bekommen ist (hier eine
    /// absurde Groesse aus einem beschaedigten Katalog), ist ein Ladefehler mit dem
    /// Dateinamen, kein Abbruch des Prozesses, und es wird nichts gelesen.
    #[test]
    fn memory_that_cannot_be_reserved_is_a_load_error_not_an_abort() {
        let tiny = Tiny::new(10.0);
        let sha = sha_hex(&tiny.graph);
        let result = OnnxBlockModel::load_verified(
            &tiny.anchor,
            [
                Wanted {
                    name: GRAPH_NAME,
                    size: tiny.graph.len() as u64,
                    sha256: Some(&sha),
                },
                Wanted {
                    name: WEIGHTS_NAME,
                    size: u64::MAX / 2,
                    sha256: None,
                },
            ],
            tiny.dir.path(),
            1,
        );
        let msg = load_message(result);
        assert!(msg.starts_with(WEIGHTS_NAME), "{msg}");
        assert!(msg.contains("Arbeitsspeicher"), "{msg}");
    }

    /// Ein Katalogeintrag ohne Pruefsumme wird nicht geladen: ohne SHA-256 gaebe es nichts,
    /// wogegen die Bytes zu pruefen waeren, und die Sitzung bekaeme ungepruefte Bytes.
    #[test]
    fn a_catalog_entry_without_a_checksum_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let files = catalog_layout(dir.path(), catalog_sizes());
        for victim in 0..2 {
            let mut place = catalog_place(&files[0]).expect("Katalogplatz");
            place.entry.files[victim].sha256 = None;
            let msg = load_message(OnnxBlockModel::load_catalog(&files[0], &place, 1));
            assert!(msg.contains("ohne SHA-256"), "{msg}");
            assert!(
                !msg.contains(dir.path().to_str().unwrap()),
                "Pfad im Text: {msg}"
            );
        }
    }

    /// Ein Link an Stelle einer der Dateien ist ein Ladefehler (Ablehnung), kein Folgen.
    #[test]
    fn a_link_in_place_of_the_weights_is_a_load_error_and_is_not_followed() {
        use crate::managers::model::download::linktest::symlink_file;
        let tiny = Tiny::new(10.0);
        let outside = tempfile::tempdir().unwrap();
        let foreign = outside.path().join("fremd.bin");
        std::fs::write(&foreign, &tiny.weights).unwrap();
        std::fs::remove_file(tiny.weights_path()).unwrap();
        if !symlink_file(&tiny.weights_path(), &foreign) {
            return;
        }
        let msg = load_message(tiny.load());
        assert!(msg.contains("Link"), "{msg}");
        assert!(
            !msg.contains(outside.path().to_str().unwrap()),
            "Pfad im Text: {msg}"
        );
        assert_eq!(std::fs::read(&foreign).unwrap(), tiny.weights);
    }
}

/// Tests mit dem echten Modell. Sie laufen, wenn `LVA_NEMOTRON_MODEL` auf die
/// `.onnx`-Datei zeigt (die `.onnx_data` liegt daneben). Die Referenz stammt aus
/// `koordination/nvidia-speech/tools/make_reference.py`.
///
/// Fehlt das Modell oder eine Pruef-WAV, gibt es zwei Faelle:
/// * ohne `LVA_NEMOTRON_REQUIRED`: der Test meldet `UEBERSPRUNGEN` (auf dem
///   echten stderr, also auch ohne `--nocapture` sichtbar) und gilt als bestanden;
///   das ist ein Lauf OHNE Modellnachweis;
/// * mit `LVA_NEMOTRON_REQUIRED=1` (Abnahme, Nachweislauf): fehlendes Modell oder
///   fehlende WAV ist ein Fehler, und der Hauptvergleich muss mindestens ein
///   Pruefaudio wirklich verglichen haben.
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

    /// Pflichtmodus: `LVA_NEMOTRON_REQUIRED` gesetzt und nicht leer/`0`/`false`.
    fn required_flag(value: Option<&str>) -> bool {
        value.is_some_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false"))
    }

    fn required() -> bool {
        required_flag(std::env::var("LVA_NEMOTRON_REQUIRED").ok().as_deref())
    }

    /// Meldung auf dem echten stderr: `eprintln!` wuerde von libtest bei
    /// bestandenen Tests verschluckt, ein uebersprungener Nachweis soll sichtbar sein.
    fn report(msg: &str) {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "{msg}");
    }

    /// Fehlt etwas, was der Test braucht: im Pflichtmodus Fehlschlag, sonst sichtbar melden.
    fn on_missing(why: &str, required: bool) {
        if required {
            panic!("LVA_NEMOTRON_REQUIRED ist gesetzt, aber: {why}");
        }
        report(&format!(
            "UEBERSPRUNGEN (kein Modellnachweis, mit LVA_NEMOTRON_REQUIRED=1 waere das ein Fehler): {why}"
        ));
    }

    fn skip(why: &str) {
        on_missing(why, required());
    }

    /// Im Pflichtmodus muss der Hauptvergleich etwas verglichen haben.
    fn check_ran(ran: usize, total: usize, required: bool) {
        if required {
            assert!(
                ran > 0,
                "LVA_NEMOTRON_REQUIRED ist gesetzt, aber von {total} Pruefaudios wurde keins verglichen"
            );
        }
    }

    /// Das Modell, oder `None` nach `skip` (im Pflichtmodus kommt es nie dazu:
    /// dann bricht `skip` den Test ab). Der Pfad steht nicht in der Meldung.
    fn model_or_skip() -> Option<PathBuf> {
        let Some(var) = std::env::var_os("LVA_NEMOTRON_MODEL") else {
            skip("LVA_NEMOTRON_MODEL nicht gesetzt");
            return None;
        };
        let path = PathBuf::from(var);
        if path.is_file() && external_data_path(&path).is_file() {
            return Some(path);
        }
        skip("LVA_NEMOTRON_MODEL zeigt nicht auf eine .onnx-Datei mit .onnx_data daneben");
        None
    }

    #[test]
    fn the_required_flag_reads_the_variable_strictly() {
        for (value, want) in [
            (None, false),
            (Some(""), false),
            (Some("0"), false),
            (Some("false"), false),
            (Some(" FALSE "), false),
            (Some("1"), true),
            (Some("true"), true),
            (Some("yes"), true),
        ] {
            assert_eq!(required_flag(value), want, "{value:?}");
        }
    }

    #[test]
    fn a_missing_model_or_audio_is_an_error_in_required_mode_and_a_visible_skip_otherwise() {
        on_missing("Modell fehlt", false);
        let err = std::panic::catch_unwind(|| on_missing("Modell fehlt", true))
            .expect_err("Pflichtmodus muss scheitern");
        let msg = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(
            msg.contains("LVA_NEMOTRON_REQUIRED") && msg.contains("Modell fehlt"),
            "{msg}"
        );
    }

    #[test]
    fn required_mode_demands_at_least_one_compared_audio() {
        check_ran(0, 2, false);
        check_ran(1, 2, true);
        check_ran(2, 2, true);
        let err = std::panic::catch_unwind(|| check_ran(0, 2, true))
            .expect_err("kein Vergleich im Pflichtmodus muss scheitern");
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("keins verglichen"), "{msg}");
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
            let (from, to) = ((t0 / 10) as usize, ((t1 / 10) as usize).min(frames));
            for cell in m.iter_mut().take(to).skip(from) {
                *cell |= 1 << (speaker - 1);
            }
        }
        m
    }

    #[test]
    fn the_model_files_are_the_pinned_catalog_files() {
        let Some(path) = model_or_skip() else {
            return;
        };
        let entry = crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
            .into_iter()
            .find(|e| e.id == crate::managers::meetings::diarize::nemotron::MODEL_ID)
            .unwrap();
        let dir = path.parent().unwrap();
        for f in &entry.files {
            let file = dir.join(&f.filename);
            assert_eq!(
                std::fs::metadata(&file).unwrap().len(),
                f.size_bytes,
                "Groesse von {} (die Statusanzeige vergleicht sie mit dem Katalog)",
                f.filename
            );
            assert_eq!(sha256(&file), f.sha256.clone().unwrap(), "{}", f.filename);
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
        let Some(path) = model_or_skip() else {
            return;
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
        let total = reference["cases"].as_array().unwrap().len();
        eprintln!("{ran} von {total} Pruefaudios gegen die Referenz verglichen");
        if ran == 0 {
            report(
                "UEBERSPRUNGEN: kein Pruefaudio verglichen, dieser Lauf belegt die Paritaet nicht",
            );
        }
        check_ran(ran, total, required());
    }

    #[test]
    fn a_truncated_weights_file_is_a_load_error_not_a_crash() {
        let Some(path) = model_or_skip() else {
            return;
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

    /// Der geschuetzte Ladeweg mit dem echten Modell: beide Dateien ueber ein Ordner-Handle
    /// gelesen, gegen Groesse und SHA-256 des Katalogs geprueft, die Sitzung aus genau
    /// diesen Bytes im Speicher gebaut (Gewichte als externe Initialisierer im Speicher).
    /// Die Logits stimmen mit denen der Sitzung ueberein, die ONNX Runtime selbst aus den
    /// Dateien laedt. Im Pflichtmodus (`LVA_NEMOTRON_REQUIRED=1`) fehlt dieser Nachweis nie.
    #[test]
    fn the_protected_load_gives_the_same_logits_as_the_load_by_path() {
        let Some(path) = model_or_skip() else {
            return;
        };
        let entry = crate::catalog::tts_entries(crate::catalog::Purpose::Diarization)
            .into_iter()
            .find(|e| e.id == crate::managers::meetings::diarize::nemotron::MODEL_ID)
            .unwrap();
        let dir = path.parent().unwrap();
        let (Some(root), Some(name)) = (dir.parent(), dir.file_name().and_then(|n| n.to_str()))
        else {
            skip("der Modellordner hat keinen Namen oder keinen uebergeordneten Ordner");
            return;
        };
        let anchor = Anchor::open(root, &[name], false)
            .expect("Ordner offen")
            .expect("Ordner da");
        let wanted = |i: usize| Wanted {
            name: &entry.files[i].filename,
            size: entry.files[i].size_bytes,
            sha256: entry.files[i].sha256.as_deref(),
        };
        let start = Instant::now();
        let verified = OnnxBlockModel::load_verified(&anchor, [wanted(0), wanted(1)], dir, 1)
            .expect("geschuetzte Generation laedt das gepruefte Paar");
        eprintln!(
            "Geschuetztes Laden (lesen, SHA-256, Sitzung aus dem Speicher): {:.2} s",
            start.elapsed().as_secs_f64()
        );
        let by_path = OnnxBlockModel::load_by_path(&path, 1).expect("Laden ueber den Pfad");
        let engine = |model| NemotronOnnxEngine {
            model,
            front: MelFrontend::new(),
            broken: false,
        };
        let pcm = clip();
        let a = engine(verified)
            .logits(&pcm, None)
            .expect("Lauf geschuetzt");
        let b = engine(by_path).logits(&pcm, None).expect("Lauf Pfad");
        assert_eq!(a.len(), b.len());
        assert!(!a.is_empty());
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 1e-4, "Logits weichen um {worst} ab");
    }

    #[test]
    fn cancel_in_the_middle_of_a_long_run_returns_cancelled_and_the_engine_survives() {
        let Some(path) = model_or_skip() else {
            return;
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
        let Some(path) = model_or_skip() else {
            return;
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
