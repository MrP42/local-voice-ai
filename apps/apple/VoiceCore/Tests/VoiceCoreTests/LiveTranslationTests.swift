import XCTest
@testable import VoiceCore

private struct Boom: Error {}

/// Fake-Übersetzer mit Drehbuch: setzt „DE:“ davor, zählt Aufrufe; einzelne Aufrufe können verzögern,
/// werfen, zu wenige oder leere Antworten liefern oder den Text unverändert zurückgeben.
private actor FakeTranslator: LiveTranslator {
    enum Step { case ok, fail, short, empty, echo }
    var calls: [[String]] = []
    var script: [Step]
    let delayPerItem: UInt64

    init(_ script: [Step] = [], delayPerItem: UInt64 = 0) { self.script = script; self.delayPerItem = delayPerItem }

    func translate(_ texts: [String], from source: String, to target: String) async throws -> [String] {
        calls.append(texts)
        let step = script.isEmpty ? .ok : script.removeFirst()
        if delayPerItem > 0 { try await Task.sleep(nanoseconds: delayPerItem * UInt64(texts.count)) }
        switch step {
        case .ok: return texts.map { "\(target.uppercased()):\($0)" }
        case .fail: throw Boom()
        case .short: return Array(texts.map { "\(target.uppercased()):\($0)" }.dropLast())
        case .empty: return texts.map { _ in " " }
        case .echo: return texts
        }
    }
}

/// Übersetzer mit steuerbaren Fortsetzungen: jeder Aufruf wartet, bis der Test ihn freigibt.
/// Ignoriert Cancellation absichtlich (wie ein Übersetzer, der sie nicht beachtet).
private actor GatedTranslator: LiveTranslator {
    private var waiting: [(texts: [String], resume: CheckedContinuation<[String], Error>)] = []
    private(set) var started = 0

    func translate(_ texts: [String], from source: String, to target: String) async throws -> [String] {
        started += 1
        return try await withCheckedThrowingContinuation { waiting.append((texts, $0)) }
    }

    /// Wartet ohne Schlafzeiten, bis `count` Aufrufe angekommen sind.
    func waitForCalls(_ count: Int) async { while started < count { await Task.yield() } }

    /// Gibt den ältesten offenen Aufruf mit `size` Texten frei (oder den ältesten überhaupt).
    func release(size: Int? = nil, _ result: Result<Void, Error> = .success(()), echo: Bool = false) {
        guard let i = waiting.firstIndex(where: { size == nil || $0.texts.count == size }) else { return }
        let call = waiting.remove(at: i)
        switch result {
        case .success: call.resume.resume(returning: echo ? call.texts : call.texts.map { "DE:" + $0 })
        case .failure(let e): call.resume.resume(throwing: e)
        }
    }
}

private func ranges(_ text: String, _ parts: [String]) -> [NSRange] {
    let s = text as NSString
    return parts.map { s.range(of: $0) }
}

