import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Schreibfolge 6, 7 und 9): Wiederanlauf nach einem Prozessende, Laufnummer und Standzeile.
// - Schreibfolge 6: Der Leser führt je Einheit und je Commit-Art einzeln „unbekannt“; der Neustart holt nichts nach, schreibt für alte
//   Einheiten keine Ergebniszeile, setzt keine Anfrage eines früheren Laufs fort, wiederholt keinen Audiostart und hängt nichts mehr
//   an den Verlauf.
// - Schreibfolge 7: Laufnummer = höchste Nummer im Protokoll plus eins, dauerhaft vor der ersten Anfrage; das Paar aus Lauf und
//   Generation wird nie wiederverwendet; die Verdichtung behält die höchste Nummer.
// - Schreibfolge 9: Die Standzeile trägt Laufnummer und wirksamen Stand der Klasse A und zeigt nur eine Abweichung vom zuletzt
//   protokollierten Stand.
// Nur synthetische Daten.

private struct QuietModel: BookTalkModel {
    var text: String
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let text = text
        return AsyncThrowingStream { c in
            c.yield(text)
            c.finish()
        }
    }
}

private struct DiskFull: Error {}

private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var on = false
    var value: Bool {
        get { lock.withLock { on } }
        set { lock.withLock { on = newValue } }
    }
}

final class AssistantRestartTests: XCTestCase {
    private var dir: URL!
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
    private let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-restart-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    // MARK: Hilfen

    private var protocolURL: URL { dir.appendingPathComponent(AssistantAuditLog.fileName) }

    /// Ein „neuer Prozess“: eigener Schreiber (nicht aus der Registry), der die Datei beim ersten Schreiben neu liest.
    private func process(in directory: URL? = nil, appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile)
        -> AssistantAuditLog {
        AssistantAuditLog(directory: directory ?? dir, reader: AssistantAuditWriter.readFile, appender: appender)
    }

    private func knowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", sections: [
            ("S. 7", "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    private func session(_ audit: AssistantAuditLog, store: AssistantReleaseStore? = nil) async -> AssistantSession {
        AssistantSession(model: QuietModel(text: "Quasselbert wandert nach Pappelhausen."), knowledge: await knowledge(), config: config, audit: audit,
                         released: true, releaseStore: store)
    }

    private func encode(_ line: AssistantJournalLine) -> Data {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return (try? e.encode(line)) ?? Data()
    }

    private func write(_ lines: [AssistantJournalLine], to directory: URL? = nil) throws {
        let target = directory ?? dir!
        try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
        var raw = Data()
        for l in lines {
            raw.append(encode(l))
            raw.append(10)
        }
        try raw.write(to: target.appendingPathComponent(AssistantAuditLog.fileName))
    }

    private func raw(_ directory: URL? = nil) -> [[String: Any]] {
        let url = (directory ?? dir!).appendingPathComponent(AssistantAuditLog.fileName)
        return ((try? String(contentsOf: url, encoding: .utf8)) ?? "").split(separator: "\n").compactMap {
            (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any]
        }
    }

    private func start(_ req: String, run: Int? = nil, at: Double = 0) -> AssistantJournalLine {
        var l = AssistantJournalLine(.start, req: req)
        l.at = Date(timeIntervalSince1970: 1_760_000_000 + at)
        l.session = "sitzung"
        l.model = .other
        l.cloud = false
        l.unsourced = false
        l.consent = 0
        l.ctx = .data
        l.reason = .locked
        l.round = .data
        l.gen = 1
        l.run = run
        return l
    }

    private func step(_ req: String, _ n: Int, _ verdict: AssistantTraceVerdict) -> AssistantJournalLine {
        var l = AssistantJournalLine(.step, req: req)
        l.n = n
        l.verdict = verdict
        l.gen = 1
        return l
    }

    private func commit(_ req: String, _ n: Int, _ stage: AssistantCommitStage, ok: Bool = true, failed: Bool = false) -> AssistantJournalLine {
        var l = AssistantJournalLine(.commit, req: req)
        l.n = n
        l.mark = AssistantCommitMark(stage: stage, ok: ok, generation: 1, failed: failed)
        return l
    }

