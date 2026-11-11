import XCTest
@testable import VoiceCore

/// Inhaltsverzeichnis beim EPUB-Import (lva#476, Codex-Befunde): nav-Dokument im Spine, Sprungmarken,
/// XML-Entitäten in href/src. Die Archive sind gespeichert (`zip -0`), damit die Tests auch ohne
/// Compression-Framework laufen.
final class EpubTocImportTests: XCTestCase {
    private func makeEpub(opf: String, files: [String: String]) throws -> Data {
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
        try write("OEBPS/content.opf", opf)
        for (path, body) in files { try write("OEBPS/" + path, body) }
        let out = dir.appendingPathComponent("book.epub")
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/zip")
        p.currentDirectoryURL = dir
        p.arguments = ["-q", "-X", "-0", "-r", out.path, "mimetype", "META-INF", "OEBPS"]
        try p.run(); p.waitUntilExit()
        XCTAssertEqual(p.terminationStatus, 0)
        return try Data(contentsOf: out)
    }

    private func opf(manifest: String, spine: String, spineAttrs: String = "") -> String {
        """
        <?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
        <metadata><dc:title>Testbuch</dc:title><dc:language>de</dc:language></metadata>
        <manifest>\(manifest)</manifest><spine \(spineAttrs)>\(spine)</spine></package>
        """
    }

    private func page(_ body: String) -> String {
        #"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>x</title></head><body>"#
            + body + "</body></html>"
    }

