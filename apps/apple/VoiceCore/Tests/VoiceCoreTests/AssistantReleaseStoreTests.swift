import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Schreibfolge 8): die Zustandsdatei der Freigaben (Klasse A) und die Sicherungsfolge der
// Sitzung. Alle Dateisystem-Schritte laufen hinter `AssistantStoreFileSystem`; die Fehlerinjektion ersetzt sie, ohne echte
// Plattenfehler. Jeder Teilschritt (Verzeichnis, Schreiben, fsync Datei, rename, fsync Verzeichnis) und jede Zeile des Protokolls
// (beabsichtigt, wirksam, release, nicht wirksam) wird einzeln scheitern gelassen; dazu Abbruch mitten im Vorgang (Prozessende),
// kaputte Dateien, Nebenläufigkeit und die Übernahme beim allerersten Start. Nur synthetische Daten.

private struct Injected: Error, Equatable { let step: String }

/// Das Dateisystem mit Fehlerinjektion: ein Teilschritt scheitert vor oder nach der Wirkung, wahlweise mit „Prozessende“ (danach
/// scheitert alles, und das Aufräumen unterbleibt, wie nach einem Absturz) oder mit halb geschriebener Datei (`torn`).
private final class FaultFS: AssistantStoreFileSystem, @unchecked Sendable {
    struct Fault {
        var after = false
        var remaining = Int.max
        var crash = false
        var torn = false
    }

    private let base = AssistantPosixStoreFileSystem()
    private let lock = NSLock()
    private var faults: [AssistantStoreStep: Fault] = [:]
    private var delays: [AssistantStoreStep: TimeInterval] = [:]
    private var crashed = false
    private var seen: [String] = []
    private var observer: (@Sendable (String) -> Void)?

    func inject(_ step: AssistantStoreStep, after: Bool = false, times: Int = .max, crash: Bool = false, torn: Bool = false) {
        lock.withLock { faults[step] = Fault(after: after, remaining: times, crash: crash, torn: torn) }
    }

    func clear() { lock.withLock { faults.removeAll() } }
    func delay(_ step: AssistantStoreStep, _ seconds: TimeInterval) { lock.withLock { delays[step] = seconds } }
    func observe(_ body: @escaping @Sendable (String) -> Void) { lock.withLock { observer = body } }
    var log: [String] { lock.withLock { seen } }
    var hasCrashed: Bool { lock.withLock { crashed } }

    private func perform(_ step: AssistantStoreStep, _ operation: () throws -> Void, partial: (() throws -> Void)? = nil) throws {
        let (isCrashed, fault, pause, watch) = lock.withLock { () -> (Bool, Fault?, TimeInterval, (@Sendable (String) -> Void)?) in
            if crashed { return (true, nil, 0, nil) }
            seen.append(step.rawValue)
            var hit: Fault?
            if var f = faults[step], f.remaining > 0 {
                f.remaining -= 1
                faults[step] = f
                hit = f
            }
            return (false, hit, delays[step] ?? 0, observer)
        }
        if isCrashed { throw Injected(step: "nach dem Prozessende") }
        watch?("fs:\(step.rawValue)")
        if pause > 0 { Thread.sleep(forTimeInterval: pause) }
        guard let fault else { return try operation() }
        if fault.torn { try? partial?() } else if fault.after { try operation() }
        if fault.crash { lock.withLock { crashed = true } }
        throw Injected(step: step.rawValue)
    }

    func read(_ url: URL) throws -> Data? { try base.read(url) }
    func exists(_ url: URL) -> Bool { base.exists(url) }
    func createDirectory(_ url: URL) throws { try perform(.prepare) { try self.base.createDirectory(url) } }

    func writeNew(_ url: URL, _ data: Data) throws {
        try perform(.write, { try self.base.writeNew(url, data) }, partial: { try self.base.writeNew(url, data.prefix(data.count / 2)) })
    }

    func syncFile(_ url: URL) throws { try perform(.syncFile) { try self.base.syncFile(url) } }
    func rename(_ from: URL, to: URL) throws { try perform(.rename) { try self.base.rename(from, to: to) } }
    func syncDirectory(_ url: URL) throws { try perform(.syncDirectory) { try self.base.syncDirectory(url) } }

    func remove(_ url: URL) {
        if lock.withLock({ crashed }) { return }
        base.remove(url)
    }
}

/// Eine gemeinsame Ereignisfolge aus Dateisystem und Protokoll, um die Reihenfolge über beide hinweg zu prüfen.
private final class Recorder: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String] = []
    func add(_ s: String) { lock.withLock { items.append(s) } }
    var all: [String] { lock.withLock { items } }
}

private struct EchoModel: BookTalkModel {
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

final class AssistantReleaseStoreTests: XCTestCase {
    private var dir: URL!
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
    private let steps: [AssistantStoreStep] = [.prepare, .write, .syncFile, .rename, .syncDirectory]

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-release-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    // MARK: Hilfen

    private var stateFile: URL { dir.appendingPathComponent(AssistantReleaseStore.fileName) }
    private var protocolFile: URL { dir.appendingPathComponent(AssistantAuditLog.fileName) }

    private func openStore(_ fs: any AssistantStoreFileSystem = AssistantPosixStoreFileSystem(), adopt: AssistantReleaseRecord? = nil)
        -> AssistantReleaseStore {
        AssistantReleaseStore(directory: dir, protocolURL: protocolFile, fileSystem: fs, adopt: adopt)
    }

    private func freshLog(appender: @escaping @Sendable (URL, Data) throws -> Void = AssistantAuditWriter.appendFile) -> AssistantAuditLog {
        AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile, appender: appender)
    }

    private func session(_ store: AssistantReleaseStore?, audit: AssistantAuditLog? = nil, knowledge: AssistantKnowledge? = nil,
                         model: any BookTalkModel = EchoModel(text: "Hallo.")) -> AssistantSession {
        AssistantSession(model: model, knowledge: knowledge ?? AssistantKnowledge(corpus: AssistantCorpus()), config: config, audit: audit,
                         released: true, releaseStore: store)
    }

    private func fileRecord() -> (record: AssistantReleaseRecord, revision: Int)? {
        guard let data = try? Data(contentsOf: stateFile) else { return nil }
        return AssistantReleaseFile.decode(data)
    }

    private func lines(_ kind: String? = nil) -> [[String: Any]] {
        ((try? String(contentsOf: protocolFile, encoding: .utf8)) ?? "").split(separator: "\n").compactMap {
            (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any]
        }.filter { kind == nil || $0["t"] as? String == kind }
    }

    private func block(_ audit: AssistantAuditLog?, failing kind: String, _ on: FailFlag) -> (@Sendable (URL, Data) throws -> Void) {
        { url, data in
            if on.value, String(decoding: data, as: UTF8.self).contains("\"t\":\"\(kind)\"") { throw DiskFull() }
            try AssistantAuditWriter.appendFile(url, data)
        }
    }

    // MARK: Start: Datei, gesperrt, allererster Start

