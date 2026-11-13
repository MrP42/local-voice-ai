import Foundation

// Assistenten-Profil (lva#555, Paket G1): ein versioniertes JSON-Dokument, gleiches Schema und gleiches Verhalten auf iPhone
// (dieser Kern) und PC (`apps/local-voice/src/components/voice-call/assistenten/`). Die Regeln stehen in
// `apps/shared/assistenten/README.md`, die gemeinsamen Prüfstände in `pruefstaende.json` (beide Kerne bestehen alle Fälle).
//
// Bauweise: Ein eigener JSON-Baum (`AssistantJSON`) mit eigenem Leser und Schreiber, damit die Ausgabe auf beiden Plattformen
// bytegleich ist (sortierte Schlüssel, Zahlen als Originaltext) und unbekannte Felder unverändert erhalten bleiben. Das typisierte
// Modell hält je Ebene nur die unbekannten Felder (`extra`) und legt beim Schreiben die typisierten Felder darüber. Alle Muster
// sind von Hand geprüft (kein Regex), Längen zählen in Unicode-Codepunkten (`unicodeScalars`).

// MARK: - JSON-Baum

/// Ein Mitglied eines JSON-Objekts. Die Reihenfolge im Baum zählt nicht; geschrieben wird nach Schlüsseln sortiert.
public struct AssistantJSONMember: Equatable, Sendable {
    public var key: String
    public var value: AssistantJSON

    public init(_ key: String, _ value: AssistantJSON) {
        self.key = key
        self.value = value
    }
}

/// JSON-Wert mit Zahlen als Originaltext (so gehen fremde Zahlen unverändert durch).
public enum AssistantJSON: Equatable, Sendable {
    case null
    case bool(Bool)
    case number(String)
    case string(String)
    case array([AssistantJSON])
    case object([AssistantJSONMember])

    /// Ganzzahl als Wert (für typisierte Felder).
    public static func integer(_ value: Int) -> AssistantJSON { .number(String(value)) }

    /// Liest strenges JSON (RFC 8259, ein Wert, nichts danach). Fehler: `nil`. Doppelte Schlüssel: der letzte gilt.
    /// Tiefer als 64 Ebenen und einzelne Surrogate in `\u`-Folgen gelten als Fehler.
    public static func parse(_ text: String) -> AssistantJSON? {
        var parser = AssistantJSONParser(bytes: Array(text.utf8))
        return parser.document()
    }

    /// Kanonische Ausgabe: Schlüssel nach UTF-8-Bytes sortiert, 2 Leerzeichen Einzug, `\n` am Ende.
    public func encoded() -> String {
        var out = ""
        write(into: &out, level: 0)
        out.append("\n")
        return out
    }

    func member(_ key: String) -> AssistantJSON? {
        guard case .object(let members) = self else { return nil }
        return members.value(key)
    }

    private func write(into out: inout String, level: Int) {
        switch self {
        case .null: out.append("null")
        case .bool(let b): out.append(b ? "true" : "false")
        case .number(let raw): out.append(raw)
        case .string(let s): AssistantJSON.writeString(s, into: &out)
        case .array(let items):
            if items.isEmpty { out.append("[]"); return }
            out.append("[\n")
            for (index, item) in items.enumerated() {
                out.append(AssistantJSON.indent(level + 1))
                item.write(into: &out, level: level + 1)
                out.append(index == items.count - 1 ? "\n" : ",\n")
            }
            out.append(AssistantJSON.indent(level))
            out.append("]")
        case .object(let members):
            if members.isEmpty { out.append("{}"); return }
            let sorted = members.sorted { $0.key.utf8.lexicographicallyPrecedes($1.key.utf8) }
            out.append("{\n")
            for (index, member) in sorted.enumerated() {
                out.append(AssistantJSON.indent(level + 1))
                AssistantJSON.writeString(member.key, into: &out)
                out.append(": ")
                member.value.write(into: &out, level: level + 1)
                out.append(index == sorted.count - 1 ? "\n" : ",\n")
            }
            out.append(AssistantJSON.indent(level))
            out.append("}")
        }
    }

    private static func indent(_ level: Int) -> String { String(repeating: "  ", count: level) }

