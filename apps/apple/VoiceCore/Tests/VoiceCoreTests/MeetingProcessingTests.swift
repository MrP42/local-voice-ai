import XCTest
@testable import VoiceCore

final class MeetingProcessingTests: XCTestCase {
    func testCancelledSectionsLeaveArchiveAndCheckpointUnchanged() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("synthetic.wav")
        try Data([1, 2, 3]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        let doc = try await archive.importMedia(source, duration: 60)
        let chunk = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            try await archive.appendChunk(doc.id, offset: 0, nextOffset: 30, segments: [])
        }
        do { try await chunk.value; XCTFail("cancelled chunk committed") }
        catch { XCTAssertTrue(error is CancellationError) }
        let interrupted = try await archive.load(doc.id)
        XCTAssertEqual(interrupted.nextOffset, 0)
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 60, segments: [])
        let saved = MinutesCheckpoint(binding: "synthetic-before", parts: [:])
        try await archive.saveMinutesCheckpoint(doc.id, checkpoint: saved)
        let minutes = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            try await archive.saveMinutesCheckpoint(doc.id, checkpoint: .init(binding: "synthetic-after", parts: [:]))
        }
        do { try await minutes.value; XCTFail("cancelled checkpoint committed") }
        catch { XCTAssertTrue(error is CancellationError) }
        let resumed = try await archive.load(doc.id)
        XCTAssertEqual(resumed.minutesCheckpoint?.binding, saved.binding)
        XCTAssertEqual(resumed.nextOffset, 60)
    }

    func testGrantSurvivesBackgroundButDenialDoesNot() {
        var granted = MeetingProcessing()
        granted.foreground = true
        granted.submitted("test-1")
        granted.foreground = false
        XCTAssertTrue(granted.allowed)
        XCTAssertTrue(granted.started("test-1"))
        var denied = MeetingProcessing()
        denied.foreground = false
        XCTAssertFalse(denied.allowed)
        XCTAssertFalse(denied.started("test-1"))
    }

    func testExpirationRejectsLateHandlerAndDoesNotRevokeForeground() {
        var state = MeetingProcessing()
        state.submitted("old")
        XCTAssertTrue(state.expire("old"))
        XCTAssertFalse(state.allowed)
        XCTAssertFalse(state.started("old"))
        state.foreground = true
        state.submitted("new")
        XCTAssertFalse(state.expire("old"))
        XCTAssertTrue(state.expire("new"))
        XCTAssertTrue(state.allowed)
    }

    func testDeferredGrantAndCompletionHaveIndependentLifetimes() {
        var state = MeetingProcessing()
        state.deferred = true
        XCTAssertTrue(state.allowed)
        state.finish()
        XCTAssertFalse(state.allowed)
        XCTAssertNil(state.identifier)
        state.submitted("test-2")
        state.finish()
        XCTAssertFalse(state.started("test-2"))
    }

    func testProgressClampsInvalidInputAndReservesCompletionForQueueDrain() {
        XCTAssertEqual(MeetingProcessing.transcription(offset: 30, duration: 60), 400)
        XCTAssertEqual(MeetingProcessing.transcription(offset: 60, duration: 60), 800)
        XCTAssertEqual(MeetingProcessing.transcription(offset: .nan, duration: 60), 0)
        XCTAssertEqual(MeetingProcessing.transcription(offset: 30, duration: 0), 0)
        XCTAssertEqual(MeetingProcessing.minutes(section: 2, total: 4), 850)
        XCTAssertEqual(MeetingProcessing.minutes(section: 9, total: 4), 999)
    }

    func testGPURequiresBothSupportAndEntitlement() {
        for supported in [false, true] {
            for entitled in [false, true] {
                XCTAssertEqual(MeetingProcessing.requestGPU(supported: supported, entitled: entitled), supported && entitled)
            }
        }
    }

    func testSignedEntitlementFailsClosedForMissingFalseOrMalformedSignature() throws {
        func executable(entitled: Bool) throws -> Data {
            let plist = try PropertyListSerialization.data(fromPropertyList:
                ["com.apple.developer.background-tasks.continued-processing.gpu": entitled], format: .xml, options: 0)
            func word(_ n: Int, little: Bool = false) -> [UInt8] {
                let bytes = (0..<4).map { UInt8(truncatingIfNeeded: n >> ($0 * 8)) }
                return little ? bytes : Array(bytes.reversed())
            }
            var image = word(0xfeedfacf, little: true) + Array(repeating: UInt8(0), count: 12)
            image += word(1, little: true) + word(16, little: true) + Array(repeating: UInt8(0), count: 8)
            image += word(0x1d, little: true) + word(16, little: true) + word(48, little: true) + word(28 + plist.count, little: true)
            image += word(0xfade0cc0) + word(28 + plist.count) + word(1) + word(5) + word(20)
            image += word(0xfade7171) + word(8 + plist.count) + Array(plist)
            return Data(image)
        }
        let granted = try executable(entitled: true)
        XCTAssertTrue(MeetingProcessing.hasGPUEntitlement(executable: granted))
        XCTAssertFalse(MeetingProcessing.hasGPUEntitlement(executable: try executable(entitled: false)))
        XCTAssertFalse(MeetingProcessing.hasGPUEntitlement(executable: Data()))
        for count in [4, 31, 47, granted.count - 1] {
            XCTAssertFalse(MeetingProcessing.hasGPUEntitlement(executable: Data(granted.prefix(count))))
        }
    }
}
