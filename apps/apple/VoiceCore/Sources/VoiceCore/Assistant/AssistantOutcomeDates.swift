import Foundation

// Auswertung am Gesprächsende (lva#559, Paket G5-Kern): Textnormalisierung für Vergleiche, Zahlwörter und die Fristauflösung.
// Reine Funktionen ohne Zeitzonenlogik (`heute` kommt als Jahr/Monat/Tag), deutsch und englisch aus einer festen Liste.
// Gegenstück am PC: `assistenten/auswertungFrist.ts`; die gemeinsamen Prüfstände stehen in
// `apps/shared/assistenten/auswertung-pruefstaende.json`.
//
// Vergleiche von Zeichenketten laufen auf Codepunkten oder UTF-8-Bytes, nie über `String ==` (Swift setzt dort kanonische
// Gleichheit an, JavaScript nicht; beide Kerne müssen dasselbe ergeben).

/// Kalendertag ohne Zeitzone.
public struct AssistantOutcomeDate: Equatable, Sendable {
    public var year: Int
    public var month: Int
    public var day: Int

    public init(year: Int, month: Int, day: Int) {
        self.year = year
        self.month = month
        self.day = day
    }
}

// MARK: - Kalender

extension AssistantOutcomeDate {
    static func isLeap(_ year: Int) -> Bool { (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 }

    static func daysIn(year: Int, month: Int) -> Int {
        if month == 2 { return isLeap(year) ? 29 : 28 }
        return [4, 6, 9, 11].contains(month) ? 30 : 31
    }

    /// Gültiger Kalendertag in den Jahren 1000 bis 9999.
    public var isValid: Bool {
        year >= 1000 && year <= 9999 && month >= 1 && month <= 12 && day >= 1 && day <= AssistantOutcomeDate.daysIn(year: year, month: month)
    }

    /// Tage seit 1970-01-01 (proleptisch gregorianisch).
    var dayNumber: Int {
        let y = month <= 2 ? year - 1 : year
        let era = Int((Double(y) / 400).rounded(.down))
        let yoe = y - era * 400
        let doy = (153 * (month + (month > 2 ? -3 : 9)) + 2) / 5 + day - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        return era * 146_097 + doe - 719_468
    }

    init(dayNumber n: Int) {
        let z = n + 719_468
        let era = Int((Double(z) / 146_097).rounded(.down))
        let doe = z - era * 146_097
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let mp = (5 * doy + 2) / 153
        let d = doy - (153 * mp + 2) / 5 + 1
        let m = mp < 10 ? mp + 3 : mp - 9
        self.init(year: yoe + era * 400 + (m <= 2 ? 1 : 0), month: m, day: d)
    }

    /// 1 = Montag ... 7 = Sonntag (wie `AssistantPlaceholderValues.weekday`).
    public var weekday: Int { (((dayNumber + 3) % 7) + 7) % 7 + 1 }

    /// `JJJJ-MM-TT`
    public var formatted: String {
        func pad(_ n: Int, _ width: Int) -> String {
            let s = String(n)
            return String(repeating: "0", count: max(0, width - s.count)) + s
        }
        return "\(pad(year, 4))-\(pad(month, 2))-\(pad(day, 2))"
    }

    /// Strenges `JJJJ-MM-TT` mit gültigem Kalendertag; sonst `nil`.
    public static func parseISO(_ s: String) -> AssistantOutcomeDate? {
        let b = Array(s.utf8)
        guard b.count == 10, b[4] == 0x2D, b[7] == 0x2D else { return nil }
        let y = Array(b[0..<4]), m = Array(b[5..<7]), d = Array(b[8..<10])
        guard AssistantOutcomeText.onlyDigits(y), AssistantOutcomeText.onlyDigits(m), AssistantOutcomeText.onlyDigits(d) else { return nil }
        let date = AssistantOutcomeDate(year: AssistantOutcomeText.int(y), month: AssistantOutcomeText.int(m), day: AssistantOutcomeText.int(d))
        return date.isValid ? date : nil
    }
}

// MARK: - Text

public enum AssistantOutcomeText {
    private static let whitespace: Set<UInt32> = [0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0xA0]
    private static let quotes: Set<UInt32> = [
        0x22, 0x27, 0x60, 0xB4, 0xAB, 0xBB, 0x2018, 0x2019, 0x201A, 0x201B, 0x201C, 0x201D, 0x201E, 0x201F, 0x2039, 0x203A,
    ]
    private static let edge: Set<UInt32> = [0x2E, 0x2C, 0x3B, 0x3A, 0x21, 0x3F, 0x2026, 0x28, 0x29, 0x5B, 0x5D, 0x7B, 0x7D, 0x2D, 0x2013, 0x2014]

    static func isWhitespace(_ c: UInt32) -> Bool { whitespace.contains(c) }
    static func scalars(_ s: String) -> [UInt32] { s.unicodeScalars.map(\.value) }

    static func string(_ scalars: [UInt32]) -> String {
        var view = String.UnicodeScalarView()
        for c in scalars { if let scalar = Unicode.Scalar(c) { view.append(scalar) } }
        return String(view)
    }

    static func string(_ scalars: ArraySlice<UInt32>) -> String { string(Array(scalars)) }

    static func same(_ a: String, _ b: String) -> Bool { a.utf8.elementsEqual(b.utf8) }

    static func onlyDigits(_ b: [UInt8]) -> Bool { !b.isEmpty && b.allSatisfy { (0x30...0x39).contains($0) } }
    static func int(_ b: [UInt8]) -> Int { b.reduce(0) { $0 * 10 + Int($1 - 0x30) } }

    private static func lowered(_ c: UInt32) -> [UInt32] {
        if c < 0x80 { return [c >= 0x41 && c <= 0x5A ? c + 32 : c] }
        guard let scalar = Unicode.Scalar(c) else { return [c] }
        return scalar.properties.lowercaseMapping.unicodeScalars.map(\.value)
    }

    /// Normalisierung für Vergleiche: Kleinschreibung (je Codepunkt), Anführungszeichen entfernen, Leerraum (U+0009–U+000D,
    /// U+0020, U+00A0) zu einem Leerzeichen zusammenfassen, Leerraum und Satzzeichen (`. , ; : ! ? … ( ) [ ] { } - – —`) an den
    /// Rändern entfernen. Innen bleiben Satzzeichen stehen.
    static func normalizedScalars(_ s: String) -> [UInt32] {
        var out: [UInt32] = []
        for c in s.unicodeScalars.map(\.value) {
            if quotes.contains(c) { continue }
            if whitespace.contains(c) {
                if let last = out.last, last != 0x20 { out.append(0x20) }
                continue
            }
            out.append(contentsOf: lowered(c))
        }
        func isEdge(_ c: UInt32) -> Bool { c == 0x20 || edge.contains(c) }
        var start = 0
        var end = out.count
        while start < end, isEdge(out[start]) { start += 1 }
        while end > start, isEdge(out[end - 1]) { end -= 1 }
        return Array(out[start..<end])
    }

