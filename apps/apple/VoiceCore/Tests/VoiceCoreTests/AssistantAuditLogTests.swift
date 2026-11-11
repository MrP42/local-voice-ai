import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Protokoll ohne Inhalte als fortgeschriebenes Journal (`protokoll.jsonl`).
// Nur Verzeichnisse unter dem Temp-Ordner des Tests. Geprüft werden das Zeilenformat, die Reihenfolge (`start` vor allem
// anderen), das Verdichten und Schließen beim Öffnen, die Fehlerfälle (Absturz, voller Datenträger, Lesefehler, Abbruch beim
// Umschreiben, gleichzeitige Schreiber) und das Zusammenspiel mit der Sitzung.

private struct AuditModel: BookTalkModel {
    var pieces: [String]
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let pieces = pieces
        return AsyncThrowingStream { c in
            for p in pieces { c.yield(p) }
            c.finish()
        }
    }
}

/// Liefert die Stücke und wartet dann auf den Abbruch (hängendes Modell).
private struct HangingModel: BookTalkModel {
    var pieces: [String]
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let pieces = pieces
        return AsyncThrowingStream { c in
            let task = Task {
                for p in pieces { c.yield(p) }
                while !Task.isCancelled { try? await Task.sleep(nanoseconds: 10_000_000) }
            }
            c.onTermination = { _ in task.cancel() }
        }
    }
}

private final class Snapshots: @unchecked Sendable {
    private let lock = NSLock()
    private var texts: [String] = []
    func note(_ s: String) { lock.lock(); texts.append(s); lock.unlock() }
    var all: [String] { lock.lock(); defer { lock.unlock() }; return texts }
}

/// Liest beim Aufruf die Protokolldatei: Zu diesem Zeitpunkt muss `start` schon darin stehen.
private struct ProbeModel: BookTalkModel {
    var url: URL
    var snapshots: Snapshots
    var pieces: [String]
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        snapshots.note((try? String(contentsOf: url, encoding: .utf8)) ?? "")
        let pieces = pieces
        return AsyncThrowingStream { c in
            for p in pieces { c.yield(p) }
            c.finish()
        }
    }
}

private struct DiskFull: Error {}

/// Schalter „Platte kaputt“ (wechselbar zur Laufzeit).
/// Wartet synchron auf ein Signal (in `async`-Tests darf ein Semaphor nicht direkt gewartet werden; hier geht es um echte Threads).
private func awaited(_ semaphore: DispatchSemaphore, seconds: TimeInterval) -> Bool {
    semaphore.wait(timeout: .now() + seconds) == .success
}

/// Wartet (begrenzt) auf einen Zustand, der von einem Rückruf gesetzt wird (z. B. das Ende eines Modellstroms), statt eine Zeit zu
/// raten. Der Test prüft ihn danach selbst; erst nach `timeout` Sekunden gilt er als nicht eingetreten.
private func eventually(timeout: TimeInterval = 5, _ condition: () -> Bool) async {
    let deadline = Date().addingTimeInterval(timeout)
    while !condition(), Date() < deadline { try? await Task.sleep(nanoseconds: 5_000_000) }
}

private final class FailSwitch: @unchecked Sendable {
    private let lock = NSLock()
    private var flag = false
    var on: Bool {
        get { lock.lock(); defer { lock.unlock() }; return flag }
        set { lock.lock(); flag = newValue; lock.unlock() }
    }
}

/// Zähler für Fehler, die nur beim n-ten Aufruf auftreten.
private final class Flaky: @unchecked Sendable {
    private let lock = NSLock()
    private var remaining: Int
    init(failures: Int) { remaining = failures }
    /// `true`, solange noch Fehler übrig sind (verbraucht einen).
    func fail() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard remaining > 0 else { return false }
        remaining -= 1
        return true
    }
}

// MARK: Zeilen

private func journalStart(_ req: String, at: Double = 0, round: AssistantRoundKind? = .data) -> AssistantJournalLine {
    var l = AssistantJournalLine(.start, req: req)
    l.at = Date(timeIntervalSince1970: 1_760_000_000 + at)
    l.session = "sitzung"
    l.model = .other
    l.cloud = false
    l.unsourced = false
    l.consent = 1
    l.ctx = .data
    l.reason = .locked
    l.round = round
    l.gen = 1
    return l
}

private func journalStep(_ req: String, _ n: Int, _ verdict: AssistantTraceVerdict = .grounded) -> AssistantJournalLine {
    var l = AssistantJournalLine(.step, req: req)
    l.n = n
    l.verdict = verdict
    l.gen = 1
    return l
}

private func journalCommit(_ req: String, _ n: Int, _ mark: AssistantCommitMark) -> AssistantJournalLine {
    var l = AssistantJournalLine(.commit, req: req)
    l.n = n
    l.mark = mark
    return l
}

private func journalEnd(_ req: String, _ outcome: AssistantAuditEntry.Outcome = .completed, areas: [String] = []) -> AssistantJournalLine {
    var l = AssistantJournalLine(.end, req: req)
    l.at = Date(timeIntervalSince1970: 1_760_000_100)
    l.outcome = outcome
    l.level = .s1
    l.areas = areas
    return l
}

private func encoded(_ l: AssistantJournalLine) -> Data {
    let e = JSONEncoder()
    e.dateEncodingStrategy = .iso8601
    e.outputFormatting = [.sortedKeys]
    return (try? e.encode(l)) ?? Data()
}

private func journalEvent(_ n: Int, kind: AssistantAuditEntry.Kind = .unsourcedOn) -> AssistantAuditEntry {
    AssistantAuditEntry(at: Date(timeIntervalSince1970: 1_760_000_000 + Double(n)), kind: kind, model: "Test", cloud: n % 2 == 0,
                        unsourcedMode: n % 3 == 0, consentVersion: 1, generation: n + 1)
}

final class AssistantAuditLogTests: XCTestCase {
    private var dir: URL!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    private var fileURL: URL { dir.appendingPathComponent(AssistantAuditLog.fileName) }

