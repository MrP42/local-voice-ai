//! Registry der Anbieter: Verzeichnis, Gesundheitsprüfung mit Zeitgrenze,
//! Auftragsverwaltung (`request_id` → [`CancelToken`]) und begrenzte
//! Wiederholung.
//!
//! Sperren (`Mutex`/`RwLock`) werden nie über ein `await` gehalten und
//! überstehen eine vergiftete Sperre (ein abgestürzter Auftrag darf die
//! Registry nicht lahmlegen).

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use tokio::sync::mpsc;

use super::contract::{
    guarded, is_valid_provider_id, send_chunk, AudioChunk, AudioClip, CancelToken,
    ProviderDescriptor, ProviderHealth, SynthesisOptions, TtsError, TtsProvider,
};

/// Härtester Deckel für Versuche, egal was die Richtlinie verlangt.
const MAX_ATTEMPTS_CAP: u32 = 5;
/// Sicherheitsnetz der Registry über der Auftragsfrist: ein Adapter, der
/// `opts.timeout` vergisst, hängt die Registry trotzdem nicht auf.
const SAFETY_GRACE: Duration = Duration::from_secs(2);
/// Kapazität des Zwischenkanals beim Weiterleiten eines Streams.
const FORWARD_CAPACITY: usize = 4;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Wie oft und mit welcher Pause ein Auftrag wiederholt wird.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Versuche insgesamt (1 = nie wiederholen); wird auf 1..=5 begrenzt.
    pub max_attempts: u32,
    /// Pause vor Versuch n+1 ist `backoff * n`.
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            backoff: Duration::from_millis(150),
        }
    }
}

/// Zählt gelieferte Chunks eines Auftrags. Wiederholung ist nur erlaubt,
/// solange der Zähler 0 ist — danach hätte der Hörer Teile schon gehört.
#[derive(Debug, Clone, Default)]
pub struct DeliveryTracker(Arc<AtomicU32>);

impl DeliveryTracker {
    pub fn mark(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
    pub fn delivered(&self) -> bool {
        self.0.load(Ordering::Acquire) > 0
    }
    pub fn count(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }
}

/// Führt `op` aus und wiederholt es bei einem `retryable` Fehler — aber nur,
/// solange `tracker` noch nichts als geliefert meldet, höchstens
/// `policy.max_attempts` Mal, und nie nach Abbruch. Die Pause dazwischen
/// ist abbrechbar. `op` bekommt die Versuchsnummer (ab 1).
pub async fn with_retry<T, F, Fut>(
    policy: &RetryPolicy,
    cancel: &CancelToken,
    tracker: &DeliveryTracker,
    mut op: F,
) -> Result<T, TtsError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, TtsError>>,
{
    let max = policy.max_attempts.clamp(1, MAX_ATTEMPTS_CAP);
    let mut attempt = 1u32;
    loop {
        let err = match op(attempt).await {
            Ok(value) => return Ok(value),
            Err(err) => err,
        };
        let may_retry =
            err.retryable && !tracker.delivered() && attempt < max && !cancel.is_cancelled();
        if !may_retry {
            return Err(err);
        }
        let pause = policy.backoff.saturating_mul(attempt);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return Err(TtsError::cancelled("Abgebrochen während der Wiederholungspause"));
            }
            _ = tokio::time::sleep(pause) => {}
        }
        attempt += 1;
    }
}