    private func navPage(_ links: String, extra: String = "") -> String {
        page(#"<nav epub:type="toc"><ol>"# + links + "</ol></nav>" + extra)
    }

    // MARK: Befund 2: nav-Dokument im Spine

    func testNavDocumentInSpineKeepsItsOwnContent() throws {
        let epub = try makeEpub(
            opf: opf(manifest: """
                <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="c1" href="k1.xhtml" media-type="application/xhtml+xml"/>
                """, spine: #"<itemref idref="nav"/><itemref idref="c1"/>"#),
            files: [
                "nav.xhtml": navPage(#"<li><a href="k1.xhtml">Erstes Kapitel</a></li>"#,
                                     extra: "<p>Widmung: Für meine Mutter.</p>"),
                "k1.xhtml": page("<h1>Erstes Kapitel</h1><p>Der erste Absatz des Kapitels.</p>"),
            ])
        let book = try BookText.epub(epub)
        XCTAssertEqual(book.text, "Widmung: Für meine Mutter.\n\n# Erstes Kapitel\n\nDer erste Absatz des Kapitels.")
    }

    func testNavDocumentOnlyTocStillDropsOut() throws {
        let epub = try makeEpub(
            opf: opf(manifest: """
                <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="c1" href="k1.xhtml" media-type="application/xhtml+xml"/>
                """, spine: #"<itemref idref="nav"/><itemref idref="c1"/>"#),
            files: [
                "nav.xhtml": navPage(#"<li><a href="k1.xhtml">Erstes Kapitel</a></li>"#),
                "k1.xhtml": page("<h1>Erstes Kapitel</h1><p>Nur dieser Text.</p>"),
            ])
        XCTAssertEqual(try BookText.epub(epub).text, "# Erstes Kapitel\n\nNur dieser Text.")
    }

    func testCommentsAndCDataDoNotConfuseNavRemoval() throws {
        let dirty = #"<!-- Beispiel: <nav> --><p>Widmung</p><nav epub:type="toc"><ol><li><a href="k1.xhtml">X</a></li></ol></nav><p>Danach</p>"#
        let out = EpubToc.removingNav(dirty)
        XCTAssertTrue(out.contains("<p>Widmung</p>") && out.contains("<p>Danach</p>"), out)
        XCTAssertFalse(out.contains("<!--") || out.contains("<nav"), out)
        XCTAssertEqual(XHTMLText.paragraphs(Data(page(out).utf8)), "Widmung\n\nDanach")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><![CDATA[ <nav> & ]]><p>b</p>"), "<p>a</p> &lt;nav&gt; &amp; <p>b</p>")
        let epub = try makeEpub(
            opf: opf(manifest: """
                <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="c1" href="k1.xhtml" media-type="application/xhtml+xml"/>
                """, spine: #"<itemref idref="nav"/><itemref idref="c1"/>"#),
            files: [
                "nav.xhtml": page(dirty.replacingOccurrences(of: "<li><a href=\"k1.xhtml\">X</a></li>", with: #"<li><a href="k1.xhtml">Erstes Kapitel</a></li>"#)),
                "k1.xhtml": page("<h1>Erstes Kapitel</h1><p>Text.</p>"),
            ])
        XCTAssertEqual(try BookText.epub(epub).text, "Widmung\n\nDanach\n\n# Erstes Kapitel\n\nText.")
    }

    func testNavScannerHandlesPIsDoctypeNestingPrefixAndCDataAtEnd() {
        // (a) PI mit <nav> darin
        XCTAssertEqual(EpubToc.removingNav(#"<?demo <nav> ?><p>Widmung</p><nav epub:type="toc">X</nav>"#), "<p>Widmung</p>")
        // (b) PI mit Kommentarbeginn
        XCTAssertEqual(EpubToc.removingNav("<?demo <!-- ?><p>Widmung</p><!-- Ende --><nav>X</nav>"), "<p>Widmung</p>")
        // (c) verschachtelt: nichts bleibt übrig, der Text danach auch
        XCTAssertEqual(EpubToc.removingNav("<nav><nav>X</nav><p>Rest</p></nav><p>Danach</p>"), "<p>Danach</p>")
        // Präfix und selbstschließend, Anführungszeichen mit > im Attribut
        XCTAssertEqual(EpubToc.removingNav(#"<p>a</p><html:nav title="a>b"><ol/></html:nav><nav/><p>b</p>"#), "<p>a</p><p>b</p>")
        // DOCTYPE mit internem Subset bleibt weg, Text bleibt
        XCTAssertEqual(EpubToc.removingNav(#"<!DOCTYPE html [ <!ENTITY x "<nav>"> ]><p>t</p>"#), "<p>t</p>")
        // Kommentar und PI innerhalb einer nav zählen nicht als Endtag
        XCTAssertEqual(EpubToc.removingNav("<nav><!-- </nav> --><?x </nav> ?>A</nav>B"), "B")
        // CDATA am Dokumentende bleibt als maskierter Text, auch unterminiert
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><![CDATA[ x <nav> & y ]]>"), "<p>a</p> x &lt;nav&gt; &amp; y ")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><![CDATA[ <b>"), "<p>a</p> &lt;b&gt;")
        // Umlaute (Mehrbyte) bleiben unverändert
        XCTAssertEqual(EpubToc.removingNav("<p>Größe über</p><nav>ä</nav>"), "<p>Größe über</p>")
    }

    func testNavScannerSubsetRawTextCaseAndUnterminatedMarkup() {
        // DOCTYPE-Subset: Kommentar mit [ und ], PI und Anführungszeichen zählen nicht als Klammern
        XCTAssertEqual(EpubToc.removingNav("<!DOCTYPE html [<!-- [ -->]><p>Widmung</p><nav>X</nav>"), "<p>Widmung</p>")
        XCTAssertEqual(EpubToc.removingNav(#"<!DOCTYPE html [<?x ] ?><!ENTITY e "]">]><p>Widmung</p><nav>X</nav>"#), "<p>Widmung</p>")
        // script/style: Raw-Text bis zum passenden Endtag, auch gemischt und mit Präfix
        XCTAssertEqual(EpubToc.removingNav(#"<script>let s="<nav>";</script><p>Widmung</p><nav>X</nav>"#), "<p>Widmung</p>")
        XCTAssertEqual(EpubToc.removingNav("<svg:STYLE>a<nav>b</nav></svg:Style><p>Widmung</p><nav>X</nav>"), "<p>Widmung</p>")
        XCTAssertEqual(EpubToc.removingNav("<nav><script>x = '</nav>';</script>A</nav><p>B</p>"), "<p>B</p>")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><script>nie zu"), "<p>a</p>")
        // gemischte Großschreibung
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><NAV><OL/></Nav><p>b</p>"), "<p>a</p><p>b</p>")
        // unterminiert: Text davor bleibt
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><!-- offen"), "<p>a</p>")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><?pi offen"), "<p>a</p>")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><p class=\"x"), "<p>a</p><p class=\"x")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><!DOCTYPE x [ <!ENTITY"), "<p>a</p>")
        XCTAssertEqual(EpubToc.removingNav("<p>a</p><!DOCTYPE html [<!-- offen"), "<p>a</p>")
    }

    /// Invariante: Text außerhalb der äußersten nav geht nie verloren. Zufällige Folgen aus Bausteinen
    /// mit bekannter Ausgabe, danach eine echte nav und ein Nachsatz; unterminierte Konstrukte am Ende
    /// dürfen nichts davor verlieren.
    func testNavScannerNeverLosesTextOutsideTheOutermostNav() {
        let items: [(input: String, output: String)] = [
            ("<p>Text</p>", "<p>Text</p>"), ("Text &amp; mehr", "Text &amp; mehr"),
            ("<!-- <nav> [ ] -->", ""), ("<!---->", ""), ("<?pi <nav> <!-- ?>", ""),
            ("<![CDATA[ <nav> & ]]>", " &lt;nav&gt; &amp; "),
            ("<!DOCTYPE html>", ""), (#"<!DOCTYPE html PUBLIC "-//W3C//DTD" "http://x/y.dtd">"#, ""),
            (#"<!DOCTYPE html [<!ENTITY e "]"><!-- [ --><?x ] ?><!ENTITY f ']'>]>"#, ""),
            ("<script>let s=\"<nav>\";</script>", ""), ("<SCRIPT type='x'>if (a<b) {}</Script >", ""),
            ("<svg:style>a<nav>b</nav></svg:style>", ""), ("<script>a</scriptx>b</script>", ""),
            ("<script/>", "<script/>"),
            (#"<p title="<nav>" data-x='a>b'>T</p>"#, #"<p title="<nav>" data-x='a>b'>T</p>"#),
            ("<navigation>x</navigation>", "<navigation>x</navigation>"), ("<nav-x>y</nav-x>", "<nav-x>y</nav-x>"),
            ("<nav/>", ""), ("<NAV><OL><li>a</li></OL></nav>", ""), ("<nav><nav>x</nav><p>r</p></nav>", ""),
            ("<html:nav><script>'</html:nav>'</script></html:nav>", ""),
        ]
        let unterminated = ["<!-- offen", "<?pi offen", "<![CDATA[ offen", "<!DOCTYPE x [ <!ENTITY e", "<!DOCTYPE x [<!-- o",
                            "<p class=\"offen", "<script>offen", "<style>a</styl", "<nav", "</"]
        var seed: UInt64 = 42
        func next(_ n: Int) -> Int { seed = seed &* 6364136223846793005 &+ 1442695040888963407; return Int((seed >> 33) % UInt64(n)) }
        for round in 0..<600 {
            var input = "", output = ""
            for _ in 0..<(1 + next(6)) {
                let item = items[next(items.count)]
                input += item.input; output += item.output
            }
            if round % 3 == 0 {
                XCTAssertEqual(EpubToc.removingNav(input + "<nav>X<script></nav></script></nav><p>Z</p>"), output + "<p>Z</p>", input)
            } else if round % 3 == 1 {
                XCTAssertEqual(EpubToc.removingNav(input), output, input)
            } else {
                let tail = unterminated[next(unterminated.count)]
                XCTAssertTrue(EpubToc.removingNav(input + tail).hasPrefix(output), input + tail)
            }
        }
    }

    func testPureFragmentLinksPointToTheTocDocumentItself() {
        let nav = ##"<nav epub:type="toc"><ol><li><a href="#start">Anfang</a></li><li><a href="k.xhtml#a">Eins</a></li></ol></nav>"##
        XCTAssertEqual(EpubToc.nav(nav, baseDir: "OEBPS", source: "OEBPS/nav.xhtml"), [
            .init(depth: 0, title: "Anfang", path: "OEBPS/nav.xhtml", fragment: "start"),
            .init(depth: 0, title: "Eins", path: "OEBPS/k.xhtml", fragment: "a"),
        ])
        let ncx = ##"<ncx><navMap><navPoint><navLabel><text>Eins</text></navLabel><content src="#f"/></navPoint></navMap></ncx>"##
        XCTAssertEqual(EpubToc.ncx(ncx, baseDir: "OEBPS", source: "OEBPS/toc.ncx"), [.init(depth: 0, title: "Eins", path: "OEBPS/toc.ncx", fragment: "f")])
        // Ohne Quelldatei wird der Eintrag verworfen, statt auf das Verzeichnis zu zeigen.
        XCTAssertEqual(EpubToc.nav(nav, baseDir: "OEBPS").map(\.title), ["Eins"])
    }

    func testExtractReadsXmlId() {
        let xhtml = page(#"<p>Eins</p><p xml:id="x1">Zwei</p><section xml:id="s"><p>Drei</p></section>"#)
        let out = XHTMLText.extract(Data(xhtml.utf8))
        XCTAssertEqual(out.text, "Eins\n\nZwei\n\nDrei")
        XCTAssertEqual(out.anchors, ["x1": 1, "s": 2])
    }

    // MARK: Befund 3: Ziele mit Sprungmarke

    func testFragmentAnchorsTheTargetParagraph() throws {
        let epub = try makeEpub(
            opf: opf(manifest: """
                <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="c1" href="chapter.xhtml" media-type="application/xhtml+xml"/>
                """, spine: #"<itemref idref="c1"/>"#),
            files: [
                "nav.xhtml": navPage("""
                    <li><a href="chapter.xhtml#start">Die Ankunft</a></li>
                    <li><a href="chapter.xhtml#fehlt">Gibt es nicht</a></li>
                    """),
                "chapter.xhtml": page(#"<p>Vorspann ohne Titel.</p><p id="start">Am Morgen begann die Reise.</p><p>Weiter.</p>"#),
            ])
        let book = try BookText.epub(epub)
        XCTAssertEqual(book.text, "Vorspann ohne Titel.\n\n## Die Ankunft\n\nAm Morgen begann die Reise.\n\nWeiter.")
    }

    func testFragmentOnTitleParagraphOrHeadingDoesNotDuplicate() {
        let e = EpubToc.Entry(depth: 1, title: "Die Ankunft", path: "x", fragment: "s")
        // Marke am Titelabsatz selbst
        XCTAssertEqual(EpubToc.apply("Vorn.\n\nDie Ankunft\n\nText.", entries: [e], anchors: ["s": 1]),
                       "Vorn.\n\n### Die Ankunft\n\nText.")
        // Marke am Absatz nach dem Titel
        XCTAssertEqual(EpubToc.apply("Vorn.\n\nDie Ankunft\n\nText.", entries: [e], anchors: ["s": 2]),
                       "Vorn.\n\n### Die Ankunft\n\nText.")
        // Ziel ist schon eine (andere) Überschrift: nichts einfügen
        XCTAssertEqual(EpubToc.apply("# Anderes\n\nText.", entries: [e], anchors: ["s": 0]), "# Anderes\n\nText.")
        // Fehlendes Ziel wird übersprungen, auch wenn der Titel irgendwo vorkommt
        XCTAssertEqual(EpubToc.apply("Vorn.\n\nDie Ankunft\n\nText.", entries: [e], anchors: [:]), "Vorn.\n\nDie Ankunft\n\nText.")
        // Ohne Marken (Fallback) wird der Titel gesucht wie bisher
        XCTAssertEqual(EpubToc.apply("Vorn.\n\nDie Ankunft\n\nText.", entries: [e], anchors: nil),
                       "Vorn.\n\n### Die Ankunft\n\nText.")
    }

    func testExtractMapsIdsToParagraphs() {
        let xhtml = page(#"<div id="k"><h2 id="h">Titel</h2><p>Eins <span id="mitte">zwei</span></p><a name="alt"/><p id="p3">Drei</p></div><p id="ende"></p>"#)
        let out = XHTMLText.extract(Data(xhtml.utf8))
        XCTAssertEqual(out.text, "# Titel\n\nEins zwei\n\nDrei")
        XCTAssertEqual(out.anchors, ["k": 0, "h": 0, "mitte": 1, "alt": 2, "p3": 2])
    }

    // MARK: Befund 4: Entitäten in href/src

    func testEntitiesInNavAndNcxHrefsAreResolvedBeforeSplitting() {
        let nav = #"<nav epub:type="toc"><ol><li><a href="Text/a&amp;b.xhtml#x&amp;y">A &amp; B</a></li><li><a href="k%26c.xhtml#a%20b">C</a></li></ol></nav>"#
        XCTAssertEqual(EpubToc.nav(nav, baseDir: "OEBPS"), [
            .init(depth: 0, title: "A & B", path: "OEBPS/Text/a&b.xhtml", fragment: "x&y"),
            .init(depth: 0, title: "C", path: "OEBPS/k&c.xhtml", fragment: "a b"),
        ])
        let ncx = #"<ncx><navMap><navPoint><navLabel><text>Eins</text></navLabel><content src="t&amp;u.xhtml#f&amp;g"/></navPoint></navMap></ncx>"#
        XCTAssertEqual(EpubToc.ncx(ncx, baseDir: ""), [.init(depth: 0, title: "Eins", path: "t&u.xhtml", fragment: "f&g")])
    }

    func testEntityHrefReachesTheChapterFileInTheArchive() throws {
        let epub = try makeEpub(
            opf: opf(manifest: """
                <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
                <item id="c1" href="a&amp;b.xhtml" media-type="application/xhtml+xml"/>
                """, spine: #"<itemref idref="c1"/>"#),
            files: [
                "nav.xhtml": navPage(#"<li><a href="a&amp;b.xhtml#s&amp;t">Der Anfang</a></li>"#),
                "a&b.xhtml": page(#"<p>Vorspann.</p><p id="s&amp;t">Es war einmal.</p>"#),
            ])
        XCTAssertEqual(try BookText.epub(epub).text, "Vorspann.\n\n## Der Anfang\n\nEs war einmal.")
    }

    func testEntityHrefInNcxReachesTheChapter() throws {
        let ncx = #"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="n"><navLabel><text>Der Anfang</text></navLabel><content src="a&amp;b.xhtml#s"/></navPoint></navMap></ncx>"#
        let epub = try makeEpub(
            opf: opf(manifest: """
                <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
                <item id="c1" href="a&amp;b.xhtml" media-type="application/xhtml+xml"/>
                """, spine: #"<itemref idref="c1"/>"#, spineAttrs: #"toc="ncx""#),
            files: [
                "toc.ncx": ncx,
                "a&b.xhtml": page(#"<p>Vorspann.</p><p id="s">Es war einmal.</p>"#),
            ])
        XCTAssertEqual(try BookText.epub(epub).text, "Vorspann.\n\n## Der Anfang\n\nEs war einmal.")
    }
}
