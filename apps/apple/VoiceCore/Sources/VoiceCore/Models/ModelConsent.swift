import Foundation

// Zustimmung und Prüfstelle der gemeinsamen Modellwahl (lva#801, lva#804).
// Aufbau (Codex-Review lva#810, Runde 4): EIN Actor `ModelAccessState` hält Kundenprofil, aufgelöste Routen und
// Zustimmungen mit monotoner Version. Ausstellen und Einlösen einer Berechtigung sind je ein Actor-Aufruf ohne
// Suspension, also atomar. Beim Einlösen (`confirm`, einmalig, unmittelbar vor dem Senden) wird der Stand neu
// bewertet und mit der Berechtigung verglichen (Route, Zweck, Gegenstand, Ziel, Anbieter, Geltungsbereich);
// jede Abweichung bricht ab. Aussteller sind `fileprivate`: außerhalb dieser Datei gibt es keinen Weg zu einer
// Berechtigung außer `ModelSession`.
// - Extern ist alles außer `.local`; Anzeige-Angaben der Auswahl werden nie vertraut.
// - Unbekannte Route (gelöschter Endpunkt, vom PC nicht gemeldet) = verboten.
// - Ohne geladenes Kundenprofil gilt `ModelPolicy.unknown` (nur lokal).

/// Aktuelle Konfiguration einer Route, wie die App sie gespeichert hat bzw. der PC sie meldet.
public struct ModelResolvedTarget: Equatable, Sendable {
    public let vendor: String
    /// Endpunkt: die Adresse, an die tatsächlich gesendet wird. PC-Relay: `nil`.
    public let url: URL?
    public let region: String?
    public init(vendor: String, url: URL?, region: String? = nil) { self.vendor = vendor; self.url = url; self.region = region }
}

/// Geltungsbereich einer Zustimmung. Jede Abweichung macht sie ungültig.
public struct ModelConsentScope: Codable, Hashable, Sendable {
    public let subjectId: String
    public let purpose: ModelPurpose
    public let routeKey: String
    /// Kleingeschrieben.
    public let vendor: String
    /// `https://host:port` des Endpunkts bzw. `pc-relay`.
    public let destination: String
    /// Sortiert, ohne Doppelte.
    public let dataCategories: [String]

    init(subjectId: String, purpose: ModelPurpose, route: ModelRoute, vendor: String, destination: String, dataCategories: [String]) {
        self.subjectId = subjectId; self.purpose = purpose; self.routeKey = route.key
        self.vendor = vendor.lowercased(); self.destination = destination
        self.dataCategories = Array(Set(dataCategories)).sorted()
    }

    /// Eindeutiger Speicherschlüssel (JSON mit sortierten Schlüsseln, keine Trennzeichen-Mehrdeutigkeit).
    public var storageKey: String {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        return (try? encoder.encode(self)).map { String(decoding: $0, as: UTF8.self) } ?? ""
    }
}

/// Nachweis, dass die Person der Verarbeitung im Geltungsbereich zugestimmt hat.
public struct ModelConsentRecord: Codable, Hashable, Sendable {
    public let scope: ModelConsentScope
    public let grantedAt: Date
    public init(scope: ModelConsentScope, grantedAt: Date) { self.scope = scope; self.grantedAt = grantedAt }

    /// Text für Dialog und Nachweis. Keine Inhalte, nur Kategorien.
    public static func summary(choice: ModelChoice, scope: ModelConsentScope, vendor: String, region: String?) -> String {
        let place = region.map { " (Verarbeitung: \($0))" } ?? ""
        let via: String
        if case .pcRelay = choice.route { via = " über deinen PC" } else { via = "" }
        return "\(scope.dataCategories.joined(separator: ", ")) gehen\(via) an \(vendor)\(place) und werden dort mit \(choice.label) verarbeitet."
    }
}

