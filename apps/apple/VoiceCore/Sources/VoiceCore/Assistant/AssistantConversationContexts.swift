import Foundation

// Kontexte des Gesprächs (lva#557): Inhalte, die der Nutzer im Gespräch anhängt. Sie gelten nur für dieses Gespräch (ein neues Gespräch
// und ein Wechsel des Assistenten leeren sie), liegen app-lokal NEBEN dem Verlauf (`kontexte.json` im selben Verzeichnis wie
// `verlauf.json`), nicht im Profil und nicht im Verzeichnis der Assistenten, und überleben einen Neustart der App zusammen mit dem
// Verlauf. Gespeichert werden nur Art, Kennung und Titel (Verweise, nie Inhalt) und der Schalter „Nur aus diesen Inhalten antworten“.
// Dieses Modul loggt nichts.

/// Ein angehängter Verweis: Art (`AssistantProfile.Context.Kind.rawValue`), Kennung und Titel.
public struct AssistantConversationContext: Equatable, Sendable, Codable {
    public var art: String
    public var id: String
    public var title: String

    public init(art: String, id: String, title: String) {
        self.art = art
        self.id = id
        self.title = title
    }

    public init(kind: AssistantProfile.Context.Kind, id: String, title: String) {
        self.init(art: kind.rawValue, id: id, title: title)
    }

    /// Der Verweis als Profil-Kontext; `nil` bei einer Art, die diese Fassung nicht kennt.
    public var context: AssistantProfile.Context? {
        AssistantProfile.Context.Kind(rawValue: art).map { AssistantProfile.Context(kind: $0, id: id, title: title) }
    }
}

public struct AssistantConversationContexts: Equatable, Sendable, Codable {
    /// Wie im Schema der Assistenten: höchstens 50.
    public static let maxCount = 50
    public static let empty = AssistantConversationContexts()

    public var contexts: [AssistantConversationContext] = []
    /// „Nur aus diesen Inhalten antworten“: setzt für dieses Gespräch den Modus `nur`.
    public var onlyFromThese = false

    public init(contexts: [AssistantConversationContext] = [], onlyFromThese: Bool = false) {
        self.contexts = contexts
        self.onlyFromThese = onlyFromThese
    }

    public var isEmpty: Bool { contexts.isEmpty && !onlyFromThese }

    public func contains(kind: AssistantProfile.Context.Kind, id: String) -> Bool {
        contexts.contains { $0.art == kind.rawValue && $0.id == id }
    }

    /// Hängt einen Verweis an. `false` (nichts geändert): schon da oder die Höchstzahl erreicht.
    @discardableResult
    public mutating func add(kind: AssistantProfile.Context.Kind, id: String, title: String) -> Bool {
        guard !id.isEmpty, contexts.count < Self.maxCount, !contains(kind: kind, id: id) else { return false }
        contexts.append(AssistantConversationContext(kind: kind, id: id, title: title))
        return true
    }

    public mutating func remove(kind: AssistantProfile.Context.Kind, id: String) {
        contexts.removeAll { $0.art == kind.rawValue && $0.id == id }
    }
}

private struct AssistantConversationContextsFile: Codable {
    var version: Int
    var contexts: AssistantConversationContexts
}

public struct AssistantConversationContextStore: Sendable {
    public static let fileVersion = 1
    public static let fileName = "kontexte.json"

    public let directory: URL
    private let remover: @Sendable (URL) throws -> Void

    /// `remover` löscht eine Datei; Tests setzen einen, der scheitert (Standard: `FileManager`).
    public init(directory: URL, remover: @escaping @Sendable (URL) throws -> Void = { try FileManager.default.removeItem(at: $0) }) {
        self.directory = directory
        self.remover = remover
    }

    public var url: URL { directory.appendingPathComponent(Self.fileName) }

    /// Gespeicherte Kontexte. Fehlt die Datei, ist sie unlesbar oder neuer, als diese Fassung versteht, ist das Ergebnis leer (eine
    /// unlesbare Datei bleibt unangetastet; das nächste Speichern ersetzt sie).
    public func load() -> AssistantConversationContexts {
        guard let data = try? Data(contentsOf: url),
              let file = try? JSONDecoder().decode(AssistantConversationContextsFile.self, from: data),
              file.version <= Self.fileVersion else { return .empty }
        var value = file.contexts
        value.contexts = Array(value.contexts.prefix(AssistantConversationContexts.maxCount))
        return value
    }

    /// Schreibt atomar, nie ins Backup, Dateischutz bis zur ersten Entsperrung. Ein leerer Stand löscht die Datei.
    public func save(_ value: AssistantConversationContexts) throws {
        guard !value.contexts.isEmpty else {
            if !clear() { throw AssistantHistoryStore.StoreError.deleteFailed }
            return
        }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        BookTalkHistoryStore.excludeFromBackup(directory)
        let data = try JSONEncoder().encode(AssistantConversationContextsFile(version: Self.fileVersion, contexts: value))
        #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try data.write(to: url, options: .atomic)
        #endif
        BookTalkHistoryStore.excludeFromBackup(url)
    }

    /// `false`, wenn eine Datei bleibt, die es nicht geben sollte. Eine Datei, die es nicht gibt, ist gelöscht.
    @discardableResult
    public func clear() -> Bool {
        guard FileManager.default.fileExists(atPath: url.path) else { return true }
        do { try remover(url) } catch { return false }
        return true
    }
}
