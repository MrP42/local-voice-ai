"""Offline wiring guards; behavioural Rust tests belong to the build thread."""
from pathlib import Path
import re
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[1] / "src-tauri"
FILES = ["src/audiobook_worker/parent.rs", "src/managers/meetings/import.rs",
         "src/managers/meetings/queue.rs", "src/managers/transcription.rs"]


def contract(files):
    parent, imp, queue, tm = (files[n] for n in FILES)
    assert "MAX_RESTARTS: usize = 2" in parent
    assert "audiobook_invalidate_worker" in parent
    assert "reserve(" in parent
    assert "worker_fallback_code" in parent
    assert "parent::Binding::enter(" in imp
    assert "if !audiobook {" in imp.split("// Kick the model load FIRST", 1)[1].split("// P8a:", 1)[0]
    assert "slot == Slot::Extra && !audiobook" in queue
    assert "parent::stop_books(ids" in queue
    body = tm.split("pub fn transcribe_segments_detecting", 1)[1]
    assert body.index("parent::infer(") < body.index("self.touch_activity()")
    assert body.index("frei.einloesen()") < body.index("parent::infer(")


class Wiring(unittest.TestCase):
    def test_production_invalidation_sql_keeps_cursor_and_rejects_old_generation(self):
        source = (ROOT / "src/managers/meetings/audiobook.rs").read_text(encoding="utf-8")
        body = source.split("pub fn audiobook_invalidate_worker", 1)[1].split("pub fn audiobook_claim", 1)[0]
        sql = re.search(r'"(UPDATE audiobook_control[^\"]+)"', body).group(1)

        def check(statement):
            with sqlite3.connect(":memory:") as db:
                db.executescript("""
                    CREATE TABLE audiobook_control(meeting_id TEXT PRIMARY KEY, generation INTEGER, next_sample INTEGER);
                    INSERT INTO audiobook_control VALUES ('synthetic', 7, 160), ('other', 3, 320);
                """)
                db.execute(statement, ("synthetic", 7))
                db.execute(statement, ("synthetic", 7))
                self.assertEqual(db.execute("SELECT generation,next_sample FROM audiobook_control WHERE meeting_id='synthetic'").fetchone(), (8, 160))
                self.assertEqual(db.execute("SELECT generation,next_sample FROM audiobook_control WHERE meeting_id='other'").fetchone(), (3, 320))

        check(sql)
        for old, new in [("generation + 1", "generation + 0"),
                         ("AND generation = ?2", "AND ?2 = ?2")]:
            with self.subTest(mutation=old), self.assertRaises(AssertionError):
                check(sql.replace(old, new))

    def test_wiring_and_counterprobes(self):
        files = {name: (ROOT / name).read_text(encoding="utf-8") if (ROOT / name).exists() else ""
                 for name in FILES}
        contract(files)
        for name, old in [(FILES[0], "MAX_RESTARTS: usize = 2"),
                          (FILES[0], "audiobook_invalidate_worker"),
                          (FILES[0], "reserve("), (FILES[0], "worker_fallback_code"),
                          (FILES[1], "parent::Binding::enter("),
                          (FILES[2], "slot == Slot::Extra && !audiobook"),
                          (FILES[2], "parent::stop_books(ids"),
                          (FILES[3], "parent::infer(")]:
            with self.subTest(removed=old):
                changed = dict(files)
                changed[name] = changed[name].replace(old, "removed")
                with self.assertRaises((AssertionError, ValueError)):
                    contract(changed)


if __name__ == "__main__":
    unittest.main()
