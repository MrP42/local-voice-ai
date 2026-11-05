import Foundation

/// Text, wie er an die Stimme geht (#314, Runde 2). Angezeigt und hervorgehoben wird
/// weiter das Original.
///
/// Abkürzungen werden ausgeschrieben: Phonon teilt seinen Text an jedem Punkt in
/// Abschnitte und spricht jeden wie einen eigenen Satz, mit fallender Stimme und Pause.
/// „z. B.“ klang so wie zwei Satzenden.
public enum SpeechText {
    private static let german: [(String, String)] = [
        (#"z\.\s?B\."#, "zum Beispiel"), (#"d\.\s?h\."#, "das heißt"), (#"u\.\s?a\."#, "unter anderem"),
        (#"u\.\s?U\."#, "unter Umständen"), (#"z\.\s?T\."#, "zum Teil"), (#"o\.\s?Ä\."#, "oder Ähnliches"),
        (#"usw\."#, "und so weiter"), (#"bzw\."#, "beziehungsweise"), (#"ca\."#, "circa"),
        (#"vgl\."#, "vergleiche"), (#"evtl\."#, "eventuell"), (#"ggf\."#, "gegebenenfalls"),
        (#"inkl\."#, "inklusive"), (#"etc\."#, "et cetera"), (#"Dr\."#, "Doktor"),
        (#"Prof\."#, "Professor"), (#"Nr\.(?=\s?\d)"#, "Nummer"), (#"S\.(?=\s?\d)"#, "Seite"),
        (#"Hr\."#, "Herr"), (#"Fr\.(?=\s\p{Lu})"#, "Frau"), (#"Mio\."#, "Millionen"),
        (#"Mrd\."#, "Milliarden"), (#"Jh\."#, "Jahrhundert"),
    ]
    private static let english: [(String, String)] = [
        (#"e\.\s?g\."#, "for example"), (#"i\.\s?e\."#, "that is"), (#"etc\."#, "et cetera"),
        (#"vs\."#, "versus"), (#"Mr\."#, "Mister"), (#"Mrs\."#, "Missus"), (#"Dr\."#, "Doctor"),
        (#"No\.(?=\s?\d)"#, "number"),
    ]

    private static let compiled: [String: [(NSRegularExpression, String)]] = {
        func build(_ list: [(String, String)]) -> [(NSRegularExpression, String)] {
            list.compactMap { pattern, spoken in
                // Nur am Wortanfang: „Erz. B.“ oder „Stand.“ bleiben stehen.
                (try? NSRegularExpression(pattern: #"(?<![\p{L}\p{N}])"# + pattern)).map { ($0, spoken) }
            }
        }
        return ["de": build(german), "en": build(english)]
    }()

    /// Sprechfassung von `text` in `language` („de“, „en“).
    public static func spoken(_ text: String, language: String) -> String {
        guard let rules = compiled[language] else { return text }
        var result = text
        for (regex, spoken) in rules {
            let all = result as NSString
            var out = ""
            var last = 0
            for m in regex.matches(in: result, range: NSRange(location: 0, length: all.length)) {
                out += all.substring(with: NSRange(location: last, length: m.range.location - last))
                out += spoken
                last = m.range.location + m.range.length
                // Am Satzende war der Abkürzungspunkt zugleich der Schlusspunkt.
                let rest = all.substring(from: last).trimmingCharacters(in: .whitespacesAndNewlines)
                if rest.isEmpty || rest.first.map({ "\"“”»«'’)".contains($0) && rest.count <= 2 }) == true { out += "." }
            }
            out += all.substring(from: last)
            result = out
        }
        return result
    }
}
