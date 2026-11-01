//! Werkzeuge fuer den Orchestrator des Sprachgespraechs (Stufe 2): `voice_tools_list` und
//! `voice_tool_call`. Die Logik steht in `crate::agent_bridge::voice` (nur lesende Werkzeuge, nur fuer
//! die Wissensbereiche, die der Nutzer freigegeben hat, jeder Aufruf durch das Tor mit Audit); hier nur
//! Zugriff auf Zustand und Blockier-Thread sowie die echte Datenquelle der Wissenswerkzeuge
//! ([`AppKnowledge`]).
//!
//! Fehlermeldungen und Antworten enthalten keine Inhalte ausser dem Ergebnis des Werkzeugs selbst,
//! das an das Sprachmodell geht; nichts davon wird geloggt.

use std::sync::{Arc, Mutex};

use chrono::DateTime;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::agent_bridge::runtime::AppBridge;
use crate::agent_bridge::tools::meetings::READ_BUSY_TIMEOUT;
use crate::agent_bridge::voice::areas::{self, Areas};
use crate::agent_bridge::voice::knowledge::{
    ConversationInfo, ConversationView, Dictation, EntryInfo, EntryLine, EventInfo,
    KnowledgeSource, MeetingInfo, PageInfo,
};
use crate::agent_bridge::voice::{self, RateLimiter, Session, VoiceOptions, VoiceToolInfo};
use crate::conversations::store::ConversationStore;
use crate::managers::history::HistoryManager;
use crate::managers::meetings::store::MeetingStore;
use crate::mcp::tools::{self as mcp_tools, Backend};

/// Aufrufe je Minute (alle Gespräche): eine Schleife des Modells darf die Datenbank nicht fluten.
static LIMITER: Lazy<Mutex<RateLimiter>> = Lazy::new(Default::default);

/// Die freigegebenen Bereiche aus den Einstellungen.
fn options(app: &AppHandle) -> VoiceOptions {
    let settings = crate::settings::get_settings(app);
    VoiceOptions {
        areas: Areas::from_settings(
            &settings.voice_call_knowledge,
            settings.voice_call_read_transcripts,
        ),
    }
}

