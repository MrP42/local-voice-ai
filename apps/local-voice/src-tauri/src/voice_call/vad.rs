use crate::audio_toolkit::vad::{SileroVad, VoiceActivityDetector};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{AppHandle, Manager};

const FRAME: usize = 480;
const MAX_SAMPLES: usize = 16000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
pub enum VadFehler {
    ZuGross,
    ZuViele,
    ModellFehlt,
    Unbekannt,
    Beschaeftigt,
    UngueltigesAudio,
    Verarbeitung,
}

pub trait ProbEngine {
    fn prob(&mut self, frame: &[f32]) -> anyhow::Result<f32>;
    fn reset(&mut self);
}

struct SileroEngine(SileroVad);
impl ProbEngine for SileroEngine {
    fn prob(&mut self, frame: &[f32]) -> anyhow::Result<f32> {
        self.0.probability(frame)
    }
    fn reset(&mut self) {
        VoiceActivityDetector::reset(&mut self.0);
    }
}

pub struct VadSession {
    engine: Box<dyn ProbEngine + Send>,
    rest: Vec<f32>,
}
impl VadSession {
    pub fn new(engine: Box<dyn ProbEngine + Send>) -> Self {
        Self {
            engine,
            rest: Vec::new(),
        }
    }
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>, VadFehler> {
        if samples.len() > MAX_SAMPLES {
            return Err(VadFehler::ZuGross);
        }
        if self.rest.len() >= FRAME {
            self.reset();
            return Err(VadFehler::Verarbeitung);
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(VadFehler::UngueltigesAudio);
        }
        self.rest.extend_from_slice(samples);
        let voll = self.rest.len() / FRAME * FRAME;
        let mut probs = Vec::with_capacity(voll / FRAME);
        for frame in self.rest[..voll].chunks_exact(FRAME) {
            match self.engine.prob(frame) {
                Ok(prob) if (0.0..=1.0).contains(&prob) => probs.push(prob),
                _ => {
                    // Nach Teilverarbeitung darf kein alter Frame erneut in das Modell gelangen.
                    self.reset();
                    return Err(VadFehler::Verarbeitung);
                }
            }
        }
        self.rest.drain(..voll);
        Ok(probs)
    }
    pub fn reset(&mut self) {
        self.rest.clear();
        self.engine.reset();
    }
}

fn sperren<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[derive(Default)]
pub struct VadSessions {
    sitzungen: Mutex<HashMap<String, VadSession>>,
    // Reservierungen zählen auch während Modellladen und Berechnung zum Limit.
    belegt: Mutex<HashSet<String>>,
    kennung: AtomicU64,
}
impl VadSessions {
    fn open(
        &self,
        laden: impl FnOnce() -> Result<VadSession, VadFehler>,
    ) -> Result<String, VadFehler> {
        let id = {
            let mut belegt = sperren(&self.belegt);
            if belegt.len() >= 4 {
                return Err(VadFehler::ZuViele);
            }
            let nummer = self
                .kennung
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .map_err(|_| VadFehler::Verarbeitung)?;
            let id = nummer.to_string();
            belegt.insert(id.clone());
            id
        };
        let sitzung = laden();
        let mut belegt = sperren(&self.belegt);
        match sitzung {
            Ok(sitzung) if belegt.contains(&id) => {
                sperren(&self.sitzungen).insert(id.clone(), sitzung);
                Ok(id)
            }
            Ok(_) => Err(VadFehler::Unbekannt),
            Err(fehler) => {
                belegt.remove(&id);
                Err(fehler)
            }
        }
    }
    pub fn push(&self, id: &str, samples: &[f32]) -> Result<Vec<f32>, VadFehler> {
        let mut sitzung = {
            let belegt = sperren(&self.belegt);
            if !belegt.contains(id) {
                return Err(VadFehler::Unbekannt);
            }
            sperren(&self.sitzungen)
                .remove(id)
                .ok_or(VadFehler::Beschaeftigt)?
        };
        let ergebnis = sitzung.push(samples);
        let belegt = sperren(&self.belegt);
        // close während compute darf die Sitzung nicht wiederauferstehen lassen.
        if belegt.contains(id) {
            sperren(&self.sitzungen).insert(id.to_owned(), sitzung);
        }
        ergebnis
    }
    pub fn close(&self, id: &str) {
        let mut belegt = sperren(&self.belegt);
        belegt.remove(id);
        sperren(&self.sitzungen).remove(id);
    }
}

