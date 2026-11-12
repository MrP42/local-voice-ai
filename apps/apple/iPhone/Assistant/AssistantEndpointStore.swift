import Foundation
import Security

// Gesprächsmodus (Paket M): Ablage des eigenen Endpunkts. Angaben in UserDefaults, Schlüssel nur im Schlüsselbund.

// MARK: - Schlüsselbund

/// Der API-Schlüssel gehört genau diesem Gerät und liegt nur im Schlüsselbund (nie in `UserDefaults`, nie in Sicherungen).
enum AssistantEndpointKeychain {
    private static let service = "de.localvoice.assistant-endpoint"
    private static let account = "api-key"

    @discardableResult static func save(_ key: String) -> Bool {
        delete()
        return SecItemAdd([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account,
                           kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, kSecValueData: Data(key.utf8)] as CFDictionary, nil) == errSecSuccess
    }

    static func load() -> String? {
        var out: CFTypeRef?
        guard SecItemCopyMatching([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account,
                                   kSecReturnData: true] as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    static func delete() {
        SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account] as CFDictionary)
    }
}

// MARK: - Store

@MainActor
final class AssistantEndpointStore: ObservableObject {
    static let shared = AssistantEndpointStore()
    nonisolated static let endpointKey = "assistant_endpoint"
    nonisolated static let consentKey = "assistant_endpoint_consent"

    @Published private(set) var endpoint: AssistantEndpoint?
    @Published private(set) var hasKey = false
    /// Freigabe als „Host|Modell“; gilt nur für genau diese Kombination.
    @Published private(set) var consentedId: String?

    init() {
        let defaults = UserDefaults.standard
        endpoint = defaults.data(forKey: Self.endpointKey).flatMap { try? JSONDecoder().decode(AssistantEndpoint.self, from: $0) }
        consentedId = defaults.string(forKey: Self.consentKey)
        hasKey = AssistantEndpointKeychain.load() != nil
    }

    /// Außerhalb von Loopback, privaten Netzen und `*.local` (Anfragen verlassen das eigene Netz).
    var isCloud: Bool { endpoint.map(Self.isCloud) ?? false }

    /// Host für Anzeige und Freigabedialog.
    var hostLabel: String { endpoint.flatMap { Self.hostName(of: $0.baseURL) } ?? "" }

    /// Freigabe je Host + Modell; lokale Endpunkte brauchen keine.
    var consented: Bool {
        guard let endpoint else { return false }
        return !Self.isCloud(endpoint) || consentedId == Self.consentId(endpoint)
    }

    func grantConsent() {
        guard let endpoint else { return }
        setConsent(Self.consentId(endpoint))
    }

    func revokeConsent() { setConsent(nil) }

    /// Speichert den Endpunkt nach Prüfung (wirft einen verständlichen `AssistantEndpointError`).
    /// `apiKey`: nil = vorhandenen Schlüssel behalten (außer der Host wechselt, dann wird er gelöscht, damit er nie an einen
    /// anderen Host geht), leer = löschen, sonst setzen.
    func save(_ endpoint: AssistantEndpoint, apiKey: String?) throws {
        let clean = try Self.validated(endpoint)
        let key = apiKey?.trimmingCharacters(in: .whitespacesAndNewlines)
        if let key, !key.isEmpty {
            guard key.unicodeScalars.allSatisfy({ (0x21...0x7E).contains($0.value) }) else { throw AssistantEndpointError.badKey }
        }
        let data = try JSONEncoder().encode(clean)
        let hostChanged = self.endpoint.flatMap { Self.hostName(of: $0.baseURL) } != Self.hostName(of: clean.baseURL)
        if let key {
            if key.isEmpty {
                AssistantEndpointKeychain.delete()
                hasKey = false
            } else {
                guard AssistantEndpointKeychain.save(key) else { throw AssistantEndpointError.keychain }
                hasKey = true
            }
        } else if hostChanged {
            AssistantEndpointKeychain.delete()
            hasKey = false
        }
        UserDefaults.standard.set(data, forKey: Self.endpointKey)
        self.endpoint = clean
        if consentedId != Self.consentId(clean) { setConsent(nil) }
    }

