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

    def test_only_gate_denial_releases_claim_and_clears_send_time(self):
        source = (ROOT / "src-tauri/src/managers/meetings/polls/mod.rs").read_text(encoding="utf-8")

        def check(text):
            self.assertIn('Ok(GateOutcome::Denied { .. }) => "pending"', text)
            self.assertIn('_ => "review"', text)
            self.assertIn('finish_send(&connection(&store)?,id,&person.adresse,&result)?', text)

        check(source)
        with self.assertRaises(AssertionError):
            check(source.replace('Ok(GateOutcome::Denied { .. }) => "pending"', 'Ok(GateOutcome::Denied { .. }) => "review"'))
        sql = re.search(r'"(UPDATE meeting_poll_people SET send_state=\?3.*?)"', source)[1]
        claim = re.search(r'"(UPDATE meeting_poll_people SET send_state=\'sending\'.*?)"', source)[1]
        self.poll()
        self.db.execute("UPDATE meeting_polls SET status='offen'")
        self.db.execute("INSERT INTO meeting_poll_people(poll_id,address,code,salt) VALUES ('T-ABCD','one@example.invalid','ABCD',0)")
        for state in ("pending", "sent", "review"):
            self.db.execute("UPDATE meeting_poll_people SET send_state='sending',sent_at=100")
            self.db.execute(sql, ("T-ABCD", "one@example.invalid", state))
            self.assertEqual(self.db.execute("SELECT send_state,sent_at FROM meeting_poll_people").fetchone(),
                             (state, None if state == "pending" else 100))
            self.assertEqual(self.db.execute(claim, ("T-ABCD", "one@example.invalid", 200)).rowcount,
                             int(state == "pending"))

    def test_page_notification_is_after_commit_before_return(self):
        source = (ROOT / "src-tauri/src/managers/meetings/polls/store.rs").read_text(encoding="utf-8")
        runtime = (ROOT / "src-tauri/src/managers/meetings/polls/mod.rs").read_text(encoding="utf-8")

        def check(text):
            page = text.split("fn apply_page(", 1)[1].split("fn review(", 1)[0]
            self.assertLess(page.index("tx.commit()"), page.index("if changed { notify(); }"))
            self.assertLess(page.index("if changed { notify(); }"), page.index("Ok((changed,"))

        check(source)
        with self.assertRaises((AssertionError, ValueError)):
            check(source.replace("if changed { notify(); }", ""))
        self.assertIn("&mut notify)?;", runtime)
        self.assertIn('runtime.sync(&svc,&store,&integration,|| updated(&app,""))', runtime)

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

    def test_draft_update_predicate_rejects_other_states_with_counterprobe(self):
        source = (ROOT / "src-tauri/src/managers/meetings/polls/store.rs").read_text(encoding="utf-8")
        sql = re.search(r'"(UPDATE meeting_polls SET definition_json=.*?)"', source)[1]
        self.poll()
        for status in ("entwurf", "offen", "entschieden", "abgebrochen"):
            self.db.execute("UPDATE meeting_polls SET status=?,definition_json='{}'", (status,))
            self.assertEqual(self.db.execute(sql, ("T-ABCD", '{"titel":"Synthetic"}')).rowcount,
                             int(status == "entwurf"))
        # Ohne Entwurf-Praedikat schreibt dieselbe Abfrage auch nach Abschluss.
        mutant = sql.replace(" AND status='entwurf'", "")
        self.assertNotEqual(mutant, sql)
        self.assertEqual(self.db.execute(mutant, ("T-ABCD", '{"titel":"Mutant"}')).rowcount, 1)

    def test_update_rolls_back_slot_deletion_on_insert_failure_with_counterprobe(self):
        source = (ROOT / "src-tauri/src/managers/meetings/polls/store.rs").read_text(encoding="utf-8")
        update = source.split("pub fn update(", 1)[1].split("pub fn get(", 1)[0]
        delete = re.search(r'"(DELETE FROM meeting_poll_slots.*?)"', update)[1]
        insert = re.search(r'"(INSERT INTO meeting_poll_slots.*?)"', update)[1]

        def contract(text):
            self.assertIn("TransactionBehavior::Immediate", text)
            self.assertLess(text.index("Transaction::new_unchecked"), text.index(delete))
            self.assertLess(text.index(insert), text.index("tx.commit()"))
            self.assertIn('return Err("poll_not_draft".into())', text)

        contract(update)
        for fragment in ("TransactionBehavior::Immediate", "tx.commit()", 'return Err("poll_not_draft".into())'):
            with self.assertRaises((AssertionError, ValueError)):
                contract(update.replace(fragment, "removed"))
        self.poll()
        self.db.execute(insert, ("T-ABCD", 3, 0, 1800000))
        self.db.execute("CREATE TRIGGER synthetic_failure BEFORE INSERT ON meeting_poll_slots BEGIN SELECT RAISE(ABORT,'synthetic'); END")
        self.db.commit()
        with self.assertRaises(sqlite3.IntegrityError):
            with self.db:
                self.db.execute("BEGIN IMMEDIATE")
                self.db.execute(delete, ("T-ABCD",))
                self.db.execute(insert, ("T-ABCD", 4, 1800000, 3600000))
        self.assertEqual(self.db.execute("SELECT nr FROM meeting_poll_slots").fetchall(), [(3,)])
        # Gegenprobe ohne gemeinsame Transaktion: das Loeschen bleibt bestehen.
        self.db.execute(delete, ("T-ABCD",))
        self.db.commit()
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute(insert, ("T-ABCD", 4, 1800000, 3600000))
        self.assertEqual(self.db.execute("SELECT nr FROM meeting_poll_slots").fetchall(), [])


if __name__ == "__main__":
    unittest.main()
