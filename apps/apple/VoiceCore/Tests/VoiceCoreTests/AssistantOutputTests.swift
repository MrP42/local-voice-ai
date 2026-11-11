import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, Runde 4, Punkt 2): geordnete Ausgabegrenze. Jeder Ausgabe-Commit läuft im selben Lock wie
// die Erhöhung der Freigabe-Generation (`commitOutput`); die Wiedergabesperre („verworfen ab Generation g“) rückt im selben
// Lock vor. Reine Kern-Typen; der Speaker und der Controller (MainActor) sind nur am Mac prüfbar.

private final class Tally: @unchecked Sendable {
    private let lock = NSLock()
    private var ran = 0
    private var refused = 0
    private var violations: [String] = []
    private var highest = 0
    func ranOnce() { lock.lock(); ran += 1; lock.unlock() }
    func refusedOnce() { lock.lock(); refused += 1; lock.unlock() }
    func violation(_ s: String) { lock.lock(); violations.append(s); lock.unlock() }
    /// Höchste Generation, deren Erhöhung bereits zurückgekehrt ist.
    func returned(_ g: Int) { lock.lock(); highest = max(highest, g); lock.unlock() }
    var returnedGeneration: Int { lock.lock(); defer { lock.unlock() }; return highest }
    var snapshot: (ran: Int, refused: Int, violations: [String]) {
        lock.lock()
        defer { lock.unlock() }
        return (ran, refused, violations)
    }
}

/// Warten in synchronem Kontext (in `async`-Tests ist `Thread.sleep` nicht erlaubt; hier geht es um echte Threads).
private func pause(_ seconds: TimeInterval) { Thread.sleep(forTimeInterval: seconds) }

private struct IdleModel: BookTalkModel {
    var pieces: [String] = []
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let pieces = pieces
        return AsyncThrowingStream { c in
            for p in pieces { c.yield(p) }
            c.finish()
        }
    }
}

