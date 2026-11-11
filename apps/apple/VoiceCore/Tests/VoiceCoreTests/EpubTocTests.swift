import XCTest
@testable import VoiceCore

/// Kapitel aus dem EPUB-Inhaltsverzeichnis (lva#476): dieselben Fälle wie die Tests in `ebook.rs`.
final class EpubTocTests: XCTestCase {
    func testNavAndNcx() {
        let nav = """
        <html><body><nav epub:type="toc"><ol>
        <li><a href="Text/k1.xhtml">Teil 1</a><ol><li><a href="Text/k1.xhtml#a2">Der Brief</a></li></ol></li>
        <li><a href="Text/k%202.xhtml">Teil <em>2</em></a></li></ol></nav>
        <nav epub:type="landmarks"><ol><li><a href="Text/k1.xhtml">Start</a></li></ol></nav></body></html>
        """
        XCTAssertEqual(EpubToc.nav(nav, baseDir: "OEBPS"), [
            .init(depth: 0, title: "Teil 1", path: "OEBPS/Text/k1.xhtml", fragment: nil),
            .init(depth: 1, title: "Der Brief", path: "OEBPS/Text/k1.xhtml", fragment: "a2"),
            .init(depth: 0, title: "Teil 2", path: "OEBPS/Text/k 2.xhtml", fragment: nil),
        ])
        let ncx = """
        <ncx><navMap><navPoint id="n1"><navLabel><text>Erstes Kapitel</text></navLabel>
        <content src="c1.xhtml"/><navPoint id="n2"><navLabel><text>Ein Abschnitt</text></navLabel>
        <content src="c1.xhtml#s"/></navPoint></navPoint></navMap></ncx>
        """
        let toc = EpubToc.ncx(ncx, baseDir: "")
        XCTAssertEqual(toc.count, 2)
        XCTAssertEqual(toc[1].depth, 1)
        XCTAssertEqual(toc[1].title, "Ein Abschnitt")
    }

    func testTitlesMarkParagraphs() {
        let e1 = EpubToc.Entry(depth: 0, title: "Der Brief", path: "x", fragment: "a")
        let e2 = EpubToc.Entry(depth: 1, title: "Am Bahnhof", path: "x", fragment: "b")
        let md = "Vorbemerkung zum Brief.\n\n**Der Brief**\n\nText.\n\nAm Bahnhof\n\nMehr Text."
        XCTAssertEqual(EpubToc.apply(md, entries: [e1, e2]),
                       "Vorbemerkung zum Brief.\n\n## Der Brief\n\nText.\n\n### Am Bahnhof\n\nMehr Text.")
        var whole = e1; whole.fragment = nil
        XCTAssertEqual(EpubToc.apply("Text ohne Kopf.", entries: [whole]), "## Der Brief\n\nText ohne Kopf.")
        XCTAssertEqual(EpubToc.apply("# Kopf\n\nText.", entries: [whole]), "# Kopf\n\nText.")
    }

    func testChapterLines() {
        let md = "Effi Briest\n\nErstes Kapitel\n\nIn Front des Herrenhauses fiel heller Sonnenschein.\n\n"
            + "**Zweites Kapitel.**\n\nDer Brief\n\nEs war ein Sonntag.\n\nKapitel 3 zeigt, wie es weiterging."
        let out = EpubToc.promoteChapterLines(md)
        XCTAssertTrue(out.contains("\n\n## Erstes Kapitel\n\n"), out)
        XCTAssertTrue(out.contains("\n\n## Zweites Kapitel: Der Brief\n\n"), out)
        XCTAssertTrue(out.contains("Kapitel 3 zeigt"), out)
        XCTAssertTrue(out.hasPrefix("Effi Briest"), out)
        XCTAssertTrue(EpubToc.promoteChapterLines("Vorwort\n\nLiebe Leserinnen und Leser!\n\nDieses Buch …")
            .hasPrefix("## Vorwort\n\nLiebe Leserinnen"))
        XCTAssertTrue(EpubToc.promoteChapterLines("Erstes Kapitel\n\n„Komm her!“\n\nSie kam.")
            .hasPrefix("## Erstes Kapitel\n\n„Komm her!“"))
    }
}
