"""Offline guards for the iOS responsiveness fixes (Time Profiler 26.11.), not Swift behavior tests.

Behavior: ReadingPageStorePerformanceTests (Mac gate). Each guard has a counterprobe that
removes the relevant implementation in memory. Nothing is built, installed or touched.
"""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    return source.split(start, 1)[1].split(end, 1)[0]


def library_guard(source):
    # Regal: Fassungen und Fortschritt nicht je Zeichnen neu dekodieren.
    audio = section(source, "func audioKey(", "func allProgressKeys(")
    assert "self.editions(book)" in audio and "BookEditions(book:" not in audio
    active = section(source, "func activeView(", "// MARK: Abgeleitete Werte")
    assert "activeViews[key], hit.book == book" in active
    assert "self.editions(book)" in active and "BookEditions(book:" not in active
    cache = section(source, "func editions(_ book: LibraryBook) -> BookEditions {", "private var editionsByBook")
    assert "editionsByBook[key], hit.book == book" in cache
    record = section(source, "func record(_ key: String) -> ProgressRecord? {", "// MARK: Stimme")
    assert "if let hit = decodedRecords[raw] { return hit }" in record
    reload = section(source, "func reload() {", "var progressURL")
    assert "invalidateDerived()" in reload


def sync_guard(sync, views):
    push = section(sync, "private func pushBooks(", "private nonisolated static func encodeBook(")
    assert push.count("await Task.yield()") >= 2
    assert "try await Self.encodeBook(current, meta)" in push
    assert "LibrarySyncFormat.canonicalHash(value)" not in push
    encode = section(sync, "private nonisolated static func encodeBook(", "private func localHash(")
    assert "Task.detached" in encode
    deletions = section(sync, "private func bookDeletions(", "private func saveBookDeletions(")
    assert "if let hit = bookDeletionsCache, hit.path == url.path { return hit.value }" in deletions
    save = section(sync, "private func saveBookDeletions(", "func queueBookDeletion(")
    assert "bookDeletionsCache = (url.path, pending)" in save
    assert ".task { await sync.runIfStale();" in views
    stale = section(sync, "func runIfStale(", "func run() async {")
    assert "Date().timeIntervalSince(last) < interval { return }" in stale


def store_guard(source):
    checked = section(source, "private func readChecked(", "private func write(")
    assert "if let hit = cache[id], hit.stamp == stamp { return (hit.record, hit.page) }" in checked
    write = section(source, "private func write(", "private static func stamp(")
    assert "cache[id] = nil" in write
    assert write.index("cache[id] = nil") < write.index("try writeData(")
    stamp = section(source, "private static func stamp(", "private func writeData(")
    assert ".modificationDate" in stamp and ".size" in stamp and ".systemFileNumber" in stamp
    loaded = section(source, "private func loaded()", "private static func safeImageName(")
    assert "for id in cache.keys where !present.contains(id) { cache[id] = nil }" in loaded


class IOSPerformanceTests(unittest.TestCase):
    def test_shelf_reuses_decoded_editions_and_progress(self):
        source = read("iPhone/Library/PhoneLibrary.swift")
        library_guard(source)
        for old, new in [("if let hit = decodedRecords[raw] { return hit }", ""),
                         ("activeViews[key], hit.book == book", "activeViews[key], false"),
                         ("        let editions = self.editions(book)\n        return (editions.edition(book.editionId)",
                          "        let editions = BookEditions(book: book)\n        return (editions.edition(book.editionId)"),
                         ("editionsByBook[key], hit.book == book", "editionsByBook[key], true"),
                         ("        invalidateDerived()\n        let fm", "        let fm")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                library_guard(source.replace(old, new))

    def test_sync_yields_and_hashes_off_main(self):
        sync = read("iPhone/Library/LibrarySync.swift")
        views = read("iPhone/Library/LibraryViews.swift")
        sync_guard(sync, views)
        for old, new in [("await Task.yield()", ""),
                         ("try await Self.encodeBook(current, meta)", "try Self.encodeBookNow(current, meta)"),
                         ("if let hit = bookDeletionsCache, hit.path == url.path { return hit.value }", ""),
                         ("bookDeletionsCache = (url.path, pending)", "")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                sync_guard(sync.replace(old, new), views)
        with self.assertRaises(AssertionError):
            sync_guard(sync, views.replace(".task { await sync.runIfStale();", ".task { await sync.run();"))

    def test_store_caches_by_file_stamp_and_forgets_on_write(self):
        source = read("VoiceCore/Sources/VoiceCore/ReadingPageStore.swift")
        store_guard(source)
        for old, new in [("if let hit = cache[id], hit.stamp == stamp", "if let hit = cache[id]"),
                         ("        cache[id] = nil\n        do {\n            try writeData(", "        do {\n            try writeData("),
                         (".systemFileNumber", ".creationDate"),
                         ("for id in cache.keys where !present.contains(id) { cache[id] = nil }", "")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                store_guard(source.replace(old, new))


if __name__ == "__main__":
    unittest.main()
