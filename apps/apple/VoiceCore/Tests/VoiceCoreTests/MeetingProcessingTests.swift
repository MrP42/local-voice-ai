import XCTest
@testable import VoiceCore

final class MeetingProcessingTests: XCTestCase {
    func testDisablingRevokesBothGrantsWithoutDuplicatingForegroundWorker() {
        for deferred in [false, true] {
            var state = MeetingProcessing()
            state.foreground = true
            if deferred { state.deferred = true }
            else { state.submitted("synthetic-task") }
            XCTAssertTrue(state.beginWorker())
            XCTAssertTrue(state.setBackgroundEnabled(false))
            XCTAssertNil(state.identifier)
            XCTAssertFalse(state.deferred)
            XCTAssertFalse(state.started("synthetic-task"))
            XCTAssertFalse(state.setBackgroundEnabled(false))
            XCTAssertTrue(state.allowStart(userInitiated: false))
            XCTAssertFalse(state.beginWorker()) // Der abgebrochene Worker endet noch.
            state.endWorker()
            XCTAssertTrue(state.beginWorker()) // Vordergrund bleibt erlaubt.
            state.endWorker()
            state.foreground = false
            XCTAssertFalse(state.allowStart(userInitiated: true))
            XCTAssertFalse(state.beginWorker())
        }
    }

    func testDisabledSettingRejectsLateGrantsAndReenablingDoesNotRestoreThem() {
        var state = MeetingProcessing()
        state.setBackgroundEnabled(false)
        state.submitted("synthetic-late")
        state.deferred = true
        state.resume()
        XCTAssertFalse(state.allowed)
        XCTAssertFalse(state.started("synthetic-late"))
        XCTAssertFalse(state.beginWorker())
        state.setBackgroundEnabled(false)
        state.setBackgroundEnabled(true)
        XCTAssertFalse(state.allowed)
        state.deferred = true
        XCTAssertTrue(state.beginWorker())
    }

    func testReadyNotificationIsClaimedOncePerDocument() {
        var notifications = MeetingReadyNotifications()
        let first = UUID(), second = UUID()
        XCTAssertTrue(notifications.claim(first))
        XCTAssertFalse(notifications.claim(first))
        XCTAssertTrue(notifications.claim(second))
        XCTAssertFalse(notifications.claim(second))
    }

