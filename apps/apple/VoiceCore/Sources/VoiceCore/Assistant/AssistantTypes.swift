import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): gemeinsame Typen. Reines Foundation, unter Linux testbar.
// Namen und Signaturen sind mit Paket A abgestimmt (assistent-entwurf.md); Erweiterungen ja, Umbenennungen nein.

/// Bereiche, aus denen der Assistent Inhalte beziehen darf.
public enum AssistantArea: String, Sendable, Codable, CaseIterable {
    case books, transcripts, files, notes

    /// Reihenfolge für Bestand und Abruf (Bücher zuerst).
    var order: Int {
        switch self {
        case .books: 0
        case .transcripts: 1
        case .files: 2
        case .notes: 3
        }
    }

    /// Kopfzeile im Bestand (Mehrzahl).
    public var inventoryLabel: String {
        switch self {
        case .books: "Bücher"
        case .transcripts: "Transkripte"
        case .files: "Dokumente"
        case .notes: "Notizen"
        }
    }

    /// Kennzeichnung eines Auszugs im Prompt (Einzahl).
    public var singularLabel: String {
        switch self {
        case .books: "Buch"
        case .transcripts: "Transkript"
        case .files: "Dokument"
        case .notes: "Notiz"
        }
    }
}

public struct AssistantItem: Sendable, Equatable, Codable {
    public let area: AssistantArea
    public let id: String
    public let title: String
    public let detail: String?      // „Autor · 42 % gelesen“, „12.10.2026 · 48 min“
    public let updated: Date?

    public init(area: AssistantArea, id: String, title: String, detail: String? = nil, updated: Date? = nil) {
        self.area = area
        self.id = id
        self.title = title
        self.detail = detail
        self.updated = updated
    }
}

public struct AssistantPassage: Sendable, Equatable, Codable {
    public let area: AssistantArea
    public let itemId: String
    public let title: String
    public let locator: String?     // „S. 12“, „00:14:03“, „Seite 2“
    public let text: String
    public let score: Double

    public init(area: AssistantArea, itemId: String, title: String, locator: String? = nil, text: String, score: Double) {
        self.area = area
        self.itemId = itemId
        self.title = title
        self.locator = locator
        self.text = text
        self.score = score
    }
}

/// Quelle mit eigenem Index (Ablage). Paket A liefert die Umsetzung.
public protocol AssistantSource: Sendable {
    var area: AssistantArea { get }
    func items() async -> [AssistantItem]
    func search(_ question: String, limit: Int) async -> [AssistantPassage]
}

/// Dokument für den eingebauten Index (Bücher, Transkripte, Notizen).
public struct AssistantDocument: Sendable {
    public struct Section: Sendable {
        public let locator: String?
        public let text: String
        public init(locator: String?, text: String) {
            self.locator = locator
            self.text = text
        }
    }

    public let item: AssistantItem
    /// Ändert sich, wenn Text oder Lesegrenze sich ändern.
    public let version: String
    public let sections: [Section]

    public init(item: AssistantItem, version: String, sections: [Section]) {
        self.item = item
        self.version = version
        self.sections = sections
    }
}

/// Ein Beitrag im Verlauf: Äußerung des Nutzers und (gesprochene) Antwort.
public struct AssistantTurn: Sendable, Equatable, Codable {
    public var user: String
    public var assistant: String
    public var interrupted: Bool
    public var sources: [AssistantPassage]
    public var general: Bool          // ohne Auszug beantwortet
    public var model: String
    public var at: Date

    public init(user: String, assistant: String, interrupted: Bool = false, sources: [AssistantPassage] = [],
                general: Bool = false, model: String = "", at: Date = Date()) {
        self.user = user
        self.assistant = assistant
        self.interrupted = interrupted
        self.sources = sources
        self.general = general
        self.model = model
        self.at = at
    }
}

public struct AssistantConfig: Sendable {
    public var modelLabel: String
    public var isCloud: Bool
    public var contextTokens: Int      // Apple 4096, Gemma laut Budget, Cloud ≥ 16000
    public var maxAnswerTokens: Int    // Apple 400, sonst 700
    public var userName: String?
    public var timeout: TimeInterval

    public init(modelLabel: String, isCloud: Bool, contextTokens: Int, maxAnswerTokens: Int, userName: String?, timeout: TimeInterval) {
        self.modelLabel = modelLabel
        self.isCloud = isCloud
        self.contextTokens = contextTokens
        self.maxAnswerTokens = maxAnswerTokens
        self.userName = userName
        self.timeout = timeout
    }
}

public struct AssistantRequest: Sendable, Equatable {
    public let requestId: String
    public let text: String
    public let afterInterruption: Bool

    public init(requestId: String, text: String, afterInterruption: Bool = false) {
        self.requestId = requestId
        self.text = text
        self.afterInterruption = afterInterruption
    }
}

public struct AssistantAnswer: Sendable, Equatable {
    public let text: String
    public let sources: [AssistantPassage]
    public let general: Bool
    public let firstSentenceMs: Int?
    public let totalMs: Int

    public init(text: String, sources: [AssistantPassage], general: Bool, firstSentenceMs: Int?, totalMs: Int) {
        self.text = text
        self.sources = sources
        self.general = general
        self.firstSentenceMs = firstSentenceMs
        self.totalMs = totalMs
    }
}

public enum AssistantFailure: String, Sendable, Equatable { case cancelled, timeout, noModel, modelFailed, offline, consentRequired }

public enum AssistantEvent: Sendable, Equatable {
    case sources(requestId: String, [AssistantPassage])
    case sentence(requestId: String, text: String, speech: String)   // speech = ohne Markdown, sprechbar
    case done(requestId: String, AssistantAnswer)
    case failed(requestId: String, AssistantFailure)
}
