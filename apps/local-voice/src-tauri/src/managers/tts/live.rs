//! Sprechwarteschlange für Live-Übersetzungen (lva#782 A3).
//! Quelltext und Zuordnung bleiben im RAM; synthetisiert wird nur gelieferter Zieltext.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use specta::Type;
use tauri::Emitter;
use tokio::sync::watch;

use super::timing::SentenceTiming;
use super::{protocol, wait_cancelled, OnPlaying, TtsManager, TtsPhase};

const CANCELLED: &str = "tts_live:cancelled";
const INVALID: &str = "tts_live:invalid_request";
const STALE: &str = "tts_live:stale_epoch";
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Deserialize, Type)]
pub struct LiveSpeechRequest {
    pub session_id: String,
    /// Appweit aufsteigende Reader-Sitzung, wie bei A1.
    pub session_generation: u32,
    pub page_id: String,
    /// Unveränderter Originaltext; alle Offsets sind UTF-16.
    pub text: String,
    pub target_lang: String,
    pub source_offset: u32,
    /// Sprech-Epoche: nur bei Sprung/Wechsel/Abbruch erhöhen, nicht beim Vorlauf.
    pub epoch: u32,
}

#[derive(Clone, Deserialize, Type)]
pub struct LiveSpeechSentence {
    pub source_start: u32,
    pub source_end: u32,
    pub translation: String,
}

struct SourceUnit {
    start: u32,
    end: u32,
    original: String,
}

fn validate(request: &LiveSpeechRequest) -> Result<Vec<SourceUnit>, &'static str> {
    if request.session_id.is_empty() || request.session_id.len() > 200
        || request.page_id.is_empty() || request.page_id.len() > 200
        || request.session_generation == 0 || request.epoch == 0
        || request.text.trim().is_empty() || request.text.len() > 16 * 1024 * 1024
        || request.target_lang.is_empty() || request.target_lang.len() > 35
        || !request.target_lang.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-' || b == b'_')
        || request.source_offset as usize > request.text.encode_utf16().count()
    {
        return Err(INVALID);
    }
    // Dieselbe Zerlegung und Lage wie A1, ohne Normalisierung oder erneutes Trimmen.
    crate::reader_live::units(&request.text).map(|units| units.into_iter().map(|unit| SourceUnit {
        start: unit.start,
        end: unit.end,
        original: request.text[unit.bytes].to_string(),
    }).collect())
}

#[derive(Clone)]
struct Run {
    id: u64,
    cancel: Arc<AtomicBool>,
}

struct Active {
    request: LiveSpeechRequest,
    units: Vec<SourceUnit>,
    position: usize,
    source_offset: u32,
    speaking: bool,
    pending: BTreeMap<usize, String>,
    run: Run,
}

#[derive(Default)]
struct Queue {
    active: Option<Active>,
    session_id: String,
    session_generation: u32,
    epoch: u32,
    serial: u64,
}

