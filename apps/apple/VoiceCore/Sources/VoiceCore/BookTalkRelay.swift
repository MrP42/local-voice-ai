import Foundation

// Buch-Gespräch (Epic #307, Auftrag lva#320, Spec §11.3 und §11.7): Relay über den PC.
// Reine Foundation (Linux-Tests). Payload-Typen, Authentifizierung (HMAC-SHA-256, R2/R3), Fristen (R11),
// Eval-Status (§11.5) und der Streaming-Client über einen abstrakten Kanal. Hub, Schlüsselbund und
// Verschlüsselung der Hub-Objekte liegen in der App (`iPhone/BookTalk/BookTalkRelayHub.swift`).

public enum BookTalkRelay {
    public static let collection = "booktalk_relay"
    public static let version = 1
    public static let function = "book_talk"
    /// `expires_at = created_at + 90 s` (R11).
    public static let ttl = 90
    /// Keine gültige erste Antwort binnen 15 s: Abbruch, `found_only offline`.
    public static let firstResponseTimeout: TimeInterval = 15
    /// Insgesamt höchstens 75 s ab Absenden.
    public static let totalTimeout: TimeInterval = 75
    public static let pollInterval: TimeInterval = 1
    /// `max_tokens` ≤ 800 (R6).
    public static let maxTokens = 800
    /// Eigene Objekte räumt das iPhone nach 10 min ab (R5).
    public static let staleAfter = 600

    public enum Kind: String, Sendable { case models, req, resp }

    /// Objekt-ID im Hub. Das Hub-Muster `^[A-Za-z0-9_-]{1,64}$` erlaubt kein „/“: statt `req/<id>` heißt es `req_<id>`.
    public static func objectId(_ kind: Kind, _ id: String) -> String { "\(kind.rawValue)_\(id)" }
}

public enum BookTalkRelayProvider: String, Sendable, Codable, Equatable {
    case claudeCLI = "claude_cli"
    case codexCLI = "codex_cli"

    /// Empfänger der Daten hinter dem PC.
    public var recipient: String { self == .claudeCLI ? "Anthropic" : "OpenAI" }
    public var quotaName: String { self == .claudeCLI ? "Claude" : "ChatGPT" }
}

public struct BookTalkRelayModelInfo: Sendable, Codable, Equatable, Identifiable {
    public let id: String
    public let label: String
    public let provider: BookTalkRelayProvider
    public let contextTokens: Int

    public init(id: String, label: String, provider: BookTalkRelayProvider, contextTokens: Int) {
        self.id = id; self.label = label; self.provider = provider; self.contextTokens = contextTokens
    }

    enum CodingKeys: String, CodingKey { case id, label, provider, contextTokens = "context_tokens" }
}

/// `models_<desktop_device_id>`: `{v:1, models:[…], updated_at, desktop_device_id, device_id, issued_at, mac}`.
/// Der PC signiert die Liste je Gerät mit dem Relay-Schlüssel (Befund 12): unsignierte oder falsch signierte Listen zählen nicht.
public struct BookTalkRelayModelList: Sendable, Codable, Equatable {
    public let v: Int
    public let models: [BookTalkRelayModelInfo]
    public let updatedAt: Double?
    public let desktopDeviceId: String?
    public let deviceId: String?
    /// Unix-Sekunden.
    public let issuedAt: Int?
    /// HMAC-SHA-256 (hex) über `canonical`.
    public let mac: String?

    public init(v: Int = BookTalkRelay.version, models: [BookTalkRelayModelInfo], updatedAt: Double? = nil, desktopDeviceId: String? = nil,
                deviceId: String? = nil, issuedAt: Int? = nil, mac: String? = nil) {
        self.v = v; self.models = models; self.updatedAt = updatedAt; self.desktopDeviceId = desktopDeviceId
        self.deviceId = deviceId; self.issuedAt = issuedAt; self.mac = mac
    }

    enum CodingKeys: String, CodingKey {
        case v, models, updatedAt = "updated_at", desktopDeviceId = "desktop_device_id", deviceId = "device_id", issuedAt = "issued_at", mac
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        v = try c.decode(Int.self, forKey: .v)
        models = try c.decode([BookTalkRelayModelInfo].self, forKey: .models)
        updatedAt = try? c.decodeIfPresent(Double.self, forKey: .updatedAt)
        desktopDeviceId = try? c.decodeIfPresent(String.self, forKey: .desktopDeviceId)
        deviceId = try? c.decodeIfPresent(String.self, forKey: .deviceId)
        issuedAt = try? c.decodeIfPresent(Int.self, forKey: .issuedAt)
        mac = try? c.decodeIfPresent(String.self, forKey: .mac)
    }

    /// Kanonische Zeichenfolge (eine Stelle für Prüfung und Gegenstück): JSON, Schlüssel alphabetisch, ohne Leerraum;
    /// Einträge nach `id` sortiert, je `context_tokens`, `id`, `label`, `provider`.
    public static func canonical(models: [BookTalkRelayModelInfo], desktopDeviceId: String, deviceId: String, issuedAt: Int,
                                 version: Int = BookTalkRelay.version) -> String {
        let entries = models.sorted { Array($0.id.utf8).lexicographicallyPrecedes(Array($1.id.utf8)) }.map { m in
            BookTalkCanonical.Value.object([
                ("context_tokens", .int(m.contextTokens)), ("id", .string(m.id)), ("label", .string(m.label)),
                ("provider", .string(m.provider.rawValue)),
            ])
        }
        return BookTalkCanonical.object([
            ("desktop_device_id", .string(desktopDeviceId)), ("device_id", .string(deviceId)), ("issued_at", .int(issuedAt)),
            ("models", .array(entries)), ("v", .int(version)),
        ])
    }