    public static func normalize(_ s: String) -> String { string(normalizedScalars(s)) }

    /// Entfernt Leerraum an den Enden (dieselbe Menge wie bei der Normalisierung).
    static func trimWhitespace(_ s: String) -> String {
        let c = scalars(s)
        var start = 0
        var end = c.count
        while start < end, whitespace.contains(c[start]) { start += 1 }
        while end > start, whitespace.contains(c[end - 1]) { end -= 1 }
        return string(c[start..<end])
    }

    /// Leerraum der Normalisierung plus die Zeilentrenner U+0085, U+2028 und U+2029.
    private static let lineBreaks: Set<UInt32> = whitespace.union([0x85, 0x2028, 0x2029])

    /// Fasst Leerraum und Zeilentrenner (U+0009–U+000D, U+0020, U+0085, U+00A0, U+2028, U+2029) zu einem Leerzeichen zusammen und
    /// kappt die Enden (einzeilige Texte im Prompt).
    static func singleLine(_ s: String) -> String {
        var out: [UInt32] = []
        for c in scalars(s) {
            if lineBreaks.contains(c) {
                if let last = out.last, last != 0x20 { out.append(0x20) }
            } else {
                out.append(c)
            }
        }
        while out.last == 0x20 { out.removeLast() }
        return string(out)
    }

    /// Ist `needle` ein zusammenhängender Teil von `haystack` (Codepunkte)?
    static func contains(_ haystack: [UInt32], _ needle: [UInt32]) -> Bool {
        if needle.isEmpty { return true }
        if needle.count > haystack.count { return false }
        // Knuth-Morris-Pratt: linear, auch bei absichtlich ungünstigen Texten
        var fail = [Int](repeating: 0, count: needle.count)
        var k = 0
        for i in 1..<max(needle.count, 1) {
            while k > 0, needle[i] != needle[k] { k = fail[k - 1] }
            if needle[i] == needle[k] { k += 1 }
            fail[i] = k
        }
        k = 0
        for c in haystack {
            while k > 0, c != needle[k] { k = fail[k - 1] }
            if c == needle[k] { k += 1 }
            if k == needle.count { return true }
        }
        return false
    }

    /// Wortzeichen eines normalisierten Textes: `a–z`, `0–9` und jeder Codepunkt ab U+0080 außer U+00A0–U+00BF (Zeichen und
    /// Satzzeichen des Latin-1-Blocks) und U+2000–U+206F (allgemeine Zeichensetzung). Alles andere trennt Wörter.
    static func isWordScalar(_ c: UInt32) -> Bool {
        if (0x61...0x7A).contains(c) || (0x30...0x39).contains(c) { return true }
        if c < 0x80 { return false }
        return !(0xA0...0xBF).contains(c) && !(0x2000...0x206F).contains(c)
    }

    /// Wörter eines normalisierten Textes: maximale Folgen von Wortzeichen.
    static func words(_ normalized: [UInt32]) -> [String] {
        var out: [String] = []
        var current: [UInt32] = []
        for c in normalized {
            if isWordScalar(c) {
                current.append(c)
            } else if !current.isEmpty {
                out.append(string(current))
                current = []
            }
        }
        if !current.isEmpty { out.append(string(current)) }
        return out
    }

    /// Zerlegt an Leerzeichen (leere Teile entfallen), höchstens `limit` Teile.
    static func tokens(_ normalized: [UInt32], limit: Int = Int.max) -> [String] {
        var out: [String] = []
        var start = 0
        var i = 0
        while i <= normalized.count, out.count < limit {
            if i == normalized.count || normalized[i] == 0x20 {
                if i > start { out.append(string(normalized[start..<i])) }
                start = i + 1
            }
            i += 1
        }
        return out
    }

    /// Die Fundstellen von `needle` in `haystack` (beide normalisiert): die Anfänge aller Vorkommen, die an Wortgrenzen beginnen
    /// und enden (vor einem Vorkommen, das mit einem Wortzeichen beginnt, und nach einem, das mit einem endet, steht kein
    /// Wortzeichen; `ja` hat in `jahr` keine Fundstelle). Aufsteigend, höchstens `limit` Stück; Vorkommen dürfen überlappen.
    static func occurrences(of needle: [UInt32], in haystack: [UInt32], limit: Int) -> [Int] {
        guard let first = needle.first, let last = needle.last, needle.count <= haystack.count, limit > 0 else { return [] }
        let leadWord = isWordScalar(first)
        let trailWord = isWordScalar(last)
        var fail = [Int](repeating: 0, count: needle.count)
        var k = 0
        for i in 1..<max(needle.count, 1) {
            while k > 0, needle[i] != needle[k] { k = fail[k - 1] }
            if needle[i] == needle[k] { k += 1 }
            fail[i] = k
        }
        var out: [Int] = []
        k = 0
        for (index, c) in haystack.enumerated() {
            while k > 0, c != needle[k] { k = fail[k - 1] }
            if c == needle[k] { k += 1 }
            guard k == needle.count else { continue }
            let start = index + 1 - needle.count
            let beforeOK = !leadWord || start == 0 || !isWordScalar(haystack[start - 1])
            let afterOK = !trailWord || index + 1 >= haystack.count || !isWordScalar(haystack[index + 1])
            if beforeOK && afterOK {
                out.append(start)
                if out.count >= limit { return out }
            }
            k = fail[k - 1]
        }
        return out
    }

    /// Steht `needle` als ganze Wortfolge in `haystack`? Wie ein zusammenhängender Teil, aber vor einem Treffer, der mit einem
    /// Wortzeichen beginnt, und nach einem, der mit einem endet, steht kein Wortzeichen (`ja` steht nicht in `jahr`).
    static func containsWordSequence(_ haystack: [UInt32], _ needle: [UInt32]) -> Bool {
        !occurrences(of: needle, in: haystack, limit: 1).isEmpty
    }

    /// Ein Satzzeichen, das die Normalisierung an den Rändern entfernt.
    static func isEdge(_ c: UInt32) -> Bool { edge.contains(c) }

