import XCTest
@testable import VoiceCore

@MainActor
final class ReadingPageEditingTests: XCTestCase {
    private func intakeBook(_ source: String = "Fotos (Texterkennung)") -> LibraryBook {
        LibraryBook(id: "synthetic-book", title: "Probe", text: LibraryTextVersion(
            pageId: "ios_synthetic-book", origin: "document", sourceName: source), addedMs: 1)
    }

    func testIsConvertibleOnlyForPhotoAndScanBooks() {
        XCTAssertTrue(ReadingPageEditor.isConvertible(intakeBook()))
        XCTAssertTrue(ReadingPageEditor.isConvertible(intakeBook("Scan (Texterkennung)")))
        XCTAssertFalse(ReadingPageEditor.isConvertible(intakeBook("Eingefügter Text")))
        var foreign = intakeBook()
        foreign.text?.pageId = "ios_other"
        XCTAssertFalse(ReadingPageEditor.isConvertible(foreign))
        var noText = intakeBook()
        noText.text = nil
        XCTAssertFalse(ReadingPageEditor.isConvertible(noText))
    }

    func testConversionRoundTripAndRetryAfterSaveBeforeRemoval() throws {
        let e = try editor()
        let book = intakeBook()
        let text = "  Probe 😀\n\nZweite Zeile.  "
        let progress = ProgressRecord(offset: 8, length: Double(text.utf16.count), updated: 7)
        let suite = "conversion-test-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        var kept = true
        let first = try XCTUnwrap(e.convert(book: book, text: { text }, progress: progress, defaults: defaults) { page in
            XCTAssertEqual(try e.store.page(page.id)?.text, text)
            XCTAssertEqual(ReadingPageLocalProgress.record(ReadingPagePlaybackSource.progressPrefix + page.id, defaults: defaults), progress)
            // Abbruch nach dem Sichern: das Buch bleibt zunächst vorhanden.
        })
        let reopened = ReadingPageEditor(store: try ReadingPageStore(root: e.store.root))
        let second = try XCTUnwrap(reopened.convert(book: book, text: { XCTFail("Erneutes Lesen"); return nil }, defaults: defaults) { _ in kept = false })
        XCTAssertEqual(first.id, second.id)
        XCTAssertEqual(second.title, book.title)
        XCTAssertEqual(second.text, text)
        XCTAssertEqual(second.source, "iPhone-Foto")
        XCTAssertEqual(ReadingPageLocalProgress.record(ReadingPagePlaybackSource.progressPrefix + second.id, defaults: defaults), progress)
        XCTAssertFalse(kept)
        XCTAssertEqual(try reopened.store.pages().count, 1)
        XCTAssertEqual(try reopened.store.captures(for: first.id), [])
    }

    func testConversionSaveFailureKeepsBook() throws {
        let e = try editor()
        try FileManager.default.removeItem(at: e.store.root)
        try Data().write(to: e.store.root)
        var kept = true
        XCTAssertThrowsError(try e.convert(book: intakeBook(), text: { "Probe" }) { _ in kept = false })
        XCTAssertTrue(kept)
    }

    func testConversionOnlyAcceptsExactLegacyPhotoAndScanOrigins() throws {
        let e = try editor()
        for source in ["Eingefügter Text", "Teilen-Menü (Texterkennung)", "Fotos", "iPhone-Foto"] {
            XCTAssertNil(try e.convert(book: intakeBook(source), text: { XCTFail("Text gelesen"); return "Probe" }) { _ in XCTFail("Buch entfernt") })
        }
        var foreign = intakeBook()
        foreign.text?.origin = "transcript"
        XCTAssertNil(try e.convert(book: foreign, text: { "Probe" }) { _ in XCTFail("Buch entfernt") })
        let scan = try XCTUnwrap(e.convert(book: intakeBook("Scan (Texterkennung)"), text: { "Scan" }) { _ in })
        XCTAssertEqual(scan.source, "iPhone-Scan")
    }

