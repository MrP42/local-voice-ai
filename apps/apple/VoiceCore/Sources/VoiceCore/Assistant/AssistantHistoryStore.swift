import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): Verlauf auf dem Gerät, Muster `BookTalkHistoryStore`.
// `Application Support/Assistant/verlauf.json`, höchstens 200 Beiträge, nie im Backup, Dateischutz bis zur ersten
// Entsperrung. Kein Rohaudio, kein Text in Logs oder Fehlern: Dieses Modul loggt nichts.

private struct AssistantHistoryFile: Codable {
    var version: Int
    var turns: [AssistantTurn]
}

public struct AssistantHistoryStore: Sendable {
    public static let fileVersion = 1
    public static let maxStoredTurns = 200
    public static let fileName = "verlauf.json"

    public let directory: URL
    private let remover: @Sendable (URL) throws -> Void

    /// `remover` löscht eine Datei; Tests setzen einen, der scheitert (Standard: `FileManager`).
    public init(directory: URL, remover: @escaping @Sendable (URL) throws -> Void = { try FileManager.default.removeItem(at: $0) }) {
        self.directory = directory
        self.remover = remover
    }

    /// Die Verlaufsdatei ließ sich nicht löschen (`save` mit leerem Verlauf).
    public enum StoreError: Error, Sendable { case deleteFailed }

    /// Standardort der App: `Application Support/Assistant`.
    public static func standardDirectory() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("Assistant", isDirectory: true)
    }

    public var url: URL { directory.appendingPathComponent(Self.fileName) }

    /// Gespeicherte Beiträge. Fehlt die Datei oder ist sie unlesbar, ist der Verlauf leer; eine unlesbare Datei wird
    /// beiseitegelegt (`.defekt`), eine neuere Fassung bleibt unangetastet.
    public func load() -> [AssistantTurn] {
        let file = url
        guard let data = try? Data(contentsOf: file) else { return [] }
        guard let decoded = try? Self.decoder().decode(AssistantHistoryFile.self, from: data) else {
            let aside = file.appendingPathExtension("defekt")
            try? FileManager.default.removeItem(at: aside)
            try? FileManager.default.moveItem(at: file, to: aside)
            return []
        }
        guard decoded.version <= Self.fileVersion else { return [] }
        return Array(decoded.turns.suffix(Self.maxStoredTurns))
    }

    /// Quellen werden auf Titel, Fundstelle, Bereich und Kennung gekürzt: Auszugstexte aus Büchern, Transkripten, Ablage
    /// und Notizen gehören nicht in die Verlaufsdatei (Anzeige und Prompt brauchen sie nicht, s. `AssistantPrompt`).
    static func minimized(_ turn: AssistantTurn) -> AssistantTurn {
        var out = turn
        out.sources = turn.sources.map {
            AssistantPassage(area: $0.area, itemId: $0.itemId, title: $0.title, locator: $0.locator, text: "", score: 0)
        }
        return out
    }

    /// Schreibt atomar die letzten 200 Beiträge (Quellen ohne Auszugstext). Ein leerer Verlauf löscht die Datei.
    public func save(_ turns: [AssistantTurn]) throws {
        let kept = turns.suffix(Self.maxStoredTurns).map(Self.minimized)
        guard !kept.isEmpty else {
            if !clear() { throw StoreError.deleteFailed }
            return
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        BookTalkHistoryStore.excludeFromBackup(directory)
        let data = try Self.encoder().encode(AssistantHistoryFile(version: Self.fileVersion, turns: kept))
        #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try data.write(to: url, options: .atomic)
        #endif
        BookTalkHistoryStore.excludeFromBackup(url)
    }

    /// „Verlauf löschen“. `false`, wenn eine Datei bleibt, die es nicht geben sollte (Runde 8, Suchlauf d: Wer „gelöscht“ sagt, ohne
    /// dass es stimmt, täuscht den Nutzer; der Aufrufer meldet es). Eine Datei, die es nicht gibt, ist gelöscht.
    @discardableResult
    public func clear() -> Bool {
        var deleted = true
        for target in [url, url.appendingPathExtension("defekt")] where FileManager.default.fileExists(atPath: target.path) {
            do { try remover(target) } catch { deleted = false }
        }
        return deleted
    }

    /// Ob überhaupt ein Verlauf auf dem Gerät liegt (für die Einstellungen).
    public var hasAny: Bool { FileManager.default.fileExists(atPath: url.path) }

    private static func encoder() -> JSONEncoder { JSONEncoder() }

    private static func decoder() -> JSONDecoder { JSONDecoder() }
}
