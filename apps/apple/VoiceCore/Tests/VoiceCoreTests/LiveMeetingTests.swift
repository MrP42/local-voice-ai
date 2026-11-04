import XCTest
@testable import VoiceCore

final class LiveMeetingTests: XCTestCase {
    private let segs = [
        LiveSegment(id: "s1", start: 0, end: 8, text: "Wir müssen den Haushaltsantrag bis Freitag einreichen.", speaker: "Anna"),
        LiveSegment(id: "s2", start: 8, end: 15, text: "Herr Berger übernimmt die Abstimmung mit der Kämmerei.", speaker: "Anna"),
        LiveSegment(id: "s3", start: 15, end: 22, text: "Offen ist noch, ob der Personalrat beteiligt werden muss?", speaker: "Bernd"),
        LiveSegment(id: "s4", start: 22, end: 30, text: "Das klär", speaker: "Bernd", isFinal: false),
    ]

    func testAddsEvidencedItemsAndRejectsUnevidenced() {
        var state = LiveState()
        let report = state.apply(LiveDelta(summary: "Haushaltsantrag wird vorbereitet.", add: [
            .init(kind: .todo, text: "Abstimmung mit der Kämmerei", owner: "Herr Berger", evidence: ["s2"]),
            .init(kind: .todo, text: "Antrag einreichen", due: "Freitag", evidence: ["s1"]),
            .init(kind: .open, text: "Personalrat beteiligen?", evidence: ["s3"]),
            .init(kind: .decision, text: "Ohne Beleg", evidence: []),
            .init(kind: .todo, text: "Erfundene Person", owner: "Frau Meier", evidence: ["s1"]),
            .init(kind: .topic, text: "Vorläufiges Segment", evidence: ["s4"]),
            .init(kind: .suggestion, text: "Termin für die Rückmeldung festlegen?", evidence: []),
        ]), segments: segs)
        XCTAssertEqual(report.added.count, 4)
        XCTAssertEqual(report.rejections, [.missingEvidence("Ohne Beleg"), .unattributed("Erfundene Person"), .unknownEvidence("Vorläufiges Segment")])
        XCTAssertEqual(state.rejected, 3)
        XCTAssertEqual(state.items(.todo).map(\.owner), ["Herr Berger", nil])
        XCTAssertEqual(state.items(.todo)[1].due, "Freitag")
        XCTAssertEqual(state.items(.todo)[0].evidence, [LiveEvidence(transcriptVersion: 1, segmentId: "s2", start: 8, end: 15)])
        XCTAssertTrue(state.items(.suggestion)[0].isAISuggestion)
        XCTAssertEqual(state.summary, "Haushaltsantrag wird vorbereitet.")
        // Nur finalisierte Segmente zählen für den Fortschritt.
        XCTAssertEqual(state.processedThrough, 22)
    }

    func testDuplicatesMergeAndUpdatesNeedEvidence() {
        var state = LiveState()
        _ = state.apply(LiveDelta(add: [.init(kind: .topic, text: "Haushalt", evidence: ["s1"])]), segments: segs)
        let id = state.items[0].id
        let r = state.apply(LiveDelta(add: [.init(kind: .topic, text: "haushalt", evidence: ["s2"])],
                                      update: [.init(id: id, text: "Haushalt 2027", evidence: ["s1"]), .init(id: "x", evidence: ["s1"]),
                                               .init(id: id, owner: "Niemand", evidence: ["s2"])]), segments: segs)
        XCTAssertEqual(state.items.count, 1)
        XCTAssertEqual(state.items[0].text, "Haushalt 2027")
        XCTAssertEqual(state.items[0].evidence.map(\.segmentId), ["s1", "s2"])
        XCTAssertEqual(r.rejections, [.unknownItem("x"), .unattributed(id)])
        XCTAssertNotEqual(state.items[0].id, "l2-1")
    }

    func testAnsweredIsOnlyProposedUntilConfirmed() {
        var state = LiveState()
        _ = state.apply(LiveDelta(add: [.init(kind: .open, text: "Personalrat beteiligen?", evidence: ["s3"])]), segments: segs)
        let id = state.items[0].id
        let later = [LiveSegment(id: "s5", start: 30, end: 36, text: "Ja, der Personalrat muss beteiligt werden.")]
        _ = state.apply(LiveDelta(answered: [.init(id: id, evidence: ["s5"])]), segments: later)
        XCTAssertEqual(state.items[0].status, .answerProposed)
        state.rejectAnswer(id)
        XCTAssertEqual(state.items[0].status, .active)
        XCTAssertTrue(state.items[0].answerEvidence.isEmpty)
        _ = state.apply(LiveDelta(answered: [.init(id: id, evidence: ["s5"])]), segments: later)
        state.confirmDone(id)
        XCTAssertEqual(state.items[0].status, .done)
        let r = state.apply(LiveDelta(update: [.init(id: id, text: "neu", evidence: ["s5"])]), segments: later)
        XCTAssertEqual(r.rejections, [.closedItem(id)])
    }

