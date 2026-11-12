import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Datenkontext mit Sperre. Ersetzt die frühere Rundenwahl nach Wortlisten.
// Grundsatz: Wortlisten und fehlende Treffer dürfen freien Text nie ERLAUBEN, nur verbieten.
//
// Zustand `.data` oder `.free`:
// - Anfang: `.free` nur, wenn der Opt-in an ist UND (der Verlauf leer ist ODER der letzte gespeicherte Beitrag eine Runde ohne
//   Quelle war); sonst `.data`. Opt-in aus: immer `.data`.
// - `.free`: Die Frage wird geprüft. Eigendatenbezug (`AssistantOwnData`) oder ein relevanter Abruftreffer macht diese Runde zur
//   Datenrunde UND rastet den Zustand auf `.data` ein. Sonst Runde ohne Quelle.
// - `.data`: jede Runde ist eine Datenrunde, ohne Ausnahme (keine Anaphern-, Wortzahl- oder „allgemein“-Regel).
// - Zurück nach `.free` nur durch die ausdrückliche Handlung `resumeFree` (Knopf „Ohne Quelle fortsetzen“). Einschalten des
//   Opt-ins setzt `.free` nur, wenn das Gespräch noch keine Datenrunde hatte. „Neues Gespräch“ beginnt nach der Anfangsregel.
// Der Prompt einer Runde ohne Quelle sieht nur Beiträge aus Runden ohne Quelle seit dem letzten `resumeFree` bzw. seit der
// letzten Datenrunde (`freeHistory`).
//
// Die Relevanz eines Abruftreffers (`isRelevant`) entscheidet nur über das Einrasten auf `.data`; ein Fehlen von Treffern
// erlaubt nichts: Frei bleibt der Zustand allein, wenn weder Eigendatenbezug noch Treffer vorliegen UND er schon `.free` war.
// Reiner Typ: ein Lock mit kurzer Haltezeit, kein I/O.

public final class AssistantContext: @unchecked Sendable {
    public enum Mode: String, Sendable, Codable, Equatable {
        case data, free
    }

    public enum Reason: String, Sendable, Codable, Equatable {
        /// Opt-in aus (oder nicht freigegeben): jede Runde ist eine Datenrunde.
        case optedOut
        /// Zustand `.data`: gesperrt.
        case locked
        /// Bestandsfrage.
        case inventory
        case ownData
        case relevantPassage
        /// Zustand `.free` und nichts spricht für Daten.
        case free
    }

    public struct Decision: Sendable, Equatable {
        public let kind: AssistantRoundKind
        public let reason: Reason
        /// Zustand vor der Entscheidung.
        public let before: Mode
    }

    /// Mindestzahl verschiedener Inhaltswörter der Frage, die ein Auszug decken muss (kleiner bei weniger Wörtern).
    static let relevantWords = 2

    private let lock = NSLock()
    private var current: Mode
    private var dataRound: Bool
    private var freeSince: Date?

    /// Anfangsregel; `optIn` ist schon mit der Freischaltung der Funktion verrechnet (`AssistantUnsourcedConsent.isReleased`).
    public init(optIn: Bool, history: [AssistantTurn]) {
        current = Self.initial(optIn: optIn, history: history)
        dataRound = history.contains { ($0.round ?? .data) == .data }
        freeSince = nil
    }

    static func initial(optIn: Bool, history: [AssistantTurn]) -> Mode {
        guard optIn else { return .data }
        guard let last = history.last else { return .free }
        return (last.round ?? .data) == .general ? .free : .data
    }

    public var mode: Mode { lock.lock(); defer { lock.unlock() }; return current }

    // MARK: Entscheidung

