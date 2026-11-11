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

    /// Die prozessweite Ordnung: Lock, Generation, Stand der Klasse A, Epochen, Wiedergabesperre, Register der Anfragen, Beobachter,
    /// Zähler, Reihe, Lauf-Token. DIESE Klasse hält davon nichts: Sie ist die Sicht einer Sitzung (lokal oder Cloud, Ziel) auf den
    /// einen Stand der Ordnung und liest ihn im selben Lesen wie die Generation (Runde 8, R3).
    let clock: AssistantReleaseClock
    let view: AssistantReleaseView
    private var lock: NSLock { clock.lock }
    /// Kennung dieser Sicht im Register der Anfragen und bei den Beobachtern.
    private let owner = UUID().uuidString
    /// Wiedergabesperre: teilt den Lock der Ordnung und wird mit der Generation vorgerückt.
    var playback: AssistantPlaybackGate { clock.playback }
    /// Wird bei Wirksamwerden einer Erweiterung und bei der Blockade genau einmal gerufen, IM Lock, an der Stelle der Erhöhung (die
    /// Zeile steht damit in derselben Reihenfolge wie die Commit-Zeilen). Kurz und ohne I/O (Zeile einreihen), ruft nie zurück.
    private let onIncrease: (@Sendable (Increase) -> Void)?

    /// Eine Sicht auf den Stand einer Ordnung.
    init(clock: AssistantReleaseClock, view: AssistantReleaseView, onIncrease: (@Sendable (Increase) -> Void)? = nil) {
        self.clock = clock
        self.view = view
        self.onIncrease = onIncrease
    }

    /// Eine Ordnung für sich mit einem Anfangsstand (Tests der Freigabe allein, Sitzung ohne Speicher): setzt den Stand der Ordnung.
    /// Die Sicht ist die eines Cloud-Ziels ohne Kennung (die Zustimmung braucht den ausdrücklichen Eintrag).
    convenience init(optIn: Bool, consentVersion: Int = 0, areas: Set<AssistantArea> = Set(AssistantArea.allCases), cloudConsent: Bool = true,
                     locked: Bool = false, blocked: Bool = false, clock: AssistantReleaseClock = AssistantReleaseClock(),
                     onIncrease: (@Sendable (Increase) -> Void)? = nil) {
        self.init(clock: clock, view: AssistantReleaseView(cloud: true, target: nil), onIncrease: onIncrease)
        clock.seed(record: AssistantReleaseRecord.seeded(optIn: optIn, consentVersion: consentVersion, areas: areas,
                                                         cloudConsent: cloudConsent, target: nil), locked: locked)
        if blocked { clock.markBlocked() }
    }

    /// Generation, Stand der Sicht und Zustand des Bestands zusammen, aus EINEM Lock: Eine Anfrage liest alles in einem Schritt.
    var snapshot: Snapshot { clock.snapshot(view: view) }

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

    /// Wendet eine Änderung der Klasse A an (siehe `AssistantReleaseClock.apply`): Einschränkung sofort mit eigener Generation für
    /// ALLE Sitzungen, Erweiterung erst mit `becomeEffective`.
    func apply(_ change: AssistantReleaseChange) -> Applied { clock.apply(change, view: view) }

    /// Der Stand, den die Zustandsdatei für diesen Teil erhalten soll (siehe `AssistantReleaseClock.recordForSave`).
    func recordForSave(_ step: Step) -> AssistantReleaseRecord { clock.recordForSave(step) }

    /// Die Sicht auf einen Gesamtstand.
    func state(of record: AssistantReleaseRecord) -> AssistantReleaseState { view.state(of: record) }

    /// Eine Erweiterung wird wirksam, im unteilbaren Schritt (Stand setzen, Generation erhöhen, Zeile „wirksam“ einreihen). Nur nach
    /// bestätigtem Austausch der Zustandsdatei aufrufen. `nil`: nicht möglich (blockiert, oder eine Einschränkung hat den Teil entzogen).
    func becomeEffective(_ step: Step) -> Int? { clock.becomeEffective(step, view: view, onIncrease: onIncrease) }

    /// Eine Erweiterung wird in diesem Lauf nicht wirksam: Ihr Schritt verlässt das Register.
    func abandon(_ step: Step) { clock.abandon(step) }

    /// Die Zustandsdatei ist (neu) gesichert: eine gesperrte Freigabe ist damit wieder gültig.
    func clearLocked() { clock.clearLocked() }

    /// Eine Einschränkung ließ sich nicht sichern: Blockade für den Rest des Laufs, im selben unteilbaren Schritt mit eigener
    /// Generation. Gibt die Generation zurück.
    @discardableResult
    func block() -> Int { clock.block(view: view, onIncrease: onIncrease) }

    var isBlocked: Bool { clock.isBlocked }

    /// Klasse B: siehe `AssistantReleaseClock.contentWillChange` (die Ordnung selbst, nicht diese Sicht, erhöht und schreibt die Zeile).
    @discardableResult
    func contentWillChange() -> (generation: Int, epoch: Int) { clock.contentWillChange() }

    @discardableResult
    func contentInstalled(epoch: Int) -> Bool { clock.contentInstalled(epoch: epoch) }

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

    private func runKey(_ id: String) -> String { "\(owner)/\(id)" }

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
    /// Mehr als die Schritte einer Antwort (`AssistantAuditLog.maxSteps`, jede Einheit ist ein Schritt): Bei den festen Grenzen läuft
    /// der Index nie voll. Läuft er trotzdem voll, ist die Antwort zu Ende (`append` gibt `false`, die Einheit geht NICHT hinaus).
    static let maxSteps = 512

    private let lock = NSLock()
    private var steps: [String: [Int]] = [:]
    private var order: [String] = []
    private let limitSteps: Int
    private let limitRequests: Int
    /// Runde 9, S1: Anfragen, die keine Commits mehr annehmen (der Abschluss hat begonnen oder die Anfrage wurde aus dem Gedächtnis der
    /// Sitzung verdrängt und abgeschlossen). Die Zuordnung Ereignis → Schritt bleibt lesbar (`step`), nur `accepts` und `append` sagen
    /// nein. Der Eintrag überlebt das Verdrängen aus `steps` (auch eine Anfrage ohne Ereignisse schließt) und wird NIE verdrängt: keine
    /// Obergrenze, eine Kennung je abgeschlossener Anfrage für die Lebensdauer der Sitzung. Eine geschlossene Anfrage wird damit auf
    /// keinem Weg wieder zulässig (auch nicht durch einen Läufer, der nach dem Abschluss weiter ausgibt).
    private var closed: Set<String> = []

    init(maxSteps: Int = AssistantStepIndex.maxSteps, maxRequests: Int = AssistantStepIndex.maxRequests) {
        limitSteps = maxSteps
        limitRequests = maxRequests
    }

    /// Trägt den Schritt des nächsten Ereignisses ein. `false`, wenn der Index für diese Anfrage voll ist (fail-closed: Der Aufrufer gibt
    /// die Einheit dann nicht aus; ein Ereignis, dessen Commits der Controller nie zuordnen könnte, gibt es nicht). Eine ältere Anfrage,
    /// die wegen der Zahl der Anfragen verdrängt wurde, ist danach unbekannt (`knows` ist `false`): Ihre Commits gelten nicht, und
    /// ihre Einheiten kommen nicht in einen Beitrag (`step` liefert `nil`).
    @discardableResult
    func append(_ id: String, step: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !closed.contains(id) else { return false }
        if steps[id] == nil {
            order.append(id)
            // Eine verdrängte Anfrage ist danach geschlossen (Suchlauf a): Sie nimmt nichts mehr an, auch kein neues Ereignis (sonst
            // finge ihre Zuordnung von vorn an und stimmte nicht mehr mit den Nummern der Ereignisse überein).
            while order.count > limitRequests {
                let old = order.removeFirst()
                steps[old] = nil
                closeLocked(old)
            }
        }
        guard steps[id, default: []].count < limitSteps else { return false }
        steps[id, default: []].append(step)
        return true
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

    /// Diese Anfrage nimmt keine Commits (Anzeige, Audiostart) und keine neuen Ereignisse mehr an. Aufrufen IM Lock der Ordnung
    /// (`AssistantRelease.inSection`), VOR der Schlusszeile und VOR dem Ende des Anspruchs auf ihren Fehler: Ein Commit, der danach den
    /// Lock der Ordnung bekommt, sieht `accepts == false` und löst nichts aus; einer, der davor lief, hat seine Zeilen vor der
    /// Schlusszeile eingereiht. Mehrfach gerufen: ohne weitere Wirkung.
    func close(_ id: String) {
        lock.lock()
        defer { lock.unlock() }
        closeLocked(id)
    }

    private func closeLocked(_ id: String) {
        closed.insert(id)
    }

    /// Die Anfrage ist bekannt UND nimmt noch Commits an (nur dann löst ein Commit etwas aus). Dieselbe Prüfung gilt im Lock der
    /// Ordnung, unmittelbar vor der Aktion.
    func accepts(_ id: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return steps[id] != nil && !closed.contains(id)
    }

    /// Schrittnummer des `index`-ten Ereignisses (ab 0).
    func step(_ id: String, index: Int) -> Int? {
        lock.lock()
        defer { lock.unlock() }
        guard let list = steps[id], list.indices.contains(index) else { return nil }
        return list[index]
    }
}

