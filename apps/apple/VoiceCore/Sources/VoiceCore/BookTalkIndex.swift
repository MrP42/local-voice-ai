import Foundation

// Buch-Gespräch (Epic #307, Paket G4): Index über den Buchtext.
// Nur Foundation, damit der Kern ohne Darwin-Frameworks (Linux-Tests) baut.
// Alle Offsets sind UTF-16-Offsets im ORIGINALTEXT (Sprung im Reader).

/// Ein Stück Buchtext (500–900 Zeichen, nie über Kapitelgrenzen).
public struct BookTalkChunk: Sendable, Equatable {
    public let id: Int
    /// 0-basiert nach Überschriften (Text vor der ersten Überschrift zählt zu 0).
    public let chapterIndex: Int
    public let chapterTitle: String
    /// Letzte Seitenmarke `{{S. n}}` vor dem Stück; nil, wenn noch keine kam.
    public let page: Int?
    public let start: Int
    public let end: Int
    /// Gesäubert: ohne Sprecher `<Name>` und Stilmarken `[calm]`, Leerraum gefaltet.
    public let text: String
    /// Originaloffset je UTF-16-Einheit von `text` (+ ein Eintrag für das Ende).
    let offsetMap: [Int]

    public static func == (a: BookTalkChunk, b: BookTalkChunk) -> Bool {
        a.id == b.id && a.start == b.start && a.end == b.end && a.text == b.text
    }

    /// Spoilergrenze: Stück am `frontier` (UTF-16-Offset im Original) kürzen, und zwar an der
    /// Satzgrenze davor. nil, wenn nichts vor der Grenze übrig bleibt.
    public func truncated(atFrontier frontier: Int) -> BookTalkChunk? {
        if end <= frontier { return self }
        if start >= frontier { return nil }
        let units = Array(text.utf16)
        var cut = 0
        for s in BookTalkSentences.split(text) where offsetMap[s.end - 1] + 1 <= frontier { cut = s.end }
        guard cut > 0 else { return nil }
        return BookTalkChunk(id: id, chapterIndex: chapterIndex, chapterTitle: chapterTitle, page: page, start: start,
                             end: offsetMap[cut - 1] + 1, text: String(decoding: units[0..<cut], as: UTF16.self),
                             offsetMap: Array(offsetMap[0...cut]))
    }
}

public struct BookTalkPageMark: Sendable, Equatable {
    public let offset: Int
    public let page: Int
}

public struct BookTalkIndex: Sendable {
    public static let minChunk = 500
    public static let maxChunk = 900

    public let title: String
    public let chunks: [BookTalkChunk]
    /// Aufsteigend nach Offset.
    public let pageMarks: [BookTalkPageMark]
    /// FNV-1a des Quelltextes: Neuaufbau, wenn sich der Text ändert.
    public let sourceHash: UInt64
    public let sourceLength: Int

    public init(title: String, text: String) {
        self.title = title
        self.sourceHash = BookTalkText.fnv1a(text)
        let units = Array(text.utf16)
        self.sourceLength = units.count
        let marks = BookTalkIndex.scanMarks(units)
        self.pageMarks = marks
        self.chunks = BookTalkIndex.pack(BookTalkIndex.parse(units), marks: marks)
    }

    /// Seite an einem Originaloffset (letzte Marke bei oder vor `offset`).
    public func page(at offset: Int) -> Int? { BookTalkIndex.page(in: pageMarks, at: offset) }

    static func page(in marks: [BookTalkPageMark], at offset: Int) -> Int? {
        var lo = 0, hi = marks.count
        while lo < hi {
            let mid = (lo + hi) / 2
            if marks[mid].offset <= offset { lo = mid + 1 } else { hi = mid }
        }
        return lo == 0 ? nil : marks[lo - 1].page
    }

    /// Alle Seitenmarken `{{S. n}}`, auch mitten im Absatz.
    private static func scanMarks(_ u: [UInt16]) -> [BookTalkPageMark] {
        var out: [BookTalkPageMark] = []
        var i = 0
        while i < u.count {
            if u[i] == 123, let m = BookTalkText.pageMarker(u, at: i, limit: u.count) {
                out.append(BookTalkPageMark(offset: i, page: m.page))
                i = m.end
            } else {
                i += 1
            }
        }
        return out
    }

