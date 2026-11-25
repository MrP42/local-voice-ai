import Foundation

// App-Adapter fuer die gemeinsame Modellwahl. Kopplung/Schluesselbund und Hub bleiben BookTalk.
struct RelayModelProvider: ModelProvider {
    let choice: ModelChoice
    let enrollment: BookTalkRelayEnrollment
    let channel: BookTalkRelayHub
    var route: ModelRoute { choice.route }
    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let provider = RelayChatProvider(route: route, enrollment: enrollment, channel: channel)
                    for try await event in provider.stream(request, authorization: authorization) { continuation.yield(event) }
                    continuation.finish()
                } catch {
                    let failure = ModelFailure.sanitized(error)
                    if let typed = failure as? ModelFailure, typed != .stateChanged, typed != .consentMissing {
                        await MainActor.run { RelayModelStore.shared.invalidate() }
                    }
                    continuation.finish(throwing: failure)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

/// Der vorbereitete Upload startet URLSession ohne weiteres await nach confirm.
struct ModelChatHTTPUpload: ModelChatUpload {
    let request: URLRequest
    let generation: ModelEndpointGeneration.Snapshot
    func start() -> AsyncThrowingStream<Void, Error> {
        AsyncThrowingStream { continuation in
            guard !Task.isCancelled else { continuation.finish(throwing: CancellationError()); return }
            guard generation.isCurrent else { continuation.finish(throwing: ModelFailure.unavailable); return }
            let configuration = URLSessionConfiguration.ephemeral
            configuration.urlCache = nil
            configuration.httpCookieStorage = nil
            configuration.httpShouldSetCookies = false
            configuration.timeoutIntervalForRequest = 10
            configuration.timeoutIntervalForResource = 10
            let session = URLSession(configuration: configuration, delegate: ModelChatNoRedirects(), delegateQueue: nil)
            let task = session.dataTask(with: request) { data, response, error in
                defer { session.finishTasksAndInvalidate() }
                guard error == nil, (response as? HTTPURLResponse)?.statusCode == 200,
                      let data, data.count <= ModelChat.maxBytes,
                      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let results = json["results"] as? [[String: Any]], results.count == 1,
                      results[0]["outcome"] as? String == "accepted" else {
                    continuation.finish(throwing: ModelFailure.provider(.network)); return
                }
                continuation.yield(())
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel(); session.invalidateAndCancel() }
            task.resume()
        }
    }
}

private final class ModelChatNoRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

@MainActor
final class RelayModelStore {
    static let shared = RelayModelStore()
    private var providers: [RelayModelProvider] = []
    private let generation = ModelEndpointGeneration()
    private var refreshVersion = 0
    private var targetUpdate: Task<Void, Never>?
    private(set) var acceptsRequests = true
    var uploadGeneration: ModelEndpointGeneration.Snapshot { generation.snapshot() }

    func setAccountAvailable(_ available: Bool) {
        acceptsRequests = available
        invalidate()
    }

    func invalidate() {
        generation.invalidate()
        refreshVersion += 1
        let routes = providers.map(\.route)
        providers = []
        let previous = targetUpdate
        targetUpdate = Task {
            await previous?.value
            for route in routes { await AppModelCatalog.shared.accessState.setTarget(nil, for: route) }
        }
    }

    func choices() async -> [ModelChoice] {
        refreshVersion += 1
        let version = refreshVersion
        guard acceptsRequests, let account = LibrarySync.shared.account, let enrollment = RelayKeychain.load(),
              enrollment.deviceId == account.deviceId, !enrollment.isExpired(now: BookTalkRelayStore.now) else {
            invalidate(); return []
        }
        let hub = BookTalkRelayHub(account: account)
        do {
            guard let data = try await ModelChatTimeout.run(seconds: 15, operation: {
                try await hub.fetch(objectId: ModelChat.modelsId(enrollment))
            }) else { throw ModelFailure.unavailable }
            let list = try ModelChatWire.decode(data)
            let now = BookTalkRelayStore.now
            guard acceptsRequests, version == refreshVersion, RelayKeychain.load() == enrollment,
                  LibrarySync.shared.account?.deviceId == account.deviceId,
                  list.kind == .models, list.deviceId == account.deviceId, list.keyId == enrollment.keyId,
                  list.desktopDeviceId == enrollment.desktopDeviceId, list.requestId == "catalog",
                  list.isCurrent(now: now),
                  try list.isAuthentic(key: enrollment.key), let models = list.models,
                  Set(models.map { $0.choice.id }).count == models.count else { throw ModelFailure.unavailable }
            let oldRoutes = providers.map(\.route)
            generation.invalidate()
            let snapshot = generation.snapshot()
            providers = models.map { RelayModelProvider(choice: $0.choice, enrollment: enrollment, channel: hub) }
            let previous = targetUpdate
            let update = Task {
                await previous?.value
                for route in oldRoutes { await AppModelCatalog.shared.accessState.setTarget(nil, for: route) }
                for model in models {
                    await AppModelCatalog.shared.accessState.setTarget(ModelResolvedTarget(vendor: model.vendor, url: nil,
                        region: model.region, generation: snapshot), for: model.choice.route)
                }
            }
            targetUpdate = update
            await update.value
            guard version == refreshVersion else { return [] }
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(max(0, list.expiresAt - now)) * 1_000_000_000)
                guard let self, self.refreshVersion == version else { return }
                self.invalidate()
            }
            return providers.map(\.choice)
        } catch {
            if version == refreshVersion { invalidate() }
            return []
        }
    }

    func provider(for choice: ModelChoice) throws -> RelayModelProvider {
        guard let provider = providers.first(where: { $0.choice == choice }),
              RelayKeychain.load() == provider.enrollment else { throw ModelFailure.unavailable }
        return provider
    }

}
