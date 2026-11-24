import Foundation

// Zustimmung und Prüfstelle der gemeinsamen Modellwahl (lva#801, lva#804).
// Regeln (Codex-Review lva#810):
// - Ausgeführt wird nur mit einer `ModelAuthorization`, die allein `ModelGate` ausstellt.
// - Extern ist alles außer `.local`; die Anzeige-Angaben der Auswahl werden nie vertraut.
// - Endpunkt und PC-Relay werden bei jeder Prüfung frisch aufgelöst; unbekannt = verboten.
// - Eine Zustimmung gilt genau für Gegenstand, Zweck, Route, Anbieter, Ziel und Datenkategorien.
// - Ohne geladenes Kundenprofil gilt `ModelPolicy.unknown` (nur lokal).

/// Aktuelle Konfiguration einer Route, wie die App sie gespeichert hat bzw. der PC sie meldet.
public struct ModelResolvedTarget: Equatable, Sendable {
    public let vendor: String
    /// Endpunkt: die Adresse, an die tatsächlich gesendet wird. PC-Relay: `nil`.
    public let url: URL?
    public let region: String?
    public init(vendor: String, url: URL?, region: String? = nil) { self.vendor = vendor; self.url = url; self.region = region }
}

public protocol ModelRouteResolver: Sendable {
    /// `nil`: Route unbekannt (gelöschter Endpunkt, PC meldet das Modell nicht) – dann ist sie verboten.
    func resolve(_ route: ModelRoute) async -> ModelResolvedTarget?
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

public protocol ModelConsentStore: Sendable {
    func consent(for scope: ModelConsentScope) async -> ModelConsentRecord?
    func record(_ consent: ModelConsentRecord) async throws
}

/// Ausgestellt nur von `ModelGate`. Provider senden nur mit ihr und nur an `destination`.
public struct ModelAuthorization: Sendable {
    public let route: ModelRoute
    public let purpose: ModelPurpose
    public let subjectId: String
    /// Endpunkt: geprüfte Zieladresse. Lokal und PC-Relay: `nil`.
    public let destination: URL?
    public let vendor: String?

    fileprivate init(route: ModelRoute, purpose: ModelPurpose, subjectId: String, destination: URL?, vendor: String?) {
        self.route = route; self.purpose = purpose; self.subjectId = subjectId; self.destination = destination; self.vendor = vendor
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

/// Einzige Prüfstelle vor jedem Modellaufruf.
public enum ModelGate {
    /// Löst die Route auf und prüft Profil und Ziel; liefert den Geltungsbereich für eine Zustimmung.
    /// Lokal: `nil` (keine Zustimmung nötig).
    public static func consentScope(route: ModelRoute, purpose: ModelPurpose, subjectId: String, dataCategories: [String],
                                    policy: ModelPolicy, resolver: any ModelRouteResolver) async throws -> (ModelConsentScope, ModelResolvedTarget)? {
        if purpose.localOnly && !route.isLocal { throw ModelFailure.localOnly }
        if route.isLocal { return nil }
        guard let target = await resolver.resolve(route) else { throw ModelFailure.unavailable }
        guard policy.permits(route: route, vendor: target.vendor) else { throw ModelFailure.blockedByPolicy }
        guard !dataCategories.isEmpty else { throw ModelFailure.consentMissing }
        let destination: String
        switch route {
        case .local: return nil
        case .endpoint:
            guard let url = target.url, let origin = validatedOrigin(url) else { throw ModelFailure.insecureDestination }
            destination = origin
        case .pcRelay:
            guard target.url == nil else { throw ModelFailure.insecureDestination }
            destination = "pc-relay"
        }
        let scope = ModelConsentScope(subjectId: subjectId, purpose: purpose, route: route, vendor: target.vendor,
                                      destination: destination, dataCategories: dataCategories)
        return (scope, target)
    }

    /// Stellt die Ausführungsberechtigung aus oder wirft `localOnly`, `unavailable`, `blockedByPolicy`,
    /// `insecureDestination` bzw. `consentMissing`.
    public static func authorize(route: ModelRoute, purpose: ModelPurpose, subjectId: String, dataCategories: [String],
                                 policy: ModelPolicy, resolver: any ModelRouteResolver,
                                 consents: any ModelConsentStore) async throws -> ModelAuthorization {
        guard let resolved = try await consentScope(route: route, purpose: purpose, subjectId: subjectId,
                                                    dataCategories: dataCategories, policy: policy, resolver: resolver) else {
            return ModelAuthorization(route: route, purpose: purpose, subjectId: subjectId, destination: nil, vendor: nil)
        }
        let (scope, target) = resolved
        guard let record = await consents.consent(for: scope), record.scope == scope else { throw ModelFailure.consentMissing }
        return ModelAuthorization(route: route, purpose: purpose, subjectId: subjectId, destination: target.url, vendor: target.vendor)
    }

    /// Öffnet eine Sitzung: prüft und bindet Provider und Berechtigung.
    public static func open(_ provider: any ModelProvider, purpose: ModelPurpose, subjectId: String, dataCategories: [String],
                            policy: ModelPolicy, resolver: any ModelRouteResolver,
                            consents: any ModelConsentStore) async throws -> ModelSession {
        let authorization = try await authorize(route: provider.route, purpose: purpose, subjectId: subjectId,
                                                dataCategories: dataCategories, policy: policy, resolver: resolver, consents: consents)
        return ModelSession(provider: provider, authorization: authorization)
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

    /// Loopback, RFC 1918, Link-Local, IPv6 ULA/Loopback und `.local`.
    public static func isPrivateHost(_ host: String) -> Bool {
        let h = host.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        if h == "localhost" || h.hasSuffix(".local") || h == "::1" { return true }
        if h.contains(":") { return h.hasPrefix("fd") || h.hasPrefix("fc") || h.hasPrefix("fe80:") }
        let parts = h.split(separator: ".").compactMap { Int($0) }
        guard parts.count == 4, parts.allSatisfy({ (0...255).contains($0) }) else { return false }
        switch (parts[0], parts[1]) {
        case (127, _), (10, _), (192, 168), (169, 254): return true
        case (172, let b): return (16...31).contains(b)
        default: return false
        }
    }
}

/// Gebundene Sitzung aus `ModelGate.open`. Der einzige vorgesehene Ausführungsweg für Aufrufer.
public struct ModelSession: Sendable {
    public let provider: any ModelProvider
    public let authorization: ModelAuthorization

    public func stream(_ request: ModelRequest) -> AsyncThrowingStream<ModelEvent, Error> {
        guard request.purpose == authorization.purpose, provider.route == authorization.route else {
            return AsyncThrowingStream { $0.finish(throwing: ModelFailure.consentMissing) }
        }
        return provider.stream(request, authorization: authorization)
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

/// Speicher im Arbeitsspeicher, für Tests und kurzlebige Gespräche.
public actor InMemoryModelConsentStore: ModelConsentStore {
    private var records: [String: ModelConsentRecord] = [:]
    public init() {}
    public func consent(for scope: ModelConsentScope) -> ModelConsentRecord? { records[scope.storageKey] }
    public func record(_ consent: ModelConsentRecord) { records[consent.scope.storageKey] = consent }
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
