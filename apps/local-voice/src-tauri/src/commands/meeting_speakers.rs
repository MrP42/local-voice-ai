//! Tauri-Commands zum Benennen von Sprechern (M3-P3c). Duenne Huelle ueber
//! `MeetingStore` und `SpeakerDirectory`; die Logik steht in den Funktionen
//! ohne Tauri (testbar mit einem Store im Tempdir).
//!
//! Fehlercodes (an die UI, die sie uebersetzt): `speaker_not_found`,
//! `speaker_invalid`, `segment_not_found`, `stale_epoch` (das Transkript wurde
//! inzwischen ersetzt: die Segmentnummer der UI gilt nicht mehr).
//!
//! Datenschutz: Sprechernamen stehen in keinem Log und in keiner Fehlermeldung.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::managers::meetings::search::indexer::{self, IndexJob};
use crate::managers::meetings::speakers::{SpeakerDirectory, DIARIZE_MIC_KEY, REPORT_KEY};
use crate::managers::meetings::store::MeetingStore;

/// Laengster Sprechername (Zeichen); mehr wird abgeschnitten.
pub const MAX_NAME_CHARS: usize = 80;

/// Ein Sprecher einer Besprechung, wie das Popover ihn braucht.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct MeetingSpeaker {
    /// 0 = Mikrofon, 1 = Systemton (Gegenseite), 2 = Import (Mischspur).
    pub channel: u8,
    /// Nummer im Kanal, ab 1.
    pub speaker_index: u32,
    /// Anzeige: der Name, sonst "Gegenseite 2" / "Raum 1" / "Person 1".
    pub label: String,
    /// Der vom Nutzer vergebene Name.
    pub display_name: Option<String>,
    /// Verweis auf die Personentabelle (gefuellt ab P3d/P5d).
    pub human_id: Option<String>,
    /// Redeanteil an der gesamten Sprechzeit, in Prozent (eine Nachkommastelle).
    pub share_pct: f64,
}

/// Namen, Zusammenfuehrungen oder Zuordnungen haben sich geaendert: offene
/// Ansichten laden Segmente und Sprecher neu.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct SpeakersChanged {
    pub meeting_id: String,
}

// ---------------------------------------------------------------------------
// Logik ohne Tauri
// ---------------------------------------------------------------------------

