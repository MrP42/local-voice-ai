import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Widerruf mit Freigabe-Generation. Reine Typen, kein Modell.

private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    func hit() { lock.lock(); n += 1; lock.unlock() }
    var value: Int { lock.lock(); defer { lock.unlock() }; return n }
}

private final class Flags: @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [String] = []
    func add(_ s: String) { lock.lock(); seen.append(s); lock.unlock() }
    var all: [String] { lock.lock(); defer { lock.unlock() }; return seen }
}

final class AssistantReleaseTests: XCTestCase {
    func testGenerationIsMonotoneAndOnlyRealChangesCount() {
        let r = AssistantRelease(optIn: false)
        XCTAssertEqual(r.snapshot, .init(generation: 0, optIn: false, consentVersion: 0))
        XCTAssertEqual(r.setOptIn(false), 0, "keine Änderung")
        XCTAssertEqual(r.setOptIn(true), 1)
        XCTAssertEqual(r.setOptIn(true), 1)
        XCTAssertEqual(r.bump(), 2)
        XCTAssertEqual(r.setOptIn(false), 3)
        XCTAssertEqual(r.setOptIn(true), 4, "Aus und wieder Ein: eine neue Generation, nie die alte")
        XCTAssertEqual(r.snapshot, .init(generation: 4, optIn: true, consentVersion: 0))
        XCTAssertEqual(r.currentGeneration, 4)
        XCTAssertTrue(r.isOptedIn)
    }

    func testBumpRevokesEveryRegisteredRunOnceAndOutsideTheLock() {
        let r = AssistantRelease(optIn: true)
        let revoked = Counter()
        let readsInside = Flags()
        for id in ["a", "b", "c"] {
            XCTAssertTrue(r.register(id, generation: 0) {
                // Würde der Lock noch gehalten, hinge dieser Zugriff.
                readsInside.add("\(id)@\(r.currentGeneration)")
                revoked.hit()
            })
        }
        r.bump()
        XCTAssertEqual(revoked.value, 3)
        XCTAssertEqual(Set(readsInside.all), ["a@1", "b@1", "c@1"], "beim Widerruf gilt die neue Generation schon")
        r.bump()
        XCTAssertEqual(revoked.value, 3, "ein Lauf wird nur einmal widerrufen")
    }

    func testOptInChangeRevokesRunsButNoChangeDoesNot() {
        let r = AssistantRelease(optIn: false)
        let revoked = Counter()
        XCTAssertTrue(r.register("a", generation: 0) { revoked.hit() })
        r.setOptIn(false)
        XCTAssertEqual(revoked.value, 0)
        r.setOptIn(true)
        XCTAssertEqual(revoked.value, 1)
    }

    func testRegisteringWithAStaleGenerationIsRefusedAndNotKept() {
        let r = AssistantRelease(optIn: true)
        let g = r.currentGeneration
        r.bump()
        let revoked = Counter()
        XCTAssertFalse(r.register("late", generation: g) { revoked.hit() })
        r.bump()
        XCTAssertEqual(revoked.value, 0, "ein abgelehnter Lauf wird nicht aufgenommen; der Aufrufer widerruft selbst")
    }

    func testUnregisteredRunIsNotRevoked() {
        let r = AssistantRelease(optIn: true)
        let revoked = Counter()
        XCTAssertTrue(r.register("a", generation: 0) { revoked.hit() })
        r.unregister("a")
        r.bump()
        XCTAssertEqual(revoked.value, 0)
    }

    func testIfCurrentRunsTheBodyOnlyForTheCurrentGeneration() {
        let r = AssistantRelease(optIn: true)
        let ran = Counter()
        XCTAssertTrue(r.ifCurrent(0) { ran.hit() })
        r.bump()
        XCTAssertFalse(r.ifCurrent(0) { ran.hit() })
        XCTAssertFalse(r.ifCurrent(5) { ran.hit() }, "auch eine künftige Generation passt nicht")
        XCTAssertTrue(r.ifCurrent(1) { ran.hit() })
        XCTAssertEqual(ran.value, 2)
    }

