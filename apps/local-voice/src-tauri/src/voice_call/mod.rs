//! Sprachgespräch mit der KI (Echtzeit, wie ein Telefonat).
//!
//! Die Gesprächsführung (Zuhören, Satzende erkennen, Unterbrechen, Wiedergabe)
//! liegt im Frontend (`src/components/voice-call/`): nur dort läuft die
//! Echo-Unterdrückung des Browsers, und sie braucht die Wiedergabe im selben
//! Audiokontext wie das Mikrofon. Dieses Modul stellt die vier Bausteine, die
//! nur der Kern kann:
//!
//! * `voice_call_start`: Sprach-, Stimm- und Sprachmodell vorwärmen, damit die
//!   erste Antwort nicht auf einen Modellstart wartet.
//! * `voice_call_transcribe`: ein Stück Sprache (16 kHz, mono, PCM16) erkennen.
//! * `voice_call_chat`: Antwort des aktiven Sprachmodells als Strom, schon
//!   beim Eintreffen in sprechbare Häppchen zerlegt (`sentences`).
//! * `turn::voice_call_turn`: ein Durchgang eines Modells als roher Textstrom für den
//!   Orchestrator (Stufe 2), wahlweise mit einem zweiten Modell (Denker).
//! * `voice_call_synthesize`: ein Häppchen mit der eingestellten Stimme
//!   (Standard: Anton) zu WAV.
//!
//! Audio geht als Base64 über die Schnittstelle: das passt zur gewöhnlichen
//! Command-Schicht (specta) und ist für Gesprächslängen von Sekunden klein genug.
//! Texte und Audio stehen nie im Log.

pub mod assistenten;
pub mod projekt;
pub mod sentences;
pub mod turn;

#[cfg(test)]
mod projekt_quelltests;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::assistent;
use crate::assistent::tor::{self, Ausgabetor};
use crate::llm_client::{self, StreamMessage};
use crate::managers::transcription::TranscriptionManager;
use crate::managers::tts::TtsManager;
use crate::managers::usage::{self, CapturedCall, Purpose};

/// Name des Ereignisses, über das Antwortstücke zum Frontend laufen.
pub const EVENT: &str = "voice-call";

/// Hoechstens so viele Zeichen Denktext je Anfrage gehen an die Oberflaeche.
pub const DENKEN_MAX_ZEICHEN: usize = 16 * 1024;

/// Denktext des Modells als Ereignis `{kind: "reasoning", request_id, text}` (lva#485): nur fuer
/// die Denk-Anzeige, nie gesprochen, nie Antwort oder Verlauf. Keine Senke bei Notizbuchdaten
/// (`geschuetzt`): dort verlaesst vor dem Vermerk nichts Rust.
pub(crate) fn denksenke(
    app: &AppHandle,
    request_id: &str,
    geschuetzt: bool,
) -> Option<llm_client::DenkSenke> {
    if geschuetzt {
        return None;
    }
    let (app, request_id) = (app.clone(), request_id.to_string());
    let gesendet = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Some(Arc::new(move |text: &str| {
        let n = text.chars().count();
        let vorher = gesendet.fetch_add(n, std::sync::atomic::Ordering::SeqCst);
        if vorher >= DENKEN_MAX_ZEICHEN {
            return;
        }
        let text: String = text.chars().take(DENKEN_MAX_ZEICHEN - vorher).collect();
        let _ = app.emit(
            EVENT,
            serde_json::json!({"kind": "reasoning", "request_id": request_id, "text": text}),
        );
    }))
}

static CANCELS: Lazy<Mutex<HashMap<String, Arc<Notify>>>> = Lazy::new(Default::default);
/// Abbrüche, die vor dem Start ihrer Anfrage eintrafen (IPC-Reihenfolge ist nicht garantiert).
/// Ein Eintrag verfällt nach `EARLY_TTL`, höchstens `EARLY_MAX` liegen zugleich.
static EARLY_CANCELS: Lazy<Mutex<Vorgemerkt>> = Lazy::new(Default::default);
/// Laufende Sprachsynthesen je Anfrage (Abbruchflags).
///
/// Sperr-Reihenfolge: `SYNTH_CANCELS` vor `EARLY_CANCELS`, nie umgekehrt. Anmelden einer Synthese
/// (`synth_anmelden`) und Abbrechen (`cancel_synth`) arbeiten beide unter dieser einen Sperre, sonst
/// könnte ein Abbruch zwischen „noch nicht angemeldet“ und „vorgemerkt“ an der Anmeldung vorbeigehen.
static SYNTH_CANCELS: Lazy<Mutex<HashMap<String, Vec<Arc<AtomicBool>>>>> =
    Lazy::new(Default::default);

/// So lange gilt ein vorgemerkter Abbruch; danach gehört er zu keiner Anfrage mehr.
const EARLY_TTL: Duration = Duration::from_secs(30);
/// So viele vorgemerkte Abbrüche liegen höchstens zugleich (ein weiterer wird nicht gemerkt).
const EARLY_MAX: usize = 64;

/// Vorgemerkte Abbrüche mit Zeitstempel: Einträge, die niemand einlöst (ein Abbruch nach dem Ende der
/// Anfrage), füllen den Vorrat nicht dauerhaft. Die Zeit wird den `*_um`-Methoden mitgegeben (Tests).
#[derive(Default)]
struct Vorgemerkt(HashMap<String, Instant>);

impl Vorgemerkt {
    /// Merkt den Abbruch; `false`, wenn der Vorrat voll ist. Abgelaufene Einträge fallen vorher weg.
    fn merke_um(&mut self, id: &str, jetzt: Instant) -> bool {
        self.0
            .retain(|_, seit| jetzt.saturating_duration_since(*seit) < EARLY_TTL);
        if self.0.len() >= EARLY_MAX && !self.0.contains_key(id) {
            return false;
        }
        self.0.insert(id.to_string(), jetzt);
        true
    }

    /// Löst den Abbruch ein und entfernt ihn; ein abgelaufener gilt nicht mehr.
    fn nimm_um(&mut self, id: &str, jetzt: Instant) -> bool {
        self.0
            .remove(id)
            .is_some_and(|seit| jetzt.saturating_duration_since(seit) < EARLY_TTL)
    }

    fn merke(&mut self, id: &str) -> bool {
        self.merke_um(id, Instant::now())
    }

    /// Wie bei der früheren Menge: `true`, wenn ein (gültiger) Abbruch vorlag; er ist danach weg.
    fn remove(&mut self, id: &str) -> bool {
        self.nimm_um(id, Instant::now())
    }
}
/// Laufende Antworten mit App-Kontext: ein Widerruf bricht sie sofort ab.
/// Bleiben bis zum Abbruch bzw. bis sie verdrängt werden eingetragen (die
/// Wiedergabe läuft über das Textende hinaus); höchstens `CONTEXT_KEEP`.
static CONTEXT_REQUESTS: Lazy<Mutex<std::collections::VecDeque<String>>> =
    Lazy::new(Default::default);
