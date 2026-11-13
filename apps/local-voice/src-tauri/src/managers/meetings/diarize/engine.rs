//! M3-P3a: Sortformer 4spk-v2.1 (GGUF) ueber transcribe-cpp 0.2.4, dazu (N1)
//! die Wahl des Motors nach der Modelldatei: `.gguf` -> Sortformer,
//! `.onnx` -> Nemotron-3-Diarization ([`super::nemotron`]).
//!
//! Schutz: Es laeuft immer hoechstens EIN Diarisierer im Prozess
//! ([`ExclusiveSlot`]); vor dem Laden prueft das RAM-Start-Gate
//! (`process_guard`), ob genug Speicher frei ist. Das Modell laeuft im
//! Prozess (FFI), nicht als Kindprozess: eine Rust-Panik im Aufruf wird zu
//! einem Fehler, der Diarisierer ist danach unbrauchbar und wird verworfen.
//!
//! Die transcribe-cpp-Backends muessen vor dem ersten Laden registriert sein
//! (`transcription::init_transcribe_backend`, beim App-Start bzw. im
//! Headless-Pfad). Hier wird bewusst NICHT initialisiert: die Registrierung
//! darf nicht mit Modell-Ladevorgaengen anderer Threads konkurrieren.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Condvar, Mutex};

use transcribe_cpp::{
    Backend, CancelToken, Diarize, Model, ModelOptions, RunExtension, RunOptions, Session,
    SessionOptions, SortformerPreset, SortformerStreamOptions,
};

use super::nemotron::NemotronOnnxEngine;
use super::postproc::RawTurn;
use super::{diarize_with, DiarizeError, DiarizeParams, Preset, Turn};

/// RAM-Bedarf des Diarisierers (MB) fuer das Start-Gate. Gemessen 0,6-0,7 GB
/// Spitze (Sortformer Q8, 10 min Audio); Nemotron (ONNX int8) 0,32 GB Spitze des
/// Testprozesses bei 100 s Audio (davon 63 MB Grundlast des Testprogramms; je
/// Block nur der Speicher des Blocks, die Merkmale entstehen blockweise), also
/// unter derselben Grenze. Die Reserve von `process_guard` kommt dazu.
pub const DIARIZE_RAM_NEED_MB: u64 = 1024;

/// Was ein Diarisierer koennen muss. Die echte Umsetzung ist
/// [`SortformerEngine`]; Tests nutzen Attrappen.
pub trait DiarizeEngine {
    /// Rohe Turns fuer 16-kHz-Mono-PCM (Zeiten in ms ab Pufferbeginn).
    fn raw_turns(
        &mut self,
        pcm: &[f32],
        preset: Preset,
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<RawTurn>, DiarizeError>;

    /// Kurzbeschreibung fuer Berichte (z. B. gebundenes Backend).
    fn describe(&self) -> String {
        String::new()
    }
}

/// Gegenseitiger Ausschluss ohne `MutexGuard` im Besitz des Aufrufers: der
/// Waechter ist `Send`, damit ein Diarisierer den Thread wechseln darf.
pub struct ExclusiveSlot {
    busy: Mutex<bool>,
    freed: Condvar,
}

impl ExclusiveSlot {
    pub const fn new() -> Self {
        Self {
            busy: Mutex::new(false),
            freed: Condvar::new(),
        }
    }

    /// Wartet, bis der Platz frei ist, und belegt ihn. Nie im Audio-Callback
    /// aufrufen (blockiert).
    pub fn acquire(&self) -> SlotGuard<'_> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        while *busy {
            busy = self.freed.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        *busy = true;
        SlotGuard { slot: self }
    }
}

impl Default for ExclusiveSlot {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SlotGuard<'a> {
    slot: &'a ExclusiveSlot,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let mut busy = self.slot.busy.lock().unwrap_or_else(|e| e.into_inner());
        *busy = false;
        self.slot.freed.notify_one();
    }
}

/// Der eine Platz fuer einen geladenen Diarisierer im Prozess.
static DIARIZER_SLOT: ExclusiveSlot = ExclusiveSlot::new();

/// Sortformer ueber transcribe-cpp.
pub struct SortformerEngine {
    session: Session,
    backend: String,
    /// Nach einer Panik im FFI-Aufruf ist der Sitzungszustand unbekannt.
    broken: bool,
}

