import Foundation
#if canImport(CryptoKit)
import CryptoKit

/// Geräte-Kopplung, Seite iPhone (P4), nach
/// docs/superpowers/specs/2026-11-05-geraete-kopplung-qr.md, Abschnitte 4 und 5.3.
/// Hier steht nur Logik ohne Oberfläche, damit sie gegen die Testvektoren und
/// einen nachgebauten Desktop laufen kann.

public enum PairingError: LocalizedError, Equatable {
    case invalidCode, outdatedCode, insecureHub
    case expired, rejected, nonceChanged, commitmentFailed, nameRejected
    case badResponse, openFailed, keyMismatch, offline, timedOut, declined, tooManyAttempts, serverFailed
    case adoptFailed(String)

    public var errorDescription: String? {
        switch self {
        case .invalidCode: return "Das ist kein Kopplungscode der Desktop-App."
        case .outdatedCode: return "Dieser Code stammt von einer anderen Version. Bitte Desktop-App und iPhone-App aktualisieren."
        case .insecureHub: return "Der Code nennt einen Server ohne https. Aus Sicherheitsgründen abgebrochen."
        case .expired: return "Code abgelaufen oder schon verwendet. Bitte am PC einen neuen Code anzeigen."
        case .rejected: return "Am PC abgelehnt."
        case .nonceChanged: return "Der Server hat die Kopplung unterwegs verändert. Aus Sicherheitsgründen abgebrochen, bitte neu beginnen."
        case .commitmentFailed: return "Die Kopplung wurde aus Sicherheitsgründen abgebrochen. Bitte neu beginnen."
        case .nameRejected: return "Der Server lehnt diesen Gerätenamen ab. Bitte einen einfacheren Namen wählen."
        case .badResponse: return "Der Server antwortet unerwartet. Bitte neu beginnen."
        case .openFailed: return "Der Schlüssel vom PC ließ sich nicht öffnen. Am PC dieses Gerät abmelden und neu beginnen."
        case .keyMismatch: return "Der Schlüssel vom PC passt nicht zum Konto. Am PC dieses Gerät abmelden und neu beginnen."
        case .offline: return "Keine Verbindung zum Server. Bitte am PC einen neuen Code anzeigen."
        case .timedOut: return "Der PC hat nicht rechtzeitig geantwortet. Bitte neu beginnen."
        case .declined: return "Kopplung abgebrochen."
        case .tooManyAttempts: return "Zu viele Versuche aus diesem Netz. Bitte in einer Viertelstunde erneut versuchen."
        case .serverFailed: return "Der Server hat gerade ein Problem. Bitte gleich am PC einen neuen Code anzeigen."
        case .adoptFailed(let why): return why
        }
    }
}

/// Bausteine aus Abschnitt 4.1. `id` geht als ASCII-Text in alle Hashes ein.
public enum PairingProto {
    private static func sha(_ parts: [[UInt8]]) -> [UInt8] {
        var hash = SHA256()
        for part in parts { hash.update(data: part) }
        return Array(hash.finalize())
    }

    private static func hmac(_ key: [UInt8], _ parts: [[UInt8]]) -> [UInt8] {
        var mac = HMAC<SHA256>(key: SymmetricKey(data: key))
        for part in parts { mac.update(data: part) }
        return Array(mac.finalize())
    }

    private static func text(_ value: String) -> [UInt8] { Array(value.utf8) }

