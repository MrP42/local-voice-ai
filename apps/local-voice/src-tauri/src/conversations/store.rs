//! Speicher der Sprachgespräche: `conversations.db` im App-Datenordner.
//!
//! Das Datenmodell folgt dem Unterhaltungs-Vertrag der Suite (wai-sdk,
//! `conversation.schema.json` und `conversation-entry.schema.json`, Version
//! 0.1.3): Feldnamen englisch in snake_case, skalare Felder als eigene
//! Spalten, verschachtelte Felder (`scope`, `participants`, `context`,
//! `attachments`, `variants`, `sources`, `ext`) als JSON-Text.
//!
//! Verbrauch (Token, Ledger-Kennungen, Anbieter) gehört nicht in den Vertrag
//! und steht nur lokal in `ext["local-voice-ai"]`. Rohaudio wird nie
//! gespeichert. Inhalte (Titel, Beitragstexte) stehen weder im Log noch in
//! Fehlermeldungen.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use log::info;
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use tauri::AppHandle;
use ulid::Ulid;

/// Vertragsversion, gegen die diese Datei schreibt.
pub const SCHEMA_VERSION: &str = "0.1.3";
/// `kind` einer Unterhaltung aus dem Sprachgespräch.
pub const KIND_VOICE: &str = "voice";
/// Kennung der Person, die das Gespräch führt (lokal immer der Besitzer).
pub const OWNER_ID: &str = "local";
/// Kennung der KI-Stimme als Teilnehmer.
pub const VOICEBOT_ID: &str = "voicebot";
/// Titel, wenn keiner angegeben ist.
pub const DEFAULT_TITLE: &str = "Neues Gespräch";
/// Standard-Datenklasse (Vertrag `DataClass`).
pub const DEFAULT_DATA_CLASS: &str = "INTERNAL";

pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_TEXT_CHARS: usize = 200_000;
const MAX_ATTACHMENTS: usize = 50;
const MAX_VARIANTS: usize = 100;
const MAX_SOURCES: usize = 50;
const MAX_SOURCE_CHARS: usize = 2000;
const MAX_REFERENCE_CHARS: usize = 1000;
const MAX_EXT_BYTES: usize = 16 * 1024;
const MAX_LIST_ROWS: i64 = 500;
/// So weit darf `created_at` eines Beitrags in der Zukunft liegen (Uhrenabweichung).
const MAX_FUTURE_MINUTES: i64 = 5;
const MAX_EXT_ID_CHARS: usize = 128;
const MAX_MODEL_CHARS: usize = 256;
const MAX_LAST_TEXT_CHARS: usize = 120;

const TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";
const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

const ROLES: &[&str] = &["user", "assistant", "system", "tool"];
const MODES: &[&str] = &["typed", "spoken", "dictated", "transcribed", "system"];
const STATUSES: &[&str] = &["complete", "interrupted", "error"];

static MIGRATIONS: &[M] = &[M::up(
    "CREATE TABLE IF NOT EXISTS conversations (
        id TEXT PRIMARY KEY NOT NULL,
        schema_version TEXT NOT NULL,
        ts TEXT NOT NULL,
        trace_id TEXT NOT NULL,
        title TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        kind TEXT NOT NULL,
        scope TEXT NOT NULL,
        participants TEXT NOT NULL,
        context TEXT NOT NULL,
        pinned INTEGER NOT NULL DEFAULT 0,
        archived INTEGER NOT NULL DEFAULT 0,
        ext TEXT
    );
    CREATE INDEX IF NOT EXISTS conversations_order
        ON conversations(archived, pinned, updated_at);
    CREATE TABLE IF NOT EXISTS conversation_entries (
        id TEXT PRIMARY KEY NOT NULL,
        conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
        schema_version TEXT NOT NULL,
        ts TEXT NOT NULL,
        trace_id TEXT NOT NULL,
        role TEXT NOT NULL,
        mode TEXT NOT NULL,
        text TEXT NOT NULL,
        status TEXT NOT NULL,
        author_id TEXT,
        triggered_by TEXT,
        model TEXT,
        attachments TEXT NOT NULL,
        variants TEXT NOT NULL,
        active_variant INTEGER,
        sources TEXT NOT NULL,
        untrusted INTEGER,
        reply_to TEXT,
        created_at TEXT NOT NULL,
        ext TEXT
    );
    CREATE INDEX IF NOT EXISTS conversation_entries_conv
        ON conversation_entries(conversation_id, created_at);",
)];

