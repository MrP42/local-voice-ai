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
// Eine Änderung, ein Stand (I2, I4): Generation UND Stand (Bereiche, Opt-in, Cloud-Zustimmung) ändern sich in einem Lock;
// `snapshot` liest beides zusammen mit dem Zustand des Bestands (Klasse B, `stale`/bereit): Eine Anfrage sieht nie eine neue
// Generation mit dem alten, größeren Stand.
//
// Gemeinsame Ordnung der Generation für Klasse A und B (ADR-0028 E4.2b, Schreibfolge 8): Die Generation wird NUR in dem
// unteilbaren Schritt vergeben, in dem eine Änderung im laufenden Prozess wirksam wird (derselbe Lock wie die Ausgabe-Commits):
// - Einschränkung (Widerruf, Bereich aus, Zustimmung entzogen): wirkt sofort, erhält dort ihre Generation; die Zustandsdatei und
//   die Zeile folgen (`AssistantSession.settle`).
// - Erweiterung (Einwilligung, Bereich an, Zustimmung erteilt): `apply` ändert nichts und vergibt KEINE Generation; erst
//   `becomeEffective` (nach bestätigtem Austausch der Zustandsdatei) setzt den Stand und erhöht die Generation in einem Schritt.
// - Klasse B (`contentWillChange`): Generation steigt synchron VOR der Änderung im Speicher, der Bestand wird `stale`, bis
//   `contentInstalled` den neuen Bestand meldet.
// Keine Zeile nennt eine geplante oder vorhergesagte Generation.
//
// Sperrordnung: Änderungen der Klasse A werden AUSSERHALB des Locks der Freigabe streng nacheinander gesichert (Karten,
// `AssistantChangeTicket`, ausgegeben im Lock, nur ein Eintrag); der Lock der Freigabe liegt INNEN und wird nie über I/O, Warten
// oder einen Rückruf gehalten. Schreiber (Protokoll, Zustandsdatei) warten nie unter diesem Lock. Reihenfolge der Sperren:
// Karten-Reihe (außen, async) → Lock der Freigabe (innen, kurz) → Einreihen einer Protokollzeile (Blatt, ohne Warten). Klasse B
// kommt über die Meldestelle der Quellen (`AssistantContentWatch`) und liegt davor: Meldestelle (außen) → Lock der Freigabe (innen);
// die Freigabe ruft nie in die Meldestelle zurück. Die Zustandsdatei hat eine eigene serielle Warteschlange (`AssistantReleaseStore`),
// auf die nur außerhalb jedes dieser Locks gewartet wird.

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
    /// Inhalt oder Schutzregel einer Quelle geändert (Klasse B: Lesegrenze, KI-Freigabe, Ablage, Quelle geändert oder gelöscht);
    /// die Zeile nennt weder Titel noch Inhalt.
    case content
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

extension AssistantReleaseState {
    var isEmpty: Bool { !optIn && areas.isEmpty && !cloudConsent }

    /// Der Teil, den `next` gegenüber `self` HINZUFÜGT (nil: nichts). Er ist der Gegenstand einer Erweiterung.
    func addition(to next: AssistantReleaseState) -> AssistantReleaseState? {
        let turnsOn = next.optIn && !optIn
        let add = AssistantReleaseState(optIn: turnsOn, consentVersion: turnsOn ? next.consentVersion : 0,
                                        areas: next.areas.subtracting(areas), cloudConsent: next.cloudConsent && !cloudConsent)
        return add.isEmpty ? nil : add
    }

    /// `self` erweitert um `add` (den noch verbliebenen Teil eines offenen Erweiterungsschritts; spätere Einschränkungen haben ihn
    /// im Register der offenen Schritte schon gekürzt, `AssistantRelease.open`).
    func expanded(by add: AssistantReleaseState) -> AssistantReleaseState {
        let turnsOn = add.optIn && !optIn
        let on = optIn || turnsOn
        return AssistantReleaseState(optIn: on, consentVersion: on ? (turnsOn ? add.consentVersion : consentVersion) : 0,
                                     areas: areas.union(add.areas), cloudConsent: cloudConsent || add.cloudConsent)
    }

