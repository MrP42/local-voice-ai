import Foundation

/// Feste Regeln, ohne Modell. Bereiche beziehen sich immer auf die unveränderte Basis.
public enum TextAufbereiten {
    private static let regelReihenfolge = ["repeated", "control", "pageNumbers", "webNoise", "fragments", "spaces", "blankLines",
                                           "nfc", "ligaturen", "weiche_trennstriche", "anfuehrungszeichen", "gedankenstrich", "auslassung", "leerzeichen", "listen"]
    public enum Stufe: Sendable { case bereinigung, format }
    public enum Fehler: Error { case veraltet, auswahl }

    public struct Aenderung: Equatable, Sendable, Identifiable {
        public let id: String
        /// Halboffener UTF-16-Bereich an erweiterten Unicode-Zeichengrenzen.
        public let bereich: NSRange
        public let alt: String
        public let neu: String
        public let regel: String
        public internal(set) var gruppe: String?
    }

    public struct Vorschlaege: Equatable, Sendable {
        public let basis: String
        public let aenderungen: [Aenderung]

        public func auswahl(angenommen: Set<String>) -> Auswahl {
            Auswahl(vorschlaege: self, angenommen: angenommen)
        }

        /// Offene und abgelehnte Vorschläge bleiben unverändert. Kein erneuter Regellauf.
        public func anwenden(angenommen: Set<String>, auf text: String) throws -> String {
            guard identisch(basis, text) else { throw Fehler.veraltet }
            guard angenommen.isSubset(of: Set(aenderungen.map(\.id))) else { throw Fehler.auswahl }
            let groups = Dictionary(grouping: aenderungen.filter { $0.gruppe != nil }, by: { $0.gruppe! })
            for group in groups.values {
                let count = group.filter { angenommen.contains($0.id) }.count
                guard count == 0 || count == group.count else { throw Fehler.auswahl }
            }
            let source = basis as NSString
            var parts: [String] = [], end = 0
            for change in aenderungen where angenommen.contains(change.id) {
                parts.append(source.substring(with: NSRange(location: end, length: change.bereich.location - end)))
                parts.append(change.neu)
                end = NSMaxRange(change.bereich)
            }
            parts.append(source.substring(from: end))
            return parts.joined()
        }
    }

    /// Die Auswahl trägt ihren eigenen Textstand; parallele Entwürfe ersetzen sich nicht.
    public struct Auswahl: Sendable {
        public let vorschlaege: Vorschlaege
        public let angenommen: Set<String>
        public func anwenden(auf text: String) throws -> String {
            try vorschlaege.anwenden(angenommen: angenommen, auf: text)
        }
    }

    /// Swift-Stringgleichheit ist NFC-unabhängig; Textstände müssen bytegleich sein.
    public static func identisch(_ a: String, _ b: String) -> Bool { a.utf8.elementsEqual(b.utf8) }

