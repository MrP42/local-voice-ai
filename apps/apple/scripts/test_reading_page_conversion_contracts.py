"""Offline wiring checks; Swift/Rust behavior tests run at their platform gates."""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def conversion_guard(source):
    body = source.split("func convert(book:", 1)[1].split("func createPage(from sources:", 1)[0]
    assert "e.convert(book: book" in body
    assert "library.record(library.textKey(book))" in body
    assert "LibrarySync.shared.deleteEverywhere(book, requireTombstone: true)" in body
    assert "library.delete(book)" not in body
    assert "catch { report(error); return nil }" in body


def core_guard(source):
    body = source.split("public func convert(book:", 1)[1].split("public func updateText(", 1)[0]
    assert "SHA256.hash(data: Data(book.id.utf8))" in body
    assert "store.archivedPage(id)" in body
    assert "store.page(id)" in body
    assert "created.text = text" in body
    assert "progress?.finished = true" in body
    assert body.index("try store.save(created)") < body.index("ReadingPageLocalProgress.put") < body.index("try afterSave(page)")


def preservation_guard(source):
    body = source.split("pub fn page_state_save(", 1)[1].split("pub fn page_dir(", 1)[0]
    assert "merge_state" in body
    assert "std::fs::read_to_string" in body


def display_guard(source):
    assert "page.source" in source
    assert "{page.source}" in source


def tombstone_guard(source):
    body = source.split("func deleteEverywhere(", 1)[1].split("/// Kennt das Konto", 1)[0]
    assert "if requireTombstone && (account == nil || running) { return }" in body
    assert "confirmed.deleted, confirmed.revision > base else { return }" in body
    assert body.index("try await push") < body.index("confirmed.deleted") < body.index("library.delete(book)")


class ConversionConnections(unittest.TestCase):
    def test_wiring_and_counterprobes(self):
        cases = [
            (conversion_guard, "apple/iPhone/Library/ReadingPagesModel.swift", "LibrarySync.shared.deleteEverywhere(book, requireTombstone: true)"),
            (preservation_guard, "local-voice/src-tauri/src/commands/pages.rs", "merge_state"),
            (display_guard, "local-voice/src/components/settings/tts/WorkspaceSidebars.tsx", "{page.source}"),
            (tombstone_guard, "apple/iPhone/Library/LibrarySync.swift", "confirmed.deleted, confirmed.revision > base else { return }"),
            (core_guard, "apple/VoiceCore/Sources/VoiceCore/ReadingPageEditing.swift", "try store.save(created)"),
        ]
        for guard, path, needle in cases:
            with self.subTest(path=path):
                source = read(path)
                guard(source)
                with self.assertRaises((AssertionError, IndexError, ValueError)):
                    guard(source.replace(needle, "REMOVED"))


if __name__ == "__main__":
    unittest.main()
