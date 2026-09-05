//! Vorlesen (TP1): Fish-Speech-TTS-Anbindung.
//!
//! `protocol` und `state` sind pure, I/O-freie Bausteine. `TtsCore` bündelt
//! die app-unabhängige Logik (HTTP, Phase, Abbruch, Besitz) und ist gegen
//! einen Mock-Server getestet; `TtsManager` ergänzt AppHandle-Belange:
//! Settings, Events, Prozess-Spawn, Idle-Watchdog und Exit-Teardown.

pub mod builder;
pub mod compile_cache;
pub mod dsp;
pub mod encode;
pub mod engine;
pub mod enhance;
pub mod loudness;
pub mod models;
pub mod piper;
pub mod player;
pub mod portable;
pub mod protocol;
pub mod registry;
pub mod state;
pub mod voices;

use engine::{EngineCaps, TtsEngine, TtsEngineKind};
use player::{PlaybackControls, Player};
use state::TtsPhase;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(2);
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(180);
pub const TTS_TIMEOUT: Duration = Duration::from_secs(300);

/// Ein zu sprechender Satz und die Stimme dafür. `None` heißt „die
/// eingestellte Stimme"; damit ist einstimmiges Vorlesen der Sonderfall
/// „überall None" und braucht keinen eigenen Pfad.
pub type Utterance = (String, Option<String>);

/// Sätze, die alle die eingestellte Stimme sprechen.
pub fn single_voice(sentences: Vec<String>) -> Vec<Utterance> {
    sentences.into_iter().map(|text| (text, None)).collect()
}
const IDLE_WATCH_INTERVAL: Duration = Duration::from_secs(30);

/// Wie weit ein einzelner Satz vom Pegel seiner Stimme abweichen darf.
/// Groß genug, damit jeder Satz den Zielpegel praktisch erreicht; klein
/// genug, dass die Betonung eines Satzes erhalten bleibt.
const SENTENCE_TRIM_DB: f32 = 3.0;

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct TtsStatus {
    pub phase: TtsPhase,
    pub owns_server: bool,
    pub message: Option<String>,
}

/// App-unabhängiger Kern: Port, Phase, HTTP, Besitz, Abbruch-Generation.
/// Der Tauri-Manager reicht Settings/Events/Prozess-Spawn von außen hinein.
pub struct TtsCore {
    port: Mutex<u16>,
    phase: Mutex<TtsPhase>,
    owns_server: AtomicBool,
    generation: AtomicU64,
    /// Abbruch-Flag des LAUFENDEN Auftrags; neue Aufträge tauschen es aus.
    cancelled: Mutex<Arc<AtomicBool>>,
    last_used: Mutex<Instant>,
    http: reqwest::Client,
    player: Arc<dyn Player>,
    seed: Mutex<i64>,
    max_chars: Mutex<u32>,
    /// Tempo und Lautstaerke, die die LAUFENDE Wiedergabe mitliest. Frueher
    /// zwei Mutex-Werte, die einmal je Satz gelesen wurden — eine Aenderung
    /// wirkte deshalb erst beim naechsten Satz.
    controls: Arc<PlaybackControls>,
    export_format: Mutex<String>,
    output_device: Mutex<Option<String>>,
    /// Aktive Referenzstimme (reference_id) oder None = Seed-Standardstimme.
    voice: Mutex<Option<String>>,
    /// Aktive Synthese-Engine (Dispatch siehe [`EngineImpl`]).
    engine: Mutex<EngineImpl>,
    /// Satz-Level-WAV-Cache: unveränderter Text (gleicher Satz, Seed und
    /// Stimme) wird beim erneuten Vorlesen nicht neu synthetisiert.
    wav_cache: Mutex<WavCache>,
    /// Persistenter Ableger des Caches auf Platte — bereits synthetisierte
    /// Bücher/Dokumente sind damit auch OHNE laufenden Fish-Server anhörbar.
    cache_dir: Mutex<Option<std::path::PathBuf>>,
    /// Genau EIN Startversuch zur Zeit. Bewusst atomar und nicht ueber die
    /// Phase geprueft: die Phasenpruefung lag VOR dem Spawn, das Setzen der
    /// Phase danach — dazwischen lagen eine Gesundheitsabfrage und ein
    /// Prozessstart. Zwei Ausloeser in diesem Fenster (etwa Vorlesen und ein
    /// Stimmwechsel) starteten beide einen Server; der zweite belegte weitere
    /// 17 GB VRAM und gehoerte niemandem. Beobachtet am 21.08.2026.
    start_claim: AtomicBool,
    /// Der Nutzer hat waehrend eines laufenden Starts abgebrochen. Dann darf
    /// kein Wiederholungsversuch anlaufen — sonst startet die App genau das
    /// wieder, was gerade beendet wurde.
    stop_requested: AtomicBool,
    /// Ob die Wiedergabe alle Stimmen auf denselben Pegel zieht.
    normalize: AtomicBool,
    /// Klangbearbeitung und ihre Stufe (None = aus).
    enhance: Mutex<Option<enhance::Strength>>,
    /// Korrekturfaktor je Stimme, einmal je Sitzung aus dem ersten
    /// synthetisierten Satz dieser Stimme gemessen. Schlüssel ist die
    /// reference_id, leer für die Seed-Standardstimme.
    ///
    /// Warum nicht Satz für Satz: die Lautheit schwankt zwischen Sätzen
    /// derselben Stimme absichtlich — ein Fragesatz ist anders betont als
    /// eine Aufzählung. Wer jeden Satz einzeln auf denselben Wert zöge,
    /// bügelte diese Betonung glatt und erzeugte hörbares Pumpen. Was
    /// wirklich stört, ist der Sprung ZWISCHEN Stimmen; genau den nimmt ein
    /// konstanter Faktor je Stimme heraus.
    voice_gains: Mutex<std::collections::HashMap<String, f32>>,
    /// Dauerhafte Klangregler je Stimme, aus `meta.json` gespiegelt
    /// (Schluessel wie bei `voice_gains`). Bewusst ein Spiegel und KEIN Cache
    /// mit eigener Invalidierung: `TtsManager::refresh_from_settings` fuellt
    /// ihn vor jedem Auftrag neu — ein geaenderter Regler wirkt damit ab dem
    /// naechsten Vorlesen, und im laufenden Auftrag liest niemand die Platte.
    voice_sounds: Mutex<std::collections::HashMap<String, registry::VoiceSound>>,
    on_phase_change: Mutex<Option<Box<dyn Fn(TtsStatus) + Send + Sync>>>,
}

/// Aktive Synthese-Engine des Kerns — Enum-Dispatch statt `dyn TtsEngine`.
///
/// Bewusst kein Trait-Objekt: die Trait-Methoden geben `impl Future` zurück
/// (async ohne neue Dependency, Begründung in engine.rs), und so ein Trait
/// ist nicht dyn-kompatibel. Die Fish-Synthese braucht ohnehin `&TtsCore`
/// (HTTP-Client, Caches) — ein besitzendes Trait-Objekt im Kern ergäbe eine
/// Besitz-Schleife.
///
/// `Clone`, damit `fetch_wav` je Auftrag EINEN konsistenten Schnappschuss
/// ziehen kann: Cache-Tag und Synthese desselben Satzes stammen dann
/// garantiert aus derselben Engine, auch wenn die Settings mittendrin
/// umgeschaltet werden (TOCTOU-Befund aus dem A3/E1-Review).
#[derive(Clone)]
enum EngineImpl {
    /// Fish Speech über den bestehenden HTTP-Pfad (`fish_synthesize`).
    Fish,
    /// Piper als CPU-Subprozess; trägt seine aufgelösten Pfade selbst.
    Piper(piper::PiperEngine),
    /// Austauschbare Engine für Tests: beweist, dass die Naht trägt.
    #[cfg(test)]
    Mock(Arc<tests::MockEngine>),
}

impl EngineImpl {
    fn kind(&self) -> TtsEngineKind {
        match self {
            Self::Fish => TtsEngineKind::Fish,
            Self::Piper(p) => p.kind(),
            #[cfg(test)]
            Self::Mock(mock) => mock.kind(),
        }
    }

    fn caps(&self) -> EngineCaps {
        match self {
            Self::Fish => engine::FISH_CAPS,
            Self::Piper(p) => p.caps(),
            #[cfg(test)]
            Self::Mock(mock) => mock.caps(),
        }
    }

    fn cache_tag(&self, voice: Option<&str>) -> String {
        match self {
            Self::Fish => engine::fish_cache_tag(voice),
            Self::Piper(p) => p.cache_tag(voice),
            #[cfg(test)]
            Self::Mock(mock) => mock.cache_tag(voice),
        }
    }

    /// Verfügbarkeit als vergleichbares Paar (Art, Unverfügbarkeits-Grund) —
    /// die Warnzeile in [`TtsCore::set_engine`] feuert nur, wenn sich genau
    /// das ändert, nicht bei jedem Settings-Refresh.
    fn availability(&self) -> (TtsEngineKind, Option<&'static str>) {
        match self {
            Self::Piper(p) => (TtsEngineKind::Piper, p.unavailable_reason()),
            other => (other.kind(), None),
        }
    }
}

/// Prozess-lebenszeitiger Audio-Cache mit Byte-Limit und FIFO-Verdrängung —
/// bewusst simpel: Wiederholungen (gleicher Text, Resume, Zurückspringen im
/// Hörbuch) treffen ihn, Speicher bleibt begrenzt.
struct WavCache {
    map: std::collections::HashMap<u64, Vec<u8>>,
    order: std::collections::VecDeque<u64>,
    bytes: usize,
}

const WAV_CACHE_LIMIT_BYTES: usize = 200 * 1024 * 1024;

impl WavCache {
    fn new() -> Self {
        Self {
            map: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
            bytes: 0,
        }
    }

    /// Streuwert eines Satzes — zugleich der Dateiname im Platten-Cache
    /// (`{key:016x}.wav`).
    ///
    /// `engine_tag` hält die Engines auseinander: eine Piper-Stimme darf nie
    /// einen Fish-Treffer liefern. KRITISCH ist die Legacy-Regel: der leere
    /// Tag (Fish-Standard) geht NICHT in den Hash ein — jeder vor der
    /// Engine-Abstraktion erzeugte Schlüssel bleibt damit byte-identisch
    /// gültig, im RAM wie auf der Platte.
    fn key(engine_tag: &str, text: &str, seed: i64, voice: Option<&str>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if !engine_tag.is_empty() {
            engine_tag.hash(&mut h);
        }
        text.hash(&mut h);
        seed.hash(&mut h);
        voice.hash(&mut h);
        h.finish()
    }

    fn get(&self, key: u64) -> Option<Vec<u8>> {
        self.map.get(&key).cloned()
    }

    fn insert(&mut self, key: u64, wav: Vec<u8>) {
        if wav.len() > WAV_CACHE_LIMIT_BYTES || self.map.contains_key(&key) {
            return;
        }
        while self.bytes + wav.len() > WAV_CACHE_LIMIT_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.map.remove(&oldest) {
                self.bytes -= evicted.len();
            }
        }
        self.bytes += wav.len();
        self.order.push_back(key);
        self.map.insert(key, wav);
    }
}

impl TtsCore {
    fn new(player: Arc<dyn Player>) -> Self {
        Self {
            port: Mutex::new(8080),
            phase: Mutex::new(TtsPhase::Stopped),
            owns_server: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            cancelled: Mutex::new(Arc::new(AtomicBool::new(false))),
            last_used: Mutex::new(Instant::now()),
            http: reqwest::Client::new(),
            player,
            seed: Mutex::new(42),
            max_chars: Mutex::new(5000),
            controls: Arc::new(PlaybackControls::default()),
            export_format: Mutex::new("wav".to_string()),
            output_device: Mutex::new(None),
            voice: Mutex::new(None),
            engine: Mutex::new(EngineImpl::Fish),
            wav_cache: Mutex::new(WavCache::new()),
            cache_dir: Mutex::new(None),
            start_claim: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            normalize: AtomicBool::new(true),
            enhance: Mutex::new(None),
            voice_gains: Mutex::new(std::collections::HashMap::new()),
            voice_sounds: Mutex::new(std::collections::HashMap::new()),
            on_phase_change: Mutex::new(None),
        }
    }

    fn disk_cache_path(&self, key: u64) -> Option<std::path::PathBuf> {
        self.cache_dir
            .lock()
            .unwrap()
            .as_ref()
            .map(|dir| dir.join(format!("{key:016x}.wav")))
    }

    /// Ist dieser Satz (mit aktueller Stimme/Seed/Engine) bereits
    /// synthetisiert — im RAM oder auf Platte?
    pub fn has_cached(&self, text: &str) -> bool {
        let seed = *self.seed.lock().unwrap();
        let voice = self.voice.lock().unwrap().clone();
        let engine_tag = self.engine_cache_tag(voice.as_deref());
        let key = WavCache::key(&engine_tag, text, seed, voice.as_deref());
        if self.wav_cache.lock().unwrap().get(key).is_some() {
            return true;
        }
        self.disk_cache_path(key).is_some_and(|p| p.exists())
    }

    /// Engine-Art für Dispatch-Entscheidungen des Managers.
    pub fn engine_kind(&self) -> TtsEngineKind {
        self.engine.lock().unwrap().kind()
    }

    /// Fähigkeiten der aktiven Engine (u. a. für das GPU-Flag der Übersetzung).
    pub fn engine_caps(&self) -> EngineCaps {
        self.engine.lock().unwrap().caps()
    }

    /// Engine wählen (aus den Settings gespiegelt).
    ///
    /// `Piper` verlangt eine aufgelöste [`piper::PiperEngine`] — und die
    /// Wahl BLEIBT Piper, auch wenn Binary oder Stimme (noch) fehlen:
    /// `ensure_ready`/`synthesize` liefern dann die konstante Fehler-ID,
    /// statt still auf den Fish-GPU-Server zurückzufallen. Ein
    /// unbrauchbarer Piper-Zustand wird EINMAL je Änderung gewarnt
    /// (Review-Befund: der frühere Fallback war still), nicht bei jedem
    /// Settings-Refresh.
    pub fn set_engine(&self, kind: TtsEngineKind, piper: Option<piper::PiperEngine>) {
        let chosen = match kind {
            TtsEngineKind::Fish => EngineImpl::Fish,
            TtsEngineKind::Piper => EngineImpl::Piper(
                // Ohne mitgelieferte Auflösung (kein Datenverzeichnis):
                // eine Engine, die ihren Fehlgrund kennt.
                piper.unwrap_or_else(|| piper::PiperEngine::resolve(None, None)),
            ),
        };
        let mut slot = self.engine.lock().unwrap();
        if slot.availability() != chosen.availability() {
            if let (TtsEngineKind::Piper, Some(reason)) = chosen.availability() {
                log::warn!("Piper-Engine gewählt, aber nicht einsatzbereit: {reason}");
            }
        }
        *slot = chosen;
    }

    /// Cache-Tag der aktiven Engine (siehe [`WavCache::key`]).
    fn engine_cache_tag(&self, voice: Option<&str>) -> String {
        self.engine.lock().unwrap().cache_tag(voice)
    }

    /// Konsistenter Schnappschuss der aktiven Engine — siehe [`EngineImpl`].
    fn engine_snapshot(&self) -> EngineImpl {
        self.engine.lock().unwrap().clone()
    }

    /// Dispatch-Punkt der Synthese: ein Cache-Miss in [`Self::fetch_wav`]
    /// geht durch die übergebene Engine (den Schnappschuss des Aufrufs).
    /// Für Fish ist das der unveränderte HTTP-Pfad
    /// ([`Self::fish_synthesize`]), für Piper der Subprozess.
    async fn engine_synthesize(
        &self,
        engine: &EngineImpl,
        port: u16,
        seed: i64,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let req = engine::SynthesisRequest {
            text,
            voice,
            seed,
            // Tempo bleibt Sache der Wiedergabe — für BEIDE Engines: in die
            // Synthese eingebacken läge es dauerhaft im Satz-Cache und gälte
            // doppelt, weil der Player bereits live skaliert.
            speed: None,
        };
        match engine {
            EngineImpl::Fish => self.fish_synthesize(port, req).await,
            EngineImpl::Piper(p) => p.synthesize(req).await,
            #[cfg(test)]
            EngineImpl::Mock(mock) => mock.synthesize(req).await,
        }
    }