    func testGateAdmitsOnlyTheCurrentGeneration() {
        XCTAssertTrue(AssistantReleaseGate.admits(eventGeneration: 3, current: 3))
        XCTAssertFalse(AssistantReleaseGate.admits(eventGeneration: 2, current: 3), "veraltet")
        XCTAssertFalse(AssistantReleaseGate.admits(eventGeneration: 4, current: 3))
        let r = AssistantRelease(optIn: true)
        let before = r.currentGeneration
        r.setOptIn(false)
        r.setOptIn(true)
        XCTAssertFalse(AssistantReleaseGate.admits(eventGeneration: before, current: r.currentGeneration), "Aus→Ein belebt nichts")
    }

    // MARK: Ausgang eines Laufs

    private func makeOutput(_ release: AssistantRelease, generation: Int)
        -> (AssistantRunOutput, AsyncStream<AssistantEvent>) {
        var cont: AsyncStream<AssistantEvent>.Continuation!
        let stream = AsyncStream<AssistantEvent> { cont = $0 }
        return (AssistantRunOutput(release: release, generation: generation, continuation: cont), stream)
    }

    private func sentence(_ text: String, generation: Int) -> AssistantEvent {
        .sentence(requestId: "r1", generation: generation, text: text, speech: text, status: .unsourced, sources: [], quote: false)
    }

    func testNothingComesOutAfterBumpReturned() async {
        let r = AssistantRelease(optIn: true)
        let (out, stream) = makeOutput(r, generation: r.currentGeneration)
        XCTAssertTrue(out.send(sentence("Erster.", generation: 0)))
        r.bump()
        XCTAssertFalse(out.send(sentence("Zweiter.", generation: 0)), "geprüfter Satz, dann Widerruf, dann kein Ereignis")
        out.sendUnchecked(.failed(requestId: "r1", .revoked))
        out.sendUnchecked(.failed(requestId: "r1", .cancelled))
        var got: [AssistantEvent] = []
        let reader = Task { () -> [AssistantEvent] in
            var all: [AssistantEvent] = []
            for await e in stream { all.append(e); if all.count == 3 { break } }
            return all
        }
        got = await reader.value
        XCTAssertEqual(got, [sentence("Erster.", generation: 0), .failed(requestId: "r1", .revoked), .failed(requestId: "r1", .cancelled)])
    }

    /// Ein Ereignis der alten Generation, das nach einem der neuen herauskäme, wird nicht gesendet.
    func testSwappedOutputOfAnOlderGenerationIsNotAdmittedAfterANewerOne() async {
        let r = AssistantRelease(optIn: true)
        let (oldOut, oldStream) = makeOutput(r, generation: 0)
        r.setOptIn(false)
        let (newOut, newStream) = makeOutput(r, generation: r.currentGeneration)
        XCTAssertTrue(newOut.send(sentence("Neu.", generation: 1)))
        XCTAssertFalse(oldOut.send(sentence("Alt.", generation: 0)))
        newOut.sendUnchecked(.failed(requestId: "r1", .cancelled))
        oldOut.sendUnchecked(.failed(requestId: "r1", .cancelled))
        var fromNew: [AssistantEvent] = []
        for await e in newStream { fromNew.append(e); if fromNew.count == 2 { break } }
        var fromOld: [AssistantEvent] = []
        for await e in oldStream { fromOld.append(e); if fromOld.count == 1 { break } }
        XCTAssertEqual(fromNew.first, sentence("Neu.", generation: 1))
        XCTAssertEqual(fromOld, [.failed(requestId: "r1", .cancelled)], "vom alten Lauf kommt nur das Ende ohne Inhalt")
    }

