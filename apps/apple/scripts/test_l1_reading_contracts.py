"""Offline wiring guards for #795 L1; behavior lives in XCTest/Rust/Node.

Each guard has an in-memory counterprobe. No builds, network or app state.
Run: python -B apps/apple/scripts/test_l1_reading_contracts.py
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    return source.split(start, 1)[1].split(end, 1)[0]


class ReadingContracts(unittest.TestCase):
    def guard(self, path, required):
        source = read(path)

        def check(text):
            for token in required:
                self.assertTrue(token in text, "Vertrag fehlt: " + token)

        check(source)
        for token in required:
            with self.subTest(counterprobe=token):
                with self.assertRaises(AssertionError):
                    check(source.replace(token, "REMOVED"))

    def test_session_lifecycle(self):
        self.guard("apps/apple/iPhone/Library/LibraryPlayback.swift", [
            "if playing { beginReadingSession() } else { checkpointReadingSession(ending: true) }",
            "else { checkpointReadingSession(ending: !playing); library.flush() }",
            "if active { VoiceCatalog.shared.refresh(); beginReadingSession() }",
            "library.noteReading(entry, bookId: id)",
            "medium: medium, from: readingOffset",
            "device: library.readingDevice",
        ])

    def test_session_core(self):
        self.guard("apps/apple/VoiceCore/Sources/VoiceCore/Library.swift", [
            "uptime - startedUptime >= 30",
            "entry.to = max(entry.to, to)",
            "devices[entry.device]?[entry.startMs]",
            "merged.endMs = max(old.endMs, entry.endMs); merged.to = max(old.to, entry.to)",
            "case readingLog = \"reading_log\"",
            "try raw.decode(PassthroughJSON.self, forKey: key)",
            "let known = [reach(older), reach(newer)]",
            "out.extra[\"coordinate_history\"] = .object(history)",
        ])

    def test_rejected_checkpoint_does_not_change_wall_clock(self):
        source = read("apps/apple/VoiceCore/Sources/VoiceCore/Library.swift")
        checkpoint = section(source, "public mutating func checkpoint(", "/// Wie `ProgressRecord`")

        def check(text):
            threshold = "guard uptime - startedUptime >= 30 else { return nil }"
            self.assertIn(threshold, text)
            self.assertLess(text.index(threshold), text.index("entry.endMs ="))

        check(checkpoint)
        # Gegenprobe: dieselbe Prüfung hinter der Mutation lässt die Wanduhr durch.
        threshold = "guard uptime - startedUptime >= 30 else { return nil }"
        late = checkpoint.replace(threshold, "") + threshold
        with self.assertRaises(AssertionError):
            check(late)

    def test_first_coordinate_union_is_normalized(self):
        self.guard("apps/apple/VoiceCore/Sources/VoiceCore/Library.swift", [
            "?? mergeCurrent(saved, saved)",
            "?? mergeCurrent(plain, plain)",
        ])

    def test_jump_without_matching_coordinates_keeps_decided_frontier(self):
        source = read("apps/apple/VoiceCore/Sources/VoiceCore/LiveTranslation.swift")
        saving = section(source, "public static func saving(", "\n    }\n}")

        def check(text):
            merge = "var merged = stored.map { LibraryProgress.merge(stamped, $0) } ?? stamped"
            restore = "if jumped, previous == nil { merged.frontier = stamped.frontier }"
            self.assertIn(restore, text)
            self.assertIn(merge, text)
            self.assertLess(text.index(merge), text.index(restore))
            self.assertLess(text.index(restore), text.index("return merged"))

        check(saving)
        # Gegenproben: ohne Ausnahme wird der Sprung zum Altstand; ohne Bedingung sinkt ein bekannter Stand.
        restore = "if jumped, previous == nil { merged.frontier = stamped.frontier }"
        for replacement in ("", "merged.frontier = stamped.frontier",
                            "if jumped { merged.frontier = stamped.frontier }"):
            with self.subTest(counterprobe=replacement):
                with self.assertRaises(AssertionError):
                    check(saving.replace(restore, replacement))

    def test_phone_progress_storage(self):
        self.guard("apps/apple/iPhone/Library/PhoneLibrary.swift", [
            'let key = "reading:\\(bookId)"',
            "rec.readingLog = ReadingLogEntry.union(rec.readingLog + [entry])",
            "kurzbuchParents[entry.id] = book.id",
            "LibraryProgress.merge(rec, $0)",
        ])

    def test_desktop_progress(self):
        self.guard("apps/local-voice/src-tauri/src/sync/books.rs", [
            'merged.insert("reading_log".into(), merge_reading_log(',
            'merged.insert("coordinate_history".into(), serde_json::to_value(coordinates).ok()?)',
            'let frontier = reach(older).max(reach(newer));',
            'let key = (device.to_string(), start.to_string());',
        ])
        self.guard("apps/local-voice/src-tauri/src/library.rs", [
            "crate::sync::books::merge_record(&next, &old)",
        ])
        self.guard("apps/local-voice/src-tauri/src/sync/uebernahme_kern.rs", [
            '"text" | "audio" | "doc" | "reading"',
        ])

    def test_description_phone(self):
        self.guard("apps/apple/VoiceCore/Sources/VoiceCore/BookText.swift", [
            'book.description = meta["dc:description"]?.first.flatMap(descriptionText)',
            "func parser(_ p: XMLParser, foundCDATA data: Data)",
            'stripDescriptionTags(decoded).split(whereSeparator: \\.isWhitespace)',
        ])
        self.guard("apps/apple/iPhone/Library/PhoneLibrary.swift", [
            "if book.description?.isEmpty != false { book.description = extracted.description }",
        ])

    def test_description_desktop(self):
        self.guard("apps/local-voice/src/lib/library/importFile.ts", [
            "commands.libraryEpubDescription(path)",
            "const description = file.description ?? (candidate?.work_key",
            "if (file.description) book = { ...book, description: file.description }",
        ])
        self.guard("apps/local-voice/src-tauri/src/lib.rs", ["library::library_epub_description,"])
        self.guard("apps/local-voice/src-tauri/src/ebook.rs", [
            "pub(crate) fn opf_description(opf: &str)",
            'let plain = strip_description_tags(&decoded);',
        ])

    def test_review_fixes(self):
        self.guard("apps/apple/VoiceCore/Sources/VoiceCore/Library.swift", [
            "ProcessInfo.processInfo.systemUptime", ".suffix(256)", "120 * 1024",
        ])
        self.guard("apps/apple/VoiceCore/Sources/VoiceCore/LiveTranslation.swift", [
            "LibraryProgress.merge(mapped, saved)",
        ])
        self.guard("apps/local-voice/src/lib/library/useReadingSession.ts", [
            "beginReadingSession(bookId", '"visibilitychange"', '"pagehide"', "clearInterval(timer)",
        ])
        self.guard("apps/local-voice/src/components/library/AudioBar.tsx", ["useReadingSession("])
        self.guard("apps/local-voice/src/components/settings/tts/TtsSettings.tsx", ["useReadingSession("])
        self.guard("apps/local-voice/src/components/settings/tts/TtsSettings.tsx", [
            "startReaderReading(", "!speaking && sectionActive && viewMode === \"read\"",
            'kurzbuch ? "kurzbuch" : "audio", readerBlocks, charOffset',
            'kurzbuch ? "kurzbuch" : "audio", readerBlocks, 0',
        ])
        self.guard("apps/local-voice/src/components/settings/tts/readerSession.ts", [
            'listen<{ phase: string }>("tts-state-changed"',
            "readingSource.locator.locate(e.payload.text)",
            "window.setInterval(checkpointReaderReading, 5000)",
        ])
        self.guard("apps/apple/docs/l1-reading-log.md", [
            "Altclients können `coordinate_history`", "Der Beschreibungsrückfall bleibt auf dem iPhone offen.",
        ])
        self.guard("apps/apple/VoiceCore/Sources/VoiceCore/BookText.swift", [
            "for c in raw.unicodeScalars", 'name == "script" || name == "style"',
            "let plain = stripDescriptionTags(raw)",
        ])
        self.guard("apps/local-voice/src-tauri/src/ebook.rs", [
            "for c in raw.chars()", 'name == "script" || name == "style"',
            "let plain = strip_description_tags(raw)",
        ])


if __name__ == "__main__":
    unittest.main()
