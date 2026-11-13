import Foundation

// Auswertung am Gesprächsende (lva#559, Paket G5-Kern): Ergebnisformat `lva.auswertung/1`, tolerantes Lesen der Rohantwort eines
// Modells mit Erdung im Code, Kodierung und Prüfung. Reine Funktionen ohne Oberfläche, ohne Modellaufruf und ohne Speicher; die
// Auswertung schreibt nie in Logs, Hinweise tragen nur Pfad und Code. Gegenstück am PC: `assistenten/auswertung.ts`. Regeln:
// `apps/shared/assistenten/README.md` (Abschnitt „Auswertung“), gemeinsame Prüfstände: `auswertung-pruefstaende.json`
// (beide Kerne bestehen alle Fälle bytegleich). Prompt und Gesprächsfassung: `AssistantOutcomePrompt.swift`, Fristauflösung und
// Textbausteine: `AssistantOutcomeDates.swift`.
//
// Vergleichsregel: Text des Nutzers oder des Modells wird überall GENAU verglichen (UTF-8-Bytes), nie über `String ==`, `Set<String>`
// oder `Dictionary<String, _>` (Swift setzt dort kanonische Gleichheit an: `é` gleich `e` + U+0301). Die `Equatable`-Umsetzungen der
// Ergebnis-Typen vergleichen darum ihre Zeichenketten über `AssistantExact` (Profil-Kern).

// MARK: - Eingabe

/// Ein Beitrag des Gesprächs; die Nummer ist die Position (ab 1) im übergebenen Verlauf.
public struct AssistantOutcomeTurn: Equatable, Sendable {
    public enum Role: String, Sendable { case user = "nutzer", assistant = "assistent" }
    public var role: Role
    public var text: String

    public init(role: Role, text: String) {
        self.role = role
        self.text = text
    }

    public static func == (a: AssistantOutcomeTurn, b: AssistantOutcomeTurn) -> Bool {
        a.role == b.role && AssistantExact.equal(a.text, b.text)
    }
}

// MARK: - Hinweise

/// Ein Hinweis der Auswertung: nur Pfad und Code, nie Inhalt des Modells.
public struct AssistantOutcomeHint: Equatable, Sendable {
    public enum Code: String, Sendable, CaseIterable {
        case quoteMissing = "beleg_fehlt"
        case valueNotInQuote = "wert_nicht_im_beleg"
        case type = "typ"
        case value = "wert"
        case unknown = "unbekannt"
        case duplicate = "doppelt"
        case length = "laenge"
        case missing = "fehlt"
        case tooMany = "zu_viele"
        case dueOtherTurn = "frist_fremder_beitrag"
    }

    public let path: String
    public let code: Code

    public init(path: String, code: Code) {
        self.path = path
        self.code = code
    }

    public static func == (a: AssistantOutcomeHint, b: AssistantOutcomeHint) -> Bool {
        a.code == b.code && AssistantExact.equal(a.path, b.path)
    }
}

/// Die Rohantwort enthält kein auffindbares JSON-Objekt; der Aufrufer kann einmal neu fragen.
public enum AssistantOutcomeFailure: String, Error, Equatable, Sendable { case json }

public struct AssistantOutcomeEvaluation: Equatable, Sendable {
    public let outcome: AssistantOutcome
    /// Nach Pfad, dann Code sortiert (UTF-8-Reihenfolge), ohne Wiederholungen.
    public let hints: [AssistantOutcomeHint]
}

// MARK: - Ergebnis

public struct AssistantOutcome: Equatable, Sendable {
    public static let schema = "lva.auswertung/1"
    public static let maxSummary = 1200
    public static let maxTasks = 20
    public static let maxTaskText = 300
    public static let maxDueText = 100
    public static let maxQuestions = 10
    public static let maxQuestion = 300
    public static let maxValue = 500
    public static let maxQuote = 500
    /// Längere Rohantworten (Codepunkte) sind der Fehler `json`, ohne weitere Arbeit.
    public static let maxRaw = 262_144
    /// So viele `{` ohne schließende Klammer werden höchstens ausprobiert; das nächste beendet die Suche mit `json`.
    public static let maxOpenStarts = 16

    static let kinds: [AssistantProfile.Field.Kind] = [.text, .number, .date, .choice]

    public enum Status: String, Sendable, CaseIterable { case clear = "klar", unclear = "unklar", notMentioned = "nicht_genannt" }

    public struct Field: Equatable, Sendable {
        public var name: String
        public var kind: AssistantProfile.Field.Kind
        public var status: Status
        public var value: String?
        public var quote: String?
        public var turn: Int?

        public init(name: String, kind: AssistantProfile.Field.Kind, status: Status = .notMentioned, value: String? = nil, quote: String? = nil, turn: Int? = nil) {
            self.name = name
            self.kind = kind
            self.status = status
            self.value = value
            self.quote = quote
            self.turn = turn
        }

        public static func == (a: Field, b: Field) -> Bool {
            AssistantExact.equal(a.name, b.name) && a.kind == b.kind && a.status == b.status && AssistantExact.equal(a.value, b.value)
                && AssistantExact.equal(a.quote, b.quote) && a.turn == b.turn
        }
    }

    public struct Task: Equatable, Sendable {
        public var text: String
        /// `JJJJ-MM-TT`, nur wenn die Fristauflösung `dueText` als ganzen, maximalen Ausdruck im Gespräch auflöst.
        public var due: String?
        public var dueText: String
        /// Nummer des Beitrags, in dem `dueText` steht (ab 1); `nil` genau dann, wenn `due` `nil` ist.
        public var dueTurn: Int?
        public var quote: String
        public var turn: Int