    public static func b64u(_ bytes: [UInt8]) -> String {
        Data(bytes).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    /// Nur kanonisches base64url ohne Padding, auch die Endbits müssen stimmen.
    private static func canonical(_ value: String, chars: Int, bytes: Int) -> [UInt8]? {
        guard value.utf8.count == chars,
              value.utf8.allSatisfy({ c in
                  (c >= 0x30 && c <= 0x39) || (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A) || c == 0x2D || c == 0x5F
              }) else { return nil }
        var plain = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        plain += String(repeating: "=", count: (4 - plain.count % 4) % 4)
        guard let raw = Data(base64Encoded: plain), raw.count == bytes else { return nil }
        let out = [UInt8](raw)
        return b64u(out) == value ? out : nil
    }

    public static func b64u32(_ value: String) -> [UInt8]? { canonical(value, chars: 43, bytes: 32) }
    public static func validPairId(_ id: String) -> Bool { canonical(id, chars: 22, bytes: 16) != nil }

    public static func commitment(id: String, phonePublic: [UInt8], phoneNonce: [UInt8], name: String) -> [UInt8] {
        let bytes = text(name)
        return sha([text("LV-PAIR-COMMIT-v2"), text(id), phonePublic, phoneNonce,
                    [UInt8(truncatingIfNeeded: bytes.count >> 8), UInt8(truncatingIfNeeded: bytes.count)], bytes])
    }

    public static func claimMac(q: [UInt8], id: String, commitment: [UInt8]) -> [UInt8] {
        hmac(q, [text("LV-PAIR-CLAIM-v2"), text(id), commitment])
    }

    /// Sechs Ziffern, mit führenden Nullen.
    public static func sas(id: String, desktopPublic: [UInt8], phonePublic: [UInt8], phoneNonce: [UInt8], desktopNonce: [UInt8]) -> String {
        let digest = sha([text("LV-PAIR-SAS-v2"), text(id), desktopPublic, phonePublic, phoneNonce, desktopNonce])
        let value = digest[0..<4].reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
        return String(format: "%06u", value % 1_000_000)
    }

    /// X25519 mit Prüfung auf den Nullpunkt (RFC 7748 §6.1), in konstanter Zeit.
    public static func shared(_ secret: Curve25519.KeyAgreement.PrivateKey, desktopPublic: [UInt8]) throws -> [UInt8] {
        do {
            let peer = try Curve25519.KeyAgreement.PublicKey(rawRepresentation: desktopPublic)
            let z = try secret.sharedSecretFromKeyAgreement(with: peer).withUnsafeBytes { Array($0) }
            guard z.count == 32, z.reduce(UInt8(0), |) != 0 else { throw PairingError.openFailed }
            return z
        } catch { throw PairingError.openFailed }
    }

    public static func sessionKey(z: [UInt8], q: [UInt8], desktopPublic: [UInt8], phonePublic: [UInt8], id: String) -> [UInt8] {
        var salt = q + desktopPublic + phonePublic
        defer { for index in salt.indices { salt[index] = 0 } }
        return HKDF<SHA256>.deriveKey(inputKeyMaterial: SymmetricKey(data: z), salt: salt,
                                      info: text("LV-PAIR-KEY-v2") + text(id), outputByteCount: 32)
            .withUnsafeBytes { Array($0) }
    }

    public static func sealAAD(id: String, desktopPublic: [UInt8], phonePublic: [UInt8]) -> [UInt8] {
        text("LV-PAIR-SEAL-v2") + text(id) + desktopPublic + phonePublic
    }

    public static func confirm(s: [UInt8], id: String) -> [UInt8] {
        hmac(s, [text("LV-PAIR-CONFIRM-v2"), text(id)])
    }

    /// `sealed` ist Standard-base64 von Nonce (24 Byte) ‖ Chiffrat ‖ Tag.
    public static func open(sealed: String, s: [UInt8], aad: [UInt8]) throws -> [UInt8] {
        guard let raw = Data(base64Encoded: sealed), raw.count > 24 + 16 else { throw PairingError.openFailed }
        let bytes = [UInt8](raw)
        do {
            return try XChaCha20Poly1305.open(key: s, nonce: Array(bytes[0..<24]), sealed: Array(bytes[24...]), aad: aad)
        } catch { throw PairingError.openFailed }
    }
}

/// Flüchtiges Geheimnis mit Löschpfad (Spec 5a.8). Eigener Speicher, weil ein `[UInt8]` beim
/// Überschreiben kopiert werden kann. `p_sk` löscht CryptoKit selbst, wenn die Session endet.
public final class PairingSecret: @unchecked Sendable, Equatable {
    private let lock = NSLock()
    private let memory: UnsafeMutableBufferPointer<UInt8>

