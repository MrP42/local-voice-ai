import Foundation

// Gesprächsmodus (Paket M): eigener OpenAI-kompatibler Endpunkt (Ollama, LM Studio, llama-server, API-Anbieter).
// Streaming über `POST {baseURL}/chat/completions`; Zeilenparser und Anfragekörper liegen im Kern (`AssistantSSE`).

/// Gespeicherte Angaben zum Endpunkt. Der Schlüssel gehört nicht hierher, er liegt nur im Schlüsselbund.
struct AssistantEndpoint: Codable, Equatable, Sendable {
    static let defaultContextTokens = 16_000

    /// Zum Beispiel `http://192.168.178.200:11434/v1` oder `https://api.openai.com/v1`.
    var baseURL: String
    var model: String
    /// Anzeigename, Standard = Modellname.
    var label: String
    var contextTokens: Int

    init(baseURL: String, model: String, label: String = "", contextTokens: Int = AssistantEndpoint.defaultContextTokens) {
        self.baseURL = baseURL
        self.model = model
        self.label = label.isEmpty ? model : label
        self.contextTokens = contextTokens
    }

    /// Adresse ohne Leerraum und ohne abschließende Schrägstriche. „/v1“ wird nie ergänzt.
    static func normalizedBase(_ text: String) -> String {
        var base = text.trimmingCharacters(in: .whitespacesAndNewlines)
        while base.hasSuffix("/") { base.removeLast() }
        return base
    }

    var chatURL: URL? { URL(string: Self.normalizedBase(baseURL) + "/chat/completions") }
}

/// Fehler des Endpunkts mit Text für die Anzeige. Enthält nie Inhalte der Antwort und nie den Schlüssel.
enum AssistantEndpointError: Error, Equatable, LocalizedError {
    case invalidAddress
    case insecureCloud(host: String)
    case missingModel
    case badContext
    case badKey
    case keychain
    case keyRejected
    case notFound
    case rateLimited
    case server(Int)
    case emptyAnswer
    case transportBlocked
    case tlsFailed

    var errorDescription: String? {
        switch self {
        case .invalidAddress: "Die Adresse ist ungültig. Sie beginnt mit http:// oder https:// und endet meist auf /v1."
        case .insecureCloud(let host): "Für \(host) ist https nötig. Unverschlüsselt (http) geht es nur im eigenen Netz."
        case .missingModel: "Trage den Namen des Modells ein."
        case .badContext: "Die Kontextgröße muss zwischen 2048 und 1000000 Token liegen."
        case .badKey: "Der Schlüssel enthält ungültige Zeichen."
        case .keychain: "Der Schlüssel konnte nicht im Schlüsselbund gespeichert werden."
        case .keyRejected: "Schlüssel abgelehnt. Prüfe, ob er zu diesem Anbieter gehört."
        case .notFound: "Modell oder Adresse nicht gefunden. Prüfe den Modellnamen und ob die Adresse auf /v1 endet."
        case .rateLimited: "Limit des Anbieters erreicht. Versuche es später noch einmal."
        case .server(let code): "Der Server meldet einen Fehler (\(code))."
        case .emptyAnswer: "Der Server liefert keinen Text. Prüfe den Modellnamen und ob der Server Streaming unterstützt."
        case .transportBlocked: "iOS blockiert unverschlüsselte Verbindungen zu dieser Adresse. Nutze eine IP-Adresse, einen .local-Namen oder https."
        case .tlsFailed: "Die sichere Verbindung ist fehlgeschlagen. Das Zertifikat des Servers wird nicht akzeptiert."
        }
    }

    /// Anzeigetext zu jedem Fehler, den `AssistantEndpointModel.stream` oder die Prüfung wirft.
    static func describe(_ error: Error) -> String {
        if let error = error as? AssistantEndpointError { return error.errorDescription ?? "Die Anfrage ist fehlgeschlagen." }
        if let error = error as? BookTalkModelError {
            switch error {
            case .offline:
                return "Der Server ist nicht erreichbar. Prüfe WLAN, Adresse und Port und ob Local Voice in den iOS-Einstellungen auf das lokale Netzwerk zugreifen darf."
            case .timeout: return "Der Server antwortet nicht (nach 30 Sekunden ohne Daten)."
            case .contextOverflow: return "Die Anfrage ist für das Modell zu lang. Verkleinere die Kontextgröße oder vergrößere sie am Server."
            case .notAllowed: return "Die Freigabe für diesen Endpunkt fehlt."
            case .unavailable: return "Der Endpunkt ist nicht verfügbar."
            }
        }
        return "Die Anfrage ist fehlgeschlagen."
    }
}

