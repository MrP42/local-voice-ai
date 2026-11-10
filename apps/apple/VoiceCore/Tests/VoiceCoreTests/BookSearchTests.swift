import XCTest
@testable import VoiceCore

/// Suche im Buch (lva#476): dieselben Fälle wie `tests/unit/bookSearch.test.ts` am Desktop.
final class BookSearchTests: XCTestCase {
    private func paras(_ texts: [String]) -> [TextParagraph] {
        LibraryText.paragraphs(texts.joined(separator: "\n\n"))
    }

    private lazy var grimm = paras([
        "# Der Froschkönig",
        "Es war einmal eine kleine süße Dirne, die über die Straße zur Großmutter ging.",
        "# Rotkäppchen",
        "Da kam der Wolf 🐸 über die Straße.",
    ])

    func testFoldingLikeTheAblage() {
        XCTAssertEqual(BookSearch.fold(text: "Großmutter  SÜSSE Æsop Øre ĲSSEL Ħal Café\u{00AD}haus"),
                       "grossmutter susse aesop ore ijssel hal cafehaus")
    }

    func testHitsCarryParagraphRangeAndSnippet() {
        let r = BookSearch.search(BookSearchIndex(paragraphs: grimm), query: "strasse")
        XCTAssertEqual(r.total, 2)
        XCTAssertEqual(r.hits.first?.paragraph, 1)
        XCTAssertEqual(r.hits.first?.match, "Straße")
        XCTAssertEqual((grimm[1].text as NSString).substring(with: r.hits[0].range), "Straße")
    }

    func testPartialSharpSAndUmlaut() {
        let index = BookSearchIndex(paragraphs: grimm)
        XCTAssertEqual(BookSearch.search(index, query: "sus").hits.first?.match, "süß")
        XCTAssertEqual(BookSearch.search(index, query: "konig").hits.map(\.match), ["könig"])
    }

    func testEmojiDoesNotShiftRanges() {
        let hit = BookSearch.search(BookSearchIndex(paragraphs: grimm), query: "🐸 über").hits.first
        XCTAssertEqual(hit?.match, "🐸 über")
        XCTAssertEqual(hit?.before, "Da kam der Wolf ")
    }

    func testAllWordsInOneParagraph() {
        let index = BookSearchIndex(paragraphs: grimm)
        let r = BookSearch.search(index, query: "wolf straße da")
        XCTAssertTrue(r.allWords)
        XCTAssertEqual(r.total, 1)
        XCTAssertEqual(r.hits.first?.match, "Da")
        XCTAssertEqual(BookSearch.search(index, query: "rotkäppchen wolf").total, 0)
        XCTAssertEqual(BookSearch.search(index, query: " e ").total, 0)
    }

    func testSnippetCutsAtWords() {
        let long = paras([String(repeating: "Wort ", count: 40) + "Ziel " + String(repeating: "Ende ", count: 40)])
        let hit = BookSearch.search(BookSearchIndex(paragraphs: long), query: "ziel").hits[0]
        XCTAssertTrue(hit.before.hasPrefix("…"))
        XCTAssertTrue(hit.before.hasSuffix("Wort "))
        XCTAssertTrue(hit.after.hasSuffix("…"))
    }

    func testContentsFromHeadings() {
        let c = BookContents.entries(grimm)
        XCTAssertEqual(c.map(\.title), ["Der Froschkönig", "Rotkäppchen"])
        XCTAssertEqual(BookContents.chapter(of: 3, in: c), 1)
        XCTAssertEqual(BookContents.chapter(of: 0, in: c), 0)
    }
}