    func testConversionRetryPreservesEditedPageAndNewerProgress() throws {
        let e = try editor()
        let suite = "conversion-test-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let old = ProgressRecord(offset: 2, length: 12, updated: 1)
        let first = try XCTUnwrap(e.convert(book: intakeBook(), text: { "Originaltext" }, progress: old, defaults: defaults) { _ in })
        try e.updateText(first.id, text: "Bearbeitet")
        let key = ReadingPagePlaybackSource.progressPrefix + first.id
        let newer = ProgressRecord(offset: 8, length: 10, updated: 2)
        ReadingPageLocalProgress.put(key, newer, defaults: defaults)
        let retry = try XCTUnwrap(e.convert(book: intakeBook(), text: { "Originaltext" }, progress: old, defaults: defaults) { _ in })
        XCTAssertEqual(retry.text, "Bearbeitet")
        XCTAssertEqual(ReadingPageLocalProgress.record(key, defaults: defaults), newer)
        try e.store.delete(first.id)
        XCTAssertNil(try e.convert(book: intakeBook(), text: { "Originaltext" }, defaults: defaults) { _ in XCTFail("Gelöschte Seite") })
        XCTAssertTrue(try e.store.pages().isEmpty)
    }

    func testIntakeSourcesArePersistedWhenCreatingPages() throws {
        let e = try editor()
        for (input, expected) in [("Fotos (Texterkennung)", "iPhone-Foto"), ("Scan (Texterkennung)", "iPhone-Scan"), ("Eingefügter Text", "iPhone-Text")] {
            let page = try e.create(text: "Probe", title: "Probe", source: input)
            XCTAssertEqual(try e.store.page(page.id)?.source, expected)
        }
        XCTAssertEqual(try e.create(text: "Probe", title: "Probe").source, "")
    }

    func testConversionDoesNotReviveRemoteTombstoneWithoutJSON() throws {
        let e = try editor()
        // Ziel-ID in einem getrennten Speicher bestimmen; hier kommt nur der Grabstein an.
        let target = try XCTUnwrap(try editor().convert(book: intakeBook(), text: { "Probe" }) { _ in })
        try e.store.apply(id: target.id, json: nil, revision: 7, deviceName: "Probe")
        let reopened = ReadingPageEditor(store: try ReadingPageStore(root: e.store.root))
        XCTAssertNil(try reopened.store.archivedPage(target.id))
        XCTAssertNil(try reopened.convert(book: intakeBook(), text: {
            XCTFail("Grabstein darf keinen Text lesen"); return "Probe"
        }) { _ in XCTFail("Buch darf nicht entfernt werden") })
        XCTAssertTrue(try reopened.store.pages().isEmpty)
    }

    func testConversionTransfersFinishedFlagWithoutProgressRecord() throws {
        let e = try editor()
        var book = intakeBook()
        book.finished = true
        let suite = "conversion-test-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let page = try XCTUnwrap(e.convert(book: book, text: { "Probe 😀" }, defaults: defaults) { _ in })
        let rec = ReadingPageLocalProgress.record(ReadingPagePlaybackSource.progressPrefix + page.id, defaults: defaults)
        XCTAssertEqual(rec?.finished, true)
        XCTAssertEqual(rec?.length, Double(page.text.utf16.count))
    }

    private func editor() throws -> ReadingPageEditor {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return ReadingPageEditor(store: try ReadingPageStore(root: root))
    }

    private func capture(_ id: String, _ text: String) -> ReadingCapture {
        ReadingCapture(id: id, image: id + ".jpg", recognizedText: text, source: "Probe", createdMs: 1)
    }

    func testDebounceIgnoresOldCallbackAndFlushSavesLatestText() throws {
        let e = try editor()
        let p = try e.create(text: "Basis", title: "Probe")
        let buffer = PendingTextBuffer()
        let old = buffer.set(p.id, text: "Erste Eingabe")
        let latest = buffer.set(p.id, text: "Letzte Eingabe")
        let save = { (id: String, text: String) in try e.updateText(id, text: text) }
        XCTAssertFalse(try buffer.flushIfCurrent(p.id, revision: old, save: save))
        XCTAssertEqual(try e.page(p.id)?.text, "Basis")
        XCTAssertTrue(try buffer.flushIfCurrent(p.id, revision: latest, save: save))
        XCTAssertEqual(try e.page(p.id)?.text, "Letzte Eingabe")
        XCTAssertTrue(buffer.pageIDs.isEmpty)
        XCTAssertFalse(try buffer.flushIfCurrent(p.id, revision: latest, save: save))
        _ = buffer.set(p.id, text: "Beim Verlassen")
        try buffer.flush(save: save)
        XCTAssertEqual(try e.page(p.id)?.text, "Beim Verlassen")
    }

