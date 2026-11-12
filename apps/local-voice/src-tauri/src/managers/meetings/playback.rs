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
//! - **Vorzustand dauerhaft** (lva#538): Status, Einwilligung und Dauer des Players
//!   stehen NICHT nur im Arbeitsspeicher, sondern in derselben Transaktion, die den
//!   Eintrag zur Aufnahme macht, in `metadata_json.playback_prior`. Ein Absturz an
//!   irgendeiner Stelle des Starts verliert sie nie (Test
//!   `the_prior_state_survives_in_the_row_not_in_memory`).
//! - **Start scheitert** (kein Ausgabegeraet, Treiber haengt, Datei nicht anlegbar,
//!   Abbruch, Loeschen): `restore_playback_target` stellt aus dem gespeicherten
//!   Vorzustand `ready`, Einwilligung und Dauer des Players wieder her, leert die
//!   Audiopfade und entfernt den Merker. Haengt der Rueckfall selbst (Datenbank weg),
//!   steht die Zeile auf `failed` mit Merker; `recover_orphans` setzt sie beim
//!   naechsten App-Start zurueck.
//! - **Absturz mitten im Start oder nach einer Aufnahme ohne Ton**: der Eintrag steht
//!   auf `recording`, die WAV ist leer (nur Kopf) oder fehlt, es gibt keine Saetze:
//!   `recover_orphans` ruft `restore_playback_target` (`never_recorded` entscheidet),
//!   der Eintrag ist wieder `ready` mit seiner Dauer und erneut startbar. Mit Ton
//!   (auch ohne erkannte Sprache) laeuft die normale Wiederherstellung; ein Merker mit
//!   Ton wird entfernt (`discard_playback_prior`).
//! - **Abbruch mitten im Vorgang**: beide Schritte sind je EINE Transaktion; ein
//!   Absturz dazwischen hinterlaesst entweder den alten oder den neuen Zustand, nie
//!   einen halben.
//! - **Altdaten**: die Zeile stammt aus A2/A3 (`youtube/source.rs`); Notizen,
//!   Projekte, `metadata_json` und die vom Player nachgetragene Dauer bleiben
//!   (Test mit einer Zeile, die alles davon traegt; eine Zeile OHNE Merker, etwa
//!   aus der ersten Fassung, wird bei einem Rueckfall nicht angefasst).

use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::empty::TARGET_NOT_EMPTY;
use super::store::{
    metadata_object_tx, write_metadata_object_tx, Meeting, MeetingSource, MeetingStatus,
    MeetingStore,
};
use crate::audio_toolkit::audio::wav_writer::HEADER_LEN;

/// `meetings.source` der Besprechung, in die mitgeschrieben wird.
pub const SOURCE_YOUTUBE: &str = MeetingSource::Youtube.as_str();

/// Schluessel in `meetings.metadata_json`: der Vorzustand des Eintrags (keine Migration).
pub const PRIOR_KEY: &str = "playback_prior";

/// Was die Zeile vor dem Start war: damit ein gescheiterter Start (oder ein Absturz
/// im Start) sie genau so zurueckgibt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Recovery: hat dieser Mitschnitt nie etwas aufgenommen? Nur eine YouTube-Zeile mit
/// dem Merker des Vorzustands (`has_prior`) und ohne Ton auf der Platte: jede
/// Tonspur fehlt (NACHGEWIESEN: `NotFound`) oder ist hoechstens der WAV-Kopf. Mit Ton
/// (auch ohne erkannte Sprache) geht es den normalen Weg; ob es Saetze gibt, prueft
/// `restore_playback_target` selbst in der Transaktion.
///
/// lva#538, fail-closed: ein Dateizustand, der sich NICHT lesen laesst (Zugriff verweigert,
/// E/A-Fehler, Pfad kaputt), ist nicht "leer". Vorher machte `unwrap_or(true)` aus jedem
/// Fehler eine leere Spur, und die Wiederherstellung setzte eine Aufnahme zurueck und
/// loeschte ihr Audio, die sie nur nicht lesen konnte. Im Zweifel bleibt die Aufnahme.
pub fn never_recorded(meeting: &Meeting, has_prior: bool) -> bool {
    meeting.source == SOURCE_YOUTUBE && has_prior && audio_state(meeting) == AudioState::Empty
}

