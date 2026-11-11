import Foundation

// Volltextsuche im offenen Buch (lva#476), wie `bookSearch.ts` am Desktop:
// gefaltet wie die Ablage (klein, ohne Diakritika, `AblageSuchtext.ftsFaltung`:
// ß → ss, æ → ae, ø → o …), Leerraum als ein Leerzeichen. Gesucht wird je
// Absatz; jeder Treffer trägt Absatz und Bereich im Absatztext (UTF-16, wie
// `ReadingMark`) und einen kurzen Ausschnitt für die Trefferliste.

public struct BookSearchHit: Equatable, Sendable {
    /// Index in den Absätzen (`LibraryText.paragraphs`).
    public var paragraph: Int
    /// Bereich in `TextParagraph.text`, UTF-16.
    public var range: NSRange
    /// Ausschnitt: Text davor, Treffer, Text danach.
    public var before: String
    public var match: String
    public var after: String
}

public struct BookSearchResult: Equatable, Sendable {
    public var hits: [BookSearchHit]
    /// Alle Treffer, auch die über die Grenze hinaus.
    public var total: Int
    /// Keine Wortfolge gefunden: Absätze, in denen alle Wörter stehen.
    public var allWords: Bool
    public static let empty = BookSearchResult(hits: [], total: 0, allWords: false)
}

/// Gefalteter Text eines Buchs, einmal gebaut, dann beliebig oft durchsucht.
public struct BookSearchIndex: Sendable {
    struct Part: Sendable {
        /// Gefalteter Absatz (UTF-16-Einheiten, damit Indizes billig sind).
        var folded: [UInt16]
        /// Je Einheit in `folded`: Beginn und Ende des Zeichens im Absatztext (UTF-16).
        var from: [Int32]
        var to: [Int32]
        var text: String
    }
    let parts: [Part]

    public init(paragraphs: [TextParagraph]) {
        parts = Self.build(paragraphs, isCancelled: { false }) ?? []
    }

    /// Wie `init(paragraphs:)`, bricht aber ab (`nil`), sobald `isCancelled` wahr wird:
    /// je Absatz einmal geprüft, damit ein abgebrochener Aufbau nicht weiterläuft.
    public init?(paragraphs: [TextParagraph], isCancelled: @Sendable () -> Bool) {
        guard let built = Self.build(paragraphs, isCancelled: isCancelled) else { return nil }
        parts = built
    }

    private static func build(_ paragraphs: [TextParagraph], isCancelled: @Sendable () -> Bool) -> [Part]? {
        var cache: [Character: [UInt16]] = [:]
        var out: [Part] = []
        out.reserveCapacity(paragraphs.count)
        for p in paragraphs {
            if isCancelled() { return nil }
            var folded: [UInt16] = []
            var from: [Int32] = []
            var to: [Int32] = []
            folded.reserveCapacity(p.text.utf16.count)
            var offset = 0
            var space = true
            var seen = 0
            for c in p.text {
                // Auch innerhalb großer Absätze regelmäßig auf Abbruch prüfen.
                seen += 1
                if seen & 4095 == 0, isCancelled() { return nil }
                let n = c.utf16.count
                defer { offset += n }
                if c.isWhitespace {
                    if !space { folded.append(32); from.append(Int32(offset)); to.append(Int32(offset + n)) }
                    space = true
                    continue
                }
                let f: [UInt16]
                if let hit = cache[c] { f = hit } else { f = Array(BookSearch.fold(c).utf16); cache[c] = f }
                if f.isEmpty { continue }
                for u in f { folded.append(u); from.append(Int32(offset)); to.append(Int32(offset + n)) }
                space = false
            }
            if folded.last == 32 { folded.removeLast(); from.removeLast(); to.removeLast() }
            out.append(Part(folded: folded, from: from, to: to, text: p.text))
        }
        return out
    }
}

public enum BookSearch {
    /// Kürzeste Anfrage (gefaltet), ab der gesucht wird.
    public static let minQuery = 2
    static let context = 48

    /// Unsichtbare Zeichen, die beim Suchen nicht zählen.
    static let invisible: Set<Unicode.Scalar> = ["\u{00AD}", "\u{200B}", "\u{200C}", "\u{200D}", "\u{2060}", "\u{FEFF}"]
    static let table: [Unicode.Scalar: String] = Dictionary(
        AblageSuchtext.ftsFaltung.map { ($0.von, $0.nach) }, uniquingKeysWith: { a, _ in a })

    /// Ein Zeichen falten; das Ergebnis kann leer oder länger sein.
    public static func fold(_ c: Character) -> String {
        var out = ""
        for s in c.unicodeScalars {
            if invisible.contains(s) { continue }
            if let r = table[s] { out += r; continue }
            out.unicodeScalars.append(s)
        }
        guard !out.isEmpty else { return "" }
        let plain = out.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de"))
        // Nach dem Falten noch einmal: „Ø“ wird erst klein zu „ø“.
        return String(plain.unicodeScalars.reduce(into: "") { acc, s in
            if let r = table[s] { acc += r } else if s.properties.generalCategory != .nonspacingMark { acc.unicodeScalars.append(s) }
        }).lowercased()
    }

