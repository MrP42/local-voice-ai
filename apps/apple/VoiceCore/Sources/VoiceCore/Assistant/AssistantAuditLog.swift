import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Protokoll ohne Inhalte, als fortgeschriebenes Journal.
// Je Ereignis wird eine JSON-Zeile angehängt (`protokoll.jsonl`):
// - `start` (VOR Modellaufruf und Ausgabe, awaitet): Anfrage-Id, inhaltsfreie Sitzungskennung, Zeit, Modellart, lokal/Cloud,
//   Modus (Opt-in), Einwilligungsversion (jetzt aus der Freigabe gelesen), Kontextzustand mit Grund, Rundenart, Generation;
// - `step`: Nummer, Einordnung oder Ablehnungsgrund, Generation (je Satz, je Ansage, je verworfenem Satz);
// - `commit`: Schrittnummer, Stufe (Kern, Anzeige, Audio, Verlauf), Ergebnis, Generation;
// - `end`: Ausgang, Stufe, Bereiche (auch die von Bestandsangaben);
// - `event`: Einwilligung und Widerruf (Opt-in, Cloud).
// Eine Antwort hat damit höchstens so viele Zeilen wie Schritte (ein Lauf endet nach 200 Schritten, Ausgang „begrenzt“); die
// Spur wird nicht gekappt und nicht im Speicher gesammelt. Ein Absturz mitten in der Antwort hinterlässt `start` und die bis
// dahin angefallenen Schritte; beim Öffnen bekommt jede Anfrage ohne `end` eines mit Ausgang „unvollständig“. Beim Öffnen
// wird außerdem auf die letzten 500 Anfragen verdichtet (Temp-Datei, dann atomar umbenennen: ein Abbruch oder ein voller
// Datenträger lässt die alte Datei unversehrt) und das alte Zeilenformat (Freitext-Modell, flache Zähler) ins neue
// übertragen. `read()` faltet die Zeilen zu `AssistantAuditEntry` (alte, schon gefaltete Zeilen bleiben lesbar).
// Ein Schreiber je Verzeichnis (Registry), eine serielle Warteschlange: Zeilen kommen in der Reihenfolge des Aufrufs, nie
// gleichzeitig. Ein Lesefehler wird gemeldet und überschreibt nichts. Keine Fragen, Antworten, Titel oder Auszüge: die Typen
// haben dafür gar kein Feld. Eigene Datei, damit „Neues Gespräch“ und „Verlauf löschen“ das Protokoll nicht mitlöschen. Dieses
// Modul loggt nichts, und es wird nie aus dem Audio-Callback gerufen.

/// Modellart als feste Kennung (kein Freitext aus der Anzeige des Modells).
public enum AssistantModelKind: String, Sendable, Codable, Equatable {
    case apple, gemma, other

    /// Ordnet eine Modellbezeichnung einer festen Art zu; die Bezeichnung selbst wird nicht gespeichert.
    public static func classify(label: String) -> AssistantModelKind {
        let l = label.lowercased()
        if l.contains("gemma") { return .gemma }
        if l.contains("apple") || l.contains("foundation") { return .apple }
        return .other
    }
}

/// Einstufung einer Antwort (ADR-0028 E4.3): S0 ist die geschlossene Liste der Vorlagen (Gruß, Bestätigung, feste Rückfrage,
/// Uhrzeit/Datum), alles andere ist mindestens S1. Eine Antwort mit Sätzen ohne Quelle ist nie S0.
public enum AssistantAuditLevel: String, Sendable, Codable, Equatable, Comparable {
    case s0, s1

    public static func < (a: Self, b: Self) -> Bool { a == .s0 && b == .s1 }

    public static func of(_ counts: AssistantSentenceCounts) -> AssistantAuditLevel {
        counts.grounded + counts.inventory + counts.unsourced > 0 ? .s1 : .s0
    }
}

/// Ausgabe-Commits, in der Reihenfolge ihres Auftretens je Satz: Ereignis aus dem Kern, Anzeige, Audiostart; dazu der Verlauf.
public enum AssistantCommitStage: String, Sendable, Codable, Equatable {
    case emit, display, audio, history

    var code: Character {
        switch self {
        case .emit: "e"
        case .display: "d"
        case .audio: "a"
        case .history: "h"
        }
    }

    init?(code: Character) {
        switch code {
        case "e": self = .emit
        case "d": self = .display
        case "a": self = .audio
        case "h": self = .history
        default: return nil
        }
    }
}

