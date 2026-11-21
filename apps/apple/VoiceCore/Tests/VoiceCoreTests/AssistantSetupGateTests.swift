import XCTest
@testable import VoiceCore

// lva#565: Nach einem Wechsel des Assistenten darf keine Anfrage mit der alten Sitzung (Rolle, Wissensbereiche) an das Modell gehen.
// Geprüft wird die Schranke (`AssistantSetupGate`) mit einer nachgestellten Steuerung: Haltepunkte sind Fortsetzungen (`Latch`), die der
// Test öffnet; gewartet wird auf das Ende des Tauschs (`idle()`), nie auf eine Zeit.

/// Haltepunkt: `wait()` kehrt zurück, sobald `open()` gerufen wurde (davor oder danach). Die Frist von 30 s ist nur ein Netz gegen einen
/// hängenden Testlauf (ein Haltepunkt, der nie erreicht wird, soll als Fehlschlag enden, nicht als Hänger); der Ablauf hängt nie von ihr ab.
@MainActor
private final class Latch {
    private var continuation: CheckedContinuation<Void, Never>?
    private var opened = false
    private var timedOut = false

    func wait(file: StaticString = #filePath, line: UInt = #line) async {
        if opened { return }
        let guardTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard !Task.isCancelled, let self, !self.opened else { return }
            self.timedOut = true
            self.open()
        }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in self.continuation = continuation }
        guardTask.cancel()
        if timedOut { XCTFail("Haltepunkt nie erreicht", file: file, line: line) }
    }

    func open() {
        opened = true
        continuation?.resume()
        continuation = nil
    }
}

private struct Sent: Equatable {
    var session: Int
    var setup: String
    var texts: [String]
}

/// Die Steuerung in klein: eine Sitzung mit Nummer und Assistent, ein gewünschter Assistent, eine laufende Anfrage. `swap` entscheidet wie
/// die echte Steuerung über `AssistantSwapPlan` und liest den gewünschten Stand NACH seinem Haltepunkt.
@MainActor
private final class World {
    var applied = "A"
    var wanted = "A"
    var session = 1
    var answerRunning = false
    var consentOpen = false
    var swapCalls = 0
    var sent: [Sent] = []
    var overflow: [String] = []
    var settledCount = 0
    /// Gesendet, während die Sitzung nicht zum gewünschten Stand passte (darf nie vorkommen).
    var sentWhileStale = 0
    /// Haltepunkt VOR der Entscheidung (einmalig): `arrivedBefore` meldet das Ankommen, der Tausch wartet auf `resumeBefore`.
    var arrivedBefore = Latch()
    var resumeBefore: Latch?
    /// Haltepunkt NACH der Entscheidung, vor der Rückgabe (einmalig).
    var arrivedAfter = Latch()
    var resumeAfter: Latch?
    /// Vorgegebene Ausgänge (zuerst abgearbeitet), danach entscheidet `AssistantSwapPlan`.
    var scripted: [AssistantSwapOutcome] = []
    /// Nach dem ersten gelungenen Einsetzen: Der Nutzer wechselt noch einmal, bevor die Schranke es bemerkt.
    var changeAfterInstall: String?
    var onDeliver: ((World) -> Void)?
    private(set) lazy var gate = AssistantSetupGate<String>(
        isStale: { [unowned self] in self.applied != self.wanted },
        swap: { [unowned self] in await self.swap() },
        onOverflow: { [unowned self] in self.overflow.append($0) },
        deliver: { [unowned self] texts in
            if self.applied != self.wanted { self.sentWhileStale += 1 }
            self.sent.append(Sent(session: self.session, setup: self.applied, texts: texts))
            self.onDeliver?(self)
        },
        settled: { [unowned self] in self.settledCount += 1 })

