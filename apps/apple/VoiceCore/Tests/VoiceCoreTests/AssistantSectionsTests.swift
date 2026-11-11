import XCTest
@testable import VoiceCore

// Quellblöcke (Paket A, lva#494, E4.2b, Befund H3): Die Einheit des Zitierens ist der gespeicherte Block zwischen STRUKTURELLEN
// Grenzen der Quelle, nie ein Satz. Die App erzeugt keine Grenze aus Satzzeichen, trennt nichts innerhalb eines Blocks und zieht
// keine zwei Blöcke zusammen.

final class AssistantSectionsTests: XCTestCase {
    // MARK: Notiz (Textblöcke zwischen Leerzeilen)

    func testNoteBlocksAreSeparatedOnlyByBlankLinesAndStayUnchangedInside() {
        let text = "Erster Block mit zwei Zeilen.\nDie zweite Zeile gehört dazu.\n\nZweiter Block.\n   \nDritter Block ohne Satzzeichen"
        let blocks = AssistantSections.blocks(of: text, locator: nil)
        XCTAssertEqual(blocks.map(\.text), [
            "Erster Block mit zwei Zeilen.\nDie zweite Zeile gehört dazu.", "Zweiter Block.", "Dritter Block ohne Satzzeichen",
        ])
    }

    /// Eine Abkürzung am Zeilenende trennt NICHTS: Der Block ist der Text zwischen Leerzeilen, auch mit „Dr.“ oder
    /// „Regierungsdir.“ am Ende einer Zeile (negativ, Codex-Gegenbeispiele).
    func testAnAbbreviationAtTheEndOfALineNeverSplitsTheBlock() {
        for first in ["Dr.", "Regierungsdir.", "Prof.", "z. B.", "Nr.", "Das gilt für Dr."] {
            let text = "\(first)\nMeyer hat das Protokoll gelesen."
            let blocks = AssistantSections.blocks(of: text, locator: "Notiz")
            XCTAssertEqual(blocks.count, 1, first)
            XCTAssertEqual(blocks.first?.text, text, first)
        }
    }

    func testTwoBlocksAreNeverMergedAndNeverShortened() {
        let a = "Anna übernimmt die Planung."
        let b = "Danach folgt die Leitung durch Berta."
        let blocks = AssistantSections.blocks(of: "\(a)\n\n\(b)", locator: nil)
        XCTAssertEqual(blocks.map(\.text), [a, b])
    }

    func testWhitespaceOnlyEdgesFallAwayButNothingElse() {
        let blocks = AssistantSections.blocks(of: "\n\n  Ein Block mit  doppelten  Leerzeichen.  \n\n\n", locator: "L")
        XCTAssertEqual(blocks.map(\.text), ["Ein Block mit  doppelten  Leerzeichen."])
        XCTAssertEqual(blocks.first?.locator, "L")
        XCTAssertTrue(AssistantSections.blocks(of: "  \n \n", locator: nil).isEmpty)
    }

    // MARK: Protokoll (erzeugt)

    func testGeneratedMinutesPartsAreOneBlockEachAndEmptyPartsAreSkipped() {
        let blocks = AssistantSections.generated(["Zusammenfassung: Es ging um Beispielburg.", "  ", "Beschlüsse: Keine."], locator: "Protokoll")
        XCTAssertEqual(blocks.map(\.text), ["Zusammenfassung: Es ging um Beispielburg.", "Beschlüsse: Keine."])
        XCTAssertEqual(Set(blocks.map(\.locator)), ["Protokoll"])
    }

    // MARK: Besprechung (Segment der Aufnahme)