/// Die Sicherungsfolge einer Änderung der Klasse A (Schreibfolge 8): EIN Ablauf für die Sitzungen und für Änderungen ohne Sitzung
/// (Runde 8, R3/R8). Er liest und ändert nur den einen Stand der Ordnung (über die Sicht `release`), die Zustandsdatei (`store`) und
/// das Protokoll (`audit`); die Zeilen „in diesem Lauf nicht wirksam“, die noch fehlen, stehen im Register der Ordnung und werden von
/// jedem Ablauf nachgeholt, gleich welcher Sitzung sie entstammen.
/// - Einschränkung: Zustandsdatei dauerhaft ersetzen → Zeile `release` → Rückkehr. Scheitert der Austausch: Ausgang offen, Blockade
///   für den Rest des Laufs (eigene Generation), Sicherung wird bei jeder Gelegenheit erneut versucht.
/// - Erweiterung: Zeile „beabsichtigt“ → Zustandsdatei ersetzen (der Commit) → erst danach im Prozess wirksam (Generation steigt im
///   unteilbaren Schritt) → Zeile „wirksam“. Scheitert ein Schritt davor, wird sie in diesem Lauf nicht wirksam.
struct AssistantReleaseSettler: Sendable {
    struct Outcome: Sendable {
        var error: AssistantAuditError?
        var storeError: AssistantStoreError?
        var effective = true
        var generation = 0
    }