    private static func writeString(_ s: String, into out: inout String) {
        out.append("\"")
        for scalar in s.unicodeScalars {
            switch scalar.value {
            case 0x22: out.append("\\\"")
            case 0x5C: out.append("\\\\")
            case 0x0A: out.append("\\n")
            case 0x0D: out.append("\\r")
            case 0x09: out.append("\\t")
            case 0x08: out.append("\\b")
            case 0x0C: out.append("\\f")
            case 0..<0x20:
                let hex = String(scalar.value, radix: 16)
                out.append("\\u" + String(repeating: "0", count: 4 - hex.count) + hex)
            default: out.unicodeScalars.append(scalar)
            }
        }
        out.append("\"")
    }
}

extension Array where Element == AssistantJSONMember {
    /// Wert zu einem Schlüssel (Vergleich auf UTF-8-Bytes, nicht auf kanonische Gleichheit).
    func value(_ key: String) -> AssistantJSON? {
        first { $0.key.utf8.elementsEqual(key.utf8) }?.value
    }

    mutating func set(_ key: String, _ value: AssistantJSON) {
        if let index = firstIndex(where: { $0.key.utf8.elementsEqual(key.utf8) }) {
            self[index].value = value
        } else {
            append(AssistantJSONMember(key, value))
        }
    }

    /// Die Mitglieder, deren Schlüssel nicht in `known` stehen.
    func unknown(excluding known: [String]) -> [AssistantJSONMember] {
        filter { member in !known.contains { $0.utf8.elementsEqual(member.key.utf8) } }
    }
}

struct AssistantJSONParser {
    let bytes: [UInt8]
    var pos = 0
    static let maxDepth = 64

    init(bytes: [UInt8]) { self.bytes = bytes }

    mutating func document() -> AssistantJSON? {
        skipSpace()
        guard let value = parseValue(depth: 0) else { return nil }
        skipSpace()
        return pos == bytes.count ? value : nil
    }

    private mutating func skipSpace() {
        while pos < bytes.count, bytes[pos] == 0x20 || bytes[pos] == 0x09 || bytes[pos] == 0x0A || bytes[pos] == 0x0D { pos += 1 }
    }

    private mutating func literal(_ word: String) -> Bool {
        let w = Array(word.utf8)
        guard pos + w.count <= bytes.count, Array(bytes[pos..<pos + w.count]) == w else { return false }
        pos += w.count
        return true
    }

    private mutating func parseValue(depth: Int) -> AssistantJSON? {
        guard depth <= AssistantJSONParser.maxDepth, pos < bytes.count else { return nil }
        switch bytes[pos] {
        case 0x7B: return parseObject(depth: depth)
        case 0x5B: return parseArray(depth: depth)
        case 0x22: return parseString().map { .string($0) }
        case 0x74: return literal("true") ? .bool(true) : nil
        case 0x66: return literal("false") ? .bool(false) : nil
        case 0x6E: return literal("null") ? .null : nil
        default: return parseNumber()
        }
    }

    private mutating func parseObject(depth: Int) -> AssistantJSON? {
        pos += 1
        var members: [AssistantJSONMember] = []
        skipSpace()
        if pos < bytes.count, bytes[pos] == 0x7D { pos += 1; return .object(members) }
        while true {
            skipSpace()
            guard pos < bytes.count, bytes[pos] == 0x22, let key = parseString() else { return nil }
            skipSpace()
            guard pos < bytes.count, bytes[pos] == 0x3A else { return nil }
            pos += 1
            skipSpace()
            guard let value = parseValue(depth: depth + 1) else { return nil }
            members.set(key, value)
            skipSpace()
            guard pos < bytes.count else { return nil }
            if bytes[pos] == 0x2C { pos += 1; continue }
            if bytes[pos] == 0x7D { pos += 1; return .object(members) }
            return nil
        }
    }

    private mutating func parseArray(depth: Int) -> AssistantJSON? {
        pos += 1
        var items: [AssistantJSON] = []
        skipSpace()
        if pos < bytes.count, bytes[pos] == 0x5D { pos += 1; return .array(items) }
        while true {
            skipSpace()
            guard let value = parseValue(depth: depth + 1) else { return nil }
            items.append(value)
            skipSpace()
            guard pos < bytes.count else { return nil }
            if bytes[pos] == 0x2C { pos += 1; continue }
            if bytes[pos] == 0x5D { pos += 1; return .array(items) }
            return nil
        }
    }

    private func hexValue(_ b: UInt8) -> UInt32? {
        switch b {
        case 0x30...0x39: return UInt32(b - 0x30)
        case 0x41...0x46: return UInt32(b - 0x41 + 10)
        case 0x61...0x66: return UInt32(b - 0x61 + 10)
        default: return nil
        }
    }

