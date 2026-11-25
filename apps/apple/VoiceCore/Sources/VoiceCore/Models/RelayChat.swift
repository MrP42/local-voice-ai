import Foundation
import CoreFoundation

// Gemeinsamer PC-Relay-Vertrag (#801 T4c, #804). Alle Felder ausser mac sind signiert.
// Dieselbe verschluesselte Sammlung wie BookTalk, eigene Objekt-Namen.
public enum ModelChat {
    public static let function = "model_chat"
    public static let maxBytes = 180_000
    public static func objectId(_ kind: String, _ id: String) -> String { "model_\(kind)_\(id)" }
    public static func modelsId(_ enrollment: BookTalkRelayEnrollment) -> String {
        objectId("models", "\(enrollment.desktopDeviceId)_\(BookTalkRelay.deviceKey(enrollment.deviceId))")
    }
}

public struct ModelChatModel: Codable, Equatable, Sendable {
    public let provider: String
    public let model: String
    public let label: String
    public let vendor: String
    public let region: String?
    public let contextTokens: Int
    public let supportsStreaming: Bool
    public let external: Bool
    enum CodingKeys: String, CodingKey {
        case provider, model, label, vendor, region, external
        case contextTokens = "context_tokens", supportsStreaming = "supports_streaming"
    }
    public var choice: ModelChoice {
        ModelChoice(route: .pcRelay(provider: provider, model: model), label: label, vendor: vendor,
                    contextTokens: contextTokens, supportsStreaming: supportsStreaming, region: region)
    }
}

public struct ModelChatInput: Codable, Sendable {
    public let provider: String
    public let model: String
    public let purpose: ModelPurpose
    public let messages: [ModelMessage]
    public let maxTokens: Int
    public let temperatureMilli: Int
    public let jsonSchema: String?
    enum CodingKeys: String, CodingKey {
        case provider, model, purpose, messages
        case maxTokens = "max_tokens", temperatureMilli = "temperature_milli", jsonSchema = "json_schema"
    }
}

public struct ModelChatChunk: Codable, Equatable, Sendable {
    public let seq: Int
    public let text: String
    public init(seq: Int, text: String) { self.seq = seq; self.text = text }
}
public enum ModelChatError: String, Codable, Sendable {
    case notAllowed = "not_allowed", modelUnavailable = "model_unavailable", limit, clock, timeout, cancelled, provider
    var failure: ModelFailure {
        switch self {
        case .notAllowed: return .blockedByPolicy
        case .modelUnavailable: return .unavailable
        case .limit: return .provider(.rateLimited)
        case .clock, .timeout: return .timeout
        case .cancelled: return .cancelled
        case .provider: return .provider(.server)
        }
    }
}
public struct ModelChatUsage: Codable, Sendable {
    public let inputTokens: Int
    public let outputTokens: Int
    enum CodingKeys: String, CodingKey { case inputTokens = "input_tokens", outputTokens = "output_tokens" }
}
public struct ModelChatWire: Codable, Sendable {
    public enum Kind: String, Codable, Sendable { case models, req, resp }
    public var v = 1
    public var function = ModelChat.function
    public var kind: Kind
    public var requestId: String
    public var deviceId: String
    public var keyId: String
    public var createdAt: Int
    public var expiresAt: Int
    public var desktopDeviceId: String?
    public var models: [ModelChatModel]?
    public var request: ModelChatInput?
    public var chunks: [ModelChatChunk]?
    public var done: Bool?
    public var error: ModelChatError?
    public var usage: ModelChatUsage?
    public var mac = ""
    enum CodingKeys: String, CodingKey {
        case v, function, kind, models, request, chunks, done, error, usage, mac
        case requestId = "request_id", deviceId = "device_id", keyId = "key_id", createdAt = "created_at"
        case expiresAt = "expires_at", desktopDeviceId = "desktop_device_id"
    }

