import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): streamender Satztrenner für die Stimme.
// Deltas beliebig zerschnitten rein, sprechbare Häppchen raus. `text` ist lesbar ohne Markdown-Zeichen, `speech` zusätzlich
// ohne Adressen, Sonderzeichen und mit ausgeschriebenen Abkürzungen („z. B.“ → „zum Beispiel“). Emojis fallen überall weg.

public enum AssistantSpeech {
    public struct Splitter: Sendable {
        /// Erstes Häppchen darf nach so vielen Zeichen am Komma enden (Latenz bis zum ersten Ton).
        static let firstCommaChars = 40
        /// Länger ohne Satzende: an Komma oder Leerzeichen trennen.
        static let maxChunk = 320

        private var buffer: [UInt16] = []
        private var emitted = 0
        private var lineStart = true

        public init() {}

        public mutating func feed(_ delta: String) -> [(text: String, speech: String)] {
            buffer.append(contentsOf: delta.utf16)
            return drain(final: false)
        }

        public mutating func finish() -> [(text: String, speech: String)] {
            let out = drain(final: true)
            buffer.removeAll()
            lineStart = true
            return out
        }

        private struct Cut {
            let end: Int
            let next: Int
            let newline: Bool
        }

        private mutating func drain(final: Bool) -> [(text: String, speech: String)] {
            var out: [(text: String, speech: String)] = []
            while !buffer.isEmpty, let cut = nextCut(final: final) {
                let chunk = String(decoding: buffer[0..<cut.end], as: UTF16.self)
                buffer.removeFirst(min(max(cut.next, 1), buffer.count))
                let atLineStart = lineStart
                lineStart = cut.newline || (lineStart && chunk.allSatisfy { $0.isWhitespace })
                if let r = AssistantSpeech.render(chunk, lineStart: atLineStart) {
                    out.append(r)
                    emitted += 1
                }
            }
            return out
        }

        // MARK: Grenzen

        private static let closers: Set<UInt16> = [0x201C, 0x201D, 0x2018, 0x2019, 34, 39, 0xBB, 0xAB, 41, 93, 42, 95, 96, 126]
        private static let openers: Set<UInt16> = [0x201E, 0x201C, 0x201A, 0x2018, 34, 39, 0xBB, 0xAB, 40, 91]
        private static let abbreviations: Set<String> = [
            "z", "b", "d", "h", "u", "a", "o", "v", "i", "t", "bzw", "usw", "ca", "dr", "nr", "s", "vgl", "etc", "evtl", "ggf", "inkl",
            "prof", "str", "sog", "bspw", "e", "g", "vs", "mr", "mrs", "ms", "st", "no", "p", "pp", "ff", "f", "hr", "fr", "abs",
            "art", "kap", "bd", "jh", "mio", "mrd", "zzgl", "ebd",
        ]
        private static let months: Set<String> = [
            "januar", "februar", "märz", "april", "mai", "juni", "juli", "august", "september", "oktober", "november", "dezember",
        ]

        private static func isSpace(_ c: UInt16) -> Bool { c == 32 || c == 9 || c == 0xA0 || c == 13 }
        private static func isDigit(_ c: UInt16) -> Bool { c >= 48 && c <= 57 }
        private static func isLetter(_ c: UInt16) -> Bool { Unicode.Scalar(c)?.properties.isAlphabetic ?? false }
        private static func isUpper(_ c: UInt16) -> Bool { Unicode.Scalar(c)?.properties.isUppercase ?? false }