impl Queue {
    fn check_request(&self, request: &LiveSpeechRequest) -> Result<(), &'static str> {
        if request.session_generation < self.session_generation
            || (request.session_id == self.session_id
                && (request.session_generation != self.session_generation || request.epoch <= self.epoch))
            || (request.session_id != self.session_id && request.session_generation <= self.session_generation)
        {
            return Err(STALE);
        }
        Ok(())
    }

    fn begin(&mut self, mut request: LiveSpeechRequest, units: Vec<SourceUnit>, cancel: Arc<AtomicBool>) -> Run {
        if let Some(old) = self.active.take() {
            old.run.cancel.store(true, Ordering::Release);
        }
        self.session_id = request.session_id.clone();
        self.session_generation = request.session_generation;
        self.epoch = request.epoch;
        self.serial += 1;
        let run = Run { id: self.serial, cancel };
        let position = units.partition_point(|unit| unit.end <= request.source_offset);
        let source_offset = units.get(position).map_or_else(|| units.last().map_or(0, |u| u.end), |u| u.start);
        // Die Einheiten halten die Originalstücke; keine zweite Volltextkopie behalten.
        request.text = String::new();
        self.active = Some(Active { request, units, position, source_offset, speaking: false, pending: BTreeMap::new(), run: run.clone() });
        run
    }

    fn current(&self, run: &Run) -> bool {
        !run.cancel.load(Ordering::Acquire)
            && self.active.as_ref().is_some_and(|a| a.run.id == run.id)
    }

    fn take_next(&mut self, run: &Run) -> Result<Option<Item>, &'static str> {
        if !self.current(run) {
            return Err(CANCELLED);
        }
        let active = self.active.as_mut().ok_or(CANCELLED)?;
        if active.position == active.units.len() {
            return Ok(None);
        }
        let Some(translation) = active.pending.remove(&active.position) else {
            return Ok(None);
        };
        let unit = &active.units[active.position];
        active.speaking = true;
        Ok(Some(Item {
            index: active.position, start: unit.start, end: unit.end,
            original: unit.original.clone(), translation,
            session_id: active.request.session_id.clone(),
            page_id: active.request.page_id.clone(), epoch: active.request.epoch,
            target_lang: active.request.target_lang.clone(),
        }))
    }

    fn progress(&self, run: &Run) -> Result<serde_json::Value, &'static str> {
        if !self.current(run) {
            return Err(CANCELLED);
        }
        let active = self.active.as_ref().ok_or(CANCELLED)?;
        Ok(serde_json::json!({
            "position": active.position as u32, "total": active.units.len() as u32,
            "source_offset": active.source_offset, "session_id": active.request.session_id,
            "page_id": active.request.page_id, "epoch": active.request.epoch,
            "target_lang": active.request.target_lang,
        }))
    }

    fn complete(&mut self, run: &Run, item: &Item) -> Result<serde_json::Value, &'static str> {
        if !self.current(run) {
            return Err(CANCELLED);
        }
        let active = self.active.as_mut().ok_or(CANCELLED)?;
        if !active.speaking || active.position != item.index {
            return Err(CANCELLED);
        }
        active.position += 1;
        active.source_offset = item.end;
        active.speaking = false;
        self.progress(run)
    }
}

struct Item {
    index: usize,
    start: u32,
    end: u32,
    original: String,
    translation: String,
    session_id: String,
    page_id: String,
    epoch: u32,
    target_lang: String,
}

impl Item {
    fn current_payload(&self, timing: &SentenceTiming, piece: usize) -> serde_json::Value {
        serde_json::json!({
            "context": "speak", "index": self.index as u32,
            "text": self.original, "source_start": self.start, "source_end": self.end,
            "session_id": self.session_id, "page_id": self.page_id,
            "epoch": self.epoch, "target_lang": self.target_lang,
            "duration_ms": timing.duration_ms, "speech_ms": timing.speech_ms,
            "pieces_ms": timing.pieces_ms,
            "offset_ms": timing.pieces_ms.get(piece).copied().unwrap_or(0),
        })
    }
}

pub(super) struct LiveSpeech {
    inner: Mutex<Queue>,
    changed: watch::Sender<()>,
}

impl Default for LiveSpeech {
    fn default() -> Self {
        Self { inner: Mutex::new(Queue::default()), changed: watch::channel(()).0 }
    }
}

