import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K, E4.2a in Paket A): Ablauf einer Antwort mit künstlichen Modellen.
// Datenrunde: Das Modell wählt, das System gibt Originalsätze aus. Allgemeinrunde: nur mit Opt-in, Prompt ohne Auszüge.

private final class Log: @unchecked Sendable {
    private let lock = NSLock()
    private var calls: [(instructions: String, prompt: String, maxTokens: Int)] = []
    private var ended = 0
    private var delivered = 0
    func hit(_ i: String, _ p: String, _ m: Int) { lock.lock(); calls.append((i, p, m)); lock.unlock() }
    func terminated() { lock.lock(); ended += 1; lock.unlock() }
    func piece() { lock.lock(); delivered += 1; lock.unlock() }
    var all: [(instructions: String, prompt: String, maxTokens: Int)] { lock.lock(); defer { lock.unlock() }; return calls }
    var terminations: Int { lock.lock(); defer { lock.unlock() }; return ended }
    var pieces: Int { lock.lock(); defer { lock.unlock() }; return delivered }
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
                    log.piece()
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

/// Was eine Anfrage ausgegeben hat, mit Zugriff auf die Satz-Ereignisse.
private struct Seen {
    struct Sentence: Equatable {
        let text: String
        let status: AssistantSentenceStatus
        let sources: [AssistantPassage]
        let quote: Bool
        let generation: Int
    }

    let events: [AssistantEvent]

