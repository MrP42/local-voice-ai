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

pub mod sentences;
pub mod turn;

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

static CANCELS: Lazy<Mutex<HashMap<String, Arc<Notify>>>> = Lazy::new(Default::default);
/// Abbrüche, die vor dem Start ihrer Anfrage eintrafen (IPC-Reihenfolge ist nicht garantiert).
static EARLY_CANCELS: Lazy<Mutex<std::collections::HashSet<String>>> = Lazy::new(Default::default);
/// Laufende Sprachsynthesen je Anfrage (Abbruchflags).
static SYNTH_CANCELS: Lazy<Mutex<HashMap<String, Vec<Arc<AtomicBool>>>>> =
    Lazy::new(Default::default);
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
        let call = llm_client::send_chat_completion_stream_routed(
            Purpose::Chat,
            &provider,
            api_key,
            &model,
            messages,
            &on_delta,
            pinned_local.as_deref(),
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
#[tauri::command]
#[specta::specta]
pub fn voice_call_cancel(request_id: String) -> bool {
    cancel_synth(&request_id);
    unmark_context(&request_id);
    let registered = CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&request_id)
        .cloned();
    match registered {
        Some(n) => {
            CANCELLED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(request_id.clone());
            n.notify_one();
            true
        }
        None => {
            let mut early = EARLY_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
            // Begrenzen: ein Abbruch ohne Anfrage darf nicht ewig liegen bleiben.
            if early.len() < 64 {
                early.insert(request_id);
            }
            false
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
    // auch die Synthese ab (Codex M5 Must-fix 3).
    if let Some(id) = &request_id {
        SYNTH_CANCELS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.clone())
            .or_default()
            .push(cancel.clone());
    }
    let wav = tts.synthesize_wav(&text, Some(&cancel)).await;
    if let Some(id) = &request_id {
        let mut map = SYNTH_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = map.get_mut(id) {
            list.retain(|c| !Arc::ptr_eq(c, &cancel));
            if list.is_empty() {
                map.remove(id);
            }
        }
    }
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("abgebrochen".into());
    }
    Ok(B64.encode(wav?))
}

/// Laufende Synthesen einer Anfrage abbrechen.
fn cancel_synth(request_id: &str) {
    if let Some(list) = SYNTH_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(request_id)
    {
        for c in list {
            c.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CONTEXT_REQUESTS`, `CANCELS` und `EARLY_CANCELS` sind prozessweit: `revoke_ids` leert die
    /// ganze Liste. Tests, die sie benutzen, laufen deshalb nacheinander (sonst nimmt ein paralleler
    /// Test die Kennung des anderen weg; Flake in lva#423).
    static GLOBALE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn globale() -> std::sync::MutexGuard<'static, ()> {
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
}
