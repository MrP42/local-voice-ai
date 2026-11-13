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

    /// Steht `needle` als ganze Wortfolge in `haystack`? Wie ein zusammenhängender Teil, aber vor einem Treffer, der mit einem
    /// Wortzeichen beginnt, und nach einem, der mit einem endet, steht kein Wortzeichen (`ja` steht nicht in `jahr`).
    static func containsWordSequence(_ haystack: [UInt32], _ needle: [UInt32]) -> Bool {
        guard let first = needle.first, let last = needle.last, needle.count <= haystack.count else { return false }
        let leadWord = isWordScalar(first)
        let trailWord = isWordScalar(last)
        var fail = [Int](repeating: 0, count: needle.count)
        var k = 0
        for i in 1..<max(needle.count, 1) {
            while k > 0, needle[i] != needle[k] { k = fail[k - 1] }
            if needle[i] == needle[k] { k += 1 }
            fail[i] = k
        }
        k = 0
        for (index, c) in haystack.enumerated() {
            while k > 0, c != needle[k] { k = fail[k - 1] }
            if c == needle[k] { k += 1 }
            guard k == needle.count else { continue }
            let start = index + 1 - needle.count
            let beforeOK = !leadWord || start == 0 || !isWordScalar(haystack[start - 1])
            let afterOK = !trailWord || index + 1 >= haystack.count || !isWordScalar(haystack[index + 1])
            if beforeOK && afterOK { return true }
            k = fail[k - 1]
        }
        return false
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

    /// Hat der normalisierte Text ein Zahlwort (null bis zwölf), das für `number` steht?
    static func hasNumberWord(_ normalized: [UInt32], _ number: Int) -> Bool {
        words(normalized).contains { numberWord($0) == number }
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

    /// Vergleichsformen der Zahlen eines normalisierten Textes (Ziffernfolgen, durch ein einzelnes `.` oder `,` zwischen Ziffern
    /// verbunden).
    static func numbers(in normalized: [UInt32]) -> [String] {
        let c = normalized
        var out: [String] = []
        var i = 0
        func isDigit(_ x: UInt32) -> Bool { x >= 0x30 && x <= 0x39 }
        while i < c.count {
            if !isDigit(c[i]) {
                i += 1
                continue
            }
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
            for form in candidates(groups, seps) where !out.contains(where: { same($0, form) }) { out.append(form) }
        }
        return out
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

    /// Die Fristsuche untersucht höchstens die ersten so vielen Wörter eines Textes.
    public static let maxDeadlineWords = 1024

    /// Größtes Fenster (in Wörtern), das die Fristauflösung betrachtet.
    public static let maxDeadlineWindow = 6

    /// Wörter, die einen Tag verschieben oder näher bestimmen, ohne dass die feste Liste der Fristauflösung sie mit dem Tag
    /// zusammen auflöst: `nächsten`/`kommenden`/`next` und `übernächsten` in allen Formen, Einheiten (`Tag`, `Woche`, `Monat`,
    /// `Jahr` und englisch). Dazu zählt `in`, wenn eine Zahl oder ein Zahlwort folgt.
    private static let modifiers: [String] = next + afterNext + [
        "nächste", "naechste", "nächstes", "naechstes", "nächstem", "naechstem", "kommende", "kommender", "kommendes", "kommendem",
        "übernächste", "uebernaechste", "übernächstes", "uebernaechstes", "übernächstem", "uebernaechstem",
        "monat", "monate", "monaten", "monats", "month", "months", "jahr", "jahre", "jahren", "jahres", "year", "years",
    ] + dayUnits + weekUnits

    /// Ein maximaler Fristausdruck: Wörter `start` bis `start + length`, der Tag und ob ihn ein nicht aufgelöstes Modifikatorwort
    /// begleitet.
    struct DeadlineMatch {
        var start: Int
        var length: Int
        var day: String
        var unsafe: Bool
    }

    private static func isModifier(_ tokens: [String], _ i: Int) -> Bool {
        if isIn(modifiers, tokens[i]) { return true }
        guard T.same(tokens[i], "in"), i + 1 < tokens.count else { return false }
        return T.onlyDigits(Array(tokens[i + 1].utf8)) || T.numberWord(tokens[i + 1]) != nil
    }

    /// Wörter eines Textes für die Fristsuche (normalisiert, an Leerzeichen getrennt, höchstens `maxDeadlineWords`).
    static func words(of text: String) -> [String] {
        T.tokens(T.normalizedScalars(text), limit: maxDeadlineWords)
    }

    /// Die maximalen Fristausdrücke der Wörter eines normalisierten Textes: von links nach rechts, an jeder Wortposition das
    /// längste Fenster (bis `maxDeadlineWindow` Wörter), das die Fristauflösung auflöst; die Wörter des Treffers sind verbraucht,
    /// Treffer überlappen nie. `unsafe` ist ein Treffer, wenn unmittelbar davor oder danach ein Modifikatorwort steht, das zu
    /// keinem Treffer gehört (`Montag übernächste Woche`: der Tag allein wäre falsch).
    static func maximalDeadlines(_ words: [String], today: AssistantOutcomeDate) -> [DeadlineMatch] {
        let tokens = Array(words.prefix(maxDeadlineWords))
        var matches: [DeadlineMatch] = []
        var used = [Bool](repeating: false, count: tokens.count)
        var i = 0
        while i < tokens.count {
            var found: (length: Int, day: String)?
            var n = min(maxDeadlineWindow, tokens.count - i)
            while n >= 1 {
                if let day = deadline(tokens[i..<(i + n)].joined(separator: " "), today: today) {
                    found = (n, day)
                    break
                }
                n -= 1
            }
            guard let found else {
                i += 1
                continue
            }
            matches.append(DeadlineMatch(start: i, length: found.length, day: found.day, unsafe: false))
            for j in i..<(i + found.length) { used[j] = true }
            i += found.length
        }
        for index in matches.indices {
            let before = matches[index].start - 1
            let after = matches[index].start + matches[index].length
            matches[index].unsafe = (before >= 0 && !used[before] && isModifier(tokens, before))
                || (after < tokens.count && !used[after] && isModifier(tokens, after))
        }
        return matches
    }

    /// Alle Tage, die die maximalen, sicheren Fristausdrücke eines Textes auflösen (siehe `maximalDeadlines`); Reihenfolge des
    /// Auftretens, ohne Wiederholung. Untersucht werden höchstens die ersten `maxDeadlineWords` Wörter.
    public static func deadlines(in text: String, today: AssistantOutcomeDate) -> [String] {
        var out: [String] = []
        for match in maximalDeadlines(words(of: text), today: today) where !match.unsafe {
            if !out.contains(where: { T.same($0, match.day) }) { out.append(match.day) }
        }
        return out
    }

    /// Der eine Tag, den ein Beleg eindeutig nennt, sonst `nil`: Der Beleg hat mindestens einen maximalen Fristausdruck, keiner
    /// ist unsicher, und alle lösen auf denselben Tag auf.
    public static func coveredDeadline(in text: String, today: AssistantOutcomeDate) -> String? {
        let matches = maximalDeadlines(words(of: text), today: today)
        guard let first = matches.first, !matches.contains(where: { $0.unsafe }) else { return nil }
        return matches.allSatisfy { T.same($0.day, first.day) } ? first.day : nil
    }

    /// Ohne führende Vorsätze (`am`, `bis`, `zum`, …; solange mehr als ein Wort bleibt).
    private static func withoutPrefixes(_ tokens: ArraySlice<String>) -> ArraySlice<String> {
        var rest = tokens
        while rest.count > 1, let first = rest.first, isIn(prefixes, first) { rest = rest.dropFirst() }
        return rest
    }

    /// Steht `expression` (normalisiert, bis auf führende Vorsätze) als ganzer, sicherer, maximaler Fristausdruck in den
    /// Treffern eines Textes, dessen Wörter `words` sind?
    static func isMaximalExpression(_ expression: [String], in words: [String], matches: [DeadlineMatch]) -> Bool {
        let core = withoutPrefixes(expression[...])
        return matches.contains { match in
            if match.unsafe { return false }
            let own = withoutPrefixes(words[match.start..<(match.start + match.length)])
            return own.count == core.count && zip(own, core).allSatisfy { T.same($0, $1) }
        }
    }
}
