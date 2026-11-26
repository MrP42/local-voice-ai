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
    assert "try LibrarySync.shared.queueBookDeletion(book)" in body
    assert "Task {" not in body
    assert "library.delete(book)" not in body
    assert "catch { report(error); return nil }" in body


def core_guard(source):
    body = source.split("public func convert(book:", 1)[1].split("public func updateText(", 1)[0]
    assert "SHA256.hash(data: Data(book.id.utf8))" in body
    assert "!store.isDeleted(id)" in body
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
    assert "private func deletePendingBooks(" in source
    body = source.split("private func deletePendingBooks(", 1)[1].split("/// Kennt das Konto", 1)[0]
    assert "confirmed.deleted, confirmed.revision > base else {" in body
    assert 'throw SyncError.http(0,' in body
    assert body.index("try await push") < body.index("confirmed.deleted") < body.index("library.delete(book)")
    assert body.index("library.delete(book)") < body.index("try saveBookDeletions(pending)")
    assert "guard !library.books.contains" in body
    assert "var pending = try bookDeletions()" in body
    assert "deletionError =" in body


def queue_guard(source):
    assert "func queueBookDeletion(" in source
    body = source.split("func queueBookDeletion(", 1)[1].split("private func deletePendingBooks(", 1)[0]
    assert body.index("try saveBookDeletions(pending)") < body.index("if running")
    assert "again = true" in body
    queued = body.split("func deleteEverywhere(", 1)[0]
    assert "Task {" not in queued
    assert "-> Bool" in body
    assert "return !(try bookDeletions()).contains(book.id)" in body
    assert 'appendingPathComponent("sync-book-deletions.json")' in source
    assert "JSONDecoder().decode(Set<String>.self" in source
    assert "options: .atomic" in source
    run = source.split("func run() async", 1)[1].split("private func pull(", 1)[0]
    assert "try await deletePendingBooks(acc)" in run
    assert "error = deletionError" in run
    assert "if again { again = false; Task" in run
    assert "if !sent.0.deleted { again = true }" in source
    books = source.split("private func pushBooks(", 1)[1].split("private func localHash(", 1)[0]
    assert books.count("if try bookDeletions().contains(book.id) { continue }") == 3


def deleted_guard(source):
    assert "func isDeleted(_ id: String) throws -> Bool { try read(id)?.deleted == true }" in source


def notification_guard(source):
    assert "LibrarySync.shared.$deletionError.sink" in source
    assert "if let message { self?.error = message }" in source


def visible_guard(source):
    assert "if await sync.deleteEverywhere(raw) { dismiss() }" in source
    assert "if let message = sync.deletionError { Text(message)" in source


class ConversionConnections(unittest.TestCase):
    def test_wiring_and_counterprobes(self):
        cases = [
            (conversion_guard, "apple/iPhone/Library/ReadingPagesModel.swift", "try LibrarySync.shared.queueBookDeletion(book)"),
            (preservation_guard, "local-voice/src-tauri/src/commands/pages.rs", "merge_state"),
            (display_guard, "local-voice/src/components/settings/tts/WorkspaceSidebars.tsx", "{page.source}"),
            (tombstone_guard, "apple/iPhone/Library/LibrarySync.swift", "confirmed.deleted, confirmed.revision > base else {"),
            (queue_guard, "apple/iPhone/Library/LibrarySync.swift", "try await deletePendingBooks(acc)"),
            (queue_guard, "apple/iPhone/Library/LibrarySync.swift", "if !sent.0.deleted { again = true }"),
            (queue_guard, "apple/iPhone/Library/LibrarySync.swift", "error = deletionError"),
            (visible_guard, "apple/iPhone/Library/LibraryViews.swift", "if await sync.deleteEverywhere(raw) { dismiss() }"),
            (core_guard, "apple/VoiceCore/Sources/VoiceCore/ReadingPageEditing.swift", "try store.save(created)"),
            (core_guard, "apple/VoiceCore/Sources/VoiceCore/ReadingPageEditing.swift", "!store.isDeleted(id)"),
            (deleted_guard, "apple/VoiceCore/Sources/VoiceCore/ReadingPageStore.swift", "read(id)?.deleted == true"),
            (notification_guard, "apple/iPhone/Library/ReadingPagesModel.swift", "LibrarySync.shared.$deletionError.sink"),
        ]
        for guard, path, needle in cases:
            with self.subTest(path=path):
                source = read(path)
                guard(source)
                with self.assertRaises((AssertionError, IndexError, ValueError)):
                    guard(source.replace(needle, "REMOVED"))


if __name__ == "__main__":
    unittest.main()
