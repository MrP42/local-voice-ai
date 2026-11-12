import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Schreibfolge 7 und 8): die prozessweite Ordnung.
//
// EINE Stelle je Prozesslauf (der Speicher der Zustandsdatei besitzt sie; Tests ohne Speicher nehmen eine eigene) für alles, was
// Generation, Prüfung, Handlung und das Einreihen der Zeile in einer Reihenfolge halten muss. Runde 8 (R3): Generation UND wirksamer
// Stand sind EIN Stand hinter dem einen Lock; eine Sitzung hält nichts davon als Kopie, sie liest nur ihre Sicht (lokal/Cloud, Ziel)
// im selben Lesen wie die Generation (`snapshot(view:)`).
// - EIN Lock (`lock`): Generation, Wiedergabesperre, Register der laufenden Anfragen aller Sitzungen, Beobachter, Änderungszähler,
//   Reihe der Änderungen, der GANZE Stand der Klasse A (Schalter, Einwilligungsversion, Bereiche lokal und Cloud, Zustimmungen je
//   Ziel, die offenen Erweiterungsschritte, gesperrt, blockiert), die Epochen des Bestands (Klasse B: bereit/`stale`) und das
//   Register der Zeilen, die nachzuholen sind, liegen hinter ihm. Er wird nie über I/O, Warten oder einen Rückruf in die Ordnung
//   gehalten (siehe `AssistantRelease`).
// - EINE Generation je Prozesslauf, für Klasse A mit und ohne Sitzung und für Klasse B. Eine Erhöhung widerruft die laufenden
//   Anfragen ALLER Sitzungen; eine neue Sitzung beginnt bei der geltenden Generation und dem geltenden Stand.
// - EIN Lauf-Token (Laufnummer einmal je Prozesslauf, beim ersten Schreiben vergeben) und EINE Standzeile.
// - Änderungsnummern prozessweit eindeutig (fortlaufend je Lauf); daraus ergeben sich auch die Fehlerschlüssel der Zeilen im
//   Schreiber: Sie tragen die Kennung dieser Ordnung, zwei Ordnungen am selben Schreiber verbrauchen einander keine Fehler.
// - EINE Reihe der Sicherungsfolgen (`AssistantChangeTicket`): Änderungen der Klasse A laufen sitzungsunabhängig streng nacheinander.
// - Klasse B (Runde 8, R4): Die Meldestelle der Quellen ruft die Ordnung unmittelbar (`contentWillChange`), auch ohne Gespräch. Jede
//   gemeldete Änderung erhöht für sich und schreibt ihre eigene Zeile „Inhalt“; dazu kennt die Ordnung das Protokoll (`attach`),
//   das der Controller beim Prozessstart einhängt, ohne Modell.
// - Pflichtzeilen, die sich nicht schreiben ließen, stehen im Register der Ordnung (Runde 9, S4: ohne Obergrenze, nichts wird je
//   verdrängt) und werden bei JEDER Gelegenheit nachgeholt (S3): Gelegenheit ist jede dauerhaft gesicherte Zeilengruppe des
//   Schreibers (`attach` meldet den Rückruf an), ohne Sitzung.

/// Was eine Erhöhung nach dem Lock zu tun übrig lässt: Anfragen widerrufen und Beobachter rufen (nie unter dem Lock).
struct AssistantRaiseAftermath: @unchecked Sendable {
    let generation: Int
    let revoke: [@Sendable () -> Void]
    let notify: [@Sendable (Int) -> Void]

    func finish() {
        for r in revoke { r() }
        for n in notify { n(generation) }
    }
}

/// Die Sicht einer Sitzung auf den einen Stand: lokal oder Cloud, und das Cloud-Ziel. Mehr ist die Sitzung für die Freigabe nicht.
/// `released == false` (nur Tests der Freischaltung): Die Sicht zeigt den Schalter „Antworten ohne Quelle“ immer aus; der Stand der
/// Ordnung bleibt davon unberührt (eine solche Sitzung schaltet ihn weder an noch aus).
struct AssistantReleaseView: Sendable, Equatable {
    let cloud: Bool
    let target: String?
    var released = true

    static let local = AssistantReleaseView(cloud: false, target: nil)

    /// Der Stand dieser Sicht aus dem Gesamtstand.
    func state(of record: AssistantReleaseRecord) -> AssistantReleaseState {
        var state = record.state(cloud: cloud, target: target)
        if !released {
            state.optIn = false
            state.consentVersion = 0
        }
        return state
    }
}