    /// Alle Seiten, die ein Bereich berührt (Seite am Anfang plus Marken darin).
    public func pages(from start: Int, to end: Int) -> [Int] {
        var out: [Int] = []
        if let p = page(at: start) { out.append(p) }
        for m in pageMarks where m.offset > start && m.offset < end && !out.contains(m.page) { out.append(m.page) }
        return out
    }

    /// Leseposition am Ende der Seite `page` (= Beginn der Marke der Folgeseite).
    public func position(endOfPage page: Int) -> Int {
        pageMarks.first(where: { $0.page > page })?.offset ?? sourceLength
    }

    // MARK: Parsen

    struct Paragraph {
        let chapterIndex: Int
        let chapterTitle: String
        let start: Int
        let end: Int
        let text: String
        let map: [Int]
    }

    private static func parse(_ u: [UInt16]) -> [Paragraph] {
        var paragraphs: [Paragraph] = []
        var chapterIndex = 0
        var chapterTitle = ""
        var open: [(s: Int, e: Int)] = []

        func flush() {
            defer { open.removeAll() }
            guard let first = open.first, let last = open.last else { return }
            var cleaner = BookTalkText.Cleaner()
            for (n, r) in open.enumerated() {
                if n > 0 { cleaner.separator(at: r.s) }
                cleaner.append(u, from: r.s, to: r.e)
            }
            let c = cleaner.finish()
            guard !c.text.isEmpty else { return }
            paragraphs.append(Paragraph(chapterIndex: chapterIndex, chapterTitle: chapterTitle,
                                        start: c.map.first ?? first.s, end: last.e, text: c.text, map: c.map))
        }

        var i = 0
        while i <= u.count {
            var lineEnd = i
            while lineEnd < u.count, u[lineEnd] != 10 { lineEnd += 1 }
            var s = i, e = lineEnd
            while s < e, BookTalkText.isSpace(u[s]) { s += 1 }
            while e > s, BookTalkText.isSpace(u[e - 1]) { e -= 1 }
            i = lineEnd + 1
            if s == e { flush(); continue }
            if u[s] == 35 {
                var h = s
                while h < e, u[h] == 35 { h += 1 }
                if h < e, BookTalkText.isSpace(u[h]) {
                    flush()
                    chapterIndex += 1
                    var t = h
                    while t < e, BookTalkText.isSpace(u[t]) { t += 1 }
                    chapterTitle = String(decoding: u[t..<e], as: UTF16.self)
                    continue
                }
            }
            while let m = BookTalkText.pageMarker(u, at: s, limit: e) {
                flush()
                s = m.end
                while s < e, BookTalkText.isSpace(u[s]) { s += 1 }
            }
            if s >= e { continue }
            open.append((s, e))
        }
        flush()
        return paragraphs
    }

    // MARK: Stücke bilden

    private static func pack(_ paragraphs: [Paragraph], marks: [BookTalkPageMark]) -> [BookTalkChunk] {
        var out: [BookTalkChunk] = []
        var group: [Paragraph] = []
        var groupLen = 0

        func emit() {
            defer { group.removeAll(); groupLen = 0 }
            guard let first = group.first, let last = group.last else { return }
            var text = ""
            var map: [Int] = []
            for (n, p) in group.enumerated() {
                if n > 0 {
                    text += " "
                    map.append(p.start)
                }
                text += p.text
                map.append(contentsOf: p.map.dropLast())
            }
            map.append(last.map.last ?? last.end)
            out.append(BookTalkChunk(id: out.count, chapterIndex: max(first.chapterIndex - 1, 0), chapterTitle: first.chapterTitle,
                                     page: page(in: marks, at: first.start), start: first.start, end: map.last ?? last.end,
                                     text: text, offsetMap: map))
        }

        for p in paragraphs {
            if let g = group.first, g.chapterIndex != p.chapterIndex { emit() }
            for piece in split(p) {
                let len = piece.text.utf16.count
                if !group.isEmpty, groupLen >= minChunk || groupLen + 1 + len > maxChunk { emit() }
                groupLen += (group.isEmpty ? 0 : 1) + len
                group.append(piece)
            }
        }
        emit()
        return out
    }

