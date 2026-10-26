//! Wiederverwendbarer Vertragstest-Satz für [`TtsProvider`].
//!
//! Jeder neue Adapter (Paket P2: phonon, openai-speech, ollama-speech) ruft
//! in seinen Tests `contract_suite` auf — mit einem Exemplar gegen eine
//! Fake-EXE bzw. einen Fake-Server — und erbt damit alle Pflichten des
//! Vertrags, statt sie neu zu erfinden:
//!
//! ```ignore
//! contract_suite(
//!     Arc::new(PhononProvider::for_test(fake_exe)),
//!     ContractExpect::available()
//!         .with_slow(Arc::new(PhononProvider::for_test(slow_fake_exe))),
//! )
//! .await;
//! ```
//!
//! Was geprüft wird (jeder Punkt ist auch einzeln aufrufbar):
//! * `check_descriptor`: Kennung, Anzeigename, Lizenzfelder (code, weights,
//!   source_url nicht leer; `verified` nur mit Prüfdatum, sonst eine `note`),
//!   Fähigkeiten vollständig, Deskriptor und Cache-Tag stabil;
//! * `check_health`: antwortet in 5 s, meldet bei „nicht verfügbar“ einen
//!   `reason`, bei „verfügbar“ keinen;
//! * `check_voices`: Stimmen vollständig, eindeutig, Sprachfilter stimmt;
//! * `check_synthesize`: gültiges WAV, Format-Metadaten passend zum Header,
//!   Dauer > 0; leerer Text ist `BadRequest`; nicht verfügbar → `Unavailable`;
//! * `check_stream`: je angebotenem Format: `seq` ab 0 lückenlos und
//!   monoton, genau EIN `is_last` am Ende, `request_id` aus dem Token,
//!   Chunk-Daten zum Format passend, „Chunk“-Modus wirklich mit mehreren
//!   Teilen; weggefallener Empfänger → `Cancelled`;
//! * `check_cancel`: vorab abgebrochen → `Cancelled` ohne Chunk; mitten im
//!   Lauf abgebrochen → `Cancelled` (braucht `slow`);
//! * `check_timeout`: Frist → `Timeout`, nicht wiederholbar (braucht `slow`).
//!
//! `slow` ist ein Exemplar desselben Anbieters, das für eine Synthese
//! deutlich länger als eine Sekunde braucht (Verzögerung im Fake). Ohne
//! `slow` entfallen die zwei Prüfungen „mitten im Lauf“ und „Frist“; der
//! Aufrufer sollte das im Testnamen vermerken.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::contract::{
    is_valid_provider_id, language_matches, AudioChunk, AudioEncoding, CancelToken, StreamingMode,
    SynthesisOptions, TtsError, TtsErrorKind, TtsProvider,
};
use super::wav::parse_wav;

/// Frist, in der Gesundheit, Stimmenliste und schnelle Fehlerpfade antworten müssen.
const QUICK: Duration = Duration::from_secs(5);

/// Was der Aufrufer vom getesteten Exemplar erwartet.
#[derive(Clone)]
pub(crate) struct ContractExpect {
    /// Soll `health_check` „verfügbar“ melden?
    pub available: bool,
    /// Erwartete Abtastrate des Clips (`None` = nicht prüfen).
    pub sample_rate: Option<u32>,
    /// Probetext; muss für den Anbieter gültig und lang genug sein, dass ein
    /// Chunk-Anbieter mehrere Teile liefert.
    pub text: String,
    pub language: String,
    pub voice: Option<String>,
    /// Langsames Exemplar für Abbruch/Frist (siehe Moduldoku).
    pub slow: Option<Arc<dyn TtsProvider>>,
}

impl ContractExpect {
    pub fn available() -> Self {
        Self {
            available: true,
            sample_rate: None,
            text: "Dies ist ein kurzer Satz für den Test. Und hier folgt ein zweiter Satz."
                .to_string(),
            language: "de".to_string(),
            voice: None,
            slow: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            available: false,
            ..Self::available()
        }
    }

    pub fn with_slow(mut self, slow: Arc<dyn TtsProvider>) -> Self {
        self.slow = Some(slow);
        self
    }

    pub fn with_sample_rate(mut self, rate: u32) -> Self {
        self.sample_rate = Some(rate);
        self
    }

    pub fn with_voice(mut self, voice: &str) -> Self {
        self.voice = Some(voice.to_string());
        self
    }

    fn opts(&self, timeout: Duration) -> SynthesisOptions {
        SynthesisOptions {
            language: self.language.clone(),
            voice: self.voice.clone(),
            timeout,
            ..Default::default()
        }
    }
}