    private mutating func hex4() -> UInt32? {
        guard pos + 4 <= bytes.count else { return nil }
        var value: UInt32 = 0
        for k in 0..<4 {
            guard let digit = hexValue(bytes[pos + k]) else { return nil }
            value = value * 16 + digit
        }
        pos += 4
        return value
    }

    private mutating func parseString() -> String? {
        pos += 1
        var buffer: [UInt8] = []
        while pos < bytes.count {
            let b = bytes[pos]
            if b == 0x22 {
                pos += 1
                return String(decoding: buffer, as: UTF8.self)
            }
            if b < 0x20 { return nil }
            guard b == 0x5C else {
                buffer.append(b)
                pos += 1
                continue
            }
            pos += 1
            guard pos < bytes.count else { return nil }
            let e = bytes[pos]
            pos += 1
            switch e {
            case 0x22: buffer.append(0x22)
            case 0x5C: buffer.append(0x5C)
            case 0x2F: buffer.append(0x2F)
            case 0x62: buffer.append(0x08)
            case 0x66: buffer.append(0x0C)
            case 0x6E: buffer.append(0x0A)
            case 0x72: buffer.append(0x0D)
            case 0x74: buffer.append(0x09)
            case 0x75:
                guard var code = hex4() else { return nil }
                if (0xDC00...0xDFFF).contains(code) { return nil }
                if (0xD800...0xDBFF).contains(code) {
                    guard pos + 1 < bytes.count, bytes[pos] == 0x5C, bytes[pos + 1] == 0x75 else { return nil }
                    pos += 2
                    guard let low = hex4(), (0xDC00...0xDFFF).contains(low) else { return nil }
                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                }
                guard let scalar = Unicode.Scalar(code) else { return nil }
                buffer.append(contentsOf: Array(String(Character(scalar)).utf8))
            default: return nil
            }
        }
        return nil
    }

    private mutating func digits() -> Bool {
        let start = pos
        while pos < bytes.count, (0x30...0x39).contains(bytes[pos]) { pos += 1 }
        return pos > start
    }

    private mutating func parseNumber() -> AssistantJSON? {
        let start = pos
        if pos < bytes.count, bytes[pos] == 0x2D { pos += 1 }
        guard pos < bytes.count else { return nil }
        if bytes[pos] == 0x30 {
            pos += 1
        } else if (0x31...0x39).contains(bytes[pos]) {
            _ = digits()
        } else {
            return nil
        }
        if pos < bytes.count, bytes[pos] == 0x2E {
            pos += 1
            guard digits() else { return nil }
        }
        if pos < bytes.count, bytes[pos] == 0x65 || bytes[pos] == 0x45 {
            pos += 1
            if pos < bytes.count, bytes[pos] == 0x2B || bytes[pos] == 0x2D { pos += 1 }
            guard digits() else { return nil }
        }
        return .number(String(decoding: bytes[start..<pos], as: UTF8.self))
    }
}

// MARK: - Fehler

/// Ein Befund der Prüfung: Pfad (`name`, `wissen.kontexte[3].art`) und Code. Es wird nie stillschweigend repariert.
public struct AssistantProfileIssue: Equatable, Sendable {
    public enum Code: String, Sendable, CaseIterable {
        case missing = "fehlt"
        case type = "typ"
        case length = "laenge"
        case pattern = "muster"
        case value = "wert"
        case tooMany = "zu_viele"
        case duplicate = "doppelt"
        case schemaUnknown = "schema_unbekannt"
        case schemaNewer = "schema_neuer"
        case json = "json"
    }

    public let path: String
    public let code: Code

    public init(path: String, code: Code) {
        self.path = path
        self.code = code
    }
}

/// Die Fehlerliste einer abgewiesenen Eingabe, nach Pfad, dann Code sortiert (UTF-8-Reihenfolge), ohne Wiederholungen.
public struct AssistantProfileFailure: Error, Equatable, Sendable {
    public let issues: [AssistantProfileIssue]

    public init(_ issues: [AssistantProfileIssue]) {
        var seen: [String] = []
        var unique: [AssistantProfileIssue] = []
        for issue in issues {
            let key = issue.path + "\u{0}" + issue.code.rawValue
            if seen.contains(key) { continue }
            seen.append(key)
            unique.append(issue)
        }
        self.issues = unique.sorted { a, b in
            if !a.path.utf8.elementsEqual(b.path.utf8) { return a.path.utf8.lexicographicallyPrecedes(b.path.utf8) }
            return a.code.rawValue.utf8.lexicographicallyPrecedes(b.code.rawValue.utf8)
        }
    }
}

