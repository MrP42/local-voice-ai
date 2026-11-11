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
                          rewriter: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.rewriteFile) -> AssistantAuditLog {
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
                                       knowledge: await geheimKnowledge(), config: config, audit: audit)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        for await e in session.events(AssistantRequest(requestId: "r1", text: question)) { _ = e }
        session.noteCommit(requestId: "r1", index: 1, stage: .display, ok: true, generation: session.releaseGeneration)
        await session.commit(requestId: "r1", user: question, spoken: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula.", interrupted: false)
        _ = await audit.record(AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 1, generation: 3))

        let keys: Set<String> = ["t", "req", "at", "session", "model", "cloud", "unsourced", "consent", "ctx", "reason", "round", "gen", "n",
                                 "verdict", "mark", "outcome", "level", "areas", "kind"]
        let words: Set<String> = [
            "start", "step", "commit", "end", "event", "r1", "apple", "gemma", "other", "data", "free", "general", "optedOut", "locked",
            "inventory", "ownData", "relevantPassage", "grounded", "unsourced", "template", "announcement", "droppedOwnData",
            "droppedNoMode", "droppedNotSnapped", "droppedDuplicate", "droppedTemplate", "droppedRevoked", "droppedFragment", "completed",
            "interrupted", "revoked", "failed", "incomplete", "limited", "s0", "s1", "books", "transcripts", "files", "notes",
            "answer", "unsourcedOn", "unsourcedOff", "cloudGranted", "cloudRevoked", "resumedFree",
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
                                       knowledge: await geheimKnowledge(), config: config, audit: audit)
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
        let entries = await audit.entries()
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
        let session = AssistantSession(model: AuditModel(pieces: []), knowledge: knowledge, config: config, audit: audit)
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
        let entries = await audit.entries()
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
                                       knowledge: await geheimKnowledge(), config: config, audit: audit)
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
        XCTAssertEqual(atModel.split(separator: "\n").count, 1, "beim Modellaufruf steht nur `start` in der Datei: \(atModel)")
        XCTAssertTrue(atModel.contains("\"t\":\"start\""))
        XCTAssertEqual(atFirstEvent.compactMap { $0["t"] as? String }.first, "start")
        XCTAssertFalse(atFirstEvent.isEmpty, "beim ersten Ereignis ist `start` geschrieben")
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)
    }

    /// Die Einwilligungsversion kommt JETZT aus der Freigabe, nicht aus der Konfiguration der Sitzung.
    func testStartLineCarriesTheConsentVersionOfTheReleaseNotOfTheConfig() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: config, audit: audit)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Wie hoch ist der Mont Blanc?")) {}
        await session.commit(requestId: "a", user: "Wie hoch ist der Mont Blanc?", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        session.setAllowUnsourced(true, consentVersion: 3)
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Wie hoch ist der Mont Blanc?")) {}
        await session.commit(requestId: "b", user: "Wie hoch ist der Mont Blanc?", spoken: "Der Mont Blanc ist 4806 Meter hoch.", interrupted: false)
        session.setAllowUnsourced(false)
        for await _ in session.events(AssistantRequest(requestId: "c", text: "Hallo")) {}
        await session.commit(requestId: "c", user: "Hallo", spoken: "", interrupted: true)
        session.setAllowUnsourced(true, consentVersion: 4)
        for await _ in session.events(AssistantRequest(requestId: "d", text: "Hallo")) {}
        await session.commit(requestId: "d", user: "Hallo", spoken: "", interrupted: true)
        let all = await audit.entries()
        XCTAssertEqual(all.map(\.requestId), ["a", "b", "c", "d"])
        XCTAssertEqual(all.map(\.consentVersion), [1, 3, 0, 4])
        XCTAssertEqual(all.map(\.unsourcedMode), [true, true, false, true])
    }

    func testStartLineCarriesContextStateReasonAndRoundKind() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Der Mont Blanc ist 4806 Meter hoch."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Wie hoch ist der Mont Blanc?")) {}
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {}
        for await _ in session.events(AssistantRequest(requestId: "c", text: "Wie hoch ist die Zugspitze?")) {}
        for id in ["a", "b", "c"] { await session.commit(requestId: id, user: "x", spoken: "", interrupted: true) }
        let all = await audit.entries()
        XCTAssertEqual(all.map(\.contextMode), [.free, .free, .data], "Zustand vor der Rundenwahl")
        XCTAssertEqual(all.map(\.contextReason), [.free, .relevantPassage, .locked])
        XCTAssertEqual(all.map(\.round), [.general, .data, .data], "nach dem Einrasten bleibt jede Runde eine Datenrunde")
    }

    /// Nach 200 Schritten endet die Antwort mit dem Ausgang „begrenzt“; es gibt höchstens 200 Schrittzeilen, nichts wird gekappt.
    func testTheAnswerEndsAfter200StepsWithOutcomeLimited() async {
        let audit = AssistantAuditLog(directory: dir)
        let many = (0..<250).map { "Satz Nummer \($0) handelt von etwas ganz anderem." }.joined(separator: " ")
        let strict = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        let session = AssistantSession(model: AuditModel(pieces: [many]), knowledge: await geheimKnowledge(), config: strict, audit: audit)
        var events: [AssistantEvent] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Was ist mit den Sätzen?")) { events.append(e) }
        await session.commit(requestId: "r1", user: "Was ist mit den Sätzen?", spoken: "", interrupted: false)
        XCTAssertEqual(AssistantAuditLog.maxSteps, 200)
        let all = await audit.entries()
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
                                       config: cloud, audit: audit)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        await session.commit(requestId: "a", user: "Hallo", spoken: "Kieselsteine sind meistens rund.", interrupted: true)
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Hallo")) {}
        await session.commit(requestId: "b", user: "Hallo", spoken: "Kieselsteine sind meistens rund.", interrupted: true, revoked: true)
        let failing = AssistantSession(model: FailingModel(), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: cloud, audit: audit)
        for await _ in failing.events(AssistantRequest(requestId: "c", text: "Hallo")) {}
        await failing.commit(requestId: "c", user: "Hallo", spoken: "", interrupted: false)
        let all = await audit.entries()
        XCTAssertEqual(all.map(\.outcome), [.interrupted, .revoked, .failed])
        XCTAssertEqual(all.map(\.cloud), [true, true, true])
        XCTAssertEqual(all.map(\.round), [.general, .general, .general])
    }

    /// Auch eine Anfrage ohne gesprochenen Text bekommt ihr abgeschlossenes Protokoll (Fehler, Abbruch, Widerruf).
    func testEntryExistsEvenWithoutSpokenText() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Hallo."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5),
                                       audit: audit)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        await session.commit(requestId: "a", user: "Hallo", spoken: "", interrupted: true)
        let all = await audit.entries()
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
                                       knowledge: await geheimKnowledge(), config: config, audit: audit)
        var done: AssistantAnswer?
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            if case .done(_, let a) = e { done = a }
        }
        await session.commit(requestId: "r1", user: "x", spoken: "", interrupted: false)
        let entries = await audit.entries()
        XCTAssertNotNil(done)
        XCTAssertEqual(entries.first?.counts, done?.counts)
        XCTAssertGreaterThan(entries.first?.counts.droppedOwnData ?? 0, 0)
        XCTAssertEqual(entries.first?.counts.grounded, 1)
    }

    /// Der Controller meldet Anzeige und Audiostart je Ereignis; die Meldungen landen als `commit` am richtigen Schritt.
    func testControllerCommitMarksLandOnTheirStepsAndUnknownIndexesAreIgnored() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit)
        let g = session.releaseGeneration
        var spoken: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        session.noteCommit(requestId: "r1", index: 1, stage: .display, ok: true, generation: g)
        session.noteCommit(requestId: "r1", index: 1, stage: .audio, ok: false, generation: g + 1)
        session.noteCommit(requestId: "r1", index: 99, stage: .display, ok: true, generation: g)
        await session.commit(requestId: "r1", user: "x", spoken: spoken.joined(separator: " "), interrupted: false)
        let e = await audit.entries().first
        XCTAssertEqual(e?.trace.map(\.verdict), [.announcement, .grounded])
        XCTAssertEqual(e?.trace[1].commits, [
            AssistantCommitMark(stage: .emit, ok: true, generation: g), AssistantCommitMark(stage: .display, ok: true, generation: g),
            AssistantCommitMark(stage: .audio, ok: false, generation: g + 1),
        ])
        XCTAssertEqual(e?.trace[0].commits, [AssistantCommitMark(stage: .emit, ok: true, generation: g)])
    }

    /// Ein Absturz mitten in der Antwort hinterlässt `start` und die bis dahin angefallenen Schritte; das nächste Öffnen schließt
    /// die Anfrage mit „unvollständig“. (Der Absturz wird nachgestellt: Stand der Datei bei laufender Antwort kopieren.)
    func testACrashMidAnswerLeavesStartAndStepsAndTheNextOpenClosesItAsIncomplete() async throws {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: HangingModel(pieces: ["Quasselbert wandert nach Pappelhausen und trifft dort Tante Lavendula. Danach"]),
                                       knowledge: await geheimKnowledge(), config: config, audit: audit)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        let task = Task { () -> Int in
            var n = 0
            for await _ in session.events(AssistantRequest(requestId: "r1", text: question)) { n += 1 }
            return n
        }
        // Warten, bis Vorspann und Zitat geschrieben sind (die Antwort hängt danach).
        var snapshot: [[String: Any]] = []
        for _ in 0..<200 {
            _ = await audit.read()
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

        XCTAssertEqual(snapshot.compactMap { $0["t"] as? String }.first, "start")
        XCTAssertFalse(snapshot.contains { $0["t"] as? String == "end" }, "beim Absturz gab es kein `end`")
        let reopened = AssistantAuditLog(directory: crashed, reader: AssistantAuditWriter.readFile)
        _ = await reopened.append(journalEvent(1))
        let read = await reopened.read()
        XCTAssertEqual(read.entries.first?.requestId, "r1")
        XCTAssertEqual(read.entries.first?.outcome, .incomplete)
        XCTAssertGreaterThanOrEqual(read.entries.first?.trace.count ?? 0, 2)
        XCTAssertEqual(read.entries.first?.round, .data)
    }

    /// Ein Fehler im Protokoll während der Antwort (Datenträger voll) hält die Antwort nicht auf, und der Beitrag steht trotzdem im
    /// Verlauf; `commit` meldet den Fehler.
    func testAnAuditFailureDuringTheAnswerNeitherStopsItNorLosesTheHistoryTurn() async {
        let audit = AssistantAuditLog(directory: dir, reader: { _ in nil }, appender: { _, _ in throw DiskFull() })
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen."]), knowledge: await geheimKnowledge(),
                                       config: config, audit: audit)
        var spoken: [String] = []
        var finished = false
        for await e in session.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
            if case .done = e { finished = true }
        }
        XCTAssertTrue(finished)
        XCTAssertEqual(spoken.count, 1)
        let wrote = await session.commit(requestId: "r1", user: "Wohin wandert Quasselbert nach Pappelhausen?", spoken: spoken.joined(separator: " "), interrupted: false)
        XCTAssertFalse(wrote, "das Protokoll konnte nicht geschrieben werden")
        let history = await session.history
        XCTAssertEqual(history.count, 1, "nur das Protokoll fehlt, der Beitrag steht trotzdem im Verlauf")
    }

    func testAnAuditFailureOnTheBlockedFileDoesNotLoseTheHistoryTurn() async throws {
        let blocked = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-blocked-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: blocked) }
        try FileManager.default.createDirectory(at: blocked.appendingPathComponent(AssistantAuditLog.fileName), withIntermediateDirectories: true)
        let session = AssistantSession(model: AuditModel(pieces: ["Hallo zusammen."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5),
                                       audit: AssistantAuditLog(directory: blocked))
        for await e in session.events(AssistantRequest(requestId: "a", text: "Hallo")) { _ = e }
        let wrote = await session.commit(requestId: "a", user: "Hallo", spoken: "Hallo zusammen.", interrupted: false)
        XCTAssertFalse(wrote)
        let history = await session.history
        XCTAssertEqual(history.count, 1)
    }
}

private struct FailingModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { c in c.finish(throwing: BookTalkModelError.offline) }
    }
}