        public init(text: String, due: String? = nil, dueText: String = "", dueTurn: Int? = nil, quote: String, turn: Int) {
            self.text = text
            self.due = due
            self.dueText = dueText
            self.dueTurn = dueTurn
            self.quote = quote
            self.turn = turn
        }

        public static func == (a: Task, b: Task) -> Bool {
            AssistantExact.equal(a.text, b.text) && AssistantExact.equal(a.due, b.due) && AssistantExact.equal(a.dueText, b.dueText)
                && a.dueTurn == b.dueTurn && AssistantExact.equal(a.quote, b.quote) && a.turn == b.turn
        }
    }

    public var assistantId: String
    /// ISO-8601 UTC (`JJJJ-MM-TTThh:mm:ssZ`), vom Aufrufer; leer, solange er nichts gesetzt hat.
    public var created: String
    public var truncated: Bool
    public var summary: String
    public var fields: [Field]
    public var tasks: [Task]
    public var openQuestions: [String]

    public init(assistantId: String, created: String = "", truncated: Bool = false, summary: String = "", fields: [Field] = [], tasks: [Task] = [], openQuestions: [String] = []) {
        self.assistantId = assistantId
        self.created = created
        self.truncated = truncated
        self.summary = summary
        self.fields = fields
        self.tasks = tasks
        self.openQuestions = openQuestions
    }

    public static func == (a: AssistantOutcome, b: AssistantOutcome) -> Bool {
        AssistantExact.equal(a.assistantId, b.assistantId) && AssistantExact.equal(a.created, b.created) && a.truncated == b.truncated
            && AssistantExact.equal(a.summary, b.summary) && a.fields == b.fields && a.tasks == b.tasks
            && AssistantExact.equal(a.openQuestions, b.openQuestions)
    }

    // MARK: Schreiben

    public func tree() -> AssistantJSON {
        func text(_ s: String?) -> AssistantJSON { s.map { .string($0) } ?? .null }
        var root: [AssistantJSONMember] = []
        root.set("schema", .string(AssistantOutcome.schema))
        root.set("assistent", .string(assistantId))
        root.set("erstellt", .string(created))
        root.set("gekuerzt", .bool(truncated))
        root.set("zusammenfassung", .string(summary))
        root.set("felder", .array(fields.map { field in
            var o: [AssistantJSONMember] = []
            o.set("name", .string(field.name))
            o.set("typ", .string(field.kind.rawValue))
            o.set("status", .string(field.status.rawValue))
            o.set("wert", text(field.value))
            o.set("beleg", text(field.quote))
            o.set("beitrag", field.turn.map { .integer($0) } ?? .null)
            return .object(o)
        }))
        root.set("aufgaben", .array(tasks.map { task in
            var o: [AssistantJSONMember] = []
            o.set("text", .string(task.text))
            o.set("frist", text(task.due))
            o.set("frist_beitrag", task.dueTurn.map { .integer($0) } ?? .null)
            o.set("frist_text", .string(task.dueText))
            o.set("beleg", .string(task.quote))
            o.set("beitrag", .integer(task.turn))
            return .object(o)
        }))
        root.set("offene_fragen", .array(openQuestions.map { .string($0) }))
        return .object(root)
    }

    /// Der kanonische Text (sortierte Schlüssel, 2 Leerzeichen, `\n` am Ende), auf beiden Plattformen bytegleich.
    public func encode() -> String { tree().encoded() }

    // MARK: Lesen und Prüfen

    /// Liest und prüft ein Ergebnis `lva.auswertung/1`. Unbekannte Schlüssel werden verworfen; Fehler wie beim Profil (Pfad, Code).
    public static func parse(json: String) -> Result<AssistantOutcome, AssistantProfileFailure> {
        guard let tree = AssistantJSON.parse(json), case .object(let root) = tree else {
            return .failure(AssistantProfileFailure([AssistantProfileIssue(path: "", code: .json)]))
        }
        var reader = AssistantOutcomeReader()
        if let outcome = reader.outcome(root), reader.issues.isEmpty { return .success(outcome) }
        return .failure(AssistantProfileFailure(reader.issues))
    }
}

/// Leser mit Fehlerliste für `AssistantOutcome.parse`.
struct AssistantOutcomeReader {
    typealias Member = AssistantJSONMember
    typealias T = AssistantOutcomeText
    var issues: [AssistantProfileIssue] = []

    mutating func add(_ path: String, _ code: AssistantProfileIssue.Code) { issues.append(AssistantProfileIssue(path: path, code: code)) }

    mutating func text(_ o: [Member], _ key: String, _ path: String, _ range: ClosedRange<Int>) -> String? {
        guard let v = o.value(key) else { add(path, .missing); return nil }
        guard case .string(let s) = v else { add(path, .type); return nil }
        if !range.contains(s.unicodeScalars.count) { add(path, .length) }
        return s
    }

    /// `null` ist erlaubt (äußeres `nil` heißt Fehler, inneres `nil` heißt null).
    mutating func textOrNull(_ o: [Member], _ key: String, _ path: String, _ max: Int) -> String?? {
        guard let v = o.value(key) else { add(path, .missing); return nil }
        if case .null = v { return .some(nil) }
        guard case .string(let s) = v else { add(path, .type); return nil }
        if s.unicodeScalars.count > max { add(path, .length) }
        return .some(s)
    }