const CONTEXT_KEEP: usize = 64;

fn mark_context(id: &str) {
    let mut q = CONTEXT_REQUESTS.lock().unwrap_or_else(|e| e.into_inner());
    if !q.iter().any(|x| x == id) {
        q.push_back(id.to_string());
        while q.len() > CONTEXT_KEEP {
            q.pop_front();
        }
    }
}

fn unmark_context(id: &str) {
    CONTEXT_REQUESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|x| x != id);
}

/// Abbrüche registrierter Anfragen (zusätzlich zum Notify): so lässt sich
/// nach der Quellenprüfung sicher fragen, ob inzwischen abgebrochen wurde.
static CANCELLED: Lazy<Mutex<std::collections::HashSet<String>>> = Lazy::new(Default::default);

/// Wurde die Anfrage abgebrochen (vor oder nach ihrer Registrierung)?
fn was_cancelled(id: &str) -> bool {
    let late = CANCELLED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id);
    take_early_cancel(id) || late
}

/// Bindet den Kontext und prüft danach, ob die Anfrage währenddessen
/// abgebrochen oder widerrufen wurde (Codex M5 Must-fix 2). `Err`: abbrechen,
/// nichts senden.
async fn bind_then_check<T, F>(request_id: &str, bind: F) -> Result<Option<T>, ()>
where
    F: std::future::Future<Output = Option<T>>,
{
    let context = bind.await;
    if was_cancelled(request_id) {
        return Err(());
    }
    Ok(context)
}

/// Ein Abbruch, der vor der Registrierung kam, einlösen.
fn take_early_cancel(id: &str) -> bool {
    EARLY_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id)
}

/// Abgeleitete Verlaufseinträge nie an einen entfernten Empfänger (Routen-
/// wechsel zur Cloud: sie fallen weg, bevor irgendetwas gesendet wird).
pub(crate) fn for_receiver(messages: Vec<VoiceCallMessage>, local: bool) -> Vec<VoiceCallMessage> {
    messages
        .into_iter()
        .filter(|m| local || !m.derived)
        .collect()
}