    func testTickPolicy() {
        let p = LiveTickPolicy()
        func d(_ now: Double, _ last: Double?, _ words: Int, running: Bool = false, thermal: LiveThermal = .nominal, low: Bool = false, fg: Bool = true, ai: Bool = true) -> LiveTickPolicy.Decision {
            p.decide(now: now, lastRun: last, newWords: words, running: running, thermal: thermal, lowPower: low, foreground: fg, aiAllowed: ai)
        }
        XCTAssertEqual(d(10, nil, 50), .wait)
        XCTAssertEqual(d(10, nil, 130), .run)
        XCTAssertEqual(d(100, 0, 10), .run)
        XCTAssertEqual(d(60, 0, 260), .run)
        XCTAssertEqual(d(60, 0, 100), .wait)
        XCTAssertEqual(d(100, 0, 10, running: true), .wait)
        XCTAssertEqual(d(100, 0, 0), .wait)
        XCTAssertEqual(d(100, 0, 400, thermal: .serious), .wait)
        XCTAssertEqual(d(310, 0, 10, low: true), .run)
        XCTAssertEqual(d(100, 0, 10, thermal: .critical), .paused("KI pausiert (Wärme)"))
        XCTAssertEqual(d(100, 0, 10, fg: false), .paused("KI pausiert, holt nach dem Entsperren auf"))
        XCTAssertEqual(d(100, 0, 10, ai: false), .paused("KI-Auswertung nicht freigegeben"))
    }

    func testBudgetAddsUpAndPromptStaysInside() {
        let budget = LiveTokenBudget()
        XCTAssertGreaterThanOrEqual(budget.reserve, 0)
        XCTAssertEqual(budget.reserve + budget.instructions + budget.schema + budget.state + budget.newSegments + budget.olderEvidence + budget.output, 4096)
        XCTAssertLessThanOrEqual(LivePromptBuilder.estimate(LivePromptBuilder.instructions), budget.instructions)

        var state = LiveState()
        state.processedThrough = 8
        let many = (0..<400).map { LiveSegment(id: "n\($0)", start: Double(10 + $0 * 5), end: Double(14 + $0 * 5), text: "Wir sprechen über Punkt \($0) und seine Folgen für den Haushalt.") }
        let prompt = LivePromptBuilder.build(state: state, pending: segs + many, older: [segs[0]])
        XCTAssertLessThanOrEqual(prompt.estimatedTokens, budget.context - budget.reserve)
        XCTAssertTrue(prompt.fits)
        // Ein einzelnes überlanges Segment und eine riesige Zusammenfassung sprengen das Budget nicht.
        var big = LiveState(); big.summary = String(repeating: "Zusammenfassung ", count: 2000)
        let long = LiveSegment(id: "a99", start: 0, end: 5, text: String(repeating: "Wort ", count: 5000))
        let capped = LivePromptBuilder.build(state: big, pending: [long])
        XCTAssertTrue(capped.fits)
        XCTAssertLessThanOrEqual(capped.estimatedTokens, budget.context - budget.reserve)
        // Nicht gekürzt, sondern geteilt: nur gesendete Teilstücke zählen als Beleg und Fortschritt.
        XCTAssertFalse(capped.segments.isEmpty)
        XCTAssertTrue(capped.segments.allSatisfy { $0.id.hasPrefix("a99.") && capped.input.contains($0.text) })
        XCTAssertLessThan(capped.through, 5)
        big.processedThrough = capped.through
        let next = LivePromptBuilder.build(state: big, pending: [long])
        XCTAssertTrue(next.fits)
        XCTAssertNotEqual(next.segments.first?.id, capped.segments.first?.id)
        XCTAssertGreaterThan(next.through, capped.through)
        let all = LivePromptBuilder.pieces(long, maxTokens: 1099, count: LivePromptBuilder.estimate)
        XCTAssertEqual(all.map(\.text).joined(separator: " "), long.text.trimmingCharacters(in: .whitespaces))
        XCTAssertEqual(all.last?.end, 5)
        XCTAssertEqual(LiveSegment.baseId("a99.3"), "a99")
        XCTAssertFalse(prompt.segments.contains { $0.id == "s4" }, "vorläufige Segmente nie auswerten")
        XCTAssertTrue(prompt.segments.contains { $0.id == "s1" }, "ältere Belege gezielt nachgeladen")
        XCTAssertTrue(prompt.input.contains("[s2] Anna: Herr Berger"))
        XCTAssertLessThan(prompt.through, many.last!.end, "Rest kommt im nächsten Takt")
        XCTAssertEqual(prompt.through, prompt.segments.last!.end)
    }