    /// Der BESTEHENDE Fish-Pfad: HTTP-POST an den lokalen Server, Antwort
    /// als WAV validiert. Unverändert aus `fetch_wav` herausgelöst;
    /// [`engine::FishEngine`] delegiert hierher.
    async fn fish_synthesize(
        &self,
        port: u16,
        req: engine::SynthesisRequest<'_>,
    ) -> Result<Vec<u8>, String> {
        let url = format!("{}/v1/tts", protocol::base_url(port));
        let body = protocol::tts_request_body(req.text, req.seed, req.voice);
        let resp = self
            .http
            .post(url)
            .json(&body)
            .timeout(TTS_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("TTS request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("TTS server answered {}", resp.status()));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
        if !protocol::looks_like_wav(&bytes) {
            return Err("TTS response is not a WAV file".into());
        }
        Ok(bytes)
    }

    fn set_phase(&self, phase: TtsPhase, message: Option<String>) {
        {
            let mut slot = self.phase.lock().unwrap();
            if *slot != phase {
                log::info!("tts phase: {:?} -> {:?}", *slot, phase);
            }
            *slot = phase;
        }
        let status = TtsStatus {
            phase,
            owns_server: self.owns_server.load(Ordering::Acquire),
            message,
        };
        if let Some(cb) = self.on_phase_change.lock().unwrap().as_ref() {
            cb(status);
        }
    }

    pub fn phase(&self) -> TtsPhase {
        *self.phase.lock().unwrap()
    }

    pub fn owns_server(&self) -> bool {
        self.owns_server.load(Ordering::Acquire)
    }

    pub fn status(&self) -> TtsStatus {
        TtsStatus {
            phase: self.phase(),
            owns_server: self.owns_server(),
            message: None,
        }
    }

    fn idle_for_secs(&self) -> u64 {
        self.last_used.lock().unwrap().elapsed().as_secs()
    }

    async fn health_ok(&self, port: u16) -> bool {
        let url = format!("{}/v1/health", protocol::base_url(port));
        matches!(
            self.http.get(url).timeout(Duration::from_secs(4)).send().await,
            Ok(resp) if resp.status().is_success()
        )
    }

    /// Health-basierter Kernpfad: läuft schon ein Server → adoptieren
    /// (owns=false, wird nie gekillt). Spawnen kann nur der Manager, weil der
    /// Pfad aus den Settings kommt.
    pub async fn ensure_server_core(&self) -> Result<(), String> {
        let port = *self.port.lock().unwrap();
        if self.phase() == TtsPhase::Ready && self.health_ok(port).await {
            return Ok(());
        }
        if self.health_ok(port).await {
            // `owns_server` wird hier NICHT mehr auf false gesetzt. Der Wert ist
            // nur an zwei Stellen wahr gemeint: true beim eigenen Spawn, false
            // beim eigenen Stopp. Diese Zeile hat ihn bei JEDER Gesundheits-
            // pruefung auf false gezwungen — also auch fuer einen Server, den
            // die App selbst gestartet hatte. Danach hielt sie ihn fuer fremd,
            // "Server stoppen" war ausgegraut und der Prozess blieb mit seinem
            // VRAM stehen, bis jemand ihn im Taskmanager erschoss. Wer nichts
            // gespawnt hat, hat hier ohnehin schon false stehen.
            //
            // NICHT waehrend eines laufenden Auftrags auf `Ready` stellen:
            // die Phase ist zugleich die Anzeige "spricht gerade", und die
            // Oberflaeche haengt ihren Stopp-Knopf daran. Ein Serverbefund
            // mitten im Vorlesen hat die Anzeige auf "Bereit" zurueckgesetzt —
            // damit war der Knopf ausgegraut und das Vorlesen nicht mehr zu
            // beenden.
            if !matches!(self.phase(), TtsPhase::Speaking | TtsPhase::Starting) {
                self.set_phase(TtsPhase::Ready, None);
            }
            return Ok(());
        }
        Err("no server reachable".into())
    }

    /// Ein Sprechauftrag: alten abbrechen, Text prüfen, WAV holen, abspielen.
    /// Rückgabe: WAV-Bytezahl (für Tests/Telemetrie; Text wird nie geloggt).
    pub async fn speak_core(&self, raw: &str) -> Result<usize, String> {
        let max_chars = *self.max_chars.lock().unwrap();
        let prepared =
            protocol::prepare_text(raw, max_chars).ok_or_else(|| "empty text".to_string())?;
        if prepared.truncated {
            log::warn!("TTS text truncated to {max_chars} chars");
        }
        let sentences = single_voice(protocol::split_sentences(&prepared.text));
        self.speak_sentence_run(sentences, 0, None, None).await
    }

    /// Gemeinsamer Sprechpfad für Freitext und Hörbuch: Sätze pipelined
    /// sprechen, ab `start_index`, mit optionalem Callback nach jedem
    /// VOLLSTÄNDIG abgespielten Satz (absoluter Index) — die Basis für die
    /// persistente Fortschrittsanzeige.
    pub async fn speak_sentence_run(
        &self,
        sentences: Vec<Utterance>,
        start_index: usize,
        on_playing: Option<Arc<dyn Fn(usize) + Send + Sync>>,
        on_played: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    ) -> Result<usize, String> {
        if sentences.is_empty() {
            return Err("empty text".into());
        }
        // Letzter gewinnt: laufenden Auftrag stornieren, eigenes Flag setzen.
        let my_generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let my_cancel = Arc::new(AtomicBool::new(false));
        {
            let mut slot = self.cancelled.lock().unwrap();
            slot.store(true, Ordering::Release);
            *slot = my_cancel.clone();
        }
        *self.last_used.lock().unwrap() = Instant::now();

        let port = *self.port.lock().unwrap();
        let seed = *self.seed.lock().unwrap();
        self.set_phase(TtsPhase::Speaking, None);
        let result = self
            .fetch_and_play_pipelined(
                port,
                seed,
                &sentences,
                start_index,
                my_cancel.clone(),
                on_playing,
                on_played,
            )
            .await;
        // Nur der jüngste, NICHT stornierte Auftrag darf den Endzustand
        // setzen — nach einem Abbruch gehört die Phase dem Abbrecher
        // (cancel_core → Ready, stop_server → Stopped).
        if self.generation.load(Ordering::Acquire) == my_generation
            && !my_cancel.load(Ordering::Acquire)
        {
            match &result {
                Ok(_) => self.set_phase(TtsPhase::Ready, None),
                Err(e) => self.set_phase(TtsPhase::Error, Some(e.clone())),
            }
            *self.last_used.lock().unwrap() = Instant::now();
        }
        result
    }

    /// WAV vom Server holen, validieren; `play` ist optional, damit der
    /// Selbsttest (Task 8) denselben Pfad ohne Soundkarte messen kann.
    /// `voice = None` heisst "die eingestellte Stimme" — so bleibt der
    /// einstimmige Pfad exakt wie vorher. Ein `Some(..)` uebersteuert sie fuer
    /// genau diesen Satz; das ist die Grundlage des Dialog-Vorlesens.
    async fn fetch_wav(
        &self,
        port: u16,
        seed: i64,
        text: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let voice = match voice {
            Some(explicit) => Some(explicit.to_string()),
            None => self.voice.lock().unwrap().clone(),
        };
        // EIN Engine-Schnappschuss je Aufruf: Cache-Tag und Synthese dieses
        // Satzes stammen garantiert aus derselben Engine — ein
        // Settings-Wechsel währenddessen kann keine Bytes mehr unter
        // fremdem Tag ablegen (TOCTOU-Befund aus dem A3/E1-Review).
        let engine = self.engine_snapshot();
        // Unveränderter Satz + gleiche Stimme/Seed/Engine → aus dem Cache,
        // ohne Server. Der Cache-Lookup bleibt VOR der Engine: was schon
        // synthetisiert ist, braucht keine Engine — egal welche.
        let engine_tag = engine.cache_tag(voice.as_deref());
        let cache_key = WavCache::key(&engine_tag, text, seed, voice.as_deref());
        if let Some(cached) = self.wav_cache.lock().unwrap().get(cache_key) {
            return Ok(cached);
        }
        // Platten-Cache: macht bereits Vorgelesenes offline abspielbar.
        if let Some(path) = self.disk_cache_path(cache_key) {
            if let Ok(bytes) = std::fs::read(&path) {
                if protocol::looks_like_wav(&bytes) {
                    self.wav_cache
                        .lock()
                        .unwrap()
                        .insert(cache_key, bytes.clone());
                    return Ok(bytes);
                }
            }
        }
        // Cache-Miss: durch den Engine-Schnappschuss (Fish: der bisherige
        // HTTP-POST; Piper: ein Subprozess).
        let bytes = self
            .engine_synthesize(&engine, port, seed, text, voice.as_deref())
            .await?;
        self.wav_cache
            .lock()
            .unwrap()
            .insert(cache_key, bytes.clone());
        if let Some(path) = self.disk_cache_path(cache_key) {
            if let Err(e) = std::fs::write(&path, &bytes) {
                log::warn!("could not persist tts cache file: {e}");
            }
        }
        Ok(bytes)
    }

    /// Wiedergabefaktor für einen synthetisierten Satz.
    ///
    /// Zwei Stufen. Der Pegel der *Stimme* ist der gleitende Mittelwert aller
    /// bisher gehörten Sätze dieser Stimme — nicht die Messung des ersten:
    /// ein kurzer Einstiegssatz misst leicht daneben, und dieser Fehler
    /// bliebe sonst für die ganze Sitzung stehen. Darauf kommt die Korrektur
    /// des *Satzes*, gedämpft auf ±3 dB um den Stimmenpegel — so wird jeder
    /// Satz auf den Zielpegel gezogen, ohne dass Betonung glattgebügelt wird
    /// oder die Lautheit zwischen zwei Sätzen hörbar pumpt.
    fn playback_gain(&self, voice: Option<&str>, wav: &[u8]) -> f32 {
        let normalize = self.normalize.load(Ordering::Acquire);
        let key = self.voice_key(voice);
        // Dritte Stufe, unabhaengig von den beiden anderen: der dauerhafte
        // Regler dieser Stimme (`meta.json` → `sound.gain_db`). Er gilt auch
        // OHNE Normalisierung — er ist eine Eigenschaft der Stimme, keine
        // Messung.
        let voice_gain = self
            .voice_sounds
            .lock()
            .unwrap()
            .get(&key)
            .map(registry::VoiceSound::gain_factor)
            .unwrap_or(1.0);
        if !normalize && (voice_gain - 1.0).abs() <= f32::EPSILON {
            return 1.0;
        }
        let Some((mono, rate, peak)) = decode_wav(wav) else {
            return 1.0;
        };
        let normalized = if normalize {
            let sentence = loudness::gain_to_target(&mono, rate, peak);

            // Gemittelt wird in dB, nicht im Faktor: Lautheit ist logarithmisch,
            // der arithmetische Mittelwert zweier Faktoren träfe die Mitte nicht.
            let base = {
                let mut gains = self.voice_gains.lock().unwrap();
                let mixed = match gains.get(&key) {
                    Some(&previous) => {
                        let db = |g: f32| 20.0 * g.max(1e-6).log10();
                        10f32.powf((db(previous) * 0.75 + db(sentence) * 0.25) / 20.0)
                    }
                    None => sentence,
                };
                gains.insert(key, mixed);
                mixed
            };

            let limit = 10f32.powf(SENTENCE_TRIM_DB / 20.0);
            sentence.clamp(base / limit, base * limit)
        } else {
            1.0
        };
        let corrected = normalized * voice_gain;
        if peak <= f32::EPSILON {
            // Stille: die Messung sagt nichts, der Dauerregler schon.
            return voice_gain;
        }
        // Die Spitze hat immer das letzte Wort: die Dämpfung oben und der
        // Dauerregler duerfen den Faktor über die Aussteuerungsgrenze gehoben
        // haben.
        corrected.min(loudness::PEAK_CEILING / peak)
    }

    /// Schluessel einer Stimme fuer die pro-Stimme-Tabellen (`voice_gains`,
    /// `voice_sounds`). „Die eingestellte Stimme" muss denselben Schluessel
    /// ergeben wie ihr expliziter Name — sonst bekaeme dieselbe Stimme zwei
    /// Eintraege.
    fn voice_key(&self, voice: Option<&str>) -> String {
        match voice {
            Some(explicit) => explicit.to_string(),
            None => self.voice.lock().unwrap().clone().unwrap_or_default(),
        }
    }

    /// Dauerhafter Tempofaktor dieser Stimme (1,0 = unveraendert). Er tritt
    /// NEBEN den Nutzerregler, nicht an seine Stelle: der Nutzerregler bleibt
    /// im Player live wirksam (`PlaybackControls`), dieser Faktor steckt in
    /// der Abtastrate des Satzes (siehe [`scale_wav_rate`]) — beides
    /// zusammen ergibt das gehoerte Tempo, multiplikativ.
    fn voice_speed(&self, voice: Option<&str>) -> f32 {
        self.voice_sounds
            .lock()
            .unwrap()
            .get(&self.voice_key(voice))
            .map(registry::VoiceSound::speed_factor)
            .unwrap_or(1.0)
    }

    /// Satz-Pipeline: Satz N wird abgespielt, während Satz N+1 bereits beim
    /// Server liegt. Die gefühlte Latenz ist damit die Synthese des ersten
    /// Satzes; bei RTF < 1 (compile) bleibt die Wiedergabe lückenlos.
    /// `on_played` feuert nach jedem vollständig abgespielten Satz mit dessen
    /// absolutem Index — bei Abbruch mitten im Satz feuert es NICHT (der Satz
    /// wird beim Fortsetzen erneut gehört, wie bei einem Hörbuch üblich).
    async fn fetch_and_play_pipelined(
        &self,
        port: u16,
        seed: i64,
        sentences: &[Utterance],
        start_index: usize,
        cancelled: Arc<AtomicBool>,
        on_playing: Option<Arc<dyn Fn(usize) + Send + Sync>>,
        on_played: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    ) -> Result<usize, String> {
        let max_chars = *self.max_chars.lock().unwrap();
        let mut previous_playback: Option<(
            usize,
            tauri::async_runtime::JoinHandle<Result<(), String>>,
        )> = None;
        let mut total_bytes = 0usize;
        let mut failure: Option<String> = None;

        let notify = |idx: usize, was_cancelled: bool| {
            if was_cancelled {
                return;
            }
            if let Some(cb) = on_played.as_ref() {
                cb(idx);
            }
        };

        for (offset, (sentence, voice)) in sentences.iter().enumerate().skip(start_index) {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            // Einzelne Sätze absichern (leere überspringen, Monster kappen).
            let Some(prepared) = protocol::prepare_text(sentence, max_chars) else {
                notify(offset, cancelled.load(Ordering::Acquire));
                continue;
            };
            // Der naechste Satz wird geholt, WAEHREND der vorige noch spielt
            // (siehe `previous_playback` unten) — deshalb faellt ein
            // Stimmwechsel nicht als Pause auf, solange der Server die
            // Referenz im Speicher haelt (`use_memory_cache`).
            match self
                .fetch_wav(port, seed, &prepared.text, voice.as_deref())
                .await
            {
                Ok(bytes) => {
                    // Aufbereitung vor der Wiedergabe: Raender entschaerfen
                    // (immer) und Klangbearbeitung (wenn eingeschaltet). Sie
                    // laeuft, waehrend der vorige Satz noch spielt (siehe
                    // Pipeline unten), faellt also nicht als Wartezeit auf.
                    let strength = *self.enhance.lock().unwrap();
                    let bytes = prepare_sentence_audio(bytes, strength);
                    total_bytes += bytes.len();
                    // Vorherigen Satz zu Ende spielen lassen (Reihenfolge!).
                    if let Some((done_idx, handle)) = previous_playback.take() {
                        match handle.await {
                            Ok(Ok(())) => {
                                notify(done_idx, cancelled.load(Ordering::Acquire));
                            }
                            Ok(Err(e)) => {
                                failure = Some(e);
                                break;
                            }
                            Err(e) => {
                                failure = Some(e.to_string());
                                break;
                            }
                        }
                    }
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    // Live-Anzeige: dieser Satz beginnt jetzt zu spielen.
                    if let Some(cb) = on_playing.as_ref() {
                        cb(offset);
                    }
                    let player = self.player.clone();
                    let device = self.output_device.lock().unwrap().clone();
                    // Stimmen gleich laut: der Pegelausgleich dieses Satzes
                    // steht fest, der Nutzerregler skaliert ihn live.
                    let gain = self.playback_gain(voice.as_deref(), &bytes);
                    // Dauerhaftes Tempo dieser Stimme. NACH der Pegelmessung,
                    // damit die Lautheit bei der echten Abtastrate gemessen
                    // wird; der Nutzerregler multipliziert im Player darauf.
                    let bytes = scale_wav_rate(bytes, self.voice_speed(voice.as_deref()));
                    let controls = Arc::clone(&self.controls);
                    let cancel_flag = cancelled.clone();
                    previous_playback = Some((
                        offset,
                        tauri::async_runtime::spawn_blocking(move || {
                            player.play(bytes, device, gain, controls, cancel_flag)
                        }),
                    ));
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if let Some((done_idx, handle)) = previous_playback {
            match handle.await {
                Ok(Ok(())) => {
                    notify(done_idx, cancelled.load(Ordering::Acquire));
                }
                Ok(Err(e)) => {
                    failure.get_or_insert(e);
                }
                Err(e) => {
                    failure.get_or_insert(e.to_string());
                }
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(total_bytes),
        }
    }

    pub fn cancel_core(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.cancelled
            .lock()
            .unwrap()
            .store(true, Ordering::Release);
        // Der stornierte Auftrag darf die Phase nicht mehr anfassen (Guard) —
        // also stellt der Abbrecher selbst den Ruhezustand wieder her. Ohne
        // das bleibt die UI auf „Spricht" hängen und der Idle-Stopp greift nie.
        if self.phase() == TtsPhase::Speaking {
            self.set_phase(TtsPhase::Ready, None);
        }
        *self.last_used.lock().unwrap() = Instant::now();
    }

    #[cfg(test)]
    pub fn for_test(port: u16) -> Self {
        let core = Self::new(Arc::new(player::CountingPlayer(std::sync::Mutex::new(0))));
        *core.port.lock().unwrap() = port;
        core
    }
}

/// Tauri-seitiger Manager: besitzt ggf. den Serverprozess und verdrahtet
/// Settings, Events und den Idle-Watchdog.
pub struct TtsManager {
    core: Arc<TtsCore>,
    app: tauri::AppHandle,
    child: Mutex<Option<Child>>,
    /// Letzte Referenzaufnahme (16 kHz mono), wartet zwischen Stopp und
    /// Speichern auf Namen + bestätigtes Transkript.
    pending_reference: Mutex<Option<Vec<f32>>>,
    /// Geöffnetes Hörbuch/Dokument (Sätze + Identität); die Position lebt im
    /// persistenten Fortschritts-Store.
    reading: Mutex<Option<ReadingSession>>,
    /// Letzter Freitext-Sprechauftrag (Sätze + Position) — Basis für
    /// Pause/Weiter im Vorlesen-Feld, bewusst nicht persistiert.
    speak_session: Mutex<Option<SpeakSession>>,
    /// Abbruch-Flag des laufenden Datei-Exports. EIGENES Flag, nicht das der
    /// Wiedergabe: sonst würde ein Klick auf Stopp im Player den Export
    /// abwürgen (und umgekehrt) — zwei Vorgänge, zwei Schalter.
    export_cancel: Mutex<Arc<AtomicBool>>,
}

struct SpeakSession {
    /// Satz samt Stimme — sonst spraeche ein "Fortsetzen" den Rest eines
    /// Dialogs mit der falschen Stimme weiter.
    sentences: Vec<Utterance>,
    position: usize,
}

struct ReadingSession {
    key: String,
    title: String,
    sentences: Vec<String>,
}

/// Fortschritt eines Dokuments — Persistenz-Eintrag und Event-Payload.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct ReadingInfo {
    /// Absoluter Dateipfad = stabile Identität des Dokuments.
    pub key: String,
    pub title: String,
    /// Nächster zu spielender Satz (0-basiert) = Anzahl fertig gehörter Sätze.
    pub position: u32,
    pub total: u32,
    pub finished: bool,
    pub playing: bool,
}

const READING_STORE: &str = "reading_progress.json";

/// Obergrenze des Platten-Caches; darüber fliegen die ältesten Dateien.
const DISK_CACHE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// FIFO-Verdrängung nach Änderungszeit — läuft einmal beim App-Start.
/// Haelt die Startsperre, solange ein Startversuch laeuft, und gibt sie beim
/// Verlassen wieder frei — auch auf jedem Fehlerpfad. Genau deshalb ein
/// Drop-Typ und kein Flag von Hand: ein vergessener Ruecksetzer bedeutete,
/// dass der Server nie wieder startet.
struct StartClaim<'a>(&'a AtomicBool);

impl Drop for StartClaim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Die aussagekraeftigsten Zeilen aus dem Startprotokoll des Servers.
///
/// Der Kindprozess schrieb seine Ausgabe bisher nach `Stdio::null()`. Faellt
/// er beim Start um, sah der Nutzer nur "exit code: 3" — die Erklaerung stand
/// derweil in einem Traceback, den niemand je zu Gesicht bekam. Beobachtet am
/// 21.08.2026: hinter Code 3 steckte eine durch einen Bluescheck zerstoerte
/// Datei im Compile-Cache von PyTorch, sichtbar nur im Traceback.
///
/// Gesucht wird die letzte Zeile, die wie eine Fehlerursache aussieht
/// (Exception-Zeilen tragen sie in Python am Ende), sonst die letzte nicht
/// leere Zeile. Bewusst wenige Zeichen: das gehoert in eine Fehlermeldung,
/// nicht in ein Protokollfenster — der vollstaendige Text steht in der Datei.
pub fn startup_error_summary(log: &str) -> Option<String> {
    const MARKERS: [&str; 6] = [
        "Error",
        "error:",
        "Exception",
        "raised",
        "failed",
        "Traceback",
    ];
    let lines: Vec<&str> = log
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let picked = lines
        .iter()
        .rev()
        // Die Zeile mit dem Ursachenwort, aber nicht die Rahmenzeilen des
        // Tracebacks selbst ("Traceback (most recent call last):", "File ...").
        .find(|l| {
            MARKERS.iter().any(|m| l.contains(m))
                && !l.starts_with("File \"")
                && !l.starts_with("Traceback")
        })
        .or_else(|| lines.last())?;
    let mut summary: String = picked.chars().take(300).collect();
    if picked.chars().count() > 300 {
        summary.push('…');
    }
    Some(summary)
}

/// Ein `Write + Seek`-Ziel im Speicher, dessen Inhalt den WavWriter überlebt.
///
/// `hound::WavWriter::finalize()` verbraucht sich selbst und gibt seinen
/// Writer nicht zurück — für den MP3-Export brauchen wir die fertigen
/// WAV-Bytes aber danach noch. Daher die geteilte Hülle.
#[derive(Clone, Default)]
struct SharedWavBuffer(Arc<Mutex<std::io::Cursor<Vec<u8>>>>);

impl SharedWavBuffer {
    /// Die fertigen WAV-Bytes herausnehmen.
    fn take(&self) -> Vec<u8> {
        std::mem::take(self.0.lock().unwrap().get_mut())
    }
}

impl std::io::Write for SharedWavBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

impl std::io::Seek for SharedWavBuffer {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.lock().unwrap().seek(pos)
    }
}

/// Wohin der zusammengesetzte Ton läuft: direkt in die Datei (WAV) oder in
/// den Speicher, um am Ende einmal kodiert zu werden (MP3).
enum ExportSink {
    File(hound::WavWriter<std::io::BufWriter<std::fs::File>>),
    Memory(hound::WavWriter<SharedWavBuffer>),
}

impl ExportSink {
    fn write_sample(&mut self, sample: i16) -> Result<(), hound::Error> {
        match self {
            ExportSink::File(w) => w.write_sample(sample),
            ExportSink::Memory(w) => w.write_sample(sample),
        }
    }

    fn finalize(self) -> Result<(), hound::Error> {
        match self {
            ExportSink::File(w) => w.finalize(),
            ExportSink::Memory(w) => w.finalize(),
        }
    }
}

/// WAV-Blob zu Mono-Downmix, Abtastrate und Spitzenwert.
///
/// `None` bei allem, was `hound` nicht lesen kann — die Wiedergabe läuft dann
/// ungeregelt weiter, statt am Pegelmessen zu scheitern.
fn decode_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32, f32)> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes)).ok()?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>().ok()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .ok()?
        }
    };
    let peak = loudness::peak(&samples);
    let channels = spec.channels.max(1) as usize;
    let mono = if channels == 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    Some((mono, spec.sample_rate, peak))
}

/// Einen synthetisierten Satz für die Ausgabe aufbereiten.
///
/// Zwei Dinge, und nur das zweite ist optional:
///
/// 1. **Ränder entschärfen — immer.** Jeder Satz ist ein eigenes Tonstück.
///    Beginnt es bei einem Wert ungleich null, ist das für den Lautsprecher
///    ein Sprung, und ein Sprung knackt. Das gehört nicht hinter einen
///    Schalter: niemand schaltet eine Verbesserung ab, um ein Knacken zu
///    bekommen.
/// 2. **Klangbearbeitung — wenn eingeschaltet.**
///
/// Lässt sich der Blob nicht lesen, kommt er unverändert zurück. Eine
/// Aufbereitung ist es nicht wert, an ihr zu scheitern.
fn prepare_sentence_audio(bytes: Vec<u8>, strength: Option<enhance::Strength>) -> Vec<u8> {
    let Ok(reader) = hound::WavReader::new(std::io::Cursor::new(bytes.as_slice())) else {
        return bytes;
    };
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    // Mehrkanaliges bleibt der Klangbearbeitung fern — die Kette ist für eine
    // Sprachspur gebaut und auch nur dafür geprüft —, bekommt aber die
    // Randbehandlung, denn Knacken hat mit der Kanalzahl nichts zu tun.
    if channels > 1 {
        let Some(mut interleaved) = decode_interleaved(&bytes) else {
            return bytes;
        };
        enhance::soften_edges(&mut interleaved, channels, spec.sample_rate);
        return write_wav_interleaved(&interleaved, spec).unwrap_or(bytes);
    }
    let Some((mut samples, rate, _)) = decode_wav(&bytes) else {
        return bytes;
    };
    if let Some(strength) = strength {
        enhance::process(&mut samples, rate, strength);
    }
    enhance::soften_edges(&mut samples, 1, rate);
    write_wav_pcm16(&samples, spec).unwrap_or(bytes)
}

/// Tempo eines WAV-Blobs aendern, indem die ANGEGEBENE Abtastrate skaliert
/// wird — dieselbe Mechanik, die auch der Nutzerregler benutzt (rodios
/// `set_speed` resampelt ebenfalls und zieht die Tonhoehe mit).
///
/// Warum am Kopf und nicht ueber [`dsp::resample_stretch`]: hier geht kein
/// einziges Sample durch eine Interpolation, das Ergebnis ist verlustfrei und
/// exakt multiplikativ zum Nutzerregler. Beruehrt werden nur `sample_rate` und
/// `byte_rate` des `fmt `-Chunks; alles andere bleibt Byte fuer Byte stehen.
///
/// Faktor 1,0, ein nicht endlicher Faktor oder ein Blob, der kein RIFF/WAVE
/// mit lesbarem `fmt `-Chunk ist, geben den Blob unveraendert zurueck: eine
/// nicht anwendbare Klangeinstellung darf nie Tonausfall bedeuten.
fn scale_wav_rate(bytes: Vec<u8>, factor: f32) -> Vec<u8> {
    if !factor.is_finite() || (factor - 1.0).abs() <= f32::EPSILON {
        return bytes;
    }
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return bytes;
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let size = word(pos + 4) as usize;
        let data = pos + 8;
        if &bytes[pos..pos + 4] == b"fmt " && size >= 16 && data + 16 <= bytes.len() {
            let scaled = |value: u32| {
                let v = (value as f64 * factor as f64).round();
                (v >= 1.0 && v <= u32::MAX as f64).then_some(v as u32)
            };
            let (Some(rate), Some(byte_rate)) = (scaled(word(data + 4)), scaled(word(data + 8)))
            else {
                return bytes;
            };
            let mut out = bytes;
            out[data + 4..data + 8].copy_from_slice(&rate.to_le_bytes());
            out[data + 8..data + 12].copy_from_slice(&byte_rate.to_le_bytes());
            return out;
        }
        // Chunks sind auf gerade Laengen aufgefuellt (RIFF-Regel).
        pos = data + size + (size & 1);
    }
    bytes
}

/// WAV-Blob als Interleave lesen, ohne Downmix.
fn decode_interleaved(bytes: &[u8]) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes)).ok()?;
    let spec = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>().ok(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .ok()
        }
    }
}

/// Interleave als 16-bit-PCM-WAV schreiben, Kanalzahl bleibt erhalten.
fn write_wav_interleaved(samples: &[f32], spec: hound::WavSpec) -> Option<Vec<u8>> {
    let out_spec = hound::WavSpec {
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
        ..spec
    };
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut out, out_spec).ok()?;
        for &s in samples {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .ok()?;
        }
        writer.finalize().ok()?;
    }
    Some(out.into_inner())
}

/// WAV-Blob durch die Klangbearbeitung schicken und neu schreiben.
///
/// `None`, wenn der Blob nicht lesbar ist — der Aufrufer behaelt dann das
/// Original. Eine Klangverbesserung ist es nicht wert, an ihr zu scheitern.
fn enhance_wav_bytes(bytes: &[u8], strength: enhance::Strength) -> Option<Vec<u8>> {
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).ok()?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let (mut mono, rate, _) = decode_wav(bytes)?;
    // Bearbeitet wird der Downmix; bei Mono ist das die Spur selbst. Fuer
    // mehrkanalige Referenzen waere eine getrennte Bearbeitung je Kanal
    // richtiger, aber Sprache ist praktisch immer mono aufgenommen, und ein
    // Downmix ist ehrlicher als eine Kanalbehandlung, die niemand geprueft hat.
    if channels > 1 {
        return None;
    }
    enhance::process(&mut mono, rate, strength);
    write_wav_pcm16(&mono, spec)
}