/// Ergebnis eines Ausgabe-Commits mit der dabei geprüften Generation. Im Protokoll als kurze feste Kennung („d+7“ = Anzeige
/// ausgeführt bei Generation 7, „a-8“ = Audiostart verworfen bei Generation 8).
public struct AssistantCommitMark: Sendable, Equatable, Codable {
    public let stage: AssistantCommitStage
    public let ok: Bool
    public let generation: Int

    public init(stage: AssistantCommitStage, ok: Bool, generation: Int) {
        self.stage = stage
        self.ok = ok
        self.generation = generation
    }

    public init(from decoder: Decoder) throws {
        let text = try decoder.singleValueContainer().decode(String.self)
        guard let first = text.first, let stage = AssistantCommitStage(code: first), text.count >= 3,
              let sign = text.dropFirst().first, sign == "+" || sign == "-", let g = Int(text.dropFirst(2)) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Commit-Kennung"))
        }
        self.init(stage: stage, ok: sign == "+", generation: g)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode("\(stage.code)\(ok ? "+" : "-")\(generation)")
    }
}

/// Einordnung eines Satzes in der Satzspur: die fünf Klassen (Originalsatz, Bestandsangabe, Satz ohne Quelle, Vorlage,
/// verworfen mit Grund) und die Ansage.
public enum AssistantTraceVerdict: String, Sendable, Codable, Equatable {
    case grounded, inventory, unsourced, template, announcement
    case droppedOwnData, droppedNoMode, droppedNotSnapped, droppedDuplicate, droppedTemplate, droppedRevoked, droppedFragment

    public init(status: AssistantSentenceStatus) {
        switch status {
        case .grounded: self = .grounded
        case .inventory: self = .inventory
        case .unsourced: self = .unsourced
        case .neutral: self = .template
        case .announcement: self = .announcement
        }
    }

    public init(dropped reason: AssistantDropReason) {
        switch reason {
        case .ownData: self = .droppedOwnData
        case .noMode: self = .droppedNoMode
        case .notSnapped: self = .droppedNotSnapped
        case .duplicate: self = .droppedDuplicate
        case .template: self = .droppedTemplate
        case .revoked: self = .droppedRevoked
        case .fragment: self = .droppedFragment
        }
    }
}

/// Ein Eintrag der Satzspur. Kein Textfeld.
public struct AssistantTraceStep: Sendable, Equatable, Codable {
    /// Laufende Nummer der Einordnung (ab 1).
    public var n: Int
    public var verdict: AssistantTraceVerdict
    /// Freigabe-Generation bei der Einordnung.
    public var generation: Int
    public var commits: [AssistantCommitMark]

    public init(n: Int, verdict: AssistantTraceVerdict, generation: Int, commits: [AssistantCommitMark] = []) {
        self.n = n
        self.verdict = verdict
        self.generation = generation
        self.commits = commits
    }

    private enum CodingKeys: String, CodingKey {
        case n
        case verdict = "v"
        case generation = "g"
        case commits = "c"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        n = try c.decode(Int.self, forKey: .n)
        verdict = try c.decode(AssistantTraceVerdict.self, forKey: .verdict)
        generation = try c.decode(Int.self, forKey: .generation)
        commits = try c.decodeIfPresent([AssistantCommitMark].self, forKey: .commits) ?? []
    }
}

public struct AssistantAuditEntry: Sendable, Equatable, Codable {
    public enum Kind: String, Sendable, Codable {
        /// `resumedFree`: der Nutzer hat mit „Ohne Quelle fortsetzen“ den Datenkontext aufgehoben.
        case answer, unsourcedOn, unsourcedOff, cloudGranted, cloudRevoked, resumedFree
    }

    public enum Outcome: String, Sendable, Codable {
        /// Zu Ende gesprochen.
        case completed
        /// Vom Nutzer unterbrochen oder abgebrochen.
        case interrupted
        /// Wegen geänderter Freigabe (Opt-in, Bereich, Cloud-Zustimmung) abgebrochen oder verworfen.
        case revoked
        /// Modell- oder Verbindungsfehler.
        case failed
        /// Ohne Abschluss geblieben (Absturz oder Prozessende mitten in der Antwort): beim Öffnen nachgetragen.
        case incomplete
        /// Nach 200 Schritten beendet.
        case limited
    }

