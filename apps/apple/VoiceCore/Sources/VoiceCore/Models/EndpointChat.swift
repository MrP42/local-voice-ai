import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

// Reiner Endpunktkern: gleicher OpenAI-kompatibler Anfragebau wie Sprechen, kein Schlüssel im Zustand.
public enum EndpointTransportEvent: Sendable {
    case response(status: Int, url: URL, isSSE: Bool)
    case data(Data)
}

public protocol EndpointChatTransport: Sendable {
    /// Startet synchron. Der Abbruch des zurückgegebenen Stroms muss die Anfrage beenden.
    func start(_ request: URLRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<EndpointTransportEvent, Error>
}

public struct EndpointChatProvider: ModelProvider {
    public let route: ModelRoute
    private let url: URL
    private let model: String
    private let transport: any EndpointChatTransport
    private let key: @Sendable () async throws -> String?

    public init(route: ModelRoute, url: URL, model: String, transport: any EndpointChatTransport,
                key: @escaping @Sendable () async throws -> String?) {
        self.route = route; self.url = url; self.model = model; self.transport = transport; self.key = key
    }

    /// Zusätzliche Grenze für Transporte; die App folgt vorsorglich gar keinen Umleitungen.
    public static func permitsRedirect(from: URL, to: URL) -> Bool {
        from.host?.lowercased() == to.host?.lowercased()
            && from.scheme?.lowercased() == to.scheme?.lowercased()
            && ModelGate.port(from) == ModelGate.port(to)
            && to.user == nil && to.password == nil
    }

    public func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let apiKey = try await key()
                    guard authorization.permitsSending(to: url) else { throw ModelFailure.consentMissing }
                    var wire = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalAndRemoteCacheData, timeoutInterval: 30)
                    wire.httpMethod = "POST"
                    wire.httpBody = try AssistantSSE.requestBody(model: model, request: request)
                    wire.setValue("application/json", forHTTPHeaderField: "Content-Type")
                    wire.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    if let apiKey, !apiKey.isEmpty {
                        guard apiKey.unicodeScalars.allSatisfy({ (0x21...0x7e).contains($0.value) }) else {
                            throw ModelFailure.provider(.unauthorized)
                        }
                        wire.setValue("Bearer " + apiKey, forHTTPHeaderField: "Authorization")
                    }
                    try await ModelProviderCheck.confirm(self, request, authorization)
                    let events = transport.start(wire, authorization: authorization)
                    var parser: EndpointChatParser?
                    for try await event in events {
                        try Task.checkCancellation()
                        switch event {
                        case .response(let status, let actual, let isSSE):
                            guard parser == nil, authorization.permitsSending(to: actual) else { throw ModelFailure.consentMissing }
                            guard (200..<300).contains(status) else {
                                throw status == 413 ? ModelFailure.contextExceeded : .provider(ModelProviderError(status: status))
                            }
                            parser = EndpointChatParser(isSSE: isSSE)
                        case .data(let data):
                            guard var active = parser else { throw ModelFailure.provider(.invalidResponse) }
                            for piece in try active.append(data) { continuation.yield(piece) }
                            parser = active
                            if active.completed { continuation.finish(); return }
                        }
                    }
                    guard var active = parser else { throw ModelFailure.provider(.invalidResponse) }
                    for piece in try active.finish() { continuation.yield(piece) }
                    continuation.finish()
                } catch let error as URLError where error.code == .timedOut {
                    continuation.finish(throwing: ModelFailure.timeout)
                } catch { continuation.finish(throwing: ModelFailure.sanitized(error)) }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

/// Byteweiser Parser: UTF-8 erst nach vollständiger Zeile dekodieren, abgeschnittene Antworten nie abschließen.
public struct EndpointChatParser: Sendable {
    private let isSSE: Bool
    private var buffer = Data()
    private var total = 0
    private var produced = false
    private var usage: ModelUsage?
    public private(set) var completed = false
    public init(isSSE: Bool) { self.isSSE = isSSE }

    public mutating func append(_ data: Data) throws -> [ModelEvent] {
        guard !completed else { return [] }
        total += data.count
        guard total <= 1_048_576 else { throw ModelFailure.provider(.invalidResponse) }
        buffer.append(data)
        guard buffer.count <= 262_144 else { throw ModelFailure.provider(.invalidResponse) }
        guard isSSE else { return [] }
        var events: [ModelEvent] = []
        while let newline = buffer.firstIndex(of: 10) {
            let line = Data(buffer[..<newline]); buffer.removeSubrange(...newline)
            events += try parseLine(line)
            if completed { break }
        }
        return events
    }

    private mutating func parseLine(_ data: Data) throws -> [ModelEvent] {
        guard let line = String(data: data, encoding: .utf8) else { throw ModelFailure.provider(.invalidResponse) }
        if AssistantSSE.isDone(line) {
            guard produced else { throw ModelFailure.provider(.invalidResponse) }
            completed = true; return [.done(usage)]
        }
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("data:") else { return [] }
        let payload = String(trimmed.dropFirst(5)).trimmingCharacters(in: .whitespaces)
        let object = try object(Data(payload.utf8))
        try validate(object)
        if let delta = AssistantSSE.delta(fromLine: line) { produced = true; return [.delta(delta)] }
        return []
    }

    private func object(_ data: Data) throws -> [String: Any] {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw ModelFailure.provider(.invalidResponse)
        }
        return object
    }

    private mutating func validate(_ object: [String: Any]) throws {
        guard object["error"] == nil, let choices = object["choices"] as? [[String: Any]] else {
            throw ModelFailure.provider(.invalidResponse)
        }
        if choices.contains(where: { ($0["finish_reason"] as? String) == "length" }) { throw ModelFailure.contextExceeded }
        if choices.contains(where: { ($0["finish_reason"] as? String) == "content_filter" }) {
            throw ModelFailure.provider(.invalidResponse)
        }
        if let tokens = object["usage"] as? [String: Any] {
            usage = ModelUsage(inputTokens: tokens["prompt_tokens"] as? Int, outputTokens: tokens["completion_tokens"] as? Int)
        }
    }

    public mutating func finish() throws -> [ModelEvent] {
        if completed { return [] }
        if isSSE {
            let remaining = buffer; buffer.removeAll()
            let events = try parseLine(remaining)
            guard completed else { throw ModelFailure.provider(.invalidResponse) }
            return events
        }
        let object = try object(buffer)
        try validate(object)
        guard let choices = object["choices"] as? [[String: Any]],
              let message = choices.first?["message"] as? [String: Any],
              let text = message["content"] as? String, !text.isEmpty else { throw ModelFailure.provider(.invalidResponse) }
        completed = true
        return [.delta(text), .done(usage)]
    }
}
