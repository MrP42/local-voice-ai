import XCTest
@testable import VoiceCore

/// Zeichenketten des Ablage-Parsers (Läufe am Stück, Kaltstart 27.11.): gleiche Ergebnisse und Fehler wie zuvor.
final class AblageJSONStringTests: XCTestCase {
    private func parse(_ text: String) throws -> AblageJSON { try AblageJSON.parse(Array(text.utf8)) }
    private func string(_ json: String) throws -> String? {
        if case .string(let s) = try parse(json) { return s }
        return nil
    }

    func testPlainAndEscapedStrings() throws {
        XCTAssertEqual(try string(#""""#), "")
        XCTAssertEqual(try string(#""Größe über Übung""#), "Größe über Übung")
        XCTAssertEqual(try string(#""a\"b\\c\/d\b\f\n\r\te""#), "a\"b\\c/d\u{8}\u{C}\n\r\te")
        XCTAssertEqual(try string(#""üß""#), "üß")
        XCTAssertEqual(try string(#""x😀y""#), "x😀y")
        XCTAssertEqual(try string(#""\"""#), "\"")
        XCTAssertEqual(try string(#""ende\\""#), "ende\\")
    }

    func testStringsInsideStructures() throws {
        let value = try parse(#"{"b":["ä\"",{"c":"Az"}],"a":"x"}"#)
        XCTAssertEqual(value.kanonischerText, #"{"a":"x","b":["ä\"",{"c":"Az"}]}"#)
    }

    func testInvalidStringsAreRejected() {
        for bad in [#""offen"#, #""\"#, #""\x""#, #""\u12""#, #""\ud83d""#, #""\ude00""#, #""\ud83dA""#,
                    "\"zeile\numbruch\"", "\"tab\there\""] {
            XCTAssertThrowsError(try parse(bad), bad)
        }
        // Ungültiges UTF-8 bleibt ein Fehler, roh wie hinter einem Escape.
        XCTAssertThrowsError(try AblageJSON.parse([0x22, 0xC3, 0x28, 0x22]))
        XCTAssertThrowsError(try AblageJSON.parse([0x22, 0x5C, 0x6E, 0xFF, 0x22]))
    }

    func testLongStringRoundTrips() throws {
        let body = String(repeating: "Synthetischer Text mit Ümlauten – und \"Zitat\". ", count: 5000)
        let json = AblageJSON.objekt([("text", .string(body))]).kanonisch()
        guard case .object(let members) = try AblageJSON.parse(json), case .string(let back) = members.first?.value else {
            return XCTFail("kein Objekt")
        }
        XCTAssertEqual(back, body)
    }
}
