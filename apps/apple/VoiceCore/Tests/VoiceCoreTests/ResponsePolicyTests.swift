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
    func testStationaryBroadbandNoiseIsFlaggedButModulatedSignalIsRetained() {
        var seed: UInt64 = 42
        let noise: [Float] = (0..<64000).map { _ in
            seed = seed &* 6364136223846793005 &+ 1
            return (Float(seed >> 32) / Float(UInt32.max) - 0.5) * 0.1
        }
        XCTAssertEqual(AudioSignal.assess(noise), .stationaryNoise)
        let modulated = noise.enumerated().map { index, sample in sample * (index / 320 % 20 < 10 ? Float(1) : Float(0.05)) }
        XCTAssertEqual(AudioSignal.assess(modulated), .signal)
    }

}
