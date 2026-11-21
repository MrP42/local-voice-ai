import Foundation
import XCTest
@testable import VoiceCore

// Gemeinsame Ersetzungsprüfstände und skalargenaue Gegenproben (lva#753).
final class AssistantPronunciationTests: XCTestCase {
    func testSharedReplacementCases() {
        let cases = AssistantSharedFiles.tree("aussprache-pruefstaende.json")["ersetzung"]?
            .items ?? []
        XCTAssertEqual(cases.count, 44)
        for c in cases {
            guard let name = c["name"]?.string else {
                XCTFail("Ersetzungsfall ohne name")
                continue
            }
            guard let text = c["text"]?.string,
                  let expected = c["erwartet"]?.string,
                  case .array(let entries)? = c["eintraege"],
                  case .array(let expectedPieces)? = c["teile"] else {
                XCTFail("Fehlende oder ungültige Felder: \(name)")
                continue
            }
            var rules: [AssistantPronunciationRule] = []
            for entry in entries {
                guard let word = entry["wort"]?.string,
                      let pronunciation = entry["aussprache"]?.string else {
                    XCTFail("Ungültiger Eintrag: \(name)")
                    continue
                }
                rules.append(AssistantPronunciationRule(word: word, pronunciation: pronunciation))
            }
            assertSameBytes(AssistantPronunciation.apply(text, rules: rules), expected, name)
            let pieces = AssistantPronunciation.pieces(text, rules: rules)
            XCTAssertEqual(pieces.count, expectedPieces.count, name)
            for (index, expectedPiece) in expectedPieces.enumerated() {
                guard let expectedText = expectedPiece["text"]?.string,
                      let replaced = expectedPiece["ersetzt"]?.bool else {
                    XCTFail("Ungültiges Stück \(index): \(name)")
                    continue
                }
                guard index < pieces.count else { continue }
                assertSameBytes(pieces[index].text, expectedText, name)
                XCTAssertEqual(pieces[index].replaced, replaced, name)
            }
        }
    }

    func testCanonicallyEquivalentWordsAreReplacedSeparately() {
        // Gegenprobe: rot, wenn String-Vergleich oder replacingOccurrences kanonisch sucht.
        let rules = [
            AssistantPronunciationRule(word: "\u{00E9}", pronunciation: "A"),
            AssistantPronunciationRule(word: "e\u{0301}", pronunciation: "B")
        ]
        assertSameBytes(
            AssistantPronunciation.apply("\u{00E9}|e\u{0301}|\u{00E9}", rules: rules), "A|B|A"
        )
    }

    func testDecomposedRuleDoesNotMatchComposedText() {
        // Gegenprobe: rot, wenn kanonisch gleiche Schreibweisen als Treffer gelten.
        assertSameBytes(AssistantPronunciation.apply("\u{00E9}", rules: [
            AssistantPronunciationRule(word: "e\u{0301}", pronunciation: "B")
        ]), "\u{00E9}")
        assertSameBytes(AssistantPronunciation.apply("e\u{0301}", rules: [
            AssistantPronunciationRule(word: "\u{00E9}", pronunciation: "A")
        ]), "e\u{0301}")
    }

    func testRuleMatchesInsideDecomposedCluster() {
        // Gegenprobe: rot, wenn Grapheme statt einzelner Skalare durchsucht werden.
        let rules = [AssistantPronunciationRule(word: "e", pronunciation: "X")]
        assertSameBytes(AssistantPronunciation.apply("e\u{0301}", rules: rules), "X\u{0301}")
        XCTAssertEqual(AssistantPronunciation.pieces("e\u{0301}", rules: rules), [
            AssistantPronunciationPiece(text: "X", replaced: true),
            AssistantPronunciationPiece(text: "\u{0301}", replaced: false)
        ])
    }

    func testKelvinSignIsNotLatinK() {
        // Gegenprobe: rot, wenn String-Vergleich Kelvinzeichen und K gleichsetzt.
        let rules = [AssistantPronunciationRule(word: "K", pronunciation: "X")]
        assertSameBytes(AssistantPronunciation.apply("\u{212A}K", rules: rules), "\u{212A}X")
    }

    func testNilAndEmptyRulesReturnTextUnchanged() {
        // Gegenprobe: rot, wenn der Rückgabepfad ohne Regeln den Text normalisiert oder leert.
        let text = "e\u{0301}|\u{00E9}|\u{1F600}"
        assertSameBytes(AssistantPronunciation.apply(text, rules: nil), text)
        assertSameBytes(AssistantPronunciation.apply(text, rules: []), text)
        XCTAssertEqual(AssistantPronunciation.pieces(text, rules: []), [
            AssistantPronunciationPiece(text: text, replaced: false)
        ])
        XCTAssertTrue(AssistantPronunciation.pieces("", rules: []).isEmpty)
    }

