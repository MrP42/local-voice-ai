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
// Geordnete Ausgabegrenze (Runde 4): Jeder Ausgabe-Commit (Anzeige, Übergabe an die Stimme) läuft als `body` von `ifCurrent` im
// selben Lock wie die Erhöhung (Sitzung: `commitOutput`), der Audiostart als `start` des Satzes (`ifCurrentAttempt`, Sitzung:
// `commitStart`): Prüfung, tatsächlicher Start und Ergebniszeile in EINER Sperre. Der `body` ist kurz und synchron, ohne I/O,
// und ruft NIE in die Freigabe zurück (`NSLock` ist nicht rekursiv: ein Zugriff auf `snapshot`, `apply`, `ifCurrent` im `body`
// hinge für immer). Dazu gehört die Wiedergabesperre `AssistantPlaybackGate`: Sie teilt den Lock der Freigabe; der tatsächliche
// Start eines Satzes läuft als `start(generation, body)` in diesem Lock, nach einer Erhöhung beginnt kein Satz älterer Generation.
//
// Eine Änderung, ein Stand (I2, I4): `apply` ändert Generation UND Stand (Bereiche, Opt-in, Cloud-Zustimmung) in einem Lock und
// reiht dort genau eine Zeile `release` ein (`onIncrease`). `snapshot` liest beides zusammen: Eine Anfrage sieht nie eine neue
// Generation mit den alten, größeren Bereichen. Was einschränkt, gilt sofort; was erweitert, erst mit `completeExpansion`, nachdem
// die Zeile dauerhaft in der Datei steht (die Generation steigt dabei nicht noch einmal). Dazwischen steht (g+1, alter engerer
// Stand): sicher. Kein Pfad erhöht die Generation ohne Zeile, und keine Zeile steht ohne Erhöhung.

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
/// Generation steigt (`apply`) im selben Lock, in dem `start` prüft und seinen `body` ausführt. Der tatsächliche Start eines
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

    /// Wie `discard`, der Aufrufer hält den Lock (die Freigabe in `apply`).
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

/// Ergebnis einer Prüfung im Lock der Freigabe: ob die Generation galt und welche Generation dabei im Lock stand (bei
/// Ablehnung die vorgefundene). Protokollzeilen tragen genau diesen Wert und lesen die Generation nie neu.
struct AssistantCheck: Sendable, Equatable {
    enum Result: Sendable, Equatable {
        /// Die Generation galt und die Aktion wurde ausgelöst.
        case done
        /// Die Generation war veraltet: die Aktion lief nicht.
        case stale
        /// Die Generation galt, die Aktion ließ sich aber nicht auslösen (z. B. Start der Stimme gescheitert).
        case failed
    }

    let result: Result
    let generation: Int

    var ok: Bool { result == .done }
    var failed: Bool { result == .failed }
    var stale: Bool { result == .stale }

    init(result: Result, generation: Int) {
        self.result = result
        self.generation = generation
    }

    init(ok: Bool, generation: Int) { self.init(result: ok ? .done : .stale, generation: generation) }

    /// Die Marke für das Protokoll: `a+g`, `a-g` oder `a!g`.
    func mark(_ stage: AssistantCommitStage) -> AssistantCommitMark {
        AssistantCommitMark(stage: stage, ok: ok, generation: generation, failed: failed)
    }
}

/// Wirksamer Stand der Freigabe: alles, was die Generation schützt. Er wird zusammen mit der Generation in EINEM Lock gesetzt und
/// gelesen (`AssistantRelease.snapshot`); keine Anfrage sieht eine neue Generation mit alten Bereichen.
struct AssistantReleaseState: Sendable, Equatable {
    var optIn: Bool
    /// Version des Einwilligungstextes, mit der der Opt-in zuletzt eingeschaltet wurde (0: aus).
    var consentVersion: Int
    /// Bereiche, aus denen Abruf, Bestand und Prüfung dieser Anfrage lesen dürfen.
    var areas: Set<AssistantArea>
    /// Zustimmung zum Cloud-Ziel (nur für ein Cloud-Modell von Belang).
    var cloudConsent: Bool

    /// Der engere von zwei Ständen: Einschränkungen gelten sofort, Erweiterungen erst, wenn ihre Zeile dauerhaft steht.
    func restricted(to target: AssistantReleaseState) -> AssistantReleaseState {
        AssistantReleaseState(optIn: optIn && target.optIn, consentVersion: optIn && target.optIn ? target.consentVersion : 0,
                              areas: areas.intersection(target.areas), cloudConsent: cloudConsent && target.cloudConsent)
    }
}