/// Eine Zeile „in diesem Lauf nicht wirksam“, die noch nicht geschrieben werden konnte (Register der Ordnung, R8): fertig gebaut,
/// damit jede Sitzung und jede Änderung ohne Sitzung sie nachholen kann.
struct AssistantPendingLine: Sendable, Equatable {
    /// Schlüssel der Zeile im Schreiber (je Lauf und Änderung eindeutig): Unter ihm wird sie erneut geschrieben.
    let key: String
    let line: AssistantJournalLine
}

/// Eine Zeile im Register der Ordnung, wie sie ein Ablauf zum Schreiben bekommt: Nummer im Register (Reihenfolge des Eintrags), Schlüssel
/// im Schreiber und die fertig gebaute Zeile.
struct AssistantPendingClaim: Sendable {
    let sequence: Int
    let key: String
    let line: AssistantJournalLine
}

/// Das Register der Pflichtzeilen, die noch fehlen (Runde 9, S4). Hinter dem Lock der Ordnung. KEINE Obergrenze und kein Verdrängen:
/// Eine Pflichtzeile verlässt das Register nur, wenn sie dauerhaft steht. Die einzige Grenze ist das Prozessende (das Register lebt im
/// Speicher; Schreibfolge 6 und 9: danach schließt das nächste Öffnen die Lücke nicht, sie bleibt als „unbekannt“ lesbar).
/// Klein gehalten durch Schlüsselsuche über Wörterbücher (nie linear) und eine knappe Darstellung: Eine Zeile der Klasse B ist nur
/// (Generation, Zeit) und wird erst beim Nachholen zur Zeile gebaut. Die Reihenfolge des Nachholens ist die Reihenfolge des Eintrags.
struct AssistantPendingRegister {
    private enum Item {
        case built(AssistantPendingLine)
        case content(generation: Int, at: Date)
    }

    private var items: [Int: Item] = [:]
    private var next = 0
    private var builtKeys: [String: Int] = [:]
    private var contentKeys: [Int: Int] = [:]
    /// Nummern, die ein Ablauf gerade schreibt (ein zweiter Ablauf nimmt sie nicht noch einmal).
    private var inFlight: Set<Int> = []

    var isEmpty: Bool { items.isEmpty }
    var count: Int { items.count }

    /// Trägt eine fertige Zeile ein (derselbe Schlüssel nur einmal).
    mutating func add(_ line: AssistantPendingLine) {
        guard builtKeys[line.key] == nil else { return }
        builtKeys[line.key] = next
        items[next] = .built(line)
        next += 1
    }

    /// Trägt eine Zeile der Klasse B (Inhalt) knapp ein (dieselbe Generation nur einmal).
    mutating func addContent(generation: Int, at: Date) {
        guard contentKeys[generation] == nil else { return }
        contentKeys[generation] = next
        items[next] = .content(generation: generation, at: at)
        next += 1
    }

    /// Nimmt alle Einträge, die kein Ablauf in Arbeit hat, in der Reihenfolge des Eintrags und markiert sie als in Arbeit.
    /// `key` und `build` liefern Schlüssel und Zeile der Einträge der Klasse B.
    mutating func claim(key: (Int) -> String, build: (Int, Date) -> AssistantJournalLine) -> [AssistantPendingClaim] {
        var out: [AssistantPendingClaim] = []
        for sequence in items.keys.sorted() where !inFlight.contains(sequence) {
            guard let item = items[sequence] else { continue }
            switch item {
            case .built(let line): out.append(AssistantPendingClaim(sequence: sequence, key: line.key, line: line.line))
            case .content(let generation, let at):
                out.append(AssistantPendingClaim(sequence: sequence, key: key(generation), line: build(generation, at)))
            }
            inFlight.insert(sequence)
        }
        return out
    }

    /// Der Ablauf ist fertig: Steht die Zeile dauerhaft (`written`), verlässt sie das Register; sonst bleibt sie für den nächsten.
    mutating func finish(_ sequence: Int, written: Bool) {
        inFlight.remove(sequence)
        guard written, let item = items.removeValue(forKey: sequence) else { return }
        switch item {
        case .built(let line): builtKeys[line.key] = nil
        case .content(let generation, _): contentKeys[generation] = nil
        }
    }