    /// Nur mit gültigem MAC, passendem PC und diesem Gerät. Fehlt eines der signierten Felder, ist die Liste unsigniert.
    public func isAuthentic(key: [UInt8], desktopDeviceId expectedDesktop: String, deviceId expectedDevice: String) -> Bool {
        guard v == BookTalkRelay.version, let desktopDeviceId, let deviceId, let issuedAt, let mac,
              desktopDeviceId == expectedDesktop, deviceId == expectedDevice else { return false }
        let text = Self.canonical(models: models, desktopDeviceId: desktopDeviceId, deviceId: deviceId, issuedAt: issuedAt, version: v)
        return BookTalkHash.constantTimeEqual(BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(text.utf8))), mac)
    }

    /// Gegenstück des PC, für Tests.
    public static func signed(key: [UInt8], models: [BookTalkRelayModelInfo], desktopDeviceId: String, deviceId: String, issuedAt: Int,
                              updatedAt: Double? = nil) -> BookTalkRelayModelList {
        let text = canonical(models: models, desktopDeviceId: desktopDeviceId, deviceId: deviceId, issuedAt: issuedAt)
        return BookTalkRelayModelList(models: models, updatedAt: updatedAt, desktopDeviceId: desktopDeviceId, deviceId: deviceId,
                                      issuedAt: issuedAt, mac: BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(text.utf8))))
    }
}

public struct BookTalkRelayRequest: Sendable, Codable, Equatable {
    public let v: Int
    public let requestId: String
    public let deviceId: String
    public let keyId: String
    public let function: String
    public let modelId: String
    public let system: String
    public let prompt: String
    public let maxTokens: Int
    /// Unix-Sekunden (UTC).
    public let createdAt: Int
    public let expiresAt: Int
    /// HMAC-SHA-256 (hex) über `canonical` (R2).
    public let mac: String

    enum CodingKeys: String, CodingKey {
        case v, requestId = "request_id", deviceId = "device_id", keyId = "key_id", function, modelId = "model_id",
             system, prompt, maxTokens = "max_tokens", createdAt = "created_at", expiresAt = "expires_at", mac
    }

    /// Kanonische Metadaten (R2): JSON, Schlüssel alphabetisch, ohne Leerraum;
    /// Texte nur als SHA-256 (hex). Der PC bildet dieselbe Zeichenfolge.
    public static func canonical(
        requestId: String, deviceId: String, keyId: String, function: String, modelId: String, system: String, prompt: String,
        maxTokens: Int, createdAt: Int, expiresAt: Int, version: Int = BookTalkRelay.version
    ) -> String {
        BookTalkCanonical.object([
            ("created_at", .int(createdAt)), ("device_id", .string(deviceId)), ("expires_at", .int(expiresAt)),
            ("function", .string(function)), ("key_id", .string(keyId)), ("max_tokens", .int(maxTokens)),
            ("model_id", .string(modelId)), ("prompt_sha256", .string(BookTalkHash.sha256Hex(prompt))),
            ("request_id", .string(requestId)), ("system_sha256", .string(BookTalkHash.sha256Hex(system))), ("v", .int(version)),
        ])
    }

    public var canonical: String {
        Self.canonical(requestId: requestId, deviceId: deviceId, keyId: keyId, function: function, modelId: modelId, system: system,
                       prompt: prompt, maxTokens: maxTokens, createdAt: createdAt, expiresAt: expiresAt, version: v)
    }

    public static func make(
        key: [UInt8], keyId: String, deviceId: String, requestId: String, modelId: String, system: String, prompt: String,
        maxTokens: Int, now: Int
    ) -> BookTalkRelayRequest {
        let tokens = min(max(maxTokens, 1), BookTalkRelay.maxTokens)
        let expires = now + BookTalkRelay.ttl
        let text = canonical(requestId: requestId, deviceId: deviceId, keyId: keyId, function: BookTalkRelay.function, modelId: modelId,
                             system: system, prompt: prompt, maxTokens: tokens, createdAt: now, expiresAt: expires)
        return BookTalkRelayRequest(
            v: BookTalkRelay.version, requestId: requestId, deviceId: deviceId, keyId: keyId, function: BookTalkRelay.function,
            modelId: modelId, system: system, prompt: prompt, maxTokens: tokens, createdAt: now, expiresAt: expires,
            mac: BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(text.utf8))))
    }

    public func macIsValid(key: [UInt8]) -> Bool {
        BookTalkHash.constantTimeEqual(BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(canonical.utf8))), mac)
    }

    /// Prüfung, wie sie der PC macht (hier für Tests und Gegenprobe): MAC, Gerät, Funktion, Frist.
    public func verify(key: [UInt8], deviceId expectedDevice: String, now: Int, leeway: Int = 120) -> Bool {
        guard v == BookTalkRelay.version, function == BookTalkRelay.function, deviceId == expectedDevice,
              expiresAt == createdAt + BookTalkRelay.ttl, abs(now - createdAt) <= leeway, now < expiresAt else { return false }
        return macIsValid(key: key)
    }
}

