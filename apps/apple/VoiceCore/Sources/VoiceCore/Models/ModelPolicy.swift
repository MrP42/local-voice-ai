import Foundation

/// Kundenprofil für die Modellwahl. `locked`: Nutzer kann es nicht ändern.
/// Standard ist geschlossen: solange kein Profil geladen ist, gilt `.unknown` und alles Externe ist verboten.
public struct ModelPolicy: Codable, Equatable, Sendable {
    public var externalAllowed: Bool
    /// `nil` = alle Anbieter; sonst nur diese (Vergleich ohne Groß-/Kleinschreibung).
    public var allowedVendors: Set<String>?
    public var locked: Bool

    public init(externalAllowed: Bool = false, allowedVendors: Set<String>? = nil, locked: Bool = false) {
        self.externalAllowed = externalAllowed; self.allowedVendors = allowedVendors; self.locked = locked
    }

    /// Profil noch nicht geladen oder unlesbar: nur lokal.
    public static let unknown = ModelPolicy(externalAllowed: false, locked: true)
    /// Vom Kundenprofil gesperrt: nur lokal.
    public static let localOnly = ModelPolicy(externalAllowed: false, locked: true)
    /// Private Nutzung ohne Einschränkung; nur ausdrücklich setzen.
    public static let open = ModelPolicy(externalAllowed: true)

    /// Darf diese Route mit diesem Anbieter genutzt werden? `vendor` kommt aus der Auflösung, nicht aus der Anzeige.
    public func permits(route: ModelRoute, vendor: String?) -> Bool {
        if route.isLocal { return true }
        guard externalAllowed else { return false }
        guard let allowed = allowedVendors else { return true }
        guard let vendor = vendor?.lowercased(), !vendor.isEmpty else { return false }
        return allowed.contains { $0.lowercased() == vendor }
    }

    /// Nur für die Anzeige in der Auswahl. Die verbindliche Prüfung macht `ModelGate`.
    public func permits(_ choice: ModelChoice) -> Bool { permits(route: choice.route, vendor: choice.vendor) }
}
