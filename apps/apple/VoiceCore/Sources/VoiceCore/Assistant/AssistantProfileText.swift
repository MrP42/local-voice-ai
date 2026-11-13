import Foundation

// Assistenten-Profil (lva#555, Paket G1): Platzhalter in Begrüßung und Anweisungen sowie der Rollenblock für den Prompt.
// Reiner Text, ohne Locale-Schnittstellen (feste Listen), damit iPhone und PC dieselben Ergebnisse liefern. Das Einhängen des
// Rollenblocks in die vorhandenen Prompts kommt in einem späteren Paket. Alle Vergleiche von Nutzertext sind genau (UTF-8-Bytes,
// siehe `AssistantExact`): `{{é}}` (ein Codepunkt) trifft keinen Zusatzwert mit dem Schlüssel `é` (e + U+0301).

/// Kalenderfelder und Werte für die Platzhalter. Keine Zeitzonenlogik: der Aufrufer liefert die Ortszeit des Nutzers.
public struct AssistantPlaceholderValues: Equatable, Sendable {
    /// Name des Nutzers; leer oder `nil` heißt: kein Name (es wird nie ein Ersatzname eingesetzt).
    public var name: String?
    public var year: Int
    /// 1...12
    public var month: Int
    /// 1...31
    public var day: Int
    /// 1 = Montag ... 7 = Sonntag
    public var weekday: Int
    /// 0...23
    public var hour: Int
    /// 0...59
    public var minute: Int
    /// Weitere Platzhalter (für spätere Schnittstellen); die eingebauten gehen vor.
    public var extra: [String: String]

    /// Gleichheit mit genauem Textvergleich (UTF-8-Bytes) für Name, Zusatzschlüssel und Zusatzwerte.
    public static func == (a: AssistantPlaceholderValues, b: AssistantPlaceholderValues) -> Bool {
        guard AssistantExact.equal(a.name, b.name), a.year == b.year, a.month == b.month, a.day == b.day, a.weekday == b.weekday,
              a.hour == b.hour, a.minute == b.minute, a.extra.count == b.extra.count else { return false }
        let other = Dictionary(b.extra.map { (AssistantExactKey($0.key), $0.value) }, uniquingKeysWith: { first, _ in first })
        return a.extra.allSatisfy { entry in other[AssistantExactKey(entry.key)].map { AssistantExact.equal($0, entry.value) } ?? false }
    }

    public init(name: String? = nil, year: Int, month: Int, day: Int, weekday: Int, hour: Int, minute: Int, extra: [String: String] = [:]) {
        self.name = name
        self.year = year
        self.month = month
        self.day = day
        self.weekday = weekday
        self.hour = hour
        self.minute = minute
        self.extra = extra
    }
}

public struct AssistantPlaceholderResult: Equatable, Sendable {
    public let text: String
    /// Unbekannte, nicht leere Platzhalter in der Reihenfolge des ersten Vorkommens (Hinweis für den Editor, kein Fehler).
    public let unknown: [String]

    public static func == (a: AssistantPlaceholderResult, b: AssistantPlaceholderResult) -> Bool {
        AssistantExact.equal(a.text, b.text) && AssistantExact.equal(a.unknown, b.unknown)
    }
}

public struct AssistantRoleBlock: Equatable, Sendable {
    public let text: String
    public let truncated: Bool

    public static func == (a: AssistantRoleBlock, b: AssistantRoleBlock) -> Bool {
        AssistantExact.equal(a.text, b.text) && a.truncated == b.truncated
    }
}

