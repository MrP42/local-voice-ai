import XCTest
@testable import VoiceCore

final class StoreTests: XCTestCase {
    func temporary() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString) }
    func testConfirmedCaptureSurvivesRestartAndDuplicateHasSameReceipt() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data([1, 2, 3]))
        let first = try DurableStore(root: root).accept(packet)
        let restarted = try DurableStore(root: root)
        XCTAssertEqual(try restarted.accept(packet), first)
        XCTAssertEqual(try restarted.entries().count, 1)
        XCTAssertEqual(try restarted.audio(for: packet.sessionId), packet.audio)
    }
    func testConflictingDuplicateDoesNotReplaceAudio() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet)
        var conflict = packet; conflict.audio = Data([2])
        XCTAssertThrowsError(try store.accept(conflict))
        XCTAssertEqual(try store.audio(for: packet.sessionId), Data([1]))
    }
    func testWrongVersionAndFullStorageAreNotAcknowledged() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root, limit: 3)
        var invalid = Packet.capture(audio: Data([1])); invalid.schemaVersion = 2
        XCTAssertThrowsError(try store.accept(invalid))
        XCTAssertThrowsError(try store.accept(.capture(audio: Data([1, 2, 3, 4]))))
        XCTAssertEqual(try store.entries().count, 0)
    }
    func testReplyPersistsAndLateReplyCannotCreateUnknownEntry() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet)
        try store.update(packet.sessionId, transcript: "Original", reply: "Antwort", state: .answered)
        XCTAssertEqual(try DurableStore(root: root).entries().first?.transcript, "Original")
        XCTAssertThrowsError(try store.update(UUID(), reply: "Spät", state: .answered))
    }
    func testInterruptedStagingDoesNotHideConfirmedEntries() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        _ = try store.accept(.capture(audio: Data([1])))
        try FileManager.default.createDirectory(at: root.appendingPathComponent(".partial-test"), withIntermediateDirectories: true)
        XCTAssertEqual(try DurableStore(root: root).entries().count, 1)
    }
    func testHundredTurnsReplayedInReverseAfterRestart() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let packets = (0..<100).map { Packet.capture(audio: Data([UInt8($0)])) }
        let initial = try DurableStore(root: root)
        let receipts = try packets.map { try initial.accept($0) }
        let restarted = try DurableStore(root: root)
        for i in packets.indices.reversed() { XCTAssertEqual(try restarted.accept(packets[i]), receipts[i]) }
        XCTAssertEqual(try restarted.entries().count, 100)
    }
}
