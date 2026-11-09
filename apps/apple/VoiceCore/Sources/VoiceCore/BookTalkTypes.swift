import Foundation

// Buch-Gespräch (Epic #307, Paket G4): Vertragstypen, camelCase exakt nach
// tooling/fixtures/buch-gespraech/vertrag.schema.json (JSON snake_case über `BookTalkJSON`).

public enum BookTalkInput: String, Sendable, Codable, Equatable {
    case voice, text
}

/// FailCode des Vertrags plus `no_hits` (nur als `reason` einer Antwort).
public enum BookTalkCode: String, Sendable, Codable, Equatable {
    case noModel = "no_model"
    case budget
    case offline
    case consentRequired = "consent_required"
    case cancelled
    case timeout
    case llmFailed = "llm_failed"
    case noFrontier = "no_frontier"
    case noHits = "no_hits"
}

/// Eingang einer Frage (Draht: snake_case wie im Schema; Swift-Properties camelCase). Bereich und Grenze sind je Anfrage unveränderlich (Schnappschuss).
public struct BookTalkRequest: Sendable, Codable, Equatable {
    public let bookId: String
    public let requestId: String
    public let question: String
    public let frontier: Int?
    public let scope: BookTalkScope
    public let input: BookTalkInput

    enum CodingKeys: String, CodingKey {
        case bookId = "book_id", requestId = "request_id", question, frontier, scope, input
    }

    public init(bookId: String, requestId: String, question: String, frontier: Int?, scope: BookTalkScope, input: BookTalkInput) {
        self.bookId = bookId
        self.requestId = requestId
        self.question = question
        self.frontier = frontier
        self.scope = scope
        self.input = input
    }
}

public struct BookTalkCitation: Sendable, Codable, Equatable {
    /// Nummer des Auszugs (`[Q<q>]`).
    public let q: Int
    /// Seite aus der letzten Seitenmarke vor `start`.
    public let page: Int?
    public let chapterIndex: Int
    public let chapterTitle: String
    /// UTF-16-Offsets im Originaltext, `[start, end)` ist das Zitat.
    public let start: Int
    public let end: Int
    /// Wörtlicher Satz (oder bis zu zwei Sätze), höchstens 240 Zeichen.
    public let quote: String

    enum CodingKeys: String, CodingKey {
        case q, page, chapterIndex = "chapter_index", chapterTitle = "chapter_title", start, end, quote
    }
}

public struct BookTalkAnswerSentence: Sendable, Codable, Equatable {
    public let index: Int
    public let text: String
    public let speech: String
    public let citations: [BookTalkCitation]
    public let general: Bool
}

public struct BookTalkAnswer: Sendable, Codable, Equatable {
    public let intent: BookTalkIntent
    public let sentences: [BookTalkAnswerSentence]
    public let text: String
    /// Sprechfassung: Satz-Sprechfassungen, dann Rückfrage („Soll ich dir die Stelle vorlesen?“) bzw. „bis Seite N“.
    public let speech: String
    public let citations: [BookTalkCitation]
    public let foundOnly: Bool
    public let reason: BookTalkCode?
    public let dropped: Int
    public let scope: BookTalkScope
    /// Grenze zum Zeitpunkt der Frage (auch bei `scope = book`).
    public let frontier: Int

    enum CodingKeys: String, CodingKey {
        case intent, sentences, text, speech, citations, foundOnly = "found_only", reason, dropped, scope, frontier
    }

    /// Kein einziger Beleg.
    public var uncited: Bool { citations.isEmpty }
    /// Nach einer `locate`-Antwort mit Beleg wartet die Rückfrage auf ein „Ja“.
    public var offersRead: Bool { intent == .locate && !citations.isEmpty }
    /// Bei „nichts gefunden / Grenze unbekannt“ im Bereich „Bis hier“: Angebot, aufs ganze Buch zu erweitern.
    public var offersScopeBook: Bool { foundOnly && scope == .untilPosition && (reason == .noHits || reason == .noFrontier) }
    /// Was nach den Sätzen noch gesprochen wird (Rückfrage, „bis Seite N“).
    public var speechTail: String {
        let head = sentences.map(\.speech).joined(separator: " ")
        guard speech.hasPrefix(head) else { return "" }
        return String(speech.dropFirst(head.count)).trimmingCharacters(in: .whitespaces)
    }
}

public enum BookTalkEvent: Sendable, Equatable, Codable {
    case sentence(requestId: String, BookTalkAnswerSentence)
    case done(requestId: String, BookTalkAnswer)
    case failed(requestId: String, code: BookTalkCode)

