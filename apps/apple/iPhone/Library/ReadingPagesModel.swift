import Foundation
import Combine

/// Seiten und Originalaufnahmen. Die Oberfläche in S2b nutzt ausschließlich diesen Einstieg.
@MainActor
final class ReadingPagesModel: ObservableObject {
    static let shared = ReadingPagesModel()

    @Published private(set) var pages: [ReadingPage] = []
    /// Erst nach dem ersten Laden (abseits des Hauptthreads) gilt eine leere Liste als „keine Seiten“.
    @Published private(set) var loaded = false
    private var initialLoad: Task<Void, Never>?
    @Published private(set) var recognizing: String?
    @Published var error: String?
    @Published var compositionChoice: CompositionChoice?

    private var recognitionGuard = DraftGuard()
    private let pendingText = PendingTextBuffer()
    private var textTasks: [String: Task<Void, Never>] = [:]
    /// Titeleingaben wie Text entprellt; `prepareForRemote` und `flushPending` sichern sie sofort.
    private var pendingTitles: [String: String] = [:]
    private var titleTasks: [String: Task<Void, Never>] = [:]
    private var syncTask: Task<Void, Never>?
    private var deletionErrors: AnyCancellable?
    private var lastDeletionError: String?

    init() {
        // Kaltstart 27.11.: Das synchrone Parsen aller Seiten hielt den Start ~5 s schwarz (die Reiter entstehen mit
        // dem ersten Bild). Jetzt parst eine Hintergrundaufgabe; danach liest `reload` nur Dateistempel.
        initialLoad = Task { [weak self] in await self?.loadInitial() }
        deletionErrors = LibrarySync.shared.$deletionError.sink { [weak self] message in
            guard let self else { return }
            if let message { self.error = message }
            else if self.error == self.lastDeletionError { self.error = nil }
            self.lastDeletionError = message
        }
    }

    private func editor() throws -> ReadingPageEditor {
        ReadingPageEditor(store: try LibrarySync.shared.readingPageStore())
    }

    private func loadInitial() async {
        let began = ContinuousClock.now
        if let store = try? LibrarySync.shared.readingPageStore() {
            let root = store.root
            let preloaded = await Task.detached(priority: .userInitiated) { ReadingPageStore.preload(root: root) }.value
            store.adopt(preloaded)
        }
        reload()
        PhoneDiagnostics.note("vorlese-seiten geladen \(pages.count) \(PhoneDiagnostics.ms(since: began))ms")
    }

    func reload() {
        defer { loaded = true }
        do {
            try prepareForRemote()
            pages = try editor().store.pages()
        } catch { report(error) }
    }

    /// Editor-Ende und Szenenwechsel sichern ohne Warten auf die Entprellung.
    func flushPending(_ pageID: String? = nil) {
        do {
            if let pageID { try flushTitle(pageID); try flushText(pageID) }
            else { try prepareForRemote() }
            pages = try editor().store.pages()
        } catch { report(error) }
    }

