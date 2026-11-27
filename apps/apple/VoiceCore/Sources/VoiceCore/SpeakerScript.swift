import Foundation

/// Sprecherprotokoll wie `tts/protocol.rs`. Alle öffentlichen Bereiche zählen UTF-16
/// im unveränderten Eingangstext; nur Synthesetext darf Marker und Tags verlieren.
public struct SpeakerScript: Sendable {
    public struct Speaker: Equatable, Sendable {
        public var id: String
        public var name: String
        public var aliases: [String]
        public init(id: String, name: String, aliases: [String] = []) {
            self.id = id; self.name = name; self.aliases = aliases
        }
    }
    public struct Voice: Sendable {
        public var id: String
        public var label: String
        public init(id: String, label: String) { self.id = id; self.label = label }
    }
    public struct Marker: Equatable, Sendable {
        public var range: NSRange
        public var name: String
        public var style: String?
    }
    public struct Segment: Equatable, Sendable {
        public var range: NSRange
        public var text: String
        public var voice: String?
        public var style: String?
        public var speakerName: String?
    }
    public struct Section: Equatable, Sendable {
        public var range: NSRange
        public var speakerName: String?
        public var known: Bool
    }
    /// Absatzlokale NSRanges für die UI; Text, Suche und Fortschritt bleiben roh.
    public struct Presentation: Equatable, Sendable {
        public var hiddenRanges: [NSRange]
        public var sections: [Section]
    }
    public struct SpeechPart: Equatable, Sendable {
        public var text: String
        public var voice: String
        public var style: String?
        public var pauseMillis: Int
        /// Je UTF-16-Einheit des Synthesetexts ihre Position im Rohtext (Wortmarkierung).
        public var sourceOffsets: [Int]
    }

    public let text: String
    public let segments: [Segment]
    public let markers: [Marker]
    public let hiddenRanges: [NSRange]
    let speakers: [Speaker]
    private let sections: [Section]
    private let hiddenEnds: [Int]

    /// `startsAtLineStart: false`, wenn `text` ein Ausschnitt ist, der mitten in einer Zeile des Quelltexts beginnt
    /// (satzweises Neuparsen): Das alte Zeilenformat `Name:` gilt dann in der ersten Zeile nicht, wie bei
    /// `split_speaker_segments`, das nur ganze Zeilen prüft.
    public init(_ text: String, speakers: [Speaker] = [], startsAtLineStart: Bool = true) {
        self.text = text; self.speakers = speakers
        markers = Self.markers(text, speakers: speakers, startsAtLineStart: startsAtLineStart)
        hiddenRanges = (Self.tagSpans(text) + markers.map(\.range)).sorted { $0.location < $1.location }
        let all = text as NSString
        var prepared: [Section] = []
        var sectionStart = 0
        var sectionName: String?, sectionKnown = false
        for marker in markers {
            prepared.append(Section(range: NSRange(location: sectionStart, length: marker.range.location - sectionStart),
                                    speakerName: sectionName, known: sectionKnown))
            sectionStart = marker.range.location
            sectionName = marker.name; sectionKnown = Self.resolve(marker.name, speakers) != nil
        }
        prepared.append(Section(range: NSRange(location: sectionStart, length: all.length - sectionStart),
                                speakerName: sectionName, known: sectionKnown))
        sections = prepared
        // Überlappende Tags können früh beginnen und erst hinter späteren Markern enden.
        var furthest = 0
        hiddenEnds = hiddenRanges.map { furthest = max(furthest, NSMaxRange($0)); return furthest }
        var result: [Segment] = []
        var start = 0
        var voice: String?, style: String?, name: String?
        func push(_ end: Int) {
            let range = Self.trimRange(all, NSRange(location: start, length: end - start))
            guard range.length > 0 else { return }
            result.append(Segment(range: range,
                                  text: all.substring(with: range).replacingOccurrences(of: "\r\n", with: "\n"),
                                  voice: voice, style: style, speakerName: name))
        }
        for marker in markers {
            push(marker.range.location)
            let speaker = Self.resolve(marker.name, speakers)
            voice = speaker?.id; style = speaker == nil ? nil : marker.style; name = marker.name
            start = NSMaxRange(marker.range)
        }
        push(all.length)
        segments = result
    }

