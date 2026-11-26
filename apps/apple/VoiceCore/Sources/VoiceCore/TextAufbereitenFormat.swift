import Foundation

/// Typografische Regelstufe aus text_aufbereiten/format.rs, getrennt von PDF-Bereinigung.
enum TextAufbereitenFormat {
    typealias Tape = TextAufbereiten.Tape
    typealias Edit = TextAufbereiten.Edit
    /// Feste Ersatztexte als UTF-16-Felder: die Änderungen einer Regel teilen sich eines.
    enum Replacement {
        static let dash: [UInt16] = [0x2013], ellipsis: [UInt16] = [0x2026], space: [UInt16] = [0x20], empty: [UInt16] = []
        static let period: [UInt16] = [0x2E], bullet: [UInt16] = [0x2D, 0x20]
        static let ff: [UInt16] = [0x66, 0x66], fi: [UInt16] = [0x66, 0x69], fl: [UInt16] = [0x66, 0x6C]
        static let ffi: [UInt16] = [0x66, 0x66, 0x69], ffl: [UInt16] = [0x66, 0x66, 0x6C], st: [UInt16] = [0x73, 0x74]
    }
    // Vereinigung mit aufbereitung::geschuetzte_bereiche: auch äußere <…<…> schützen.
    // Lookahead erhält überlappende Marken, etwa <a[b>c], vor der Vereinigung.
    // `marks` ist die Vorlage (Desktop-Muster, vom Test auf Gültigkeit geprüft); gesucht wird mit `markTrigger` und
    // `markBody`, die es zerlegen: ein Lookahead an jeder der 290 000 Stellen eines langen Buchs kostet in ICU Sekunden.
    static let marks = TextAufbereiten.regex(#"(?=(\{\{S\.[^\S\r\n]*[0-9]+[^\S\r\n]*\}\}|\[[^\]\r\n]{0,80}\]|<[^>\r\n]{1,40}>|(?<![0-9])[0-9]{2}:[0-9]{2}(?::[0-9]{2})?(?:[.,][0-9]{1,3})?(?![0-9])))"#)

    /// Stellen, an denen eine Marke beginnen kann: `{`, `[`, `<` oder eine Uhrzeit (zwei Ziffern und `:`).
    static let markTrigger = TextAufbereiten.regex(#"[\{\[<]|(?<![0-9])(?=[0-9]{2}:)[0-9]"#)
    /// Die Marke selbst, ohne den Lookahead von `marks`.
    static let markBody = TextAufbereiten.regex(#"\{\{S\.[^\S\r\n]*[0-9]+[^\S\r\n]*\}\}|\[[^\]\r\n]{0,80}\]|<[^>\r\n]{1,40}>|(?<![0-9])[0-9]{2}:[0-9]{2}(?::[0-9]{2})?(?:[.,][0-9]{1,3})?(?![0-9])"#)

    static func protected(_ text: String) -> [NSRange] {
        let units = Array(text.utf16)
        return protected(ns: NSString(characters: units, length: units.count))
    }

    /// Marken (Seitenmarken, `[…]`, `<…>`, Uhrzeiten) an jeder Stelle, an der eine beginnt; sie dürfen überlappen.
    /// Anfangszeichen einmal suchen, die Marke nur dort verankert prüfen. `ns` ist UTF-16-gestützt.
    static func markRanges(ns: NSString) -> [NSRange] {
        let text = ns as String, length = ns.length
        var ranges: [NSRange] = []
        markTrigger.enumerateMatches(in: text, range: NSRange(location: 0, length: length)) { match, _, _ in
            guard let match else { return }
            let from = match.range.location
            if let found = markBody.firstMatch(in: text, options: [.anchored, .withTransparentBounds], range: NSRange(location: from, length: length - from)) {
                ranges.append(found.range)
            }
        }
        return ranges
    }

    /// Geschützte Bereiche: Marken und Code (Zäune, Inline-Code), vereinigt und geordnet.
    static func protected(ns: NSString) -> [NSRange] {
        let text = ns as String
        let markRanges = markRanges(ns: ns)
        var codeRanges: [NSRange] = []
        // Ohne Backtick und Tilde gibt es weder Zaun noch Inline-Code: die zeilenweise Prüfung entfällt.
        let rows = ns.range(of: "`", options: .literal).location == NSNotFound && ns.range(of: "~", options: .literal).location == NSNotFound
            ? [] : TextAufbereitenPaste.lines(text)
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
    @discardableResult
    static func replace(_ tape: inout Tape, pattern: String, replacement: String, rule: String) -> Bool {
        let before = tape.revision
        tape.replace(pattern, replacement, rule: rule)
        return tape.revision != before
    }
    static func nfc(_ tape: inout Tape, tokens: NSRegularExpression) {
        let ns = tape.ns, text = tape.text, length = ns.length
        // Ist schon der ganze Text (bytegleich) NFC, kann auch keine Einzelnormalisierung etwas ändern.
        if ns.isEqual(to: ns.precomposedStringWithCanonicalMapping) { return }
        // Ein kombinierendes Zeichen kann noch zum geschützten Vorgänger gehören (Marke + U+0301 sind ein Zeichen).
        func boundary(_ offset: Int) -> Bool {
            if offset == 0 || offset == length { return true }
            let index = String.Index(utf16Offset: offset, in: text)
            return text.index(before: text.index(after: index)) == index
        }
        var edits: [Edit] = [], start = 0
        func normalize(until end: Int) {
            // Dann den gesamten Abschnitt unverändert lassen, auch nach dem Trennstrichlauf.
            guard boundary(start), boundary(end) else { return }
            var offset = start
            for char in ns.substring(with: NSRange(location: start, length: end - start)) {
                let old = String(char), new = old.precomposedStringWithCanonicalMapping
                if !TextAufbereiten.identisch(old, new) { edits.append(Edit(NSRange(location: offset, length: old.utf16.count), new)) }
                offset += old.utf16.count
            }
        }
        for token in tokens.matches(in: ns as String, range: NSRange(location: 0, length: length)) {
            normalize(until: token.range.location)
            start = NSMaxRange(token.range)
        }
        normalize(until: length)
        apply(&tape, edits: edits, rule: "nfc")
    }

    /// Echte Platzhalter vor dem ersten Regellauf; die Tabelle bildet UTF-16 auf die Basis ab.
    /// Nur Änderungen außerhalb der Platzhalter werden ins ursprüngliche Herkunftsband übernommen.
    static func run(_ original: inout Tape) {
        let ns = original.ns, source = original.text
        // Ein einzelnes, im Text nicht vorkommendes privates Zeichen: feste Länge unabhängig von der Eingabe.
        // Kommt jedes vor, bleibt der Text ohne Typografie-Vorschläge.
        let candidates = UInt32(0xe020)...UInt32(0xf8ff)
        var candidate: Unicode.Scalar? = Unicode.Scalar(candidates.lowerBound)
        // Normalfall: kein privates Zeichen im Text (ein Durchlauf in Foundation statt einer Skalarmenge).
        if let low = Unicode.Scalar(candidates.lowerBound), let high = Unicode.Scalar(candidates.upperBound),
           ns.rangeOfCharacter(from: CharacterSet(charactersIn: low...high)).location != NSNotFound {
            let used = Set(source.unicodeScalars)
            candidate = candidates.lazy.compactMap { Unicode.Scalar($0) }.first { !used.contains($0) }
        }
        guard let sentinel = candidate else { return }
        let prefix = String(Character(sentinel))
        // Abschnitte des maskierten Textes: sichtbarer Text (ab Dokumentstelle `doc`) oder eine Marke (Dokumentende
        // `docEnd`). Statt je Einheit eine Tabelle zu führen, wird die Dokumentstelle nachgeschlagen.
        struct Segment { let masked: Int; let length: Int; let doc: Int; let isToken: Bool; let docEnd: Int }
        var segments: [Segment] = [], parts: [String] = [], maskedLength = 0, at = 0, number = 0
        func visible(_ end: Int) {
            guard end > at else { return }
            parts.append(ns.substring(with: NSRange(location: at, length: end - at)))
            segments.append(Segment(masked: maskedLength, length: end - at, doc: at, isToken: false, docEnd: end))
            maskedLength += end - at
            at = end
        }
        for range in protected(ns: ns) where range.length > 0 {
            visible(range.location)
            for row in TextAufbereitenPaste.lines(ns.substring(with: range)) {
                // split_inclusive('\n') erzeugt nach abschließendem LF keine weitere Marke.
                if row.offset == range.length { break }
                let end = range.location + row.offset + row.length
                var contentEnd = end
                while contentEnd > at && (ns.character(at: contentEnd - 1) == 13 || ns.character(at: contentEnd - 1) == 10) { contentEnd -= 1 }
                let index = String(number).unicodeScalars.map { String(Unicode.Scalar(0xe010 + $0.value - 48)!) }.joined()
                let token = prefix + index + "\u{e001}"
                let length = token.utf16.count
                parts.append(token)
                segments.append(Segment(masked: maskedLength, length: length, doc: at, isToken: true, docEnd: contentEnd))
                maskedLength += length
                at = contentEnd; number += 1
                visible(min(NSMaxRange(range), end + 1))
            }
        }
        visible(ns.length)
        /// Dokumentstelle zu einer Stelle im maskierten Text; -1 mitten in einer Marke.
        func document(_ masked: Int) -> Int {
            guard masked > 0 else { return 0 }
            var low = 0, high = segments.count - 1
            // letzter Abschnitt, der vor `masked` beginnt
            while low < high {
                let mid = (low + high + 1) / 2
                if segments[mid].masked < masked { low = mid } else { high = mid - 1 }
            }
            let segment = segments[low]
            if segment.isToken { return masked == segment.masked + segment.length ? segment.docEnd : -1 }
            return segment.doc + masked - segment.masked
        }
        // Ohne Marken ist der maskierte Text der Text selbst: Stellen werden unverändert übernommen.
        let identity = segments.count == 1 && !segments[0].isToken && segments[0].doc == 0
        var tape = Tape(parts.joined(), scalarBased: true)
        let tokens = TextAufbereiten.regex(prefix + "[\u{e010}-\u{e019}]+\u{e001}")
        rules(&tape, tokens: tokens)
        let edits = tape.pieces.compactMap { piece -> Edit? in
            guard !TextAufbereiten.identisch(piece.alt, piece.text) else { return nil }
            guard piece.range.location >= 0, NSMaxRange(piece.range) <= maskedLength else { return nil }
            let start = identity ? piece.range.location : document(piece.range.location)
            let end = identity ? NSMaxRange(piece.range) : document(NSMaxRange(piece.range))
            // Unerwartete Masken- oder Zeichengrenzen verwerfen, nie auf Nutzertext abbrechen.
            guard start >= 0, end >= start, end <= ns.length else { return nil }
            return Edit(NSRange(location: start, length: end - start), piece.text, rules: piece.rules)
        }
        original.apply(edits, rule: "format")
    }

    static func rules(_ tape: inout Tape, tokens: NSRegularExpression) {
        nfc(&tape, tokens: tokens)
        let ligatures = ligatureEdits(tape)
        apply(&tape, edits: ligatures, rule: "ligaturen")
        let hyphens = replace(&tape, pattern: "\u{ad}", replacement: "", rule: "weiche_trennstriche")
        // Ligaturen (ffi + U+0301) und entfernte Trennstriche können neue Kompositionen freilegen; sonst ist nichts zu tun.
        if !ligatures.isEmpty || hyphens { nfc(&tape, tokens: tokens) }
        spaces(&tape)
        lists(&tape)
        quotes(&tape)
        dashes(&tape)
        apply(&tape, edits: ellipsisEdits(tape), rule: "auslassung")
    }

    /// Ligaturzeichen U+FB00 bis U+FB06 (ﬀ ﬁ ﬂ ﬃ ﬄ ﬅ ﬆ) werden aufgelöst.
    static func ligatureEdits(_ tape: Tape) -> [Edit] {
        let units = tape.units
        var edits: [Edit] = []
        units.withUnsafeBufferPointer { p in
            var i = 0
            while i < p.count {
                let unit = p[i]
                if unit >= 0xFB00 && unit <= 0xFB06 {
                    let replacement: String, units: [UInt16]
                    switch unit {
                    case 0xFB00: replacement = "ff"; units = Replacement.ff
                    case 0xFB01: replacement = "fi"; units = Replacement.fi
                    case 0xFB02: replacement = "fl"; units = Replacement.fl
                    case 0xFB03: replacement = "ffi"; units = Replacement.ffi
                    case 0xFB04: replacement = "ffl"; units = Replacement.ffl
                    default: replacement = "st"; units = Replacement.st
                    }
                    edits.append(Edit(NSRange(location: i, length: 1), replacement, units: units))
                }
                i += 1
            }
        }
        return edits
    }

    /// Genau drei Punkte in Folge (nicht Teil einer längeren Punktfolge) werden zu „…“.
    static func ellipsisEdits(_ tape: Tape) -> [Edit] {
        let units = tape.units
        var edits: [Edit] = []
        units.withUnsafeBufferPointer { p in
            var i = 0
            while i < p.count {
                guard p[i] == 0x2E else { i += 1; continue }
                var end = i + 1
                while end < p.count && p[end] == 0x2E { end += 1 }
                if end - i == 3 { edits.append(Edit(NSRange(location: i, length: 3), "…", units: Replacement.ellipsis)) }
                i = end
            }
        }
        return edits
    }

    static func spaces(_ tape: inout Tape) {
        apply(&tape, edits: spacesEdits(tape), rule: "leerzeichen")
    }

    /// Wie `leerzeichen` in format.rs, je Zeile ohne `|`. Nur Läufe, die sich ändern können, kommen in die Prüfung:
    /// zwei oder mehr Zeichen, ein einzelnes geschütztes Leerzeichen und ein einzelnes Leerzeichen vor Satzzeichen,
    /// Zeilenende oder Textende (Läufe mit Tabulator bleiben immer). Alles andere ist ein einzelnes Leerzeichen
    /// mitten in der Zeile und bleibt, ohne dass die Zeile je zerlegt werden muss.
    static func spacesEdits(_ tape: Tape) -> [Edit] {
        var edits: [Edit] = []
        tape.units.withUnsafeBufferPointer { units in
            let length = units.count
            let blanks = CharacterSet.whitespacesAndNewlines, letters = CharacterSet.letters
            // Zeile des aktuellen Treffers: Anfang, Ende (Zeilenvorschub oder Textende), Inhaltsende (ohne CR am Ende).
            var rowStart = 0, rowEnd = -1, contentEnd = 0, hasPipe = false
            func scalar(_ position: Int) -> Unicode.Scalar? {
                guard position >= rowStart && position < contentEnd else { return nil }
                let unit = units[position]
                if (0xD800...0xDBFF).contains(unit), position + 1 < contentEnd {
                    return Unicode.Scalar(0x10000 + ((UInt32(unit) - 0xD800) << 10) + (UInt32(units[position + 1]) - 0xDC00))
                }
                if (0xDC00...0xDFFF).contains(unit), position > rowStart {
                    return Unicode.Scalar(0x10000 + ((UInt32(units[position - 1]) - 0xD800) << 10) + (UInt32(unit) - 0xDC00))
                }
                return Unicode.Scalar(unit)
            }
            func isDigit(_ s: Unicode.Scalar?) -> Bool { s.map { (48...57).contains($0.value) } == true }
            func examine(_ location: Int, _ end: Int) {
                if rowEnd < 0 || location > rowEnd {
                    var start = location, floor = rowEnd < 0 ? 0 : rowEnd + 1, stop = end
                    while start > floor && units[start - 1] != 10 { start -= 1 }
                    while stop < length && units[stop] != 10 { stop += 1 }
                    rowStart = start; rowEnd = stop
                    hasPipe = false
                    var k = start
                    while k < stop { if units[k] == 0x7C { hasPipe = true; break }; k += 1 }
                    contentEnd = stop > start && units[stop - 1] == 13 ? stop - 1 : stop
                }
                if hasPipe { return }
                let relative = location - rowStart
                var onlySpaces = true, run = location
                while run < end { if units[run] != 0x20 { onlySpaces = false; break }; run += 1 }
                let single = end - location == 1
                let previous = scalar(location - 1), next = scalar(end)
                let listSpace = relative <= 6 && TextAufbereiten.matches(
                    String(decoding: units[rowStart..<location], as: UTF16.self), #"^ {0,3}[-•·‣◦▪●]$"#)
                var punctuation = false
                if !listSpace, let next {
                    switch next.value {
                    case 46, 44, 59, 58, 33, 63: punctuation = scalar(end + 1).map { blanks.contains($0) } ?? true
                    default: break
                    }
                }
                var ratio = false
                if next == ":" {
                    var afterColon = end + 1
                    while let s = scalar(afterColon), blanks.contains(s) { afterColon += s.utf16.count }
                    ratio = isDigit(previous) && isDigit(scalar(afterColon))
                }
                let unit = single && units[location] == 0xA0 && isDigit(previous) && next.map { letters.contains($0) } == true
                if unit { return }
                if end == contentEnd {
                    if single == false && onlySpaces && rowEnd + 1 < length {
                        // Harter Umbruch (zwei Leerzeichen am Zeilenende) bleibt, wenn davor Text steht.
                        var text = false
                        var k = rowStart
                        while k < location {
                            if let s = scalar(k), blanks.contains(s) { k += s.utf16.count } else { text = true; break }
                        }
                        if text { return }
                    }
                    edits.append(Edit(NSRange(location: location, length: end - location), "", units: Replacement.empty))
                } else if relative == 0 {
                    return
                } else if punctuation && !ratio {
                    edits.append(Edit(NSRange(location: location, length: end - location), "", units: Replacement.empty))
                } else if !(single && units[location] == 0x20) {
                    edits.append(Edit(NSRange(location: location, length: end - location), " ", units: Replacement.space))
                }
            }
            do {
                var i = 0
                while i < units.count {
                    let first = units[i]
                    guard first == 0x20 || first == 0x09 || first == 0xA0 else { i += 1; continue }
                    // Maximaler Lauf aus Leerzeichen, Tabulatoren und geschützten Leerzeichen.
                    var j = i + 1, hasTab = first == 0x09
                    while j < units.count {
                        let unit = units[j]
                        if unit == 0x20 || unit == 0xA0 { j += 1 } else if unit == 0x09 { hasTab = true; j += 1 } else { break }
                    }
                    if !hasTab {
                        var interesting = j - i >= 2 || first == 0xA0
                        if !interesting {
                            // Einzelnes Leerzeichen: nur vor Satzzeichen, Zeilenende oder Textende kann es sich ändern.
                            if j == units.count { interesting = true } else {
                                switch units[j] {
                                case 0x2E, 0x2C, 0x3B, 0x3A, 0x21, 0x3F, 0x0D, 0x0A: interesting = true
                                default: break
                                }
                            }
                        }
                        if interesting { examine(i, j) }
                    }
                    i = j
                }
            }
        }
        return edits
    }

    static func lists(_ tape: inout Tape) {
        apply(&tape, edits: listsEdits(tape), rule: "listen")
    }

    /// Nummerierte Zeilen mit aufsteigendem Nachbarn und Aufzählungszeichen, ohne die Zeilen einzeln zu zerlegen:
    /// Zeilenanfänge (Anfang oder nach LF) mit passendem Muster liefert ein Durchlauf in ICU, nur diese Zeilen
    /// werden geprüft. Zeilen mit `|` zählen nicht (Tabellen).
    static func listsEdits(_ tape: Tape) -> [Edit] {
        let ns = tape.ns, units = tape.units, length = units.count
        let text = ns as String, whole = NSRange(location: 0, length: length)
        let numbered = TextAufbereiten.regex(#"(?<![^\n]) {0,3}([0-9]+)\) (?=[^\n]*\S)"#)
        let bullet = TextAufbereiten.regex(#"(?<![^\n])( {0,3})[•·‣◦▪●][ \t][ \t\u00A0]*(?=\S)"#)
        func hasPipe(from start: Int) -> Bool {
            var k = start
            while k < length && units[k] != 10 { if units[k] == 0x7C { return true }; k += 1 }
            return false
        }
        // Zeilenanfang, Ziffernbereich und Zeilenende der nummerierten Zeilen, in Textreihenfolge.
        var rows: [(start: Int, end: Int, digits: NSRange)] = []
        numbered.enumerateMatches(in: text, range: whole) { match, _, _ in
            guard let match, !hasPipe(from: match.range.location) else { return }
            var end = NSMaxRange(match.range)
            while end < length && units[end] != 10 { end += 1 }
            rows.append((match.range.location, end, match.range(at: 1)))
        }
        func ascending(_ a: Int, _ b: Int) -> Bool {
            // Nur benachbarte Zeilen (genau ein LF dazwischen) zählen.
            guard rows[a].end + 1 == rows[b].start else { return false }
            let x = Array(units[rows[a].digits.location..<NSMaxRange(rows[a].digits)].reversed())
            let y = Array(units[rows[b].digits.location..<NSMaxRange(rows[b].digits)].reversed())
            var carry: UInt16 = 1
            for (i, digit) in y.enumerated() {
                let sum = (i < x.count ? x[i] - 48 : 0) + carry
                if digit - 48 != sum % 10 { return false }
                carry = sum / 10
            }
            return carry == 0 && x.dropFirst(y.count).allSatisfy { $0 == 48 }
        }
        var edits: [Edit] = []
        for i in rows.indices {
            guard (i > 0 && ascending(i - 1, i)) || (i + 1 < rows.count && ascending(i, i + 1)) else { continue }
            edits.append(Edit(NSRange(location: NSMaxRange(rows[i].digits), length: 1), ".", units: Replacement.period))
        }
        var bullets: [Edit] = []
        bullet.enumerateMatches(in: text, range: whole) { match, _, _ in
            guard let match, !hasPipe(from: match.range.location) else { return }
            let indent = match.range(at: 1).length
            bullets.append(Edit(NSRange(location: match.range.location + indent, length: match.range.length - indent), "- ", units: Replacement.bullet))
        }
        guard !bullets.isEmpty else { return edits }
        // Beide Listen sind nach Zeilenanfang geordnet.
        var merged: [Edit] = [], n = 0, b = 0
        merged.reserveCapacity(edits.count + bullets.count)
        while n < edits.count || b < bullets.count {
            if b == bullets.count || (n < edits.count && edits[n].range.location < bullets[b].range.location) { merged.append(edits[n]); n += 1 }
            else { merged.append(bullets[b]); b += 1 }
        }
        return merged
    }

    static func quotes(_ tape: inout Tape) {
        // Ohne gerades Anführungszeichen gibt es nichts zu setzen.
        // `.literal`: sonst findet die Suche kein Zeichen, dem ein kombinierendes Zeichen folgt.
        guard tape.ns.range(of: "\"", options: .literal).location != NSNotFound else { return }
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
        apply(&tape, edits: dashesEdits(tape), rule: "gedankenstrich")
    }

    /// Wie `gedankenstriche` in format.rs: ein Bindestrich zwischen zwei Leerzeichen wird Gedankenstrich, nicht am
    /// Zeilenanfang (nach Leerraum, `>` und `|`), nicht in Zitat-, Tabellen- und Trennzeilen, nicht zwischen Ziffern.
    /// Die Kandidaten liefert ein Durchlauf in ICU; nur sie werden mit ihrer Zeile geprüft.
    static func dashesEdits(_ tape: Tape) -> [Edit] {
        var edits: [Edit] = []
        tape.units.withUnsafeBufferPointer { units in
            let length = units.count
            let blanks = CharacterSet.whitespacesAndNewlines
            func blank(_ unit: UInt16) -> Bool {
                if unit < 0x80 { return unit == 0x20 || (unit >= 9 && unit <= 13) }
                return Unicode.Scalar(unit).map { blanks.contains($0) } ?? false
            }
            func digit(_ unit: UInt16) -> Bool { unit >= 0x30 && unit <= 0x39 }
            // Zeile (zwischen CR/LF) des letzten Kandidaten mit ihrem Urteil; Kandidaten laufen aufsteigend.
            var lineStart = 0, lineEnd = -1, blocked = false
            func examine(_ position: Int) {
                if position >= lineEnd {
                    var start = position, stop = position + 1
                    while start > 0 && units[start - 1] != 10 && units[start - 1] != 13 { start -= 1 }
                    while stop < length && units[stop] != 10 && units[stop] != 13 { stop += 1 }
                    lineStart = start; lineEnd = stop
                    var firstSeen = false, quote = false, pipe = false, plainRule = true, k = start
                    while k < stop {
                        let unit = units[k]
                        k += 1
                        if unit == 0x7C { pipe = true }
                        let isBlank = blank(unit)
                        if !firstSeen && !isBlank { firstSeen = true; quote = unit == 0x3E }
                        if !(isBlank || unit == 0x2D || unit == 0x2A || unit == 0x5F) { plainRule = false }
                    }
                    blocked = quote || pipe || plainRule
                }
                if blocked { return }
                var k = position - 1
                while k >= lineStart {
                    let unit = units[k]
                    if !(blank(unit) || unit == 0x3E || unit == 0x7C) { break }
                    k -= 1
                }
                // Alles davor in der Zeile ist Leerraum, `>` oder `|`: der Strich steht am Zeilenanfang.
                if k < lineStart { return }
                var left = position - 1, right = position + 1
                while left >= 0 && blank(units[left]) { left -= 1 }
                while right < length && blank(units[right]) { right += 1 }
                if left >= 0 && right < length && digit(units[left]) && digit(units[right]) { return }
                edits.append(Edit(NSRange(location: position, length: 1), "–", units: Replacement.dash))
            }
            // Kandidaten: ein Bindestrich mit je einem Leerzeichen davor und dahinter.
            do {
                var i = 1
                while i + 1 < units.count {
                    if units[i] == 0x2D && units[i - 1] == 0x20 && units[i + 1] == 0x20 { examine(i) }
                    i += 1
                }
            }
        }
        return edits
    }
}