    /// Der leere Stand (nichts ist eingeschaltet).
    static let none = AssistantReleaseState(optIn: false, consentVersion: 0, areas: [], cloudConsent: false)
}

/// Eine Sicherungsfolge in der Reihe der Änderungen der Klasse A. Die Karten werden im Lock der Freigabe in der Reihenfolge der
/// Änderungen ausgegeben (nur ein Eintrag, nie Warten); die Folge beginnt erst, wenn die vorige abgeschlossen oder gescheitert ist
/// (`turn`, `complete`). Eine Karte, deren Folge nie läuft (Aufrufer ruft `settle` nicht), gibt sich beim Aufräumen frei.
final class AssistantChangeTicket: @unchecked Sendable {
    private let lock = NSLock()
    private var previous: AssistantChangeTicket?
    private var done = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(previous: AssistantChangeTicket?) { self.previous = previous }

    deinit { complete() }

    /// Wartet, bis die vorige Folge abgeschlossen ist.
    func turn() async {
        await takePrevious()?.finished()
    }

    private func takePrevious() -> AssistantChangeTicket? {
        lock.lock()
        defer { lock.unlock() }
        let before = previous
        previous = nil
        return before
    }

    private func finished() async {
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            lock.lock()
            if done {
                lock.unlock()
                continuation.resume()
            } else {
                waiters.append(continuation)
                lock.unlock()
            }
        }
    }

    func complete() {
        lock.lock()
        done = true
        let resume = waiters
        waiters.removeAll()
        lock.unlock()
        for continuation in resume { continuation.resume() }
    }
}

final class AssistantRelease: @unchecked Sendable {
    struct Snapshot: Sendable, Equatable {
        let generation: Int
        let state: AssistantReleaseState
        /// Die Zustandsdatei war beim Start ungültig: kein Bereich, keine Zustimmung, bis der Nutzer neu setzt und das gesichert ist.
        let locked: Bool
        /// Eine Einschränkung ließ sich nicht sichern: für den Rest des Laufs nichts aus eigenen Inhalten.
        let blocked: Bool
        /// Der Bestand steht für die jüngste Änderung eines Inhalts (Klasse B), nicht `stale`.
        let corpusReady: Bool
        /// Zahl der Änderungen von Inhalten (Klasse B) bis zu dieser Generation.
        let contentEpoch: Int

        init(generation: Int, state: AssistantReleaseState, locked: Bool = false, blocked: Bool = false, corpusReady: Bool = true,
             contentEpoch: Int = 0) {
            self.generation = generation
            self.state = state
            self.locked = locked
            self.blocked = blocked
            self.corpusReady = corpusReady
            self.contentEpoch = contentEpoch
        }

        var optIn: Bool { state.optIn }
        var consentVersion: Int { state.consentVersion }
        var areas: Set<AssistantArea> { state.areas }
        var cloudConsent: Bool { state.cloudConsent }

        /// Aus eigenen Inhalten darf ausgegeben werden: nicht gesperrt, nicht blockiert, Bestand nicht `stale`.
        var contentAllowed: Bool { !locked && !blocked && corpusReady }

        /// Die Ansicht einer Anfrage: ohne Zugang zu eigenen Inhalten sind die Bereiche leer. Alles, was die Anfrage liest, kommt
        /// aus diesem EINEN Lesen von Generation, Stand und Bestand.
        var forRequest: Snapshot {
            guard !contentAllowed, !state.areas.isEmpty else { return self }
            var gated = state
            gated.areas = []
            return Snapshot(generation: generation, state: gated, locked: locked, blocked: blocked, corpusReady: corpusReady,
                            contentEpoch: contentEpoch)
        }
    }

