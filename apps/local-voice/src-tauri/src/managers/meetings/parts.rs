//! Mehrere Audiodateien (Teile) in EINER Besprechung, z. B. die Aufnahmen vor
//! und nach einer Pause.
//!
//! Entscheidung: die Teile werden PHYSISCH zusammengefuehrt. Jede Datei laeuft
//! wie bisher als eigener Auftrag durch die Import-Warteschlange (parallel,
//! Fortschritt, Stopp, Fortsetzen, Absturz-Wiederherstellung bleiben, wie sie
//! sind). Sobald alle Teile fertig sind, entsteht in der ERSTEN Besprechung
//! (dem Ziel) je Spur eine durchgehende WAV, das Transkript wird zu einer
//! fortlaufenden Zeitachse (Teil 2 beginnt bei der Laenge von Teil 1), die
//! angehaengten Besprechungen gehen in den Papierkorb (weich geloescht).
//! Danach laeuft die Sprechertrennung einmal ueber die ganze Besprechung.
//!
//! Warum so und nicht als Teilliste mit eigenen Dateien: jeder Leser des
//! Transkripts (Player, Sprung zur Zeitmarke, Sprecher, Notizen, Protokoll,
//! Suche, Export, MCP) kennt genau EINE Zeitachse und eine Datei je Spur. Mit
//! der physischen Zusammenfuehrung bleibt das so; nichts davon muss Teile kennen.
//! Die Teilliste (`meeting_parts`) ist nur Anzeige: welche Datei bei welcher
//! Zeitmarke beginnt.
//!
//! Zwei Wege fuehren hierher:
//! - **Anhaengen** (`append_pending`): beim Import mehrerer Dateien „als eine
//!   Besprechung“ oder nachtraeglich „Audiodatei anhaengen“. Die angehaengte
//!   Datei bekommt eine eigene Besprechung in der Warteschlange und eine Zeile
//!   in `meeting_append_pending`. Ist der letzte Teil fertig, fuehrt
//!   [`after_import`] (aus der Warteschlange) die Gruppe zusammen.
//! - **Zusammenfuehren von Hand**: Mehrfachauswahl in der Liste, Reihenfolge
//!   nach Aufnahmebeginn ([`merge_now`]).
//!
//! Absicherung:
//! - Alles in der Datenbank geschieht in EINER IMMEDIATE-Transaktion
//!   ([`MeetingStore::apply_merge`]): entweder ist die Besprechung vollstaendig
//!   zusammengefuehrt oder nichts hat sich geaendert. Die neuen WAVs entstehen
//!   VORHER unter eigenem Namen; scheitert die Transaktion, werden sie entfernt.
//! - Jede beteiligte Besprechung muss `ready` sein, ihr Transkript darf sich seit
//!   dem Lesen nicht geaendert haben (`content_revision`), und sie darf nur eine
//!   Transkript-Fassung haben (weitere Fassungen passten nicht zur neuen Zeitachse).
//! - Sprecher: je Kanal bekommen spaetere Teile einen Versatz (Teil 2 „Person 1“
//!   wird z. B. Person 4), Namen wandern mit; Index 0 („ganzer Kanal“) bleibt ohne
//!   Versatz. Die gespeicherten Turns werden ebenso verschoben, so dass die neue
//!   Sprechertrennung die Namen ueber `remap_speakers` weitertraegt.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::Utc;
use log::{info, warn};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use ulid::Ulid;

use super::notes::model::NoteBlock;
use super::store::{Meeting, MeetingStore, SpeakerRow, StoredSegment};

#[cfg(test)]
mod tests;

/// Migration Index 15. Nur CREATE ... IF NOT EXISTS: vorhandene Zeilen bleiben
/// unveraendert.
///
/// - `meeting_parts`: Anzeige der Teile einer zusammengefuehrten Besprechung
///   (Titel, Quelldatei, Beginn auf der Zeitachse, Laenge). Eine Besprechung ohne
///   Zeilen besteht aus einem Teil.
/// - `meeting_append_pending`: diese Besprechung (ein Teil in der Warteschlange)
///   wird an `target_id` angehaengt, sobald alle Teile fertig sind; `position`
///   ordnet die Teile hinter dem Ziel.
pub const PARTS_MIGRATION: &str = "CREATE TABLE IF NOT EXISTS meeting_parts (
      meeting_id TEXT NOT NULL, part_index INTEGER NOT NULL, title TEXT NOT NULL,
      source_path TEXT, offset_ms INTEGER NOT NULL, duration_ms INTEGER NOT NULL,
      created_at INTEGER NOT NULL, PRIMARY KEY (meeting_id, part_index));
    CREATE TABLE IF NOT EXISTS meeting_append_pending (
      meeting_id TEXT PRIMARY KEY, target_id TEXT NOT NULL, position INTEGER NOT NULL,
      created_at INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS idx_append_pending_target
      ON meeting_append_pending(target_id, position);";

/// Abtastrate aller Besprechungs-WAVs (Import und Aufnahme).
const RATE: u32 = 16_000;
/// Schlüssel in `metadata_json`, den das Umbenennen von Sprechern pflegt
/// („Kanal:Index“). Er passt nach dem Versatz nicht mehr und entfaellt.
const FORMER_NAMES_KEY: &str = "speaker_former_names";
/// Hoechstens so viele Teile in einer Besprechung (Schutz vor Fehlbedienung).
pub const MAX_PARTS: usize = 64;

/// Ein Teil einer Besprechung (Anzeige).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MeetingPart {
    pub part_index: u32,
    pub title: String,
    pub source_path: Option<String>,
    /// Beginn des Teils auf der Zeitachse der Besprechung.
    pub offset_ms: u64,
    pub duration_ms: u64,
}

