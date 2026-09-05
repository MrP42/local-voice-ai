import XCTest
@testable import VoiceCore

final class RecordingRecoveryTests: XCTestCase {
    func testDraftRecoversWithStableIdentityAcrossRestartAndReplay() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let id = UUID()
        let draft = root.appendingPathComponent(".recording-\(id.uuidString).m4a")
        let audio = Data([1, 2, 3])
        try audio.write(to: draft)
        let receipt = try store.recoverRecording(at: draft)
        XCTAssertEqual(receipt.sessionId, id)
        XCTAssertFalse(FileManager.default.fileExists(atPath: draft.path))
        // A crash before draft deletion leaves the same source; retry must not create another entry.
        try audio.write(to: draft)
        let restarted = try DurableStore(root: root)
        XCTAssertEqual(try restarted.recoverRecording(at: draft), receipt)
        XCTAssertEqual(try restarted.entries().count, 1)
        XCTAssertEqual(try restarted.audio(for: id), audio)
    }
    func testFullStoreKeepsDraftForLaterRecovery() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let full = try DurableStore(root: root, limit: 1)
        let draft = root.appendingPathComponent(".recording-\(UUID().uuidString).m4a")
        try Data([1, 2]).write(to: draft)
        XCTAssertThrowsError(try full.recoverRecording(at: draft))
        XCTAssertTrue(FileManager.default.fileExists(atPath: draft.path))
        let available = try DurableStore(root: root, limit: 10)
        _ = try available.recoverRecording(at: draft)
        XCTAssertEqual(try available.entries().count, 1)
    }
    func testEmptyDraftIsRetainedWithoutReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let draft = root.appendingPathComponent(".recording-\(UUID().uuidString).m4a")
        try Data().write(to: draft)
        XCTAssertThrowsError(try store.recoverRecording(at: draft))
        XCTAssertTrue(FileManager.default.fileExists(atPath: draft.path))
        XCTAssertEqual(try store.entries().count, 0)
    }
}