    mutating func turn(_ o: [Member], _ key: String, _ path: String, nullAllowed: Bool) -> Int?? {
        guard let v = o.value(key) else { add(path, .missing); return nil }
        if case .null = v, nullAllowed { return .some(nil) }
        guard case .number(let raw) = v else { add(path, .type); return nil }
        let b = Array(raw.utf8)
        let digits = b.first == 0x2D ? Array(b.dropFirst()) : b
        guard T.onlyDigits(digits), raw == "0" || digits.first != 0x30 else { add(path, .type); return nil }
        guard raw.utf8.count <= 9, let n = Int(raw), n >= 1 else { add(path, .value); return nil }
        return .some(n)
    }

    mutating func list(_ o: [Member], _ key: String, _ max: Int) -> [AssistantJSON] {
        guard let v = o.value(key) else { add(key, .missing); return [] }
        guard case .array(let items) = v else { add(key, .type); return [] }
        if items.count > max { add(key, .tooMany) }
        return items
    }

    static func schemaCode(_ s: String) -> AssistantProfileIssue.Code {
        let prefix = "lva.auswertung/"
        guard s.utf8.starts(with: prefix.utf8) else { return .schemaUnknown }
        let rest = Array(s.utf8.dropFirst(prefix.utf8.count))
        guard let head = rest.first, (0x31...0x39).contains(head), rest.allSatisfy({ (0x30...0x39).contains($0) }) else {
            return .schemaUnknown
        }
        return rest == [0x31] ? .schemaUnknown : .schemaNewer
    }

    /// `JJJJ-MM-TTThh:mm:ssZ` mit gültigem Kalendertag und Uhrzeit.
    static func isTimestamp(_ s: String) -> Bool {
        let b = Array(s.utf8)
        guard b.count == 20, b[10] == 0x54, b[13] == 0x3A, b[16] == 0x3A, b[19] == 0x5A else { return false }
        guard AssistantOutcomeDate.parseISO(String(decoding: b[0..<10], as: UTF8.self)) != nil else { return false }
        let parts = [b[11..<13], b[14..<16], b[17..<19]].map { Array($0) }
        guard parts.allSatisfy({ T.onlyDigits($0) }) else { return false }
        return T.int(parts[0]) <= 23 && T.int(parts[1]) <= 59 && T.int(parts[2]) <= 59
    }

    mutating func outcome(_ o: [Member]) -> AssistantOutcome? {
        switch o.value("schema") {
        case nil: add("schema", .missing)
        case .string(let s)?:
            if !AssistantOutcomeText.same(s, AssistantOutcome.schema) {
                issues = [AssistantProfileIssue(path: "schema", code: AssistantOutcomeReader.schemaCode(s))]
                return nil
            }
        default: add("schema", .type)
        }
        let assistant = text(o, "assistent", "assistent", 1...64) ?? ""
        if !assistant.isEmpty, !AssistantProfilePatterns.id(assistant) { add("assistent", .pattern) }
        let created = text(o, "erstellt", "erstellt", 0...20) ?? ""
        if !created.isEmpty, !AssistantOutcomeReader.isTimestamp(created) { add("erstellt", .pattern) }
        var truncated = false
        switch o.value("gekuerzt") {
        case nil: add("gekuerzt", .missing)
        case .bool(let b)?: truncated = b
        default: add("gekuerzt", .type)
        }
        let summary = text(o, "zusammenfassung", "zusammenfassung", 0...AssistantOutcome.maxSummary) ?? ""

        var names: [String] = []
        var fields: [AssistantOutcome.Field] = []
        for (i, item) in list(o, "felder", 30).enumerated() {
            if let field = field(item, i, &names) { fields.append(field) }
        }
        var tasks: [AssistantOutcome.Task] = []
        for (i, item) in list(o, "aufgaben", AssistantOutcome.maxTasks).enumerated() {
            if let task = task(item, i) { tasks.append(task) }
        }
        var questions: [String] = []
        for (i, item) in list(o, "offene_fragen", AssistantOutcome.maxQuestions).enumerated() {
            let path = "offene_fragen[\(i)]"
            guard case .string(let s) = item else { add(path, .type); continue }
            let length = s.unicodeScalars.count
            if length < 1 || length > AssistantOutcome.maxQuestion { add(path, .length) }
            questions.append(s)
        }
        return AssistantOutcome(assistantId: assistant, created: created, truncated: truncated, summary: summary, fields: fields, tasks: tasks, openQuestions: questions)
    }

