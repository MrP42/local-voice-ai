import XCTest
@testable import VoiceCore

/// Gemeinsame Fälle mit dem Desktop (`apps/local-voice/tests/fixtures/book-start.json`).
final class BookStartTests: XCTestCase {
    struct Fixture: Decodable { let books: [Book] }
    struct Book: Decodable { let name: String; let text: String; let cases: [Case] }
    struct Case: Decodable { let readPreface: Bool; let startsWith: String; let hasPreface: Bool }

    func testSharedFixturesMatchDesktop() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("local-voice/tests/fixtures/book-start.json")
        let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: url))
        XCTAssertGreaterThan(fixture.books.count, 5)
        for book in fixture.books {
            let ns = book.text as NSString
            for c in book.cases {
                let r = BookStart.start(book.text, readPreface: c.readPreface)
                let label = "\(book.name) (Vorwort \(c.readPreface ? "mit" : "ohne"))"
                if c.startsWith.isEmpty {
                    XCTAssertEqual(r.offset, 0, label)
                } else {
                    let len = min((c.startsWith as NSString).length, ns.length - r.offset)
                    XCTAssertEqual(ns.substring(with: NSRange(location: r.offset, length: len)), c.startsWith, label)
                }
                XCTAssertEqual(r.hasPreface, c.hasPreface, label)
            }
        }
    }

    func testHeadingsCompareWithoutMarkdown() {
        XCTAssertEqual(BookStart.normalizeHeading("## *Inhalt:* {{S. 3}}"), "inhalt")
        XCTAssertEqual(BookStart.start(""), BookStart.Result(offset: 0, hasPreface: false))
    }

    func testOffsetMatchesLibraryParagraphs() {
        // Der Startpunkt fällt auf einen Absatzbeginn, den LibraryText kennt.
        let text = "# Inhalt\n\nEins\n\nZwei\n\nDrei\n\n# Eins\n\n" + String(repeating: "Ein langer Satz über das Meer. ", count: 30)
        let r = BookStart.start(text)
        let paras = LibraryText.paragraphs(text)
        XCTAssertEqual(paras[LibraryText.paragraphIndex(paras, at: Double(r.offset))].start, r.offset)
    }
}