    public func canonical() throws -> String {
        let data = try JSONEncoder().encode(self)
        guard var object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw ModelFailure.provider(.invalidResponse) }
        object.removeValue(forKey: "mac")
        return try Self.canonicalValue(object)
    }
    private static func canonicalValue(_ value: Any) throws -> String {
        if let dict = value as? [String: Any] {
            return "{" + (try dict.keys.sorted { Array($0.utf8).lexicographicallyPrecedes(Array($1.utf8)) }.map {
                BookTalkCanonical.quote($0) + ":" + (try canonicalValue(dict[$0]!))
            }).joined(separator: ",") + "}"
        }
        if let list = value as? [Any] { return "[" + (try list.map(canonicalValue)).joined(separator: ",") + "]" }
        if let string = value as? String { return BookTalkCanonical.quote(string) }
        if let number = value as? NSNumber {
            if CFGetTypeID(number) == CFBooleanGetTypeID() { return number.boolValue ? "true" : "false" }
            return number.stringValue
        }
        if value is NSNull { return "null" }
        throw ModelFailure.provider(.invalidResponse)
    }
    public mutating func sign(key: [UInt8]) throws {
        mac = BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(try canonical().utf8)))
    }
    public func isAuthentic(key: [UInt8]) throws -> Bool {
        guard v == 1, function == ModelChat.function else { return false }
        return BookTalkHash.constantTimeEqual(BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(try canonical().utf8))), mac)
    }
    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= ModelChat.maxBytes else { throw ModelFailure.contextExceeded }
        return try JSONDecoder().decode(Self.self, from: data)
    }
    public func isCurrent(now: Int) -> Bool {
        let expiry = createdAt.addingReportingOverflow(90)
        return !expiry.overflow && expiry.partialValue == expiresAt && now < expiresAt
            && createdAt <= now.addingReportingOverflow(120).partialValue
    }
}

public struct ModelChatDeadline: Sendable {
    public let start: TimeInterval
    public init(start: TimeInterval) { self.start = start }
    public func expired(now: TimeInterval, received: Bool) -> Bool { now - start >= (received ? 75 : 15) }
}

public enum ModelChatTimeout {
    public static func run<T: Sendable>(seconds: TimeInterval,
        operation: @escaping @Sendable () async throws -> T) async throws -> T {
        guard seconds > 0 else { throw ModelFailure.timeout }
        return try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await operation() }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(min(seconds, 75) * 1_000_000_000))
                throw ModelFailure.timeout
            }
            defer { group.cancelAll() }
            return try await group.next()!
        }
    }
}

/// Jeder Zwischenstand enthaelt alle bisherigen nummerierten Stuecke. Verlorene Polls sind erlaubt,
/// fehlende, doppelte, umsortierte oder nachtraeglich geaenderte Stuecke niemals.
public struct ModelChatCursor: Sendable {
    private var seen: [ModelChatChunk] = []
    private var finished = false
    public init() {}
    public func validated(_ response: ModelChatWire) throws -> [ModelChatChunk] {
        guard let chunks = response.chunks, chunks.count <= 1000 else { throw ModelFailure.provider(.invalidResponse) }
        for (index, chunk) in chunks.enumerated() {
            guard chunk.seq == index + 1 else { throw ModelFailure.provider(.invalidResponse) }
        }
        guard chunks.count >= seen.count, Array(chunks.prefix(seen.count)) == seen else { throw ModelFailure.provider(.invalidResponse) }
        return chunks
    }
    public mutating func accept(_ response: ModelChatWire) throws -> [ModelEvent] {
        if let error = response.error { throw error.failure }
        let chunks = try validated(response)
        if finished { return [] }
        var events = chunks.dropFirst(seen.count).map { ModelEvent.delta($0.text) }
        seen = chunks
        if response.done == true {
            finished = true
            events.append(.done(response.usage.map { ModelUsage(inputTokens: $0.inputTokens, outputTokens: $0.outputTokens) }))
        }
        return events
    }
}

/// Vorbereitung darf warten (Verschluesselung, Revision, Konto); start beginnt den Upload synchron.
public protocol ModelChatUpload: Sendable {
    func start() -> AsyncThrowingStream<Void, Error>
}
public protocol ModelChatChannel: BookTalkRelayChannel {
    func prepare(objectId: String, json: Data) async throws -> any ModelChatUpload
}

