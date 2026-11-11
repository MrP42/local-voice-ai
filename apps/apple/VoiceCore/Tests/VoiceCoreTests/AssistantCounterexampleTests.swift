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

/// Wartet auf ein Semaphor, ohne den aufrufenden asynchronen Test zu blockieren (`wait` ist aus asynchronem Code nicht erlaubt).
private func awaitSignal(_ semaphore: DispatchSemaphore) async {
    await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
        DispatchQueue.global().async {
            semaphore.wait()
            c.resume()
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

/// Schlägt die ersten `times` Schreibaufrufe fehl, die `matches` bejaht (zählt atomar).
private final class FailFirst: @unchecked Sendable {
    private let lock = NSLock()
    private var left: Int
    init(_ times: Int) { left = times }
    func shouldFail() -> Bool {
        lock.withLock {
            guard left > 0 else { return false }
            left -= 1
            return true
        }
    }
}

/// Ein Haltepunkt für den Erzeuger einer Anfrage (Runde 10, `ProducerHold`): Der `holdAt`-te Besuch suspendiert (er blockiert keinen Thread; der
/// Abbruch der Aufgabe wirkt nicht, der Lauf hängt wirklich) bis `release()`; der Test wartet mit `awaitReached()`, bis der Erzeuger da ist.
/// Haltepunkte statt Zeitannahmen.
private final class ProducerHold: @unchecked Sendable {
    private let lock = NSLock()
    private var visits = 0
    private var reached = false
    private var released = false
    private var producer: CheckedContinuation<Void, Never>?
    private var testers: [CheckedContinuation<Void, Never>] = []

    func visit(holdAt n: Int) async {
        guard lock.withLock({ () -> Bool in
            visits += 1
            return visits == n
        }) else { return }
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            let (waiting, resumeNow) = lock.withLock { () -> ([CheckedContinuation<Void, Never>], Bool) in
                reached = true
                let waiting = testers
                testers = []
                if released { return (waiting, true) }
                producer = c
                return (waiting, false)
            }
            for t in waiting { t.resume() }
            if resumeNow { c.resume() }
        }
    }

    func awaitReached() async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            let now = lock.withLock { () -> Bool in
                if reached { return true }
                testers.append(c)
                return false
            }
            if now { c.resume() }
        }
    }

    func release() {
        let parked = lock.withLock { () -> CheckedContinuation<Void, Never>? in
            released = true
            let p = producer
            producer = nil
            return p
        }
        parked?.resume()
    }
}

