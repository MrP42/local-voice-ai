use super::*;
use std::time::Duration;

fn request(epoch: u32, offset: u32) -> LiveSpeechRequest {
    LiveSpeechRequest {
        session_id: "synthetic".into(),
        session_generation: 1,
        page_id: "page".into(),
        text: "😀 Eins. Zwei! Drei?".into(),
        target_lang: "en".into(),
        source_offset: offset,
        epoch,
    }
}

fn begin(state: &LiveSpeech, epoch: u32, offset: u32) -> Run {
    let request = request(epoch, offset);
    let units = validate(&request).unwrap();
    state.inner.lock().unwrap().begin(request, units, Arc::new(AtomicBool::new(false)))
}

fn sentence(start: u32, end: u32, text: &str) -> LiveSpeechSentence {
    LiveSpeechSentence { source_start: start, source_end: end, translation: text.into() }
}

#[test]
fn source_coordinates_survive_different_translation_lengths_and_splitting() {
    let state = LiveSpeech::default();
    let run = begin(&state, 1, 9);
    state.push("synthetic", 1, 1, vec![sentence(9, 14, "A much longer target. Another sentence!")]).unwrap();
    let mut queue = state.inner.lock().unwrap();
    let item = queue.take_next(&run).unwrap().unwrap();
    assert_eq!(item.index, 1);
    assert_eq!((item.start, item.end), (9, 14));
    assert_eq!(item.original, "Zwei!");
    let payload = item.current_payload(&SentenceTiming::default(), 0);
    assert_eq!(payload["index"], 1);
    assert_eq!(payload["source_start"], 9);
    assert_eq!(payload["text"], "Zwei!");
    assert_eq!(queue.progress(&run).unwrap()["source_offset"], 9);
    let progress = queue.complete(&run, &item).unwrap();
    assert_eq!(progress["position"], 2);
    assert_eq!(progress["total"], 3);
    assert_eq!(progress["source_offset"], 14);
    // Gegenprobe: Zieltext-Indizes/-Längen (0 bzw. 38) wären hier falsch.
    assert_ne!(payload["source_end"], item.translation.encode_utf16().count() as u32);
}

#[tokio::test]
async fn missing_sentence_waits_even_when_later_translation_is_ready() {
    let state = LiveSpeech::default();
    let run = begin(&state, 1, 0);
    state.push("synthetic", 1, 1, vec![sentence(9, 14, "Second!")]).unwrap();
    // Gegenprobe: Überspringen der Lücke oder Rückfall aufs Original würde sofort liefern.
    assert!(tokio::time::timeout(Duration::from_millis(40), state.next(&run)).await.is_err());
    state.push("synthetic", 1, 1, vec![sentence(0, 8, "First.")]).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), state.next(&run)).await.unwrap().unwrap().unwrap();
    assert_eq!(first.translation, "First.");
    state.inner.lock().unwrap().complete(&run, &first).unwrap();
    assert_eq!(state.next(&run).await.unwrap().unwrap().translation, "Second!");
}

#[tokio::test]
async fn epoch_change_rejects_old_deliveries_and_completion() {
    let state = LiveSpeech::default();
    let old = begin(&state, 1, 0);
    state.push("synthetic", 1, 1, vec![sentence(0, 8, "Old.")]).unwrap();
    let item = state.next(&old).await.unwrap().unwrap();
    let new = begin(&state, 2, 9);
    assert!(old.cancel.load(Ordering::Acquire));
    assert!(state.push("synthetic", 1, 1, vec![sentence(9, 14, "Late!")]).is_err());
    assert!(state.inner.lock().unwrap().complete(&old, &item).is_err());
    assert!(state.next(&old).await.is_err());
    assert!(state.inner.lock().unwrap().take_next(&new).unwrap().is_none());
    // Gegenprobe: Gleichsetzen von alter und neuer Epoche ließe Late/Old durch.
    assert!(state.inner.lock().unwrap().check_request(&request(2, 9)).is_err());
}

#[tokio::test]
async fn cancel_wakes_waiter_and_cannot_cancel_a_new_epoch() {
    let state = LiveSpeech::default();
    let run = begin(&state, 1, 0);
    assert!(!state.cancel("other", 1, 2));
    let (waiting, ()) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(1), state.next(&run)),
        async {
            tokio::task::yield_now().await;
            assert!(state.cancel("synthetic", 1, 2));
        },
    );
    assert!(waiting.unwrap().is_err());
    assert!(state.push("synthetic", 1, 1, vec![sentence(0, 8, "Late.")]).is_err());
    let new = begin(&state, 3, 9);
    assert!(!state.cancel("synthetic", 1, 2));
    assert!(!new.cancel.load(Ordering::Acquire));
    // Auch normales tts_cancel / ein neuer TTS-Auftrag setzt dieses Kern-Flag.
    let (waiting, ()) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(1), state.next(&new)),
        async {
            tokio::task::yield_now().await;
            new.cancel.store(true, Ordering::Release);
        },
    );
    assert!(waiting.unwrap().is_err());
}

#[test]
fn malformed_spans_empty_targets_and_stale_sessions_are_rejected_atomically() {
    let state = LiveSpeech::default();
    let run = begin(&state, 1, 0);
    assert!(state.push("synthetic", 1, 1, vec![sentence(0, 8, "Good."), sentence(10, 14, "Bad!")]).is_err());
    assert!(state.push("synthetic", 1, 1, vec![sentence(0, 8, " ")]).is_err());
    assert!(state.push("synthetic", 2, 1, vec![sentence(0, 8, "Wrong session.")]).is_err());
    assert!(state.inner.lock().unwrap().take_next(&run).unwrap().is_none());
    let mut stale = request(9, 0);
    stale.session_id = "other".into();
    assert!(state.inner.lock().unwrap().check_request(&stale).is_err());
}

