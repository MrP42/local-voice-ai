import XCTest
@testable import VoiceCore

final class TextIntakeTests: XCTestCase {
    // MARK: Bereinigung

    func testCleanKeepsParagraphsAndNormalizesLineEnds() {
        XCTAssertEqual(TextIntake.clean("Erster Absatz.\r\n\r\n\r\nZweiter\u{00A0}Absatz.  \r\n"), "Erster Absatz.\n\nZweiter Absatz.")
    }

    func testCleanJoinsHardWrapInsideSentence() {
        XCTAssertEqual(TextIntake.clean("Die Sitzung beginnt\num neun Uhr und endet\ngegen elf."), "Die Sitzung beginnt um neun Uhr und endet gegen elf.")
    }

    func testCleanResolvesHyphenation() {
        XCTAssertEqual(TextIntake.clean("Die Verwal-\ntung prüft den An-\ntrag."), "Die Verwaltung prüft den Antrag.")
    }

    func testCleanKeepsCompoundHyphenBeforeCapital() {
        // „E-Mail-\nAdresse“: Großbuchstabe danach, also kein Silbentrenner. Zeile bleibt getrennt.
        XCTAssertEqual(TextIntake.clean("Bitte die E-Mail-\nAdresse angeben."), "Bitte die E-Mail-\nAdresse angeben.")
    }

    func testCleanKeepsListsAndSentenceEnds() {
        let list = "Einkauf:\nBrot\nMilch\nÄpfel"
        XCTAssertEqual(TextIntake.clean(list), list)
        XCTAssertEqual(TextIntake.clean("Satz eins.\nweiter klein"), "Satz eins.\nweiter klein")
    }

    func testCleanRemovesInvisibleCharactersAndCollapsesSpaces() {
        XCTAssertEqual(TextIntake.clean("Zeit\u{00AD}ung\u{200B}  mit   Leer\tzeichen"), "Zeitung mit Leer zeichen")
    }

    func testCleanEmpty() {
        XCTAssertEqual(TextIntake.clean("  \n\n \r\n"), "")
    }

    // MARK: Titel

    func testTitleFromFirstLine() {
        XCTAssertEqual(TextIntake.title(for: "\n\n# Bekanntmachung  der Gemeinde\nText", fallback: "x"), "Bekanntmachung der Gemeinde")
    }

    func testTitleCutsAtWordBoundary() {
        let line = "Hinweise zur Abfallentsorgung in den Ortsteilen Nord und Süd ab dem ersten Januar"
        let t = TextIntake.title(for: line, fallback: "x")
        XCTAssertTrue(t.hasSuffix("…"))
        XCTAssertLessThanOrEqual(t.count, TextIntake.maxTitleLength + 1)
        XCTAssertEqual(t, "Hinweise zur Abfallentsorgung in den Ortsteilen Nord und…")
    }

    func testTitleFallback() {
        XCTAssertEqual(TextIntake.title(for: " \n- \n", fallback: "Eingefügter Text"), "Eingefügter Text")
    }

    // MARK: Texterkennung

    private func line(_ text: String, x: Double = 0.1, top: Double, width: Double = 0.8, height: Double = 0.03) -> OcrLine {
        OcrLine(text: text, minX: x, minY: top - height, maxX: x + width, maxY: top)
    }

    func testAssembleOrdersTopDownAndJoinsWrappedLines() {
        // Absichtlich ungeordnet, wie Vision es liefern kann.
        let lines = [
            line("gegen elf Uhr.", top: 0.86, width: 0.3),
            line("Die Sitzung beginnt um neun und endet", top: 0.90),
            line("Neuer Absatz nach Abstand.", top: 0.70, width: 0.5),
        ]
        XCTAssertEqual(TextIntake.assemble(lines), "Die Sitzung beginnt um neun und endet gegen elf Uhr.\n\nNeuer Absatz nach Abstand.")
    }

    func testAssembleBreaksAfterShortSentenceEnd() {
        let lines = [
            line("Kurzer Satz.", top: 0.90, width: 0.3),
            line("Nächster Absatz direkt darunter.", top: 0.86),
        ]
        XCTAssertEqual(TextIntake.assemble(lines), "Kurzer Satz.\n\nNächster Absatz direkt darunter.")
    }

    func testAssembleMergesSameRowLeftToRight() {
        let lines = [
            OcrLine(text: "Betrag", minX: 0.6, minY: 0.80, maxX: 0.8, maxY: 0.83),
            OcrLine(text: "Gebühr:", minX: 0.1, minY: 0.801, maxX: 0.3, maxY: 0.829),
        ]
        XCTAssertEqual(TextIntake.assemble(lines), "Gebühr: Betrag")
    }

