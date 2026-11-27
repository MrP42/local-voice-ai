import XCTest
@testable import VoiceCore

/// Hänger 26.11.: Öffnen von Vorlesen und Abgleich parsten bei jedem Aufruf alle Seiten neu.
/// Realistischer Bestand: 400 Seiten mit je rund 6 KB Text (synthetisch).
@MainActor
final class ReadingPageStorePerformanceTests: XCTestCase {
    private static let count = 400

    private func filledStore() throws -> ReadingPageStore {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let store = try ReadingPageStore(root: root)
        let body = String(repeating: "Synthetischer Probetext für die Vorleseseite. ", count: 130)
        for i in 0..<Self.count {
            var page = try ReadingPage(id: "page_perf\(i)", title: "Seite \(i)")
            page.text = body + String(i)
            try store.save(page)
        }
        return store
    }

    func testRepeatedPagesStayWithinMainThreadBudget() throws {
        let store = try filledStore()
        XCTAssertEqual(try store.pages().count, Self.count)
        let start = Date()
        let again = try store.pages()
        let elapsed = Date().timeIntervalSince(start)
        XCTAssertEqual(again.count, Self.count)
        // Budget Main-Thread < 100 ms beim Öffnen; ohne Zwischenspeicher dauert das ein Vielfaches.
        XCTAssertLessThan(elapsed, 0.1, "pages() blockiert \(Int(elapsed * 1000)) ms")
    }

    func testRepeatedPagesKeepIdentityForCheapSwiftUIDiffs() throws {
        let store = try filledStore()
        let first = try store.pages()
        let second = try store.pages()
        XCTAssertEqual(first, second)
        let start = Date()
        for _ in 0..<20 { XCTAssertEqual(first, second) }
        XCTAssertLessThan(Date().timeIntervalSince(start), 0.05)
    }

    func testCacheSeesChangesFromAnotherStoreInstanceAndDeletions() throws {
        let store = try filledStore()
        _ = try store.pages()
        let other = try ReadingPageStore(root: store.root)
        var page = try XCTUnwrap(other.page("page_perf7"))
        page.text = "Geändert von der Share-Erweiterung"
        try other.save(page)
        XCTAssertEqual(try store.page("page_perf7")?.text, "Geändert von der Share-Erweiterung")
        try FileManager.default.removeItem(at: store.root.appendingPathComponent("page_perf8.json"))
        XCTAssertNil(try store.page("page_perf8"))
        XCTAssertEqual(try store.pages().count, Self.count - 1)
        try store.delete("page_perf9")
        XCTAssertEqual(try store.pages().count, Self.count - 2)
        XCTAssertNotNil(try store.archivedPage("page_perf9"))
    }
}