/// f32-Mono als 16-bit-PCM-WAV in den Speicher schreiben.
fn write_wav_pcm16(samples: &[f32], spec: hound::WavSpec) -> Option<Vec<u8>> {
    let out_spec = hound::WavSpec {
        channels: 1,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
        ..spec
    };
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut out, out_spec).ok()?;
        for &s in samples {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .ok()?;
        }
        writer.finalize().ok()?;
    }
    Some(out.into_inner())
}

/// WAV-Blob auf `loudness::TARGET_LUFS` gezogen neu schreiben (16-bit PCM).
///
/// `None`, wenn der Blob nicht lesbar ist oder ohnehin schon passt — der
/// Aufrufer behält dann das Original, statt am Pegeln zu scheitern.
fn normalize_wav_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let (mono, rate, peak) = decode_wav(bytes)?;
    let gain = loudness::gain_to_target(&mono, rate, peak);
    if (gain - 1.0).abs() < 0.01 {
        return None;
    }
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).ok()?;
    let spec = reader.spec();
    let out_spec = hound::WavSpec {
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
        ..spec
    };
    // Kanäle bleiben, wie sie sind: gemessen wurde über den Downmix, der
    // Faktor gilt für alle Kanäle gleichermaßen.
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut out, out_spec).ok()?;
        let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes)).ok()?;
        let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
        match spec.sample_format {
            hound::SampleFormat::Float => {
                for sample in reader.samples::<f32>() {
                    let v = (sample.ok()? * gain).clamp(-1.0, 1.0) * i16::MAX as f32;
                    writer.write_sample(v as i16).ok()?;
                }
            }
            hound::SampleFormat::Int => {
                for sample in reader.samples::<i32>() {
                    let v = (sample.ok()? as f32 / scale * gain).clamp(-1.0, 1.0) * i16::MAX as f32;
                    writer.write_sample(v as i16).ok()?;
                }
            }
        }
        writer.finalize().ok()?;
    }
    Some(out.into_inner())
}

/// Mono-f32 zurueck in ein 16-Bit-PCM-WAV. Gegenstueck zu `decode_wav`.
fn encode_wav_mono(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Den Tiefe-Regler auf WAV-Bytes anwenden: dekodieren, strecken, wieder als
/// WAV kodieren. `factor <= 1.0` gibt die Bytes unveraendert zurueck, damit
/// der Normalfall keine Rechenzeit und keine Requantisierung kostet.
fn apply_depth(wav: &[u8], factor: f32) -> Option<Vec<u8>> {
    if !(factor > 1.0) {
        return Some(wav.to_vec());
    }
    let (mono, rate, _peak) = decode_wav(wav)?;
    let stretched = dsp::resample_stretch(&mono, factor);
    Some(encode_wav_mono(&stretched, rate))
}

/// Obergrenze fuer eine Referenzaufnahme in Sekunden.
///
/// Zero-Shot-Klonen zieht seine Stimmidentitaet aus wenigen Sekunden; 10 bis
/// 30 Sekunden sind der brauchbare Bereich. Laenger hilft nicht, sondern
/// schadet: mehr Material heisst mehr Raumhall, mehr Atmer, mehr
/// Pegelschwankung — und jede Sekunde davon geht in die Referenz ein.
const MAX_REFERENCE_SEC: f32 = 30.0;

/// Eine WAV auf `start_sec..end_sec` zuschneiden (Mono, 16 Bit).
///
/// `0.0/0.0` — allgemeiner: jedes Ende, das nicht hinter dem Start liegt —
/// bedeutet "die ganze Datei". Der Zuschnitt wird in JEDEM Fall auf
/// `MAX_REFERENCE_SEC` ab `start_sec` begrenzt, auch bei der ganzen Datei:
/// eine Fuenf-Minuten-Aufnahme als Referenz waere sonst der stille Weg zu
/// einer schlechteren Stimme.
///
/// `None`, wenn sich der Blob nicht lesen laesst.
fn trim_wav_bytes(bytes: &[u8], start_sec: f32, end_sec: f32) -> Option<Vec<u8>> {
    let (mono, rate, _peak) = decode_wav(bytes)?;
    let rate_f = rate.max(1) as f32;
    let start = ((start_sec.max(0.0) * rate_f) as usize).min(mono.len());
    let end = if end_sec > start_sec {
        ((end_sec * rate_f) as usize).clamp(start, mono.len())
    } else {
        mono.len()
    };
    let max_len = (MAX_REFERENCE_SEC * rate_f) as usize;
    let end = end.min(start.saturating_add(max_len));
    Some(encode_wav_mono(&mono[start..end], rate))
}

/// Marker, dass die Hörproben im Verzeichnis mit Pegelausgleich entstanden.
const DEMOS_NORMALIZED_MARKER: &str = ".loudness-v2";

/// Einmalig alle Hörproben löschen, die noch ohne Pegelausgleich entstanden
/// sind. Sie werden beim nächsten Anhören neu erzeugt — das kostet einmal
/// wenige Sekunden GPU-Zeit und ist der einzige Weg, sie loszuwerden, ohne
/// ihnen anzusehen, wie sie entstanden sind.
fn discard_stale_demos(dir: &std::path::Path) {
    if dir.join(DEMOS_NORMALIZED_MARKER).exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut removed = 0usize;
    for path in entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
    {
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    if let Err(e) = std::fs::write(dir.join(DEMOS_NORMALIZED_MARKER), b"") {
        log::warn!("could not mark demo dir: {e}");
        return;
    }
    if removed > 0 {
        log::info!("{removed} Hörprobe(n) ohne Pegelausgleich verworfen");
    }
}

fn prune_disk_cache(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::path::PathBuf, std::time::SystemTime, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((e.path(), meta.modified().ok()?, meta.len()))
        })
        .collect();
    let total: u64 = files.iter().map(|(_, _, len)| len).sum();
    if total <= DISK_CACHE_LIMIT_BYTES {
        return;
    }
    files.sort_by_key(|(_, modified, _)| *modified);
    let mut remaining = total;
    for (path, _, len) in files {
        if remaining <= DISK_CACHE_LIMIT_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            remaining -= len;
        }
    }
    log::info!("tts cache pruned to {} MB", remaining / (1024 * 1024));
}

/// Binding-Id des Referenzaufnahme-Flows im AudioRecordingManager.
const REFERENCE_BINDING: &str = "voice_reference";
/// Binding-Id des Übersetzungsaufnahme-Flows.
const TRANSLATE_BINDING: &str = "translate_input";
/// Binding-Id des Stimmwechsler-Flows.
const VOICECHANGE_BINDING: &str = "voicechange_input";

/// Binding-Id des Diktats fuer das Vorlesefeld.
const DICTATE_BINDING: &str = "dictate_input";

impl TtsManager {
    pub fn new(app: &tauri::AppHandle) -> Arc<Self> {
        use tauri::Emitter;

        let core = Arc::new(TtsCore::new(Arc::new(player::RodioPlayer)));
        let emitter = app.clone();
        *core.on_phase_change.lock().unwrap() = Some(Box::new(move |status: TtsStatus| {
            if let Err(e) = emitter.emit("tts-state-changed", status) {
                log::warn!("Could not emit tts-state-changed: {e}");
            }
        }));

        let manager = Arc::new(Self {
            core,
            app: app.clone(),
            child: Mutex::new(None),
            pending_reference: Mutex::new(None),
            reading: Mutex::new(None),
            speak_session: Mutex::new(None),
            export_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
        });
        manager.refresh_from_settings();

        // Persistenter Audio-Cache: macht bereits Vorgelesenes offline
        // (ohne Fish-Server) abspielbar. Begrenzung siehe prune_disk_cache.
        {
            use tauri::Manager;
            let base = crate::portable::data_dir()
                .cloned()
                .or_else(|| app.path().app_local_data_dir().ok());
            if let Some(dir) = base.map(|b| b.join("tts_cache")) {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    log::warn!("tts cache dir unavailable: {e}");
                } else {
                    *manager.core.cache_dir.lock().unwrap() = Some(dir.clone());
                    std::thread::spawn(move || prune_disk_cache(&dir));
                }
            }
        }

        // Compile-Cache aus %TEMP% holen, bevor der Server das erste Mal
        // startet — dort raeumt die Datentraegerbereinigung ihn irgendwann weg.
        manager.migrate_inductor_cache();

        // Bestandsstimmen einmalig auf das Lautheitsmaß nachziehen. Im
        // Hintergrund: der Lauf liest und schreibt Dateien und darf den
        // App-Start nicht aufhalten.
        {
            let fish_dir = manager.fish_dir();
            let demos = manager.demo_dir();
            std::thread::spawn(move || {
                let count = voices::renormalize_existing(&fish_dir);
                if count > 0 {
                    log::info!("{count} Referenzstimme(n) auf -20 LUFS nachgezogen");
                }
                // Hörproben aus der Zeit vor dem Ausgleich verwerfen. Sie
                // entstehen sonst nie neu: nachgezogen wird eine Hörprobe nur,
                // wenn die Referenz JÜNGER ist als sie — und das ist sie nach
                // diesem Lauf gerade nicht mehr.
                if let Some(dir) = demos {
                    discard_stale_demos(&dir);
                }
            });
        }

        // Vorwärmen: den Server im Hintergrund hochfahren, damit die gut
        // eine Minute Ladezeit ablaeuft, waehrend der Nutzer ohnehin einen
        // Text einfuegt oder eine Stimme waehlt. Standardmaessig aus — 17 GB
        // Grafikspeicher sind nichts, was man ungefragt belegt.
        if crate::settings::get_settings(app).tts_prewarm {
            let warming = Arc::clone(&manager);
            tauri::async_runtime::spawn(async move {
                log::info!("Vorwaermen: mache die TTS-Engine im Hintergrund bereit");
                // Durch die Engine-Naht, nicht Fish-hart: bei aktiver
                // Piper-Engine ist das nur ein Pfad-Check — der Fish-Server
                // (17 GB VRAM) darf dann nicht anlaufen.
                if let Err(e) = warming.ensure_engine_ready().await {
                    log::warn!("Vorwaermen fehlgeschlagen: {e}");
                }
            });
        }

        // Idle-Watchdog: beendet einen selbst gestarteten Server nach der
        // konfigurierten Leerlaufzeit, damit die 17 GB VRAM wieder frei werden.
        let watchdog = Arc::downgrade(&manager);
        std::thread::spawn(move || loop {
            std::thread::sleep(IDLE_WATCH_INTERVAL);
            let Some(manager) = watchdog.upgrade() else {
                break;
            };
            let idle_minutes = crate::settings::get_settings(&manager.app).tts_idle_minutes;
            if state::should_idle_stop(
                manager.core.idle_for_secs(),
                idle_minutes,
                manager.core.owns_server(),
                manager.core.phase(),
            ) {
                log::info!("TTS server idle for {idle_minutes} min — stopping to free VRAM");
                manager.stop_server();
            }
        });

