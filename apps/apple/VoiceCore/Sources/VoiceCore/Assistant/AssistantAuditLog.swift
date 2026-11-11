import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Protokoll ohne Inhalte.
// Je Anfrage GENAU ein Eintrag (auch bei Fehler, Abbruch oder Widerruf ohne gesprochenen Text): Zeit, Modellkennung (Art
// aus einer festen Liste plus lokal/Cloud, kein Freitext), Modus, Einwilligungsversion, Rundenart, Stufe, Generation beim
// Start, Zahl der Sätze je Einordnung und Ablehnungsgrund, Bereiche der Quellen, Ausgang, dazu die Satzspur in Reihenfolge
// (je eingeordnetem Satz: laufende Nummer, Einordnung oder Ablehnungsgrund, Generation bei der Einordnung, je Ausgabe-Commit
// Anzeige/Audiostart/Verlauf das Ergebnis mit der geprüften Generation). Je Einwilligung und Widerruf ein Eintrag mit der
// neuen Generation. Keine Fragen, Antworten, Titel oder Auszüge: die Typen haben dafür gar kein Feld.
// JSON-Zeilen (`protokoll.jsonl`) neben dem Verlauf, höchstens 500 Einträge, Schreiben atomar (Datei komplett neu, dann
// umbenennen: ein Abbruch oder ein voller Datenträger lässt die alte Datei unversehrt). Zeilen im alten Format werden dabei
// ins neue übertragen (kein Freitext-Modell mehr); das Lesen allein ändert nichts. Ein Schreiber je Verzeichnis
// (Registry), damit zwei Handles nicht gegeneinander lesen-ändern-schreiben. Ein Lesefehler wird gemeldet und überschreibt
// nichts. Eigene Datei, damit „Neues Gespräch“ und „Verlauf löschen“ das Protokoll nicht mitlöschen. Dieses Modul loggt nichts.

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
    case droppedOwnData, droppedNoMode, droppedNotSnapped, droppedDuplicate, droppedTemplate, droppedRevoked

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
        case answer, unsourcedOn, unsourcedOff, cloudGranted, cloudRevoked
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
    }

    /// Höchstens so viele Schritte der Satzspur je Eintrag (der Rest zählt in `traceOverflow`).
    public static let maxTrace = 32

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
    /// Freigabe-Generation beim Start der Anfrage; bei Einwilligung und Widerruf die neue.
    public var generation: Int
    /// Zahl der Sätze je Einordnung und Ablehnungsgrund.
    public var counts: AssistantSentenceCounts
    /// Bereiche der Quellen (Rohwerte `books`, `transcripts`, `files`, `notes`), sortiert.
    public var areas: [String]
    public var outcome: Outcome?
    public var trace: [AssistantTraceStep]
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
        self.trace = Array(trace.prefix(Self.maxTrace))
        self.traceOverflow = traceOverflow + max(0, trace.count - Self.maxTrace)
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
        case traceOverflow, history
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