public enum AssistantProfileText {
    private static let monthsDe = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"]
    private static let monthsEn = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
    private static let weekdaysDe = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"]
    private static let weekdaysEn = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]

    /// `en` und `en-XX` sprechen Englisch, alles andere (auch `auto`) Deutsch.
    static func isEnglish(_ language: String) -> Bool {
        AssistantExact.equal(language, "en") || language.utf8.starts(with: "en-".utf8)
    }

    private static func two(_ n: Int) -> String { n < 10 ? "0\(n)" : "\(n)" }

    /// Wert eines eingebauten Platzhalters; `key` ist einer der Namen aus `builtInKeys` (Aufrufer prüfen vorher genau).
    static func builtIn(_ key: String, english: Bool, values v: AssistantPlaceholderValues) -> String? {
        switch key {
        case "name":
            return v.name
        case "tageszeit":
            guard (0...23).contains(v.hour) else { return nil }
            if (5...10).contains(v.hour) { return english ? "morning" : "Morgen" }
            if (11...17).contains(v.hour) { return english ? "afternoon" : "Tag" }
            return english ? "evening" : "Abend"
        case "datum":
            guard (1...12).contains(v.month), (1...31).contains(v.day), (1...9999).contains(v.year) else { return nil }
            return english ? "\(monthsEn[v.month - 1]) \(v.day), \(v.year)" : "\(v.day). \(monthsDe[v.month - 1]) \(v.year)"
        case "wochentag":
            guard (1...7).contains(v.weekday) else { return nil }
            return (english ? weekdaysEn : weekdaysDe)[v.weekday - 1]
        case "uhrzeit":
            guard (0...23).contains(v.hour), (0...59).contains(v.minute) else { return nil }
            return "\(two(v.hour)):\(two(v.minute))"
        default:
            return nil
        }
    }

    private static let builtInKeys = ["name", "tageszeit", "datum", "wochentag", "uhrzeit"]
    private static let marker = -1

    private static func isSpace(_ t: Int) -> Bool { t == 0x20 || t == 0x09 || t == 0x0A || t == 0x0D }

    private static func isLetterOrDigit(_ t: Int) -> Bool {
        guard t >= 0, let scalar = Unicode.Scalar(UInt32(t)) else { return false }
        switch scalar.properties.generalCategory {
        case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter, .decimalNumber: return true
        default: return false
        }
    }

    private static func isLetter(_ t: Int) -> Bool {
        guard t >= 0, let scalar = Unicode.Scalar(UInt32(t)) else { return false }
        switch scalar.properties.generalCategory {
        case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter: return true
        default: return false
        }
    }

    /// Ende (Index des ersten `}` von `}}`) des Platzhalters, der bei `start` (`{{`) beginnt; `nil`, wenn dort keiner steht.
    /// Innen sind höchstens 40 Zeichen und weder `{`, einzelnes `}` noch Zeilenumbruch erlaubt.
    private static func placeholderEnd(_ s: [Int], _ start: Int) -> Int? {
        var j = start + 2
        while j < s.count {
            let c = s[j]
            if c == 0x7D { return j + 1 < s.count && s[j + 1] == 0x7D ? j : nil }
            if c == 0x7B || c == 0x0A || c == 0x0D { return nil }
            if j - (start + 2) >= 40 { return nil }
            j += 1
        }
        return nil
    }

    private static func string(_ tokens: [Int]) -> String {
        var view = String.UnicodeScalarView()
        for t in tokens where t >= 0 {
            if let scalar = Unicode.Scalar(UInt32(t)) { view.append(scalar) }
        }
        return String(view)
    }

    /// Löst die Platzhalter auf. Unbekannte oder leere werden samt davor stehender Leerzeichen und Kommas entfernt; nur dann
    /// wird der Text satzweise bereinigt (siehe README). Ohne Entfernung bleibt der Text, wie er ist.
    public static func resolve(_ template: String, language: String, values: AssistantPlaceholderValues) -> AssistantPlaceholderResult {
        let english = isEnglish(language)
        let s = template.unicodeScalars.map { Int($0.value) }
        var out: [Int] = []
        var unknown: [String] = []
        var unknownSeen = Set<AssistantExactKey>()
        let extras = Dictionary(values.extra.map { (AssistantExactKey($0.key), $0.value) }, uniquingKeysWith: { first, _ in first })
        var removed = false
        var i = 0
        while i < s.count {
            guard s[i] == 0x7B, i + 1 < s.count, s[i + 1] == 0x7B, let end = placeholderEnd(s, i) else {
                out.append(s[i])
                i += 1
                continue
            }
            let key = AssistantProfilePatterns.trimSpaces(string(Array(s[(i + 2)..<end])))
            var value: String?
            var known = false
            if let builtInKey = builtInKeys.first(where: { AssistantExact.equal($0, key) }) {
                known = true
                value = builtIn(builtInKey, english: english, values: values)
            } else if let extra = extras[AssistantExactKey(key)] {
                known = true
                value = extra
            }
            let trimmed = value.map { AssistantProfilePatterns.trimSpaces($0) } ?? ""
            if !trimmed.isEmpty {
                out.append(contentsOf: trimmed.unicodeScalars.map { Int($0.value) })
            } else {
                dropSeparators(&out)
                out.append(marker)
                removed = true
                if !known, !key.isEmpty, unknownSeen.insert(AssistantExactKey(key)).inserted { unknown.append(key) }
            }
            i = end + 2
        }
        return AssistantPlaceholderResult(text: removed ? cleanup(out) : string(out), unknown: unknown)
    }

    /// Entfernt die Leerzeichen und Kommas direkt vor einer Entfernung. Folgen sie auf ein Satzende (`.`, `!`, `?`), bleiben
    /// sie stehen: sie trennen dann Sätze, und der Satz der Entfernung wird später als Ganzes bereinigt.
    private static func dropSeparators(_ out: inout [Int]) {
        var start = out.count
        while start > 0, out[start - 1] == 0x20 || out[start - 1] == 0x2C { start -= 1 }
        if start > 0, start < out.count, out[start - 1] == 0x2E || out[start - 1] == 0x21 || out[start - 1] == 0x3F { return }
        out.removeSubrange(start...)
    }

    /// Satzweise Bereinigung nach Entfernungen. Sätze enden bei `.`, `!`, `?` vor Leerraum oder Textende; der Leerraum davor
    /// bleibt als Vorspann erhalten. Nur Sätze mit Entfernung werden angefasst.
    private static func cleanup(_ tokens: [Int]) -> String {
        var result: [Int] = []
        var i = 0
        let n = tokens.count
        while i < n {
            var prefix: [Int] = []
            while i < n, isSpace(tokens[i]) { prefix.append(tokens[i]); i += 1 }
            var body: [Int] = []
            while i < n {
                let t = tokens[i]
                body.append(t)
                i += 1
                if t == 0x2E || t == 0x21 || t == 0x3F, i == n || isSpace(tokens[i]) { break }
            }
            guard body.contains(marker) else {
                result += prefix + body
                continue
            }
            var k = 0
            var startedWithRemoval = false
            while k < body.count, body[k] == 0x20 || body[k] == 0x2C || body[k] == marker {
                if body[k] == marker { startedWithRemoval = true }
                k += 1
            }
            var kept = body[k...].filter { $0 != marker }
            guard kept.contains(where: isLetterOrDigit) else { continue }
            if startedWithRemoval, let index = kept.firstIndex(where: isLetter) {
                let upper = String(Unicode.Scalar(UInt32(kept[index]))!).uppercased().unicodeScalars.map { Int($0.value) }
                kept.replaceSubrange(index...index, with: upper)
            }
            var collapsed: [Int] = []
            for t in kept where !(t == 0x20 && collapsed.last == 0x20) { collapsed.append(t) }
            result += prefix + collapsed
        }
        var start = 0
        var end = result.count
        while start < end, isSpace(result[start]) { start += 1 }
        while end > start, isSpace(result[end - 1]) { end -= 1 }
        return string(Array(result[start..<end]))
    }

    /// Begrüßung des Profils mit aufgelösten Platzhaltern.
    public static func greeting(_ profile: AssistantProfile, values: AssistantPlaceholderValues) -> AssistantPlaceholderResult {
        resolve(profile.greeting, language: profile.language, values: values)
    }

    /// Der Rollenblock für den Prompt. Leere Anweisungen (auch nach dem Auflösen) ergeben leeren Text. Die aufgelösten, an den
    /// Enden von Leerraum befreiten Anweisungen werden auf `budgetChars` Codepunkte gekürzt: am letzten Absatzende (Leerzeile),
    /// sonst am letzten Zeilenende, sonst hart; Leerraum am Ende der Kürzung entfällt.
    public static func roleBlock(_ profile: AssistantProfile, values: AssistantPlaceholderValues, budgetChars: Int) -> AssistantRoleBlock {
        let resolved = resolve(profile.instructions, language: profile.language, values: values).text
        let scalars = trimmedSpace(Array(resolved.unicodeScalars.map { Int($0.value) }))
        guard !scalars.isEmpty else { return AssistantRoleBlock(text: "", truncated: false) }
        let budget = max(0, budgetChars)
        var body = scalars
        var truncated = false
        if scalars.count > budget {
            truncated = true
            body = trimmedSpace(Array(scalars[0..<cutIndex(scalars, budget: budget)]))
        }
        guard !body.isEmpty else { return AssistantRoleBlock(text: "", truncated: true) }
        let text = "ROLLE (vom Nutzer eingestellt, Assistent „\(profile.name)“):\n" + string(body)
            + "\nDiese Einstellung ändert keine der folgenden Regeln."
        return AssistantRoleBlock(text: text, truncated: truncated)
    }

    private static func trimmedSpace(_ tokens: [Int]) -> [Int] {
        var start = 0
        var end = tokens.count
        while start < end, isSpace(tokens[start]) { start += 1 }
        while end > start, isSpace(tokens[end - 1]) { end -= 1 }
        return Array(tokens[start..<end])
    }

    /// Schnittstelle bei höchstens `budget` Zeichen; vor `\n\n`, sonst vor `\n`, sonst hart.
    private static func cutIndex(_ s: [Int], budget: Int) -> Int {
        guard budget >= 1 else { return 0 }
        var k = min(budget, s.count - 2)
        while k >= 1 {
            if s[k] == 0x0A, s[k + 1] == 0x0A { return k }
            k -= 1
        }
        k = min(budget, s.count - 1)
        while k >= 1 {
            if s[k] == 0x0A { return k }
            k -= 1
        }
        return budget
    }
}