    init(_ bytes: [UInt8]) {
        memory = .allocate(capacity: bytes.count)
        _ = memory.initialize(from: bytes)
    }

    deinit {
        wipe()
        memory.deallocate()
    }

    /// Das Array gilt nur innerhalb von `body` und wird danach genullt.
    public func use<T>(_ body: ([UInt8]) throws -> T) rethrows -> T {
        var copy = lock.withLock { Array(memory) }
        defer { for index in copy.indices { copy[index] = 0 } }
        return try body(copy)
    }

    public func wipe() {
        lock.withLock { memory.update(repeating: 0) }
    }

    public static func == (a: PairingSecret, b: PairingSecret) -> Bool { a === b }
}

/// Inhalt des QR-Codes. `q` und `d_pk` kennt der Hub nicht.
public struct PairingCode: Equatable, Sendable {
    public let hub: String
    public let id: String
    public let secret: String
    public let q: PairingSecret
    public let desktopPublic: [UInt8]

    /// Adresse für die Anzeige, damit ein fremder Server auffällt.
    public var hubHost: String { URLComponents(string: hub)?.host ?? hub }

    /// `allowLocalHTTP` nur für den Test-Hub im Simulator.
    public init(uri: String, allowLocalHTTP: Bool = false) throws {
        guard let parts = URLComponents(string: uri.trimmingCharacters(in: .whitespacesAndNewlines)),
              parts.scheme?.lowercased() == "localvoice", parts.host?.lowercased() == "pair",
              let items = parts.queryItems else { throw PairingError.invalidCode }
        var values: [String: String] = [:]
        for item in items {
            // Doppelte Felder sind kein gültiger Code: nicht raten, welches gilt.
            guard let value = item.value, values[item.name] == nil else { throw PairingError.invalidCode }
            values[item.name] = value
        }
        guard let version = values["v"] else { throw PairingError.invalidCode }
        guard version == "2" else { throw PairingError.outdatedCode }
        guard let rawHub = values["h"], let id = values["p"], PairingProto.validPairId(id),
              let secret = values["s"], PairingProto.b64u32(secret) != nil,
              var q = values["q"].flatMap(PairingProto.b64u32),
              let key = values["k"].flatMap(PairingProto.b64u32) else { throw PairingError.invalidCode }
        self.hub = try Self.hubBase(rawHub, allowLocalHTTP: allowLocalHTTP)
        self.id = id
        self.secret = secret
        self.q = PairingSecret(q)
        for index in q.indices { q[index] = 0 }
        self.desktopPublic = key
    }