    /// Was bei einer Erhöhung im Lock festgehalten wird (für die Zeile im Protokoll). Einschränkungen schreiben ihre Zeile erst
    /// nach der Zustandsdatei (`AssistantSession.settle`); hier stehen Erweiterungen (wirksam), Änderungen von Inhalten und die
    /// Blockade.
    struct Increase: Sendable, Equatable {
        enum Kind: Sendable, Equatable {
            /// Eine Erweiterung wurde wirksam (Zeile „wirksam“ mit Lauf, Änderungsnummer und dieser Generation).
            case effective
            /// Ein Inhalt oder eine Schutzregel hat sich geändert (Klasse B, Ursache „Inhalt“).
            case content
            /// Eine Einschränkung ließ sich nicht sichern: Blockade für den Rest des Laufs.
            case block
        }

        let kind: Kind
        let generation: Int
        /// Änderungsnummer (je Lauf); `nil` bei Änderungen von Inhalten.
        let change: Int?
        let cause: AssistantReleaseCause
        let state: AssistantReleaseState
        /// Fehlerschlüssel der Zeile im Schreiber (mit der Kennung der Ordnung: je Lauf und Änderung eindeutig).
        let key: String
        var optIn: Bool { state.optIn }
        var consentVersion: Int { state.consentVersion }
    }

    /// Ein Teil einer Änderung der Klasse A. Eine Änderung wird zerlegt: erst der einschränkende Teil (wirkt sofort, eigene
    /// Generation), dann der erweiternde (wirkt erst nach bestätigtem Austausch der Zustandsdatei, Generation im Schritt des
    /// Wirksamwerdens). Ohne Änderung des Stands (`force`) gibt es ein Ereignis ohne Datei.
    struct Step: Sendable {
        enum Kind: Sendable, Equatable { case restriction, expansion, event }

        let kind: Kind
        /// Änderungsnummer, fortlaufend je Lauf.
        let number: Int
        let cause: AssistantReleaseCause
        /// Einschränkung und Ereignis: die beim Anwenden vergebene Generation. Erweiterung: 0 (wird erst beim Wirksamwerden vergeben).
        let generation: Int
        /// Erweiterung: der hinzukommende Teil. Einschränkung und Ereignis: der wirksame Stand unmittelbar nach dem Anwenden.
        let delta: AssistantReleaseState
        let ticket: AssistantChangeTicket
        /// Fehlerschlüssel der Zeilen dieser Änderung im Schreiber (je Lauf und Änderung eindeutig).
        let key: String
    }

    /// Ergebnis von `apply`: die Generation nach dem sofort wirkenden Teil und die Teile, die noch zu sichern sind.
    struct Applied: Sendable {
        let generation: Int
        /// Der Stand oder die Generation hat sich geändert (genau dann steht eine Sicherungsfolge aus).
        let changed: Bool
        let steps: [Step]
        var expands: Bool { steps.contains { $0.kind == .expansion } }
    }

    /// Die prozessweite Ordnung: Lock, Generation, Wiedergabesperre, Register der Anfragen, Beobachter, Zähler, Reihe, Lauf-Token.
    let clock: AssistantReleaseClock
    private var lock: NSLock { clock.lock }
    /// Kennung dieser Sicht im Register der Anfragen und bei den Beobachtern.
    private let owner = UUID().uuidString
    /// Wiedergabesperre: teilt den Lock der Ordnung und wird mit der Generation vorgerückt.
    var playback: AssistantPlaybackGate { clock.playback }
    /// Der WIRKSAME Stand dieser Sicht.
    private var state: AssistantReleaseState
    /// Der beabsichtigte Stand: wirksamer Stand plus alle noch offenen Erweiterungen (immer aus beidem berechnet).
    private var target: AssistantReleaseState
    /// Register der offenen Erweiterungsschritte (Nummer → der noch verbliebene Teil). Eine Einschränkung streicht die entzogenen
    /// Teile aus ALLEN offenen Schritten (im selben Lock); ein Schritt macht nur seinen verbliebenen Teil wirksam, unabhängig vom
    /// späteren Ziel. Ein später beantragter Teil wirkt nur über die eigene Folge.
    private var open: [Int: AssistantReleaseState] = [:]
    private var lockedState: Bool
    private var blockedState: Bool
    private var contentEpoch = 0
    private var corpusEpoch = 0
    /// Wird bei Wirksamwerden einer Erweiterung, bei Änderungen von Inhalten und bei der Blockade genau einmal gerufen, IM Lock, an
    /// der Stelle der Erhöhung (die Zeile steht damit in derselben Reihenfolge wie die Commit-Zeilen). Kurz und ohne I/O (Zeile
    /// einreihen), ruft nie zurück.
    private let onIncrease: (@Sendable (Increase) -> Void)?

