import Foundation

// App-Adapter: Einstellungen und Schlüsselbund bleiben die einzige Quelle für Endpunkte.
struct EndpointModelProvider: ModelProvider {
    let choice: ModelChoice
    let endpoint: AssistantEndpoint
    var route: ModelRoute { choice.route }

    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        do {
            let clean = try AssistantEndpointStore.validated(endpoint)
            guard let url = clean.chatURL, authorization.permitsSending(to: url) else { throw ModelFailure.consentMissing }
            let core = EndpointChatProvider(route: route, url: url, model: clean.model,
                transport: EndpointURLTransport(), key: { [route] in
                    try await AssistantEndpointStore.shared.modelKey(route: route, endpoint: clean)
                })
            return core.stream(request, authorization: authorization)
        } catch {
            return AsyncThrowingStream { $0.finish(throwing: ModelFailure.sanitized(error)) }
        }
    }
}

struct EndpointURLTransport: EndpointChatTransport {
    func start(_ request: URLRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<EndpointTransportEvent, Error> {
        AsyncThrowingStream { continuation in
            let configuration = URLSessionConfiguration.ephemeral
            configuration.urlCache = nil
            configuration.httpCookieStorage = nil
            configuration.httpShouldSetCookies = false
            configuration.requestCachePolicy = .reloadIgnoringLocalAndRemoteCacheData
            configuration.waitsForConnectivity = false
            configuration.timeoutIntervalForRequest = 30
            configuration.timeoutIntervalForResource = 300
            let delegate = EndpointURLDelegate(authorization: authorization, continuation: continuation)
            let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
            let task = session.dataTask(with: request)
            continuation.onTermination = { _ in task.cancel(); session.invalidateAndCancel() }
            task.resume()
        }
    }
}

private final class EndpointURLDelegate: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let authorization: ModelAuthorization
    private let continuation: AsyncThrowingStream<EndpointTransportEvent, Error>.Continuation
    init(authorization: ModelAuthorization, continuation: AsyncThrowingStream<EndpointTransportEvent, Error>.Continuation) {
        self.authorization = authorization; self.continuation = continuation
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        // Kein zweiter Sendestart mit derselben Berechtigung, auch nicht auf demselben Host.
        completionHandler(nil)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let http = response as? HTTPURLResponse, let url = http.url,
              authorization.permitsSending(to: url) else {
            completionHandler(.cancel)
            continuation.finish(throwing: ModelFailure.consentMissing)
            return
        }
        continuation.yield(.response(status: http.statusCode, url: url,
            isSSE: http.mimeType?.lowercased() == "text/event-stream"))
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        continuation.yield(.data(data))
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error { continuation.finish(throwing: error) }
        else { continuation.finish() }
    }
}
