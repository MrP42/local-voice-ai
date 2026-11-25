import XCTest
@testable import VoiceCore

@MainActor
final class ReadingPageEditingTests: XCTestCase {
    private func editor() throws -> ReadingPageEditor {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return ReadingPageEditor(store: try ReadingPageStore(root: root))
    }

    private func capture(_ id: String, _ text: String) -> ReadingCapture {
        ReadingCapture(id: id, image: id + ".jpg", recognizedText: text, source: "Probe", createdMs: 1)
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
            XCTAssertEqual(try e.page(p.id)?.text, edited ? "Eigener Text" : "B\n\nA")
            let deleted = try e.deleteCapture(p.id, captureID: "b")
            XCTAssertEqual(deleted != nil, edited)
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