/// Fehlercode des PCs. Als Zeichenkette gehalten (nicht als Enum), damit ein unbekannter Code den MAC nicht bricht:
/// der MAC deckt den Rohtext ab, die Abbildung auf Fehler der App steht in `BookTalkRelayModel.map`.
public struct BookTalkRelayError: RawRepresentable, Sendable, Codable, Equatable, Hashable {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }

    public static let modelUnavailable = BookTalkRelayError(rawValue: "model_unavailable")
    public static let timeout = BookTalkRelayError(rawValue: "timeout")
    public static let llmFailed = BookTalkRelayError(rawValue: "llm_failed")
    public static let notAllowed = BookTalkRelayError(rawValue: "not_allowed")
    /// Grenze je Gerät erreicht (R6).
    public static let limit = BookTalkRelayError(rawValue: "limit")
    /// Uhren von iPhone und PC weichen zu weit ab (R2).
    public static let clock = BookTalkRelayError(rawValue: "clock")

    public init(from decoder: Decoder) throws {
        rawValue = try decoder.singleValueContainer().decode(String.self)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }
}

public struct BookTalkRelayUsage: Sendable, Codable, Equatable {
    public let inputTokens: Int?
    public let outputTokens: Int?
    enum CodingKeys: String, CodingKey { case inputTokens = "input_tokens", outputTokens = "output_tokens" }

    public init(inputTokens: Int? = nil, outputTokens: Int? = nil) { self.inputTokens = inputTokens; self.outputTokens = outputTokens }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        inputTokens = try? c.decodeIfPresent(Int.self, forKey: .inputTokens)
        outputTokens = try? c.decodeIfPresent(Int.self, forKey: .outputTokens)
    }
}

public struct BookTalkRelayResponse: Sendable, Codable, Equatable {
    public let v: Int
    public let requestId: String
    public let deviceId: String
    public let seq: Int
    /// Tatsächlich verwendetes Modell laut CLI-Ausgabe (R7).
    public let modelTuple: String
    public let text: String
    public let done: Bool
    public let error: BookTalkRelayError?
    public let usage: BookTalkRelayUsage?
    /// HMAC-SHA-256 (hex) über `canonical` (R3).
    public let mac: String

    enum CodingKeys: String, CodingKey {
        case v, requestId = "request_id", deviceId = "device_id", seq, modelTuple = "model_tuple", text, done, error, usage, mac
    }

    public init(v: Int = BookTalkRelay.version, requestId: String, deviceId: String, seq: Int, modelTuple: String, text: String,
                done: Bool, error: BookTalkRelayError? = nil, usage: BookTalkRelayUsage? = nil, mac: String) {
        self.v = v; self.requestId = requestId; self.deviceId = deviceId; self.seq = seq; self.modelTuple = modelTuple
        self.text = text; self.done = done; self.error = error; self.usage = usage; self.mac = mac
    }

    public static func canonical(
        requestId: String, deviceId: String, seq: Int, modelTuple: String, text: String, done: Bool, error: BookTalkRelayError?,
        version: Int = BookTalkRelay.version
    ) -> String {
        BookTalkCanonical.object([
            ("device_id", .string(deviceId)), ("done", .bool(done)), ("error", .string(error?.rawValue ?? "")),
            ("model_tuple", .string(modelTuple)), ("request_id", .string(requestId)), ("seq", .int(seq)),
            ("text_sha256", .string(BookTalkHash.sha256Hex(text))), ("v", .int(version)),
        ])
    }

    public var canonical: String {
        Self.canonical(requestId: requestId, deviceId: deviceId, seq: seq, modelTuple: modelTuple, text: text, done: done,
                       error: error, version: v)
    }

    /// Antwort mit gültigem MAC (Gegenstück des PC, für Tests).
    public static func signed(
        key: [UInt8], requestId: String, deviceId: String, seq: Int, modelTuple: String, text: String, done: Bool,
        error: BookTalkRelayError? = nil, usage: BookTalkRelayUsage? = nil
    ) -> BookTalkRelayResponse {
        let c = canonical(requestId: requestId, deviceId: deviceId, seq: seq, modelTuple: modelTuple, text: text, done: done, error: error)
        return BookTalkRelayResponse(requestId: requestId, deviceId: deviceId, seq: seq, modelTuple: modelTuple, text: text, done: done,
                                     error: error, usage: usage, mac: BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(c.utf8))))
    }

    public func macIsValid(key: [UInt8]) -> Bool {
        BookTalkHash.constantTimeEqual(BookTalkHash.hex(BookTalkHash.hmacSHA256(key: key, message: Array(canonical.utf8))), mac)
    }
}

// MARK: - Einrichtung (R1)

