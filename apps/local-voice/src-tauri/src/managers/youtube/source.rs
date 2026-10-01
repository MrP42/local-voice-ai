//! Eine YouTube-Quelle anlegen (A2): Register, Tor, Audit, oEmbed, Besprechung
//! mit Projekt und Provenienz.
//!
//! Ablauf von `add_youtube_source` (jeder Schritt bricht den Rest ab):
//! 1. Link pruefen (`link`): kein Netz, keine Datenbank, bei Fehler nichts getan.
//! 2. Sperre je (Datenbank, Video-ID): ein zweites Anlegen desselben Videos waehrend
//!    des ersten meldet `Busy`.
//! 3. Projekt pruefen, YouTube-Integration sicherstellen, Tor `media.fetch` fuer den
//!    Nutzer, Audit-Eintrag `pending`. Laesst sich das Audit nicht schreiben, geht
//!    KEIN Byte ins Netz (fail closed, wie im Tor aus A1).
//! 4. oEmbed-Abruf (der einzige Netzverkehr).
//! 5. Audit auf `ok`/`error` setzen, Register (`last_ok_at`/`last_error`).
//! 6. Besprechung, Projektzuordnung und Provenienz in EINER Transaktion.
//!
//! Der Nutzer in der Oberflaeche steht nicht im Audit des Tores (A1); diese
//! Stelle schreibt ihn ausdruecklich hinein, weil der Schritt Verkehr nach aussen
//! ausloest. Das Ziel im Audit ist `youtube:<ID>`, nie die Adresse.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use specta::Type;
use ulid::Ulid;

use super::link::{normalize_link, valid_video_id, YoutubeRef};
use super::oembed::{self, VideoMeta};
use super::{YoutubeError, INTEGRATION_ID, METADATA_KEY, SOURCE_KIND};
use crate::managers::integrations::audit;
use crate::managers::integrations::gate::{self, Decision, Request};
use crate::managers::integrations::model::{
    AuditOutcome, Caller, Capability, Direction, Integration, IntegrationError, Kind, NewAudit,
    NewIntegration,
};
use crate::managers::integrations::store as register;
use crate::managers::meetings::empty::{fill_empty_tx, EmptyFill};
use crate::managers::meetings::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore};
use crate::managers::provenance::{self, ActorKind, NewProvenance, SourceRef, SubjectKind};

#[cfg(test)]
mod tests;

/// Die Angaben zur Quelle, wie die Oberflaeche sie liest (Player, Kopf, Herkunft).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
pub struct YoutubeSource {
    pub video_id: String,
    /// Bereinigte Adresse (nur die ID), immer aus `video_id` abgeleitet.
    pub url: String,
    pub title: String,
    pub channel: String,
    pub channel_url: Option<String>,
    pub thumbnail_url: Option<String>,
    /// Startzeit aus dem Link in Sekunden.
    pub start_s: Option<u32>,
}

/// Wohin und wie abgerufen wird.
#[derive(Clone, Debug)]
pub struct AddOptions {
    pub oembed_base: String,
    pub fetch: oembed::FetchOpts,
}