    private func writeRaw(_ data: Data) throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try data.write(to: fileURL)
    }

    private func writeLines(_ lines: [AssistantJournalLine]) throws {
        var raw = Data()
        for l in lines {
            raw.append(encoded(l))
            raw.append(10)
        }
        try writeRaw(raw)
    }

    /// Eigener Schreiber (nicht aus der Registry) auf dem Testverzeichnis, mit echten Dateizugriffen und Austauschpunkten.
    private func freshLog(appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile,
                          rewriter: (@Sendable (URL, Data) throws -> Void)? = nil) -> AssistantAuditLog {
        AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile, appender: appender, rewriter: rewriter)
    }

    private func objects(_ url: URL? = nil) throws -> [[String: Any]] {
        let raw = try String(contentsOf: url ?? fileURL, encoding: .utf8)
        return try raw.split(separator: "\n", omittingEmptySubsequences: true).map {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any])
        }
    }

    private func rawLog() -> String { (try? String(contentsOf: fileURL, encoding: .utf8)) ?? "" }

    // MARK: Format und Faltung

    /// Start, Schritte, Commits und Ende einer Anfrage werden zu einem Eintrag gefaltet; jedes Feld kommt zurück.
    func testJournalLinesFoldToOneEntryWithEveryField() async {
        let log = AssistantAuditLog(directory: dir)
        var s = journalStart("r1", round: .general)
        s.cloud = true
        s.unsourced = true
        s.consent = 3
        s.ctx = .free
        s.reason = .free
        s.gen = 7
        let began = await log.begin(s)
        XCTAssertTrue(began)
        log.enqueue(journalStep("r1", 1, .announcement))
        log.enqueue(journalStep("r1", 2, .unsourced))
        log.enqueue(journalStep("r1", 3, .droppedOwnData))
        log.enqueue(journalCommit("r1", 2, AssistantCommitMark(stage: .emit, ok: true, generation: 7)))
        log.enqueue(journalCommit("r1", 2, AssistantCommitMark(stage: .display, ok: true, generation: 7)))
        log.enqueue(journalCommit("r1", 2, AssistantCommitMark(stage: .audio, ok: false, generation: 8)))
        log.enqueue(journalCommit("r1", 0, AssistantCommitMark(stage: .history, ok: true, generation: 8)))
        log.enqueue(journalEnd("r1", .interrupted, areas: ["books"]))
        let read = await log.read()
        XCTAssertEqual(read.skippedLines, 0)
        XCTAssertNil(read.error)
        XCTAssertEqual(read.entries.count, 1)
        guard let e = read.entries.first else { return }
        XCTAssertEqual(e.kind, .answer)
        XCTAssertEqual(e.requestId, "r1")
        XCTAssertEqual(e.sessionId, "sitzung")
        XCTAssertEqual(e.modelKind, .other)
        XCTAssertTrue(e.cloud)
        XCTAssertTrue(e.unsourcedMode)
        XCTAssertEqual(e.consentVersion, 3)
        XCTAssertEqual(e.contextMode, .free)
        XCTAssertEqual(e.contextReason, .free)
        XCTAssertEqual(e.round, .general)
        XCTAssertEqual(e.generation, 7)
        XCTAssertEqual(e.outcome, .interrupted)
        XCTAssertEqual(e.areas, ["books"])
        XCTAssertEqual(e.trace.map(\.n), [1, 2, 3])
        XCTAssertEqual(e.trace.map(\.verdict), [.announcement, .unsourced, .droppedOwnData])
        XCTAssertEqual(e.trace[1].commits, [
            AssistantCommitMark(stage: .emit, ok: true, generation: 7), AssistantCommitMark(stage: .display, ok: true, generation: 7),
            AssistantCommitMark(stage: .audio, ok: false, generation: 8),
        ])
        XCTAssertEqual(e.history, AssistantCommitMark(stage: .history, ok: true, generation: 8))
        XCTAssertEqual(e.counts, AssistantSentenceCounts(unsourced: 1, droppedOwnData: 1), "gezählt aus den Schritten, Ansagen zählen nicht")
        XCTAssertEqual(e.level, .s1)
    }

    func testFileIsOneJSONObjectPerLineAndStartComesFirst() async throws {
        let log = AssistantAuditLog(directory: dir)
        let began = await log.begin(journalStart("r1"))
        XCTAssertTrue(began)
        log.enqueue(journalStep("r1", 1))
        log.enqueue(journalCommit("r1", 1, AssistantCommitMark(stage: .emit, ok: true, generation: 1)))
        log.enqueue(journalEnd("r1"))
        let err = await log.flush(request: "r1")
        XCTAssertNil(err)
        let raw = try String(contentsOf: fileURL, encoding: .utf8)
        XCTAssertTrue(raw.hasSuffix("\n"))
        let objs = try objects()
        XCTAssertEqual(objs.compactMap { $0["t"] as? String }, ["start", "step", "commit", "end"])
        XCTAssertTrue(objs.allSatisfy { $0["req"] as? String == "r1" })
        XCTAssertEqual(raw.split(separator: "\n").count, 4, "keine Zeile enthält einen Zeilenumbruch")
    }

    func testEventsCarryTheNewGenerationAndAnswersAreNotAcceptedByAppend() async {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 2, generation: 7))
        _ = await log.record(AssistantAuditEntry(kind: .unsourcedOff, consentVersion: 2, generation: 8))
        _ = await log.record(AssistantAuditEntry(kind: .cloudRevoked, cloud: true, generation: 9))
        _ = await log.record(AssistantAuditEntry(kind: .resumedFree, unsourcedMode: true, consentVersion: 2, generation: 10))
        let refused = await log.append(AssistantAuditEntry(kind: .answer))
        XCTAssertEqual(refused, .encodeFailed, "Antworten schreibt nur der Kern als Journal")
        let all = await log.entries()
        XCTAssertEqual(all.map(\.kind), [.unsourcedOn, .unsourcedOff, .cloudRevoked, .resumedFree])
        XCTAssertEqual(all.map(\.generation), [7, 8, 9, 10])
        XCTAssertEqual(all.map(\.consentVersion), [2, 2, 0, 2])
    }

    func testFoldSkipsOrphanedAndUnknownLinesAndCountsThem() async throws {
        let unknownKind = #"{"t":"zukunft","req":"x"}"#
        var raw = Data()
        for l in [journalStep("verwaist", 1), journalCommit("verwaist", 1, AssistantCommitMark(stage: .emit, ok: true, generation: 1)),
                  journalStart("r1"), journalCommit("r1", 5, AssistantCommitMark(stage: .display, ok: true, generation: 1)),
                  journalStep("r1", 1), journalEnd("r1"), journalEnd("verwaist")] {
            raw.append(encoded(l))
            raw.append(10)
        }
        raw.append(Data((unknownKind + "\n").utf8))
        try writeRaw(raw)
        let read = await freshLog().read()
        XCTAssertEqual(read.entries.count, 1)
        XCTAssertEqual(read.entries[0].trace.map(\.n), [1])
        XCTAssertEqual(read.entries[0].trace[0].commits, [], "Commit zu unbekanntem Schritt wird nicht erfunden")
        XCTAssertEqual(read.skippedLines, 5, "vier verwaiste Zeilen und eine unbekannte Art")
    }

    func testCountsFromTheTraceMarkAnUndeliveredSentenceAsRevoked() {
        let ok = AssistantCommitMark(stage: .emit, ok: true, generation: 1)
        let no = AssistantCommitMark(stage: .emit, ok: false, generation: 2)
        let trace = [
            AssistantTraceStep(n: 1, verdict: .announcement, generation: 1, commits: [ok]),
            AssistantTraceStep(n: 2, verdict: .grounded, generation: 1, commits: [ok]),
            AssistantTraceStep(n: 3, verdict: .grounded, generation: 1, commits: [no]),
            AssistantTraceStep(n: 4, verdict: .inventory, generation: 1, commits: [no]),
            AssistantTraceStep(n: 5, verdict: .template, generation: 1, commits: [ok]),
            AssistantTraceStep(n: 6, verdict: .droppedFragment, generation: 1),
            AssistantTraceStep(n: 7, verdict: .droppedRevoked, generation: 2),
            AssistantTraceStep(n: 8, verdict: .droppedNotSnapped, generation: 2),
        ]
        XCTAssertEqual(AssistantJournalFold.counts(from: trace),
                       AssistantSentenceCounts(grounded: 1, neutral: 1, droppedNotSnapped: 1, revoked: 3, droppedFragment: 1))
    }

    // MARK: Öffnen: Verdichten

    /// 520 Anfragen: Beim Öffnen bleiben die letzten 500 (mit allen ihren Zeilen), die Datei wird atomar ersetzt.
    func testOpeningCompactsToTheLast500Requests() async throws {
        var lines: [AssistantJournalLine] = []
        for n in 0..<520 { lines += [journalStart("q\(n)", at: Double(n)), journalStep("q\(n)", 1), journalEnd("q\(n)")] }
        try writeLines(lines)
        let log = freshLog()
        let err = await log.append(journalEvent(1))
        XCTAssertNil(err)
        let read = await log.read()
        XCTAssertEqual(read.entries.count, AssistantAuditLog.maxEntries + 1, "500 Anfragen und das neue Ereignis")
        XCTAssertEqual(read.entries.first?.requestId, "q20")
        XCTAssertEqual(read.entries[499].requestId, "q519")
        XCTAssertEqual(read.entries.last?.kind, .unsourcedOn)
        XCTAssertEqual(try objects().count, 500 * 3 + 1)
        XCTAssertEqual(read.skippedLines, 0)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path), [AssistantAuditLog.fileName], "keine Temp-Reste")
    }

    /// Die Zeilen einer Anfrage liegen nicht beieinander (gleichzeitige Antworten): sie bleiben oder fallen als Ganzes.
    func testCompactionDropsARequestWithAllItsLinesEvenWhenTheyAreSpreadOut() async throws {
        var lines: [AssistantJournalLine] = [journalStart("alt")]
        for n in 0..<500 { lines += [journalStart("q\(n)"), journalEnd("q\(n)")] }
        lines += [journalStep("alt", 1), journalEnd("alt")]
        try writeLines(lines)
        let log = freshLog()
        _ = await log.append(journalEvent(1))
        let raw = rawLog()
        XCTAssertFalse(raw.contains("\"alt\""), "keine Zeile der ältesten Anfrage bleibt übrig")
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 501)
        XCTAssertEqual(read.skippedLines, 0)
        XCTAssertEqual(read.entries.first?.requestId, "q0")
    }

    func testEventsCountAsUnitsWhenCompacting() async throws {
        var raw = Data()
        for n in 0..<300 {
            raw.append(encoded(journalStart("q\(n)")))
            raw.append(10)
            raw.append(encoded(journalEnd("q\(n)")))
            raw.append(10)
        }
        // 300 Anfragen und 250 alte Ereigniszeilen (altes Format): 550 Einheiten, die ältesten 50 fallen.
        for n in 0..<250 {
            raw.append(Data(#"{"at":"2025-10-09T09:20:00Z","kind":"unsourcedOn","consentVersion":\#(n)}"#.utf8))
            raw.append(10)
        }
        try writeRaw(raw)
        let log = freshLog()
        _ = await log.append(journalEvent(1))
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 501)
        XCTAssertEqual(read.entries.first?.requestId, "q50")
    }

    // MARK: Öffnen: unvollständige Anfragen schließen

    func testOpeningClosesUnfinishedRequestsAsIncompleteExactlyOnce() async throws {
        try writeLines([journalStart("a"), journalStep("a", 1), journalEnd("a"), journalStart("b"), journalStep("b", 1),
                        journalStep("b", 2)])
        let log = freshLog()
        _ = await log.append(journalEvent(1))
        let read = await log.read()
        XCTAssertEqual(read.entries.map(\.requestId), ["a", "b", nil])
        XCTAssertEqual(read.entries.map(\.outcome), [.completed, .incomplete, nil])
        XCTAssertEqual(read.entries[1].trace.map(\.n), [1, 2], "die bis zum Absturz angefallenen Schritte bleiben")
        XCTAssertEqual(rawLog().components(separatedBy: "\"outcome\":\"incomplete\"").count - 1, 1)

        // Ein zweiter Schreiber auf derselben Datei schließt nichts mehr.
        let again = freshLog()
        _ = await again.append(journalEvent(2))
        XCTAssertEqual(rawLog().components(separatedBy: "\"outcome\":\"incomplete\"").count - 1, 1)
        let all = await again.entries()
        XCTAssertEqual(all.map(\.outcome), [.completed, .incomplete, nil, nil])
    }

    /// Auch ohne Verdichten (Datei klein) und mit abgebrochener letzter Zeile wird geschlossen; die halbe Zeile bleibt, wie sie ist.
    func testClosingAfterATornLastLineStartsOnANewLine() async throws {
        var raw = Data()
        for l in [journalStart("a"), journalStep("a", 1)] {
            raw.append(encoded(l))
            raw.append(10)
        }
        raw.append(Data(#"{"t":"step","req":"a","n":"#.utf8))
        try writeRaw(raw)
        let log = freshLog()
        _ = await log.append(journalEvent(1))
        let read = await log.read()
        XCTAssertEqual(read.entries.map(\.outcome), [.incomplete, nil])
        XCTAssertEqual(read.skippedLines, 1, "nur die halbe Zeile")
        XCTAssertTrue(rawLog().contains("{\"t\":\"step\",\"req\":\"a\",\"n\":\n"), "die halbe Zeile ist unverändert, dahinter ein Zeilenumbruch")
    }

    // MARK: Fehlerfälle

    /// Ein abgebrochener Schreibvorgang hinterlässt eine halbe letzte Zeile: Sie wird übersprungen und gezählt, der Rest bleibt
    /// lesbar, und die nächste Zeile landet sauber dahinter.
    func testTornLastLineIsSkippedCountedAndTheRestSurvives() async throws {
        let log = AssistantAuditLog(directory: dir)
        let began = await log.begin(journalStart("r1"))
        XCTAssertTrue(began)
        log.enqueue(journalEnd("r1"))
        _ = await log.flush(request: "r1")
        var data = try Data(contentsOf: fileURL)
        data.append(Data(#"{"at":"2025-10-"#.utf8))
        try data.write(to: fileURL)
        var read = await freshLog().read()
        XCTAssertEqual(read.entries.count, 1)
        XCTAssertEqual(read.skippedLines, 1)
        XCTAssertNil(read.error)
        let second = freshLog()
        _ = await second.append(journalEvent(3))
        read = await second.read()
        XCTAssertEqual(read.entries.count, 2)
        XCTAssertEqual(read.skippedLines, 1, "die beschädigte Zeile bleibt erhalten und wird weiter gemeldet")
        XCTAssertEqual(read.entries.last?.kind, .unsourcedOn)
    }

    /// Voller Datenträger mitten im Anhängen: die halbe Zeile bleibt, der Fehler wird für die Anfrage gemeldet (nicht
    /// verschluckt), die folgenden Zeilen beginnen in einer neuen Zeile und gehen durch.
    func testAppendFailureMidLineIsReportedAndLaterLinesStartOnANewLine() async throws {
        let flaky = Flaky(failures: 1)
        let log = freshLog(appender: { url, data in
            // Die erste Zeile nach `start` bricht nach der Hälfte ab (Datenträger voll).
            if data.count > 40, String(decoding: data, as: UTF8.self).contains("\"t\":\"step\""), flaky.fail() {
                try? AssistantAuditWriter.appendFile(url, data.prefix(data.count / 2))
                throw DiskFull()
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let began = await log.begin(journalStart("r1"))
        XCTAssertTrue(began)
        log.enqueue(journalStep("r1", 1))
        log.enqueue(journalStep("r1", 2))
        log.enqueue(journalEnd("r1"))
        let err = await log.flush(request: "r1")
        XCTAssertEqual(err, .writeFailed, "der Fehler geht an den Abschluss der Anfrage")
        let again = await log.flush(request: "r1")
        XCTAssertNil(again, "ein gemeldeter Fehler wird vergessen")
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 1)
        XCTAssertEqual(read.entries[0].trace.map(\.n), [2], "Schritt 1 ging verloren und wurde gemeldet, Schritt 2 steht")
        XCTAssertEqual(read.entries[0].outcome, .completed)
        XCTAssertEqual(read.skippedLines, 1)
    }

    /// Fehler beim ersten Schreiben (`start`): `begin` meldet `false`, der Fehler bleibt für den Abschluss gemerkt.
    func testBeginReportsAFailureAndKeepsItForTheEndOfTheRequest() async throws {
        let log = freshLog(appender: { _, _ in throw DiskFull() })
        let began = await log.begin(journalStart("r1"))
        XCTAssertFalse(began)
        let err = await log.flush(request: "r1")
        XCTAssertEqual(err, .writeFailed)
    }

    /// Lesefehler: wird gemeldet, nichts wird überschrieben, die Datei bleibt byte-gleich; der nächste Versuch klappt wieder.
    func testReadErrorIsReportedOverwritesNothingAndIsRetried() async throws {
        let healthy = AssistantAuditLog(directory: dir)
        _ = await healthy.record(journalEvent(1))
        _ = await healthy.record(journalEvent(2))
        let before = try Data(contentsOf: fileURL)

        let flaky = Flaky(failures: 3)   // Schreiben, Anfangszeile, Lesen scheitern; der vierte Zugriff geht
        let failing = AssistantAuditLog(directory: dir, reader: { url in
            if flaky.fail() { throw DiskFull() }
            return try AssistantAuditWriter.readFile(url)
        })
        let first = await failing.append(journalEvent(3))
        XCTAssertEqual(first, .readFailed)
        let began = await failing.begin(journalStart("r1"))
        XCTAssertFalse(began)
        XCTAssertEqual(try Data(contentsOf: fileURL), before, "nichts überschrieben")
        let unreadable = await failing.read()
        XCTAssertEqual(unreadable.error, .readFailed)
        XCTAssertEqual(unreadable.entries, [])
        let intact = await healthy.entries()
        XCTAssertEqual(intact.count, 2)
        // Das Lesen scheiterte schon dreimal; jetzt geht es wieder, und das Schreiben holt auf.
        let retried = await failing.append(journalEvent(4))
        XCTAssertNil(retried)
        XCTAssertEqual(try objects().count, 3)
    }

    /// Schreibfehler bei fehlendem Verzeichnis (an seiner Stelle liegt eine Datei; so verhält sich auch ein schreibgeschützter
    /// Datenträger): gemeldet, nichts stürzt, die Datei bleibt.
    func testWriteFailureIsReportedAsWriteFailed() async throws {
        let blockerFile = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-file-\(UUID().uuidString)")
        try Data("x".utf8).write(to: blockerFile)
        defer { try? FileManager.default.removeItem(at: blockerFile) }
        let log = AssistantAuditLog(directory: blockerFile.appendingPathComponent("unter"))
        let result = await log.append(journalEvent(1))
        XCTAssertEqual(result, .writeFailed)
        let ok = await log.record(journalEvent(2))
        XCTAssertFalse(ok)
        XCTAssertEqual(try Data(contentsOf: blockerFile), Data("x".utf8))
    }

    /// Ein Verzeichnis an der Stelle der Datei: Lesen scheitert, andere Protokolle bleiben unberührt.
    func testFailedAccessIsReportedAndOtherLogsAreUnaffected() async throws {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(journalEvent(1))
        let before = try Data(contentsOf: fileURL)
        let blocked = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-blocked-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: blocked) }
        try FileManager.default.createDirectory(at: blocked.appendingPathComponent(AssistantAuditLog.fileName), withIntermediateDirectories: true)
        let broken = AssistantAuditLog(directory: blocked)
        let ok = await broken.record(journalEvent(2))
        XCTAssertFalse(ok)
        XCTAssertEqual(try Data(contentsOf: fileURL), before)
        let stillThere = await log.record(journalEvent(3))
        XCTAssertTrue(stillThere, "ein Fehler an anderer Stelle beeinträchtigt dieses Protokoll nicht")
    }

    /// Das Umschreiben beim Öffnen scheitert (Datenträger voll, Abbruch mitten im Schreiben der Temp-Datei): Die alte Datei bleibt
    /// byte-gleich, der Schreiber hängt trotzdem an (ohne Verdichtung), schließt offene Anfragen und versucht es später erneut.
    func testFailedRewriteLeavesTheOldFileIdenticalAndTheWriterStillAppends() async throws {
        var lines: [AssistantJournalLine] = []
        for n in 0..<520 { lines += [journalStart("q\(n)"), journalEnd("q\(n)")] }
        lines += [journalStart("offen"), journalStep("offen", 1)]
        try writeLines(lines)
        let before = try Data(contentsOf: fileURL)
        let partial = dir.appendingPathComponent("rest.tmp")
        let log = freshLog(rewriter: { _, data in
            try? data.prefix(data.count / 2).write(to: partial)   // abgebrochen mitten im Schreiben der Temp-Datei
            throw DiskFull()
        })
        let err = await log.append(journalEvent(1))
        XCTAssertNil(err, "ohne Verdichtung wird trotzdem angehängt")
        let after = try Data(contentsOf: fileURL)
        XCTAssertEqual(after.prefix(before.count), before[...], "die alte Datei ist unversehrt, alles Neue steht dahinter")
        XCTAssertGreaterThan(after.count, before.count)
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 520 + 1 + 1, "nichts verdichtet, nichts verloren")
        XCTAssertEqual(read.entries.first(where: { $0.requestId == "offen" })?.outcome, .incomplete, "offene Anfrage trotzdem geschlossen")
        // Beim nächsten Öffnen mit funktionierendem Datenträger wird verdichtet.
        let later = freshLog()
        _ = await later.append(journalEvent(2))
        let compacted = await later.read()
        XCTAssertEqual(compacted.entries.count, 501, "500 Einheiten und das neue Ereignis")
        XCTAssertEqual(compacted.entries.last?.kind, .unsourcedOn)
    }

    func testFailedRewriteKeepsLegacyLinesAsTheyWere() async throws {
        let legacy = #"{"areas":["books"],"at":"2025-10-09T09:20:00Z","cloud":false,"consentVersion":1,"dropped":3,"grounded":2,"kind":"answer","model":"Apple Foundation Models","neutral":2,"outcome":"completed","unsourced":1,"unsourcedMode":true}"#
        try writeRaw(Data((legacy + "\n").utf8))
        let log = freshLog(rewriter: { _, _ in throw DiskFull() })
        _ = await log.append(journalEvent(1))
        XCTAssertTrue(rawLog().hasPrefix(legacy + "\n"), "die alte Zeile steht unverändert da")
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 2)
        XCTAssertEqual(read.entries[0].modelKind, .apple)
    }

    func testNoStrayTemporaryFilesAfterWritingAndRewriting() async throws {
        var lines: [AssistantJournalLine] = []
        for n in 0..<510 { lines += [journalStart("q\(n)"), journalEnd("q\(n)")] }
        try writeLines(lines)
        let log = freshLog()
        for n in 0..<5 { _ = await log.record(journalEvent(n)) }
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path), [AssistantAuditLog.fileName])
    }

    func testClearRemovesTheFile() async {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(journalEvent(1))
        await log.clear()
        let all = await log.entries()
        XCTAssertEqual(all, [])
        XCTAssertFalse(FileManager.default.fileExists(atPath: log.url.path))
        let again = await log.record(journalEvent(2))
        XCTAssertTrue(again, "nach dem Löschen geht es weiter")
    }

    // MARK: Gleichzeitigkeit

    /// 40 Anfragen schreiben gleichzeitig (Start, drei Schritte, Ende): keine Zeile geht verloren, innerhalb einer Anfrage bleibt
    /// die Reihenfolge, keine Zeile wird zerrissen.
    func testConcurrentRequestsKeepEveryLineAndTheOrderWithinARequest() async throws {
        let log = AssistantAuditLog(directory: dir)
        await withTaskGroup(of: Void.self) { group in
            for r in 0..<40 {
                group.addTask {
                    let id = "q\(r)"
                    _ = await log.begin(journalStart(id, at: Double(r)))
                    for n in 1...3 { log.enqueue(journalStep(id, n)) }
                    log.enqueue(journalEnd(id))
                    _ = await log.flush(request: id)
                }
            }
        }
        let objs = try objects()
        XCTAssertEqual(objs.count, 40 * 5)
        var byRequest: [String: [String]] = [:]
        for o in objs {
            let id = try XCTUnwrap(o["req"] as? String)
            byRequest[id, default: []].append(o["t"] as? String ?? "?")
        }
        XCTAssertEqual(byRequest.count, 40)
        for (id, order) in byRequest { XCTAssertEqual(order, ["start", "step", "step", "step", "end"], id) }
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 40)
        XCTAssertEqual(read.skippedLines, 0)
        XCTAssertTrue(read.entries.allSatisfy { $0.trace.map(\.n) == [1, 2, 3] && $0.outcome == .completed })
    }

    /// Zwei Handles auf dasselbe Verzeichnis (zum Beispiel Controller und Sitzung) teilen einen Schreiber: 50 + 50
    /// gleichzeitige Ereignisse ergeben 100, keines überschreibt das andere.
    func testTwoHandlesOnTheSameDirectoryShareOneWriter() async {
        let a = AssistantAuditLog(directory: dir)
        let b = AssistantAuditLog(directory: dir.appendingPathComponent("unterordner").appendingPathComponent(".."))
        XCTAssertTrue(AssistantAuditRegistry.writer(for: dir) === AssistantAuditRegistry.writer(for: b.directory),
                      "gleiches Verzeichnis, auch bei anderer Schreibweise des Pfads")
        let first = (0..<50).map { journalEvent($0) }
        let second = (50..<100).map { journalEvent($0) }
        await withTaskGroup(of: Void.self) { group in
            for e in first { group.addTask { _ = await a.record(e) } }
            for e in second { group.addTask { _ = await b.record(e) } }
        }
        let all = await a.entries()
        XCTAssertEqual(all.count, 100)
        XCTAssertEqual(Set(all.map(\.generation)), Set(1...100))
        let other = AssistantAuditLog(directory: dir.appendingPathComponent("anderes"))
        XCTAssertFalse(AssistantAuditRegistry.writer(for: dir) === AssistantAuditRegistry.writer(for: other.directory))
    }

    // MARK: Altdaten (Migration vorwärts)

    private let legacyLine = #"{"areas":["books"],"at":"2025-10-09T09:20:00Z","cloud":false,"consentVersion":1,"dropped":3,"grounded":2,"kind":"answer","model":"Apple Foundation Models","neutral":2,"outcome":"completed","unsourced":1,"unsourcedMode":true}"#

    /// Zeilen aus der Zeit vor dem Journal (flache Zähler, `model` als Freitext) laden und werden zu Art und neuen Zählern. Das
    /// Lesen verändert die Datei nicht; erst das Öffnen zum Schreiben überträgt sie ins neue Format (kein Freitext mehr, Zähler
    /// in `counts`) und hängt die neue Zeile an.
    func testLegacyLinesLoadWithoutTouchingTheFileAndAreMigratedOnTheFirstWrite() async throws {
        try writeRaw(Data((legacyLine + "\n").utf8))
        let log = AssistantAuditLog(directory: dir)
        var all = await log.entries()
        XCTAssertEqual(all.count, 1)
        let old = all[0]
        XCTAssertEqual(old.modelKind, .apple, "der Freitext wird nur zur Art")
        XCTAssertEqual(old.counts, AssistantSentenceCounts(grounded: 2, unsourced: 1, neutral: 2, droppedNotSnapped: 3))
        XCTAssertEqual(old.level, .s1)
        XCTAssertNil(old.round)
        XCTAssertEqual(old.generation, 0)
        XCTAssertEqual(old.trace, [])
        XCTAssertNil(old.history)
        XCTAssertNil(old.requestId)
        XCTAssertEqual(old.areas, ["books"])
        XCTAssertEqual(old.outcome, .completed)
        XCTAssertEqual(rawLog(), legacyLine + "\n", "Lesen schreibt nichts")

        let ok = await log.record(journalEvent(5))
        XCTAssertTrue(ok)
        all = await log.entries()
        XCTAssertEqual(all.count, 2)
        XCTAssertEqual(all[0], old, "inhaltlich unverändert")
        XCTAssertEqual(all[1].kind, .unsourcedOn)
        let raw = rawLog()
        XCTAssertFalse(raw.contains("Foundation"), "der Freitext ist weg")
        XCTAssertFalse(raw.contains("\"model\":\"Apple"))
        XCTAssertFalse(raw.contains("\"dropped\""), "flache Zähler sind nach `counts` übertragen")
        XCTAssertEqual(raw.split(separator: "\n").count, 2)
    }

    /// Unlesbares und Fremdes bleibt beim Schreiben Byte für Byte erhalten (nie Datenverlust durch die Übertragung).
    func testMigrationKeepsUnreadableAndFutureLinesVerbatim() async throws {
        let future = #"{"at":"2030-01-01T00:00:00Z","kind":"answer","neuesFeld":{"x":1}}"#
        let broken = #"{"at":"2025-10-"#
        let brokenModel = #"{"model":"Fremd","kind":"gibtEsNicht"}"#
        try writeRaw(Data([legacyLine, future, broken, brokenModel].joined(separator: "\n").appending("\n").utf8))
        let log = AssistantAuditLog(directory: dir)
        let ok = await log.record(journalEvent(1))
        XCTAssertTrue(ok)
        let lines = rawLog().split(separator: "\n").map(String.init)
        XCTAssertEqual(lines.count, 5)
        XCTAssertFalse(lines[0].contains("Foundation"))
        XCTAssertEqual(Array(lines[1...3]), [future, broken, brokenModel], "nur bekannte alte Zeilen werden übertragen")
        let read = await log.read()
        XCTAssertEqual(read.skippedLines, 2)
        XCTAssertEqual(read.entries.count, 3)
    }

    /// Alte Zeilen und Journalzeilen in einer Datei: beides wird gelesen, die Reihenfolge bleibt, neue Zeilen kommen dahinter.
    func testLegacyAndJournalLinesCoexist() async throws {
        var raw = Data((legacyLine + "\n").utf8)
        for l in [journalStart("neu"), journalStep("neu", 1), journalEnd("neu")] {
            raw.append(encoded(l))
            raw.append(10)
        }
        try writeRaw(raw)
        let log = freshLog()
        _ = await log.append(journalEvent(2))
        let read = await log.read()
        XCTAssertEqual(read.entries.map(\.requestId), [nil, "neu", nil])
        XCTAssertEqual(read.entries.map(\.kind), [.answer, .answer, .unsourcedOn])
        XCTAssertEqual(read.entries[1].counts, AssistantSentenceCounts(grounded: 1))
        XCTAssertEqual(read.skippedLines, 0)
    }

    func testLegacyLineWithMinimalFieldsLoads() async throws {
        try writeRaw(Data((#"{"at":"2025-10-09T09:20:00Z","kind":"unsourcedOn"}"# + "\n").utf8))
        let all = await AssistantAuditLog(directory: dir).entries()
        XCTAssertEqual(all.first?.kind, .unsourcedOn)
        XCTAssertEqual(all.first?.counts, AssistantSentenceCounts())
        XCTAssertNil(all.first?.level, "nur Antworten haben eine Stufe")
    }

    // MARK: Stufe, Kennungen

    func testLevelNeverS0WithUnsourcedSentences() throws {
        let s0 = AssistantAuditEntry(kind: .answer, counts: AssistantSentenceCounts(neutral: 2), level: .s0)
        XCTAssertEqual(s0.level, .s0, "nur Vorlagen: S0")
        let clamped = AssistantAuditEntry(kind: .answer, counts: AssistantSentenceCounts(unsourced: 1), level: .s0)
        XCTAssertEqual(clamped.level, .s1, "mit Sätzen ohne Quelle nie S0")
        let grounded = AssistantAuditEntry(kind: .answer, counts: AssistantSentenceCounts(grounded: 1))
        XCTAssertEqual(grounded.level, .s1)
        XCTAssertEqual(AssistantAuditEntry(kind: .answer, counts: AssistantSentenceCounts(inventory: 1)).level, .s1)
        XCTAssertEqual(AssistantAuditEntry(kind: .answer).level, .s0, "eine Anfrage ohne Satz ist S0")
        XCTAssertNil(AssistantAuditEntry(kind: .unsourcedOn).level)
        let json = #"{"at":"2025-10-09T09:20:00Z","kind":"answer","level":"s0","counts":{"unsourced":1}}"#
        XCTAssertEqual(try AssistantJournalFold.decoder().decode(AssistantAuditEntry.self, from: Data(json.utf8)).level, .s1)
    }

    func testCommitMarkCodecUsesShortFixedIdentifiers() throws {
        let marks = [
            AssistantCommitMark(stage: .emit, ok: true, generation: 7), AssistantCommitMark(stage: .display, ok: true, generation: 7),
            AssistantCommitMark(stage: .audio, ok: false, generation: 8), AssistantCommitMark(stage: .history, ok: true, generation: 123),
        ]
        let data = try JSONEncoder().encode(marks)
        XCTAssertEqual(String(decoding: data, as: UTF8.self), #"["e+7","d+7","a-8","h+123"]"#)
        XCTAssertEqual(try JSONDecoder().decode([AssistantCommitMark].self, from: data), marks)
        for bad in ["x+1", "e*1", "e+", "e", "", "e+x", "d1"] {
            XCTAssertThrowsError(try JSONDecoder().decode([AssistantCommitMark].self, from: Data("[\"\(bad)\"]".utf8)), bad)
        }
    }

    func testModelIsAControlledKindNotFreeText() async throws {
        XCTAssertEqual(AssistantModelKind.classify(label: "Apple-Systemmodell · lokal"), .apple)
        XCTAssertEqual(AssistantModelKind.classify(label: "Gemma 3n"), .gemma)
        XCTAssertEqual(AssistantModelKind.classify(label: "Mein Geheimes Modell 7"), .other)
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(AssistantAuditEntry(kind: .cloudGranted, model: "Mein Geheimes Modell 7", cloud: true))
        let raw = rawLog()
        XCTAssertFalse(raw.contains("Geheim"))
        XCTAssertTrue(raw.contains("\"model\":\"other\""))
        XCTAssertTrue(raw.contains("\"cloud\":true"))
    }

    // MARK: Keine Inhalte

    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400,
                                         userName: "Wurzelbrunst", timeout: 5, allowUnsourced: true, consentVersion: 1)

    private func geheimKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", detail: "Autorin Zimtstern", sections: [
            ("S. 7", "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."),
        ]))
        await corpus.upsert(assistantDoc("n1", "Notiz Kirschkernweitwurf", area: .notes, sections: [
            (nil, "Der Kirschkernweitwurf findet am Sonntag statt."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    /// Das Journal hat gar kein Feld für Text: Schlüssel und Zeichenketten stammen aus festen Listen oder sind Kennungen.
    func testJournalLinesHaveOnlyTheAgreedKeysAndOnlyControlledStrings() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen. Kieselsteine sind meistens rund."]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        for await e in session.events(AssistantRequest(requestId: "r1", text: question)) { _ = e }
        session.noteCommit(requestId: "r1", index: 1, stage: .display, ok: true, generation: session.releaseGeneration)
        await session.commit(requestId: "r1", user: question, spoken: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula.", interrupted: false)
        _ = await audit.record(AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 1, generation: 3))
        // Jede Art der Erhöhung, damit auch die Zeilen `release` (Grund, neue Bereichsnamen) unter die Prüfung fallen.
        await session.setAllowUnsourced(false)
        await session.setAllowUnsourced(true, consentVersion: 1)
        await session.releaseDidChange(.consent)
        await session.releaseDidChange()
        await session.setAreas([.books, .notes])
        _ = await audit.flushReleaseLines()

        let keys: Set<String> = ["t", "req", "at", "session", "model", "cloud", "unsourced", "consent", "ctx", "reason", "round", "gen", "n",
                                 "verdict", "mark", "outcome", "level", "areas", "kind", "cause", "granted", "run", "chg", "locked"]
        let words: Set<String> = [
            "start", "step", "commit", "end", "event", "round", "areas", "r1", "apple", "gemma", "other", "data", "free", "general", "optedOut", "locked",
            "inventory", "ownData", "relevantPassage", "grounded", "unsourced", "template", "announcement", "droppedOwnData",
            "droppedNoMode", "droppedNotSnapped", "droppedDuplicate", "droppedTemplate", "droppedRevoked", "droppedFragment", "completed",
            "interrupted", "revoked", "failed", "incomplete", "limited", "s0", "s1", "books", "transcripts", "files", "notes",
            "answer", "unsourcedOn", "unsourcedOff", "cloudGranted", "cloudRevoked", "resumedFree",
            "release", "state", "optIn", "consent", "resumeFree", "intent", "effective", "ineffective",
        ]
        func walk(_ any: Any, _ strings: inout [String]) {
            if let s = any as? String { strings.append(s) }
            else if let a = any as? [Any] { a.forEach { walk($0, &strings) } }
            else if let d = any as? [String: Any] { d.values.forEach { walk($0, &strings) } }
        }
        let objs = try objects()
        XCTAssertGreaterThanOrEqual(objs.count, 6)
        let marks = try NSRegularExpression(pattern: "^[edah][+-][0-9]+$")
        let formatter = ISO8601DateFormatter()
        for o in objs {
            XCTAssertTrue(Set(o.keys).isSubset(of: keys), "unerwartete Schlüssel: \(Set(o.keys).subtracting(keys))")
            var strings: [String] = []
            walk(o, &strings)
            for s in strings where !words.contains(s) {
                let isMark = marks.firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) != nil
                let isDate = formatter.date(from: s) != nil
                let isId = UUID(uuidString: s) != nil
                if !(isMark || isDate || isId) { XCTFail("unerwartete Zeichenkette im Journal: \(s)") }
            }
        }
    }

    /// Negativtest: Frage, Antwort, Titel und Auszug einer echten Antwort dürfen nirgends im Protokoll stehen.
    func testJournalOfARealAnswerContainsNoQuestionAnswerTitleOrExcerpt() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen. Kieselsteine sind meistens rund."]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        var spoken: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: question)) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        XCTAssertEqual(spoken, ["Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."])
        await session.commit(requestId: "r1", user: question, spoken: spoken.joined(separator: " "), interrupted: false)
        _ = await audit.record(AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 1))
        let raw = rawLog()
        for word in ["Quasselbert", "Pappelhausen", "Fahrrad", "Kieselsteine", "Flimmerkasten", "Zimtstern", "Lavendula", "Wurzelbrunst", "wandert",
                     "Wohin", "Titel", "S. 7", "Kirschkern"] {
            XCTAssertFalse(raw.localizedCaseInsensitiveContains(word), "\(word) steht im Protokoll:\n\(raw)")
        }
        let entries = await audit.entries().filter { $0.kind != .state }   // der Stand beim Öffnen steht davor
        XCTAssertEqual(entries.count, 2)
        XCTAssertEqual(entries[0].kind, .answer)
        XCTAssertEqual(entries[0].modelKind, .other)
        XCTAssertEqual(entries[0].round, .data)
        XCTAssertEqual(entries[0].counts, AssistantSentenceCounts(grounded: 1, droppedNotSnapped: 1))
        XCTAssertEqual(entries[0].level, .s1)
        XCTAssertEqual(entries[0].areas, ["books"])
        XCTAssertTrue(entries[0].unsourcedMode)
        XCTAssertFalse(entries[0].cloud)
        XCTAssertEqual(entries[0].consentVersion, 1)
        XCTAssertEqual(entries[0].outcome, .completed)
        XCTAssertEqual(entries[1].kind, .unsourcedOn)
    }

    /// Bestandsangaben: Titel stehen ungekürzt in der Ausgabe, aber nie im Protokoll; die Bereiche der Angaben stehen in `end`.
    func testInventoryStatementsKeepTheirTitlesOutOfTheJournalButTheirAreasInTheEndLine() async throws {
        let corpus = AssistantCorpus()
        let longTitle = "Ein ungewöhnlich langer Buchtitel Zitronenfalter " + String(repeating: "und noch mehr Worte ", count: 8) + "Ende"
        await corpus.upsert(assistantDoc("b1", longTitle, sections: [("S. 1", "Irgendein Text über Zitronenfalter.")]))
        await corpus.upsert(assistantDoc("n1", "Notiz Kirschkernweitwurf", area: .notes, sections: [(nil, "Der Weitwurf findet statt.")]))
        await corpus.upsert(assistantDoc("t1", "Besprechung Himbeerkuchen", area: .transcripts, sections: [("00:01", "Ein Satz im Transkript.")]))
        let knowledge = AssistantKnowledge(corpus: corpus)
        await knowledge.setAreas([.books, .notes])
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: []), knowledge: knowledge, config: config, audit: audit, released: true)
        var texts: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Was habe ich gespeichert?")) {
            if case .sentence(_, _, let t, _, _, _, _) = e { texts.append(t) }
        }
        XCTAssertTrue(texts.contains { $0.contains(longTitle) }, "der Titel steht ungekürzt in der Ausgabe: \(texts)")
        await session.commit(requestId: "r1", user: "Was habe ich gespeichert?", spoken: texts.joined(separator: " "), interrupted: false)
        let raw = rawLog()
        for word in ["Zitronenfalter", "Kirschkern", "Himbeerkuchen", "Buchtitel", "Weitwurf"] {
            XCTAssertFalse(raw.localizedCaseInsensitiveContains(word), word)
        }
        let entries = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries[0].areas, ["books", "notes"], "Bereiche der Bestandsangaben, nicht die der freigegebenen Bereiche insgesamt")
        XCTAssertEqual(entries[0].counts.inventory, 4)
        XCTAssertEqual(entries[0].level, .s1)
        XCTAssertEqual(entries[0].outcome, .completed)
        XCTAssertEqual(entries[0].round, .data)
        XCTAssertEqual(entries[0].contextReason, .inventory)
    }

    // MARK: Zusammenspiel mit der Sitzung

    private func lines(_ audit: AssistantAuditLog) -> [[String: Any]] {
        ((try? String(contentsOf: audit.url, encoding: .utf8)) ?? "").split(separator: "\n").compactMap {
            (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any]
        }
    }

    /// `start` steht in der Datei, bevor das Modell läuft und bevor das erste Ereignis hinausgeht.
    func testStartLineIsOnTheDiskBeforeTheModelRunsAndBeforeAnyOutput() async {
        let audit = AssistantAuditLog(directory: dir)
        let snapshots = Snapshots()
        let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: snapshots, pieces: ["Quasselbert wandert nach Pappelhausen."]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        var atFirstEvent: [[String: Any]] = []
        var first = true
        for await _ in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            if first {
                first = false
                atFirstEvent = lines(audit)
            }
        }
        XCTAssertEqual(snapshots.all.count, 1)
        let atModel = snapshots.all.first ?? ""
        // Beim Modellaufruf stehen genau `start`, `round` und `areas` in der Datei (der Kern wartet auf das Schreiben jeder dieser
        // Zeilen), nichts von der Antwort selbst.
        let kinds = atModel.split(separator: "\n").compactMap {
            ((try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any])?["t"] as? String
        }
        XCTAssertEqual(kinds, ["state", "start", "round", "areas"], "beim Modellaufruf: \(atModel)")
        XCTAssertEqual(atFirstEvent.compactMap { $0["t"] as? String }.prefix(2).map { $0 }, ["state", "start"])
        XCTAssertFalse(atFirstEvent.isEmpty, "beim ersten Ereignis ist `start` geschrieben")
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
    }

    /// Die Einwilligungsversion kommt JETZT aus der Freigabe, nicht aus der Konfiguration der Sitzung.
    func testStartLineCarriesTheConsentVersionOfTheReleaseNotOfTheConfig() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: config, audit: audit, released: true)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Wie hoch ist der Mont Blanc?")) {}
        await session.commit(requestId: "a", user: "Wie hoch ist der Mont Blanc?", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        await session.setAllowUnsourced(true, consentVersion: 3)
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Wie hoch ist der Mont Blanc?")) {}
        await session.commit(requestId: "b", user: "Wie hoch ist der Mont Blanc?", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        await session.setAllowUnsourced(false)
        for await _ in session.events(AssistantRequest(requestId: "c", text: "Hallo")) {}
        await session.commit(requestId: "c", user: "Hallo", spoken: "", interrupted: true)
        await session.setAllowUnsourced(true, consentVersion: 4)
        for await _ in session.events(AssistantRequest(requestId: "d", text: "Hallo")) {}
        await session.commit(requestId: "d", user: "Hallo", spoken: "", interrupted: true)
        let all = await audit.entries().filter { $0.kind == .answer }   // die Erhöhungen stehen als Einträge `release` dazwischen
        XCTAssertEqual(all.map(\.requestId), ["a", "b", "c", "d"])
        XCTAssertEqual(all.map(\.consentVersion), [1, 3, 0, 4])
        XCTAssertEqual(all.map(\.unsourcedMode), [true, true, false, true])
        // Die Einschränkung (Opt-in aus, c) schreibt eine Zeile `release` mit der Generation, die Erweiterung (Opt-in an, d) die
        // Zeilen `intent` (ohne Generation) und `effective` (mit der Generation des Wirksamwerdens).
        let releases = await audit.entries().filter { $0.kind == .release }
        let intents = await audit.entries().filter { $0.kind == .intent }
        // Ein Wechsel der Einwilligungsversion bei unverändertem Opt-in (b) ist keine Erhöhung und schreibt keine Zeile.
        XCTAssertEqual(releases.map(\.cause), [.optIn])
        XCTAssertEqual(releases.map(\.consentVersion), [0])
        XCTAssertEqual(intents.map(\.cause), [.optIn])
        XCTAssertEqual(intents.map(\.consentVersion), [4])
        XCTAssertEqual(intents.map(\.effect), [.effective])
        XCTAssertEqual(intents.compactMap(\.effectiveGeneration).count, 1)
        XCTAssertEqual(all.last?.generation, intents.first?.effectiveGeneration, "die Anfrage d startet auf der Generation der Erweiterung")
    }

    func testStartLineCarriesContextStateReasonAndRoundKind() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Wie hoch ist der Mont Blanc?")) {}
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {}
        for await _ in session.events(AssistantRequest(requestId: "c", text: "Wie hoch ist die Zugspitze?")) {}
        for id in ["a", "b", "c"] { await session.commit(requestId: id, user: "x", spoken: "", interrupted: true) }
        let all = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(all.map(\.contextMode), [.free, .free, .data], "Zustand vor der Rundenwahl")
        XCTAssertEqual(all.map(\.contextReason), [.free, .relevantPassage, .locked])
        XCTAssertEqual(all.map(\.round), [.general, .data, .data], "nach dem Einrasten bleibt jede Runde eine Datenrunde")
    }

    /// Nach 200 Schritten endet die Antwort mit dem Ausgang „begrenzt“; es gibt höchstens 200 Schrittzeilen, nichts wird gekappt.
    func testTheAnswerEndsAfter200StepsWithOutcomeLimited() async {
        let audit = AssistantAuditLog(directory: dir)
        let many = (0..<250).map { "Satz Nummer \($0) handelt von etwas ganz anderem." }.joined(separator: " ")
        let strict = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        let session = AssistantSession(model: AuditModel(pieces: [many]), knowledge: await geheimKnowledge(), config: strict, audit: audit, released: true)
        var events: [AssistantEvent] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Was ist mit den Sätzen?")) { events.append(e) }
        await session.commit(requestId: "r1", user: "Was ist mit den Sätzen?", spoken: "", interrupted: false)
        XCTAssertEqual(AssistantAuditLog.maxSteps, 200)
        let all = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(all.count, 1)
        XCTAssertEqual(all[0].outcome, .limited)
        XCTAssertEqual(all[0].trace.count, AssistantAuditLog.maxSteps)
        XCTAssertEqual(all[0].trace.map(\.n), Array(1...200))
        XCTAssertEqual(all[0].counts.droppedNoMode, 200, "ohne Opt-in fällt jeder unbelegte Satz mit Grund „kein Modus“")
        XCTAssertEqual(lines(audit).filter { $0["t"] as? String == "step" }.count, 200)
        if case .done? = events.last {} else { XCTFail("die Antwort endet regulär, nicht mit einem Fehler: \(String(describing: events.last))") }
    }

    func testInterruptedRevokedAndFailedOutcomes() async {
        let audit = AssistantAuditLog(directory: dir)
        let cloud = AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 16_000, maxAnswerTokens: 700, userName: nil, timeout: 5,
                                    allowUnsourced: true, consentVersion: 1)
        let session = AssistantSession(model: AuditModel(pieces: ["Kieselsteine sind meistens rund."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: cloud, audit: audit, released: true)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        await session.commit(requestId: "a", user: "Hallo", spoken: "Kieselsteine sind meistens rund.", interrupted: true)
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Hallo")) {}
        await session.commit(requestId: "b", user: "Hallo", spoken: "Kieselsteine sind meistens rund.", interrupted: true, revoked: true)
        let failing = AssistantSession(model: FailingModel(), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: cloud, audit: audit, released: true)
        for await _ in failing.events(AssistantRequest(requestId: "c", text: "Hallo")) {}
        await failing.commit(requestId: "c", user: "Hallo", spoken: "", interrupted: false)
        let all = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(all.map(\.outcome), [.interrupted, .revoked, .failed])
        XCTAssertEqual(all.map(\.cloud), [true, true, true])
        XCTAssertEqual(all.map(\.round), [.general, .general, .general])
    }

    /// Auch eine Anfrage ohne gesprochenen Text bekommt ihr abgeschlossenes Protokoll (Fehler, Abbruch, Widerruf).
    func testEntryExistsEvenWithoutSpokenText() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Hallo."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5),
                                       audit: audit, released: true)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        await session.commit(requestId: "a", user: "Hallo", spoken: "", interrupted: true)
        let all = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(all.count, 1)
        XCTAssertEqual(all[0].outcome, .interrupted)
        XCTAssertNil(all[0].history)
        let history = await session.history
        XCTAssertEqual(history, [], "ohne Gesprochenes kein Beitrag")
    }

    /// Die Zähler des Protokolls (aus den Schritten gefaltet) sind genau die des Laufs.
    func testCountsFoldedFromTheStepsEqualTheCountsOfTheRun() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Deine Notiz ist falsch. Quasselbert wandert nach Pappelhausen. Kieselsteine sind rund."]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        var done: AssistantAnswer?
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            if case .done(_, let a) = e { done = a }
        }
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: false)
        let entries = await audit.entries().filter { $0.kind == .answer }
        XCTAssertNotNil(done)
        XCTAssertEqual(entries.first?.counts, done?.counts)
        XCTAssertGreaterThan(entries.first?.counts.droppedOwnData ?? 0, 0)
        XCTAssertEqual(entries.first?.counts.grounded, 1)
    }

    /// Der Controller meldet Anzeige und Audiostart je Ereignis; die Meldungen landen als `commit` am richtigen Schritt.
    func testControllerCommitMarksLandOnTheirStepsAndUnknownIndexesAreIgnored() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        var spoken: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        session.noteCommit(requestId: "r1", index: 1, stage: .display, ok: true, generation: g)
        session.noteCommit(requestId: "r1", index: 1, stage: .audio, ok: false, generation: g + 1)
        session.noteCommit(requestId: "r1", index: 99, stage: .display, ok: true, generation: g)
        await session.commit(requestId: "r1", user: "x", spoken: spoken.joined(separator: " "), interrupted: false)
        let e = await audit.entries().filter { $0.kind == .answer }.first
        XCTAssertEqual(e?.trace.map(\.verdict), [.announcement, .grounded])
        XCTAssertEqual(e?.trace[1].commits, [
            AssistantCommitMark(stage: .emit, ok: true, generation: g), AssistantCommitMark(stage: .display, ok: true, generation: g),
            AssistantCommitMark(stage: .audio, ok: false, generation: g + 1),
            AssistantCommitMark(stage: .history, ok: true, generation: g),
        ])
        XCTAssertEqual(e?.trace[0].commits, [AssistantCommitMark(stage: .emit, ok: true, generation: g),
                                             AssistantCommitMark(stage: .history, ok: false, generation: g)],
                       "die Ansage steht nicht im Beitrag: Verlauf `h-`")
    }

    /// Ein Absturz mitten in der Antwort hinterlässt `start` und die bis dahin angefallenen Schritte; das nächste Öffnen schließt
    /// die Anfrage mit „unvollständig“. (Der Absturz wird nachgestellt: Stand der Datei bei laufender Antwort kopieren.)
    func testACrashMidAnswerLeavesStartAndStepsAndTheNextOpenClosesItAsIncomplete() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: HangingModel(pieces: ["Quasselbert wandert nach Pappelhausen und trifft dort Tante Lavendula. Danach"]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        let task = Task { () -> Int in
            var n = 0
            for await _ in session.events(AssistantRequest(requestId: "r1", text: question)) { n += 1 }
            return n
        }
        // Warten, bis Vorspann und Zitat geschrieben sind (die Antwort hängt danach).
        var snapshot: [[String: Any]] = []
        for _ in 0..<200 {
            snapshot = lines(audit)
            if snapshot.filter({ $0["t"] as? String == "step" }).count >= 2 { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        let crashed = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-crash-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: crashed) }
        try FileManager.default.createDirectory(at: crashed, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: audit.url, to: crashed.appendingPathComponent(AssistantAuditLog.fileName))
        await session.cancel(requestId: "r1")
        await session.commit(requestId: "r1", user: question, spoken: "", interrupted: true)
        _ = await task.value

        XCTAssertEqual(Array(snapshot.compactMap { $0["t"] as? String }.prefix(2)), ["state", "start"], "zuerst der Stand beim Öffnen, dann `start`")
        XCTAssertFalse(snapshot.contains { $0["t"] as? String == "end" }, "beim Absturz gab es kein `end`")
        let reopened = AssistantAuditLog(directory: crashed, reader: AssistantAuditWriter.readFile)
        _ = await reopened.append(journalEvent(1))
        let read = await reopened.read()
        XCTAssertEqual(read.entries.first(where: { $0.kind == .answer })?.requestId, "r1")
        XCTAssertEqual(read.entries.first(where: { $0.kind == .answer })?.outcome, .incomplete)
        XCTAssertGreaterThanOrEqual(read.entries.first(where: { $0.kind == .answer })?.trace.count ?? 0, 2)
        XCTAssertEqual(read.entries.first(where: { $0.kind == .answer })?.round, .data)
    }

    // MARK: Runde 4: `start` vor dem Abruf, `round` und `areas` als eigene Zeilen

    private func kinds(_ audit: AssistantAuditLog) -> [String] { lines(audit).compactMap { $0["t"] as? String } }

    /// Kopie der Datei, wie sie bei einem Prozessende dagelegen hätte.
    private func crashCopy(of audit: AssistantAuditLog) throws -> URL {
        let crashed = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-crash-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: crashed, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: audit.url, to: crashed.appendingPathComponent(AssistantAuditLog.fileName))
        return crashed
    }

    /// Das Prozessende mitten im Abruf (die Quelle hängt) hinterlässt `start` ohne Rundenart und Bereiche; das nächste Öffnen
    /// schließt die Anfrage als unvollständig.
    func testProcessEndDuringRetrieveLeavesStartAndTheNextOpenClosesItAsIncomplete() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let knowledge = AssistantKnowledge(corpus: AssistantCorpus(), sources: [HangingSource(area: .notes)])
        let session = AssistantSession(model: AuditModel(pieces: ["Hallo."]), knowledge: knowledge, config: config, audit: audit, released: true)
        let task = Task { () -> Int in
            var n = 0
            for await _ in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) { n += 1 }
            return n
        }
        var seen: [String] = []
        for _ in 0..<200 {
            seen = kinds(audit)
            if !seen.isEmpty { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        try await Task.sleep(nanoseconds: 150_000_000)   // der Abruf hängt jetzt; es darf nichts nachkommen
        let crashed = try crashCopy(of: audit)
        defer { try? FileManager.default.removeItem(at: crashed) }
        await session.cancel(requestId: "r1")
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
        _ = await task.value

        XCTAssertEqual(seen, ["state", "start"], "VOR dem Abruf stehen der Stand beim Öffnen und `start` in der Datei, sonst nichts (keine Rundenart, keine Bereiche)")
        let reopened = AssistantAuditLog(directory: crashed, reader: AssistantAuditWriter.readFile)
        _ = await reopened.append(journalEvent(1))
        let entry = await reopened.read().entries.first(where: { $0.kind == .answer })
        XCTAssertEqual(entry?.requestId, "r1")
        XCTAssertEqual(entry?.outcome, .incomplete)
        XCTAssertNil(entry?.round, "die Rundenwahl war noch nicht gefallen")
        XCTAssertEqual(entry?.contextMode, .free, "Kontextzustand vorher steht schon im `start`")
        XCTAssertEqual(entry?.areas, [])
        XCTAssertEqual(entry?.unsourcedMode, true)
        XCTAssertEqual(entry?.consentVersion, 1)
    }

    /// Die Rundenart steht als eigene Zeile `round` hinter `start` (Art und Grund); im `start` selbst fehlt sie.
    func testRoundDecisionIsItsOwnLineAfterStart() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        var spoken: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        await session.commit(requestId: "r1", user: "x", spoken: spoken.joined(separator: " "), interrupted: false)
        let objs = try objects()
        XCTAssertEqual(objs.first?["t"] as? String, "state", "der Stand beim Öffnen steht vor der ersten Anfrage")
        let start = try XCTUnwrap(objs.first { $0["t"] as? String == "start" })
        XCTAssertNil(start["round"], "die Rundenart steht nicht im `start`")
        XCTAssertNil(start["reason"])
        XCTAssertEqual(start["ctx"] as? String, "free", "Kontextzustand vorher")
        let round = try XCTUnwrap(objs.first { $0["t"] as? String == "round" })
        XCTAssertEqual(round["round"] as? String, "data")
        XCTAssertNotNil(round["reason"])
        let order = objs.compactMap { $0["t"] as? String }
        XCTAssertLessThan(try XCTUnwrap(order.firstIndex(of: "start")), try XCTUnwrap(order.firstIndex(of: "round")))
        XCTAssertLessThan(try XCTUnwrap(order.firstIndex(of: "round")), try XCTUnwrap(order.firstIndex(of: "step")))
        let entry = await audit.entries().filter { $0.kind == .answer }.first
        XCTAssertEqual(entry?.round, .data)
        XCTAssertNotNil(entry?.contextReason)
        XCTAssertEqual(entry?.contextMode, .free)
    }

    /// Absturz vor `end`: Rundenart und Bereiche stehen in eigenen Zeilen und bleiben erhalten; ohne Öffnen faltet die Ansicht sie,
    /// beim Öffnen trägt das Wiederherstellungs-Ende die Bereiche.
    func testACrashBeforeEndKeepsRoundAndAreasAndTheRecoveryEndFoldsThem() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: HangingModel(pieces: ["Quasselbert wandert nach Pappelhausen und trifft dort Tante Lavendula. Danach"]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        let task = Task { () -> Int in
            var n = 0
            for await _ in session.events(AssistantRequest(requestId: "r1", text: question)) { n += 1 }
            return n
        }
        var seen: [String] = []
        for _ in 0..<200 {
            seen = kinds(audit)
            if seen.filter({ $0 == "step" }).count >= 2 { break }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        let crashed = try crashCopy(of: audit)
        defer { try? FileManager.default.removeItem(at: crashed) }
        await session.cancel(requestId: "r1")
        await session.commit(requestId: "r1", user: question, spoken: "", interrupted: true)
        _ = await task.value

        XCTAssertFalse(seen.contains("end"), "beim Absturz gab es kein `end`")
        XCTAssertEqual(Array(seen.prefix(2)), ["state", "start"])
        XCTAssertTrue(seen.contains("round") && seen.contains("areas"), "Rundenart und Bereiche stehen schon vor dem Ende: \(seen)")
        // Nur lesen (kein Öffnen zum Schreiben): die Faltung nimmt Rundenart und Bereiche aus den Zeilen.
        let viewOnly = await AssistantAuditLog(directory: crashed, reader: AssistantAuditWriter.readFile).read().entries.first(where: { $0.kind == .answer })
        XCTAssertEqual(viewOnly?.outcome, .incomplete)
        XCTAssertEqual(viewOnly?.round, .data)
        XCTAssertEqual(viewOnly?.areas, ["books"])
        // Öffnen zum Schreiben: das Wiederherstellungs-Ende faltet die Bereiche in seine eigene Zeile.
        let reopened = AssistantAuditLog(directory: crashed, reader: AssistantAuditWriter.readFile)
        _ = await reopened.append(journalEvent(1))
        let ends = try objects(crashed.appendingPathComponent(AssistantAuditLog.fileName)).filter { $0["t"] as? String == "end" }
        XCTAssertEqual(ends.count, 1)
        XCTAssertEqual(ends.first?["areas"] as? [String], ["books"])
        XCTAssertEqual(ends.first?["outcome"] as? String, "incomplete")
        let entry = await reopened.read().entries.first(where: { $0.kind == .answer })
        XCTAssertEqual(entry?.round, .data)
        XCTAssertEqual(entry?.areas, ["books"])
        XCTAssertEqual(entry?.outcome, .incomplete)
    }

    /// Bestandsangaben: Die Bereiche stehen als Zeile `areas` in der Datei, bevor die erste Angabe hinausgeht. Gelesen wird die rohe
    /// Datei (kein Umweg über `audit.read()`, das die Schreibwarteschlange leerte und so jedes Einreihen wie ein Schreiben
    /// aussehen ließe); der Schreiber ist langsam: Nur wer auf das Schreiben wartet, besteht.
    func testInventoryAreasAreWrittenBeforeTheStatementsGoOut() async throws {
        let audit = slowLog()
        let session = AssistantSession(model: AuditModel(pieces: []), knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        var firstSentenceSeen = false
        var atFirstSentence: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Was habe ich gespeichert?")) {
            if case .sentence = e, !firstSentenceSeen {
                firstSentenceSeen = true
                atFirstSentence = kinds(audit)
            }
        }
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
        XCTAssertTrue(firstSentenceSeen)
        XCTAssertEqual(Array(atFirstSentence.prefix(4)), ["state", "start", "round", "areas"],
                       "schon beim ersten Satz stehen state, start, round und areas in der Datei: \(atFirstSentence)")
        let order = try objects().compactMap { $0["t"] as? String }
        let areasIndex = try XCTUnwrap(order.firstIndex(of: "areas"))
        let stepIndex = try XCTUnwrap(order.firstIndex(of: "step"))
        XCTAssertLessThan(areasIndex, stepIndex, "die Bereiche stehen vor den Schritten, nicht erst im `end`")
        let entry = await audit.entries().filter { $0.kind == .answer }.first
        XCTAssertTrue(Set(["books", "notes"]).isSubset(of: Set(entry?.areas ?? [])), "Bestandsbereiche: \(entry?.areas ?? [])")
    }

    // MARK: Runde 5: alle Zeilen vor dem Modell, jede Erhöhung eine Zeile

    /// Langsamer Schreiber (je Zeile 40 ms auf der Platte): Beim Modellaufruf stehen nur dann alle Zeilen in der Datei, wenn der
    /// Kern auf das Schreiben WARTET, nicht nur einreiht.
    private func slowLog() -> AssistantAuditLog {
        freshLog(appender: { url, data in
            Thread.sleep(forTimeInterval: 0.04)
            try AssistantAuditWriter.appendFile(url, data)
        })
    }

    /// Welche Zeilenarten stehen in der rohen Datei, wenn das Modell aufgerufen wird?
    private func kindsAtModelCall(cloud: Bool, question: String) async -> [String] {
        let audit = slowLog()
        await audit.clear()
        let snapshots = Snapshots()
        let cfg = AssistantConfig(modelLabel: cloud ? "Cloud" : "Test", isCloud: cloud, contextTokens: 4_096, maxAnswerTokens: 400,
                                  userName: nil, timeout: 5, allowUnsourced: true, consentVersion: 1)
        let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: snapshots, pieces: ["Quasselbert wandert nach Pappelhausen."]),
                                       knowledge: await geheimKnowledge(), config: cfg, audit: audit, released: true)
        for await _ in session.events(AssistantRequest(requestId: "r1", text: question)) {}
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
        return (snapshots.all.first ?? "").split(separator: "\n").compactMap {
            ((try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any])?["t"] as? String
        }
    }

    /// Datenrunde, lokales Modell: beim Modellaufruf stehen genau `state`, `start`, `round` und `areas` in der Datei (nichts von der Antwort).
    func testStartRoundAndAreasAreInTheFileBeforeALocalModelRuns() async {
        let kinds = await kindsAtModelCall(cloud: false, question: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")
        XCTAssertEqual(kinds, ["state", "start", "round", "areas"])
    }

    /// Datenrunde, Cloud-Modell: ebenso.
    func testStartRoundAndAreasAreInTheFileBeforeACloudModelRuns() async {
        let kinds = await kindsAtModelCall(cloud: true, question: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")
        XCTAssertEqual(kinds, ["state", "start", "round", "areas"])
    }

    /// Runde ohne Quelle (keine Bereiche): `start` und `round` stehen in der Datei, bevor das Modell läuft; es gibt keine Zeile `areas`.
    func testStartAndRoundAreInTheFileBeforeAModelRunsInAGeneralRound() async {
        let local = await kindsAtModelCall(cloud: false, question: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(local, ["state", "start", "round"])
        let cloud = await kindsAtModelCall(cloud: true, question: "Wie hoch ist der Mont Blanc?")
        XCTAssertEqual(cloud, ["state", "start", "round"])
    }

    // MARK: Runde 5: jede Erhöhung der Generation genau eine Zeile `release`, vom Kern an der Stelle der Erhöhung

    /// Die Zeilen `release` der rohen Datei (nach dem Warten auf das Schreiben der Zeilen ohne Anfrage).
    private func releaseLines(_ audit: AssistantAuditLog) async -> [[String: Any]] {
        _ = await audit.flushReleaseLines()
        return lines(audit).filter { $0["t"] as? String == "release" }
    }

    private func releaseSession(_ audit: AssistantAuditLog?, history: [AssistantTurn] = []) async -> AssistantSession {
        AssistantSession(model: AuditModel(pieces: []), knowledge: await geheimKnowledge(), config: config, history: history, audit: audit,
                         released: true)
    }

    /// Die Datei, wie sie JETZT ist (ohne auf die Warteschlange zu warten): Was hier steht, ist dauerhaft geschrieben.
    private func rawReleaseLines(_ audit: AssistantAuditLog) -> [[String: Any]] {
        lines(audit).filter { $0["t"] as? String == "release" }
    }

    /// Zeilen der Klasse A einer Art aus der Datei, wie sie jetzt ist (`intent` = beabsichtigt, `effective` = wirksam).
    private func rawLines(_ audit: AssistantAuditLog, _ kind: String) -> [[String: Any]] {
        lines(audit).filter { $0["t"] as? String == kind }
    }

    /// Wendet alle Änderungen an und sichert sie danach in der Reihenfolge ihrer Änderungsnummern (die Reihe läuft streng nacheinander).
    private func settleAll(_ session: AssistantSession, _ pendings: [AssistantReleasePending]) async {
        let ordered = pendings.filter { $0.changed }.sorted { ($0.applied.steps.first?.number ?? 0) < ($1.applied.steps.first?.number ?? 0) }
        for p in ordered { _ = await session.settle(p) }
    }

    /// Pfad `setAllowUnsourced` (Opt-in; Sitzung mit `released: true`, Opt-in war an): je echte Änderung eine Zeile (Einschränkung)
    /// bzw. zwei (Erweiterung: beabsichtigt, wirksam), Grund `optIn`; ohne Änderung weder Erhöhung noch Zeile. Die Zeilen stehen
    /// schon in der Datei, wenn der Aufruf zurückkehrt (I2).
    func testSetAllowUnsourcedWritesOneLineWithCauseOptInAndNoneWithoutAChange() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let s = await releaseSession(audit)
        let off = await s.setAllowUnsourced(false)
        XCTAssertEqual(rawReleaseLines(audit).compactMap { $0["gen"] as? Int }, [off.generation], "Zeile steht bei der Rückkehr in der Datei")
        let r1 = await s.setAllowUnsourced(false)
        XCTAssertEqual(r1.generation, off.generation, "keine Änderung: keine Erhöhung")
        XCTAssertFalse(r1.changed)
        let on = await s.setAllowUnsourced(true, consentVersion: 2)
        XCTAssertTrue(on.effective)
        // Die Einschränkung schreibt `release` (Generation beim Anwenden), die Erweiterung `intent` (ohne Generation) und danach
        // `effective` (mit der Generation des Wirksamwerdens): jeweils genau eine Zeile je Änderung.
        let rel = rawReleaseLines(audit)
        let intent = rawLines(audit, "intent")
        let effective = rawLines(audit, "effective")
        XCTAssertEqual(rel.count, 1, "eine Einschränkung, eine Zeile `release`")
        XCTAssertEqual(intent.count, 1)
        XCTAssertEqual(effective.count, 1)
        XCTAssertEqual(rel.compactMap { $0["gen"] as? Int }, [off.generation])
        XCTAssertNil(intent[0]["gen"], "die Zeile „beabsichtigt“ trägt keine Generation")
        XCTAssertEqual(effective[0]["gen"] as? Int, on.generation)
        XCTAssertEqual(intent[0]["chg"] as? Int, effective[0]["chg"] as? Int, "beide Zeilen gehören zu derselben Änderung")
        XCTAssertNotNil(intent[0]["run"] as? Int)
        XCTAssertEqual(intent[0]["run"] as? Int, effective[0]["run"] as? Int)
        XCTAssertEqual(rel[0]["cause"] as? String, "optIn")
        XCTAssertEqual(intent[0]["cause"] as? String, "optIn")
        XCTAssertEqual(effective[0]["cause"] as? String, "optIn")
        XCTAssertEqual(rel[0]["unsourced"] as? Bool, false, "Opt-in-Zustand NACH der Änderung")
        XCTAssertEqual(effective[0]["unsourced"] as? Bool, true)
        XCTAssertEqual(rel[0]["consent"] as? Int, 0)
        XCTAssertEqual(effective[0]["consent"] as? Int, 2)
        XCTAssertEqual(rel[0]["areas"] as? [String], ["books", "files", "notes", "transcripts"], "Klasse A: die Bereichsnamen des Stands (feste Kennungen)")
        XCTAssertEqual(rel[0]["model"] as? String, "other")
        XCTAssertEqual(rel[0]["cloud"] as? Bool, false)
        XCTAssertNil(rel[0]["req"], "eine Erhöhung gehört zu keiner Anfrage")
        XCTAssertEqual(on.generation, s.releaseGeneration)
        XCTAssertTrue(s.allowsUnsourced)
    }

    /// Pfad `setAreas`: EINE Erhöhung und EINE Zeile je Änderung, mit den NEUEN Bereichsnamen (sortiert); gleiche Bereiche: nichts.
    func testSetAreasWritesExactlyOneLineWithTheNewAreaNames() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let s = await releaseSession(audit)
        let before = s.releaseGeneration
        let first = await s.setAreas([.notes, .books])
        XCTAssertEqual(first.generation, before + 1, "genau eine Erhöhung")
        XCTAssertTrue(first.effective)
        XCTAssertEqual(s.effectiveAreas, [.books, .notes])
        XCTAssertEqual(rawReleaseLines(audit).compactMap { $0["gen"] as? Int }, [before + 1])
        let same = await s.setAreas([.books, .notes])
        XCTAssertFalse(same.changed, "gleiche Bereiche: keine Erhöhung, keine Zeile")
        _ = await s.setAreas([])
        let rel = rawReleaseLines(audit)
        XCTAssertEqual(rel.count, 2)
        XCTAssertEqual(rel.compactMap { $0["cause"] as? String }, ["areas", "areas"])
        XCTAssertEqual(rel.compactMap { $0["areas"] as? [String] }, [["books", "notes"], []], "alle aus: leere Liste, kein fehlender Schlüssel")
        // Die Ansicht faltet die Zeilen zu Einträgen der Art `release`.
        let entries = await audit.entries().filter { $0.kind == .release }
        XCTAssertEqual(entries.map(\.cause), [.areas, .areas])
        XCTAssertEqual(entries.map(\.generation), [before + 1, before + 2])
        XCTAssertEqual(entries.first?.areas, ["books", "notes"])
        XCTAssertEqual(entries.last?.areas, [])
    }

    /// Pfad `releaseDidChange` / Cloud-Zustimmung: eine Zeile mit dem gegebenen Grund; die Zustimmung steht als `granted` in der Zeile.
    func testReleaseDidChangeAndCloudConsentWriteOneLineWithTheGivenCause() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let s = await releaseSession(audit)
        let a = await s.releaseDidChange().generation
        let b = await s.releaseDidChange(.consent).generation
        let c = await s.setCloudConsent(false)
        XCTAssertTrue(c.changed)
        XCTAssertFalse(s.effectiveCloudConsent, "Entzug wirkt sofort")
        let again = await s.setCloudConsent(false)
        XCTAssertFalse(again.changed, "keine Änderung: keine Zeile")
        let d = await s.setCloudConsent(true)
        XCTAssertTrue(d.effective)
        XCTAssertTrue(s.effectiveCloudConsent)
        let rel = rawReleaseLines(audit)
        XCTAssertEqual(rel.compactMap { $0["gen"] as? Int }, [a, b, c.generation])
        XCTAssertEqual(rel.compactMap { $0["cause"] as? String }, ["other", "consent", "consent"])
        XCTAssertEqual(rel.compactMap { $0["granted"] as? Bool }, [true, true, false])
        XCTAssertEqual(rel[0]["areas"] as? [String], ["books", "files", "notes", "transcripts"])
        // Die Erteilung ist eine Erweiterung: `intent` ohne Generation, dann `effective` mit der Generation des Wirksamwerdens.
        let intent = rawLines(audit, "intent")
        let effective = rawLines(audit, "effective")
        XCTAssertEqual(intent.count, 1)
        XCTAssertNil(intent.first?["gen"])
        XCTAssertEqual(effective.compactMap { $0["gen"] as? Int }, [d.generation])
        XCTAssertEqual(effective.first?["cause"] as? String, "consent")
        XCTAssertEqual(effective.first?["granted"] as? Bool, true)
    }

    /// Pfad `resumeFree`: eine Zeile mit Grund `resumeFree`; ein zweiter Aufruf (nichts zu tun) erhöht nicht und schreibt nichts.
    func testResumeFreeWritesOneLineWithCauseResumeFree() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let s = await releaseSession(audit, history: [AssistantTurn(user: "Frage", assistant: "Antwort", at: Date(), round: .data)])
        XCTAssertEqual(s.contextMode, .data)
        let r2 = await s.resumeFree()
        XCTAssertTrue(r2.changed)
        XCTAssertTrue(r2.effective)
        XCTAssertEqual(s.contextMode, .free)
        let r3 = await s.resumeFree()
        XCTAssertFalse(r3.changed)
        let rel = rawReleaseLines(audit)
        XCTAssertEqual(rel.count, 1)
        XCTAssertEqual(rel.first?["cause"] as? String, "resumeFree")
        XCTAssertEqual(rel.first?["gen"] as? Int, s.releaseGeneration)
        XCTAssertEqual(rel.first?["unsourced"] as? Bool, true)
    }

    /// Alle Pfade zugleich aus mehreren Threads: Jede Generation 1 … n steht in genau einer Zeile (`release` bei Einschränkung und
    /// Ereignis, `effective` bei Erweiterung), keine fehlt, keine doppelt. Die Generation steigt im Lock (Einschränkung beim
    /// Anwenden, Erweiterung beim Wirksamwerden); die Zeile wird erst beim Sichern geschrieben, streng nacheinander.
    func testEveryIncreaseLeavesExactlyOneLineInTheOrderOfTheGenerations() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let s = await releaseSession(audit)
        let collected = NSLock()
        nonisolated(unsafe) var pendings: [AssistantReleasePending] = []
        let group = DispatchGroup()
        for t in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                for i in 0..<60 {
                    let p: AssistantReleasePending
                    switch (t + i) % 3 {
                    case 0: p = s.applyReleaseChange()
                    case 1: p = s.applyReleaseChange(.consent)
                    default: p = s.applyAllowUnsourced(i % 2 == 0)
                    }
                    collected.lock()
                    pendings.append(p)
                    collected.unlock()
                }
                group.leave()
            }
        }
        XCTAssertEqual(group.wait(timeout: .now() + 20), .success)
        let all = collected.withLock { pendings }
        await settleAll(s, all)
        let n = s.releaseGeneration
        XCTAssertGreaterThan(n, 100)
        _ = await audit.flushReleaseLines()
        let generations = lines(audit).filter { ["release", "effective"].contains($0["t"] as? String ?? "") }.compactMap { $0["gen"] as? Int }
        XCTAssertEqual(generations.count, n, "eine Zeile je Erhöhung")
        XCTAssertEqual(generations.sorted(), Array(1...n), "jede Generation genau einmal, ohne Lücke")
        let opening = lines(audit).filter { ["release", "intent"].contains($0["t"] as? String ?? "") }.compactMap { $0["chg"] as? Int }
        XCTAssertEqual(Set(opening).count, opening.count, "jede Änderungsnummer eröffnet genau eine Zeile (`release` oder `intent`)")
        let closing = lines(audit).filter { $0["t"] as? String == "effective" }.compactMap { $0["chg"] as? Int }
        XCTAssertTrue(Set(closing).isSubset(of: Set(opening)), "jede Zeile `effective` gehört zu einer Zeile `intent`")
    }

    /// Das Anwenden (Einreihen) wartet nie auf die Platte: die Zeilen entstehen erst beim Sichern AUSSERHALB des Locks.
    func testRaisingTheGenerationDoesNotWaitForTheDisk() async throws {
        let audit = freshLog(appender: { url, data in
            Thread.sleep(forTimeInterval: 0.2)
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = await releaseSession(audit)
        let started = Date()
        var pendings: [AssistantReleasePending] = []
        for _ in 0..<5 { pendings.append(s.applyReleaseChange()) }
        XCTAssertLessThan(Date().timeIntervalSince(started), 0.3, "fünf Erhöhungen, jede Zeile braucht 0,2 s auf der Platte: es wird nicht gewartet")
        XCTAssertEqual(s.releaseGeneration, 5, "die Generation steht sofort")
        await settleAll(s, pendings)
        let rel = await releaseLines(audit)
        XCTAssertEqual(rel.count, 5, "alle Zeilen kommen trotzdem in der Datei an")
    }

    /// I2, alte Reihenfolge rot: Nur Einreihen (`apply…`, so lief der Rückgabewert früher) lässt die Zeile bei der Rückkehr noch
    /// aus der Datei fehlen; `setAllowUnsourced` (einreihen, dann AUSSERHALB des Locks warten) kehrt erst zurück, wenn sie steht.
    func testTheReleaseLineIsInTheFileWhenTheMethodReturnsNotJustEnqueued() async throws {
        let audit = freshLog(appender: { url, data in
            Thread.sleep(forTimeInterval: 0.3)
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = await releaseSession(audit)
        // Alte Reihenfolge (nur einreihen, sofort zurück): Die Zeile fehlt noch.
        let pending = s.applyAllowUnsourced(false)
        XCTAssertEqual(rawReleaseLines(audit).count, 0, "alte Reihenfolge: bei der Rückkehr steht die Zeile NICHT in der Datei")
        // Neue Reihenfolge: erst nach der dauerhaften Zeile zurück.
        let r = await s.settle(pending)
        XCTAssertNil(r.error)
        XCTAssertEqual(rawReleaseLines(audit).compactMap { $0["gen"] as? Int }, [pending.generation], "neu: die Zeile steht bei der Rückkehr")
        let areas = await s.setAreas([.books])
        XCTAssertEqual(rawReleaseLines(audit).compactMap { $0["gen"] as? Int }, [pending.generation, areas.generation])
        let consent = await s.setCloudConsent(false)
        XCTAssertEqual(rawReleaseLines(audit).last?["gen"] as? Int, consent.generation)
        let other = await s.releaseDidChange()
        XCTAssertEqual(rawReleaseLines(audit).last?["gen"] as? Int, other.generation)
    }

    /// Ein Schreibfehler bei einer Zeile `release` bleibt nicht still: Er kommt als `error` zurück. Eine Einschränkung gilt trotzdem
    /// (Widerruf wirkt auch ohne Zeile), eine Erweiterung nicht (mehr Bereiche, Opt-in an, Zustimmung erteilt).
    func testAWriteFailureOfAReleaseLineIsReportedRestrictionsHoldExpansionsDoNot() async throws {
        let broken = FailSwitch()
        let audit = freshLog(appender: { url, data in
            if broken.on { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = await releaseSession(audit)
        // Ausgangslage mit gesunder Platte: nur Bücher, Opt-in aus, keine Cloud-Zustimmung.
        _ = await s.setAreas([.books])
        _ = await s.setAllowUnsourced(false)
        _ = await s.setCloudConsent(false)
        broken.on = true
        // Erweiterungen scheitern: nicht wirksam, Fehler gemeldet.
        let areas = await s.setAreas([.books, .notes])
        XCTAssertEqual(areas.error, .writeFailed)
        XCTAssertFalse(areas.effective)
        XCTAssertEqual(s.effectiveAreas, [.books], "mehr Bereiche ohne dauerhafte Zeile: nicht wirksam")
        let optIn = await s.setAllowUnsourced(true, consentVersion: 1)
        XCTAssertEqual(optIn.error, .writeFailed)
        XCTAssertFalse(optIn.effective)
        XCTAssertFalse(s.allowsUnsourced, "Opt-in an ohne dauerhafte Zeile: nicht wirksam")
        XCTAssertEqual(s.contextMode, .data, "auch der Kontext wird nicht umgestellt")
        let consent = await s.setCloudConsent(true)
        XCTAssertEqual(consent.error, .writeFailed)
        XCTAssertFalse(consent.effective)
        XCTAssertFalse(s.effectiveCloudConsent, "Zustimmung ohne dauerhafte Zeile: nicht wirksam")
        // Einschränkungen gelten trotz Fehler.
        broken.on = false
        _ = await s.setAreas([.books, .notes])
        _ = await s.setAllowUnsourced(true, consentVersion: 1)
        _ = await s.setCloudConsent(true)
        XCTAssertEqual(s.effectiveAreas, [.books, .notes])
        XCTAssertTrue(s.allowsUnsourced)
        XCTAssertTrue(s.effectiveCloudConsent)
        broken.on = true
        let less = await s.setAreas([.books])
        XCTAssertEqual(less.error, .writeFailed)
        XCTAssertTrue(less.effective, "Einschränkung wirkt auch, wenn die Zeile nicht geschrieben werden kann")
        XCTAssertEqual(s.effectiveAreas, [.books])
        let off = await s.setAllowUnsourced(false)
        XCTAssertEqual(off.error, .writeFailed)
        XCTAssertTrue(off.effective)
        XCTAssertFalse(s.allowsUnsourced, "Widerruf wirkt auch ohne Zeile")
        let revoked = await s.setCloudConsent(false)
        XCTAssertEqual(revoked.error, .writeFailed)
        XCTAssertFalse(s.effectiveCloudConsent)
        // Der Fehler wurde mit der Rückgabe abgeholt: kein zweites Mal.
        let again = await audit.flushReleaseLines()
        XCTAssertNil(again)
    }

    /// Ohne Protokoll gibt es keine Zeile und keinen Fehler.
    func testWithoutAnAuditLogRaisingTheGenerationStillWorks() async {
        let s = await releaseSession(nil)
        let r4 = await s.releaseDidChange(.consent)
        XCTAssertEqual(r4.generation, 1)
        XCTAssertNil(r4.error)
        let r5 = await s.setAllowUnsourced(false)
        XCTAssertEqual(r5.generation, 2)
        let last = await s.setAreas([.books])
        XCTAssertEqual(last.generation, 3)
        XCTAssertTrue(last.effective)
    }

    /// Standzeile (E4.2b): Beim Öffnen des Protokolls steht der Freigabezustand vor der ersten Anfrage in der Datei.
    func testTheStateLineIsWrittenAtOpeningBeforeTheFirstRequest() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let s = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                 config: config, audit: audit, released: true, cloudConsent: false)
        _ = await audit.flushReleaseLines()
        let state = lines(audit).first { $0["t"] as? String == "state" }
        XCTAssertNotNil(state, "die Standzeile steht beim Öffnen in der Datei, ohne dass eine Anfrage lief")
        XCTAssertEqual(state?["unsourced"] as? Bool, true)
        XCTAssertEqual(state?["consent"] as? Int, 1)
        XCTAssertEqual(state?["areas"] as? [String], ["books", "files", "notes", "transcripts"])
        XCTAssertEqual(state?["granted"] as? Bool, false, "Cloud-Zustimmung: nein")
        XCTAssertEqual(state?["gen"] as? Int, 0)
        for await _ in s.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert?")) {}
        let all = lines(audit).compactMap { $0["t"] as? String }
        XCTAssertEqual(all.first, "state")
        XCTAssertLessThan(all.firstIndex(of: "state") ?? 99, all.firstIndex(of: "start") ?? -1, "state vor start")
        XCTAssertEqual(all.filter { $0 == "state" }.count, 1, "genau eine Standzeile")
        // Gefaltet: ein Eintrag der Art `state`.
        let entries = await audit.entries().filter { $0.kind == .state }
        XCTAssertEqual(entries.count, 1)
        XCTAssertEqual(entries.first?.cloudConsent, false)
        XCTAssertEqual(entries.first?.areas, ["books", "files", "notes", "transcripts"])
    }

    /// Eine Zeile `release` mit unbekanntem Grund (neuere Fassung) wird übersprungen und gezählt, der Rest bleibt lesbar.
    func testAReleaseLineWithAnUnknownCauseIsSkippedAndCounted() async throws {
        var ok = AssistantJournalLine(.release)
        ok.at = Date(timeIntervalSince1970: 1_760_000_000)
        ok.gen = 1
        ok.cause = .consent
        var raw = encoded(ok)
        raw.append(10)
        raw.append(Data(#"{"at":"2026-01-01T10:00:00Z","cause":"zauber","gen":2,"t":"release"}"#.utf8))
        raw.append(10)
        try writeRaw(raw)
        let read = await freshLog().read()
        XCTAssertEqual(read.entries.count, 1)
        XCTAssertEqual(read.entries.first?.cause, .consent)
        XCTAssertEqual(read.skippedLines, 1)
    }

    /// Eine Zeile `release` ohne Generation (Änderung ohne Sitzung) ist mit Lauf und Änderungsnummer lesbar; ohne beides ist sie es nicht.
    /// „Beabsichtigt“ und „wirksam“ ohne Generation ordnet der Leser zu; ohne „wirksam“ bleibt die Wirkung unbekannt.
    func testLinesWithoutAGenerationAreReadWhenTheyCarryRunAndChangeNumber() async throws {
        var released = AssistantJournalLine(.release)
        released.at = Date(timeIntervalSince1970: 1_760_000_000)
        released.cause = .areas
        released.areas = ["books"]
        released.run = 3
        released.chg = 1
        var bare = AssistantJournalLine(.release)
        bare.at = Date(timeIntervalSince1970: 1_760_000_001)
        bare.cause = .areas
        var intent = AssistantJournalLine(.intent)
        intent.at = Date(timeIntervalSince1970: 1_760_000_002)
        intent.cause = .areas
        intent.run = 3
        intent.chg = 2
        var effective = AssistantJournalLine(.effective)
        effective.at = Date(timeIntervalSince1970: 1_760_000_003)
        effective.cause = .areas
        effective.run = 3
        effective.chg = 2
        var lonely = intent
        lonely.chg = 3
        var raw = Data()
        for l in [released, bare, intent, effective, lonely] {
            raw.append(encoded(l))
            raw.append(10)
        }
        try writeRaw(raw)
        let read = await freshLog().read()
        XCTAssertEqual(read.skippedLines, 1, "nur die Zeile ohne Generation, Lauf und Änderungsnummer")
        let release = read.entries.filter { $0.kind == .release }
        XCTAssertEqual(release.count, 1)
        XCTAssertEqual(release.first?.generation, 0)
        XCTAssertEqual(release.first?.run, 3)
        XCTAssertEqual(release.first?.change, 1)
        XCTAssertEqual(release.first?.areas, ["books"])
        let intents = read.entries.filter { $0.kind == .intent }
        XCTAssertEqual(intents.map(\.effect), [.effective, .unknown])
        XCTAssertEqual(intents.map(\.effectiveGeneration), [nil, nil])
    }

    /// Zeilen `release` zählen beim Verdichten wie Ereignisse als eigene Einheiten (die letzten 500 bleiben).
    func testReleaseLinesCountAsUnitsWhenCompacting() async throws {
        var raw = Data()
        for n in 0..<300 {
            raw.append(encoded(journalStart("q\(n)")))
            raw.append(10)
            raw.append(encoded(journalEnd("q\(n)")))
            raw.append(10)
        }
        for g in 1...250 {
            var l = AssistantJournalLine(.release)
            l.at = Date(timeIntervalSince1970: 1_760_000_000 + Double(g))
            l.gen = g
            l.cause = .other
            raw.append(encoded(l))
            raw.append(10)
        }
        try writeRaw(raw)
        let log = freshLog()
        _ = await log.append(journalEvent(1))
        let read = await log.read()
        XCTAssertEqual(read.entries.count, 501)
        XCTAssertEqual(read.entries.first?.requestId, "q50")
        XCTAssertEqual(read.entries.filter { $0.kind == .release }.count, 250)
    }

    /// Dateien im bisherigen Format (Rundenart und Kontext im `start`, Bereiche nur im `end`) laden unverändert.
    func testLinesOfTheFormerFormatStillLoad() async throws {
        let raw = [
            #"{"at":"2026-01-01T10:00:00Z","cloud":false,"consent":1,"ctx":"free","gen":2,"model":"apple","reason":"ownData","req":"alt1","round":"data","session":"s0","t":"start","unsourced":true}"#,
            #"{"gen":2,"n":1,"req":"alt1","t":"step","verdict":"announcement"}"#,
            #"{"gen":2,"n":2,"req":"alt1","t":"step","verdict":"grounded"}"#,
            #"{"mark":"e+2","n":2,"req":"alt1","t":"commit"}"#,
            #"{"areas":["books"],"at":"2026-01-01T10:00:02Z","level":"s0","outcome":"completed","req":"alt1","t":"end"}"#,
        ].joined(separator: "\n") + "\n"
        try writeRaw(Data(raw.utf8))
        let audit = AssistantAuditLog(directory: dir)
        let read = await audit.read()
        XCTAssertEqual(read.skippedLines, 0, "keine Zeile des alten Formats wird übersprungen")
        let entry = try XCTUnwrap(read.entries.first)
        XCTAssertEqual(entry.requestId, "alt1")
        XCTAssertEqual(entry.round, .data)
        XCTAssertEqual(entry.contextMode, .free)
        XCTAssertEqual(entry.contextReason, .ownData)
        XCTAssertEqual(entry.areas, ["books"])
        XCTAssertEqual(entry.outcome, .completed)
        XCTAssertEqual(entry.trace.map(\.verdict), [.announcement, .grounded])
        // Neue Zeilen kommen hinter die alten, und die alte Anfrage bleibt, wie sie ist.
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        for await _ in session.events(AssistantRequest(requestId: "neu1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {}
        await session.commit(requestId: "neu1", user: "x", spoken: "", interrupted: true)
        let all = await audit.entries().filter { $0.kind == .answer }
        XCTAssertEqual(all.map(\.requestId), ["alt1", "neu1"])
        XCTAssertEqual(all.first?.areas, ["books"])
        XCTAssertEqual(all.first?.outcome, .completed)
        XCTAssertEqual(all.last?.round, .data)
    }

    // MARK: Schreibfolge (ADR-0028 E4.2b): fail-closed

    /// Austauschpunkt „Platte“: Zeilen der Art `kind` scheitern, solange der Schalter an ist; alles andere wird wirklich geschrieben.
    private func failing(_ kind: String, _ sw: FailSwitch) -> @Sendable (URL, Data) throws -> Void {
        { url, data in
            if sw.on, String(decoding: data, as: UTF8.self).contains("\"t\":\"\(kind)\"") { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        }
    }

    private func sentenceCount(_ events: [AssistantEvent]) -> Int {
        events.filter { if case .sentence = $0 { true } else { false } }.count
    }

    private func lastFailure(_ events: [AssistantEvent]) -> AssistantFailure? {
        if case .failed(_, let f)? = events.last { f } else { nil }
    }

    private func collect(_ session: AssistantSession, _ id: String, _ text: String) async -> [AssistantEvent] {
        var events: [AssistantEvent] = []
        for await e in session.events(AssistantRequest(requestId: id, text: text)) { events.append(e) }
        return events
    }

    private let dataQuestion = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"

    /// I1: Misslingt das Schreiben einer Pflichtzeile (`state` beim Öffnen, `start`, `round`, `areas`), läuft kein Modell und geht
    /// nichts hinaus; Ausgang „Fehler“ (`.auditUnavailable`), Beitrag gibt es keinen. Die alte Reihenfolge (Antwort läuft weiter, der
    /// Fehler wird nur gemeldet) gäbe hier einen Modellaufruf und ein Satz-Ereignis. Danach geht die nächste Anfrage durch.
    func testAMandatoryLineThatCannotBeWrittenMeansNoModelNoOutputAndOutcomeError() async {
        for kind in ["state", "start", "round", "areas"] {
            try? FileManager.default.removeItem(at: dir)
            let sw = FailSwitch()
            sw.on = true
            let audit = freshLog(appender: failing(kind, sw))
            let witness = Snapshots()
            let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: witness, pieces: ["Quasselbert wandert nach Pappelhausen."]),
                                           knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
            let events = await collect(session, "r1", dataQuestion)
            XCTAssertEqual(witness.all.count, 0, "\(kind): das Modell lief nicht")
            XCTAssertEqual(sentenceCount(events), 0, "\(kind): nichts ging hinaus")
            XCTAssertEqual(lastFailure(events), .auditUnavailable, kind)
            let wrote = await session.commit(requestId: "r1", user: dataQuestion, spoken: "", interrupted: false)
            // Fehlte nur die Standzeile, schreibt `commit` `start` und `end` ohne Fehler; sonst scheitert auch dieses Schreiben.
            if kind != "state" { XCTAssertFalse(wrote, "\(kind): `commit` meldet den Fehler") }
            let none = await session.history
            XCTAssertEqual(none, [], "\(kind): kein Beitrag")
            sw.on = false
            let failed = await audit.entries().first { $0.requestId == "r1" }
            // Ohne `start` in der Datei gibt es keine Anfrage darin (ein einzelnes `end` ist verwaist); sonst steht Ausgang „Fehler“.
            if kind == "start" { XCTAssertNil(failed, "start: die Anfrage steht gar nicht in der Datei") } else { XCTAssertEqual(failed?.outcome, .failed, "\(kind): Ausgang „Fehler“") }
            let again = await collect(session, "r2", dataQuestion)
            XCTAssertEqual(witness.all.count, 1, "\(kind): erneuter Versuch läuft")
            XCTAssertGreaterThan(sentenceCount(again), 0, kind)
            if case .done? = again.last {} else { XCTFail("\(kind): die zweite Anfrage endet regulär: \(String(describing: again.last))") }
            await session.commit(requestId: "r2", user: dataQuestion, spoken: "", interrupted: true)
        }
    }

    /// I1, Rundenart ohne Quelle (keine Zeile `areas`): `start` oder `round` nicht schreibbar, kein Modell.
    func testAGeneralRoundWithAnUnwritableStartOrRoundLineRunsNoModelEither() async {
        for kind in ["start", "round"] {
            try? FileManager.default.removeItem(at: dir)
            let sw = FailSwitch()
            sw.on = true
            let audit = freshLog(appender: failing(kind, sw))
            let witness = Snapshots()
            let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: witness, pieces: ["Der Mont Blanc ist 4806 Meter hoch."]),
                                           knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
            let events = await collect(session, "r1", "Wie hoch ist der Mont Blanc?")
            XCTAssertEqual(witness.all.count, 0, kind)
            XCTAssertEqual(sentenceCount(events), 0, kind)
            XCTAssertEqual(lastFailure(events), .auditUnavailable, kind)
        }
    }

    /// Cloud-Ziel ohne Zustimmung: kein Abruf, kein Modell, `.consentRequired` (Zeilen `state` und `start` stehen trotzdem).
    func testACloudTargetWithoutConsentRunsNoModelAndReportsConsentRequired() async {
        let audit = AssistantAuditLog(directory: dir)
        let witness = Snapshots()
        let cloud = AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 16_000, maxAnswerTokens: 700, userName: nil, timeout: 5)
        let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: witness, pieces: ["Hallo."]), knowledge: await geheimKnowledge(),
                                       config: cloud, audit: audit)
        let events = await collect(session, "r1", dataQuestion)
        XCTAssertEqual(witness.all.count, 0)
        XCTAssertEqual(sentenceCount(events), 0)
        XCTAssertEqual(lastFailure(events), .consentRequired)
        await session.commit(requestId: "r1", user: dataQuestion, spoken: "", interrupted: false)
        XCTAssertEqual(kinds(audit).prefix(2), ["state", "start"])
    }

    /// Schreibfolge (2): Die Einordnung (`step`) einer Einheit steht dauerhaft in der Datei, bevor das Ereignis hinausgeht (bevor
    /// der erste Ausgabe-Commit geprüft wird). Der Schreiber ist langsam; die alte Reihenfolge (einreihen und senden) zeigte hier
    /// weniger Schrittzeilen in der Datei als bereits empfangene Einheiten.
    func testEveryUnitsStepLineIsInTheFileBeforeItsEventGoesOut() async {
        let audit = slowLog()
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        var received = 0
        var stepsAtEvent: [Int] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: dataQuestion)) {
            guard case .sentence = e else { continue }
            received += 1
            stepsAtEvent.append(kinds(audit).filter { $0 == "step" }.count)
        }
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
        XCTAssertEqual(received, 2, "Ansage und Block")
        for (i, steps) in stepsAtEvent.enumerated() { XCTAssertGreaterThanOrEqual(steps, i + 1, "Einheit \(i + 1): Schrittzeile vor dem Ereignis") }
    }

    /// Schreibfolge (2), Fehler: Lässt sich die Einordnung nicht schreiben, wird die Einheit nicht ausgegeben (kein Satz-Ereignis),
    /// das Modell lief (die Pflichtzeilen standen), Ausgang „Fehler“.
    func testAUnitWhoseStepLineCannotBeWrittenIsNotOutput() async {
        let sw = FailSwitch()
        sw.on = true
        let audit = freshLog(appender: failing("step", sw))
        let witness = Snapshots()
        let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: witness, pieces: ["Quasselbert wandert nach Pappelhausen."]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit, released: true)
        let events = await collect(session, "r1", dataQuestion)
        XCTAssertEqual(witness.all.count, 1)
        XCTAssertEqual(sentenceCount(events), 0, "weder Ansage noch Block")
        XCTAssertEqual(lastFailure(events), .auditUnavailable)
        let wrote = await session.commit(requestId: "r1", user: dataQuestion, spoken: "", interrupted: false)
        XCTAssertFalse(wrote)
        sw.on = false
        let entry = await audit.entries().first { $0.kind == .answer }
        XCTAssertEqual(entry?.outcome, .failed)
    }

    /// Schreibfolge (5): Misslingt eine Ergebniszeile (`commit`), geht keine weitere Einheit hinaus und es gibt kein `done`; die
    /// erste Einheit (die Ansage) ging hinaus, ihr Ergebnis ließ sich nicht festhalten.
    func testAResultLineThatCannotBeWrittenStopsTheOutputAndEndsWithAnError() async {
        let sw = FailSwitch()
        sw.on = true
        let audit = freshLog(appender: failing("commit", sw))
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let events = await collect(session, "r1", dataQuestion)
        let texts = events.compactMap { e -> String? in if case .sentence(_, _, let t, _, _, _, _) = e { t } else { nil } }
        XCTAssertEqual(texts, ["Wörtlicher Auszug aus ‚Der Titel Flimmerkasten‘:"], "nur die erste Einheit; der Block kommt nicht mehr: \(texts)")
        XCTAssertEqual(lastFailure(events), .auditUnavailable)
        XCTAssertFalse(events.contains { if case .done = $0 { true } else { false } }, "kein `done`")
        let wrote = await session.commit(requestId: "r1", user: dataQuestion, spoken: texts.joined(separator: " "), interrupted: false)
        XCTAssertFalse(wrote)
        sw.on = false
        let entry = await audit.entries().first { $0.kind == .answer }
        XCTAssertEqual(entry?.outcome, .failed, "Ausgang „Fehler“")
    }

    /// Schreibfolge (6): Ein Schritt ohne Ergebnis beim Prozessende heißt „Ausgabe möglich, Ergebnis unbekannt“, die Anfrage wird
    /// beim Öffnen als „unvollständig“ geschlossen. Gelesen wird genau diese Datei: Schritt 1 mit Ergebnis, Schritt 2 ohne, Schritt 3
    /// ein verworfener Kandidat (behauptet nichts, nie „unbekannt“).
    func testAStepWithoutAResultAtProcessEndIsReadAsResultUnknownAndTheRequestAsIncomplete() async throws {
        try writeLines([
            journalStart("a"), journalStep("a", 1, .announcement), journalCommit("a", 1, AssistantCommitMark(stage: .emit, ok: true, generation: 1)),
            journalStep("a", 2, .grounded), journalStep("a", 3, .droppedNotSnapped),
        ])
        let audit = AssistantAuditLog(directory: dir)
        let view = await audit.read().entries.first { $0.requestId == "a" }
        XCTAssertEqual(view?.outcome, .incomplete)
        // Je Art des Ergebnisses (Schreibfolge 6): Die Einheit 1 hat nur das Ergebnis „Ereignis“; Anzeige und Audio sind UNBEKANNT,
        // nicht „nicht ausgeführt“. Einheit 2 hat gar kein Ergebnis, Einheit 3 (verworfen) sieht keine Ausgabe vor.
        XCTAssertEqual(view?.trace.map(\.resultUnknown), [true, true, false])
        XCTAssertEqual(view?.trace[0].commits, [AssistantCommitMark(stage: .emit, ok: true, generation: 1)])
        XCTAssertEqual(view?.trace[0].result(for: .emit), .executed)
        XCTAssertEqual(view?.trace[0].result(for: .display), .unknown)
        XCTAssertEqual(view?.trace[0].result(for: .audio), .unknown)
        XCTAssertEqual(view?.trace[0].unknownStages, [.display, .audio, .history])
        XCTAssertEqual(view?.trace[2].unknownStages, [])
    }

    /// Dasselbe aus einer echten Antwort: Das Prozessende liegt zwischen Einordnung und Ergebnis (der Schreiber ist langsam, die
    /// Datei wird beim ersten Ereignis kopiert, die Ergebniszeile steht noch nicht darin).
    func testProcessEndBetweenStepAndResultOfARealAnswerLeavesAnUnknownResult() async throws {
        let audit = slowLog()
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        var crashed: URL?
        defer { if let crashed { try? FileManager.default.removeItem(at: crashed) } }
        for await e in session.events(AssistantRequest(requestId: "r1", text: dataQuestion)) {
            if case .sentence = e, crashed == nil { crashed = try crashCopy(of: audit) }
        }
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
        let copy = try XCTUnwrap(crashed)
        let reopened = AssistantAuditLog(directory: copy, reader: AssistantAuditWriter.readFile)
        _ = await reopened.append(journalEvent(1))
        let entry = await reopened.read().entries.first { $0.requestId == "r1" }
        XCTAssertEqual(entry?.outcome, .incomplete)
        XCTAssertEqual(entry?.trace.first?.verdict, .announcement)
        XCTAssertEqual(entry?.trace.first?.resultUnknown, true, "Einordnung ohne Ergebnis: Ausgabe möglich, Ergebnis unbekannt")
    }

    /// Die Protokolldatei ist durch ein Verzeichnis blockiert (nie schreibbar): kein Modell, keine Ausgabe, Fehler.
    func testABlockedLogFileMeansNoModelAndNoOutput() async throws {
        let blocked = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-blocked-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: blocked) }
        try FileManager.default.createDirectory(at: blocked.appendingPathComponent(AssistantAuditLog.fileName), withIntermediateDirectories: true)
        let witness = Snapshots()
        let audit = AssistantAuditLog(directory: blocked)
        let session = AssistantSession(model: ProbeModel(url: audit.url, snapshots: witness, pieces: ["Hallo zusammen."]),
                                       knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5),
                                       audit: audit, released: true)
        let events = await collect(session, "a", "Hallo")
        XCTAssertEqual(witness.all.count, 0)
        XCTAssertEqual(sentenceCount(events), 0)
        XCTAssertEqual(lastFailure(events), .auditUnavailable)
        let wrote = await session.commit(requestId: "a", user: "Hallo", spoken: "", interrupted: false)
        XCTAssertFalse(wrote)
        let history = await session.history
        XCTAssertEqual(history, [])
    }

    // MARK: I3: Audiostart atomar mit der Prüfung der Generation

    /// Hält einen Aufruf an einer Stelle fest, bis der Test ihn freigibt (deterministische Verschränkung ohne Schlaf).
    private final class Gate: @unchecked Sendable {
        private let entered = DispatchSemaphore(value: 0)
        private let released = DispatchSemaphore(value: 0)
        func arrive() { entered.signal(); released.wait() }
        /// Meldet nur das Eintreffen, ohne zu warten.
        func signal() { entered.signal() }
        func waitUntilArrived() { entered.wait() }
        func open() { released.signal() }
    }

    /// Wartepunkt für eine Quelle im Abruf: `arrive` meldet das Eintreffen und wartet, bis `open` ruft (ohne Thread zu blockieren).
    private actor AsyncGate {
        private var arrived = false
        private var opened = false
        private var arrivedWaiters: [CheckedContinuation<Void, Never>] = []
        private var openWaiters: [CheckedContinuation<Void, Never>] = []
        private(set) var calls = 0

        func arrive() async {
            calls += 1
            arrived = true
            for w in arrivedWaiters { w.resume() }
            arrivedWaiters = []
            if opened { return }
            await withCheckedContinuation { openWaiters.append($0) }
        }

        func waitUntilArrived() async {
            if arrived { return }
            await withCheckedContinuation { arrivedWaiters.append($0) }
        }

        func open() {
            opened = true
            for w in openWaiters { w.resume() }
            openWaiters = []
        }
    }

    /// Quelle der Ablage, die im Abruf an einem Wartepunkt hängt.
    private struct GatedSource: AssistantSource {
        let area: AssistantArea
        let gate: AsyncGate
        func items() async -> [AssistantItem] { [] }
        func search(_ question: String, limit: Int) async -> [AssistantPassage] {
            await gate.arrive()
            return [AssistantPassage(area: area, itemId: "f1", title: "Ablagedatei Zimtsterne", locator: nil,
                                     text: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen.", score: 1, isBlock: true, quotable: true,
                                     raw: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen.")]
        }
    }

    /// I4, Haltepunkt: Die Anfrage liest Generation und Bereiche in EINEM Schritt (`snapshot`) und gibt genau diese an Abruf,
    /// Bestand und Abgleich. Wird mitten im Abruf (hier: im Abruf der Ablage) ein Bereich entzogen, ist die laufende Anfrage
    /// widerrufen und gibt nichts aus; sie sah nie (g+1, die alten größeren Bereiche). Die nächste Anfrage trägt (g+1, engere
    /// Bereiche) und ruft die entzogene Quelle gar nicht erst auf. Die alte Reihenfolge (Bereiche aus der Wissensbasis, Generation
    /// gesondert gelesen) ließe die nächste Anfrage mit der neuen Generation und den alten Bereichen laufen.
    func testARestrictionDuringTheRetrievalRevokesTheRunAndTheNextRunNeverSeesTheOldAreas() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let gate = AsyncGate()
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", sections: [
            ("S. 7", "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."),
        ]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [GatedSource(area: .files, gate: gate)])
        await knowledge.setAreas([.books, .files])
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: knowledge,
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        let question = dataQuestion
        let first = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in session.events(AssistantRequest(requestId: "r1", text: question)) { out.append(e) }
            return out
        }
        await gate.waitUntilArrived()   // die Anfrage hat (g, Bücher und Ablage) gelesen und steckt im Abruf der Ablage

        let change = await session.setAreas([.books])
        XCTAssertEqual(change.generation, g + 1)
        XCTAssertEqual(session.effectiveAreas, [.books])
        await gate.open()
        let events = await first.value

        XCTAssertFalse(events.contains { if case .sentence = $0 { true } else { false } }, "nach dem Entzug kommt nichts mehr hinaus")
        for e in events {
            if case .sources(_, let gen, _) = e { XCTAssertEqual(gen, g, "nie die neue Generation mit den alten Bereichen") }
        }
        let lastFailure: AssistantFailure? = { if case .failed(_, let f)? = events.last { f } else { nil } }()
        XCTAssertTrue(lastFailure == .revoked || lastFailure == .cancelled, "\(String(describing: events.last))")
        let callsAfterFirst = await gate.calls
        XCTAssertEqual(callsAfterFirst, 1)

        // Die nächste Anfrage: neue Generation, engere Bereiche, die Quelle der Ablage wird nicht aufgerufen.
        let second = await collect(session, "r2", dataQuestion)
        let secondGen = second.compactMap { e -> Int? in if case .sources(_, let gen, _) = e { gen } else { nil } }
        XCTAssertEqual(secondGen, [g + 1])
        let callsAfterSecond = await gate.calls
        XCTAssertEqual(callsAfterSecond, 1, "die entzogene Quelle wird nicht mehr gefragt")
        for e in second {
            if case .sources(_, _, let passages) = e { XCTAssertFalse(passages.contains { $0.area == .files }) }
        }
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
        await session.commit(requestId: "r2", user: "x", spoken: "", interrupted: true)

        // Im Protokoll trägt keine Zeile der ersten Anfrage die neue Generation.
        _ = await audit.waitUntilWritten(request: "r1")
        for l in lines(audit) where l["req"] as? String == "r1" && ["start", "round", "areas"].contains(l["t"] as? String ?? "") {
            XCTAssertEqual(l["gen"] as? Int, g, "\(l)")
        }
    }

    private func marks(_ audit: AssistantAuditLog, _ rid: String) async -> [String] {
        _ = await audit.waitUntilWritten(request: rid)
        return lines(audit).filter { $0["t"] as? String == "commit" && $0["req"] as? String == rid }.compactMap { $0["mark"] as? String }
    }

    /// I3, Kernprüfung: Zwischen „Start gemeldet“ und tatsächlichem Start steigt die Generation. Der Start wird nicht ausgelöst, die
    /// Marke lautet „verworfen“ mit der vorgefundenen Generation, nie „ausgeführt“.
    func testAStartAfterAnIncreaseOfTheGenerationIsRejectedAndNeverMarkedAsExecuted() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        _ = await collect(session, "r1", dataQuestion)
        await session.setAllowUnsourced(false)   // die Freigabe ändert sich, BEVOR die Stimme den Satz startet
        var started = 0
        let result = session.commitStart(requestId: "r1", index: 1, generation: g) { started += 1; return true }
        XCTAssertFalse(result.ok)
        XCTAssertEqual(started, 0, "der Start läuft nach der Erhöhung nicht mehr")
        XCTAssertEqual(result.generation, g + 1, "die vorgefundene Generation")
        let m = await marks(audit, "r1")
        XCTAssertTrue(m.contains("a-\(g + 1)"), "\(m)")
        XCTAssertFalse(m.contains { $0.hasPrefix("a+") }, "kein „ausgeführt“ ohne Start: \(m)")
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
    }

    /// I3: Gilt die Generation, läuft der Start und die Marke „ausgeführt“ trägt genau die geprüfte Generation; lässt sich der Start
    /// nicht auslösen (Stimme verweigert), steht „fehlgeschlagen“.
    func testAStartThatRunsIsMarkedExecutedAndARefusedStartAsFailed() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        _ = await collect(session, "r1", dataQuestion)
        var started = 0
        let ok = session.commitStart(requestId: "r1", index: 0, generation: g) { started += 1; return true }
        let refused = session.commitStart(requestId: "r1", index: 1, generation: g) { started += 1; return false }
        XCTAssertTrue(ok.ok)
        XCTAssertFalse(refused.ok)
        XCTAssertTrue(refused.failed)
        XCTAssertEqual(started, 2)
        let m = await marks(audit, "r1")
        XCTAssertTrue(m.contains("a+\(g)"), "\(m)")
        XCTAssertTrue(m.contains("a!\(g)"), "\(m)")
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
    }

    /// I3, Verschränkung mit Sperre: Während der Start läuft (er hält die Sperre der Freigabe), kann eine Erhöhung nicht
    /// dazwischentreten; sie kehrt erst NACH dem Start zurück, und die Marke trägt die Generation vor ihr.
    func testAnIncreaseCannotInterleaveWithARunningStart() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        _ = await collect(session, "r1", dataQuestion)
        let inside = Gate()
        let bodyDone = FailSwitch()
        let increaseSawStartDone = FailSwitch()
        let returnedWhileTheStartRan = FailSwitch()
        // Zwei Signale des Erhöhenden: „zurückgekehrt“ (der Start beobachtet es) und „Thread zu Ende“ (der Test wartet darauf, nicht
        // auf einen Zustand, den der Thread unterwegs erreicht).
        let increaseReturned = DispatchSemaphore(value: 0)
        let increaserEnded = DispatchSemaphore(value: 0)
        let increaser = Thread {
            inside.waitUntilArrived()
            _ = session.applyReleaseChange()   // blockiert, bis der Start fertig ist
            increaseSawStartDone.on = bodyDone.on
            increaseReturned.signal()
            increaserEnded.signal()
        }
        increaser.start()
        let result = session.commitStart(requestId: "r1", index: 1, generation: g) {
            inside.signal()   // hier greift die andere Änderung an
            // Haltepunkt statt Schlafen: Die Erhöhung läuft an und muss auf die Sperre warten. Dass sie in dieser Zeit NICHT
            // zurückkehrt, ist die Beobachtung selbst (ein wartender Thread lässt sich nur so erkennen). Die Länge des Fensters
            // macht aus Grün nie Rot, nur die Prüfung schwächer.
            if awaited(increaseReturned, seconds: 0.3) { returnedWhileTheStartRan.on = true }
            bodyDone.on = true
            return true
        }
        XCTAssertTrue(result.ok)
        XCTAssertEqual(result.generation, g)
        XCTAssertTrue(awaited(increaserEnded, seconds: 10), "der Erhöhende ist zu Ende")
        XCTAssertFalse(returnedWhileTheStartRan.on, "die Erhöhung kehrte nicht zurück, solange der Start lief")
        XCTAssertTrue(increaseSawStartDone.on, "die Erhöhung kehrte erst zurück, als der Start vollständig war")
        XCTAssertEqual(session.releaseGeneration, g + 1)
        let m = await marks(audit, "r1")
        XCTAssertTrue(m.contains("a+\(g)"), "\(m)")
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
    }

    /// I3, Beweis der Lücke der alten Reihenfolge (Meta-Test mit denselben Bausteinen): Wer die Generation prüft, die Sperre
    /// loslässt und DANN startet, lässt die Erhöhung dazwischentreten; sie kehrt vor dem Start zurück. Das ist der Zustand, den
    /// `commitStart` ausschließt (der Test davor ist die Gegenprobe und wäre mit der alten Reihenfolge rot).
    func testTheOldOrderCheckThenStartLeavesAWindowThatTheAtomicStartClosed() async {
        let release = AssistantRelease(optIn: true, consentVersion: 1, areas: Set(AssistantArea.allCases), cloudConsent: true, onIncrease: nil)
        let g = release.currentGeneration
        let between = Gate()
        var startedAfterIncrease = false
        let increaser = Thread {
            between.waitUntilArrived()
            _ = release.apply(AssistantReleaseChange(cause: .other, force: true))
            between.open()
        }
        increaser.start()
        // Alte Reihenfolge: prüfen (Lock kurz), Lock frei, dann starten.
        let stillCurrent = release.ifCurrent(g) {}.ok
        XCTAssertTrue(stillCurrent)
        between.arrive()   // hier tritt die Erhöhung ein, vollständig
        startedAfterIncrease = release.currentGeneration != g
        XCTAssertTrue(startedAfterIncrease, "alte Reihenfolge: der Start liefe NACH der Erhöhung (veraltet) weiter")
    }

    // MARK: I5: geprüfte Generation wird durchgereicht

    /// I5: Der Verlaufs-Commit trägt die geprüfte Generation. Ein Widerruf unmittelbar nach dem Anhängen ändert die Marke nicht.
    func testARevocationRightAfterTheHistoryAppendDoesNotChangeTheCheckedGenerationInTheMark() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        var spoken: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: dataQuestion)) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        await session.commit(requestId: "r1", user: "x", spoken: spoken.joined(separator: " "), interrupted: false)
        let after = await session.setAllowUnsourced(false)   // Widerruf unmittelbar nach dem Anhängen
        XCTAssertEqual(after.generation, g + 1)
        let m = await marks(audit, "r1")
        XCTAssertTrue(m.contains("h+\(g)"), "die Marke trägt die geprüfte Generation g, nicht g+1: \(m)")
        XCTAssertFalse(m.contains("h+\(g + 1)"))
    }

    /// I5: Eine Ablehnung (Widerruf vor dem Anhängen) trägt die dabei VORGEFUNDENE Generation.
    func testAHistoryRejectionCarriesTheFoundGeneration() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        _ = await collect(session, "r1", dataQuestion)
        await session.setAllowUnsourced(false)
        await session.commit(requestId: "r1", user: "x", spoken: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula.", interrupted: false)
        let m = await marks(audit, "r1")
        XCTAssertTrue(m.contains("h-\(g + 1)"), "\(m)")
        let history = await session.history
        XCTAssertEqual(history, [])
    }

    /// I5: Die Anzeige trägt die geprüfte Generation, auch wenn eine Erhöhung während des Commits wartet.
    func testTheDisplayMarkCarriesTheCheckedGenerationWhileAnIncreaseWaits() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit, released: true)
        let g = session.releaseGeneration
        _ = await collect(session, "r1", dataQuestion)
        let inside = Gate()
        let increaseReturned = DispatchSemaphore(value: 0)
        let increaserEnded = DispatchSemaphore(value: 0)
        let returnedWhileTheCommitRan = FailSwitch()
        let increaser = Thread {
            inside.waitUntilArrived()
            _ = session.applyReleaseChange()
            increaseReturned.signal()
            increaserEnded.signal()
        }
        increaser.start()
        let result = session.commitOutput(requestId: "r1", index: 1, generation: g, stage: .display) {
            inside.signal()
            // Haltepunkt statt Schlafen (siehe `testAnIncreaseCannotInterleaveWithARunningStart`).
            if awaited(increaseReturned, seconds: 0.3) { returnedWhileTheCommitRan.on = true }
        }
        XCTAssertTrue(result.ok)
        XCTAssertTrue(awaited(increaserEnded, seconds: 10), "der Erhöhende ist zu Ende")
        XCTAssertFalse(returnedWhileTheCommitRan.on, "die Erhöhung kehrte nicht zurück, solange der Commit lief")
        XCTAssertEqual(session.releaseGeneration, g + 1)
        let m = await marks(audit, "r1")
        XCTAssertTrue(m.contains("d+\(g)"), "\(m)")
        XCTAssertFalse(m.contains("d+\(g + 1)"))
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
    }
}

private struct FailingModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { c in c.finish(throwing: BookTalkModelError.offline) }
    }
}

/// Eine Quelle, deren Suche hängt (bis zum Abbruch): der Abruf steht still.
private struct HangingSource: AssistantSource {
    let area: AssistantArea
    func items() async -> [AssistantItem] { [] }
    func search(_ question: String, limit: Int) async -> [AssistantPassage] {
        while !Task.isCancelled { try? await Task.sleep(nanoseconds: 10_000_000) }
        return []
    }
}