/// Eine Änderung der Freigabe. Nur die gesetzten Teile ändern sich.
struct AssistantReleaseChange: Sendable {
    var cause: AssistantReleaseCause
    var optIn: (on: Bool, consentVersion: Int)?
    var areas: Set<AssistantArea>?
    var cloudConsent: Bool?
    /// Erhöht die Generation auch ohne geänderten Stand (`resumeFree`, `releaseDidChange` ohne Angaben).
    var force = false
}

final class AssistantRelease: @unchecked Sendable {
    struct Snapshot: Sendable, Equatable {
        let generation: Int
        let state: AssistantReleaseState
        var optIn: Bool { state.optIn }
        var consentVersion: Int { state.consentVersion }
        var areas: Set<AssistantArea> { state.areas }
        var cloudConsent: Bool { state.cloudConsent }
    }

    /// Was bei einer Erhöhung festgehalten wird (für die Zeile `release` im Protokoll). `state` ist der ZIELSTAND der Änderung.
    struct Increase: Sendable, Equatable {
        let generation: Int
        let cause: AssistantReleaseCause
        /// Nur bei Grund `areas` und `consent`: die NEUEN Bereichsnamen, sortiert.
        let areas: [String]?
        let state: AssistantReleaseState
        /// Fehlerschlüssel der Zeile im Schreiber (`flush(request:)`).
        var key: String { "release:\(generation)" }
        var optIn: Bool { state.optIn }
        var consentVersion: Int { state.consentVersion }
    }

    /// Ergebnis von `apply`: die Generation danach und was noch aussteht.
    struct Applied: Sendable, Equatable {
        let generation: Int
        /// Der Stand oder die Generation hat sich geändert (genau dann steht eine Zeile `release` aus).
        let changed: Bool
        /// Der Zielstand ist weiter als der wirksame: Er wird erst mit `completeExpansion` wirksam, wenn die Zeile dauerhaft steht.
        let target: AssistantReleaseState?
        let key: String?
    }

    private let lock: NSLock
    /// Wiedergabesperre: teilt diesen Lock und wird in `apply` mit der Generation vorgerückt.
    let playback: AssistantPlaybackGate
    private var generation = 0
    private var state: AssistantReleaseState
    private var runs: [String: @Sendable () -> Void] = [:]
    private var observer: (@Sendable (Int) -> Void)?
    /// Wird bei JEDER Erhöhung genau einmal gerufen, IM Lock, an der Stelle der Erhöhung (die Zeile steht damit in derselben
    /// Reihenfolge wie die Commit-Zeilen, die in `ifCurrent` laufen). Kurz und ohne I/O (Zeile einreihen), ruft nie zurück.
    private let onIncrease: (@Sendable (Increase) -> Void)?

    init(optIn: Bool, consentVersion: Int = 0, areas: Set<AssistantArea> = Set(AssistantArea.allCases), cloudConsent: Bool = true,
         onIncrease: (@Sendable (Increase) -> Void)? = nil) {
        let shared = NSLock()
        self.lock = shared
        self.playback = AssistantPlaybackGate(lock: shared)
        self.state = AssistantReleaseState(optIn: optIn, consentVersion: optIn ? consentVersion : 0, areas: areas,
                                           cloudConsent: cloudConsent)
        self.onIncrease = onIncrease
    }

    /// Generation und Stand zusammen, aus EINEM Lock: Eine Anfrage liest beides in einem Schritt (Rundenart, Bereiche und
    /// Generation passen zusammen).
    var snapshot: Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return Snapshot(generation: generation, state: state)
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

