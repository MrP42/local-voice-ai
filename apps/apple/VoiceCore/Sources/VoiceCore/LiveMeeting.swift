import Foundation

// Live-Stand einer laufenden Besprechung (Epic #247, P1). Reine Logik ohne
// Apple-Frameworks: Datenmodell, Delta-Anwendung mit Belegprüfung, Takt,
// Prompt-Bau im festen Tokenbudget und Einwilligungsnachweis.
// Ein gültiger Beleg beweist nicht die richtige Zuordnung: der Live-Stand ist
// ein prüfbarer Entwurf, jede Zeile springt zu ihrer Belegstelle.

// MARK: - Segmente und Belege

/// Ein Stück Transkript mit stabiler ID. Nur finalisierte Segmente werden ausgewertet.
public struct LiveSegment: Codable, Equatable, Sendable {
    public let id: String
    public let start: Double
    public let end: Double
    public var text: String
    public var speaker: String?
    public var isFinal: Bool
    public init(id: String, start: Double, end: Double, text: String, speaker: String? = nil, isFinal: Bool = true) {
        self.id = id; self.start = start; self.end = end; self.text = text; self.speaker = speaker; self.isFinal = isFinal
    }
}

/// Beleg mit Transkriptversion, Segment-ID und Audiozeitbereich. Der Zeitbereich
/// erlaubt, den Beleg nach einem Endlauf mit anderen Segmentgrenzen neu zuzuordnen.
public struct LiveEvidence: Codable, Hashable, Sendable {
    public let transcriptVersion: Int
    public let segmentId: String
    public let start: Double
    public let end: Double
    public init(transcriptVersion: Int, segmentId: String, start: Double, end: Double) {
        self.transcriptVersion = transcriptVersion; self.segmentId = segmentId; self.start = start; self.end = end
    }
}

// MARK: - Zustand

public struct LiveItem: Codable, Equatable, Identifiable, Sendable {
    public enum Kind: String, Codable, CaseIterable, Sendable { case topic, decision, todo, open, suggestion }
    /// `answerProposed`: das Modell meint, die Frage sei beantwortet. Erst ein Tipp setzt `done`.
    public enum Status: String, Codable, Sendable { case active, answerProposed, done }
    public let id: String
    public let kind: Kind
    public var text: String
    public var owner: String?
    public var due: String?
    public var status: Status
    public var evidence: [LiveEvidence]
    /// Vorschläge der KI sind als solche gekennzeichnet (nie Tatsache).
    public var isAISuggestion: Bool { kind == .suggestion }
    public var answerEvidence: [LiveEvidence]
}

public struct LiveState: Codable, Equatable, Sendable {
    public var summary: String = ""
    public var items: [LiveItem] = []
    public var transcriptVersion: Int = 1
    /// Audioposition, bis zu der finalisierte Segmente ausgewertet sind.
    public var processedThrough: Double = 0
    public var tick: Int = 0
    /// Verworfene Einträge (fehlender Beleg, unbekannte ID …), zur Anzeige und Auswertung.
    public var rejected: Int = 0
    public init() {}

    public func items(_ kind: LiveItem.Kind) -> [LiveItem] { items.filter { $0.kind == kind } }
}

// MARK: - Delta (Ausgabe des Modells)

/// Das Modell schreibt nie den ganzen Stand neu, sondern liefert Änderungen.
/// Belege sind Segment-IDs aus der Eingabe.
public struct LiveDelta: Codable, Equatable, Sendable {
    public struct Add: Codable, Equatable, Sendable {
        public var kind: LiveItem.Kind; public var text: String
        public var owner: String?; public var due: String?; public var evidence: [String]
        public init(kind: LiveItem.Kind, text: String, owner: String? = nil, due: String? = nil, evidence: [String]) {
            self.kind = kind; self.text = text; self.owner = owner; self.due = due; self.evidence = evidence
        }
    }
    public struct Update: Codable, Equatable, Sendable {
        public var id: String; public var text: String?; public var owner: String?; public var due: String?; public var evidence: [String]
        public init(id: String, text: String? = nil, owner: String? = nil, due: String? = nil, evidence: [String]) {
            self.id = id; self.text = text; self.owner = owner; self.due = due; self.evidence = evidence
        }
    }
    public struct Answered: Codable, Equatable, Sendable {
        public var id: String; public var evidence: [String]
        public init(id: String, evidence: [String]) { self.id = id; self.evidence = evidence }
    }
    public var summary: String?
    public var add: [Add]
    public var update: [Update]
    public var answered: [Answered]
    public init(summary: String? = nil, add: [Add] = [], update: [Update] = [], answered: [Answered] = []) {
        self.summary = summary; self.add = add; self.update = update; self.answered = answered
    }
}

