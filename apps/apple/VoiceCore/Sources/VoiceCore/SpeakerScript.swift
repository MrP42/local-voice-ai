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

    /// `startsAtLineStart: false`, wenn `text` ein Ausschnitt ist, der mitten in einer Zeile des Quelltexts beginnt
    /// (satzweises Neuparsen): Das alte Zeilenformat `Name:` gilt dann in der ersten Zeile nicht, wie bei
    /// `split_speaker_segments`, das nur ganze Zeilen prüft.
    public init(_ text: String, speakers: [Speaker] = [], startsAtLineStart: Bool = true) {
        self.text = text; self.speakers = speakers
        markers = Self.markers(text, speakers: speakers, startsAtLineStart: startsAtLineStart)
        hiddenRanges = (Self.tagSpans(text) + markers.map(\.range)).sorted { $0.location < $1.location }
        let all = text as NSString
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

    public static func splitSpeakerSegments(_ text: String, speakers: [Speaker]) -> [Segment] {
        SpeakerScript(text, speakers: speakers).segments
    }

    public static func foldSpeakerName(_ name: String) -> String {
        let lower = name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
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
        func local(_ range: NSRange) -> NSRange? {
            let overlap = NSIntersectionRange(range, paragraph)
            return overlap.length > 0 ? NSRange(location: overlap.location - paragraph.location, length: overlap.length) : nil
        }
        let length = (text as NSString).length
        let starts = [0] + markers.map { $0.range.location }
        var sections: [Section] = []
        for (i, start) in starts.enumerated() {
            let end = i + 1 < starts.count ? starts[i + 1] : length
            guard let range = local(NSRange(location: start, length: end - start)) else { continue }
            let marker = i > 0 ? markers[i - 1] : nil
            sections.append(Section(range: range, speakerName: marker?.name,
                                    known: marker.map { Self.resolve($0.name, speakers) != nil } ?? false))
        }
        return Presentation(hiddenRanges: hiddenRanges.compactMap(local), sections: sections)
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
