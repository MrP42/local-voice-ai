import Foundation

/// Feste Regeln, ohne Modell. Bereiche beziehen sich immer auf die unveränderte Basis.
public enum TextAufbereiten {
    /// Regeln als Bitmenge, in der festen Berichtsreihenfolge; „format“ steht für die Auffangregel der Formatstufe.
    /// Ein Stück trägt je Änderung die beteiligten Regeln; Mengen von Zeichenketten kosteten bei jeder Änderung eine
    /// Allokation und eine Reihe von Hash-Suchen.
    struct RuleSet: OptionSet, Hashable {
        let rawValue: UInt32
        static let reihenfolge = ["repeated", "control", "pageNumbers", "webNoise", "fragments", "spaces", "blankLines",
                                  "nfc", "ligaturen", "weiche_trennstriche", "anfuehrungszeichen", "gedankenstrich", "auslassung", "leerzeichen", "listen"]
        /// Unbekannte Namen (Proben) tragen keine Bits und erscheinen im Bericht nicht, wie bisher.
        static func named(_ name: String) -> RuleSet {
            if name == "format" { return RuleSet(rawValue: 1 << UInt32(reihenfolge.count)) }
            return reihenfolge.firstIndex(of: name).map { RuleSet(rawValue: 1 << UInt32($0)) } ?? RuleSet.none
        }
        static let none = RuleSet(rawValue: 0)
        static let format = named("format"), nfc = named("nfc"), listen = named("listen"), anfuehrungszeichen = named("anfuehrungszeichen")
        /// Berichtsname: „format“ oder die beteiligten Regeln in fester Reihenfolge, mit „+“ verbunden.
        var name: String {
            if contains(.format) { return "format" }
            return Self.reihenfolge.enumerated().filter { contains(RuleSet(rawValue: 1 << UInt32($0.offset))) }.map(\.element).joined(separator: "+")
        }
    }
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
        /// Indizes der Änderungen je Gruppe mit mindestens zwei Mitgliedern (Anführungszeichen-Paare). Einzelgruppen
        /// können nie teilweise angenommen sein und brauchen keine Prüfung.
        let gemeinsameGruppen: [[Int]]

        init(basis: String, aenderungen: [Aenderung], gemeinsameGruppen: [[Int]]? = nil) {
            self.basis = basis
            self.aenderungen = aenderungen
            self.gemeinsameGruppen = gemeinsameGruppen ?? Dictionary(grouping: aenderungen.indices.filter { aenderungen[$0].gruppe != nil },
                                                                      by: { aenderungen[$0].gruppe! }).values.filter { $0.count > 1 }.map { $0.sorted() }
        }

        public func auswahl(angenommen: Set<String>) -> Auswahl {
            Auswahl(vorschlaege: self, angenommen: angenommen)
        }

