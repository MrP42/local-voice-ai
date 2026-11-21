import Foundation
import XCTest
@testable import VoiceCore

/// Nur Zähler und Testzustand; keine Audiodaten oder Inhalte.
private final class MuteDeliveryCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var closed = false
    private var count = 0
    private var violations = 0

    func setClosed(_ value: Bool) { lock.withLock { closed = value } }

    func record() {
        lock.withLock {
            count += 1
            if closed { violations += 1 }
        }
    }

    var totals: (count: Int, violations: Int) {
        lock.withLock { (count, violations) }
    }
}

final class AssistantMuteGateTests: XCTestCase {
    func testMuteReturnPreventsWorkAndUnmuteAllowsItAgain() {
        let gate = AssistantMuteGate()
        let counter = MuteDeliveryCounter()
        gate.configure(muted: false)
        gate.setLive(true)
        XCTAssertTrue(gate.deliver { counter.record() })
        gate.setMuted(true)
        for _ in 0..<100 {
            XCTAssertFalse(gate.deliver { counter.record() })
        }
        XCTAssertEqual(counter.totals.count, 1)
        gate.setMuted(false)
        XCTAssertTrue(gate.deliver { counter.record() })
        XCTAssertEqual(counter.totals.count, 2)
    }

    func testConfiguredMuteAndStoppedGateRejectWork() {
        let gate = AssistantMuteGate()
        var count = 0
        XCTAssertFalse(gate.deliver { count += 1 })
        gate.configure(muted: true)
        gate.setLive(true)
        XCTAssertFalse(gate.deliver { count += 1 })
        gate.setMuted(false)
        XCTAssertTrue(gate.deliver { count += 1 })
        gate.setLive(false)
        XCTAssertFalse(gate.deliver { count += 1 })
        XCTAssertEqual(count, 1)
    }

    func testPreparedWorkCannotCrossMuteOrConfigurationChanges() throws {
        let gate = AssistantMuteGate()
        gate.configure(muted: false)
        gate.setLive(true)
        let beforeMute = try XCTUnwrap(gate.snapshot())
        gate.setMuted(true)
        gate.setMuted(false)
        var count = 0
        XCTAssertFalse(gate.deliver(generation: beforeMute) { count += 1 })
        let beforeConfigure = try XCTUnwrap(gate.snapshot())
        gate.configure(muted: false)
        gate.setLive(true)
        XCTAssertFalse(gate.deliver(generation: beforeConfigure) { count += 1 })
        let beforeStop = try XCTUnwrap(gate.snapshot())
        gate.setLive(false)
        gate.setLive(true)
        XCTAssertFalse(gate.deliver(generation: beforeStop) { count += 1 })
        let current = try XCTUnwrap(gate.snapshot())
        XCTAssertTrue(gate.deliver(generation: current) { count += 1 })
        XCTAssertEqual(count, 1)
    }

    func testMuteWaitsForAnOngoingDeliveryToFinish() {
        let muting = DispatchSemaphore(value: 0)
        let gate = AssistantMuteGate { lock in
            // Gegenprobe: Ohne Sperre während work gelingt dieser echte Sperrversuch.
            let acquired = lock.try()
            if acquired { lock.unlock() }
            XCTAssertFalse(acquired, "Laufende Übergabe muss die Sperre halten")
            muting.signal()
        }
        gate.configure(muted: false)
        gate.setLive(true)
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        let completed = DispatchSemaphore(value: 0)
        let returned = DispatchSemaphore(value: 0)
        let deliveryEnded = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            gate.deliver {
                entered.signal()
                release.wait()
                completed.signal()
            }
            deliveryEnded.signal()
        }
        defer { release.signal() }
        guard entered.wait(timeout: .now() + 5) == .success else {
            XCTFail("Übergabe wurde nicht gestartet")
            return
        }
        DispatchQueue.global().async {
            gate.setMuted(true)
            returned.signal()
        }
        XCTAssertEqual(muting.wait(timeout: .now() + 5), .success)
        // Der Sperrversuch im Haken ist abgeschlossen; keine Wartezeit als Beweis.
        let earlyReturn = returned.wait(timeout: .now())
        XCTAssertEqual(earlyReturn, .timedOut)
        release.signal()
        if earlyReturn != .success {
            XCTAssertEqual(returned.wait(timeout: .now() + 5), .success)
        }
        XCTAssertEqual(completed.wait(timeout: .now()), .success)
        XCTAssertEqual(deliveryEnded.wait(timeout: .now() + 5), .success)
        XCTAssertFalse(gate.deliver { XCTFail("Übergabe nach Stummrückkehr") })
    }

    func testParallelDeliveriesNeverRunDuringConfirmedMute() {
        let gate = AssistantMuteGate()
        let counter = MuteDeliveryCounter()
        gate.configure(muted: false)
        gate.setLive(true)
        gate.deliver { counter.record() }
        DispatchQueue.concurrentPerform(iterations: 5) { worker in
            if worker == 0 {
                for _ in 0..<1_000 {
                    gate.setMuted(true)
                    counter.setClosed(true)
                    // Auch im bestätigten Stummfenster aktiv Übergaben versuchen.
                    for _ in 0..<10 { gate.deliver { counter.record() } }
                    counter.setClosed(false)
                    gate.setMuted(false)
                }
            } else {
                for _ in 0..<10_000 { gate.deliver { counter.record() } }
            }
        }
        gate.setMuted(true)
        counter.setClosed(true)
        let before = counter.totals.count
        DispatchQueue.concurrentPerform(iterations: 1_000) { _ in
            gate.deliver { counter.record() }
        }
        XCTAssertGreaterThan(before, 0)
        XCTAssertEqual(counter.totals.count, before)
        XCTAssertEqual(counter.totals.violations, 0)
    }
}
