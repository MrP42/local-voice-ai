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

    public init(directory: URL) { self.directory = directory }

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

    /// Schreibt atomar die letzten 200 Beiträge. Ein leerer Verlauf löscht die Datei.
    public func save(_ turns: [AssistantTurn]) throws {
        let kept = Array(turns.suffix(Self.maxStoredTurns))
        guard !kept.isEmpty else { return clear() }
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

    /// „Verlauf löschen“.
    public func clear() {
        try? FileManager.default.removeItem(at: url)
        try? FileManager.default.removeItem(at: url.appendingPathExtension("defekt"))
    }

    /// Ob überhaupt ein Verlauf auf dem Gerät liegt (für die Einstellungen).
    public var hasAny: Bool { FileManager.default.fileExists(atPath: url.path) }

    private static func encoder() -> JSONEncoder { JSONEncoder() }

    private static func decoder() -> JSONDecoder { JSONDecoder() }
}
