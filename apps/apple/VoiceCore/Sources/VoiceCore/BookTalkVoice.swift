import Foundation

// Buch-Gespräch (Epic #307, Paket G5): reine Logik hinter Hören und Verlauf, ohne Audio- und UI-Frameworks.
// Der Kern (`BookTalkSession`) bleibt unverändert; hier steht, was die Oberfläche zusätzlich braucht.

/// Satzende nach Stille („Freihand“): Pegel kommen als Effektivwert (0…1) je Puffer.
/// Rauschpegel wird laufend geschätzt; Sprache ist deutlich darüber. Endet nach `silenceMs` Stille,
/// wenn vorher mindestens `minSpeechMs` Sprache kam.
public struct BookTalkEndpointer: Sendable {
    public enum Event: Sendable, Equatable {
        case none
        case speechStarted
        /// Frage zu Ende (Stille nach Sprache oder Höchstdauer).
        case ended
        /// Es kam nie Sprache innerhalb von `noSpeechMs`.
        case noSpeech
    }

    public let silenceMs: Int
    public let minSpeechMs: Int
    public let noSpeechMs: Int
    public let maxMs: Int
    /// Untergrenze für „Sprache“, damit ein stilles Zimmer keine Schwelle nahe 0 bekommt.
    public let minThreshold: Float

    private var floor: Float = 0.004
    private var elapsed = 0
    private var voiced = 0
    private var silent = 0
    private var started = false
    private var done = false

    public init(silenceMs: Int = 800, minSpeechMs: Int = 200, noSpeechMs: Int = 8_000, maxMs: Int = 45_000,
                minThreshold: Float = 0.012) {
        self.silenceMs = silenceMs
        self.minSpeechMs = minSpeechMs
        self.noSpeechMs = noSpeechMs
        self.maxMs = maxMs
        self.minThreshold = minThreshold
    }

    public var speechStarted: Bool { started }
    public var threshold: Float { max(minThreshold, floor * 3) }

    public mutating func feed(rms: Float, ms: Int) -> Event {
        guard !done, ms > 0 else { return .none }
        elapsed += ms
        let level = rms.isFinite ? max(0, rms) : 0
        let loud = level > threshold
        // Rauschpegel: nur in Stille nachführen (langsam hoch, schnell runter).
        if !loud { floor = level < floor ? level : floor + (level - floor) * 0.02 }
        var event = Event.none
        if loud {
            voiced += ms
            silent = 0
            if !started, voiced >= minSpeechMs { started = true; event = .speechStarted }
        } else {
            silent += ms
            if !started { voiced = 0 }
        }
        if started, silent >= silenceMs { done = true; return .ended }
        if elapsed >= maxMs { done = true; return started ? .ended : .noSpeech }
        if !started, elapsed >= noSpeechMs { done = true; return .noSpeech }
        return event
    }
}

public enum BookTalkAbort: Sendable, Equatable {
    /// Bereich gewechselt (Plan §3.7): „abgebrochen, Bereich gewechselt“.
    case scopeChanged
    /// App im Hintergrund oder gesperrt (Plan §8.3).
    case background
    /// Nutzer hat die Antwort angehalten.
    case user
    /// Modell- oder Systemfehler ohne Antwort.
    case failed

    public var note: String {
        switch self {
        case .scopeChanged: "abgebrochen, Bereich gewechselt"
        case .background: "abgebrochen, App im Hintergrund"
        case .user: "angehalten"
        case .failed: "abgebrochen"
        }
    }
}

public struct BookTalkEntry: Identifiable, Sendable, Equatable {
    public enum Status: Sendable, Equatable {
        case answering
        case done
        case aborted(BookTalkAbort)
    }

    public let id: String
    public let question: String
    public let input: BookTalkInput
    public let scope: BookTalkScope
    public let frontier: Int
    public var sentences: [BookTalkAnswerSentence]
    public var answer: BookTalkAnswer?
    public var status: Status

    public var isActive: Bool { status == .answering }
    public var isAborted: Bool { if case .aborted = status { true } else { false } }
    /// Alle Belege der angezeigten Sätze.
    public var citations: [BookTalkCitation] { answer?.citations ?? sentences.flatMap(\.citations) }
    public var text: String { answer?.text ?? sentences.map(\.text).joined(separator: " ") }
}

/// Verlauf für die Anzeige: gespeicherte Wechsel plus die laufende Frage. Ereignisse einer anderen
/// `request_id` als der aktiven werden verworfen; abgebrochene Antworten bleiben sichtbar, aber nur die
/// abgeschlossenen (`done`) Wechsel gehören in den gespeicherten Verlauf (`storedTurns`).
public struct BookTalkConversation: Sendable, Equatable {
    public private(set) var entries: [BookTalkEntry]
    private var stored: [BookTalkStoredTurn]

    public init(history: [BookTalkStoredTurn] = []) {
        stored = history
        entries = history.enumerated().map { Self.entry($0.element, id: "h\($0.offset)") }
    }

    private static func entry(_ turn: BookTalkStoredTurn, id: String) -> BookTalkEntry {
        BookTalkEntry(id: id, question: turn.question, input: turn.input, scope: turn.scope, frontier: turn.frontier,
                      sentences: turn.answer.sentences, answer: turn.answer, status: .done)
    }

    public var active: BookTalkEntry? { entries.last(where: \.isActive) }
    public var activeRequestId: String? { active?.id }
    public var storedTurns: [BookTalkStoredTurn] { stored }
    public var isEmpty: Bool { entries.isEmpty }

    public mutating func begin(_ request: BookTalkRequest) {
        abort(.failed)
        entries.append(BookTalkEntry(id: request.requestId, question: request.question, input: request.input, scope: request.scope,
                                     frontier: request.frontier ?? 0, sentences: [], answer: nil, status: .answering))
    }

    /// Übernimmt ein Ereignis. `false`: verworfen (alte oder unbekannte `request_id`, schon abgebrochen).
    @discardableResult
    public mutating func apply(_ event: BookTalkEvent) -> Bool {
        let rid: String
        switch event {
        case .sentence(let id, _), .done(let id, _), .failed(let id, _): rid = id
        }
        guard let i = entries.lastIndex(where: { $0.id == rid }), entries[i].isActive else { return false }
        switch event {
        case .sentence(_, let s):
            entries[i].sentences.append(s)
        case .done(_, let answer):
            entries[i].answer = answer
            entries[i].sentences = answer.sentences
            entries[i].status = .done
        case .failed:
            entries[i].status = .aborted(.user)
        }
        return true
    }

    /// Die laufende Frage abbrechen: Sätze bleiben sichtbar, nichts wird gespeichert.
    public mutating func abort(_ reason: BookTalkAbort) {
        guard let i = entries.lastIndex(where: \.isActive) else { return }
        entries[i].status = .aborted(reason)
    }

    /// Nach `done`: Wechsel in den gespeicherten Verlauf übernehmen (zusammen mit dem Kern, der ihn führt).
    public mutating func sync(stored turns: [BookTalkStoredTurn]) { stored = turns }

    public mutating func clear() {
        entries.removeAll()
        stored.removeAll()
    }
}
