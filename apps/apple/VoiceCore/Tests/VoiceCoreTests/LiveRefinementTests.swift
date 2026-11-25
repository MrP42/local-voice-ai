import XCTest
@testable import VoiceCore

final class LiveRefinementTests: XCTestCase {
    func testRingWrapAndShortOverflowBoundary() {
        var ring = LiveAudioRing(capacity: 4)
        [Float](arrayLiteral: 1, 2, 3).withUnsafeBufferPointer { ring.append(startFrame: 0, samples: $0) }
        [Float](arrayLiteral: 4, 5).withUnsafeBufferPointer { ring.append(startFrame: 3, samples: $0) }
        XCTAssertEqual(ring.pop()?.samples, [1, 2, 3])
        [Float](arrayLiteral: 6, 7, 8).withUnsafeBufferPointer { ring.append(startFrame: 5, samples: $0) }
        let next = ring.pop()
        XCTAssertEqual(next?.startFrame, 5)
        XCTAssertEqual(next?.samples, [6, 7, 8])
        XCTAssertEqual(next?.boundaryBefore, true, "A dropped buffer below 200 ms must still close the chunk")
        XCTAssertEqual(ring.takeMissed(), LiveRefinementRange(start: 3.0 / 16000, end: 5.0 / 16000))
        XCTAssertNil(ring.takeMissed())
        XCTAssertNil(ring.pop())
    }

    func testLiveChunkerSilenceAndTimeline() {
        var chunker = LiveChunker()
        XCTAssertTrue(chunker.push(track: "mic", startFrame: 0, samples: [Float](repeating: 0, count: 16000)).isEmpty)
        let speech = [Float](repeating: 0.1, count: 8000)
        XCTAssertTrue(chunker.push(track: "mic", startFrame: 16000, samples: speech).isEmpty)
        let chunks = chunker.push(track: "mic", startFrame: 24000, samples: [Float](repeating: 0, count: 16000))
        XCTAssertEqual(chunks.count, 1)
        XCTAssertEqual(chunks.first?.startFrame, 16000)
        XCTAssertEqual(chunks.first?.samples.count, 11200)
        XCTAssertEqual(chunks.first?.clearMs, 500)
    }

    func testLiveChunkerMaximumCutsAtQuietestWindow() {
        var chunker = LiveChunker()
        var samples = [Float](repeating: 0.1, count: 240000)
        samples.replaceSubrange(224000..<225600, with: repeatElement(Float(0.025), count: 1600))
        let chunks = chunker.push(track: "mic", startFrame: 0, samples: samples)
        XCTAssertEqual(chunks.first?.samples.count, 225600)
        XCTAssertEqual(chunker.flush().first?.startFrame, 225600)
    }

    func testLiveChunkerNoiseFloorAndPauseTail() {
        for noise: Float in [0.0001, 0.01] {
            var chunker = LiveChunker()
            _ = chunker.push(track: "mic", startFrame: 0, samples: [Float](repeating: noise, count: 48000))
            _ = chunker.flush()
            _ = chunker.push(track: "mic", startFrame: 48000, samples: [Float](repeating: 0.1, count: 8100))
            let chunks = chunker.flush()
            XCTAssertEqual(chunks.first?.samples.count, 8100, "Flush preserves the incomplete last window")
        }
    }

    func testLiveLanguageLockAndConsecutiveSameCandidate() {
        var lock = LiveLanguageLock()
        XCTAssertEqual(lock.language(track: "mic"), "auto")
        lock.accept(track: "mic", language: "de", prob: 0.8, text: "Guten Morgen")
        XCTAssertEqual(lock.language(track: "mic"), "de")
        XCTAssertEqual(lock.retry(track: "mic", language: "en", prob: 0.9, text: "Good morning"), "en")
        lock.accept(track: "mic", language: "en", prob: 0.9, text: "Good morning")
        XCTAssertEqual(lock.language(track: "mic"), "de")
        lock.accept(track: "mic", language: "fr", prob: 0.95, text: "Bonjour ensemble")
        XCTAssertEqual(lock.language(track: "mic"), "de")
        lock.accept(track: "mic", language: "en", prob: 0.95, text: "Good evening")
        XCTAssertEqual(lock.language(track: "mic"), "de")
        lock.accept(track: "mic", language: "en", prob: 0.95, text: "Next topic")
        XCTAssertEqual(lock.language(track: "mic"), "en")
        XCTAssertEqual(lock.language(track: "other"), "auto")
        XCTAssertEqual(lock.retry(track: "mic", language: "en", prob: 1, text: ""), "auto")
        lock.accept(track: "mic", language: "auto", prob: 0, text: "")
        XCTAssertEqual(lock.language(track: "mic"), "en")
        var fixed = LiveLanguageLock(fixed: "it")
        fixed.accept(track: "mic", language: "de", prob: 1, text: "Guten Morgen")
        XCTAssertEqual(fixed.language(track: "mic"), "it")
        XCTAssertNil(fixed.retry(track: "mic", language: "de", prob: 1, text: ""))
    }

    func testLiveHallucinationEchoStockAndRealSpeech() {
        XCTAssertTrue(LiveHallucination.isPromptEcho("Anna Must", prompt: "Anna Muster Jonas Beispiel"))
        XCTAssertFalse(LiveHallucination.isPromptEcho("Anna", prompt: "Anna Muster"))
        XCTAssertEqual(LiveHallucination.clean("Anna Muster: Wir beginnen jetzt", prompt: "Anna Muster", clearMs: 0), "Wir beginnen jetzt")
        for text in ["Thank you", "Vielen Dank", "Merci", "Gracias", "Grazie", "Bedankt", "Dziękuję", "Teşekkürler"] {
            XCTAssertNil(LiveHallucination.clean(text, prompt: "", clearMs: 299))
            XCTAssertNotNil(LiveHallucination.clean(text, prompt: "", clearMs: 300))
        }
        XCTAssertTrue(LiveHallucination.isGarbled("ja ja ja ja ja ja ja ja"))
        XCTAssertFalse(LiveHallucination.isGarbled("Wir sprechen heute über den nächsten Termin"))
        XCTAssertNil(LiveHallucination.clean("Subtitles by Example", prompt: "", clearMs: 1000))
        XCTAssertNotNil(LiveHallucination.clean("We discuss subtitles today", prompt: "", clearMs: 1000))
    }