    let release: AssistantRelease
    let store: AssistantReleaseStore?
    let audit: AssistantAuditLog?
    /// Art des Modells, wie die Zeilen sie tragen (ohne Sitzung: keine).
    let model: AssistantModelKind?

    var clock: AssistantReleaseClock { release.clock }

    /// Eine Zeile der Klasse A (`release`, `intent`, `effective`, `ineffective`): Lauf kommt vom Token beim Schreiben; Änderungsnummer,
    /// Ursache und Stand (ohne Titel und Inhalte). Die Generation steht nur in Zeilen, in denen sie für genau diese Änderung vergeben
    /// wurde (`release`, `effective`), nie in `intent` und `ineffective`.
    static func line(_ kind: AssistantJournalLine.Kind, change: Int?, generation: Int?, cause: AssistantReleaseCause,
                     state: AssistantReleaseState, cloud: Bool, model: AssistantModelKind?) -> AssistantJournalLine {
        var line = AssistantJournalLine(kind)
        line.at = Date()
        line.gen = generation
        line.chg = change
        line.cause = cause
        line.areas = AssistantRelease.names(state.areas)
        line.unsourced = state.optIn
        line.consent = state.consentVersion
        line.granted = state.cloudConsent
        line.model = model
        line.cloud = cloud
        return line
    }

    /// Die Zeile zu einer Erhöhung, die im Lock der Ordnung geschieht (Erweiterung wirksam, Blockade): wird IM Lock an der Stelle der
    /// Erhöhung gerufen (kurz, nur Einreihen, kein I/O). Einschränkungen schreiben ihre Zeile erst nach der Zustandsdatei. Ohne
    /// Protokoll gibt es keine Zeile.
    static func increaseWriter(audit: AssistantAuditLog?, clock: AssistantReleaseClock, cloud: Bool, model: AssistantModelKind?)
        -> (@Sendable (AssistantRelease.Increase) -> Void)? {
        guard let audit else { return nil }
        let run = clock.run
        return { [weak clock] increase in
            let kind: AssistantJournalLine.Kind = increase.kind == .effective ? .effective : .release
            let line = Self.line(kind, change: increase.change, generation: increase.generation, cause: increase.cause,
                                 state: increase.state, cloud: cloud, model: model)
            // Die Zeile „wirksam“ erwartet der Ablauf der Erweiterung (er wartet unter diesem Schlüssel: Anspruch). Auf die Zeile einer
            // Blockade oder eines Inhalts wartet niemand: Scheitert sie, geht sie ins Register zum Nachholen (R6, R8).
            if increase.kind == .effective {
                audit.enqueue(line, key: increase.key, run: run)
            } else {
                audit.enqueue(line, key: increase.key, run: run, onFailure: { [weak clock] _ in
                    clock?.addPendingLine(AssistantPendingLine(key: increase.key, line: line))
                })
            }
        }
    }

