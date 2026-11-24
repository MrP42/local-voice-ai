import Foundation

/// Kundenprofil für die Modellwahl. `locked`: Nutzer kann es nicht ändern.
public struct ModelPolicy: Codable, Equatable, Sendable {
    public var externalAllowed: Bool
    /// `nil` = alle Anbieter; sonst nur diese (Vergleich ohne Groß-/Kleinschreibung).
    public var allowedVendors: Set<String>?
    public var locked: Bool

    public init(externalAllowed: Bool = true, allowedVendors: Set<String>? = nil, locked: Bool = false) {
        self.externalAllowed = externalAllowed; self.allowedVendors = allowedVendors; self.locked = locked
    }

    public static let localOnly = ModelPolicy(externalAllowed: false, locked: true)

    public func permits(_ choice: ModelChoice) -> Bool {
        guard choice.isExternal else { return true }
        guard externalAllowed else { return false }
        guard let allowed = allowedVendors else { return true }
        guard let vendor = choice.vendor?.lowercased() else { return false }
        return allowed.contains { $0.lowercased() == vendor }
    }
}