    private mutating func field(_ item: AssistantJSON, _ i: Int, _ names: inout [String]) -> AssistantOutcome.Field? {
        let path = "felder[\(i)]"
        guard case .object(let o) = item else { add(path, .type); return nil }
        let name = text(o, "name", path + ".name", 1...40) ?? ""
        if !name.isEmpty {
            if !AssistantProfilePatterns.key(name) { add(path + ".name", .pattern) }
            else if names.contains(where: { T.same($0, name) }) { add(path + ".name", .duplicate) }
            names.append(name)
        }
        let kindText = text(o, "typ", path + ".typ", 1...40)
        let kind = AssistantOutcome.kinds.first { T.same($0.rawValue, kindText ?? "") }
        if kindText != nil, kind == nil { add(path + ".typ", .value) }
        let statusText = text(o, "status", path + ".status", 1...40)
        let status = AssistantOutcome.Status.allCases.first { T.same($0.rawValue, statusText ?? "") }
        if statusText != nil, status == nil { add(path + ".status", .value) }
        let value = textOrNull(o, "wert", path + ".wert", AssistantOutcome.maxValue) ?? nil
        let quote = textOrNull(o, "beleg", path + ".beleg", AssistantOutcome.maxQuote) ?? nil
        let turnRead = self.turn(o, "beitrag", path + ".beitrag", nullAllowed: true)
        let turn = turnRead ?? nil
        let resolved = status ?? .notMentioned
        if let quote, quote.isEmpty { add(path + ".beleg", .length) }
        if resolved == .notMentioned, value != nil || quote != nil || turn != nil { add(path, .value) }
        if case .some = turnRead, (quote == nil) != (turn == nil) { add(path + ".beitrag", .value) }
        if resolved != .notMentioned, value != nil, quote == nil { add(path + ".beleg", .value) }
        if resolved == .clear { clearValue(kind ?? .text, value, path + ".wert") }
        return AssistantOutcome.Field(name: name, kind: kind ?? .text, status: resolved, value: value, quote: quote, turn: turn)
    }

    /// `klar` verlangt einen typgerecht kanonischen Wert (Auswahloptionen kennt der Leser nicht).
    private mutating func clearValue(_ kind: AssistantProfile.Field.Kind, _ value: String?, _ path: String) {
        guard let value else { add(path, .value); return }
        switch kind {
        case .date: if AssistantOutcomeDate.parseISO(value) == nil { add(path, .pattern) }
        case .number: if !T.isCanonicalNumber(value) { add(path, .pattern) }
        case .text, .choice: if value.isEmpty { add(path, .value) }
        }
    }

    private mutating func task(_ item: AssistantJSON, _ i: Int) -> AssistantOutcome.Task? {
        let path = "aufgaben[\(i)]"
        guard case .object(let o) = item else { add(path, .type); return nil }
        let text = self.text(o, "text", path + ".text", 1...AssistantOutcome.maxTaskText) ?? ""
        let dueRead = textOrNull(o, "frist", path + ".frist", 10)
        let due = dueRead ?? nil
        if let due, AssistantOutcomeDate.parseISO(due) == nil { add(path + ".frist", .pattern) }
        let dueText = self.text(o, "frist_text", path + ".frist_text", 0...AssistantOutcome.maxDueText) ?? ""
        let dueTurnRead = self.turn(o, "frist_beitrag", path + ".frist_beitrag", nullAllowed: true)
        let dueTurn = dueTurnRead ?? nil
        let quote = self.text(o, "beleg", path + ".beleg", 1...AssistantOutcome.maxQuote) ?? ""
        let turn = self.turn(o, "beitrag", path + ".beitrag", nullAllowed: false) ?? nil
        if due != nil, dueText.isEmpty { add(path + ".frist_text", .value) }
        if case .some = dueTurnRead, case .some = dueRead, (due == nil) != (dueTurn == nil) { add(path + ".frist_beitrag", .value) }
        return AssistantOutcome.Task(text: text, due: due, dueText: dueText, dueTurn: dueTurn, quote: quote, turn: turn ?? 1)
    }
}

// MARK: - Auswerten

extension AssistantOutcome {
    /// Liest die Rohantwort des Modells und erdet sie im Gespräch. Kein auffindbares JSON-Objekt: `.failure(.json)` (der Aufrufer
    /// kann einmal neu fragen). Sonst das Ergebnis `lva.auswertung/1` mit allen Feldern des Profils in dessen Reihenfolge und die
    /// nach Pfad und Code sortierten Hinweise. `today` ist der Tag des Gesprächs; `created` und `truncated` (aus `prompt`) setzt
    /// der Aufrufer, der Kern kennt sie nicht. Ein `created`, das nicht `JJJJ-MM-TTThh:mm:ssZ` mit gültigem Tag und gültiger
    /// Uhrzeit ist, gilt als nicht gesetzt (leer), damit das Ergebnis immer lesbar bleibt. Das Profil muss die Profilprüfung
    /// bestanden haben (Id, Feldnamen, Optionen); sonst kann das Ergebnis nicht gelesen werden.
    public static func evaluate(
        profile: AssistantProfile,
        turns: [AssistantOutcomeTurn],
        raw: String,
        today: AssistantOutcomeDate,
        created: String = "",
        truncated: Bool = false
    ) -> Result<AssistantOutcomeEvaluation, AssistantOutcomeFailure> {
        guard let root = AssistantOutcomeGrounding.firstObject(in: raw) else { return .failure(.json) }
        var grounding = AssistantOutcomeGrounding(turns: turns, today: today)
        grounding.reportUnknown(root, known: ["felder", "zusammenfassung", "aufgaben", "offene_fragen"], path: "")
        let outcome = AssistantOutcome(
            assistantId: profile.id,
            created: AssistantOutcomeReader.isTimestamp(created) ? created : "",
            truncated: truncated,
            summary: grounding.summary(profile, root),
            fields: grounding.fields(profile, root),
            tasks: grounding.tasks(profile, root),
            openQuestions: grounding.questions(root)
        )
        return .success(AssistantOutcomeEvaluation(outcome: outcome, hints: grounding.orderedHints()))
    }
}

/// Zustand der Erdung: normalisierte Beiträge des ungekürzten Gesprächs, `heute` und die Hinweise.
struct AssistantOutcomeGrounding {
    typealias Member = AssistantJSONMember
    typealias T = AssistantOutcomeText
    typealias Code = AssistantOutcomeHint.Code