    /// Rundenart der Frage. Rastet `.free` auf `.data` ein, wenn die Runde eine Datenrunde ist.
    public func decide(question: String, titles: [String], passages: [AssistantPassage], optIn: Bool, inventory: Bool) -> Decision {
        // Die Prüfung der Frage liegt außerhalb des Locks (sie ist rein); nur der Zustand wird gelesen und gesetzt.
        let before = mode
        guard optIn else { return settle(.data, .optedOut, before: before, latch: true) }
        if before == .data { return settle(.data, .locked, before: before, latch: false) }
        if inventory { return settle(.data, .inventory, before: before, latch: true) }
        if AssistantOwnData.refers(question, mode: .question, titles: titles) {
            return settle(.data, .ownData, before: before, latch: true)
        }
        if passages.contains(where: { Self.isRelevant($0, to: question) }) {
            return settle(.data, .relevantPassage, before: before, latch: true)
        }
        return settle(.general, .free, before: before, latch: false)
    }

    func settle(_ kind: AssistantRoundKind, _ reason: Reason, before: Mode, latch: Bool) -> Decision {
        lock.lock()
        defer { lock.unlock() }
        if kind == .data { dataRound = true }
        if latch { current = .data }
        // Ein Lauf, der `.free` sah, während ein anderer schon auf `.data` eingerastet ist: strenger, der Zustand gilt jetzt.
        if kind == .general, current == .data { return Decision(kind: .data, reason: .locked, before: before) }
        return Decision(kind: kind, reason: reason, before: before)
    }

    // MARK: Zustandswechsel

    /// Der Opt-in hat sich geändert. Aus: `.data`. An: `.free` nur, wenn das Gespräch noch keine Datenrunde hatte.
    public func optInChanged(_ on: Bool) {
        lock.lock()
        current = on && !dataRound ? .free : .data
        lock.unlock()
    }

    /// Ausdrückliche Nutzerhandlung „Ohne Quelle fortsetzen“. `false`, wenn nichts zu tun ist (Opt-in aus oder schon `.free`).
    @discardableResult
    public func resumeFree(optIn: Bool, now: Date = Date()) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard optIn, current == .data else { return false }
        current = .free
        freeSince = now
        return true
    }

    /// Neues Gespräch, Verlauf geladen oder gelöscht: Anfangsregel.
    public func reset(optIn: Bool, history: [AssistantTurn]) {
        lock.lock()
        current = Self.initial(optIn: optIn, history: history)
        dataRound = history.contains { ($0.round ?? .data) == .data }
        freeSince = nil
        lock.unlock()
    }

    // MARK: Verlauf für die Runde ohne Quelle

    /// Beiträge, die ein Prompt ohne Quelle sehen darf: nur Runden ohne Quelle, die nach der letzten Datenrunde und nach dem
    /// letzten `resumeFree` entstanden sind.
    public func freeHistory(_ turns: [AssistantTurn]) -> [AssistantTurn] {
        lock.lock()
        let since = freeSince
        lock.unlock()
        return Self.freeHistory(turns, since: since)
    }

    static func freeHistory(_ turns: [AssistantTurn], since: Date?) -> [AssistantTurn] {
        var tail: [AssistantTurn] = []
        for t in turns.reversed() {
            guard (t.round ?? .data) == .general else { break }
            if let since, t.at <= since { break }
            tail.append(t)
        }
        return tail.reversed()
    }

    // MARK: Relevanz

    /// Ein Auszug ist relevant, wenn Titel und Text mindestens zwei verschiedene Inhaltswörter der Frage decken (bei einer Frage
    /// mit nur einem Inhaltswort: dieses), gedeckt wie in der Belegprüfung. Eine Frage ganz ohne Inhaltswort kann nichts
    /// ausschließen: dann gilt jeder Auszug als relevant.
    public static func isRelevant(_ passage: AssistantPassage, to question: String) -> Bool {
        let wanted = Set(BookTalkTokens.contentWords(question))
        guard !wanted.isEmpty else { return true }
        let words = BookTalkTokens.foldedWords(passage.title + " " + passage.text)
        let hits = wanted.filter { BookTalkCitations.covered($0, by: words) }.count
        return hits >= min(relevantWords, wanted.count)
    }
}
