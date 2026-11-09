import XCTest
@testable import VoiceCore

final class ReadAlongTests: XCTestCase {
    func testSurfacesDependOnText() {
        XCTAssertEqual(PlayerSurface.available(hasText: false), [.listen])
        XCTAssertEqual(PlayerSurface.available(hasText: true), [.listen, .readListen, .read])
        XCTAssertEqual(PlayerSurface.initial(stored: nil, hasText: true), .readListen)
        XCTAssertEqual(PlayerSurface.initial(stored: "read", hasText: true), .read)
        // Gemerktes „Lesen“ ohne Text fällt auf Hören zurück.
        XCTAssertEqual(PlayerSurface.initial(stored: "read", hasText: false), .listen)
        XCTAssertEqual(PlayerSurface.initial(stored: "unsinn", hasText: false), .listen)
        XCTAssertFalse(PlayerSurface.read.keepsPlaying)
        XCTAssertTrue(PlayerSurface.readListen.keepsPlaying)
    }

    func testSourcePrefersStoredThenLocalAudio() {
        XCTAssertEqual(PlayerSource.initial(stored: nil, audioLocal: true, hasText: true), .audiobook)
        XCTAssertEqual(PlayerSource.initial(stored: "voice", audioLocal: true, hasText: true), .voice)
        // Gemerktes Hörbuch, das nicht mehr auf dem Gerät liegt: Stimme liest.
        XCTAssertEqual(PlayerSource.initial(stored: "audiobook", audioLocal: false, hasText: true), .voice)
        XCTAssertNil(PlayerSource.initial(stored: nil, audioLocal: false, hasText: false))
    }

    func testWordRangeTrimsPunctuationAndSkipsSpace() {
        let text = "Er sagte: „Hallo, Welt!“ Dann ging er."
        let s = text as NSString
        func word(_ at: Int) -> String? { ReadAlong.wordRange(text, at: at).map { s.substring(with: $0) } }
        XCTAssertEqual(word(0), "Er")
        XCTAssertEqual(word(4), "sagte")
        XCTAssertEqual(word(s.range(of: "Hallo").location + 2), "Hallo")
        XCTAssertEqual(word(s.range(of: "Welt").location), "Welt")
        XCTAssertEqual(word(2), "sagte")  // Leerzeichen → nächstes Wort
        XCTAssertNil(ReadAlong.wordRange("", at: 0))
        XCTAssertNil(ReadAlong.wordRange("   ", at: 1))
    }

    func testMarkFollowsTimelineToSentenceAndWord() {
        let text = "# Kapitel 1\n\nErster Satz hier. Zweiter Satz dort.\n\nDritter Absatz."
        let paragraphs = LibraryText.paragraphs(text)
        let second = (text as NSString).range(of: "Zweiter").location
        // Zeitachse: Zeichen `second` wird bei 10 s gesprochen.
        let map = FollowMap(sync: [LibrarySyncPoint(chars: Double(second), ms: 10_000)],
                            textLength: Double(text.utf16.count), totalMs: 20_000)
        let mark = ReadAlong.mark(atMs: 10_000, map: map, paragraphs: paragraphs) { LibraryText.sentences(paragraphs[$0].text) }
        XCTAssertEqual(mark?.paragraph, 1)
        XCTAssertEqual(mark?.precise, true)
        let p = paragraphs[1].text as NSString
        XCTAssertEqual(mark?.sentence.map { p.substring(with: $0).trimmingCharacters(in: .whitespaces) }, "Zweiter Satz dort.")
        XCTAssertEqual(mark?.word.map { p.substring(with: $0) }, "Zweiter")
    }

    func testEstimatedTimelineMarksParagraphOnly() {
        let text = "Eins zwei.\n\nDrei vier."
        let paragraphs = LibraryText.paragraphs(text)
        let map = FollowMap(sync: [], textLength: Double(text.utf16.count), totalMs: 10_000)
        let mark = ReadAlong.mark(atMs: 9_000, map: map, paragraphs: paragraphs) { LibraryText.sentences(paragraphs[$0].text) }
        XCTAssertEqual(mark, ReadingMark(paragraph: 1, sentence: nil, word: nil, precise: false))
    }

    func testHandoverStartsAtSentenceBeginning() {
        let text = "Erster Satz hier. Zweiter Satz dort."
        let paragraphs = LibraryText.paragraphs(text)
        let second = (text as NSString).range(of: "Zweiter").location
        let map = FollowMap(sync: [LibrarySyncPoint(chars: Double(second + 8), ms: 5_000)],
                            textLength: Double(text.utf16.count), totalMs: 8_000)
        let chars = ReadAlong.handoverChars(atMs: 5_000, map: map, paragraphs: paragraphs) { LibraryText.sentences(paragraphs[$0].text) }
        XCTAssertEqual(chars, second)
    }
}