        manager
    }

    /// Settings in den Kern spiegeln. Vor jedem Auftrag aufgerufen, damit
    /// Änderungen ohne App-Neustart wirken.
    pub fn refresh_from_settings(&self) {
        let settings = crate::settings::get_settings(&self.app);
        *self.core.port.lock().unwrap() = settings.tts_port;
        *self.core.seed.lock().unwrap() = settings.tts_seed;
        *self.core.max_chars.lock().unwrap() = settings.tts_max_chars;
        *self.core.enhance.lock().unwrap() = settings
            .tts_enhance
            .then_some(settings.tts_enhance_strength);
        self.core.controls.set_volume(settings.tts_volume);
        self.core.controls.set_speed(settings.tts_speed);
        *self.core.export_format.lock().unwrap() = settings.tts_export_format;
        *self.core.output_device.lock().unwrap() = settings.selected_output_device;
        *self.core.voice.lock().unwrap() = settings.tts_voice;
        // Dauerhafte Klangregler der Stimmen einmal je Auftrag von der Platte
        // holen — nicht je Satz. Stimmen ohne `sound` stehen gar nicht erst
        // in der Tabelle, der Normalfall kostet also nichts.
        {
            let fish_dir = std::path::PathBuf::from(&settings.tts_fish_dir);
            let sounds: std::collections::HashMap<String, registry::VoiceSound> =
                voices::list_voices(&fish_dir)
                    .into_iter()
                    .filter_map(|id| {
                        registry::read_meta(&fish_dir, &id)
                            .sound
                            .map(|sound| (id, sound))
                    })
                    .collect();
            *self.core.voice_sounds.lock().unwrap() = sounds;
        }
        // Engine-Wahl in den Kern spiegeln; unbekannte Werte fallen auf
        // Fish zurück (from_setting). Für Piper werden Binary und Stimme
        // hier — also vor jedem Auftrag — neu aufgelöst: eben erst geladene
        // Dateien (Paket E3) wirken damit ohne App-Neustart.
        let kind = TtsEngineKind::from_setting(&settings.tts_engine);
        let piper_engine = (kind == TtsEngineKind::Piper).then(|| {
            piper::PiperEngine::resolve(
                self.data_base_dir().as_deref(),
                settings.tts_piper_voice.as_deref(),
            )
        });
        self.core.set_engine(kind, piper_engine);
        // Beim Umschalten die gemessenen Faktoren verwerfen: sonst hinge der
        // Pegel an einer Messung aus der Zeit vor dem Umschalten.
        let previous = self
            .core
            .normalize
            .swap(settings.tts_normalize, Ordering::Release);
        if previous != settings.tts_normalize {
            self.core.voice_gains.lock().unwrap().clear();
        }
    }

    pub fn status(&self) -> TtsStatus {
        self.core.status()
    }

    pub fn cancel(&self) {
        self.core.cancel_core();
    }

    /// Freitext sprechen: legt eine Pause/Weiter-fähige Session an und meldet
    /// den Satzfortschritt als `tts-speak-progress`-Event.
    pub async fn speak_text(self: &Arc<Self>, raw: &str) -> Result<usize, String> {
        use tauri::Emitter;
        let max_chars = *self.core.max_chars.lock().unwrap();
        let prepared =
            protocol::prepare_text(raw, max_chars).ok_or_else(|| "empty text".to_string())?;
        // Eine Kuerzung muss man sehen. Bisher fiel sie nur einer Funktion
        // auf, die gar nicht mehr aufgerufen wird — der Nutzer merkte sie
        // daran, dass das Vorlesen mitten im Text aufhoerte, ohne dass
        // irgendwo stand, warum.
        if prepared.truncated {
            let total = raw.trim().chars().count() as u32;
            log::warn!("Vorlesetext auf {max_chars} von {total} Zeichen gekuerzt");
            let _ = self.app.emit(
                "tts-text-truncated",
                serde_json::json!({ "limit": max_chars, "total": total }),
            );
        }
        let sentences = self.utterances(&prepared.text);
        *self.speak_session.lock().unwrap() = Some(SpeakSession {
            sentences: sentences.clone(),
            position: 0,
        });
        self.run_speak_session(sentences, 0).await
    }

    /// Vorlesetext in Saetze samt Stimme zerlegen.
    ///
    /// Erkannt wird `<Name>` bzw. `<Name:Stil>` (inline oder am Zeilenanfang)
    /// und das alte Zeilenformat `Name:` — jeweils gegen die Sprecher-Registry
    /// (`known_speakers`), also gegen voice_id UND Anzeigename aus der
    /// `meta.json`. Genau das war vorher der Bruch: der Parser bekam nur die
    /// nackten Ids, weshalb `Anna:` nichts schaltete, wenn die Stimme intern
    /// anders hiess — der Marker wurde dann sogar mit vorgelesen.
    ///
    /// Ein Marker gilt bis zum naechsten; ohne jede Markierung ist das
    /// Ergebnis Satz fuer Satz dasselbe wie vorher, nur mit `None` als Stimme.
    ///
    /// Satztrennung passiert INNERHALB eines Sprecherabschnitts: ein Satz darf
    /// nie zwei Sprecher enthalten, und die Pipeline holt den naechsten Satz
    /// bereits waehrend der vorige spielt — deshalb klingt der Wechsel fluessig.
    ///
    /// Der Stil aus `<Name:Stil>` wird geparst (und damit aus dem gesprochenen
    /// Text entfernt), aber noch nicht aufgeloest — das ist Paket S5.
    fn utterances(&self, text: &str) -> Vec<Utterance> {
        let speakers = self.known_speakers();
        protocol::split_speaker_segments(text, &speakers)
            .into_iter()
            .flat_map(|segment| {
                protocol::split_sentences(&segment.text)
                    .into_iter()
                    .map(move |sentence| (sentence, segment.voice.clone()))
            })
            .collect()
    }

    /// Pausiertes Freitext-Vorlesen ab dem letzten vollständig gehörten Satz
    /// fortsetzen.
    pub async fn speak_resume(self: &Arc<Self>) -> Result<usize, String> {
        let (sentences, position) = {
            let guard = self.speak_session.lock().unwrap();
            let session = guard.as_ref().ok_or("nichts zum Fortsetzen")?;
            if session.position >= session.sentences.len() {
                return Err("bereits zu Ende gelesen".into());
            }
            (session.sentences.clone(), session.position)
        };
        self.run_speak_session(sentences, position).await
    }

    /// Alles im Cache → gar keinen Server anfassen (Offline-Wiedergabe);
    /// sonst die Engine bereitmachen. Vorher refresh, damit Stimme/Seed für
    /// die Cache-Schlüssel aktuell sind.
    pub async fn ensure_server_for(&self, sentences: &[String]) -> Result<(), String> {
        if !sentences.is_empty() && sentences.iter().all(|s| self.core.has_cached(s)) {
            log::info!("playback served entirely from cache — no server needed");
            return Ok(());
        }
        self.ensure_engine_ready().await
    }

    /// Die konfigurierte Engine bereitmachen — der zweite Dispatch-Punkt der
    /// Engine-Naht. Für Fish ist das die BESTEHENDE Startlogik
    /// ([`Self::ensure_server`]): Spawn, Health-Poll, Compile-Cache-Reparatur
    /// und Doppelstart-Schutz bleiben unangetastet. Für Piper nur ein
    /// Pfad-Check — es gibt keinen Server, und der Fish-GPU-Server darf bei
    /// aktiver Piper-Engine NIRGENDS anlaufen.
    pub async fn ensure_engine_ready(&self) -> Result<(), String> {
        match self.core.engine_snapshot() {
            EngineImpl::Fish => self.ensure_server().await,
            EngineImpl::Piper(p) => p.ensure_ready().await,
            #[cfg(test)]
            EngineImpl::Mock(mock) => mock.ensure_ready().await,
        }
    }

    async fn run_speak_session(
        self: &Arc<Self>,
        sentences: Vec<Utterance>,
        start: usize,
    ) -> Result<usize, String> {
        use tauri::Emitter;
        self.refresh_from_settings();
        let texts: Vec<String> = sentences[start.min(sentences.len())..]
            .iter()
            .map(|(text, _)| text.clone())
            .collect();
        self.ensure_server_for(&texts).await?;
        // Standardstimme an ihre Referenz binden, damit sie ueber Saetze
        // hinweg dieselbe bleibt (siehe ensure_seed_reference).
        self.bind_seed_voice().await;
        let total = sentences.len() as u32;
        let cb_manager = Arc::clone(self);
        let on_played: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |idx| {
            if let Some(session) = cb_manager.speak_session.lock().unwrap().as_mut() {
                session.position = idx + 1;
            }
            let _ = cb_manager.app.emit(
                "tts-speak-progress",
                serde_json::json!({ "position": idx as u32 + 1, "total": total }),
            );
        });
        // Live-Anzeige des Satzes, der gerade zu hören ist.
        let now_manager = Arc::clone(self);
        let now_sentences: Vec<String> = sentences.iter().map(|(text, _)| text.clone()).collect();
        let on_playing: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |idx| {
            let _ = now_manager.app.emit(
                "tts-current-sentence",
                serde_json::json!({
                    "context": "speak",
                    "index": idx as u32,
                    "text": now_sentences.get(idx).cloned().unwrap_or_default(),
                }),
            );
        });
        self.core
            .speak_sentence_run(sentences, start, Some(on_playing), Some(on_played))
            .await
    }

    /// Server sicherstellen — mit einem zweiten Anlauf, wenn der erste an
    /// einem zerstoerten Compile-Cache gescheitert ist.
    ///
    /// Ein Systemabsturz waehrend des Kompilierens hinterlaesst im Cache von
    /// TorchInductor Dateien, die nur noch aus Nullbytes bestehen. Der Server
    /// stirbt daran beim Aufwaermen, und zwar bei JEDEM weiteren Start —
    /// heilen tut sich das nie von selbst. Bis v0.8.8 half nur, die Dateien
    /// von Hand zu suchen und zu loeschen; das ist keine Zumutung, die man
    /// einem Nutzer stellen darf, und die Bedingung dafuer (Nullbytes bei
    /// korrekter Laenge) ist maschinell pruefbar.
    ///
    /// Deshalb: EIN Versuch, bei Verdacht Reparatur, dann EIN zweiter
    /// Versuch. Nicht mehr — schlaegt auch der fehl, liegt es an etwas
    /// anderem, und eine Schleife machte es nur langsamer, nicht besser.
    pub async fn ensure_server(&self) -> Result<(), String> {
        let first = self.try_start_server().await;
        let Err(error) = first else {
            return Ok(());
        };
        let log = self
            .startup_log_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        if self.core.stop_requested.load(Ordering::Acquire) {
            // Der Nutzer hat den Start abgebrochen. Kein zweiter Anlauf.
            return Err(error);
        }
        if !compile_cache::looks_like_broken_compile_cache(&log) {
            return Err(error);
        }
        let Some(dir) = self.inductor_cache_dir().or_else(compile_cache::cache_dir) else {
            return Err(error);
        };
        let removed = match compile_cache::repair(&dir) {
            Ok(removed) => removed,
            Err(e) => {
                log::warn!("compile cache repair refused: {e}");
                return Err(error);
            }
        };
        if removed.is_empty() {
            // Der Verdacht stimmte, aber es gibt nichts zu loeschen — ein
            // zweiter Anlauf brauchte nur Zeit und endete gleich.
            return Err(error);
        }
        log::warn!(
            "{} zerstoerte Datei(en) im Compile-Cache entfernt, zweiter Startversuch",
            removed.len()
        );
        self.core.set_phase(
            TtsPhase::Starting,
            Some(format!(
                "Beschaedigter Compile-Cache bereinigt ({} Datei(en)) — starte erneut",
                removed.len()
            )),
        );
        self.try_start_server().await
    }

    /// Ein Startversuch: adoptieren, sonst spawnen und Health pollen.
    async fn try_start_server(&self) -> Result<(), String> {
        // Die Sperre wird VOR allem anderen atomar beansprucht und beim
        // Verlassen der Funktion wieder freigegeben. Vorher wurde die Phase
        // geprueft und erst nach dem Spawn gesetzt — dazwischen lagen eine
        // Gesundheitsabfrage und ein Prozessstart. Zwei Ausloeser in diesem
        // Fenster starteten beide einen Server. Der zweite belegte weitere
        // 17 GB VRAM und gehoerte niemandem; die App konnte ihn nicht mehr
        // beenden, weil sie ihn nicht als ihren kannte.
        let _claim = match self.core.start_claim.compare_exchange(
            false,
            true,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => StartClaim(&self.core.start_claim),
            Err(_) => return Err("Der Server startet bereits — bitte warten.".to_string()),
        };
        self.core.stop_requested.store(false, Ordering::Release);

        if self.core.ensure_server_core().await.is_ok() {
            return Ok(());
        }

        let settings = crate::settings::get_settings(&self.app);
        let fish_dir = std::path::PathBuf::from(&settings.tts_fish_dir);
        let port = settings.tts_port;
        let python = fish_dir.join(r".venv\Scripts\python.exe");
        let api_script = fish_dir.join("tools").join("api_server.py");
        if !python.exists() || !api_script.exists() {
            let msg = format!(
                "Fish Speech nicht gefunden unter '{}'. Erwartet: .venv\\Scripts\\python.exe und tools\\api_server.py — siehe C:\\AI\\fish-speech\\INSTALL-REPORT.md",
                fish_dir.display()
            );
            self.core.set_phase(TtsPhase::Error, Some(msg.clone()));
            return Err(msg);
        }

        // Liegengebliebenen (abgestürzten) eigenen Prozess aufräumen.
        self.kill_owned_child();

        // Ausgabe des Kindprozesses in eine Datei statt ins Nichts. Ohne das
        // ist ein Startfehler nicht diagnostizierbar: der Nutzer sieht eine
        // Nummer, die Erklaerung steht in einem Traceback, den niemand liest.
        let startup_log = self.startup_log_path();
        let log_handles = startup_log.as_ref().and_then(|path| {
            let file = std::fs::File::create(path).ok()?;
            let clone = file.try_clone().ok()?;
            Some((file, clone))
        });

        let mut cmd = std::process::Command::new(&python);
        cmd.args([
            "tools/api_server.py",
            "--listen",
            &format!("127.0.0.1:{port}"),
        ])
        .current_dir(&fish_dir)
        .env("HF_HUB_DISABLE_TELEMETRY", "1");
        // Compile-Cache an einen Ort, den keine Datenträgerbereinigung leert.
        if let Some(cache) = self.inductor_cache_dir() {
            if std::fs::create_dir_all(&cache).is_ok() {
                cmd.env("TORCHINDUCTOR_CACHE_DIR", &cache);
            }
        }
        match log_handles {
            Some((out, err)) => {
                cmd.stdout(std::process::Stdio::from(out))
                    .stderr(std::process::Stdio::from(err));
            }
            None => {
                cmd.stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
            }
        }
        if settings.tts_compile {
            // 9x schnellere Synthese (RTF ~0,65 statt ~6), kostet ~60 s beim Start.
            cmd.arg("--compile");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("could not start fish-speech: {e}"))?;
        *self.child.lock().unwrap() = Some(child);
        self.core.owns_server.store(true, Ordering::Release);
        self.core.set_phase(TtsPhase::Starting, None);
        log::info!("Started fish-speech server on 127.0.0.1:{port}, waiting for health");

        let started = Instant::now();
        let mut hint_sent = false;
        loop {
            // Ein Abbruch waehrend des Startens muss sofort wirken: genau
            // dann will man ihn, weil der Start gerade den Speicher fuellt.
            if self.core.stop_requested.load(Ordering::Acquire) {
                self.kill_owned_child();
                self.core.owns_server.store(false, Ordering::Release);
                self.core.set_phase(TtsPhase::Stopped, None);
                return Err("Start abgebrochen".to_string());
            }
            if self.core.health_ok(port).await {
                self.core.set_phase(TtsPhase::Ready, None);
                *self.core.last_used.lock().unwrap() = Instant::now();
                log::info!("fish-speech ready after {} s", started.elapsed().as_secs());
                return Ok(());
            }
            // Früher Kindprozess-Tod (falscher Pfad, kaputtes venv) → klarer Fehler.
            if let Some(child) = self.child.lock().unwrap().as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    // Die Ursache steht im Protokoll des Kindprozesses; ohne
                    // sie ist "exit code 3" fuer niemanden verwertbar.
                    let detail = startup_log
                        .as_ref()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .as_deref()
                        .and_then(startup_error_summary)
                        .map(|line| format!(" — {line}"))
                        .unwrap_or_default();
                    let where_ = startup_log
                        .as_ref()
                        .map(|p| format!(" (Protokoll: {})", p.display()))
                        .unwrap_or_default();
                    let msg =
                        format!("fish-speech exited during startup ({status}){detail}{where_}");
                    self.core.owns_server.store(false, Ordering::Release);
                    self.core.set_phase(TtsPhase::Error, Some(msg.clone()));
                    return Err(msg);
                }
            }
            let elapsed = started.elapsed();
            if elapsed > HEALTH_TIMEOUT {
                self.kill_owned_child();
                let msg = format!(
                    "fish-speech not healthy after {} s — VRAM prüfen: andere GPU-Apps schließen",
                    elapsed.as_secs()
                );
                self.core.set_phase(TtsPhase::Error, Some(msg.clone()));
                return Err(msg);
            }
            if !hint_sent {
                if let Some(hint) = state::start_hint_after(elapsed.as_secs()) {
                    self.core
                        .set_phase(TtsPhase::Starting, Some(hint.to_string()));
                    hint_sent = true;
                }
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
    }

    fn fish_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(crate::settings::get_settings(&self.app).tts_fish_dir)
    }

    /// `fish_dir` fuer die Command-Schicht: die Commands brauchen den Pfad,
    /// der Manager haelt ihn aber bewusst privat, damit niemand am Manager
    /// vorbei am Stimmenordner arbeitet.
    pub fn fish_dir_public(&self) -> std::path::PathBuf {
        self.fish_dir()
    }

    pub fn list_voice_ids(&self) -> Vec<String> {
        voices::list_voices(&self.fish_dir())
    }

    /// Referenzaufnahme starten (VAD aus — auch leise Passagen gehören in die
    /// Referenz). Stößt parallel das STT-Modell-Laden an, damit das Transkript
    /// beim Stopp ohne Wartezeit entsteht.
    pub fn record_reference_start(&self) -> Result<(), String> {
        use tauri::Manager;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        tm.initiate_model_load();
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        rm.try_start_recording(REFERENCE_BINDING, crate::audio_toolkit::VadPolicy::Disabled)
    }

    /// Aufnahme beenden, Samples einbehalten, Transkript per STT liefern.
    /// Ein STT-Fehler verwirft die Aufnahme nicht — das Transkript kommt dann
    /// leer zurück und wird im UI von Hand ergänzt.
    pub fn record_reference_stop(&self) -> Result<String, String> {
        use tauri::Manager;
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        let generation = rm.cancel_generation();
        let samples = rm
            .stop_recording(REFERENCE_BINDING, generation)
            .ok_or_else(|| "no reference recording in progress".to_string())?;
        if !voices::reference_long_enough(samples.len()) {
            return Err(format!(
                "Aufnahme zu kurz ({:.1} s) — mindestens {} s einsprechen",
                samples.len() as f32 / 16_000.0,
                voices::MIN_REFERENCE_SECS
            ));
        }
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        let transcript = match tm.transcribe(samples.clone()) {
            Ok(text) => text,
            Err(e) => {
                log::warn!("reference transcription failed, keeping audio: {e}");
                String::new()
            }
        };
        *self.pending_reference.lock().unwrap() = Some(samples);
        Ok(transcript)
    }

    /// Einbehaltene Aufnahme unter einem Namen als Stimme speichern.
    /// Rückgabe: die sanierte Stimm-Id.
    pub fn save_pending_voice(&self, name: &str, transcript: &str) -> Result<String, String> {
        let id = voices::sanitize_voice_id(name)
            .ok_or_else(|| "Name ergibt keine gültige Stimm-Id".to_string())?;
        let samples = self
            .pending_reference
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| "keine Referenzaufnahme vorhanden".to_string())?;
        if let Err(e) = voices::save_voice(
            &self.fish_dir(),
            &id,
            &samples,
            transcript,
            *self.core.enhance.lock().unwrap(),
        ) {
            // Aufnahme zurücklegen, damit ein Tippfehler sie nicht kostet.
            *self.pending_reference.lock().unwrap() = Some(samples);
            return Err(e);
        }
        voices::update_registry(&self.fish_dir());
        Ok(id)
    }

    /// WAV-Datei als Stimme übernehmen. Ohne mitgeliefertes Transkript wird
    /// die Datei für die STT auf 16 kHz mono gewandelt und transkribiert; die
    /// Referenz selbst bleibt das unveränderte Original.
    pub fn import_voice_file(
        &self,
        name: &str,
        wav_path: &str,
        transcript: Option<String>,
    ) -> Result<(String, String), String> {
        use tauri::Manager;
        let id = voices::sanitize_voice_id(name)
            .ok_or_else(|| "Name ergibt keine gültige Stimm-Id".to_string())?;
        // Nicht-WAV-Quellen (mp3, m4a, mp4, …) über ffmpeg in hochwertiges
        // Mono-WAV wandeln; WAV geht unverändert durch.
        let (source, _tmp_guard) =
            crate::media::ensure_wav(std::path::Path::new(wav_path), 44_100)?;
        let transcript = match transcript.filter(|t| !t.trim().is_empty()) {
            Some(t) => t,
            None => {
                let samples = voices::load_wav_mono_16k(&source)?;
                let tm = self
                    .app
                    .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
                tm.initiate_model_load();
                tm.transcribe(samples).map_err(|e| {
                    format!("Transkription fehlgeschlagen ({e}) — Transkript bitte manuell angeben")
                })?
            }
        };
        voices::import_voice(
            &self.fish_dir(),
            &id,
            &source,
            &transcript,
            *self.core.enhance.lock().unwrap(),
        )?;
        voices::update_registry(&self.fish_dir());
        Ok((id, transcript))
    }

    /// Text übersetzen und die Übersetzung sprechen. Die Rückgabe (der
    /// übersetzte Text) kommt sofort; das Sprechen läuft im Hintergrund und
    /// meldet Fehler über die tts-state-changed-Events.
    pub async fn translate_and_speak(
        self: &Arc<Self>,
        text: &str,
        target_lang: &str,
    ) -> Result<String, String> {
        let settings = crate::settings::get_settings(&self.app);
        let translation = crate::translator::translate(&settings, text, target_lang).await?;
        self.speak_in_background(translation.clone());
        Ok(translation)
    }

    /// Text übersetzen — ohne ihn abzuspielen.
    ///
    /// Ob die Grafikkarte gerade dem TTS-Server gehört. Nicht "gehört der
    /// Server uns", sondern "läuft überhaupt einer": ein fremd gestarteter
    /// belegt dieselbe Grafikkarte. Und nur bei einer GPU-Engine (Fish:
    /// needs_gpu) — eine CPU-Engine wie Piper lässt die Grafikkarte frei,
    /// dann dürfen Übersetzung und Auto-Tagging sie nutzen.
    pub fn gpu_busy(&self) -> bool {
        self.core.engine_caps().needs_gpu && self.core.phase() != TtsPhase::Stopped
    }

    /// Bewusst getrennt vom Vorlesen: ein Knopf, der zwei Dinge tut, nimmt
    /// die Entscheidung ab, welches der beiden man wollte. Abspielen gibt es
    /// bereits; hier entsteht nur der Text.
    ///
    /// Zwischengespeichert je Originaltext UND Zielsprache, auf Platte.
    /// Zwischen zwei Sprachen hin und her zu wechseln kostet damit nach dem
    /// ersten Mal nichts mehr — und ein Neustart der App wirft es nicht weg.
    ///
    /// Läuft der Fish-Server, weicht die Übersetzung auf die CPU aus: er hält
    /// rund 17 GB Grafikspeicher, und ein zweites Modell daneben bringt beide
    /// zum Straucheln.
    pub async fn translate_text(&self, text: &str, target_lang: &str) -> Result<String, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err("kein Text zum Übersetzen".to_string());
        }
        if let Some(hit) = self.cached_translation(trimmed, target_lang) {
            return Ok(hit);
        }
        let settings = crate::settings::get_settings(&self.app);
        let gpu_busy = self.gpu_busy();
        // Die Sprachmodell-Anzeige lebt von diesen beiden Ereignissen: Gelb,
        // solange uebersetzt wird, danach zurueck (oder Orange bei Fehler).
        use tauri::Emitter;
        let _ = self
            .app
            .emit("llm-activity", serde_json::json!({ "busy": true }));
        let outcome =
            crate::translator::translate_on(&settings, trimmed, target_lang, gpu_busy).await;
        let _ = self.app.emit(
            "llm-activity",
            serde_json::json!({
                "busy": false,
                "error": outcome.as_ref().err().cloned(),
            }),
        );
        let translated = outcome?;
        self.store_translation(trimmed, target_lang, &translated);
        Ok(translated)
    }

    /// Liegt für diesen Text in dieser Sprache schon eine Übersetzung?
    pub fn cached_translation(&self, text: &str, target_lang: &str) -> Option<String> {
        let path = self.translation_path(text, target_lang)?;
        std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
    }

    fn store_translation(&self, text: &str, target_lang: &str, translation: &str) {
        let Some(path) = self.translation_path(text, target_lang) else {
            return;
        };
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("translation cache unavailable: {e}");
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, translation) {
            log::warn!("could not cache translation: {e}");
        }
    }

    /// Ablageort einer Übersetzung. Der Dateiname ist der Streuwert aus Text
    /// und Sprache — ändert sich das Original, zeigt er woandershin, und die
    /// alte Übersetzung wird nie fälschlich ausgeliefert.
    fn translation_path(&self, text: &str, target_lang: &str) -> Option<std::path::PathBuf> {
        use std::hash::{Hash, Hasher};
        use tauri::Manager;
        let base = crate::portable::data_dir()
            .cloned()
            .or_else(|| self.app.path().app_local_data_dir().ok())?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        target_lang.hash(&mut hasher);
        let key = hasher.finish();
        Some(base.join("translations").join(format!("{key:016x}.txt")))
    }

    /// Diktat für das Vorlesefeld: Aufnahme starten.
    ///
    /// Eigener Weg neben `record_translate_*` und `record_voicechange_*`,
    /// weil er als Einziger NUR aufnimmt und transkribiert — kein Übersetzen,
    /// kein Sprechen. Was mit dem Text geschieht, entscheidet danach der
    /// Nutzer.
    pub fn record_dictate_start(&self) -> Result<(), String> {
        use tauri::Manager;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        tm.initiate_model_load();
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        rm.try_start_recording(DICTATE_BINDING, crate::audio_toolkit::VadPolicy::Offline)
    }

    /// Diktat beenden: Aufnahme transkribieren und den Text zurückgeben.
    pub async fn record_dictate_stop(&self) -> Result<String, String> {
        use tauri::Manager;
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        let generation = rm.cancel_generation();
        let samples = rm
            .stop_recording(DICTATE_BINDING, generation)
            .ok_or("Keine Aufnahme erhalten")?;
        if samples.is_empty() {
            return Err("Aufnahme enthielt keine Sprache".into());
        }
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        let transcript = tm
            .transcribe(samples)
            .map_err(|e| format!("Transkription fehlgeschlagen: {e}"))?;
        if transcript.trim().is_empty() {
            return Err("Es wurde keine Sprache erkannt".into());
        }
        Ok(transcript)
    }

    /// Aufnahme für die Sprach-zu-Sprach-Übersetzung starten (VAD wie beim
    /// Diktat; STT-Modell wird parallel geladen).
    pub fn record_translate_start(&self) -> Result<(), String> {
        use tauri::Manager;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        tm.initiate_model_load();
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        rm.try_start_recording(TRANSLATE_BINDING, crate::audio_toolkit::VadPolicy::Offline)
    }

    /// Aufnahme beenden: transkribieren, übersetzen, Übersetzung sprechen.
    /// Rückgabe: (Transkript, Übersetzung) — das Sprechen läuft im Hintergrund.
    pub async fn record_translate_stop(
        self: &Arc<Self>,
        target_lang: &str,
    ) -> Result<(String, String), String> {
        use tauri::Manager;
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        let generation = rm.cancel_generation();
        let samples = rm
            .stop_recording(TRANSLATE_BINDING, generation)
            .ok_or_else(|| "no translate recording in progress".to_string())?;
        if samples.is_empty() {
            return Err("Aufnahme enthielt keine Sprache".into());
        }
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        let transcript = tm
            .transcribe(samples)
            .map_err(|e| format!("Transkription fehlgeschlagen: {e}"))?;
        if transcript.trim().is_empty() {
            return Err("Es wurde keine Sprache erkannt".into());
        }
        let translation = self.translate_and_speak(&transcript, target_lang).await?;
        Ok((transcript, translation))
    }

    /// Stimmwechsler: Aufnahme starten (Kaskade Aufnahme → STT → TTS in der
    /// aktiven Stimme; offline, kein Echtzeit-Effekt).
    pub fn record_voicechange_start(&self) -> Result<(), String> {
        use tauri::Manager;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        tm.initiate_model_load();
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        rm.try_start_recording(
            VOICECHANGE_BINDING,
            crate::audio_toolkit::VadPolicy::Offline,
        )
    }

    /// Stimmwechsler-Aufnahme beenden: transkribieren und in der aktiven
    /// Stimme nachsprechen. Rückgabe: das Transkript (Sprechen läuft im
    /// Hintergrund, Fehler kommen über tts-state-changed).
    pub async fn record_voicechange_stop(self: &Arc<Self>) -> Result<String, String> {
        use tauri::Manager;
        let rm = self
            .app
            .state::<Arc<crate::managers::audio::AudioRecordingManager>>();
        let generation = rm.cancel_generation();
        let samples = rm
            .stop_recording(VOICECHANGE_BINDING, generation)
            .ok_or_else(|| "no voice-change recording in progress".to_string())?;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        let transcript = tm
            .transcribe(samples)
            .map_err(|e| format!("Transkription fehlgeschlagen: {e}"))?;
        if transcript.trim().is_empty() {
            return Err("Es wurde keine Sprache erkannt".into());
        }
        self.speak_in_background(transcript.clone());
        Ok(transcript)
    }

    /// Stimmwechsler für eine Audio-/Videodatei (WAV direkt, alles andere
    /// über ffmpeg): transkribieren und in der aktiven Stimme nachsprechen.
    /// Rückgabe: das Transkript.
    pub async fn respeak_file(self: &Arc<Self>, wav_path: &str) -> Result<String, String> {
        use tauri::Manager;
        let (wav_source, _tmp_guard) =
            crate::media::ensure_wav(std::path::Path::new(wav_path), 16_000)?;
        let samples = voices::load_wav_mono_16k(&wav_source)?;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        tm.initiate_model_load();
        let transcript = tm
            .transcribe(samples)
            .map_err(|e| format!("Transkription fehlgeschlagen: {e}"))?;
        if transcript.trim().is_empty() {
            return Err("In der Datei wurde keine Sprache erkannt".into());
        }
        self.speak_in_background(transcript.clone());
        Ok(transcript)
    }

    fn speak_in_background(self: &Arc<Self>, text: String) {
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            // speak_text sichert Server (bzw. Cache-Offline-Pfad) selbst.
            if let Err(e) = manager.speak_text(&text).await {
                log::warn!("respeak: speaking failed: {e}");
            }
        });
    }

    /// Text in der aktiven Stimme als Audiodatei synthetisieren (ein Request,
    /// ohne Playback) — der Datei-Export. Format aus `tts_export_format`
    /// (wav/mp3/opus, der Fish-Server encodiert direkt). Bei aktiver
    /// Piper-Engine geht der Export durch die Naht ([`TtsCore::fetch_wav`])
    /// statt durch den Fish-HTTP-Pfad — dieser Weg lief bis Paket E2
    /// komplett an der Engine vorbei und hätte den GPU-Server gestartet.
    pub async fn synthesize_to_file(&self, text: &str, out_path: &str) -> Result<usize, String> {
        self.refresh_from_settings();
        if self.core.engine_kind() == TtsEngineKind::Piper {
            return self.piper_synthesize_to_file(text, out_path).await;
        }
        self.ensure_server().await?;
        let prepared = {
            let max_chars = *self.core.max_chars.lock().unwrap();
            protocol::prepare_text(text, max_chars).ok_or_else(|| "empty text".to_string())?
        };
        let port = *self.core.port.lock().unwrap();
        let seed = *self.core.seed.lock().unwrap();
        let voice = self.core.voice.lock().unwrap().clone();
        let format = self.core.export_format.lock().unwrap().clone();
        let url = format!("{}/v1/tts", protocol::base_url(port));
        let body =
            protocol::tts_request_body_in_format(&prepared.text, seed, voice.as_deref(), &format);
        let resp = self
            .core
            .http
            .post(url)
            .json(&body)
            .timeout(TTS_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("TTS request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("TTS server answered {}", resp.status()));
        }
        let audio = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
        if !protocol::looks_like_audio(&audio, &format) {
            return Err(format!("TTS response is not valid {format} audio"));
        }
        std::fs::write(out_path, &audio).map_err(|e| format!("could not write {out_path}: {e}"))?;
        *self.core.last_used.lock().unwrap() = Instant::now();
        Ok(audio.len())
    }

    /// Der Piper-Zweig des Datei-Exports: ein Lauf durch die Engine-Naht
    /// (samt Satz-Cache), das Ergebnis als WAV auf die Platte. Andere
    /// Formate encodiert der Fish-Server; Piper kann nur WAV
    /// (`PIPER_CAPS.export_formats`) — dann lieber eine klare Ansage als
    /// eine Datei mit falscher Endung.
    async fn piper_synthesize_to_file(&self, text: &str, out_path: &str) -> Result<usize, String> {
        self.ensure_engine_ready().await?;
        let format = self.core.export_format.lock().unwrap().clone();
        if format != "wav" {
            return Err(format!(
                "Die Piper-Engine exportiert nur WAV — eingestellt ist {format}"
            ));
        }
        let prepared = {
            let max_chars = *self.core.max_chars.lock().unwrap();
            protocol::prepare_text(text, max_chars).ok_or_else(|| "empty text".to_string())?
        };
        let port = *self.core.port.lock().unwrap();
        let seed = *self.core.seed.lock().unwrap();
        let wav = self
            .core
            .fetch_wav(port, seed, &prepared.text, None)
            .await?;
        std::fs::write(out_path, &wav).map_err(|e| format!("could not write {out_path}: {e}"))?;
        *self.core.last_used.lock().unwrap() = Instant::now();
        Ok(wav.len())
    }

    /// Derselbe Satz für jede Stimme — nur so vergleicht man Stimmen und nicht
    /// zwei verschiedene Aufnahmen. Bewusst kurz und vollständig: Klangfarbe,
    /// Tempo und Satzmelodie hört man an einem Satz, nicht an einem Wort.
    pub const DEMO_TEXT: &'static str = "Guten Tag. So klingt diese Stimme:         ein kurzer Satz, damit Sie Klangfarbe, Tempo und Betonung vergleichen können.";

    /// Wo der Compile-Cache von TorchInductor liegen soll.
    ///
    /// Nicht mehr in `%TEMP%`. Dort legt PyTorch ihn von sich aus ab — und
    /// genau dieses Verzeichnis leeren die Windows-Datenträgerbereinigung und
    /// die Speicheroptimierung. Ist der Cache weg, wird aus 25 Sekunden
    /// Aufwärmen ein Vielfaches, ohne dass irgendetwas kaputt wäre.
    ///
    /// Der Name behält das Präfix `torchinductor_`, weil die Reparatur in
    /// [`compile_cache`] nur in solchen Verzeichnissen etwas löscht.
    fn inductor_cache_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.data_base_dir()?.join("torchinductor_cache"))
    }

    /// Basis des App-Datenverzeichnisses (portable-bewusst) — darunter
    /// liegen tts_cache, torchinductor_cache und die Piper-Ablage
    /// (`tts/piper/…`, Vertrag siehe [`piper`]).
    fn data_base_dir(&self) -> Option<std::path::PathBuf> {
        use tauri::Manager;
        crate::portable::data_dir()
            .cloned()
            .or_else(|| self.app.path().app_local_data_dir().ok())
    }

    /// Einen vorhandenen Cache aus `%TEMP%` an den neuen Ort holen.
    ///
    /// Ein Umzug statt eines Neuanfangs: das dort liegende Kompilat ist
    /// gemessene 231 MB und mehrere Minuten Rechenzeit wert. Beide Orte
    /// liegen auf demselben Laufwerk, das Umbenennen kostet also nichts.
    ///
    /// Nur, wenn am Ziel noch nichts liegt — ein bestehender Cache wird
    /// niemals überschrieben.
    fn migrate_inductor_cache(&self) {
        let Some(target) = self.inductor_cache_dir() else {
            return;
        };
        if target.exists() {
            return;
        }
        let Some(old) = compile_cache::default_temp_cache_dir() else {
            return;
        };
        if !old.is_dir() {
            return;
        }
        if let Some(parent) = target.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        match std::fs::rename(&old, &target) {
            Ok(()) => log::info!(
                "Compile-Cache aus {} nach {} verschoben",
                old.display(),
                target.display()
            ),
            // Kein Fehler, der jemanden interessieren muss: dann wird eben
            // neu kompiliert, einmalig.
            Err(e) => log::warn!("Compile-Cache liess sich nicht verschieben: {e}"),
        }
    }

    /// Wohin der Serverprozess seine Ausgabe schreibt. Eine Datei, bei jedem
    /// Start ueberschrieben: interessant ist immer der letzte Versuch.
    fn startup_log_path(&self) -> Option<std::path::PathBuf> {
        use tauri::Manager;
        let base = crate::portable::data_dir()
            .cloned()
            .or_else(|| self.app.path().app_local_data_dir().ok())?;
        std::fs::create_dir_all(&base).ok()?;
        Some(base.join("fish-speech-start.log"))
    }

    /// Wo die Hörproben liegen. Eigenes Verzeichnis neben `tts_cache`, damit
    /// `prune_disk_cache` sie nicht wegräumt — der Cache ist nach Größe
    /// begrenzt, die Hörproben sind wenige Dateien, die bleiben sollen.
    fn demo_dir(&self) -> Option<std::path::PathBuf> {
        use tauri::Manager;
        let base = crate::portable::data_dir()
            .cloned()
            .or_else(|| self.app.path().app_local_data_dir().ok())?;
        let dir = base.join("voice_demos");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// Der Standardstimme eine echte Referenz verschaffen — einmal je Seed.
    ///
    /// Ohne Referenz wuerfelt Fish Speech die Sprecheridentitaet gemeinsam mit
    /// dem Inhalt aus. Der Seed geht zwar bei jeder Anfrage mit, aber jeder
    /// Satz ist eine eigene Anfrage mit eigenem Text — und damit klingt jede
    /// Zeile nach einer anderen Person. Fuer eine Vorlesestimme ist das
    /// unbrauchbar; genau das war die Beobachtung am 20.08.2026.
    ///
    /// Der Ausweg: den Demosatz EINMAL mit diesem Seed erzeugen und das
    /// Ergebnis als Referenz ablegen. Ab da spricht die Standardstimme so
    /// stabil wie eine geklonte — und weil Text und Seed feststehen, ergibt
    /// derselbe Seed auch nach einer Neuinstallation dieselbe Stimme.
    ///
    /// Die Referenz liegt unter `__seed_<seed>` und ist damit aus der
    /// Stimmenliste ausgenommen (siehe `voices::INTERNAL_PREFIX`).
    ///
    /// Rueckgabe: die Referenz-Kennung, oder `None`, wenn sie sich nicht
    /// anlegen liess — dann laeuft alles wie bisher weiter, nur eben
    /// wechselhaft. Ein Vorlesen daran scheitern zu lassen waere schlimmer.
    async fn ensure_seed_reference(&self, port: u16, seed: i64) -> Option<String> {
        let id = voices::seed_voice_id(seed);
        let fish_dir = self.fish_dir();
        if voices::voice_is_complete(&fish_dir, &id) {
            return Some(id);
        }
        let body = protocol::tts_request_body_in_format(Self::DEMO_TEXT, seed, None, "wav");
        let resp = self
            .core
            .http
            .post(format!("{}/v1/tts", protocol::base_url(port)))
            .json(&body)
            .timeout(TTS_TIMEOUT)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            log::warn!("seed reference: server answered {}", resp.status());
            return None;
        }
        let audio = resp.bytes().await.ok()?.to_vec();
        if !protocol::looks_like_wav(&audio) {
            log::warn!("seed reference: answer was not a WAV");
            return None;
        }
        // Auf denselben Pegel wie jede andere Referenz: die Standardstimme
        // soll sich in einen Dialog einreihen koennen, ohne herauszustechen.
        let audio = normalize_wav_bytes(&audio).unwrap_or(audio);
        let dir = voices::voice_dir(&fish_dir, &id);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("seed reference: could not create {}: {e}", dir.display());
            return None;
        }
        if let Err(e) = std::fs::write(dir.join("sample.wav"), &audio) {
            log::warn!("seed reference: could not write sample: {e}");
            return None;
        }
        if let Err(e) = std::fs::write(dir.join("sample.lab"), Self::DEMO_TEXT.as_bytes()) {
            log::warn!("seed reference: could not write transcript: {e}");
            return None;
        }
        log::info!("Standardstimme fuer Seed {seed} als Referenz {id} festgehalten");
        Some(id)
    }

    /// Die aktuelle Seed-Stimme unter einem Namen festhalten.
    ///
    /// Ein Seed ist fluechtig: wer weiterwuerfelt, verliert die Stimme, die
    /// ihm eben gefiel — und denselben Zahlenwert wiederzufinden ist
    /// aussichtslos. Speichern macht aus dem Wurf eine benannte Stimme, die
    /// in der Auswahl steht wie jede geklonte.
    ///
    /// Umgesetzt als Kopie der Seed-Referenz: `ensure_seed_reference` erzeugt
    /// (oder findet) die interne Referenz dieses Seeds, und die wird unter
    /// dem Namen abgelegt. Kein zweiter Synthese-Weg, der eigene Fehler
    /// haben koennte. Dazu der Seed als Herkunftsvermerk und ein frisches
    /// Stimmen-Register.
    pub async fn save_seed_voice(&self, name: &str) -> Result<String, String> {
        let id = voices::sanitize_voice_id(name)
            .ok_or_else(|| "Der Name ergibt keinen brauchbaren Stimmennamen".to_string())?;
        let fish_dir = self.fish_dir();
        if voices::voice_is_complete(&fish_dir, &id) {
            return Err(format!("Die Stimme '{id}' existiert bereits"));
        }
        self.refresh_from_settings();
        self.ensure_server().await?;
        let port = *self.core.port.lock().unwrap();
        let seed = *self.core.seed.lock().unwrap();
        let source_id = self
            .ensure_seed_reference(port, seed)
            .await
            .ok_or_else(|| "Die Seed-Referenz liess sich nicht erzeugen".to_string())?;
        let source = voices::voice_dir(&fish_dir, &source_id);
        let target = voices::voice_dir(&fish_dir, &id);
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("could not create {}: {e}", target.display()))?;
        for file in ["sample.wav", "sample.lab"] {
            std::fs::copy(source.join(file), target.join(file))
                .map_err(|e| format!("could not copy {file}: {e}"))?;
        }
        voices::write_seed_marker(&fish_dir, &id, seed);
        voices::update_registry(&fish_dir);
        log::info!("Seed {seed} als Stimme '{id}' gespeichert");
        Ok(id)
    }

    /// `core.voice` auf die Seed-Referenz setzen, wenn keine Stimme gewaehlt
    /// ist. Vor jedem Sprechlauf aufgerufen, nachdem der Server steht.
    async fn bind_seed_voice(&self) {
        // Seed-Referenzen sind eine Fish-Faehigkeit (Cloning). Bei anderen
        // Engines laeuft kein Server — der HTTP-Versuch dahinter waere ein
        // sinnloser Fehlschlag je Sprechlauf.
        if self.core.engine_kind() != TtsEngineKind::Fish {
            return;
        }
        if self.core.voice.lock().unwrap().is_some() {
            return;
        }
        let port = *self.core.port.lock().unwrap();
        let seed = *self.core.seed.lock().unwrap();
        if let Some(id) = self.ensure_seed_reference(port, seed).await {
            *self.core.voice.lock().unwrap() = Some(id);
        }
    }

    /// Hörprobe einer Stimme: `DEMO_TEXT`, mit genau dieser Stimme erzeugt und
    /// als WAV zwischengespeichert.
    ///
    /// Erzeugt wird nur beim ersten Mal — und erneut, wenn die Referenzaufnahme
    /// der Stimme jünger ist als die Hörprobe: wer eine Stimme unter demselben
    /// Namen neu aufnimmt, soll nicht die alte hören.
    ///
    /// Anders als `synthesize_to_file` hängt das NICHT an der aktiven Stimme —
    /// man will ja gerade die anderen hören, ohne umzuschalten.
    pub async fn synthesize_voice_demo(
        &self,
        voice_id: &str,
    ) -> Result<std::path::PathBuf, String> {
        let dir = self
            .demo_dir()
            .ok_or_else(|| "Kein Ablageort für Hörproben".to_string())?;
        // Vor der Ablagefrage, weil der Dateiname der Standardstimme ihren
        // Seed trägt: ein anderer Seed ist eine andere Stimme.
        self.refresh_from_settings();
        let seed = *self.core.seed.lock().unwrap();
        // Leere Kennung = Standardstimme (Seed), die Stimme ohne Referenz.
        // Sie ist so anhörbar wie jede andere — man wählt sie ja gegen die
        // anderen aus, und das geht nur, wenn man sie auch hören kann.
        let reference = (!voice_id.trim().is_empty()).then_some(voice_id);
        let out = match reference {
            Some(id) => dir.join(format!("{id}.wav")),
            None => dir.join(format!("seed-{seed}.wav")),
        };

        let reference_mtime = voices::voice_sample(&self.fish_dir(), voice_id)
            .and_then(|(wav, _)| std::fs::metadata(wav).ok())
            .and_then(|meta| meta.modified().ok());
        let demo_mtime = std::fs::metadata(&out).ok().and_then(|m| m.modified().ok());
        if let (Some(demo), Some(reference)) = (demo_mtime, reference_mtime) {
            if demo >= reference {
                return Ok(out);
            }
        } else if demo_mtime.is_some() && reference_mtime.is_none() {
            return Ok(out);
        }

        self.ensure_server().await?;
        let port = *self.core.port.lock().unwrap();
        // Die Standardstimme wird ueber ihre Seed-Referenz angehoert, nicht
        // referenzlos: sonst waere die Hoerprobe eine andere Person als die,
        // die danach vorliest.
        let seed_reference = match reference {
            Some(_) => None,
            None => self.ensure_seed_reference(port, seed).await,
        };
        let reference = reference.or(seed_reference.as_deref());
        // Immer WAV, unabhängig vom Export-Format des Nutzers: die Hörprobe ist
        // ein interner Cache mit vorhersagbarem Namen, kein Liefergegenstand.
        let body = protocol::tts_request_body_in_format(Self::DEMO_TEXT, seed, reference, "wav");
        let resp = self
            .core
            .http
            .post(format!("{}/v1/tts", protocol::base_url(port)))
            .json(&body)
            .timeout(TTS_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("TTS request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("TTS server answered {}", resp.status()));
        }
        let audio = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
        if !protocol::looks_like_audio(&audio, "wav") {
            return Err("TTS response is not valid wav audio".to_string());
        }
        // Die Hörprobe ist eine eigene Datei, die ein <audio>-Element abspielt
        // — sie geht den Wiedergabe-Pfad NICHT und bekäme dessen Ausgleich
        // sonst nie. Ausgerechnet die Vorschau, an der man Stimmen
        // vergleicht, wäre damit die einzige ungeregelte Stelle.
        let audio = normalize_wav_bytes(&audio).unwrap_or(audio);
        std::fs::write(&out, &audio)
            .map_err(|e| format!("could not write {}: {e}", out.display()))?;
        *self.core.last_used.lock().unwrap() = Instant::now();
        Ok(out)
    }

    /// Den ganzen Vorlesetext — Dialog eingeschlossen — in EINE Datei
    /// schreiben, statt ihn nur zu hören.
    ///
    /// Das Format bestimmt die Einstellung `tts_export_format`, NICHT die
    /// Endung des Zielpfads: wer MP3 eingestellt hat, bekommt MP3, auch wenn
    /// der Dialog eine `.wav` vorgeschlagen hat. Deshalb kommt der
    /// TATSÄCHLICH geschriebene Pfad zurück — die Oberfläche soll die Datei
    /// anzeigen, die es wirklich gibt.
    ///
    /// `opus` wird vorerst wie `wav` behandelt: einen Opus-Kodierer haben wir
    /// nicht, und der Server liefert Opus nur je SATZ — solche Stücke lassen
    /// sich nicht sauber zu einer Datei zusammensetzen (jedes brächte eigene
    /// Kopfdaten mit). Lieber eine brauchbare WAV als eine kaputte Opus.
    ///
    /// Geht bewusst durch dieselbe Zerlegung wie das Abspielen
    /// (`utterances`), damit die Datei Satz für Satz klingt wie das, was man
    /// vorher gehört hat. Zusammengefügt wird mit `hound`: die Teile kommen
    /// als eigenständige WAVs vom Server, und ein simples Aneinanderhängen der
    /// Bytes ergäbe eine Datei mit Kopfdaten mitten im Ton.
    pub async fn speak_to_file(
        self: &Arc<Self>,
        raw: &str,
        out_path: &str,
    ) -> Result<(usize, String), String> {
        let max_chars = *self.core.max_chars.lock().unwrap();
        // Der GANZE Text, nicht die ersten `tts_max_chars` Zeichen. Die
        // Grenze schuetzt das Vorlesen davor, sich an einem Monsterdokument
        // festzufahren — bei einer Datei ist sie sinnlos: wer exportiert,
        // will den Text, den er sieht, und nicht dessen erste 5000 Zeichen.
        // Bis v0.9.0 wurde hier stillschweigend abgeschnitten; sichtbar war
        // das nur daran, dass die Datei frueher endete als der Text.
        //
        // Die Grenze gilt weiterhin je SATZ (siehe unten): ein einzelner
        // Satz, der laenger als das Limit ist, waere fuer den Server ein
        // Problem, nicht fuer uns.
        let utterances = self.utterances(raw.trim());
        if utterances.is_empty() {
            return Err("empty text".to_string());
        }
        self.refresh_from_settings();
        // Das Format entscheidet die Einstellung, nicht die vorgeschlagene
        // Dateiendung — und bei MP3 wird die Endung entsprechend korrigiert.
        let format = self
            .core
            .export_format
            .lock()
            .unwrap()
            .trim()
            .to_lowercase();
        let as_mp3 = format == "mp3";
        let out_path = if as_mp3 {
            std::path::Path::new(out_path)
                .with_extension("mp3")
                .to_string_lossy()
                .into_owned()
        } else {
            out_path.to_string()
        };
        let out_path = out_path.as_str();
        let bitrate = crate::settings::get_settings(&self.app).tts_export_bitrate;
        // Die Bereitschaft durch die Naht, nicht Fish-hart: die Synthese
        // unten läuft ohnehin über `fetch_wav` durch die aktive Engine —
        // nur der Startpfad war bis Paket E2 auf den Fish-Server verdrahtet
        // (und hätte bei Piper 180 s im Health-Timeout gehangen).
        self.ensure_engine_ready().await?;
        self.bind_seed_voice().await;
        let port = *self.core.port.lock().unwrap();
        let seed = *self.core.seed.lock().unwrap();

        // Eigenes Abbruch-Flag je Lauf; ein neuer Export storniert den alten.
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut slot = self.export_cancel.lock().unwrap();
            slot.store(true, Ordering::Release);
            *slot = cancel.clone();
        }
        let total = utterances.len() as u32;
        self.emit_export_progress(0, total, false);

        // Bei MP3 entsteht die WAV im Speicher und wird am Ende EINMAL
        // kodiert; bei WAV (und dem als WAV behandelten Opus) läuft der Ton
        // wie bisher direkt in die Datei.
        let buffer = as_mp3.then(SharedWavBuffer::default);
        let mut writer: Option<ExportSink> = None;
        let mut written = 0usize;
        for (index, (sentence, voice)) in utterances.iter().enumerate() {
            if cancel.load(Ordering::Acquire) {
                // Halbe Datei ist schlimmer als keine: sie sieht fertig aus.
                drop(writer);
                let _ = std::fs::remove_file(out_path);
                self.emit_export_progress(index as u32, total, true);
                return Err("abgebrochen".to_string());
            }
            let Some(part) = protocol::prepare_text(sentence, max_chars) else {
                continue;
            };
            let bytes = self
                .core
                .fetch_wav(port, seed, &part.text, voice.as_deref())
                .await?;
            // Dieselbe Aufbereitung wie beim Hoeren — die Datei soll klingen
            // wie das, was man vorher gehoert hat.
            let strength = *self.core.enhance.lock().unwrap();
            let bytes = prepare_sentence_audio(bytes, strength);
            // Derselbe Ausgleich wie beim Hören: eine exportierte Datei mit
            // wechselnden Stimmen soll nicht lauter und leiser werden.
            let gain = self.core.playback_gain(voice.as_deref(), &bytes);
            let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
                .map_err(|e| format!("Teilstueck nicht lesbar: {e}"))?;
            let spec = reader.spec();
            if writer.is_none() {
                writer = Some(match &buffer {
                    Some(buffer) => ExportSink::Memory(
                        hound::WavWriter::new(buffer.clone(), spec)
                            .map_err(|e| format!("could not write {out_path}: {e}"))?,
                    ),
                    None => ExportSink::File(
                        hound::WavWriter::create(out_path, spec)
                            .map_err(|e| format!("could not write {out_path}: {e}"))?,
                    ),
                });
            }
            let sink = writer.as_mut().expect("writer exists");
            for sample in reader.samples::<i16>() {
                let sample = sample.map_err(|e| format!("Teilstueck beschaedigt: {e}"))?;
                let sample = if gain == 1.0 {
                    sample
                } else {
                    (sample as f32 * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16
                };
                sink.write_sample(sample)
                    .map_err(|e| format!("could not write {out_path}: {e}"))?;
                written += 1;
            }
            self.emit_export_progress(index as u32 + 1, total, false);
        }
        writer
            .ok_or_else(|| "nichts zu schreiben".to_string())?
            .finalize()
            .map_err(|e| format!("could not finish {out_path}: {e}"))?;
        if let Some(buffer) = buffer {
            let mp3 = encode::wav_to_mp3(&buffer.take(), bitrate)?;
            std::fs::write(out_path, mp3)
                .map_err(|e| format!("could not write {out_path}: {e}"))?;
        }
        *self.core.last_used.lock().unwrap() = Instant::now();
        self.emit_export_progress(total, total, false);
        Ok((written, out_path.to_string()))
    }

    /// Zugriff auf den AppHandle fuer Ereignisse aus Hintergrundlaeufen.
    pub fn app_handle(&self) -> tauri::AppHandle {
        self.app.clone()
    }

    /// Um `delta` Saetze springen und von dort weiterlesen.
    ///
    /// Das Vorlesen ist satzweise aufgebaut, nicht als durchgehender Strom —
    /// ein Sprung um 15 Sekunden gaebe es hier gar nicht. Der Satz ist die
    /// Einheit, in der man sich in vorgelesenem Text bewegt.
    pub async fn speak_seek(self: &Arc<Self>, delta: i32) -> Result<usize, String> {
        use tauri::Emitter;
        let (sentences, target) = {
            let mut guard = self.speak_session.lock().unwrap();
            let session = guard.as_mut().ok_or("nichts zum Springen")?;
            let len = session.sentences.len() as i32;
            let next = (session.position as i32 + delta).clamp(0, (len - 1).max(0));
            session.position = next as usize;
            (session.sentences.clone(), next as usize)
        };
        // Die neue Position melden. Ohne das erfuhr die Oberfläche vom Sprung
        // nichts: ihr "Fortsetzen möglich" hängt am Fortschritt, und der kam
        // bisher nur, wenn ein Satz VOLLSTÄNDIG gespielt wurde. Wer sprang und
        // dann pausierte, bekam beim nächsten Druck auf Play einen Neustart
        // von vorn statt der Fortsetzung an der Sprungmarke.
        let _ = self.app.emit(
            "tts-speak-progress",
            serde_json::json!({ "position": target as u32, "total": sentences.len() as u32 }),
        );
        self.run_speak_session(sentences, target).await
    }

    /// Tempo und Lautstaerke der laufenden Wiedergabe. Oeffentlich, damit
    /// die Einstellungsbefehle sie SOFORT setzen koennen — sonst wirkt ein
    /// Dreh am Regler erst beim naechsten Satz, weil `refresh_from_settings`
    /// nur zu Beginn eines Sprechlaufs laeuft.
    pub fn controls(&self) -> &Arc<PlaybackControls> {
        &self.core.controls
    }

    /// Die aktive Stimme hat sich geändert — sofort übernehmen.
    ///
    /// Während des Vorlesens genügt es NICHT, die Einstellung zu spiegeln: die
    /// Satz-Pipeline holt den nächsten Satz bereits, während der aktuelle noch
    /// spielt, ein Wechsel wäre also erst zwei Sätze später zu hören. Läuft
    /// gerade eine Wiedergabe, beginnt sie deshalb beim aktuellen Satz neu —
    /// der wird in der neuen Stimme wiederholt, und ab da gilt sie.
    ///
    /// Sätze mit ausdrücklicher Stimme (Dialogzeilen wie `olga:`) bleiben
    /// unberührt: dort hat der Text die Stimme bestimmt, nicht die Einstellung.
    pub fn apply_voice_change(self: &Arc<Self>) {
        self.refresh_from_settings();
        if self.core.phase() != TtsPhase::Speaking {
            return;
        }
        let Some((sentences, position)) = ({
            let guard = self.speak_session.lock().unwrap();
            guard
                .as_ref()
                .map(|session| (session.sentences.clone(), session.position))
        }) else {
            return;
        };
        if position >= sentences.len() {
            return;
        }
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = manager.run_speak_session(sentences, position).await {
                log::warn!("voice change restart failed: {e}");
            }
        });
    }

    /// Laufenden Datei-Export abbrechen.
    pub fn cancel_export(&self) {
        self.export_cancel
            .lock()
            .unwrap()
            .store(true, Ordering::Release);
    }

    fn emit_export_progress(&self, position: u32, total: u32, cancelled: bool) {
        use tauri::Emitter;
        let _ = self.app.emit(
            "tts-export-progress",
            serde_json::json!({
                "position": position,
                "total": total,
                "cancelled": cancelled,
            }),
        );
    }

    /// Aktuell konfiguriertes Export-Format ("wav" | "mp3" | "opus") — für
    /// den Save-Dialog des Frontends.
    pub fn export_format(&self) -> String {
        crate::settings::get_settings(&self.app).tts_export_format
    }

    // ------------------------------------------------------------------
    // Hörbuch / Dokument-Vorlesen mit persistentem Fortschritt
    // ------------------------------------------------------------------

    fn reading_store(&self) -> Option<std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>> {
        use tauri_plugin_store::StoreExt;
        self.app
            .store(crate::portable::store_path(READING_STORE))
            .map_err(|e| log::warn!("reading store unavailable: {e}"))
            .ok()
    }

    fn stored_reading(&self, key: &str) -> Option<ReadingInfo> {
        let value = self.reading_store()?.get(key)?;
        Some(ReadingInfo {
            key: key.to_string(),
            title: value["title"].as_str().unwrap_or(key).to_string(),
            position: value["position"].as_u64().unwrap_or(0) as u32,
            total: value["total"].as_u64().unwrap_or(0) as u32,
            finished: value["finished"].as_bool().unwrap_or(false),
            playing: false,
        })
    }

    fn persist_reading(&self, key: &str, title: &str, position: u32, total: u32) {
        if let Some(store) = self.reading_store() {
            store.set(
                key.to_string(),
                serde_json::json!({
                    "title": title,
                    "position": position,
                    "total": total,
                    "finished": position >= total && total > 0,
                    "updated": chrono::Utc::now().to_rfc3339(),
                }),
            );
        }
    }

    fn emit_reading(&self, info: &ReadingInfo) {
        use tauri::Emitter;
        if let Err(e) = self.app.emit("tts-reading-progress", info.clone()) {
            log::warn!("Could not emit tts-reading-progress: {e}");
        }
    }

    /// Dokument öffnen (txt/md/pdf/docx): Text extrahieren, in Sätze teilen,
    /// gespeicherten Fortschritt übernehmen. Der Eintrag erscheint sofort in
    /// der Bibliotheksliste.
    pub fn reading_open(&self, path: &str) -> Result<ReadingInfo, String> {
        let p = std::path::Path::new(path);
        let text = crate::media::extract_document_text(p)?;
        let sentences = protocol::split_sentences(&text);
        let total = sentences.len() as u32;
        if total == 0 {
            return Err("Das Dokument enthält keine vorlesbaren Sätze".into());
        }
        let title = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        let position = self
            .stored_reading(path)
            .map(|info| state::resume_position(info.position, total))
            .unwrap_or(0);
        *self.reading.lock().unwrap() = Some(ReadingSession {
            key: path.to_string(),
            title: title.clone(),
            sentences,
        });
        self.persist_reading(path, &title, position, total);
        let info = ReadingInfo {
            key: path.to_string(),
            title,
            position,
            total,
            finished: false,
            playing: false,
        };
        self.emit_reading(&info);
        Ok(info)
    }

    /// Wiedergabe des geöffneten Dokuments ab der gespeicherten Position.
    /// Kehrt sofort zurück; Fortschritt kommt als `tts-reading-progress`.
    pub fn reading_play(self: &Arc<Self>) -> Result<(), String> {
        let (key, title, sentences) = {
            let guard = self.reading.lock().unwrap();
            let session = guard.as_ref().ok_or("kein Dokument geöffnet")?;
            (
                session.key.clone(),
                session.title.clone(),
                session.sentences.clone(),
            )
        };
        let total = sentences.len() as u32;
        let start = self
            .stored_reading(&key)
            .map(|info| state::resume_position(info.position, total))
            .unwrap_or(0);

        let manager = Arc::clone(self);
        let task_key = key.clone();
        let task_title = title.clone();
        tauri::async_runtime::spawn(async move {
            let (key, title) = (task_key, task_title);
            manager.refresh_from_settings();
            // Bereits synthetisierte Passagen spielen offline — der Server
            // startet nur, wenn noch Sätze fehlen.
            if let Err(e) = manager
                .ensure_server_for(&sentences[(start as usize).min(sentences.len())..])
                .await
            {
                log::error!("reading: server start failed: {e}");
                return;
            }
            // Live-Anzeige des gelesenen Satzes.
            let now_manager = Arc::clone(&manager);
            let now_sentences = sentences.clone();
            let on_playing: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |idx| {
                use tauri::Emitter;
                let _ = now_manager.app.emit(
                    "tts-current-sentence",
                    serde_json::json!({
                        "context": "reading",
                        "index": idx as u32,
                        "text": now_sentences.get(idx).cloned().unwrap_or_default(),
                    }),
                );
            });
            let cb_manager = Arc::clone(&manager);
            let cb_key = key.clone();
            let cb_title = title.clone();
            let on_played: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |idx| {
                let position = idx as u32 + 1;
                cb_manager.persist_reading(&cb_key, &cb_title, position, total);
                cb_manager.emit_reading(&ReadingInfo {
                    key: cb_key.clone(),
                    title: cb_title.clone(),
                    position,
                    total,
                    finished: position >= total,
                    playing: position < total,
                });
            });
            let result = manager
                .core
                .speak_sentence_run(
                    single_voice(sentences),
                    start as usize,
                    Some(on_playing),
                    Some(on_played),
                )
                .await;
            if let Err(e) = result {
                log::warn!("reading: playback ended with error: {e}");
            }
            // Endzustand melden (Pause oder fertig): playing=false.
            if let Some(info) = manager.stored_reading(&key) {
                manager.emit_reading(&info);
            }
        });
        // Startzustand sofort melden.
        self.emit_reading(&ReadingInfo {
            key,
            title,
            position: start,
            total,
            finished: false,
            playing: true,
        });
        Ok(())
    }

    /// Pause = Abbruch des laufenden Sprechens; die Position des letzten
    /// vollständig gehörten Satzes ist bereits persistiert.
    pub fn reading_pause(&self) {
        self.core.cancel_core();
    }

    /// Bibliothek: alle gespeicherten Dokumente mit Fortschritt.
    pub fn reading_list(&self) -> Vec<ReadingInfo> {
        let Some(store) = self.reading_store() else {
            return Vec::new();
        };
        let mut list: Vec<ReadingInfo> = store
            .keys()
            .into_iter()
            .filter_map(|key| self.stored_reading(&key))
            .collect();
        list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        list
    }

    /// Fortschritt eines Dokuments auf Anfang zurücksetzen.
    pub fn reading_reset(&self, key: &str) -> Result<(), String> {
        let info = self.stored_reading(key).ok_or("unbekanntes Dokument")?;
        self.persist_reading(key, &info.title, 0, info.total);
        self.emit_reading(&ReadingInfo {
            position: 0,
            finished: false,
            playing: false,
            ..info
        });
        Ok(())
    }

    /// Satzweises Springen im geöffneten Dokument (delta z. B. -1/+1).
    /// Läuft die Wiedergabe, setzt sie an der neuen Position fort.
    pub fn reading_seek(self: &Arc<Self>, delta: i32) -> Result<ReadingInfo, String> {
        let (key, title, total) = {
            let guard = self.reading.lock().unwrap();
            let session = guard.as_ref().ok_or("kein Dokument geöffnet")?;
            (
                session.key.clone(),
                session.title.clone(),
                session.sentences.len() as u32,
            )
        };
        let current = self.stored_reading(&key).map(|i| i.position).unwrap_or(0);
        let new_pos = (i64::from(current) + i64::from(delta)).clamp(0, i64::from(total) - 1) as u32;
        let was_playing = self.core.phase() == TtsPhase::Speaking;
        self.persist_reading(&key, &title, new_pos, total);
        let info = ReadingInfo {
            key,
            title,
            position: new_pos,
            total,
            finished: false,
            playing: was_playing,
        };
        self.emit_reading(&info);
        if was_playing {
            self.core.cancel_core();
            self.reading_play()?;
        }
        Ok(info)
    }

    /// Dokument aus der Bibliothek entfernen (Datei bleibt unberührt).
    pub fn reading_remove(&self, key: &str) -> Result<(), String> {
        if let Some(store) = self.reading_store() {
            store.delete(key.to_string());
        }
        let mut guard = self.reading.lock().unwrap();
        if guard.as_ref().is_some_and(|s| s.key == key) {
            *guard = None;
        }
        Ok(())
    }

    /// Stimme löschen; war sie aktiv, fällt die Auswahl auf die
    /// Seed-Standardstimme zurück.
    pub fn delete_voice_id(&self, id: &str) -> Result<(), String> {
        voices::delete_voice(&self.fish_dir(), id)
            .inspect(|_| voices::update_registry(&self.fish_dir()))?;
        let mut settings = crate::settings::get_settings(&self.app);
        if settings.tts_voice.as_deref() == Some(id) {
            settings.tts_voice = None;
            crate::settings::write_settings(&self.app, settings);
        }
        self.refresh_from_settings();
        Ok(())
    }

    /// Selbsttest-Messpfad: Server sicherstellen, WAV holen (ohne Playback),
    /// Zeiten melden. Rückgabe: (wav, server_start_ms, tts_ms) —
    /// server_start_ms ist 0, wenn ein Server bereits lief. `voice_override`
    /// übersteuert die persistierte Stimme nur für diesen Lauf.
    pub async fn bench_fetch(
        &self,
        text: &str,
        voice_override: Option<&str>,
    ) -> Result<(Vec<u8>, u64, u64), String> {
        self.refresh_from_settings();
        if let Some(voice) = voice_override {
            *self.core.voice.lock().unwrap() = Some(voice.to_string());
        }
        // Der Health-Poll und das "läuft schon"-Konzept gehören zum
        // Fish-Server; bei Piper gibt es keinen Prozess, der schon laufen
        // könnte — dort misst der Startanteil nur den Pfad-Check (~0 ms)
        // und der Fish-Server bleibt aus.
        let fish = self.core.engine_kind() == TtsEngineKind::Fish;
        let already_running = fish && self.core.ensure_server_core().await.is_ok();
        let start = Instant::now();
        if !already_running {
            self.ensure_engine_ready().await?;
        }
        let server_start_ms = if already_running {
            0
        } else {
            start.elapsed().as_millis() as u64
        };

        let prepared = {
            let max_chars = *self.core.max_chars.lock().unwrap();
            protocol::prepare_text(text, max_chars).ok_or_else(|| "empty text".to_string())?
        };
        let port = *self.core.port.lock().unwrap();
        let seed = *self.core.seed.lock().unwrap();
        let tts_start = Instant::now();
        let wav = self
            .core
            .fetch_wav(port, seed, &prepared.text, None)
            .await?;
        let tts_ms = tts_start.elapsed().as_millis() as u64;
        Ok((wav, server_start_ms, tts_ms))
    }

    /// Beendet AUSSCHLIESSLICH einen selbst gestarteten Serverprozess.
    /// Den Fish-Speech-Server beenden — auch einen, den die App nicht selbst
    /// gestartet hat.
    ///
    /// Frueher hat sie fremde Prozesse grundsaetzlich in Ruhe gelassen. Das ist
    /// als Regel vertretbar, in der Praxis aber unbrauchbar: der Server belegt
    /// rund 17 GB VRAM, und wer ihn einmal von Hand gestartet hat, musste zum
    /// Taskmanager greifen, um seine Grafikkarte zurueckzubekommen.
    ///
    /// Erkannt wird er ueber zwei Merkmale zugleich — er lauscht auf dem
    /// eingestellten TTS-Port UND antwortet auf `/v1/health`. Ein fremdes
    /// Programm, das zufaellig denselben Port belegt, wird damit nicht
    /// getroffen; die Gesundheitsantwort ist der Ausweis.
    pub async fn stop_server_any(&self) -> Result<(), String> {
        self.core.cancel_core();

        if self.core.owns_server() {
            self.kill_owned_child();
            self.core.owns_server.store(false, Ordering::Release);
            self.core.set_phase(TtsPhase::Stopped, None);
            return Ok(());
        }

        let port = *self.core.port.lock().unwrap();
        if !self.core.health_ok(port).await {
            // Nichts da, was zu beenden waere — Zustand nur aufraeumen.
            self.core.set_phase(TtsPhase::Stopped, None);
            return Ok(());
        }
        let pid = listening_pid(port)
            .ok_or_else(|| format!("Kein Prozess gefunden, der auf Port {port} lauscht"))?;
        kill_pid(pid)?;
        log::info!("fish-speech (fremd gestartet, PID {pid}) auf Port {port} beendet");
        self.core.set_phase(TtsPhase::Stopped, None);
        Ok(())
    }

    /// Hart beenden: alles abschießen, was auf dem TTS-Port lauscht — ohne
    /// vorher zu fragen, ob es gesund ist.
    ///
    /// `stop_server_any` prüft bei einem fremd gestarteten Server erst die
    /// Gesundheit und meldet „nichts zu beenden", wenn keine Antwort kommt.
    /// Genau dann braucht man diesen Knopf aber: ein Server, der beim Starten
    /// hängt oder nicht mehr antwortet, hält trotzdem rund 17 GB VRAM fest,
    /// und der einzige Ausweg war bisher der Taskmanager.
    ///
    /// Rückgabe: was tatsächlich passiert ist, für die Rückmeldung an den
    /// Nutzer — „nichts gefunden" ist ein Ergebnis, kein Fehler.
    pub fn kill_server_hard(&self) -> Result<String, String> {
        // Zuerst: ein laufender Startversuch soll nicht weiterlaufen und
        // hinterher auch nicht wiederholt werden.
        self.core.stop_requested.store(true, Ordering::Release);
        self.core.cancel_core();
        let owned = self.core.owns_server();
        if owned {
            self.kill_owned_child();
            self.core.owns_server.store(false, Ordering::Release);
        }
        let port = *self.core.port.lock().unwrap();
        // Auch nach dem eigenen Kind noch auf dem Port nachsehen: ein
        // Serverstart, der zweimal lief, hinterlässt einen Prozess, der uns
        // nicht mehr gehört (beobachtet am 20.08.2026, drei Startzeilen).
        let killed_foreign = match listening_pid(port) {
            Some(pid) => match kill_pid(pid) {
                Ok(()) => {
                    log::info!("fish-speech auf Port {port} (PID {pid}) hart beendet");
                    true
                }
                Err(e) => {
                    self.core.set_phase(TtsPhase::Stopped, None);
                    return Err(e);
                }
            },
            None => false,
        };
        self.core.set_phase(TtsPhase::Stopped, None);
        Ok(match (owned, killed_foreign) {
            (_, true) => format!("Prozess auf Port {port} beendet"),
            (true, false) => "Eigener Serverprozess beendet".to_string(),
            (false, false) => format!("Kein Prozess auf Port {port} gefunden"),
        })
    }

    /// Nur den selbst gestarteten Prozess beenden (Idle-Watchdog, Herunterfahren).
    pub fn stop_server(&self) {
        self.core.stop_requested.store(true, Ordering::Release);
        self.core.cancel_core();
        self.kill_owned_child();
        // Auch das, was uns nicht gehoert: beim Beenden der Anwendung darf
        // kein Serverprozess ueberleben, egal wer ihn gestartet hat. Ein
        // verwaister Prozess haelt 17 GB VRAM, die niemand mehr freigibt —
        // die App kann ihn danach nicht einmal mehr finden.
        let port = *self.core.port.lock().unwrap();
        if let Some(pid) = listening_pid(port) {
            if let Err(e) = kill_pid(pid) {
                log::warn!("Could not stop server on port {port}: {e}");
            }
        }
        self.core.owns_server.store(false, Ordering::Release);
        self.core.set_phase(TtsPhase::Stopped, None);
    }

    /// Den eigenen Serverprozess beenden — samt seiner Kinder.
    ///
    /// `Child::kill` beendet unter Windows NUR den direkten Prozess. Der
    /// Fish-API-Server startet aber einen Arbeitsprozess, und der haelt das
    /// Modell: gemessen am 21.08.2026 7,92 GB, die nach einem vermeintlich
    /// erfolgreichen Stopp weiterliefen. Deshalb erst den Baum ueber
    /// `taskkill /T`, danach der uebliche Weg als Rueckfallebene.
    fn kill_owned_child(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            #[cfg(windows)]
            if let Err(e) = kill_pid(child.id()) {
                log::warn!("Could not kill fish-speech process tree: {e}");
            }
            if let Err(e) = child.kill() {
                log::debug!("fish-speech child already gone: {e}");
            }
            let _ = child.wait();
        }
    }

    // ---- Sprecher-Registry (Paket B-S1) ------------------------------------

    /// Alle bekannten Sprecher fuer den Sprecher-Parser
    /// (`protocol::split_speaker_segments`): id und Anzeigename als Namen,
    /// gegen die ein `<Name>`/`Name:`-Marker im Vorlesetext abgeglichen wird.
    /// Nur nicht-interne Stimmen (siehe `voices::list_voices`).
    pub fn known_speakers(&self) -> Vec<protocol::KnownSpeaker> {
        let fish_dir = self.fish_dir();
        voices::list_voices(&fish_dir)
            .into_iter()
            .map(|id| {
                let meta = registry::read_meta(&fish_dir, &id);
                protocol::KnownSpeaker {
                    names: vec![id.clone(), meta.display_name],
                    id,
                }
            })
            .collect()
    }

    /// Alle Stimmen samt Metadaten, Herkunft und Avatar-Pfad — Grundlage der
    /// Stimmenuebersicht.
    pub fn list_voice_infos(&self) -> Vec<registry::VoiceInfo> {
        let fish_dir = self.fish_dir();
        voices::list_voices(&fish_dir)
            .into_iter()
            .map(|id| {
                let meta = registry::read_meta(&fish_dir, &id);
                let origin = match voices::read_seed_marker(&fish_dir, &id) {
                    Some(seed) => registry::VoiceOrigin::Seed(seed),
                    None => registry::VoiceOrigin::Recording,
                };
                let avatar_path =
                    voices::avatar_path(&fish_dir, &id).map(|p| p.to_string_lossy().into_owned());
                registry::VoiceInfo {
                    id,
                    meta,
                    origin,
                    avatar_path,
                }
            })
            .collect()
    }

    /// Metadaten einer Stimme lesen (Default, falls keine `meta.json`
    /// existiert). Lehnt Pfad-Traversal UND unbekannte Stimmen ab — siehe
    /// `registry::require_known_voice`.
    pub fn get_voice_meta(&self, id: &str) -> Result<registry::VoiceMeta, String> {
        registry::get_voice_meta_checked(&self.fish_dir(), id)
    }

    /// Metadaten einer Stimme validieren und speichern.
    pub fn set_voice_meta(&self, id: &str, meta: registry::VoiceMeta) -> Result<(), String> {
        registry::set_voice_meta_checked(&self.fish_dir(), id, meta)
    }

    /// Avatar-Datei einer Stimme setzen/ersetzen (und `meta.avatar`
    /// mitfuehren). `ext` ohne Punkt (`png`/`webp`/`jpg`). Bytes kommen roh
    /// (kein Base64 — siehe `voices::save_avatar`).
    pub fn set_voice_avatar(&self, id: &str, bytes: Vec<u8>, ext: &str) -> Result<(), String> {
        registry::set_voice_avatar_checked(&self.fish_dir(), id, &bytes, ext)
    }

    /// Avatar-Datei einer Stimme entfernen (und `meta.avatar` zuruecksetzen,
    /// falls er ein Bild war).
    pub fn clear_voice_avatar(&self, id: &str) -> Result<(), String> {
        registry::clear_voice_avatar_checked(&self.fish_dir(), id)
    }

    /// Die einbehaltene Referenzaufnahme (siehe `record_reference_stop`) als
    /// Stil-Referenz einer Stimme speichern und in ihrer `meta.json`
    /// eintragen (ersetzt einen vorhandenen Stil gleicher `style_id`).
    ///
    /// Anders als beim Speichern der Hauptreferenz gibt es hier keinen
    /// Zwischenschritt zum Nachbearbeiten des Transkripts durch den Nutzer:
    /// die STT laeuft genau hier, einmalig. Schlaegt sie fehl, bleibt das
    /// Transkript leer — und `save_voice` lehnt eine leere Transkription ab,
    /// genau wie bei jeder anderen Referenz.
    pub fn save_style_reference(
        &self,
        voice: &str,
        style_id: &str,
        name: &str,
    ) -> Result<(), String> {
        use tauri::Manager;
        let fish_dir = self.fish_dir();
        // Traversal-/Existenzschutz fuer BEIDE Kennungen, ALS ALLERERSTES —
        // per tempdir-Test in registry.rs belegt (`resolve_style_target`),
        // dass ein Fehler hier `pending_reference` unangetastet laesst:
        // diese Zeile kommt vor jedem Zugriff darauf.
        let (voice, style_id) = registry::resolve_style_target(&fish_dir, voice, style_id)?;
        let samples = self
            .pending_reference
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| "keine Referenzaufnahme vorhanden".to_string())?;
        let tm = self
            .app
            .state::<Arc<crate::managers::transcription::TranscriptionManager>>();
        let transcript = tm.transcribe(samples.clone()).unwrap_or_default();
        let reference = match voices::save_style_voice(
            &fish_dir,
            &voice,
            &style_id,
            &samples,
            &transcript,
            *self.core.enhance.lock().unwrap(),
        ) {
            Ok(reference) => reference,
            Err(e) => {
                *self.pending_reference.lock().unwrap() = Some(samples);
                return Err(e);
            }
        };
        let mut meta = registry::read_meta(&fish_dir, &voice);
        meta.styles.retain(|s| s.id != style_id);
        meta.styles.push(registry::VoiceStyle {
            id: style_id,
            name: name.to_string(),
            tags: Vec::new(),
            reference: Some(reference),
        });
        registry::write_meta(&fish_dir, &voice, &meta)
    }

    /// Einen Stil samt seiner Referenzaufnahme entfernen.
    pub fn delete_style(&self, voice: &str, style_id: &str) -> Result<(), String> {
        registry::delete_style_checked(&self.fish_dir(), voice, style_id)
    }

    /// Referenzaufnahme einer gespeicherten Stimme auf die Lautstaerke-
    /// Heuristik hin analysieren (siehe `registry::analyze_reference`).
    pub fn analyze_reference(&self, voice: &str) -> Result<registry::ReferenceAnalysis, String> {
        registry::analyze_stored_reference(&self.fish_dir(), voice)
    }

    /// Wie `analyze_reference`, aber fuer die noch nicht gespeicherte,
    /// einbehaltene Aufnahme (`pending_reference`) — fuer die Vorschau VOR
    /// dem Speichern einer neuen Stimme.
    pub fn analyze_pending_reference(&self) -> Result<registry::ReferenceAnalysis, String> {
        let samples = self
            .pending_reference
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "keine Referenzaufnahme vorhanden".to_string())?;
        Ok(registry::analyze_reference(&samples, 16_000))
    }

    /// Hoerprobe eines beliebigen Seeds, ohne ihn als Stimme zu sichern —
    /// Verallgemeinerung von `ensure_seed_reference`/`synthesize_voice_demo`
    /// mit explizitem Seed statt dem eingestellten. Liefert die WAV-Bytes
    /// direkt, ohne sie im Hoerproben-Cache abzulegen: der Aufrufer
    /// entscheidet erst danach, ob dieser Seed ueberhaupt eine Stimme wird.
    pub async fn seed_preview(&self, seed: i64) -> Result<Vec<u8>, String> {
        self.refresh_from_settings();
        self.ensure_server().await?;
        let port = *self.core.port.lock().unwrap();
        let body = protocol::tts_request_body_in_format(Self::DEMO_TEXT, seed, None, "wav");
        let resp = self
            .core
            .http
            .post(format!("{}/v1/tts", protocol::base_url(port)))
            .json(&body)
            .timeout(TTS_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("TTS request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("TTS server answered {}", resp.status()));
        }
        let audio = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
        if !protocol::looks_like_wav(&audio) {
            return Err("TTS response is not a WAV file".to_string());
        }
        Ok(normalize_wav_bytes(&audio).unwrap_or(audio))
    }

    /// Wie `save_seed_voice`, aber mit explizitem Seed (statt dem
    /// eingestellten) und mit `VoiceMeta` statt bloss einem Namen — die
    /// Metadaten werden vor dem Speichern validiert.
    pub async fn save_seed_voice_v2(
        &self,
        seed: i64,
        display_name: &str,
        meta: registry::VoiceMeta,
    ) -> Result<String, String> {
        let id = voices::sanitize_voice_id(display_name)
            .ok_or_else(|| "Der Name ergibt keinen brauchbaren Stimmennamen".to_string())?;
        let fish_dir = self.fish_dir();
        if voices::voice_is_complete(&fish_dir, &id) {
            return Err(format!("Die Stimme '{id}' existiert bereits"));
        }
        let others = registry::other_voice_names(&fish_dir, Some(&id));
        registry::validate_meta(&meta, &others)?;

        self.refresh_from_settings();
        self.ensure_server().await?;
        let port = *self.core.port.lock().unwrap();
        let source_id = self
            .ensure_seed_reference(port, seed)
            .await
            .ok_or_else(|| "Die Seed-Referenz liess sich nicht erzeugen".to_string())?;
        let source = voices::voice_dir(&fish_dir, &source_id);
        let target = voices::voice_dir(&fish_dir, &id);
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("could not create {}: {e}", target.display()))?;
        for file in ["sample.wav", "sample.lab"] {
            if let Err(e) = std::fs::copy(source.join(file), target.join(file)) {
                // Unvollstaendiges Verzeichnis nicht stehen lassen — sonst
                // meldet `voice_is_complete` beim naechsten Versuch
                // faelschlich "existiert bereits" fuer eine Stimme, die nie
                // fertig wurde.
                let _ = std::fs::remove_dir_all(&target);
                return Err(format!("could not copy {file}: {e}"));
            }
        }
        voices::write_seed_marker(&fish_dir, &id, seed);
        registry::write_meta(&fish_dir, &id, &meta)?;
        voices::update_registry(&fish_dir);
        log::info!("Seed {seed} als Stimme '{id}' (mit Metadaten) gespeichert");
        Ok(id)
    }

    // ---- Stimmen-Baukasten (Etappe 1) --------------------------------------

    /// Kandidaten fuer einen Entwurf erzeugen: je Kandidat ein zufaelliger
    /// Seed, derselbe Probesatz. Nach JEDEM fertigen Kandidaten wird der
    /// Entwurf geschrieben — bricht der Lauf ab (Abbruch, Absturz,
    /// Serverfehler), bleibt alles bereits Gewuerfelte erhalten.
    ///
    /// Der Seed ist der einzige Regler fuer die Stimmidentitaet (Fish-Speech
    /// kennt keine Konditionierung auf eine Beschreibung) — deshalb ist das
    /// Wuerfeln hier der Kern und nicht ein Beiwerk.
    pub async fn builder_generate(
        &self,
        draft_id: &str,
        count: usize,
        mut cancel: tokio::sync::watch::Receiver<bool>,
        mut on_candidate: impl FnMut(usize, usize, &builder::Candidate),
    ) -> Result<builder::BuilderDraft, String> {
        let fish_dir = self.fish_dir();
        let mut draft = builder::load_draft(&fish_dir, draft_id)?;
        if draft.probe_text.trim().is_empty() {
            return Err("Ohne Probesatz gibt es nichts zu sprechen".to_string());
        }
        self.refresh_from_settings();
        self.ensure_server().await?;
        let port = *self.core.port.lock().unwrap();
        let dir = builder::draft_dir(&fish_dir, draft_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

        // Die Tags des Entwurfs gehoeren in den Probesatz: die Prosodie der
        // Referenz uebertraegt sich beim Klonen, ein "[slow]" hier wirkt also
        // auf die spaetere Stimme, nicht nur auf diese eine Aufnahme.
        let mut text = String::new();
        for tag in &draft.tags {
            text.push('[');
            text.push_str(tag);
            text.push_str("] ");
        }
        text.push_str(&draft.probe_text);

        for index in 0..count {
            if *cancel.borrow_and_update() {
                break;
            }
            // Zufall ohne `rand`: die unteren Bits einer ULID sind der
            // Zufallsanteil, und `ulid` ist im Projekt bereits Abhaengigkeit.
            let seed: i64 = (ulid::Ulid::new().0 as u32) as i64;
            let body = protocol::tts_request_body_in_format(&text, seed, None, "wav");
            let resp = self
                .core
                .http
                .post(format!("{}/v1/tts", protocol::base_url(port)))
                .json(&body)
                .timeout(TTS_TIMEOUT)
                .send()
                .await
                .map_err(|e| format!("Kandidat {} nicht erzeugt: {e}", index + 1))?;
            if !resp.status().is_success() {
                return Err(format!(
                    "Kandidat {} nicht erzeugt: Server antwortete {}",
                    index + 1,
                    resp.status()
                ));
            }
            let audio = resp
                .bytes()
                .await
                .map_err(|e| format!("Kandidat {} unvollstaendig: {e}", index + 1))?
                .to_vec();
            if !protocol::looks_like_wav(&audio) {
                return Err(format!("Kandidat {} war kein WAV", index + 1));
            }
            let audio = normalize_wav_bytes(&audio).unwrap_or(audio);
            let file = format!("cand_{seed}.wav");
            std::fs::write(dir.join(&file), &audio)
                .map_err(|e| format!("Kandidat {} nicht gespeichert: {e}", index + 1))?;
            let candidate = builder::Candidate {
                seed,
                file,
                created_at: chrono::Utc::now().timestamp(),
                source: builder::CandidateSource::Seed,
            };
            draft.candidates.push(candidate.clone());
            draft.updated_at = chrono::Utc::now().timestamp();
            // Erst die Datei, dann der Entwurf: ein Absturz dazwischen laesst
            // eine verwaiste WAV zurueck (harmlos), nie einen Entwurf, der auf
            // eine fehlende Datei zeigt.
            builder::save_draft(&fish_dir, &draft)?;
            on_candidate(index + 1, count, &candidate);
        }
        Ok(draft)
    }

    /// Einen Kandidaten zum Anhoeren liefern — mit dem aktuellen Tiefe-Regler
    /// des Entwurfs. Die Original-WAV bleibt unveraendert liegen, damit der
    /// Regler beliebig oft neu gestellt werden kann, ohne neu zu wuerfeln.
    pub fn builder_candidate_wav(&self, draft_id: &str, seed: i64) -> Result<Vec<u8>, String> {
        let fish_dir = self.fish_dir();
        let draft = builder::load_draft(&fish_dir, draft_id)?;
        let candidate = draft
            .candidates
            .iter()
            .find(|c| c.seed == seed)
            .ok_or_else(|| format!("Kandidat {seed} gehoert nicht zu diesem Entwurf"))?;
        let path = builder::draft_dir(&fish_dir, draft_id).join(&candidate.file);
        let raw =
            std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        apply_depth(&raw, draft.depth).ok_or_else(|| "Kandidat liess sich nicht lesen".to_string())
    }

    /// Eine WAV-Datei als Kandidat in einen Entwurf holen (Etappe 2).
    ///
    /// Derselbe Weg wie `builder_generate`, nur ohne Server: zuschneiden,
    /// auf denselben Pegel bringen wie jede andere Referenz
    /// (`ensure_seed_reference` tut nichts anderes), als `cand_<kennzahl>.wav`
    /// in den Entwurfsordner schreiben, dann den Entwurf sichern — erst die
    /// Datei, dann der Entwurf, damit nie ein Entwurf auf eine fehlende Datei
    /// zeigt.
    ///
    /// `start_sec`/`end_sec` schneiden zu; `0.0/0.0` nimmt die ganze Datei.
    /// Ueber `MAX_REFERENCE_SEC` wird gekappt (siehe dort).
    pub fn builder_add_wav(
        &self,
        draft_id: &str,
        wav_path: &str,
        start_sec: f32,
        end_sec: f32,
    ) -> Result<builder::BuilderDraft, String> {
        let fish_dir = self.fish_dir();
        let mut draft = builder::load_draft(&fish_dir, draft_id)?;
        let path = std::path::Path::new(wav_path);
        let raw =
            std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        if !protocol::looks_like_wav(&raw) {
            return Err(format!("{} ist keine brauchbare WAV-Datei", path.display()));
        }
        let trimmed = trim_wav_bytes(&raw, start_sec, end_sec)
            .ok_or_else(|| format!("{} liess sich nicht lesen", path.display()))?;
        let audio = normalize_wav_bytes(&trimmed).unwrap_or(trimmed);

        // Kennzahl statt Seed: dieser Kandidat ist nicht gewuerfelt und
        // nicht reproduzierbar. Die Zahl adressiert ihn nur — gewonnen wie
        // in `builder_generate` aus dem Zufallsanteil einer ULID.
        let key: i64 = (ulid::Ulid::new().0 as u32) as i64;
        // Was im eigenen Datenverzeichnis liegt, hat die App selbst
        // aufgenommen; alles andere hat der Nutzer von aussen gewaehlt. Fuer
        // die Verarbeitung macht es keinen Unterschied — beide tragen keinen
        // Seed —, aber die Oberflaeche darf sagen, woher es kam.
        let source = if path.starts_with(&fish_dir) {
            builder::CandidateSource::Recording
        } else {
            builder::CandidateSource::Import
        };
        let dir = builder::draft_dir(&fish_dir, draft_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let file = format!("cand_{key}.wav");
        std::fs::write(dir.join(&file), &audio)
            .map_err(|e| format!("Kandidat nicht gespeichert: {e}"))?;
        draft.candidates.push(builder::Candidate {
            seed: key,
            file,
            created_at: chrono::Utc::now().timestamp(),
            source,
        });
        draft.updated_at = chrono::Utc::now().timestamp();
        builder::save_draft(&fish_dir, &draft)?;
        Ok(draft)
    }

    /// Den gewaehlten Kandidaten als Stimme speichern.
    ///
    /// Bewusst dieselbe Strecke wie `save_seed_voice_v2` — nur die Quelle der
    /// WAV ist eine andere: Kandidat statt Seed-Referenz. Zwei Speicherwege
    /// wuerden garantiert auseinanderlaufen.
    pub async fn builder_commit(
        &self,
        draft_id: &str,
        meta: registry::VoiceMeta,
    ) -> Result<String, String> {
        let fish_dir = self.fish_dir();
        let draft = builder::load_draft(&fish_dir, draft_id)?;
        let seed = draft
            .selected
            .ok_or_else(|| "Kein Kandidat gewaehlt".to_string())?;
        let id = voices::sanitize_voice_id(&meta.display_name)
            .ok_or_else(|| "Der Name ergibt keinen brauchbaren Stimmennamen".to_string())?;
        if voices::voice_is_complete(&fish_dir, &id) {
            return Err(format!("Die Stimme '{id}' existiert bereits"));
        }
        let others = registry::other_voice_names(&fish_dir, Some(&id));
        registry::validate_meta(&meta, &others)?;

        let audio = self.builder_candidate_wav(draft_id, seed)?;
        let target = voices::voice_dir(&fish_dir, &id);
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("could not create {}: {e}", target.display()))?;
        // Vollstaendig oder gar nicht — die Regel aus `save_seed_voice_v2`:
        // ein halbes Verzeichnis meldet beim naechsten Versuch faelschlich
        // "existiert bereits".
        if let Err(e) = std::fs::write(target.join("sample.wav"), &audio) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(format!("could not write sample.wav: {e}"));
        }
        if let Err(e) = std::fs::write(target.join("sample.lab"), draft.probe_text.as_bytes()) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(format!("could not write sample.lab: {e}"));
        }
        // Der Seed-Vermerk nur fuer einen gewuerfelten Kandidaten: bei einem
        // eingespielten stuende in `seed.txt` eine Kennzahl, die nichts
        // reproduziert — eine Zusage, die die Datei nicht halten kann.
        if let Some(seed) = builder::seed_marker_for(&draft) {
            voices::write_seed_marker(&fish_dir, &id, seed);
        }
        registry::write_meta(&fish_dir, &id, &meta)?;
        voices::update_registry(&fish_dir);
        builder::delete_draft(&fish_dir, draft_id)?;
        log::info!("Baukasten: Entwurf {draft_id} als Stimme '{id}' gespeichert");
        Ok(id)
    }
}

