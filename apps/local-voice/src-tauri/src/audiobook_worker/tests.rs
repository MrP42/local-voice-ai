use super::*;
use std::io::Cursor;
use std::time::{Duration, Instant};

#[test]
fn oversized_frame_is_rejected_before_payload_read_or_write() {
    struct HeaderOnly(Cursor<Vec<u8>>);
    impl std::io::Read for HeaderOnly {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.0.position() < 4, "payload must not be read");
            std::io::Read::read(&mut self.0, bytes)
        }
    }
    let header = ((protocol::MAX_FRAME_BYTES + 1) as u32).to_le_bytes().to_vec();
    let error = protocol::read::<Request>(&mut HeaderOnly(Cursor::new(header))).err().unwrap();
    assert_eq!(error, WorkerError::Limit);
    let mut output = Vec::new();
    assert_eq!(protocol::write(&mut output, &"x".repeat(protocol::MAX_FRAME_BYTES)), Err(WorkerError::Limit));
    assert!(output.is_empty());
    // Gegenprobe: ein kleiner Frame darf denselben Writer passieren.
    protocol::write(&mut output, &"synthetic").unwrap();
    assert_eq!(protocol::read::<String>(&mut Cursor::new(output)).unwrap().as_deref(), Some("synthetic"));
}

#[test]
fn protocol_rejects_lengths_versions_garbage_and_truncation() {
    let valid = Request::Shutdown;
    let mut bytes = Vec::new();
    protocol::write(&mut bytes, &valid).unwrap();
    assert!(matches!(protocol::read(&mut Cursor::new(&bytes)), Ok(Some(Request::Shutdown))));
    assert!(protocol::read::<Request>(&mut Cursor::new([])).unwrap().is_none());
    for bad in [
        ((protocol::MAX_FRAME_BYTES + 1) as u32).to_le_bytes().to_vec(),
        [vec![1, 0, 0, 0], vec![b'x']].concat(),
        [vec![9, 0, 0, 0], b"{\"v\":999}".to_vec()].concat(),
        bytes[..bytes.len() - 1].to_vec(),
        vec![1, 0],
    ] {
        assert!(protocol::read::<Request>(&mut Cursor::new(bad)).is_err());
    }
    // Gegenprobe: dieselbe gueltige Nachricht mit fremder Version.
    let mut wrong = bytes.clone();
    let body = String::from_utf8(wrong.split_off(4)).unwrap().replace("\"version\":1", "\"version\":2");
    let mut wrong = (body.len() as u32).to_le_bytes().to_vec();
    wrong.extend(body.as_bytes());
    assert_eq!(protocol::read::<Request>(&mut Cursor::new(wrong)).err().unwrap(), WorkerError::Version);
}

#[test]
fn invalid_samples_and_payloads_have_only_fixed_error_codes() {
    let request = Request::Infer { generation: 7, samples: vec![0.0; protocol::MAX_SAMPLES + 1] };
    assert_eq!(request.validate(), Err(WorkerError::Limit));
    let mut input = Vec::new();
    protocol::write(&mut input, &request).unwrap();
    let mut output = Vec::new();
    assert_eq!(serve(&mut Cursor::new(input), &mut output), Err(WorkerError::Limit));
    assert!(output.is_empty());
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(Request::Infer { generation: 7, samples: vec![value] }.validate(), Err(WorkerError::Invalid));
    }
    assert!(Request::Infer { generation: 7, samples: vec![0.0; protocol::MAX_SAMPLES] }.validate().is_ok());
    let secret = b"synthetic-private-marker";
    let mut bytes = (secret.len() as u32).to_le_bytes().to_vec();
    bytes.extend(secret);
    let error = protocol::read::<Request>(&mut Cursor::new(bytes)).err().unwrap();
    assert_eq!(format!("{error:?}"), "Invalid");
    assert!(!format!("{error:?}").contains("synthetic-private-marker"));
}

