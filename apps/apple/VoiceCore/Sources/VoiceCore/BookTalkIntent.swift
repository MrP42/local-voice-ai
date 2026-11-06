import Foundation

// Buch-Gespräch (Epic #307, Paket G4): Absichtserkennung und Sprachbefehle.
// Reine Regeln (de + en), kein Modell. Gleiche Listen wie der Rust-Kern (G1).

public enum BookTalkIntent: String, Sendable, Codable, Equatable, CaseIterable {
    case recap, locate, explain, question
}

public enum BookTalkScope: String, Sendable, Codable, Equatable {
    case untilPosition = "until_position"
    case book
}

public enum BookTalkLanguage: String, Sendable, Equatable {
    case de, en
}

enum BookTalkNormalize {
    /// Klein, ä→ae, ö→oe, ü→ue, ß→ss, Satzzeichen zu Leerzeichen, Leerraum gefaltet.
    static func words(_ s: String) -> [String] {
        var out = ""
        for ch in s.lowercased() {
            switch ch {
            case "ä": out += "ae"
            case "ö": out += "oe"
            case "ü": out += "ue"
            case "ß": out += "ss"
            case "'", "’", "‘": continue
            default: out += (ch.isLetter || ch.isNumber) ? String(ch) : " "
            }
        }
        return out.split(separator: " ").map(String.init)
    }
}

public enum BookTalkIntents {
    private static let recap = [
        "bisher", "was ist passiert", "was passiert ist", "was geschah", "verpasst", "zusammenfass", "zusammengefasst",
        "so far", "recap", "what happened", "summari", "i missed", "have i missed", "catch me up",
    ]
    private static let locateWords: Set<String> = ["wo", "where"]
    private static let locate = [
        "an welcher stelle", "welche seite", "welcher seite", "auf welcher seite", "zum ersten mal", "erstmals",
        "wann wird", "which page", "what page", "first time", "first mention", "which chapter", "welches kapitel",
    ]
    private static let explain = [
        "erklaer", "was bedeutet", "was heisst", "was versteht man", "gemeint", "bedeutung", "was ist ein ",
        "was ist eine ", "explain", "what does", "what is meant", "what do you mean", "meaning of", "define ",
    ]

    /// Reihenfolge: recap, locate, explain, question. „Wo wird das erklärt?“ ist locate.
    public static func detect(_ text: String) -> BookTalkIntent {
        let words = BookTalkNormalize.words(text)
        let flat = " " + words.joined(separator: " ") + " "
        if recap.contains(where: { flat.contains($0) }) { return .recap }
        if words.contains(where: { locateWords.contains($0) }) || locate.contains(where: { flat.contains($0) }) { return .locate }
        if explain.contains(where: { flat.contains($0) }) { return .explain }
        return .question
    }

    private static let germanHints: Set<String> = [
        "der", "die", "das", "und", "ist", "nicht", "was", "wer", "wo", "wie", "warum", "ich", "mir", "mein", "eine", "einen",
        "erklaer", "bitte", "von", "mit", "wird", "sind", "hat", "kapitel", "buch", "bisher", "gibt", "es", "zu", "im", "dem",
    ]
    private static let englishHints: Set<String> = [
        "the", "and", "is", "not", "what", "who", "where", "how", "why", "me", "my", "explain", "please", "of", "with",
        "are", "has", "chapter", "book", "far", "there", "does", "did", "in", "to", "it",
    ]

    /// Sprache der Frage (für System-Prompt und Sprechfassung); Standard Deutsch.
    public static func language(of text: String) -> BookTalkLanguage {
        let words = BookTalkNormalize.words(text)
        if text.contains(where: { "äöüÄÖÜß".contains($0) }) { return .de }
        let de = words.filter { germanHints.contains($0) }.count
        let en = words.filter { englishHints.contains($0) }.count
        return en > de ? .en : .de
    }
}

public enum BookTalkCommand: String, Sendable, Codable, Equatable, CaseIterable {
    case jump
    case readCitation = "read_citation"
    case confirm
    case resume
    case scopeBook = "scope_book"
    case scopePosition = "scope_position"
    case stop
    case end
}

public enum BookTalkCommands {
    private static let fillers: Set<String> = [
        "bitte", "mal", "doch", "jetzt", "mir", "uns", "hey", "please", "just", "now", "kurz",
    ]

    /// Formen sind bereits normalisiert (ä→ae …); Vergleich über die ganze Äußerung.
    private static let table: [(BookTalkCommand, [String])] = [
        (.jump, [
            "spring dahin", "spring hin", "spring zur stelle", "zeig die stelle", "zeig sie", "geh zu der stelle",
            "geh zur stelle", "geh dahin", "gehe dahin", "jump there", "jump to it", "go there", "show the passage",
            "take me there", "show me the passage",
        ]),
        (.readCitation, [
            "lies die stelle vor", "lies das vor", "lies es vor", "lies sie vor", "lies vor", "lies die stelle",
            "vorlesen", "read it to me", "read it to us", "read it", "read that", "read it out", "read it aloud",
            "read the passage", "read the passage to me", "read that to me",
        ]),
        (.confirm, ["ja", "jo", "jawohl", "gerne", "klar", "ja gerne", "ja klar", "yes", "yeah", "sure", "okay", "yes please"]),
        (.resume, [
            "weiterlesen", "lies weiter", "weiter", "mach weiter", "mach weiter wo ich war", "mach weiter wo ich war",
            "keep reading", "continue", "continue reading", "resume", "read on", "carry on",
        ]),
        (.scopeBook, [
            "ganzes buch", "das ganze buch", "auch spaetere kapitel", "ganzes buch erlauben", "whole book", "the whole book",
            "entire book", "full book", "include later chapters", "also later chapters",
        ]),
        (.scopePosition, [
            "nur bis hier", "bis hier", "nur bis zur leseposition", "nur bis zur aktuellen stelle", "spoilerschutz an",
            "only up to here", "up to here", "only until here", "no spoilers",
        ]),
        (.stop, ["stopp", "stop", "halt", "halt stopp", "sei still", "ruhe", "hoer auf", "aufhoeren", "be quiet", "shut up"]),
        (.end, [
            "gespraech beenden", "beende das gespraech", "beende gespraech", "gespraech ende", "ende", "tschuess",
            "end conversation", "end the conversation", "goodbye", "bye",
        ]),
    ]

    private static let lookup: [String: BookTalkCommand] = {
        var d: [String: BookTalkCommand] = [:]
        for (cmd, forms) in table { for f in forms { d[f] = cmd } }
        return d
    }()

    /// Ganze Äußerung muss ein Befehl sein; alles mit zusätzlicher Frage ist keiner.
    public static func detect(_ text: String) -> BookTalkCommand? {
        let words = BookTalkNormalize.words(text).filter { !fillers.contains($0) }
        guard !words.isEmpty, words.count <= 8 else { return nil }
        return lookup[words.joined(separator: " ")]
    }

    /// „Ja“ ist nur nach der Rückfrage „Soll ich dir die Stelle vorlesen?“ ein Befehl, sonst eine Frage.
    public static func effective(_ text: String, awaitingReadOffer: Bool) -> BookTalkCommand? {
        guard let c = detect(text) else { return nil }
        guard c == .confirm else { return c }
        return awaitingReadOffer ? .readCitation : nil
    }
}