    func testFailedSaveRetainsPendingTextAndCanBeRetried() throws {
        let buffer = PendingTextBuffer()
        _ = buffer.set("page_probe", text: "Eingabe")
        XCTAssertThrowsError(try buffer.flush { _, _ in throw ReadingPageError.persistence })
        XCTAssertEqual(buffer.pageIDs, ["page_probe"])
        var saved = ""
        try buffer.flush { _, text in saved = text }
        XCTAssertEqual(saved, "Eingabe")
        XCTAssertTrue(buffer.pageIDs.isEmpty)
    }

    func testPageFlushLeavesOtherPagesAndNewerInputPending() throws {
        let buffer = PendingTextBuffer()
        _ = buffer.set("page_a", text: "A")
        _ = buffer.set("page_b", text: "B")
        var saved: [String] = []
        try buffer.flush("page_a") { id, text in
            saved.append(text)
            // Auch eine synchrone neue Eingabe beim Speichern darf nicht verschwinden.
            _ = buffer.set(id, text: "Neuer")
        }
        XCTAssertEqual(saved, ["A"])
        XCTAssertEqual(buffer.pageIDs, ["page_a", "page_b"])
        try buffer.flush("page_a") { _, text in saved.append(text) }
        XCTAssertEqual(saved, ["A", "Neuer"])
        XCTAssertEqual(buffer.pageIDs, ["page_b"])
    }

    func testMissingPageDoesNotSilentlyConsumePendingText() throws {
        let e = try editor()
        let buffer = PendingTextBuffer()
        _ = buffer.set("page_missing", text: "Eingabe")
        XCTAssertThrowsError(try buffer.flush { id, text in try e.updateText(id, text: text) })
        XCTAssertEqual(buffer.pageIDs, ["page_missing"])
    }

    func testPullPreservesPendingInputAsConflictIncludingTombstone() throws {
        for (deleted, flush) in [(false, false), (false, true), (true, false), (true, true)] {
            let e = try editor()
            defer { e.store.willApplyRemote = nil }
            let p = try e.create(text: "Basis", title: "Probe")
            let sent = try XCTUnwrap(e.store.pending().first)
            try e.store.acknowledge(sent, revision: 1)
            let buffer = PendingTextBuffer()
            _ = buffer.set(p.id, text: "Ungesicherte Eingabe")
            if flush {
                e.store.willApplyRemote = {
                    try buffer.flush { id, text in try e.updateText(id, text: text) }
                }
            }
            var remote = p
            remote.text = "Server"
            try e.store.apply(id: p.id, json: deleted ? nil : remote.json(), revision: 2, deviceName: "Probe")
            let copy = try e.store.pages().first { $0.id != p.id }
            // Gegenprobe ohne Hook: Die jüngste Eingabe landet nicht in einer Konfliktkopie.
            XCTAssertEqual(copy?.text, flush ? "Ungesicherte Eingabe" : nil)
            XCTAssertEqual(try e.page(p.id)?.text, deleted ? nil : "Server")
            XCTAssertEqual(buffer.pageIDs.isEmpty, flush)
        }
    }

    func testFailedFlushAbortsPullBeforeChangingOriginal() throws {
        let e = try editor()
        let p = try e.create(text: "Basis", title: "Probe")
        e.store.willApplyRemote = { throw ReadingPageError.persistence }
        XCTAssertThrowsError(try e.store.apply(id: p.id, json: nil, revision: 2, deviceName: "Probe"))
        XCTAssertEqual(try e.page(p.id)?.text, "Basis")
    }

    func testInterruptedRecognitionCannotApplyAfterNewOperation() throws {
        let e = try editor()
        let p = try e.create(text: "", title: "Probe")
        var gate = DraftGuard()
        gate.switchTo(UUID())
        let interrupted = gate.ticket()
        gate.switchTo(nil)
        gate.switchTo(UUID())
        let current = gate.ticket()
        // Injizierte OCR-Fortsetzung nach dem Warten, ohne Bilder oder OCR-Dienst.
        func complete(_ ticket: DraftGuard.Ticket, text: String) throws {
            guard gate.isValid(ticket), try e.page(p.id) != nil else { return }
            _ = try e.insert([capture("a", text)], into: p.id, after: nil)
        }
        try complete(interrupted, text: "Veraltet")
        XCTAssertTrue(try e.store.captures(for: p.id).isEmpty)
        try complete(current, text: "Aktuell")
        XCTAssertEqual(try e.page(p.id)?.text, "Aktuell")
    }