    func testEmptyWordRuleIsIgnored() {
        // Gegenprobe: rot, wenn ein leeres Wort eingefügt wird oder die Suche nicht fortschreitet.
        let rules = [
            AssistantPronunciationRule(word: "", pronunciation: "Z"),
            AssistantPronunciationRule(word: "a", pronunciation: "b")
        ]
        assertSameBytes(AssistantPronunciation.apply("aa", rules: rules), "bb")
        assertSameBytes(AssistantPronunciation.apply("c", rules: rules), "c")
        XCTAssertTrue(AssistantPronunciation.pieces("", rules: rules).isEmpty)
    }

    func testReplacedTextIsNotSearchedAgain() {
        // Gegenprobe: rot, wenn eingesetzter Text erneut durchsucht wird (Kette oder Schleife).
        assertSameBytes(AssistantPronunciation.apply("aaa", rules: [
            AssistantPronunciationRule(word: "a", pronunciation: "aa")
        ]), "aaaaaa")
        assertSameBytes(AssistantPronunciation.apply("ab", rules: [
            AssistantPronunciationRule(word: "a", pronunciation: "b"),
            AssistantPronunciationRule(word: "b", pronunciation: "a")
        ]), "ba")
    }

    func testLongestMatchWinsThenListOrder() {
        // Gegenprobe: rot, wenn Listenreihenfolge vor Länge oder der spätere Gleichstand gewinnt.
        assertSameBytes(AssistantPronunciation.apply("abc", rules: [
            AssistantPronunciationRule(word: "ab", pronunciation: "1"),
            AssistantPronunciationRule(word: "abc", pronunciation: "2")
        ]), "2")
        assertSameBytes(AssistantPronunciation.apply("x", rules: [
            AssistantPronunciationRule(word: "x", pronunciation: "1"),
            AssistantPronunciationRule(word: "x", pronunciation: "2")
        ]), "1")
    }

    func testNonBMPScalarsAreOneUnit() {
        // Gegenprobe: rot, wenn UTF-16-Längen zum Fortschreiten im Skalarfeld verwendet werden.
        let rules = [AssistantPronunciationRule(word: "\u{1F600}", pronunciation: "smile")]
        assertSameBytes(
            AssistantPronunciation.apply("a\u{1F600}b\u{1F601}", rules: rules),
            "asmileb\u{1F601}"
        )
    }

    func testEqualityPreservesScalarDifferences() {
        // Gegenprobe: rot, wenn die Gleichheit von Regeln oder Stücken String == verwendet.
        let composed = "\u{00E9}"
        let decomposed = "e\u{0301}"
        XCTAssertNotEqual(
            AssistantPronunciationRule(word: composed, pronunciation: "x"),
            AssistantPronunciationRule(word: decomposed, pronunciation: "x")
        )
        XCTAssertNotEqual(
            AssistantPronunciationRule(word: "x", pronunciation: composed),
            AssistantPronunciationRule(word: "x", pronunciation: decomposed)
        )
        // Gegenprobe: rot, wenn `extra` aus der Gleichheit faellt.
        let composedExtra = [AssistantJSONMember("notiz", .string(composed))]
        let decomposedExtra = [AssistantJSONMember("notiz", .string(decomposed))]
        XCTAssertNotEqual(
            AssistantPronunciationRule(word: "a", pronunciation: "x", extra: composedExtra),
            AssistantPronunciationRule(word: "a", pronunciation: "x", extra: decomposedExtra)
        )
        XCTAssertEqual(
            AssistantPronunciationRule(word: "a", pronunciation: "x", extra: composedExtra),
            AssistantPronunciationRule(word: "a", pronunciation: "x", extra: composedExtra)
        )
        XCTAssertNotEqual(
            AssistantPronunciationPiece(text: composed, replaced: true),
            AssistantPronunciationPiece(text: decomposed, replaced: true)
        )
        XCTAssertNotEqual(
            AssistantPronunciationPiece(text: "x", replaced: true),
            AssistantPronunciationPiece(text: "x", replaced: false)
        )
    }

    func testForbiddenScalars() {
        // Gegenprobe: rot, wenn eine verbotene Gruppe fehlt oder ihre Grenzen erweitert werden.
        let forbiddenRanges: [ClosedRange<UInt32>] = [
            0x0000...0x001F, 0x007F...0x009F, 0x2028...0x202E, 0x2066...0x2069
        ]
        for range in forbiddenRanges {
            for scalar in range {
                XCTAssertTrue(AssistantPronunciation.isForbidden(scalar))
            }
        }
        let forbidden: [UInt32] = [0x061C, 0x200E, 0x200F, 0x3C, 0x3E, 0x5B, 0x5D]
        for scalar in forbidden {
            XCTAssertTrue(AssistantPronunciation.isForbidden(scalar))
        }
        let allowed: [UInt32] = [
            0x20, 0x7E, 0xA0, 0x061B, 0x061D, 0x200D, 0x2010,
            0x2027, 0x202F, 0x2065, 0x206A, 0x3B, 0x3D, 0x3F, 0x5A, 0x5C, 0x5E,
            0x0301, 0x1F600
        ]
        for scalar in allowed {
            XCTAssertFalse(AssistantPronunciation.isForbidden(scalar))
        }
    }
}