    public var at: Date
    public var kind: Kind
    /// Art des Modells (feste Kennung), kein Freitext.
    public var modelKind: AssistantModelKind
    public var cloud: Bool
    /// „Antworten ohne Quelle“ war bei Beginn der Antwort an.
    public var unsourcedMode: Bool
    public var consentVersion: Int
    public var round: AssistantRoundKind?
    public var level: AssistantAuditLevel?
    /// Anfrage-Id und inhaltsfreie Sitzungskennung (nur Antworten aus dem Journal).
    public var requestId: String?
    public var sessionId: String?
    /// Kontextzustand vor der Rundenwahl und der Grund der Wahl.
    public var contextMode: AssistantContext.Mode?
    public var contextReason: AssistantContext.Reason?
    /// Freigabe-Generation beim Start der Anfrage; bei Einwilligung und Widerruf die neue.
    public var generation: Int
    /// Zahl der Sätze je Einordnung und Ablehnungsgrund.
    public var counts: AssistantSentenceCounts
    /// Bereiche der Quellen (Rohwerte `books`, `transcripts`, `files`, `notes`), sortiert.
    public var areas: [String]
    public var outcome: Outcome?
    public var trace: [AssistantTraceStep]
    /// Nur alte Zeilen (vor dem Journal) kannten eine Kappung der Spur.
    public var traceOverflow: Int
    /// Verlaufs-Commit der Antwort (nil: kein Beitrag vorgesehen).
    public var history: AssistantCommitMark?

    public init(at: Date = Date(), kind: Kind, model: String = "", cloud: Bool = false, unsourcedMode: Bool = false,
                consentVersion: Int = 0, counts: AssistantSentenceCounts = AssistantSentenceCounts(), areas: [AssistantArea] = [],
                outcome: Outcome? = nil, round: AssistantRoundKind? = nil, level: AssistantAuditLevel? = nil, generation: Int = 0,
                trace: [AssistantTraceStep] = [], traceOverflow: Int = 0, history: AssistantCommitMark? = nil) {
        self.at = at
        self.kind = kind
        self.modelKind = AssistantModelKind.classify(label: model)
        self.cloud = cloud
        self.unsourcedMode = unsourcedMode
        self.consentVersion = consentVersion
        self.round = round
        self.generation = generation
        self.counts = counts
        self.areas = Set(areas.map(\.rawValue)).sorted()
        self.outcome = outcome
        self.trace = trace
        self.traceOverflow = traceOverflow
        self.history = history
        if kind == .answer {
            // Mit Sätzen ohne Quelle mindestens S1, nie S0.
            let derived = AssistantAuditLevel.of(counts)
            var l = level ?? derived
            if counts.unsourced > 0, l < .s1 { l = .s1 }
            self.level = max(l, derived)
        } else {
            self.level = level
        }
    }

