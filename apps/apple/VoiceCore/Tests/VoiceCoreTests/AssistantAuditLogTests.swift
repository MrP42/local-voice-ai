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

    private func entry(_ n: Int, kind: AssistantAuditEntry.Kind = .answer) -> AssistantAuditEntry {
        AssistantAuditEntry(at: Date(timeIntervalSince1970: 1_760_000_000 + Double(n)), kind: kind, model: "Test", cloud: n % 2 == 0,
                            unsourcedMode: n % 3 == 0, consentVersion: 1, grounded: n, unsourced: 1, neutral: 2, dropped: 3,
                            areas: [.notes, .books, .books], outcome: .completed)
    }

    func testRoundTripAndAreasAreSortedAndUnique() async {
        let log = AssistantAuditLog(directory: dir)
        let initial = await log.entries()
        XCTAssertEqual(initial, [])
        let ok = await log.record(entry(1))
        XCTAssertTrue(ok)
        _ = await log.record(entry(2, kind: .unsourcedOn))
        let loaded = await AssistantAuditLog(directory: dir).entries()
        XCTAssertEqual(loaded, [entry(1), entry(2, kind: .unsourcedOn)])
        XCTAssertEqual(loaded[0].areas, ["books", "notes"])
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
        XCTAssertEqual(all.first?.grounded, 20)
        XCTAssertEqual(all.last?.grounded, 519)
    }

    /// Mehrere Schreiber gleichzeitig (Antworten und Einwilligung): nichts geht verloren, nichts wird zerrissen.
    func testConcurrentWritersLoseNothing() async {
        let log = AssistantAuditLog(directory: dir)
        let entries = (0..<60).map { entry($0) }
        await withTaskGroup(of: Void.self) { group in
            for e in entries { group.addTask { _ = await log.record(e) } }
        }
        let all = await log.entries()
        XCTAssertEqual(all.count, 60)
        XCTAssertEqual(Set(all.map(\.grounded)), Set(0..<60))
    }

    /// Ein abgebrochener Schreibvorgang hinterlässt eine halbe letzte Zeile: Sie wird übersprungen, der Rest bleibt lesbar,
    /// und der nächste Eintrag landet sauber dahinter.
    func testTruncatedLastLineIsSkippedAndTheRestSurvives() async throws {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(entry(1))
        _ = await log.record(entry(2))
        var data = try Data(contentsOf: log.url)
        data.append(Data(#"{"at":"2025-10-"#.utf8))
        try data.write(to: log.url)
        var all = await log.entries()
        XCTAssertEqual(all.map(\.grounded), [1, 2])
        _ = await log.record(entry(3))
        all = await log.entries()
        XCTAssertEqual(all.map(\.grounded), [1, 2, 3])
    }

    /// Schreibfehler (voller Datenträger, fehlende Rechte; hier: ein Verzeichnis an der Stelle der Datei): `record` meldet
    /// `false`, nichts stürzt, andere Protokolle bleiben unberührt. Dass bei einem Abbruch mitten im Schreiben die alte Datei
    /// ganz bleibt, leistet `.atomic` (temporäre Datei, dann Umbenennen); `testNoStrayTemporaryFilesAfterWriting` prüft,
    /// dass dabei nichts liegen bleibt.
    func testFailedWriteIsReportedAndOtherLogsAreUnaffected() async throws {
        let log = AssistantAuditLog(directory: dir)
        _ = await log.record(entry(1))
        let before = try Data(contentsOf: log.url)
        let blocked = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-blocked-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: blocked) }
        // Ein Verzeichnis an der Stelle der Datei: Lesen und Ersetzen scheitern, wie bei einem Schreibfehler.
        try FileManager.default.createDirectory(at: blocked.appendingPathComponent(AssistantAuditLog.fileName), withIntermediateDirectories: true)
        let broken = AssistantAuditLog(directory: blocked)
        let ok = await broken.record(entry(2))
        XCTAssertFalse(ok)
        XCTAssertEqual(try Data(contentsOf: log.url), before)
        let stillThere = await log.record(entry(3))
        XCTAssertTrue(stillThere, "ein Fehler an anderer Stelle beeinträchtigt dieses Protokoll nicht")
    }

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

    // MARK: Keine Inhalte

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
            if case .sentence(_, let t, _, let st, _) = e, st != .announcement { spoken.append(t) }
        }
        XCTAssertEqual(spoken.count, 2)
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
        XCTAssertEqual(entries[0].model, "Test")
        XCTAssertEqual(entries[0].grounded, 1)
        XCTAssertEqual(entries[0].unsourced, 1)
        XCTAssertEqual(entries[0].dropped, 0)
        XCTAssertEqual(entries[0].areas, ["books"])
        XCTAssertTrue(entries[0].unsourcedMode)
        XCTAssertFalse(entries[0].cloud)
        XCTAssertEqual(entries[0].consentVersion, 1)
        XCTAssertEqual(entries[0].outcome, .completed)
        XCTAssertEqual(entries[1].kind, .unsourcedOn)
    }

    /// Das Protokoll hat gar kein Feld für Text: die Schlüssel sind eine feste, kurze Liste.
    func testEntryHasOnlyTheAgreedKeys() throws {
        let data = try JSONEncoder().encode(AssistantAuditEntry(kind: .answer, model: "M", areas: [.books], outcome: .interrupted))
        let keys = Set((try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]).keys)
        XCTAssertEqual(keys, ["at", "kind", "model", "cloud", "unsourcedMode", "consentVersion", "grounded", "unsourced", "neutral",
                              "dropped", "areas", "outcome"])
    }

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
    }

    func testNoEntryWithoutSpokenText() async {
        let audit = AssistantAuditLog(directory: dir)
        let session = AssistantSession(model: AuditModel(pieces: ["Hallo."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5),
                                       audit: audit)
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        await session.commit(requestId: "a", user: "Hallo", spoken: "", interrupted: true)
        let all = await audit.entries()
        XCTAssertEqual(all, [])
    }

    func testAnAuditFailureDoesNotLoseTheHistoryTurn() async throws {
        let blocked = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-audit-blocked-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: blocked) }
        try FileManager.default.createDirectory(at: blocked.appendingPathComponent(AssistantAuditLog.fileName), withIntermediateDirectories: true)
        let session = AssistantSession(model: AuditModel(pieces: ["Hallo zusammen."]), knowledge: AssistantKnowledge(corpus: AssistantCorpus()),
                                       config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5),
                                       audit: AssistantAuditLog(directory: blocked))
        for await _ in session.events(AssistantRequest(requestId: "a", text: "Hallo")) {}
        let wrote = await session.commit(requestId: "a", user: "Hallo", spoken: "Hallo zusammen.", interrupted: false)
        XCTAssertFalse(wrote)
        let history = await session.history
        XCTAssertEqual(history.count, 1)
    }
}