public enum LiveRejection: Equatable, Sendable {
    case missingEvidence(String), unknownEvidence(String), unattributed(String), unknownItem(String), closedItem(String), tooLong(String), full
}

public struct LiveApplyReport: Equatable, Sendable {
    public var added: [String] = []
    public var updated: [String] = []
    public var proposedAnswers: [String] = []
    public var rejections: [LiveRejection] = []
}

public enum LiveStandLimits {
    public static let maxItems = 200
    public static let maxText = 300
    public static let maxSummary = 800
}

extension LiveState {
    /// Delta anwenden. `segments`: alle Segmente, die dem Modell in diesem Takt
    /// gezeigt wurden (neue und gezielt nachgeladene ältere). Unbelegte Einträge
    /// werden verworfen und gezählt, der Rest wird übernommen.
    public mutating func apply(_ delta: LiveDelta, segments: [LiveSegment]) -> LiveApplyReport {
        var report = LiveApplyReport()
        let byId = Dictionary(segments.filter(\.isFinal).map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        tick += 1

        func refs(_ ids: [String], _ label: String, required: Bool) -> [LiveEvidence]? {
            var seen = Set<String>(), out: [LiveEvidence] = []
            for id in ids where seen.insert(id).inserted {
                guard let s = byId[id] else { report.rejections.append(.unknownEvidence(label)); return nil }
                out.append(LiveEvidence(transcriptVersion: transcriptVersion, segmentId: s.id, start: s.start, end: s.end))
            }
            if required && out.isEmpty { report.rejections.append(.missingEvidence(label)); return nil }
            return out
        }
        func source(_ ev: [LiveEvidence]) -> String { ev.compactMap { byId[$0.segmentId].map { ($0.speaker ?? "") + " " + $0.text } }.joined(separator: " ") }
        /// Namen und Termine nur, wenn sie im Beleg wörtlich vorkommen.
        func attributed(_ value: String?, _ ev: [LiveEvidence]) -> Bool {
            guard let value, !Self.norm(value).isEmpty else { return true }
            return Self.norm(source(ev)).contains(Self.norm(value))
        }
        func clean(_ s: String?) -> String? {
            guard let t = s?.trimmingCharacters(in: .whitespacesAndNewlines), !t.isEmpty else { return nil }
            return t
        }

        for add in delta.add {
            let text = add.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty, text.count <= LiveStandLimits.maxText else { report.rejections.append(.tooLong(add.text)); continue }
            // Vorschläge dürfen ohne Beleg kommen; alles andere braucht einen.
            guard let ev = refs(add.evidence, text, required: add.kind != .suggestion) else { continue }
            let owner = clean(add.owner), due = clean(add.due)
            guard attributed(owner, ev), attributed(due, ev) else { report.rejections.append(.unattributed(text)); continue }
            if let i = items.firstIndex(where: { $0.kind == add.kind && $0.status != .done && Self.norm($0.text) == Self.norm(text) }) {
                // Doppelt genannt: Belege zusammenführen statt neuer Zeile.
                items[i].evidence = Self.merge(items[i].evidence, ev)
                if items[i].owner == nil { items[i].owner = owner }
                if items[i].due == nil { items[i].due = due }
                report.updated.append(items[i].id); continue
            }
            guard items.count < LiveStandLimits.maxItems else { report.rejections.append(.full); continue }
            let id = "l\(tick)-\(report.added.count + 1)"
            items.append(LiveItem(id: id, kind: add.kind, text: text, owner: owner, due: due, status: .active, evidence: ev, answerEvidence: []))
            report.added.append(id)
        }

        for up in delta.update {
            guard let i = items.firstIndex(where: { $0.id == up.id }) else { report.rejections.append(.unknownItem(up.id)); continue }
            guard items[i].status != .done else { report.rejections.append(.closedItem(up.id)); continue }
            guard let ev = refs(up.evidence, up.id, required: true) else { continue }
            let owner = clean(up.owner), due = clean(up.due)
            guard attributed(owner, ev), attributed(due, ev) else { report.rejections.append(.unattributed(up.id)); continue }
            if let t = clean(up.text) {
                guard t.count <= LiveStandLimits.maxText else { report.rejections.append(.tooLong(up.id)); continue }
                items[i].text = t
            }
            if let owner { items[i].owner = owner }
            if let due { items[i].due = due }
            items[i].evidence = Self.merge(items[i].evidence, ev)
            report.updated.append(up.id)
        }

        for ans in delta.answered {
            guard let i = items.firstIndex(where: { $0.id == ans.id }), items[i].kind == .open else { report.rejections.append(.unknownItem(ans.id)); continue }
            guard items[i].status == .active else { continue }
            guard let ev = refs(ans.evidence, ans.id, required: true) else { continue }
            items[i].status = .answerProposed
            items[i].answerEvidence = ev
            report.proposedAnswers.append(ans.id)
        }

        if let s = clean(delta.summary) { summary = String(s.prefix(LiveStandLimits.maxSummary)) }
        if let last = segments.filter(\.isFinal).map(\.end).max(), last > processedThrough { processedThrough = last }
        rejected += report.rejections.count
        return report
    }

    /// Nutzer bestätigt „beantwortet“ (oder hakt ein To-do ab).
    public mutating func confirmDone(_ id: String) {
        guard let i = items.firstIndex(where: { $0.id == id }) else { return }
        items[i].status = .done
    }

    /// Nutzer verwirft den Vorschlag „beantwortet“: Frage bleibt offen.
    public mutating func rejectAnswer(_ id: String) {
        guard let i = items.firstIndex(where: { $0.id == id }), items[i].status == .answerProposed else { return }
        items[i].status = .active; items[i].answerEvidence = []
    }

    static func merge(_ a: [LiveEvidence], _ b: [LiveEvidence]) -> [LiveEvidence] {
        var seen = Set(a.map(\.segmentId)); return a + b.filter { seen.insert($0.segmentId).inserted }
    }

    static func norm(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de"))
            .components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty }.joined(separator: " ")
    }
}