    /// Löscht nur den Schlüssel.
    func removeKey() {
        AssistantEndpointKeychain.delete()
        hasKey = false
    }

    /// Löscht Endpunkt, Schlüssel und Freigabe.
    func remove() {
        AssistantEndpointKeychain.delete()
        UserDefaults.standard.removeObject(forKey: Self.endpointKey)
        endpoint = nil
        hasKey = false
        setConsent(nil)
    }

    func makeModel() -> AssistantEndpointModel? {
        guard let endpoint else { return nil }
        return AssistantEndpointModel(endpoint: endpoint, apiKey: AssistantEndpointKeychain.load(),
                                      authorized: { AssistantEndpointStore.isAuthorized(endpoint) })
    }

    /// Gespeicherter Schlüssel, nur wenn er zu genau diesem Host gehört (für „Verbindung prüfen“ mit geänderter Adresse).
    func storedKey(forHost host: String) -> String? {
        guard let endpoint, Self.hostName(of: endpoint.baseURL) == host else { return nil }
        return AssistantEndpointKeychain.load()
    }

    private func setConsent(_ id: String?) {
        consentedId = id
        if let id { UserDefaults.standard.set(id, forKey: Self.consentKey) } else { UserDefaults.standard.removeObject(forKey: Self.consentKey) }
    }

    // MARK: Prüfung und Einordnung (ohne Zustand, von jedem Thread aufrufbar)

    nonisolated static func consentId(_ endpoint: AssistantEndpoint) -> String {
        (hostName(of: endpoint.baseURL) ?? "") + "|" + endpoint.model
    }

    nonisolated static func isCloud(_ endpoint: AssistantEndpoint) -> Bool {
        guard let host = hostName(of: endpoint.baseURL) else { return true }
        return !isLocalHost(host)
    }

    /// Darf der Endpunkt jetzt angefragt werden? Lokal immer, Cloud nur mit gespeicherter Freigabe für Host + Modell.
    nonisolated static func isAuthorized(_ endpoint: AssistantEndpoint) -> Bool {
        guard isCloud(endpoint) else { return true }
        return UserDefaults.standard.string(forKey: consentKey) == consentId(endpoint)
    }

    /// Host der Adresse in Kleinbuchstaben, ohne eckige Klammern (IPv6) und ohne Zonenkennung. nil bei ungültiger Adresse.
    nonisolated static func hostName(of baseURL: String) -> String? {
        guard let host = URLComponents(string: AssistantEndpoint.normalizedBase(baseURL))?.host else { return nil }
        var name = host.lowercased()
        if name.hasPrefix("["), name.hasSuffix("]") { name = String(name.dropFirst().dropLast()) }
        if let zone = name.firstIndex(of: "%") { name = String(name[..<zone]) }
        guard !name.isEmpty,
              name.unicodeScalars.allSatisfy({ ($0.value < 128) && (CharacterSet.alphanumerics.contains($0) || "-._:".unicodeScalars.contains($0)) })
        else { return nil }
        return name
    }

    /// Prüft die Eingaben und liefert die bereinigte Form. http nur für lokale Hosts, Cloud nur mit https.
    nonisolated static func validated(_ endpoint: AssistantEndpoint) throws -> AssistantEndpoint {
        let base = AssistantEndpoint.normalizedBase(endpoint.baseURL)
        guard base.rangeOfCharacter(from: .whitespacesAndNewlines) == nil,
              let parts = URLComponents(string: base), let scheme = parts.scheme?.lowercased(), scheme == "http" || scheme == "https",
              parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil,
              let host = hostName(of: base)
        else { throw AssistantEndpointError.invalidAddress }
        let model = endpoint.model.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !model.isEmpty else { throw AssistantEndpointError.missingModel }
        guard scheme == "https" || isLocalHost(host) else { throw AssistantEndpointError.insecureCloud(host: host) }
        guard (2_048...1_000_000).contains(endpoint.contextTokens) else { throw AssistantEndpointError.badContext }
        let label = endpoint.label.trimmingCharacters(in: .whitespacesAndNewlines)
        return AssistantEndpoint(baseURL: base, model: model, label: label, contextTokens: endpoint.contextTokens)
    }