    private static func hubBase(_ raw: String, allowLocalHTTP: Bool) throws -> String {
        let base = raw.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard let url = URLComponents(string: base), let host = url.percentEncodedHost, !host.isEmpty,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              // Nur ASCII: Sonst sieht ein fremder Server in der Anzeige aus wie der eigene.
              host.utf8.allSatisfy({ c in
                  (c >= 0x30 && c <= 0x39) || (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A) || c == 0x2D || c == 0x2E
              }) else { throw PairingError.insecureHub }
        let scheme = url.scheme?.lowercased()
        guard scheme == "https" || (allowLocalHTTP && scheme == "http" && host == "127.0.0.1") else { throw PairingError.insecureHub }
        return base
    }
}

/// Gerätename nach 4.1: NFC, 1–60 Zeichen, ohne `\p{C}`. Der Hub lehnt ab statt zu
/// korrigieren, weil der Name in das Commitment eingeht; also bereinigt der Client.
public enum PairingName {
    public static func clean(_ raw: String, fallback: String = "iPhone") -> String {
        let hidden: Set<Unicode.GeneralCategory> = [.control, .format, .surrogate, .privateUse, .unassigned, .lineSeparator, .paragraphSeparator]
        var scalars = String.UnicodeScalarView()
        scalars.append(contentsOf: raw.precomposedStringWithCanonicalMapping.unicodeScalars.filter {
            !hidden.contains($0.properties.generalCategory) && !$0.properties.isDefaultIgnorableCodePoint
        })
        let trimmed = String(scalars).trimmingCharacters(in: .whitespacesAndNewlines)
        var limited = String.UnicodeScalarView()
        limited.append(contentsOf: trimmed.unicodeScalars.prefix(60))
        let name = String(limited).trimmingCharacters(in: .whitespacesAndNewlines).precomposedStringWithCanonicalMapping
        return name.isEmpty ? fallback : name
    }
}

/// Inhalt von `sealed` nach der Prüfung.
public struct PairedAccount: Equatable, Sendable {
    public let hub: String
    public let email: String
    public let userId: Int64
    public let token: String
    public let key: [UInt8]
    public let deviceName: String
}

public protocol PairingTransport: Sendable {
    func post(_ path: String, body: [String: String], pollToken: String?) async throws -> (Int, Data)
    func get(_ path: String, pollToken: String) async throws -> (Int, Data)
}

public enum PairingStage: Equatable, Sendable {
    case claiming, waitingForDesktop
    /// Vergleichscode, solange der PC noch nicht bestätigt hat.
    case compare(String)
    case opening
}

/// Der Ablauf aus Tabelle 5.3. Geheimnisse leben nur für die Dauer von `run`.
public struct PairingSession: Sendable {
    public static let pollNanos: UInt64 = 1_500_000_000  // Hub drosselt auf 120 Abfragen je Minute
    /// Frist am Hub sind 120 s; dieses Limit greift nur, wenn der Hub nicht mehr antwortet.
    private static let maxPolls = 140
    /// Der Hub lässt 120 s; danach kann nichts mehr kommen, auch wenn er schweigt.
    private static let maxSeconds: TimeInterval = 150
    private static let confirmTries = 4

    let code: PairingCode
    let name: String
    let transport: any PairingTransport
    let phoneKey: Curve25519.KeyAgreement.PrivateKey
    let phoneNonce: [UInt8]
    let pause: @Sendable (UInt64) async throws -> Void
    let now: @Sendable () -> Date

    public init(code: PairingCode, name: String, transport: any PairingTransport,
                phoneKey: Curve25519.KeyAgreement.PrivateKey = .init(), phoneNonce: [UInt8]? = nil,
                pause: @escaping @Sendable (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) },
                now: @escaping @Sendable () -> Date = { Date() }) {
        self.code = code
        self.name = name
        self.transport = transport
        self.phoneKey = phoneKey
        self.phoneNonce = phoneNonce ?? SymmetricKey(size: .bits256).withUnsafeBytes { Array($0) }
        self.pause = pause
        self.now = now
    }

