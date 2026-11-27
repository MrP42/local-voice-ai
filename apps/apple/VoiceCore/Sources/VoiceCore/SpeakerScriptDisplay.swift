import Foundation

/// Anzeige eines Absatzes aus einem Sprecher-Skript (lva#954, Paket 2): Sprechermarken und Regie-Tags verschwinden,
/// ein Sprecherwechsel mitten im Absatz wird zu einem kurzen Namen im Text, der Sprecher am Absatzanfang steht als
/// Kopfzeile darüber (wie die Leseansicht am Desktop). Positionen bleiben im Rohtext; `map` rechnet Markierungen
/// (Satz, Wort, Fundstelle) in den angezeigten Text um.
public struct SpeakerScriptDisplay: Equatable, Sendable {
    /// Ersetzung eines Rohbereichs; `label` ist der Name, der an dieser Stelle im Text erscheint (sonst entfällt er).
    public struct Edit: Equatable, Sendable {
        public var raw: NSRange
        public var label: String?
        public var known: Bool
        /// Angezeigter Text an dieser Stelle (Name mit Trenner oder leer).
        public var replacement: String { label.map { $0 + " · " } ?? "" }
    }

    /// Bereiche im angezeigten Text, die ein Name im Text belegen.
    public struct InlineLabel: Equatable, Sendable {
        public var range: NSRange
        public var name: String
        public var known: Bool
    }

    public let raw: String
    public let edits: [Edit]
    /// Sprecher am Absatzanfang (auch geerbt), `nil` ohne Sprecher.
    public let leadName: String?
    public let leadKnown: Bool
    /// Sprecher am Absatzende, für die Frage, ob der nächste Absatz eine Kopfzeile braucht.
    public let lastName: String?

    public init(paragraph raw: String, presentation: SpeakerScript.Presentation) {
        self.raw = raw
        let all = raw as NSString
        let length = all.length
        func isHorizontalSpace(_ c: unichar) -> Bool { c == 32 || c == 9 || c == 0xA0 }

        // Verdeckte Bereiche samt folgendem Leerraum der Zeile, überlappende zusammengelegt.
        var hidden: [NSRange] = []
        for range in presentation.hiddenRanges.sorted(by: { $0.location < $1.location }) {
            let start = max(0, min(range.location, length))
            var end = min(length, NSMaxRange(range))
            while end < length, isHorizontalSpace(all.character(at: end)) { end += 1 }
            guard end > start else { continue }
            if let last = hidden.last, start <= NSMaxRange(last) {
                hidden[hidden.count - 1] = NSRange(location: last.location, length: max(NSMaxRange(last), end) - last.location)
            } else {
                hidden.append(NSRange(location: start, length: end - start))
            }
        }

        // Abschnitte mit Sprecher, die in diesem Absatz beginnen: ihr Marker liegt am Abschnittsanfang. Ein Marker
        // am Absatzanfang wird nie ein Name im Text (dafür gibt es die Kopfzeile), darum zählt Position 0 nicht.
        let named = presentation.sections.filter { $0.speakerName != nil && $0.range.location > 0 }
        func sectionStarting(in range: NSRange) -> SpeakerScript.Section? {
            named.last { NSLocationInRange($0.range.location, range) }
        }

        var edits: [Edit] = []
        var visibleBefore = false
        var cursor = 0
        func hasVisible(_ from: Int, _ to: Int) -> Bool {
            guard to > from else { return false }
            let piece = all.substring(with: NSRange(location: from, length: to - from))
            return !piece.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
        for range in hidden {
            if hasVisible(cursor, range.location) { visibleBefore = true }
            let section = sectionStarting(in: range)
            let label = visibleBefore ? section?.speakerName : nil
            edits.append(Edit(raw: range, label: label, known: section?.known ?? false))
            cursor = NSMaxRange(range)
        }
        self.edits = edits

        // Sprecher am ersten sichtbaren Zeichen bzw. am Ende.
        var firstVisible = 0
        for edit in edits where edit.raw.location <= firstVisible { firstVisible = max(firstVisible, NSMaxRange(edit.raw)) }
        while firstVisible < length, isHorizontalSpace(all.character(at: firstVisible)) || all.character(at: firstVisible) == 10 { firstVisible += 1 }
        let lead = presentation.sections.last { $0.range.location <= firstVisible }
        leadName = lead?.speakerName
        leadKnown = lead?.known ?? false
        lastName = presentation.sections.last?.speakerName
    }

    /// Ohne verdeckte Bereiche ändert sich nichts an der Anzeige.
    public var isIdentity: Bool { edits.isEmpty }

    /// Angezeigter Text.
    public var text: String {
        guard !edits.isEmpty else { return raw }
        let all = raw as NSString
        var out = "", cursor = 0
        for edit in edits {
            out += all.substring(with: NSRange(location: cursor, length: edit.raw.location - cursor)) + edit.replacement
            cursor = NSMaxRange(edit.raw)
        }
        return out + all.substring(from: cursor)
    }

    /// Namen im angezeigten Text.
    public var inlineLabels: [InlineLabel] {
        var out: [InlineLabel] = []
        var shift = 0
        for edit in edits {
            let replacementLength = (edit.replacement as NSString).length
            if let label = edit.label {
                out.append(InlineLabel(range: NSRange(location: edit.raw.location + shift, length: replacementLength),
                                       name: label, known: edit.known))
            }
            shift += replacementLength - edit.raw.length
        }
        return out
    }

    /// Rohposition → angezeigte Position. Eine Position in einem verdeckten Bereich rückt an dessen Rand:
    /// ein Anfang hinter die Ersetzung, ein Ende davor.
    public func position(_ rawPosition: Int, isEnd: Bool) -> Int {
        var shift = 0
        for edit in edits {
            let replacementLength = (edit.replacement as NSString).length
            if rawPosition <= edit.raw.location { break }
            if rawPosition < NSMaxRange(edit.raw) {
                return edit.raw.location + shift + (isEnd ? 0 : replacementLength)
            }
            shift += replacementLength - edit.raw.length
        }
        return rawPosition + shift
    }

    /// Rohbereich → angezeigter Bereich; `nil`, wenn er ganz in verdeckten Text fällt.
    public func map(_ range: NSRange) -> NSRange? {
        guard !edits.isEmpty else { return range }
        let a = position(range.location, isEnd: false)
        let b = position(NSMaxRange(range), isEnd: true)
        return b > a ? NSRange(location: a, length: b - a) : nil
    }
}