    private func swap() async -> AssistantSwapOutcome {
        swapCalls += 1
        if let resume = resumeBefore {
            resumeBefore = nil
            arrivedBefore.open()
            await resume.wait()
        }
        var outcome: AssistantSwapOutcome
        if !scripted.isEmpty {
            outcome = scripted.removeFirst()
        } else {
            switch AssistantSwapPlan.decide(.init(sessionUnchanged: true, chainUnchanged: true, clearSettled: true,
                                                  answerRunning: answerRunning, jobRunning: false, consentOpen: consentOpen)) {
            case .install:
                // Der gewünschte Stand wird erst jetzt gelesen, nach allen Wartepunkten, und im selben Zug eingesetzt.
                applied = wanted
                session += 1
                outcome = .installed
                if let next = changeAfterInstall {
                    changeAfterInstall = nil
                    wanted = next
                }
            case .busy: outcome = .busy
            case .retryNow: outcome = .retryNow
            }
        }
        if let resume = resumeAfter {
            resumeAfter = nil
            arrivedAfter.open()
            await resume.wait()
        }
        return outcome
    }
}

@MainActor
final class AssistantSetupGateTests: XCTestCase {
    // MARK: Begrenztes Zurückhalten (lva#748)

    // Gegenprobe: Ohne Anzahlprüfung wächst held; mit <= statt < wird die neunte Eingabe angenommen.
    // Mit einer um eins kleineren Grenze wird bereits die achte Eingabe abgelehnt.
    func testHeldCountAcceptsExactLimitAndReportsEveryOverflowWithoutChangingInputs() async {
        let world = World()
        world.wanted = "B"
        world.answerRunning = true
        let texts = (0..<AssistantSetupGate<String>.maxHeldCount).map { "Frage \($0)" }
        for text in texts { XCTAssertEqual(world.gate.submit(text), .held) }
        await world.gate.idle()
        let bytes = world.gate.heldByteCount
        for _ in 0..<3 {
            XCTAssertEqual(world.gate.submit("Überlauf"), .rejectedOverflow)
        }
        XCTAssertEqual(world.overflow, Array(repeating: "Überlauf", count: 3))
        XCTAssertEqual(world.gate.heldInputs, texts)
        XCTAssertEqual(world.gate.heldByteCount, bytes)
        XCTAssertEqual(world.sent, [])

        world.answerRunning = false
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent.map(\.texts), [texts])
        XCTAssertEqual(world.gate.heldCount, 0)
        XCTAssertEqual(world.gate.heldByteCount, 0)
    }

    // Gegenprobe: String.count statt utf8.count nimmt die zusätzliche Eingabe an;
    // < statt <= bei der Byteprüfung lehnt die Eingabe genau auf der Grenze ab.
    func testHeldUTF8BytesAcceptExactTotalLimitAndPreserveInputsOnOverflow() async {
        let world = World()
        world.wanted = "B"
        world.answerRunning = true
        let limit = AssistantSetupGate<String>.maxHeldBytes
        let first = String(repeating: "ä", count: (limit - 4) / 2)
        XCTAssertEqual(world.gate.hold(first), .held)
        XCTAssertEqual(world.gate.submit("🙂"), .held)
        await world.gate.idle()
        XCTAssertEqual(world.gate.heldByteCount, limit)
        XCTAssertEqual(world.gate.submit("a"), .rejectedOverflow)
        XCTAssertEqual(world.overflow, ["a"])
        XCTAssertEqual(world.gate.heldInputs, [first, "🙂"])
        XCTAssertEqual(world.gate.heldByteCount, limit)
        world.answerRunning = false
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent.map(\.texts), [[first, "🙂"]])
    }

    // Gegenprobe: Fehlende Größenmessung für AssistantHeldRequest nimmt den zu großen Rohtext an;
    // Trimmen vor der Messung nimmt den Rohtext mit zusätzlichem Leerzeichen an.
    func testHeldRequestsRejectOversizedRawTextAndKeepCombinedTextBounded() async {
        var stale = true
        var sent: [[AssistantHeldRequest]] = []
        var overflow: [AssistantHeldRequest] = []
        let gate = AssistantSetupGate<AssistantHeldRequest>(
            isStale: { stale },
            swap: { .unavailable },
            onOverflow: { overflow.append($0) },
            deliver: { sent.append($0) })
        let limit = AssistantSetupGate<AssistantHeldRequest>.maxHeldBytes
        let exact = AssistantHeldRequest(
            text: String(repeating: "🙂", count: limit / 4),
            afterInterruption: true, endedAtMs: 100)
        var tooLarge = exact
        tooLarge.text += " "
        XCTAssertEqual(gate.hold(tooLarge), .rejectedOverflow)
        XCTAssertEqual(gate.heldCount, 0)
        XCTAssertEqual(gate.heldByteCount, 0)
        XCTAssertEqual(gate.hold(exact), .held)
        for _ in 0..<3 {
            XCTAssertEqual(gate.hold(tooLarge), .rejectedOverflow)
            XCTAssertEqual(AssistantHeldRequest.combined(gate.heldInputs), exact)
        }
        XCTAssertEqual(overflow, Array(repeating: tooLarge, count: 4))
        XCTAssertEqual(gate.heldInputs, [exact])
        await gate.idle()
        stale = false
        gate.kick()
        await gate.idle()
        XCTAssertEqual(sent, [[exact]])
        XCTAssertEqual(gate.heldByteCount, 0)
    }

    // Gegenprobe: Ohne Zurücksetzen der Bytezahl in dropHeld wird die neue volle Warteschlange abgelehnt.
    // Ohne Leeren von held ist die Anzahlgrenze nach dem Abbruch weiter belegt.
    func testDropHeldResetsBothLimitsForNextSwitch() async {
        let world = World()
        world.wanted = "B"
        world.answerRunning = true
        let count = AssistantSetupGate<String>.maxHeldCount
        let limit = AssistantSetupGate<String>.maxHeldBytes
        let texts = [String(repeating: "a", count: limit - count + 1)]
            + Array(repeating: "b", count: count - 1)
        for text in texts { XCTAssertEqual(world.gate.hold(text), .held) }
        await world.gate.idle()
        XCTAssertEqual(world.gate.heldCount, count)
        XCTAssertEqual(world.gate.heldByteCount, limit)
        XCTAssertEqual(world.gate.dropHeld(), texts)
        XCTAssertEqual(world.gate.heldCount, 0)
        XCTAssertEqual(world.gate.heldByteCount, 0)
        world.wanted = "C"
        for text in texts { XCTAssertEqual(world.gate.hold(text), .held) }
        world.answerRunning = false
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "C", texts: texts)])
        XCTAssertEqual(world.overflow, [])
    }

    // Gegenprobe: Ohne Zurücksetzen vor deliver wird die zweite volle Eingabe beim Wiedereintritt
    // abgelehnt. Zurücksetzen erst nach deliver beschädigt die Bytezahl der neu gehaltenen Eingabe.
    func testDeliveryResetsBytesBeforeReentrantSwitchAndHold() async {
        let world = World()
        world.wanted = "B"
        let text = String(repeating: "a", count: AssistantSetupGate<String>.maxHeldBytes)
        world.onDeliver = { world in
            world.onDeliver = nil
            XCTAssertEqual(world.gate.heldCount, 0)
            XCTAssertEqual(world.gate.heldByteCount, 0)
            world.wanted = "C"
            XCTAssertEqual(world.gate.submit(text), .held)
            XCTAssertEqual(world.gate.heldByteCount, text.utf8.count)
        }
        XCTAssertEqual(world.gate.submit(text), .held)
        await world.gate.idle()
        XCTAssertEqual(world.sent.map(\.texts), [[text], [text]])
        XCTAssertEqual(world.gate.heldCount, 0)
        XCTAssertEqual(world.gate.heldByteCount, 0)
        XCTAssertEqual(world.overflow, [])
    }

    // MARK: (a) Wechsel während einer laufenden Anfrage

    func testSwitchDuringRunningRequestNextRequestUsesNewProfile() async {
        let world = World()
        world.answerRunning = true          // eine Anfrage des alten Assistenten läuft
        world.wanted = "B"                  // der Nutzer wechselt den Assistenten
        world.gate.kick()
        await world.gate.idle()
        XCTAssertTrue(world.gate.isParked, "mit laufender Anfrage wartet der Tausch auf deren Ende")
        XCTAssertEqual(world.applied, "A")
        XCTAssertEqual(world.swapCalls, 1)

        world.gate.submit("Frage")          // die nächste Anfrage: darf nicht mit der alten Sitzung laufen
        await world.gate.idle()
        XCTAssertEqual(world.sent, [], "nichts geht an die alte Sitzung")
        XCTAssertEqual(world.gate.heldCount, 1, "die Eingabe wird nicht verworfen, sondern gehalten")

        world.answerRunning = false         // die laufende Anfrage endet: Anstoß der Steuerung
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["Frage"])], "gesendet mit der neuen Sitzung, genau einmal")
        XCTAssertEqual(world.gate.heldCount, 0)
        XCTAssertEqual(world.sentWhileStale, 0)
    }

    func testWithoutSwitchSessionStaysTheSame() async {
        let world = World()
        world.answerRunning = true
        world.gate.submit("Frage")
        XCTAssertEqual(world.sent, [Sent(session: 1, setup: "A", texts: ["Frage"])], "ohne Wechsel sofort, mit der bestehenden Sitzung")
        world.answerRunning = false
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.session, 1, "kein Tausch ohne Wechsel")
        XCTAssertEqual(world.swapCalls, 0)
        XCTAssertFalse(world.gate.isHolding)
    }

    // MARK: (b) Wechsel direkt vor dem Senden

    func testSwitchRightBeforeSendingRequestWaitsAndUsesNewProfile() async {
        let world = World()
        world.wanted = "B"                  // gewechselt, die Sitzung ist noch die alte, kein Anstoß war schon da
        let resume = Latch()
        world.resumeBefore = resume
        world.gate.submit("Frage")
        XCTAssertEqual(world.sent, [], "die Anfrage wartet, statt mit der alten Sitzung zu laufen")
        await world.arrivedBefore.wait()    // der Tausch steht mitten im Aufbau
        world.gate.submit("Nachtrag")       // eine weitere Eingabe in der Wartezeit
        XCTAssertEqual(world.sent, [], "auch während des Tauschs geht nichts an die alte Sitzung")
        XCTAssertEqual(world.gate.heldInputs, ["Frage", "Nachtrag"], "Reihenfolge bleibt")
        resume.open()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["Frage", "Nachtrag"])], "eine Lieferung, neue Sitzung")
        XCTAssertEqual(world.settledCount, 1)
        XCTAssertEqual(world.sentWhileStale, 0)
    }

    func testWithoutSwitchRequestIsSentImmediately() async {
        let world = World()
        world.gate.submit("Frage")
        XCTAssertEqual(world.sent, [Sent(session: 1, setup: "A", texts: ["Frage"])], "synchron, ohne Warten")
        await world.gate.idle()
        XCTAssertEqual(world.swapCalls, 0)
        XCTAssertEqual(world.settledCount, 0)
    }

    // MARK: Nebenläufigkeit

    /// Wechselt der Nutzer noch einmal, während der Tausch läuft, gilt der letzte Stand: Die neue Sitzung wird nicht für den
    /// vorletzten gebaut.
    func testSwitchAgainDuringSwapInstallsLatestProfile() async {
        let world = World()
        world.wanted = "B"
        let resume = Latch()
        world.resumeBefore = resume
        world.gate.submit("Frage")
        await world.arrivedBefore.wait()
        world.wanted = "C"
        resume.open()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "C", texts: ["Frage"])])
        XCTAssertEqual(world.session, 2, "eine einzige neue Sitzung")
    }

    /// Wechselt der Nutzer in dem Augenblick zwischen Einsetzen und Rückkehr, sieht die Schranke die neue Abweichung und tauscht noch
    /// einmal, statt mit der halb überholten Sitzung zu senden.
    func testChangeBetweenInstallAndReturnSwapsAgainBeforeSending() async {
        let world = World()
        world.wanted = "B"
        world.changeAfterInstall = "C"
        world.gate.submit("Frage")
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 3, setup: "C", texts: ["Frage"])])
        XCTAssertEqual(world.sentWhileStale, 0)
        XCTAssertEqual(world.swapCalls, 2)
    }

    /// Verlorenes Aufwecken: Der Versuch hat „beschäftigt“ entschieden, die Anfrage endet und stößt an, bevor der Versuch zurückkehrt.
    /// Die Schranke muss noch einmal ansehen, nicht warten, bis ein Anstoß kommt, der nie mehr kommt.
    func testKickDuringDecisionIsNotLost() async {
        let world = World()
        world.answerRunning = true
        world.wanted = "B"
        let resume = Latch()
        world.resumeAfter = resume          // nach der Entscheidung „beschäftigt“, vor der Rückgabe
        world.gate.submit("Frage")
        await world.arrivedAfter.wait()
        world.answerRunning = false         // die Anfrage endet jetzt, der Anstoß trifft den laufenden Versuch
        world.gate.kick()
        resume.open()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["Frage"])], "kein Verhungern des Tauschs")
        XCTAssertFalse(world.gate.isParked)
    }

    /// Eine beschäftigte Sitzung ohne Anstoß wartet, ohne sich im Kreis zu drehen (keine Versuchsschleife).
    func testBusyDoesNotSpin() async {
        let world = World()
        world.answerRunning = true
        world.wanted = "B"
        world.gate.kick()
        await world.gate.idle()
        for _ in 0..<20 { await Task.yield() }
        await world.gate.idle()
        XCTAssertEqual(world.swapCalls, 1, "genau ein Versuch, dann Warten auf das Ende der Anfrage")
        XCTAssertTrue(world.gate.isParked)
    }

    func testOpenConsentQuestionBlocksSwapUntilAnswered() async {
        let world = World()
        world.consentOpen = true
        world.wanted = "B"
        world.gate.submit("Frage")
        await world.gate.idle()
        XCTAssertEqual(world.sent, [])
        XCTAssertTrue(world.gate.isParked)
        world.consentOpen = false
        world.gate.kick()                   // `finishConsent`
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["Frage"])])
    }

    // MARK: Abbruch mitten im Vorgang

    /// Stopp oder „Neues Gespräch“ verwirft die gehaltene Eingabe ausdrücklich; der Tausch hört deshalb nicht auf (die Sitzung muss zum
    /// Assistenten passen, auch wenn keine Eingabe wartet).
    func testDropHeldDiscardsInputButSwapStillHappens() async {
        let world = World()
        world.answerRunning = true
        world.wanted = "B"
        world.gate.submit("Frage")
        await world.gate.idle()
        XCTAssertEqual(world.gate.dropHeld(), ["Frage"])
        XCTAssertEqual(world.gate.heldCount, 0)
        world.answerRunning = false
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [], "verworfen bleibt verworfen")
        XCTAssertEqual(world.applied, "B", "der Tausch fand trotzdem statt")
        XCTAssertEqual(world.settledCount, 1)
    }

    func testDroppedInputDoesNotBlockLaterInput() async {
        let world = World()
        world.answerRunning = true
        world.wanted = "B"
        world.gate.submit("alt")
        world.gate.dropHeld()
        world.answerRunning = false
        world.gate.submit("neu")
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["neu"])])
    }

    // MARK: Versuchsausgänge

    func testRetryNowLoopsUntilInstalled() async {
        let world = World()
        world.wanted = "B"
        world.scripted = [.retryNow, .retryNow]
        world.gate.submit("Frage")
        await world.gate.idle()
        XCTAssertEqual(world.swapCalls, 3)
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["Frage"])])
    }

    /// Fehlendes Modell: nichts, womit sich tauschen ließe. Die Eingabe bleibt liegen (nie mit der alten Sitzung, nie still weg) und geht,
    /// sobald ein Tausch möglich ist.
    func testUnavailableKeepsInputUntilSwapPossible() async {
        let world = World()
        world.wanted = "B"
        world.scripted = [.unavailable]
        world.gate.submit("Frage")
        await world.gate.idle()
        XCTAssertEqual(world.sent, [])
        XCTAssertEqual(world.gate.heldCount, 1)
        XCTAssertTrue(world.gate.isParked)
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["Frage"])])
    }

    /// Ein Tausch, der „gelungen“ meldet und die Sitzung überholt lässt, ist ein Fehler der Steuerung: Die Schranke gibt nach wenigen
    /// Wiederholungen auf zu drehen und sendet NICHT.
    func testInstalledButStillStaleParksInsteadOfLoopingOrSending() async {
        let world = World()
        world.wanted = "B"
        world.scripted = Array(repeating: .installed, count: 50)
        world.gate.submit("Frage")
        await world.gate.idle()
        XCTAssertEqual(world.swapCalls, 3)
        XCTAssertEqual(world.sent, [])
        XCTAssertEqual(world.gate.heldCount, 1)
        XCTAssertTrue(world.gate.isParked)
    }

    // MARK: Wiedereintritt

    /// Die Zustimmungsabfrage sendet nach der Zusage erneut (`send(text:)` aus dem Senden heraus): Die zweite Eingabe darf nicht liegen
    /// bleiben, und die erste nicht doppelt gehen.
    func testSubmitFromInsideDeliverIsNotLost() async {
        let world = World()
        world.wanted = "B"
        world.onDeliver = { world in
            world.onDeliver = nil
            world.gate.submit("zweite")
        }
        world.gate.submit("erste")
        await world.gate.idle()
        XCTAssertEqual(world.sent.map(\.texts), [["erste"], ["zweite"]])
        XCTAssertEqual(world.sentWhileStale, 0)
    }

    /// Wechselt beim Senden schon wieder etwas (`deliver` ändert den Wunsch) und kommt dabei eine Eingabe, wartet sie auf einen
    /// weiteren Tausch.
    func testStaleAgainInsideDeliverWaitsForNextSwap() async {
        let world = World()
        world.wanted = "B"
        world.onDeliver = { world in
            world.onDeliver = nil
            world.wanted = "C"
            world.gate.submit("zweite")
        }
        world.gate.submit("erste")
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 2, setup: "B", texts: ["erste"]), Sent(session: 3, setup: "C", texts: ["zweite"])])
        XCTAssertEqual(world.sentWhileStale, 0)
    }

    func testKickWithoutStaleAndWithoutInputDoesNothing() async {
        let world = World()
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.swapCalls, 0)
        XCTAssertEqual(world.settledCount, 0)
    }

    /// Steht die Eingabe nur noch aus, weil ein Modellwechsel die Sitzung schon passend gebaut hat (nicht überholt, aber gehalten),
    /// sendet der nächste Anstoß sie.
    func testKickFlushesInputWhenSessionAlreadyMatches() async {
        let world = World()
        world.answerRunning = true
        world.wanted = "B"
        world.gate.submit("Frage")
        await world.gate.idle()
        world.applied = "B"                 // ein anderer Weg (Modellwechsel: `adopt`) hat die Sitzung passend gebaut
        world.gate.kick()
        await world.gate.idle()
        XCTAssertEqual(world.sent, [Sent(session: 1, setup: "B", texts: ["Frage"])])
    }

    // MARK: Zufallsfolge (deterministisch), Invarianten

    /// Beliebige Folgen aus Eingaben, Wechseln, Ende der Anfrage und Anstößen. Immer: nie mit überholter Sitzung gesendet; am Ende ist
    /// jede angenommene, nicht ausdrücklich verworfene Eingabe genau einmal und in der Reihenfolge
    /// gesendet; der Tausch ist nie hängen geblieben, sobald nichts mehr läuft.
    func testRandomInterleavingsKeepInvariants() async {
        for seed in 1...40 {
            var rng = SplitMix64(seed: UInt64(seed))
            let world = World()
            var expected: [String] = []
            var counter = 0
            for _ in 0..<120 {
                switch rng.next() % 7 {
                case 0, 1:
                    counter += 1
                    let text = "e\(counter)"
                    // Gegenprobe: Auch abgelehnte Eingaben zu erwarten macht diese Invariante rot.
                    if world.gate.submit(text) != .rejectedOverflow { expected.append(text) }
                case 2: world.wanted = ["A", "B", "C"][Int(rng.next() % 3)]
                case 3: world.answerRunning = rng.next() % 2 == 0
                case 4: world.gate.kick()
                case 5: world.consentOpen = rng.next() % 3 == 0
                default:
                    if rng.next() % 6 == 0, !world.gate.heldInputs.isEmpty {
                        let dropped = world.gate.dropHeld()
                        expected.removeAll { dropped.contains($0) }
                    }
                }
                if rng.next() % 2 == 0 { await Task.yield() }
            }
            world.answerRunning = false
            world.consentOpen = false
            world.gate.kick()
            await world.gate.idle()
            XCTAssertEqual(world.sentWhileStale, 0, "Seed \(seed): mit überholter Sitzung gesendet")
            XCTAssertEqual(world.sent.flatMap(\.texts), expected, "Seed \(seed): Eingaben verloren, doppelt oder vertauscht")
            XCTAssertEqual(world.gate.heldCount, 0, "Seed \(seed)")
            XCTAssertEqual(world.applied, world.wanted, "Seed \(seed): Tausch hängt")
            XCTAssertFalse(world.gate.isHolding, "Seed \(seed)")
        }
    }

    // MARK: Reine Typen

    func testCombinedKeepsOrderTrimsAndFlagsInterruption() {
        let a = AssistantHeldRequest(text: " Wie wird das Wetter? ", afterInterruption: false, endedAtMs: 100)
        let b = AssistantHeldRequest(text: "und morgen", afterInterruption: true, endedAtMs: 250)
        XCTAssertEqual(AssistantHeldRequest.combined([a, b]),
                       AssistantHeldRequest(text: "Wie wird das Wetter? und morgen", afterInterruption: true, endedAtMs: 250))
        XCTAssertEqual(AssistantHeldRequest.combined([a]), AssistantHeldRequest(text: "Wie wird das Wetter?", afterInterruption: false, endedAtMs: 100))
        XCTAssertNil(AssistantHeldRequest.combined([]))
        let blank = AssistantHeldRequest(text: "  ", afterInterruption: false, endedAtMs: 5)
        XCTAssertEqual(AssistantHeldRequest.combined([blank, b])?.text, "und morgen", "leere Eingaben fügen nichts ein")
    }

    func testSwapPlanDecisions() {
        func snap(session: Bool = true, chain: Bool = true, clear: Bool = true, answer: Bool = false, job: Bool = false,
                  consent: Bool = false) -> AssistantSwapPlan.Snapshot {
            .init(sessionUnchanged: session, chainUnchanged: chain, clearSettled: clear, answerRunning: answer, jobRunning: job,
                  consentOpen: consent)
        }
        XCTAssertEqual(AssistantSwapPlan.decide(snap()), .install)
        XCTAssertEqual(AssistantSwapPlan.decide(snap(answer: true)), .busy)
        XCTAssertEqual(AssistantSwapPlan.decide(snap(job: true)), .busy)
        XCTAssertEqual(AssistantSwapPlan.decide(snap(consent: true)), .busy)
        XCTAssertEqual(AssistantSwapPlan.decide(snap(session: false)), .retryNow, "Sitzung ersetzt: neu ansetzen")
        XCTAssertEqual(AssistantSwapPlan.decide(snap(chain: false)), .retryNow, "neuer Auftrag in der Kette: neu ansetzen")
        XCTAssertEqual(AssistantSwapPlan.decide(snap(clear: false)), .retryNow, "„Neues Gespräch“ steht aus: der alte Verlauf bliebe")
        XCTAssertEqual(AssistantSwapPlan.decide(snap(session: false, answer: true)), .retryNow, "verschobener Zustand geht vor „beschäftigt“")
    }
}

final class AssistantSetupGateGrenzwerteTests: XCTestCase {
    /// Die Grenzwerte sind Vertragswerte (lva#748): Eine stille Änderung der Konstanten lässt diesen Test scheitern
    /// (Gegenprobe: `maxHeldCount` auf 7 oder `maxHeldBytes` auf 8192 setzen).
    func testGrenzwerteSindUnabhaengigFestgeschrieben() {
        XCTAssertEqual(AssistantSetupGate<String>.maxHeldCount, 8)
        XCTAssertEqual(AssistantSetupGate<String>.maxHeldBytes, 16_384)
    }
}

/// Feste Zufallsfolge (SplitMix64), damit ein Fehlschlag mit dem Seed wiederholbar ist.
private struct SplitMix64 {
    private var state: UInt64
    init(seed: UInt64) { state = seed &+ 0x9E37_79B9_7F4A_7C15 }
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}
