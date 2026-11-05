import Foundation
import SwiftUI
import UIKit

/// Steuerung des Buch-Gesprächs (Epic #307, G5): verbindet Kern (`BookTalkSession`), Hören, Sprechen, Verlauf,
/// Bereich und Wiedergabe. Zustandsmaschine: idle → listening → thinking → speaking → idle.
/// Nichts davon läuft im Hintergrund: `sceneInactive()` beendet Mikrofon und Antwort (Plan §8.3).
@MainActor
final class BookTalkController: ObservableObject {
    enum Phase: Equatable { case idle, listening, thinking, speaking }

    /// Ausgewählter Beleg (Belegkarte).
    struct Selection: Equatable {
        let entryId: String
        let citation: BookTalkCitation
    }

    // MARK: Anzeige

    @Published private(set) var phase = Phase.idle
    @Published private(set) var conversation = BookTalkConversation()
    @Published private(set) var stableText = ""
    @Published private(set) var volatileText = ""
    @Published private(set) var scope = BookTalkScope.untilPosition
    @Published private(set) var page: Int?
    @Published private(set) var loading = true
    @Published private(set) var engine = BookTalkEngine.make(choice: .stored, enabled: false)
    @Published private(set) var choice = BookTalkModelChoice.stored
    @Published private(set) var enabled = BookTalkEngine.enabled
    @Published private(set) var selection: Selection?
    @Published var notice: String?
    @Published var typing = false
    @Published var draft = ""
    @Published var confirmingBook = false
    /// Mündliche Rückfrage „Auch spätere Kapitel einbeziehen? Sag Ja.“ läuft.
    @Published private(set) var awaitingScopeConfirm = false
    @Published private(set) var speaksAloud: Bool

    static let antonKey = "book_talk_anton"

    /// Eine Regel für Blatt und Einstellungen: ausdrücklich gewählt, sonst an, außer VoiceOver liest selbst vor.
    static func storedSpeaksAloud() -> Bool {
        (UserDefaults.standard.object(forKey: antonKey) as? Bool) ?? !UIAccessibility.isVoiceOverRunning
    }

    // MARK: Abhängigkeiten

