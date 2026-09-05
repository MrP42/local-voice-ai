import XCTest

@MainActor
final class VoiceUITests: XCTestCase {
    func testStartStopAndReturnFromHome() throws {
        let app = XCUIApplication()
        app.launch()
        let record = app.buttons["record"]
        XCTAssertTrue(record.waitForExistence(timeout: 15))
        record.tap()
        let permission = app.alerts.firstMatch
        if permission.waitForExistence(timeout: 2) {
            let allow = permission.buttons.matching(NSPredicate(format: "label IN %@", ["Erlauben", "Allow"])).firstMatch
            XCTAssertTrue(allow.exists)
            allow.tap()
        }
        let recording = NSPredicate(format: "label == %@", "Aufnahme sichern")
        expectation(for: recording, evaluatedWith: record)
        waitForExpectations(timeout: 15)
        record.tap()
        expectation(for: NSPredicate(format: "label == %@", "Sprechen"), evaluatedWith: record)
        waitForExpectations(timeout: 10)
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertTrue(record.waitForExistence(timeout: 10))
        XCTAssertEqual(record.label, "Sprechen")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
    #if os(iOS)
    func testModelPanelShowsLocalModelChoices() throws {
        let app = XCUIApplication(); app.launch()
        let button = app.buttons["Lokale Sprachmodelle"]
        XCTAssertTrue(button.waitForExistence(timeout: 15)); button.tap()
        XCTAssertTrue(app.navigationBars["Lokale Sprachmodelle"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Whisper Base"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Whisper Small"].exists)
        XCTAssertTrue(app.staticTexts["Qwen – kurze Antworten"].exists)
    }
    func testDamagedHistoryIsVisibleAndCanBeRecovered() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--corrupt-history-probe"]
        app.launch()
        defer { app.launchArguments = ["--restore-history-probe"]; app.launch() }
        XCTAssertTrue(app.staticTexts["Verlauf beschädigt – Originaldateien erhalten"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["Audiodatei sichern"].exists)
        XCTAssertTrue(app.buttons["Wiederherstellung versuchen"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot()); screenshot.lifetime = .keepAlways; add(screenshot)
        app.launchArguments = ["--restore-history-probe"]; app.launch()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 15))
        XCTAssertFalse(app.staticTexts["Verlauf beschädigt – Originaldateien erhalten"].exists)
    }
    #endif

}
