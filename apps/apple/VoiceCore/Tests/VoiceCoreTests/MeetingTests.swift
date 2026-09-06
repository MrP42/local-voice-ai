import XCTest
@testable import VoiceCore

final class MeetingTests: XCTestCase {
    func testMixedAudioNeverInventsSpeakerShares() throws {
        let segments = [MeetingSegment(index: 0, start: 0, end: 4, text: "Hallo"), MeetingSegment(index: 1, start: 4, end: 9, text: "Guten Tag")]
        XCTAssertTrue(MeetingExport.speakerShares(segments).isEmpty)
    }
    func testAssignedSharesUseTimedIntervalsAndKeepUnknownCoverage() throws {
        let segments = [MeetingSegment(index: 0, start: 0, end: 6, text: "A", speaker: "Anna"), MeetingSegment(index: 1, start: 3, end: 8, text: "A", speaker: "Anna"), MeetingSegment(index: 2, start: 8, end: 10, text: "B", speaker: "Ben"), MeetingSegment(index: 3, start: 10, end: 12, text: "?")]
        let shares = MeetingExport.speakerShares(segments)
        XCTAssertEqual(shares.first(where: { $0.name == "Anna" })?.seconds, 8)
        XCTAssertEqual(shares.first(where: { $0.name == "Nicht zugeordnet" })?.seconds, 2)
        XCTAssertEqual(shares.reduce(0) { $0 + $1.percent }, 100, accuracy: 0.001)
    }
    func testOriginalAndProgressSurviveReopen() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("source.wav"); try Data([1,2,3]).write(to: source)
        let folder = root.appendingPathComponent("archive")
        let store = try MeetingArchive(root: folder, quota: 100)
        let doc = try await store.importMedia(source, duration: 70)
        try await store.appendChunk(doc.id, offset: 0, nextOffset: 30, segments: [MeetingSegment(index: 0, start: 1, end: 3, text: "Gespeichert")])
        let reopened = try MeetingArchive(root: folder, quota: 100)
        let loaded = try await reopened.load(doc.id)
        XCTAssertEqual(loaded.nextOffset, 30)
        XCTAssertEqual(loaded.segments.first?.text, "Gespeichert")
        let file = try await reopened.originalURL(doc.id)
        XCTAssertEqual(try Data(contentsOf: file), Data([1,2,3]))
        do { try await reopened.appendChunk(doc.id, offset: 0, nextOffset: 30, segments: []); XCTFail("Stale chunk must not replace persisted progress") } catch {}
    }
    func testQuotaFailurePreservesExistingOriginal() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("source.wav"); try Data([1,2,3]).write(to: source)
        let store = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 5)
        let first = try await store.importMedia(source, duration: 2)
        do { _ = try await store.importMedia(source, duration: 2); XCTFail("Must reject over quota") } catch {}
        let file = try await store.originalURL(first.id)
        XCTAssertEqual(try Data(contentsOf: file), Data([1,2,3]))
    }
    func testSubtitleAndMarkupExportsPreserveTextWithoutExecutingMarkup() throws {
        let segment = MeetingSegment(index: 0, start: 1.25, end: 3.5, text: "<script> & Hallo", speaker: "A")
        XCTAssertTrue(MeetingExport.srt([segment]).contains("00:00:01,250 --> 00:00:03,500"))
        XCTAssertTrue(MeetingExport.html("<script> & Hallo").contains("&lt;script&gt; &amp; Hallo"))
    }
    func testMinutesRequireCompleteSchemaAndRejectUnboundedText() throws {
        XCTAssertThrowsError(try MeetingMinutes.decode(Data("{\"summary\":\"fake\"}".utf8)))
        let report = MeetingMinutes(summary: "Kurz", scope: "Thema", decisions: [], tasks: [], next_steps: [], follow_ups: [], open_questions: [])
        XCTAssertEqual(try MeetingMinutes.decode(JSONEncoder().encode(report)).summary, "Kurz")
        var oversized = report; oversized.summary = String(repeating: "x", count: 8193)
        XCTAssertThrowsError(try MeetingMinutes.decode(JSONEncoder().encode(oversized)))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(report)) as? [String: Any])
        object["tasks"] = [["text": "Aufgabe"]]
        XCTAssertThrowsError(try MeetingMinutes.decode(JSONSerialization.data(withJSONObject: object)))
    }
}
