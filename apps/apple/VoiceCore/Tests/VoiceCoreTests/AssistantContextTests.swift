import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Datenkontext mit Sperre. Wortlisten und fehlende Treffer dürfen freien Text nie
// ERLAUBEN, nur verbieten. Zustand `.data` oder `.free`; `.free` rastet bei Eigendatenbezug oder relevantem Treffer auf `.data`
// ein; zurück nur durch `resumeFree`. Dazu Widerruf und Freigabe auf Sitzungsebene.

private final class Prompts: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [(instructions: String, prompt: String)] = []
    func add(_ i: String, _ p: String) { lock.lock(); items.append((i, p)); lock.unlock() }
    var all: [(instructions: String, prompt: String)] { lock.lock(); defer { lock.unlock() }; return items }
}

private struct ReplyModel: BookTalkModel {
    var pieces: [String]
    var prompts = Prompts()
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        prompts.add(instructions, prompt)
        let pieces = pieces
        return AsyncThrowingStream { c in
            for p in pieces { c.yield(p) }
            c.finish()
        }
    }
}

private final class Generations: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [Int] = []
    func add(_ g: Int) { lock.lock(); items.append(g); lock.unlock() }
    var all: [Int] { lock.lock(); defer { lock.unlock() }; return items }
}

private let castorpText = "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."
private let castorpQuestion = "Wohin fährt Hans Castorp im Sanatorium nach Davos?"
private let mont = "Der Mont Blanc ist 4806 Meter hoch."