/// Stream mit Wiederholung: leitet die Chunks des Anbieters an `sink` weiter
/// und merkt sich, ob schon einer beim Hörer angekommen ist. Je Versuch ein
/// eigener, begrenzter Zwischenkanal; ein Fehler vor dem ersten weitergeleiteten
/// Chunk darf (bei `retryable`) einen neuen Versuch auslösen, danach nie.
pub async fn stream_with_retry(
    provider: &dyn TtsProvider,
    text: &str,
    opts: &SynthesisOptions,
    cancel: &CancelToken,
    sink: mpsc::Sender<AudioChunk>,
    policy: &RetryPolicy,
) -> Result<(), TtsError> {
    let tracker = DeliveryTracker::default();
    let safety_net = opts.timeout.saturating_add(SAFETY_GRACE);
    let what = provider.descriptor().id;
    with_retry(policy, cancel, &tracker, |_attempt| {
        let (tx, mut rx) = mpsc::channel::<AudioChunk>(FORWARD_CAPACITY);
        let (sink, tracker, what) = (sink.clone(), tracker.clone(), what.clone());
        async move {
            let produce = provider.stream_synthesize(text, opts, cancel, tx);
            let forward = async {
                while let Some(chunk) = rx.recv().await {
                    send_chunk(&sink, chunk, cancel).await?;
                    tracker.mark();
                }
                Ok::<(), TtsError>(())
            };
            guarded(cancel, safety_net, &what, async {
                let (produced, forwarded) = futures_util::future::join(produce, forward).await;
                // Ist der Hörer weg, scheitert zuerst die Weiterleitung; der
                // Fehler des Anbieters ist dann nur noch die Folge davon.
                forwarded?;
                produced
            })
            .await
        }
    })
    .await
}

type RequestTable = Arc<Mutex<HashMap<String, CancelToken>>>;

/// Hält einen Auftrag in der Registry; beim Drop (Ende, Fehler, abgebrochenes
/// Future, Panik) verschwindet der Eintrag wieder.
pub struct RequestGuard {
    table: RequestTable,
    token: CancelToken,
}

impl RequestGuard {
    pub fn token(&self) -> &CancelToken {
        &self.token
    }
    pub fn request_id(&self) -> &str {
        self.token.request_id()
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        let mut table = lock(&self.table);
        // Nur den eigenen Eintrag entfernen (Kennungen sind eindeutig, die
        // Prüfung schützt trotzdem vor einem späteren Auftrag gleicher Kennung).
        if table
            .get(self.token.request_id())
            .is_some_and(|t| t.ptr_eq(&self.token))
        {
            table.remove(self.token.request_id());
        }
    }
}