    /// Text falten wie den Index (für Anfragen und Tests).
    public static func fold(text: String) -> String {
        var out = ""
        var space = true
        for c in text {
            if c.isWhitespace { if !space { out += " " }; space = true; continue }
            let f = fold(c)
            if f.isEmpty { continue }
            out += f
            space = false
        }
        return out.hasSuffix(" ") ? String(out.dropLast()) : out
    }

    /// Zuerst die Anfrage als Wortfolge; findet die nichts und hat sie mehrere
    /// Wörter, zählt ein Absatz, in dem alle Wörter stehen (markiert wird das erste).
    public static func search(_ index: BookSearchIndex, query: String, limit: Int = 300) -> BookSearchResult {
        run(index, query: query, limit: limit, isCancelled: { false }) ?? .empty
    }

    /// Wie `search`, bricht aber ab (`nil`), sobald `isCancelled` wahr wird (je 64 Absätze geprüft).
    public static func search(_ index: BookSearchIndex, query: String, limit: Int, isCancelled: @Sendable () -> Bool) -> BookSearchResult? {
        run(index, query: query, limit: limit, isCancelled: isCancelled)
    }

    private static func run(_ index: BookSearchIndex, query: String, limit: Int, isCancelled: @Sendable () -> Bool) -> BookSearchResult? {
        let needle = Array(fold(text: query).utf16)
        guard needle.count >= minQuery else { return .empty }
        var hits: [BookSearchHit] = []
        var total = 0
        for (pi, part) in index.parts.enumerated() {
            if pi & 63 == 0, isCancelled() { return nil }
            var at = 0
            while let i = find(needle, in: part.folded, from: at) {
                total += 1
                if hits.count < limit { hits.append(hit(part, paragraph: pi, from: i, to: i + needle.count)) }
                at = i + needle.count
            }
        }
        if total > 0 { return BookSearchResult(hits: hits, total: total, allWords: false) }

        var seen = Set<[UInt16]>()
        let words = needle.split(separator: 32).map(Array.init).filter { $0.count >= minQuery && seen.insert($0).inserted }
        guard words.count >= 2 else { return .empty }
        for (pi, part) in index.parts.enumerated() {
            if pi & 63 == 0, isCancelled() { return nil }
            var first: (at: Int, len: Int)?
            var all = true
            for w in words {
                guard let i = find(w, in: part.folded, from: 0) else { all = false; break }
                if first == nil || i < first!.at { first = (i, w.count) }
            }
            guard all, let f = first else { continue }
            total += 1
            if hits.count < limit { hits.append(hit(part, paragraph: pi, from: f.at, to: f.at + f.len)) }
        }
        return BookSearchResult(hits: hits, total: total, allWords: total > 0)
    }

    static func find(_ needle: [UInt16], in hay: [UInt16], from: Int) -> Int? {
        guard let first = needle.first, needle.count <= hay.count else { return nil }
        var i = from
        let last = hay.count - needle.count
        while i <= last {
            if hay[i] == first {
                var k = 1
                while k < needle.count && hay[i + k] == needle[k] { k += 1 }
                if k == needle.count { return i }
            }
            i += 1
        }
        return nil
    }

    static func hit(_ part: BookSearchIndex.Part, paragraph: Int, from: Int, to: Int) -> BookSearchHit {
        let a = Int(part.from[from]), b = Int(part.to[to - 1])
        let ns = part.text as NSString
        var lo = max(0, a - context), hi = min(ns.length, b + context)
        // Ausschnitt an Wortgrenzen und nie mitten in einem Zeichen.
        if lo > 0 {
            let sp = ns.range(of: " ", range: NSRange(location: lo, length: a - lo))
            if sp.location != NSNotFound { lo = sp.location + 1 }
            lo = ns.rangeOfComposedCharacterSequence(at: lo).location
        }
        if hi < ns.length {
            let sp = ns.range(of: " ", options: .backwards, range: NSRange(location: b, length: hi - b))
            if sp.location != NSNotFound { hi = sp.location } else { hi = NSMaxRange(ns.rangeOfComposedCharacterSequence(at: hi - 1)) }
        }
        let before = ns.substring(with: NSRange(location: lo, length: a - lo))
        let after = ns.substring(with: NSRange(location: b, length: hi - b))
        return BookSearchHit(
            paragraph: paragraph, range: NSRange(location: a, length: b - a),
            before: (lo > 0 ? "…" : "") + before.drop(while: \.isWhitespace),
            match: ns.substring(with: NSRange(location: a, length: b - a)),
            after: String(after.reversed().drop(while: \.isWhitespace).reversed()) + (hi < ns.length ? "…" : ""))
    }
}

/// Inhaltsverzeichnis eines Textbuchs aus seinen Überschriften (`# …`).
public struct BookContentsEntry: Equatable, Sendable {
    public var paragraph: Int
    public var title: String
}

public enum BookContents {
    public static func entries(_ paragraphs: [TextParagraph]) -> [BookContentsEntry] {
        paragraphs.enumerated().compactMap { i, p in
            guard p.heading else { return nil }
            let title = p.text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
            return title.isEmpty ? nil : BookContentsEntry(paragraph: i, title: title)
        }
    }

    /// Kapitel, in dem ein Absatz steht (`nil`: vor der ersten Überschrift).
    public static func chapter(of paragraph: Int, in entries: [BookContentsEntry]) -> Int? {
        var found: Int?
        for (i, e) in entries.enumerated() { if e.paragraph <= paragraph { found = i } else { break } }
        return found
    }
}