    /// Wendet eine Änderung an: EINE Erhöhung, EINE Zeile. Im selben Lock steigt die Generation, wird der WIRKSAME Stand gesetzt
    /// und die Zeile eingereiht. Einschränkungen (weniger Bereiche, Opt-in aus, Zustimmung entzogen) wirken sofort und ganz;
    /// Erweiterungen bleiben bis `completeExpansion` aus dem wirksamen Stand heraus (die Generation steigt trotzdem: laufende
    /// Antworten enden). Bei unverändertem Stand und ohne `force` geschieht nichts (keine Erhöhung, keine Zeile); höchstens die
    /// Einwilligungsversion wird nachgeführt. Die Widerrufe und der Beobachter laufen außerhalb des Locks.
    func apply(_ change: AssistantReleaseChange) -> Applied {
        lock.lock()
        var target = state
        if let optIn = change.optIn {
            target.optIn = optIn.on
            target.consentVersion = optIn.on ? optIn.consentVersion : 0
        }
        if let areas = change.areas { target.areas = areas }
        if let granted = change.cloudConsent { target.cloudConsent = granted }
        // Bei unverändertem Opt-in wird nur die Einwilligungsversion nachgeführt; sie allein ändert die Freigabe nicht.
        if target.optIn == state.optIn, target.optIn {
            state.consentVersion = target.consentVersion
        }
        if target == state, !change.force {
            let g = generation
            lock.unlock()
            return Applied(generation: g, changed: false, target: nil, key: nil)
        }
        let effective = state.restricted(to: target)
        let pending = effective == target ? nil : target
        state = effective
        generation += 1
        let g = generation
        playback.discardLocked(below: g)   // im selben Lock: Ab jetzt startet kein Satz älterer Generation mehr
        let increase = Increase(generation: g, cause: change.cause,
                                areas: change.cause == .areas || change.cause == .consent ? Self.names(target.areas) : nil, state: target)
        onIncrease?(increase)
        let revoke = Array(runs.values)
        let notify = observer
        runs.removeAll()
        lock.unlock()
        for r in revoke { r() }
        notify?(g)
        return Applied(generation: g, changed: true, target: pending, key: increase.key)
    }

    /// Macht eine Erweiterung wirksam, nachdem ihre Zeile dauerhaft steht: nur, wenn die Generation seitdem nicht gestiegen ist
    /// (eine spätere Änderung hat Vorrang). Keine neue Generation. `false`: nicht mehr maßgeblich.
    func completeExpansion(_ applied: Applied) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard let target = applied.target, generation == applied.generation else { return false }
        state = target
        return true
    }

    static func names(_ areas: Set<AssistantArea>) -> [String] { areas.map(\.rawValue).sorted() }

    /// Setzt den Opt-in. Nur eine echte Änderung zählt (Generation steigt, laufende Antworten werden widerrufen); bei
    /// unverändertem Zustand wird höchstens die Einwilligungsversion nachgeführt. Gibt die Generation danach zurück (Tests).
    /// Eine Erweiterung gilt hier sofort (die Zeile gilt als dauerhaft geschrieben): nur für Tests der Freigabe allein.
    @discardableResult
    func setOptIn(_ on: Bool, consentVersion: Int = 0) -> Int {
        let applied = apply(AssistantReleaseChange(cause: .optIn, optIn: (on, consentVersion)))
        _ = completeExpansion(applied)
        return applied.generation
    }

    /// Jede andere Änderung der Freigabe ohne Stand (Tests, `resumeFree`): Generation hoch.
    @discardableResult
    func bump(_ cause: AssistantReleaseCause = .other, areas: [String]? = nil) -> Int {
        apply(AssistantReleaseChange(cause: cause, force: true)).generation
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
    /// dazwischen ist ausgeschlossen. Gibt die im Lock geprüfte (bei Ablehnung: die vorgefundene) Generation zurück. `body` muss
    /// kurz sein (Ereignis in den Strom legen, Beitrag anhängen, Anzeige und Übergabe an die Stimme, Zeile einreihen), ohne I/O
    /// und ohne Warten, und darf NICHT in diese Freigabe zurückrufen (`NSLock` ist nicht rekursiv: `snapshot`, `apply`,
    /// `register`, `ifCurrent` im `body` hängen für immer).
    func ifCurrent(_ generation: Int, _ body: () -> Void) -> AssistantCheck {
        lock.lock()
        defer { lock.unlock() }
        guard self.generation == generation else { return AssistantCheck(result: .stale, generation: self.generation) }
        body()
        return AssistantCheck(result: .done, generation: generation)
    }

    /// Wie `ifCurrent`, für Aktionen, die sich nicht auslösen lassen können: `attempt` gibt zurück, ob die Aktion ausgelöst wurde
    /// (`false`: Ergebnis `failed`, die Generation galt, die Aktion nicht). Gilt die Generation nicht, läuft `attempt` nicht.
    /// Dieselben Pflichten wie bei `ifCurrent` (kurz, ohne I/O, kein Rückruf in die Freigabe). Die Ergebniszeile des Protokolls
    /// reiht `attempt` selbst im Lock ein, mit dem Ergebnis der Aktion.
    func ifCurrentAttempt(_ generation: Int, _ attempt: () -> Bool) -> AssistantCheck {
        lock.lock()
        defer { lock.unlock() }
        guard self.generation == generation else { return AssistantCheck(result: .stale, generation: self.generation) }
        return AssistantCheck(result: attempt() ? .done : .failed, generation: generation)
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
