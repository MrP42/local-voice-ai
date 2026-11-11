import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Klasse B): die Meldestelle der Quellen (`AssistantContentWatch`).
// Vor jeder Änderung einer Quelle ruft sie synchron `contentWillChange` der Sitzung (Generation steigt, Bestand `stale`); der Bestand
// steht erst, wenn ein Durchgang der Indexierung, der NACH der Änderung begann, regulär endet. Nur synthetische Daten.

private struct QuietModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { $0.finish() }
    }
}

final class AssistantContentWatchTests: XCTestCase {
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)

    private func session() -> AssistantSession {
        AssistantSession(model: QuietModel(), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: config, released: true)
    }

    func testAChangeBeforeTheSessionIsAttachedMakesTheSessionStaleUntilAPassEnds() {
        let watch = AssistantContentWatch()
        let ticket = watch.willChange()
        watch.changed(ticket)
        let s = session()
        XCTAssertTrue(s.isContentReady, "eine neue Sitzung ist zunächst bereit")
        watch.attach(s)
        XCTAssertFalse(s.isContentReady, "der Bestand entspricht nicht dem Stand der Quellen")
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(s.isContentReady)
    }

    func testWillChangeRaisesTheGenerationSynchronouslyAndMarksTheStockStale() {
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        XCTAssertTrue(s.isContentReady)
        let before = s.releaseGeneration
        let ticket = watch.willChange()
        XCTAssertGreaterThan(s.releaseGeneration, before, "die Generation steht beim Rückkehren des Aufrufs")
        XCTAssertFalse(s.isContentReady)
        XCTAssertTrue(watch.isStale)
        watch.changed(ticket)
        XCTAssertFalse(s.isContentReady, "die Änderung allein macht den Bestand nicht bereit")
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(s.isContentReady)
        XCTAssertFalse(watch.isStale)
    }

    func testAPassThatBeganBeforeTheChangeEndedDoesNotInstallIt() {
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        let ticket = watch.willChange()
        let early = watch.mark   // Durchgang beginnt, bevor die Änderung beendet ist
        XCTAssertEqual(early, 0)
        watch.changed(ticket)
        watch.passCompleted(mark: early)
        XCTAssertFalse(s.isContentReady, "die Marke deckt die Änderung nicht")
        XCTAssertTrue(watch.isStale)
        watch.passCompleted(mark: watch.mark)
        XCTAssertTrue(s.isContentReady)
    }

    func testAChangeDuringAPassKeepsTheStockStale() {
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        let first = watch.willChange()
        watch.changed(first)
        let mark = watch.mark
        let second = watch.willChange()   // eine zweite Änderung, während der Durchgang läuft
        XCTAssertFalse(watch.passCompleted(mark: mark))
        XCTAssertFalse(s.isContentReady)
        watch.changed(second)
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(s.isContentReady)
    }

    func testAnUnfinishedChangeHoldsTheMarkBackSoTheStockStaysStale() {
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        let open = watch.willChange()
        let later = watch.willChange()
        watch.changed(later)
        XCTAssertEqual(watch.mark, 0, "die frühere Änderung ist nicht beendet: die Marke bleibt davor")
        watch.passCompleted(mark: watch.mark)
        XCTAssertFalse(s.isContentReady)
        watch.changed(open)
        XCTAssertEqual(watch.mark, 2)
        watch.passCompleted(mark: watch.mark)
        XCTAssertTrue(s.isContentReady)
    }

    func testAroundReportsTheEndAlsoWhenTheChangeFails() {
        struct Failure: Error {}
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        XCTAssertThrowsError(try watch.around { throw Failure() })
        XCTAssertEqual(watch.mark, 1, "das Ende ist gemeldet, die Marke rückt vor")
        watch.passCompleted(mark: watch.mark)
        XCTAssertTrue(s.isContentReady)
        XCTAssertEqual(watch.around { 7 }, 7)
    }

    func testTheObserverIsCalledAfterEveryFinishedChangeOutsideTheLock() {
        let watch = AssistantContentWatch()
        let calls = Counter()
        watch.setObserver { [watch] in
            _ = watch.mark   // würde bei gehaltener Sperre hängen
            calls.add()
        }
        watch.around {}
        watch.around {}
        XCTAssertEqual(calls.value, 2)
    }

    func testWithoutASessionOnlyTheCountersMove() {
        let watch = AssistantContentWatch()
        watch.around {}
        watch.around {}
        XCTAssertEqual(watch.issuedCount, 2)
        XCTAssertTrue(watch.isStale)
        XCTAssertFalse(watch.passCompleted(mark: 1), "ein Durchgang bis Marke 1 deckt Änderung 2 nicht")
        XCTAssertTrue(watch.passCompleted(mark: 2))
        XCTAssertFalse(watch.isStale)
        let s = session()
        watch.attach(s)
        XCTAssertTrue(s.isContentReady, "der Bestand steht: die neue Sitzung ist bereit")
    }

    /// Runde 8 (R4): Die Meldestelle ruft die Ordnung des Prozesses unmittelbar, gleich ob ein Gespräch offen ist. `attach(nil)` löst
    /// nichts (das Ende eines Gesprächs lässt Änderungen nicht wirkungslos), und eine Änderung ohne Gespräch erhöht die Generation.
    func testTheOrderStaysBoundWhenAConversationEnds() {
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        watch.attach(nil)
        let before = s.releaseGeneration
        watch.around {}
        XCTAssertEqual(s.releaseGeneration, before + 1, "die Ordnung bleibt gebunden: auch ohne Gespräch erhöht jede Änderung")
        XCTAssertFalse(s.isContentReady, "und der Bestand ist stale, bis ein Durchgang ihn meldet")
    }

    /// Viele Änderungen von vielen Threads, ein Durchgang dazwischen und einer am Ende: am Ende steht der Bestand, und die
    /// Generation ist um mindestens die Zahl der Änderungen gestiegen. Mehrfach ausführen (Nebenläufigkeit).
    func testConcurrentChangesAndPassesEndUpReadyAndNeverDeadlock() async {
        let watch = AssistantContentWatch()
        let s = session()
        watch.attach(s)
        let before = s.releaseGeneration
        let writers = 8
        let each = 40
        await withTaskGroup(of: Void.self) { group in
            for _ in 0..<writers {
                group.addTask {
                    for _ in 0..<each { watch.around {} }
                }
            }
            group.addTask {
                for _ in 0..<each { watch.passCompleted(mark: watch.mark) }
            }
        }
        XCTAssertEqual(watch.issuedCount, writers * each)
        XCTAssertGreaterThanOrEqual(s.releaseGeneration - before, writers * each)
        XCTAssertEqual(watch.mark, writers * each, "alle Änderungen sind beendet")
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(s.isContentReady)
        XCTAssertFalse(watch.isStale)
    }
}

private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    func add() { lock.lock(); count += 1; lock.unlock() }
    var value: Int { lock.lock(); defer { lock.unlock() }; return count }
}
