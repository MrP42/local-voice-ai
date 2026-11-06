import XCTest
@testable import VoiceCore

/// Fake-Übersetzer: setzt „DE:“ davor, zählt Aufrufe, kann verzögern oder Unbrauchbares liefern.
private actor FakeTranslator: LiveTranslator {
    var calls: [[String]] = []
    var delay: UInt64 = 0
    var echo: Set<String> = []
    let echoAlways: Bool

    init(delay: UInt64 = 0, echo: Set<String> = [], echoAlways: Bool = false) {
        self.delay = delay; self.echo = echo; self.echoAlways = echoAlways
    }

    func translate(_ texts: [String], from source: String, to target: String) async throws -> [String] {
        calls.append(texts)
        if delay > 0 { try await Task.sleep(nanoseconds: delay) }
        return texts.map { t in
            if echo.contains(t) { if !echoAlways { echo.remove(t) }; return t }
            return "\(target.uppercased()):\(t)"
        }
    }
}

private func ranges(_ text: String, _ parts: [String]) -> [NSRange] {
    let s = text as NSString
    return parts.map { s.range(of: $0) }
}

final class LiveTranslationTests: XCTestCase {
    func testOneUnitPerSentenceEvenWhenShort() {
        let text = "„Komm“, sagte er. Dann ging er langsam zur Tür hinaus. Ja."
        let parts = ["„Komm“, sagte er.", "Dann ging er langsam zur Tür hinaus.", "Ja."]
        let units = LiveTranslation.units(paragraph: 3, text: text, sentences: ranges(text, parts))
        XCTAssertEqual(units.map(\.text), parts)
        XCTAssertEqual(units.map(\.sentences), [0...0, 1...1, 2...2])
        XCTAssertEqual(units[1].paragraph, 3)
        XCTAssertEqual(LiveTranslation.unitIndex(units, sentence: 2), 2)
        XCTAssertTrue(LiveTranslation.units(paragraph: 2, text: "", sentences: []).isEmpty)
    }

    func testRangesAreUTF16() {
        let text = "Emoji 😀 steht hier im ersten Satz. Und hier folgt der zweite Satz."
        let parts = ["Emoji 😀 steht hier im ersten Satz.", "Und hier folgt der zweite Satz."]
        let units = LiveTranslation.units(paragraph: 0, text: text, sentences: ranges(text, parts))
        XCTAssertEqual(units.count, 2)
        XCTAssertEqual((text as NSString).substring(with: units[1].range), parts[1])
    }

    private func book(_ paragraphs: Int, unitsEach: Int, chars: Int) -> (Int) -> [LiveUnit] {
        { p in
            (0..<unitsEach).map { u in
                LiveUnit(paragraph: p, range: NSRange(location: u * chars, length: chars),
                         text: String(repeating: "a", count: chars), sentences: u...u)
            }
        }
    }

