import XCTest
@testable import VoiceCore

final class InventoryCacheTests: XCTestCase {
    func testReloadExposesCorruptAudioAndRetainsHealthyHistory() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root)
        let bad = Packet.capture(audio: Data([1,2])), good = Packet.capture(audio: Data([3]))
        _ = try store.accept(bad); _ = try store.accept(good)
        XCTAssertEqual(try store.entries().count, 2)
        try Data([9,9]).write(to: store.audioURL(for: bad.sessionId))
        store.invalidateInventory()
        let current = try store.inventory(includeUsage: false)
        XCTAssertEqual(current.entries.map(\.id), [good.sessionId])
        XCTAssertEqual(current.issues.map(\.sessionId), [bad.sessionId])
        XCTAssertEqual(try store.audio(for: bad.sessionId), Data([9,9]))
    }
    func testCommittedUpdatesAreImmediatelyVisibleInInventory() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try DurableStore(root: root); let packet = Packet.capture(audio: Data([1])); _ = try store.accept(packet)
        _ = try store.entries()
        try store.update(packet.sessionId, transcript: "Text", reply: "Antwort", state: .answered)
        XCTAssertEqual(try store.entries().first?.reply, "Antwort")
        XCTAssertEqual(try store.entries().first?.state, .answered)
    }
    func testUncertainWriteDoesNotLeaveAnOldCachedState() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let packet = Packet.capture(audio: Data([1])); _ = try DurableStore(root: root).accept(packet)
        let faulty = try DurableStore(root: root, fault: { if $0 == .fileSynced { throw VoiceError.persistence } })
        _ = try faulty.entries()
        XCTAssertThrowsError(try faulty.update(packet.sessionId, reply: "Antwort", state: .answered))
        let disk = try DurableStore(root: root).entries().first
        XCTAssertEqual(try faulty.entries().first?.reply, disk?.reply)
        XCTAssertEqual(try faulty.entries().first?.state, disk?.state)
    }
}