    /// Absätze über `maxChunk` werden an Satzgrenzen (notfalls Wortgrenzen) geteilt.
    private static func split(_ p: Paragraph) -> [Paragraph] {
        let u = Array(p.text.utf16)
        guard u.count > maxChunk else { return [p] }
        var ranges: [(Int, Int)] = []
        var from = 0, to = 0
        for s in BookTalkSentences.split(p.text) {
            if s.end - from > maxChunk, to > from {
                ranges.append((from, to))
                from = s.start
            }
            while s.end - from > maxChunk {
                var e = from + maxChunk
                while e > from + minChunk, u[e - 1] != 32 { e -= 1 }
                ranges.append((from, e))
                from = e
                while from < s.end, u[from] == 32 { from += 1 }
            }
            to = s.end
        }
        if to > from { ranges.append((from, to)) }
        return ranges.compactMap { r in
            var a = r.0, b = r.1
            while a < b, u[a] == 32 { a += 1 }
            while b > a, u[b - 1] == 32 { b -= 1 }
            guard b > a else { return nil }
            let map = Array(p.map[a...b])
            return Paragraph(chapterIndex: p.chapterIndex, chapterTitle: p.chapterTitle,
                             start: map[0], end: map[map.count - 1],
                             text: String(decoding: u[a..<b], as: UTF16.self), map: map)
        }
    }
}

// MARK: - Textwerkzeuge

enum BookTalkText {
    static func isSpace(_ c: UInt16) -> Bool { c == 32 || c == 9 || c == 10 || c == 13 || c == 0xA0 }

    /// Gefaltete Wortfolge (ä→a …) für Namens- und Formvergleiche.
    static func foldedSequence(_ s: String) -> [String] { BookTalkTokens.foldedWords(s) }

    /// Ganzes Wort bzw. Wortfolge (ohne Groß/Klein) in einer gefalteten Wortfolge.
    static func contains(_ words: [String], form: String) -> Bool {
        let f = foldedSequence(form)
        guard !f.isEmpty, words.count >= f.count else { return false }
        for i in 0...(words.count - f.count) where Array(words[i..<(i + f.count)]) == f { return true }
        return false
    }

    static func fnv1a(_ s: String) -> UInt64 {
        var h: UInt64 = 0xcbf29ce484222325
        for b in s.utf8 { h = (h ^ UInt64(b)) &* 0x100000001b3 }
        return h
    }

    /// `{{S. 12}}` ab `at`; liefert Seite und Offset hinter der Marke.
    static func pageMarker(_ u: [UInt16], at: Int, limit: Int) -> (page: Int, end: Int)? {
        let head = Array("{{S.".utf16)
        guard at + head.count < limit, Array(u[at..<(at + head.count)]) == head else { return nil }
        var i = at + head.count
        while i < limit, isSpace(u[i]) { i += 1 }
        var n = 0, digits = 0
        while i < limit, u[i] >= 48, u[i] <= 57, digits < 7 {
            n = n * 10 + Int(u[i] - 48)
            i += 1
            digits += 1
        }
        guard digits > 0 else { return nil }
        while i < limit, isSpace(u[i]) { i += 1 }
        guard i + 1 < limit, u[i] == 125, u[i + 1] == 125 else { return nil }
        return (n, i + 2)
    }

    /// Säubert (Sprecher `<Name>`, Stilmarken `[calm]`, Leerraum falten) und führt eine Offset-Abbildung mit.
    struct Cleaner {
        private var units: [UInt16] = []
        private var map: [Int] = []
        private var pendingSpace: Int?

        mutating func separator(at offset: Int) {
            if pendingSpace == nil, !units.isEmpty { pendingSpace = offset }
        }

        mutating func append(_ u: [UInt16], from: Int, to: Int) {
            var i = from
            while i < to {
                let c = u[i]
                if BookTalkText.isSpace(c) {
                    if pendingSpace == nil, !units.isEmpty { pendingSpace = i }
                    i += 1
                    continue
                }
                if c == 123, let m = BookTalkText.pageMarker(u, at: i, limit: to) {
                    i = m.end
                    continue
                }
                if c == 60, !(i > from && u[i - 1] == 60), let j = closing(u, open: i, to: to, maxLen: 40),
                   !(j + 1 < to && u[j + 1] == 62) {
                    i = j + 1
                    continue
                }
                if c == 91, let j = closing(u, open: i, to: to, maxLen: 24) {
                    i = j + 1
                    continue
                }
                if let p = pendingSpace {
                    units.append(32)
                    map.append(p)
                    pendingSpace = nil
                }
                units.append(c)
                map.append(i)
                i += 1
            }
        }