    init(optIn: Bool, consentVersion: Int = 0, areas: Set<AssistantArea> = Set(AssistantArea.allCases), cloudConsent: Bool = true,
         locked: Bool = false, blocked: Bool = false, clock: AssistantReleaseClock = AssistantReleaseClock(),
         onIncrease: (@Sendable (Increase) -> Void)? = nil) {
        self.clock = clock
        let initial = AssistantReleaseState(optIn: optIn, consentVersion: optIn ? consentVersion : 0, areas: areas, cloudConsent: cloudConsent)
        self.state = initial
        self.target = initial
        self.lockedState = locked
        self.blockedState = blocked
        self.onIncrease = onIncrease
    }

    /// Der beabsichtigte Stand aus dem wirksamen und den offenen Schritten (Aufrufer hält den Lock).
    private func recomputeTargetLocked() {
        open = open.filter { !$0.value.isEmpty }
        var next = state
        for number in open.keys.sorted() { next = next.expanded(by: open[number] ?? .none) }
        target = next
    }

    private func runKey(_ id: String) -> String { "\(owner)/\(id)" }

    /// Generation, Stand und Zustand des Bestands zusammen, aus EINEM Lock: Eine Anfrage liest alles in einem Schritt.
    var snapshot: Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return Snapshot(generation: clock.generationLocked, state: state, locked: lockedState, blocked: blockedState,
                        corpusReady: corpusEpoch == contentEpoch, contentEpoch: contentEpoch)
    }

    var currentGeneration: Int { snapshot.generation }
    var isOptedIn: Bool { snapshot.optIn }

    /// Zahl der angemeldeten Läufe (Tests).
    var registeredCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return clock.registeredCountLocked(prefix: owner + "/")
    }

    /// Der Beobachter dieser Sicht wird nach JEDER Erhöhung der Generation der Ordnung gerufen (auch nach der einer anderen Sitzung;
    /// außerhalb des Locks, auf dem Thread des Auslösers).
    func setObserver(_ observer: (@Sendable (Int) -> Void)?) {
        lock.lock()
        clock.setObserverLocked(ObjectIdentifier(self), observer)
        lock.unlock()
    }

    /// Eine Karte in der Reihe der Sicherungsfolgen, ohne Änderung (Wiederholung der Sicherung).
    func enqueueTicket() -> AssistantChangeTicket { clock.nextTicket() }

    /// Wendet eine Änderung der Klasse A an. Sie wird gegen den beabsichtigten Stand (`target`) verglichen und zerlegt:
    /// - Einschränkung (etwas fällt weg): wirkt sofort, im selben Lock steigt die Generation und wird der Stand gesetzt (eine
    ///   Änderung, ein Stand). Laufende Antworten enden.
    /// - Erweiterung (etwas kommt dazu): ändert hier NICHTS und erhält hier KEINE Generation. Sie wird erst mit `becomeEffective`
    ///   wirksam, nachdem die Zustandsdatei bestätigt ist; dort steigt die Generation im unteilbaren Schritt.
    /// - Eine gemischte Änderung ergibt zwei Teile, erst den einschränkenden, dann den erweiternden.
    /// - Ohne Änderung des Stands und ohne `force` geschieht nichts (höchstens die Einwilligungsversion wird nachgeführt).
    /// Die Teile tragen Karten der Reihe: Ihre Sicherungsfolgen laufen streng nacheinander (`AssistantChangeTicket`).
    func apply(_ change: AssistantReleaseChange) -> Applied {
        lock.lock()
        var next = target
        if let optIn = change.optIn {
            next.optIn = optIn.on
            next.consentVersion = optIn.on ? optIn.consentVersion : 0
        }
        if let areas = change.areas { next.areas = areas }
        if let granted = change.cloudConsent { next.cloudConsent = granted }
        // Bei unverändertem Opt-in wird nur die Einwilligungsversion nachgeführt; sie allein ändert die Freigabe nicht.
        if next.optIn == target.optIn, next.optIn {
            target.consentVersion = next.consentVersion
            if state.optIn { state.consentVersion = next.consentVersion }
        }
        let removes = target.restricted(to: next) != target
        let adds = target.addition(to: next)
        if !removes, adds == nil, !change.force {
            let g = clock.generationLocked
            lock.unlock()
            return Applied(generation: g, changed: false, steps: [])
        }
        var steps: [Step] = []
        var raised = false
        if removes {
            state = state.restricted(to: next)
            // Die Einschränkung streicht die entzogenen Teile aus allen früher ausgegebenen, noch nicht wirksamen Erweiterungen.
            for (number, remaining) in open { open[number] = remaining.restricted(to: next) }
            let number = clock.nextChangeLocked()
            let g = clock.raiseLocked()
            steps.append(Step(kind: .restriction, number: number, cause: change.cause, generation: g, delta: state,
                              ticket: clock.nextTicketLocked(), key: clock.key("chg", number)))
            raised = true
        }
        if let adds {
            let number = clock.nextChangeLocked()
            open[number] = adds
            steps.append(Step(kind: .expansion, number: number, cause: change.cause, generation: 0, delta: adds,
                              ticket: clock.nextTicketLocked(), key: clock.key("chg", number)))
        }
        if steps.isEmpty {
            let number = clock.nextChangeLocked()
            let g = clock.raiseLocked()
            steps.append(Step(kind: .event, number: number, cause: change.cause, generation: g, delta: state,
                              ticket: clock.nextTicketLocked(), key: clock.key("chg", number)))
            raised = true
        }
        recomputeTargetLocked()
        let g = clock.generationLocked
        let after = raised ? clock.drainLocked(g) : nil
        lock.unlock()
        after?.finish()
        return Applied(generation: g, changed: true, steps: steps)
    }

    /// Der Stand, den die Zustandsdatei für diesen Teil erhalten soll: bei einer Erweiterung der wirksame Stand mit dem noch
    /// verbliebenen Teil DIESES Schritts (nie mit einem späteren Ziel), sonst der wirksame Stand. Gelesen im Lock, damit eine
    /// zwischenzeitliche Einschränkung in die Datei eingeht (und eine frühere, nicht gesicherte gleich mit).
    func viewForSave(_ step: Step) -> AssistantReleaseState {
        lock.lock()
        defer { lock.unlock() }
        return step.kind == .expansion ? state.expanded(by: open[step.number] ?? .none) : state
    }

    /// Eine Erweiterung wird wirksam, im unteilbaren Schritt: Stand setzen, Generation erhöhen, Wiedergabesperre vorrücken, Zeile
    /// „wirksam“ einreihen, alles in EINEM Lock. Nur nach bestätigtem Austausch der Zustandsdatei aufrufen. Gesetzt wird nur der
    /// verbliebene Teil des eigenen Schritts. `nil`: nicht möglich (blockiert, oder eine Einschränkung hat den Teil entzogen).
    func becomeEffective(_ step: Step) -> Int? {
        lock.lock()
        guard !blockedState, let remaining = open[step.number], !remaining.isEmpty else {
            lock.unlock()
            return nil
        }
        open[step.number] = nil
        state = state.expanded(by: remaining)
        lockedState = false
        recomputeTargetLocked()
        let g = clock.raiseLocked()
        onIncrease?(Increase(kind: .effective, generation: g, change: step.number, cause: step.cause, state: state, key: step.key))
        let after = clock.drainLocked(g)
        lock.unlock()
        after.finish()
        return g
    }

    /// Eine Erweiterung wird in diesem Lauf nicht wirksam: Ihr Schritt verlässt das Register, der beabsichtigte Stand verliert, was
    /// nur sie wollte (andere offene Schritte bleiben), und dieselbe Erweiterung lässt sich noch einmal anstoßen.
    func abandon(_ step: Step) {
        lock.lock()
        open[step.number] = nil
        recomputeTargetLocked()
        lock.unlock()
    }

    /// Die Zustandsdatei ist (neu) gesichert: eine gesperrte Freigabe ist damit wieder gültig.
    func clearLocked() {
        lock.lock()
        lockedState = false
        lock.unlock()
    }

    /// Eine Einschränkung ließ sich nicht sichern: Blockade für den Rest des Laufs, im selben unteilbaren Schritt mit eigener
    /// Generation (der Bestand an Blöcken, der freigegeben ist, ändert sich). Gibt die Generation zurück.
    @discardableResult
    func block() -> Int {
        lock.lock()
        if blockedState {
            let g = clock.generationLocked
            lock.unlock()
            return g
        }
        blockedState = true
        let number = clock.nextChangeLocked()
        let g = clock.raiseLocked()
        onIncrease?(Increase(kind: .block, generation: g, change: number, cause: .other, state: state, key: clock.key("chg", number)))
        let after = clock.drainLocked(g)
        lock.unlock()
        after.finish()
        return g
    }

    var isBlocked: Bool { snapshot.blocked }

    /// Klasse B: Ein Inhalt oder eine Schutzregel ändert sich (Lesegrenze, KI-Freigabe, Schutz der Ablage, Quelle geändert oder
    /// gelöscht). Synchron und VOR der Änderung im Speicher: Die Generation steigt im selben Lock wie bei Klasse A (gemeinsame
    /// Ordnung), der Bestand wird `stale` (keine Ausgabe aus eigenen Inhalten, bis der neue Bestand steht), die Zeile mit Ursache
    /// „Inhalt“ wird nur eingereiht. Gibt Generation und Epoche zurück; `contentInstalled(epoch:)` meldet den neuen Bestand.
    @discardableResult
    func contentWillChange() -> (generation: Int, epoch: Int) {
        lock.lock()
        contentEpoch += 1
        let epoch = contentEpoch
        let g = clock.raiseLocked()
        onIncrease?(Increase(kind: .content, generation: g, change: nil, cause: .content, state: state, key: clock.key("gen", g)))
        let after = clock.drainLocked(g)
        lock.unlock()
        after.finish()
        return (g, epoch)
    }

    /// Der Bestand steht für die Änderung dieser Epoche (und alle früheren). Keine zweite Erhöhung: Die Änderung selbst hat die
    /// Generation schon erhöht. `false`: eine neuere Änderung ist inzwischen dazugekommen, der Bestand bleibt `stale`.
    @discardableResult
    func contentInstalled(epoch: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        corpusEpoch = max(corpusEpoch, min(epoch, contentEpoch))
        return corpusEpoch == contentEpoch
    }

    static func names(_ areas: Set<AssistantArea>) -> [String] { areas.map(\.rawValue).sorted() }

    /// Macht alle Erweiterungen eines `apply` sofort wirksam, ohne Datei und ohne Zeilen (Tests der Freigabe allein). `true`, wenn
    /// alle wirksam wurden.
    @discardableResult
    func completeExpansion(_ applied: Applied) -> Bool {
        var all = true
        for step in applied.steps where step.kind == .expansion {
            if becomeEffective(step) == nil { all = false }
        }
        for step in applied.steps { step.ticket.complete() }
        return all
    }

    /// Setzt den Opt-in. Nur eine echte Änderung zählt (Generation steigt, laufende Antworten werden widerrufen); bei
    /// unverändertem Zustand wird höchstens die Einwilligungsversion nachgeführt. Gibt die Generation danach zurück (Tests).
    /// Eine Erweiterung gilt hier sofort (die Sicherung gilt als bestätigt): nur für Tests der Freigabe allein.
    @discardableResult
    func setOptIn(_ on: Bool, consentVersion: Int = 0) -> Int {
        let applied = apply(AssistantReleaseChange(cause: .optIn, optIn: (on, consentVersion)))
        completeExpansion(applied)
        return currentGeneration
    }

    /// Jede andere Änderung der Freigabe ohne Stand (Tests, `resumeFree`): Generation hoch.
    @discardableResult
    func bump(_ cause: AssistantReleaseCause = .other, areas: [String]? = nil) -> Int {
        let applied = apply(AssistantReleaseChange(cause: cause, force: true))
        for step in applied.steps { step.ticket.complete() }
        return applied.generation
    }

    /// Meldet einen Lauf an. `false`, wenn die Generation schon veraltet ist (dann wurde `revoke` nicht aufgenommen; der
    /// Aufrufer widerruft selbst).
    func register(_ id: String, generation: Int, revoke: @escaping @Sendable () -> Void) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard clock.generationLocked == generation else { return false }
        clock.registerLocked(runKey(id), revoke: revoke)
        return true
    }

    func unregister(_ id: String) {
        lock.lock()
        clock.unregisterLocked(runKey(id))
        lock.unlock()
    }

    /// Führt `body` im Lock der Ordnung aus, mit der dort stehenden Generation (für Zeilen, die in der Reihenfolge der Erhöhungen
    /// stehen müssen, auch wenn die Aktion außerhalb lief: gemeldete Ergebnisse des Controllers). Dieselben Pflichten wie bei `ifCurrent`.
    func inSection<T>(_ body: (Int) -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body(clock.generationLocked)
    }

    /// Führt `body` nur aus, wenn `generation` die aktuelle ist, und zwar im selben Lock wie die Prüfung: Eine Änderung
    /// dazwischen ist ausgeschlossen. Gibt die im Lock geprüfte (bei Ablehnung: die vorgefundene) Generation zurück. `body` muss
    /// kurz sein (Ereignis in den Strom legen, Beitrag anhängen, Anzeige und Übergabe an die Stimme, Zeile einreihen), ohne I/O
    /// und ohne Warten, und darf NICHT in diese Freigabe zurückrufen (`NSLock` ist nicht rekursiv: `snapshot`, `apply`,
    /// `register`, `ifCurrent` im `body` hängen für immer).
    func ifCurrent(_ generation: Int, onStale: ((Int) -> Void)? = nil, _ body: () -> Void) -> AssistantCheck {
        lock.lock()
        defer { lock.unlock() }
        let found = clock.generationLocked
        guard found == generation else {
            onStale?(found)
            return AssistantCheck(result: .stale, generation: found)
        }
        body()
        return AssistantCheck(result: .done, generation: generation)
    }

    /// Wie `ifCurrent`, für Aktionen, die sich nicht auslösen lassen können: `attempt` gibt zurück, ob die Aktion ausgelöst wurde
    /// (`false`: Ergebnis `failed`, die Generation galt, die Aktion nicht). Gilt die Generation nicht, läuft `attempt` nicht und
    /// `onStale` (Ablehnungszeile, IM Lock mit der vorgefundenen Generation: eine Zeile „verworfen“ steht nie hinter der Zeile einer
    /// späteren Änderung). Dieselben Pflichten wie bei `ifCurrent` (kurz, ohne I/O, kein Rückruf in die Freigabe). Die Ergebniszeile
    /// des Protokolls reiht `attempt` selbst im Lock ein, mit dem Ergebnis der Aktion.
    func ifCurrentAttempt(_ generation: Int, onStale: ((Int) -> Void)? = nil, _ attempt: () -> Bool) -> AssistantCheck {
        lock.lock()
        defer { lock.unlock() }
        let found = clock.generationLocked
        guard found == generation else {
            onStale?(found)
            return AssistantCheck(result: .stale, generation: found)
        }
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

    /// Diese Anfrage hat in diesem Prozesslauf Ereignisse ausgegeben (nur dann gelten ihre Commits).
    func knows(_ id: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return steps[id] != nil
    }

    /// Schrittnummer des `index`-ten Ereignisses (ab 0).
    func step(_ id: String, index: Int) -> Int? {
        lock.lock()
        defer { lock.unlock() }
        guard let list = steps[id], list.indices.contains(index) else { return nil }
        return list[index]
    }
}
