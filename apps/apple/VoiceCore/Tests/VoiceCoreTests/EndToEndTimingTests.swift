import XCTest
@testable import VoiceCore

final class EndToEndTimingTests: XCTestCase {
    func testPersistedClockOriginAndFirstReplyTimingSurviveDuplicatesAndRestart() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet)
        let origin = try XCTUnwrap(store.entries().first?.timings["capture_saved_at_ms"])
        XCTAssertGreaterThan(origin, 0)
        let reply = VoiceEnvelope(sessionId: packet.sessionId, transcript: "Test", reply: "Erhalten.")
        _ = try store.acceptReply(reply)
        let first = try XCTUnwrap(store.entries().first)
        XCTAssertGreaterThanOrEqual(try XCTUnwrap(first.timings["reply_e2e_ms"]), 0)
        let restarted = try DurableStore(root: root)
        _ = try restarted.accept(packet); _ = try restarted.acceptReply(reply)
        XCTAssertEqual(try restarted.entries().first?.timings, first.timings)
        XCTAssertEqual(try restarted.entries().first?.timings["capture_saved_at_ms"], origin)
    }
}