impl LiveSpeech {
    pub(super) fn push(&self, session_id: &str, generation: u32, epoch: u32, sentences: Vec<LiveSpeechSentence>) -> Result<(), &'static str> {
        let mut queue = self.inner.lock().unwrap();
        let active = queue.active.as_mut().ok_or(CANCELLED)?;
        if active.run.cancel.load(Ordering::Acquire) || active.request.session_id != session_id
            || active.request.session_generation != generation || active.request.epoch != epoch
        {
            return Err(STALE);
        }
        if sentences.len() > 40 {
            return Err(INVALID);
        }
        // Erst den gesamten Stapel prüfen: eine ungültige Lage übernimmt nichts.
        let mut additions = BTreeMap::new();
        for sentence in sentences {
            let index = active.units.binary_search_by_key(&sentence.source_start, |u| u.start).map_err(|_| INVALID)?;
            if active.units[index].end != sentence.source_end || sentence.translation.trim().is_empty()
                || sentence.translation.len() > 8 * 1024
            {
                return Err(INVALID);
            }
            if index >= active.position && !(index == active.position && active.speaking) {
                additions.insert(index, sentence.translation);
            }
        }
        let bytes: usize = active.pending.iter().filter(|(i, _)| !additions.contains_key(*i)).map(|(_, s)| s.len()).sum::<usize>()
            + additions.values().map(String::len).sum::<usize>();
        let count = active.pending.keys().filter(|i| !additions.contains_key(*i)).count() + additions.len();
        if bytes > MAX_PENDING_BYTES || count > 2000 {
            return Err("tts_live:queue_full");
        }
        active.pending.extend(additions);
        self.changed.send_replace(());
        Ok(())
    }

    pub(super) fn cancel(&self, session_id: &str, generation: u32, epoch: u32) -> bool {
        let mut queue = self.inner.lock().unwrap();
        if queue.session_id != session_id || queue.session_generation != generation || epoch <= queue.epoch {
            return false;
        }
        queue.epoch = epoch;
        if let Some(active) = queue.active.take() {
            active.run.cancel.store(true, Ordering::Release);
        }
        self.changed.send_replace(());
        true
    }

    async fn next(&self, run: &Run) -> Result<Option<Item>, &'static str> {
        let mut changed = self.changed.subscribe();
        loop {
            {
                let mut queue = self.inner.lock().unwrap();
                if let Some(item) = queue.take_next(run)? {
                    return Ok(Some(item));
                }
                let active = queue.active.as_ref().ok_or(CANCELLED)?;
                if active.position == active.units.len() {
                    return Ok(None);
                }
            }
            tokio::select! {
                biased;
                _ = wait_cancelled(&run.cancel) => return Err(CANCELLED),
                _ = changed.changed() => {},
            }
        }
    }
}

async fn finish_engine_start(
    cancel: &AtomicBool,
    start: impl std::future::Future<Output = Result<(), String>>,
) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    // Wie im normalen Vorlesepfad den Start zu Ende abwarten: nach dem Spawn
    // müssen Health-Polling und Timeout auch bei Sprechabbruch weiterlaufen.
    // Sonst bleiben Prozess und Phase Starting ohne Idle-Watchdog zurück.
    start.await?;
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    Ok(())
}

impl TtsManager {
    /// A2b wählt vorher die Zielstimme über die bestehenden TTS-Einstellungen.
    /// Aufruf läuft bis zum Quellende; push darf währenddessen nachliefern.
    pub async fn live_speak(self: &Arc<Self>, request: LiveSpeechRequest) -> Result<(), String> {
        let _update_start = crate::update_gate::pruefe_start(&self.app, crate::update_gate::Grund::Vorlesen)?;
        let units = validate(&request)?;
        let (run, generation) = {
            let mut queue = self.live_speech.inner.lock().unwrap();
            queue.check_request(&request)?;
            // Wie der normale Sprechauftrag: ein Flag über Warten, Synthese und Audio.
            let cancel = Arc::new(AtomicBool::new(false));
            let generation = self.core.generation.fetch_add(1, Ordering::AcqRel) + 1;
            let mut slot = self.core.cancelled.lock().unwrap();
            slot.store(true, Ordering::Release);
            *slot = cancel.clone();
            let run = queue.begin(request, units, cancel);
            self.live_speech.changed.send_replace(());
            self.core.set_phase(TtsPhase::Speaking, None);
            *self.core.last_used.lock().unwrap() = std::time::Instant::now();
            let _ = self.app.emit("tts-speak-progress", queue.progress(&run)?);
            (run, generation)
        };
        *self.speak_session.lock().unwrap() = None;
        self.provider_preview.cancel_active();
        let result = self.run_live_speech(&run).await;
        let mut queue = self.live_speech.inner.lock().unwrap();
        if self.core.generation.load(Ordering::Acquire) == generation {
            *self.core.last_used.lock().unwrap() = std::time::Instant::now();
            if !run.cancel.load(Ordering::Acquire) {
                match &result {
                    Ok(()) => self.core.set_phase(TtsPhase::Ready, None),
                    Err(e) => self.core.set_phase(TtsPhase::Error, Some(e.clone())),
                }
            } else if self.core.phase() == TtsPhase::Speaking {
                self.core.set_phase(TtsPhase::Ready, None);
            }
        }
        if queue.active.as_ref().is_some_and(|a| a.run.id == run.id) {
            queue.active = None;
        }
        result
    }