    /// Kleinschreibung wie Rusts `str::to_lowercase`, also mit der Final_Sigma-Regel: Ein großes Σ am Wortende wird
    /// ς, sonst σ (Foundation kennt die Regel nicht; Golden-Vektor „ii̇ς“, Mac-Tor #964).
    static func rustLowercased(_ text: String) -> String {
        let scalars = Array(text.unicodeScalars)
        guard scalars.contains("\u{03A3}") else { return text.lowercased() }
        func casedNeighbor(from start: Int, step: Int) -> Bool {
            var i = start
            while i >= 0, i < scalars.count {
                let p = scalars[i].properties
                if p.isCaseIgnorable { i += step; continue }
                return p.isCased
            }
            return false
        }
        var out = String.UnicodeScalarView()
        for (i, c) in scalars.enumerated() {
            if c == "\u{03A3}" {
                let final = casedNeighbor(from: i - 1, step: -1) && !casedNeighbor(from: i + 1, step: 1)
                out.append(final ? "\u{03C2}" : "\u{03C3}")
            } else {
                out.append(contentsOf: String(c).lowercased().unicodeScalars)
            }
        }
        return String(out)
    }

    public static func splitSpeakerSegments(_ text: String, speakers: [Speaker]) -> [Segment] {
        SpeakerScript(text, speakers: speakers).segments
    }

    public static func foldSpeakerName(_ name: String) -> String {
        let lower = rustLowercased(name.trimmingCharacters(in: .whitespacesAndNewlines))
        var out = "", lastSpace = false
        for c in lower.unicodeScalars {
            switch c {
            case "ä": out += "ae"; lastSpace = false
            case "ö": out += "oe"; lastSpace = false
            case "ü": out += "ue"; lastSpace = false
            case "ß": out += "ss"; lastSpace = false
            default:
                if c == "-" || c == "_" || c.properties.isWhitespace {
                    if !lastSpace { out += " " }; lastSpace = true
                } else { out.unicodeScalars.append(c); lastSpace = false }
            }
        }
        return out
    }

    private static func resolve(_ name: String, _ speakers: [Speaker]) -> Speaker? {
        let needle = foldSpeakerName(name)
        guard !needle.isEmpty else { return nil }
        return speakers.first { ([$0.name] + $0.aliases).contains { foldSpeakerName($0) == needle } }
    }

    public static func tagSpans(_ text: String) -> [NSRange] {
        scan(text, opening: 91, closing: 93, forbidden: [10])
    }

    private static func scan(_ text: String, opening: UInt16, closing: UInt16, forbidden: Set<UInt16>) -> [NSRange] {
        let units = Array(text.utf16)
        var out: [NSRange] = [], i = 0
        while i < units.count {
            if units[i] == opening {
                var j = i + 1
                while j < units.count, units[j] != closing, !forbidden.contains(units[j]) { j += 1 }
                if j < units.count, units[j] == closing {
                    out.append(NSRange(location: i, length: j + 1 - i)); i = j + 1; continue
                }
            }
            i += 1
        }
        return out
    }

