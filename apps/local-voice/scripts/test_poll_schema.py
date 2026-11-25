"""Offline SQL checks for TF2; no app database or network access."""
import pathlib
import re
import sqlite3
import unittest
import json

ROOT = pathlib.Path(__file__).resolve().parents[1]


class PollSchema(unittest.TestCase):
    def setUp(self):
        source = ROOT / "src-tauri/src/managers/meetings/polls/schema.rs"
        self.assertTrue(source.exists(), "TF2 migration missing")
        sql = re.search(r'r#"(.*?)"#', source.read_text(encoding="utf-8"), re.S)[1]
        self.db = sqlite3.connect(":memory:", cached_statements=0)
        self.db.execute("PRAGMA foreign_keys=ON")
        self.db.executescript(sql)

    def poll(self, ident="T-ABCD"):
        self.db.execute("INSERT INTO meeting_polls(id,integration_id,account_key,definition_json,status,created_at) VALUES (?,?,?,'{}','entwurf',0)", (ident, "synthetic", "tenant/account"))

    def test_codes_collide_only_inside_one_poll(self):
        self.poll()
        self.poll("T-EFGH")
        insert = "INSERT INTO meeting_poll_people(poll_id,address,code,salt) VALUES (?,?,?,0)"
        self.db.execute(insert, ("T-ABCD", "one@example.invalid", "ABCD"))
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute(insert, ("T-ABCD", "two@example.invalid", "ABCD"))
        self.db.execute(insert, ("T-EFGH", "two@example.invalid", "ABCD"))

    def test_deduplication_retains_history_and_cascades(self):
        self.poll()
        insert = "INSERT INTO meeting_poll_answers(poll_id,message_id,sender,received_at,result_json) VALUES ('T-ABCD',?,'one@example.invalid',0,'{}')"
        self.db.execute(insert, ("m1",))
        self.db.execute(insert, ("m2",))
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute(insert, ("m1",))
        self.assertEqual(self.db.execute("SELECT count(*) FROM meeting_poll_answers").fetchone()[0], 2)
        self.db.execute("DELETE FROM meeting_polls")
        self.assertEqual(self.db.execute("SELECT count(*) FROM meeting_poll_answers").fetchone()[0], 0)

    def test_no_mail_content_columns(self):
        for table in ("meeting_poll_answers", "meeting_poll_sync", "meeting_poll_people"):
            names = {row[1] for row in self.db.execute(f"PRAGMA table_info({table})")}
            self.assertFalse(names & {"body", "subject", "display_name", "headers"})

    def test_delta_column_accepts_only_encrypted_secret_slot_references(self):
        self.db.execute("INSERT INTO meeting_poll_sync(integration_id,account_key,delta_token) VALUES ('synthetic','tenant','poll_delta0')")
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute("UPDATE meeting_poll_sync SET delta_token='https://graph.example.invalid/delta?secret=SYNTHETIC'")

    def test_production_send_claim_is_once_only_and_stops_after_close(self):
        source = (ROOT / "src-tauri/src/managers/meetings/polls/mod.rs").read_text(encoding="utf-8")
        sql = re.search(r'"(UPDATE meeting_poll_people SET send_state=\'sending\'.*?)"', source)[1]
        self.poll()
        self.db.execute("UPDATE meeting_polls SET status='offen'")
        self.db.execute("INSERT INTO meeting_poll_people(poll_id,address,code,salt) VALUES ('T-ABCD','one@example.invalid','ABCD',0)")
        args = ("T-ABCD", "one@example.invalid", 100)
        self.assertEqual(self.db.execute(sql, args).rowcount, 1)
        self.assertEqual(self.db.execute(sql, args).rowcount, 0)
        mutant = sql.replace(" AND send_state='pending'", "")
        self.assertEqual(self.db.execute(mutant, args).rowcount, 1)
        self.db.execute("UPDATE meeting_poll_people SET send_state='pending'")
        self.db.execute("UPDATE meeting_polls SET status='abgebrochen'")
        self.assertEqual(self.db.execute(sql, args).rowcount, 0)

    def test_counterprobes_detect_removed_constraints(self):
        source = ROOT / "src-tauri/src/managers/meetings/polls/schema.rs"
        sql = re.search(r'r#"(.*?)"#', source.read_text(encoding="utf-8"), re.S)[1]
        # Mutationen nur in frischen In-Memory-Datenbanken, nie im Arbeitsbaum.
        for constraint in ("UNIQUE(poll_id, code)", "UNIQUE(poll_id, message_id)"):
            mutant = sqlite3.connect(":memory:")
            mutant.executescript(sql.replace(",\n    " + constraint, ""))
            definition = "INSERT INTO meeting_polls(id,integration_id,account_key,definition_json,status,created_at) VALUES ('T-ABCD','synthetic','tenant','{}','entwurf',0)"
            mutant.execute(definition)
            if "code" in constraint:
                mutant.execute("INSERT INTO meeting_poll_people(poll_id,address,code,salt) VALUES ('T-ABCD','one@example.invalid','ABCD',0)")
                mutant.execute("INSERT INTO meeting_poll_people(poll_id,address,code,salt) VALUES ('T-ABCD','two@example.invalid','ABCD',0)")
                table = "meeting_poll_people"
            else:
                insert = "INSERT INTO meeting_poll_answers(poll_id,message_id,sender,received_at,result_json) VALUES ('T-ABCD','same','one@example.invalid',0,'{}')"
                mutant.execute(insert)
                mutant.execute(insert)
                table = "meeting_poll_answers"
            self.assertEqual(mutant.execute(f"SELECT count(*) FROM {table}").fetchone()[0], 2)

    def test_production_queries_prepare_against_the_migration(self):
        queries = []
        directory = ROOT / "src-tauri/src/managers/meetings/polls"
        for name in ("store.rs", "mod.rs"):
            source = (directory / name).read_text(encoding="utf-8")
            for literal in re.findall(r'"((?:[^"\\]|\\.)*)"', source):
                if re.match(r"^(SELECT|INSERT|UPDATE|DELETE) ", literal):
                    queries.append(json.loads('"' + literal + '"'))
        self.assertGreaterEqual(len(queries), 20)
        for sql in queries:
            count = max([int(n) for n in re.findall(r"\?(\d+)", sql)] or [0])
            self.db.execute("EXPLAIN " + sql, [None] * count)
        # Die Gegenprobe macht genau dieselben Abfragen mit inkompatiblem Schema.
        self.db.execute("ALTER TABLE meeting_poll_answers RENAME COLUMN result_json TO missing_result")
        with self.assertRaises(sqlite3.OperationalError):
            for sql in queries:
                count = max([int(n) for n in re.findall(r"\?(\d+)", sql)] or [0])
                self.db.execute("EXPLAIN " + sql, [None] * count)


if __name__ == "__main__":
    unittest.main()
