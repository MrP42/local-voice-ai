import Foundation

/// Port von pasteCleanup.ts/textCleanup.ts. Keine Absatzbildung aus einzelnen Satzzeilen.
enum TextAufbereitenPaste {
    typealias Tape = TextAufbereiten.Tape
    typealias Edit = TextAufbereiten.Edit
    enum Kind { case code, block, text, blank }
    struct Line {
        let text: String
        let offset: Int
        var length: Int { text.utf16.count }
        var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }
    }
    static let block = TextAufbereiten.regex(#"^\s*(?:#{1,6}\s|>\s?|[-*+]\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$|⁂|\d{1,3}[.)]\s|<[^<>\n]{1,40}>|\[[^\]\n]*\]|\{\{S\.)"#)
    static let fence = TextAufbereiten.regex(#"^ {0,3}(`{3,}|~{3,})(.*)$"#)
    static let bareNumber = TextAufbereiten.regex(#"^[0-9]{1,3}$"#)
    // Explizite Buchstabenpaare wie JS /i ohne /u: kein ſ, İ oder ı durch ICU-Faltung.
    static let wordPage = TextAufbereiten.regex(#"^(?:[Ss][Ee][Ii][Tt][Ee]|[Ss]\.|[Pp][Aa][Gg][Ee]|[Pp]\.)\s*([0-9]{1,3})(?:\s*(?:[Vv][Oo][Nn]|[Oo][Ff]|/)\s*([0-9]{1,3}))?$"#)
    static let slashPage = TextAufbereiten.regex(#"^([0-9]{1,3})\s*/\s*([0-9]{1,3})$"#)
    static let dashedNumber = TextAufbereiten.regex(#"^[-–—]\s*[0-9]{1,3}\s*[-–—]$"#)
    static let sentence = TextAufbereiten.regex(#"[.!?…:;]["'»«“”‘’)\]]*\s*$"#)
    static let lower = TextAufbereiten.regex(#"^\p{Ll}"#)
    static let hyphen = TextAufbereiten.regex(#"\p{Ll}-$"#)
    static let suspended = TextAufbereiten.regex(#"^(?:und|oder|bzw\.|sowie|als auch|noch)\s"#)
    static let initials = TextAufbereiten.regex(#"^(?:\p{Lu}\.\s*){1,3}$"#)
    // tidySpaces nutzt die kleinere Strukturdefinition aus textCleanup.ts.
    static let spaceBlock = TextAufbereiten.regex(#"^\s*(?:#{1,3}\s|>\s?|[-*]\s|-{3,}\s*$|\*{3,}\s*$|_{3,}\s*$|⁂|<[^<>\n]{1,40}>|\{\{S\.)"#)
    static let timestamp = TextAufbereiten.regex(#"^[ \t]*[\[(]\d{1,2}:\d{2}(?::\d{2})?(?:[.,]\d{1,3})?[\])][ \t]*(?::[ \t]*)?|^[ \t]*:[ \t]+(?=\S)"#)

    static func test(_ regex: NSRegularExpression, _ s: String) -> Bool {
        regex.firstMatch(in: s, range: NSRange(location: 0, length: s.utf16.count)) != nil
    }
    static func lines(_ text: String) -> [Line] {
        var offset = 0
        return text.components(separatedBy: "\n").map { line in
            defer { offset += line.utf16.count + 1 }
            return Line(text: line, offset: offset)
        }
    }
    static func isPageNumber(_ line: String) -> Bool {
        let s = line.trimmingCharacters(in: .whitespacesAndNewlines)
        guard s.utf16.count <= 40 else { return false }
        if test(bareNumber, s) || test(dashedNumber, s) { return true }
        let range = NSRange(location: 0, length: s.utf16.count)
        guard let m = wordPage.firstMatch(in: s, range: range) ?? slashPage.firstMatch(in: s, range: range) else { return false }
        if m.range(at: 2).location == NSNotFound { return true }
        let ns = s as NSString
        return (Int(ns.substring(with: m.range(at: 2))) ?? 0) >= (Int(ns.substring(with: m.range(at: 1))) ?? 0)
    }
    static func structural(_ s: String) -> Bool {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        return test(block, s) || t.hasPrefix("|") || t.hasSuffix("|")
    }
    static func classify(_ rows: [Line]) -> [Kind] {
        var kinds = [Kind](repeating: .text, count: rows.count)
        var open: (String, Int)?
        var indented = false, blankStart: Int?, blanksAfterIndented = false
        for (i, row) in rows.enumerated() {
            let s = row.text, ns = s as NSString
            let match = fence.firstMatch(in: s, range: NSRange(location: 0, length: ns.length))
            let mark = match.map { ns.substring(with: $0.range(at: 1)) }
            let rest = match.map { ns.substring(with: $0.range(at: 2)) } ?? ""
            if let current = open {
                kinds[i] = .code
                if let mark, String(mark.prefix(1)) == current.0, mark.count >= current.1, rest.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { open = nil }
                continue
            }
            if row.trimmed.isEmpty {
                if blankStart == nil { blankStart = i; blanksAfterIndented = indented }
                kinds[i] = .blank; indented = false; continue
            }
            let startsBlock = blankStart != nil || i == 0
            if let mark, !(mark.hasPrefix("`") && rest.contains("`")) {
                open = (String(mark.prefix(1)), mark.count)
                kinds[i] = .code; blankStart = nil; indented = false; continue
            }
            if (s.hasPrefix("    ") || s.hasPrefix("\t")) && (indented || (startsBlock && !isPageNumber(s))) {
                if let start = blankStart, blanksAfterIndented {
                    for j in start..<i { kinds[j] = .code }
                }
                kinds[i] = .code; indented = true; blankStart = nil; continue
            }
            kinds[i] = structural(s) ? .block : .text
            indented = false; blankStart = nil
        }
        return kinds
    }

    static func run(_ tape: inout Tape) {
        let original = tape
        let trimmed = tape.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.utf16.count < 80 && !trimmed.contains(where: { "\r\n\u{c}".contains($0) }) { return }
        tape.replace("\r\n?", "\n", rule: "control")
        for _ in 0..<16 {
            let before = tape.text
            repeated(&tape)
            control(&tape)
            drop(&tape, rule: "pageNumbers") { rows, kinds in
                Set(rows.indices.filter { kinds[$0] != .code && isPageNumber(rows[$0].text) })
            }
            webNoise(&tape)
            fragments(&tape)
            spaces(&tape)
            blanks(&tape)
            if TextAufbereiten.identisch(before, tape.text) { return }
        }
        tape = original
    }

    /// Wie mapRuns: Code wird vor einer Zeichenänderung klassifiziert.
    static func outsideCode(_ tape: inout Tape, rule: String, transform: (String) -> String) {
        let rows = lines(tape.text), kinds = classify(rows)
        var edits: [Edit] = [], start = 0
        while start < rows.count {
            if kinds[start] == .code { start += 1; continue }
            var end = start + 1
            while end < rows.count && kinds[end] != .code { end += 1 }
            let old = rows[start..<end].map(\.text).joined(separator: "\n")
            let new = transform(old)
            edits += differences(old, new, offset: rows[start].offset)
            start = end
        }
        tape.apply(edits, rule: rule)
    }

    /// Begrenzte lokale Suche statt quadratischem Gesamtdiff. Entfernte Zeilen
    /// werden bereits direkt adressiert; Zeichenregeln brauchen nur kurze Anker.
    static func differences(_ old: String, _ new: String, offset: Int) -> [Edit] {
        guard !TextAufbereiten.identisch(old, new) else { return [] }
        let a = old.map(String.init), b = new.map(String.init)
        var offsets = [0]
        for char in a { offsets.append(offsets.last! + char.utf16.count) }
        var i = 0, j = 0, edits: [Edit] = []
        func equal(_ x: Int, _ y: Int) -> Bool { TextAufbereiten.identisch(a[x], b[y]) }
        while i < a.count || j < b.count {
            if i < a.count && j < b.count && equal(i, j) { i += 1; j += 1; continue }
            let startI = i, startJ = j
            var anchor: (Int, Int)?
            search: for distance in 1...64 {
                for da in max(0, distance - 32)...min(32, distance) {
                    let db = distance - da, x = i + da, y = j + db
                    guard x < a.count, y < b.count, equal(x, y) else { continue }
                    let look = min(4, min(a.count - x, b.count - y))
                    if (0..<look).allSatisfy({ equal(x + $0, y + $0) }) {
                        anchor = (x, y); break search
                    }
                }
            }
            if let anchor { i = anchor.0; j = anchor.1 }
            else {
                var suffix = 0
                while suffix < min(a.count - i, b.count - j) && equal(a.count - 1 - suffix, b.count - 1 - suffix) { suffix += 1 }
                i = a.count - suffix; j = b.count - suffix
            }
            edits.append(Edit(NSRange(location: offset + offsets[startI], length: offsets[i] - offsets[startI]), b[startJ..<j].joined()))
        }
        return edits
    }

    static func control(_ tape: inout Tape) {
        if tape.text.hasPrefix("\u{feff}") { tape.apply([Edit(NSRange(location: 0, length: 1), "")], rule: "control") }
        outsideCode(&tape, rule: "control") { s in
            var s = s.replacingOccurrences(of: "\u{c}", with: "\n")
            for (pattern, replacement) in [
                (#"[\u0000-\u0008\u000B\u000E-\u001F\u007F-\u009F]"#, ""),
                (#"\u00AD[ \t]*\n(?=\p{Ll})"#, ""),
                (#"[\u00AD\u200B\uFEFF]"#, ""), (#"\u00A0"#, " ")
            ] { s = TextAufbereiten.replacing(s, pattern, replacement) }
            return s
        }
    }

    static func drop(_ tape: inout Tape, rule: String, keepFeed: Bool = false,
                     select: ([Line], [Kind]) -> Set<Int>) {
        let rows = lines(tape.text), kinds = classify(rows), dropped = select(rows, kinds)
        var edits: [Edit] = [], skipBlank = false
        var outputWasBlank = true
        for i in rows.indices {
            let row = rows[i], trailing = i + 1 < rows.count ? 1 : 0
            if skipBlank && kinds[i] == .blank {
                edits.append(Edit(NSRange(location: row.offset, length: row.length + trailing), ""))
                skipBlank = false; continue
            }
            skipBlank = false
            if kinds[i] == .code || !dropped.contains(i) { outputWasBlank = row.trimmed.isEmpty; continue }
            if keepFeed && row.text.contains("\u{c}") {
                edits.append(Edit(NSRange(location: row.offset, length: row.length), "\u{c}"))
                outputWasBlank = true; continue
            }
            // Letzte Zeile: auch ihren vorangehenden Umbruch entfernen.
            if trailing == 0 && row.offset > 0 && edits.last.map({ NSMaxRange($0.range) <= row.offset - 1 }) != false {
                edits.append(Edit(NSRange(location: row.offset - 1, length: row.length + 1), ""))
            } else { edits.append(Edit(NSRange(location: row.offset, length: row.length + trailing), "")) }
            skipBlank = outputWasBlank && i + 1 < rows.count && kinds[i + 1] == .blank
        }
        tape.apply(edits, rule: rule)
    }

    static func repeated(_ tape: inout Tape) {
        guard tape.text.utf16.count >= 1500 else { return }
        drop(&tape, rule: "repeated", keepFeed: true) { rows, kinds in
            var page = [Int](repeating: 0, count: rows.count), contents: [[Int]] = [[]], current = 0
            func advance(_ n: Int) { current += n; while contents.count <= current { contents.append([]) } }
            for i in rows.indices {
                let s = rows[i].text
                let feeds = kinds[i] == .code ? 0 : s.filter { $0 == "\u{c}" }.count
                let leading = kinds[i] == .code ? 0 : s.drop(while: { $0 == " " || $0 == "\t" }).prefix(while: { $0 == "\u{c}" }).count
                if leading > 0 { advance(leading) }
                page[i] = current
                let bare = feeds > 0 ? s.replacingOccurrences(of: "\u{c}", with: "") : s
                if kinds[i] != .code && isPageNumber(bare) { advance(1) }
                else if !bare.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { contents[current].append(i) }
                if feeds > leading { advance(feeds - leading) }
            }
            let pages = contents.filter { !$0.isEmpty }.count
            guard pages >= 2 else { return [] }
            var edge = Set<Int>(), seen: [Data: [Int]] = [:]
            for rows in contents { edge.formUnion(rows.prefix(2)); edge.formUnion(rows.suffix(2)) }
            for i in rows.indices where kinds[i] == .text {
                guard let key = repeatKey(rows[i].text) else { continue }
                // JavaScript-Schlüssel vergleichen Zeichenfolgen ohne NFC-Gleichsetzung.
                seen[Data(key.utf8), default: []].append(i)
            }
            var dropped = Set<Int>()
            for at in seen.values where at.count >= 3 {
                let gaps = zip(at.dropFirst(), at).map { $0.0 - $0.1 }
                guard gaps.allSatisfy({ $0 >= 8 }) else { continue }
                let onPages = Set(at.map { page[$0] }).count >= Int(ceil(Double(pages) * 0.6))
                let median = Double(select(gaps, rank: gaps.count / 2))
                let rhythmic = at.count >= 4 && at.allSatisfy { edge.contains($0) }
                    && gaps.allSatisfy { abs(Double($0) - median) <= median * 0.3 }
                if onPages || rhythmic { dropped.formUnion(at) }
            }
            return dropped
        }
    }
    /// Median der Abstände in linearer Zeit, auch bei einem sehr langen Dokument.
    static func select(_ values: [Int], rank: Int) -> Int {
        if values.count <= 5 { return values.sorted()[rank] }
        let medians = stride(from: 0, to: values.count, by: 5).map { start -> Int in
            let group = values[start..<min(start + 5, values.count)].sorted()
            return group[group.count / 2]
        }
        let pivot = select(medians, rank: medians.count / 2)
        let lower = values.filter { $0 < pivot }, upper = values.filter { $0 > pivot }
        if rank < lower.count { return select(lower, rank: rank) }
        if rank >= values.count - upper.count { return select(upper, rank: rank - (values.count - upper.count)) }
        return pivot
    }
    static func repeatKey(_ raw: String) -> String? {
        guard raw.utf16.count <= 5000 else { return nil }
        let s = TextAufbereiten.replacing(TextAufbereiten.replacing(raw, #"[\u00AD\u200B\uFEFF\u000C]"#, ""), #"\s+"#, " ").trimmingCharacters(in: .whitespacesAndNewlines)
        guard (3...120).contains(s.utf16.count), !structural(s), !isPageNumber(s),
              !TextAufbereiten.matches(s, #"[.!?…]["'»«“”‘’)\]]*$|\p{L}-$"#) else { return nil }
        let page = #"(?i)(?<![\p{L}\d])(?:(?:Seite|S\.|Page|p\.)\s*\d{1,3}(?!\d)|\d{1,3}\s*(?:von|of|/)\s*\d{1,3}(?!\d))"#
        let heading = #"(?i)^(?:(?:Kapitel|Teil|Abschnitt|Artikel|Anlage|Anhang|Chapter|Part|Section|Lektion|Folie|Tag)\s+\d|§\s*\d|(?!\d{1,3}\s*(?:von|of)\s*\d)\d+(?:\.\d+)*[.)]?\s+\p{L})"#
        return TextAufbereiten.matches(s, page) && !TextAufbereiten.matches(s, heading) ? TextAufbereiten.replacing(s, #"\d+"#, "#") : s
    }

    static let phrases: Set<String> = ["Zum Inhalt springen", "Zur Navigation springen", "Skip to content", "Nach oben", "Back to top", "Seite drucken", "Druckversion", "Artikel teilen", "Link kopieren", "Cookie-Einstellungen", "Alle akzeptieren", "Zurück zur Übersicht"].reduce(into: []) { $0.insert($1.lowercased()) }
    static let words: Set<String> = ["Drucken", "Teilen", "Zurück", "Mehr lesen", "Weiterlesen", "Print", "Share", "Facebook", "LinkedIn", "Twitter", "X", "Xing", "WhatsApp", "E-Mail"].reduce(into: []) { $0.insert($1.lowercased()) }
    static func noise(_ raw: String) -> Int {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !s.isEmpty, s.utf16.count <= 200 else { return 0 }
        let parts = s.components(separatedBy: CharacterSet(charactersIn: "·|/–•")).map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }.filter { !$0.isEmpty }
        guard !parts.isEmpty, parts.allSatisfy({ phrases.contains($0) || words.contains($0) }) else { return 0 }
        return parts.count >= 2 || phrases.contains(parts[0]) ? 2 : 1
    }
    static func webNoise(_ tape: inout Tape) {
        drop(&tape, rule: "webNoise") { rows, kinds in
            let noise = rows.indices.map { kinds[$0] == .text ? self.noise(rows[$0].text) : 0 }
            return Set(rows.indices.filter { noise[$0] == 2 || (noise[$0] == 1 && (($0 > 0 && noise[$0 - 1] > 0) || ($0 + 1 < rows.count && noise[$0 + 1] > 0))) })
        }
    }

    static func fragments(_ tape: inout Tape) {
        let rows = lines(tape.text), kinds = classify(rows)
        var edits: [Edit] = [], tail = "", initialText = "", content = 0, sentenceEnd = false, previousPlain = false
        for i in rows.indices {
            let raw = rows[i].text, plain = kinds[i] == .text && !rows[i].trimmed.isEmpty
            let body = timestamp.stringByReplacingMatches(in: raw, range: NSRange(location: 0, length: raw.utf16.count), withTemplate: "").trimmingCharacters(in: .whitespacesAndNewlines)
            let hyphenated = test(hyphen, tail) && test(lower, body)
            let joins = i > 0 && previousPlain && plain && !body.isEmpty
                && (hyphenated || (content <= 64 && test(initials, initialText.trimmingCharacters(in: .whitespacesAndNewlines))) || (!sentenceEnd && test(lower, body)))
            if joins {
                let removeHyphen = hyphenated && !test(suspended, body)
                let prev = rows[i - 1]
                let trimmedEnd = TextAufbereiten.replacing(prev.text, #"\s+$"#, "")
                let left = prev.offset + trimmedEnd.utf16.count - (removeHyphen ? 1 : 0)
                // Zeitstempel und Einrückung gehören zum zusammengefügten Satz.
                let bodyStart = raw.utf16.count - TextAufbereiten.replacing(raw, #"^\s+"#, "").utf16.count
                let prefix = timestamp.firstMatch(in: raw, range: NSRange(location: 0, length: raw.utf16.count)).map { NSMaxRange($0.range) } ?? 0
                let start = max(bodyStart, prefix)
                edits.append(Edit(NSRange(location: left, length: rows[i].offset + start - left), removeHyphen ? "" : " "))
                content += body.filter { !$0.isWhitespace }.utf16.count
                if content <= 64 { initialText += (removeHyphen ? "" : " ") + body }
            } else {
                content = raw.filter { !$0.isWhitespace }.utf16.count
                initialText = content <= 64 ? body : ""
            }
            tail = String((joins ? body : raw).trimmingCharacters(in: .whitespacesAndNewlines).suffix(4))
            sentenceEnd = test(sentence, joins ? body : raw)
            previousPlain = plain
        }
        tape.apply(edits, rule: "fragments")
    }

    static func spaces(_ tape: inout Tape) {
        outsideCode(&tape, rule: "spaces") { run in
            lines(run).map { row in
                let lead = String(row.text.prefix(while: \.isWhitespace))
                var body = String(row.text.dropFirst(lead.count))
                body = TextAufbereiten.replacing(body, "[ \t ]{2,}", " ")
                body = TextAufbereiten.replacing(body, #"(\p{L}|\d) +([,.;!?])(?=\s|$)"#, "$1$2")
                body = TextAufbereiten.replacing(body, "[ \t]+$", "")
                let plain = !row.trimmed.isEmpty && !test(spaceBlock, row.text)
                return (plain ? "" : lead) + body
            }.joined(separator: "\n")
        }
    }
    static func blanks(_ tape: inout Tape) {
        let rows = lines(tape.text), kinds = classify(rows)
        var start = 0, end = rows.count
        while start < end && kinds[start] == .blank { start += 1 }
        while end > start && kinds[end - 1] == .blank { end -= 1 }
        var output: [String] = [], run = 0
        for i in start..<end {
            if kinds[i] == .blank { run += 1; continue }
            if run > 0 { output += Array(repeating: "", count: run >= 3 ? 1 : run); run = 0 }
            output.append(rows[i].text)
        }
        tape.apply(differences(tape.text, output.joined(separator: "\n"), offset: 0), rule: "blankLines")
    }
}