    /// `adopt` speichert das Konto. Erst danach geht die Bestätigung an den Hub.
    /// Rückgabe: ob die Bestätigung sicher angekommen ist. Sonst ist sie unklar, das Konto steht trotzdem.
    @discardableResult
    public func run(stage: @escaping @Sendable (PairingStage) async -> Void,
                    adopt: @Sendable (PairedAccount) async throws -> Void) async throws -> Bool {
        defer { code.q.wipe() }  // bei Erfolg, Fehler und Abbruch
        let phonePublic = [UInt8](phoneKey.publicKey.rawRepresentation)
        await stage(.claiming)
        let (poll, expires) = try await claim(phonePublic)
        await stage(.waitingForDesktop)

        var desktopNonce: [UInt8]?
        var revealTries = 0
        var lostContact = false
        let started = now()
        // Frist des Hubs. Liegt sie schon zurück, geht die Uhr hier falsch: dann gilt nur die Obergrenze.
        let limit = started.addingTimeInterval(Self.maxSeconds)
        let deadline = expires.flatMap { $0 > started ? min($0, limit) : nil } ?? limit
        func wait() async throws {
            let left = deadline.timeIntervalSince(now())
            try await pause(min(Self.pollNanos, UInt64(max(0, left) * 1_000_000_000)))
        }
        for _ in 0..<Self.maxPolls {
            try Task.checkCancellation()
            if now() > deadline { break }
            let answer: (Int, Data)
            do {
                answer = try await transport.get("/api/v1/voice/pair/\(code.id)/result", pollToken: poll)
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                lostContact = true
                try await wait()
                continue
            }
            lostContact = false
            switch answer.0 {
            case 200:
                guard let json = (try? JSONSerialization.jsonObject(with: answer.1)) as? [String: Any],
                      let status = json["status"] as? String else { throw PairingError.badResponse }
                switch status {
                case "challenge":
                    guard let nonce = (json["desktop_nonce"] as? String).flatMap(PairingProto.b64u32) else { throw PairingError.badResponse }
                    if let known = desktopNonce {
                        guard known == nonce else { throw PairingError.nonceChanged }
                    } else {
                        desktopNonce = nonce
                    }
                    try await reveal(phonePublic, poll: poll, tries: &revealTries)
                    if revealTries == 1 {
                        await stage(.compare(PairingProto.sas(id: code.id, desktopPublic: code.desktopPublic, phonePublic: phonePublic,
                                                              phoneNonce: phoneNonce, desktopNonce: nonce)))
                    }
                case "ok":
                    guard revealTries > 0 else { throw PairingError.badResponse }
                    guard now() <= deadline else { throw PairingError.expired }
                    await stage(.opening)
                    return try await finish(json, phonePublic: phonePublic, poll: poll, adopt: adopt)
                case "waiting":
                    break
                default:
                    throw PairingError.badResponse
                }
            case 202:
                break
            case 403:
                throw PairingError.rejected
            case 410:
                throw PairingError.expired
            case 429, 500...599:
                break  // Drosselung und Serverfehler: nur die Abfrage wiederholen
            default:
                throw PairingError.badResponse
            }
            try await wait()
        }
        throw lostContact ? PairingError.offline : PairingError.timedOut
    }

    private func claim(_ phonePublic: [UInt8]) async throws -> (poll: String, expires: Date?) {
        let commitment = PairingProto.commitment(id: code.id, phonePublic: phonePublic, phoneNonce: phoneNonce, name: name)
        let mac = code.q.use { PairingProto.claimMac(q: $0, id: code.id, commitment: commitment) }
        let answer: (Int, Data)
        do {
            answer = try await transport.post("/api/v1/voice/pair/claim", body: [
                "pairing_id": code.id, "secret": code.secret, "phone_commit": PairingProto.b64u(commitment),
                "phone_mac": PairingProto.b64u(mac), "device_name": name,
            ], pollToken: nil)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw PairingError.offline  // Claim gilt einmal: eine verlorene Antwort lässt sich nicht nachholen.
        }
        switch answer.0 {
        case 200:
            guard let json = (try? JSONSerialization.jsonObject(with: answer.1)) as? [String: Any],
                  let poll = json["poll_token"] as? String, !poll.isEmpty else { throw PairingError.badResponse }
            return (poll, (json["expires_at"] as? String).flatMap { ISO8601DateFormatter().date(from: $0) })
        case 410: throw PairingError.expired
        case 400:
            let error = ((try? JSONSerialization.jsonObject(with: answer.1)) as? [String: Any])?["error"] as? [String: Any]
            throw error?["code"] as? String == "invalid_device_name" ? PairingError.nameRejected : PairingError.badResponse
        case 429: throw PairingError.tooManyAttempts
        case 500...599: throw PairingError.serverFailed
        default: throw PairingError.badResponse
        }
    }