        private func nextCut(final: Bool) -> Cut? {
            let u = buffer
            let n = u.count
            var i = 0
            var bracket = 0
            while i < n {
                let c = u[i]
                if c == 10 { return Cut(end: i, next: i + 1, newline: true) }
                if c == 91 { bracket += 1 }
                if c == 93 {
                    bracket = max(0, bracket - 1)
                    if i + 1 >= n, !final { return nil }
                    if i + 1 < n, u[i + 1] == 40 {
                        var j = i + 2
                        while j < n, u[j] != 41, u[j] != 10 { j += 1 }
                        if j >= n, !final { return nil }
                        i = j < n && u[j] == 41 ? j + 1 : j
                        continue
                    }
                }
                guard bracket == 0 else { i += 1; continue }

                if c == 46 || c == 33 || c == 63 || c == 0x2026 {
                    var j = i + 1
                    while j < n, u[j] == 46 || u[j] == 33 || u[j] == 63 || u[j] == 0x2026 { j += 1 }
                    let terminators = j - i
                    while j < n, Self.closers.contains(u[j]) { j += 1 }
                    if j >= n { return final ? Cut(end: n, next: n, newline: false) : nil }
                    guard Self.isSpace(u[j]) else { i = max(j, i + 1); continue }
                    var k = j
                    while k < n, Self.isSpace(u[k]) { k += 1 }
                    if k >= n { return final ? Cut(end: j, next: n, newline: false) : nil }
                    if u[k] == 10 { i = k; continue }
                    if c == 46, i > 0, Self.isDigit(u[i - 1]), !final {
                        // Ordnungszahl vor Monatsnamen: das ganze nächste Wort muss da sein.
                        var e = k
                        while e < n, Self.isLetter(u[e]) { e += 1 }
                        if e >= n { return nil }
                    }
                    if isBoundary(u, at: i, terminators: terminators, next: k) { return Cut(end: j, next: k, newline: false) }
                    i = k
                    continue
                }

                if emitted == 0, c == 44 || c == 59 || c == 58 {
                    if i + 1 >= n, !final { return nil }
                    if i + 1 < n, Self.isSpace(u[i + 1]) {
                        let prefix = String(decoding: u[0..<i + 1], as: UTF16.self)
                        if (AssistantSpeech.render(prefix, lineStart: lineStart)?.text.count ?? 0) >= Self.firstCommaChars {
                            var k = i + 1
                            while k < n, Self.isSpace(u[k]) { k += 1 }
                            return Cut(end: i + 1, next: k, newline: false)
                        }
                    }
                }
                i += 1
            }
            guard n > Self.maxChunk else { return final ? Cut(end: n, next: n, newline: false) : nil }
            return capped(u)
        }

        /// Sehr lange Satzfolgen ohne Satzende: lieber am Komma, sonst am Leerzeichen trennen.
        private func capped(_ u: [UInt16]) -> Cut {
            let lower = Self.maxChunk / 2
            var k = Self.maxChunk
            while k > lower { if u[k - 1] == 44, Self.isSpace(u[k]) { return Cut(end: k, next: k + 1, newline: false) }; k -= 1 }
            k = Self.maxChunk
            while k > lower { if Self.isSpace(u[k]) { return Cut(end: k, next: k + 1, newline: false) }; k -= 1 }
            let end = (0xD800...0xDBFF).contains(u[Self.maxChunk - 1]) ? Self.maxChunk - 1 : Self.maxChunk
            return Cut(end: end, next: end, newline: false)
        }

        /// Satzende bei „.“, „!“, „?“, „…“, wenn danach (nach Leerzeichen) ein neuer Satz beginnt.
        /// Abkürzungen („z. B.“, „Dr.“, „S. 12“), Aufzählungsnummern und Datumsordnungszahlen („11. November“) trennen nicht.
        private func isBoundary(_ u: [UInt16], at i: Int, terminators: Int, next k: Int) -> Bool {
            let c = u[i]
            if c == 33 || c == 63 { return true }
            let next = u[k]
            guard Self.isUpper(next) || Self.isDigit(next) || Self.openers.contains(next) else { return false }
            if c == 0x2026 || terminators > 1 { return i >= 12 }
            var w = i
            while w > 0, Self.isLetter(u[w - 1]) { w -= 1 }
            if w < i {
                let word = String(decoding: u[w..<i], as: UTF16.self).lowercased()
                return !(Self.abbreviations.contains(word) && word.utf16.count <= 4)
            }
            var d = i
            while d > 0, Self.isDigit(u[d - 1]) { d -= 1 }
            guard d < i else { return true }
            if d == 0 { return false }
            if i - d <= 2 {
                var e = k
                while e < u.count, Self.isLetter(u[e]) { e += 1 }
                if Self.months.contains(String(decoding: u[k..<e], as: UTF16.self).lowercased()) { return false }
            }
            return true
        }
    }

