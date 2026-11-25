import Foundation
import XCTest
@testable import VoiceCore

private actor MinutesProbe {
    var calls: [ModelRequest] = []
    var checkpoint: MinutesCheckpoint?
    var failAt: Int?
    var cancelAt: Int?
    func save(_ value: MinutesCheckpoint) { checkpoint = value }
    func record(_ request: ModelRequest) throws -> String {
        calls.append(request)
        if calls.count == failAt { throw ModelFailure.unavailable }
        if calls.count == cancelAt { throw CancellationError() }
        // Synthetischer Beleg, der in allen Testabschnitten vorhanden ist.
        return "{\"items\":[{\"kind\":\"summary\",\"text\":\"Test\",\"quote\":\"Test\",\"evidence\":[\"a\"]}]}"
    }
    func stop(at value: Int?) { failAt = value }
    func cancel(at value: Int) { cancelAt = value }
}

private struct MinutesFake: ModelProvider {
    let route: ModelRoute
    let probe: MinutesProbe
    var tokenDivisor: Int? = nil
    func countTokens(_ text: String) async -> Int? {
        tokenDivisor.map { (text.utf8.count + $0 - 1) / $0 }
    }
    func stream(_ request: ModelRequest, authorization: ModelAuthorization) -> AsyncThrowingStream<ModelEvent, Error> {
        AsyncThrowingStream { c in
            let task = Task {
                do {
                    try await ModelProviderCheck.confirm(self, request, authorization)
                    let answer = try await probe.record(request)
                    c.yield(.delta(answer)); c.yield(.done(nil)); c.finish()
                } catch { c.finish(throwing: error) }
            }
            c.onTermination = { _ in task.cancel() }
        }
    }
}

final class MinutesPipelineTests: XCTestCase, @unchecked Sendable {
    func testQuotesRejectFabricationAndValidateTaskFields() throws {
        let source = [LiveSegment(id: "a", start: 0, end: 1,
            text: "Das Budget ist offen. Mats schickt bis Dienstag die Unterlagen.")]
        let raw = #"{"items":[{"kind":"decision","text":"Anna genehmigt 50000 Euro bis Freitag.","quote":"Das Budget ist offen.","evidence":["a"]},{"kind":"task","text":"Mats schickt bis Dienstag die Unterlagen.","quote":"MATS schickt, bis Dienstag die Unterlagen!","evidence":["a"],"assignee":"Mats","due":"Dienstag"},{"kind":"summary","text":"Das Budget ist offen.","evidence":["a"]},{"kind":"task","text":"Mats schickt bis Dienstag die Unterlagen.","quote":"Mats schickt bis Dienstag die Unterlagen.","evidence":["a"],"assignee":"Anna","due":"Freitag"}]}"#
        let result = try MinutesResult.decode(raw, segments: source)
        XCTAssertEqual(result.items.count, 1)
        XCTAssertEqual(result.rejected, 3)
        XCTAssertEqual(result.items.first?.assignee, "Mats")
        XCTAssertEqual(result.items.first?.due, "Dienstag")
        let encoded = try JSONEncoder().encode(result)
        XCTAssertEqual(try JSONDecoder().decode(MinutesResult.self, from: encoded), result)
    }