final class AssistantContextTests: XCTestCase {
    private let titles = ["Der Zauberberg"]
    private let castorp = assistantPassage(.books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: castorpText)
    private let open = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil,
                                       timeout: 5, allowUnsourced: true, consentVersion: 1)
    private let strict = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)

    private func turn(_ round: AssistantRoundKind?, _ user: String = "Frage", at: Double = 0) -> AssistantTurn {
        AssistantTurn(user: user, assistant: "Antwort", at: Date(timeIntervalSince1970: 1_760_000_000 + at), round: round)
    }

    private func decide(_ c: AssistantContext, _ q: String, passages: [AssistantPassage] = [], optIn: Bool = true, inventory: Bool = false) -> AssistantContext.Decision {
        c.decide(question: q, titles: titles, passages: passages, optIn: optIn, inventory: inventory)
    }

    // MARK: Anfangsregel

    func testInitialRule() {
        XCTAssertEqual(AssistantContext.initial(optIn: false, history: []), .data, "Opt-in aus: immer Daten")
        XCTAssertEqual(AssistantContext.initial(optIn: false, history: [turn(.general)]), .data)
        XCTAssertEqual(AssistantContext.initial(optIn: true, history: []), .free, "leerer Verlauf")
        XCTAssertEqual(AssistantContext.initial(optIn: true, history: [turn(.data), turn(.general)]), .free, "letzter Beitrag ohne Quelle")
        XCTAssertEqual(AssistantContext.initial(optIn: true, history: [turn(.general), turn(.data)]), .data, "letzter Beitrag aus Daten")
        XCTAssertEqual(AssistantContext.initial(optIn: true, history: [turn(nil)]), .data, "alter Beitrag ohne Rundenart zählt als Daten")
        XCTAssertEqual(AssistantContext(optIn: true, history: [turn(.general)]).mode, .free)
        XCTAssertEqual(AssistantContext(optIn: true, history: [turn(.data)]).mode, .data)
    }

    // MARK: Entscheidung

    func testOptedOutIsAlwaysDataAndCountsAsADataRound() {
        let c = AssistantContext(optIn: false, history: [])
        let d = decide(c, "Wie hoch ist der Mont Blanc?", optIn: false)
        XCTAssertEqual(d, AssistantContext.Decision(kind: .data, reason: .optedOut, before: .data))
        c.optInChanged(true)
        XCTAssertEqual(c.mode, .data, "das Gespräch hatte schon eine Datenrunde: Einschalten gibt nichts frei")
    }

    func testFreeStateWithNothingForDataStaysFreeAndDoesNotLatch() {
        let c = AssistantContext(optIn: true, history: [])
        for q in ["Wie hoch ist der Mont Blanc?", "Hallo", "Und wie alt ist er?", "Wie funktioniert eine Wärmepumpe?"] {
            XCTAssertEqual(decide(c, q), AssistantContext.Decision(kind: .general, reason: .free, before: .free), q)
        }
        XCTAssertEqual(c.mode, .free)
        // Irrelevante Treffer (andere Wörter) ändern daran nichts.
        XCTAssertEqual(decide(c, "Wie hoch ist der Mont Blanc?", passages: [castorp]).kind, .general)
    }

    func testOwnDataReferenceMakesADataRoundAndLatches() {
        for q in ["Was steht in meiner Notiz?", "Welche Termine habe ich morgen?", "Was habe ich gestern gesagt?", "Was steht im Zauberberg?"] {
            let c = AssistantContext(optIn: true, history: [])
            let d = decide(c, q)
            XCTAssertEqual(d, AssistantContext.Decision(kind: .data, reason: .ownData, before: .free), q)
            XCTAssertEqual(c.mode, .data, "eingerastet: \(q)")
        }
    }

    func testRelevantPassageAndInventoryLatch() {
        let hit = AssistantContext(optIn: true, history: [])
        XCTAssertEqual(decide(hit, castorpQuestion, passages: [castorp]).reason, .relevantPassage)
        XCTAssertEqual(hit.mode, .data)
        let inv = AssistantContext(optIn: true, history: [])
        XCTAssertEqual(decide(inv, "Welche Bücher habe ich?", inventory: true), AssistantContext.Decision(kind: .data, reason: .inventory, before: .free))
        XCTAssertEqual(inv.mode, .data)
    }

    /// „Allgemein:“ vor der Frage ändert nichts: Ein Treffer macht die Runde zur Datenrunde.
    func testAGeneralPrefixDoesNotOverrideAHit() {
        let c = AssistantContext(optIn: true, history: [])
        let d = decide(c, "Allgemein: Wohin fährt Hans Castorp?", passages: [castorp])
        XCTAssertEqual(d.kind, .data)
        XCTAssertEqual(d.reason, .relevantPassage)
        XCTAssertEqual(c.mode, .data)
    }

    /// Zustand `.data`: jede Runde ist eine Datenrunde, ohne Ausnahme (keine Anaphern-, Wortzahl- oder „allgemein“-Regel).
    func testDataStateIsLockedWithoutExceptions() {
        let c = AssistantContext(optIn: true, history: [turn(.data)])
        XCTAssertEqual(c.mode, .data)
        for q in ["Welche Person trägt künftig die Verantwortung?", "Hallo", "Wie hoch ist der Mont Blanc?", "Und was macht er dort?", "Allgemein: Wie hoch ist der Mont Blanc?",
                  "Das war's.", "Erkläre mir bitte ausführlich die Unterschiede zwischen Wärmepumpen und Gasheizungen in Altbauten", "?"] {
            XCTAssertEqual(decide(c, q), AssistantContext.Decision(kind: .data, reason: .locked, before: .data), q)
        }
        XCTAssertEqual(c.mode, .data)
    }

    func testAfterADataRoundTheQuestionAboutResponsibilityStaysADataRound() {
        let c = AssistantContext(optIn: true, history: [])
        XCTAssertEqual(decide(c, castorpQuestion, passages: [castorp]).kind, .data)
        let next = decide(c, "Welche Person trägt künftig die Verantwortung?")
        XCTAssertEqual(next, AssistantContext.Decision(kind: .data, reason: .locked, before: .data))
    }

    /// Ein Lauf, der `.free` sah, während ein anderer schon eingerastet ist: strenger, der Zustand gilt jetzt.
    func testARunThatSawFreeWhileAnotherLatchedIsDowngradedToData() {
        let c = AssistantContext(optIn: true, history: [])
        _ = decide(c, "Was steht in meiner Notiz?")
        XCTAssertEqual(c.mode, .data)
        let late = c.settle(.general, .free, before: .free, latch: false)
        XCTAssertEqual(late, AssistantContext.Decision(kind: .data, reason: .locked, before: .free))
    }

    /// Viele gleichzeitige Entscheidungen: Sobald eine eingerastet ist, kommt aus keiner späteren eine Runde ohne Quelle.
    func testConcurrentDecisionsNeverAllowFreeTextAfterTheLatch() async {
        let c = AssistantContext(optIn: true, history: [])
        let kinds = await withTaskGroup(of: AssistantRoundKind.self) { group -> [AssistantRoundKind] in
            for i in 0..<200 {
                group.addTask {
                    let q = i == 100 ? "Was steht in meiner Notiz?" : "Wie hoch ist der Mont Blanc?"
                    return c.decide(question: q, titles: [], passages: [], optIn: true, inventory: false).kind
                }
            }
            var out: [AssistantRoundKind] = []
            for await k in group { out.append(k) }
            return out
        }
        XCTAssertEqual(kinds.count, 200)
        XCTAssertEqual(c.mode, .data)
        XCTAssertEqual(c.decide(question: "Wie hoch ist der Mont Blanc?", titles: [], passages: [], optIn: true, inventory: false).kind, .data)
    }

    // MARK: Zustandswechsel

    func testResumeFreeIsOnlyAnExplicitActionWhenThereIsSomethingToDo() {
        let free = AssistantContext(optIn: true, history: [])
        XCTAssertFalse(free.resumeFree(optIn: true), "schon frei: nichts zu tun")
        let c = AssistantContext(optIn: true, history: [turn(.data)])
        XCTAssertFalse(c.resumeFree(optIn: false), "Opt-in aus: nichts zu tun")
        XCTAssertEqual(c.mode, .data)
        XCTAssertTrue(c.resumeFree(optIn: true))
        XCTAssertEqual(c.mode, .free)
        XCTAssertEqual(decide(c, "Wie hoch ist der Mont Blanc?").kind, .general)
        _ = decide(c, "Was steht in meiner Notiz?")
        XCTAssertEqual(c.mode, .data, "ein Eigendatenbezug rastet erneut ein")
        XCTAssertTrue(c.resumeFree(optIn: true))
    }

    func testOptInChangedFollowsTheRules() {
        let fresh = AssistantContext(optIn: false, history: [])
        fresh.optInChanged(true)
        XCTAssertEqual(fresh.mode, .free, "noch keine Datenrunde")
        fresh.optInChanged(false)
        XCTAssertEqual(fresh.mode, .data, "aus: immer Daten")
        fresh.optInChanged(true)
        XCTAssertEqual(fresh.mode, .free)

        let used = AssistantContext(optIn: false, history: [turn(.data)])
        used.optInChanged(true)
        XCTAssertEqual(used.mode, .data, "der Verlauf hat eine Datenrunde")

        let general = AssistantContext(optIn: false, history: [turn(.general)])
        general.optInChanged(true)
        XCTAssertEqual(general.mode, .free, "nur Runden ohne Quelle im Verlauf")
    }

    func testResetFollowsTheInitialRuleAndForgetsTheLatch() {
        let c = AssistantContext(optIn: true, history: [])
        _ = decide(c, "Was steht in meiner Notiz?")
        XCTAssertEqual(c.mode, .data)
        c.reset(optIn: true, history: [])
        XCTAssertEqual(c.mode, .free, "neues Gespräch")
        c.reset(optIn: true, history: [turn(.general), turn(.data)])
        XCTAssertEqual(c.mode, .data, "Verlauf geladen")
        c.optInChanged(false)
        c.optInChanged(true)
        XCTAssertEqual(c.mode, .data, "die geladene Datenrunde zählt")
        c.reset(optIn: false, history: [turn(.general)])
        XCTAssertEqual(c.mode, .data)
    }

    // MARK: Verlauf für die Runde ohne Quelle

    func testFreeHistoryHoldsOnlyGeneralTurnsAfterTheLastDataTurnAndTheLastResume() {
        let turns = [turn(.general, "A", at: 1), turn(.data, "B", at: 2), turn(.general, "C", at: 3), turn(.general, "D", at: 4)]
        XCTAssertEqual(AssistantContext.freeHistory(turns, since: nil).map(\.user), ["C", "D"], "nichts vor der letzten Datenrunde")
        let since = Date(timeIntervalSince1970: 1_760_000_003)
        XCTAssertEqual(AssistantContext.freeHistory(turns, since: since).map(\.user), ["D"], "nichts seit dem letzten `resumeFree`")
        XCTAssertEqual(AssistantContext.freeHistory([turn(nil, "alt")], since: nil), [], "ein Beitrag ohne Rundenart gilt als Datenrunde")
        XCTAssertEqual(AssistantContext.freeHistory([], since: nil), [])
    }

    // MARK: Relevanz

    func testRelevanceNeedsTwoContentWordsOrTheOnlyOne() {
        XCTAssertTrue(AssistantContext.isRelevant(castorp, to: "Wohin fährt Hans Castorp?"))
        XCTAssertTrue(AssistantContext.isRelevant(castorp, to: "Davos?"), "bei einem Inhaltswort genügt dieses")
        XCTAssertFalse(AssistantContext.isRelevant(castorp, to: "Venedig?"))
        XCTAssertFalse(AssistantContext.isRelevant(castorp, to: "Wie hoch ist der Mont Blanc?"))
        XCTAssertFalse(AssistantContext.isRelevant(castorp, to: "Wie hoch ist der Mont Blanc in Davos?"), "nur ein Wort gedeckt, zwei verlangt")
        XCTAssertTrue(AssistantContext.isRelevant(castorp, to: "Und?"), "eine Frage ohne Inhaltswort kann nichts ausschließen")
    }

    // MARK: Sitzung: Freigabe

    private func castorpKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorpText + " Er bleibt drei Wochen.")]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func collect(_ s: AssistantSession, _ id: String, _ text: String) async -> [AssistantEvent] {
        var out: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: id, text: text)) { out.append(e) }
        return out
    }

    private func answer(_ events: [AssistantEvent]) -> AssistantAnswer? { if case .done(_, let a)? = events.last { a } else { nil } }

    private func statuses(_ events: [AssistantEvent]) -> [AssistantSentenceStatus] {
        events.compactMap { if case .sentence(_, _, _, _, let st, _, _) = $0 { st } else { nil } }
    }

    /// Freischaltung `false` (injizierbar): weder Opt-in noch Knopf, die Sitzung verhält sich wie Opt-in aus.
    func testNotReleasedBehavesLikeOptInOff() async {
        let prompts = Prompts()
        let s = AssistantSession(model: ReplyModel(pieces: [mont], prompts: prompts), knowledge: await castorpKnowledge(), config: open, released: false)
        XCTAssertFalse(s.isReleased)
        XCTAssertFalse(s.allowsUnsourced, "die Konfiguration mit Opt-in zählt nicht")
        XCTAssertFalse(s.canResumeFree)
        XCTAssertEqual(s.contextMode, .data)
        let g = s.releaseGeneration
        XCTAssertEqual(s.setAllowUnsourced(true, consentVersion: 2), g, "Einschalten ändert nichts")
        XCTAssertFalse(s.allowsUnsourced)
        XCTAssertFalse(s.resumeFree())
        XCTAssertEqual(s.releaseGeneration, g)
        let events = await collect(s, "r1", "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(answer(events)?.round, .data)
        XCTAssertFalse(statuses(events).contains(.unsourced))
        XCTAssertTrue(prompts.all.first?.instructions.contains("Unter AUSZÜGE stehen") ?? false)
        XCTAssertFalse(prompts.all.first?.instructions.contains("In dieser Runde siehst du keine Inhalte") ?? true)
    }

    func testReleasedIsTheDefaultAndMatchesTheConstant() async {
        XCTAssertTrue(AssistantUnsourcedConsent.isReleased)
        let s = AssistantSession(model: ReplyModel(pieces: []), knowledge: await castorpKnowledge(), config: open)
        XCTAssertEqual(s.isReleased, AssistantUnsourcedConsent.isReleased)
        XCTAssertTrue(s.allowsUnsourced)
        XCTAssertEqual(s.contextMode, .free)
        XCTAssertFalse(s.canResumeFree, "frei: kein Knopf")
    }

    /// „Allgemein: Wohin fährt Hans Castorp?“ mit Treffer bleibt Datenrunde; danach bleibt „Welche Person trägt künftig die
    /// Verantwortung?“ eine Datenrunde, auch wenn das Modell Allgemeinwissen anbietet.
    func testHitStaysADataRoundAndTheNextQuestionStaysLockedInTheSession() async {
        let prompts = Prompts()
        let s = AssistantSession(model: ReplyModel(pieces: [castorpText.replacingOccurrences(of: "ins Sanatorium ", with: "")], prompts: prompts),
                                 knowledge: await castorpKnowledge(), config: open)
        let first = await collect(s, "r1", "Allgemein: Wohin fährt Hans Castorp?")
        XCTAssertEqual(answer(first)?.round, .data)
        XCTAssertEqual(statuses(first), [.announcement, .grounded])
        XCTAssertEqual(s.contextMode, .data)
        await s.commit(requestId: "r1", user: "Allgemein: Wohin fährt Hans Castorp?", spoken: castorpText, interrupted: false)

        let kept = await s.history
        let talking = AssistantSession(model: ReplyModel(pieces: ["Die Verantwortung trägt künftig Anna. Sie ist die neue Leiterin."], prompts: prompts),
                                       knowledge: await castorpKnowledge(), config: open, history: kept)
        let second = await collect(talking, "r2", "Welche Person trägt künftig die Verantwortung?")
        XCTAssertEqual(answer(second)?.round, .data, "nach einer Datenrunde bleibt es dabei")
        XCTAssertFalse(statuses(second).contains(.unsourced))
        XCTAssertTrue(prompts.all.last?.instructions.contains("Unter AUSZÜGE stehen") ?? false)
    }

    func testStateAfterNewConversationFollowsTheInitialRule() async {
        let s = AssistantSession(model: ReplyModel(pieces: [castorpText]), knowledge: await castorpKnowledge(), config: open)
        _ = await collect(s, "r1", castorpQuestion)
        XCTAssertEqual(s.contextMode, .data)
        await s.clearHistory()
        XCTAssertEqual(s.contextMode, .free, "„Neues Gespräch“: Anfangsregel")
        await s.restore(history: [AssistantTurn(user: "Frage", assistant: "Antwort", round: .data)])
        XCTAssertEqual(s.contextMode, .data)
        XCTAssertTrue(s.canResumeFree)
    }

    /// Nach `resumeFree()` enthält der Prompt keinen früheren Datenbeitrag; auch keinen Beitrag ohne Quelle vor der letzten
    /// Datenrunde oder vor dem Zurückschalten.
    func testPromptAfterResumeFreeContainsNoEarlierDataTurnNorEarlierFreeTurn() async {
        let prompts = Prompts()
        let hist = [AssistantTurn(user: "Frage zum Buch", assistant: "Hans Castorp fährt nach Davos zu Joachim.", round: .data)]
        let s = AssistantSession(model: ReplyModel(pieces: [mont], prompts: prompts), knowledge: await castorpKnowledge(), config: open, history: hist)
        XCTAssertEqual(s.contextMode, .data)
        XCTAssertTrue(s.canResumeFree)
        let g = s.releaseGeneration
        XCTAssertTrue(s.resumeFree())
        XCTAssertGreaterThan(s.releaseGeneration, g, "das Zurückschalten erhöht die Generation")
        XCTAssertEqual(s.contextMode, .free)
        XCTAssertFalse(s.canResumeFree)
        let events = await collect(s, "r1", "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(answer(events)?.round, .general)
        var p = prompts.all.last?.prompt ?? ""
        for word in ["Castorp", "Joachim", "Davos", "Frage zum Buch"] { XCTAssertFalse(p.contains(word), "\(word)\n\(p)") }
        await s.commit(requestId: "r1", user: "Wie hoch ist der Mont Blanc?", spoken: mont, interrupted: false)

        _ = await collect(s, "r2", "Und wie alt ist er?")
        p = prompts.all.last?.prompt ?? ""
        XCTAssertTrue(p.contains("Nutzer: Wie hoch ist der Mont Blanc?"), "Beiträge ohne Quelle seit dem Zurückschalten sind im Prompt\n\(p)")
        XCTAssertFalse(p.contains("Frage zum Buch"))
        await s.commit(requestId: "r2", user: "Und wie alt ist er?", spoken: "Der Berg ist sehr alt.", interrupted: false)

        // Eine Datenrunde (nicht gesprochen, also nicht im Verlauf) rastet ein; nach dem Zurückschalten sieht der Prompt auch die
        // früheren Beiträge ohne Quelle nicht mehr.
        let data = await collect(s, "r3", "Was steht in meiner Notiz?")
        XCTAssertEqual(answer(data)?.round, .data)
        XCTAssertEqual(s.contextMode, .data)
        XCTAssertTrue(s.resumeFree())
        _ = await collect(s, "r4", "Wie tief ist der Marianengraben?")
        p = prompts.all.last?.prompt ?? ""
        XCTAssertFalse(p.contains("Mont Blanc"), "Beiträge vor dem letzten Zurückschalten sind weg\n\(p)")
        XCTAssertFalse(p.contains("Frage zum Buch"))
    }

    /// `resumeFree` ist ein Ereignis der Freigabe: Die Generation steigt, laufende Antworten werden widerrufen, der Beobachter hört es.
    func testResumeFreeRaisesTheGenerationRevokesRunsAndNotifiesTheObserver() async {
        let seen = Generations()
        let s = AssistantSession(model: ReplyModel(pieces: [mont]), knowledge: await castorpKnowledge(), config: open,
                                 history: [AssistantTurn(user: "F", assistant: "A", round: .data)])
        s.setReleaseObserver { seen.add($0) }
        let g = s.releaseGeneration
        XCTAssertTrue(s.resumeFree())
        XCTAssertEqual(seen.all, [g + 1])
        XCTAssertFalse(s.resumeFree(), "schon frei: keine Änderung, keine neue Generation")
        XCTAssertEqual(seen.all, [g + 1])
        XCTAssertEqual(s.releaseGeneration, g + 1)
    }

    // MARK: Sitzung: Widerruf

    /// Der Beobachter hört jede Erhöhung, auch nach dem Modellende, bei Bestandsantworten und ohne angemeldeten Lauf.
    func testObserverFiresOnEveryIncreaseIncludingInventoryAnswersAfterTheModelEnded() async {
        let seen = Generations()
        let s = AssistantSession(model: ReplyModel(pieces: []), knowledge: await castorpKnowledge(), config: strict)
        s.setReleaseObserver { seen.add($0) }
        let g0 = s.releaseGeneration
        let events = await collect(s, "r1", "Welche Bücher habe ich?")
        XCTAssertEqual(answer(events)?.counts.inventory, 2, "eine Bestandsantwort ohne Quellen und ohne Modell")
        XCTAssertEqual(s.registeredRunCount, 1, "der Lauf bleibt bis `commit` angemeldet")
        XCTAssertEqual(s.releaseDidChange(), g0 + 1)
        XCTAssertEqual(s.registeredRunCount, 0, "der Widerruf erreicht auch den fertigen Lauf")
        await s.setAreas([.books])
        s.setAllowUnsourced(true)
        s.setAllowUnsourced(true)   // keine Änderung
        XCTAssertTrue(s.resumeFree())
        XCTAssertEqual(seen.all, Array((g0 + 1)...(g0 + 5)), "genau eine Meldung je Erhöhung, in Reihenfolge")
        XCTAssertEqual(s.releaseGeneration, g0 + 5)
    }

    /// Ein Lauf bleibt bis zum Abschluss seiner Ausgabe (Commit) angemeldet: Ein Widerruf nach dem Modellende erreicht ihn noch,
    /// der Beitrag kommt nicht in den Verlauf, und das Protokoll sagt „widerrufen“.
    func testARunStaysRegisteredUntilCommitSoARevocationAfterTheModelEndStillReachesIt() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-context-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = AssistantSession(model: ReplyModel(pieces: [castorpText]), knowledge: await castorpKnowledge(), config: strict, audit: audit)
        let events = await collect(s, "r1", castorpQuestion)
        XCTAssertNotNil(answer(events), "das Modell ist fertig, die Antwort auch")
        XCTAssertEqual(s.registeredRunCount, 1)
        s.releaseDidChange()
        XCTAssertEqual(s.registeredRunCount, 0)
        let wrote = await s.commit(requestId: "r1", user: castorpQuestion, spoken: castorpText, interrupted: false)
        XCTAssertTrue(wrote)
        let history = await s.history
        XCTAssertEqual(history, [], "nach dem Widerruf kein Beitrag")
        let entries = await audit.entries()
        XCTAssertEqual(entries.first?.outcome, .revoked)
        XCTAssertEqual(entries.first?.history?.ok, false)

        // Ohne Widerruf endet die Anmeldung mit dem Commit.
        let again = await collect(s, "r2", castorpQuestion)
        XCTAssertNotNil(answer(again))
        XCTAssertEqual(s.registeredRunCount, 1)
        await s.commit(requestId: "r2", user: castorpQuestion, spoken: castorpText, interrupted: false)
        XCTAssertEqual(s.registeredRunCount, 0)
        let kept = await s.history
        XCTAssertEqual(kept.count, 1)
    }

    /// Prüfung der Generation und `history.append` im selben Lock wie `bump`: Nach der Rückkehr von `bump` kommt kein Beitrag mehr
    /// in den Verlauf, egal wie sich `commit` und der Widerruf überlappen. Das Protokoll stimmt mit dem Verlauf überein.
    func testHistoryAppendAndBumpNeverInterleave() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-context-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        var appended = 0
        var refused = 0
        for i in 0..<60 {
            let id = "r\(i)"
            let s = AssistantSession(model: ReplyModel(pieces: [castorpText]), knowledge: await castorpKnowledge(), config: strict, audit: audit)
            _ = await collect(s, id, castorpQuestion)
            let results = await withTaskGroup(of: Int.self) { group -> [Int] in
                group.addTask {
                    s.releaseDidChange()
                    return await s.history.count   // nach der Rückkehr von `bump` gelesen
                }
                group.addTask {
                    await s.commit(requestId: id, user: castorpQuestion, spoken: castorpText, interrupted: false)
                    return -1
                }
                var out: [Int] = []
                for await r in group { out.append(r) }
                return out
            }
            let afterBump = results.first { $0 >= 0 } ?? -2
            let final = await s.history.count
            XCTAssertEqual(afterBump, final, "nach `bump` kommt nichts mehr hinzu (\(id))")
            if final == 1 { appended += 1 } else { refused += 1 }
        }
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 60)
        XCTAssertEqual(entries.filter { $0.history?.ok == true }.count, appended)
        XCTAssertEqual(entries.filter { $0.history?.ok == false }.count, refused)
        XCTAssertEqual(entries.filter { $0.outcome == .revoked }.count, refused, "ein verworfener Verlaufs-Commit ist ein widerrufener Lauf")
    }
}
