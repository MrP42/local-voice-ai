import Foundation

// Einheits-Player (Plan `apps/apple/docs/einheits-player.md`): ein Bildschirm für Hören,
// Mitlesen und Lesen, Wechsel mit einem Tipp, Position bleibt. Hier liegt die Logik ohne UI.

/// Ansicht im Einheits-Player. Vorbild ist der Umschalter „Hören / Lesen & Hören“ von Audible.
public enum PlayerSurface: String, CaseIterable, Codable, Sendable {
    /// Cover und Steuerung.
    case listen
    /// Text mit Satz- und Wortmarkierung, scrollt mit. Die Wiedergabe läuft.
    case readListen
    /// Nur Text, Wiedergabe pausiert. Antippen setzt die Stelle.
    case read

    public var title: String {
        switch self {
        case .listen: "Hören"
        case .readListen: "Lesen & Hören"
        case .read: "Lesen"
        }
    }

    /// Ohne Text gibt es nur „Hören“. Mit Text alle drei, auch ohne Hörbuch (dann liest die Stimme).
    public static func available(hasText: Bool) -> [PlayerSurface] { hasText ? allCases : [.listen] }

    /// Zuletzt gewählte Ansicht des Buchs, falls verfügbar. Sonst mit Text „Lesen & Hören“.
    public static func initial(stored: String?, hasText: Bool) -> PlayerSurface {
        if let s = stored.flatMap(PlayerSurface.init(rawValue:)), available(hasText: hasText).contains(s) { return s }
        return hasText ? .readListen : .listen
    }

    /// Läuft die Wiedergabe in dieser Ansicht weiter? „Lesen“ hält an, die anderen behalten den Zustand.
    public var keepsPlaying: Bool { self != .read }
}

/// Tonquelle: Hörbuch mit Sprecher oder Vorlesestimme (TTS) über dem Text.
public enum PlayerSource: String, Codable, Sendable {
    case audiobook, voice

    public var title: String { self == .audiobook ? "Hörbuch" : "Vorlesestimme" }

    /// Gemerkte Quelle, falls vorhanden. Sonst das Hörbuch, wenn es auf dem Gerät liegt, sonst die Stimme.
    public static func initial(stored: String?, audioLocal: Bool, hasText: Bool) -> PlayerSource? {
        let ok: (PlayerSource) -> Bool = { $0 == .audiobook ? audioLocal : hasText }
        if let s = stored.flatMap(PlayerSource.init(rawValue:)), ok(s) { return s }
        if audioLocal { return .audiobook }
        return hasText ? .voice : nil
    }
}

/// Markierung beim Mitlesen. Bereiche in UTF-16 relativ zum Absatztext.
public struct ReadingMark: Equatable {
    public var paragraph: Int
    public var sentence: NSRange?
    public var word: NSRange?
    /// Nur mit echter Zeitachse. Ohne sie zeigt die Ansicht den Absatz statt Satz und Wort.
    public var precise: Bool

    public init(paragraph: Int, sentence: NSRange?, word: NSRange?, precise: Bool) {
        self.paragraph = paragraph; self.sentence = sentence; self.word = word; self.precise = precise
    }
}

public enum ReadAlong {
    /// Hörbuch: Markierung an der Abspielstelle. Zwischen zwei Stützpunkten der Zeitachse wird die
    /// Zeichenstelle anteilig geschätzt; daraus folgen Satz und Wort. `sentences(i)` liefert die Satz-
    /// bereiche des Absatzes `i` (der Aufrufer hält sie im Zwischenspeicher, NLTokenizer ist teuer).
    public static func mark(atMs ms: Double, map: FollowMap, paragraphs: [TextParagraph],
                            sentences: (Int) -> [NSRange]) -> ReadingMark? {
        mark(atChars: map.chars(atMs: ms), paragraphs: paragraphs, precise: !map.approximate, sentences: sentences)
    }

    /// Markierung an einer Zeichenstelle im Gesamttext (UTF-16).
    public static func mark(atChars chars: Double, paragraphs: [TextParagraph], precise: Bool,
                            sentences: (Int) -> [NSRange]) -> ReadingMark? {
        guard !paragraphs.isEmpty, chars.isFinite else { return nil }
        let index = LibraryText.paragraphIndex(paragraphs, at: chars)
        guard precise else { return ReadingMark(paragraph: index, sentence: nil, word: nil, precise: false) }
        let p = paragraphs[index]
        let length = p.text.utf16.count
        guard length > 0 else { return ReadingMark(paragraph: index, sentence: nil, word: nil, precise: true) }
        let local = min(max(0, Int(chars.rounded(.down)) - p.textStart), length - 1)
        let ranges = sentences(index)
        let sentence = ranges.isEmpty ? nil : ranges[LibraryText.sentenceIndex(ranges, at: local)]
        return ReadingMark(paragraph: index, sentence: sentence, word: wordRange(p.text, at: local), precise: true)
    }

    /// Wort an der Stelle `at` (UTF-16) ohne anhängende Satzzeichen. Liegt die Stelle in einem
    /// Leerraum, gilt das nächste Wort danach.
    public static func wordRange(_ text: String, at: Int) -> NSRange? {
        let s = text as NSString
        let n = s.length
        guard n > 0 else { return nil }
        let space = CharacterSet.whitespacesAndNewlines
        let edge = CharacterSet.punctuationCharacters.union(.symbols).union(space)
        func isSpace(_ i: Int) -> Bool { unitIn(s, i, space) }
        var i = min(max(0, at), n - 1)
        while i < n, isSpace(i) { i += 1 }
        guard i < n else { return nil }
        var start = i, end = i + 1
        while start > 0, !isSpace(start - 1) { start -= 1 }
        while end < n, !isSpace(end) { end += 1 }
        while start < end, unitIn(s, start, edge) { start += 1 }
        while end > start, unitIn(s, end - 1, edge) { end -= 1 }
        return end > start ? NSRange(location: start, length: end - start) : nil
    }

    private static func unitIn(_ s: NSString, _ i: Int, _ set: CharacterSet) -> Bool {
        guard let scalar = Unicode.Scalar(s.character(at: i)) else { return false }  // Ersatzpaare zählen als Wort
        return set.contains(scalar)
    }

    /// Stelle im Text, die beim Quellenwechsel Hörbuch → Stimme gilt (Beginn des laufenden Satzes,
    /// damit der Satz vollständig gelesen wird).
    public static func handoverChars(atMs ms: Double, map: FollowMap, paragraphs: [TextParagraph],
                                     sentences: (Int) -> [NSRange]) -> Int {
        let chars = map.chars(atMs: ms)
        guard let mark = mark(atChars: chars, paragraphs: paragraphs, precise: true, sentences: sentences) else { return 0 }
        return paragraphs[mark.paragraph].textStart + (mark.sentence?.location ?? 0)
    }
}
