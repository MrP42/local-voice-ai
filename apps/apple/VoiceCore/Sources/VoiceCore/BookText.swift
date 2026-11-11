import Foundation
#if canImport(FoundationXML)
import FoundationXML
#endif
#if canImport(Compression)
import Compression
#endif

// Text aus Buchdateien für die Bibliothek (Epic #210): EPUB ohne fremde
// Bibliothek (ZIP + Deflate über das System-Framework Compression), TXT und
// Markdown direkt. Das Ergebnis hat dieselbe Form wie am Desktop: Absätze durch
// Leerzeilen getrennt, Überschriften mit „# “.

public enum BookTextError: Error, Equatable {
    case notZip, unsupportedCompression(UInt16), corrupt(String), missing(String), empty
}

/// Minimaler Leser für ZIP-Archive (gespeichert oder Deflate, ohne Zip64).
public struct ZipArchive {
    public struct Entry { public let name: String; let method: UInt16; let compressed: Int; let size: Int; let offset: Int }
    private let data: Data
    public let entries: [String: Entry]
    /// Schutz gegen Zip-Bomben: keine einzelne Datei über 64 MiB entpackt.
    public static let maxEntrySize = 64 * 1024 * 1024
    /// Gesamtbudget je Archiv: viele Einträge auf dieselben Daten (Zip-Bombe)
    /// können zusammen nicht mehr als 256 MiB entpacken.
    public static let maxTotalSize = 256 * 1024 * 1024
    private var inflated = 0

    public init(data: Data) throws {
        self.data = data
        let bytes = [UInt8](data)
        guard bytes.count >= 22 else { throw BookTextError.notZip }
        var eocd = -1
        var i = bytes.count - 22
        let lower = max(0, bytes.count - 22 - 65_535)
        while i >= lower { if bytes[i] == 0x50, bytes[i+1] == 0x4b, bytes[i+2] == 0x05, bytes[i+3] == 0x06 { eocd = i; break }; i -= 1 }
        guard eocd >= 0 else { throw BookTextError.notZip }
        func u16(_ o: Int) -> Int { Int(bytes[o]) | Int(bytes[o+1]) << 8 }
        func u32(_ o: Int) -> Int { u16(o) | u16(o+2) << 16 }
        let count = u16(eocd + 10)
        var p = u32(eocd + 16)
        var out: [String: Entry] = [:]
        for _ in 0..<count {
            guard p + 46 <= bytes.count, u32(p) == 0x02014b50 else { throw BookTextError.corrupt("Verzeichnis") }
            let nameLen = u16(p + 28), extraLen = u16(p + 30), commentLen = u16(p + 32)
            guard p + 46 + nameLen <= bytes.count else { throw BookTextError.corrupt("Name") }
            let name = String(decoding: bytes[(p + 46)..<(p + 46 + nameLen)], as: UTF8.self)
            out[name] = Entry(name: name, method: UInt16(u16(p + 10)), compressed: u32(p + 20), size: u32(p + 24), offset: u32(p + 42))
            p += 46 + nameLen + extraLen + commentLen
        }
        entries = out
    }

    public mutating func read(_ name: String) throws -> Data {
        guard let e = entries[name] else { throw BookTextError.missing(name) }
        guard e.size <= Self.maxEntrySize, inflated + e.size <= Self.maxTotalSize else { throw BookTextError.corrupt("zu groß: \(name)") }
        inflated += e.size
        let bytes = data
        let base = bytes.startIndex
        func u16(_ o: Int) -> Int { Int(bytes[base + o]) | Int(bytes[base + o + 1]) << 8 }
        guard e.offset + 30 <= bytes.count, u16(e.offset) == 0x4b50, u16(e.offset + 2) == 0x0403 else { throw BookTextError.corrupt(name) }
        let start = e.offset + 30 + u16(e.offset + 26) + u16(e.offset + 28)
        guard start + e.compressed <= bytes.count else { throw BookTextError.corrupt(name) }
        let raw = bytes.subdata(in: (base + start)..<(base + start + e.compressed))
        switch e.method {
        case 0: return raw
        case 8: return try Self.inflate(raw, size: e.size)
        default: throw BookTextError.unsupportedCompression(e.method)
        }
    }

    static func inflate(_ raw: Data, size: Int) throws -> Data {
        #if canImport(Compression)
        if size == 0 { return Data() }
        var out = Data(count: size)
        let written = out.withUnsafeMutableBytes { dst in
            raw.withUnsafeBytes { src in
                compression_decode_buffer(dst.bindMemory(to: UInt8.self).baseAddress!, size,
                                          src.bindMemory(to: UInt8.self).baseAddress!, raw.count, nil, COMPRESSION_ZLIB)
            }
        }
        guard written == size else { throw BookTextError.corrupt("Deflate") }
        return out
        #else
        throw BookTextError.unsupportedCompression(8)
        #endif
    }
}

