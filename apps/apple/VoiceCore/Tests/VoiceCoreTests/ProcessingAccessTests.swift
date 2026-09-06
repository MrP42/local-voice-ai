import XCTest
@testable import VoiceCore

final class ProcessingAccessTests: XCTestCase {
    func testWatchWakeAllowsProcessingWithoutForegroundAndExpiryRevokesIt() {
        var access = ProcessingAccess()
        XCTAssertFalse(access.allowed)
        access.watchLease = true
        XCTAssertTrue(access.allowed)
        access.watchLease = false
        XCTAssertFalse(access.allowed)
    }
    func testForegroundAndScheduledTaskHaveIndependentLifetimes() {
        var access = ProcessingAccess()
        access.foreground = true; access.watchLease = true
        access.watchLease = false
        XCTAssertTrue(access.allowed)
        access.scheduledTask = true; access.foreground = false
        XCTAssertTrue(access.allowed)
        access.scheduledTask = false
        XCTAssertFalse(access.allowed)
    }
}