    func testLiveRefinementReplacesAndRebindsEvidence() {
        let old = LiveSegment(id: "a0", start: 0, end: 4, text: "Beispiel")
        let untouched = LiveSegment(id: "a1", start: 4, end: 6, text: "Weiter")
        let refined = LiveSegment(id: "w0", start: 1, end: 3, text: "Korrektur", language: "de", refined: true)
        var state = LiveState()
        _ = state.apply(LiveDelta(add: [.init(kind: .topic, text: "Thema", evidence: ["a0"])]), segments: [old])
        var segments = [old, untouched]
        state.replaceRefined(segments: &segments, from: 0, to: 4, lines: [refined])
        XCTAssertEqual(segments, [refined, untouched])
        XCTAssertEqual(state.transcriptVersion, 2)
        XCTAssertEqual(state.items.first?.evidence.first?.segmentId, "w0")
        XCTAssertEqual(state.items.first?.evidence.first?.transcriptVersion, 2)
        XCTAssertEqual(state.items.first?.evidence.first?.start, 1)
        XCTAssertEqual(state.items.first?.evidence.first?.end, 3)
        XCTAssertEqual(state.processedThrough, 0)
        XCTAssertEqual(LiveRefinement.preferredSegments([old, refined, untouched]), [refined, untouched])
    }

    func testRefinedCodableAndPieces() throws {
        let old = Data(#"{"id":"a0","start":0,"end":1,"text":"Test","isFinal":true}"#.utf8)
        XCTAssertFalse(try JSONDecoder().decode(LiveSegment.self, from: old).refined)
        let new = LiveSegment(id: "w1", start: 0, end: 3, text: String(repeating: "synthetisch ", count: 80), refined: true)
        XCTAssertEqual(try JSONDecoder().decode(LiveSegment.self, from: JSONEncoder().encode(new)), new)
        XCTAssertTrue(LivePromptBuilder.pieces(new).allSatisfy(\.refined))
    }

    func testPromptVocabularyBudgetAndFreemail() {
        let prompt = LiveRefinement.prompt(names: ["Anna Muster"], emails: ["a@beispiel.test", "b@gmail.com"], glossary: "Quantenfrosch", count: { $0.utf8.count })
        XCTAssertTrue(prompt.contains("Anna Muster"))
        XCTAssertTrue(prompt.contains("beispiel"))
        XCTAssertFalse(prompt.contains("gmail"))
        XCTAssertLessThanOrEqual(prompt.utf8.count, 200)
    }

    func testNoReplacementOnEmptyRefinementAndEvidenceSplit() {
        let a = LiveSegment(id: "a0", start: 0, end: 6, text: "Test")
        var segments = [a]
        var state = LiveState()
        state.replaceRefined(segments: &segments, from: 0, to: 6, lines: [])
        XCTAssertEqual(segments, [a])
        XCTAssertEqual(state.transcriptVersion, 1)
        _ = state.apply(LiveDelta(add: [.init(kind: .open, text: "Frage", evidence: ["a0"])]), segments: segments)
        _ = state.apply(LiveDelta(answered: [.init(id: state.items[0].id, evidence: ["a0"])]), segments: segments)
        let lines = [LiveSegment(id: "w0", start: 0, end: 3, text: "Eins", refined: true),
                     LiveSegment(id: "w1", start: 3, end: 6, text: "Zwei", refined: true)]
        state.replaceRefined(segments: &segments, from: 0, to: 6, lines: lines)
        XCTAssertEqual(state.items[0].evidence.map(\.segmentId), ["w0", "w1"])
        XCTAssertEqual(state.items[0].answerEvidence.map(\.segmentId), ["w0", "w1"])
        XCTAssertTrue(state.items[0].evidence.allSatisfy { $0.transcriptVersion == 2 })
    }

    func testDeferredRangesSurviveArchiveAndRepeatedImport() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("synthetic.caf")
        try Data([1, 2, 3]).write(to: source)
        let archive = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        let consent = ConsentLog(records: [ConsentRecord(confirmedAt: Date(), channel: .app, reason: .start,
            scopes: [.recording, .transcription], purpose: "Test", controller: "Test", retentionDays: 1)])
        let ranges = [LiveRefinementRange(start: 1, end: 2)]
        let live = MeetingArchive.LiveImport(consent: consent, live: LiveState(), segments: [], notes: nil, deferredRefinement: ranges)
        let id = UUID()
        _ = try await archive.importMedia(source, duration: 3, live: live, id: id)
        let repeated = try await archive.importMedia(source, duration: 3, live: live, id: id)
        XCTAssertEqual(repeated.deferredRefinement, ranges)
        let reopened = try MeetingArchive(root: root.appendingPathComponent("archive"), quota: 100)
        let saved = try await reopened.load(id)
        XCTAssertEqual(saved.deferredRefinement, ranges)
        var changed = live
        changed.deferredRefinement = []
        do {
            _ = try await archive.importMedia(source, duration: 3, live: changed, id: id)
            XCTFail("A repeated import must not silently lose deferred ranges")
        } catch VoiceError.conflict { }
    }
}
