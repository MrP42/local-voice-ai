import XCTest
@testable import VoiceCore

final class WriteFailureTests: XCTestCase {
    func testFullDiskBeforeCaptureWriteDoesNotAcknowledgeOrModifyPreviousCapture() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let previous = Packet.capture(audio: Data([1])); let receipt = try DurableStore(root: root).accept(previous)
        let faulty = try DurableStore(root: root, fault: { checkpoint in
            if checkpoint == .beforeWrite { throw NSError(domain: NSPOSIXErrorDomain, code: 28) }
        })
        XCTAssertThrowsError(try faulty.accept(.capture(audio: Data([2]))))
        let restarted = try DurableStore(root: root)
        XCTAssertEqual(try restarted.accept(previous), receipt)
        XCTAssertEqual(try restarted.entries().count, 1)
        XCTAssertEqual(try restarted.audio(for: previous.sessionId), previous.audio)
    }
    func testFailedReplyWriteKeepsPendingCapture() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data([1])); _ = try DurableStore(root: root).accept(packet)
        let faulty = try DurableStore(root: root, fault: { checkpoint in
            if checkpoint == .beforeWrite { throw NSError(domain: NSPOSIXErrorDomain, code: 28) }
        })
        XCTAssertThrowsError(try faulty.update(packet.sessionId, reply: "Nicht gesichert.", state: .answered))
        let entry = try XCTUnwrap(DurableStore(root: root).entries().first)
        XCTAssertNil(entry.reply)
        XCTAssertEqual(entry.state, .saved)
    }
}
