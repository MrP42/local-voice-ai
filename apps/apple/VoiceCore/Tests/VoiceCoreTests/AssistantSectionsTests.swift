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

    // MARK: Rohtext (B1, Runde 7): der gespeicherte Block, Zeichen für Zeichen

    private func bytes(_ s: String?) -> [UInt8]? { s.map { Array($0.utf8) } }

    /// Notiz: Der Rohtext ist der Teilstring des gespeicherten Textes zwischen den Trennern, ohne Ersetzen von Zeilentrennern,
    /// Falten, Trimmen oder Normalisieren. Trenner (Zeilenende, Leerzeile) gehören nie zum Block, Zeilenenden darin schon.
    func testNoteRawIsTheStoredSubstringForEveryKindOfLineEndAndWhitespace() {
        let nfd = "Mu\u{0308}hlbach bru\u{0308}tet"
        let cases: [(name: String, text: String, raw: [String])] = [
            ("CR", "Alpha\rBeta", ["Alpha\rBeta"]),
            ("CRLF", "Alpha\r\nBeta\r\n\r\nGamma", ["Alpha\r\nBeta", "Gamma"]),
            ("LF", "Alpha\nBeta", ["Alpha\nBeta"]),
            ("U+2028", "Alpha\u{2028}Beta", ["Alpha\u{2028}Beta"]),
            ("U+0085", "Alpha\u{0085}Beta", ["Alpha\u{0085}Beta"]),
            ("doppelte Leerzeichen", "Alpha  Beta   Gamma", ["Alpha  Beta   Gamma"]),
            ("Tab", "Alpha\tBeta\t\tGamma", ["Alpha\tBeta\t\tGamma"]),
            ("NFD", nfd, [nfd]),
            ("Rand-Leerraum", "  Alpha \t\n\n\t Beta  \n", ["  Alpha \t", "\t Beta  "]),
            ("gemischt", "A\r\n B  \rC\n\n  \n\nD", ["A\r\n B  \rC", "D"]),
        ]
        for c in cases {
            let blocks = AssistantSections.blocks(of: c.text, locator: nil)
            XCTAssertEqual(blocks.compactMap { bytes($0.raw) }, c.raw.map { Array($0.utf8) }, c.name)
            XCTAssertTrue(blocks.allSatisfy { $0.quotable }, c.name)
        }
    }

    /// Die aufbereitete Fassung (`text`) bleibt für die Suche, ist aber NICHT der Rohtext (Gegenprobe: sie ersetzt `\r` durch `\n`).
    func testTheProcessedTextIsOnlyForSearchAndDiffersFromTheRawText() {
        let block = AssistantSections.blocks(of: "Alpha\rBeta", locator: nil)[0]
        XCTAssertEqual(block.text, "Alpha\nBeta")
        XCTAssertEqual(bytes(block.raw), Array("Alpha\rBeta".utf8))
        let nfd = AssistantSections.blocks(of: "Mu\u{0308}hl", locator: nil)[0]
        XCTAssertEqual(nfd.raw, "Mühl", "kanonisch gleich ...")
        XCTAssertNotEqual(bytes(nfd.raw), Array("Mühl".utf8), "... aber nicht dieselben Bytes: NFD bleibt NFD")
    }

    /// Transkript: Der Rohtext ist das gespeicherte Textfeld des Segments (ohne Sprecher, nicht getrimmt); ohne ihn ist das Segment
    /// nicht zitierbar, nie ersatzweise die Zeile mit Sprecher.
    func testMeetingRawIsTheStoredSegmentTextWithoutTheSpeakerAndWithoutRawItIsNotQuotable() {
        let stored = "  Wir beginnen\r\nmit dem  Budget.\t"
        let blocks = AssistantSections.meeting([
            AssistantSegment(start: 0, line: "Anna: Wir beginnen mit dem Budget.", raw: stored, usable: true),
            AssistantSegment(start: 5, line: "Berta: ohne Rohtext", usable: true),
        ]) { "\(Int($0)) s" }
        XCTAssertEqual(blocks.count, 2)
        XCTAssertEqual(bytes(blocks[0].raw), Array(stored.utf8))
        XCTAssertTrue(blocks[0].quotable)
        XCTAssertNil(blocks[1].raw)
        XCTAssertFalse(blocks[1].quotable)
        XCTAssertEqual(blocks[0].text, "Anna: Wir beginnen mit dem Budget.", "die Zeile mit Sprecher dient nur der Suche")
    }

    /// Buch: Der Rohtext ist der Teilstring des gespeicherten Buchtextes (Zeilenenden und doppelte Leerzeichen darin bleiben).
    func testBookRawIsTheStoredParagraphIncludingLineEndsAndDoubleSpaces() {
        let one = "Quasselbert  wandert mit dem Fahrrad\r\nnach Pappelhausen."
        let two = "Die Tante\tLavendula wartet dort."
        let text = "# Kapitel Eins\r\n\r\n\(one)\r\n\r\n\(two)\r\n"
        let sections = AssistantSections.book(BookTalkIndex.blocks(in: text), frontier: nil)
        XCTAssertEqual(sections.compactMap { bytes($0.raw) }, [Array(one.utf8), Array(two.utf8)])
        XCTAssertTrue(sections.allSatisfy(\.quotable))
        XCTAssertEqual(sections.map(\.text), ["Quasselbert wandert mit dem Fahrrad nach Pappelhausen.", "Die Tante Lavendula wartet dort."],
                       "aufbereitet nur für die Suche")
    }

    /// Buch: Enthält der Absatz Sprecher-, Stil- oder Seitenmarken, wäre der aufbereitete Text nicht der gespeicherte: kein
    /// eindeutiger Rohtext, der Absatz ist nicht zitierbar (und die gesäuberte Fassung nie ein Ersatz).
    func testABookParagraphWithMarksHasNoRawTextAndIsNotQuotable() {
        let cases = [
            "<Erzähler> Quasselbert wandert nach Pappelhausen.",
            "Quasselbert [calm] wandert nach Pappelhausen.",
            "Quasselbert wandert {{S. 4}} nach Pappelhausen.",
        ]
        for paragraph in cases {
            let sections = AssistantSections.book(BookTalkIndex.blocks(in: "# Kapitel\n\n\(paragraph)"), frontier: nil)
            XCTAssertEqual(sections.count, 1, paragraph)
            XCTAssertNil(sections.first?.raw, paragraph)
            XCTAssertEqual(sections.first?.quotable, false, paragraph)
        }
    }

    /// Buch: Ein Absatz über der Höchstlänge hat keinen Rohtext (er wird nie ausgegeben, nie gekürzt).
    func testABookParagraphLongerThanTheLimitHasNoRawText() {
        let long = Array(repeating: "Wort", count: 200).joined(separator: " ")
        XCTAssertGreaterThan(long.utf16.count, AssistantGrounding.maxBlockChars)
        let sections = AssistantSections.book(BookTalkIndex.blocks(in: long), frontier: nil)
        XCTAssertNil(sections.first?.raw)
    }

    /// Der Rohtext wird gegen die Höchstlänge gemessen, nicht der aufbereitete: 200 Zeilen „ab“ mit CRLF sind aufbereitet 599, roh 798.
    func testTheLimitCountsTheRawBlockNotTheProcessedText() async {
        let text = Array(repeating: "ab", count: 200).joined(separator: "\r\n")
        let section = AssistantSections.blocks(of: text, locator: nil)[0]
        XCTAssertLessThanOrEqual(section.text.count, AssistantGrounding.maxBlockChars)
        XCTAssertGreaterThan(section.raw?.utf16.count ?? 0, AssistantGrounding.maxBlockChars)
        let corpus = AssistantCorpus()
        await corpus.upsert(AssistantDocument(item: AssistantItem(area: .notes, id: "n", title: "Notiz"), version: "1", sections: [section]))
        let stats = await corpus.stats()
        XCTAssertEqual(stats.tooLong, 1)
        XCTAssertEqual(stats.blocks, 0)
    }
}