/// Was der PC je Gerät erzeugt und das iPhone (QR-Code oder Einfügen) übernimmt. Der Schlüssel gehört genau diesem Gerät;
/// Konto-Schlüssel und Hub-Zugang genügen nie. Wird im Schlüsselbund gehalten, nie in `UserDefaults`.
public struct BookTalkRelayEnrollment: Sendable, Codable, Equatable {
    public let v: Int
    public let keyId: String
    public let key: [UInt8]
    public let desktopDeviceId: String
    public let desktopName: String
    /// Dieses iPhone laut PC (muss zur Hub-Geräte-ID des iPhones passen).
    public let deviceId: String
    /// Vom PC erlaubte Modelle; leer = alle veröffentlichten.
    public let modelIds: [String]
    /// Ablauf des Codes (ca. 10 min), Unix-Sekunden. Gilt nur für das Einlösen.
    public let expiresAt: Int
    /// Ablauf der Freigabe (Standard 90 Tage), Unix-Sekunden. Fehlt das Feld (älterer PC), gilt `expiresAt` auch für die Freigabe.
    public let grantExpiresAt: Int?

    enum CodingKeys: String, CodingKey {
        case v, keyId = "key_id", key = "k", desktopDeviceId = "desktop_device_id", desktopName = "desktop_name",
             deviceId = "device_id", modelIds = "model_ids", expiresAt = "expires_at", grantExpiresAt = "grant_expires_at"
    }

    public init(keyId: String, key: [UInt8], desktopDeviceId: String, desktopName: String, deviceId: String, modelIds: [String] = [],
                expiresAt: Int, grantExpiresAt: Int? = nil) {
        self.v = BookTalkRelay.version; self.keyId = keyId; self.key = key; self.desktopDeviceId = desktopDeviceId
        self.desktopName = desktopName; self.deviceId = deviceId; self.modelIds = modelIds; self.expiresAt = expiresAt
        self.grantExpiresAt = grantExpiresAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        v = try c.decode(Int.self, forKey: .v)
        keyId = try c.decode(String.self, forKey: .keyId)
        let raw = try c.decode(String.self, forKey: .key)
        guard let bytes = BookTalkBase64URL.decode(raw), bytes.count == 32 else {
            throw DecodingError.dataCorruptedError(forKey: .key, in: c, debugDescription: "Schlüssel")
        }
        key = bytes
        desktopDeviceId = try c.decode(String.self, forKey: .desktopDeviceId)
        desktopName = try c.decode(String.self, forKey: .desktopName)
        deviceId = try c.decode(String.self, forKey: .deviceId)
        modelIds = try c.decodeIfPresent([String].self, forKey: .modelIds) ?? []
        expiresAt = try c.decode(Int.self, forKey: .expiresAt)
        grantExpiresAt = try c.decodeIfPresent(Int.self, forKey: .grantExpiresAt)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(v, forKey: .v); try c.encode(keyId, forKey: .keyId)
        try c.encode(BookTalkBase64URL.encode(key), forKey: .key)
        try c.encode(desktopDeviceId, forKey: .desktopDeviceId); try c.encode(desktopName, forKey: .desktopName)
        try c.encode(deviceId, forKey: .deviceId); try c.encode(modelIds, forKey: .modelIds); try c.encode(expiresAt, forKey: .expiresAt)
        try c.encodeIfPresent(grantExpiresAt, forKey: .grantExpiresAt)
    }

    /// Text aus dem QR-Code: JSON, mit oder ohne Präfix `lvrelay1:`.
    public static func parse(_ text: String) -> BookTalkRelayEnrollment? {
        var s = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.hasPrefix("lvrelay1:") { s.removeFirst("lvrelay1:".count) }
        guard let data = s.data(using: .utf8), let e = try? JSONDecoder().decode(BookTalkRelayEnrollment.self, from: data),
              e.v == BookTalkRelay.version, !e.keyId.isEmpty, !e.desktopDeviceId.isEmpty else { return nil }
        return e
    }

    /// Der Code selbst ist abgelaufen (nur beim Einlösen prüfen).
    public func codeExpired(now: Int) -> Bool { now >= expiresAt }

    /// Die Freigabe ist abgelaufen: `grant_expires_at`, ohne dieses Feld `expires_at`.
    public func isExpired(now: Int) -> Bool { now >= (grantExpiresAt ?? expiresAt) }

    /// Modelle, die dieses Gerät benutzen darf (Schnitt aus der PC-Liste und der Einrichtung).
    public func allowed(_ models: [BookTalkRelayModelInfo]) -> [BookTalkRelayModelInfo] {
        modelIds.isEmpty ? models : models.filter { modelIds.contains($0.id) }
    }
}

// MARK: - Eval-Status (§11.5, R12)

public enum BookTalkEvalStatus: String, Sendable, Codable, Equatable {
    case unchecked = "ungeprueft"
    case passed = "bestanden"
    case failed = "durchgefallen"
}

public enum BookTalkVersions {
    public static let prompt = "book_talk_prompt@1"
    public static let check = "book_talk_check@1"
}

/// Tupel, an das ein Prüfergebnis gebunden ist.
public struct BookTalkEvalTuple: Sendable, Codable, Equatable, Hashable {
    public let provider: String
    public let model: String
    public let prompt: String
    public let check: String
    public let budget: String

    public init(provider: String, model: String, budget: String, prompt: String = BookTalkVersions.prompt, check: String = BookTalkVersions.check) {
        self.provider = provider; self.model = model; self.prompt = prompt; self.check = check; self.budget = budget
    }
}

