import XCTest
import Foundation
@testable import VoiceCore

final class FollowScrollTests: XCTestCase {
    /// Stelle der gelesenen Zeile in der Fläche nach `scrollTo` mit Anker a: y − a·(h − v).
    private func lineInViewport(fraction f: Double, h: Double, v: Double) -> Double? {
        guard let a = FollowScroll.anchor(fraction: f, paragraphHeight: h, viewportHeight: v) else { return nil }
        return f * h - a * (h - v)
    }

    func testFittingOrUnmeasuredParagraphCentres() {
        XCTAssertNil(FollowScroll.anchor(fraction: 0.7, paragraphHeight: 300, viewportHeight: 400))
        XCTAssertNil(FollowScroll.anchor(fraction: 0.7, paragraphHeight: 400, viewportHeight: 400))
        XCTAssertNil(FollowScroll.anchor(fraction: 0.7, paragraphHeight: 900, viewportHeight: 0))
    }

    /// Codex-Fall: 8000 pt Absatz, 400 pt Fläche. Die Zeile bleibt an jeder Stelle sichtbar.
    func testLineStaysVisibleInVeryTallParagraph() {
        for i in 0...100 {
            let f = Double(i) / 100
            let y = lineInViewport(fraction: f, h: 8000, v: 400)!
            XCTAssertGreaterThanOrEqual(y, 0, "f=\(f)")
            XCTAssertLessThanOrEqual(y, 400, "f=\(f)")
        }
        // Mitten im Absatz steht sie bei gut einem Drittel (plus höchstens eine Stufe).
        let mid = lineInViewport(fraction: 0.5, h: 8000, v: 400)!
        XCTAssertGreaterThanOrEqual(mid, 0.35 * 400 - 0.001)
        XCTAssertLessThanOrEqual(mid, 0.35 * 400 + 0.25 * 400)
    }

    func testSlightlyTallParagraphKeepsLineVisible() {
        for i in 0...20 {
            let f = Double(i) / 20
            let y = lineInViewport(fraction: f, h: 520, v: 400)!
            XCTAssertGreaterThanOrEqual(y, 0); XCTAssertLessThanOrEqual(y, 400)
        }
    }

    func testAnchorMovesOnlyInSteps() {
        let a = FollowScroll.anchor(fraction: 0.100, paragraphHeight: 8000, viewportHeight: 400)
        let b = FollowScroll.anchor(fraction: 0.101, paragraphHeight: 8000, viewportHeight: 400)
        XCTAssertEqual(a, b)
        XCTAssertEqual(FollowScroll.anchor(fraction: 3, paragraphHeight: 8000, viewportHeight: 400), 1)
        XCTAssertEqual(FollowScroll.anchor(fraction: -1, paragraphHeight: 8000, viewportHeight: 400), 0)
    }

    func testMarkFractionPrefersWord() {
        let m = ReadingMark(paragraph: 0, sentence: NSRange(location: 100, length: 100), word: NSRange(location: 120, length: 5), precise: true)
        XCTAssertEqual(m.fraction(inParagraphOf: 400), 0.3, accuracy: 1e-9)
    }

    func testMarkFractionUsesSentenceMiddle() {
        let m = ReadingMark(paragraph: 0, sentence: NSRange(location: 100, length: 100), word: nil, precise: false)
        XCTAssertEqual(m.fraction(inParagraphOf: 400), 0.375, accuracy: 1e-9)
    }

    func testMarkFractionWithoutRangesOrLength() {
        XCTAssertEqual(ReadingMark(paragraph: 0, sentence: nil, word: nil, precise: false).fraction(inParagraphOf: 400), 0)
        XCTAssertEqual(ReadingMark(paragraph: 0, sentence: nil, word: NSRange(location: 5, length: 1), precise: true).fraction(inParagraphOf: 0), 0)
    }

    func testTextLengthMatchesText() {
        for p in LibraryText.paragraphs("# Kapitel eins\n\nErster Absatz mit Umlauten äöü und Emoji 🙂.\n\nZweiter.") {
            XCTAssertEqual(p.textLength, p.text.utf16.count, p.text)
        }
    }
}
