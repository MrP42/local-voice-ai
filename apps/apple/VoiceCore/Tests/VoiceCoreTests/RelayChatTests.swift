import XCTest
@testable import VoiceCore

private final class RelayUploadProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var sent: Int { lock.lock(); defer { lock.unlock() }; return count }
    func hit() { lock.lock(); count += 1; lock.unlock() }
}
private struct RelayTestUpload: ModelChatUpload {
    let probe: RelayUploadProbe
    func start() -> AsyncThrowingStream<Void, Error> {
        probe.hit()
        return AsyncThrowingStream { $0.yield(()); $0.finish() }
    }
}
private actor RelayTestChannel: ModelChatChannel {
    let probe: RelayUploadProbe
    let revoke: ModelAccessState?
    private var wire: ModelChatWire?
    init(probe: RelayUploadProbe, revoke: ModelAccessState? = nil) { self.probe = probe; self.revoke = revoke }
    func prepare(objectId: String, json: Data) async throws -> any ModelChatUpload {
        wire = try ModelChatWire.decode(json)
        if let revoke { await revoke.revokeAll(subjectId: "synthetic") }
        return RelayTestUpload(probe: probe)
    }
    func put(objectId: String, json: Data) async throws {}
    func remove(objectId: String) async throws {}
    func fetch(objectId: String) async throws -> Data? {
        guard var response = wire else { return nil }
        response.kind = .resp; response.request = nil
        response.chunks = [.init(seq: 1, text: "synthetic")]; response.done = true
        try response.sign(key: Array(repeating: 7, count: 32))
        return try JSONEncoder().encode(response)
    }
}

final class RelayChatTests: XCTestCase, @unchecked Sendable {
    private func vectors() throws -> [Data] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("tooling/fixtures/model-chat/golden.json"))
        let json = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try XCTUnwrap(json["vectors"] as? [[String: Any]]).map { entry in
            let data = try JSONSerialization.data(withJSONObject: entry["wire"]!)
            let wire = try JSONDecoder().decode(ModelChatWire.self, from: data)
            XCTAssertEqual(try wire.canonical(), entry["canonical"] as? String)
            XCTAssertTrue(try wire.isAuthentic(key: Array(repeating: 7, count: 32)))
            XCTAssertFalse(try wire.isAuthentic(key: Array(repeating: 8, count: 32)))
            return data
        }
    }

    func testSharedVectorsAndChunks() throws {
        let wires = try vectors().map { try JSONDecoder().decode(ModelChatWire.self, from: $0) }
        var response = try XCTUnwrap(wires.first { $0.kind == .resp })
        var cursor = ModelChatCursor()
        XCTAssertEqual(try cursor.accept(response), [.delta("synthetic"), .done(nil)])
        XCTAssertEqual(try cursor.accept(response), [])
        response.chunks = [.init(seq: 2, text: "gap")]
        XCTAssertThrowsError(try ModelChatCursor().validated(response))
        response.chunks = [.init(seq: 1, text: "a"), .init(seq: 1, text: "b")]
        XCTAssertThrowsError(try ModelChatCursor().validated(response))
    }

    func testDeadline() throws {
        XCTAssertFalse(ModelChatDeadline(start: 10).expired(now: 24, received: false))
        XCTAssertTrue(ModelChatDeadline(start: 10).expired(now: 25, received: false))
        XCTAssertFalse(ModelChatDeadline(start: 10).expired(now: 25, received: true))
        XCTAssertTrue(ModelChatDeadline(start: 10).expired(now: 85, received: true))
    }

    func testRevocationDuringPreparationPreventsUploadWithPositiveControl() async throws {
        let route = ModelRoute.pcRelay(provider: "synthetic-provider", model: "synthetic-model")
        let enrollment = BookTalkRelayEnrollment(keyId: "synthetic-key", key: Array(repeating: 7, count: 32),
            desktopDeviceId: "synthetic-pc", desktopName: "PC", deviceId: "synthetic-phone", expiresAt: Int.max)
        for revoked in [false, true] {
            let state = ModelAccessState(policy: .open, targets: [route: .init(vendor: "Example", url: nil)])
            let (scope, _) = try await state.consentScope(route: route, purpose: .meetingMinutes,
                subjectId: "synthetic", dataCategories: ["Transkript"])!
            await state.record(.init(scope: scope, grantedAt: Date(timeIntervalSince1970: 1)))
            let probe = RelayUploadProbe()
            let provider = RelayChatProvider(route: route, enrollment: enrollment,
                channel: RelayTestChannel(probe: probe, revoke: revoked ? state : nil))
            let session = try await ModelGate.open(provider, purpose: .meetingMinutes,
                subjectId: "synthetic", dataCategories: ["Transkript"], state: state)
            var events: [ModelEvent] = []
            do {
                for try await event in session.stream(.init(purpose: .meetingMinutes,
                    messages: [.init(role: .user, text: "synthetic")])) { events.append(event) }
                XCTAssertFalse(revoked)
                XCTAssertEqual(events, [.delta("synthetic"), .done(nil)])
            } catch { XCTAssertTrue(revoked); XCTAssertEqual(error as? ModelFailure, .consentMissing) }
            XCTAssertEqual(probe.sent, revoked ? 0 : 1)
        }
    }

    func testTimeoutStopsOperationWithPositiveControl() async throws {
        let value = try await ModelChatTimeout.run(seconds: 1) { 42 }
        XCTAssertEqual(value, 42)
        do {
            _ = try await ModelChatTimeout.run(seconds: 0) { 42 }
            XCTFail("expired operation started")
        } catch { XCTAssertEqual(error as? ModelFailure, .timeout) }
    }
}
