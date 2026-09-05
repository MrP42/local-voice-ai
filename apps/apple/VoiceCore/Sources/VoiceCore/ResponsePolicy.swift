import Foundation

public enum ResponsePolicy {
    public static let capabilityReply = "Ich kann hier antworten und Notizen speichern. Eine externe Aktion habe ich nicht ausgeführt."
    private static func normalized(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de-DE"))
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
    private static func matches(_ pattern: String, _ text: String) -> Bool {
        text.range(of: pattern, options: .regularExpression) != nil
    }
    public static func isActionRequest(_ text: String) -> Bool {
        let text = normalized(text)
        let verbs = #"(stell(?:e|en)?|setz(?:e|en)?|weck(?:e|en)?|erinner(?:e|n)?|schick(?:e|en)?|send(?:e|en)?|ruf(?:e|en)?|buch(?:e|en)?|bestell(?:e|en)?|offne(?:n)?|losch(?:e|en)?|trag(?:e|en)?|plan(?:e|en)?|reservier(?:e|en)?|uberweis(?:e|en)?)"#
        return matches(#"^(?:bitte\s+)?"# + verbs + #"\b"#, text)
            || (matches(#"^(?:kannst|konntest|wurdest)\s+du\b"#, text) && matches(#"\b"# + verbs + #"\b"#, text))
    }
    /// Conservative guard for the tested German action vocabulary, not a general semantic proof.
    public static func safeAnswer(_ answer: String) -> String {
        let text = normalized(answer)
        if matches(#"\b(gesendet|verschickt|geschickt|gestellt|reserviert|gebucht|bestellt|geloscht|eingetragen|eingerichtet|eingestellt|aktiviert|deaktiviert|erledigt|uberwiesen|abgesagt|ausgefuhrt|done|sent|booked|reserved)\b|alarm is set"#, text) {
            return capabilityReply
        }
        return answer
    }
}

public enum SignalAssessment: Sendable { case empty, silent, signal, invalid }
public enum AudioSignal {
    /// Rejects digital silence and non-finite PCM. Noise detection needs a separately evaluated VAD.
    public static func assess<S: Sequence>(_ samples: S) -> SignalAssessment where S.Element == Float {
        var count = 0, signal = false
        for sample in samples {
            guard sample.isFinite else { return .invalid }
            count += 1
            if abs(sample) > 0.00001 { signal = true }
        }
        return count == 0 ? .empty : signal ? .signal : .silent
    }
}
