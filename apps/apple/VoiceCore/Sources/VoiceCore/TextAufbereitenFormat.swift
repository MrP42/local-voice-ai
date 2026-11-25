import Foundation

/// Typografische Regelstufe aus text_aufbereiten/format.rs, getrennt von PDF-Bereinigung.
enum TextAufbereitenFormat {
    typealias Tape = TextAufbereiten.Tape
    typealias Edit = TextAufbereiten.Edit
    // Vereinigung mit aufbereitung::geschuetzte_bereiche: auch äußere <…<…> schützen.
    // Lookahead erhält überlappende Marken, etwa <a[b>c], vor der Vereinigung.
    static let marks = TextAufbereiten.regex(#"(?=(\{\{S\.[^\S\r\n]*[0-9]+[^\S\r\n]*\}\}|\[[^\]\r\n]{0,80}\]|<[^>\r\n]{1,40}>|(?<![0-9])[0-9]{2}:[0-9]{2}(?::[0-9]{2})?(?:[.,][0-9]{1,3})?(?![0-9])))"#)

    static func protected(_ text: String) -> [NSRange] {
        let rows = TextAufbereitenPaste.lines(text)
        let markRanges = marks.matches(in: text, range: NSRange(location: 0, length: text.utf16.count)).map { $0.range(at: 1) }
        var codeRanges: [NSRange] = []
        var fence: (Character, Int)?
        // Inline-Code: nur ein gleich breiter schließender Backtick-Lauf beendet ihn.
        for i in rows.indices {
            let row = rows[i]
            let head = row.text.drop(while: { $0 == " " || $0 == "\t" })
            let first = head.first
            let width = first.map { char in head.prefix(while: { $0 == char }).count } ?? 0
            if let current = fence {
                codeRanges.append(NSRange(location: row.offset, length: row.length + (i + 1 < rows.count ? 1 : 0)))
                if first == current.0, width >= current.1,
                   head.dropFirst(width).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { fence = nil }
                continue
            }
            if (first == "`" || first == "~") && width >= 3 {
                fence = (first!, width)
                codeRanges.append(NSRange(location: row.offset, length: row.length + (i + 1 < rows.count ? 1 : 0))); continue
            }
            let scalars = Array(row.text.unicodeScalars)
            var offsets = [0]
            for scalar in scalars { offsets.append(offsets.last! + scalar.utf16.count) }
            var at = 0, segment = 0, runs: [(start: Int, end: Int, segment: Int)] = []
            while at < scalars.count {
                if scalars[at] == "\r" { segment += 1 }
                guard scalars[at] == "`" else { at += 1; continue }
                let start = at
                while at < scalars.count && scalars[at] == "`" { at += 1 }
                runs.append((start, at, segment))
            }
            var nextByWidth: [Int: Int] = [:], closing: [Int: Int] = [:]
            for i in runs.indices.reversed() {
                if runs[i].segment != segment { nextByWidth = [:]; segment = runs[i].segment }
                let width = runs[i].end - runs[i].start
                closing[i] = nextByWidth[width]; nextByWidth[width] = i
            }
            var run = 0
            while run < runs.count {
                if let close = closing[run] {
                    let start = offsets[runs[run].start], end = offsets[runs[close].end]
                    codeRanges.append(NSRange(location: row.offset + start, length: end - start))
                    run = close + 1
                } else { run += 1 }
            }
        }
        // Beide Eingangslisten sind geordnet; kein Sortieren eines langen Buchs.
        var ranges: [NSRange] = [], m = 0, c = 0
        while m < markRanges.count || c < codeRanges.count {
            if c == codeRanges.count || (m < markRanges.count && markRanges[m].location <= codeRanges[c].location) {
                ranges.append(markRanges[m]); m += 1
            } else {
                ranges.append(codeRanges[c]); c += 1
            }
        }
        var merged: [NSRange] = []
        for range in ranges {
            if let last = merged.last, range.location <= NSMaxRange(last) {
                merged[merged.count - 1] = NSRange(location: last.location, length: max(NSMaxRange(range), NSMaxRange(last)) - last.location)
            } else { merged.append(range) }
        }
        return merged
    }

    static func apply(_ tape: inout Tape, edits: [Edit], rule: String) {
        tape.apply(edits, rule: rule)
    }
    static func replace(_ tape: inout Tape, pattern: String, replacement: String, rule: String) {
        let source = tape.text, ns = source as NSString, regex = TextAufbereiten.regex(pattern)
        let edits = regex.matches(in: source, range: NSRange(location: 0, length: ns.length)).compactMap { match -> Edit? in
            let next = regex.replacementString(for: match, in: source, offset: 0, template: replacement)
            return TextAufbereiten.identisch(ns.substring(with: match.range), next) ? nil : Edit(match.range, next)
        }
        apply(&tape, edits: edits, rule: rule)
    }
    static func nfc(_ tape: inout Tape, tokens: NSRegularExpression) {
        let text = tape.text, ns = text as NSString
        var boundaries: Set<Int> = [0], offset = 0, edits: [Edit] = []
        for char in text { offset += String(char).utf16.count; boundaries.insert(offset) }
        var start = 0
        func normalize(until end: Int) {
            // Ein kombinierendes Zeichen kann noch zum geschützten Vorgänger gehören.
            // Dann den gesamten Abschnitt unverändert lassen, auch nach dem Trennstrichlauf.
            guard boundaries.contains(start), boundaries.contains(end) else { return }
            var offset = start
            for char in ns.substring(with: NSRange(location: start, length: end - start)) {
                let old = String(char), new = old.precomposedStringWithCanonicalMapping
                if !TextAufbereiten.identisch(old, new) { edits.append(Edit(NSRange(location: offset, length: old.utf16.count), new)) }
                offset += old.utf16.count
            }
        }
        for token in tokens.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            normalize(until: token.range.location)
            start = NSMaxRange(token.range)
        }
        normalize(until: ns.length)
        apply(&tape, edits: edits, rule: "nfc")
    }