public struct RelayChatProvider: ModelProvider {
    public let route: ModelRoute
    public let enrollment: BookTalkRelayEnrollment
    public let channel: any ModelChatChannel
    public init(route: ModelRoute, enrollment: BookTalkRelayEnrollment, channel: any ModelChatChannel) {
        self.route = route; self.enrollment = enrollment; self.channel = channel
    }
    public func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                let id = UUID().uuidString.lowercased()
                let reqId = ModelChat.objectId("req", id), respId = ModelChat.objectId("resp", id)
                func cleanup() async {
                    // Eigene, unabhaengige Task: auch nach Abbruch muss geloescht werden.
                    await Task.detached {
                        try? await ModelChatTimeout.run(seconds: 10) { try await channel.remove(objectId: reqId) }
                        try? await ModelChatTimeout.run(seconds: 10) { try await channel.remove(objectId: respId) }
                    }.value
                }
                do {
                    guard case .pcRelay(let provider, let model) = route, !request.purpose.localOnly,
                          request.maxTokens > 0, request.maxTokens <= request.purpose.maxOutputTokens,
                          request.temperature.isFinite, (0...2).contains(request.temperature),
                          !enrollment.isExpired(now: Int(Date().timeIntervalSince1970)) else { throw ModelFailure.unavailable }
                    let now = Int(Date().timeIntervalSince1970)
                    var wire = ModelChatWire(kind: .req, requestId: id, deviceId: enrollment.deviceId,
                        keyId: enrollment.keyId, createdAt: now, expiresAt: now + 90,
                        request: ModelChatInput(provider: provider, model: model, purpose: request.purpose,
                            messages: request.messages, maxTokens: request.maxTokens,
                            temperatureMilli: Int((request.temperature * 1000).rounded()), jsonSchema: request.jsonSchema))
                    try wire.sign(key: enrollment.key)
                    let data = try JSONEncoder().encode(wire)
                    guard data.count <= ModelChat.maxBytes else { throw ModelFailure.contextExceeded }
                    let deadline = ModelChatDeadline(start: ProcessInfo.processInfo.systemUptime)
                    let upload = try await ModelChatTimeout.run(seconds: 15) { try await channel.prepare(objectId: reqId, json: data) }
                    try Task.checkCancellation()
                    guard !enrollment.isExpired(now: Int(Date().timeIntervalSince1970)) else { throw ModelFailure.unavailable }
                    try await ModelProviderCheck.confirm(self, request, authorization)
                    let sent = upload.start()
                    try await ModelChatTimeout.run(seconds: 15 - (ProcessInfo.processInfo.systemUptime - deadline.start)) {
                        for try await _ in sent { try Task.checkCancellation() }
                    }
                    var received = false, cursor = ModelChatCursor()
                    while true {
                        try Task.checkCancellation()
                        guard !deadline.expired(now: ProcessInfo.processInfo.systemUptime, received: received) else { throw ModelFailure.timeout }
                        let remaining = (received ? 75.0 : 15.0) - (ProcessInfo.processInfo.systemUptime - deadline.start)
                        if let data = try await ModelChatTimeout.run(seconds: remaining, operation: { try await channel.fetch(objectId: respId) }) {
                            let response = try ModelChatWire.decode(data)
                            guard response.kind == .resp, response.requestId == id,
                                  response.deviceId == enrollment.deviceId, response.keyId == enrollment.keyId,
                                  response.createdAt == wire.createdAt, response.expiresAt == wire.expiresAt,
                                  try response.isAuthentic(key: enrollment.key) else { throw ModelFailure.provider(.invalidResponse) }
                            received = true
                            for event in try cursor.accept(response) { continuation.yield(event) }
                            if response.done == true { break }
                        }
                        try await Task.sleep(nanoseconds: 1_000_000_000)
                    }
                    continuation.finish()
                    await cleanup()
                } catch {
                    continuation.finish(throwing: ModelFailure.sanitized(error))
                    await cleanup()
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
