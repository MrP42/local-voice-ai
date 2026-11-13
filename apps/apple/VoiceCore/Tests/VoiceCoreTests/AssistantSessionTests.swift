import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K, E4.2a in Paket A): Ablauf einer Antwort mit künstlichen Modellen.
// Datenrunde: Das Modell wählt, das System gibt ganze gespeicherte Blöcke aus. Allgemeinrunde: nur mit Opt-in, Prompt ohne Auszüge.

/// Wartet (begrenzt) auf einen Zustand, den ein Rückruf setzt (Ende eines Modellstroms, gelieferte Stücke), statt eine Zeit zu raten.
/// Der Test prüft den Zustand danach selbst; erst nach `timeout` Sekunden gilt er als nicht eingetreten.
private func eventually(timeout: TimeInterval = 60, _ condition: () -> Bool) async {
    let deadline = Date().addingTimeInterval(timeout)
    while !condition(), Date() < deadline { try? await Task.sleep(nanoseconds: 5_000_000) }
}

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

/// Haltepunkt für ein Modell (Runde 8: Ordnung ohne Uhr). Das Modell wartet an ihm, bis der Test ihn öffnet oder die Aufgabe des Modells
/// abgebrochen wird. Das Warten fragt nur eine Marke ab (die Länge des Abfrageschritts ändert keine Aussage).
private final class ModelHold: @unchecked Sendable {
    private let lock = NSLock()
    private var opened = false
    var isOpen: Bool { lock.withLock { opened } }
    func open() { lock.withLock { opened = true } }
}

/// Eine Marke, die ein Rückruf setzt und auf die der Test wartet (statt eine Zeit zu raten).
private final class Latch: @unchecked Sendable {
    private let lock = NSLock()
    private var fired = false
    var isFired: Bool { lock.withLock { fired } }
    func fire() { lock.withLock { fired = true } }
}

