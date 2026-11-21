import Foundation

// Ausspracheregeln (lva#753): siehe `apps/shared/assistenten/README.md` („Aussprache“) und
// `aussprache-pruefstaende.json`, Abschnitt `ersetzung`. Vorbild ist `aussprache.ts`.
// Verglichen werden Unicode-Skalare ohne Normalisierung, Muster oder Wortgrenzen.

/// Eine Ausspracheregel ersetzt `word` nur für die Sprachausgabe durch `pronunciation`.
public struct AssistantPronunciationRule: Equatable, Sendable {
    public var word: String // JSON "wort"
    public var pronunciation: String // JSON "aussprache"
    /// Unbekannte Felder des Eintrags (bleiben beim Schreiben erhalten).
    public var extra: [AssistantJSONMember] = []

    public init(word: String, pronunciation: String, extra: [AssistantJSONMember] = []) {
        self.word = word
        self.pronunciation = pronunciation
        self.extra = extra
    }

    public static func == (a: Self, b: Self) -> Bool {
        AssistantExact.equal(a.word, b.word)
            && AssistantExact.equal(a.pronunciation, b.pronunciation)
            && a.extra.sameMembers(as: b.extra)
    }
}

/// Ein Stück der Zerlegung: unveränderter Text oder die Aussprache eines Treffers.
public struct AssistantPronunciationPiece: Equatable, Sendable {
    public let text: String
    public let replaced: Bool

    public init(text: String, replaced: Bool) {
        self.text = text
        self.replaced = replaced
    }

    public static func == (a: Self, b: Self) -> Bool {
        AssistantExact.equal(a.text, b.text) && a.replaced == b.replaced
    }
}

public enum AssistantPronunciation {
    public static let maxRules = 50
    public static let textLengthRange = 1...60 // In Unicode-Skalaren.

    /// Steuerzeichen, Richtungssteuerung und die Zeichen < > [ ] sind abgewiesen.
    static func isForbidden(_ c: UInt32) -> Bool {
        AssistantProfilePatterns.isControl(c)
            || c == 0x061C || c == 0x200E || c == 0x200F
            || (0x202A...0x202E).contains(c) || (0x2066...0x2069).contains(c)
            || c == 0x3C || c == 0x3E || c == 0x5B || c == 0x5D
    }

    public static func pieces(
        _ text: String, rules: [AssistantPronunciationRule]
    ) -> [AssistantPronunciationPiece] {
        let scalars: [Unicode.Scalar] = Array(text.unicodeScalars)
        let candidates = Array(rules.enumerated()).map { entry in
            (index: entry.offset, rule: entry.element,
             word: Array(entry.element.word.unicodeScalars))
        }.filter { !$0.word.isEmpty }.sorted { a, b in
            if a.word.count != b.word.count {
                return a.word.count > b.word.count
            }
            return a.index < b.index
        }

        var result: [AssistantPronunciationPiece] = []
        var open = 0
        var position = 0
        while position < scalars.count {
            var matchedLength = 0
            for candidate in candidates {
                guard candidate.word.count <= scalars.count - position else { continue }
                var matches = true
                for offset in candidate.word.indices {
                    if scalars[position + offset].value != candidate.word[offset].value {
                        matches = false
                        break
                    }
                }
                guard matches else { continue }
                if open < position {
                    result.append(AssistantPronunciationPiece(
                        text: String(String.UnicodeScalarView(scalars[open..<position])),
                        replaced: false
                    ))
                }
                result.append(AssistantPronunciationPiece(
                    text: candidate.rule.pronunciation, replaced: true
                ))
                matchedLength = candidate.word.count
                break
            }
            if matchedLength == 0 {
                position += 1
            } else {
                // Nur im Eingabetext weitersuchen; eingesetzter Text bleibt unangetastet.
                position += matchedLength
                open = position
            }
        }
        if open < scalars.count {
            result.append(AssistantPronunciationPiece(
                text: String(String.UnicodeScalarView(scalars[open...])), replaced: false
            ))
        }
        return result
    }

    public static func apply(_ text: String, rules: [AssistantPronunciationRule]?) -> String {
        guard let rules, !rules.isEmpty else { return text }
        var scalars = String.UnicodeScalarView()
        for piece in pieces(text, rules: rules) {
            scalars.append(contentsOf: piece.text.unicodeScalars)
        }
        return String(scalars)
    }
}
