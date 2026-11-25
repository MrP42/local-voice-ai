import Foundation
import FoundationModels

// Lokaler Adapter der gemeinsamen Modellwahl. Keine Downloads und kein externer Rückfall.
struct LocalModelProvider: ModelProvider {
    let choice: ModelChoice
    var route: ModelRoute { choice.route }
    private let runtime = LocalMinutesRuntime()

    static func available() async -> [LocalModelProvider] {
        let inventory = await ModelLibrary.shared.inventory()
        func installed(_ name: String) -> Bool { inventory.contains { $0.model.name == name && $0.installed } }
        var result: [LocalModelProvider] = []
        if ProcessInfo.processInfo.physicalMemory >= 8_000_000_000,
           installed(LocalModel.qwen3Name) {
            result.append(Self(choice: ModelChoice(route: .local(modelId: LocalModel.qwen3Name),
                label: "Qwen3-4B-Instruct-2507 · Q4_K_M", contextTokens: 4096)))
        }
        if SystemLanguageModel.default.availability == .available {
            result.append(Self(choice: ModelChoice(route: .local(modelId: "apple.foundation"),
                label: "Apple Foundation Models", contextTokens: 4096)))
        }
        let small = "qwen2.5-1.5b-instruct-q4_k_m.gguf"
        if installed(small) {
            result.append(Self(choice: ModelChoice(route: .local(modelId: small),
                label: "Qwen2.5-1.5B · Q4_K_M", contextTokens: 4096)))
        }
        return result
    }

    func countTokens(_ text: String) async -> Int? {
        guard case .local(let id) = route, id != "apple.foundation" else { return nil }
        let cancellation = InferenceCancellation()
        return await withTaskCancellationHandler {
            await runtime.count(text, model: id, cancellation: cancellation)
        } onCancel: { cancellation.cancel() }
    }

    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        AsyncThrowingStream { continuation in
            let cancellation = InferenceCancellation()
            let task = Task {
                do {
                    guard case .local(let id) = route else { throw ModelFailure.localOnly }
                    if id == "apple.foundation" {
                        guard SystemLanguageModel.default.availability == .available else { throw ModelFailure.unavailable }
                        // Die gemeinsame native Sperre bleibt über Suspensionen hinweg belegt.
                        while lv_inference_try_acquire() == 0 {
                            try Task.checkCancellation()
                            try await Task.sleep(for: .milliseconds(20))
                        }
                        defer { lv_inference_release() }
                        try Task.checkCancellation()
                        guard SystemLanguageModel.default.availability == .available else { throw ModelFailure.unavailable }
                        let parts = try Self.messages(request)
                        let session = LanguageModelSession(instructions: parts.0)
                        try await ModelProviderCheck.confirm(self, request, authorization)
                        let response = session.streamResponse(to: parts.1,
                            options: GenerationOptions(temperature: request.temperature, maximumResponseTokens: request.maxTokens))
                        var sent = ""
                        for try await snapshot in response {
                            try Task.checkCancellation()
                            // Freier Text kann umgeschrieben werden; deshalb erst die fertige Antwort liefern.
                            sent = snapshot.content
                        }
                        continuation.yield(.delta(sent))
                    } else {
                        try await runtime.generate(provider: self, request: request, authorization: authorization,
                                                   model: id, cancellation: cancellation, continuation: continuation)
                    }
                    try Task.checkCancellation()
                    continuation.yield(.done(nil)); continuation.finish()
                } catch { continuation.finish(throwing: ModelFailure.sanitized(error)) }
            }
            continuation.onTermination = { _ in cancellation.cancel(); task.cancel() }
        }
    }

    fileprivate static func messages(_ request: ModelRequest) throws -> (String, String) {
        guard request.messages.count == 2, request.messages[0].role == .system,
              request.messages[1].role == .user, request.maxTokens > 0,
              request.messages.allSatisfy({ !$0.text.contains("\0") }) else { throw ModelFailure.provider(.invalidResponse) }
        return (request.messages[0].text, request.messages[1].text)
    }
}

private actor LocalMinutesRuntime {
    private static let head = "<|im_start|>system\n"
    private static let middle = "<|im_end|>\n<|im_start|>user\n"
    private static let tail = "<|im_end|>\n<|im_start|>assistant\n"

    func count(_ text: String, model: String, cancellation: InferenceCancellation) async -> Int? {
        guard [LocalModel.qwen3Name, "qwen2.5-1.5b-instruct-q4_k_m.gguf"].contains(model),
              !text.contains("\0"), !Task.isCancelled,
              let url = try? await CPULocalProviders.shared.minutesModelURL(model),
              let handle = lv_llama_load_cancellable(url.path, 1, cancellation.pointer) else { return nil }
        defer { lv_llama_free(handle) }
        let n = lv_llama_count(handle, text, 0)
        return n < 0 ? nil : Int(n) + 32 // Chat-Rahmen und Token-Grenzen je Nachricht.
    }

    func generate(provider: LocalModelProvider, request: ModelRequest, authorization: ModelAuthorization,
                  model: String, cancellation: InferenceCancellation,
                  continuation: AsyncThrowingStream<ModelEvent, Error>.Continuation) async throws {
        try Task.checkCancellation()
        let allowed = [LocalModel.qwen3Name, "qwen2.5-1.5b-instruct-q4_k_m.gguf"]
        guard allowed.contains(model) else { throw ModelFailure.unavailable }
        if model == LocalModel.qwen3Name {
            // Das Entitlement erhöht das mögliche Limit; Laden kann trotzdem fehlschlagen.
            guard ProcessInfo.processInfo.physicalMemory >= 8_000_000_000 else { throw ModelFailure.unavailable }
        }
        let parts = try LocalModelProvider.messages(request)
        let url: URL
        do { url = try await CPULocalProviders.shared.minutesModelURL(model) }
        catch {
            try Task.checkCancellation()
            throw ModelFailure.unavailable
        }
        try Task.checkCancellation()
        guard let handle = lv_llama_load_cancellable(url.path, 0, cancellation.pointer) else {
            try Task.checkCancellation()
            throw ModelFailure.unavailable
        }
        defer { lv_llama_free(handle) }
        try Task.checkCancellation()
        // Nach der letzten Suspension folgen nur synchrone Bridge-Aufrufe.
        try await ModelProviderCheck.confirm(provider, request, authorization)
        let sink = MinutesSink(continuation)
        let code = lv_llama_chat(handle, Self.head, parts.0, Self.middle, parts.1, Self.tail,
            Int32(request.maxTokens), 4096,
            { bytes, length, context in
                guard let bytes, let context, length > 0 else { return 0 }
                let sink = Unmanaged<MinutesSink>.fromOpaque(context).takeUnretainedValue()
                let piece = String(decoding: UnsafeBufferPointer(start: UnsafeRawPointer(bytes).assumingMemoryBound(to: UInt8.self),
                                                               count: Int(length)), as: UTF8.self)
                sink.continuation.yield(.delta(piece))
                return 0
            }, Unmanaged.passUnretained(sink).toOpaque(), cancellation.pointer)
        try Task.checkCancellation()
        switch code {
        case 0: break
        case 4: throw ModelFailure.contextExceeded
        case 7: throw ModelFailure.cancelled
        default: throw ModelFailure.provider(.invalidResponse)
        }
    }
}

private final class MinutesSink {
    let continuation: AsyncThrowingStream<ModelEvent, Error>.Continuation
    init(_ continuation: AsyncThrowingStream<ModelEvent, Error>.Continuation) { self.continuation = continuation }
}