/// Was die Dateien einer Aufnahme hergeben (lva#538, Befund 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioState {
    /// Jede Tonspur fehlt nachweislich (`NotFound`) oder ist hoechstens der WAV-Kopf.
    Empty,
    /// Mindestens eine Spur hat Ton.
    Present,
    /// Mindestens eine Spur laesst sich nicht lesen (und keine hat nachweislich Ton): im Zweifel
    /// bleibt alles, wie es ist.
    Unknown,
}

/// Der Zustand der Tonspuren einer Besprechung. Ton auf einer Spur gewinnt vor einem Fehler auf
/// einer anderen; ein Fehler (nicht `NotFound`) macht eine sonst leere Aufnahme `Unknown`.
pub fn audio_state(meeting: &Meeting) -> AudioState {
    let mut unknown = false;
    for path in [&meeting.mic_audio_path, &meeting.system_audio_path]
        .into_iter()
        .flatten()
    {
        match std::fs::metadata(path) {
            Ok(m) if m.len() > HEADER_LEN => return AudioState::Present,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => unknown = true,
        }
    }
    if unknown {
        AudioState::Unknown
    } else {
        AudioState::Empty
    }
}

impl MeetingStore {
    /// Wandelt eine YouTube-Besprechung ohne Transkript in eine Aufnahme
    /// (`recording`, Einwilligung gesetzt) und merkt sich ihren Vorzustand in
    /// DERSELBEN Transaktion (`metadata_json.playback_prior`).
    /// Fehler: `meeting_not_found`, `target_not_empty` (siehe Moduldoku).
    pub fn begin_playback_capture(&self, id: &str, consent_at: i64) -> Result<Meeting> {
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
        // Der Vorzustand gehoert zur selben Entscheidung: ohne ihn gaebe es nach einem
        // Absturz keinen Weg zurueck (lva#538).
        let mut meta = metadata_object_tx(&tx, id)?;
        meta.insert(
            PRIOR_KEY.to_string(),
            serde_json::to_value(PlaybackPrior {
                status,
                duration_ms,
                consent_confirmed_at: consent,
            })?,
        );
        write_metadata_object_tx(&tx, id, meta, now)?;
        tx.commit()?;
        self.get_meeting(id)?
            .ok_or_else(|| anyhow!("meeting_not_found"))
    }

    /// Der gespeicherte Vorzustand (`None`: kein Merker oder nicht lesbar).
    pub fn playback_prior(&self, id: &str) -> Result<Option<PlaybackPrior>> {
        Ok(self
            .metadata_json(id)?
            .and_then(|meta| meta.get(PRIOR_KEY).cloned())
            .and_then(|value| serde_json::from_value(value).ok()))
    }

    /// Der Mitschnitt kam nicht zustande (Start gescheitert, abgebrochen, Absturz):
    /// die Zeile ist wieder die YouTube-Besprechung von vorher, mit Status,
    /// Einwilligung und Dauer aus dem GESPEICHERTEN Vorzustand und ohne Audiopfade;
    /// der Merker entfaellt. Greift nur, wenn der Merker da ist und die Zeile eine
    /// YouTube-Zeile im Zustand `recording`/`processing`/`failed` ohne Transkript-
    /// Saetze ist; sonst `false` und alles bleibt, wie es ist.
    pub fn restore_playback_target(&self, id: &str) -> Result<bool> {
        self.restore_playback_target_with(id, false)
    }

    /// lva#538: das Zuruecksetzen eines Starts, der NIE veroeffentlicht wurde (Abbruch,
    /// Fehler). Die Pipeline kann vor dem Abbruch schon Saetze gespeichert haben; sie
    /// gehoeren zu diesem Start (der Eintrag hatte vorher keine, `begin_playback_capture`
    /// prueft das) und werden in DERSELBEN Transaktion verworfen wie das Zuruecksetzen:
    /// entweder beides oder nichts. Ohne das verweigerte `restore_playback_target` (Saetze
    /// vorhanden) das Zuruecksetzen, nachdem der Aufrufer die Dateien schon geloescht hatte.
    /// Gesicherte Fassungen (`transcript_variants`) fasst es nie an: gibt es dort Saetze,
    /// bleibt alles, wie es ist (`false`).
    pub fn rollback_playback_capture(&self, id: &str) -> Result<bool> {
        self.restore_playback_target_with(id, true)
    }

