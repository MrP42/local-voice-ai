"""Offline guards for library scroll jank (Time Profiler 27.11.: 2 hangs of 273/598 ms while scrolling
during playback), not Swift behavior tests.

Each guard has a counterprobe that removes the relevant implementation in memory. Nothing is built,
installed or touched. Behavior and timing: Mac gate plus a fresh Time Profiler run on the device.
"""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def section(source, start, end):
    assert start in source, start
    return source.split(start, 1)[1].split(end, 1)[0]


def shelf_item_guard(views):
    item = section(views, "struct ShelfItem: View {", "/// Grüner Kreis")
    assert "library.shelfEntry(book)" in item and "library.shelfFiles(entry.view)" in item
    for slow in ["library.hasAudio(", "library.audioInAccount(", "library.activeView(", "library.coverThumbnail(",
                 "LanguageBadges(book: book, compact: true)\n", "LanguageBadges.spoken(book)"]:
        assert slow not in item, slow
    assert item.count("editions: entry.editions") >= 3
    button = section(views, "struct ShelfPlayButton: View {", "private struct ShelfPlayControl")
    assert "library.shelfFiles(book)" in button and "library.hasAudio(" not in button
    shelf = section(views, "private func textFraction(", "var body: some View {")
    assert "library.activeView(" not in shelf and shelf.count("library.shelfEntry(") >= 4
    cover = section(views, "struct ShelfCover: View {", "/// Erzeugtes Cover aus Titel")
    assert "await library.loadCoverThumbnail(book)" in cover and ".task(id: key)" in cover
    assert "coverThumbnail(book)" not in cover


def library_guard(source):
    books = section(source, "@Published private(set) var books: [LibraryBook] = [] {", "@Published private(set) var anyAudioDownloaded")
    assert "shelfEntries.removeAll()" in books and "shelfAudio.removeAll()" in books
    meta = section(source, "func meta(_ book: LibraryBook) -> SyncMeta {", "/// Dateiverweis nach einem Upload")
    assert "if let hit = metaCache[book.id]" in meta
    set_meta = section(source, "func setMeta(_ book: LibraryBook, _ meta: SyncMeta) {", "/// Eigene Änderung am Buch")
    assert "metaCache[book.id] = nil" in set_meta
    assert set_meta.index("metaCache[book.id] = nil") < set_meta.index(".write(to:")
    assert "invalidateAudioState()" in set_meta
    derived = section(source, "private func invalidateDerived() {", "// MARK: Regal")
    for name in ["metaCache", "shelfEntries", "shelfAudio"]:
        assert f"{name}.removeAll()" in derived, name
    files = section(source, "func shelfFiles(_ view: LibraryBook) -> ShelfFiles {", "func invalidateAudioState()")
    assert "now.timeIntervalSince(hit.at) < Self.shelfAudioLifetime" in files
    assert source.count("metaCache[book.id] = nil\n") >= 3  # setMeta + beide Ordner-Löschungen
    assert "forgetLocalAudio(bookId: book.id, files: Array(names))\n        invalidateAudioState()" in source


def cover_guard(metadata):
    load = section(metadata, "func loadCoverThumbnail(_ book: LibraryBook) async -> UIImage? {", "/// Erste Kacheln vorladen")
    assert "Task.detached" in load and "Self.decodeThumbnail(" in load
    assert "PhoneLibraryCoverCache.revision == revision" in load
    thumb = section(metadata, "func coverThumbnail(_ book: LibraryBook) -> UIImage? {", "/// Schon geladenes")
    assert "coverImage(book)" not in thumb  # kein volles Cover im Speicher für Vorschaubilder
    decode = section(metadata, "nonisolated static func decodeThumbnail(", "/// Cover des Buchs")
    assert "kCGImageSourceThumbnailMaxPixelSize" in decode and "kCGImageSourceShouldCache: false" in decode
    assert "loading[id] = nil; revision += 1" in metadata


def downloads_guard(downloads):
    jobs = section(downloads, "@Published private(set) var jobs:", "@Published private(set) var done")
    assert "library.invalidateAudioState()" in jobs


def player_guard(playback, phonon):
    speakers = section(playback, "private var scriptSpeakers: [SpeakerScript.Speaker] {", "private var scriptDocument")
    assert "if let hit = speakerCache, hit.key == key { return hit.speakers }" in speakers
    assert "voiceCatalogRevision" in speakers and "PhononEngine.shared.voicesRevision" in speakers
    assert speakers.index("speakerCache, hit.key == key") < speakers.index("PhononEngine.shared.scriptSpeakers(")
    assert "didSet { voiceCatalogRevision += 1 }" in playback
    assert "didSet { voicesRevision += 1 }" in phonon
    live = section(playback, "func liveParagraph(_ index: Int) -> LiveParagraph? {", "private func clearDisplayCaches()")
    assert "hit.revision == liveRevision" in live
    display = section(playback, "func scriptDisplay(inParagraph index: Int) -> SpeakerScriptDisplay? {", "private func scriptSentence(")
    assert "if let hit = scriptDisplayCache[index] { return hit }" in display
    assert "if let hit = liveDisplayCache[index], hit.value == value { return hit.display }" in display
    document = section(playback, "private var scriptDocument: SpeakerScript {", "/// API für Paket 2")
    assert "clearDisplayCaches()" in document
    assert "didSet { if sentenceCache.isEmpty { clearDisplayCaches() } }" in playback
    # Jede Änderung am Spiegel erhöht die Revision, sonst zeigt der Zwischenspeicher alte Übersetzungen.
    store = section(playback, "private func storeLive(", "/// Unterlauf oder Fehler aufheben")
    assert "liveRevision += 1" in store