    /// Alle Einträge in der Reihenfolge des Eintrags als fertige Zeilen (Tests, Anzeige).
    func lines(key: (Int) -> String, build: (Int, Date) -> AssistantJournalLine) -> [AssistantPendingLine] {
        var out: [AssistantPendingLine] = []
        for sequence in items.keys.sorted() {
            guard let item = items[sequence] else { continue }
            switch item {
            case .built(let line): out.append(line)
            case .content(let generation, let at): out.append(AssistantPendingLine(key: key(generation), line: build(generation, at)))
            }
        }
        return out
    }
}

final class AssistantReleaseClock: @unchecked Sendable {
    /// Der eine Lock dieser Dinge (die Wiedergabesperre und der Speicher teilen ihn).
    let lock: NSLock
    let playback: AssistantPlaybackGate
    /// Kennung dieser Ordnung: Teil jedes Fehlerschlüssels.
    let id = String(UUID().uuidString.prefix(8))
    /// Das Lauf-Token dieses Prozesslaufs.
    let run = AssistantRunToken()
    /// Die Standzeile dieses Prozesslaufs.
    let stateLine = AssistantStateLine()

    // Hinter `lock`:
    private var generationValue = 0
    private var changeValue = 0
    private var runs: [String: @Sendable () -> Void] = [:]
    private var observers: [ObjectIdentifier: @Sendable (Int) -> Void] = [:]
    private var tail: AssistantChangeTicket?
    /// Klasse A: der WIRKSAME Stand des ganzen Prozesses (alle Sitzungen und Änderungen ohne Sitzung lesen ihn).
    private var effective: AssistantReleaseRecord
    /// Offene Erweiterungsschritte (Nummer → der noch verbliebene Teil). Eine Einschränkung streicht die entzogenen Teile aus ALLEN
    /// offenen Schritten (im selben Lock); ein Schritt macht nur seinen verbliebenen Teil wirksam.
    private var open: [Int: AssistantReleaseRecord] = [:]
    private var lockedFlag = false
    private var blockedFlag = false
    /// Klasse B: Zahl der Änderungen von Inhalten und die jüngste, für die der Bestand steht.
    private var contentEpoch = 0
    private var corpusEpoch = 0
    /// R8, S4: Pflichtzeilen, die noch fehlen (ohne Obergrenze, siehe `AssistantPendingRegister`).
    private var register = AssistantPendingRegister()
    /// R4: das Protokoll der Ordnung (für die Zeilen der Klasse B).
    private var audit: AssistantAuditLog?
    /// Der Speicher, dem diese Ordnung gehört (schwach; `nil`: Ordnung ohne Speicher). Das Nachholen einer nicht gesicherten
    /// Einschränkung braucht ihn.
    private weak var owner: AssistantReleaseStore?
    /// S3: Zustand des Nachholens bei jeder Gelegenheit. Eigener, kurzer Lock (ein Blatt: nie wird darunter ein anderer genommen, nie
    /// I/O oder Warten; er darf von der Schreibwarteschlange aus genommen werden).
    private let runnerLock = NSLock()
    private var runner = Runner()

    private struct Runner {
        /// Es kann etwas nachzuholen sein (Vorprüfung ohne Ordnungs-Lock): gesetzt, wenn etwas ins Register kommt oder eine Sicherung
        /// scheitert, und am Ende eines Laufs, wenn noch etwas aussteht; gelöscht, bevor ein Lauf prüft.
        var maybe = false
        var running = false
        /// Eine Gelegenheit kam, während ein Lauf lief: Er wird einmal wiederholt.
        var again = false
        /// Nur Tests (`setOpportunitiesPaused`): Gelegenheiten starten keinen Lauf.
        var paused = false
        var idle: [CheckedContinuation<Void, Never>] = []
    }
    /// Nur Tests: wird gerufen, sobald eine Änderung den Lock der Ordnung anfordert (VOR dem Anfordern, ohne ihn zu halten). Hinter
    /// einem eigenen, kurzen Lock, der nie zusammen mit `lock` gehalten wird.
    private let probeLock = NSLock()
    private var probe: (@Sendable () -> Void)?

    init(record: AssistantReleaseRecord = .locked, locked: Bool = false) {
        let shared = NSLock()
        lock = shared
        playback = AssistantPlaybackGate(lock: shared)
        effective = record
        lockedFlag = locked
    }

    // MARK: Generation

    /// Die geltende Generation (nur unter `lock` lesen).
    var generationLocked: Int { generationValue }

