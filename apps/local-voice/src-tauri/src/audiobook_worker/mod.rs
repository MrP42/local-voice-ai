//! lva#806 P1b: eigener Arbeitsprozess fuer Hoerbuch-Importe.
//! Der Worker kennt weder Datenbank noch Freigaben, Logger oder AppHandle.

pub mod process;
pub mod protocol;
pub mod parent;
mod safety;

pub use crate::managers::transcription::TimedSegment;
use crate::managers::transcription;
use protocol::{Reply, Request, WorkerError};
use std::io::{Read, Write};
use std::time::Instant;

pub trait InferenceWorker: Send {
    fn start(&mut self, generation: u64, samples: Vec<f32>) -> Result<(), WorkerError>;
    fn poll(&mut self) -> Result<Option<Reply>, WorkerError>;
    fn terminate(&mut self) -> Result<(), WorkerError>;
}

pub struct InferResult {
    pub generation: u64,
    pub segments: Vec<TimedSegment>,
    pub detected_language: Option<String>,
}

/// Ein Block pro Worker. Der Aufrufer uebergibt ALLEN Teilen dieselbe absolute
/// Frist (Nutzeraktion + 2 s), statt fuer jeden Teil erneut zwei Sekunden.
pub struct InferenceSession<W: InferenceWorker> {
    worker: W,
    generation: Option<u64>,
    stop_at: Option<Instant>,
    stopped: bool,
}

impl<W: InferenceWorker> InferenceSession<W> {
    pub(crate) fn worker_mut(&mut self) -> &mut W { &mut self.worker }
    pub(crate) fn is_stopped(&self) -> bool { self.stopped }
    pub fn new(worker: W) -> Self {
        Self { worker, generation: None, stop_at: None, stopped: false }
    }

    pub fn start(&mut self, generation: u64, samples: Vec<f32>) -> Result<(), WorkerError> {
        if self.stopped || self.stop_at.is_some() { return Err(WorkerError::Stopped); }
        if self.generation.is_some() { return Err(WorkerError::Busy); }
        self.worker.start(generation, samples)?;
        self.generation = Some(generation);
        Ok(())
    }

    pub fn request_stop(&mut self, deadline: Instant) -> Result<(), WorkerError> {
        self.stop_at = Some(self.stop_at.map_or(deadline, |old| old.min(deadline)));
        if self.generation.is_none() { self.stop()?; }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), WorkerError> {
        // Vor dem Kill sperren: selbst bei einem Betriebssystemfehler wird
        // keine spaete Antwort mehr zur Parent-Transaktion durchgereicht.
        self.generation = None;
        self.stopped = true;
        self.worker.terminate()
    }

    pub fn tick(&mut self, now: Instant) -> Result<Option<InferResult>, WorkerError> {
        if self.stopped { return Ok(None); }
        if self.stop_at.is_some_and(|deadline| now >= deadline) {
            self.stop()?;
            return Ok(None);
        }
        let reply = match self.worker.poll() {
            Ok(reply) => reply,
            Err(error) => { let _ = self.stop(); return Err(error); }
        };
        match reply {
            Some(Reply::Result { generation, segments, detected_language }) if self.generation == Some(generation) => {
                self.generation = None;
                if self.stop_at.is_some() { self.stop()?; }
                Ok(Some(InferResult { generation, segments, detected_language }))
            }
            Some(Reply::Error { code }) => { let _ = self.stop(); Err(code) }
            _ => Ok(None),
        }
    }
}

impl<W: InferenceWorker> Drop for InferenceSession<W> {
    fn drop(&mut self) { let _ = self.stop(); }
}

fn serve(input: &mut impl Read, output: &mut impl Write) -> Result<(), WorkerError> {
    let mut engine = None;
    let mut language = String::new();
    while let Some(request) = protocol::read::<Request>(input)? {
        request.validate()?;
        let reply = match request {
            Request::Load { model, n_threads, accelerator, ort_accelerator, device, language: intent } => {
                if engine.is_some() { return Err(WorkerError::Invalid); }
                use crate::settings::OrtAcceleratorSetting;
                use transcribe_rs::accel::{self, OrtAccelerator};
                accel::set_ort_accelerator(match ort_accelerator {
                    OrtAcceleratorSetting::Auto => OrtAccelerator::Auto,
                    OrtAcceleratorSetting::Cpu => OrtAccelerator::CpuOnly,
                    OrtAcceleratorSetting::Cuda => OrtAccelerator::Cuda,
                    OrtAcceleratorSetting::DirectMl => OrtAccelerator::DirectMl,
                    OrtAcceleratorSetting::Rocm => OrtAccelerator::Rocm,
                });
                language = crate::managers::model::effective_language(
                    &intent, &model.supported_languages, model.supports_language_detection,
                );
                match transcription::build_local_engine(
                    "", model.kind.into(), &model.path, None, Some(n_threads),
                    accelerator, device, &|_| {},
                ) {
                    Ok(loaded) => { engine = Some(loaded); Reply::Loaded }
                    Err(_) => { protocol::write(output, &Reply::Error { code: WorkerError::LoadFailed })?; return Ok(()); }
                }
            }
            request @ (Request::Infer { .. } | Request::InferConfigured { .. }) => {
                let (generation, samples, intent, custom_words, model_id) = match request {
                    Request::Infer { generation, samples } => (generation, samples, language.clone(), vec![], String::new()),
                    Request::InferConfigured { generation, samples, language, custom_words, model_id } =>
                        (generation, samples, language, custom_words, model_id),
                    _ => unreachable!(),
                };
                let Some(engine) = engine.as_mut() else {
                    protocol::write(output, &Reply::Error { code: WorkerError::NotLoaded })?;
                    return Ok(());
                };
                match transcription::infer_local_segments(engine, &samples, &intent, false, &custom_words, &model_id) {
                    Ok((segments, detected_language)) => {
                        let (segments, _) = crate::managers::meetings::hallucination::collapse_loops_fail_open(segments);
                        Reply::Result { generation, segments, detected_language }
                    }
                    Err(_) => { protocol::write(output, &Reply::Error { code: WorkerError::InferFailed })?; return Ok(()); }
                }
            }
            Request::Shutdown => return Ok(()),
        };
        protocol::write(output, &reply)?;
    }
    Ok(())
}

pub fn main() -> i32 {
    // Keine Panic-Payload (native Bibliotheken koennen Pfade/Text darin tragen).
    std::panic::set_hook(Box::new(|_| {}));
    if safety::initialize().is_err() { return 65; }
    let result = std::panic::catch_unwind(|| {
        serve(&mut std::io::stdin().lock(), &mut std::io::stdout().lock())
    });
    match result { Ok(Ok(())) => 0, _ => 65 }
}

#[cfg(test)]
mod tests;