/// `book_talk_eval.json`: Einträge `{tuple, status, report_sha256}`. Kein Eintrag = ungeprüft.
public struct BookTalkEvalTable: Sendable, Codable, Equatable {
    public struct Entry: Sendable, Codable, Equatable {
        public let tuple: BookTalkEvalTuple
        public let status: BookTalkEvalStatus
        public let reportSha256: String
        enum CodingKeys: String, CodingKey { case tuple, status, reportSha256 = "report_sha256" }
        public init(tuple: BookTalkEvalTuple, status: BookTalkEvalStatus, reportSha256: String) {
            self.tuple = tuple; self.status = status; self.reportSha256 = reportSha256
        }
    }

    public let entries: [Entry]
    public init(entries: [Entry] = []) { self.entries = entries }

    public static func load(_ data: Data?) -> BookTalkEvalTable {
        guard let data, let table = try? JSONDecoder().decode(BookTalkEvalTable.self, from: data) else { return BookTalkEvalTable() }
        return table
    }

    /// `apple-fm@1` ist unbewiesen und damit immer ungeprüft (R12); unbekannte Tupel sind ungeprüft;
    /// `durchgefallen` gewinnt vor `bestanden`.
    public func status(for tuple: BookTalkEvalTuple) -> BookTalkEvalStatus {
        if tuple.budget == BookTalkBudget.appleFM1.name { return .unchecked }
        let hits = entries.filter { $0.tuple == tuple }.map(\.status)
        if hits.contains(.failed) { return .failed }
        return hits.contains(.passed) ? .passed : .unchecked
    }
}

// MARK: - Streaming-Client

/// Transport für Relay-Objekte (Klartext-JSON). Die App verschlüsselt und spricht mit dem Hub; Tests nutzen einen Fake.
public protocol BookTalkRelayChannel: Sendable {
    func put(objectId: String, json: Data) async throws
    /// Aktueller Stand des Objekts, `nil` wenn es fehlt oder gelöscht ist.
    func fetch(objectId: String) async throws -> Data?
    func remove(objectId: String) async throws
}

/// Wo eine Relay-Anfrage steht (für die Anzeige).
public enum BookTalkRelayPhase: Sendable, Equatable {
    /// Anfrage liegt im Hub, noch kein Lebenszeichen.
    case sending
    /// Der PC hat sich gemeldet und rechnet (`pending`).
    case computing
    /// Antworttext kommt.
    case answering
}

/// Zuordnung Alias → erlaubte Modellfamilie (Befund 6). Kein „enthält“: ein Alias erlaubt nur seine Familie mit Versionsmuster.
public enum BookTalkRelayModelMatch {
    /// Modell der finalen Antwort, wenn der PC das tatsächliche Modell nicht ermitteln konnte: erlaubt, aber ungeprüft.
    public static let unknown = "unknown"
    /// Platzhalter ohne Text, solange der PC rechnet: Lebenszeichen, kein Modell.
    public static let pending = "pending"

    private static let families: [String: String] = ["sonnet": "claude-sonnet", "opus": "claude-opus", "haiku": "claude-haiku"]

    /// Exakter Treffer, oder Familien-/ID-Präfix gefolgt von `-<Ziffern>` (Version, Datum), nie Zusatzwörter wie `mini`.
    public static func accepts(alias: String, tuple: String) -> Bool {
        let t = tuple.trimmingCharacters(in: .whitespaces).lowercased()
        let a = alias.trimmingCharacters(in: .whitespaces).lowercased()
        guard !t.isEmpty, !a.isEmpty, t != unknown, t != pending else { return false }
        if t == a { return true }
        let base = families[a] ?? a
        guard t.hasPrefix(base + "-") else { return false }
        return isVersion(String(t.dropFirst(base.count + 1)))
    }

    /// `5-5`, `4-5-20250929`, `2025-11-01`, `1.2`: nur Ziffern (und Punkte darin), durch „-“ getrennt.
    static func isVersion(_ rest: String) -> Bool {
        let parts = rest.split(separator: "-", omittingEmptySubsequences: false)
        return !parts.isEmpty && parts.allSatisfy { part in
            guard let first = part.unicodeScalars.first, ("0"..."9").contains(first) else { return false }
            return part.unicodeScalars.allSatisfy { ("0"..."9").contains($0) || $0 == "." } && !part.hasSuffix(".")
        }
    }
}

/// Aufräumen eigener Relay-Objekte (R5, Befund 10) als reine Entscheidung; der Hub in der App führt sie aus.
public enum BookTalkRelayCleanup {
    public struct Object: Sendable {
        public let id: String
        public let json: Data
        /// Wann dieses iPhone das Objekt zum ersten Mal sah (Unix-Sekunden); `resp` trägt keinen eigenen Zeitstempel.
        public let firstSeen: Int
        public init(id: String, json: Data, firstSeen: Int) { self.id = id; self.json = json; self.firstSeen = firstSeen }
    }