// MARK: - Modell

public struct AssistantProfile: Equatable, Sendable {
    public static let schema = "lva.assistent/1"
    /// Bekannte Bereichsschlüssel des Profils (auf einer Plattform ohne Wirkung, wenn sie dort nichts abbildet).
    public static let knownAreaKeys = [
        "buecher", "buchnotizen", "ki_dokumente", "besprechungen", "sprachnotizen",
        "ablage", "gespraeche", "kalender", "vorlesen", "notizbuch",
    ]

    public struct Behavior: Equatable, Sendable {
        public enum Style: String, Sendable { case natural = "natuerlich", literal = "woertlich" }
        public enum Pause: String, Sendable { case short = "kurz", normal = "normal", long = "lang" }
        public var style: Style = .natural
        public var pause: Pause = .normal
        public var interruptible = true
        /// 0 (aus) oder 5...120 Sekunden Stille bis zur Nachfrage.
        public var silenceFollowUpSeconds = 0
        public var extra: [AssistantJSONMember] = []
        public init() {}
    }

    public struct Context: Equatable, Sendable {
        public enum Kind: String, Sendable, CaseIterable {
            case book = "buch", meeting = "besprechung", voiceNote = "sprachnotiz", file = "datei"
            case folder = "ordner", project = "projekt", section = "abschnitt"
        }
        public var kind: Kind
        public var id: String
        public var title: String
        public var extra: [AssistantJSONMember] = []
        public init(kind: Kind, id: String, title: String = "") {
            self.kind = kind
            self.id = id
            self.title = title
        }
    }

    public struct Knowledge: Equatable, Sendable {
        public enum Mode: String, Sendable { case all = "alles", first = "zuerst", only = "nur" }
        public var allAreas = false
        public var areas: [String] = []
        public var mode: Mode = .all
        public var contexts: [Context] = []
        public var extra: [AssistantJSONMember] = []
        public init() {}
    }

    public struct Field: Equatable, Sendable {
        public enum Kind: String, Sendable { case text = "text", number = "zahl", date = "datum", choice = "auswahl" }
        public var name: String
        public var descriptionText: String = ""
        public var kind: Kind = .text
        public var options: [String] = []
        public var extra: [AssistantJSONMember] = []
        public init(name: String, descriptionText: String = "", kind: Kind = .text, options: [String] = []) {
            self.name = name
            self.descriptionText = descriptionText
            self.kind = kind
            self.options = options
        }
    }

    public struct Evaluation: Equatable, Sendable {
        public var summary = false
        public var tasks = false
        public var fields: [Field] = []
        public var extra: [AssistantJSONMember] = []
        public init() {}
    }