    func testMeetingBlockIsOneUsableSegmentInTimeOrder() {
        let segments = [
            AssistantSegment(start: 0, line: "Anna: Wir beginnen mit dem Budget", usable: true),
            AssistantSegment(start: 5, line: "Berta: nicht freigegeben", usable: false),
            AssistantSegment(start: 9, line: "   ", usable: true),
            AssistantSegment(start: 12, line: "Berta: Das Budget steht. Der Plan folgt.", usable: true),
        ]
        let blocks = AssistantSections.meeting(segments) { "\(Int($0)) s" }
        XCTAssertEqual(blocks.map(\.text), ["Anna: Wir beginnen mit dem Budget", "Berta: Das Budget steht. Der Plan folgt."])
        XCTAssertEqual(blocks.map(\.locator), ["0 s", "12 s"])
    }

    func testMeetingWithoutUsableSegmentsHasNoBlocks() {
        XCTAssertTrue(AssistantSections.meeting([AssistantSegment(start: 0, line: "A: x", usable: false)]) { _ in "" }.isEmpty)
        XCTAssertTrue(AssistantSections.meeting([]) { _ in "" }.isEmpty)
    }

    // MARK: Buch (Absatz)

    private func paragraph(_ tag: String) -> String {
        (1...4).map { "Satz \(tag) \($0) handelt von der Stadt Beispielburg am Fluss." }.joined(separator: " ")
    }

    private func bookBlocks(_ parts: [String]) -> [BookTalkBlock] {
        BookTalkIndex.blocks(in: parts.joined(separator: "\n\n"))
    }

    func testBookBlockIsOneParagraphWithChapterTitleAsLocator() {
        let paragraphs = bookBlocks(["# Kapitel Eins", paragraph("A"), paragraph("B"), "# Kapitel Zwei", paragraph("C")])
        let sections = AssistantSections.book(paragraphs, frontier: nil)
        XCTAssertEqual(sections.map(\.text), [paragraph("A"), paragraph("B"), paragraph("C")], "ganze Absätze, unverändert")
        XCTAssertEqual(sections.map(\.locator), ["Kapitel Eins", "Kapitel Eins", "Kapitel Zwei"])
    }

    func testBookParagraphWithAnAbbreviationAtALineEndIsOneBlock() {
        let text = "# Kapitel Eins\n\nRegierungsdir.\nMeyer hat das Protokoll gelesen und unterschrieben."
        let sections = AssistantSections.book(BookTalkIndex.blocks(in: text), frontier: nil)
        XCTAssertEqual(sections.count, 1)
        XCTAssertTrue(sections.first?.text.hasPrefix("Regierungsdir.") ?? false)
        XCTAssertTrue(sections.first?.text.hasSuffix("unterschrieben.") ?? false, "kein Schnitt nach der Abkürzung")
    }

    /// Ein Absatz, der die Lesegrenze überragt, fällt GANZ weg (er wird nicht an der Grenze gekürzt).
    func testBookParagraphCrossingTheFrontierIsDroppedWholeNotShortened() {
        let paragraphs = bookBlocks(["# Kapitel Eins", paragraph("A"), paragraph("B"), paragraph("C")])
        XCTAssertEqual(paragraphs.count, 3)
        let inside = paragraphs[1].start + 40
        let sections = AssistantSections.book(paragraphs, frontier: inside)
        XCTAssertEqual(sections.map(\.text), [paragraph("A")], "B beginnt vor der Grenze, endet dahinter: ganz weg; C ebenso")
        XCTAssertTrue(AssistantSections.book(paragraphs, frontier: paragraphs[1].end).map(\.text) == [paragraph("A"), paragraph("B")])
    }

    func testBookFrontierBeforeEverythingGivesNoBlocks() {
        XCTAssertTrue(AssistantSections.book(bookBlocks(["# Kapitel Eins", paragraph("A")]), frontier: 0).isEmpty)
    }

    func testBookPageMarkBecomesTheLocator() {
        let paragraphs = BookTalkIndex.blocks(in: "# Kapitel Eins\n\n{{S. 12}}\n\n\(paragraph("A"))")
        let sections = AssistantSections.book(paragraphs, frontier: nil)
        XCTAssertEqual(sections.count, 1)
        XCTAssertEqual(sections.first?.locator, "S. 12")
        XCTAssertEqual(sections.first?.text, paragraph("A"))
    }
}
