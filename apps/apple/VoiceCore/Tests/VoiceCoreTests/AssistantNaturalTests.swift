import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“, Stil `natural` (ADR-0028 E4.2c, Teil B): Sprecher und Denker, Suche mit Überbrückung, Protokollfolge,
// Ansagen für eine langsame Antwort. Künstliche Modelle unterscheiden Sprecher- und Denker-Aufruf an der Anweisung (nur die des
// Sprechers nennt „SUCHE:“). Nur synthetische Daten.
//
// Fehlerfälle dieses Pakets (Nebenläufigkeit, Abbruch mitten im Vorgang, voller Datenträger, fehlendes Gerät, Absturz des Modells):
// - Nebenläufigkeit: `testConcurrentNaturalRequests…`, `testCommitNotesAttachAndNoteRaceNeverLosesTheCommit`, Ansagen nie nebenläufig
//   (`testFillerSteps…`).
// - Abbruch mitten im Vorgang: `testEveryPrefixOfAJournal…`, `testCancel…`, `testRevocationBetweenThePhases…`, `testFillersEnd…`.
// - Voller Datenträger / Schreibfehler: `testFailing…LineMeansNoModelAndNoOutput`, `testNoRetrievalWhenTheDataRoundLineFails`.
// - Fehlendes Gerät / Modell nicht verfügbar: `testModelErrorsMapToTheFailureOfTheRun`.
// - Absturz eines Kindprozesses (hier: das Modell bricht mitten im Strom mit einem Fehler ab): `testModelCrashInTheMiddleOfTheThinkerStream…`.
// Erzwungene Suche (direkter Inhaltsbezug, Abschnitt „Erzwungene Suche“ am Ende):
// - Nebenläufigkeit: Frist und Modellstrom laufen in DENSELBEN Briefkasten, einen Verbraucher (`testMailboxDeadline…`,
//   `testConcurrentForcedRequests…`, `testSpeakerAnswerAndDeadlineAtTheSameTime…`, `testAStaleDeadline…`).
// - Abbruch mitten im Vorgang: `testCancelWhileTheForcedSpeakerRuns…`, `testRevocationWhileTheForcedSpeakerRuns…`.
// - Voller Datenträger: `testWriteFailuresInTheForcedSearch…` (Rundenzeile, Schrittzeile des Zwischensatzes, Rundenzeile der Daten).
// - Modell nicht verfügbar / Absturz im Strom: `testAForcedSpeakerError…` (Fehler sofort, nach dem Satz, mitten im Satz, Überlauf).
// - Hängender Sprecher: `testAHangingForcedSpeaker…` (die Frist beendet die Stufe, die Suche läuft trotzdem).

// MARK: Hilfen

/// Zähler mit unteilbarem Erhöhen (mehrere Zeitgeber rufen gleichzeitig).
private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    @discardableResult
    func increment() -> Int { lock.withLock { n += 1; return n } }
    var value: Int { lock.withLock { n } }
}

/// Haltepunkt statt fester Wartezeit: `wait` kehrt zurück, sobald `open()` gerufen wurde (auch wenn das schon vorher geschah), und gibt
/// zurück, ob er wirklich geöffnet wurde. Die Frist (60 s) ist nur ein Netz gegen ein nie endendes Warten, keine Abstimmung. Ein
/// abgebrochener Wartender kehrt sofort zurück.
private final class Latch: @unchecked Sendable {
    private let lock = NSLock()
    private var opened = false
    private var waiters: [UUID: CheckedContinuation<Void, Never>] = [:]

    func open() {
        let all: [UUID: CheckedContinuation<Void, Never>] = lock.withLock {
            opened = true
            let w = waiters
            waiters = [:]
            return w
        }
        for continuation in all.values { continuation.resume() }
    }

    var isOpen: Bool { lock.withLock { opened } }

    private func release(_ id: UUID) {
        let continuation = lock.withLock { waiters.removeValue(forKey: id) }
        continuation?.resume()
    }

    @discardableResult
    func wait(timeoutSeconds: UInt64 = 60) async -> Bool {
        let id = UUID()
        let timer = Task { [self] in
            try? await Task.sleep(nanoseconds: timeoutSeconds * 1_000_000_000)
            if !Task.isCancelled { release(id) }
        }
        await withTaskCancellationHandler {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                let ready: Bool = lock.withLock {
                    if opened || Task.isCancelled { return true }
                    waiters[id] = continuation
                    return false
                }
                if ready { continuation.resume() }
            }
        } onCancel: {
            release(id)
        }
        timer.cancel()
        return isOpen
    }
}

private enum Role: String, Sendable { case speaker, thinker }

private final class ModelCalls: @unchecked Sendable {
    private let lock = NSLock()
    private var entries: [(role: Role, instructions: String, prompt: String)] = []
    private var terminated = 0
    private var latches: [String: Latch] = [:]
    /// Nummer des Aufrufs dieser Rolle (ab 1).
    func hit(_ role: Role, _ instructions: String, _ prompt: String) -> Int {
        let (number, latch): (Int, Latch?) = lock.withLock {
            entries.append((role, instructions, prompt))
            let n = entries.filter { $0.role == role }.count
            return (n, latches["call:\(role.rawValue):\(n)"])
        }
        latch?.open()
        return number
    }
    func ended() {
        let latch: Latch? = lock.withLock {
            terminated += 1
            return latches["end:\(terminated)"]
        }
        latch?.open()
    }

    private func latch(_ key: String, reached: () -> Bool) -> Latch {
        lock.withLock {
            if let known = latches[key] { return known }
            let made = Latch()
            latches[key] = made
            if reached() { made.open() }
            return made
        }
    }
    /// Haltepunkt: kehrt zurück, sobald das Modell-Double den `count`-ten Aufruf dieser Rolle angenommen hat.
    @discardableResult
    func accepted(_ role: Role, _ count: Int = 1) async -> Bool {
        let latch = latch("call:\(role.rawValue):\(count)") { entries.filter { $0.role == role }.count >= count }
        return await latch.wait()
    }
    /// Haltepunkt: kehrt zurück, sobald `count` Modellströme beendet sind (abgebrochen oder zu Ende).
    @discardableResult
    func ended(_ count: Int) async -> Bool {
        let latch = latch("end:\(count)") { terminated >= count }
        return await latch.wait()
    }
    var roles: [Role] { lock.withLock { entries.map(\.role) } }
    func prompts(_ role: Role) -> [String] { lock.withLock { entries.filter { $0.role == role }.map(\.prompt) } }
    func instructions(_ role: Role) -> [String] { lock.withLock { entries.filter { $0.role == role }.map(\.instructions) } }
    func count(_ role: Role) -> Int { lock.withLock { entries.filter { $0.role == role }.count } }
    var terminations: Int { lock.withLock { terminated } }
}

/// Eine Rolle des künstlichen Modells: Stücke mit Pause VOR dem i-ten Stück, danach Ende, Fehler oder Hängen. `holds[i]` hält das i-te Stück
/// zurück, bis der Haltepunkt offen ist (davor, dann kommt die Pause): So hängt die Reihenfolge der Ereignisse nicht von der Geschwindigkeit
/// des Rechners ab.
private struct Script: Sendable {
    var pieces: [String] = []
    var pausesMs: [UInt64] = []
    var fail: (any Error)?
    var hang = false
    /// Die ersten so vielen Aufrufe dieser Rolle werfen `contextOverflow`.
    var overflows = 0
    var holds: [Int: Latch] = [:]

    /// Dasselbe Skript, dessen i-tes Stück auf `latch` wartet.
    func holding(_ index: Int = 0, until latch: Latch) -> Script {
        var copy = self
        copy.holds[index] = latch
        return copy
    }

    func play(_ c: AsyncThrowingStream<String, Error>.Continuation) async {
        for (i, p) in pieces.enumerated() {
            if let hold = holds[i] { await hold.wait() }
            let pause = i < pausesMs.count ? pausesMs[i] : 0
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
}

private struct RoleModel: BookTalkModel {
    var speaker = Script()
    var thinker = Script()
    var calls = ModelCalls()
    var onCall: (@Sendable (Role) -> Void)?

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let role: Role = instructions.contains(AssistantNatural.searchMarker) ? .speaker : .thinker
        let n = calls.hit(role, instructions, prompt)
        onCall?(role)
        let script = role == .speaker ? speaker : thinker
        let calls = calls
        return AsyncThrowingStream { c in
            if n <= script.overflows {
                c.finish(throwing: BookTalkModelError.contextOverflow)
                return
            }
            let task = Task { await script.play(c) }
            c.onTermination = { _ in calls.ended(); task.cancel() }
        }
    }
}

/// Wie `RoleModel`, aber der Sprecher-Aufruf MIT offenem Suchauftrag (Block „OFFENER SUCHAUFTRAG“ im Prompt) hat ein eigenes Skript.
private struct PendingAwareModel: BookTalkModel {
    var plainSpeaker = Script()
    var pendingSpeaker = Script()
    var thinker = Script()
    var calls = ModelCalls()

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let isSpeaker = instructions.contains(AssistantNatural.searchMarker)
        _ = calls.hit(isSpeaker ? .speaker : .thinker, instructions, prompt)
        let script = !isSpeaker ? thinker : (prompt.contains("OFFENER SUCHAUFTRAG") ? pendingSpeaker : plainSpeaker)
        let calls = calls
        return AsyncThrowingStream { c in
            let task = Task { await script.play(c) }
            c.onTermination = { _ in calls.ended(); task.cancel() }
        }
    }
}

private struct Crash: Error {}
private struct DiskFull: Error {}

private final class Box<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: T
    init(_ value: T) { stored = value }
    var value: T {
        get { lock.withLock { stored } }
        set { lock.withLock { stored = newValue } }
    }
}

private final class FailSwitch: @unchecked Sendable {
    private let lock = NSLock()
    private var flag = false
    var on: Bool {
        get { lock.withLock { flag } }
        set { lock.withLock { flag = newValue } }
    }
}

/// Eine Quelle, die den Abruf bemerkt (Bereich Notizen) und dabei einen Zustand der Protokolldatei festhält.
private struct ProbeSource: AssistantSource {
    let area: AssistantArea = .notes
    var onSearch: @Sendable () -> Void
    func items() async -> [AssistantItem] { [] }
    func search(_ question: String, limit: Int) async -> [AssistantPassage] {
        onSearch()
        return []
    }
}

private final class Snaps: @unchecked Sendable {
    private let lock = NSLock()
    private var texts: [String] = []
    func add(_ text: String) { lock.withLock { texts.append(text) } }
    var all: [String] { lock.withLock { texts } }
}

private struct Seen {
    struct Sentence: Equatable {
        let text: String
        let status: AssistantSentenceStatus
        let sources: [AssistantPassage]
    }
    let events: [AssistantEvent]
    var sentences: [Sentence] {
        events.compactMap { e in
            if case .sentence(_, _, let t, _, let st, let src, _) = e { Sentence(text: t, status: st, sources: src) } else { nil }
        }
    }
    var statuses: [AssistantSentenceStatus] { sentences.map(\.status) }
    var texts: [String] { sentences.map(\.text) }
    var spoken: [String] { sentences.filter { $0.status != .announcement }.map(\.text) }
    var announcements: [String] { sentences.filter { $0.status == .announcement }.map(\.text) }
    var answer: AssistantAnswer? { if case .done(_, let a)? = events.last { a } else { nil } }
    var failure: AssistantFailure? { if case .failed(_, let f)? = events.last { f } else { nil } }
    var sourceEvents: [[AssistantPassage]] {
        events.compactMap { if case .sources(_, _, let p) = $0 { p } else { nil } }
    }
    var progress: [AssistantProgress] {
        events.compactMap { if case .progress(_, _, let p) = $0 { p } else { nil } }
    }
    private static func shape(_ p: AssistantProgress) -> String {
        switch p {
        case .speaker: "speaker"
        case .searching: "searching"
        case .found(let titles): "found:" + titles.joined(separator: ",")
        case .composing: "composing"
        case .draft: "draft"
        case .abandon: "abandon"
        }
    }
    /// Der Fortschritt in kurzer Form (Entwürfe als „draft“), damit die Reihenfolge lesbar bleibt.
    var progressShape: [String] { progress.map(Self.shape) }
    /// Fortschritt und Sätze in der Reihenfolge des Eintreffens (Sätze als „sentence“, Entwürfe als „draft“).
    var orderShape: [String] {
        events.compactMap { e in
            switch e {
            case .progress(_, _, let p): Self.shape(p)
            case .sentence: "sentence"
            default: nil
            }
        }
    }
    var drafts: [String] { progress.compactMap { if case .draft(let t) = $0 { t } else { nil } } }
}

private func collect(_ s: AssistantSession, _ r: AssistantRequest) async -> Seen {
    var out: [AssistantEvent] = []
    for await e in s.events(r) { out.append(e) }
    return Seen(events: out)
}

private let castorp = "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."
private let stay = "Er bleibt drei Wochen."
private let nordlicht = "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag."
/// Kein Eigendatenbezug (kein Titel, kein Stamm, kein Besitz): geht zum Sprecher.
private let castorpQuestion = "Wohin fährt Hans Castorp?"
private let weather = "Wie ist das Wetter?"
/// Direkter Inhaltsbezug („meinem“).
private let ownQuestion = "Was steht in meinem Buch über Davos?"
private let inventoryQuestion = "Welche Bücher habe ich?"
private let searchLine = "SUCHE: Davos Sanatorium Castorp\n"

private func journalTags(_ lines: [AssistantJournalLine]) -> [String] {
    lines.compactMap { l -> String? in
        switch l.t {
        case .start: "start:\(l.style?.rawValue ?? "verbatim")"
        case .round: "round:\(l.round?.rawValue ?? "-")/\(l.reason?.rawValue ?? "-")"
        case .areas: "areas:" + (l.areas ?? []).joined(separator: ",")
        case .step: "step:\(l.verdict?.rawValue ?? "-")"
        case .end: "end"
        default: nil
        }
    }
}

private func naturalTurn(_ user: String, _ assistant: String, round: AssistantRoundKind, sources: [AssistantPassage] = []) -> AssistantTurn {
    AssistantTurn(user: user, assistant: assistant, sources: sources, general: sources.isEmpty,
                  parts: [AssistantTurnPart(text: assistant, unsourced: false,
                                            note: sources.isEmpty ? AssistantTurnPart.conversationNote : AssistantTurnPart.summaryNote)],
                  model: "Test", at: Date(), round: round, style: .natural)
}

final class AssistantNaturalTests: XCTestCase {
    private var dirs: [URL] = []

    override func tearDown() {
        for d in dirs { try? FileManager.default.removeItem(at: d) }
        dirs = []
        super.tearDown()
    }

    private func tempDir() -> URL {
        let d = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-natural-\(UUID().uuidString)", isDirectory: true)
        dirs.append(d)
        return d
    }

    /// Die Standardwerte der Floskeln sind aus (`bridge`, `fillers`); wer die Zeitgeber testet, gibt `first`/`second` an und schaltet sie damit ein.
    private func config(bridge: Bool = false, cloud: Bool = false, maxSentences: Int = 4, passageLimit: Int = 4,
                        passageChars: Int = 2_400, first: Int? = nil, second: Int? = nil, timeout: TimeInterval = 5,
                        tokens: Int = 4_096, forcedMs: Int? = nil) -> AssistantConfig {
        AssistantConfig(modelLabel: "Test", isCloud: cloud, contextTokens: tokens, maxAnswerTokens: 400, userName: "Patrick",
                        timeout: timeout, style: .natural, bridge: bridge, naturalPassageLimit: passageLimit,
                        naturalPassageChars: passageChars, naturalMaxSentences: maxSentences, fillers: first != nil || second != nil,
                        fillerFirstMs: first ?? 1_200, fillerSecondMs: second ?? 8_000, forcedSpeakerMs: forcedMs ?? 2_500)
    }