/// PID des Prozesses, der auf `127.0.0.1:port` lauscht.
///
/// Ueber `netstat -ano` statt einer Crate: das Werkzeug gehoert zu Windows, die
/// Ausgabe ist seit Jahrzehnten stabil, und der Alternativweg (IP Helper API)
/// waere fuer eine einzige Abfrage viel unsafe-Code.
/// PID des Prozesses, der auf `port` lauscht.
fn listening_pid(port: u16) -> Option<u32> {
    let output = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .output()
        .ok()?;
    parse_listening_pid(&String::from_utf8_lossy(&output.stdout), port)
}

/// Die Zeile eines lauschenden Sockets aus einer netstat-Ausgabe heraussuchen.
///
/// Erkannt wird an der STRUKTUR, nicht am Statuswort: `netstat` uebersetzt es
/// (deutsch "ABHOEREN", englisch "LISTENING"). Der Vergleich mit "LISTENING"
/// lief auf einem deutschen Windows deshalb immer ins Leere — beide
/// Stopp-Wege der App taten schlicht nichts, und der Serverprozess hielt
/// seine 17 GB VRAM weiter fest (beobachtet 20.08.2026).
///
/// Ein lauschender Socket hat keine Gegenstelle; seine Remoteadresse ist
/// `0.0.0.0:0` bzw. `[::]:0`. Eine ausgehende Verbindung von demselben Port
/// hat dort eine echte Adresse und wird dadurch ausgeschlossen. Das gilt in
/// jeder Sprache, weil dort nur Zahlen stehen.
fn parse_listening_pid(netstat_output: &str, port: u16) -> Option<u32> {
    let wanted = format!(":{port}");
    for line in netstat_output.lines() {
        let mut fields = line.split_whitespace();
        let (Some(proto), Some(local), Some(remote), Some(_state), Some(pid)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        if !proto.eq_ignore_ascii_case("TCP") {
            continue;
        }
        // Nur die Loopback-Adresse: der Server der App laeuft auf 127.0.0.1,
        // und ein fremder Dienst auf 0.0.0.0 desselben Ports geht uns nichts an.
        let local_matches = local.ends_with(&wanted) && local.contains("127.0.0.1");
        let is_listening = remote.ends_with(":0");
        if local_matches && is_listening {
            return pid.parse().ok();
        }
    }
    None
}

fn kill_pid(pid: u32) -> Result<(), String> {
    let output = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .map_err(|e| format!("taskkill nicht ausfuehrbar: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "Prozess {pid} liess sich nicht beenden: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// Winzige WAV-Erzeugung fuer Tests — echte Audiodateien im Repo waeren fuer
/// diese Pruefungen unnoetiger Ballast.
#[cfg(test)]
mod test_support {
    /// Mono, 16 Bit, `rate` Hz, `samples` Werte einer Sinusschwingung.
    pub fn sine_wav(rate: u32, samples: usize) -> Vec<u8> {
        let data_len = samples * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..samples {
            let v = ((i as f32 / 8.0).sin() * 12_000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Der Statustext ist uebersetzt — die Erkennung darf nicht daran haengen.
    /// Beide Ausgaben stammen von echten Systemen (de-DE und en-US).
    #[test]
    fn der_lauschende_prozess_wird_in_jeder_sprache_gefunden() {
        let deutsch = concat!(
            "Aktive Verbindungen\r\n\r\n",
            "  Proto  Lokale Adresse         Remoteadresse          Status           PID\r\n",
            "  TCP    0.0.0.0:135            0.0.0.0:0              ABHÖREN         2284\r\n",
            "  TCP    127.0.0.1:8080         0.0.0.0:0              ABHÖREN         87820\r\n"
        );
        let englisch = concat!(
            "Active Connections\r\n\r\n",
            "  Proto  Local Address          Foreign Address        State           PID\r\n",
            "  TCP    127.0.0.1:8080         0.0.0.0:0              LISTENING       4711\r\n"
        );
        assert_eq!(parse_listening_pid(deutsch, 8080), Some(87820));
        assert_eq!(parse_listening_pid(englisch, 8080), Some(4711));
    }

    /// Eine ausgehende Verbindung VON diesem Port ist kein Server.
    #[test]
    fn eine_bestehende_verbindung_wird_nicht_fuer_den_server_gehalten() {
        let text = "  TCP    127.0.0.1:8080         127.0.0.1:53318        HERGESTELLT     999\r\n";
        assert_eq!(parse_listening_pid(text, 8080), None);
    }

    /// Ein anderer Port und ein Dienst auf allen Adressen gehen uns nichts an.
    #[test]
    fn fremde_ports_und_fremde_adressen_werden_uebergangen() {
        let text = concat!(
            "  TCP    127.0.0.1:8081         0.0.0.0:0              ABHÖREN         111\r\n",
            "  TCP    0.0.0.0:8080           0.0.0.0:0              ABHÖREN         222\r\n"
        );
        assert_eq!(parse_listening_pid(text, 8080), None);
    }

    /// Der echte Fall vom 21.08.2026: Startprotokoll eines Servers, den ein
    /// zerstoerter Compile-Cache umgebracht hat. Die Meldung muss die
    /// Ursachenzeile tragen, nicht die Rahmenzeilen des Tracebacks.
    #[test]
    fn die_ursachenzeile_wird_aus_dem_startprotokoll_gezogen() {
        let log = concat!(
            "Traceback (most recent call last):\r\n",
            r#"  File "C:\AI\fish-speech\tools\api_server.py", line 89, in initialize_app"#,
            "\r\n",
            "    app.state.model_manager = ModelManager(\r\n",
            "torch._dynamo.exc.BackendCompilerFailed: backend='inductor' raised:\r\n",
            "JSONDecodeError: Expecting value: line 1 column 1 (char 0)\r\n",
            "\r\n",
            "ERROR:    Application startup failed. Exiting.\r\n"
        );
        let summary = startup_error_summary(log).expect("Zusammenfassung");
        assert!(summary.contains("Application startup failed"), "{summary}");
        assert!(
            !summary.starts_with("File \""),
            "Rahmenzeile gewaehlt: {summary}"
        );
    }

    /// Ohne Fehlerwort bleibt die letzte nicht leere Zeile — irgendetwas ist
    /// immer besser als eine nackte Nummer.
    #[test]
    fn ohne_fehlerwort_bleibt_die_letzte_zeile() {
        let log = "lade Modell\r\n\r\nfertig\r\n\r\n";
        assert_eq!(startup_error_summary(log).as_deref(), Some("fertig"));
    }

    #[test]
    fn ein_leeres_protokoll_ergibt_keine_zusammenfassung() {
        assert_eq!(startup_error_summary(""), None);
        assert_eq!(startup_error_summary("   \r\n\r\n  "), None);
    }

    /// Eine Fehlermeldung ist kein Protokollfenster: sehr lange Zeilen werden
    /// gekappt, damit sie im Fehlerband der Oberflaeche noch lesbar sind.
    #[test]
    fn sehr_lange_zeilen_werden_gekappt() {
        let log = format!("Error: {}", "x".repeat(500));
        let summary = startup_error_summary(&log).expect("Zusammenfassung");
        assert!(
            summary.chars().count() <= 301,
            "{} Zeichen",
            summary.chars().count()
        );
        assert!(summary.ends_with('…'));
    }

    /// Minimaler HTTP-Server: beantwortet GET /v1/health mit ok und
    /// POST /v1/tts mit einem RIFF-Blob. Zählt TTS-Aufrufe. Schließt jede
    /// Verbindung nach einer Antwort (Connection: close), damit reqwest
    /// nicht auf Keep-Alive besteht.
    async fn spawn_mock(
        tts_calls: Arc<AtomicUsize>,
        tts_bodies: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let calls = tts_calls.clone();
                let bodies = tts_bodies.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let mut read = 0usize;
                    loop {
                        let n = sock.read(&mut buf[read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        read += n;
                        let text = String::from_utf8_lossy(&buf[..read]).to_lowercase();
                        if let Some(header_end) = text.find("\r\n\r\n") {
                            let is_tts = text.starts_with("post /v1/tts");
                            let content_length = text
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length: "))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if read >= header_end + 4 + content_length {
                                let body: Vec<u8> = if is_tts {
                                    calls.fetch_add(1, Ordering::SeqCst);
                                    let received = String::from_utf8_lossy(
                                        &buf[header_end + 4..header_end + 4 + content_length],
                                    )
                                    .to_string();
                                    bodies.lock().unwrap().push(received);
                                    let mut wav = b"RIFF".to_vec();
                                    wav.extend_from_slice(&[0u8; 4096]);
                                    wav
                                } else {
                                    br#"{"status":"ok"}"#.to_vec()
                                };
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
                                    body.len(),
                                    if is_tts { "audio/wav" } else { "application/json" }
                                );
                                let _ = sock.write_all(head.as_bytes()).await;
                                let _ = sock.write_all(&body).await;
                                let _ = sock.shutdown().await;
                                break;
                            }
                        }
                    }
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn an_external_healthy_server_is_adopted_not_owned() {
        let calls = Arc::new(AtomicUsize::new(0));
        let port = spawn_mock(calls, Arc::new(std::sync::Mutex::new(Vec::new()))).await;
        let core = TtsCore::for_test(port);
        core.ensure_server_core().await.unwrap();
        assert_eq!(core.phase(), TtsPhase::Ready);
        assert!(
            !core.owns_server(),
            "extern erkannt → kein Besitz, kein Kill"
        );
    }

    #[tokio::test]
    async fn ein_selbst_gestarteter_server_bleibt_nach_der_gesundheitspruefung_eigener() {
        let calls = Arc::new(AtomicUsize::new(0));
        let port = spawn_mock(calls, Arc::new(std::sync::Mutex::new(Vec::new()))).await;
        let core = TtsCore::for_test(port);
        // So sieht es aus, nachdem die App selbst gespawnt hat.
        core.owns_server.store(true, Ordering::Release);

        core.ensure_server_core().await.unwrap();

        assert!(
            core.owns_server(),
            "die Gesundheitspruefung hat den eigenen Server enteignet — danach              war 'Server stoppen' ausgegraut und der Prozess blieb mit seinem              VRAM stehen"
        );
    }

    #[tokio::test]
    async fn eine_laufende_wiedergabe_wird_von_der_gesundheitspruefung_nicht_beendet() {
        let calls = Arc::new(AtomicUsize::new(0));
        let port = spawn_mock(calls, Arc::new(std::sync::Mutex::new(Vec::new()))).await;
        let core = TtsCore::for_test(port);
        core.set_phase(TtsPhase::Speaking, None);

        core.ensure_server_core().await.unwrap();

        assert_eq!(
            core.phase(),
            TtsPhase::Speaking,
            "die Phase ist zugleich die Anzeige 'spricht gerade'; wird sie              mitten im Vorlesen auf 'Bereit' gesetzt, graut die Oberflaeche              ihren einzigen Stopp-Knopf aus"
        );
    }

    #[tokio::test]
    async fn speak_fetches_wav_and_hands_it_to_the_player() {
        let calls = Arc::new(AtomicUsize::new(0));
        let port = spawn_mock(calls.clone(), Arc::new(std::sync::Mutex::new(Vec::new()))).await;
        let core = TtsCore::for_test(port);
        core.ensure_server_core().await.unwrap();
        let played = core.speak_core("Hallo Welt").await.unwrap();
        assert!(played > 1024, "WAV-Bytes kamen beim Player an");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            core.phase(),
            TtsPhase::Ready,
            "nach dem Sprechen wieder Ready"
        );
    }

    #[tokio::test]
    async fn blank_text_never_reaches_the_server() {
        let calls = Arc::new(AtomicUsize::new(0));
        let port = spawn_mock(calls.clone(), Arc::new(std::sync::Mutex::new(Vec::new()))).await;
        let core = TtsCore::for_test(port);
        core.ensure_server_core().await.unwrap();
        assert!(core.speak_core("   ").await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unreachable_port_is_reported_not_adopted() {
        // Port 1 ist praktisch nie belegt; der Kern darf dann nichts adoptieren.
        let core = TtsCore::for_test(1);
        assert!(core.ensure_server_core().await.is_err());
        assert_eq!(core.phase(), TtsPhase::Stopped);
        assert!(!core.owns_server());
    }

    #[tokio::test]
    async fn multi_sentence_text_is_pipelined_as_separate_requests() {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let port = spawn_mock(calls.clone(), bodies.clone()).await;
        let core = TtsCore::for_test(port);
        core.ensure_server_core().await.unwrap();
        let total = core
            .speak_core("Der erste Satz ist lang genug. Der zweite Satz ist es ebenfalls.")
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "ein Request pro Satz");
        assert!(total > 2 * 1024, "beide WAVs gezählt");
        let all = bodies.lock().unwrap().join("|");
        assert!(all.contains("Der erste Satz"));
        assert!(all.contains("Der zweite Satz"));
    }

    #[tokio::test]
    async fn unchanged_text_is_served_from_the_cache_on_replay() {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let port = spawn_mock(calls.clone(), bodies).await;
        let core = TtsCore::for_test(port);
        core.ensure_server_core().await.unwrap();
        let text = "Dieser Satz ist lang genug für den Cache-Test.";
        core.speak_core(text).await.unwrap();
        core.speak_core(text).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "der zweite, unveränderte Lauf kommt aus dem Cache"
        );
        core.speak_core("Ein anderer Satz erzwingt eine neue Synthese.")
            .await
            .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "neuer Text → neuer Request"
        );
    }

    /// Bereits Vorgelesenes muss OHNE Server abspielbar sein: Der zweite
    /// Kern (leerer RAM-Cache, unerreichbarer Port) bedient sich vom
    /// Platten-Cache des ersten.
    #[tokio::test]
    async fn cached_audio_plays_without_any_server() {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let port = spawn_mock(calls.clone(), bodies).await;
        let cache_dir = tempfile::tempdir().unwrap();

        let text = "Dieser Satz landet im persistenten Plattencache.";
        let online = TtsCore::for_test(port);
        *online.cache_dir.lock().unwrap() = Some(cache_dir.path().to_path_buf());
        online.ensure_server_core().await.unwrap();
        online.speak_core(text).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let offline = TtsCore::for_test(1); // Port 1: kein Server erreichbar
        *offline.cache_dir.lock().unwrap() = Some(cache_dir.path().to_path_buf());
        assert!(
            offline.has_cached(text),
            "Platten-Cache muss erkannt werden"
        );
        let played = offline.speak_core(text).await.unwrap();
        assert!(played > 1024, "Wiedergabe kam vollständig von der Platte");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "kein weiterer Server-Request"
        );
    }

    #[tokio::test]
    async fn a_selected_voice_travels_as_reference_id() {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let port = spawn_mock(calls, bodies.clone()).await;
        let core = TtsCore::for_test(port);
        *core.voice.lock().unwrap() = Some("patrick".into());
        core.ensure_server_core().await.unwrap();
        core.speak_core("Hallo").await.unwrap();
        let all = bodies.lock().unwrap().join("");
        assert!(
            all.contains(r#""reference_id":"patrick""#),
            "Request muss die Stimme tragen, war: {all}"
        );
        assert!(
            all.contains(r#""use_memory_cache":"on""#),
            "Referenz-Cache muss aktiv sein"
        );
    }

    #[tokio::test]
    async fn cancel_marks_the_running_jobs_flag() {
        let core = TtsCore::for_test(1);
        let flag = core.cancelled.lock().unwrap().clone();
        assert!(!flag.load(Ordering::Acquire));
        core.cancel_core();
        assert!(
            flag.load(Ordering::Acquire),
            "cancel muss den laufenden Auftrag treffen"
        );
    }

    /// Regression (19.08.2026): Nach Pause blieb die Phase auf „Spricht"
    /// hängen — Server-Stopp wirkte blockiert und der Idle-Stopp griff nie.
    #[tokio::test]
    async fn cancel_returns_a_speaking_phase_to_ready() {
        let core = TtsCore::for_test(1);
        core.set_phase(TtsPhase::Speaking, None);
        core.cancel_core();
        assert_eq!(core.phase(), TtsPhase::Ready);
        // In anderen Phasen (z. B. Starting) mischt sich cancel nicht ein.
        core.set_phase(TtsPhase::Starting, None);
        core.cancel_core();
        assert_eq!(core.phase(), TtsPhase::Starting);
    }

    // ------------------------------------------------------------------
    // Engine-Naht (Paket A3): Cache-Schlüssel-Stabilität und Mock-Engine
    // ------------------------------------------------------------------

    /// KRITISCH: ohne Engine-Tag muss der Schlüssel byte-identisch zu dem
    /// sein, den der Bestandscode (v0.13.x: text, seed, voice durch den
    /// DefaultHasher) erzeugte — sonst wären alle bereits synthetisierten
    /// Sätze in RAM- und Platten-Cache auf einen Schlag unauffindbar. Die
    /// Referenzwerte sind mit dem Bestandsalgorithmus fixiert; schlägt der
    /// Test fehl, ist der Platten-Cache jedes Bestandsnutzers entwertet.
    #[test]
    fn cache_schluessel_ohne_engine_tag_bleibt_byte_identisch() {
        assert_eq!(
            WavCache::key("", "Hallo Welt.", 42, Some("patrick")),
            0x1ed95698a67842f1
        );
        assert_eq!(
            WavCache::key("", "Hallo Welt.", 42, None),
            0x4ec295e94cdd5369
        );
    }

    /// Ein nicht-leerer Tag trennt die Engines: gleicher Satz, gleiche
    /// Stimme, gleicher Seed — anderer Schlüssel.
    #[test]
    fn ein_engine_tag_ergibt_einen_anderen_cache_schluessel() {
        let fish = WavCache::key("", "Hallo Welt.", 42, Some("patrick"));
        let piper = WavCache::key("piper/eva", "Hallo Welt.", 42, Some("patrick"));
        assert_ne!(fish, piper);
        assert_ne!(
            WavCache::key("piper/eva", "Hallo Welt.", 42, None),
            WavCache::key("piper/thorsten", "Hallo Welt.", 42, None),
            "auch zwei Stimmen derselben Engine trennen sich"
        );
    }

    /// Zweite Engine im Kleinen: liefert fertige WAV-Blobs ohne jeden
    /// Server und zählt ihre Aufrufe — die Trait-Implementierung, mit der
    /// die Naht bewiesen wird.
    pub(super) struct MockEngine {
        pub(super) calls: AtomicUsize,
    }

    impl engine::TtsEngine for MockEngine {
        fn kind(&self) -> TtsEngineKind {
            TtsEngineKind::Piper
        }

        fn caps(&self) -> EngineCaps {
            EngineCaps {
                style_tags: false,
                cloning: false,
                voice_switching: false,
                streaming: false,
                needs_gpu: false,
                export_formats: &["wav"],
            }
        }

        fn cache_tag(&self, voice: Option<&str>) -> String {
            format!("mock/{}", voice.unwrap_or("standard"))
        }

        async fn ensure_ready(&self) -> Result<(), String> {
            Ok(())
        }

        async fn synthesize(&self, _req: engine::SynthesisRequest<'_>) -> Result<Vec<u8>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut wav = b"RIFF".to_vec();
            wav.extend_from_slice(&[0u8; 4096]);
            Ok(wav)
        }
    }

    /// Die Naht trägt: die Satz-Pipeline läuft komplett über eine fremde
    /// Engine — ohne dass irgendwo ein Server erreichbar wäre.
    #[tokio::test]
    async fn eine_mock_engine_traegt_die_synthese_ohne_jeden_server() {
        // Port 1: kein Server. Was hier spielt, kam durch die Engine-Naht.
        let core = TtsCore::for_test(1);
        let mock = Arc::new(MockEngine {
            calls: AtomicUsize::new(0),
        });
        *core.engine.lock().unwrap() = EngineImpl::Mock(Arc::clone(&mock));

        let played = core
            .speak_core("Der erste Satz ist lang genug. Der zweite Satz ist es ebenfalls.")
            .await
            .unwrap();

        assert!(played > 2 * 1024, "beide WAVs kamen beim Player an");
        assert_eq!(mock.calls.load(Ordering::SeqCst), 2, "ein Aufruf pro Satz");
        assert_eq!(core.phase(), TtsPhase::Ready);
    }

    /// Der Satz-Cache greift auch vor einer Mock-Engine — und sein Tag
    /// trennt die Engines: ein Mock-Treffer ist für Fish unsichtbar.
    #[tokio::test]
    async fn der_cache_trennt_die_engines_ueber_den_tag() {
        let core = TtsCore::for_test(1);
        let mock = Arc::new(MockEngine {
            calls: AtomicUsize::new(0),
        });
        *core.engine.lock().unwrap() = EngineImpl::Mock(Arc::clone(&mock));
        let text = "Dieser Satz wird nur ein einziges Mal synthetisiert.";

        core.speak_core(text).await.unwrap();
        core.speak_core(text).await.unwrap();
        assert_eq!(
            mock.calls.load(Ordering::SeqCst),
            1,
            "der zweite Lauf kommt aus dem Cache"
        );
        assert!(core.has_cached(text));

        *core.engine.lock().unwrap() = EngineImpl::Fish;
        assert!(
            !core.has_cached(text),
            "der Mock-Eintrag liegt unter seinem Engine-Tag — Fish darf ihn nicht sehen"
        );
    }

    /// Der Kern startet mit Fish (samt Legacy-Cache-Tag) — und eine
    /// Piper-Wahl BLEIBT Piper, auch wenn Binary/Stimme fehlen: kein
    /// stiller Rückfall auf die GPU-Engine mehr (Review-Befund zu A3/E1).
    #[test]
    fn die_fish_engine_ist_standard_und_eine_piper_wahl_bleibt_piper() {
        let core = TtsCore::for_test(1);
        assert_eq!(core.engine_kind(), TtsEngineKind::Fish);
        assert!(core.engine_caps().needs_gpu);
        assert_eq!(
            core.engine_cache_tag(Some("patrick")),
            "",
            "Fish trägt den leeren Legacy-Tag"
        );
        core.set_engine(
            TtsEngineKind::Piper,
            Some(piper::PiperEngine::resolve(None, Some("eva"))),
        );
        assert_eq!(core.engine_kind(), TtsEngineKind::Piper);
        assert!(!core.engine_caps().needs_gpu, "Piper ist die CPU-Engine");
        assert_eq!(
            core.engine_cache_tag(Some("patrick")),
            "piper/eva",
            "der Tag trägt die Piper-Stimme, nicht die Fish-Referenz"
        );
        core.set_engine(TtsEngineKind::Fish, None);
        assert_eq!(core.engine_kind(), TtsEngineKind::Fish, "und zurück");
    }

    /// Eine gewählte, aber nicht einsatzbereite Piper-Engine meldet ihre
    /// konstante Fehler-ID — statt heimlich über Fish zu synthetisieren
    /// (Port 1: ein Fish-Versuch ergäbe einen Verbindungsfehler, keinen
    /// Piper-Text).
    #[tokio::test]
    async fn eine_unaufgeloeste_piper_engine_faellt_beim_sprechen_nicht_auf_fish_zurueck() {
        let core = TtsCore::for_test(1);
        core.set_engine(
            TtsEngineKind::Piper,
            Some(piper::PiperEngine::resolve(None, None)),
        );
        let err = core
            .speak_core("Dieser Satz verlangt die Piper-Engine.")
            .await
            .unwrap_err();
        assert_eq!(err, piper::ERR_BINARY_MISSING);
    }

    #[test]
    fn tiefe_streckt_die_kandidaten_wav_und_laesst_sie_eine_wav_bleiben() {
        // Ein kleines, gueltiges WAV bauen und durch die Tiefe schicken.
        // Ueber 1 KiB Nutzlast, weil `protocol::looks_like_wav` genau das
        // als Untergrenze verlangt (RIFF-Magic allein reicht ihm nicht).
        let wav = super::test_support::sine_wav(16_000, 2_000);
        let tiefer = super::apply_depth(&wav, 1.15).expect("Tiefe muss rechnen");
        assert!(protocol::looks_like_wav(&tiefer), "bleibt ein WAV");
        assert!(
            tiefer.len() > wav.len(),
            "gestreckt heisst mehr Bytes: {} vs {}",
            tiefer.len(),
            wav.len()
        );
    }

    #[test]
    fn tiefe_eins_gibt_die_bytes_unveraendert_zurueck() {
        let wav = super::test_support::sine_wav(16_000, 2_000);
        assert_eq!(super::apply_depth(&wav, 1.0).unwrap(), wav);
    }

    // ---- Dauerhafte Klangregler je Stimme (`VoiceMeta::sound`) -----------

    fn test_core() -> TtsCore {
        TtsCore::new(Arc::new(player::CountingPlayer(std::sync::Mutex::new(0))))
    }

    fn sound_for(core: &TtsCore, voice: &str, speed: f32, gain_db: f32) {
        core.voice_sounds
            .lock()
            .unwrap()
            .insert(voice.to_string(), registry::VoiceSound { speed, gain_db });
    }

    /// Der Regler skaliert die angegebene Abtastrate — Daten bleiben Byte fuer
    /// Byte stehen, damit nichts durch eine Interpolation muss.
    #[test]
    fn scale_wav_rate_skaliert_kopf_und_laesst_die_daten_in_ruhe() {
        let wav = super::test_support::sine_wav(16_000, 500);
        let schneller = super::scale_wav_rate(wav.clone(), 1.25);
        assert_eq!(schneller.len(), wav.len());
        let reader = hound::WavReader::new(std::io::Cursor::new(schneller.as_slice())).unwrap();
        assert_eq!(reader.spec().sample_rate, 20_000);
        // byte_rate zieht mit (16 Bit mono: 2 Bytes je Sample).
        assert_eq!(
            u32::from_le_bytes(schneller[28..32].try_into().unwrap()),
            40_000
        );
        assert_eq!(&schneller[44..], &wav[44..], "Audiodaten veraendert");
    }

    #[test]
    fn scale_wav_rate_laesst_unveraendert_was_es_nicht_anfassen_darf() {
        let wav = super::test_support::sine_wav(24_000, 100);
        assert_eq!(super::scale_wav_rate(wav.clone(), 1.0), wav);
        assert_eq!(super::scale_wav_rate(wav.clone(), f32::NAN), wav);
        // Kein RIFF/WAVE: unveraendert statt kaputt.
        let kein_wav = b"nicht einmal ein header".to_vec();
        assert_eq!(super::scale_wav_rate(kein_wav.clone(), 1.5), kein_wav);
    }

    /// Das gehoerte Tempo ist das Produkt aus Nutzerregler und Stimmenregler:
    /// der Stimmenregler steckt in der Abtastrate, der Nutzerregler bleibt im
    /// Player. Hier wird die eine Haelfte geprueft — dass sie den
    /// Nutzerregler NICHT anfasst, ist die andere.
    #[test]
    fn stimmen_tempo_wirkt_neben_dem_nutzerregler_nicht_an_seiner_stelle() {
        let core = test_core();
        core.controls.set_speed(1.5);
        sound_for(&core, "pyrion", 1.2, 0.0);
        assert!((core.voice_speed(Some("pyrion")) - 1.2).abs() < 1e-6);
        assert_eq!(core.voice_speed(Some("olga")), 1.0);
        assert_eq!(
            core.controls.speed(),
            1.5,
            "der Stimmenregler darf den Nutzerregler nicht ueberschreiben"
        );
    }

    #[test]
    fn stimmen_tempo_gilt_auch_fuer_die_eingestellte_stimme_ohne_expliziten_namen() {
        let core = test_core();
        *core.voice.lock().unwrap() = Some("pyrion".to_string());
        sound_for(&core, "pyrion", 0.8, 0.0);
        assert!((core.voice_speed(None) - 0.8).abs() < 1e-6);
    }

    /// 4 s bei 16 kHz; geschnitten auf 1 s bleibt ein Viertel uebrig.
    #[test]
    fn zuschnitt_kuerzt_wirklich_und_bleibt_ein_wav() {
        let wav = super::test_support::sine_wav(16_000, 64_000);
        let kurz = trim_wav_bytes(&wav, 1.0, 2.0).unwrap();
        assert!(kurz.len() < wav.len(), "{} vs {}", kurz.len(), wav.len());
        assert!(protocol::looks_like_wav(&kurz), "bleibt ein WAV");
        let (mono, rate, _) = decode_wav(&kurz).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(mono.len(), 16_000, "genau eine Sekunde");
    }

    #[test]
    fn zuschnitt_ohne_grenzen_nimmt_die_ganze_datei() {
        let wav = super::test_support::sine_wav(16_000, 32_000);
        let ganz = trim_wav_bytes(&wav, 0.0, 0.0).unwrap();
        assert_eq!(decode_wav(&ganz).unwrap().0.len(), 32_000);
    }

    /// 60 s Material, 40 s gewuenscht — beides ueber der Grenze fuer eine
    /// Zero-Shot-Referenz, beides landet bei 30 s ab Startpunkt.
    #[test]
    fn zuschnitt_ueber_dreissig_sekunden_wird_gekappt() {
        let wav = super::test_support::sine_wav(16_000, 60 * 16_000);
        let gekappt = trim_wav_bytes(&wav, 5.0, 45.0).unwrap();
        assert_eq!(decode_wav(&gekappt).unwrap().0.len(), 30 * 16_000);
        let ganz = trim_wav_bytes(&wav, 0.0, 0.0).unwrap();
        assert_eq!(
            decode_wav(&ganz).unwrap().0.len(),
            30 * 16_000,
            "auch die ganze Datei wird gekappt"
        );
    }

    /// Ein Ende hinter dem Dateiende darf nicht ueber den Rand greifen.
    #[test]
    fn zuschnitt_haelt_sich_an_die_dateilaenge() {
        let wav = super::test_support::sine_wav(16_000, 16_000);
        let kurz = trim_wav_bytes(&wav, 0.5, 99.0).unwrap();
        assert_eq!(decode_wav(&kurz).unwrap().0.len(), 8_000);
        let leer = trim_wav_bytes(&wav, 99.0, 100.0).unwrap();
        assert_eq!(decode_wav(&leer).unwrap().0.len(), 0);
    }

    /// Ohne Normalisierung ist der Dauerregler der einzige Faktor — er ist
    /// eine Eigenschaft der Stimme, keine Messung, und faellt deshalb nicht
    /// mit der Normalisierung weg.
    #[test]
    fn gain_db_wirkt_auch_bei_abgeschalteter_normalisierung() {
        let core = test_core();
        core.normalize.store(false, Ordering::Release);
        let wav = super::test_support::sine_wav(16_000, 4_000);
        assert_eq!(core.playback_gain(Some("pyrion"), &wav), 1.0);
        sound_for(&core, "pyrion", 1.0, 6.0);
        let gain = core.playback_gain(Some("pyrion"), &wav);
        assert!((gain - 1.995).abs() < 0.01, "Faktor {gain} statt ~2,0");
        // Andere Stimmen bleiben unberuehrt.
        assert_eq!(core.playback_gain(Some("olga"), &wav), 1.0);
    }

    /// Die Spitze hat das letzte Wort — auch gegen den Dauerregler.
    #[test]
    fn gain_db_wird_von_der_aussteuerungsgrenze_gedeckelt() {
        let core = test_core();
        core.normalize.store(false, Ordering::Release);
        sound_for(&core, "pyrion", 1.0, 12.0);
        let wav = super::test_support::sine_wav(16_000, 4_000);
        let peak = decode_wav(&wav).unwrap().2;
        let gain = core.playback_gain(Some("pyrion"), &wav);
        assert!(
            gain * peak <= loudness::PEAK_CEILING + 1e-6,
            "Spitze {} ueber der Grenze",
            gain * peak
        );
        assert!(gain < 3.981, "die Grenze haette greifen muessen: {gain}");
    }

    /// Mit Normalisierung liegt der Dauerregler OBEN DRAUF: derselbe Satz
    /// wird um genau den eingestellten Faktor lauter.
    #[test]
    fn gain_db_multipliziert_den_gemessenen_ausgleich() {
        let wav = super::test_support::sine_wav(16_000, 4_000);
        let ohne = test_core().playback_gain(Some("pyrion"), &wav);
        let core = test_core();
        sound_for(&core, "pyrion", 1.0, -6.0);
        let mit = core.playback_gain(Some("pyrion"), &wav);
        assert!((mit / ohne - 0.501).abs() < 0.01, "{mit} vs {ohne}");
    }
}