    func testAppendAfterMarkedCaptureAndComposeWithoutEdits() throws {
        let e = try editor()
        let p = try e.create(text: "", title: "Probe")
        XCTAssertNil(try e.insert([capture("a", "A"), capture("c", "C")], into: p.id, after: nil))
        XCTAssertNil(try e.insert([capture("b", "B")], into: p.id, after: "a"))
        XCTAssertEqual(try e.store.captures(for: p.id).map(\.id), ["a", "b", "c"])
        XCTAssertEqual(try e.page(p.id)?.text, "A\n\nB\n\nC")
    }

    func testMoveAndDeleteComposeOnlyWithoutEdits() throws {
        for edited in [false, true] {
            let e = try editor()
            let p = try e.create(text: "", title: "Probe")
            _ = try e.insert([capture("a", "A"), capture("b", "B")], into: p.id, after: nil)
            if edited { try e.updateText(p.id, text: "Eigener Text") }
            let moved = try e.move(p.id, from: IndexSet(integer: 0), to: 2)
            XCTAssertEqual(moved != nil, edited)
            // Umsortieren/Entfernen bringt keinen neuen Text: „Anhängen“ darf nicht angeboten werden.
            XCTAssertEqual(moved?.appendedText ?? "", "")
            XCTAssertEqual(try e.page(p.id)?.text, edited ? "Eigener Text" : "B\n\nA")
            let deleted = try e.deleteCapture(p.id, captureID: "b")
            XCTAssertEqual(deleted != nil, edited)
            XCTAssertEqual(deleted?.appendedText ?? "", "")
            XCTAssertEqual(try e.page(p.id)?.text, edited ? "Eigener Text" : "A")
        }
    }

    func testRerecognitionRequiresChoiceAndCancelKeepsEditedText() throws {
        let e = try editor()
        let p = try e.create(text: "", title: "Probe")
        _ = try e.insert([capture("a", "Alt")], into: p.id, after: nil)
        try e.updateText(p.id, text: "Bearbeitet")
        let choice = try XCTUnwrap(e.recognized("Neu", pageID: p.id, captureID: "a"))
        XCTAssertEqual(try e.page(p.id)?.text, "Bearbeitet")
        XCTAssertTrue(try e.resolve(choice, with: .cancel))
        XCTAssertEqual(try e.page(p.id)?.text, "Bearbeitet")
        XCTAssertEqual(try e.store.captures(for: p.id).first?.recognizedText, "Alt")
        XCTAssertNil(try e.store.captures(for: p.id).first?.pendingRecognition)
        let again = try XCTUnwrap(e.recognized("Noch neuer", pageID: p.id, captureID: "a"))
        XCTAssertTrue(try e.resolve(again, with: .append))
        XCTAssertEqual(try e.page(p.id)?.text, "Bearbeitet\n\nNoch neuer")
        let replace = try XCTUnwrap(e.recognized("Ersetzt", pageID: p.id, captureID: "a"))
        XCTAssertTrue(try e.resolve(replace, with: .replace))
        XCTAssertEqual(try e.page(p.id)?.text, "Ersetzt")
        XCTAssertFalse(try e.store.requiresCompositionChoice(p.id))
    }

    func testStaleChoiceCannotOverwriteNewerText() throws {
        let e = try editor()
        let p = try e.create(text: "Manuell", title: "Probe")
        let choice = try XCTUnwrap(e.insert([capture("a", "A")], into: p.id, after: nil))
        try e.updateText(p.id, text: "Neuer")
        XCTAssertFalse(try e.resolve(choice, with: .replace))
        XCTAssertEqual(try e.page(p.id)?.text, "Neuer")
    }

    func testStaleChoiceCannotUndoAnotherCaptureChange() throws {
        let e = try editor()
        let p = try e.create(text: "Manuell", title: "Probe")
        let choice = try XCTUnwrap(e.insert([capture("a", "A")], into: p.id, after: nil))
        _ = try e.insert([capture("b", "B")], into: p.id, after: "a")
        XCTAssertFalse(try e.resolve(choice, with: .replace))
        XCTAssertEqual(try e.store.captures(for: p.id).map(\.id), ["a", "b"])
    }

    func testRerecognitionWithoutEditsAndMissingInsertionMarker() throws {
        let e = try editor()
        let p = try e.create(text: "", title: "Probe")
        _ = try e.insert([capture("a", "A")], into: p.id, after: nil)
        XCTAssertNil(try e.recognized("Neu", pageID: p.id, captureID: "a"))
        XCTAssertEqual(try e.page(p.id)?.text, "Neu")
        XCTAssertThrowsError(try e.insert([capture("b", "B")], into: p.id, after: "fehlt"))
        XCTAssertEqual(try e.store.captures(for: p.id).map(\.id), ["a"])
    }