    /// Die Grenzen der Nachbarschaft der Spanne `start..<end`: bis zu `words` ganze Wörter (an Leerzeichen getrennt) davor und
    /// danach; je Seite werden höchstens `limit` Codepunkte gelesen. Reicht das nicht und der Text geht dort weiter: `nil`.
    static func neighbourhood(_ c: [UInt32], start: Int, end: Int, words: Int, limit: Int) -> Range<Int>? {
        var i = start
        var read = 0
        var spaces = 0
        var from = -1
        while i > 0, read < limit {
            i -= 1
            read += 1
            if c[i] == 0x20 {
                spaces += 1
                if spaces == words + 1 {
                    from = i + 1
                    break
                }
            }
        }
        if from < 0 {
            if i > 0 { return nil }
            from = 0
        }
        i = end
        read = 0
        spaces = 0
        var to = -1
        while i < c.count, read < limit {
            let x = c[i]
            i += 1
            read += 1
            if x == 0x20 {
                spaces += 1
                if spaces == words + 1 {
                    to = i - 1
                    break
                }
            }
        }
        if to < 0 {
            if i < c.count { return nil }
            to = c.count
        }
        return from..<to
    }

    // MARK: Zahlwörter

    /// Zahlwörter null bis zwölf, deutsch und englisch (`ein`-Formen zählen als 1).
    static let numberWords: [(String, Int)] = [
        ("null", 0), ("zero", 0),
        ("ein", 1), ("eins", 1), ("eine", 1), ("einen", 1), ("einem", 1), ("einer", 1), ("eines", 1), ("one", 1),
        ("zwei", 2), ("two", 2), ("drei", 3), ("three", 3), ("vier", 4), ("four", 4),
        ("fünf", 5), ("fuenf", 5), ("five", 5), ("sechs", 6), ("six", 6), ("sieben", 7), ("seven", 7),
        ("acht", 8), ("eight", 8), ("neun", 9), ("nine", 9), ("zehn", 10), ("ten", 10),
        ("elf", 11), ("eleven", 11), ("zwölf", 12), ("zwoelf", 12), ("twelve", 12),
    ]

    static func numberWord(_ word: String) -> Int? {
        numberWords.first { same($0.0, word) }?.1
    }

    // MARK: Zahlen

    /// Vergleichsform: ohne führende Nullen vorn (mindestens eine Ziffer bleibt) und nachlaufende Nullen im Bruch.
    private static func comparable(_ whole: [UInt8], _ fraction: [UInt8]) -> String {
        var w = whole[...]
        while w.count > 1, w.first == 0x30 { w = w.dropFirst() }
        var f = fraction[...]
        while f.last == 0x30 { f = f.dropLast() }
        let wholeText = String(decoding: w, as: UTF8.self)
        return f.isEmpty ? wholeText : wholeText + "." + String(decoding: f, as: UTF8.self)
    }

    /// Kanonische Schreibweise: optionales `-`, Ganzzahlteil ohne führende Nullen (`0` allein bleibt), optional `.` und
    /// Nachkommastellen (unverändert). Ein `-` vor einem Betrag aus lauter Nullen entfällt (`-0` wird `0`).
    private static func canonicalNumber(sign: String, whole: [UInt8], fraction: [UInt8]) -> String {
        var w = whole[...]
        while w.count > 1, w.first == 0x30 { w = w.dropFirst() }
        let zero = w.count == 1 && w.first == 0x30 && fraction.allSatisfy { $0 == 0x30 }
        let wholeText = String(decoding: w, as: UTF8.self)
        let fractionText = fraction.isEmpty ? "" : "." + String(decoding: fraction, as: UTF8.self)
        return (zero ? "" : sign) + wholeText + fractionText
    }

    /// Ist `s` die kanonische Schreibweise einer Zahl (siehe `readNumber`)?
    static func isCanonicalNumber(_ s: String) -> Bool {
        let b = Array(s.utf8)
        let negative = b.first == 0x2D
        var i = negative ? 1 : 0
        let wholeStart = i
        while i < b.count, (0x30...0x39).contains(b[i]) { i += 1 }
        let whole = b[wholeStart..<i]
        guard !whole.isEmpty, whole.count == 1 || whole.first != 0x30 else { return false }
        var fraction: ArraySlice<UInt8> = []
        if i < b.count {
            guard b[i] == 0x2E else { return false }
            let fractionStart = i + 1
            i = fractionStart
            while i < b.count, (0x30...0x39).contains(b[i]) { i += 1 }
            fraction = b[fractionStart..<i]
            guard !fraction.isEmpty, i == b.count else { return false }
        }
        return !(negative && whole.count == 1 && whole.first == 0x30 && fraction.allSatisfy { $0 == 0x30 })
    }

    /// Ziffernfolgen, getrennt durch einzelne Zeichen aus `separators`; `nil`, wenn der Text nicht genau so aufgebaut ist.
    private static func digitGroups(_ b: [UInt8], separators: Set<UInt8>) -> (groups: [[UInt8]], separators: [UInt8])? {
        var groups: [[UInt8]] = []
        var seps: [UInt8] = []
        var current: [UInt8] = []
        for byte in b {
            if (0x30...0x39).contains(byte) {
                current.append(byte)
            } else if separators.contains(byte), !current.isEmpty {
                groups.append(current)
                seps.append(byte)
                current = []
            } else {
                return nil
            }
        }
        guard !current.isEmpty else { return nil }
        groups.append(current)
        return (groups, seps)
    }

    /// Kanonische Schreibweise einer Zahl als Text (Punkt als Dezimaltrenner, ohne Tausenderzeichen, ohne führende Nullen) und
    /// ihre Vergleichsform. Gelesen werden `-?\d+`, `-?\d+[.,]\d+`, `1.250,50`, `1,250.50` und Zahlwörter null bis zwölf; sonst `nil`.
    static func readNumber(_ s: String) -> (canonical: String, comparable: String)? {
        let trimmed = trimWhitespace(s)
        let all = Array(trimmed.utf8)
        let sign = all.first == 0x2D ? "-" : ""
        let body = sign.isEmpty ? all : Array(all.dropFirst())
        let dot: UInt8 = 0x2E, comma: UInt8 = 0x2C
        if let parsed = digitGroups(body, separators: [dot, comma]), parsed.groups.count <= 2 {
            let whole = parsed.groups[0]
            let fraction = parsed.groups.count == 2 ? parsed.groups[1] : []
            return (canonicalNumber(sign: sign, whole: whole, fraction: fraction), comparable(whole, fraction))
        }
        // Gruppierte Schreibweisen: `1.250,50` (Punkt-Gruppen, Komma-Bruch) und `1,250.50` (umgekehrt)
        for (group, decimal) in [(dot, comma), (comma, dot)] {
            guard let parsed = digitGroups(body, separators: [group, decimal]), parsed.groups.count >= 3 else { continue }
            let g = parsed.groups
            let s = parsed.separators
            guard s.last == decimal, s.dropLast().allSatisfy({ $0 == group }) else { continue }
            guard (1...3).contains(g[0].count), g[1..<(g.count - 1)].allSatisfy({ $0.count == 3 }) else { continue }
            let whole = g[0..<(g.count - 1)].flatMap { $0 }
            return (canonicalNumber(sign: sign, whole: whole, fraction: g[g.count - 1]), comparable(whole, g[g.count - 1]))
        }
        if sign.isEmpty, let n = numberWord(normalize(trimmed)) { return (String(n), String(n)) }
        return nil
    }