    /// Echte Platzhalter vor dem ersten Regellauf; die Tabelle bildet UTF-16 auf die Basis ab.
    /// Nur Änderungen außerhalb der Platzhalter werden ins ursprüngliche Herkunftsband übernommen.
    static func run(_ original: inout Tape) {
        let source = original.text, ns = source as NSString
        var longest = 0, run = 0
        for scalar in source.unicodeScalars {
            run = scalar == "\u{e000}" ? run + 1 : 0
            longest = max(longest, run)
        }
        let prefix = String(repeating: "\u{e000}", count: longest + 1)
        var parts: [String] = [], positions = [0], at = 0, number = 0
        func visible(_ end: Int) {
            guard end > at else { return }
            parts.append(ns.substring(with: NSRange(location: at, length: end - at)))
            positions.append(contentsOf: (at + 1)...end)
            at = end
        }
        for range in protected(source) where range.length > 0 {
            visible(range.location)
            for row in TextAufbereitenPaste.lines(ns.substring(with: range)) {
                // split_inclusive('\n') erzeugt nach abschließendem LF keine weitere Marke.
                if row.offset == range.length { break }
                let end = range.location + row.offset + row.length
                var contentEnd = end
                while contentEnd > at && (ns.character(at: contentEnd - 1) == 13 || ns.character(at: contentEnd - 1) == 10) { contentEnd -= 1 }
                let index = String(number).unicodeScalars.map { String(Unicode.Scalar(0xe010 + $0.value - 48)!) }.joined()
                let token = prefix + index + "\u{e001}"
                parts.append(token)
                positions.append(contentsOf: repeatElement(-1, count: token.utf16.count - 1))
                positions.append(contentEnd)
                at = contentEnd; number += 1
                visible(min(NSMaxRange(range), end + 1))
            }
        }
        visible(ns.length)
        var tape = Tape(parts.joined(), scalarBased: true)
        let tokens = TextAufbereiten.regex(prefix + "[\u{e010}-\u{e019}]+\u{e001}")
        rules(&tape, tokens: tokens)
        let edits = tape.pieces.compactMap { piece -> Edit? in
            guard !TextAufbereiten.identisch(piece.alt, piece.text) else { return nil }
            guard piece.range.location >= 0, NSMaxRange(piece.range) < positions.count else { return nil }
            let start = positions[piece.range.location], end = positions[NSMaxRange(piece.range)]
            // Unerwartete Masken- oder Zeichengrenzen verwerfen, nie auf Nutzertext abbrechen.
            guard start >= 0, end >= start, end <= ns.length else { return nil }
            return Edit(NSRange(location: start, length: end - start), piece.text, rules: piece.rules)
        }
        original.apply(edits, rule: "format")
    }

