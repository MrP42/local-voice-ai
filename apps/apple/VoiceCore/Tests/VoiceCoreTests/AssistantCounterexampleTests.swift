import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Runde 7): die Gegenbeispiele der Prüfung, wörtlich als Tests. Jeder Test trägt den
// Befund (B1 bis B10) im Namen und zeigt das Gegenbeispiel, das gegen den Stand 5080d95 rot war. Nur synthetische Daten.

private struct Injected: Error {}

/// Eine gemeinsame Ereignisfolge (Dateisystem des Speichers und des Protokolls, Zeilen), um Reihenfolgen zu prüfen.
private final class Recorder: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String] = []
    func add(_ s: String) { lock.withLock { items.append(s) } }
    func reset() { lock.withLock { items.removeAll() } }
    var all: [String] { lock.withLock { items } }
}

private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var flag = false
    var value: Bool {
        get { lock.withLock { flag } }
        set { lock.withLock { flag = newValue } }
    }
}

private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    func bump() { lock.withLock { n += 1 } }
    var value: Int { lock.withLock { n } }
}

/// Dateisystem mit Fehlerinjektion je Teilschritt (Speicher UND Protokoll teilen das Protokoll `AssistantStoreFileSystem`).
private final class CxFS: AssistantStoreFileSystem, @unchecked Sendable {
    private let base = AssistantPosixStoreFileSystem()
    private let name: String
    private let rec: Recorder?
    private let lock = NSLock()
    private var failing: [AssistantStoreStep: Int] = [:]

    init(_ name: String, _ rec: Recorder? = nil) {
        self.name = name
        self.rec = rec
    }

    /// Die nächsten `times` Aufrufe dieses Teilschritts scheitern.
    func fail(_ step: AssistantStoreStep, times: Int = .max) { lock.withLock { failing[step] = times } }
    func heal() { lock.withLock { failing.removeAll() } }

    private func gate(_ step: AssistantStoreStep) throws {
        rec?.add("\(name):\(step.rawValue)")
        let hit = lock.withLock { () -> Bool in
            guard let left = failing[step], left > 0 else { return false }
            failing[step] = left - 1
            return true
        }
        if hit { throw Injected() }
    }

    func read(_ url: URL) throws -> Data? { try base.read(url) }
    func exists(_ url: URL) -> Bool { base.exists(url) }
    func createDirectory(_ url: URL) throws { try gate(.prepare); try base.createDirectory(url) }
    func writeNew(_ url: URL, _ data: Data) throws { try gate(.write); try base.writeNew(url, data) }
    func syncFile(_ url: URL) throws { try gate(.syncFile); try base.syncFile(url) }
    func rename(_ from: URL, to: URL) throws { try gate(.rename); try base.rename(from, to: to) }
    func syncDirectory(_ url: URL) throws { try gate(.syncDirectory); try base.syncDirectory(url) }
    func remove(_ url: URL) { base.remove(url) }
}

private struct EchoModel: BookTalkModel {
    var text = "Hallo."
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let text = text
        return AsyncThrowingStream { c in
            c.yield(text)
            c.finish()
        }
    }
}

/// Zählt die Aufrufe und antwortet nie (zum Beweis „kein Modell“).
private final class CountingModel: BookTalkModel, @unchecked Sendable {
    let calls = Counter()
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        calls.bump()
        return AsyncThrowingStream { $0.finish() }
    }
}

/// Wartet (bis zum Abbruch), damit eine Anfrage im Modellaufruf steht.
private struct SleepyModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { c in
            let task = Task {
                try? await Task.sleep(for: .seconds(20))
                c.finish()
            }
            c.onTermination = { _ in task.cancel() }
        }
    }
}

/// Liest eine Anfrage bis zum Ende; gibt die Ereignisse und den Grund eines Fehlers zurück.
private func drainEvents(_ session: AssistantSession, _ request: AssistantRequest) async -> (events: [AssistantEvent], failure: AssistantFailure?) {
    var events: [AssistantEvent] = []
    var failure: AssistantFailure?
    for await e in session.events(request) {
        events.append(e)
        if case .failed(_, let f) = e { failure = f }
    }
    return (events, failure)
}

private func sentences(_ s: AssistantSession, _ id: String, _ text: String) async -> [String] {
    var out: [String] = []
    for await e in s.events(AssistantRequest(requestId: id, text: text)) {
        if case .sentence(_, _, let sentence, _, _, _, _) = e { out.append(sentence) }
    }
    return out
}