    /// Objekt-IDs, die weg dürfen: eigene `req_` älter als 10 min, eigene `resp_` nur wenn ihre `request_id` nicht offen ist,
    /// kein zugehöriges junges `req_` steht und das iPhone sie seit mehr als 10 min kennt. Fremde Objekte nie.
    public static func stale(objects: [Object], deviceId: String, openRequestIds: Set<String>, now: Int) -> [String] {
        let decoder = JSONDecoder()
        var youngRequests = Set<String>()
        var staleRequests: [String] = []
        for o in objects where o.id.hasPrefix("req_") {
            guard let r = try? decoder.decode(BookTalkRelayRequest.self, from: o.json), r.deviceId == deviceId else { continue }
            if !openRequestIds.contains(r.requestId), now - r.createdAt > BookTalkRelay.staleAfter { staleRequests.append(o.id) } else { youngRequests.insert(r.requestId) }
        }
        var out = staleRequests
        for o in objects where o.id.hasPrefix("resp_") {
            guard let r = try? decoder.decode(BookTalkRelayResponse.self, from: o.json), r.deviceId == deviceId,
                  !openRequestIds.contains(r.requestId), !youngRequests.contains(r.requestId),
                  now - o.firstSeen > BookTalkRelay.staleAfter else { continue }
            out.append(o.id)
        }
        return out
    }
}

/// Modell hinter dem PC (R2, R3, R7, R10, R11). Liefert Rohtext; Belegprüfung und Spoilergrenze laufen danach im Kern (R8).
///
/// Antwortsemantik: Jede Antwort braucht gültigen MAC, passende `request_id`/`device_id` und steigendes `seq`.
/// - `pending` ohne Fehler und ohne `done`: Lebenszeichen („PC rechnet“). Setzt die 15-s-Frist zurück (Gesamtfrist 75 s bleibt),
///   kein Modell-Pinning, ein etwaiger Text wird nicht angezeigt (das Modell ist noch nicht bekannt).
/// - `error` mit `unknown` (oder dem angefragten Modell): Fehlermeldung des PCs, abgebildet in `map`.
/// - echtes Tupel: muss zum Alias passen (`BookTalkRelayModelMatch`), wird bei der ersten Antwort gepinnt; `durchgefallen` für dieses
///   Tupel verwirft die Antwort.
/// - `unknown` nur als finale Antwort (`done`), ohne vorheriges Pinning: erlaubt, aber ungeprüft.
public final class BookTalkRelayModel: BookTalkModel, @unchecked Sendable {
    private let info: BookTalkRelayModelInfo
    private let enrollment: BookTalkRelayEnrollment
    private let channel: any BookTalkRelayChannel
    private let authorized: @Sendable () -> Bool
    private let evalStatusOf: @Sendable (String) -> BookTalkEvalStatus
    private let now: @Sendable () -> Date
    private let sleep: @Sendable (TimeInterval) async throws -> Void
    private let makeId: @Sendable () -> String
    private let lock = NSLock()
    private var actual: String?
    private var actualStatus: BookTalkEvalStatus = .unchecked
    private var blocked = false
    private var handler: (@Sendable (BookTalkRelayPhase) -> Void)?

    /// - Parameters:
    ///   - authorized: Freigabe für dieses Modell steht noch (Widerruf bricht eine laufende Anfrage ab).
    ///   - evalStatus: Prüfstatus des tatsächlichen Modells (Tupel aus der Antwort), nie des Alias.
    public init(
        info: BookTalkRelayModelInfo, enrollment: BookTalkRelayEnrollment, channel: any BookTalkRelayChannel,
        authorized: @escaping @Sendable () -> Bool = { true },
        evalStatus: @escaping @Sendable (String) -> BookTalkEvalStatus = { _ in .unchecked },
        now: @escaping @Sendable () -> Date = { Date() },
        sleep: @escaping @Sendable (TimeInterval) async throws -> Void = { try await Task.sleep(nanoseconds: UInt64($0 * 1_000_000_000)) },
        makeId: @escaping @Sendable () -> String = { BookTalkRelayModel.randomId() }
    ) {
        self.info = info; self.enrollment = enrollment; self.channel = channel; self.authorized = authorized
        self.evalStatusOf = evalStatus; self.now = now; self.sleep = sleep; self.makeId = makeId
    }

    public static func randomId() -> String {
        var g = SystemRandomNumberGenerator()
        return (0..<16).map { _ in String(format: "%02x", UInt8.random(in: 0...255, using: &g)) }.joined()
    }

    /// Tatsächliches Modell der letzten Antwort (R7), für die Kopfzeile; `unknown`, wenn der PC es nicht ermitteln konnte.
    public var actualModel: String? { lock.lock(); defer { lock.unlock() }; return actual }

    /// Prüfstatus des tatsächlichen Modells der letzten Antwort; ohne Antwort oder bei `unknown` ungeprüft.
    public var actualEvalStatus: BookTalkEvalStatus { lock.lock(); defer { lock.unlock() }; return actualStatus }

    /// Die letzte Antwort wurde verworfen, weil das tatsächliche Modell den Prüflauf nicht bestanden hat.
    public var rejectedByEval: Bool { lock.lock(); defer { lock.unlock() }; return blocked }

    /// Meldet Wechsel von „gesendet“ zu „PC rechnet“ zu „Text kommt“ (Aufruf aus beliebigem Thread).
    public func setPhaseHandler(_ handler: (@Sendable (BookTalkRelayPhase) -> Void)?) {
        lock.lock(); self.handler = handler; lock.unlock()
    }

