import XCTest

@MainActor
final class VoiceUITests: XCTestCase {
    private func keepScreenshot(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name; attachment.lifetime = .keepAlways; add(attachment)
    }

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
    func testRetryReconstructsComponentsAfterSetupFailure() throws {
        let app = XCUIApplication(); app.launchArguments = ["--setup-failure-probe"]; app.launch()
        let status = app.staticTexts["status"]
        XCTAssertTrue(status.waitForExistence(timeout: 15))
        XCTAssertTrue(status.label.contains("Speicherprüfung"))
        app.buttons["Erneut versuchen"].tap()
        let record = app.buttons["record"]; record.tap()
        expectation(for: NSPredicate(format: "label == %@", "Aufnahme sichern"), evaluatedWith: record)
        waitForExpectations(timeout: 15)
        record.tap()
        XCTAssertEqual(record.label, "Sprechen")
    }
    func testHistoryNavigationAndModelDismissal() throws {
        let app = XCUIApplication(); app.launch()
        app.buttons["Lokale Sprachmodelle"].tap()
        XCTAssertTrue(app.buttons["Fertig"].waitForExistence(timeout: 10))
        app.buttons["Fertig"].tap()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 10))
        app.tabBars.buttons["Verlauf"].tap()
        XCTAssertTrue(app.navigationBars["Verlauf"].waitForExistence(timeout: 10))
        let entry = app.buttons.matching(identifier: "historyEntry").firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 10)); entry.tap()
        XCTAssertTrue(app.staticTexts["Deine Aufnahme"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Antwort"].exists)
        keepScreenshot(app, name: "Sprachnotiz")
        app.navigationBars.buttons.firstMatch.tap()
        app.swipeDown()
        let search = app.textFields["historySearch"]
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        search.tap(); search.typeText("zz_nonexistent_voice_note_zz")
        XCTAssertTrue(app.staticTexts["Keine passende Aufnahme"].waitForExistence(timeout: 10))
        keepScreenshot(app, name: "Verlauf-Suche")
        let cancelSearch = app.buttons["closeSearch"]
        XCTAssertTrue(cancelSearch.waitForExistence(timeout: 5)); cancelSearch.tap()
        app.tabBars.buttons["Sprechen"].tap()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 10))
    }
    func testModelPanelShowsLocalModelChoices() throws {
        let app = XCUIApplication(); app.launch()
        let button = app.buttons["Lokale Sprachmodelle"]
        XCTAssertTrue(button.waitForExistence(timeout: 15)); button.tap()
        XCTAssertTrue(app.navigationBars["Lokale Sprachmodelle"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Whisper Base"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Whisper Small"].exists)
        XCTAssertTrue(app.staticTexts["Qwen – kurze Antworten"].exists)
        keepScreenshot(app, name: "Sprachmodelle")
    }
    func testDamagedHistoryIsVisibleAndCanBeRecovered() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--corrupt-history-probe"]
        app.launch()
        defer { app.launchArguments = ["--restore-history-probe"]; app.launch() }
        XCTAssertTrue(app.staticTexts["Verlauf beschädigt – Originaldateien erhalten"].waitForExistence(timeout: 15))
        app.swipeUp()
        XCTAssertTrue(app.buttons["Audiodatei sichern"].exists)
        XCTAssertTrue(app.buttons["Wiederherstellung versuchen"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot()); screenshot.lifetime = .keepAlways; add(screenshot)
        app.launchArguments = ["--restore-history-probe"]; app.launch()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 15))
        XCTAssertFalse(app.staticTexts["Verlauf beschädigt – Originaldateien erhalten"].exists)
    }
    #endif

}
