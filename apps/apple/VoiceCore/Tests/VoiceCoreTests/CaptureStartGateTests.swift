import XCTest
@testable import VoiceCore

final class CaptureStartGateTests: XCTestCase {
    func testInactiveSceneCannotRequestRecording() {
        var gate = CaptureStartGate()
        XCTAssertNil(gate.begin(active: false))
    }
    func testCurrentForegroundPermissionStartsRecordingOnce() {
        var gate = CaptureStartGate()
        let token = gate.begin(active: true)!
        XCTAssertNil(gate.begin(active: true))
        XCTAssertEqual(gate.resolve(token, allowed: true, active: true), .start)
        XCTAssertEqual(gate.resolve(token, allowed: true, active: true), .stale)
    }
    func testLeavingAndReturningDoesNotRevivePendingRecording() {
        var gate = CaptureStartGate()
        let token = gate.begin(active: true)!
        gate.sceneBecameInactive()
        XCTAssertEqual(gate.resolve(token, allowed: true, active: true), .cancelled)
    }
    func testPermissionArrivingWhileInactiveCannotStart() {
        var gate = CaptureStartGate()
        let token = gate.begin(active: true)!
        XCTAssertEqual(gate.resolve(token, allowed: true, active: false), .cancelled)
    }
    func testDenialRemainsVisibleAfterPermissionDialogInactivatesScene() {
        var gate = CaptureStartGate()
        let token = gate.begin(active: true)!
        gate.sceneBecameInactive()
        XCTAssertEqual(gate.resolve(token, allowed: false, active: false), .denied)
    }
    func testOldCallbackCannotConsumeNewRequest() {
        var gate = CaptureStartGate()
        let old = gate.begin(active: true)!
        XCTAssertEqual(gate.resolve(old, allowed: false, active: true), .denied)
        let current = gate.begin(active: true)!
        XCTAssertEqual(gate.resolve(old, allowed: true, active: true), .stale)
        XCTAssertEqual(gate.resolve(current, allowed: true, active: true), .start)
    }
}
