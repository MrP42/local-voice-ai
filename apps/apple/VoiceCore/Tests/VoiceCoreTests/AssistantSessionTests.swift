import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Ablauf einer Antwort mit künstlichen Modellen.

private final class Log: @unchecked Sendable {
    private let lock = NSLock()
    private var calls: [(instructions: String, prompt: String, maxTokens: Int)] = []
    private var ended = 0
    func hit(_ i: String, _ p: String, _ m: Int) { lock.lock(); calls.append((i, p, m)); lock.unlock() }
    func terminated() { lock.lock(); ended += 1; lock.unlock() }
    var all: [(instructions: String, prompt: String, maxTokens: Int)] { lock.lock(); defer { lock.unlock() }; return calls }
    var terminations: Int { lock.lock(); defer { lock.unlock() }; return ended }
}

/// Liefert Stücke mit Pause dazwischen; `fail` nach den Stücken, `hang` wartet danach auf Abbruch.
private struct ScriptModel: BookTalkModel {
    var pieces: [String]
    var pauseMs: UInt64 = 0
    var fail: (any Error)?
    var hang = false
    var log = Log()

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        log.hit(instructions, prompt, maxTokens)
        let pieces = pieces, pause = pauseMs, fail = fail, hang = hang, log = log
        return AsyncThrowingStream { c in
            let task = Task {
                for p in pieces {
                    if pause > 0 { try? await Task.sleep(nanoseconds: pause * 1_000_000) }
                    if Task.isCancelled { return }
                    c.yield(p)
                }
                if hang {
                    while !Task.isCancelled { try? await Task.sleep(nanoseconds: 10_000_000) }
                    return
                }
                c.finish(throwing: fail)
            }
            c.onTermination = { _ in log.terminated(); task.cancel() }
        }
    }
}

/// Erste Aufrufe werfen `contextOverflow`, danach kommt die Antwort.
private struct OverflowModel: BookTalkModel {
    var failures: Int
    var answer = "Alles klar. Das passt jetzt."
    var log = Log()
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        log.hit(instructions, prompt, maxTokens)
        let n = log.all.count, failures = failures, answer = answer
        return AsyncThrowingStream { c in
            if n <= failures { c.finish(throwing: BookTalkModelError.contextOverflow) } else { c.yield(answer); c.finish() }
        }
    }
}

private func collect(_ s: AssistantSession, _ r: AssistantRequest) async -> [AssistantEvent] {
    var out: [AssistantEvent] = []
    for await e in s.events(r) { out.append(e) }
    return out
}