    /// Nebenläufig: Sobald `bump` zurückgekehrt ist, kommt kein Ereignis der alten Generation mehr heraus.
    func testNoOldEventAfterBumpReturnedUnderContention() {
        let r = AssistantRelease(optIn: true)
        let sent = Counter()
        let group = DispatchGroup()
        for _ in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                let deadline = Date().addingTimeInterval(5)
                while Date() < deadline {
                    if !r.ifCurrent(0, { sent.hit() }) { break }
                }
                group.leave()
            }
        }
        Thread.sleep(forTimeInterval: 0.05)
        r.bump()
        let atBump = sent.value
        XCTAssertEqual(group.wait(timeout: .now() + 10), .success)
        XCTAssertGreaterThan(atBump, 0, "die Erzeuger liefen vor dem Widerruf")
        XCTAssertEqual(sent.value, atBump, "nach der Rückkehr von bump kommt nichts Altes mehr")
    }

    func testConcurrentBumpsAreStrictlyCounted() {
        let r = AssistantRelease(optIn: true)
        let group = DispatchGroup()
        for _ in 0..<8 {
            group.enter()
            DispatchQueue.global().async {
                for _ in 0..<250 { r.bump() }
                group.leave()
            }
        }
        XCTAssertEqual(group.wait(timeout: .now() + 20), .success)
        XCTAssertEqual(r.currentGeneration, 2_000)
    }

    // MARK: Beobachter, Einwilligungsversion

    func testObserverFiresOnEveryGenerationIncreaseAndOutsideTheLock() {
        let r = AssistantRelease(optIn: false)
        let seen = Flags()
        r.setObserver { g in seen.add("\(g)@\(r.currentGeneration)") }   // würde der Lock gehalten, hinge das Lesen
        r.bump()
        r.setOptIn(true)
        r.setOptIn(true)    // keine Änderung: kein Ruf
        r.bump()
        r.setOptIn(false)
        XCTAssertEqual(seen.all, ["1@1", "2@2", "3@3", "4@4"])
        r.setObserver(nil)
        r.bump()
        XCTAssertEqual(seen.all.count, 4, "ohne Beobachter kein Ruf")
    }

    func testObserverFiresEvenWhenNoRunIsRegistered() {
        let r = AssistantRelease(optIn: true)
        let fired = Counter()
        r.setObserver { _ in fired.hit() }
        r.bump()
        XCTAssertEqual(fired.value, 1, "auch ohne laufende Antwort (Wiedergabe nach Modellende)")
    }

    func testConsentVersionTravelsWithOptInAndIsReadFromTheSnapshot() {
        let r = AssistantRelease(optIn: false, consentVersion: 7)
        XCTAssertEqual(r.snapshot.consentVersion, 0, "aus: keine Einwilligung")
        r.setOptIn(true, consentVersion: 2)
        XCTAssertEqual(r.snapshot.consentVersion, 2)
        let g = r.currentGeneration
        r.setOptIn(true, consentVersion: 3)
        XCTAssertEqual(r.snapshot.consentVersion, 3, "neue Fassung bei unverändertem Zustand wird nachgeführt")
        XCTAssertEqual(r.currentGeneration, g, "ohne Erhöhung der Generation")
        r.setOptIn(false)
        XCTAssertEqual(r.snapshot.consentVersion, 0)
    }

    func testRegisteredRunsAreCountedAndStayUntilUnregistered() {
        let r = AssistantRelease(optIn: true)
        XCTAssertTrue(r.register("a", generation: 0) {})
        XCTAssertTrue(r.register("b", generation: 0) {})
        XCTAssertEqual(r.registeredCount, 2)
        r.unregister("a")
        XCTAssertEqual(r.registeredCount, 1)
        r.bump()
        XCTAssertEqual(r.registeredCount, 0)
    }

    // MARK: Schrittindex (Rückmeldungen des Controllers)

    func testStepIndexMapsEventNumbersToSteps() {
        let index = AssistantStepIndex()
        index.append("a", step: 1)
        index.append("a", step: 3)
        index.append("b", step: 5)
        XCTAssertEqual(index.step("a", index: 0), 1)
        XCTAssertEqual(index.step("a", index: 1), 3)
        XCTAssertNil(index.step("a", index: 2))
        XCTAssertEqual(index.step("b", index: 0), 5)
        XCTAssertNil(index.step("unbekannt", index: 0))
        index.removeLast("a")
        XCTAssertNil(index.step("a", index: 1), "ein nicht gesendetes Ereignis ist wieder ausgetragen")
        XCTAssertEqual(index.step("a", index: 0), 1)
    }

    func testStepIndexIsBounded() {
        let index = AssistantStepIndex()
        for i in 0..<(AssistantStepIndex.maxRequests + 10) { index.append("r\(i)", step: 1) }
        XCTAssertNil(index.step("r0", index: 0), "die ältesten Anfragen fallen weg")
        XCTAssertEqual(index.step("r\(AssistantStepIndex.maxRequests + 9)", index: 0), 1)
        for i in 0..<(AssistantStepIndex.maxSteps + 50) { index.append("viele", step: i) }
        XCTAssertNotNil(index.step("viele", index: AssistantStepIndex.maxSteps - 1))
        XCTAssertNil(index.step("viele", index: AssistantStepIndex.maxSteps))
    }
}
