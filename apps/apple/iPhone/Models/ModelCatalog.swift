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
    private var defaultsObserver: NSObjectProtocol?
    private var policyChain: Task<ModelPolicy, Never>?

    private init() {
        // MDM-Konfiguration und eigener Schalter liegen in UserDefaults: jede Änderung sofort anwenden,
        // damit eine laufende externe Auswertung beim nächsten Senden am neuen Profil scheitert.
        defaultsObserver = NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { _ in
            Task { @MainActor in await AppModelCatalog.shared.refreshPolicy() }
        }
        Task { @MainActor in await AppModelCatalog.shared.refreshPolicy() }
    }

    /// Kundenprofil neu lesen: verwaltete Konfiguration (MDM) vor der eigenen Einstellung, siehe `ModelPolicyProfile`.
    @discardableResult
    func refreshPolicy() async -> ModelPolicy {
        // Geordnet: jeder Lauf wartet auf den vorigen und liest die Einstellungen erst dann; so gewinnt immer der jüngste Stand.
        let previous = policyChain
        let state = accessState
        let run = Task { @MainActor [weak self] () -> ModelPolicy in
            _ = await previous?.value
            let defaults = UserDefaults.standard
            let next = ModelPolicyProfile.resolve(managed: defaults.dictionary(forKey: ModelPolicyProfile.managedDomain),
                                                  userAllowsExternal: defaults.bool(forKey: ModelPolicyProfile.userKey))
            self?.policy = next
            await state.updatePolicy(next)
            return next
        }
        policyChain = run
        return await run.value
    }

    func choices(for purpose: ModelPurpose) async -> [ModelChoice] {
        await refreshPolicy()
        locals = await LocalModelProvider.available()
        let store = AssistantEndpointStore.shared
        await store.modelTargetReady()
        var choices = locals.map(\.choice)
        if !purpose.localOnly, let choice = store.modelChoice { choices.append(choice) }
        if !purpose.localOnly { choices += await RelayModelStore.shared.choices() }
        return choices
    }

    func provider(for choice: ModelChoice) throws -> any ModelProvider {
        if let local = locals.first(where: { $0.choice == choice }) { return local }
        if case .pcRelay = choice.route { return try RelayModelStore.shared.provider(for: choice) }
        let store = AssistantEndpointStore.shared
        if store.modelChoice == choice, let endpoint = store.endpoint {
            return EndpointModelProvider(choice: choice, endpoint: endpoint)
        }
        throw ModelFailure.unavailable
    }
}
