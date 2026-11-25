import XCTest
@testable import VoiceCore

final class LanguageDownloadChoiceTests: XCTestCase {
    func testRepeatedChoiceKeepsDownload() {
        var choice = LanguageDownloadChoice()
        XCTAssertEqual(choice.choose("en", installed: false, downloadable: true), .download("en"))
        XCTAssertEqual(choice.choose("en", installed: false, downloadable: true), .none)
        XCTAssertEqual(choice.loading, "en")
        XCTAssertNil(choice.pendingChoice)
        XCTAssertEqual(choice.finished(installed: ["en"]), .apply("en"))
        XCTAssertNil(choice.loading)
    }

    func testLastOtherChoiceWinsAfterSuccess() {
        var choice = LanguageDownloadChoice()
        _ = choice.choose("en", installed: false, downloadable: true)
        XCTAssertEqual(choice.choose("fr", installed: true, downloadable: false), .none)
        XCTAssertEqual(choice.choose("it", installed: true, downloadable: false), .none)
        XCTAssertEqual(choice.loading, "en")
        XCTAssertEqual(choice.pendingChoice, "it")
        XCTAssertEqual(choice.finished(installed: ["en", "fr", "it"]), .apply("it"))
    }

    func testPendingInstalledChoiceAppliesAfterFailure() {
        var choice = LanguageDownloadChoice()
        _ = choice.choose("en", installed: false, downloadable: true)
        _ = choice.choose("fr", installed: true, downloadable: false)
        XCTAssertEqual(choice.finished(installed: ["fr"]), .apply("fr"))
    }

    func testSameLoadingChoiceLeavesPendingChoiceUntouched() {
        var choice = LanguageDownloadChoice()
        _ = choice.choose("en", installed: false, downloadable: true)
        _ = choice.choose("fr", installed: true, downloadable: false)
        XCTAssertEqual(choice.choose("en", installed: false, downloadable: true), .none)
        XCTAssertEqual(choice.pendingChoice, "fr")
        XCTAssertEqual(choice.finished(installed: ["en", "fr"]), .apply("fr"))
    }

    func testMissingPendingChoiceDoesNotStartAnotherDownload() {
        var choice = LanguageDownloadChoice()
        _ = choice.choose("en", installed: false, downloadable: true)
        _ = choice.choose("fr", installed: false, downloadable: true)
        XCTAssertEqual(choice.finished(installed: ["en"]), .unavailable("fr"))
        XCTAssertNil(choice.loading)
        XCTAssertNil(choice.pendingChoice)
    }

    func testFailureAndCloseDiscardChoices() {
        var choice = LanguageDownloadChoice()
        _ = choice.choose("en", installed: false, downloadable: true)
        XCTAssertEqual(choice.finished(installed: []), .unavailable("en"))
        _ = choice.choose("en", installed: false, downloadable: true)
        _ = choice.choose("fr", installed: true, downloadable: false)
        choice.close()
        XCTAssertEqual(choice.finished(installed: ["en", "fr"]), .none)
        XCTAssertNil(choice.pendingChoice)
        XCTAssertEqual(choice.choose("it", installed: true, downloadable: false), .apply("it"))
    }
}
