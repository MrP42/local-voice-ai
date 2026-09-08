import XCTest
@testable import VoiceCore

final class ChunkTests: XCTestCase {
    func testPartialTransferSurvivesRestartAndNeverAcknowledgesEarly() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data(repeating: 7, count: 142099))
        let pieces = try CaptureChunk.split(packet)
        let first = try ChunkInbox(root: root)
        XCTAssertNil(try first.receive(pieces[0]))
        XCTAssertNil(try first.receive(pieces[0]))
        let restarted = try ChunkInbox(root: root)
        var result: Packet?
        for piece in pieces.dropFirst().reversed() { result = try restarted.receive(piece) ?? result }
        XCTAssertEqual(result?.audio, packet.audio)
        XCTAssertEqual(result?.messageId, packet.messageId)
        XCTAssertEqual(result?.sessionId, packet.sessionId)
    }
    func testConflictingPartAndOversizeAreRejected() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let inbox = try ChunkInbox(root: root)
        let packet = Packet.capture(audio: Data(repeating: 1, count: 50000))
        let pieces = try CaptureChunk.split(packet)
        XCTAssertNil(try inbox.receive(pieces[0]))
        var wrong = pieces[0]; wrong.data = Data(repeating: 2, count: wrong.data.count)
        XCTAssertThrowsError(try inbox.receive(wrong))
        XCTAssertThrowsError(try CaptureChunk.split(.capture(audio: Data(repeating: 0, count: 1024 * 1024 + 1))))
    }
    func testChunkEnvelopeFitsInteractiveBudget() throws {
        let chunks = try CaptureChunk.split(.capture(audio: Data(repeating: 3, count: 1024 * 1024)))
        for chunk in chunks { XCTAssertLessThan(try JSONEncoder().encode(VoiceEnvelope(chunk: chunk)).count, 48000) }
    }
}