/// Braucht dieser Aufruf die gebundene lokale Route? Ja, sobald App-Kontext
/// oder ein daraus abgeleiteter Verlaufseintrag mitgeht (Codex M5/M6).
pub(crate) fn needs_pinned(
    bound_context: bool,
    messages: &[VoiceCallMessage],
    local: bool,
) -> bool {
    bound_context || (local && messages.iter().any(|m| m.derived))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VoiceCallReady {
    /// Anzeigename des Sprachmodells (`Anbieter · Modell`), leer wenn keins eingerichtet ist.
    pub llm: String,
    /// Lokales Sprachmodell (kostenlos, aber CPU/GPU-lastig).
    pub llm_local: bool,
    /// Hinweise, die das Gespräch verlangsamen würden (nichts Hartes).
    pub warnings: Vec<String>,
    /// Dauer des Vorwärmens in Millisekunden.
    pub warmup_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VoiceCallMessage {
    /// `system` | `user` | `assistant`
    pub role: String,
    pub content: String,
    /// Aus App-Kontext entstanden (Frage oder Antwort eines Schritts mit
    /// Auszügen): geht nur an einen lokalen Empfänger (Codex M6).
    #[serde(default)]
    pub derived: bool,
}

fn pcm16_b64_to_f32(b64: &str) -> Result<Vec<f32>, String> {
    let bytes = B64
        .decode(b64)
        .map_err(|e| format!("Audio ungültig: {e}"))?;
    if bytes.len() % 2 != 0 {
        return Err("Audio ungültig: ungerade Bytezahl".into());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}

/// Verbrauch der erfassten Modellaufrufe für das `done`-Ereignis: Summe der
/// Token, die Ereignisnummern des Ledgers als Liste (`usage_event_ids`; die
/// erste zusätzlich als `usage_event_id`), Anbieter und ob er lokal läuft.
fn usage_payload(calls: &[CapturedCall]) -> serde_json::Value {
    let ids: Vec<i64> = calls.iter().filter_map(|c| c.usage_event_id).collect();
    serde_json::json!({
        "prompt_tokens": calls.iter().map(|c| c.prompt_tokens).sum::<u64>(),
        "completion_tokens": calls.iter().map(|c| c.completion_tokens).sum::<u64>(),
        "usage_event_id": ids.first(),
        "usage_event_ids": ids,
        "provider_id": calls.last().map(|c| c.provider_id.clone()),
        "local": calls.last().map(|c| c.local).unwrap_or(false),
    })
}

/// Wärmt STT, TTS und LLM vor und meldet, was das Gespräch verlangsamen würde.
#[tauri::command]
#[specta::specta]
pub async fn voice_call_start(app: AppHandle) -> Result<VoiceCallReady, String> {
    let started = Instant::now();
    // Der Sprecher des Gespraechs: eigenes Modell (Modelle je Funktion), sonst das Standardmodell.
    let settings =
        crate::settings::get_settings(&app).for_function(crate::settings::FN_VOICE_SPEAKER);
    let mut warnings = Vec::new();

    let mut speaker_warm = None;
    let (llm, llm_local) = match crate::managers::meetings::llm_call::resolve_provider(&settings) {
        Ok((provider, model, _)) => {
            let target = warm_target(&provider, &model);
            let local = target.is_some() || crate::managers::llm::is_local(&provider);
            // Lokaler Sprecher: parallel zu STT und Stimme laden, damit der erste
            // Satz nicht auf den Modellstart wartet (Codex E2).
            speaker_warm = target.map(|t| tokio::spawn(warm_speaker(t)));
            (format!("{} · {}", provider.label, model), local)
        }
        Err(e) => {
            warnings.push(e);
            (String::new(), false)
        }
    };

    if settings.model_unload_timeout == crate::settings::ModelUnloadTimeout::Immediately {
        warnings.push(
            "Das Spracherkennungsmodell wird nach jeder Eingabe entladen (Einstellungen → Eingabe). Das kostet im Gespräch jedes Mal Sekunden; „Nie“ oder „2 Minuten“ wählen."
                .into(),
        );
    }

    // Scheitert STT oder Stimme, braucht es auch den Sprecher nicht mehr.
    let abort_warm = |task: &Option<tokio::task::JoinHandle<Result<(), String>>>| {
        if let Some(t) = task {
            t.abort();
        }
    };
    let stt = app.state::<Arc<TranscriptionManager>>().inner().clone();
    let stt_model = settings.selected_model.clone();
    if !load_stt(stt.clone(), stt_model.clone()).await {
        abort_warm(&speaker_warm);
        return Err("Das Spracherkennungsmodell konnte nicht geladen werden (Einstellungen → Eingabe → Modell).".into());
    }

    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.refresh_from_settings();
    if let Err(e) = tts.ensure_engine_ready().await {
        abort_warm(&speaker_warm);
        return Err(format!("Die Stimme ist nicht bereit: {e}"));
    }

    if let Some(task) = speaker_warm {
        // Höchstens LLM_WARM_TIMEOUT warten; ein langsamer Start ist ein Hinweis,
        // kein Fehler, und es gibt keinen Ausweich auf ein Cloud-Modell.
        match tokio::time::timeout(LLM_WARM_TIMEOUT, task).await {
            Ok(Ok(Ok(()))) => {
                // Der Speicherschutz des lokalen Servers darf ein gerade untätiges
                // Erkennungsmodell entladen haben: dann jetzt neu laden statt beim
                // ersten Satz.
                if stt.get_current_model().as_deref() != Some(stt_model.as_str())
                    && !load_stt(stt.clone(), stt_model.clone()).await
                {
                    warnings.push(
                        "Das Spracherkennungsmodell wurde wegen Speichermangel entladen; der erste Satz lädt es neu."
                            .into(),
                    );
                }
            }
            Ok(Ok(Err(e))) => warnings.push(format!(
                "Das lokale Sprachmodell ist noch nicht bereit: {e}. Die erste Antwort kann dauern."
            )),
            Ok(Err(_)) | Err(_) => warnings.push(
                "Das lokale Sprachmodell lädt noch. Die erste Antwort kann einige Sekunden dauern."
                    .into(),
            ),
        }
    }

    Ok(VoiceCallReady {
        llm,
        llm_local,
        warnings,
        warmup_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

/// Höchstdauer, die der Gesprächsstart auf das Vorwärmen des lokalen Sprechers
/// wartet. Länger lädt das Modell im Hintergrund weiter.
const LLM_WARM_TIMEOUT: Duration = Duration::from_secs(45);
/// So lange hält Ollama den Sprecher nach dem Vorwärmen im Speicher. Jede
/// Antwort über den kompatiblen Pfad setzt die Frist auf Ollamas Standard
/// (5 min) zurück; danach gibt Ollama ihn selbst frei.
const OLLAMA_KEEP_ALIVE: &str = "15m";

/// Lädt das Erkennungsmodell (höchstens 90 s) und sagt, ob es danach bereitsteht.
async fn load_stt(stt: Arc<TranscriptionManager>, model: String) -> bool {
    tokio::task::spawn_blocking(move || {
        stt.initiate_model_load();
        stt.wait_until_loaded(&model, Duration::from_secs(90))
    })
    .await
    .unwrap_or(false)
}

/// Was beim Gesprächsstart vorzuwärmen ist.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WarmTarget {
    /// Der eingebaute lokale Server (llama.cpp) mit diesem Modell.
    BuiltIn(String),
    /// Ein Ollama-Dienst auf diesem Rechner: `/api/generate` ohne Prompt lädt das Modell.
    Ollama { url: String, model: String },
}

/// Rein: Ziel des Vorwärmens; `None` für Cloud-, Abo- und fremde Dienste.
fn warm_target(provider: &crate::settings::PostProcessProvider, model: &str) -> Option<WarmTarget> {
    if crate::managers::llm::is_local(provider) {
        return Some(WarmTarget::BuiltIn(model.to_string()));
    }
    let base = provider.base_url.trim_end_matches('/');
    let is_ollama = provider.id == "ollama" || base.contains(":11434");
    if !is_ollama {
        return None;
    }
    let chat = llm_client::ollama_native_url(base)?;
    Some(WarmTarget::Ollama {
        url: chat.replace("/api/chat", "/api/generate"),
        model: model.to_string(),
    })
}

/// Empfänger des App-Kontexts ist lokal: nur das eingebaute Modell. Ein
/// Endpunkt auf localhost (Ollama, Proxy, Relay) zählt nicht, weil er weiter
/// ins Netz leiten kann (Codex M5 Must-fix 1). Der Versand selbst läuft über
/// `send_chat_completion_stream_routed(.., pinned_local)` ohne Proxy/Umleitung.
fn receiver_is_local(provider: &crate::settings::PostProcessProvider) -> bool {
    crate::managers::llm::is_local(provider)
}

/// Kontext in die letzte Nutzernachricht einbetten (vor die Frage): keine
/// Systemnachricht mitten im Verlauf, die manche Chat-Vorlagen lokaler
/// Modelle ablehnen; der vordere Teil bleibt für den Prompt-Cache gleich (L3).
fn with_context_message(
    mut messages: Vec<VoiceCallMessage>,
    context: Option<String>,
) -> Vec<VoiceCallMessage> {
    let Some(context) = context else {
        return messages;
    };
    match messages.iter_mut().rev().find(|m| m.role == "user") {
        Some(last) => last.content = format!("{context}\n\n{}", last.content),
        None => messages.push(VoiceCallMessage {
            role: "user".into(),
            content: context,
            derived: true,
        }),
    }
    messages
}

/// Alle Anfragen mit Kontext abbrechen: Modell und Sprachsynthese, auch wenn
/// der Text schon fertig ist und nur noch gesprochen wird. Liefert ihre IDs.
fn revoke_ids() -> Vec<String> {
    let ids: Vec<String> = CONTEXT_REQUESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect();
    for id in &ids {
        voice_call_cancel(id.clone());
    }
    ids
}

/// Freigaben entzogen: alle laufenden Antworten mit App-Kontext abbrechen und
/// der Oberfläche melden, damit sie auch die Sprachausgabe stoppt (Auflagen 9/10).
pub fn revoke_context_requests(app: &AppHandle) {
    let ids = revoke_ids();
    // Auch Antworten, deren Text fertig ist, aber noch gesprochen wird:
    // die Oberfläche stoppt bei `*` jede Wiedergabe mit Kontext (Auflage 10).
    let _ = app.emit(
        EVENT,
        serde_json::json!({"kind": "revoked", "request_id": "*"}),
    );
    for id in ids {
        let _ = app.emit(
            EVENT,
            serde_json::json!({"kind": "revoked", "request_id": id}),
        );
    }
}

/// Widerruf einzelner Projektläufe: bricht **genau diese** Anfragen ab und meldet der Oberfläche je
/// Anfrage `revoked` (sie nimmt den Text weg und stoppt die Sprachausgabe). Alle übrigen Antworten mit
/// App-Kontext laufen weiter: Eine Änderung an einem Projekt betrifft nur dessen Läufe. `ids` nennt
/// `ProjektTor::widerrufen`.
pub fn revoke_requests(app: &AppHandle, ids: &[String]) {
    let betroffen = revoke_only(ids);
    for id in betroffen {
        let _ = app.emit(
            EVENT,
            serde_json::json!({"kind": "revoked", "request_id": id}),
        );
    }
}

/// Bricht genau die genannten Anfragen ab und nimmt sie aus der Liste der Anfragen mit Kontext;
/// andere bleiben unberührt. Liefert die Kennungen, sortiert und ohne Doppelte.
fn revoke_only(ids: &[String]) -> Vec<String> {
    let mut betroffen: Vec<String> = ids.to_vec();
    betroffen.sort();
    betroffen.dedup();
    CONTEXT_REQUESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|x| !betroffen.contains(x));
    for id in &betroffen {
        voice_call_cancel(id.clone());
    }
    betroffen
}

/// Lädt den Sprecher. Der eingebaute Server startet über `ensure_local` (mit
/// Speicherschutz); Ollama lädt das Modell bei einem leeren Aufruf.
async fn warm_speaker(target: WarmTarget) -> Result<(), String> {
    match target {
        WarmTarget::BuiltIn(model) => crate::managers::llm::ensure_local(&model).await.map(|_| ()),
        WarmTarget::Ollama { url, model } => {
            let body = serde_json::json!({ "model": model, "keep_alive": OLLAMA_KEEP_ALIVE });
            let resp = reqwest::Client::new()
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Ollama nicht erreichbar ({e})"))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("Ollama antwortet {}", resp.status()))
            }
        }
    }
}

/// Erkennt ein Stück Sprache. `pcm16_b64` = 16 kHz, mono, 16 Bit little endian.
#[tauri::command]
#[specta::specta]
pub async fn voice_call_transcribe(app: AppHandle, pcm16_b64: String) -> Result<String, String> {
    let samples = pcm16_b64_to_f32(&pcm16_b64)?;
    let stt = app.state::<Arc<TranscriptionManager>>().inner().clone();
    tokio::task::spawn_blocking(move || stt.transcribe(samples).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
        .map(|t| t.trim().to_string())
}

/// Antwort des aktiven Sprachmodells als Strom. Jedes Häppchen kommt sofort als
/// Ereignis `voice-call` (`{kind: "sentence", request_id, index, text}`), die
/// fertige Antwort ist der Rückgabewert. Bricht `voice_call_cancel` ab, endet
/// der Aufruf mit `Ok` und leerem Text.
#[tauri::command]
#[specta::specta]
pub async fn voice_call_chat(
    app: AppHandle,
    request_id: String,
    messages: Vec<VoiceCallMessage>,
    context: Option<crate::app_context::model::SpeakerContext>,
    conversation_id: Option<String>,
    expected_stand: Option<String>,
) -> Result<String, String> {
    // Ein vorgezogener Abbruch wird zuerst eingelöst, damit er auch bei einem Fehler unten
    // nicht ewig liegen bleibt.
    if EARLY_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id)
    {
        return Ok(String::new());
    }
    // Projektgespräch (lva#365 P4c-2): ausschließlich über das Versandtor. Der Zweig steht VOR dem
    // Notizbuchschutz und lässt ihn unberührt; ein mitgegebener App-Kontext wird nicht gelesen. Ist die
    // Bindung des Gesprächs nicht sicher zu lesen, geht nichts hinaus.
    if let Some(projekt) = projekt::bindung(&app, conversation_id.as_deref()).await? {
        let gespraech = conversation_id.unwrap_or_default();
        return projekt::chat(
            &app,
            request_id,
            messages,
            projekt,
            gespraech,
            expected_stand,
        )
        .await;
    }
    // ADR-0028 E4: Einstufung in Rust vor jedem Modellaufruf. S0, Fehler, leerer oder zu langer
    // Text: kein Netzaufruf, unabhängig vom Schalter der Oberfläche.
    let sprache = assistent::sprache_der_app(&app);
    // Notizbuchdaten (Codex lva#414): auch der alte Weg ersetzt entzogene Inhalte und schickt
    // geschützte nur an einen lokalen Empfänger.
    let pruefung =
        crate::commands::voice_tools::schutz_pruefen(&app, messages, conversation_id.as_deref());
    if pruefung.gesperrt {
        return Err(assistent::schutz::UNGESICHERT.to_string());
    }
    let messages = pruefung.nachrichten.clone();
    tor::vor_nachrichten(&messages, sprache, conversation_id.as_deref())?;
    let settings =
        crate::settings::get_settings(&app).for_function(crate::settings::FN_VOICE_SPEAKER);
    let (provider, model, api_key) =
        crate::managers::meetings::llm_call::resolve_provider(&settings)?;
    // App-Kontext nur an einen lokalen Empfänger, unmittelbar vor dem Start
    // geprüft (Codex M5/M6); sonst antwortet das Modell ohne Kontext.
    let local = receiver_is_local(&provider);
    if pruefung.geschuetzt && !local {
        return Err(assistent::schutz::NUR_LOKAL.to_string());
    }
    let messages = tor::mit_markerregel(for_receiver(messages, local), sprache);
    // Werkzeugrolle (Codex lva#414 M5) auch im alten Weg: Ergebnisse als Datenbloecke.
    let messages = assistent::schutz::werkzeugrolle(messages, &assistent::schutz::neue_kennung());
    let guarded = needs_pinned(context.is_some(), &messages, local);
    // Erst Abbruch und Widerruf registrieren, dann prüfen: was dazwischen kommt,
    // trifft die Anfrage (der Notify merkt sich den Abbruch).
    let cancel = Arc::new(Notify::new());
    CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), cancel.clone());
    if guarded {
        mark_context(&request_id);
    }
    let unregister = |id: &str| {
        CANCELS.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        CANCELLED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        unmark_context(id);
    };
    // Das lokale Modell zuerst starten (das kann dauern), danach prüfen:
    // nichts veraltet zwischen Prüfung und Versand (Codex Must-fix 4).
    let local_base = if guarded && local {
        match crate::managers::llm::ensure_local(&model).await {
            Ok(base) => Some(base),
            Err(e) => {
                unregister(&request_id);
                return Err(e);
            }
        }
    } else {
        None
    };
    let question = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    // Prüfung liest Quellen frisch (Datei/SQLite): außerhalb des async-Threads.
    let bind = async {
        match context {
            Some(c) => {
                let (app2, rid) = (app.clone(), request_id.clone());
                tauri::async_runtime::spawn_blocking(move || {
                    crate::app_context::service::bind_for_speaker_quellen(
                        &app2, &c, local, &rid, &question,
                    )
                })
                .await
                .ok()
                .flatten()
            }
            None => None,
        }
    };
    let Ok(gebunden) = bind_then_check(&request_id, bind).await else {
        unregister(&request_id);
        return Ok(String::new());
    };
    let (context, quellen) = match gebunden {
        Some(g) => (Some(g.nachricht), g.quellen),
        None => (None, Vec::new()),
    };
    let pinned = needs_pinned(context.is_some(), &messages, local);
    // Gesendet wird an genau die vor der Prüfung gestartete Adresse.
    let pinned_local: Option<String> = if pinned { local_base.clone() } else { None };
    if pinned && pinned_local.is_none() {
        unregister(&request_id);
        return Err("Lokales Modell nicht bereit".into());
    }
    if !pinned {
        unmark_context(&request_id);
    }
    let messages = with_context_message(messages, context);
    let messages: Vec<StreamMessage> = messages
        .into_iter()
        .map(|m| StreamMessage::new(&m.role, m.content))
        .collect();

    // Belegregel (E4.2): nur geprüft freigegebene Sätze gehen an Anzeige und Sprachausgabe.
    let tor_strom = Arc::new(Mutex::new(Ausgabetor::neu(sprache, &quellen)));
    let index = Arc::new(Mutex::new(0u32));
    let emit_piece = {
        let app = app.clone();
        let request_id = request_id.clone();
        let index = index.clone();
        Arc::new(move |text: String| {
            let mut i = index.lock().unwrap_or_else(|e| e.into_inner());
            let _ = app.emit(
                EVENT,
                serde_json::json!({"kind": "sentence", "request_id": request_id, "index": *i, "text": text}),
            );
            *i += 1;
        })
    };
    // Antwort aus Notizbuchdaten: nichts verlässt Rust, bevor der Vermerk gesichert ist.
    let puffer = Arc::new(Mutex::new(assistent::schutz::Antwortpuffer::<String>::neu(
        &pruefung,
    )));
    let on_delta = {
        let tor = tor_strom.clone();
        let emit_piece = emit_piece.clone();
        let puffer = puffer.clone();
        move |delta: &str| {
            let pieces = tor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .einspeisen(delta);
            let mut p = puffer.lock().unwrap_or_else(|e| e.into_inner());
            for piece in pieces {
                if let Some(piece) = p.durchlassen(piece) {
                    emit_piece(piece);
                }
            }
        }
    };

    // Erfassungsbereich: der Aufruf bucht seinen Verbrauch, und das Gespräch
    // zeigt Token und Kosten. Gelesen wird innerhalb des Bereichs, auch nach
    // dem Abbruch (dann ist nur der Verbrauch bis dahin gebucht, falls überhaupt).
    let (result, captured) = usage::with_capture(async {
        let call = llm_client::mit_denksenke(
            denksenke(&app, &request_id, pruefung.geschuetzt),
            llm_client::send_chat_completion_stream_routed(
                Purpose::Chat,
                &provider,
                api_key,
                &model,
                messages,
                &on_delta,
                pinned_local.as_deref(),
            ),
        );
        // Abbruch zuerst prüfen: ein schon vorliegender Abbruch verhindert
        // den Versand, statt mit ihm zu wetteifern.
        let result = tokio::select! {
            biased;
            _ = cancel.notified() => None,
            r = call => Some(r),
        };
        (result, usage::captured_calls().0)
    })
    .await;
    CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id);
    CANCELLED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id);

    match result {
        // Abgebrochen: das Restliche wird verworfen, die Sprachausgabe ist schon gestoppt. Auch
        // zurückgehaltene Sätze aus Notizbuchdaten gehen nie hinaus (kein Vermerk nötig).
        None => {
            puffer.lock().unwrap_or_else(|e| e.into_inner()).verwerfen();
            Ok(String::new())
        }
        Some(Ok(_roh)) => {
            // Der Rückgabewert ist der freigegebene Text, nie die rohe Modellausgabe.
            let (rest, freigegeben) = {
                let mut t = tor_strom.lock().unwrap_or_else(|e| e.into_inner());
                let rest = t.beenden();
                (rest, t.freigegebener_text())
            };
            // Erst nach dauerhaft gesichertem Vermerk wird die Antwort freigegeben (Recheck 3/1, 4):
            // gepufferte Sätze gehen erst danach hinaus, bei Fehler werden sie verworfen.
            let gesichert = assistent::schutz::abgeleitet_merken(
                &pruefung,
                &freigegeben,
                conversation_id.as_deref(),
            )
            .is_ok();
            let hinaus = puffer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .abschliessen(gesichert, rest);
            for piece in hinaus {
                emit_piece(piece);
            }
            if !gesichert {
                let _ = app.emit(
                    EVENT,
                    serde_json::json!({"kind": "failed", "request_id": request_id}),
                );
                return Err(assistent::schutz::UNGESICHERT.to_string());
            }
            tor::ausgabe_merken(conversation_id.as_deref(), &freigegeben);
            let _ = app.emit(
                EVENT,
                serde_json::json!({
                    "kind": "done",
                    "request_id": request_id,
                    "usage": usage_payload(&captured),
                }),
            );
            Ok(freigegeben)
        }
        Some(Err(e)) => {
            puffer.lock().unwrap_or_else(|e| e.into_inner()).verwerfen();
            // Auch ein fehlgeschlagener Aufruf kann schon Token verbraucht haben.
            let _ = app.emit(
                EVENT,
                serde_json::json!({
                    "kind": "failed",
                    "request_id": request_id,
                    "usage": (!captured.is_empty()).then(|| usage_payload(&captured)),
                }),
            );
            Err(e)
        }
    }
}