impl AddOptions {
    /// Die echte Adresse (in der Sandbox ggf. ein Teststand) mit den Standardgrenzen.
    pub fn production() -> Self {
        Self {
            oembed_base: oembed::oembed_base(),
            fetch: oembed::FetchOpts::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AddedSource {
    pub meeting: Meeting,
    pub meta: VideoMeta,
    pub video: YoutubeRef,
}

/// Wer den Link anlegt (A8): der Nutzer per Knopfdruck oder ein externer Agent. Es bestimmt, unter
/// welchem Aufrufer der Abruf im Audit steht und von wem die Herkunft des Transkripts
/// stammt. Das RECHT des Agenten (`youtube.add`) prueft die Agentenbruecke VOR dem Aufruf; hier
/// geht es nur um die richtige Zuordnung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub actor: ActorKind,
    /// Kennung des Zugangs bei einem Agenten.
    pub actor_ref: Option<String>,
}

impl Origin {
    pub fn user() -> Self {
        Self {
            actor: ActorKind::User,
            actor_ref: None,
        }
    }

    pub fn agent_external(client_id: &str) -> Self {
        Self {
            actor: ActorKind::AgentExternal,
            actor_ref: Some(client_id.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Sperre je Video
// ---------------------------------------------------------------------------

fn in_flight() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Haelt die Sperre fuer ein Video; der Drop gibt sie frei, auch bei Fehler und
/// Abbruch des Futures.
struct FlightGuard(String);

impl Drop for FlightGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = in_flight().lock() {
            set.remove(&self.0);
        }
    }
}

/// Der Schluessel enthaelt den Datenbankpfad: zwei Stores (Tests, Sandbox neben
/// der App) sperren sich nicht gegenseitig.
fn acquire(store: &MeetingStore, video_id: &str) -> Result<FlightGuard, YoutubeError> {
    let key = format!("{}|{video_id}", store.db_path().display());
    let mut set = in_flight()
        .lock()
        .map_err(|_| YoutubeError::Store("Sperre nicht verfügbar".to_string()))?;
    if !set.insert(key.clone()) {
        return Err(YoutubeError::Busy);
    }
    Ok(FlightGuard(key))
}

// ---------------------------------------------------------------------------
// Register
// ---------------------------------------------------------------------------

/// Die YouTube-Integration im Register: die vorhandene (feste Kennung, sonst die
/// erste der Art), sonst neu angelegt (lesend, Datenklasse `internal`, E7). Ein
/// vom Nutzer geaenderter Name oder ausgeschalteter Schalter bleibt unberuehrt.
pub fn ensure_integration(conn: &Connection, now_ms: i64) -> Result<Integration, YoutubeError> {
    if let Some(found) = register::get(conn, INTEGRATION_ID)? {
        if found.kind == Kind::Youtube {
            return Ok(found);
        }
        return Err(YoutubeError::Store(
            "Die Kennung der YouTube-Integration ist anderweitig vergeben.".to_string(),
        ));
    }
    if let Some(other) = register::list(conn)?
        .into_iter()
        .find(|i| i.kind == Kind::Youtube)
    {
        return Ok(other);
    }
    let mut new = NewIntegration::new(Kind::Youtube, "YouTube");
    new.id = Some(INTEGRATION_ID.to_string());
    new.direction = Some(Direction::Read);
    new.account_hint = Some("youtube.com".to_string());
    new.data_class = Some("internal".to_string());
    match register::create(conn, &new, now_ms) {
        Ok(created) => Ok(created),
        // Ein zweiter Aufrufer war schneller: dessen Eintrag gilt.
        Err(IntegrationError::Invalid(_)) => register::get(conn, INTEGRATION_ID)?
            .ok_or_else(|| YoutubeError::Store("Integration nicht angelegt".to_string())),
        Err(e) => Err(e.into()),
    }
}

fn project_exists(conn: &Connection, project_id: &str) -> Result<bool, YoutubeError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM meeting_folders WHERE id = ?1 AND deleted_at IS NULL)",
        params![project_id],
        |r| r.get(0),
    )?)
}

// ---------------------------------------------------------------------------
// Anlegen
// ---------------------------------------------------------------------------

/// Besprechung, Projektzuordnung und Provenienz in EINER Transaktion: alles oder
/// nichts. Rueckgabe: die ID der Besprechung.
#[cfg(test)]
pub fn create_meeting(
    conn: &mut Connection,
    video: &YoutubeRef,
    meta: &VideoMeta,
    project_id: Option<&str>,
    now_ms: i64,
) -> Result<String, YoutubeError> {
    create_meeting_into(conn, video, meta, project_id, None, now_ms)
}