/// Liefert Stücke mit Pause dazwischen; `fail` nach den Stücken, `hang` wartet danach auf Abbruch. `holdAfter`: nach so vielen Stücken
/// hält das Modell am Haltepunkt `hold`, bis er geöffnet oder abgebrochen wird (statt „die Pause ist lang genug“).
private struct ScriptModel: BookTalkModel {
    var pieces: [String]
    var pauseMs: UInt64 = 0
    var fail: (any Error)?
    var hang = false
    var log = Log()
    var holdAfter: Int?
    var hold = ModelHold()

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        log.hit(instructions, prompt, maxTokens)
        let pieces = pieces, pause = pauseMs, fail = fail, hang = hang, log = log, holdAfter = holdAfter, hold = hold
        return AsyncThrowingStream { c in
            let task = Task {
                for (index, p) in pieces.enumerated() {
                    if pause > 0 { try? await Task.sleep(nanoseconds: pause * 1_000_000) }
                    if Task.isCancelled { return }
                    log.piece()
                    c.yield(p)
                    if holdAfter == index + 1 {
                        while !hold.isOpen, !Task.isCancelled { try? await Task.sleep(nanoseconds: 5_000_000) }
                        if Task.isCancelled { return }
                    }
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
/// Frage, die beide Blöcke des Buchs abruft (Reise und Aufenthalt).
private let both = "Wohin fährt Hans Castorp im Sanatorium nach Davos und wie viele Wochen bleibt er?"
private let preface = "Wörtlicher Auszug aus ‚Der Zauberberg‘:"
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
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorp), ("S. 13", stay)]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func twoSourceKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorp), ("S. 13", stay)]))
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

    /// R1: Der Auszug wird so gesprochen, wie er gespeichert ist; das Sprechfeld ist der Blocktext selbst (kein Dezimalkomma, keine
    /// Auflösung von Kürzeln, keine Entfernung von Markdown oder Adressen). Einzelheiten der Aussprache liegen bei der Stimme.
    func testQuoteSpeechFieldIsTheBlockTextItself() async {
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
        XCTAssertEqual(pairs.map(\.1), ["Der Zaun misst 3.5 Meter und braucht jedes Jahr frische Farbe."], "Sprechfeld = Blocktext, unverändert")
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

    func testSecondBlockOfTheSameSourceIsQuotedToo() async {
        let seen = await run([castorpModel + " ", "Er bleibt drei Wochen dort."], text: both)
        XCTAssertEqual(seen.spoken, [castorp, stay])
        XCTAssertEqual(seen.statuses, [.announcement, .grounded, .announcement, .grounded], "ein Vorspann vor JEDEM Block, auch bei gleicher Quelle")
        XCTAssertEqual(seen.texts, [preface, castorp, preface, stay])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 2))
        XCTAssertEqual(seen.answer?.sources.map(\.locator), ["S. 12", "S. 13"], "zwei Blöcke, zwei Quellstellen")
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
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
        let e = await audit.entries().filter { $0.kind == .answer }.first
        XCTAssertEqual(e?.trace.map(\.verdict), [.announcement, .grounded, .droppedFragment])
        XCTAssertEqual(e?.counts.droppedFragment, 1)
        XCTAssertFalse(rawLog(audit).contains("künftig"))
    }

    /// Satzzeichen sind kein Kriterium: Ein gespeicherter Block ohne Satzzeichen („Anna für Projekt Nordlicht“) ist als Ganzes
    /// zitierbar, mit Vorspann; er wird nie ergänzt, gekürzt oder mit dem Modelltext vermischt.
    func testAStoredBlockWithoutPunctuationIsQuotedWholeAndUnchanged() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Notiz", area: .notes, sections: [(nil, "Anna für Projekt Nordlicht")]))
        let k = AssistantKnowledge(corpus: corpus)
        let s = AssistantSession(model: ScriptModel(pieces: ["Anna übernimmt die Leitung für das Projekt Nordlicht."]), knowledge: k, config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wer macht das Projekt Nordlicht?")))
        XCTAssertEqual(seen.texts, ["Wörtlicher Auszug aus ‚Notiz‘:", "Anna für Projekt Nordlicht"])
        XCTAssertEqual(seen.statuses, [.announcement, .grounded])
        XCTAssertFalse(seen.texts.joined().contains("Leitung"), "kein Wort des Modells im Zitat")
        XCTAssertEqual(seen.answer?.sources.map(\.itemId), ["n1"])
    }

    /// Ausgegeben wird der GANZE gespeicherte Block, nie ein Teil davon (auch nicht der Satz, der eingerastet ist); ein Block über
    /// der Höchstlänge wird nie ausgegeben, auch nicht gekürzt.
    func testTheWholeStoredBlockIsQuotedNeverAPartOfIt() async {
        let block = "Die Sitzung beginnt um zehn Uhr im großen Sitzungssaal. Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag. Bernd kümmert sich um das Budget des Projekts Nordlicht."
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("t1", "Besprechung", area: .transcripts, sections: [("00:01", block)]))
        let s = AssistantSession(model: ScriptModel(pieces: ["Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."]),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Wer plant das Projekt Nordlicht?")))
        XCTAssertEqual(seen.spoken, [block], "der ganze Block, beide Nachbarsätze inklusive")
        XCTAssertTrue(seen.texts.contains("Wörtlicher Auszug aus ‚Besprechung‘:"), "Kennzeichnung vor dem Block")

        // Ein Block über der Höchstlänge wird nie ausgegeben.
        let long = String(repeating: "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag. ", count: 12)
        XCTAssertGreaterThan(long.count, AssistantGrounding.maxBlockChars)
        let corpus2 = AssistantCorpus()
        await corpus2.upsert(assistantDoc("t2", "Besprechung", area: .transcripts, sections: [("00:02", long)]))
        let s2 = AssistantSession(model: ScriptModel(pieces: ["Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."]),
                                  knowledge: AssistantKnowledge(corpus: corpus2), config: strict, released: true)
        let seen2 = Seen(events: await collect(s2, AssistantRequest(requestId: "r2", text: "Wer plant das Projekt Nordlicht?")))
        XCTAssertFalse(seen2.texts.contains { $0.contains("Anna übernimmt") }, "auch nicht gekürzt oder in Teilen")
        XCTAssertEqual(seen2.texts, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
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
        "Wörtlicher Auszug aus ‚Der Zauberberg‘: Hans ist tot.",
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
                    case .conversational, .summarized:
                        XCTFail("Einordnung des Stils natural im Stil verbatim: \(text) → \(s.text)")
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
            castorp, "Joachim Ziemßen ist Soldat und kränkelt schon lange.",
            "Der Hofrat Behrens leitet das Sanatorium oben auf dem Berg.", "Madame Chauchat wohnt im Sanatorium und isst am Tisch der Russen.",
            "Settembrini ist Humanist und streitet gern mit Naphta über den Fortschritt.",
        ]
        let corpus = AssistantCorpus()
        // Je Block ein eigenes Element: Der Abruf nimmt höchstens drei Blöcke je Element auf.
        for (i, sentence) in sentences.enumerated() {
            await corpus.upsert(assistantDoc("b\(i + 1)", "Der Zauberberg", sections: [("S. \(12 + i)", sentence)]))
        }
        let log = Log()
        let s = AssistantSession(model: ScriptModel(pieces: sentences.map { $0 + " " }, pauseMs: 20, hang: true, log: log),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        // Dass `collect` zurückkehrt, beweist: Das hängende Modell wurde abgebrochen, nicht abgewartet (kein Zeitmaß; gäbe es das
        // Abbrechen nicht, bliebe der Test hängen und der Lauf bräche ihn ab).
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Hans Castorp Davos Sanatorium Joachim Soldat Hofrat Behrens Chauchat Settembrini Humanist")))
        XCTAssertEqual(seen.spoken, Array(sentences.prefix(3)))
        XCTAssertEqual(seen.statuses, [.announcement, .grounded, .announcement, .grounded, .announcement, .grounded])
        XCTAssertTrue(seen.hasDone)
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 3))
        await eventually { log.terminations >= 1 }
        XCTAssertGreaterThanOrEqual(log.terminations, 1, "Modellstrom wurde beendet")
    }

    func testDuplicateQuotesAreDroppedAndCounted() async {
        let seen = await run([castorpModel + " ", "Hans Castorp reist nach Davos in das Sanatorium zu seinem Vetter Joachim."])
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(seen.answer?.counts, AssistantSentenceCounts(grounded: 1, droppedDuplicate: 1))
    }

    func testPrefaceComesBeforeEveryBlock() async {
        let s = AssistantSession(model: ScriptModel(pieces: [castorpModel + " ", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag. ",
                                                              "Er bleibt drei Wochen."]),
                                 knowledge: await twoSourceKnowledge(), config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Hans Castorp Davos Sanatorium Wochen Anna Planung Projekt Nordlicht")))
        XCTAssertEqual(Set(seen.retrieved?.map(\.itemId) ?? []), ["b1", "t1"])
        XCTAssertEqual(seen.texts, [
            preface, castorp,
            "Wörtlicher Auszug aus ‚Besprechung‘:", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.",
            preface, stay,
        ], "vor jedem Block steht sein Vorspann, auch beim Wechsel A, B, A")
        XCTAssertEqual(seen.sentences.filter { $0.status == .grounded }.map { $0.sources.first?.itemId }, ["b1", "t1", "b1"])
        XCTAssertEqual(seen.answer?.sources.map(\.itemId).sorted(), ["b1", "b1", "t1"], "je Block eine Quellstelle")
    }

    func testPrefaceTitleIsCleanedAndShortened() async {
        let corpus = AssistantCorpus()
        let longTitle = "Ein ‚sehr‘ \"langer\" Titel\nmit Zeilenumbruch " + String(repeating: "wort ", count: 40)
        await corpus.upsert(assistantDoc("n1", longTitle, area: .notes, sections: [(nil, "Der Zaun braucht jedes Jahr frische Farbe im Frühling.")]))
        let s = AssistantSession(model: ScriptModel(pieces: ["Der Zaun braucht jedes Jahr frische Farbe im Frühling."]),
                                 knowledge: AssistantKnowledge(corpus: corpus), config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Welche Farbe braucht der Zaun im Frühling?")))
        guard let first = seen.sentences.first, first.status == .announcement else { return XCTFail("\(seen.events)") }
        XCTAssertTrue(first.text.hasPrefix("Wörtlicher Auszug aus ‚Ein sehr langer Titel mit Zeilenumbruch"), first.text)
        XCTAssertTrue(first.text.hasSuffix("…‘:"), first.text)
        XCTAssertLessThanOrEqual(first.text.count, "Wörtlicher Auszug aus ‚".count + AssistantGrounding.maxPrefaceTitle + "‘:".count)
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
        var k = await knowledge()
        k = k.withAreas([.notes])
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
        var k = await twoSourceKnowledge()
        k = k.withAreas([.transcripts])
        let s = AssistantSession(model: ScriptModel(pieces: []), knowledge: k, config: strict, released: true)
        let seen = Seen(events: await collect(s, AssistantRequest(requestId: "r1", text: "Was habe ich gespeichert?")))
        XCTAssertEqual(seen.spoken, ["Du hast ein Transkript.", "Neueste Transkripte: Besprechung."])
        XCTAssertFalse(seen.texts.joined().contains("Zauberberg"))
    }

    // MARK: Allgemeinrunde

    func testGeneralRoundPromptHasNoExcerptsNoInventoryAndHistoryOnlyFromGeneralRounds() async {
        let log = Log()
        let hist = [
            AssistantTurn(user: "Frage zum Buch", assistant: "Hans Castorp fährt nach Davos.", round: .data).shown,
            AssistantTurn(user: "Alte Frage ohne Rundenart", assistant: "Alte Antwort mit Joachim.").shown,
            AssistantTurn(user: "Wie hoch ist die Zugspitze?", assistant: "Rund 2962 Meter.", round: .general).shown,
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
        var k = await twoSourceKnowledge()
        k = k.withAreas([.books])
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
            if commitFirst { await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false) }
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
        let old = [AssistantTurn(user: "Frage", assistant: "Antwort").shown]
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
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: true)
        h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertEqual(h[0].user, question)
        XCTAssertEqual(h[0].assistant, castorp, "der Block steht ganz im Beitrag, byte-gleich, nie gekürzt")
        XCTAssertTrue(h[0].interrupted)
        XCTAssertFalse(h[0].general)
        XCTAssertEqual(h[0].sources.first?.itemId, "b1")
        XCTAssertEqual(h[0].model, "Test")
        XCTAssertEqual(h[0].round, .data)
        let label = h[0].sources.first?.excerptLine
        XCTAssertEqual(label, "Wörtlicher Auszug aus Der Zauberberg · S. 12", "die Quellzeile des Zitats kommt aus dem Auszug")
        XCTAssertEqual(h[0].parts, [AssistantTurnPart(text: castorp, unsourced: false, quote: true, source: label)],
                       "der Block ist ein Zitat, mit seiner Quelle")
        XCTAssertFalse(h[0].unsourced)
    }

    func testCommitOfAGeneralRoundMarksTheTurnUnsourcedAndGeneral() async {
        let s = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), config: open)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: "Wie hoch ist der Mont Blanc?"))
        await s.commit(requestId: "r1", user: "Wie hoch ist der Mont Blanc?", spoken: .all, interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertTrue(h[0].general)
        XCTAssertTrue(h[0].unsourced)
        XCTAssertEqual(h[0].round, .general)
        XCTAssertEqual(h[0].parts, [AssistantTurnPart(text: "Der Mont Blanc ist 4806 Meter hoch.", unsourced: true)],
                       "auch ein rein unbelegter Beitrag trägt seine Abschnitte mit der Marke „Ohne Quelle“: Es gibt keinen Beitrag ohne Abschnitte")
        XCTAssertTrue(h[0].sources.isEmpty)
    }

    func testCommitIgnoresEmptyUserEmptySpokenTextAndUnknownRequests() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: .nothing, interrupted: true)
        var h = await s.history
        XCTAssertTrue(h.isEmpty, "keine leeren Beiträge")
        _ = await collect(s, AssistantRequest(requestId: "r2", text: question))
        await s.commit(requestId: "r2", user: "   ", spoken: .all, interrupted: false)
        await s.commit(requestId: "unbekannt", user: "Noch etwas", spoken: .all, interrupted: true)
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
        h = await s.history
        XCTAssertTrue(h.isEmpty, "unbekannt oder schon abgeschlossen: nichts")
    }

    func testCommitTwiceWritesOneTurn() async {
        let s = await session(ScriptModel(pieces: [castorpModel]))
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
    }

    func testUnitsAreChosenFromTheRunRecordNeverDerivedFromText() {
        var record = AssistantRunRecord()
        let passage = AssistantPassage(area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: castorp, score: 1)
        record.sentences = [
            AssistantEmitted(text: "Ich suche kurz.", status: .announcement, sources: [], quote: false, step: 1),
            AssistantEmitted(text: castorp, status: .grounded, sources: [passage], quote: true, step: 2),
            AssistantEmitted(text: "Hallo!", status: .neutral, sources: [], quote: false, step: 3),
            AssistantEmitted(text: "Vielleicht auch so.", status: .unsourced, sources: [], quote: false, step: 4),
        ]
        let index = AssistantStepIndex()
        for step in 1...4 { index.append("r", step: step) }
        func units(_ spoken: AssistantSpokenUnits) -> [AssistantEmitted] {
            AssistantSession.units(of: record, spoken: spoken, stepIndex: index, requestId: "r")
        }
        XCTAssertEqual(units(.all).map(\.step), [2, 3, 4], "Ansagen zählen nie; sonst alle Einheiten in der Reihenfolge der Ausgabe")
        XCTAssertEqual(units(.nothing), [])
        // Die Ereignisse 0 (Ansage) und 1 (Zitat) wurden ganz gesprochen: nur das Zitat, mit Art und Quelle, unverändert.
        let cut = units(.events([0, 1]))
        XCTAssertEqual(cut.map(\.text), [castorp])
        XCTAssertEqual(cut.map(\.status), [.grounded])
        XCTAssertEqual(cut.map(\.quote), [true])
        // Die Auswahl hängt an den Ereignisnummern, nicht an der Reihenfolge der Liste.
        XCTAssertEqual(units(.events([3, 2])).map(\.step), [3, 4])
        XCTAssertEqual(units(.events([9])), [], "eine unbekannte Ereignisnummer ergibt nichts")
        let unknown = AssistantSession.units(of: record, spoken: .events([1]), stepIndex: AssistantStepIndex(), requestId: "r")
        XCTAssertEqual(unknown, [], "ein unbekannter Lauf (verdrängt oder fremd) trägt nichts in den Verlauf")
    }

    func testAUnitWithoutItsMarkNeverEntersTheHistory() {
        var record = AssistantRunRecord()
        // Ein Zitat ohne Quelle hat keine Marke: Es käme im Verlauf nur als bloßer Text oder als „Quelle nicht gespeichert“.
        record.sentences = [AssistantEmitted(text: castorp, status: .grounded, sources: [], quote: true, step: 1)]
        let index = AssistantStepIndex()
        index.append("r", step: 1)
        XCTAssertEqual(AssistantSession.units(of: record, spoken: .all, stepIndex: index, requestId: "r"), [])
    }

    // MARK: Cloud-Schutz für den Verlauf

    private func historyForAreas() -> [AssistantTurn] {
        func source(_ area: AssistantArea) -> [AssistantPassage] {
            [AssistantPassage(area: area, itemId: "x-\(area.rawValue)", title: "Titel", text: "Text", score: 1)]
        }
        return [
            AssistantTurn(user: "FrageBuch", assistant: "AntwortBuch", sources: source(.books), general: false).shown,
            AssistantTurn(user: "FrageAblage", assistant: "AntwortAblage", sources: source(.files), general: false).shown,
            AssistantTurn(user: "FrageAllgemein", assistant: "AntwortAllgemein", general: true).shown,
            AssistantTurn(user: "FrageGemischt", assistant: "AntwortGemischt", sources: source(.books) + source(.notes), general: false).shown,
        ]
    }

    /// Wissen mit dem Buch aus `knowledge()`, aber nur den gegebenen Bereichen freigegeben.
    private func knowledge(areas: Set<AssistantArea>) async -> AssistantKnowledge {
        var k = await knowledge()
        k = k.withAreas(areas)
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
        _ = await s.setAreas([.books, .files, .notes])
        _ = await collect(s, AssistantRequest(requestId: "r2", text: "Hallo zusammen"))
        let all = log.all.last?.prompt ?? ""
        for word in ["AntwortBuch", "AntwortAblage", "AntwortAllgemein", "AntwortGemischt"] { XCTAssertTrue(all.contains(word), "\(word)\n\(all)") }
        _ = await s.setAreas([])
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
        let hist = historyForAreas() + [AssistantTurn(user: "FrageLaufend", assistant: "AntwortLaufend", round: .general).shown]
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
        let model = ScriptModel(pieces: ["Erster Satz kommt hier. ", "Zweiter Satz folgt später. ", "Dritter Satz auch noch."], log: log, holdAfter: 2)
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
        await eventually { log.terminations >= 1 }
        XCTAssertGreaterThanOrEqual(log.terminations, 1, "Modellstrom wurde beendet")
    }

    func testCancelBeforeAnySentenceWhileModelHangs() async {
        let log = Log()
        let s = await session(ScriptModel(pieces: [], hang: true, log: log), config: open)
        let task = Task { await collect(s, AssistantRequest(requestId: "r1", text: "Hallo")) }
        await eventually { log.all.count >= 1 }   // der Modellaufruf steht (Haltepunkt statt einer geratenen Wartezeit)
        XCTAssertEqual(log.all.count, 1)
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
        let model = ScriptModel(pieces: ["Ein Satz kommt. ", "Noch ein Satz folgt. ", "Und noch einer."], log: log, holdAfter: 1)
        let s = await session(model, config: open)
        let task = Task { () -> Int in
            var n = 0
            for await e in s.events(AssistantRequest(requestId: "r1", text: "Hallo")) { if case .sentence = e { n += 1 } }
            return n
        }
        await eventually { log.pieces >= 1 }   // der Lauf steht mitten im Strom
        task.cancel()
        _ = await task.value
        await eventually { log.terminations >= 1 }
        XCTAssertGreaterThanOrEqual(log.terminations, 1)
        let h = await s.history
        XCTAssertTrue(h.isEmpty)
    }

    func testTimeoutFromConfig() async {
        var c = open
        c.timeout = 0.3
        let s = await session(ScriptModel(pieces: ["Halber Satz ohne Ende"], hang: true), config: c)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(events.last, .failed(requestId: "r1", .timeout))
        XCTAssertEqual(events.filter { if case .failed = $0 { true } else { false } }.count, 1)
    }

    func testSlowButSteadyStreamDoesNotTimeOut() async {
        var c = open
        // Zeitannahme: Die Pause zwischen zwei Stücken (100 ms) liegt zehnfach unter der Zeitgrenze (1 s), die Gesamtdauer (14 Stücke,
        // mindestens 1,4 s: `Task.sleep` wartet nie kürzer) darüber. Auf einer langsameren Maschine werden die Pausen nur länger, die
        // Aussage „die Gesamtdauer übersteigt die Grenze“ bleibt, und die Lücke je Stück bleibt weit unter der Grenze.
        c.timeout = 1.0
        let pieces = (0..<14).map { "Satz Nummer \($0) kommt. " }
        let s = await session(ScriptModel(pieces: pieces, pauseMs: 100), config: c)
        let events = await collect(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        guard case .done(_, let a)? = events.last else { return XCTFail("\(events)") }
        XCTAssertGreaterThan(a.totalMs, 1_000)
    }

    // MARK: Fehler

    func testContextOverflowRetriesOnceWithHalfPassagesAndNoHistory() async {
        let log = Log()
        let hist = [AssistantTurn(user: "Frühere Frage", assistant: "Frühere Antwort").shown]
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
        // Beide Läufe halten nach dem ersten Stück am Haltepunkt: „spec“ ist damit sicher noch nicht fertig, wenn der Abbruch kommt.
        // Erst danach wird der Haltepunkt geöffnet, und „final“ läuft zu Ende.
        let log = Log()
        let model = ScriptModel(pieces: ["Erster Satz. ", "Zweiter Satz. ", "Dritter Satz."], log: log, holdAfter: 1)
        let s = await session(model, config: open)
        async let a = collect(s, AssistantRequest(requestId: "spec", text: "Wie wird das Wetter"))
        async let b = collect(s, AssistantRequest(requestId: "final", text: "Wie wird das Wetter morgen"))
        await eventually { log.pieces >= 1 }
        await s.cancel(requestId: "spec")
        model.hold.open()
        let (ea, eb) = await (a, b)
        XCTAssertEqual(ea.last, .failed(requestId: "spec", .cancelled))
        guard case .done("final", _)? = eb.last else { return XCTFail("\(eb)") }
        var h = await s.history
        XCTAssertTrue(h.isEmpty)
        await s.commit(requestId: "final", user: "Wie wird das Wetter morgen", spoken: .all, interrupted: false)
        h = await s.history
        XCTAssertEqual(h.count, 1)
    }

    // MARK: Widerruf mit Generation

    func testGenerationStartsMonotoneAndChangesOnlyWithARealChange() async {
        let s = await session(ScriptModel(pieces: []), config: strict)
        let g0 = s.releaseGeneration
        let r6 = await s.setAllowUnsourced(false)
        XCTAssertEqual(r6.generation, g0, "keine Änderung: keine neue Generation")
        let g1 = await s.setAllowUnsourced(true).generation
        XCTAssertGreaterThan(g1, g0)
        XCTAssertTrue(s.allowsUnsourced)
        let g2 = await s.setAllowUnsourced(false).generation
        let g3 = await s.setAllowUnsourced(true).generation
        XCTAssertTrue(g1 < g2 && g2 < g3, "Aus und wieder Ein gibt nie eine alte Generation zurück")
        XCTAssertEqual(s.releaseGeneration, g3)
        let r7 = await s.releaseDidChange()
        XCTAssertGreaterThan(r7.generation, g3, "Cloud-Zustimmung: jede Änderung zählt")
        let before = s.releaseGeneration
        let after = await s.setAreas([.books])
        XCTAssertEqual(after.generation, before + 1, "EINE Erhöhung je Änderung der Bereiche (nicht mehr zwei)")
        XCTAssertEqual(s.releaseGeneration, after.generation)
    }

    func testRevokingOptInMidStreamStopsTheAnswerAndNothingOldComesOutAfterwards() async {
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. Katzen", " putzen sich mehrmals am Tag. ", "Pferde schlafen oft im Stehen."], holdAfter: 1)
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
                await s.setAllowUnsourced(false)
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
        // Eine spätere Frage mit widerrufenem Opt-in läuft als Datenrunde (der Haltepunkt des Modells ist für sie offen).
        model.hold.open()
        let next = Seen(events: await collect(s, AssistantRequest(requestId: "r2", text: "Erzähl etwas über Tiere")))
        XCTAssertEqual(next.answer?.round, .data)
        XCTAssertFalse(next.statuses.contains(.unsourced))
    }

    func testOffThenOnDoesNotReviveTheOldRun() async {
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. Katzen", " putzen sich mehrmals am Tag."], holdAfter: 1)
        let s = await session(model, config: open)
        let g0 = s.releaseGeneration
        var events: [AssistantEvent] = []
        var flipped = false
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            events.append(e)
            if case .sentence(_, _, _, _, .unsourced, _, _) = e, !flipped {
                flipped = true
                await s.setAllowUnsourced(false)
                await s.setAllowUnsourced(true)
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
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], holdAfter: 1)
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
        XCTAssertEqual(s.releaseGeneration, g0 + 1, "eine Änderung, eine Erhöhung")
        XCTAssertEqual(s.effectiveAreas, [.notes])
    }

    func testCloudConsentChangeRevokesRunningAnswers() async {
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], holdAfter: 1)
        let s = await session(model, config: strict)
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            events.append(e)
            if case .sentence(_, _, _, _, .grounded, _, _) = e { _ = await s.releaseDidChange() }
        }
        XCTAssertEqual(Seen(events: events).failure, .revoked)
        XCTAssertEqual(Seen(events: events).spoken, [castorp])
    }

    func testRevokedAnswerIsNeverWrittenToTheHistory() async {
        let s = await session(ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], holdAfter: 1), config: strict)
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            if case .sentence(_, _, _, _, .grounded, _, _) = e { _ = await s.releaseDidChange() }
        }
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
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
        let now = await s.setAllowUnsourced(true).generation
        XCTAssertGreaterThan(now, started)
        let wrote = await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
        XCTAssertTrue(wrote)
        let h = await s.history
        XCTAssertTrue(h.isEmpty)
        let entries = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].history, AssistantCommitMark(stage: .history, ok: false, generation: now))
        XCTAssertEqual(entries[0].outcome, .revoked)
        XCTAssertEqual(entries[0].generation, started, "die Generation beim Start der Anfrage")
    }

    // MARK: Commit und Lauf

    /// Suchlauf (d), Begründung für das `try?` im Warten von `settle`: Ein abgebrochener Aufrufer von `commit` (sein `sleep` wirft
    /// sofort) wartet trotzdem auf das geordnete Ende des Laufs und schreibt denselben Beitrag wie ein nicht abgebrochener. Gemessen:
    /// Es gibt keine Hänger (die Schleife gibt den Aktor mit jedem `await` ab, das Ende des Laufs kommt zum Zug). Der Test hält das fest.
    func testCommitFromACancelledCallerStillWaitsForTheOrderedEndOfTheRun() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], hang: true)
        let s = await session(model, config: strict, audit: audit)
        let firstUnit = Latch()
        let task = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
                out.append(e)
                if case .sentence(_, _, _, _, let status, _, _) = e, status != .announcement { firstUnit.fire() }
            }
            return out
        }
        await eventually { firstUnit.isFired }
        XCTAssertTrue(firstUnit.isFired)
        let caller = Task { () -> Bool in
            while !Task.isCancelled { await Task.yield() }
            return await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: true)
        }
        caller.cancel()
        let wrote = await caller.value
        XCTAssertTrue(wrote)
        let seen = Seen(events: await task.value)
        XCTAssertEqual(seen.failure, .cancelled)
        XCTAssertEqual(seen.spoken, [castorp])
        XCTAssertEqual(s.registeredRunCount, 0, "der Lauf ist geordnet beendet und abgemeldet")
        let history = await s.history
        XCTAssertEqual(history.count, 1)
    }

    func testCommitWhileTheRunIsStillGoingCancelsItAndWaitsForTheOrderedEnd() async throws {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let model = ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], hang: true)
        let s = await session(model, config: strict, audit: audit)
        let firstUnit = Latch()
        let task = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
                out.append(e)
                if case .sentence(_, _, _, _, let status, _, _) = e, status != .announcement { firstUnit.fire() }
            }
            return out
        }
        await eventually { firstUnit.isFired }   // der erste Satz ist da, der zweite kann nicht kommen (das Modell hängt)
        XCTAssertTrue(firstUnit.isFired)
        // Dass `commit` zurückkehrt, beweist: kein Warten auf das hängende Modell (kein Zeitmaß).
        let wrote = await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: true)
        XCTAssertTrue(wrote)
        let seen = Seen(events: await task.value)
        XCTAssertEqual(seen.failure, .cancelled)
        XCTAssertEqual(seen.spoken, [castorp])

        let h = await s.history
        XCTAssertEqual(h.count, 1)
        XCTAssertEqual(h[0].sources.map(\.itemId), ["b1"], "der Satz ist als Originalsatz belegt, nicht als unbelegt")
        XCTAssertFalse(h[0].unsourced)
        XCTAssertEqual(h[0].parts, [AssistantTurnPart(text: castorp, unsourced: false, quote: true, source: h[0].sources.first?.excerptLine)])
        XCTAssertNotNil(h[0].parts.first?.source)
        XCTAssertTrue(h[0].interrupted)
        let entries = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(entries.count, 1, "genau ein Eintrag je Anfrage")
        XCTAssertEqual(entries[0].outcome, .interrupted)
        XCTAssertEqual(entries[0].counts.grounded, 1)
        XCTAssertEqual(entries[0].areas, ["books"])
        XCTAssertEqual(entries[0].history?.ok, true)

        // Ein zweiter commit zur selben Anfrage ergibt weder Beitrag noch Eintrag.
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: true)
        let again = await s.history
        XCTAssertEqual(again.count, 1)
        let entriesAgain = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(entriesAgain.count, 1)
    }

    func testCommitWaitsForTheFinalStateAndDoesNotMisattributeGroundedSentences() async {
        // Der Lauf endet erst nach dem Abbruch durch commit; der Stand wird vor dem Schreiben festgehalten.
        let s = await session(ScriptModel(pieces: [castorpModel + " Das", " ist völlig unklar. "], hang: true), config: open)
        let firstUnit = Latch()
        let task = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
                out.append(e)
                if case .sentence(_, _, _, _, let status, _, _) = e, status != .announcement { firstUnit.fire() }
            }
            return out
        }
        await eventually { firstUnit.isFired }
        XCTAssertTrue(firstUnit.isFired)
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: true)
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
        let wrote = await s.commit(requestId: "r1", user: question, spoken: .nothing, interrupted: false)
        XCTAssertTrue(wrote)
        let h = await s.history
        XCTAssertTrue(h.isEmpty, "kein Beitrag ohne Gesprochenes")
        let entries = await audit.entries().filter { $0.kind == .answer }
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
        await s.commit(requestId: "r1", user: "Hallo", spoken: .nothing, interrupted: false)
        let entries = await audit.entries().filter { $0.kind == .answer }
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
        await done.commit(requestId: "fertig", user: question, spoken: .nothing, interrupted: true)
        await done.commit(requestId: "fertig", user: question, spoken: .nothing, interrupted: true)   // zweites Mal: kein zweiter Eintrag
        let finished = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(finished.count, 1)
        XCTAssertEqual(finished[0].outcome, .interrupted)
        XCTAssertEqual(finished[0].round, .data)
        let doneHistory = await done.history
        XCTAssertTrue(doneHistory.isEmpty)

        let hangingLog = Log()
        let hanging = await session(ScriptModel(pieces: [castorpModel + " Er", " bleibt drei Wochen."], hang: true, log: hangingLog),
                                    config: strict, audit: audit)
        let task = Task { await collect(hanging, AssistantRequest(requestId: "haengt", text: question)) }
        await eventually { hangingLog.pieces >= 2 }   // beide Stücke sind da, das Modell hängt
        XCTAssertGreaterThanOrEqual(hangingLog.pieces, 2)
        await hanging.cancel(requestId: "haengt")
        let wrote = await hanging.commit(requestId: "haengt", user: question, spoken: .nothing, interrupted: true)
        XCTAssertTrue(wrote)
        let seen = Seen(events: await task.value)
        XCTAssertEqual(seen.failure, .cancelled)
        let all = await audit.entries().filter { $0.kind == .answer }
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
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)

        let entries = await audit.entries().filter { $0.kind == .answer }
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
            AssistantCommitMark(stage: .audio, ok: false, generation: g),   // gemeldet unter g + 1: „verworfen“ mit der gefundenen Generation (R5)
            AssistantCommitMark(stage: .history, ok: true, generation: g),
        ])
        XCTAssertEqual(e.trace[0].commits, [], "verworfene Sätze haben keinen Ausgabe-Commit")
    }

    func testAuditFileContainsNoContentOfTheConversation() async {
        let dir = tempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let audit = AssistantAuditLog(directory: dir)
        let s = await session(ScriptModel(pieces: [castorpModel]), config: strict, audit: audit)
        _ = await collect(s, AssistantRequest(requestId: "r1", text: question))
        await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
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
        await s.commit(requestId: "r1", user: "Hallo", spoken: .all, interrupted: false)
        let t = await session(ScriptModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), config: open, audit: audit)
        _ = await collect(t, AssistantRequest(requestId: "r2", text: "Wie hoch ist der Mont Blanc?"))
        await t.commit(requestId: "r2", user: "Wie hoch ist der Mont Blanc?", spoken: .all, interrupted: false)
        let entries = await audit.entries().filter { $0.kind == .answer }
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
        let model = ScriptModel(pieces: ["Hunde schwitzen vor allem über die Pfoten. Katzen", " putzen sich mehrmals am Tag."], holdAfter: 1)
        let s = await session(model, config: open, audit: audit)
        var spoken: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Erzähl etwas über Tiere")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
            if case .sentence(_, _, _, _, .unsourced, _, _) = e { _ = await s.setAllowUnsourced(false) }
        }
        let wrote = await s.commit(requestId: "r1", user: "Erzähl etwas über Tiere", spoken: .all, interrupted: false)
        XCTAssertTrue(wrote)
        let entries = await audit.entries().filter { $0.kind == .answer }
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
        let ok = await s.commit(requestId: "r1", user: question, spoken: .all, interrupted: false)
        XCTAssertTrue(ok)
        let h = await s.history
        XCTAssertEqual(h.count, 1)
    }

    // MARK: Prompt

    func testDataPromptHasExcerptsAndHistoryButNoInventory() async {
        let log = Log()
        let hist = [AssistantTurn(user: "Frühere Frage", assistant: "Frühere Antwort", interrupted: true).shown]
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
        await s.setAllowUnsourced(true)
        _ = await collect(s, AssistantRequest(requestId: "r2", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("Unter AUSZÜGE stehen") ?? false,
                      "das Gespräch hatte schon eine Datenrunde: Einschalten gibt keine Runde ohne Quelle frei")
        XCTAssertTrue(s.canResumeFree)
        let r8 = await s.resumeFree()
        XCTAssertTrue(r8.changed)
        _ = await collect(s, AssistantRequest(requestId: "r3", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("In dieser Runde siehst du keine Inhalte") ?? false, "erst nach `resumeFree` eine Allgemeinrunde")
        XCTAssertFalse(log.all.last?.prompt.contains("AUSZÜGE") ?? true)
        let fresh = await session(ScriptModel(pieces: ["Gut."], log: log), config: strict)
        await fresh.setAllowUnsourced(true)
        _ = await collect(fresh, AssistantRequest(requestId: "r4", text: "Wie hoch ist der Mont Blanc?"))
        XCTAssertTrue(log.all.last?.instructions.contains("In dieser Runde siehst du keine Inhalte") ?? false,
                      "ein Gespräch ohne Datenrunde: Einschalten setzt den Zustand frei")
    }
}