// ---------------------------------------------------------------------------
// Vertragstypen
// ---------------------------------------------------------------------------

/// Bereich der Unterhaltung (Vertrag `ConversationScope`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationScope {
    pub data_class: String,
    pub special_category: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_area: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sovereignty: Option<String>,
}

/// Teilnehmer (Vertrag `ConversationParticipant`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationParticipant {
    pub id: String,
    /// `person`, `ai`, `agent` oder `voicebot`.
    pub kind: String,
    /// `owner`, `member` oder `reader`.
    pub role: String,
    pub added_by: String,
    pub since: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_from_entry_id: Option<String>,
}

/// Verknüpfte Quelle (Vertrag `ContextSource`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ContextSource {
    /// `meeting`, `document`, `case` oder `conversation`.
    pub kind: String,
    /// `wai://`-Kennung.
    #[serde(rename = "ref")]
    pub reference: String,
    pub added_by: String,
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_release_id: Option<String>,
}

/// Verbrauch und Herkunft einer Antwort. Nur lokal, nie Teil des Vertrags.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct VoiceExt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    /// Ereignisse des Verbrauchs-Ledgers (`usage.db`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub usage_event_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<bool>,
}

/// Erweiterung je Herkunft (Vertrag `ConversationExt`): diese App schreibt nur ihren Schlüssel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationExt {
    #[serde(
        rename = "local-voice-ai",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub local_voice_ai: Option<VoiceExt>,
}

/// Anhang eines Beitrags (Vertrag `ConversationAttachment`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationAttachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    /// Fehlt, wenn die Größe nicht zu ermitteln war.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u32>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// Eine Antwortfassung (Vertrag `Variant`, ohne Quellen- und Werkzeugfelder).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EntryVariant {
    pub id: String,
    pub text: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub status: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// Unterhaltung (Vertrag `Conversation`; ohne `shared`, `ai_settings` und
/// `cost_bearer`: Sprachgespräche sind persönlich und haben keinen Kostenträger).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Conversation {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub kind: String,
    pub scope: ConversationScope,
    pub participants: Vec<ConversationParticipant>,
    pub context: Vec<ContextSource>,
    pub pinned: bool,
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<ConversationExt>,
}

/// Beitrag (Vertrag `ConversationEntry`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationEntry {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    pub conversation_id: String,
    /// `user`, `assistant`, `system` oder `tool`.
    pub role: String,
    /// `typed`, `spoken`, `dictated`, `transcribed` oder `system`.
    pub mode: String,
    pub text: String,
    /// `complete`, `interrupted` oder `error`.
    pub status: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggered_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ConversationAttachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<EntryVariant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_variant: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untrusted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<ConversationExt>,
}

/// Was die Oberfläche beim Anhängen mitbringt. Kennung, Zeit, Trace und
/// Schema-Version setzt der Speicher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct NewEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub role: String,
    pub mode: String,
    pub text: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggered_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ConversationAttachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<EntryVariant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_variant: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untrusted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<ConversationExt>,
}

/// Zeile der Verlaufsliste.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    pub pinned: bool,
    pub archived: bool,
    pub entry_count: u32,
    /// Anfang des jüngsten Beitrags, höchstens 120 Zeichen, einzeilig.
    pub last_text: String,
}

/// Ein Beitrag mit Gespräch und Titel (Suche des Sprachgesprächs, nur lesend).
#[derive(Debug, Clone, PartialEq)]
pub struct EntryHit {
    pub conversation_id: String,
    pub title: String,
    pub role: String,
    pub created_at: String,
    pub text: String,
}