    /// Ein Zahl-Token eines Textes (Codepunkte `start..<end`): Ziffernfolgen, durch einzelne `.` oder `,` zwischen Ziffern verbunden.
    struct NumberToken {
        var start: Int
        var end: Int
        var forms: [String]
    }

    /// Die Zahl-Token eines normalisierten Textes mit ihren Vergleichsformen (leer, wenn die Folge keine Zahl ergibt, etwa `1.2.3`).
    static func numberTokens(_ c: [UInt32]) -> [NumberToken] {
        var out: [NumberToken] = []
        var i = 0
        func isDigit(_ x: UInt32) -> Bool { x >= 0x30 && x <= 0x39 }
        while i < c.count {
            if !isDigit(c[i]) {
                i += 1
                continue
            }
            let start = i
            var groups: [[UInt8]] = []
            var seps: [UInt32] = []
            while true {
                var j = i
                while j < c.count, isDigit(c[j]) { j += 1 }
                groups.append(c[i..<j].map { UInt8($0) })
                i = j
                if i + 1 < c.count, c[i] == 0x2E || c[i] == 0x2C, isDigit(c[i + 1]) {
                    seps.append(c[i])
                    i += 1
                } else {
                    break
                }
            }
            out.append(NumberToken(start: start, end: i, forms: candidates(groups, seps)))
        }
        return out
    }

    /// Deckt die Spanne `start..<end` des normalisierten `turn` im Kontext die Zahl mit der Vergleichsform `comparable`? Ein
    /// Zahl-Token, das die Spanne schneidet, wird in der Nachbarschaft gebildet (der Beleg `12` in `12,5 Liter` ergibt das Token
    /// `12,5` und deckt 12 nicht); gedeckt ist, wenn eines dieser Token den Wert hat oder ein Zahlwort null bis zwölf als ganzes
    /// Wort in der Spanne für ihn steht.
    static func numberCovered(_ turn: [UInt32], start: Int, end: Int, comparable cmp: String) -> Bool {
        guard let range = neighbourhood(turn, start: start, end: end, words: AssistantOutcomeDates.contextWords, limit: AssistantOutcomeDates.contextScalars) else {
            return false
        }
        let c = Array(turn[range])
        let s = start - range.lowerBound
        let e = end - range.lowerBound
        if numberTokens(c).contains(where: { $0.start < e && $0.end > s && $0.forms.contains { same($0, cmp) } }) { return true }
        let digits = Array(cmp.utf8)
        guard onlyDigits(digits), digits.count <= 2, int(digits) <= 12 else { return false }
        let value = int(digits)
        var i = 0
        while i < c.count {
            if !isWordScalar(c[i]) {
                i += 1
                continue
            }
            var j = i
            while j < c.count, isWordScalar(c[j]) { j += 1 }
            if i < e, j > s, numberWord(string(c[i..<j])) == value { return true }
            i = j
        }
        return false
    }

    private static func candidates(_ groups: [[UInt8]], _ seps: [UInt32]) -> [String] {
        if groups.count == 1 { return [comparable(groups[0], [])] }
        func thousands(_ parts: ArraySlice<[UInt8]>) -> Bool {
            guard let first = parts.first else { return false }
            return (1...3).contains(first.count) && parts.dropFirst().allSatisfy { $0.count == 3 }
        }
        var out: [String] = []
        let last = seps[seps.count - 1]
        if seps.allSatisfy({ $0 == seps[0] }) {
            if groups.count == 2 { out.append(comparable(groups[0], groups[1])) }
            if thousands(groups[...]) { out.append(comparable(groups.flatMap { $0 }, [])) }
            return out
        }
        // gemischte Trenner: der letzte ist das Komma des Bruchs, davor stehen Tausendergruppen mit dem anderen Zeichen
        let before = groups.dropLast()
        let others = seps.dropLast()
        if let firstOther = others.first, others.allSatisfy({ $0 == firstOther }), firstOther != last, thousands(before) {
            out.append(comparable(Array(before.flatMap { $0 }), groups[groups.count - 1]))
        }
        return out
    }
}

// MARK: - Fristauflösung

public enum AssistantOutcomeDates {
    private static let months: [(String, Int)] = [
        ("januar", 1), ("january", 1), ("februar", 2), ("february", 2), ("märz", 3), ("maerz", 3), ("march", 3),
        ("april", 4), ("mai", 5), ("may", 5), ("juni", 6), ("june", 6), ("juli", 7), ("july", 7), ("august", 8),
        ("september", 9), ("oktober", 10), ("october", 10), ("november", 11), ("dezember", 12), ("december", 12),
    ]
    private static let weekdays: [(String, Int)] = [
        ("montag", 1), ("monday", 1), ("dienstag", 2), ("tuesday", 2), ("mittwoch", 3), ("wednesday", 3),
        ("donnerstag", 4), ("thursday", 4), ("freitag", 5), ("friday", 5), ("samstag", 6), ("sonnabend", 6),
        ("saturday", 6), ("sonntag", 7), ("sunday", 7),
    ]
    private static let prefixes = ["am", "bis", "zum", "zur", "spätestens", "spaetestens", "by", "until", "on"]
    private static let next = ["nächsten", "naechsten", "nächster", "naechster", "kommenden", "next"]
    private static let afterNext = ["übernächsten", "uebernaechsten", "übernächster", "uebernaechster"]
    private static let dayUnits = ["tag", "tage", "tagen", "day", "days"]
    private static let weekUnits = ["woche", "wochen", "week", "weeks"]