    fn restore_playback_target_with(&self, id: &str, purge_live_segments: bool) -> Result<bool> {
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut meta = match metadata_object_tx(&tx, id) {
            Ok(meta) => meta,
            Err(e) if is_no_row(&e) => return Ok(false),
            Err(e) => return Err(e),
        };
        let Some(prior) = meta
            .get(PRIOR_KEY)
            .cloned()
            .and_then(|v| serde_json::from_value::<PlaybackPrior>(v).ok())
        else {
            return Ok(false);
        };
        if purge_live_segments {
            // Nur die aktive Fassung dieses Starts. Greift die Zeile unten nicht (Status,
            // gesicherte Fassung mit Saetzen), faellt die Transaktion zurueck: nichts wurde
            // verworfen.
            tx.execute(
                "DELETE FROM transcript_deltas
                  WHERE transcript_id IN (SELECT id FROM transcripts WHERE meeting_id = ?1)",
                params![id],
            )?;
            tx.execute(
                "UPDATE transcripts
                    SET segments_json = '[]', content_revision = content_revision + 1,
                        updated_at = ?2
                  WHERE meeting_id = ?1 AND segments_json != '[]'",
                params![id, now],
            )?;
        }
        let changed = tx.execute(
            &format!(
                "UPDATE meetings
                    SET status = ?2, consent_confirmed_at = ?3, duration_ms = ?4,
                        mic_audio_path = NULL, system_audio_path = NULL, updated_at = ?5
                  WHERE id = ?1 AND deleted_at IS NULL AND source = 'youtube'
                    AND status IN ('recording', 'processing', 'failed')
                    AND NOT {HAS_SEGMENTS}"
            ),
            params![
                id,
                prior.status,
                prior.consent_confirmed_at,
                prior.duration_ms,
                now
            ],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        meta.remove(PRIOR_KEY);
        write_metadata_object_tx(&tx, id, meta, now)?;
        tx.commit()?;
        Ok(true)
    }

    /// Der Mitschnitt hat Ton: der Merker des Vorzustands wird nicht mehr gebraucht.
    pub fn discard_playback_prior(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let mut conn = self.get_connection()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut meta = match metadata_object_tx(&tx, id) {
            Ok(meta) => meta,
            Err(e) if is_no_row(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        if meta.remove(PRIOR_KEY).is_some() {
            write_metadata_object_tx(&tx, id, meta, now)?;
            tx.commit()?;
        }
        Ok(())
    }
}

/// Gibt es die Zeile nicht (mehr)? (`query_row` ohne Treffer in `metadata_object_tx`)
fn is_no_row(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::QueryReturnedNoRows)
    )
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

        let meeting = store.begin_playback_capture(&id, 777).unwrap();

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
            store.playback_prior(&id).unwrap(),
            Some(PlaybackPrior {
                status: "ready".into(),
                duration_ms: Some(1_805_000),
                consent_confirmed_at: None
            })
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
        let results: Vec<Result<Meeting>> = (0..2)
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
        store.begin_playback_capture(&id, 55).unwrap();
        // Wie `start_row`: die Pfade stehen schon in der Datenbank, die Dauer ist weg.
        store
            .set_audio_paths(&id, None, Some("C:/m/system.wav"), None)
            .unwrap();
        store.set_status(&id, MeetingStatus::Failed).unwrap();

        assert!(store.restore_playback_target(&id).unwrap());

        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "ready");
        assert_eq!(after.source, "youtube");
        assert_eq!(after.consent_confirmed_at, None);
        assert_eq!(after.mic_audio_path, None);
        assert_eq!(after.system_audio_path, None);
        assert_eq!(after.duration_ms, before.duration_ms);
        assert_eq!(after.title, before.title);
        assert_eq!(store.meeting_folder_ids(&id).unwrap(), vec![folder]);
        // Der Merker ist weg, alles andere in `metadata_json` bleibt.
        assert_eq!(store.playback_prior(&id).unwrap(), None);
        let metadata = store.metadata_json(&id).unwrap().unwrap();
        assert_eq!(metadata["youtube"]["video_id"], "abcdefghijk");
        // Und der Nutzer kann es noch einmal versuchen.
        assert!(store.begin_playback_capture(&id, 56).is_ok());
    }

    /// lva#538: der Vorzustand steht in der Zeile, nicht im Arbeitsspeicher. Ein Absturz
    /// nach dem Anlegen (der Store ist weg, ein neuer Prozess oeffnet dieselbe Datei)
    /// verliert weder Einwilligung noch Dauer des Players noch den Weg zurueck.
    #[test]
    fn the_prior_state_survives_in_the_row_not_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("meetings.db");
        let id = {
            let store = MeetingStore::open_at(&db).unwrap();
            let (id, _) = youtube_entry(&store);
            store.begin_playback_capture(&id, 55).unwrap();
            // Die Dauer des Players geht beim Anlegen der Pfade verloren (`start_row`).
            store
                .set_audio_paths(&id, None, Some("C:/m/system.wav"), None)
                .unwrap();
            id
        };

        // Neuer Prozess: nur noch die Datei.
        let store = MeetingStore::open_at(&db).unwrap();
        assert_eq!(
            store.playback_prior(&id).unwrap(),
            Some(PlaybackPrior {
                status: "ready".into(),
                duration_ms: Some(1_805_000),
                consent_confirmed_at: None
            })
        );
        assert!(store.restore_playback_target(&id).unwrap());
        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(
            (after.status.as_str(), after.duration_ms),
            ("ready", Some(1_805_000)),
            "Dauer des Players und Status wie vor dem Start"
        );
        assert_eq!(after.system_audio_path, None);
        assert!(
            store.begin_playback_capture(&id, 56).is_ok(),
            "erneut startbar"
        );
    }

    /// Altdaten: eine Zeile ohne Merker (die erste Fassung kannte ihn nicht, oder jemand
    /// hat ihn entfernt) wird von einem Rueckfall nicht angefasst.
    #[test]
    fn a_row_without_the_marker_is_never_restored() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET status = 'recording' WHERE id = ?1",
                params![id],
            )
            .unwrap();
        assert!(!store.restore_playback_target(&id).unwrap());
        assert_eq!(
            store.get_meeting(&id).unwrap().unwrap().status,
            "recording",
            "nichts wurde zurueckgedreht"
        );
        assert!(!store.restore_playback_target("gibt-es-nicht").unwrap());
    }

    fn wav(path: &std::path::Path, seconds: u32) {
        let mut w = crate::audio_toolkit::audio::StreamingWavWriter::create(path, 16_000).unwrap();
        w.append(&vec![100i16; 16_000 * seconds as usize]).unwrap();
        w.finalize().unwrap();
    }

    fn orphan_with_paths(
        store: &MeetingStore,
        system: Option<&std::path::Path>,
    ) -> (String, Meeting) {
        let (id, _) = youtube_entry(store);
        store.begin_playback_capture(&id, 55).unwrap();
        if let Some(path) = system {
            store
                .set_audio_paths(&id, None, path.to_str(), None)
                .unwrap();
        }
        let meeting = store.get_meeting(&id).unwrap().unwrap();
        (id, meeting)
    }

    /// lva#538, Crash VOR dem Speichern der Pfade: `recording` ohne Pfad und ohne Ton.
    #[test]
    fn a_crash_before_the_paths_were_stored_is_a_start_that_never_recorded() {
        let (_dir, store) = store();
        let (id, meeting) = orphan_with_paths(&store, None);
        let has_prior = store.playback_prior(&id).unwrap().is_some();
        assert!(has_prior);
        assert!(never_recorded(&meeting, has_prior));
        assert!(store.restore_playback_target(&id).unwrap());
        assert_eq!(store.get_meeting(&id).unwrap().unwrap().status, "ready");
    }

    /// lva#538, Crash NACH dem Speichern der Pfade: die Pfade zeigen auf eine leere
    /// WAV (nur der Kopf), die Dauer des Players ist weg. Nach dem Rueckfall stimmt
    /// alles wieder, und der naechste Mitschnitt scheitert nicht an `target_not_empty`.
    #[test]
    fn a_crash_after_the_paths_were_stored_resets_to_a_startable_entry() {
        let (dir, store) = store();
        let system = dir.path().join("system.wav");
        drop(crate::audio_toolkit::audio::StreamingWavWriter::create(&system, 16_000).unwrap());
        let (id, meeting) = orphan_with_paths(&store, Some(&system));
        assert!(meeting.system_audio_path.is_some());
        // Voraussetzung (der Befund): `set_audio_paths(.., None)` hat die Dauer geloescht.
        assert_eq!(meeting.duration_ms, None, "Voraussetzung");
        assert!(never_recorded(&meeting, true));
        // Die Datei fehlt gar: ebenfalls nichts aufgenommen.
        let mut gone = meeting.clone();
        gone.system_audio_path = Some(dir.path().join("weg.wav").to_string_lossy().into_owned());
        assert!(never_recorded(&gone, true));

        assert!(store.restore_playback_target(&id).unwrap());

        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "ready");
        assert_eq!(
            after.system_audio_path, None,
            "keine Pfade auf eine leere WAV"
        );
        assert_eq!(
            after.duration_ms,
            Some(1_805_000),
            "Dauer des Players zurueck"
        );
        assert!(store.begin_playback_capture(&id, 56).is_ok());
    }

    /// Mit Ton auf der Platte ist es kein Start ohne Aufnahme: die normale
    /// Wiederherstellung holt ihn nach. Ebenso ohne Merker und fuer andere Quellen.
    #[test]
    fn a_recording_with_audio_is_not_reset() {
        let (dir, store) = store();
        let system = dir.path().join("system.wav");
        wav(&system, 3);
        let (id, meeting) = orphan_with_paths(&store, Some(&system));
        assert!(!never_recorded(&meeting, true));
        // Ein kaputter Kopf mit Daten dahinter (Absturz, Kopf noch auf null) ist Ton.
        let torn = dir.path().join("torn.wav");
        let mut bytes = std::fs::read(&system).unwrap();
        bytes[40..44].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(&torn, bytes).unwrap();
        let mut torn_meeting = meeting.clone();
        torn_meeting.system_audio_path = Some(torn.to_string_lossy().into_owned());
        assert!(!never_recorded(&torn_meeting, true));
        // Ohne Merker: nicht anfassen.
        assert!(!never_recorded(&meeting, false));
        let mut live = meeting.clone();
        live.source = "live".into();
        assert!(!never_recorded(&live, true));
        // Der Merker ist nach einer Aufnahme mit Ton ueberfluessig.
        store.discard_playback_prior(&id).unwrap();
        assert_eq!(store.playback_prior(&id).unwrap(), None);
        // Zweimal und fuer Unbekanntes: kein Fehler.
        store.discard_playback_prior(&id).unwrap();
        store.discard_playback_prior("gibt-es-nicht").unwrap();
    }

    /// Nach dem Rueckfall sind keine Reste da, die der naechste App-Start als
    /// Absturz beleben koennte.
    #[test]
    fn a_restored_entry_is_not_a_crash_leftover() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store.begin_playback_capture(&id, 1).unwrap();
        store.restore_playback_target(&id).unwrap();
        let status = store.get_meeting(&id).unwrap().unwrap().status;
        assert!(!crate::managers::meetings::recorder::is_orphan_status(
            &status
        ));
    }

    #[test]
    fn a_restore_never_touches_a_row_that_got_transcript_segments() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store.begin_playback_capture(&id, 1).unwrap();
        store.append_delta(&id, &delta("Aufgenommen")).unwrap();

        assert!(!store.restore_playback_target(&id).unwrap());

        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "recording", "nichts wurde zurueckgedreht");
        assert_eq!(store.get_segments(&id).unwrap().len(), 1);
    }

    /// lva#538 Befund 3: das Zuruecksetzen eines NIE veroeffentlichten Starts verwirft die Saetze,
    /// die die Pipeline vor dem Abbruch schon gespeichert hat, in DERSELBEN Transaktion wie das
    /// Zuruecksetzen. Das einfache `restore_playback_target` verweigert hier (Saetze da); der
    /// Aufrufer loeschte die Dateien vorher und blieb mit einer Zeile ohne Ton zurueck.
    #[test]
    fn a_rollback_discards_the_segments_of_the_aborted_start_and_restores_in_one_step() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store.begin_playback_capture(&id, 1).unwrap();
        store
            .append_delta(&id, &delta("Aus dem abgebrochenen Start"))
            .unwrap();
        assert!(
            !store.restore_playback_target(&id).unwrap(),
            "Voraussetzung: das einfache Zuruecksetzen verweigert"
        );

        assert!(store.rollback_playback_capture(&id).unwrap());

        let after = store.get_meeting(&id).unwrap().unwrap();
        assert_eq!(after.status, "ready");
        assert_eq!(after.system_audio_path, None);
        assert!(store.get_segments(&id).unwrap().is_empty());
        assert_eq!(store.playback_prior(&id).unwrap(), None);
        assert!(
            store.begin_playback_capture(&id, 2).is_ok(),
            "der Eintrag ist wieder startbar"
        );
    }

    /// Fail-closed: hat die Besprechung Saetze in einer gesicherten Fassung, aendert das
    /// Zuruecksetzen NICHTS (auch die Saetze der aktiven Fassung bleiben): die Transaktion
    /// faellt zurueck, entweder beides oder nichts.
    #[test]
    fn a_rollback_changes_nothing_when_a_saved_version_has_segments() {
        let (_dir, store) = store();
        let (id, _) = youtube_entry(&store);
        store.begin_playback_capture(&id, 1).unwrap();
        store.append_delta(&id, &delta("Live")).unwrap();
        store
            .get_connection()
            .unwrap()
            .execute(
                "INSERT INTO transcript_variants
                    (id, meeting_id, kind, segments_json, number, created_at, active)
                 VALUES ('v-alt', ?1, 'stt', '[{\"x\":1}]', 1, 1, 0)",
                params![id],
            )
            .unwrap();

        assert!(!store.rollback_playback_capture(&id).unwrap());

        assert_eq!(
            store.get_meeting(&id).unwrap().unwrap().status,
            "recording",
            "nichts wurde zurueckgedreht"
        );
        assert_eq!(
            store.get_segments(&id).unwrap().len(),
            1,
            "nichts verworfen"
        );
        assert!(store.playback_prior(&id).unwrap().is_some());
    }

    #[test]
    fn a_rollback_without_the_marker_or_for_unknown_rows_changes_nothing() {
        let (_dir, store) = store();
        let live = store
            .create_meeting("Live", MeetingSource::Live, Some(1))
            .unwrap();
        store.append_delta(&live.id, &delta("Bleibt")).unwrap();
        assert!(!store.rollback_playback_capture(&live.id).unwrap());
        assert_eq!(store.get_segments(&live.id).unwrap().len(), 1);
        assert!(!store.rollback_playback_capture("gibt-es-nicht").unwrap());
    }

    /// lva#538 Befund 7: ein Dateizustand, der sich nicht lesen laesst, ist NICHT "nie
    /// aufgenommen". Nur `NotFound` oder hoechstens der WAV-Kopf zaehlt als leer; jeder andere
    /// Fehler laesst die Aufnahme stehen (die Wiederherstellung loescht sonst Audio, das sie nur
    /// nicht lesen konnte).
    #[test]
    fn an_unreadable_audio_state_is_not_an_empty_recording() {
        let (dir, store) = store();
        let system = dir.path().join("system.wav");
        drop(crate::audio_toolkit::audio::StreamingWavWriter::create(&system, 16_000).unwrap());
        let (_id, meeting) = orphan_with_paths(&store, Some(&system));
        assert!(
            never_recorded(&meeting, true),
            "Voraussetzung: nachweislich leer"
        );

        // Ein Pfad, dessen Zustand sich nicht lesen laesst (kein NotFound).
        let mut unreadable = meeting.clone();
        unreadable.system_audio_path = Some("C:/m/sys\0tem.wav".to_string());
        assert!(!never_recorded(&unreadable, true));
        // Eine nachweislich leere Spur macht eine unlesbare nicht leer.
        unreadable.mic_audio_path = Some(system.to_string_lossy().into_owned());
        assert!(!never_recorded(&unreadable, true));
        // Fehlt die Datei nachweislich, ist es weiterhin leer.
        let mut gone = meeting.clone();
        gone.system_audio_path = Some(dir.path().join("weg.wav").to_string_lossy().into_owned());
        assert!(never_recorded(&gone, true));
    }

    #[test]
    fn a_restore_leaves_other_sources_and_finished_rows_alone() {
        let (_dir, store) = store();
        let live = store
            .create_meeting("Live", MeetingSource::Live, Some(1))
            .unwrap();
        assert!(!store.restore_playback_target(&live.id).unwrap());
        // Eine fertige YouTube-Zeile mit altem Merker: `ready` ist kein Startfehler.
        let (id, _) = youtube_entry(&store);
        store.begin_playback_capture(&id, 1).unwrap();
        store.set_status(&id, MeetingStatus::Ready).unwrap();
        assert!(!store.restore_playback_target(&id).unwrap());
        assert!(!store.restore_playback_target("gibt-es-nicht").unwrap());
    }
}