    /// Immer dieselben Werte. Nach einem ersten Versuch heißt 410: die Antwort ging verloren.
    private func reveal(_ phonePublic: [UInt8], poll: String, tries: inout Int) async throws {
        tries += 1
        let answer: (Int, Data)
        do {
            answer = try await transport.post("/api/v1/voice/pair/\(code.id)/reveal", body: [
                "phone_pub": PairingProto.b64u(phonePublic), "phone_nonce": PairingProto.b64u(phoneNonce),
            ], pollToken: poll)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            return  // Der Hub wiederholt die Challenge, solange er nichts bekommen hat.
        }
        switch answer.0 {
        case 200: break
        case 409: throw PairingError.commitmentFailed
        case 410: if tries == 1 { throw PairingError.expired }
        case 429, 500...599: break
        default: throw PairingError.badResponse
        }
    }

    private func finish(_ json: [String: Any], phonePublic: [UInt8], poll: String,
                        adopt: @Sendable (PairedAccount) async throws -> Void) async throws -> Bool {
        guard let token = json["token"] as? String, !token.isEmpty, let sealed = json["sealed"] as? String,
              let user = json["user"] as? [String: Any], let userId = (user["id"] as? NSNumber)?.int64Value,
              let hubEmail = user["email"] as? String else {
            throw PairingError.badResponse
        }
        var z = try PairingProto.shared(phoneKey, desktopPublic: code.desktopPublic)
        defer { Self.wipe(&z) }
        var s = code.q.use { PairingProto.sessionKey(z: z, q: $0, desktopPublic: code.desktopPublic, phonePublic: phonePublic, id: code.id) }
        defer { Self.wipe(&s) }
        var plain = try PairingProto.open(sealed: sealed, s: s, aad: PairingProto.sealAAD(id: code.id, desktopPublic: code.desktopPublic, phonePublic: phonePublic))
        defer { Self.wipe(&plain) }
        struct Payload: Decodable {
            let v: Int, k: String, email: String, user_id: Int64, key_id: String
        }
        guard let payload = try? JSONDecoder().decode(Payload.self, from: Data(plain)), payload.v == 1,
              let key = PairingProto.b64u32(payload.k), !payload.email.isEmpty else { throw PairingError.openFailed }
        // Die Anzeige beruht auf der Angabe des Hubs: Der PC-Inhalt allein könnte ein fremdes Konto tarnen.
        guard payload.key_id == SyncCrypto.keyId(key), payload.user_id == userId,
              payload.email.caseInsensitiveCompare(hubEmail) == .orderedSame else { throw PairingError.keyMismatch }

        try Task.checkCancellation()
        try await adopt(PairedAccount(hub: code.hub, email: hubEmail, userId: userId, token: token, key: key, deviceName: name))

        // Das Konto steht: Die Bestätigung geht auch raus, wenn die Oberfläche inzwischen weg ist.
        let tag = PairingProto.b64u(PairingProto.confirm(s: s, id: code.id))
        let session = self
        return await Task.detached { await session.confirm(tag, poll: poll) }.value
    }

    private func confirm(_ tag: String, poll: String) async -> Bool {
        // Nur 200 ist eine Bestätigung. 410 nach einem verlorenen Versuch kann beides heißen.
        for attempt in 0..<Self.confirmTries {
            if attempt > 0 { try? await pause(Self.pollNanos) }
            guard let answer = try? await transport.post("/api/v1/voice/pair/\(code.id)/confirm", body: ["confirm": tag], pollToken: poll) else { continue }
            if answer.0 == 200 { return true }
            if answer.0 != 429 && answer.0 < 500 { return false }
        }
        return false
    }

    private static func wipe(_ bytes: inout [UInt8]) {
        for index in bytes.indices { bytes[index] = 0 }
    }
}
#endif
