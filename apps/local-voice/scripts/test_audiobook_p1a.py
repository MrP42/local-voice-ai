"""Offline SQL/contract checks; Rust integration tests run separately in the app crate."""
import pathlib
import re
import sqlite3
import tempfile
import unittest
from contextlib import contextmanager

ROOT = pathlib.Path(__file__).resolve().parents[1]
MEETINGS = ROOT / "src-tauri/src/managers/meetings"


def sql(source, marker):
    """Use the production SQL literals, not copies of the statements under test."""
    matches = [text for text in re.findall(r'"([^"\\]*(?:\\.[^"\\]*)*)"', source)
               if marker in text and text.lstrip().startswith(("SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP"))]
    matches = [text for text in matches if text == marker] or list(dict.fromkeys(matches))
    if len(matches) != 1:
        raise AssertionError(f"expected one SQL statement for {marker!r}, got {len(matches)}")
    return matches[0]


def source(name):
    return (MEETINGS / name).read_text(encoding="utf-8")


class ClosingConnection(sqlite3.Connection):
    def __exit__(self, *args):
        try:
            return super().__exit__(*args)
        finally:
            self.close()


def connect(path):
    return sqlite3.connect(path, factory=ClosingConnection)


@contextmanager
def transaction(db):
    try:
        yield
        db.commit()
    except Exception:
        db.rollback()
        raise


def database(path=":memory:"):
    db = connect(path)
    db.executescript(sql(source("store.rs"), "CREATE TABLE meetings ("))
    db.execute("ALTER TABLE meetings ADD COLUMN source_path TEXT")
    db.executescript(sql(source("queue_store.rs"), "CREATE TABLE import_queue ("))
    db.executescript(sql(source("queue_store.rs"), "CREATE TABLE IF NOT EXISTS import_part ("))
    db.executescript((MEETINGS / "audiobook.sql").read_text(encoding="utf-8"))
    for name, kind in [("book", "audiobook"), ("other", "audiobook"), ("meeting", "meeting")]:
        db.execute("INSERT INTO meetings(id,title,status,source,created_at,updated_at,mic_audio_path) VALUES (?,?,'queued','import',0,0,?)", (name, name, f"synthetic-{name}.wav"))
        db.execute("INSERT INTO import_part(meeting_id,kind) VALUES (?,?)", (name, kind))
        db.execute("INSERT INTO import_queue VALUES (?,1,'waiting',0)", (name,))
    db.commit()
    return db


