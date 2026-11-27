"""Offline SQL regression; the Rust MCP test covers the complete call path."""

from pathlib import Path
from contextlib import closing
import re
import sqlite3
import tempfile
import threading
import unittest


STORE = Path(__file__).parents[1] / "src-tauri/src/managers/meetings/store.rs"


def write_sql(with_fix=True):
    source = STORE.read_text(encoding="utf-8")
    body = source.split("pub fn update_segment_text(", 1)[1].split(
        "pub fn upsert_document(", 1
    )[0]
    update = re.search(r'"(UPDATE transcripts SET[^"]+)"', body).group(1)
    statements = [(update, ('[{"text":"synthetic new"}]', 2, "t"))]
    if with_fix and "bump_ki_revision(&tx)?" in body:
        helper = source.split("fn bump_ki_revision(", 1)[1].split("\n}", 1)[0]
        bump = re.search(r'"(UPDATE meeting_ai_revision[^"]+)"', helper).group(1)
        statements.append((bump, ()))
    return statements


class TranscriptAiRevision(unittest.TestCase):
    def race(self, with_fix):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "synthetic.db")
            with closing(sqlite3.connect(path)) as reader:
                reader.executescript("""
                    CREATE TABLE meeting_ai_revision (id INTEGER PRIMARY KEY, rev INTEGER);
                    INSERT INTO meeting_ai_revision VALUES (1, 0);
                    CREATE TABLE transcripts (id TEXT, segments_json TEXT,
                        content_revision INTEGER, updated_at INTEGER);
                    INSERT INTO transcripts VALUES ('t', 'synthetic old', 1, 1);
                """)
                before = reader.execute("SELECT rev FROM meeting_ai_revision").fetchone()[0]
                # Freigabe hat den alten, bestaetigten Stand gesehen. Der zweite
                # Anschluss schreibt vor dem Nutzlastlesen; kein sleep/Rennen auf Zeit.
                errors = []

                def writer():
                    try:
                        with closing(sqlite3.connect(path)) as connection:
                            with connection:
                                for sql, args in write_sql(with_fix):
                                    connection.execute(sql, args)
                    except BaseException as error:
                        errors.append(error)

                thread = threading.Thread(target=writer, daemon=True)
                thread.start()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive(), "writer timed out")
                self.assertFalse(errors, "writer failed")
                self.assertEqual(reader.execute(
                    "SELECT content_revision FROM transcripts"
                ).fetchone()[0], 2)
                self.assertEqual(reader.execute(
                    "SELECT segments_json FROM transcripts"
                ).fetchone()[0], '[{"text":"synthetic new"}]')
                after = reader.execute("SELECT rev FROM meeting_ai_revision").fetchone()[0]
                return before != after

    def test_parallel_edit_is_detected_by_final_revision_check(self):
        self.assertTrue(self.race(True), "parallel transcript edit escaped final check")

    def test_counterprobe_without_bump_escapes_final_check(self):
        self.assertFalse(self.race(False))

    def test_failed_revision_write_rolls_back_transcript(self):
        with closing(sqlite3.connect(":memory:")) as connection:
            connection.executescript("""
                CREATE TABLE transcripts (id TEXT, segments_json TEXT,
                    content_revision INTEGER, updated_at INTEGER);
                INSERT INTO transcripts VALUES ('t', 'synthetic old', 1, 1);
            """)
            with self.assertRaises(sqlite3.OperationalError):
                with connection:
                    for sql, args in write_sql():
                        connection.execute(sql, args)
            self.assertEqual(connection.execute(
                "SELECT content_revision FROM transcripts"
            ).fetchone()[0], 1)


if __name__ == "__main__":
    unittest.main()
