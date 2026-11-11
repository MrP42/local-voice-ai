import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Protokoll ohne Inhalte. Nur Verzeichnisse unter dem Temp-Ordner des Tests.

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

final class AssistantAuditLogTests: XCTestCase {
    private var dir: URL!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    private func counts(_ n: Int) -> AssistantSentenceCounts {
        AssistantSentenceCounts(grounded: n, unsourced: 0, neutral: 2, droppedNotSnapped: 3)
    }

    private func entry(_ n: Int, kind: AssistantAuditEntry.Kind = .answer) -> AssistantAuditEntry {
        AssistantAuditEntry(at: Date(timeIntervalSince1970: 1_760_000_000 + Double(n)), kind: kind, model: "Test", cloud: n % 2 == 0,
                            unsourcedMode: n % 3 == 0, consentVersion: 1, counts: counts(n), areas: [.notes, .books, .books],
                            outcome: .completed, round: .data, generation: n + 1,
                            trace: [AssistantTraceStep(n: 1, verdict: .grounded, generation: n + 1,
                                                       commits: [AssistantCommitMark(stage: .emit, ok: true, generation: n + 1)])],
                            history: AssistantCommitMark(stage: .history, ok: true, generation: n + 1))
    }

    // MARK: Datei

    func testRoundTripKeepsEveryFieldAndAreasAreSortedAndUnique() async {
        let log = AssistantAuditLog(directory: dir)
        let initial = await log.entries()
        XCTAssertEqual(initial, [])
        let ok = await log.record(entry(1))
        XCTAssertTrue(ok)
        _ = await log.record(entry(2, kind: .unsourcedOn))
        let loaded = await AssistantAuditLog(directory: dir).entries()
        XCTAssertEqual(loaded, [entry(1), entry(2, kind: .unsourcedOn)])
        XCTAssertEqual(loaded[0].areas, ["books", "notes"])
        XCTAssertEqual(loaded[0].round, .data)
        XCTAssertEqual(loaded[0].generation, 2)
        XCTAssertEqual(loaded[0].trace.first?.commits, [AssistantCommitMark(stage: .emit, ok: true, generation: 2)])
        XCTAssertEqual(loaded[0].history, AssistantCommitMark(stage: .history, ok: true, generation: 2))
    }

