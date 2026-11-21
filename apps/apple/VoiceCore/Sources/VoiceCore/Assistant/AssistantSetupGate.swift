import Foundation

// Assistenten (lva#556, lva#565): Die Sitzung des Gesprächs trägt Rolle, Wissensbereiche und Stil unveränderlich. Wechselt der Nutzer den
// Assistenten (oder ändert dessen Vorgaben oder den Stil), muss die Sitzung neu gebaut werden. Bis das geschehen ist, darf KEINE Anfrage an
// das Modell gehen: Sie liefe mit Rolle und Wissensbereichen des alten Assistenten. Diese Datei ist die reine, testbare Hälfte davon
// (ohne Modell, ohne Uhr): die Schranke (`AssistantSetupGate`), die Eingaben zurückhält und den Tausch betreibt, und die Entscheidung des
// Tauschversuchs (`AssistantSwapPlan`). Der Aufbau der Sitzung selbst bleibt in der Steuerung (iPhone), die beides einhängt.

/// Eine zurückgehaltene Eingabe (gesprochene Äußerung oder getippter Text), die nach dem Tausch gesendet wird.
public struct AssistantHeldRequest: Equatable, Sendable {
    public var text: String
    /// Die Anfrage folgt auf eine unterbrochene Antwort (Hinweissatz im Prompt).
    public var afterInterruption: Bool
    /// Ende der Äußerung in ms seit Gesprächsbeginn (Latenzmessung; getippt: der Moment des Absendens). Das Warten auf den Tausch
    /// zählt mit: Es ist die Zeit, die der Nutzer tatsächlich wartet.
    public var endedAtMs: Int

    public init(text: String, afterInterruption: Bool, endedAtMs: Int) {
        self.text = text
        self.afterInterruption = afterInterruption
        self.endedAtMs = endedAtMs
    }

    /// Mehrere Eingaben, die in derselben Wartezeit eintrafen, werden EINE Anfrage (der Reihe nach, durch Leerzeichen getrennt): Keine geht
    /// verloren, und es entsteht nicht für jede ein Modellaufruf, dessen Antwort die nächste sofort wieder abbräche. Das Ende der letzten
    /// Äußerung zählt.
    public static func combined(_ list: [AssistantHeldRequest]) -> AssistantHeldRequest? {
        guard let last = list.last else { return nil }
        let texts = list.map { $0.text.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        return AssistantHeldRequest(text: texts.joined(separator: " "), afterInterruption: list.contains { $0.afterInterruption },
                                    endedAtMs: last.endedAtMs)
    }
}

/// Ergebnis der Annahme einer Eingabe; ein Überlauf muss der Person angezeigt werden.
public enum AssistantSubmissionOutcome: Sendable, Equatable {
    case delivered
    case held
    case rejectedOverflow
}

/// Ergebnis eines Tauschversuchs.
public enum AssistantSwapOutcome: Sendable, Equatable {
    /// Die Sitzung passt (neu gebaut oder es gab nichts zu tauschen).
    case installed
    /// Der Zustand hat sich beim Warten verschoben (neuer Auftrag in der Kette, Sitzung ersetzt): sofort neu ansetzen.
    case retryNow
    /// Eine Anfrage oder eine Zustimmungsfrage ist offen: Der Tausch wartet auf deren Ende und wird dann angestoßen (`kick()`).
    case busy
    /// Es gibt nichts, womit sich tauschen ließe (kein Modell): wartet auf den nächsten Anstoß; zurückgehaltene Eingaben bleiben liegen.
    case unavailable
}

/// Die Entscheidung am Ende eines Tauschversuchs, nach allen Wartepunkten (Kette, Wissen, Verlauf der alten Sitzung). Zwischen dieser
/// Entscheidung und dem Einsetzen der neuen Sitzung gibt es keinen Wartepunkt mehr.
public enum AssistantSwapPlan {
    public struct Snapshot: Equatable, Sendable {
        /// Dieselbe Sitzung wie zu Beginn des Versuchs (nicht durch einen Modellwechsel ersetzt).
        public var sessionUnchanged: Bool
        /// Seit Beginn des Versuchs ist nichts Neues in die Kette gekommen.
        public var chainUnchanged: Bool
        /// „Neues Gespräch“ ist angefordert UND erledigt (sonst käme der alte Verlauf in die neue Sitzung).
        public var clearSettled: Bool
        /// Eine Vordergrund-Antwort läuft.
        public var answerRunning: Bool
        /// Ein Suchauftrag läuft im Hintergrund.
        public var jobRunning: Bool
        /// Eine Zustimmungsfrage ist offen (sie bindet die alte Sitzung).
        public var consentOpen: Bool