/// Gemeinsamer, versionierter Zustand aller Modellfreigaben. Die App hält eine Instanz, setzt das Profil, die
/// Routen (Endpunkt-Einstellungen, Modellliste des PCs) und die Zustimmungen und speichert `consentRecords()`.
public actor ModelAccessState {
    public private(set) var version: UInt64 = 0
    private var policy: ModelPolicy
    private var targets: [String: ModelResolvedTarget]
    private var consents: [String: ModelConsentRecord]

    public init(policy: ModelPolicy = .unknown, targets: [ModelRoute: ModelResolvedTarget] = [:], consents: [ModelConsentRecord] = []) {
        self.policy = policy
        self.targets = Dictionary(uniqueKeysWithValues: targets.map { ($0.key.key, $0.value) })
        self.consents = Dictionary(consents.map { ($0.scope.storageKey, $0) }, uniquingKeysWith: { _, b in b })
    }

    public func setPolicy(_ p: ModelPolicy) { policy = p; version &+= 1 }
    public func currentPolicy() -> ModelPolicy { policy }
    /// `nil` entfernt die Route (danach verboten).
    public func setTarget(_ target: ModelResolvedTarget?, for route: ModelRoute) { targets[route.key] = target; version &+= 1 }
    public func record(_ consent: ModelConsentRecord) { consents[consent.scope.storageKey] = consent; version &+= 1 }
    public func revoke(_ scope: ModelConsentScope) { consents[scope.storageKey] = nil; version &+= 1 }
    public func revokeAll(subjectId: String) { consents = consents.filter { $0.value.scope.subjectId != subjectId }; version &+= 1 }
    public func consentRecords() -> [ModelConsentRecord] { consents.values.sorted { $0.grantedAt < $1.grantedAt } }

    /// Geltungsbereich für eine Zustimmung (Dialog). Lokal: `nil`. Wirft `localOnly`, `unavailable`,
    /// `blockedByPolicy`, `insecureDestination`, `consentMissing` (keine Datenkategorie).
    public func consentScope(route: ModelRoute, purpose: ModelPurpose, subjectId: String,
                             dataCategories: [String]) throws -> (ModelConsentScope, ModelResolvedTarget)? {
        if purpose.localOnly && !route.isLocal { throw ModelFailure.localOnly }
        if route.isLocal { return nil }
        guard let target = targets[route.key] else { throw ModelFailure.unavailable }
        guard policy.permits(route: route, vendor: target.vendor) else { throw ModelFailure.blockedByPolicy }
        guard !dataCategories.isEmpty else { throw ModelFailure.consentMissing }
        let destination: String
        switch route {
        case .local: return nil
        case .endpoint:
            guard let url = target.url, let origin = ModelGate.validatedOrigin(url) else { throw ModelFailure.insecureDestination }
            destination = origin
        case .pcRelay:
            guard target.url == nil else { throw ModelFailure.insecureDestination }
            destination = "pc-relay"
        }
        let scope = ModelConsentScope(subjectId: subjectId, purpose: purpose, route: route, vendor: target.vendor,
                                      destination: destination, dataCategories: dataCategories)
        return (scope, target)
    }

    /// Bewertet den aktuellen Stand (ohne Suspension).
    fileprivate func evaluate(route: ModelRoute, purpose: ModelPurpose, subjectId: String, dataCategories: [String]) throws -> ModelGrant {
        guard let resolved = try consentScope(route: route, purpose: purpose, subjectId: subjectId, dataCategories: dataCategories) else {
            return ModelGrant(route: route, purpose: purpose, subjectId: subjectId, destination: nil, vendor: nil, scope: nil)
        }
        let (scope, target) = resolved
        guard let record = consents[scope.storageKey], record.scope == scope else { throw ModelFailure.consentMissing }
        return ModelGrant(route: route, purpose: purpose, subjectId: subjectId, destination: target.url, vendor: target.vendor, scope: scope)
    }

    /// Stellt eine Berechtigung aus (atomar). Nur `ModelSession` ruft das auf.
    fileprivate func issue(route: ModelRoute, purpose: ModelPurpose, subjectId: String, dataCategories: [String]) throws -> ModelAuthorization {
        let grant = try evaluate(route: route, purpose: purpose, subjectId: subjectId, dataCategories: dataCategories)
        return ModelAuthorization(grant: grant, dataCategories: dataCategories, version: version, state: self)
    }

    /// Löst eine Berechtigung ein (atomar): einmalig, und nur wenn der aktuelle Stand genau dasselbe erlaubt.
    fileprivate func redeem(_ authorization: ModelAuthorization) throws {
        guard !redeemed.contains(authorization.id) else { throw ModelFailure.consentMissing }
        redeemed.insert(authorization.id)
        // Jede Änderung seit dem Ausstellen (auch hin und zurück) macht die Berechtigung ungültig.
        guard authorization.version == version else { throw ModelFailure.stateChanged }
        let now = try evaluate(route: authorization.grant.route, purpose: authorization.grant.purpose,
                               subjectId: authorization.grant.subjectId, dataCategories: authorization.dataCategories)
        guard now == authorization.grant else { throw ModelFailure.consentMissing }
    }
    private var redeemed: Set<UUID> = []
}

