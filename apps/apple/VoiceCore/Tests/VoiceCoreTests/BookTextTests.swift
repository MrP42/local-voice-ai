import XCTest
@testable import VoiceCore

final class BookTextTests: XCTestCase {
    private func makeEpub(chapters: [String], compress: Bool = true) throws -> Data {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dir) }
        func write(_ path: String, _ text: String) throws {
            let url = dir.appendingPathComponent(path)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try text.write(to: url, atomically: true, encoding: .utf8)
        }
        try write("mimetype", "application/epub+zip")
        try write("META-INF/container.xml", """
        <?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
        <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>
        """)
        let items = chapters.indices.map { "<item id=\"c\($0)\" href=\"text/kap%20\($0).xhtml\" media-type=\"application/xhtml+xml\"/>" }.joined()
        let refs = chapters.indices.reversed().map { "<itemref idref=\"c\($0)\"/>" }.joined()
        try write("OEBPS/content.opf", """
        <?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
        <metadata><dc:title>Die Verwandlung</dc:title><dc:creator>Franz Kafka</dc:creator><dc:language>de</dc:language></metadata>
        <manifest>\(items)</manifest><spine>\(refs)</spine></package>
        """)
        for (i, body) in chapters.enumerated() { try write("OEBPS/text/kap \(i).xhtml", body) }
        let out = dir.appendingPathComponent("book.epub")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/zip")
        p.currentDirectoryURL = dir
        p.arguments = ["-q", "-X", compress ? "-9" : "-0", "-r", out.path, "mimetype", "META-INF", "OEBPS"]
        try p.run(); p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0)
        return try Data(contentsOf: out)
    }

    private let chapterA = """
    <?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>x</title><style>p{}</style></head>
    <body><h1>Erstes Kapitel</h1><p>Als Gregor Samsa eines Morgens&nbsp;aus unruhigen Träumen erwachte,</p>
    <p>fand er sich <em>verwandelt</em>.<br/>Neue Zeile</p></body></html>
    """
    private let chapterB = """
    <?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><body><div><p>Zweites &mdash; Ende</p></div></body></html>
    """

    func testEpubReadsSpineInOrderWithMetadata() throws {
        for compress in [true, false] {
            let book = try BookText.epub(try makeEpub(chapters: [chapterA, chapterB], compress: compress))
            XCTAssertEqual(book.title, "Die Verwandlung")
            XCTAssertEqual(book.authors, ["Franz Kafka"])
            XCTAssertEqual(book.language, "de")
            // Die Spine im Test ist umgekehrt: Kapitel B zuerst.
            XCTAssertEqual(book.text, "Zweites — Ende\n\n# Erstes Kapitel\n\nAls Gregor Samsa eines Morgens aus unruhigen Träumen erwachte,\n\nfand er sich verwandelt. Neue Zeile")
            let paragraphs = LibraryText.paragraphs(book.text)
            XCTAssertEqual(paragraphs.filter(\.heading).map(\.text), ["Erstes Kapitel"])
        }
    }

    func testBrokenChapterFallsBackToTagStripping() {
        let text = XHTMLText.paragraphs(Data("<html><body><p>Eins<p>Zwei &amp; drei</p><script>x()</script></body>".utf8))
        XCTAssertTrue(text.contains("Eins"))
        XCTAssertFalse(text.contains("x()"))
    }

    func testRejectsNonZipAndEmptyText() {
        XCTAssertThrowsError(try ZipArchive(data: Data("kein zip".utf8)))
        XCTAssertThrowsError(try BookText.plain(Data("  \n ".utf8)))
        XCTAssertEqual(try BookText.plain(Data("a\r\nb".utf8)), "a\nb")
    }

    func testPathNormalization() {
        XCTAssertEqual(BookText.normalize("OEBPS/text/../img/./a.png"), "OEBPS/img/a.png")
    }
}

final class BookTextEntityTests: XCTestCase {
    func testHtmlEntitiesAndUnclosedVoidTagsKeepTheChapter() {
        let html = "<html><body><h2>K&auml;fer</h2><p>Gro&szlig;e &bdquo;Rede&ldquo;<br>neu &amp; alt &#8211; &copy; &unbekannt;</p></body></html>"
        XCTAssertEqual(XHTMLText.paragraphs(Data(html.utf8)), "# Käfer\n\nGroße „Rede“ neu & alt – ©")
    }

    func testFallbackDecodesEntitiesAndKeepsHeadings() {
        let broken = "<html><body><h1>Titel</h1><p>A &amp; B &#x2014; C<p>Zwei</body>"
        XCTAssertEqual(XHTMLText.paragraphs(Data(broken.utf8)), "# Titel\n\nA & B — C\n\nZwei")
    }
}

final class LibraryHeardTests: XCTestCase {
    func testMarkHeardCountsLongParagraphAndFinishes() {
        let rec = ProgressRecord(offset: 0, length: 10_000, updated: 1)
        let out = LibraryProgress.markHeard(rec, from: 0, to: 9_900)
        XCTAssertEqual(out.heard, [[0, 9_900]])
        XCTAssertEqual(out.finished, true)
    }
}