/// Wie `create_meeting`; mit `target` (G1, #70) wird statt einer neuen Besprechung
/// ein vorhandener LEERER Eintrag zur YouTube-Besprechung: Id, Projekte und
/// Notizen bleiben, `project_id` entfaellt, der Titel wird nur ersetzt, solange
/// er noch der vorgeschlagene ist. Ein anderes Ziel bricht mit
/// `Target("target_not_empty")` ab (nichts geschrieben). Der Nutzer ist der Anleger; die
/// App ruft `create_meeting_for` (A8), diese Kurzform gibt es fuer die Tests.
#[cfg(test)]
pub fn create_meeting_into(
    conn: &mut Connection,
    video: &YoutubeRef,
    meta: &VideoMeta,
    project_id: Option<&str>,
    target: Option<&str>,
    now_ms: i64,
) -> Result<String, YoutubeError> {
    create_meeting_for(conn, video, meta, project_id, target, now_ms, &Origin::user())
}

/// Wie `create_meeting_into`, mit Angabe, wer anlegt (A8).
pub fn create_meeting_for(
    conn: &mut Connection,
    video: &YoutubeRef,
    meta: &VideoMeta,
    project_id: Option<&str>,
    target: Option<&str>,
    now_ms: i64,
    origin: &Origin,
) -> Result<String, YoutubeError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if target.is_none() {
        if let Some(project) = project_id {
            if !project_exists(&tx, project)? {
                return Err(YoutubeError::Project);
            }
        }
    }
    let secs = now_ms / 1000;
    let url = video.canonical_url();
    let youtube = json!({
        "video_id": video.video_id,
        "url": url,
        "title": meta.title,
        "channel": meta.channel,
        "channel_url": meta.channel_url,
        "thumbnail_url": meta.thumbnail_url,
        "start_s": video.start_s,
        "added_at": now_ms,
    });
    let id = match target {
        Some(target_id) => {
            let mut fill = EmptyFill::new(MeetingSource::Youtube, MeetingStatus::Ready);
            fill.title = Some(&meta.title);
            fill.only_if_default = true;
            fill.metadata = Some((METADATA_KEY, youtube));
            fill_empty_tx(&tx, target_id, &fill).map_err(|e| match e.to_string().as_str() {
                "meeting_not_found" => YoutubeError::Target("meeting_not_found"),
                "target_not_empty" => YoutubeError::Target("target_not_empty"),
                _ => YoutubeError::Store(e.to_string()),
            })?;
            target_id.to_string()
        }
        None => {
            let id = Ulid::new().to_string();
            let metadata = json!({ METADATA_KEY: youtube });
            tx.execute(
                "INSERT INTO meetings (id, title, status, source, consent_confirmed_at, created_at,
                                       updated_at, metadata_json)
                 VALUES (?1, ?2, 'ready', ?3, NULL, ?4, ?4, ?5)",
                params![
                    id,
                    meta.title,
                    MeetingSource::Youtube.as_str(),
                    secs,
                    metadata.to_string()
                ],
            )?;
            if let Some(project) = project_id {
                tx.execute(
                    "INSERT INTO meeting_folder_items (folder_id, meeting_id, added_at)
                     VALUES (?1, ?2, ?3)",
                    params![project, id, secs],
                )?;
            }
            id
        }
    };
    // Die Herkunft des (kuenftigen) Transkripts: dieser Link. Untertitel und
    // eigene Transkription (A3) haengen ihre Eintraege dahinter an.
    let mut source = SourceRef::new("youtube", &video.video_id, Some(&meta.title));
    source.url = Some(url);
    let mut entry = NewProvenance::new(
        SubjectKind::Transcript,
        &id,
        "youtube_source",
        origin.actor,
    );
    entry.actor_ref = origin.actor_ref.clone();
    entry.provider = Some("youtube".to_string());
    entry.sources = vec![source];
    entry.params = Some(json!({ "channel": meta.channel, "start_s": video.start_s }));
    provenance::record_at(&tx, &entry, now_ms).map_err(|e| YoutubeError::Store(e.to_string()))?;
    tx.commit()?;
    Ok(id)
}

