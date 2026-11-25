import Foundation

@MainActor
protocol ModelCatalog {
    func choices(for purpose: ModelPurpose) async -> [ModelChoice]
    func provider(for choice: ModelChoice) throws -> any ModelProvider
}

@MainActor
final class AppModelCatalog: ModelCatalog {
    static let shared = AppModelCatalog()
    /// Gemeinsamer Stand für Besprechung und Sprechen; bis zum Laden des Kundenprofils nur lokal.
    let accessState = ModelAccessState()
    private var locals: [LocalModelProvider] = []
    /// Zuletzt angewandtes Kundenprofil (Anzeige); verbindlich ist der Stand in `accessState`.
    private(set) var policy: ModelPolicy = .unknown

    /// Kundenprofil neu lesen: verwaltete Konfiguration (MDM) vor der eigenen Einstellung, siehe `ModelPolicyProfile`.
    @discardableResult
    func refreshPolicy() async -> ModelPolicy {
        let defaults = UserDefaults.standard
        let next = ModelPolicyProfile.resolve(managed: defaults.dictionary(forKey: ModelPolicyProfile.managedDomain),
                                              userAllowsExternal: defaults.bool(forKey: ModelPolicyProfile.userKey))
        if next != policy || (await accessState.currentPolicy()) != next {
            policy = next
            await accessState.setPolicy(next)
        }
        return next
    }

    func choices(for purpose: ModelPurpose) async -> [ModelChoice] {
        await refreshPolicy()
        locals = await LocalModelProvider.available()
        let store = AssistantEndpointStore.shared
        await store.modelTargetReady()
        var choices = locals.map(\.choice)
        if !purpose.localOnly, let choice = store.modelChoice { choices.append(choice) }
        return choices
    }

    func provider(for choice: ModelChoice) throws -> any ModelProvider {
        if let local = locals.first(where: { $0.choice == choice }) { return local }
        let store = AssistantEndpointStore.shared
        if store.modelChoice == choice, let endpoint = store.endpoint {
            return EndpointModelProvider(choice: choice, endpoint: endpoint)
        }
        throw ModelFailure.unavailable
    }
}