    async fn run_live_speech(self: &Arc<Self>, run: &Run) -> Result<(), String> {
        while let Some(item) = self.live_speech.next(run).await? {
            // Keine Sprecher-Tags aus Modellantworten auswerten. Lange Zieltexte
            // vollständig teilen; alle Stücke gehören weiter zu EINEM Originalsatz.
            self.refresh_from_settings();
            let max_chars = *self.core.max_chars.lock().unwrap();
            let parts = protocol::split_oversized(&item.translation, max_chars);
            loop {
                let epoch = self.core.voice_epoch();
                self.refresh_from_settings();
                finish_engine_start(&run.cancel, self.ensure_server_for(&parts)).await?;
                self.bind_seed_voice().await;
                if self.core.voice_epoch() == epoch {
                    break;
                }
            }
            if run.cancel.load(Ordering::Acquire) {
                return Err(CANCELLED.into());
            }
            {
                let queue = self.live_speech.inner.lock().unwrap();
                if !queue.current(run) {
                    return Err(CANCELLED.into());
                }
                // ensure_engine_ready kann Ready gesetzt haben. Auch das Warten
                // auf den nächsten Zieltext gehört noch zum laufenden Vorlesen.
                self.core.set_phase(TtsPhase::Speaking, None);
            }
            let item = Arc::new(item);
            let manager = Arc::clone(self);
            let callback_run = run.clone();
            let callback_item = item.clone();
            let on_playing: OnPlaying = Arc::new(move |_, timing, piece| {
                let queue = manager.live_speech.inner.lock().unwrap();
                if queue.current(&callback_run) {
                    let event = if piece == 0 { "tts-current-sentence" } else { "tts-sentence-piece" };
                    let _ = manager.app.emit(event, callback_item.current_payload(timing, piece));
                }
            });
            // Den Kern-Future zu Ende abwarten: er wartet auch auf den Audio-Thread
            // nach Abbruch. Kein abgelöster Player, der noch alte Ereignisse sendet.
            self.core.fetch_and_play_pipelined(
                &super::single_voice(parts), 0, run.cancel.clone(), Some(on_playing), None, &[],
            ).await?;
            let mut queue = self.live_speech.inner.lock().unwrap();
            let payload = queue.complete(run, &item)?;
            let _ = self.app.emit("tts-speak-progress", payload);
        }
        Ok(())
    }

    pub fn live_speech_push(&self, session_id: &str, generation: u32, epoch: u32, sentences: Vec<LiveSpeechSentence>) -> Result<(), String> {
        self.live_speech.push(session_id, generation, epoch, sentences).map_err(str::to_string)
    }

    pub fn live_speech_cancel(&self, session_id: &str, generation: u32, epoch: u32) -> bool {
        self.live_speech.cancel(session_id, generation, epoch)
    }
}

#[cfg(test)]
#[path = "live/tests.rs"]
mod tests;