/// Alle Prüfungen nacheinander. Fehlschlag = `panic!` mit Anbieterkennung.
pub(crate) async fn contract_suite(p: Arc<dyn TtsProvider>, expect: ContractExpect) {
    check_descriptor(&p);
    check_health(&p, &expect).await;
    check_voices(&p, &expect).await;
    check_synthesize(&p, &expect).await;
    check_stream(&p, &expect).await;
    check_cancel(&p, &expect).await;
    check_timeout(&expect).await;
}

fn id_of(p: &Arc<dyn TtsProvider>) -> String {
    p.descriptor().id
}

/// Fehlerform: Meldung vorhanden, nie wiederholbar bei Klassen, bei denen
/// Wiederholung sinnlos oder schädlich ist.
fn assert_error_shape(who: &str, e: &TtsError) {
    assert!(
        !e.message.trim().is_empty(),
        "[{who}] Fehler ohne Meldung: {e:?}"
    );
    if !matches!(e.kind, TtsErrorKind::Network | TtsErrorKind::Backend) {
        assert!(
            !e.retryable,
            "[{who}] {:?} darf nicht wiederholbar sein",
            e.kind
        );
    }
}

fn assert_full_date(who: &str, d: &str) {
    let ok = d.len() == 10
        && d.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    assert!(ok, "[{who}] verified_on ist kein JJJJ-MM-TT: {d:?}");
}

pub(crate) fn check_descriptor(p: &Arc<dyn TtsProvider>) {
    let d = p.descriptor();
    let who = d.id.clone();
    assert!(is_valid_provider_id(&d.id), "[{who}] ungültige Kennung");
    assert!(!d.label.trim().is_empty(), "[{who}] Anzeigename fehlt");
    assert_eq!(d, p.descriptor(), "[{who}] Deskriptor ist nicht stabil");

    let l = &d.license;
    for (name, v) in [
        ("code", &l.code),
        ("weights", &l.weights),
        ("source_url", &l.source_url),
    ] {
        assert!(!v.trim().is_empty(), "[{who}] Lizenzfeld {name} ist leer");
    }
    if l.verified {
        assert_full_date(
            &who,
            l.verified_on
                .as_deref()
                .unwrap_or_else(|| panic!("[{who}] verified ohne verified_on")),
        );
    } else {
        assert!(
            !l.note.trim().is_empty(),
            "[{who}] unverifizierte Lizenz braucht eine note"
        );
    }

    let c = &d.caps;
    assert!(
        !c.languages.is_empty() && c.languages.iter().all(|s| !s.trim().is_empty()),
        "[{who}] Sprachen fehlen"
    );
    assert!(!c.formats.is_empty(), "[{who}] Formate fehlen");
    assert!(
        !c.sample_rates.is_empty() && c.sample_rates.iter().all(|r| *r > 0),
        "[{who}] Abtastraten fehlen"
    );
    assert!(!c.devices.is_empty(), "[{who}] Geräte fehlen");
    assert!(
        !c.platforms.is_empty() && c.platforms.iter().all(|s| !s.trim().is_empty()),
        "[{who}] Plattformen fehlen"
    );

    // Cache-Tag: deterministisch; wenn nicht leer (Fish: Legacy-Regel), dann
    // mit der Kennung beginnend und je Stimme verschieden.
    let (a1, a2, b) = (
        p.cache_tag(Some("a")),
        p.cache_tag(Some("a")),
        p.cache_tag(Some("b")),
    );
    assert_eq!(a1, a2, "[{who}] Cache-Tag ist nicht deterministisch");
    if !a1.is_empty() {
        assert!(
            a1.starts_with(&d.id),
            "[{who}] Cache-Tag {a1:?} beginnt nicht mit der Kennung"
        );
        assert_ne!(a1, b, "[{who}] Cache-Tag unterscheidet Stimmen nicht");
    }
}

pub(crate) async fn check_health(p: &Arc<dyn TtsProvider>, expect: &ContractExpect) {
    let who = id_of(p);
    let h = tokio::time::timeout(QUICK, p.health_check())
        .await
        .unwrap_or_else(|_| panic!("[{who}] health_check hängt"));
    assert_eq!(h.available, expect.available, "[{who}] Gesundheit: {h:?}");
    if h.available {
        assert!(
            h.reason.is_none(),
            "[{who}] verfügbar, aber mit reason: {h:?}"
        );
    } else {
        assert!(
            h.reason.as_deref().is_some_and(|r| !r.trim().is_empty()),
            "[{who}] nicht verfügbar ohne reason: {h:?}"
        );
    }
}

