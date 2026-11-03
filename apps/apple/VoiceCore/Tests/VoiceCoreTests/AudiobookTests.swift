import XCTest
@testable import VoiceCore

final class AudiobookTests: XCTestCase {
    private func ch(_ t: String, _ s: Double, _ d: Double = 0) -> AudiobookChapter { AudiobookChapter(title: t, startMs: s, durationMs: d) }

    func testMergeUsesFileChaptersAndOffsetsLaterParts() {
        let tracks = [LibraryAudioTrack(file: "teil_001.m4b", title: "Buch 1", durationMs: 60_000),
                      LibraryAudioTrack(file: "teil_002.m4b", title: "Buch 2", durationMs: 40_000)]
        let files = [AudiobookFileInfo(chapters: [ch("Vorspann", 0, 5_000), ch("Eins", 5_000, 55_000)]),
                     AudiobookFileInfo(chapters: [ch("Zwei", 0, 20_000), ch("Drei", 20_000, 20_000)])]
        let merged = AudiobookChapters.merge(files: files, tracks: tracks)
        XCTAssertEqual(merged.map(\.title), ["Vorspann", "Eins", "Zwei", "Drei"])
        XCTAssertEqual(merged.map(\.startMs), [0, 5_000, 60_000, 80_000])
        XCTAssertEqual(merged.last?.endMs, 100_000)
    }

    func testPartWithoutChaptersBecomesOneChapterWithCleanTitle() {
        let tracks = [LibraryAudioTrack(file: "a.mp3", title: "01 - Der Anfang.mp3", durationMs: 30_000),
                      LibraryAudioTrack(file: "b.mp3", title: "02_-_Das_Ende", durationMs: 10_000)]
        let merged = AudiobookChapters.merge(files: [], tracks: tracks)
        XCTAssertEqual(merged.map(\.title), ["Der Anfang", "Das Ende"])
        XCTAssertEqual(merged.map(\.durationMs), [30_000, 10_000])
    }

    func testFirstChapterStartsAtPartStartAndDurationsFollowStarts() {
        let tracks = [LibraryAudioTrack(file: "a.m4b", title: "A", durationMs: 100_000)]
        // Kapitelspur beginnt erst bei 2 s und hat falsche Längen.
        let files = [AudiobookFileInfo(chapters: [ch("Eins", 2_000, 999), ch("Zwei", 50_000, 1)])]
        let merged = AudiobookChapters.merge(files: files, tracks: tracks)
        XCTAssertEqual(merged.map(\.startMs), [0, 50_000])
        XCTAssertEqual(merged.map(\.durationMs), [50_000, 50_000])
    }

    func testNormalizedDropsDuplicateStarts() {
        let out = AudiobookChapters.normalized([ch("A", 0), ch("B", 0), ch("C", 10)], totalMs: 20)
        XCTAssertEqual(out.map(\.title), ["B", "C"])
    }

    func testIndexFractionAndTargets() {
        let chapters = AudiobookChapters.normalized([ch("A", 0), ch("B", 10_000), ch("C", 20_000)], totalMs: 30_000)
        XCTAssertEqual(AudiobookChapters.index(at: 0, in: chapters), 0)
        XCTAssertEqual(AudiobookChapters.index(at: 15_000, in: chapters), 1)
        XCTAssertEqual(AudiobookChapters.index(at: 29_999, in: chapters), 2)
        XCTAssertEqual(AudiobookChapters.fraction(at: 15_000, in: chapters[1]), 0.5, accuracy: 0.0001)
        // Mitten im Kapitel: zurück an dessen Anfang; kurz nach Beginn: ins vorige.
        XCTAssertEqual(AudiobookChapters.previousTarget(at: 15_000, in: chapters), 10_000)
        XCTAssertEqual(AudiobookChapters.previousTarget(at: 11_000, in: chapters), 0)
        XCTAssertEqual(AudiobookChapters.nextTarget(at: 15_000, in: chapters), 20_000)
        XCTAssertNil(AudiobookChapters.nextTarget(at: 25_000, in: chapters))
    }

    func testPages() {
        XCTAssertEqual(AudiobookChapters.estimatedPage(at: 0, totalMs: 1000, pages: 300), 1)
        XCTAssertEqual(AudiobookChapters.estimatedPage(at: 500, totalMs: 1000, pages: 300), 151)
        XCTAssertEqual(AudiobookChapters.estimatedPage(at: 1000, totalMs: 1000, pages: 300), 300)
        XCTAssertEqual(AudiobookChapters.pageCount(known: 320, textUTF16: 9_000, totalMs: 0)?.pages, 320)
        XCTAssertEqual(AudiobookChapters.pageCount(known: 320, textUTF16: nil, totalMs: 0)?.estimated, false)
        XCTAssertEqual(AudiobookChapters.pageCount(known: nil, textUTF16: 18_000, totalMs: 0)?.pages, 10)
        // 9:52:42 ≈ 349 Seiten
        XCTAssertEqual(AudiobookChapters.pageCount(known: nil, textUTF16: nil, totalMs: 35_562_000)?.pages, 349)
        XCTAssertNil(AudiobookChapters.pageCount(known: nil, textUTF16: nil, totalMs: 0))
    }

    func testCombineAndPeople() {
        var a = AudiobookInfo(); a.title = "Alles Idioten!"; a.authors = ["Hans Muster"]; a.fileBytes = 10; a.hasCover = true
        var b = AudiobookInfo(); b.title = "Teil 2"; b.authors = ["hans muster", "Eva Beispiel"]; b.publisher = "Hörverlag"; b.fileBytes = 5
        let c = AudiobookInfo.combine([a, b])
        XCTAssertEqual(c.title, "Alles Idioten!")
        XCTAssertEqual(c.authors, ["Hans Muster", "Eva Beispiel"])
        XCTAssertEqual(c.publisher, "Hörverlag")
        XCTAssertEqual(c.fileBytes, 15)
        XCTAssertTrue(c.hasCover)
        XCTAssertEqual(AudiobookInfo.people("A & B; C und D, A"), ["A", "B", "C", "D"])
        XCTAssertEqual(AudiobookInfo.displayDate("2019-03-14T00:00:00Z"), "14.03.2019")
        XCTAssertEqual(AudiobookInfo.displayDate("2019"), "2019")
    }

    func testInfoRoundTripsAsJSON() throws {
        var info = AudiobookInfo(); info.title = "X"; info.chapters = [ch("A", 0, 10)]; info.chaptersFromFile = true
        let back = try JSONDecoder().decode(AudiobookInfo.self, from: JSONEncoder().encode(info))
        XCTAssertEqual(back, info)
    }
}