    // MARK: Aufbereitung eines Häppchens

    /// Entfernt Markdown und Emojis. `nil`, wenn nichts Lesbares bleibt. `lineStart`: das Häppchen beginnt am Zeilenanfang
    /// (Überschriften, Aufzählungszeichen, Zitat-Striche fallen dann weg).
    static func render(_ raw: String, lineStart: Bool) -> (text: String, speech: String)? {
        var s = String(String.UnicodeScalarView(raw.unicodeScalars.filter { !isEmoji($0) }))
        s = s.replacingOccurrences(of: "\t", with: " ").replacingOccurrences(of: "\r", with: " ")
            .replacingOccurrences(of: "\u{00A0}", with: " ")
        if lineStart {
            var t = Substring(s.drop(while: { $0 == " " }))
            if t.hasPrefix("```") { return nil }
            if t.filter({ $0 != " " }).count >= 3, t.allSatisfy({ "-*_ =".contains($0) }) { return nil }
            var hashes = 0
            while hashes < 6, t.dropFirst(hashes).first == "#" { hashes += 1 }
            if hashes > 0, t.dropFirst(hashes).first == " " { t = t.dropFirst(hashes) }
            while t.first == ">" { t = t.dropFirst().drop(while: { $0 == " " }) }
            for marker in ["- ", "* ", "+ ", "• ", "– ", "— "] where t.hasPrefix(marker) {
                t = t.dropFirst(marker.count)
                break
            }
            s = String(t)
        }
        let text = tidy(collapse(stripInline(s)))
        guard text.contains(where: { $0.isLetter || $0.isNumber }) else { return nil }
        return (text, speechForm(text))
    }

    /// Links `[Text](url)` → Text, Bilder → Beschriftung, Hervorhebungen, Code, Tabellenstriche.
    private static func stripInline(_ s: String) -> String {
        let ch = Array(s)
        var out = ""
        var i = 0
        while i < ch.count {
            let c = ch[i]
            if c == "[", let close = ch[(i + 1)...].firstIndex(of: "]"), close + 1 < ch.count, ch[close + 1] == "(",
               let end = ch[(close + 2)...].firstIndex(of: ")") {
                if out.hasSuffix("!") { out.removeLast() }
                out += String(ch[(i + 1)..<close])
                i = end + 1
                continue
            }
            switch c {
            case "*", "`": break
            case "~": if i + 1 < ch.count, ch[i + 1] == "~" { i += 1 } else { out.append(c) }
            case "_":
                let prev = i > 0 ? ch[i - 1] : " "
                let next = i + 1 < ch.count ? ch[i + 1] : " "
                if (prev.isLetter || prev.isNumber) && (next.isLetter || next.isNumber) { out.append(c) }
            case "|": out.append(" ")
            default: out.append(c)
            }
            i += 1
        }
        return out
    }

    /// Nach entfernten Emojis bleibt oft ein Leerzeichen vor dem Satzzeichen.
    private static func tidy(_ s: String) -> String {
        var s = s
        for punct in [",", ".", "!", "?", ";", ":"] { s = s.replacingOccurrences(of: " \(punct)", with: punct) }
        return s
    }

