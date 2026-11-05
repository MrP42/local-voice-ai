import Foundation

// Buch-Gespräch (Epic #307, Paket G5a): Verlauf je Buch auf dem Gerät.
// `Application Support/BookTalk/<bookId>.json`, höchstens 200 Wechsel, nie im Backup, Dateischutz bis zur
// ersten Entsperrung. Kein Frage-, Antwort- oder Buchtext in Logs oder Fehlern: Dieses Modul loggt nichts.

private struct BookTalkHistoryFile: Codable {
    var version: Int
    var turns: [BookTalkStoredTurn]
}

public struct BookTalkHistoryStore: Sendable {
    public static let fileVersion = 1
    public let directory: URL

    public init(directory: URL) { self.directory = directory }

    /// Standardort der App: `Application Support/BookTalk`.
    public static func standardDirectory() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("BookTalk", isDirectory: true)
    }

    /// Dateiname aus der Buch-ID; Zeichen außerhalb von `A-Za-z0-9_-` werden ersetzt (kein Pfadausbruch).
    public static func fileName(bookId: String) -> String {
        let safe = bookId.unicodeScalars.map { s -> Character in
            switch s.value {
            case 0x30...0x39, 0x41...0x5A, 0x61...0x7A, 0x5F, 0x2D: return Character(s)
            default: return "_"
            }
        }
        let name = String(safe.prefix(120))
        return (name.isEmpty ? "_" : name) + ".json"
    }

    public func url(bookId: String) -> URL { directory.appendingPathComponent(Self.fileName(bookId: bookId)) }

    /// Gespeicherte Wechsel. Fehlt die Datei oder ist sie unlesbar, ist der Verlauf leer;
    /// eine unlesbare Datei wird beiseitegelegt (`.defekt`), eine neuere Fassung bleibt unangetastet.
    public func load(bookId: String) -> [BookTalkStoredTurn] {
        let file = url(bookId: bookId)
        guard let data = try? Data(contentsOf: file) else { return [] }
        guard let decoded = try? JSONDecoder().decode(BookTalkHistoryFile.self, from: data) else {
            let aside = file.appendingPathExtension("defekt")
            try? FileManager.default.removeItem(at: aside)
            try? FileManager.default.moveItem(at: file, to: aside)
            return []
        }
        guard decoded.version <= Self.fileVersion else { return [] }
        return Array(decoded.turns.suffix(BookTalkSession.maxStoredTurns))
    }

    /// Schreibt atomar die letzten 200 Wechsel. Ein leerer Verlauf löscht die Datei.
    public func save(bookId: String, turns: [BookTalkStoredTurn]) throws {
        let kept = Array(turns.suffix(BookTalkSession.maxStoredTurns))
        guard !kept.isEmpty else { return clear(bookId: bookId) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        Self.excludeFromBackup(directory)
        let data = try JSONEncoder().encode(BookTalkHistoryFile(version: Self.fileVersion, turns: kept))
        let file = url(bookId: bookId)
        #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
        try data.write(to: file, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try data.write(to: file, options: .atomic)
        #endif
        Self.excludeFromBackup(file)
    }

    /// „Verlauf leeren“ und Löschen des Buchs.
    public func clear(bookId: String) {
        let file = url(bookId: bookId)
        try? FileManager.default.removeItem(at: file)
        try? FileManager.default.removeItem(at: file.appendingPathExtension("defekt"))
    }

    /// „Alle Buchgespräche löschen“ (Einstellungen).
    public func clearAll() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// Ob überhaupt ein Verlauf auf dem Gerät liegt (für die Einstellungen).
    public var hasAny: Bool {
        ((try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []).contains { $0.hasSuffix(".json") }
    }

    static func excludeFromBackup(_ url: URL) {
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        var target = url
        try? target.setResourceValues(values)
    }
}