        public init(sessionUnchanged: Bool, chainUnchanged: Bool, clearSettled: Bool, answerRunning: Bool, jobRunning: Bool,
                    consentOpen: Bool) {
            self.sessionUnchanged = sessionUnchanged
            self.chainUnchanged = chainUnchanged
            self.clearSettled = clearSettled
            self.answerRunning = answerRunning
            self.jobRunning = jobRunning
            self.consentOpen = consentOpen
        }
    }

    public enum Decision: Sendable, Equatable { case install, retryNow, busy }

    /// Erst „hat sich der Zustand beim Warten verschoben“ (dann gilt, was wir gelesen haben, nicht mehr: neu ansetzen), dann „läuft etwas“
    /// (wartet auf dessen Ende), sonst einsetzen. Nirgends „später irgendwann“: Jeder Ausgang ist entweder das Einsetzen, ein sofortiger
    /// neuer Versuch oder ein Warten auf ein bestimmtes Ereignis.
    public static func decide(_ s: Snapshot) -> Decision {
        if !s.sessionUnchanged || !s.chainUnchanged || !s.clearSettled { return .retryNow }
        if s.answerRunning || s.jobRunning || s.consentOpen { return .busy }
        return .install
    }
}

/// Die Schranke vor der Sitzung. Solange die Sitzung nicht zum gewählten Assistenten (Stil, Vorgaben) passt (`isStale`), laufen neue
/// Anfragen nicht, sondern werden zurückgehalten und nach dem Tausch gemeinsam gesendet (`deliver`). Der Tausch (`swap`) wird von der
/// Schranke betrieben, bis er gelungen ist: Er hat keine Versuchsgrenze, sondern wartet bei `busy` auf den nächsten Anstoß (`kick()`),
/// den die Steuerung am Ende jeder Antwort, jedes Suchauftrags und jeder Zustimmungsfrage gibt.
///
/// Zusagen (Tests: `AssistantSetupGateTests`):
/// - `deliver` läuft nur, wenn `isStale()` unmittelbar davor falsch war, und ohne Wartepunkt dazwischen.
/// - Eine Eingabe, die ankommt, solange ein Tausch läuft oder etwas zurückgehalten ist, wird zurückgehalten (Reihenfolge bleibt).
/// - Ein Anstoß, der während eines Versuchs kommt, geht nicht verloren (`rerun`).
/// - Ein Tausch, der nicht gelingt, dreht sich nicht im Kreis: Er wartet auf den nächsten Anstoß.
/// - Angenommene Eingaben bleiben bis zum Senden oder ausdrücklichen Verwerfen (`dropHeld`).
/// - Überlauf wird vor der Annahme ausdrücklich gemeldet; bereits gehaltene Eingaben bleiben intakt.
@MainActor
public final class AssistantSetupGate<Input: Sendable> {
    private let isStale: @MainActor () -> Bool
    private let swap: @MainActor () async -> AssistantSwapOutcome
    private let deliver: @MainActor ([Input]) -> Void
    private let settled: @MainActor () -> Void
    private let textByteCount: @MainActor (Input) -> Int
    private let onOverflow: @MainActor (Input) -> Void
    private var held: [Input] = []
    /// Rohtext in UTF-8-Bytes, einschließlich Leerraum. Beim Kombinieren kommen höchstens
    /// sieben Leerzeichen hinzu.
    public private(set) var heldByteCount = 0
    /// Acht Äußerungen bzw. 16 KiB Rohtext begrenzen Speicher und wiederholtes Kombinieren
    /// während des Tauschs.
    public static var maxHeldCount: Int { 8 }
    public static var maxHeldBytes: Int { 16_384 }
    private var swapTask: Task<Void, Never>?
    private var rerun = false
    /// Der letzte Versuch endete mit `busy`/`unavailable`: Es wartet auf `kick()` (Anzeige und Tests).
    public private(set) var isParked = false
    /// Wie oft in Folge ein „gelungener“ Tausch die Sitzung trotzdem als überholt hinterließ (Schutz vor einer Endlosschleife).
    private static var maxInstalledButStale: Int { 3 }