    // MARK: Hostklassifikation

    /// true bei Loopback (`localhost`, 127/8, ::1), privaten Netzen (10/8, 172.16/12, 192.168/16, fc00::/7), Link-Local
    /// (169.254/16, fe80::/10), geteiltem Adressraum 100.64/10 (Tailscale) und `*.local`. IPv4-in-IPv6 (`::ffff:a.b.c.d`)
    /// zählt nach der IPv4-Adresse. Alles andere, auch unbekannte oder einteilige Namen, gilt als Cloud.
    nonisolated static func isLocalHost(_ host: String) -> Bool {
        var name = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if name.hasPrefix("["), name.hasSuffix("]") { name = String(name.dropFirst().dropLast()) }
        if let zone = name.firstIndex(of: "%") { name = String(name[..<zone]) }
        while name.hasSuffix(".") { name.removeLast() }
        guard !name.isEmpty else { return false }
        if name == "localhost" || (name.hasSuffix(".local") && name.count > ".local".count) { return true }
        if let v4 = ipv4(name) { return isLocalV4(v4) }
        if let v6 = ipv6(name) { return isLocalV6(v6) }
        return false
    }

    /// Vier Bytes aus „a.b.c.d“ (nur Dezimalziffern, 0–255, keine führenden Nullen), sonst nil.
    private nonisolated static func ipv4(_ text: String) -> [Int]? {
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return nil }
        var bytes: [Int] = []
        for part in parts {
            guard (1...3).contains(part.count), part.allSatisfy({ $0 >= "0" && $0 <= "9" }), part.count == 1 || part.first != "0",
                  let value = Int(part), value <= 255 else { return nil }
            bytes.append(value)
        }
        return bytes
    }

    private nonisolated static func isLocalV4(_ b: [Int]) -> Bool {
        switch (b[0], b[1]) {
        case (10, _), (127, _), (192, 168), (169, 254): true
        case (172, 16...31): true
        case (100, 64...127): true
        default: false
        }
    }

    /// Acht 16-Bit-Gruppen aus einer IPv6-Adresse (mit „::“ und IPv4-Endstück), sonst nil.
    private nonisolated static func ipv6(_ text: String) -> [UInt16]? {
        guard text.contains(":") else { return nil }
        let halves = text.components(separatedBy: "::")
        guard halves.count <= 2 else { return nil }
        func groups(_ part: String, v4Tail: Bool) -> [UInt16]? {
            if part.isEmpty { return [] }
            var pieces = part.split(separator: ":", omittingEmptySubsequences: false).map(String.init)
            var tail: [UInt16] = []
            if v4Tail, let last = pieces.last, last.contains(".") {
                guard let v4 = ipv4(last) else { return nil }
                pieces.removeLast()
                tail = [UInt16(v4[0] << 8 | v4[1]), UInt16(v4[2] << 8 | v4[3])]
            }
            var out: [UInt16] = []
            for piece in pieces {
                guard (1...4).contains(piece.count), piece.allSatisfy({ $0.isASCII && $0.isHexDigit }),
                      let value = UInt16(piece, radix: 16) else { return nil }
                out.append(value)
            }
            return out + tail
        }
        if halves.count == 1 {
            guard let all = groups(text, v4Tail: true), all.count == 8 else { return nil }
            return all
        }
        guard let head = groups(halves[0], v4Tail: false), let tail = groups(halves[1], v4Tail: true), head.count + tail.count <= 7 else { return nil }
        return head + [UInt16](repeating: 0, count: 8 - head.count - tail.count) + tail
    }

    private nonisolated static func isLocalV6(_ g: [UInt16]) -> Bool {
        if g == [0, 0, 0, 0, 0, 0, 0, 1] { return true }
        if g[0] & 0xffc0 == 0xfe80 { return true }
        if g[0] & 0xfe00 == 0xfc00 { return true }
        if g[0...4].allSatisfy({ $0 == 0 }), g[5] == 0xffff {
            return isLocalV4([Int(g[6] >> 8), Int(g[6] & 0xff), Int(g[7] >> 8), Int(g[7] & 0xff)])
        }
        return false
    }
}