#[tokio::test]
async fn last_sentence_finishes_only_after_completion_and_duplicates_do_not_repeat() {
    let state = LiveSpeech::default();
    let run = begin(&state, 1, 15);
    state.push("synthetic", 1, 1, vec![sentence(15, 20, "Third?")]).unwrap();
    let item = state.next(&run).await.unwrap().unwrap();
    assert_eq!(item.index, 2);
    assert_eq!(state.inner.lock().unwrap().progress(&run).unwrap()["position"], 2);
    state.push("synthetic", 1, 1, vec![sentence(15, 20, "Duplicate?")]).unwrap();
    let progress = state.inner.lock().unwrap().complete(&run, &item).unwrap();
    assert_eq!(progress["position"], 3);
    assert_eq!(progress["source_offset"], 20);
    assert!(state.next(&run).await.unwrap().is_none());
    assert!(state.inner.lock().unwrap().complete(&run, &item).is_err());
}

#[test]
fn invalid_start_does_not_replace_the_active_session() {
    let state = LiveSpeech::default();
    let run = begin(&state, 1, 0);
    for invalid in [
        LiveSpeechRequest { source_offset: 21, ..request(2, 0) },
        LiveSpeechRequest { target_lang: "\nen".into(), ..request(2, 0) },
        LiveSpeechRequest { session_generation: 0, ..request(2, 0) },
        LiveSpeechRequest { epoch: 0, ..request(2, 0) },
    ] {
        assert!(validate(&invalid).is_err());
        assert!(state.inner.lock().unwrap().current(&run));
    }
}

// Mock der Manager-/Prozessgrenze: Spawn setzt Starting, erst Health bzw.
// Start-Timeout beendet den Versuch. Kein echter Prozess und kein Audio.
struct MockStartup {
    core: super::super::TtsCore,
    running: AtomicBool,
    finished: AtomicBool,
}

impl MockStartup {
    fn new() -> Self {
        Self {
            core: super::super::TtsCore::for_test(0),
            running: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    async fn start(
        &self,
        started: tokio::sync::oneshot::Sender<()>,
        health: tokio::sync::oneshot::Receiver<Result<(), String>>,
    ) -> Result<(), String> {
        self.running.store(true, Ordering::Release);
        self.core.owns_server.store(true, Ordering::Release);
        self.core.set_phase(TtsPhase::Starting, None);
        started.send(()).unwrap();
        let result = health.await.unwrap();
        match &result {
            Ok(()) => self.core.set_phase(TtsPhase::Ready, None),
            Err(error) => {
                self.running.store(false, Ordering::Release);
                self.core.owns_server.store(false, Ordering::Release);
                self.core.set_phase(TtsPhase::Error, Some(error.clone()));
            }
        }
        self.finished.store(true, Ordering::Release);
        result
    }
}

#[tokio::test]
async fn cancel_or_seek_during_start_finishes_health_polling_before_returning() {
    for seek in [false, true] {
        for fail_start in [false, true] {
            let state = LiveSpeech::default();
            let run = begin(&state, 1, 0);
            let startup = MockStartup::new();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (health_tx, health_rx) = tokio::sync::oneshot::channel();
            let start = finish_engine_start(&run.cancel, startup.start(started_tx, health_rx));
            tokio::pin!(start);
            tokio::time::timeout(Duration::from_secs(1), async {
                tokio::select! {
                    result = &mut start => panic!("Start vor Health beendet: {result:?}"),
                    result = started_rx => result.unwrap(),
                }
            }).await.unwrap();
            assert_eq!(startup.core.phase(), TtsPhase::Starting);
            if seek {
                begin(&state, 2, 9);
            } else {
                assert!(state.cancel("synthetic", 1, 2));
            }
            assert!(run.cancel.load(Ordering::Acquire));
            // Gegenprobe: die alte select!-Logik kehrt nach dem Abbruchpoll
            // (25 ms) zurück und verwirft den noch startenden Prozesspfad.
            // Nur die Leihgabe an timeout verwerfen, nicht den Start-Future.
            assert!(tokio::time::timeout(Duration::from_millis(50), &mut start).await.is_err());
            let health = if fail_start { Err("synthetic startup timeout".into()) } else { Ok(()) };
            health_tx.send(health).unwrap();
            let result = tokio::time::timeout(Duration::from_secs(1), &mut start).await.unwrap();
            assert!(startup.finished.load(Ordering::Acquire), "Start-Future verworfen");
            assert_ne!(startup.core.phase(), TtsPhase::Starting);
            if fail_start {
                assert_eq!(result.unwrap_err(), "synthetic startup timeout");
                assert_eq!(startup.core.phase(), TtsPhase::Error);
                assert!(!startup.running.load(Ordering::Acquire));
                assert!(!startup.core.owns_server());
            } else {
                assert_eq!(result.unwrap_err(), CANCELLED);
                assert_eq!(startup.core.phase(), TtsPhase::Ready);
                assert!(startup.running.load(Ordering::Acquire));
                assert!(super::super::state::should_idle_stop(60, 1, startup.core.owns_server(), startup.core.phase()));
            }
        }
    }
}

#[tokio::test]
async fn already_cancelled_run_does_not_start_an_engine() {
    let cancel = AtomicBool::new(true);
    let result = finish_engine_start(&cancel, async {
        panic!("abgebrochener Auftrag darf keinen Prozess starten");
    }).await;
    assert_eq!(result.unwrap_err(), CANCELLED);
}
