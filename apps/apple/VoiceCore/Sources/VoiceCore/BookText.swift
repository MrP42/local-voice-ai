import Foundation
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
        for attrs in XMLScan.attributes(in: opf, element: "item") { if let id = attrs["id"], let href = attrs["href"] { manifest[id] = href } }
        var seen = Set<String>()
        let spine = XMLScan.attributes(in: opf, element: "itemref").compactMap { $0["idref"] }.filter { seen.insert($0).inserted }
        var parts: [String] = []
        for id in spine {
            guard let href = manifest[id]?.removingPercentEncoding ?? manifest[id] else { continue }
            let path = dir.isEmpty ? href : dir + "/" + href
            guard let xhtml = try? zip.read(normalize(path)) else { continue }
            let text = XHTMLText.paragraphs(xhtml)
            if !text.isEmpty { parts.append(text) }
        }
        let text = parts.joined(separator: "\n\n")
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw BookTextError.empty }
        let meta = XMLScan.texts(in: opf, elements: ["dc:title", "dc:creator", "dc:language"])
        return ExtractedBook(title: meta["dc:title"]?.first, authors: meta["dc:creator"] ?? [], language: meta["dc:language"]?.first, text: text)
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
            if XHTMLText.headings.contains(n) { flush(); heading = true }
            else if XHTMLText.blocks.contains(n) { flush() }
            else if n == "br" { buffer += " " }
        }
        func parser(_ p: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName q: String?) {
            let n = local(name)
            if XHTMLText.skip.contains(n) { skipDepth = max(0, skipDepth - 1); return }
            if XHTMLText.headings.contains(n) || XHTMLText.blocks.contains(n) { flush() }
        }
        func parser(_ p: XMLParser, foundCharacters s: String) { if skipDepth == 0 { buffer += s } }
    }

    static func paragraphs(_ data: Data) -> String {
        // XHTML kennt benannte HTML-Entitäten nicht; die häufigsten vorher ersetzen.
        var source = String(decoding: data, as: UTF8.self)
        // XHTML kennt nur die fünf XML-Entitäten; HTML-Namen in Zeichen wandeln,
        // unbekannte entfernen, damit der Parser das Kapitel nicht verwirft.
        source = replaceEntities(source, keepXML: true)
        source = source.replacingOccurrences(of: #"(?i)<(br|hr|img|meta|link|input|wbr)\b([^>]*?)\s*/?>"#, with: "<$1$2/>", options: .regularExpression)
        let w = Walker(); let p = XMLParser(data: Data(source.utf8)); p.delegate = w
        if p.parse() { w.flush(); return w.paragraphs.joined(separator: "\n\n") }
        let stripped = source.replacingOccurrences(of: #"(?is)<(script|style|head)[^>]*>.*?</\1>"#, with: "", options: .regularExpression)
            .replacingOccurrences(of: #"(?i)<h[1-6][^>]*>"#, with: "\n\n# ", options: .regularExpression)
            .replacingOccurrences(of: #"(?i)</(p|div|h[1-6]|li|blockquote|section)>|<(p|div|li|blockquote|section)\b[^>]*>"#, with: "\n\n", options: .regularExpression)
            .replacingOccurrences(of: #"<[^>]+>"#, with: "", options: .regularExpression)
        return replaceEntities(stripped, keepXML: false).components(separatedBy: "\n\n").map {
            $0.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression).trimmingCharacters(in: .whitespaces)
        }.filter { !$0.isEmpty && $0 != "#" }.joined(separator: "\n\n")
    }
}