    func testTheVeryFirstStartSecuresTheShippedStateBeforeAnythingIsUsable() {
        let store = openStore()
        XCTAssertEqual(store.opening, .firstStart)
        XCTAssertFalse(store.isLocked)
        let stored = fileRecord()
        XCTAssertEqual(stored?.record, .shipped, "erst gesichert, dann nutzbar")
        XCTAssertEqual(stored?.revision, 1)
        XCTAssertEqual(store.confirmedRevision, 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent(AssistantReleaseStore.tempName).path), "keine Temp-Datei bleibt")
        XCTAssertFalse(store.record.optIn)
        XCTAssertEqual(store.record.areas(cloud: false), Set(AssistantArea.allCases))
        XCTAssertEqual(store.record.areas(cloud: true), [.books])
    }

    func testTheFirstStartThatCannotBeSecuredStaysLockedAndAssumesNothing() async {
        for step in steps where step != .syncDirectory {
            let fs = FaultFS()
            fs.inject(step)
            let store = openStore(fs)
            XCTAssertEqual(store.opening, .locked(.replaceFailed(step)), "\(step)")
            XCTAssertTrue(store.isLocked, "\(step)")
            XCTAssertEqual(store.record, .locked, "\(step): kein Bereich, keine Zustimmung")
            let s = session(store)
            XCTAssertTrue(s.isReleaseLocked)
            XCTAssertEqual(s.effectiveAreas, [])
            XCTAssertFalse(s.effectiveCloudConsent)
            try? FileManager.default.removeItem(at: dir)
        }
    }

    func testAMissingFileWithAProtocolLocksAndAMissingFileWithoutOneIsTheFirstStart() throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("{\"t\":\"state\"}\n".utf8).write(to: protocolFile)
        let locked = openStore()
        XCTAssertEqual(locked.opening, .locked(.missing))
        XCTAssertTrue(locked.isLocked)
        XCTAssertFalse(FileManager.default.fileExists(atPath: stateFile.path), "gesperrt schreibt nichts von selbst")
        try FileManager.default.removeItem(at: protocolFile)
        XCTAssertEqual(openStore().opening, .firstStart)
    }

    /// Jede Art von kaputter Datei sperrt; das Protokoll wird nie als Quelle des Stands gelesen (auch wenn es Bereiche nennt).
    func testEveryKindOfBrokenFileLocksAndTheProtocolIsNeverTheSource() throws {
        let valid = try XCTUnwrap(AssistantReleaseFile.encode(.shipped, revision: 7))
        var flipped = valid
        let at = try XCTUnwrap(valid.firstIndex(of: UInt8(ascii: "{"))) + 5
        flipped[at] = flipped[at] == UInt8(ascii: "x") ? UInt8(ascii: "y") : UInt8(ascii: "x")
        let foreign = Data(String(decoding: valid, as: UTF8.self).replacingOccurrences(of: "LVA-FREIGABEN 1", with: "LVA-FREIGABEN 2").utf8)
        let cases: [(String, Data)] = [
            ("kaputter Prüfwert", flipped),
            ("leer", Data()),
            ("abgeschnitten", valid.prefix(valid.count / 2)),
            ("ohne Prüfzeile", Data(valid.split(separator: 10, omittingEmptySubsequences: false).prefix(2).joined(separator: [10]))),
            ("fremde Fassung", foreign),
            ("Müll", Data("kein Freigabestand".utf8)),
            ("zusätzliche Zeile", valid + Data("zusatz\n".utf8)),
        ]
        for (name, data) in cases {
            try? FileManager.default.removeItem(at: dir)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try data.write(to: stateFile)
            // Ein Protokoll, das einen großzügigen Stand behauptet, ändert nichts.
            try Data("{\"t\":\"state\",\"areas\":[\"books\",\"files\",\"notes\",\"transcripts\"],\"unsourced\":true,\"granted\":true}\n".utf8)
                .write(to: protocolFile)
            let store = openStore()
            XCTAssertEqual(store.opening, .locked(.invalid), name)
            XCTAssertTrue(store.isLocked, name)
            XCTAssertEqual(store.record, .locked, name)
            let s = session(store)
            XCTAssertTrue(s.isReleaseLocked, name)
            XCTAssertEqual(s.effectiveAreas, [], name)
            XCTAssertFalse(s.allowsUnsourced, name)
            XCTAssertEqual(try Data(contentsOf: stateFile), data, "\(name): die Datei bleibt unberührt, bis der Nutzer neu setzt")
        }
    }

    func testAnUnreadableFileLocksAsReadFailed() throws {
        try FileManager.default.createDirectory(at: stateFile, withIntermediateDirectories: true)   // ein Verzeichnis statt der Datei
        let store = openStore()
        XCTAssertEqual(store.opening, .locked(.readFailed))
        XCTAssertTrue(store.isLocked)
    }

    func testASessionOnALockedStoreSeesNothingAndTheUserCanResetItAndItIsSecuredFirst() async throws {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Notiz Eisvogel", area: .notes, sections: [(nil, "Der Eisvogel brütet am Mühlbach in der Uferböschung.")]))
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("kaputt".utf8).write(to: stateFile)
        let audit = freshLog()
        let store = openStore()
        let s = session(store, audit: audit, knowledge: AssistantKnowledge(corpus: corpus),
                        model: EchoModel(text: "Der Eisvogel brütet am Mühlbach in der Uferböschung."))
        XCTAssertTrue(s.isReleaseLocked)
        var texts: [String] = []
        for await e in s.events(AssistantRequest(requestId: "r1", text: "Wo brütet der Eisvogel?")) {
            if case .sentence(_, _, let t, _, _, _, _) = e { texts.append(t) }
        }
        XCTAssertFalse(texts.contains { $0.contains("Eisvogel") }, "gesperrt: nichts aus eigenen Inhalten: \(texts)")
        _ = await s.commit(requestId: "r1", user: "x", spoken: .nothing, interrupted: true)
        let stateLines = lines("state")
        XCTAssertEqual(stateLines.first?["locked"] as? Bool, true, "die Standzeile nennt „gesperrt“")
        XCTAssertEqual(stateLines.first?["areas"] as? [String], [])
        // Der Nutzer stellt neu ein: erst sichern, dann gilt es; danach ist nichts mehr gesperrt.
        let r = await s.setAreas([.notes])
        XCTAssertTrue(r.effective)
        XCTAssertNil(r.storeError)
        XCTAssertFalse(s.isReleaseLocked)
        XCTAssertFalse(store.isLocked)
        XCTAssertEqual(s.effectiveAreas, [.notes])
        let stored = try XCTUnwrap(fileRecord())
        XCTAssertEqual(stored.record.localAreas, ["notes"])
        XCTAssertEqual(stored.record.cloudAreas, [], "gesperrt gestartet: von der leeren Grundlage, keine alten Cloud-Bereiche")
        XCTAssertFalse(stored.record.optIn)
        XCTAssertEqual(openStore().opening, .loaded(revision: stored.revision))
    }

    // MARK: Übernahme beim allerersten Start

    func testTheOneTimeAdoptionAtTheVeryFirstStartOnlyNarrowsAndNeverGrantsAnything() {
        let old = AssistantReleaseRecord(optIn: true, consentVersion: 3, localAreas: [.books, .notes], cloudAreas: [.books, .files],
                                         consents: ["ziel": [.books]])
        let store = openStore(adopt: old)
        XCTAssertEqual(store.opening, .firstStart)
        XCTAssertFalse(store.record.optIn, "nie ein Schalter aus alten Einstellungen")
        XCTAssertEqual(store.record.consentVersion, 0)
        XCTAssertEqual(store.record.areas(cloud: false), [.books, .notes], "enger als der Auslieferungsstand")
        XCTAssertEqual(store.record.areas(cloud: true), [.books], "Schnittmenge, nie mehr als ausgeliefert")
        XCTAssertTrue(store.record.consents.isEmpty, "nie eine Zustimmung")
        // Eine vorhandene Datei gewinnt immer: keine zweite Übernahme.
        let reopened = openStore(adopt: AssistantReleaseRecord(localAreas: [], cloudAreas: []))
        XCTAssertEqual(reopened.record.areas(cloud: false), [.books, .notes])
    }

    // MARK: Einschränkung: sofort, dann Datei, dann Zeile; Fehler je Schritt

    func testARestrictionHoldsImmediatelyAndAFailedStepLeavesAnOpenOutcomeBlocksTheRunAndIsRetriedLater() async throws {
        for step in steps {
            try? FileManager.default.removeItem(at: dir)
            let fs = FaultFS()
            let store = openStore(fs)
            store.clock.setOpportunitiesPaused(true)   // Zwischenzustände der Fehlerfolge beobachten, ohne dass der Hintergrundlauf sie abräumt
            let audit = freshLog()
            let s = session(store, audit: audit)
            let before = s.releaseGeneration
            fs.inject(step)
            let r = await s.setAreas([.books])
            XCTAssertEqual(r.storeError, .replaceFailed(step), "\(step)")
            XCTAssertTrue(r.effective, "\(step): eine Einschränkung gilt trotzdem")
            XCTAssertEqual(s.effectiveAreas, [.books], "\(step)")
            XCTAssertGreaterThan(s.releaseGeneration, before, "\(step)")
            XCTAssertTrue(s.isContentBlocked, "\(step): für den Rest des Laufs nichts aus eigenen Inhalten")
            XCTAssertTrue(s.needsSave, "\(step): nicht gesichert")
            XCTAssertTrue(store.isDirty, "\(step)")
            // Zwei Erhöhungen, je eine Zeile `release`: die Einschränkung selbst (Ursache Bereiche, nach dem gescheiterten Austausch
            // geschrieben) und die Blockade für den Rest des Laufs (Ursache `other`, eigene Generation).
            _ = await audit.flushReleaseLines()
            let released = lines("release")
            XCTAssertEqual(released.count, 2, "\(step)")
            XCTAssertEqual(Set(released.compactMap { $0["cause"] as? String }), ["areas", "other"], "\(step)")
            XCTAssertEqual(Set(released.compactMap { $0["gen"] as? Int }).count, 2, "\(step): jede Generation genau eine Zeile")
            XCTAssertEqual(released.compactMap { $0["gen"] as? Int }.max(), s.releaseGeneration, "\(step)")
            // Dateizustand: bis zum rename die alte Fassung, nach dem rename die neue (der Ausgang ist OFFEN, nie halb).
            let onDisk = try XCTUnwrap(fileRecord(), "\(step): die Datei ist immer lesbar")
            if step == .syncDirectory {
                XCTAssertEqual(onDisk.record.localAreas, ["books"], "\(step): neue Fassung liegt, ohne bestätigt zu sein")
            } else {
                XCTAssertEqual(onDisk.record, .shipped, "\(step): die alte Fassung bleibt unberührt")
                XCTAssertEqual(onDisk.revision, 1, "\(step)")
                XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent(AssistantReleaseStore.tempName).path),
                               "\(step): die Temp-Datei wird entfernt")
            }
            // Ein Neustart (neue Instanz) sieht eine gültige Fassung, nie eine halbe.
            let again = openStore()
            XCTAssertEqual(again.opening, .loaded(revision: onDisk.revision), "\(step)")
            // Später erneut sichern: gelingt, die Blockade für den Rest des Laufs bleibt.
            fs.clear()
            let retried = await s.retrySave()
            XCTAssertNil(retried, "\(step)")
            XCTAssertFalse(s.needsSave, "\(step)")
            XCTAssertEqual(try XCTUnwrap(fileRecord()).record.localAreas, ["books"], "\(step)")
            XCTAssertTrue(s.isContentBlocked, "\(step): die Blockade bleibt bis zum nächsten Prozesslauf")
        }
    }

    func testAfterAnUnsavedRestrictionTheRunOutputsNothingFromOwnContent() async throws {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Notiz Eisvogel", area: .notes, sections: [(nil, "Der Eisvogel brütet am Mühlbach in der Uferböschung.")]))
        let fs = FaultFS()
        let store = openStore(fs)
        let s = session(store, knowledge: AssistantKnowledge(corpus: corpus), model: EchoModel(text: "Der Eisvogel brütet am Mühlbach in der Uferböschung."))
        // Vor der Einschränkung kommt das Zitat; die Einschränkung betrifft nicht die Notizen, aber ihr Sichern scheitert.
        var before: [String] = []
        for await e in s.events(AssistantRequest(requestId: "a", text: "Wo brütet der Eisvogel?")) {
            if case .sentence(_, _, let t, _, _, _, _) = e { before.append(t) }
        }
        _ = await s.commit(requestId: "a", user: "x", spoken: .nothing, interrupted: true)
        XCTAssertTrue(before.contains { $0.contains("Eisvogel") }, "Gegenprobe: ohne Blockade kommt das Zitat")
        fs.inject(.rename)
        let r = await s.setAreas([.notes, .books])
        XCTAssertNotNil(r.storeError)
        var after: [String] = []
        for await e in s.events(AssistantRequest(requestId: "b", text: "Wo brütet der Eisvogel?")) {
            if case .sentence(_, _, let t, _, _, _, _) = e { after.append(t) }
        }
        _ = await s.commit(requestId: "b", user: "x", spoken: .nothing, interrupted: true)
        XCTAssertFalse(after.contains { $0.contains("Eisvogel") }, "blockiert: nichts aus eigenen Inhalten: \(after)")
    }

    // MARK: Erweiterung: beabsichtigt, Austausch, wirksam; Fehler je Schritt

    func testAnExpansionIsOnlyEffectiveAfterAConfirmedReplaceAndEveryFailedStepLeavesItIneffectiveInThisRun() async throws {
        for step in steps {
            try? FileManager.default.removeItem(at: dir)
            let fs = FaultFS()
            let store = openStore(fs)
            store.clock.setOpportunitiesPaused(true)
            let audit = freshLog()
            let s = session(store, audit: audit)
            let before = s.releaseGeneration
            fs.inject(step)
            let r = await s.setAllowUnsourced(true, consentVersion: 1)
            XCTAssertEqual(r.storeError, .replaceFailed(step), "\(step)")
            XCTAssertFalse(r.effective, "\(step): nicht wirksam in diesem Lauf")
            XCTAssertFalse(s.allowsUnsourced, "\(step)")
            XCTAssertEqual(s.releaseGeneration, before, "\(step): die Generation steigt nur im Schritt des Wirksamwerdens")
            _ = await audit.flushReleaseLines()
            let intent = lines("intent"), effective = lines("effective"), ineffective = lines("ineffective")
            XCTAssertEqual(intent.count, 1, "\(step): die Absicht steht, bevor ersetzt wird")
            XCTAssertNil(intent.first?["gen"], "\(step): die Absicht trägt keine Generation")
            XCTAssertEqual(effective.count, 0, "\(step)")
            XCTAssertEqual(ineffective.count, 1, "\(step): „Erweiterung in diesem Lauf nicht wirksam“")
            XCTAssertNil(ineffective.first?["gen"], "\(step)")
            XCTAssertEqual(ineffective.first?["chg"] as? Int, intent.first?["chg"] as? Int, "\(step)")
            XCTAssertEqual(lines("release").count, 0, "\(step)")
            let folded = await audit.entries().filter { $0.kind == .intent }
            XCTAssertEqual(folded.map(\.effect), [.notEffective], "\(step)")
            let onDisk = try XCTUnwrap(fileRecord(), "\(step)")
            if step == .syncDirectory {
                // Fehler NACH dem rename vor dem Verzeichnis-fsync: im Lauf nicht wirksam, die neue Fassung liegt aber.
                XCTAssertTrue(onDisk.record.optIn, "\(step)")
                let reopened = openStore()
                XCTAssertEqual(reopened.opening, .loaded(revision: onDisk.revision))
                XCTAssertTrue(reopened.record.optIn, "eine neue Instanz lädt die neue Fassung")
                XCTAssertTrue(session(reopened).allowsUnsourced)
            } else {
                XCTAssertEqual(onDisk.record, .shipped, "\(step)")
            }
            // Dieselbe Erweiterung lässt sich in diesem Lauf noch einmal anstoßen (der beabsichtigte Stand wurde zurückgenommen).
            fs.clear()
            let again = await s.setAllowUnsourced(true, consentVersion: 1)
            XCTAssertTrue(again.effective, "\(step)")
            XCTAssertTrue(s.allowsUnsourced, "\(step)")
            XCTAssertEqual(s.releaseGeneration, before + 1, "\(step)")
        }
    }

    func testAnExpansionThatSucceedsWritesIntentThenReplacesTheFileThenRaisesTheGenerationThenWritesEffective() async throws {
        let rec = Recorder()
        let fs = FaultFS()
        fs.observe { rec.add($0) }
        let store = openStore(fs)
        let audit = freshLog(appender: { url, data in
            if let kind = (try? JSONSerialization.jsonObject(with: data.prefix(while: { $0 != 10 })) as? [String: Any])?["t"] as? String {
                rec.add("line:\(kind)")
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(store, audit: audit)
        let g0 = s.releaseGeneration
        let observed = Recorder()
        s.setReleaseObserver { observed.add("gen:\($0)") }
        let pending = s.applyAllowUnsourced(true, consentVersion: 2)
        XCTAssertEqual(s.releaseGeneration, g0, "die Absicht allein erhöht nichts")
        XCTAssertFalse(s.allowsUnsourced)
        XCTAssertEqual(pending.generation, g0)
        let r = await s.settle(pending)
        XCTAssertTrue(r.effective)
        XCTAssertEqual(r.generation, g0 + 1)
        XCTAssertEqual(observed.all, ["gen:\(g0 + 1)"])
        let order = rec.all.filter { ["line:intent", "line:effective", "fs:rename"].contains($0) }.drop { $0 != "line:intent" }
        XCTAssertEqual(Array(order), ["line:intent", "fs:rename", "line:effective"])
        let effective = lines("effective")
        XCTAssertEqual(effective.first?["gen"] as? Int, g0 + 1)
        XCTAssertEqual(effective.first?["consent"] as? Int, 2)
        XCTAssertEqual(fileRecord()?.record.consentVersion, 2)
        let folded = await audit.entries().filter { $0.kind == .intent }
        XCTAssertEqual(folded.map(\.effect), [.effective])
        XCTAssertEqual(folded.first?.effectiveGeneration, g0 + 1)
    }

    // MARK: Fehler der Zeilen

    func testALineThatCannotBeWrittenIsReportedAndKeepsEachChangeConsistent() async throws {
        let off = FailFlag()
        // Zeile „beabsichtigt“ scheitert: keine Erweiterung, die Datei wird nicht angefasst.
        do {
            let store = openStore()
            let audit = freshLog(appender: block(nil, failing: "intent", off))
            let s = session(store, audit: audit)
            off.value = true
            let r = await s.setAllowUnsourced(true, consentVersion: 1)
            XCTAssertFalse(r.effective)
            XCTAssertNotNil(r.error)
            XCTAssertEqual(store.confirmedRevision, 1, "ohne Zeile „beabsichtigt“ kein Austausch")
            XCTAssertEqual(fileRecord()?.record, .shipped)
            XCTAssertFalse(s.allowsUnsourced)
            off.value = false
        }
        try? FileManager.default.removeItem(at: dir)
        // Zeile „wirksam“ scheitert: die Erweiterung ist gesichert und gilt, der Fehler wird gemeldet; der Leser sagt „unbekannt“.
        do {
            let store = openStore()
            let audit = freshLog(appender: block(nil, failing: "effective", off))
            let s = session(store, audit: audit)
            off.value = true
            let r = await s.setAllowUnsourced(true, consentVersion: 1)
            XCTAssertTrue(r.effective)
            XCTAssertNotNil(r.error)
            XCTAssertTrue(s.allowsUnsourced)
            XCTAssertTrue(fileRecord()?.record.optIn ?? false)
            off.value = false
            let folded = await audit.entries().filter { $0.kind == .intent }
            XCTAssertEqual(folded.map(\.effect), [.unknown], "„beabsichtigt“ ohne „wirksam“: Wirkung unbekannt")
        }
        try? FileManager.default.removeItem(at: dir)
        // Zeile `release` einer Einschränkung scheitert: die Einschränkung steht (Prozess und Datei), der Fehler wird gemeldet.
        do {
            let store = openStore()
            let audit = freshLog(appender: block(nil, failing: "release", off))
            let s = session(store, audit: audit)
            off.value = true
            let r = await s.setAreas([.books])
            XCTAssertTrue(r.effective)
            XCTAssertNotNil(r.error)
            XCTAssertNil(r.storeError)
            XCTAssertEqual(s.effectiveAreas, [.books])
            XCTAssertEqual(fileRecord()?.record.localAreas, ["books"])
            off.value = false
        }
    }

    /// „In diesem Lauf nicht wirksam“ lässt sich bei vollem Datenträger nicht schreiben: Die Zeile wird bei der nächsten Gelegenheit nachgeholt.
    func testTheLineIneffectiveIsCaughtUpAsSoonAsTheDiskAllowsIt() async throws {
        let off = FailFlag()
        let fs = FaultFS()
        let store = openStore(fs)
        let audit = freshLog(appender: block(nil, failing: "ineffective", off))
        let s = session(store, audit: audit)
        fs.inject(.rename)
        off.value = true
        let r = await s.setAllowUnsourced(true, consentVersion: 1)
        XCTAssertFalse(r.effective)
        XCTAssertEqual(lines("ineffective").count, 0, "voller Datenträger: nicht schreibbar")
        off.value = false
        fs.clear()
        let next = await s.setAreas([.books])
        XCTAssertTrue(next.effective)
        XCTAssertEqual(lines("ineffective").count, 1, "bei der nächsten Gelegenheit nachgeholt")
        let intents = await audit.entries().filter { $0.kind == .intent }
        XCTAssertEqual(intents.map(\.effect), [.notEffective])
    }

    // MARK: Abbruch mitten im Vorgang (Prozessende)

    /// Nach einem Prozessende an jedem Teilschritt, vor und nach der Wirkung, liegt entweder die vollständige alte oder die
    /// vollständige neue Fassung; ein Neustart lädt sie und räumt die Temp-Datei weg. Nie eine halbe Fassung, nie Datenverlust.
    func testAProcessEndInTheMiddleOfAReplaceLeavesAWholeOldOrWholeNewFileAndTheRestartLoadsIt() async throws {
        for step in steps {
            for after in [false, true] {
                try? FileManager.default.removeItem(at: dir)
                let fs = FaultFS()
                let store = openStore(fs)
                let s = session(store)
                fs.inject(step, after: after, crash: true)
                let r = await s.setAreas([.books])
                XCTAssertNotNil(r.storeError, "\(step) after=\(after)")
                XCTAssertTrue(fs.hasCrashed)
                let restarted = openStore()
                guard case .loaded(let revision) = restarted.opening else { return XCTFail("\(step) after=\(after): \(restarted.opening)") }
                XCTAssertFalse(restarted.isLocked)
                let wholeNew = restarted.record.localAreas == ["books"]
                let wholeOld = restarted.record == .shipped
                XCTAssertTrue(wholeNew || wholeOld, "\(step) after=\(after): weder die alte noch die neue Fassung")
                // Die Wirkung (rename) ist der Schnitt: davor die alte, danach die neue Fassung.
                let renamed = step == .syncDirectory || (step == .rename && after)
                XCTAssertEqual(wholeNew, renamed, "\(step) after=\(after)")
                XCTAssertEqual(revision, wholeNew ? 2 : 1, "\(step) after=\(after)")
                XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent(AssistantReleaseStore.tempName).path),
                               "\(step) after=\(after): der Neustart räumt die Temp-Datei weg")
            }
        }
    }

    /// Halb geschriebene Temp-Datei (Platte voll mitten im Schreiben, danach Prozessende): Die alte Datei bleibt unberührt.
    func testATornTempFileNeverReachesTheRealFile() async throws {
        let fs = FaultFS()
        let store = openStore(fs)
        let s = session(store)
        fs.inject(.write, crash: true, torn: true)
        _ = await s.setAreas([.books])
        XCTAssertTrue(FileManager.default.fileExists(atPath: dir.appendingPathComponent(AssistantReleaseStore.tempName).path), "die halbe Temp-Datei liegt (kein Aufräumen nach dem Prozessende)")
        XCTAssertEqual(fileRecord()?.record, .shipped, "die echte Datei ist unberührt")
        let restarted = openStore()
        XCTAssertEqual(restarted.opening, .loaded(revision: 1))
        XCTAssertEqual(restarted.record, .shipped)
    }

    // MARK: Nebenläufigkeit

    /// Mehrere Änderungen zugleich: Jede Sicherungsfolge läuft ganz durch, bevor die nächste beginnt (beabsichtigt → Austausch →
    /// wirksam); die Folgen überlappen nie, auch wenn der Austausch langsam ist.
    func testConcurrentChangesRunStrictlyOneAfterTheOther() async throws {
        let rec = Recorder()
        let fs = FaultFS()
        fs.observe { rec.add($0) }
        fs.delay(.syncFile, 0.03)
        let store = openStore(fs)
        let audit = freshLog(appender: { url, data in
            if let kind = (try? JSONSerialization.jsonObject(with: data.prefix(while: { $0 != 10 })) as? [String: Any])?["t"] as? String {
                rec.add("line:\(kind)")
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = session(store, audit: audit)
        _ = await s.setAreas([.books])   // Ausgangslage: eng; die folgenden Änderungen erweitern
        let generationBefore = s.releaseGeneration
        let first = s.applyAreas([.books, .notes])
        let second = s.applyAllowUnsourced(true, consentVersion: 1)
        let third = s.applyAreas([.books, .notes, .files])
        // Alle drei zugleich sichern (jede wartet auf ihre Reihe).
        await withTaskGroup(of: Void.self) { group in
            for pending in [third, first, second] { group.addTask { _ = await s.settle(pending) } }
        }
        let relevant = rec.all.filter { ["line:intent", "line:effective", "line:release", "fs:rename"].contains($0) }
        let afterStart = Array(relevant.drop { $0 != "line:intent" })
        XCTAssertEqual(afterStart, Array(repeating: ["line:intent", "fs:rename", "line:effective"], count: 3).flatMap { $0 },
                       "streng nacheinander, nie verschränkt: \(afterStart)")
        XCTAssertEqual(store.overlaps, 0)
        XCTAssertEqual(s.releaseGeneration, generationBefore + 3, "je Erweiterung eine Generation, im Schritt des Wirksamwerdens")
        XCTAssertEqual(s.effectiveAreas, [.books, .notes, .files])
        XCTAssertTrue(s.allowsUnsourced)
        let stored = try XCTUnwrap(fileRecord())
        XCTAssertEqual(stored.record.localAreas, ["books", "files", "notes"])
        XCTAssertTrue(stored.record.optIn)
        let numbers = lines("intent").compactMap { $0["chg"] as? Int }
        XCTAssertEqual(numbers, numbers.sorted(), "die Änderungsnummern laufen in der Reihenfolge des Anwendens")
        XCTAssertEqual(Set(numbers).count, 3)
    }

    /// Viele Änderungen aus mehreren Threads: keine Überlappung, die Datei trägt am Ende genau den Stand der Sitzung.
    func testManyChangesFromSeveralThreadsNeverOverlapAndTheFileEndsAtTheStateOfTheSession() async throws {
        let fs = FaultFS()
        fs.delay(.rename, 0.001)
        let store = openStore(fs)
        let s = session(store, audit: freshLog())
        let collected = NSLock()
        nonisolated(unsafe) var pendings: [AssistantReleasePending] = []
        let group = DispatchGroup()
        for t in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                for i in 0..<12 {
                    let p: AssistantReleasePending
                    switch (t + i) % 3 {
                    case 0: p = s.applyAllowUnsourced(i % 2 == 0, consentVersion: 1)
                    case 1: p = s.applyAreas(i % 2 == 0 ? [.books] : [.books, .notes, .files, .transcripts])
                    default: p = s.applyCloudConsent(i % 2 == 0)
                    }
                    collected.withLock { pendings.append(p) }
                }
                group.leave()
            }
        }
        XCTAssertEqual(group.wait(timeout: .now() + 120), .success)
        let all = collected.withLock { pendings }
        await withTaskGroup(of: Void.self) { tasks in
            for p in all where p.changed { tasks.addTask { _ = await s.settle(p) } }
        }
        XCTAssertEqual(store.overlaps, 0)
        // Alle Sicherungsfolgen sind durch; das Gesicherte ist der wirksame Stand der Sitzung (nichts hinkt hinterher).
        let stored = try XCTUnwrap(fileRecord()).record
        XCTAssertEqual(stored.localAreas, AssistantReleaseRecord.names(s.effectiveAreas))
        XCTAssertEqual(stored.optIn, s.allowsUnsourced)
        XCTAssertFalse(store.isDirty)
    }

    /// Gemischte Änderung: Der einschränkende Teil gilt sofort (eigene Erhöhung), der erweiternde erst nach dem bestätigten Austausch.
    func testAMixedChangeIsSplitIntoARestrictionThatHoldsAtOnceAndAnExpansionThatWaits() async throws {
        let audit = freshLog()
        let store = openStore()
        let s = session(store, audit: audit)
        _ = await s.setAreas([.books, .notes])
        let g0 = s.releaseGeneration
        let pending = s.applyAreas([.books, .files])   // Notizen weg (enger), Dokumente neu (weiter)
        XCTAssertEqual(pending.applied.steps.map(\.kind), [.restriction, .expansion])
        XCTAssertEqual(s.effectiveAreas, [.books], "der engere Teil gilt sofort, der weitere noch nicht")
        XCTAssertEqual(s.releaseGeneration, g0 + 1, "genau eine Erhöhung beim Anwenden")
        XCTAssertEqual(Set(pending.applied.steps.map(\.number)).count, 2, "je Teil eine eigene Änderungsnummer")
        let r = await s.settle(pending)
        XCTAssertTrue(r.effective)
        XCTAssertEqual(s.effectiveAreas, [.books, .files])
        XCTAssertEqual(s.releaseGeneration, g0 + 2)
        XCTAssertEqual(try XCTUnwrap(fileRecord()).record.localAreas, ["books", "files"])
    }

    // MARK: Dateisystem des Systems

    func testTheRealFileSystemReplacesAtomicallyAndKeepsCountingRevisions() async throws {
        let store = openStore()
        let s = session(store)
        for (i, areas) in [[AssistantArea.books], [.books, .notes], [.notes]].enumerated() {
            let r = await s.setAreas(Set(areas))
            XCTAssertNil(r.storeError)
            XCTAssertEqual(try XCTUnwrap(fileRecord()).revision, i + 2, "die erste Fassung (Auslieferung) hat Nummer 1")
            XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent(AssistantReleaseStore.tempName).path))
        }
        XCTAssertEqual(store.confirmedRevision, 4)
        XCTAssertEqual(store.overlaps, 0)
        XCTAssertEqual(openStore().opening, .loaded(revision: 4))
    }

    func testTheCloudConsentIsKeptPerTargetWithTheAreasItCovers() async throws {
        let store = openStore()
        let cloud = AssistantConfig(modelLabel: "Cloud", isCloud: true, contextTokens: 16_000, maxAnswerTokens: 700, userName: nil, timeout: 5)
        let s = AssistantSession(model: EchoModel(text: "Hallo."), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: cloud,
                                 released: true, releaseStore: store, cloudTarget: "ziel-a")
        XCTAssertFalse(s.effectiveCloudConsent, "ausgeliefert: keine Zustimmung")
        XCTAssertEqual(s.effectiveAreas, [.books], "Cloud: nur Bücher")
        let granted = await s.setCloudConsent(true)
        XCTAssertTrue(granted.effective)
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [.books])
        XCTAssertEqual(store.record.consentedAreas("ziel-b"), [], "ein anderes Ziel hat keine Zustimmung")
        // Mehr Bereiche als die Zustimmung deckt: Die Zustimmung gilt für den neuen Stand nicht (Erweiterung der Bereiche mit Entzug).
        let wider = await s.setAreas([.books, .notes], cloudConsent: false)
        XCTAssertTrue(wider.effective)
        XCTAssertFalse(s.effectiveCloudConsent)
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [], "über die Zustimmung hinaus: der Eintrag fällt weg, er lebt nicht wieder auf")
        let narrowed = await s.setAreas([.books])
        XCTAssertTrue(narrowed.effective)
        XCTAssertFalse(s.effectiveCloudConsent, "auch bei wieder engeren Bereichen gilt keine alte Zustimmung")
        _ = await s.setAreas([.books, .notes])
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [])
        let granted2 = await s.setCloudConsent(true)
        XCTAssertTrue(granted2.effective)
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [.books, .notes])
        let revoked = await s.setCloudConsent(false)
        XCTAssertFalse(s.effectiveCloudConsent)
        XCTAssertNil(revoked.storeError)
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [], "der Entzug löscht den Eintrag des Ziels")
        // Ein neuer Lauf auf derselben Datei sieht dieselbe Lage.
        let next = AssistantSession(model: EchoModel(text: "Hallo."), knowledge: AssistantKnowledge(corpus: AssistantCorpus()), config: cloud,
                                    released: true, releaseStore: openStore(), cloudTarget: "ziel-a")
        XCTAssertFalse(next.effectiveCloudConsent)
        XCTAssertEqual(next.effectiveAreas, [.books, .notes])
    }

    // MARK: Änderung ohne Sitzung (kein Modell)

    func testAreasChangedWithoutASessionAreSecuredInTheFileAndAFollowingSessionSeesThem() async {
        let store = openStore()
        let change = await store.updateAreas([.books, .notes], cloud: false)
        XCTAssertNil(change.storeError)
        XCTAssertNil(change.auditError)
        XCTAssertTrue(change.applied)
        XCTAssertEqual(change.effective, [.books, .notes])
        XCTAssertEqual(fileRecord()?.record.areas(cloud: false), [.books, .notes])
        XCTAssertEqual(store.confirmedRevision, 2)
        XCTAssertEqual(session(openStore()).effectiveAreas, [.books, .notes])
        XCTAssertEqual(store.overlaps, 0)
    }

    /// Ein Eintrag der Cloud-Zustimmung, den die neuen Cloud-Bereiche nicht decken, fällt weg; einer, den sie decken, bleibt.
    func testCloudAreasChangedWithoutASessionDropConsentsThatDoNotCoverThem() async {
        let seed = AssistantReleaseRecord(localAreas: Set(AssistantArea.allCases), cloudAreas: [.books],
                                          consents: ["ziel-a": [.books], "ziel-b": [.books, .notes]])
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? AssistantReleaseFile.encode(seed, revision: 3)?.write(to: stateFile)
        let store = openStore()
        XCTAssertEqual(store.opening, .loaded(revision: 3))
        let change = await store.updateAreas([.books, .notes], cloud: true)
        XCTAssertNil(change.storeError)
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [], "deckt die neuen Bereiche nicht: gelöscht")
        XCTAssertEqual(store.record.consentedAreas("ziel-b"), [.books, .notes], "deckt sie: bleibt")
        let narrower = await store.updateAreas([.books], cloud: true)
        XCTAssertNil(narrower.storeError)
        XCTAssertEqual(store.record.consentedAreas("ziel-a"), [], "ein gelöschter Eintrag lebt bei engeren Bereichen nicht wieder auf")
        XCTAssertEqual(store.record.consentedAreas("ziel-b"), [.books, .notes])
    }

    /// Scheitert der Austausch bei einer Einschränkung (jeder Teilschritt), ist der Speicher blockiert und die Datei bleibt die alte
    /// vollständige Fassung; bei einer reinen Erweiterung bleibt alles, wie es war.
    func testAFailedRestrictionWithoutASessionBlocksTheStoreAndAFailedExpansionDoesNot() async {
        for step in steps where step != .syncDirectory {
            try? FileManager.default.removeItem(at: dir)
            let fs = FaultFS()
            let store = openStore(fs)
            fs.inject(step, times: 1)
            let restriction = await store.updateAreas([.books], cloud: false)
            XCTAssertEqual(restriction.storeError, .replaceFailed(step), "\(step)")
            XCTAssertEqual(restriction.effective, [.books], "\(step): die Einschränkung gilt trotzdem")
            XCTAssertTrue(store.isBlocked, "\(step): Widerruf nicht gesichert")
            XCTAssertTrue(store.isDirty, "\(step)")
            XCTAssertEqual(fileRecord()?.record.areas(cloud: false), Set(AssistantArea.allCases), "\(step): die alte Fassung liegt vollständig vor")
            XCTAssertTrue(session(store).isContentBlocked, "\(step): eine spätere Sitzung gibt nichts aus eigenen Inhalten aus")
        }
        try? FileManager.default.removeItem(at: dir)
        let fs = FaultFS()
        let store = openStore(fs)
        _ = await store.updateAreas([.books], cloud: false)
        fs.inject(.rename, times: 1)
        let expansion = await store.updateAreas([.books, .notes], cloud: false)
        XCTAssertEqual(expansion.storeError, .replaceFailed(.rename))
        XCTAssertFalse(expansion.applied)
        XCTAssertFalse(store.isBlocked, "eine gescheiterte Erweiterung blockiert nichts: der enge Stand gilt weiter")
        XCTAssertEqual(store.record.areas(cloud: false), [.books])
    }

    func testAnUpdateWithoutASessionUnlocksALockedStoreOnlyAfterTheFileIsConfirmed() async throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("kaputt".utf8).write(to: stateFile)
        let fs = FaultFS()
        let store = openStore(fs)
        XCTAssertTrue(store.isLocked)
        fs.inject(.write, times: 1)
        let failed = await store.updateAreas([.books], cloud: false)
        XCTAssertEqual(failed.storeError, .replaceFailed(.write))
        XCTAssertTrue(store.isLocked, "nicht bestätigt: weiter gesperrt")
        let ok = await store.updateAreas([.books], cloud: false)
        XCTAssertNil(ok.storeError)
        XCTAssertFalse(store.isLocked)
        XCTAssertEqual(store.record.areas(cloud: false), [.books])
        XCTAssertEqual(openStore().opening, .loaded(revision: store.confirmedRevision))
    }

    // MARK: Änderung ohne Sitzung: Zeilen im Protokoll (Schreibfolge 8)

    private func recordedAudit(_ rec: Recorder) -> AssistantAuditLog {
        freshLog(appender: { url, data in
            if let kind = (try? JSONSerialization.jsonObject(with: data.prefix(while: { $0 != 10 })) as? [String: Any])?["t"] as? String {
                rec.add("line:\(kind)")
            }
            try AssistantAuditWriter.appendFile(url, data)
        })
    }

    private func seed(_ record: AssistantReleaseRecord, revision: Int = 5) throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try XCTUnwrap(AssistantReleaseFile.encode(record, revision: revision)).write(to: stateFile)
    }

    /// Einschränkung ohne Sitzung: erst die Datei, dann die Zeile `release` mit Lauf, Änderungsnummer, Ursache, Stand und der
    /// Generation, die im Schritt des Wirksamwerdens (im Lock) vergeben wurde (Runde 7, B9: die eine Ordnung gilt auch ohne Sitzung).
    func testARestrictionWithoutASessionReplacesTheFileThenWritesTheLineWithItsGeneration() async throws {
        let rec = Recorder()
        let fs = FaultFS()
        let store = openStore(fs)
        fs.observe { rec.add($0) }
        let audit = recordedAudit(rec)
        let change = await store.updateAreas([.books], cloud: false, audit: audit)
        XCTAssertNil(change.storeError)
        XCTAssertNil(change.auditError)
        XCTAssertTrue(change.applied)
        XCTAssertEqual(change.effective, [.books])
        XCTAssertEqual(rec.all.filter { ["fs:rename", "line:release"].contains($0) }, ["fs:rename", "line:release"], "erst die Datei, dann die Zeile")
        let released = lines("release")
        XCTAssertEqual(released.count, 1)
        let line = try XCTUnwrap(released.first)
        XCTAssertEqual(line["gen"] as? Int, 1, "die Generation der Ordnung, im selben Lock-Schritt wie der engere Stand vergeben")
        XCTAssertEqual(line["run"] as? Int, 1)
        XCTAssertEqual(line["chg"] as? Int, 1)
        XCTAssertEqual(line["cause"] as? String, "areas")
        XCTAssertEqual(line["areas"] as? [String], ["books"])
        XCTAssertNil(line["model"], "kein Modell, keine erfundene Art")
        // Der Leser versteht die Zeile ohne Generation.
        let read = await audit.read()
        XCTAssertEqual(read.skippedLines, 0)
        let folded = read.entries.filter { $0.kind == .release }
        XCTAssertEqual(folded.count, 1)
        XCTAssertEqual(folded.first?.generation, 1)
        XCTAssertEqual(folded.first?.run, 1)
        XCTAssertEqual(folded.first?.change, 1)
        XCTAssertEqual(folded.first?.cause, .areas)
        XCTAssertEqual(folded.first?.areas, ["books"])
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"])
    }

    /// Erweiterung ohne Sitzung: Zeile „beabsichtigt“ (ohne Generation) vor dem Austausch, Zeile „wirksam“ danach, MIT der im Lock des
    /// Wirksamwerdens vergebenen Generation; Lauf und Änderungsnummern laufen fortlaufend über die Änderungen.
    func testAnExpansionWithoutASessionWritesIntentThenReplacesTheFileThenWritesEffective() async throws {
        let rec = Recorder()
        let fs = FaultFS()
        let store = openStore(fs)
        let audit = recordedAudit(rec)
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        fs.observe { rec.add($0) }
        let change = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
        XCTAssertNil(change.storeError)
        XCTAssertTrue(change.applied)
        XCTAssertEqual(change.effective, [.books, .notes])
        let order = rec.all.filter { ["line:intent", "line:effective", "fs:rename"].contains($0) }.drop { $0 != "line:intent" }
        XCTAssertEqual(Array(order), ["line:intent", "fs:rename", "line:effective"])
        XCTAssertEqual(lines().compactMap { $0["t"] as? String }, ["release", "intent", "effective"])
        XCTAssertEqual(lines().compactMap { $0["chg"] as? Int }, [1, 2, 2], "Änderungsnummern fortlaufend, beide Zeilen einer Erweiterung gleich")
        XCTAssertEqual(Set(lines().compactMap { $0["run"] as? Int }), [1], "ein Lauf")
        XCTAssertEqual(lines().map { $0["gen"] as? Int }, [1, nil, 2], "release 1; intent ohne; effective mit der Generation des Wirksamwerdens")
        let intents = await audit.entries().filter { $0.kind == .intent }
        XCTAssertEqual(intents.map(\.effect), [.effective])
        XCTAssertEqual(intents.first?.effectiveGeneration, 2)
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books", "notes"])
    }

    /// Gemischte Änderung: erst der einschränkende Teil (Austausch, Zeile `release`), dann der erweiternde (Zeile „beabsichtigt“,
    /// Austausch, Zeile „wirksam“): zwei Austausche, zwei Änderungsnummern.
    func testAMixedChangeWithoutASessionRestrictsFirstThenExpands() async throws {
        let rec = Recorder()
        let fs = FaultFS()
        let store = openStore(fs)
        let audit = recordedAudit(rec)
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        fs.observe { rec.add($0) }
        let before = store.confirmedRevision
        let change = await store.updateAreas([.notes], cloud: false, audit: audit)
        XCTAssertTrue(change.applied)
        XCTAssertEqual(change.effective, [.notes])
        XCTAssertEqual(store.confirmedRevision, before + 2, "zwei Austausche")
        let order = rec.all.filter { ["line:release", "line:intent", "line:effective", "fs:rename"].contains($0) }.drop { $0 != "fs:rename" }
        XCTAssertEqual(Array(order), ["fs:rename", "line:release", "line:intent", "fs:rename", "line:effective"])
        XCTAssertEqual(lines().compactMap { $0["chg"] as? Int }, [1, 2, 3, 3])
        XCTAssertEqual(fileRecord()?.record.localAreas, ["notes"])
    }

    func testAnExpansionWithoutASessionIsOnlyEffectiveAfterAConfirmedReplaceAndEveryFailedStepLeavesItIneffective() async throws {
        for step in steps {
            try? FileManager.default.removeItem(at: dir)
            let fs = FaultFS()
            let store = openStore(fs)
            let audit = freshLog()
            let first = await store.updateAreas([.books], cloud: false, audit: audit)
            XCTAssertNil(first.storeError)
            fs.inject(step)
            let change = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
            XCTAssertEqual(change.storeError, .replaceFailed(step), "\(step)")
            XCTAssertNil(change.auditError, "\(step)")
            XCTAssertFalse(change.applied, "\(step)")
            XCTAssertEqual(change.effective, [.books], "\(step): die Erweiterung gilt in diesem Lauf nicht")
            XCTAssertFalse(store.isBlocked, "\(step): eine gescheiterte Erweiterung blockiert nichts")
            XCTAssertFalse(store.hasPendingRestriction, "\(step): eine gescheiterte Erweiterung wird nicht gemerkt")
            let intent = lines("intent"), effective = lines("effective"), ineffective = lines("ineffective")
            XCTAssertEqual(intent.count, 1, "\(step): die Absicht steht, bevor ersetzt wird")
            XCTAssertEqual(effective.count, 0, "\(step)")
            XCTAssertEqual(ineffective.count, 1, "\(step): „Erweiterung in diesem Lauf nicht wirksam“")
            XCTAssertNil(intent.first?["gen"], "\(step)")
            XCTAssertNil(ineffective.first?["gen"], "\(step)")
            XCTAssertEqual(ineffective.first?["chg"] as? Int, intent.first?["chg"] as? Int, "\(step)")
            let folded = await audit.entries().filter { $0.kind == .intent }
            XCTAssertEqual(folded.map(\.effect), [.notEffective], "\(step)")
            let onDisk = try XCTUnwrap(fileRecord(), "\(step)")
            XCTAssertEqual(onDisk.record.localAreas, step == .syncDirectory ? ["books", "notes"] : ["books"], "\(step)")
            // Dieselbe Erweiterung lässt sich noch einmal anstoßen.
            fs.clear()
            let again = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
            XCTAssertTrue(again.applied, "\(step)")
            XCTAssertEqual(try XCTUnwrap(fileRecord()).record.localAreas, ["books", "notes"], "\(step)")
        }
    }

    /// Scheitert die Zeile „beabsichtigt“, gibt es keinen Austausch und keine Erweiterung; die Zeile „nicht wirksam“ steht trotzdem.
    func testAnExpansionWithoutASessionWhoseIntentLineFailsDoesNotTouchTheFile() async throws {
        let off = FailFlag()
        let store = openStore()
        let audit = freshLog(appender: block(nil, failing: "intent", off))
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        let revision = store.confirmedRevision
        off.value = true
        let change = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
        XCTAssertNotNil(change.auditError, "der Fehler der Zeile wird gemeldet")
        XCTAssertNil(change.storeError)
        XCTAssertFalse(change.applied)
        XCTAssertEqual(store.confirmedRevision, revision, "ohne Zeile „beabsichtigt“ kein Austausch")
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"])
        XCTAssertEqual(change.effective, [.books])
        off.value = false
        XCTAssertEqual(lines("ineffective").count, 1)
        let folded = await audit.entries().filter { $0.kind == .intent }
        XCTAssertEqual(folded.map(\.effect), [.notEffective])
    }

    /// „In diesem Lauf nicht wirksam“ lässt sich nicht schreiben: Die Zeile wird bei der nächsten Gelegenheit nachgeholt.
    func testTheIneffectiveLineWithoutASessionIsCaughtUpAsSoonAsTheDiskAllowsIt() async throws {
        let off = FailFlag()
        let fs = FaultFS()
        let store = openStore(fs)
        let audit = freshLog(appender: block(nil, failing: "ineffective", off))
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        fs.inject(.rename, times: 1)
        off.value = true
        let change = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
        XCTAssertEqual(change.storeError, .replaceFailed(.rename))
        XCTAssertNotNil(change.auditError)
        XCTAssertEqual(lines("ineffective").count, 0, "voller Datenträger: nicht schreibbar")
        off.value = false
        let next = await store.updateAreas([.books], cloud: false, audit: audit)
        XCTAssertNil(next.auditError)
        XCTAssertEqual(lines("ineffective").count, 1, "bei der nächsten Gelegenheit nachgeholt")
        let intents = await audit.entries().filter { $0.kind == .intent }
        XCTAssertEqual(intents.map(\.effect), [.notEffective])
    }

    /// Scheitert die Zeile „wirksam“, ist die Erweiterung gesichert und gilt; der Fehler wird gemeldet, der Leser sagt „unbekannt“, und
    /// ein Neustart lädt die neue Fassung (Prozessende zwischen Austausch und Zeile).
    func testAnEffectiveLineThatCannotBeWrittenKeepsTheSecuredExpansionAndTheReaderSaysUnknown() async throws {
        let off = FailFlag()
        let store = openStore()
        let audit = freshLog(appender: block(nil, failing: "effective", off))
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        off.value = true
        let change = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
        XCTAssertNotNil(change.auditError)
        XCTAssertNil(change.storeError)
        XCTAssertTrue(change.applied)
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books", "notes"])
        off.value = false
        let folded = await audit.entries().filter { $0.kind == .intent }
        XCTAssertEqual(folded.map(\.effect), [.unknown], "„beabsichtigt“ ohne „wirksam“: Wirkung unbekannt")
        XCTAssertEqual(openStore().record.areas(cloud: false), [.books, .notes], "ein Neustart lädt die neue Fassung")
    }

    /// Scheitert die Zeile `release` (voller Datenträger) oder das Protokoll ist nicht lesbar (fehlendes Gerät), bleibt die gesicherte
    /// Einschränkung; der Fehler wird gemeldet.
    func testARestrictionWithoutASessionWhoseLineFailsStaysSecuredAndIsReported() async throws {
        let off = FailFlag()
        do {
            let store = openStore()
            let audit = freshLog(appender: block(nil, failing: "release", off))
            off.value = true
            let change = await store.updateAreas([.books], cloud: false, audit: audit)
            XCTAssertNotNil(change.auditError)
            XCTAssertNil(change.storeError)
            XCTAssertEqual(change.effective, [.books])
            XCTAssertEqual(fileRecord()?.record.localAreas, ["books"])
            off.value = false
        }
        try? FileManager.default.removeItem(at: dir)
        let store = openStore()
        let unreadable = AssistantAuditLog(directory: dir, reader: { _ in throw DiskFull() })
        let change = await store.updateAreas([.books], cloud: false, audit: unreadable)
        XCTAssertEqual(change.auditError, .readFailed, "Protokoll nicht lesbar: gemeldet")
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"], "der gesicherte Stand bleibt")
    }

    /// EIN Lauf-Token je Prozesslauf (je Speicher): alle Zeilen ohne Sitzung UND die einer Sitzung danach tragen dieselbe Laufnummer.
    func testTheLinesWithoutASessionAndOfASessionAfterwardsShareOneRun() async throws {
        let store = openStore()
        let audit = freshLog()
        _ = await store.updateAreas([.books], cloud: false, audit: audit)
        _ = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
        let s = session(store, audit: audit)
        _ = await s.setAreas([.books])
        _ = await audit.flushReleaseLines()
        let runs = lines().filter { $0["t"] as? String != "state" }.compactMap { $0["run"] as? Int }
        XCTAssertEqual(runs.count, 4, "release; intent, effective; release der Sitzung")
        XCTAssertEqual(Set(runs), [1], "ohne Sitzung und die Sitzung danach: derselbe Prozesslauf, dieselbe Laufnummer")
    }

    /// Ein Blockierter Speicher nimmt keine Erweiterung an; die noch nicht gesicherte Einschränkung wird bei dieser Gelegenheit
    /// erneut gesichert.
    func testABlockedStoreRefusesAnExpansionWithoutASessionAndRetriesTheRestrictionOnTheWay() async throws {
        let fs = FaultFS()
        let store = openStore(fs)
        let audit = freshLog()
        fs.inject(.rename, times: 1)
        let failed = await store.updateAreas([.books], cloud: false, audit: audit)
        XCTAssertEqual(failed.storeError, .replaceFailed(.rename))
        XCTAssertTrue(store.isBlocked)
        XCTAssertEqual(fileRecord()?.record.areas(cloud: false), Set(AssistantArea.allCases), "die alte Fassung liegt")
        let change = await store.updateAreas([.books, .notes], cloud: false, audit: audit)
        XCTAssertNil(change.storeError)
        XCTAssertFalse(change.applied, "blockiert: keine Erweiterung in diesem Lauf")
        XCTAssertEqual(change.effective, [.books])
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"], "bei dieser Gelegenheit gesichert")
        XCTAssertFalse(store.isDirty)
        XCTAssertTrue(store.isBlocked, "die Blockade bleibt bis zum nächsten Lauf")
        XCTAssertEqual(lines("intent").count, 0, "keine Absicht: es wird nichts versucht")
        XCTAssertEqual(lines("ineffective").count, 1)
        // Runde 8 (R3): Die Blockade ist ein Ereignis der einen Ordnung mit eigener Generation und eigener Zeile (auch ohne Sitzung):
        // eine laufende Anfrage, die zwischen Einschränkung und Blockade begann, endet damit.
        XCTAssertEqual(lines("release").compactMap { $0["cause"] as? String }.sorted(), ["areas", "other"],
                       "die Einschränkung hat ihre Zeile, auch wenn die Sicherung erst später gelang; dazu die Zeile der Blockade")
    }

    // MARK: Gescheiterte Einschränkung: der engere Stand geht nicht verloren

    private var cloudSeed: AssistantReleaseRecord {
        AssistantReleaseRecord(localAreas: Set(AssistantArea.allCases), cloudAreas: [.books, .notes], consents: ["ziel": [.books, .notes]])
    }

    /// Eine Einschränkung der Cloud-Bereiche scheitert; danach gelingt eine Änderung der lokalen Bereiche: Die Datei enthält den
    /// entzogenen Cloud-Bereich und die Zustimmung dazu NICHT wieder. Für jeden Teilschritt.
    func testAFailedRestrictionIsNeverWrittenBackAsGrantedByALaterChangeWithoutASession() async throws {
        for step in steps {
            try? FileManager.default.removeItem(at: dir)
            try seed(cloudSeed)
            let fs = FaultFS()
            let store = openStore(fs)
            fs.inject(step, times: 1)
            let failed = await store.updateAreas([.books], cloud: true)
            XCTAssertEqual(failed.storeError, .replaceFailed(step), "\(step)")
            XCTAssertEqual(failed.effective, [.books], "\(step)")
            XCTAssertTrue(store.hasPendingRestriction, "\(step)")
            XCTAssertEqual(store.record.areas(cloud: true), [.books], "\(step): der gültige Stand ist der engere")
            let ok = await store.updateAreas([.books, .files], cloud: false)
            XCTAssertNil(ok.storeError, "\(step)")
            let onDisk = try XCTUnwrap(fileRecord(), "\(step)")
            XCTAssertEqual(onDisk.record.localAreas, ["books", "files"], "\(step)")
            XCTAssertEqual(onDisk.record.cloudAreas, ["books"], "\(step): der entzogene Cloud-Bereich bleibt entzogen")
            XCTAssertFalse(store.hasPendingRestriction, "\(step): bestätigt, nichts mehr gemerkt")
            XCTAssertFalse(store.isDirty, "\(step)")
            XCTAssertTrue(store.isBlocked, "\(step): die Blockade bleibt bis zum nächsten Lauf")
        }
    }

    /// Dasselbe mit Sitzungen: eine Sitzung, die nach dem Fehler beginnt (sie startet auf dem engeren Stand und blockiert), und eine, die
    /// schon vorher lief und danach `persist` ruft.
    func testAFailedRestrictionIsNeverWrittenBackAsGrantedByASessionThatSavesLater() async throws {
        try seed(cloudSeed)
        let fs = FaultFS()
        let store = openStore(fs)
        fs.inject(.rename, times: 1)
        let failed = await store.updateAreas([.books], cloud: true)
        XCTAssertNotNil(failed.storeError)
        // Eine Sitzung nach dem Fehler: blockiert, Bereiche des lokalen Modells unverändert, Cloud-Stand nicht weiter als eingeschränkt.
        let after = session(store)
        XCTAssertTrue(after.isContentBlocked)
        XCTAssertEqual(after.effectiveAreas, Set(AssistantArea.allCases))
        let r1 = await after.setAreas([.books, .notes])
        XCTAssertNil(r1.storeError)
        XCTAssertEqual(try XCTUnwrap(fileRecord()).record.cloudAreas, ["books"], "die Sitzung danach schreibt den Cloud-Bereich nicht zurück")
        // Die Sitzung von vorher, ohne Blockade, ändert ihre lokalen Bereiche: wieder gelingt es, wieder bleibt der Cloud-Bereich weg.
        try seed(cloudSeed, revision: 50)
        let reopened = openStore(fs)
        let early = session(reopened)
        fs.inject(.rename, times: 1)
        _ = await reopened.updateAreas([.books], cloud: true)
        let r2 = await early.setAreas([.books, .files, .notes])
        XCTAssertNil(r2.storeError)
        XCTAssertTrue(r2.effective)
        XCTAssertEqual(try XCTUnwrap(fileRecord()).record.localAreas, ["books", "files", "notes"], "eine unabhängige Änderung wird nicht mit begrenzt")
        XCTAssertEqual(try XCTUnwrap(fileRecord()).record.cloudAreas, ["books"])
    }

    /// Eine Zustimmung, die wegen weiterer Cloud-Bereiche entfällt (Entzug), geht bei gescheiterter Sicherung ebenso nicht verloren:
    /// Eine spätere Änderung schreibt sie nicht wieder in die Datei.
    func testAConsentThatWasWithdrawnButNotSavedIsNeverWrittenBackByALaterChange() async throws {
        try seed(AssistantReleaseRecord(localAreas: Set(AssistantArea.allCases), cloudAreas: [.books], consents: ["ziel": [.books], "anderes": [.books, .notes]]))
        let fs = FaultFS()
        let store = openStore(fs)
        // Der Austausch der Einschränkung scheitert, und das Nachholen zu Beginn des erweiternden Teils derselben Änderung ebenso.
        fs.inject(.rename, times: 2)
        let failed = await store.updateAreas([.books, .notes], cloud: true)
        XCTAssertEqual(failed.storeError, .replaceFailed(.rename))
        XCTAssertEqual(store.record.consentedAreas("ziel"), [], "gilt sofort: die Zustimmung deckt die neuen Bereiche nicht")
        XCTAssertEqual(store.record.consentedAreas("anderes"), [.books, .notes], "eine Zustimmung, die sie deckt, bleibt")
        XCTAssertEqual(fileRecord()?.record.consents["ziel"], ["books"], "in der Datei steht sie noch")
        let ok = await store.updateAreas([.books], cloud: false)
        XCTAssertNil(ok.storeError)
        XCTAssertNil(fileRecord()?.record.consents["ziel"], "der Entzug ist gesichert und lebt nicht wieder auf")
        XCTAssertEqual(fileRecord()?.record.consents["anderes"], ["books", "notes"])
    }

    /// Scheitert die Einschränkung IN der Sitzung, geht auch sie nicht verloren, wenn danach eine Änderung ohne Sitzung gelingt.
    func testARestrictionThatFailedInASessionIsKeptByALaterChangeWithoutASession() async throws {
        let fs = FaultFS()
        let store = openStore(fs)
        let s = session(store)
        fs.inject(.rename, times: 1)
        let failed = await s.setAreas([.books])
        XCTAssertNotNil(failed.storeError)
        XCTAssertEqual(fileRecord()?.record.areas(cloud: false), Set(AssistantArea.allCases), "die alte Fassung liegt")
        XCTAssertEqual(store.record.areas(cloud: false), [.books], "gemerkt")
        let ok = await store.updateAreas([], cloud: true)
        XCTAssertNil(ok.storeError)
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"], "die Einschränkung der Sitzung steht jetzt in der Datei")
        XCTAssertEqual(fileRecord()?.record.cloudAreas, [])
    }

    /// Eine noch nicht gesicherte Einschränkung wird bei der nächsten Gelegenheit (hier: der Anfang der nächsten Sicherungsfolge)
    /// gesichert, BEVOR irgendetwas anderes ersetzt wird (Runde 7, B10). Seit Runde 8 (R3) ist die Blockade ein Stand der einen Ordnung:
    /// Auch eine Sitzung, die vor dem Fehler begann, ist blockiert und nimmt in diesem Lauf keine Erweiterung mehr an.
    func testAnUnsavedRestrictionIsSecuredOnTheNextOccasionAndTheBlockedOrderRefusesTheExpansion() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books], cloudAreas: [.books]))
        let fs = FaultFS()
        let rec = Recorder()
        let store = openStore(fs)
        let s = session(store)
        fs.inject(.rename, times: 1)
        let failed = await store.updateAreas([], cloud: false)
        XCTAssertNotNil(failed.storeError)
        XCTAssertTrue(s.isContentBlocked, "die Blockade der Änderung ohne Sitzung gilt in der Sitzung, die schon lief")
        XCTAssertEqual(s.effectiveAreas, [], "die Einschränkung gilt in jeder Sitzung sofort")
        fs.observe { rec.add($0) }
        let r = await s.setAreas([.books, .notes])
        XCTAssertNil(r.storeError, "die Einschränkung wurde zuerst gesichert")
        XCTAssertFalse(r.effective, "blockiert: keine Erweiterung in diesem Lauf")
        XCTAssertEqual(rec.all.filter { $0 == "fs:rename" }.count, 1, "ein Austausch: die Einschränkung, keiner für die Erweiterung")
        XCTAssertEqual(s.effectiveAreas, [])
        XCTAssertEqual(fileRecord()?.record.localAreas, [], "die Einschränkung steht jetzt in der Datei")
        XCTAssertFalse(store.hasPendingRestriction)
    }

    /// Lässt sich die Einschränkung weiter nicht sichern, gilt eine Erweiterung nie als wirksam, und der Fehler des Nachholens wird gemeldet.
    func testAnExpansionNeverCountsAsEffectiveWhileTheRestrictionStaysUnsaved() async throws {
        try seed(AssistantReleaseRecord(localAreas: [.books], cloudAreas: [.books]))
        let fs = FaultFS()
        let store = openStore(fs)
        let s = session(store)
        fs.inject(.rename, times: 2)   // die Einschränkung und das Nachholen am Anfang der Folge der Erweiterung
        let failed = await store.updateAreas([], cloud: false)
        XCTAssertNotNil(failed.storeError)
        let r = await s.setAreas([.books, .notes])
        XCTAssertNotNil(r.storeError, "der Fehler des Nachholens wird gemeldet")
        XCTAssertFalse(r.effective, "die Erweiterung gilt nicht")
        XCTAssertEqual(s.effectiveAreas, [], "die Einschränkung gilt in der Sitzung, die Erweiterung nicht")
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"], "die Datei trägt weiter die alte Fassung")
        XCTAssertTrue(store.hasPendingRestriction, "die Einschränkung bleibt gemerkt")
    }

    /// „Erneut sichern“ ohne Sitzung: sichert den gemerkten engeren Stand (gleiche Warteschlange); die Blockade bleibt.
    func testRetryingTheSaveWithoutASessionSecuresTheKeptRestriction() async throws {
        let fs = FaultFS()
        let store = openStore(fs)
        fs.inject(.write)
        let failed = await store.updateAreas([.books], cloud: false)
        XCTAssertEqual(failed.storeError, .replaceFailed(.write))
        XCTAssertTrue(store.isDirty)
        let stillFailing = await store.retrySave()
        XCTAssertEqual(stillFailing, .replaceFailed(.write), "scheitert es weiter, bleibt der engere Stand gemerkt")
        XCTAssertTrue(store.hasPendingRestriction)
        XCTAssertEqual(store.record.areas(cloud: false), [.books])
        fs.clear()
        let retried = await store.retrySave()
        XCTAssertNil(retried)
        XCTAssertFalse(store.isDirty)
        XCTAssertFalse(store.hasPendingRestriction)
        XCTAssertEqual(fileRecord()?.record.localAreas, ["books"])
        XCTAssertTrue(store.isBlocked, "die Blockade bleibt bis zum nächsten Lauf")
    }

    func testRetryingTheSaveOnALockedStoreDoesNotUnlockIt() async throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("kaputt".utf8).write(to: stateFile)
        let store = openStore()
        let retried = await store.retrySave()
        XCTAssertEqual(retried, .invalid)
        XCTAssertTrue(store.isLocked)
        XCTAssertEqual(try Data(contentsOf: stateFile), Data("kaputt".utf8), "die Datei bleibt unberührt")
    }

    /// Die benannte Grenze: Das Gemerkte lebt nur im Prozess. Nach einem Neustart vor der gelungenen Sicherung gilt die Datei.
    func testTheKeptRestrictionLivesOnlyInTheRunAndARestartReadsTheFile() async throws {
        let fs = FaultFS()
        let store = openStore(fs)
        fs.inject(.rename, times: 1)
        _ = await store.updateAreas([.books], cloud: false)
        XCTAssertEqual(store.record.areas(cloud: false), [.books])
        let restarted = openStore()
        XCTAssertEqual(restarted.record.areas(cloud: false), Set(AssistantArea.allCases), "nach dem Neustart gilt die Datei")
        XCTAssertFalse(restarted.hasPendingRestriction)
        XCTAssertFalse(restarted.isDirty)
    }

    /// Die Obergrenze selbst: nur, was einschränkte; Erweiterungen und Unabhängiges nicht.
    func testTheCeilingKeepsOnlyWhatNarrowedAndLimitsLaterVersions() {
        let from = AssistantReleaseRecord(optIn: true, consentVersion: 3, localAreas: [.books, .notes], cloudAreas: [.books],
                                          consents: ["a": [.books], "b": [.books, .notes]])
        let to = AssistantReleaseRecord(optIn: false, consentVersion: 0, localAreas: [.books], cloudAreas: [.books, .files],
                                        consents: ["a": [.books]])
        let ceiling = AssistantReleaseCeiling(narrowing: from, to: to)
        XCTAssertTrue(ceiling.optInOff)
        XCTAssertEqual(ceiling.local, [.books])
        XCTAssertNil(ceiling.cloud, "mehr Cloud-Bereiche sind eine Erweiterung: keine Grenze")
        XCTAssertEqual(ceiling.consents, ["b": []], "die Zustimmung für b entfiel; a blieb")
        var later = from
        later.localAreas = ["books", "files", "notes"]
        later.cloudAreas = ["books", "files"]
        let limited = ceiling.apply(to: later)
        XCTAssertFalse(limited.optIn)
        XCTAssertEqual(limited.consentVersion, 0)
        XCTAssertEqual(limited.localAreas, ["books"])
        XCTAssertEqual(limited.cloudAreas, ["books", "files"], "unabhängig: nicht mit begrenzt")
        XCTAssertNil(limited.consents["b"])
        XCTAssertEqual(limited.consents["a"], ["books"])
        XCTAssertTrue(AssistantReleaseCeiling().isEmpty)
        XCTAssertEqual(ceiling.merged(with: AssistantReleaseCeiling(narrowing: to, to: AssistantReleaseRecord(localAreas: [], cloudAreas: [.books]))).local, [])
    }

    /// Änderungen ohne Sitzung und mit Sitzung laufen auf derselben Warteschlange: nie überlappend, kein Stand geht verloren. Die Zeilen
    /// der Folgen ohne Sitzung vermischen sich nie: Änderungsnummern fortlaufend, die Zeilen einer Änderung nebeneinander und in Folge.
    func testChangesWithAndWithoutASessionNeverOverlapAndLoseNothing() async {
        let store = openStore()
        let audit = freshLog()
        let s = session(store, audit: audit)
        await withTaskGroup(of: Void.self) { group in
            group.addTask {
                for i in 0..<20 { _ = await store.updateAreas(i % 2 == 0 ? [.books] : [.books, .notes], cloud: true, audit: audit) }
            }
            group.addTask {
                for i in 0..<20 { _ = await s.setAreas(i % 2 == 0 ? [.books, .files] : [.files], cloudConsent: nil) }
            }
        }
        XCTAssertEqual(store.overlaps, 0)
        let final = openStore().record
        XCTAssertEqual(final.areas(cloud: false), s.effectiveAreas, "die Datei trägt den letzten Stand der Sitzung")
        XCTAssertEqual(final.areas(cloud: true), [.books, .notes], "und den letzten Stand der Änderung ohne Sitzung")
        assertLinesAreSequential(lines())
    }

    /// Mehrere Folgen ohne Sitzung zugleich (lokal und Cloud, viele Änderungen): streng nacheinander, nichts überlappt, nichts geht verloren.
    func testSeveralChangesWithoutASessionAtOnceRunStrictlyOneAfterTheOther() async {
        let store = openStore()
        let audit = freshLog()
        await withTaskGroup(of: Void.self) { group in
            for task in 0..<4 {
                group.addTask {
                    for i in 0..<10 {
                        let cloud = task % 2 == 0
                        let wide = i % 2 == 1
                        _ = await store.updateAreas(cloud ? (wide ? [.books, .notes] : [.books]) : (wide ? [.books, .files] : [.files]),
                                                    cloud: cloud, audit: audit)
                    }
                }
            }
        }
        XCTAssertEqual(store.overlaps, 0)
        XCTAssertEqual(openStore().record, store.record, "die Datei trägt genau den Stand im Prozess")
        XCTAssertFalse(store.isBlocked)
        XCTAssertFalse(store.hasPendingRestriction)
        assertLinesAreSequential(lines())
    }

    /// Die Zeilen ohne Sitzung: ein Lauf, Änderungsnummern 1...n ohne Lücke, die Zeilen einer Änderung nebeneinander, vor „wirksam“ steht
    /// „beabsichtigt“.
    private func assertLinesAreSequential(_ all: [[String: Any]], file: StaticString = #filePath, line: UInt = #line) {
        // Die eine Ordnung: Änderungen mit und ohne Sitzung teilen Lauf und Nummernraum; die Zeilen EINER Änderung stehen nebeneinander.
        let changes = all.filter { ["release", "intent", "effective", "ineffective"].contains($0["t"] as? String ?? "") }
        XCTAssertEqual(Set(changes.compactMap { $0["run"] as? Int }).count, 1, "ein Lauf", file: file, line: line)
        let numbers = changes.compactMap { $0["chg"] as? Int }
        XCTAssertEqual(numbers.count, changes.count, file: file, line: line)
        XCTAssertEqual(Set(numbers), Set(1...(numbers.max() ?? 0)), "fortlaufend, ohne Lücke", file: file, line: line)
        var seen = Set<Int>()
        var previous: Int?
        for n in numbers {
            if n != previous { XCTAssertTrue(seen.insert(n).inserted, "die Zeilen der Änderung \(n) stehen nebeneinander", file: file, line: line) }
            previous = n
        }
        for n in Set(numbers) {
            let kinds = changes.filter { $0["chg"] as? Int == n }.compactMap { $0["t"] as? String }
            XCTAssertTrue(kinds == ["release"] || kinds == ["intent", "effective"] || kinds == ["intent", "ineffective"],
                          "Änderung \(n): \(kinds)", file: file, line: line)
        }
    }
}

/// Schalter „Zeilen dieser Art scheitern“ (wechselbar zur Laufzeit).
private final class FailFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var flag = false
    var value: Bool {
        get { lock.withLock { flag } }
        set { lock.withLock { flag = newValue } }
    }
}
