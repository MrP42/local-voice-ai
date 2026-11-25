import Foundation

/// Woher das Kundenprofil der Modellwahl kommt (lva#801 T3b).
///
/// 1. Verwaltete App-Konfiguration (MDM, `UserDefaults` „com.apple.configuration.managed“), Schlüssel `modelPolicy`:
///    `{"externalAllowed": Bool, "allowedVendors": [String]}`. Gilt gesperrt; der Nutzer kann es nicht ändern.
///    Ist der Schlüssel vorhanden, aber unlesbar, gilt `.localOnly` (geschlossen statt offen).
///    Verwaltete Konfiguration ohne `modelPolicy` gilt ebenfalls als `.localOnly`.
/// 2. Sonst die eigene Einstellung „Externe Modelle erlauben“ (Standard aus), nicht gesperrt.
public enum ModelPolicyProfile {
    public static let managedDomain = "com.apple.configuration.managed"
    public static let managedKey = "modelPolicy"
    public static let userKey = "modelExternalAllowed"

    public static func resolve(managed: [String: Any]?, userAllowsExternal: Bool) -> ModelPolicy {
        guard let managed, !managed.isEmpty else {
            return ModelPolicy(externalAllowed: userAllowsExternal, allowedVendors: nil, locked: false)
        }
        // Verwaltetes Gerät ohne Modellprofil: die Organisation hat nichts freigegeben, also nur lokal.
        guard let raw = managed[managedKey] else { return .localOnly }
        guard let profile = raw as? [String: Any], let external = profile["externalAllowed"] as? Bool else { return .localOnly }
        var vendors: Set<String>?
        if let value = profile["allowedVendors"] {
            guard let list = value as? [String] else { return .localOnly }
            vendors = Set(list.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty })
        }
        return ModelPolicy(externalAllowed: external, allowedVendors: vendors, locked: true)
    }

    /// Erklärung für die Einstellungen; ohne Inhalte.
    public static func explanation(_ policy: ModelPolicy) -> String {
        guard policy.locked else {
            return policy.externalAllowed
                ? "Externe Modelle sind erlaubt. Vor jedem Senden fragt die App je Besprechung nach deiner Zustimmung."
                : "Alles bleibt auf dem iPhone."
        }
        guard policy.externalAllowed else { return "Deine Organisation erlaubt nur Modelle auf dem Gerät." }
        if let vendors = policy.allowedVendors, !vendors.isEmpty {
            return "Deine Organisation erlaubt externe Modelle nur von: " + vendors.sorted().joined(separator: ", ") + "."
        }
        return "Deine Organisation erlaubt externe Modelle. Vor jedem Senden fragt die App je Besprechung nach deiner Zustimmung."
    }
}
