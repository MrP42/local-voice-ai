import XCTest
@testable import VoiceCore

final class StorageIsolationTests: XCTestCase {
    func testCorruptMetadataIsVisibleAndDoesNotBlockHealthyHistory() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let damaged = Packet.capture(audio: Data([1, 2])), healthy = Packet.capture(audio: Data([3]))
        _ = try store.accept(damaged); _ = try store.accept(healthy)
        try Data("broken".utf8).write(to: root.appendingPathComponent(damaged.sessionId.uuidString + "/entry.json"))
        let inventory = try store.inventory()
        XCTAssertEqual(inventory.entries.map(\.id), [healthy.sessionId])
        XCTAssertEqual(inventory.issues.map(\.sessionId), [damaged.sessionId])
        XCTAssertEqual(try store.audio(for: damaged.sessionId), damaged.audio)
        try store.update(healthy.sessionId, reply: "Gespeichert.", state: .answered)
        XCTAssertThrowsError(try store.accept(damaged))
        _ = try store.accept(.capture(audio: Data([4])))
        XCTAssertEqual(try store.inventory().issues.count, 1)
    }
    func testCorruptIdentityCannotBeReusedUnderAnotherSession() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1]))
        _ = try store.accept(packet)
        try Data().write(to: root.appendingPathComponent(packet.sessionId.uuidString + "/entry.json"))
        var collision = packet; collision.sessionId = UUID()
        XCTAssertThrowsError(try store.accept(collision))
        XCTAssertEqual(try store.audio(for: packet.sessionId), packet.audio)
    }
    func testLegacyEntryMigratesAdditivelyWithoutChangingReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1]))
        let receipt = try store.accept(packet)
        let manifest = root.appendingPathComponent(packet.sessionId.uuidString + "/identity.json")
        try FileManager.default.removeItem(at: manifest)
        try store.migrateIdentities()
        XCTAssertEqual(try store.accept(packet), receipt)
        XCTAssertTrue(FileManager.default.fileExists(atPath: manifest.path))
    }
    func testCompleteOrphanStageRecoversButUnidentifiedAudioIsRetained() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1]))
        let receipt = try store.accept(packet)
        let stage = root.appendingPathComponent(".partial-" + UUID().uuidString)
        try FileManager.default.moveItem(at: root.appendingPathComponent(packet.sessionId.uuidString), to: stage)
        let unknown = root.appendingPathComponent(".partial-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: unknown, withIntermediateDirectories: true)
        try Data([8]).write(to: unknown.appendingPathComponent("audio.m4a"))
        try store.recoverStaging()
        XCTAssertEqual(try store.accept(packet), receipt)
        XCTAssertFalse(FileManager.default.fileExists(atPath: stage.path))
        XCTAssertEqual(try Data(contentsOf: unknown.appendingPathComponent("audio.m4a")), Data([8]))
        XCTAssertTrue(try store.inventory().issues.contains { $0.url.lastPathComponent == unknown.lastPathComponent })
    }
    func testTransferBudgetRejectsExtraCopiesWithoutAffectingOriginal() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root, temporaryLimit: 2048)
        let packet = Packet.capture(audio: Data([1])); _ = try store.accept(packet)
        XCTAssertThrowsError(try store.prepareTransfer(for: packet.sessionId, data: Data(repeating: 2, count: 2049)))
        XCTAssertEqual(try store.audio(for: packet.sessionId), packet.audio)
        _ = try store.prepareTransfer(for: packet.sessionId, data: Data([2, 3]))
        XCTAssertEqual(try store.inventory().temporaryBytes, 2)
    }
}
