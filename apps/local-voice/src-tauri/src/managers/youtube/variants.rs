//! Untertitel und Dauer einer YouTube-Besprechung als Fassung bzw. Angabe (A3).
//!
//! Die Ablaeufe verbinden die Bausteine: Zugang pruefen (`fetch::resolve`), Tor und
//! Audit (`fetch::begin`/`end`), yt-dlp laufen lassen (`subtitles`), die Fassung
//! anlegen (`meetings::variants`) und ihre Herkunft festhalten. Alles blockierend;
//! die Commands rufen es aus `spawn_blocking`.

use std::sync::atomic::AtomicBool;

use rusqlite::params;
use serde_json::json;

use super::fetch::{self, ToolRun};
use super::source::read_source;
use super::subtitles::{self, SubtitleTrack};
use super::tool::ToolStatus;
use super::{YoutubeError, SOURCE_KIND};
use crate::managers::meetings::store::{MeetingStore, StoredSegment};
use crate::managers::meetings::variants::{self as fassungen, NewVariant};
use crate::managers::provenance::{self, ActorKind, NewProvenance, SourceRef, SubjectKind};

/// Laengste plausible Videodauer, die der Player nachtragen darf (7 Tage).
const MAX_DURATION_S: f64 = 7.0 * 24.0 * 3600.0;

fn video_id_of(store: &MeetingStore, meeting_id: &str) -> Result<String, YoutubeError> {
    read_source(store, meeting_id)?
        .map(|s| s.video_id)
        .ok_or_else(|| YoutubeError::BadResponse("Keine YouTube-Besprechung.".to_string()))
}

/// Welche Untertitelspuren hat das Video? Ein Seitenabruf ueber yt-dlp (im Audit).
pub fn tracks(
    store: &MeetingStore,
    meeting_id: &str,
    private: bool,
    status: &ToolStatus,
    cancel: Option<&AtomicBool>,
    temp_base: Option<&std::path::Path>,
) -> Result<Vec<SubtitleTrack>, YoutubeError> {
    let video_id = video_id_of(store, meeting_id)?;
    let exe = fetch::resolve(private, status)?;
    let guard = fetch::begin(store, &video_id, "subtitles_list")?;
    let result = subtitles::list_tracks(
        &ToolRun {
            exe: &exe,
            temp_base,
        },
        &video_id,
        cancel,
    );
    fetch::end(
        store,
        guard,
        &result.as_ref().map(|_| ()).map_err(Clone::clone),
    );
    result
}

/// Laedt eine Spur und legt sie als Fassung an (nicht aktiv, ausser es ist die erste).
/// Gibt die Kennung der Fassung zurueck.
pub fn fetch_subtitles(
    store: &MeetingStore,
    meeting_id: &str,
    private: bool,
    status: &ToolStatus,
    track: &SubtitleTrack,
    cancel: Option<&AtomicBool>,
    temp_base: Option<&std::path::Path>,
) -> Result<String, YoutubeError> {
    let video_id = video_id_of(store, meeting_id)?;
    let exe = fetch::resolve(private, status)?;
    let guard = fetch::begin(store, &video_id, "subtitles")?;
    let fetched = subtitles::fetch_track(
        &ToolRun {
            exe: &exe,
            temp_base,
        },
        &video_id,
        track,
        cancel,
    );
    fetch::end(
        store,
        guard,
        &fetched.as_ref().map(|_| ()).map_err(Clone::clone),
    );
    let segments = fetched?;
    add_subtitle_variant(
        store,
        meeting_id,
        &video_id,
        track,
        segments,
        status.version.as_deref(),
    )
}

/// Legt Untertitel als Fassung an und schreibt ihre Herkunft. Die Provenienz darf
/// das Anlegen nie scheitern lassen.
pub fn add_subtitle_variant(
    store: &MeetingStore,
    meeting_id: &str,
    video_id: &str,
    track: &SubtitleTrack,
    segments: Vec<StoredSegment>,
    tool_version: Option<&str>,
) -> Result<String, YoutubeError> {
    let count = segments.len();
    let kind = if track.auto {
        fassungen::KIND_SUBTITLES_AUTO
    } else {
        fassungen::KIND_SUBTITLES_MANUAL
    };
    let mut conn = store
        .get_connection()
        .map_err(|e| YoutubeError::Store(e.to_string()))?;
    let id = fassungen::add(
        &mut conn,
        NewVariant {
            meeting_id: meeting_id.to_string(),
            kind,
            language: Some(track.language.clone()),
            model: None,
            segments,
            activate: false,
        },
    )
    .map_err(|e| YoutubeError::Store(e.to_string()))?;
    let mut source = SourceRef::new("subtitle", video_id, Some(&track.name));
    source.url = Some(subtitles::watch_url(video_id));
    let mut entry = NewProvenance::new(
        SubjectKind::TranscriptVariant,
        &id,
        "subtitles_import",
        ActorKind::User,
    );
    entry.provider = Some("youtube".to_string());
    entry.sources = vec![source];
    entry.params = Some(json!({
        "language": track.language,
        "auto": track.auto,
        "segments": count,
        "tool": "yt-dlp",
        "tool_version": tool_version,
    }));
    if let Err(e) = provenance::record(&conn, &entry) {
        log::warn!("Provenienz (Untertitel) nicht geschrieben: {e}");
    }
    Ok(id)
}

/// Die Videodauer aus dem Player nachtragen, wenn sie fehlt. `true`: geschrieben.
/// Nur fuer YouTube-Besprechungen, nur sinnvolle Werte, nie ein Ueberschreiben.
pub fn set_duration_if_missing(
    store: &MeetingStore,
    meeting_id: &str,
    seconds: f64,
) -> Result<bool, YoutubeError> {
    if !seconds.is_finite() || !(1.0..=MAX_DURATION_S).contains(&seconds) {
        return Ok(false);
    }
    let conn = store
        .get_connection()
        .map_err(|e| YoutubeError::Store(e.to_string()))?;
    let changed = conn.execute(
        "UPDATE meetings SET duration_ms = ?1, updated_at = ?2
         WHERE id = ?3 AND source = ?4 AND duration_ms IS NULL AND deleted_at IS NULL",
        params![
            (seconds * 1000.0).round() as i64,
            chrono::Utc::now().timestamp(),
            meeting_id,
            SOURCE_KIND
        ],
    )?;
    Ok(changed > 0)
}