        /// Offene und abgelehnte Vorschläge bleiben unverändert. Kein erneuter Regellauf.
        public func anwenden(angenommen: Set<String>, auf text: String) throws -> String {
            guard identisch(basis, text) else { throw Fehler.veraltet }
            // Die Kennungen sind je Regel durchnummeriert, also eindeutig: wer alle angenommenen findet, hat eine Teilmenge.
            var accepted = [Bool](repeating: false, count: aenderungen.count), matched = 0, index = 0
            if !angenommen.isEmpty {
                while index < aenderungen.count {
                    if angenommen.contains(aenderungen[index].id) { accepted[index] = true; matched += 1 }
                    index += 1
                }
            }
            guard matched == angenommen.count else { throw Fehler.auswahl }
            for group in gemeinsameGruppen {
                var count = 0
                for member in group where accepted[member] { count += 1 }
                guard count == 0 || count == group.count else { throw Fehler.auswahl }
            }
            let units = Array(basis.utf16), source = NSString(characters: units, length: units.count)
            var parts: [String] = [], end = 0
            parts.reserveCapacity(matched * 2 + 1)
            index = 0
            while index < aenderungen.count {
                if accepted[index] {
                    let change = aenderungen[index]
                    parts.append(source.substring(with: NSRange(location: end, length: change.bereich.location - end)))
                    parts.append(change.neu)
                    end = NSMaxRange(change.bereich)
                }
                index += 1
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
    public static func identisch(_ a: String, _ b: String) -> Bool {
        guard a.utf8.count == b.utf8.count else { return false }
        // Zusammenhängende UTF-8-Bytes ohne Kopie vergleichen; nur für Strings ohne eigenen Speicher (selten) elementweise.
        let same = a.utf8.withContiguousStorageIfAvailable { x in
            b.utf8.withContiguousStorageIfAvailable { y in x.isEmpty || memcmp(x.baseAddress!, y.baseAddress!, x.count) == 0 }
        }
        if let same, let result = same { return result }
        return a.utf8.elementsEqual(b.utf8)
    }

    public static func vorschlaege(_ text: String, stufe: Stufe = .bereinigung) -> Vorschlaege {
        var tape = Tape(text)
        switch stufe {
        case .bereinigung: TextAufbereitenPaste.run(&tape)
        case .format: TextAufbereitenFormat.run(&tape)
        }
        // Fällt ein Muster aus, wären auch Schutzmuster (Marken, Satzenden, Aufzählungen) wirkungslos und die
        // Vorschläge könnten Text beschädigen. Dann lieber gar keine Vorschläge als falsche.
        guard ungueltigeMuster.isEmpty else { return Vorschlaege(basis: text, aenderungen: []) }
        var changedPieces: [Piece] = []
        let allPieces = tape.pieces
        var pieceIndex = 0
        while pieceIndex < allPieces.count {
            var piece = allPieces[pieceIndex]
            pieceIndex += 1
            if identisch(piece.alt, piece.text) { continue }
            if stufe == .format {
                // Wie aenderungen.rs: gemeinsame Ränder aussparen, Zeichengrenzen erhalten.
                piece.alt = piece.alt
                trimCommonEdges(&piece)
                // Der Desktop ordnet isolierte Striche ohne Kontext der Auffangregel zu.
                if piece.alt.utf8.count == 1 && piece.alt.utf8.first == 0x2D && piece.text == "–" { piece.rules = .format }
                if piece.rules.contains(.listen) {
                    let bullet = piece.alt.trimmingCharacters(in: CharacterSet(charactersIn: " \t\u{a0}"))
                    if ["•", "·", "‣", "◦", "▪", "●"].contains(bullet), piece.text == "-" || piece.text == "- " { piece.rules = .listen }
                }
                // NFC nach entferntem Trennstrich zählt nur mit, wenn die Einzelprobe greift.
                if piece.rules.contains(.nfc), identisch(piece.alt, piece.alt.precomposedStringWithCanonicalMapping) { piece.rules.remove(.nfc) }
            }
            let lastIndex = changedPieces.count - 1
            if lastIndex >= 0, changedPieces[lastIndex].range.length == 0, changedPieces[lastIndex].range.location == piece.range.location {
                var merged = changedPieces[lastIndex]
                merged.range = piece.range; merged.alt += piece.alt; merged.text += piece.text
                merged.rules.formUnion(piece.rules)
                changedPieces[lastIndex] = merged
            } else { changedPieces.append(piece) }
        }
        // Namen, Zähler und Gruppenzähler je Regelmenge einmal führen statt je Änderung zu suchen und zu bilden.
        var names: [UInt32: String] = [:], counts: [UInt32: Int] = [:], groups: [UInt32: Int] = [:]
        var quotes: [Int] = []
        var changes: [Aenderung] = []
        changes.reserveCapacity(changedPieces.count)
        for piece in changedPieces {
            let key = piece.rules.rawValue
            let rule: String
            if let known = names[key] { rule = known } else { rule = piece.rules.name; names[key] = rule }
            let number = (counts[key] ?? 0) + 1
            counts[key] = number
            if rule != "format" && piece.rules.contains(.anfuehrungszeichen) { quotes.append(changes.count) }
            changes.append(Aenderung(id: rule + "-" + String(number), bereich: piece.range,
                                     alt: piece.alt, neu: piece.text, regel: rule, gruppe: nil))
        }
        var opening: Int?, groupNumber = 0, shared: [[Int]] = []
        for i in quotes {
            for scalar in changes[i].neu.unicodeScalars {
                if scalar == "„" { opening = i }
                else if scalar == "“", let first = opening {
                    groupNumber += 1
                    let group = "anfuehrungszeichen#" + String(groupNumber)
                    changes[first].gruppe = group; changes[i].gruppe = group; opening = nil
                    shared.append(first == i ? [i] : [first, i])
                }
            }
        }
        if stufe == .format {
            for i in changes.indices where changes[i].gruppe == nil {
                let key = changedPieces[i].rules.rawValue
                let number = (groups[key] ?? 0) + 1
                groups[key] = number
                changes[i].gruppe = changes[i].regel + "#" + String(number)
            }
        }
        return Vorschlaege(basis: text, aenderungen: changes, gemeinsameGruppen: shared.filter { $0.count > 1 })
    }

    /// Gemeinsamen Anfang und Rand von `alt` und `text` abschneiden (an Zeichengrenzen).
    /// Reine Texte ohne kombinierende Zeichen, Surrogate oder CR liefern je UTF-16-Einheit ein Zeichen; nur sonst
    /// wird zeichenweise verglichen.
    static func trimCommonEdges(_ piece: inout Piece) {
        // Gleiche Zeichen am Rand haben gleiche erste bzw. letzte Einheit; sonst ist nichts abzuschneiden.
        guard piece.alt.utf8.first == piece.text.utf8.first || piece.alt.utf8.last == piece.text.utf8.last else { return }
        // Reines ASCII (ohne CR): ein Byte ist ein Zeichen, kein Umweg über UTF-16-Felder.
        enum Ascii { case no, unchanged, trimmed(prefix: Int, alt: String, text: String) }
        var ascii = Ascii.no
        _ = piece.alt.utf8.withContiguousStorageIfAvailable { a in
            piece.text.utf8.withContiguousStorageIfAvailable { b in
                var k = 0
                while k < a.count { if a[k] >= 0x80 || a[k] == 0x0D { return }; k += 1 }
                k = 0
                while k < b.count { if b[k] >= 0x80 || b[k] == 0x0D { return }; k += 1 }
                var prefix = 0, suffix = 0
                let shorter = min(a.count, b.count)
                while prefix < shorter && a[prefix] == b[prefix] { prefix += 1 }
                while suffix < shorter - prefix && a[a.count - 1 - suffix] == b[b.count - 1 - suffix] { suffix += 1 }
                if prefix == 0 && suffix == 0 { ascii = .unchanged; return }
                ascii = .trimmed(prefix: prefix, alt: String(decoding: UnsafeBufferPointer(rebasing: a[prefix..<(a.count - suffix)]), as: UTF8.self),
                                 text: String(decoding: UnsafeBufferPointer(rebasing: b[prefix..<(b.count - suffix)]), as: UTF8.self))
            }
        }
        switch ascii {
        case .unchanged: return
        case .trimmed(let prefix, let alt, let text):
            piece.alt = alt; piece.text = text
            piece.range = NSRange(location: piece.range.location + prefix, length: alt.utf8.count)
            return
        case .no: break
        }
        let a = Array(piece.alt.utf16), b = Array(piece.text.utf16)
        if a.allSatisfy(plain) && b.allSatisfy(plain) {
            var prefix = 0, suffix = 0
            let shorter = min(a.count, b.count)
            while prefix < shorter && a[prefix] == b[prefix] { prefix += 1 }
            while suffix < shorter - prefix && a[a.count - 1 - suffix] == b[b.count - 1 - suffix] { suffix += 1 }
            guard prefix > 0 || suffix > 0 else { return }
            piece.alt = String(decoding: a[prefix..<(a.count - suffix)], as: UTF16.self)
            piece.text = String(decoding: b[prefix..<(b.count - suffix)], as: UTF16.self)
            piece.range = NSRange(location: piece.range.location + prefix, length: a.count - suffix - prefix)
            return
        }
        let old = piece.alt.map(String.init), new = piece.text.map(String.init)
        var prefix = 0, suffix = 0
        while prefix < min(old.count, new.count) && identisch(old[prefix], new[prefix]) { prefix += 1 }
        while suffix < min(old.count, new.count) - prefix
            && identisch(old[old.count - 1 - suffix], new[new.count - 1 - suffix]) { suffix += 1 }
        let skipped = old.prefix(prefix).joined().utf16.count
        piece.alt = old[prefix..<(old.count - suffix)].joined()
        piece.text = new[prefix..<(new.count - suffix)].joined()
        piece.range = NSRange(location: piece.range.location + skipped, length: piece.alt.utf16.count)
    }

    /// Einheit, die allein ein Zeichen bildet und sich mit keiner Nachbareinheit verbindet: Latein bis U+02FF
    /// (ohne CR), allgemeine Satzzeichen U+2010–2027 und die Ligaturen U+FB00–FB06.
    static func plain(_ unit: UInt16) -> Bool {
        (unit < 0x300 && unit != 0x0D) || (0x2010...0x2027).contains(unit) || (0xFB00...0xFB06).contains(unit)
    }

    struct Edit {
        let range: NSRange
        let text: String
        /// UTF-16-Einheiten von `text`; Regeln mit festem Ersatzzeichen teilen sich ein Feld.
        let units: [UInt16]
        let rules: RuleSet?
        init(_ range: NSRange, _ text: String, rules: RuleSet? = nil, units: [UInt16]? = nil) {
            self.range = range; self.text = text; self.rules = rules; self.units = units ?? Array(text.utf16)
        }
    }

    struct Piece {
        var range: NSRange
        var text: String
        var rules: RuleSet
        private var storedAlt: String?
        /// Ursprungstext, aus dem `alt` bei Bedarf geschnitten wird: Stücke der Maskenstufe brauchen ihn nie.
        private let source: NSString?
        init(range: NSRange, alt: String, text: String, rules: RuleSet = RuleSet.none) {
            self.range = range; self.text = text; self.rules = rules; storedAlt = alt; source = nil
        }
        init(range: NSRange, source: NSString, text: String, rules: RuleSet) {
            self.range = range; self.text = text; self.rules = rules; storedAlt = nil; self.source = source
        }
        /// Ursprünglicher Text des Bereichs. Wer `range` ändert, liest vorher `alt` (und setzt es dann).
        var alt: String {
            get { storedAlt ?? source?.substring(with: range) ?? "" }
            set { storedAlt = newValue }
        }
        /// Aktuelle Länge in UTF-16-Einheiten.
        var length: Int { text.utf16.count }
    }

    /// Herkunftsband statt globalem Textdiff. Es führt nur die GEÄNDERTEN Stücke (und Einfügungen) mit dem
    /// ursprünglichen Bereich; alles dazwischen ist unverändert und wird nicht einzeln gehalten. So kostet ein
    /// Regellauf Zeit nach Zahl der Änderungen (plus einen linearen, im Speicher kopierenden Durchgang für den neuen
    /// Text), nicht nach Zahl der Zeichen (lva#914: vorher je Zeichen ein Stück, bei 10 000 Zeilen mehrere Sekunden).
    /// Zusammenhängende Regeln auf derselben Stelle werden zu einem Vorschlag vereinigt.
    /// Ein Vorschlag teilt weder Emoji-Folgen noch kombinierende Zeichen oder CRLF: die Grenzen der unveränderten
    /// Zeichen liefert Swift selbst (`String.index(after:)` rundet auf die Zeichengrenze ab), nur an den Änderungsstellen.
    struct Tape {
        private(set) var pieces: [Piece] = []
        /// Zählt wirksame `apply`-Läufe; wer wissen will, ob eine Regel etwas geändert hat, vergleicht ihn.
        private(set) var revision = 0
        /// Aktueller Text als Swift-String und als UTF-16-gestützter NSString. Reguläre Ausdrücke und Teilstücke
        /// laufen über `ns`: ICU liest einen UTF-8-String Stück für Stück um (bei 290 000 Zeichen 16-mal langsamer).
        private(set) var text: String
        private(set) var ns: NSString
        private(set) var units: [UInt16]
        private let originNS: NSString
        private let originUnits: [UInt16]
        private let cuts: Cuts

        init(_ text: String, scalarBased: Bool = false) {
            self.text = text
            units = Array(text.utf16); originUnits = units
            ns = NSString(characters: units, length: units.count)
            originNS = ns
            cuts = Cuts(origin: text, units: units, scalarBased: scalarBased)
        }

        /// Grenzen der Zeichen (bzw. Skalare) des Ursprungstexts. Eine Klasse, damit die Abfragen je Änderung
        /// nicht den ganzen Tape kopieren.
        private final class Cuts {
            let origin: String, units: [UInt16], scalarBased: Bool
            init(origin: String, units: [UInt16], scalarBased: Bool) { self.origin = origin; self.units = units; self.scalarBased = scalarBased }

            /// Ursprungsbereich [start, end) des Zeichens (bzw. Skalars), das `offset` enthält; `offset` < Textlänge.
            func unit(containing offset: Int) -> (start: Int, end: Int) {
                if scalarBased {
                    let unit = units[offset]
                    if (unit & 0xFC00) == 0xDC00, offset > 0, (units[offset - 1] & 0xFC00) == 0xD800 { return (offset - 1, offset + 1) }
                    return (offset, (unit & 0xFC00) == 0xD800 && offset + 1 < units.count ? offset + 2 : offset + 1)
                }
                let index = String.Index(utf16Offset: offset, in: origin)
                let after = origin.index(after: index)
                let before = origin.index(before: after)
                return (before.utf16Offset(in: origin), after.utf16Offset(in: origin))
            }
        }

        /// Wendet geordnete, nicht überlappende Änderungen (Bereiche am aktuellen Text) an.
        mutating func apply(_ edits: [Edit], rule: String) {
            guard !edits.isEmpty else { return }
            revision += 1
            let named = RuleSet.named(rule)
            let current = units, currentLength = current.count, old = pieces
            let cuts = self.cuts, originUnits = self.originUnits, originNS = self.originNS, scalarBased = cuts.scalarBased
            var output: [Piece] = []
            // Der Text eines vereinigten Stücks wird erst aus dem fertigen NSString geschnitten (billiger als je Stück zu dekodieren).
            var pending: [(piece: Int, range: NSRange)] = []
            output.reserveCapacity(old.count + edits.count)
            // Obergrenze der neuen Länge: der alte Text plus alle Ersatztexte.
            var capacity = currentLength + 16, counted = 0
            while counted < edits.count { capacity += edits[counted].units.count; counted += 1 }
            /// Zeichen-/Skalargrenze im Ursprungstext: zwei einzelne Zeichen nebeneinander trennt jedes Verfahren
            /// (CR+LF ausgenommen), nur sonst fragt `Cuts` Swift selbst.
            func isBoundary(_ offset: Int) -> Bool {
                guard offset > 0 && offset < originUnits.count else { return true }
                let before = originUnits[offset - 1], after = originUnits[offset]
                if scalarBased { return !((after & 0xFC00) == 0xDC00 && (before & 0xFC00) == 0xD800) }
                if before < 0x300 && after < 0x300 && !(before == 0x0D && after == 0x0A) { return true }
                if TextAufbereiten.plain(before) && TextAufbereiten.plain(after) { return true }
                return cuts.unit(containing: offset).start == offset
            }
            func floorBoundary(_ offset: Int) -> Int {
                offset >= originUnits.count || isBoundary(offset) ? min(offset, originUnits.count) : cuts.unit(containing: offset).start
            }
            func ceilBoundary(_ offset: Int) -> Int {
                offset >= originUnits.count || isBoundary(offset) ? min(offset, originUnits.count) : cuts.unit(containing: offset).end
            }
            // `offset`-Größen sind Koordinaten des alten aktuellen Textes; `delta` = alt-aktuell minus Ursprung,
            // nur aus den bereits durchlaufenen alten Stücken (neue Stücke zählen nicht).
            var copied = 0, index = 0, delta = 0, e = 0, written = 0
            func startOfPiece(_ i: Int) -> Int { old[i].range.location + delta }
            let built = [UInt16](unsafeUninitializedCapacity: capacity) { buffer, count in
                let target = buffer.baseAddress!
                current.withUnsafeBufferPointer { source in
                    func copy(_ from: Int, _ to: Int) {
                        guard to > from else { return }
                        (target + written).update(from: source.baseAddress! + from, count: to - from)
                        written += to - from
                    }
                    func put(_ units: [UInt16]) {
                        guard !units.isEmpty else { return }
                        units.withUnsafeBufferPointer { (target + written).update(from: $0.baseAddress!, count: $0.count) }
                        written += units.count
                    }
                    while e < edits.count {
                        let first = edits[e], location = first.range.location
                        // Stücke, die vor der Stelle enden (auch leere an ihr), bleiben unverändert.
                        while index < old.count, startOfPiece(index) + old[index].length <= location {
                            output.append(old[index])
                            delta += old[index].length - old[index].range.length
                            index += 1
                        }
                        let regionEnd = index < old.count ? startOfPiece(index) : currentLength
                        var start: Int, originStart: Int
                        if index < old.count && location >= regionEnd {
                            start = regionEnd; originStart = old[index].range.location
                        } else if location < regionEnd {
                            originStart = floorBoundary(location - delta); start = originStart + delta
                        } else {
                            // Hinter dem Textende; nur eine Einfügung genau am Ende ist gültig.
                            start = currentLength; originStart = currentLength - delta
                            guard location == currentLength, first.range.length == 0 else { break }
                        }
                        if first.range.length == 0 && start == location {
                            output.append(Piece(range: NSRange(location: location - delta, length: 0), alt: "", text: first.text, rules: first.rules ?? named))
                            copy(copied, location); put(first.units)
                            copied = location
                            e += 1; continue
                        }
                        guard start < currentLength else { break }
                        var offset = start, end = min(NSMaxRange(first.range), currentLength), next = e + 1
                        var rules = RuleSet.none
                        repeat {
                            while offset < end {
                                if index < old.count, startOfPiece(index) == offset {
                                    let piece = old[index]
                                    rules = RuleSet(rawValue: rules.rawValue | piece.rules.rawValue)
                                    offset += piece.length; delta += piece.length - piece.range.length; index += 1
                                } else {
                                    let limit = index < old.count ? startOfPiece(index) : currentLength
                                    guard offset < limit else { end = offset; break }
                                    offset = ceilBoundary(min(end, limit) - delta) + delta
                                }
                            }
                            while next < edits.count && edits[next].range.location < offset {
                                end = min(max(end, NSMaxRange(edits[next].range)), currentLength); next += 1
                            }
                        } while offset < end
                        copy(copied, start)
                        let groupStart = written
                        var at = start, k = e
                        while k < next {
                            let edit = edits[k]
                            copy(at, edit.range.location); put(edit.units)
                            at = NSMaxRange(edit.range)
                            rules = RuleSet(rawValue: rules.rawValue | (edit.rules ?? named).rawValue)
                            k += 1
                        }
                        copy(at, offset)
                        copied = offset
                        let originEnd = offset - delta
                        pending.append((output.count, NSRange(location: groupStart, length: written - groupStart)))
                        output.append(Piece(range: NSRange(location: originStart, length: originEnd - originStart),
                                            source: originNS, text: "", rules: rules))
                        e = next
                    }
                    copy(copied, currentLength)
                }
                count = written
            }
            var rest = index
            while rest < old.count { output.append(old[rest]); rest += 1 }
            units = built
            ns = NSString(characters: built, length: built.count)
            var item = 0
            while item < pending.count { output[pending[item].piece].text = ns.substring(with: pending[item].range); item += 1 }
            pieces = output
            // UTF-16 -> UTF-8 in Foundation (bulk); `String(decoding:as: UTF16.self)` ist im Debug-Bau je Zeichen ein Parser.
            text = ns.data(using: String.Encoding.utf8.rawValue).map { String(decoding: $0, as: UTF8.self) }
                ?? String(decoding: built, as: UTF16.self)
        }

        mutating func replace(_ pattern: String, _ replacement: String, rule: String) {
            let ns = self.ns, source = ns as String
            let regex = TextAufbereiten.regex(pattern)
            // Eine Schablone ohne `$` und `\` ersetzt wörtlich; nur sonst muss je Treffer eingesetzt werden.
            let literal = !replacement.contains("$") && !replacement.contains("\\")
            var edits: [Edit] = []
            regex.enumerateMatches(in: source, range: NSRange(location: 0, length: ns.length)) { match, _, _ in
                guard let match else { return }
                let next = literal ? replacement : regex.replacementString(for: match, in: source, offset: 0, template: replacement)
                if match.range.length == next.utf16.count, TextAufbereiten.identisch(ns.substring(with: match.range), next) { return }
                edits.append(Edit(match.range, next))
            }
            apply(edits, rule: rule)
        }
    }

    /// Muster, das nie trifft: Ersatz, falls ein internes Muster ungültig ist (ICU ist strenger als JS/Rust).
    static let nieTreffend = try! NSRegularExpression(pattern: "(?!)")

    /// Jedes Muster wird einmal übersetzt: `matches`/`replacing` laufen je Zeile, und das Übersetzen eines
    /// ICU-Musters kostet ein Vielfaches eines Treffers (bei 10 000 Zeilen der größte Posten). Die Tabelle ist
    /// durch eine Sperre geschützt (NSRegularExpression selbst ist für Treffer thread-sicher) und hat eine Obergrenze,
    /// damit dynamisch gebildete Muster (Maske mit freiem Zeichen) sie nicht unbegrenzt füllen.
    static func regex(_ pattern: String) -> NSRegularExpression {
        musterSperre.lock(); defer { musterSperre.unlock() }
        if let cached = musterTabelle[pattern] { return cached }
        // Ausschließlich interne, konstante Muster; keine Benutzereingabe. Ein ungültiges Muster darf die
        // App nicht beenden (lva#915: SIGILL beim Öffnen von „Text aufbereiten“); die Regel fällt dann aus,
        // und der Test `testAlleMusterSindGueltig` macht es sichtbar.
        let compiled: NSRegularExpression
        do { compiled = try NSRegularExpression(pattern: pattern) } catch {
            _ungueltigeMuster.append(pattern)
            compiled = nieTreffend
        }
        if musterTabelle.count >= 512 { musterTabelle.removeAll(keepingCapacity: true) }
        musterTabelle[pattern] = compiled
        return compiled
    }
    private static let musterSperre = NSLock()
    nonisolated(unsafe) private static var musterTabelle: [String: NSRegularExpression] = [:]
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