    /// Der Store ruft dies im selben MainActor-Schritt wie seine Konfliktprüfung auf.
    func prepareForRemote() throws {
        for id in Array(pendingTitles.keys) { try flushTitle(id) }
        for id in pendingText.pageIDs { try flushText(id) }
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
            let page = try editor().create(text: cleaned, title: pageTitle(title, text: cleaned), source: source)
            changed()
            return page
        } catch { report(error); return nil }
    }

    func canConvert(_ book: LibraryBook) -> Bool { ReadingPageEditor.isConvertible(book) }

    /// Erst dauerhaft sichern; danach entfernt der bestehende Sync-Weg das Buch nach bestätigtem Grabstein.
    /// Nicht im Konto (oder abgemeldet): kein Grabstein möglich, das Buch wird wie bei „Vom iPhone entfernen“ lokal entfernt.
    func convert(book: LibraryBook) -> ReadingPage? {
        let library = PhoneLibrary.shared
        let sync = LibrarySync.shared
        guard canConvert(book) else { error = "Dieses Buch lässt sich nicht in eine Vorlese-Seite umwandeln."; return nil }
        var saved = false
        do {
            let e = try editor()
            let page = try e.convert(book: book, text: { library.text(book) },
                                     progress: library.record(library.textKey(book))) { _ in
                saved = true
                if sync.isSynced(book) { try sync.queueBookDeletion(book) }
                else {
                    library.delete(book)
                    guard !library.books.contains(where: { $0.id == book.id }) else { throw ReadingPageError.persistence }
                }
            }
            guard let page else {
                error = "Die Vorlese-Seite zu diesem Buch wurde bereits gelöscht oder das Buch hat keinen Text."
                return nil
            }
            changed()
            return page
        } catch {
            // Die Seite ist schon gesichert: ehrlich melden, dass nur das Entfernen des Buchs fehlt (erneuter Aufruf ist idempotent).
            if saved {
                changed()
                self.error = "Die Vorlese-Seite ist angelegt, aber das Buch konnte noch nicht aus der Bibliothek entfernt werden. Bitte erneut umwandeln."
            } else { report(error) }
            return nil
        }
    }

    func createPage(from sources: [ImageSource], source: String) async -> ReadingPage? {
        guard !sources.isEmpty, canRecognize else { return nil }
        do {
            let page = try editor().create(text: "", title: TextIntakeModel.fallbackTitle(), source: source)
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

    /// Eine Editor-Fassung, die nicht mehr auf dem aktuellen Stand der Seite aufsetzt, als eigene Seite
    /// sichern (wie eine Sync-Konfliktkopie), statt sie zu verwerfen. Text unverändert, ohne Bereinigung.
    @discardableResult
    func saveConflictCopy(of pageID: String, text: String, title: String? = nil) -> ReadingPage? {
        let typedTitle = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !typedTitle.isEmpty else { return nil }
        do {
            let e = try editor()
            let base = typedTitle.isEmpty
                ? (try e.page(pageID)?.title ?? pages.first(where: { $0.id == pageID })?.title ?? "Seite")
                : typedTitle
            let copy = try e.create(text: text, title: base + " (nicht übernommene Eingabe)")
            changed()
            return copy
        } catch { report(error); return nil }
    }

    /// Titel aus dem Editor: sofort in `pages` sichtbar, gespeichert nach ~0,8 s Pause.
    func updateTitle(_ pageID: String, title: String) {
        guard let index = pages.firstIndex(where: { $0.id == pageID }) else { return }
        pendingTitles[pageID] = title
        pages[index].title = pageTitle(title, text: pages[index].text)
        titleTasks[pageID]?.cancel()
        titleTasks[pageID] = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 800_000_000) } catch { return }
            guard let self else { return }
            do {
                if try self.flushTitle(pageID) { self.changed() }
            } catch { self.report(error) }
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
        let revision = pendingText.set(pageID, text: text)
        textTasks[pageID]?.cancel()
        textTasks[pageID] = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 800_000_000) } catch { return }
            guard let self else { return }
            do {
                if try self.pendingText.flushIfCurrent(pageID, revision: revision, save: { id, text in
                    try self.editor().updateText(id, text: text)
                }) { self.changed() }
            } catch { self.report(error) }
        }
    }

    /// Bereinigung und Typografie bleiben wie am Desktop getrennte Regelstufen.
    func aufbereitenVorschlaege(pageID: String, stufe: TextAufbereiten.Stufe = .bereinigung) async throws -> TextAufbereiten.Vorschlaege {
        try flushText(pageID)
        guard let page = try editor().page(pageID) else { throw ReadingPageError.invalid }
        let text = page.text
        return await Task.detached(priority: .userInitiated) {
            TextAufbereiten.vorschlaege(text, stufe: stufe)
        }.value
    }

    func uebernehmen(pageID: String, angenommen: TextAufbereiten.Auswahl) async throws {
        try flushText(pageID)
        let e = try editor()
        guard let page = try e.page(pageID) else { throw ReadingPageError.invalid }
        let basis = page.text
        let text = try await Task.detached(priority: .userInitiated) {
            try angenommen.anwenden(auf: basis)
        }.value
        // Während der Berechnung können Eingaben, OCR oder Sync die Seite ändern.
        try flushText(pageID)
        guard let current = try e.page(pageID), TextAufbereiten.identisch(current.text, basis) else {
            error = "Die Seite wurde inzwischen geändert. Bitte die Vorschläge erneut erstellen."
            throw TextAufbereiten.Fehler.veraltet
        }
        guard !TextAufbereiten.identisch(text, basis) else { return }
        // ReadingPage hat keine aufbereitete Fassung im Sync-Schema. Die Desktop-
        // Fassungen liegen separat in aufbereitung/, nicht in state.json.
        try e.applyPreparedText(pageID, basis: basis, text: text)
        changed()
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

    @discardableResult
    private func flushTitle(_ id: String) throws -> Bool {
        titleTasks[id]?.cancel(); titleTasks[id] = nil
        guard let title = pendingTitles.removeValue(forKey: id) else { return false }
        let e = try editor()
        // Gelöschte Seite: Die Oberfläche sichert die Eingabe selbst (`saveConflictCopy`).
        guard var page = try e.page(id) else { return false }
        let wanted = pageTitle(title, text: page.text)
        guard wanted != page.title else { return false }
        page.title = wanted
        try e.store.save(page)
        scheduleSync()
        return true
    }

    private func flushText(_ id: String) throws {
        guard pendingText.pageIDs.contains(id) else { return }
        try pendingText.flush(id) { id, text in try editor().updateText(id, text: text) }
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
