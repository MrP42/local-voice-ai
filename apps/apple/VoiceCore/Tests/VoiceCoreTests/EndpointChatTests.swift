import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import XCTest
@testable import VoiceCore

private final class EndpointProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    private var stopped = false
    var sent: Int { lock.lock(); defer { lock.unlock() }; return count }
    var cancelled: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
    func hit() { lock.lock(); count += 1; lock.unlock() }
    func cancel() { lock.lock(); stopped = true; lock.unlock() }
}

private struct EndpointWaitingTransport: EndpointChatTransport {
    let probe: EndpointProbe
    var timeout = false
    func start(_ request: URLRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<EndpointTransportEvent, Error> {
        AsyncThrowingStream { continuation in
            continuation.onTermination = { _ in probe.cancel() }
            probe.hit()
            if timeout { continuation.finish(throwing: URLError(.timedOut, userInfo: [NSLocalizedDescriptionKey: "synthetic-secret"])) }
        }
    }
}

private struct EndpointFakeTransport: EndpointChatTransport {
    let probe: EndpointProbe
    var status = 200
    var body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hallo\"}}]}\n\ndata: [DONE]\n\n"
    func start(_ request: URLRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<EndpointTransportEvent, Error> {
        probe.hit()
        return AsyncThrowingStream {
            $0.yield(.response(status: status, url: request.url!, isSSE: true))
            for byte in body.utf8 { $0.yield(.data(Data([byte]))) }
            $0.finish()
        }
    }
}

final class EndpointChatTests: XCTestCase, @unchecked Sendable {
    private let url = URL(string: "https://api.example.test/v1/chat/completions")!
    private let route = ModelRoute.endpoint(id: UUID(uuidString: "00000000-0000-0000-0000-000000000004")!)
    private var request: ModelRequest {
        ModelRequest(purpose: .meetingMinutes, messages: [.init(role: .user, text: "Synthetisch")],
                     maxTokens: 123, temperature: 0.3, jsonSchema: "{\"type\":\"object\"}")
    }

    private func state() async throws -> ModelAccessState {
        let state = ModelAccessState(policy: .open, targets: [route: .init(vendor: "Test", url: url)])
        let (scope, _) = try await state.consentScope(route: route, purpose: .meetingMinutes,
            subjectId: "synthetic", dataCategories: ["Transkript"])!
        await state.record(.init(scope: scope, grantedAt: Date(timeIntervalSince1970: 1)))
        return state
    }

    private func run(_ provider: EndpointChatProvider, state: ModelAccessState) async throws -> String {
        let session = try await ModelGate.open(provider, purpose: .meetingMinutes, subjectId: "synthetic",
            dataCategories: ["Transkript"], state: state)
        return try await session.complete(request)
    }

    func testRequestUsesSharedBuilderAndSchemaWithoutKeyInBody() throws {
        let data = try AssistantSSE.requestBody(model: "synthetic", request: request)
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(body["max_tokens"] as? Int, 123)
        XCTAssertEqual(body["temperature"] as? Double, 0.3)
        XCTAssertEqual(body["stream"] as? Bool, true)
        let messages = try XCTUnwrap(body["messages"] as? [[String: String]])
        XCTAssertTrue(messages[0]["content"]!.contains(request.jsonSchema!))
        XCTAssertEqual(messages.last?["content"], "Synthetisch")
        var invalid = request; invalid.maxTokens = 0
        XCTAssertThrowsError(try AssistantSSE.requestBody(model: "test", request: invalid))
        var capped = request; capped.maxTokens = 9000
        let cappedBody = try XCTUnwrap(JSONSerialization.jsonObject(with: AssistantSSE.requestBody(model: "test", request: capped)) as? [String: Any])
        XCTAssertEqual(cappedBody["max_tokens"] as? Int, 4000)
        var embedded = request
        embedded.messages.insert(.init(role: .system, text: request.jsonSchema!), at: 0)
        let embeddedBody = try XCTUnwrap(JSONSerialization.jsonObject(with: AssistantSSE.requestBody(model: "test", request: embedded)) as? [String: Any])
        XCTAssertEqual((embeddedBody["messages"] as? [[String: String]])?.count, 2)
    }

    func testSplitChunksDoneAndTruncatedJSON() throws {
        var parser = EndpointChatParser(isSSE: true)
        let wire = "data: {\"choices\":[{\"delta\":{\"content\":\"Grüße\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n"
        var events: [ModelEvent] = []
        for byte in wire.utf8 { events += try parser.append(Data([byte])) }
        events += try parser.finish()
        XCTAssertEqual(events, [.delta("Grüße"), .done(nil)])
        var truncated = EndpointChatParser(isSSE: true)
        _ = try truncated.append(Data("data: {\"choices\":".utf8))
        XCTAssertThrowsError(try truncated.finish())
        var missingDone = EndpointChatParser(isSSE: true)
        _ = try missingDone.append(Data("data: {\"choices\":[{\"delta\":{\"content\":\"X\"}}]}\n\n".utf8))
        XCTAssertThrowsError(try missingDone.finish())
        var json = EndpointChatParser(isSSE: false)
        _ = try json.append(Data("{\"choices\":[{\"message\":{\"content\":\"JSON\"}}]}".utf8))
        XCTAssertEqual(try json.finish(), [.delta("JSON"), .done(nil)])
        var broken = EndpointChatParser(isSSE: false)
        _ = try broken.append(Data("{\"choices\":[".utf8))
        XCTAssertThrowsError(try broken.finish())
        var refused = EndpointChatParser(isSSE: true)
        XCTAssertThrowsError(try refused.append(Data("data: {\"error\":{\"message\":\"synthetic-secret\"}}\n".utf8))) { error in
            XCTAssertEqual(error as? ModelFailure, .provider(.invalidResponse))
            XCTAssertFalse(error.localizedDescription.contains("synthetic-secret"))
        }
        var length = EndpointChatParser(isSSE: true)
        XCTAssertThrowsError(try length.append(Data("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n".utf8))) { error in
            XCTAssertEqual(error as? ModelFailure, .contextExceeded)
        }
    }

