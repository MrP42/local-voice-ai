//! M8 meetings: audio retention policy and the hard-delete cascade (Spec A2).
//!
//! `retention_until` is pure — it only decides *when* a meeting's audio
//! expires, never touches the filesystem or the store. `purge_due_audio` and
//! `delete_audio_files` do the actual I/O: the former sweeps the store for
//! meetings whose `audio_retention_until` is in the past (startup + the
//! `AfterMinutes` fast path right after a protocol is generated), the latter
//! is the shared file-deletion primitive also used by the soft-delete
//! cascade (a tombstoned meeting must never leave an orphaned WAV behind).

use log::warn;

use super::store::{Meeting, MeetingStore};

/// How long a meeting's audio survives after the meeting ends.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingAudioRetention {
    /// Delete the audio as soon as a minutes document exists for the
    /// meeting (the spec default — the transcript plus protocol are the
    /// durable record, the raw audio is not).
    AfterMinutes,
    /// Keep the audio for a fixed number of days after the meeting ended.
    Days(u32),
    /// Never delete the audio automatically.
    Forever,
}

const SECONDS_PER_DAY: i64 = 86_400;

/// Pure: determines the audio's expiry timestamp, or `None` if it must not
/// be deleted (yet). `AfterMinutes` only expires once a minutes document
/// exists — before that, `has_minutes` is `false` and the audio is kept
/// indefinitely, same as `Forever`.
pub fn retention_until(
    policy: &MeetingAudioRetention,
    now_unix: i64,
    ended_at: i64,
    has_minutes: bool,
) -> Option<i64> {
    match policy {
        MeetingAudioRetention::AfterMinutes => has_minutes.then_some(now_unix),
        MeetingAudioRetention::Days(days) => Some(ended_at + (*days as i64) * SECONDS_PER_DAY),
        MeetingAudioRetention::Forever => None,
    }
}

/// Outcome of one attempted file deletion. `removed` is true only when this
/// call actually unlinked the file (for the "files actually deleted"
/// counters). `cleared` is true when the DB path may safely be nulled —
/// either `removed`, or the file was already gone (`NotFound`, e.g. a
/// previous sweep that died between deleting the file and clearing the
/// path). Any other error (permission denied, file still open, ...) leaves
/// `cleared` false so the caller keeps both the path and the retention
/// marker for a retry.
struct DeleteOutcome {
    path: String,
    removed: bool,
    cleared: bool,
}

fn delete_audio_file(path: &str) -> DeleteOutcome {
    let (removed, cleared) = match std::fs::remove_file(path) {
        Ok(()) => (true, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, true),
        Err(e) => {
            // Path only, never file content — D9 log privacy.
            warn!("meetings: could not delete audio file {path}: {e}");
            (false, false)
        }
    };
    DeleteOutcome {
        path: path.to_string(),
        removed,
        cleared,
    }
}

/// Pure decision logic: given a meeting's current mic/system paths and the
/// delete outcome for each path that was attempted, decides which of the two
/// DB columns may be cleared. A path that was never attempted (already
/// `None`) counts as cleared (there is nothing to keep). Exposed separately
/// from the filesystem I/O so the retry-on-failure behaviour is unit
/// testable without depending on a platform-specific way to make a file
/// undeletable.
fn paths_to_clear(
    mic: Option<&str>,
    system: Option<&str>,
    outcomes: &[DeleteOutcome],
) -> (bool, bool) {
    let was_cleared = |candidate: &str| {
        outcomes
            .iter()
            .find(|o| o.path == candidate)
            .map(|o| o.cleared)
            .unwrap_or(true)
    };
    (
        mic.map(was_cleared).unwrap_or(true),
        system.map(was_cleared).unwrap_or(true),
    )
}