    private enum Fixed { case plus(Int), endOfWeek, endOfMonth, nextWeek }
    private static let phrases: [(String, Fixed)] = [
        ("heute", .plus(0)), ("today", .plus(0)),
        ("morgen", .plus(1)), ("tomorrow", .plus(1)),
        ("übermorgen", .plus(2)), ("uebermorgen", .plus(2)), ("day after tomorrow", .plus(2)),
        ("ende der woche", .endOfWeek), ("ende dieser woche", .endOfWeek), ("end of the week", .endOfWeek),
        ("end of this week", .endOfWeek),
        ("ende des monats", .endOfMonth), ("ende dieses monats", .endOfMonth), ("end of the month", .endOfMonth),
        ("end of this month", .endOfMonth),
        ("nächste woche", .nextWeek), ("naechste woche", .nextWeek), ("next week", .nextWeek),
    ]

    private typealias T = AssistantOutcomeText

    private static func lookup(_ table: [(String, Int)], _ key: String) -> Int? { table.first { T.same($0.0, key) }?.1 }
    private static func isIn(_ list: [String], _ key: String) -> Bool { list.contains { T.same($0, key) } }

    /// Datum ab `today` plus `days`; `nil` außerhalb der Jahre 1000–9999.
    private static func plusDays(_ today: AssistantOutcomeDate, _ days: Int) -> String? {
        let d = AssistantOutcomeDate(dayNumber: today.dayNumber + days)
        return d.isValid ? d.formatted : nil
    }

    private static func withYear(_ year: Int, _ month: Int, _ day: Int) -> String? {
        let d = AssistantOutcomeDate(year: year, month: month, day: day)
        return d.isValid ? d.formatted : nil
    }

    /// Nächstes Vorkommen von Monat/Tag ab `today` (heute eingeschlossen); 29.2. sucht das nächste Schaltjahr.
    private static func nextOccurrence(_ today: AssistantOutcomeDate, _ month: Int, _ day: Int) -> String? {
        let from = today.dayNumber
        for year in today.year...(today.year + 8) {
            let d = AssistantOutcomeDate(year: year, month: month, day: day)
            if d.isValid, d.dayNumber >= from { return d.formatted }
        }
        return nil
    }

    /// `14`, `14.`, `14th`, `14,` (Tag eines Datums mit Monatsnamen); sonst `nil`.
    private static func dayToken(_ token: String) -> Int? {
        var b = Array(token.utf8)
        if let last = b.last, last == 0x2C || last == 0x2E { b.removeLast() }
        for suffix in ["st", "nd", "rd", "th"] where b.count >= 2 && Array(b.suffix(2)) == Array(suffix.utf8) {
            b.removeLast(2)
            break
        }
        guard T.onlyDigits(b), b.count <= 2 else { return nil }
        return T.int(b)
    }

    private static func yearToken(_ token: String) -> Int? {
        let b = Array(token.utf8)
        return T.onlyDigits(b) && b.count == 4 ? T.int(b) : nil
    }

    private static func dateWithMonthName(_ tokens: [String], _ today: AssistantOutcomeDate) -> String? {
        // `14. November [2026]`, `14 November`, `14th of November`
        if tokens.count >= 2, let day = dayToken(tokens[0]) {
            var i = 1
            if T.same(tokens[i], "of") { i += 1 }
            if i < tokens.count, let month = lookup(months, tokens[i]) {
                let rest = Array(tokens[(i + 1)...])
                if rest.isEmpty { return nextOccurrence(today, month, day) }
                guard rest.count == 1, let year = yearToken(rest[0]) else { return nil }
                return withYear(year, month, day)
            }
        }
        // `November 14[th][, 2026]`
        if let month = lookup(months, tokens[0]), tokens.count >= 2 {
            guard let day = dayToken(tokens[1]) else { return nil }
            let rest = Array(tokens[2...])
            if rest.isEmpty { return nextOccurrence(today, month, day) }
            guard rest.count == 1, let year = yearToken(rest[0]) else { return nil }
            return withYear(year, month, day)
        }
        return nil
    }

    /// `.some(nil)` heißt: sieht wie eine Zahlenschreibweise aus, ist aber ungültig; `nil`: keine Zahlenschreibweise.
    private static func numericDate(_ token: String, _ today: AssistantOutcomeDate) -> String?? {
        let b = Array(token.utf8)
        if b.count == 10, b[4] == 0x2D {
            if let d = AssistantOutcomeDate.parseISO(token) { return .some(d.formatted) }
            return .some(nil)
        }
        let parts = b.split(separator: 0x2E, omittingEmptySubsequences: false).map { Array($0) }
        guard parts.count >= 2, parts.count <= 3, parts.allSatisfy({ T.onlyDigits($0) }) else { return nil }
        guard parts[0].count <= 2, parts[1].count <= 2 else { return nil }
        let day = T.int(parts[0]), month = T.int(parts[1])
        if parts.count == 2 { return .some(nextOccurrence(today, month, day)) }
        guard parts[2].count == 4 else { return .some(nil) }
        return .some(withYear(T.int(parts[2]), month, day))
    }

    /// Löst eine Fristangabe zu `JJJJ-MM-TT` auf; alles außerhalb der festen Liste ist `nil`. Der Ausdruck wird normalisiert,
    /// führende Vorsätze (`am`, `bis`, `zum`, `zur`, `spätestens`, `by`, `until`, `on`) entfallen. Wochentage meinen den nächsten
    /// solchen Tag nach `today` (`übernächsten`: eine Woche später), `Ende der Woche` den Freitag der laufenden Woche (am
    /// Wochenende den der folgenden), `nächste Woche` den Montag der Folgewoche, Datumsangaben ohne Jahr das nächste Vorkommen
    /// ab `today`.
    public static func deadline(_ expression: String, today: AssistantOutcomeDate) -> String? {
        guard today.isValid else { return nil }
        let all = T.tokens(T.normalizedScalars(expression))
        var skip = 0
        while all.count - skip > 1, isIn(prefixes, all[skip]) { skip += 1 }
        let tokens = Array(all[skip...])
        if tokens.isEmpty || tokens.count > 6 { return nil }
        let text = tokens.joined(separator: " ")
        let todayWeekday = today.weekday

        if let fixed = phrases.first(where: { T.same($0.0, text) })?.1 {
            switch fixed {
            case .plus(let n): return plusDays(today, n)
            case .endOfWeek: return plusDays(today, todayWeekday <= 5 ? 5 - todayWeekday : 12 - todayWeekday)
            case .endOfMonth: return withYear(today.year, today.month, AssistantOutcomeDate.daysIn(year: today.year, month: today.month))
            case .nextWeek: return plusDays(today, 8 - todayWeekday)
            }
        }

        func untilWeekday(_ target: Int) -> Int { ((target - todayWeekday + 6) % 7) + 1 }
        if tokens.count == 1, let w = lookup(weekdays, tokens[0]) { return plusDays(today, untilWeekday(w)) }
        if tokens.count == 2, let w = lookup(weekdays, tokens[1]) {
            if isIn(next, tokens[0]) { return plusDays(today, untilWeekday(w)) }
            if isIn(afterNext, tokens[0]) { return plusDays(today, untilWeekday(w) + 7) }
        }

        if tokens.count == 3, T.same(tokens[0], "in") {
            let digits = Array(tokens[1].utf8)
            let number: Int? = T.onlyDigits(digits) && digits.count <= 3 ? T.int(digits) : T.numberWord(tokens[1])
            guard let n = number else { return nil }
            if isIn(dayUnits, tokens[2]) { return plusDays(today, n) }
            if isIn(weekUnits, tokens[2]) { return plusDays(today, n * 7) }
            return nil
        }

        if tokens.count == 1 {
            if let numeric = numericDate(tokens[0], today) { return numeric }
            return nil
        }
        return dateWithMonthName(tokens, today)
    }