    static func rules(_ tape: inout Tape, tokens: NSRegularExpression) {
        nfc(&tape, tokens: tokens)
        let ligatures = ["ﬀ": "ff", "ﬁ": "fi", "ﬂ": "fl", "ﬃ": "ffi", "ﬄ": "ffl", "ﬅ": "st", "ﬆ": "st"]
        let source = tape.text, ns = source as NSString
        let edits = TextAufbereiten.regex("[ﬀﬁﬂﬃﬄﬅﬆ]").matches(in: source, range: NSRange(location: 0, length: source.utf16.count)).map {
            Edit($0.range, ligatures[ns.substring(with: $0.range)]!)
        }
        apply(&tape, edits: edits, rule: "ligaturen")
        replace(&tape, pattern: "\u{ad}", replacement: "", rule: "weiche_trennstriche")
        nfc(&tape, tokens: tokens)
        spaces(&tape)
        lists(&tape)
        quotes(&tape)
        dashes(&tape)
        replace(&tape, pattern: #"(?<!\.)\.{3}(?!\.)"#, replacement: "…", rule: "auslassung")
    }

    static func spaces(_ tape: inout Tape) {
        let rows = TextAufbereitenPaste.lines(tape.text)
        let whitespace = TextAufbereiten.regex("[ \t ]+")
        var edits: [Edit] = []
        for (i, row) in rows.enumerated() where !row.text.contains("|") {
            let ns = row.text as NSString
            // CR gehört bei CRLF zum Zeilenende, nicht zum Inhalt.
            let content = row.text.hasSuffix("\r") ? String(row.text.dropLast()) : row.text
            let contentNS = content as NSString
            func scalar(at position: Int) -> Unicode.Scalar? {
                guard position >= 0 && position < contentNS.length else { return nil }
                let unit = contentNS.character(at: position)
                if (0xD800...0xDBFF).contains(unit), position + 1 < contentNS.length {
                    return contentNS.substring(with: NSRange(location: position, length: 2)).unicodeScalars.first
                }
                if (0xDC00...0xDFFF).contains(unit), position > 0 {
                    return contentNS.substring(with: NSRange(location: position - 1, length: 2)).unicodeScalars.first
                }
                return Unicode.Scalar(unit)
            }
            for m in whitespace.matches(in: content, range: NSRange(location: 0, length: contentNS.length)) {
                let old = ns.substring(with: m.range), end = NSMaxRange(m.range)
                let previous = scalar(at: m.range.location - 1), next = scalar(at: end)
                let listSpace = m.range.location <= 6 && TextAufbereiten.matches(contentNS.substring(to: m.range.location), #"^ {0,3}[-•·‣◦▪●]$"#)
                let punctuation = !listSpace && next.map { ".,;:!?".unicodeScalars.contains($0) } == true
                    && (scalar(at: end + 1).map { CharacterSet.whitespacesAndNewlines.contains($0) } ?? true)
                var afterColon = end + 1
                if next == ":" {
                    while let s = scalar(at: afterColon), CharacterSet.whitespacesAndNewlines.contains(s) { afterColon += s.utf16.count }
                }
                let ratio = next == ":" && previous.map { (48...57).contains($0.value) } == true
                    && scalar(at: afterColon).map { (48...57).contains($0.value) } == true
                let unit = old == " " && previous.map { (48...57).contains($0.value) } == true
                    && next.map { CharacterSet.letters.contains($0) } == true
                let hardBreak = end == contentNS.length && i + 1 < rows.count - (rows.last?.text.isEmpty == true ? 1 : 0)
                    && !contentNS.substring(to: m.range.location).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && old.count >= 2 && old.allSatisfy { $0 == " " }
                let new: String
                if old.contains("\t") || unit || hardBreak { new = old }
                else if end == contentNS.length { new = "" }
                else if m.range.location == 0 { new = old }
                else if punctuation && !ratio { new = "" }
                else { new = " " }
                if !TextAufbereiten.identisch(old, new) {
                    edits.append(Edit(NSRange(location: row.offset + m.range.location, length: m.range.length), new))
                }
            }
        }
        apply(&tape, edits: edits, rule: "leerzeichen")
    }

    static func lists(_ tape: inout Tape) {
        let rows = TextAufbereitenPaste.lines(tape.text)
        let numbered = TextAufbereiten.regex(#"^ {0,3}([0-9]+)\) (?=[\s\S]*\S)([^\n]*)$"#)
        let bullet = TextAufbereiten.regex(#"^( {0,3})[•·‣◦▪●][ \t][ \t ]*(?=\S)"#)
        let numbers: [NSTextCheckingResult?] = rows.map { row in
            guard !row.text.contains("|") else { return nil }
            return numbered.firstMatch(in: row.text, range: NSRange(location: 0, length: row.length))
        }
        func ascending(_ a: Int, _ b: Int) -> Bool {
            guard let left = numbers[a], let right = numbers[b] else { return false }
            let x = Array((rows[a].text as NSString).substring(with: left.range(at: 1)).utf8.reversed())
            let y = Array((rows[b].text as NSString).substring(with: right.range(at: 1)).utf8.reversed())
            var carry: UInt8 = 1
            for (i, digit) in y.enumerated() {
                let sum = (i < x.count ? x[i] - 48 : 0) + carry
                if digit - 48 != sum % 10 { return false }
                carry = sum / 10
            }
            return carry == 0 && x.dropFirst(y.count).allSatisfy { $0 == 48 }
        }
        var edits: [Edit] = []
        for i in rows.indices {
            let row = rows[i]
            if let number = numbers[i], (i > 0 && ascending(i - 1, i)) || (i + 1 < rows.count && ascending(i, i + 1)) {
                edits.append(Edit(NSRange(location: row.offset + NSMaxRange(number.range(at: 1)), length: 1), "."))
            } else if !row.text.contains("|"), let m = bullet.firstMatch(in: row.text, range: NSRange(location: 0, length: row.length)) {
                let indent = m.range(at: 1).length
                edits.append(Edit(NSRange(location: row.offset + indent, length: m.range.length - indent), "- "))
            }
        }
        apply(&tape, edits: edits, rule: "listen")
    }

    static func quotes(_ tape: inout Tape) {
        let text = tape.text, rows = TextAufbereitenPaste.lines(text)
        var edits: [Edit] = [], paragraph: [Edit] = [], open = false, valid = true, previous: Unicode.Scalar?
        func flush() {
            if !open && valid { edits.append(contentsOf: paragraph) }
            paragraph = []; open = false; valid = true; previous = nil
        }
        for row in rows {
            if row.trimmed.isEmpty { flush(); continue }
            var offset = row.offset
            for scalar in row.text.unicodeScalars {
                if scalar == "\"" {
                    let opening = previous.map { CharacterSet.whitespacesAndNewlines.contains($0) || "([*_".unicodeScalars.contains($0) } ?? true
                    if opening == open { valid = false }
                    paragraph.append(Edit(NSRange(location: offset, length: 1), opening ? "„" : "“"))
                    open = opening
                }
                previous = scalar
                offset += scalar.utf16.count
            }
            previous = "\n"
        }
        flush()
        tape.apply(edits, rule: "anfuehrungszeichen")
    }

    static func dashes(_ tape: inout Tape) {
        let text = tape.text, scalars = Array(text.unicodeScalars)
        var right = [Unicode.Scalar?](repeating: nil, count: scalars.count), next: Unicode.Scalar?
        for i in scalars.indices.reversed() {
            right[i] = next
            if !CharacterSet.whitespacesAndNewlines.contains(scalars[i]) { next = scalars[i] }
        }
        var edits: [Edit] = [], index = 0, offset = 0, left: Unicode.Scalar?
        for content in text.components(separatedBy: CharacterSet(charactersIn: "\r\n")) {
            let blocked = content.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix(">") || content.contains("|") || content.unicodeScalars.allSatisfy { CharacterSet.whitespacesAndNewlines.contains($0) || "-*_".unicodeScalars.contains($0) }
            var atStart = true
            for s in content.unicodeScalars {
                if s == "-", index > 0, index + 1 < scalars.count, scalars[index - 1] == " ", scalars[index + 1] == " ", !atStart, !blocked,
                   !(left.map { (48...57).contains($0.value) } == true && right[index].map { (48...57).contains($0.value) } == true) {
                    edits.append(Edit(NSRange(location: offset, length: 1), "–"))
                }
                atStart = atStart && (CharacterSet.whitespacesAndNewlines.contains(s) || s == ">" || s == "|")
                if !CharacterSet.whitespacesAndNewlines.contains(s) { left = s }
                offset += s.utf16.count; index += 1
            }
            // components(separatedBy:) behält die letzte leere Zeile.
            if index < scalars.count { index += 1; offset += 1 }
        }
        apply(&tape, edits: edits, rule: "gedankenstrich")
    }
}
