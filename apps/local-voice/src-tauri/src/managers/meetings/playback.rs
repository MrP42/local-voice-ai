//! #537: eine YouTube-Besprechung bekommt ihr Transkript beim Abspielen.
//!
//! Ohne yt-dlp gibt es keinen Weg an Untertitel oder Ton (Rechtsrahmen E1: kein
//! Download, der Einbett-Player bleibt unveraendert). Der Nutzer spielt das
//! Video im eingebetteten Player ab, die App nimmt NUR den Systemton auf
//! (Loopback, kein Mikrofon) und transkribiert ihn wie eine Live-Aufnahme. Dieses
//! Modul ist der Teil in der Datenbank: eine YouTube-Besprechung ohne Transkript
//! wird zur Aufnahme (`begin_playback_capture`), und scheitert der Start, wird
//! sie in ihren Vorzustand zurueckgesetzt (`restore_playback_target`). Die
//! Aufnahme selbst steht in `recorder.rs` (`start_playback_capture`).
//!
//! Was bleibt, was sich aendert: die Zeile behaelt Id, Titel, `source = youtube`,
//! `metadata_json` (Video, Herkunft), Notizen, Projekte und Teilnehmende. Es aendern
//! sich nur `status` (`recording`), `consent_confirmed_at` und (waehrend der
//! Aufnahme) die Audiopfade; `source` bleibt `youtube`, damit Player, Quelle und
//! Fassungen ohne Sonderweg weiterarbeiten. Keine Migration, kein neues Feld.
//!
//! Fehlerfaelle und Absicherung:
//! - **Zwei Starts gleichzeitig** (Doppelklick, zwei Fenster): Pruefung und Umwandlung
//!   laufen in EINER IMMEDIATE-Transaktion; einer gewinnt, der andere bekommt
//!   `target_not_empty`. Test: `only_one_of_two_simultaneous_starts_wins`.
//! - **Ziel ist nicht (mehr) bereit**: kein YouTube-Eintrag, laeuft schon etwas
//!   (`recording`/`processing`), hat Transkript-Saetze (aktive oder gesicherte
//!   Fassung) oder schon eine Aufnahme auf der Platte: `target_not_empty`, nichts
//!   aendert sich und vor allem wird keine vorhandene WAV ueberschrieben (kein
//!   Datenverlust); unbekannt oder geloescht: `meeting_not_found`.
//! - **Start scheitert** (kein Ausgabegeraet, Treiber haengt, Datei nicht anlegbar):
//!   `restore_playback_target` stellt `ready`, Einwilligung und Dauer des Players
//!   wieder her und leert die Audiopfade (Test `a_failed_start_puts_the_entry_back...`).
//!   Haengt der Rueckfall selbst (Datenbank weg), steht die Zeile auf `failed`, nie auf
//!   `recording`: der naechste App-Start soll sie nicht als Absturzrest beleben.
//! - **Abbruch mitten im Vorgang**: beide Schritte sind je ein einzelnes UPDATE
//!   in einer Transaktion; ein Absturz dazwischen hinterlaesst entweder den alten
//!   oder den neuen Zustand, nie einen halben. Der neue Zustand `recording` ohne
//!   Audio ist der Absturzfall des Starts und wird vom Recovery wie jeder andere
//!   Rest behandelt (kein Ton, kein Transkript: Besprechung `ready`, leer).
//! - **Altdaten**: die Zeile stammt aus A2/A3 (`youtube/source.rs`); Notizen,
//!   Projekte, `metadata_json` und die vom Player nachgetragene Dauer bleiben
//!   (Test mit einer Zeile, die alles davon traegt).

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::empty::TARGET_NOT_EMPTY;
use super::store::{Meeting, MeetingSource, MeetingStatus, MeetingStore};

/// `meetings.source` der Besprechung, in die mitgeschrieben wird.
pub const SOURCE_YOUTUBE: &str = MeetingSource::Youtube.as_str();

/// Was die Zeile vor dem Start war: damit ein gescheiterter Start sie genau so
/// zurueckgibt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackPrior {
    pub status: String,
    pub duration_ms: Option<i64>,
    pub consent_confirmed_at: Option<i64>,
}

