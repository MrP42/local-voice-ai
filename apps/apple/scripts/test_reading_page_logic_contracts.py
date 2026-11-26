"""Offline connection guards, not Swift behavior tests.

Behavior and persistence: ReadingPageEditingTests (Mac gate). Each connection
guard has a counterprobe that removes its relevant implementation in memory.
No project generation, installs, network, or application state is touched.
"""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    return source.split(start, 1)[1].split(end, 1)[0]


def route_guard(source):
    assert "static let routesToReadingPages = true" in source
    branch = section(source, "if Self.routesToReadingPages {", "\n        do {")
    assert "createPage(fromText: text, title: title, source: d.source)" in branch
    assert "pendingPageID = page.id" in branch
    assert "draft = nil" in branch
    assert "return" in branch
    assert "LibraryPlayback" not in branch
    dismiss = section(source, "func sheetDismissed()", "} else if let book")
    assert "openPageID = id" in dismiss
    assert "LibraryPlayback" not in dismiss


def player_guard(source):
    entry = section(source, "func open(page:", "/// Ein Einstieg für alles")
    assert "ReadingPagePlaybackSource(page: page)" in entry
    assert "openReadAloud(source.book, page: page)" in entry
    assert "presented = true" in entry
    assert all(call not in entry for call in ("importPlainText", "library.write", "library.put"))
    load = section(source, "func openReadAloud(", "private var phononVoice")
    assert "page?.text ?? library.text(book)" in load
    assert all(call not in load for call in ("importPlainText", "library.write(", "library.storeRemote(", "library.touch("))
    alignment = section(load, "private func alignTextRecord(", "return result.record")
    assert "let key = library.textKey(book)" in alignment
    assert "library.put(key, result.record)" in alignment
    save = section(source, "private func saveText(", "static func defaultVoice")
    assert "library.put(library.textKey(book), rec)" in save
    assert "guard pageID == nil else { return }" in section(source, "func switchSource(", "// MARK: Sprachfassungen")


def progress_guard(source):
    record = section(source, "func record(", "// MARK: Stimme")
    assert "return ReadingPageLocalProgress.record(key)" in record
    put = section(source, "func put(", "func setFinished(")
    local = section(put, "if key.hasPrefix(ReadingPagePlaybackSource.progressPrefix)", "guard let data")
    assert "ReadingPageLocalProgress.put(key, rec)" in local
    assert "return" in local
    key = section(source, "func textKey(", "func audioKey(")
    assert "ReadingPagePlaybackSource.progressPrefix +" in key


def pull_guard(store, sync):
    apply = section(store, "public func apply(id:", "private static func hash")
    assert "try willApplyRemote?()" in apply
    assert apply.index("try willApplyRemote?()") < apply.index("var record =")
    assert "await" not in apply
    factory = section(sync, "func readingPageStore()", "private func readingPageStore(")
    assert "store.willApplyRemote = { try ReadingPagesModel.shared.prepareForRemote() }" in factory
    apply = section(sync, "private func apply(", "let collections =")
    # Hänger 26.11.: kein Laden/Vergleichen aller Seiten je Objekt, sondern vormerken und einmal je Lauf neu laden.
    assert "store.pages()" not in apply
    assert "readingPagesTouched = true" in apply
    assert "await" not in apply
    run = section(sync, "func run() async {", "private func pull(")
    assert "if readingPagesTouched { readingPagesTouched = false; ReadingPagesModel.shared.reload() }" in run


def lifecycle_guard(app, model):
    scene = section(app, ".onChange(of: phase, initial: true)", "private struct VoiceHome")
    assert "if value != .active { ReadingPagesModel.shared.flushPending() }" in scene
    assert "func flushPending(_ pageID: String? = nil)" in model
    assert "private let pendingText = PendingTextBuffer()" in model
    assert "flushIfCurrent(pageID, revision: revision" in model
    flush = section(model, "private func flushText(", "private func perform(")
    assert "try pendingText.flush(id)" in flush
    assert "try editor().updateText" in flush
    assert "await" not in flush


def buffer_guard(source):
    buffer = section(source, "public final class PendingTextBuffer", "/// Plattformunabhängige Bearbeitung")
    flush = section(buffer, "public func flush(", "@discardableResult")
    assert "try save(id, entry.text)" in flush
    assert flush.index("try save(id, entry.text)") < flush.index("entries[id] = nil")
    assert "if entries[id]?.revision == entry.revision" in flush
    assert "guard entries[id]?.revision == revision else { return false }" in buffer
    assert "await" not in buffer
    update = section(source, "public func updateText(", "public func insert(")
    assert "guard var page = try page(id) else { throw ReadingPageError.invalid }" in update


