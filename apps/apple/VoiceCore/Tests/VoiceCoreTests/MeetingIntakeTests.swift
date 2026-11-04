import XCTest
@testable import VoiceCore

/// Testprüfsumme ohne CryptoKit (läuft auch unter Linux); 64 Hexzeichen wie SHA-256.
struct TestDigest: ContentDigest {
    func hex(of url: URL) throws -> String {
        var h: UInt64 = 0xcbf29ce484222325
        for b in try Data(contentsOf: url) { h = (h ^ UInt64(b)) &* 0x100000001b3 }
        return String(repeating: String(format: "%016llx", h), count: 4)
    }
}

final class MeetingIntakeTests: XCTestCase {
    private var tmp: URL!
    override func setUpWithError() throws {
        tmp = FileManager.default.temporaryDirectory.appendingPathComponent("intake-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
    }
    override func tearDownWithError() throws { try? FileManager.default.removeItem(at: tmp) }

    private func file(_ name: String, _ bytes: [UInt8]) throws -> URL {
        let url = tmp.appendingPathComponent(name); try Data(bytes).write(to: url); return url
    }
    private func consent(_ channel: ConsentRecord.Channel, _ reason: ConsentRecord.Reason) -> ConsentRecord {
        ConsentRecord(confirmedAt: Date(), channel: channel, reason: reason, scopes: ConsentRecord.Scope.allCases,
                      purpose: "Protokoll", controller: "Amt 12", retentionDays: 90)
    }

    func testInboxDepositIsIdempotentAndNeedsConsent() async throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let memo = try file("Neue Aufnahme 3.m4a", [1, 2, 3, 4])
        XCTAssertThrowsError(try inbox.deposit(memo, from: .shareSheet, consent: consent(.app, .start))) {
            XCTAssertEqual($0 as? IntakeError, .consentMissing)
        }
        let a = try inbox.deposit(memo, from: .shareSheet, consent: consent(.import, .import))
        let b = try inbox.deposit(memo, from: .openIn, consent: consent(.import, .import))
        XCTAssertEqual(a, b)
        XCTAssertEqual(try inbox.pending().map(\.id), [a.id])
        // Kennung aus dem Inhalt: stabil über Neustarts, gleiche Datei = gleiche Kennung.
        XCTAssertEqual(a.id, MeetingInbox.itemId(digest: a.digest))
        // Dieselbe Datei mit anderer Einwilligung (ohne KI): nie still die frühere KI-Freigabe übernehmen.
        let noAI = ConsentRecord(confirmedAt: Date(), channel: .import, reason: .import, scopes: [.recording, .transcription],
                                 purpose: "Protokoll", controller: "Amt 12", retentionDays: 90)
        XCTAssertThrowsError(try inbox.deposit(memo, from: .openIn, consent: noAI)) {
            guard case .conflict = $0 as? IntakeError else { return XCTFail("\($0)") }
        }
        XCTAssertEqual(try Data(contentsOf: inbox.fileURL(a)), Data([1, 2, 3, 4]))
        XCTAssertEqual(a.originalExtension, "m4a")
        XCTAssertThrowsError(try inbox.deposit(try file("x.m4a", []), from: .shareSheet, consent: consent(.import, .import)))

        // Fehlgeschlagene Übernahme: Eintrag bleibt.
        struct Fail: Error {}
        do { _ = try await inbox.take(a) { _, _ in throw Fail() }; XCTFail() } catch is Fail {}
        XCTAssertEqual(try inbox.pending().count, 1)
        let name = try await inbox.take(a) { url, item in try Data(contentsOf: url).count == item.bytes ? item.originalName : "" }
        XCTAssertEqual(name, "Neue Aufnahme 3.m4a")
        XCTAssertTrue(try inbox.pending().isEmpty)
    }

