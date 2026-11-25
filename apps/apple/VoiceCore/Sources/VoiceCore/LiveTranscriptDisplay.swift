import Foundation

/// Anzeige-Regeln für das Live-Transkript (lva#801 T3): Sprachkennzeichen und Stand der Feinfassung.
/// Reine Logik ohne SwiftUI, damit sie unter Linux testbar bleibt.
public enum LiveTranscriptDisplay {
    /// Hauptsprache der Besprechung: häufigste Sprache der verfeinerten Zeilen, bei Gleichstand die früheste.
    public static func mainLanguage(_ segments: [LiveSegment]) -> String? {
        var counts: [String: Int] = [:]
        var first: [String: Int] = [:]
        for (i, s) in segments.enumerated() where s.isFinal && s.refined {
            guard let code = normalized(s.language) else { continue }
            counts[code, default: 0] += 1
            if first[code] == nil { first[code] = i }
        }
        return counts.max { a, b in
            a.value != b.value ? a.value < b.value : first[a.key, default: 0] > first[b.key, default: 0]
        }?.key
    }

    /// Kennzeichen nur, wenn eine Zeile von der Hauptsprache abweicht; sonst würde jede Zeile markiert.
    public static func badge(for segment: LiveSegment, main: String?) -> String? {
        guard let code = normalized(segment.language), let main, code != main else { return nil }
        return code.uppercased()
    }

    /// Ausgeschriebener Sprachname für Bedienungshilfen, z. B. „Englisch“.
    public static func languageName(_ code: String, locale: Locale = Locale(identifier: "de_DE")) -> String {
        let lower = code.lowercased()
        return locale.localizedString(forLanguageCode: lower) ?? code.uppercased()
    }

    public enum Refinement: Equatable, Sendable {
        /// Alles verfeinert oder noch nichts gesagt.
        case idle
        /// Es gibt vorläufige Zeilen, die gleich genauer gelesen werden.
        case running(provisional: Int)
        /// Mindestens ein Abschnitt wurde zurückgestellt (Rückstand, Wärme); der Abschluss holt ihn nach.
        case deferred(provisional: Int)
    }

    public static func refinement(displayed: [LiveSegment], deferredRanges: Int) -> Refinement {
        let provisional = displayed.filter { $0.isFinal && !$0.refined }.count
        if deferredRanges > 0 { return .deferred(provisional: provisional) }
        return provisional > 0 ? .running(provisional: provisional) : .idle
    }

    public static func statusText(_ state: Refinement) -> String? {
        switch state {
        case .idle: return nil
        case .running: return "Feinfassung läuft. Helle Zeilen sind vorläufig und werden gleich genauer gelesen."
        case .deferred: return "Feinfassung teilweise zurückgestellt, weil das Gerät ausgelastet ist. Nach der Aufnahme wird sie nachgeholt."
        }
    }

    private static func normalized(_ code: String?) -> String? {
        guard let raw = code?.trimmingCharacters(in: .whitespaces).lowercased(), !raw.isEmpty, raw != "auto" else { return nil }
        // „de-DE“ und „de_DE“ zählen wie „de“.
        return String(raw.split(whereSeparator: { $0 == "-" || $0 == "_" }).first ?? Substring(raw))
    }
}