/// Die gelesene Zeile: Quelle, Status, beide Tonpfade, Dauer, Einwilligung.
type TargetRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// Hat die Besprechung Transkript-Saetze, in der aktiven Fassung (`transcripts`)
/// oder in einer gesicherten (`transcript_variants`)?
const HAS_SEGMENTS: &str = "(EXISTS (SELECT 1 FROM transcripts
                                    WHERE meeting_id = ?1 AND deleted_at IS NULL
                                      AND segments_json != '[]')
         OR EXISTS (SELECT 1 FROM transcript_variants
                    WHERE meeting_id = ?1 AND deleted_at IS NULL
                      AND segments_json != '[]'))";

impl MeetingStore {
    /// Wandelt eine YouTube-Besprechung ohne Transkript in eine Aufnahme
    /// (`recording`, Einwilligung gesetzt) und liefert sie samt ihrem Vorzustand.
    /// Fehler: `meeting_not_found`, `target_not_empty` (siehe Moduldoku).
    pub fn begin_playback_capture(
        &self,
        id: &str,
        consent_at: i64,
    ) -> Result<(Meeting, PlaybackPrior)> {
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<TargetRow> = tx
            .query_row(
                "SELECT source, status, mic_audio_path, system_audio_path, duration_ms,
                        consent_confirmed_at
                   FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((source, status, mic, system, duration_ms, consent)) = row else {
            return Err(anyhow!("meeting_not_found"));
        };
        let idle = status == MeetingStatus::Ready.as_str();
        let has_audio = mic.is_some() || system.is_some();
        let has_segments: bool =
            tx.query_row(&format!("SELECT {HAS_SEGMENTS}"), params![id], |r| r.get(0))?;
        if source != SOURCE_YOUTUBE || !idle || has_audio || has_segments {
            return Err(anyhow!(TARGET_NOT_EMPTY));
        }

        let now = Utc::now().timestamp();
        let changed = tx.execute(
            "UPDATE meetings
                SET status = 'recording', consent_confirmed_at = ?2, updated_at = ?3
              WHERE id = ?1 AND deleted_at IS NULL AND source = 'youtube' AND status = 'ready'
                AND mic_audio_path IS NULL AND system_audio_path IS NULL",
            params![id, consent_at, now],
        )?;
        if changed != 1 {
            return Err(anyhow!(TARGET_NOT_EMPTY));
        }
        tx.commit()?;
        let meeting = self
            .get_meeting(id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))?;
        Ok((
            meeting,
            PlaybackPrior {
                status,
                duration_ms,
                consent_confirmed_at: consent,
            },
        ))
    }

    /// Der Start scheiterte, bevor etwas aufgenommen wurde: die Zeile ist wieder
    /// die YouTube-Besprechung von vorher (Status, Einwilligung und Dauer wie
    /// zuvor, keine Audiopfade). Greift nur fuer eine YouTube-Zeile im Zustand
    /// `recording`/`failed` ohne Transkript-Saetze; `false`, wenn sie so nicht
    /// aussieht (dann bleibt alles, wie es ist).
    pub fn restore_playback_target(&self, id: &str, prior: &PlaybackPrior) -> Result<bool> {
        let now = Utc::now().timestamp();
        let conn = self.get_connection()?;
        let changed = conn.execute(
            &format!(
                "UPDATE meetings
                    SET status = ?2, consent_confirmed_at = ?3, duration_ms = ?4,
                        mic_audio_path = NULL, system_audio_path = NULL, updated_at = ?5
                  WHERE id = ?1 AND deleted_at IS NULL AND source = 'youtube'
                    AND status IN ('recording', 'failed') AND NOT {HAS_SEGMENTS}"
            ),
            params![
                id,
                prior.status,
                prior.consent_confirmed_at,
                prior.duration_ms,
                now
            ],
        )?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::store::{StoredSegment, TranscriptDelta};
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Arc<MeetingStore>) {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        (dir, Arc::new(s))
    }