    /// Die geltende Generation, in einem eigenen Lesen.
    var generation: Int {
        lock.lock()
        defer { lock.unlock() }
        return generationValue
    }

    /// Hebt die Generation um eins und rückt die Wiedergabesperre vor (Aufrufer hält `lock`).
    func raiseLocked() -> Int {
        generationValue += 1
        playback.discardLocked(below: generationValue)
        return generationValue
    }

    /// Die nächste Änderungsnummer (Aufrufer hält `lock`).
    func nextChangeLocked() -> Int {
        changeValue += 1
        return changeValue
    }

    /// Eine Karte in der Reihe der Sicherungsfolgen (Aufrufer hält `lock`).
    func nextTicketLocked() -> AssistantChangeTicket {
        let ticket = AssistantChangeTicket(previous: tail)
        tail = ticket
        return ticket
    }

    func nextTicket() -> AssistantChangeTicket {
        lock.lock()
        defer { lock.unlock() }
        return nextTicketLocked()
    }

    /// Fehlerschlüssel einer Zeile: Art, Kennung dieser Ordnung, Nummer.
    func key(_ kind: String, _ number: Int) -> String { "\(kind):\(id):\(number)" }

    // MARK: Anfragen und Beobachter

    func registerLocked(_ key: String, revoke: @escaping @Sendable () -> Void) { runs[key] = revoke }
    func unregisterLocked(_ key: String) { runs[key] = nil }
    func registeredCountLocked(prefix: String) -> Int { runs.keys.filter { $0.hasPrefix(prefix) }.count }
    func setObserverLocked(_ owner: ObjectIdentifier, _ observer: (@Sendable (Int) -> Void)?) { observers[owner] = observer }

    /// Nimmt alle angemeldeten Anfragen und Beobachter für die Nachbereitung einer Erhöhung (Aufrufer hält `lock` und ruft danach
    /// `finish` ohne Lock).
    func drainLocked(_ generation: Int) -> AssistantRaiseAftermath {
        let revoke = Array(runs.values)
        runs.removeAll()
        return AssistantRaiseAftermath(generation: generation, revoke: revoke, notify: Array(observers.values))
    }

    // MARK: Klasse A: der eine Stand

    /// Nur beim Aufbau: den Stand setzen (Speicher: aus der Datei; Sitzung ohne Speicher: aus ihren Angaben).
    func seed(record: AssistantReleaseRecord, locked: Bool) {
        lock.lock()
        effective = record
        lockedFlag = locked
        open.removeAll()
        lock.unlock()
    }

    /// Der wirksame Stand des Prozesses (in einem eigenen Lesen).
    var record: AssistantReleaseRecord {
        lock.lock()
        defer { lock.unlock() }
        return effective
    }

    /// Generation, Stand der Sicht und Zustand des Bestands zusammen, aus EINEM Lock (Aufrufer hält ihn): Eine Anfrage liest alles in
    /// einem Schritt und sieht nie eine neue Generation mit einem alten, größeren Stand.
    func snapshotLocked(_ view: AssistantReleaseView) -> AssistantRelease.Snapshot {
        AssistantRelease.Snapshot(generation: generationValue, state: view.state(of: effective),
                                  locked: lockedFlag, blocked: blockedFlag, corpusReady: corpusEpoch == contentEpoch,
                                  contentEpoch: contentEpoch)
    }