    func testRedirectOtherHostAndDowngradeRejected() {
        XCTAssertFalse(EndpointChatProvider.permitsRedirect(from: url, to: URL(string: "https://other.test/v1/chat/completions")!))
        XCTAssertFalse(EndpointChatProvider.permitsRedirect(from: url, to: URL(string: "http://api.example.test/v1/chat/completions")!))
        XCTAssertTrue(EndpointChatProvider.permitsRedirect(from: url, to: url))
    }

    func testRedeemAfterKeyReadRevokedConsentSendsNothing() async throws {
        let state = try await self.state()
        let probe = EndpointProbe()
        let provider = EndpointChatProvider(route: route, url: url, model: "test",
            transport: EndpointFakeTransport(probe: probe), key: {
                await state.revokeAll(subjectId: "synthetic")
                return "synthetic-secret"
            })
        do { _ = try await run(provider, state: state); XCTFail("revoked") }
        catch { XCTAssertEqual(error as? ModelFailure, .consentMissing) }
        XCTAssertEqual(probe.sent, 0)
        let allowed = EndpointChatProvider(route: route, url: url, model: "test",
            transport: EndpointFakeTransport(probe: probe), key: { "synthetic-secret" })
        let fresh = try await self.state()
        let text = try await run(allowed, state: fresh)
        XCTAssertEqual(text, "Hallo")
        XCTAssertEqual(probe.sent, 1)
    }

    func testHTTPFailuresContainNeitherKeyNorBody() async throws {
        for status in [401, 403, 413, 429, 500] {
            let probe = EndpointProbe()
            let provider = EndpointChatProvider(route: route, url: url, model: "test",
                transport: EndpointFakeTransport(probe: probe, status: status, body: "synthetic-secret"),
                key: { "synthetic-secret" })
            let state = try await self.state()
            do { _ = try await run(provider, state: state); XCTFail("HTTP failure") }
            catch {
                XCTAssertEqual(error as? ModelFailure, status == 413 ? .contextExceeded : .provider(ModelProviderError(status: status)))
                XCTAssertFalse(String(describing: error).contains("synthetic-secret"))
                XCTAssertFalse(error.localizedDescription.contains("synthetic-secret"))
            }
        }
    }

    func testCancellationTerminatesTransportAndTimeoutIsSanitized() async throws {
        let probe = EndpointProbe()
        let provider = EndpointChatProvider(route: route, url: url, model: "test",
            transport: EndpointWaitingTransport(probe: probe), key: { nil })
        let state = try await self.state()
        let task = Task { try await run(provider, state: state) }
        for _ in 0..<10_000 {
            if probe.sent > 0 { break }
            await Task.yield()
        }
        XCTAssertEqual(probe.sent, 1)
        task.cancel()
        do { _ = try await task.value; XCTFail("cancelled") }
        catch { XCTAssertTrue(error is CancellationError) }
        // Die Beendigung der verschachtelten Streams darf asynchron auslaufen.
        for _ in 0..<10_000 {
            if probe.cancelled { break }
            await Task.yield()
        }
        XCTAssertTrue(probe.cancelled)
        let timed = EndpointChatProvider(route: route, url: url, model: "test",
            transport: EndpointWaitingTransport(probe: EndpointProbe(), timeout: true), key: { nil })
        do { _ = try await run(timed, state: state); XCTFail("timeout") }
        catch {
            XCTAssertEqual(error as? ModelFailure, .timeout)
            XCTAssertFalse(error.localizedDescription.contains("synthetic-secret"))
        }
    }

    func testStepsAndFollowUpsUseSameQuoteAndAttributionGuards() throws {
        let segment = LiveSegment(id: "a", start: 0, end: 1, text: "Ada prüft morgen den Test.", isFinal: true)
        for kind in ["next_steps", "follow_ups", "task"] {
            func reply(_ text: String, _ assignee: String) -> String {
                "{\"items\":[{\"kind\":\"\(kind)\",\"text\":\"\(text)\",\"quote\":\"Ada prüft morgen den Test.\",\"assignee\":\"\(assignee)\",\"due\":\"morgen\",\"evidence\":[\"a\"]}]}"
            }
            let valid = try MinutesResult.decode(reply("prüft morgen den Test", "Ada"), segments: [segment])
            XCTAssertEqual(valid.items.count, 1)
            XCTAssertEqual(try MinutesResult.decode(reply("Erfundene Aussage", "Ada"), segments: [segment]).rejected, 1)
            XCTAssertEqual(try MinutesResult.decode(reply("prüft morgen den Test", "Bob"), segments: [segment]).rejected, 1)
            XCTAssertEqual(try MinutesResult.decode(reply("prüft morgen den Test", "Ada"), segments: [segment], originalQuotes: []).rejected, 1)
        }
    }
}
