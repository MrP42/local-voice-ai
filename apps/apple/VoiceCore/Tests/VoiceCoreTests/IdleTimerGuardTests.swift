import XCTest
@testable import VoiceCore

@MainActor
final class IdleTimerGuardTests: XCTestCase {
    private var calls: [Bool] = []
    private func makeGuard() -> IdleTimerGuard { IdleTimerGuard { [unowned self] in self.calls.append($0) } }

    func testSingleAcquireAndRelease() {
        let g = makeGuard()
        let t = g.acquire("Player")
        XCTAssertTrue(g.isActive)
        g.release(t)
        XCTAssertFalse(g.isActive)
        XCTAssertEqual(calls, [true, false])
    }

    /// Player schließt, während die Live-Aufnahme läuft: die Aufnahme hält das Display weiter wach.
    func testOverlappingRequestsKeepAwakeUntilLastRelease() {
        let g = makeGuard()
        let live = g.acquire("Live-Aufnahme")
        let player = g.acquire("Player")
        g.release(player)
        XCTAssertTrue(g.isActive)
        XCTAssertEqual(g.reasons, ["Live-Aufnahme"])
        g.release(live)
        XCTAssertFalse(g.isActive)
        XCTAssertEqual(calls, [true, false])
    }

    /// Umgekehrte Reihenfolge und zwei Player-Instanzen.
    func testReleaseOrderDoesNotMatter() {
        let g = makeGuard()
        let a = g.acquire("Player")
        let b = g.acquire("Player")
        g.release(a)
        XCTAssertTrue(g.isActive)
        g.release(b)
        XCTAssertFalse(g.isActive)
        XCTAssertEqual(calls, [true, false])
    }

    func testDoubleAndNilReleaseAreHarmless() {
        let g = makeGuard()
        let keep = g.acquire("Live-Aufnahme")
        let t = g.acquire("Player")
        g.release(t)
        g.release(t)
        g.release(nil)
        XCTAssertTrue(g.isActive)
        XCTAssertEqual(calls, [true])
        g.release(keep)
        XCTAssertEqual(calls, [true, false])
    }

    func testReacquireAfterRelease() {
        let g = makeGuard()
        g.release(g.acquire("Player"))
        _ = g.acquire("Player")
        XCTAssertEqual(calls, [true, false, true])
    }
}