    func testWindowNeedsCountAndTime() {
        // 60 Zeichen = 4 s je Einheit: 30 s brauchen 8 Einheiten (mehr als die Mindestzahl 6).
        let w = LiveTranslation.window(paragraph: 0, unit: 2, paragraphCount: 10, units: book(10, unitsEach: 3, chars: 60))
        XCTAssertEqual(w.count, 8)
        XCTAssertEqual(w.first?.paragraph, 0)
        XCTAssertEqual(w.first?.range.location, 120)
        // Lange Einheiten: die Mindestzahl entscheidet.
        let long = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 10, units: book(10, unitsEach: 3, chars: 600))
        XCTAssertEqual(long.count, 6)
        // Winzige Einheiten: Obergrenze.
        let tiny = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 100, units: book(100, unitsEach: 3, chars: 5))
        XCTAssertEqual(tiny.count, LiveTranslation.maxAheadUnits)
        // Buchende: nur was da ist.
        let end = LiveTranslation.window(paragraph: 9, unit: 1, paragraphCount: 10, units: book(10, unitsEach: 3, chars: 60))
        XCTAssertEqual(end.count, 2)
        // Schnelleres Tempo braucht mehr Vorlauf für dieselbe Zeit.
        let fast = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 10, rate: 2, units: book(10, unitsEach: 3, chars: 60))
        XCTAssertEqual(fast.count, 15)
    }

    func testBatchesPerParagraphSplitLongOnes() {
        let units = book(3, unitsEach: 2, chars: 30)
        let list = units(0) + units(1) + [units(2)[1]]
        XCTAssertEqual(LiveTranslation.batches(list).map { $0.count }, [2, 2, 1])
        let long = book(1, unitsEach: 30, chars: 10)(0)
        XCTAssertEqual(LiveTranslation.batches(long).map { $0.count }, [12, 12, 6])
    }

    func testUnchangedAnswersAreNotErrors() {
        XCTAssertFalse(LiveTranslation.usable("  "))
        XCTAssertTrue(LiveTranslation.usable("Harry."))
        XCTAssertTrue(LiveTranslation.worthRetry(source: "Hi", translated: "", from: "en", to: "de"))
        XCTAssertFalse(LiveTranslation.worthRetry(source: "Harry.", translated: "Harry.", from: "en", to: "de"))
        XCTAssertFalse(LiveTranslation.worthRetry(source: "1984", translated: "1984", from: "en", to: "de"))
        XCTAssertTrue(LiveTranslation.worthRetry(source: "This is a long sentence.", translated: "this is a long sentence.", from: "en", to: "de"))
        XCTAssertFalse(LiveTranslation.worthRetry(source: "This is a long sentence.", translated: "This is a long sentence.", from: "en-US", to: "en-GB"))
        XCTAssertEqual(LiveTranslation.base("zh_Hans"), "zh")
    }

    func testCacheEvictsOldest() {
        var cache = LiveCache(limit: 2)
        let u = book(1, unitsEach: 3, chars: 10)(0)
        let keys = u.map { LiveKey(book: "b", sourceHash: "h", target: "de-DE", unit: $0) }
        cache.put(keys[0], "a"); cache.put(keys[1], "b")
        XCTAssertEqual(cache.get(keys[0]), "a")  // macht 0 frisch
        cache.put(keys[2], "c")
        XCTAssertNil(cache.peek(keys[1]))
        XCTAssertEqual(cache.peek(keys[0]), "a")
        XCTAssertEqual(cache.count, 2)
        XCTAssertEqual(keys[0].target, "de")
    }

    func testPipelineTranslatesOnlyWindowAndReusesInflight() async throws {
        let fake = FakeTranslator(delay: 20_000_000)
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let units = book(10, unitsEach: 3, chars: 60)
        let window = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 10, units: units)
        await pipe.prefetch(window)
        // Wartet auf den laufenden Stapel, startet keinen zweiten.
        let first = try await pipe.text(window[0])
        XCTAssertEqual(first, "DE:" + window[0].text)
        for unit in window { _ = try await pipe.text(unit) }
        let calls = await fake.calls
        XCTAssertEqual(calls.map(\.count), [3, 3, 2])
        let chars = await pipe.translatedChars
        XCTAssertEqual(chars, window.count * 60)
        // Erneut lesen kostet nichts.
        _ = try await pipe.text(window[1])
        let after = await fake.calls.count
        XCTAssertEqual(after, 3)
        let ready = await pipe.ready(window[7])
        XCTAssertNotNil(ready)
    }

    func testUnchangedLongAnswerIsRetriedOnceThenAccepted() async throws {
        let unit = LiveUnit(paragraph: 0, range: NSRange(location: 0, length: 26), text: "This is a long sentence ok", sentences: 0...0)
        let fake = FakeTranslator(echo: [unit.text], echoAlways: true)
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let text = try await pipe.text(unit)
        XCTAssertEqual(text, unit.text)
        let calls = await fake.calls.count
        XCTAssertEqual(calls, 2)
    }

    func testUnusableAnswerIsRetriedOnce() async throws {
        let unit = LiveUnit(paragraph: 0, range: NSRange(location: 0, length: 26), text: "This is a long sentence ok", sentences: 0...0)
        let fake = FakeTranslator(echo: [unit.text])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let text = try await pipe.text(unit)
        XCTAssertEqual(text, "DE:" + unit.text)
        let calls = await fake.calls.count
        XCTAssertEqual(calls, 2)
    }

    func testCancelDropsLateResults() async throws {
        let fake = FakeTranslator(delay: 50_000_000)
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let units = book(2, unitsEach: 2, chars: 40)(0)
        await pipe.prefetch(units)
        await pipe.cancelPending()
        let gen = await pipe.currentGeneration
        XCTAssertEqual(gen, 1)
        try await Task.sleep(nanoseconds: 100_000_000)
        let late = await pipe.ready(units[0])
        XCTAssertNil(late)
    }
}
