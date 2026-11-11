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
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick", timeout: 5)

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
        let model = ScriptModel(pieces: ["Castorp fährt nach Davos", ", um seinen Vetter zu besuchen. Er bleibt", " dort länger als geplant."])
        let s = await session(model)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wohin fährt Hans Castorp im Sanatorium nach Davos?"))

        guard case .sources("r1", let sources)? = events.first else { return XCTFail("sources zuerst: \(events)") }
        XCTAssertEqual(sources.first?.itemId, "b1")
        let sentences = events.compactMap { e -> String? in if case .sentence(_, let t, _) = e { return t } else { return nil } }
        XCTAssertEqual(sentences, ["Castorp fährt nach Davos, um seinen Vetter zu besuchen.", "Er bleibt dort länger als geplant."])
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

    func testGeneralWhenNoPassagesUsed() async {
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc"))
        guard case .sources(_, let sources)? = events.first, case .done(_, let a)? = events.last else { return XCTFail("\(events)") }
        XCTAssertTrue(sources.isEmpty)
        XCTAssertTrue(a.general)
        XCTAssertTrue(a.sources.isEmpty)
    }

    func testSpeechFieldIsSpeakable() async {
        let s = await session(ScriptModel(pieces: ["**Das** gilt z. B. für Hunde 😀. Siehe [Link](https://example.com)."]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        let pairs = events.compactMap { e -> (String, String)? in if case .sentence(_, let t, let sp) = e { return (t, sp) } else { return nil } }
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
        XCTAssertTrue(h[1].general)
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
        XCTAssertTrue(events.contains { if case .sentence(_, "Ein erster Satz.", _) = $0 { true } else { false } })
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

    func testBlankQuestionStillAnswers() async {
        let s = await session(ScriptModel(pieces: ["Ja bitte?"]))
        let events = await collect(s, AssistantRequest(requestId: "r1", text: ""))
        guard case .done = events.last else { return XCTFail("\(events)") }
    }
}