    public var id: String
    public var name: String
    public var descriptionText = ""
    public var instructions = ""
    public var greeting = ""
    public var language = "de"
    public var voice: String?
    public var behavior = Behavior()
    public var knowledge = Knowledge()
    public var interfaces: [String] = []
    public var evaluation = Evaluation()
    /// Unbekannte Felder der obersten Ebene.
    public var extra: [AssistantJSONMember] = []

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }

    // MARK: Lesen

    public static func parse(json: String) -> Result<AssistantProfile, AssistantProfileFailure> {
        guard let tree = AssistantJSON.parse(json) else {
            return .failure(AssistantProfileFailure([AssistantProfileIssue(path: "", code: .json)]))
        }
        return parse(tree: tree)
    }

    /// Ungültiges UTF-8 ist ein JSON-Fehler.
    public static func parse(data: Data) -> Result<AssistantProfile, AssistantProfileFailure> {
        guard let text = String(data: data, encoding: .utf8) else {
            return .failure(AssistantProfileFailure([AssistantProfileIssue(path: "", code: .json)]))
        }
        return parse(json: text)
    }

    public static func parse(tree: AssistantJSON) -> Result<AssistantProfile, AssistantProfileFailure> {
        guard case .object(let root) = tree else {
            return .failure(AssistantProfileFailure([AssistantProfileIssue(path: "", code: .json)]))
        }
        var checker = AssistantProfileChecker()
        let profile = checker.profile(root)
        if checker.issues.isEmpty, let profile { return .success(profile) }
        return .failure(AssistantProfileFailure(checker.issues))
    }

    /// Prüft das Profil in seinem heutigen Stand (nach Änderungen an den typisierten Feldern).
    public func validate() -> [AssistantProfileIssue] {
        if case .failure(let failure) = AssistantProfile.parse(tree: tree()) { return failure.issues }
        return []
    }

    // MARK: Schreiben

    /// Der kanonische Text (sortierte Schlüssel, 2 Leerzeichen, `\n` am Ende), auf beiden Plattformen bytegleich.
    public func encode() -> String { tree().encoded() }

    public func tree() -> AssistantJSON {
        var root = extra
        root.set("schema", .string(AssistantProfile.schema))
        root.set("id", .string(id))
        root.set("name", .string(name))
        root.set("beschreibung", .string(descriptionText))
        root.set("anweisungen", .string(instructions))
        root.set("begruessung", .string(greeting))
        root.set("sprache", .string(language))
        root.set("stimme", voice.map { .string($0) } ?? .null)

        var b = behavior.extra
        b.set("stil", .string(behavior.style.rawValue))
        b.set("sprechpause", .string(behavior.pause.rawValue))
        b.set("unterbrechen", .bool(behavior.interruptible))
        b.set("nachfrage_stille_s", .integer(behavior.silenceFollowUpSeconds))
        root.set("verhalten", .object(b))

        var k = knowledge.extra
        k.set("alle_bereiche", .bool(knowledge.allAreas))
        k.set("bereiche", .array(knowledge.areas.map { .string($0) }))
        k.set("modus", .string(knowledge.mode.rawValue))
        k.set("kontexte", .array(knowledge.contexts.map { context in
            var c = context.extra
            c.set("art", .string(context.kind.rawValue))
            c.set("id", .string(context.id))
            c.set("titel", .string(context.title))
            return .object(c)
        }))
        root.set("wissen", .object(k))

        root.set("schnittstellen", .array(interfaces.map { .string($0) }))

        var e = evaluation.extra
        e.set("zusammenfassung", .bool(evaluation.summary))
        e.set("aufgaben", .bool(evaluation.tasks))
        e.set("felder", .array(evaluation.fields.map { field in
            var f = field.extra
            f.set("name", .string(field.name))
            f.set("beschreibung", .string(field.descriptionText))
            f.set("typ", .string(field.kind.rawValue))
            f.set("optionen", .array(field.options.map { .string($0) }))
            return .object(f)
        }))
        root.set("auswertung", .object(e))
        return .object(root)
    }
}

// MARK: - Bereiche und Kontextmodus

extension AssistantProfile {
    /// Abbildung der Profil-Schlüssel auf die Bereiche des iPhone; übrige Schlüssel haben dort keine Wirkung.
    public static let areaMapping: [(key: String, area: AssistantArea)] = [
        ("buecher", .books), ("besprechungen", .transcripts), ("ablage", .files), ("sprachnotizen", .notes),
    ]

    /// Wirksame Bereiche = (alle Bereiche der Plattform ODER die abgebildeten Schlüssel des Profils) ∩ freigegebene.
    /// Das Profil öffnet nie mehr, als freigegeben ist. Reihenfolge wie `AssistantArea.allCases`.
    public func effectiveAreas(released: Set<AssistantArea>) -> [AssistantArea] {
        let wanted: Set<AssistantArea> = knowledge.allAreas
            ? Set(AssistantArea.allCases)
            : Set(AssistantProfile.areaMapping.filter { entry in
                knowledge.areas.contains { $0.utf8.elementsEqual(entry.key.utf8) }
            }.map(\.area))
        return AssistantArea.allCases.filter { wanted.contains($0) && released.contains($0) }
    }

    /// `zuerst`/`nur` ohne Kontexte wirkt wie `alles`.
    public var effectiveContextMode: Knowledge.Mode {
        knowledge.mode != .all && knowledge.contexts.isEmpty ? .all : knowledge.mode
    }
}

// MARK: - Prüfung

/// Muster von Hand (nur ASCII): erstes Zeichen aus `first`, übrige aus `rest`, Länge 1...`maxLength`.
enum AssistantProfilePatterns {
    static func lower(_ c: UInt32) -> Bool { (0x61...0x7A).contains(c) }
    static func upper(_ c: UInt32) -> Bool { (0x41...0x5A).contains(c) }
    static func digit(_ c: UInt32) -> Bool { (0x30...0x39).contains(c) }

    private static func matches(_ s: String, maxLength: Int, first: (UInt32) -> Bool, rest: (UInt32) -> Bool) -> Bool {
        let scalars = s.unicodeScalars.map(\.value)
        guard let head = scalars.first, scalars.count <= maxLength, first(head) else { return false }
        return scalars.dropFirst().allSatisfy(rest)
    }

