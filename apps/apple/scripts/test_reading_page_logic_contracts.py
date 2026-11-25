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
    assert "static let routesToReadingPages = false" in source
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


class ReadingPageConnectionTests(unittest.TestCase):
    def test_intake_route_is_prepared_but_off_and_opens_only_after_dismissal(self):
        source = read("iPhone/Library/TextIntakeViews.swift")
        route_guard(source)
        for old, new in [("routesToReadingPages = false", "routesToReadingPages = true"),
                         ("pendingPageID = page.id", ""), ("openPageID = id", "")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                route_guard(source.replace(old, new))

    def test_player_uses_page_text_without_importing_a_book(self):
        source = read("iPhone/Library/LibraryPlayback.swift")
        player_guard(source)
        for old, new in [("page?.text ?? library.text(book)", "library.text(book)"),
                         ("openReadAloud(source.book, page: page)", "library.write(source.book)"),
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


if __name__ == "__main__":
    unittest.main()