/// Unterhaltung samt Beiträgen, älteste zuerst.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ConversationDetail {
    pub conversation: Conversation,
    pub entries: Vec<ConversationEntry>,
}

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

fn now_ts() -> String {
    Utc::now().format(TS_FORMAT).to_string()
}

/// Bringt einen RFC-3339-Zeitstempel in die Vertragsform (UTC, `Z`, Millisekunden).
fn normalize_ts(value: &str) -> Result<String> {
    let parsed =
        DateTime::parse_from_rfc3339(value).map_err(|_| anyhow!("Ungültiger Zeitstempel."))?;
    Ok(parsed.with_timezone(&Utc).format(TS_FORMAT).to_string())
}

/// Wie `normalize_ts`, aber nicht weiter als `MAX_FUTURE_MINUTES` in der Zukunft:
/// ein falsch gehender Browser-Takt würde sonst die Reihenfolge der Beiträge zerlegen.
fn capped_ts(value: &str) -> Result<String> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| anyhow!("Ungültiger Zeitstempel."))?
        .with_timezone(&Utc);
    let limit = Utc::now() + chrono::Duration::minutes(MAX_FUTURE_MINUTES);
    Ok(parsed.min(limit).format(TS_FORMAT).to_string())
}

/// 32 Hexzeichen, wie das W3C-Trace-Context verlangt.
fn new_trace_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn new_id(prefix: &str) -> String {
    format!("{}_{}", prefix, Ulid::new())
}

/// Vertrags-`Id`: `<präfix>_<ULID>`.
fn valid_id(value: &str) -> bool {
    let Some((prefix, rest)) = value.split_once('_') else {
        return false;
    };
    (2..=12).contains(&prefix.len())
        && prefix.chars().all(|c| c.is_ascii_lowercase())
        && rest.chars().count() == 26
        && rest.chars().all(|c| CROCKFORD.contains(c))
}

fn check_id(value: &str) -> Result<()> {
    if valid_id(value) {
        Ok(())
    } else {
        bail!("Ungültige Kennung.")
    }
}

fn check_ext_id(value: &str) -> Result<()> {
    let len = value.chars().count();
    if len == 0 || len > MAX_EXT_ID_CHARS {
        bail!("Ungültige Kennung.");
    }
    Ok(())
}

fn clean_title(value: &str) -> Result<String> {
    let title = value.trim();
    if title.is_empty() {
        bail!("Der Titel darf nicht leer sein.");
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        bail!("Der Titel darf höchstens {MAX_TITLE_CHARS} Zeichen lang sein.");
    }
    Ok(title.to_string())
}

fn check_enum(value: &str, allowed: &[&str], what: &str) -> Result<()> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        bail!("Ungültiger Wert für {what}.")
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn from_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|_| anyhow!("Gespeicherte Daten sind beschädigt."))
}

fn opt_to_json<T: Serialize>(value: &Option<T>) -> Result<Option<String>> {
    value.as_ref().map(to_json).transpose()
}

fn opt_from_json<T: DeserializeOwned>(text: Option<String>) -> Result<Option<T>> {
    text.as_deref().map(from_json).transpose()
}