/// Was eine Berechtigung erlaubt. Gleichheit = gleiche Route, Zweck, Gegenstand, Ziel, Anbieter, Geltungsbereich.
fileprivate struct ModelGrant: Equatable, Sendable {
    let route: ModelRoute
    let purpose: ModelPurpose
    let subjectId: String
    let destination: URL?
    let vendor: String?
    let scope: ModelConsentScope?
}

/// Ausgestellt nur von `ModelSession` über `ModelAccessState`. Provider senden nur nach `confirm()` und nur an `destination`.
public struct ModelAuthorization: Sendable {
    fileprivate let id = UUID()
    fileprivate let grant: ModelGrant
    fileprivate let dataCategories: [String]
    fileprivate let version: UInt64
    private let state: ModelAccessState

    fileprivate init(grant: ModelGrant, dataCategories: [String], version: UInt64, state: ModelAccessState) {
        self.grant = grant; self.dataCategories = dataCategories; self.version = version; self.state = state
    }

    public var route: ModelRoute { grant.route }
    public var purpose: ModelPurpose { grant.purpose }
    public var subjectId: String { grant.subjectId }
    /// Endpunkt: geprüfte Zieladresse. Lokal und PC-Relay: `nil`.
    public var destination: URL? { grant.destination }
    public var vendor: String? { grant.vendor }

    /// Unmittelbar vor dem Senden aufrufen (nach dem letzten eigenen `await`). Einmalig; prüft atomar,
    /// dass der aktuelle Stand genau diese Berechtigung noch erlaubt.
    public func confirm() async throws {
        try Task.checkCancellation()
        try await state.redeem(self)
        try Task.checkCancellation()
    }

    /// Darf an genau diese Adresse gesendet werden (Schema, Host, Port, Pfad gleich, keine Zugangsdaten)?
    public func permitsSending(to url: URL) -> Bool {
        guard let destination, url.user == nil, url.password == nil else { return false }
        return url.scheme?.lowercased() == destination.scheme?.lowercased()
            && url.host?.lowercased() == destination.host?.lowercased()
            && ModelGate.port(url) == ModelGate.port(destination)
            && url.path == destination.path
    }
}

/// Einstieg für Aufrufer und Prüfhelfer.
public enum ModelGate {
    /// Öffnet eine Sitzung und prüft sofort (frühe Fehlermeldung). Jeder Aufruf prüft erneut.
    public static func open(_ provider: any ModelProvider, purpose: ModelPurpose, subjectId: String, dataCategories: [String],
                            state: ModelAccessState) async throws -> ModelSession {
        let session = ModelSession(provider: provider, purpose: purpose, subjectId: subjectId, dataCategories: dataCategories, state: state)
        try await session.check()
        return session
    }

    /// `scheme://host:port` für https, http nur für Loopback und private Netze; keine Zugangsdaten in der Adresse.
    static func validatedOrigin(_ url: URL) -> String? {
        guard url.user == nil, url.password == nil, let scheme = url.scheme?.lowercased(),
              let host = url.host?.lowercased(), !host.isEmpty else { return nil }
        switch scheme {
        case "https": break
        case "http": guard isPrivateHost(host) else { return nil }
        default: return nil
        }
        return "\(scheme)://\(host):\(port(url))"
    }

    static func port(_ url: URL) -> Int { url.port ?? (url.scheme?.lowercased() == "http" ? 80 : 443) }

    /// Loopback, RFC 1918, Link-Local, IPv6 Loopback/ULA/Link-Local und `localhost`. Hostnamen sonst nie
    /// (für http im eigenen Netz die IP-Adresse eintragen).
    /// IPv4 nur als vollständige Punktschreibweise: genau vier Dezimalteile 0–255 ohne führende Nullen.
    public static func isPrivateHost(_ host: String) -> Bool {
        var h = host.lowercased()
        if h.hasPrefix("[") && h.hasSuffix("]") { h = String(h.dropFirst().dropLast()) }
        if h == "localhost" { return true }
        if h.contains(":") { return isPrivateIPv6(h) }
        if let ip = ipv4(h) {
            switch (ip[0], ip[1]) {
            case (127, _), (10, _), (192, 168), (169, 254): return true
            case (172, let b): return (16...31).contains(b)
            default: return false
            }
        }
        // Namen (auch `*.local`) sind nie privat: ihre Auflösung ist nicht an eine private Adresse gebunden.
        return false
    }