    let book: LibraryBook
    var close: (() -> Void)?
    private let library: PhoneLibrary
    private let playback: LibraryPlayback
    private let store = BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory())
    private let listener = BookTalkListener()
    private let speaker = BookTalkSpeaker()
    private var session: BookTalkSession?
    private var index: BookTalkIndex?
    private var textLength = 0
    private var followMap: FollowMap?
    private var frontier: Int?
    private var runTask: Task<Void, Never>?
    private var pressTask: Task<Void, Never>?
    private var pushActive = false
    private var awaitingReadOffer = false
    private var lastInput = BookTalkInput.text
    private var opened = false
    private var rebuildGeneration = 0
    /// Abbruch und Zurücksetzen des Kern-Verlaufs nach `stopAnswer`; eine neue Frage wartet darauf.
    private var settle: Task<Void, Never>?

    init(book: LibraryBook, library: PhoneLibrary = .shared, playback: LibraryPlayback = .shared) {
        self.book = book
        self.library = library
        self.playback = playback
        speaksAloud = Self.storedSpeaksAloud()
        wire()
    }

    // MARK: Öffnen und Schließen

    /// Blatt geht auf: Vorlesen pausiert, Text und Index entstehen abseits des Hauptthreads.
    func open() async {
        guard !opened else { return }
        opened = true
        if playback.busy { playback.pause() }
        library.flush()
        playback.remoteIntercept = { [weak self] command in
            guard let self else { return false }
            // Läuft etwas, hält jede Fernbedienung es an. Sonst darf „Play“ nicht unter dem Blatt vorlesen.
            if self.phase != .idle { self.stopAnswer(); return true }
            return command != .pause
        }
        speaker.prepare()
        let url = library.textURL(book)
        let title = book.title
        let built: (BookTalkIndex, Int)? = await Task.detached(priority: .userInitiated) {
            guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
            return (BookTalkIndex(title: title, text: text), text.utf16.count)
        }.value
        guard opened else { return }
        if let built {
            index = built.0
            textLength = built.1
            if let audio = book.audio, let total = audio.totalDurationMs, total > 0, !audio.sync.isEmpty {
                followMap = FollowMap(sync: audio.sync, textLength: Double(built.1), totalMs: total)
            }
        } else {
            notice = "Der Text dieses Buchs ist nicht auf dem iPhone."
        }
        let turns = store.load(bookId: book.id)
        conversation = BookTalkConversation(history: turns)
        await rebuild(history: turns)
        guard opened else { return }
        updateFrontier()
        loading = false
        // VoiceOver-Nutzer hören die Antwort selbst: Anton nur auf Wunsch (Menü), nie ungefragt.
    }

    /// Blatt ist zu: alles still, Modell frei, Wiedergabe-Haken zurück.
    func teardown() {
        stopAnswer(announce: false)
        pressCancel()
        listener.cancel()
        playback.remoteIntercept = nil
        engine.llama?.release()
        opened = false
        rebuildGeneration += 1
    }

    // MARK: Modell und Schalter

    func setEnabled(_ value: Bool) {
        UserDefaults.standard.set(value, forKey: BookTalkEngine.enabledKey)
        enabled = value
        Task { await rebuild(history: conversation.storedTurns) }
    }

    func setChoice(_ value: BookTalkModelChoice) {
        UserDefaults.standard.set(value.rawValue, forKey: BookTalkModelChoice.key)
        choice = value
        Task { await rebuild(history: conversation.storedTurns) }
    }

    func setSpeaksAloud(_ value: Bool) {
        UserDefaults.standard.set(value, forKey: Self.antonKey)
        speaksAloud = value
        if !value { speaker.stop(); if phase == .speaking { phase = .idle } }
    }

    /// Nur der jüngste Aufruf gilt: ältere (Schalter schnell umgelegt) und solche nach dem Schließen geben ihr Modell frei.
    private func rebuild(history: [BookTalkStoredTurn]) async {
        rebuildGeneration += 1
        let generation = rebuildGeneration
        stopAnswer(announce: false)
        engine.llama?.release()
        let choice = self.choice, enabled = self.enabled
        let made = await Task.detached(priority: .userInitiated) { BookTalkEngine.make(choice: choice, enabled: enabled) }.value
        guard generation == rebuildGeneration, opened else { made.llama?.release(); return }
        engine = made
        guard let index else { session = nil; return }
        session = BookTalkSession(index: index, model: made.model, budget: made.budget, tokenCounter: made.counter, history: history)
    }

    // MARK: Bereich und Grenze

    /// Weitester Stand aus Lesen und Hören; steigt nur (Plan §3.0).
    private func updateFrontier() {
        library.flush()
        let text = library.record(library.textKey(book))
        let audio = library.record(library.audioKey(book))
        let found = LibraryProgress.frontier(text: text, audio: audio, map: followMap)
        if let found { frontier = max(frontier ?? 0, found) }
        page = frontier.flatMap { index?.page(at: max(0, $0 - 1)) }
    }

    var hasPosition: Bool { frontier != nil }

    /// Menü „Bis S. n“: `until_position` sofort, „Ganzes Buch“ erst nach Bestätigung.
    func chooseScope(_ value: BookTalkScope) {
        if value == .book, scope != .book { confirmingBook = true } else { applyScope(value) }
    }

    func confirmBookScope() { confirmingBook = false; applyScope(.book) }

    private func applyScope(_ value: BookTalkScope) {
        guard value != scope else { return }
        let hadActive = conversation.active != nil
        if hadActive || phase != .idle { stopAnswer(reason: .scopeChanged, announce: false) }
        scope = value
        awaitingScopeConfirm = false
        let text = value == .book ? "Ganzes Buch, auch spätere Kapitel. Gilt, bis das Blatt schließt." : "Nur bis zu deiner Leseposition."
        notice = hadActive ? "Bereich gewechselt, Antwort abgebrochen." : text
        announce(hadActive ? "Bereich gewechselt. Antwort abgebrochen." : text)
    }

    var scopeTitle: String {
        if scope == .book { return "Ganzes Buch" }
        if let page { return "Bis S. \(page)" }
        return "Bis Leseposition"
    }

    // MARK: Fragen

    /// Text aus Eingabefeld oder Spracherkennung.
    func submit(_ raw: String, input: BookTalkInput) {
        let question = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !question.isEmpty else { return }
        lastInput = input
        if awaitingScopeConfirm {
            awaitingScopeConfirm = false
            if BookTalkCommands.detect(question) == .confirm { applyScope(.book); return }
        }
        if let command = BookTalkCommands.effective(question, awaitingReadOffer: awaitingReadOffer) {
            perform(command)
            return
        }
        ask(question, input: input)
    }

    func sendDraft() {
        let text = draft
        draft = ""
        typing = false
        submit(text, input: .text)
    }

    /// Abgebrochene Frage erneut stellen (neuer Bereich, nach dem Hintergrund).
    func reask(_ entry: BookTalkEntry) { ask(entry.question, input: entry.input) }

    private func ask(_ question: String, input: BookTalkInput) {
        guard let session else { notice = "Das Gespräch ist noch nicht bereit."; return }
        stopAnswer(announce: false)
        notice = nil
        awaitingReadOffer = false
        selection = nil
        updateFrontier()
        let request = BookTalkRequest(bookId: book.id, requestId: UUID().uuidString, question: question, frontier: frontier, scope: scope, input: input)
        conversation.begin(request)
        phase = .thinking
        speaker.begin(language: BookTalkIntents.language(of: question))
        let waiting = settle
        runTask = Task { [weak self] in
            await waiting?.value
            guard !Task.isCancelled else { return }
            for await event in session.events(request) {
                guard let self else { return }
                await self.receive(event, session: session)
            }
        }
    }

    private func receive(_ event: BookTalkEvent, session: BookTalkSession) async {
        // Ereignisse einer alten `request_id` (Bereichswechsel, Abbruch) verwirft die Unterhaltung.
        guard conversation.apply(event) else { return }
        switch event {
        case .sentence(_, let sentence):
            if speaksAloud { speaker.enqueue(sentence.speech) }
        case .done(_, let answer):
            awaitingReadOffer = answer.offersRead
            // Gespeichert wird nur, was die Anzeige angenommen hat: bisheriger Stand plus dieser Wechsel.
            var turns = conversation.storedTurns
            if let last = await session.history.last, last.question == entry(for: event)?.question { turns.append(last) }
            conversation.sync(stored: turns)
            await session.restore(history: turns)
            try? store.save(bookId: book.id, turns: turns)
            announce(answer.citations.isEmpty ? "Antwort fertig." : "Antwort fertig, \(answer.citations.count) Belege.")
            if speaksAloud {
                speaker.enqueue(answer.speechTail)
                speaker.endOfInput()
                if phase == .thinking {
                    phase = speaker.isActive ? .speaking : .idle
                    if phase == .idle { continueIfOffered() }
                }
            } else {
                phase = .idle
                continueIfOffered()
            }
        case .failed(_, let code):
            phase = .idle
            speaker.stop()
            notice = Self.message(for: code)
        }
    }

    private func entry(for event: BookTalkEvent) -> BookTalkEntry? {
        guard case .done(let id, _) = event else { return nil }
        return conversation.entries.last { $0.id == id }
    }

    private static func message(for code: BookTalkCode) -> String? {
        switch code {
        case .cancelled: nil
        case .offline: "Keine Verbindung."
        case .timeout: "Die Antwort hat zu lange gedauert."
        case .noModel: "Es ist kein Modell eingerichtet."
        default: "Die Antwort ist fehlgeschlagen."
        }
    }

    /// Antwort sofort beenden (Anhalten, Barge-in, Bereichswechsel, Hintergrund). Gespeichert wird nichts.
    func stopAnswer(reason: BookTalkAbort = .user, announce shouldAnnounce: Bool = true) {
        if let id = conversation.activeRequestId, let session {
            // Ein Wechsel, den die Anzeige verwirft, darf auch im Kern nicht bleiben (Prompt, Platte).
            let stored = conversation.storedTurns
            let before = settle
            settle = Task {
                await before?.value
                await session.cancel(requestId: id)
                await session.restore(history: stored)
            }
        }
        runTask?.cancel()
        runTask = nil
        conversation.abort(reason)
        speaker.stop()
        if phase == .thinking || phase == .speaking { phase = .idle }
        if shouldAnnounce { announce("Antwort angehalten.") }
    }

    // MARK: Befehle

    private func perform(_ command: BookTalkCommand) {
        switch command {
        case .jump:
            if let citation = lastCitation { go(to: citation, autoplay: false) } else { notice = "Es gibt noch keine Fundstelle." }
        case .readCitation, .confirm:
            if let citation = lastCitation { go(to: citation, autoplay: true) } else { notice = "Es gibt noch keine Fundstelle." }
        case .resume:
            resumeReading()
        case .scopeBook:
            askScopeByVoice()
        case .scopePosition:
            applyScope(.untilPosition)
        case .stop:
            stopAnswer()
        case .end:
            close?()
        }
    }

    private var lastCitation: BookTalkCitation? {
        conversation.entries.last(where: { $0.status == .done && !$0.citations.isEmpty })?.citations.first
    }

    private func askScopeByVoice() {
        guard scope != .book else { return }
        awaitingScopeConfirm = true
        let text = "Auch spätere Kapitel einbeziehen? Sag Ja."
        notice = text
        announce(text)
        speaker.begin(language: .de)
        if speaksAloud { speaker.enqueue(text); speaker.endOfInput() } else { continueIfOffered() }
    }

    func select(_ citation: BookTalkCitation, in entry: BookTalkEntry) {
        let value = Selection(entryId: entry.id, citation: citation)
        selection = selection == value ? nil : value
        if selection != nil {
            let page = citation.page.map { "Seite \($0)" } ?? citation.chapterTitle
            announce("Beleg \(page): \(citation.quote)")
        }
    }

    func clearSelection() { selection = nil }

    /// Belegkarte: im Buch an die Stelle gehen (zählt nicht als gelesen).
    func go(to citation: BookTalkCitation, autoplay: Bool) {
        stopAnswer(announce: false)
        if autoplay {
            Task {
                await listener.cancelAndWait()
                playback.jump(toOffset: citation.start, autoplay: true)
                close?()
            }
        } else {
            playback.jump(toOffset: citation.start, autoplay: false)
            let place = citation.page.map { "Seite \($0)" } ?? citation.chapterTitle
            notice = "Im Buch bei \(place). Das Vorlesen ist pausiert."
            announce(notice ?? "")
        }
    }

    /// „Weiterlesen“: Gespräch stumm, Vorlesen ab der gemerkten Stelle, Blatt zu.
    func resumeReading() {
        stopAnswer(announce: false)
        pressCancel()
        phase = .idle
        Task {
            // Erst wenn das Mikrofon und die Sitzung frei sind, darf das Vorlesen die Sitzung übernehmen.
            await listener.cancelAndWait()
            playback.play()
            close?()
        }
    }

    // MARK: Mikrofon

    var micLabel: String {
        switch phase {
        case .listening: "Zuhören beenden"
        case .thinking, .speaking: "Antwort anhalten und sprechen"
        case .idle: "Frage sprechen"
        }
    }

    /// Fingerdruck beginnt: nach kurzer Zeit Push-to-talk, sonst Tipp.
    func pressDown() {
        guard pressTask == nil else { return }
        pushActive = false
        pressTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled, let self else { return }
            self.pushActive = true
            self.startListening(.pushToTalk)
        }
    }

    /// Geste abgebrochen (Hintergrund, Blatt zu): nichts senden, Aufnahme verwerfen.
    func pressCancel() {
        pressTask?.cancel()
        pressTask = nil
        if pushActive { pushActive = false; listener.cancel(); phase = .idle }
    }

    func pressUp() {
        // Ohne laufenden Druck (abgebrochen durch `pressCancel`) ist das Loslassen nur das Echo der Geste.
        let pressed = pressTask != nil
        pressTask?.cancel()
        pressTask = nil
        if pushActive {
            pushActive = false
            listener.finish()
        } else if pressed {
            tap()
        }
    }

    /// Tipp: Freihand bis 800 ms Stille; beim Zuhören beendet er die Aufnahme, bei Antwort ist es der Barge-in.
    func tap() {
        switch phase {
        case .listening: listener.finish()
        case .idle, .thinking, .speaking: startListening(.handsFree)
        }
    }

    private func startListening(_ mode: BookTalkListener.Mode) {
        guard !listener.isBusy else { return }
        if phase != .idle { stopAnswer(announce: false) }
        speaker.stop()
        notice = nil
        stableText = ""; volatileText = ""
        phase = .listening
        announce("Ich höre zu.")
        let code = Locale.current.language.languageCode?.identifier
        Task { [weak self] in
            guard let self else { return }
            do { try await listener.start(mode: mode, languageCode: code) } catch is CancellationError {
                if phase == .listening { phase = .idle }
            } catch {
                // Erster Start mit Berechtigungsdialog: die Taste ist beim Schließen des Dialogs los, noch einmal tippen.
                phase = .idle
                notice = error.localizedDescription
            }
        }
    }

    /// Nach einem Angebot („Soll ich die Stelle vorlesen?“, „Sag Ja.“) bei Spracheingabe gleich weiter zuhören.
    private func continueIfOffered() {
        guard lastInput == .voice, awaitingReadOffer || awaitingScopeConfirm, phase == .idle, opened else { return }
        startListening(.handsFree)
    }

    private func wire() {
        listener.onText = { [weak self] stable, volatile in
            self?.stableText = stable
            self?.volatileText = volatile
        }
        listener.onFinished = { [weak self] outcome in
            guard let self else { return }
            stableText = ""; volatileText = ""
            guard phase == .listening else { return }
            phase = .idle
            switch outcome {
            case .text(let text):
                submit(text, input: .voice)
            case .noSpeech:
                notice = "Ich habe nichts gehört. Tippe auf das Mikrofon und frag noch einmal."
            case .interrupted(let why, let partial):
                notice = why
                if !partial.isEmpty { draft = partial; typing = true }
            }
        }
        speaker.onAudible = { [weak self] in
            guard let self, phase == .thinking else { return }
            phase = .speaking
        }
        speaker.onIdle = { [weak self] in
            guard let self else { return }
            if phase == .speaking { phase = .idle }
            continueIfOffered()
        }
    }

    // MARK: Vordergrundregel

    /// `scenePhase != .active`: Mikrofon und Antwort aus, Stand sichern. Weiter nur durch den Nutzer.
    func sceneInactive(background: Bool = false) {
        guard opened else { return }
        pressCancel()
        listener.cancel()
        stableText = ""; volatileText = ""
        let hadAnswer = conversation.active != nil
        stopAnswer(reason: .background, announce: false)
        phase = .idle
        awaitingScopeConfirm = false
        if hadAnswer { notice = "Das Gespräch wurde angehalten. Tippe auf Weiter fragen." }
        // Modell erst im Hintergrund freigeben: ein kurzes Inaktiv (Kontrollzentrum, Anruf-Banner) soll es nicht kosten.
        if background { engine.llama?.release() }
        library.flush()
    }

    // MARK: Verlauf

    func clearHistory() {
        stopAnswer(announce: false)
        conversation.clear()
        selection = nil
        store.clear(bookId: book.id)
        let session = session
        Task { await session?.clearHistory() }
        notice = "Verlauf gelöscht."
        announce("Verlauf gelöscht.")
    }

    private func announce(_ text: String) {
        guard !text.isEmpty else { return }
        AccessibilityNotification.Announcement(text).post()
    }
}