final class AssistantSessionTests: XCTestCase {
    /// Ablauf-, Abbruch- und Fehlertests prüfen nicht die Belegtreue: Hier sind unbelegte Sätze erlaubt, damit jeder Satz
    /// des künstlichen Modells ankommt. Die Satzprüfung testen `strict` und der Abschnitt „Antworten ohne Quelle“.
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick", timeout: 5,
                                         allowUnsourced: true, consentVersion: 1)
    private var strict: AssistantConfig {
        AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick", timeout: 5)
    }

    private func knowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [
            ("S. 12", "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func session(_ model: any BookTalkModel, history: [AssistantTurn] = [], config: AssistantConfig? = nil) async -> AssistantSession {
        AssistantSession(model: model, knowledge: await knowledge(), config: config ?? self.config, history: history)
    }

    // MARK: Ablauf

    func testNormalFlowSourcesSentencesDone() async {
        let model = ScriptModel(pieces: ["Castorp fährt nach Davos", ", um seinen Vetter zu besuchen. Joachim ist", " dort sein Vetter."])
        let s = await session(model)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?"))

        guard case .sources("r1", let sources)? = events.first else { return XCTFail("sources zuerst: \(events)") }
        XCTAssertEqual(sources.first?.itemId, "b1")
        let sentences = events.compactMap { e -> String? in if case .sentence(_, let t, _, _, _) = e { return t } else { return nil } }
        XCTAssertEqual(sentences, ["Castorp fährt nach Davos, um seinen Vetter zu besuchen.", "Joachim ist dort sein Vetter."])
        let statuses = events.compactMap { e -> AssistantSentenceStatus? in if case .sentence(_, _, _, let st, _) = e { return st } else { return nil } }
        XCTAssertEqual(statuses, [.grounded, .grounded], "rein belegt: keine Ansage")
        guard case .done("r1", let answer)? = events.last else { return XCTFail("done zuletzt: \(events)") }
        XCTAssertEqual(answer.text, sentences.joined(separator: " "))
        XCTAssertEqual(answer.sources, sources)
        XCTAssertFalse(answer.general)
        XCTAssertNotNil(answer.firstSentenceMs)
        XCTAssertGreaterThanOrEqual(answer.totalMs, answer.firstSentenceMs ?? 0)
        XCTAssertEqual(events.count, 1 + 2 + 1)
        // Reihenfolge: sources vor allen sentences.
        XCTAssertEqual(events.firstIndex { if case .sources = $0 { true } else { false } }, 0)
    }

    func testGeneralAnswerWithOptInIsAnnouncedAndMarkedUnsourced() async {
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc"))
        guard case .sources(_, let sources)? = events.first, case .done(_, let a)? = events.last else { return XCTFail("\(events)") }
        XCTAssertTrue(sources.isEmpty)
        XCTAssertTrue(a.general)
        XCTAssertTrue(a.unsourced)
        XCTAssertTrue(a.sources.isEmpty)
        XCTAssertEqual(a.counts, AssistantSentenceCounts(unsourced: 1))
        XCTAssertEqual(a.text, "Der Mont Blanc ist 4806 Meter hoch.", "Ansagen gehören nicht zum Antworttext")
    }

    func testSpeechFieldIsSpeakable() async {
        let s = await session(ScriptModel(pieces: ["**Das** gilt z. B. für Hunde 😀. Siehe [Link](https://example.com)."]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        let pairs = events.compactMap { e -> (String, String)? in
            if case .sentence(_, let t, let sp, let st, _) = e, st != .announcement { return (t, sp) } else { return nil }
        }
        XCTAssertEqual(pairs.map(\.0), ["Das gilt z. B. für Hunde.", "Siehe Link."])
        XCTAssertEqual(pairs.map(\.1), ["Das gilt zum Beispiel für Hunde.", "Siehe Link."])
    }

    func testPromptContainsInstructionsInventoryPassagesHistoryAndInterruption() async {
        let log = Log()
        let model = ScriptModel(pieces: ["Gut."], log: log)
        let hist = [AssistantTurn(user: "Frühere Frage", assistant: "Frühere Antwort", interrupted: true)]
        let s = await session(model, history: hist)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?", afterInterruption: true))
        let call = log.all.first
        XCTAssertEqual(call?.maxTokens, 400)
        XCTAssertTrue(call?.instructions.contains("Local Voice auf dem iPhone von Patrick") ?? false)
        let p = call?.prompt ?? ""
        XCTAssertTrue(p.contains("BESTAND:\nBücher: 1 insgesamt"), p)
        XCTAssertTrue(p.contains("[1] Buch · Der Zauberberg · S. 12"), p)
        XCTAssertTrue(p.contains("Assistent: Frühere Antwort …"), p)
        XCTAssertTrue(p.contains("Der Nutzer hat dich gerade unterbrochen."), p)
    }

    func testEventsDoNotWriteHistoryButCommitDoes() async {
        let s = await session(ScriptModel(pieces: ["Castorp fährt nach Davos."]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?"))
        guard case .done = events.last else { return XCTFail() }
        var h = await s.history
        XCTAssertTrue(h.isEmpty)
        await s.commit(requestId: "r1", user: "Wohin fährt Hans Castorp im Sanatorium nach Davos?", spoken: "Castorp fährt nach", interrupted: true)
        h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertEqual(h[0].user, "Wohin fährt Hans Castorp im Sanatorium nach Davos?")
        XCTAssertEqual(h[0].assistant, "Castorp fährt nach")
        XCTAssertTrue(h[0].interrupted)
        XCTAssertFalse(h[0].general)
        XCTAssertEqual(h[0].sources.first?.itemId, "b1")
        XCTAssertEqual(h[0].model, "Test")
    }

    func testCommitWithoutSourcesIsGeneralAndEmptyUserIsIgnored() async {
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc"))
        await s.commit(requestId: "r1", user: "Wie hoch ist der Mont Blanc", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        await s.commit(requestId: "r2", user: "   ", spoken: "egal", interrupted: false)
        await s.commit(requestId: "unbekannt", user: "Noch etwas", spoken: "Ein Satz", interrupted: true)
        let h = await s.history
        XCTAssertEqual(h.count, 2)
        XCTAssertTrue(h[0].general)
        XCTAssertTrue(h[0].unsourced)
        XCTAssertTrue(h[1].general)
        XCTAssertTrue(h[1].unsourced, "Gesprochenes ohne Entsprechung in der Prüfung gilt als unbelegt")
        XCTAssertTrue(h[1].interrupted)
    }

    /// Eine abgebrochene Frage ohne ein einziges gesprochenes Wort darf nicht als leerer Beitrag im Verlauf stehen.
    func testCommitRejectsEmptySpokenText() async {
        let s = await session(ScriptModel(pieces: ["Castorp fährt nach Davos."]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?"))
        await s.commit(requestId: "r1", user: "Wohin fährt Hans Castorp im Sanatorium nach Davos?", spoken: "", interrupted: true)
        await s.commit(requestId: "r1", user: "Wohin fährt Hans Castorp im Sanatorium nach Davos?", spoken: " \n ", interrupted: true)
        var h = await s.history
        XCTAssertTrue(h.isEmpty)
        // Die Quellen der verworfenen Anfrage sind verbraucht; ein späterer Beitrag erbt sie nicht.
        await s.commit(requestId: "r1", user: "Nochmal", spoken: "Ja", interrupted: false)
        h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertTrue(h[0].sources.isEmpty)
    }

    // MARK: Cloud-Schutz für den Verlauf

    private func historyForAreas() -> [AssistantTurn] {
        func source(_ area: AssistantArea) -> [AssistantPassage] {
            [AssistantPassage(area: area, itemId: "x-\(area.rawValue)", title: "Titel", text: "Text", score: 1)]
        }
        return [
            AssistantTurn(user: "FrageBuch", assistant: "AntwortBuch", sources: source(.books), general: false),
            AssistantTurn(user: "FrageAblage", assistant: "AntwortAblage", sources: source(.files), general: false),
            AssistantTurn(user: "FrageAllgemein", assistant: "AntwortAllgemein", general: true),
            AssistantTurn(user: "FrageGemischt", assistant: "AntwortGemischt", sources: source(.books) + source(.notes), general: false),
        ]
    }

    private var cloud: AssistantConfig {
        AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 16_000, maxAnswerTokens: 700, userName: "Patrick", timeout: 5)
    }

    /// Wissen mit dem Buch aus `knowledge()`, aber nur den gegebenen Bereichen freigegeben.
    private func knowledge(areas: Set<AssistantArea>) async -> AssistantKnowledge {
        let k = await knowledge()
        await k.setAreas(areas)
        return k
    }

    func testCloudPromptHistoryFiltersByAreasButKeepsHistoryComplete() async {
        let log = Log()
        let k = await knowledge(areas: [.books])
        let s = AssistantSession(model: ScriptModel(pieces: ["Gut."], log: log), knowledge: k, config: cloud, history: historyForAreas())
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo zusammen"))
        let p = log.all.last?.prompt ?? ""
        XCTAssertTrue(p.contains("AntwortBuch"), p)
        XCTAssertTrue(p.contains("AntwortAllgemein"), "Beitrag ohne Quellen ist erlaubt")
        XCTAssertFalse(p.contains("AntwortAblage"), "Ablage ist für die Cloud nicht freigegeben")
        XCTAssertFalse(p.contains("AntwortGemischt"), "ein gesperrter Bereich genügt, der ganze Beitrag bleibt draußen")
        var h = await s.history
        XCTAssertEqual(h.count, 4, "Anzeige und Datei behalten alles")

        // Die Bereiche ändern sich zwischen zwei Fragen: der Verlauf folgt sofort, ohne neue Sitzung.
        await k.setAreas([.books, .files, .notes])
        _ = await collect(s, AssistantRequest(requestId: "r2", text: "Hallo zusammen"))
        let all = log.all.last?.prompt ?? ""
        for word in ["AntwortBuch", "AntwortAblage", "AntwortAllgemein", "AntwortGemischt"] { XCTAssertTrue(all.contains(word), "\(word)\n\(all)") }
        await k.setAreas([])
        _ = await collect(s, AssistantRequest(requestId: "r3", text: "Hallo zusammen"))
        let none = log.all.last?.prompt ?? ""
        XCTAssertTrue(none.contains("AntwortAllgemein"))
        for word in ["AntwortBuch", "AntwortAblage", "AntwortGemischt"] { XCTAssertFalse(none.contains(word), word) }
        h = await s.history
        XCTAssertEqual(h.count, 4)
    }

    /// Lokale Modelle bekommen den ganzen Verlauf, unabhängig von den freigegebenen Bereichen.
    func testLocalPromptHistoryIsUnrestricted() async {
        let log = Log()
        let k = await knowledge(areas: [])
        let s = AssistantSession(model: ScriptModel(pieces: ["Gut."], log: log), knowledge: k, config: config, history: historyForAreas())
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo zusammen"))
        let p = log.all.last?.prompt ?? ""
        for word in ["AntwortBuch", "AntwortAblage", "AntwortAllgemein", "AntwortGemischt"] { XCTAssertTrue(p.contains(word), word) }
    }

    /// Ein Verlauf aus der Datei (Quellen ohne Auszugstext) wird genauso nach Bereichen gefiltert.
    func testCloudHistoryFromStoredTurnsIsFilteredToo() async {
        let log = Log()
        let stored = historyForAreas().map(AssistantHistoryStore.minimized)
        let k = await knowledge(areas: [.books])
        let s = AssistantSession(model: ScriptModel(pieces: ["Gut."], log: log), knowledge: k, config: cloud, history: stored)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo zusammen"))
        let p = log.all.last?.prompt ?? ""
        XCTAssertTrue(p.contains("AntwortBuch") && p.contains("AntwortAllgemein"), p)
        XCTAssertFalse(p.contains("AntwortAblage") || p.contains("AntwortGemischt"), p)
    }

    // MARK: Abbruch

    func testCancelByRequestIdYieldsExactlyOneCancelled() async {
        let log = Log()
        let model = ScriptModel(pieces: ["Erster Satz kommt hier. ", "Zweiter Satz folgt später. ", "Dritter Satz auch noch."], pauseMs: 150, log: log)
        let s = await session(model)
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas")) {
            events.append(e)
            if case .sentence = e { await s.cancel(requestId: "r1") }
        }
        XCTAssertEqual(events.filter { if case .failed(_, .cancelled) = $0 { true } else { false } }.count, 1)
        XCTAssertFalse(events.contains { if case .done = $0 { true } else { false } })
        XCTAssertEqual(events.last, .failed(requestId: "r1", .cancelled))
        let h = await s.history
        XCTAssertTrue(h.isEmpty)
        try? await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertGreaterThanOrEqual(log.terminations, 1, "Modellstrom wurde beendet")
    }

    func testCancelBeforeAnySentenceWhileModelHangs() async {
        let model = ScriptModel(pieces: [], hang: true)
        let s = await session(model)
        let task = Task { await collect(s, AssistantRequest(requestId: "r1", text: "Hallo")) }
        try? await Task.sleep(nanoseconds: 200_000_000)
        await s.cancel(requestId: "r1")
        let events = await task.value
        XCTAssertEqual(events.last, .failed(requestId: "r1", .cancelled))
        XCTAssertEqual(events.filter { if case .failed = $0 { true } else { false } }.count, 1)
    }

    func testCancelArrivingBeforeStartIsHonoured() async {
        let s = await session(ScriptModel(pieces: ["Ein Satz."]))
        await s.cancel(requestId: "r1")
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events, [.failed(requestId: "r1", .cancelled)])
    }

    func testCancellingTheConsumerTaskStopsTheModel() async {
        let log = Log()
        let model = ScriptModel(pieces: ["Ein Satz kommt. ", "Noch ein Satz folgt. ", "Und noch einer."], pauseMs: 200, log: log)
        let s = await session(model)
        let task = Task { () -> Int in
            var n = 0
            for await e in s.events(AssistantRequest(requestId: "r1", text: "Hallo")) { if case .sentence = e { n += 1 } }
            return n
        }
        try? await Task.sleep(nanoseconds: 350_000_000)
        task.cancel()
        _ = await task.value
        try? await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertGreaterThanOrEqual(log.terminations, 1)
        let h = await s.history
        XCTAssertTrue(h.isEmpty)
    }

    func testTimeoutFromConfig() async {
        var c = config
        c.timeout = 0.3
        let s = await session(ScriptModel(pieces: ["Halber Satz ohne Ende"], hang: true), config: c)
        let start = ContinuousClock.now
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events.last, .failed(requestId: "r1", .timeout))
        XCTAssertLessThan(start.duration(to: .now), .seconds(3))
        XCTAssertEqual(events.filter { if case .failed = $0 { true } else { false } }.count, 1)
    }

    func testSlowButSteadyStreamDoesNotTimeOut() async {
        var c = config
        c.timeout = 0.4
        let pieces = (0..<8).map { "Satz Nummer \($0) kommt. " }
        let s = await session(ScriptModel(pieces: pieces, pauseMs: 120), config: c)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        guard case .done(_, let a)? = events.last else { return XCTFail("\(events)") }
        XCTAssertGreaterThan(a.totalMs, 800)
    }

    // MARK: Fehler

    func testContextOverflowRetriesOnceWithHalfPassagesAndNoHistory() async {
        let log = Log()
        let model = OverflowModel(failures: 1, log: log)
        let hist = [AssistantTurn(user: "Frühere Frage", assistant: "Frühere Antwort")]
        let s = await session(model, history: hist)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?"))
        guard case .done = events.last else { return XCTFail("\(events)") }
        let calls = log.all
        XCTAssertEqual(calls.count, 2)
        XCTAssertTrue(calls[0].prompt.contains("VERLAUF:"))
        XCTAssertFalse(calls[1].prompt.contains("VERLAUF:"))
        XCTAssertTrue(events.contains { if case .sentence = $0 { true } else { false } })
    }

    func testContextOverflowTwiceFails() async {
        let log = Log()
        let s = await session(OverflowModel(failures: 5, log: log))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events.last, .failed(requestId: "r1", .modelFailed))
        XCTAssertEqual(log.all.count, 2, "genau eine Wiederholung")
    }

    func testEmptyModelAnswerFails() async {
        for pieces in [[], [""], ["   \n"], ["😀"], ["---"]] {
            let s = await session(ScriptModel(pieces: pieces))
            let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
            XCTAssertEqual(events.last, .failed(requestId: "r1", .modelFailed), "\(pieces)")
            XCTAssertFalse(events.contains { if case .done = $0 { true } else { false } })
        }
    }

    func testModelErrorsAreMapped() async {
        let cases: [(any Error, AssistantFailure)] = [
            (BookTalkModelError.offline, .offline), (BookTalkModelError.unavailable, .noModel), (BookTalkModelError.timeout, .timeout),
            (BookTalkModelError.notAllowed, .consentRequired), (NSError(domain: "x", code: 1), .modelFailed),
        ]
        for (error, expected) in cases {
            let s = await session(ScriptModel(pieces: [], fail: error))
            let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
            XCTAssertEqual(events.last, .failed(requestId: "r1", expected), "\(error)")
        }
    }

    func testErrorAfterSomeSentencesFailsInsteadOfDone() async {
        let s = await session(ScriptModel(pieces: ["Ein erster Satz. Und noch ein"], fail: BookTalkModelError.offline))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertTrue(events.contains { if case .sentence(_, "Ein erster Satz.", _, _, _) = $0 { true } else { false } })
        XCTAssertEqual(events.last, .failed(requestId: "r1", .offline))
    }

    // MARK: Gleichzeitigkeit

    func testSeveralRequestsRunConcurrentlyAndOnlyCommitWritesHistory() async {
        let model = ScriptModel(pieces: ["Erster Satz. ", "Zweiter Satz. ", "Dritter Satz."], pauseMs: 100)
        let s = await session(model)
        async let a = collect(s, AssistantRequest(requestId: "spec", text: "Wie wird das Wetter"))
        async let b = collect(s, AssistantRequest(requestId: "final", text: "Wie wird das Wetter morgen"))
        try? await Task.sleep(nanoseconds: 150_000_000)
        await s.cancel(requestId: "spec")
        let (ea, eb) = await (a, b)
        XCTAssertEqual(ea.last, .failed(requestId: "spec", .cancelled))
        guard case .done("final", _)? = eb.last else { return XCTFail("\(eb)") }
        var h = await s.history
        XCTAssertTrue(h.isEmpty)
        await s.commit(requestId: "final", user: "Wie wird das Wetter morgen", spoken: "Erster Satz. Zweiter Satz. Dritter Satz.", interrupted: false)
        h = await s.history
        XCTAssertEqual(h.count, 1)
    }

    // MARK: Antworten ohne Quelle (E4.2a)

    private struct Seen {
        let events: [AssistantEvent]
        var statuses: [AssistantSentenceStatus] {
            events.compactMap { if case .sentence(_, _, _, let st, _) = $0 { st } else { nil } }
        }
        var texts: [String] {
            events.compactMap { if case .sentence(_, let t, _, _, _) = $0 { t } else { nil } }
        }
        var spoken: [String] {
            events.compactMap { if case .sentence(_, let t, _, let st, _) = $0, st != .announcement { t } else { nil } }
        }
        var answer: AssistantAnswer? {
            if case .done(_, let a)? = events.last { a } else { nil }
        }
    }

    private func ask(_ pieces: [String], config c: AssistantConfig? = nil, text: String = "Wohin fährt Hans Castorp im Sanatorium nach Davos?",
                     session custom: AssistantSession? = nil) async -> Seen {
        let s: AssistantSession
        if let custom { s = custom } else { s = await session(ScriptModel(pieces: pieces), config: c ?? strict) }
        return Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: text)))
    }

    func testPurelyGroundedAnswerHasNoAnnouncementAndCarriesTheCoveringSource() async {
        let seen = await ask(["Castorp fährt nach Davos, um seinen Vetter zu besuchen."])
        XCTAssertEqual(seen.statuses, [.grounded])
        guard case .sentence(_, _, _, _, let sources)? = seen.events.first(where: { if case .sentence = $0 { true } else { false } }) else { return XCTFail() }
        XCTAssertEqual(sources.first?.itemId, "b1")
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1))
        XCTAssertEqual(seen.answer?.unsourced, false)
        XCTAssertEqual(seen.answer?.sources.map(\.itemId), ["b1"])
        XCTAssertEqual(seen.answer?.general, false)
    }

    func testMixedAnswerAnnouncesBothTransitions() async {
        let seen = await ask(["Castorp fährt nach Davos. Hunde schwitzen vor allem über die Pfoten. Joachim ist dort sein Vetter."], config: config)
        XCTAssertEqual(seen.statuses, [.grounded, .announcement, .unsourced, .announcement, .grounded])
        XCTAssertEqual(seen.texts, ["Castorp fährt nach Davos.", "Ohne Quelle:", "Hunde schwitzen vor allem über die Pfoten.",
                                    "Aus deinen Inhalten:", "Joachim ist dort sein Vetter."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 2, unsourced: 1))
        XCTAssertEqual(seen.answer?.unsourced, true)
        XCTAssertEqual(seen.answer?.text, "Castorp fährt nach Davos. Hunde schwitzen vor allem über die Pfoten. Joachim ist dort sein Vetter.",
                       "Ansagen gehören nicht zum Antworttext")
    }

    func testPurelyUnsourcedAnswerHasExactlyOneAnnouncementAtTheStart() async {
        let seen = await ask(["Hunde schwitzen vor allem über die Pfoten. Katzen putzen sich mehrmals am Tag."], config: config)
        XCTAssertEqual(seen.statuses, [.announcement, .unsourced, .unsourced])
        XCTAssertEqual(seen.texts.first, "Ohne Quelle:")
    }

    func testNeutralSentenceDoesNotChangeTheSectionButFirstUnsourcedStillGetsItsAnnouncement() async {
        let seen = await ask(["Gerne. Hunde schwitzen vor allem über die Pfoten. Soll ich mehr dazu sagen?"], config: config)
        XCTAssertEqual(seen.statuses, [.neutral, .announcement, .unsourced, .neutral])
    }

    func testMadeUpSourceIsNeverSpokenEvenWithOptIn() async {
        let seen = await ask(["Laut deiner Notiz Urlaubsplanung fährst du im Mai nach Italien. Hunde schwitzen vor allem über die Pfoten."], config: config)
        XCTAssertEqual(seen.statuses, [.announcement, .unsourced])
        XCTAssertFalse(seen.texts.joined().contains("Notiz"))
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 1, dropped: 1))
    }

    func testWrongAppClaimIsDroppedAndTheFixedSentenceFollowsWithoutOptIn() async {
        let seen = await ask(["Hans Castorp fährt nach Venedig."])
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.texts, ["Dazu finde ich in deinen Inhalten nichts. Antworten ohne Quelle kannst du im Menü einschalten."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1, dropped: 1))
        XCTAssertEqual(seen.answer?.text, seen.texts.first)
        XCTAssertEqual(seen.answer?.general, true)
        XCTAssertEqual(seen.answer?.unsourced, false)
    }

    func testOptOutDropsEveryUnsourcedSentenceAndSaysTheFixedSentenceOnce() async {
        let seen = await ask(["Der Mont Blanc ist 4806 Meter hoch. Er liegt in den Alpen. Hunde schwitzen über die Pfoten."], text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(seen.spoken, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1, dropped: 3))
    }

    func testOptInWithNothingGroundedSaysTheShorterFixedSentence() async {
        let seen = await ask(["Hans Castorp fährt nach Venedig."], config: config)
        XCTAssertEqual(seen.spoken, ["Dazu finde ich nichts Belegtes."])
    }

    func testOptInWithRemainingUnsourcedSentenceHasNoFixedSentence() async {
        let seen = await ask(["Hans Castorp fährt nach Venedig. Hunde schwitzen über die Pfoten."], config: config)
        XCTAssertEqual(seen.statuses, [.announcement, .unsourced])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 1, dropped: 1))
    }

    func testNeutralOnlyAnswerPassesWithoutFixedSentence() async {
        let seen = await ask(["Hallo! Wie kann ich dir helfen?"], text: "Hallo")
        XCTAssertEqual(seen.statuses, [.neutral, .neutral])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 2))
    }

    func testModelsOwnNothingFoundTemplateCountsAsAnswerAndGetsNoSecondSentence() async {
        let seen = await ask(["Dazu finde ich in deinen Inhalten nichts."], text: "Was steht in Kapitel 99?")
        XCTAssertEqual(seen.texts, ["Dazu finde ich in deinen Inhalten nichts."])
    }

    func testInventoryAnswerIsGroundedWithoutPassageSources() async {
        let seen = await ask(["Du hast 1 Buch."], text: "Welche Bücher habe ich?")
        XCTAssertEqual(seen.statuses, [.grounded])
        XCTAssertEqual(seen.answer?.sources, [])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1))
        let wrong = await ask(["Du hast 5 Bücher."], text: "Welche Bücher habe ich?")
        XCTAssertEqual(wrong.statuses, [.neutral], "falsche Zahl: verworfen, fester Satz")
    }

    func testClockAnswerIsNeutralAndAllowedWithoutOptIn() async {
        let date = AssistantPrompt.germanDate(Date(), timeZone: .current)
        let day = String(date[..<date.lastIndex(of: ",")!])
        let seen = await ask(["Heute ist \(day)."], text: "Welcher Tag ist heute?")
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1))
    }

    func testAnnouncementsAreNotPartOfSpeechHistoryText() async {
        let s = await session(ScriptModel(pieces: ["Castorp fährt nach Davos. Hunde schwitzen vor allem über die Pfoten."]), config: config)
        let seen = await ask([], session: s)
        let spoken = seen.spoken.joined(separator: " ")
        await s.commit(requestId: "r1", user: "Frage", spoken: spoken, interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertFalse(h[0].assistant.contains("Ohne Quelle:"))
        XCTAssertEqual(h[0].assistant, "Castorp fährt nach Davos. Hunde schwitzen vor allem über die Pfoten.")
    }

    // Widerruf

    func testRevokingOptInMidStreamDropsFurtherUnsourcedSentences() async {
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. ", "Katzen putzen sich mehrmals am Tag. ", "Pferde schlafen oft im Stehen."], pauseMs: 150)
        let s = await session(model, config: config)
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            events.append(e)
            if case .sentence(_, _, _, .unsourced, _) = e { s.setAllowUnsourced(false) }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.spoken, ["Hunde schwitzen vor allem über die Pfoten."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 1, dropped: 2, revoked: 2))
        let allows = s.allowsUnsourced
        XCTAssertFalse(allows)
        // Eine spätere Frage mit widerrufenem Opt-in läuft strikt.
        let next = await ask([], session: s)
        XCTAssertFalse(next.statuses.contains(.unsourced))
    }

    func testRevokingAnAreaMidStreamDropsFurtherGroundedSentencesOfThatArea() async {
        let k = await knowledge()
        let model = ScriptModel(pieces: ["Castorp fährt nach Davos. ", "Joachim ist dort sein Vetter. "], pauseMs: 150)
        let s = AssistantSession(model: model, knowledge: k, config: strict)
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?")) {
            events.append(e)
            if case .sentence(_, _, _, .grounded, _) = e { await k.setAreas([.notes]) }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.spoken, ["Castorp fährt nach Davos."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1, dropped: 1, revoked: 1))
    }

    func testRevokedOptInIsReportedAsRevokedInTheAuditLog() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-session-audit-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. ", "Katzen putzen sich mehrmals am Tag."], pauseMs: 150)
        let s = AssistantSession(model: model, knowledge: await knowledge(), config: config, audit: audit)
        var spoken: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            if case .sentence(_, let t, _, let st, _) = e, st != .announcement { spoken.append(t) }
            if case .sentence(_, _, _, .unsourced, _) = e { s.setAllowUnsourced(false) }
        }
        let wrote = await s.commit(requestId: "r1", user: "Erzähl etwas über Tiere", spoken: spoken.joined(separator: " "), interrupted: false)
        XCTAssertTrue(wrote)
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].outcome, .revoked)
        XCTAssertEqual(entries[0].unsourced, 1)
        XCTAssertEqual(entries[0].dropped, 1)
        XCTAssertTrue(entries[0].unsourcedMode)
    }

    // Beitrag

    private func twoSourceKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [
            ("S. 12", "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."),
        ]))
        await corpus.upsert(assistantDoc("t1", "Besprechung", area: .transcripts, sections: [
            ("00:14:03", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    func testCommitKeepsOnlySourcesThatCoverASpokenSentenceAndMarksMixedParts() async {
        let s = AssistantSession(model: ScriptModel(pieces: ["Castorp fährt nach Davos. Hunde schwitzen vor allem über die Pfoten."]),
                                 knowledge: await twoSourceKnowledge(), config: config)
        let question = "Hans Castorp Davos Sanatorium Anna Planung Projekt Nordlicht"
        let seen = await ask([], text: question, session: s)
        guard case .sources(_, let retrieved)? = seen.events.first else { return XCTFail() }
        XCTAssertEqual(Set(retrieved.map(\.itemId)), ["b1", "t1"], "beide wurden abgerufen")
        await s.commit(requestId: "r1", user: question, spoken: seen.spoken.joined(separator: " "), interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.last?.sources.map(\.itemId), ["b1"], "nur die deckende Quelle")
        XCTAssertEqual(h.last?.unsourced, true)
        XCTAssertEqual(h.last?.general, false)
        XCTAssertEqual(h.last?.parts, [AssistantTurnPart(text: "Castorp fährt nach Davos.", unsourced: false),
                                       AssistantTurnPart(text: "Hunde schwitzen vor allem über die Pfoten.", unsourced: true)])
    }

    func testInterruptedAfterTheGroundedSentenceIsNotMarkedUnsourced() async {
        let s = AssistantSession(model: ScriptModel(pieces: ["Castorp fährt nach Davos. Hunde schwitzen vor allem über die Pfoten."]),
                                 knowledge: await twoSourceKnowledge(), config: config)
        _ = await ask([], text: "Hans Castorp Davos Sanatorium", session: s)
        await s.commit(requestId: "r1", user: "Hans Castorp Davos Sanatorium", spoken: "Castorp fährt nach Davos.", interrupted: true)
        let h = await s.history
        XCTAssertEqual(h.last?.unsourced, false)
        XCTAssertEqual(h.last?.parts, [])
        XCTAssertEqual(h.last?.sources.map(\.itemId), ["b1"])
        XCTAssertEqual(h.last?.interrupted, true)
    }

    func testPurelyUnsourcedTurnIsMarkedWithoutParts() async {
        let s = await session(ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten."]), config: config)
        let seen = await ask([], text: "Wie schwitzen Hunde?", session: s)
        await s.commit(requestId: "r1", user: "Wie schwitzen Hunde?", spoken: seen.spoken.joined(separator: " "), interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.last?.unsourced, true)
        XCTAssertEqual(h.last?.general, true)
        XCTAssertEqual(h.last?.parts, [])
    }

    // Prompt

    func testPromptInstructionsFollowTheOptInAtRequestTime() async {
        let log = Log()
        let s = await session(ScriptModel(pieces: ["Gut."], log: log), config: strict)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertTrue(log.all.last?.instructions.contains("Antworte ausschließlich aus BESTAND und AUSZÜGEN") ?? false)
        s.setAllowUnsourced(true)
        _ = await collect(s, AssistantRequest(requestId: "r2", text: "Hallo"))
        XCTAssertTrue(log.all.last?.instructions.contains("Sätze aus Allgemeinwissen stehen nie im selben Satz") ?? false)
    }

    func testBlankQuestionStillAnswers() async {
        let s = await session(ScriptModel(pieces: ["Ja bitte?"]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: ""))
        guard case .done = events.last else { return XCTFail("\(events)") }
    }
}