    /// Wörter und maximale Fristausdrücke eines Beitrags.
    struct TurnDeadlines {
        let words: [String]
        let matches: [AssistantOutcomeDates.DeadlineMatch]
    }

    let normalized: [[UInt32]]
    let today: AssistantOutcomeDate
    private(set) var hints: [AssistantOutcomeHint] = []
    /// Je Beitrag (Nummer ab 1) bei Bedarf berechnet.
    private var deadlineCache: [Int: TurnDeadlines] = [:]

    init(turns: [AssistantOutcomeTurn], today: AssistantOutcomeDate) {
        self.normalized = turns.map { T.normalizedScalars($0.text) }
        self.today = today
    }

    mutating func add(_ path: String, _ code: Code) { hints.append(AssistantOutcomeHint(path: path, code: code)) }

    func orderedHints() -> [AssistantOutcomeHint] {
        var seen = Set<AssistantExactKey>()
        var unique: [AssistantOutcomeHint] = []
        for hint in hints where seen.insert(AssistantExactKey(hint.path + "\u{0}" + hint.code.rawValue)).inserted {
            unique.append(hint)
        }
        return unique.sorted { a, b in
            if !T.same(a.path, b.path) { return a.path.utf8.lexicographicallyPrecedes(b.path.utf8) }
            return a.code.rawValue.utf8.lexicographicallyPrecedes(b.code.rawValue.utf8)
        }
    }

    // MARK: Rohantwort lesen

    /// Index (Byte) der schließenden Klammer des Objekts ab `start` (Zeichenketten und Escapes beachtet), sonst `nil`.
    private static func balancedEnd(_ b: [UInt8], from start: Int) -> Int? {
        var depth = 0
        var inString = false
        var escape = false
        var i = start
        while i < b.count {
            let c = b[i]
            if inString {
                if escape { escape = false } else if c == 0x5C { escape = true } else if c == 0x22 { inString = false }
            } else if c == 0x22 {
                inString = true
            } else if c == 0x7B {
                depth += 1
            } else if c == 0x7D {
                depth -= 1
                if depth == 0 { return i }
            }
            i += 1
        }
        return nil
    }

    /// Mehr als `max` Codepunkte? Zählt ohne Kopie und hört beim Überschreiten auf.
    private static func longerThan(_ s: String, _ max: Int) -> Bool {
        var n = 0
        for _ in s.unicodeScalars {
            n += 1
            if n > max { return true }
        }
        return false
    }

    /// Das erste ausbalancierte, gültige JSON-Objekt im Text (auch in Code-Zäunen oder mit Text davor und danach). Grenzen: mehr
    /// als `maxRaw` Codepunkte sind `nil`; ein `{` ohne schließende Klammer zählt als offener Start, der 17. beendet die Suche
    /// (sonst wäre sie quadratisch); die Tiefe begrenzt der strenge JSON-Leser wie beim Profil (64 Ebenen).
    static func firstObject(in text: String) -> [Member]? {
        if longerThan(text, AssistantOutcome.maxRaw) { return nil }
        let b = Array(text.utf8)
        var openStarts = 0
        var start = b.firstIndex(of: 0x7B)
        while let from = start {
            guard let end = balancedEnd(b, from: from) else {
                openStarts += 1
                if openStarts > AssistantOutcome.maxOpenStarts { return nil }
                start = b[(from + 1)...].firstIndex(of: 0x7B)
                continue
            }
            if let tree = AssistantJSON.parse(String(decoding: b[from...end], as: UTF8.self)), case .object(let members) = tree {
                return members
            }
            start = b[(end + 1)...].firstIndex(of: 0x7B)
        }
        return nil
    }

    // MARK: Helfer

    /// Meldet unbekannte Schlüssel eines Objekts: ein Hinweis `unbekannt` am Pfad des Objekts selbst, nie mit dem Schlüssel (er ist
    /// Inhalt des Modells).
    mutating func reportUnknown(_ o: [Member], known: [String], path: String) {
        if o.contains(where: { member in !known.contains(where: { T.same($0, member.key) }) }) { add(path, .unknown) }
    }

    /// Text (oder Zahl als Originaltext) eines Rohwerts, an den Enden von Leerraum befreit; leer und `null` ergeben `nil`.
    private mutating func readText(_ v: AssistantJSON?, _ path: String, allowNumber: Bool) -> String? {
        guard let v else { return nil }
        switch v {
        case .null: return nil
        case .string(let s):
            let t = T.trimWhitespace(s)
            return t.isEmpty ? nil : t
        case .number(let raw) where allowNumber: return raw
        default:
            add(path, .type)
            return nil
        }
    }

    private mutating func readTurn(_ v: AssistantJSON?, _ path: String) -> Int? {
        guard let v else { return nil }
        switch v {
        case .null: return nil
        case .number(let raw):
            let b = Array(raw.utf8)
            if T.onlyDigits(b), b.count <= 9, raw == "0" || b.first != 0x30 { return T.int(b) }
        case .string(let s):
            let b = Array(T.trimWhitespace(s).utf8)
            if T.onlyDigits(b), b.count <= 6 { return T.int(b) }
        default: break
        }
        add(path, .type)
        return nil
    }

    private static let rolePrefixes = ["nutzer", "assistent", "user", "assistant"]

