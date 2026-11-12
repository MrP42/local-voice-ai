import XCTest
@testable import VoiceCore

/// Fehlertexte des Ablage-Abgleichs: ein Hub ohne Ablage-Sammlungen (vor wai-portal#21) wird benannt.
final class AblageFehlertextTests: XCTestCase {
    func testVeralteterHubWirdAlsServerUpdateBenannt() {
        XCTAssertEqual(AblageLaufFehler.hub(.http(422, "invalid_collection")).errorDescription,
                       "Ablage: Der Server kennt die Ablage noch nicht. Ein Update des Servers ist nötig.")
    }

    func testAndereFehlerBleibenBeimCode() {
        XCTAssertEqual(AblageLaufFehler.hub(.http(422, "checksum")).errorDescription,
                       "Ablage: Der Server antwortet mit Fehler 422.")
        XCTAssertEqual(AblageLaufFehler.hub(.http(500, "")).errorDescription,
                       "Ablage: Der Server antwortet mit Fehler 500.")
    }
}
