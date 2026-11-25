import Foundation
import Combine

/// Seiten und Originalaufnahmen. Die Oberfläche in S2b nutzt ausschließlich diesen Einstieg.
@MainActor
final class ReadingPagesModel: ObservableObject {
    static let shared = ReadingPagesModel()

    @Published private(set) var pages: [ReadingPage] = []
    @Published private(set) var recognizing: String?
    @Published var error: String?
    @Published var compositionChoice: CompositionChoice?

    private var recognitionGuard = DraftGuard()
    private var pendingText: [String: String] = [:]
    private var textTasks: [String: Task<Void, Never>] = [:]
    private var syncTask: Task<Void, Never>?

    init() { reload() }

    private func editor() throws -> ReadingPageEditor {
        ReadingPageEditor(store: try LibrarySync.shared.readingPageStore())
    }

    func reload() {
        do {
            for id in Array(pendingText.keys) { try flushText(id) }
            pages = try editor().store.pages()
        } catch { report(error) }
    }

    func captures(of pageID: String) -> [ReadingCapture] {
        do { return try editor().store.captures(for: pageID) }
        catch { report(error); return [] }
    }

    func imageURL(_ capture: ReadingCapture, page pageID: String) -> URL? {
        do { return try editor().store.imageURL(named: capture.image, for: pageID) }
        catch { report(error); return nil }
    }

    func createPage(fromText text: String, title: String?, source: String) -> ReadingPage? {
        let cleaned = TextIntake.clean(text)
        guard !cleaned.isEmpty else { error = "Es gibt noch keinen Text zum Vorlesen."; return nil }
        do {
            let page = try editor().create(text: cleaned, title: pageTitle(title, text: cleaned))
            changed()
            return page
        } catch { report(error); return nil }
    }

    func createPage(from sources: [ImageSource], source: String) async -> ReadingPage? {
        guard !sources.isEmpty, canRecognize else { return nil }
        do {
            let page = try editor().create(text: "", title: TextIntakeModel.fallbackTitle())
            changed()
            await append(sources, to: page.id, after: nil, source: source)
            return try editor().page(page.id)
        } catch { report(error); return nil }
    }

    func append(_ sources: [ImageSource], to pageID: String, after captureID: String?, source: String) async {
        guard !sources.isEmpty, canRecognize else { return }
        do {
            try flushText(pageID)
            let e = try editor()
            guard try e.page(pageID) != nil else { return }
            let existing = try e.store.captures(for: pageID)
            if let captureID, !existing.contains(where: { $0.id == captureID }) { return }
            let available = max(0, TextInbox.maxImages - existing.count)
            var notes: [String] = []
            if sources.count > available { notes.append("Nur die ersten \(available) Bilder wurden gelesen.") }
            var total = try existing.reduce(0) { sum, capture in
                sum + (TextInbox.fileSize(try e.store.imageURL(named: capture.image, for: pageID)) ?? 0)
            }
            beginRecognition(pageID)
            let ticket = recognitionGuard.ticket()
            defer { recognizing = nil }
            var added: [ReadingCapture] = []
            var unreadable = 0, tooBig = 0
            for item in sources.prefix(available) {
                let name = UUID().uuidString + ".image"
                do {
                    let result = try await item.read(total: &total, saving: { data in
                        try e.store.storeImage(data, named: name, for: pageID)
                    })
                    guard recognitionGuard.isValid(ticket), try e.page(pageID) != nil else { return }
                    added.append(ReadingCapture(image: name, recognizedText: result.text, source: source,
                                                createdMs: PhoneLibrary.nowMs()))
                } catch TextInbox.Failure.tooLarge { tooBig += 1 }
                catch ReadingPageError.persistence { throw ReadingPageError.persistence }
                catch { unreadable += 1 }
                guard recognitionGuard.isValid(ticket), try e.page(pageID) != nil else { return }
            }
            // Während OCR kann Text bearbeitet oder die Aufnahmefolge geändert worden sein.
            try flushText(pageID)
            compositionChoice = try e.insert(added, into: pageID, after: captureID)
            if unreadable > 0 { notes.append("\(unreadable) Bild(er) konnten nicht gelesen werden.") }
            if tooBig > 0 { notes.append("\(tooBig) Bild(er) ausgelassen (je bis 40 MB, zusammen bis 200 MB).") }
            if !added.isEmpty, ReadingCapture.compose(added).isEmpty { notes.append("Auf den Bildern wurde kein Text erkannt.") }
            changed()
            error = notes.isEmpty ? nil : notes.joined(separator: " ")
        } catch { report(error) }
    }

    func append(text: String, to pageID: String) {
        perform(pageID) { e in
            guard let page = try e.page(pageID) else { return }
            try e.updateText(pageID, text: ReadingPageEditor.appending(TextIntake.clean(text), to: page.text))
        }
    }

