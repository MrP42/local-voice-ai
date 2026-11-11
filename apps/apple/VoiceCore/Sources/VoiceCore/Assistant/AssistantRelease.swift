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
//
// Geordnete Ausgabegrenze (Runde 4): Jeder Ausgabe-Commit (Anzeige, Übergabe an die Stimme, Audiostart-Meldung) läuft als `body`
// von `ifCurrent` im selben Lock wie die Erhöhung (Sitzung: `commitOutput`). Der `body` ist kurz und synchron, ohne I/O, und
// ruft NIE in die Freigabe zurück (`NSLock` ist nicht rekursiv: ein Zugriff auf `snapshot`, `bump`, `ifCurrent` im `body`
// hinge für immer). Dazu gehört die Wiedergabesperre `AssistantPlaybackGate`: Sie teilt den Lock der Freigabe; der tatsächliche
// Start eines Satzes läuft als `start(generation, body)` in diesem Lock, nach einer Erhöhung beginnt kein Satz älterer Generation.
//
// Protokoll der Erhöhungen (Runde 5): Jede Erhöhung (`finishBump`, die einzige Stelle) reiht im selben Lock genau eine Zeile
// `release` ein (`onIncrease`): neue Generation, Grund (`AssistantReleaseCause`), bei Bereichen die neuen Bereichsnamen. Kein
// Pfad erhöht die Generation ohne diese Zeile, und keine Zeile steht ohne Erhöhung.

/// Grund einer Erhöhung der Freigabe-Generation (feste Liste, kein Freitext). Jede Erhöhung schreibt der Kern als genau eine
/// Zeile `release` ins Protokoll, mit diesem Grund und der neuen Generation.
public enum AssistantReleaseCause: String, Sendable, Codable, Equatable, CaseIterable {
    /// Opt-in „Antworten ohne Quelle“ an oder aus.
    case optIn
    /// Bereiche geändert (die Zeile trägt die NEUEN Bereichsnamen).
    case areas
    /// Cloud-Zustimmung erteilt oder entzogen.
    case consent
    /// „Ohne Quelle fortsetzen“ (Zustand `.data` aufgehoben).
    case resumeFree
    /// Jede andere Änderung der Freigabe.
    case other
}

/// Prüfung, die der Controller vor Anzeige und Audiostart macht (und die Tests ohne Sitzung ausüben): Ein Ereignis gilt nur,
/// wenn seine Generation die aktuelle ist. Ein Ereignis mit alter Generation nach einem neueren wird nie angenommen.
public enum AssistantReleaseGate {
    public static func admits(eventGeneration: Int, current: Int) -> Bool { eventGeneration == current }
}

/// Sperre „verworfen ab Generation g“ für die Wiedergabe. Reiner Typ ohne Aktor. Sie teilt den LOCK der Freigabe: Die
/// Generation steigt (`finishBump`) im selben Lock, in dem `start` prüft und seinen `body` ausführt. Der tatsächliche Start eines
/// Satzes (Systemstimme: `synth.speak`; Phonon: Einreihen des ersten Tons) läuft als `body` von `start`: Nach der Rückkehr
/// einer Erhöhung auf eine neuere Generation beginnt kein Satz älterer Generation mehr (kein Start-Body danach). `body`: kurz,
/// synchron, ohne I/O und Warten, ruft NIE zurück (weder `start` noch `permits` noch die Freigabe: `NSLock` ist nicht rekursiv).
/// `permits` ist nur eine VORPRÜFUNG (billig, früh aussteigen), kein Start. Die Grenze: Ein Satz, dessen Klang schon begonnen
/// hat, verstummt erst, wenn der MainActor `stop()` ausführt (Aufräumen im Controller, einige Millisekunden); der Speaker
/// gehört dem MainActor und ist von anderen Threads nicht zu stoppen.
public final class AssistantPlaybackGate: @unchecked Sendable {
    private let lock: NSLock
    private var floor = 0

    /// Eigener Lock (reine Verwendung ohne Freigabe, Tests).
    public init() { lock = NSLock() }

    /// Der Lock der Freigabe (`AssistantRelease`).
    init(lock: NSLock) { self.lock = lock }

    /// Alles unterhalb von `generation` ist verworfen. Monoton. Der Aufrufer hält den Lock NICHT.
    func discard(below generation: Int) {
        lock.lock()
        discardLocked(below: generation)
        lock.unlock()
    }

