import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Klasse B): die Meldung einer Änderung an den Notizen.
//
// Der Speicher der Notizen (`DurableStore`) gehört auch zur Uhr; der Assistent läuft nur auf dem iPhone. Darum kennt der Speicher
// nur diese kleine Weiche: Die iPhone-App hängt die Meldestelle der Quellen ein (`AssistantSources`), auf der Uhr bleibt sie leer
// und `around` führt die Änderung unverändert aus.
//
// `around` ruft `begin` VOR der Änderung (Generation steigt, Bestand ist `stale`) und danach, auch bei einem Fehler, den
// Abschluss, den `begin` geliefert hat. Kein Warten, kein I/O; der Lock wird nie über die Änderung oder einen Rückruf gehalten.

public final class NoteChangeHook: @unchecked Sendable {
    /// Beginn einer Änderung: liefert den Abschluss, der danach gerufen wird.
    public typealias Begin = @Sendable () -> (@Sendable () -> Void)

    /// Die Weiche des Prozesses (Tests nehmen eine eigene Instanz).
    public static let shared = NoteChangeHook()

    public init() {}

    private let lock = NSLock()
    private var begin: Begin?

    /// Hängt die Meldestelle ein (`nil`: löst sie).
    public func install(_ begin: Begin?) {
        lock.lock()
        self.begin = begin
        lock.unlock()
    }

    /// Führt eine Änderung zwischen Beginn und Abschluss aus.
    @discardableResult
    public func around<T>(_ change: () throws -> T) rethrows -> T {
        lock.lock()
        let begin = self.begin
        lock.unlock()
        let finish = begin?()
        defer { finish?() }
        return try change()
    }
}
