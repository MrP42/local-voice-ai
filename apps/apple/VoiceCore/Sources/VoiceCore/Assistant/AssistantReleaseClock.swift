import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Schreibfolge 7 und 8): die prozessweite Ordnung.
//
// EINE Stelle je Prozesslauf (der Speicher der Zustandsdatei besitzt sie; Tests ohne Speicher nehmen eine eigene) für alles, was
// Generation, Prüfung, Handlung und das Einreihen der Zeile in einer Reihenfolge halten muss:
// - EIN Lock (`lock`): Generation, Wiedergabesperre, Register der laufenden Anfragen aller Sitzungen, Beobachter, Änderungszähler,
//   Reihe der Änderungen und der Stand der Zustandsdatei im Speicher (`AssistantReleaseStore`) liegen hinter ihm. Er wird nie
//   über I/O, Warten oder einen Rückruf in die Ordnung gehalten (siehe `AssistantRelease`).
// - EINE Generation je Prozesslauf, für Klasse A mit und ohne Sitzung und für Klasse B. Eine Erhöhung widerruft die laufenden
//   Anfragen ALLER Sitzungen; eine neue Sitzung beginnt bei der geltenden Generation.
// - EIN Lauf-Token (Laufnummer einmal je Prozesslauf, beim ersten Schreiben vergeben) und EINE Standzeile.
// - Änderungsnummern prozessweit eindeutig (fortlaufend je Lauf); daraus ergeben sich auch die Fehlerschlüssel der Zeilen im
//   Schreiber: Sie tragen die Kennung dieser Ordnung, zwei Ordnungen am selben Schreiber verbrauchen einander keine Fehler.
// - EINE Reihe der Sicherungsfolgen (`AssistantChangeTicket`): Änderungen der Klasse A laufen sitzungsunabhängig streng nacheinander.
// Die Sitzungen lesen nur ihre Sicht (lokal/Cloud, Ziel) und halten sonst keinen eigenen Zustand dieser Dinge.

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

    init() {
        let shared = NSLock()
        lock = shared
        playback = AssistantPlaybackGate(lock: shared)
    }

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
}