struct Fake {
    response: Option<Reply>,
    dead: bool,
}

impl InferenceWorker for Fake {
    fn start(&mut self, _: u64, _: Vec<f32>) -> Result<(), WorkerError> { Ok(()) }
    fn poll(&mut self) -> Result<Option<Reply>, WorkerError> { Ok(self.response.take()) }
    fn terminate(&mut self) -> Result<(), WorkerError> { self.dead = true; Ok(()) }
}

fn session() -> InferenceSession<Fake> {
    InferenceSession::new(Fake { response: None, dead: false })
}

#[test]
fn four_hanging_workers_share_one_deadline_and_discard_late_results() {
    let now = Instant::now();
    let deadline = now + Duration::from_secs(2);
    let mut sessions: Vec<_> = (0..4).map(|_| session()).collect();
    for session in &mut sessions {
        session.start(7, vec![0.0]).unwrap();
        session.request_stop(deadline).unwrap();
        assert!(session.tick(now).unwrap().is_none());
        assert!(!session.worker.dead);
        session.worker.response = Some(Reply::Result { generation: 7, segments: vec![], detected_language: None });
    }
    for session in &mut sessions {
        assert!(session.tick(deadline).unwrap().is_none());
        assert!(session.worker.dead);
        assert!(session.tick(deadline).unwrap().is_none());
        assert_eq!(session.start(8, vec![0.0]), Err(WorkerError::Stopped));
    }
    assert!(deadline.duration_since(now) < Duration::from_secs(5));
}

#[test]
fn current_result_before_deadline_survives_but_other_generations_do_not() {
    let now = Instant::now();
    let mut session = session();
    session.start(7, vec![0.0]).unwrap();
    session.worker.response = Some(Reply::Result { generation: 6, segments: vec![], detected_language: None });
    assert!(session.tick(now).unwrap().is_none());
    assert_eq!(session.start(8, vec![]), Err(WorkerError::Busy));
    session.request_stop(now + Duration::from_secs(2)).unwrap();
    // Ein weiteres Signal darf die gemeinsame Frist nicht nach hinten schieben.
    session.request_stop(now + Duration::from_secs(20)).unwrap();
    assert_eq!(session.stop_at, Some(now + Duration::from_secs(2)));
    session.worker.response = Some(Reply::Result { generation: 7, segments: vec![], detected_language: None });
    assert!(session.tick(now).unwrap().is_some());
    assert!(session.worker.dead);
}

#[test]
fn crash_does_not_produce_a_committable_result() {
    let mut session = session();
    session.start(1, vec![0.0]).unwrap();
    session.worker.response = Some(Reply::Error { code: WorkerError::Crashed });
    assert_eq!(session.tick(Instant::now()).err().unwrap(), WorkerError::Crashed);
    assert!(session.worker.dead);
    // Gegenprobe: ohne Crash steht ein gueltiges Ergebnis zur Parent-Transaktion bereit.
    let mut session = self::session();
    session.start(1, vec![0.0]).unwrap();
    session.worker.response = Some(Reply::Result { generation: 1, segments: vec![], detected_language: None });
    assert!(session.tick(Instant::now()).unwrap().is_some());
    session.worker.response = Some(Reply::Result { generation: 1, segments: vec![], detected_language: None });
    assert!(session.tick(Instant::now()).unwrap().is_none());
}

#[test]
fn shutdown_and_invalid_load_order_need_no_model() {
    for request in [Request::Shutdown, Request::Infer { generation: 1, samples: vec![0.0] }] {
        let mut input = Vec::new();
        protocol::write(&mut input, &request).unwrap();
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        if matches!(request, Request::Shutdown) {
            assert!(output.is_empty());
        } else {
            assert!(matches!(protocol::read(&mut Cursor::new(output)), Ok(Some(Reply::Error { code: WorkerError::NotLoaded }))));
        }
    }
}