pub(crate) async fn check_voices(p: &Arc<dyn TtsProvider>, expect: &ContractExpect) {
    let who = id_of(p);
    let all = tokio::time::timeout(QUICK, p.list_voices(None))
        .await
        .unwrap_or_else(|_| panic!("[{who}] list_voices hängt"));
    let all = match all {
        Ok(v) => v,
        Err(e) if !expect.available => {
            assert_eq!(e.kind, TtsErrorKind::Unavailable, "[{who}] {e:?}");
            return;
        }
        Err(e) => panic!("[{who}] list_voices schlug fehl: {e:?}"),
    };
    let mut ids = std::collections::HashSet::new();
    for v in &all {
        assert!(!v.id.trim().is_empty(), "[{who}] Stimme ohne Kennung");
        assert!(
            !v.label.trim().is_empty(),
            "[{who}] Stimme {} ohne Anzeigename",
            v.id
        );
        assert!(
            !v.language.trim().is_empty(),
            "[{who}] Stimme {} ohne Sprache",
            v.id
        );
        assert!(
            ids.insert(v.id.clone()),
            "[{who}] Stimmenkennung doppelt: {}",
            v.id
        );
        if let Some(l) = &v.license {
            assert!(!l.code.trim().is_empty(), "[{who}] Stimmenlizenz ohne code");
        }
    }
    let filtered = tokio::time::timeout(QUICK, p.list_voices(Some(&expect.language)))
        .await
        .unwrap_or_else(|_| panic!("[{who}] list_voices(Sprache) hängt"))
        .unwrap_or_else(|e| panic!("[{who}] list_voices(Sprache): {e:?}"));
    for v in &filtered {
        assert!(
            language_matches(&v.language, Some(&expect.language)),
            "[{who}] Sprachfilter {:?} lieferte Stimme {} ({})",
            expect.language,
            v.id,
            v.language
        );
        assert!(
            ids.contains(&v.id),
            "[{who}] gefilterte Stimme {} fehlt in der Gesamtliste",
            v.id
        );
    }
}

pub(crate) async fn check_synthesize(p: &Arc<dyn TtsProvider>, expect: &ContractExpect) {
    let who = id_of(p);
    let opts = expect.opts(Duration::from_secs(30));
    let cancel = CancelToken::new("contract-synth");

    if !expect.available {
        let e = tokio::time::timeout(QUICK, p.synthesize(&expect.text, &opts, &cancel))
            .await
            .unwrap_or_else(|_| panic!("[{who}] synthesize hängt trotz Nichtverfügbarkeit"))
            .expect_err("nicht verfügbarer Anbieter lieferte Audio");
        assert_eq!(e.kind, TtsErrorKind::Unavailable, "[{who}] {e:?}");
        assert_error_shape(&who, &e);
        return;
    }

    let clip = p
        .synthesize(&expect.text, &opts, &cancel)
        .await
        .unwrap_or_else(|e| panic!("[{who}] synthesize schlug fehl: {e:?}"));
    let info = parse_wav(&clip.data).unwrap_or_else(|e| panic!("[{who}] kein gültiges WAV: {e:?}"));
    assert_eq!(
        clip.format.encoding,
        AudioEncoding::Wav,
        "[{who}] Clip ist kein WAV-Format"
    );
    assert_eq!(
        clip.format.sample_rate, info.sample_rate,
        "[{who}] Rate passt nicht zum Header"
    );
    assert_eq!(
        clip.format.channels, info.channels,
        "[{who}] Kanäle passen nicht zum Header"
    );
    assert_eq!(
        clip.duration_ms, info.duration_ms,
        "[{who}] Dauer passt nicht zum Header"
    );
    assert!(
        clip.duration_ms > 0 && info.data_len > 0,
        "[{who}] leeres Audio"
    );
    if let Some(rate) = expect.sample_rate {
        assert_eq!(
            clip.format.sample_rate, rate,
            "[{who}] erwartete Abtastrate"
        );
    }

    for empty in ["", "   \n\t "] {
        let e = tokio::time::timeout(QUICK, p.synthesize(empty, &opts, &CancelToken::detached()))
            .await
            .unwrap_or_else(|_| panic!("[{who}] synthesize(leer) hängt"))
            .expect_err("leerer Text lieferte Audio");
        assert_eq!(
            e.kind,
            TtsErrorKind::BadRequest,
            "[{who}] leerer Text: {e:?}"
        );
        assert_error_shape(&who, &e);
    }
}

