import XCTest
@testable import VoiceCore

final class ReviewRegressionTests: XCTestCase {
    func testReplyCannotEraseOrReplacePersistedTranscript() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let s = try DurableStore(root: root), p = Packet.capture(audio: Data([1]))
        _ = try s.accept(p); try s.update(p.sessionId, transcript: "Original", state: .deferred)
        XCTAssertThrowsError(try s.acceptReply(VoiceEnvelope(sessionId: p.sessionId, transcript: nil, reply: "Antwort", replyMessageId: UUID())))
        XCTAssertEqual(try s.entries().first?.transcript, "Original")
    }
    func testIncomingReplyCannotReplaceAnIssuedResponseIdentity() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let s = try DurableStore(root: root), p = Packet.capture(audio: Data([1]))
        _ = try s.accept(p); try s.update(p.sessionId, transcript: "Text", reply: "Antwort", state: .answered)
        let issued = try s.replyEnvelope(for: p.sessionId)
        XCTAssertThrowsError(try s.acceptReply(VoiceEnvelope(sessionId: p.sessionId, transcript: "Text", reply: "Antwort", replyMessageId: UUID())))
        XCTAssertEqual(try s.replyEnvelope(for: p.sessionId).messageId, issued.messageId)
    }
    func testRecordingAdmissionReservesSpaceInConfirmedQuota() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let s = try DurableStore(root: root, limit: 1024 * 1024)
        _ = try s.accept(.capture(audio: Data([1])))
        XCTAssertFalse(try s.canStartRecording())
    }
    func testMissingAudioDoesNotBlockUnrelatedCaptureWithKnownIdentity() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let s = try DurableStore(root: root), p = Packet.capture(audio: Data([1]))
        _ = try s.accept(p); try FileManager.default.removeItem(at: s.audioURL(for: p.sessionId)); s.invalidateInventory()
        let healthy = Packet.capture(audio: Data([2]))
        XCTAssertNoThrow(try s.accept(healthy))
        XCTAssertEqual(try s.inventory().issues.first?.sessionId, p.sessionId)
    }
}