fn pcm16_b64_to_f32(text: &str) -> Result<Vec<f32>, VadFehler> {
    if text.len() > (MAX_SAMPLES * 2).div_ceil(3) * 4 {
        return Err(VadFehler::ZuGross);
    }
    if text.len() % 4 != 0 {
        return Err(VadFehler::UngueltigesAudio);
    }
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    for (index, block) in text.as_bytes().chunks_exact(4).enumerate() {
        let mut werte = [0u32; 4];
        let mut padding = 0;
        for (i, &zeichen) in block.iter().enumerate() {
            werte[i] = match zeichen {
                b'A'..=b'Z' if padding == 0 => (zeichen - b'A') as u32,
                b'a'..=b'z' if padding == 0 => (zeichen - b'a' + 26) as u32,
                b'0'..=b'9' if padding == 0 => (zeichen - b'0' + 52) as u32,
                b'+' if padding == 0 => 62,
                b'/' if padding == 0 => 63,
                b'=' if i >= 2 && index + 1 == text.len() / 4 => {
                    padding += 1;
                    0
                }
                _ => return Err(VadFehler::UngueltigesAudio),
            };
        }
        if (padding == 2 && werte[1] & 15 != 0) || (padding == 1 && werte[2] & 3 != 0) {
            return Err(VadFehler::UngueltigesAudio);
        }
        let zahl = (werte[0] << 18) | (werte[1] << 12) | (werte[2] << 6) | werte[3];
        bytes.extend_from_slice(&zahl.to_be_bytes()[1..4 - padding]);
    }
    if bytes.len() > MAX_SAMPLES * 2 {
        return Err(VadFehler::ZuGross);
    }
    if bytes.len() % 2 != 0 {
        return Err(VadFehler::UngueltigesAudio);
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn voice_call_vad_open(app: AppHandle) -> Result<String, VadFehler> {
    let zustand = Arc::clone(app.state::<Arc<VadSessions>>().inner());
    tauri::async_runtime::spawn_blocking(move || {
        zustand.open(|| {
            let pfad = app
                .path()
                .resolve(
                    "resources/models/silero_vad_v4.onnx",
                    tauri::path::BaseDirectory::Resource,
                )
                .map_err(|_| VadFehler::ModellFehlt)?;
            let engine = SileroVad::new(pfad, 0.5).map_err(|_| VadFehler::ModellFehlt)?;
            Ok(VadSession::new(Box::new(SileroEngine(engine))))
        })
    })
    .await
    .map_err(|_| VadFehler::Verarbeitung)?
}

#[tauri::command]
#[specta::specta]
pub async fn voice_call_vad_push(
    app: AppHandle,
    session_id: String,
    pcm16_b64: String,
) -> Result<Vec<f32>, VadFehler> {
    let zustand = Arc::clone(app.state::<Arc<VadSessions>>().inner());
    tauri::async_runtime::spawn_blocking(move || {
        let samples = pcm16_b64_to_f32(&pcm16_b64)?;
        zustand.push(&session_id, &samples)
    })
    .await
    .map_err(|_| VadFehler::Verarbeitung)?
}

#[tauri::command]
#[specta::specta]
pub fn voice_call_vad_close(app: AppHandle, session_id: String) -> Result<(), VadFehler> {
    app.state::<Arc<VadSessions>>().close(&session_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mittelwert;
    impl ProbEngine for Mittelwert {
        fn prob(&mut self, frame: &[f32]) -> anyhow::Result<f32> {
            assert_eq!(frame.len(), FRAME);
            Ok(frame.iter().sum::<f32>() / FRAME as f32)
        }
        fn reset(&mut self) {}
    }
    fn sitzung() -> VadSession {
        VadSession::new(Box::new(Mittelwert))
    }
    struct FesterWert(f32);
    impl ProbEngine for FesterWert {
        fn prob(&mut self, _: &[f32]) -> anyhow::Result<f32> {
            Ok(self.0)
        }
        fn reset(&mut self) {
            self.0 = 0.0;
        }
    }
    #[test]
    fn modellbereich_und_fehlerreset() {
        for wert in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            let mut vad = VadSession::new(Box::new(FesterWert(wert)));
            assert_eq!(vad.push(&[0.0; 481]), Err(VadFehler::Verarbeitung));
            assert!(vad.rest.is_empty());
            assert_eq!(vad.push(&[0.0; 480]).unwrap(), vec![0.0]);
        }
    }
    #[test]
    fn frames_rest_reihenfolge_und_reset() {
        let mut vad = sitzung();
        assert!(vad.push(&[0.25; 479]).unwrap().is_empty());
        assert_eq!(vad.push(&[0.25]).unwrap(), vec![0.25]);
        let mut samples = vec![0.5; 480];
        samples.extend_from_slice(&[0.75; 480]);
        samples.extend_from_slice(&[1.0; 7]);
        assert_eq!(vad.push(&samples).unwrap(), vec![0.5, 0.75]);
        assert_eq!(vad.rest, vec![1.0; 7]);
        assert_eq!(vad.push(&[1.0; 473]).unwrap(), vec![1.0]);
        vad.push(&[1.0; 479]).unwrap();
        vad.reset();
        assert!(vad.rest.is_empty());
        assert!(vad.push(&[0.0]).unwrap().is_empty());
    }
    #[test]
    fn grenzen_und_probabilitaeten() {
        let mut vad = sitzung();
        vad.push(&[0.5; 479]).unwrap();
        assert_eq!(vad.push(&[0.0; 16001]), Err(VadFehler::ZuGross));
        assert_eq!(vad.rest.len(), 479);
        assert_eq!(vad.push(&[0.5]).unwrap(), vec![0.5]);
        for wert in [-1.0, 2.0, f32::NAN] {
            assert!(vad.push(&[wert; 480]).is_err());
        }
        assert_eq!(vad.push(&[0.0; 480]).unwrap(), vec![0.0]);
        assert_eq!(vad.push(&[1.0; 480]).unwrap(), vec![1.0]);
        assert_eq!(vad.push(&[0.0; 16000]).unwrap().len(), 33);
        assert_eq!(vad.rest.len(), 160);
    }
    #[test]
    fn sitzungslimit_close_und_ladefehler() {
        let zustand = VadSessions::default();
        assert_eq!(
            zustand.open(|| Err(VadFehler::ModellFehlt)),
            Err(VadFehler::ModellFehlt)
        );
        let ids: Vec<_> = (0..4)
            .map(|_| zustand.open(|| Ok(sitzung())).unwrap())
            .collect();
        assert_eq!(zustand.open(|| Ok(sitzung())), Err(VadFehler::ZuViele));
        zustand.close(&ids[0]);
        zustand.close(&ids[0]);
        assert_eq!(zustand.push(&ids[0], &[]), Err(VadFehler::Unbekannt));
        assert!(zustand.open(|| Ok(sitzung())).is_ok());
    }
    #[test]
    fn dekodierung_und_fehler() {
        assert_eq!(
            pcm16_b64_to_f32("AIAAAP9/AAA=").unwrap(),
            vec![-1.0, 0.0, 32767.0 / 32768.0, 0.0]
        );
        for text in ["AA==", "A", "!!!!", "AA=A", "AB==", "AAB="] {
            assert_eq!(pcm16_b64_to_f32(text), Err(VadFehler::UngueltigesAudio));
        }
        assert!(pcm16_b64_to_f32("").unwrap().is_empty());
        assert_eq!(
            pcm16_b64_to_f32(&"A".repeat(42672)),
            Err(VadFehler::ZuGross)
        );
    }
}