    func testStateCompactionDropsDoneThenOldTopics() {
        var state = LiveState()
        state.summary = "Kurz."
        let seg = (0..<120).map { LiveSegment(id: "t\($0)", start: Double($0), end: Double($0) + 1, text: "Thema Nummer \($0) mit etwas längerem Text zur Beschreibung des Punktes") }
        _ = state.apply(LiveDelta(add: seg.map { .init(kind: .topic, text: $0.text, evidence: [$0.id]) }
                                  + [.init(kind: .todo, text: "Wichtige Aufgabe", evidence: ["t0"])]), segments: seg)
        let text = LivePromptBuilder.compact(state, budget: 900, count: LivePromptBuilder.estimate)
        XCTAssertLessThanOrEqual(LivePromptBuilder.estimate(text), 900)
        XCTAssertTrue(text.contains("Wichtige Aufgabe"))
        XCTAssertFalse(text.contains("Nummer 0 "))
    }

    func testConsentLog() throws {
        let t0 = Date(timeIntervalSince1970: 1_000)
        func rec(_ reason: ConsentRecord.Reason, _ at: Double, scopes: [ConsentRecord.Scope] = ConsentRecord.Scope.allCases, controller: String = "Landratsamt, Amt 12") -> ConsentRecord {
            ConsentRecord(confirmedAt: t0.addingTimeInterval(at), channel: .app, reason: reason, scopes: scopes,
                          purpose: "Protokoll der Teamrunde", controller: controller, retentionDays: 90, audioOffset: at)
        }
        var log = ConsentLog()
        XCTAssertFalse(log.canRecord)
        XCTAssertThrowsError(try log.append(rec(.personJoined, 0))) { XCTAssertEqual($0 as? ConsentLog.ConsentError, .notStart) }
        XCTAssertThrowsError(try log.append(rec(.start, 0, controller: " "))) { XCTAssertEqual($0 as? ConsentLog.ConsentError, .incomplete) }
        try log.append(rec(.start, 0))
        XCTAssertTrue(log.canRecord)
        XCTAssertTrue(log.allows(.aiAnalysis))
        // Person kommt hinzu und stimmt der KI-Auswertung nicht zu.
        try log.append(rec(.personJoined, 600, scopes: [.recording, .transcription]))
        XCTAssertTrue(log.allows(.aiAnalysis, at: 300))
        XCTAssertFalse(log.allows(.aiAnalysis, at: 700))
        XCTAssertFalse(log.allows(.aiAnalysis))
        XCTAssertThrowsError(try log.append(rec(.personJoined, 100))) { XCTAssertEqual($0 as? ConsentLog.ConsentError, .outOfOrder) }
        // Ganzes Intervall: ein Segment über die Beitrittsstelle hinweg ist nicht freigegeben.
        XCTAssertTrue(log.allows(.aiAnalysis, from: 550, to: 600))
        XCTAssertFalse(log.allows(.aiAnalysis, from: 590, to: 610))
        XCTAssertFalse(log.allows(.aiAnalysis, from: 650, to: 660))
        XCTAssertTrue(log.allows(.transcription, from: 0, to: 900))
        XCTAssertEqual(log.version, 2)
        // Kein reiner Aufnahmemodus: ohne Transkription unvollständig.
        var bare = ConsentLog()
        XCTAssertThrowsError(try bare.append(rec(.start, 0, scopes: [.recording]))) { XCTAssertEqual($0 as? ConsentLog.ConsentError, .incomplete) }
        let round = try JSONDecoder().decode(ConsentLog.self, from: JSONEncoder().encode(log))
        XCTAssertEqual(round, log)
        XCTAssertEqual(round.records[0].noticeVersion, ConsentNotice.version)
        let notice = ConsentNotice.text(purpose: "Protokoll", controller: "Amt 12", retentionDays: 90, ai: true)
        for word in ["aufgenommen", "transkribiert", "KI", "Zweck", "Verantwortlich", "90 Tagen", "Widerruf", "zugestimmt"] {
            XCTAssertTrue(notice.contains(word), word)
        }
    }
}
