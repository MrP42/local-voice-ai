"""Offline SQL regression for the production queue context lookup (synthetic data)."""
import re
import sqlite3
import unittest
from pathlib import Path


SOURCE = (Path(__file__).resolve().parents[1] /
          "src-tauri/src/managers/meetings/audiobook_run.rs").read_text(encoding="utf-8")
BODY = SOURCE.split("pub fn audiobook_contexts(", 1)[1].split("pub fn audiobook_settle(", 1)[0]
SQL = re.search(r'conn.prepare\("([^"]+)"\)', BODY).group(1)
MEETINGS = Path(__file__).resolve().parents[1] / "src-tauri/src/managers/meetings"
STORE = (MEETINGS / "store.rs").read_text(encoding="utf-8")
CONTROL = (MEETINGS / "audiobook.sql").read_text(encoding="utf-8")
ADOPTION = re.search(r'M::up\("(CREATE TABLE audiobook_adoption[^"]+)"\)', STORE).group(1)
CONTEXT = re.search(r'AUDIOBOOK_MIGRATION: &str = "([^"]+)";', SOURCE).group(1)
STEPS = [sql for _, sql in sorted([
    (STORE.index("M::up(super::audiobook::MIGRATION)"), CONTROL),
    (STORE.index('M::up("CREATE TABLE audiobook_adoption'), ADOPTION),
    (STORE.index("M::up(super::audiobook_run::AUDIOBOOK_MIGRATION)"), CONTEXT),
])]


def lookup(sql):
    with sqlite3.connect(":memory:") as db:
        db.executescript("""
            CREATE TABLE audiobook_runs (id TEXT PRIMARY KEY, book_id TEXT,
                requested_model_id TEXT, effective_model_id TEXT, language_mode TEXT,
                language TEXT, language_source TEXT, language_state TEXT,
                model_fallback_reason TEXT);
            CREATE TABLE import_part (meeting_id TEXT PRIMARY KEY, audiobook_run_id TEXT);
            INSERT INTO audiobook_runs VALUES
                ('history', 'book', NULL, 'model_A', 'auto', 'en', 'metadata', 'resolved', NULL),
                ('active', 'book', NULL, 'model_X', 'fr', 'fr', 'setting', 'resolved', NULL);
        """)
        db.executemany("INSERT INTO import_part VALUES (?, 'history')",
                       ((f"old_{i}",) for i in range(10_000)))
        db.execute("INSERT INTO import_part VALUES ('candidate', 'active')")
        # Bound the actual SQLite work as well as the returned rows: no history scan.
        steps = 0

        def progress():
            nonlocal steps
            steps += 1
            return steps > 1000

        db.set_progress_handler(progress, 1)
        try:
            return db.execute(sql, ("candidate",) if "?1" in sql else ()).fetchall()
        finally:
            db.set_progress_handler(None, 0)


class ContextLookupTests(unittest.TestCase):
    def test_only_candidate_is_read_from_large_history(self):
        rows = lookup(SQL)
        self.assertEqual([(row[0], row[3], row[5], row[9]) for row in rows],
                         [("active", "model_X", "fr", "candidate")])

    def test_counterprobe_unfiltered_lookup_exceeds_work_budget(self):
        unfiltered = SQL.replace("p.meeting_id=?1 AND ", "")
        with self.assertRaises(sqlite3.OperationalError):
            lookup(unfiltered)


def upgrade_control(steps):
    for prefix in (1, 2):
        with sqlite3.connect(":memory:") as db:
            db.execute("PRAGMA foreign_keys=ON")
            db.executescript("""
                CREATE TABLE import_part (meeting_id TEXT PRIMARY KEY, kind TEXT);
                INSERT INTO import_part VALUES ('synthetic', 'audiobook');
            """)
            for step in steps[:prefix]:
                db.executescript(step)
            tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            assert "audiobook_control" in tables
            assert "audiobook_runs" not in tables, "run context must follow both P1a steps"
            assert ("audiobook_adoption" in tables) == (prefix == 2)
            db.execute("""UPDATE audiobook_control SET desired_state='paused', generation=7,
                          next_sample=320000 WHERE meeting_id='synthetic'""")
            if prefix == 2:
                db.execute("INSERT INTO audiobook_adoption VALUES ('synthetic', 'book', 'token')")
            for step in steps[prefix:]:
                db.executescript(step)
            assert db.execute("SELECT desired_state, generation, next_sample FROM audiobook_control").fetchone() == (
                "paused", 7, 320000)
            assert db.execute("SELECT COUNT(*) FROM audiobook_adoption").fetchone()[0] == prefix - 1
            assert db.execute("SELECT audiobook_run_id FROM import_part").fetchone() == (None,)
            # Neue Auftraege erhalten beide unabhaengigen Zuordnungen per Trigger.
            db.execute("INSERT INTO audiobook_runs VALUES ('run', 'book', NULL, 'model', 'auto', 'en', 'probe', 'resolved', NULL)")
            db.execute("INSERT INTO import_part VALUES ('new', 'audiobook', 'run')")
            assert db.execute("SELECT desired_state FROM audiobook_control WHERE meeting_id='new'").fetchone() == ("running",)
            assert db.execute("SELECT run_id FROM audiobook_run_jobs WHERE meeting_id='new'").fetchone() == ("run",)


class MigrationMergeTests(unittest.TestCase):
    def test_p1a_prefixes_upgrade_without_changing_pause_cursor_or_claim(self):
        upgrade_control(STEPS)

    def test_counterprobe_context_before_p1a_breaks_historical_prefix(self):
        with self.assertRaises(AssertionError):
            upgrade_control([CONTROL, CONTEXT, ADOPTION])


if __name__ == "__main__":
    unittest.main()