async fn run_stream(
    p: &Arc<dyn TtsProvider>,
    text: &str,
    opts: &SynthesisOptions,
    cancel: &CancelToken,
    capacity: usize,
) -> (Result<(), TtsError>, Vec<AudioChunk>) {
    let (tx, mut rx) = mpsc::channel(capacity);
    let produce = p.stream_synthesize(text, opts, cancel, tx);
    let consume = async {
        let mut got = Vec::new();
        while let Some(c) = rx.recv().await {
            got.push(c);
        }
        got
    };
    futures_util::future::join(produce, consume).await
}

pub(crate) async fn check_stream(p: &Arc<dyn TtsProvider>, expect: &ContractExpect) {
    let d = p.descriptor();
    let who = d.id.clone();

    if !expect.available {
        let opts = expect.opts(Duration::from_secs(30));
        let (r, got) = tokio::time::timeout(
            QUICK,
            run_stream(
                p,
                &expect.text,
                &opts,
                &CancelToken::new("contract-stream"),
                8,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("[{who}] Stream hängt trotz Nichtverfügbarkeit"));
        let e = r.expect_err("nicht verfügbarer Anbieter streamte");
        assert_eq!(e.kind, TtsErrorKind::Unavailable, "[{who}] {e:?}");
        assert!(got.is_empty(), "[{who}] Chunks trotz Nichtverfügbarkeit");
        return;
    }

    for encoding in d.caps.formats.clone() {
        let mut opts = expect.opts(Duration::from_secs(30));
        opts.format = encoding;
        let cancel = CancelToken::new("contract-stream");
        let (r, got) = run_stream(p, &expect.text, &opts, &cancel, 16).await;
        r.unwrap_or_else(|e| panic!("[{who}] Stream ({encoding:?}) schlug fehl: {e:?}"));
        assert!(!got.is_empty(), "[{who}] Stream ({encoding:?}) ohne Chunks");

        let mut last_elapsed = 0;
        for (i, c) in got.iter().enumerate() {
            assert_eq!(c.seq, i as u32, "[{who}] seq nicht lückenlos ab 0");
            assert_eq!(
                c.request_id, "contract-stream",
                "[{who}] request_id nicht aus dem Token"
            );
            assert_eq!(
                c.is_last,
                i + 1 == got.len(),
                "[{who}] is_last nicht genau am Ende"
            );
            assert!(
                c.elapsed_ms >= last_elapsed,
                "[{who}] elapsed_ms nicht monoton"
            );
            last_elapsed = c.elapsed_ms;
            assert_eq!(
                c.format.encoding, encoding,
                "[{who}] Chunk-Kodierung != angefordert"
            );
            assert!(
                c.format.sample_rate > 0 && c.format.channels > 0,
                "[{who}] Chunk-Format leer"
            );
            assert!(!c.data.is_empty(), "[{who}] leerer Chunk {i}");
            match encoding {
                AudioEncoding::Pcm16Le | AudioEncoding::F32Le => {
                    let frame =
                        encoding.bytes_per_sample().unwrap() as usize * c.format.channels as usize;
                    assert_eq!(
                        c.data.len() % frame,
                        0,
                        "[{who}] Chunk {i} endet im halben Sample"
                    );
                }
                AudioEncoding::Wav => {
                    let info = parse_wav(&c.data)
                        .unwrap_or_else(|e| panic!("[{who}] Wav-Chunk {i} ungültig: {e:?}"));
                    assert_eq!(info.sample_rate, c.format.sample_rate, "[{who}] Chunk-Rate");
                    assert_eq!(info.channels, c.format.channels, "[{who}] Chunk-Kanäle");
                }
            }
        }
        if d.caps.streaming == StreamingMode::Chunk {
            assert!(
                got.len() >= 2,
                "[{who}] deklariert Chunk-Streaming, lieferte aber nur {} Teil(e)",
                got.len()
            );
        }
    }

    // Leerer Text: BadRequest, kein Chunk.
    let opts = expect.opts(Duration::from_secs(30));
    let (r, got) = tokio::time::timeout(
        QUICK,
        run_stream(p, "  ", &opts, &CancelToken::new("contract-stream"), 8),
    )
    .await
    .unwrap_or_else(|_| panic!("[{who}] Stream(leer) hängt"));
    assert_eq!(
        r.expect_err("leerer Text streamte").kind,
        TtsErrorKind::BadRequest
    );
    assert!(got.is_empty());

    // Empfänger weg: Fehler `Cancelled`, kein Hängen, keine Panik.
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    let cancel = CancelToken::new("contract-closed");
    let e = tokio::time::timeout(QUICK, p.stream_synthesize(&expect.text, &opts, &cancel, tx))
        .await
        .unwrap_or_else(|_| panic!("[{who}] Stream mit totem Empfänger hängt"))
        .expect_err("Stream ohne Empfänger meldete Erfolg");
    assert_eq!(e.kind, TtsErrorKind::Cancelled, "[{who}] {e:?}");
}

pub(crate) async fn check_cancel(p: &Arc<dyn TtsProvider>, expect: &ContractExpect) {
    let who = id_of(p);
    if expect.available {
        // Vorab abgebrochen: sofort `Cancelled`, kein Chunk.
        let opts = expect.opts(Duration::from_secs(30));
        let cancel = CancelToken::new("contract-cancel");
        cancel.cancel();
        let e = tokio::time::timeout(QUICK, p.synthesize(&expect.text, &opts, &cancel))
            .await
            .unwrap_or_else(|_| panic!("[{who}] synthesize hängt bei vorab gesetztem Abbruch"))
            .expect_err("abgebrochener Auftrag lieferte Audio");
        assert_eq!(e.kind, TtsErrorKind::Cancelled, "[{who}] {e:?}");
        assert_error_shape(&who, &e);
        let (r, got) = tokio::time::timeout(QUICK, run_stream(p, &expect.text, &opts, &cancel, 8))
            .await
            .unwrap_or_else(|_| panic!("[{who}] Stream hängt bei vorab gesetztem Abbruch"));
        assert_eq!(
            r.expect_err("abgebrochener Stream lief").kind,
            TtsErrorKind::Cancelled
        );
        assert!(
            got.is_empty(),
            "[{who}] Chunks trotz vorab gesetztem Abbruch"
        );
    }

    // Mitten im Lauf (nur mit langsamem Exemplar).
    let Some(slow) = &expect.slow else { return };
    let opts = expect.opts(Duration::from_secs(60));

    let cancel = CancelToken::new("contract-cancel-mid");
    let canceller = {
        let c = cancel.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            c.cancel();
        }
    };
    let started = std::time::Instant::now();
    let (r, ()) = tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::future::join(slow.synthesize(&expect.text, &opts, &cancel), canceller),
    )
    .await
    .unwrap_or_else(|_| panic!("[{who}] synthesize reagiert nicht auf Abbruch"));
    assert_eq!(
        r.expect_err("Abbruch ignoriert").kind,
        TtsErrorKind::Cancelled
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "[{who}] Abbruch zu spät"
    );

    let cancel = CancelToken::new("contract-cancel-mid-stream");
    let canceller = {
        let c = cancel.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            c.cancel();
        }
    };
    let ((r, got), ()) = tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::future::join(run_stream(slow, &expect.text, &opts, &cancel, 8), canceller),
    )
    .await
    .unwrap_or_else(|_| panic!("[{who}] Stream reagiert nicht auf Abbruch"));
    assert_eq!(
        r.expect_err("Abbruch ignoriert").kind,
        TtsErrorKind::Cancelled
    );
    assert!(
        got.iter().all(|c| !c.is_last),
        "[{who}] abgebrochener Stream lieferte ein Ende-Chunk"
    );
}