    /// Streift ein Beitragspräfix vom Beleg: `[3] Nutzer: …`, `[3] …`, `Nutzer: …`, `Assistent: …`, `User: …`, `Assistant: …` (Rolle
    /// ohne Beachtung der Groß-/Kleinschreibung, Leerraum um Nummer und Doppelpunkt frei). Die Nummer (1 bis 6 Ziffern in eckigen
    /// Klammern) wird mitgeliefert. Ohne Präfix bleibt der Text unverändert.
    static func stripPrefix(_ quote: String) -> (text: String, turn: Int?) {
        let c = T.scalars(quote)
        var i = 0
        var number: Int?
        func skipSpace() { while i < c.count, T.isWhitespace(c[i]) { i += 1 } }
        func isLetter(_ x: UInt32) -> Bool { (0x41...0x5A).contains(x) || (0x61...0x7A).contains(x) }
        skipSpace()
        if i < c.count, c[i] == 0x5B {
            var j = i + 1
            while j < c.count, (0x30...0x39).contains(c[j]) { j += 1 }
            if j > i + 1, j - (i + 1) <= 6, j < c.count, c[j] == 0x5D {
                number = T.int(c[(i + 1)..<j].map { UInt8($0) })
                i = j + 1
                skipSpace()
            }
        }
        var j = i
        while j < c.count, j - i <= 9, isLetter(c[j]) { j += 1 }
        if j > i, j - i <= 9 {
            let word = T.string(c[i..<j].map { $0 >= 0x41 && $0 <= 0x5A ? $0 + 32 : $0 })
            var k = j
            while k < c.count, T.isWhitespace(c[k]) { k += 1 }
            if rolePrefixes.contains(where: { T.same($0, word) }), k < c.count, c[k] == 0x3A {
                i = k + 1
                skipSpace()
            }
        }
        if i == 0, number == nil { return (quote, nil) }
        return (T.string(c[i...]), number)
    }

    /// Beleg der Rohantwort: Präfix ab, auf `maxQuote` gekürzt; die Nummer des Präfixes gilt, wenn die Antwort keinen Beitrag nennt.
    private static func readQuote(_ raw: String?, _ rawTurn: Int?) -> (text: String?, capped: Bool, turn: Int?) {
        guard let raw else { return (nil, false, rawTurn) }
        let stripped = stripPrefix(raw)
        let turn = rawTurn ?? stripped.turn
        if stripped.text.isEmpty { return (nil, false, turn) }
        let short = cutHard(stripped.text, AssistantOutcome.maxQuote)
        return (short, !T.same(short, stripped.text), turn)
    }

    /// Nummer des Beitrags, in dem der (normalisierte) Beleg steht: der angegebene, sonst der erste, der ihn enthält.
    private func locate(_ quote: String, _ turn: Int?) -> Int? {
        let needle = T.normalizedScalars(quote)
        if needle.isEmpty { return nil }
        if let turn, turn >= 1, turn <= normalized.count, T.contains(normalized[turn - 1], needle) { return turn }
        return normalized.firstIndex { T.contains($0, needle) }.map { $0 + 1 }
    }

    /// Erste `max` Codepunkte, Leerraum am Ende entfernt.
    static func cutHard(_ s: String, _ max: Int) -> String {
        let c = T.scalars(s)
        if c.count <= max { return s }
        var end = max
        while end > 0, T.isWhitespace(c[end - 1]) { end -= 1 }
        return T.string(c[0..<end])
    }

    /// Kürzt am letzten Satzende (`.`, `!`, `?` vor Leerraum oder Textende) innerhalb von `max` Codepunkten, sonst hart.
    static func cutAtSentence(_ s: String, _ max: Int) -> String {
        let c = T.scalars(s)
        if c.count <= max { return s }
        var k = max - 1
        while k >= 0 {
            let end = c[k] == 0x2E || c[k] == 0x21 || c[k] == 0x3F
            if end, k + 1 == c.count || T.isWhitespace(c[k + 1]) { return T.string(c[0...k]) }
            k -= 1
        }
        return cutHard(s, max)
    }

    // MARK: Felder

    private func emptyField(_ f: AssistantProfile.Field) -> AssistantOutcome.Field {
        AssistantOutcome.Field(name: f.name, kind: f.kind)
    }

    private struct Canonical {
        var value: String
        var typeIssue: Code?
        var covered: Bool
    }

    /// Kanonischer Wert nach Typ und ob der Beleg ihn deckt (siehe README).
    private mutating func canonical(_ f: AssistantProfile.Field, _ rawValue: String, _ quote: String, _ path: String) -> Canonical {
        let quoteNormalized = T.normalizedScalars(quote)
        func failure(_ code: Code, _ g: inout AssistantOutcomeGrounding) -> Canonical {
            g.add(path, code)
            return Canonical(value: AssistantOutcomeGrounding.cutHard(rawValue, AssistantOutcome.maxValue), typeIssue: code, covered: false)
        }
        switch f.kind {
        case .text:
            let value = AssistantOutcomeGrounding.cutHard(rawValue, AssistantOutcome.maxValue)
            if !T.same(value, rawValue) { add(path, .length) }
            let n = T.normalizedScalars(value)
            return Canonical(value: value, typeIssue: nil, covered: !n.isEmpty && T.contains(quoteNormalized, n))
        case .number:
            guard let number = T.readNumber(rawValue), number.canonical.utf8.count <= AssistantOutcome.maxValue else {
                return failure(.type, &self)
            }
            let digits = Array(number.comparable.utf8)
            let whole = T.onlyDigits(digits) && digits.count <= 2 && T.int(digits) <= 12
            let covered = T.numbers(in: quoteNormalized).contains { T.same($0, number.comparable) }
                || (whole && T.hasNumberWord(quoteNormalized, T.int(digits)))
            return Canonical(value: number.canonical, typeIssue: nil, covered: covered)
        case .date:
            let direct = AssistantOutcomeDate.parseISO(rawValue)?.formatted
            guard let text = direct ?? AssistantOutcomeDates.deadline(rawValue, today: today) else { return failure(.type, &self) }
            let covered = AssistantOutcomeDates.coveredDeadline(in: quote, today: today).map { T.same($0, text) } ?? false
            return Canonical(value: text, typeIssue: nil, covered: covered)
        case .choice:
            let n = T.normalizedScalars(rawValue)
            let option = f.options.first { T.same($0, rawValue) }
                ?? f.options.first { !n.isEmpty && T.normalizedScalars($0) == n }
            guard let option else { return failure(.value, &self) }
            let nOption = T.normalizedScalars(option)
            return Canonical(value: option, typeIssue: nil, covered: !nOption.isEmpty && T.containsWordSequence(quoteNormalized, nOption))
        }
    }