    /// `deadlines(in:)` untersucht Ausdrücke, die in den ersten so vielen Wörtern eines Textes beginnen.
    public static let maxDeadlineWords = 1024

    /// Größtes Fenster (in Wörtern), das die Fristauflösung betrachtet.
    public static let maxDeadlineWindow = 6

    /// Höchstens so viele Fundstellen eines Belegs in einem Beitrag werden für die Deckung geprüft; mehr: nicht gedeckt.
    public static let maxOccurrences = 16

    /// Nachbarschaft einer Fundstelle: so viele Wörter davor und danach gehören zum Kontext.
    public static let contextWords = 8

    /// Dafür werden je Seite höchstens so viele Codepunkte gelesen; reicht das nicht und der Beitrag geht weiter: nicht gedeckt.
    public static let contextScalars = 400

    /// Wörter, die einen Tag verschieben oder näher bestimmen: `nächsten`/`kommenden`/`next` und `übernächsten` in allen Formen,
    /// die Einheiten `Tag`, `Woche`, `Monat`, `Jahr` (deutsch und englisch, alle Formen der Liste).
    private static let modifiers: [String] = next + afterNext + [
        "nächste", "naechste", "nächstes", "naechstes", "nächstem", "naechstem", "kommende", "kommender", "kommendes", "kommendem",
        "übernächste", "uebernaechste", "übernächstes", "uebernaechstes", "übernächstem", "uebernaechstem",
        "monat", "monate", "monaten", "monats", "month", "months", "jahr", "jahre", "jahren", "jahres", "year", "years",
    ] + dayUnits + weekUnits

    /// Der feste Wortschatz der Fristauflösung (Datumsbestandteile) ohne Zahlwörter: Wochentage, Monate, Tagesworte, Modifikatoren
    /// und Einheiten. Führende Vorsätze (`am`, `bis`, …) gehören nicht dazu. Zahlwörter (null bis zwölf, `ein`-Formen, `one`)
    /// zählen nur unter der Bedingung von `isDateComponent`.
    private static let dateWords: [String] = weekdays.map(\.0) + months.map(\.0) + modifiers + [
        "heute", "today", "morgen", "tomorrow", "übermorgen", "uebermorgen", "ende", "end", "dieser", "dieses", "this",
    ]

    /// Wörter hinter einer Zahl, die sie zur Uhrzeit machen (`10 Uhr`, `10 am`); `o'clock` steht normalisiert als `oclock` da.
    private static let timeAfter = ["uhr", "h", "oclock", "am", "pm"]

    /// Ein Wort ohne die Satzzeichen an seinen Rändern (dieselbe Menge wie bei der Normalisierung).
    private static func withoutEdge(_ token: String) -> String {
        let c = T.scalars(token)
        var from = 0
        var to = c.count
        while from < to, T.isEdge(c[from]) { from += 1 }
        while to > from, T.isEdge(c[to - 1]) { to -= 1 }
        return T.string(c[from..<to])
    }

    /// Reines Zifferntoken: Ziffernfolgen, durch je ein `.`, `,`, `-` oder `/` verbunden (`14`, `14.11.`, `1.250,50`, `2026-11-14`).
    private static func isDigitToken(_ token: String) -> Bool {
        let c = T.scalars(token)
        func isDigit(_ x: UInt32) -> Bool { x >= 0x30 && x <= 0x39 }
        var i = 0
        while true {
            let start = i
            while i < c.count, isDigit(c[i]) { i += 1 }
            if i == start { return false }
            if i == c.count { return true }
            guard c[i] == 0x2E || c[i] == 0x2C || c[i] == 0x2D || c[i] == 0x2F else { return false }
            i += 1
        }
    }

    /// Verbinder und Verneiner: Steht einer direkt hinter einem Fristausdruck (oder direkt davor, führende Vorsätze übersprungen),
    /// ist der Tag womöglich nicht gemeint (`Montag oder Dienstag`, `nicht am Montag`).
    private static let connectors = [
        "oder", "bzw", "beziehungsweise", "entweder", "weder", "noch", "nicht", "kein", "keine", "keinen", "keinem", "keiner",
        "statt", "anstatt", "außer", "ausser", "or", "either", "neither", "nor", "not", "no", "instead", "except",
    ]