    /// Alte Zeilen (vor Paket A, Runde E4.2a): `model` (Freitext, wird nur zur Art), flache Zähler `grounded`, `unsourced`,
    /// `neutral`, `dropped`. Neue Felder fehlen dort und bekommen Vorgabewerte.
    private enum CodingKeys: String, CodingKey {
        case at, kind, modelKind, cloud, unsourcedMode, consentVersion, round, level, generation, counts, areas, outcome, trace
        case traceOverflow, history, requestId, sessionId, contextMode, contextReason
        case model, grounded, unsourced, neutral, dropped
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        at = try c.decode(Date.self, forKey: .at)
        kind = try c.decode(Kind.self, forKey: .kind)
        if let k = try c.decodeIfPresent(AssistantModelKind.self, forKey: .modelKind) {
            modelKind = k
        } else {
            modelKind = AssistantModelKind.classify(label: try c.decodeIfPresent(String.self, forKey: .model) ?? "")
        }
        cloud = try c.decodeIfPresent(Bool.self, forKey: .cloud) ?? false
        unsourcedMode = try c.decodeIfPresent(Bool.self, forKey: .unsourcedMode) ?? false
        consentVersion = try c.decodeIfPresent(Int.self, forKey: .consentVersion) ?? 0
        round = try c.decodeIfPresent(AssistantRoundKind.self, forKey: .round)
        generation = try c.decodeIfPresent(Int.self, forKey: .generation) ?? 0
        if let counts = try c.decodeIfPresent(AssistantSentenceCounts.self, forKey: .counts) {
            self.counts = counts
        } else {
            var legacy = AssistantSentenceCounts()
            legacy.grounded = try c.decodeIfPresent(Int.self, forKey: .grounded) ?? 0
            legacy.unsourced = try c.decodeIfPresent(Int.self, forKey: .unsourced) ?? 0
            legacy.neutral = try c.decodeIfPresent(Int.self, forKey: .neutral) ?? 0
            legacy.droppedNotSnapped = try c.decodeIfPresent(Int.self, forKey: .dropped) ?? 0
            counts = legacy
        }
        areas = try c.decodeIfPresent([String].self, forKey: .areas) ?? []
        outcome = try c.decodeIfPresent(Outcome.self, forKey: .outcome)
        trace = try c.decodeIfPresent([AssistantTraceStep].self, forKey: .trace) ?? []
        traceOverflow = try c.decodeIfPresent(Int.self, forKey: .traceOverflow) ?? 0
        history = try c.decodeIfPresent(AssistantCommitMark.self, forKey: .history)
        requestId = try c.decodeIfPresent(String.self, forKey: .requestId)
        sessionId = try c.decodeIfPresent(String.self, forKey: .sessionId)
        contextMode = try c.decodeIfPresent(AssistantContext.Mode.self, forKey: .contextMode)
        contextReason = try c.decodeIfPresent(AssistantContext.Reason.self, forKey: .contextReason)
        let derived = AssistantAuditLevel.of(counts)
        let stored = try c.decodeIfPresent(AssistantAuditLevel.self, forKey: .level)
        level = kind == .answer ? max(stored ?? derived, derived) : stored
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(at, forKey: .at)
        try c.encode(kind, forKey: .kind)
        try c.encode(modelKind, forKey: .modelKind)
        try c.encode(cloud, forKey: .cloud)
        try c.encode(unsourcedMode, forKey: .unsourcedMode)
        try c.encode(consentVersion, forKey: .consentVersion)
        try c.encodeIfPresent(round, forKey: .round)
        try c.encodeIfPresent(level, forKey: .level)
        try c.encode(generation, forKey: .generation)
        try c.encode(counts, forKey: .counts)
        try c.encode(areas, forKey: .areas)
        try c.encodeIfPresent(outcome, forKey: .outcome)
        if !trace.isEmpty { try c.encode(trace, forKey: .trace) }
        if traceOverflow > 0 { try c.encode(traceOverflow, forKey: .traceOverflow) }
        try c.encodeIfPresent(history, forKey: .history)
        try c.encodeIfPresent(requestId, forKey: .requestId)
        try c.encodeIfPresent(sessionId, forKey: .sessionId)
        try c.encodeIfPresent(contextMode, forKey: .contextMode)
        try c.encodeIfPresent(contextReason, forKey: .contextReason)
    }
}


public enum AssistantAuditError: Error, Sendable, Equatable {
    /// Die vorhandene Datei war nicht lesbar: nichts wurde geschrieben, die Datei bleibt, wie sie ist.
    case readFailed
    case encodeFailed
    case writeFailed
}

/// Ergebnis des Lesens: lesbare Einträge, Zahl der übersprungenen (beschädigten) Zeilen, Lesefehler.
public struct AssistantAuditRead: Sendable, Equatable {
    public var entries: [AssistantAuditEntry]
    public var skippedLines: Int
    public var error: AssistantAuditError?
}

/// Eine Zeile des Journals (`t` = Art). Felder, die nicht gelten, fehlen. Kein Textfeld.
struct AssistantJournalLine: Codable, Equatable {
    enum Kind: String, Codable, Equatable {
        case start, step, commit, end, event
    }

    var t: Kind
    /// Anfrage-Id (nicht bei `event`).
    var req: String?
    var at: Date?
    /// Inhaltsfreie Sitzungskennung (`start`).
    var session: String?
    var model: AssistantModelKind?
    var cloud: Bool?
    /// Modus: Opt-in „Antworten ohne Quelle“ war an.
    var unsourced: Bool?
    var consent: Int?
    var ctx: AssistantContext.Mode?
    var reason: AssistantContext.Reason?
    var round: AssistantRoundKind?
    var gen: Int?
    /// Schrittnummer (`step`, `commit`; 0 = der Verlaufs-Commit der Antwort).
    var n: Int?
    var verdict: AssistantTraceVerdict?
    var mark: AssistantCommitMark?
    var outcome: AssistantAuditEntry.Outcome?
    var level: AssistantAuditLevel?
    var areas: [String]?
    /// Art des Ereignisses (`event`).
    var kind: AssistantAuditEntry.Kind?