final class AssistantOutputTests: XCTestCase {
    private var dir: URL!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-output-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil,
                                         timeout: 5, allowUnsourced: true, consentVersion: 1)

    private func knowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", detail: "Autorin Zimtstern", sections: [
            ("S. 7", "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func session(audit: AssistantAuditLog? = nil, pieces: [String] = [], history: [AssistantTurn] = []) async -> AssistantSession {
        AssistantSession(model: IdleModel(pieces: pieces), knowledge: await knowledge(), config: config, history: history, audit: audit,
                         released: true)
    }

    // MARK: Wiedergabesperre (reiner Typ)

    func testPlaybackGateDiscardsBelowAndIsMonotone() {
        let g = AssistantPlaybackGate()
        XCTAssertTrue(g.permits(0))
        g.discard(below: 3)
        XCTAssertFalse(g.permits(2), "verworfen")
        XCTAssertTrue(g.permits(3), "die neue Generation darf starten")
        XCTAssertTrue(g.permits(9), "künftig ist nicht verworfen")
        g.discard(below: 1)
        XCTAssertEqual(g.discardedBelow, 3, "nie zurück: ein kleinerer Wert ändert nichts")
        XCTAssertFalse(g.permits(2))
    }

    func testEveryIncreaseOfTheGenerationAdvancesTheGateInTheSameStep() async {
        let s = await session()
        XCTAssertEqual(s.playbackGate.discardedBelow, 0)
        let g0 = s.releaseGeneration
        await s.setAllowUnsourced(false)   // Opt-in war an: echte Änderung
        XCTAssertEqual(s.playbackGate.discardedBelow, s.releaseGeneration)
        XCTAssertFalse(s.playbackGate.permits(g0), "Satz der alten Generation startet nicht mehr")
        await s.releaseDidChange()
        XCTAssertEqual(s.playbackGate.discardedBelow, s.releaseGeneration)
        await s.setAllowUnsourced(true)
        await s.setAreas([.books])
        XCTAssertEqual(s.playbackGate.discardedBelow, s.releaseGeneration, "auch Bereiche")
        XCTAssertTrue(s.playbackGate.permits(s.releaseGeneration))
        XCTAssertFalse(s.playbackGate.permits(s.releaseGeneration - 1))
        // „Ohne Quelle fortsetzen“ (Zustand .data nach einer Datenrunde)
        let d = await session(history: [AssistantTurn(user: "Frage", assistant: "Antwort", at: Date(), round: .data)])
        XCTAssertEqual(d.contextMode, .data)
        let before = d.releaseGeneration
        let r9 = await d.resumeFree()
        XCTAssertTrue(r9.changed)
        XCTAssertEqual(d.playbackGate.discardedBelow, before + 1)
        XCTAssertFalse(d.playbackGate.permits(before))
    }

    func testTheGateIsAlreadyClosedWhenTheObserverFires() async {
        let s = await session()
        let seen = Tally()
        let gate = s.playbackGate
        s.setReleaseObserver { g in
            if gate.permits(g - 1) { seen.violation("alte Generation \(g - 1) noch erlaubt") }
            if !gate.permits(g) { seen.violation("neue Generation \(g) gesperrt") }
            seen.ranOnce()
        }
        await s.releaseDidChange()
        await s.setAllowUnsourced(false)
        let r = seen.snapshot
        XCTAssertEqual(r.ran, 2)
        XCTAssertEqual(r.violations, [])
    }

    /// Atomarer Start (Runde 5, H4): Prüfung UND Start laufen als `start(generation, body)` in einem Lock. Nach der Rückkehr von
    /// `bump` läuft nebenläufig KEIN Start-Body der alten Generation mehr. Der Body liest das Flag „bump zurückgekehrt“ selbst, im
    /// Lock: War es gesetzt, hätte der Body nach der Erhöhung gestartet.
    func testNoStartBodyRunsAfterABumpToANewerGenerationHasReturnedUnderContention() async {
        let s = await session()
        let gate = s.playbackGate
        let tally = Tally()
        let stop = Tally()
        let group = DispatchGroup()
        for _ in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                while stop.snapshot.ran == 0 {
                    let started = gate.start(0) {
                        if tally.returnedGeneration > 0 { tally.violation("Start-Body der Generation 0 nach der Rückkehr von bump") }
                        tally.ranOnce()
                    }
                    if !started { tally.refusedOnce(); break }
                }
                group.leave()
            }
        }
        pause(0.05)
        let g = await s.releaseDidChange().generation
        tally.returned(g)
        XCTAssertEqual(group.wait(timeout: .now() + 10), .success)
        stop.ranOnce()
        let r = tally.snapshot
        XCTAssertGreaterThan(r.ran, 0, "die Starter liefen vor dem Widerruf")
        XCTAssertEqual(r.refused, 4, "jeder Starter wurde danach abgewiesen")
        XCTAssertEqual(r.violations, [], "nach der Rückkehr von bump startet nichts Altes mehr")
    }

    /// Dasselbe mit vielen Erhöhungen und Starts der jeweils aktuellen Generation (Stresstest).
    func testNoStartBodyRunsAfterABumpToANewerGenerationHasReturnedStress() async {
        let s = await session()
        let gate = s.playbackGate
        let tally = Tally()
        let stop = Tally()
        let group = DispatchGroup()
        for _ in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                while stop.snapshot.ran == 0 {
                    let g = s.releaseGeneration
                    let started = gate.start(g) {
                        let returned = tally.returnedGeneration
                        if returned > g { tally.violation("Start-Body der Generation \(g) nach Rückkehr von \(returned)") }
                        tally.ranOnce()
                    }
                    if !started { tally.refusedOnce() }
                }
                group.leave()
            }
        }
        for _ in 0..<1_500 {
            let g = await s.releaseDidChange().generation
            tally.returned(g)
            if g % 50 == 0 { pause(0.0005) }
        }
        stop.ranOnce()
        XCTAssertEqual(group.wait(timeout: .now() + 20), .success)
        let r = tally.snapshot
        XCTAssertEqual(r.violations, [], "kein Start-Body nach der Rückkehr einer neueren Generation")
        XCTAssertGreaterThan(r.ran, 0, "die Starter liefen")
        XCTAssertGreaterThan(r.refused, 0, "und wurden zwischendurch abgewiesen")
        XCTAssertEqual(s.releaseGeneration, 1_500)
    }

    func testStartRunsTheBodyOnlyWhileTheGenerationIsNotDiscardedAndAnEarlyPermitDoesNotCarryOver() async {
        let s = await session()
        let gate = s.playbackGate
        let g = s.releaseGeneration
        var ran = 0
        XCTAssertTrue(gate.start(g) { ran += 1 })
        XCTAssertTrue(gate.start(g + 3) { ran += 1 }, "künftig ist nicht verworfen")
        XCTAssertEqual(ran, 2)
        XCTAssertTrue(gate.permits(g), "die Vorprüfung sagt: darf")
        await s.releaseDidChange()
        XCTAssertFalse(gate.start(g) { ran += 1 }, "die Vorprüfung gilt nicht für den Start: nach der Erhöhung wird abgewiesen")
        XCTAssertEqual(ran, 2, "der Body lief nicht")
        XCTAssertTrue(gate.start(g + 1) { ran += 1 })
        XCTAssertEqual(ran, 3)
    }

    /// Haltepunkt ZWISCHEN Prüfung und Start. Gibt zurück, ob der Start-Body NACH der Rückkehr der Erhöhung lief.
    /// `oldOrder`: die alte Reihenfolge (`permits`, danach Start außerhalb des Locks); sonst `start(generation, body)`.
    private func startRanAfterTheBumpReturned(oldOrder: Bool, session s: AssistantSession) -> (late: Bool, bumpWaitedForTheBody: Bool) {
        let gate = s.playbackGate
        let g = s.releaseGeneration
        let tally = Tally()
        let bodyDone = Tally()
        let atBarrier = DispatchSemaphore(value: 0)
        let bumpMayProceed = DispatchSemaphore(value: 0)
        let bumpDone = DispatchSemaphore(value: 0)
        let waited = Tally()
        DispatchQueue.global().async {
            atBarrier.wait()                // der Starter steht zwischen Prüfung und Start
            bumpMayProceed.signal()         // Hinweis an den Körper: jetzt wird erhöht
            let newer = s.applyReleaseChange().generation
            tally.returned(newer)
            if bodyDone.snapshot.ran > 0 { waited.ranOnce() }   // die Erhöhung kam erst nach dem Body zurück
            bumpDone.signal()
        }
        let body = {
            if tally.returnedGeneration > g { tally.violation("Start-Body nach der Rückkehr der Erhöhung") }
            bodyDone.ranOnce()
        }
        if oldOrder {
            if gate.permits(g) {            // Prüfung ...
                atBarrier.signal()          // ... Haltepunkt: die Erhöhung läuft (kein Lock gehalten) und kehrt zurück ...
                bumpDone.wait()
                body()                      // ... erst dann der Start
            }
        } else {
            _ = gate.start(g) {
                atBarrier.signal()          // Haltepunkt im Start: die Erhöhung steht vor dem Lock und muss warten
                _ = bumpMayProceed.wait(timeout: .now() + 0.2)
                pause(0.1)
                body()
            }
            bumpDone.wait()
        }
        return (!tally.snapshot.violations.isEmpty, waited.snapshot.ran > 0)
    }

    /// Mit der alten Reihenfolge ist der Haltepunkt sicher rot (der Start läuft nach der Rückkehr der Erhöhung) ...
    func testBarrierBetweenCheckAndStartIsRedWithTheOldOrder() async {
        let s = await session()
        let r = startRanAfterTheBumpReturned(oldOrder: true, session: s)
        XCTAssertTrue(r.late, "die alte Reihenfolge (permits, dann Start außerhalb des Locks) startet nach dem Widerruf")
        XCTAssertFalse(r.bumpWaitedForTheBody)
    }

    /// ... und mit `start(generation, body)` grün: die Erhöhung wartet auf den Body, der Start liegt vor ihr.
    func testBarrierBetweenCheckAndStartIsGreenWithStart() async {
        let s = await session()
        let r = startRanAfterTheBumpReturned(oldOrder: false, session: s)
        XCTAssertFalse(r.late, "kein Start-Body nach der Rückkehr der Erhöhung")
        XCTAssertTrue(r.bumpWaitedForTheBody, "die Erhöhung kehrte erst nach dem Body zurück")
        XCTAssertEqual(s.playbackGate.discardedBelow, s.releaseGeneration)
    }

    // MARK: Ausgabe-Commit im Lock

    func testCommitOutputRunsTheBodyOnlyForTheCurrentGeneration() async {
        let s = await session()
        s.admitForTesting(requestId: "r1")
        let ran = Tally()
        let g = s.releaseGeneration
        XCTAssertTrue(s.commitOutput(requestId: "r1", index: 0, generation: g, stage: .display) { ran.ranOnce() }.ok)
        await s.releaseDidChange()
        XCTAssertFalse(s.commitOutput(requestId: "r1", index: 0, generation: g, stage: .display) { ran.ranOnce() }.ok, "veraltet")
        XCTAssertFalse(s.commitOutput(requestId: "r1", index: 0, generation: g + 5, stage: .audio) { ran.ranOnce() }.ok, "künftig passt nicht")
        XCTAssertTrue(s.commitOutput(requestId: "r1", index: 0, generation: g + 1, stage: .audio) { ran.ranOnce() }.ok)
        XCTAssertEqual(ran.snapshot.ran, 2)
    }

    /// Der Body läuft im Lock der Freigabe: `bump` aus einem anderen Thread wartet, bis der Body fertig ist. (Die Sperre teilt
    /// diesen Lock: Im Body darf nichts in Freigabe oder Sperre zurückrufen, es hinge; geprüft wird nach dem Body.)
    func testBodyRunsInsideTheLockOfTheReleaseAndABumpWaitsForIt() async {
        let s = await session()
        s.admitForTesting(requestId: "r1")
        let gate = s.playbackGate
        let g = s.releaseGeneration
        let bumped = Tally()
        let entered = DispatchSemaphore(value: 0)
        let observed = Tally()
        let ok = s.commitOutput(requestId: "r1", index: 0, generation: g, stage: .display) {
            DispatchQueue.global().async {
                entered.signal()
                _ = s.applyReleaseChange()
                bumped.ranOnce()
            }
            entered.wait()
            pause(0.1)   // der Auslöser steht jetzt vor dem Lock
            if bumped.snapshot.ran != 0 { observed.violation("bump lief mitten im Body") }
        }
        XCTAssertTrue(ok.ok)
        let deadline = Date().addingTimeInterval(5)
        while bumped.snapshot.ran == 0, Date() < deadline { pause(0.01) }
        XCTAssertEqual(bumped.snapshot.ran, 1, "der Widerruf kam nach dem Body")
        XCTAssertEqual(observed.snapshot.violations, [])
        XCTAssertEqual(gate.discardedBelow, g + 1)
    }

    /// Stresstest: Der Body läuft nie nach der Rückkehr einer Erhöhung auf eine NEUERE Generation.
    func testBodyNeverRunsAfterABumpToANewerGenerationHasReturned() async {
        let s = await session()
        for t in 0..<4 { s.admitForTesting(requestId: "r\(t)") }
        let tally = Tally()
        let stop = Tally()
        let group = DispatchGroup()
        for t in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                while stop.snapshot.ran == 0 {
                    let g = s.releaseGeneration
                    let ok = s.commitOutput(requestId: "r\(t)", index: 0, generation: g, stage: t % 2 == 0 ? .display : .audio) {
                        let returned = tally.returnedGeneration
                        if returned > g { tally.violation("Body der Generation \(g) nach Rückkehr von \(returned)") }
                        tally.ranOnce()
                    }
                    if !ok.ok { tally.refusedOnce() }
                }
                group.leave()
            }
        }
        for _ in 0..<1_500 {
            let g = await s.releaseDidChange().generation
            tally.returned(g)
            if g % 50 == 0 { pause(0.0005) }
        }
        stop.ranOnce()
        XCTAssertEqual(group.wait(timeout: .now() + 20), .success)
        let r = tally.snapshot
        XCTAssertEqual(r.violations, [], "kein Body nach einer neueren Generation")
        XCTAssertGreaterThan(r.ran, 0, "die Erzeuger liefen")
        XCTAssertGreaterThan(r.refused, 0, "und wurden zwischendurch widerrufen")
        XCTAssertEqual(s.releaseGeneration, 1_500)
    }

    /// Anzeige und Stimme melden ihre Commits mit der geprüften Generation: Annahme mit der des Satzes, Ablehnung mit der
    /// aktuellen; `reportPass: false` meldet nur Ablehnungen.
    func testCommitOutputWritesTheMarksWithTheCheckedGeneration() async {
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(audit: audit, pieces: ["Quasselbert wandert nach Pappelhausen."])
        let g = s.releaseGeneration
        var spoken: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        XCTAssertTrue(s.commitOutput(requestId: "r1", index: 1, generation: g, stage: .display) {}.ok)
        XCTAssertTrue(s.commitOutput(requestId: "r1", index: 1, generation: g, stage: .audio, reportPass: false) {}.ok)
        await s.releaseDidChange()
        XCTAssertFalse(s.commitOutput(requestId: "r1", index: 1, generation: g, stage: .audio, reportPass: false) {}.ok)
        XCTAssertFalse(s.commitOutput(requestId: "r1", index: 99, generation: g, stage: .display) {}.ok, "unbekannter Index: nur abgelehnt")
        await s.commit(requestId: "r1", user: "x", spoken: spoken.joined(separator: " "), interrupted: false, revoked: true)
        let e = await audit.entries().filter { $0.kind == .answer }.first
        XCTAssertEqual(e?.trace[1].commits, [
            AssistantCommitMark(stage: .emit, ok: true, generation: g),
            AssistantCommitMark(stage: .display, ok: true, generation: g),
            AssistantCommitMark(stage: .audio, ok: false, generation: g + 1),
        ])
    }
}