    /// Eine YouTube-Besprechung, wie `youtube/source.rs` sie anlegt, samt allem, was
    /// danach dazukam: Projekt, Notiz, vom Player nachgetragene Dauer.
    fn youtube_entry(store: &MeetingStore) -> (String, String) {
        let name = format!("Kunde {}", ulid::Ulid::new());
        let folder = store.folder_save(None, &name, None).unwrap().id;
        let meeting = store
            .create_meeting("Vortrag", MeetingSource::Youtube, None)
            .unwrap();
        store
            .set_metadata_key(
                &meeting.id,
                "youtube",
                serde_json::json!({"video_id": "abcdefghijk", "channel": "Kanal"}),
            )
            .unwrap();
        store
            .set_meeting_folders(&meeting.id, std::slice::from_ref(&folder))
            .unwrap();
        let conn = store.get_connection().unwrap();
        conn.execute(
            "UPDATE meetings SET duration_ms = 1805000, status = 'ready' WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
        (meeting.id, folder)
    }

    fn delta(text: &str) -> TranscriptDelta {
        TranscriptDelta {
            new_segments: vec![StoredSegment {
                segment_index: 0,
                start_ms: 0,
                end_ms: 1_000,
                text: text.to_string(),
                channel: 1,
                speaker_index: None,
                words: None,
            }],
        }
    }

    #[test]
    fn a_youtube_entry_without_a_transcript_becomes_a_recording_and_keeps_everything_else() {
        let (_dir, store) = store();
        let (id, folder) = youtube_entry(&store);

        let (meeting, prior) = store.begin_playback_capture(&id, 777).unwrap();

        assert_eq!(meeting.id, id, "keine neue Besprechung");
        assert_eq!(
            (meeting.source.as_str(), meeting.status.as_str()),
            ("youtube", "recording")
        );
        assert_eq!(meeting.consent_confirmed_at, Some(777));
        assert_eq!(meeting.title, "Vortrag");
        assert_eq!(meeting.duration_ms, Some(1_805_000));
        assert_eq!(store.meeting_folder_ids(&id).unwrap(), vec![folder]);
        let metadata = store.metadata_json(&id).unwrap().unwrap();
        assert_eq!(metadata["youtube"]["video_id"], "abcdefghijk");
        assert_eq!(
            prior,
            PlaybackPrior {
                status: "ready".into(),
                duration_ms: Some(1_805_000),
                consent_confirmed_at: None
            }
        );
    }

    #[test]
    fn a_youtube_entry_with_segments_is_refused_and_stays_untouched() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store.append_delta(&id, &delta("Schon da")).unwrap();

        let err = store.begin_playback_capture(&id, 1).unwrap_err();

        assert_eq!(err.to_string(), "target_not_empty");
        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "ready");
        assert_eq!(after.consent_confirmed_at, None);
        assert_eq!(store.get_segments(&id).unwrap().len(), 1);
    }

    #[test]
    fn everything_but_an_idle_youtube_entry_is_refused() {
        let (_dir, store) = store();
        // Eine Live-Besprechung, ein leerer Eintrag, ein Import.
        let live = store
            .create_meeting("Live", MeetingSource::Live, Some(1))
            .unwrap();
        let empty = store.create_empty_meeting("Neu", None).unwrap();
        let import = store
            .create_meeting("Datei", MeetingSource::Import, None)
            .unwrap();
        for id in [&live.id, &empty.id, &import.id] {
            assert_eq!(
                store.begin_playback_capture(id, 1).unwrap_err().to_string(),
                "target_not_empty",
                "{id}"
            );
        }
        // Eine YouTube-Besprechung, die gerade etwas tut.
        for busy in ["recording", "processing", "cancelled", "failed"] {
            let (id, _) = youtube_entry(&store);
            store
                .get_connection()
                .unwrap()
                .execute(
                    "UPDATE meetings SET status = ?2 WHERE id = ?1",
                    params![id, busy],
                )
                .unwrap();
            assert_eq!(
                store
                    .begin_playback_capture(&id, 1)
                    .unwrap_err()
                    .to_string(),
                "target_not_empty",
                "{busy}"
            );
        }
        assert_eq!(
            store
                .begin_playback_capture("gibt-es-nicht", 1)
                .unwrap_err()
                .to_string(),
            "meeting_not_found"
        );
    }

    #[test]
    fn a_deleted_entry_is_not_found() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET deleted_at = 5 WHERE id = ?1",
                params![id],
            )
            .unwrap();
        assert_eq!(
            store
                .begin_playback_capture(&id, 1)
                .unwrap_err()
                .to_string(),
            "meeting_not_found"
        );
    }

    /// Ein vorhandener Ton wird nie ueberschrieben: auch ohne Saetze (Aufnahme ohne
    /// erkannte Sprache) bleibt die Datei, der Nutzer kann sie neu transkribieren.
    #[test]
    fn an_entry_with_a_recording_on_disk_is_refused_so_no_audio_is_overwritten() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store
            .set_audio_paths(&id, None, Some("C:/m/system.wav"), Some(60_000))
            .unwrap();
        assert_eq!(
            store
                .begin_playback_capture(&id, 1)
                .unwrap_err()
                .to_string(),
            "target_not_empty"
        );
        assert_eq!(
            store
                .get_meeting(&id)
                .unwrap()
                .unwrap()
                .system_audio_path
                .as_deref(),
            Some("C:/m/system.wav")
        );
    }

    #[test]
    fn a_saved_variant_with_segments_counts_as_a_transcript() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store
            .get_connection()
            .unwrap()
            .execute(
                "INSERT INTO transcript_variants
                   (id, meeting_id, kind, segments_json, number, created_at, active)
                 VALUES ('v-x', ?1, 'subtitles_auto', '[{\"text\":\"x\"}]', 1, 1, 0)",
                params![id],
            )
            .unwrap();
        assert_eq!(
            store
                .begin_playback_capture(&id, 1)
                .unwrap_err()
                .to_string(),
            "target_not_empty"
        );
    }

    #[test]
    fn only_one_of_two_simultaneous_starts_wins() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let results: Vec<Result<(Meeting, PlaybackPrior)>> = (0..2)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let id = id.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.begin_playback_capture(&id, 1)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let won = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(won, 1, "genau ein Start gewinnt");
        let lost = results.into_iter().find_map(|r| r.err()).unwrap();
        assert_eq!(lost.to_string(), "target_not_empty");
    }

    #[test]
    fn a_failed_start_puts_the_entry_back_exactly_as_it_was() {
        let (_dir, store) = store();
        let (id, folder) = youtube_entry(&store);
        let before = store.get_meeting(&id).unwrap().unwrap();
        let (_, prior) = store.begin_playback_capture(&id, 55).unwrap();
        // Wie `start_row`: die Pfade stehen schon in der Datenbank, die Dauer ist weg.
        store
            .set_audio_paths(&id, None, Some("C:/m/system.wav"), None)
            .unwrap();
        store.set_status(&id, MeetingStatus::Failed).unwrap();

        assert!(store.restore_playback_target(&id, &prior).unwrap());

        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "ready");
        assert_eq!(after.source, "youtube");
        assert_eq!(after.consent_confirmed_at, None);
        assert_eq!(after.mic_audio_path, None);
        assert_eq!(after.system_audio_path, None);
        assert_eq!(after.duration_ms, before.duration_ms);
        assert_eq!(after.title, before.title);
        assert_eq!(store.meeting_folder_ids(&id).unwrap(), vec![folder]);
        // Und der Nutzer kann es noch einmal versuchen.
        assert!(store.begin_playback_capture(&id, 56).is_ok());
    }

    /// Nach dem Rueckfall sind keine Reste da, die der naechste App-Start als
    /// Absturz beleben koennte.
    #[test]
    fn a_restored_entry_is_not_a_crash_leftover() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        let (_, prior) = store.begin_playback_capture(&id, 1).unwrap();
        store.restore_playback_target(&id, &prior).unwrap();
        let status = store.get_meeting(&id).unwrap().unwrap().status;
        assert!(!crate::managers::meetings::recorder::is_orphan_status(
            &status
        ));
    }

    #[test]
    fn a_restore_never_touches_a_row_that_got_transcript_segments() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        let (_, prior) = store.begin_playback_capture(&id, 1).unwrap();
        store.append_delta(&id, &delta("Aufgenommen")).unwrap();

        assert!(!store.restore_playback_target(&id, &prior).unwrap());

        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "recording", "nichts wurde zurueckgedreht");
        assert_eq!(store.get_segments(&id).unwrap().len(), 1);
    }

    #[test]
    fn a_restore_leaves_other_sources_and_finished_rows_alone() {
        let (_dir, store) = store();
        let live = store
            .create_meeting("Live", MeetingSource::Live, Some(1))
            .unwrap();
        let prior = PlaybackPrior {
            status: "ready".into(),
            duration_ms: None,
            consent_confirmed_at: None,
        };
        assert!(!store.restore_playback_target(&live.id, &prior).unwrap());
        let (id, _) = youtube_entry(&store);
        // `ready` ist kein Startfehler: nichts zu tun.
        assert!(!store.restore_playback_target(&id, &prior).unwrap());
        assert!(!store
            .restore_playback_target("gibt-es-nicht", &prior)
            .unwrap());
    }
}