pub struct ProviderRegistry {
    providers: RwLock<Vec<(String, Arc<dyn TtsProvider>)>>,
    requests: RequestTable,
    retry: RetryPolicy,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Gesundheit mit Zeitgrenze; Hänger und Abstürze werden zu „nicht verfügbar“.
async fn probe(provider: Arc<dyn TtsProvider>, limit: Duration) -> ProviderHealth {
    let started = Instant::now();
    let checked =
        AssertUnwindSafe(async { tokio::time::timeout(limit, provider.health_check()).await })
            .catch_unwind()
            .await;
    let mut health = match checked {
        Ok(Ok(health)) => health,
        Ok(Err(_elapsed)) => ProviderHealth::unavailable(
            format!(
                "Zeitüberschreitung: keine Antwort nach {} ms",
                limit.as_millis()
            ),
            Some("Anbieter prüfen oder neu starten.".to_string()),
        ),
        Err(_panic) => ProviderHealth::unavailable(
            "Die Prüfung des Anbieters ist abgestürzt".to_string(),
            Some("Anbieter neu einrichten oder neu starten.".to_string()),
        ),
    };
    health.checked_ms = started.elapsed().as_millis() as u64;
    health
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(Vec::new()),
            requests: Arc::new(Mutex::new(HashMap::new())),
            retry: RetryPolicy::default(),
        }
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    fn snapshot(&self) -> Vec<(String, Arc<dyn TtsProvider>)> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Anbieter eintragen. Eine vorhandene Kennung wird ersetzt (Rückgabe
    /// `true`), die Reihenfolge bleibt. Ungültige Kennung: `InvalidConfig`.
    pub fn register(&self, provider: Arc<dyn TtsProvider>) -> Result<bool, TtsError> {
        let id = provider.descriptor().id;
        if !is_valid_provider_id(&id) {
            return Err(TtsError::invalid_config(format!(
                "Ungültige Anbieterkennung {id:?} (erlaubt: a-z, 0-9, _ und -)"
            )));
        }
        let mut list = self.providers.write().unwrap_or_else(|e| e.into_inner());
        match list.iter_mut().find(|(existing, _)| *existing == id) {
            Some(slot) => {
                slot.1 = provider;
                Ok(true)
            }
            None => {
                list.push((id, provider));
                Ok(false)
            }
        }
    }

    pub fn unregister(&self, id: &str) -> bool {
        let mut list = self.providers.write().unwrap_or_else(|e| e.into_inner());
        let before = list.len();
        list.retain(|(existing, _)| existing != id);
        list.len() != before
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn TtsProvider>> {
        self.snapshot()
            .into_iter()
            .find(|(existing, _)| existing == id)
            .map(|(_, p)| p)
    }

    /// Alle Anbieter in Registrierungsreihenfolge.
    pub fn list(&self) -> Vec<Arc<dyn TtsProvider>> {
        self.snapshot().into_iter().map(|(_, p)| p).collect()
    }

    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.list().iter().map(|p| p.descriptor()).collect()
    }

    fn unknown(id: &str) -> TtsError {
        TtsError::bad_request(format!("Unbekannter Anbieter '{id}'"))
    }

    /// Gesundheit EINES Anbieters mit Zeitgrenze.
    pub async fn health(&self, id: &str, limit: Duration) -> Result<ProviderHealth, TtsError> {
        let provider = self.get(id).ok_or_else(|| Self::unknown(id))?;
        Ok(probe(provider, limit).await)
    }

    /// Gesundheit aller Anbieter, parallel, mit Zeitgrenze JE Anbieter. Ein
    /// hängender oder abgestürzter Anbieter wird als „nicht verfügbar“ mit
    /// Grund gemeldet und hält die anderen nicht auf.
    pub async fn health_all(&self, limit: Duration) -> Vec<(String, ProviderHealth)> {
        let checks = self
            .snapshot()
            .into_iter()
            .map(|(id, provider)| async move {
                let health = probe(provider, limit).await;
                (id, health)
            });
        futures_util::future::join_all(checks).await
    }

    /// Auftrag anmelden. Doppelte oder leere Kennung: `BadRequest`.
    pub fn begin_request(&self, request_id: &str) -> Result<RequestGuard, TtsError> {
        if request_id.trim().is_empty() {
            return Err(TtsError::bad_request("Leere Auftragskennung"));
        }
        let token = CancelToken::new(request_id);
        let mut table = lock(&self.requests);
        if table.contains_key(request_id) {
            return Err(TtsError::bad_request(format!(
                "Auftrag '{request_id}' läuft bereits"
            )));
        }
        table.insert(request_id.to_string(), token.clone());
        drop(table);
        Ok(RequestGuard {
            table: self.requests.clone(),
            token,
        })
    }

    /// Auftrag abbrechen; `false`, wenn er nicht (mehr) läuft.
    pub fn cancel(&self, request_id: &str) -> bool {
        // Erst herausnehmen, dann auslösen: kein `cancel()` unter der Sperre.
        let token = lock(&self.requests).get(request_id).cloned();
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Alle laufenden Aufträge abbrechen (z. B. beim Beenden der App).
    pub fn cancel_all(&self) -> usize {
        let tokens: Vec<CancelToken> = lock(&self.requests).values().cloned().collect();
        tokens.iter().for_each(CancelToken::cancel);
        tokens.len()
    }

    pub fn active_requests(&self) -> usize {
        lock(&self.requests).len()
    }

    /// Synthese über die Registry: Auftrag anmelden, Anbieter suchen,
    /// mit Wiederholung ausführen, Auftrag abmelden.
    pub async fn synthesize(
        &self,
        provider_id: &str,
        request_id: &str,
        text: &str,
        opts: &SynthesisOptions,
    ) -> Result<AudioClip, TtsError> {
        let guard = self.begin_request(request_id)?;
        let provider = self
            .get(provider_id)
            .ok_or_else(|| Self::unknown(provider_id))?;
        let cancel = guard.token().clone();
        let tracker = DeliveryTracker::default();
        let safety_net = opts.timeout.saturating_add(SAFETY_GRACE);
        with_retry(&self.retry, &cancel, &tracker, |_attempt| {
            guarded(
                &cancel,
                safety_net,
                provider_id,
                provider.synthesize(text, opts, &cancel),
            )
        })
        .await
    }

    /// Wie [`Self::synthesize`] für Streams.
    pub async fn stream_synthesize(
        &self,
        provider_id: &str,
        request_id: &str,
        text: &str,
        opts: &SynthesisOptions,
        sink: mpsc::Sender<AudioChunk>,
    ) -> Result<(), TtsError> {
        let guard = self.begin_request(request_id)?;
        let provider = self
            .get(provider_id)
            .ok_or_else(|| Self::unknown(provider_id))?;
        stream_with_retry(&*provider, text, opts, guard.token(), sink, &self.retry).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::{TtsErrorKind, VoiceDescriptor};
    use crate::managers::tts::providers::mock::{MockFailure, MockProvider};
    use futures_util::future::BoxFuture;
    use std::time::Instant;

    fn opts() -> SynthesisOptions {
        SynthesisOptions {
            timeout: Duration::from_secs(10),
            ..Default::default()
        }
    }

    fn fast_retry(n: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts: n,
            backoff: Duration::from_millis(5),
        }
    }

    // ---- Verzeichnis ----------------------------------------------------

    #[test]
    fn register_list_get_und_ersetzen_halten_die_reihenfolge() {
        let reg = ProviderRegistry::new();
        assert!(!reg
            .register(Arc::new(MockProvider::new().with_id("a")))
            .unwrap());
        assert!(!reg
            .register(Arc::new(MockProvider::new().with_id("b")))
            .unwrap());
        assert!(!reg
            .register(Arc::new(MockProvider::new().with_id("c")))
            .unwrap());
        let ids: Vec<_> = reg.descriptors().into_iter().map(|d| d.id).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        // Ersetzen: gleiche Stelle, Rückgabe true.
        assert!(reg
            .register(Arc::new(MockProvider::new().with_id("b").silence()))
            .unwrap());
        let ids: Vec<_> = reg.list().iter().map(|p| p.descriptor().id).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert!(reg.get("b").is_some());
        assert!(reg.get("zzz").is_none());
        assert!(reg.unregister("a"));
        assert!(!reg.unregister("a"));
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn ungueltige_kennungen_werden_abgelehnt() {
        let reg = ProviderRegistry::new();
        for bad in ["", "Gross", "mit leerzeichen", "-start", "ä"] {
            let e = reg
                .register(Arc::new(MockProvider::new().with_id(bad)))
                .unwrap_err();
            assert_eq!(e.kind, TtsErrorKind::InvalidConfig, "{bad:?}");
        }
        assert!(reg.list().is_empty());
    }

    // ---- Gesundheit -----------------------------------------------------

    /// Anbieter, dessen Prüfung abstürzt: Test-Gegenstück zum Hänger des Mocks.
    struct PanicHealth(MockProvider);
    impl TtsProvider for PanicHealth {
        fn descriptor(&self) -> ProviderDescriptor {
            self.0.descriptor()
        }
        fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
            Box::pin(async { panic!("Prüfung explodiert") })
        }
        fn list_voices<'a>(
            &'a self,
            l: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
            self.0.list_voices(l)
        }
        fn synthesize<'a>(
            &'a self,
            t: &'a str,
            o: &'a SynthesisOptions,
            c: &'a CancelToken,
        ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
            self.0.synthesize(t, o, c)
        }
        fn cache_tag(&self, v: Option<&str>) -> String {
            self.0.cache_tag(v)
        }
    }

