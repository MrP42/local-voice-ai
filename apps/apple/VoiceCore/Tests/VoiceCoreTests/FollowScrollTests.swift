import XCTest
import Foundation
@testable import VoiceCore

final class FollowScrollTests: XCTestCase {
    func testShortParagraphCentres() {
        XCTAssertNil(FollowScroll.anchor(fraction: 0.7, length: 200))
    }

    func testLongParagraphFollowsInSteps() {
        let n = 2000
        XCTAssertEqual(FollowScroll.anchor(fraction: 0, length: n), 0)
        XCTAssertEqual(FollowScroll.anchor(fraction: 0.1, length: n), 0)
        XCTAssertEqual(FollowScroll.anchor(fraction: 0.5, length: n), 0.5)
        XCTAssertEqual(FollowScroll.anchor(fraction: 0.99, length: n), 0.875)
        XCTAssertEqual(FollowScroll.anchor(fraction: 1, length: n), 1)
        XCTAssertEqual(FollowScroll.anchor(fraction: 3, length: n), 1)
        XCTAssertEqual(FollowScroll.anchor(fraction: -1, length: n), 0)
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
        let m = ReadingMark(paragraph: 0, sentence: nil, word: nil, precise: false)
        XCTAssertEqual(m.fraction(inParagraphOf: 400), 0)
        let w = ReadingMark(paragraph: 0, sentence: nil, word: NSRange(location: 5, length: 1), precise: true)
        XCTAssertEqual(w.fraction(inParagraphOf: 0), 0)
    }
}