    func testDisablingRunningProcessingKeepsLastCheckpointForForegroundResume() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("synthetic.wav")
        try Data([1, 2, 3]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        let doc = try await archive.importMedia(source, duration: 60)
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 60, segments: [])
        let saved = MinutesCheckpoint(binding: "synthetic-saved", parts: [:])
        try await archive.saveMinutesCheckpoint(doc.id, checkpoint: saved)
        var state = MeetingProcessing()
        state.foreground = true
        state.submitted("synthetic-task")
        XCTAssertTrue(state.beginWorker())
        let worker = Task {
            try await Task.sleep(for: .seconds(60))
            try await archive.saveMinutesCheckpoint(doc.id, checkpoint: .init(binding: "synthetic-unsaved", parts: [:]))
        }
        XCTAssertTrue(state.setBackgroundEnabled(false))
        worker.cancel() // Wie onExpire in der App; kein weiterer Abschnitt wird committed.
        do { try await worker.value; XCTFail("cancelled checkpoint committed") }
        catch { XCTAssertTrue(error is CancellationError) }
        XCTAssertFalse(state.beginWorker())
        state.endWorker()
        let resumed = try await archive.load(doc.id)
        XCTAssertEqual(resumed.nextOffset, 60)
        XCTAssertEqual(resumed.minutesCheckpoint?.binding, saved.binding)
        XCTAssertTrue(state.beginWorker())
        state.endWorker()
        state.foreground = false
        XCTAssertFalse(state.beginWorker())
    }

    func testUserStartResumesExpiredProcessingWithoutDuplicatingWorker() {
        for deferred in [false, true] {
            var state = MeetingProcessing()
            state.foreground = true
            if deferred { state.deferred = true }
            else { state.submitted("synthetic-task") }
            XCTAssertTrue(state.beginWorker())
            if deferred { XCTAssertTrue(state.expireDeferred()) }
            else { XCTAssertTrue(state.expire("synthetic-task")) }
            XCTAssertFalse(state.allowStart(userInitiated: false))
            XCTAssertTrue(state.allowStart(userInitiated: true))
            XCTAssertFalse(state.beginWorker()) // Abgebrochener Worker endet noch.
            state.endWorker()
            XCTAssertTrue(state.allowStart(userInitiated: false))
            XCTAssertTrue(state.beginWorker())
            XCTAssertFalse(state.beginWorker())
        }
    }

    func testAutomaticStartStaysSuspendedAfterExpiredWorkerEnds() {
        var state = MeetingProcessing()
        state.foreground = true
        state.submitted("synthetic-task")
        XCTAssertTrue(state.beginWorker())
        XCTAssertTrue(state.expire("synthetic-task"))
        state.endWorker()
        XCTAssertFalse(state.allowStart(userInitiated: false))
        XCTAssertFalse(state.beginWorker())
    }

    func testUserStartDoesNotGrantBackgroundExecution() {
        var state = MeetingProcessing()
        state.deferred = true
        XCTAssertTrue(state.expireDeferred())
        XCTAssertFalse(state.allowStart(userInitiated: true))
        XCTAssertFalse(state.beginWorker())
        XCTAssertNil(state.identifier)
        XCTAssertFalse(state.deferred)
    }

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
        let cancelledCheckpoint = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            try await archive.saveMinutesCheckpoint(doc.id, checkpoint: .init(binding: "synthetic-cancelled", parts: [:]))
        }
        do { try await cancelledCheckpoint.value; XCTFail("cancelled checkpoint committed") }
        catch { XCTAssertTrue(error is CancellationError) }
        var state = MeetingProcessing()
        state.foreground = true
        state.submitted("synthetic-task")
        XCTAssertTrue(state.beginWorker())
        let minutes = Task {
            // Die Pause hält den Worker bis zur Expiration vor dem nächsten Commit.
            try await Task.sleep(for: .seconds(60))
            try await archive.saveMinutesCheckpoint(doc.id, checkpoint: .init(binding: "synthetic-after", parts: [:]))
        }
        XCTAssertTrue(state.expire("synthetic-task"))
        minutes.cancel()
        do { try await minutes.value; XCTFail("cancelled checkpoint committed") }
        catch { XCTAssertTrue(error is CancellationError) }
        state.endWorker()
        XCTAssertFalse(state.beginWorker())
        let resumed = try await archive.load(doc.id)
        XCTAssertEqual(resumed.minutesCheckpoint?.binding, saved.binding)
        XCTAssertEqual(resumed.nextOffset, 60)
        state.resume() // Nächstes Vordergrundereignis.
        XCTAssertTrue(state.beginWorker())
        XCTAssertFalse(state.beginWorker())
        try await archive.saveMinutesCheckpoint(doc.id, checkpoint: .init(binding: "synthetic-resumed", parts: [:]))
        state.endWorker()
        state.submitted("synthetic-resumed-task")
        XCTAssertTrue(state.finish())
        XCTAssertFalse(state.finish())
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

    func testExpirationStopsWorkerUntilNextEvent() {
        var state = MeetingProcessing()
        state.foreground = true
        state.submitted("old")
        XCTAssertTrue(state.beginWorker())
        XCTAssertFalse(state.beginWorker())
        XCTAssertTrue(state.expire("old"))
        XCTAssertFalse(state.allowed)
        XCTAssertFalse(state.started("old"))
        XCTAssertFalse(state.expire("old")) // Kein zweiter Abschluss desselben Tasks.
        XCTAssertFalse(state.finish()) // Expiration hat den Abschluss bereits übernommen.
        state.endWorker()
        XCTAssertFalse(state.beginWorker()) // Auch bei weiter aktivem Vordergrund.
        state.resume()
        XCTAssertTrue(state.beginWorker())
        XCTAssertFalse(state.beginWorker())
        state.endWorker()
        state.submitted("new")
        XCTAssertFalse(state.expire("old"))
        XCTAssertTrue(state.allowed)
    }

    func testResumeBeforeCancelledWorkerEndsNeverStartsDuplicate() {
        var state = MeetingProcessing()
        state.deferred = true
        XCTAssertTrue(state.beginWorker())
        XCTAssertTrue(state.expireDeferred())
        XCTAssertFalse(state.expireDeferred())
        XCTAssertFalse(state.allowed)
        state.deferred = true
        state.resume()
        XCTAssertFalse(state.beginWorker())
        state.endWorker()
        XCTAssertTrue(state.beginWorker())
        state.endWorker()
        XCTAssertTrue(state.finish())
        XCTAssertFalse(state.finish())
        XCTAssertFalse(state.allowed)
    }

    func testQueueProgressHasFixedTotalAndNeverMovesBackwards() {
        let first = UUID(), second = UUID(), later = UUID()
        var queue = MeetingQueueProgress(documents: [first, second])
        XCTAssertEqual(queue.total, 2000)
        queue.update(first, units: 999)
        XCTAssertEqual(queue.completed, 999)
        queue.complete(first)
        queue.update(second, units: 0)
        XCTAssertEqual(queue.completed, 1000)
        queue.update(second, units: 850)
        queue.update(second, units: 800) // Modell-Fallback oder wiederholter Callback.
        XCTAssertEqual(queue.completed, 1850)
        queue.update(later, units: 999)
        XCTAssertFalse(queue.contains(later))
        XCTAssertEqual(queue.next(in: [later, second]), second)
        XCTAssertEqual(queue.next(in: [later]), later)
        XCTAssertEqual(queue.total, 2000)
        queue.update(second, units: Int64.max)
        XCTAssertEqual(queue.completed, 1999)
        queue.complete(second)
        queue.complete(second)
        XCTAssertEqual(queue.completed, 2000)
        queue.complete(second, success: false)
        XCTAssertFalse(queue.successful)
    }

    func testCancelledQueueItemRetainsItsProgressForResume() {
        let document = UUID()
        var queue = MeetingQueueProgress(documents: [document])
        queue.update(document, units: 850)
        // Ein abgebrochener Worker meldet keinen Abschluss; der Checkpoint kommt erneut.
        queue.update(document, units: 800)
        XCTAssertEqual(queue.completed, 850)
        queue.complete(document)
        XCTAssertEqual(queue.completed, queue.total)
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
