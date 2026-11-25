// nach Yogurt (MIT), FrigadeHQ — FrigadeHQ/yogurt @76e5595
import Foundation

public enum LiveHallucination {
    public static func tokens(_ text: String) -> [String] {
        text.precomposedStringWithCompatibilityMapping.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty }
    }

    public static func isPromptEcho(_ text: String, prompt: String) -> Bool {
        let t = tokens(text), p = tokens(prompt)
        guard t.count >= 2, t.count <= p.count + 1 else { return false }
        var run = 0
        for a in t.indices {
            for i in p.indices {
                var n = 0
                while a + n < t.count && i + n < p.count {
                    let k = a + n, word = p[i + n]
                    guard word == t[k] || (k == t.count - 1 && t[k].count >= 3 && word.hasPrefix(t[k])) else { break }
                    n += 1
                }
                run = max(run, n)
            }
        }
        return run == t.count || (run >= 4 && t.count - run <= 1)
    }

    public static func isGarbled(_ text: String) -> Bool {
        let words = tokens(text)
        if words.isEmpty { return true }
        // Drei gleiche Wortgruppen in Folge, die mindestens drei Viertel ausmachen.
        for size in 1...max(1, words.count / 3) {
            guard words.count >= size * 3 else { continue }
            for start in 0...(words.count - size * 3) {
                let phrase = Array(words[start..<start + size])
                var end = start + size
                while end + size <= words.count && Array(words[end..<end + size]) == phrase { end += size }
                if end - start >= size * 3 && (end - start) * 4 >= words.count * 3 { return true }
            }
        }
        return false
    }

    private static let guesses: Set<String> = Set([
        "you", "thank you", "thank you very much", "thanks", "thanks for watching", "thank you for watching",
        "thanks for watching bye", "thank you bye", "bye", "bye bye", "please subscribe", "like and subscribe",
        "vielen dank", "danke", "danke fürs zuschauen", "vielen dank fürs zuschauen", "auf wiedersehen",
        "merci", "merci beaucoup", "merci de votre attention", "merci d avoir regardé", "au revoir",
        "gracias", "muchas gracias", "gracias por ver", "hasta luego", "grazie", "grazie mille", "arrivederci",
        "bedankt", "dank u", "bedankt voor het kijken", "tot ziens", "dziękuję", "dziękuję za uwagę",
        "dziękuję za oglądanie", "teşekkürler", "teşekkür ederim", "izlediğiniz için teşekkürler",
        "for more information visit www fema org", "ご視聴ありがとうございました", "продолжение следует"
    ].map { tokens($0).joined(separator: " ") })

    public static func isSubtitleCredit(_ text: String) -> Bool {
        let text = text.precomposedStringWithCompatibilityMapping.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.count <= 80 else { return false }
        return [
            #"^(?:subtitles|subtitled|captions|captioned|captioning) by\b"#,
            #"^untertitel(?:ung)?\s+(?:im auftrag|des|der|von)\b"#,
            #"^sous[- ]titr(?:age|es?)\b(?!\s+(?:sont|est|en|du|de la|pour)\b)"#,
            #"^sottotitoli (?:creati|a cura|di|e revisione)\b"#,
            #"^subtítulos (?:realizados|por|hechos|creados)\b"#,
            #"^napisy (?:wykonane|stworzone|przygotowa)"#,
            #"^ondertiteling (?:door|van)\b"#,
            #"^altyaz[ıi] (?:haz[ıi]rlayan|çeviri)\b"#,
            #"^редактор субтитров\b"#, #"^字幕(?:由|提供|制作|製作)"#
        ].contains { text.range(of: $0, options: [.regularExpression, .caseInsensitive]) != nil }
    }

    public static func withoutPromptLabel(_ text: String, prompt: String) -> String {
        guard let colon = text.firstIndex(of: ":"), text[..<colon].count <= 120 else { return text }
        let label = tokens(String(text[..<colon])), p = Set(tokens(prompt))
        guard !label.isEmpty, label.allSatisfy({ p.contains($0) }) else { return text }
        return String(text[text.index(after: colon)...]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Stille Nähe entspricht der unteren RMS-Schwelle des Live-Chunkers.
    public static func nearSilence(_ samples: [Float]) -> Bool {
        guard !samples.isEmpty, samples.allSatisfy({ $0.isFinite }) else { return false }
        return sqrt(samples.reduce(0.0) { $0 + Double($1) * Double($1) } / Double(samples.count)) <= 0.0015
    }

    public static func clean(_ text: String, prompt: String, clearMs: Int,
                             wholeSectionIsEcho: Bool = false, nearSilence: Bool = false) -> String? {
        let line = withoutPromptLabel(text.components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " "), prompt: prompt)
        if isGarbled(line) || isSubtitleCredit(line) { return nil }
        if isPromptEcho(line, prompt: prompt) && (clearMs < 300 || (wholeSectionIsEcho && nearSilence)) { return nil }
        if clearMs < 300 && guesses.contains(tokens(line).joined(separator: " ")) { return nil }
        return line
    }
}
