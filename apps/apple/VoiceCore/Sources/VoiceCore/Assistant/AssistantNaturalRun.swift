import Foundation

// Stil `natural` (ADR-0028 E4.2c): Bausteine des Laufs einer Anfrage, die keine Sitzung brauchen.
// - `AssistantCommitNotes`: der Hinweis „Äußerung gilt“ je Anfrage (`AssistantSession.noteCommitted`), mit dem Zeitpunkt.
// - `AssistantNaturalMailbox`: der EINE Ereignisstrom einer Anfrage. Modellstücke (je Stufe) und Wecker der Ansagen laufen in
//   ihn hinein; ein einziger Verbraucher (der Lauf) verarbeitet sie der Reihe nach. So gibt es nie zwei gleichzeitige Ausgaben
//   (`RunEmitter.emit` läuft nur im Verbraucher), und die Reihenfolge der Spur ist die Reihenfolge der Verarbeitung.
// - `AssistantNaturalFillers`: die zwei Zeitgeber der Ansagen für eine langsame Antwort (je einmal je Anfrage). Sie setzen nur eine
//   Marke „fällig“ und wecken den Verbraucher; ob die Ansage noch gilt, entscheidet er im Augenblick der Verarbeitung.
// Alles hier ist Zustand mit kurzem Lock ohne Wartezeit und ohne I/O (Blatt-Locks, nie unter einem anderen Lock gerufen).

/// Wann die Steuerung gemeldet hat, dass die Äußerung einer Anfrage zählt. Eine Meldung gilt je Kennung einmal (die erste); ohne Meldung
/// gibt es keine Ansagen für eine langsame Antwort. Der Lauf meldet sich mit `attach` an und erfährt den Zeitpunkt GENAU einmal:
/// sofort, wenn die Meldung schon da war, sonst bei ihrem Eintreffen. Anmelden und Eintreffen entscheiden unter EINEM Lock
/// (kein Fenster, in dem die Meldung zwischen beiden verloren ginge).
final class AssistantCommitNotes: @unchecked Sendable {
    typealias Observer = @Sendable (ContinuousClock.Instant) -> Void

    /// Wie viele Meldungen ohne Lauf (frühe oder späte) gemerkt werden; danach fällt die älteste weg.
    static let maxKept = 64

    private let lock = NSLock()
    private var times: [String: ContinuousClock.Instant] = [:]
    private var order: [String] = []
    private var observers: [String: Observer] = [:]

    /// Die Äußerung zählt. Der Beobachter (falls der Lauf schon angemeldet ist) läuft außerhalb des Locks.
    func note(_ requestId: String, at instant: ContinuousClock.Instant = ContinuousClock.now) {
        let observer: Observer? = lock.withLock {
            guard times[requestId] == nil else { return nil }
            times[requestId] = instant
            order.append(requestId)
            while order.count > Self.maxKept { times.removeValue(forKey: order.removeFirst()) }
            return observers.removeValue(forKey: requestId)
        }
        observer?(instant)
    }

    /// Meldet den Zeitpunkt, sobald er feststeht (oder sofort). Höchstens einmal je Anmeldung.
    func attach(_ requestId: String, _ observer: @escaping Observer) {
        let known: ContinuousClock.Instant? = lock.withLock {
            if let t = times[requestId] { return t }
            observers[requestId] = observer
            return nil
        }
        if let known { observer(known) }
    }

    /// Der Lauf ist zu Ende: keine Meldung ruft ihn mehr.
    func detach(_ requestId: String) { lock.withLock { observers[requestId] = nil } }

    /// Die Anfrage ist abgeschlossen: Ihr Zeitpunkt wird nicht mehr gebraucht.
    func forget(_ requestId: String) {
        lock.withLock {
            times[requestId] = nil
            observers[requestId] = nil
            order.removeAll { $0 == requestId }
        }
    }

    /// Nur Tests: Zahl der gemerkten Zeitpunkte und angemeldeten Beobachter.
    var counts: (times: Int, observers: Int) { lock.withLock { (times.count, observers.count) } }
}

/// Der eine Ereignisstrom einer Anfrage im Stil `natural`. Erzeuger: je Stufe (Sprecher, Denker) ein Task, der den Modellstrom liest,
/// und die Zeitgeber der Ansagen (`wake`). Verbraucher: der Lauf, ein einziger Iterator für die ganze Anfrage (ein verworfener Iterator
/// beendet einen `AsyncStream`). Ereignisse einer älteren Stufe (nach einem Abbruch oder einer Wiederholung) erkennt der Verbraucher an
/// der Stufennummer und überspringt sie.
final class AssistantNaturalMailbox: @unchecked Sendable {
    enum Event: Sendable {
        case piece(stage: Int, String)
        case end(stage: Int)
        case failed(stage: Int, any Error)
        /// Ein Zeitgeber ist abgelaufen: Der Verbraucher prüft, was fällig ist.
        case wake
        /// Die Frist einer Stufe ist abgelaufen (`AssistantNaturalMailbox.deadline`): Der Verbraucher beendet die Stufe, wenn sie noch läuft.
        case deadline(stage: Int)
    }

    let stream: AsyncStream<Event>
    private let continuation: AsyncStream<Event>.Continuation
    private let lock = NSLock()
    private var pumps: [Int: Task<Void, Never>] = [:]
    private var timers: [Int: Task<Void, Never>] = [:]
    private var stage = 0
    private var closed = false

