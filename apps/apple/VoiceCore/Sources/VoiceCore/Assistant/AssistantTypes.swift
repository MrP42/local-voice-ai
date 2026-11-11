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

/// Wertung eines Satzes vor der Ausgabe (Antworten ohne Quelle, E4.2a).
public enum AssistantSentenceStatus: String, Sendable, Codable, Equatable {
    /// Durch einen Auszug (oder den Bestand) gedeckt.
    case grounded
    /// Allgemeinwissen ohne Quelle; nur nach eigenem Opt-in.
    case unsourced
    /// Rückfrage, Gruß, feste Vorlage, Angabe zur Uhrzeit: weder belegt noch unbelegt.
    case neutral
    /// Hörbare Ansage „Ohne Quelle:“ bzw. „Aus deinen Inhalten:“. Gehört nicht zum Antworttext.
    case announcement
}

/// Abschnitt eines Beitrags mit gleicher Kennzeichnung (nur gespeichert, wenn belegte und unbelegte Sätze gemischt sind).
public struct AssistantTurnPart: Sendable, Equatable, Codable {
    public var text: String
    public var unsourced: Bool

    public init(text: String, unsourced: Bool) {
        self.text = text
        self.unsourced = unsourced
    }
}

/// Ein Beitrag im Verlauf: Äußerung des Nutzers und (gesprochene) Antwort.
public struct AssistantTurn: Sendable, Equatable, Codable {
    public var user: String
    public var assistant: String
    public var interrupted: Bool
    /// Nur Quellen, die mindestens einen gesprochenen Satz decken.
    public var sources: [AssistantPassage]
    public var general: Bool          // ohne deckenden Auszug beantwortet
    /// Mindestens ein unbelegter Satz wurde gesprochen. Alte Beiträge ohne das Feld: wie `general`.
    public var unsourced: Bool
    /// Gemischte Antwort: Abschnitte in Sprechreihenfolge mit Kennzeichnung; sonst leer (dann gilt `unsourced` für alles).
    public var parts: [AssistantTurnPart]
    public var model: String
    public var at: Date

    public init(user: String, assistant: String, interrupted: Bool = false, sources: [AssistantPassage] = [],
                general: Bool = false, unsourced: Bool = false, parts: [AssistantTurnPart] = [], model: String = "", at: Date = Date()) {
        self.user = user
        self.assistant = assistant
        self.interrupted = interrupted
        self.sources = sources
        self.general = general
        self.unsourced = unsourced
        self.parts = parts
        self.model = model
        self.at = at
    }

    private enum CodingKeys: String, CodingKey {
        case user, assistant, interrupted, sources, general, unsourced, parts, model, at
    }

    /// Die synthetisierte Fassung ignoriert Standardwerte: Beiträge aus Verlaufsdateien vor dem Feld `unsourced` müssen laden.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        user = try c.decode(String.self, forKey: .user)
        assistant = try c.decode(String.self, forKey: .assistant)
        interrupted = try c.decode(Bool.self, forKey: .interrupted)
        sources = try c.decode([AssistantPassage].self, forKey: .sources)
        general = try c.decode(Bool.self, forKey: .general)
        unsourced = try c.decodeIfPresent(Bool.self, forKey: .unsourced) ?? general
        parts = try c.decodeIfPresent([AssistantTurnPart].self, forKey: .parts) ?? []
        model = try c.decode(String.self, forKey: .model)
        at = try c.decode(Date.self, forKey: .at)
    }
}

public struct AssistantConfig: Sendable {
    public var modelLabel: String
    public var isCloud: Bool
    public var contextTokens: Int      // Apple 4096, Gemma laut Budget, Cloud ≥ 16000
    public var maxAnswerTokens: Int    // Apple 400, sonst 700
    public var userName: String?
    public var timeout: TimeInterval
    /// Antworten ohne Quelle (Allgemeinwissen) erlaubt. Standard: aus. Die Sitzung kann es zur Laufzeit ändern.
    public var allowUnsourced: Bool
    /// Version des Einwilligungstextes zu `allowUnsourced` (nur für das Protokoll).
    public var consentVersion: Int

    public init(modelLabel: String, isCloud: Bool, contextTokens: Int, maxAnswerTokens: Int, userName: String?, timeout: TimeInterval,
                allowUnsourced: Bool = false, consentVersion: Int = 0) {
        self.modelLabel = modelLabel
        self.isCloud = isCloud
        self.contextTokens = contextTokens
        self.maxAnswerTokens = maxAnswerTokens
        self.userName = userName
        self.timeout = timeout
        self.allowUnsourced = allowUnsourced
        self.consentVersion = consentVersion
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

/// Zahl der Sätze je Wertung in einer Antwort (für Protokoll und Anzeige; keine Inhalte).
public struct AssistantSentenceCounts: Sendable, Equatable, Codable {
    public var grounded = 0
    public var unsourced = 0
    public var neutral = 0
    /// Verworfen (nicht gesprochen, nicht angezeigt), einschließlich `revoked`.
    public var dropped = 0
    /// Davon: wegen entzogenem Opt-in oder entzogenem Bereich während der Antwort.
    public var revoked = 0

    public init(grounded: Int = 0, unsourced: Int = 0, neutral: Int = 0, dropped: Int = 0, revoked: Int = 0) {
        self.grounded = grounded
        self.unsourced = unsourced
        self.neutral = neutral
        self.dropped = dropped
        self.revoked = revoked
    }
}

public struct AssistantAnswer: Sendable, Equatable {
    /// Die gesprochenen und angezeigten Sätze (ohne Ansagen, ohne verworfene).
    public let text: String
    /// Nur Quellen, die mindestens einen ausgegebenen Satz decken.
    public let sources: [AssistantPassage]
    public let general: Bool
    public let firstSentenceMs: Int?
    public let totalMs: Int
    /// Mindestens ein unbelegter Satz wurde ausgegeben.
    public let unsourced: Bool
    public let counts: AssistantSentenceCounts

    public init(text: String, sources: [AssistantPassage], general: Bool, firstSentenceMs: Int?, totalMs: Int,
                unsourced: Bool = false, counts: AssistantSentenceCounts = AssistantSentenceCounts()) {
        self.text = text
        self.sources = sources
        self.general = general
        self.firstSentenceMs = firstSentenceMs
        self.totalMs = totalMs
        self.unsourced = unsourced
        self.counts = counts
    }
}

public enum AssistantFailure: String, Sendable, Equatable { case cancelled, timeout, noModel, modelFailed, offline, consentRequired }

public enum AssistantEvent: Sendable, Equatable {
    /// Abgerufene Auszüge dieser Anfrage (Vorab-Anzeige). Gedeckt wird erst je Satz, siehe `sentence`.
    case sources(requestId: String, [AssistantPassage])
    /// Ein Satz zur Ausgabe. `speech` = ohne Markdown, sprechbar. `sources` nur bei `grounded`: die deckenden Auszüge
    /// (bei Bestandsangaben leer). Verworfene Sätze kommen nie.
    case sentence(requestId: String, text: String, speech: String, status: AssistantSentenceStatus, sources: [AssistantPassage])
    case done(requestId: String, AssistantAnswer)
    case failed(requestId: String, AssistantFailure)
}