fn validate_entry(entry: &NewEntry) -> Result<()> {
    check_enum(&entry.role, ROLES, "role")?;
    check_enum(&entry.mode, MODES, "mode")?;
    check_enum(&entry.status, STATUSES, "status")?;
    if entry.text.chars().count() > MAX_TEXT_CHARS {
        bail!("Der Text darf höchstens {MAX_TEXT_CHARS} Zeichen lang sein.");
    }
    if let Some(id) = &entry.id {
        check_id(id)?;
    }
    if let Some(id) = &entry.reply_to {
        check_id(id)?;
    }
    for id in [&entry.author_id, &entry.triggered_by]
        .into_iter()
        .flatten()
    {
        check_ext_id(id)?;
    }
    if let Some(model) = &entry.model {
        let len = model.chars().count();
        if len == 0 || len > MAX_MODEL_CHARS {
            bail!("Ungültiger Modellname.");
        }
    }
    if entry.attachments.len() > MAX_ATTACHMENTS
        || entry.variants.len() > MAX_VARIANTS
        || entry.sources.len() > MAX_SOURCES
    {
        bail!("Zu viele Anhänge, Fassungen oder Quellen.");
    }
    if entry
        .sources
        .iter()
        .any(|s| s.chars().count() > MAX_SOURCE_CHARS)
    {
        bail!("Eine Quellenangabe ist zu lang.");
    }
    if let Some(ext) = &entry.ext {
        if to_json(ext)?.len() > MAX_EXT_BYTES {
            bail!("Die Zusatzdaten sind zu groß.");
        }
    }
    for a in &entry.attachments {
        if let Some(r) = &a.reference {
            if r.chars().count() > MAX_REFERENCE_CHARS {
                bail!("Ungültiger Anhang.");
            }
        }
        check_ext_id(&a.id)?;
        let name_len = a.name.chars().count();
        if name_len == 0 || name_len > 255 || a.mime.is_empty() || a.mime.chars().count() > 127 {
            bail!("Ungültiger Anhang.");
        }
    }
    for v in &entry.variants {
        check_ext_id(&v.id)?;
        check_enum(&v.status, STATUSES, "status")?;
        normalize_ts(&v.created_at)?;
        if v.text.chars().count() > MAX_TEXT_CHARS {
            bail!("Der Text darf höchstens {MAX_TEXT_CHARS} Zeichen lang sein.");
        }
    }
    if let Some(active) = entry.active_variant {
        if active as usize >= entry.variants.len() {
            bail!("Die aktive Fassung gibt es nicht.");
        }
    }
    Ok(())
}

/// Einzeilig und auf `MAX_LAST_TEXT_CHARS` Zeichen gekürzt.
fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(MAX_LAST_TEXT_CHARS).collect()
}

struct RawConversation {
    id: String,
    schema_version: String,
    ts: String,
    trace_id: String,
    title: String,
    created_at: String,
    updated_at: String,
    kind: String,
    scope: String,
    participants: String,
    context: String,
    pinned: i64,
    archived: i64,
    ext: Option<String>,
}

const CONVERSATION_COLUMNS: &str = "id, schema_version, ts, trace_id, title, created_at, \
     updated_at, kind, scope, participants, context, pinned, archived, ext";

fn raw_conversation(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawConversation> {
    Ok(RawConversation {
        id: r.get(0)?,
        schema_version: r.get(1)?,
        ts: r.get(2)?,
        trace_id: r.get(3)?,
        title: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        kind: r.get(7)?,
        scope: r.get(8)?,
        participants: r.get(9)?,
        context: r.get(10)?,
        pinned: r.get(11)?,
        archived: r.get(12)?,
        ext: r.get(13)?,
    })
}

fn conversation_from(raw: RawConversation) -> Result<Conversation> {
    Ok(Conversation {
        schema_version: raw.schema_version,
        id: raw.id,
        ts: raw.ts,
        trace_id: raw.trace_id,
        title: raw.title,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
        kind: raw.kind,
        scope: from_json(&raw.scope)?,
        participants: from_json(&raw.participants)?,
        context: from_json(&raw.context)?,
        pinned: raw.pinned != 0,
        archived: raw.archived != 0,
        ext: opt_from_json(raw.ext)?,
    })
}

struct RawEntry {
    id: String,
    conversation_id: String,
    schema_version: String,
    ts: String,
    trace_id: String,
    role: String,
    mode: String,
    text: String,
    status: String,
    author_id: Option<String>,
    triggered_by: Option<String>,
    model: Option<String>,
    attachments: String,
    variants: String,
    active_variant: Option<i64>,
    sources: String,
    untrusted: Option<i64>,
    reply_to: Option<String>,
    created_at: String,
    ext: Option<String>,
}

const ENTRY_COLUMNS: &str = "id, conversation_id, schema_version, ts, trace_id, role, mode, \
     text, status, author_id, triggered_by, model, attachments, variants, active_variant, \
     sources, untrusted, reply_to, created_at, ext";

fn raw_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok(RawEntry {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        schema_version: r.get(2)?,
        ts: r.get(3)?,
        trace_id: r.get(4)?,
        role: r.get(5)?,
        mode: r.get(6)?,
        text: r.get(7)?,
        status: r.get(8)?,
        author_id: r.get(9)?,
        triggered_by: r.get(10)?,
        model: r.get(11)?,
        attachments: r.get(12)?,
        variants: r.get(13)?,
        active_variant: r.get(14)?,
        sources: r.get(15)?,
        untrusted: r.get(16)?,
        reply_to: r.get(17)?,
        created_at: r.get(18)?,
        ext: r.get(19)?,
    })
}