    func testFileIsOneJSONObjectPerLine() async throws {
        let log = AssistantAuditLog(directory: dir)
        for n in 0..<3 { _ = await log.record(entry(n)) }
        let raw = try String(contentsOf: log.url, encoding: .utf8)
        let lines = raw.split(separator: "\n", omittingEmptySubsequences: true)
        XCTAssertEqual(lines.count, 3)
        for line in lines { XCTAssertNotNil(try JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any]) }
        XCTAssertTrue(raw.hasSuffix("\n"))
    }

    func testKeepsOnlyTheLast500() async {
        let log = AssistantAuditLog(directory: dir)
        for n in 0..<520 { _ = await log.record(entry(n)) }
        let all = await log.entries()
        XCTAssertEqual(all.count, AssistantAuditLog.maxEntries)
        XCTAssertEqual(all.first?.counts.grounded, 20)
        XCTAssertEqual(all.last?.counts.grounded, 519)
    }

    /// Mehrere Schreiber gleichzeitig über ein Handle (Antworten und Einwilligung): nichts geht verloren.
    func testConcurrentWritersLoseNothing() async {
        let log = AssistantAuditLog(directory: dir)
        let entries = (0..<60).map { entry($0) }
        await withTaskGroup(of: Void.self) { group in
            for e in entries { group.addTask { _ = await log.record(e) } }
        }
        let all = await log.entries()
        XCTAssertEqual(all.count, 60)
        XCTAssertEqual(Set(all.map(\.counts.grounded)), Set(0..<60))
    }

    /// Zwei Handles auf dasselbe Verzeichnis (zum Beispiel Controller und Sitzung) teilen einen Schreiber: 50 + 50
    /// gleichzeitige Einträge ergeben 100, keiner überschreibt den anderen.
    func testTwoHandlesOnTheSameDirectoryShareOneWriter() async {
        let a = AssistantAuditLog(directory: dir)
        let b = AssistantAuditLog(directory: dir.appendingPathComponent("unterordner").appendingPathComponent(".."))
        XCTAssertTrue(AssistantAuditRegistry.writer(for: dir) === AssistantAuditRegistry.writer(for: b.directory),
                      "gleiches Verzeichnis, auch bei anderer Schreibweise des Pfads")
        let first = (0..<50).map { entry($0) }
        let second = (50..<100).map { entry($0) }
        await withTaskGroup(of: Void.self) { group in
            for e in first { group.addTask { _ = await a.record(e) } }
            for e in second { group.addTask { _ = await b.record(e) } }
        }
        let all = await a.entries()
        XCTAssertEqual(all.count, 100)
        XCTAssertEqual(Set(all.map(\.counts.grounded)), Set(0..<100))
        let other = AssistantAuditLog(directory: dir.appendingPathComponent("anderes"))
        XCTAssertFalse(AssistantAuditRegistry.writer(for: dir) === AssistantAuditRegistry.writer(for: other.directory))
    }

    /// Ein abgebrochener Schreibvorgang hinterlässt eine halbe letzte Zeile: Sie wird übersprungen und gezählt, der Rest bleibt
    /// lesbar, und der nächste Eintrag landet sauber dahinter.
    func testTruncatedLastLineIsSkippedCountedAndTheRestSurvives() async throws {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(entry(1))
        _ = await log.record(entry(2))
        var data = try Data(contentsOf: log.url)
        data.append(Data(#"{"at":"2025-10-"#.utf8))
        try data.write(to: log.url)
        var read = await log.read()
        XCTAssertEqual(read.entries.map(\.counts.grounded), [1, 2])
        XCTAssertEqual(read.skippedLines, 1)
        XCTAssertNil(read.error)
        _ = await log.record(entry(3))
        read = await log.read()
        XCTAssertEqual(read.entries.map(\.counts.grounded), [1, 2, 3])
        XCTAssertEqual(read.skippedLines, 1, "die beschädigte Zeile bleibt erhalten und wird weiter gemeldet")
    }

    // MARK: Fehlerfälle beim Schreiben und Lesen

    /// Lesefehler: wird gemeldet, nichts wird überschrieben, die Datei bleibt byte-gleich.
    func testReadErrorIsReportedAndOverwritesNothing() async throws {
        let healthy = AssistantAuditLog(directory: dir)
        _ = await healthy.record(entry(1))
        _ = await healthy.record(entry(2))
        let before = try Data(contentsOf: healthy.url)

        struct Unreadable: Error {}
        let failing = AssistantAuditLog(directory: dir, reader: { _ in throw Unreadable() })
        let result = await failing.append(entry(3))
        XCTAssertEqual(result, .readFailed)
        let recorded = await failing.record(entry(4))
        XCTAssertFalse(recorded)
        XCTAssertEqual(try Data(contentsOf: healthy.url), before, "nichts überschrieben")
        let read = await failing.read()
        XCTAssertEqual(read.error, .readFailed)
        XCTAssertEqual(read.entries, [])
        let intact = await healthy.entries()
        XCTAssertEqual(intact.count, 2)
    }

    /// Schreibfehler nach erfolgreichem Lesen (hier: der Ordner kann nicht angelegt werden, weil an seiner Stelle eine Datei
    /// liegt; so verhält sich auch ein voller oder schreibgeschützter Datenträger): gemeldet, nichts stürzt.
    func testWriteFailureIsReportedAsWriteFailed() async throws {
        let blockerFile = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-file-\(UUID().uuidString)")
        try Data("x".utf8).write(to: blockerFile)
        defer { try? FileManager.default.removeItem(at: blockerFile) }
        let log = AssistantAuditLog(directory: blockerFile.appendingPathComponent("unter"))
        let result = await log.append(entry(1))
        XCTAssertEqual(result, .writeFailed)
        let ok = await log.record(entry(2))
        XCTAssertFalse(ok)
        XCTAssertEqual(try Data(contentsOf: blockerFile), Data("x".utf8))
    }

    /// Ein Verzeichnis an der Stelle der Datei: Lesen scheitert, die vorhandene Datei in einem anderen Verzeichnis bleibt.
    func testFailedAccessIsReportedAndOtherLogsAreUnaffected() async throws {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(entry(1))
        let before = try Data(contentsOf: log.url)
        let blocked = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-blocked-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: blocked) }
        try FileManager.default.createDirectory(at: blocked.appendingPathComponent(AssistantAuditLog.fileName), withIntermediateDirectories: true)
        let broken = AssistantAuditLog(directory: blocked)
        let ok = await broken.record(entry(2))
        XCTAssertFalse(ok)
        XCTAssertEqual(try Data(contentsOf: log.url), before)
        let stillThere = await log.record(entry(3))
        XCTAssertTrue(stillThere, "ein Fehler an anderer Stelle beeinträchtigt dieses Protokoll nicht")
    }

    /// Dass bei einem Abbruch mitten im Schreiben die alte Datei ganz bleibt, leistet `.atomic` (temporäre Datei, dann
    /// Umbenennen); hier: nach dem Schreiben bleibt nichts anderes liegen.
    func testNoStrayTemporaryFilesAfterWriting() async throws {
        let log = AssistantAuditLog(directory: dir)
        for n in 0..<5 { _ = await log.record(entry(n)) }
        let names = try FileManager.default.contentsOfDirectory(atPath: dir.path)
        XCTAssertEqual(names, [AssistantAuditLog.fileName])
    }

    func testClearRemovesTheFile() async {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(entry(1))
        await log.clear()
        let all = await log.entries()
        XCTAssertEqual(all, [])
        XCTAssertFalse(FileManager.default.fileExists(atPath: log.url.path))
    }

    // MARK: Altdaten (Migration vorwärts)

    private let legacyLine = #"{"areas":["books"],"at":"2025-10-09T09:20:00Z","cloud":false,"consentVersion":1,"dropped":3,"grounded":2,"kind":"answer","model":"Apple Foundation Models","neutral":2,"outcome":"completed","unsourced":1,"unsourcedMode":true}"#

    /// Zeilen aus der Zeit vor der Satzspur (flache Zähler, `model` als Freitext) laden und werden zu Art und neuen Zählern. Das
    /// Lesen verändert die Datei nicht; erst das nächste Schreiben überträgt sie ins neue Format (kein Freitext mehr, Zähler in
    /// `counts`) und hängt den neuen Eintrag an.
    func testLegacyLinesLoadWithoutTouchingTheFileAndAreMigratedOnTheNextWrite() async throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(AssistantAuditLog.fileName)
        try Data((legacyLine + "\n").utf8).write(to: url)
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
        XCTAssertEqual(old.areas, ["books"])
        XCTAssertEqual(old.outcome, .completed)
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), legacyLine + "\n", "Lesen schreibt nichts")

        let ok = await log.record(entry(5))
        XCTAssertTrue(ok)
        all = await log.entries()
        XCTAssertEqual(all.count, 2)
        XCTAssertEqual(all[0], old, "inhaltlich unverändert")
        XCTAssertEqual(all[1], entry(5))
        let raw = try String(contentsOf: url, encoding: .utf8)
        XCTAssertFalse(raw.contains("Foundation"), "der Freitext ist weg")
        XCTAssertFalse(raw.contains("\"model\""))
        XCTAssertFalse(raw.contains("\"dropped\""), "flache Zähler sind nach `counts` übertragen")
        XCTAssertEqual(raw.split(separator: "\n").count, 2)
    }

    /// Unlesbares und Fremdes bleibt beim Schreiben Byte für Byte erhalten (nie Datenverlust durch die Übertragung).
    func testMigrationKeepsUnreadableAndFutureLinesVerbatim() async throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(AssistantAuditLog.fileName)
        let future = #"{"at":"2030-01-01T00:00:00Z","kind":"answer","neuesFeld":{"x":1}}"#
        let broken = #"{"at":"2025-10-"#
        let brokenModel = #"{"model":"Fremd","kind":"gibtEsNicht"}"#
        try Data([legacyLine, future, broken, brokenModel].joined(separator: "\n").appending("\n").utf8).write(to: url)
        let log = AssistantAuditLog(directory: dir)
        let ok = await log.record(entry(1))
        XCTAssertTrue(ok)
        let lines = try String(contentsOf: url, encoding: .utf8).split(separator: "\n").map(String.init)
        XCTAssertEqual(lines.count, 5)
        XCTAssertFalse(lines[0].contains("Foundation"))
        XCTAssertEqual(Array(lines[1...3]), [future, broken, brokenModel], "nur bekannte alte Zeilen werden übertragen")
        let read = await log.read()
        XCTAssertEqual(read.skippedLines, 2)
        XCTAssertEqual(read.entries.count, 3)
    }

    func testLegacyLineWithMinimalFieldsLoads() async throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let line = #"{"at":"2025-10-09T09:20:00Z","kind":"unsourcedOn"}"#
        try Data((line + "\n").utf8).write(to: dir.appendingPathComponent(AssistantAuditLog.fileName))
        let all = await AssistantAuditLog(directory: dir).entries()
        XCTAssertEqual(all.first?.kind, .unsourcedOn)
        XCTAssertEqual(all.first?.counts, AssistantSentenceCounts())
        XCTAssertNil(all.first?.level, "nur Antworten haben eine Stufe")
    }

    // MARK: Stufe, Spur, Kennungen

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
        // Auch beim Laden: eine gespeicherte Stufe S0 mit Sätzen ohne Quelle wird zu S1.
        let json = #"{"at":"2025-10-09T09:20:00Z","kind":"answer","level":"s0","counts":{"unsourced":1}}"#
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        XCTAssertEqual(try d.decode(AssistantAuditEntry.self, from: Data(json.utf8)).level, .s1)
    }

    func testTraceIsCappedAndOverflowIsCounted() {
        let steps = (1...40).map { AssistantTraceStep(n: $0, verdict: .droppedNoMode, generation: 1) }
        let e = AssistantAuditEntry(kind: .answer, trace: steps)
        XCTAssertEqual(e.trace.count, AssistantAuditEntry.maxTrace)
        XCTAssertEqual(e.trace.last?.n, AssistantAuditEntry.maxTrace)
        XCTAssertEqual(e.traceOverflow, 40 - AssistantAuditEntry.maxTrace)
        let more = AssistantAuditEntry(kind: .answer, trace: steps, traceOverflow: 5)
        XCTAssertEqual(more.traceOverflow, 5 + 40 - AssistantAuditEntry.maxTrace)
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

    func testModelIsAControlledKindNotFreeText() throws {
        XCTAssertEqual(AssistantModelKind.classify(label: "Apple-Systemmodell · lokal"), .apple)
        XCTAssertEqual(AssistantModelKind.classify(label: "Gemma 3n"), .gemma)
        XCTAssertEqual(AssistantModelKind.classify(label: "Mein Geheimes Modell 7"), .other)
        let e = AssistantAuditEntry(kind: .answer, model: "Mein Geheimes Modell 7", cloud: true)
        let raw = String(decoding: try JSONEncoder().encode(e), as: UTF8.self)
        XCTAssertFalse(raw.contains("Geheim"))
        XCTAssertTrue(raw.contains("\"modelKind\":\"other\""))
        XCTAssertTrue(raw.contains("\"cloud\":true"))
    }

    func testConsentAndRevocationEntriesCarryTheNewGeneration() async {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 2, generation: 7))
        _ = await log.record(AssistantAuditEntry(kind: .unsourcedOff, consentVersion: 2, generation: 8))
        _ = await log.record(AssistantAuditEntry(kind: .cloudRevoked, cloud: true, generation: 9))
        let all = await log.entries()
        XCTAssertEqual(all.map(\.kind), [.unsourcedOn, .unsourcedOff, .cloudRevoked])
        XCTAssertEqual(all.map(\.generation), [7, 8, 9])
    }

    // MARK: Keine Inhalte

    /// Das Protokoll hat gar kein Feld für Text: Schlüssel und alle Zeichenketten stammen aus festen Listen.
    func testEntryHasOnlyTheAgreedKeysAndOnlyControlledStrings() throws {
        let e = entry(1)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let object = try JSONSerialization.jsonObject(with: encoder.encode(e)) as? [String: Any] ?? [:]
        XCTAssertEqual(Set(object.keys), ["at", "kind", "modelKind", "cloud", "unsourcedMode", "consentVersion", "round", "level",
                                          "generation", "counts", "areas", "outcome", "trace", "history"])
        let counts = object["counts"] as? [String: Any] ?? [:]
        XCTAssertEqual(Set(counts.keys), ["grounded", "inventory", "unsourced", "neutral", "droppedOwnData", "droppedNoMode",
                                          "droppedNotSnapped", "droppedDuplicate", "droppedTemplate", "revoked"])
        let step = (object["trace"] as? [[String: Any]])?.first ?? [:]
        XCTAssertEqual(Set(step.keys), ["n", "v", "g", "c"])

        var strings: [String] = []
        func walk(_ any: Any) {
            if let s = any as? String { strings.append(s) }
            else if let a = any as? [Any] { a.forEach(walk) }
            else if let d = any as? [String: Any] { d.values.forEach(walk) }
        }
        walk(object)
        let allowed: Set<String> = ["answer", "other", "data", "s1", "s0", "completed", "books", "notes", "grounded", "e+2", "h+2",
                                    "2025-10-09T08:53:21Z"]
        for s in strings where !allowed.contains(s) {
            XCTFail("unerwartete Zeichenkette im Protokoll: \(s)")
        }
    }

    /// Negativtest: Frage, Antwort, Titel und Auszug einer echten Antwort dürfen nirgends im Protokoll stehen.
    func testAuditEntryOfARealAnswerContainsNoQuestionAnswerTitleOrExcerpt() async throws {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", detail: "Autorin Zimtstern", sections: [
            ("S. 7", "Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."),
        ]))
        let audit = AssistantAuditLog(directory: dir)
        let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Wurzelbrunst",
                                     timeout: 5, allowUnsourced: true, consentVersion: 1)
        let question = "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"
        let session = AssistantSession(model: AuditModel(pieces: ["Quasselbert wandert nach Pappelhausen. Kieselsteine sind meistens rund."]),
                                       knowledge: AssistantKnowledge(corpus: corpus), config: config, audit: audit)
        var spoken: [String] = []
        for await e in session.events(AssistantRequest(requestId: "r1", text: question)) {
            if case .sentence(_, _, let t, _, let st, _, _) = e, st != .announcement { spoken.append(t) }
        }
        XCTAssertEqual(spoken, ["Quasselbert wandert mit dem Fahrrad nach Pappelhausen und trifft dort Tante Lavendula."])
        await session.commit(requestId: "r1", user: question, spoken: spoken.joined(separator: " "), interrupted: false)
        _ = await audit.record(AssistantAuditEntry(kind: .unsourcedOn, consentVersion: 1))
        let raw = try String(contentsOf: audit.url, encoding: .utf8)
        for word in ["Quasselbert", "Pappelhausen", "Fahrrad", "Kieselsteine", "Flimmerkasten", "Zimtstern", "Lavendula", "Wurzelbrunst", "wandert",
                     "Wohin", "Titel", "S. 7"] {
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

    // MARK: Zusammenspiel mit der Sitzung

    func testInterruptedAndRevokedOutcomes() async {
        let corpus = AssistantCorpus()
        let audit = AssistantAuditLog(directory: dir)
        let config = AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 16_000, maxAnswerTokens: 700, userName: nil, timeout: 5,
                                     allowUnsourced: true, consentVersion: 1)
        let session = AssistantSession(model: AuditModel(pieces: ["Kieselsteine sind meistens rund."]), knowledge: AssistantKnowledge(corpus: corpus),
                                       config: config, audit: audit)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        await session.commit(requestId: "a", user: "Hallo", spoken: "Kieselsteine sind meistens rund.", interrupted: true)
        for await _ in session.events(AssistantRequest(requestId: "b", text: "Hallo")) {}
        await session.commit(requestId: "b", user: "Hallo", spoken: "Kieselsteine sind meistens rund.", interrupted: true, revoked: true)
        let all = await audit.entries()
        XCTAssertEqual(all.map(\.outcome), [.interrupted, .revoked])
        XCTAssertEqual(all.map(\.cloud), [true, true])
        XCTAssertEqual(all.map(\.round), [.general, .general])
    }

    /// Auch eine Anfrage ohne gesprochenen Text bekommt ihren Eintrag (Fehler, Abbruch, Widerruf).
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

    func testAnAuditFailureDoesNotLoseTheHistoryTurn() async throws {
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
        XCTAssertEqual(history.count, 1, "nur das Protokoll fehlt, der Beitrag steht trotzdem im Verlauf")
    }
}