    private func line(_ kind: AssistantJournalLine.Kind, step: AssistantRelease.Step, generation: Int?, state: AssistantReleaseState)
        -> AssistantJournalLine {
        Self.line(kind, change: step.number, generation: generation, cause: step.cause, state: state, cloud: release.view.cloud, model: model)
    }

    /// Sichert die Teile der Änderung der Reihe nach (streng nacheinander mit den Folgen anderer Änderungen, gleich welcher Sitzung)
    /// und gibt den Stand zurück. Jede Folge wartet auf ihre Reihe in der Reihenfolge des Anwendens. Auch eine Änderung ohne Teile
    /// (derselbe Stand) ist eine Gelegenheit: Was früher nicht ging, wird nachgeholt.
    func settle(_ applied: AssistantRelease.Applied) async -> Outcome {
        var out = Outcome()
        out.generation = applied.generation
        if applied.steps.isEmpty { return await catchUp(from: out) }
        for step in applied.steps {
            await step.ticket.turn()
            await settleStep(step, into: &out)
            step.ticket.complete()
        }
        return out
    }

    private func save(_ record: AssistantReleaseRecord) async -> AssistantStoreError? {
        guard let store else { return nil }
        return await store.persist(record)
    }

    private func settleStep(_ step: AssistantRelease.Step, into out: inout Outcome) async {
        await catchUpInSequence(into: &out)
        switch step.kind {
        case .event:
            out.generation = step.generation
            await writeLine(.release, step: step, generation: step.generation, state: step.delta, into: &out)
        case .restriction:
            out.generation = step.generation
            if let error = await save(release.recordForSave(step)) {
                // Der Ausgang des Austauschs ist offen: Die Einschränkung gilt im Prozess, ist aber nicht als gesichert bestätigt.
                out.storeError = error
                release.block()
            } else {
                release.clearLocked()
            }
            await writeLine(.release, step: step, generation: step.generation, state: step.delta, into: &out)
        case .expansion:
            await settleExpansion(step, into: &out)
        }
    }

    private func settleExpansion(_ step: AssistantRelease.Step, into out: inout Outcome) async {
        let intended = release.recordForSave(step)
        let view = release.state(of: intended)
        // (1) „Erweiterung beabsichtigt“, ohne Generation, dauerhaft. Scheitert das, wird die Erweiterung nicht wirksam und nicht gesichert.
        // Eine blockierte Ordnung (Einschränkung nicht gesichert) nimmt keine Erweiterung an; die Sicherung hat `catchUpInSequence` zu
        // Beginn dieser Folge schon erneut versucht (ihr Fehler steht in `out`).
        if release.isBlocked { return await notEffective(step, state: view, into: &out) }
        var wrote = true
        await writeLine(.intent, step: step, generation: nil, state: view, into: &out, ok: &wrote)
        guard wrote else { return await notEffective(step, state: view, into: &out) }
        // (2) Zustandsdatei ersetzen: das ist der Commit der Erweiterung. Scheitert oder bricht er ab, ist der Ausgang offen.
        if let error = await save(intended) {
            out.storeError = error
            return await notEffective(step, state: view, into: &out)
        }
        // (3) Erst jetzt wirksam im Prozess: Stand, Generation und Zeile „wirksam“ in einem unteilbaren Schritt.
        guard let generation = release.becomeEffective(step) else { return await notEffective(step, state: view, into: &out) }
        out.generation = generation
        // (4) Die Zeile „wirksam“ steht dauerhaft; ihr Fehler wird gemeldet, die Erweiterung bleibt wirksam (sie ist gesichert).
        if let error = await audit?.waitForLine(key: step.key) { out.error = out.error ?? error }
    }

    /// Die Erweiterung wird in diesem Lauf nicht wirksam: Die Zeile „in diesem Lauf nicht wirksam“ kommt in das Register der Ordnung
    /// (jeder Ablauf holt sie nach) und wird sofort geschrieben, wenn es geht.
    private func notEffective(_ step: AssistantRelease.Step, state: AssistantReleaseState, into out: inout Outcome) async {
        out.effective = false
        release.abandon(step)
        clock.addPendingLine(AssistantPendingLine(key: step.key, line: line(.ineffective, step: step, generation: nil, state: state)))
        if let error = await flushPending() { out.error = out.error ?? error }
    }

