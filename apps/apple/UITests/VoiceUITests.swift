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
}
