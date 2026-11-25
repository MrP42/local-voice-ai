//! lva#806 P1b: lokale Pipes, vier Byte LE-Laenge vor versioniertem JSON.
//! Keine Nutzdaten in Fehlern; Grenzen gelten vor der Payload-Allokation.

use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::TimedSegment;
use crate::managers::model::EngineType;
use crate::settings::TranscribeAcceleratorSetting;

pub const VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SAMPLES: usize = 20 * 16_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerError {
    Io,
    Limit,
    Version,
    Invalid,
    NotLoaded,
    LoadFailed,
    InferFailed,
    Crashed,
    Busy,
    Stopped,
    Timeout,
    SpawnFailed,
    ContainmentFailed,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    TranscribeCpp,
    Parakeet,
    Moonshine,
    MoonshineStreaming,
    SenseVoice,
    GigaAm,
    Canary,
    Cohere,
}

impl From<ModelKind> for EngineType {
    fn from(kind: ModelKind) -> Self {
        match kind {
            ModelKind::TranscribeCpp => Self::TranscribeCpp,
            ModelKind::Parakeet => Self::Parakeet,
            ModelKind::Moonshine => Self::Moonshine,
            ModelKind::MoonshineStreaming => Self::MoonshineStreaming,
            ModelKind::SenseVoice => Self::SenseVoice,
            ModelKind::GigaAm => Self::GigaAM,
            ModelKind::Canary => Self::Canary,
            ModelKind::Cohere => Self::Cohere,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalModel {
    pub path: PathBuf,
    pub kind: ModelKind,
    pub supported_languages: Vec<String>,
    pub supports_language_detection: bool,
}

// Absichtlich kein Debug: PCM, Pfade und Text gehoeren nie in Diagnoseausgaben.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Load {
        model: LocalModel,
        n_threads: i32,
        accelerator: TranscribeAcceleratorSetting,
        #[serde(default)]
        ort_accelerator: crate::settings::OrtAcceleratorSetting,
        device: i32,
        language: String,
    },
    Infer { generation: u64, samples: Vec<f32> },
    InferConfigured { generation: u64, samples: Vec<f32>, language: String, custom_words: Vec<String>, model_id: String },
    Shutdown,
}

impl Request {
    pub fn validate(&self) -> Result<(), WorkerError> {
        match self {
            Self::Load { model, n_threads, language, .. } => {
                if !(1..=1024).contains(n_threads)
                    || !model.path.is_absolute()
                    || model.path.as_os_str().len() > 32_768
                    || language.len() > 64
                    || model.supported_languages.len() > 1024
                    || model.supported_languages.iter().any(|l| l.len() > 64)
                {
                    return Err(WorkerError::Invalid);
                }
            }
            Self::Infer { samples, .. } | Self::InferConfigured { samples, .. } => {
                if samples.len() > MAX_SAMPLES { return Err(WorkerError::Limit); }
                if samples.iter().any(|s| !s.is_finite()) { return Err(WorkerError::Invalid); }
            }
            Self::Shutdown => {}
        }
        if let Self::InferConfigured { language, custom_words, model_id, .. } = self {
            if language.len() > 64 || model_id.len() > 32_768
                || custom_words.iter().map(String::len).sum::<usize>() > 1_048_576 {
                return Err(WorkerError::Limit);
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Loaded,
    Result { generation: u64, segments: Vec<TimedSegment>, detected_language: Option<String> },
    Error { code: WorkerError },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame<T> {
    version: u32,
    message: T,
}

pub fn read<T: DeserializeOwned>(input: &mut impl Read) -> Result<Option<T>, WorkerError> {
    let mut size = [0u8; 4];
    // Nur EOF vor dem Header ist ein sauberes Ende, nicht ein halber Frame.
    loop {
        match input.read(&mut size[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(WorkerError::Io),
        }
    }
    input.read_exact(&mut size[1..]).map_err(|_| WorkerError::Invalid)?;
    let len = u32::from_le_bytes(size) as usize;
    if len == 0 || len > MAX_FRAME_BYTES { return Err(WorkerError::Limit); }
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes).map_err(|_| WorkerError::Invalid)?;
    let frame: Frame<T> = serde_json::from_slice(&bytes).map_err(|_| WorkerError::Invalid)?;
    if frame.version != VERSION { return Err(WorkerError::Version); }
    Ok(Some(frame.message))
}

pub fn write<T: Serialize>(output: &mut impl Write, message: &T) -> Result<(), WorkerError> {
    // Der Writer stoppt auch waehrend der Serialisierung, statt erst nach einer
    // beliebig grossen zweiten Text-/PCM-Kopie die Grenze zu pruefen.
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_FRAME_BYTES - self.0.len() {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer(&mut bytes, &Frame { version: VERSION, message }).map_err(|_| WorkerError::Limit)?;
    output.write_all(&(bytes.0.len() as u32).to_le_bytes()).map_err(|_| WorkerError::Io)?;
    output.write_all(&bytes.0).map_err(|_| WorkerError::Io)?;
    output.flush().map_err(|_| WorkerError::Io)
}