    public func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let task = Task { await self.run(instructions, prompt, maxTokens, continuation) }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func reset() {
        lock.lock(); actual = nil; actualStatus = .unchecked; blocked = false; lock.unlock()
    }

    private func setActual(_ tuple: String, _ status: BookTalkEvalStatus) {
        lock.lock(); actual = tuple; actualStatus = status; lock.unlock()
    }

    private func markBlocked(_ tuple: String) {
        lock.lock(); actual = tuple; actualStatus = .failed; blocked = true; lock.unlock()
    }

    private func announce(_ phase: BookTalkRelayPhase) {
        lock.lock(); let h = handler; lock.unlock()
        h?(phase)
    }

    private enum Verdict {
        case ignore
        case life
        case failure(Error)
        case reject
        case blockedByEval(String)
        case answer(tuple: String, status: BookTalkEvalStatus)
    }

    private func judge(_ r: BookTalkRelayResponse, pinned: String?) -> Verdict {
        let tuple = r.modelTuple
        let matches = BookTalkRelayModelMatch.accepts(alias: info.id, tuple: tuple)
        if let error = r.error {
            guard tuple == BookTalkRelayModelMatch.unknown || (matches && (pinned == nil || pinned == tuple)) else { return .reject }
            return .failure(Self.map(error))
        }
        if tuple == BookTalkRelayModelMatch.pending { return r.done ? .ignore : .life }
        if tuple == BookTalkRelayModelMatch.unknown {
            guard r.done, pinned == nil else { return .reject }
            return .answer(tuple: tuple, status: .unchecked)
        }
        guard matches, pinned == nil || pinned == tuple else { return .reject }
        let status = evalStatusOf(tuple)
        return status == .failed ? .blockedByEval(tuple) : .answer(tuple: tuple, status: status)
    }

    private func run(_ system: String, _ prompt: String, _ maxTokens: Int, _ out: AsyncThrowingStream<String, Error>.Continuation) async {
        let requestId = makeId()
        let reqId = BookTalkRelay.objectId(.req, requestId), respId = BookTalkRelay.objectId(.resp, requestId)
        reset()
        func cleanup() async { try? await channel.remove(objectId: reqId); try? await channel.remove(objectId: respId) }
        guard authorized(), !enrollment.isExpired(now: Int(now().timeIntervalSince1970)) else {
            out.finish(throwing: BookTalkModelError.notAllowed)
            return
        }
        let started = now()
        let request = BookTalkRelayRequest.make(
            key: enrollment.key, keyId: enrollment.keyId, deviceId: enrollment.deviceId, requestId: requestId, modelId: info.id,
            system: system, prompt: prompt, maxTokens: maxTokens, now: Int(started.timeIntervalSince1970))
        do {
            try await channel.put(objectId: reqId, json: JSONEncoder().encode(request))
        } catch {
            out.finish(throwing: BookTalkModelError.offline)
            return
        }
        announce(.sending)
        var lastSeq = -1
        var pinned: String?
        var sent = ""
        var gotReply = false
        var lastLife = started
        while true {
            if Task.isCancelled { await cleanup(); out.finish(throwing: CancellationError()); return }
            if !authorized() { await cleanup(); out.finish(throwing: CancellationError()); return }
            let current = now()
            if !gotReply, current.timeIntervalSince(lastLife) >= BookTalkRelay.firstResponseTimeout {
                await cleanup(); out.finish(throwing: BookTalkModelError.offline); return
            }
            if current.timeIntervalSince(started) >= BookTalkRelay.totalTimeout { await cleanup(); out.finish(throwing: BookTalkModelError.timeout); return }
            if let data = try? await channel.fetch(objectId: respId), let r = try? JSONDecoder().decode(BookTalkRelayResponse.self, from: data),
               r.v == BookTalkRelay.version, r.requestId == requestId, r.deviceId == enrollment.deviceId, r.seq > lastSeq,
               r.macIsValid(key: enrollment.key) {
                switch judge(r, pinned: pinned) {
                case .ignore:
                    break
                case .life:
                    lastSeq = r.seq
                    if !gotReply { lastLife = now(); announce(.computing) }
                case .failure(let error):
                    await cleanup()
                    out.finish(throwing: error)
                    return
                case .reject:
                    await cleanup()
                    out.finish(throwing: BookTalkModelError.unavailable)
                    return
                case .blockedByEval(let tuple):
                    markBlocked(tuple)
                    await cleanup()
                    out.finish(throwing: BookTalkModelError.unavailable)
                    return
                case .answer(let tuple, let status):
                    if !gotReply { announce(.answering) }
                    gotReply = true
                    lastSeq = r.seq
                    pinned = tuple == BookTalkRelayModelMatch.unknown ? nil : tuple
                    setActual(tuple, status)
                    let common = sent.commonPrefix(with: r.text)
                    let delta = r.text.dropFirst(common.count)
                    if !delta.isEmpty { out.yield(String(delta)) }
                    sent = r.text
                    if r.done { await cleanup(); out.finish(); return }
                }
            }
            do { try await sleep(BookTalkRelay.pollInterval) } catch { await cleanup(); out.finish(throwing: CancellationError()); return }
        }
    }