    /// Wie `discard`, der Aufrufer hält den Lock (die Freigabe in `finishBump`).
    func discardLocked(below generation: Int) {
        if generation > floor { floor = generation }
    }

    /// Vorprüfung: Darf ein Satz dieser Generation noch starten? Kein Start; zwischen dieser Antwort und dem Start kann die
    /// Generation steigen, entscheidend ist nur `start`. Nie im `body` von `start` oder `commitOutput` aufrufen (Lock).
    public func permits(_ generation: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return generation >= floor
    }

    /// Der tatsächliche Start: führt `body` aus, wenn die Generation nicht verworfen ist, und zwar im selben Lock, in dem die
    /// Generation steigt. `false`: verworfen, `body` lief nicht (nichts wurde gestartet; der Satz gilt als nicht gesprochen).
    func start(_ generation: Int, _ body: () -> Void) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard generation >= floor else { return false }
        body()
        return true
    }

    /// Erste Generation, die noch starten darf (Tests, Anzeige). Nie im `body` aufrufen.
    public var discardedBelow: Int {
        lock.lock()
        defer { lock.unlock() }
        return floor
    }
}

final class AssistantRelease: @unchecked Sendable {
    struct Snapshot: Sendable, Equatable {
        let generation: Int
        let optIn: Bool
        /// Version des Einwilligungstextes, mit der der Opt-in zuletzt eingeschaltet wurde (0: aus).
        let consentVersion: Int
    }

    /// Was bei einer Erhöhung festgehalten wird (für die Zeile `release` im Protokoll).
    struct Increase: Sendable, Equatable {
        let generation: Int
        let cause: AssistantReleaseCause
        /// Nur bei Grund `areas` (und `consent` mit Bereichen): die NEUEN Bereichsnamen, sortiert.
        let areas: [String]?
        let optIn: Bool
        let consentVersion: Int
    }

    private let lock: NSLock
    /// Wiedergabesperre: teilt diesen Lock und wird in `finishBump` mit der Generation vorgerückt.
    let playback: AssistantPlaybackGate
    private var generation = 0
    private var optIn: Bool
    private var consent: Int
    private var runs: [String: @Sendable () -> Void] = [:]
    private var observer: (@Sendable (Int) -> Void)?
    /// Wird bei JEDER Erhöhung genau einmal gerufen, IM Lock, an der Stelle der Erhöhung (die Zeile steht damit in derselben
    /// Reihenfolge wie die Commit-Zeilen, die in `ifCurrent` laufen). Kurz und ohne I/O (Zeile einreihen), ruft nie zurück.
    private let onIncrease: (@Sendable (Increase) -> Void)?

    init(optIn: Bool, consentVersion: Int = 0, onIncrease: (@Sendable (Increase) -> Void)? = nil) {
        let shared = NSLock()
        self.lock = shared
        self.playback = AssistantPlaybackGate(lock: shared)
        self.optIn = optIn
        self.consent = optIn ? consentVersion : 0
        self.onIncrease = onIncrease
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
        return finishBump(.optIn, areas: nil)
    }

    /// Bereiche, Cloud-Zustimmung oder Zustand haben sich geändert (oder jede andere Änderung der Freigabe). `cause` kommt als
    /// Grund ins Protokoll, `areas` (nur bei `areas`/`consent`) sind die neuen Bereichsnamen.
    @discardableResult
    func bump(_ cause: AssistantReleaseCause = .other, areas: [String]? = nil) -> Int {
        lock.lock()
        return finishBump(cause, areas: areas)
    }

    /// Mit gehaltenem Lock aufrufen; gibt ihn frei. Die Widerrufe und der Beobachter laufen außerhalb des Locks.
    private func finishBump(_ cause: AssistantReleaseCause, areas: [String]?) -> Int {
        generation += 1
        let g = generation
        playback.discardLocked(below: g)   // im selben Lock: Ab jetzt startet kein Satz älterer Generation mehr
        onIncrease?(Increase(generation: g, cause: cause, areas: areas, optIn: optIn, consentVersion: consent))
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
    /// dazwischen ist ausgeschlossen. `body` muss kurz sein (Ereignis in den Strom legen, Beitrag anhängen, Anzeige und
    /// Übergabe an die Stimme), ohne I/O und ohne Warten, und darf NICHT in diese Freigabe zurückrufen (`NSLock` ist nicht
    /// rekursiv: `snapshot`, `bump`, `register`, `ifCurrent` im `body` hängen für immer).
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
