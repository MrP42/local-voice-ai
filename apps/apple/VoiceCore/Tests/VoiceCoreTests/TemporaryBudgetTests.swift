import XCTest
@testable import VoiceCore

final class TemporaryBudgetTests: XCTestCase {
    func testChunkMetadataCannotBypassBudgetAndExistingPartSurvivesFullBudget() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data(repeating: 1, count: 40000)); let parts = try CaptureChunk.split(packet)
        let inbox = try ChunkInbox(root: root, limit: 34000)
        XCTAssertNil(try inbox.receive(parts[0]))
        XCTAssertThrowsError(try inbox.receive(parts[1]))
        XCTAssertNil(try inbox.receive(parts[0]))
        XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent(packet.sessionId.uuidString + "/0.part")), parts[0].data)
    }
    func testOtherTemporaryCopiesCountBeforeCreatingAManifest() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data(repeating: 1, count: 40000)); let part = try CaptureChunk.split(packet)[0]
        let inbox = try ChunkInbox(root: root, limit: 34000, additionalBytes: { 2000 })
        XCTAssertThrowsError(try inbox.receive(part))
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: root.path).isEmpty)
    }
}