    /// Ist das Wort `i` ein Datumsbestandteil des Ausdrucks, an dessen Rand es steht (`after`: rechts davon, sonst links davon)?
    /// - Wörter des festen Wortschatzes (Wochentag, Monat, Modifikator, Einheit, Tagesworte): immer.
    /// - Reine Zifferntoken (`14`, `14.11.`): immer.
    /// - Zahlwörter (`zwei`, `eine`, `one`): nur, wenn das nächste Wort in derselben Richtung (vom Ausdruck aus das übernächste)
    ///   zum Wortschatz gehört oder ein Zifferntoken ist (`Montag zwei Wochen`); `Montag zwei Personen` bleibt sicher.
    /// - `in` vor einer Ziffernfolge oder vor einem Zahlwort, das selbst zählt (`in zwei Wochen`).
    /// Ausnahme Uhrzeit: Ein Zahlwort oder Zifferntoken **hinter** dem Ausdruck zählt nicht, wenn `uhr`, `h`, `oclock`, `am` oder
    /// `pm` folgt (`Montag 10 Uhr`); `hh:mm` ist kein Zifferntoken.
    private static func isDateComponent(_ tokens: [String], _ i: Int, after: Bool) -> Bool {
        let t = withoutEdge(tokens[i])
        if t.isEmpty { return false }
        func word(_ k: Int) -> String? { k >= 0 && k < tokens.count ? withoutEdge(tokens[k]) : nil }
        func countsAsNumber(_ k: Int, _ step: Int) -> Bool {
            guard let w = word(k), !w.isEmpty else { return false }
            if isDigitToken(w) { return true }
            guard T.numberWord(w) != nil, let n = word(k + step) else { return false }
            return isDigitToken(n) || isIn(dateWords, n)
        }
        let isNumber = T.numberWord(t) != nil || isDigitToken(t)
        if isNumber, after, let n = word(i + 1), isIn(timeAfter, n) { return false }
        if isDigitToken(t) || isIn(dateWords, t) { return true }
        if T.numberWord(t) != nil { return countsAsNumber(i, after ? 1 : -1) }
        guard T.same(t, "in"), let n = word(i + 1) else { return false }
        return T.onlyDigits(Array(n.utf8)) || countsAsNumber(i + 1, 1)
    }

    /// Steht direkt hinter dem Ausdruck (Wörter `start` bis `end` einschließlich) oder direkt davor (führende Vorsätze
    /// übersprungen) ein Verbinder oder Verneiner?
    private static func hasConnector(_ tokens: [String], start: Int, end: Int) -> Bool {
        if end + 1 < tokens.count, isIn(connectors, withoutEdge(tokens[end + 1])) { return true }
        var before = start - 1
        while before >= 0, isIn(prefixes, withoutEdge(tokens[before])) { before -= 1 }
        return before >= 0 && isIn(connectors, withoutEdge(tokens[before]))
    }

    /// Brückenwörter: Wörter zwischen einem Fristausdruck und einem weiteren Datumsbestandteil (`Montag bis Freitag`, `am Montag im
    /// Mai`, `Montag den 14. Dezember`, `Montag und Dienstag`). Eine eigene Menge; sie schließt die führenden Vorsätze ein.
    private static let bridges = [
        "am", "an", "bis", "zum", "zur", "spätestens", "spaetestens", "im", "in", "den", "dem", "der", "des", "vom", "von", "ab", "um", "gegen", "und", "sowie", "by", "until", "till", "on", "to", "through", "thru", "and", "of", "the", "at", "from",
    ]

    /// Höchstens so viele Brückenwörter werden zwischen Ausdruck und Datumsbestandteil übersprungen.
    public static let maxBridges = 2

    /// Steht hinter (`after`) oder vor dem Ausdruck, nach 1 bis `maxBridges` übersprungenen Brückenwörtern, ein Datumsbestandteil?
    /// `i` ist das erste Wort neben dem Ausdruck in dieser Richtung. Das direkte Nachbarwort (kein Brückenwort übersprungen)
    /// prüft `isDateComponent` selbst. Die Uhrzeit-Ausnahme gilt auch hinter Brückenwörtern (`Montag um 10 Uhr`).
    private static func bridge(_ tokens: [String], _ i: Int, after: Bool) -> Bool {
        let step = after ? 1 : -1
        var k = i
        var skipped = 0
        while skipped < maxBridges, k >= 0, k < tokens.count, isIn(bridges, withoutEdge(tokens[k])) {
            k += step
            skipped += 1
        }
        if skipped == 0 || k < 0 || k >= tokens.count { return false }
        return isDateComponent(tokens, k, after: after)
    }

    /// Ein inklusionsmaximaler Fristausdruck: Wörter `start` bis `start + length`, der Tag und ob ihn ein Datumsbestandteil
    /// begleitet.
    struct DeadlineMatch {
        var start: Int
        var length: Int
        var day: String
        var unsafe: Bool
    }

    /// Die inklusionsmaximalen Fristausdrücke unter den Wörtern `tokens`, die die Wörter `p` bis `q` (einschließlich) schneiden:
    /// Unter allen Fenstern von höchstens `maxDeadlineWindow` Wörtern, die die Fristauflösung auflöst, bleiben die übrig, die kein
    /// längeres auflösendes Fenster enthält (egal, ob es `p...q` verlässt). Reihenfolge: nach Anfang, bei gleichem Anfang kürzere
    /// zuerst. `unsafe` ist ein Treffer, wenn das Wort direkt davor oder direkt danach ein Datumsbestandteil ist (`isDateComponent`)
    /// oder direkt daneben ein Verbinder oder Verneiner steht (`hasConnector`) oder nach höchstens
    /// `maxBridges` Brückenwörtern ein Datumsbestandteil folgt (`bridge`). Die Treffer
    /// können überlappen und auf verschiedene Tage auflösen; die Zerlegung hängt nicht von der Suchrichtung ab.
    static func maximalDeadlines(_ tokens: [String], from p: Int, to q: Int, today: AssistantOutcomeDate) -> [DeadlineMatch] {
        let width = maxDeadlineWindow + 1
        var resolved = Set<Int>()
        var order: [(start: Int, length: Int, day: String)] = []
        var i = max(0, p - (maxDeadlineWindow - 1))
        while i <= q, i < tokens.count {
            let most = min(maxDeadlineWindow, tokens.count - i)
            var n = 1
            while n <= most {
                if i + n - 1 >= p, let day = deadline(tokens[i..<(i + n)].joined(separator: " "), today: today) {
                    resolved.insert(i * width + n)
                    order.append((i, n, day))
                }
                n += 1
            }
            i += 1
        }
        func contained(_ i: Int, _ n: Int) -> Bool {
            var m = n + 1
            while m <= maxDeadlineWindow {
                var j = max(0, i + n - m)
                while j <= i {
                    if resolved.contains(j * width + m) { return true }
                    j += 1
                }
                m += 1
            }
            return false
        }
        var out: [DeadlineMatch] = []
        for item in order where !contained(item.start, item.length) {
            let before = item.start - 1
            let after = item.start + item.length
            let unsafe = (before >= 0 && isDateComponent(tokens, before, after: false))
                || (after < tokens.count && isDateComponent(tokens, after, after: true))
                || hasConnector(tokens, start: item.start, end: after - 1)
                || bridge(tokens, before, after: false)
                || bridge(tokens, after, after: true)
            out.append(DeadlineMatch(start: item.start, length: item.length, day: item.day, unsafe: unsafe))
        }
        return out
    }