    public static func vorschlaege(_ text: String, stufe: Stufe = .bereinigung) -> Vorschlaege {
        var tape = Tape(text)
        switch stufe {
        case .bereinigung: TextAufbereitenPaste.run(&tape)
        case .format: TextAufbereitenFormat.run(&tape)
        }
        // Fällt ein Muster aus, wären auch Schutzmuster (Marken, Satzenden, Aufzählungen) wirkungslos und die
        // Vorschläge könnten Text beschädigen. Dann lieber gar keine Vorschläge als falsche.
        guard ungueltigeMuster.isEmpty else { return Vorschlaege(basis: text, aenderungen: []) }
        var counts: [String: Int] = [:]
        var changedPieces: [Piece] = []
        for var piece in tape.pieces where !identisch(piece.alt, piece.text) {
            if stufe == .format {
                // Wie aenderungen.rs: gemeinsame Ränder aussparen, Zeichengrenzen erhalten.
                let old = piece.alt.map(String.init), new = piece.text.map(String.init)
                var prefix = 0, suffix = 0
                while prefix < min(old.count, new.count) && identisch(old[prefix], new[prefix]) { prefix += 1 }
                while suffix < min(old.count, new.count) - prefix
                    && identisch(old[old.count - 1 - suffix], new[new.count - 1 - suffix]) { suffix += 1 }
                let skipped = old.prefix(prefix).joined().utf16.count
                piece.alt = old[prefix..<(old.count - suffix)].joined()
                piece.text = new[prefix..<(new.count - suffix)].joined()
                piece.range = NSRange(location: piece.range.location + skipped, length: piece.alt.utf16.count)
                // Der Desktop ordnet isolierte Striche ohne Kontext der Auffangregel zu.
                if piece.alt == "-" && piece.text == "–" { piece.rules = ["format"] }
                let bullet = piece.alt.trimmingCharacters(in: CharacterSet(charactersIn: " \t "))
                if piece.rules.contains("listen"), ["•", "·", "‣", "◦", "▪", "●"].contains(bullet),
                   piece.text == "-" || piece.text == "- " { piece.rules = ["listen"] }
                // NFC nach entferntem Trennstrich zählt nur mit, wenn die Einzelprobe greift.
                if identisch(piece.alt, piece.alt.precomposedStringWithCanonicalMapping) { piece.rules.remove("nfc") }
            }
            if let last = changedPieces.last, last.range.length == 0, last.range.location == piece.range.location {
                var merged = last
                merged.range = piece.range; merged.alt += piece.alt; merged.text += piece.text
                merged.rules.formUnion(piece.rules)
                changedPieces[changedPieces.count - 1] = merged
            } else { changedPieces.append(piece) }
        }
        var changes: [Aenderung] = changedPieces.map { piece in
            let rule = piece.rules.contains("format") ? "format" : regelReihenfolge.filter { piece.rules.contains($0) }.joined(separator: "+")
            counts[rule, default: 0] += 1
            return Aenderung(id: "\(rule)-\(counts[rule]!)", bereich: piece.range,
                             alt: piece.alt, neu: piece.text, regel: rule, gruppe: nil)
        }
        var opening: Int?, groupNumber = 0
        for i in changes.indices where changes[i].regel.contains("anfuehrungszeichen") {
            for scalar in changes[i].neu.unicodeScalars {
                if scalar == "„" { opening = i }
                else if scalar == "“", let first = opening {
                    groupNumber += 1
                    let group = "anfuehrungszeichen#\(groupNumber)"
                    changes[first].gruppe = group; changes[i].gruppe = group; opening = nil
                }
            }
        }
        if stufe == .format {
            var groups: [String: Int] = [:]
            for i in changes.indices where changes[i].gruppe == nil {
                let rule = changes[i].regel
                groups[rule, default: 0] += 1
                changes[i].gruppe = "\(rule)#\(groups[rule]!)"
            }
        }
        return Vorschlaege(basis: text, aenderungen: changes)
    }

    struct Edit {
        let range: NSRange
        let text: String
        let rules: Set<String>?
        init(_ range: NSRange, _ text: String, rules: Set<String>? = nil) {
            self.range = range; self.text = text; self.rules = rules
        }
    }

    struct Piece {
        var range: NSRange
        var alt: String
        var text: String
        var rules: Set<String> = []
    }

    /// Herkunftsband statt globalem Textdiff: jeder Durchlauf liest und schreibt linear.
    /// Zusammenhängende Regeln auf derselben Stelle werden zu einem Vorschlag vereinigt.
    /// Ein Vorschlag teilt weder Emoji-Folgen noch kombinierende Zeichen oder CRLF.
    struct Tape {
        var pieces: [Piece]
        var text: String { pieces.map(\.text).joined() }
        init(_ text: String, scalarBased: Bool = false) {
            var offset = 0
            let units = scalarBased ? text.unicodeScalars.map(String.init) : text.map(String.init)
            pieces = units.map { s in
                let length = s.utf16.count
                defer { offset += length }
                return Piece(range: NSRange(location: offset, length: length), alt: s, text: s)
            }
        }