/// Bricht eine laufende Antwort ab (Unterbrechen durch den Sprecher).
///
/// Ohne laufenden Durchgang (`CANCELS`) und ohne laufende Synthese wird der Abbruch vorgemerkt: er
/// gilt dem Start, der noch folgt (`voice_call_chat`, `voice_call_turn`, `voice_call_synthesize`).
#[tauri::command]
#[specta::specta]
pub fn voice_call_cancel(request_id: String) -> bool {
    unmark_context(&request_id);
    let registered = CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&request_id)
        .cloned();
    match registered {
        Some(n) => {
            cancel_synth(&request_id, false);
            CANCELLED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(request_id.clone());
            n.notify_one();
            true
        }
        None => {
            cancel_synth(&request_id, true);
            false
        }
    }
}

/// Meldet eine Synthese an ihrer Anfrage an. `true`: die Anfrage war schon abgebrochen (der vorgemerkte
/// Abbruch ist damit eingelöst und entfernt, das Flag ist gesetzt, es wird nichts eingetragen); die
/// Synthese darf dann nicht mehr starten. Prüfen und Eintragen stehen unter derselben Sperre wie das
/// Abbrechen in `cancel_synth`, ein gleichzeitiger Abbruch geht also nie an beiden vorbei.
fn synth_anmelden(request_id: &str, cancel: &Arc<AtomicBool>) -> bool {
    let mut map = SYNTH_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
    if take_early_cancel(request_id) {
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        return true;
    }
    map.entry(request_id.to_string())
        .or_default()
        .push(cancel.clone());
    false
}