/// Schritt 3: Projekt, Register, Tor und Audit VOR dem Netz. `Err` heisst: nichts
/// geht ins Netz.
fn prepare(
    store: &MeetingStore,
    video: &YoutubeRef,
    project_id: Option<&str>,
    target: Option<&str>,
    now_ms: i64,
    origin: &Origin,
) -> Result<(String, i64), YoutubeError> {
    let conn = store
        .get_connection()
        .map_err(|e| YoutubeError::Store(e.to_string()))?;
    match target {
        // G1: ein Ziel muss ein leerer Eintrag sein, bevor etwas ins Netz geht.
        Some(target_id) => {
            let source: Option<String> = conn
                .query_row(
                    "SELECT source FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                    params![target_id],
                    |r| r.get(0),
                )
                .optional()?;
            match source.as_deref() {
                None => return Err(YoutubeError::Target("meeting_not_found")),
                Some(crate::managers::meetings::empty::SOURCE_EMPTY) => {}
                Some(_) => return Err(YoutubeError::Target("target_not_empty")),
            }
        }
        None => {
            if let Some(project) = project_id {
                if !project_exists(&conn, project)? {
                    return Err(YoutubeError::Project);
                }
            }
        }
    }
    let integration = ensure_integration(&conn, now_ms)?;
    let decision = gate::check(
        &conn,
        &Request {
            caller: Caller::User,
            integration_id: &integration.id,
            capability: Capability::MediaFetch,
            target: None,
            args: None,
            tool_mode: None,
        },
        now_ms,
    )?;
    match decision {
        Decision::Allowed => {}
        Decision::Denied { reason, .. } => return Err(YoutubeError::Disabled(reason)),
        Decision::NeedsApproval { .. } => {
            return Err(YoutubeError::Disabled(
                "Für diesen Schritt ist eine Freigabe nötig.".to_string(),
            ))
        }
    }
    let audit_id = audit::record_at(
        &conn,
        &NewAudit {
            caller: origin.actor.as_str().to_string(),
            integration_id: Some(integration.id.clone()),
            capability: Some(Capability::MediaFetch.as_str().to_string()),
            target: Some(format!("youtube:{}", video.video_id)),
            outcome: AuditOutcome::Pending,
            detail: Some(match &origin.actor_ref {
                Some(client) => json!({ "phase": "oembed", "client": client }),
                None => json!({ "phase": "oembed" }),
            }),
        },
        now_ms,
    )?;
    Ok((integration.id, audit_id))
}

/// Schritt 5: Ergebnis im Audit und im Register festhalten. Ein Fehler hier ist
/// nur eine Warnung: der Abruf ist gelaufen, der Eintrag bleibt dann `pending`.
fn finish(
    store: &MeetingStore,
    integration_id: &str,
    audit_id: i64,
    outcome: &Result<VideoMeta, YoutubeError>,
    now_ms: i64,
) {
    let conn = match store.get_connection() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("youtube: Ergebnis nicht festgehalten: {e}");
            return;
        }
    };
    let (audit_outcome, detail) = match outcome {
        Ok(_) => (AuditOutcome::Ok, json!({ "phase": "done" })),
        Err(e) => (
            AuditOutcome::Error,
            json!({ "phase": "done", "error": e.code() }),
        ),
    };
    if let Err(e) = audit::set_outcome(&conn, audit_id, audit_outcome, Some(detail)) {
        log::warn!("youtube: Audit-Ergebnis nicht gesetzt (Eintrag {audit_id}): {e}");
    }
    let marked = match outcome {
        Ok(_) => register::mark_ok(&conn, integration_id, now_ms),
        Err(e) => register::mark_error(&conn, integration_id, &e.to_string(), now_ms),
    };
    if let Err(e) = marked {
        log::warn!("youtube: Register nicht aktualisiert: {e}");
    }
}

/// Legt aus einem eingefuegten Link eine Besprechung mit Quelle `youtube` an
/// (siehe Moduldoku fuer den Ablauf).
pub async fn add_youtube_source(
    store: &MeetingStore,
    raw_url: &str,
    project_id: Option<&str>,
    opts: &AddOptions,
) -> Result<AddedSource, YoutubeError> {
    add_youtube_source_into(store, raw_url, project_id, None, opts).await
}