    func testReduceCannotChangeQuoteOrBorrowAnotherSegment() throws {
        let source = [segment("Das Budget ist offen. Der Start ist Dienstag.")]
        let raw = #"{"items":[{"kind":"summary","text":"Das Budget ist offen.","quote":"Das Budget ist offen.","evidence":["a"]}]}"#
        let original = try MinutesResult.decode(raw, segments: source)
        XCTAssertEqual(try MinutesResult.decode(raw, segments: source, originalQuotes: original.items).items.count, 1)
        let changed = raw.replacingOccurrences(of: "Das Budget ist offen.", with: "Der Start ist Dienstag.")
        XCTAssertTrue(try MinutesResult.decode(changed, segments: source, originalQuotes: original.items).items.isEmpty)
        let invented = raw.replacingOccurrences(of: "Das Budget ist offen.", with: "Anna zahlt 50000 Euro am 12. Mai.")
        XCTAssertTrue(try MinutesResult.decode(invented, segments: source).items.isEmpty)
        let other = LiveSegment(id: "b", start: 1, end: 2, text: "Andere Stelle.")
        XCTAssertTrue(try MinutesResult.decode(raw.replacingOccurrences(of: #"["a"]"#, with: #"["b"]"#),
            segments: source + [other]).items.isEmpty)
        var cached = original
        cached.items[0].quote = "Erfunden"
        XCTAssertTrue(cached.validated(segments: source).items.isEmpty)
        XCTAssertEqual(cached.validated(segments: source).rejected, 1)
        XCTAssertEqual(original.validated(segments: source).items, original.items)
        XCTAssertTrue(try MinutesResult.decode(changed, segments: source)
            .validated(segments: source, originalQuotes: original.items).items.isEmpty)
    }

    func testQuoteNormalizationKeepsNameAndNumberBoundaries() {
        XCTAssertTrue(MeetingMinutes.containsQuote("MATS, schickt!", in: "Mats\n schickt Unterlagen."))
        XCTAssertFalse(MeetingMinutes.containsQuote("Anna", in: "Annabel"))
        XCTAssertFalse(MeetingMinutes.containsQuote("10", in: "2010"))
        XCTAssertFalse(MeetingMinutes.containsQuote("10", in: "10.5"))
        XCTAssertFalse(MeetingMinutes.containsQuote("12. Mai", in: "13. Mai"))
        XCTAssertFalse(MeetingMinutes.containsQuote("50000 Euro", in: "5000 Euro"))
        XCTAssertFalse(MeetingMinutes.containsQuote("...", in: "Budget offen"))
    }
    private var consent: ConsentLog {
        ConsentLog(records: [.init(confirmedAt: Date(timeIntervalSince1970: 0), channel: .import,
            reason: .start, scopes: [.recording, .transcription, .aiAnalysis],
            purpose: "Test", controller: "Test", retentionDays: 1)])
    }
    private func segment(_ text: String = "Test") -> LiveSegment {
        .init(id: "a", start: 0, end: 1, text: text, speaker: "Test", language: "en")
    }
    private func pipeline(_ probe: MinutesProbe, context: Int = 4096, divisor: Int? = nil) async throws -> MinutesPipeline {
        let provider = MinutesFake(route: .local(modelId: "fake"), probe: probe, tokenDivisor: divisor)
        let session = try await ModelGate.open(provider, purpose: .meetingMinutes, subjectId: "test",
                                               dataCategories: ["Transkript", "Teilnehmernamen"], state: ModelAccessState())
        return MinutesPipeline(session: session, contextTokens: context, outputTokens: 256)
    }

    func testJSONRepairAndUnknownEvidenceAreNotAccepted() throws {
        let raw = "```json\n{\"items\":[{\"kind\":\"summary\",\"text\":\"Test,}\",\"quote\":\"Test,}\",\"evidence\":[\"a\"],}, {\"kind\":\"task\",\"text\":\"Erfunden\",\"evidence\":[\"missing\"]}]}\n```"
        let result = try MinutesResult.decode(raw, segments: [segment()])
        XCTAssertEqual(result.items.count, 1)
        XCTAssertEqual(result.items.first?.text, "Test,}")
        XCTAssertEqual(result.items.first?.evidence.first?.segmentId, "a")
        XCTAssertEqual(result.rejected, 1)
        XCTAssertThrowsError(try MinutesResult.decode("{broken", segments: [segment()]))
    }

    func testMapReduceSplitsOversizedSegmentAndHonorsWholeRequestBudget() async throws {
        let probe = MinutesProbe()
        let pipeline = try await pipeline(probe)
        let result = try await pipeline.run(segments: [segment(String(repeating: "Test ", count: 1400))], consent: consent)
        XCTAssertFalse(result.items.isEmpty)
        let calls = await probe.calls
        XCTAssertGreaterThan(calls.count, 2)
        for call in calls {
            XCTAssertLessThanOrEqual(call.messages.reduce(0) { $0 + $1.text.utf8.count } + call.maxTokens + 128, 4096)
            XCTAssertTrue(call.messages[0].text.contains("Deutsch"))
            XCTAssertNotNil(call.jsonSchema)
        }
    }

    func testHierarchicalReduceNeedsMoreThanOneLevel() async throws {
        let probe = MinutesProbe()
        let pipeline = try await pipeline(probe)
        _ = try await pipeline.run(segments: [segment(String(repeating: "Test ", count: 20000))], consent: consent)
        let calls = await probe.calls
        let reduces = calls.filter { $0.messages.last?.text.hasPrefix("[{\"evidence\":") == true }
        XCTAssertGreaterThan(reduces.count, 2)
        // Mindestens zwei Reduce-Gruppen und ein abschließender Reduce-Aufruf.
        XCTAssertTrue(reduces.last!.messages.last!.text.contains("Test"))
    }

    func testExternalConsentIsCheckedBeforeEveryRequest() async throws {
        let probe = MinutesProbe()
        let provider = MinutesFake(route: .endpoint(id: UUID()), probe: probe)
        do {
            _ = try await ModelGate.open(provider, purpose: .meetingMinutes, subjectId: "test",
                dataCategories: ["Transkript"], state: ModelAccessState())
            XCTFail("unknown external route")
        } catch { XCTAssertEqual(error as? ModelFailure, .unavailable) }
        let calls = await probe.calls
        XCTAssertTrue(calls.isEmpty)
    }

    func testTokenizerUsedAndTooSmallBudgetFailsBeforeSending() async throws {
        let probe = MinutesProbe()
        let measured = try await pipeline(probe, context: 4096, divisor: 4)
        _ = try await measured.run(segments: [segment(String(repeating: "T", count: 4000))], consent: consent)
        let calls = await probe.calls
        XCTAssertEqual(calls.count, 1)
        let tinyProbe = MinutesProbe()
        let tiny = try await pipeline(tinyProbe, context: 100)
        do { _ = try await tiny.run(segments: [segment()], consent: consent); XCTFail("budget") }
        catch { XCTAssertEqual(error as? ModelFailure, .contextExceeded) }
        let tinyCalls = await tinyProbe.calls
        XCTAssertTrue(tinyCalls.isEmpty)
    }

    func testExactByteBudgetBoundary() async throws {
        let input = [segment(String(repeating: "T", count: 400))]
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        let size = try encoder.encode(input).count + MinutesTemplate.minutes.instructions.utf8.count + 256 + 128
        let exactProbe = MinutesProbe()
        let exact = try await pipeline(exactProbe, context: size)
        _ = try await exact.run(segments: input, consent: consent)
        let exactCalls = await exactProbe.calls
        XCTAssertEqual(exactCalls.count, 1)
        let smallerProbe = MinutesProbe()
        let smaller = try await pipeline(smallerProbe, context: size - 1)
        _ = try await smaller.run(segments: input, consent: consent)
        let splitCalls = await smallerProbe.calls
        XCTAssertGreaterThan(splitCalls.count, 1)
    }

    func testCancellationDuringMapPreservesFirstCompletedPart() async throws {
        let probe = MinutesProbe()
        await probe.cancel(at: 2)
        let pipeline = try await pipeline(probe)
        do {
            _ = try await pipeline.run(segments: [segment(String(repeating: "T", count: 5000))], consent: consent,
                onCheckpoint: { await probe.save($0) })
            XCTFail("cancelled map")
        } catch { XCTAssertTrue(error is CancellationError) }
        let checkpoint = await probe.checkpoint
        XCTAssertEqual(checkpoint?.parts.count, 1)
    }

    func testResumeKeepsCompletedMapsAndInvalidatesChangedInput() async throws {
        let probe = MinutesProbe()
        await probe.stop(at: 2)
        let pipeline = try await pipeline(probe)
        let input = [segment((0..<900).map { "Test\($0) " }.joined())]
        do {
            _ = try await pipeline.run(segments: input, consent: consent, onCheckpoint: { await probe.save($0) })
            XCTFail("provider should stop")
        } catch { XCTAssertEqual(error as? ModelFailure, .unavailable) }
        let checkpoint = await probe.checkpoint
        XCTAssertEqual(checkpoint?.parts.count, 1)
        await probe.stop(at: nil)
        _ = try await pipeline.run(segments: input, consent: consent, checkpoint: checkpoint)
        let calls = await probe.calls
        XCTAssertEqual(calls.filter { $0.messages == calls[0].messages }.count, 1)
        _ = try await pipeline.run(segments: [segment("Geändert")], consent: consent, checkpoint: checkpoint)
        let changed = await probe.calls
        XCTAssertTrue(changed.last?.messages.last?.text.contains("Geändert") == true)
    }

    func testCancellationAndConsentGapsSendNothing() async throws {
        let probe = MinutesProbe()
        let pipeline = try await pipeline(probe)
        let empty = try await pipeline.run(segments: [segment()], consent: ConsentLog())
        XCTAssertTrue(empty.items.isEmpty)
        let task = Task { try await pipeline.run(segments: [segment()], consent: consent) }
        task.cancel()
        do { _ = try await task.value; XCTFail("cancelled") } catch { XCTAssertTrue(error is CancellationError) }
        let calls = await probe.calls
        XCTAssertTrue(calls.isEmpty)
    }

    func testFinalRefinementPreferredAndConsentCoversEntireRange() async throws {
        let probe = MinutesProbe()
        let pipeline = try await pipeline(probe)
        var refined = segment("Verfeinert"); refined.refined = true
        let draft = LiveSegment(id: "draft", start: 0, end: 1, text: "Nicht final", isFinal: false)
        _ = try await pipeline.run(segments: [segment("Ursprung"), refined, draft], consent: consent)
        let calls = await probe.calls
        XCTAssertFalse(calls[0].messages.last!.text.contains("Ursprung"))
        XCTAssertFalse(calls[0].messages.last!.text.contains("Nicht final"))
        var gap = consent
        try gap.append(.init(confirmedAt: Date(timeIntervalSince1970: 1), channel: .import,
            reason: .personJoined, scopes: [.recording, .transcription], purpose: "Test", controller: "Test",
            retentionDays: 1, audioOffset: 0.5))
        let result = try await pipeline.run(segments: [segment()], consent: gap)
        XCTAssertTrue(result.items.isEmpty)
        let after = await probe.calls
        XCTAssertEqual(after.count, 1)
    }
}
