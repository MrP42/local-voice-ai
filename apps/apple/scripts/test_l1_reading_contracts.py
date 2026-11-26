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
            "entry.endMs - entry.startMs >= 30_000",
            "entry.to = max(entry.to, to)",
            "devices[entry.device]?[entry.startMs]",
            "merged.endMs = max(old.endMs, entry.endMs); merged.to = max(old.to, entry.to)",
            "case readingLog = \"reading_log\"",
            "try raw.decode(PassthroughJSON.self, forKey: key)",
            "let known = [reach(older), reach(newer)]",
            "out.extra[\"coordinate_history\"] = .object(history)",
        ])

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
            'XHTMLText.replaceEntities(plain, keepXML: false).split(whereSeparator: \\.isWhitespace)',
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
            'let plain = TAG_RE.replace_all(&decoded, " ");',
        ])


if __name__ == "__main__":
    unittest.main()
