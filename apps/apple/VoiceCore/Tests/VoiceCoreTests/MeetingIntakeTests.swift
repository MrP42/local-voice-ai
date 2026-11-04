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
        try FileManager.default.createDirectory(at: root.appendingPathComponent(UUID().uuidString), withIntermediateDirectories: true)
        XCTAssertTrue(try inbox.pending().isEmpty)
        // Eine frische Staging-Kopie gehört womöglich gerade der Extension: nicht anfassen.
        inbox.sweepAbandoned()
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent(".tmp-abc").path))
        inbox.sweepAbandoned(now: Date().addingTimeInterval(MeetingInbox.abandonedAfter + 60))
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent(".tmp-abc").path))
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
