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
    func testMinutesRejectInventedDatesAndUnquotedFacts() throws {
        let source = "Wir starten im Oktober. Anna erstellt bis Freitag den Entwurf. Die Kosten sind noch offen."
        let invented = Data(#"{"summary":"Wir starten im Oktober.","scope":"Oktober","decisions":[],"tasks":[{"text":"Anna erstellt bis Freitag den Entwurf.","assignee":"Anna","due":"10. Oktober"}],"next_steps":[],"follow_ups":[],"open_questions":[]}"#.utf8)
        let report = try MeetingMinutes.decode(invented)
        XCTAssertThrowsError(try report.validateEvidence(in: source))
        let grounded = Data(#"{"summary":"Wir starten im Oktober.","scope":"Oktober","decisions":[],"tasks":[{"text":"Anna erstellt bis Freitag den Entwurf.","assignee":"Anna","due":"Freitag"}],"next_steps":[],"follow_ups":[],"open_questions":[{"text":"Die Kosten sind noch offen."}]}"#.utf8)
        try MeetingMinutes.decode(grounded).validateEvidence(in: source)
        var unsupported = try MeetingMinutes.decode(grounded); unsupported.summary = "Ben bezahlt die Rechnung."
        XCTAssertThrowsError(try unsupported.validateEvidence(in: source))
    }
    func testLocalSelectionUsesOriginalQuotesAndDiscardsUnsupportedAttribution() throws {
        let source = ["Wir starten im Oktober.", "Anna erstellt bis Freitag den Entwurf.", "Die Kosten sind offen."]
        let selection = Data(#"{"summary":[0,2],"decisions":[0],"tasks":[{"index":1,"assignee":"Ben","due":"10. Oktober"}],"next_steps":[],"follow_ups":[],"open_questions":[2]}"#.utf8)
        let report = try MeetingMinutes.fromSelection(selection, source: source)
        XCTAssertEqual(report.summary, source[0] + "\n" + source[2])
        XCTAssertEqual(report.tasks.first?.text, source[1])
        XCTAssertNil(report.tasks.first?.assignee)
        XCTAssertNil(report.tasks.first?.due)
        let invalid = Data(#"{"summary":[99],"decisions":[],"tasks":[],"next_steps":[],"follow_ups":[],"open_questions":[]}"#.utf8)
        XCTAssertThrowsError(try MeetingMinutes.fromSelection(invalid, source: source))
        let fragments = Data(#"{"summary":[0],"decisions":[],"tasks":[{"index":0,"assignee":"Anna","due":"10"}],"next_steps":[],"follow_ups":[],"open_questions":[]}"#.utf8)
        let safe = try MeetingMinutes.fromSelection(fragments, source: ["Annabel prüft den Bericht von 2010."])
        XCTAssertNil(safe.tasks.first?.assignee)
        XCTAssertNil(safe.tasks.first?.due)
    }
    func testGermanMinutesKeepQuestionsAndCollectiveNextStepsOutOfTasks() throws {
        let source = ["Wir besprechen heute die Webseite.", "Wir entscheiden uns für Oktober.", "Anna erstellt bis Freitag den Entwurf.", "Nächste Woche vergleichen wir die Ergebnisse.", "Die Kosten sind noch offen."]
        let selection = Data(#"{"summary":[3],"decisions":[1],"tasks":[{"index":2,"assignee":"Anna","due":"Freitag"},{"index":3,"assignee":null,"due":null},{"index":4,"assignee":null,"due":null}],"next_steps":[],"follow_ups":[],"open_questions":[0,4]}"#.utf8)
        let report = try MeetingMinutes.fromSelection(selection, source: source)
        XCTAssertEqual(report.tasks.map(\.text), [source[2]])
        XCTAssertEqual(report.next_steps.map(\.text), [source[3]])
        XCTAssertEqual(report.open_questions.map(\.text), [source[4]])
        XCTAssertEqual(report.summary, source[1] + "\n" + source[2])
        XCTAssertEqual(report.scope, source[0])
    }
    func testRecommendationsKeepTheirSourceReason() throws {
        let selection = Data(#"{"summary":[0],"decisions":[],"tasks":[],"next_steps":[],"follow_ups":[{"index":0,"text":"Kosten vor dem Start klären."}],"open_questions":[0]}"#.utf8)
        let report = try MeetingMinutes.fromSelection(selection, source: ["Die Kosten sind offen."])
        XCTAssertEqual(report.follow_ups.first?.text, "Kosten vor dem Start klären.")
        XCTAssertEqual(report.follow_ups.first?.reason, "Die Kosten sind offen.")
        try report.validateEvidence(in: "Die Kosten sind offen.")
    }
    func testSummaryProgressIsDurableAndCannotSkipTranscription() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("source.wav"); try Data([1,2,3]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        let doc = try await archive.importMedia(source, duration: 2)
        let report = MeetingMinutes(summary: "Inhalt", scope: "Inhalt", decisions: [], tasks: [], next_steps: [], follow_ups: [], open_questions: [])
        do { try await archive.appendMinutes(doc.id, from: 0, through: 1, minutes: report); XCTFail("Requires completed transcript") } catch {}
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 2, segments: [MeetingSegment(index: 0, start: 0, end: 2, text: "Inhalt")])
        try await archive.appendMinutes(doc.id, from: 0, through: 1, minutes: report)
        let loaded = try await archive.load(doc.id)
        XCTAssertEqual(loaded.summarizedSegments, 1)
        XCTAssertEqual(loaded.summaryParts?.first?.summary, "Inhalt")
        do { try await archive.appendMinutes(doc.id, from: 0, through: 1, minutes: report); XCTFail("Stale summary must not duplicate content") } catch {}
        try await archive.finishMinutes(doc.id)
        let finished = try await archive.load(doc.id)
        XCTAssertEqual(finished.minutes?.summary, "Inhalt")
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