    public static func scanMarkerCandidates(_ text: String) -> [Marker] {
        let all = text as NSString
        return scan(text, opening: 60, closing: 62, forbidden: [60, 10]).compactMap { range in
            let inner = all.substring(with: NSRange(location: range.location + 1, length: range.length - 2))
            guard (1...60).contains(inner.unicodeScalars.count) else { return nil }
            let parts = inner.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)
            let style = parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespacesAndNewlines) : ""
            return Marker(range: range, name: parts[0].trimmingCharacters(in: .whitespacesAndNewlines),
                          style: style.isEmpty ? nil : style)
        }
    }

    private static func markers(_ text: String, speakers: [Speaker], startsAtLineStart: Bool = true) -> [Marker] {
        let all = text as NSString
        var result = scanMarkerCandidates(text).filter {
            let inner = all.substring(with: NSRange(location: $0.range.location + 1, length: $0.range.length - 2))
            return inner.unicodeScalars.first?.properties.isWhitespace != true && inner.unicodeScalars.last?.properties.isWhitespace != true
        }
        var offset = 0
        for (index, line) in text.components(separatedBy: "\n").enumerated() {
            let ns = line as NSString
            if index > 0 || startsAtLineStart, let colon = line.firstIndex(of: ":") {
                let name = String(line[..<colon]).trimmingCharacters(in: .whitespacesAndNewlines)
                if resolve(name, speakers) != nil {
                    let end = NSRange(line.startIndex...colon, in: line).length
                    result.append(Marker(range: NSRange(location: offset, length: end), name: name, style: nil))
                }
            }
            offset += ns.length + 1
        }
        return result.sorted { $0.range.location < $1.range.location }
    }

    public static func pauseMillis(_ inner: String) -> Int? {
        switch inner.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "pause": return 500
        case "short pause": return 250
        case "long pause": return 1000
        case "break": return 700
        default: return nil
        }
    }

    public func presentation(in paragraph: NSRange) -> Presentation {
        presentation(in: paragraph, visit: {})
    }

    /// Interner Prüf-Hook: ein Besuch je binärem Vergleich und je lokal geprüftem Bereich.
    func presentation(in paragraph: NSRange, visit: () -> Void) -> Presentation {
        func local(_ range: NSRange) -> NSRange? {
            let overlap = NSIntersectionRange(range, paragraph)
            return overlap.length > 0 ? NSRange(location: overlap.location - paragraph.location, length: overlap.length) : nil
        }
        guard paragraph.length > 0 else { return Presentation(hiddenRanges: [], sections: []) }
        var visibleSections: [Section] = []
        var i = Self.lowerBound(sections.count) { visit(); return NSMaxRange(sections[$0].range) > paragraph.location }
        while i < sections.count, sections[i].range.location < NSMaxRange(paragraph) {
            visit()
            if let range = local(sections[i].range) {
                visibleSections.append(Section(range: range, speakerName: sections[i].speakerName, known: sections[i].known))
            }
            i += 1
        }
        var hidden: [NSRange] = []
        i = Self.lowerBound(hiddenEnds.count) { visit(); return hiddenEnds[$0] > paragraph.location }
        while i < hiddenRanges.count, hiddenRanges[i].location < NSMaxRange(paragraph) {
            visit()
            if let range = local(hiddenRanges[i]) { hidden.append(range) }
            i += 1
        }
        return Presentation(hiddenRanges: hidden, sections: visibleSections)
    }

    /// Erster Index, für den das monotone Prädikat gilt, sonst `count`.
    private static func lowerBound(_ count: Int, _ matches: (Int) -> Bool) -> Int {
        var low = 0, high = count
        while low < high {
            let mid = low + (high - low) / 2
            if matches(mid) { high = mid } else { low = mid + 1 }
        }
        return low
    }

    /// Übersetzter Absatz mit dem Sprecherzustand an seiner Quellposition; lokale Marker haben Vorrang.
    public func display(translated value: String, at offset: Int) -> SpeakerScriptDisplay? {
        guard !markers.isEmpty else { return nil }
        let script = SpeakerScript(value, speakers: speakers, startsAtLineStart: Self.atLineStart(text, offset))
        var presentation = script.presentation(in: NSRange(location: 0, length: value.utf16.count))
        let inherited = Self.lowerBound(sections.count) { sections[$0].range.location > offset } - 1
        if inherited >= 0, let first = presentation.sections.first, first.speakerName == nil {
            presentation.sections[0].speakerName = sections[inherited].speakerName
            presentation.sections[0].known = sections[inherited].known
        }
        let display = SpeakerScriptDisplay(paragraph: value, presentation: presentation)
        return display.isIdentity && display.leadName == nil ? nil : display
    }

    /// Steht vor `offset` (UTF-16) in seiner Zeile nur Leerraum?
    public static func atLineStart(_ text: String, _ offset: Int) -> Bool {
        let all = text as NSString
        var i = min(offset, all.length) - 1
        while i >= 0 {
            let c = all.character(at: i)
            if c == 10 || c == 13 { return true }
            guard c == 32 || c == 9 || c == 0xA0 else { return false }
            i -= 1
        }
        return true
    }

    /// Stimmzustand vor einer Teilstrecke (z. B. einem späteren Satz ohne eigenen Marker).
    public func voice(at offset: Int) -> String? {
        markers.last { $0.range.location <= offset }.flatMap { Self.resolve($0.name, speakers)?.id }
    }

    public func speechParts(fallbackVoice: String, initialVoice: String? = nil) -> [SpeechPart] {
        var out: [SpeechPart] = []
        let rawText = text as NSString
        let tags = Self.tagSpans(text)
        for segment in segments {
            let voice = segment.voice ?? (segment.speakerName == nil ? initialVoice : nil) ?? fallbackVoice
            let raw = (text as NSString).substring(with: segment.range)
            let all = raw as NSString
            var buffer = "", offsets: [Int] = [], start = 0
            func append(_ end: Int) {
                guard end > start else { return }
                buffer += all.substring(with: NSRange(location: start, length: end - start))
                offsets += (start..<end).map { segment.range.location + $0 }
            }
            func flush() {
                let range = Self.trimRange(buffer as NSString, NSRange(location: 0, length: (buffer as NSString).length))
                if range.length > 0 {
                    out.append(SpeechPart(text: (buffer as NSString).substring(with: range), voice: voice,
                                          style: segment.style, pauseMillis: 0,
                                          sourceOffsets: Array(offsets[range.location..<NSMaxRange(range)])))
                }
                buffer = ""; offsets = []
            }
            for tag in tags {
                let overlap = NSIntersectionRange(tag, segment.range)
                guard overlap.length > 0 else { continue }
                let local = NSRange(location: overlap.location - segment.range.location, length: overlap.length)
                append(local.location)
                let inner = rawText.substring(with: NSRange(location: tag.location + 1, length: tag.length - 2))
                if tag.location >= segment.range.location, let ms = Self.pauseMillis(inner) {
                    flush(); out.append(SpeechPart(text: "", voice: voice, style: nil, pauseMillis: ms, sourceOffsets: []))
                }
                start = NSMaxRange(local)
            }
            append(all.length); flush()
        }
        return out
    }

    private static func trimRange(_ all: NSString, _ range: NSRange) -> NSRange {
        let raw = all.substring(with: range)
        // Trimmen über String-Indizes hält auch mehrteilige Unicode-Zeichen auf ihren Rohpositionen.
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return NSRange(location: range.location, length: 0) }
        let found = (raw as NSString).range(of: trimmed, options: .literal)
        return NSRange(location: range.location + found.location, length: found.length)
    }

    /// Ein Tokenizer sieht Satzzeichen nur außerhalb von Steuerbereichen. Die
    /// UTF-16-Länge bleibt selbst bei Emoji und kombinierten Zeichen unverändert.
    public static func maskedControls(_ text: String, speakers: [Speaker] = []) -> String {
        let script = SpeakerScript(text, speakers: speakers)
        var units = Array(text.utf16)
        for range in script.hiddenRanges { for i in range.location..<NSMaxRange(range) { units[i] = 120 } }
        return String(decoding: units, as: UTF16.self)
    }

    /// Tags maskieren, ohne Offsets zu verschieben. Sprecherwechsel sind zusätzliche Satzgrenzen.
    public static func sentenceRanges(_ text: String, speakers: [Speaker]) -> [NSRange] {
        let script = SpeakerScript(text, speakers: speakers)
        guard !script.hiddenRanges.isEmpty else { return LibraryText.sentences(text) }
        let masked = maskedControls(text, speakers: speakers)
        let tags = tagSpans(text)
        let markerCuts = script.markers.map { $0.range.location }.filter { offset in
            !tags.contains { offset > $0.location && offset < NSMaxRange($0) }
        }
        let cuts = Set([0, text.utf16.count] + LibraryText.sentences(masked).map(\.location) + markerCuts).sorted()
        let all = text as NSString
        return zip(cuts, cuts.dropFirst()).compactMap { a, b in
            let range = trimRange(all, NSRange(location: a, length: b - a))
            return range.length > 0 ? range : nil
        }
    }

    /// Desktop-Regeln: default auslassen, Kurzname vor „ – “, Kennung als Alias,
    /// Dubletten als „Name (Kennung)“. Auch installierte eigene Stimmen laufen hierdurch.
    public static func roster(_ voices: [Voice]) -> [Speaker] {
        func safe(_ name: String) -> Bool {
            !name.isEmpty && name.unicodeScalars.count <= 60 && !name.contains { "<>:\n\r".contains($0) }
        }
        var out: [Speaker] = [], claimed: Set<String> = []
        for voice in voices {
            let id = voice.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, id != "default", !out.contains(where: { $0.id == voice.id }) else { continue }
            let short = (voice.label.components(separatedBy: " – ").first ?? voice.label).trimmingCharacters(in: .whitespacesAndNewlines)
            let base = safe(short) ? short : id
            guard safe(base) else { continue }
            let name = claimed.contains(foldSpeakerName(base)) ? "\(base) (\(id))" : base
            guard safe(name), !claimed.contains(foldSpeakerName(name)) else { continue }
            claimed.insert(foldSpeakerName(name))
            var aliases = [id]
            let label = voice.label.trimmingCharacters(in: .whitespacesAndNewlines)
            if label != name, safe(label), !claimed.contains(foldSpeakerName(label)) {
                claimed.insert(foldSpeakerName(label)); aliases.append(label)
            }
            out.append(Speaker(id: voice.id, name: name, aliases: aliases))
        }
        return out
    }
}