fn entry_from(raw: RawEntry) -> Result<ConversationEntry> {
    Ok(ConversationEntry {
        schema_version: raw.schema_version,
        id: raw.id,
        ts: raw.ts,
        trace_id: raw.trace_id,
        conversation_id: raw.conversation_id,
        role: raw.role,
        mode: raw.mode,
        text: raw.text,
        status: raw.status,
        created_at: raw.created_at,
        author_id: raw.author_id,
        triggered_by: raw.triggered_by,
        model: raw.model,
        attachments: from_json(&raw.attachments)?,
        variants: from_json(&raw.variants)?,
        active_variant: raw.active_variant.map(|v| v as u32),
        sources: from_json(&raw.sources)?,
        untrusted: raw.untrusted.map(|v| v != 0),
        reply_to: raw.reply_to,
        ext: opt_from_json(raw.ext)?,
    })
}

// ---------------------------------------------------------------------------
// Speicher
// ---------------------------------------------------------------------------

pub struct ConversationStore {
    conn: Mutex<Connection>,
}

impl ConversationStore {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let dir = crate::portable::app_data_dir(app_handle)?;
        std::fs::create_dir_all(&dir)?;
        Self::open(&dir.join("conversations.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self::init(conn)?;
        info!("Gesprächsverlauf geöffnet: {}", path.display());
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // SQLite prüft Fremdschlüssel nur, wenn man es je Verbindung einschaltet;
        // ohne das kaskadiert das Löschen nicht.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let migrations = Migrations::new(MIGRATIONS.to_vec());
        #[cfg(debug_assertions)]
        migrations
            .validate()
            .expect("Invalid conversations migrations");
        migrations.to_latest(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Angeheftete zuerst, dann nach letzter Änderung absteigend.
    pub fn list(&self, include_archived: bool) -> Result<Vec<ConversationSummary>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.title, c.updated_at, c.pinned, c.archived,
                    (SELECT COUNT(*) FROM conversation_entries e
                      WHERE e.conversation_id = c.id),
                    (SELECT substr(e.text, 1, 240) FROM conversation_entries e
                      WHERE e.conversation_id = c.id
                      ORDER BY e.created_at DESC, e.rowid DESC LIMIT 1)
             FROM conversations c
             WHERE (?1 = 1 OR c.archived = 0)
             ORDER BY c.pinned DESC, c.updated_at DESC, c.rowid DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![include_archived as i64, MAX_LIST_ROWS], |r| {
            Ok(ConversationSummary {
                id: r.get(0)?,
                title: r.get(1)?,
                updated_at: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                archived: r.get::<_, i64>(4)? != 0,
                entry_count: r.get::<_, i64>(5)? as u32,
                last_text: r
                    .get::<_, Option<String>>(6)?
                    .map(|t| one_line(&t))
                    .unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Beitraege nicht archivierter Gespraeche, neueste zuerst, hoechstens `limit` (Sprachgespraech:
    /// Suche in frueheren Gespraechen). Je Beitrag: Gespraech, Titel, Rolle, Zeitstempel, Text.
    pub fn recent_entries(&self, limit: usize) -> Result<Vec<EntryHit>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT e.conversation_id, c.title, e.role, e.created_at, e.text
             FROM conversation_entries e JOIN conversations c ON c.id = e.conversation_id
             WHERE c.archived = 0
             ORDER BY e.created_at DESC, e.rowid DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.min(5_000) as i64], |r| {
            Ok(EntryHit {
                conversation_id: r.get(0)?,
                title: r.get(1)?,
                role: r.get(2)?,
                created_at: r.get(3)?,
                text: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get(&self, id: &str) -> Result<ConversationDetail> {
        check_id(id)?;
        let conn = self.lock();
        let conversation = Self::conversation(&conn, id)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM conversation_entries
             WHERE conversation_id = ?1 ORDER BY created_at ASC, rowid ASC"
        ))?;
        let raws = stmt
            .query_map(params![id], raw_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let entries = raws
            .into_iter()
            .map(entry_from)
            .collect::<Result<Vec<_>>>()?;
        Ok(ConversationDetail {
            conversation,
            entries,
        })
    }

    pub fn create(&self, title: Option<String>) -> Result<Conversation> {
        let title = match title.as_deref().map(str::trim) {
            None | Some("") => DEFAULT_TITLE.to_string(),
            Some(t) => clean_title(t)?,
        };
        let now = now_ts();
        let conversation = Conversation {
            schema_version: SCHEMA_VERSION.to_string(),
            id: new_id("conv"),
            ts: now.clone(),
            trace_id: new_trace_id(),
            title,
            created_at: now.clone(),
            updated_at: now.clone(),
            kind: KIND_VOICE.to_string(),
            scope: ConversationScope {
                data_class: DEFAULT_DATA_CLASS.to_string(),
                special_category: false,
                project_id: None,
                knowledge_area: None,
                sovereignty: None,
            },
            participants: vec![
                ConversationParticipant {
                    id: OWNER_ID.to_string(),
                    kind: "person".to_string(),
                    role: "owner".to_string(),
                    added_by: OWNER_ID.to_string(),
                    since: now.clone(),
                    agent_version: None,
                    visible_from_entry_id: None,
                },
                ConversationParticipant {
                    id: VOICEBOT_ID.to_string(),
                    kind: "voicebot".to_string(),
                    role: "member".to_string(),
                    added_by: OWNER_ID.to_string(),
                    since: now,
                    agent_version: None,
                    visible_from_entry_id: None,
                },
            ],
            context: Vec::new(),
            pinned: false,
            archived: false,
            ext: None,
        };
        let conn = self.lock();
        conn.execute(
            &format!(
                "INSERT INTO conversations ({CONVERSATION_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
            ),
            params![
                conversation.id,
                conversation.schema_version,
                conversation.ts,
                conversation.trace_id,
                conversation.title,
                conversation.created_at,
                conversation.updated_at,
                conversation.kind,
                to_json(&conversation.scope)?,
                to_json(&conversation.participants)?,
                to_json(&conversation.context)?,
                conversation.pinned as i64,
                conversation.archived as i64,
                opt_to_json(&conversation.ext)?,
            ],
        )?;
        Ok(conversation)
    }

    /// Ändert Titel, Anheften oder Archiv. `updated_at` bleibt unberührt: es
    /// zählt die letzte Aktivität im Gespräch, sonst springt ein umbenannter
    /// oder angehefteter Eintrag in der Liste nach oben.
    pub fn update(
        &self,
        id: &str,
        title: Option<String>,
        pinned: Option<bool>,
        archived: Option<bool>,
    ) -> Result<Conversation> {
        check_id(id)?;
        let title = title.as_deref().map(clean_title).transpose()?;
        let conn = self.lock();
        let current = Self::conversation(&conn, id)?;
        conn.execute(
            "UPDATE conversations SET title = ?1, pinned = ?2, archived = ?3 WHERE id = ?4",
            params![
                title.unwrap_or(current.title),
                pinned.unwrap_or(current.pinned) as i64,
                archived.unwrap_or(current.archived) as i64,
                id,
            ],
        )?;
        Self::conversation(&conn, id)
    }

    /// Löscht die Unterhaltung samt Beiträgen (Fremdschlüssel kaskadiert).
    pub fn delete(&self, id: &str) -> Result<()> {
        check_id(id)?;
        let conn = self.lock();
        let removed = conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        if removed == 0 {
            bail!("Das Gespräch gibt es nicht (mehr).");
        }
        Ok(())
    }

    pub fn append_entry(
        &self,
        conversation_id: &str,
        entry: NewEntry,
    ) -> Result<ConversationEntry> {
        check_id(conversation_id)?;
        validate_entry(&entry)?;
        let now = now_ts();
        let created_at = match &entry.created_at {
            Some(t) => capped_ts(t)?,
            None => now.clone(),
        };
        let stored = ConversationEntry {
            schema_version: SCHEMA_VERSION.to_string(),
            id: entry.id.unwrap_or_else(|| new_id("entry")),
            ts: now.clone(),
            trace_id: new_trace_id(),
            conversation_id: conversation_id.to_string(),
            role: entry.role,
            mode: entry.mode,
            text: entry.text,
            status: entry.status,
            created_at,
            author_id: entry.author_id,
            triggered_by: entry.triggered_by,
            model: entry.model,
            attachments: entry.attachments,
            variants: entry.variants,
            active_variant: entry.active_variant,
            sources: entry.sources,
            untrusted: entry.untrusted,
            reply_to: entry.reply_to,
            ext: entry.ext,
        };
        let conn = self.lock();
        let tx = conn.unchecked_transaction()?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM conversations WHERE id = ?1",
                params![conversation_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if exists.is_none() {
            bail!("Das Gespräch gibt es nicht (mehr).");
        }
        tx.execute(
            &format!(
                "INSERT INTO conversation_entries ({ENTRY_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20)"
            ),
            params![
                stored.id,
                stored.conversation_id,
                stored.schema_version,
                stored.ts,
                stored.trace_id,
                stored.role,
                stored.mode,
                stored.text,
                stored.status,
                stored.author_id,
                stored.triggered_by,
                stored.model,
                to_json(&stored.attachments)?,
                to_json(&stored.variants)?,
                stored.active_variant.map(i64::from),
                to_json(&stored.sources)?,
                stored.untrusted.map(|b| b as i64),
                stored.reply_to,
                stored.created_at,
                opt_to_json(&stored.ext)?,
            ],
        )
        .context("Der Beitrag konnte nicht gespeichert werden.")?;
        tx.execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![now, conversation_id],
        )?;
        tx.commit()?;
        Ok(stored)
    }

    /// Ändert Text und Status eines Beitrags (eine als vollständig gespeicherte
    /// Antwort wurde beim Sprechen unterbrochen). Der Beitrag bleibt derselbe;
    /// `updated_at` der Unterhaltung ändert sich nicht.
    pub fn update_entry(
        &self,
        id: &str,
        text: String,
        status: String,
    ) -> Result<ConversationEntry> {
        check_id(id)?;
        check_enum(&status, STATUSES, "status")?;
        if text.chars().count() > MAX_TEXT_CHARS {
            bail!("Der Text darf höchstens {MAX_TEXT_CHARS} Zeichen lang sein.");
        }
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE conversation_entries SET text = ?1, status = ?2 WHERE id = ?3",
            params![text, status, id],
        )?;
        if changed == 0 {
            bail!("Den Beitrag gibt es nicht (mehr).");
        }
        let raw = conn.query_row(
            &format!("SELECT {ENTRY_COLUMNS} FROM conversation_entries WHERE id = ?1"),
            params![id],
            raw_entry,
        )?;
        entry_from(raw)
    }

    fn conversation(conn: &Connection, id: &str) -> Result<Conversation> {
        let raw = conn
            .query_row(
                &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"),
                params![id],
                raw_conversation,
            )
            .optional()?
            .ok_or_else(|| anyhow!("Das Gespräch gibt es nicht (mehr)."))?;
        conversation_from(raw)
    }
}