    private func books() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorp), ("S. 13", stay)]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func booksAndMeeting() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [("S. 12", castorp), ("S. 13", stay)]))
        await corpus.upsert(assistantDoc("t1", "Besprechung", area: .transcripts, sections: [("00:14:03", nordlicht)]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func newSession(_ model: any BookTalkModel, config c: AssistantConfig? = nil, history: [AssistantTurn] = [],
                            audit: AssistantAuditLog? = nil, knowledge k: AssistantKnowledge? = nil, cloudConsent: Bool = true,
                            stepIndex: AssistantStepIndex = AssistantStepIndex()) async -> AssistantSession {
        let know: AssistantKnowledge
        if let k { know = k } else { know = await books() }
        return AssistantSession(model: model, knowledge: know, config: c ?? config(), history: history, audit: audit, released: true,
                                cloudConsent: cloudConsent, stepIndex: stepIndex)
    }

    private func ask(_ s: AssistantSession, _ text: String = castorpQuestion, id: String = "r1") async -> Seen {
        await collect(s, AssistantRequest(requestId: id, text: text))
    }

    private func close(_ s: AssistantSession, _ id: String = "r1", user: String = castorpQuestion) async {
        await s.commit(requestId: id, user: user, spoken: .all, interrupted: false)
    }

    private func journal(_ audit: AssistantAuditLog, _ id: String = "r1") -> [AssistantJournalLine] {
        let raw = (try? String(contentsOf: audit.url, encoding: .utf8)) ?? ""
        return raw.split(separator: "\n").compactMap { AssistantJournalFold.journal(Data($0.utf8)) }.filter { $0.req == id }
    }

    private func rawJournal(_ audit: AssistantAuditLog) -> String { (try? String(contentsOf: audit.url, encoding: .utf8)) ?? "" }

    private func failingAppender(_ needle: String, _ sw: FailSwitch) -> @Sendable (URL, Data) throws -> Void {
        { url, data in
            if sw.on, String(decoding: data, as: UTF8.self).contains(needle) { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        }
    }

    private let search = Script(pieces: [searchLine])
    private let answer = Script(pieces: ["Castorp fährt ins Sanatorium nach Davos. ", "Dort besucht er seinen Vetter Joachim."])

    // MARK: Direkte Antwort des Sprechers

    /// Wetterfrage: Der Sprecher antwortet frei, ohne Auszüge, ohne Quellen, ohne Abruf; kein Denker.
    func testWeatherQuestionIsAnsweredDirectlyAsConversationalWithoutSources() async {
        let model = RoleModel(speaker: Script(pieces: ["Das Wetter kenne ich leider nicht. ", "Ich habe keinen Internetzugang."]))
        let s = await newSession(model)
        let seen = await ask(s, weather)

        XCTAssertEqual(seen.statuses, [.conversational, .conversational])
        XCTAssertEqual(seen.spoken, ["Das Wetter kenne ich leider nicht.", "Ich habe keinen Internetzugang."])
        XCTAssertTrue(seen.sentences.allSatisfy { $0.sources.isEmpty })
        XCTAssertEqual(model.calls.roles, [.speaker], "kein Denker")
        let prompt = model.calls.prompts(.speaker)[0]
        XCTAssertFalse(prompt.contains("AUSZÜGE"), "keine Auszüge im Prompt des Sprechers")
        XCTAssertFalse(prompt.contains("Castorp"))
        XCTAssertEqual(seen.sourceEvents, [[]], "nur die leere Vorab-Anzeige")
        guard let a = seen.answer else { return XCTFail("done fehlt: \(seen.events)") }
        XCTAssertEqual(a.counts.conversational, 2)
        XCTAssertEqual(a.counts.summarized, 0)
        XCTAssertEqual(a.round, .general)
        XCTAssertTrue(a.sources.isEmpty)
        XCTAssertTrue(a.general)
        XCTAssertFalse(a.unsourced)
    }

    func testSpeakerPromptIsTheSpeakerInstructionAndNamesTheSearchProtocol() async {
        let model = RoleModel(speaker: Script(pieces: ["Hallo!"]))
        let s = await newSession(model)
        _ = await ask(s, "Hallo")
        let instructions = model.calls.instructions(.speaker)[0]
        XCTAssertTrue(instructions.contains("SUCHE:"))
        XCTAssertTrue(instructions.contains("Internetzugang"))
        XCTAssertFalse(instructions.contains("AUSZÜGE"))
    }

    // MARK: Suche

    /// `SUCHE:`-Zeile: Überbrückung zuerst (mit `bridge`), dann `.summarized` mit den Auszügen als Quellen; nichts von der Zeile wird ausgegeben.
    func testSearchLineGivesBridgeThenSummarizedSentencesWithSources() async {
        let model = RoleModel(speaker: search, thinker: answer)
        let s = await newSession(model, config: config(bridge: true))
        let seen = await ask(s)

        XCTAssertEqual(seen.statuses, [.announcement, .summarized, .summarized])
        XCTAssertTrue(AssistantNatural.bridges.contains(seen.texts.first ?? ""))
        XCTAssertEqual(seen.spoken, ["Castorp fährt ins Sanatorium nach Davos.", "Dort besucht er seinen Vetter Joachim."])
        XCTAssertFalse(seen.texts.joined().contains("SUCHE"))
        XCTAssertFalse(seen.texts.joined().contains("Sanatorium Castorp\n"))
        for sentence in seen.sentences where sentence.status == .summarized {
            XCTAssertEqual(Set(sentence.sources.map(\.itemId)), ["b1"])
        }
        XCTAssertEqual(model.calls.roles, [.speaker, .thinker])
        let prompt = model.calls.prompts(.thinker)[0]
        XCTAssertTrue(prompt.contains("AUSZÜGE"))
        XCTAssertTrue(prompt.contains("Sanatorium"))
        guard let a = seen.answer else { return XCTFail("done fehlt: \(seen.events)") }
        XCTAssertEqual(a.counts.summarized, 2)
        XCTAssertEqual(a.counts.conversational, 0)
        XCTAssertEqual(a.round, .data)
        XCTAssertEqual(a.sources.map(\.itemId), ["b1"])
        XCTAssertEqual(a.text, "Castorp fährt ins Sanatorium nach Davos. Dort besucht er seinen Vetter Joachim.", "die Überbrückung ist nicht im Antworttext")
        XCTAssertFalse(a.general)
        XCTAssertEqual(seen.sourceEvents.count, 2, "leer vor dem Sprecher, dann die verwendeten Auszüge")
        XCTAssertEqual(seen.sourceEvents[0], [])
        XCTAssertEqual(seen.sourceEvents[1].map(\.itemId), ["b1"])
    }

    func testNoBridgeWithoutTheBridgeSetting() async {
        let s = await newSession(RoleModel(speaker: search, thinker: answer), config: config(bridge: false))
        let seen = await ask(s)
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
        XCTAssertTrue(seen.announcements.isEmpty)
    }

    /// Erkennung am Stromanfang: Groß-/Kleinschreibung, führende Leerzeile, Anführungszeichen, Markdown, Leerzeichen vor dem Doppelpunkt,
    /// über Stückgrenzen hinweg.
    func testSearchMarkerVariantsAreRecognized() async {
        let variants: [[String]] = [
            ["suche: Davos Sanatorium"], ["\nSUCHE: Davos Sanatorium"], ["\"Suche : Davos Sanatorium\""], ["  **SUCHE:** Davos Sanatorium"],
            ["SU", "CHE", ": Da", "vos Sanatorium\n"], ["Suche:Davos Sanatorium"], ["Suche\u{00A0}: Davos Sanatorium"],
        ]
        for (i, pieces) in variants.enumerated() {
            let model = RoleModel(speaker: Script(pieces: pieces), thinker: answer)
            let s = await newSession(model)
            let seen = await ask(s, castorpQuestion, id: "v\(i)")
            XCTAssertEqual(seen.statuses, [.summarized, .summarized], "Variante \(i): \(pieces)")
            XCTAssertEqual(model.calls.roles, [.speaker, .thinker], "Variante \(i)")
            XCTAssertFalse(seen.texts.joined().lowercased().contains("suche"), "Variante \(i)")
        }
    }

    /// Das Wort „suche“ mitten im Satz ist keine Suche; eine Protokollzeile hinter einem Satz wird nie gesprochen.
    func testSucheInsideAnAnswerIsNoSearchAndProtocolLinesAreNeverSpoken() async {
        let model = RoleModel(speaker: Script(pieces: ["Ich suche gern nach Kaffee. ", "Wirklich."]))
        let seen = await ask(await newSession(model))
        XCTAssertEqual(seen.spoken, ["Ich suche gern nach Kaffee.", "Wirklich."])
        XCTAssertEqual(model.calls.roles, [.speaker])

        let model2 = RoleModel(speaker: Script(pieces: ["Moin! ", "suche: Davos."]))
        let seen2 = await ask(await newSession(model2))
        XCTAssertEqual(seen2.spoken, ["Moin!"])
        XCTAssertEqual(seen2.answer?.counts.droppedTemplate, 1, "die Protokollzeile ist verworfen")
        XCTAssertEqual(model2.calls.roles, [.speaker], "keine Suche, wenn die Zeile nicht am Zeilenanfang oder in Großbuchstaben hinter einem Satzende steht")
    }

    /// Eine Suche ohne Suchwörter sucht mit der Frage.
    func testEmptySearchWordsSearchWithTheQuestion() async {
        let model = RoleModel(speaker: Script(pieces: ["SUCHE:\n"]), thinker: answer)
        let seen = await ask(await newSession(model))
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
        XCTAssertTrue(model.calls.prompts(.thinker)[0].contains("Sanatorium"), "Treffer über die Frage")
    }

    func testSearchWordsAreAddedToTheQuestionForTheRetrieval() async {
        // Die Frage allein trifft nichts; die Suchwörter des Sprechers führen zu den Auszügen.
        let model = RoleModel(speaker: Script(pieces: ["SUCHE: Sanatorium Davos Vetter Joachim\n"]), thinker: answer)
        let seen = await ask(await newSession(model), "Wie heißt der Mann?")
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
        XCTAssertTrue(model.calls.prompts(.thinker)[0].contains("Joachim"))
    }

    /// Direkter Inhaltsbezug: der Sprecher läuft (erzwungene Suche, ohne eigenen Satz hier), danach die Daten; die Suche startet in jedem Fall.
    func testDirectContentReferenceRunsTheForcedSpeakerThenTheData() async {
        let model = RoleModel(thinker: answer)
        let s = await newSession(model, config: config(bridge: true))
        let seen = await ask(s, ownQuestion)
        XCTAssertEqual(model.calls.roles, [.speaker, .thinker], "erst der Sprecher, dann der Denker")
        XCTAssertEqual(seen.statuses, [.announcement, .summarized, .summarized], "der leere Sprecher gibt keinen Satz")
        XCTAssertEqual(seen.answer?.round, .data)
        XCTAssertTrue((model.calls.prompts(.speaker).first ?? "").contains("SUCHE LÄUFT BEREITS"), "der Zusatz der erzwungenen Suche")
        XCTAssertFalse((model.calls.prompts(.thinker).first ?? "").contains("SUCHE LÄUFT BEREITS"), "nur der Sprecher bekommt ihn")
    }

    func testNoPassagesGiveTheFixedSentenceWithoutThinkerCall() async {
        let model = RoleModel(speaker: Script(pieces: ["SUCHE: Einkaufsliste Kühlschrank Eier\n"]), thinker: answer)
        let s = await newSession(model, config: config(bridge: true))
        let seen = await ask(s, "Wie viele Eier brauche ich?")
        XCTAssertEqual(seen.statuses.suffix(1), [.neutral])
        XCTAssertEqual(seen.texts.last, AssistantNatural.nothingFoundSentence)
        XCTAssertEqual(model.calls.roles, [.speaker], "der Denker läuft nicht")
        XCTAssertEqual(seen.answer?.counts.neutral, 1)
        XCTAssertEqual(seen.answer?.counts.summarized, 0)
    }

    func testInventoryQuestionIsAnsweredBySystemWithoutAnyModel() async {
        let model = RoleModel(speaker: Script(pieces: ["Unsinn."]), thinker: Script(pieces: ["Unsinn."]))
        let seen = await ask(await newSession(model), inventoryQuestion)
        XCTAssertTrue(model.calls.roles.isEmpty, "kein Modell")
        XCTAssertEqual(seen.statuses.first, .announcement)
        XCTAssertTrue(seen.statuses.dropFirst().allSatisfy { $0 == .inventory })
        XCTAssertFalse(seen.statuses.dropFirst().isEmpty)
        XCTAssertEqual(seen.answer?.round, .data)
    }

    // MARK: Grenzen und Verwerfen

    func testMaxSentencesLimitsTheFreeSentencesOfEachPhase() async {
        let many = Script(pieces: ["Eins. ", "Zwei. ", "Drei. ", "Vier."])
        let model = RoleModel(speaker: many)
        let seen = await ask(await newSession(model, config: config(maxSentences: 2)))
        XCTAssertEqual(seen.spoken, ["Eins.", "Zwei."])
        XCTAssertEqual(seen.answer?.counts.conversational, 2)
        let ended = await model.calls.ended(1)
        XCTAssertTrue(ended, "der Modellstrom wurde abgebrochen")

        let model2 = RoleModel(speaker: search, thinker: many)
        let seen2 = await ask(await newSession(model2, config: config(maxSentences: 3)), castorpQuestion, id: "r2")
        XCTAssertEqual(seen2.spoken, ["Eins.", "Zwei.", "Drei."])
    }

    /// Sätze des Systems (Fehl-Vorlagen, feste Antworten, Überbrückung) spricht nur das System: Modelltext damit wird verworfen.
    func testSystemSentencesFromTheModelAreDropped() async {
        let speakerSaysIt = Script(pieces: ["Dazu finde ich in deinen Inhalten nichts. ", "Ich schaue kurz nach. ", "Hallo!"])
        let seen = await ask(await newSession(RoleModel(speaker: speakerSaysIt)))
        XCTAssertEqual(seen.spoken, ["Hallo!"])
        XCTAssertEqual(seen.answer?.counts.droppedTemplate, 2)
    }

    func testSpeakerThatOnlySaysDroppedThingsGivesTheFixedNotUnderstoodSentence() async {
        let seen = await ask(await newSession(RoleModel(speaker: Script(pieces: ["Dazu finde ich in deinen Inhalten nichts."]))))
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.texts, [AssistantNatural.notUnderstoodSentence])
        XCTAssertNil(seen.failure)
    }

    func testThinkerThatOnlySaysDroppedThingsGivesTheFixedNothingFoundSentence() async {
        let thinker = Script(pieces: ["Dazu habe ich in deinen Inhalten nichts Passendes gefunden."])
        let seen = await ask(await newSession(RoleModel(speaker: search, thinker: thinker)))
        XCTAssertEqual(seen.statuses, [.neutral])
        XCTAssertEqual(seen.texts, [AssistantNatural.nothingFoundSentence])
    }

    func testEmptySpeakerStreamIsAFailureOfTheModel() async {
        let seen = await ask(await newSession(RoleModel(speaker: Script(pieces: []))))
        XCTAssertEqual(seen.failure, .modelFailed)
        XCTAssertTrue(seen.sentences.isEmpty)
    }

    func testTruncatedSentenceWithoutPunctuationIsDroppedAsFragment() async {
        let seen = await ask(await newSession(RoleModel(speaker: Script(pieces: ["Hallo zusammen. ", "Und dann wollte ich noch"]))))
        XCTAssertEqual(seen.spoken, ["Hallo zusammen."])
        XCTAssertEqual(seen.answer?.counts.droppedFragment, 1)
    }

    // MARK: Überbrückung

    /// Nie im Verlauf, nie im Antworttext, nie zweimal derselbe hintereinander (je Sitzung), reihum.
    func testBridgeIsNeverInHistoryAndRotatesWithoutRepeating() async {
        let model = RoleModel(speaker: search, thinker: answer)
        let s = await newSession(model, config: config(bridge: true))
        var bridges: [String] = []
        for i in 0..<5 {
            let id = "b\(i)"
            let seen = await ask(s, castorpQuestion, id: id)
            bridges += seen.announcements
            await close(s, id)
        }
        XCTAssertEqual(bridges.count, 5)
        for (a, b) in zip(bridges, bridges.dropFirst()) { XCTAssertNotEqual(a, b, "nie zweimal derselbe hintereinander") }
        XCTAssertEqual(Set(bridges.prefix(AssistantNatural.bridges.count)).count, AssistantNatural.bridges.count, "reihum durch die ganze Liste")
        XCTAssertTrue(bridges.allSatisfy { AssistantNatural.bridges.contains($0) })
        let history = await s.history
        XCTAssertEqual(history.count, 5)
        for turn in history {
            XCTAssertFalse(AssistantNatural.bridges.contains { turn.assistant.contains($0) })
            XCTAssertFalse(turn.parts.contains { part in AssistantNatural.bridges.contains { part.text.contains($0) } })
        }
    }

    func testBridgeRotationUnderConcurrentTakesIsBalanced() async {
        let rotation = AssistantBridgeRotation()
        let taken = await withTaskGroup(of: String.self) { group -> [String] in
            for _ in 0..<40 { group.addTask { rotation.take() } }
            var all: [String] = []
            for await t in group { all.append(t) }
            return all
        }
        for bridge in AssistantNatural.bridges {
            XCTAssertEqual(taken.filter { $0 == bridge }.count, 40 / AssistantNatural.bridges.count)
        }
    }

    /// Kein Ansage- oder Überbrückungstext enthält ein Stoppwort (stopp, stop, halt, warte, moment; normalisiert): Das Mikrofon könnte
    /// die eigene Ausgabe sonst als Stoppbefehl hören.
    func testNoAnnouncementOrBridgeTextContainsAStopWord() {
        XCTAssertEqual(AssistantNatural.stoppWords, ["stopp", "stop", "halt", "warte", "moment"])
        let preface = AssistantGrounding.quotePreface(for: assistantPassage(title: "Der Zauberberg", text: castorp))
        let all = AssistantNatural.announcementTexts
            + [AssistantGrounding.inventoryAnnouncement, AssistantGrounding.unsourcedAnnouncement, preface]
        XCTAssertEqual(AssistantNatural.announcementTexts.count, AssistantNatural.bridges.count + 2)
        XCTAssertEqual(AssistantNatural.bridges, ["Ich schaue kurz nach.", "Ich suche das heraus.", "Einen Augenblick, ich sehe nach.",
                                                  "Das schlage ich kurz nach."])
        XCTAssertEqual(AssistantNatural.fillerFirstSentence, "Einen Augenblick.")
        XCTAssertEqual(AssistantNatural.fillerSecondSentence, "Das dauert noch etwas.")
        for text in all {
            let words = Set(AssistantGrounding.normalize(text).split(separator: " ").map(String.init))
            XCTAssertTrue(words.isDisjoint(with: AssistantNatural.stoppWords), "Stoppwort in „\(text)“: \(words)")
        }
        // Die neuen Texte (Überbrückung, Ansagen für eine langsame Antwort) enthalten die Stoppwörter auch nicht als Wortteil.
        // (Die vorhandene Ansage „Aus deinen Inhalten:“ des Stils verbatim enthält „halt“ nur als Teil von „Inhalten“; sie bleibt.)
        for text in AssistantNatural.announcementTexts {
            for stop in AssistantNatural.stoppWords {
                XCTAssertFalse(text.lowercased().contains(stop), "Stoppwort „\(stop)“ in „\(text)“ (auch als Teil eines Wortes)")
            }
        }
    }

    // MARK: Protokollfolge

    private func auditSession(_ model: any BookTalkModel, config c: AssistantConfig? = nil, history: [AssistantTurn] = [],
                              knowledge: AssistantKnowledge? = nil) async -> (AssistantSession, AssistantAuditLog) {
        let audit = AssistantAuditLog(directory: tempDir())
        return (await newSession(model, config: c, history: history, audit: audit, knowledge: knowledge), audit)
    }

    /// Zeilenfolge je Variante; der Prüfer lässt genau sie zu.
    func testJournalLineSequenceForEachVariant() async {
        // 1. Nur Sprecher
        let (s1, a1) = await auditSession(RoleModel(speaker: Script(pieces: ["Hallo zusammen."])))
        _ = await ask(s1, "Hallo")
        await close(s1, "r1", user: "Hallo")
        XCTAssertEqual(journalTags(journal(a1)), ["start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "end"])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a1)), [])

        // 2. Sprecher dann Daten, mit Überbrückung
        let (s2, a2) = await auditSession(RoleModel(speaker: search, thinker: answer), config: config(bridge: true))
        _ = await ask(s2)
        await close(s2)
        XCTAssertEqual(journalTags(journal(a2)), [
            "start:natural", "round:general/naturalTalk", "areas:", "round:data/naturalSearch", "step:announcement", "areas:books",
            "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a2)), [])

        // 3. Erzwungene Suche (direkter Inhaltsbezug), ohne Überbrückung: wie „Sprecher dann Daten“, der Sprecher ohne eigenen Satz
        let (s3, a3) = await auditSession(RoleModel(thinker: answer))
        _ = await ask(s3, ownQuestion)
        await close(s3, "r1", user: ownQuestion)
        XCTAssertEqual(journalTags(journal(a3)), [
            "start:natural", "round:general/naturalTalk", "areas:", "round:data/naturalSearch", "areas:books", "step:summarized",
            "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a3)), [])

        // 4. Bestandsfrage: Datenphase mit Grund `inventory`
        let (s4, a4) = await auditSession(RoleModel())
        _ = await ask(s4, inventoryQuestion)
        await close(s4, "r1", user: inventoryQuestion)
        let tags4 = journalTags(journal(a4))
        XCTAssertEqual(Array(tags4.prefix(3)), ["start:natural", "round:data/inventory", "areas:books"])
        XCTAssertEqual(tags4[3], "step:announcement")
        XCTAssertEqual(tags4.last, "end")
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a4)), [])

        // 5. Keine Treffer: feste Antwort, keine Bereichszeile der Auszüge
        let (s5, a5) = await auditSession(RoleModel(speaker: Script(pieces: ["SUCHE: Einkaufsliste Kühlschrank\n"])))
        _ = await ask(s5, "Wie viele Eier brauche ich?")
        await close(s5, "r1", user: "Wie viele Eier brauche ich?")
        XCTAssertEqual(journalTags(journal(a5)), [
            "start:natural", "round:general/naturalTalk", "areas:", "round:data/naturalSearch", "step:template", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a5)), [])
    }

    /// Die Rundenzeile der Daten-Phase trägt die zulässigen Abrufbereiche (der Stand dieser Anfrage), nicht eine eigene Auswahl.
    func testDataRoundLineCarriesThePermittedRetrievalAreas() async {
        let (s, audit) = await auditSession(RoleModel(speaker: search, thinker: answer), knowledge: await booksAndMeeting())
        _ = await s.setAreas([.books, .transcripts])
        _ = await ask(s)
        await close(s)
        let rounds = journal(audit).filter { $0.t == .round }
        XCTAssertEqual(rounds.count, 2)
        XCTAssertNil(rounds[0].areas, "die Sprecher-Rundenzeile trägt keine Abrufbereiche")
        XCTAssertEqual(rounds[1].areas, ["books", "transcripts"])
        let entries = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].style, .natural)
        XCTAssertEqual(entries[0].round, .data, "die letzte Rundenart")
        XCTAssertEqual(entries[0].contextReason, .naturalSearch)
        XCTAssertEqual(entries[0].areas, ["books"], "bedeckt sind nur die Bereiche der Quellen der Sätze")
    }

    /// Vor dem Sprecher stehen Rundenzeile und Bereichszeile dauerhaft in der Datei (das Modell liest die Datei beim Aufruf), vor dem
    /// Abruf die Rundenzeile der Daten-Phase mit den Abrufbereichen, vor dem Denker die Bereichszeile der verwendeten Auszüge.
    func testMandatoryLinesAreDurableBeforeSpeakerRetrievalAndThinker() async {
        let dir = tempDir()
        let audit = AssistantAuditLog(directory: dir)
        let url = audit.url
        let file: @Sendable () -> String = { (try? String(contentsOf: url, encoding: .utf8)) ?? "" }
        let atModel = Snaps()
        let atSearch = Snaps()
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", castorp), ("S. 13", stay)]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [ProbeSource { atSearch.add(file()) }])
        let model = RoleModel(speaker: search, thinker: answer, onCall: { role in atModel.add("\(role.rawValue)|" + file()) })
        let s = await newSession(model, config: config(bridge: true), audit: audit, knowledge: knowledge)
        _ = await ask(s)
        await close(s)

        let models = atModel.all
        XCTAssertEqual(models.count, 2)
        let speakerFile = models[0]
        XCTAssertTrue(speakerFile.hasPrefix("speaker|"))
        XCTAssertTrue(speakerFile.contains("\"reason\":\"naturalTalk\""), "Rundenzeile vor dem Sprecher")
        XCTAssertTrue(speakerFile.contains("\"t\":\"areas\""), "Bereichszeile des Verlaufs vor dem Sprecher")
        XCTAssertFalse(speakerFile.contains("naturalSearch"))
        let searches = atSearch.all
        XCTAssertEqual(searches.count, 1, "ein Abruf")
        XCTAssertTrue(searches[0].contains("\"reason\":\"naturalSearch\""), "Rundenzeile der Daten-Phase vor dem Abruf")
        XCTAssertTrue(searches[0].contains("\"areas\":[\"books\",\"files\",\"notes\",\"transcripts\"]"), "mit den zulässigen Abrufbereichen: \(searches[0])")
        let thinkerFile = models[1]
        XCTAssertTrue(thinkerFile.hasPrefix("thinker|"))
        XCTAssertTrue(thinkerFile.contains("\"areas\":[\"books\"]"), "Bereichszeile der verwendeten Auszüge vor dem Denker")
    }

    /// Bereichszeile vor dem Sprecher: die Bereiche der Quellen der Verlaufsbeiträge, die im Prompt stehen. Beiträge, die wegen der
    /// Grenze von sechs nicht mehr im Prompt stehen, zählen nicht.
    func testSpeakerAreasLineListsTheAreasOfTheHistoryTurnsInThePromptOnly() async {
        var history: [AssistantTurn] = [
            naturalTurn("Alt eins", "Antwort eins.", round: .data, sources: [assistantPassage(.notes, itemId: "n1", title: "Notiz", text: "Alt.")]),
            naturalTurn("Alt zwei", "Antwort zwei.", round: .data, sources: [assistantPassage(.files, itemId: "f1", title: "Datei", text: "Alt.")]),
        ]
        history += (0..<4).map { naturalTurn("Neu \($0)", "Antwort \($0).", round: .general) }
        history.append(naturalTurn("Neu vier", "Antwort vier.", round: .data, sources: [assistantPassage(.books, itemId: "b1", title: "Buch", text: castorp)]))
        history.append(naturalTurn("Neu fünf", "Antwort fünf.", round: .data,
                                   sources: [assistantPassage(.transcripts, itemId: "t1", title: "Besprechung", text: nordlicht)]))
        XCTAssertEqual(history.count, 8)
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]))
        let (s, audit) = await auditSession(model, history: history)
        _ = await ask(s, "Hallo")
        await close(s, "r1", user: "Hallo")

        let prompt = model.calls.prompts(.speaker)[0]
        XCTAssertTrue(prompt.contains("Neu fünf"))
        XCTAssertTrue(prompt.contains("Neu 0"))
        XCTAssertFalse(prompt.contains("Alt eins"), "nur die jüngsten sechs Beiträge")
        XCTAssertFalse(prompt.contains("Alt zwei"))
        let areas = journal(audit).filter { $0.t == .areas }
        XCTAssertEqual(areas.count, 1)
        XCTAssertEqual(areas[0].areas, ["books", "transcripts"], "Quellen der Beiträge im Prompt, nicht die der älteren")
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
    }

    /// Nennt der Sprecher dieselben Bereiche wie später die verwendeten Auszüge (Verlauf aus einem Buch, Abruf im Buch), steht die Bereichszeile
    /// der Daten-Phase trotzdem: Zu JEDER Phase gehört ihre eigene Zeile vor dem Modell (die Zeile des Sprechers ersetzt sie nicht).
    func testTheDataPhaseHasItsOwnAreasLineEvenWhenTheSpeakerNamedTheSameAreas() async {
        let history = [naturalTurn("Frühere Frage", "Antwort.", round: .data,
                                   sources: [assistantPassage(.books, itemId: "b1", title: "Der Zauberberg", text: castorp)])]
        let model = RoleModel(speaker: search, thinker: answer)
        let (s, audit) = await auditSession(model, config: config(bridge: true), history: history)
        let seen = await ask(s)
        await close(s)
        XCTAssertEqual(seen.statuses, [.announcement, .summarized, .summarized])
        XCTAssertEqual(journalTags(journal(audit)), [
            "start:natural", "round:general/naturalTalk", "areas:books", "round:data/naturalSearch", "step:announcement",
            "areas:books", "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
    }

    /// Cloud: Die Verlaufsbeiträge im Prompt und in der Bereichszeile sind schon nach den freigegebenen Bereichen gefiltert.
    func testCloudHistoryIsFilteredByTheAllowedAreasAndSoIsTheAreasLine() async {
        let history = [
            naturalTurn("Geheim Besprechung", "Antwort A.", round: .data,
                        sources: [assistantPassage(.transcripts, itemId: "t1", title: "Besprechung", text: nordlicht)]),
            naturalTurn("Offen Buch", "Antwort B.", round: .data, sources: [assistantPassage(.books, itemId: "b1", title: "Buch", text: castorp)]),
        ]
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]))
        let audit = AssistantAuditLog(directory: tempDir())
        let s = await newSession(model, config: config(cloud: true), history: history, audit: audit, knowledge: await booksAndMeeting())
        _ = await s.setAreas([.books], cloudConsent: true)
        _ = await ask(s, "Hallo")
        await close(s, "r1", user: "Hallo")
        let prompt = model.calls.prompts(.speaker)[0]
        XCTAssertFalse(prompt.contains("Geheim Besprechung"), "Gesprochenes aus einem gesperrten Bereich verlässt das Gerät nicht")
        XCTAssertTrue(prompt.contains("Offen Buch"))
        XCTAssertEqual(journal(audit).filter { $0.t == .areas }.first?.areas, ["books"])
    }

    func testCloudWithoutConsentRunsNeitherSpeakerNorThinker() async {
        let model = RoleModel(speaker: search, thinker: answer)
        let s = await newSession(model, config: config(cloud: true), cloudConsent: false)
        let seen = await ask(s)
        XCTAssertEqual(seen.failure, .consentRequired)
        XCTAssertTrue(seen.sentences.isEmpty)
        XCTAssertTrue(model.calls.roles.isEmpty)
    }

    /// Quellen eines `.summarized`-Satzes = genau die Auszüge im Prompt des Denkers dieser Anfrage (nicht die abgerufenen).
    func testSourcesOfSummarizedSentencesAreExactlyTheExcerptsInTheThinkerPrompt() async {
        // Zwei Treffer in verschiedenen Elementen, aber nur ein Auszug im Prompt (Grenze `naturalPassageLimit`).
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", castorp)]))
        await corpus.upsert(assistantDoc("b2", "Davos Reisebericht", sections: [("S. 3", "Davos liegt hoch in den Bergen und hat ein Sanatorium.")]))
        let model = RoleModel(speaker: search, thinker: answer)
        let (s, audit) = await auditSession(model, config: config(passageLimit: 1), knowledge: AssistantKnowledge(corpus: corpus))
        let seen = await ask(s)
        await close(s)
        let prompt = model.calls.prompts(.thinker)[0]
        let inPrompt = ["b1": castorp.prefix(40), "b2": "Davos liegt hoch".prefix(16)].filter { prompt.contains($0.value) }.map(\.key)
        XCTAssertEqual(inPrompt.count, 1, "genau ein Auszug im Prompt: \(prompt)")
        for sentence in seen.sentences where sentence.status == .summarized {
            XCTAssertEqual(sentence.sources.map(\.itemId), inPrompt, "Quellen = Auszüge im Prompt")
        }
        XCTAssertEqual(seen.answer?.sources.map(\.itemId), inPrompt)
        let turn = await s.history.last
        XCTAssertEqual(turn?.sources.map(\.itemId), inPrompt)
        XCTAssertEqual(seen.sourceEvents.last?.map(\.itemId), inPrompt, "auch die Vorab-Anzeige")
        let areas = journal(audit).filter { $0.t == .areas }.map(\.areas)
        XCTAssertEqual(areas.last ?? nil, ["books"])
    }

    /// Nach `contextOverflow` wiederholt der Denker mit halbem Auszugsbudget und ohne Verlauf; die Quellen sind die Auszüge der Wiederholung.
    func testSourcesAfterAnOverflowRetryAreTheExcerptsOfTheRetry() async {
        let corpus = AssistantCorpus()
        // Blöcke unter `maxBlockChars` (600), sonst werden sie nicht indexiert.
        let tail1 = String(repeating: "Hans Castorp fährt ins Sanatorium nach Davos. ", count: 10)
        let tail2 = String(repeating: "Davos liegt hoch in den Bergen und hat ein Sanatorium. ", count: 9)
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: (1...3).map { ("S. \($0)", "Hans Castorp reist, Teil \($0). " + tail1) }))
        await corpus.upsert(assistantDoc("b2", "Davos Reisebericht", sections: (1...3).map { ("S. \($0)", "Davos und Castorp, Teil \($0). " + tail2) }))
        let model = RoleModel(speaker: search, thinker: Script(pieces: ["Das passt jetzt."], overflows: 1))
        let history = [naturalTurn("Frühere Frage", "Frühere Antwort.", round: .general)]
        let s = await newSession(model, config: config(passageChars: 1_800), history: history, knowledge: AssistantKnowledge(corpus: corpus))
        let seen = await ask(s)
        let prompts = model.calls.prompts(.thinker)
        XCTAssertEqual(prompts.count, 2, "ein Aufruf, eine Wiederholung: \(seen.texts)")
        guard prompts.count == 2 else { return }
        XCTAssertTrue(prompts[0].contains("Frühere Frage"))
        XCTAssertFalse(prompts[1].contains("Frühere Frage"), "Wiederholung ohne Verlauf")
        XCTAssertLessThan(prompts[1].count, prompts[0].count)
        XCTAssertEqual(seen.spoken, ["Das passt jetzt."])
        let ids = ["b1": "Hans Castorp reist", "b2": "Davos und Castorp"].filter { prompts[1].contains($0.value) }.map(\.key).sorted()
        XCTAssertFalse(ids.isEmpty)
        let sourceIds = Set(seen.sentences.last?.sources.map(\.itemId) ?? [])
        XCTAssertEqual(sourceIds, Set(ids), "Quellen = Auszüge des zweiten Prompts")
        let firstIds = Set(["b1": "Hans Castorp reist", "b2": "Davos und Castorp"].filter { prompts[0].contains($0.value) }.map(\.key))
        XCTAssertTrue(sourceIds.isSubset(of: firstIds))
        func excerpts(_ prompt: String) -> Int { prompt.split(separator: "\n").filter { $0.hasPrefix("[") && $0.contains(" · ") }.count }
        XCTAssertLessThan(excerpts(prompts[1]), excerpts(prompts[0]), "die Wiederholung hat weniger Auszüge")
        XCTAssertEqual(seen.sentences.last?.sources.count, excerpts(prompts[1]), "genau die Auszüge der Wiederholung, nicht die des ersten Aufrufs")
        XCTAssertEqual(seen.answer?.sources.count, excerpts(prompts[1]))
    }

    // MARK: Verlauf

    func testTurnHasConversationalAndSummarizedPartsAndTheNaturalStyle() async {
        let s = await newSession(RoleModel(speaker: search, thinker: answer), config: config(bridge: true))
        _ = await ask(s)
        await close(s)
        let turn = await s.history.last
        XCTAssertEqual(turn?.style, .natural)
        XCTAssertEqual(turn?.round, .data)
        XCTAssertEqual(turn?.assistant, "Castorp fährt ins Sanatorium nach Davos. Dort besucht er seinen Vetter Joachim.")
        XCTAssertEqual(turn?.sources.map(\.itemId), ["b1"])
        XCTAssertEqual(turn?.parts.count, 1, "zwei Sätze gleicher Einordnung sind ein Abschnitt")
        XCTAssertEqual(turn?.parts.first?.note, AssistantTurnPart.summaryNote)
        XCTAssertEqual(turn?.parts.first?.isFreeform, true)
        XCTAssertEqual(turn?.parts.first?.quote, false)
        XCTAssertEqual(turn?.hasMarkedUnits, true)
        XCTAssertEqual(turn?.general, false)
        XCTAssertEqual(turn?.isUnsourced, false)

        let s2 = await newSession(RoleModel(speaker: Script(pieces: ["Hallo zusammen."])))
        _ = await ask(s2, "Hallo")
        await close(s2, "r1", user: "Hallo")
        let talk = await s2.history.last
        XCTAssertEqual(talk?.round, .general)
        XCTAssertEqual(talk?.style, .natural)
        XCTAssertEqual(talk?.parts.first?.note, AssistantTurnPart.conversationNote)
        XCTAssertTrue(talk?.sources.isEmpty ?? false)
        XCTAssertEqual(talk?.isUnsourced, false, "ein freies Gespräch im Stil natural ist keine „Antwort ohne Quelle“")
    }

    func testNaturalSpeakerTurnSurvivesAdmissionWhileAVerbatimGeneralTurnDoesNot() {
        let natural = naturalTurn("Hallo", "Hallo zusammen.", round: .general)
        var verbatim = AssistantTurn(user: "Hallo", assistant: "Hallo.", general: true,
                                     parts: [AssistantTurnPart(text: "Hallo.", unsourced: false, note: AssistantTurnPart.templateNote)],
                                     model: "Test", at: Date(), round: .general)
        verbatim.style = nil
        XCTAssertEqual(AssistantTurn.admitted([natural, verbatim], released: false).count, 1)
        XCTAssertEqual(AssistantTurn.admitted([natural, verbatim], released: false).first?.style, .natural)
    }

    func testSpeakerPromptSeesTurnsOfBothRoundKinds() async {
        let history = [naturalTurn("Allgemeine Frage", "Allgemeine Antwort.", round: .general),
                       naturalTurn("Frage zum Buch", "Antwort zum Buch.", round: .data,
                                   sources: [assistantPassage(.books, itemId: "b1", title: "Buch", text: castorp)])]
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]))
        _ = await ask(await newSession(model, history: history), "Hallo")
        let prompt = model.calls.prompts(.speaker)[0]
        XCTAssertTrue(prompt.contains("Allgemeine Frage"))
        XCTAssertTrue(prompt.contains("Frage zum Buch"))
    }

    // MARK: Fehlerfälle

    func testModelErrorsMapToTheFailureOfTheRun() async {
        let cases: [(BookTalkModelError, AssistantFailure)] = [(.offline, .offline), (.unavailable, .noModel), (.timeout, .timeout), (.notAllowed, .consentRequired)]
        for (error, failure) in cases {
            let seen = await ask(await newSession(RoleModel(speaker: Script(fail: error))))
            XCTAssertEqual(seen.failure, failure, "Sprecher: \(error)")
            let seen2 = await ask(await newSession(RoleModel(speaker: search, thinker: Script(fail: error))), castorpQuestion, id: "r2")
            XCTAssertEqual(seen2.failure, failure, "Denker: \(error)")
        }
    }

    /// Absturz des Modells (Kindprozess) mitten im Strom des Denkers: Der erste Satz ging hinaus, danach Fehler, kein `done`, kein Hängen.
    func testModelCrashInTheMiddleOfTheThinkerStreamEndsWithAFailureAndNoDone() async {
        let thinker = Script(pieces: ["Castorp fährt nach Davos. ", "Dort bleibt er"], fail: Crash())
        let seen = await ask(await newSession(RoleModel(speaker: search, thinker: thinker)))
        XCTAssertEqual(seen.spoken, ["Castorp fährt nach Davos."])
        XCTAssertEqual(seen.failure, .modelFailed)
        XCTAssertNil(seen.answer)
    }

    func testContextOverflowIsRetriedOnceWithoutHistoryAndOnlyOnce() async {
        let history = [naturalTurn("Frühere Frage", "Frühere Antwort.", round: .general)]
        let once = RoleModel(speaker: Script(pieces: ["Hallo zusammen."], overflows: 1))
        let seen = await ask(await newSession(once, history: history), "Hallo")
        XCTAssertEqual(seen.spoken, ["Hallo zusammen."])
        let prompts = once.calls.prompts(.speaker)
        XCTAssertEqual(prompts.count, 2)
        XCTAssertTrue(prompts[0].contains("Frühere Frage"))
        XCTAssertFalse(prompts[1].contains("Frühere Frage"))

        let twice = RoleModel(speaker: Script(pieces: ["Hallo zusammen."], overflows: 2))
        let seen2 = await ask(await newSession(twice), "Hallo")
        XCTAssertEqual(seen2.failure, .modelFailed)
        XCTAssertEqual(twice.calls.count(.speaker), 2, "genau eine Wiederholung")
    }

    func testCancelWhileTheSpeakerRunsStopsEverythingAndRunsNoThinker() async {
        let model = RoleModel(speaker: Script(hang: true), thinker: answer)
        let s = await newSession(model)
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
            events.append(e)
            // Haltepunkt: erst abbrechen, wenn das Double den Sprecher-Aufruf angenommen hat (nicht schon bei der Vorab-Anzeige).
            if case .sources = e {
                let accepted = await model.calls.accepted(.speaker)
                XCTAssertTrue(accepted, "der Sprecher-Aufruf kam nie am Modell an")
                await s.cancel(requestId: "r1")
            }
        }
        XCTAssertEqual(Seen(events: events).failure, .cancelled)
        XCTAssertEqual(model.calls.roles, [.speaker])
        let ended = await model.calls.ended(1)
        XCTAssertTrue(ended)
        XCTAssertEqual(model.calls.terminations, 1, "der Modellstrom des Sprechers wurde beendet")
    }

    func testCancelWhileTheThinkerRunsEndsWithCancelled() async {
        let model = RoleModel(speaker: search, thinker: Script(hang: true))
        let s = await newSession(model)
        var events: [AssistantEvent] = []
        var sourceEvents = 0
        for await e in s.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
            events.append(e)
            if case .sources = e {
                sourceEvents += 1
                if sourceEvents == 2 {
                    // Haltepunkt: Der Denker-Aufruf muss am Double angekommen sein, sonst gäbe es keinen zweiten Strom zu beenden.
                    let accepted = await model.calls.accepted(.thinker)
                    XCTAssertTrue(accepted, "der Denker-Aufruf kam nie am Modell an")
                    await s.cancel(requestId: "r1")
                }
            }
        }
        XCTAssertEqual(Seen(events: events).failure, .cancelled)
        let ended = await model.calls.ended(2)
        XCTAssertTrue(ended)
        XCTAssertEqual(model.calls.terminations, 2, "beide Modellströme sind beendet")
    }

    func testHangingSpeakerEndsWithTimeout() async {
        let seen = await ask(await newSession(RoleModel(speaker: Script(hang: true)), config: config(timeout: 0.3)))
        XCTAssertEqual(seen.failure, .timeout)
    }

    func testRevocationWhileTheThinkerRunsEndsWithRevokedAndOutputsNothingMore() async {
        let model = RoleModel(speaker: search, thinker: Script(pieces: ["Castorp fährt nach Davos. Dort "], hang: true))
        let s = await newSession(model)
        var events: [AssistantEvent] = []
        var revoked = false
        for await e in s.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
            events.append(e)
            if !revoked, case .sentence = e {
                revoked = true
                _ = s.applyReleaseChange(.other)
            }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertEqual(seen.sentences.count, 1)
    }

    /// Widerruf zwischen den Phasen: Die zweite Rundenzeile steht, aber kein Abruf und kein Denker; die Folge ist für den Prüfer in Ordnung.
    func testRevocationBetweenThePhasesRunsNoRetrievalAndNoThinker() async {
        let index = AssistantStepIndex()
        let box = Box<AssistantSession?>(nil)
        let visits = Box(0)
        index.setProbe { _, point in
            guard point == .beforeRound else { return }
            visits.value += 1
            if visits.value == 2 { _ = box.value?.applyReleaseChange(.other) }
        }
        let audit = AssistantAuditLog(directory: tempDir())
        let retrievals = Box(0)
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", castorp)]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [ProbeSource { retrievals.value += 1 }])
        let model = RoleModel(speaker: search, thinker: answer)
        let s = await newSession(model, audit: audit, knowledge: knowledge, stepIndex: index)
        box.value = s
        let seen = await ask(s)
        await close(s)
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertEqual(model.calls.roles, [.speaker], "kein Denker")
        XCTAssertEqual(retrievals.value, 0, "kein Abruf")
        XCTAssertTrue(seen.sentences.isEmpty)
        XCTAssertEqual(seen.progress, [.speaker], "nach dem Widerruf kein Fortschritt (weder searching noch found noch composing)")
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
        let revokedEntries = await audit.entries()
        XCTAssertEqual(revokedEntries.first { $0.kind == .answer }?.outcome, .revoked)
    }

    /// Widerruf während des Abrufs: Nach `searching` kommt kein Fortschritt mehr (weder `found` noch `composing` noch ein Entwurf), kein Denker.
    func testRevocationDuringTheRetrievalSendsNoFurtherProgress() async {
        let box = Box<AssistantSession?>(nil)
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", castorp)]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [ProbeSource { _ = box.value?.applyReleaseChange(.other) }])
        let model = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\n"]), thinker: answer)
        let s = await newSession(model, knowledge: knowledge)
        box.value = s
        let seen = await ask(s)
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertEqual(seen.progress, [.speaker, .searching(query: "Davos Sanatorium Castorp")])
        XCTAssertEqual(model.calls.roles, [.speaker], "kein Denker")
        XCTAssertEqual(seen.statuses, [.conversational], "der Zwischensatz ging vor dem Widerruf hinaus, danach nichts mehr")
    }

    // MARK: Schreibfehler (voller Datenträger)

    private func failingRun(_ needle: String, model: RoleModel, knowledge: AssistantKnowledge? = nil, text: String = castorpQuestion,
                            switchOn: Bool = true) async -> Seen {
        let sw = FailSwitch()
        sw.on = switchOn
        let audit = AssistantAuditLog(directory: tempDir(), reader: AssistantAuditWriter.readFile, appender: failingAppender(needle, sw))
        let s = await newSession(model, audit: audit, knowledge: knowledge)
        return await ask(s, text)
    }

    /// Misslingt das Schreiben einer Pflichtzeile (`round`, `areas`, `start`), läuft kein Modell und geht nichts hinaus; Ausgang „Fehler“.
    func testFailingMandatoryLineMeansNoModelAndNoOutput() async {
        for needle in ["\"t\":\"round\"", "\"t\":\"areas\"", "\"t\":\"start\""] {
            let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]))
            let seen = await failingRun(needle, model: model)
            XCTAssertEqual(seen.failure, .auditUnavailable, needle)
            XCTAssertTrue(seen.sentences.isEmpty, needle)
            XCTAssertTrue(model.calls.roles.isEmpty, "\(needle): kein Modell")
        }
    }

    func testFailingStepLineStopsTheOutput() async {
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen. Wie ", "geht es dir?"]))
        let seen = await failingRun("\"t\":\"step\"", model: model)
        XCTAssertEqual(seen.failure, .auditUnavailable)
        XCTAssertTrue(seen.sentences.isEmpty, "die Einordnung steht nicht in der Datei: die Einheit geht nicht hinaus")
        XCTAssertNil(seen.answer)
    }

    /// Scheitert die Rundenzeile der Daten-Phase, läuft weder der Abruf noch der Denker.
    func testNoRetrievalWhenTheDataRoundLineFails() async {
        let retrievals = Box(0)
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", castorp)]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [ProbeSource { retrievals.value += 1 }])
        let model = RoleModel(speaker: search, thinker: answer)
        let seen = await failingRun("naturalSearch", model: model, knowledge: knowledge)
        XCTAssertEqual(seen.failure, .auditUnavailable)
        XCTAssertEqual(retrievals.value, 0)
        XCTAssertEqual(model.calls.roles, [.speaker], "kein Denker")
    }

    /// Scheitert die Bereichszeile der verwendeten Auszüge (die Zeile des Sprechers steht), läuft der Denker nicht.
    func testNoThinkerWhenTheAreasLineOfTheUsedExcerptsFails() async {
        let appender: @Sendable (URL, Data) throws -> Void = { url, data in
            let text = String(decoding: data, as: UTF8.self)
            if text.contains("\"t\":\"areas\""), text.contains("\"books\"") { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        }
        let audit = AssistantAuditLog(directory: tempDir(), reader: AssistantAuditWriter.readFile, appender: appender)
        let model = RoleModel(speaker: search, thinker: answer)
        let s = await newSession(model, audit: audit)
        let seen = await ask(s)
        XCTAssertEqual(seen.failure, .auditUnavailable)
        XCTAssertEqual(model.calls.roles, [.speaker])
        XCTAssertTrue(seen.sentences.isEmpty)
    }

    // MARK: Abbruch mitten im Vorgang, Altdaten

    /// Jedes Präfix der Zeilenfolge (Prozessende an jeder Stelle) ist für den Prüfer in Ordnung; nichts wird fortgesetzt: Die Datei einer
    /// abgebrochenen Anfrage ohne `end` faltet zu einem unvollständigen Eintrag mit Stil `natural`.
    func testEveryPrefixOfAJournalIsValidAndAnUnfinishedRequestFoldsAsIncomplete() async {
        let (s, audit) = await auditSession(RoleModel(speaker: search, thinker: answer), config: config(bridge: true))
        _ = await ask(s)
        await close(s)
        let lines = journal(audit)
        XCTAssertGreaterThan(lines.count, 8)
        for k in 1...lines.count {
            XCTAssertEqual(AssistantProtocolOrder.check(Array(lines.prefix(k))), [], "Präfix der Länge \(k)")
        }
        let cut = lines.prefix { $0.t != .end }
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let datas = cut.compactMap { try? encoder.encode($0) }
        let folded = AssistantJournalFold.fold(datas)
        XCTAssertEqual(folded.entries.count, 1)
        XCTAssertEqual(folded.entries[0].style, .natural)
        XCTAssertEqual(folded.entries[0].outcome, .incomplete, "ohne `end` bleibt der Ausgang offen (unvollständig), nichts wird fortgesetzt")
    }

    /// Altdaten: Zeilen und Einträge ohne die neuen Felder laden; im Stil `verbatim` bleibt die Startzeile Byte für Byte ohne `style`.
    func testOldDataLoadsAndVerbatimJournalBytesAreUnchanged() async throws {
        let oldStart = #"{"t":"start","req":"alt1","at":"2026-11-01T10:00:00Z","session":"s","model":"other","cloud":false,"unsourced":false,"consent":1,"ctx":"data","reason":"locked","round":"data","gen":1}"#
        let line = try XCTUnwrap(AssistantJournalFold.journal(Data(oldStart.utf8)))
        XCTAssertNil(line.style)
        let folded = AssistantJournalFold.fold([Data(oldStart.utf8)])
        XCTAssertEqual(folded.entries.first?.style, .verbatim)

        let entry = AssistantAuditEntry(at: Date(), kind: .answer, cloud: false, unsourcedMode: false, consentVersion: 1)
        let encoded = try JSONEncoder().encode(entry)
        XCTAssertFalse(String(decoding: encoded, as: UTF8.self).contains("style"), "ein Eintrag im Stil verbatim schreibt kein `style`")
        XCTAssertEqual(try JSONDecoder().decode(AssistantAuditEntry.self, from: encoded).style, .verbatim)

        let oldTurn = #"{"user":"a","assistant":"b","interrupted":false,"sources":[],"general":false,"model":"m","at":0}"#
        let turn = try JSONDecoder().decode(AssistantTurn.self, from: Data(oldTurn.utf8))
        XCTAssertNil(turn.style)
        XCTAssertFalse(String(decoding: try JSONEncoder().encode(turn), as: UTF8.self).contains("\"style\""))
        var natural = turn
        natural.style = .natural
        let back = try JSONDecoder().decode(AssistantTurn.self, from: try JSONEncoder().encode(natural))
        XCTAssertEqual(back.style, .natural)

        let oldCounts = #"{"grounded":1,"inventory":0,"unsourced":0,"neutral":0,"droppedOwnData":0,"droppedNoMode":0,"droppedNotSnapped":0,"droppedDuplicate":0,"droppedTemplate":0,"revoked":0,"droppedFragment":0}"#
        let counts = try JSONDecoder().decode(AssistantSentenceCounts.self, from: Data(oldCounts.utf8))
        XCTAssertEqual(counts, AssistantSentenceCounts(grounded: 1))
        let zero = String(decoding: try JSONEncoder().encode(counts), as: UTF8.self)
        XCTAssertFalse(zero.contains("conversational") || zero.contains("summarized"), "neue Zähler nur, wenn sie > 0 sind")
        let some = String(decoding: try JSONEncoder().encode(AssistantSentenceCounts(conversational: 1, summarized: 2)), as: UTF8.self)
        XCTAssertTrue(some.contains("\"conversational\":1") && some.contains("\"summarized\":2"))

        // Eine Datei mit einer Anfrage im Stil verbatim und einer im Stil natural faltet zu zwei Einträgen mit dem richtigen Stil.
        let audit = AssistantAuditLog(directory: tempDir())
        let verbatimConfig = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        let v = AssistantSession(model: RoleModel(thinker: Script(pieces: ["Hans Castorp fährt nach Davos und besucht seinen Vetter Joachim."])),
                                 knowledge: await books(), config: verbatimConfig, audit: audit, released: true)
        _ = await ask(v, "Wohin fährt Hans Castorp im Sanatorium nach Davos?", id: "v1")
        await v.commit(requestId: "v1", user: "x", spoken: .all, interrupted: false)
        let n = await newSession(RoleModel(speaker: Script(pieces: ["Hallo zusammen."])), audit: audit)
        _ = await ask(n, "Hallo", id: "n1")
        await n.commit(requestId: "n1", user: "Hallo", spoken: .all, interrupted: false)
        let raw = rawJournal(audit)
        let startLines = raw.split(separator: "\n").filter { $0.contains("\"t\":\"start\"") }
        XCTAssertEqual(startLines.count, 2)
        XCTAssertFalse(startLines[0].contains("\"style\""), "verbatim: kein Schlüssel `style`")
        XCTAssertTrue(startLines[1].contains("\"style\":\"natural\""))
        let answers = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(answers.map(\.style), [.verbatim, .natural])
        XCTAssertEqual(answers.map(\.outcome), [.completed, .completed])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit, "v1")), [], "verbatim: die alte Folge")
    }

    func testTheDefaultStyleIsVerbatimAndAVerbatimRunIgnoresTheNaturalPath() async {
        XCTAssertEqual(AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5).style, .verbatim)
        // Im Stil verbatim kennt die Anweisung die Suchzeile nicht: Das Modell antwortet in der Rolle „thinker“ dieses Doubles.
        let model = RoleModel(thinker: Script(pieces: ["Das Wetter ist schön."]))
        var verbatim = config()
        verbatim.style = .verbatim
        let seen = await ask(await newSession(model, config: verbatim), weather)
        XCTAssertFalse(seen.statuses.contains(.conversational))
        XCTAssertFalse(seen.statuses.contains(.summarized))
        XCTAssertFalse(seen.texts.contains("Das Wetter ist schön."), "verbatim: ein freier Modellsatz ohne Quelle wird nie ausgegeben: \(seen.texts)")
        XCTAssertTrue(seen.statuses.allSatisfy { $0 == .neutral }, "verbatim ohne Opt-in: nur der feste Satz: \(seen.statuses)")
    }

    // MARK: Zwischensatz vor der Steuerzeile

    /// Ein kurzer Satz vor `SUCHE:` geht sofort hinaus (`.conversational`, ohne Quellen), die Steuerzeile nie; danach die Daten-Phase. Der
    /// Zwischensatz steht im Antworttext vor der zusammengefassten Antwort und im Verlauf als eigener Abschnitt „Frei formuliert“.
    func testInterimSentenceIsSpokenBeforeTheControlLineAndTheControlLineNeverIs() async {
        let model = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\n"]), thinker: answer)
        let s = await newSession(model)
        let seen = await ask(s)
        await close(s)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.texts.first, "Ich sehe in deinem Buch nach.")
        XCTAssertTrue(seen.sentences[0].sources.isEmpty)
        XCTAssertFalse(seen.sentences[1].sources.isEmpty)
        XCTAssertFalse(seen.texts.joined().uppercased().contains("SUCHE"), "die Steuerzeile wird nie ausgegeben: \(seen.texts)")
        XCTAssertEqual(model.calls.roles, [.speaker, .thinker])
        XCTAssertEqual(seen.answer?.text, "Ich sehe in deinem Buch nach. Castorp fährt ins Sanatorium nach Davos. Dort besucht er seinen Vetter Joachim.")
        XCTAssertEqual(seen.answer?.counts.conversational, 1)
        XCTAssertEqual(seen.answer?.counts.summarized, 2)
        XCTAssertEqual(seen.answer?.round, .data)
        let turn = await s.history.last
        XCTAssertEqual(turn?.parts.map(\.note), [AssistantTurnPart.conversationNote, AssistantTurnPart.summaryNote])
        XCTAssertEqual(turn?.hasMarkedUnits, true)
        XCTAssertEqual(turn?.sources.map(\.itemId), ["b1"])
    }

    /// „Sofort“: Der Zwischensatz geht hinaus, sobald die Steuerzeile erkannt ist, nicht erst am Zeilenende oder am Ende des Sprecher-Stroms.
    func testInterimSentenceIsOutputAsSoonAsTheControlLineIsRecognized() async {
        // Der Rest der Steuerzeile (Stück 1) wird zurückgehalten, bis der Zwischensatz bei der Anzeige ankam: keine Zeitmessung nötig.
        // Käme der Satz erst mit dem Rest der Zeile, bliebe der Strom hängen und die Anfrage endete mit `timeout`.
        let release = Latch()
        let model = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE:", " Davos Sanatorium Castorp\n"])
                                .holding(1, until: release),
                              thinker: answer)
        let s = await newSession(model, config: config(timeout: 30))
        var sentenceBeforeTheRest = false
        var out: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
            if case .sentence = e, !release.isOpen {
                sentenceBeforeTheRest = true
                release.open()
            }
            out.append(e)
        }
        XCTAssertTrue(sentenceBeforeTheRest, "der Zwischensatz kam vor dem Rest der Steuerzeile")
        XCTAssertEqual(Seen(events: out).statuses, [.conversational, .summarized, .summarized])
    }

    /// Stückgrenzen, fehlendes Satzzeichen vor dem Zeilenumbruch, mehrere Sätze vor der Steuerzeile.
    func testInterimSentenceVariants() async {
        // Über Stückgrenzen, Steuerzeile klein geschrieben.
        let m1 = RoleModel(speaker: Script(pieces: ["Ich sehe ", "in deinem Buch nach.\nSU", "CHE: Davos\n"]), thinker: answer)
        let seen1 = await ask(await newSession(m1), castorpQuestion, id: "a")
        XCTAssertEqual(seen1.texts.first, "Ich sehe in deinem Buch nach.")
        XCTAssertEqual(seen1.statuses, [.conversational, .summarized, .summarized])

        // Ohne Satzzeichen vor dem Zeilenumbruch: der offene Satz gilt als vollständig.
        let m2 = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach\nSUCHE: Davos\n"]), thinker: answer)
        let seen2 = await ask(await newSession(m2), castorpQuestion, id: "b")
        XCTAssertEqual(seen2.texts.first, "Ich sehe in deinem Buch nach.")
        XCTAssertEqual(seen2.statuses, [.conversational, .summarized, .summarized])

        // Höchstens ein Satz vor der Steuerzeile: der zweite (noch im Satztrenner) wird verworfen.
        let m3 = RoleModel(speaker: Script(pieces: ["Das steht sicher in deinem Buch. Zweiter Satz.\nSUCHE: Davos\n"]), thinker: answer)
        let seen3 = await ask(await newSession(m3), castorpQuestion, id: "c")
        XCTAssertEqual(seen3.spoken.prefix(1), ["Das steht sicher in deinem Buch."])
        XCTAssertFalse(seen3.texts.contains("Zweiter Satz."))
        XCTAssertEqual(seen3.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen3.answer?.counts.droppedTemplate, 1, "der zweite Satz ist verworfen")
        XCTAssertEqual(m3.calls.roles, [.speaker, .thinker], "die Suche läuft trotzdem")

        // Ein Satz des Systems (hier die Ansage) ist als Zwischensatz verworfen; die Suche läuft.
        let m4 = RoleModel(speaker: Script(pieces: ["\(AssistantNatural.fillerFirstSentence)\nSUCHE: Davos\n"]), thinker: answer)
        let seen4 = await ask(await newSession(m4), castorpQuestion, id: "d")
        XCTAssertEqual(seen4.statuses, [.summarized, .summarized])
        XCTAssertEqual(seen4.answer?.counts.droppedTemplate, 1)

        // Steht „suche:“ klein geschrieben in derselben Zeile hinter dem Satz, ist es keine Steuerzeile: keine Suche, der Text bleibt Text
        // (eine direkte Antwort).
        let m5 = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach. suche: Davos.\n"]), thinker: answer)
        let seen5 = await ask(await newSession(m5), castorpQuestion, id: "e")
        XCTAssertEqual(m5.calls.roles, [.speaker])
        XCTAssertEqual(seen5.statuses, [.conversational])
    }

    // MARK: Steuerzeile in derselben Zeile hinter einem Satzende

    /// „SUCHE:“ in Großbuchstaben mitten in der Zeile, NACH einem Satzende (. ! ? …) und Leerraum, ist eine Steuerzeile: der Satz davor ist
    /// der Zwischensatz, der Rest sind die Suchwörter. Auch über Stückgrenzen (auch mitten im Wort „SUCHE“) und am Stromende ohne Zeilenumbruch.
    func testControlLineInTheSameLineAfterASentenceEndIsRecognized() async {
        let variants: [[String]] = [
            ["Ich sehe in deinem Buch nach. SUCHE: Davos Sanatorium Castorp\n"],
            ["Ich sehe in deinem Buch nach! SUCHE: Davos Sanatorium Castorp\n"],
            ["Ich sehe in deinem Buch nach? SUCHE: Davos Sanatorium Castorp\n"],
            ["Ich sehe in deinem Buch nach\u{2026} SUCHE: Davos Sanatorium Castorp\n"],
            ["Ich sehe in deinem Buch nach.  \tSUCHE : Davos Sanatorium Castorp\n"],
            ["Ich sehe in deinem Buch nach. SUCHE: Davos Sanatorium Castorp"],
            ["Ich sehe in deinem Buch nach. S", "U", "CHE", ":", " Davos Sanatorium", " Castorp\n"],
            ["Ich sehe in deinem Buch nach.", " ", "SUCHE: Davos Sanatorium Castorp\n"],
            ["Ich sehe ", "in deinem Buch nach", ". SU", "CHE: Davos Sanatorium Castorp\n"],
        ]
        for (i, pieces) in variants.enumerated() {
            let model = RoleModel(speaker: Script(pieces: pieces), thinker: answer)
            let seen = await ask(await newSession(model), castorpQuestion, id: "x\(i)")
            XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized], "Variante \(i): \(pieces) -> \(seen.statuses)")
            XCTAssertEqual(model.calls.roles, [.speaker, .thinker], "Variante \(i)")
            XCTAssertFalse(seen.texts.joined().uppercased().contains("SUCHE"), "Variante \(i): die Steuerzeile wird nie ausgegeben: \(seen.texts)")
            XCTAssertTrue(seen.texts.first?.hasPrefix("Ich sehe in deinem Buch nach") == true, "Variante \(i): \(seen.texts)")
            XCTAssertEqual(seen.progress.contains(.searching(query: "Davos Sanatorium Castorp")), true, "Variante \(i): die Suchwörter")
        }
    }

    /// Keine Steuerzeile: klein geschriebenes „suche:“ hinter dem Satzende, „SUCHE:“ ohne Leerraum hinter dem Satzende, „SUCHE:“ mitten im
    /// Satz ohne Satzende davor, „Suche:“ in gemischter Schreibweise hinter dem Satzende. Die Anfrage geht ohne Suche zu Ende, die
    /// Protokollzeile wird nie gesprochen (soweit sie am Satzanfang steht).
    func testControlLineInTheSameLineNeedsASentenceEndWhitespaceAndCapitals() async {
        let texts: [[String]] = [
            ["Das sehe ich in deinem Buch. suche: Davos Sanatorium."],
            ["Das sehe ich in deinem Buch. Suche: Davos Sanatorium."],
            ["Das sehe ich in deinem Buch.SUCHE: Davos Sanatorium."],
            ["Das sehe ich in deinem Buch, SUCHE: Davos Sanatorium."],
            ["Ich suche: Davos Sanatorium und finde es."],
            ["Das sehe ich in deinem Buch. ", "suche: Davos ", "Sanatorium."],
            ["Das sehe ich in deinem Buch. Su", "che: Davos Sanatorium."],
        ]
        for (i, pieces) in texts.enumerated() {
            let model = RoleModel(speaker: Script(pieces: pieces), thinker: answer)
            let seen = await ask(await newSession(model), castorpQuestion, id: "n\(i)")
            XCTAssertEqual(model.calls.roles, [.speaker], "Variante \(i): \(pieces): keine Suche")
            XCTAssertFalse(seen.progress.contains { if case .searching = $0 { true } else { false } }, "Variante \(i)")
            XCTAssertFalse(seen.statuses.contains(.summarized), "Variante \(i)")
            XCTAssertTrue(seen.texts.allSatisfy { !AssistantNatural.isProtocolLine($0) }, "Variante \(i): \(seen.texts)")
        }
    }

    /// Der Satz vor der Steuerzeile in derselben Zeile durchläuft denselben Filter wie der Zwischensatz in der Zeile davor.
    func testInterimSentenceBeforeAControlLineInTheSameLineIsFilteredToo() async {
        let model = RoleModel(speaker: Script(pieces: ["Ich schaue kurz nach. SUCHE: Davos Sanatorium Castorp\n"]), thinker: answer)
        let seen = await ask(await newSession(model))
        XCTAssertEqual(seen.statuses, [.summarized, .summarized], "der Floskelsatz ist verworfen, die Suche läuft")
        XCTAssertEqual(seen.answer?.counts.droppedTemplate, 1)
        XCTAssertEqual(model.calls.roles, [.speaker, .thinker])
        XCTAssertTrue(seen.announcements.isEmpty)
    }

    // MARK: Filter für den Zwischensatz

    /// Je Regel ein Satz, der verworfen wird, und einer, der bleibt (Wörter wie `AssistantWords.keys`, Groß-/Kleinschreibung egal).
    func testFillerInterimFilterDropsAndAllowsPerRule() {
        let f = AssistantNatural.isFillerInterim
        // Stoppwörter als Wort
        for text in ["Stopp, ich sehe in deinem Buch nach.", "Stop das steht in deinem Buch.", "Das steht, halt, in deinem Buch.",
                     "Warte, ich sehe in deinem Buch nach.", "MOMENT, ich sehe in deinem Buch nach."] {
            XCTAssertTrue(f(text), text)
        }
        XCTAssertFalse(f("Ich halte das in deinem Buch fest."), "„halte“ ist nicht „halt“")
        XCTAssertFalse(f("Das steht im Haltungskapitel deines Buches."), "kein Wortvergleich auf Teilstrings")
        // weniger als 4 Wörter
        for text in ["Ich sehe nach.", "Okay.", "Gleich, Moment.", "Dazu gleich mehr."] { XCTAssertTrue(f(text), text) }
        XCTAssertFalse(f("Das steht in deinem Buch."))
        XCTAssertFalse(f("Davos liegt in der Schweiz."))
        // „denke … nach“ mit höchstens zwei Wörtern dazwischen
        for text in ["Ich denke kurz darüber nach.", "Ich denke darüber nach, Patrick.", "Denke ich nach, Patrick?", "Ich denke nach, Patrick, und finde es."] {
            XCTAssertTrue(f(text), text)
        }
        XCTAssertFalse(f("Ich denke, das steht in deinem Buch."))
        XCTAssertFalse(f("Ich denke, dass Davos in der Schweiz liegt und bald nach Hause kommt."), "„nach“ weiter als zwei Wörter hinter „denke“")
        XCTAssertFalse(f("Ich denke einmal ganz kurz nach."), "drei Wörter zwischen „denke“ und „nach“: außerhalb des Musters")
        // überlege / überlegen
        XCTAssertTrue(f("Ich überlege gerade, wo das steht."))
        XCTAssertTrue(f("Da muss ich erst überlegen, Patrick."))
        XCTAssertTrue(f("Ich ueberlege kurz, ob das hier passt."), "die Schreibweise ohne Umlaut zählt auch")
        XCTAssertFalse(f("Das steht wohl in deinem Buch."))
        // lass mich
        XCTAssertTrue(f("Lass mich kurz in deinem Buch blättern."))
        XCTAssertFalse(f("Das Buch lässt dich nicht los, sagt Patrick."))
        // augenblick, sekunde
        XCTAssertTrue(f("Einen Augenblick, das steht in deinem Buch."))
        XCTAssertTrue(f("Eine Sekunde, das steht in deinem Buch."))
        XCTAssertFalse(f("Das steht im Kapitel Augenblicke der Stille."), "„augenblicke“ ist ein anderes Wort")
        // gleich wieder, bin gleich
        XCTAssertTrue(f("Ich bin gleich mit dem Ergebnis bei dir."))
        XCTAssertTrue(f("Ich melde mich gleich wieder bei dir."))
        XCTAssertFalse(f("Das Buch ist gleich im Regal neben dir."), "„gleich“ allein reicht nicht")
        XCTAssertFalse(f("Ich bin im Buch gleich auf Seite zwölf."), "„bin“ und „gleich“ nicht benachbart")
        // Suchverb + „nach“ ohne Inhaltswort
        for text in ["Ich schaue kurz nach.", "Ich schaue mal kurz nach.", "Ich schau mal nach, okay.", "Ich sehe gleich kurz mal nach.",
                     "Ich gucke einmal schnell nach.", "Ich suche kurz mal nach, ja.", "Ich such das jetzt nach.", "Dann schaue ich kurz nach."] {
            XCTAssertTrue(f(text), text)
        }
        XCTAssertFalse(f("Ich schaue in deinem Humboldt-Buch nach."), "mit Inhalt erlaubt")
        XCTAssertFalse(f("Ich schaue kurz in deinem Humboldt-Buch nach."))
        XCTAssertFalse(f("Ich sehe bei Humboldt nach."))
        XCTAssertFalse(f("Ich schaue im Kapitel zwei nach."))
        XCTAssertFalse(f("Ich schaue nach, ob Davos dort steht."), "mit Inhalt hinter „nach“ erlaubt")
        XCTAssertFalse(f("Ich suche das Kapitel über Davos."), "kein „nach“: kein Muster")
        // Schreibweise egal
        XCTAssertTrue(f("ICH SCHAUE KURZ NACH."))
        XCTAssertFalse(f("ICH SCHAUE IN DEINEM BUCH NACH."))
    }

    /// Im Ablauf: Ein verworfener Zwischensatz (je Regel einer) gibt nichts aus, zählt als `droppedTemplate`, die Suche läuft trotzdem,
    /// es gibt weder Ersatzsatz noch Ansage. Ein erlaubter Zwischensatz geht wie bisher hinaus.
    func testFilteredInterimSentenceIsDroppedAndTheSearchRunsWithoutAReplacement() async {
        let dropped = [
            "Moment, ich sehe in deinem Buch nach.", "Ich sehe nach.", "Ich denke kurz darüber nach.", "Ich überlege gerade, wo das steht.",
            "Lass mich kurz in deinem Buch blättern.", "Einen Augenblick, das steht in deinem Buch.", "Eine Sekunde, das steht in deinem Buch.",
            "Ich bin gleich mit dem Ergebnis bei dir.", "Ich melde mich gleich wieder bei dir.", "Ich schaue kurz nach.",
        ]
        for (i, text) in dropped.enumerated() {
            for style in 0..<2 {
                // Steuerzeile in der nächsten Zeile (style 0) und in derselben Zeile hinter dem Satzende (style 1)
                let line = style == 0 ? "\(text)\nSUCHE: Davos Sanatorium Castorp\n" : "\(text) SUCHE: Davos Sanatorium Castorp\n"
                let model = RoleModel(speaker: Script(pieces: [line]), thinker: answer)
                let seen = await ask(await newSession(model), castorpQuestion, id: "f\(i)-\(style)")
                XCTAssertEqual(seen.statuses, [.summarized, .summarized], "verworfen: \(text) -> \(seen.statuses)")
                XCTAssertEqual(seen.answer?.counts.droppedTemplate, 1, text)
                XCTAssertEqual(model.calls.roles, [.speaker, .thinker], "die Suche läuft: \(text)")
                XCTAssertTrue(seen.announcements.isEmpty, "keine Ansage: \(text)")
                XCTAssertFalse(seen.texts.contains { $0.contains(text) }, "nicht ausgegeben: \(text)")
                XCTAssertFalse(seen.texts.joined().uppercased().contains("SUCHE"), text)
            }
        }
        let allowed = [
            "Ich schaue in deinem Humboldt-Buch nach.", "Ich denke, das steht in deinem Buch.", "Das steht sicher in deinem Buch.",
            "Davos liegt in der Schweiz, ich sehe im Buch nach.",
        ]
        for (i, text) in allowed.enumerated() {
            let model = RoleModel(speaker: Script(pieces: ["\(text)\nSUCHE: Davos Sanatorium Castorp\n"]), thinker: answer)
            let seen = await ask(await newSession(model), castorpQuestion, id: "g\(i)")
            XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized], "erlaubt: \(text) -> \(seen.statuses)")
            XCTAssertEqual(seen.texts.first, text)
            XCTAssertEqual(seen.answer?.counts.droppedTemplate, 0, text)
            XCTAssertEqual(model.calls.roles, [.speaker, .thinker], text)
        }
    }

    /// Direkte Antworten werden nie gefiltert (auch nicht, wenn sie wie eine Floskel aussehen); die Zeilenfolge eines verworfenen
    /// Zwischensatzes hat eine Einheit „verworfen“ (Vorlage) und besteht den Prüfer.
    func testDirectAnswersAreNeverFilteredAndADroppedInterimSentenceKeepsTheJournalValid() async {
        let direct = RoleModel(speaker: Script(pieces: ["Ich denke kurz nach. Das Wetter kenne ich leider nicht."]))
        let seenDirect = await ask(await newSession(direct), weather)
        XCTAssertEqual(seenDirect.spoken, ["Ich denke kurz nach.", "Das Wetter kenne ich leider nicht."])
        XCTAssertEqual(seenDirect.answer?.counts.droppedTemplate, 0)
        let short = await ask(await newSession(RoleModel(speaker: Script(pieces: ["Okay."]))), weather)
        XCTAssertEqual(short.spoken, ["Okay."])
        XCTAssertEqual(short.statuses, [.conversational])

        let (s, audit) = await auditSession(RoleModel(speaker: Script(pieces: ["Ich schaue kurz nach.\nSUCHE: Davos\n"]), thinker: answer))
        _ = await ask(s)
        await close(s)
        XCTAssertEqual(journalTags(journal(audit)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:droppedTemplate", "round:data/naturalSearch", "areas:books", "step:summarized",
            "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
    }

    /// Die Zeilenfolge mit Zwischensatz: Sprecher-Phase mit dem Satz, dann die Datenphase; der Prüfer lässt sie zu, auch mit Überbrückung.
    func testJournalSequenceWithAnInterimSentence() async {
        let (s1, a1) = await auditSession(RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos\n"]), thinker: answer))
        _ = await ask(s1)
        await close(s1)
        XCTAssertEqual(journalTags(journal(a1)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "round:data/naturalSearch", "areas:books",
            "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a1)), [])

        let (s2, a2) = await auditSession(RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos\n"]), thinker: answer),
                                          config: config(bridge: true))
        _ = await ask(s2)
        await close(s2)
        XCTAssertEqual(journalTags(journal(a2)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "round:data/naturalSearch", "step:announcement",
            "areas:books", "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(a2)), [])
    }

    // MARK: Fortschritt für die Denk-Anzeige

    /// Direkte Antwort: nur `speaker`.
    func testProgressOfADirectAnswerIsOnlyTheSpeaker() async {
        let seen = await ask(await newSession(RoleModel(speaker: Script(pieces: ["Das Wetter kenne ich leider nicht."]))), weather)
        XCTAssertEqual(seen.progress, [.speaker])
        XCTAssertEqual(seen.statuses, [.conversational])
    }

    /// Zwischensatz und Suche: speaker, searching (mit den Suchwörtern), found (Titel der Auszüge im Prompt), composing, draft…; die Entwürfe
    /// sind kumulativ und der letzte ist der ganze Rohtext des Denkers. Gesprochen wird nur über `sentence`.
    func testProgressOfAnInterimSentenceAndASearch() async {
        let model = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\n"]), thinker: answer)
        let seen = await ask(await newSession(model))
        let shape = seen.progressShape
        XCTAssertEqual(Array(shape.prefix(4)), ["speaker", "searching", "found:Der Zauberberg", "composing"], "\(shape)")
        XCTAssertTrue(shape.dropFirst(4).allSatisfy { $0 == "draft" } && shape.count > 4, "\(shape)")
        XCTAssertEqual(seen.progress[1], .searching(query: "Davos Sanatorium Castorp"))
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        let drafts = seen.drafts
        XCTAssertEqual(drafts.last, answer.pieces.joined(), "am Ende einmal vollständig")
        for pair in zip(drafts, drafts.dropFirst()) { XCTAssertTrue(pair.1.hasPrefix(pair.0), "kumulativ") }
        XCTAssertEqual(Set(drafts).count, drafts.count, "kein Entwurf doppelt")
        // Alle Fortschrittsereignisse tragen Kennung und Generation der Sätze.
        let ids = Set(seen.events.compactMap { e -> String? in if case .progress(let id, _, _) = e { id } else { nil } })
        XCTAssertEqual(ids, ["r1"])
        let gens = Set(seen.events.compactMap { e -> Int? in
            switch e {
            case .progress(_, let g, _): g
            case .sentence(_, let g, _, _, _, _, _): g
            default: nil
            }
        })
        XCTAssertEqual(gens.count, 1)
    }

    /// Reihenfolge der Ereignisse insgesamt: Der Zwischensatz kommt zwischen `speaker` und `searching`.
    func testProgressAndSentencesInOrderForInterimSentenceAndSearch() async {
        let model = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos\n"]), thinker: answer)
        let seen = await ask(await newSession(model))
        let order = seen.orderShape
        // speaker, Satz des Zwischensatzes, searching, found, composing, dann Entwürfe und Sätze durchmischt.
        XCTAssertEqual(Array(order.prefix(5)), ["speaker", "sentence", "searching", "found:Der Zauberberg", "composing"], "\(order)")
    }

    /// Nur Suche (ohne Zwischensatz): speaker, searching, found, composing, draft…
    func testProgressOfASearchWithoutAnInterimSentence() async {
        let seen = await ask(await newSession(RoleModel(speaker: search, thinker: answer)))
        XCTAssertEqual(Array(seen.progressShape.prefix(4)), ["speaker", "searching", "found:Der Zauberberg", "composing"])
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
    }

    /// Direkter Inhaltsbezug (erzwungene Suche): speaker, searching (ohne Suchwörter: der Fragetext), found, composing.
    func testProgressOfADirectContentReferenceStartsWithTheSpeakerThenSearching() async {
        let seen = await ask(await newSession(RoleModel(thinker: answer)), ownQuestion)
        XCTAssertEqual(seen.progress.first, .speaker)
        XCTAssertEqual(seen.progress.dropFirst().first, .searching(query: ownQuestion))
        XCTAssertEqual(Array(seen.progressShape.prefix(4)), ["speaker", "searching", "found:Der Zauberberg", "composing"])
    }

    /// Suche ohne Treffer: speaker, searching, found (leer); weder composing noch draft, kein Denker.
    func testProgressOfASearchWithoutHits() async {
        let model = RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos\n"]), thinker: answer)
        let seen = await ask(await newSession(model, knowledge: AssistantKnowledge(corpus: AssistantCorpus())))
        XCTAssertEqual(seen.progress, [.speaker, .searching(query: "Davos"), .found(titles: [])])
        XCTAssertEqual(seen.statuses, [.conversational, .neutral])
        XCTAssertEqual(model.calls.roles, [.speaker], "kein Denker")
    }

    /// Eine Bestandsfrage hat keinen Fortschritt (kein Modell); im Stil verbatim gibt es nie Fortschritt.
    func testNoProgressForInventoryAndInVerbatim() async {
        let inv = await ask(await newSession(RoleModel(thinker: answer)), inventoryQuestion)
        XCTAssertTrue(inv.progress.isEmpty)
        var verbatim = config()
        verbatim.style = .verbatim
        let seen = await ask(await newSession(RoleModel(thinker: Script(pieces: ["Das Wetter ist schön."])), config: verbatim), weather)
        XCTAssertTrue(seen.progress.isEmpty)
    }

    /// Die Entwürfe sind gedrosselt (höchstens etwa alle 150 ms; der erste sofort) und am Ende einmal vollständig.
    func testDraftsAreThrottledAndComplete() async {
        // Viele Stücke ohne Pause: Es reicht, dass irgendwelche zwei innerhalb der 150 ms verarbeitet werden (bei vielen Stücken auch
        // auf einem ausgelasteten Rechner), damit die Zahl der Entwürfe unter der der Stücke bleibt.
        let pieces = (1...40).map { "Wort\($0) " } + ["Davos."]
        let model = RoleModel(speaker: search, thinker: Script(pieces: pieces))
        let seen = await ask(await newSession(model))
        let drafts = seen.drafts
        XCTAssertEqual(drafts.first, "Wort1 ", "der erste Entwurf sofort")
        XCTAssertEqual(drafts.last, pieces.joined(), "am Ende einmal vollständig")
        XCTAssertLessThan(drafts.count, pieces.count, "gedrosselt: nicht jedes Stück: \(drafts.count) Entwürfe")
        for pair in zip(drafts, drafts.dropFirst()) { XCTAssertTrue(pair.1.hasPrefix(pair.0)) }

        // Langsamer Denker: Je Stück ein Entwurf. Jedes weitere Stück wartet, bis der Entwurf davor bei der Anzeige ankam, und dann 200 ms
        // (mehr als die Drosselung): Der Abstand ist so auch bei einem ausgelasteten Rechner nie kürzer.
        let afterFirst = Latch()
        let afterSecond = Latch()
        let slow = RoleModel(speaker: search,
                             thinker: Script(pieces: ["Castorp ", "fährt ", "nach Davos."], pausesMs: [0, 200, 200])
                                .holding(1, until: afterFirst).holding(2, until: afterSecond))
        let s = await newSession(slow, config: config(timeout: 30))
        var slowDrafts: [String] = []
        for await e in s.events(AssistantRequest(requestId: "slow", text: castorpQuestion)) {
            guard case .progress(_, _, .draft(let text)) = e else { continue }
            slowDrafts.append(text)
            if text == "Castorp " { afterFirst.open() }
            if text == "Castorp fährt " { afterSecond.open() }
        }
        XCTAssertEqual(slowDrafts, ["Castorp ", "Castorp fährt ", "Castorp fährt nach Davos."])
    }

    /// Nach `cancel` und nach einem Widerruf kommt kein Fortschritt mehr.
    func testNoProgressAfterCancelOrRevocation() async {
        for revoke in [false, true] {
            // Das zweite Stück kommt erst, nachdem Abbruch bzw. Widerruf angewendet sind: Alles, was danach noch ankäme, wäre ein Fehler.
            let applied = Latch()
            let model = RoleModel(speaker: search,
                                  thinker: Script(pieces: ["Castorp ", "fährt ", "ins ", "Sanatorium ", "nach ", "Davos. "], pausesMs: [0, 200, 200, 200, 200, 200])
                                    .holding(1, until: applied))
            let s = await newSession(model)
            var after: [AssistantProgress] = []
            var stopped = false
            for await e in s.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
                guard case .progress(_, _, let p) = e else { continue }
                if stopped { after.append(p); continue }
                if case .draft = p {
                    stopped = true
                    if revoke { _ = s.applyReleaseChange(.other) } else { await s.cancel(requestId: "r1") }
                    applied.open()
                }
            }
            XCTAssertTrue(stopped, "es gab einen Entwurf")
            XCTAssertEqual(after, [], "nach \(revoke ? "Widerruf" : "Abbruch") kein Fortschritt mehr")
        }
        // Abbruch beim ersten Fortschritt (Sprecher hängt): nichts folgt.
        let hang = await newSession(RoleModel(speaker: Script(hang: true)))
        var later: [AssistantProgress] = []
        var cancelled = false
        for await e in hang.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
            guard case .progress(_, _, let p) = e else { continue }
            if cancelled { later.append(p) } else if p == .speaker {
                cancelled = true
                await hang.cancel(requestId: "r1")
            }
        }
        XCTAssertTrue(cancelled)
        XCTAssertEqual(later, [])
    }

    /// Fortschritt ist nur Anzeige: nicht im Protokoll (Titel, Suchwörter und Entwurf sind Inhalte) und nicht im Verlauf.
    func testProgressIsNeverWrittenToTheJournalOrTheHistory() async {
        let (s, audit) = await auditSession(RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\n"]), thinker: answer))
        let seen = await ask(s)
        await close(s)
        XCTAssertFalse(seen.progress.isEmpty)
        let raw = rawJournal(audit)
        for needle in ["draft", "searching", "composing", "progress", "found", "Davos Sanatorium Castorp", "Castorp fährt ins Sanatorium", "Der Zauberberg"] {
            XCTAssertFalse(raw.contains(needle), "im Protokoll: \(needle)")
        }
        XCTAssertEqual(journalTags(journal(audit)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "round:data/naturalSearch", "areas:books",
            "step:summarized", "step:summarized", "end",
        ], "genau die Zeilen wie ohne Fortschritt")
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
        let turn = await s.history.last
        XCTAssertFalse((turn?.assistant ?? "").contains("Davos Sanatorium Castorp"))
        XCTAssertEqual(turn?.parts.count, 2)
    }

    // MARK: Keine festen Floskeln (Standard)

    /// Standard: `bridge` und `fillers` aus. Der Kern gibt dann in keinem Fall einen festen Überbrückungs- oder Ansagesatz aus, auch bei
    /// langsamem Modell und gemeldeter Äußerung; es bleibt kein Zeitpunkt gemerkt.
    func testWithBridgeAndFillersOffNoFixedPhraseIsEverOutput() async {
        let defaults = AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        XCTAssertFalse(defaults.bridge)
        XCTAssertFalse(defaults.fillers)
        var c = config()
        XCTAssertFalse(c.bridge)
        XCTAssertFalse(c.fillers)
        c.fillerFirstMs = 0     // Zeiten gesetzt, aber `fillers` aus
        c.fillerSecondMs = 0
        let cases: [(String, RoleModel, String, AssistantKnowledge?)] = [
            ("direkt", RoleModel(speaker: Script(pieces: ["Hallo zusammen."], pausesMs: [400])), "Hallo", nil),
            ("Zwischensatz und Suche", RoleModel(speaker: Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos\n"], pausesMs: [300]),
                                                  thinker: Script(pieces: answer.pieces, pausesMs: [400])), castorpQuestion, nil),
            ("nur Suche", RoleModel(speaker: Script(pieces: [searchLine], pausesMs: [300]), thinker: Script(pieces: answer.pieces, pausesMs: [400])),
             castorpQuestion, nil),
            ("keine Treffer", RoleModel(speaker: Script(pieces: [searchLine], pausesMs: [300]), thinker: answer), castorpQuestion,
             AssistantKnowledge(corpus: AssistantCorpus())),
            ("direkter Bezug", RoleModel(thinker: Script(pieces: answer.pieces, pausesMs: [400])), ownQuestion, nil),
        ]
        for (name, model, text, knowledge) in cases {
            let s = await newSession(model, config: c, knowledge: knowledge)
            let stream = s.events(AssistantRequest(requestId: "r1", text: text))
            s.noteCommitted(requestId: "r1")
            var out: [AssistantEvent] = []
            for await e in stream { out.append(e) }
            let seen = Seen(events: out)
            XCTAssertTrue(seen.announcements.isEmpty, "\(name): \(seen.announcements)")
            for fixed in AssistantNatural.announcementTexts { XCTAssertFalse(seen.texts.contains(fixed), "\(name): \(fixed)") }
            XCTAssertNil(seen.failure, name)
            XCTAssertEqual(s.commitNoteCountsForTesting.times, 0, "\(name): kein Zeitpunkt gemerkt")
            XCTAssertEqual(s.commitNoteCountsForTesting.observers, 0, name)
        }
    }

    // MARK: Nebenläufigkeit

    func testConcurrentNaturalRequestsKeepTheirOwnLinesAndBridges() async {
        let (s, audit) = await auditSession(RoleModel(speaker: search, thinker: answer), config: config(bridge: true))
        let ids = (0..<8).map { "c\($0)" }
        let results = await withTaskGroup(of: (String, Seen).self) { group -> [(String, Seen)] in
            for id in ids { group.addTask { (id, await collect(s, AssistantRequest(requestId: id, text: castorpQuestion))) } }
            var out: [(String, Seen)] = []
            for await r in group { out.append(r) }
            return out
        }
        XCTAssertEqual(results.count, 8)
        for (id, seen) in results {
            XCTAssertEqual(seen.statuses, [.announcement, .summarized, .summarized], id)
            await close(s, id)
        }
        for id in ids {
            XCTAssertEqual(AssistantProtocolOrder.check(journal(audit, id)), [], id)
            XCTAssertEqual(journalTags(journal(audit, id)).count, 9, id)
        }
        let bridges = results.flatMap { $0.1.announcements }
        for bridge in AssistantNatural.bridges { XCTAssertEqual(bridges.filter { $0 == bridge }.count, 2, "reihum, jeder gleich oft: \(bridge)") }
    }

    // MARK: Prüfer, Erkennung, Prompts (reine Funktionen)

    private func line(_ t: AssistantJournalLine.Kind, gen: Int? = 1, _ body: (inout AssistantJournalLine) -> Void = { _ in }) -> AssistantJournalLine {
        var l = AssistantJournalLine(t, req: "r1")
        l.gen = gen
        body(&l)
        return l
    }

    private func start(_ style: AssistantStyle?) -> AssistantJournalLine { line(.start) { $0.style = style } }
    private func round(_ kind: AssistantRoundKind, _ reason: AssistantContext.Reason, areas: [String]? = nil) -> AssistantJournalLine {
        line(.round) { $0.round = kind; $0.reason = reason; $0.areas = areas }
    }
    private func areas(_ names: [String]) -> AssistantJournalLine { line(.areas) { $0.areas = names } }
    private func step(_ verdict: AssistantTraceVerdict) -> AssistantJournalLine { line(.step) { $0.n = 1; $0.verdict = verdict } }

    func testProtocolOrderAcceptsExactlyTheAllowedSequences() {
        typealias V = AssistantProtocolOrder.Violation
        let n = AssistantStyle.natural
        // erlaubt
        XCTAssertEqual(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.conversational), line(.end)]), [])
        XCTAssertEqual(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas(["books"]), round(.data, .naturalSearch, areas: ["books"]),
                                                     step(.announcement), areas(["books"]), step(.summarized), line(.end)]), [])
        XCTAssertEqual(AssistantProtocolOrder.check([start(n), round(.data, .naturalSearch, areas: []), areas(["books"]), step(.summarized)]), [])
        // Zwischensatz des Sprechers, dann (mit `bridge`) die Überbrückung am Anfang der Datenphase.
        XCTAssertEqual(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.conversational),
                                                     round(.data, .naturalSearch, areas: ["books"]), step(.announcement), areas(["books"]),
                                                     step(.summarized), line(.end)]), [])
        XCTAssertEqual(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.conversational),
                                                     round(.data, .naturalSearch, areas: ["books"]), areas(["books"]), step(.summarized),
                                                     step(.announcement)]), [V.announcementAfterAnswer], "nach einer Antworteinheit der Datenphase nicht")
        XCTAssertEqual(AssistantProtocolOrder.check([start(nil), round(.data, .locked), areas(["books"]), step(.announcement), step(.grounded)]), [])
        // verboten
        XCTAssertEqual(AssistantProtocolOrder.check([start(nil), round(.data, .locked), round(.general, .free)]), [V.roundCount])
        XCTAssertEqual(AssistantProtocolOrder.check([start(nil), round(.data, .naturalSearch, areas: [])]), [V.naturalInVerbatim])
        XCTAssertEqual(AssistantProtocolOrder.check([start(nil), round(.data, .locked), areas([]), step(.summarized)]), [V.naturalInVerbatim])
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), step(.conversational)]).contains(V.speakerAreasMissing))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.data, .naturalSearch)]).contains(V.retrievalAreasMissing))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.data, .naturalSearch, areas: []), step(.summarized)]).contains(V.dataAreasMissing))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.data, .naturalSearch, areas: []), areas([]), step(.conversational)]).contains(V.verdictNotAllowed))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.summarized)]).contains(V.verdictNotAllowed))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), round(.general, .naturalTalk)]).contains(V.roundNotAllowed))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.data, .naturalSearch, areas: []), areas([]), round(.data, .naturalSearch, areas: [])]).contains(V.roundNotAllowed))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), step(.conversational)]).contains(V.stepBeforeRound))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), areas([])]).contains(V.areasBeforeRound))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), line(.end), step(.conversational)]).contains(V.lineAfterEnd))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), line(.round, gen: 2) { $0.round = .general; $0.reason = .naturalTalk }]).contains(V.generationMismatch))
        XCTAssertEqual(AssistantProtocolOrder.check([line(.round) { $0.round = .data }]), [V.startMissing])
        // Ansagen: nach einer Antworteinheit nicht mehr; höchstens drei
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.conversational), step(.announcement)]).contains(V.announcementAfterAnswer))
        XCTAssertTrue(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.announcement), step(.announcement),
                                                    step(.announcement), step(.announcement)]).contains(V.tooManyAnnouncements))
        XCTAssertEqual(AssistantProtocolOrder.check([start(n), round(.general, .naturalTalk), areas([]), step(.announcement), step(.announcement),
                                                     step(.conversational)]), [])
    }

    /// Das Tor trennt Text und Steuerzeile an jedem Zeilenanfang; die Steuerzeile geht nie als Text hinaus.
    func testSpeakerGateSeparatesTextFromTheControlLineAtEveryLineStart() {
        typealias Gate = AssistantNatural.SpeakerGate
        typealias Step = Gate.Step
        func search(_ words: String, _ complete: Bool) -> Gate.Search { Gate.Search(words: words, complete: complete) }

        // Nur Steuerzeile, über Stückgrenzen.
        var only = Gate()
        XCTAssertEqual(only.feed("SU"), Step())
        XCTAssertEqual(only.feed("CHE "), Step())
        XCTAssertEqual(only.feed(": Davos"), Step(speech: "", search: search(" Davos", false)))
        XCTAssertEqual(only.feed(" Berg\nRest"), Step(speech: "", search: search(" Davos Berg\nRest", true)))

        // Zwischensatz, dann Steuerzeile in neuer Zeile: der Satz geht als Text hinaus, die Zeile nicht.
        var interim = Gate()
        XCTAssertEqual(interim.feed("Ich sehe nach.\nSU"), Step(speech: "Ich sehe nach.\n"), "der Anfang der nächsten Zeile wird zurückgehalten")
        XCTAssertEqual(interim.feed("CHE: Davos\n"), Step(speech: "", search: search(" Davos\n", true)))
        var oneShot = Gate()
        XCTAssertEqual(oneShot.feed("Ich sehe nach.\nSUCHE: Davos\n"), Step(speech: "Ich sehe nach.\n", search: search(" Davos\n", true)))
        var blank = Gate()
        XCTAssertEqual(blank.feed("Ich sehe nach.\n\n  \nsuche : Davos"), Step(speech: "Ich sehe nach.\n", search: search(" Davos", false)),
                       "Leerzeilen vor der Steuerzeile gehören nicht zum Text")

        // Direkte Antwort: alles geht als Text hinaus, auch das Übersprungene.
        var talk = Gate()
        XCTAssertEqual(talk.feed("  \"Hallo"), Step(speech: "  \"Hallo"))
        XCTAssertEqual(talk.feed(" du.\nSonne scheint."), Step(speech: " du.\nSonne scheint."))
        XCTAssertEqual(talk.finish(), Step())

        // Ein bloßes „Suche“ am Zeilenanfang oder am Ende ist keine Steuerzeile.
        var bare = Gate()
        XCTAssertEqual(bare.feed("Hallo.\nSuche"), Step(speech: "Hallo.\n"))
        XCTAssertEqual(bare.finish(), Step(speech: "Suche"), "ein bloßes „Suche“ am Ende ist keine Suche")
        var colon = Gate()
        XCTAssertEqual(colon.feed("Suche Kaffee: ja."), Step(speech: "Suche Kaffee: ja."))

        // Mitten in einer Zeile: NUR „SUCHE:“ in Großbuchstaben, NACH Satzende (. ! ? …) und Leerraum, ist eine Steuerzeile; der Satz davor
        // geht als Text hinaus (mit dem Leerraum), die Zeile nicht.
        for end in [".", "!", "?", "\u{2026}"] {
            var inside = Gate()
            XCTAssertEqual(inside.feed("Moin\(end) SUCHE: Davos"), Step(speech: "Moin\(end) ", search: search(" Davos", false)), end)
            XCTAssertEqual(inside.feed(" Berg\nRest"), Step(speech: "", search: search(" Davos Berg\nRest", true)), end)
        }
        var pieces = Gate()
        XCTAssertEqual(pieces.feed("Moin."), Step(speech: "Moin."))
        XCTAssertEqual(pieces.feed(" "), Step(speech: " "))
        XCTAssertEqual(pieces.feed("S"), Step(), "der Anfang wird zurückgehalten")
        XCTAssertEqual(pieces.feed("UCHE"), Step())
        XCTAssertEqual(pieces.feed(" :"), Step(speech: "", search: search("", false)))
        XCTAssertEqual(pieces.feed(" Davos\n"), Step(speech: "", search: search(" Davos\n", true)))
        // Mehr Leerraum, auch ein Tabulator, vor der Zeile
        var spaced = Gate()
        XCTAssertEqual(spaced.feed("Moin.  \t SUCHE: Davos\n"), Step(speech: "Moin.  \t ", search: search(" Davos\n", true)))
        // Kein Satzende davor, kein Leerraum danach, klein oder gemischt geschrieben: Text, unverändert und vollständig
        for text in ["Moin, SUCHE: Davos.", "Moin.SUCHE: Davos.", "Moin! suche: Davos.", "Moin! Suche: Davos.", "Ich SUCHE: Davos.", "Moin! SUCHEN: Davos.",
                     "Moin! SUCHE Davos.", "Moin! SUCH", "Moin! SUCHE"] {
            var plain = Gate()
            var out = plain.feed(text)
            let rest = plain.finish()
            XCTAssertNil(out.search, text)
            XCTAssertNil(rest.search, text)
            out.speech += rest.speech
            XCTAssertEqual(out.speech, text, "alles geht als Text hinaus, auch das Zurückgehaltene: \(text)")
        }
        // Dasselbe über Stückgrenzen
        var cut = Gate()
        var spoken = ""
        for piece in ["Moin! ", "Su", "che", ": Davos."] { spoken += cut.feed(piece).speech }
        spoken += cut.finish().speech
        XCTAssertEqual(spoken, "Moin! Suche: Davos.")

        // Stromende in der Steuerzeile: sie gilt als abgeschlossen.
        var open = Gate()
        _ = open.feed("SUCHE: Davos")
        XCTAssertEqual(open.finish(), Step(speech: "", search: search(" Davos", true)))

        var long = Gate()
        _ = long.feed("SUCHE: ")
        let step = long.feed(String(repeating: "x", count: 2 * AssistantNatural.maxSearchChars + 1))
        XCTAssertEqual(step.search?.complete, true, "Obergrenze der Zeile")
        XCTAssertLessThanOrEqual(AssistantNatural.searchWords(String(repeating: "wort ", count: 100)).count, AssistantNatural.maxSearchChars)
        XCTAssertEqual(AssistantNatural.searchWords(" \"Davos  Sanatorium.\" \nzweite Zeile"), "Davos Sanatorium")
    }

    func testPromptsOfTheNaturalStyle() {
        let c = config()
        let history = (0..<10).map { naturalTurn("Frage \($0)", "Antwort \($0).", round: .general) }
        let built = AssistantPrompt.buildSpeaker(question: "Hallo?", afterInterruption: false, history: history, config: c)
        XCTAssertEqual(built.history.count, AssistantPrompt.speakerHistoryTurns)
        XCTAssertTrue(built.prompt.contains("Frage 9"))
        XCTAssertTrue(built.prompt.contains("Frage 4"))
        XCTAssertFalse(built.prompt.contains("Frage 3"), "höchstens sechs Beiträge")
        XCTAssertFalse(built.prompt.contains("AUSZÜGE"))
        let bare = AssistantPrompt.buildSpeaker(question: "Hallo?", afterInterruption: false, history: history, config: c, includeHistory: false)
        XCTAssertTrue(bare.history.isEmpty)
        XCTAssertFalse(bare.prompt.contains("VERLAUF"))
        XCTAssertTrue(bare.prompt.contains("Hallo?"))

        let long = String(repeating: "Hans Castorp fährt ins Sanatorium nach Davos. ", count: 40)
        let passages = (0..<4).map { assistantPassage(.books, itemId: "b\($0)", title: "Buch \($0)", text: long) }
        let thinker = AssistantPrompt.buildThinker(question: "Frage?", afterInterruption: false, passages: passages, history: [], config: config(passageChars: 2_000))
        XCTAssertFalse(thinker.used.isEmpty)
        XCTAssertLessThan(thinker.used.count, 4, "das kleinere Budget des Stils begrenzt die Auszüge")
        XCTAssertLessThanOrEqual(thinker.used.reduce(0) { $0 + $1.text.count }, 2_000)
        XCTAssertTrue(thinker.prompt.contains("AUSZÜGE"))
        XCTAssertEqual(AssistantPrompt.naturalPassageBudget(config: config(passageChars: 500)), 500)
        let ti = AssistantPrompt.thinkerInstructions(config: c, now: Date(), timeZone: .current)
        XCTAssertTrue(ti.contains("AUSZÜGE"))
        XCTAssertFalse(ti.contains("SUCHE:"))

        // Der Sprecher darf EINEN kurzen Satz vor der Zeile schreiben, soll dabei etwas Konkretes sagen und darf keine Floskel nutzen.
        let si = AssistantPrompt.speakerInstructions(config: c, now: Date(), timeZone: .current)
        XCTAssertTrue(si.contains("SUCHE:"))
        XCTAssertTrue(si.contains("EINEN kurzen, natürlichen Satz"))
        XCTAssertTrue(si.contains("NEUEN Zeile"))
        XCTAssertTrue(si.contains("etwas Konkretes"))
        for phrase in ["Ich denke nach", "Einen Moment", "Ich schaue kurz nach", "Lass mich überlegen"] {
            XCTAssertTrue(si.contains("„\(phrase)“"), "ausdrücklich verboten: \(phrase)")
        }
        XCTAssertTrue(si.contains("Verboten sind Floskeln"))
    }

    // MARK: Ansagen für eine langsame Antwort

    /// Erste Ansage vor dem ersten Satz, wenn das Modell langsam ist; sie ist keine Antwort: nicht im Text, nicht im Verlauf, nicht gezählt.
    func testFirstFillerComesBeforeTheFirstSentenceAndIsNeitherAnswerTextNorHistory() async {
        let audit = AssistantAuditLog(directory: tempDir())
        // Das Modell antwortet erst, nachdem die Ansage bei der Anzeige ankam (Haltepunkt statt Zeitabstand).
        let release = Latch()
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]).holding(0, until: release))
        let s = await newSession(model, config: config(first: 50, second: 10_000), audit: audit)
        let stream = s.events(AssistantRequest(requestId: "r1", text: "Hallo"))
        s.noteCommitted(requestId: "r1")
        var out: [AssistantEvent] = []
        for await e in stream {
            out.append(e)
            if case .sentence(_, _, _, _, .announcement, _, _) = e { release.open() }
        }
        let seen = Seen(events: out)
        XCTAssertEqual(seen.statuses, [.announcement, .conversational])
        XCTAssertEqual(seen.texts, [AssistantNatural.fillerFirstSentence, "Hallo zusammen."])
        XCTAssertEqual(seen.answer?.text, "Hallo zusammen.")
        XCTAssertEqual(seen.answer?.counts.conversational, 1)
        XCTAssertEqual(seen.answer?.counts.neutral, 0)
        await close(s, "r1", user: "Hallo")
        let turn = await s.history.last
        XCTAssertEqual(turn?.assistant, "Hallo zusammen.")
        XCTAssertEqual(turn?.parts.map(\.text), ["Hallo zusammen."])
        XCTAssertEqual(journalTags(journal(audit)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:announcement", "step:conversational", "end",
        ], "die Spur trägt die Ansage vor dem Satz")
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
    }

    func testNoFirstFillerWhenABridgeOrASentenceIsAlreadyThere() async {
        // Überbrückung schon da (Suche sofort, Denker langsam): keine erste Ansage. Die Zeiten haben reichlich Spielraum (Faktor 5 und mehr)
        // für einen langsamen Rechner: Die Überbrückung muss vor der ersten Ansage (1 s) hinausgehen, der Denker kommt nach ihr (1,5 s).
        let m1 = RoleModel(speaker: search, thinker: Script(pieces: answer.pieces, pausesMs: [1_500]))
        let s1 = await newSession(m1, config: config(bridge: true, first: 1_000, second: 10_000))
        let stream1 = s1.events(AssistantRequest(requestId: "r1", text: castorpQuestion))
        s1.noteCommitted(requestId: "r1")
        var out1: [AssistantEvent] = []
        for await e in stream1 { out1.append(e) }
        let seen1 = Seen(events: out1)
        XCTAssertEqual(seen1.statuses, [.announcement, .summarized, .summarized])
        XCTAssertTrue(AssistantNatural.bridges.contains(seen1.texts.first ?? ""), "die Überbrückung, nicht die Ansage")

        // Satz schon da (zweiter Satz langsam): keine erste Ansage.
        let m2 = RoleModel(speaker: Script(pieces: ["Hallo zusammen. Wie ", "geht es dir?"], pausesMs: [0, 1_500]))
        let s2 = await newSession(m2, config: config(first: 1_000, second: 10_000))
        let stream2 = s2.events(AssistantRequest(requestId: "r1", text: "Hallo"))
        s2.noteCommitted(requestId: "r1")
        var out2: [AssistantEvent] = []
        for await e in stream2 { out2.append(e) }
        XCTAssertEqual(Seen(events: out2).statuses, [.conversational, .conversational])
    }

    /// Kommt der Hinweis erst, nachdem Einheiten hinausgingen, entfällt die erste Ansage; die zweite gilt, solange keine Antworteinheit da ist.
    func testFirstFillerIsOmittedWhenTheNoticeComesAfterUnitsWereEmitted() async {
        // Der Denker antwortet erst, nachdem die zweite Ansage bei der Anzeige ankam (Haltepunkt statt Zeitabstand).
        let secondSeen = Latch()
        let model = RoleModel(speaker: search, thinker: Script(pieces: answer.pieces).holding(0, until: secondSeen))
        let s = await newSession(model, config: config(bridge: true, first: 0, second: 250))
        var out: [AssistantEvent] = []
        var noted = false
        for await e in s.events(AssistantRequest(requestId: "r1", text: castorpQuestion)) {
            out.append(e)
            if !noted, case .sentence(_, _, _, _, .announcement, _, _) = e {
                noted = true
                s.noteCommitted(requestId: "r1")
            } else if case .sentence(_, _, let text, _, .announcement, _, _) = e, text == AssistantNatural.fillerSecondSentence {
                secondSeen.open()
            }
        }
        let seen = Seen(events: out)
        XCTAssertEqual(seen.statuses, [.announcement, .announcement, .summarized, .summarized])
        XCTAssertTrue(AssistantNatural.bridges.contains(seen.texts.first ?? ""))
        XCTAssertEqual(seen.texts.dropFirst().first, AssistantNatural.fillerSecondSentence)
        XCTAssertFalse(seen.texts.contains(AssistantNatural.fillerFirstSentence))
    }

    func testSecondFillerComesOnlyWhileNoAnswerUnitWasEmitted() async {
        // Ganz langsam: beide Ansagen vor dem ersten Satz, in dieser Reihenfolge.
        let secondSeen1 = Latch()
        let m1 = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]).holding(0, until: secondSeen1))
        let s1 = await newSession(m1, config: config(first: 40, second: 250))
        let stream1 = s1.events(AssistantRequest(requestId: "r1", text: "Hallo"))
        s1.noteCommitted(requestId: "r1")
        var out1: [AssistantEvent] = []
        for await e in stream1 {
            out1.append(e)
            if case .sentence(_, _, let text, _, .announcement, _, _) = e, text == AssistantNatural.fillerSecondSentence { secondSeen1.open() }
        }
        XCTAssertEqual(Seen(events: out1).texts, [AssistantNatural.fillerFirstSentence, AssistantNatural.fillerSecondSentence, "Hallo zusammen."])

        // Der erste Satz kommt nach der ersten Ansage und vor der zweiten (fällig nach 1 s), der zweite lässt sich Zeit (1,5 s nach dem ersten,
        // also nach der Fälligkeit der zweiten Ansage): keine zweite Ansage.
        let firstSeen2 = Latch()
        let m2 = RoleModel(speaker: Script(pieces: ["Hallo zusammen. Wie ", "geht es dir?"], pausesMs: [0, 1_500]).holding(0, until: firstSeen2))
        let s2 = await newSession(m2, config: config(first: 40, second: 1_000))
        let stream2 = s2.events(AssistantRequest(requestId: "r1", text: "Hallo"))
        s2.noteCommitted(requestId: "r1")
        var out2: [AssistantEvent] = []
        for await e in stream2 {
            out2.append(e)
            if case .sentence(_, _, let text, _, .announcement, _, _) = e, text == AssistantNatural.fillerFirstSentence { firstSeen2.open() }
        }
        XCTAssertEqual(Seen(events: out2).texts, [AssistantNatural.fillerFirstSentence, "Hallo zusammen.", "Wie geht es dir?"])

        // Überbrückung, langsamer Denker: erste Ansage entfällt (Überbrückung ist eine Einheit), zweite kommt (keine Antworteinheit).
        // Die Überbrückung muss vor der ersten Ansage (0,8 s) hinausgehen; der Denker antwortet erst nach der zweiten Ansage.
        let secondSeen3 = Latch()
        let m3 = RoleModel(speaker: search, thinker: Script(pieces: answer.pieces).holding(0, until: secondSeen3))
        let s3 = await newSession(m3, config: config(bridge: true, first: 800, second: 1_000))
        let stream3 = s3.events(AssistantRequest(requestId: "r1", text: castorpQuestion))
        s3.noteCommitted(requestId: "r1")
        var out3: [AssistantEvent] = []
        for await e in stream3 {
            out3.append(e)
            if case .sentence(_, _, let text, _, .announcement, _, _) = e, text == AssistantNatural.fillerSecondSentence { secondSeen3.open() }
        }
        let seen3 = Seen(events: out3)
        XCTAssertEqual(seen3.statuses, [.announcement, .announcement, .summarized, .summarized])
        XCTAssertEqual(seen3.texts.dropFirst().first, AssistantNatural.fillerSecondSentence)
    }

    func testNoFillersWithoutTheCommitNotice() async {
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."], pausesMs: [500]))
        let s = await newSession(model, config: config(first: 10, second: 20))
        let seen = await ask(s, "Hallo")
        XCTAssertEqual(seen.statuses, [.conversational], "ohne `noteCommitted` keine Ansagen (Vorarbeit, ältere Aufrufer)")
        XCTAssertEqual(s.commitNoteCountsForTesting.observers, 0, "der Lauf meldet sich am Ende ab, auch wenn nie ein Hinweis kam")
        XCTAssertEqual(s.commitNoteCountsForTesting.times, 0)
    }

    func testTheCommitNoticeMayComeBeforeTheRunStarts() async {
        // `first: 0`: Die erste Ansage ist schon fällig, sobald der Lauf anfängt (keine Wartezeit zur Abstimmung); das Modell antwortet erst,
        // nachdem sie bei der Anzeige ankam.
        let release = Latch()
        let model = RoleModel(speaker: Script(pieces: ["Hallo zusammen."]).holding(0, until: release))
        let s = await newSession(model, config: config(first: 0, second: 10_000))
        s.noteCommitted(requestId: "r1")
        var out: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Hallo")) {
            out.append(e)
            if case .sentence(_, _, _, _, .announcement, _, _) = e { release.open() }
        }
        let seen = Seen(events: out)
        XCTAssertEqual(seen.texts.first, AssistantNatural.fillerFirstSentence, "schon fällig, sobald der Lauf anfängt")
        XCTAssertEqual(seen.statuses, [.announcement, .conversational])
    }

    func testNoteCommittedHasNoEffectInTheVerbatimStyle() async {
        var verbatim = config(first: 0, second: 0)
        verbatim.style = .verbatim
        let s = await newSession(RoleModel(thinker: Script(pieces: ["Unsinn."], pausesMs: [300])), config: verbatim)
        let stream = s.events(AssistantRequest(requestId: "r1", text: castorpQuestion))
        s.noteCommitted(requestId: "r1")
        var out: [AssistantEvent] = []
        for await e in stream { out.append(e) }
        let announced = Seen(events: out).announcements
        XCTAssertFalse(announced.contains(AssistantNatural.fillerFirstSentence) || announced.contains(AssistantNatural.fillerSecondSentence), "keine Ansage im Stil verbatim: \(announced)")
        XCTAssertEqual(s.commitNoteCountsForTesting.times, 0)
    }

    /// Die Ansagen gehen nie nebenläufig hinaus: Satzspur und Ereignisse haben dieselbe Reihenfolge, die Schritte sind lückenlos aufsteigend,
    /// jeder Schritt hat genau eine Ergebniszeile des Ausgabe-Commits.
    func testFillerStepsAreSequentialAndEachHasOneEmitResult() async {
        let audit = AssistantAuditLog(directory: tempDir())
        let model = RoleModel(speaker: search, thinker: Script(pieces: ["Eins. ", "Zwei. ", "Drei."], pausesMs: [300, 300, 300]))
        let s = await newSession(model, config: config(bridge: true, first: 0, second: 0), audit: audit)
        let stream = s.events(AssistantRequest(requestId: "r1", text: castorpQuestion))
        s.noteCommitted(requestId: "r1")
        var out: [AssistantEvent] = []
        for await e in stream { out.append(e) }
        await close(s)
        let seen = Seen(events: out)
        let steps = journal(audit).filter { $0.t == .step }
        XCTAssertEqual(steps.compactMap(\.n), Array(1...steps.count), "lückenlos aufsteigend")
        XCTAssertEqual(steps.compactMap(\.verdict), seen.statuses.map { AssistantTraceVerdict(status: $0) }, "Spur = Reihenfolge der Ereignisse")
        let marks = journal(audit).filter { $0.t == .commit && $0.mark?.stage == .emit }
        XCTAssertEqual(marks.compactMap(\.n).sorted(), Array(1...steps.count))
        XCTAssertTrue(marks.allSatisfy { $0.mark?.ok == true })
        XCTAssertLessThanOrEqual(seen.announcements.count, 3)
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
    }

    /// Abbruch beendet die Zeitgeber: nach dem Abbruch kommt keine Ansage mehr, und es bleibt nichts angemeldet.
    func testFillersEndWithCancelAndRevoke() async {
        for revoke in [false, true] {
            let audit = AssistantAuditLog(directory: tempDir())
            let model = RoleModel(speaker: Script(hang: true))
            let s = await newSession(model, config: config(first: 40, second: 600), audit: audit)
            let stream = s.events(AssistantRequest(requestId: "r1", text: "Hallo"))
            let commitAt = ContinuousClock.now
            s.noteCommitted(requestId: "r1")
            var out: [AssistantEvent] = []
            var done = false
            for await e in stream {
                out.append(e)
                if !done, case .sentence(_, _, let text, _, .announcement, _, _) = e, text == AssistantNatural.fillerFirstSentence {
                    done = true
                    if revoke { _ = s.applyReleaseChange(.other) } else { await s.cancel(requestId: "r1") }
                }
            }
            let seen = Seen(events: out)
            XCTAssertEqual(seen.failure, revoke ? .revoked : .cancelled)
            XCTAssertEqual(seen.announcements, [AssistantNatural.fillerFirstSentence], "keine zweite Ansage nach dem Ende")
            // Die zweite Ansage wäre 600 ms nach dem Hinweis fällig gewesen: bis 500 ms danach (vom Hinweis an gemessen, nicht vom Ende des
            // Laufs) wird nichts mehr hinausgehen oder angemeldet bleiben.
            try? await Task.sleep(until: commitAt.advanced(by: .milliseconds(1_100)), clock: .continuous)
            await close(s, "r1", user: "Hallo")
            XCTAssertEqual(journal(audit).filter { $0.t == .step && $0.verdict == .announcement }.count, 1)
            XCTAssertEqual(s.commitNoteCountsForTesting.observers, 0, "nichts bleibt angemeldet")
            XCTAssertEqual(s.commitNoteCountsForTesting.times, 0, "nach dem Abschluss der Anfrage ist ihr Zeitpunkt vergessen")
        }
    }

    // MARK: Hinweis „Äußerung gilt“ und Zeitgeber (Bausteine)

    func testCommitNotesReportEachNoticeExactlyOnceAndTheFirstCallWins() {
        let notes = AssistantCommitNotes()
        let calls = Box<[ContinuousClock.Instant]>([])
        let t0 = ContinuousClock.now
        notes.attach("a") { calls.value.append($0) }
        XCTAssertTrue(calls.value.isEmpty)
        notes.note("a", at: t0)
        notes.note("a", at: t0.advanced(by: .seconds(5)))
        XCTAssertEqual(calls.value, [t0], "genau einmal, der erste Zeitpunkt")

        notes.note("b", at: t0)
        notes.attach("b") { calls.value.append($0) }
        XCTAssertEqual(calls.value, [t0, t0], "eine frühere Meldung wird sofort gemeldet")

        notes.attach("c") { _ in XCTFail("nach detach darf nichts mehr gerufen werden") }
        notes.detach("c")
        notes.note("c")
        XCTAssertEqual(notes.counts.observers, 0)
        notes.forget("c")
        notes.forget("b")
        XCTAssertEqual(notes.counts.times, 1)
    }

    func testCommitNotesStayBounded() {
        let notes = AssistantCommitNotes()
        for i in 0..<(AssistantCommitNotes.maxKept * 3) { notes.note("x\(i)") }
        XCTAssertEqual(notes.counts.times, AssistantCommitNotes.maxKept)
    }

    /// Anmelden und Eintreffen laufen gleichzeitig: Der Zeitpunkt geht nie verloren und kommt nie zweimal.
    func testCommitNotesAttachAndNoteRaceNeverLosesTheCommit() async {
        for round in 0..<200 {
            let notes = AssistantCommitNotes()
            let calls = Counter()
            let id = "r\(round)"
            await withTaskGroup(of: Void.self) { group in
                group.addTask { notes.attach(id) { _ in calls.increment() } }
                group.addTask { notes.note(id) }
            }
            XCTAssertEqual(calls.value, 1, "Runde \(round)")
        }
    }

    func testFillerTimersMarkDueOnceAndStopOnShutdown() async {
        // Nur die erste Ansage fällig (die zweite weit weg): genau [1], einmal.
        let firstWake = Latch()
        let onlyFirst = AssistantNaturalFillers(firstMs: 0, secondMs: 600_000) { firstWake.open() }
        XCTAssertTrue(onlyFirst.takeDue().isEmpty, "ohne `start` nichts fällig")
        onlyFirst.start(commit: ContinuousClock.now.advanced(by: .milliseconds(-10)))
        onlyFirst.start(commit: ContinuousClock.now)
        let woken = await firstWake.wait()
        XCTAssertTrue(woken)
        XCTAssertEqual(onlyFirst.takeDue(), [1])
        XCTAssertTrue(onlyFirst.takeDue().isEmpty, "jede Ansage höchstens einmal")
        onlyFirst.shutdown()

        // Beide fällig: in der Reihenfolge 1, 2, jede einmal, auch wenn das zweite `start` nichts ändert.
        let wakes = Counter()
        let secondWake = Latch()
        let both = AssistantNaturalFillers(firstMs: 0, secondMs: 40) {
            if wakes.increment() >= 2 { secondWake.open() }
        }
        both.start(commit: ContinuousClock.now)
        both.start(commit: ContinuousClock.now)
        let wokenTwice = await secondWake.wait()
        XCTAssertTrue(wokenTwice)
        XCTAssertEqual(both.takeDue(), [1, 2])
        XCTAssertTrue(both.takeDue().isEmpty, "jede Ansage höchstens einmal")
        XCTAssertEqual(wakes.value, 2, "ein zweites `start` ändert nichts")
        both.shutdown()

        // `shutdown` gleich nach `start`: Der Zeitgeber (hier 300 ms) weckt nichts mehr; bis weit nach seiner Fälligkeit warten.
        let lateWakes = Counter()
        let late = AssistantNaturalFillers(firstMs: 300, secondMs: -1) { lateWakes.increment() }
        let startedAt = ContinuousClock.now
        late.start(commit: startedAt)
        late.shutdown()
        try? await Task.sleep(until: startedAt.advanced(by: .milliseconds(800)), clock: .continuous)
        XCTAssertEqual(lateWakes.value, 0, "nach `shutdown` weckt kein Zeitgeber mehr")
        XCTAssertTrue(late.takeDue().isEmpty)
    }

    // MARK: Offener Suchauftrag (`AssistantRequest.pending`) und `ABBRUCH`

    /// Frage und Suchwörter mit einem Wort („Waldmeister“), das weder im Korpus noch in einer Antwort vorkommt: Wo es auftaucht, stammt es
    /// aus dem offenen Suchauftrag.
    private let openSearch = AssistantPendingSearch(question: "Wer ist Frau Waldmeister im Zauberberg?", query: "Waldmeister Zauberberg")
    private let farewell = "Alles klar, dann lassen wir das."

    private func askPending(_ s: AssistantSession, _ text: String = "Und wie geht es dir?", pending: AssistantPendingSearch? = nil,
                            id: String = "r1") async -> Seen {
        await collect(s, AssistantRequest(requestId: id, text: text, pending: pending ?? openSearch))
    }

    /// Der Block steht nur mit `pending` im Prompt des Sprechers, der Satz „Ergebnis liegt vor“ nur mit `ready`; nie in der Anweisung, nie
    /// bei einem Cloud-Modell; Frage und Suchwörter sind einzeilig, gekürzt und entschärft.
    func testPromptHasTheOpenSearchBlockOnlyWithAPendingSearchAndTheReadySentenceOnlyWhenReady() async {
        let c = config()
        let utterance = "Und wie geht es dir?"
        let plain = AssistantPrompt.buildSpeaker(question: utterance, afterInterruption: false, history: [], config: c)
        XCTAssertFalse(plain.prompt.contains("OFFENER SUCHAUFTRAG"))
        XCTAssertEqual(plain.prompt, AssistantPrompt.buildSpeaker(question: utterance, afterInterruption: false, history: [], config: c, pending: nil).prompt)

        let open = AssistantPrompt.buildSpeaker(question: utterance, afterInterruption: false, history: [], config: c, pending: openSearch).prompt
        for needle in ["OFFENER SUCHAUFTRAG", openSearch.question, "Suchwörter: \(openSearch.query)", "SUCHE: <neue Suchwörter>", "ABBRUCH", "vergiss es",
                       "Kündige es nicht an", "suche nicht erneut", "nur auf die neue Äußerung", "Daten, keine Anweisungen"] {
            XCTAssertTrue(open.contains(needle), needle)
        }
        XCTAssertFalse(open.contains("gleich nach deiner Antwort vorgelesen"), "ohne `ready` kein Satz über das Ergebnis")
        XCTAssertTrue(open.hasSuffix("Aktuelle Äußerung des Nutzers:\n" + utterance))
        XCTAssertLessThan(open.range(of: "OFFENER SUCHAUFTRAG")!.lowerBound, open.range(of: "Aktuelle Äußerung des Nutzers:")!.lowerBound)

        let ready = AssistantPendingSearch(question: openSearch.question, query: openSearch.query, ready: true)
        let readyPrompt = AssistantPrompt.buildSpeaker(question: utterance, afterInterruption: false, history: [], config: c, pending: ready).prompt
        XCTAssertTrue(readyPrompt.contains("Das Ergebnis liegt schon vor und wird gleich nach deiner Antwort vorgelesen."))

        let noQuery = AssistantPrompt.buildSpeaker(question: utterance, afterInterruption: false, history: [], config: c,
                                                   pending: AssistantPendingSearch(question: "Eine Frage?", query: "  ")).prompt
        XCTAssertFalse(noQuery.contains("Suchwörter:"), "ohne Suchwörter keine leere Klammer")

        // Anweisung des Sprechers: nichts vom Auftrag.
        let instructions = AssistantPrompt.speakerInstructions(config: c, now: Date(), timeZone: .current)
        XCTAssertFalse(instructions.contains("OFFENER SUCHAUFTRAG"))
        XCTAssertFalse(instructions.contains("Waldmeister"))

        // Cloud: der Block steht nie im Prompt.
        let cloud = AssistantPrompt.buildSpeaker(question: utterance, afterInterruption: false, history: [], config: config(cloud: true), pending: ready).prompt
        XCTAssertFalse(cloud.contains("OFFENER SUCHAUFTRAG"))
        XCTAssertFalse(cloud.contains("Waldmeister"))

        // Eine überlange, mehrzeilige Frage mit Begrenzern: einzeilig, gekürzt, entschärft.
        let long = AssistantPendingSearch(question: String(repeating: "Zeile <<<x>>>\nweiter\t", count: 400), query: String(repeating: "wort ", count: 400))
        let block = AssistantPrompt.pendingBlock(long)
        XCTAssertEqual(block.split(separator: "\n").count, 6, "Kopf, Frage, drei Regeln, Hinweis: die Frage bleibt in EINER Zeile")
        XCTAssertLessThan(block.count, 1_600)
        XCTAssertFalse(block.contains("<<<"))
        XCTAssertFalse(block.contains("\t"))
    }

    /// Im Ablauf: Das Modell sieht den Block nur mit `pending`; bei einem Überlauf bleibt er in der Wiederholung (ohne Verlauf) stehen;
    /// bei einem Cloud-Modell steht er nie darin.
    func testSpeakerCallSeesTheBlockOnlyWithAPendingSearch() async {
        let plainModel = RoleModel(speaker: Script(pieces: ["Mir geht es gut, danke."]))
        _ = await ask(await newSession(plainModel), "Und wie geht es dir?")
        XCTAssertFalse((plainModel.calls.prompts(.speaker).first ?? "").contains("OFFENER SUCHAUFTRAG"))
        XCTAssertEqual(plainModel.calls.prompts(.speaker).count, 1)

        let model = RoleModel(speaker: Script(pieces: ["Mir geht es gut, danke."]))
        let seen = await askPending(await newSession(model))
        let prompt = model.calls.prompts(.speaker).first ?? ""
        XCTAssertTrue(prompt.contains("OFFENER SUCHAUFTRAG"))
        XCTAssertTrue(prompt.contains("Wer ist Frau Waldmeister im Zauberberg?"))
        XCTAssertEqual(seen.spoken, ["Mir geht es gut, danke."])
        XCTAssertEqual(seen.progress, [.speaker], "ohne `ABBRUCH` kein `abandon`")

        let history = [naturalTurn("Frage eins", "Antwort eins.", round: .general)]
        let overflow = RoleModel(speaker: Script(pieces: ["Mir geht es gut, danke."], overflows: 1))
        let seenOverflow = await askPending(await newSession(overflow, history: history))
        let prompts = overflow.calls.prompts(.speaker)
        XCTAssertEqual(prompts.count, 2)
        XCTAssertTrue((prompts.first ?? "").contains("VERLAUF:"))
        XCTAssertFalse((prompts.last ?? "VERLAUF:").contains("VERLAUF:"), "die Wiederholung kommt ohne Verlauf")
        XCTAssertTrue(!prompts.isEmpty && prompts.allSatisfy { $0.contains("OFFENER SUCHAUFTRAG") }, "der Block bleibt")
        XCTAssertEqual(seenOverflow.spoken, ["Mir geht es gut, danke."])

        let cloudModel = RoleModel(speaker: Script(pieces: ["Mir geht es gut, danke."]))
        _ = await askPending(await newSession(cloudModel, config: config(cloud: true)))
        XCTAssertEqual(cloudModel.calls.prompts(.speaker).count, 1)
        XCTAssertFalse((cloudModel.calls.prompts(.speaker).first ?? "").contains("OFFENER SUCHAUFTRAG"), "Cloud: im Zweifel weglassen")
        XCTAssertFalse((cloudModel.calls.prompts(.speaker).first ?? "").contains("Waldmeister"))
    }

    /// `ABBRUCH` (ganze Zeile am Zeilenanfang, auch klein, mit Markdown oder Punkt; oder in Großbuchstaben als letztes Wort der Zeile nach
    /// einem Satzende; auch über Stückgrenzen und am Stromende): `.abandon` genau einmal, die Zeile nie gesprochen, ein Satz davor ist
    /// eine normale direkte Antwort (`.conversational`, ungefiltert), keine Suche.
    func testAbandonLineSendsAbandonOnceAndIsNeverSpoken() async {
        let cases: [(pieces: [String], spoken: [String])] = [
            ([farewell + "\nABBRUCH\n"], [farewell]),
            ([farewell + "\nABBRUCH"], [farewell]),
            ([farewell + " ABBRUCH"], [farewell]),
            ([farewell + " ABBRUCH\n"], [farewell]),
            ([farewell + "\nabbruch\n"], [farewell]),
            ([farewell + "\n**ABBRUCH**\n"], [farewell]),
            ([farewell + "\n  ABBRUCH.  \n"], [farewell]),
            ([farewell + "\nAB", "BRU", "CH", "\n"], [farewell]),
            ([farewell + " A", "BBRUCH"], [farewell]),
            (["Alles klar, dann lassen wir das\nABBRUCH\n"], [farewell]),
            (["ABBRUCH\n"], []),
            (["ABBRUCH"], []),
            (["\n  \"Abbruch\"\n"], []),
            (["Nein, kein Problem. Ich sage nichts mehr dazu.\nABBRUCH\n"], ["Nein, kein Problem.", "Ich sage nichts mehr dazu."]),
        ]
        for (i, c) in cases.enumerated() {
            let model = RoleModel(speaker: Script(pieces: c.pieces), thinker: answer)
            let seen = await askPending(await newSession(model), id: "a\(i)")
            XCTAssertEqual(seen.progress, [.speaker, .abandon], "Fall \(i): \(c.pieces) -> \(seen.progressShape)")
            XCTAssertEqual(seen.spoken, c.spoken, "Fall \(i): \(c.pieces)")
            XCTAssertTrue(seen.statuses.allSatisfy { $0 == .conversational }, "Fall \(i): ein Satz davor ist eine normale direkte Antwort")
            XCTAssertFalse(seen.texts.joined().lowercased().contains("abbruch"), "Fall \(i): die Zeile wird nie ausgegeben: \(seen.texts)")
            XCTAssertEqual(model.calls.roles, [.speaker], "Fall \(i): keine Suche")
            XCTAssertNil(seen.failure, "Fall \(i): ein abgebrochener Auftrag ohne Satz ist kein Fehler")
            XCTAssertNotNil(seen.answer, "Fall \(i)")
            XCTAssertEqual(seen.answer?.counts.droppedTemplate, 0, "Fall \(i)")
        }
        // Hängt der Modellstrom nach der Zeile, endet der Lauf trotzdem sofort: der Strom wird nach `ABBRUCH` abgebrochen (keine Zeitüberschreitung).
        let hang = RoleModel(speaker: Script(pieces: [farewell + "\nABBRUCH\n"], hang: true), thinker: answer)
        let started = ContinuousClock.now
        let seenHang = await askPending(await newSession(hang), id: "hang")
        XCTAssertEqual(seenHang.progress, [.speaker, .abandon])
        XCTAssertEqual(seenHang.spoken, [farewell])
        XCTAssertNil(seenHang.failure)
        XCTAssertLessThan(ContinuousClock.now - started, .seconds(2), "der Strom wird nach der Zeile abgebrochen")
    }

    /// Keine Steuerzeile `ABBRUCH`: mit Komma, Doppelpunkt oder weiteren Buchstaben dahinter, klein geschrieben mitten in der Zeile, ohne
    /// Satzende davor, ein bloßes Anfangsstück am Stromende. Der Text bleibt Text; ein Satz nur aus „ABBRUCH“ wird nie gesprochen.
    func testAbandonNeedsAWholeLineOrAWholeLastWord() async {
        let notAbandon: [[String]] = [
            ["Das passt. ABBRUCH, bitte.\n"],
            ["Das passt gut.\nABBRUCH: ja.\n"],
            ["Abbruch bedeutet das Ende eines Vorgangs.\n"],
            ["Das passt gut. abbruch\n"],
            ["Das passt gut. ABBRUCHS ist ein Wort.\n"],
            ["Das passt gut, ABBRUCH\n"],
            ["Das passt gut.ABBRUCH\n"],
            ["Alles klar, ich lasse das sein.\nABBRU"],
            ["Alles klar, ich lasse das sein.\nABBRUCHX\n"],
        ]
        for (i, pieces) in notAbandon.enumerated() {
            let model = RoleModel(speaker: Script(pieces: pieces), thinker: answer)
            let seen = await askPending(await newSession(model), id: "n\(i)")
            XCTAssertEqual(seen.progress, [.speaker], "Fall \(i): \(pieces) -> \(seen.progressShape)")
            XCTAssertEqual(model.calls.roles, [.speaker], "Fall \(i)")
        }
        let model = RoleModel(speaker: Script(pieces: ["Das passt gut. ABBRUCH. Danke dir.\n"]))
        let seen = await askPending(await newSession(model))
        XCTAssertFalse(seen.texts.contains { $0.contains("ABBRUCH") }, "\(seen.texts)")
        XCTAssertEqual(seen.progress, [.speaker])
        XCTAssertTrue(AssistantNatural.isProtocolLine("ABBRUCH."))
        XCTAssertTrue(AssistantNatural.isProtocolLine("  **ABBRUCH** "))
        XCTAssertFalse(AssistantNatural.isProtocolLine("Abbruch."), "das gewöhnliche Wort bleibt ein Satz")
        XCTAssertFalse(AssistantNatural.isProtocolLine("Der ABBRUCH ist da."))
    }

    /// Ohne `pending` wird eine `ABBRUCH`-Zeile still verworfen: kein Ereignis, nie gesprochen; eine Antwort nur aus dieser Zeile endet mit dem
    /// festen Satz „nicht verstanden“, nicht mit einem Fehler und nicht stumm.
    func testAbandonWithoutAPendingSearchIsDroppedSilently() async {
        let model = RoleModel(speaker: Script(pieces: [farewell + "\nABBRUCH\n"]))
        let seen = await ask(await newSession(model))
        XCTAssertEqual(seen.progress, [.speaker], "kein `abandon` ohne offenen Suchauftrag")
        XCTAssertEqual(seen.spoken, [farewell])
        XCTAssertFalse(seen.texts.joined().contains("ABBRUCH"))
        XCTAssertEqual(seen.answer?.counts.droppedTemplate, 1)

        let only = await ask(await newSession(RoleModel(speaker: Script(pieces: ["ABBRUCH\n"]))), id: "r2")
        XCTAssertEqual(only.progress, [.speaker])
        XCTAssertEqual(only.statuses, [.neutral])
        XCTAssertEqual(only.texts, [AssistantNatural.notUnderstoodSentence])
    }

    /// `SUCHE:` und `ABBRUCH` zusammen: die erste Steuerzeile gilt, die zweite wird nicht mehr gelesen.
    func testTheFirstControlLineWinsWhenSearchAndAbandonCombine() async {
        let abandonFirst: [[String]] = [
            [farewell + "\nABBRUCH\nSUCHE: Davos Sanatorium Castorp\n"],
            [farewell + "\nABBRUCH\n", "SUCHE: Davos Sanatorium Castorp\n"],
            ["ABBRUCH\nSUCHE: Davos Sanatorium Castorp\n"],
        ]
        for (i, pieces) in abandonFirst.enumerated() {
            let model = RoleModel(speaker: Script(pieces: pieces), thinker: answer)
            let seen = await askPending(await newSession(model), id: "f\(i)")
            XCTAssertEqual(seen.progress, [.speaker, .abandon], "ABBRUCH zuerst, Fall \(i)")
            XCTAssertEqual(model.calls.roles, [.speaker], "keine Suche, Fall \(i)")
            XCTAssertFalse(seen.statuses.contains(.summarized), "Fall \(i)")
        }
        let searchFirst: [[String]] = [
            ["SUCHE: Davos Sanatorium Castorp\nABBRUCH\n"],
            ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\nABBRUCH\n"],
            ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\n", "ABBRUCH\n"],
        ]
        for (i, pieces) in searchFirst.enumerated() {
            let model = RoleModel(speaker: Script(pieces: pieces), thinker: answer)
            let seen = await askPending(await newSession(model), id: "g\(i)")
            XCTAssertFalse(seen.progress.contains(.abandon), "SUCHE zuerst, Fall \(i): \(seen.progressShape)")
            XCTAssertTrue(seen.progress.contains(.searching(query: "Davos Sanatorium Castorp")), "Fall \(i)")
            XCTAssertEqual(model.calls.roles, [.speaker, .thinker], "Fall \(i)")
            XCTAssertEqual(seen.statuses.suffix(2), [.summarized, .summarized], "Fall \(i)")
        }
    }

    /// Abbruch und Widerruf vor der Zeile: kein `abandon`, kein Satz mehr; ein zurückgehaltenes Anfangsstück („ABBRU“) wird weder
    /// ausgegeben noch als Zeile gewertet, wenn der Lauf endet.
    func testNoAbandonAfterCancelOrRevocationAndAPartialLineIsNeverOutput() async {
        for revoke in [false, true] {
            // Der Text des Sprechers kommt erst, nachdem Abbruch bzw. Widerruf angewendet sind (Haltepunkt statt Wartezeit).
            let applied = Latch()
            let model = RoleModel(speaker: Script(pieces: [farewell + "\nABBRUCH\n"]).holding(0, until: applied), thinker: answer)
            let s = await newSession(model)
            var later: [AssistantProgress] = []
            var sentencesLater = 0
            var stopped = false
            for await e in s.events(AssistantRequest(requestId: "r1", text: "Und wie geht es dir?", pending: openSearch)) {
                switch e {
                case .progress(_, _, let p):
                    if stopped {
                        later.append(p)
                    } else if p == .speaker {
                        stopped = true
                        if revoke { _ = s.applyReleaseChange(.other) } else { await s.cancel(requestId: "r1") }
                        applied.open()
                    }
                case .sentence:
                    if stopped { sentencesLater += 1 }
                default:
                    break
                }
            }
            XCTAssertTrue(stopped)
            XCTAssertEqual(later, [], "nach \(revoke ? "Widerruf" : "Abbruch") kein Fortschritt mehr")
            XCTAssertEqual(sentencesLater, 0)
        }

        // Der Strom hängt mitten in der Zeile („ABBRU“), dann Abbruch: nichts davon geht hinaus.
        let hang = RoleModel(speaker: Script(pieces: [farewell + "\nABBRU"], hang: true))
        let s = await newSession(hang)
        var out: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Und wie geht es dir?", pending: openSearch)) {
            out.append(e)
            if case .progress(_, _, .speaker) = e {
                // Haltepunkt: erst abbrechen, wenn das Double den Sprecher-Aufruf (mit dem hängenden Rest „ABBRU“) angenommen hat.
                Task {
                    await hang.calls.accepted(.speaker)
                    await s.cancel(requestId: "r1")
                }
            }
        }
        let seen = Seen(events: out)
        XCTAssertEqual(seen.failure, .cancelled)
        XCTAssertFalse(seen.progress.contains(.abandon))
        XCTAssertFalse(seen.texts.joined().contains("ABBRU"))
    }

    /// Verlangt der Sprecher mit `pending` eine Suche, fragt der Denker „<Frage des Auftrags> <neue Äußerung>“ (beide, in dieser Reihenfolge);
    /// der Abruf nutzt die neuen Suchwörter, ohne Suchwörter beide Fragetexte. Der Sprecher sieht nur die neue Äußerung als Äußerung.
    func testSearchWithAPendingSearchAsksTheThinkerWithBothQuestionsInOrder() async {
        let pending = AssistantPendingSearch(question: "Wohin fährt Hans Castorp?", query: "Castorp Reise")
        let both = "Wohin fährt Hans Castorp? Und wie lange bleibt er dort?"

        let model = RoleModel(speaker: Script(pieces: ["SUCHE: Davos Sanatorium\n"]), thinker: answer)
        let seen = await askPending(await newSession(model), "Und wie lange bleibt er dort?", pending: pending)
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
        XCTAssertTrue((model.calls.prompts(.thinker).first ?? "").hasSuffix("Aktuelle Äußerung des Nutzers:\n" + both), "Denker-Frage: beide, alte zuerst")
        XCTAssertTrue((model.calls.prompts(.speaker).first ?? "").hasSuffix("Aktuelle Äußerung des Nutzers:\nUnd wie lange bleibt er dort?"), "Sprecher: nur die neue Äußerung")
        XCTAssertEqual(seen.progress.dropFirst().first, .searching(query: "Davos Sanatorium"))
        XCTAssertEqual(seen.progress.first, .speaker)

        // Ohne Suchwörter: Abruf und Anzeige mit beiden Fragetexten. Die neue Äußerung allein findet nichts, mit der alten Frage gibt es Auszüge.
        let noWords = RoleModel(speaker: Script(pieces: ["SUCHE:\n"]), thinker: answer)
        let seenNoWords = await askPending(await newSession(noWords), "Und danach?", pending: pending, id: "r2")
        XCTAssertEqual(seenNoWords.statuses, [.summarized, .summarized], "die alte Frage trägt den Abruf")
        XCTAssertEqual(seenNoWords.progress.dropFirst().first, .searching(query: "Wohin fährt Hans Castorp? Und danach?"))
        XCTAssertTrue((noWords.calls.prompts(.thinker).first ?? "").contains("Sanatorium"))
        let control = RoleModel(speaker: Script(pieces: ["SUCHE:\n"]), thinker: answer)
        let seenControl = await ask(await newSession(control), "Und danach?", id: "r3")
        XCTAssertEqual(seenControl.statuses, [.neutral], "ohne den Auftrag findet die Äußerung allein nichts")
        XCTAssertEqual(control.calls.roles, [.speaker], "kein Denker")

        // Direkter Inhaltsbezug (erzwungene Suche): der Sprecher sieht nur die neue Äußerung, die Frage des Denkers hat beide Texte.
        let direct = RoleModel(thinker: answer)
        let seenDirect = await askPending(await newSession(direct), ownQuestion, pending: pending, id: "r4")
        XCTAssertEqual(direct.calls.roles, [.speaker, .thinker])
        XCTAssertTrue((direct.calls.prompts(.speaker).first ?? "").hasSuffix("Aktuelle Äußerung des Nutzers:\n" + ownQuestion))
        XCTAssertEqual(seenDirect.progress.first, .speaker)
        XCTAssertEqual(seenDirect.progress.dropFirst().first, .searching(query: "Wohin fährt Hans Castorp? " + ownQuestion))
        XCTAssertTrue((direct.calls.prompts(.thinker).first ?? "").hasSuffix("Aktuelle Äußerung des Nutzers:\nWohin fährt Hans Castorp? " + ownQuestion))
        XCTAssertEqual(seenDirect.statuses, [.summarized, .summarized])

        // Die Frage des Auftrags ist gekürzt, die neue Äußerung bleibt ganz.
        let longQuestion = AssistantPendingSearch(question: String(repeating: "lange Frage ", count: 100), query: "Davos")
        XCTAssertEqual(AssistantNatural.dataQuestion(text: "Neue Äußerung?", pending: nil), "Neue Äußerung?")
        let combined = AssistantNatural.dataQuestion(text: "Neue Äußerung?", pending: longQuestion)
        XCTAssertTrue(combined.hasSuffix(" Neue Äußerung?"))
        XCTAssertLessThanOrEqual(combined.count, AssistantPrompt.maxPendingQuestionChars + 1 + "Neue Äußerung?".count)
        XCTAssertEqual(AssistantNatural.dataQuestion(text: "Neue Äußerung?", pending: AssistantPendingSearch(question: "  \n ", query: "x")), "Neue Äußerung?")
    }

    /// Der Stil `verbatim` ignoriert `pending`: dieselben Sätze und derselbe Prompt wie ohne, kein Fortschritt, nichts vom Auftrag im Prompt.
    func testVerbatimIgnoresThePendingSearch() async {
        var verbatim = config()
        verbatim.style = .verbatim
        for text in [weather, castorpQuestion] {
            let withPending = RoleModel(thinker: Script(pieces: ["Das Wetter ist schön."]))
            let seenWith = await askPending(await newSession(withPending, config: verbatim), text)
            let without = RoleModel(thinker: Script(pieces: ["Das Wetter ist schön."]))
            let seenWithout = await ask(await newSession(without, config: verbatim), text)
            XCTAssertEqual(seenWith.statuses, seenWithout.statuses, text)
            XCTAssertEqual(seenWith.texts, seenWithout.texts, text)
            XCTAssertTrue(seenWith.progress.isEmpty, text)
            XCTAssertEqual(withPending.calls.prompts(.thinker), without.calls.prompts(.thinker), "derselbe Prompt: \(text)")
            for p in withPending.calls.prompts(.thinker) {
                XCTAssertFalse(p.contains("Waldmeister") || p.contains("OFFENER"), text)
            }
            XCTAssertFalse(withPending.calls.roles.isEmpty)
        }
    }

    /// Nichts aus `pending` kommt ins Protokoll oder in den Verlauf (nur in den Prompt): weder Frage noch Suchwörter noch das Wort
    /// `ABBRUCH`; die Zeilenfolge ist die ohne Auftrag.
    func testNothingOfThePendingSearchReachesTheJournalOrTheHistory() async {
        let scripts: [(Script, [String])] = [
            (Script(pieces: ["Ich sehe in deinem Buch nach.\nSUCHE: Davos Sanatorium Castorp\n"]),
             ["start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "round:data/naturalSearch", "areas:books", "step:summarized",
              "step:summarized", "end"]),
            (Script(pieces: [farewell + "\nABBRUCH\n"]), ["start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "end"]),
            (Script(pieces: ["SUCHE: Davos Sanatorium Castorp\n"]),
             ["start:natural", "round:general/naturalTalk", "areas:", "round:data/naturalSearch", "areas:books", "step:summarized", "step:summarized", "end"]),
        ]
        for (i, entry) in scripts.enumerated() {
            let (s, audit) = await auditSession(RoleModel(speaker: entry.0, thinker: answer))
            let seen = await askPending(s)
            await close(s, "r1", user: "Und wie geht es dir?")
            let raw = rawJournal(audit)
            for needle in ["Waldmeister", "Wer ist Frau", "OFFENER", "ABBRUCH", "abandon", "pending", "Zauberberg?"] {
                XCTAssertFalse(raw.contains(needle), "Fall \(i), im Protokoll: \(needle)")
            }
            XCTAssertEqual(journalTags(journal(audit)), entry.1, "Fall \(i)")
            XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [], "Fall \(i)")
            let turn = await s.history.last
            XCTAssertEqual(turn?.user, "Und wie geht es dir?", "Fall \(i): der Verlauf hat nur die neue Äußerung")
            let kept = ([turn?.user, turn?.assistant] + (turn?.parts.map(\.text) ?? [])).compactMap { $0 }.joined(separator: " ")
            XCTAssertFalse(kept.contains("Waldmeister") || kept.contains("Wer ist Frau") || kept.contains("ABBRUCH"), "Fall \(i), im Verlauf: \(kept)")
            XCTAssertFalse(seen.texts.joined().contains("Waldmeister"), "Fall \(i)")
        }
    }

    /// Zwei nebenläufige Anfragen stören sich nicht: A ist in der Datenphase (der Denker läuft), B ist eine Sprecher-Anfrage mit offenem
    /// Suchauftrag und endet mit `ABBRUCH`. Jede hat ihre eigenen Sätze, ihren eigenen Fortschritt und ihre eigene Zeilenfolge; `abandon` gehört
    /// nur zu B; B ist fertig, während A noch läuft.
    func testTwoConcurrentRequestsOneInTheDataPhaseOneSpeakerWithAPendingSearchDoNotDisturbEachOther() async {
        // Der Denker von A hält sein zweites Stück zurück, bis B fertig ist (Haltepunkt statt Wartezeit): „B ist fertig, während A läuft“
        // gilt dann auf jedem Rechner.
        let bIsDone = Latch()
        let model = PendingAwareModel(
            plainSpeaker: search,
            pendingSpeaker: Script(pieces: ["Mir geht es gut, danke.\nABBRUCH\n"]),
            thinker: Script(pieces: ["Castorp fährt ins Sanatorium nach Davos. ", "Dort besucht er seinen Vetter Joachim."])
                .holding(1, until: bIsDone))
        let (s, audit) = await auditSession(model)
        let composing = Latch()
        let finishedA = Box(false)
        let taskA = Task { () -> Seen in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "A", text: castorpQuestion)) {
                if case .progress(_, _, .composing) = e { composing.open() }
                out.append(e)
            }
            finishedA.value = true
            return Seen(events: out)
        }
        let aInDataPhase = await composing.wait()
        XCTAssertTrue(aInDataPhase, "A ist in der Datenphase")
        let pending = AssistantPendingSearch(question: castorpQuestion, query: "Davos Sanatorium Castorp")
        let seenB = await collect(s, AssistantRequest(requestId: "B", text: "Und wie geht es dir?", pending: pending))
        XCTAssertFalse(finishedA.value, "B ist fertig, während A noch läuft")
        bIsDone.open()
        let seenA = await taskA.value
        await s.commit(requestId: "A", user: castorpQuestion, spoken: .all, interrupted: false)
        await s.commit(requestId: "B", user: "Und wie geht es dir?", spoken: .all, interrupted: false)

        XCTAssertEqual(seenA.statuses, [.summarized, .summarized])
        XCTAssertEqual(seenA.texts, ["Castorp fährt ins Sanatorium nach Davos.", "Dort besucht er seinen Vetter Joachim."])
        XCTAssertFalse(seenA.progress.contains(.abandon), "`abandon` gehört nur zu B")
        XCTAssertEqual(seenA.answer?.round, .data)
        XCTAssertEqual(seenB.statuses, [.conversational])
        XCTAssertEqual(seenB.texts, ["Mir geht es gut, danke."])
        XCTAssertEqual(seenB.progress, [.speaker, .abandon])
        XCTAssertEqual(seenB.answer?.round, .general)

        XCTAssertEqual(journalTags(journal(audit, "A")), [
            "start:natural", "round:general/naturalTalk", "areas:", "round:data/naturalSearch", "areas:books", "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(journalTags(journal(audit, "B")), ["start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "end"])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit, "A")), [])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit, "B")), [])

        let speakerPrompts = model.calls.prompts(.speaker)
        XCTAssertEqual(speakerPrompts.count, 2)
        XCTAssertEqual(speakerPrompts.filter { $0.contains("OFFENER SUCHAUFTRAG") }.count, 1, "nur B hat den Block")
        let thinkerPrompt = model.calls.prompts(.thinker)
        XCTAssertEqual(thinkerPrompt.count, 1)
        XCTAssertFalse((thinkerPrompt.first ?? "").contains("OFFENER") || (thinkerPrompt.first ?? "").contains("Und wie geht es dir?"), "der Denker von A weiß nichts von B")
    }

    /// Das Tor erkennt `ABBRUCH` als ganze Zeile (Zeilenanfang, ohne Rücksicht auf Groß-/Kleinschreibung) und nach einem Satzende als letztes
    /// Wort der Zeile (nur in Großbuchstaben); es steht erst am Zeilenende (oder am Stromende) fest; danach ist das Tor still.
    func testSpeakerGateRecognizesTheAbandonLine() {
        typealias Gate = AssistantNatural.SpeakerGate
        typealias Step = Gate.Step

        var whole = Gate()
        XCTAssertEqual(whole.feed("Alles klar.\nABBRUCH\n"), Step(speech: "Alles klar.\n", abandon: true))
        XCTAssertEqual(whole.feed("SUCHE: Davos\n"), Step(), "nach ABBRUCH ist das Tor still: auch ein späteres SUCHE: zählt nicht")
        XCTAssertEqual(whole.finish(), Step())

        var held = Gate()
        XCTAssertEqual(held.feed("ABBRUCH"), Step(), "ohne Zeilenende steht es noch nicht fest")
        XCTAssertEqual(held.feed("  "), Step())
        XCTAssertEqual(held.finish(), Step(abandon: true), "das Stromende ist das Zeilenende")

        var markdown = Gate()
        XCTAssertEqual(markdown.feed("\"**Abbruch**.\"\n"), Step(abandon: true))

        var inside = Gate()
        XCTAssertEqual(inside.feed("Alles klar. ABBRUCH"), Step(speech: "Alles klar. "))
        XCTAssertEqual(inside.feed("!\n"), Step(abandon: true))

        var pieces = Gate()
        XCTAssertEqual(pieces.feed("Alles klar. A"), Step(speech: "Alles klar. "))
        XCTAssertEqual(pieces.feed("BBRU"), Step())
        XCTAssertEqual(pieces.feed("CH."), Step())
        XCTAssertEqual(pieces.feed("  \n"), Step(abandon: true))

        // Keine Zeile: alles geht als Text hinaus, auch das Zurückgehaltene; keine Suche, kein `abandon`.
        for text in ["ABBRUCH, ja\n", "ABBRUCH: ja\n", "ABBRUCHS\n", "Abbruch bedeutet das Ende\n", "Alles klar. abbruch\n", "Alles klar, ABBRUCH\n",
                     "Alles klar.ABBRUCH\n", "ABBRU", "Alles klar. ABBRUCHX", "ABB RUCH\n"] {
            var gate = Gate()
            var out = gate.feed(text)
            let rest = gate.finish()
            XCTAssertFalse(out.abandon || rest.abandon, text)
            XCTAssertNil(out.search, text)
            out.speech += rest.speech
            XCTAssertEqual(out.speech, text, text)
        }

        // Die erste Steuerzeile gilt.
        var abandonFirst = Gate()
        XCTAssertEqual(abandonFirst.feed("ABBRUCH\nSUCHE: Davos\n"), Step(abandon: true))
        var searchFirst = Gate()
        let step = searchFirst.feed("SUCHE: Davos\nABBRUCH\n")
        XCTAssertFalse(step.abandon)
        XCTAssertEqual(step.search, Gate.Search(words: " Davos\nABBRUCH\n", complete: true))
        XCTAssertEqual(AssistantNatural.searchWords(step.search?.words ?? ""), "Davos")
    }

    // MARK: Erzwungene Suche (direkter Inhaltsbezug: der Sprecher sagt höchstens einen Satz, danach laufen die Daten)

    /// Ein Satz, der den Filter passiert (konkret, keine Floskel). Das Folgewort dahinter gibt ihn frei: Ohne ein folgendes Zeichen
    /// behält der Satztrenner einen Satz noch zurück.
    private let lookSentence = "Ich sehe in deinem Buch nach."
    private let lookAndMore = "Ich sehe in deinem Buch nach. Dazu"

    private func forcedModel(_ pieces: [String], pauses: [UInt64] = [], hang: Bool = false, fail: (any Error)? = nil,
                              overflows: Int = 0, thinker: Script? = nil) -> RoleModel {
        RoleModel(speaker: Script(pieces: pieces, pausesMs: pauses, fail: fail, hang: hang, overflows: overflows), thinker: thinker ?? answer)
    }

    /// Reihenfolge: speaker, Zwischensatz, searching, found, composing; der Zwischensatz ist `.conversational` ohne Quellen, der Sprecher
    /// bekommt den Zusatz der erzwungenen Suche (nur er), danach läuft der Denker.
    func testForcedSearchRunsSpeakerSentenceSearchingFoundComposingInThatOrder() async {
        let model = forcedModel([lookAndMore])
        let seen = await ask(await newSession(model), ownQuestion)
        XCTAssertEqual(Array(seen.orderShape.prefix(5)), ["speaker", "sentence", "searching", "found:Der Zauberberg", "composing"], "\(seen.orderShape)")
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.texts.first, lookSentence)
        XCTAssertEqual(seen.sentences.first?.sources.count, 0, "der Zwischensatz hat keine Quellen")
        XCTAssertEqual(seen.progress.dropFirst().first, .searching(query: ownQuestion), "ohne Zeile SUCHE gilt der Fragetext")
        XCTAssertEqual(model.calls.roles, [.speaker, .thinker])
        XCTAssertEqual(seen.answer?.round, .data)
        XCTAssertEqual(seen.answer?.counts.conversational, 1)
        XCTAssertEqual(seen.answer?.counts.summarized, 2)
        XCTAssertEqual(seen.sourceEvents.first?.isEmpty, true, "die leere Vorab-Anzeige steht vor dem Sprecher")
        XCTAssertFalse(seen.sourceEvents.last?.isEmpty ?? true)

        let speakerPrompt = model.calls.prompts(.speaker).first ?? ""
        for needle in ["SUCHE LÄUFT BEREITS", "Sage NUR einen kurzen, konkreten Satz", "Beantworte die Frage nicht", "Keine Floskel"] {
            XCTAssertTrue(speakerPrompt.contains(needle), needle)
        }
        XCTAssertTrue(speakerPrompt.hasSuffix("Aktuelle Äußerung des Nutzers:\n" + ownQuestion))
        XCTAssertFalse((model.calls.prompts(.thinker).first ?? "").contains("SUCHE LÄUFT BEREITS"))

        // Ohne direkten Inhaltsbezug gibt es den Zusatz nicht.
        let plain = RoleModel(speaker: Script(pieces: ["Das Wetter kenne ich leider nicht."]))
        _ = await ask(await newSession(plain), weather)
        XCTAssertFalse((plain.calls.prompts(.speaker).first ?? "").contains("SUCHE LÄUFT BEREITS"))
    }

    /// Der Strom des Sprechers endet mit dem ersten Satz (nicht erst an der Frist) und wird abgebrochen.
    func testTheForcedSpeakerStreamEndsAtTheFirstSentenceNotAtTheDeadline() async {
        let model = forcedModel([lookAndMore], hang: true)
        let seen = await ask(await newSession(model, config: config(forcedMs: 30_000)), ownQuestion)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        let ended = await model.calls.ended(2)
        XCTAssertTrue(ended)
        XCTAssertEqual(model.calls.terminations, 2, "Sprecher- und Denkerstrom sind beendet")
    }

    /// Weitere Sätze des Sprechers werden verworfen (`droppedTemplate`), nur der erste geht hinaus; die Suche läuft danach.
    func testASecondSentenceOfTheForcedSpeakerIsDroppedAndOnlyTheFirstGoesOut() async {
        let model = forcedModel([lookSentence + " Ich finde bestimmt etwas Schönes. Und noch mehr Sätze. Mehr"])
        let (s, audit) = await auditSession(model)
        let seen = await ask(s, ownQuestion)
        await close(s, "r1", user: ownQuestion)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.texts.first, lookSentence)
        let joined = seen.texts.joined(separator: " ")
        XCTAssertFalse(joined.contains("bestimmt"))
        XCTAssertFalse(joined.contains("Sätze"))
        XCTAssertEqual(seen.answer?.counts.conversational, 1)
        XCTAssertEqual(journalTags(journal(audit)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "step:droppedTemplate", "step:droppedTemplate",
            "round:data/naturalSearch", "areas:books", "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
        let turn = await s.history.last
        XCTAssertFalse((turn?.assistant ?? "").contains("bestimmt"), "Verworfenes steht nicht im Verlauf")
    }

    /// Antwortet der Sprecher inhaltlich in mehreren Sätzen, geht nur der erste (gefilterte) Satz hinaus; danach läuft die Suche.
    func testASubstantiveForcedSpeakerAnswerYieldsOnlyItsFirstSentenceThenTheSearchRuns() async {
        let model = forcedModel(["Hier steht zunächst etwas über Davos. ", "Außerdem gibt es ein Kapitel über Schnee. ", "Zum Schluss kommt der Abschied. Ende"])
        let seen = await ask(await newSession(model), ownQuestion)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.texts.first, "Hier steht zunächst etwas über Davos.")
        let joined = seen.texts.joined(separator: " ")
        XCTAssertFalse(joined.contains("Schnee"))
        XCTAssertFalse(joined.contains("Abschied"))
        XCTAssertEqual(Array(seen.progressShape.prefix(4)), ["speaker", "searching", "found:Der Zauberberg", "composing"])
        XCTAssertEqual(model.calls.roles, [.speaker, .thinker])
    }

    /// Eine Floskel als erster Satz: kein Satz, die Suche läuft trotzdem; auch ein Satz DAHINTER geht nicht hinaus (höchstens ein Satz).
    func testAFillerSentenceOfTheForcedSpeakerGivesNoSentenceButTheSearchStillRuns() async {
        for first in ["Einen Moment.", "Ich denke kurz darüber nach.", "Ich schaue kurz nach.", "Ich suche."] {
            let model = forcedModel([first + " ", lookAndMore])
            let seen = await ask(await newSession(model), ownQuestion)
            XCTAssertEqual(seen.statuses, [.summarized, .summarized], first)
            XCTAssertFalse(seen.texts.contains(lookSentence), "\(first): der zweite Satz zählt nicht")
            XCTAssertEqual(Array(seen.progressShape.prefix(4)), ["speaker", "searching", "found:Der Zauberberg", "composing"], first)
            XCTAssertEqual(model.calls.roles, [.speaker, .thinker], first)
            XCTAssertEqual(seen.failure, nil, first)
        }
        // Eine Floskel ALLEIN, auch ohne Satzende-Nachfolger (Stromende gibt den Satz frei).
        let alone = forcedModel(["Einen Moment."])
        let seenAlone = await ask(await newSession(alone), ownQuestion)
        XCTAssertEqual(seenAlone.statuses, [.summarized, .summarized])
    }

    /// Das Protokollwort als Satz („ABBRUCH.“ mit Text dahinter, also keine Steuerzeile) wird wie jeder verworfene Satz behandelt: kein Satz,
    /// Suche läuft.
    func testAProtocolWordAsTheFirstSentenceOfTheForcedSpeakerIsDropped() async {
        let model = forcedModel(["ABBRUCH. " + lookAndMore])
        let seen = await ask(await newSession(model), ownQuestion)
        XCTAssertFalse(seen.texts.contains { $0.contains("ABBRUCH") })
        XCTAssertNil(seen.failure)
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
    }

    /// Hängt der Sprecher ohne Satz, beginnt die Suche nach `forcedSpeakerMs`: kein Satz, kein Fehler; ein halber Satz zählt nicht.
    func testAHangingForcedSpeakerEndsAfterTheDeadlineWithoutASentenceAndTheSearchRuns() async {
        for pieces in [[String](), ["Ich sehe in deinem"], ["SUCHE: Dav"]] {
            let model = forcedModel(pieces, hang: true)
            let started = ContinuousClock.now
            let seen = await ask(await newSession(model, config: config(forcedMs: 150)), ownQuestion)
            let took = ContinuousClock.now - started
            XCTAssertEqual(seen.statuses, [.summarized, .summarized], "\(pieces)")
            XCTAssertEqual(seen.progress.dropFirst().first, .searching(query: ownQuestion), "ein halbe Suchzeile gilt nicht: \(pieces)")
            XCTAssertEqual(model.calls.roles, [.speaker, .thinker], "\(pieces)")
            XCTAssertGreaterThanOrEqual(took, .milliseconds(140), "die Suche beginnt erst an der Frist: \(pieces)")
            XCTAssertNil(seen.failure, "\(pieces)")
            let ended = await model.calls.ended(2)
            XCTAssertTrue(ended, "\(pieces)")
            XCTAssertEqual(model.calls.terminations, 2, "der hängende Sprecherstrom wurde abgebrochen: \(pieces)")
        }
    }

    /// Ein vollständiger erster Satz ohne folgendes Zeichen und ein hängender Strom: An der Frist gilt der vollständige Satz.
    func testAFinishedSentenceWithoutALookaheadIsSpokenAtTheDeadline() async {
        let model = forcedModel([lookSentence], hang: true)
        let seen = await ask(await newSession(model, config: config(forcedMs: 150)), ownQuestion)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.texts.first, lookSentence)
        XCTAssertEqual(Array(seen.orderShape.prefix(3)), ["speaker", "sentence", "searching"])
    }

    /// Der Sprecher endet ohne Satzzeichen (Strom zu Ende): kein Satz aus dem Bruchstück, die Suche läuft; die Verwerfung ist gezählt.
    func testAnUnterminatedForcedSpeakerFragmentIsDroppedAsAFragment() async {
        let (s, audit) = await auditSession(forcedModel(["Ich sehe in deinem Buch"]))
        let seen = await ask(s, ownQuestion)
        await close(s, "r1", user: ownQuestion)
        XCTAssertEqual(seen.statuses, [.summarized, .summarized])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])
        XCTAssertTrue(journalTags(journal(audit)).contains("step:droppedFragment"), "\(journalTags(journal(audit)))")
    }

    /// Ein Fehler des Sprechers (sofort, nach dem ersten Satz, mitten im Satz, Überlauf des Kontexts) ist kein Fehler der Anfrage: die Suche
    /// läuft, ein schon vollständiger Satz bleibt, nach außen kommt kein Fehler; ein Überlauf wird nicht wiederholt.
    func testAForcedSpeakerErrorNeverReachesTheOutsideAndTheSearchRuns() async {
        let immediate = forcedModel([], fail: Crash())
        let seenImmediate = await ask(await newSession(immediate), ownQuestion)
        XCTAssertEqual(seenImmediate.statuses, [.summarized, .summarized], "Fehler sofort")
        XCTAssertNil(seenImmediate.failure)
        XCTAssertEqual(immediate.calls.roles, [.speaker, .thinker])

        let afterSentence = forcedModel([lookSentence + " Da"], fail: Crash())
        let seenAfter = await ask(await newSession(afterSentence), ownQuestion, id: "r2")
        XCTAssertEqual(seenAfter.statuses, [.conversational, .summarized, .summarized], "der vollständige Satz bleibt")
        XCTAssertNil(seenAfter.failure)

        let midSentence = forcedModel(["Ich sehe in deinem Bu"], fail: Crash())
        let seenMid = await ask(await newSession(midSentence), ownQuestion, id: "r3")
        XCTAssertEqual(seenMid.statuses, [.summarized, .summarized], "ein halber Satz geht nicht hinaus")
        XCTAssertNil(seenMid.failure)

        let overflow = forcedModel([lookSentence], overflows: 5)
        let seenOverflow = await ask(await newSession(overflow), ownQuestion, id: "r4")
        XCTAssertEqual(seenOverflow.statuses, [.summarized, .summarized])
        XCTAssertEqual(overflow.calls.count(.speaker), 1, "keine Wiederholung des Sprechers")
        XCTAssertNil(seenOverflow.failure)

        // Der Fehler gilt nur dem Sprecher: Scheitert dagegen der Denker, endet die Anfrage mit dem Fehler.
        let thinkerFails = forcedModel([lookAndMore], thinker: Script(fail: Crash()))
        let seenThinker = await ask(await newSession(thinkerFails), ownQuestion, id: "r5")
        XCTAssertEqual(seenThinker.failure, .modelFailed)
    }

    /// Zeile `SUCHE:` des erzwungenen Sprechers: Ihre Wörter ersetzen den Fragetext in Anzeige und Abruf, ein Satz davor geht hinaus.
    func testTheForcedSpeakersSearchLineWordsAreUsedAndASentenceBeforeItGoesOut() async {
        let withSentence = forcedModel([lookSentence + "\nSUCHE: Davos Sanatorium\n"])
        let seen = await ask(await newSession(withSentence), ownQuestion)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.progress.dropFirst().first, .searching(query: "Davos Sanatorium"))

        let onlyLine = forcedModel(["SUCHE: Sanatorium Davos Vetter\n"])
        let hardQuestion = "Wie heißt der Mann in meinem Notizbuch?"
        let seenLine = await ask(await newSession(onlyLine), hardQuestion, id: "r2")
        XCTAssertEqual(seenLine.progress.dropFirst().first, .searching(query: "Sanatorium Davos Vetter"))
        XCTAssertEqual(seenLine.statuses, [.summarized, .summarized], "der Abruf nutzt die Wörter")
        XCTAssertTrue((onlyLine.calls.prompts(.thinker).first ?? "").contains("Vetter"))
        let control = forcedModel([])
        let seenControl = await ask(await newSession(control), hardQuestion, id: "r2c")
        XCTAssertEqual(seenControl.statuses, [.neutral], "Kontrolle: die Frage allein trifft nichts")

        let emptyLine = forcedModel(["SUCHE:\n"])
        let seenEmpty = await ask(await newSession(emptyLine), ownQuestion, id: "r3")
        XCTAssertEqual(seenEmpty.progress.dropFirst().first, .searching(query: ownQuestion), "ohne Wörter der Fragetext")

        // Eine vollständige Zeile SUCHE beendet die Stufe sofort: Ein hängender Strom hält die Suche nicht bis zur Frist auf.
        let hangingAfterLine = forcedModel(["SUCHE: Davos Sanatorium\n"], hang: true)
        let seenHang = await ask(await newSession(hangingAfterLine, config: config(forcedMs: 30_000)), ownQuestion, id: "r4")
        XCTAssertEqual(seenHang.progress.dropFirst().first, .searching(query: "Davos Sanatorium"))
        XCTAssertEqual(seenHang.statuses, [.summarized, .summarized])
    }

    /// `ABBRUCH` hat im erzwungenen Modus keine Wirkung: kein `.abandon`, kein Satz daraus, die Suche läuft, auch mit offenem Suchauftrag.
    func testABBRUCHIsIgnoredByTheForcedSpeaker() async {
        for pending in [nil, openSearch] {
            for pieces in [["ABBRUCH\n"], [lookSentence + "\nABBRUCH\n"], ["Alles klar. ABBRUCH"]] {
                let model = forcedModel(pieces)
                let (s, audit) = await auditSession(model)
                let seen: Seen
                if let pending { seen = await askPending(s, ownQuestion, pending: pending) } else { seen = await ask(s, ownQuestion) }
                await close(s, "r1", user: ownQuestion)
                XCTAssertFalse(seen.progress.contains(.abandon), "\(pieces)")
                XCTAssertNil(seen.failure, "\(pieces)")
                XCTAssertEqual(seen.statuses.suffix(2), [.summarized, .summarized], "\(pieces)")
                XCTAssertFalse(seen.texts.joined().contains("ABBRUCH"), "\(pieces)")
                XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [], "\(pieces)")
                if pieces == ["ABBRUCH\n"] {
                    XCTAssertEqual(journalTags(journal(audit)).filter { $0.hasPrefix("step:dropped") }, [], "ABBRUCH allein erzeugt keine Verwerfung")
                }
            }
        }
    }

    /// Offener Suchauftrag und erzwungene Suche zusammen: beide Blöcke im Prompt des Sprechers (Auftrag zuerst), der Denker fragt mit beiden
    /// Fragetexten.
    func testThePendingBlockAndTheForcedBlockAreBothInTheSpeakerPrompt() async {
        let model = forcedModel([lookAndMore])
        let seen = await askPending(await newSession(model), ownQuestion, pending: openSearch)
        let prompt = model.calls.prompts(.speaker).first ?? ""
        let pendingAt = prompt.range(of: "OFFENER SUCHAUFTRAG")?.lowerBound
        let forcedAt = prompt.range(of: "SUCHE LÄUFT BEREITS")?.lowerBound
        XCTAssertNotNil(pendingAt)
        XCTAssertNotNil(forcedAt)
        if let pendingAt, let forcedAt { XCTAssertLessThan(pendingAt, forcedAt) }
        XCTAssertTrue(prompt.hasSuffix("Aktuelle Äußerung des Nutzers:\n" + ownQuestion))
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(seen.progress.dropFirst().first, .searching(query: openSearch.question + " " + ownQuestion))
    }

    /// Protokollfolge: Der Prüfer lässt die Folge der erzwungenen Suche zu, mit und ohne Zwischensatz, mit Überbrückung, ohne Treffer, nach
    /// der Frist; jedes Präfix (Prozessende an jeder Stelle) ist in Ordnung.
    func testTheForcedSearchProtocolIsAcceptedByTheVerifier() async {
        // a) mit Zwischensatz
        let (sa, aa) = await auditSession(forcedModel([lookAndMore]))
        _ = await ask(sa, ownQuestion)
        await close(sa, "r1", user: ownQuestion)
        let tagsA = journalTags(journal(aa))
        XCTAssertEqual(tagsA, [
            "start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "round:data/naturalSearch", "areas:books",
            "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(aa)), [])
        let lines = journal(aa)
        for k in 1...lines.count { XCTAssertEqual(AssistantProtocolOrder.check(Array(lines.prefix(k))), [], "Präfix \(k)") }

        // b) mit verworfener Floskel und Überbrückung
        let (sb, ab) = await auditSession(forcedModel(["Einen Moment. "]), config: config(bridge: true))
        _ = await ask(sb, ownQuestion)
        await close(sb, "r1", user: ownQuestion)
        XCTAssertEqual(journalTags(journal(ab)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:droppedTemplate", "round:data/naturalSearch", "step:announcement",
            "areas:books", "step:summarized", "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(ab)), [])

        // c) ohne Zwischensatz, Sprecher hängt bis zur Frist
        let (sc, ac) = await auditSession(forcedModel([], hang: true), config: config(forcedMs: 100))
        _ = await ask(sc, ownQuestion)
        await close(sc, "r1", user: ownQuestion)
        XCTAssertEqual(journalTags(journal(ac)), [
            "start:natural", "round:general/naturalTalk", "areas:", "round:data/naturalSearch", "areas:books", "step:summarized",
            "step:summarized", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(ac)), [])

        // d) ohne Treffer: Zwischensatz, dann der feste Satz; keine Bereichszeile der Auszüge
        let (sd, ad) = await auditSession(forcedModel([lookAndMore]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()))
        let seenD = await ask(sd, ownQuestion)
        await close(sd, "r1", user: ownQuestion)
        XCTAssertEqual(seenD.statuses, [.conversational, .neutral])
        XCTAssertEqual(journalTags(journal(ad)), [
            "start:natural", "round:general/naturalTalk", "areas:", "step:conversational", "round:data/naturalSearch", "step:template", "end",
        ])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(ad)), [])
    }

    /// Bestandsfragen bleiben ohne Modell und ohne Fortschritt: auch ein bereitstehender Sprecher läuft nicht.
    func testAnInventoryQuestionStaysWithoutAnyModelAndWithoutSpeakerProgress() async {
        let model = forcedModel([lookAndMore])
        let seen = await ask(await newSession(model), inventoryQuestion)
        XCTAssertTrue(model.calls.roles.isEmpty, "kein Modell")
        XCTAssertTrue(seen.progress.isEmpty, "kein Fortschritt")
        XCTAssertEqual(seen.statuses.first, .announcement)
        XCTAssertTrue(seen.statuses.dropFirst().allSatisfy { $0 == .inventory })
        XCTAssertEqual(seen.answer?.round, .data)
    }

    /// `verbatim` bleibt unverändert: kein Sprecher, kein Fortschritt auch bei direktem Inhaltsbezug.
    func testVerbatimNeverRunsTheForcedSpeaker() async {
        var verbatim = config()
        verbatim.style = .verbatim
        let model = forcedModel([lookAndMore])
        let seen = await ask(await newSession(model, config: verbatim), ownQuestion)
        XCTAssertEqual(model.calls.count(.speaker), 0)
        XCTAssertTrue(seen.progress.isEmpty)
        XCTAssertFalse(model.calls.prompts(.thinker).contains { $0.contains("SUCHE LÄUFT BEREITS") })
    }

    /// Cloud: Verlauf und Bereichszeile des Sprechers sind nach den Freigaben gefiltert wie beim normalen Sprecher; der Suchauftrag steht
    /// in der Cloud nicht im Prompt, der Zusatz der erzwungenen Suche schon. Ohne Einwilligung läuft gar kein Modell.
    func testForcedSearchInTheCloudFiltersTheHistoryAndOmitsThePendingBlock() async {
        let history = [
            naturalTurn("Geheim Besprechung", "Antwort A.", round: .data,
                        sources: [assistantPassage(.transcripts, itemId: "t1", title: "Besprechung", text: nordlicht)]),
            naturalTurn("Offen Buch", "Antwort B.", round: .data, sources: [assistantPassage(.books, itemId: "b1", title: "Buch", text: castorp)]),
        ]
        let model = forcedModel([lookAndMore])
        let audit = AssistantAuditLog(directory: tempDir())
        let s = await newSession(model, config: config(cloud: true), history: history, audit: audit, knowledge: await booksAndMeeting())
        _ = await s.setAreas([.books], cloudConsent: true)
        let seen = await askPending(s, ownQuestion, pending: openSearch)
        await close(s, "r1", user: ownQuestion)
        let prompt = model.calls.prompts(.speaker).first ?? ""
        XCTAssertFalse(prompt.contains("Geheim Besprechung"), "Gesprochenes aus einem gesperrten Bereich verlässt das Gerät nicht")
        XCTAssertTrue(prompt.contains("Offen Buch"))
        XCTAssertTrue(prompt.contains("SUCHE LÄUFT BEREITS"))
        XCTAssertFalse(prompt.contains("OFFENER SUCHAUFTRAG"))
        XCTAssertFalse(prompt.contains("Waldmeister"), "nichts vom Suchauftrag in der Cloud")
        XCTAssertEqual(journal(audit).filter { $0.t == .areas }.first?.areas, ["books"])
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized])
        XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [])

        let none = forcedModel([lookAndMore])
        let denied = await newSession(none, config: config(cloud: true), cloudConsent: false)
        let seenDenied = await ask(denied, ownQuestion)
        XCTAssertEqual(seenDenied.failure, .consentRequired)
        XCTAssertTrue(none.calls.roles.isEmpty)
    }

    /// Ansagen für eine langsame Antwort gelten weiter: Hängt der Sprecher über die erste Ansage hinaus, kommt sie vor der Suche; ein
    /// schon ausgegebener Zwischensatz macht sie überflüssig.
    func testFillersStillApplyWhileTheForcedSpeakerHangsButNotAfterItsSentence() async {
        let hanging = forcedModel([], hang: true)
        // Die Frist (1,5 s) ist weit hinter der ersten Ansage (40 ms), auch auf einem ausgelasteten Rechner.
        let s1 = await newSession(hanging, config: config(first: 40, second: 10_000, forcedMs: 1_500))
        let stream1 = s1.events(AssistantRequest(requestId: "r1", text: ownQuestion))
        s1.noteCommitted(requestId: "r1")
        var out1: [AssistantEvent] = []
        for await e in stream1 { out1.append(e) }
        let seen1 = Seen(events: out1)
        XCTAssertEqual(seen1.statuses, [.announcement, .summarized, .summarized])
        XCTAssertEqual(seen1.texts.first, AssistantNatural.fillerFirstSentence)
        XCTAssertEqual(Array(seen1.orderShape.prefix(3)), ["speaker", "sentence", "searching"])

        let talking = forcedModel([lookAndMore], thinker: Script(pieces: answer.pieces, pausesMs: [800]))
        let s2 = await newSession(talking, config: config(first: 500, second: 10_000))
        let stream2 = s2.events(AssistantRequest(requestId: "r1", text: ownQuestion))
        s2.noteCommitted(requestId: "r1")
        var out2: [AssistantEvent] = []
        for await e in stream2 { out2.append(e) }
        XCTAssertEqual(Seen(events: out2).statuses, [.conversational, .summarized, .summarized], "keine Ansage nach dem Zwischensatz")
    }

    /// Abbruch während des Sprechers der erzwungenen Suche: Ausgang `cancelled`, keine Suche, kein Denker, der Strom ist beendet.
    func testCancelWhileTheForcedSpeakerRunsStopsEverythingAndStartsNoSearch() async {
        let model = forcedModel([], hang: true)
        let s = await newSession(model, config: config(forcedMs: 30_000))
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: ownQuestion)) {
            events.append(e)
            if case .progress(_, _, .speaker) = e {
                // Haltepunkt: erst abbrechen, wenn das Double den Sprecher-Aufruf angenommen hat.
                Task {
                    await model.calls.accepted(.speaker)
                    await s.cancel(requestId: "r1")
                }
            }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.failure, .cancelled)
        XCTAssertEqual(model.calls.roles, [.speaker])
        XCTAssertFalse(seen.progressShape.contains("searching"))
        let ended = await model.calls.ended(1)
        XCTAssertTrue(ended)
        XCTAssertEqual(model.calls.terminations, 1)
    }

    /// Widerruf der Freigaben während des Sprechers der erzwungenen Suche: Ausgang `revoked`, keine Suche, kein Denker.
    func testRevocationWhileTheForcedSpeakerRunsEndsWithRevokedAndStartsNoSearch() async {
        let model = forcedModel([], hang: true)
        let s = await newSession(model, config: config(forcedMs: 30_000))
        var events: [AssistantEvent] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: ownQuestion)) {
            events.append(e)
            if case .progress(_, _, .speaker) = e {
                // Haltepunkt: erst widerrufen, wenn das Double den Sprecher-Aufruf angenommen hat.
                Task {
                    await model.calls.accepted(.speaker)
                    _ = s.applyReleaseChange(.other)
                }
            }
        }
        let seen = Seen(events: events)
        XCTAssertEqual(seen.failure, .revoked)
        XCTAssertEqual(model.calls.roles, [.speaker])
        XCTAssertFalse(seen.progressShape.contains("searching"))
    }

    /// Schreibfehler der Spur: Rundenzeile des Sprechers (kein Modell), Schrittzeile des Zwischensatzes (nichts geht hinaus, kein Abruf,
    /// kein Denker), Rundenzeile der Daten (kein Abruf, kein Denker).
    func testWriteFailuresInTheForcedSearchStopBeforeTheNextStep() async {
        let roundModel = forcedModel([lookAndMore])
        let seenRound = await failingRun("\"t\":\"round\"", model: roundModel, text: ownQuestion)
        XCTAssertEqual(seenRound.failure, .auditUnavailable)
        XCTAssertTrue(seenRound.sentences.isEmpty)
        XCTAssertTrue(roundModel.calls.roles.isEmpty, "kein Modell ohne Rundenzeile")

        let retrievals = Box(0)
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [("S. 12", castorp)]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [ProbeSource { retrievals.value += 1 }])

        let stepModel = forcedModel([lookAndMore])
        let seenStep = await failingRun("\"t\":\"step\"", model: stepModel, knowledge: knowledge, text: ownQuestion)
        XCTAssertEqual(seenStep.failure, .auditUnavailable)
        XCTAssertTrue(seenStep.sentences.isEmpty, "die Einordnung steht nicht in der Datei: der Satz geht nicht hinaus")
        XCTAssertEqual(retrievals.value, 0)
        XCTAssertEqual(stepModel.calls.roles, [.speaker], "kein Denker")

        let dataModel = forcedModel([lookAndMore])
        let seenData = await failingRun("naturalSearch", model: dataModel, knowledge: knowledge, text: ownQuestion)
        XCTAssertEqual(seenData.failure, .auditUnavailable)
        XCTAssertEqual(retrievals.value, 0)
        XCTAssertEqual(dataModel.calls.roles, [.speaker], "kein Denker")
        XCTAssertEqual(seenData.statuses, [.conversational], "der Zwischensatz ging vor der Rundenzeile der Daten hinaus")
    }

    /// Mehrere Anfragen gleichzeitig (mit Satz und mit hängendem Sprecher und Frist): Jede hat ihre eigene Folge und ihre eigenen Zeilen.
    func testConcurrentForcedRequestsKeepTheirOwnSentencesAndLines() async {
        let talkative = forcedModel([lookAndMore])
        let (s, audit) = await auditSession(talkative)
        let ids = (0..<8).map { "f\($0)" }
        let results = await withTaskGroup(of: (String, Seen).self) { group -> [(String, Seen)] in
            for id in ids { group.addTask { (id, await collect(s, AssistantRequest(requestId: id, text: ownQuestion))) } }
            var out: [(String, Seen)] = []
            for await r in group { out.append(r) }
            return out
        }
        XCTAssertEqual(results.count, 8)
        for (id, seen) in results {
            XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized], id)
            await close(s, id, user: ownQuestion)
        }
        for id in ids {
            XCTAssertEqual(AssistantProtocolOrder.check(journal(audit, id)), [], id)
            XCTAssertEqual(journalTags(journal(audit, id)).count, 9, id)
        }

        let hanging = forcedModel([], hang: true)
        let (h, hAudit) = await auditSession(hanging, config: config(forcedMs: 80))
        let hangResults = await withTaskGroup(of: (String, Seen).self) { group -> [(String, Seen)] in
            for id in ids { group.addTask { (id, await collect(h, AssistantRequest(requestId: id, text: ownQuestion))) } }
            var out: [(String, Seen)] = []
            for await r in group { out.append(r) }
            return out
        }
        for (id, seen) in hangResults {
            XCTAssertEqual(seen.statuses, [.summarized, .summarized], id)
            XCTAssertNil(seen.failure, id)
            await close(h, id, user: ownQuestion)
        }
        for id in ids {
            XCTAssertEqual(AssistantProtocolOrder.check(journal(hAudit, id)), [], id)
            XCTAssertEqual(journalTags(journal(hAudit, id)).count, 8, id)
        }
        let allEnded = await hanging.calls.ended(16)
        XCTAssertTrue(allEnded)
        XCTAssertEqual(hanging.calls.terminations, 16, "alle Sprecher- und Denkerströme sind beendet")
    }

    /// Antwort des Sprechers und Frist fallen (fast) zusammen: Beides ist zulässig (Satz oder kein Satz), nie ein Fehler, nie ein Satz
    /// zweimal, die Spur bleibt gültig.
    func testSpeakerAnswerAndDeadlineAtTheSameTimeNeverBreakTheRun() async {
        for round in 0..<25 {
            let model = forcedModel([lookAndMore], pauses: [15], hang: true)
            let (s, audit) = await auditSession(model, config: config(forcedMs: 15))
            let seen = await ask(s, ownQuestion)
            await close(s, "r1", user: ownQuestion)
            let ok = seen.statuses == [.conversational, .summarized, .summarized] || seen.statuses == [.summarized, .summarized]
            XCTAssertTrue(ok, "Runde \(round): \(seen.statuses)")
            XCTAssertNil(seen.failure, "Runde \(round)")
            XCTAssertEqual(seen.texts.filter { $0 == lookSentence }.count, seen.statuses.first == .conversational ? 1 : 0, "Runde \(round)")
            XCTAssertEqual(AssistantProtocolOrder.check(journal(audit)), [], "Runde \(round)")
        }
    }

    /// Eine Frist der Sprecher-Stufe darf den Denker nie beenden (ältere Stufen werden übersprungen, die Frist wird mit der Stufe abgebrochen):
    /// Der Denker braucht länger als die Frist.
    func testAStaleDeadlineOfTheForcedSpeakerNeverStopsTheThinker() async {
        let slowThinker = Script(pieces: answer.pieces, pausesMs: [0, 400])
        let model = forcedModel([lookAndMore], thinker: slowThinker)
        let seen = await ask(await newSession(model, config: config(forcedMs: 100)), ownQuestion)
        XCTAssertEqual(seen.statuses, [.conversational, .summarized, .summarized], "beide Sätze des langsamen Denkers kommen")
        XCTAssertNil(seen.failure)
        XCTAssertEqual(seen.answer?.counts.summarized, 2)
    }

    // MARK: Frist im Briefkasten (Baustein)

    /// Die Frist kommt genau einmal als Ereignis der Stufe; eine abgebrochene Stufe und eine negative Frist liefern nichts; nach `close` endet
    /// der Strom ohne Nachzügler.
    func testMailboxDeadlineArrivesOnceForItsStageAndNeverAfterCancelOrWithANegativeValue() async {
        // Stufe 2 hat 300 ms und wird gleich abgebrochen (reichlich Zeit auch auf einem ausgelasteten Rechner); danach wird bis 700 ms nach
        // dem Setzen gewartet, ob doch noch etwas kommt.
        let box = AssistantNaturalMailbox()
        let setAt = ContinuousClock.now
        box.deadline(stage: 1, ms: 20)
        box.deadline(stage: 2, ms: 300)
        box.cancel(stage: 2)
        box.deadline(stage: 3, ms: -1)
        var iterator = box.stream.makeAsyncIterator()
        let first = await iterator.next()
        guard case .deadline(let stage)? = first else { return XCTFail("keine Frist: \(String(describing: first))") }
        XCTAssertEqual(stage, 1)
        try? await Task.sleep(until: setAt.advanced(by: .milliseconds(700)), clock: .continuous)
        box.close()
        let rest = await iterator.next()
        XCTAssertNil(rest, "keine zweite Frist: Stufe 2 war abgebrochen, Stufe 3 ohne Frist")
    }

    /// Eine Frist, die nach `close` gesetzt wird, läuft nicht mehr an; eine offene Frist endet mit `close` ohne Ereignis.
    func testMailboxDeadlineSetAfterCloseOrPendingAtCloseNeverFires() async {
        let late = AssistantNaturalMailbox()
        late.close()
        late.deadline(stage: 1, ms: 0)
        var lateIterator = late.stream.makeAsyncIterator()
        let none = await lateIterator.next()
        XCTAssertNil(none)

        let pending = AssistantNaturalMailbox()
        let setAt = ContinuousClock.now
        pending.deadline(stage: 1, ms: 300)
        pending.close()
        try? await Task.sleep(until: setAt.advanced(by: .milliseconds(700)), clock: .continuous)
        var pendingIterator = pending.stream.makeAsyncIterator()
        let gone = await pendingIterator.next()
        XCTAssertNil(gone, "die Frist wurde mit `close` abgebrochen")
    }
}