impl SortformerEngine {
    /// Laedt das GGUF und legt eine Sitzung mit `threads` CPU-Threads an.
    /// Prueft die Architektur: ein STT-Modell hier waere ein Bedienfehler.
    pub fn load(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        let backend = if crate::utils::is_windows_x64_emulated_on_arm64() {
            Backend::Cpu
        } else {
            Backend::Auto
        };
        let options = ModelOptions {
            backend,
            device: None,
        };
        let model = Model::load_with(model_path, &options)
            .map_err(|e| DiarizeError::Load(format!("{}: {e}", model_path.display())))?;
        let arch = model.arch();
        if !arch.to_ascii_lowercase().contains("sortformer") {
            return Err(DiarizeError::WrongModel(arch));
        }
        let session = model
            .session_with(&SessionOptions {
                n_threads: threads.clamp(1, 64) as i32,
                ..Default::default()
            })
            .map_err(|e| DiarizeError::Load(format!("Sitzung: {e}")))?;
        Ok(Self {
            backend: model.backend(),
            session,
            broken: false,
        })
    }
}

fn to_sortformer_preset(p: Preset) -> SortformerPreset {
    match p {
        Preset::Default => SortformerPreset::Default,
        Preset::VeryHighLatency => SortformerPreset::VeryHighLatency,
        Preset::HighLatency => SortformerPreset::HighLatency,
        Preset::LowLatency => SortformerPreset::LowLatency,
    }
}

impl DiarizeEngine for SortformerEngine {
    fn raw_turns(
        &mut self,
        pcm: &[f32],
        preset: Preset,
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<RawTurn>, DiarizeError> {
        if self.broken {
            return Err(DiarizeError::Run(
                "Diarisierer nach einem Fehler unbrauchbar".into(),
            ));
        }
        match cancel {
            Some(token) => self.session.set_cancel_token(token),
            None => self.session.clear_cancel_token(),
        }
        let options = RunOptions {
            diarize: Diarize::On,
            family: Some(RunExtension::Sortformer(SortformerStreamOptions {
                preset: Some(to_sortformer_preset(preset)),
            })),
            ..Default::default()
        };
        let session = &mut self.session;
        let result = catch_unwind(AssertUnwindSafe(|| session.run(pcm, &options)));
        let transcript = match result {
            Ok(Ok(t)) => t,
            Ok(Err(transcribe_cpp::Error::Aborted { .. })) => return Err(DiarizeError::Cancelled),
            Ok(Err(e)) => return Err(DiarizeError::Run(e.to_string())),
            Err(_) => {
                self.broken = true;
                return Err(DiarizeError::Run("Panik im Diarisierer".into()));
            }
        };
        Ok(transcript
            .speaker_segments
            .iter()
            .map(|s| RawTurn {
                t0_ms: s.t0_ms,
                t1_ms: s.t1_ms,
                speaker_id: s.speaker_id,
            })
            .collect())
    }

    fn describe(&self) -> String {
        self.backend.clone()
    }
}

/// Welcher Motor zu einer Modelldatei gehoert (nach der Endung).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// `.gguf`: Sortformer ueber transcribe-cpp.
    Sortformer,
    /// `.onnx`: Nemotron-3-Diarization ueber ONNX Runtime.
    Nemotron,
}

impl EngineKind {
    /// Name fuer Meldungen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sortformer => "Sortformer (GGUF)",
            Self::Nemotron => "Nemotron-3-Diarization (ONNX)",
        }
    }

    /// `None` fuer jede andere Endung (kein Raten am Dateiinhalt).
    pub fn of_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "gguf" => Some(Self::Sortformer),
            "onnx" => Some(Self::Nemotron),
            _ => None,
        }
    }
}

/// Der Motor, den [`Diarizer`] standardmaessig haelt: je nach Modelldatei
/// Sortformer oder Nemotron. Aufrufer aendern sich dadurch nicht.
pub enum AnyEngine {
    Sortformer(SortformerEngine),
    Nemotron(NemotronOnnxEngine),
}