    private func end(_ req: String) -> AssistantJournalLine {
        var l = AssistantJournalLine(.end, req: req)
        l.at = Date(timeIntervalSince1970: 1_760_000_100)
        l.outcome = .completed
        l.level = .s1
        l.areas = ["books"]
        return l
    }

    private func slowLog() -> AssistantAuditLog {
        process(appender: { url, data in
            Thread.sleep(forTimeInterval: 0.05)
            try AssistantAuditWriter.appendFile(url, data)
        })
    }

    /// Kopie der Protokolldatei in ein neues Verzeichnis: der Zustand, den ein Prozessende zu dieser Zeit hinterlassen hätte.
    private func crashCopy() throws -> URL {
        let copy = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-restart-copy-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: protocolURL, to: copy.appendingPathComponent(AssistantAuditLog.fileName))
        return copy
    }

    // MARK: Schreibfolge 6: je Einheit und je Commit-Art

    /// Der Leser führt jede Art einzeln: Anzeige ausgeführt und Audio ohne Zeile ist „Anzeige ausgeführt, Audio unbekannt“.
    func testEachCommitKindOfAUnitIsReadOnItsOwn() async throws {
        try write([
            start("a"),
            step("a", 1, .grounded), commit("a", 1, .emit), commit("a", 1, .display), commit("a", 1, .audio), commit("a", 1, .history), // vollständig
            step("a", 2, .grounded), commit("a", 2, .emit), commit("a", 2, .display),                                           // Prozessende nach Anzeige
            step("a", 3, .announcement), commit("a", 3, .emit),                                                                 // nach Übergabe
            step("a", 4, .grounded), commit("a", 4, .emit), commit("a", 4, .display, ok: false), // verworfen: Audio begann nie
            step("a", 5, .template), commit("a", 5, .emit), commit("a", 5, .display), commit("a", 5, .audio, ok: false, failed: true),
            step("a", 6, .inventory),                                                                                           // gar kein Ergebnis
            step("a", 7, .droppedNotSnapped),                                                                                   // nichts vorgesehen
        ])
        let read = await process().entries()
        let entry = try XCTUnwrap(read.first { $0.requestId == "a" })
        let trace = Dictionary(uniqueKeysWithValues: entry.trace.map { ($0.n, $0) })
        func results(_ n: Int) -> [AssistantTraceStep.UnitResult] { AssistantTraceStep.unitStages.map { trace[n]!.result(for: $0) } }
        XCTAssertEqual(AssistantTraceStep.unitStages, [.emit, .display, .audio, .history], "auch der Verlauf hat sein Ergebnis je Einheit")
        XCTAssertEqual(results(1), [.executed, .executed, .executed, .executed])
        XCTAssertEqual(results(2), [.executed, .executed, .unknown, .unknown], "Anzeige ausgeführt, Audio und Verlauf unbekannt")
        XCTAssertEqual(results(3), [.executed, .unknown, .unknown, .unknown])
        // Der Leser schließt nie über die Art hinweg (Runde 8, R7): Eine fehlende Zeile in einer nicht regulär beendeten Anfrage ist
        // „unbekannt“, auch hinter einer verworfenen Anzeige oder einem gescheiterten Audiostart (der Prozess kann dazwischen geendet sein).
        XCTAssertEqual(results(4), [.executed, .discarded, .unknown, .unknown], "keine Zeile zu Audio und Verlauf: unbekannt, nicht geschlossen")
        XCTAssertEqual(results(5), [.executed, .executed, .failed, .unknown])
        XCTAssertEqual(results(6), [.unknown, .unknown, .unknown, .unknown])
        XCTAssertEqual(results(7), [.notPlanned, .notPlanned, .notPlanned, .notPlanned])
        XCTAssertEqual(trace[2]?.unknownStages, [.audio, .history])
        XCTAssertFalse(trace[1]!.resultUnknown)
        XCTAssertTrue(trace[2]!.resultUnknown)
        XCTAssertFalse(trace[7]!.resultUnknown)
        XCTAssertEqual(entry.outcome, .incomplete, "kein `end`: die Anfrage gilt als unvollständig")
    }

    /// Echte Antwort, Prozessende NACH der Anzeige vor dem Audioergebnis: Die Datei zu diesem Zeitpunkt zeigt es so; der Neustart
    /// schreibt weder Ergebnisse noch Einordnungen nach, sondern nur das Schließen der Anfrage (`end`, unvollständig).
    func testTheRestartWritesNoResultLineForOldUnitsAndOnlyClosesTheRequest() async throws {
        let audit = slowLog()
        let s = await session(audit)
        var copy: URL?
        defer { if let copy { try? FileManager.default.removeItem(at: copy) } }
        var received = 0
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            guard case .sentence(_, let generation, _, _, _, _, _) = e else { continue }
            received += 1
            guard copy == nil else { continue }
            // Die Anzeige läuft; das Ergebnis der Übergabe an die Stimme (Audio) steht noch nicht in der Datei.
            XCTAssertTrue(s.commitOutput(requestId: "r1", index: received - 1, generation: generation, stage: .display) {}.ok)
            _ = await audit.flush(request: "r1")   // die Ergebniszeile der Anzeige steht dauerhaft
            copy = try crashCopy()
        }
        _ = await s.commit(requestId: "r1", user: "x", spoken: .nothing, interrupted: true)
        let before = raw(try XCTUnwrap(copy)).filter { $0["req"] as? String == "r1" }

        // Neuer Prozess auf der Kopie: öffnet, schließt die Anfrage, läuft weiter.
        let reopened = process(in: copy)
        let s2 = await session(reopened)
        for await _ in s2.events(AssistantRequest(requestId: "r2", text: "Wie hoch ist die Zugspitze?")) {}
        _ = await s2.commit(requestId: "r2", user: "x", spoken: .nothing, interrupted: true)
        let after = raw(copy).filter { $0["req"] as? String == "r1" }
        let added = Array(after.dropFirst(before.count))
        XCTAssertEqual(added.compactMap { $0["t"] as? String }, ["end"], "nur das Schließen: keine Ergebniszeile, keine Einordnung, keine Verlaufszeile")
        XCTAssertEqual(added.first?["outcome"] as? String, "incomplete")
        let reread = await reopened.entries()
        let entry = try XCTUnwrap(reread.first { $0.requestId == "r1" })
        XCTAssertEqual(entry.outcome, .incomplete)
        let unit = try XCTUnwrap(entry.trace.first)
        XCTAssertEqual(unit.result(for: .emit), .executed)
        XCTAssertEqual(unit.result(for: .display), .executed, "die Anzeige ist festgehalten")
        XCTAssertEqual(unit.result(for: .audio), .unknown, "das Audio ist unbekannt, nichts wurde nachgetragen")
        XCTAssertNil(entry.history, "auch der Verlauf wurde nicht nachträglich als ausgeführt geschrieben")
    }

    /// Kein Weg setzt eine Anfrage eines früheren Prozesslaufs fort: weder Ausgabe-Commit noch Audiostart noch Verlauf noch Ergebnis.
    func testNoPathResumesARequestOfAnEarlierProcessRunRepeatsAudioOrAppendsHistory() async throws {
        try write([
            start("alt", run: 1), step("alt", 1, .grounded), commit("alt", 1, .emit), commit("alt", 1, .display),
        ])
        let before = raw().count
        let audit = process()
        let s = await session(audit)
        let g = s.releaseGeneration
        var displayRan = false
        var audioRan = false
        let display = s.commitOutput(requestId: "alt", index: 1, generation: g, stage: .display) { displayRan = true }
        let audioStart = s.commitStart(requestId: "alt", index: 1, generation: g) { audioRan = true; return true }
        s.noteCommit(requestId: "alt", index: 1, stage: .audio, ok: true, generation: g)
        let wrote = await s.commit(requestId: "alt", user: "Frage", spoken: .all, interrupted: false)
        XCTAssertFalse(display.ok)
        XCTAssertFalse(audioStart.ok)
        XCTAssertFalse(displayRan, "kein Auslösen der Anzeige für eine Anfrage eines früheren Laufs")
        XCTAssertFalse(audioRan, "kein Audiostart wiederholt")
        XCTAssertTrue(wrote)
        let history = await s.history
        XCTAssertTrue(history.isEmpty, "nichts wird später an den Verlauf gehängt")
        _ = await audit.flushReleaseLines()
        let after = raw().filter { $0["req"] as? String == "alt" }
        XCTAssertEqual(after.count, 4 + 1, "die vier alten Zeilen und das Schließen beim Öffnen; sonst nichts")
        XCTAssertEqual(after.last?["t"] as? String, "end")
        XCTAssertGreaterThan(raw().count, before)   // `state` des neuen Laufs
    }

    // MARK: Schreibfolge 7: Laufnummer

    /// Zwei Prozessläufe mit je Generation 1: im Leser unterscheidbar (Lauf 1 und Lauf 2), das Paar wird nie wiederverwendet.
    func testTwoProcessRunsWithGenerationOneAreToldApartByTheReader() async throws {
        let first = process()
        let sA = await session(first)
        let a = await sA.releaseDidChange()
        XCTAssertEqual(a.generation, 1)
        _ = await first.flushReleaseLines()
        let second = process()
        let sB = await session(second)
        let b = await sB.releaseDidChange()
        XCTAssertEqual(b.generation, 1, "ein neuer Prozesslauf beginnt wieder bei Generation 1")
        _ = await second.flushReleaseLines()
        let entries = await process().entries()
        let releases = entries.filter { $0.kind == .release }
        XCTAssertEqual(releases.map(\.generation), [1, 1])
        XCTAssertEqual(releases.map(\.run), [1, 2], "unterscheidbar nach dem Lauf")
        let pairs = releases.map { "\($0.run ?? 0)/\($0.generation)" }
        XCTAssertEqual(Set(pairs).count, pairs.count, "das Paar aus Lauf und Generation ist eindeutig")
        XCTAssertEqual(entries.filter { $0.kind == .state }.map(\.run), [1, 2])
    }

    /// Die Nummer ist die höchste im Protokoll plus eins und steht in der Datei, bevor die erste Anfrage beginnt.
    func testTheRunNumberIsTheHighestPlusOneAndDurableBeforeTheFirstRequest() async throws {
        try write([start("alt", run: 5), end("alt"), start("alt2", run: 3), end("alt2")])
        let audit = process()
        let s = await session(audit)
        XCTAssertEqual(raw().compactMap { $0["run"] as? Int }.max(), 5, "vor der ersten Anfrage ist noch nichts geschrieben")
        let seen = Flag()
        var firstEvent = true
        for await _ in s.events(AssistantRequest(requestId: "n1", text: question)) {
            if firstEvent {
                firstEvent = false
                let kinds = raw().compactMap { $0["t"] as? String }
                seen.value = kinds.contains("state") && kinds.contains("start")
            }
        }
        _ = await s.commit(requestId: "n1", user: "x", spoken: .nothing, interrupted: true)
        XCTAssertTrue(seen.value, "Standzeile und `start` stehen dauerhaft, bevor das erste Ereignis hinausgeht")
        let mine = raw().filter { ($0["t"] as? String == "state") || ($0["t"] as? String == "start" && $0["req"] as? String == "n1") }
        XCTAssertEqual(mine.compactMap { $0["run"] as? Int }, [6, 6], "höchste Nummer plus eins, auf Standzeile und `start`")
    }

    /// Lässt sich die Standzeile mit der Laufnummer nicht schreiben, nimmt die Sitzung keine Anfrage an („Protokoll nicht schreibbar“,
    /// strenger als nur gesperrte eigene Inhalte): kein Modell, keine Ausgabe. Dieselbe Nummer gilt für den nächsten Versuch.
    func testARunNumberThatCannotBeWrittenRefusesEveryRequestAndTheNextAttemptReusesTheNumber() async throws {
        try write([start("alt", run: 4), end("alt")])
        let broken = Flag()
        broken.value = true
        let audit = process(appender: { url, data in
            if broken.value { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = await session(audit)
        var failure: AssistantFailure?
        var sentences = 0
        for await e in s.events(AssistantRequest(requestId: "x1", text: question)) {
            if case .failed(_, let f) = e { failure = f }
            if case .sentence = e { sentences += 1 }
        }
        XCTAssertEqual(failure, .auditUnavailable)
        XCTAssertEqual(sentences, 0, "keine Ausgabe ohne dauerhafte Laufnummer")
        _ = await s.commit(requestId: "x1", user: "x", spoken: .nothing, interrupted: true)
        broken.value = false
        for await _ in s.events(AssistantRequest(requestId: "x2", text: question)) {}
        _ = await s.commit(requestId: "x2", user: "x", spoken: .nothing, interrupted: true)
        let runs = raw().compactMap { $0["run"] as? Int }
        XCTAssertEqual(runs.max(), 5, "die Nummer nach dem Fehlschlag ist die, die vergeben wurde: höchste plus eins, kein Sprung")
        XCTAssertEqual(Set(runs.filter { $0 > 4 }), [5])
    }

    /// Die Verdichtung (mehr Einheiten als erlaubt) behält die höchste Laufnummer, auch wenn ihre Einheit die älteste ist; die Zählung geht weiter.
    func testCompactionKeepsTheHighestRunNumberSoCountingContinues() async throws {
        var lines: [AssistantJournalLine] = []
        var oldState = AssistantJournalLine(.state)
        oldState.at = Date(timeIntervalSince1970: 1_760_000_000)
        oldState.run = 7
        oldState.gen = 0
        oldState.areas = ["books"]
        lines.append(oldState)
        for i in 0..<(AssistantAuditLog.maxEntries + 60) {
            lines.append(start("q\(i)", at: Double(i)))
            lines.append(end("q\(i)"))
        }
        try write(lines)
        let audit = process()
        let s = await session(audit)
        _ = await s.releaseDidChange()
        _ = await audit.flushReleaseLines()
        let runs = raw().compactMap { $0["run"] as? Int }
        XCTAssertTrue(runs.contains(7), "die Einheit mit der höchsten Nummer bleibt erhalten")
        XCTAssertEqual(runs.max(), 8, "die Zählung geht nach der Verdichtung weiter (7 + 1), keine Wiederverwendung")
        XCTAssertLessThan(raw().filter { $0["t"] as? String == "start" }.count, AssistantAuditLog.maxEntries + 60, "es wurde tatsächlich verdichtet")
        let third = process()
        let s3 = await session(third)
        _ = await s3.releaseDidChange()
        _ = await third.flushReleaseLines()
        XCTAssertEqual(raw().compactMap { $0["run"] as? Int }.max(), 9)
    }

    /// Vorwärts mit Altdaten: Ein Protokoll aus einer Fassung ohne Laufnummern (alte Zeilen `release`, `state`, Anfragen) bleibt lesbar
    /// und Byte für Byte erhalten; der erste Lauf danach bekommt die Nummer 1, der nächste 2.
    func testAnOldProtocolWithoutRunNumbersIsKeptAndTheNumberingStartsAtOne() async throws {
        var oldState = AssistantJournalLine(.state)
        oldState.at = Date(timeIntervalSince1970: 1_760_000_000)
        oldState.gen = 0
        oldState.unsourced = false
        oldState.areas = ["books", "files", "notes", "transcripts"]
        var oldRelease = AssistantJournalLine(.release)
        oldRelease.at = Date(timeIntervalSince1970: 1_760_000_010)
        oldRelease.gen = 1
        oldRelease.cause = .areas
        oldRelease.areas = ["books"]
        try write([oldState, oldRelease, start("alt"), step("alt", 1, .grounded), commit("alt", 1, .emit), end("alt")])
        let original = try Data(contentsOf: protocolURL)
        let first = process()
        let s = await session(first)
        _ = await s.releaseDidChange()
        _ = await first.flushReleaseLines()
        let bytes = try Data(contentsOf: protocolURL)
        XCTAssertEqual(bytes.prefix(original.count), original, "die alten Zeilen bleiben Byte für Byte")
        let entries = await first.entries()
        XCTAssertEqual(entries.filter { $0.kind == .release }.map(\.run), [nil, 1], "die alte Zeile hat keinen Lauf, die neue die Nummer 1")
        XCTAssertEqual(entries.filter { $0.kind == .release }.map(\.generation), [1, 1])
        let second = process()
        let s2 = await session(second)
        _ = await s2.releaseDidChange()
        _ = await second.flushReleaseLines()
        XCTAssertEqual(raw().compactMap { $0["run"] as? Int }.max(), 2)
    }

    // MARK: Schreibfolge 9: Standzeile

    /// Die Standzeile trägt Laufnummer und wirksamen Stand und zeigt nur die Abweichung vom zuletzt protokollierten Stand.
    func testTheStateLineCarriesTheRunAndShowsOnlyADeviationFromTheLastLoggedState() async throws {
        let store = AssistantReleaseStore(directory: dir, protocolURL: protocolURL)
        let first = process()
        let s1 = await session(first, store: store)
        for await _ in s1.events(AssistantRequest(requestId: "a", text: question)) {}
        _ = await s1.commit(requestId: "a", user: "x", spoken: .nothing, interrupted: true)
        _ = await s1.setAreas([.books])   // Zeile `release`: der zuletzt protokollierte Stand
        _ = await first.flushReleaseLines()

        // Lauf 2: Die Datei trägt genau den zuletzt protokollierten Stand: keine Abweichung.
        let second = process()
        let s2 = await session(second, store: AssistantReleaseStore(directory: dir, protocolURL: protocolURL))
        for await _ in s2.events(AssistantRequest(requestId: "b", text: question)) {}
        _ = await s2.commit(requestId: "b", user: "x", spoken: .nothing, interrupted: true)

        // Lauf 3: Eine Einschränkung kam in der Datei an, ihre Zeile ging verloren (voller Datenträger): Abweichung erkennbar.
        let broken = Flag()
        let third = process(appender: { url, data in
            if broken.value, String(decoding: data, as: UTF8.self).contains("\"t\":\"release\"") { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s3 = await session(third, store: AssistantReleaseStore(directory: dir, protocolURL: protocolURL))
        for await _ in s3.events(AssistantRequest(requestId: "c", text: question)) {}
        _ = await s3.commit(requestId: "c", user: "x", spoken: .nothing, interrupted: true)
        broken.value = true
        let lost = await s3.setAreas([])   // reine Einschränkung: Datei und Prozess gelten, die Zeile `release` scheitert
        XCTAssertNotNil(lost.error, "die Zeile ging verloren")
        XCTAssertEqual(try XCTUnwrap(AssistantReleaseFile.decode(Data(contentsOf: dir.appendingPathComponent(AssistantReleaseStore.fileName)))).record.localAreas, [])

        // Lauf 4: Der Stand der Datei weicht vom zuletzt protokollierten ab.
        let fourth = process()
        let s4 = await session(fourth, store: AssistantReleaseStore(directory: dir, protocolURL: protocolURL))
        for await _ in s4.events(AssistantRequest(requestId: "d", text: question)) {}
        _ = await s4.commit(requestId: "d", user: "x", spoken: .nothing, interrupted: true)

        let states = await process().entries().filter { $0.kind == .state }
        XCTAssertEqual(states.map(\.run), [1, 2, 3, 4], "jede Standzeile trägt die Laufnummer")
        XCTAssertEqual(states.map(\.deviatesFromLogged), [nil, false, false, true], "nur die Abweichung wird erkennbar, nichts wird rekonstruiert")
        XCTAssertEqual(states[3].areas, [], "der wirksame Stand steht in der Zeile")
        XCTAssertEqual(states[0].cloudConsent, false, "Cloud-Zustimmung als Ja oder Nein")
        XCTAssertEqual(states[0].areas, ["books", "files", "notes", "transcripts"])
    }

    /// Eine unlesbare Zustandsdatei steht als „gesperrt“ in der Standzeile.
    func testTheStateLineSaysLockedWhenTheStateFileIsUnreadable() async throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("kaputt".utf8).write(to: dir.appendingPathComponent(AssistantReleaseStore.fileName))
        let audit = process()
        let s = await session(audit, store: AssistantReleaseStore(directory: dir, protocolURL: protocolURL))
        for await _ in s.events(AssistantRequest(requestId: "a", text: question)) {}
        _ = await s.commit(requestId: "a", user: "x", spoken: .nothing, interrupted: true)
        let all = await process().entries()
        let state = try XCTUnwrap(all.first { $0.kind == .state })
        XCTAssertEqual(state.locked, true)
        XCTAssertEqual(state.areas, [])
        XCTAssertEqual(state.cloudConsent, false)
        XCTAssertEqual(state.run, 1)
    }
}
