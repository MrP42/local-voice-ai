import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Widerruf mit Generation.
// Die Freigabe-Generation ist monoton und steigt bei JEDER Änderung von Opt-in (an oder aus), Bereichen und Cloud-Zustimmung.
// Dabei werden alle laufenden Antworten abgebrochen (Ausgang „widerrufen“). Jedes Ereignis trägt die Generation seines Laufs;
// unmittelbar vor dem Senden prüft der Kern sie im selben Lock, in dem sie steigt: Nach der Rückkehr von `bump` kann kein
// Ereignis einer älteren Generation mehr herauskommen. Aus und wieder Ein macht alte Ergebnisse nicht wieder gültig (die
// Generation wird nie zurückgesetzt). Lock mit kurzer Haltezeit, kein I/O, nur für Steuerung und Anzeige, nie im Audio-Callback.

/// Prüfung, die der Controller vor Anzeige und Audiostart macht (und die Tests ohne Sitzung ausüben): Ein Ereignis gilt nur,
/// wenn seine Generation die aktuelle ist. Ein Ereignis mit alter Generation nach einem neueren wird nie angenommen.
public enum AssistantReleaseGate {
    public static func admits(eventGeneration: Int, current: Int) -> Bool { eventGeneration == current }
}

final class AssistantRelease: @unchecked Sendable {
    struct Snapshot: Sendable, Equatable {
        let generation: Int
        let optIn: Bool
    }

    private let lock = NSLock()
    private var generation = 0
    private var optIn: Bool
    private var runs: [String: @Sendable () -> Void] = [:]

    init(optIn: Bool) { self.optIn = optIn }

    /// Generation und Opt-in zusammen, aus einem Lock: Die Rundenart einer Anfrage und ihre Generation passen zusammen.
    var snapshot: Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return Snapshot(generation: generation, optIn: optIn)
    }

    var currentGeneration: Int { snapshot.generation }
    var isOptedIn: Bool { snapshot.optIn }

    /// Setzt den Opt-in. Nur eine echte Änderung zählt (Generation steigt, laufende Antworten werden widerrufen). Gibt die
    /// Generation danach zurück.
    @discardableResult
    func setOptIn(_ on: Bool) -> Int {
        lock.lock()
        guard optIn != on else {
            let g = generation
            lock.unlock()
            return g
        }
        optIn = on
        return finishBump()
    }

    /// Bereiche oder Cloud-Zustimmung haben sich geändert (oder jede andere Änderung der Freigabe).
    @discardableResult
    func bump() -> Int {
        lock.lock()
        return finishBump()
    }

    /// Mit gehaltenem Lock aufrufen; gibt ihn frei. Die Widerrufe laufen außerhalb des Locks.
    private func finishBump() -> Int {
        generation += 1
        let g = generation
        let revoke = Array(runs.values)
        runs.removeAll()
        lock.unlock()
        for r in revoke { r() }
        return g
    }

    /// Meldet einen Lauf an. `false`, wenn die Generation schon veraltet ist (dann wurde `revoke` nicht aufgenommen; der
    /// Aufrufer widerruft selbst).
    func register(_ id: String, generation: Int, revoke: @escaping @Sendable () -> Void) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard self.generation == generation else { return false }
        runs[id] = revoke
        return true
    }

    func unregister(_ id: String) {
        lock.lock()
        runs[id] = nil
        lock.unlock()
    }

    /// Führt `body` nur aus, wenn `generation` die aktuelle ist, und zwar im selben Lock wie die Prüfung: Eine Änderung
    /// dazwischen ist ausgeschlossen. `body` muss kurz sein (Ereignis in den Strom legen).
    func ifCurrent(_ generation: Int, _ body: () -> Void) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard self.generation == generation else { return false }
        body()
        return true
    }
}

/// Rückmeldungen des Controllers (Anzeige, Audiostart) je Anfrage, bis `commit` sie in die Satzspur übernimmt.
final class AssistantOutputLedger: @unchecked Sendable {
    struct Note: Sendable {
        let index: Int
        let mark: AssistantCommitMark
    }

    static let maxRequests = 32
    static let maxNotes = 512

    private let lock = NSLock()
    private var notes: [String: [Note]] = [:]
    private var order: [String] = []

    func add(_ id: String, _ note: Note) {
        lock.lock()
        defer { lock.unlock() }
        if notes[id] == nil {
            order.append(id)
            while order.count > Self.maxRequests { notes[order.removeFirst()] = nil }
        }
        guard notes[id, default: []].count < Self.maxNotes else { return }
        notes[id, default: []].append(note)
    }

    func take(_ id: String) -> [Note] {
        lock.lock()
        defer { lock.unlock() }
        order.removeAll { $0 == id }
        return notes.removeValue(forKey: id) ?? []
    }
}
