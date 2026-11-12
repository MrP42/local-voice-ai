import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Schreibfolge 8, Klasse B): die Meldestelle der Quellen.
// Jede Änderung an dem, was in den Bestand des Assistenten fließt (Lesegrenze eines Buchs, KI-Freigabe einer Besprechung, Schutz der
// Ablage, Änderung oder Löschen einer Quelle, auch eingespielt oder per Abgleich), meldet sich hier VOR der Änderung im Speicher der
// Quelle (`willChange`) und danach (`changed`). Die Sitzung hört dabei nicht zu: Sie hängt sich hier an (`attach`).
//
// - `willChange` (synchron, ohne Warten, ohne I/O): vergibt eine Nummer und ruft `AssistantSession.contentWillChange`: die Generation
//   steigt im selben Lock wie bei Klasse A, laufende Antworten enden, der Bestand ist `stale`.
// - `changed`: die Änderung steht im Speicher der Quelle. Erst danach darf ein Durchgang der Indexierung sie sehen.
// - Ein Durchgang der Indexierung nimmt bei seinem Beginn die Marke (`mark`: alle bis dahin gemeldeten Änderungen sind beendet) und
//   meldet beim regulären Ende (`passCompleted`), dass der Bestand für alle Änderungen bis zur Marke steht. Ein abgebrochener
//   Durchgang meldet nichts. Reißt eine Änderung ab, ohne `changed` zu melden, bleibt der Bestand `stale` (fail-closed); die
//   Hilfe `around` meldet das Ende auch bei einem Fehler.
// - Binden sich die Ordnung und die Meldestelle erst, nachdem sich schon etwas geändert hat (es gab noch keine Ordnung, also lief auch
//   keine Anfrage), ist der Bestand mit einer einzigen Erhöhung `stale`, bis ein Durchgang ihn meldet.
// - Wer meldet: Jede Schreibstelle der Quellen ruft `around` (Bücher: PhoneLibrary.importText und LibrarySync.place, Ablage:
//   AblageSpeicher und AblageSync). Der Speicher der Notizen (`DurableStore`, auch auf der Uhr) kennt die Meldestelle nicht, sondern
//   die Weiche `NoteChangeHook` (Löschen, Transkript, Antwort, eingehende Antwort); die Quelle der iPhone-Seite hängt die Meldestelle
//   dort ein. Zusätzlich hören die Quellen der iPhone-Seite auf die @Published-Sichten (`willSet`, Netz zweiter Linie, siehe
//   `AssistantSources.observeStores`); `VoiceModel.deleteNote` hält die äußere Klammer offen, bis auch die Sicht den neuen Stand trägt.
//
// Sperrordnung (außen nach innen): Meldestelle → Sperre der Freigabe → Zeilenschlange des Protokolls (Blatt). Die Freigabe ruft nie
// hierher zurück, und unter der Sperre der Freigabe wird nie auf die Meldestelle gewartet.

public final class AssistantContentWatch: @unchecked Sendable {
    /// Eine Änderung einer Quelle: ihre Nummer, vergeben VOR der Änderung.
    public struct Ticket: Sendable, Hashable {
        public let number: Int
    }

    /// Die Meldestelle des Prozesses. Tests nehmen eine eigene Instanz.
    public static let shared = AssistantContentWatch()

    public init() {}

    private let lock = NSLock()
    // Hinter `lock`:
    private var issued = 0
    /// Beendete Nummern oberhalb der lückenlosen Marke.
    private var finished: Set<Int> = []
    /// Alle Nummern bis hierher sind beendet.
    private var contiguous = 0
    /// Höchste Marke, für die ein vollständiger Durchgang der Indexierung beendet ist.
    private var installedThrough = 0
    /// Die prozessweite Ordnung (R4): ruft jede Änderung unmittelbar, gleich ob ein Gespräch offen ist.
    private var order: AssistantReleaseClock?
    /// Nummer → Epoche in der Ordnung (nur Nummern oberhalb von `installedThrough`).
    private var epochs: [Int: Int] = [:]
    private var observer: (@Sendable () -> Void)?