    /// Fehlercode des PCs → Fehler der App (und damit `BookTalkCode`): `not_allowed` → Freigabe fehlt, `model_unavailable` → kein Modell,
    /// `timeout` und `clock` (Uhrabweichung) → Zeitrahmen, `limit`, `llm_failed` und Unbekanntes → Modell fehlgeschlagen.
    static func map(_ e: BookTalkRelayError) -> Error {
        switch e.rawValue {
        case BookTalkRelayError.modelUnavailable.rawValue: BookTalkModelError.unavailable
        case BookTalkRelayError.timeout.rawValue, BookTalkRelayError.clock.rawValue: BookTalkModelError.timeout
        case BookTalkRelayError.notAllowed.rawValue: BookTalkModelError.notAllowed
        default: BookTalkRelayFailure.llmFailed
        }
    }
}

public enum BookTalkRelayFailure: Error, Equatable { case llmFailed }

// MARK: - Hilfen: kanonisches JSON, Base64url, SHA-256, HMAC

enum BookTalkCanonical {
    indirect enum Value { case string(String), int(Int), bool(Bool), array([Value]), object([(String, Value)]) }

    /// Reihenfolge ist die des Aufrufers (alphabetisch).
    static func object(_ pairs: [(String, Value)]) -> String {
        "{" + pairs.map { quote($0.0) + ":" + render($0.1) }.joined(separator: ",") + "}"
    }

    private static func render(_ value: Value) -> String {
        switch value {
        case .string(let s): quote(s)
        case .int(let n): String(n)
        case .bool(let b): b ? "true" : "false"
        case .array(let items): "[" + items.map(render).joined(separator: ",") + "]"
        case .object(let pairs): object(pairs)
        }
    }

    static func quote(_ s: String) -> String {
        var out = "\""
        for u in s.unicodeScalars {
            switch u {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            default:
                if u.value < 0x20 { out += String(format: "\\u%04x", u.value) } else { out.unicodeScalars.append(u) }
            }
        }
        return out + "\""
    }
}

public enum BookTalkBase64URL {
    public static func decode(_ s: String) -> [UInt8]? {
        var b = s.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while b.count % 4 != 0 { b += "=" }
        return Data(base64Encoded: b).map { [UInt8]($0) }
    }

    public static func encode(_ bytes: [UInt8]) -> String {
        Data(bytes).base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// SHA-256 und HMAC ohne CryptoKit (Linux-Tests); gegen RFC-Vektoren geprüft.
public enum BookTalkHash {
    private static let k: [UInt32] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ]

    public static func sha256(_ message: [UInt8]) -> [UInt8] {
        var h: [UInt32] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
        var data = message
        data.append(0x80)
        while data.count % 64 != 56 { data.append(0) }
        let bits = UInt64(message.count) * 8
        for i in (0..<8).reversed() { data.append(UInt8((bits >> UInt64(i * 8)) & 0xff)) }
        func rotr(_ x: UInt32, _ n: UInt32) -> UInt32 { (x >> n) | (x << (32 - n)) }
        for chunk in stride(from: 0, to: data.count, by: 64) {
            var w = [UInt32](repeating: 0, count: 64)
            for i in 0..<16 {
                let j = chunk + i * 4
                w[i] = UInt32(data[j]) << 24 | UInt32(data[j + 1]) << 16 | UInt32(data[j + 2]) << 8 | UInt32(data[j + 3])
            }
            for i in 16..<64 {
                let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3)
                let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10)
                w[i] = w[i - 16] &+ s0 &+ w[i - 7] &+ s1
            }
            var a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7]
            for i in 0..<64 {
                let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)
                let ch = (e & f) ^ (~e & g)
                let t1 = hh &+ s1 &+ ch &+ k[i] &+ w[i]
                let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
                let maj = (a & b) ^ (a & c) ^ (b & c)
                let t2 = s0 &+ maj
                hh = g; g = f; f = e; e = d &+ t1; d = c; c = b; b = a; a = t1 &+ t2
            }
            h[0] = h[0] &+ a; h[1] = h[1] &+ b; h[2] = h[2] &+ c; h[3] = h[3] &+ d
            h[4] = h[4] &+ e; h[5] = h[5] &+ f; h[6] = h[6] &+ g; h[7] = h[7] &+ hh
        }
        return h.flatMap { [UInt8($0 >> 24), UInt8(($0 >> 16) & 0xff), UInt8(($0 >> 8) & 0xff), UInt8($0 & 0xff)] }
    }

    public static func hmacSHA256(key: [UInt8], message: [UInt8]) -> [UInt8] {
        var k = key.count > 64 ? sha256(key) : key
        k += [UInt8](repeating: 0, count: 64 - k.count)
        let inner = sha256(k.map { $0 ^ 0x36 } + message)
        return sha256(k.map { $0 ^ 0x5c } + inner)
    }

    public static func hex(_ bytes: [UInt8]) -> String { bytes.map { String(format: "%02x", $0) }.joined() }
    public static func sha256Hex(_ text: String) -> String { hex(sha256(Array(text.utf8))) }

    public static func constantTimeEqual(_ a: String, _ b: String) -> Bool {
        let x = Array(a.utf8), y = Array(b.utf8)
        guard x.count == y.count else { return false }
        var diff: UInt8 = 0
        for i in x.indices { diff |= x[i] ^ y[i] }
        return diff == 0
    }
}