final class AssistantCounterexampleTests: XCTestCase {
    private var dir: URL!
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil,
                                         timeout: 5)
    private var cloudConfig: AssistantConfig {
        AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
    }

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-cx-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDown() { try? FileManager.default.removeItem(at: dir) }

    // MARK: Hilfen

    private var stateFile: URL { dir.appendingPathComponent(AssistantReleaseStore.fileName) }
    private var protocolFile: URL { dir.appendingPathComponent(AssistantAuditLog.fileName) }

    private func seed(_ record: AssistantReleaseRecord) throws {
        try XCTUnwrap(AssistantReleaseFile.encode(record, revision: 5)).write(to: stateFile)
    }

    private func fileRecord() -> AssistantReleaseRecord? {
        guard let data = try? Data(contentsOf: stateFile) else { return nil }
        return AssistantReleaseFile.decode(data)?.record
    }

    private func openStore(_ fs: any AssistantStoreFileSystem = AssistantPosixStoreFileSystem()) -> AssistantReleaseStore {
        AssistantReleaseStore(directory: dir, protocolURL: protocolFile, fileSystem: fs)
    }

    private func log(fileSystem: any AssistantStoreFileSystem = AssistantPosixStoreFileSystem(),
                     appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile) -> AssistantAuditLog {
        AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile, appender: appender, fileSystem: fileSystem)
    }

    private func session(_ store: AssistantReleaseStore?, audit: AssistantAuditLog? = nil, model: any BookTalkModel = EchoModel(),
                         knowledge: AssistantKnowledge? = nil, config: AssistantConfig? = nil) -> AssistantSession {
        AssistantSession(model: model, knowledge: knowledge ?? AssistantKnowledge(corpus: AssistantCorpus()), config: config ?? self.config,
                         audit: audit, released: true, releaseStore: store)
    }

    private func lines(_ kinds: Set<String>? = nil) -> [[String: Any]] {
        ((try? String(contentsOf: protocolFile, encoding: .utf8)) ?? "").split(separator: "\n").compactMap {
            (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any]
        }.filter { kinds == nil || kinds!.contains($0["t"] as? String ?? "") }
    }

    // MARK: B1: der Block wird zeichengleich mit dem Gespeicherten ausgegeben

    /// Die ausgegebenen Zitate (Status `grounded`) einer Anfrage über genau diese Dokumente.
    private func quotes(of documents: [AssistantDocument], question: String, answer: String) async -> [String] {
        let corpus = AssistantCorpus()
        for document in documents { await corpus.upsert(document) }
        let s = session(nil, model: EchoModel(text: answer), knowledge: AssistantKnowledge(corpus: corpus))
        var out: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: question)) {
            if case .sentence(_, _, let text, _, let status, _, _) = e, status == .grounded { out.append(text) }
        }
        return out
    }

    private func note(_ stored: String, id: String = "n1", version: String = "1") -> AssistantDocument {
        AssistantDocument(item: AssistantItem(area: .notes, id: id, title: "Notiz vom 11.11.2026"), version: version,
                          sections: AssistantSections.blocks(of: stored, locator: "11.11.2026, 14:03"))
    }

    /// B1: Die Notiz ist „Alpha\rBeta“ (ein Zeilenende `\r`). Ausgegeben wird GENAU diese Bytefolge. Gegen 5080d95 rot: Der Kern
    /// setzte die Zeilen mit `\n` zusammen und gab „Alpha\nBeta“ aus, also nicht den gespeicherten Text.
    func testB1ANoteStoredAsAlphaCarriageReturnBetaIsOutputWithTheCarriageReturn() async {
        let out = await quotes(of: [note("Alpha\rBeta")], question: "Alpha Beta?", answer: "Alpha Beta")
        XCTAssertEqual(out.count, 1, "das Zitat kommt: \(out)")
        XCTAssertEqual(out.first.map { Array($0.utf8) }, Array("Alpha\rBeta".utf8), "Bytes wie gespeichert, kein `\\n` statt `\\r`")
    }

    /// B1: Zeilenenden (`\r\n`), doppelte Leerzeichen, Tabs, NFD-Umlaute und Rand-Leerraum bleiben, wie gespeichert; ein Trenner
    /// (Leerzeile) gehört nie zum Block, ein Zeilenende darin schon.
    func testB1WhitespaceAndUnicodeFormOfTheStoredNoteSurviveByteForByte() async {
        let cases: [(name: String, stored: String, expected: String)] = [
            ("CRLF", "Quasselbert wandert mit dem Fahrrad\r\nnach Pappelhausen.", "Quasselbert wandert mit dem Fahrrad\r\nnach Pappelhausen."),
            ("doppelte Leerzeichen", "Quasselbert  wandert mit dem  Fahrrad nach Pappelhausen.", "Quasselbert  wandert mit dem  Fahrrad nach Pappelhausen."),
            ("Tab", "Quasselbert\twandert mit dem Fahrrad\tnach Pappelhausen.", "Quasselbert\twandert mit dem Fahrrad\tnach Pappelhausen."),
            ("NFD", "Quasselbert wandert mit dem Fahrrad nach Mu\u{0308}hlbach.", "Quasselbert wandert mit dem Fahrrad nach Mu\u{0308}hlbach."),
            ("Rand-Leerraum", "Erster Block.\n\n  Quasselbert wandert mit dem Fahrrad nach Pappelhausen.  \n\nDritter Block.",
             "  Quasselbert wandert mit dem Fahrrad nach Pappelhausen.  "),
            ("U+2028", "Quasselbert wandert mit dem Fahrrad\u{2028}nach Pappelhausen.", "Quasselbert wandert mit dem Fahrrad\u{2028}nach Pappelhausen."),
        ]
        for c in cases {
            let out = await quotes(of: [note(c.stored)], question: "Wohin wandert Quasselbert mit dem Fahrrad?",
                                   answer: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen.")
            XCTAssertEqual(out.map { Array($0.utf8) }, [Array(c.expected.utf8)], c.name)
        }
    }

    /// B1, Quellart Besprechung: Ausgegeben wird das gespeicherte Textfeld des Segments (ohne Sprecher, nicht getrimmt), nicht die
    /// Suchzeile „Anna: …“; ohne Rohtext kommt das Segment nie.
    func testB1ATranscriptSegmentIsOutputAsItsStoredTextFieldAndNeverAsTheSearchLine() async {
        let stored = " Quasselbert wandert  mit dem Fahrrad\r\nnach Pappelhausen. "
        func meeting(raw: String?) -> AssistantDocument {
            AssistantDocument(item: AssistantItem(area: .transcripts, id: "t1", title: "Besprechung Nordlicht"), version: "1",
                              sections: AssistantSections.meeting([AssistantSegment(start: 3, line: "Anna: " + stored.trimmingCharacters(in: .whitespaces),
                                                                                   raw: raw, usable: true)]) { "00:00:0\(Int($0))" })
        }
        let answer = "Quasselbert wandert mit dem Fahrrad nach Pappelhausen."
        let question = "Wohin wandert Quasselbert mit dem Fahrrad?"
        let out = await quotes(of: [meeting(raw: stored)], question: question, answer: answer)
        XCTAssertEqual(out.map { Array($0.utf8) }, [Array(stored.utf8)])
        let without = await quotes(of: [meeting(raw: nil)], question: question, answer: answer)
        XCTAssertTrue(without.isEmpty, "kein eindeutiger Rohtext: nicht zitierbar, nie die aufbereitete Fassung: \(without)")
    }

    /// B1, Quellart Buch: Der Absatz wird aus dem gespeicherten Buchtext geschnitten (Zeilenende und doppelte Leerzeichen bleiben);
    /// enthält er Stil- oder Sprechermarken, gibt es keinen eindeutigen Rohtext: der Absatz kommt nie.
    func testB1ABookParagraphIsCutFromTheStoredBookTextAndOneWithMarksIsNeverQuoted() async {
        func book(_ paragraph: String) -> AssistantDocument {
            AssistantDocument(item: AssistantItem(area: .books, id: "b1", title: "Der Titel Flimmerkasten"), version: "1",
                              sections: AssistantSections.book(BookTalkIndex.blocks(in: "# Kapitel Eins\r\n\r\n\(paragraph)\r\n"), frontier: nil))
        }
        let question = "Wohin wandert Quasselbert mit dem Fahrrad?"
        let answer = "Quasselbert wandert mit dem Fahrrad nach Pappelhausen."
        let plain = "Quasselbert  wandert mit dem Fahrrad\r\nnach Pappelhausen."
        let out = await quotes(of: [book(plain)], question: question, answer: answer)
        XCTAssertEqual(out.map { Array($0.utf8) }, [Array(plain.utf8)])
        for marked in ["<Erzähler> Quasselbert wandert mit dem Fahrrad nach Pappelhausen.",
                       "Quasselbert [calm] wandert mit dem Fahrrad nach Pappelhausen."] {
            let none = await quotes(of: [book(marked)], question: question, answer: answer)
            XCTAssertTrue(none.isEmpty, "Absatz mit Marken ist nicht zitierbar: \(none)")
        }
    }

    /// B1: Erzeugter Text, Mail und Kalender kommen nie, auch wenn der Aufrufer einen Rohtext mitgibt.
    func testB1GeneratedMailAndCalendarBlocksAreNeverQuotedEvenWithARawText() async {
        for kind in [AssistantSourceKind.generated, .mail, .calendar] {
            let text = "Quasselbert wandert mit dem Fahrrad nach Pappelhausen."
            let document = AssistantDocument(item: AssistantItem(area: .notes, id: "n-\(kind.rawValue)", title: "Notiz"), version: "1",
                                             sections: [.init(locator: nil, text: text, kind: kind, raw: text)])
            let out = await quotes(of: [document], question: "Wohin wandert Quasselbert mit dem Fahrrad?", answer: text)
            XCTAssertTrue(out.isEmpty, "\(kind): \(out)")
        }
    }

    // MARK: B2: das Protokoll ist dauerhaft

    /// B2: Die Zeile „beabsichtigt“ steht dauerhaft (fsync der Datei), BEVOR der Austausch der Zustandsdatei beginnt. Gegen 5080d95 rot:
    /// Der Schreiber sicherte nie (kein fsync), die Zeile konnte nach einem Stromausfall fehlen, obwohl die Datei schon ersetzt war.
    func testB2TheIntentLineIsDurableBeforeTheStateFileReplaceBegins() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books], cloudAreas: [.books]))
        let rec = Recorder()
        let storeFS = CxFS("store", rec), journalFS = CxFS("journal", rec)
        let store = openStore(storeFS)
        let audit = log(fileSystem: journalFS)
        let s = session(store, audit: audit)
        _ = await audit.flushReleaseLines()   // die Standzeile ist durch
        rec.reset()
        let result = await s.setAreas([.books, .notes])
        XCTAssertTrue(result.effective)
        let events = rec.all
        let intentSynced = events.firstIndex(of: "journal:syncFile")
        let replace = events.firstIndex(of: "store:rename")
        XCTAssertNotNil(intentSynced, "die Zeile wurde dauerhaft gesichert")
        XCTAssertNotNil(replace)
        XCTAssertLessThan(intentSynced ?? Int.max, replace ?? 0, "erst die gesicherte Zeile, dann der Austausch")
        XCTAssertEqual(lines(["intent", "effective"]).compactMap { $0["t"] as? String }, ["intent", "effective"])
    }

    /// B2: Scheitert das fsync der Pflichtzeile, gilt sie als nicht geschrieben: kein Modell, keine Ausgabe, Ausgang „Fehler“. Die
    /// Laufnummer wird nicht doppelt vergeben: Der zweite Versuch trägt dieselbe, ein neuer Prozesslauf die nächste. Gegen 5080d95 rot:
    /// ohne fsync gab es keinen Fehler, die Anfrage lief.
    func testB2AFailingFsyncOfTheMandatoryLineMeansNoModelAndTheRunNumberIsNotDuplicatedAfterALostSave() async throws {
        let journalFS = CxFS("journal")
        let store = openStore()
        let audit = log(fileSystem: journalFS)
        let model = CountingModel()
        journalFS.fail(.syncFile, times: 1)
        let s = session(store, audit: audit, model: model)
        let first = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(first.failure, .auditUnavailable)
        XCTAssertEqual(model.calls.value, 0, "kein Modell")
        XCTAssertTrue(first.events.allSatisfy { if case .sentence = $0 { false } else { true } }, "keine Ausgabe")
        let second = await drainEvents(s, AssistantRequest(requestId: "r2", text: "Hallo"))
        XCTAssertNotEqual(second.failure, .auditUnavailable, "der nächste Versuch gelingt mit dem dann gültigen Stand")
        XCTAssertEqual(model.calls.value, 1, "jetzt läuft das Modell")
        let runs = lines(["state"]).compactMap { $0["run"] as? Int }
        XCTAssertEqual(runs, [1, 1], "dieselbe Laufnummer für den zweiten Versuch")
        // Ein neuer Prozesslauf (neue Ordnung, neuer Schreiber) bekommt die nächste Nummer, nie dieselbe.
        let next = session(openStore(), audit: log())
        _ = await drainEvents(next, AssistantRequest(requestId: "r3", text: "Hallo"))
        XCTAssertEqual(lines(["state"]).compactMap { $0["run"] as? Int }, [1, 1, 2])
    }

    /// B2: Das Verdichten scheitert in jedem Teilschritt, ohne die alte Datei zu verlieren (eigene Datei, fsync, rename, fsync des
    /// Verzeichnisses). Gegen 5080d95 rot: Das Verdichten ersetzte die Datei ohne eigene Sicherung der Teilschritte.
    func testB2CompactionFailsPerStepWithoutLosingTheOldFile() throws {
        let lineText = "{\"t\":\"event\",\"kind\":\"unsourcedOn\"}\n"
        let original = String(repeating: lineText, count: AssistantAuditWriter.maxUnits + 40)
        for step in [AssistantStoreStep.prepare, .write, .syncFile, .rename] {
            try? FileManager.default.removeItem(at: dir)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try original.write(to: protocolFile, atomically: true, encoding: .utf8)
            let fs = CxFS("journal")
            fs.fail(step, times: 1)
            let audit = log(fileSystem: fs)
            var line = AssistantJournalLine(.event)
            line.at = Date()
            audit.enqueue(line)
            let finished = expectation(description: "geschrieben \(step)")
            Task {
                _ = await audit.flushReleaseLines()
                finished.fulfill()
            }
            wait(for: [finished], timeout: 10)
            let after = try String(contentsOf: protocolFile, encoding: .utf8)
            XCTAssertTrue(after.hasPrefix(original), "\(step): die alte Datei ist unversehrt, nur angehängt wird")
            XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent(AssistantAuditWriter.compactName).path),
                           "\(step): keine Temp-Datei bleibt")
        }
        // Gelingt jeder Schritt, ist die Datei verdichtet und vollständig lesbar.
        try? FileManager.default.removeItem(at: dir)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try original.write(to: protocolFile, atomically: true, encoding: .utf8)
        let audit = log()
        audit.enqueue(AssistantJournalLine(.event))
        let done = expectation(description: "verdichtet")
        Task {
            _ = await audit.flushReleaseLines()
            done.fulfill()
        }
        wait(for: [done], timeout: 10)
        let compacted = try String(contentsOf: protocolFile, encoding: .utf8).split(separator: "\n")
        XCTAssertLessThanOrEqual(compacted.count, AssistantAuditWriter.maxUnits + 1)
    }

    // MARK: B3: ein Schreibfehler stoppt die Ausgabe sofort

    /// B3: Der Audiostart der Einheit 1 gelingt, seine Ergebniszeile `a+` lässt sich nicht schreiben (während der Wiedergabe). Danach
    /// startet die Einheit 2 NICHT: kein Body, Ergebnis „Protokollfehler“ (nicht „widerrufen“). Gegen 5080d95 rot: Die Einheit 2 startete,
    /// weil der Kern den klebrigen Fehler nur meldete, nachdem sie ausgelöst war.
    func testB3AWriteErrorOfTheStartLineOfUnitOneStopsUnitTwo() async {
        let fail = Flag()
        let audit = log(appender: { url, data in
            if fail.value, String(decoding: data, as: UTF8.self).contains("\"mark\":\"a+") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(nil, audit: audit)
        s.admitForTesting(requestId: "r1")
        s.admitForTesting(requestId: "r1")
        let g = s.releaseGeneration
        fail.value = true
        let started = Counter()
        let one = s.commitStart(requestId: "r1", index: 0, generation: g) { started.bump(); return true }
        XCTAssertTrue(one.ok, "die Einheit 1 begann")
        let healthy = await s.auditHealthy(requestId: "r1")
        XCTAssertFalse(healthy, "die Zeile `a+` der Einheit 1 ließ sich nicht schreiben")
        let two = s.commitStart(requestId: "r1", index: 1, generation: g) { started.bump(); return true }
        XCTAssertEqual(started.value, 1, "die Einheit 2 startet nicht")
        XCTAssertFalse(two.ok)
        XCTAssertTrue(two.auditFailed, "Protokollfehler, nicht Widerruf")
        let ran = Counter()
        let shown = s.commitOutput(requestId: "r1", index: 1, generation: g, stage: .display) { ran.bump() }
        XCTAssertEqual(ran.value, 0, "auch die Anzeige löst nichts mehr aus")
        XCTAssertTrue(shown.auditFailed)
        XCTAssertTrue(s.hasAuditFailure(requestId: "r1"))
    }

    /// B3: Ein Schreibfehler geht dem Widerruf voran: Eine Anfrage mit Schreibfehler UND Widerruf endet mit „Fehler“.
    func testB3AWriteErrorPrecedesRevokedInTheOutcome() async {
        let fail = Flag()
        let audit = log(appender: { url, data in
            if fail.value, String(decoding: data, as: UTF8.self).contains("\"mark\":\"a+") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(nil, audit: audit)
        _ = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        s.admitForTesting(requestId: "r1")
        fail.value = true
        _ = s.commitStart(requestId: "r1", index: 0, generation: s.releaseGeneration) { true }
        _ = await s.auditHealthy(requestId: "r1")
        await s.commit(requestId: "r1", user: "x", spoken: "", interrupted: false, revoked: true)
        let outcome = await audit.entries().first { $0.requestId == "r1" }?.outcome
        XCTAssertEqual(outcome, .failed, "Fehler, nicht „widerrufen“")
    }

    /// B3: Der Controller erfährt den Schreibfehler SOFORT (Beobachter am Protokoll), nicht erst beim Start der nächsten Einheit:
    /// Er stoppt damit die laufende Wiedergabe und verwirft die Wartenden. Der Schlüssel ist die Anfrage; ein Fehler einer fremden
    /// Zeile (ohne Anfrage) trägt einen anderen Schlüssel.
    func testB3TheObserverOfTheProtocolIsToldTheMomentAStartLineFailsDuringPlayback() async {
        let fail = Flag()
        let audit = log(appender: { url, data in
            if fail.value, String(decoding: data, as: UTF8.self).contains("\"mark\":\"a+") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let told = Recorder()
        audit.setFailureObserver { told.add($0) }
        let s = session(nil, audit: audit)
        s.admitForTesting(requestId: "r1")
        let g = s.releaseGeneration
        fail.value = true
        XCTAssertTrue(s.commitStart(requestId: "r1", index: 0, generation: g) { true }.ok, "die Einheit 1 begann (sie klingt)")
        _ = await s.auditHealthy(requestId: "r1")
        XCTAssertEqual(told.all, ["r1"], "der Beobachter wurde gerufen, ohne dass eine weitere Einheit nötig war")
        XCTAssertTrue(s.hasAuditFailure(requestId: "r1"))
    }

    // MARK: B4: der Verlauf hat sein Ergebnis je Einheit

    /// B4: Zwei Blöcke, nur der erste steht im Verlauf (die Antwort wurde nach dem ersten unterbrochen). Der Leser meldet je Einheit
    /// Anzeige, Audiostart UND Verlauf einzeln: Der erste Block „Verlauf ausgeführt“, der zweite „Verlauf nicht aufgenommen“, nie
    /// beide gleich. Gegen 5080d95 rot: Der Verlauf stand nur als EINE Marke der ganzen Anfrage; je Einheit gab es kein Ergebnis.
    func testB4TwoBlocksOneInTheHistoryAreReadPerUnit() async throws {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", detail: "Autorin Zimtstern", sections: [
            ("S. 7", "Quasselbert wandert mit dem Fahrrad nach Pappelhausen."),
            ("S. 9", "Quasselbert trifft in Pappelhausen die Tante Lavendula."),
        ]))
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: "Quasselbert wandert mit dem Fahrrad nach Pappelhausen. Quasselbert trifft in Pappelhausen die Tante Lavendula."),
                       knowledge: AssistantKnowledge(corpus: corpus))
        let g = s.releaseGeneration
        var quotes: [String] = []
        var index = 0
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            guard case .sentence(_, _, let text, _, let status, _, _) = e else { continue }
            if status == .grounded { quotes.append(text) }
            s.noteCommit(requestId: "r1", index: index, stage: .display, ok: status != .announcement, generation: g)
            index += 1
        }
        guard quotes.count >= 2 else { return XCTFail("zwei Quellblöcke wurden erwartet, ausgegeben: \(quotes.count)") }
        // Gesprochen (und im Verlauf) ist nur der erste Block.
        await s.commit(requestId: "r1", user: "Wohin?", spoken: quotes[0], interrupted: true)
        let entries = await audit.entries()
        let entry = try XCTUnwrap(entries.first { $0.requestId == "r1" })
        let grounded = entry.trace.filter { $0.verdict == .grounded }
        guard grounded.count >= 2 else { return XCTFail("zwei Einheiten mit Quelle erwartet") }
        XCTAssertEqual(grounded[0].result(for: .history), .executed, "der erste Block steht im Verlauf")
        XCTAssertEqual(grounded[1].result(for: .history), .discarded, "der zweite Block steht nicht darin")
        XCTAssertEqual(grounded[0].result(for: .display), .executed)
        let history = await s.history
        XCTAssertEqual(history.count, 1)
        XCTAssertEqual(history.first?.assistant, quotes[0])
    }

    // MARK: B5: eine überholte Erweiterung überholt keinen späteren Widerruf

    /// B5: A erweitert um Notizen, B schränkt (auf Bücher) ein, C erweitert um Dateien; gesichert wird in der Reihenfolge A, B, C. Die
    /// Einschränkung B streicht die Notizen aus A: Nach A gelten weiter nur Bücher, die Datei trägt keine Notizen, A wird nie wirksam
    /// („nicht wirksam“); erst C wirkt, mit Büchern und Dateien. Gegen 5080d95 rot: A setzte nach B den Stand auf das gemeinsame Ziel
    /// zurück und machte die Notizen wieder wirksam.
    func testB5OvertakenExpansionCannotOvertakeALaterRevocation() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books], cloudAreas: [.books]))
        let store = openStore()
        let audit = log()
        let s = session(store, audit: audit)
        let g0 = s.releaseGeneration
        let a = s.applyAreas([.books, .notes])
        let b = s.applyAreas([.books])
        let c = s.applyAreas([.books, .books, .files].reduce(into: Set<AssistantArea>()) { $0.insert($1) })
        XCTAssertEqual(s.effectiveAreas, [.books], "nach dem Anwenden: nur Bücher (A und C wirken erst nach der Sicherung)")
        XCTAssertEqual(s.releaseGeneration, g0 + 1, "nur die Einschränkung B hat eine Generation vergeben")
        func kinds() -> [String] { lines(["intent", "ineffective", "release", "effective"]).compactMap { $0["t"] as? String } }

        let ra = await s.settle(a)
        XCTAssertFalse(ra.effective, "A: von B überholt, nicht wirksam")
        XCTAssertEqual(s.effectiveAreas, [.books])
        XCTAssertEqual(fileRecord()?.localAreas, ["books"], "die Datei trägt keine Notizen")
        XCTAssertEqual(kinds(), ["intent", "ineffective"])
        XCTAssertEqual(s.releaseGeneration, g0 + 1, "A vergibt keine Generation")

        let rb = await s.settle(b)
        XCTAssertTrue(rb.effective)
        XCTAssertEqual(fileRecord()?.localAreas, ["books"])
        XCTAssertEqual(kinds(), ["intent", "ineffective", "release"])

        let rc = await s.settle(c)
        XCTAssertTrue(rc.effective, "C: wirksam, über die eigene Folge")
        XCTAssertEqual(s.effectiveAreas, [.books, .files], "Bücher und Dateien, keine Notizen")
        XCTAssertEqual(fileRecord()?.localAreas, ["books", "files"])
        XCTAssertEqual(kinds(), ["intent", "ineffective", "release", "intent", "effective"])
        XCTAssertEqual(lines(["effective"]).first?["gen"] as? Int, g0 + 2)
        XCTAssertEqual(lines(["intent", "ineffective", "release", "effective"]).compactMap { $0["chg"] as? Int }, [1, 1, 2, 3, 3])
    }

    // MARK: B6: Ablehnungszeilen im Lock mit der vorgefundenen Generation

    /// B6: Prüfung bei 7, Erhöhung auf 8, Ablehnungszeile: Die Zeile wird IM Lock eingereiht. Ein Haltepunkt in der Ablehnung hält den
    /// Lock; die Erhöhung auf 9 kann erst danach geschehen, ihre Zeile steht hinter der Ablehnung „gefunden 8“. Gegen 5080d95 rot (nicht
    /// ausdrückbar: die Ablehnung wurde erst NACH dem Lock eingereiht; dort bewies nur der Wettlauf unten die Verletzung).
    func testB6TheRejectionLineIsEnqueuedInsideTheLockWithTheFoundGeneration() {
        let order = Recorder()
        let release = AssistantRelease(optIn: false, areas: [.books], cloudConsent: false, onIncrease: { increase in
            order.add("zeile \(increase.kind) \(increase.generation)")
        })
        for _ in 0..<7 { release.bump() }
        XCTAssertEqual(release.currentGeneration, 7)
        release.bump()
        let inside = DispatchSemaphore(value: 0)
        let proceed = DispatchSemaphore(value: 0)
        let raiserDone = DispatchSemaphore(value: 0)
        let rejectorDone = DispatchSemaphore(value: 0)
        let rejector = Thread {
            _ = release.ifCurrent(7, onStale: { found in
                order.add("abgelehnt gefunden \(found)")
                inside.signal()
                proceed.wait()   // Haltepunkt: der Lock bleibt gehalten
            }) { order.add("body") }
            rejectorDone.signal()
        }
        let raiser = Thread {
            _ = release.contentWillChange()   // Erhöhung auf 9, eigene Zeile im Lock
            raiserDone.signal()
        }
        rejector.start()
        inside.wait()
        raiser.start()
        // Der Erhöhende wartet auf den Lock des Ablehnenden (er kann nicht fertig sein).
        XCTAssertEqual(raiserDone.wait(timeout: .now() + 0.3), .timedOut, "die Erhöhung wartet auf den Lock der Ablehnung")
        proceed.signal()
        XCTAssertEqual(rejectorDone.wait(timeout: .now() + 5), .success)
        XCTAssertEqual(raiserDone.wait(timeout: .now() + 5), .success)
        XCTAssertEqual(order.all, ["abgelehnt gefunden 8", "zeile content 9"], "kein body, die Ablehnung steht vor der Zeile der späteren Änderung")
    }

    /// B6 als Wettlauf auf der Sitzung: Ablehnungen (`a-g`) und Erhöhungen der Klasse B nebeneinander. In der Datei steht nie eine
    /// Zeile einer späteren Generation vor einer Ablehnung mit kleinerer vorgefundener Generation.
    func testB6RejectionsNeverStandBehindTheLineOfALaterIncrease() async {
        let audit = log()
        let s = session(nil, audit: audit)
        s.admitForTesting(requestId: "r1")
        let stale = s.releaseGeneration
        _ = s.contentWillChange()
        await withTaskGroup(of: Void.self) { group in
            group.addTask {
                for _ in 0..<300 { _ = s.commitOutput(requestId: "r1", index: 0, generation: stale, stage: .display) {} }
            }
            group.addTask {
                for _ in 0..<300 { _ = s.contentWillChange() }
            }
        }
        _ = await audit.flushReleaseLines()
        _ = await audit.waitUntilWritten(request: "r1")
        var highestRelease = 0
        var violations: [String] = []
        let all = (try? String(contentsOf: protocolFile, encoding: .utf8)) ?? ""
        for raw in all.split(separator: "\n") {
            guard let line = (try? JSONSerialization.jsonObject(with: Data(raw.utf8))) as? [String: Any] else { continue }
            if line["t"] as? String == "release", let g = line["gen"] as? Int { highestRelease = max(highestRelease, g) }
            if line["t"] as? String == "commit", let mark = line["mark"] as? String, mark.hasPrefix("d-"), let g = Int(mark.dropFirst(2)), g < highestRelease {
                violations.append(mark)
            }
        }
        XCTAssertEqual(violations, [], "die Ablehnung steht vor der Zeile jeder späteren Erhöhung")
    }

    // MARK: B7: Fehlerschlüssel je Lauf und Änderung

    /// B7: Zwei Sitzungen (lokal und Cloud) im selben Lauf ändern gleichzeitig. Ein Schreibfehler der Zeile „beabsichtigt“ der Sitzung A
    /// kann von B nicht abgeholt werden: A wird nicht wirksam, B bleibt unberührt. Gegen 5080d95: Beide hießen „chg:1“ (je Sitzung
    /// gezählt, ohne Lauf), B konnte den Fehler von A abholen, und A galt als wirksam, obwohl ihre Zeile fehlte. Das ist ein Rennen
    /// (B muss seine Abholung zwischen Einreihen und Warten von A einreihen): gegen 5080d95 ROT NUR GELEGENTLICH (gemessen 4 von 300
    /// Läufen); der deterministische Beleg für denselben Fehler ist der Test der Standzeile darunter. Jetzt: die Nummer zählt die
    /// Ordnung des Prozesslaufs (nie doppelt), der Schlüssel trägt ihre Kennung.
    func testB7TwoSessionsChangeNumberOneAndTheErrorOfOneIsNeverConsumedByTheOther() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books], cloudAreas: [.books]))
        let store = openStore()
        let failIntentOfA = Flag()
        let audit = log(appender: { url, data in
            let text = String(decoding: data, as: UTF8.self)
            if failIntentOfA.value, text.contains("\"t\":\"intent\""), text.contains("\"cloud\":false") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let a = session(store, audit: audit, config: config)
        let b = session(store, audit: audit, config: cloudConfig)
        failIntentOfA.value = true
        let pa = a.applyAreas([.books, .notes])
        let pb = b.applyAreas([.books, .notes], cloudConsent: true)
        async let ra = a.settle(pa)
        async let rb = b.settle(pb)
        let (resultA, resultB) = await (ra, rb)
        XCTAssertFalse(resultA.effective, "A: Die Zeile „beabsichtigt“ steht nicht, also nicht wirksam")
        XCTAssertNotNil(resultA.error)
        XCTAssertEqual(a.effectiveAreas, [.books])
        XCTAssertTrue(resultB.effective, "B: unberührt")
        XCTAssertNil(resultB.error, "B holt den Fehler von A nicht ab")
        XCTAssertEqual(b.effectiveAreas, [.books, .notes])
    }

    /// B7 (Standzeile, deterministisch): Die Zeile `state` der Sitzung A scheitert beim Schreiben; die Sitzung B (Cloud) fragt zuerst an,
    /// dann A. Gegen 5080d95 ROT: Beide Standzeilen hießen „state“; B holte den Fehler von A ab (B abgewiesen, obwohl ihre Zeile
    /// stand), und A nahm danach eine Anfrage an, obwohl ihre eigene Standzeile NIE in der Datei stand. Jetzt: EINE Standzeile je
    /// Prozesslauf mit dem Schlüssel der Ordnung; ihr Fehler gehört dem einen Versuch, und die Anfrage von A schreibt sie neu.
    func testB7TheErrorOfTheStateLineOfOneSessionIsNeverConsumedByTheRequestOfAnother() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books], cloudAreas: [.books]))
        let store = openStore()
        let failFirstStateLine = Flag()
        failFirstStateLine.value = true
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\""), failFirstStateLine.value {
                failFirstStateLine.value = false
                throw Injected()
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let model = CountingModel()
        let a = session(store, audit: audit, model: model, config: config)
        let b = session(store, audit: audit, model: model, config: cloudConfig)
        let fromB = await drainEvents(b, AssistantRequest(requestId: "rb", text: "Hallo"))
        XCTAssertEqual(fromB.failure, .auditUnavailable, "die erste Standzeile steht nicht: keine Anfrage")
        XCTAssertEqual(model.calls.value, 0, "kein Modell vor der Standzeile")
        let fromA = await drainEvents(a, AssistantRequest(requestId: "ra", text: "Hallo"))
        XCTAssertNotEqual(fromA.failure, .auditUnavailable, "der nächste Versuch schreibt die Standzeile neu")
        XCTAssertTrue(lines(["state"]).contains { $0["cloud"] as? Bool == false },
                      "eine angenommene Anfrage der Sitzung A setzt ihre Standzeile in der Datei voraus")
    }

    // MARK: B8: Meldung der Quellen VOR der lokalen Wirksamkeit

    /// Eine Quelle, deren Text über die Meldestelle ersetzt wird (so wie der Speicher eines Buchs, einer Notiz oder der Ablage es an
    /// jeder Schreibstelle tut): Die Meldung kommt VOR der Änderung; im Haltepunkt ist die Generation schon gestiegen.
    private final class ReplaceableSource: @unchecked Sendable {
        private let lock = NSLock()
        private var stored: String
        init(_ text: String) { stored = text }
        var text: String { lock.withLock { stored } }
        func replace(_ new: String, watch: AssistantContentWatch, whileChanging: () -> Void) {
            watch.around {
                whileChanging()
                lock.withLock { stored = new }
            }
        }
    }

    /// Haltepunkt für eine Quelle im Abruf (ohne Thread zu blockieren).
    private actor Hold {
        private var arrived = false
        private var opened = false
        private var arrivedWaiters: [CheckedContinuation<Void, Never>] = []
        private var openWaiters: [CheckedContinuation<Void, Never>] = []
        func arrive() async {
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

    private struct HeldSource: AssistantSource {
        let area: AssistantArea
        let hold: Hold
        func items() async -> [AssistantItem] { [] }
        func search(_ question: String, limit: Int) async -> [AssistantPassage] {
            await hold.arrive()
            return []
        }
    }

    /// B8: Der Text einer Quelle wird ersetzt (Buch neu importiert, Text per Abgleich ausgetauscht, Notiz geändert, Datei der Ablage
    /// geändert). Eine Anfrage steht mit dem alten Bestand im Abruf; die Änderung meldet sich über die Meldestelle VOR dem Schreiben.
    /// Folge: Der alte Block startet nicht mehr, auch nicht nach dem Ende der Änderung, solange kein Durchgang NACH ihr den Bestand neu
    /// gebildet hat; danach kommt nur der neue Text. Gegen 5080d95 rot: Dort meldeten sich der Textabgleich eines Buchs, der Import
    /// eines gleichnamigen Buchs, das Löschen einer Notiz (Sicht erst nach `refresh`), der Speicher der Ablage und die Ablage-Sicht
    /// nicht oder erst nach der Änderung; der alte Block blieb bis zum nächsten Durchgang startbar.
    func testB8ASourceTextReplacedThroughTheWatchStopsTheWaitingOldBlockAndOnlyTheNewTextComesAfterThePass() async {
        let oldText = "Der Eisvogel brütet am Mühlbach in der Uferböschung unter dem Weidenbaum."
        let newText = "Der Eisvogel brütet jetzt im Steilufer am Weiher bei der Mühle."
        let question = "Wo brütet der Eisvogel am Mühlbach?"
        let watch = AssistantContentWatch()
        let source = ReplaceableSource(oldText)
        let corpus = AssistantCorpus()
        await corpus.upsert(note(source.text))
        let hold = Hold()
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [HeldSource(area: .files, hold: hold)])
        await knowledge.setAreas([.notes, .files])
        let s = session(nil, model: EchoModel(text: "Der Eisvogel brütet am Mühlbach."), knowledge: knowledge)
        watch.attach(s)
        XCTAssertTrue(watch.passCompleted(mark: watch.mark), "Ausgangslage: der Bestand steht")
        XCTAssertTrue(s.isContentReady)
        let g = s.releaseGeneration

        let waiting = Task { await sentences(s, "r1", question) }
        await hold.waitUntilArrived()   // die Anfrage hat Generation, Stand und alten Bestand gelesen und wartet

        var raisedBeforeTheWrite = false
        source.replace(newText, watch: watch) {
            raisedBeforeTheWrite = s.releaseGeneration == g + 1 && !s.isContentReady   // Haltepunkt: die Quelle ist noch unverändert
        }
        XCTAssertTrue(raisedBeforeTheWrite, "die Generation steigt VOR dem Schreiben der Quelle")
        XCTAssertEqual(source.text, newText)
        await hold.open()
        let old = await waiting.value
        XCTAssertFalse(old.contains { $0.contains("Uferböschung") }, "der alte Block startet nicht mehr: \(old)")

        // Die Änderung ist beendet, aber noch kein Durchgang hat den Bestand neu gebildet: weiter nichts aus eigenen Inhalten.
        XCTAssertFalse(s.isContentReady)
        let before = await sentences(s, "r2", question)
        XCTAssertFalse(before.contains { $0.contains("Uferböschung") || $0.contains("Steilufer") }, "stale: nichts aus Inhalten: \(before)")

        // Der Durchgang NACH der Änderung: Marke nehmen, Bestand aus der Quelle bilden, melden.
        let mark = watch.mark
        await corpus.install([note(source.text, version: "2")])
        XCTAssertTrue(watch.passCompleted(mark: mark))
        XCTAssertTrue(s.isContentReady)
        let after = await sentences(s, "r3", question)
        XCTAssertTrue(after.contains { $0 == newText }, "der neue Text gilt: \(after)")
        XCTAssertFalse(after.contains { $0.contains("Uferböschung") }, "der alte Text ist ersetzt, nicht ergänzt")
    }

    // MARK: B9: eine Ordnung für den ganzen Prozesslauf

    /// B9: Bereich aus und wieder an OHNE Modell: zwei Erhöhungen, zwei Zeilen mit der Generation (die Einschränkung `release`, die
    /// Erweiterung „wirksam“); dieselbe Laufnummer. Gegen 5080d95 rot: Ohne Sitzung gab es keine Generation und keine Erhöhung.
    func testB9AreaOffAndOnWithoutAModelRaiseTwiceAndTheLinesCarryTheGeneration() async {
        let store = openStore()
        let audit = log()
        let before = store.clock.generation
        _ = await store.updateAreas([], cloud: false, audit: audit)
        XCTAssertEqual(store.clock.generation, before + 1, "die Einschränkung erhöht die Generation im selben Schritt")
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        XCTAssertEqual(store.clock.generation, before + 2, "die Erweiterung erhöht im Schritt des Wirksamwerdens")
        let changes = lines(["release", "intent", "effective"])
        XCTAssertEqual(changes.compactMap { $0["t"] as? String }, ["release", "intent", "effective"])
        XCTAssertEqual(changes.map { $0["gen"] as? Int }, [before + 1, nil, before + 2])
        XCTAssertEqual(Set(changes.compactMap { $0["run"] as? Int }), [1])
        XCTAssertEqual(changes.compactMap { $0["chg"] as? Int }, [1, 2, 2])
    }

    /// B9: Zwei Sitzungen teilen Generation und Lauf; die Erhöhung in A widerruft die laufende Anfrage in B. Eine neue Sitzung beginnt
    /// bei der geltenden Generation. Gegen 5080d95 rot: Jede Sitzung hatte ihre eigene Generation, Lauf und Standzeile.
    func testB9TwoSessionsShareGenerationAndRunAndARaiseInOneRevokesTheRequestInTheOther() async {
        let store = openStore()
        let audit = log()
        let a = session(store, audit: audit, config: config)
        let b = session(store, audit: audit, model: SleepyModel(), config: config)
        XCTAssertEqual(a.releaseGeneration, b.releaseGeneration)
        let request = Task { await drainEvents(b, AssistantRequest(requestId: "rb", text: "Hallo")) }
        let deadline = Date().addingTimeInterval(10)
        while b.registeredRunCount == 0, Date() < deadline { try? await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(b.registeredRunCount, 1, "die Anfrage steht im Modellaufruf")
        let g = a.releaseGeneration
        await a.releaseDidChange()
        XCTAssertEqual(a.releaseGeneration, g + 1)
        XCTAssertEqual(b.releaseGeneration, g + 1, "eine Generation je Prozesslauf")
        let result = await request.value
        XCTAssertEqual(result.failure, .revoked, "die Erhöhung in A widerruft die Anfrage in B")
        let late = session(store, audit: audit)
        XCTAssertEqual(late.releaseGeneration, g + 1, "eine neue Sitzung beginnt bei der geltenden Generation")
        let runs = Set(lines(["state", "start", "release"]).compactMap { $0["run"] as? Int })
        XCTAssertEqual(runs, [1], "eine Laufnummer, eine Standzeile")
        XCTAssertEqual(lines(["state"]).count, 1, "eine Standzeile je Prozesslauf")
    }

    // MARK: B10: Nachholen bei jeder Gelegenheit

    /// B10: Der Speicher ist wieder schreibbar: Die nächste Anfrage sichert die Einschränkung (nach ihrer gesicherten `start`-Zeile) und
    /// schreibt die fehlende Zeile „nicht wirksam“. Gegen 5080d95 rot: Nachgeholt wurde nur bei der nächsten Änderung, eine Anfrage
    /// holte nichts nach, und eine blockierte Sitzung lehnte Erweiterungen ab, ohne die Sicherung neu zu versuchen.
    func testB10StoreWritableAgainTheNextRequestSecuresTheRestrictionAndWritesTheLine() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books, .notes], cloudAreas: [.books]))
        let storeFS = CxFS("store")
        let store = openStore(storeFS)
        let failIneffective = Flag()
        let audit = log(appender: { url, data in
            if failIneffective.value, String(decoding: data, as: UTF8.self).contains("\"t\":\"ineffective\"") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(store, audit: audit)
        storeFS.fail(.rename)
        failIneffective.value = true
        let narrowed = await s.setAreas([.books])
        XCTAssertNotNil(narrowed.storeError, "der Austausch der Einschränkung scheiterte")
        XCTAssertTrue(s.isContentBlocked)
        let refused = await s.setAreas([.books, .files])
        XCTAssertFalse(refused.effective, "blockiert: nicht wirksam")
        XCTAssertTrue(lines(["ineffective"]).isEmpty, "die Zeile „nicht wirksam“ ließ sich nicht schreiben")
        XCTAssertTrue(store.hasPendingRestriction)
        XCTAssertEqual(fileRecord()?.localAreas, ["books", "notes"], "die Datei trägt noch den alten Stand")
        // Heilung: Speicher und Protokoll sind wieder schreibbar. Die nächste ANFRAGE holt beides nach.
        storeFS.heal()
        failIneffective.value = false
        _ = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertFalse(store.hasPendingRestriction, "die Einschränkung ist gesichert")
        XCTAssertEqual(fileRecord()?.localAreas, ["books"])
        XCTAssertEqual(lines(["ineffective"]).count, 1, "und die Zeile „nicht wirksam“ steht")
        XCTAssertTrue(s.isContentBlocked, "die Blockade für den Rest des Laufs bleibt")
    }
}
