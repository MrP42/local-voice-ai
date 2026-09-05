import XCTest

@MainActor
final class SimulatorControlsTests: XCTestCase {
    func testInspectSimulatorControls() {
        let simulator = XCUIApplication(bundleIdentifier: "com.apple.iphonesimulator")
        simulator.activate()
        XCTAssertTrue(simulator.wait(for: .runningForeground, timeout: 10))
        for window in simulator.windows.allElementsBoundByIndex {
            print("SIMULATOR_WINDOW: \(window.title)")
            for button in window.buttons.allElementsBoundByIndex { print("SIMULATOR_BUTTON: \(button.identifier) | \(button.label)") }
        }
        for menu in simulator.menuBars.menuBarItems.allElementsBoundByIndex { print("SIMULATOR_MENU: \(menu.title)") }
    }
}