    private mutating func field(_ f: AssistantProfile.Field, _ p: Int, _ raw: [Member]) -> AssistantOutcome.Field {
        let path = "felder[\(p)]"
        reportUnknown(raw, known: ["name", "status", "wert", "beleg", "beitrag"], path: path)
        let rawValue = readText(raw.value("wert"), path + ".wert", allowNumber: true)
        let rawQuote = readText(raw.value("beleg"), path + ".beleg", allowNumber: false)
        let rawTurn = readTurn(raw.value("beitrag"), path + ".beitrag")

        var status: AssistantOutcome.Status = rawValue == nil ? .notMentioned : .clear
        switch raw.value("status") {
        case nil: add(path + ".status", .missing)
        case .string(let s)?:
            if let known = AssistantOutcome.Status.allCases.first(where: { T.same($0.rawValue, s) }) { status = known }
            else { add(path + ".status", .value) }
        default: add(path + ".status", .type)
        }
        if status == .notMentioned { return emptyField(f) }
        if rawValue == nil, status == .clear { return emptyField(f) }

        let read = AssistantOutcomeGrounding.readQuote(rawQuote, rawTurn)
        let quote = read.text
        if read.capped { add(path + ".beleg", .length) }
        if rawValue == nil, quote == nil {
            var unclear = emptyField(f)
            unclear.status = .unclear
            return unclear
        }
        guard let quote, let turn = locate(quote, read.turn) else {
            add(path + ".beleg", .quoteMissing)
            return emptyField(f)
        }
        guard let rawValue else {
            var unclear = emptyField(f)
            unclear.status = .unclear
            unclear.quote = quote
            unclear.turn = turn
            return unclear
        }
        let c = canonical(f, rawValue, quote, path + ".wert")
        if c.typeIssue == nil, !c.covered { add(path + ".wert", .valueNotInQuote) }
        let clear = status == .clear && c.typeIssue == nil && c.covered
        return AssistantOutcome.Field(name: f.name, kind: f.kind, status: clear ? .clear : .unclear, value: c.value, quote: quote, turn: turn)
    }

    mutating func fields(_ profile: AssistantProfile, _ root: [Member]) -> [AssistantOutcome.Field] {
        let profileFields = profile.evaluation.fields
        var entries: [AssistantJSON] = []
        switch root.value("felder") {
        case nil, .null?: break
        case .array(let items)?: entries = items
        default: add("felder", .type)
        }
        if profileFields.isEmpty {
            if !entries.isEmpty { add("felder", .unknown) }
            return []
        }
        var found: [(name: String, members: [Member])] = []
        for (i, entry) in entries.enumerated() {
            guard case .object(let o) = entry else { add("felder[\(i)]", .type); continue }
            guard let nameValue = o.value("name") else { add("felder[\(i)]", .missing); continue }
            guard case .string(let name) = nameValue else { add("felder[\(i)]", .type); continue }
            if !profileFields.contains(where: { T.same($0.name, name) }) { add("felder[\(i)]", .unknown); continue }
            if found.contains(where: { T.same($0.name, name) }) { add("felder[\(i)]", .duplicate); continue }
            found.append((name, o))
        }
        return profileFields.enumerated().map { p, f in
            guard let entry = found.first(where: { T.same($0.name, f.name) }) else { return emptyField(f) }
            return field(f, p, entry.members)
        }
    }

    // MARK: Aufgaben, Zusammenfassung, Fragen

    /// Maximale Fristausdrücke des Beitrags `turn` (ab 1), höchstens die ersten `maxDeadlineWords` Wörter; je Beitrag einmal berechnet.
    private mutating func deadlines(of turn: Int) -> TurnDeadlines {
        if let cached = deadlineCache[turn] { return cached }
        let tokens = T.tokens(normalized[turn - 1], limit: AssistantOutcomeDates.maxDeadlineWords)
        let result = TurnDeadlines(words: tokens, matches: AssistantOutcomeDates.maximalDeadlines(tokens, today: today))
        deadlineCache[turn] = result
        return result
    }

    private mutating func deadlineFits(_ turn: Int, _ needle: [UInt32], _ expression: [String]) -> Bool {
        if !T.contains(normalized[turn - 1], needle) { return false }
        let d = deadlines(of: turn)
        return AssistantOutcomeDates.isMaximalExpression(expression, in: d.words, matches: d.matches)
    }

