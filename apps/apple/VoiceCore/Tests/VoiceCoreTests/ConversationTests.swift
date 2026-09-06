import XCTest
@testable import VoiceCore

final class ConversationTests: XCTestCase {
    func testContextSurvivesRestartAndNeverIncludesAnotherConversation() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root), group = UUID()
        var first = Packet.capture(audio: Data([1])); first.payload.conversationId = group
        first.createdAt = Date(timeIntervalSince1970: 1)
        _ = try store.accept(first)
        try store.update(first.sessionId, transcript: "Mein Hund heißt Milo.", reply: "Ich merke mir Milo für dieses Gespräch.", state: .answered)
        var other = Packet.capture(audio: Data([2])); other.payload.conversationId = UUID()
        other.createdAt = Date(timeIntervalSince1970: 2)
        _ = try store.accept(other)
        try store.update(other.sessionId, transcript: "Nicht zuordnen", reply: "Fremdes Gespräch", state: .answered)
        var next = Packet.capture(audio: Data([3])); next.payload.conversationId = group
        _ = try store.accept(next)
        let entries = try DurableStore(root: root).entries()
        let current = try XCTUnwrap(entries.first { $0.id == next.sessionId })
        let context = ConversationContext.previousTurns(for: current, in: entries)
        XCTAssertEqual(context.map(\.user), ["Mein Hund heißt Milo."])
        XCTAssertEqual(try store.packet(for: current).payload.conversationId, group)
    }
    @MainActor
    func testJobProcessorUsesStoredConversationForItsReply() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root), group = UUID()
        var first = Packet.capture(audio: Data([1])); first.payload.conversationId = group; first.createdAt = Date(timeIntervalSince1970: 1)
        _ = try store.accept(first)
        try store.update(first.sessionId, transcript: "Mein Hund heißt Milo.", reply: "Milo.", state: .answered)
        var next = Packet.capture(audio: Data([2])); next.payload.conversationId = group
        _ = try store.accept(next)
        try store.update(next.sessionId, transcript: "Wie heißt er?", state: .saved)
        let worker = JobProcessor(store: store, annotatedTranscribe: { _ in throw VoiceError.invalid }, conversationalReply: { _, history in
            ProcessingOutput(text: history.first?.user ?? "missing", model: "Context test")
        })
        worker.setActive(true)
        for _ in 0..<100 {
            if !worker.isProcessing { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(try store.entries().first(where: { $0.id == next.sessionId })?.reply, "Mein Hund heißt Milo.")
        worker.setActive(false)
    }
    func testDraftRecoveryPreservesConversationAfterRestart() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root), group = UUID()
        let file = root.appendingPathComponent(".recording-" + UUID().uuidString + "_" + group.uuidString + ".m4a")
        try Data([1, 2, 3]).write(to: file)
        let receipt = try store.recoverRecording(at: file)
        XCTAssertEqual(try DurableStore(root: root).entries().first(where: { $0.id == receipt.sessionId })?.conversationId, group)
    }
    func testConversationIdentitySurvivesChunkedTransport() throws {
        var packet = Packet.capture(audio: Data(repeating: 1, count: 70000)); packet.payload.conversationId = UUID()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let inbox = try ChunkInbox(root: root)
        var result: Packet?
        for part in try CaptureChunk.split(packet) { result = try inbox.receive(part) }
        XCTAssertEqual(result?.payload.conversationId, packet.payload.conversationId)
    }
    func testSilenceDetectorRequiresSpeechBeforeClosingATurn() {
        var detector = VoiceTurnDetector()
        XCTAssertNil(detector.observe(powerDB: -80, elapsed: 1))
        XCTAssertNil(detector.observe(powerDB: -20, elapsed: 2))
        XCTAssertNil(detector.observe(powerDB: -20, elapsed: 2.5))
        XCTAssertNil(detector.observe(powerDB: -80, elapsed: 3))
        XCTAssertEqual(detector.observe(powerDB: -80, elapsed: 4), .finishedSpeaking)
        var silence = VoiceTurnDetector()
        XCTAssertEqual(silence.observe(powerDB: -80, elapsed: 8), .noSpeech)
    }
}
