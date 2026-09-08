import XCTest
@testable import VoiceCore

final class DeliveryMaintenanceTests: XCTestCase {
    func testTransferCleanupPreservesActiveCopiesAndOriginalCapture() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let packet = Packet.capture(audio: Data([1, 2]))
        _ = try store.accept(packet)
        let completed = root.appendingPathComponent(".transfer-\(packet.sessionId.uuidString).json")
        let active = root.appendingPathComponent(".transfer-\(UUID().uuidString).json")
        let unrelated = root.appendingPathComponent(".transfer-not-a-session.json")
        for url in [completed, active, unrelated] { try Data([9]).write(to: url) }
        try store.cleanupTransfers(keeping: [active])
        XCTAssertFalse(FileManager.default.fileExists(atPath: completed.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: active.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: unrelated.path))
        XCTAssertEqual(try store.audio(for: packet.sessionId), packet.audio)
        XCTAssertEqual(try store.entries().count, 1)
    }
    func testSavedCapturesAreDeliveredOldestFirstBeforeAcceptedCaptures() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        var packets = (0..<4).map { _ in Packet.capture(audio: Data([1])) }
        for i in packets.indices {
            packets[i].createdAt = Date(timeIntervalSince1970: Double(i))
            _ = try store.accept(packets[i])
        }
        try store.update(packets[0].sessionId, state: .accepted)
        try store.update(packets[3].sessionId, reply: "done", state: .answered)
        XCTAssertEqual(Entry.pendingDelivery(in: try store.entries()).map(\.id),
                       [packets[1].sessionId, packets[2].sessionId, packets[0].sessionId])
    }
}
