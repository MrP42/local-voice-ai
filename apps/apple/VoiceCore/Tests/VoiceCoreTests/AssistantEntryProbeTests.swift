import Foundation
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Runde 8): gemeinsame Hilfe der Tests für Aussagen über den Lock der Ordnung ohne Uhr.
// Liegt in einer Datei mit dem Namensmuster `Assistant*Tests.swift` (Paketziel und Scratch-Paket nehmen sie so mit).

/// Beobachtung am Eingang der Ordnung OHNE Uhr (Runde 8): Eine Änderung ruft die Probe, BEVOR sie den Lock der Ordnung anfordert
/// (`AssistantReleaseClock.setLockProbe`). Die Probe fragt den Lock mit `try`: Hält ihn ein anderer (der Haltepunkt im Start oder
/// Commit), ist die Änderung beim Eintreffen nachweislich ausgesperrt. Das Eintreffen meldet sie als Signal, auf das der
/// Haltepunkt wartet; eine Frist gibt es nur für den Fehlerfall (der Test hängt nicht ewig), nie als Teil der Aussage.
final class EntryProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var heldByOthers: [Bool] = []
    private let arrivedSignal = DispatchSemaphore(value: 0)

    func install(on order: AssistantReleaseClock) {
        order.setLockProbe { [self] in
            let free = order.lock.try()
            if free { order.lock.unlock() }
            lock.withLock { heldByOthers.append(!free) }
            arrivedSignal.signal()
        }
    }

    func remove(from order: AssistantReleaseClock) { order.setLockProbe(nil) }

    /// Wartet auf das Eintreffen der Änderung am Eingang. Gibt `false` nur im Fehlerfall zurück (Frist abgelaufen).
    func waitUntilArrived() -> Bool { arrivedSignal.wait(timeout: .now() + 60) == .success }

    /// Je Eintreffen: war der Lock beim Eintreffen von einem anderen gehalten?
    var observations: [Bool] { lock.withLock { heldByOthers } }
}
