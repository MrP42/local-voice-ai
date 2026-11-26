import XCTest
@testable import VoiceCore

/// Hänger 26.11.: Gleichheit über Fingerabdruck, ohne die Bedeutung von == zu ändern.
final class ReadingPageEqualityTests: XCTestCase {
    func testFingerprintKeepsEqualitySemantics() throws {
        let json = Data(#"{"title":"A","state":{"text":"x"}}"#.utf8)
        let a = try ReadingPage(id: "page_abc", json: json), b = try ReadingPage(id: "page_abc", json: json)
        XCTAssertEqual(a, b)
        var changed = a; changed.text = "y"
        XCTAssertNotEqual(a, changed)
        var same = a; same.text = "x"
        XCTAssertEqual(a, same, "gleicher Inhalt nach einer Änderung bleibt gleich")
        XCTAssertNotEqual(a, try ReadingPage(id: "page_abd", json: json))
        XCTAssertNotEqual(a, try ReadingPage(id: "page_abc", json: Data(#"{"title":"B","state":{"text":"x"}}"#.utf8)))
    }
}
