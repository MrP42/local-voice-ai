import XCTest
@testable import VoiceCore

final class JobTests: XCTestCase {
    func testRetriesPersistAcrossRestartAndStopAfterThreeFailures() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data([1])); _ = try DurableStore(root: root).accept(packet)
        let start = Date(timeIntervalSince1970: 1000)
        for attempt in 0..<3 {
            let store = try DurableStore(root: root)
            let now = start.addingTimeInterval(Double(attempt * 1000))
            XCTAssertTrue(try store.beginJob(packet.sessionId, now: now))
            try store.failJob(packet.sessionId, reason: .unavailable, now: now)
            XCTAssertFalse(try store.beginJob(packet.sessionId, now: now))
        }
        let store = try DurableStore(root: root)
        XCTAssertFalse(try store.beginJob(packet.sessionId, now: start.addingTimeInterval(10000)))
        XCTAssertEqual(try store.entries().first?.job?.attempts, 3)
        XCTAssertEqual(try store.entries().first?.job?.phase, .failed)
        try store.retryJob(packet.sessionId)
        XCTAssertTrue(try store.beginJob(packet.sessionId, now: start.addingTimeInterval(10001)))
        XCTAssertEqual(try store.audio(for: packet.sessionId), packet.audio)
    }
    func testInterruptedGenerationRetainsTranscriptAndCannotBeRunTwice() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1])); _ = try store.accept(packet)
        XCTAssertTrue(try store.beginJob(packet.sessionId))
        XCTAssertFalse(try store.beginJob(packet.sessionId))
        try store.update(packet.sessionId, transcript: "Ein unverändertes Transkript.", state: .deferred)
        try store.setJobPhase(packet.sessionId, phase: .generating)
        let restarted = try DurableStore(root: root)
        try restarted.recoverInterruptedJobs()
        XCTAssertEqual(try restarted.entries().first?.job?.phase, .paused)
        XCTAssertEqual(try restarted.entries().first?.transcript, "Ein unverändertes Transkript.")
        XCTAssertTrue(try restarted.beginJob(packet.sessionId))
        XCTAssertEqual(try restarted.entries().first?.job?.phase, .generating)
    }
    func testAnsweredJobCannotBeRetriedOrRewrittenByLateCancellation() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1])); _ = try store.accept(packet)
        XCTAssertTrue(try store.beginJob(packet.sessionId))
        try store.update(packet.sessionId, reply: "Erhalten.", state: .answered)
        try store.pauseJob(packet.sessionId)
        try store.retryJob(packet.sessionId)
        XCTAssertFalse(try store.beginJob(packet.sessionId))
        XCTAssertEqual(try store.entries().first?.state, .answered)
        XCTAssertEqual(try store.entries().first?.reply, "Erhalten.")
    }
}