// MARK: - Takt

public enum LiveThermal: Int, Codable, Sendable, Comparable {
    case nominal, fair, serious, critical
    public static func < (a: Self, b: Self) -> Bool { a.rawValue < b.rawValue }
}

/// Wann der nächste Takt läuft. Nie parallel zu sich selbst.
public struct LiveTickPolicy: Equatable, Sendable {
    public var interval: Double = 90
    public var words = 250
    public var throttledInterval: Double = 300
    public init() {}

    public enum Decision: Equatable, Sendable { case run, wait, paused(String) }

    public func decide(now: Double, lastRun: Double?, newWords: Int, running: Bool,
                       thermal: LiveThermal, lowPower: Bool, foreground: Bool, aiAllowed: Bool) -> Decision {
        guard aiAllowed else { return .paused("KI-Auswertung nicht freigegeben") }
        guard thermal < .critical else { return .paused("KI pausiert (Wärme)") }
        guard foreground else { return .paused("KI pausiert, holt nach dem Entsperren auf") }
        if running || newWords == 0 { return .wait }
        let gap = (thermal >= .serious || lowPower) ? throttledInterval : interval
        guard let lastRun else { return newWords >= words / 2 ? .run : .wait }
        if now - lastRun >= gap { return .run }
        if gap == interval && newWords >= words { return .run }
        return .wait
    }
}

// MARK: - Prompt im festen Tokenbudget

/// Budget je Takt (Foundation Model: 4096 Token), vollständig gerechnet.
public struct LiveTokenBudget: Equatable, Sendable {
    public var context = 4096
    public var instructions = 450
    public var schema = 350
    public var state = 900
    public var newSegments = 1100
    public var olderEvidence = 300
    public var output = 700
    public init() {}
    public var reserve: Int { context - instructions - schema - state - newSegments - olderEvidence - output }
}

public struct LivePrompt: Equatable, Sendable {
    public let instructions: String
    public let input: String
    /// Segmente, die tatsächlich im Prompt stehen (für `apply`).
    public let segments: [LiveSegment]
    /// Bis hierhin gilt der Takt als ausgewertet; spätere Segmente kommen im nächsten Takt.
    public let through: Double
    public let estimatedTokens: Int
}