    /// Nachholen bei jeder Gelegenheit: Die nicht gesicherte Einschränkung wird erneut gesichert und die Zeilen „in diesem Lauf nicht
    /// wirksam“, die noch fehlen (aus dem Register der Ordnung), werden geschrieben. Die Fehler stehen im Ergebnis. Nur innerhalb der
    /// Reihe der Folgen aufrufen.
    func catchUpInSequence(into out: inout Outcome) async {
        if let error = await secureOutstandingRestriction() { out.storeError = out.storeError ?? error }
        if let error = await flushPending() { out.error = out.error ?? error }
    }

    /// Sichert eine noch nicht gesicherte Einschränkung erneut. Die Blockade für den Rest des Laufs bleibt in jedem Fall. Gibt den
    /// Fehler des Austauschs zurück.
    private func secureOutstandingRestriction() async -> AssistantStoreError? {
        guard let store, store.needsCatchUp else { return nil }
        let error = await store.persist(clock.record)
        if error == nil { release.clearLocked() }
        return error
    }

    /// Schreibt die Zeilen des Registers der Ordnung, die noch fehlen, als EINE Gruppe (ein fsync, ein Warten) in der Reihenfolge des
    /// Eintrags (Runde 9, S3). Eine Zeile verlässt das Register erst, wenn sie dauerhaft steht; eine, die weiter scheitert, bleibt und
    /// ist ein Fehler im Ergebnis (der der ersten). Was ein anderer Ablauf gerade schreibt, nimmt dieser nicht noch einmal.
    @discardableResult
    func flushPending() async -> AssistantAuditError? {
        guard let audit else { return nil }
        let batch = clock.claimPending()
        guard !batch.isEmpty else { return nil }
        let failed = await audit.writeGroup(batch.map { (line: $0.line, key: $0.key) }, run: clock.run)
        var first: AssistantAuditError?
        for item in batch {
            let error = failed[item.key]
            first = first ?? error
            clock.finishPending(item, written: error == nil)
        }
        return first
    }

    /// Es gibt etwas nachzuholen (billige Vorprüfung ohne Warten).
    var needsCatchUp: Bool { (store?.needsCatchUp ?? false) || clock.hasPending }

    /// Holt nach, was bei einer früheren Gelegenheit nicht ging: eine nicht gesicherte Einschränkung erneut sichern, fehlende Zeilen
    /// schreiben. Läuft bei jeder Anfrage (nach ihrer gesicherten `start`-Zeile) und bei jeder Änderung; streng nacheinander mit den
    /// Folgen der Änderungen (eine Karte der Reihe), und nur, wenn etwas aussteht.
    @discardableResult
    func catchUp(from start: Outcome = Outcome()) async -> Outcome {
        var out = start
        guard needsCatchUp else { return out }
        let ticket = clock.nextTicket()
        await ticket.turn()
        defer { ticket.complete() }
        await catchUpInSequence(into: &out)
        return out
    }

    private func writeLine(_ kind: AssistantJournalLine.Kind, step: AssistantRelease.Step, generation: Int?, state: AssistantReleaseState,
                           into out: inout Outcome) async {
        var ok = true
        await writeLine(kind, step: step, generation: generation, state: state, into: &out, ok: &ok)
    }

    private func writeLine(_ kind: AssistantJournalLine.Kind, step: AssistantRelease.Step, generation: Int?, state: AssistantReleaseState,
                           into out: inout Outcome, ok: inout Bool) async {
        guard let audit else { return }
        audit.enqueue(line(kind, step: step, generation: generation, state: state), key: step.key, run: clock.run)
        if let error = await audit.waitForLine(key: step.key) {
            out.error = out.error ?? error
            ok = false
        }
    }

    /// Versucht die Sicherung des jetzigen Stands erneut (nach einem gescheiterten Austausch der Zustandsdatei). Läuft streng
    /// nacheinander mit den anderen Änderungen. Auch ohne Fehler schreibt es die Datei neu. Die Blockade für den Rest des Laufs
    /// bleibt in jedem Fall bestehen.
    @discardableResult
    func retrySave() async -> AssistantStoreError? {
        let ticket = clock.nextTicket()
        await ticket.turn()
        defer { ticket.complete() }
        let error = await save(clock.record)
        if error == nil { release.clearLocked() }
        return error
    }
}