        /// `<Name>` (Buchstaben, Ziffern, Leerzeichen, Punkt, Bindestrich) oder `[calm]` (a–z, -, _).
        private func closing(_ u: [UInt16], open: Int, to: Int, maxLen: Int) -> Int? {
            let isAngle = u[open] == 60
            let close: UInt16 = isAngle ? 62 : 93
            var j = open + 1
            while j < to, j - open <= maxLen {
                let c = u[j]
                if c == close { return j > open + 1 ? j : nil }
                let ok: Bool
                if isAngle {
                    ok = c == 32 || c == 46 || c == 45 || c == 95 || c == 39
                        || (Unicode.Scalar(c).map { $0.properties.isAlphabetic || ($0.value >= 48 && $0.value <= 57) } ?? false)
                } else {
                    ok = (c >= 97 && c <= 122) || c == 45 || c == 95
                }
                if !ok { return nil }
                j += 1
            }
            return nil
        }

        func finish() -> (text: String, map: [Int]) {
            var m = map
            m.append((map.last ?? -1) + 1)
            return (String(decoding: units, as: UTF16.self), m)
        }
    }
}

// MARK: - Sätze

struct BookTalkSentence: Sendable, Equatable {
    /// UTF-16-Offsets im untersuchten Text; `end` exklusiv.
    let start: Int
    let end: Int
}

enum BookTalkSentences {
    private static let abbreviations: Set<String> = [
        "z", "b", "d", "h", "u", "a", "o", "bzw", "usw", "ca", "dr", "nr", "s", "vgl", "etc", "evtl", "ggf", "inkl",
        "prof", "str", "sog", "bspw", "e", "g", "i", "vs", "mr", "mrs", "ms", "st", "no", "p", "pp", "ff", "f",
    ]
    private static let closers: Set<UInt16> = [0x201C, 0x201D, 0x2018, 0x2019, 34, 39, 0xBB, 0xAB, 41, 93]
    private static let openers: Set<UInt16> = [0x201E, 0x201C, 0x201A, 0x2018, 34, 39, 0xBB, 0xAB, 40]

    private static func isUpperOrDigit(_ c: UInt16) -> Bool {
        guard let sc = Unicode.Scalar(c) else { return false }
        return sc.properties.isUppercase || (c >= 48 && c <= 57)
    }

    static func split(_ text: String) -> [BookTalkSentence] {
        let u = Array(text.utf16)
        var out: [BookTalkSentence] = []
        var start = 0
        var i = 0
        while i < u.count {
            let c = u[i]
            guard c == 46 || c == 33 || c == 63 || c == 0x2026 else {
                i += 1
                continue
            }
            var j = i + 1
            while j < u.count, u[j] == 46 || u[j] == 33 || u[j] == 63 { j += 1 }
            while j < u.count, closers.contains(u[j]) { j += 1 }
            guard j < u.count, u[j] == 32 else {
                i = max(j, i + 1)
                continue
            }
            var k = j
            while k < u.count, u[k] == 32 { k += 1 }
            let next = k < u.count ? u[k] : 0
            var boundary = isUpperOrDigit(next) || openers.contains(next)
            if boundary, c == 46, j == i + 1 {
                var w = i
                while w > start, let sc = Unicode.Scalar(u[w - 1]), sc.properties.isAlphabetic { w -= 1 }
                let word = String(decoding: u[w..<i], as: UTF16.self).lowercased()
                if abbreviations.contains(word), word.utf16.count <= 4 { boundary = false }
            }
            if boundary {
                out.append(BookTalkSentence(start: start, end: j))
                start = k
            }
            i = max(k, i + 1)
        }
        var e = u.count
        while e > start, u[e - 1] == 32 { e -= 1 }
        if e > start { out.append(BookTalkSentence(start: start, end: e)) }
        return out
    }
}