    /// `^[a-z0-9][a-z0-9-]{0,63}$`
    static func id(_ s: String) -> Bool {
        matches(s, maxLength: 64, first: { lower($0) || digit($0) }, rest: { lower($0) || digit($0) || $0 == 0x2D })
    }

    /// `^[a-z][a-z0-9_]{0,39}$` (Bereichsschlüssel und Feldnamen)
    static func key(_ s: String) -> Bool {
        matches(s, maxLength: 40, first: { lower($0) }, rest: { lower($0) || digit($0) || $0 == 0x5F })
    }

    /// `^[a-z0-9][a-z0-9._-]{0,63}$`
    static func interface(_ s: String) -> Bool {
        matches(s, maxLength: 64, first: { lower($0) || digit($0) }, rest: { lower($0) || digit($0) || $0 == 0x2E || $0 == 0x5F || $0 == 0x2D })
    }

    /// `"auto"` oder `^[a-z]{2}(-[A-Z]{2})?$`
    static func language(_ s: String) -> Bool {
        if s == "auto" { return true }
        let c = s.unicodeScalars.map(\.value)
        if c.count == 2 { return lower(c[0]) && lower(c[1]) }
        return c.count == 5 && lower(c[0]) && lower(c[1]) && c[2] == 0x2D && upper(c[3]) && upper(c[4])
    }

    /// Steuerzeichen: U+0000...U+001F, U+007F...U+009F, U+2028, U+2029.
    static func isControl(_ c: UInt32) -> Bool {
        c < 0x20 || (0x7F...0x9F).contains(c) || c == 0x2028 || c == 0x2029
    }

    static func count(_ s: String) -> Int { s.unicodeScalars.count }

    /// Entfernt nur U+0020 an beiden Enden.
    static func trimSpaces(_ s: String) -> String {
        var scalars = Array(s.unicodeScalars)
        while scalars.last?.value == 0x20 { scalars.removeLast() }
        var start = 0
        while start < scalars.count, scalars[start].value == 0x20 { start += 1 }
        var out = String.UnicodeScalarView()
        out.append(contentsOf: scalars[start...])
        return String(out)
    }
}

struct AssistantProfileChecker {
    typealias Member = AssistantJSONMember
    typealias P = AssistantProfilePatterns
    var issues: [AssistantProfileIssue] = []

    mutating func add(_ path: String, _ code: AssistantProfileIssue.Code) {
        issues.append(AssistantProfileIssue(path: path, code: code))
    }

    // Lesehilfen: liefern den Wert bei richtigem Typ, melden sonst `fehlt` (nur bei `required`) oder `typ`.
    mutating func text(_ o: [Member], _ key: String, _ path: String, required: Bool = false) -> String? {
        guard let v = o.value(key) else {
            if required { add(path, .missing) }
            return nil
        }
        guard case .string(let s) = v else { add(path, .type); return nil }
        return s
    }

    mutating func flag(_ o: [Member], _ key: String, _ path: String) -> Bool? {
        guard let v = o.value(key) else { return nil }
        guard case .bool(let b) = v else { add(path, .type); return nil }
        return b
    }

    mutating func list(_ o: [Member], _ key: String, _ path: String, required: Bool = false) -> [AssistantJSON]? {
        guard let v = o.value(key) else {
            if required { add(path, .missing) }
            return nil
        }
        guard case .array(let items) = v else { add(path, .type); return nil }
        return items
    }

    mutating func object(_ o: [Member], _ key: String, _ path: String) -> [Member]? {
        guard let v = o.value(key) else { return nil }
        guard case .object(let members) = v else { add(path, .type); return nil }
        return members
    }

    /// Text mit Längengrenzen in Codepunkten; bei Verstoß `laenge`.
    mutating func bounded(_ s: String?, _ path: String, _ range: ClosedRange<Int>) -> String? {
        guard let s else { return nil }
        if !range.contains(P.count(s)) { add(path, .length) }
        return s
    }

    mutating func texts(_ items: [AssistantJSON], _ path: String, _ range: ClosedRange<Int>) -> [String] {
        var out: [String] = []
        for (i, item) in items.enumerated() {
            guard case .string(let s) = item else { add("\(path)[\(i)]", .type); continue }
            if !range.contains(P.count(s)) { add("\(path)[\(i)]", .length) }
            out.append(s)
        }
        return out
    }

