import Foundation

/// Zusatzwörter nur aus Bestandsmetadaten, niemals aus Textauszügen.
public enum AssistantVocabulary {
    public static func contextualStrings(items: [AssistantItem], limit: Int = 100) -> [String] {
        guard limit > 0 else { return [] }
        let ordered = items.enumerated().sorted { lhs, rhs in
            if (lhs.element.area == .books) != (rhs.element.area == .books) {
                return lhs.element.area == .books
            }
            let left = lhs.element.updated ?? .distantPast
            let right = rhs.element.updated ?? .distantPast
            return left == right ? lhs.offset < rhs.offset : left > right
        }
        var seen = Set<String>()
        var words: [String] = []
        for entry in ordered {
            let item = entry.element
            var candidates = [item.title]
            if item.area == .books, let detail = item.detail {
                let authors = detail.components(separatedBy: "·")[0]
                candidates += authors.components(separatedBy: ",").filter { !$0.contains("%") }
            }
            for candidate in candidates {
                let clean = candidate.components(separatedBy: .controlCharacters)
                    .joined(separator: " ")
                    .split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
                let word = String(clean.precomposedStringWithCanonicalMapping.prefix(64))
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                guard !word.isEmpty, seen.insert(word.lowercased()).inserted else { continue }
                words.append(word)
                if words.count == min(limit, 100) { return words }
            }
        }
        return words
    }
}
