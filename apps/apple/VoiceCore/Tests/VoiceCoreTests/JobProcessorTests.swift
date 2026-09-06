import XCTest
@testable import VoiceCore

final class JobProcessorTests: XCTestCase {
    @MainActor
    func testActualProviderIdentityAndRuleAnswerArePersisted() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); _ = try store.accept(.capture(audio: Data([1])))
        let worker = JobProcessor(store: store, annotatedTranscribe: { _ in ProcessingOutput(text: "Text", model: "Whisper Test") }, annotatedReply: { _ in ProcessingOutput(text: "Feste Antwort", model: "Funktionsregel", isAI: false) })
        worker.setActive(true)
        try await waitUntil { !worker.isProcessing }
        let events = try XCTUnwrap(DurableStore(root: root).entries().first?.processingEvents)
        XCTAssertEqual(events.map(\.model), ["Whisper Test", "Funktionsregel"])
        XCTAssertEqual(events.map(\.isAI), [true, false])
        XCTAssertEqual(events.map(\.operation), ["Transkription", "Antwort"])
        XCTAssertTrue(events.allSatisfy { $0.durationMS >= 0 && $0.completedAt <= Date() })
    }
    @MainActor
    func waitUntil(_ predicate: () throws -> Bool) async throws {
        for _ in 0..<200 {
            if try predicate() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Worker did not reach the expected checkpoint")
    }
    @MainActor
    func testCancelDuringGenerationPreservesSTTAndResumeDoesNotTranscribeAgain() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1])); _ = try store.accept(packet)
        let worker = JobProcessor(store: store, transcribe: { _ in "Mein gespeicherter Text." }, reply: { _ in
            try await Task.sleep(for: .seconds(30)); return "Zu spät."
        })
        worker.setActive(true)
        try await waitUntil { try store.entries().first?.job?.phase == .generating }
        worker.setActive(false)
        try await waitUntil { !worker.isProcessing }
        XCTAssertEqual(try store.entries().first?.job?.phase, .paused)
        XCTAssertEqual(try store.entries().first?.transcript, "Mein gespeicherter Text.")
        XCTAssertNil(try store.entries().first?.reply)
        let resumed = JobProcessor(store: store, transcribe: { _ in throw VoiceError.invalid }, reply: { text in text })
        resumed.setActive(true)
        try await waitUntil { try store.entries().first?.reply != nil }
        XCTAssertEqual(try store.entries().first?.reply, "Mein gespeicherter Text.")
        resumed.setActive(false)
        try await waitUntil { !resumed.isProcessing }
    }
    @MainActor
    func testProviderDeadlineUsesBoundedRetryInsteadOfCancellation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); _ = try store.accept(.capture(audio: Data([1])))
        let worker = JobProcessor(store: store, transcribe: { _ in
            let deadline = ProcessingDeadline(); deadline.expire(); try deadline.check(); return "late"
        }, reply: { _ in "Must not run" })
        worker.setActive(true)
        try await waitUntil { try store.entries().first?.job?.phase == .waiting }
        let job = try XCTUnwrap(store.entries().first?.job)
        XCTAssertEqual(job.attempts, 1); XCTAssertEqual(job.failure, .unavailable)
        XCTAssertNotNil(job.nextAttemptAt)
        worker.setActive(false); try await waitUntil { !worker.isProcessing }
        let deadline = ProcessingDeadline(); deadline.expire()
        XCTAssertThrowsError(try deadline.check(cancelled: true)) { XCTAssertTrue($0 is CancellationError) }
    }
    @MainActor
    func testNoSpeechIsNotSentToGeneratorOrRepeatedAutomatically() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1])); _ = try store.accept(packet)
        let worker = JobProcessor(store: store, transcribe: { _ in "  " }, reply: { _ in "Should not be called" })
        worker.setActive(true)
        try await waitUntil { !worker.isProcessing }
        let entry = try XCTUnwrap(store.entries().first)
        XCTAssertEqual(entry.job?.failure, .noSpeech)
        XCTAssertEqual(entry.job?.attempts, 1)
        XCTAssertNil(entry.reply)
        worker.start()
        try await waitUntil { !worker.isProcessing }
        XCTAssertEqual(try store.entries().first?.job?.attempts, 1)
    }
}