    /// - `isStale`: Die Sitzung passt nicht zum gewünschten Zustand (und es gibt eine Sitzung). Wird synchron und oft gefragt.
    /// - `swap`: Ein Versuch, die Sitzung zu tauschen. Liest den gewünschten Zustand NACH seinem letzten Wartepunkt.
    /// - `deliver`: Sendet die zurückgehaltenen Eingaben (in Ankunftsreihenfolge) an die jetzt passende Sitzung.
    /// - `settled`: Nach dem Tausch und dem Senden (z. B. Begrüßung des neuen Assistenten).
    /// - `textByteCount`: UTF-8-Bytes des Rohtexts. Andere Eingabetypen als String/AssistantHeldRequest
    ///   benötigen diese Messung; ohne sie werden sie beim Zurückhalten ausdrücklich abgelehnt.
    /// - `onOverflow`: Synchrone Meldung für die Anzeige einer nicht angenommenen Äußerung. Alternativ
    ///   wertet die Steuerung `rejectedOverflow` aus; der Callback darf keine Inhalte protokollieren.
    public init(isStale: @escaping @MainActor () -> Bool, swap: @escaping @MainActor () async -> AssistantSwapOutcome,
                textByteCount: @escaping @MainActor (Input) -> Int = { input in
                    if let request = input as? AssistantHeldRequest { return request.text.utf8.count }
                    if let text = input as? String { return text.utf8.count }
                    return Int.max
                },
                onOverflow: @escaping @MainActor (Input) -> Void = { _ in },
                deliver: @escaping @MainActor ([Input]) -> Void, settled: @escaping @MainActor () -> Void = {}) {
        self.isStale = isStale
        self.swap = swap
        self.deliver = deliver
        self.settled = settled
        self.textByteCount = textByteCount
        self.onOverflow = onOverflow
    }

    /// Neue Anfragen müssen warten: Die Sitzung ist überholt, ein Tausch läuft, oder es liegt schon etwas vor ihnen (Reihenfolge).
    public var isHolding: Bool { swapTask != nil || !held.isEmpty || isStale() }
    public var heldCount: Int { held.count }
    public var heldInputs: [Input] { held }

    /// Sendet sofort, wenn nichts im Weg ist, sonst zurückhalten (und den Tausch anstoßen).
    @discardableResult
    public func submit(_ input: Input) -> AssistantSubmissionOutcome {
        if isHolding { return hold(input) }
        deliver([input])
        return .delivered
    }

    /// Hält die Eingabe zurück und stößt den Tausch an (auch ohne Anlass: ein `kick()` ohne Überholtes sendet nur).
    @discardableResult
    public func hold(_ input: Input) -> AssistantSubmissionOutcome {
        let bytes = textByteCount(input)
        guard held.count < Self.maxHeldCount, bytes >= 0,
              bytes <= Self.maxHeldBytes - heldByteCount else {
            onOverflow(input)
            return .rejectedOverflow
        }
        held.append(input)
        heldByteCount += bytes
        kick()
        return .held
    }

    /// Anstoß: Es hat sich etwas geändert, das den Tausch möglich macht oder nötig (Wahl des Assistenten, Ende einer Antwort,
    /// Zustimmung beantwortet, Modell übernommen). Läuft schon ein Versuch, wird er nach seinem Ende noch einmal angesetzt. Sendet nie
    /// synchron: Der Aufrufer steckt mitten in seinem eigenen Zustandswechsel (`current = nil`), das Senden folgt im eigenen Zug.
    public func kick() {
        if swapTask != nil {
            rerun = true
            return
        }
        guard isStale() || !held.isEmpty else { return }
        isParked = false
        swapTask = Task { @MainActor [weak self] in await self?.run() }
    }

    /// Verwirft, was zurückgehalten ist, auf ausdrücklichen Wunsch des Nutzers (Stopp, neues Gespräch, Ende). Der Tausch läuft weiter.
    @discardableResult
    public func dropHeld() -> [Input] {
        let dropped = held
        held = []
        heldByteCount = 0
        return dropped
    }

    /// Wartet, bis kein Tausch mehr läuft (er ist gelungen oder wartet auf einen Anstoß). Für Tests und für Aufrufer, die auf das Ende
    /// eines angestoßenen Tauschs warten, ohne zu raten, wie lange es dauert.
    public func idle() async {
        while let task = swapTask { await task.value }
    }

    private func run() async {
        var installedButStale = 0
        while true {
            rerun = false
            guard isStale() else { break }
            switch await swap() {
            case .installed:
                if isStale() {
                    installedButStale += 1
                    if installedButStale >= Self.maxInstalledButStale { return park() }
                }
            case .retryNow:
                await Task.yield()
            case .busy:
                // Ein Anstoß kam, während der Versuch entschied: noch einmal ansehen (sonst ginge das Aufwecken verloren).
                if !rerun { return park() }
                await Task.yield()
            case .unavailable:
                return park()
            }
        }
        // Ab hier ohne Wartepunkt: Was `isStale()` eben als passend bestätigt hat, gilt beim Senden noch.
        let batch = held
        held = []
        heldByteCount = 0
        swapTask = nil
        isParked = false
        if !batch.isEmpty { deliver(batch) }
        settled()
        // Etwas kam während des Sendens zurückgehalten dazu (Wiedereintritt aus `deliver`): nicht liegen lassen.
        if !held.isEmpty || isStale() { kick() }
    }

    private func park() {
        swapTask = nil
        isParked = true
    }
}