    init(_ t: Kind, req: String? = nil) {
        self.t = t
        self.req = req
    }
}

/// Faltet Journalzeilen zu Einträgen (die Ansicht für Prüfung und Tests). Zeilen ohne `t` sind alte, schon gefaltete Einträge.
enum AssistantJournalFold {
    static func decoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }

    static func journal(_ line: Data) -> AssistantJournalLine? { try? decoder().decode(AssistantJournalLine.self, from: line) }

    /// Zähler aus der Satzspur: ein Satz, dessen Ereignis nicht hinausging (`e-`), zählt als widerrufen.
    static func counts(from trace: [AssistantTraceStep]) -> AssistantSentenceCounts {
        var c = AssistantSentenceCounts()
        for step in trace {
            let delivered = !step.commits.contains { $0.stage == .emit && !$0.ok }
            switch step.verdict {
            case .announcement: continue
            case .droppedOwnData: c.droppedOwnData += 1
            case .droppedNoMode: c.droppedNoMode += 1
            case .droppedNotSnapped: c.droppedNotSnapped += 1
            case .droppedDuplicate: c.droppedDuplicate += 1
            case .droppedTemplate: c.droppedTemplate += 1
            case .droppedRevoked: c.revoked += 1
            case .droppedFragment: c.droppedFragment += 1
            case .grounded: if delivered { c.grounded += 1 } else { c.revoked += 1 }
            case .inventory: if delivered { c.inventory += 1 } else { c.revoked += 1 }
            case .unsourced: if delivered { c.unsourced += 1 } else { c.revoked += 1 }
            case .template: if delivered { c.neutral += 1 } else { c.revoked += 1 }
            }
        }
        return c
    }

    static func fold(_ lines: [Data]) -> (entries: [AssistantAuditEntry], skipped: Int) {
        let plain = decoder()
        var entries: [AssistantAuditEntry] = []
        var byRequest: [String: Int] = [:]
        var endLevel: [Int: AssistantAuditLevel] = [:]
        var journalEntries: [Int] = []
        var skipped = 0
        for data in lines {
            guard let l = journal(data) else {
                if let e = try? plain.decode(AssistantAuditEntry.self, from: data) { entries.append(e) } else { skipped += 1 }
                continue
            }
            switch l.t {
            case .start:
                guard let req = l.req else { skipped += 1; continue }
                var e = AssistantAuditEntry(at: l.at ?? .distantPast, kind: .answer, cloud: l.cloud ?? false,
                                            unsourcedMode: l.unsourced ?? false, consentVersion: l.consent ?? 0, round: l.round,
                                            generation: l.gen ?? 0)
                e.modelKind = l.model ?? .other
                e.requestId = req
                e.sessionId = l.session
                e.contextMode = l.ctx
                e.contextReason = l.reason
                entries.append(e)
                byRequest[req] = entries.count - 1
                journalEntries.append(entries.count - 1)
            case .step:
                guard let req = l.req, let i = byRequest[req], let n = l.n, let v = l.verdict else { skipped += 1; continue }
                entries[i].trace.append(AssistantTraceStep(n: n, verdict: v, generation: l.gen ?? 0))
            case .commit:
                guard let req = l.req, let i = byRequest[req], let n = l.n, let mark = l.mark else { skipped += 1; continue }
                if n == 0 {
                    entries[i].history = mark
                } else if let k = entries[i].trace.firstIndex(where: { $0.n == n }) {
                    entries[i].trace[k].commits.append(mark)
                } else {
                    skipped += 1
                }
            case .end:
                guard let req = l.req, let i = byRequest[req] else { skipped += 1; continue }
                entries[i].outcome = l.outcome
                entries[i].areas = l.areas ?? []
                if let level = l.level { endLevel[i] = level }
            case .event:
                var e = AssistantAuditEntry(at: l.at ?? .distantPast, kind: l.kind ?? .unsourcedOn, cloud: l.cloud ?? false,
                                            unsourcedMode: l.unsourced ?? false, consentVersion: l.consent ?? 0, generation: l.gen ?? 0)
                e.modelKind = l.model ?? .other
                entries.append(e)
            }
        }
        for i in journalEntries {
            entries[i].counts = counts(from: entries[i].trace)
            if entries[i].outcome == nil { entries[i].outcome = .incomplete }
            entries[i].level = max(AssistantAuditLevel.of(entries[i].counts), endLevel[i] ?? .s0)
        }
        return (entries, skipped)
    }
}