    /// Der Beitrag, in dem `dueText` als ganzer, sicherer, maximaler Fristausdruck steht: zuerst der Beitrag des Aufgabenbelegs
    /// (`origin`), dann der nächstgelegene (kleinster Abstand, bei Gleichstand der spätere); sonst `nil`.
    private mutating func searchDeadline(_ dueText: String, _ origin: Int) -> Int? {
        let needle = T.normalizedScalars(dueText)
        let expression = AssistantOutcomeDates.words(of: dueText)
        if deadlineFits(origin, needle, expression) { return origin }
        let count = normalized.count
        var distance = 1
        while origin - distance >= 1 || origin + distance <= count {
            if origin + distance <= count, deadlineFits(origin + distance, needle, expression) { return origin + distance }
            if origin - distance >= 1, deadlineFits(origin - distance, needle, expression) { return origin - distance }
            distance += 1
        }
        return nil
    }

    /// `frist_text` einer Aufgabe: im Gespräch gesucht, maximal und eindeutig aufgelöst (siehe README „Aufgaben“).
    private mutating func resolveDue(_ rawDue: String?, _ origin: Int, _ path: String) -> (text: String, day: String?, turn: Int?) {
        guard let rawDue else { return ("", nil, nil) }
        let text = AssistantOutcomeGrounding.cutHard(rawDue, AssistantOutcome.maxDueText)
        let needle = T.normalizedScalars(text)
        if needle.isEmpty || !normalized.contains(where: { T.contains($0, needle) }) {
            add(path, .quoteMissing)
            return ("", nil, nil)
        }
        guard let day = AssistantOutcomeDates.deadline(text, today: today) else { return (text, nil, nil) }
        guard let found = searchDeadline(text, origin) else {
            add(path, .valueNotInQuote)
            return (text, nil, nil)
        }
        if found != origin { add(path, .dueOtherTurn) }
        return (text, day, found)
    }

    mutating func tasks(_ profile: AssistantProfile, _ root: [Member]) -> [AssistantOutcome.Task] {
        var items: [AssistantJSON] = []
        switch root.value("aufgaben") {
        case nil, .null?: return []
        case .array(let list)?: items = list
        default:
            add("aufgaben", .type)
            return []
        }
        if !profile.evaluation.tasks {
            if !items.isEmpty { add("aufgaben", .unknown) }
            return []
        }
        var out: [AssistantOutcome.Task] = []
        var tooMany = false
        for (i, item) in items.enumerated() {
            let path = "aufgaben[\(i)]"
            guard case .object(let o) = item else { add(path, .type); continue }
            reportUnknown(o, known: ["text", "frist_text", "beleg", "beitrag"], path: path)
            let text = readText(o.value("text"), path + ".text", allowNumber: false)
            let rawDue = readText(o.value("frist_text"), path + ".frist_text", allowNumber: false)
            let rawQuote = readText(o.value("beleg"), path + ".beleg", allowNumber: false)
            let rawTurn = readTurn(o.value("beitrag"), path + ".beitrag")
            guard let text else { add(path + ".text", .missing); continue }
            let quote = AssistantOutcomeGrounding.readQuote(rawQuote, rawTurn)
            guard let quoteText = quote.text, let turn = locate(quoteText, quote.turn) else {
                add(path + ".beleg", .quoteMissing)
                continue
            }
            if out.count >= AssistantOutcome.maxTasks {
                tooMany = true
                continue
            }
            let short = AssistantOutcomeGrounding.cutAtSentence(text, AssistantOutcome.maxTaskText)
            if !T.same(short, text) { add(path + ".text", .length) }
            let due = resolveDue(rawDue, turn, path + ".frist_text")
            out.append(AssistantOutcome.Task(
                text: short,
                due: due.day,
                dueText: due.text,
                dueTurn: due.turn,
                quote: quoteText,
                turn: turn
            ))
        }
        if tooMany { add("aufgaben", .tooMany) }
        return out
    }

    mutating func summary(_ profile: AssistantProfile, _ root: [Member]) -> String {
        switch root.value("zusammenfassung") {
        case nil, .null?: return ""
        case .string(let s)?:
            let t = T.trimWhitespace(s)
            if !profile.evaluation.summary {
                if !t.isEmpty { add("zusammenfassung", .unknown) }
                return ""
            }
            let short = AssistantOutcomeGrounding.cutAtSentence(t, AssistantOutcome.maxSummary)
            if !T.same(short, t) { add("zusammenfassung", .length) }
            return short
        default:
            add("zusammenfassung", .type)
            return ""
        }
    }

    mutating func questions(_ root: [Member]) -> [String] {
        var items: [AssistantJSON] = []
        switch root.value("offene_fragen") {
        case nil, .null?: return []
        case .array(let list)?: items = list
        default:
            add("offene_fragen", .type)
            return []
        }
        var out: [String] = []
        var tooMany = false
        for (i, item) in items.enumerated() {
            let path = "offene_fragen[\(i)]"
            guard case .string(let s) = item else { add(path, .type); continue }
            let t = T.trimWhitespace(s)
            if t.isEmpty { continue }
            if out.count >= AssistantOutcome.maxQuestions {
                tooMany = true
                continue
            }
            let short = AssistantOutcomeGrounding.cutAtSentence(t, AssistantOutcome.maxQuestion)
            if !T.same(short, t) { add(path, .length) }
            out.append(short)
        }
        if tooMany { add("offene_fragen", .tooMany) }
        return out
    }
}
