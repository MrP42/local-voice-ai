import XCTest

@MainActor
final class SimulatorControlsTests: XCTestCase {
    func testLockPhone() {
        let simulator = XCUIApplication(bundleIdentifier: "com.apple.iphonesimulator")
        simulator.activate()
        let phone = simulator.windows["Local Voice iPhone 15 Pro Max – iOS 26.3"]
        XCTAssertTrue(phone.waitForExistence(timeout: 10))
        phone.buttons["Sleep/Wake"].click()
        let attachment = XCTAttachment(screenshot: phone.screenshot())
        attachment.lifetime = .keepAlways
        add(attachment)
    }
    func testRaisePhone() {
        let simulator = XCUIApplication(bundleIdentifier: "com.apple.iphonesimulator")
        simulator.activate()
        let phone = simulator.windows["Local Voice iPhone 15 Pro Max – iOS 26.3"]
        XCTAssertTrue(phone.waitForExistence(timeout: 10))
        phone.toolbars.buttons["Home"].click()
    }
    func testLowerWrist() {
        setWrist(down: true)
    }
    func testRaiseWrist() {
        setWrist(down: false)
    }
    private func setWrist(down: Bool) {
        let simulator = XCUIApplication(bundleIdentifier: "com.apple.iphonesimulator")
        simulator.activate()
        let watch = simulator.windows["Local Voice Watch Ultra 3 – watchOS 26.2"]
        XCTAssertTrue(watch.waitForExistence(timeout: 10))
        let control = watch.toolbars.checkBoxes["Always On"]
        XCTAssertTrue(control.exists)
        if down {
            if String(describing: control.value ?? "") == "1" { control.click() }
            print("WRIST_CONTROL_READY")
            // The external test harness starts simctl; xcrun cannot run inside XCTest's app sandbox.
            Thread.sleep(forTimeInterval: 3)
        }
        if (String(describing: control.value ?? "") == "1") != down { control.click() }
        XCTAssertEqual(String(describing: control.value ?? ""), down ? "1" : "0")
    }
}