class AudiobookP1a(unittest.TestCase):
    def test_completion_claim_rejects_cancelled_and_old_generations(self):
        claim = sql(source("audiobook.rs"), "c.generation = ?2")
        with database() as db:
            db.execute("UPDATE meetings SET status = 'ready' WHERE id = 'book'")
            self.assertEqual(db.execute(claim, ("book", 0)).fetchone()[0], 1)
            update = sql(source("audiobook.rs"), "UPDATE audiobook_control SET desired_state")
            db.execute(update, ("book", "cancelled", False))
            self.assertEqual(db.execute(claim, ("book", 0)).fetchone()[0], 0)
            db.execute(update, ("book", "running", True))
            self.assertEqual(db.execute(claim, ("book", 0)).fetchone()[0], 0)
            self.assertEqual(db.execute(claim, ("book", 1)).fetchone()[0], 1)
            # Counterprobe: the previous job-ID-only guard accepts discarded text.
            old = "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)"
            self.assertEqual(db.execute(old, ("book",)).fetchone()[0], 1)

    def test_user_pause_yields_instead_of_waiting_with_a_slot(self):
        checkpoint = source("audiobook.rs").split("pub fn checkpoint", 1)[1].split("pub struct ThreadPriority", 1)[0]
        self.assertIn('if ctl.desired_state == "paused" { return Err(YIELDED.into()); }', checkpoint)
        runner = source("queue.rs")
        self.assertIn("Ok(RunOutcome::Yielded)", runner)
        self.assertIn("Err(e) if e == super::audiobook::YIELDED", source("import.rs"))

    def test_retained_wav_precedes_original_source_validation(self):
        runner = source("queue.rs").split("fn run_thread(", 1)[1]
        self.assertLess(runner.index("retained_audiobook_wav"), runner.index("source_problem"))
        self.assertIn("retained_audiobook_wav", source("import.rs"))

    def test_historical_upgrade_fixture_has_a_fixed_version(self):
        fixture = source("ki_freigabe_tests.rs").split("fn upgrade_von_stand_19", 1)[1]
        self.assertIn("MIGRATIONS[..20]", fixture)
        self.assertNotIn("MIGRATIONS.len() - 1", fixture)

    def test_historical_fixture_removes_later_schema_before_upgrade_with_counterprobe(self):
        cleanup = sql(source("ki_freigabe_tests.rs"), "DROP TRIGGER audiobook_part_insert")
        migration = (MEETINGS / "audiobook.sql").read_text(encoding="utf-8")
        for fixed in [False, True]:
            with database() as db:
                db.executescript("""
                    CREATE TABLE meeting_ai_revision(rev INTEGER);
                    CREATE TABLE meeting_ai_policy(policy TEXT);
                    CREATE TABLE meeting_ai_confirmations(meeting_id TEXT, einwilligungen_belegt INTEGER, rechtsgrundlage TEXT);
                """)
                old = "DROP TABLE meeting_ai_revision;" + cleanup.split("DROP TABLE meeting_ai_revision;", 1)[1]
                db.executescript(cleanup if fixed else old)
                if fixed:
                    db.executescript(migration)
                    self.assertEqual(db.execute("SELECT COUNT(*) FROM audiobook_control").fetchone()[0], 2)
                else:
                    with self.assertRaisesRegex(sqlite3.OperationalError, "already exists"):
                        db.executescript(migration)

    def test_health_allows_a_start_longer_than_five_seconds(self):
        source = (ROOT / "src-tauri/src/managers/tts/providers/phonon.rs").read_text(encoding="utf-8")
        timeout = int(re.search(r"const VERSION_TIMEOUT: Duration = Duration::from_secs\((\d+)\)", source)[1])
        self.assertEqual(timeout, 30)
        self.assertGreater(timeout, 6)
        timeout_arm = source.split("timeout(self.version_timeout", 1)[1].split("Ok(Err(e))", 1)[0]
        self.assertNotIn("CPU", timeout_arm)
        self.assertIn("Startprüfung hat das Zeitlimit überschritten", timeout_arm)

    def test_additive_migration_preserves_legacy_imports(self):
        migration = ROOT / "src-tauri/src/managers/meetings/audiobook.sql"
        self.assertTrue(migration.exists(), "durable audiobook control/cursor migration missing")
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE import_part(meeting_id TEXT PRIMARY KEY, kind TEXT, range_start_ms INTEGER, range_end_ms INTEGER)")
            db.execute("INSERT INTO import_part VALUES ('legacy', 'audiobook', 12, 24)")
            db.executescript(migration.read_text(encoding="utf-8"))
            self.assertEqual(db.execute("SELECT * FROM import_part").fetchone(), ("legacy", "audiobook", 12, 24))
            self.assertEqual(db.execute("SELECT desired_state, next_sample FROM audiobook_control").fetchone(), ("running", None))

    def test_user_pause_survives_database_reopen_and_blocks_claim(self):
        state_sql = sql(source("audiobook.rs"), "UPDATE audiobook_control SET desired_state")
        claim = sql(source("queue_store.rs"), "UPDATE import_queue SET state = ?2 WHERE meeting_id = ?1 AND state = ?3")
        with tempfile.TemporaryDirectory() as directory:
            path = str(pathlib.Path(directory) / "synthetic.db")
            with database(path) as db:
                db.execute(state_sql, ("book", "paused", True))
            with connect(path) as db:
                self.assertEqual(db.execute(claim, ("book", "running", "waiting")).rowcount, 0)
                self.assertEqual(db.execute(claim, ("meeting", "running", "waiting")).rowcount, 1)
                # Counterprobe: the old claim predicate ignores the persistent pause.
                old = claim.split("\n             AND NOT EXISTS", 1)[0]
                self.assertEqual(db.execute(old, ("book", "running", "waiting")).rowcount, 1)

    def test_pause_after_claim_is_visible_to_the_start_gate(self):
        with database() as db:
            claim = sql(source("queue_store.rs"), "UPDATE import_queue SET state = ?2 WHERE meeting_id = ?1 AND state = ?3")
            self.assertEqual(db.execute(claim, ("book", "running", "waiting")).rowcount, 1)
            db.execute(sql(source("audiobook.rs"), "UPDATE audiobook_control SET desired_state"), ("book", "paused", True))
            control = sql(source("audiobook.rs"), "SELECT desired_state, keep_partial, generation, next_sample")
            self.assertEqual(db.execute(control, ("book",)).fetchone()[0], "paused")
            # The SQL check alone cannot protect the gap after claim: require the
            # actual runner to check again before loading its engine.
            runner = source("queue.rs").split("fn run_request(", 1)[1]
            self.assertLess(runner.index("super::audiobook::checkpoint"), runner.index("tm.load_extra_engine"))

    def test_cursor_failure_rolls_back_delta_materialization_and_silence_receipt(self):
        with database() as db:
            db.execute(sql(source("audiobook.rs"), "UPDATE audiobook_control SET next_sample = ?2 WHERE meeting_id = ?1 AND next_sample IS NULL"), ("book", 0))
            db.execute("INSERT INTO transcripts(id,meeting_id,created_at,updated_at) VALUES ('transcript','book',0,0)")
            db.commit()
            db.execute("CREATE TRIGGER fail_cursor BEFORE UPDATE OF next_sample ON audiobook_control BEGIN SELECT RAISE(ABORT, 'synthetic'); END")
            insert_delta = sql(source("store.rs"), "INSERT INTO transcript_deltas (transcript_id, sequence, delta_json, created_at)")
            materialize = sql(source("store.rs"), "UPDATE transcripts SET segments_json = ?1, content_revision = content_revision + 1, updated_at = ?2")
            receipt = sql(source("audiobook.rs"), "INSERT INTO audiobook_chunk (meeting_id, generation, start_sample, end_sample)")
            advance = sql(source("audiobook.rs"), "UPDATE audiobook_control SET next_sample = ?2 WHERE meeting_id = ?1")
            for segments in ['[]', '[{"text":"synthetic"}]']:
                with self.assertRaises(sqlite3.IntegrityError):
                    with transaction(db):
                        db.execute(insert_delta, ("transcript", 1, segments, 0))
                        db.execute(materialize, (segments, 0, "transcript"))
                        db.execute(receipt, ("book", 0, 0, 320000))
                        db.execute(advance, ("book", 320000))
                self.assertEqual(db.execute("SELECT segments_json FROM transcripts").fetchone()[0], "[]")
                self.assertEqual(db.execute("SELECT COUNT(*) FROM transcript_deltas").fetchone()[0], 0)
                self.assertEqual(db.execute("SELECT COUNT(*) FROM audiobook_chunk").fetchone()[0], 0)
                self.assertEqual(db.execute("SELECT next_sample FROM audiobook_control WHERE meeting_id = 'book'").fetchone()[0], 0)
            # Counterprobe: committing the delta separately leaves text behind.
            db.execute(insert_delta, ("transcript", 1, "[]", 0))
            db.commit()
            with self.assertRaises(sqlite3.IntegrityError):
                with transaction(db):
                    db.execute(advance, ("book", 320000))
            self.assertEqual(db.execute("SELECT COUNT(*) FROM transcript_deltas").fetchone()[0], 1)
        body = source("audiobook.rs").split("pub fn append_audiobook_block", 1)[1].split("pub fn audiobook_discard_cancelled", 1)[0]
        self.assertLess(body.index("Self::append_delta_tx"), body.index("UPDATE audiobook_control SET next_sample"))
        self.assertEqual(body.count("tx.commit()?"), 1)

    def test_committed_silence_survives_reopen_and_duplicate_receipt_is_unique(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(pathlib.Path(directory) / "synthetic.db")
            with database(path) as db:
                receipt = sql(source("audiobook.rs"), "INSERT INTO audiobook_chunk (meeting_id, generation, start_sample, end_sample)")
                db.execute(receipt, ("book", 0, 0, 320000))
                db.execute(sql(source("audiobook.rs"), "UPDATE audiobook_control SET next_sample = ?2 WHERE meeting_id = ?1"), ("book", 320000))
            with connect(path) as db:
                self.assertEqual(db.execute("SELECT next_sample FROM audiobook_control WHERE meeting_id = 'book'").fetchone()[0], 320000)
                with self.assertRaises(sqlite3.IntegrityError):
                    db.execute(receipt, ("book", 0, 0, 320000))
            # Production must ACK the receipt before trying to write it again.
            body = source("audiobook.rs").split("pub fn append_audiobook_block", 1)[1]
            self.assertLess(body.index("if committed == Some(end) { return Ok(false); }"), body.index("Self::append_delta_tx"))

    def test_cancel_retention_and_generation_are_scoped_to_one_result(self):
        with database() as db:
            update = sql(source("audiobook.rs"), "UPDATE audiobook_control SET desired_state")
            retain = sql(source("audiobook.rs"), "m.status <> 'ready' AND c.keep_partial = 1")
            db.execute(update, ("book", "cancelled", True))
            self.assertEqual(db.execute(retain, ("book",)).fetchone()[0], 1)
            self.assertEqual(db.execute(retain, ("meeting",)).fetchone()[0], 0)
            db.execute(update, ("book", "cancelled", False))
            self.assertEqual(db.execute(retain, ("book",)).fetchone()[0], 0)
            self.assertEqual(db.execute("SELECT generation FROM audiobook_control WHERE meeting_id = 'book'").fetchone()[0], 1)
            self.assertEqual(db.execute("SELECT generation FROM audiobook_control WHERE meeting_id = 'other'").fetchone()[0], 0)
            self.assertEqual(db.execute("SELECT mic_audio_path FROM meetings WHERE id = 'book'").fetchone()[0], "synthetic-book.wav")
            # Counterprobe: the old unconditional retention path cannot distinguish these.
            self.assertNotEqual(db.execute(retain, ("other",)).fetchone()[0], 0)
        self.assertIn("store.audiobook_retains_audio(&meeting.id)?", source("retention.rs"))

    def test_completion_cannot_override_a_new_pause_or_cancel(self):
        with database() as db:
            finish = sql(source("audiobook.rs"), "UPDATE meetings SET status = 'ready' WHERE id = ?1")
            update = sql(source("audiobook.rs"), "UPDATE audiobook_control SET desired_state")
            for state in ["paused", "cancelled"]:
                db.execute(update, ("book", state, True))
                self.assertEqual(db.execute(finish, ("book",)).rowcount, 0)
            db.execute(update, ("book", "running", True))
            self.assertEqual(db.execute(finish, ("book",)).rowcount, 1)

    def test_changed_files_use_lf(self):
        paths = [
            "src-tauri/src/background_activity.rs", "src-tauri/src/commands/library_transcription.rs",
            "src-tauri/src/managers/meetings/audiobook.rs", "src-tauri/src/managers/meetings/audiobook.sql",
            "src/components/library/TranscriptionControls.tsx", "scripts/audiobook-p1a.test.mjs",
            "scripts/test_audiobook_p1a.py", "src-tauri/src/commands/mod.rs", "src-tauri/src/lib.rs",
            "src-tauri/src/managers/meetings/import.rs", "src-tauri/src/managers/meetings/job.rs",
            "src-tauri/src/managers/meetings/mod.rs", "src-tauri/src/managers/meetings/queue.rs",
            "src-tauri/src/managers/meetings/queue_store.rs", "src-tauri/src/managers/meetings/retention.rs",
            "src-tauri/src/managers/meetings/store.rs", "src-tauri/src/managers/transcription.rs",
            "src-tauri/src/managers/tts/mod.rs", "src-tauri/src/managers/tts/providers/phonon.rs",
            "src-tauri/src/managers/tts/providers/registry.rs", "src-tauri/src/settings.rs",
            "src/bindings.ts", "src/components/library/LibraryPage.tsx", "src/components/library/useLibrary.ts",
            "src/components/settings/meetings/MeetingModelSetting.tsx", "src/i18n/locales/de/translation.json",
            "src/i18n/locales/en/translation.json", "src/lib/library/importFile.ts", "src/stores/settingsStore.ts",
        ]
        for path in paths:
            self.assertTrue(b"\r" not in (ROOT / path).read_bytes(), path)


if __name__ == "__main__":
    unittest.main()