private func unit(_ text: String, paragraph: Int = 0, at location: Int = 0) -> LiveUnit {
    LiveUnit(paragraph: paragraph, range: NSRange(location: location, length: text.utf16.count), text: text, sentences: 0...0)
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

    #if canImport(NaturalLanguage)
    /// Integration mit der echten Satzteilung des Readers (läuft auf dem Mac, nicht unter Linux).
    func testUnitsFollowLibraryTextSentences() {
        let text = "Dr. Watson kam spät. „Komm“, sagte er. Emoji 😀 hier. Ende."
        let sentences = LibraryText.sentences(text)
        let units = LiveTranslation.units(paragraph: 0, text: text, sentences: sentences)
        XCTAssertEqual(units.count, sentences.count)
        for (u, r) in zip(units, sentences) {
            XCTAssertEqual(u.range, r)
            XCTAssertEqual(u.text, (text as NSString).substring(with: r))
        }
        XCTAssertTrue(units[0].text.hasPrefix("Dr. Watson"))  // Abkürzung trennt nicht
    }
    #endif

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
        let long = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 10, units: book(10, unitsEach: 3, chars: 600))
        XCTAssertEqual(long.count, 6)
        // Kurze Absätze/Einheiten: die Höchstgrenze hat Vorrang.
        let tiny = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 100, units: book(100, unitsEach: 3, chars: 5))
        XCTAssertEqual(tiny.count, LiveTranslation.maxAheadUnits)
        let end = LiveTranslation.window(paragraph: 9, unit: 1, paragraphCount: 10, units: book(10, unitsEach: 3, chars: 60))
        XCTAssertEqual(end.count, 2)
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
    }

    func testLanguageKeepsScript() {
        XCTAssertEqual(LiveTranslation.base("zh_Hans_CN"), "zh-hans")
        XCTAssertEqual(LiveTranslation.base("zh-Hant"), "zh-hant")
        XCTAssertEqual(LiveTranslation.base("en-GB"), "en")
        XCTAssertEqual(LiveTranslation.base("de"), "de")
        // Schriftvarianten gelten als verschiedene Sprachen: unveränderter Satz wird nachgefragt.
        let s = "这是一个很长的句子 有 四 个 词"
        XCTAssertTrue(LiveTranslation.worthRetry(source: s, translated: s, from: "zh-Hans", to: "zh-Hant"))
        // Getrennte Speicherschlüssel.
        let u = unit("你好世界")
        XCTAssertNotEqual(LiveKey(book: "b", sourceHash: "h", target: "zh-Hans", unit: u),
                          LiveKey(book: "b", sourceHash: "h", target: "zh-Hant", unit: u))
    }

    func testCacheEvictsOldest() {
        var cache = LiveCache(limit: 2)
        let u = book(1, unitsEach: 3, chars: 10)(0)
        let keys = u.map { LiveKey(book: "b", sourceHash: "h", target: "de-DE", unit: $0) }
        cache.put(keys[0], "a"); cache.put(keys[1], "b")
        XCTAssertEqual(cache.get(keys[0]), "a")
        cache.put(keys[2], "c")
        XCTAssertNil(cache.peek(keys[1]))
        XCTAssertEqual(cache.peek(keys[0]), "a")
        XCTAssertEqual(cache.count, 2)
        XCTAssertEqual(keys[0].target, "de")
    }

    func testPipelineTranslatesOnlyWindowAndReusesInflight() async throws {
        let fake = FakeTranslator(delayPerItem: 2_000_000)
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let units = book(10, unitsEach: 3, chars: 60)
        let window = LiveTranslation.window(paragraph: 0, unit: 0, paragraphCount: 10, units: units)
        await pipe.prefetch(window)
        for u in window { _ = try await pipe.text(u) }
        let calls = await fake.calls
        // Erste Einheit allein, Rest je Absatz.
        XCTAssertEqual(calls.map(\.count), [1, 2, 3, 2])
        let chars = await pipe.translatedChars
        XCTAssertEqual(chars, window.count * 60)
        _ = try await pipe.text(window[1])
        let after = await fake.calls.count
        XCTAssertEqual(after, 4)
    }

    func testThrownErrorIsRetriedOnce() async throws {
        let fake = FakeTranslator([.fail])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let text = try await pipe.text(unit("Hello there"))
        XCTAssertEqual(text, "DE:Hello there")
        let calls = await pipe.calls
        XCTAssertEqual(calls, 2)
    }

    func testWrongCountIsRetriedThenFails() async {
        let fake = FakeTranslator([.short, .short])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        do { _ = try await pipe.text(unit("Hello there")); XCTFail("erwartet Fehler") }
        catch { XCTAssertEqual(error as? LiveTranslationError, .countMismatch) }
        let calls = await pipe.calls
        XCTAssertEqual(calls, 2)
    }

    func testEmptyAnswerIsRetriedThenUnusable() async {
        let fake = FakeTranslator([.empty, .empty])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        do { _ = try await pipe.text(unit("Hello there")); XCTFail("erwartet Fehler") }
        catch { XCTAssertEqual(error as? LiveTranslationError, .unusable) }
        let calls = await pipe.calls
        XCTAssertEqual(calls, 2)
    }

    func testEmptyAnswerRecoveredByRetry() async throws {
        let fake = FakeTranslator([.empty, .ok])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let text = try await pipe.text(unit("Hello there"))
        XCTAssertEqual(text, "DE:Hello there")
    }

    func testUnchangedLongAnswerIsRetriedOnceThenAccepted() async throws {
        let long = "This is a long sentence ok"
        let fake = FakeTranslator([.echo, .echo])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let text = try await pipe.text(unit(long))
        XCTAssertEqual(text, long)
        let calls = await pipe.calls
        XCTAssertEqual(calls, 2)
        let chars = await pipe.translatedChars
        XCTAssertEqual(chars, 2 * long.utf16.count)
    }

    func testNewRequestAfterFailure() async throws {
        let fake = FakeTranslator([.fail, .fail])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let u = unit("Hello there")
        do { _ = try await pipe.text(u); XCTFail("erwartet Fehler") } catch {}
        // Der gescheiterte Auftrag hängt nicht mehr: ein neuer Aufruf übersetzt erneut.
        let text = try await pipe.text(u)
        XCTAssertEqual(text, "DE:Hello there")
        let calls = await pipe.calls
        XCTAssertEqual(calls, 3)
    }

    func testFirstUnitHasPriority() async throws {
        let gate = GatedTranslator()
        let pipe = LivePipeline(translator: gate, book: "b", sourceHash: "h", source: "en", target: "de")
        let units = book(1, unitsEach: 12, chars: 10)(0)
        await pipe.prefetch(units)
        await gate.waitForCalls(2)  // erste Einheit allein + Stapel aus 11
        await gate.release(size: 1)
        let first = try await pipe.text(units[0])
        XCTAssertEqual(first, "DE:" + units[0].text)
        let rest = await pipe.ready(units[5])
        XCTAssertNil(rest)  // der Stapel läuft noch, die erste Einheit wartete nicht darauf
        await gate.release(size: 11)
        let fifth = try await pipe.text(units[5])
        XCTAssertEqual(fifth, "DE:" + units[5].text)
    }

    func testRetryBudgetFailThenEchoIsTwoCalls() async throws {
        let long = "This is a long sentence ok"
        let fake = FakeTranslator([.fail, .echo, .echo])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        let text = try await pipe.text(unit(long))
        XCTAssertEqual(text, long)  // zweite (gelungene) Antwort gilt, keine dritte Nachfrage
        let calls = await pipe.calls
        XCTAssertEqual(calls, 2)
    }

    func testRetryBudgetShortThenEmptyIsTwoCalls() async {
        let fake = FakeTranslator([.short, .empty, .ok])
        let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
        do { _ = try await pipe.text(unit("Hello there")); XCTFail("erwartet Fehler") }
        catch { XCTAssertEqual(error as? LiveTranslationError, .unusable) }
        let calls = await pipe.calls
        XCTAssertEqual(calls, 2)
    }

    func testFailedFollowUpIsPassedOnNotCached() async {
        let long = "This is a long sentence ok"
        for (script, expected) in [([FakeTranslator.Step.echo, .fail], nil as LiveTranslationError?),
                                   ([.echo, .short], .countMismatch)] {
            let fake = FakeTranslator(script)
            let pipe = LivePipeline(translator: fake, book: "b", sourceHash: "h", source: "en", target: "de")
            let u = unit(long)
            do { _ = try await pipe.text(u); XCTFail("Nachfragefehler muss ankommen") }
            catch {
                if let expected { XCTAssertEqual(error as? LiveTranslationError, expected) } else { XCTAssertTrue(error is Boom) }
            }
            let cached = await pipe.ready(u)
            XCTAssertNil(cached)  // der unveränderte Ausgangstext landet nicht im Speicher
        }
    }

    func testWaiterFailsOnGenerationChangeEvenIfTranslatorIgnoresCancel() async throws {
        let gate = GatedTranslator()
        let pipe = LivePipeline(translator: gate, book: "b", sourceHash: "h", source: "en", target: "de")
        let u = unit("Hello there")
        let ok = Task { try await pipe.text(u) }
        await gate.waitForCalls(1)
        await pipe.cancelPending()
        await gate.release(.success(()))  // spätes Ergebnis trotz Abbruch
        do { _ = try await ok.value; XCTFail("alte Generation darf nicht zurückkommen") }
        catch { XCTAssertTrue(error is CancellationError) }
        let late = await pipe.ready(u)
        XCTAssertNil(late)

        // Später Fehler statt Ergebnis: der Wartende bekommt trotzdem CancellationError.
        let failing = Task { try await pipe.text(u) }
        await gate.waitForCalls(2)
        await pipe.cancelPending()
        await gate.release(.failure(Boom()))   // später Fehler; abgebrochen, daher keine Wiederholung
        do { _ = try await failing.value; XCTFail("erwartet Abbruch") }
        catch { XCTAssertTrue(error is CancellationError, "\(error)") }
        let gen = await pipe.currentGeneration
        XCTAssertEqual(gen, 2)
    }

    /// Catch-Guard gezielt: erste Antwort unverändert, Nachfrage blockiert, Generation wechselt,
    /// dann endet die Nachfrage mit einem fremden Fehler. Der Wartende bekommt CancellationError, nicht Boom.
    func testGenerationWinsOverLateFollowUpError() async throws {
        let gate = GatedTranslator()
        let pipe = LivePipeline(translator: gate, book: "b", sourceHash: "h", source: "en", target: "de")
        let u = unit("This is a long sentence ok")
        let waiter = Task { try await pipe.text(u) }
        await gate.waitForCalls(1)
        await gate.release(echo: true)          // erste Antwort: unverändert → Nachfrage
        await gate.waitForCalls(2)              // Nachfrage hängt
        await pipe.cancelPending()
        await gate.release(.failure(Boom()))    // Nachfrage scheitert nach dem Wechsel
        do { _ = try await waiter.value; XCTFail("erwartet Abbruch") }
        catch { XCTAssertTrue(error is CancellationError, "\(error)") }
        let cached = await pipe.ready(u)
        XCTAssertNil(cached)
    }

    func testCancelDoesNotRemoveNewerRequests() async throws {
        let gate = GatedTranslator()
        let pipe = LivePipeline(translator: gate, book: "b", sourceHash: "h", source: "en", target: "de")
        let u = unit("Hello there")
        await pipe.prefetch([u])
        await gate.waitForCalls(1)
        await pipe.cancelPending()
        await pipe.prefetch([u])  // neuer Auftrag derselben Einheit
        await gate.waitForCalls(2)
        await gate.release(.success(()))  // alter Auftrag endet zuerst und räumt auf
        // Der neue Auftrag steht noch: text() hängt sich an ihn statt einen dritten zu starten.
        let waiter = Task { try await pipe.text(u) }
        await gate.release(.success(()))
        let text = try await waiter.value
        XCTAssertEqual(text, "DE:Hello there")
        let started = await gate.started
        XCTAssertEqual(started, 2)
    }
}