    #[tokio::test]
    async fn health_all_ist_parallel_zeitbegrenzt_und_ueberlebt_haenger_und_absturz() {
        let reg = ProviderRegistry::new();
        reg.register(Arc::new(MockProvider::new().with_id("gut")))
            .unwrap();
        reg.register(Arc::new(
            MockProvider::new().with_id("haengt").hang_health(),
        ))
        .unwrap();
        reg.register(Arc::new(PanicHealth(
            MockProvider::new().with_id("absturz"),
        )))
        .unwrap();
        reg.register(Arc::new(
            MockProvider::new()
                .with_id("fehlt")
                .unavailable("Programm fehlt"),
        ))
        .unwrap();
        let t0 = Instant::now();
        let all = reg.health_all(Duration::from_millis(150)).await;
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "health_all war nicht parallel/zeitbegrenzt: {:?}",
            t0.elapsed()
        );
        let names: Vec<_> = all.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, ["gut", "haengt", "absturz", "fehlt"]);
        assert!(all[0].1.available);
        assert!(!all[1].1.available);
        assert!(all[1].1.reason.as_deref().unwrap().contains("Zeit"));
        assert!(!all[2].1.available);
        assert!(all[2].1.reason.is_some());
        assert_eq!(all[3].1.reason.as_deref(), Some("Programm fehlt"));
        // Gemessene Dauer: der Hänger brauchte mindestens die Frist.
        assert!(all[1].1.checked_ms >= 140, "{}", all[1].1.checked_ms);
    }

    #[tokio::test]
    async fn health_eines_anbieters_und_unbekannte_kennung() {
        let reg = ProviderRegistry::new();
        reg.register(Arc::new(MockProvider::new())).unwrap();
        assert!(
            reg.health("mock", Duration::from_secs(1))
                .await
                .unwrap()
                .available
        );
        let e = reg
            .health("nope", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
    }

    // ---- Aufträge -------------------------------------------------------

    #[test]
    fn auftraege_sind_eindeutig_und_raeumen_beim_drop_auf() {
        let reg = ProviderRegistry::new();
        let g = reg.begin_request("r1").unwrap();
        assert_eq!(g.request_id(), "r1");
        assert_eq!(reg.active_requests(), 1);
        assert_eq!(
            reg.begin_request("r1").err().unwrap().kind,
            TtsErrorKind::BadRequest
        );
        assert_eq!(
            reg.begin_request("").err().unwrap().kind,
            TtsErrorKind::BadRequest
        );
        // cancel wirkt auf das Token des Guards.
        assert!(!g.token().is_cancelled());
        assert!(reg.cancel("r1"));
        assert!(g.token().is_cancelled());
        drop(g);
        assert_eq!(reg.active_requests(), 0);
        assert!(
            !reg.cancel("r1"),
            "beendeter Auftrag lässt sich nicht abbrechen"
        );
        // Kennung ist wieder frei und der alte Abbruch gilt nicht für den neuen Auftrag.
        let g2 = reg.begin_request("r1").unwrap();
        assert!(!g2.token().is_cancelled());
    }

    #[test]
    fn cancel_all_bricht_alle_laufenden_auftraege() {
        let reg = ProviderRegistry::new();
        let a = reg.begin_request("a").unwrap();
        let b = reg.begin_request("b").unwrap();
        assert_eq!(reg.cancel_all(), 2);
        assert!(a.token().is_cancelled() && b.token().is_cancelled());
    }

    #[tokio::test]
    async fn gleichzeitige_auftraege_bleiben_getrennt() {
        let reg = Arc::new(ProviderRegistry::new());
        reg.register(Arc::new(
            MockProvider::new().latency(Duration::from_millis(30)),
        ))
        .unwrap();
        let mut tasks = Vec::new();
        for i in 0..16 {
            let reg = reg.clone();
            tasks.push(tokio::spawn(async move {
                reg.synthesize("mock", &format!("r{i}"), "Hallo Welt", &opts())
                    .await
            }));
        }
        for t in tasks {
            assert!(t.await.unwrap().unwrap().duration_ms > 0);
        }
        assert_eq!(reg.active_requests(), 0);
    }

    #[tokio::test]
    async fn synthesize_ueber_die_registry_meldet_unbekannten_anbieter_und_raeumt_auf() {
        let reg = ProviderRegistry::new();
        let e = reg
            .synthesize("nope", "r1", "Hallo", &opts())
            .await
            .unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        assert_eq!(reg.active_requests(), 0);
    }

    #[tokio::test]
    async fn cancel_von_aussen_beendet_eine_laufende_synthese_ohne_reste() {
        let reg = Arc::new(ProviderRegistry::new());
        let mock = Arc::new(MockProvider::new().latency(Duration::from_secs(30)));
        reg.register(mock.clone()).unwrap();
        let reg2 = reg.clone();
        let task =
            tokio::spawn(
                async move { reg2.synthesize("mock", "lang", "Hallo Welt", &opts()).await },
            );
        // Warten, bis der Auftrag wirklich läuft.
        for _ in 0..100 {
            if mock.in_flight() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(mock.in_flight(), 1);
        assert!(reg.cancel("lang"));
        let r = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("Abbruch kam nicht an")
            .unwrap();
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert_eq!(reg.active_requests(), 0);
        assert_eq!(mock.in_flight(), 0, "die Arbeit lief weiter");
    }

    #[tokio::test]
    async fn ein_abgebrochenes_future_hinterlaesst_keinen_auftragseintrag() {
        let reg = Arc::new(ProviderRegistry::new());
        reg.register(Arc::new(
            MockProvider::new().latency(Duration::from_secs(30)),
        ))
        .unwrap();
        let reg2 = reg.clone();
        let task = tokio::spawn(async move {
            let _ = reg2.synthesize("mock", "weg", "Hallo", &opts()).await;
        });
        for _ in 0..100 {
            if reg.active_requests() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(reg.active_requests(), 1);
        task.abort();
        let _ = task.await;
        assert_eq!(reg.active_requests(), 0);
    }

    // ---- Wiederholung ---------------------------------------------------

    #[tokio::test]
    async fn retry_wiederholt_nur_wiederholbare_fehler_begrenzt() {
        let cancel = CancelToken::detached();
        let tracker = DeliveryTracker::default();

        // Erst Netzfehler, dann Erfolg.
        let n = AtomicU32::new(0);
        let r = with_retry(&fast_retry(3), &cancel, &tracker, |attempt| {
            n.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt < 2 {
                    Err(TtsError::network("aus"))
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;
        assert_eq!(r.unwrap(), 2);
        assert_eq!(n.load(Ordering::SeqCst), 2);

        // Nicht wiederholbar: genau ein Versuch.
        let n = AtomicU32::new(0);
        let r: Result<(), _> = with_retry(&fast_retry(3), &cancel, &tracker, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            async { Err(TtsError::backend("kaputt")) }
        })
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Backend);
        assert_eq!(n.load(Ordering::SeqCst), 1);

        // Immer wiederholbar: Deckel greift, letzter Fehler kommt zurück.
        let n = AtomicU32::new(0);
        let r: Result<(), _> = with_retry(&fast_retry(3), &cancel, &tracker, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            async { Err(TtsError::network("immer")) }
        })
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Network);
        assert_eq!(n.load(Ordering::SeqCst), 3);

        // Deckel ist hart: 0 wird zu 1, riesige Werte zu höchstens 5.
        let n = AtomicU32::new(0);
        let _: Result<(), _> = with_retry(&fast_retry(0), &cancel, &tracker, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            async { Err(TtsError::network("x")) }
        })
        .await;
        assert_eq!(n.load(Ordering::SeqCst), 1);
        let n = AtomicU32::new(0);
        let _: Result<(), _> = with_retry(&fast_retry(1000), &cancel, &tracker, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            async { Err(TtsError::network("x")) }
        })
        .await;
        assert_eq!(n.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn retry_stoppt_nach_dem_ersten_chunk_und_bei_abbruch() {
        let cancel = CancelToken::detached();
        // Schon etwas geliefert: kein zweiter Versuch, auch bei wiederholbarem Fehler.
        let tracker = DeliveryTracker::default();
        let n = AtomicU32::new(0);
        let r: Result<(), _> = with_retry(&fast_retry(3), &cancel, &tracker, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            tracker.mark();
            async { Err(TtsError::network("mittendrin")) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(n.load(Ordering::SeqCst), 1);

        // Abbruch während der Pause.
        let tracker = DeliveryTracker::default();
        let cancel = CancelToken::detached();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            c2.cancel();
        });
        let slow = RetryPolicy {
            max_attempts: 3,
            backoff: Duration::from_secs(30),
        };
        let t0 = Instant::now();
        let r: Result<(), _> = with_retry(&slow, &cancel, &tracker, |_| async {
            Err(TtsError::network("x"))
        })
        .await;
        assert_eq!(r.unwrap_err().kind, TtsErrorKind::Cancelled);
        assert!(t0.elapsed() < Duration::from_secs(5));
    }

    async fn collect_stream(
        provider: &MockProvider,
        policy: &RetryPolicy,
    ) -> (Vec<AudioChunk>, Result<(), TtsError>) {
        let (tx, mut rx) = mpsc::channel(8);
        let cancel = CancelToken::new("s1");
        let o = opts();
        let produce = stream_with_retry(
            provider,
            "Hallo Welt, das ist ein Test.",
            &o,
            &cancel,
            tx,
            policy,
        );
        let consume = async {
            let mut got = Vec::new();
            while let Some(c) = rx.recv().await {
                got.push(c);
            }
            got
        };
        let (res, got) = futures_util::future::join(produce, consume).await;
        (got, res)
    }

    #[tokio::test]
    async fn stream_wiederholt_vor_dem_ersten_chunk_ohne_doppelte_teile() {
        let mock = MockProvider::new()
            .chunked(4, Duration::ZERO)
            .failing(MockFailure {
                error: TtsError::network("Aussetzer"),
                at_chunk: 0,
                for_attempts: Some(1),
            });
        let (got, res) = collect_stream(&mock, &fast_retry(3)).await;
        res.unwrap();
        assert_eq!(mock.attempts(), 2);
        let seqs: Vec<_> = got.iter().map(|c| c.seq).collect();
        assert_eq!(seqs, [0, 1, 2, 3]);
        assert_eq!(got.iter().filter(|c| c.is_last).count(), 1);
        assert!(got.last().unwrap().is_last);
    }

    #[tokio::test]
    async fn stream_wiederholt_nicht_mehr_nach_gelieferten_chunks() {
        let mock = MockProvider::new()
            .chunked(5, Duration::ZERO)
            .failing(MockFailure {
                error: TtsError::network("Aussetzer mittendrin"),
                at_chunk: 2,
                for_attempts: Some(1),
            });
        let (got, res) = collect_stream(&mock, &fast_retry(3)).await;
        assert_eq!(res.unwrap_err().kind, TtsErrorKind::Network);
        assert_eq!(
            mock.attempts(),
            1,
            "nach Chunk 2 darf nicht neu begonnen werden"
        );
        let seqs: Vec<_> = got.iter().map(|c| c.seq).collect();
        assert_eq!(seqs, [0, 1], "keine doppelten Audioteile");
        assert!(got.iter().all(|c| !c.is_last));
    }

    #[tokio::test]
    async fn registry_stream_mit_wiederholung_und_aufraeumen() {
        let reg = ProviderRegistry::new().with_retry_policy(fast_retry(2));
        let mock = Arc::new(
            MockProvider::new()
                .chunked(3, Duration::ZERO)
                .failing(MockFailure {
                    error: TtsError::network("x"),
                    at_chunk: 0,
                    for_attempts: Some(1),
                }),
        );
        reg.register(mock.clone()).unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let o = opts();
        let produce = reg.stream_synthesize("mock", "st1", "Hallo Welt", &o, tx);
        let consume = async {
            let mut n = 0;
            while rx.recv().await.is_some() {
                n += 1;
            }
            n
        };
        let (res, n) = futures_util::future::join(produce, consume).await;
        res.unwrap();
        assert_eq!(n, 3);
        assert_eq!(mock.attempts(), 2);
        assert_eq!(reg.active_requests(), 0);
    }
}