/// Der eine Schreiber eines Verzeichnisses. Alle Handles auf dasselbe Verzeichnis teilen ihn. Alle Zugriffe auf die Datei laufen
/// auf einer seriellen Warteschlange: `enqueue` kehrt sofort zurück, die Zeilen werden in dieser Reihenfolge angehängt.
final class AssistantAuditWriter: @unchecked Sendable {
    let directory: URL
    var url: URL { directory.appendingPathComponent(AssistantAuditLog.fileName) }

    private let reader: @Sendable (URL) throws -> Data?
    private let appender: @Sendable (URL, Data) throws -> Void
    private let rewriter: @Sendable (URL, Data) throws -> Void
    private let queue = DispatchQueue(label: "de.wolffappliedai.localvoice.assistant.journal")

    // Nur auf `queue`:
    private var opened = false
    /// Das Ende der Datei ist unsicher (letzter Schreibvorgang abgebrochen): die nächste Zeile beginnt mit einem Zeilenumbruch.
    private var torn = false
    private var openError: AssistantAuditError = .readFailed
    private var errors: [String: AssistantAuditError] = [:]

    /// Anfragen, über die verdichtet wird (die letzten).
    static let maxUnits = AssistantAuditLog.maxEntries

    init(directory: URL, reader: @escaping @Sendable (URL) throws -> Data? = AssistantAuditWriter.readFile,
         appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile,
         rewriter: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.rewriteFile) {
        self.directory = directory
        self.reader = reader
        self.appender = appender
        self.rewriter = rewriter
    }

    // MARK: Dateizugriff