    func testInboxKeepsDisplayNameAndRejectsLinks() throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let f = try file("A1B2C3.m4a", [1, 2, 3, 4])
        let item = try inbox.deposit(f, from: .shareSheet, consent: consent(.import, .import), name: "Teamrunde/03.11.:2026")
        XCTAssertEqual(item.originalName, "Teamrunde03.11.2026.m4a")
        XCTAssertEqual(MeetingInbox.displayName("..", fallback: "x.m4a", ext: "m4a"), "Aufnahme.m4a")
        XCTAssertEqual(MeetingInbox.displayName(nil, fallback: "Sprachmemo.m4a", ext: "m4a"), "Sprachmemo.m4a")
        // Symlink auf eine Datei wird nicht übernommen.
        let link = tmp.appendingPathComponent("link.m4a")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: try file("ziel.m4a", [9, 9]))
        XCTAssertThrowsError(try inbox.deposit(link, from: .openIn, consent: consent(.import, .import))) { XCTAssertEqual($0 as? IntakeError, .invalid) }
        // Keine Staging-Reste nach Fehlern.
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: inbox.root.path).filter { $0.hasPrefix(".tmp-") }
        XCTAssertEqual(leftovers, [])
    }

    func testInboxVerifiesCommittedEntry() throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let item = try inbox.deposit(try file("v.m4a", [4, 3, 2, 1]), from: .openIn, consent: consent(.import, .import))
        XCTAssertTrue(try inbox.verifyCommitted(item))
        // Verändertes Original: nicht intakt, die Quelle darf nicht gelöscht werden.
        try Data([4, 3, 2, 0]).write(to: inbox.fileURL(item))
        XCTAssertFalse(try inbox.verifyCommitted(item))
        // Fehlendes Original: ebenso.
        try FileManager.default.removeItem(at: inbox.fileURL(item))
        XCTAssertFalse(try inbox.verifyCommitted(item))
    }

    func testInboxRepairsDamagedDuplicate() throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let source = try file("r.m4a", [7, 7, 7, 1])
        let terms = consent(.import, .import)
        let item = try inbox.deposit(source, from: .openIn, consent: terms)
        // Beschädigtes Original: erneutes Teilen meldet keinen Erfolg auf kaputtem Stand, sondern repariert.
        try Data([7, 7, 7, 0]).write(to: inbox.fileURL(item))
        let repaired = try inbox.deposit(source, from: .openIn, consent: terms)
        XCTAssertEqual(repaired.id, item.id)
        XCTAssertTrue(try inbox.verifyCommitted(repaired))
        XCTAssertEqual(try Data(contentsOf: inbox.fileURL(repaired)), Data([7, 7, 7, 1]))
        // Unlesbare Metadaten derselben Kennung: ebenso neu anlegen statt am Rename zu scheitern.
        try Data("kaputt".utf8).write(to: inbox.fileURL(item).deletingLastPathComponent().appendingPathComponent("eingang.json"))
        let again = try inbox.deposit(source, from: .openIn, consent: terms)
        XCTAssertEqual(again.id, item.id)
        XCTAssertEqual(try inbox.pending().map(\.id), [item.id])
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
    }

    private func plainConsent() -> ConsentRecord {
        ConsentRecord(confirmedAt: Date(), channel: .import, reason: .import, scopes: [.recording, .transcription],
                      purpose: "Protokoll", controller: "Amt 12", retentionDays: 90)
    }
    private func metadataURL(_ inbox: MeetingInbox, _ item: InboxItem) -> URL {
        inbox.root.appendingPathComponent(item.id.uuidString).appendingPathComponent("eingang.json")
    }
    private func withBytes(_ item: InboxItem, _ bytes: Int) -> InboxItem {
        InboxItem(id: item.id, source: item.source, originalName: item.originalName, originalExtension: item.originalExtension,
                  bytes: bytes, digest: item.digest, createdAt: item.createdAt, consent: item.consent)
    }

    func testInboxRepairsWrongSizeInDecodableMetadata() throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let source = try file("b.m4a", [9, 8, 7, 6])
        let item = try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))
        try JSONEncoder().encode(withBytes(item, 5)).write(to: metadataURL(inbox, item))
        // Abweichende Einwilligung auf dem BESCHÄDIGTEN Bestand: Konflikt, keine Reparatur, nichts verändert.
        XCTAssertThrowsError(try inbox.deposit(source, from: .openIn, consent: plainConsent())) { error in
            guard case IntakeError.conflict = error else { return XCTFail("\(error)") }
        }
        XCTAssertEqual(try JSONDecoder().decode(InboxItem.self, from: Data(contentsOf: metadataURL(inbox, item))).bytes, 5)
        // Gleiche Bedingungen: reparieren statt Konflikt.
        let repaired = try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))
        XCTAssertEqual(repaired.bytes, 4)
        XCTAssertTrue(try inbox.verifyCommitted(repaired))
        XCTAssertEqual(try inbox.pending().count, 1)
    }

    /// Prüfsumme, die neue Reparaturkopien ablehnt: simuliert einen Fehlschlag mitten in der Reparatur.
    struct RepairFailsDigest: ContentDigest {
        func hex(of url: URL) throws -> String {
            url.lastPathComponent.hasPrefix(".neu-") ? String(repeating: "0", count: 64) : try TestDigest().hex(of: url)
        }
    }

    func testInboxRepairFailureKeepsExistingEntry() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let source = try file("f.m4a", [1, 2, 3, 4])
        let item = try MeetingInbox(root: root, digest: TestDigest()).deposit(source, from: .openIn, consent: consent(.import, .import))
        let inbox = try MeetingInbox(root: root, digest: RepairFailsDigest())
        try JSONEncoder().encode(withBytes(item, 9)).write(to: metadataURL(inbox, item))
        let metaBefore = try Data(contentsOf: metadataURL(inbox, item))
        XCTAssertThrowsError(try inbox.deposit(source, from: .openIn, consent: consent(.import, .import)))
        // Altbestand unverändert (Original und Metadaten), keine Reste, Eintragsordner da.
        XCTAssertEqual(try Data(contentsOf: inbox.fileURL(item)), Data([1, 2, 3, 4]))
        XCTAssertEqual(try Data(contentsOf: metadataURL(inbox, item)), metaBefore)
        let dir = root.appendingPathComponent(item.id.uuidString)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted(), ["eingang.json", "original.m4a"])
    }

    /// Prüfsumme, die die Quelle vor dem ersten Hashen verlängert (Datei ändert sich während der Übernahme).
    final class GrowingSourceDigest: ContentDigest, @unchecked Sendable {
        let source: URL; var grown = false
        init(source: URL) { self.source = source }
        func hex(of url: URL) throws -> String {
            if url.standardizedFileURL.path == source.standardizedFileURL.path, !grown {
                grown = true
                let h = try FileHandle(forWritingTo: url); try h.seekToEnd(); try h.write(contentsOf: Data([1, 2, 3])); try h.close()
            }
            return try TestDigest().hex(of: url)
        }
    }

    func testInboxRejectsSourceChangedDuringIntake() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let source = try file("g.m4a", [5, 5, 5, 5])
        let inbox = try MeetingInbox(root: root, digest: GrowingSourceDigest(source: source))
        XCTAssertThrowsError(try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))) {
            XCTAssertEqual($0 as? IntakeError, .digestMismatch)
        }
        // Kein Eintrag mit veralteter Größe, keine Reste; ein neuer Versuch übernimmt den neuen Stand.
        XCTAssertTrue(try inbox.pending().isEmpty)
        XCTAssertFalse(try FileManager.default.contentsOfDirectory(atPath: root.path).contains { $0.hasPrefix(".tmp-") })
        let item = try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))
        XCTAssertEqual(item.bytes, 7)
        XCTAssertTrue(try inbox.verifyCommitted(item))
    }

    /// Prüfsumme, die die Quelle NACH dem ersten Hashen bei gleicher Größe überschreibt.
    final class RewritingSourceDigest: ContentDigest, @unchecked Sendable {
        let source: URL; var rewritten = false
        init(source: URL) { self.source = source }
        func hex(of url: URL) throws -> String {
            let hex = try TestDigest().hex(of: url)
            if url.standardizedFileURL.path == source.standardizedFileURL.path, !rewritten {
                rewritten = true
                try Data([9, 9, 9, 9]).write(to: url)
            }
            return hex
        }
    }

    func testInboxRejectsSameSizeChangeDuringIntake() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let source = try file("h.m4a", [5, 5, 5, 5])
        let inbox = try MeetingInbox(root: root, digest: RewritingSourceDigest(source: source))
        XCTAssertThrowsError(try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))) {
            XCTAssertEqual($0 as? IntakeError, .digestMismatch)
        }
        XCTAssertTrue(try inbox.pending().isEmpty)
        // Staging vollständig aufgeräumt; nur die (leere) Sperrdatei bleibt.
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: root.path).allSatisfy { $0.hasPrefix(".lock-") })
    }

    struct Crash: Error {}

    func testInboxRepairThrowingAtEveryStepKeepsReadableEntry() throws {
        let steps: [MeetingInbox.RepairStep] = [.afterOriginal, .beforeMetadata, .afterMetadata]
        for step in steps {
            let root = tmp.appendingPathComponent("Eingang-\(step)")
            let content: [UInt8] = [4, 2, 4, 2]
            let item = try MeetingInbox(root: root, digest: TestDigest()).deposit(try file("s.m4a", content), from: .openIn, consent: consent(.import, .import))
            let dir = root.appendingPathComponent(item.id.uuidString)
            try Data([0, 0, 0, 0]).write(to: dir.appendingPathComponent("original.m4a"))
            // Reparatur mit Endungswechsel, Ausfall am Schritt `step`.
            var faulty = try MeetingInbox(root: root, digest: TestDigest())
            faulty.fault = { if $0 == step { throw Crash() } }
            let reshared = try file("s.qta", content)
            XCTAssertThrowsError(try faulty.deposit(reshared, from: .shareSheet, consent: consent(.import, .import)))
            // Wieder geöffneter Bestand: nie leer, nie ein halber Zustand ohne Original.
            let reopened = try MeetingInbox(root: root, digest: TestDigest())
            let names = try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted()
            XCTAssertFalse(names.contains { $0.hasPrefix(".neu-") }, "\(step)")
            XCTAssertEqual(names, ["eingang.json", "original.m4a", "original.qta"], "\(step)")
            let seen = try XCTUnwrap(reopened.pending().first, "\(step)")
            if step == .afterMetadata {
                XCTAssertEqual(seen.originalExtension, "qta"); XCTAssertTrue(try reopened.verifyCommitted(seen))
            } else {
                XCTAssertEqual(seen.originalExtension, "m4a"); XCTAssertFalse(try reopened.verifyCommitted(seen))
            }
            // Erneutes Teilen schließt jede Variante sauber ab.
            let done = try reopened.deposit(reshared, from: .shareSheet, consent: consent(.import, .import))
            XCTAssertTrue(try reopened.verifyCommitted(done), "\(step)")
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted(), ["eingang.json", "original.qta"], "\(step)")
        }
    }

    /// Prozessabbruch (kein defer, kein Aufräumen) an jeder Stelle der Reparatur, als Zustand auf
    /// der Platte nachgestellt: halbe `.neu-`-Kopie, beide Originale mit alten oder neuen Metadaten.
    func testInboxRepairRecoversFromProcessAbortStates() throws {
        let content: [UInt8] = [1, 3, 5, 7]
        for state in 0..<3 {
            let root = tmp.appendingPathComponent("Abbruch-\(state)")
            let inbox = try MeetingInbox(root: root, digest: TestDigest())
            let item = try inbox.deposit(try file("a.m4a", content), from: .openIn, consent: consent(.import, .import))
            let dir = root.appendingPathComponent(item.id.uuidString)
            try Data([0, 0, 0, 0]).write(to: dir.appendingPathComponent("original.m4a"))
            switch state {
            case 0: // Abbruch beim Kopieren: halbe Kopie liegt im Eintrag.
                try Data([1, 3]).write(to: dir.appendingPathComponent(".neu-HALB"))
            case 1: // Abbruch nach dem Original-Rename: neues Original, alte Metadaten.
                try Data(content).write(to: dir.appendingPathComponent("original.qta"))
            default: // Abbruch nach dem Metadatenersatz: neuer Stand, altes Original liegt noch.
                try Data(content).write(to: dir.appendingPathComponent("original.qta"))
                let next = InboxItem(id: item.id, source: .shareSheet, originalName: "a.qta", originalExtension: "qta", bytes: 4,
                                     digest: item.digest, createdAt: item.createdAt, consent: item.consent)
                try JSONEncoder().encode(next).write(to: dir.appendingPathComponent("eingang.json"))
            }
            let reopened = try MeetingInbox(root: root, digest: TestDigest())
            XCTAssertEqual(try reopened.pending().count, 1, "\(state)")
            let done = try reopened.deposit(try file("a.qta", content), from: .shareSheet, consent: consent(.import, .import))
            XCTAssertTrue(try reopened.verifyCommitted(done), "\(state)")
            XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted(), ["eingang.json", "original.qta"], "\(state)")
        }
    }

    func testInboxRepairResumesAfterAbortAndSwitchesExtension() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let inbox = try MeetingInbox(root: root, digest: TestDigest())
        let item = try inbox.deposit(try file("x.m4a", [3, 3, 3, 3]), from: .openIn, consent: consent(.import, .import))
        let dir = root.appendingPathComponent(item.id.uuidString)
        // Abbruch mitten in einer früheren Reparatur: verwaiste `.neu-…`-Kopie, kaputte Metadaten.
        try Data([0]).write(to: dir.appendingPathComponent(".neu-ALT"))
        try Data("kaputt".utf8).write(to: dir.appendingPathComponent("eingang.json"))
        XCTAssertTrue(try inbox.pending().isEmpty)
        // Dieselbe Aufnahme erneut, diesmal als .qta: Reparatur wechselt die Endung sauber.
        let repaired = try inbox.deposit(try file("x.qta", [3, 3, 3, 3]), from: .shareSheet, consent: consent(.import, .import))
        XCTAssertEqual(repaired.id, item.id)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted(), ["eingang.json", "original.qta"])
        XCTAssertTrue(try inbox.verifyCommitted(repaired))
    }

    func testTakeImportsFromPrivateSnapshot() async throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let source = try file("t.m4a", [6, 6, 6, 6])
        let item = try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))
        let read: Data = try await inbox.take(item) { url, current in
            // Während des Imports beschädigt und repariert die Extension den Eintrag (andere Endung).
            try Data([0, 0, 0, 0]).write(to: inbox.fileURL(item))
            _ = try inbox.deposit(try self.file("t.qta", [6, 6, 6, 6]), from: .shareSheet, consent: self.consent(.import, .import))
            XCTAssertEqual(current.id, item.id)
            return try Data(contentsOf: url)
        }
        XCTAssertEqual(read, Data([6, 6, 6, 6]))
        // Gleicher Inhalt, gleiche Bedingungen: nach dem Import entfernt; keine Momentaufnahme bleibt.
        XCTAssertTrue(try inbox.pending().isEmpty)
        XCTAssertFalse(try FileManager.default.contentsOfDirectory(atPath: inbox.root.path).contains { $0.hasPrefix(".take-") || $0.hasPrefix(".tmp-") })
    }

    func testTakeKeepsEntryReplacedWithOtherConsent() async throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let source = try file("k.m4a", [8, 1, 8, 1])
        let item = try inbox.deposit(source, from: .openIn, consent: consent(.import, .import))
        _ = try await inbox.take(item) { _, _ in
            // Metadaten gehen kaputt, die Extension legt den Eintrag mit anderer Einwilligung neu an.
            try Data("kaputt".utf8).write(to: self.metadataURL(inbox, item))
            _ = try inbox.deposit(source, from: .shareSheet, consent: self.plainConsent())
        }
        let left = try XCTUnwrap(inbox.pending().first)
        XCTAssertTrue(left.consent.sameTerms(as: plainConsent()))
    }

    func testTakeAndDiscardNeverWaitForTheLock() async throws {
        let inbox = try MeetingInbox(root: tmp.appendingPathComponent("Eingang"), digest: TestDigest())
        let item = try inbox.deposit(try file("l.m4a", [2, 4, 6, 8]), from: .openIn, consent: consent(.import, .import))
        let done = DispatchSemaphore(value: 0)
        // Semaphore-wait ist in async-Kontexten nicht erlaubt: auf die Sperre per Continuation warten.
        defer { done.signal() }
        // Kann die Sperre nicht genommen werden, wird die Continuation trotzdem fortgesetzt (Test scheitert statt zu hängen).
        let locked = await withCheckedContinuation { (held: CheckedContinuation<Bool, Never>) in
            Thread.detachNewThread {
                do { try inbox.locked(item.id) { held.resume(returning: true); done.wait() } }
                catch { held.resume(returning: false) }
            }
        }
        guard locked else { return XCTFail("Sperre nicht genommen") }
        do { _ = try await inbox.take(item) { _, _ in XCTFail("darf nicht importieren") }; XCTFail("busy erwartet") }
        catch IntakeError.busy {}
        XCTAssertThrowsError(try inbox.discard(item)) { XCTAssertEqual($0 as? IntakeError, .busy) }
        XCTAssertEqual(try inbox.pending().map(\.id), [item.id])
    }

    /// Langsame Prüfsumme: erzwingt Überlappung paralleler Übernahmen.
    struct SlowDigest: ContentDigest {
        func hex(of url: URL) throws -> String { usleep(5_000); return try TestDigest().hex(of: url) }
    }

    func testInboxSerializesConcurrentDeposits() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let source = try file("c.m4a", Array(repeating: 5, count: 64 * 1024))
        let ai = consent(.import, .import), plain = plainConsent()
        // App und Extension (getrennte Instanzen, eigene Sperr-Deskriptoren) gleichzeitig, mit
        // abweichender Einwilligung: genau einer gewinnt, sein Eintrag bleibt intakt und unverändert.
        final class Results: @unchecked Sendable { let lock = NSLock(); var ok: [InboxItem] = []; var conflicts = 0 }
        let results = Results()
        DispatchQueue.concurrentPerform(iterations: 8) { i in
            do {
                let inbox = try MeetingInbox(root: root, digest: SlowDigest())
                let item = try inbox.deposit(source, from: .shareSheet, consent: i.isMultiple(of: 2) ? ai : plain)
                results.lock.lock(); results.ok.append(item); results.lock.unlock()
            } catch IntakeError.conflict {
                results.lock.lock(); results.conflicts += 1; results.lock.unlock()
            } catch { XCTFail("unerwartet: \(error)") }
        }
        let inbox = try MeetingInbox(root: root, digest: TestDigest())
        let stored = try XCTUnwrap(inbox.pending().first)
        XCTAssertEqual(try inbox.pending().count, 1)
        XCTAssertTrue(try inbox.verifyCommitted(stored))
        XCTAssertEqual(results.ok.count, 4)
        XCTAssertEqual(results.conflicts, 4)
        XCTAssertTrue(results.ok.allSatisfy { $0.consent.sameTerms(as: stored.consent) && $0.id == stored.id })
        XCTAssertFalse(try FileManager.default.contentsOfDirectory(atPath: root.path).contains { $0.hasPrefix(".tmp-") })
    }

    func testInboxSerializesConcurrentRepairs() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let source = try file("p.m4a", Array(repeating: 7, count: 32 * 1024))
        let terms = consent(.import, .import)
        let first = try MeetingInbox(root: root, digest: TestDigest())
        let item = try first.deposit(source, from: .openIn, consent: terms)
        try Data([0]).write(to: first.fileURL(item))
        final class Count: @unchecked Sendable { let lock = NSLock(); var ok = 0 }
        let count = Count()
        DispatchQueue.concurrentPerform(iterations: 6) { _ in
            do {
                _ = try MeetingInbox(root: root, digest: SlowDigest()).deposit(source, from: .shareSheet, consent: terms)
                count.lock.lock(); count.ok += 1; count.lock.unlock()
            } catch { XCTFail("unerwartet: \(error)") }
        }
        XCTAssertEqual(count.ok, 6)
        let stored = try XCTUnwrap(first.pending().first)
        XCTAssertTrue(try first.verifyCommitted(stored))
        let dir = root.appendingPathComponent(item.id.uuidString)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted(), ["eingang.json", "original.m4a"])
    }

    func testInboxCreatesWholeDirectoryChain() throws {
        let deep = tmp.appendingPathComponent("a/b/Eingang")
        _ = try MeetingInbox(root: deep, digest: TestDigest())
        var isDir: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: deep.path, isDirectory: &isDir) && isDir.boolValue)
    }

    func testInboxIgnoresAbandonedCopies() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let inbox = try MeetingInbox(root: root, digest: TestDigest())
        try FileManager.default.createDirectory(at: root.appendingPathComponent(".tmp-abc"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent(".take-abc"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent(UUID().uuidString), withIntermediateDirectories: true)
        XCTAssertTrue(try inbox.pending().isEmpty)
        // Eine frische Staging-Kopie gehört womöglich gerade der Extension: nicht anfassen.
        inbox.sweepAbandoned()
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent(".tmp-abc").path))
        inbox.sweepAbandoned(now: Date().addingTimeInterval(MeetingInbox.abandonedAfter + 60))
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent(".tmp-abc").path))
        // Momentaufnahme eines abgebrochenen Imports: ebenso nach Ablauf weggeräumt.
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent(".take-abc").path))
    }

    func testWatchChunksAreDurableIdempotentAndGapAware() throws {
        let asm = try WatchRecordingAssembler(root: tmp.appendingPathComponent("Uhr"), digest: TestDigest())
        let outbox = try WatchChunkOutbox(root: tmp.appendingPathComponent("Ausgang"))
        let session = UUID()
        let d = TestDigest()
        func chunk(_ seq: Int, first: Int64, last: Bool = false, consent c: ConsentRecord? = nil) throws -> (WatchChunkHeader, URL) {
            let f = try file("c\(seq)-\(first).m4a", [UInt8(seq), 7, 7])
            return (WatchChunkHeader(sessionId: session, sequence: seq, firstSample: first, sampleCount: 480_000,
                                     digest: try d.hex(of: f), isLast: last, consent: c), f)
        }
        // Überlaufender Kopf wird vor jeder weiteren Prüfung abgelehnt.
        let huge = WatchChunkHeader(sessionId: session, sequence: 0, firstSample: .max - 1, sampleCount: 10, digest: String(repeating: "0", count: 64))
        XCTAssertNil(huge.endSample)
        XCTAssertThrowsError(try asm.receive(huge, file: try file("huge.m4a", [1]))) { XCTAssertEqual($0 as? IntakeError, .invalid) }
        // Stück 0 ohne Einwilligung wird abgelehnt.
        let (h0bad, f0bad) = try chunk(0, first: 0)
        XCTAssertThrowsError(try asm.receive(h0bad, file: f0bad)) { XCTAssertEqual($0 as? IntakeError, .consentMissing) }

        let (h0, f0) = try chunk(0, first: 0, consent: consent(.watch, .start))
        let (h2, f2) = try chunk(2, first: 960_000 + 16_000, last: true)   // 1 s Lücke vor Stück 2
        // Über den Ausgang der Uhr, opportunistisch und außer der Reihe.
        try outbox.enqueue(h0, file: f0); try outbox.enqueue(h2, file: f2)
        XCTAssertEqual(outbox.pending().map(\.0.sequence), [0, 2])
        for (h, url) in outbox.pending().reversed() { _ = try asm.receive(h, file: url) }
        var st = try asm.status(session)
        XCTAssertEqual(st.received, [0, 2]); XCTAssertEqual(st.missingSequences, [1]); XCTAssertFalse(st.complete)
        XCTAssertEqual(try asm.contiguousPrefix(session).map(\.sequence), [0])

        let (h1, f1) = try chunk(1, first: 480_000)
        let r1 = try asm.receive(h1, file: f1)
        XCTAssertEqual(try asm.receive(h1, file: f1), r1, "Duplikat: gleiche Quittung")
        // Gleiches Stück, anderer Inhalt: Konflikt.
        let other = try file("anders.m4a", [9, 9, 9])
        let h1b = WatchChunkHeader(sessionId: session, sequence: 1, firstSample: 480_000, sampleCount: 480_000, digest: try d.hex(of: other))
        XCTAssertThrowsError(try asm.receive(h1b, file: other))
        // Falsche Prüfsumme
        let h1c = WatchChunkHeader(sessionId: UUID(), sequence: 0, firstSample: 0, sampleCount: 1, digest: String(repeating: "0", count: 64), consent: consent(.watch, .start))
        XCTAssertThrowsError(try asm.receive(h1c, file: other)) { XCTAssertEqual($0 as? IntakeError, .digestMismatch) }

        st = try asm.status(session)
        // Alle Stücke da, aber 1 s Samplelücke: nicht vollständig.
        XCTAssertTrue(st.missingSequences.isEmpty && st.lastReceived && st.startsAtZero && st.uniformRate && st.singleFinalChunk)
        XCTAssertFalse(st.complete)
        XCTAssertEqual(st.gaps, [60...61])
        XCTAssertEqual(try asm.contiguousPrefix(session).map(\.sequence), [0, 1, 2])
        XCTAssertEqual(try asm.consent(session)?.channel, .watch)
        XCTAssertEqual(try Data(contentsOf: asm.audioURL(h2)), Data([2, 7, 7]))

        // Lückenlose Sitzung mit einem Abschlussstück ist vollständig.
        let clean = UUID()
        func cchunk(_ seq: Int, first: Int64, last: Bool = false, consent c: ConsentRecord? = nil) throws -> (WatchChunkHeader, URL) {
            let f = try file("k\(seq).m4a", [UInt8(seq), 3])
            return (WatchChunkHeader(sessionId: clean, sequence: seq, firstSample: first, sampleCount: 480_000, digest: try d.hex(of: f), isLast: last, consent: c), f)
        }
        for (h, f) in [try cchunk(0, first: 0, consent: consent(.watch, .start)), try cchunk(1, first: 480_000, last: true)] { _ = try asm.receive(h, file: f) }
        XCTAssertTrue(try asm.status(clean).complete)

        // Dauerhaftigkeit vor der Quittung: je Versuch erst Sitzungsordner, dann Wurzel.
        let fresh = UUID()
        let sessionDir = asm.root.appendingPathComponent(fresh.uuidString)
        let log = SyncLog()
        log.failRoot = 1
        var probe = asm
        probe.syncDirectory = { url in
            log.add(url)
            if url.standardizedFileURL.path == asm.root.standardizedFileURL.path, log.take(root: true) { throw IntakeError.invalid }
            if url.standardizedFileURL.path == sessionDir.standardizedFileURL.path, log.take(root: false) { throw IntakeError.invalid }
        }
        func lastTwo() -> [String] { Array(log.all.suffix(2)) }
        let expected = [sessionDir, asm.root].map(\.standardizedFileURL.path)
        let f0x = try file("x0.m4a", [5, 5])
        let h0x = WatchChunkHeader(sessionId: fresh, sequence: 0, firstSample: 0, sampleCount: 480_000, digest: try d.hex(of: f0x), consent: consent(.watch, .start))
        // a) Wurzel-fsync scheitert: keine Quittung, Ordner bleibt.
        XCTAssertThrowsError(try probe.receive(h0x, file: f0x))
        XCTAssertEqual(lastTwo(), expected)
        XCTAssertTrue(FileManager.default.fileExists(atPath: sessionDir.path))
        // b) Neuschreiben bei schon vorhandenem Ordner (Kopf fehlt, z. B. Absturz davor).
        try FileManager.default.removeItem(at: sessionDir.appendingPathComponent("000000.json"))
        log.reset()
        _ = try probe.receive(h0x, file: f0x)
        XCTAssertEqual(lastTwo(), expected, "Neuschreiben: Ordner vor Wurzel")
        // c) Duplikat: ebenfalls Ordner vor Wurzel.
        log.reset()
        _ = try probe.receive(h0x, file: f0x)
        XCTAssertEqual(log.all, expected, "Duplikat: Ordner vor Wurzel")
        // d) Sitzungsordner-fsync scheitert: Fehler wird weitergegeben, Wurzel wird nicht mehr angefasst.
        log.failFolder = 1; log.reset()
        XCTAssertThrowsError(try probe.receive(h0x, file: f0x)) { XCTAssertEqual($0 as? IntakeError, .invalid) }
        XCTAssertEqual(log.all, [sessionDir.standardizedFileURL.path])

        // Uhr löscht nur bei passender Quittung.
        XCTAssertFalse(outbox.acknowledge(WatchChunkReceipt(sessionId: session, sequence: 0, digest: String(repeating: "f", count: 64))))
        XCTAssertTrue(outbox.acknowledge(WatchChunkReceipt(sessionId: session, sequence: 0, digest: h0.digest)))
        XCTAssertEqual(outbox.pending().map(\.0.sequence), [2])
        XCTAssertEqual(h2.end, 91)
    }
}

final class SyncLog: @unchecked Sendable {
    private let lock = NSLock()
    private var urls: [String] = []
    func add(_ u: URL) { lock.lock(); urls.append(u.standardizedFileURL.path); lock.unlock() }
    var failRoot = 0, failFolder = 0
    func take(root: Bool) -> Bool {
        lock.lock(); defer { lock.unlock() }
        if root { guard failRoot > 0 else { return false }; failRoot -= 1 } else { guard failFolder > 0 else { return false }; failFolder -= 1 }
        return true
    }
    func reset() { lock.lock(); urls = []; lock.unlock() }
    var all: [String] { lock.lock(); defer { lock.unlock() }; return urls }
}