    func rename(_ pageID: String, to title: String) {
        perform(pageID) { e in
            guard var page = try e.page(pageID) else { return }
            page.title = pageTitle(title, text: page.text)
            try e.store.save(page)
        }
    }

    func updateText(_ pageID: String, text: String) {
        guard text.utf16.count <= TextIntake.maxCharacters else {
            error = "Der Text ist zu lang (höchstens 2 Millionen Zeichen)."; return
        }
        guard let index = pages.firstIndex(where: { $0.id == pageID }) else { return }
        pages[index].text = text
        pendingText[pageID] = text
        textTasks[pageID]?.cancel()
        textTasks[pageID] = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 800_000_000) } catch { return }
            guard let self else { return }
            do { try self.flushText(pageID); self.changed() } catch { self.report(error) }
        }
    }

    func moveCaptures(_ pageID: String, from: IndexSet, to: Int) {
        perform(pageID) { compositionChoice = try $0.move(pageID, from: from, to: to) }
    }

    func deleteCapture(_ pageID: String, captureID: String) {
        perform(pageID) { compositionChoice = try $0.deleteCapture(pageID, captureID: captureID) }
    }

    func rerecognize(_ pageID: String, captureID: String) async {
        guard canRecognize else { return }
        do {
            try flushText(pageID)
            let e = try editor()
            guard try e.page(pageID) != nil,
                  let capture = try e.store.captures(for: pageID).first(where: { $0.id == captureID }) else { return }
            let url = try e.store.imageURL(named: capture.image, for: pageID)
            beginRecognition(pageID)
            let ticket = recognitionGuard.ticket()
            defer { recognizing = nil }
            var total = 0
            let result: (text: String, bytes: Int)
            do { result = try await ImageSource.file(url, scoped: false).read(total: &total) }
            catch {
                guard recognitionGuard.isValid(ticket), try e.page(pageID) != nil else { return }
                if case TextInbox.Failure.tooLarge = error {
                    self.error = "1 Bild(er) ausgelassen (je bis 40 MB, zusammen bis 200 MB)."
                } else { self.error = "1 Bild(er) konnten nicht gelesen werden." }
                return
            }
            guard recognitionGuard.isValid(ticket), try e.page(pageID) != nil,
                  try e.store.captures(for: pageID).contains(capture) else { return }
            try flushText(pageID)
            compositionChoice = try e.recognized(result.text, pageID: pageID, captureID: captureID)
            changed()
        } catch { report(error) }
    }

    func resolve(_ choice: CompositionChoice, with action: CompositionAction) {
        perform(choice.pageID) { e in
            guard try e.resolve(choice, with: action) else {
                error = "Die Seite wurde inzwischen geändert. Bitte die Aufnahme erneut auswählen."
                compositionChoice = nil
                return
            }
            compositionChoice = nil
        }
    }

    func deletePage(_ pageID: String) {
        do {
            try flushText(pageID)
            try editor().store.delete(pageID)
            if recognizing == pageID { recognitionGuard.switchTo(nil) }
            if compositionChoice?.pageID == pageID { compositionChoice = nil }
            changed()
        } catch { report(error) }
    }

    func readAloud(_ pageID: String) {
        do {
            try flushText(pageID)
            if let page = try editor().page(pageID) { LibraryPlayback.shared.open(page: page, autoplay: true) }
        } catch { report(error) }
    }

    private var canRecognize: Bool {
        guard recognizing == nil else { error = "Es läuft noch eine Texterkennung. Bitte kurz warten."; return false }
        return true
    }

    private func beginRecognition(_ id: String) {
        recognizing = id
        recognitionGuard.switchTo(UUID())
        error = nil
    }

    private func flushText(_ id: String) throws {
        guard let text = pendingText[id] else { return }
        try editor().updateText(id, text: text)
        pendingText[id] = nil
        textTasks[id]?.cancel(); textTasks[id] = nil
        scheduleSync()
    }

    private func perform(_ id: String, _ operation: (ReadingPageEditor) throws -> Void) {
        do { try flushText(id); try operation(editor()); changed() }
        catch { report(error) }
    }

    private func changed() { reload(); scheduleSync() }

    private func scheduleSync() {
        syncTask?.cancel()
        syncTask = Task {
            do { try await Task.sleep(nanoseconds: 800_000_000) } catch { return }
            await LibrarySync.shared.run()
        }
    }

    private func pageTitle(_ title: String?, text: String) -> String {
        let typed = (title ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return typed.isEmpty ? TextIntake.title(for: String(text.prefix(2_000)), fallback: TextIntakeModel.fallbackTitle()) : typed
    }

    private func report(_ failure: Error) {
        if case ReadingPageError.tooLarge = failure {
            error = "Der Text ist zu lang (höchstens 2 Millionen Zeichen)."
        } else {
            error = "Der Text konnte nicht gespeichert werden. Bitte freien Speicher prüfen."
        }
    }
}