pub(crate) async fn check_timeout(expect: &ContractExpect) {
    let Some(slow) = &expect.slow else { return };
    let who = slow.descriptor().id;
    let opts = expect.opts(Duration::from_millis(60));
    let cancel = CancelToken::new("contract-timeout");

    let e = tokio::time::timeout(
        Duration::from_secs(10),
        slow.synthesize(&expect.text, &opts, &cancel),
    )
    .await
    .unwrap_or_else(|_| panic!("[{who}] synthesize beachtet die Frist nicht"))
    .expect_err("Frist ignoriert");
    assert_eq!(e.kind, TtsErrorKind::Timeout, "[{who}] {e:?}");
    assert!(!e.retryable, "[{who}] Timeout darf nicht wiederholbar sein");

    let cancel = CancelToken::new("contract-timeout-s");
    let (r, got) = tokio::time::timeout(
        Duration::from_secs(10),
        run_stream(slow, &expect.text, &opts, &cancel, 8),
    )
    .await
    .unwrap_or_else(|_| panic!("[{who}] Stream beachtet die Frist nicht"));
    assert_eq!(r.expect_err("Frist ignoriert").kind, TtsErrorKind::Timeout);
    assert!(
        got.iter().all(|c| !c.is_last),
        "[{who}] Ende-Chunk trotz Fristablauf"
    );
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use crate::managers::tts::providers::mock::{MockFailure, MockProvider, MOCK_SAMPLE_RATE};

    /// Alle Mock-Varianten als (Name, Exemplar, langsames Exemplar, Erwartung).
    fn variants() -> Vec<(&'static str, MockProvider, MockProvider)> {
        let slow = |m: MockProvider| m.latency(Duration::from_secs(20));
        vec![
            ("standard", MockProvider::new(), slow(MockProvider::new())),
            (
                "stille",
                MockProvider::new().silence(),
                slow(MockProvider::new().silence()),
            ),
            (
                "chunks",
                MockProvider::new().chunked(5, Duration::ZERO),
                slow(MockProvider::new().chunked(5, Duration::ZERO)),
            ),
            (
                "chunks-mit-pause",
                MockProvider::new().chunked(4, Duration::from_millis(5)),
                slow(MockProvider::new().chunked(4, Duration::from_millis(5))),
            ),
            (
                "latenz",
                MockProvider::new().latency(Duration::from_millis(20)),
                slow(MockProvider::new().latency(Duration::from_millis(20))),
            ),
        ]
    }

    #[tokio::test]
    async fn mock_besteht_den_vertrag_in_allen_varianten() {
        for (name, mock, slow) in variants() {
            eprintln!("Vertrag: Mock-Variante {name}");
            contract_suite(
                Arc::new(mock),
                ContractExpect::available()
                    .with_sample_rate(MOCK_SAMPLE_RATE)
                    .with_slow(Arc::new(slow)),
            )
            .await;
        }
    }

    #[tokio::test]
    async fn nicht_verfuegbarer_mock_besteht_den_vertrag_mit_grund() {
        contract_suite(
            Arc::new(MockProvider::new().unavailable("Programm fehlt")),
            ContractExpect::unavailable(),
        )
        .await;
    }

    #[tokio::test]
    async fn mock_ist_deterministisch_und_die_chunks_ergeben_den_clip() {
        let mock = Arc::new(MockProvider::new().chunked(7, Duration::ZERO));
        let p: Arc<dyn TtsProvider> = mock.clone();
        let text = "Ein Satz, der ein Signal ergibt.";
        let opts = SynthesisOptions::default();
        let a = p
            .synthesize(text, &opts, &CancelToken::detached())
            .await
            .unwrap();
        let b = p
            .synthesize(text, &opts, &CancelToken::detached())
            .await
            .unwrap();
        assert_eq!(a, b, "zwei Läufe, zwei Ergebnisse");

        let pcm = &a.data[44..];
        assert_eq!(pcm, &mock.expected_pcm(text)[..]);
        assert_eq!(a.format.sample_rate, 24_000);
        assert_eq!(a.format.channels, 1);

        let mut sopts = opts.clone();
        sopts.format = AudioEncoding::Pcm16Le;
        let (r, got) = run_stream(&p, text, &sopts, &CancelToken::detached(), 16).await;
        r.unwrap();
        assert_eq!(got.len(), 7);
        let joined: Vec<u8> = got.iter().flat_map(|c| c.data.clone()).collect();
        assert_eq!(joined, pcm, "Chunks zusammen == Clip");

        // Sinus: beginnt bei 0, erreicht ~25 % des Vollausschlags.
        let samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples[0], 0);
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap() as f64;
        assert!((peak / 32767.0 - 0.25).abs() < 0.01, "Spitze {peak}");

        // Stille ist wirklich still.
        let silent: Arc<dyn TtsProvider> = Arc::new(MockProvider::new().silence());
        let s = silent
            .synthesize(text, &opts, &CancelToken::detached())
            .await
            .unwrap();
        assert!(s.data[44..].iter().all(|b| *b == 0));
    }

    #[tokio::test]
    async fn mock_dauer_und_chunkzahl_sind_begrenzt() {
        let p: Arc<dyn TtsProvider> =
            Arc::new(MockProvider::new().chunked(100_000, Duration::ZERO));
        let opts = SynthesisOptions::default();
        // Winziger Text: Mindestdauer 100 ms.
        let clip = p
            .synthesize("a", &opts, &CancelToken::detached())
            .await
            .unwrap();
        assert_eq!(clip.duration_ms, 100);
        // Riesiger Text: Deckel 20 s, nicht proportional zum Text.
        let huge = "wort ".repeat(200_000);
        let clip = p
            .synthesize(&huge, &opts, &CancelToken::detached())
            .await
            .unwrap();
        assert_eq!(clip.duration_ms, 20_000);
        // Mehr Chunks gewünscht als Samples da sind: begrenzt, jeder Chunk nicht leer.
        let mut sopts = opts.clone();
        sopts.format = AudioEncoding::Pcm16Le;
        let (r, got) = run_stream(&p, "a", &sopts, &CancelToken::detached(), 4096).await;
        r.unwrap();
        assert_eq!(got.len(), 2400);
        assert!(got.iter().all(|c| !c.data.is_empty()));
    }

    #[tokio::test]
    async fn mock_bricht_zwischen_den_chunks_ab() {
        let mock = Arc::new(MockProvider::new().chunked(20, Duration::from_millis(100)));
        let p: Arc<dyn TtsProvider> = mock.clone();
        let opts = SynthesisOptions {
            format: AudioEncoding::Pcm16Le,
            ..Default::default()
        };
        let cancel = CancelToken::new("zwischen");
        let c2 = cancel.clone();
        let canceller = async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            c2.cancel();
        };
        let ((r, got), ()) = futures_util::future::join(
            run_stream(
                &p,
                "Ein ausreichend langer Text zum Streamen.",
                &opts,
                &cancel,
                32,
            ),
            canceller,
        )
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert!(!got.is_empty() && got.len() < 20, "{} Chunks", got.len());
        assert!(got.iter().all(|c| !c.is_last));
        assert_eq!(mock.in_flight(), 0, "Abbruch räumt auf");
    }

    #[tokio::test]
    async fn mock_frist_gilt_fuer_den_ganzen_stream_und_raeumt_auf() {
        let mock = Arc::new(MockProvider::new().chunked(10, Duration::from_millis(60)));
        let p: Arc<dyn TtsProvider> = mock.clone();
        let opts = SynthesisOptions {
            format: AudioEncoding::Pcm16Le,
            timeout: Duration::from_millis(150),
            ..Default::default()
        };
        let (r, got) = run_stream(&p, "Ein Text.", &opts, &CancelToken::detached(), 32).await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Timeout);
        assert!(got.len() < 10);
        assert_eq!(mock.in_flight(), 0);
    }

    #[tokio::test]
    async fn mock_haengt_bei_vollem_empfaenger_nicht_ueber_den_abbruch_hinaus() {
        // Kapazität 1, niemand liest: der Mock steht beim zweiten Chunk im
        // Rückstau. Abbruch muss ihn lösen (kein Verschlucken, kein Hängen).
        let mock = Arc::new(MockProvider::new().chunked(5, Duration::ZERO));
        let p: Arc<dyn TtsProvider> = mock.clone();
        let opts = SynthesisOptions {
            format: AudioEncoding::Pcm16Le,
            ..Default::default()
        };
        let cancel = CancelToken::new("rueckstau");
        let (tx, rx) = mpsc::channel(1);
        let c2 = cancel.clone();
        let canceller = async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            c2.cancel();
        };
        let stalls_before = crate::managers::tts::providers::contract::sink_stall_count();
        let (r, ()) = tokio::time::timeout(
            Duration::from_secs(5),
            futures_util::future::join(
                p.stream_synthesize("Ein Text zum Stauen.", &opts, &cancel, tx),
                canceller,
            ),
        )
        .await
        .expect("Rückstau löste sich nicht");
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert!(crate::managers::tts::providers::contract::sink_stall_count() > stalls_before);
        drop(rx);
        assert_eq!(mock.chunks_sent(), 1, "genau ein Chunk passte in den Kanal");
        assert_eq!(mock.in_flight(), 0);
    }

    #[tokio::test]
    async fn mock_fehler_vor_und_mitten_im_stream() {
        let failure = |at| MockFailure {
            error: TtsError::backend("Motor aus"),
            at_chunk: at,
            for_attempts: None,
        };
        let opts = SynthesisOptions {
            format: AudioEncoding::Pcm16Le,
            ..Default::default()
        };

        // Mitten im Stream: zwei Chunks kommen an, dann der Fehler.
        let p: Arc<dyn TtsProvider> = Arc::new(
            MockProvider::new()
                .chunked(5, Duration::ZERO)
                .failing(failure(2)),
        );
        let (r, got) = run_stream(
            &p,
            "Ein Text zum Scheitern.",
            &opts,
            &CancelToken::detached(),
            16,
        )
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Backend);
        assert_eq!(got.iter().map(|c| c.seq).collect::<Vec<_>>(), [0, 1]);

        // synthesize liefert bei demselben Fehler keinen halben Clip.
        let e = p
            .synthesize("Ein Text zum Scheitern.", &opts, &CancelToken::detached())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);

        // Vor dem ersten Chunk.
        let p: Arc<dyn TtsProvider> = Arc::new(
            MockProvider::new()
                .chunked(3, Duration::ZERO)
                .failing(failure(0)),
        );
        let (r, got) = run_stream(
            &p,
            "Ein Text zum Scheitern.",
            &opts,
            &CancelToken::detached(),
            16,
        )
        .await;
        assert!(r.is_err() && got.is_empty());
    }

    #[tokio::test]
    async fn mock_zaehlt_versuche_und_fehler_gelten_nur_fuer_die_ersten() {
        let mock = Arc::new(MockProvider::new().failing(MockFailure {
            error: TtsError::network("kurz weg"),
            at_chunk: 0,
            for_attempts: Some(2),
        }));
        let p: Arc<dyn TtsProvider> = mock.clone();
        let opts = SynthesisOptions::default();
        for expected_ok in [false, false, true, true] {
            let r = p
                .synthesize("Ein Text.", &opts, &CancelToken::detached())
                .await;
            assert_eq!(r.is_ok(), expected_ok);
        }
        assert_eq!(mock.attempts(), 4);
    }
}