/// Hard-deletes one meeting's own audio files and updates its DB state
/// honestly: a path (and, if both are gone, the retention marker) is only
/// cleared once its file is actually gone (removed now, or already
/// `NotFound`). If deletion fails for another reason (locked file,
/// permissions, ...) the path AND the retention marker are kept so the next
/// sweep retries instead of silently losing the pointer to audio that is
/// still on disk. Returns the number of files actually removed. Shared by
/// `purge_due_audio` (the startup/periodic sweep) and the `AfterMinutes`
/// fast path right after a protocol is generated (`minutes::generate_minutes`).
pub fn purge_meeting_audio(store: &MeetingStore, meeting: &Meeting) -> u32 {
    let paths = meeting.audio_paths();
    let outcomes: Vec<DeleteOutcome> = paths.iter().map(|p| delete_audio_file(p)).collect();
    // M2-P2c2: die entechote Kopie der Mikrofonspur (`mic_aec.wav`) teilt das
    // Schicksal der `mic.wav`; ist sie nicht loeschbar, bleibt auch der
    // mic-Pfad fuer den naechsten Versuch stehen.
    let derived: Vec<DeleteOutcome> = meeting
        .mic_audio_path
        .as_deref()
        .map(super::derived_audio_paths)
        .unwrap_or_default()
        .iter()
        .map(|p| delete_audio_file(p))
        .collect();
    let deleted = outcomes.iter().chain(&derived).filter(|o| o.removed).count() as u32;

    let (clear_mic, clear_system) = paths_to_clear(
        meeting.mic_audio_path.as_deref(),
        meeting.system_audio_path.as_deref(),
        &outcomes,
    );
    let clear_mic = clear_mic && derived.iter().all(|o| o.cleared);
    let new_mic = if clear_mic {
        None
    } else {
        meeting.mic_audio_path.as_deref()
    };
    let new_system = if clear_system {
        None
    } else {
        meeting.system_audio_path.as_deref()
    };
    if let Err(e) = store.set_audio_paths(&meeting.id, new_mic, new_system, meeting.duration_ms) {
        warn!(
            "meetings: purge could not update audio paths for {}: {e}",
            meeting.id
        );
    }

    if clear_mic && clear_system {
        if let Err(e) = store.set_retention_until(&meeting.id, None) {
            warn!(
                "meetings: purge could not clear retention marker for {}: {e}",
                meeting.id
            );
        }
    } else {
        warn!(
            "meetings: purge kept {} due for retry — at least one audio file could not be deleted",
            meeting.id
        );
    }

    deleted
}

/// Hard-deletes every audio file whose meeting is due (`audio_retention_until
/// <= now_unix`). See `purge_meeting_audio` for the per-meeting honesty
/// guarantee. Returns the number of files actually removed across all due
/// meetings.
pub fn purge_due_audio(store: &MeetingStore, now_unix: i64) -> anyhow::Result<u32> {
    let due = store.meetings_with_due_audio(now_unix)?;
    let mut deleted = 0u32;
    for meeting in due {
        deleted += purge_meeting_audio(store, &meeting);
    }
    Ok(deleted)
}

/// Hard-deletes the given audio files from disk unconditionally. Used by the
/// soft-delete cascade (`meetings_delete`), which already nulled the DB
/// paths as part of `soft_delete_meeting`'s own transaction and so has no
/// path state left to keep on a failed deletion — unlike `purge_due_audio`,
/// there is nothing here to retry from. A file that is already missing is
/// not an error — deleting is idempotent. Returns the number of files
/// actually removed.
pub fn delete_audio_files(paths: &[String]) -> u32 {
    paths
        .iter()
        .flat_map(|p| std::iter::once(p.clone()).chain(super::derived_audio_paths(p)))
        .map(|p| delete_audio_file(&p))
        .filter(|o| o.removed)
        .count() as u32
}