    mutating func profile(_ root: [Member]) -> AssistantProfile? {
        switch root.value("schema") {
        case nil: add("schema", .missing)
        case .string(let s)?:
            if s != AssistantProfile.schema {
                issues = [AssistantProfileIssue(path: "schema", code: AssistantProfileChecker.schemaCode(s))]
                return nil
            }
        default: add("schema", .type)
        }

        let idText = text(root, "id", "id", required: true)
        if let idText, !P.id(idText) { add("id", .pattern) }
        let nameText = text(root, "name", "name", required: true).map { P.trimSpaces($0) }
        if let nameText {
            if !(1...60).contains(P.count(nameText)) { add("name", .length) }
            if nameText.unicodeScalars.contains(where: { P.isControl($0.value) }) { add("name", .pattern) }
        }
        var profile = AssistantProfile(id: idText ?? "", name: nameText ?? "")
        profile.descriptionText = bounded(text(root, "beschreibung", "beschreibung"), "beschreibung", 0...200) ?? ""
        profile.instructions = bounded(text(root, "anweisungen", "anweisungen"), "anweisungen", 0...8000) ?? ""
        profile.greeting = bounded(text(root, "begruessung", "begruessung"), "begruessung", 0...300) ?? ""

        if let lang = text(root, "sprache", "sprache") {
            if !P.language(lang) { add("sprache", .pattern) }
            profile.language = lang
        }
        switch root.value("stimme") {
        case .string(let s)?:
            profile.voice = bounded(s, "stimme", 1...120)
        case .null?, nil: break
        default: add("stimme", .type)
        }

        if let o = object(root, "verhalten", "verhalten") { profile.behavior = behavior(o) }
        if let o = object(root, "wissen", "wissen") { profile.knowledge = knowledge(o) }
        if let items = list(root, "schnittstellen", "schnittstellen") { profile.interfaces = interfaces(items) }
        if let o = object(root, "auswertung", "auswertung") { profile.evaluation = evaluation(o) }

        profile.extra = root.unknown(excluding: [
            "schema", "id", "name", "beschreibung", "anweisungen", "begruessung", "sprache", "stimme",
            "verhalten", "wissen", "schnittstellen", "auswertung",
        ])
        return profile
    }

    static func schemaCode(_ s: String) -> AssistantProfileIssue.Code {
        let prefix = "lva.assistent/"
        guard s.utf8.starts(with: prefix.utf8) else { return .schemaUnknown }
        let rest = Array(s.utf8.dropFirst(prefix.utf8.count))
        guard let head = rest.first, (0x31...0x39).contains(head), rest.allSatisfy({ (0x30...0x39).contains($0) }) else {
            return .schemaUnknown
        }
        return rest == [0x31] ? .schemaUnknown : .schemaNewer
    }

    mutating func behavior(_ o: [Member]) -> AssistantProfile.Behavior {
        var b = AssistantProfile.Behavior()
        if let s = text(o, "stil", "verhalten.stil") {
            if let v = AssistantProfile.Behavior.Style(rawValue: s) { b.style = v } else { add("verhalten.stil", .value) }
        }
        if let s = text(o, "sprechpause", "verhalten.sprechpause") {
            if let v = AssistantProfile.Behavior.Pause(rawValue: s) { b.pause = v } else { add("verhalten.sprechpause", .value) }
        }
        if let v = flag(o, "unterbrechen", "verhalten.unterbrechen") { b.interruptible = v }
        if let v = o.value("nachfrage_stille_s") {
            let path = "verhalten.nachfrage_stille_s"
            switch AssistantProfileChecker.integer(v) {
            case .notInteger: add(path, .type)
            case .outOfRange: add(path, .value)
            case .value(let n):
                if n == 0 || (5...120).contains(n) { b.silenceFollowUpSeconds = n } else { add(path, .value) }
            }
        }
        b.extra = o.unknown(excluding: ["stil", "sprechpause", "unterbrechen", "nachfrage_stille_s"])
        return b
    }

    enum IntegerReading { case notInteger, outOfRange, value(Int) }

    /// Ganzzahl-Schreibweise `-?(0|[1-9][0-9]*)` bis 15 Ziffern; Bruch oder Exponent sind keine Ganzzahl.
    static func integer(_ v: AssistantJSON) -> IntegerReading {
        guard case .number(let raw) = v else { return .notInteger }
        let b = Array(raw.utf8)
        let digits = b.first == 0x2D ? Array(b.dropFirst()) : b
        guard !digits.isEmpty, digits.allSatisfy({ (0x30...0x39).contains($0) }) else { return .notInteger }
        guard digits.count <= 15, let n = Int(raw) else { return .outOfRange }
        return .value(n)
    }

