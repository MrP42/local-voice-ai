import XCTest
@testable import VoiceCore

final class ReplyTests: XCTestCase {
    func testOversizedTranscriptCannotCreateUndeliverableAnswer() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let capture = Packet.capture(audio: Data([1]))
        _ = try store.accept(capture)
        XCTAssertThrowsError(try store.update(capture.sessionId, transcript: String(repeating: "x", count: 16001), reply: "answer", state: .answered))
        XCTAssertNil(try store.entries().first?.reply)
        XCTAssertEqual(try store.entries().first?.state, .saved)
    }
    func testUnsendableGeneratedReplyDoesNotCompleteCapture() throws {
        for reply in ["", "  \n", String(repeating: "x", count: 501)] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let store = try DurableStore(root: root)
            let capture = Packet.capture(audio: Data([1]))
            _ = try store.accept(capture)
            XCTAssertThrowsError(try store.update(capture.sessionId, reply: reply, state: .answered))
            let entry = try XCTUnwrap(store.entries().first)
            XCTAssertNil(entry.reply)
            XCTAssertEqual(entry.state, .saved)
            XCTAssertEqual(try store.audio(for: entry.id), capture.audio)
        }
    }
    func testWhitespaceIncomingReplyIsNotAcknowledged() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let capture = Packet.capture(audio: Data([1]))
        _ = try store.accept(capture)
        XCTAssertThrowsError(try store.acceptReply(VoiceEnvelope(sessionId: capture.sessionId, reply: " \n")))
        XCTAssertNil(try store.entries().first?.replyReceipt)
    }
    func testReplyAndReceiptSurviveRestartWithoutDuplicatePlayback() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let phone = try DurableStore(root: root.appendingPathComponent("phone"))
        let watch = try DurableStore(root: root.appendingPathComponent("watch"))
        let capture = Packet.capture(audio: Data([1]))
        _ = try phone.accept(capture); _ = try watch.accept(capture)
        try phone.update(capture.sessionId, transcript: "Original", reply: "Antwort", state: .answered)
        let answer = try phone.replyEnvelope(for: capture.sessionId)
        let first = try watch.acceptReply(answer)
        XCTAssertTrue(first.isNew)
        let watchRestart = try DurableStore(root: watch.root)
        let repeated = try watchRestart.acceptReply(answer)
        XCTAssertFalse(repeated.isNew)
        XCTAssertEqual(first.receipt, repeated.receipt)
        let phoneRestart = try DurableStore(root: phone.root)
        XCTAssertEqual(try phoneRestart.replyEnvelope(for: capture.sessionId).messageId, answer.messageId)
        try phoneRestart.acknowledgeReply(repeated.receipt)
        XCTAssertTrue(try XCTUnwrap(phoneRestart.entries().first).replyAcknowledged == true)
    }
    func testUnknownAndConflictingRepliesAreRejected() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let capture = Packet.capture(audio: Data([1])); _ = try store.accept(capture)
        XCTAssertThrowsError(try store.acceptReply(VoiceEnvelope(sessionId: UUID(), reply: "unknown")))
        let answer = VoiceEnvelope(sessionId: capture.sessionId, reply: "original")
        _ = try store.acceptReply(answer)
        var conflict = answer; conflict.payload.reply = "changed"
        XCTAssertThrowsError(try store.acceptReply(conflict))
        XCTAssertEqual(try store.entries().first?.reply, "original")
    }
    func testReceiptReplayPreservesReplyDestination() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let local = Packet.capture(audio: Data([1])); _ = try store.accept(local)
        XCTAssertEqual(try store.entries().first?.replyToWatch, false)
        let remote = Packet.capture(audio: Data([2]))
        let receipt = try store.accept(remote, replyToWatch: true)
        XCTAssertEqual(try store.accept(remote), receipt)
        XCTAssertEqual(try store.entries().first(where: { $0.id == remote.sessionId })?.replyToWatch, true)
    }
    func testWrongReceiptCannotSilencePendingResponse() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let capture = Packet.capture(audio: Data([1])); _ = try store.accept(capture)
        try store.update(capture.sessionId, reply: "answer", state: .answered)
        XCTAssertThrowsError(try store.acknowledgeReply(Receipt(sessionId: capture.sessionId, messageId: UUID(), receiptId: UUID())))
        XCTAssertNotEqual(try store.entries().first?.replyAcknowledged, true)
    }
}
