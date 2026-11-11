import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Protokoll ohne Inhalte.
// Je Antwort ein Eintrag (Zeit, Modell, lokal/Cloud, Modus „ohne Quelle“, Einwilligungsversion, Zahl der Sätze je Wertung,
// Bereiche der Quellen, Ausgang), je Einwilligung und Widerruf einer. Keine Fragen, Antworten, Titel oder Auszüge: die Typen
// haben dafür gar kein Feld. JSON-Zeilen (`protokoll.jsonl`) neben dem Verlauf, höchstens 500 Einträge, Schreiben atomar
// (Datei komplett neu, dann umbenennen: ein Abbruch oder ein voller Datenträger lässt die alte Datei unversehrt).
// Eigene Datei, damit „Neues Gespräch“ und „Verlauf löschen“ das Protokoll nicht mitlöschen. Dieses Modul loggt nichts.

public struct AssistantAuditEntry: Sendable, Equatable, Codable {
    public enum Kind: String, Sendable, Codable {
        case answer, unsourcedOn, unsourcedOff, cloudGranted, cloudRevoked
    }

    public enum Outcome: String, Sendable, Codable {
        /// Zu Ende gesprochen.
        case completed
        /// Vom Nutzer oder durch einen Fehler unterbrochen.
        case interrupted
        /// Wegen entzogenem Opt-in, Bereich oder entzogener Zustimmung verworfen oder angehalten.
        case revoked
    }

    public var at: Date
    public var kind: Kind
    /// Anzeigename des Modells (z. B. „Gemma 4 E2B“), kein Inhalt.
    public var model: String
    public var cloud: Bool
    /// „Antworten ohne Quelle“ war bei Beginn der Antwort an.
    public var unsourcedMode: Bool
    public var consentVersion: Int
    /// Gesprochene Sätze je Wertung.
    public var grounded: Int
    public var unsourced: Int
    public var neutral: Int
    /// Verworfene Sätze (nicht gesprochen).
    public var dropped: Int
    /// Bereiche der Quellen (Rohwerte `books`, `transcripts`, `files`, `notes`), sortiert.
    public var areas: [String]
    public var outcome: Outcome?

    public init(at: Date = Date(), kind: Kind, model: String = "", cloud: Bool = false, unsourcedMode: Bool = false,
                consentVersion: Int = 0, grounded: Int = 0, unsourced: Int = 0, neutral: Int = 0, dropped: Int = 0,
                areas: [AssistantArea] = [], outcome: Outcome? = nil) {
        self.at = at
        self.kind = kind
        self.model = model
        self.cloud = cloud
        self.unsourcedMode = unsourcedMode
        self.consentVersion = consentVersion
        self.grounded = grounded
        self.unsourced = unsourced
        self.neutral = neutral
        self.dropped = dropped
        self.areas = Set(areas.map(\.rawValue)).sorted()
        self.outcome = outcome
    }
}

public actor AssistantAuditLog {
    public static let maxEntries = 500
    public static let fileName = "protokoll.jsonl"

    public let directory: URL

    public init(directory: URL) { self.directory = directory }

    /// Standardort der App: neben dem Verlauf in `Application Support/Assistant`.
    public static func standardDirectory() -> URL { AssistantHistoryStore.standardDirectory() }

    public nonisolated var url: URL { directory.appendingPathComponent(Self.fileName) }

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

    private func readLines() -> [Data] {
        guard let data = try? Data(contentsOf: url), !data.isEmpty else { return [] }
        return data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true).map { Data($0) }
    }

    /// Hängt einen Eintrag an und kürzt auf die letzten 500. `false`, wenn das Schreiben scheiterte (Speicher voll …);
    /// die bisherige Datei bleibt dann unverändert.
    @discardableResult
    public func record(_ entry: AssistantAuditEntry) -> Bool {
        guard let line = try? Self.encoder().encode(entry) else { return false }
        var lines = readLines()
        lines.append(line)
        if lines.count > Self.maxEntries { lines.removeFirst(lines.count - Self.maxEntries) }
        var data = Data()
        for l in lines {
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
            return true
        } catch {
            return false
        }
    }

    /// Alle lesbaren Einträge, älteste zuerst; beschädigte Zeilen (abgebrochener Schreibvorgang) werden übersprungen.
    public func entries() -> [AssistantAuditEntry] {
        let decoder = Self.decoder()
        return readLines().compactMap { try? decoder.decode(AssistantAuditEntry.self, from: $0) }
    }

    /// Nur auf ausdrücklichen Wunsch (z. B. Prüfung); „Neues Gespräch“ löscht das Protokoll nicht.
    public func clear() { try? FileManager.default.removeItem(at: url) }
}