/// Nimmt eine beendete Synthese wieder aus der Anmeldung.
fn synth_abmelden(request_id: &str, cancel: &Arc<AtomicBool>) {
    let mut map = SYNTH_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(list) = map.get_mut(request_id) {
        list.retain(|c| !Arc::ptr_eq(c, cancel));
        if list.is_empty() {
            map.remove(request_id);
        }
    }
}

/// Ein Häppchen mit der eingestellten Stimme zu WAV (Base64).
#[tauri::command]
#[specta::specta]
pub async fn voice_call_synthesize(
    app: AppHandle,
    text: String,
    request_id: Option<String>,
) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    let cancel = Arc::new(AtomicBool::new(false));
    // An die Anfrage gebunden: `voice_call_cancel` und ein Widerruf brechen
    // auch die Synthese ab (Codex M5 Must-fix 3). Kam der Abbruch schon vor dieser
    // Anmeldung, startet die Synthese gar nicht erst.
    if let Some(id) = &request_id {
        if synth_anmelden(id, &cancel) {
            return Err("abgebrochen".into());
        }
    }
    let wav = tts.synthesize_wav(&text, Some(&cancel)).await;
    if let Some(id) = &request_id {
        synth_abmelden(id, &cancel);
    }
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("abgebrochen".into());
    }
    Ok(B64.encode(wav?))
}