impl AnyEngine {
    /// Waehlt den Motor nach der Dateiendung. Eine fremde Endung ist ein
    /// Bedienfehler ([`DiarizeError::WrongModel`]), kein Ladeversuch.
    pub fn load(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        match EngineKind::of_path(model_path) {
            Some(EngineKind::Sortformer) => {
                SortformerEngine::load(model_path, threads).map(Self::Sortformer)
            }
            Some(EngineKind::Nemotron) => {
                NemotronOnnxEngine::load(model_path, threads).map(Self::Nemotron)
            }
            None => Err(DiarizeError::WrongModel(format!(
                "Dateityp '{}'",
                model_path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ))),
        }
    }

    pub fn kind(&self) -> EngineKind {
        match self {
            Self::Sortformer(_) => EngineKind::Sortformer,
            Self::Nemotron(_) => EngineKind::Nemotron,
        }
    }
}

impl DiarizeEngine for AnyEngine {
    fn raw_turns(
        &mut self,
        pcm: &[f32],
        preset: Preset,
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<RawTurn>, DiarizeError> {
        match self {
            Self::Sortformer(e) => e.raw_turns(pcm, preset, cancel),
            Self::Nemotron(e) => e.raw_turns(pcm, preset, cancel),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Sortformer(e) => e.describe(),
            Self::Nemotron(e) => e.describe(),
        }
    }
}

/// Ein geladener Diarisierer, der den Prozess-Platz haelt, bis er faellt.
pub struct Diarizer<E: DiarizeEngine = AnyEngine> {
    engine: E,
    _slot: SlotGuard<'static>,
}

impl Diarizer<AnyEngine> {
    /// Wartet auf den Platz, prueft Modelldatei und RAM, laedt dann den Motor,
    /// der zur Dateiendung gehoert.
    pub fn load(model_path: &Path, threads: usize) -> Result<Self, DiarizeError> {
        Self::load_with(
            &DIARIZER_SLOT,
            model_path,
            crate::process_guard::check_ram_for_start,
            |path| AnyEngine::load(path, threads),
        )
    }
}

impl<E: DiarizeEngine> Diarizer<E> {
    /// Ladereihenfolge mit austauschbaren Teilen (Tests): Platz belegen,
    /// Datei pruefen, RAM-Gate, erst dann das Modell laden.
    pub fn load_with(
        slot: &'static ExclusiveSlot,
        model_path: &Path,
        ram_gate: impl FnOnce(u64) -> Result<u64, String>,
        load: impl FnOnce(&Path) -> Result<E, DiarizeError>,
    ) -> Result<Self, DiarizeError> {
        let guard = slot.acquire();
        if !model_path.is_file() {
            return Err(DiarizeError::ModelMissing(model_path.to_path_buf()));
        }
        ram_gate(DIARIZE_RAM_NEED_MB).map_err(DiarizeError::LowMemory)?;
        let engine = load(model_path)?;
        Ok(Self {
            engine,
            _slot: guard,
        })
    }