/// Ein Name, wie er gespeichert wird: getrimmt, Leerraum zusammengezogen,
/// `:` durch `-` ersetzt (das Label steht als `Name: Text` in Chunks und
/// Export und wird beim Lesen am ersten `: ` getrennt), hoechstens
/// [`MAX_NAME_CHARS`] Zeichen. Leer ergibt `None` (Name loeschen).
pub fn clean_name(name: Option<&str>) -> Option<String> {
    let joined = name?
        .replace(':', "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cut: String = joined.chars().take(MAX_NAME_CHARS).collect();
    let cut = cut.trim().to_string();
    (!cut.is_empty()).then_some(cut)
}

fn store_err(e: anyhow::Error) -> String {
    e.to_string()
}

/// Alle Sprecher der Besprechung: die in den Segmenten vorkommen und die mit
/// einer Zeile in `speakers`, nach Kanal und Nummer.
pub fn list_speakers(
    store: &MeetingStore,
    meeting_id: &str,
) -> Result<Vec<MeetingSpeaker>, String> {
    let segments = store.get_segments(meeting_id).map_err(store_err)?;
    let rows = store.speaker_rows(meeting_id).map_err(store_err)?;
    let directory = SpeakerDirectory::load(store, meeting_id);

    let mut speech_ms: BTreeMap<(u8, u32), u64> = BTreeMap::new();
    let mut total_ms = 0u64;
    for segment in &segments {
        let duration = segment.end_ms.saturating_sub(segment.start_ms);
        total_ms += duration;
        if let Some(index) = segment.speaker_index {
            *speech_ms.entry((segment.channel, index)).or_insert(0) += duration;
        }
    }
    let mut human: BTreeMap<(u8, u32), Option<String>> = BTreeMap::new();
    for row in &rows {
        human.insert((row.channel, row.speaker_index), row.human_id.clone());
        speech_ms
            .entry((row.channel, row.speaker_index))
            .or_insert(0);
    }

    Ok(speech_ms
        .into_iter()
        .map(|((channel, speaker_index), ms)| MeetingSpeaker {
            channel,
            speaker_index,
            label: directory.label_for(channel, Some(speaker_index)),
            display_name: directory.name(channel, speaker_index).map(str::to_string),
            human_id: human.get(&(channel, speaker_index)).cloned().flatten(),
            share_pct: if total_ms == 0 {
                0.0
            } else {
                (ms as f64 * 1000.0 / total_ms as f64).round() / 10.0
            },
        })
        .collect())
}

/// Einen Sprecher benennen (`None` oder leer: Namen loeschen). Der Name gilt
/// fuer alle Segmente dieses Sprechers. Gibt den Sprecher danach zurueck.
pub fn rename_speaker(
    store: &MeetingStore,
    meeting_id: &str,
    channel: u8,
    speaker_index: u32,
    name: Option<&str>,
) -> Result<MeetingSpeaker, String> {
    let exists = |list: &[MeetingSpeaker]| {
        list.iter()
            .any(|s| s.channel == channel && s.speaker_index == speaker_index)
    };
    if !exists(&list_speakers(store, meeting_id)?) {
        return Err("speaker_not_found".to_string());
    }
    let name = clean_name(name);
    store
        .set_speaker_name(meeting_id, channel, speaker_index, name.as_deref())
        .map_err(store_err)?;
    list_speakers(store, meeting_id)?
        .into_iter()
        .find(|s| s.channel == channel && s.speaker_index == speaker_index)
        .ok_or_else(|| "speaker_not_found".to_string())
}

/// Hinweise zur Sprechertrennung aus `metadata_json.diarize` (Codes, keine
/// Texte): warum Kanaele nicht getrennt wurden (`disabled`, `model_missing`,
/// `crash_loop`, `low_memory`, `failed`, `too_long`, `unreadable`) und
/// `mic_without_aec` (Mikrofon getrennt, aber ohne Echo-freie Spur).
pub fn speaker_notices(store: &MeetingStore, meeting_id: &str) -> Vec<String> {
    const SKIPS: [&str; 7] = [
        "disabled",
        "model_missing",
        "crash_loop",
        "low_memory",
        "failed",
        "too_long",
        "unreadable",
    ];
    let Some(report) = store
        .metadata_json(meeting_id)
        .ok()
        .flatten()
        .and_then(|m| m.get(REPORT_KEY).cloned())
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let skipped = report
        .get("channels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("skipped").and_then(Value::as_str));
    for reason in skipped {
        if SKIPS.contains(&reason) && !out.iter().any(|o| o == reason) {
            out.push(reason.to_string());
        }
    }
    if report.get("mic_without_aec").and_then(Value::as_bool) == Some(true) {
        out.push("mic_without_aec".to_string());
    }
    out
}

/// Einstellung `meeting_diarization`: alles ausser `off` ist `auto`.
fn normalize_diarization(mode: &str) -> &'static str {
    if crate::settings::meeting_diarization_enabled(mode) {
        "auto"
    } else {
        "off"
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Nach jeder Aenderung: offene Ansichten benachrichtigen und den Such-Index
/// neu aufbauen (Namen stehen im Chunk-Text; `content_revision` ist gestiegen).
fn changed(app: &AppHandle, meeting_id: &str) {
    let _ = SpeakersChanged {
        meeting_id: meeting_id.to_string(),
    }
    .emit(app);
    indexer::submit(app, IndexJob::Meeting(meeting_id.to_string()));
}

/// Die Sprecher einer Besprechung (Popover, Zusammenfuehren, Zuordnen).
#[tauri::command]
#[specta::specta]
pub async fn meeting_speakers_list(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<MeetingSpeaker>, String> {
    list_speakers(&store, &meeting_id)
}

/// Benennt einen Sprecher; gilt fuer alle seine Segmente.
#[tauri::command]
#[specta::specta]
pub async fn meeting_speaker_rename(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    channel: u8,
    speaker_index: u32,
    name: Option<String>,
) -> Result<MeetingSpeaker, String> {
    let speaker = rename_speaker(&store, &meeting_id, channel, speaker_index, name.as_deref())?;
    changed(&app, &meeting_id);
    Ok(speaker)
}

/// Fuehrt zwei Sprecher eines Kanals zusammen (`from` geht in `into` auf).
#[tauri::command]
#[specta::specta]
pub async fn meeting_speaker_merge(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    channel: u8,
    from: u32,
    into: u32,
) -> Result<(), String> {
    store
        .merge_speakers(&meeting_id, channel, from, into)
        .map_err(store_err)?;
    changed(&app, &meeting_id);
    Ok(())
}

/// Ordnet ein einzelnes Segment einem anderen Sprecher zu (`None`: Zuordnung
/// aufheben). `epoch` ist die Epoche, auf der die Ansicht das Segment sah.
#[tauri::command]
#[specta::specta]
pub async fn meeting_segment_set_speaker(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    segment_index: u32,
    epoch: u32,
    speaker_index: Option<u32>,
) -> Result<(), String> {
    store
        .set_segment_speaker(&meeting_id, segment_index, epoch, speaker_index)
        .map_err(store_err)?;
    changed(&app, &meeting_id);
    Ok(())
}

/// Hinweise zur Sprechertrennung dieser Besprechung (Codes, siehe
/// [`speaker_notices`]).
#[tauri::command]
#[specta::specta]
pub async fn meeting_speaker_notices(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<Vec<String>, String> {
    Ok(speaker_notices(&store, &meeting_id))
}

/// "Mehrere Personen am Mikrofon": das Mikrofon dieser Besprechung wird
/// ebenfalls in Sprecher getrennt (`metadata_json.diarize_mic`). Wirkt beim
/// Enddurchlauf nach dem Stopp.
#[tauri::command]
#[specta::specta]
pub async fn meetings_set_diarize_mic(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    enabled: bool,
) -> Result<(), String> {
    store
        .set_metadata_key(&meeting_id, DIARIZE_MIC_KEY, json!(enabled))
        .map_err(store_err)
}

/// Einstellung `meeting_diarization` (`auto` | `off`): wirkt ab dem naechsten
/// Stopp, Import und der naechsten Neu-Transkription.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_diarization_setting(app: AppHandle, mode: String) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.meeting_diarization = normalize_diarization(&mode).to_string();
    crate::settings::write_settings(&app, settings);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::diarize::Turn;
    use crate::managers::meetings::export::build_bundle;
    use crate::managers::meetings::search::chunking::{chunk_transcript_with, ChunkHead};
    use crate::managers::meetings::speakers::{
        hints_with_turns, turns_from_hints, HINTS_MODEL_NAME,
    };
    use crate::managers::meetings::store::{
        MeetingSource, MeetingStatus, SpeakerWrite, StoredSegment, TranscriptDelta,
    };

    struct Fixture {
        _dir: tempfile::TempDir,
        store: MeetingStore,
        id: String,
    }

    fn segment(index: u32, channel: u8, from: u64, to: u64, speaker: Option<u32>) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: format!("Text {index}"),
            start_ms: from,
            end_ms: to,
            channel,
            speaker_index: speaker,
            words: None,
        }
    }

    /// Gegenseite mit zwei Sprechern (1 und 2: 6 s und 2 s), dazu "Ich" ohne
    /// Sprechertrennung (2 s) und die Turns der Gegenseite in `speaker_hints_json`.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let m = store
            .create_meeting("t", MeetingSource::Live, Some(0))
            .unwrap();
        store.set_status(&m.id, MeetingStatus::Processing).unwrap();
        let segments = vec![
            segment(0, 0, 0, 2_000, None),
            segment(1, 1, 2_000, 5_000, Some(1)),
            segment(2, 1, 5_000, 7_000, Some(2)),
            segment(3, 1, 7_000, 10_000, Some(1)),
        ];
        store
            .append_delta(
                &m.id,
                &TranscriptDelta {
                    new_segments: segments.clone(),
                },
            )
            .unwrap();
        let turns = vec![
            Turn {
                start_ms: 2_000,
                end_ms: 5_000,
                speaker: 1,
            },
            Turn {
                start_ms: 5_000,
                end_ms: 7_000,
                speaker: 2,
            },
            Turn {
                start_ms: 7_000,
                end_ms: 10_000,
                speaker: 1,
            },
        ];
        let write = SpeakerWrite {
            hints_json: hints_with_turns(
                None,
                HINTS_MODEL_NAME,
                1_000,
                100,
                &BTreeMap::from([(1u8, turns)]),
            ),
            present: vec![(1, 1), (1, 2)],
            fresh: BTreeMap::new(),
        };
        let snapshot = store.transcript_snapshot(&m.id).unwrap();
        store
            .update_segments_with_speakers(&m.id, &segments, snapshot.revision, false, &write)
            .unwrap();
        Fixture {
            _dir: dir,
            store,
            id: m.id,
        }
    }

    fn revision(f: &Fixture) -> i64 {
        f.store.transcript_snapshot(&f.id).unwrap().revision
    }

    fn speaker(f: &Fixture, channel: u8, index: u32) -> MeetingSpeaker {
        list_speakers(&f.store, &f.id)
            .unwrap()
            .into_iter()
            .find(|s| s.channel == channel && s.speaker_index == index)
            .unwrap()
    }

    #[test]
    fn the_list_has_labels_and_shares() {
        let f = fixture();
        let list = list_speakers(&f.store, &f.id).unwrap();
        assert_eq!(list.len(), 2, "\"Ich\" ohne Sprecher zaehlt nicht mit");
        assert_eq!(list[0].label, "Gegenseite 1");
        assert_eq!(list[1].label, "Gegenseite 2");
        assert_eq!(list[0].display_name, None);
        // 6 s von 10 s Sprechzeit gesamt, 2 s davon "Ich".
        assert!((list[0].share_pct - 60.0).abs() < 0.01, "{list:?}");
        assert!((list[1].share_pct - 20.0).abs() < 0.01, "{list:?}");
    }

    #[test]
    fn renaming_applies_to_the_speaker_and_raises_the_revision() {
        let f = fixture();
        let before = revision(&f);
        let s = rename_speaker(&f.store, &f.id, 1, 2, Some("  Anna   Berg ")).unwrap();
        assert_eq!(s.label, "Anna Berg");
        assert_eq!(s.display_name.as_deref(), Some("Anna Berg"));
        assert_eq!(revision(&f), before + 1, "der Such-Index muss neu bauen");
        // Alle Segmente dieses Sprechers tragen jetzt den Namen (Verzeichnis).
        let dir = SpeakerDirectory::load(&f.store, &f.id);
        let labels: Vec<String> = f
            .store
            .get_segments(&f.id)
            .unwrap()
            .iter()
            .map(|s| dir.label(s))
            .collect();
        assert_eq!(labels, ["Ich", "Gegenseite 1", "Anna Berg", "Gegenseite 1"]);
        // Derselbe Name noch einmal: keine neue Revision.
        rename_speaker(&f.store, &f.id, 1, 2, Some("Anna Berg")).unwrap();
        assert_eq!(revision(&f), before + 1);
        // Leer loescht den Namen.
        let s = rename_speaker(&f.store, &f.id, 1, 2, Some("   ")).unwrap();
        assert_eq!((s.label.as_str(), s.display_name), ("Gegenseite 2", None));
    }

    #[test]
    fn names_are_cleaned_and_unknown_speakers_are_rejected() {
        assert_eq!(
            clean_name(Some("Dr. Meier: Vertrieb")).as_deref(),
            Some("Dr. Meier- Vertrieb")
        );
        assert_eq!(clean_name(Some("a\nb\t c")).as_deref(), Some("a b c"));
        assert_eq!(
            clean_name(Some("x".repeat(200).as_str()))
                .unwrap()
                .chars()
                .count(),
            MAX_NAME_CHARS
        );
        assert_eq!(clean_name(Some(" \n ")), None);
        assert_eq!(clean_name(None), None);
        let f = fixture();
        assert_eq!(
            rename_speaker(&f.store, &f.id, 1, 9, Some("X")).unwrap_err(),
            "speaker_not_found"
        );
        assert_eq!(
            rename_speaker(&f.store, "gibt-es-nicht", 1, 1, Some("X")).unwrap_err(),
            "speaker_not_found"
        );
    }

    #[test]
    fn merging_moves_segments_and_turns_and_keeps_the_name() {
        let f = fixture();
        rename_speaker(&f.store, &f.id, 1, 2, Some("Anna")).unwrap();
        let epoch = f.store.segment_epoch(&f.id).unwrap();
        let before = revision(&f);
        f.store.merge_speakers(&f.id, 1, 2, 1).unwrap();

        let segments = f.store.get_segments(&f.id).unwrap();
        assert!(segments
            .iter()
            .filter(|s| s.channel == 1)
            .all(|s| s.speaker_index == Some(1)));
        assert_eq!(segments[0].speaker_index, None, "\"Ich\" bleibt");
        let list = list_speakers(&f.store, &f.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "Anna", "der Name von `from` bleibt erhalten");
        assert!((list[0].share_pct - 80.0).abs() < 0.01);
        assert_eq!(revision(&f), before + 1);
        assert_eq!(
            f.store.segment_epoch(&f.id).unwrap(),
            epoch,
            "Epoche bleibt"
        );
        // Auch die gespeicherten Turns (Neu-Transkription) kennen Sprecher 2 nicht mehr.
        let hints: Value =
            serde_json::from_str(&f.store.speaker_hints(&f.id).unwrap().unwrap()).unwrap();
        let turns = &turns_from_hints(&hints)[&1];
        assert!(turns.iter().all(|t| t.speaker == 1));
        assert_eq!(hints["model"], HINTS_MODEL_NAME, "Modell bleibt stehen");
    }

    #[test]
    fn merging_keeps_the_target_name_and_validates_its_input() {
        let f = fixture();
        rename_speaker(&f.store, &f.id, 1, 1, Some("Ben")).unwrap();
        rename_speaker(&f.store, &f.id, 1, 2, Some("Anna")).unwrap();
        f.store.merge_speakers(&f.id, 1, 2, 1).unwrap();
        assert_eq!(speaker(&f, 1, 1).label, "Ben", "Ziel behaelt seinen Namen");

        assert_eq!(
            f.store
                .merge_speakers(&f.id, 1, 1, 1)
                .unwrap_err()
                .to_string(),
            "speaker_invalid"
        );
        assert_eq!(
            f.store
                .merge_speakers(&f.id, 1, 2, 1)
                .unwrap_err()
                .to_string(),
            "speaker_not_found",
            "Sprecher 2 gibt es nicht mehr"
        );
        assert_eq!(
            f.store
                .merge_speakers(&f.id, 0, 1, 2)
                .unwrap_err()
                .to_string(),
            "speaker_not_found",
            "auf Kanal 0 gibt es keine Sprecher"
        );
    }

    #[test]
    fn merging_into_an_unnamed_target_takes_over_the_name_row() {
        let f = fixture();
        rename_speaker(&f.store, &f.id, 1, 1, Some("Ben")).unwrap();
        // 1 -> 2: Ziel 2 hat keinen Namen, der Name von 1 geht mit.
        f.store.merge_speakers(&f.id, 1, 1, 2).unwrap();
        let list = list_speakers(&f.store, &f.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].speaker_index, list[0].label.as_str()), (2, "Ben"));
    }

    #[test]
    fn a_single_segment_can_move_to_another_or_a_new_speaker() {
        let f = fixture();
        let epoch = f.store.segment_epoch(&f.id).unwrap();
        let before = revision(&f);
        f.store
            .set_segment_speaker(&f.id, 3, epoch, Some(2))
            .unwrap();
        let segments = f.store.get_segments(&f.id).unwrap();
        assert_eq!(segments[3].speaker_index, Some(2));
        assert_eq!(segments[1].speaker_index, Some(1), "nur dieses Segment");
        assert_eq!(revision(&f), before + 1);
        assert_eq!(f.store.segment_epoch(&f.id).unwrap(), epoch);

        // Neuer Sprecher: Nummer 3 bekommt eine Zeile und erscheint in der Liste.
        f.store
            .set_segment_speaker(&f.id, 1, epoch, Some(3))
            .unwrap();
        let list = list_speakers(&f.store, &f.id).unwrap();
        assert_eq!(
            list.iter().map(|s| s.speaker_index).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(speaker(&f, 1, 3).label, "Gegenseite 3");
        // Zuordnung aufheben.
        f.store.set_segment_speaker(&f.id, 1, epoch, None).unwrap();
        assert_eq!(f.store.get_segments(&f.id).unwrap()[1].speaker_index, None);
    }

    #[test]
    fn reassigning_reports_stale_epochs_and_bad_input() {
        let f = fixture();
        let epoch = f.store.segment_epoch(&f.id).unwrap();
        let before = revision(&f);
        let err = |r: anyhow::Result<()>| r.unwrap_err().to_string();
        assert_eq!(
            err(f.store.set_segment_speaker(&f.id, 1, epoch + 1, Some(2))),
            "stale_epoch"
        );
        assert_eq!(
            err(f.store.set_segment_speaker(&f.id, 99, epoch, Some(2))),
            "segment_not_found"
        );
        assert_eq!(
            err(f.store.set_segment_speaker(&f.id, 1, epoch, Some(0))),
            "speaker_invalid"
        );
        assert_eq!(revision(&f), before, "Fehler aendern nichts");
        assert_eq!(
            f.store.get_segments(&f.id).unwrap()[1].speaker_index,
            Some(1)
        );
    }

    #[test]
    fn names_reach_the_search_index_text_and_the_export() {
        let f = fixture();
        rename_speaker(&f.store, &f.id, 1, 2, Some("Anna Berg")).unwrap();
        let segments = f.store.get_segments(&f.id).unwrap();
        let head = ChunkHead {
            title: "t".into(),
            started_at: None,
            folder_names: Vec::new(),
        };
        let dir = SpeakerDirectory::load(&f.store, &f.id);
        let chunks = chunk_transcript_with(&segments, 0, &head, &dir);
        let text = chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("S2 00:05 Anna Berg: Text 2"), "{text}");
        assert!(text.contains("Gegenseite 1: Text 1"), "{text}");

        let bundle = build_bundle(&f.store, &f.id).unwrap();
        assert_eq!(bundle.speakers.label(&segments[2]), "Anna Berg");
        assert_eq!(bundle.speakers.label(&segments[1]), "Gegenseite 1");
    }

    #[test]
    fn notices_come_from_the_diarization_report() {
        let f = fixture();
        assert!(
            speaker_notices(&f.store, &f.id).is_empty(),
            "ohne Bericht keine Hinweise"
        );
        f.store
            .set_metadata_key(
                &f.id,
                REPORT_KEY,
                json!({
                    "state": "skipped",
                    "channels": [
                        {"channel": 1, "skipped": "model_missing"},
                        {"channel": 0, "skipped": "model_missing"},
                        {"channel": 2, "skipped": "too_short"},
                        {"channel": 3, "skipped": "crash_loop"},
                    ],
                    "mic_without_aec": true,
                }),
            )
            .unwrap();
        assert_eq!(
            speaker_notices(&f.store, &f.id),
            ["model_missing", "crash_loop", "mic_without_aec"]
        );
    }

    #[test]
    fn the_diarize_mic_flag_is_read_by_the_plan() {
        let f = fixture();
        f.store
            .set_metadata_key(&f.id, DIARIZE_MIC_KEY, json!(true))
            .unwrap();
        let metadata = f.store.metadata_json(&f.id).unwrap();
        assert!(crate::managers::meetings::speakers::diarize_mic(
            metadata.as_ref()
        ));
    }

    #[test]
    fn the_setting_is_normalised() {
        assert_eq!(normalize_diarization("off"), "off");
        assert_eq!(normalize_diarization(" OFF "), "off");
        assert_eq!(normalize_diarization("auto"), "auto");
        assert_eq!(normalize_diarization("irgendwas"), "auto");
    }
}