    mutating func knowledge(_ o: [Member]) -> AssistantProfile.Knowledge {
        var k = AssistantProfile.Knowledge()
        if let v = flag(o, "alle_bereiche", "wissen.alle_bereiche") { k.allAreas = v }
        if let items = list(o, "bereiche", "wissen.bereiche") {
            var seen: [String] = []
            for (i, item) in items.enumerated() {
                let path = "wissen.bereiche[\(i)]"
                guard case .string(let s) = item else { add(path, .type); continue }
                if !P.key(s) { add(path, .pattern) }
                else if seen.contains(s) { add(path, .duplicate) }
                seen.append(s)
                k.areas.append(s)
            }
        }
        if let s = text(o, "modus", "wissen.modus") {
            if let v = AssistantProfile.Knowledge.Mode(rawValue: s) { k.mode = v } else { add("wissen.modus", .value) }
        }
        if let items = list(o, "kontexte", "wissen.kontexte") {
            if items.count > 50 { add("wissen.kontexte", .tooMany) }
            for (i, item) in items.enumerated() {
                let path = "wissen.kontexte[\(i)]"
                guard case .object(let c) = item else { add(path, .type); continue }
                let kindText = text(c, "art", path + ".art", required: true)
                var kind = AssistantProfile.Context.Kind.book
                if let kindText {
                    if let v = AssistantProfile.Context.Kind(rawValue: kindText) { kind = v } else { add(path + ".art", .value) }
                }
                let idText = bounded(text(c, "id", path + ".id", required: true), path + ".id", 1...200) ?? ""
                let title = bounded(text(c, "titel", path + ".titel"), path + ".titel", 0...200) ?? ""
                var context = AssistantProfile.Context(kind: kind, id: idText, title: title)
                context.extra = c.unknown(excluding: ["art", "id", "titel"])
                k.contexts.append(context)
            }
        }
        k.extra = o.unknown(excluding: ["alle_bereiche", "bereiche", "modus", "kontexte"])
        return k
    }

    mutating func interfaces(_ items: [AssistantJSON]) -> [String] {
        if items.count > 32 { add("schnittstellen", .tooMany) }
        var seen: [String] = []
        var out: [String] = []
        for (i, item) in items.enumerated() {
            let path = "schnittstellen[\(i)]"
            guard case .string(let s) = item else { add(path, .type); continue }
            if !P.interface(s) { add(path, .pattern) }
            else if seen.contains(s) { add(path, .duplicate) }
            seen.append(s)
            out.append(s)
        }
        return out
    }

    mutating func evaluation(_ o: [Member]) -> AssistantProfile.Evaluation {
        var e = AssistantProfile.Evaluation()
        if let v = flag(o, "zusammenfassung", "auswertung.zusammenfassung") { e.summary = v }
        if let v = flag(o, "aufgaben", "auswertung.aufgaben") { e.tasks = v }
        if let items = list(o, "felder", "auswertung.felder") {
            if items.count > 30 { add("auswertung.felder", .tooMany) }
            var names: [String] = []
            for (i, item) in items.enumerated() {
                let path = "auswertung.felder[\(i)]"
                guard case .object(let f) = item else { add(path, .type); continue }
                var field = AssistantProfile.Field(name: "")
                if let name = text(f, "name", path + ".name", required: true) {
                    if !P.key(name) { add(path + ".name", .pattern) }
                    else if names.contains(name) { add(path + ".name", .duplicate) }
                    names.append(name)
                    field.name = name
                }
                field.descriptionText = bounded(text(f, "beschreibung", path + ".beschreibung"), path + ".beschreibung", 0...200) ?? ""
                var isChoice = false
                if let t = text(f, "typ", path + ".typ", required: true) {
                    if let v = AssistantProfile.Field.Kind(rawValue: t) {
                        field.kind = v
                        isChoice = v == .choice
                    } else {
                        add(path + ".typ", .value)
                    }
                }
                let optionsPath = path + ".optionen"
                if let items = list(f, "optionen", optionsPath, required: isChoice) {
                    if isChoice {
                        if items.count < 2 { add(optionsPath, .length) }
                        if items.count > 20 { add(optionsPath, .tooMany) }
                    }
                    field.options = texts(items, optionsPath, 1...60)
                }
                field.extra = f.unknown(excluding: ["name", "beschreibung", "typ", "optionen"])
                e.fields.append(field)
            }
        }
        e.extra = o.unknown(excluding: ["zusammenfassung", "aufgaben", "felder"])
        return e
    }
}
