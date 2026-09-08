import XCTest
@testable import VoiceCore

final class StoreTests: XCTestCase {
    func testProcessingProvenanceSurvivesRestartAndWatchDelivery() throws {
        let root = temporary(), watchRoot = temporary()
        defer { try? FileManager.default.removeItem(at: root); try? FileManager.default.removeItem(at: watchRoot) }
        let store = try DurableStore(root: root), watch = try DurableStore(root: watchRoot)
        let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet); _ = try watch.accept(packet)
        let event = ProcessingEvent(operation: "Antwort", model: "Testmodell", completedAt: Date(), durationMS: 120, isAI: true)
        try store.update(packet.sessionId, reply: "**Hallo**", state: .answered, event: event)
        let restarted = try DurableStore(root: root)
        XCTAssertEqual(try restarted.entries().first?.processingEvents?.first, event)
        _ = try watch.acceptReply(restarted.replyEnvelope(for: packet.sessionId))
        XCTAssertEqual(try DurableStore(root: watchRoot).entries().first?.processingEvents?.first, event)
    }
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
    func testLateReceiptCannotRegressAnsweredState() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet)
        try store.update(packet.sessionId, reply: "Antwort", state: .answered)
        try store.update(packet.sessionId, state: .accepted)
        XCTAssertEqual(try store.entries().first?.state, .answered)
    }
    func testCorruptedAudioIsNotAcknowledgedAgain() throws {
        let root = temporary(); defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet)
        try Data([2]).write(to: store.audioURL(for: packet.sessionId))
        XCTAssertThrowsError(try store.accept(packet))
    }
    func testWireHasVersionedPayload() throws {
        let data = try JSONEncoder().encode(Packet.capture(audio: Data([1])))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertNotNil(object["payload"])
        XCTAssertNil(object["audio"])
        XCTAssertEqual(object["schemaVersion"] as? Int, 1)
    }
    func testTransportEnvelopeRoundTripsCaptureAndReceipt() throws {
        let packet = Packet.capture(audio: Data([1, 2, 3]))
        let encoded = try JSONEncoder().encode(VoiceEnvelope(capture: packet))
        let decoded = try JSONDecoder().decode(VoiceEnvelope.self, from: encoded)
        XCTAssertEqual(decoded.sessionId, packet.sessionId)
        XCTAssertEqual(decoded.messageId, packet.messageId)
        XCTAssertEqual(decoded.kind, "capture")
        XCTAssertEqual(decoded.capture?.audio, packet.audio)
        let receipt = Receipt(sessionId: packet.sessionId, messageId: packet.messageId, receiptId: UUID())
        let response = try JSONDecoder().decode(VoiceEnvelope.self, from: JSONEncoder().encode(VoiceEnvelope(receipt: receipt)))
        XCTAssertEqual(response.receipt, receipt)
        XCTAssertEqual(response.messageId, receipt.receiptId)
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