    var sentences: [Sentence] {
        events.compactMap { e in
            if case .sentence(_, let g, let t, _, let st, let src, let q) = e { Sentence(text: t, status: st, sources: src, quote: q, generation: g) } else { nil }
        }
    }
    var statuses: [AssistantSentenceStatus] { sentences.map(\.status) }
    var texts: [String] { sentences.map(\.text) }
    /// Ausgegebene Sätze ohne Ansagen.
    var spoken: [String] { sentences.filter { $0.status != .announcement }.map(\.text) }
    var answer: AssistantAnswer? { if case .done(_, let a)? = events.last { a } else { nil } }
    var failure: AssistantFailure? { if case .failed(_, let f)? = events.last { f } else { nil } }
    var retrieved: [AssistantPassage]? { if case .sources(_, _, let p)? = events.first { p } else { nil } }
    var hasDone: Bool { events.contains { if case .done = $0 { true } else { false } } }
}

private let castorp = "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."
private let stay = "Er bleibt drei Wochen."
/// Ein Modellsatz, der auf `castorp` einrastet (andere Wörter, gleiche Inhalte).
private let castorpModel = "Hans Castorp fährt nach Davos und besucht seinen Vetter Joachim."
private let question = "Wohin fährt Hans Castorp im Sanatorium nach Davos?"
private let preface = "Aus deinen Inhalten, ‚Der Zauberberg‘:"
private let mont = "Der Mont Blanc ist 4806 Meter hoch."

final class AssistantSessionTests: XCTestCase {
    /// Opt-in an: Allgemeinrunden sind möglich (Fragen ohne Bezug zu den Inhalten).
    private let open = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick",
                                       timeout: 5, allowUnsourced: true, consentVersion: 1)
    /// Opt-in aus (Standard): jede Runde ist eine Datenrunde.
    private let strict = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick",
                                         timeout: 5)
    private let cloud = AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 16_000, maxAnswerTokens: 700, userName: "Patrick",
                                        timeout: 5)

    private func knowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorp + " " + stay)]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func twoSourceKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorp + " " + stay)]))
        await corpus.upsert(assistantDoc("t1", "Besprechung", area: .transcripts, sections: [
            ("00:14:03", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func session(_ model: any BookTalkModel, history: [AssistantTurn] = [], config: AssistantConfig? = nil,
                         audit: AssistantAuditLog? = nil) async -> AssistantSession {
        AssistantSession(model: model, knowledge: await knowledge(), config: config ?? strict, history: history, audit: audit, released: true)
    }

    private func run(_ pieces: [String], config c: AssistantConfig? = nil, text: String = question, id: String = "r1",
                     history: [AssistantTurn] = []) async -> Seen {
        let s = await session(ScriptModel(pieces: pieces), history: history, config: c ?? strict)
        return Seen(events: await collect(s, AssistantRequest(requestId: id, text: text)))
    }

    private func tempDir() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("assistant-session-\(UUID().uuidString)", isDirectory: true)
    }

    // MARK: Datenrunde: Originalsätze

    func testDataRoundOutputsTheSourceSentenceWithPrefaceAndSource() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        let g = s.releaseGeneration
        let events = await collect(s, AssistantRequest(requestId: "r1", text: question))
        let seen = Seen(events: events)

        guard case .sources("r1", let gen, let sources)? = events.first else { return XCTFail("sources zuerst: \(events)") }
        XCTAssertEqual(gen, g)
        XCTAssertEqual(sources.map(\.itemId), ["b1"])
        XCTAssertEqual(seen.statuses, [.announcement, .grounded])
        XCTAssertEqual(seen.texts, [preface, castorp], "ausgegeben wird der Quellsatz, nicht der Modellsatz")
        XCTAssertEqual(seen.sentences.map(\.quote), [false, true])
        XCTAssertEqual(seen.sentences[1].sources.map(\.itemId), ["b1"])
        XCTAssertEqual(seen.sentences[1].sources.first?.locator, "S. 12")
        XCTAssertEqual(Set(seen.sentences.map(\.generation)), [g])
        guard let answer = seen.answer else { return XCTFail("done zuletzt: \(events)") }
        XCTAssertEqual(answer.text, castorp, "Ansagen gehören nicht zum Antworttext")
        XCTAssertEqual(answer.sources.map(\.itemId), ["b1"])
        XCTAssertEqual(answer.counts, AssistantSentenceCounts(grounded: 1))
        XCTAssertEqual(answer.round, .data)
        XCTAssertEqual(answer.generation, g)
        XCTAssertFalse(answer.general)
        XCTAssertFalse(answer.unsourced)
        XCTAssertNotNil(answer.firstSentenceMs)
        XCTAssertGreaterThanOrEqual(answer.totalMs, answer.firstSentenceMs ?? 0)
        XCTAssertEqual(events.firstIndex { if case .sources = $0 { true } else { false } }, 0, "sources vor allen Sätzen")
    }

    func testDataRoundIsTheSameWithAndWithoutOptIn() async {
        let off = await run([castorpModel], config: strict)
        let on = await run([castorpModel], config: open)
        XCTAssertEqual(off.texts, on.texts)
        XCTAssertEqual(on.answer?.round, .data, "Frage mit Abruftreffer ist eine Datenrunde")
        XCTAssertEqual(on.answer?.counts, AssistantSentenceCounts(grounded: 1))
    }

    func testQuoteSpeechFieldIsSpeakable() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Gartenpflege", area: .notes, sections: [
            (nil, "Der Zaun misst 3.5 Meter und braucht jedes Jahr frische Farbe."),
        ]))
        let s = AssistantSession(model: ScriptModel(pieces: ["Der Zaun misst 3.5 Meter und braucht jedes Jahr frische Farbe."]),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Wie viele Meter misst der Zaun und welche Farbe braucht er?"))
        let pairs = events.compactMap { e -> (String, String)? in
            if case .sentence(_, _, let t, let sp, .grounded, _, _) = e { (t, sp) } else { nil }
        }
        XCTAssertEqual(pairs.map(\.0), ["Der Zaun misst 3.5 Meter und braucht jedes Jahr frische Farbe."], "Originalwortlaut")
        XCTAssertEqual(pairs.map(\.1), ["Der Zaun misst 3,5 Meter und braucht jedes Jahr frische Farbe."], "Dezimalkomma nur für die Stimme")
    }

    /// J: Der Modelltext über die Quelle wird nie wörtlich gesprochen; kein Teil wird vor dem Satzende ausgegeben.
    func testNothingIsOutputBeforeTheSentenceEndsAndTheInventedTailIsNeverSpoken() async {
        let log = Log()
        let pieces = ["In deinem Buch fährt Hans Castorp nach Davos, ", "aber nur in einer erfundenen ", "Traumsequenz."]
        for config in [strict, open] {
            let s = await session(ScriptModel(pieces: pieces, pauseMs: 80, log: log), config: config)
            var firstEventPieces: Int?
            var events: [AssistantEvent] = []
            let before = log.pieces
            for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
                if case .sentence = e, firstEventPieces == nil { firstEventPieces = log.pieces - before }
                events.append(e)
            }
            XCTAssertEqual(firstEventPieces, 3, "erst nach dem letzten Stück, also nach dem Satzende")
            let seen = Seen(events: events)
            XCTAssertEqual(seen.spoken, [castorp])
            XCTAssertFalse(seen.texts.joined().contains("Traumsequenz"))
            XCTAssertFalse(seen.texts.joined().contains("erfunden"))
        }
    }

    func testModelParaphraseOfTheSourceIsReplacedByTheSourceSentence() async {
        let seen = await run(["Joachim besucht seinen Vetter Hans in Davos."])
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1))
    }

    func testSecondSentenceOfTheSourceCanBeQuotedToo() async {
        let seen = await run([castorpModel + " ", "Er bleibt drei Wochen dort."])
        XCTAssertEqual(seen.spoken, [castorp, stay])
        XCTAssertEqual(seen.statuses, [.announcement, .grounded, .grounded], "ein Vorspann je Quelle")
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 2))
        XCTAssertEqual(seen.answer?.sources.count, 1)
    }

    // MARK: Fragmente (H3)

    /// Ein Rest ohne Satzzeichen am Stromende wird verworfen (Grund fragment), in beiden Rundenarten; er wird nie ausgegeben.
    func testRestWithoutPunctuationAtTheStreamEndIsNeverOutputInEitherRoundKind() async {
        let rest = "Anna übernimmt künftig"
        let data = await run([castorpModel + " " + rest])
        XCTAssertEqual(data.spoken, [castorp])
        XCTAssertFalse(data.texts.joined().contains("künftig"))
        XCTAssertEqual(data.answer?.counts, AssistantSentenceCounts(grounded: 1, droppedFragment: 1))

        let general = await run([mont + " " + rest], config: open, text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(general.answer?.round, .general)
        XCTAssertEqual(general.spoken, [mont])
        XCTAssertFalse(general.texts.joined().contains("künftig"))
        XCTAssertEqual(general.answer?.counts, AssistantSentenceCounts(unsourced: 1, droppedFragment: 1))

        // Nur der Rest: nichts wird ausgegeben, der feste Satz sagt es (kein Fehler des Modells).
        let onlyData = await run([rest], text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(onlyData.texts, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
        XCTAssertEqual(onlyData.answer?.counts, AssistantSentenceCounts(neutral: 1, droppedFragment: 1))
        let onlyGeneral = await run([rest], config: open, text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(onlyGeneral.texts, [AssistantGrounding.fixedSentence(allowUnsourced: true)])
        XCTAssertEqual(onlyGeneral.answer?.counts, AssistantSentenceCounts(neutral: 1, droppedFragment: 1))
        XCTAssertNil(onlyGeneral.failure)
    }

    func testDroppedRestIsInTheJournalAsFragment() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: [castorpModel + " Anna übernimmt künftig"]), config: strict, audit: audit)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        let e = await audit.entries().first
        XCTAssertEqual(e?.trace.map(\.verdict), [.announcement, .grounded, .droppedFragment])
        XCTAssertEqual(e?.counts.droppedFragment, 1)
        XCTAssertFalse(rawLog(audit).contains("künftig"))
    }

    /// Ein ganzer Auszug ohne Satzzeichen („Anna für Projekt Nordlicht“) ist nicht zitierbar, auch nicht als einziger Satz.
    func testWholeExcerptWithoutPunctuationIsNeverQuoted() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Notiz", area: .notes, sections: [(nil, "Anna für Projekt Nordlicht")]))
        let k = AssistantKnowledge(corpus: corpus)
        let s = AssistantSession(model: ScriptModel(pieces: ["Anna übernimmt die Leitung für das Projekt Nordlicht."]), knowledge: k, config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wer macht das Projekt Nordlicht?")))
        XCTAssertFalse(seen.texts.contains("Anna für Projekt Nordlicht"))
        XCTAssertFalse(seen.statuses.contains(.grounded))
        XCTAssertEqual(seen.texts, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
        XCTAssertEqual(seen.answer?.sources, [])
    }

    /// Ohne nachgewiesene Satzgrenze (angeschnittener Auszuganfang oder -ende) werden erster und letzter Satz nicht zitiert; mit
    /// Nachweis schon.
    func testCutExcerptEdgeSentencesAreNotQuotedButProvenOnesAre() async {
        let text = "Die Sitzung beginnt um zehn Uhr im großen Saal. Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag. Bernd kümmert sich um das Budget des Projekts Nordlicht."
        for (proven, expected) in [(false, ["Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."]),
                                   (true, ["Die Sitzung beginnt um zehn Uhr im großen Saal."])] {
            let corpus = AssistantCorpus()
            await corpus.upsert(assistantDoc("t1", "Besprechung", area: .transcripts, boundaries: proven, sections: [("00:01", text)]))
            let s = AssistantSession(model: ScriptModel(pieces: [proven ? "Die Sitzung beginnt um zehn Uhr im großen Saal im Haus." : "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."]),
                                     knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
            let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wann beginnt die Sitzung und wer plant das Projekt Nordlicht?")))
            XCTAssertEqual(seen.spoken, expected, "Auszug mit Nachweis: \(proven)")
        }
        // Der Satz am angeschnittenen Rand rastet nicht ein.
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("t1", "Besprechung", area: .transcripts, boundaries: false, sections: [("00:01", text)]))
        let s = AssistantSession(model: ScriptModel(pieces: ["Bernd kümmert sich um das Budget des Projekts Nordlicht."]),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Wer kümmert sich um das Budget?")))
        XCTAssertFalse(seen.texts.contains { $0.contains("Bernd kümmert") })
    }

    // MARK: Datenrunde: Eigenschaft über böswillige Modellausgaben

    private let hostile: [String] = [
        "Wenn du magst, deine Notiz ist gefälscht.",
        "Deine Notiz ist falsch, ich bin mir nicht sicher.",
        "Warum ist deine Notiz gefälscht?",
        "Deine Mail wurde um 08:05 Uhr gelöscht.",
        "Hans Castorp fährt nach venedig.",
        "Eva übernimmt die Planung bis Freitag.",
        "Ihre Termine wurden abgesagt.",
        "Deine Kalendertermine wurden abgesagt.",
        "Deine Korrespondenz wurde gelöscht.",
        "Dein K\u{0430}lender wurde gelöscht.",
        "Er hat gekündigt.",
        "In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.",
        "Laut deiner Notiz Urlaubsplanung fährst du im Mai nach Italien.",
        "Aus deinen Inhalten, ‚Der Zauberberg‘: Hans ist tot.",
        "Ohne Quelle: Hunde schwitzen über die Pfoten.",
        "Dazu finde ich in deinen Inhalten nichts.",
        "Dazu finde ich nichts Belegtes.",
        "Hallo! Du hast 3 Bücher.",
        "Ja. Nein.",
        "Soll ich dir die Stelle vorlesen? Das ist deine Entscheidung. Möchtest du mehr dazu hören?",
        "Du solltest das Buch zuerst lesen und danach deine Notizen ordnen.",
        "**Hans Castorp** fährt nach Davos. Dort wartet [Joachim](https://example.com) auf ihn.",
        "Es ist 99:99 Uhr.",
        "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim, der gestern verstorben ist.",
        "Der Mont Blanc ist 4806 Meter hoch. Hunde schwitzen über die Pfoten. Katzen putzen sich täglich.",
        "Seite 12: Castorp fährt nach Davos.",
        "Er bleibt drei Wochen. Er bleibt drei Wochen. Er bleibt drei Wochen.",
    ]

    /// Eigenschaft: In Datenrunden ist jeder ausgegebene Satz zeichengleich mit einem Quellsatz, einer Systemvorlage oder dem
    /// festen Satz, jede Ansage ist genau der Vorspann der Quelle. Mit und ohne Opt-in, nie ein Satz ohne Quelle.
    func testPropertyDataRoundsOnlyOutputSourceSentencesTemplatesAndPrefaces() async {
        let sourceSentences: Set<String> = [castorp, stay]
        let systemSentences = Set(AssistantGrounding.templateSentences + [AssistantGrounding.fixedSentence(allowUnsourced: false),
                                                                           AssistantGrounding.fixedSentence(allowUnsourced: true)])
        for config in [strict, open] {
            for text in hostile {
                let seen = await run([text], config: config)
                XCTAssertEqual(seen.answer?.round, .data, text)
                for s in seen.sentences {
                    switch s.status {
                    case .announcement:
                        XCTAssertEqual(s.text, preface, "\(text) → \(s.text)")
                    case .grounded:
                        XCTAssertTrue(sourceSentences.contains(s.text), "\(text) → \(s.text)")
                        XCTAssertTrue(s.quote)
                        XCTAssertFalse(s.sources.isEmpty)
                    case .neutral:
                        XCTAssertTrue(systemSentences.contains(s.text), "\(text) → \(s.text)")
                    case .unsourced, .inventory:
                        XCTFail("in einer Datenrunde nie \(s.status): \(text) → \(s.text)")
                    }
                }
                XCTAssertFalse(seen.spoken.isEmpty, "es bleibt immer etwas Sprechbares (mindestens der feste Satz): \(text)")
                XCTAssertFalse(seen.statuses.contains(.unsourced))
            }
        }
    }

    func testMixedAnswerNeverMixesGroundedAndUnsourcedSentences() async {
        let mixed = "Hans Castorp fährt nach Davos. Der Mont Blanc ist 4806 Meter hoch. Er bleibt drei Wochen. Deine Notiz ist falsch."
        let cases: [(AssistantConfig, String)] = [
            (strict, question), (open, question), (open, "Wie hoch ist der Mont Blanc?"), (strict, "Wie hoch ist der Mont Blanc?"),
            (open, "Wohin fährt Hans Castorp?"), (open, "Allgemein: Wohin fährt Hans Castorp im Sanatorium nach Davos?"),
        ]
        for (config, text) in cases {
            let seen = await run([mixed], config: config, text: text)
            let set = Set(seen.statuses)
            XCTAssertFalse(set.contains(.grounded) && set.contains(.unsourced), "\(text): \(seen.statuses)")
            switch seen.answer?.round {
            case .data?: XCTAssertFalse(set.contains(.unsourced), text)
            case .general?: XCTAssertFalse(set.contains(.grounded), text)
            case nil: XCTFail("keine Antwort für \(text): \(seen.events)")
            }
            XCTAssertFalse(seen.texts.contains { $0.contains("Notiz") }, "Eigendatenbezug nie gesprochen: \(text)")
        }
    }

    // MARK: Datenrunde: Rückfall, fester Satz, Obergrenzen

    func testNothingSnapsFallsBackToTheBestSourceSentenceAgainstTheQuestion() async {
        let seen = await run(["Das ist völlig unklar."])
        XCTAssertEqual(seen.statuses, [.announcement, .grounded])
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1, droppedNoMode: 1))
    }

    func testNothingSnapsAndNoSourceFitsTheQuestionSaysTheFixedSentence() async {
        let seen = await run(["Der Mont Blanc ist 4806 Meter hoch."], text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.texts, ["Dazu finde ich in deinen Inhalten nichts. Antworten ohne Quelle kannst du im Menü einschalten."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1, droppedNoMode: 1))
        XCTAssertEqual(seen.answer?.text, seen.texts.first)
        XCTAssertEqual(seen.answer?.general, true)
        XCTAssertEqual(seen.answer?.round, .data)
    }

    func testFixedSentenceWithOptInInADataRoundHasNoHintButSaysNothingGrounded() async {
        // Eigendatenbezug in der Frage erzwingt die Datenrunde auch mit Opt-in; im Abruf steht nichts dazu.
        let seen = await run(["Der Mont Blanc ist 4806 Meter hoch."], config: open, text: "Was steht in meiner Notiz über den Mont Blanc?")
        XCTAssertEqual(seen.answer?.round, .data)
        XCTAssertEqual(seen.texts, ["Dazu finde ich nichts Belegtes."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1, droppedNotSnapped: 1))
    }

    func testOwnDataClaimInTheModelTextIsCountedAndNeverSpoken() async {
        let seen = await run(["Laut deiner Notiz Urlaubsplanung fährst du im Mai nach Italien."], config: open)
        XCTAssertEqual(seen.answer?.round, .data, "Eigendatenbezug in der Frage: Datenrunde")
        XCTAssertEqual(seen.answer?.counts.droppedOwnData, 1)
        XCTAssertFalse(seen.texts.joined().contains("Italien"))
        XCTAssertFalse(seen.statuses.contains(.unsourced))
    }

    func testModelsOwnNothingFoundTemplateIsDroppedAndTheSystemSaysIt() async {
        let seen = await run(["Dazu finde ich in deinen Inhalten nichts."], text: "Was steht in Kapitel 99?")
        XCTAssertEqual(seen.answer?.counts.droppedTemplate, 1)
        XCTAssertEqual(seen.texts, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
    }

    func testAtMostThreeQuotesThenTheModelIsStopped() async {
        let sentences = [
            castorp, stay, "Joachim Ziemßen ist Soldat und kränkelt schon lange.",
            "Der Hofrat Behrens leitet das Sanatorium oben auf dem Berg.", "Madame Chauchat wohnt im Sanatorium und isst am Tisch der Russen.",
        ]
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", sentences.joined(separator: " "))]))
        let log = Log()
        let s = AssistantSession(model: ScriptModel(pieces: sentences.map { $0 + " " }, pauseMs: 20, hang: true, log: log),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        let start = ContinuousClock.now
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Hans Castorp Davos Sanatorium Hofrat Behrens Chauchat Berg")))
        XCTAssertLessThan(start.duration(to: .now), .seconds(3), "das hängende Modell wurde abgebrochen, nicht abgewartet")
        XCTAssertEqual(seen.spoken, Array(sentences.prefix(3)))
        XCTAssertEqual(seen.statuses, [.announcement, .grounded, .grounded, .grounded])
        XCTAssertTrue(seen.hasDone)
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 3))
        try? await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertGreaterThanOrEqual(log.terminations, 1, "Modellstrom wurde beendet")
    }

    func testDuplicateQuotesAreDroppedAndCounted() async {
        let seen = await run([castorpModel + " ", "Hans Castorp reist nach Davos in das Sanatorium zu seinem Vetter Joachim."])
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1, droppedDuplicate: 1))
    }

    func testPrefaceComesAtEverySourceChange() async {
        let s = AssistantSession(model: ScriptModel(pieces: [castorpModel + " ", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag. ",
                                                              "Er bleibt drei Wochen."]),
                                 knowledge: await twoSourceKnowledge(), config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Hans Castorp Davos Sanatorium Anna Planung Projekt Nordlicht")))
        XCTAssertEqual(Set(seen.retrieved?.map(\.itemId) ?? []), ["b1", "t1"])
        XCTAssertEqual(seen.texts, [
            preface, castorp,
            "Aus deinen Inhalten, ‚Besprechung‘:", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.",
            preface, stay,
        ], "A, B, A: nach dem Wechsel zurück zu A kommt A erneut als Vorspann, sonst wäre das Zitat falsch zugeordnet")
        XCTAssertEqual(seen.sentences.filter { $0.status == .grounded }.map { $0.sources.first?.itemId }, ["b1", "t1", "b1"])
        XCTAssertEqual(seen.answer?.sources.map(\.itemId).sorted(), ["b1", "t1"])
    }

    func testPrefaceTitleIsCleanedAndShortened() async {
        let corpus = AssistantCorpus()
        let longTitle = "Ein ‚sehr‘ \"langer\" Titel\nmit Zeilenumbruch " + String(repeating: "wort ", count: 40)
        await corpus.upsert(assistantDoc("n1", longTitle, area: .notes, sections: [(nil, "Der Zaun braucht jedes Jahr frische Farbe im Frühling.")]))
        let s = AssistantSession(model: ScriptModel(pieces: ["Der Zaun braucht jedes Jahr frische Farbe im Frühling."]),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Welche Farbe braucht der Zaun im Frühling?")))
        guard let first = seen.sentences.first, first.status == .announcement else { return XCTFail("\(seen.events)") }
        XCTAssertTrue(first.text.hasPrefix("Aus deinen Inhalten, ‚Ein sehr langer Titel mit Zeilenumbruch"), first.text)
        XCTAssertTrue(first.text.hasSuffix("…‘:"), first.text)
        XCTAssertLessThanOrEqual(first.text.count, "Aus deinen Inhalten, ‚".count + AssistantGrounding.maxPrefaceTitle + "‘:".count)
        XCTAssertFalse(first.text.contains("\n"))
    }

    // MARK: Bestandsangaben (vom System)

    func testInventoryQuestionIsAnsweredBySystemFromInventoryDataWithoutAModelCall() async {
        let log = Log()
        let s = await session(ScriptModel(pieces: ["Du hast 5 Bücher."], log: log), config: open)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Welche Bücher habe ich?")))
        XCTAssertEqual(log.all.count, 0, "kein Modellaufruf")
        XCTAssertEqual(seen.retrieved, [])
        XCTAssertEqual(seen.statuses, [.announcement, .inventory, .inventory])
        XCTAssertEqual(seen.texts, ["Aus deinen Inhalten:", "Du hast ein Buch.", "Neueste Bücher: Der Zauberberg."])
        XCTAssertEqual(seen.sentences.map(\.quote), [false, false, false])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(inventory: 2))
        XCTAssertEqual(seen.answer?.round, .data)
        XCTAssertEqual(seen.answer?.sources, [])
        XCTAssertEqual(seen.answer?.unsourced, false)
        XCTAssertEqual(seen.answer?.text, "Du hast ein Buch. Neueste Bücher: Der Zauberberg.")
    }

    func testInventoryQuestionWithoutAnEnabledAreaSaysTheFixedSentenceStillWithoutAModel() async {
        let log = Log()
        let k = await knowledge()
        await k.setAreas([.notes])
        for (config, expected) in [(strict, AssistantGrounding.fixedSentence(allowUnsourced: false)),
                                   (open, AssistantGrounding.fixedSentence(allowUnsourced: true))] {
            let s = AssistantSession(model: ScriptModel(pieces: ["Du hast ein Buch."], log: log), knowledge: k, config: config, released: true)
            let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Welche Bücher habe ich?")))
            XCTAssertEqual(seen.texts, [expected])
            XCTAssertEqual(seen.statuses, [.neutral])
        }
        XCTAssertEqual(log.all.count, 0)
    }

    func testInventoryOfAnEnabledAreaNamesNothingFromOtherAreas() async {
        let k = await twoSourceKnowledge()
        await k.setAreas([.transcripts])
        let s = AssistantSession(model: ScriptModel(pieces: []), knowledge: k, config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Was habe ich gespeichert?")))
        XCTAssertEqual(seen.spoken, ["Du hast ein Transkript.", "Neueste Transkripte: Besprechung."])
        XCTAssertFalse(seen.texts.joined().contains("Zauberberg"))
    }

    // MARK: Allgemeinrunde

    func testGeneralRoundPromptHasNoExcerptsNoInventoryAndHistoryOnlyFromGeneralRounds() async {
        let log = Log()
        let hist = [
            AssistantTurn(user: "Frage zum Buch", assistant: "Hans Castorp fährt nach Davos.", round: .data),
            AssistantTurn(user: "Alte Frage ohne Rundenart", assistant: "Alte Antwort mit Joachim."),
            AssistantTurn(user: "Wie hoch ist die Zugspitze?", assistant: "Rund 2962 Meter.", round: .general),
        ]
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."], log: log), history: hist, config: open)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?")))
        let call = log.all.first
        let p = call?.prompt ?? ""
        XCTAssertFalse(p.contains("AUSZÜGE"), p)
        XCTAssertFalse(p.contains("BESTAND"), p)
        for word in ["Castorp", "Joachim", "Zauberberg", "Davos", "Frage zum Buch"] { XCTAssertFalse(p.contains(word), "\(word)\n\(p)") }
        XCTAssertTrue(p.contains("Nutzer: Wie hoch ist die Zugspitze?"), p)
        XCTAssertTrue(call?.instructions.contains("In dieser Runde siehst du keine Inhalte des Nutzers") ?? false)
        XCTAssertFalse(call?.instructions.contains("AUSZÜGE") ?? true)
        XCTAssertEqual(seen.retrieved, [], "in einer Allgemeinrunde keine Auszüge")
        XCTAssertEqual(seen.statuses, [.announcement, .unsourced])
        XCTAssertEqual(seen.texts, ["Ohne Quelle:", "Der Mont Blanc ist 4806 Meter hoch."])
        XCTAssertEqual(seen.sentences.map(\.quote), [false, false])
        XCTAssertEqual(seen.answer?.round, .general)
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 1))
        XCTAssertEqual(seen.answer?.unsourced, true)
        XCTAssertEqual(seen.answer?.text, "Der Mont Blanc ist 4806 Meter hoch.", "Ansagen gehören nicht zum Antworttext")
        XCTAssertEqual(seen.answer?.sources, [])
    }

    func testGeneralRoundOnlyAnnouncesOnceAndNeverQuotes() async {
        let seen = await run(["Der Mont Blanc ist 4806 Meter hoch. Er liegt in den Alpen. " + castorp], config: open, text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(seen.statuses, [.announcement, .unsourced, .unsourced], "der dritte Satz ist ein Satz aus dem eigenen Buch: verworfen")
        XCTAssertFalse(seen.statuses.contains(.grounded), "kein Satz einer Allgemeinrunde gilt je als belegt")
        XCTAssertEqual(seen.sentences.filter { $0.quote }.count, 0)
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 2, droppedOwnData: 1))
    }

    /// Zweite Schicht (c): Ein Satz ohne Quelle, der einem Quellsatz ähnelt (Einrast-Regel), wird verworfen (Grund ownData), auch
    /// wenn weder Stamm noch Titel ihn verraten. Quelle: „Anna übernimmt die Planung …“, Satz: „Anna übernimmt die Leitung.“
    func testGeneralRoundDropsASentenceThatResemblesOwnContent() async {
        let s = AssistantSession(model: ScriptModel(pieces: ["Anna übernimmt die Leitung. Der Mont Blanc ist 4806 Meter hoch."]),
                                 knowledge: await twoSourceKnowledge(), config: open, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?")))
        XCTAssertEqual(seen.answer?.round, .general)
        XCTAssertEqual(seen.texts, ["Ohne Quelle:", "Der Mont Blanc ist 4806 Meter hoch."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 1, droppedOwnData: 1))
    }

    /// Dasselbe Verfahren sieht nur die freigegebenen Bereiche: Ist der Bereich der Quelle aus, ist der Satz kein Eigeninhalt.
    func testResemblanceCheckOnlySeesEnabledAreas() async {
        let k = await twoSourceKnowledge()
        await k.setAreas([.books])
        let s = AssistantSession(model: ScriptModel(pieces: ["Anna übernimmt die Leitung."]), knowledge: k, config: open, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?")))
        XCTAssertEqual(seen.statuses, [.announcement, .unsourced])
    }

    func testGeneralRoundDropsEverySentenceThatRefersToOwnData() async {
        let seen = await run(["Deine Notiz ist falsch. Der Mont Blanc ist 4806 Meter hoch. Deine Mail wurde um 08:05 Uhr gelöscht."],
                             config: open, text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(seen.texts, ["Ohne Quelle:", "Der Mont Blanc ist 4806 Meter hoch."])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(unsourced: 1, droppedOwnData: 2))
    }

    func testGeneralRoundWithOnlyOwnDataSentencesSaysTheFixedSentence() async {
        let seen = await run(["Deine Notiz ist falsch."], config: open, text: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(seen.texts, ["Dazu finde ich nichts Belegtes."])
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1, droppedOwnData: 1))
        XCTAssertEqual(seen.answer?.unsourced, false)
    }

    func testGeneralRoundTemplatesAreNeutralWithoutAnAnnouncement() async {
        let seen = await run(["Hallo! Wie kann ich dir helfen?"], config: open, text: "Hallo")
        XCTAssertEqual(seen.statuses, [.neutral, .neutral])
        XCTAssertEqual(seen.answer?.round, .general)
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 2))
    }

    func testClockAnswerIsASystemSentenceAndAllowedWithoutOptIn() async {
        let date = AssistantPrompt.germanDate(Date(), timeZone: .current)
        let day = String(date[..<date.lastIndex(of: ",")!])
        let seen = await run(["Heute ist \(day)."], text: "Welcher Tag ist heute?")
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(neutral: 1))
        let wrong = await run(["Heute ist Montag, der 1. Januar 2001."], text: "Welcher Tag ist heute?")
        XCTAssertEqual(wrong.answer?.counts.neutral, 1, "falsches Datum: verworfen, fester Satz")
        XCTAssertEqual(wrong.answer?.counts.dropped, 1)
        XCTAssertEqual(wrong.texts, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
    }

    /// „Allgemein:“ vor der Frage schaltet nichts um: Ein relevanter Abruftreffer macht die Runde zur Datenrunde (und rastet ein).
    func testAGeneralPrefixDoesNotMakeAGeneralRoundWhenThereIsAHit() async {
        let log = Log()
        let s = await session(ScriptModel(pieces: [castorpModel], log: log), config: open)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Allgemein: Wohin fährt Hans Castorp im Sanatorium nach Davos?")))
        XCTAssertEqual(seen.answer?.round, .data)
        XCTAssertTrue(log.all.first?.prompt.contains("AUSZÜGE") ?? false)
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(s.contextMode, .data, "die Runde rastet den Zustand ein")
        let own = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Allgemein gefragt: was steht in meiner Notiz?")))
        XCTAssertEqual(own.answer?.round, .data, "Eigendatenbezug und Sperre")
    }

    // MARK: Rundenart und Vorrunde

    func testFollowUpAfterADataRoundStaysADataRoundCommittedOrNot() async {
        for commitFirst in [false, true] {
            let log = Log()
            let s = await session(ScriptModel(pieces: [castorpModel], log: log), config: open)
            let first = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: question)))
            XCTAssertEqual(first.answer?.round, .data)
            if commitFirst { await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false) }
            let second = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Und was macht er dort?")))
            XCTAssertEqual(second.answer?.round, .data, "anaphorische Anschlussfrage nach einer Datenrunde, commit=\(commitFirst)")
            XCTAssertTrue(log.all.last?.instructions.contains("Unter AUSZÜGE stehen") ?? false)
            XCTAssertFalse(second.statuses.contains(.unsourced))
        }
    }

    func testFollowUpAfterAGeneralRoundStaysGeneral() async {
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), config: open)
        let first = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?")))
        XCTAssertEqual(first.answer?.round, .general)
        let second = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Und wie alt ist er?")))
        XCTAssertEqual(second.answer?.round, .general, "keine Übernahme aus einer Allgemeinrunde")
    }

    func testRestoredHistoryWithoutRoundCountsAsDataAndClearForgets() async {
        let old = [AssistantTurn(user: "Frage", assistant: "Antwort")]
        let s = await session(ScriptModel(pieces: ["Er hat gekündigt."]), history: old, config: open)
        let carried = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Er hat gekündigt.")))
        XCTAssertEqual(carried.answer?.round, .data, "ein Beitrag ohne Rundenart gilt als Datenrunde")
        await s.clearHistory()
        let cleared = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Er hat gekündigt.")))
        XCTAssertEqual(cleared.answer?.round, .general, "nach „Neues Gespräch“ keine Vorrunde")
        await s.restore(history: old)
        let restored = Seen(events: await collect(s, AssistantRequest(requestId: "r3", text: "Er hat gekündigt.")))
        XCTAssertEqual(restored.answer?.round, .data)
    }

    // MARK: Beitrag und Verlauf

    func testEventsDoNotWriteHistoryButCommitDoes() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: question)))
        XCTAssertTrue(seen.hasDone)
        var h = await s.history
        XCTAssertTrue(h.isEmpty)
        await s.commit(requestId: "r1", user: question, spoken: "Hans Castorp fährt ins Sanatorium nach", interrupted: true)
        h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertEqual(h[0].user, question)
        XCTAssertEqual(h[0].assistant, "Hans Castorp fährt ins Sanatorium nach")
        XCTAssertTrue(h[0].interrupted)
        XCTAssertFalse(h[0].general)
        XCTAssertEqual(h[0].sources.first?.itemId, "b1")
        XCTAssertEqual(h[0].model, "Test")
        XCTAssertEqual(h[0].round, .data)
        let label = h[0].sources.first?.sourceLabel
        XCTAssertTrue(label?.hasPrefix("Buch · ") == true, "die Quellzeile des Zitats kommt aus dem Auszug")
        XCTAssertEqual(h[0].parts, [AssistantTurnPart(text: "Hans Castorp fährt ins Sanatorium nach", unsourced: false, quote: true, source: label)],
                       "das Gesprochene ist ein angesprochener Originalsatz und bleibt ein Zitat, mit seiner Quelle")
        XCTAssertFalse(h[0].unsourced)
    }

    func testCommitOfAGeneralRoundMarksTheTurnUnsourcedAndGeneral() async {
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), config: open)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?"))
        await s.commit(requestId: "r1", user: "Wie hoch ist der Mont Blanc?", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertTrue(h[0].general)
        XCTAssertTrue(h[0].unsourced)
        XCTAssertEqual(h[0].round, .general)
        XCTAssertEqual(h[0].parts, [], "rein unbelegt: keine Abschnitte")
        XCTAssertTrue(h[0].sources.isEmpty)
    }

    func testCommitIgnoresEmptyUserEmptySpokenTextAndUnknownRequests() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: "", interrupted: true)
        var h = await s.history
        XCTAssertTrue(h.isEmpty, "keine leeren Beiträge")
        _ = await collect(s, AssistantRequest(requestId: "r2", text: question))
        await s.commit(requestId: "r2", user: "   ", spoken: "egal", interrupted: false)
        await s.commit(requestId: "unbekannt", user: "Noch etwas", spoken: "Ein Satz", interrupted: true)
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        h = await s.history
        XCTAssertTrue(h.isEmpty, "unbekannt oder schon abgeschlossen: nichts")
    }

    func testCommitTwiceWritesOneTurn() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
    }

    func testAttributeMapsSpokenTextToTheClassifiedSentencesAndNeverToUnsourced() {
        var record = AssistantRunRecord()
        let passage = AssistantPassage(area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: castorp, score: 1)
        record.sentences = [
            AssistantEmitted(text: castorp, status: .grounded, sources: [passage], quote: true),
            AssistantEmitted(text: "Hallo!", status: .neutral, sources: [], quote: false),
        ]
        let full = AssistantSession.attribute(spoken: castorp + " Hallo!", record: record)
        XCTAssertEqual(full.map(\.status), [.grounded, .neutral])
        XCTAssertEqual(full.map(\.quote), [true, false])
        let cut = AssistantSession.attribute(spoken: "Hans Castorp fährt ins", record: record)
        XCTAssertEqual(cut.map(\.text), ["Hans Castorp fährt ins"])
        XCTAssertEqual(cut.map(\.status), [.grounded], "ein angesprochener Satz zählt mit seiner Einordnung")
        let extra = AssistantSession.attribute(spoken: castorp + " Und noch etwas Unbekanntes.", record: record)
        XCTAssertEqual(extra.last?.status, .neutral, "Gesprochenes ohne Entsprechung wird nie als „ohne Quelle“ gewertet")
        XCTAssertEqual(extra.last?.sources, [])
        XCTAssertEqual(AssistantSession.attribute(spoken: "Irgendwas", record: nil).map(\.status), [.neutral])
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

    /// Wissen mit dem Buch aus `knowledge()`, aber nur den gegebenen Bereichen freigegeben.
    private func knowledge(areas: Set<AssistantArea>) async -> AssistantKnowledge {
        let k = await knowledge()
        await k.setAreas(areas)
        return k
    }

    func testCloudPromptHistoryFiltersByAreasButKeepsHistoryComplete() async {
        let log = Log()
        let k = await knowledge(areas: [.books])
        let s = AssistantSession(model: ScriptModel(pieces: ["Gut."], log: log), knowledge: k, config: cloud, history: historyForAreas(), released: true)
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
        let s = AssistantSession(model: ScriptModel(pieces: ["Gut."], log: log), knowledge: k, config: strict, history: historyForAreas(), released: true)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo zusammen"))
        let p = log.all.last?.prompt ?? ""
        for word in ["AntwortBuch", "AntwortAblage", "AntwortAllgemein", "AntwortGemischt"] { XCTAssertTrue(p.contains(word), word) }
    }

    /// Ein Verlauf aus der Datei (Quellen ohne Auszugstext) wird genauso nach Bereichen gefiltert.
    func testCloudHistoryFromStoredTurnsIsFilteredToo() async {
        let log = Log()
        let stored = historyForAreas().map(AssistantHistoryStore.minimized)
        let k = await knowledge(areas: [.books])
        let s = AssistantSession(model: ScriptModel(pieces: ["Gut."], log: log), knowledge: k, config: cloud, history: stored, released: true)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo zusammen"))
        let p = log.all.last?.prompt ?? ""
        XCTAssertTrue(p.contains("AntwortBuch") && p.contains("AntwortAllgemein"), p)
        XCTAssertFalse(p.contains("AntwortAblage") || p.contains("AntwortGemischt"), p)
    }

    /// Eine Cloud-Allgemeinrunde sieht nur frühere Allgemeinrunden, auch wenn der Bereich freigegeben ist.
    func testCloudGeneralRoundSeesOnlyEarlierGeneralRounds() async {
        let log = Log()
        var c = cloud
        c.allowUnsourced = true
        let hist = historyForAreas() + [AssistantTurn(user: "FrageLaufend", assistant: "AntwortLaufend", round: .general)]
        let s = AssistantSession(model: ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."], log: log), knowledge: await knowledge(),
                                 config: c, history: hist, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?")))
        XCTAssertEqual(seen.answer?.round, .general)
        let p = log.all.last?.prompt ?? ""
        XCTAssertTrue(p.contains("AntwortLaufend"), p)
        for word in ["AntwortBuch", "AntwortAblage", "AntwortAllgemein", "AntwortGemischt", "AUSZÜGE"] { XCTAssertFalse(p.contains(word), word) }
    }

    // MARK: Abbruch

    func testCancelByRequestIdYieldsExactlyOneCancelled() async {
        let log = Log()
        let model = ScriptModel(pieces: ["Erster Satz kommt hier. ", "Zweiter Satz folgt später. ", "Dritter Satz auch noch."], pauseMs: 150, log: log)
        let s = await session(model, config: open)
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
        let s = await session(ScriptModel(pieces: [], hang: true), config: open)
        let task = Task { await collect(s, AssistantRequest(requestId: "r1", text: "Hallo")) }
        try? await Task.sleep(nanoseconds: 200_000_000)
        await s.cancel(requestId: "r1")
        let events = await task.value
        XCTAssertEqual(events.last, .failed(requestId: "r1", .cancelled))
        XCTAssertEqual(events.filter { if case .failed = $0 { true } else { false } }.count, 1)
    }

    func testCancelArrivingBeforeStartIsHonoured() async {
        let s = await session(ScriptModel(pieces: ["Ein Satz."]), config: open)
        await s.cancel(requestId: "r1")
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events, [.failed(requestId: "r1", .cancelled)])
    }

    func testCancellingTheConsumerTaskStopsTheModel() async {
        let log = Log()
        let model = ScriptModel(pieces: ["Ein Satz kommt. ", "Noch ein Satz folgt. ", "Und noch einer."], pauseMs: 200, log: log)
        let s = await session(model, config: open)
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
        var c = open
        c.timeout = 0.3
        let s = await session(ScriptModel(pieces: ["Halber Satz ohne Ende"], hang: true), config: c)
        let start = ContinuousClock.now
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events.last, .failed(requestId: "r1", .timeout))
        XCTAssertLessThan(start.duration(to: .now), .seconds(3))
        XCTAssertEqual(events.filter { if case .failed = $0 { true } else { false } }.count, 1)
    }

    func testSlowButSteadyStreamDoesNotTimeOut() async {
        var c = open
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
        let hist = [AssistantTurn(user: "Frühere Frage", assistant: "Frühere Antwort")]
        let s = await session(OverflowModel(failures: 1, log: log), history: hist, config: open)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: question)))
        XCTAssertTrue(seen.hasDone, "\(seen.events)")
        let calls = log.all
        XCTAssertEqual(calls.count, 2)
        XCTAssertTrue(calls[0].prompt.contains("VERLAUF:"))
        XCTAssertFalse(calls[1].prompt.contains("VERLAUF:"))
        XCTAssertFalse(seen.sentences.isEmpty)
    }

    func testContextOverflowTwiceFails() async {
        let log = Log()
        let s = await session(OverflowModel(failures: 5, log: log), config: open)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events.last, .failed(requestId: "r1", .modelFailed))
        XCTAssertEqual(log.all.count, 2, "genau eine Wiederholung")
    }

    func testEmptyModelAnswerFails() async {
        for pieces in [[], [""], ["   \n"], ["😀"], ["---"]] {
            let s = await session(ScriptModel(pieces: pieces), config: open)
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
            let s = await session(ScriptModel(pieces: [], fail: error), config: open)
            let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
            XCTAssertEqual(events.last, .failed(requestId: "r1", expected), "\(error)")
        }
    }

    func testErrorAfterSomeSentencesFailsInsteadOfDone() async {
        let s = await session(ScriptModel(pieces: ["Ein erster Satz. Und noch ein"], fail: BookTalkModelError.offline), config: open)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Hallo")))
        XCTAssertTrue(seen.spoken.contains("Ein erster Satz."), "\(seen.events)")
        XCTAssertEqual(seen.failure, .offline)
        XCTAssertFalse(seen.hasDone)
    }

    func testBlankQuestionStillAnswers() async {
        let seen = await run(["Hallo!"], config: open, text: "")
        XCTAssertTrue(seen.hasDone, "\(seen.events)")
    }

    // MARK: Gleichzeitigkeit

    func testSeveralRequestsRunConcurrentlyAndOnlyCommitWritesHistory() async {
        let model = ScriptModel(pieces: ["Erster Satz. ", "Zweiter Satz. ", "Dritter Satz."], pauseMs: 100)
        let s = await session(model, config: open)
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

    // MARK: Widerruf mit Generation

    func testGenerationStartsMonotoneAndChangesOnlyWithARealChange() async {
        let s = await session(ScriptModel(pieces: []), config: strict)
        let g0 = s.releaseGeneration
        XCTAssertEqual(s.setAllowUnsourced(false), g0, "keine Änderung: keine neue Generation")
        let g1 = s.setAllowUnsourced(true)
        XCTAssertGreaterThan(g1, g0)
        XCTAssertTrue(s.allowsUnsourced)
        let g2 = s.setAllowUnsourced(false)
        let g3 = s.setAllowUnsourced(true)
        XCTAssertTrue(g1 < g2 && g2 < g3, "Aus und wieder Ein gibt nie eine alte Generation zurück")
        XCTAssertEqual(s.releaseGeneration, g3)
        XCTAssertGreaterThan(s.releaseDidChange(), g3, "Cloud-Zustimmung: jede Änderung zählt")
        let before = s.releaseGeneration
        let after = await s.setAreas([.books])
        XCTAssertGreaterThanOrEqual(after, before + 2, "Generation vor und nach dem Ändern der Bereiche")
        XCTAssertEqual(s.releaseGeneration, after)
    }

    func testRevokingOptInMidStreamStopsTheAnswerAndNothingOldComesOutAfterwards() async {
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. Katzen", " putzen sich mehrmals am Tag. ", "Pferde schlafen oft im Stehen."], pauseMs: 150)
        let s = await session(model, config: open)
        let g0 = s.releaseGeneration
        var events: [AssistantEvent] = []
        var after: [AssistantEvent] = []
        var bumped = false
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            events.append(e)
            if bumped { after.append(e) }
            if case .sentence(_, _, _, _, .unsourced, _, _) = e, !bumped {
                bumped = true
                s.setAllowUnsourced(false)
            }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.spoken, ["Hunde schwitzen vor allem über die Pfoten."])
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertFalse(seen.hasDone)
        XCTAssertEqual(after.count, 1, "nach dem Widerruf nur noch das Ende: \(after)")
        XCTAssertEqual(after.first, .failed(requestId: "r1", .revoked))
        XCTAssertEqual(Set(seen.sentences.map(\.generation)), [g0])
        XCTAssertGreaterThan(s.releaseGeneration, g0)
        XCTAssertFalse(s.allowsUnsourced)
        // Eine spätere Frage mit widerrufenem Opt-in läuft als Datenrunde.
        let next = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Erzähl etwas über Tiere")))
        XCTAssertEqual(next.answer?.round, .data)
        XCTAssertFalse(next.statuses.contains(.unsourced))
    }

    func testOffThenOnDoesNotReviveTheOldRun() async {
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. Katzen", " putzen sich mehrmals am Tag."], pauseMs: 150)
        let s = await session(model, config: open)
        let g0 = s.releaseGeneration
        var events: [AssistantEvent] = []
        var flipped = false
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            events.append(e)
            if case .sentence(_, _, _, _, .unsourced, _, _) = e, !flipped {
                flipped = true
                s.setAllowUnsourced(false)
                s.setAllowUnsourced(true)
            }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.spoken, ["Hunde schwitzen vor allem über die Pfoten."], "Aus und wieder Ein belebt den alten Lauf nicht")
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertGreaterThan(s.releaseGeneration, g0 + 1)
        let fresh = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Erzähl etwas über Tiere")))
        XCTAssertEqual(Set(fresh.sentences.map(\.generation)), [s.releaseGeneration], "ein neuer Lauf trägt die neue Generation")
    }

    func testChangingTheAreasMidStreamRevokesTheRun() async {
        let k = await knowledge()
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], pauseMs: 150)
        let s = AssistantSession(model: model, knowledge: k, config: strict, released: true)
        let g0 = s.releaseGeneration
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            events.append(e)
            if case .sentence(_, _, _, _, .grounded, _, _) = e { await s.setAreas([.notes]) }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertGreaterThanOrEqual(s.releaseGeneration, g0 + 2)
        let areas = await k.areas
        XCTAssertEqual(areas, [.notes])
    }

    func testCloudConsentChangeRevokesRunningAnswers() async {
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], pauseMs: 150)
        let s = await session(model, config: strict)
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            events.append(e)
            if case .sentence(_, _, _, _, .grounded, _, _) = e { s.releaseDidChange() }
        }
        XCTAssertEqual(Seen(events: events).failure, .revoked)
        XCTAssertEqual(Seen(events: events).spoken, [castorp])
    }

    func testRevokedAnswerIsNeverWrittenToTheHistory() async {
        let s = await session(ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], pauseMs: 150), config: strict)
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            if case .sentence(_, _, _, _, .grounded, _, _) = e { s.releaseDidChange() }
        }
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        let h = await s.history
        XCTAssertTrue(h.isEmpty, "Gesprochenes unter veralteter Generation kommt nicht in den Verlauf")
    }

    func testStaleCommitAfterACompleteRunIsDiscardedAndLogged() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: [castorpModel]), config: strict, audit: audit)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: question)))
        XCTAssertTrue(seen.hasDone)
        let started = seen.answer?.generation ?? -1
        let now = s.setAllowUnsourced(true)
        XCTAssertGreaterThan(now, started)
        let wrote = await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        XCTAssertTrue(wrote)
        let h = await s.history
        XCTAssertTrue(h.isEmpty)
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].history, AssistantCommitMark(stage: .history, ok: false, generation: now))
        XCTAssertEqual(entries[0].outcome, .revoked)
        XCTAssertEqual(entries[0].generation, started, "die Generation beim Start der Anfrage")
    }

    // MARK: Commit und Lauf

    func testCommitWhileTheRunIsStillGoingCancelsItAndWaitsForTheOrderedEnd() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], pauseMs: 250, hang: true)
        let s = await session(model, config: strict, audit: audit)
        let task = Task { await collect(s, AssistantRequest(requestId: "r1", text: question)) }
        try? await Task.sleep(nanoseconds: 400_000_000)   // der erste Satz ist da, der zweite noch nicht
        let start = ContinuousClock.now
        let wrote = await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: true)
        XCTAssertTrue(wrote)
        XCTAssertLessThan(start.duration(to: .now), .seconds(2), "kein Warten auf das hängende Modell")
        let seen = Seen(events: await task.value)
        XCTAssertEqual(seen.failure, .cancelled)
        XCTAssertEqual(seen.spoken, [castorp])

        let h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertEqual(h[0].sources.map(\.itemId), ["b1"], "der Satz ist als Originalsatz belegt, nicht als unbelegt")
        XCTAssertFalse(h[0].unsourced)
        XCTAssertEqual(h[0].parts, [AssistantTurnPart(text: castorp, unsourced: false, quote: true, source: h[0].sources.first?.sourceLabel)])
        XCTAssertNotNil(h[0].parts.first?.source)
        XCTAssertTrue(h[0].interrupted)
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 1, "genau ein Eintrag je Anfrage")
        XCTAssertEqual(entries[0].outcome, .interrupted)
        XCTAssertEqual(entries[0].counts.grounded, 1)
        XCTAssertEqual(entries[0].areas, ["books"])
        XCTAssertEqual(entries[0].history?.ok, true)

        // Ein zweiter commit zur selben Anfrage ergibt weder Beitrag noch Eintrag.
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: true)
        let again = await s.history
        XCTAssertEqual(again.count, 1)
        let entriesAgain = await audit.entries()
        XCTAssertEqual(entriesAgain.count, 1)
    }

    func testCommitWaitsForTheFinalStateAndDoesNotMisattributeGroundedSentences() async {
        // Der Lauf endet erst nach dem Abbruch durch commit; der Stand wird vor dem Schreiben festgehalten.
        let s = await session(ScriptModel(pieces: [castorpModel + " Das", " ist völlig unklar. "], pauseMs: 100, hang: true), config: open)
        let task = Task { await collect(s, AssistantRequest(requestId: "r1", text: question)) }
        try? await Task.sleep(nanoseconds: 350_000_000)
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: true)
        _ = await task.value
        let h = await s.history
        XCTAssertEqual(h.first?.unsourced, false)
        XCTAssertEqual(h.first?.sources.map(\.itemId), ["b1"])
    }

    // MARK: Protokoll

    private func rawLog(_ audit: AssistantAuditLog) -> String {
        (try? String(contentsOf: audit.url, encoding: .utf8)) ?? ""
    }

    func testAuditEntryExistsForARequestWithoutSpokenText() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: [], fail: BookTalkModelError.offline), config: strict, audit: audit)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: question)))
        XCTAssertEqual(seen.failure, .offline)
        let wrote = await s.commit(requestId: "r1", user: question, spoken: "", interrupted: false)
        XCTAssertTrue(wrote)
        let h = await s.history
        XCTAssertTrue(h.isEmpty, "kein Beitrag ohne Gesprochenes")
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].kind, .answer)
        XCTAssertEqual(entries[0].outcome, .failed)
        XCTAssertEqual(entries[0].round, .data)
        XCTAssertEqual(entries[0].level, .s0)
        XCTAssertEqual(entries[0].counts, AssistantSentenceCounts())
        XCTAssertNil(entries[0].history)
        XCTAssertEqual(entries[0].modelKind, .other)
        XCTAssertFalse(entries[0].cloud)
    }

    func testAuditEntryForAnEarlyCancelAndNoSecondEntryFromCommit() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: ["Ein Satz."]), config: strict, audit: audit)
        await s.cancel(requestId: "r1")
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events, [.failed(requestId: "r1", .cancelled)])
        await s.commit(requestId: "r1", user: "Hallo", spoken: "", interrupted: false)
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].outcome, .interrupted)
        XCTAssertNil(entries[0].round, "die Rundenart wurde nicht mehr festgelegt")
    }

    /// Verworfene Vorarbeit (Controller: `cancel`, dann `commit` mit leerem Text): genau ein Eintrag je Anfrage, nie ein Beitrag,
    /// ob der Lauf schon fertig war oder noch hing.
    func testAbandonedSpeculationWritesExactlyOneEntryAndNoTurn() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)

        let done = await session(ScriptModel(pieces: [castorpModel]), config: strict, audit: audit)
        _ = await collect(done, AssistantRequest(requestId: "fertig", text: question))
        await done.cancel(requestId: "fertig")
        await done.commit(requestId: "fertig", user: question, spoken: "", interrupted: true)
        await done.commit(requestId: "fertig", user: question, spoken: "", interrupted: true)   // zweites Mal: kein zweiter Eintrag
        let finished = await audit.entries()
        XCTAssertEqual(finished.count, 1)
        XCTAssertEqual(finished[0].outcome, .interrupted)
        XCTAssertEqual(finished[0].round, .data)
        let doneHistory = await done.history
        XCTAssertTrue(doneHistory.isEmpty)

        let hanging = await session(ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], pauseMs: 250, hang: true),
                                    config: strict, audit: audit)
        let task = Task { await collect(hanging, AssistantRequest(requestId: "haengt", text: question)) }
        try? await Task.sleep(nanoseconds: 400_000_000)
        await hanging.cancel(requestId: "haengt")
        let wrote = await hanging.commit(requestId: "haengt", user: question, spoken: "", interrupted: true)
        XCTAssertTrue(wrote)
        let seen = Seen(events: await task.value)
        XCTAssertEqual(seen.failure, .cancelled)
        let all = await audit.entries()
        XCTAssertEqual(all.count, 2, "ein Eintrag je Anfrage")
        XCTAssertEqual(all[1].outcome, .interrupted)
        let hangingHistory = await hanging.history
        XCTAssertTrue(hangingHistory.isEmpty)
    }

    func testAuditEntryOfADataRoundHasRoundLevelCountersAndTheSentenceTrace() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: ["Deine Notiz ist falsch. " + castorpModel + " Ihre Termine wurden abgesagt."]),
                              config: strict, audit: audit)
        let g = s.releaseGeneration
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: question)))
        XCTAssertEqual(seen.spoken, [castorp])
        // Der Controller meldet je Ereignis die Commits: Nr. 0 = Vorspann, Nr. 1 = Zitat.
        s.noteCommit(requestId: "r1", index: 1, stage: .display, ok: true, generation: g)
        s.noteCommit(requestId: "r1", index: 1, stage: .audio, ok: false, generation: g + 1)
        s.noteCommit(requestId: "r1", index: 99, stage: .display, ok: true, generation: g)   // unbekannter Index: ignoriert
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)

        let entries = await audit.entries()
        guard let e = entries.first else { return XCTFail("kein Eintrag") }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(e.round, .data)
        XCTAssertEqual(e.level, .s1)
        XCTAssertEqual(e.generation, g)
        XCTAssertEqual(e.outcome, .completed)
        XCTAssertFalse(e.unsourcedMode)
        XCTAssertEqual(e.areas, ["books"])
        XCTAssertEqual(e.counts, AssistantSentenceCounts(grounded: 1, droppedOwnData: 2))
        XCTAssertEqual(e.history, AssistantCommitMark(stage: .history, ok: true, generation: g))
        // Satzspur in Einordnungs-Reihenfolge: verworfen, Vorspann, Zitat, verworfen.
        XCTAssertEqual(e.trace.map(\.verdict), [.droppedOwnData, .announcement, .grounded, .droppedOwnData])
        XCTAssertEqual(e.trace.map(\.n), [1, 2, 3, 4])
        XCTAssertEqual(e.trace.map(\.generation), [g, g, g, g])
        let quoteStep = e.trace[2]
        XCTAssertEqual(quoteStep.commits, [
            AssistantCommitMark(stage: .emit, ok: true, generation: g),
            AssistantCommitMark(stage: .display, ok: true, generation: g),
            AssistantCommitMark(stage: .audio, ok: false, generation: g + 1),
        ])
        XCTAssertEqual(e.trace[0].commits, [], "verworfene Sätze haben keinen Ausgabe-Commit")
    }

    func testAuditFileContainsNoContentOfTheConversation() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: [castorpModel]), config: strict, audit: audit)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        let raw = rawLog(audit)
        XCTAssertFalse(raw.isEmpty)
        for word in ["Castorp", "Davos", "Zauberberg", "Joachim", "Sanatorium", "Wohin", "Mann", "S. 12"] {
            XCTAssertFalse(raw.contains(word), "\(word) steht im Protokoll:\n\(raw)")
        }
        XCTAssertTrue(raw.contains("\"books\""))
    }

    func testAuditLevelOfATemplateOnlyAnswerIsS0AndUnsourcedIsAtLeastS1() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: ["Hallo! Wie kann ich dir helfen?"]), config: open, audit: audit)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        await s.commit(requestId: "r1", user: "Hallo", spoken: "Hallo! Wie kann ich dir helfen?", interrupted: false)
        let t = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), config: open, audit: audit)
        _ = await collect(t, AssistantRequest(requestId: "r2", text: "Wie hoch ist der Mont Blanc?"))
        await t.commit(requestId: "r2", user: "Wie hoch ist der Mont Blanc?", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 2)
        XCTAssertEqual(entries[0].level, .s0)
        XCTAssertEqual(entries[0].round, .general)
        XCTAssertEqual(entries[0].counts, AssistantSentenceCounts(neutral: 2))
        XCTAssertEqual(entries[1].level, .s1)
        XCTAssertEqual(entries[1].counts, AssistantSentenceCounts(unsourced: 1))
        XCTAssertTrue(entries[1].unsourcedMode)
        XCTAssertEqual(entries[1].trace.map(\.verdict), [.announcement, .unsourced])
    }

    func testRevokedAnswerIsReportedAsRevokedInTheAuditLogWithTheCounters() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. Katzen", " putzen sich mehrmals am Tag."], pauseMs: 150)
        let s = await session(model, config: open, audit: audit)
        var spoken: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
            if case .sentence(_, _, _, _, .unsourced, _, _) = e { s.setAllowUnsourced(false) }
        }
        let wrote = await s.commit(requestId: "r1", user: "Erzähl etwas über Tiere", spoken: spoken.joined(separator: " "), interrupted: false)
        XCTAssertTrue(wrote)
        let entries = await audit.entries()
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].outcome, .revoked)
        XCTAssertEqual(entries[0].counts.unsourced, 1)
        XCTAssertTrue(entries[0].unsourcedMode, "Modus beim Start der Anfrage")
        XCTAssertEqual(entries[0].level, .s1)
        XCTAssertEqual(entries[0].history?.ok, false)
        let h = await s.history
        XCTAssertTrue(h.isEmpty)
    }

    func testAnswerWithoutAuditLogStillCommits() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        let ok = await s.commit(requestId: "r1", user: question, spoken: castorp, interrupted: false)
        XCTAssertTrue(ok)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
    }

    // MARK: Prompt

    func testDataPromptHasExcerptsAndHistoryButNoInventory() async {
        let log = Log()
        let hist = [AssistantTurn(user: "Frühere Frage", assistant: "Frühere Antwort", interrupted: true)]
        let s = await session(ScriptModel(pieces: ["Gut."], log: log), history: hist, config: strict)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question, afterInterruption: true))
        let call = log.all.first
        XCTAssertEqual(call?.maxTokens, 400)
        XCTAssertTrue(call?.instructions.contains("Local Voice auf dem iPhone von Patrick") ?? false)
        XCTAssertTrue(call?.instructions.contains("Unter AUSZÜGE stehen") ?? false)
        let p = call?.prompt ?? ""
        XCTAssertFalse(p.contains("BESTAND"), p)
        XCTAssertTrue(p.contains("[1] Buch · Der Zauberberg · S. 12"), p)
        XCTAssertTrue(p.contains("Assistent: Frühere Antwort …"), p)
        XCTAssertTrue(p.contains("Der Nutzer hat dich gerade unterbrochen."), p)
    }

    func testPromptKindFollowsTheOptInAtRequestTimeAndTheLock() async {
        let log = Log()
        let s = await session(ScriptModel(pieces: ["Gut."], log: log), config: strict)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("Unter AUSZÜGE stehen") ?? false, "ohne Opt-in eine Datenrunde")
        s.setAllowUnsourced(true)
        _ = await collect(s, AssistantRequest(requestId: "r2", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("Unter AUSZÜGE stehen") ?? false,
                      "das Gespräch hatte schon eine Datenrunde: Einschalten gibt keine Runde ohne Quelle frei")
        XCTAssertTrue(s.canResumeFree)
        XCTAssertTrue(s.resumeFree())
        _ = await collect(s, AssistantRequest(requestId: "r3", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("In dieser Runde siehst du keine Inhalte") ?? false, "erst nach `resumeFree` eine Allgemeinrunde")
        XCTAssertFalse(log.all.last?.prompt.contains("AUSZÜGE") ?? true)
        let fresh = await session(ScriptModel(pieces: ["Gut."], log: log), config: strict)
        fresh.setAllowUnsourced(true)
        _ = await collect(fresh, AssistantRequest(requestId: "r4", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("In dieser Runde siehst du keine Inhalte") ?? false,
                      "ein Gespräch ohne Datenrunde: Einschalten setzt den Zustand frei")
    }
}
