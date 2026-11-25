import XCTest
@testable import VoiceCore

final class LocalMinutesResourcesTests: XCTestCase {
    func testBudgetIncludesWeightsContextAndBuffer() {
        // Synthetische Datei: 1000 Bytes + 1 GiB Kontext/KV + 512 MiB Puffer.
        XCTAssertFalse(LocalMinutesResources.canLoadQwen3(availableBytes: 1_610_613_735, modelBytes: 1000))
        XCTAssertTrue(LocalMinutesResources.canLoadQwen3(availableBytes: 1_610_613_736, modelBytes: 1000))
    }

    func testUnknownAndOverflowBudgetsFailClosed() {
        XCTAssertFalse(LocalMinutesResources.canLoadQwen3(availableBytes: 0, modelBytes: 1000))
        XCTAssertFalse(LocalMinutesResources.canLoadQwen3(availableBytes: .max, modelBytes: 0))
        XCTAssertFalse(LocalMinutesResources.canLoadQwen3(availableBytes: .max, modelBytes: .max))
    }

    func testOnlyResourceFailureAllowsFallback() {
        XCTAssertNil(LocalMinutesResources.chatFailure(code: 0))
        XCTAssertEqual(LocalMinutesResources.chatFailure(code: 3), .unavailable)
        XCTAssertEqual(LocalMinutesResources.chatFailure(code: 4), .contextExceeded)
        XCTAssertEqual(LocalMinutesResources.chatFailure(code: 7), .cancelled)
        for code in [Int32(1), 2, 5, 8, 12, 99] {
            XCTAssertEqual(LocalMinutesResources.chatFailure(code: code), .provider(.invalidResponse))
        }
    }
}