class ReadingPageConnectionTests(unittest.TestCase):
    def test_buffer_only_removes_saved_text_and_rejects_stale_callbacks(self):
        source = read("VoiceCore/Sources/VoiceCore/ReadingPageEditing.swift")
        buffer_guard(source)
        for old, new in [
            ("try save(id, entry.text)", ""),
            ("if entries[id]?.revision == entry.revision", "if true"),
            ("guard entries[id]?.revision == revision else { return false }", ""),
            ("guard var page = try page(id) else { throw ReadingPageError.invalid }",
             "guard var page = try page(id) else { return }"),
        ]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                buffer_guard(source.replace(old, new))

    def test_pull_flushes_before_conflict_check_and_reloads_changed_pages(self):
        store = read("VoiceCore/Sources/VoiceCore/ReadingPageStore.swift")
        sync = read("iPhone/Library/LibrarySync.swift")
        pull_guard(store, sync)
        for old in ["try willApplyRemote?()"]:
            with self.assertRaises(AssertionError):
                pull_guard(store.replace(old, ""), sync)
        for old in ["store.willApplyRemote = { try ReadingPagesModel.shared.prepareForRemote() }",
                    "readingPagesTouched = true", "ReadingPagesModel.shared.reload()"]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                pull_guard(store, sync.replace(old, ""))
        with self.assertRaises(AssertionError):
            pull_guard(store.replace("try willApplyRemote?()", "try willApplyRemote?(); await Task.yield()"), sync)

    def test_scene_and_editor_flush_use_the_testable_buffer(self):
        app = read("Shared/VoiceApp.swift")
        model = read("iPhone/Library/ReadingPagesModel.swift")
        lifecycle_guard(app, model)
        with self.assertRaises(AssertionError):
            lifecycle_guard(app.replace("if value != .active { ReadingPagesModel.shared.flushPending() }", ""), model)
        for old in ["func flushPending(_ pageID: String? = nil)", "try pendingText.flush(id)",
                    "flushIfCurrent(pageID, revision: revision"]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                lifecycle_guard(app, model.replace(old, ""))

    def test_intake_route_is_on_and_opens_only_after_dismissal(self):
        source = read("iPhone/Library/TextIntakeViews.swift")
        route_guard(source)
        for old, new in [("routesToReadingPages = true", "routesToReadingPages = false"),
                         ("pendingPageID = page.id", ""), ("openPageID = id", "")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                route_guard(source.replace(old, new))

    def test_player_uses_page_text_without_importing_a_book(self):
        source = read("iPhone/Library/LibraryPlayback.swift")
        player_guard(source)
        for old, new in [("page?.text ?? library.text(book)", "library.text(book)"),
                         ("openReadAloud(source.book, page: page)", "library.write(source.book)"),
                         ("let key = library.textKey(book)", 'let key = "book-progress"'),
                         ("library.put(library.textKey(book), rec)", 'library.put("book-progress", rec)'),
                         ("pageID = page?.id; pageText = page?.text", "library.write(book)"),
                         ("guard pageID == nil else { return }", "")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                player_guard(source.replace(old, new))

    def test_page_progress_bypasses_synced_book_progress(self):
        source = read("iPhone/Library/PhoneLibrary.swift")
        progress_guard(source)
        for old in ["return ReadingPageLocalProgress.record(key)",
                    "ReadingPageLocalProgress.put(key, rec)",
                    "ReadingPagePlaybackSource.progressPrefix +"]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                progress_guard(source.replace(old, ""))

    def test_ocr_has_ticket_and_live_page_guards_after_await(self):
        source = read("iPhone/Library/ReadingPagesModel.swift")
        for operation, end in [("func append(_ sources:", "func append(text:"),
                               ("func rerecognize(", "func resolve(")]:
            block = section(source, operation, end)
            after_await = block.split("try await", 1)[1]
            guard = "recognitionGuard.isValid(ticket), try e.page(pageID) != nil"
            self.assertIn(guard, after_await)
            with self.assertRaises(AssertionError):
                self.assertIn(guard, after_await.replace(guard, ""))
        delete = section(source, "func deletePage(", "func readAloud(")
        self.assertIn("recognitionGuard.switchTo(nil)", delete)
        with self.assertRaises(AssertionError):
            self.assertIn("recognitionGuard.switchTo(nil)", delete.replace("recognitionGuard.switchTo(nil)", ""))


if __name__ == "__main__":
    unittest.main()