    func snapshot(view: AssistantReleaseView) -> AssistantRelease.Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return snapshotLocked(view)
    }

    var isLocked: Bool {
        lock.lock()
        defer { lock.unlock() }
        return lockedFlag
    }

    var isBlocked: Bool {
        lock.lock()
        defer { lock.unlock() }
        return blockedFlag
    }

    /// Wie `clearLocked`, der Aufrufer hält den Lock (die Zustandsdatei im Schritt der Bestätigung).
    func clearLockedLocked() { lockedFlag = false }

    /// Die Zustandsdatei ist (neu) gesichert: eine gesperrte Freigabe ist damit wieder gültig.
    func clearLocked() {
        lock.lock()
        lockedFlag = false
        lock.unlock()
    }

    /// Der beabsichtigte Stand: wirksamer Stand plus alle offenen Erweiterungsschritte (Aufrufer hält den Lock).
    private func targetLocked() -> AssistantReleaseRecord {
        var next = effective
        for number in open.keys.sorted() { next = next.expanded(by: open[number] ?? AssistantReleaseRecord.nothing) }
        return next
    }

    /// Nur Tests: ruft `probe`, sobald eine Änderung den Lock der Ordnung anfordert.
    func setLockProbe(_ probe: (@Sendable () -> Void)?) {
        probeLock.lock()
        self.probe = probe
        probeLock.unlock()
    }

    /// Fordert den Lock für eine Änderung an (Erhöhung, Anwenden, Wirksamwerden, Blockade). Die Probe für Tests läuft davor.
    private func enterForChange() {
        probeLock.lock()
        let current = probe
        probeLock.unlock()
        current?()
        lock.lock()
    }

    /// Wendet eine Änderung der Klasse A aus der Sicht `view` an. Sie wird gegen den beabsichtigten Stand verglichen und zerlegt:
    /// - Einschränkung (etwas fällt weg): wirkt sofort, im selben Lock steigt die Generation und wird der Stand gesetzt (eine
    ///   Änderung, ein Stand). Laufende Antworten ALLER Sitzungen enden.
    /// - Erweiterung (etwas kommt dazu): ändert hier NICHTS und erhält hier KEINE Generation (`becomeEffective`).
    /// - Eine gemischte Änderung ergibt zwei Teile, erst den einschränkenden, dann den erweiternden.
    /// - Ohne Änderung des Stands und ohne `force` geschieht nichts (höchstens die Einwilligungsversion wird nachgeführt).
    func apply(_ change: AssistantReleaseChange, view: AssistantReleaseView) -> AssistantRelease.Applied {
        enterForChange()
        let targetBefore = targetLocked()
        var next = view.state(of: targetBefore)
        if let optIn = change.optIn, view.released {
            next.optIn = optIn.on
            next.consentVersion = optIn.on ? optIn.consentVersion : 0
        }
        if let areas = change.areas { next.areas = areas }
        if let granted = change.cloudConsent { next.cloudConsent = granted }
        // Bei unverändertem Opt-in wird nur die Einwilligungsversion nachgeführt; sie allein ändert die Freigabe nicht.
        if next.optIn == targetBefore.optIn, next.optIn {
            if effective.optIn { effective.consentVersion = next.consentVersion }
            for number in open.keys where open[number]?.optIn == true { open[number]?.consentVersion = next.consentVersion }
        }
        let target = targetLocked()
        var wanted = target.applying(next, cloud: view.cloud, target: view.target, cause: change.cause)
        if !view.released {
            // Eine Sicht ohne Freischaltung lässt den Schalter der Ordnung unberührt.
            wanted.optIn = target.optIn
            wanted.consentVersion = target.consentVersion
        }
        let removes = target.restricted(to: wanted) != target.normalized
        let adds = target.addition(to: wanted)
        if !removes, adds == nil, !change.force {
            let g = generationValue
            lock.unlock()
            return AssistantRelease.Applied(generation: g, changed: false, steps: [])
        }
        let before = view.state(of: target)
        var steps: [AssistantRelease.Step] = []
        var raised = false
        if removes {
            effective = effective.restricted(to: wanted)
            // Die Einschränkung streicht die entzogenen Teile aus allen früher ausgegebenen, noch nicht wirksamen Erweiterungen.
            for (number, remaining) in open { open[number] = remaining.restrictedPending(to: wanted) }
            let number = nextChangeLocked()
            let g = raiseLocked()
            steps.append(AssistantRelease.Step(kind: .restriction, number: number, cause: change.cause, generation: g,
                                               delta: view.state(of: effective),
                                               ticket: nextTicketLocked(), key: key("chg", number)))
            raised = true
        }
        if let adds {
            let number = nextChangeLocked()
            open[number] = adds
            let delta = before.addition(to: view.state(of: wanted)) ?? .none
            steps.append(AssistantRelease.Step(kind: .expansion, number: number, cause: change.cause, generation: 0, delta: delta,
                                               ticket: nextTicketLocked(), key: key("chg", number)))
        }
        if steps.isEmpty {
            let number = nextChangeLocked()
            let g = raiseLocked()
            steps.append(AssistantRelease.Step(kind: .event, number: number, cause: change.cause, generation: g,
                                               delta: view.state(of: effective),
                                               ticket: nextTicketLocked(), key: key("chg", number)))
            raised = true
        }
        open = open.filter { !$0.value.isNothing }
        let g = generationValue
        let after = raised ? drainLocked(g) : nil
        lock.unlock()
        after?.finish()
        return AssistantRelease.Applied(generation: g, changed: true, steps: steps)
    }

    /// Der Stand, den die Zustandsdatei für diesen Teil erhalten soll: bei einer Erweiterung der wirksame Stand mit dem noch
    /// verbliebenen Teil DIESES Schritts (nie mit einem späteren Ziel), sonst der wirksame Stand. Gelesen im Lock, damit eine
    /// zwischenzeitliche Einschränkung in die Datei eingeht (und eine frühere, nicht gesicherte gleich mit).
    func recordForSave(_ step: AssistantRelease.Step) -> AssistantReleaseRecord {
        lock.lock()
        defer { lock.unlock() }
        return step.kind == .expansion ? effective.expanded(by: open[step.number] ?? .nothing) : effective
    }

    /// Eine Erweiterung wird wirksam, im unteilbaren Schritt: Stand setzen, Generation erhöhen, Wiedergabesperre vorrücken, Zeile
    /// „wirksam“ einreihen, alles in EINEM Lock. Nur nach bestätigtem Austausch der Zustandsdatei aufrufen. Gesetzt wird nur der
    /// verbliebene Teil des eigenen Schritts. `nil`: nicht möglich (blockiert, oder eine Einschränkung hat den Teil entzogen).
    func becomeEffective(_ step: AssistantRelease.Step, view: AssistantReleaseView,
                         onIncrease: (@Sendable (AssistantRelease.Increase) -> Void)?) -> Int? {
        enterForChange()
        guard !blockedFlag, let remaining = open[step.number], !remaining.isNothing else {
            lock.unlock()
            return nil
        }
        open[step.number] = nil
        effective = effective.expanded(by: remaining)
        lockedFlag = false
        let g = raiseLocked()
        onIncrease?(AssistantRelease.Increase(kind: .effective, generation: g, change: step.number, cause: step.cause,
                                              state: view.state(of: effective), key: step.key))
        let after = drainLocked(g)
        lock.unlock()
        after.finish()
        return g
    }

    /// Eine Erweiterung wird in diesem Lauf nicht wirksam: Ihr Schritt verlässt das Register, und dieselbe Erweiterung lässt sich
    /// noch einmal anstoßen.
    func abandon(_ step: AssistantRelease.Step) {
        lock.lock()
        open[step.number] = nil
        lock.unlock()
    }

    /// Eine Einschränkung ließ sich nicht sichern: Blockade für den Rest des Laufs, im selben unteilbaren Schritt mit eigener
    /// Generation (der Bestand an Blöcken, der freigegeben ist, ändert sich). Gibt die Generation zurück.
    @discardableResult
    func block(view: AssistantReleaseView, onIncrease: (@Sendable (AssistantRelease.Increase) -> Void)?) -> Int {
        enterForChange()
        if blockedFlag {
            let g = generationValue
            lock.unlock()
            return g
        }
        blockedFlag = true
        let number = nextChangeLocked()
        let g = raiseLocked()
        onIncrease?(AssistantRelease.Increase(kind: .block, generation: g, change: number, cause: .other,
                                              state: view.state(of: effective), key: key("chg", number)))
        let after = drainLocked(g)
        lock.unlock()
        after.finish()
        return g
    }

    /// Ohne Raise: nur die Blockade setzen (Änderung ohne Sitzung: die Einschränkung selbst hat die Generation schon erhöht).
    func markBlocked() {
        lock.lock()
        blockedFlag = true
        lock.unlock()
    }

    // MARK: Klasse B: Bestand

    /// Das Protokoll der Ordnung (R4): die Zeilen der Klasse B (`Inhalt`) entstehen hier, auch ohne Sitzung. Der Controller hängt es
    /// beim Prozessstart ein, eine Sitzung mit Protokoll ebenso.
    func attach(audit: AssistantAuditLog?) {
        guard let audit else { return }
        lock.lock()
        let old = self.audit
        self.audit = audit
        lock.unlock()
        if let old, !old.sharesWriter(with: audit) { old.setDurableObserver(owner: id, nil) }
        // S3: Jede dauerhaft gesicherte Zeilengruppe dieses Schreibers ist eine Gelegenheit zum Nachholen. Der Rückruf hält die
        // Ordnung nur schwach (der Schreiber lebt je Verzeichnis für den ganzen Prozess).
        audit.setDurableObserver(owner: id) { [weak self] in self?.writeOpportunity() }
    }

    /// Der Speicher, dem diese Ordnung gehört, hängt sich ein (einmal, beim Aufbau).
    func adopt(store: AssistantReleaseStore) {
        lock.lock()
        owner = store
        lock.unlock()
    }

    /// Das eingehängte Protokoll.
    var attachedAudit: AssistantAuditLog? {
        lock.lock()
        defer { lock.unlock() }
        return audit
    }

    /// Klasse B: Ein Inhalt oder eine Schutzregel ändert sich (Lesegrenze, KI-Freigabe, Schutz der Ablage, Quelle geändert oder
    /// gelöscht). Synchron und VOR der Änderung im Speicher: Die Generation steigt im selben Lock wie bei Klasse A (gemeinsame
    /// Ordnung), laufende Antworten ALLER Sitzungen enden, der Bestand wird `stale` (keine Ausgabe aus eigenen Inhalten, bis der
    /// neue Bestand steht), die Zeile mit Ursache „Inhalt“ wird in diesem Lock eingereiht (nur Einreihen). JEDE gemeldete Änderung
    /// erhöht für sich und schreibt ihre eigene Zeile. Gibt Generation und Epoche zurück.
    @discardableResult
    func contentWillChange() -> (generation: Int, epoch: Int) {
        enterForChange()
        contentEpoch += 1
        let epoch = contentEpoch
        let g = raiseLocked()
        if let audit {
            // Auf diese Zeile wartet niemand: Scheitert das Schreiben, geht sie ins Register zum Nachholen (R6, R8, S4: knapp als
            // Generation und Zeit, gebaut wird sie beim Nachholen), statt dass ihr Fehler in einer Tabelle des Schreibers steht, aus der
            // er verdrängt werden kann.
            let at = Date()
            audit.enqueue(Self.contentLine(generation: g, at: at), key: key("gen", g), run: run, onFailure: { [weak self] _ in
                self?.addPendingContent(generation: g, at: at)
            })
        }
        let after = drainLocked(g)
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

    /// Der Bestand ist für die jüngste Änderung bereit.
    var isContentReady: Bool {
        lock.lock()
        defer { lock.unlock() }
        return corpusEpoch == contentEpoch
    }

    // MARK: Zeilen, die nachzuholen sind (R8, S3, S4)

    /// Die Zeile „Inhalt“ einer Änderung der Klasse B (so entsteht sie beim Einreihen und beim Nachholen).
    static func contentLine(generation: Int, at: Date) -> AssistantJournalLine {
        var line = AssistantJournalLine(.release)
        line.at = at
        line.gen = generation
        line.cause = .content
        return line
    }

    /// Trägt eine fertige Zeile ein, die sich nicht schreiben ließ (derselbe Schlüssel nur einmal). Kein Verdrängen, keine Obergrenze.
    func addPendingLine(_ item: AssistantPendingLine) {
        lock.withLock { register.add(item) }
        noteObligation()
    }

    /// Trägt eine Zeile der Klasse B knapp ein (Generation und Zeit).
    func addPendingContent(generation: Int, at: Date) {
        lock.withLock { register.addContent(generation: generation, at: at) }
        noteObligation()
    }

    /// Nimmt die Einträge, die kein Ablauf in Arbeit hat, in der Reihenfolge des Eintrags (Eigentum des Ablaufs, bis `finishPending`).
    func claimPending() -> [AssistantPendingClaim] {
        lock.withLock {
            register.claim(key: { key("gen", $0) }, build: { Self.contentLine(generation: $0, at: $1) })
        }
    }

    /// Der Ablauf hat den Eintrag geschrieben (`written`: er steht dauerhaft und verlässt das Register) oder nicht (er bleibt).
    func finishPending(_ item: AssistantPendingClaim, written: Bool) {
        lock.withLock { register.finish(item.sequence, written: written) }
    }

    /// Es fehlt mindestens eine Pflichtzeile.
    var hasPending: Bool { lock.withLock { !register.isEmpty } }

    /// Zahl der fehlenden Pflichtzeilen.
    var pendingCount: Int { lock.withLock { register.count } }

    /// Die fehlenden Zeilen in der Reihenfolge des Eintrags (Tests, Anzeige).
    var pending: [AssistantPendingLine] {
        lock.withLock {
            register.lines(key: { key("gen", $0) }, build: { Self.contentLine(generation: $0, at: $1) })
        }
    }

    // MARK: Nachholen bei jeder Gelegenheit (S3)

    /// Es kann etwas nachzuholen sein: ein Eintrag im Register oder eine Sicherung, die scheiterte (der Speicher meldet sie). Nur die
    /// Vorprüfung für `writeOpportunity`; entschieden wird beim Nachholen selbst.
    func noteObligation() {
        runnerLock.withLock { runner.maybe = true }
    }

    /// Eine Gelegenheit: Die Schreibwarteschlange meldet, dass eine Zeilengruppe dauerhaft steht (nach dem fsync, außerhalb ihrer
    /// Locks). Steht etwas aus, startet EIN gemeinsamer Lauf (höchstens einer zugleich; kommt während des Laufs eine weitere Gelegenheit,
    /// wird er einmal wiederholt). Der Lauf ist nicht an eine Sitzung gebunden.
    /// Sperrordnung: Der Aufruf nimmt nur `runnerLock` (ein Blatt) und startet eine Aufgabe; er wartet nie, schreibt nie und nimmt nie
    /// das Lock der Ordnung, das beim Einreihen der Zeilen gehalten wird (`contentWillChange` reiht darunter ein). Kein Kreis: Ein Lauf,
    /// dessen Schreiben scheitert, meldet nichts und löst keinen weiteren aus; nur ein Erfolg ist eine Gelegenheit, und jeder Lauf
    /// leert das Register um alles, was dauerhaft steht.
    func writeOpportunity() {
        let start = runnerLock.withLock { () -> Bool in
            guard runner.maybe, !runner.paused else { return false }
            if runner.running {
                runner.again = true
                return false
            }
            runner.running = true
            return true
        }
        guard start else { return }
        Task.detached(priority: .utility) { [self] in await self.runCatchUp() }
    }

    private func runCatchUp() async {
        while true {
            // Erst löschen, dann prüfen: Was danach hinzukommt, setzt die Vorprüfung neu.
            runnerLock.withLock {
                runner.maybe = false
                runner.again = false
            }
            await catchUpAtOpportunity()
            let stillNeeded = needsCatchUp
            let waiters = runnerLock.withLock { () -> [CheckedContinuation<Void, Never>]? in
                // Was nach diesem Lauf noch aussteht (ein Schreiben scheiterte), bleibt für die nächste Gelegenheit vorgemerkt, löst aber
                // selbst keinen Lauf aus.
                runner.maybe = runner.maybe || stillNeeded
                if runner.again { return nil }
                runner.running = false
                let all = runner.idle
                runner.idle = []
                return all
            }
            guard let waiters else { continue }
            for waiter in waiters { waiter.resume() }
            return
        }
    }

    /// Ein Lauf: Ablauf der Sicherungsfolge ohne Sitzung (Sicht „lokal“, keine Modellart), streng in der Reihe der Folgen.
    private func catchUpAtOpportunity() async {
        let (audit, store) = lock.withLock { (self.audit, self.owner) }
        guard audit != nil || store != nil else { return }
        let view = AssistantRelease(clock: self, view: .local, onIncrease: nil)
        await AssistantReleaseSettler(release: view, store: store, audit: audit, model: nil).catchUp()
    }

    /// Etwas steht aus (billige Vorprüfung ohne Warten): Zeilen im Register oder eine nicht gesicherte Einschränkung.
    var needsCatchUp: Bool { hasPending || (lock.withLock { owner }?.needsCatchUp ?? false) }

    /// Nur Tests: hält das Nachholen bei Gelegenheiten an (Gelegenheiten, die währenddessen kommen, starten keinen Lauf), damit ein Test
    /// Zwischenzustände einer Fehlerfolge (Temp-Datei, Datei vor der Bestätigung) beobachten kann, ohne dass der Lauf im Hintergrund sie
    /// gleichzeitig abräumt. Die Produktion ruft das nie; ein bereits laufender Lauf wird nicht unterbrochen (`awaitCatchUp` wartet auf ihn).
    func setOpportunitiesPaused(_ paused: Bool) {
        runnerLock.withLock { runner.paused = paused }
    }

    /// Kehrt zurück, wenn kein Lauf des Nachholens mehr läuft (Tests, Beenden): Haltepunkt statt einer Zeitannahme.
    func awaitCatchUp() async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            let idle = runnerLock.withLock { () -> Bool in
                guard runner.running else { return true }
                runner.idle.append(c)
                return false
            }
            if idle { c.resume() }
        }
    }
}