        mutating func apply(_ edits: [Edit], rule: String) {
            guard !edits.isEmpty else { return }
            var output: [Piece] = [], p = 0, e = 0, offset = 0
            output.reserveCapacity(pieces.count)
            while e < edits.count {
                let first = edits[e]
                while p < pieces.count && offset + pieces[p].text.utf16.count <= first.range.location {
                    output.append(pieces[p]); offset += pieces[p].text.utf16.count; p += 1
                }
                if first.range.length == 0 && offset == first.range.location {
                    let originalOffset = p < pieces.count ? pieces[p].range.location : pieces.last.map { NSMaxRange($0.range) } ?? 0
                    output.append(Piece(range: NSRange(location: originalOffset, length: 0), alt: "", text: first.text, rules: first.rules ?? [rule]))
                    e += 1; continue
                }
                guard p < pieces.count else { break }
                let start = p, startOffset = offset
                var end = NSMaxRange(first.range), next = e + 1
                repeat {
                    while p < pieces.count && offset < end {
                        offset += pieces[p].text.utf16.count; p += 1
                    }
                    while next < edits.count && edits[next].range.location < offset {
                        end = max(end, NSMaxRange(edits[next].range)); next += 1
                    }
                } while offset < end
                let group = pieces[start..<p]
                let current = group.map(\.text).joined() as NSString
                var parts: [String] = [], at = 0
                for edit in edits[e..<next] {
                    let local = edit.range.location - startOffset
                    parts.append(current.substring(with: NSRange(location: at, length: local - at)))
                    parts.append(edit.text)
                    at = local + edit.range.length
                }
                parts.append(current.substring(from: at))
                let last = pieces[p - 1]
                var rules: Set<String> = []
                for edit in edits[e..<next] { rules.formUnion(edit.rules ?? [rule]) }
                for piece in group { rules.formUnion(piece.rules) }
                output.append(Piece(range: NSRange(location: pieces[start].range.location,
                                                   length: NSMaxRange(last.range) - pieces[start].range.location),
                                    alt: group.map(\.alt).joined(), text: parts.joined(), rules: rules))
                e = next
            }
            output.append(contentsOf: pieces[p...])
            pieces = output
        }

        mutating func replace(_ pattern: String, _ replacement: String, rule: String) {
            let source = text
            let ns = source as NSString
            let regex = TextAufbereiten.regex(pattern)
            let edits = regex.matches(in: source, range: NSRange(source.startIndex..., in: source)).compactMap { match -> Edit? in
                let next = regex.replacementString(for: match, in: source, offset: 0, template: replacement)
                guard !identisch(ns.substring(with: match.range), next) else { return nil }
                return Edit(match.range, next)
            }
            apply(edits, rule: rule)
        }
    }

    /// Muster, das nie trifft: Ersatz, falls ein internes Muster ungültig ist (ICU ist strenger als JS/Rust).
    static let nieTreffend = try! NSRegularExpression(pattern: "(?!)")

    static func regex(_ pattern: String) -> NSRegularExpression {
        // Ausschließlich interne, konstante Muster; keine Benutzereingabe. Ein ungültiges Muster darf die
        // App nicht beenden (lva#915: SIGILL beim Öffnen von „Text aufbereiten“); die Regel fällt dann aus,
        // und der Test `testAlleMusterSindGueltig` macht es sichtbar.
        do { return try NSRegularExpression(pattern: pattern) } catch {
            musterSperre.lock(); defer { musterSperre.unlock() }
            _ungueltigeMuster.append(pattern)
            return nieTreffend
        }
    }
    private static let musterSperre = NSLock()
    nonisolated(unsafe) private static var _ungueltigeMuster: [String] = []
    static var ungueltigeMuster: [String] {
        musterSperre.lock(); defer { musterSperre.unlock() }
        return _ungueltigeMuster
    }
    static func matches(_ text: String, _ pattern: String) -> Bool {
        regex(pattern).firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
    }
    static func replacing(_ text: String, _ pattern: String, _ replacement: String) -> String {
        regex(pattern).stringByReplacingMatches(in: text, range: NSRange(text.startIndex..., in: text), withTemplate: replacement)
    }
}