struct AssistantEndpointModel: BookTalkModel {
    /// Zeit ohne Daten, bis eine Anfrage als gescheitert gilt (gilt auch für den Verbindungsaufbau, siehe Bericht).
    static let idleTimeout: TimeInterval = 30

    let endpoint: AssistantEndpoint
    private let apiKey: String?
    private let authorized: @Sendable () -> Bool

    /// `authorized` wird vor jeder Anfrage gefragt (Freigabe kann mitten im Gespräch widerrufen werden).
    init(endpoint: AssistantEndpoint, apiKey: String?, authorized: @escaping @Sendable () -> Bool = { true }) {
        self.endpoint = endpoint
        self.apiKey = apiKey
        self.authorized = authorized
    }

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let endpoint = endpoint, apiKey = apiKey, authorized = authorized
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard authorized() else { throw BookTalkModelError.notAllowed }
                    try await Self.run(endpoint: endpoint, apiKey: apiKey, instructions: instructions, prompt: prompt,
                                       maxTokens: maxTokens, into: continuation)
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: Self.mapped(error))
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private static func configuration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.requestCachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        configuration.waitsForConnectivity = false
        configuration.timeoutIntervalForRequest = idleTimeout
        configuration.timeoutIntervalForResource = 300
        return configuration
    }

    private static func run(endpoint: AssistantEndpoint, apiKey: String?, instructions: String, prompt: String, maxTokens: Int,
                            into continuation: AsyncThrowingStream<String, Error>.Continuation) async throws {
        guard let url = endpoint.chatURL else { throw AssistantEndpointError.invalidAddress }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalAndRemoteCacheData, timeoutInterval: idleTimeout)
        request.httpMethod = "POST"
        request.httpBody = try AssistantSSE.requestBody(model: endpoint.model, instructions: instructions, prompt: prompt, maxTokens: maxTokens)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        if let key = apiKey, !key.isEmpty { request.setValue("Bearer " + key, forHTTPHeaderField: "Authorization") }

        let session = URLSession(configuration: configuration())
        defer { session.invalidateAndCancel() }
        let (bytes, response) = try await session.bytes(for: request)
        guard let http = response as? HTTPURLResponse else { throw AssistantEndpointError.server(0) }
        guard (200..<300).contains(http.statusCode) else {
            // Nur für die Einordnung eines 400; der Text wird weder gespeichert noch angezeigt.
            var head = [UInt8]()
            if http.statusCode == 400 {
                for try await byte in bytes {
                    head.append(byte)
                    if head.count >= 4_096 { break }
                }
            }
            throw error(status: http.statusCode, body: String(decoding: head, as: UTF8.self))
        }

        var produced = false
        for try await line in bytes.lines {
            try Task.checkCancellation()
            if AssistantSSE.isDone(line) { break }
            guard let delta = AssistantSSE.delta(fromLine: line) else { continue }
            produced = true
            continuation.yield(delta)
        }
        if !produced { throw AssistantEndpointError.emptyAnswer }
    }

    /// HTTP-Status zu Fehler. Kontextüberlauf wird dem Kern als `contextOverflow` gemeldet.
    private static func error(status: Int, body: String) -> Error {
        switch status {
        case 401, 403: AssistantEndpointError.keyRejected
        case 404: AssistantEndpointError.notFound
        case 429: AssistantEndpointError.rateLimited
        default:
            AssistantSSE.isContextOverflow(status: status, body: body) ? BookTalkModelError.contextOverflow : AssistantEndpointError.server(status)
        }
    }

    /// Übersetzt Netzwerkfehler in Fehler, die der Kern kennt (`offline`, `timeout`) oder die verständlich anzeigbar sind.
    private static func mapped(_ error: Error) -> Error {
        guard let error = error as? URLError else { return error }
        switch error.code {
        case .cancelled: return CancellationError()
        case .timedOut: return BookTalkModelError.timeout
        case .notConnectedToInternet, .cannotConnectToHost, .cannotFindHost, .networkConnectionLost, .dnsLookupFailed,
             .dataNotAllowed, .internationalRoamingOff:
            return BookTalkModelError.offline
        case .appTransportSecurityRequiresSecureConnection: return AssistantEndpointError.transportBlocked
        case .secureConnectionFailed, .serverCertificateUntrusted, .serverCertificateHasBadDate, .serverCertificateHasUnknownRoot,
             .serverCertificateNotYetValid, .clientCertificateRejected, .clientCertificateRequired:
            return AssistantEndpointError.tlsFailed
        default: return error
        }
    }
}