    /// Genau vier Dezimalteile 0–255, keine führenden Nullen, kein Hex/Oktal, nichts sonst.
    static func ipv4(_ h: String) -> [Int]? {
        let parts = h.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return nil }
        var out: [Int] = []
        for p in parts {
            guard !p.isEmpty, p.count <= 3, p.allSatisfy({ $0 >= "0" && $0 <= "9" }), p == "0" || p.first != "0",
                  let v = Int(p), v <= 255 else { return nil }
            out.append(v)
        }
        return out
    }

    /// `::1`, ULA `fc00::/7`, Link-Local `fe80::/10`. Nur Hexziffern und Doppelpunkte, keine Zone, keine IPv4-Einbettung.
    static func isPrivateIPv6(_ h: String) -> Bool {
        let hex = Set("0123456789abcdef:")
        guard h.allSatisfy(hex.contains), h.filter({ $0 == ":" }).count >= 2, h.count <= 39 else { return false }
        if h == "::1" { return true }
        let groups = h.split(separator: ":", omittingEmptySubsequences: false)
        guard groups.allSatisfy({ $0.count <= 4 }), let first = groups.first, !first.isEmpty, let v = Int(first, radix: 16) else { return false }
        return (v & 0xfe00) == 0xfc00 || (v & 0xffc0) == 0xfe80
    }
}

/// Gebundene Sitzung aus `ModelGate.open`. Der einzige Weg zu einer Berechtigung.
public struct ModelSession: Sendable {
    public let provider: any ModelProvider
    public let purpose: ModelPurpose
    public let subjectId: String
    public let dataCategories: [String]
    private let state: ModelAccessState

    fileprivate init(provider: any ModelProvider, purpose: ModelPurpose, subjectId: String, dataCategories: [String], state: ModelAccessState) {
        self.provider = provider; self.purpose = purpose; self.subjectId = subjectId; self.dataCategories = dataCategories; self.state = state
    }

    /// Prüft jetzt, ohne zu senden.
    public func check() async throws {
        _ = try await state.issue(route: provider.route, purpose: purpose, subjectId: subjectId, dataCategories: dataCategories)
    }

    public func stream(_ request: ModelRequest) -> AsyncThrowingStream<ModelEvent, Error> {
        if Task.isCancelled { return AsyncThrowingStream { $0.finish(throwing: CancellationError()) } }
        let session = self
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard request.purpose == session.purpose else { throw ModelFailure.consentMissing }
                    // Ändert sich der Freigabestand zwischen Ausstellen und Einlösen, wird genau einmal neu
                    // ausgestellt (und neu geprüft), solange noch nichts geliefert wurde.
                    var attempt = 0
                    while true {
                        attempt += 1
                        var yielded = false
                        do {
                            let authorization = try await session.state.issue(route: session.provider.route, purpose: session.purpose,
                                                                              subjectId: session.subjectId, dataCategories: session.dataCategories)
                            try Task.checkCancellation()
                            for try await event in session.provider.stream(request, authorization: authorization) {
                                try Task.checkCancellation()
                                yielded = true
                                continuation.yield(event)
                            }
                            break
                        } catch ModelFailure.stateChanged where !yielded && attempt < 2 {
                            continue
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// Sammelt den Strom zu einem Text; prüft den Abbruch vor dem ersten Ereignis.
    public func complete(_ request: ModelRequest) async throws -> String {
        try Task.checkCancellation()
        var text = ""
        for try await event in stream(request) {
            try Task.checkCancellation()
            if case .delta(let piece) = event { text += piece }
        }
        try Task.checkCancellation()
        return text
    }
}

/// Gewählte Route je Zweck. Fehlt sie oder ist sie nicht mehr erlaubt/vorhanden, gilt die erste lokale Wahl.
public enum ModelSelection {
    public static func settingsKey(_ purpose: ModelPurpose) -> String { "modelChoice.\(purpose.rawValue)" }

    public static func resolve(storedKey: String?, choices: [ModelChoice], purpose: ModelPurpose, policy: ModelPolicy) -> ModelChoice? {
        let usable = choices.filter { policy.permits($0) && (!purpose.localOnly || $0.route.isLocal) }
        if let key = storedKey, let hit = usable.first(where: { $0.id == key }) { return hit }
        return usable.first(where: { $0.route.isLocal })
    }
}
