import Foundation

/// Nachweis, dass die Person der Verarbeitung durch einen externen Anbieter zugestimmt hat,
/// je Gegenstand (Besprechung, Gespräch), Zweck und Route.
public struct ModelConsentRecord: Codable, Hashable, Sendable {
    public let subjectId: String
    public let purpose: ModelPurpose
    public let route: ModelRoute
    public let vendor: String
    /// Was das Gerät verlässt, z. B. „Transkript“, „Teilnehmernamen“, „Gesprochene Eingabe“.
    public let dataCategories: [String]
    public let grantedAt: Date

    public init(subjectId: String, purpose: ModelPurpose, route: ModelRoute, vendor: String, dataCategories: [String], grantedAt: Date) {
        self.subjectId = subjectId; self.purpose = purpose; self.route = route; self.vendor = vendor
        self.dataCategories = dataCategories; self.grantedAt = grantedAt
    }

    /// Text für Dialog und Nachweis. Keine Inhalte, nur Kategorien.
    public static func summary(choice: ModelChoice, dataCategories: [String]) -> String {
        let vendor = choice.vendor ?? choice.label
        let place = choice.region.map { " (Verarbeitung: \($0))" } ?? ""
        let via: String
        if case .pcRelay = choice.route { via = " über deinen PC" } else { via = "" }
        return "\(dataCategories.joined(separator: ", ")) gehen\(via) an \(vendor)\(place) und werden dort mit \(choice.label) verarbeitet."
    }
}

public protocol ModelConsentStore: Sendable {
    func consent(subjectId: String, purpose: ModelPurpose, route: ModelRoute) async -> ModelConsentRecord?
    func record(_ consent: ModelConsentRecord) async throws
}

/// Einzige Prüfstelle vor jedem Modellaufruf.
public enum ModelGate {
    public static func check(_ choice: ModelChoice, purpose: ModelPurpose, subjectId: String,
                             policy: ModelPolicy, consents: any ModelConsentStore) async throws {
        if purpose.localOnly && !choice.route.isLocal { throw ModelFailure.localOnly }
        guard policy.permits(choice) else { throw ModelFailure.blockedByPolicy }
        guard choice.isExternal else { return }
        guard await consents.consent(subjectId: subjectId, purpose: purpose, route: choice.route) != nil else {
            throw ModelFailure.consentMissing
        }
    }
}

/// Speicher im Arbeitsspeicher, für Tests und kurzlebige Gespräche.
public actor InMemoryModelConsentStore: ModelConsentStore {
    private var records: [String: ModelConsentRecord] = [:]
    public init() {}
    private static func key(_ s: String, _ p: ModelPurpose, _ r: ModelRoute) -> String { "\(s)|\(p.rawValue)|\(r.key)" }
    public func consent(subjectId: String, purpose: ModelPurpose, route: ModelRoute) -> ModelConsentRecord? {
        records[Self.key(subjectId, purpose, route)]
    }
    public func record(_ consent: ModelConsentRecord) {
        records[Self.key(consent.subjectId, consent.purpose, consent.route)] = consent
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
