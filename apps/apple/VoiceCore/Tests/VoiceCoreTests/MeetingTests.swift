import XCTest
@testable import VoiceCore

final class MeetingTests: XCTestCase {
    func testPipelineStepsFollowUpsAndExternalConsentSurviveArchiveRoundtrip() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("synthetic.wav")
        try Data([1, 2, 3]).write(to: source)
        let folder = root.appendingPathComponent("archive")
        let archive = try MeetingArchive(root: folder, quota: 100)
        let consent = ConsentLog(records: [.init(confirmedAt: Date(timeIntervalSince1970: 0), channel: .import,
            reason: .start, scopes: [.recording, .transcription, .aiAnalysis],
            purpose: "Test", controller: "Test", retentionDays: 1)])
        let doc = try await archive.importMedia(source, duration: 1,
            live: .init(consent: consent, live: nil, segments: [], notes: nil))
        let quote = "Ada prüft morgen den Test."
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 1,
            segments: [.init(index: 0, start: 0, end: 1, text: quote)])
        let loaded = try await archive.load(doc.id)
        let raw = #"{"items":[{"kind":"next_steps","text":"Ada prüft morgen den Test.","quote":"Ada prüft morgen den Test.","assignee":"Ada","evidence":["final:0"]},{"kind":"follow_ups","text":"prüft morgen den Test","quote":"Ada prüft morgen den Test.","evidence":["final:0"]}]}"#
        let result = try MinutesResult.decode(raw, segments: loaded.minutesSegments())
        let route = ModelRoute.endpoint(id: UUID())
        let state = ModelAccessState(policy: .open, targets: [route: .init(vendor: "Test", url: URL(string: "https://example.test/v1/chat/completions")!)])
        let (scope, _) = try await state.consentScope(route: route, purpose: .meetingMinutes,
            subjectId: doc.id.uuidString, dataCategories: ["Transkript"])!
        let record = ModelConsentRecord(scope: scope, grantedAt: Date(timeIntervalSince1970: 1))
        do { try await archive.updateModelConsent(UUID(), scope: scope, record: record); XCTFail("wrong subject") }
        catch { if case VoiceError.invalid = error {} else { XCTFail("unexpected error") } }
        try await archive.updateModelConsent(doc.id, scope: scope, record: record)
        let event = ProcessingEvent(operation: "Test", model: "Synthetic endpoint", completedAt: Date(), durationMS: 0, isAI: true, route: route.key)
        var forged = result; forged.items[0].assignee = "Bob"
        do { try await archive.savePipelineMinutes(doc.id, result: forged, event: event); XCTFail("unquoted owner") }
        catch { if case VoiceError.invalid = error {} else { XCTFail("unexpected error") } }
        try await archive.savePipelineMinutes(doc.id, result: result, event: event)
        let reopened = try MeetingArchive(root: folder, quota: 100)
        let saved = try await reopened.load(doc.id)
        XCTAssertEqual(saved.modelConsents, [record])
        XCTAssertEqual(saved.minutes?.next_steps.first?.owner, "Ada")
        XCTAssertEqual(saved.minutes?.follow_ups.first?.reason, quote)
        XCTAssertEqual(saved.minutes?.quotes?["next_steps.0"], quote)
        XCTAssertEqual(saved.minutes?.evidence?["follow_ups.0"]?.first?.segmentId, "final:0")
        XCTAssertEqual(saved.processingEvents?.last?.route, route.key)
        XCTAssertEqual(saved.processingEvents?.last?.model, "Synthetic endpoint")
        let (otherScope, _) = try await state.consentScope(route: route, purpose: .meetingSummary,
            subjectId: doc.id.uuidString, dataCategories: ["Transkript"])!
        let other = ModelConsentRecord(scope: otherScope, grantedAt: Date(timeIntervalSince1970: 2))
        async let grant: Void = try await archive.updateModelConsent(doc.id, scope: otherScope, record: other)
        async let revoke: Void = try await archive.updateModelConsent(doc.id, scope: scope, record: nil)
        _ = try await (grant, revoke)
        let concurrent = try await archive.load(doc.id)
        XCTAssertEqual(concurrent.modelConsents, [other])
        try await archive.updateModelConsent(doc.id, scope: otherScope, record: nil)
        let revoked = try await archive.load(doc.id)
        XCTAssertEqual(revoked.modelConsents, [])
    }

    func testPipelineSavesOnlyQuotedTaskFieldsAndRechecksBeforeSaving() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("source.wav")
        try Data([1, 2, 3]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        let consent = ConsentLog(records: [.init(confirmedAt: Date(timeIntervalSince1970: 0), channel: .import,
            reason: .start, scopes: [.recording, .transcription, .aiAnalysis],
            purpose: "Test", controller: "Test", retentionDays: 1)])
        let doc = try await archive.importMedia(source, duration: 1,
            live: .init(consent: consent, live: nil, segments: [], notes: nil))
        let quote = "Mats schickt bis Dienstag die Unterlagen."
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 1,
            segments: [.init(index: 0, start: 0, end: 1, text: quote)])
        let loaded = try await archive.load(doc.id)
        let raw = #"{"items":[{"kind":"task","text":"Mats schickt bis Dienstag die Unterlagen.","quote":"Mats schickt bis Dienstag die Unterlagen.","evidence":["final:0"],"assignee":"Mats","due":"Dienstag"}]}"#
        let result = try MinutesResult.decode(raw, segments: loaded.minutesSegments())
        let event = ProcessingEvent(operation: "Test", model: "Test", completedAt: Date(), durationMS: 0, isAI: true)
        var forged = result
        forged.items[0] = MinutesStatement(kind: .decision, text: "Anna genehmigt 50000 Euro bis Freitag.",
            evidence: result.items[0].evidence, quote: quote)
        do { try await archive.savePipelineMinutes(doc.id, result: forged, event: event); XCTFail("unquoted statement") }
        catch { if case VoiceError.invalid = error {} else { XCTFail("unexpected error") } }
        forged = result
        forged.items[0].due = "Freitag"
        do { try await archive.savePipelineMinutes(doc.id, result: forged, event: event); XCTFail("unquoted due") }
        catch { if case VoiceError.invalid = error {} else { XCTFail("unexpected error") } }
        try await archive.savePipelineMinutes(doc.id, result: result, event: event)
        let saved = try await archive.load(doc.id)
        XCTAssertEqual(saved.minutes?.tasks.first?.assignee, "Mats")
        XCTAssertEqual(saved.minutes?.tasks.first?.due, "Dienstag")
        XCTAssertEqual(saved.minutes?.quotes?["tasks.0"], quote)
        let roundtrip = try MeetingMinutes.decode(JSONEncoder().encode(saved.minutes!))
        XCTAssertEqual(roundtrip, saved.minutes!)
    }

    func testMissingConsentPreservesLegacyPartialResults() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("source.wav")
        try Data([1, 2, 3]).write(to: source)
        let folder = root.appendingPathComponent("archive")
        let archive = try MeetingArchive(root: folder, quota: 100)
        let doc = try await archive.importMedia(source, duration: 1)
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 1,
            segments: [.init(index: 0, start: 0, end: 1, text: "Inhalt")])
        let part = MeetingMinutes(summary: "Inhalt", scope: "", decisions: [], tasks: [], next_steps: [], follow_ups: [], open_questions: [])
        try await archive.appendMinutes(doc.id, from: 0, through: 1, minutes: part)
        try await archive.saveMinutesCheckpoint(doc.id, checkpoint: .init(binding: "Test", parts: [:]))
        let metadata = folder.appendingPathComponent(doc.id.uuidString).appendingPathComponent("meeting.json")
        let persisted = try Data(contentsOf: metadata)
        let before = try await archive.load(doc.id)
        XCTAssertTrue(before.requiresMinutesConsent)
        do {
            try await archive.savePipelineMinutes(doc.id, result: MinutesResult(), event:
                ProcessingEvent(operation: "Test", model: "Test", completedAt: Date(), durationMS: 0, isAI: false))
            XCTFail("missing consent must not finalize empty minutes")
        } catch { if case VoiceError.invalid = error {} else { XCTFail("unexpected error") } }
        let reopened = try MeetingArchive(root: folder, quota: 100)
        let after = try await reopened.load(doc.id)
        XCTAssertEqual(after.summaryParts, before.summaryParts)
        XCTAssertEqual(after.summarizedSegments, before.summarizedSegments)
        XCTAssertEqual(after.processingEvents, before.processingEvents)
        XCTAssertEqual(after.minutesCheckpoint, before.minutesCheckpoint)
        XCTAssertEqual(try Data(contentsOf: metadata), persisted)
        XCTAssertNil(after.minutes)
        XCTAssertTrue(after.requiresMinutesConsent)
    }
    func testPipelineEvidenceAndRouteRemainBackwardCompatible() throws {
        let old = Data(#"{"summary":"Test","scope":"","decisions":[],"tasks":[],"next_steps":[],"follow_ups":[],"open_questions":[]}"#.utf8)
        var minutes = try MeetingMinutes.decode(old)
        XCTAssertNil(minutes.evidence)
        minutes.evidence = ["summary.0": [LiveEvidence(transcriptVersion: 1, segmentId: "a", start: 0, end: 1)]]
        let roundtrip = try MeetingMinutes.decode(JSONEncoder().encode(minutes))
        XCTAssertEqual(roundtrip, minutes)
        let event = try JSONDecoder().decode(ProcessingEvent.self, from:
            Data(#"{"operation":"Test","model":"Test","completedAt":0,"durationMS":0,"isAI":true}"#.utf8))
        XCTAssertNil(event.route)
    }

    func testPipelineCheckpointSurvivesArchiveReopenAndRejectsUnconsentedEvidence() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("source.wav")
        try Data([1, 2, 3]).write(to: source)
        let folder = root.appendingPathComponent("archive")
        let archive = try MeetingArchive(root: folder, quota: 100)
        let doc = try await archive.importMedia(source, duration: 1)
        try await archive.appendChunk(doc.id, offset: 0, nextOffset: 1,
            segments: [.init(index: 0, start: 0, end: 1, text: "Test")])
        let checkpoint = MinutesCheckpoint(binding: "synthetic", parts: [:])
        try await archive.saveMinutesCheckpoint(doc.id, checkpoint: checkpoint)
        let reopened = try MeetingArchive(root: folder, quota: 100)
        let loaded = try await reopened.load(doc.id)
        XCTAssertEqual(loaded.minutesCheckpoint, checkpoint)
        let result = try MinutesResult.decode(#"{"items":[{"kind":"summary","text":"Test","quote":"Test","evidence":["final:0"]}]}"#,
            segments: loaded.minutesSegments())
        do {
            try await reopened.savePipelineMinutes(doc.id, result: result,
                event: ProcessingEvent(operation: "Test", model: "Test", completedAt: Date(), durationMS: 0, isAI: true))
            XCTFail("missing AI consent")
        } catch {
            if case VoiceError.invalid = error {} else { XCTFail("unexpected archive failure") }
        }
    }
    func testLanguageDecodesOldAndNewMeetingSegments() throws {
        let old = Data(#"{"index":0,"start":0,"end":1,"text":"Test"}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(MeetingSegment.self, from: old).language)
        for language in ["de", "en"] {
            let json = """
            {"index":0,"start":0,"end":1,"text":"Test","language":"\(language)"}
            """
            let segment = try JSONDecoder().decode(MeetingSegment.self, from: Data(json.utf8))
            XCTAssertEqual(segment.language, language)
            let round = try JSONDecoder().decode(
                MeetingSegment.self, from: JSONEncoder().encode(segment)
            )
            XCTAssertEqual(round, segment)
        }
        let null = Data(#"{"index":0,"start":0,"end":1,"text":"Test","language":null}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(MeetingSegment.self, from: null).language)
        XCTAssertNil(MeetingSegment(index: 0, start: 0, end: 1, text: "Test").language)
    }

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
        try await store.appendChunk(
            doc.id, offset: 0, nextOffset: 30,
            segments: [MeetingSegment(
                index: 0, start: 1, end: 3, text: "Gespeichert", language: "de"
            )]
        )
        let reopened = try MeetingArchive(root: folder, quota: 100)
        let loaded = try await reopened.load(doc.id)
        XCTAssertEqual(loaded.nextOffset, 30)
        XCTAssertEqual(loaded.segments.first?.text, "Gespeichert")
        XCTAssertEqual(loaded.segments.first?.language, "de")
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
    func testLiveImportIsIdempotentAndRespectsAIConsent() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("aufnahme.caf"); try Data([1,2,3,4]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        func rec(_ reason: ConsentRecord.Reason, _ at: Double, _ scopes: [ConsentRecord.Scope]) -> ConsentRecord {
            ConsentRecord(confirmedAt: Date(timeIntervalSince1970: 1_000 + at), channel: .app, reason: reason, scopes: scopes,
                          purpose: "Protokoll", controller: "Amt 12", retentionDays: 90, audioOffset: at)
        }
        var consent = ConsentLog()
        try consent.append(rec(.start, 0, [.recording, .transcription]))
        try consent.append(rec(.personJoined, 4, ConsentRecord.Scope.allCases))
        let live = MeetingArchive.LiveImport(consent: consent, live: nil, segments: [], notes: "Notiz")
        let id = UUID()
        let doc = try await archive.importMedia(source, duration: 8, live: live, id: id)
        XCTAssertEqual(doc.id, id)
        XCTAssertEqual(doc.consent, consent)
        // Wiederholung nach Absturz: dasselbe Dokument, keine Kopie.
        let again = try await archive.importMedia(source, duration: 8, live: live, id: id)
        XCTAssertEqual(again.digest, doc.digest)
        let listed = try await archive.list()
        XCTAssertEqual(listed.count, 1)
        // Andere Sitzungsdaten oder anderes Original unter derselben Kennung: Konflikt, nichts überschrieben.
        let other = MeetingArchive.LiveImport(consent: consent, live: nil, segments: [], notes: "Andere Notiz")
        do { _ = try await archive.importMedia(source, duration: 8, live: other, id: id); XCTFail("Different session data") } catch {}
        try Data([9,9,9,9]).write(to: source)
        do { _ = try await archive.importMedia(source, duration: 8, live: live, id: id); XCTFail("Different original") } catch {}
        try await archive.appendChunk(id, offset: 0, nextOffset: 8, segments: [MeetingSegment(index: 0, start: 0, end: 3, text: "Ohne KI"),
                                                                              MeetingSegment(index: 1, start: 5, end: 7, text: "Mit KI")])
        let report = MeetingMinutes(summary: "Mit KI", scope: "Mit KI", decisions: [], tasks: [], next_steps: [], follow_ups: [], open_questions: [])
        do { try await archive.appendMinutes(id, from: 0, through: 1, minutes: report); XCTFail("No AI consent for 0–3 s") } catch {}
        do { try await archive.skipMinutes(id, from: 0, through: 2); XCTFail("Must not skip permitted segments") } catch {}
        try await archive.skipMinutes(id, from: 0, through: 1)
        try await archive.appendMinutes(id, from: 1, through: 2, minutes: report)
        try await archive.finishMinutes(id)
        let withAI = try await archive.load(id)
        XCTAssertEqual(withAI.minutes?.summary, "Mit KI")
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