public struct ExtractedBook: Equatable, Sendable {
    public var title: String?
    public var authors: [String]
    public var language: String?
    public var text: String
    /// Coverbild aus dem EPUB (Rohdaten, JPEG/PNG/WebP), falls vorhanden.
    public var cover: Data? = nil
}

public enum BookText {
    /// Text aus TXT oder Markdown; Zeilenenden vereinheitlicht.
    public static func plain(_ data: Data) throws -> String {
        let text = String(decoding: data, as: UTF8.self).replacingOccurrences(of: "\r\n", with: "\n")
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw BookTextError.empty }
        return text
    }

    public static func epub(_ data: Data) throws -> ExtractedBook {
        var zip = try ZipArchive(data: data)
        let container = try zip.read("META-INF/container.xml")
        guard let opfPath = XMLScan.attributes(in: container, element: "rootfile").first?["full-path"] else { throw BookTextError.missing("OPF") }
        let opf = try zip.read(opfPath)
        let dir = (opfPath as NSString).deletingLastPathComponent
        var manifest: [String: String] = [:]
        let items = XMLScan.attributes(in: opf, element: "item")
        for attrs in items { if let id = attrs["id"], let href = attrs["href"] { manifest[id] = href } }
        var seen = Set<String>()
        let spine = XMLScan.attributes(in: opf, element: "itemref").compactMap { $0["idref"] }.filter { seen.insert($0).inserted }
        // Inhaltsverzeichnis (lva#476): EPUB-3-nav, sonst die NCX aus `<spine toc>`.
        func resolved(_ href: String) -> String {
            let decoded = href.removingPercentEncoding ?? href
            return normalize(dir.isEmpty ? decoded : dir + "/" + decoded)
        }
        func folder(_ path: String) -> String { (path as NSString).deletingLastPathComponent }
        let navPath = items.first { ($0["properties"] ?? "").split(separator: " ").contains("nav") }?["href"].map(resolved)
        var toc: [EpubToc.Entry] = []
        if let navPath, let data = try? zip.read(navPath) {
            toc = EpubToc.nav(String(decoding: data, as: UTF8.self), baseDir: folder(navPath), source: navPath)
        }
        if toc.isEmpty, let ncxId = XMLScan.attributes(in: opf, element: "spine").first?["toc"],
           let href = manifest[ncxId], let data = try? zip.read(resolved(href)) {
            toc = EpubToc.ncx(String(decoding: data, as: UTF8.self), baseDir: folder(resolved(href)), source: resolved(href))
        }
        var parts: [String] = []
        for id in spine {
            guard let href = manifest[id] else { continue }
            let path = resolved(href)
            guard let xhtml = try? zip.read(path) else { continue }
            let entries = toc.filter { $0.path.lowercased() == path.lowercased() }
            // Das EPUB-3-Inhaltsverzeichnis ist kein Lesetext; steht das nav-Dokument im Spine und hat
            // auch Inhalt, bleibt dieser erhalten (nur die `<nav>`-Blöcke fallen weg, gleiche Tokenisierung
            // wie bei jedem anderen Kapitel).
            let chapter = XHTMLText.extract(xhtml, skipNav: path == navPath)
            let text = EpubToc.promoteChapterLines(EpubToc.apply(chapter.text, entries: entries, anchors: chapter.anchors))
            if !text.isEmpty { parts.append(text) }
        }
        let text = parts.joined(separator: "\n\n")
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw BookTextError.empty }
        let meta = XMLScan.texts(in: opf, elements: ["dc:title", "dc:creator", "dc:language"])
        var book = ExtractedBook(title: meta["dc:title"]?.first, authors: meta["dc:creator"] ?? [], language: meta["dc:language"]?.first, text: text)
        if let href = coverHref(items: items, metas: XMLScan.attributes(in: opf, element: "meta")) {
            let decoded = href.removingPercentEncoding ?? href
            let path = normalize(dir.isEmpty ? decoded : dir + "/" + decoded)
            // Ein defektes Cover kostet nie das Buch.
            if let entry = zip.entries[path], entry.size <= maxCoverBytes, let data = try? zip.read(path), !data.isEmpty { book.cover = data }
        }
        return book
    }

    /// Größtes zulässiges Coverbild im EPUB.
    public static let maxCoverBytes = 20 * 1024 * 1024

    /// Cover wie am Desktop suchen: EPUB 3 `properties="cover-image"`, dann
    /// EPUB 2 `<meta name="cover" content="id">`, dann ein Bild mit „cover“ im Namen.
    static func coverHref(items: [[String: String]], metas: [[String: String]]) -> String? {
        func isImage(_ a: [String: String]) -> Bool { (a["media-type"] ?? "").lowercased().hasPrefix("image/") }
        if let a = items.first(where: { ($0["properties"] ?? "").split(separator: " ").contains("cover-image") }), let h = a["href"] { return h }
        if let id = metas.first(where: { $0["name"]?.lowercased() == "cover" })?["content"],
           let a = items.first(where: { $0["id"] == id && isImage($0) }), let h = a["href"] { return h }
        return items.first(where: { isImage($0) && (($0["id"] ?? "") + " " + ($0["href"] ?? "")).lowercased().contains("cover") })?["href"]
    }

    static func normalize(_ path: String) -> String {
        var out: [String] = []
        for part in path.split(separator: "/", omittingEmptySubsequences: true) {
            if part == ".." { _ = out.popLast() } else if part != "." { out.append(String(part)) }
        }
        return out.joined(separator: "/")
    }
}