public enum LivePromptBuilder {
    public static let instructions = """
    Du führst das laufende Protokoll einer Besprechung. Antworte nur mit Änderungen am Stand.
    Neue Einträge: Themen, Entscheidungen, Aufgaben (wer, bis wann), offene Punkte oder Fragen,
    Vorschläge für Fragen, die man jetzt noch ansprechen könnte. Jeder Eintrag außer Vorschlägen
    nennt die Segment-IDs als Beleg. Namen und Termine nur, wenn sie im Beleg wörtlich stehen.
    Ist eine offene Frage beantwortet, melde sie unter „answered“ mit Beleg; abschließen tut der Mensch.
    Fasse den aktuellen Stand in höchstens fünf Sätzen zusammen. Erfinde nichts.
    """

    /// Konservative Schätzung für Deutsch (ca. 3 Zeichen je Token). Am Gerät wird
    /// stattdessen die echte Zählung des Modells übergeben.
    public static func estimate(_ s: String) -> Int { (s.count + 2) / 3 }

    public static func build(state: LiveState, pending: [LiveSegment], older: [LiveSegment] = [],
                             budget: LiveTokenBudget = LiveTokenBudget(), count: (String) -> Int = estimate) -> LivePrompt {
        let fresh = pending.filter { $0.isFinal && $0.end > state.processedThrough }.sorted { $0.start < $1.start }
        func line(_ s: LiveSegment) -> String { "[\(s.id)] " + (s.speaker.map { $0 + ": " } ?? "") + s.text }

        var included: [LiveSegment] = [], used = 0
        for s in fresh {
            let t = count(line(s)) + 1
            if used + t > budget.newSegments && !included.isEmpty { break }
            included.append(s); used += t
        }
        var olderLines: [LiveSegment] = [], olderUsed = 0
        let ids = Set(included.map(\.id))
        for s in older where s.isFinal && !ids.contains(s.id) {
            let t = count(line(s)) + 1
            if olderUsed + t > budget.olderEvidence { break }
            olderLines.append(s); olderUsed += t
        }
        let stateText = compact(state, budget: budget.state, count: count)
        let input = "STAND:\n" + stateText + "\n\nFRÜHERE BELEGE:\n" + olderLines.map(line).joined(separator: "\n")
            + "\n\nNEUE SEGMENTE:\n" + included.map(line).joined(separator: "\n")
        let through = included.last?.end ?? state.processedThrough
        return LivePrompt(instructions: instructions, input: input, segments: olderLines + included, through: through,
                          estimatedTokens: count(instructions) + count(input) + budget.schema + budget.output)
    }

    /// Stand kompakt; passt er nicht, fallen erledigte Einträge, dann die ältesten Themen weg.
    static func compact(_ state: LiveState, budget: Int, count: (String) -> Int) -> String {
        var items = state.items.filter { $0.status != .done }
        func render() -> String {
            (["Zusammenfassung: " + state.summary] + items.map { i in
                "\(i.id) \(i.kind.rawValue)\(i.status == .answerProposed ? " (beantwortet?)" : ""): \(i.text)"
                    + (i.owner.map { " · wer: " + $0 } ?? "") + (i.due.map { " · bis: " + $0 } ?? "")
            }).joined(separator: "\n")
        }
        var text = render()
        while count(text) > budget, !items.isEmpty {
            if let t = items.firstIndex(where: { $0.kind == .topic }) { items.remove(at: t) } else { items.removeFirst() }
            text = render()
        }
        return text
    }
}

// MARK: - Einwilligung

