import XCTest
import Foundation
@testable import VoiceCore

final class FollowScrollTests: XCTestCase {
    private func visibleLine(_ y: Double, h: Double, v: Double) -> Double? {
        guard let a = FollowScroll.anchor(lineY: y, paragraphHeight: h, viewportHeight: v) else { return nil }
        return FollowScroll.lineInViewport(lineY: y, anchor: a, paragraphHeight: h, viewportHeight: v)
    }

    func testFittingOrUnmeasuredParagraphCentres() {
        XCTAssertNil(FollowScroll.anchor(lineY: 100, paragraphHeight: 300, viewportHeight: 400))
        XCTAssertNil(FollowScroll.anchor(lineY: 100, paragraphHeight: 400, viewportHeight: 400))
        XCTAssertNil(FollowScroll.anchor(lineY: 100, paragraphHeight: 900, viewportHeight: 0))
        XCTAssertNil(FollowScroll.anchor(lineY: .nan, paragraphHeight: 900, viewportHeight: 400))
    }

    /// Jede gemessene Zeilenlage in einem sehr hohen Absatz bleibt nach dem Ausrichten in der Fläche.
    func testEveryMeasuredLineStaysVisible() {
        for y in stride(from: 0.0, through: 8000, by: 7) {
            let seen = visibleLine(y, h: 8000, v: 400)!
            XCTAssertGreaterThanOrEqual(seen, 0, "y=\(y)")
            XCTAssertLessThanOrEqual(seen, 400, "y=\(y)")
        }
    }

    /// Codex-Fall mit ungleichen Zeilen: 100 Kurzzeilen „a“ (je ~20 pt) und danach Fließtext. Das Wort nach 50
    /// Kurzzeilen liegt laut Layout bei ~1010 pt, obwohl es beim Zeichenanteil weit vorne wäre. Maßgeblich ist die
    /// gemessene Lage: Die Zeile steht danach in der Fläche, bei gut einem Drittel.
    func testUnevenLinesUseMeasuredPosition() {
        let h = 2000.0, v = 400.0
        let measured = 1010.0
        let seen = visibleLine(measured, h: h, v: v)!
        XCTAssertGreaterThanOrEqual(seen, 0.35 * v - 0.001)
        XCTAssertLessThanOrEqual(seen, 0.35 * v + 0.25 * v)
        // Eine Schätzung aus dem Zeichenanteil (~202 pt) läge nach diesem Anker außerhalb der Fläche.
        let a = FollowScroll.anchor(lineY: measured, paragraphHeight: h, viewportHeight: v)!
        XCTAssertLessThan(FollowScroll.lineInViewport(lineY: 202, anchor: a, paragraphHeight: h, viewportHeight: v), 0)
    }

    func testAnchorMovesOnlyInSteps() {
        XCTAssertEqual(FollowScroll.anchor(lineY: 800, paragraphHeight: 8000, viewportHeight: 400),
                       FollowScroll.anchor(lineY: 850, paragraphHeight: 8000, viewportHeight: 400))
        XCTAssertNotEqual(FollowScroll.anchor(lineY: 800, paragraphHeight: 8000, viewportHeight: 400),
                          FollowScroll.anchor(lineY: 900, paragraphHeight: 8000, viewportHeight: 400))
        XCTAssertEqual(FollowScroll.anchor(lineY: 9000, paragraphHeight: 8000, viewportHeight: 400), 1)
        XCTAssertEqual(FollowScroll.anchor(lineY: -50, paragraphHeight: 8000, viewportHeight: 400), 0)
    }

    func testTextLengthMatchesText() {
        for p in LibraryText.paragraphs("# Kapitel eins\n\nErster Absatz mit Umlauten äöü und Emoji 🙂.\n\nZweiter.") {
            XCTAssertEqual(p.textLength, p.text.utf16.count, p.text)
        }
    }
}
