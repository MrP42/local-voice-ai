import XCTest
@testable import VoiceCore

final class ResponsePolicyTests: XCTestCase {
    func testActionRequestsGetAnHonestCapabilityBoundary() {
        for text in ["Stell einen Wecker auf sieben Uhr.", "Bitte schick meinem Chef eine Nachricht.", "Kannst du einen Tisch reservieren?", "Überweise zehn Euro."] {
            XCTAssertTrue(ResponsePolicy.isActionRequest(text), text)
        }
        XCTAssertFalse(ResponsePolicy.isActionRequest("Was bedeutet Photosynthese?"))
    }
    func testCompletionClaimsAreReplacedAndOrdinaryAnswersStayIntact() {
        for text in ["Ich habe den Wecker auf sieben Uhr gestellt.", "Die Nachricht wurde gesendet.", "Dein Tisch ist reserviert.", "Done, your alarm is set."] {
            XCTAssertEqual(ResponsePolicy.safeAnswer(text), ResponsePolicy.capabilityReply, text)
        }
        XCTAssertEqual(ResponsePolicy.safeAnswer("Berlin ist die Hauptstadt Deutschlands."), "Berlin ist die Hauptstadt Deutschlands.")
    }
    func testDigitalSilenceAndInvalidSamplesAreNotSpeechEvidence() {
        XCTAssertEqual(AudioSignal.assess([Float]()), .empty)
        XCTAssertEqual(AudioSignal.assess(Array(repeating: Float(0), count: 16000)), .silent)
        XCTAssertEqual(AudioSignal.assess([Float.nan]), .invalid)
        XCTAssertEqual(AudioSignal.assess([Float.infinity]), .invalid)
        XCTAssertEqual(AudioSignal.assess([0, Float(0.03), -0.02]), .signal)
    }
}