/// Kleine XML-Hilfen auf XMLParser (Foundation).
enum XMLScan {
    final class Collector: NSObject, XMLParserDelegate {
        let wanted: Set<String>; var attrs: [[String: String]] = []
        var texts: [String: [String]] = [:]; var current: String?; var buffer = ""
        let collectText: Bool
        init(_ wanted: Set<String>, text: Bool) { self.wanted = wanted; collectText = text }
        func parser(_ p: XMLParser, didStartElement name: String, namespaceURI: String?, qualifiedName q: String?, attributes a: [String: String] = [:]) {
            let key = q ?? name
            guard wanted.contains(key) || wanted.contains(name) else { return }
            if collectText { current = key; buffer = "" } else { attrs.append(a) }
        }
        func parser(_ p: XMLParser, foundCharacters s: String) { if current != nil { buffer += s } }
        func parser(_ p: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName q: String?) {
            guard let c = current, c == (q ?? name) else { return }
            let t = buffer.trimmingCharacters(in: .whitespacesAndNewlines)
            if !t.isEmpty { texts[c, default: []].append(t) }
            current = nil
        }
    }
    static func attributes(in data: Data, element: String) -> [[String: String]] {
        let c = Collector([element], text: false); let p = XMLParser(data: data); p.delegate = c; p.parse(); return c.attrs
    }
    static func texts(in data: Data, elements: [String]) -> [String: [String]] {
        let c = Collector(Set(elements), text: true); let p = XMLParser(data: data); p.delegate = c; p.parse(); return c.texts
    }
}

/// XHTML eines EPUB-Kapitels → Absätze. Fällt bei kaputtem XML auf eine
/// Tag-Entfernung zurück, damit ein fehlerhaftes Kapitel das Buch nicht verliert.
enum XHTMLText {
    static let named: [String: String] = [
        "nbsp": "\u{00A0}", "shy": "", "mdash": "—", "ndash": "–", "hellip": "…", "laquo": "«", "raquo": "»", "bdquo": "„",
        "ldquo": "“", "rdquo": "”", "lsquo": "‘", "rsquo": "’", "sbquo": "‚", "lsaquo": "‹", "rsaquo": "›",
        "auml": "ä", "ouml": "ö", "uuml": "ü", "Auml": "Ä", "Ouml": "Ö", "Uuml": "Ü", "szlig": "ß",
        "eacute": "é", "egrave": "è", "ecirc": "ê", "aacute": "á", "agrave": "à", "acirc": "â", "ccedil": "ç",
        "iacute": "í", "oacute": "ó", "uacute": "ú", "ntilde": "ñ", "Eacute": "É", "oslash": "ø", "aring": "å", "aelig": "æ",
        "copy": "©", "reg": "®", "trade": "™", "euro": "€", "pound": "£", "deg": "°", "middot": "·", "bull": "•",
        "times": "×", "divide": "÷", "sect": "§", "para": "¶", "dagger": "†", "prime": "′", "thinsp": "\u{2009}",
        "ensp": "\u{2002}", "emsp": "\u{2003}", "zwnj": "", "zwj": "", "frac12": "½", "frac14": "¼", "frac34": "¾",
    ]