    /// `nil`: die Datei gibt es (noch) nicht. Jeder andere Fehler wird geworfen.
    static func readFile(_ url: URL) throws -> Data? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try Data(contentsOf: url)
    }

    /// Hängt Bytes an (legt Verzeichnis und Datei bei Bedarf an). Ein Fehler lässt Bestehendes unberührt.
    static func appendFile(_ url: URL, _ data: Data) throws {
        let fm = FileManager.default
        if !fm.fileExists(atPath: url.path) {
            let dir = url.deletingLastPathComponent()
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            BookTalkHistoryStore.excludeFromBackup(dir)
            #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
            let created = fm.createFile(atPath: url.path, contents: nil,
                                        attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
            #else
            let created = fm.createFile(atPath: url.path, contents: nil)
            #endif
            guard created else { throw AssistantAuditError.writeFailed }
            BookTalkHistoryStore.excludeFromBackup(url)
        }
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.seekToEnd()
        try handle.write(contentsOf: data)
    }

    /// Schreibt die ganze Datei neu: Temp-Datei, dann atomar umbenennen. Ein Abbruch oder ein voller Datenträger lässt die alte
    /// Datei unversehrt.
    static func rewriteFile(_ url: URL, _ data: Data) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try data.write(to: url, options: .atomic)
        #endif
        BookTalkHistoryStore.excludeFromBackup(url)
    }

    private static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return e
    }

    private static func splitLines(_ data: Data) -> [Data] {
        data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true).map { Data($0) }
    }

    /// Zeilen im alten Format (Freitext `model`, flache Zähler) gehen beim Verdichten ins neue über: Art statt Freitext, Zähler in
    /// `counts`. Alles andere, auch Unlesbares oder von einer neueren Fassung, bleibt Byte für Byte.
    static func migrated(_ line: Data) -> Data {
        guard let object = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any], object["t"] == nil,
              object["model"] != nil || object["grounded"] != nil || object["dropped"] != nil,
              let entry = try? AssistantJournalFold.decoder().decode(AssistantAuditEntry.self, from: line),
              let data = try? encoder().encode(entry) else { return line }
        return data
    }

    // MARK: Schreiben

    /// Hängt eine Zeile an (asynchron, in der Reihenfolge der Aufrufe). Kehrt sofort zurück; ein Fehler wird für die Anfrage
    /// gemerkt und von `flush` gemeldet.
    func enqueue(_ line: AssistantJournalLine) {
        let req = line.req ?? ""
        guard let data = try? Self.encoder().encode(line) else {
            queue.async { self.note(req, .encodeFailed) }
            return
        }
        queue.async { self.write(data, req: req) }
    }

    /// Wartet, bis alles bis hierher Eingereihte geschrieben ist. Gibt den Fehler zurück, der für `request` (oder ohne Anfrage:
    /// für Ereignisse) aufgetreten ist, und vergisst ihn.
    /// `consume: false` lässt den Fehler stehen (er wird später beim Abschluss der Anfrage gemeldet).
    func flush(request: String?, consume: Bool = true) async -> AssistantAuditError? {
        await withCheckedContinuation { c in
            queue.async {
                let key = request ?? ""
                c.resume(returning: consume ? self.errors.removeValue(forKey: key) : self.errors[key])
            }
        }
    }

    private func note(_ req: String, _ error: AssistantAuditError) {
        if errors.count > 64 { errors.removeAll() }
        errors[req] = error
    }

    private func write(_ line: Data, req: String) {
        guard openIfNeeded() else { return note(req, openError) }
        var data = Data()
        if torn { data.append(10) }
        data.append(line)
        data.append(10)
        do {
            try appender(url, data)
            torn = false
        } catch {
            torn = true
            note(req, .writeFailed)
        }
    }

    /// Einmal je Schreiber vor der ersten Zeile: offene Anfragen schließen, verdichten, altes Format übertragen. Ein Lesefehler
    /// ändert nichts und wird beim nächsten Mal erneut versucht.
    private func openIfNeeded() -> Bool {
        if opened { return true }
        let data: Data?
        do { data = try reader(url) } catch {
            openError = .readFailed
            return false
        }
        guard let data, !data.isEmpty else {
            opened = true
            return true
        }
        torn = data.last != UInt8(ascii: "\n")
        var lines = Self.splitLines(data)

        var keys: [String] = []
        var started: [String] = []
        var ended = Set<String>()
        for (i, l) in lines.enumerated() {
            if let j = AssistantJournalFold.journal(l), let req = j.req, j.t != .event {
                keys.append("r:" + req)
                if j.t == .start, !started.contains(req) { started.append(req) }
                if j.t == .end { ended.insert(req) }
            } else {
                keys.append("l:\(i)")
            }
        }
        let closing: [Data] = started.filter { !ended.contains($0) }.compactMap { req in
            var end = AssistantJournalLine(.end, req: req)
            end.at = Date()
            end.outcome = .incomplete
            return try? Self.encoder().encode(end)
        }

        var order: [String] = []
        var seen = Set<String>()
        for k in keys where seen.insert(k).inserted { order.append(k) }
        let drop = Set(order.prefix(max(0, order.count - Self.maxUnits)))
        let migrated = lines.map(Self.migrated)
        let changedFormat = zip(lines, migrated).contains { $0 != $1 }
        if !drop.isEmpty || changedFormat {
            lines = zip(migrated, keys).filter { !drop.contains($0.1) }.map(\.0) + closing
            var out = Data()
            for l in lines {
                out.append(l)
                out.append(10)
            }
            do {
                try rewriter(url, out)
                torn = false
                opened = true
                return true
            } catch {
                // Die alte Datei ist unversehrt; ohne Verdichtung weiterschreiben, beim nächsten Öffnen erneut versuchen.
            }
        }
        for l in closing {
            var bytes = Data()
            if torn { bytes.append(10) }
            bytes.append(l)
            bytes.append(10)
            do {
                try appender(url, bytes)
                torn = false
            } catch {
                torn = true
                openError = .writeFailed
                return false
            }
        }
        opened = true
        return true
    }

    // MARK: Lesen und Löschen

    func read() async -> AssistantAuditRead {
        await withCheckedContinuation { c in queue.async { c.resume(returning: self.readNow()) } }
    }

    private func readNow() -> AssistantAuditRead {
        let data: Data?
        do { data = try reader(url) } catch { return AssistantAuditRead(entries: [], skippedLines: 0, error: .readFailed) }
        guard let data else { return AssistantAuditRead(entries: [], skippedLines: 0, error: nil) }
        let folded = AssistantJournalFold.fold(Self.splitLines(data))
        return AssistantAuditRead(entries: folded.entries, skippedLines: folded.skipped, error: nil)
    }

    func clear() async {
        await withCheckedContinuation { c in
            queue.async {
                try? FileManager.default.removeItem(at: self.url)
                self.torn = false
                c.resume()
            }
        }
    }
}