    private static func collapse(_ s: String) -> String {
        s.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    private static let abbreviationsDe: [(String, String)] = [
        ("z. B.", "zum Beispiel"), ("z.B.", "zum Beispiel"), ("d. h.", "das heißt"), ("d.h.", "das heißt"),
        ("u. a.", "unter anderem"), ("u.a.", "unter anderem"), ("u. v. m.", "und vieles mehr"), ("z. T.", "zum Teil"),
        ("i. d. R.", "in der Regel"), ("o. ä.", "oder ähnliches"), ("v. a.", "vor allem"), ("usw.", "und so weiter"),
        ("bzw.", "beziehungsweise"), ("ca.", "circa"), ("vgl.", "vergleiche"), ("ggf.", "gegebenenfalls"),
        ("evtl.", "eventuell"), ("Nr.", "Nummer"), ("Prof.", "Professor"), ("Dr.", "Doktor"), ("etc.", "et cetera"),
        ("Mio.", "Millionen"), ("Mrd.", "Milliarden"),
    ]

    /// Fassung für die Stimme: ohne Adressen und Sonderzeichen, Abkürzungen ausgeschrieben, Dezimalpunkt als Komma.
    private static func speechForm(_ text: String) -> String {
        var s = text.split(separator: " ").filter { w in
            !(w.hasPrefix("http://") || w.hasPrefix("https://") || w.hasPrefix("www."))
        }.joined(separator: " ")
        for (a, b) in abbreviationsDe { s = expand(s, a, b, digitAfter: false) }
        s = expand(s, "S.", "Seite", digitAfter: true)
        for arrow in ["→", "⇒", "->", "=>"] { s = s.replacingOccurrences(of: arrow, with: ", ") }
        s = s.replacingOccurrences(of: "&", with: " und ").replacingOccurrences(of: "...", with: "…")
        s = String(s.filter { !"[]{}<>\\^~|#".contains($0) }).replacingOccurrences(of: "_", with: " ")
        s = decimalCommas(s)
        s = collapse(s)
        for punct in [",", ".", "!", "?", ";", ":"] { s = s.replacingOccurrences(of: " \(punct)", with: punct) }
        while let f = s.first, ",;:".contains(f) { s.removeFirst(); s = s.trimmingCharacters(in: .whitespaces) }
        return s.contains(where: { $0.isLetter || $0.isNumber }) ? s : ""
    }

    /// Ersetzt `abbr` ohne Rücksicht auf Groß-/Kleinschreibung, aber nur am Wortanfang (nicht in „Bus.“ → „BuSeite“).
    private static func expand(_ s: String, _ abbr: String, _ full: String, digitAfter: Bool) -> String {
        var out = ""
        var cursor = s.startIndex
        while let r = s.range(of: abbr, options: .caseInsensitive, range: cursor..<s.endIndex) {
            out += s[cursor..<r.lowerBound]
            let before = r.lowerBound > s.startIndex ? s[s.index(before: r.lowerBound)] : nil
            var ok = before.map { !($0.isLetter || $0.isNumber) } ?? true
            if ok, digitAfter { ok = s[r.upperBound...].drop(while: { $0 == " " }).first?.isNumber ?? false }
            if ok {
                let capital = s[r.lowerBound].isUppercase
                out += capital ? full.prefix(1).uppercased() + full.dropFirst() : full
            } else {
                out += s[r]
            }
            cursor = r.upperBound
        }
        return out + s[cursor...]
    }

    /// „3.5“ → „3,5“ (sonst liest die Stimme „drei Punkt fünf“); „1.000“ und Datumsangaben („12.10.2026“) bleiben.
    private static func decimalCommas(_ s: String) -> String {
        let ch = Array(s)
        var out = ""
        var i = 0
        while i < ch.count {
            if ch[i].isNumber {
                var j = i
                while j < ch.count, ch[j].isNumber || (ch[j] == "." && j + 1 < ch.count && ch[j + 1].isNumber) { j += 1 }
                let run = String(ch[i..<j])
                let parts = run.split(separator: ".", omittingEmptySubsequences: false)
                if parts.count == 2, parts[1].count != 3 { out += parts[0] + "," + parts[1] } else { out += run }
                i = j
            } else {
                out.append(ch[i])
                i += 1
            }
        }
        return out
    }

    static func isEmoji(_ s: Unicode.Scalar) -> Bool {
        switch s.value {
        case 0x1F000...0x1FAFF, 0x2600...0x27BF, 0x2B00...0x2BFF, 0xFE00...0xFE0F, 0x200D, 0x20E3, 0xE0020...0xE007F,
             0x203C, 0x2049, 0x231A...0x231B, 0x23E9...0x23F3, 0x23F8...0x23FA, 0x24C2, 0x25AA...0x25AB, 0x25B6, 0x25C0,
             0x25FB...0x25FE, 0x2934...0x2935, 0x3030, 0x303D, 0x3297, 0x3299, 0x00A9, 0x00AE:
            return true
        default:
            return false
        }
    }
}