/// Ein Teil, der noch verarbeitet wird und danach angehaengt wird.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PendingPart {
    pub meeting_id: String,
    pub title: String,
    pub status: String,
    pub position: i64,
}

/// Alles, was die Oberflaeche ueber die Teile einer Besprechung braucht.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PartsInfo {
    /// Die zusammengefuehrten Teile (leer = ein einziger Teil).
    pub parts: Vec<MeetingPart>,
    /// Teile, die noch laufen oder warten und danach angehaengt werden.
    pub pending: Vec<PendingPart>,
    /// Diese Besprechung ist selbst ein wartender Teil: Ziel (ID, Titel).
    pub append_to: Option<(String, String)>,
}

/// Warum nicht zusammengefuehrt wird (Code fuer `meetingErrors.ts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartsError {
    /// Weniger als zwei Besprechungen oder eine doppelt.
    TooFew,
    TooMany,
    NotFound(String),
    /// Eine Besprechung ist noch nicht fertig (laeuft, wartet, gescheitert).
    NotReady(String),
    /// Untertitel, YouTube oder leerer Eintrag: kein Audio.
    NoAudio(String),
    /// Die Audiodatei fehlt (Aufbewahrungsfrist abgelaufen, geloescht).
    AudioMissing(String),
    /// Aufnahme und Import gemischt.
    MixedSources,
    /// Mehr als eine Transkript-Fassung.
    HasVariants(String),
    /// Das Transkript hat sich waehrend des Zusammenfuehrens geaendert.
    Changed,
    /// Gerade laeuft eine andere Verarbeitung.
    Busy,
    /// Lesen, Schreiben, Datenbank.
    Io(String),
}

impl PartsError {
    pub fn code(&self) -> &'static str {
        match self {
            PartsError::TooFew => "parts_too_few",
            PartsError::TooMany => "parts_too_many",
            PartsError::NotFound(_) => "meeting_not_found",
            PartsError::NotReady(_) => "parts_not_ready",
            PartsError::NoAudio(_) => "parts_no_audio",
            PartsError::AudioMissing(_) => "parts_audio_missing",
            PartsError::MixedSources => "parts_mixed_sources",
            PartsError::HasVariants(_) => "parts_has_variants",
            PartsError::Changed => "parts_changed",
            PartsError::Busy => "parts_busy",
            PartsError::Io(_) => "parts_failed",
        }
    }
}

impl std::fmt::Display for PartsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartsError::Io(e) => write!(f, "{}: {e}", self.code()),
            other => write!(f, "{}", other.code()),
        }
    }
}

impl std::error::Error for PartsError {}

// ---------------------------------------------------------------------------
// Plan (rein, ohne Dateien und Datenbank)
// ---------------------------------------------------------------------------

/// Was ein Teil zur Zusammenfuehrung beitraegt (gelesen aus dem Store).
#[derive(Clone, Debug)]
pub struct PartSource {
    pub meeting: Meeting,
    pub segments: Vec<StoredSegment>,
    /// `content_revision` beim Lesen (Pruefung in der Transaktion).
    pub revision: i64,
    pub hints: Option<String>,
    pub speakers: Vec<SpeakerRow>,
    pub notes: Vec<NoteBlock>,
    /// Die Teile, aus denen dieser Teil selbst schon besteht (leer = einer).
    pub parts: Vec<MeetingPart>,
    /// Laenge des Audios dieses Teils (laengste Spur).
    pub length_ms: u64,
}