/// Ein Schreiber je Verzeichnis, prozessweit.
final class AssistantAuditRegistry: @unchecked Sendable {
    static let shared = AssistantAuditRegistry()
    private let lock = NSLock()
    private var writers: [String: AssistantAuditWriter] = [:]

    static func key(_ directory: URL) -> String { directory.standardizedFileURL.resolvingSymlinksInPath().path }

    static func writer(for directory: URL) -> AssistantAuditWriter { shared.writer(for: directory) }

    private func writer(for directory: URL) -> AssistantAuditWriter {
        lock.lock()
        defer { lock.unlock() }
        let k = Self.key(directory)
        if let w = writers[k] { return w }
        let w = AssistantAuditWriter(directory: directory)
        writers[k] = w
        return w
    }
}

/// Handle auf das Protokoll eines Verzeichnisses. Beliebig viele Handles teilen einen Schreiber (Registry).
public struct AssistantAuditLog: Sendable {
    /// Anfragen (und Ereignisse), auf die beim Öffnen verdichtet wird.
    public static let maxEntries = 500
    public static let fileName = "protokoll.jsonl"
    /// Nach so vielen Schritten endet eine Antwort (Ausgang „begrenzt“).
    public static let maxSteps = 200

    public let directory: URL
    private let writer: AssistantAuditWriter

    public init(directory: URL) {
        self.directory = directory
        self.writer = AssistantAuditRegistry.writer(for: directory)
    }

    /// Nur für Tests: eigener Schreiber (nicht in der Registry) mit austauschbarem Lesen und Schreiben.
    init(directory: URL, reader: @escaping @Sendable (URL) throws -> Data?,
         appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile,
         rewriter: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.rewriteFile) {
        self.directory = directory
        self.writer = AssistantAuditWriter(directory: directory, reader: reader, appender: appender, rewriter: rewriter)
    }

    /// Standardort der App: neben dem Verlauf in `Application Support/Assistant`.
    public static func standardDirectory() -> URL { AssistantHistoryStore.standardDirectory() }

    public var url: URL { directory.appendingPathComponent(Self.fileName) }

    // MARK: Journal (Kern)

    /// Reiht eine Zeile ein; kehrt sofort zurück.
    func enqueue(_ line: AssistantJournalLine) { writer.enqueue(line) }

    /// Schreibt die erste Zeile einer Anfrage und wartet, bis sie steht. `false` bei einem Fehler; der Fehler bleibt für den
    /// Abschluss der Anfrage (`flush`) gemerkt, damit er nie verloren geht.
    func begin(_ line: AssistantJournalLine) async -> Bool {
        writer.enqueue(line)
        return await writer.flush(request: line.req, consume: false) == nil
    }

    /// Wartet, bis alle Zeilen der Anfrage geschrieben sind. Der Fehler dieser Anfrage, sonst `nil`.
    func flush(request: String) async -> AssistantAuditError? { await writer.flush(request: request) }

    // MARK: Ereignisse und Lesen

    /// Hängt ein Ereignis an (Einwilligung, Widerruf). Antworten schreibt der Kern als Journal (`start`, `step`, `commit`,
    /// `end`); ein Eintrag der Art `answer` wird hier nicht angenommen. Ein Fehler (Speicher voll, Datei nicht lesbar) lässt die
    /// bisherige Datei unverändert.
    public func append(_ entry: AssistantAuditEntry) async -> AssistantAuditError? {
        guard entry.kind != .answer else { return .encodeFailed }
        var line = AssistantJournalLine(.event)
        line.at = entry.at
        line.kind = entry.kind
        line.model = entry.modelKind
        line.cloud = entry.cloud
        line.unsourced = entry.unsourcedMode
        line.consent = entry.consentVersion
        line.gen = entry.generation
        writer.enqueue(line)
        return await writer.flush(request: nil)
    }

    /// Wie `append`, mit `false` bei einem Fehler.
    @discardableResult
    public func record(_ entry: AssistantAuditEntry) async -> Bool { await append(entry) == nil }

    public func read() async -> AssistantAuditRead { await writer.read() }

    /// Alle lesbaren Einträge, älteste zuerst; beschädigte Zeilen (abgebrochener Schreibvorgang) werden übersprungen.
    public func entries() async -> [AssistantAuditEntry] { await writer.read().entries }

    /// Nur auf ausdrücklichen Wunsch (z. B. Prüfung); „Neues Gespräch“ löscht das Protokoll nicht.
    public func clear() async { await writer.clear() }
}