/// Der eine Schreiber eines Verzeichnisses. Alle Handles auf dasselbe Verzeichnis teilen ihn.
actor AssistantAuditWriter {
    let directory: URL
    var url: URL { directory.appendingPathComponent(AssistantAuditLog.fileName) }
    private let reader: @Sendable (URL) throws -> Data?

    init(directory: URL, reader: @escaping @Sendable (URL) throws -> Data? = AssistantAuditWriter.readFile) {
        self.directory = directory
        self.reader = reader
    }

    /// `nil`: die Datei gibt es (noch) nicht. Jeder andere Fehler wird geworfen.
    static func readFile(_ url: URL) throws -> Data? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try Data(contentsOf: url)
    }

    private static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return e
    }

    private static func decoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }

    private func lines() throws -> [Data] {
        guard let data = try reader(url), !data.isEmpty else { return [] }
        return data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true).map { Data($0) }
    }

    /// Zeilen im alten Format (Freitext `model`, flache Zähler) gehen beim Schreiben ins neue über: Art statt Freitext,
    /// Zähler in `counts`. Alles andere, auch Unlesbares oder von einer neueren Fassung, bleibt Byte für Byte. Geschrieben wird
    /// danach wie immer atomar; das Lesen allein ändert nichts.
    static func migrated(_ line: Data) -> Data {
        guard let object = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any],
              object["model"] != nil || object["grounded"] != nil || object["dropped"] != nil,
              let entry = try? decoder().decode(AssistantAuditEntry.self, from: line),
              let data = try? encoder().encode(entry) else { return line }
        return data
    }

    func append(_ entry: AssistantAuditEntry) -> AssistantAuditError? {
        guard let line = try? Self.encoder().encode(entry) else { return .encodeFailed }
        var all: [Data]
        do { all = try lines() } catch { return .readFailed }
        all = all.map(Self.migrated)
        all.append(line)
        if all.count > AssistantAuditLog.maxEntries { all.removeFirst(all.count - AssistantAuditLog.maxEntries) }
        var data = Data()
        for l in all {
            data.append(l)
            data.append(UInt8(ascii: "\n"))
        }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            BookTalkHistoryStore.excludeFromBackup(directory)
            #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            #else
            try data.write(to: url, options: .atomic)
            #endif
            BookTalkHistoryStore.excludeFromBackup(url)
            return nil
        } catch {
            return .writeFailed
        }
    }

    func read() -> AssistantAuditRead {
        let decoder = Self.decoder()
        let all: [Data]
        do { all = try lines() } catch { return AssistantAuditRead(entries: [], skippedLines: 0, error: .readFailed) }
        var entries: [AssistantAuditEntry] = []
        var skipped = 0
        for l in all {
            if let e = try? decoder.decode(AssistantAuditEntry.self, from: l) { entries.append(e) } else { skipped += 1 }
        }
        return AssistantAuditRead(entries: entries, skippedLines: skipped, error: nil)
    }

    func clear() { try? FileManager.default.removeItem(at: url) }
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

/// Handle auf das Protokoll eines Verzeichnisses. Beliebig viele Handles teilen einen Schreiber (Registry), damit keine zwei
/// gleichzeitig lesen-ändern-schreiben.
public struct AssistantAuditLog: Sendable {
    public static let maxEntries = 500
    public static let fileName = "protokoll.jsonl"

    public let directory: URL
    private let writer: AssistantAuditWriter

    public init(directory: URL) {
        self.directory = directory
        self.writer = AssistantAuditRegistry.writer(for: directory)
    }

    /// Nur für Tests: eigener Schreiber (nicht in der Registry) mit austauschbarem Lesen.
    init(directory: URL, reader: @escaping @Sendable (URL) throws -> Data?) {
        self.directory = directory
        self.writer = AssistantAuditWriter(directory: directory, reader: reader)
    }

    /// Standardort der App: neben dem Verlauf in `Application Support/Assistant`.
    public static func standardDirectory() -> URL { AssistantHistoryStore.standardDirectory() }

    public var url: URL { directory.appendingPathComponent(Self.fileName) }

    /// Hängt einen Eintrag an und kürzt auf die letzten 500. Ein Fehler (Speicher voll, Datei nicht lesbar) lässt die
    /// bisherige Datei unverändert.
    public func append(_ entry: AssistantAuditEntry) async -> AssistantAuditError? { await writer.append(entry) }

    /// Wie `append`, mit `false` bei einem Fehler.
    @discardableResult
    public func record(_ entry: AssistantAuditEntry) async -> Bool { await writer.append(entry) == nil }

    public func read() async -> AssistantAuditRead { await writer.read() }

    /// Alle lesbaren Einträge, älteste zuerst; beschädigte Zeilen (abgebrochener Schreibvorgang) werden übersprungen.
    public func entries() async -> [AssistantAuditEntry] { await writer.read().entries }

    /// Nur auf ausdrücklichen Wunsch (z. B. Prüfung); „Neues Gespräch“ löscht das Protokoll nicht.
    public func clear() async { await writer.clear() }
}