    /// Alle Tage, die die inklusionsmaximalen, sicheren Fristausdrücke eines Textes auflösen; Reihenfolge des Auftretens, ohne
    /// Wiederholung. Untersucht werden Ausdrücke, die in den ersten `maxDeadlineWords` Wörtern beginnen; ob ein Treffer an dieser
    /// Grenze sicher ist, entscheidet das Wort dahinter (gelesen wird über die Grenze hinaus).
    public static func deadlines(in text: String, today: AssistantOutcomeDate) -> [String] {
        let tokens = T.tokens(T.normalizedScalars(text), limit: maxDeadlineWords + maxDeadlineWindow + maxBridges)
        if tokens.isEmpty { return [] }
        var out: [String] = []
        let q = min(maxDeadlineWords, tokens.count) - 1
        for match in maximalDeadlines(tokens, from: 0, to: q, today: today) where !match.unsafe {
            if !out.contains(where: { T.same($0, match.day) }) { out.append(match.day) }
        }
        return out
    }

    /// Ohne führende Vorsätze (`am`, `bis`, `zum`, …; solange mehr als ein Wort bleibt).
    private static func withoutPrefixes(_ tokens: ArraySlice<String>) -> ArraySlice<String> {
        var rest = tokens
        while rest.count > 1, let first = rest.first, isIn(prefixes, first) { rest = rest.dropFirst() }
        return rest
    }

    /// Die Wörter der Nachbarschaft einer Fundstelle und die Nummern der Wörter, die sie berühren.
    struct Neighbourhood {
        var tokens: [String]
        var p: Int
        var q: Int
    }

    /// Die Nachbarschaft der Spanne `start..<end` im normalisierten `turn`: die Wörter, die die Spanne berühren, plus
    /// `contextWords` ganze Wörter davor und danach. Je Seite werden höchstens `contextScalars` Codepunkte gelesen; findet sich
    /// darin nicht die Grenze zum neunten Wort (und der Beitrag ist dort nicht zu Ende), gibt es keine Nachbarschaft (`nil`).
    static func neighbourhood(_ turn: [UInt32], start: Int, end: Int) -> Neighbourhood? {
        guard let range = T.neighbourhood(turn, start: start, end: end, words: contextWords, limit: contextScalars) else { return nil }
        var tokens: [String] = []
        var p = -1
        var q = -1
        var cursor = range.lowerBound
        while true {
            var stop = cursor
            while stop < range.upperBound, turn[stop] != 0x20 { stop += 1 }
            if stop > start, cursor < end {
                if p < 0 { p = tokens.count }
                q = tokens.count
            }
            tokens.append(T.string(turn[cursor..<stop]))
            if stop >= range.upperBound { break }
            cursor = stop + 1
        }
        return p < 0 ? nil : Neighbourhood(tokens: tokens, p: p, q: q)
    }

    /// Der eine Tag, den die Spanne `start..<end` des normalisierten `turn` im Kontext nennt, sonst `nil`: Unter den
    /// inklusionsmaximalen Fristausdrücken der Nachbarschaft, die die Spanne schneiden, ist mindestens einer, keiner ist unsicher, und
    /// alle lösen auf denselben Tag auf.
    static func contextDeadline(_ turn: [UInt32], start: Int, end: Int, today: AssistantOutcomeDate) -> String? {
        guard let n = neighbourhood(turn, start: start, end: end) else { return nil }
        let matches = maximalDeadlines(n.tokens, from: n.p, to: n.q, today: today)
        guard let first = matches.first, !matches.contains(where: { $0.unsafe }) else { return nil }
        return matches.allSatisfy { T.same($0.day, first.day) } ? first.day : nil
    }

    /// Nennt der Beleg `quote` (normalisiert) im normalisierten `turn` den Tag `day`? An allen Fundstellen (mindestens eine, höchstens
    /// `maxOccurrences`) muss `contextDeadline` genau diesen Tag liefern.
    static func deadlineCovered(_ turn: [UInt32], quote: [UInt32], day: String, today: AssistantOutcomeDate) -> Bool {
        let spots = T.occurrences(of: quote, in: turn, limit: maxOccurrences + 1)
        if spots.isEmpty || spots.count > maxOccurrences { return false }
        return spots.allSatisfy { start in
            contextDeadline(turn, start: start, end: start + quote.count, today: today).map { T.same($0, day) } ?? false
        }
    }

    /// Nennt der Beleg `quote` (normalisiert) im normalisierten `turn` die Zahl mit der Vergleichsform `comparable`? An allen
    /// Fundstellen (mindestens eine, höchstens `maxOccurrences`) muss `T.numberCovered` gelten.
    static func numberCovered(_ turn: [UInt32], quote: [UInt32], comparable: String) -> Bool {
        let spots = T.occurrences(of: quote, in: turn, limit: maxOccurrences + 1)
        if spots.isEmpty || spots.count > maxOccurrences { return false }
        return spots.allSatisfy { T.numberCovered(turn, start: $0, end: $0 + quote.count, comparable: comparable) }
    }

    /// Steht `expression` (Wörter eines `frist_text`, bis auf führende Vorsätze) an der Fundstelle `start..<end` des normalisierten
    /// `turn` als ganzer, sicherer, inklusionsmaximaler Fristausdruck?
    static func isDeadlineOccurrence(_ turn: [UInt32], start: Int, end: Int, expression: [String], today: AssistantOutcomeDate) -> Bool {
        guard let n = neighbourhood(turn, start: start, end: end) else { return false }
        let core = withoutPrefixes(expression[...])
        return maximalDeadlines(n.tokens, from: n.p, to: n.q, today: today).contains { match in
            if match.unsafe || match.start > n.p || match.start + match.length - 1 < n.q { return false }
            let own = withoutPrefixes(n.tokens[match.start..<(match.start + match.length)])
            return own.count == core.count && zip(own, core).allSatisfy { T.same($0, $1) }
        }
    }

    /// Steht `expression` in `turn` an einer der ersten `maxOccurrences` Fundstellen von `text` (normalisiert) als ganzer, sicherer,
    /// inklusionsmaximaler Fristausdruck?
    static func containsDeadlineExpression(_ turn: [UInt32], text: [UInt32], expression: [String], today: AssistantOutcomeDate) -> Bool {
        T.occurrences(of: text, in: turn, limit: maxOccurrences).contains {
            isDeadlineOccurrence(turn, start: $0, end: $0 + text.count, expression: expression, today: today)
        }
    }
}