/// Ein Haltepunkt der Schreibwarteschlange (Runde 11): Die ERSTE Zeile, die `isFirst()` bejaht, hält den Schreiber an, bis der Test
/// `release` meldet (es blockiert nur den Thread der Warteschlange im synchronen Anhänger, nie den asynchronen Test). `entered` meldet,
/// dass der Schreiber angekommen ist; der Test wartet mit `awaitSignal(entered)` ohne Thread.
private final class WriterHold: @unchecked Sendable {
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var seen = 0
    func isFirst() -> Bool { lock.withLock { seen += 1; return seen == 1 } }
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
                         knowledge: AssistantKnowledge? = nil, config: AssistantConfig? = nil,
                         stepIndex: AssistantStepIndex = AssistantStepIndex(),
                         commitWait: Duration = AssistantSession.defaultCommitWait) -> AssistantSession {
        AssistantSession(model: model, knowledge: knowledge ?? AssistantKnowledge(corpus: AssistantCorpus()), config: config ?? self.config,
                         audit: audit, released: true, releaseStore: store, stepIndex: stepIndex, commitWait: commitWait)
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

    /// B1: Die Notiz ist „Alpha\rBeta“ (ein Zeilenende `\r`). Ausgegeben wird GENAU diese Bytefolge. Gegen 5080d95 ROT (gemessen): Der
    /// Kern setzte die Zeilen mit `\n` zusammen und gab „Alpha\nBeta“ aus, also nicht den gespeicherten Text.
    func testB1ANoteStoredAsAlphaCarriageReturnBetaIsOutputWithTheCarriageReturn() async {
        let out = await quotes(of: [note("Alpha\rBeta")], question: "Alpha Beta?", answer: "Alpha Beta")
        XCTAssertEqual(out.count, 1, "das Zitat kommt: \(out)")
        XCTAssertEqual(out.first.map { Array($0.utf8) }, Array("Alpha\rBeta".utf8), "Bytes wie gespeichert, kein `\\n` statt `\\r`")
    }

    /// B1: Zeilenenden (`\r\n`), doppelte Leerzeichen, Tabs, NFD-Umlaute und Rand-Leerraum bleiben, wie gespeichert; ein Trenner
    /// (Leerzeile) gehört nie zum Block, ein Zeilenende darin schon. Gegen 5080d95 ROT (gemessen).
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
    /// Suchzeile „Anna: …“; ohne Rohtext kommt das Segment nie. Gegen 5080d95 NICHT AUSDRÜCKBAR (den Rohtext `raw` der Abschnitte gibt
    /// es dort nicht; Beleg: der `\r`-Test oben ist rot).
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
    /// enthält er Stil- oder Sprechermarken, gibt es keinen eindeutigen Rohtext: der Absatz kommt nie. Gegen 5080d95 NICHT AUSDRÜCKBAR
    /// (den Rohtext `raw` der Abschnitte gibt es dort nicht; Beleg: der `\r`-Test oben ist rot).
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

    /// B1: Erzeugter Text, Mail und Kalender kommen nie, auch wenn der Aufrufer einen Rohtext mitgibt. Gegen 5080d95 NICHT AUSDRÜCKBAR
    /// (den Rohtext `raw` der Abschnitte gibt es dort nicht; Beleg: der `\r`-Test oben ist rot).
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

    /// B2: Die Zeile „beabsichtigt“ steht dauerhaft (fsync der Datei), BEVOR der Austausch der Zustandsdatei beginnt. Der Schreiber von
    /// 5080d95 sicherte nie (kein fsync): Die Zeile konnte nach einem Stromausfall fehlen, obwohl die Datei schon ersetzt war. Gegen
    /// 5080d95 NICHT AUSDRÜCKBAR (das Dateisystem des Protokolls ist dort nicht einhängbar); statischer Beleg: in
    /// AssistantAuditLog.swift von 5080d95 steht kein `synchronize`, `fsync` oder `syncFile` (Suche leer).
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
    /// Laufnummer wird nicht doppelt vergeben: Der zweite Versuch trägt dieselbe, ein neuer Prozesslauf die nächste. Gegen 5080d95:
    /// ohne fsync gab es keinen Fehler, die Anfrage lief. Gegen 5080d95 NICHT AUSDRÜCKBAR (das Dateisystem des Protokolls ist dort
    /// nicht einhängbar); statischer Beleg: in AssistantAuditLog.swift von 5080d95 steht kein `synchronize`, `fsync` oder `syncFile`
    /// (Suche leer).
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
    /// Verzeichnisses). Gegen 5080d95: Das Verdichten ersetzte die Datei ohne eigene Sicherung der Teilschritte. Gegen 5080d95 NICHT
    /// AUSDRÜCKBAR (das Dateisystem des Protokolls ist dort nicht einhängbar); statischer Beleg: in AssistantAuditLog.swift von 5080d95
    /// steht kein `synchronize`, `fsync` oder `syncFile` (Suche leer).
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
            wait(for: [finished], timeout: 60)
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
        wait(for: [done], timeout: 60)
        let compacted = try String(contentsOf: protocolFile, encoding: .utf8).split(separator: "\n")
        XCTAssertLessThanOrEqual(compacted.count, AssistantAuditWriter.maxUnits + 1)
    }

    // MARK: B3: ein Schreibfehler stoppt die Ausgabe sofort

    /// B3: Der Audiostart der Einheit 1 gelingt, seine Ergebniszeile `a+` lässt sich nicht schreiben (während der Wiedergabe). Danach
    /// startet die Einheit 2 NICHT: kein Body, Ergebnis „Protokollfehler“ (nicht „widerrufen“). Gegen 5080d95 ROT (gemessen): Die
    /// Einheit 2 startete, weil der Kern den klebrigen Fehler nur meldete, nachdem sie ausgelöst war.
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

    /// B3: Ein Schreibfehler geht dem Widerruf voran: Eine Anfrage mit Schreibfehler UND Widerruf endet mit „Fehler“. Gegen 5080d95 ROT
    /// (gemessen): Ausgang „widerrufen“.
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
        await s.commit(requestId: "r1", user: "x", spoken: .nothing, interrupted: false, revoked: true)
        let outcome = await audit.entries().first { $0.requestId == "r1" }?.outcome
        XCTAssertEqual(outcome, .failed, "Fehler, nicht „widerrufen“")
    }

    /// B3: Der Controller erfährt den Schreibfehler SOFORT (Beobachter am Protokoll), nicht erst beim Start der nächsten Einheit: Er
    /// stoppt damit die laufende Wiedergabe und verwirft die Wartenden. Der Schlüssel ist die Anfrage; ein Fehler einer fremden Zeile
    /// (ohne Anfrage) trägt einen anderen Schlüssel. Gegen 5080d95 NICHT AUSDRÜCKBAR (den Beobachter am Protokoll `setFailureObserver`
    /// gibt es dort nicht; der Fehler wurde erst beim Start der nächsten Einheit gemeldet, siehe die Tests oben).
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
    /// Anzeige, Audiostart UND Verlauf einzeln: Der erste Block „Verlauf ausgeführt“, der zweite „Verlauf nicht aufgenommen“, nie beide
    /// gleich. Gegen 5080d95 ROT (gemessen): Der Verlauf stand nur als EINE Marke der ganzen Anfrage; je Einheit gab es kein Ergebnis
    /// („nicht geplant“).
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
        var quoteEvents: [Int] = []
        var index = 0
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert nach Pappelhausen?")) {
            guard case .sentence(_, _, let text, _, let status, _, _) = e else { continue }
            if status == .grounded { quotes.append(text); quoteEvents.append(index) }
            s.noteCommit(requestId: "r1", index: index, stage: .display, ok: status != .announcement, generation: g)
            index += 1
        }
        guard quotes.count >= 2 else { return XCTFail("zwei Quellblöcke wurden erwartet, ausgegeben: \(quotes.count)") }
        // Ganz gesprochen (und im Verlauf) ist nur der erste Block: Die Steuerung meldet seine Ereignisnummer.
        await s.commit(requestId: "r1", user: "Wohin?", spoken: .events([quoteEvents[0]]), interrupted: true)
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
    /// („nicht wirksam“); erst C wirkt, mit Büchern und Dateien. Gegen 5080d95 ROT (gemessen): A setzte nach B den Stand auf das
    /// gemeinsame Ziel zurück und machte die Notizen wieder wirksam.
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
    /// Lock; die Erhöhung auf 9 kann erst danach geschehen, ihre Zeile steht hinter der Ablehnung „gefunden 8“. Gegen 5080d95 NICHT
    /// AUSDRÜCKBAR (`ifCurrent(_:onStale:)` gibt es dort nicht: die Ablehnung wurde erst NACH dem Lock eingereiht); dort belegt der
    /// Wettlauf unten die Verletzung.
    func testB6TheRejectionLineIsEnqueuedInsideTheLockWithTheFoundGeneration() {
        let order = Recorder()
        let release = AssistantRelease(optIn: false, areas: [.books], cloudConsent: false, onIncrease: { increase in
            order.add("zeile \(increase.kind) \(increase.generation)")
        })
        for _ in 0..<7 { release.bump() }
        XCTAssertEqual(release.currentGeneration, 7)
        release.bump()
        // Die spätere Erhöhung: eine Erweiterung, die wirksam wird (ihre Zeile entsteht im Lock der Erhöhung; die Zeilen der Klasse B
        // entstehen seit Runde 8 in der Ordnung selbst und sind in `testR4…` abgedeckt).
        let expansion = release.apply(AssistantReleaseChange(cause: .areas, areas: [.books, .notes])).steps
        XCTAssertEqual(expansion.map(\.kind), [.expansion])
        let inside = DispatchSemaphore(value: 0)
        let raiserDone = DispatchSemaphore(value: 0)
        let rejectorDone = DispatchSemaphore(value: 0)
        // Ohne Uhr: Der Haltepunkt in der Ablehnung hält den Lock, bis die Probe am Eingang der Ordnung das Eintreffen des Erhöhenden
        // meldet (und dort den Lock gehalten findet).
        let probe = EntryProbe()
        probe.install(on: release.clock)
        let rejector = Thread {
            _ = release.ifCurrent(7, onStale: { found in
                order.add("abgelehnt gefunden \(found)")
                inside.signal()
                if !probe.waitUntilArrived() { order.add("Erhöhender kam nie am Eingang an") }   // Haltepunkt: der Lock bleibt gehalten
            }) { order.add("body") }
            rejectorDone.signal()
        }
        let raiser = Thread {
            _ = release.becomeEffective(expansion[0])   // Erhöhung auf 9, eigene Zeile im Lock
            raiserDone.signal()
        }
        rejector.start()
        inside.wait()
        raiser.start()
        XCTAssertEqual(rejectorDone.wait(timeout: .now() + 60), .success)
        XCTAssertEqual(raiserDone.wait(timeout: .now() + 60), .success)
        probe.remove(from: release.clock)
        XCTAssertEqual(probe.observations, [true], "die Erhöhung fand beim Eintreffen den Lock der Ablehnung gehalten")
        XCTAssertEqual(order.all, ["abgelehnt gefunden 8", "zeile effective 9"], "kein body, die Ablehnung steht vor der Zeile der späteren Änderung")
    }

    /// B6 als Wettlauf auf der Sitzung: Ablehnungen (`a-g`) und Erhöhungen der Klasse B nebeneinander. In der Datei steht nie eine
    /// Zeile einer späteren Generation vor einer Ablehnung mit kleinerer vorgefundener Generation. Gegen 5080d95 ROT (gemessen).
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

    /// B7: Zwei Sitzungen (lokal und Cloud) im selben Lauf ändern gleichzeitig. Ein Schreibfehler der Zeile „beabsichtigt“ der Sitzung
    /// A kann von B nicht abgeholt werden: A wird nicht wirksam, B bleibt unberührt. Gegen 5080d95: Beide hießen „chg:1“ (je Sitzung
    /// gezählt, ohne Lauf), B konnte den Fehler von A abholen, und A galt als wirksam, obwohl ihre Zeile fehlte. Das ist ein Rennen (B
    /// muss seine Abholung zwischen Einreihen und Warten von A einreihen): gegen 5080d95 ROT NUR GELEGENTLICH (gemessen 4 von 300
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

    /// B7 (Standzeile, deterministisch): Die Zeile `state` der Sitzung A scheitert beim Schreiben; die Sitzung B (Cloud) fragt zuerst
    /// an, dann A. Gegen 5080d95 ROT: Beide Standzeilen hießen „state“; B holte den Fehler von A ab (B abgewiesen, obwohl ihre Zeile
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
    /// gebildet hat; danach kommt nur der neue Text. Gegen 5080d95 GRÜN im Kern (gemessen): Der Kern der Meldestelle war nicht der
    /// Fehler; die Meldung fehlte an den Schreibstellen der Quellen, und die sind nur auf dem Mac prüfbar (Tests der Speicher in
    /// `AssistantNoteHookTests`, hinter `canImport(CryptoKit)`). Dort meldeten sich der Textabgleich eines Buchs, der Import eines
    /// gleichnamigen Buchs, das Löschen einer Notiz (Sicht erst nach `refresh`), der Speicher der Ablage und die Ablage-Sicht nicht
    /// oder erst nach der Änderung; der alte Block blieb bis zum nächsten Durchgang startbar.
    func testB8ASourceTextReplacedThroughTheWatchStopsTheWaitingOldBlockAndOnlyTheNewTextComesAfterThePass() async {
        let oldText = "Der Eisvogel brütet am Mühlbach in der Uferböschung unter dem Weidenbaum."
        let newText = "Der Eisvogel brütet jetzt im Steilufer am Weiher bei der Mühle."
        let question = "Wo brütet der Eisvogel am Mühlbach?"
        let watch = AssistantContentWatch()
        let source = ReplaceableSource(oldText)
        let corpus = AssistantCorpus()
        await corpus.upsert(note(source.text))
        let hold = Hold()
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [HeldSource(area: .files, hold: hold)], areas: [.notes, .files])
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
    /// Erweiterung „wirksam“); dieselbe Laufnummer. Gegen 5080d95 NICHT AUSDRÜCKBAR (`store.clock` und `updateAreas` gibt es dort
    /// nicht): Ohne Sitzung gab es keine Generation und keine Erhöhung; der Test der zwei Sitzungen unten ist rot.
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
    /// bei der geltenden Generation. Gegen 5080d95 ROT (gemessen, Zeitüberschreitung): Jede Sitzung hatte ihre eigene Generation, Lauf
    /// und Standzeile.
    func testB9TwoSessionsShareGenerationAndRunAndARaiseInOneRevokesTheRequestInTheOther() async {
        let store = openStore()
        let audit = log()
        let a = session(store, audit: audit, config: config)
        let b = session(store, audit: audit, model: SleepyModel(), config: config)
        XCTAssertEqual(a.releaseGeneration, b.releaseGeneration)
        let request = Task { await drainEvents(b, AssistantRequest(requestId: "rb", text: "Hallo")) }
        let deadline = Date().addingTimeInterval(60)
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

    /// B10: Der Speicher ist wieder schreibbar: Die nächste Anfrage sichert die Einschränkung (nach ihrer gesicherten `start`-Zeile)
    /// und schreibt die fehlende Zeile „nicht wirksam“. Gegen 5080d95 ROT (gemessen): Nachgeholt wurde nur bei der nächsten Änderung,
    /// eine Anfrage holte nichts nach, und eine blockierte Sitzung lehnte Erweiterungen ab, ohne die Sicherung neu zu versuchen.
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

    // MARK: Runde 8, R3: EIN Stand hinter dem einen Lock

    private static let eisvogelBook = "Der Eisvogel brütet am Mühlbach in der Uferböschung unter dem Weidenbaum."
    private static let eisvogelQuestion = "Wo brütet der Eisvogel am Mühlbach?"

    /// Ein Bestand mit einem Buch (Bereich Bücher) und einer Notiz (Bereich Notizen) zum selben Thema.
    private func eisvogelKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", sections: [("S. 7", Self.eisvogelBook)]))
        await corpus.upsert(assistantDoc("n1", "Notiz Mühlbach", area: .notes, sections: [(nil, "Der Eisvogel brütet am Mühlbach, notiert am Ufer.")]))
        return AssistantKnowledge(corpus: corpus)
    }

    /// R3: Eine neue Anfrage in Sitzung B liest nach dem Widerruf in Sitzung A die engeren Bereiche (ein Stand, kein Abbild je Sitzung).
    /// Gegen 4d802f3 ROT: Jede Sitzung trug ihre eigenen Bereiche; B kannte den Widerruf in A nicht.
    func testR3ARequestInSessionBAfterARevocationInSessionAReadsTheNarrowerAreas() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books, .notes], cloudAreas: [.books]))
        let store = openStore()
        let knowledge = await eisvogelKnowledge()
        let answer = "Der Eisvogel brütet am Mühlbach."
        let a = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        let b = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        let before = await sentences(b, "r1", Self.eisvogelQuestion)
        XCTAssertTrue(before.contains { $0.contains("notiert am Ufer") }, "Ausgangslage: B zitiert aus der Notiz: \(before)")
        let revoked = await a.setAreas([.books])
        XCTAssertNil(revoked.storeError)
        XCTAssertEqual(b.effectiveAreas, [.books], "B liest den Stand der Ordnung, nicht eine Kopie")
        let after = await sentences(b, "r2", Self.eisvogelQuestion)
        XCTAssertFalse(after.contains { $0.contains("notiert am Ufer") }, "nach dem Widerruf in A kommt aus B nichts mehr aus den Notizen: \(after)")
        XCTAssertTrue(after.contains { $0.contains("Uferböschung") }, "der Bereich Bücher gilt weiter: \(after)")
    }

    /// R3: Die Blockade (eine Einschränkung ließ sich nicht sichern) aus Sitzung A gilt in Sitzung B, die schon vorher lief, und in
    /// einer Sitzung danach: nichts aus eigenen Inhalten. Gegen 4d802f3 ROT: Die Blockade lag in der Sitzung, die sie auslöste.
    func testR3ABlockFromSessionAActsInSessionB() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books, .notes], cloudAreas: [.books]))
        let fs = CxFS("store")
        let store = openStore(fs)
        let knowledge = await eisvogelKnowledge()
        let answer = "Der Eisvogel brütet am Mühlbach."
        let a = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        let b = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        XCTAssertFalse(b.isContentBlocked)
        fs.fail(.rename)
        let failed = await a.setAreas([.books])
        XCTAssertNotNil(failed.storeError, "der Austausch der Einschränkung scheiterte")
        XCTAssertTrue(a.isContentBlocked)
        XCTAssertTrue(b.isContentBlocked, "die Blockade ist ein Stand der Ordnung: sie gilt in B")
        let out = await sentences(b, "r1", Self.eisvogelQuestion)
        XCTAssertFalse(out.contains { $0.contains("Uferböschung") || $0.contains("notiert am Ufer") }, "blockiert: nichts aus Inhalten: \(out)")
        let late = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        XCTAssertTrue(late.isContentBlocked, "auch eine neue Sitzung")
    }

    /// R3: Ein veralteter Bestand (Klasse B: eine Quelle änderte sich, noch kein Durchgang meldete den neuen Bestand) gilt in jeder
    /// Sitzung, nicht nur in der, die die Meldestelle kennt. Gegen 4d802f3 ROT: Die Epochen lagen in der einen Sitzung, an der die
    /// Meldestelle hing.
    func testR3AStaleStockSeenThroughSessionAActsInSessionB() async throws {
        let store = openStore()
        let watch = AssistantContentWatch()
        let knowledge = await eisvogelKnowledge()
        let answer = "Der Eisvogel brütet am Mühlbach."
        let a = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        let b = session(store, model: EchoModel(text: answer), knowledge: knowledge)
        watch.attach(a)
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(b.isContentReady)
        let before = await sentences(b, "r1", Self.eisvogelQuestion)
        XCTAssertTrue(before.contains { $0.contains("notiert am Ufer") }, "Ausgangslage: der Bestand steht, B zitiert: \(before)")
        watch.around {}
        XCTAssertFalse(a.isContentReady)
        XCTAssertFalse(b.isContentReady, "der Bestand ist stale, in BEIDEN Sitzungen")
        let stale = await sentences(b, "r2", Self.eisvogelQuestion)
        XCTAssertFalse(stale.contains { $0.contains("notiert am Ufer") }, "stale: aus B nichts aus eigenen Inhalten: \(stale)")
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(b.isContentReady)
        let after = await sentences(b, "r3", Self.eisvogelQuestion)
        XCTAssertTrue(after.contains { $0.contains("notiert am Ufer") }, "nach dem Durchgang gilt der Bestand wieder: \(after)")
    }

    // MARK: Runde 8, R4: Klasse B ohne Sitzung

    /// R4: Zwei Änderungen der Quellen bei geschlossenem Gespräch: zwei Generationen, zwei Zeilen „Inhalt“, je Änderung eine. Gegen
    /// 4d802f3 ROT: Die Meldestelle hing an der Sitzung (`attach(nil)` löste sie); ohne Gespräch geschah nichts, mit Gespräch erhöhte die
    /// Meldestelle nur im Durchgang.
    func testR4TwoSourceChangesWithTheConversationClosedRaiseTwiceAndWriteTwoLines() async {
        let store = openStore()
        let audit = log()
        let watch = AssistantContentWatch()
        let s = session(store, audit: audit)
        watch.attach(s)
        watch.attach(nil)   // das Gespräch ist zu Ende
        let before = store.clock.generation
        watch.around {}
        watch.around {}
        XCTAssertEqual(store.clock.generation, before + 2, "jede gemeldete Änderung erhöht für sich")
        _ = await audit.flushReleaseLines()
        let content = lines(["release"]).filter { $0["cause"] as? String == "content" }
        XCTAssertEqual(content.compactMap { $0["gen"] as? Int }, [before + 1, before + 2], "je Änderung eine Zeile mit ihrer Generation")
        XCTAssertEqual(Set(content.compactMap { $0["run"] as? Int }), [1], "derselbe Prozesslauf")
        XCTAssertFalse(watch.isStale && store.clock.isContentReady, "die Meldestelle und die Ordnung sind sich einig")
    }

    /// R4: Ohne jede Sitzung seit dem Prozessstart (der Controller hängt Meldestelle und Protokoll beim Start ein, ohne Modell): dasselbe.
    /// Gegen 4d802f3 NICHT AUSDRÜCKBAR (`bind(watch:audit:)` gibt es dort nicht); der Test oben ist rot.
    func testR4TwoSourceChangesWithoutAnySessionSinceTheProcessStartRaiseTwiceAndWriteTwoLines() async {
        let store = openStore()
        let audit = log()
        let watch = AssistantContentWatch()
        store.bind(watch: watch, audit: audit)
        let before = store.clock.generation
        XCTAssertFalse(store.clock.snapshot(view: .local).corpusReady && watch.isStale, "Ausgangslage: nichts gemeldet")
        watch.around {}
        watch.around {}
        XCTAssertEqual(store.clock.generation, before + 2)
        XCTAssertFalse(store.clock.isContentReady, "der Bestand ist stale")
        _ = await audit.flushReleaseLines()
        let content = lines(["release"]).filter { $0["cause"] as? String == "content" }
        XCTAssertEqual(content.compactMap { $0["gen"] as? Int }, [before + 1, before + 2])
        XCTAssertTrue(watch.passCompleted(mark: watch.mark))
        XCTAssertTrue(store.clock.isContentReady)
        XCTAssertEqual(store.clock.generation, before + 2, "der Durchgang erhöht nicht noch einmal")
    }

    // MARK: Runde 8, R8: Zeilen zum Nachholen überleben den Wechsel der Sitzung

    /// R8: Die Zeile „in diesem Lauf nicht wirksam“ ließ sich nicht schreiben; die Sitzung wird ersetzt. Die nächste Anfrage der NEUEN
    /// Sitzung schreibt sie. Gegen 4d802f3 ROT: Das Register der Zeilen lag in der Sitzung und ging mit ihr verloren.
    func testR8ALineThatCouldNotBeWrittenSurvivesTheReplacementOfTheSession() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books, .notes], cloudAreas: [.books]))
        let storeFS = CxFS("store")
        let store = openStore(storeFS)
        let failIneffective = Flag()
        let audit = log(appender: { url, data in
            if failIneffective.value, String(decoding: data, as: UTF8.self).contains("\"t\":\"ineffective\"") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        var old: AssistantSession? = session(store, audit: audit)
        storeFS.fail(.rename)
        failIneffective.value = true
        _ = await old?.setAreas([.books])
        let refused = await old?.setAreas([.books, .files])
        XCTAssertEqual(refused?.effective, false)
        XCTAssertTrue(lines(["ineffective"]).isEmpty, "die Zeile ließ sich nicht schreiben")
        old = nil   // das Gespräch endet, die Sitzung ist weg
        storeFS.heal()
        failIneffective.value = false
        let next = session(store, audit: audit)
        _ = await drainEvents(next, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(lines(["ineffective"]).count, 1, "die Anfrage der neuen Sitzung holt die Zeile der alten nach")
    }

    /// R8: Dasselbe ohne Sitzung: Die Änderung der Bereiche ohne Gespräch holt die Zeile nach, die eine Sitzung hinterließ.
    func testR8ALineLeftByASessionIsCaughtUpByAChangeWithoutASession() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books, .notes], cloudAreas: [.books]))
        let storeFS = CxFS("store")
        let store = openStore(storeFS)
        let failIneffective = Flag()
        let audit = log(appender: { url, data in
            if failIneffective.value, String(decoding: data, as: UTF8.self).contains("\"t\":\"ineffective\"") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        var old: AssistantSession? = session(store, audit: audit)
        storeFS.fail(.rename)
        failIneffective.value = true
        _ = await old?.setAreas([.books])
        _ = await old?.setAreas([.books, .files])
        XCTAssertTrue(lines(["ineffective"]).isEmpty)
        old = nil
        storeFS.heal()
        failIneffective.value = false
        let change = await store.updateAreas([.books], cloud: false, audit: audit)
        XCTAssertNil(change.auditError)
        XCTAssertEqual(lines(["ineffective"]).count, 1, "auch eine Änderung ohne Sitzung holt nach")
        XCTAssertFalse(store.hasUnwrittenLines)
    }

    // MARK: Latenz: dauerhafte Wartepunkte vor dem ersten Ton

    /// Messung (kein Befund): Zahl der fsync der Protokolldatei (`syncCount`; ein fsync deckt als Gruppen-Commit alles, was bis zu
    /// seinem Lauf eingereiht ist) bis zum Eintreffen jeder Einheit beim Leser. Gemessen (Linux, tmp-Datei): erste Anfrage des
    /// Prozesslaufs 5 fsync bis zur ersten Einheit (Standzeile, `start`, `round`, `areas`, Einordnung der Einheit), jede spätere
    /// Anfrage 4; JEDE weitere Einheit +2 (die Ergebniszeile der vorigen Einheit wird als eigene Gruppe gesichert, danach die
    /// Einordnung der nächsten; der Ausgeber wartet nur auf die zweite, sie steht aber hinter der ersten in der Reihe). Die Werte beim
    /// Leser sind eine Obergrenze (der Ausgeber läuft schon weiter), darum prüft der Test nur lockere Schranken: höchstens 3 fsync je
    /// Einheit und eine feste Zahl davor.
    func testLatencyTheNumberOfDurableWaitPointsBeforeTheFirstUnitIsFixedAndEachFurtherUnitIsBounded() async {
        let corpus = AssistantCorpus()
        let blocks = ["Quasselbert wandert mit dem Fahrrad nach Pappelhausen.", "Quasselbert trifft in Pappelhausen die Tante Lavendula.",
                      "Quasselbert isst in Pappelhausen mit der Tante Lavendula Kuchen."]
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", detail: "Autorin Zimtstern",
                                         sections: blocks.enumerated().map { ("S. \($0.offset + 7)", $0.element) }))
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: blocks.joined(separator: " ")), knowledge: AssistantKnowledge(corpus: corpus))
        var measured: [[Int]] = []
        for id in ["r1", "r2"] {
            let before = audit.syncCount
            var marks: [Int] = []
            for await e in s.events(AssistantRequest(requestId: id, text: "Was macht Quasselbert in Pappelhausen mit der Tante Lavendula?")) {
                if case .sentence = e { marks.append(audit.syncCount - before) }
            }
            measured.append(marks)
        }
        print("LATENZ fsync bis zur Einheit (Anfrage 1, Anfrage 2): \(measured)")
        for marks in measured {
            XCTAssertGreaterThanOrEqual(marks.count, 3, "der Vorspann und die Einheiten kamen: \(marks)")
            XCTAssertLessThanOrEqual(marks.first ?? .max, 7, "vor der ersten Einheit eine feste, kleine Zahl: \(marks)")
            for (before, after) in zip(marks, marks.dropFirst()) {
                XCTAssertLessThanOrEqual(after - before, 3, "jede weitere Einheit kostet höchstens drei fsync: \(marks)")
            }
        }
    }
    // MARK: Runde 8, R1: Blocktreue bis in jede Ausgabe

    private let twoBlocks = ["Quasselbert wandert mit dem Fahrrad nach Pappelhausen.", "Quasselbert trifft in Pappelhausen die Tante Lavendula."]

    private func twoBlockKnowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Titel Flimmerkasten", detail: "Autorin Zimtstern",
                                         sections: twoBlocks.enumerated().map { ("S. \($0.offset + 7)", $0.element) }))
        return AssistantKnowledge(corpus: corpus)
    }

    /// R1: Der Block `"  Alpha Beta.  "` (Rand-Leerraum) ist in jedem Weg derselbe: live ausgegeben, im Verlauf der Sitzung, im
    /// Abschnitt des Beitrags, in der Verlaufsdatei und nach dem Laden. Gegen 4d802f3 ROT (gemessen): Der Beitrag entstand aus dem
    /// gesprochenen TEXT (die Steuerung trimmt, der Kern zerlegte neu), der Block stand im Verlauf nicht mehr zeichengleich.
    func testR1ABlockWithEdgeWhitespaceIsByteEqualLiveInTheHistoryAndAfterSaveAndLoad() async throws {
        let stored = "  Alpha Beta.  "
        let corpus = AssistantCorpus()
        await corpus.upsert(note("Erster Block.\n\n" + stored + "\n\nDritter Block."))
        let s = session(nil, model: EchoModel(text: "Alpha Beta."), knowledge: AssistantKnowledge(corpus: corpus))
        var live: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Alpha Beta?")) {
            if case .sentence(_, _, let text, let speech, .grounded, _, true) = e {
                live.append(text)
                XCTAssertEqual(Array(speech.utf8), Array(stored.utf8), "auch das Sprechfeld des Auszugs ist der Blocktext selbst")
            }
        }
        XCTAssertEqual(live.map { Array($0.utf8) }, [Array(stored.utf8)], "live")
        await s.commit(requestId: "r1", user: "Alpha Beta?", spoken: .all, interrupted: false)
        let history = await s.history
        XCTAssertEqual(history.count, 1)
        XCTAssertEqual(history.first.map { Array($0.assistant.utf8) }, Array(stored.utf8), "im Verlauf der Sitzung")
        XCTAssertEqual(history.first?.parts.map { Array($0.text.utf8) }, [Array(stored.utf8)], "im Abschnitt des Beitrags")
        let files = AssistantHistoryStore(directory: dir.appendingPathComponent("verlauf", isDirectory: true))
        try files.save(history)
        let loaded = AssistantTurn.admitted(files.load(), released: true)
        XCTAssertEqual(loaded.count, 1, "der Beitrag besteht die Prüfung beim Laden")
        XCTAssertEqual(loaded.first.map { Array($0.assistant.utf8) }, Array(stored.utf8), "nach dem Laden")
        XCTAssertEqual(loaded.first?.parts.map { Array($0.text.utf8) }, [Array(stored.utf8)], "Abschnitt nach dem Laden")
    }

    /// R1: Ein Quellblock kommt ganz in den Verlauf oder gar nicht. Die Steuerung meldet die ganz gesprochenen EINHEITEN; ein
    /// Block, dessen Ausgabe unterbrochen wurde, steht nie gekürzt im Beitrag. Gegen 4d802f3 ROT (gemessen): Die Steuerung gab den
    /// gesprochenen Text mit, und der Kern ordnete einen angesprochenen Satz als Zitat ein (gekürzter Block im Verlauf).
    func testR1ABlockIsInTheHistoryWholeOrNotAtAllNeverCutOrReSplit() async {
        let s = session(nil, model: EchoModel(text: twoBlocks.joined(separator: " ")), knowledge: await twoBlockKnowledge())
        var quoteEvents: [Int] = []
        var texts: [String] = []
        var index = 0
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Was macht Quasselbert in Pappelhausen mit der Tante Lavendula?")) {
            guard case .sentence(_, _, let text, _, let status, _, _) = e else { continue }
            if status == .grounded { quoteEvents.append(index); texts.append(text) }
            index += 1
        }
        guard quoteEvents.count == 2 else { return XCTFail("zwei Quellblöcke erwartet: \(texts)") }
        // Der zweite Block wurde unterbrochen: Die Steuerung meldet nur die ganz gesprochene Einheit.
        await s.commit(requestId: "r1", user: "Was macht Quasselbert?", spoken: .events([quoteEvents[0]]), interrupted: true)
        let history = await s.history
        XCTAssertEqual(history.count, 1)
        XCTAssertEqual(history.first?.assistant, texts[0], "nur der erste Block, ganz")
        XCTAssertEqual(history.first?.parts.count, 1)
        XCTAssertFalse(history.first?.assistant.contains(texts[1]) ?? true)
        XCTAssertFalse(history.first?.assistant.contains("Tante") ?? true, "kein Rest des unterbrochenen Blocks")
        XCTAssertTrue(history.first?.interrupted ?? false)
        // Nichts ganz gesprochen: gar kein Beitrag.
        for await _ in s.events(AssistantRequest(requestId: "r2", text: "Was macht Quasselbert in Pappelhausen mit der Tante Lavendula?")) {}
        await s.commit(requestId: "r2", user: "Und?", spoken: .events([]), interrupted: true)
        let after = await s.history
        XCTAssertEqual(after.count, 1, "ein Beitrag ohne ganz gesprochene Einheit entsteht nicht")
    }

    /// R1/U2: Beiträge ohne gespeicherte, vollständig markierte Einheiten (ältere Dateien) und Zitate ohne Quellzeile werden nie
    /// angezeigt und nie in einen Prompt gegeben: weder beim Laden noch beim Wiederherstellen. Gegen 4d802f3 ROT (gemessen): Solche
    /// Beiträge wurden angezeigt (als einfacher Text oder „Quelle nicht gespeichert“).
    func testR1TurnsWithoutStoredMarkedUnitsAreNeverShownNeverInAPromptAndAreNotRestored() async throws {
        let passage = assistantPassage(.books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: "Hans Castorp fährt nach Davos.")
        let legacy = AssistantTurn(user: "Alt", assistant: "Hans Castorp fährt nach Davos.", sources: [passage], model: "T", round: .data)
        let withoutSource = AssistantTurn(user: "Ohne Quelle", assistant: "Hans Castorp fährt nach Davos.",
                                          parts: [AssistantTurnPart(text: "Hans Castorp fährt nach Davos.", unsourced: false, quote: true)],
                                          model: "T", round: .data)
        let good = AssistantTurn(user: "Gut", assistant: "Hans Castorp fährt nach Davos.", sources: [passage],
                                 parts: [AssistantTurnPart(text: "Hans Castorp fährt nach Davos.", unsourced: false, quote: true, source: passage.excerptLine)],
                                 model: "T", round: .data)
        let all = [legacy, withoutSource, good]
        for released in [false, true] {
            XCTAssertEqual(AssistantTurn.admitted(all, released: released).map(\.user), ["Gut"], "released: \(released)")
        }
        // Eine alte Datei (JSON ohne `parts`) besteht die Prüfung nicht.
        let oldFile = Data(#"{"user":"Alt","assistant":"Text","interrupted":false,"sources":[],"general":false,"model":"T","at":0}"#.utf8)
        let decoded = try JSONDecoder().decode(AssistantTurn.self, from: oldFile)
        XCTAssertEqual(AssistantTurn.admitted([decoded], released: true).count, 0)
        // Wiederherstellen und Anlegen mit Verlauf: nur der gute Beitrag.
        let s = session(nil)
        await s.restore(history: all)
        let restoredTurns = await s.history
        XCTAssertEqual(restoredTurns.map(\.user), ["Gut"])
        let created = AssistantSession(model: EchoModel(), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: config,
                                       history: all, released: true)
        let createdTurns = await created.history
        XCTAssertEqual(createdTurns.map(\.user), ["Gut"])
        // Die Datei bleibt unberührt, bis der nächste Verlauf gespeichert wird.
        let files = AssistantHistoryStore(directory: dir.appendingPathComponent("verlauf", isDirectory: true))
        try files.save(all)
        XCTAssertEqual(files.load().count, 3)
        XCTAssertEqual(AssistantTurn.admitted(files.load(), released: true).map(\.user), ["Gut"])
    }

    /// U2 (Ansicht ohne Marke zeigt nichts): Die Ansicht zeigt nur, was `displayedParts` (Beitrag) und `displayed` (laufende Antwort)
    /// liefern. Ohne Marke kommt nichts heraus: weder der bloße Text (`assistant`), noch ein Zitat ohne Quellzeile oder mit einer Zeile
    /// ohne den festen Beginn, noch eine Einheit ohne Kennzeichnung; ein unterbrochener Beitrag trägt die Unterbrechung NICHT im Text
    /// der Einheiten (kein „ …“). Die Wahrheitstafel von `isMarked` ist hier vollständig. Gegen 4d802f3 NICHT AUSDRÜCKBAR (die Ansicht
    /// fiel dort auf den Text zurück, im Mac-Ziel; `displayedParts`/`displayed` gab es nicht).
    func testU2TheViewShowsNothingWithoutAMarkAndNeverAddsAnEllipsisToUnits() {
        let source = "Wörtlicher Auszug aus ‚Der Zauberberg‘ · S. 12"
        let quoteWithSource = AssistantTurnPart(text: "Block eins.\nZweite Zeile.", unsourced: false, quote: true, source: source)
        let quoteWithoutSource = AssistantTurnPart(text: "Ohne Quelle", unsourced: false, quote: true)
        let quoteBareSource = AssistantTurnPart(text: "Nur Präfix", unsourced: false, quote: true, source: AssistantPassage.excerptPrefix)
        let quoteOldSource = AssistantTurnPart(text: "Alte Zeile", unsourced: false, quote: true, source: "Buch · Der Zauberberg · S. 12")
        let plain = AssistantTurnPart(text: "Einfacher Text", unsourced: false)
        let note = AssistantTurnPart(text: "Fest.", unsourced: false, note: AssistantTurnPart.templateNote)
        let unsourced = AssistantTurnPart(text: "Allgemein.", unsourced: true)
        // Wahrheitstafel
        XCTAssertTrue(quoteWithSource.isMarked)
        XCTAssertTrue(note.isMarked)
        XCTAssertTrue(unsourced.isMarked)
        for unmarked in [quoteWithoutSource, quoteBareSource, quoteOldSource, plain] {
            XCTAssertFalse(unmarked.isMarked, unmarked.text)
        }
        // Laufende Antwort: nur markierte Einheiten, jede unverändert.
        let live = AssistantTurnPart.displayed([plain, quoteWithSource, quoteWithoutSource, note])
        XCTAssertEqual(live, [quoteWithSource, note])
        XCTAssertEqual(AssistantTurnPart.displayed([plain, quoteWithoutSource]), [], "ohne Marke steht nichts da")
        // Beitrag: alles oder nichts; kein Rückfall auf `assistant`.
        let whole = AssistantTurn(user: "F", assistant: "Block eins. Zweite Zeile. Fest.", interrupted: true,
                                  parts: [quoteWithSource, note], model: "T", round: .data)
        XCTAssertEqual(whole.displayedParts, [quoteWithSource, note])
        XCTAssertFalse(whole.displayedParts.contains { $0.text.hasSuffix("…") || $0.text.contains(" …") },
                       "die Unterbrechung steht nicht im Text der Einheiten")
        XCTAssertEqual(whole.displayedParts.first?.text, "Block eins.\nZweite Zeile.", "der Block bleibt buchstabengleich")
        let mixed = AssistantTurn(user: "F", assistant: "Text", parts: [quoteWithSource, plain], model: "T", round: .data)
        XCTAssertEqual(mixed.displayedParts, [], "ein Beitrag mit einer Einheit ohne Marke zeigt gar nichts")
        let legacy = AssistantTurn(user: "F", assistant: "Alter Text", model: "T", round: .data)
        XCTAssertEqual(legacy.displayedParts, [], "ohne gespeicherte Einheiten kein Rückfall auf den Text")
    }

    /// U2 (Entstehung der Einheiten): Eine Einheit, die über `AssistantTurnPart.append` entsteht, ist genau dann markiert, wenn sie ein
    /// Zitat MIT Quelle, eine Kennzeichnung (feste Antwort, Bestandsangabe) oder „ohne Quelle“ ist. Ein Zitat ohne Quelle und ein
    /// Satz „gedeckt“ ohne Zitat (kein Auszug) tragen keine Marke und werden nie angezeigt.
    func testU2UnitsCreatedByAppendAreMarkedOnlyWithQuoteAndSourceOrANote() {
        let passage = assistantPassage(.books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: "Hans Castorp fährt nach Davos.")
        var list: [AssistantTurnPart] = []
        AssistantTurnPart.append(to: &list, text: "Auszug.", status: .grounded, quote: true, sources: [passage])
        AssistantTurnPart.append(to: &list, text: "Hallo.", status: .neutral, quote: false, sources: [])
        AssistantTurnPart.append(to: &list, text: "Du hast 3 Bücher.", status: .inventory, quote: false, sources: [])
        AssistantTurnPart.append(to: &list, text: "Allgemein.", status: .unsourced, quote: false, sources: [])
        XCTAssertEqual(list.count, 4)
        XCTAssertTrue(list.allSatisfy(\.isMarked), "\(list)")
        XCTAssertEqual(list[0].source, passage.excerptLine)
        var broken: [AssistantTurnPart] = []
        AssistantTurnPart.append(to: &broken, text: "Zitat ohne Quelle.", status: .grounded, quote: true, sources: [])
        AssistantTurnPart.append(to: &broken, text: "Gedeckt, aber kein Auszug.", status: .grounded, quote: false, sources: [passage])
        XCTAssertEqual(broken.count, 2)
        XCTAssertEqual(AssistantTurnPart.displayed(broken), [], "ohne Marke steht nichts da")
    }

    /// R1 (Weg zur Stimme): Das Sprechfeld eines Auszugs ist der Blocktext selbst: kein Dezimalkomma, keine Auflösung von Kürzeln,
    /// keine Entfernung von Markdown, Adressen oder Zeichen. Gegen 4d802f3 ROT (gemessen): Das Sprechfeld war `AssistantSpeech.render`
    /// des Blocks („3,5“, ohne Adresse, ohne Markdown).
    func testR1TheSpeechFieldOfAnExcerptIsTheBlockTextForEveryKindOfContent() async {
        let blocks = ["Der Zaun misst 3.5 m, siehe https://example.org/zaun und ist **frisch** gestrichen \u{1F600}.",
                      "Der Zaun misst z. B. 3.5 Meter und ist  \u{201E}frisch\u{201C}  gestrichen."]
        for stored in blocks {
            let corpus = AssistantCorpus()
            await corpus.upsert(note(stored))
            let s = session(nil, model: EchoModel(text: stored), knowledge: AssistantKnowledge(corpus: corpus))
            var pairs: [(text: String, speech: String)] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: "Wie misst der Zaun und ist er frisch gestrichen?")) {
                if case .sentence(_, _, let text, let speech, .grounded, _, true) = e { pairs.append((text, speech)) }
            }
            XCTAssertEqual(pairs.count, 1, "das Zitat kam: \(stored)")
            XCTAssertEqual(pairs.first.map { Array($0.text.utf8) }, Array(stored.utf8))
            XCTAssertEqual(pairs.first.map { Array($0.speech.utf8) }, Array(stored.utf8), "Sprechfeld = Blocktext, Byte für Byte")
        }
    }

    // MARK: Runde 8, R2: das Verzeichnis nach dem Verdichten

    /// R2: Scheitert der fsync des Verzeichnisses NACH dem Umbenennen beim Verdichten, ist das ein Schreibfehler: Die neue Fassung
    /// liegt schon unter dem Namen der Datei, der Eintrag ist aber nicht bestätigt. Bis ein späterer fsync das Verzeichnis bestätigt
    /// hat, gilt keine weitere Zeile als gesichert; jeder Wartepunkt versucht es zuerst erneut. Gegen 4d802f3 ROT (gemessen): Der
    /// Fehler wurde verschluckt (`try?`), die Zeile galt als gesichert.
    func testR2ADirectoryFsyncFailingAfterTheCompactionRenameIsAWriteErrorUntilALaterOneConfirmsIt() async throws {
        let lineText = "{\"t\":\"event\",\"kind\":\"unsourcedOn\"}\n"
        try String(repeating: lineText, count: AssistantAuditWriter.maxUnits + 40).write(to: protocolFile, atomically: true, encoding: .utf8)
        let rec = Recorder()
        let fs = CxFS("journal", rec)
        fs.fail(.syncDirectory)
        let audit = log(fileSystem: fs)
        var line = AssistantJournalLine(.event)
        line.at = Date()
        audit.enqueue(line, key: "k1")
        let first = await audit.waitForLine(key: "k1")
        XCTAssertEqual(first, .writeFailed, "die erste Zeile (löst das Verdichten aus) gilt nicht als gesichert")
        XCTAssertGreaterThanOrEqual(rec.all.filter { $0 == "journal:syncDirectory" }.count, 1)
        let attempts = rec.all.filter { $0 == "journal:syncDirectory" }.count
        audit.enqueue(line, key: "k2")
        let second = await audit.waitForLine(key: "k2")
        XCTAssertEqual(second, .writeFailed, "auch die nächste Zeile nicht: das Verzeichnis ist weiter unbestätigt")
        XCTAssertGreaterThan(rec.all.filter { $0 == "journal:syncDirectory" }.count, attempts, "der Wartepunkt versuchte es erneut")
        fs.heal()
        audit.enqueue(line, key: "k3")
        let third = await audit.waitForLine(key: "k3")
        XCTAssertNil(third, "bestätigt: wieder gesichert")
        let calls = rec.all.filter { $0 == "journal:syncDirectory" }.count
        audit.enqueue(line, key: "k4")
        let fourth = await audit.waitForLine(key: "k4")
        XCTAssertNil(fourth)
        XCTAssertEqual(rec.all.filter { $0 == "journal:syncDirectory" }.count, calls, "einmal bestätigt, kein weiterer Aufwand")
    }

    /// R2: Dasselbe beim Anlegen der Datei (die Datei gab es nicht): Das Verzeichnis ist erst mit dem bestätigten fsync gesichert.
    func testR2TheDirectoryFsyncOfANewlyCreatedFileIsRetriedByEveryWaitPointUntilItSucceeds() async {
        let rec = Recorder()
        let fs = CxFS("journal", rec)
        fs.fail(.syncDirectory)
        let audit = log(fileSystem: fs)
        var line = AssistantJournalLine(.event)
        line.at = Date()
        for key in ["k1", "k2"] {
            audit.enqueue(line, key: key)
            let error = await audit.waitForLine(key: key)
            XCTAssertEqual(error, .writeFailed, key)
        }
        XCTAssertGreaterThanOrEqual(rec.all.filter { $0 == "journal:syncDirectory" }.count, 2, "jede Zeile versuchte es erneut")
        fs.heal()
        audit.enqueue(line, key: "k3")
        let third = await audit.waitForLine(key: "k3")
        XCTAssertNil(third)
    }

    // MARK: Runde 8, R5: der Bericht der Steuerung wird im Lock geprüft

    /// R5: Ein von der Steuerung gemeldeter Audio-Fehler `a!g` gilt nur, wenn `g` im Lock der Ordnung noch die Generation ist. War sie
    /// inzwischen erhöht (8), steht „verworfen“ mit der gefundenen Generation (`a-8`), nie „fehlgeschlagen unter 7“. Gegen 4d802f3
    /// ROT (gemessen): Die Zeile `a!7` stand trotz Generation 8 in der Datei.
    func testR5AnAudioFailureReportedAfterARevocationIsWrittenAsDiscardedWithTheFoundGeneration() async throws {
        let store = openStore()
        let audit = log()
        let s = session(store, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        let g = s.releaseGeneration
        var quoteEvent = -1
        var index = 0
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {
            if case .sentence(_, _, _, _, .grounded, _, _) = e { quoteEvent = index }
            if case .sentence = e { index += 1 }
        }
        XCTAssertGreaterThanOrEqual(quoteEvent, 0)
        // Zuerst gilt die Generation: `a!g`, `d+g` stehen, wie gemeldet.
        s.noteCommit(requestId: "r1", index: quoteEvent, stage: .audio, ok: false, generation: g, failed: true)
        s.noteCommit(requestId: "r1", index: quoteEvent, stage: .display, ok: true, generation: g)
        await s.releaseDidChange()   // Generation g + 1
        let found = s.releaseGeneration
        XCTAssertEqual(found, g + 1)
        // Nach der Erhöhung gemeldet unter der alten Generation: verworfen mit der gefundenen.
        s.noteCommit(requestId: "r1", index: quoteEvent, stage: .audio, ok: false, generation: g, failed: true)
        s.noteCommit(requestId: "r1", index: quoteEvent, stage: .display, ok: true, generation: g)
        await s.commit(requestId: "r1", user: "x", spoken: .nothing, interrupted: true, revoked: true)
        let allEntries = await audit.entries()
        let entry = allEntries.first { $0.requestId == "r1" }
        let quote = entry?.trace.first { $0.verdict == .grounded }
        XCTAssertEqual(quote?.commits.filter { $0.stage == .audio }, [
            AssistantCommitMark(stage: .audio, ok: false, generation: g, failed: true),
            AssistantCommitMark(stage: .audio, ok: false, generation: found),
        ], "a!g solange g galt, danach a-(g+1)")
        XCTAssertEqual(quote?.commits.filter { $0.stage == .display }, [
            AssistantCommitMark(stage: .display, ok: true, generation: g),
            AssistantCommitMark(stage: .display, ok: false, generation: found),
        ])
        XCTAssertEqual(lines(["commit"]).filter { ($0["mark"] as? String) == "a!\(g)" }.count, 1, "keine Mischung: `a!g` steht genau einmal")
    }

    // MARK: Runde 8, R6: Fehler hängen an ihrem Anspruch

    /// R6: Der Fehler einer Anfrage geht nicht verloren, auch wenn 300 fremde Schlüssel scheitern (die Tabelle der Fehler war auf 256
    /// begrenzt und verdrängte den ältesten, also den der Anfrage). Der Fehler hängt am Anspruch der Anfrage und endet erst mit ihm.
    /// Gegen 4d802f3 ROT (gemessen): Der Fehler der Anfrage war nach 300 fremden Fehlern weg (`flush` lieferte `nil`).
    func testR6TheErrorOfARequestSurvivesThreeHundredForeignErrorKeys() async {
        let failing = Flag()
        let audit = log(appender: { url, data in
            if failing.value { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        audit.claim(request: "r1")
        failing.value = true
        var own = AssistantJournalLine(.round, req: "r1")
        own.at = Date()
        audit.enqueue(own)
        for i in 0..<300 {
            var foreign = AssistantJournalLine(.event)
            foreign.at = Date()
            audit.enqueue(foreign, key: "fremd:\(i)", onFailure: { _ in })
        }
        await audit.barrier()
        XCTAssertTrue(audit.hasFailed(request: "r1"), "sichtbar ohne Warten")
        let seen = await audit.waitUntilWritten(request: "r1")
        XCTAssertEqual(seen, .writeFailed, "klebrig: bleibt, bis die Anfrage abgeschlossen ist")
        let again = await audit.waitUntilWritten(request: "r1")
        XCTAssertEqual(again, .writeFailed, "alle Entscheidungen lesen dieselbe Quelle")
        XCTAssertTrue(audit.hasFailed(request: "r1"))
        audit.release(request: "r1")
        XCTAssertEqual(audit.claimCount, 0, "mit dem Abschluss endet der Anspruch")
    }

    /// R6: Derselbe Fall für die Zeile einer Änderung (Schlüssel `chg:…`): Sie wird mit `waitForLine` erwartet, ihr Fehler geht nicht
    /// in einer Tabelle unter, aus der fremde Fehler ihn verdrängen. Gegen 4d802f3 ROT (gemessen).
    func testR6TheErrorOfAChangeLineSurvivesThreeHundredForeignErrorKeys() async {
        let failing = Flag()
        let audit = log(appender: { url, data in
            if failing.value { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        failing.value = true
        var change = AssistantJournalLine(.event)
        change.at = Date()
        audit.enqueue(change, key: "chg:1:1")
        for i in 0..<300 { audit.enqueue(change, key: "fremd:\(i)", onFailure: { _ in }) }
        await audit.barrier()
        XCTAssertTrue(audit.hasFailed(request: "chg:1:1"))
        let error = await audit.waitForLine(key: "chg:1:1")
        XCTAssertEqual(error, .writeFailed)
        XCTAssertEqual(audit.claimCount, 0, "gemeldet: der Anspruch ist beendet")
        // Ohne Anspruch (niemand wartet) werden nur Fehler fremder Schlüssel begrenzt und verdrängt.
        failing.value = false
        audit.enqueue(change, key: "chg:1:2")
        let ok = await audit.waitForLine(key: "chg:1:2")
        XCTAssertNil(ok)
    }

    /// R6: Nach `commit` hat die Anfrage keinen Anspruch mehr (die Zahl der Ansprüche wächst nicht mit den Anfragen).
    func testR6TheClaimOfARequestEndsWithItsCommitAndNoClaimIsLeftBehind() async {
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        for i in 0..<5 {
            let id = "r\(i)"
            for await _ in s.events(AssistantRequest(requestId: id, text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {}
            XCTAssertEqual(audit.claimCount, 1, "bis zum Abschluss der Anfrage hält sie ihren Anspruch")
            await s.commit(requestId: id, user: "x", spoken: .all, interrupted: false)
            XCTAssertEqual(audit.claimCount, 0, "mit dem Commit endet er")
        }
    }

    // MARK: Runde 8, R7: der Leser schließt nie über die Art hinweg

    private func encoded(_ line: AssistantJournalLine) -> Data {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return (try? e.encode(line)) ?? Data()
    }

    private func writeJournal(_ journal: [AssistantJournalLine]) throws {
        var raw = Data()
        for l in journal {
            raw.append(encoded(l))
            raw.append(10)
        }
        try raw.write(to: protocolFile)
    }

    private func journalStart(_ req: String) -> AssistantJournalLine {
        var l = AssistantJournalLine(.start, req: req)
        l.at = Date(timeIntervalSince1970: 1_760_000_000)
        l.session = "sitzung"
        l.model = .other
        l.cloud = false
        l.unsourced = false
        l.consent = 0
        l.ctx = .data
        l.reason = .locked
        l.round = .data
        l.gen = 1
        return l
    }

    private func journalStep(_ req: String, _ n: Int, _ verdict: AssistantTraceVerdict) -> AssistantJournalLine {
        var l = AssistantJournalLine(.step, req: req)
        l.n = n
        l.verdict = verdict
        l.gen = 1
        return l
    }

    private func journalCommit(_ req: String, _ n: Int, _ stage: AssistantCommitStage, ok: Bool = true, failed: Bool = false) -> AssistantJournalLine {
        var l = AssistantJournalLine(.commit, req: req)
        l.n = n
        l.mark = AssistantCommitMark(stage: stage, ok: ok, generation: 1, failed: failed)
        return l
    }

    private func journalEnd(_ req: String, outcome: AssistantAuditEntry.Outcome = .completed, lost: Bool? = nil) -> AssistantJournalLine {
        var l = AssistantJournalLine(.end, req: req)
        l.at = Date(timeIntervalSince1970: 1_760_000_100)
        l.outcome = outcome
        l.level = .s1
        l.areas = ["books"]
        l.lost = lost
        return l
    }

    /// R7: `e+ d+ a!`, der Beitrag wurde angehängt, das Prozessende kam vor der Zeile `h`: Der Verlauf dieser Einheit ist UNBEKANNT.
    /// Der Leser schloss vorher aus `a!` auf „Verlauf nicht aufgenommen“. Ein Vergleich: Dieselben Zeilen in einer regulär beendeten
    /// Anfrage heißen „nicht ausgeführt“ (der Schreiber schreibt für eine Handlung, die nicht begann, keine Zeile). Gegen 4d802f3 ROT
    /// (gemessen): `result(for: .history)` lieferte `notPlanned` statt `unknown`.
    func testR7AnAudioFailureWithTheHistoryLineMissingAfterTheProcessEndIsUnknownForTheHistoryAndNotExecutedWhenTheRequestEndedRegularly() async throws {
        let unit: [AssistantJournalLine] = [
            journalStart("a"), journalStep("a", 1, .grounded),
            journalCommit("a", 1, .emit), journalCommit("a", 1, .display), journalCommit("a", 1, .audio, ok: false, failed: true),
        ]
        try writeJournal(unit)   // kein `end`: Prozessende
        let openEntries = await log().entries()
        let open = try XCTUnwrap(openEntries.first { $0.requestId == "a" })
        XCTAssertEqual(open.outcome, .incomplete)
        let cut = try XCTUnwrap(open.trace.first)
        XCTAssertEqual(cut.result(for: .emit), .executed)
        XCTAssertEqual(cut.result(for: .display), .executed)
        XCTAssertEqual(cut.result(for: .audio), .failed)
        XCTAssertEqual(cut.result(for: .history), .unknown, "der Verlauf: unbekannt, nicht aus `a!` geschlossen")
        XCTAssertEqual(cut.unknownStages, [.history])
        try? FileManager.default.removeItem(at: protocolFile)
        try writeJournal(unit + [journalEnd("a")])
        let closedEntries = await log().entries()
        let closed = try XCTUnwrap(closedEntries.first { $0.requestId == "a" })
        XCTAssertEqual(closed.outcome, .completed)
        XCTAssertEqual(closed.trace.first?.result(for: .history), .notPlanned, "regulär beendet: nicht ausgeführt")
        XCTAssertEqual(closed.trace.first?.result(for: .audio), .failed)
    }

    /// R7: Eine Anfrage, bei der Zeilen nicht geschrieben werden konnten (Vermerk `lost` im `end`), gilt nicht als regulär beendet:
    /// Was in der Datei fehlt, ist unbekannt, auch wenn der Ausgang „Fehler“ ist.
    func testR7ARequestThatLostLinesIsNotReadAsRegularlyEnded() async throws {
        try writeJournal([
            journalStart("a"), journalStep("a", 1, .grounded), journalCommit("a", 1, .emit),
            journalEnd("a", outcome: .failed, lost: true),
        ])
        let readEntries = await log().entries()
        let entry = try XCTUnwrap(readEntries.first { $0.requestId == "a" })
        XCTAssertEqual(entry.outcome, .failed)
        let step = try XCTUnwrap(entry.trace.first)
        XCTAssertEqual(step.result(for: .display), .unknown)
        XCTAssertEqual(step.result(for: .audio), .unknown)
        XCTAssertEqual(step.result(for: .history), .unknown)
        XCTAssertEqual(step.result(for: .emit), .executed, "was steht, gilt")
    }

    /// R7: Ein verworfener Kandidat behauptet nichts: Er hat in jeder Art „nicht ausgeführt“, auch in einer unvollständigen Anfrage.
    func testR7ADiscardedCandidateClaimsNothingEvenInAnIncompleteRequest() async throws {
        try writeJournal([journalStart("a"), journalStep("a", 1, .droppedNotSnapped)])
        let readEntries = await log().entries()
        let entry = try XCTUnwrap(readEntries.first { $0.requestId == "a" })
        XCTAssertEqual(entry.outcome, .incomplete)
        XCTAssertEqual(AssistantTraceStep.unitStages.map { entry.trace[0].result(for: $0) }, [.notPlanned, .notPlanned, .notPlanned, .notPlanned])
    }

    /// Suchlauf (e), die eine Leserregel aus einer Abwesenheit (`counts(from:)`): Fehlt die Zeile des Ausgabe-Commits ganz (Prozessende
    /// nach dem Schritt), zählt der Satz als möglicherweise ausgegeben, nie als widerrufen; die Art selbst ist „unbekannt“. Ein
    /// verworfener Kandidat zählt nie als ausgegeben. Gegen 4d802f3 gleich (die Regel besteht unverändert; der Test hält die
    /// vorsichtige Richtung fest).
    func testECountsTreatAMissingEmitLineAsPossiblyOutputAndTheKindAsUnknown() async throws {
        try writeJournal([journalStart("a"), journalStep("a", 1, .grounded), journalStep("a", 2, .droppedNotSnapped)])
        let readEntries = await log().entries()
        let entry = try XCTUnwrap(readEntries.first { $0.requestId == "a" })
        XCTAssertEqual(entry.outcome, .incomplete)
        XCTAssertEqual(entry.counts.grounded, 1, "möglicherweise ausgegeben")
        XCTAssertEqual(entry.counts.revoked, 0, "nie als sicher nicht ausgegeben")
        XCTAssertEqual(entry.counts.droppedNotSnapped, 1)
        XCTAssertEqual(entry.trace[0].result(for: .emit), .unknown)
        XCTAssertEqual(entry.trace[1].result(for: .emit), .notPlanned, "ein verworfener Kandidat behauptet nichts")
    }

    // MARK: Runde 8, Tabelle (c): Obergrenzen, deren Überlauf nichts verschlucken darf

    /// (c) Der Index der Schritte ist voll (hier 2 Einheiten): Die dritte Einheit geht NICHT hinaus, die Zeile des Ausgabe-Commits
    /// sagt „nicht ausgegeben“, die Antwort endet mit dem Ausgang „Grenze erreicht“. Bei den festen Grenzen (512 gegenüber 200
    /// Schritten) läuft der Index nie voll; der Test setzt die Grenze klein. Gegen 4d802f3 NICHT AUSDRÜCKBAR (die Grenzen sind dort
    /// nicht einstellbar); statischer Beleg: `AssistantStepIndex.append` ignorierte dort die volle Tabelle still, die Einheit ging
    /// ohne Schritt hinaus.
    func testTableCAFullStepIndexStopsTheAnswerAndNothingGoesOutWithoutAStep() async throws {
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks.joined(separator: " ")), knowledge: await twoBlockKnowledge(),
                        stepIndex: AssistantStepIndex(maxSteps: 2))
        var units: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Was macht Quasselbert in Pappelhausen mit der Tante Lavendula?")) {
            if case .sentence(_, _, let text, _, _, _, _) = e { units.append(text) }
        }
        XCTAssertEqual(units.count, 2, "Ansage und erster Block; danach ging nichts mehr hinaus: \(units)")
        await s.commit(requestId: "r1", user: "x", spoken: .all, interrupted: false)
        let auditEntries = await audit.entries()
        let entry = try XCTUnwrap(auditEntries.first { $0.requestId == "r1" })
        XCTAssertEqual(entry.outcome, .limited)
        let last = try XCTUnwrap(entry.trace.last)
        XCTAssertEqual(last.verdict, .announcement, "die Ansage des zweiten Blocks war die dritte Einheit")
        XCTAssertEqual(last.result(for: .emit), .discarded, "ausdrücklich „nicht ausgegeben“")
        XCTAssertFalse(units.contains(twoBlocks[1]), "der zweite Block ging nicht hinaus")
    }

    /// (c) Ein verdrängter Eintrag des Index (hier: ein Eintrag für die Anfragen): Für die verdrängte Anfrage startet nie ein Ton
    /// (Schreibfolge 6: nichts wird wiederholt), ihre Meldungen werden ignoriert, und ihre Einheiten kommen nicht als „ganz gesprochen“
    /// in einen Beitrag. Bei den festen Grenzen (32 gegenüber 16 gemerkten Anfragen) kann die Steuerung nie eine verdrängte Anfrage
    /// melden. Gegen 4d802f3 NICHT AUSDRÜCKBAR (die Grenze ist dort nicht einstellbar).
    func testTableCAnEvictedRequestNeverStartsAudioAndItsReportsAndUnitsAreIgnored() async {
        let s = session(nil, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge(), stepIndex: AssistantStepIndex(maxRequests: 1))
        let g = s.releaseGeneration
        for id in ["r1", "r2"] {
            for await _ in s.events(AssistantRequest(requestId: id, text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {}
        }
        let started = Flag()
        let result = s.commitStart(requestId: "r1", index: 0, generation: g) { started.value = true; return true }
        XCTAssertFalse(result.ok)
        XCTAssertFalse(started.value, "für eine verdrängte Anfrage startet kein Ton")
        s.noteCommit(requestId: "r1", index: 0, stage: .display, ok: true, generation: g)   // ohne Wirkung, ohne Absturz
        await s.commit(requestId: "r1", user: "x", spoken: .events([0, 1]), interrupted: true)
        let turns = await s.history
        XCTAssertEqual(turns.count, 0, "keine Einheit der verdrängten Anfrage gilt als gesprochen")
    }

    // MARK: Runde 9, S1: der Anspruch auf den Fehler endet nur mit dem Abschluss der Anfrage

    /// Eine Anfrage bis zum Ende lesen und die Zahl ihrer Ereignisse nennen.
    private func runToEnd(_ s: AssistantSession, _ id: String) async -> Int {
        var count = 0
        for await e in s.events(AssistantRequest(requestId: id, text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {
            if case .sentence = e { count += 1 }
        }
        return count
    }

    /// Ein Protokoll, dessen Audio-Ergebniszeilen (`a!`) sich nicht schreiben lassen, solange `failAudio` gesetzt ist.
    private func logFailingAudioResults(_ failAudio: Flag) -> AssistantAuditLog {
        log(appender: { url, data in
            if failAudio.value, String(decoding: data, as: UTF8.self).contains("\"mark\":\"a!") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
    }

    /// S1 (Rest von R6): Eine Anfrage R ist fertig, aber nicht abgeschlossen; ihre Audio-Ergebniszeile ließ sich nicht schreiben (der
    /// Fehler ist klebrig); danach enden 16 andere Anfragen. Das Gedächtnis der Sitzung verdrängt R. Früher endete dabei ihr Anspruch
    /// (der Fehler war weg), der Index der Schritte kannte R aber weiter: `commitStart(R)` löste den Ton aus, und R hatte nie eine
    /// Schlusszeile. Jetzt wird R beim Verdrängen abgeschlossen (für Commits geschlossen → Schlusszeile → Anspruch beendet): Der Start
    /// löst nichts aus, und R hat eine Schlusszeile mit Ausgang „Fehler“ und `lost`. Gegen a356460 ROT.
    func testS1TheErrorClaimOfARequestEndsOnlyWithItsCompletionNeverByEviction() async throws {
        let failAudio = Flag()
        let audit = logFailingAudioResults(failAudio)
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        let g = s.releaseGeneration
        let events = await runToEnd(s, "r0")
        XCTAssertGreaterThan(events, 0, "R gibt Ereignisse aus (sonst kennt der Index R nicht)")
        failAudio.value = true
        s.noteCommit(requestId: "r0", index: 0, stage: .audio, ok: false, generation: g, failed: true)
        await audit.barrier()
        failAudio.value = false
        XCTAssertTrue(audit.hasFailed(request: "r0"), "der Schreibfehler von R steht klebrig")
        for i in 1...AssistantSession.maxRemembered { _ = await runToEnd(s, "r\(i)") }   // der 17. gemerkte Lauf verdrängt R
        let started = Flag()
        let result = s.commitStart(requestId: "r0", index: 0, generation: g) { started.value = true; return true }
        XCTAssertFalse(started.value, "für die verdrängte Anfrage startet kein Ton")
        XCTAssertFalse(result.ok)
        XCTAssertFalse(result.failed, "eine geschlossene Anfrage meldet kein Scheitern des Starts, sie löst nur nichts aus")
        await audit.barrier()
        let ends = lines(["end"]).filter { $0["req"] as? String == "r0" }
        XCTAssertEqual(ends.count, 1, "R hat genau eine Schlusszeile")
        XCTAssertEqual(ends.first?["outcome"] as? String, "failed", "Ausgang „Fehler“")
        XCTAssertEqual(ends.first?["lost"] as? Bool, true, "ihr Protokoll ist unvollständig")
        XCTAssertFalse(lines(["commit"]).contains { $0["req"] as? String == "r0" && ($0["mark"] as? String)?.hasPrefix("h+") == true },
                       "kein Beitrag der verdrängten Anfrage")
        let history = await s.history
        XCTAssertEqual(history.count, 0, "kein Beitrag")
        XCTAssertEqual(audit.claimCount, AssistantSession.maxRemembered, "R hat ihren Anspruch abgegeben, die 16 anderen halten ihren")
    }

    /// S1: Nach dem regulären Abschluss löst weder der Audiostart noch die Anzeige etwas aus, und es entsteht keine Zeile mehr hinter
    /// der Schlusszeile. Gegen a356460 ROT: Der Index kannte die Anfrage weiter, `commitStart` löste unter derselben Generation aus.
    func testS1ACommitStartAndCommitOutputAfterTheRegularCompletionTriggerNothing() async {
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        let g = s.releaseGeneration
        _ = await runToEnd(s, "r1")
        await s.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        let started = Flag()
        let shown = Flag()
        let audio = s.commitStart(requestId: "r1", index: 0, generation: g) { started.value = true; return true }
        let display = s.commitOutput(requestId: "r1", index: 0, generation: g, stage: .display) { shown.value = true }
        XCTAssertFalse(started.value, "nach dem Abschluss startet kein Ton")
        XCTAssertFalse(shown.value, "nach dem Abschluss erscheint nichts")
        XCTAssertFalse(audio.ok)
        XCTAssertFalse(display.ok)
        s.noteCommit(requestId: "r1", index: 0, stage: .audio, ok: true, generation: g)   // auch eine späte Meldung schreibt nichts mehr
        await audit.barrier()
        let kinds = lines().filter { $0["req"] as? String == "r1" }.compactMap { $0["t"] as? String }
        XCTAssertEqual(kinds.last, "end", "hinter der Schlusszeile steht nichts mehr: \(kinds)")
    }

    /// S1: Ein Commit, der VOR dem Abschluss im Lock der Ordnung läuft, hat seine Zeile vor der Schlusszeile eingereiht; der
    /// Abschluss schließt die Anfrage im selben Lock und wartet daher, bis er fertig ist (Haltepunkt: Der Start steht im Lock, der
    /// Abschluss beginnt, der Start wird freigegeben). Reihenfolge in der Datei: Audio-Marke, dann Schlusszeile.
    func testS1ACommitRunningBeforeTheCompletionStandsBeforeTheClosingLine() async throws {
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        let g = s.releaseGeneration
        _ = await runToEnd(s, "r1")
        let entered = DispatchSemaphore(value: 0)
        let proceed = DispatchSemaphore(value: 0)
        let done = DispatchSemaphore(value: 0)
        let started = Flag()
        let thread = Thread {
            _ = s.commitStart(requestId: "r1", index: 0, generation: g) {
                entered.signal()
                proceed.wait()
                started.value = true
                return true
            }
            done.signal()
        }
        thread.start()
        await awaitSignal(entered)   // der Start steht im Lock der Ordnung
        let completion = Task { await s.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false) }
        proceed.signal()
        await awaitSignal(done)
        _ = await completion.value
        XCTAssertTrue(started.value)
        await audit.barrier()
        let rows = lines().filter { $0["req"] as? String == "r1" }
        let audioMark = rows.firstIndex { ($0["mark"] as? String)?.hasPrefix("a+") == true }
        let closing = rows.firstIndex { $0["t"] as? String == "end" }
        XCTAssertNotNil(audioMark, "die Marke des Starts steht")
        XCTAssertNotNil(closing)
        if let audioMark, let closing { XCTAssertLessThan(audioMark, closing, "erst die Marke, dann die Schlusszeile") }
    }

    // MARK: Runde 9, S2: der Verlauf prüft den Schreibfehler im Lock

    /// S2: Die Audio-Ergebniszeile ließ sich nicht schreiben (klebriger Fehler); danach kommt `commit(spoken: .all)` unter derselben
    /// Generation. Früher prüfte der Verlaufs-Commit nur die Generation: Der Beitrag entstand, `h+` stand in der Datei. Jetzt prüft er im
    /// Lock auch den Fehler: kein Beitrag, `h!` für den Beitrag (Schritt 0) und `h-` je Einheit mit der geprüften Generation, Ausgang
    /// „Fehler“. Gegen a356460 ROT.
    func testS2AStickyWriteErrorStopsTheHistoryCommitNoTurnNoPlusMarkOutcomeFailed() async throws {
        let failAudio = Flag()
        let audit = logFailingAudioResults(failAudio)
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        let g = s.releaseGeneration
        _ = await runToEnd(s, "r1")
        failAudio.value = true
        s.noteCommit(requestId: "r1", index: 0, stage: .audio, ok: false, generation: g, failed: true)
        await audit.barrier()
        failAudio.value = false
        XCTAssertTrue(audit.hasFailed(request: "r1"))
        let ok = await s.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        XCTAssertFalse(ok, "das Protokoll ist nicht in Ordnung")
        let history = await s.history
        XCTAssertEqual(history.count, 0, "kein Beitrag")
        let marks = lines(["commit"]).filter { $0["req"] as? String == "r1" }.compactMap { $0["mark"] as? String }
        XCTAssertFalse(marks.contains { $0.hasPrefix("h+") }, "keine Marke „Verlauf ausgeführt“: \(marks)")
        XCTAssertTrue(marks.contains("h!\(g)"), "der Beitrag (Schritt 0): fehlgeschlagen mit der geprüften Generation: \(marks)")
        XCTAssertTrue(marks.contains("h-\(g)"), "jede Einheit: nicht im Verlauf: \(marks)")
        let end = lines(["end"]).first { $0["req"] as? String == "r1" }
        XCTAssertEqual(end?["outcome"] as? String, "failed")
        XCTAssertEqual(end?["lost"] as? Bool, true)
    }

    // MARK: Runde 9, Suchlauf (b) und (c)

    /// Suchlauf (b): Ereignisse mit Inhalt (`sources`, `done`) gehen bei einem klebrigen Schreibfehler IM Lock nicht hinaus. Der Sender
    /// bekommt den Grund (`refusal`), die Ereignisse des Stroms sind genau die ohne Fehler.
    func testSearchBDoneAndSourcesAreHeldBackInTheLockByAStickyWriteError() async {
        let failing = Flag()
        let audit = log(appender: { url, data in
            if failing.value { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        audit.claim(request: "r1")
        let release = AssistantRelease(optIn: false)
        var continuation: AsyncStream<AssistantEvent>.Continuation!
        let stream = AsyncStream<AssistantEvent> { continuation = $0 }
        let output = AssistantRunOutput(release: release, generation: release.currentGeneration, continuation: continuation,
                                        gate: AssistantRequestGate(request: "r1", index: AssistantStepIndex(), audit: audit))
        let answer = AssistantAnswer(text: "Hallo.", sources: [], general: true, firstSentenceMs: nil, totalMs: 0, unsourced: false,
                                     counts: AssistantSentenceCounts(), round: .general, generation: release.currentGeneration)
        let before = output.deliver(.sources(requestId: "r1", generation: release.currentGeneration, []))
        XCTAssertTrue(before.check.ok)
        XCTAssertNil(before.refusal, "ohne Fehler geht es hinaus")
        failing.value = true
        var own = AssistantJournalLine(.round, req: "r1")
        own.at = Date()
        audit.enqueue(own)
        await audit.barrier()
        XCTAssertTrue(audit.hasFailed(request: "r1"))
        let sources = output.deliver(.sources(requestId: "r1", generation: release.currentGeneration, []))
        let done = output.deliver(.done(requestId: "r1", answer))
        XCTAssertEqual(sources.refusal, .auditFailed)
        XCTAssertEqual(done.refusal, .auditFailed)
        XCTAssertFalse(sources.check.ok)
        XCTAssertFalse(done.check.ok)
        continuation.finish()
        var received: [AssistantEvent] = []
        for await e in stream { received.append(e) }
        XCTAssertEqual(received.count, 1, "nur das Ereignis vor dem Fehler ist hinausgegangen")
    }

    /// Suchlauf (c): Ein Abbruch vor dem Start schreibt `start` und `end`; der Anspruch endet erst, wenn der Fehler gemeldet ist. Mit
    /// gesundem Protokoll: „abgebrochen“ und beide Zeilen stehen; mit nicht schreibbarem Protokoll: „Protokoll nicht schreibbar“
    /// (früher: Der Fehler wurde verworfen, und „abgebrochen“ gab vor, es sei alles in Ordnung). Gegen a356460 ROT für den zweiten Teil.
    func testSearchCAnEarlyCancelReportsTheErrorOfItsLinesAndEndsItsClaimOnlyAfterwards() async {
        let healthy = log()
        let a = session(nil, audit: healthy)
        XCTAssertEqual(healthy.claimCount, 1, "vor der ersten Sicherung hält nur die Standzeile ihren Anspruch")
        await a.cancel(requestId: "r1")
        let ok = await drainEvents(a, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(ok.failure, .cancelled)
        XCTAssertEqual(lines(["start", "end"]).filter { $0["req"] as? String == "r1" }.count, 2, "start und end stehen")
        XCTAssertEqual(healthy.claimCount, 0, "der Anspruch der Anfrage ist beendet (der der Standzeile endet mit ihrer Sicherung im Abschluss)")
        try? FileManager.default.removeItem(at: protocolFile)
        let failing = log(appender: { _, _ in throw Injected() })
        let b = session(nil, audit: failing)
        XCTAssertEqual(failing.claimCount, 1, "vor der ersten Sicherung hält nur die Standzeile ihren Anspruch")
        await b.cancel(requestId: "r2")
        let bad = await drainEvents(b, AssistantRequest(requestId: "r2", text: "Hallo"))
        XCTAssertEqual(bad.failure, .auditUnavailable, "der Fehler der Zeilen ist der Ausgang")
        XCTAssertEqual(failing.claimCount, 0, "der Anspruch ist danach beendet (auch der der Standzeile: ihr Fehler ist abgeholt)")
    }

    // MARK: Runde 9, S3: Nachholen bei JEDER Gelegenheit, ohne Sitzung

    /// Aufbau für S3 und S4: Ein Speicher mit Protokoll ohne jede Sitzung, dessen Austausch der Datei (`storeFS`) scheitern kann und
    /// dessen Protokoll jede Zeile ablehnt, die `rejects` (auf den Text der Zeile) bejaht.
    private struct NoSession {
        let store: AssistantReleaseStore
        let storeFS: CxFS
        let audit: AssistantAuditLog
        let watch: AssistantContentWatch
    }

    private func noSession(rejects: @escaping @Sendable (String) -> Bool = { _ in false }) throws -> NoSession {
        try seed(AssistantReleaseRecord(localAreas: [.books, .notes], cloudAreas: [.books]))
        let storeFS = CxFS("store")
        let store = openStore(storeFS)
        let audit = log(appender: { url, data in
            if rejects(String(decoding: data, as: UTF8.self)) { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let watch = AssistantContentWatch()
        store.bind(watch: watch, audit: audit)   // Prozessstart: Ordnung, Protokoll, Meldestelle; kein Modell, keine Sitzung
        return NoSession(store: store, storeFS: storeFS, audit: audit, watch: watch)
    }

    /// S3: Eine Erweiterung hinterlässt die fehlende Zeile „in diesem Lauf nicht wirksam“; keine Sitzung ist offen, es kommt keine
    /// Anfrage und keine Änderung der Bereiche; der Speicher ist wieder in Ordnung. Dann ändert sich NUR eine Quelle (Klasse B): Ihre
    /// Zeile „Inhalt“ UND die ältere Pflichtzeile stehen danach dauerhaft in der Datei, das Register ist leer. Früher holten nur
    /// Anfragen und Änderungen der Klasse A nach. Gegen a356460 ROT.
    func testS3AClassBChangeWithoutASessionCatchesUpTheOlderMandatoryLine() async throws {
        let failIneffective = Flag()
        let t = try noSession(rejects: { failIneffective.value && $0.contains("\"t\":\"ineffective\"") })
        t.storeFS.fail(.rename)
        failIneffective.value = true
        _ = await t.store.updateAreas([.books], cloud: false, audit: t.audit)   // Einschränkung ohne Sicherung: Blockade
        let refused = await t.store.updateAreas([.books, .files], cloud: false, audit: t.audit)
        XCTAssertFalse(refused.applied, "die Erweiterung gilt in diesem Lauf nicht")
        XCTAssertTrue(lines(["ineffective"]).isEmpty, "die Zeile ließ sich nicht schreiben")
        XCTAssertTrue(t.store.hasUnwrittenLines, "sie steht im Register")
        await t.store.clock.awaitCatchUp()   // Haltepunkt: kein Lauf des Nachholens ist mehr unterwegs
        t.storeFS.heal()
        failIneffective.value = false
        XCTAssertTrue(lines(["ineffective"]).isEmpty, "bis hierher hat nichts nachgeholt")
        t.watch.around {}   // die EINE Änderung einer Quelle
        await t.audit.barrier()
        await t.store.clock.awaitCatchUp()
        XCTAssertEqual(lines(["ineffective"]).count, 1, "die ältere Pflichtzeile steht dauerhaft")
        XCTAssertEqual(lines(["release"]).filter { $0["cause"] as? String == "content" }.count, 1, "und die Zeile „Inhalt“ der Änderung")
        XCTAssertFalse(t.store.hasUnwrittenLines, "das Register ist leer")
    }

    /// S3 (zusätzlich): Eine nicht gesicherte Einschränkung wird bei derselben Gelegenheit erneut gesichert (auch ohne Sitzung).
    func testS3AnUnsecuredRestrictionIsSecuredAgainAtTheSameOpportunity() async throws {
        let t = try noSession()
        t.storeFS.fail(.rename)
        _ = await t.store.updateAreas([.books], cloud: false, audit: t.audit)
        XCTAssertTrue(t.store.isDirty, "die Einschränkung gilt im Prozess, die Datei ist nicht gesichert")
        XCTAssertEqual(fileRecord()?.areas(cloud: false), [.books, .notes], "die Datei enthält noch die weitere Fassung")
        await t.store.clock.awaitCatchUp()
        t.storeFS.heal()
        t.watch.around {}
        await t.audit.barrier()
        await t.store.clock.awaitCatchUp()
        XCTAssertFalse(t.store.isDirty, "wieder gesichert")
        XCTAssertEqual(fileRecord()?.areas(cloud: false), [.books], "die Datei trägt die Einschränkung")
    }

    // MARK: Runde 9, S4: das Register verliert nie eine Pflichtzeile

    /// S4 (Einheit): Das Register hat keine Obergrenze, nimmt jeden Schlüssel nur einmal, gibt die Einträge in der Reihenfolge des
    /// Eintrags heraus, gibt einen Eintrag nie zwei Abläufen zugleich und behält, was sich nicht schreiben ließ.
    func testS4TheRegisterHasNoLimitKeepsEntryOrderAndHandsNothingOutTwice() {
        var register = AssistantPendingRegister()
        let key: (Int) -> String = { "k:\($0)" }
        let build: (Int, Date) -> AssistantJournalLine = { g, at in
            var line = AssistantJournalLine(.release)
            line.gen = g
            line.at = at
            return line
        }
        let count = 20_000
        for g in 0..<count { register.addContent(generation: g, at: Date(timeIntervalSince1970: 0)) }
        register.add(AssistantPendingLine(key: "built", line: AssistantJournalLine(.event)))
        register.addContent(generation: 7, at: Date())   // dieselbe Generation: kein zweiter Eintrag
        register.add(AssistantPendingLine(key: "built", line: AssistantJournalLine(.event)))
        XCTAssertEqual(register.count, count + 1, "nichts verdrängt, nichts doppelt")
        let first = register.claim(key: key, build: build)
        XCTAssertEqual(first.count, count + 1)
        XCTAssertEqual(first.prefix(count).map(\.key), (0..<count).map(key), "Reihenfolge des Eintrags")
        XCTAssertEqual(first.last?.key, "built", "die fertige Zeile steht hinter den früheren")
        XCTAssertTrue(register.claim(key: key, build: build).isEmpty, "ein zweiter Ablauf bekommt nichts, was schon in Arbeit ist")
        for item in first { register.finish(item.sequence, written: item.key != "k:5") }
        XCTAssertEqual(register.count, 1, "nur die Zeile, die sich nicht schreiben ließ, bleibt")
        XCTAssertEqual(register.claim(key: key, build: build).map(\.key), ["k:5"], "und ist wieder zu haben")
    }

    /// S4: 1500 Zeilen „Inhalt“ ließen sich nicht schreiben (Obergrenze früher 1024: danach ging die nächste verloren und es blieb nur
    /// die Sperre aller Anfragen), dazu scheitert eine Zeile „in diesem Lauf nicht wirksam“. Der Speicher wird wieder schreibbar: Danach
    /// stehen ALLE in der Datei, in der Reihenfolge des Eintrags, das Register ist leer; das Schreiben ist EINE Gruppe (ein fsync, nicht
    /// 1501). Gegen a356460 ROT (die Lücke und der Verlust der Zeilen ab 1025).
    func testS4MoreThanAThousandPendingLinesAreAllKeptAndAllStandInEntryOrderAfterTheStoreHeals() async throws {
        // Eine Platte, die alles ablehnt, solange `failing` gilt: Zeilen „Inhalt“ UND die Zeile „nicht wirksam“.
        let failing = Flag()
        let t = try noSession(rejects: { _ in failing.value })
        let audit = t.audit
        failing.value = true
        let changes = 1_500
        for _ in 0..<changes { t.watch.around {} }
        await audit.barrier()
        XCTAssertEqual(t.store.clock.pendingCount, changes, "alle 1500 stehen im Register, keine ging verloren")
        t.storeFS.fail(.rename)
        _ = await t.store.updateAreas([.books], cloud: false, audit: audit)
        let refused = await t.store.updateAreas([.books, .files], cloud: false, audit: audit)
        XCTAssertFalse(refused.applied)
        XCTAssertEqual(t.store.clock.pendingCount, changes + 2, "dazu die Zeile der Blockade und die Zeile „in diesem Lauf nicht wirksam“")
        let expected = t.store.clock.pending
        let expectedGenerations = expected.prefix(changes).compactMap { $0.line.gen }
        XCTAssertEqual(expectedGenerations.count, changes)
        XCTAssertEqual(expectedGenerations, expectedGenerations.sorted(), "Reihenfolge des Eintrags = Reihenfolge der Generationen")
        await t.store.clock.awaitCatchUp()
        t.storeFS.heal()
        failing.value = false
        let syncsBefore = audit.syncCount
        t.watch.around {}
        let trigger = t.store.clock.generation
        await audit.barrier()
        await t.store.clock.awaitCatchUp()
        XCTAssertEqual(t.store.clock.pendingCount, 0, "das Register ist leer")
        let rows = lines()
        let content = rows.enumerated().filter { $0.element["t"] as? String == "release" && $0.element["cause"] as? String == "content" }
        let standing = content.compactMap { $0.element["gen"] as? Int }.filter { $0 != trigger }
        XCTAssertEqual(standing, expectedGenerations, "alle 1500 stehen, in der Reihenfolge des Eintrags")
        XCTAssertTrue(content.contains { $0.element["gen"] as? Int == trigger }, "und die Zeile der letzten Änderung")
        let ineffective = rows.enumerated().filter { $0.element["t"] as? String == "ineffective" }
        XCTAssertEqual(ineffective.count, 1)
        if let lastContent = content.filter({ $0.element["gen"] as? Int != trigger }).last?.offset, let at = ineffective.first?.offset {
            XCTAssertGreaterThan(at, lastContent, "die Zeile „nicht wirksam“ kam nach den Zeilen „Inhalt“ in das Register")
        }
        XCTAssertLessThanOrEqual(audit.syncCount - syncsBefore, 3, "eine Gruppe, nicht ein fsync je Zeile (\(audit.syncCount - syncsBefore))")
    }

    /// Nebenläufigkeit (Fehlerfall „zwei Änderer zugleich“, Sperrordnung): Vier Threads melden zugleich Änderungen der Quellen
    /// (jede reiht ihre Zeile UNTER dem Lock der Ordnung ein), während die Platte jede dritte Zeile ablehnt und die Rückrufe der
    /// Schreibwarteschlange Läufe des Nachholens starten. Nichts hängt (der Test endet), keine Zeile geht verloren und keine steht
    /// doppelt: nach dem letzten Erfolg stehen alle Generationen genau einmal in der Datei, das Register ist leer.
    func testS3ConcurrentChangesWhileLinesFailNeverDeadlockLoseALineOrWriteOneTwice() async throws {
        let writes = Counter()
        let flaky = Flag()
        flaky.value = true
        let t = try noSession(rejects: { _ in
            writes.bump()
            return flaky.value && writes.value % 3 == 0
        })
        let audit = t.audit
        let threads = 4, each = 60
        let group = DispatchGroup()
        for _ in 0..<threads {
            group.enter()
            Thread {
                for _ in 0..<each { t.watch.around {} }
                group.leave()
            }.start()
        }
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in group.notify(queue: .global()) { c.resume() } }
        await audit.barrier()
        await t.store.clock.awaitCatchUp()
        flaky.value = false
        t.watch.around {}
        await audit.barrier()
        await t.store.clock.awaitCatchUp()
        let total = threads * each + 1
        let generations = lines(["release"]).filter { $0["cause"] as? String == "content" }.compactMap { $0["gen"] as? Int }
        XCTAssertEqual(generations.count, total, "jede Änderung hat ihre Zeile")
        XCTAssertEqual(Set(generations).count, total, "keine steht doppelt")
        XCTAssertEqual(t.store.clock.pendingCount, 0, "das Register ist leer")
    }


    // MARK: Runde 10: ein Lebenslauf der Anfrage (offen → geschlossen → abgeschlossen)

    /// Aufbau für T1 und T2: Eine Anfrage `r1` mit zwei Zitaten (vier Einheiten); der Erzeuger hängt bei der dritten Einheit zwischen dem
    /// Eintrag im Index und dem Senden (Haltepunkt `ProducerHold`, der Abbruch des Laufs wirkt nicht). Die Frist des Abschlusses ist null: Der
    /// Abschluss lässt den hängenden Lauf sofort zurück.
    private struct HeldRun {
        let session: AssistantSession
        let audit: AssistantAuditLog
        let hold: ProducerHold
        let failAudio: Flag
        let generation: Int
        let events: Task<[AssistantEvent], Never>
    }

    private func heldRun(holdAt visit: Int = 3, point: AssistantRequestPoint = .beforeSend) async -> HeldRun {
        let failAudio = Flag()
        let audit = logFailingAudioResults(failAudio)
        let hold = ProducerHold()
        let index = AssistantStepIndex()
        index.setProbe { id, reached in
            guard id == "r1", reached == point else { return }
            await hold.visit(holdAt: visit)
        }
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks.joined(separator: " ")), knowledge: await twoBlockKnowledge(),
                        stepIndex: index, commitWait: .zero)
        let g = s.releaseGeneration
        let events = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: "Was macht Quasselbert in Pappelhausen mit der Tante Lavendula?")) {
                out.append(e)
            }
            return out
        }
        return HeldRun(session: s, audit: audit, hold: hold, failAudio: failAudio, generation: g, events: events)
    }

    /// Macht die Audio-Ergebniszeile der ersten Einheit zu einem klebrigen Schreibfehler der Anfrage `r1`.
    private func breakAudioResult(_ run: HeldRun) async {
        run.failAudio.value = true
        run.session.noteCommit(requestId: "r1", index: 0, stage: .audio, ok: false, generation: run.generation, failed: true)
        await run.audit.barrier()
        run.failAudio.value = false
    }

    private func rows(of request: String) -> [[String: Any]] { lines().filter { $0["req"] as? String == request } }

    /// T1 (Codex 1): Der Lauf hängt, `commit(R)` wird zweimal nacheinander gerufen. Der erste läuft in die Frist und schreibt den
    /// Fehlerabschluss (der Anspruch ist danach frei); der zweite fand R noch in `active` und fragte `hasFailed` ohne Anspruch: Er
    /// schrieb Beitrag, `h+` und eine zweite Schlusszeile. Jetzt gehört der Abschluss dem ersten Aufrufer (L1): der zweite kehrt sofort
    /// zurück und schreibt nichts. Das späte Ende des Laufs nach dem Abschluss schreibt nichts und sendet nichts mit Inhalt. Gegen
    /// aec52bde ROT (nur mit der Test-Naht gemessen).
    func testT1ASecondCommitOfAHangingRunWritesNothingTheCompletionBelongsToTheFirstCaller() async throws {
        let run = await heldRun()
        await run.hold.awaitReached()
        await breakAudioResult(run)
        XCTAssertTrue(run.audit.hasFailed(request: "r1"), "der klebrige Fehler steht")
        let first = await run.session.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        XCTAssertFalse(first, "das Protokoll ist nicht in Ordnung")
        XCTAssertEqual(run.audit.claimCount, 0, "der Anspruch ist nach dem Abschluss frei")
        let second = await run.session.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        XCTAssertTrue(second, "der zweite Aufruf hat nichts zu schreiben")
        await run.audit.barrier()
        let afterSecond = rows(of: "r1")
        XCTAssertEqual(afterSecond.filter { $0["t"] as? String == "end" }.count, 1, "genau eine Schlusszeile")
        XCTAssertEqual(afterSecond.first { $0["t"] as? String == "end" }?["outcome"] as? String, "failed", "Ausgang „Fehler“")
        let marks = afterSecond.compactMap { $0["mark"] as? String }
        XCTAssertFalse(marks.contains { $0.hasPrefix("h+") }, "kein Beitrag, keine Marke „Verlauf ausgeführt“: \(marks)")
        let historyAfter = await run.session.history
        XCTAssertEqual(historyAfter.count, 0, "kein Beitrag")
        // Das späte Ende des Laufs (`finishedWhileRunning`): keine Zeile, kein Ereignis mit Inhalt.
        run.hold.release()
        let events = await run.events.value
        await run.audit.barrier()
        XCTAssertEqual(events.filter { if case .sentence = $0 { true } else { false } }.count, 2, "nur die Einheiten vor dem Abschluss")
        XCTAssertFalse(events.contains { if case .done = $0 { true } else { false } }, "kein `done` nach dem Abschluss")
        let third = await run.session.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        XCTAssertTrue(third)
        await run.audit.barrier()
        let kinds = rows(of: "r1").compactMap { $0["t"] as? String }
        XCTAssertEqual(kinds.filter { $0 == "end" }.count, 1, "auch nach dem Ende des Laufs bleibt es bei einer Schlusszeile: \(kinds)")
        XCTAssertEqual(kinds.last, "end", "hinter der Schlusszeile steht nichts: \(kinds)")
    }

    /// T1 (Rahmen): Zwei Aufrufe zugleich, der Abschluss gehört genau einem; und der Eintrag im Lebenslauf (`claimCompletion`) vergibt
    /// den Besitz für eine Kennung genau einmal, auch unter Last mehrerer Threads.
    func testT1ClaimingTheCompletionSucceedsExactlyOnceEvenWhenCalledConcurrently() async {
        let index = AssistantStepIndex()
        let winners = await withTaskGroup(of: Bool.self) { group -> Int in
            for _ in 0..<64 { group.addTask { index.claimCompletion("r1") } }
            var n = 0
            for await won in group where won { n += 1 }
            return n
        }
        XCTAssertEqual(winners, 1, "genau ein Aufrufer bekommt den Abschluss")
        XCTAssertFalse(index.isOpen("r1"), "mit dem Besitz ist die Anfrage geschlossen")
        XCTAssertFalse(index.claimCompletion("r1"), "später bekommt ihn niemand mehr")
        XCTAssertTrue(index.isOpen("r2"), "andere Kennungen bleiben offen")
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        _ = await runToEnd(s, "r3")
        async let a = s.commit(requestId: "r3", user: "Wohin?", spoken: .all, interrupted: false)
        async let b = s.commit(requestId: "r3", user: "Wohin?", spoken: .all, interrupted: false)
        _ = await (a, b)
        await audit.barrier()
        XCTAssertEqual(rows(of: "r3").filter { $0["t"] as? String == "end" }.count, 1, "zwei Aufrufe zugleich: eine Schlusszeile")
    }

    /// Eine geschlossene Kennung läuft nie wieder: kein Anspruch, keine Zeile, kein Ereignis mit Inhalt (ein Lauf mit derselben Kennung nach
    /// dem Abschluss hätte sonst einen Anspruch angelegt, den niemand mehr beendet).
    func testAnIdThatIsAlreadyClosedNeverRunsAgainNoClaimNoLineNoContent() async {
        let audit = log()
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks[0]), knowledge: await twoBlockKnowledge())
        _ = await runToEnd(s, "r1")
        await s.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        await audit.barrier()
        let before = rows(of: "r1").count
        let claims = audit.claimCount
        let again = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?"))
        XCTAssertEqual(again.failure, .cancelled)
        XCTAssertEqual(again.events.count, 1, "nur das Ende des Stroms: \(again.events)")
        await audit.barrier()
        XCTAssertEqual(rows(of: "r1").count, before, "keine Zeile")
        XCTAssertEqual(audit.claimCount, claims, "kein neuer Anspruch")
    }

    /// T2 (Codex 2): Der Erzeuger hat den Index passiert und hält vor dem Senden; die Audio-Ergebniszeile scheitert; `commit` schließt,
    /// schreibt den Fehlerabschluss und gibt den Anspruch frei; der Erzeuger lief danach weiter, sendete eine weitere `sentence` und reihte
    /// `e+` HINTER der Schlusszeile ein. Jetzt prüft das Senden die Anfrage IM Lock („offen?“): nichts geht hinaus, keine Zeile entsteht.
    /// Gegen aec52bde ROT (nur mit der Test-Naht gemessen).
    func testT2AProducerHeldBeforeSendingSendsNothingAndEnqueuesNothingBehindTheClosingLine() async throws {
        let run = await heldRun()
        await run.hold.awaitReached()
        await breakAudioResult(run)
        let ok = await run.session.commit(requestId: "r1", user: "Wohin?", spoken: .all, interrupted: false)
        XCTAssertFalse(ok)
        await run.audit.barrier()
        let atClose = rows(of: "r1")
        XCTAssertEqual(atClose.last?["t"] as? String, "end", "die Schlusszeile ist die letzte")
        run.hold.release()
        let events = await run.events.value
        await run.audit.barrier()
        let sentences = events.compactMap { e -> String? in if case .sentence(_, _, let text, _, _, _, _) = e { text } else { nil } }
        XCTAssertEqual(sentences.count, 2, "keine weitere Einheit nach dem Abschluss: \(sentences)")
        let after = rows(of: "r1")
        XCTAssertEqual(after.count, atClose.count, "keine Zeile der Anfrage entstand nach der Schlusszeile")
        XCTAssertEqual(after.last?["t"] as? String, "end")
        let emitMarks = after.filter { $0["t"] as? String == "commit" && ($0["mark"] as? String)?.hasPrefix("e") == true }
        XCTAssertEqual(emitMarks.count, 2, "nur die Marken der beiden gesendeten Einheiten, keine für die dritte")
    }

    /// T2, Zusatz: Der Erzeuger hängt VOR der Zeile `round`; der Abschluss läuft durch; der Erzeuger läuft weiter. Es entsteht keine Zeile
    /// hinter der Schlusszeile und kein Modellaufruf (der Lauf endet, ohne die Rundenzeile einzureihen). Gegen aec52bde ROT (die Zeile
    /// `round` stand hinter `end`; nur mit der Test-Naht gemessen).
    func testT2AProducerHeldBeforeTheRoundLineNeitherWritesItNorCallsTheModelAfterTheCompletion() async throws {
        let failAudio = Flag()
        let audit = logFailingAudioResults(failAudio)
        let hold = ProducerHold()
        let index = AssistantStepIndex()
        index.setProbe { id, point in
            guard id == "r1", point == .beforeRound else { return }
            await hold.visit(holdAt: 1)
        }
        let model = CountingModel()
        let s = session(nil, audit: audit, model: model, knowledge: await twoBlockKnowledge(), stepIndex: index, commitWait: .zero)
        let events = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: "Wohin wandert Quasselbert mit dem Fahrrad nach Pappelhausen?")) {
                out.append(e)
            }
            return out
        }
        await hold.awaitReached()
        let ok = await s.commit(requestId: "r1", user: "", spoken: .nothing, interrupted: true)
        XCTAssertTrue(ok, "ein Abschluss ohne Fehler")
        await audit.barrier()
        let atClose = rows(of: "r1")
        XCTAssertEqual(atClose.compactMap { $0["t"] as? String }, ["start", "end"], "der Lauf war bei `start` stehen geblieben")
        hold.release()
        let received = await events.value
        await audit.barrier()
        let after = rows(of: "r1")
        XCTAssertEqual(after.compactMap { $0["t"] as? String }, ["start", "end"], "nach der Schlusszeile entsteht nichts: \(after)")
        XCTAssertEqual(model.calls.value, 0, "kein Modellaufruf")
        XCTAssertFalse(received.contains { if case .sentence = $0 { true } else { false } })
        XCTAssertFalse(received.contains { if case .done = $0 { true } else { false } })
    }

    /// T3 (Codex 3): Ein früher Abbruch legte `end.outcome = interrupted` fest, bevor die Schreibfehler abgeholt waren: Scheiterte nur
    /// `start` und gelang `end`, meldete die Steuerung `.auditUnavailable`, das Journal aber „interrupted“, und die Schlusszeile hatte
    /// keine Startzeile (der Leser kann sie keiner Anfrage zuordnen). Jetzt läuft der Abbruch durch denselben Abschlusscode (L3): `start`
    /// zuerst und abgewartet; scheitert sie, `start` und `end` (Ausgang Fehler, `lost`) als Gruppe (kein `end` nach einem fehlgeschlagenen `start`). Journal und Steuerung
    /// stimmen überein. Gegen aec52bde ROT.
    func testT3AnEarlyCancelWhoseFirstStartLineFailsEndsWithOutcomeFailedAndLostAndTheControlAgrees() async {
        let failures = FailFirst(1)
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"start\""), failures.shouldFail() { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(nil, audit: audit)
        XCTAssertEqual(audit.claimCount, 1, "vor der ersten Sicherung hält nur die Standzeile ihren Anspruch")
        await s.cancel(requestId: "r1")
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .auditUnavailable, "die Steuerung erfährt: Protokoll nicht schreibbar")
        await audit.barrier()
        let mine = rows(of: "r1")
        XCTAssertEqual(mine.compactMap { $0["t"] as? String }, ["start", "end"], "Anfang und Ende, in dieser Reihenfolge")
        let end = mine.last
        XCTAssertEqual(end?["outcome"] as? String, "failed", "derselbe Ausgang wie bei der Steuerung")
        XCTAssertEqual(end?["lost"] as? Bool, true, "die erste Startzeile fehlt: unbekannt")
        XCTAssertFalse(lines().contains { $0["outcome"] as? String == "interrupted" }, "nie „abgebrochen“ im Journal bei `.auditUnavailable` an der Steuerung")
        XCTAssertEqual(audit.claimCount, 0, "der Anspruch endet erst danach und ist dann frei (auch der der Standzeile)")
    }

    /// T3, Zusatz: Scheitern beide Versuche der Startzeile, bleibt auch die Schlusszeile aus (kein `end` nach einem fehlgeschlagenen `start`): keine verwaiste `end` ohne
    /// `start`. Die Platte lehnt genau die ersten beiden Startzeilen ab und nimmt danach alles an; ohne die Abhängigkeit stünde die
    /// Schlusszeile allein in der Datei. Gegen aec52bde ROT.
    func testT3WhenBothStartAttemptsFailThereIsNoOrphanEndWithoutItsStart() async {
        let failures = FailFirst(2)
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"start\""), failures.shouldFail() { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(nil, audit: audit)
        await s.cancel(requestId: "r1")
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .auditUnavailable)
        await audit.barrier()
        XCTAssertTrue(rows(of: "r1").isEmpty, "weder Start noch Ende: \(rows(of: "r1"))")
        XCTAssertEqual(audit.claimCount, 0, "kein Anspruch bleibt liegen")
    }

    /// T3, Rahmen: Ein gesunder früher Abbruch endet unverändert mit `start`, `end` (abgebrochen) und der Meldung „abgebrochen“.
    func testT3AHealthyEarlyCancelStillWritesStartThenEndInterruptedAndReportsCancelled() async {
        let audit = log()
        let s = session(nil, audit: audit)
        await s.cancel(requestId: "r1")
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .cancelled)
        await audit.barrier()
        let mine = rows(of: "r1")
        XCTAssertEqual(mine.compactMap { $0["t"] as? String }, ["start", "end"])
        XCTAssertEqual(mine.last?["outcome"] as? String, "interrupted")
        XCTAssertNil(mine.last?["lost"])
        XCTAssertEqual(audit.claimCount, 0, "kein Anspruch bleibt liegen (die Standzeile ist gesichert, ihr Anspruch beendet)")
    }


    /// Suchlauf (d): Der Abschluss überholt den Lauf (die Steuerung ruft `commit`, bevor die Anfrage in der Sitzung angekommen ist). Er
    /// schreibt nichts und schließt nichts; der Lauf, der danach ankommt, endet als Abbruch vor dem Start und bekommt genau einen Eintrag
    /// (`start`, `end` mit „abgebrochen“), ohne Modell. Ohne das hätte die Anfrage keinen Protokolleintrag.
    func testACommitThatOvertakesItsRunWritesNothingAndTheLateRunGetsItsOneEntryWithoutAModel() async {
        let audit = log()
        let model = CountingModel()
        let s = session(nil, audit: audit, model: model)
        let ok = await s.commit(requestId: "r1", user: "Hallo", spoken: .nothing, interrupted: true)
        XCTAssertTrue(ok)
        await audit.barrier()
        XCTAssertEqual(rows(of: "r1").count, 0, "der Abschluss einer unbekannten Anfrage schreibt nichts")
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .cancelled)
        XCTAssertEqual(model.calls.value, 0, "kein Modell")
        await audit.barrier()
        let mine = rows(of: "r1")
        XCTAssertEqual(mine.compactMap { $0["t"] as? String }, ["start", "end"], "genau ein Eintrag")
        XCTAssertEqual(mine.last?["outcome"] as? String, "interrupted")
        XCTAssertEqual(audit.claimCount, 0, "kein Anspruch bleibt liegen")
        // Ein weiterer Lauf derselben Kennung wird abgewiesen: keine Zeile mehr.
        let again = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(again.failure, .cancelled)
        await audit.barrier()
        XCTAssertEqual(rows(of: "r1").count, 2)
        XCTAssertEqual(model.calls.value, 0)
    }


    /// T3, Rückgabewert (Suchlauf b): Endete der Lauf, weil das Protokoll nicht schrieb (die Standzeile scheiterte, es gibt keinen Fehler der
    /// Anfrage selbst), steht in der Schlusszeile „Fehler“, und `commit` meldet es auch (früher: `true`, obwohl das Journal „Fehler“
    /// sagte). Gegen aec52bde ROT.
    func testT3CommitReportsFailureWhenTheRunEndedBecauseTheProtocolWasNotWritable() async {
        let failures = FailFirst(1)
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\""), failures.shouldFail() { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let model = CountingModel()
        let s = session(nil, audit: audit, model: model)
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .auditUnavailable, "die Steuerung erfährt: Protokoll nicht schreibbar")
        XCTAssertEqual(model.calls.value, 0, "kein Modell")
        let ok = await s.commit(requestId: "r1", user: "Hallo", spoken: .nothing, interrupted: false)
        XCTAssertFalse(ok, "derselbe Ausgang wie in der Schlusszeile")
        await audit.barrier()
        let mine = rows(of: "r1")
        XCTAssertEqual(mine.last?["t"] as? String, "end")
        XCTAssertEqual(mine.last?["outcome"] as? String, "failed")
        XCTAssertEqual(mine.first?["t"] as? String, "start", "die Schlusszeile hat ihren Anfang")
    }

    // MARK: Runde 11, V1: Eine Kennung hat im Protokoll genau EINEN Eintrag, gleich wie oft ihre Startzeile in der Datei steht

    private func entries(of request: String, in audit: AssistantAuditLog) async -> [AssistantAuditEntry] {
        await audit.entries().filter { $0.requestId == request }
    }

    private func kinds(of request: String) -> [String] { rows(of: request).compactMap { $0["t"] as? String } }

    /// V1 (A, Codex): Früher Abbruch; die erste Startzeile wird vollständig angehängt, ihr fsync scheitert; die Gruppe `start` + `end`
    /// (Fehler, `lost`) gelingt. Die Datei enthält wirklich `start, start, end`. Der Leser zeigte für dieselbe Anfrage erst
    /// „unvollständig“, dann „Fehler“ (zwei Einträge). Jetzt: genau ein Eintrag, Ausgang Fehler; das Wiederöffnen fügt nichts hinzu (das
    /// vorhandene `end` schließt die Kennung). Gegen cd111e2a ROT (zwei Einträge).
    func testV1AFirstStartWhoseFsyncFailsAfterAppendingLeavesTwoStartLinesAndTheReaderShowsOneEntryWithOutcomeFailed() async throws {
        let fs = CxFS("p")
        let audit = log(fileSystem: fs)
        let s = session(nil, audit: audit)
        await audit.barrier()                      // die Standzeile steht dauerhaft
        fs.fail(.syncFile, times: 1)               // das fsync der nächsten Gruppe scheitert, NACHDEM sie angehängt wurde
        await s.cancel(requestId: "r1")
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .auditUnavailable, "die Steuerung erfährt: Protokoll nicht schreibbar")
        await audit.barrier()
        XCTAssertEqual(kinds(of: "r1"), ["start", "start", "end"], "die Wiederholung steht wirklich in der Datei (Doppel statt Verlust)")
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "failed")
        XCTAssertEqual(rows(of: "r1").last?["lost"] as? Bool, true)
        let read = await audit.read()
        let mine = read.entries.filter { $0.requestId == "r1" }
        XCTAssertEqual(mine.count, 1, "genau ein Eintrag für die Kennung")
        XCTAssertEqual(mine.first?.outcome, .failed)
        XCTAssertEqual(read.skippedLines, 0, "die Wiederholung ist lesbar, nicht übersprungen")
        // Wiederöffnen (neuer Schreiber auf derselben Datei): kein Nachtrag, kein zweiter Eintrag.
        let reopened = log()
        reopened.enqueue(journalStart("zz"))
        await reopened.barrier()
        XCTAssertEqual(kinds(of: "r1"), ["start", "start", "end"], "das Öffnen fügt der Kennung nichts hinzu")
        let again = await entries(of: "r1", in: reopened)
        XCTAssertEqual(again.count, 1)
        XCTAssertEqual(again.first?.outcome, .failed)
    }

    /// V1 (B, Codex, ohne Schreibfehler): Der Erzeuger hat `start` eingereiht und wartet (die Zeile ist beim Schreiber unterwegs); der
    /// Abschluss überschreitet die Frist, las `journaled == false` und schrieb einen Ersatzstart; der ursprüngliche Start wurde danach
    /// doch bestätigt: zwei Startzeilen, zwei Einträge. Jetzt erkennt der Abschluss, dass der Erzeuger seinen Start schon eingereiht hat,
    /// und wartet auf DIESE Zeile: genau eine Startzeile, genau ein Eintrag mit dem Ausgang der Schlusszeile. Gegen cd111e2a ROT.
    func testV1BTheCompletionOfAHangingProducerWaitsForItsStartLineInsteadOfWritingASecondOne() async throws {
        let writer = WriterHold()
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"start\""), writer.isFirst() {
                writer.entered.signal()
                writer.release.wait()
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let closingHold = ProducerHold()
        let index = AssistantStepIndex()
        index.setProbe { id, point in
            guard id == "r1", point == .closing else { return }
            await closingHold.visit(holdAt: 1)
        }
        let s = session(nil, audit: audit, model: EchoModel(text: twoBlocks.joined(separator: " ")), knowledge: await twoBlockKnowledge(),
                        stepIndex: index, commitWait: .zero)
        let events = Task { await drainEvents(s, AssistantRequest(requestId: "r1", text: "Was macht Quasselbert in Pappelhausen mit der Tante Lavendula?")) }
        await awaitSignal(writer.entered)           // `start` des Erzeugers ist eingereiht, die Zeile ist beim Schreiber unterwegs
        let completion = Task { await s.commit(requestId: "r1", user: "", spoken: .nothing, interrupted: true) }
        await closingHold.awaitReached()            // der Abschluss hat die Frist verlassen und seinen Stand gelesen (der Start ist noch offen)
        writer.release.signal()                     // der ursprüngliche Start wird jetzt bestätigt
        closingHold.release()
        let ok = await completion.value
        _ = await events.value
        await audit.barrier()
        XCTAssertTrue(ok, "ein Abschluss ohne Fehler")
        XCTAssertEqual(kinds(of: "r1").filter { $0 == "start" }.count, 1, "keine zweite Startzeile: \(kinds(of: "r1"))")
        XCTAssertEqual(kinds(of: "r1").last, "end")
        let mine = await entries(of: "r1", in: audit)
        XCTAssertEqual(mine.count, 1, "genau ein Eintrag")
        XCTAssertEqual(mine.first?.outcome, .interrupted, "der Ausgang der Schlusszeile")
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "interrupted")
    }

    /// V1 (C): Zwei Starts ohne `end` und ein Wiederöffnen: genau ein Eintrag „unvollständig“ und genau EIN Nachtrag (nicht einer je
    /// Startzeile). Gegen cd111e2a ROT nur im Leser (zwei Einträge); der Nachtrag war schon einmalig.
    func testV1CTwoStartLinesWithoutAnEndAreOneIncompleteEntryAndTheReopeningAddsExactlyOneEnd() async throws {
        try writeJournal([journalStart("r1"), journalStep("r1", 1, .grounded), journalStart("r1")])
        let reopened = log()
        reopened.enqueue(journalStart("zz"))
        await reopened.barrier()
        XCTAssertEqual(kinds(of: "r1"), ["start", "step", "start", "end"], "genau ein Nachtrag")
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "incomplete")
        let mine = await entries(of: "r1", in: reopened)
        XCTAssertEqual(mine.count, 1, "genau ein Eintrag")
        XCTAssertEqual(mine.first?.outcome, .incomplete)
        XCTAssertEqual(mine.first?.trace.count, 1, "der Schritt gehört zum einen Eintrag")
        // Ein zweites Öffnen ändert nichts mehr.
        let third = log()
        third.enqueue(journalStart("zy"))
        await third.barrier()
        XCTAssertEqual(kinds(of: "r1"), ["start", "step", "start", "end"])
    }

    /// V1, Suchlauf (a): Jede Zeilenart einer Anfrage, doppelt in der Datei, ergibt denselben einen Eintrag, nichts wird doppelt gezählt,
    /// und die Felder der ERSTEN Startzeile bleiben. Gegen cd111e2a ROT (zwei Einträge, Spur und Zähler doppelt).
    func testV1TheReaderFoldsEveryDuplicatedLineOfARequestIntoTheOneEntryAndCountsNothingTwice() async throws {
        var secondStart = journalStart("r1")
        secondStart.gen = 9
        var round = AssistantJournalLine(.round, req: "r1")
        round.round = .data
        round.reason = .locked
        var areas = AssistantJournalLine(.areas, req: "r1")
        areas.areas = ["books"]
        try writeJournal([
            journalStart("r1"), secondStart,
            journalStep("r1", 1, .grounded), journalStep("r1", 1, .grounded), journalStep("r1", 2, .unsourced),
            journalCommit("r1", 1, .emit), journalCommit("r1", 1, .emit), journalCommit("r1", 0, .history), journalCommit("r1", 0, .history),
            round, round, areas, areas,
            journalEnd("r1"), journalEnd("r1"),
        ])
        let read = await log().read()
        XCTAssertEqual(read.skippedLines, 0, "Doppelzeilen sind lesbar")
        let mine = read.entries.filter { $0.requestId == "r1" }
        XCTAssertEqual(mine.count, 1, "genau ein Eintrag")
        let entry = try XCTUnwrap(mine.first)
        XCTAssertEqual(entry.generation, 1, "die Felder der ersten Startzeile bleiben")
        XCTAssertEqual(entry.trace.map(\.n), [1, 2], "jede Schrittnummer einmal")
        XCTAssertEqual(entry.trace.first?.commits.count, 1, "dieselbe Marke einmal")
        XCTAssertEqual(entry.counts.grounded, 1, "nichts doppelt gezählt")
        XCTAssertEqual(entry.counts.unsourced, 1)
        XCTAssertEqual(entry.outcome, .completed)
        XCTAssertEqual(entry.areas, ["books"])
    }

    /// V1, Verdichtung: Eine Kennung mit zwei Startzeilen ist EINE Einheit (nicht zwei): Bei genau `maxEntries` Anfragen, von denen die
    /// älteste doppelt gestartet wurde, wird nichts verworfen. (Wächter: bei der Einheit je Kennung schon vorher grün.)
    func testV1ADoubledStartIsOneUnitForTheCompactionAndNoRequestIsDroppedAtTheLimit() async throws {
        var journal: [AssistantJournalLine] = [journalStart("q0"), journalStart("q0"), journalEnd("q0")]
        for i in 1..<AssistantAuditLog.maxEntries { journal.append(contentsOf: [journalStart("q\(i)"), journalEnd("q\(i)")]) }
        try writeJournal(journal)
        let reopened = log()
        reopened.enqueue(journalStart("zz"))
        await reopened.barrier()
        XCTAssertEqual(kinds(of: "q0"), ["start", "start", "end"], "die älteste Kennung bleibt vollständig")
        let all = await reopened.entries()
        XCTAssertEqual(all.filter { $0.requestId != nil }.count, AssistantAuditLog.maxEntries + 1, "alle Anfragen plus die neue")
    }

    // MARK: Runde 11, V2: Der Abschluss entscheidet aus den Fehlerquellen selbst, zum Zeitpunkt der Entscheidung

    /// Der Ablauf von Codex: R hängt in `stateLine.ensure` (die Standzeile wird beim Schreiber gehalten). `commit` schließt R, verlässt die
    /// Frist (`commitWait` null) und liest den Stand: kein Fehler. Danach scheitert die Standzeile (auch jeder weitere Versuch); der
    /// Start des Abschlusses gelingt. Der Fehler der Standzeile hängt nicht am Anspruch der Anfrage: Der Abschluss lieferte
    /// „unterbrochen“ und `true`. Jetzt sichert er die Standzeile selbst (`ensure`), bevor er `start` schreibt, und liest ihr Ergebnis
    /// bei der Entscheidung: Schlusszeile „Fehler“, `lost`, `commit` meldet `false`. Gegen cd111e2a ROT.
    func testV2AStateLineThatFailsAfterTheCompletionReadItsStateMakesTheOutcomeFailedAndCommitFalse() async throws {
        let writer = WriterHold()
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\"") {
                if writer.isFirst() {
                    writer.entered.signal()
                    writer.release.wait()
                }
                throw Injected()
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let producerHold = ProducerHold()
        let closingHold = ProducerHold()
        let index = AssistantStepIndex()
        index.setProbe { id, point in
            guard id == "r1" else { return }
            if point == .beforeState { await producerHold.visit(holdAt: 1) }
            if point == .closing { await closingHold.visit(holdAt: 1) }
        }
        let model = CountingModel()
        let s = session(nil, audit: audit, model: model, stepIndex: index, commitWait: .zero)
        let events = Task { await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo")) }
        await producerHold.awaitReached()
        producerHold.release()                      // der Erzeuger geht in `ensure` und wartet auf die gehaltene Standzeile
        await awaitSignal(writer.entered)
        let completion = Task { await s.commit(requestId: "r1", user: "", spoken: .nothing, interrupted: true) }
        await closingHold.awaitReached()            // der Abschluss hat den Stand gelesen: kein Fehler
        writer.release.signal()                     // jetzt scheitert die Standzeile
        closingHold.release()
        let ok = await completion.value
        let received = await events.value
        await audit.barrier()
        XCTAssertFalse(ok, "die Standzeile steht nicht: Protokoll nicht schreibbar")
        XCTAssertEqual(model.calls.value, 0, "kein Modell")
        XCTAssertEqual(kinds(of: "r1"), ["start", "end"])
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "failed", "Schlusszeile „Fehler“")
        XCTAssertEqual(rows(of: "r1").last?["lost"] as? Bool, true)
        XCTAssertFalse(received.events.contains { if case .sentence = $0 { true } else { false } })
    }

    /// V2, Zusatz: Gelingt die Standzeile spät (nach dem Lesen des Standes durch den Abschluss), bleibt es beim Ausgang wie bisher:
    /// „unterbrochen“, `commit` meldet `true`, die Standzeile steht in der Datei vor der Anfrage.
    func testV2AStateLineThatSucceedsLateLeavesTheOutcomeAsBefore() async throws {
        let writer = WriterHold()
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\""), writer.isFirst() {
                writer.entered.signal()
                writer.release.wait()
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let producerHold = ProducerHold()
        let closingHold = ProducerHold()
        let index = AssistantStepIndex()
        index.setProbe { id, point in
            guard id == "r1" else { return }
            if point == .beforeState { await producerHold.visit(holdAt: 1) }
            if point == .closing { await closingHold.visit(holdAt: 1) }
        }
        let model = CountingModel()
        let s = session(nil, audit: audit, model: model, stepIndex: index, commitWait: .zero)
        let events = Task { await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo")) }
        await producerHold.awaitReached()
        producerHold.release()
        await awaitSignal(writer.entered)
        let completion = Task { await s.commit(requestId: "r1", user: "", spoken: .nothing, interrupted: true) }
        await closingHold.awaitReached()
        writer.release.signal()                     // die Standzeile gelingt, nachdem der Abschluss seinen Stand las
        closingHold.release()
        let ok = await completion.value
        _ = await events.value
        await audit.barrier()
        XCTAssertTrue(ok, "ohne Fehler")
        XCTAssertEqual(model.calls.value, 0)
        XCTAssertEqual(kinds(of: "r1"), ["start", "end"])
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "interrupted")
        XCTAssertNil(rows(of: "r1").last?["lost"])
        XCTAssertEqual(lines(["state"]).count, 1, "die Standzeile steht in der Datei")
    }

    /// V2, früher Abbruch: Auch ein Abschluss, dessen Lauf nie ankam, entscheidet aus der Standzeile: Scheitert sie, melden Schlusszeile
    /// und Steuerung „Fehler“ (früher „abgebrochen“, obwohl keine gültige Standzeile vor dem Eintrag steht). Gegen cd111e2a ROT.
    func testV2AnEarlyCancelWhoseStateLineCannotBeWrittenEndsWithOutcomeFailedAndTheControlAgrees() async {
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\"") { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(nil, audit: audit)
        await s.cancel(requestId: "r1")
        let result = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(result.failure, .auditUnavailable, "die Steuerung erfährt: Protokoll nicht schreibbar")
        await audit.barrier()
        XCTAssertEqual(kinds(of: "r1"), ["start", "end"])
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "failed")
        XCTAssertEqual(rows(of: "r1").last?["lost"] as? Bool, true)
    }

    // MARK: Runde 12, W1: Der Fehler der Standzeile hängt an der Anfrage, bevor der gescheiterte Versuch zurückgegeben wird

    /// Der Ablauf von Codex: Die Standzeile scheitert EINMAL. `ensure` meldet es dem Erzeuger von R und gibt den Versuch frei; die
    /// Fortsetzung von `run`, die den Fehler in den Stand des Laufs übernimmt, läuft noch nicht (der Erzeuger steht am Haltepunkt). Der
    /// Abschluss verlässt die Frist (`commitWait` null), sein eigenes `ensure` ist ein neuer Versuch und gelingt, `start` gelingt, der
    /// Stand des Laufs zeigt noch keinen Fehler: Der Abschluss schrieb „unterbrochen“ und lieferte `true`. Jetzt steht der Fehler am
    /// Anspruch der Anfrage, bevor der Versuch freigegeben wird: Schlusszeile „Fehler“, `lost`, `commit` meldet `false`. Gegen 894ea911
    /// ROT (mit dem Haltepunkt gemessen).
    func testW1AStateLineFailureThatASuccessfulRetryFollowsStillDecidesTheOutcomeOfTheRequestThatSawIt() async throws {
        let failures = FailFirst(1)
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\""), failures.shouldFail() { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let refused = ProducerHold()
        let index = AssistantStepIndex()
        index.setProbe { id, point in
            guard id == "r1", point == .stateRefused else { return }
            await refused.visit(holdAt: 1)
        }
        let model = CountingModel()
        let s = session(nil, audit: audit, model: model, stepIndex: index, commitWait: .zero)
        let events = Task { await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo")) }
        await refused.awaitReached()                // der Versuch ist gescheitert und freigegeben; `run` hat den Fehler noch nicht
        XCTAssertTrue(audit.hasFailed(request: "r1"), "der Fehler hängt schon an der Anfrage")
        let ok = await s.commit(requestId: "r1", user: "", spoken: .nothing, interrupted: true)
        await audit.barrier()
        XCTAssertFalse(ok, "der erkannte Protokollfehler bestimmt den Ausgang")
        XCTAssertEqual(kinds(of: "r1"), ["start", "end"])
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "failed", "Schlusszeile „Fehler“")
        XCTAssertEqual(rows(of: "r1").last?["lost"] as? Bool, true)
        XCTAssertEqual(lines(["state"]).count, 1, "die Wiederholung des Abschlusses hat die Standzeile geschrieben")
        refused.release()
        let received = await events.value
        await audit.barrier()
        XCTAssertEqual(model.calls.value, 0, "kein Modell")
        XCTAssertFalse(received.events.contains { if case .sentence = $0 { true } else { false } })
        XCTAssertEqual(kinds(of: "r1"), ["start", "end"], "das späte Ende des Laufs schreibt nichts")
        XCTAssertEqual(audit.claimCount, 0, "kein Anspruch bleibt liegen")
    }

    /// W1, Gegenprobe: Der Fehler hängt nur an den Anfragen, die den gescheiterten Versuch geteilt haben. Eine Anfrage danach, deren
    /// Versuch gelingt, endet ohne Fehler; die erste endet mit „Fehler“, auch wenn sie erst nach der zweiten abgeschlossen wird.
    func testW1ARequestAfterTheFailedAttemptIsNotMarkedAndTheFirstStaysFailed() async {
        let failures = FailFirst(1)
        let audit = log(appender: { url, data in
            if String(decoding: data, as: UTF8.self).contains("\"t\":\"state\""), failures.shouldFail() { throw Injected() }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(nil, audit: audit)
        let first = await drainEvents(s, AssistantRequest(requestId: "r1", text: "Hallo"))
        XCTAssertEqual(first.failure, .auditUnavailable)
        let second = await drainEvents(s, AssistantRequest(requestId: "r2", text: "Hallo"))
        XCTAssertNil(second.failure, "der nächste Versuch gelingt")
        XCTAssertFalse(audit.hasFailed(request: "r2"))
        let secondOk = await s.commit(requestId: "r2", user: "Hallo", spoken: .nothing, interrupted: false)
        let firstOk = await s.commit(requestId: "r1", user: "Hallo", spoken: .nothing, interrupted: false)
        await audit.barrier()
        XCTAssertTrue(secondOk)
        XCTAssertFalse(firstOk)
        XCTAssertEqual(rows(of: "r2").last?["outcome"] as? String, "completed")
        XCTAssertNil(rows(of: "r2").last?["lost"])
        XCTAssertEqual(rows(of: "r1").last?["outcome"] as? String, "failed")
        XCTAssertEqual(audit.claimCount, 0)
    }

}