/// #15: der Ordner einer geloeschten Besprechung (`<meetings>/<id>`). `meetings_delete`
/// loeschte bisher nur die Audiodateien aus der Datenbank; der Ordner blieb mit
/// allem, was die App daneben ablegt (Protokoll als Markdown, `import.wav` ohne
/// DB-Eintrag, Reste), fuer immer liegen. Gibt `Ok(true)` zurueck, wenn er
/// entfernt wurde, `Ok(false)`, wenn es ihn nicht gab. Eine `meeting_id`, die kein
/// einzelner Ordnername ist (`..`, Trennzeichen, leer, absolut), wird abgelehnt,
/// ohne etwas zu loeschen: die ID kommt zwar aus der Datenbank, aber ein
/// `remove_dir_all` auf einem zusammengesetzten Pfad prueft sich selbst.
pub fn remove_meeting_dir(base: &std::path::Path, meeting_id: &str) -> Result<bool, String> {
    use std::path::Component;
    let mut parts = std::path::Path::new(meeting_id).components();
    let one_name = matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None));
    // Auch ein Rueckwaertsstrich (unter Linux ein gueltiger Name) zaehlt als Trenner:
    // die IDs sind ULIDs, nie etwas anderes.
    if !one_name || meeting_id.contains(['/', '\\']) {
        return Err("meeting_dir_invalid_id".to_string());
    }
    match std::fs::remove_dir_all(base.join(meeting_id)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        // Nur der Pfad im Detail, nie Inhalt (Log-Datenschutz).
        Err(e) => Err(format!("meeting_dir_remove_failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::MeetingSource;
    use super::*;

    fn store() -> (MeetingStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let s = MeetingStore::open_at(&path).unwrap();
        (s, dir)
    }

    #[test]
    fn after_minutes_policy_expires_once_minutes_exist() {
        let p = MeetingAudioRetention::AfterMinutes;
        assert_eq!(retention_until(&p, 1_000, 900, false), None);
        assert_eq!(retention_until(&p, 1_000, 900, true), Some(1_000));
    }

    #[test]
    fn days_policy_counts_from_meeting_end() {
        let p = MeetingAudioRetention::Days(3);
        assert_eq!(
            retention_until(&p, 0, 1_000_000, false),
            Some(1_000_000 + 3 * 86_400)
        );
    }

    #[test]
    fn forever_never_expires() {
        assert_eq!(
            retention_until(&MeetingAudioRetention::Forever, 5, 1, true),
            None
        );
    }

    /// Regression for the review finding: a `Days(n)` expiry must stay
    /// anchored to the meeting's real `ended_at` (persisted via
    /// `set_ended_at`, mirroring what `stop()`/import do) even when it is
    /// recomputed much later — e.g. once minutes are generated well after
    /// the meeting ended. Recomputing with "now" instead of the stored
    /// `ended_at` would silently push the expiry further into the future
    /// every time it's recomputed.
    #[test]
    fn days_retention_stays_anchored_to_stored_ended_at_when_recomputed_later() {
        let (s, _dir) = store();
        let meeting = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();

        let ended_at = 1_000_000;
        s.set_ended_at(&meeting.id, ended_at).unwrap();

        let policy = MeetingAudioRetention::Days(3);
        let expected = Some(ended_at + 3 * 86_400);

        // Computed right at meeting end (mirrors `stop()`).
        let now_at_end = ended_at + 100;
        let until_at_end = retention_until(&policy, now_at_end, ended_at, false);
        s.set_retention_until(&meeting.id, until_at_end).unwrap();
        assert_eq!(until_at_end, expected);

        // Recomputed much later (mirrors minutes generated after a delay):
        // reading `ended_at` back from the store, not substituting "now".
        let now_much_later = ended_at + 500_000;
        let stored = s.get_meeting(&meeting.id).unwrap().unwrap();
        let until_later = retention_until(&policy, now_much_later, stored.ended_at.unwrap(), true);
        assert_eq!(
            until_later, expected,
            "Days(n) expiry must not drift when recomputed later"
        );
    }

    #[test]
    fn purge_deletes_files_and_clears_paths() {
        let (s, dir) = store();
        let meeting = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();

        let mic_path = dir.path().join("mic.wav");
        let system_path = dir.path().join("system.wav");
        std::fs::write(&mic_path, b"RIFF....WAVEfmt ").unwrap();
        std::fs::write(&system_path, b"RIFF....WAVEfmt ").unwrap();

        s.set_audio_paths(
            &meeting.id,
            mic_path.to_str(),
            system_path.to_str(),
            Some(1_000),
        )
        .unwrap();
        // Due in the past relative to the `now_unix` passed to purge below.
        s.set_retention_until(&meeting.id, Some(500)).unwrap();

        let deleted = purge_due_audio(&s, 1_000).unwrap();
        assert_eq!(deleted, 2);
        assert!(!mic_path.exists());
        assert!(!system_path.exists());

        let stored = s.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(stored.mic_audio_path, None);
        assert_eq!(stored.system_audio_path, None);
        // #15: sind beide Dateien weg, faellt auch die Marke; sonst meldete sich
        // die Besprechung bei jedem Start erneut als "faellig".
        assert_eq!(stored.audio_retention_until, None, "Marke wird geleert");
        assert!(
            s.meetings_with_due_audio(1_000_000).unwrap().is_empty(),
            "nichts mehr faellig"
        );
    }

    // -- Review finding #2: purge must not claim deletion that didn't happen --

    #[test]
    fn paths_to_clear_keeps_a_path_whose_deletion_failed() {
        let outcomes = vec![
            DeleteOutcome {
                path: "mic.wav".into(),
                removed: false,
                cleared: false, // e.g. locked / permission denied
            },
            DeleteOutcome {
                path: "system.wav".into(),
                removed: true,
                cleared: true,
            },
        ];
        let (clear_mic, clear_system) =
            paths_to_clear(Some("mic.wav"), Some("system.wav"), &outcomes);
        assert!(!clear_mic, "an undeletable file must keep its DB path");
        assert!(clear_system, "a successfully deleted file clears its path");
    }

    #[test]
    fn paths_to_clear_clears_a_path_that_was_already_gone() {
        let outcomes = vec![DeleteOutcome {
            path: "mic.wav".into(),
            removed: false, // NotFound: nothing to unlink
            cleared: true,  // but nothing left to keep a pointer to either
        }];
        let (clear_mic, clear_system) = paths_to_clear(Some("mic.wav"), None, &outcomes);
        assert!(
            clear_mic,
            "NotFound clears the path just like a real delete"
        );
        assert!(clear_system, "a path that was never set is trivially clear");
    }

    #[test]
    fn purge_clears_the_path_when_the_file_is_already_gone() {
        let (s, dir) = store();
        let meeting = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();

        // Points at a file that was never written / already removed by hand.
        let missing_path = dir.path().join("missing.wav");
        s.set_audio_paths(&meeting.id, missing_path.to_str(), None, Some(1_000))
            .unwrap();
        s.set_retention_until(&meeting.id, Some(500)).unwrap();

        let deleted = purge_due_audio(&s, 1_000).unwrap();
        assert_eq!(deleted, 0, "nothing was actually unlinked");

        let stored = s.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            stored.mic_audio_path, None,
            "a NotFound file still clears its path — there's nothing to retry"
        );
        assert_eq!(stored.audio_retention_until, None);
    }

    /// Deterministic simulation of an undeletable file. Two more obvious
    /// approaches were tried and rejected here because Rust's std on Windows
    /// works around them (both let the delete through, which would make
    /// this test flake on a false negative): a plain open read handle
    /// (`std::fs::File` opens with `FILE_SHARE_DELETE`, so `DeleteFileW`
    /// succeeds while a reader holds it), and a read-only attribute
    /// (`std::fs::remove_file` clears `FILE_ATTRIBUTE_READONLY` and retries
    /// before giving up, to match Unix `unlink` semantics). What reliably
    /// fails on every platform: `remove_file` refuses a path that is a
    /// directory rather than a file, with neither `Ok` nor `NotFound` — a
    /// stand-in for "a file that exists at the stored path but cannot be
    /// removed by `remove_file`".
    #[test]
    fn purge_keeps_path_and_marker_when_the_file_cannot_be_deleted() {
        let (s, dir) = store();
        let meeting = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();

        // A directory at the audio path: `remove_file` errors on it (neither
        // `Ok` nor `NotFound`) on every platform, deterministically.
        let mic_path = dir.path().join("mic.wav");
        std::fs::create_dir(&mic_path).unwrap();

        s.set_audio_paths(&meeting.id, mic_path.to_str(), None, Some(1_000))
            .unwrap();
        s.set_retention_until(&meeting.id, Some(500)).unwrap();

        let deleted = purge_due_audio(&s, 1_000).unwrap();

        assert_eq!(deleted, 0, "the locked file could not be removed");
        assert!(mic_path.exists(), "file must survive an undeletable purge");

        let stored = s.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(
            stored.mic_audio_path.as_deref(),
            mic_path.to_str(),
            "path must be kept so a real orphan isn't created"
        );
        assert!(
            stored.audio_retention_until.is_some(),
            "retention marker must survive so the next sweep retries"
        );
    }

    #[test]
    fn soft_delete_cascade_removes_files_from_disk() {
        let (s, dir) = store();
        let meeting = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();

        let mic_path = dir.path().join("mic.wav");
        std::fs::write(&mic_path, b"RIFF....WAVEfmt ").unwrap();
        s.set_audio_paths(&meeting.id, mic_path.to_str(), None, Some(1_000))
            .unwrap();

        let paths = s.soft_delete_meeting(&meeting.id).unwrap();
        let deleted = delete_audio_files(&paths);

        assert_eq!(deleted, 1);
        assert!(!mic_path.exists());
    }

    // -- M2-P2c2: mic_aec.wav teilt das Schicksal der mic.wav --

    #[test]
    fn purge_and_delete_also_remove_the_echo_free_mic_track() {
        let (s, dir) = store();
        let meeting = s.create_meeting("T", MeetingSource::Live, Some(1)).unwrap();
        let mic_path = dir.path().join("mic.wav");
        let aec_path = dir.path().join(super::super::MIC_AEC_FILE);
        std::fs::write(&mic_path, b"RIFF....WAVEfmt ").unwrap();
        std::fs::write(&aec_path, b"RIFF....WAVEfmt ").unwrap();
        s.set_audio_paths(&meeting.id, mic_path.to_str(), None, Some(1_000))
            .unwrap();
        s.set_retention_until(&meeting.id, Some(500)).unwrap();

        assert_eq!(purge_due_audio(&s, 1_000).unwrap(), 2);
        assert!(!mic_path.exists());
        assert!(!aec_path.exists(), "Aufbewahrung gilt auch fuer die Kopie");
        let stored = s.get_meeting(&meeting.id).unwrap().unwrap();
        assert_eq!(stored.mic_audio_path, None);

        // Loeschen der Besprechung (soft delete + Dateien).
        let meeting = s.create_meeting("T2", MeetingSource::Live, Some(1)).unwrap();
        std::fs::write(&mic_path, b"RIFF....WAVEfmt ").unwrap();
        std::fs::write(&aec_path, b"RIFF....WAVEfmt ").unwrap();
        s.set_audio_paths(&meeting.id, mic_path.to_str(), None, Some(1_000))
            .unwrap();
        let paths = s.soft_delete_meeting(&meeting.id).unwrap();
        assert_eq!(delete_audio_files(&paths), 2);
        assert!(!aec_path.exists());
    }

    // -- #15: der Ordner einer geloeschten Besprechung geht mit --

    #[test]
    fn deleting_a_meeting_removes_its_whole_folder_including_what_the_db_does_not_know() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("01ABC");
        std::fs::create_dir_all(folder.join("unter")).unwrap();
        std::fs::write(folder.join("mic.wav"), b"RIFF").unwrap();
        std::fs::write(folder.join("Protokoll.md"), b"# Protokoll").unwrap();
        std::fs::write(folder.join("import.wav"), b"RIFF").unwrap();
        std::fs::write(folder.join("unter").join("rest.bin"), b"x").unwrap();
        let other = dir.path().join("01XYZ");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("mic.wav"), b"RIFF").unwrap();

        assert_eq!(remove_meeting_dir(dir.path(), "01ABC"), Ok(true));
        assert!(!folder.exists());
        assert!(other.join("mic.wav").exists(), "eine andere Besprechung bleibt");
        // Idempotent: den Ordner gibt es nicht mehr.
        assert_eq!(remove_meeting_dir(dir.path(), "01ABC"), Ok(false));
    }

    #[test]
    fn a_meeting_id_that_is_not_one_folder_name_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("meetings");
        std::fs::create_dir_all(base.join("01ABC")).unwrap();
        let victim = dir.path().join("fremd");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("wichtig.txt"), b"x").unwrap();

        for bad in ["", ".", "..", "../fremd", "01ABC/..", "a/b", "a\\b", "C:\\Windows", "/etc"] {
            assert!(
                remove_meeting_dir(&base, bad).is_err(),
                "{bad:?} muss abgelehnt werden"
            );
        }
        assert!(victim.join("wichtig.txt").exists());
        assert!(base.join("01ABC").exists());
    }

    #[test]
    fn a_missing_base_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            remove_meeting_dir(&dir.path().join("gibt-es-nicht"), "01ABC"),
            Ok(false)
        );
    }
}