    /// Benannte und numerische Entitäten ersetzen. `keepXML` lässt die fünf
    /// XML-Entitäten (und Zahlen) für den Parser stehen.
    static func replaceEntities(_ s: String, keepXML: Bool) -> String {
        guard s.contains("&") else { return s }
        let re = try! NSRegularExpression(pattern: #"&(#x[0-9A-Fa-f]{1,6}|#[0-9]{1,7}|[A-Za-z][A-Za-z0-9]{1,31});"#)
        let ns = s as NSString
        var out = ""; var last = 0
        for m in re.matches(in: s, range: NSRange(location: 0, length: ns.length)) {
            out += ns.substring(with: NSRange(location: last, length: m.range.location - last))
            let name = ns.substring(with: m.range(at: 1))
            let whole = ns.substring(with: m.range)
            if name.hasPrefix("#") {
                let hex = name.hasPrefix("#x") || name.hasPrefix("#X")
                let value = UInt32(name.dropFirst(hex ? 2 : 1), radix: hex ? 16 : 10)
                out += keepXML ? whole : (value.flatMap(Unicode.Scalar.init).map { String($0) } ?? "")
            } else if ["amp", "lt", "gt", "quot", "apos"].contains(name) {
                out += keepXML ? whole : ["amp": "&", "lt": "<", "gt": ">", "quot": "\"", "apos": "'"][name]!
            } else {
                out += named[name] ?? ""
            }
            last = m.range.location + m.range.length
        }
        return out + ns.substring(from: last)
    }

    static let blocks: Set<String> = ["p", "div", "li", "blockquote", "section", "article", "tr", "dd", "dt", "figcaption", "pre"]
    static let headings: Set<String> = ["h1", "h2", "h3", "h4", "h5", "h6"]
    static let skip: Set<String> = ["script", "style", "head", "title"]

    final class Walker: NSObject, XMLParserDelegate {
        var paragraphs: [String] = []; var buffer = ""; var heading = false; var skipDepth = 0
        /// Inhalte innerhalb von `<nav>` überspringen (nur beim nav-Dokument); `navDepth` zählt Verschachtelung.
        let skipNav: Bool; var navDepth = 0
        init(skipNav: Bool) { self.skipNav = skipNav }
        /// `id` (und `<a name>`) → Index des Absatzes, in dem das Element liegt bzw. der danach folgt.
        var anchors: [String: Int] = [:]
        func local(_ n: String) -> String { (n.split(separator: ":").last.map(String.init) ?? n).lowercased() }
        func flush() {
            let t = buffer.replacingOccurrences(of: #"[\s\u00A0]+"#, with: " ", options: .regularExpression)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if !t.isEmpty { paragraphs.append(heading ? "# " + t : t) }
            buffer = ""; heading = false
        }
        func parser(_ p: XMLParser, didStartElement name: String, namespaceURI: String?, qualifiedName q: String?, attributes a: [String: String] = [:]) {
            let n = local(name)
            if XHTMLText.skip.contains(n) { skipDepth += 1; return }
            if skipDepth > 0 { return }
            if skipNav, n == "nav" { if navDepth == 0 { flush() }; navDepth += 1; return }
            if navDepth > 0 { return }
            if XHTMLText.headings.contains(n) { flush(); heading = true }
            else if XHTMLText.blocks.contains(n) { flush() }
            else if n == "br" { buffer += " " }
            for key in ["id", "xml:id"] { if let id = a[key], !id.isEmpty, anchors[id] == nil { anchors[id] = paragraphs.count } }
            if n == "a", let id = a["name"], !id.isEmpty, anchors[id] == nil { anchors[id] = paragraphs.count }
        }
        func parser(_ p: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName q: String?) {
            let n = local(name)
            if XHTMLText.skip.contains(n) { skipDepth = max(0, skipDepth - 1); return }
            if skipDepth > 0 { return }
            if skipNav, n == "nav" { navDepth = max(0, navDepth - 1); return }
            if navDepth > 0 { return }
            if XHTMLText.headings.contains(n) || XHTMLText.blocks.contains(n) { flush() }
        }
        func parser(_ p: XMLParser, foundCDATA block: Data) { if skipDepth == 0, navDepth == 0 { buffer += String(decoding: block, as: UTF8.self) } }
        func parser(_ p: XMLParser, foundCharacters s: String) { if skipDepth == 0, navDepth == 0 { buffer += s } }
    }

    static func paragraphs(_ data: Data) -> String { extract(data).text }

    /// Absätze des Kapitels und Sprungmarken auf Absatzindizes; `anchors` ist `nil`, wenn das Kapitel
    /// nur über die Tag-Entfernung gelesen werden konnte.
    static func extract(_ data: Data, skipNav: Bool = false) -> (text: String, anchors: [String: Int]?) {
        // XHTML kennt benannte HTML-Entitäten nicht; die häufigsten vorher ersetzen.
        var source = String(decoding: data, as: UTF8.self)
        // XHTML kennt nur die fünf XML-Entitäten; HTML-Namen in Zeichen wandeln,
        // unbekannte entfernen, damit der Parser das Kapitel nicht verwirft.
        source = replaceEntities(source, keepXML: true)
        source = source.replacingOccurrences(of: #"(?i)<(br|hr|img|meta|link|input|wbr)\b([^>]*?)\s*/?>"#, with: "<$1$2/>", options: .regularExpression)
        let w = Walker(skipNav: skipNav); let p = XMLParser(data: Data(source.utf8)); p.delegate = w
        // Unter Linux liefert `parse()` auch bei Wohlgeformtheitsfehlern `true`; `parserError` entscheidet.
        if p.parse(), p.parserError == nil {
            w.flush()
            let count = w.paragraphs.count
            return (w.paragraphs.joined(separator: "\n\n"), w.anchors.filter { $0.value < count })
        }
        return (fallback(source, skipNav: skipNav), nil)
    }

    static let navTag = try! NSRegularExpression(pattern: #"(?i)</?(?:[\w.-]+:)?nav(?![\w:.-])[^>]*>"#)

    /// Inhalt der äußersten `<nav>`-Blöcke entfernen (Tiefe gezählt, Präfix und Schreibweise egal).
    static func removingNavBlocks(_ s: String) -> String {
        let ns = s as NSString
        var out = ""; var last = 0; var depth = 0
        for m in navTag.matches(in: s, range: NSRange(location: 0, length: ns.length)) {
            let tag = ns.substring(with: m.range)
            if depth == 0 { out += ns.substring(with: NSRange(location: last, length: m.range.location - last)) }
            if tag.hasPrefix("</") { depth = max(0, depth - 1) } else if !tag.hasSuffix("/>") { depth += 1 }
            last = NSMaxRange(m.range)
        }
        return depth == 0 ? out + ns.substring(from: last) : out
    }

    static let rawOpen = try! NSRegularExpression(pattern: #"(?i)<(?:[\w.-]+:)?(script|style|head)(?![\w:.-])[^>]*>"#)
    static let rawClose = ["script", "style", "head"].reduce(into: [String: NSRegularExpression]()) {
        $0[$1] = try! NSRegularExpression(pattern: "(?i)</(?:[\\w.-]+:)?\($1)\\s*>")
    }

    /// script/style/head samt Inhalt entfernen, in einem Durchgang: Anfänge und Endtags werden je einmal
    /// gesammelt, viele offene Anfänge oder Endtag-Kandidaten kosten keine quadratische Laufzeit.
    static func removingRawText(_ s: String) -> String {
        let ns = s as NSString
        let all = NSRange(location: 0, length: ns.length)
        let closes = rawClose.mapValues { $0.matches(in: s, range: all).map(\.range) }
        var next = [String: Int]()
        var out = ""; var last = 0
        for m in rawOpen.matches(in: s, range: all) where m.range.location >= last {
            let name = ns.substring(with: m.range(at: 1)).lowercased()
            let list = closes[name] ?? []
            var k = next[name] ?? 0
            while k < list.count, list[k].location < NSMaxRange(m.range) { k += 1 }
            next[name] = k
            guard k < list.count else { continue }
            out += ns.substring(with: NSRange(location: last, length: m.range.location - last))
            last = NSMaxRange(list[k])
        }
        return out + ns.substring(from: last)
    }

    /// Kommentare, Verarbeitungsanweisungen und DOCTYPE entfernen, CDATA als maskierten Text behalten;
    /// ein Durchgang, Unterminiertes verschluckt nur den Rest (der Text davor bleibt).
    static func removingMarkupDeclarations(_ s: String) -> String {
        let u = Array(s.utf16)
        let n = u.count
        var out: [UInt16] = []
        out.reserveCapacity(n)
        func has(_ lit: String, at p: Int, caseInsensitive: Bool = false) -> Bool {
            var k = p
            for w in lit.utf16 {
                guard k < n else { return false }
                var c = u[k]
                if caseInsensitive, c >= 0x61, c <= 0x7A { c -= 32 }
                if c != w { return false }
                k += 1
            }
            return true
        }
        func find(_ lit: String, from p: Int) -> Int? {
            var q = p
            while q < n { if has(lit, at: q) { return q }; q += 1 }
            return nil
        }
        var i = 0
        while i < n {
            guard u[i] == 0x3C else { out.append(u[i]); i += 1; continue }
            if has("<!--", at: i) {
                guard let e = find("-->", from: i + 4) else { break }
                i = e + 3
            } else if has("<?", at: i) {
                guard let e = find("?>", from: i + 2) else { break }
                i = e + 2
            } else if has("<![CDATA[", at: i) {
                let e = find("]]>", from: i + 9)
                for c in u[(i + 9)..<(e ?? n)] {
                    switch c {
                    case 0x26: out.append(contentsOf: Array("&amp;".utf16))
                    case 0x3C: out.append(contentsOf: Array("&lt;".utf16))
                    case 0x3E: out.append(contentsOf: Array("&gt;".utf16))
                    default: out.append(c)
                    }
                }
                i = (e ?? n - 3) + 3
            } else if has("<!DOCTYPE", at: i, caseInsensitive: true) {
                var q = i + 9
                var quote: UInt16?
                var square = 0
                var end: Int?
                while q < n, end == nil {
                    let c = u[q]
                    if let open = quote { if c == open { quote = nil } }
                    else if square > 0, has("<!--", at: q) { guard let e = find("-->", from: q + 4) else { q = n; break }; q = e + 2 }
                    else if square > 0, has("<?", at: q) { guard let e = find("?>", from: q + 2) else { q = n; break }; q = e + 1 }
                    else if c == 0x22 || c == 0x27 { quote = c }
                    else if c == 0x5B { square += 1 }
                    else if c == 0x5D, square > 0 { square -= 1 }
                    else if c == 0x3E, square == 0 { end = q }
                    q += 1
                }
                guard let e = end else { break }
                i = e + 1
            } else {
                out.append(u[i]); i += 1
            }
        }
        return String(decoding: out, as: UTF16.self)
    }

    /// Fallback für nicht wohlgeformtes XHTML: Tag-Entfernung mit derselben Behandlung von Kommentaren,
    /// Verarbeitungsanweisungen, CDATA (als Text), DOCTYPE, script/style/head und – beim nav-Dokument – `<nav>`.
    static func fallback(_ source: String, skipNav: Bool = false) -> String {
        var stripped = removingMarkupDeclarations(source)
        stripped = removingRawText(stripped)
        if skipNav { stripped = removingNavBlocks(stripped) }
        stripped = stripped
            .replacingOccurrences(of: #"(?i)<h[1-6][^>]*>"#, with: "\n\n# ", options: .regularExpression)
            .replacingOccurrences(of: #"(?i)</(p|div|h[1-6]|li|blockquote|section)>|<(p|div|li|blockquote|section)\b[^>]*>"#, with: "\n\n", options: .regularExpression)
            .replacingOccurrences(of: #"</?[A-Za-z][^>]*>"#, with: "", options: .regularExpression)
        return replaceEntities(stripped, keepXML: false).components(separatedBy: "\n\n").map {
            $0.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression).trimmingCharacters(in: .whitespaces)
        }.filter { !$0.isEmpty && $0 != "#" }.joined(separator: "\n\n")
    }
}

/// Kapitel aus dem Inhaltsverzeichnis eines EPUBs (lva#476, wie `ebook.rs` am Desktop): EPUB-3-`nav`,
/// sonst die NCX. Absätze, die wörtlich ein Verzeichnistitel sind, werden Überschriften; dazu Kapitelzeilen
/// wie „Erstes Kapitel“ oder „Chapter IV“. Lieber keine Überschrift als eine falsche.
enum EpubToc {
    struct Entry: Equatable {
        /// Tiefe, 0 = oberste Ebene.
        var depth: Int
        var title: String
        /// Datei im Archiv (aufgelöst) und Sprungmarke.
        var path: String
        var fragment: String?
    }

    static func re(_ p: String) -> NSRegularExpression { try! NSRegularExpression(pattern: p) }
    static let navStart = re(#"(?is)<nav\b[^>]*\btype\s*=\s*["']toc["'][^>]*>"#)
    static let navPart = re(#"(?is)</?ol\b[^>]*>|<a\b[^>]*>.*?</a\s*>"#)
    static let navLink = re(#"(?is)^<a\b([^>]*)>(.*?)</a\s*>$"#)
    static let ncxPart = re(#"(?is)</?navPoint\b[^>]*>|<text\b[^>]*>.*?</text\s*>|<content\b[^>]*>"#)
    static let attr = re(#"([A-Za-z_:][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
    static let tag = re(#"<[^>]*>"#)
    static let chapterHead = re(#"(?i)^(?:(?:kapitel|chapter|teil|part|buch|book)\s+(?:\d{1,3}|(?-i:[IVXLCDM]{1,8})|(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty)(?:[- ](?:one|two|three|four|five|six|seven|eight|nine))?)|\p{L}{3,}(?:tes|ter|te)\s+(?:kapitel|buch|teil|abschnitt|gesang))\.?$"#)
    static let specialHead = re(#"(?i)^(?:prolog|epilog|vorwort|nachwort|einleitung|prologue|epilogue|preface|foreword|afterword|introduction)$"#)

    static func matches(_ r: NSRegularExpression, _ s: String) -> Bool {
        r.firstMatch(in: s, range: NSRange(location: 0, length: (s as NSString).length)) != nil
    }

    static func attributes(_ s: String) -> [String: String] {
        let ns = s as NSString
        var out: [String: String] = [:]
        for m in attr.matches(in: s, range: NSRange(location: 0, length: ns.length)) {
            let name = ns.substring(with: m.range(at: 1)).lowercased()
            let local = name.split(separator: ":").last.map(String.init) ?? name
            let value = m.range(at: 2).location != NSNotFound ? ns.substring(with: m.range(at: 2)) : ns.substring(with: m.range(at: 3))
            // Attributwerte tragen XML-Entitäten (`&amp;` in href/src): zuerst auflösen, erst danach
            // Fragment trennen und Prozentkodierung entfernen.
            out[local] = XHTMLText.replaceEntities(value, keepXML: false)
        }
        return out
    }

    static let navBlock = re(#"(?is)<nav\b[^>]*>.*?</nav\s*>"#)

    /// Klartext eines Verzeichnistitels: ohne Tags, Entitäten aufgelöst, Leerraum zusammengefasst.
    static func plainText(_ raw: String) -> String {
        let noTags = tag.stringByReplacingMatches(in: raw, range: NSRange(location: 0, length: (raw as NSString).length), withTemplate: "")
        return XHTMLText.replaceEntities(noTags, keepXML: false).split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    /// `source`: Pfad des Verzeichnisdokuments selbst; ein reiner Fragmentlink (`#start`) zeigt auf diese Datei.
    static func push(_ out: inout [Entry], depth: Int, title: String, href: String, baseDir: String, source: String?) {
        guard !title.isEmpty, !href.isEmpty, !href.contains("://") else { return }
        let parts = href.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)
        let file = parts[0].removingPercentEncoding ?? parts[0]
        let fragment = parts.count > 1 ? (parts[1].removingPercentEncoding ?? parts[1]) : nil
        let path: String
        if file.isEmpty {
            guard let source else { return }
            path = source
        } else {
            path = BookText.normalize(baseDir.isEmpty ? file : baseDir + "/" + file)
        }
        out.append(Entry(depth: depth, title: title, path: path, fragment: fragment?.isEmpty == false ? fragment : nil))
    }

    /// Inhaltsverzeichnis aus `<nav epub:type="toc">`: Tiefe über `<ol>`, Einträge sind die Links.
    static func nav(_ xhtml: String, baseDir: String, source: String? = nil) -> [Entry] {
        let ns = xhtml as NSString
        guard let start = navStart.firstMatch(in: xhtml, range: NSRange(location: 0, length: ns.length)) else { return [] }
        var body = ns.substring(from: NSMaxRange(start.range))
        if let end = body.range(of: "</nav", options: .caseInsensitive) { body = String(body[..<end.lowerBound]) }
        let b = body as NSString
        var depth = 0
        var out: [Entry] = []
        for m in navPart.matches(in: body, range: NSRange(location: 0, length: b.length)) {
            let piece = b.substring(with: m.range)
            let lower = piece.lowercased()
            if lower.hasPrefix("<ol") { depth += 1 }
            else if lower.hasPrefix("</ol") { depth = max(0, depth - 1) }
            else if let l = navLink.firstMatch(in: piece, range: NSRange(location: 0, length: (piece as NSString).length)) {
                let p = piece as NSString
                let href = attributes(p.substring(with: l.range(at: 1)))["href"] ?? ""
                push(&out, depth: max(0, depth - 1), title: plainText(p.substring(with: l.range(at: 2))), href: href, baseDir: baseDir, source: source)
            }
        }
        return out
    }

    /// Inhaltsverzeichnis aus einer NCX (`navPoint`, verschachtelt).
    static func ncx(_ ncx: String, baseDir: String, source: String? = nil) -> [Entry] {
        let ns = ncx as NSString
        var depth = 0
        var title: String?
        var out: [Entry] = []
        for m in ncxPart.matches(in: ncx, range: NSRange(location: 0, length: ns.length)) {
            let piece = ns.substring(with: m.range)
            let lower = piece.lowercased()
            if lower.hasPrefix("<navpoint") { depth += 1; title = nil }
            else if lower.hasPrefix("</navpoint") { depth = max(0, depth - 1) }
            else if lower.hasPrefix("<text") { if title == nil { title = plainText(piece) } }
            else if lower.hasPrefix("<content"), let t = title {
                push(&out, depth: max(0, depth - 1), title: t, href: attributes(piece)["src"] ?? "", baseDir: baseDir, source: source)
                title = nil
            }
        }
        return out
    }

    /// Text eines Absatzes ohne Überschriften- und Betonungszeichen.
    static func blockPlain(_ block: String) -> String {
        var s = block.trimmingCharacters(in: .whitespacesAndNewlines)
        while s.hasPrefix("#") { s.removeFirst() }
        return s.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "*_ "))
    }

    static func titleNorm(_ s: String) -> String {
        s.lowercased().replacingOccurrences(of: "ß", with: "ss")
            .map { $0.isLetter || $0.isNumber ? String($0) : " " }.joined()
            .split(separator: " ").joined(separator: " ")
    }

    /// Ist der Absatz der ganze Titel (höchstens ein paar Zeichen mehr, etwa ein Fußnotenstern)?
    static func isTitle(_ line: String, _ title: String) -> Bool {
        let l = titleNorm(line), t = titleNorm(title)
        guard t.filter(\.isLetter).count >= 2, !l.isEmpty else { return false }
        return l == t || (l.hasPrefix(t) && l.utf8.count <= t.utf8.count + 3)
    }

    /// Überschriften eines Kapitels nach dem Inhaltsverzeichnis setzen (`##`, darunter `###`).
    /// Mit `anchors` (Sprungmarke → Absatzindex, aus `XHTMLText.extract`) wird ein Eintrag mit
    /// Sprungmarke am Zielabsatz verankert: ist dieser (oder der Absatz davor) der Titel, wird er
    /// Überschrift, sonst kommt die Überschrift vor den Zielabsatz; ein Ziel, das fehlt, wird
    /// übersprungen. Ohne `anchors` (Kapitel nur per Tag-Entfernung gelesen) und für Einträge ohne
    /// Sprungmarke wird der Titel in Reihenfolge der Einträge unter den Absätzen gesucht; ein Eintrag
    /// ohne Sprungmarke für eine Datei, die ohne Überschrift beginnt, kommt davor.
    static func apply(_ md: String, entries: [Entry], anchors: [String: Int]? = nil) -> String {
        guard !entries.isEmpty else { return md }
        var blocks = md.components(separatedBy: "\n\n")
        var cursor = 0
        var prepend: String?
        var insertions: [Int: [String]] = [:]
        func isHeading(_ i: Int) -> Bool { blocks[i].trimmingCharacters(in: .whitespaces).hasPrefix("#") }
        for e in entries {
            let level = String(repeating: "#", count: min(e.depth + 2, 3))
            if let anchors, let fragment = e.fragment {
                guard let target = anchors[fragment], blocks.indices.contains(target) else { continue }
                let titled = [target, target - 1].first { blocks.indices.contains($0) && blocks[$0].count <= 200 && isTitle(blockPlain(blocks[$0]), e.title) }
                if let t = titled {
                    if !isHeading(t) { blocks[t] = "\(level) \(blockPlain(blocks[t]))" }
                } else if !isHeading(target) {
                    let line = "\(level) \(e.title)"
                    if insertions[target]?.contains(line) != true { insertions[target, default: []].append(line) }
                }
                continue
            }
            let found = blocks.indices.dropFirst(cursor).first { blocks[$0].count <= 200 && isTitle(blockPlain(blocks[$0]), e.title) }
            if let i = found {
                if !isHeading(i) { blocks[i] = "\(level) \(blockPlain(blocks[i]))" }
                cursor = i + 1
            } else if e.fragment == nil, cursor == 0, prepend == nil {
                let first = blocks.first { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
                if first?.trimmingCharacters(in: .whitespaces).hasPrefix("#") != true { prepend = "\(level) \(e.title)" }
            }
        }
        var out: [String] = []
        for (i, b) in blocks.enumerated() { out.append(contentsOf: insertions[i] ?? []); out.append(b) }
        let text = out.joined(separator: "\n\n")
        return prepend.map { "\($0)\n\n\(text)" } ?? text
    }

    static func looksLikeChapterTitle(_ line: String) -> Bool {
        let n = line.count
        return (2...60).contains(n)
            && line.split(whereSeparator: \.isWhitespace).count <= 9
            && line.first?.isUppercase == true
            && line.filter(\.isLetter).count >= 2
            && !line.hasSuffix(".") && !line.hasSuffix(",") && !line.hasSuffix(";") && !line.hasSuffix(":")
            && !line.hasSuffix("-") && !line.hasSuffix("–")
            && !line.allSatisfy({ $0.isNumber || $0.isWhitespace })
            && !matches(chapterHead, line) && !matches(specialHead, line)
    }

    /// Kapitelzeilen als Überschrift: „Erstes Kapitel“, „Kapitel 3“, „Prolog“ … (auch fett) werden `##`;
    /// ein kurzer Titel danach gehört dazu, außer bei Vorwort & Co. und bei Ausruf oder Zitat.
    static func promoteChapterLines(_ md: String) -> String {
        let blocks = md.components(separatedBy: "\n\n")
        var out: [String] = []
        var i = 0
        while i < blocks.count {
            let b = blocks[i]
            var plain = blockPlain(b)
            while plain.hasSuffix(".") { plain.removeLast() }
            let isHead = !b.trimmingCharacters(in: .whitespaces).hasPrefix("#") && !plain.contains("\n")
                && (matches(chapterHead, plain) || matches(specialHead, plain))
            guard isHead else { out.append(b); i += 1; continue }
            if i + 1 < blocks.count, !matches(specialHead, plain) {
                let next = blocks[i + 1]
                let t = blockPlain(next)
                let ending = t.last.map { "!?…\"“”»«".contains($0) } ?? false
                if !next.trimmingCharacters(in: .whitespaces).hasPrefix("#"), !t.contains("\n"), !ending, looksLikeChapterTitle(t) {
                    out.append("## \(plain): \(t)")
                    i += 2
                    continue
                }
            }
            out.append("## \(plain)")
            i += 1
        }
        return out.joined(separator: "\n\n")
    }
}
