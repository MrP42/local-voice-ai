import Foundation

public struct ModelMessage: Codable, Equatable, Sendable {
    public enum Role: String, Codable, Sendable { case system, user, assistant }
    public let role: Role
    public let text: String
    public init(role: Role, text: String) { self.role = role; self.text = text }
}

public struct ModelRequest: Sendable {
    public var purpose: ModelPurpose
    public var messages: [ModelMessage]
    public var maxTokens: Int
    public var temperature: Double
    /// Strukturierte Ausgabe. Anbieter ohne eigene Unterstützung bekommen das Schema in den Prompt.
    public var jsonSchema: String?

    public init(purpose: ModelPurpose, messages: [ModelMessage], maxTokens: Int? = nil, temperature: Double = 0.2, jsonSchema: String? = nil) {
        self.purpose = purpose; self.messages = messages
        self.maxTokens = min(maxTokens ?? purpose.maxOutputTokens, purpose.maxOutputTokens)
        self.temperature = temperature; self.jsonSchema = jsonSchema
    }
}

public struct ModelUsage: Codable, Equatable, Sendable {
    public let inputTokens: Int?
    public let outputTokens: Int?
    public init(inputTokens: Int?, outputTokens: Int?) { self.inputTokens = inputTokens; self.outputTokens = outputTokens }
}

public enum ModelEvent: Equatable, Sendable {
    case delta(String)
    case done(ModelUsage?)
}

/// Fehler eines Anbieters, nur als feste Art. Antworttexte, Schlüssel oder Prompts gelangen nie in einen Fehler.
public enum ModelProviderError: String, Codable, Equatable, Sendable {
    case unauthorized, rateLimited, contextTooLong, server, network, invalidResponse, notInstalled

    /// HTTP-Status auf eine feste Art abbilden.
    public init(status: Int) {
        switch status {
        case 401, 403: self = .unauthorized
        case 413: self = .contextTooLong
        case 429: self = .rateLimited
        case 400..<500: self = .invalidResponse
        default: self = .server
        }
    }

    public var message: String {
        switch self {
        case .unauthorized: return "Der Anbieter hat den Schlüssel abgelehnt."
        case .rateLimited: return "Der Anbieter ist gerade ausgelastet. Bitte gleich noch einmal versuchen."
        case .contextTooLong: return "Der Text ist für dieses Modell zu lang."
        case .server: return "Beim Anbieter ist ein Fehler aufgetreten."
        case .network: return "Keine Verbindung zum Anbieter."
        case .invalidResponse: return "Die Antwort des Anbieters war unbrauchbar."
        case .notInstalled: return "Das Modell ist auf diesem Gerät nicht installiert."
        }
    }
}

/// Fehler ohne Inhalte und ohne Schlüssel.
public enum ModelFailure: Error, Equatable, Sendable, LocalizedError {
    case unavailable, consentMissing, blockedByPolicy, localOnly, insecureDestination, contextExceeded, cancelled, timeout
    /// Freigaben haben sich seit dem Ausstellen geändert; erneut starten.
    case stateChanged
    case provider(ModelProviderError)

    public var errorDescription: String? {
        switch self {
        case .unavailable: return "Das gewählte Modell ist gerade nicht erreichbar."
        case .consentMissing: return "Für diesen Anbieter fehlt deine Zustimmung."
        case .blockedByPolicy: return "Externe Modelle sind in diesem Profil gesperrt."
        case .localOnly: return "Diese Auswertung läuft nur auf dem Gerät."
        case .insecureDestination: return "Die Adresse des Anbieters ist nicht sicher (https nötig, ohne Zugangsdaten in der Adresse)."
        case .contextExceeded: return "Der Text ist für dieses Modell zu lang."
        case .cancelled: return "Abgebrochen."
        case .timeout: return "Das Modell hat nicht rechtzeitig geantwortet."
        case .stateChanged: return "Die Freigaben haben sich gerade geändert. Bitte noch einmal starten."
        case .provider(let error): return error.message
        }
    }
}

/// Ein Modellweg. Aufrufer nutzen ausschließlich `ModelGate.open(...)` und die `ModelSession`; nur sie stellt
/// Berechtigungen aus. Pflicht für Umsetzungen: unmittelbar vor dem Senden (nach dem letzten eigenen `await`)
/// `try await ModelProviderCheck.confirm(self, request, authorization)` aufrufen; Endpunkte senden zusätzlich nur,
/// wenn `authorization.permitsSending(to:)` für die tatsächlich benutzte URL gilt.
public protocol ModelProvider: Sendable {
    var route: ModelRoute { get }
    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error>
    /// `nil`: Aufrufer rechnet konservativ (UTF-8-Bytes als Obergrenze).
    func countTokens(_ text: String) async -> Int?
}

public extension ModelProvider {
    func countTokens(_ text: String) async -> Int? { nil }
    /// Tokenzahl oder sichere Obergrenze.
    func boundedTokens(_ text: String) async -> Int { await countTokens(text) ?? text.utf8.count }
}

public enum ModelProviderCheck {
    /// Prüft Route und Zweck und löst die Berechtigung ein (einmalig, mit erneuter Prüfung des aktuellen Stands).
    public static func confirm(_ provider: any ModelProvider, _ request: ModelRequest, _ authorization: ModelAuthorization) async throws {
        guard authorization.route == provider.route, authorization.purpose == request.purpose else { throw ModelFailure.consentMissing }
        try await authorization.confirm()
    }
}