def badges_guard(sheet):
    badges = section(sheet, "struct LanguageBadges: View {", "/// Hinweis nach dem Sprachwechsel")
    body = section(badges, "var body: some View {", "/// Für VoiceOver")
    assert "self.editions ?? PhoneLibrary.shared.editions(book)" in body and "BookEditions(book:" not in body


class IOSScrollTests(unittest.TestCase):
    def test_shelf_item_reads_only_caches(self):
        views = read("iPhone/Library/LibraryViews.swift")
        shelf_item_guard(views)
        for old, new in [("let files = library.shelfFiles(entry.view)", "let files = library.shelfFiles(entry.view); _ = library.hasAudio(book)"),
                         ("ShelfCover(book: book, revision: library.coverRevision, titleTopInset: 24)",
                          "LibraryCover(title: book.title, image: library.coverThumbnail(book), titleTopInset: 24)"),
                         ("LanguageBadges(book: book, compact: true, editions: entry.editions).fixedSize()",
                          "LanguageBadges(book: book, compact: true).fixedSize()"),
                         ("let files = library.shelfFiles(book)\n", "let files = library.shelfFiles(book); _ = library.hasAudio(book)\n"),
                         ("let e = library.shelfEntry(b)\n", "let e = library.activeView(b)\n"),
                         ("let thumb = await library.loadCoverThumbnail(book)", "let thumb = library.coverThumbnail(book)")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                shelf_item_guard(views.replace(old, new))

    def test_library_caches_meta_and_file_state(self):
        source = read("iPhone/Library/PhoneLibrary.swift")
        library_guard(source)
        for old, new in [("didSet { shelfEntries.removeAll(); shelfAudio.removeAll() }", "didSet { }"),
                         ("if let hit = metaCache[book.id] {", "if false, let hit = metaCache[book.id] {"),
                         ("        metaCache[book.id] = nil\n        invalidateAudioState()\n        guard",
                          "        invalidateAudioState()\n        guard"),
                         ("        metaCache.removeAll()\n", ""),
                         ("now.timeIntervalSince(hit.at) < Self.shelfAudioLifetime", "true"),
                         ("forgetLocalAudio(bookId: book.id, files: Array(names))\n        invalidateAudioState()",
                          "forgetLocalAudio(bookId: book.id, files: Array(names))")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                library_guard(source.replace(old, new))

    def test_cover_thumbnails_decode_off_main(self):
        metadata = read("iPhone/Library/AudiobookMetadata.swift")
        cover_guard(metadata)
        for old, new in [("task = Task.detached(priority: .userInitiated) {", "task = Task { @MainActor in"),
                         ("if PhoneLibraryCoverCache.revision == revision { PhoneLibraryCoverCache.thumbs[id]",
                          "if true { PhoneLibraryCoverCache.thumbs[id]"),
                         ("let thumb = coverURL(book).flatMap { Self.decodeThumbnail($0) }",
                          "let thumb = coverImage(book)"),
                         ("loading[id] = nil; revision += 1", "revision += 1")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                cover_guard(metadata.replace(old, new))

    def test_downloads_invalidate_shelf_state(self):
        downloads = read("iPhone/Library/AudioDownloads.swift")
        downloads_guard(downloads)
        with self.assertRaises(AssertionError):
            downloads_guard(downloads.replace("{ library.invalidateAudioState() }", "{ }"))

    def test_player_caches_speakers_and_paragraph_display(self):
        playback = read("iPhone/Library/LibraryPlayback.swift")
        phonon = read("iPhone/Library/PhononEngine.swift")
        player_guard(playback, phonon)
        for old, new in [("if let hit = speakerCache, hit.key == key { return hit.speakers }", ""),
                         ("didSet { voiceCatalogRevision += 1 }", ""),
                         ("hit.revision == liveRevision", "true"),
                         ("if let hit = scriptDisplayCache[index] { return hit }", ""),
                         ("if let hit = liveDisplayCache[index], hit.value == value { return hit.display }",
                          "if let hit = liveDisplayCache[index] { return hit.display }"),
                         ("didSet { if sentenceCache.isEmpty { clearDisplayCaches() } }", "didSet { }")]:
            with self.subTest(counterprobe=old), self.assertRaises(AssertionError):
                player_guard(playback.replace(old, new), phonon)
        with self.assertRaises(AssertionError):
            player_guard(playback, phonon.replace("didSet { voicesRevision += 1 }", ""))

    def test_language_badges_use_cached_editions(self):
        sheet = read("iPhone/Library/LanguageSourceSheet.swift")
        badges_guard(sheet)
        with self.assertRaises(AssertionError):
            badges_guard(sheet.replace("self.editions ?? PhoneLibrary.shared.editions(book)", "BookEditions(book: book)"))


if __name__ == "__main__":
    unittest.main()