    func testLimitsRejectInsteadOfTruncatingAndKeepOriginalText() throws {
        let e = try editor()
        let p = try e.create(text: "Original", title: "Probe")
        XCTAssertThrowsError(try e.updateText(p.id, text: String(repeating: "x", count: TextIntake.maxCharacters + 1)))
        XCTAssertEqual(try e.page(p.id)?.text, "Original")
        let captures = (0...TextInbox.maxImages).map { capture(String($0), "A") }
        XCTAssertThrowsError(try e.insert(captures, into: p.id, after: nil))
        XCTAssertEqual(try e.store.captures(for: p.id).count, 0)
        XCTAssertEqual(ReadingPageEditor.appending("", to: "A"), "A")
        XCTAssertEqual(ReadingPageEditor.appending("B", to: ""), "B")
    }

    func testPageProgressSurvivesReloadAndStaysOutsideBookProgress() throws {
        let name = "ReadingPageTests." + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name) }
        let key = ReadingPagePlaybackSource.progressPrefix + "page_probe"
        let record = ProgressRecord(offset: 5, length: 20, updated: 1)
        ReadingPageLocalProgress.put(key, record, defaults: defaults)
        XCTAssertEqual(ReadingPageLocalProgress.record(key, defaults: try XCTUnwrap(UserDefaults(suiteName: name))), record)
        let bookKey = LibraryProgress.textKey(pageId: "page_probe")
        XCTAssertNil(defaults.object(forKey: bookKey))
        ReadingPageLocalProgress.put(bookKey, record, defaults: defaults)
        XCTAssertNil(ReadingPageLocalProgress.record(bookKey, defaults: defaults))
        XCTAssertNil(defaults.object(forKey: bookKey))
    }

    func testModificationOrderAndCompositionBaselineSurviveReload() throws {
        let e = try editor()
        let a = try ReadingPage(id: "page_z", title: "A")
        var b = try ReadingPage(id: "page_a", title: "B")
        b.text = "B"
        try e.store.save(a)
        try e.store.setCaptures([], for: a.id, composedText: "")
        try e.store.save(b)
        _ = try e.insert([capture("a", "Aufnahme")], into: a.id, after: nil)
        // Feste lokale Zeiten statt Warten oder Annahmen über die Auflösung der Uhr.
        for (id, ms) in [(a.id, 20), (b.id, 10)] {
            let url = e.store.root.appendingPathComponent(id + ".json")
            var record = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
            record["modifiedMs"] = ms
            try JSONSerialization.data(withJSONObject: record).write(to: url)
        }
        let reopened = try ReadingPageStore(root: e.store.root)
        XCTAssertEqual(try reopened.pages().map(\.id), [a.id, b.id])
        XCTAssertFalse(try reopened.requiresCompositionChoice(a.id))
        XCTAssertTrue(try reopened.requiresCompositionChoice(b.id))
        XCTAssertEqual(try reopened.composedText(a.id), "Aufnahme")
    }

    func testLateRecognitionAfterDeletionDoesNotResurrectPage() throws {
        let e = try editor()
        let p = try e.create(text: "", title: "Probe")
        _ = try e.insert([capture("a", "A")], into: p.id, after: nil)
        try e.store.delete(p.id)
        XCTAssertNil(try e.recognized("Spät", pageID: p.id, captureID: "a"))
        XCTAssertNil(try e.page(p.id))
        XCTAssertEqual(try e.store.archivedPage(p.id)?.text, "A")
        var gate = DraftGuard()
        gate.switchTo(UUID())
        let ticket = gate.ticket()
        gate.switchTo(nil)
        XCTAssertFalse(gate.isValid(ticket))
    }

    func testPlaybackSourceIsTransientAndHasNoAudiobook() throws {
        let e = try editor()
        let p = try e.create(text: "Probe", title: "Seite")
        let before = try e.store.pending()
        let source = ReadingPagePlaybackSource(page: p)
        XCTAssertEqual(source.book.id, "page:" + p.id)
        XCTAssertNil(source.book.audio)
        XCTAssertEqual(source.text, "Probe")
        XCTAssertEqual(try e.store.pending(), before)
        XCTAssertEqual(try e.store.pages().count, 1)
    }
}