    /// Zahl der vergebenen Nummern (Tests, Anzeige).
    public var issuedCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return issued
    }

    /// Alle bis hierher gemeldeten Änderungen sind beendet: ein Durchgang, der jetzt beginnt, sieht sie.
    public var mark: Int {
        lock.lock()
        defer { lock.unlock() }
        return contiguous
    }

    /// Der Bestand der Indexierung entspricht (noch) nicht allen gemeldeten Änderungen.
    public var isStale: Bool {
        lock.lock()
        defer { lock.unlock() }
        return issued > installedThrough
    }

    /// Eine Änderung beginnt. Synchron aufrufen, VOR der Änderung im Speicher der Quelle, auf jedem Thread.
    public func willChange() -> Ticket {
        lock.lock()
        defer { lock.unlock() }
        issued += 1
        if let order { epochs[issued] = order.contentWillChange().epoch }
        return Ticket(number: issued)
    }

    /// Die Änderung steht im Speicher der Quelle (auch wenn sie misslang: dann hat sich nichts geändert, und der nächste Durchgang
    /// bestätigt den unveränderten Stand). Ruft den Beobachter (er stößt die Indexierung an) außerhalb der Sperre.
    public func changed(_ ticket: Ticket) {
        lock.lock()
        finished.insert(ticket.number)
        while finished.remove(contiguous + 1) != nil { contiguous += 1 }
        let notify = observer
        lock.unlock()
        notify?()
    }

    /// Führt eine Änderung zwischen `willChange` und `changed` aus; das Ende wird auch bei einem Fehler gemeldet.
    @discardableResult
    public func around<T>(_ change: () throws -> T) rethrows -> T {
        let ticket = willChange()
        defer { changed(ticket) }
        return try change()
    }

    /// Wie `around` für eine Änderung mit Wartepunkten (eigener Name, damit ein Aufruf in `async`-Kontext nicht mehrdeutig wird).
    @discardableResult
    public func aroundAsync<T>(_ change: () async throws -> T) async rethrows -> T {
        let ticket = willChange()
        defer { changed(ticket) }
        return try await change()
    }

    /// Ein Durchgang der Indexierung, der bei `mark` begann, ist regulär zu Ende. `true`, wenn der Bestand der Ordnung jetzt für alle
    /// gemeldeten Änderungen steht (oder keine Ordnung gebunden ist); `false`, wenn eine neuere Änderung dazugekommen ist.
    @discardableResult
    public func passCompleted(mark: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        installedThrough = max(installedThrough, min(mark, contiguous))
        let covered = epochs.filter { $0.key <= installedThrough }
        epochs = epochs.filter { $0.key > installedThrough }
        guard let order else { return issued <= installedThrough }
        guard let epoch = covered.max(by: { $0.key < $1.key })?.value else { return order.isContentReady }
        return order.contentInstalled(epoch: epoch)
    }

    /// Bindet die Ordnung des Prozesses (einmal, beim Prozessstart; dieselbe Ordnung erneut zu binden ändert nichts). Entspricht der
    /// Bestand nicht dem Stand der Quellen, ist er mit EINER Erhöhung `stale`, bis ein Durchgang ihn meldet.
    func bind(_ clock: AssistantReleaseClock?) {
        lock.lock()
        defer { lock.unlock() }
        guard order !== clock else { return }
        order = clock
        epochs.removeAll()
        guard let clock, issued > installedThrough else { return }
        epochs[issued] = clock.contentWillChange().epoch
    }

    /// Bindet die Ordnung der Sitzung des Gesprächs (Abkürzung für Tests und alte Aufrufer). `nil` löst nichts: Die Ordnung des
    /// Prozesses bleibt gebunden, auch wenn das Gespräch zu Ende ist (sonst blieben Änderungen ohne Gespräch ohne Wirkung).
    public func attach(_ session: AssistantSession?) {
        guard let session else { return }
        bind(session.order)
    }

    /// Der Beobachter wird nach jeder beendeten Änderung gerufen (außerhalb der Sperre).
    public func setObserver(_ observer: (@Sendable () -> Void)?) {
        lock.lock()
        self.observer = observer
        lock.unlock()
    }
}