    init() {
        let made = AsyncStream<Event>.makeStream()
        stream = made.stream
        continuation = made.continuation
    }

    func wake() { continuation.yield(.wake) }

    /// Startet den Modellstrom einer neuen Stufe und gibt deren Nummer zurück. Nach `close` läuft nichts mehr an.
    func pump(_ model: any BookTalkModel, instructions: String, prompt: String, maxTokens: Int) -> Int {
        let number: Int = lock.withLock {
            stage += 1
            return stage
        }
        let continuation = self.continuation
        let task = Task {
            do {
                for try await piece in model.stream(instructions: instructions, prompt: prompt, maxTokens: maxTokens) {
                    if Task.isCancelled { return }
                    continuation.yield(.piece(stage: number, piece))
                }
                if !Task.isCancelled { continuation.yield(.end(stage: number)) }
            } catch {
                if !Task.isCancelled { continuation.yield(.failed(stage: number, error)) }
            }
        }
        let late: Bool = lock.withLock {
            if closed { return true }
            pumps[number] = task
            return false
        }
        if late { task.cancel() }
        return number
    }

    /// Setzt eine Frist für eine Stufe: Nach `ms` Millisekunden kommt `.deadline(stage:)` in den Strom, außer die Stufe wurde vorher mit
    /// `cancel(stage:)` beendet oder die Anfrage ist zu Ende. Negativ: keine Frist. Der Zeitgeber setzt nur das Ereignis; was daraus
    /// folgt, entscheidet der Verbraucher.
    func deadline(stage number: Int, ms: Int) {
        guard ms >= 0 else { return }
        let continuation = self.continuation
        let task = Task {
            do {
                try await Task.sleep(nanoseconds: UInt64(ms) * 1_000_000)
            } catch {
                return
            }
            continuation.yield(.deadline(stage: number))
        }
        let late: Bool = lock.withLock {
            if closed { return true }
            timers[number] = task
            return false
        }
        if late { task.cancel() }
    }

    /// Bricht den Modellstrom einer Stufe und ihre Frist ab (Suche erkannt, Grenze erreicht, Stufe beendet).
    func cancel(stage number: Int) {
        let tasks: [Task<Void, Never>] = lock.withLock { [pumps.removeValue(forKey: number), timers.removeValue(forKey: number)].compactMap { $0 } }
        for task in tasks { task.cancel() }
    }

    /// Ende der Anfrage: alle Modellströme und Fristen abbrechen, den Strom beenden.
    func close() {
        let tasks: [Task<Void, Never>] = lock.withLock {
            closed = true
            let all = Array(pumps.values) + Array(timers.values)
            pumps.removeAll()
            timers.removeAll()
            return all
        }
        for task in tasks { task.cancel() }
        continuation.finish()
    }
}

/// Die zwei Ansagen für eine langsame Antwort. `start` (genau einmal, mit dem Zeitpunkt „Äußerung gilt“) setzt zwei Zeitgeber:
/// `firstMs` und `secondMs` nach diesem Zeitpunkt. Ein Zeitgeber markiert „fällig“ und weckt den Verbraucher; `takeDue` gibt jede
/// Ansage höchstens einmal heraus (1 und 2, in dieser Reihenfolge). Ob eine fällige Ansage noch gilt (keine Einheit bzw. keine
/// Antworteinheit ausgegeben), entscheidet der Verbraucher beim Verarbeiten. Ein negativer Wert schaltet die Ansage aus.
final class AssistantNaturalFillers: @unchecked Sendable {
    private let lock = NSLock()
    private let delays: [Int]
    private let wake: @Sendable () -> Void
    private var due = [false, false]
    private var taken = [false, false]
    private var tasks: [Task<Void, Never>] = []
    private var started = false
    private var closed = false

    init(firstMs: Int, secondMs: Int, wake: @escaping @Sendable () -> Void) {
        delays = [firstMs, secondMs]
        self.wake = wake
    }

    func start(commit: ContinuousClock.Instant) {
        lock.withLock {
            guard !closed, !started else { return }
            started = true
            for (i, ms) in delays.enumerated() where ms >= 0 {
                tasks.append(Task { [weak self] in
                    do {
                        try await Task.sleep(until: commit.advanced(by: .milliseconds(ms)), clock: .continuous)
                    } catch {
                        return
                    }
                    self?.fire(i)
                })
            }
        }
    }

    private func fire(_ i: Int) {
        let live: Bool = lock.withLock {
            guard !closed else { return false }
            due[i] = true
            return true
        }
        if live { wake() }
    }

    /// Die fälligen, noch nicht herausgegebenen Ansagen (1: „Einen Moment.“, 2: „Das dauert noch einen Augenblick.“).
    func takeDue() -> [Int] {
        lock.withLock {
            var out: [Int] = []
            for i in 0..<2 where due[i] && !taken[i] {
                taken[i] = true
                out.append(i + 1)
            }
            return out
        }
    }

    /// Ende der Anfrage (auch Abbruch und Widerruf): kein Zeitgeber läuft weiter.
    func shutdown() {
        let all: [Task<Void, Never>] = lock.withLock {
            closed = true
            let t = tasks
            tasks.removeAll()
            return t
        }
        for task in all { task.cancel() }
    }
}
