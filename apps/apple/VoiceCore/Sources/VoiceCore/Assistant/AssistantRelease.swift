import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Widerruf mit Generation.
// Die Freigabe-Generation ist monoton und steigt bei JEDER Änderung von Opt-in (an oder aus), Bereichen, Cloud-Zustimmung und
// beim Zurückschalten in den freien Zustand (`resumeFree`). Dabei werden alle angemeldeten Läufe abgebrochen (Ausgang
// „widerrufen“), und ein Beobachter (der Controller) wird bei JEDER Erhöhung gerufen, unabhängig von Quellen und davon, ob
// das Modell schon fertig ist: Er stoppt die Wiedergabe und leert die Warteschlange. Ein Lauf bleibt bis zum Abschluss seiner
// Ausgabe (Commit) angemeldet, nicht nur bis zum Modellende. Jedes Ereignis trägt die Generation seines Laufs; unmittelbar vor
// dem Senden prüft der Kern sie im selben Lock, in dem sie steigt: Nach der Rückkehr von `bump` kann kein Ereignis einer
// älteren Generation mehr herauskommen. Auch der Verlaufseintrag wird in diesem Lock angehängt (`ifCurrent`). Aus und wieder
// Ein macht alte Ergebnisse nicht wieder gültig (die Generation wird nie zurückgesetzt). Lock mit kurzer Haltezeit, kein I/O,
// nur für Steuerung und Anzeige, nie im Audio-Callback.

/// Prüfung, die der Controller vor Anzeige und Audiostart macht (und die Tests ohne Sitzung ausüben): Ein Ereignis gilt nur,
/// wenn seine Generation die aktuelle ist. Ein Ereignis mit alter Generation nach einem neueren wird nie angenommen.
public enum AssistantReleaseGate {
    public static func admits(eventGeneration: Int, current: Int) -> Bool { eventGeneration == current }
}

final class AssistantRelease: @unchecked Sendable {
    struct Snapshot: Sendable, Equatable {
        let generation: Int
        let optIn: Bool
        /// Version des Einwilligungstextes, mit der der Opt-in zuletzt eingeschaltet wurde (0: aus).
        let consentVersion: Int
    }

    private let lock = NSLock()
    private var generation = 0
    private var optIn: Bool
    private var consent: Int
    private var runs: [String: @Sendable () -> Void] = [:]
    private var observer: (@Sendable (Int) -> Void)?

    init(optIn: Bool, consentVersion: Int = 0) {
        self.optIn = optIn
        self.consent = optIn ? consentVersion : 0
    }

    /// Generation, Opt-in und Einwilligungsversion zusammen, aus einem Lock: Die Rundenart einer Anfrage und ihre Generation
    /// passen zusammen.
    var snapshot: Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return Snapshot(generation: generation, optIn: optIn, consentVersion: consent)
    }

    var currentGeneration: Int { snapshot.generation }
    var isOptedIn: Bool { snapshot.optIn }

    /// Zahl der angemeldeten Läufe (Tests).
    var registeredCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return runs.count
    }

    /// Der Beobachter wird nach JEDER Erhöhung der Generation gerufen (außerhalb des Locks, auf dem Thread des Auslösers).
    func setObserver(_ observer: (@Sendable (Int) -> Void)?) {
        lock.lock()
        self.observer = observer
        lock.unlock()
    }

    /// Setzt den Opt-in. Nur eine echte Änderung zählt (Generation steigt, laufende Antworten werden widerrufen); bei
    /// unverändertem Zustand wird höchstens die Einwilligungsversion nachgeführt. Gibt die Generation danach zurück.
    @discardableResult
    func setOptIn(_ on: Bool, consentVersion: Int = 0) -> Int {
        lock.lock()
        guard optIn != on else {
            if on { consent = consentVersion }
            let g = generation
            lock.unlock()
            return g
        }
        optIn = on
        consent = on ? consentVersion : 0
        return finishBump()
    }

    /// Bereiche, Cloud-Zustimmung oder Zustand haben sich geändert (oder jede andere Änderung der Freigabe).
    @discardableResult
    func bump() -> Int {
        lock.lock()
        return finishBump()
    }

    /// Mit gehaltenem Lock aufrufen; gibt ihn frei. Die Widerrufe und der Beobachter laufen außerhalb des Locks.
    private func finishBump() -> Int {
        generation += 1
        let g = generation
        let revoke = Array(runs.values)
        let notify = observer
        runs.removeAll()
        lock.unlock()
        for r in revoke { r() }
        notify?(g)
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
    /// dazwischen ist ausgeschlossen. `body` muss kurz sein (Ereignis in den Strom legen, Beitrag anhängen).
    func ifCurrent(_ generation: Int, _ body: () -> Void) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard self.generation == generation else { return false }
        body()
        return true
    }
}

/// Zuordnung „n-tes gesendetes Satz-Ereignis einer Anfrage → Nummer des Schritts im Protokoll“. Der Controller meldet Anzeige
/// und Audiostart mit der Ereignisnummer (`noteCommit`); das Protokoll braucht die Schrittnummer. Der Kern trägt sie VOR dem
/// Senden ein, damit eine schnelle Meldung des Controllers nie ins Leere läuft. Begrenzt; ein Eintrag je Anfrage.
final class AssistantStepIndex: @unchecked Sendable {
    static let maxRequests = 32
    static let maxSteps = 512

    private let lock = NSLock()
    private var steps: [String: [Int]] = [:]
    private var order: [String] = []

    /// Trägt den Schritt des nächsten Ereignisses ein.
    func append(_ id: String, step: Int) {
        lock.lock()
        defer { lock.unlock() }
        if steps[id] == nil {
            order.append(id)
            while order.count > Self.maxRequests { steps[order.removeFirst()] = nil }
        }
        guard steps[id, default: []].count < Self.maxSteps else { return }
        steps[id, default: []].append(step)
    }

    /// Nimmt den zuletzt eingetragenen Schritt zurück (das Ereignis wurde nicht gesendet).
    func removeLast(_ id: String) {
        lock.lock()
        _ = steps[id]?.popLast()
        lock.unlock()
    }

    /// Schrittnummer des `index`-ten Ereignisses (ab 0).
    func step(_ id: String, index: Int) -> Int? {
        lock.lock()
        defer { lock.unlock() }
        guard let list = steps[id], list.indices.contains(index) else { return nil }
        return list[index]
    }
}
