use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct Fake {
    dead: Arc<AtomicBool>,
    reply: Option<Reply>,
}
impl InferenceWorker for Fake {
    fn start(&mut self, _: u64, _: Vec<f32>) -> Result<(), WorkerError> { Ok(()) }
    fn poll(&mut self) -> Result<Option<Reply>, WorkerError> { Ok(self.reply.take()) }
    fn terminate(&mut self) -> Result<(), WorkerError> { self.dead.store(true, Ordering::Release); Ok(()) }
}

#[test]
fn hanging_trait_workers_use_one_book_deadline_and_never_return_a_block() {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut sessions = Vec::new();
    let mut deaths = Vec::new();
    for _ in 0..4 {
        let dead = Arc::new(AtomicBool::new(false));
        let mut session = InferenceSession::new(Fake { dead: dead.clone(), reply: None });
        session.start(7, vec![0.0]).unwrap();
        sessions.push(session);
        deaths.push(dead);
    }
    assert!(deaths.iter().all(|dead| !dead.load(Ordering::Acquire)));
    let start = Instant::now();
    for session in &mut sessions {
        assert_eq!(poll_block(session, || Ok(Some(deadline))).err().as_deref(), Some(audiobook::YIELDED));
    }
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(deaths.iter().all(|dead| dead.load(Ordering::Acquire)));
    // Gegenprobe: ein gueltiges Ergebnis ohne Hold geht zur Commitstelle weiter.
    let mut session = InferenceSession::new(Fake { dead: Arc::new(AtomicBool::new(false)),
        reply: Some(Reply::Result { generation: 7, segments: vec![], detected_language: Some("en".into()) }) });
    session.start(7, vec![0.0]).unwrap();
    assert_eq!(poll_block(&mut session, || Ok(None)).unwrap().detected_language.as_deref(), Some("en"));
}

#[test]
fn book_signal_cannot_extend_a_previous_deadline() {
    let signal = Arc::new(Mutex::new(None));
    let at = Instant::now();
    set_deadline(&signal, at);
    set_deadline(&signal, at + Duration::from_secs(20));
    assert_eq!(*signal.lock().unwrap(), Some(at));
    // Gegenprobe: eine neue Registrierung hat keine alte Pause geerbt.
    let fresh: Signal = Arc::new(Mutex::new(None));
    assert!(fresh.lock().unwrap().is_none());
}

#[test]
fn tempo_changes_replace_the_idle_worker_before_the_next_block() {
    let dead = Arc::new(AtomicBool::new(false));
    let mut session = Some(InferenceSession::new(Fake { dead: dead.clone(), reply: None }));
    let mut old = (audiobook::AudiobookTempo::Gentle, 1);
    // Gegenprobe: unveraendertes Tempo behaelt das geladene Modell.
    refresh_profile(&mut session, &mut old, (audiobook::AudiobookTempo::Gentle, 1));
    assert!(session.is_some());
    assert!(!dead.load(Ordering::Acquire));
    refresh_profile(&mut session, &mut old, (audiobook::AudiobookTempo::Fast, 4));
    assert!(session.is_none());
    assert!(dead.load(Ordering::Acquire));
    assert_eq!(old, (audiobook::AudiobookTempo::Fast, 4));
}

#[test]
fn loaded_worker_is_counted_without_deducting_its_memory_twice() {
    let ledger = crate::managers::engine_ledger::Ledger::new();
    let lease = ledger.worker_started();
    assert_eq!(ledger.worker_count(), 1);
    assert_eq!(ledger.pending_mb(), (0, 0));
    drop(lease);
    assert_eq!(ledger.worker_count(), 0);
}

#[test]
fn crashes_restart_twice_and_a_successful_block_needs_no_restart() {
    for fail in [true, false] {
        let mut starts = 0;
        let mut grants = 0;
        let mut session = None;
        let result = retry_block(&mut session,
            || {
                starts += 1;
                Ok(InferenceSession::new(Fake { dead: Arc::new(AtomicBool::new(false)),
                    reply: Some(if fail { Reply::Error { code: WorkerError::Crashed } }
                        else { Reply::Result { generation: 7, segments: vec![], detected_language: None } }) }))
            },
            |session| {
                session.start(7, vec![0.0]).unwrap();
                poll_block(session, || Ok(None))
            },
            || { grants += 1; Ok(()) },
        );
        if fail {
            assert_eq!(result.err().as_deref(), Some("audiobook_worker_crashed"));
            assert_eq!(starts, 3, "erster Prozess und hoechstens zwei Neustarts");
            assert_eq!(grants, 2, "jede Wiederholung hat ein neues Tor");
            assert!(session.is_none());
        } else {
            assert_eq!(result.unwrap().generation, 7);
            assert_eq!(starts, 1);
            assert_eq!(grants, 0);
        }
    }
}

#[test]
fn a_stop_never_restarts_a_worker() {
    let mut session = None::<InferenceSession<Fake>>;
    let mut loads = 0;
    let result = retry_block(&mut session,
        || { loads += 1; Err(audiobook::YIELDED.into()) },
        |_| panic!("kein Block nach Stopp"),
        || panic!("kein Neustart nach Stopp"),
    );
    assert_eq!(result.err().as_deref(), Some(audiobook::YIELDED));
    assert_eq!(loads, 1);
}

#[test]
fn startup_failure_falls_back_visibly_but_stop_and_memory_never_do() {
    for code in ["audiobook_worker_start_failed", "audiobook_worker_load_failed"] {
        let mut notice = None;
        let backend = start_or_fallback(|| Err::<(), _>(code.into()), |reason| {
            notice = Some(reason);
            Ok("in_process")
        }).unwrap();
        assert!(matches!(backend, Backend::Fallback("in_process")));
        assert_eq!(notice.as_deref(), Some(code));
    }
    // Gegenproben: erfolgreicher Start, Pause und Speichermangel laden keine Ersatzengine.
    assert!(matches!(start_or_fallback(|| Ok::<_, String>(7), |_| Err::<(), _>("fallback".into())).unwrap(), Backend::Worker(7)));
    for code in [audiobook::YIELDED, "audiobook_memory"] {
        assert_eq!(start_or_fallback(|| Err::<(), _>(code.into()), |_| -> Result<(), String> {
            panic!("kein Rueckfall bei Pause oder Speichermangel")
        }).err().as_deref(), Some(code));
    }
}

#[test]
fn book_action_gives_all_registered_parts_the_same_deadline() {
    let ids: Vec<_> = (0..4).map(|i| format!("synthetic-stop-book-{i}")).collect();
    let signals: Vec<Signal> = ids.iter().map(|_| Arc::new(Mutex::new(None))).collect();
    {
        let mut registry = WORKERS.get_or_init(Default::default).lock().unwrap();
        for (id, signal) in ids.iter().zip(&signals) { registry.insert(id.clone(), Arc::downgrade(signal)); }
    }
    let at = Instant::now() + Duration::from_secs(2);
    stop_books(&ids, at);
    stop_books(&ids, at + Duration::from_secs(20));
    for signal in &signals { assert_eq!(*signal.lock().unwrap(), Some(at)); }
    let mut registry = WORKERS.get().unwrap().lock().unwrap();
    for id in &ids { registry.remove(id); }
}