/// Das Ergebnis fuer die Zielbesprechung.
#[derive(Clone, Debug, PartialEq)]
pub struct MergePlan {
    pub segments: Vec<StoredSegment>,
    pub hints: Option<String>,
    pub speakers: Vec<SpeakerRow>,
    pub notes: Vec<NoteBlock>,
    pub parts: Vec<MeetingPart>,
    pub duration_ms: u64,
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Turns je Kanal aus `speaker_hints_json` (wie `speakers::turns_from_hints`).
fn turns_of(hints: Option<&str>) -> BTreeMap<u8, Vec<(u64, u64, u32)>> {
    let Some(v) = hints.and_then(|t| serde_json::from_str::<Value>(t).ok()) else {
        return BTreeMap::new();
    };
    super::speakers::turns_from_hints(&v)
        .into_iter()
        .map(|(c, turns)| {
            (
                c,
                turns
                    .into_iter()
                    .map(|t| (t.start_ms, t.end_ms, t.speaker))
                    .collect(),
            )
        })
        .collect()
}

/// Die Teile in der gegebenen Reihenfolge zu einer Zeitachse.
pub fn plan(sources: &[PartSource]) -> MergePlan {
    let mut segments = Vec::new();
    let mut notes = Vec::new();
    let mut parts = Vec::new();
    let mut speakers: Vec<SpeakerRow> = Vec::new();
    let mut turns: BTreeMap<u8, Vec<(u64, u64, u32)>> = BTreeMap::new();
    // Je Kanal: hoechste bisher vergebene Sprechernummer (Versatz des naechsten Teils).
    let mut base: HashMap<u8, u32> = HashMap::new();
    let mut hints_root: Option<serde_json::Map<String, Value>> = None;
    let mut offset = 0u64;

    for src in sources {
        let start = base.clone();
        let shift_of = |channel: u8| start.get(&channel).copied().unwrap_or(0);
        let shifted = |channel: u8, n: u32| if n == 0 { 0 } else { n + shift_of(channel) };
        let mut seen: HashMap<u8, u32> = HashMap::new();
        let mut note = |channel: u8, n: u32| {
            let e = seen.entry(channel).or_insert(0);
            *e = (*e).max(n);
        };

        for s in &src.segments {
            let mut s = s.clone();
            s.start_ms += offset;
            s.end_ms += offset;
            if let Some(words) = s.words.as_mut() {
                for w in words.iter_mut() {
                    w.start_ms += offset;
                    w.end_ms += offset;
                }
            }
            if let Some(n) = s.speaker_index {
                note(s.channel, n);
                s.speaker_index = Some(shifted(s.channel, n));
            }
            segments.push(s);
        }

        for (channel, list) in turns_of(src.hints.as_deref()) {
            let out = turns.entry(channel).or_default();
            for (start, end, speaker) in list {
                note(channel, speaker);
                out.push((start + offset, end + offset, shifted(channel, speaker)));
            }
        }
        if hints_root.is_none() {
            hints_root = src
                .hints
                .as_deref()
                .and_then(|t| serde_json::from_str::<Value>(t).ok())
                .and_then(|v| match v {
                    Value::Object(m) => Some(m),
                    _ => None,
                });
        }

        for row in &src.speakers {
            note(row.channel, row.speaker_index);
            let index = shifted(row.channel, row.speaker_index);
            if let Some(existing) = speakers
                .iter_mut()
                .find(|r| r.channel == row.channel && r.speaker_index == index)
            {
                // Nur Index 0 kann doppelt sein: der erste Name gilt.
                if existing.display_name.is_none() {
                    existing.display_name = row.display_name.clone();
                    existing.human_id = row.human_id.clone();
                    existing.consent_state = row.consent_state.clone();
                }
                continue;
            }
            speakers.push(SpeakerRow {
                speaker_index: index,
                ..row.clone()
            });
        }

        for block in &src.notes {
            let mut block = block.clone();
            block.at_ms = block.at_ms.map(|ms| ms + offset);
            notes.push(block);
        }

        if src.parts.is_empty() {
            let title = src
                .meeting
                .source_path
                .as_deref()
                .map(file_name)
                .unwrap_or_else(|| src.meeting.title.clone());
            parts.push(MeetingPart {
                part_index: 0,
                title,
                source_path: src.meeting.source_path.clone(),
                offset_ms: offset,
                duration_ms: src.length_ms,
            });
        } else {
            for p in &src.parts {
                parts.push(MeetingPart {
                    offset_ms: p.offset_ms + offset,
                    ..p.clone()
                });
            }
        }

        for (channel, max) in seen {
            let next = shift_of(channel) + max;
            let e = base.entry(channel).or_insert(0);
            *e = (*e).max(next);
        }
        offset += src.length_ms;
    }

    for (i, s) in segments.iter_mut().enumerate() {
        s.segment_index = i as u32;
    }
    for (i, p) in parts.iter_mut().enumerate() {
        p.part_index = i as u32;
    }
    speakers.sort_by_key(|r| (r.channel, r.speaker_index));

    let hints = (!turns.is_empty()).then(|| {
        let mut root = hints_root.unwrap_or_default();
        let channels: serde_json::Map<String, Value> = turns
            .iter()
            .map(|(c, list)| {
                (
                    c.to_string(),
                    Value::Array(
                        list.iter()
                            .map(|(s, e, n)| serde_json::json!([s, e, n]))
                            .collect(),
                    ),
                )
            })
            .collect();
        root.insert("channels".into(), Value::Object(channels));
        Value::Object(root).to_string()
    });

    MergePlan {
        segments,
        hints,
        speakers,
        notes,
        parts,
        duration_ms: offset,
    }
}

/// Reihenfolge fuer das Zusammenfuehren von Hand: nach Aufnahmebeginn
/// (`started_at`, sonst Anlage), bei Gleichstand nach Quelldatei (Aufnahme 112
/// vor 113) und ID.
pub fn chronological(meetings: &mut [Meeting]) {
    meetings.sort_by(|a, b| {
        let ka = a.started_at.unwrap_or(a.created_at);
        let kb = b.started_at.unwrap_or(b.created_at);
        ka.cmp(&kb)
            .then_with(|| natural_key(a.source_path.as_deref()).cmp(&natural_key(b.source_path.as_deref())))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Dateiname mit Zahlen als Zahlen („Aufnahme 9“ vor „Aufnahme 10“).
fn natural_key(path: Option<&str>) -> Vec<(String, u64)> {
    let name = path.map(file_name).unwrap_or_default().to_lowercase();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push((std::mem::take(&mut text), digits.parse().unwrap_or(u64::MAX)));
                digits.clear();
            }
            text.push(c);
        }
    }
    out.push((text, digits.parse().unwrap_or(0)));
    out
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// Eine Spur als 16-kHz-Mono-Samples: direkt gestreamt, wenn die Datei schon so
/// vorliegt (Normalfall), sonst ganz gelesen und umgerechnet.
enum Track {
    Stream(hound::WavReader<std::io::BufReader<std::fs::File>>, u64),
    Loaded(Vec<i16>),
}

impl Track {
    fn open(path: &Path) -> Result<Self, String> {
        let reader = hound::WavReader::open(path).map_err(|e| format!("wav_unreadable: {e}"))?;
        let spec = reader.spec();
        if spec.sample_rate == RATE
            && spec.channels == 1
            && spec.sample_format == hound::SampleFormat::Int
            && spec.bits_per_sample == 16
        {
            let len = u64::from(reader.duration());
            return Ok(Track::Stream(reader, len));
        }
        drop(reader);
        super::import::read_wav_i16_mono_16k(path).map(Track::Loaded)
    }

    fn len(&self) -> u64 {
        match self {
            Track::Stream(_, len) => *len,
            Track::Loaded(v) => v.len() as u64,
        }
    }

    fn copy_to(self, out: &mut WavOut) -> Result<u64, String> {
        let mut written = 0u64;
        match self {
            Track::Stream(mut reader, _) => {
                for s in reader.samples::<i16>() {
                    out.write(s.map_err(|e| format!("wav_unreadable: {e}"))?)?;
                    written += 1;
                }
            }
            Track::Loaded(v) => {
                for s in v {
                    out.write(s)?;
                    written += 1;
                }
            }
        }
        Ok(written)
    }
}

struct WavOut(hound::WavWriter<std::io::BufWriter<std::fs::File>>);

impl WavOut {
    fn create(path: &Path) -> Result<Self, String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        hound::WavWriter::create(path, spec)
            .map(WavOut)
            .map_err(|e| format!("wav_write_failed: {e}"))
    }

    fn write(&mut self, s: i16) -> Result<(), String> {
        self.0
            .write_sample(s)
            .map_err(|e| format!("wav_write_failed: {e}"))
    }

    fn silence(&mut self, n: u64) -> Result<(), String> {
        for _ in 0..n {
            self.write(0)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<(), String> {
        self.0
            .finalize()
            .map_err(|e| format!("wav_write_failed: {e}"))
    }
}

/// Ergebnis von [`concat_audio`].
pub struct ConcatAudio {
    pub mic: PathBuf,
    pub system: Option<PathBuf>,
    /// Laenge je Teil in ms (laengste Spur des Teils).
    pub lengths_ms: Vec<u64>,
}

/// Haengt die Spuren der Teile aneinander (`dir/teile-<ulid>-mic.wav`, bei
/// Systemton auch `-system.wav`). Fehlt einem Teil eine Spur oder ist sie
/// kuerzer, wird mit Stille aufgefuellt, damit beide Spuren dieselbe Zeitachse
/// haben. Bei einem Fehler bleiben keine halben Dateien liegen.
pub fn concat_audio(
    tracks: &[(PathBuf, Option<PathBuf>)],
    dir: &Path,
) -> Result<ConcatAudio, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("meeting_dir_failed: {e}"))?;
    let stem = format!("teile-{}", Ulid::new().to_string().to_lowercase());
    let mic_path = dir.join(format!("{stem}-mic.wav"));
    let system_path = tracks
        .iter()
        .any(|(_, s)| s.is_some())
        .then(|| dir.join(format!("{stem}-system.wav")));
    let result = (|| {
        let mut mic_out = WavOut::create(&mic_path)?;
        let mut sys_out = match &system_path {
            Some(p) => Some(WavOut::create(p)?),
            None => None,
        };
        let mut lengths_ms = Vec::with_capacity(tracks.len());
        for (mic, system) in tracks {
            let mic = Track::open(mic)?;
            let system = system.as_deref().map(Track::open).transpose()?;
            let len = mic.len().max(system.as_ref().map_or(0, Track::len));
            let written = mic.copy_to(&mut mic_out)?;
            mic_out.silence(len.saturating_sub(written))?;
            if let Some(out) = sys_out.as_mut() {
                let written = match system {
                    Some(track) => track.copy_to(out)?,
                    None => 0,
                };
                out.silence(len.saturating_sub(written))?;
            }
            lengths_ms.push(len * 1_000 / u64::from(RATE));
        }
        mic_out.finish()?;
        if let Some(out) = sys_out {
            out.finish()?;
        }
        Ok(lengths_ms)
    })();
    match result {
        Ok(lengths_ms) => Ok(ConcatAudio {
            mic: mic_path,
            system: system_path,
            lengths_ms,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&mic_path);
            if let Some(p) = &system_path {
                let _ = std::fs::remove_file(p);
            }
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Was [`MeetingStore::apply_merge`] schreibt.
pub struct MergeWrite<'a> {
    pub target: &'a str,
    /// Die angehaengten Besprechungen (werden weich geloescht).
    pub members: &'a [String],
    /// Erwartete `content_revision` je Besprechung (Ziel und Teile).
    pub revisions: &'a [(String, i64)],
    pub plan: &'a MergePlan,
    pub mic: &'a str,
    pub system: Option<&'a str>,
}

impl MeetingStore {
    /// Die Teile einer Besprechung, in Reihenfolge.
    pub fn meeting_parts(&self, meeting_id: &str) -> Result<Vec<MeetingPart>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT part_index, title, source_path, offset_ms, duration_ms FROM meeting_parts
             WHERE meeting_id = ?1 ORDER BY part_index",
        )?;
        let rows = stmt
            .query_map(params![meeting_id], |row| {
                Ok(MeetingPart {
                    part_index: row.get::<_, i64>(0)?.max(0) as u32,
                    title: row.get(1)?,
                    source_path: row.get(2)?,
                    offset_ms: row.get::<_, i64>(3)?.max(0) as u64,
                    duration_ms: row.get::<_, i64>(4)?.max(0) as u64,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Merkt `meeting_id` zum Anhaengen an `target_id` vor (hinter allen
    /// schon vorgemerkten Teilen). Gibt die Position zurueck.
    pub fn append_pending_add(&self, meeting_id: &str, target_id: &str) -> Result<i64> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::ensure_meeting_is_live(&tx, target_id)?;
        let position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM meeting_append_pending WHERE target_id = ?1",
            params![target_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meeting_append_pending (meeting_id, target_id, position, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![meeting_id, target_id, position, Utc::now().timestamp()],
        )?;
        tx.commit()?;
        Ok(position)
    }

    /// Ziel, an das `meeting_id` angehaengt werden soll.
    pub fn append_target_of(&self, meeting_id: &str) -> Result<Option<String>> {
        let conn = self.get_connection()?;
        Ok(conn
            .query_row(
                "SELECT target_id FROM meeting_append_pending WHERE meeting_id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Die vorgemerkten Teile eines Ziels (lebende Besprechungen), in Reihenfolge.
    pub fn append_pending_of(&self, target_id: &str) -> Result<Vec<PendingPart>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT p.meeting_id, m.title, m.status, p.position
             FROM meeting_append_pending p JOIN meetings m ON m.id = p.meeting_id
             WHERE p.target_id = ?1 AND m.deleted_at IS NULL
             ORDER BY p.position, p.created_at",
        )?;
        let rows = stmt
            .query_map(params![target_id], |row| {
                Ok(PendingPart {
                    meeting_id: row.get(0)?,
                    title: row.get(1)?,
                    status: row.get(2)?,
                    position: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Ein Teil wird doch nicht angehaengt (Nutzer hat ihn geloest).
    pub fn append_pending_remove(&self, meeting_id: &str) -> Result<bool> {
        let conn = self.get_connection()?;
        Ok(conn.execute(
            "DELETE FROM meeting_append_pending WHERE meeting_id = ?1",
            params![meeting_id],
        )? > 0)
    }

    /// Anzahl lebender Transkript-Fassungen.
    fn variant_count(&self, meeting_id: &str) -> Result<i64> {
        let conn = self.get_connection()?;
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM transcript_variants WHERE meeting_id = ?1 AND deleted_at IS NULL",
            params![meeting_id],
            |row| row.get(0),
        )?)
    }

    /// Schreibt die Zusammenfuehrung in EINER Transaktion (siehe Moduldoku).
    /// Gibt die nicht mehr gebrauchten Audiopfade zurueck (alte Spuren des
    /// Ziels, Spuren der Teile); der Aufrufer loescht sie nach dem Erfolg.
    pub fn apply_merge(&self, w: &MergeWrite<'_>) -> std::result::Result<Vec<String>, PartsError> {
        let io = |e: &dyn std::fmt::Display| PartsError::Io(e.to_string());
        let mut conn = self.get_connection().map_err(|e| io(&e))?;
        let now = Utc::now().timestamp();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| io(&e))?;

        for (id, expected) in w.revisions {
            let row: Option<String> = tx
                .query_row(
                    "SELECT status FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                    params![id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| io(&e))?;
            match row.as_deref() {
                None => return Err(PartsError::NotFound(id.clone())),
                Some("ready") => {}
                Some(_) => return Err(PartsError::NotReady(id.clone())),
            }
            let found: i64 = tx
                .query_row(
                    "SELECT content_revision FROM transcripts WHERE meeting_id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| io(&e))?
                .unwrap_or(0);
            if found != *expected {
                return Err(PartsError::Changed);
            }
        }

        let (old_mic, old_system, description): (Option<String>, Option<String>, Option<String>) = tx
            .query_row(
                "SELECT mic_audio_path, system_audio_path, description FROM meetings WHERE id = ?1",
                params![w.target],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| io(&e))?;
        let mut old_paths: Vec<String> = [old_mic, old_system].into_iter().flatten().collect();

        // Beginn, Ende und Beschreibung ueber alle Teile.
        let mut ids: Vec<&str> = vec![w.target];
        ids.extend(w.members.iter().map(String::as_str));
        let mut started: Option<i64> = None;
        let mut ended: Option<i64> = None;
        let mut first_description = description.filter(|d| !d.trim().is_empty());
        for id in &ids {
            let (s, e, d): (Option<i64>, Option<i64>, Option<String>) = tx
                .query_row(
                    "SELECT started_at, ended_at, description FROM meetings WHERE id = ?1",
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(|e| io(&e))?;
            started = match (started, s) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            ended = match (ended, e) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            if first_description.is_none() {
                first_description = d.filter(|d| !d.trim().is_empty());
            }
        }
        tx.execute(
            "UPDATE meetings SET mic_audio_path = ?1, system_audio_path = ?2, duration_ms = ?3,
                 started_at = ?4, ended_at = ?5, description = ?6, updated_at = ?7
             WHERE id = ?8",
            params![
                w.mic,
                w.system,
                w.plan.duration_ms as i64,
                started,
                ended,
                first_description,
                now,
                w.target
            ],
        )
        .map_err(|e| io(&e))?;

        // Metadaten: frueher Namen der Sprecher passen nach dem Versatz nicht mehr.
        let meta: Option<Option<String>> = tx
            .query_row(
                "SELECT metadata_json FROM meetings WHERE id = ?1",
                params![w.target],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| io(&e))?;
        if let Some(Ok(Value::Object(mut map))) =
            meta.flatten().map(|t| serde_json::from_str::<Value>(&t))
        {
            if map.remove(FORMER_NAMES_KEY).is_some() {
                tx.execute(
                    "UPDATE meetings SET metadata_json = ?1 WHERE id = ?2",
                    params![Value::Object(map).to_string(), w.target],
                )
                .map_err(|e| io(&e))?;
            }
        }

        // Transkript: neue Segmente und Turns, Epoche und Revision + 1.
        let json = serde_json::to_string(&w.plan.segments).map_err(|e| io(&e))?;
        let transcript_id: Option<String> = tx
            .query_row(
                "SELECT id FROM transcripts WHERE meeting_id = ?1",
                params![w.target],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| io(&e))?;
        match transcript_id {
            Some(tid) => {
                tx.execute(
                    "DELETE FROM transcript_deltas WHERE transcript_id = ?1",
                    params![tid],
                )
                .map_err(|e| io(&e))?;
                tx.execute(
                    "UPDATE transcripts SET segments_json = ?1, speaker_hints_json = ?2,
                         content_revision = content_revision + 1,
                         segment_epoch = segment_epoch + 1, updated_at = ?3, deleted_at = NULL
                     WHERE id = ?4",
                    params![json, w.plan.hints, now, tid],
                )
                .map_err(|e| io(&e))?;
            }
            None => {
                tx.execute(
                    "INSERT INTO transcripts (id, meeting_id, segments_json, speaker_hints_json,
                         content_revision, segment_epoch, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, ?5)",
                    params![Ulid::new().to_string(), w.target, json, w.plan.hints, now],
                )
                .map_err(|e| io(&e))?;
            }
        }
        // Die (einzige) aktive Fassung haelt denselben Stand.
        tx.execute(
            "UPDATE transcript_variants SET segments_json = ?1, speaker_hints_json = ?2
             WHERE meeting_id = ?3 AND active = 1 AND deleted_at IS NULL",
            params![json, w.plan.hints, w.target],
        )
        .map_err(|e| io(&e))?;

        // Sprecher: alte Zeilen des Ziels weich loeschen, neue mit Versatz anlegen.
        tx.execute(
            "UPDATE speakers SET deleted_at = ?1, updated_at = ?1
             WHERE meeting_id = ?2 AND deleted_at IS NULL",
            params![now, w.target],
        )
        .map_err(|e| io(&e))?;
        for row in &w.plan.speakers {
            tx.execute(
                "INSERT INTO speakers (id, meeting_id, channel, speaker_index, human_id,
                     display_name, consent_state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    Ulid::new().to_string(),
                    w.target,
                    i64::from(row.channel),
                    i64::from(row.speaker_index),
                    row.human_id,
                    row.display_name,
                    row.consent_state,
                    now
                ],
            )
            .map_err(|e| io(&e))?;
        }

        // Notizen (eine lebende Datei je Besprechung, Revision + 1).
        let blocks = serde_json::to_string(&w.plan.notes).map_err(|e| io(&e))?;
        tx.execute(
            "INSERT INTO meeting_notes (meeting_id, blocks_json, revision, created_at, updated_at)
             VALUES (?1, ?2, 1, ?3, ?3)
             ON CONFLICT(meeting_id) DO UPDATE SET blocks_json = excluded.blocks_json,
                 revision = meeting_notes.revision + 1, updated_at = excluded.updated_at,
                 deleted_at = NULL",
            params![w.target, blocks, now],
        )
        .map_err(|e| io(&e))?;

        // Projekte und Teilnehmende der Teile gehen auf das Ziel ueber.
        for member in w.members {
            tx.execute(
                "INSERT OR IGNORE INTO meeting_folder_items (folder_id, meeting_id, added_at)
                 SELECT folder_id, ?1, ?2 FROM meeting_folder_items WHERE meeting_id = ?3",
                params![w.target, now, member],
            )
            .map_err(|e| io(&e))?;
            tx.execute(
                "INSERT OR IGNORE INTO meeting_participants (meeting_id, human_id, role, source, created_at)
                 SELECT ?1, human_id, role, source, ?2 FROM meeting_participants WHERE meeting_id = ?3",
                params![w.target, now, member],
            )
            .map_err(|e| io(&e))?;
        }

        // Teilliste.
        tx.execute(
            "DELETE FROM meeting_parts WHERE meeting_id = ?1",
            params![w.target],
        )
        .map_err(|e| io(&e))?;
        for p in &w.plan.parts {
            tx.execute(
                "INSERT INTO meeting_parts (meeting_id, part_index, title, source_path, offset_ms,
                     duration_ms, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    w.target,
                    i64::from(p.part_index),
                    p.title,
                    p.source_path,
                    p.offset_ms as i64,
                    p.duration_ms as i64,
                    now
                ],
            )
            .map_err(|e| io(&e))?;
        }

        // Vormerkungen der Gruppe sind erledigt; die Teile gehen in den Papierkorb.
        tx.execute(
            "DELETE FROM meeting_append_pending WHERE target_id = ?1",
            params![w.target],
        )
        .map_err(|e| io(&e))?;
        for member in w.members {
            let paths = Self::soft_delete_in_tx(&tx, member, now).map_err(|e| io(&e))?;
            old_paths.extend(paths);
        }
        // Der Such-Index der Zielbesprechung wird neu aufgebaut.
        tx.execute(
            "DELETE FROM meeting_chunks WHERE meeting_id = ?1",
            params![w.target],
        )
        .map_err(|e| io(&e))?;
        tx.execute(
            "DELETE FROM meeting_index_state WHERE meeting_id = ?1",
            params![w.target],
        )
        .map_err(|e| io(&e))?;

        tx.commit().map_err(|e| io(&e))?;
        Ok(old_paths)
    }
}

// ---------------------------------------------------------------------------
// Ablauf
// ---------------------------------------------------------------------------

/// Ein Zusammenfuehren zur Zeit (zwei Teile, die gleichzeitig fertig werden,
/// loesen sonst beide dieselbe Gruppe aus).
static MERGE_LOCK: Mutex<()> = Mutex::new(());

/// Liest und prueft die Besprechungen in der gegebenen Reihenfolge.
pub fn load_sources(store: &MeetingStore, ids: &[String]) -> Result<Vec<PartSource>, PartsError> {
    let io = |e: &dyn std::fmt::Display| PartsError::Io(e.to_string());
    if ids.len() < 2 {
        return Err(PartsError::TooFew);
    }
    let mut seen = std::collections::HashSet::new();
    if !ids.iter().all(|id| seen.insert(id.as_str())) {
        return Err(PartsError::TooFew);
    }
    let mut out = Vec::with_capacity(ids.len());
    let mut source_kind: Option<String> = None;
    for id in ids {
        let meeting = match store.get_meeting(id).map_err(|e| io(&e))? {
            Some(m) if m.deleted_at.is_none() => m,
            _ => return Err(PartsError::NotFound(id.clone())),
        };
        if meeting.status != "ready" {
            return Err(PartsError::NotReady(id.clone()));
        }
        if !matches!(meeting.source.as_str(), "import" | "live") {
            return Err(PartsError::NoAudio(id.clone()));
        }
        match &source_kind {
            None => source_kind = Some(meeting.source.clone()),
            Some(k) if *k != meeting.source => return Err(PartsError::MixedSources),
            Some(_) => {}
        }
        let mic_ok = meeting
            .mic_audio_path
            .as_deref()
            .is_some_and(|p| Path::new(p).is_file());
        let sys_ok = meeting
            .system_audio_path
            .as_deref()
            .is_none_or(|p| Path::new(p).is_file());
        if !mic_ok || !sys_ok {
            return Err(PartsError::AudioMissing(id.clone()));
        }
        if store.variant_count(id).map_err(|e| io(&e))? > 1 {
            return Err(PartsError::HasVariants(id.clone()));
        }
        let snapshot = store.transcript_snapshot(id).map_err(|e| io(&e))?;
        out.push(PartSource {
            segments: snapshot.segments,
            revision: snapshot.revision,
            hints: store.speaker_hints(id).map_err(|e| io(&e))?,
            speakers: store.speaker_rows(id).map_err(|e| io(&e))?,
            notes: store.get_notes(id).map_err(|e| io(&e))?.blocks,
            parts: store.meeting_parts(id).map_err(|e| io(&e))?,
            length_ms: 0,
            meeting,
        });
    }
    let parts: usize = out.iter().map(|s| s.parts.len().max(1)).sum();
    if parts > MAX_PARTS {
        return Err(PartsError::TooMany);
    }
    Ok(out)
}

/// Fuehrt `ids` in dieser Reihenfolge in `ids[0]` zusammen: Audio, Datenbank,
/// alte Dateien. `meeting_dir` ist der Ordner der Zielbesprechung. Ohne Tauri
/// (Tests); die Sprechertrennung danach macht [`merge_now`].
pub fn merge_into_first(
    store: &MeetingStore,
    ids: &[String],
    meeting_dir: &Path,
) -> Result<String, PartsError> {
    let _lock = MERGE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut sources = load_sources(store, ids)?;
    let tracks: Vec<(PathBuf, Option<PathBuf>)> = sources
        .iter()
        .map(|s| {
            (
                PathBuf::from(s.meeting.mic_audio_path.clone().unwrap_or_default()),
                s.meeting.system_audio_path.clone().map(PathBuf::from),
            )
        })
        .collect();
    let audio = concat_audio(&tracks, meeting_dir).map_err(PartsError::Io)?;
    for (src, len) in sources.iter_mut().zip(&audio.lengths_ms) {
        src.length_ms = *len;
    }
    let merge_plan = plan(&sources);
    let revisions: Vec<(String, i64)> = sources
        .iter()
        .map(|s| (s.meeting.id.clone(), s.revision))
        .collect();
    let target = ids[0].clone();
    let members: Vec<String> = ids[1..].to_vec();
    let mic = audio.mic.to_string_lossy().to_string();
    let system = audio.system.as_ref().map(|p| p.to_string_lossy().to_string());
    let applied = store.apply_merge(&MergeWrite {
        target: &target,
        members: &members,
        revisions: &revisions,
        plan: &merge_plan,
        mic: &mic,
        system: system.as_deref(),
    });
    match applied {
        Ok(old_paths) => {
            super::retention::delete_audio_files(&old_paths);
            info!(
                "meetings: {} parts merged into {target} ({} ms)",
                ids.len(),
                merge_plan.duration_ms
            );
            Ok(target)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&audio.mic);
            if let Some(p) = &audio.system {
                let _ = std::fs::remove_file(p);
            }
            Err(e)
        }
    }
}

/// Zusammenfuehren in der App: Auftrag (blockt andere Verarbeitungen der
/// Zielbesprechung), Ordner der Teile entfernen, Such-Index, Ereignisse fuer die
/// Oberflaeche. Die Sprechertrennung ueber die ganze Besprechung laeuft danach
/// im Hintergrund ([`speakers_after_merge`]); der Aufrufer wartet nicht auf sie.
pub fn merge_now(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    ids: &[String],
) -> Result<String, PartsError> {
    use super::job::{self, JobPhase};
    use tauri_specta::Event;

    let Some(target) = ids.first() else {
        return Err(PartsError::TooFew);
    };
    if ids.iter().any(|id| job::global().is_running(id)) {
        return Err(PartsError::Busy);
    }
    let base = super::meetings_data_dir(app).map_err(|e| PartsError::Io(e.to_string()))?;
    let guard = job::global()
        .try_start(target, job::app_emit(app))
        .map_err(|_| PartsError::Busy)?;
    guard.handle().begin_phase(JobPhase::Prepare, 0);
    let merged = merge_into_first(store, ids, &base.join(target));
    drop(guard);
    let target = merged?;
    for member in &ids[1..] {
        super::search::indexer::submit(app, super::search::indexer::IndexJob::Deleted(member.clone()));
        if let Err(e) = super::retention::remove_meeting_dir(&base, member) {
            warn!("meetings: folder of merged part {member} not removed: {e}");
        }
    }
    super::search::indexer::submit(app, super::search::indexer::IndexJob::Meeting(target.clone()));
    let _ = (super::recorder::MeetingEvent::Reset {
        meeting_id: target.clone(),
    })
    .emit(app);
    let _ = (super::recorder::MeetingEvent::State {
        meeting_id: target.clone(),
        status: "ready".into(),
        paused: false,
    })
    .emit(app);

    let app = app.clone();
    let store = Arc::clone(store);
    let id = target.clone();
    let spawned = std::thread::Builder::new()
        .name("meeting-parts-speakers".into())
        .stack_size(crate::commands::big_stack::JOB_STACK_BYTES)
        .spawn(move || speakers_after_merge(&app, &store, &id));
    if let Err(e) = spawned {
        warn!("meetings: speaker step after merge not started ({target}): {e}");
    }
    Ok(target)
}

/// Sprechertrennung ueber die ganze zusammengefuehrte Besprechung. Die
/// verschobenen Turns gelten als die "alten": Namen wandern ueber
/// `remap_speakers` auf die neuen Sprecher. Ein Fehler laesst das Transkript,
/// wie es ist (Sprecher mit Versatz je Teil).
fn speakers_after_merge(app: &tauri::AppHandle, store: &Arc<MeetingStore>, target: &str) {
    use super::job::{self, JobPhase};
    use tauri_specta::Event;

    let Ok(guard) = job::global().try_start(target, job::app_emit(app)) else {
        info!("meetings: speaker step after merge skipped, another job runs ({target})");
        return;
    };
    let mut diarizer = super::final_pass::AppDiarizer::from_app(
        app,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    guard.handle().begin_phase(JobPhase::Speakers, 0);
    diarizer.attach_job(Arc::clone(guard.handle()));
    let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::speakers::step_on_stored(store, target, &mut diarizer, false)
    }));
    drop(guard);
    match step {
        Ok((_, super::speakers::ApplyOutcome::Applied { .. })) => {
            let _ = (super::recorder::MeetingEvent::Reset {
                meeting_id: target.to_string(),
            })
            .emit(app);
            super::search::indexer::submit(
                app,
                super::search::indexer::IndexJob::Meeting(target.to_string()),
            );
        }
        Ok(_) => {}
        Err(_) => warn!("meetings: speaker step after merge panicked ({target})"),
    }
    let _ = (super::recorder::MeetingEvent::State {
        meeting_id: target.to_string(),
        status: "ready".into(),
        paused: false,
    })
    .emit(app);
}

/// Aus der Warteschlange, nachdem `meeting_id` fertig verarbeitet ist: gehoert
/// die Besprechung zu einer Gruppe (Ziel oder vorgemerkter Teil) und sind jetzt
/// alle Teile fertig, wird zusammengefuehrt. Ein gescheiterter oder gestoppter
/// Teil haelt die Gruppe an; die Oberflaeche zeigt ihn als wartenden Teil.
pub fn after_import(app: &tauri::AppHandle, store: &Arc<MeetingStore>, meeting_id: &str) {
    let target = match store.append_target_of(meeting_id) {
        Ok(Some(t)) => t,
        Ok(None) => meeting_id.to_string(),
        Err(e) => {
            warn!("meetings: append group not read ({meeting_id}): {e}");
            return;
        }
    };
    let pending = match store.append_pending_of(&target) {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => return,
        Err(e) => {
            warn!("meetings: append group not read ({target}): {e}");
            return;
        }
    };
    let target_ready = matches!(store.get_meeting(&target), Ok(Some(m)) if m.status == "ready" && m.deleted_at.is_none());
    if !target_ready || pending.iter().any(|p| p.status != "ready") {
        return;
    }
    let mut ids = vec![target.clone()];
    ids.extend(pending.into_iter().map(|p| p.meeting_id));
    match merge_now(app, store, &ids) {
        Ok(_) => {}
        // Noch etwas laeuft (z. B. der zweite Teil endet gerade): dessen Ende
        // loest die Gruppe erneut aus.
        Err(PartsError::Busy) => info!("meetings: append group {target} waits for a running job"),
        Err(e) => {
            warn!("meetings: append group {target} not merged: {e}");
            use tauri_specta::Event;
            let _ = (super::recorder::MeetingEvent::Error {
                meeting_id: target.clone(),
                message: e.code().to_string(),
            })
            .emit(app);
        }
    }
}

/// Teile-Info fuer die Oberflaeche.
pub fn parts_info(store: &MeetingStore, meeting_id: &str) -> Result<PartsInfo> {
    let append_to = match store.append_target_of(meeting_id)? {
        Some(target) => store
            .get_meeting(&target)?
            .filter(|m| m.deleted_at.is_none())
            .map(|m| (m.id, m.title)),
        None => None,
    };
    Ok(PartsInfo {
        parts: store.meeting_parts(meeting_id)?,
        pending: store.append_pending_of(meeting_id)?,
        append_to,
    })
}