/// „Transkripte im Sprachgespräch lesen“ (Standard aus; wirkt nur mit dem Bereich Besprechungen).
#[tauri::command]
#[specta::specta]
pub fn change_voice_call_read_transcripts_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.voice_call_read_transcripts = enabled;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Wissensbereiche, die das Sprachgespräch nachschlagen darf (Standard: keiner). Gespeichert werden
/// nur bekannte Bereiche, ohne Doppelte; „Transkripte“ hat den Unterschalter oben.
#[tauri::command]
#[specta::specta]
pub fn change_voice_call_knowledge_setting(
    app: AppHandle,
    areas: Vec<String>,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.voice_call_knowledge = areas::normalize_keys(&areas);
    crate::settings::write_settings(&app, settings);
    Ok(())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Bruecke und Datenbank des Laufs; fehlt eines, ist das ein Klartext (die App startet noch).
fn parts(app: &AppHandle) -> Result<(Arc<AppBridge>, Arc<MeetingStore>), String> {
    let bridge = app
        .try_state::<Arc<AppBridge>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Die Werkzeuge sind noch nicht bereit.".to_string())?;
    let store = app
        .try_state::<Arc<MeetingStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Die Besprechungsdatenbank ist noch nicht bereit.".to_string())?;
    Ok((bridge, store))
}

/// Die lesenden Werkzeuge, die das Sprachgespräch jetzt benutzen darf (nach den Bereichen).
#[tauri::command]
#[specta::specta]
pub async fn voice_tools_list(app: AppHandle) -> Result<Vec<VoiceToolInfo>, String> {
    let opts = options(&app);
    if opts.areas.is_empty() {
        return Ok(Vec::new());
    }
    let (bridge, _) = parts(&app)?;
    Ok(voice::list(bridge.server().bridge().registry(), opts))
}

/// Ruft ein lesendes Werkzeug (Bereich frei, Audit über das Tor). `args` ist ein JSON-Objekt als
/// Text, `conversation_id` das laufende Gespräch (es bleibt den Gesprächswerkzeugen verborgen). Die
/// Antwort ist der JSON-Text des Ergebnisses; eine zu lange Antwort wird nie still gekürzt, sondern
/// abgelehnt.
#[tauri::command]
#[specta::specta]
pub async fn voice_tool_call(
    app: AppHandle,
    name: String,
    args: String,
    conversation_id: Option<String>,
) -> Result<String, String> {
    let (bridge, store) = parts(&app)?;
    let opts = options(&app);
    let allowed = LIMITER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allow(now_ms());
    if !allowed {
        return Err(format!(
            "Zu viele Werkzeugaufrufe in kurzer Zeit (höchstens {} pro Minute). Es wurde nichts ausgeführt.",
            RateLimiter::MAX_CALLS
        ));
    }
    let source = AppKnowledge::new(&app, store.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store.get_connection().map_err(|e| e.to_string())?;
        let session = Session {
            registry: bridge.server().bridge().registry(),
            source: &source,
            conversation: conversation_id.as_deref(),
        };
        voice::call(&conn, &session, opts, &name, &args, now_ms())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Datenquelle der Wissenswerkzeuge
// ---------------------------------------------------------------------------

/// Der echte Zugriff der Wissenswerkzeuge auf die Daten der App. Alles hier liest nur; ein Netzabruf
/// kommt nirgends vor (Termine kommen aus dem Cache der Kalenderquellen).
struct AppKnowledge {
    app: AppHandle,
    store: Arc<MeetingStore>,
}

impl AppKnowledge {
    fn new(app: &AppHandle, store: Arc<MeetingStore>) -> Self {
        Self {
            app: app.clone(),
            store,
        }
    }

    fn history(&self) -> Result<Arc<HistoryManager>, String> {
        self.app
            .try_state::<Arc<HistoryManager>>()
            .map(|s| s.inner().clone())
            .ok_or_else(|| "Der Diktatverlauf ist noch nicht bereit.".to_string())
    }

    fn conversation_store(&self) -> Result<Arc<ConversationStore>, String> {
        self.app
            .try_state::<Arc<ConversationStore>>()
            .map(|s| s.inner().clone())
            .ok_or_else(|| "Der Gesprächsverlauf ist nicht verfügbar.".to_string())
    }
}

/// Millisekunden eines RFC-3339-Zeitstempels (0, wenn nicht lesbar).
fn ts_ms(text: &str) -> i64 {
    DateTime::parse_from_rfc3339(text)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

/// Aufgerundete Sekunden: ein Eintrag zaehlt ab dem ersten ganzen Zeitpunkt im Fenster.
fn ceil_secs(ms: i64) -> i64 {
    (ms + 999).div_euclid(1000)
}

/// Fehlertext ohne Inhalte (die Datenbankfehler nennen keine Texte, werden aber nie ausgegeben).
fn read_error(what: &str, e: &dyn std::fmt::Display) -> String {
    log::warn!("voice_tools: {what} nicht lesbar: {e}");
    format!("{what} ist gerade nicht lesbar.")
}

impl KnowledgeSource for AppKnowledge {
    fn dictations(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Dictation>, String> {
        let entries = self
            .history()?
            .dictations_between(from_ms.map(ceil_secs), to_ms.map(ceil_secs), limit)
            .map_err(|e| read_error("Der Diktatverlauf", &e))?;
        Ok(entries
            .into_iter()
            .map(|e| Dictation {
                id: e.id,
                at_ms: e.timestamp * 1000,
                text: e
                    .post_processed_text
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or(e.transcription_text),
            })
            .collect())
    }

    fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
        let list = self
            .conversation_store()?
            .list(false)
            .map_err(|e| read_error("Der Gesprächsverlauf", &e))?;
        let mut out: Vec<ConversationInfo> = list
            .into_iter()
            .map(|c| ConversationInfo {
                updated_ms: ts_ms(&c.updated_at),
                id: c.id,
                title: c.title,
                entries: c.entry_count,
                last_text: c.last_text,
            })
            .collect();
        out.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
        Ok(out)
    }

    fn conversation_entries(&self, limit: usize) -> Result<Vec<EntryInfo>, String> {
        Ok(self
            .conversation_store()?
            .recent_entries(limit)
            .map_err(|e| read_error("Der Gesprächsverlauf", &e))?
            .into_iter()
            .map(|h| EntryInfo {
                at_ms: ts_ms(&h.created_at),
                conversation_id: h.conversation_id,
                title: h.title,
                role: h.role,
                text: h.text,
            })
            .collect())
    }

    fn conversation(&self, id: &str) -> Result<ConversationView, String> {
        let detail = self
            .conversation_store()?
            .get(id)
            .map_err(|_| "Das Gespräch gibt es nicht (mehr).".to_string())?;
        if detail.conversation.archived {
            return Err("Das Gespräch gibt es nicht (mehr).".to_string());
        }
        Ok(ConversationView {
            id: detail.conversation.id,
            title: detail.conversation.title,
            updated_ms: ts_ms(&detail.conversation.updated_at),
            entries: detail
                .entries
                .into_iter()
                .map(|e| EntryLine {
                    at_ms: ts_ms(&e.created_at),
                    role: e.role,
                    text: e.text,
                })
                .collect(),
        })
    }

    fn calendar(&self, from_ms: i64, to_ms: i64) -> Result<Vec<EventInfo>, String> {
        Ok(self
            .store
            .calendar_events_between(from_ms, to_ms, false)
            .map_err(|e| read_error("Der Kalender-Cache", &e))?
            .into_iter()
            .map(|e| EventInfo {
                title: e.title,
                starts_ms: e.starts_at,
                ends_ms: e.ends_at,
                all_day: e.all_day,
                location: e.location,
            })
            .collect())
    }

    fn meetings(&self, from_ms: i64, to_ms: i64, limit: usize) -> Result<Vec<MeetingInfo>, String> {
        let iso = |ms: i64| {
            DateTime::from_timestamp_millis(ms)
                .map(|d| d.to_rfc3339())
                .unwrap_or_default()
        };
        // Dieselbe Logik und Grenzen wie `meetings_list` (nur fertige Besprechungen).
        let backend = Backend {
            db_path: self.store.db_path(),
            busy_timeout: READ_BUSY_TIMEOUT,
            include_transcript: false,
            transcript_page_chars: voice::VOICE_PAGE_CHARS,
        };
        let args = json!({ "from": iso(from_ms), "to": iso(to_ms - 1), "limit": limit });
        let outcome = mcp_tools::call("list_meetings", &args, &backend);
        if outcome.is_error {
            return Err(outcome.text);
        }
        let parsed: Value = serde_json::from_str(&outcome.text)
            .map_err(|_| "Die Antwort der Besprechungsdatenbank war nicht lesbar.".to_string())?;
        Ok(parsed["meetings"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|m| MeetingInfo {
                        id: m["id"].as_str().unwrap_or_default().to_string(),
                        title: m["title"].as_str().unwrap_or_default().to_string(),
                        started_ms: ts_ms(m["date"].as_str().unwrap_or_default()),
                        minutes: m["duration_seconds"].as_u64().map(|s| s / 60),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn pages(&self) -> Result<Vec<PageInfo>, String> {
        Ok(crate::commands::pages::pages_snapshot(&self.app)
            .map_err(|e| read_error("Die Seitenliste", &e))?
            .into_iter()
            .map(|p| PageInfo {
                id: p.id,
                title: p.title,
                modified_ms: p.modified_ms as i64,
                preview: p.preview,
            })
            .collect())
    }

    fn page_text(&self, id: &str) -> Result<String, String> {
        let raw = crate::commands::pages::page_state_load(self.app.clone(), id.to_string())
            .map_err(|_| "Die Seite gibt es nicht.".to_string())?;
        if raw.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_string))
            .unwrap_or(raw))
    }
}