/// Selbsttest des Satzes: absichtlich fehlerhafte Anbieter MÜSSEN durchfallen,
/// sonst prüft `contract_suite` nichts.
#[cfg(test)]
mod suite_selbsttest {
    use super::*;
    use crate::managers::tts::providers::contract::{
        send_chunk, AudioClip, ProviderHealth, VoiceDescriptor,
    };
    use crate::managers::tts::providers::mock::MockProvider;
    use futures_util::future::BoxFuture;

    #[derive(Clone, Copy)]
    enum Defect {
        /// "Nicht verfügbar" ohne Begründung.
        NoReason,
        /// Jeder Chunk trägt `is_last`.
        EveryChunkLast,
        /// `synthesize` ignoriert den Abbruch.
        IgnoresCancel,
        /// Rate im Clip passt nicht zum WAV-Header.
        WrongRate,
    }

    struct Broken {
        inner: MockProvider,
        defect: Defect,
    }

    impl TtsProvider for Broken {
        fn descriptor(&self) -> crate::managers::tts::providers::contract::ProviderDescriptor {
            self.inner.descriptor()
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            if matches!(self.defect, Defect::NoReason) {
                return Box::pin(async {
                    let mut h = ProviderHealth::ok();
                    h.available = false;
                    h
                });
            }
            self.inner.health_check()
        }
        fn list_voices<'a>(
            &'a self,
            l: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.inner.list_voices(l)
        }
        fn synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            match self.defect {
                Defect::IgnoresCancel => {
                    Box::pin(
                        async move { self.inner.synthesize(t, o, &CancelToken::detached()).await },
                    )
                }
                Defect::WrongRate => Box::pin(async move {
                    let mut clip = self.inner.synthesize(t, o, c).await?;
                    clip.format.sample_rate += 1;
                    Ok(clip)
                }),
                _ => self.inner.synthesize(t, o, c),
            }
        }
        fn stream_synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
            sink: mpsc::Sender<AudioChunk>,
        ) -> BoxFuture<'a, Result<(), TtsError>> {
            if !matches!(self.defect, Defect::EveryChunkLast) {
                return self.inner.stream_synthesize(t, o, c, sink);
            }
            Box::pin(async move {
                let (tx, mut rx) = mpsc::channel(16);
                let (r, chunks) =
                    futures_util::future::join(self.inner.stream_synthesize(t, o, c, tx), async {
                        let mut v = Vec::new();
                        while let Some(x) = rx.recv().await {
                            v.push(x);
                        }
                        v
                    })
                    .await;
                r?;
                for mut chunk in chunks {
                    chunk.is_last = true;
                    send_chunk(&sink, chunk, c).await?;
                }
                Ok(())
            })
        }
        fn cache_tag(&self, v: Option<&str>) -> String {
            self.inner.cache_tag(v)
        }
    }

    fn broken(defect: Defect) -> Arc<dyn TtsProvider> {
        Arc::new(Broken {
            inner: MockProvider::new().chunked(3, Duration::ZERO),
            defect,
        })
    }

    #[tokio::test]
    #[should_panic(expected = "nicht verfügbar ohne reason")]
    async fn suite_erkennt_fehlenden_grund_bei_nicht_verfuegbar() {
        contract_suite(broken(Defect::NoReason), ContractExpect::unavailable()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "is_last nicht genau am Ende")]
    async fn suite_erkennt_mehrfaches_is_last() {
        contract_suite(broken(Defect::EveryChunkLast), ContractExpect::available()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "abgebrochener Auftrag lieferte Audio")]
    async fn suite_erkennt_ignorierten_abbruch() {
        contract_suite(broken(Defect::IgnoresCancel), ContractExpect::available()).await;
    }

    #[tokio::test]
    #[should_panic(expected = "Rate passt nicht zum Header")]
    async fn suite_erkennt_falsche_metadaten() {
        contract_suite(broken(Defect::WrongRate), ContractExpect::available()).await;
    }
}