    private enum Keys: String, CodingKey { case kind, requestId = "request_id", sentence, answer, code }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        let id = try c.decode(String.self, forKey: .requestId)
        switch try c.decode(String.self, forKey: .kind) {
        case "sentence": self = .sentence(requestId: id, try c.decode(BookTalkAnswerSentence.self, forKey: .sentence))
        case "done": self = .done(requestId: id, try c.decode(BookTalkAnswer.self, forKey: .answer))
        case "failed": self = .failed(requestId: id, code: try c.decode(BookTalkCode.self, forKey: .code))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unbekannte Art")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .sentence(let id, let s):
            try c.encode("sentence", forKey: .kind); try c.encode(id, forKey: .requestId); try c.encode(s, forKey: .sentence)
        case .done(let id, let a):
            try c.encode("done", forKey: .kind); try c.encode(id, forKey: .requestId); try c.encode(a, forKey: .answer)
        case .failed(let id, let code):
            try c.encode("failed", forKey: .kind); try c.encode(id, forKey: .requestId); try c.encode(code, forKey: .code)
        }
    }
}

public struct BookTalkStoredTurn: Sendable, Codable, Equatable {
    public let question: String
    public let answer: BookTalkAnswer
    public let scope: BookTalkScope
    public let frontier: Int
    public let input: BookTalkInput
    /// ISO-8601, UTC.
    public let at: String

    public init(question: String, answer: BookTalkAnswer, scope: BookTalkScope, frontier: Int, input: BookTalkInput, at: String) {
        self.question = question
        self.answer = answer
        self.scope = scope
        self.frontier = frontier
        self.input = input
        self.at = at
    }

    var promptTurn: BookTalkTurn {
        BookTalkTurn(question: question, answer: answer.text, scope: scope, frontier: frontier)
    }
}

/// Objekt mit `kind` (Schema `command.schema.json`).
public struct BookTalkCommandMessage: Sendable, Codable, Equatable {
    public let kind: BookTalkCommand
    public init(kind: BookTalkCommand) { self.kind = kind }
}

/// Figur, Ort oder Begriff aus Buchgedächtnis bzw. `begriffe.json`. Nur Namen und Varianten, keine Beschreibungen.
public struct BookTalkEntity: Sendable, Equatable {
    public enum Kind: Sendable, Equatable { case person, place, event, term }
    public let name: String
    public let variants: [String]
    public let kind: Kind

    public init(name: String, variants: [String] = [], kind: Kind = .person) {
        self.name = name
        self.variants = variants
        self.kind = kind
    }

    public var forms: [String] { [name] + variants }
}

/// Gemessenes Byte-Profil für Modelle ohne exakten Zähler: Obergrenze = UTF-8-Bytes aller Nachrichtentexte
/// + gemessene Vorlagenkosten. Ein Profil gehört nur in die App-Liste, wenn ein Test mit dem echten Tokenizer
/// an den Fixtures zeigt, dass die Obergrenze nie unterschritten wird. Dieser Kern liefert keine Profile mit.
public struct BookTalkByteProfile: Sendable, Equatable {
    public let name: String
    /// Feste Vorlagen-/Sondertoken (BOS, Generierungsprompt …).
    public let fixedTokens: Int
    /// Rahmen je Nachricht (Rolle, Trenner).
    public let perMessageTokens: Int

    public init(name: String, fixedTokens: Int, perMessageTokens: Int) {
        self.name = name
        self.fixedTokens = fixedTokens
        self.perMessageTokens = perMessageTokens
    }
}

extension BookTalkByteProfile {
    /// `apple-fm@1` (Plan §11.1): Obergrenze = UTF-8-Bytes aller Texte + 600 Token Rahmen. Gilt als „gemessen“,
    /// sobald der Gerätetest (Grenzsuche über `exceededContextWindowSize` mit den Fixtures) es bestätigt; bis dahin Vorschau.
    public static let appleFM1 = BookTalkByteProfile(name: "apple-fm@1", fixedTokens: 600, perMessageTokens: 0)
    /// `relay-cli@1` (Plan §11.3): Bytes + 2.000 Token Rahmen ≤ gemeldete `context_tokens` des PC-Modells.
    public static let relayCLI1 = BookTalkByteProfile(name: "relay-cli@1", fixedTokens: 2_000, perMessageTokens: 0)
}

/// Exakter Tokenzähler des Modells (llama_tokenize, `/tokenize`, Foundation Models `tokenCount`). Optional.
public protocol BookTalkTokenCounter: Sendable {
    func count(_ text: String) -> Int
}