/// Laufende Synthesen einer Anfrage abbrechen. `true`: es lief mindestens eine. Läuft keine und ist
/// `vormerken` gesetzt, wird der Abbruch unter derselben Sperre vorgemerkt, mit der `synth_anmelden`
/// prüft und einträgt: eine Synthese, die gleich danach anmeldet, löst ihn ein. Eine Synthese, die
/// der Abbruch fand, hinterlässt keinen Eintrag (kein Nachzügler im Vorrat).
fn cancel_synth(request_id: &str, vormerken: bool) -> bool {
    cancel_synth_mit(request_id, vormerken, || {})
}

/// Wie `cancel_synth`; `vor_vormerken` läuft im entscheidenden Fenster: nachdem keine Synthese gefunden wurde
/// und bevor der Abbruch vorgemerkt wird, noch unter der Sperre. Die Produktion übergibt nichts; ein Test
/// schiebt hier eine gleichzeitige Anmeldung ein, die an der Sperre warten muss.
fn cancel_synth_mit(request_id: &str, vormerken: bool, vor_vormerken: impl FnOnce()) -> bool {
    let mut map = SYNTH_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(list) = map.remove(request_id) {
        for c in list {
            c.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        return true;
    }
    if vormerken {
        vor_vormerken();
        EARLY_CANCELS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .merke(request_id);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CONTEXT_REQUESTS`, `CANCELS` und `EARLY_CANCELS` sind prozessweit: `revoke_ids` leert die
    /// ganze Liste. Tests, die sie benutzen, laufen deshalb nacheinander (sonst nimmt ein paralleler
    /// Test die Kennung des anderen weg; Flake in lva#423).
    static GLOBALE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(super) fn globale() -> std::sync::MutexGuard<'static, ()> {
        GLOBALE.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn provider(id: &str, base: &str) -> crate::settings::PostProcessProvider {
        crate::settings::PostProcessProvider {
            id: id.into(),
            label: id.into(),
            base_url: base.into(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    #[test]
    fn warms_only_local_speakers() {
        assert_eq!(
            warm_target(&provider("local", "http://127.0.0.1:0/v1"), "qwen3-4b"),
            Some(WarmTarget::BuiltIn("qwen3-4b".into()))
        );
        assert_eq!(
            warm_target(
                &provider("ollama", "http://127.0.0.1:11434/v1"),
                "gemma3:4b"
            ),
            Some(WarmTarget::Ollama {
                url: "http://127.0.0.1:11434/api/generate".into(),
                model: "gemma3:4b".into()
            })
        );
        // Cloud, Abo-CLI und andere lokale Dienste (LM Studio, vLLM): nichts vorwärmen.
        assert_eq!(
            warm_target(&provider("openai", "https://api.openai.com/v1"), "gpt"),
            None
        );
        assert_eq!(
            warm_target(&provider("claude_cli", "cli://claude"), "haiku@low"),
            None
        );
        assert_eq!(
            warm_target(&provider("lmstudio", "http://localhost:1234/v1"), "m"),
            None
        );
        // Ollama auf einem anderen Rechner: kein Vorwärmen über das Netz.
        assert_eq!(
            warm_target(&provider("ollama", "http://192.168.1.5:11434/v1"), "m"),
            None
        );
    }

    #[test]
    fn decodes_pcm16() {
        let bytes: Vec<u8> = [0i16, 16384, -32768]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let v = pcm16_b64_to_f32(&B64.encode(bytes)).unwrap();
        assert_eq!(v, vec![0.0, 0.5, -1.0]);
    }

    #[test]
    fn usage_payload_sums_calls() {
        let call = |p, c, id| CapturedCall {
            purpose: "chat".into(),
            provider_id: "openai".into(),
            local: false,
            connection_label: String::new(),
            model_id: "m".into(),
            model_label: "m".into(),
            prompt_tokens: p,
            completion_tokens: c,
            duration_ms: 1,
            ok: true,
            usage_event_id: id,
        };
        let v = usage_payload(&[call(10, 2, Some(7)), call(5, 1, None)]);
        assert_eq!(v["prompt_tokens"], 15);
        assert_eq!(v["completion_tokens"], 3);
        assert_eq!(v["usage_event_ids"], serde_json::json!([7]));
        assert_eq!(v["provider_id"], "openai");
        assert_eq!(usage_payload(&[])["prompt_tokens"], 0);
    }

    #[test]
    fn rejects_odd_length() {
        assert!(pcm16_b64_to_f32(&B64.encode([1u8, 2, 3])).is_err());
    }

    #[test]
    fn context_nur_an_das_eingebaute_modell() {
        assert!(receiver_is_local(&provider(
            crate::managers::llm::LOCAL_PROVIDER_ID,
            "http://127.0.0.1:1/v1"
        )));
        // Auch localhost zählt nicht: dahinter kann ein Proxy oder Relay stehen.
        assert!(!receiver_is_local(&provider(
            "ollama",
            "http://localhost:11434/v1"
        )));
        assert!(!receiver_is_local(&provider(
            "x",
            "http://127.0.0.1:8080/v1"
        )));
        assert!(!receiver_is_local(&provider(
            "ollama",
            "http://gpu.example.com:11434"
        )));
        assert!(!receiver_is_local(&provider(
            "openai",
            "https://api.example.com/v1"
        )));
    }

    #[test]
    fn kontext_steht_vor_der_letzten_frage() {
        let m = |r: &str, c: &str| VoiceCallMessage {
            role: r.into(),
            content: c.into(),
            derived: false,
        };
        let msgs = vec![
            m("system", "S"),
            m("user", "a"),
            m("assistant", "b"),
            m("user", "c"),
        ];
        let out = with_context_message(msgs.clone(), Some("K".into()));
        let got: Vec<&str> = out.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(got, ["S", "a", "b", "K\n\nc"]);
        assert_eq!(with_context_message(msgs, None).len(), 4);
    }

    fn msg(role: &str, content: &str, derived: bool) -> VoiceCallMessage {
        VoiceCallMessage {
            role: role.into(),
            content: content.into(),
            derived,
        }
    }

    #[test]
    fn routenwechsel_mit_abgeleitetem_verlauf() {
        let history = vec![
            msg("system", "S", false),
            msg("user", "Wer übernimmt?", false),
            msg("assistant", "Frau Berger [aus Auszug]", true),
            msg("user", "Und bis wann?", false),
        ];
        // Lokal: alles geht mit, aber nur über die gebundene Route.
        let local = for_receiver(history.clone(), true);
        assert_eq!(local.len(), 4);
        assert!(needs_pinned(false, &local, true));
        // Route jetzt Cloud: der abgeleitete Eintrag fällt weg, nichts ist gebunden.
        let remote = for_receiver(history, false);
        assert_eq!(remote.len(), 3);
        assert!(remote.iter().all(|m| !m.derived));
        assert!(!needs_pinned(false, &remote, false));
    }

    #[tokio::test]
    async fn abbruch_waehrend_der_quellenpruefung_verhindert_den_versand() {
        let _g = globale();
        // Registriert (wie in voice_call_chat) und während der Prüfung abgebrochen.
        let id = "bind-cancel-1";
        CANCELS
            .lock()
            .unwrap()
            .insert(id.into(), Arc::new(Notify::new()));
        let r = bind_then_check(id, async {
            voice_call_cancel(id.into());
            Some("KONTEXT".to_string())
        })
        .await;
        assert!(r.is_err(), "kein Versand nach Abbruch");
        CANCELS.lock().unwrap().remove(id);

        // Noch nicht registriert, Widerruf während der Prüfung.
        let id = "bind-cancel-2";
        mark_context(id);
        let r = bind_then_check(id, async {
            let _ = revoke_ids();
            Some("KONTEXT".to_string())
        })
        .await;
        assert!(r.is_err(), "Widerruf während der Prüfung");

        // Ohne Abbruch: Kontext bleibt.
        let r = bind_then_check("bind-ok", async { Some("K".to_string()) }).await;
        assert_eq!(r, Ok(Some("K".to_string())));
    }

    #[test]
    fn widerruf_von_projektlaeufen_trifft_nur_die_genannten_anfragen() {
        let _g = globale();
        // Zwei Anfragen mit Kontext: eine Projektlauf, eine mit Notizbuch (andere Herkunft).
        let (projekt, fremd) = ("ctx-projekt-1", "ctx-notizbuch-1");
        mark_context(projekt);
        mark_context(fremd);
        let flag = Arc::new(AtomicBool::new(false));
        let flag_fremd = Arc::new(AtomicBool::new(false));
        SYNTH_CANCELS
            .lock()
            .unwrap()
            .entry(projekt.into())
            .or_default()
            .push(flag.clone());
        SYNTH_CANCELS
            .lock()
            .unwrap()
            .entry(fremd.into())
            .or_default()
            .push(flag_fremd.clone());
        let ids = revoke_only(&[projekt.to_string(), projekt.to_string()]);
        assert_eq!(ids, vec![projekt.to_string()], "ohne Doppelte");
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            !flag_fremd.load(std::sync::atomic::Ordering::SeqCst),
            "die fremde Anfrage läuft weiter"
        );
        // Die fremde bleibt als Anfrage mit Kontext eingetragen und ist später noch widerrufbar.
        let rest = revoke_ids();
        assert!(rest.iter().any(|x| x == fremd));
        assert!(!rest.iter().any(|x| x == projekt));
        let _ = take_early_cancel(projekt);
        let _ = take_early_cancel(fremd);
    }

    #[test]
    fn widerruf_stoppt_auch_die_laufende_synthese() {
        let _g = globale();
        let id = "ctx-revoke-1";
        mark_context(id);
        let flag = Arc::new(AtomicBool::new(false));
        SYNTH_CANCELS
            .lock()
            .unwrap()
            .entry(id.into())
            .or_default()
            .push(flag.clone());
        // Das Modell ist fertig (kein CANCELS-Eintrag), die Wiedergabe läuft:
        // der Eintrag bleibt bis zum Widerruf bestehen.
        let ids = revoke_ids();
        assert!(ids.iter().any(|x| x == id));
        assert!(
            flag.load(std::sync::atomic::Ordering::SeqCst),
            "Synthese abgebrochen"
        );
        assert!(!revoke_ids().iter().any(|x| x == id));
        let _ = take_early_cancel(id);
    }

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn gesetzt(f: &Arc<AtomicBool>) -> bool {
        f.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn angemeldet(id: &str) -> bool {
        SYNTH_CANCELS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    #[test]
    fn abbruch_vor_der_anmeldung_verhindert_die_synthese() {
        let _g = globale();
        let id = "synth-vorab-1";
        // Nichts angemeldet, kein Durchgang: der Abbruch wird vorgemerkt.
        assert!(!voice_call_cancel(id.into()));
        let f = flag();
        assert!(synth_anmelden(id, &f), "die Anfrage war schon abgebrochen");
        assert!(gesetzt(&f), "Flag gesetzt, die Synthese startet nicht");
        assert!(
            !angemeldet(id),
            "eine abgebrochene Anfrage wird nicht eingetragen"
        );
        assert!(!take_early_cancel(id), "beim Einlösen entfernt");
        // Dieselbe Kennung danach: wieder ein gewöhnlicher Start, nicht noch einmal abgebrochen.
        let f2 = flag();
        assert!(!synth_anmelden(id, &f2));
        assert!(!gesetzt(&f2));
        assert!(angemeldet(id));
        synth_abmelden(id, &f2);
        assert!(!angemeldet(id));
    }

    #[test]
    fn angemeldete_synthese_wird_wie_bisher_abgebrochen_und_hinterlaesst_nichts() {
        let _g = globale();
        let id = "synth-laeuft-1";
        let f = flag();
        assert!(!synth_anmelden(id, &f));
        // Kein Durchgang unter dieser Kennung: `voice_call_cancel` meldet false, bricht die Synthese ab.
        assert!(!voice_call_cancel(id.into()));
        assert!(gesetzt(&f));
        assert!(!angemeldet(id));
        assert!(
            !take_early_cancel(id),
            "ein Abbruch, der eine laufende Synthese fand, bleibt nicht als Nachzügler liegen"
        );
        synth_abmelden(id, &f);
    }

    #[test]
    fn abbruch_ohne_vormerken_laesst_nichts_liegen() {
        let _g = globale();
        let id = "synth-ohne-vormerken-1";
        assert!(!cancel_synth(id, false));
        assert!(!take_early_cancel(id));
        let f = flag();
        assert!(!synth_anmelden(id, &f), "startet gewöhnlich");
        assert!(cancel_synth(id, false), "laufende Synthese gefunden");
        assert!(gesetzt(&f));
        synth_abmelden(id, &f);
    }

    #[test]
    fn abbruch_und_anmeldung_gleichzeitig_gehen_nie_aneinander_vorbei() {
        let _g = globale();
        for i in 0..2000 {
            let id = format!("synth-rennen-{i}");
            let f = flag();
            let tor = Arc::new(std::sync::Barrier::new(2));
            let abbrechen = {
                let (id, tor) = (id.clone(), tor.clone());
                std::thread::spawn(move || {
                    tor.wait();
                    voice_call_cancel(id)
                })
            };
            let anmelden = {
                let (id, f, tor) = (id.clone(), f.clone(), tor.clone());
                std::thread::spawn(move || {
                    tor.wait();
                    synth_anmelden(&id, &f)
                })
            };
            let _ = abbrechen.join().unwrap();
            let schon_abgebrochen = anmelden.join().unwrap();
            assert!(
                gesetzt(&f),
                "Durchlauf {i}: weder der Abbruch noch die Anmeldung hat das Flag gesetzt \
                 (Anmeldung meldete {schon_abgebrochen})"
            );
            assert!(!angemeldet(&id), "Durchlauf {i}: ein Eintrag blieb zurück");
            assert!(
                !take_early_cancel(&id),
                "Durchlauf {i}: ein Abbruch blieb vorgemerkt"
            );
            synth_abmelden(&id, &f);
        }
    }

    /// Deterministisch statt statistisch: Mitten im Abbruch (keine Synthese gefunden, noch nicht vorgemerkt)
    /// versucht eine Anmeldung durchzukommen. Sie muss an der Sperre warten; erst danach sieht sie den
    /// vorgemerkten Abbruch und startet nicht.
    #[test]
    fn eine_anmeldung_wartet_auf_den_abbruch_der_gerade_vormerkt() {
        let _g = globale();
        let id = "synth-fenster-1";
        let f = flag();
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let mut anmeldung = None;
        let mitten_drin = {
            let (f, tx) = (f.clone(), tx.clone());
            let anmeldung = &mut anmeldung;
            move || {
                *anmeldung = Some(std::thread::spawn(move || {
                    let schon_abgebrochen = synth_anmelden(id, &f);
                    let _ = tx.send(schon_abgebrochen);
                }));
            }
        };
        let durchgekommen = std::cell::Cell::new(false);
        let gefunden = cancel_synth_mit(id, true, || {
            mitten_drin();
            // Die Sperre liegt noch beim Abbruch: die Anmeldung darf in dieser Zeit nicht fertig werden.
            durchgekommen.set(rx.recv_timeout(Duration::from_millis(300)).is_ok());
        });
        assert!(!gefunden, "es lief keine Synthese");
        assert!(
            !durchgekommen.get(),
            "die Anmeldung kam mitten im Abbruch durch: Prüfen und Vormerken stehen nicht unter einer Sperre"
        );
        anmeldung.take().unwrap().join().unwrap();
        assert!(
            rx.try_recv().unwrap_or(false),
            "nach dem Abbruch sieht die Anmeldung den vorgemerkten Abbruch"
        );
        assert!(gesetzt(&f));
        assert!(!angemeldet(id));
        assert!(!take_early_cancel(id), "eingelöst, nichts bleibt liegen");
    }

    /// Der Befehl selbst ist ohne `AppHandle` nicht aufrufbar: dieser Wächter hält fest, dass er über
    /// `synth_anmelden` anmeldet, bevor er synthetisiert, und den Abbruch nicht an der Anmeldung vorbei
    /// einträgt.
    #[test]
    fn der_befehl_synthetisiert_erst_nach_der_anmeldung() {
        let quelle = include_str!("mod.rs");
        let start = quelle
            .find("pub async fn voice_call_synthesize(")
            .expect("Befehl fehlt");
        let ende = start + quelle[start..].find("\n}\n").expect("Ende fehlt");
        let rumpf = &quelle[start..ende];
        let anmelden = rumpf.find("synth_anmelden(").expect("Anmeldung fehlt");
        let synthese = rumpf.find("synthesize_wav(").expect("Synthese fehlt");
        assert!(anmelden < synthese, "erst anmelden, dann synthetisieren");
        assert!(
            rumpf[anmelden..synthese].contains("return Err"),
            "ein vorab abgebrochener Aufruf kehrt zurück, bevor synthetisiert wird"
        );
        assert!(
            !rumpf.contains("SYNTH_CANCELS"),
            "kein Eintrag an der gemeinsamen Sperre vorbei"
        );
        assert!(rumpf.contains("synth_abmelden("));
    }

    #[test]
    fn vorgemerkter_abbruch_verfaellt_nach_der_ablaufzeit() {
        let t0 = Instant::now();
        let mut v = Vorgemerkt::default();
        assert!(v.merke_um("a", t0));
        assert!(
            v.nimm_um("a", t0 + EARLY_TTL - Duration::from_secs(1)),
            "innerhalb der Frist gilt er"
        );
        assert!(!v.nimm_um("a", t0), "eingelöst heißt entfernt");
        assert!(v.merke_um("b", t0));
        assert!(
            !v.nimm_um("b", t0 + EARLY_TTL + Duration::from_secs(1)),
            "abgelaufen: zählt nicht mehr"
        );
        assert!(v.0.is_empty(), "und liegt auch nicht mehr da");
    }

    #[test]
    fn ablaufzeit_gibt_beim_einfuegen_platz_frei_ein_voller_vorrat_nimmt_nichts_mehr() {
        let t0 = Instant::now();
        let mut v = Vorgemerkt::default();
        for i in 0..EARLY_MAX {
            assert!(v.merke_um(&format!("alt-{i}"), t0));
        }
        assert!(
            !v.merke_um("zu-viel", t0 + Duration::from_secs(5)),
            "voll und alles frisch: nichts mehr"
        );
        assert_eq!(v.0.len(), EARLY_MAX);
        let spaeter = t0 + EARLY_TTL + Duration::from_secs(1);
        assert!(v.merke_um("neu", spaeter), "die abgelaufenen sind weg");
        assert_eq!(
            v.0.len(),
            1,
            "unbenutzte Kennungen füllen den Vorrat nicht dauerhaft"
        );
        assert!(v.nimm_um("neu", spaeter));
    }
}
