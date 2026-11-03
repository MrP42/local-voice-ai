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

    func testInboxIgnoresAbandonedCopies() throws {
        let root = tmp.appendingPathComponent("Eingang")
        let inbox = try MeetingInbox(root: root, digest: TestDigest())
        try FileManager.default.createDirectory(at: root.appendingPathComponent(".tmp-abc"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent(UUID().uuidString), withIntermediateDirectories: true)
        XCTAssertTrue(try inbox.pending().isEmpty)
        inbox.sweepAbandoned()
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
        XCTAssertTrue(st.complete)
        XCTAssertEqual(st.gaps, [60...61])
        XCTAssertEqual(try asm.contiguousPrefix(session).map(\.sequence), [0, 1, 2])
        XCTAssertEqual(try asm.consent(session)?.channel, .watch)
        XCTAssertEqual(try Data(contentsOf: asm.audioURL(h2)), Data([2, 7, 7]))

        // Uhr löscht nur bei passender Quittung.
        XCTAssertFalse(outbox.acknowledge(WatchChunkReceipt(sessionId: session, sequence: 0, digest: String(repeating: "f", count: 64))))
        XCTAssertTrue(outbox.acknowledge(WatchChunkReceipt(sessionId: session, sequence: 0, digest: h0.digest)))
        XCTAssertEqual(outbox.pending().map(\.0.sequence), [2])
        XCTAssertEqual(h2.end, 91)
    }
}
