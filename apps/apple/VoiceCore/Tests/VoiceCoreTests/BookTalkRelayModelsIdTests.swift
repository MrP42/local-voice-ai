import XCTest
@testable import VoiceCore


final class BookTalkRelayModelsIdTests: XCTestCase {
    func testDeviceKeyMatchesHubAndPc() {
        // Wie Rust `device_key` und PHP substr(hash('sha256', …), 0, 12).
        XCTAssertEqual(BookTalkRelay.deviceKey("abc"), "ba7816bf8f01")
    }

    func testModelsObjectIdPerDevice() {
        XCTAssertEqual(BookTalkRelay.modelsObjectId(desktopDeviceId: "pc-1", deviceId: "abc"), "models_pc-1_ba7816bf8f01")
    }
}