    /// Diarisieren inkl. Nachbearbeitung.
    pub fn diarize(&mut self, pcm: &[f32], p: &DiarizeParams) -> Result<Vec<Turn>, DiarizeError> {
        diarize_with(&mut self.engine, pcm, p)
    }
}

/// Ein geladener Diarisierer ist selbst ein Motor (das DER-Werkzeug braucht
/// die rohen Turns, um "roh" und "nachbearbeitet" zu messen).
impl<E: DiarizeEngine> DiarizeEngine for Diarizer<E> {
    fn raw_turns(
        &mut self,
        pcm: &[f32],
        preset: Preset,
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<RawTurn>, DiarizeError> {
        self.engine.raw_turns(pcm, preset, cancel)
    }

    fn describe(&self) -> String {
        self.engine.describe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct Nop;
    impl DiarizeEngine for Nop {
        fn raw_turns(
            &mut self,
            _pcm: &[f32],
            _preset: Preset,
            _cancel: Option<&CancelToken>,
        ) -> Result<Vec<RawTurn>, DiarizeError> {
            Ok(vec![])
        }
    }

    fn leak_slot() -> &'static ExclusiveSlot {
        Box::leak(Box::new(ExclusiveSlot::new()))
    }

    #[test]
    fn only_one_diarizer_runs_at_a_time() {
        let slot = leak_slot();
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (active, peak) = (active.clone(), peak.clone());
                std::thread::spawn(move || {
                    let _g = slot.acquire();
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(15));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_missing_model_fails_before_the_ram_gate_and_frees_the_slot() {
        let slot = leak_slot();
        let gate_calls = AtomicUsize::new(0);
        let err = Diarizer::<Nop>::load_with(
            slot,
            Path::new("Z:/gibt/es/nicht/sortformer.gguf"),
            |_| {
                gate_calls.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            },
            |_| Ok(Nop),
        )
        .err()
        .expect("fehlendes Modell muss scheitern");
        assert!(matches!(err, DiarizeError::ModelMissing(_)), "{err}");
        assert_eq!(gate_calls.load(Ordering::SeqCst), 0);
        // Der Platz ist wieder frei (sonst haengt dieser Aufruf).
        drop(slot.acquire());
    }

    #[test]
    fn low_memory_refuses_to_load_the_model() {
        let slot = leak_slot();
        let file = tempfile::NamedTempFile::new().unwrap();
        let loads = AtomicUsize::new(0);
        let err = Diarizer::<Nop>::load_with(
            slot,
            file.path(),
            |need| {
                assert_eq!(need, DIARIZE_RAM_NEED_MB);
                Err("Zu wenig freier Arbeitsspeicher".into())
            },
            |_| {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(Nop)
            },
        )
        .err()
        .expect("RAM-Gate muss den Start verweigern");
        assert!(matches!(err, DiarizeError::LowMemory(_)), "{err}");
        assert_eq!(
            loads.load(Ordering::SeqCst),
            0,
            "Modell trotz RAM-Sperre geladen"
        );
        drop(slot.acquire());
    }

    #[test]
    fn a_loaded_diarizer_holds_the_slot_until_dropped() {
        let slot = leak_slot();
        let file = tempfile::NamedTempFile::new().unwrap();
        let d = Diarizer::<Nop>::load_with(slot, file.path(), |_| Ok(0), |_| Ok(Nop)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _g = slot.acquire();
            tx.send(()).unwrap();
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(80)).is_err(),
            "zweiter Diarisierer durfte laden, waehrend der erste lebt"
        );
        drop(d);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("Platz nach dem Entladen nicht frei");
        waiter.join().unwrap();
    }

    #[test]
    fn the_engine_follows_the_model_file_extension() {
        let kind = |p: &str| EngineKind::of_path(Path::new(p));
        assert_eq!(
            kind("a/b/sortformer-Q8_0.gguf"),
            Some(EngineKind::Sortformer)
        );
        assert_eq!(kind("a/b/SORTFORMER.GGUF"), Some(EngineKind::Sortformer));
        assert_eq!(kind("a/b/model_quantized.onnx"), Some(EngineKind::Nemotron));
        assert_eq!(kind("a/b/model.ONNX"), Some(EngineKind::Nemotron));
        // Die Gewichte-Datei ist kein Modell, das man laedt.
        assert_eq!(kind("a/b/model_quantized.onnx_data"), None);
        assert_eq!(kind("a/b/model.bin"), None);
        assert_eq!(kind("a/b/ohne-endung"), None);
    }

    #[test]
    fn an_unknown_file_type_is_wrong_model_not_a_load_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.bin");
        std::fs::write(&path, b"x").unwrap();
        match AnyEngine::load(&path, 1) {
            Err(DiarizeError::WrongModel(what)) => assert!(what.contains("bin"), "{what}"),
            other => panic!("erwartet WrongModel, bekam {:?}", other.err()),
        }
    }

    /// Ueber den Platz und das RAM-Gate geladen waehlt `.onnx` den Nemotron-
    /// Motor: ohne `.onnx_data` meldet er `ModelMissing` mit der Gewichte-Datei,
    /// und der Platz ist danach wieder frei.
    #[test]
    fn an_onnx_file_goes_to_the_nemotron_engine_and_frees_the_slot_on_error() {
        let slot = leak_slot();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model_quantized.onnx");
        std::fs::write(&path, b"graph").unwrap();
        let err =
            Diarizer::<AnyEngine>::load_with(slot, &path, |_| Ok(0), |p| AnyEngine::load(p, 1))
                .err()
                .expect("ohne Gewichte muss das Laden scheitern");
        match err {
            DiarizeError::ModelMissing(p) => {
                assert!(p.ends_with("model_quantized.onnx_data"), "{}", p.display())
            }
            other => panic!("erwartet ModelMissing, bekam {other}"),
        }
        drop(slot.acquire());
    }
}