    func testAssembleReadsTwoColumnsLeftThenRight() {
        var lines: [OcrLine] = []
        for i in 0..<4 {
            let top = 0.9 - Double(i) * 0.04
            lines.append(line("links \(i)", x: 0.05, top: top, width: 0.38))
            lines.append(line("rechts \(i)", x: 0.55, top: top, width: 0.38))
        }
        let text = TextIntake.assemble(lines)
        XCTAssertEqual(text, "links 0 links 1 links 2 links 3\n\nrechts 0 rechts 1 rechts 2 rechts 3")
    }

    func testAssembleSingleColumnWhenLineCrossesGutter() {
        var lines: [OcrLine] = [line("Überschrift über beide Spalten", x: 0.05, top: 0.95, width: 0.9)]
        for i in 0..<4 {
            let top = 0.85 - Double(i) * 0.04
            lines.append(line("L\(i)", x: 0.05, top: top, width: 0.38))
            lines.append(line("R\(i)", x: 0.55, top: top, width: 0.38))
        }
        XCTAssertTrue(TextIntake.assemble(lines).hasPrefix("Überschrift über beide Spalten\n\nL0 R0"))
    }

    func testAssembleIgnoresEmptyLines() {
        XCTAssertEqual(TextIntake.assemble([line("   ", top: 0.5)]), "")
        XCTAssertEqual(TextIntake.assemble([]), "")
    }

    // MARK: Texteingang

    private func tempRoot() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("textinbox-\(UUID().uuidString)", isDirectory: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func imageFile(_ bytes: [UInt8]) throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("bild-\(UUID().uuidString)")
        try Data(bytes).write(to: url)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    func testInboxRoundTripOldestFirst() throws {
        let inbox = TextInbox(root: try tempRoot())
        let source = try imageFile([1, 2, 3])
        let second = try inbox.deposit(text: "Zweiter", imageFiles: [], source: "Teilen-Menü", now: Date(timeIntervalSince1970: 200))
        let first = try inbox.deposit(text: nil, imageFiles: [source], source: nil, now: Date(timeIntervalSince1970: 100))
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path), "Quelle bleibt; der Eingang hat eine Kopie")
        let items = inbox.pending()
        XCTAssertEqual(items.map(\.id), [first, second])
        XCTAssertEqual(items[0].images.count, 1)
        XCTAssertEqual(try Data(contentsOf: items[0].images[0]), Data([1, 2, 3]))
        XCTAssertNil(items[0].text)
        XCTAssertEqual(items[1].text, "Zweiter")
        XCTAssertEqual(items[1].source, "Teilen-Menü")
        inbox.remove(first)
        XCTAssertEqual(inbox.pending().map(\.id), [second])
    }

    func testInboxRejectsEmptyAndTooMany() throws {
        let inbox = TextInbox(root: try tempRoot())
        XCTAssertThrowsError(try inbox.deposit(text: "  \n", imageFiles: [], source: nil)) { XCTAssertEqual($0 as? TextInbox.Failure, .empty) }
        let one = try imageFile([0])
        let many = Array(repeating: one, count: TextInbox.maxImages + 1)
        XCTAssertThrowsError(try inbox.deposit(text: nil, imageFiles: many, source: nil)) { XCTAssertEqual($0 as? TextInbox.Failure, .tooManyImages) }
        let missing = FileManager.default.temporaryDirectory.appendingPathComponent("fehlt-\(UUID().uuidString)")
        XCTAssertThrowsError(try inbox.deposit(text: "x", imageFiles: [missing], source: nil)) { XCTAssertEqual($0 as? TextInbox.Failure, .tooLarge) }
        XCTAssertTrue(inbox.pending().isEmpty)
    }

    func testInboxIgnoresStagingAndForeignPaths() throws {
        let root = try tempRoot()
        let inbox = TextInbox(root: root)
        try FileManager.default.createDirectory(at: root.appendingPathComponent(".neu-x"), withIntermediateDirectories: true)
        let bad = root.appendingPathComponent("boese", isDirectory: true)
        try FileManager.default.createDirectory(at: bad, withIntermediateDirectories: true)
        let manifest = #"{"version":1,"created":1,"text":"../../geheim.txt","images":["/etc/passwd"]}"#
        try Data(manifest.utf8).write(to: bad.appendingPathComponent("eingang.json"))
        XCTAssertTrue(inbox.pending().isEmpty)
        inbox.remove("../boese")
        XCTAssertTrue(FileManager.default.fileExists(atPath: bad.path))
    }
}
