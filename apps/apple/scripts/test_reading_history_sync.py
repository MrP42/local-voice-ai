"""Offline d3edb3203 client fixture and wiring guards; synthetic data only."""
import copy
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[3]


def old_client(objects, phone):
    # d3edb3203: LibrarySync.apply whitelist; engine.apply_remote default Ok(false).
    known = {"book", "progress", "bookmark", "page", "book_file"}
    writes = {}
    for (collection, ident), value in objects.items():
        if collection not in known:
            continue
        if collection == "progress":
            record = copy.deepcopy(value["record"])
            if phone:
                fields = {"offset", "anchor", "length", "heard", "read", "updated",
                          "finished", "unit", "frontier", "source_edition", "source_hash"}
                record = {k: v for k, v in record.items() if k in fields}
                for key, default in (("length", 0), ("heard", []), ("read", []), ("updated", 0)):
                    record.setdefault(key, default)
            record["offset"] = 2
            record["updated"] = 9
            writes[collection, ident] = {"key": value["key"], "record": record}
    return writes


class ReadingHistorySync(unittest.TestCase):
    def check_guard(self, source, tokens):
        def check(text):
            for token in tokens:
                self.assertTrue(token in text, "Missing wiring: " + token)
        check(source)
        for token in tokens:
            with self.assertRaises(AssertionError):
                check(source.replace(token, "REMOVED"))

    def test_fixture_matches_old_dispatch_and_writer(self):
        def old(path):
            return subprocess.check_output(["git", "show", "d3edb3203:" + path], cwd=ROOT).decode("utf-8")
        phone = old("apps/apple/iPhone/Library/LibrarySync.swift")
        self.check_guard(phone, ["guard collections.contains(remote.collection) else { return }",
                                 "for key in library.progressKeys"])
        self.assertLess(phone.index("guard collections.contains"), phone.index("guard remote.keyId == acc.keyId else { throw SyncError.wrongKey }", phone.index("let collections =")))
        desktop = old("apps/local-voice/src-tauri/src/sync/engine.rs")
        self.check_guard(desktop, ['if !remote.deleted && remote.key_id != my_key_id',
                                   'other => {', 'Ok(changed(false))'])
        ledger = desktop.split("const TOMBSTONED:", 1)[1].split("];", 1)[0]
        self.assertNotIn("reading_history", ledger)
        old_record = old("apps/apple/VoiceCore/Sources/VoiceCore/Library.swift")
        self.assertNotIn("reading_log", old_record)
        self.assertNotIn("coordinate_history", old_record)

    def test_old_writers_leave_separate_history_untouched(self):
        record = {"offset": 80, "updated": 1, "coordinate_history": {"raw": {"offset": 20}},
                  "reading_log": [{"device": "synthetic", "start_ms": 1, "end_ms": 31001}]}
        history = {"book": "book_test", "records": {"text:page_test": record}}
        for phone in (True, False):
            hub = {("reading_history", "book_test"): copy.deepcopy(history),
                   ("progress", "p_test"): {"key": "text:page_test", "record": record}}
            hub.update(old_client(hub, phone))
            self.assertEqual(hub["reading_history", "book_test"], history)
            self.assertNotIn(("reading_history", "book_test"), old_client(hub, phone))
        # Counterprobe: the old phone strips embedded history on replacement.
        legacy = {("progress", "p_test"): {"key": "text:page_test", "record": record}}
        self.assertNotIn("reading_log", old_client(legacy, True)["progress", "p_test"]["record"])

    def test_production_wiring(self):
        checks = {
            "apps/local-voice/src-tauri/src/sync/books.rs": ["COLLECTION_READING_HISTORY", "progress_wire_record", "history_objects"],
            "apps/local-voice/src-tauri/src/sync/engine.rs": ["books::COLLECTION_READING_HISTORY"],
            "apps/apple/VoiceCore/Sources/VoiceCore/LibrarySyncModel.swift": ["struct SyncedReadingHistory", "progressWireRecord"],
            "apps/apple/iPhone/Library/LibrarySync.swift": ["pushReadingHistory", "collectionReadingHistory"],
        }
        for path, tokens in checks.items():
            source = (ROOT / path).read_text(encoding="utf-8")
            with self.subTest(path=path):
                self.check_guard(source, tokens)

    def test_shared_account_key_path_and_replay(self):
        phone = (ROOT / "apps/apple/iPhone/Library/LibrarySync.swift").read_text(encoding="utf-8")
        history = phone.split("private func pushReadingHistory", 1)[1].split("private func pushVoicePrefs", 1)[0]
        progress = phone.split("private func pushProgress", 1)[1].split("private func pushReadingHistory", 1)[0]
        self.check_guard(history, ["sealObject(collection, history.book, value, base: entry?.revision ?? 0, acc)"])
        self.check_guard(progress, ["sealObject(LibrarySyncFormat.collectionProgress, id, value, base: entry?.revision ?? 0, acc)"])
        seal = phone.split("private func sealObject", 1)[1].split("private func push(", 1)[0]
        self.check_guard(seal, ["keyId: acc.keyId", "key: [UInt8](acc.key)"])
        self.check_guard(phone, ["var keyId: String { SyncCrypto.keyId([UInt8](key)) }"])
        self.check_guard(phone, ["if needsPages || needsHistory { ledger.cursor = 0 }", 'ledger.prepareReadingHistoryPull()', 'ledger.readingHistoryPulled = true'])
        engine = (ROOT / "apps/local-voice/src-tauri/src/sync/engine.rs").read_text(encoding="utf-8")
        # All LocalObjects, including progress and history, share this single builder.
        builder = engine.split("for o in &locals {", 1)[1].split("// Löschbremse", 1)[0]
        self.check_guard(builder, ["collection: o.collection.to_string()", "key_id: my_key_id.clone()"])
        self.check_guard(engine, ["let key = cfg.key().map_err(HubError::Other)?", "let my_key_id = crypto::key_id(&key)", 'led.prepare_reading_history_pull()',
                                  "remote.collection != books::COLLECTION_READING_HISTORY"])
        self.check_guard((ROOT / "apps/local-voice/src-tauri/src/sync/ledger.rs").read_text(encoding="utf-8"),
                         ["RESCAN_LEVEL: u32 = 2"])

    def test_phone_oversize_is_object_local_and_run_continues(self):
        phone = (ROOT / "apps/apple/iPhone/Library/LibrarySync.swift").read_text(encoding="utf-8")
        history = phone.split("private func pushReadingHistory", 1)[1].split("private func pushVoicePrefs", 1)[0]
        self.check_guard(history, ['ledger.holdOversizedReadingHistory(history.book, hash: hash, bytes: value.count)',
                                   'markDead(collection, history.book, hash, "Eine Lesehistorie ist zu groß für den Abgleich (too_large).")',
                                   'continue'])
        self.assertNotIn('throw SyncError.corrupt', history)
        run = phone.split('func run() async', 1)[1].split('private func pull', 1)[0]
        steps = ['try await pull(acc)', 'try await pushReadingHistory(acc)',
                 'try await pushProgress(acc)', 'try await pushVoicePrefs(acc)']
        self.assertEqual(sorted(run.index(step) for step in steps), [run.index(step) for step in steps])

    def test_desktop_upgrade_pulls_all_history_before_collecting_or_pushing(self):
        engine = (ROOT / "apps/local-voice/src-tauri/src/sync/engine.rs").read_text(encoding="utf-8")
        self.assertTrue('if led.prepare_reading_history_pull() {' in engine, 'Missing pre-push history pull')
        bootstrap = engine.split('if led.prepare_reading_history_pull() {', 1)[1].split('// ---- Sammeln', 1)[0]
        self.check_guard(bootstrap, ['let mut since = 0;', 'client.pull(since, PULL_LIMIT).await?',
                                    'if remote.collection != books::COLLECTION_READING_HISTORY',
                                    'apply_remote(app, cfg, &key, remote, None, &mut led.files)',
                                    'led.reading_history_pulled = true;', 'if !page.more {'])
        # Failure paths must leave the durable marker unset; no own-echo shortcut.
        self.assertLess(bootstrap.index('apply_remote('), bootstrap.index('led.reading_history_pulled = true;'))
        self.assertLess(bootstrap.index('if !page.more {'), bootstrap.index('led.reading_history_pulled = true;'))
        self.assertNotIn('remote.device_id == cfg.device_id', bootstrap)
        self.assertLess(engine.index('if led.prepare_reading_history_pull()'), engine.index('// ---- Sammeln'))
        self.assertLess(engine.index('// ---- Sammeln'), engine.index('// ---- Push'))

    def test_history_marker_is_ledger_scoped_and_old_entries_are_unread(self):
        swift = (ROOT / "apps/apple/VoiceCore/Sources/VoiceCore/LibrarySyncModel.swift").read_text(encoding="utf-8")
        self.check_guard(swift, ['decodeIfPresent(Bool.self, forKey: .readingHistoryPulled) ?? false',
                                 'guard !readingHistoryPulled else { return false }',
                                 'entries = entries.filter { !$0.key.hasPrefix(prefix) }'])
        rust = (ROOT / "apps/local-voice/src-tauri/src/sync/ledger.rs").read_text(encoding="utf-8")
        self.check_guard(rust, ['#[serde(default)]\n    pub reading_history_pulled: bool',
                                'if self.reading_history_pulled {',
                                'self.objects.retain(|k, _| !k.starts_with("reading_history/"))'])
        phone = (ROOT / "apps/apple/iPhone/Library/LibrarySync.swift").read_text(encoding="utf-8")
        self.assertNotIn('"sync.readingHistoryGeholt"', phone)
        pull = phone.split('private func pull(', 1)[1].split('private func apply(', 1)[0]
        self.assertLess(pull.index('while more {'), pull.index('ledger.readingHistoryPulled = true'))
        self.check_guard(pull, ['do { try apply(remote, acc) }', 'more = page.more'])

    def test_migration_and_wire_projection(self):
        books = (ROOT / "apps/local-voice/src-tauri/src/sync/books.rs").read_text(encoding="utf-8")
        projection = books.split("pub fn progress_wire_record", 1)[1].split("fn history_objects", 1)[0]
        self.check_guard(projection, ['m.remove("coordinate_history")', 'm.remove("reading_log")'])
        self.check_guard(books, ['"record": progress_wire_record(&record)',
                                 'out.extend(history_objects(map, &owners))',
                                 'let changed = apply_history_map(&mut map, id, value)?'])
        swift = (ROOT / "apps/apple/VoiceCore/Sources/VoiceCore/LibrarySyncModel.swift").read_text(encoding="utf-8")
        self.check_guard(swift, ['out.readingLog = []', 'out.extra["coordinate_history"] = nil'])
        phone = (ROOT / "apps/apple/iPhone/Library/LibrarySync.swift").read_text(encoding="utf-8")
        self.check_guard(phone, ['record: LibrarySyncFormat.progressWireRecord(rec)',
                                 'try library.mergeReadingHistory(history)'])


if __name__ == "__main__":
    unittest.main()