/// Nachweis in den Metadaten der Aufnahme. Die Bestätigung ist eine Schranke:
/// sie dokumentiert, dass die aufnehmende Person die Zustimmung aller eingeholt hat,
/// ersetzt aber weder diese Zustimmung noch einen vollständigen Nachweis nach Art. 7 DSGVO.
public struct ConsentRecord: Codable, Equatable, Sendable {
    public enum Channel: String, Codable, Sendable { case app, watch, `import`, shortcut }
    public enum Scope: String, Codable, Sendable, CaseIterable { case recording, transcription, aiAnalysis }
    public enum Reason: String, Codable, Sendable { case start, personJoined, `import` }
    public let confirmedAt: Date
    public let channel: Channel
    public let reason: Reason
    public let noticeVersion: String
    public let scopes: [Scope]
    public let purpose: String
    public let controller: String
    public let retentionDays: Int
    /// Audioposition der Bestätigung (0 beim Start; bei „Person kommt hinzu“ die Pausenstelle).
    public let audioOffset: Double
    public init(confirmedAt: Date, channel: Channel, reason: Reason, noticeVersion: String = ConsentNotice.version,
                scopes: [Scope], purpose: String, controller: String, retentionDays: Int, audioOffset: Double = 0) {
        self.confirmedAt = confirmedAt; self.channel = channel; self.reason = reason; self.noticeVersion = noticeVersion
        self.scopes = scopes; self.purpose = purpose; self.controller = controller; self.retentionDays = retentionDays; self.audioOffset = audioOffset
    }
    public var isComplete: Bool {
        // Kein reiner Aufnahmemodus: die Live-Erkennung läuft immer mit.
        scopes.contains(.recording) && scopes.contains(.transcription) && !purpose.trimmingCharacters(in: .whitespaces).isEmpty
            && !controller.trimmingCharacters(in: .whitespaces).isEmpty && retentionDays > 0 && audioOffset.isFinite && audioOffset >= 0
    }
}

public enum ConsentNotice {
    public static let version = "2026-11-04"

    public static func text(purpose: String, controller: String, retentionDays: Int, ai: Bool) -> String {
        """
        Diese Besprechung wird aufgenommen und auf diesem Gerät transkribiert\(ai ? " und mit KI ausgewertet (Zusammenfassung, Aufgaben, offene Fragen)" : "").
        Zweck: \(purpose). Verantwortlich: \(controller).
        Die Aufnahme wird nach \(retentionDays) Tagen gelöscht, früher auf Wunsch.
        Widerruf jederzeit: Aufnahme stoppen und Dokument löschen.
        Alle Anwesenden müssen vorher zugestimmt haben. Kommt jemand hinzu, Aufnahme pausieren und erneut zustimmen lassen.
        """
    }
}

/// Alle Bestätigungen einer Aufnahme, in zeitlicher Reihenfolge.
public struct ConsentLog: Codable, Equatable, Sendable {
    public private(set) var records: [ConsentRecord] = []
    public init(records: [ConsentRecord] = []) { self.records = records }

    public enum ConsentError: Error, Equatable { case incomplete, notStart, outOfOrder, missingStart }

    public mutating func append(_ r: ConsentRecord) throws {
        guard r.isComplete else { throw ConsentError.incomplete }
        if records.isEmpty {
            guard r.reason != .personJoined else { throw ConsentError.notStart }
        } else {
            guard r.reason == .personJoined, let last = records.last, r.audioOffset >= last.audioOffset, r.confirmedAt >= last.confirmedAt else { throw ConsentError.outOfOrder }
        }
        records.append(r)
    }

    /// Darf die Aufnahme starten bzw. fortgesetzt werden?
    public var canRecord: Bool { records.first.map(\.isComplete) ?? false }

    /// Zählt jede Änderung; ein KI-Takt mit älterer Version wird verworfen.
    public var version: Int { records.count }

    /// Gilt ein Umfang ab einer Audioposition? Jede neue Bestätigung (Person kommt
    /// hinzu) legt den Umfang ab ihrer Stelle neu fest.
    public func allows(_ scope: ConsentRecord.Scope, at offset: Double = .infinity) -> Bool {
        guard let current = records.last(where: { $0.audioOffset <= offset }) else { return false }
        return current.scopes.contains(scope)
    }

    /// Gilt ein Umfang für das ganze Intervall `[from, to]`? Jede Bestätigung, die
    /// das Intervall berührt, muss ihn enthalten. Eine Bestätigung genau bei `to`
    /// gilt erst danach.
    public func allows(_ scope: ConsentRecord.Scope, from: Double, to: Double) -> Bool {
        guard from.isFinite, to.isFinite, to >= from,
              let first = records.lastIndex(where: { $0.audioOffset <= from }) else { return false }
        return records[first...].enumerated().allSatisfy { i, r in
            (i > 0 && r.audioOffset >= to) || r.scopes.contains(scope)
        }
    }
}