/// Wie `add_youtube_source`; mit `target` (G1, #70) fuellt der Link einen
/// vorhandenen LEEREN Eintrag (`create_meeting_into`). Ein anderes Ziel wird
/// abgewiesen, BEVOR etwas ins Netz geht (Tor, Audit und oEmbed bleiben wie sonst).
pub async fn add_youtube_source_into(
    store: &MeetingStore,
    raw_url: &str,
    project_id: Option<&str>,
    target: Option<&str>,
    opts: &AddOptions,
) -> Result<AddedSource, YoutubeError> {
    add_youtube_source_for(store, raw_url, project_id, target, opts, &Origin::user()).await
}

/// Wie `add_youtube_source_into`, mit Angabe, wer anlegt (A8: ein externer Agent). Audit-Aufrufer
/// und Herkunft folgen `origin`; sonst ist alles gleich (kein Netz ausser dem einen
/// oEmbed-Abruf, nie yt-dlp).
pub async fn add_youtube_source_for(
    store: &MeetingStore,
    raw_url: &str,
    project_id: Option<&str>,
    target: Option<&str>,
    opts: &AddOptions,
    origin: &Origin,
) -> Result<AddedSource, YoutubeError> {
    let video = normalize_link(raw_url)?;
    let _flight = acquire(store, &video.video_id)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let (integration_id, audit_id) = prepare(store, &video, project_id, target, now_ms, origin)?;

    let fetched = oembed::fetch(&opts.oembed_base, &video, &opts.fetch).await;
    finish(
        store,
        &integration_id,
        audit_id,
        &fetched,
        chrono::Utc::now().timestamp_millis(),
    );
    let meta = fetched?;

    let mut conn = store
        .get_connection()
        .map_err(|e| YoutubeError::Store(e.to_string()))?;
    let id = create_meeting_for(
        &mut conn,
        &video,
        &meta,
        project_id,
        target,
        chrono::Utc::now().timestamp_millis(),
        origin,
    )?;
    drop(conn);
    let meeting = store
        .get_meeting(&id)
        .map_err(|e| YoutubeError::Store(e.to_string()))?
        .ok_or_else(|| YoutubeError::Store("Besprechung nicht gefunden".to_string()))?;
    log::info!(
        "youtube: Quelle {} angelegt (Besprechung {})",
        video.video_id,
        id
    );
    Ok(AddedSource {
        meeting,
        meta,
        video,
    })
}

// ---------------------------------------------------------------------------
// Lesen
// ---------------------------------------------------------------------------

fn text(object: &Value, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Die Quelle einer Besprechung, oder `None`: andere Quelle, unbekannte oder
/// beschaedigte Daten (eine kaputte Angabe darf die Oberflaeche nie stoeren).
pub fn read_source(
    store: &MeetingStore,
    meeting_id: &str,
) -> Result<Option<YoutubeSource>, YoutubeError> {
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| YoutubeError::Store(e.to_string()))?;
    let Some(meeting) = meeting.filter(|m| m.source == SOURCE_KIND) else {
        return Ok(None);
    };
    let metadata = match store.metadata_json(&meeting.id) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("youtube: metadata_json nicht lesbar: {e}");
            return Ok(None);
        }
    };
    let Some(object) = metadata
        .as_ref()
        .and_then(|m| m.get(METADATA_KEY))
        .filter(|v| v.is_object())
    else {
        return Ok(None);
    };
    let Some(video_id) = text(object, "video_id").filter(|id| valid_video_id(id)) else {
        return Ok(None);
    };
    let Some(title) = text(object, "title") else {
        return Ok(None);
    };
    Ok(Some(YoutubeSource {
        url: format!("https://www.youtube.com/watch?v={video_id}"),
        video_id,
        title,
        channel: text(object, "channel").unwrap_or_default(),
        channel_url: text(object, "channel_url"),
        thumbnail_url: text(object, "thumbnail_url"),
        start_s: object
            .get("start_s")
            .and_then(Value::as_u64)
            .and_then(|s| u32::try_from(s).ok()),
    }))
}
