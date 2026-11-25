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

    func choices(for purpose: ModelPurpose) async -> [ModelChoice] {
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
