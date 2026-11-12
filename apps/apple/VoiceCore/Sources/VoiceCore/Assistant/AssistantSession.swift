import Foundation

// Gesprächsmodus „Sprechen“ (Paket K, erweitert in Paket A um „Antworten ohne Quelle“, E4.2a, und um Quellblöcke, E4.2b):
// Ablauf einer Antwort.
// Protokollzeilen `state` (beim Anlegen der Sitzung) und `start` (vor dem Abruf) → Abruf (mit den Bereichen aus EINEM Lesen von
// Generation und Freigabezustand) → Rundenart (`AssistantContext`, Zeile `round`) → Zeile `areas`, Ereignis `sources` →
// Modellstrom → Satztrenner (nur ganze Sätze, Reste ohne Satzzeichen werden verworfen) → Prüfung (`AssistantGrounding.judge`) →
// `sentence`-Ereignisse → `done`. Muster: `BookTalkSession` (Strom, Abbruch, Wächter). Mehrere Anfragen dürfen gleichzeitig
// laufen (Vorarbeit und Abbruch); keine schreibt in den Verlauf, das macht allein `commit`.
//
// Zwei Kanäle je Runde:
// - DATENRUNDE (Standard, mit und ohne Opt-in): Das Modell wählt, das System gibt aus. Je vollständigem Modellsatz rastet der
//   beste QUELLBLOCK der Auszüge ein und wird GANZ und unverändert ausgegeben, davor jedes Mal die Ansage „Wörtlicher Auszug
//   aus ‚Titel‘:“ (höchstens drei Blöcke, keine Doppelten, danach bricht das Modell ab). Rastet nichts ein, kommt ein
//   deterministischer Rückfall gegen die Frage oder der feste Satz. Bestandsfragen beantwortet das System aus den
//   Bestandsdaten, ohne Modell.
// - ALLGEMEINRUNDE (nur mit Opt-in und nur im Zustand `.free`, Prompt ohne Auszüge und Bestand): Vorlage → neutral,
//   Eigendatenbezug oder Ähnlichkeit mit eigenem Inhalt → verworfen, sonst „ohne Quelle“ mit Ansage.
//
// Schreibfolge (ADR-0028 E4.2b), Pflicht und fail-closed: `state` (beim Öffnen), `start`, `round` und `areas` stehen dauerhaft
// in der Datei, bevor das Modell läuft oder etwas ausgegeben wird; ein Fehler ergibt Ausgang „Fehler“ (`.auditUnavailable`),
// kein Modell, keine Ausgabe. Die Einordnung (`step`) JEDER Einheit steht dauerhaft in der Datei, bevor der erste Commit der
// Einheit geprüft wird. Die Ergebniszeile eines Commits wird im selben Lock eingereiht wie Prüfung und Aktion. Ein Schreibfehler
// stoppt die Ausgabe: keine weitere Einheit, kein `done`.
//
// Widerruf: Jede Änderung von Opt-in, Bereichen, Cloud-Zustimmung oder des Zustands (`resumeFree`) erhöht die Freigabe-
// Generation GENAU EINMAL, bricht alle angemeldeten Läufe ab (Ausgang „widerrufen“) und ruft den Beobachter (Controller:
// Wiedergabe stoppen, Warteschlange leeren). Der wirksame Stand (Bereiche, Opt-in, Cloud-Zustimmung) wird im selben Lock gesetzt
// (`AssistantRelease.apply`). Ein Lauf bleibt bis `commit` angemeldet. Jedes Ereignis trägt die Generation seines Laufs und wird
// unmittelbar vor dem Senden gegen sie geprüft; der Verlaufseintrag entsteht im selben Lock (`AssistantRelease`).
//
// Lebenslauf einer Anfrage (Runde 10): offen → geschlossen → abgeschlossen (`AssistantStepIndex`). Der Besitz des Abschlusses wird im
// Abschnitt der Ordnung vergeben und schließt zugleich; die Verdrängung schließt auch unter dem Lock des Index allein (kein
// Gegenbeispiel, siehe dort: jede Offen-Prüfung liest den Index-Lock und handelt im selben Abschnitt). Der Abschluss (`commit`) gehört
// genau EINEM Aufrufer (L1). Ab dem Schließen sendet der Erzeuger nichts mehr und reiht keine Zeile mehr ein, jeder seiner Abschnitte
// fragt „offen?“ (L2: `AssistantRequestGate`, `RunEmitter.enqueue`); nur die Zeilen des Abschlusses selbst kommen noch. Die Schlusszeile
// legt ihren Ausgang erst fest, wenn die Schreibergebnisse der Zeilen davor bekannt sind, und er ist der, den die Steuerung erfährt
// (L3: `finishRequest`; auch der Abbruch vor dem Start läuft durch denselben Abschluss).
// Runde 11: Der Abschluss bestimmt den Ausgang aus den Fehlerquellen selbst, zum Zeitpunkt der Entscheidung: Anspruch der Anfrage,
// Standzeile (er sichert sie selbst, vor `start`), Stand des Laufs frisch gelesen. Eine Kennung hat im Protokoll genau EINEN Eintrag, auch
// wenn ihre Startzeile mehrfach in der Datei steht (Doppel statt Verlust; der Leser faltet, das Öffnen schließt je Kennung einmal).

/// Ein ausgegebener Satz (ohne Ansagen) mit Einordnung und Quellen, für `commit`.
struct AssistantEmitted: Sendable, Equatable {
    let text: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
    let quote: Bool
    /// Schrittnummer der Einheit im Protokoll (0: unbekannt, nur Tests).
    var step = 0

    /// Die Einheit trägt im Verlauf ihre vollständige Marke: ein Zitat mit seiner Quelle, sonst Kennzeichnung oder „ohne Quelle“.
    var isRepresentable: Bool {
        AssistantTurnPart(text: text, unsourced: status == .unsourced, quote: quote,
                          source: quote ? sources.first?.excerptLine : nil,
                          note: AssistantTurnPart.note(for: status, quote: quote)).isMarked
    }
}

/// Was eine Anfrage hervorgebracht hat; `commit` macht daraus den Beitrag und die Schlusszeilen des Protokolls. Die Satzspur
/// steht nicht hier, sondern Zeile für Zeile im Journal.
struct AssistantRunRecord: Sendable {
    var round: AssistantRoundKind = .data
    /// Die Rundenart wurde festgelegt (sonst endete die Anfrage davor).
    var decided = false
    /// Freigabe-Generation beim Start.
    var generation = 0
    /// „Ohne Quelle“ war bei Beginn der Anfrage erlaubt.
    var optInAtStart = false
    /// Einwilligungsversion zum Start, aus der Freigabe gelesen.
    var consentVersion = 0
    /// Kontextzustand vor der Rundenwahl und Grund der Wahl.
    var contextBefore: AssistantContext.Mode?
    var contextReason: AssistantContext.Reason?
    var startedAt = Date()
    /// Die Journalzeile `start` ist geschrieben (vom Erzeuger bestätigt; nach dem Schließen der Anfrage bleibt es `false`, auch wenn die
    /// Zeile noch landet: dann gilt `startQueued` und das Ergebnis des Schreibers).
    var journaled = false
    /// Runde 11, V1: Der Erzeuger hat seine Zeile `start` eingereiht (im Abschnitt der Ordnung, vor dem Schließen). Der Abschluss
    /// schreibt dann keine zweite, sondern wartet auf diese.
    var startQueued = false
    var used: [AssistantPassage] = []
    var sentences: [AssistantEmitted] = []
    /// Schrittnummern ALLER ausgegebenen Einheiten (auch Ansagen) in der Reihenfolge der Ausgabe: Grundlage der Ergebniszeile des
    /// Verlaufs je Einheit.
    var outputSteps: [Int] = []
    var counts = AssistantSentenceCounts()
    /// Bereiche der Bestandsangaben (gehen mit ins Protokoll).
    var inventoryAreas: [AssistantArea] = []
    /// Bereiche, die schon als Zeile `areas` im Journal stehen.
    var areasWritten: Set<String> = []
    /// Laufende Nummer der Einordnungen (Schritte).
    var steps = 0
    /// Die Antwort hat die Schrittgrenze erreicht und endet.
    var limited = false
    var revoked = false
    var cancelled = false
    var failure: AssistantFailure?

    /// Nächste Schrittnummer; ab `AssistantAuditLog.maxSteps` endet die Antwort.
    mutating func nextStep() -> Int {
        steps += 1
        if steps >= AssistantAuditLog.maxSteps { limited = true }
        return steps
    }
}

/// Gemeinsamer Zustand einer Anfrage zwischen Arbeit, Wächter und Sitzung.
private final class RunState: @unchecked Sendable {
    private let lock = NSLock()
    private var lastProgress = Date()
    private var timedOut = false
    private var record: AssistantRunRecord

    init(snapshot: AssistantRelease.Snapshot) {
        record = AssistantRunRecord()
        record.generation = snapshot.generation
        record.optInAtStart = snapshot.optIn
        record.consentVersion = snapshot.consentVersion
    }

    func touch() { lock.lock(); lastProgress = Date(); lock.unlock() }
    var idle: TimeInterval { lock.lock(); defer { lock.unlock() }; return Date().timeIntervalSince(lastProgress) }
    func fireTimeout() { lock.lock(); timedOut = true; lock.unlock() }
    var didTimeOut: Bool { lock.lock(); defer { lock.unlock() }; return timedOut }
    func update(_ body: (inout AssistantRunRecord) -> Void) { lock.lock(); body(&record); lock.unlock() }
    var snapshot: AssistantRunRecord { lock.lock(); defer { lock.unlock() }; return record }
    func markRevoked() { update { $0.revoked = true } }
    func markCancelled() { update { $0.cancelled = true } }
    func nextStep() -> Int { lock.lock(); defer { lock.unlock() }; return record.nextStep() }
}


/// Warum eine Anfrage nichts mehr sendet oder einreiht (Runde 10, L2).
enum AssistantRefusal: Sendable, Equatable {
    /// Ihr Abschluss hat begonnen (oder sie wurde verdrängt und wird abgeschlossen): Nach dem Schließen entsteht nichts mehr außer den
    /// Zeilen des Abschlusses selbst.
    case closed
    /// Das Protokoll der Anfrage hat einen klebrigen Schreibfehler.
    case auditFailed
}

/// Die Schranke einer Anfrage an jedem Abschnitt, in dem ihr Erzeuger sendet oder eine Zeile einreiht (Runde 10, L2): EINE Prüfung,
/// IM Lock der Ordnung (nicht an einer Vorprüfung davor). Geschlossen geht vor dem Schreibfehler.
struct AssistantRequestGate: Sendable {
    let request: String
    let index: AssistantStepIndex
    let audit: AssistantAuditLog?

    /// Nur im Lock der Ordnung aufrufen (`AssistantRelease.inSection` und die Abschnitte von `AssistantRunOutput`).
    func refusal() -> AssistantRefusal? {
        guard index.isOpen(request) else { return .closed }
        if audit?.hasFailed(request: request) == true { return .auditFailed }
        return nil
    }
}

/// Ausgang eines Sendens: die Prüfung der Generation und, falls die Anfrage sie verweigerte, der Grund.
struct AssistantDelivery {
    let check: AssistantCheck
    let refusal: AssistantRefusal?
}

/// Ausgang eines Laufs. Jedes Ereignis geht durch `send` bzw. `deliver`: Die Prüfung der Generation, der Lebenslauf der Anfrage
/// (offen?) und das Senden geschehen im selben Lock, nach einem Widerruf oder nach dem Schließen kommt also nichts mehr heraus.
final class AssistantRunOutput: @unchecked Sendable {
    let release: AssistantRelease
    let generation: Int
    private let continuation: AsyncStream<AssistantEvent>.Continuation
    private let gate: AssistantRequestGate?

    init(release: AssistantRelease, generation: Int, continuation: AsyncStream<AssistantEvent>.Continuation,
         gate: AssistantRequestGate? = nil) {
        self.release = release
        self.generation = generation
        self.continuation = continuation
        self.gate = gate
    }

    /// Sendet nur, wenn die Generation noch gilt (`stale`: nichts gesendet) und die Anfrage offen ist und kein Schreibfehler vorliegt.
    /// `onResult` läuft im selben Lock unmittelbar nach dem Senden, mit `true`, wenn das Ereignis den Strom erreicht hat, und `false`,
    /// wenn der Strom schon beendet war (Leser weg: die Aktion ließ sich nicht auslösen, Ergebnis `failed`). Dort wird die Ergebniszeile
    /// des Protokolls eingereiht (kurz, ohne Wartezeit): Sie steht damit vor jeder Rückmeldung des Controllers und stimmt mit dem
    /// tatsächlichen Senden überein. `onStale`: die Ablehnung wird IM Lock mit der vorgefundenen Generation gemeldet (Zeile
    /// „verworfen“ in der Reihenfolge der Erhöhungen) und nur, solange die Anfrage offen ist. `onRefused`: die Anfrage verweigert das
    /// Senden (geschlossen: keine Zeile mehr; Schreibfehler: Ergebniszeile „fehlgeschlagen“), ebenfalls im Lock.
    func deliver(_ event: AssistantEvent, onResult: ((Bool) -> Void)? = nil, onStale: ((Int) -> Void)? = nil,
                 onRefused: ((AssistantRefusal) -> Void)? = nil) -> AssistantDelivery {
        var refusal: AssistantRefusal?
        let gate = self.gate
        let check = release.ifCurrentAttempt(generation, onStale: { found in
            if let gate, !gate.index.isOpen(gate.request) {
                refusal = .closed
                return
            }
            onStale?(found)
        }) {
            if let why = gate?.refusal() {
                refusal = why
                onRefused?(why)
                return false
            }
            let delivered: Bool
            switch continuation.yield(event) {
            case .terminated: delivered = false
            default: delivered = true
            }
            onResult?(delivered)
            return delivered
        }
        return AssistantDelivery(check: check, refusal: refusal)
    }

    /// Wie `deliver`, ohne Auskunft über den Grund (Tests und Aufrufer, die ihn nicht brauchen).
    func send(_ event: AssistantEvent, onResult: ((Bool) -> Void)? = nil, onStale: ((Int) -> Void)? = nil) -> AssistantCheck {
        deliver(event, onResult: onResult, onStale: onStale).check
    }

    /// Fehler und Abbruch tragen keine Inhalte und gehen immer hinaus.
    func sendUnchecked(_ event: AssistantEvent) { continuation.yield(event) }
}

/// Die Standzeile (`state`) dieses Prozesslaufs (EINE je Ordnung, `AssistantReleaseClock.stateLine`): Sie wird von der ersten
/// Sitzung beim Anlegen eingereiht; die erste Anfrage wird erst angenommen, wenn sie in der Datei steht. Misslingt das Schreiben,
/// nimmt keine Sitzung eine Anfrage an (`.auditUnavailable`); die nächste Anfrage versucht es mit dem dann gültigen Stand erneut.
/// Mehrere gleichzeitige Anfragen (auch verschiedener Sitzungen) teilen EINEN Versuch. Der Lock ist ein Blatt: unter ihm wird nur
/// `make` gerufen (liest den Stand im Lock der Ordnung, Reihenfolge StateLine → Ordnung) und eingereiht, nie gewartet.
///
/// Der Fehler eines Versuchs gehört den ANFRAGEN, die ihn geteilt haben (Runde 12, W1; E4.2a Punkt 8, Schreibfolge 1): Jede Anfrage
/// meldet sich beim Beitritt an (`waiting`). Scheitert der Versuch, wird der Fehler am Anspruch jeder angemeldeten Anfrage vermerkt
/// (`noteFailure`, klebrig bis zu ihrem Abschluss), im selben Lock-Schritt und BEVOR der Versuch freigegeben wird. Ein späterer,
/// gelungener Versuch (einer anderen Anfrage oder des Abschlusses derselben) kann ihn also nicht verdecken: Wer den neuen Versuch
/// beginnen kann, findet den Fehler des alten schon an der Anfrage. Der Vermerk hängt nicht an der Fortsetzung des Erzeugers.
final class AssistantStateLine: @unchecked Sendable {
    private let lock = NSLock()
    private var make: (@Sendable () -> AssistantJournalLine)?
    private var run: AssistantRunToken?
    private var key = "state"
    private var enqueued = false
    private var confirmed = false
    private var attempt: Task<Bool, Never>?
    /// Die Anfragen des laufenden Versuchs (mit dem Protokoll, an dessen Anspruch ihr Fehler vermerkt wird).
    private var waiting: [(audit: AssistantAuditLog, request: String)] = []

    init() {}

    /// Die Zeile trägt die Laufnummer dieses Prozesslaufs (Schreibfolge 7): Sie wird beim ersten Schreiben vergeben und steht mit
    /// dieser Zeile dauerhaft im Protokoll, bevor eine Anfrage angenommen wird. Nur die erste Sitzung mit Protokoll reiht ein;
    /// weitere Sitzungen desselben Laufs ändern nichts (eine Zeile je Prozesslauf, mit dem ganzen Stand der Klasse A).
    func begin(audit: AssistantAuditLog?, run: AssistantRunToken, key: String, make: @escaping @Sendable () -> AssistantJournalLine) {
        lock.lock()
        defer { lock.unlock() }
        if enqueued || confirmed { return }
        self.make = make
        self.run = run
        self.key = key
        guard let audit else { return }
        audit.enqueue(make(), key: key, run: run)
        enqueued = true
    }

    /// `false`: Der Versuch, den diese Anfrage geteilt hat, ist gescheitert; der Fehler steht dann schon an ihrem Anspruch.
    func ensure(_ audit: AssistantAuditLog, request: String) async -> Bool {
        guard let task = startOrJoin(audit, request: request) else { return true }
        let ok = await task.value
        finish(task, ok: ok)
        return ok
    }

    /// `nil`: schon bestätigt (oder nie begonnen). Sonst der (geteilte) Versuch; der erste reiht die Zeile bei Bedarf neu ein.
    private func startOrJoin(_ audit: AssistantAuditLog, request: String) -> Task<Bool, Never>? {
        lock.lock()
        defer { lock.unlock() }
        guard let make, let run else { return nil }
        if confirmed { return nil }
        waiting.append((audit: audit, request: request))
        if let attempt { return attempt }
        let key = self.key
        if !enqueued {
            audit.enqueue(make(), key: key, run: run)
            enqueued = true
        }
        let task = Task { await audit.waitForLine(key: key) == nil }
        attempt = task
        return task
    }

    private func finish(_ task: Task<Bool, Never>, ok: Bool) {
        lock.lock()
        defer { lock.unlock() }
        if ok {
            confirmed = true
            waiting.removeAll()
        } else if attempt == task {
            // Erst der Vermerk an jeder Anfrage dieses Versuchs, dann die Freigabe (derselbe Lock-Schritt): Der nächste Versuch beginnt
            // erst danach. `noteFailure` nimmt nur den kurzen Lock der Ansprüche (ein Blatt) und wirkt nur auf einen bestehenden Anspruch.
            for item in waiting { item.audit.noteFailure(request: item.request) }
            waiting.removeAll()
            attempt = nil
            enqueued = false
        }
    }
}

/// Gibt die Sätze eines Laufs aus und führt Buch: Zähler, Zitate, Vorspann und die Protokollzeilen `step` und `commit`.
/// Nur innerhalb des Laufs benutzt.
private final class RunEmitter {
    enum Flow { case next, enough, revoked, limit, failed }
    /// Ausgang einer Ausgabe: gesendet, veraltet oder abgelehnt (`revoked`: Generation veraltet oder Leser weg), oder die
    /// Einordnung ließ sich nicht dauerhaft schreiben (`failed`: nichts wurde ausgegeben).
    enum Delivery { case delivered, revoked, failed, limit }

    let rid: String
    let generation: Int
    let release: AssistantRelease
    let output: AssistantRunOutput
    let state: RunState
    let journal: AssistantAuditLog?
    let index: AssistantStepIndex
    let clock = ContinuousClock()
    let started: ContinuousClock.Instant
    /// Stand dieser Anfrage aus dem EINEN Lesen von Generation und Freigabezustand: Bereiche und Opt-in für Abruf, Bestand,
    /// Prüfung. Nichts davon wird später aus veränderlichen Objekten nachgelesen.
    let areas: Set<AssistantArea>
    let optIn: Bool

    var texts: [String] = []
    var firstMs: Int?
    var taken = Set<String>()
    var quotes = 0
    var dropped = 0
    var unsourcedEmitted = 0
    var announcedUnsourced = false
    /// Stil `natural`: Es ging schon eine Einheit hinaus (auch Überbrückung und Ansage) bzw. eine Antworteinheit (jede außer Ansagen).
    /// Die Ansagen für eine langsame Antwort lesen es im Augenblick ihrer Verarbeitung.
    var anyEmitted = false
    var answerEmitted = false
    /// Die Schrittgrenze ist erreicht: die Antwort endet.
    var reachedLimit = false

    /// Höchstens so viele Zitate je Antwort; danach wird das Modell abgebrochen.
    static let maxQuotes = 3

    /// Die feste Antwort darf auf den Schalter „Antworten ohne Quelle“ hinweisen (nur, wenn es ihn gibt: freigeschaltet).
    let canOptIn: Bool

    init(rid: String, snapshot: AssistantRelease.Snapshot, release: AssistantRelease, output: AssistantRunOutput, state: RunState,
         journal: AssistantAuditLog?, index: AssistantStepIndex, started: ContinuousClock.Instant, canOptIn: Bool) {
        self.canOptIn = canOptIn
        self.rid = rid
        self.generation = snapshot.generation
        self.areas = snapshot.areas
        self.optIn = snapshot.optIn
        self.release = release
        self.output = output
        self.state = state
        self.journal = journal
        self.index = index
        self.started = started
    }

    private static func milliseconds(_ d: Duration) -> Int {
        Int(d.components.seconds) * 1000 + Int(d.components.attoseconds / 1_000_000_000_000_000)
    }

    var elapsedMs: Int { Self.milliseconds(started.duration(to: clock.now)) }

    func speech(_ text: String) -> String {
        AssistantSpeech.render(text, lineStart: false)?.speech ?? text
    }

    /// Reiht eine Zeile der Anfrage ein, solange sie OFFEN ist (Runde 10, L2): die Prüfung und das Einreihen in EINEM Abschnitt der
    /// Ordnung, nicht an einer Vorprüfung davor. `false`: geschlossen, nichts wurde eingereiht (keine Zeile hinter der Schlusszeile).
    /// Ohne Protokoll gibt es nichts einzureihen (`true`).
    @discardableResult
    private func enqueue(_ line: AssistantJournalLine, run: AssistantRunToken? = nil) -> Bool {
        guard let journal else { return true }
        return release.inSection { _ in
            guard index.isOpen(rid) else { return false }
            journal.enqueue(line, run: run)
            // Im selben Abschnitt wie das Einreihen: Ein Abschluss, der danach den Besitz nimmt, liest es (Runde 11, V1).
            if line.t == .start { state.update { $0.startQueued = true } }
            return true
        }
    }

    /// Ergebnis einer Pflichtzeile der Anfrage (`start`, `round`, `areas`), auf die der Erzeuger wartet.
    enum Landing {
        /// Die Zeile steht dauerhaft in der Datei (und die Anfrage ist noch offen).
        case stands
        /// Die Zeile ließ sich nicht schreiben (oder ein früherer Fehler der Anfrage liegt vor): kein Modell, keine Ausgabe, Ausgang „Fehler“.
        case failed
        /// Die Anfrage wurde inzwischen geschlossen: Die Zeile wurde nicht eingereiht, der Erzeuger endet als abgebrochen.
        case closed
    }

    /// Reiht eine Pflichtzeile ein (nur, solange die Anfrage offen ist; `nil`: nichts Neues) und wartet, bis alles bis hierher
    /// Eingereichte der Anfrage steht.
    func land(_ line: AssistantJournalLine?, run: AssistantRunToken? = nil) async -> Landing {
        guard let journal else { return .stands }
        if let line, !enqueue(line, run: run) { return .closed }
        if await journal.waitUntilWritten(request: rid) != nil { return .failed }
        return index.isOpen(rid) ? .stands : .closed
    }

    /// Wie `land`, mit mehreren Pflichtzeilen in der Reihenfolge der Liste: erst alle einreihen (solange die Anfrage offen ist), dann
    /// EINMAL warten, bis alles bis hierher Eingereichte steht (Stil `natural`: Rundenzeile und Bereichszeile vor dem Sprecher).
    func land(all lines: [AssistantJournalLine]) async -> Landing {
        guard let journal else { return .stands }
        for line in lines {
            guard enqueue(line) else { return .closed }
        }
        if await journal.waitUntilWritten(request: rid) != nil { return .failed }
        return index.isOpen(rid) ? .stands : .closed
    }

    /// Nummeriert einen Schritt und reiht seine Zeile ein (nur Einreihen; das Warten macht der Aufrufer). `nil`: Die Anfrage ist
    /// geschlossen; es gibt keinen Schritt und keine Zeile mehr (L2). Nummer und Zeile entstehen im selben Abschnitt.
    private func step(_ verdict: AssistantTraceVerdict) -> Int? {
        release.inSection { _ in
            guard index.isOpen(rid) else { return nil }
            let n = state.nextStep()
            if n >= AssistantAuditLog.maxSteps { reachedLimit = true }
            if let journal {
                var l = AssistantJournalLine(.step, req: rid)
                l.n = n
                l.verdict = verdict
                l.gen = generation
                journal.enqueue(l)
            }
            return n
        }
    }

    private func commitLine(_ n: Int, _ mark: AssistantCommitMark) -> AssistantJournalLine {
        var l = AssistantJournalLine(.commit, req: rid)
        l.n = n
        l.mark = mark
        return l
    }

    /// Die Ergebniszeile einer Einheit, die nicht hinausging („nicht ausgegeben“), im Abschnitt der Ordnung mit der dort gefundenen
    /// Generation, solange die Anfrage offen ist. Eine geschlossene Anfrage bekommt sie nicht mehr; ohne Ergebniszeile liest die Ansicht
    /// die Einheit einer abgeschlossenen Anfrage als „nicht vorgesehen“.
    private func enqueueNotEmitted(_ n: Int) {
        guard let journal else { return }
        release.inSection { found in
            guard index.isOpen(rid) else { return }
            journal.enqueue(commitLine(n, AssistantCommitMark(stage: .emit, ok: false, generation: found)))
        }
    }

    /// Gibt eine Einheit aus. Reihenfolge (Schreibfolge E4.2b): Einordnung (`step`) einreihen, dauerhaft schreiben lassen (das
    /// deckt auch die Ergebniszeilen der vorigen Einheiten und jeden früheren Fehler der Anfrage), erst dann den Ausgabe-Commit
    /// prüfen und senden. Misslingt das Schreiben, wird die Einheit NICHT ausgegeben. Ist die Anfrage geschlossen (ihr Abschluss hat
    /// begonnen), geht nichts hinaus und es entsteht keine Zeile mehr (Runde 10, L2): `.revoked`.
    func emit(_ text: String, speech: String, status: AssistantSentenceStatus, sources: [AssistantPassage] = [],
              quote: Bool = false) async -> Delivery {
        let announcement = status == .announcement
        guard let n = step(AssistantTraceVerdict(status: status)) else { return .revoked }
        if let journal, await journal.waitUntilWritten(request: rid) != nil {
            // Nichts wurde ausgegeben: das Ergebnis ist „verworfen“, nicht „unbekannt“.
            enqueueNotEmitted(n)
            if !announcement { state.update { $0.counts.drop(.revoked) } }
            return .failed
        }
        // Der Schritt steht im Index, BEVOR das Ereignis hinausgeht: Eine schnelle Rückmeldung des Controllers läuft nie ins Leere. Ist
        // der Index voll (bei den festen Grenzen nicht möglich, siehe `AssistantStepIndex.maxSteps`), geht die Einheit NICHT hinaus: Die
        // Antwort endet an der Grenze, die Zeile des Ausgabe-Commits sagt „nicht ausgegeben“. Ein geschlossener Index ist keine Grenze:
        // Die Anfrage ist abgeschlossen, die Einheit geht nicht hinaus und es entsteht keine Zeile.
        guard index.append(rid, step: n) else {
            guard index.isOpen(rid) else { return .revoked }
            enqueueNotEmitted(n)
            reachedLimit = true
            state.update { $0.limited = true }
            return .limit
        }
        await index.reach(rid, .beforeSend)
        let gen = generation
        // Das Senden prüft IM Lock die Generation, den Lebenslauf (offen?) und den Schreibfehler; Zeile und Buchführung der gesendeten
        // Einheit entstehen im selben Lock: Eine gesendete Einheit steht damit immer vor der Schlusszeile und im Stand des Abschlusses.
        let sent = output.deliver(.sentence(requestId: rid, generation: gen, text: text, speech: speech, status: status,
                                            sources: sources, quote: quote), onResult: { delivered in
            self.journal?.enqueue(self.commitLine(n, AssistantCommitMark(stage: .emit, ok: delivered, generation: gen, failed: !delivered)))
            guard delivered else { return }
            self.state.update { r in
                switch status {
                case .grounded: r.counts.grounded += 1
                case .inventory: r.counts.inventory += 1
                case .unsourced: r.counts.unsourced += 1
                case .neutral: r.counts.neutral += 1
                case .conversational: r.counts.conversational += 1
                case .summarized: r.counts.summarized += 1
                case .announcement: break
                }
                r.outputSteps.append(n)
                if !announcement { r.sentences.append(AssistantEmitted(text: text, status: status, sources: sources, quote: quote, step: n)) }
            }
        }, onStale: { found in
            self.journal?.enqueue(self.commitLine(n, AssistantCommitMark(stage: .emit, ok: false, generation: found)))
        }, onRefused: { why in
            // Ein Schreibfehler zwischen dem Warten und dem Senden (klebrig): nichts geht hinaus. Für eine geschlossene Anfrage keine Zeile.
            guard why == .auditFailed else { return }
            self.journal?.enqueue(self.commitLine(n, AssistantCommitMark(stage: .emit, ok: false, generation: gen, failed: true)))
        })
        let delivered = sent.check.ok
        if !delivered {
            index.removeLast(rid)
            if !announcement { state.update { $0.counts.drop(.revoked) } }
        }
        switch sent.refusal {
        case .auditFailed?: return .failed
        case .closed?: return .revoked
        case nil: break
        }
        if delivered, firstMs == nil { firstMs = elapsedMs }
        if delivered, !announcement { texts.append(text) }
        if delivered {
            anyEmitted = true
            if !announcement { answerEmitted = true }
        }
        return delivered ? .delivered : .revoked
    }

    private func flow(_ d: Delivery) -> Flow {
        switch d {
        case .delivered: .next
        case .revoked: .revoked
        case .failed: .failed
        case .limit: .limit
        }
    }

    /// Schreibt die Bereiche der Quellen, sobald sie feststehen (Zeile `areas`), nicht erst im `end`: Ein Prozessende danach
    /// verliert sie nicht. Nur Neues wird geschrieben, und die Zeile steht dauerhaft in der Datei, wenn die Methode mit `.stands`
    /// zurückkehrt. `.failed`: die Pflichtzeile ließ sich nicht schreiben (kein Modell, keine Ausgabe); `.closed`: die Anfrage wurde
    /// inzwischen geschlossen (nichts eingereiht).
    func noteAreas(_ areas: [AssistantArea]) async -> Landing {
        await land(areasLine(areas, force: false))
    }

    /// Die Zeile `areas` für diese Bereiche. Ohne `force` nur, wenn etwas Neues dabei ist (`nil`: nichts zu schreiben); mit `force`
    /// immer, auch für eine leere Menge oder schon geschriebene Bereiche (Stil `natural`: jede Phase hat IHRE Bereichszeile).
    func areasLine(_ areas: [AssistantArea], force: Bool) -> AssistantJournalLine? {
        let names = Set(areas.map(\.rawValue))
        var fresh = false
        state.update { r in
            guard force || !names.subtracting(r.areasWritten).isEmpty else { return }
            r.areasWritten.formUnion(names)
            fresh = true
        }
        guard fresh else { return nil }
        var l = AssistantJournalLine(.areas, req: rid)
        l.areas = names.sorted()
        l.gen = generation
        return l
    }

    /// Ein verworfener Kandidat: Auch seine Einordnung (Grund und Generation) wird eingereiht; sie behauptet nichts. Ihr Fehler
    /// bleibt klebrig und stoppt spätestens die nächste Ausgabe und den Abschluss.
    func drop(_ reason: AssistantDropReason) {
        dropped += 1
        _ = step(AssistantTraceVerdict(dropped: reason))
        state.update { $0.counts.drop(reason) }
    }

    /// Einen ganzen Quellblock mit Vorspann „Wörtlicher Auszug aus ‚Titel‘:“ VOR JEDEM Block (nicht nur beim Quellenwechsel):
    /// Jeder Block ist für sich gekennzeichnet, hörbar und sichtbar. Der Block geht unverändert hinaus.
    func emitQuote(_ q: AssistantGrounding.SourceBlock) async -> Flow {
        // Zweite, unabhängige Prüfung unmittelbar vor der Ausgabe (die erste ist die Auswahl in `Basis`): nur ein ganzer Block einer
        // zitierbaren Quellart, zeichengleich mit dem gespeicherten Text. Erzeugter Text, Mails und Kalendereinträge gehen nie hinaus.
        guard AssistantGrounding.mayOutput(q) else { drop(.notSnapped); return .next }
        let preface = AssistantGrounding.quotePreface(for: q.passage)
        let first = await emit(preface, speech: preface, status: .announcement)
        guard first == .delivered else { return flow(first) }
        // Die Stimme spricht den Block, wie er gespeichert ist (Runde 8, R1): kein `AssistantSpeech.render` (Markdown, Adressen, Emojis,
        // Abkürzungen, Dezimalpunkt) für einen wörtlichen Auszug. Was gesprochen wird, ist der Text, der angezeigt und gespeichert wird.
        let second = await emit(q.text, speech: q.text, status: .grounded, sources: [q.passage], quote: true)
        guard second == .delivered else { return flow(second) }
        taken.insert(q.key)
        quotes += 1
        return quotes >= Self.maxQuotes ? .enough : .next
    }

    func emitFixed(optIn: Bool) async -> Flow {
        let s = AssistantGrounding.fixedSentence(allowUnsourced: optIn, hint: canOptIn)
        return flow(await emit(s, speech: s, status: .neutral))
    }

    /// Prüft einen vollständigen Modellsatz der Datenrunde.
    func handleData(_ text: String, basis: AssistantGrounding.Basis, titles: [String], clock: [String]) async -> Flow {
        switch AssistantGrounding.judge(text, round: .data, basis: basis, titles: titles, clock: clock,
                                        optIn: optIn, taken: taken) {
        case .quote(let q): return await emitQuote(q)
        case .template(let t): return flow(await emit(t, speech: t, status: .neutral))
        case .dropped(let reason): drop(reason); return .next
        case .unsourced: drop(.notSnapped); return .next
        }
    }

    /// Prüft einen vollständigen Modellsatz der Allgemeinrunde. Ein Satz ohne Quelle wird zusätzlich gegen die eigenen Inhalte
    /// geprüft: Abruf mit dem Satz als Anfrage (über die Bereiche dieser Anfrage); rastet er an einem Quellblock ein, wird er
    /// verworfen.
    func handleGeneral(_ text: String, speech: String, raw: String, titles: [String], clock: [String],
                       knowledge: AssistantKnowledge) async -> Flow {
        switch AssistantGrounding.judge(text, round: .general, basis: AssistantGrounding.Basis(passages: []), titles: titles,
                                        clock: clock, optIn: optIn, raw: raw) {
        case .template(let t): return flow(await emit(t, speech: t, status: .neutral))
        case .dropped(let reason): drop(reason); return .next
        case .quote: drop(.notSnapped); return .next
        case .unsourced:
            let probed = await knowledge.probe(sentence: text, areas: areas)
            if AssistantGrounding.resemblesOwnContent(text, passages: probed) { drop(.ownData); return .next }
            if !announcedUnsourced {
                let a = AssistantGrounding.unsourcedAnnouncement
                let d = await emit(a, speech: a, status: .announcement)
                guard d == .delivered else { return flow(d) }
                announcedUnsourced = true
            }
            let d = await emit(text, speech: speech, status: .unsourced)
            guard d == .delivered else { return flow(d) }
            unsourcedEmitted += 1
            return .next
        }
    }
}

/// Ergebnis einer Änderung der Freigabe (Opt-in, Bereiche, Cloud-Zustimmung, „Ohne Quelle fortsetzen“), nachdem ihre Sicherungsfolge
/// (Zustandsdatei, Zeilen des Protokolls) abgeschlossen oder gescheitert ist.
public struct AssistantReleaseResult: Sendable, Equatable {
    /// Die Generation nach der Änderung (bei einer Änderung: die neue; bei einer Erweiterung die, die im Schritt des Wirksamwerdens
    /// vergeben wurde).
    public let generation: Int
    /// Die Änderung wurde angewendet: eine Sicherungsfolge lief. `false`: Der Stand war schon so, nichts geschah.
    public let changed: Bool
    /// Der gewünschte Stand gilt jetzt. Eine Einschränkung gilt immer (auch bei einem Schreibfehler); eine Erweiterung nur, wenn die
    /// Zustandsdatei den weiteren Stand bestätigt hat und sie im Prozess wirksam wurde.
    public let effective: Bool
    /// Fehler beim Schreiben einer Zeile des Protokolls (Speicher voll, Datei nicht lesbar); `nil`: alle Zeilen stehen.
    public let error: AssistantAuditError?
    /// Fehler beim Austausch der Zustandsdatei: der Ausgang ist OFFEN (die neue Fassung kann schon liegen, ohne bestätigt zu sein).
    /// Eine Einschränkung gilt trotzdem und blockiert eigene Inhalte für den Rest des Laufs („Widerruf nicht gesichert“); eine
    /// Erweiterung wird in diesem Lauf nicht wirksam. `nil`: der Austausch ist bestätigt.
    public let storeError: AssistantStoreError?

    public init(generation: Int, changed: Bool, effective: Bool, error: AssistantAuditError?, storeError: AssistantStoreError? = nil) {
        self.generation = generation
        self.changed = changed
        self.effective = effective
        self.error = error
        self.storeError = storeError
    }

    /// Eine Einschränkung ließ sich nicht dauerhaft sichern („Widerruf nicht gesichert“).
    public var revocationNotSaved: Bool { storeError != nil }
}

/// Eine angewendete, noch nicht abgeschlossene Änderung der Freigabe: Der einschränkende Teil gilt schon (Generation erhöht, Stand
/// im Lock gesetzt); `AssistantSession.settle` sichert die Zustandsdatei und schreibt die Zeilen und macht Erweiterungen erst
/// danach wirksam. Das erlaubt dem Controller, eine Einschränkung auf dem MainActor ohne Wartepunkt auszulösen und den Rest nachzuholen.
/// Wird ein Pending verworfen, ohne dass `settle` lief, geben seine Karten die Reihe frei (spätere Änderungen warten nicht ewig).
public final class AssistantReleasePending: @unchecked Sendable {
    let applied: AssistantRelease.Applied
    /// Opt-in wurde eingeschaltet (Erweiterung): der Datenkontext wird erst nach dem Wirksamwerden umgestellt.
    let optInExpansion: Bool
    /// „Ohne Quelle fortsetzen“: der Zustand `.data` wird erst nach der dauerhaften Zeile aufgehoben.
    let resume: Bool

    init(applied: AssistantRelease.Applied, optInExpansion: Bool, resume: Bool) {
        self.applied = applied
        self.optInExpansion = optInExpansion
        self.resume = resume
    }

    deinit { for step in applied.steps { step.ticket.complete() } }

    /// Die Generation nach dem sofort wirkenden Teil (bei einer reinen Erweiterung die unveränderte).
    public var generation: Int { applied.generation }
    /// Eine Änderung wurde angewendet.
    public var changed: Bool { applied.changed }
}

/// Ergebnis eines Ausgabe-Commits (Anzeige, Audiostart, Verlauf): ob die Aktion ausgelöst wurde und bei welcher Generation, die im
/// selben Lock geprüft wurde, in dem die Aktion lief (bei Ablehnung die vorgefundene).
public struct AssistantCommitResult: Sendable, Equatable {
    /// Die Aktion wurde ausgelöst.
    public let ok: Bool
    /// Die Generation galt, die Aktion ließ sich aber nicht auslösen (z. B. Start der Stimme gescheitert).
    public let failed: Bool
    public let generation: Int
    /// Ein Schreibfehler des Protokolls dieser Anfrage ist schon bekannt (Controller: laufende Ausgabe stoppen, „Protokoll nicht
    /// schreibbar“). Zeigt nur schon Geschehenes; den aktuellen Stand liefert `AssistantSession.auditHealthy`.
    public let auditFailed: Bool

    public init(ok: Bool, failed: Bool = false, generation: Int, auditFailed: Bool = false) {
        self.ok = ok
        self.failed = failed
        self.generation = generation
        self.auditFailed = auditFailed
    }
}

public actor AssistantSession {
    public static let maxStoredTurns = 200
    /// Höchstens so viele Auszüge je Frage.
    static let passageLimit = 8
    /// Wie lange `commit` auf den geordneten Abschluss eines Laufs wartet (hängendes Modell). Je Sitzung einstellbar (Tests).
    static let defaultCommitWait: Duration = .seconds(2)
    /// Wie viele Endstände unabgeschlossener Läufe gemerkt werden (danach wird der älteste abgemeldet).
    static let maxRemembered = 16

    private enum Outcome: Sendable {
        case finished(text: String, firstMs: Int?, totalMs: Int, failure: AssistantFailure?)
        case cancelled
    }

    private struct ActiveRun {
        let stop: @Sendable () -> Void
        let state: RunState
    }

    private let model: any BookTalkModel
    private let knowledge: AssistantKnowledge
    private let config: AssistantConfig
    private let release: AssistantRelease
    private let context: AssistantContext
    private let released: Bool
    private let stepIndex: AssistantStepIndex
    private let commitWait: Duration
    private let sessionId = UUID().uuidString
    private let audit: AssistantAuditLog?
    private let stateLine: AssistantStateLine
    private let runToken: AssistantRunToken
    /// Stil `natural`: Überbrückungssätze reihum (je Sitzung) und die Hinweise „Äußerung gilt“ je Anfrage.
    private let bridges = AssistantBridgeRotation()
    private let commitNotes = AssistantCommitNotes()
    private nonisolated var clock: AssistantReleaseClock { release.clock }
    /// Die Ordnung dieses Prozesslaufs (für die Meldestelle der Quellen und Tests).
    nonisolated var order: AssistantReleaseClock { release.clock }
    /// Die Sicherungsfolge der Änderungen der Klasse A (Zustandsdatei, Zeilen, Register der Ordnung): derselbe Ablauf wie ohne Sitzung.
    private let settler: AssistantReleaseSettler
    private var active: [String: ActiveRun] = [:]
    private var earlyCancel: Set<String> = []
    private var results: [String: RunState] = [:]
    private var resultOrder: [String] = []
    /// Anfragen, die `commit` abgeschlossen hat, während ihr Lauf noch hing: Das späte Ende des Laufs merkt sie nicht noch einmal.
    private var finishedWhileRunning: Set<String> = []
    public private(set) var history: [AssistantTurn]

    /// Öffentlicher Initializer: Die Freischaltung der Funktion „Antworten ohne Quelle“ ist die Konstante
    /// `AssistantUnsourcedConsent.isReleased` und nicht wählbar. Solange sie `false` ist, verhält sich die Sitzung wie mit
    /// ausgeschaltetem Opt-in: Keine öffentliche Einstellung (Konfiguration `allowUnsourced`, `setAllowUnsourced`, `resumeFree`,
    /// gespeicherte Einwilligung, Verlauf mit einer Runde ohne Quelle) erreicht den freien Zustand.
    /// `cloudConsent`: Zustimmung zum Cloud-Ziel. Ohne sie endet jede Anfrage an ein Cloud-Modell mit `.consentRequired`
    /// (Vorgabe: nein). Die Bereiche kommen aus `knowledge.currentAreas`.
    /// `releaseStore`: die Zustandsdatei der Freigaben (Schreibfolge 8, Klasse A). Mit ihr kommen Bereiche, Opt-in und Cloud-Zustimmung
    /// aus der gültigen Fassung der Datei (`cloudTarget`: Kennung des Cloud-Ziels, für das die Zustimmung gilt), und jede Änderung
    /// wird dort gesichert; ohne sie gelten `knowledge.currentAreas` und `cloudConsent`, und nichts wird gesichert.
    public init(model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig, history: [AssistantTurn] = [],
                audit: AssistantAuditLog? = nil, cloudConsent: Bool = false, releaseStore: AssistantReleaseStore? = nil,
                cloudTarget: String? = nil) {
        self.init(model: model, knowledge: knowledge, config: config, history: history, audit: audit,
                  released: AssistantUnsourcedConsent.isReleased, cloudConsent: cloudConsent, releaseStore: releaseStore,
                  cloudTarget: cloudTarget)
    }

    /// Nur für Tests (`@testable`): wählt die Freischaltung, um den freien Modus unabhängig von der Produktkonstante zu prüfen.
    /// Beiträge aus Runden ohne Quelle kommen bei `released == false` gar nicht erst in den Verlauf (`AssistantTurn.admitted`).
    init(model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig, history: [AssistantTurn] = [],
         audit: AssistantAuditLog? = nil, released: Bool, cloudConsent: Bool = true, releaseStore: AssistantReleaseStore? = nil,
         cloudTarget: String? = nil, stepIndex: AssistantStepIndex = AssistantStepIndex(),
         commitWait: Duration = AssistantSession.defaultCommitWait) {
        self.stepIndex = stepIndex
        self.commitWait = commitWait
        // Die eine Ordnung dieses Prozesslaufs: mit Speicher die seine (alle Sitzungen und Änderungen ohne Sitzung teilen Lock,
        // Generation, Stand der Klasse A, Epochen, Lauf-Token, Standzeile, Zähler und Reihe), ohne Speicher eine eigene, mit dem Stand
        // aus den Angaben. Die Sitzung hält davon nichts als Kopie: Sie liest ihre Sicht im selben Lesen wie die Generation.
        let clock = releaseStore?.clock ?? AssistantReleaseClock()
        let run = clock.run
        if releaseStore == nil {
            clock.seed(record: AssistantReleaseRecord.seeded(optIn: config.allowUnsourced && released, consentVersion: config.consentVersion,
                                                             areas: knowledge.currentAreas, cloudConsent: cloudConsent, target: cloudTarget),
                       locked: false)
        }
        if let audit { clock.attach(audit: audit) }
        let kind = AssistantModelKind.classify(label: config.modelLabel)
        let release = AssistantRelease(clock: clock, view: AssistantReleaseView(cloud: config.isCloud, target: cloudTarget, released: released),
                                       onIncrease: AssistantReleaseSettler.increaseWriter(audit: audit, clock: clock, cloud: config.isCloud, model: kind))
        let optIn = release.snapshot.optIn
        self.runToken = run
        self.settler = AssistantReleaseSettler(release: release, store: releaseStore, audit: audit, model: kind)
        self.model = model
        self.knowledge = knowledge
        self.config = config
        self.released = released
        self.release = release
        let admitted = AssistantTurn.admitted(history, released: released)
        let kept = admitted.count > Self.maxStoredTurns ? Array(admitted.suffix(Self.maxStoredTurns)) : admitted
        self.context = AssistantContext(optIn: optIn, history: kept)
        self.audit = audit
        self.history = kept
        // Die Standzeile des Öffnens: EINE je Prozesslauf (die erste Sitzung reiht sie ein), Zustand vor jeder Anfrage; die erste
        // Anfrage wartet auf sie.
        self.stateLine = clock.stateLine
        clock.stateLine.begin(audit: audit, run: run, key: clock.key("state", 0)) { [config] in
            Self.stateLine(release.snapshot, config: config, store: releaseStore)
        }
    }

    // MARK: Freigabe

    /// Die Standzeile: Freigabezustand beim Öffnen (Opt-in, Einwilligungsversion, Bereichsnamen, Cloud-Zustimmung).
    private static func stateLine(_ snapshot: AssistantRelease.Snapshot, config: AssistantConfig,
                                  store: AssistantReleaseStore?) -> AssistantJournalLine {
        var line = AssistantJournalLine(.state)
        line.at = Date()
        line.gen = snapshot.generation
        line.unsourced = snapshot.optIn
        line.consent = snapshot.consentVersion
        line.areas = AssistantRelease.names(snapshot.areas)
        line.granted = snapshot.cloudConsent
        line.locked = snapshot.locked ? true : nil
        line.model = AssistantModelKind.classify(label: config.modelLabel)
        line.cloud = config.isCloud
        // Der ganze Stand der Klasse A: mit der Zustandsdatei auch die Bereiche des anderen Ziels (lokal/Cloud).
        if let store { line.otherAreas = AssistantReleaseRecord.names(store.record.areas(cloud: !config.isCloud)) }
        return line
    }

    /// Ob unbelegte Sätze (Allgemeinwissen) gesprochen werden dürfen. Ohne Aktor-Sprung lesbar (hinter einem Lock).
    public nonisolated var allowsUnsourced: Bool { release.isOptedIn }

    /// Die Funktion „Antworten ohne Quelle“ ist freigeschaltet (sonst gibt es weder Schalter noch Knopf).
    public nonisolated var isReleased: Bool { released }

    /// Aktuelle Freigabe-Generation (monoton). Ohne Aktor-Sprung lesbar; der Controller prüft sie vor Anzeige und Audiostart.
    public nonisolated var releaseGeneration: Int { release.currentGeneration }

    /// Die wirksamen Bereiche dieser Sitzung (aus der Freigabe, mit der Generation gelesen).
    public nonisolated var effectiveAreas: Set<AssistantArea> { release.snapshot.areas }

    /// Die wirksame Cloud-Zustimmung dieser Sitzung.
    public nonisolated var effectiveCloudConsent: Bool { release.snapshot.cloudConsent }

    /// Bereiche UND Cloud-Zustimmung aus EINEM Lesen: Wer beides zusammen braucht, liest nicht zweimal (dazwischen kann sich der Stand
    /// durch eine Änderung ändern).
    public nonisolated var effectiveState: (areas: Set<AssistantArea>, cloudConsent: Bool) {
        let snapshot = release.snapshot
        return (snapshot.areas, snapshot.cloudConsent)
    }

    /// Zustand des Datenkontexts: `.data` (jede Runde antwortet aus den Inhalten) oder `.free` (Runden ohne Quelle möglich).
    public nonisolated var contextMode: AssistantContext.Mode { context.mode }

    /// Der Knopf „Ohne Quelle fortsetzen“ ist sinnvoll: Opt-in an, freigeschaltet und der Zustand `.data`.
    public nonisolated var canResumeFree: Bool { released && release.isOptedIn && context.mode == .data }

    /// Zahl der angemeldeten Läufe (ein Lauf bleibt bis `commit` angemeldet; Tests).
    nonisolated var registeredRunCount: Int { release.registeredCount }

    /// Der Beobachter wird nach JEDER Erhöhung der Generation gerufen (auf dem Thread des Auslösers, ohne Aktor-Sprung): Der
    /// Controller stoppt dann die Wiedergabe und leert die Warteschlange, unabhängig von Quellen und vom Modellende.
    public nonisolated func setReleaseObserver(_ observer: (@Sendable (Int) -> Void)?) { release.setObserver(observer) }

    // Änderungen der Freigabe (Klasse A, ADR-0028 E4.2b Schreibfolge 8). Jede läuft in zwei Schritten:
    // 1. `apply…` (synchron, ohne Warten): Der EINSCHRÄNKENDE Teil gilt SOFORT (eine Erhöhung der Generation, Stand im selben Lock,
    //    laufende Antworten enden). Ein erweiternder Teil ändert hier nichts und erhält hier keine Generation.
    // 2. `settle` (wartet, streng nacheinander mit den Sicherungsfolgen der anderen Änderungen):
    //    - Einschränkung: Zustandsdatei dauerhaft ersetzen → Zeile `release` (Lauf, Änderungsnummer, Generation, Ursache) → Rückkehr.
    //      Scheitert der Austausch: Ausgang offen, „Widerruf nicht gesichert“, für den Rest des Laufs nichts mehr aus eigenen Inhalten,
    //      die Sicherung wird bei jeder Gelegenheit erneut versucht.
    //    - Erweiterung: Zeile „beabsichtigt“ (ohne Generation) → Zustandsdatei ersetzen (das ist der Commit) → erst nach bestätigtem
    //      Austausch im Prozess wirksam (Generation steigt dort, im unteilbaren Schritt) → Zeile „wirksam“ → Rückkehr. Scheitert ein
    //      Schritt davor, wird sie in diesem Lauf nicht wirksam (Zeile „Erweiterung in diesem Lauf nicht wirksam“ sobald möglich).
    //    - Gemischte Änderung: erst der einschränkende, dann der erweiternde Teil, je mit eigener Änderungsnummer.
    // Die Komfortmethoden (`setAllowUnsourced`, `setAreas`, …) machen beides hintereinander.

    /// Schaltet „Antworten ohne Quelle“ um. Aus: wirkt sofort (Einschränkung). An: erst nach bestätigter Zustandsdatei; der Datenkontext
    /// wird dann auf `.free` gestellt, aber nur, wenn das Gespräch noch keine Datenrunde hatte. `consentVersion`: Fassung des
    /// Einwilligungstextes (sonst die der Konfiguration).
    public nonisolated func applyAllowUnsourced(_ on: Bool, consentVersion: Int? = nil) -> AssistantReleasePending {
        let effective = on && released
        let before = release.isOptedIn
        let applied = release.apply(AssistantReleaseChange(cause: .optIn, optIn: (effective, consentVersion ?? config.consentVersion)))
        if applied.steps.contains(where: { $0.kind == .restriction }), before, !effective { context.optInChanged(false) }   // sofort
        let expansion = applied.steps.contains { $0.kind == .expansion && $0.delta.optIn }
        return AssistantReleasePending(applied: applied, optInExpansion: expansion, resume: false)
    }

    /// Ändert die Bereiche. Weniger Bereiche gelten sofort, mehr erst nach bestätigter Zustandsdatei.
    /// `cloudConsent`: gilt die Zustimmung zum Cloud-Ziel für den NEUEN Stand der Bereiche? Sie wird in derselben Änderung
    /// gesetzt (nicht gesondert danach): Reichen die Bereiche der Zustimmung nicht, ist sie mit der Erhöhung sofort weg, nie
    /// gelten mehr Bereiche mit einer Zustimmung, die sie nicht deckt. `nil`: unverändert (lokale Modelle).
    public nonisolated func applyAreas(_ areas: Set<AssistantArea>, cloudConsent: Bool? = nil) -> AssistantReleasePending {
        let applied = release.apply(AssistantReleaseChange(cause: .areas, areas: areas, cloudConsent: cloudConsent))
        return AssistantReleasePending(applied: applied, optInExpansion: false, resume: false)
    }

    /// Cloud-Zustimmung erteilt oder entzogen. Entzug gilt sofort, Erteilung erst nach bestätigter Zustandsdatei.
    public nonisolated func applyCloudConsent(_ granted: Bool) -> AssistantReleasePending {
        let applied = release.apply(AssistantReleaseChange(cause: .consent, cloudConsent: granted))
        return AssistantReleasePending(applied: applied, optInExpansion: false, resume: false)
    }

    /// „Ohne Quelle fortsetzen“: ausdrückliche Nutzerhandlung, die den Zustand `.data` aufhebt, aber erst nach der dauerhaften
    /// Zeile. Ohne Wirkung (`changed == false`), wenn nicht freigeschaltet, Opt-in aus oder schon `.free`. Der Zustand gehört dem
    /// Prozess (nicht der Zustandsdatei): Die Änderung hat ein Ereignis mit Generation und Zeile, aber keine Sicherungsfolge der Datei.
    public nonisolated func applyResumeFree() -> AssistantReleasePending {
        guard released, release.isOptedIn, context.mode == .data else {
            return AssistantReleasePending(applied: AssistantRelease.Applied(generation: release.currentGeneration, changed: false, steps: []),
                                           optInExpansion: false, resume: false)
        }
        let applied = release.apply(AssistantReleaseChange(cause: .resumeFree, force: true))
        return AssistantReleasePending(applied: applied, optInExpansion: false, resume: true)
    }

    /// Jede andere Änderung der Freigabe: Generation hoch, laufende Antworten abbrechen. Mit `cloudConsent` zusätzlich die
    /// Zustimmung setzen (Grund `consent`).
    public nonisolated func applyReleaseChange(_ cause: AssistantReleaseCause = .other, cloudConsent: Bool? = nil) -> AssistantReleasePending {
        let applied = release.apply(AssistantReleaseChange(cause: cause, cloudConsent: cloudConsent, force: cloudConsent == nil))
        return AssistantReleasePending(applied: applied, optInExpansion: false, resume: false)
    }

    /// Sichert die Teile der Änderung der Reihe nach (streng nacheinander mit den Folgen anderer Änderungen, gleich welcher Sitzung,
    /// `AssistantReleaseSettler`) und gibt den Stand zurück. Jede Folge wartet auf ihre Reihe in der Reihenfolge des Anwendens: Wer mehrere
    /// Änderungen angewendet hat, wickelt sie in dieser Reihenfolge ab (der Controller: eine Kette in der Reihenfolge des Anwendens)
    /// oder nebenläufig; ein späteres `settle` vor dem früheren, das nie läuft, wartete ewig. Ein verworfenes
    /// `AssistantReleasePending` gibt seine Reihe frei. Ein Fehler steht in `error` (Zeile des Protokolls) oder `storeError`
    /// (Zustandsdatei, Ausgang offen); eine Einschränkung gilt trotzdem, eine Erweiterung nur nach bestätigtem Austausch.
    public nonisolated func settle(_ pending: AssistantReleasePending) async -> AssistantReleaseResult {
        let applied = pending.applied
        let out = await settler.settle(applied)
        let expands = applied.expands || pending.optInExpansion || pending.resume
        var done = out.effective
        if done, pending.optInExpansion { context.optInChanged(true) }
        if done, pending.resume { done = context.resumeFree(optIn: release.isOptedIn) }
        return AssistantReleaseResult(generation: out.generation, changed: applied.changed, effective: !expands || done, error: out.error,
                                      storeError: out.storeError)
    }

    /// Holt nach, was bei einer früheren Gelegenheit nicht ging (siehe `AssistantReleaseSettler.catchUp`): eine nicht gesicherte
    /// Einschränkung erneut sichern, fehlende Zeilen „in diesem Lauf nicht wirksam“ schreiben, auch solche, die eine ANDERE Sitzung oder
    /// eine Änderung ohne Sitzung hinterlassen hat (Register der Ordnung). Läuft bei jeder Anfrage (nach ihrer gesicherten
    /// `start`-Zeile) und bei jeder Änderung.
    public nonisolated func catchUp() async { await settler.catchUp() }

    /// Versucht die Sicherung des jetzigen Stands erneut (nach einem gescheiterten Austausch der Zustandsdatei). Läuft streng
    /// nacheinander mit den anderen Änderungen. Auch ohne Fehler schreibt es die Datei neu. Die Blockade für den Rest des Laufs
    /// bleibt in jedem Fall bestehen.
    @discardableResult
    public nonisolated func retrySave() async -> AssistantStoreError? { await settler.retrySave() }

    /// Eine Einschränkung ließ sich nicht sichern oder die Datei steht noch nicht fest: nicht bestätigt.
    public nonisolated var needsSave: Bool { settler.store?.isDirty ?? false }

    /// Die Freigaben sind gesperrt (ungültige Zustandsdatei beim Start): kein Bereich, keine Zustimmung, Anzeige „Freigaben prüfen“.
    public nonisolated var isReleaseLocked: Bool { release.snapshot.locked }

    /// Eine Einschränkung ließ sich nicht sichern: für den Rest des Laufs nichts aus eigenen Inhalten („Widerruf nicht gesichert“).
    public nonisolated var isContentBlocked: Bool { release.snapshot.blocked }

    // MARK: Klasse B: Inhalte der Quellen

    /// Ein Inhalt oder eine Schutzregel einer Quelle ändert sich (Lesegrenze, KI-Freigabe einer Besprechung, Schutz der Ablage, Quelle
    /// geändert oder gelöscht, auch eingespielt von einem anderen Gerät). Synchron und VOR der Änderung im Speicher der Quelle
    /// aufrufen: Die Generation steigt im selben Lock wie bei Klasse A (gemeinsame Ordnung), laufende Antworten enden, der Bestand
    /// ist `stale`: aus eigenen Inhalten wird nichts ausgegeben, bis `contentInstalled` den neuen Bestand meldet. Die Zeile mit
    /// Ursache „Inhalt“ (ohne Titel und Inhalt) wird nur eingereiht, nicht abgewartet. Gibt die Epoche zurück.
    @discardableResult
    public nonisolated func contentWillChange() -> Int { release.contentWillChange().epoch }

    /// Der neue Bestand steht für die Änderung dieser Epoche. `false`: Eine neuere Änderung ist dazugekommen, der Bestand bleibt `stale`.
    @discardableResult
    public nonisolated func contentInstalled(epoch: Int) -> Bool { release.contentInstalled(epoch: epoch) }

    /// Baut den Bestand neu und meldet ihn: ersetzt den Index durch genau diese Dokumente, dann `contentInstalled`.
    @discardableResult
    public func install(_ documents: [AssistantDocument], epoch: Int) async -> Bool {
        await knowledge.install(documents)
        return release.contentInstalled(epoch: epoch)
    }

    /// Der Bestand steht (nicht `stale`).
    public nonisolated var isContentReady: Bool { release.snapshot.corpusReady }

    @discardableResult
    public nonisolated func setAllowUnsourced(_ on: Bool, consentVersion: Int? = nil) async -> AssistantReleaseResult {
        await settle(applyAllowUnsourced(on, consentVersion: consentVersion))
    }

    @discardableResult
    public nonisolated func setAreas(_ areas: Set<AssistantArea>, cloudConsent: Bool? = nil) async -> AssistantReleaseResult {
        await settle(applyAreas(areas, cloudConsent: cloudConsent))
    }

    @discardableResult
    public nonisolated func setCloudConsent(_ granted: Bool) async -> AssistantReleaseResult {
        await settle(applyCloudConsent(granted))
    }

    @discardableResult
    public nonisolated func resumeFree() async -> AssistantReleaseResult {
        await settle(applyResumeFree())
    }

    @discardableResult
    public nonisolated func releaseDidChange(_ cause: AssistantReleaseCause = .other, cloudConsent: Bool? = nil) async -> AssistantReleaseResult {
        await settle(applyReleaseChange(cause, cloudConsent: cloudConsent))
    }

    /// Wiedergabesperre dieser Sitzung („verworfen ab Generation g“). Die Freigabe rückt sie im selben Lock vor, in dem die
    /// Generation steigt; die Stimme fragt sie vor jedem Start eines Satzes (`permits`), ohne Aktor-Sprung und ohne Warten.
    public nonisolated var playbackGate: AssistantPlaybackGate { release.playback }

    /// Geordnete Ausgabegrenze: Führt einen Ausgabe-Commit der Anzeige eines Ereignisses der Generation `generation` aus, und zwar
    /// IM selben Lock, in dem die Generation steigt: `body` läuft nie nach der Rückkehr einer Erhöhung auf eine neuere Generation.
    /// Die Ergebniszeile (`commit`) wird im selben Lock eingereiht, mit der dort geprüften Generation: „ausgeführt“ nur, wenn
    /// `body` lief, sonst „verworfen“ mit der vorgefundenen Generation. `index` = Nummer in der Reihenfolge der empfangenen
    /// `sentence`-Ereignisse dieser Anfrage. `reportPass: false`: nur eine Ablehnung melden.
    ///
    /// Pflichten von `body`: kurz und synchron, ohne I/O und ohne Warten; ruft NIE in die Sitzung oder Freigabe zurück
    /// (`releaseGeneration`, `setAllowUnsourced`, `commitOutput`, … hängen, denn `NSLock` ist nicht rekursiv).
    @discardableResult
    public nonisolated func commitOutput(requestId: String, index: Int, generation: Int, stage: AssistantCommitStage,
                                         reportPass: Bool = true, _ body: () -> Void) -> AssistantCommitResult {
        // Eine Anfrage, die dieser Prozesslauf nicht kennt (früherer Lauf), wird nie fortgesetzt: nichts wird ausgelöst.
        guard stepIndex.knows(requestId) else {
            return AssistantCommitResult(ok: false, generation: release.currentGeneration, auditFailed: false)
        }
        let step = stepIndex.step(requestId, index: index)
        // Ein Schreibfehler des Protokolls dieser Anfrage ist klebrig und geht vor allem anderen: Nichts wird mehr ausgelöst. Die
        // Prüfung steht IM Lock der Ordnung (vor `body`), nicht davor. Ebenso die Prüfung „nimmt noch Commits an“ (Runde 9, S1): Eine
        // Anfrage, deren Abschluss begonnen hat, löst nichts mehr aus; sie wird im selben Lock geschlossen (`closeForCommits`), in dem
        // diese Prüfung läuft.
        var blocked = false
        var closed = false
        let check = release.ifCurrent(generation, onStale: { found in
            // Die Ablehnung wird im Lock eingereiht, mit der vorgefundenen Generation: nie hinter der Zeile einer späteren Änderung.
            guard self.stepIndex.accepts(requestId) else { return }
            self.enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: stage, ok: false, generation: found))
        }) {
            guard stepIndex.accepts(requestId) else {
                closed = true
                return
            }
            if audit?.hasFailed(request: requestId) == true {
                blocked = true
                enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: stage, ok: false, generation: generation, failed: true))
                return
            }
            body()
            if reportPass { enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: stage, ok: true, generation: generation)) }
        }
        return AssistantCommitResult(ok: check.ok && !blocked && !closed, generation: check.generation,
                                     auditFailed: blocked || (audit?.hasFailed(request: requestId) ?? false))
    }

    /// Schließt die Anfrage (Anzeige, Audiostart, Senden und Einreihen des Erzeugers), im Lock der Ordnung (Runde 9, S1). Immer VOR der
    /// Schlusszeile und VOR dem Ende des Anspruchs auf ihren Fehler: Ein Commit oder Erzeuger-Abschnitt, der danach den Lock bekommt, löst
    /// nichts aus und reiht nichts ein; einer, der davor lief, hat seine Zeilen vor der Schlusszeile eingereiht. Die Zuordnung
    /// Ereignis → Schritt bleibt bis zum Ende des Abschlusses lesbar. Ohne Besitz des Abschlusses: für das Verdrängen aus dem Gedächtnis
    /// (der Abschluss folgt dann durch `commit`).
    private nonisolated func closeForCommits(_ requestId: String) {
        release.inSection { _ in stepIndex.close(requestId) }
    }

    /// Runde 10, L1: Übernimmt den Abschluss der Anfrage und schließt sie, in EINEM Schritt im Lock der Ordnung. `true` genau für den
    /// ersten Aufrufer pro Kennung; jeder weitere (auch nach einem hängenden Lauf, auch die Verdrängung) kehrt sofort zurück und schreibt
    /// nichts. Wird im Aktor VOR dem ersten Wartepunkt gerufen: Zwischen Prüfung und Wirkung liegt kein `await`.
    private func claimCompletion(_ requestId: String) -> Bool {
        release.inSection { _ in stepIndex.claimCompletion(requestId) }
    }

    /// Der Audiostart eines Satzes: Prüfung der Generation, der TATSÄCHLICHE Start (`start`: gibt zurück, ob er ausgelöst wurde)
    /// und die Ergebniszeile geschehen im selben Lock. Die Marke „ausgeführt“ steht nur, wenn `start` lief und `true` zurückgab;
    /// „fehlgeschlagen“, wenn die Generation galt, der Start sich aber nicht auslösen ließ; „verworfen“ bei veralteter Generation
    /// (dann läuft `start` nicht). Dieselben Pflichten wie bei `commitOutput`: kurz, ohne I/O, kein Rückruf in die Sitzung.
    @discardableResult
    public nonisolated func commitStart(requestId: String, index: Int, generation: Int, _ start: () -> Bool) -> AssistantCommitResult {
        // Kein Audiostart für eine Anfrage, die dieser Prozesslauf nicht kennt (Schreibfolge 6: nichts wird wiederholt).
        guard stepIndex.knows(requestId) else {
            return AssistantCommitResult(ok: false, failed: false, generation: release.currentGeneration, auditFailed: false)
        }
        let step = stepIndex.step(requestId, index: index)
        var blocked = false
        var closed = false
        let check = release.ifCurrentAttempt(generation, onStale: { found in
            guard self.stepIndex.accepts(requestId) else { return }
            self.enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: .audio, ok: false, generation: found))
        }) {
            // Eine Anfrage, deren Abschluss begonnen hat (oder die das Gedächtnis verdrängt hat), startet nichts mehr (Runde 9, S1): die
            // Prüfung im Lock, nicht nur davor.
            guard stepIndex.accepts(requestId) else {
                closed = true
                return false
            }
            // Schreibfehler dieser Anfrage: kein Start (klebrig, geht dem Widerruf voran).
            if audit?.hasFailed(request: requestId) == true {
                blocked = true
                enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: .audio, ok: false, generation: generation, failed: true))
                return false
            }
            let started = start()
            enqueueMark(requestId: requestId, step: step,
                        mark: AssistantCommitMark(stage: .audio, ok: started, generation: generation, failed: !started))
            return started
        }
        return AssistantCommitResult(ok: check.ok, failed: check.failed && !blocked && !closed, generation: check.generation,
                                     auditFailed: blocked || (audit?.hasFailed(request: requestId) ?? false))
    }

    /// Nur für Tests der Commits ohne Lauf: macht der Sitzung eine Anfrage bekannt, als hätte dieser Prozesslauf ein Ereignis
    /// ausgegeben (sonst gelten Commits für unbekannte Anfragen nie).
    nonisolated func admitForTesting(requestId: String) { stepIndex.append(requestId, step: 0) }

    /// Meldet das Ergebnis eines Ausgabe-Commits, das NICHT im Lock lief (der Controller hat gar nichts ausgelöst, z. B. weil
    /// die Stimme die Generation schon verworfen hatte, oder ein Audiostart ließ sich nicht auslösen). Ohne Warten. Die Prüfung
    /// geschieht im Lock der Ordnung (Runde 8, R5): Gilt die mitgebrachte Generation dort noch, trägt die Zeile genau sie
    /// (`failed`: die Aktion ließ sich nicht auslösen, obwohl nichts widerrufen war: „fehlgeschlagen“, nie „ausgeführt“). Ist sie
    /// inzwischen veraltet, steht NICHT „fehlgeschlagen unter der alten Generation“ (das behauptete, die Generation habe beim
    /// Scheitern noch gegolten), sondern „verworfen“ mit der dort vorgefundenen Generation: Ein später gemeldetes Ergebnis wird
    /// nie älter als der Stand, in dem es gemeldet wurde.
    public nonisolated func noteCommit(requestId: String, index: Int, stage: AssistantCommitStage, ok: Bool, generation: Int,
                                       failed: Bool = false) {
        let step = stepIndex.step(requestId, index: index)
        release.inSection { found in
            // Eine geschlossene Anfrage (ihr Abschluss hat begonnen) bekommt nach ihrer Schlusszeile keine Zeile mehr (Runde 9, S1).
            guard stepIndex.accepts(requestId) else { return }
            let mark = generation == found
                ? AssistantCommitMark(stage: stage, ok: ok, generation: generation, failed: failed)
                : AssistantCommitMark(stage: stage, ok: false, generation: found)
            enqueueMark(requestId: requestId, step: step, mark: mark)
        }
    }

    /// Stil `natural` mit `AssistantConfig.fillers`: Die Äußerung dieser Anfrage zählt (die Steuerung ruft es, sobald sie die Äußerung
    /// als Frage übernimmt, nicht schon bei der Vorarbeit). Der Lauf merkt sich den Zeitpunkt; ab da laufen die Zeitgeber für die
    /// Ansagen bei einer langsamen Antwort (`AssistantConfig.fillerFirstMs`, `fillerSecondMs`). Ohne diesen Aufruf gibt es keine
    /// solchen Ansagen (Vorarbeit, ältere Aufrufer). Der erste Aufruf je Kennung gilt. Ohne Warten und ohne Aktor-Sprung, aus jedem
    /// Thread; im Stil `verbatim` und ohne `fillers` (Standard) ohne Wirkung. Kommt der Hinweis, nachdem schon Einheiten hinausgingen, entfällt die erste Ansage (sie gilt nur, solange noch nichts
    /// zu hören war); die zweite gilt, solange noch keine Antworteinheit hinausging.
    public nonisolated func noteCommitted(requestId: String) {
        guard config.style == .natural, config.fillers else { return }
        commitNotes.note(requestId)
    }

    /// Nur Tests: Zahl der gemerkten Zeitpunkte und angemeldeten Beobachter (nach dem Ende aller Anfragen müssen beide 0 sein).
    nonisolated var commitNoteCountsForTesting: (times: Int, observers: Int) { commitNotes.counts }

    /// Hat das Protokoll dieser Anfrage schon einen Schreibfehler gemeldet? Ohne Warten, aus jedem Thread (Controller: Ausgabe sofort
    /// stoppen).
    public nonisolated func hasAuditFailure(requestId: String) -> Bool { audit?.hasFailed(request: requestId) ?? false }

    /// Die Marke des Verlaufs je ausgegebener Einheit (nur im Lock der Ordnung, zusammen mit dem Anhängen oder der Ablehnung).
    private nonisolated func enqueueHistoryMarks(requestId: String, steps: [Int], included: Set<Int>, generation: Int) {
        for step in steps {
            enqueueMark(requestId: requestId, step: step,
                        mark: AssistantCommitMark(stage: .history, ok: included.contains(step), generation: generation))
        }
    }

    private nonisolated func enqueueMark(requestId: String, step: Int?, mark: AssistantCommitMark) {
        guard let audit, let n = step else { return }
        var line = AssistantJournalLine(.commit, req: requestId)
        line.n = n
        line.mark = mark
        audit.enqueue(line)
    }

    /// Wartet, bis alle Zeilen der Anfrage geschrieben sind, und sagt, ob das Protokoll in Ordnung ist. `false`: ein Schreibfehler
    /// (der Controller stoppt die Ausgabe und zeigt „Protokoll nicht schreibbar“). Der Fehler bleibt für `commit` gemerkt.
    public nonisolated func auditHealthy(requestId: String) async -> Bool {
        guard let audit else { return true }
        return await audit.waitUntilWritten(request: requestId) == nil
    }

    // MARK: Verlauf

    public func restore(history stored: [AssistantTurn]) {
        let turns = AssistantTurn.admitted(stored, released: released)
        history = turns.count > Self.maxStoredTurns ? Array(turns.suffix(Self.maxStoredTurns)) : turns
        context.reset(optIn: release.isOptedIn, history: history)
    }

    public func clearHistory() {
        history.removeAll()
        context.reset(optIn: release.isOptedIn, history: [])
    }

    /// Cloud-Schutz: Die Beiträge, die als VERLAUF ins Modell dürfen. Lokal alle; bei einem Cloud-Modell nur Beiträge ohne
    /// Quellen oder mit Quellen ausschließlich aus den für die Cloud freigegebenen Bereichen (die Bereiche dieser Anfrage aus
    /// dem Freigabezustand, dieselben wie beim Abruf): Gesprochenes aus einem gesperrten Bereich verlässt das Gerät nicht.
    /// `history` selbst (Anzeige, Datei) bleibt vollständig; die Auswahl gilt je Anfrage und folgt damit jeder Änderung.
    static func promptHistory(_ turns: [AssistantTurn], areas: Set<AssistantArea>?) -> [AssistantTurn] {
        guard let areas else { return turns }
        return turns.filter { turn in turn.sources.allSatisfy { areas.contains($0.area) } }
    }

    /// Die Einheiten, die in den Beitrag kommen (Runde 8, R1): Aus dem Stand der Anfrage ausgewählt, nie aus Text abgeleitet. Ansagen
    /// zählen nicht. Reihenfolge der Ausgabe. Eine Einheit ohne vollständige Marke (ein Zitat ohne Quelle, ein Satz ohne Kennzeichnung)
    /// kommt nicht in den Verlauf: Sie bekäme dort nie eine Ansicht als Auszug oder als Aussage ohne Marke.
    static func units(of record: AssistantRunRecord, spoken: AssistantSpokenUnits, stepIndex: AssistantStepIndex, requestId: String)
        -> [AssistantEmitted] {
        let candidates = record.sentences.filter { $0.status != .announcement && $0.isRepresentable }
        switch spoken {
        case .nothing:
            return []
        case .all:
            return candidates
        case .events(let indices):
            let wanted = Set(indices.compactMap { stepIndex.step(requestId, index: $0) })
            return candidates.filter { wanted.contains($0.step) }
        }
    }

    private static func distinct(_ passages: [AssistantPassage]) -> [AssistantPassage] {
        var seen = Set<String>()
        return passages.filter { seen.insert("\($0.area.rawValue)/\($0.itemId)/\($0.locator ?? "")").inserted }
    }

    /// Wartet (begrenzt) auf den geordneten Abschluss des Laufs: erst Abbruch, dann Endstand.
    private func settle(_ requestId: String) async -> RunState? {
        guard let run = active[requestId] else { return forget(requestId) }
        run.state.markCancelled()
        run.stop()
        let deadline = ContinuousClock.now.advanced(by: commitWait)
        // Runde 8 (Suchlauf d): `try?` ist hier begründet. Der Schlaf ist nur eine Pause zwischen zwei Prüfungen; ist der Aufrufer
        // abgebrochen, wirft er sofort, setzt aber trotzdem aus und gibt den Aktor ab (gemessen mit
        // `testCommitFromACancelledCallerStillWaitsForTheOrderedEndOfTheRun`), und die Frist begrenzt die Schleife in jedem Fall.
        while active[requestId] != nil, ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
        if active[requestId] != nil { return run.state }   // hängendes Modell: Stand bis hierher
        return forget(requestId)
    }

    private func forget(_ id: String) -> RunState? {
        resultOrder.removeAll { $0 == id }
        return results.removeValue(forKey: id)
    }

    /// Die Zeile `start` einer Anfrage (ohne Inhalte).
    private static func startLine(_ rid: String, session: String, config: AssistantConfig, record r: AssistantRunRecord) -> AssistantJournalLine {
        var l = AssistantJournalLine(.start, req: rid)
        l.at = r.startedAt
        l.session = session
        l.model = AssistantModelKind.classify(label: config.modelLabel)
        l.cloud = config.isCloud
        l.unsourced = r.optInAtStart
        l.consent = r.consentVersion
        l.ctx = r.contextBefore
        l.reason = r.contextReason
        l.round = r.decided ? r.round : nil
        l.gen = r.generation
        // Nur der Stil `natural` steht in der Zeile; für `verbatim` bleibt die Zeile Byte für Byte, wie sie war.
        l.style = config.style == .natural ? .natural : nil
        return l
    }

    /// Nach `done` oder nach Barge-in (dann `spoken` = die Einheiten, die ganz gesprochen wurden, `interrupted` = true) trägt die Steuerung
    /// den Beitrag ein. Sie übergibt EINHEITEN (`AssistantSpokenUnits`), keinen Text: Der Beitrag besteht aus den ausgegebenen Einheiten
    /// selbst (Art, Text unverändert, Quelle), ganz oder gar nicht; nichts wird getrimmt, gekürzt oder neu zerlegt (Runde 8, R1).
    /// Für JEDE Anfrage entsteht ein abgeschlossenes Protokoll (`start` … `end`), auch ohne gesprochenen Text
    /// (Fehler, Abbruch, Widerruf); einen Beitrag im Verlauf gibt es nur mit Nutzertext, mit tatsächlich Gesprochenem, mit
    /// bekanntem Lauf und bei noch gültiger Freigabe-Generation: Die Prüfung, das Anhängen UND die Ergebniszeile
    /// (`h+<Generation>`) geschehen im selben Lock wie die Erhöhung der Generation; eine Ablehnung trägt die dabei gefundene
    /// Generation. Läuft die Anfrage noch, bricht `commit` sie ab und wartet auf ihren Endstand, bevor es schreibt. Quellen (nur
    /// die, die einen gesprochenen Satz decken) kommen aus der Prüfung dieser Anfrage. Mit dem Commit endet die Anmeldung des
    /// Laufs für den Widerruf.
    ///
    /// Lebenslauf (Runde 10): offen → geschlossen → abgeschlossen, EIN Mechanismus (`AssistantStepIndex`). Der Abschluss gehört genau
    /// EINEM Aufrufer (L1): Wer ihn nicht bekommt (zweiter `commit`, Abschluss durch Verdrängung nach dem der Steuerung, Aufruf nach einem
    /// hängenden Lauf), kehrt sofort mit `true` zurück und schreibt nichts. Mit dem Besitz ist die Anfrage geschlossen (L2): Erzeuger und
    /// Controller reihen nichts mehr ein und senden nichts mehr; nur die Zeilen des Abschlusses selbst (`start` falls nie bestätigt,
    /// `h`-Marken, `end`) kommen noch. Der Ausgang der Schlusszeile wird erst festgelegt, wenn die Schreibergebnisse der Zeilen davor
    /// bekannt sind (L3), und ist derselbe, den dieser Rückgabewert der Steuerung meldet. Reihenfolge am Ende der Anfrage: erst
    /// schließen (im Lock der Ordnung), dann die Schlusszeile, dann der Anspruch auf ihren Fehler. `false`, wenn das Protokoll nicht
    /// geschrieben werden konnte; die Anfrage endet dann mit Ausgang „Fehler“. Der Beitrag steht dann nur im Verlauf, wenn der Fehler
    /// erst nach seinem Anhängen entstand (S2: ein Fehler, der schon feststeht, verhindert ihn).
    @discardableResult
    public func commit(requestId: String, user: String, spoken: AssistantSpokenUnits, interrupted: Bool, revoked: Bool = false) async -> Bool {
        // Eine Anfrage, die diese Sitzung (noch) nicht kennt (ihr Lauf ist nie angekommen): Es gibt nichts abzuschließen und nichts zu
        // schreiben. Kommt ihr Lauf doch noch, endet er als Abbruch vor dem Start und bekommt dort seinen Eintrag (`start`, `end`); so hat
        // jede Anfrage genau einen Protokolleintrag, auch wenn ihr Abschluss den Lauf überholt. Eine schon abgeschlossene Kennung bleibt
        // geschlossen (ihr Lauf wird abgewiesen). Zwischen dieser Prüfung und dem Besitz liegt kein Wartepunkt.
        guard active[requestId] != nil || results[requestId] != nil else {
            if stepIndex.isOpen(requestId) { earlyCancel.insert(requestId) }
            return true
        }
        guard claimCompletion(requestId) else { return true }
        // Ein laufender Lauf wird abgebrochen und, wenn er nicht mehr endet, nach der Frist zurückgelassen.
        if let run = active[requestId] {
            run.state.markCancelled()
            run.stop()
        }
        guard let state = await settle(requestId) else { return true }   // unbekannte Anfrage (nichts gelaufen, nichts zu schreiben)
        // Ein hängender Lauf, den `settle` nicht abwarten konnte: Sein spätes Ende nimmt die Anfrage nicht noch einmal ins Gedächtnis
        // (sie ist abgeschlossen, ihr Anspruch beendet).
        if active[requestId] != nil { finishedWhileRunning.insert(requestId) }
        return await finishRequest(requestId, state: state, user: user, spoken: spoken, interrupted: interrupted, revoked: revoked)
    }

    /// Der Anspruch der Schreibfehler der Zeile `start` (und `end`), die der Abschluss selbst schreibt: eigene Schlüssel, damit der
    /// Fehler genau dieser Zeile erkennbar ist (der der Anfrage hängt am Anspruch der Anfrage).
    private static func startKey(_ requestId: String) -> String { "\(requestId)#start" }
    private static func endKey(_ requestId: String) -> String { "\(requestId)#end" }

    /// Runde 11, V1: `true`, wenn die Zeile `start` der Anfrage dauerhaft in der Datei steht. Gelesen wird der Stand des Laufs JETZT (nicht
    /// eine frühe Kopie): Hat der Erzeuger seine Zeile eingereiht (`startQueued`), wartet der Abschluss auf DIESE Zeile, statt eine zweite
    /// zu schreiben (`waitUntilWritten` deckt alles bis hierher Eingereihte der Anfrage; ein Fehler dort lässt den Abschluss die
    /// Startzeile gemeinsam mit `end` wiederholen: Doppel statt Verlust, der Leser faltet sie). Hat er sie nie eingereiht (Abbruch vor dem
    /// Start, Abschluss vor der Startzeile, Anfrage schon geschlossen), schreibt der Abschluss sie unter eigenem Schlüssel.
    private func secureStart(_ audit: AssistantAuditLog, _ requestId: String, state: RunState, record: AssistantRunRecord) async -> Bool {
        let now = state.snapshot
        if now.journaled { return true }
        if now.startQueued { return await audit.waitUntilWritten(request: requestId) == nil }
        audit.enqueue(Self.startLine(requestId, session: sessionId, config: config, record: record),
                      key: Self.startKey(requestId), run: runToken)
        return await audit.waitForLine(key: Self.startKey(requestId)) == nil
    }

    /// Der Abschluss selbst (Besitz und Schließen: `commit`). Schreibt, in dieser Reihenfolge: Standzeile (falls nicht bestätigt) und `start`
    /// (falls der Lauf es nie bestätigt hat), die Marken des Verlaufs, die Schlusszeile; gibt den Anspruch auf den Fehler der Anfrage erst
    /// danach frei. Der Ausgang wird aus den Fehlerquellen selbst bestimmt, zum Zeitpunkt der Entscheidung (Runde 11, V2), nie aus einer
    /// früh gezogenen Kopie: (1) klebrige Schreibfehler der Anfrage (`waitUntilWritten`), (2) die Standzeile (`ensure`), (3) der Stand des
    /// Laufs frisch gelesen. Zu (1) zählt seit Runde 12 (W1) auch der Fehler eines Standzeilenversuchs, den die Anfrage geteilt hat: Er
    /// steht an ihrem Anspruch, bevor der Versuch freigegeben wird, und bleibt es, auch wenn die Wiederholung hier unter (2) gelingt.
    /// Die frühe Kopie `record` dient nur dem Inhalt (Einheiten, Marken, Bereiche), der nach dem Schließen feststeht:
    /// Gesendetes wird im Abschnitt der Ordnung gebucht, und danach geht nichts mehr hinaus.
    private func finishRequest(_ requestId: String, state: RunState, user: String, spoken: AssistantSpokenUnits, interrupted: Bool,
                               revoked: Bool) async -> Bool {
        release.unregister(requestId)
        commitNotes.forget(requestId)
        let record = state.snapshot
        let text = user.trimmingCharacters(in: .whitespacesAndNewlines)
        let units = Self.units(of: record, spoken: spoken, stepIndex: stepIndex, requestId: requestId)
        await stepIndex.reach(requestId, .closing)

        // Standzeile und `start` zuerst, abgewartet: Die Zeilen der Anfrage stehen in der Reihenfolge state, start, …, h, end. Der Abschluss
        // stellt die Standzeile SELBST sicher (`ensure`, Schreibfolge 1/9: ohne dauerhafte Standzeile kein gültiger Anfrageeintrag), bevor
        // er `start` schreibt, und liest ihr Ergebnis hier, bei der Entscheidung (Runde 11, V2): Eine frühe Kopie des Laufstandes sähe den
        // Fehler dieses Versuchs nicht. Der Fehler eines Versuchs, den die Anfrage geteilt hat, steht außerdem an ihrem Anspruch (Runde 12,
        // W1) und wirkt weiter unten über `hasFailed` und `waitUntilWritten`, auch wenn die Wiederholung hier gelingt. Steht sie nicht oder
        // ließ sich `start` nicht
        // schreiben, endet die Anfrage mit Ausgang „Fehler“ und `lost`, ohne Beitrag und ohne Marken; die Schlusszeile bekommt ihre
        // Startzeile (Gruppe, kein `end` nach einem fehlgeschlagenen Anhängen des `start`). Ein `start`, das stehen blieb, kann ohne `end`
        // bleiben (dann „unvollständig“). Der Leser faltet doppelte Startzeilen zu EINEM Eintrag.
        if let audit {
            let stateStands = await stateLine.ensure(audit, request: requestId)
            let startStands = await secureStart(audit, requestId, state: state, record: record)
            if !stateStands || !startStands {
                var end = Self.endLine(requestId, record: record, outcome: .failed)
                end.lost = true
                var group: [(line: AssistantJournalLine, key: String)] = [(line: end, key: Self.endKey(requestId))]
                if !startStands {
                    group.insert((line: Self.startLine(requestId, session: sessionId, config: config, record: record),
                                  key: Self.startKey(requestId)), at: 0)
                }
                _ = await audit.writeGroup(group, run: runToken, dependent: true)
                _ = await audit.flush(request: requestId)
                audit.release(request: requestId)
                return false
            }
        }

        var historyRejected = false
        // Ergebnis des Verlaufs JE EINHEIT (Runde 7, B4): Jede ausgegebene Einheit bekommt ihre Marke `h+g` (steht im Beitrag) oder `h-g`
        // (nicht im Beitrag: nicht gesprochen, abgelehnt oder gar kein Beitrag), in demselben Lock-Schritt wie das Anhängen.
        let outputSteps = record.outputSteps
        if text.isEmpty || units.isEmpty {
            release.inSection { found in
                enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: [], generation: found)
            }
        }
        if !text.isEmpty, !units.isEmpty {
            let included = Set(units.compactMap { $0.step > 0 ? $0.step : nil })
            let sources = Self.distinct(units.filter { $0.status.coversSources }.flatMap(\.sources))
            let anyUnsourced = units.contains { $0.status == .unsourced }
            // Abschnitte: IMMER (Runde 8, R1): Jede Einheit trägt ihre Marke (Zitat mit Quellzeile, Kennzeichnung, „Ohne Quelle“), auch ein
            // Beitrag nur aus Sätzen ohne Quelle. Ein Beitrag ohne Abschnitte gibt es nicht; die Ansicht zeigt nie bloßen Text.
            var turnParts: [AssistantTurnPart] = []
            for u in units {
                AssistantTurnPart.append(to: &turnParts, text: u.text, status: u.status, quote: u.quote, sources: u.sources)
            }
            let turn = AssistantTurn(
                user: text, assistant: units.map(\.text).joined(separator: " "), interrupted: interrupted, sources: sources,
                general: sources.isEmpty, unsourced: anyUnsourced, parts: turnParts, model: config.modelLabel, at: Date(),
                round: record.decided ? record.round : nil, style: config.style == .natural ? .natural : nil)
            // Prüfung der Generation, Anhängen und Ergebniszeile im selben Lock wie die Erhöhung der Generation: Ein Widerruf
            // dazwischen ist ausgeschlossen, die Marke trägt genau die geprüfte Generation.
            if !revoked, !record.revoked {
                // Runde 9, S2: Ein Schreibfehler des Protokolls dieser Anfrage ist klebrig und geht vor allem anderen, auch vor dem
                // Beitrag (wie bei Anzeige und Audiostart): Die Prüfung steht IM Lock, vor dem Anhängen. Dann entsteht kein Beitrag; die
                // Marke des Beitrags (Schritt 0) sagt „fehlgeschlagen“ mit der geprüften Generation, jede Einheit „nicht im Verlauf“, und
                // die Anfrage endet mit dem Ausgang „Fehler“.
                var auditBlocked = false
                let check = release.ifCurrent(record.generation, onStale: { found in
                    self.enqueueMark(requestId: requestId, step: 0, mark: AssistantCommitMark(stage: .history, ok: false, generation: found))
                    self.enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: [], generation: found)
                }) {
                    if audit?.hasFailed(request: requestId) == true {
                        auditBlocked = true
                        enqueueMark(requestId: requestId, step: 0,
                                    mark: AssistantCommitMark(stage: .history, ok: false, generation: record.generation, failed: true))
                        enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: [], generation: record.generation)
                        return
                    }
                    history.append(turn)
                    if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }
                    enqueueMark(requestId: requestId, step: 0, mark: AssistantCommitMark(stage: .history, ok: true, generation: record.generation))
                    enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: included, generation: record.generation)
                }
                historyRejected = !check.ok || auditBlocked
            } else {
                historyRejected = true
                // Schon widerrufen: Die Ablehnung steht ebenfalls im Lock, mit der dort vorgefundenen Generation.
                release.inSection { found in
                    enqueueMark(requestId: requestId, step: 0, mark: AssistantCommitMark(stage: .history, ok: false, generation: found))
                    enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: [], generation: found)
                }
            }
        }

        guard let audit else { return true }
        // Ein Schreibfehler irgendeiner Zeile dieser Anfrage (auch einer Ergebniszeile des Controllers) ist klebrig: Ausgang „Fehler“.
        let writeFailed = await audit.waitUntilWritten(request: requestId) != nil
        // Der Stand des Laufs JETZT (Runde 11, V2): Ein Lauf, den die Frist zurückließ, kann seinen Fehler erst nach der frühen Kopie
        // übernehmen (`run` setzt `failure` am Ende). Zwischen dieser Zeile und dem Einreihen der Schlusszeile liegt kein Wartepunkt.
        let now = state.snapshot
        let outcome: AssistantAuditEntry.Outcome
        if writeFailed || now.failure == .auditUnavailable {
            outcome = .failed   // ein Schreibfehler geht dem Widerruf voran (Schreibfolge 7: nichts wird ausgegeben, Ausgang „Fehler“)
        } else if revoked || now.revoked || now.failure == .revoked || historyRejected {
            outcome = .revoked   // auch ein fertiger Lauf, dessen Verlaufs-Commit die geänderte Freigabe verwirft
        } else if let failure = now.failure {
            outcome = failure == .cancelled ? .interrupted : .failed
        } else if now.limited {
            outcome = .limited
        } else {
            outcome = interrupted || now.cancelled ? .interrupted : .completed
        }
        var end = Self.endLine(requestId, record: record, outcome: outcome)
        // Ließ sich eine Zeile der Anfrage nicht schreiben, steht das am Ende: Was dann in der Datei fehlt, ist „unbekannt“ (R7).
        if writeFailed { end.lost = true }
        audit.enqueue(end)
        let tail = await audit.flush(request: requestId)
        audit.release(request: requestId)
        // L3: Was die Schlusszeile als „Fehler“ ausweist, meldet auch der Rückgabewert (nicht nur ein Schreibfehler dieser Anfrage, auch
        // ein Lauf, der wegen des Protokolls endete, etwa weil die Standzeile nicht schrieb).
        return !writeFailed && tail == nil && now.failure != .auditUnavailable
    }

    /// Die Schlusszeile einer Anfrage (ohne `lost`).
    private static func endLine(_ requestId: String, record: AssistantRunRecord, outcome: AssistantAuditEntry.Outcome) -> AssistantJournalLine {
        var end = AssistantJournalLine(.end, req: requestId)
        end.at = Date()
        end.outcome = outcome
        end.level = AssistantAuditLevel.of(record.counts)
        let covering = distinct(record.sentences.filter { $0.status.coversSources }.flatMap(\.sources)).map(\.area)
        end.areas = Set((covering + record.inventoryAreas).map(\.rawValue)).sorted()
        return end
    }

    // MARK: Abbruch

    /// Bricht eine laufende Anfrage ab: nur noch `.failed(.cancelled)`, nichts wird gespeichert. Ein Abbruch, der vor dem
    /// Start der Anfrage eintrifft, wirkt beim Start.
    public func cancel(requestId: String) {
        if let run = active[requestId] {
            run.state.markCancelled()
            return run.stop()
        }
        // Runde 9 (Suchlauf a): kein Verdrängen mehr (früher: ab 64 Einträgen alle vergessen, und eine abgebrochene Anfrage lief dann
        // doch). Ein Eintrag ist eine kurze Kennung und wird vom Start seiner Anfrage verbraucht; ein Abbruch ohne spätere Anfrage bleibt
        // bis zum Ende der Sitzung liegen (Bytes je Tippen auf „Stopp“).
        earlyCancel.insert(requestId)
    }

    // MARK: Strom

    /// Antwort streamen. Schreibt NICHT in den Verlauf; das entscheidet die Steuerung über `commit`. Endet mit `.done` oder
    /// `.failed`; wer den Strom nicht mehr liest (Abbruch des Tasks), bricht die Anfrage ab.
    public nonisolated func events(_ request: AssistantRequest) -> AsyncStream<AssistantEvent> {
        AsyncStream { continuation in
            let task = Task { await self.run(request, out: continuation) }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func run(_ request: AssistantRequest, out: AsyncStream<AssistantEvent>.Continuation) async {
        defer { out.finish() }
        let rid = request.requestId
        // Generation, Opt-in, Bereiche und Cloud-Zustimmung in EINEM Lesen: Alles, was diese Anfrage später liest (Abruf,
        // Bestand, Prüfung, Verlauf für die Cloud), stammt aus diesem Stand. Ändert sich die Freigabe danach, ist die Generation
        // veraltet und der Lauf wird beim Anmelden widerrufen. Vor der Abbruchprüfung, damit zwischen Prüfung und Eintrag in
        // `active` kein Wartepunkt liegt.
        // Bestand und Zugang gehören dazu: Gesperrt, blockiert oder `stale` (Klasse B) sind die Bereiche dieser Anfrage leer.
        let snapshot = self.release.snapshot.forRequest
        let allowedAreas: Set<AssistantArea>? = self.config.isCloud ? snapshot.areas : nil
        // Runde 10: Eine Kennung, die schon geschlossen ist (abgeschlossen, verdrängt, früher unbekannt abgeschlossen), läuft nicht mehr:
        // kein Anspruch, keine Zeile, kein Ereignis mit Inhalt. (Zwischen dieser Prüfung und der Anmeldung in `active` liegt kein Wartepunkt.)
        guard stepIndex.isOpen(rid) else {
            out.yield(.failed(requestId: rid, .cancelled))
            return
        }
        // Runde 8, R6: Der Fehler jeder Zeile dieser Anfrage hängt ab hier an ihrem Anspruch (nie verdrängt, bis `commit` ihn beendet).
        audit?.claim(request: rid)
        if earlyCancel.remove(rid) != nil {
            // Ein Abbruch vor dem Start ist ein Abschluss wie jeder andere (Runde 10, T3): derselbe Code (`finishRequest`: `start` schreiben und
            // abwarten, bei einem Fehler `start` und `end` als Gruppe, sonst `end` mit dem Ausgang „abgebrochen“), nicht ein zweiter Pfad. Das
            // Ereignis an die Steuerung ist der Ausgang der Schlusszeile: „abgebrochen“, wenn alles dauerhaft steht, sonst „Protokoll nicht
            // schreibbar“. Der Anspruch endet erst danach.
            let state = RunState(snapshot: snapshot)
            state.update {
                $0.cancelled = true
                $0.failure = .cancelled
            }
            guard claimCompletion(rid) else {
                out.yield(.failed(requestId: rid, .cancelled))
                return
            }
            let stands = await finishRequest(rid, state: state, user: "", spoken: .nothing, interrupted: true, revoked: false)
            out.yield(.failed(requestId: rid, stands ? .cancelled : .auditUnavailable))
            return
        }
        var runConfig = self.config
        runConfig.allowUnsourced = snapshot.optIn
        let model = self.model, knowledge = self.knowledge, release = self.release, context = self.context
        let audit = self.audit, index = self.stepIndex, canOptIn = self.released, stateLine = self.stateLine, runToken = self.runToken
        let promptHistory = Self.promptHistory(history, areas: allowedAreas)
        let state = RunState(snapshot: snapshot)
        let output = AssistantRunOutput(release: release, generation: snapshot.generation, continuation: out,
                                        gate: AssistantRequestGate(request: rid, index: index, audit: audit))
        let bridges = self.bridges, commitNotes = self.commitNotes
        let work = Task<Outcome, Never> {
            // Stil `natural`: eigener Pfad neben `generate` (Sprecher, Suche, Denker); `verbatim` bleibt der bisherige.
            if runConfig.style == .natural {
                return await Self.generateNatural(
                    request, model: model, knowledge: knowledge, config: runConfig, snapshot: snapshot, release: release,
                    context: context, history: promptHistory, state: state, output: output, audit: audit, index: index,
                    session: self.sessionId, stateLine: stateLine, run: runToken, catchUp: { await self.catchUp() },
                    bridges: bridges, commitNotes: commitNotes)
            }
            return await Self.generate(request, model: model, knowledge: knowledge, config: runConfig, snapshot: snapshot, release: release,
                                       context: context, history: promptHistory, state: state, output: output, audit: audit,
                                       index: index, session: self.sessionId, canOptIn: canOptIn, stateLine: stateLine, run: runToken,
                                       catchUp: { await self.catchUp() })
        }
        active[rid] = ActiveRun(stop: { work.cancel() }, state: state)
        let revoke: @Sendable () -> Void = { state.markRevoked(); work.cancel() }
        if !release.register(rid, generation: snapshot.generation, revoke: revoke) { revoke() }
        let timeout = runConfig.timeout
        let watchdog = Task {
            guard timeout > 0 else { return }
            let step = UInt64(max(0.01, min(timeout / 4, 0.5)) * 1_000_000_000)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: step)
                if Task.isCancelled { return }
                if state.idle > timeout {
                    state.fireTimeout()
                    work.cancel()
                    return
                }
            }
        }
        let outcome = await withTaskCancellationHandler { await work.value } onCancel: { work.cancel() }
        watchdog.cancel()
        // Der Lauf bleibt angemeldet, bis `commit` ihn abschließt: Ein Widerruf nach dem Modellende erreicht ihn noch.

        var failure: AssistantFailure?
        switch outcome {
        case .cancelled:
            let r = state.snapshot
            failure = r.revoked ? .revoked : (state.didTimeOut ? .timeout : .cancelled)
        case .finished(_, _, _, let f):
            failure = state.snapshot.revoked ? .revoked : (Task.isCancelled ? .cancelled : f)
        }
        state.update { $0.failure = failure }
        // Der Endstand steht fest, bevor das letzte Ereignis hinausgeht und bevor `commit` ihn liest.
        let evicted = remember(rid, state)
        active[rid] = nil

        if let failure {
            output.sendUnchecked(.failed(requestId: rid, failure))
        } else if case .finished(let text, let firstMs, let totalMs, _) = outcome {
            let record = state.snapshot
            let covering = Self.distinct(record.sentences.filter { $0.status.coversSources }.flatMap(\.sources))
            let done = AssistantAnswer(
                text: text, sources: covering, general: covering.isEmpty, firstSentenceMs: firstMs, totalMs: totalMs,
                unsourced: record.counts.unsourced > 0, counts: record.counts, round: record.round, generation: record.generation)
            // Das Senden prüft IM Lock auch den Lebenslauf: Ein später Lauf, dessen Anfrage schon abgeschlossen ist, sendet `done` nicht mehr.
            let sent = output.deliver(.done(requestId: rid, done))
            switch sent.refusal {
            case .auditFailed?:
                // Ein Schreibfehler zwischen dem Warten und dem Senden (klebrig): Auch `done` trägt den Text und geht nicht hinaus.
                state.update { $0.failure = .auditUnavailable }
                output.sendUnchecked(.failed(requestId: rid, .auditUnavailable))
            case .closed?:
                // Die Anfrage ist abgeschlossen (ihr Ausgang steht in der Schlusszeile): nur noch das Ende des Stroms, ohne Inhalt.
                output.sendUnchecked(.failed(requestId: rid, .cancelled))
            case nil:
                if !sent.check.ok {
                    state.markRevoked()
                    state.update { $0.failure = .revoked }
                    output.sendUnchecked(.failed(requestId: rid, .revoked))
                }
            }
        }
        // Runde 9, S1: Was das Gedächtnis verdrängt hat, ohne dass die Steuerung es abgeschlossen hat, wird JETZT abgeschlossen (nach den
        // Ereignissen dieser Anfrage; der Strom endet erst danach): kein Beitrag, `h-` je Einheit, Schlusszeile mit dem dann geltenden
        // Ausgang (bei einem Schreibfehler „Fehler“ und `lost`). Erst danach endet der Anspruch auf den Fehler der verdrängten Anfrage.
        for old in evicted { _ = await commit(requestId: old, user: "", spoken: .nothing, interrupted: true) }
    }

    /// Nimmt den Endstand ins Gedächtnis. Gibt die Anfragen zurück, die dabei verdrängt wurden: Sie sind schon für Commits geschlossen
    /// (im Lock der Ordnung) und stehen noch in `results`, bis ihr Abschluss sie holt; ihr Anspruch auf den Fehler endet NIE durch das
    /// Verdrängen, nur mit ihrem Abschluss (`commit`: Schlusszeile, dann Anspruch).
    private func remember(_ id: String, _ state: RunState) -> [String] {
        // Der Abschluss war schon da (hängendes Modell): nichts mehr zu merken.
        if finishedWhileRunning.remove(id) != nil { return [] }
        results[id] = state
        resultOrder.removeAll { $0 == id }
        resultOrder.append(id)
        var evicted: [String] = []
        while resultOrder.count > Self.maxRemembered {
            let old = resultOrder.removeFirst()
            closeForCommits(old)
            evicted.append(old)
        }
        return evicted
    }

    private static func generate(
        _ request: AssistantRequest, model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig,
        snapshot: AssistantRelease.Snapshot, release: AssistantRelease, context: AssistantContext, history: [AssistantTurn],
        state: RunState, output: AssistantRunOutput, audit: AssistantAuditLog?, index: AssistantStepIndex, session: String,
        canOptIn: Bool, stateLine: AssistantStateLine, run: AssistantRunToken, catchUp: @Sendable () async -> Void
    ) async -> Outcome {
        let rid = request.requestId
        let budget = AssistantPrompt.budget(config: config)
        let emitter = RunEmitter(rid: rid, snapshot: snapshot, release: release, output: output, state: state,
                                 journal: audit, index: index, started: ContinuousClock().now, canOptIn: canOptIn)
        // Die Bereiche dieser Anfrage: genau der Stand, mit dem die Generation gelesen wurde.
        let areas = snapshot.areas
        func fail(_ failure: AssistantFailure) -> Outcome {
            .finished(text: "", firstMs: nil, totalMs: emitter.elapsedMs, failure: failure)
        }
        func finished(_ failure: AssistantFailure? = nil) async -> Outcome {
            var f = failure
            // Alle Zeilen der Anfrage stehen in der Datei, bevor `done` hinausgeht; ein Fehler ersetzt jedes andere Ergebnis.
            if let audit, await audit.waitUntilWritten(request: rid) != nil { f = .auditUnavailable }
            return .finished(text: emitter.texts.joined(separator: " "), firstMs: emitter.firstMs, totalMs: emitter.elapsedMs, failure: f)
        }
        /// Ergebnis einer Pflichtzeile: `nil`, wenn sie steht; sonst das Ende des Laufs (geschlossen: abgebrochen, Schreibfehler: Fehler).
        func ended(_ landing: RunEmitter.Landing) -> Outcome? {
            switch landing {
            case .stands: nil
            case .closed: .cancelled
            case .failed: fail(.auditUnavailable)
            }
        }
        /// Ergebnis des Sendens von `sources`: `nil`, wenn es hinausging; sonst das Ende des Laufs.
        func ended(_ sent: AssistantDelivery) async -> Outcome? {
            switch sent.refusal {
            case .auditFailed?: return await finished(.auditUnavailable)
            case .closed?: return .cancelled
            case nil: return sent.check.ok ? nil : .cancelled
            }
        }

        // Pflichtzeilen, fail-closed: Die Standzeile des Öffnens, `start` (vor Abruf und Bestandstiteln) und später `round` und
        // `areas` stehen DAUERHAFT in der Datei, bevor das Modell läuft oder etwas ausgegeben wird (`await` auf das Schreiben,
        // nicht nur Einreihen), bei JEDEM Modell. Ein Fehler: kein Modell, keine Ausgabe, Ausgang „Fehler“. Ein Prozessende
        // nach `start` hinterlässt die Anfrage; das nächste Öffnen schließt sie als unvollständig.
        state.update { $0.contextBefore = context.mode }
        if let audit {
            await index.reach(rid, .beforeState)
            guard await stateLine.ensure(audit, request: rid) else {
                await index.reach(rid, .stateRefused)
                return fail(.auditUnavailable)
            }
            if Task.isCancelled { return .cancelled }
            // `start`: eingereiht nur, solange die Anfrage offen ist (L2), und abgewartet.
            let landing = await emitter.land(startLine(rid, session: session, config: config, record: state.snapshot), run: run)
            state.update { $0.journaled = landing == .stands }
            if let end = ended(landing) { return end }
            if Task.isCancelled { return .cancelled }
            // Nachholen bei jeder Gelegenheit: nicht gesicherte Einschränkung erneut sichern, fehlende Zeilen schreiben.
            await catchUp()
        }
        // Cloud-Ziel ohne Zustimmung (aus dem Stand dieser Anfrage): kein Abruf, kein Modell.
        if config.isCloud, !snapshot.cloudConsent { return fail(.consentRequired) }

        let passages = await knowledge.retrieve(question: request.text, previous: history.last?.user, limit: passageLimit,
                                                maxChars: budget.passages, areas: areas)
        if Task.isCancelled { return .cancelled }
        let titles = await knowledge.inventoryTitles(areas: areas)
        if Task.isCancelled { return .cancelled }
        state.touch()

        // Rundenart über den Datenkontext, vor dem Modellaufruf. Bestandsfragen sind immer Datenrunden.
        let inventoryQuestion = AssistantKnowledge.isInventoryQuestion(request.text)
        let decision = context.decide(question: request.text, titles: titles, passages: passages, optIn: snapshot.optIn,
                                      inventory: inventoryQuestion)
        let kind = decision.kind
        state.update {
            $0.round = kind
            $0.decided = true
            $0.contextBefore = decision.before
            $0.contextReason = decision.reason
        }
        if audit != nil {
            var l = AssistantJournalLine(.round, req: rid)
            l.round = kind
            l.reason = decision.reason
            l.gen = snapshot.generation
            await index.reach(rid, .beforeRound)
            if let end = ended(await emitter.land(l)) { return end }
            if Task.isCancelled { return .cancelled }
        }

        let now = Date()
        let clockSentences = AssistantGrounding.clockSentences(now: now, timeZone: .current)

        if inventoryQuestion {
            if let end = await ended(output.deliver(.sources(requestId: rid, generation: snapshot.generation, []))) { return end }
            let groups = await knowledge.inventoryGroups(question: request.text, areas: areas)
            if Task.isCancelled { return .cancelled }
            let statements = groups.flatMap(\.statements)
            if statements.isEmpty {
                switch await emitter.emitFixed(optIn: snapshot.optIn) {
                case .revoked: return .cancelled
                case .failed: return await finished(.auditUnavailable)
                default: return await finished()
                }
            }
            state.update { $0.inventoryAreas = groups.map(\.area) }
            // Die Bereiche der Bestandsangaben stehen in der Datei, bevor die erste Angabe hinausgeht.
            if let end = ended(await emitter.noteAreas(groups.map(\.area))) { return end }
            if Task.isCancelled { return .cancelled }
            let preface = AssistantGrounding.inventoryAnnouncement
            switch await emitter.emit(preface, speech: preface, status: .announcement) {
            case .revoked: return .cancelled
            case .failed: return await finished(.auditUnavailable)
            case .limit: return await finished()
            case .delivered: break
            }
            for s in statements {
                switch await emitter.emit(s, speech: emitter.speech(s), status: .inventory) {
                case .revoked: return .cancelled
                case .failed: return await finished(.auditUnavailable)
                case .limit: return await finished()
                case .delivered: break
                }
            }
            return await finished()
        }

        let instructions = kind == .data
            ? AssistantPrompt.dataInstructions(config: config, now: now, timeZone: .current)
            : AssistantPrompt.generalInstructions(config: config, now: now, timeZone: .current)
        // Der Prompt einer Runde ohne Quelle sieht nur Beiträge aus Runden ohne Quelle seit dem letzten `resumeFree`.
        let promptHistory = kind == .data ? history : context.freeHistory(history)
        var scale = 1.0
        var includeHistory = true
        var announcedSources = false
        var retried = false

        while true {
            var prompt: String
            var used: [AssistantPassage] = []
            if kind == .data {
                let built = AssistantPrompt.buildData(question: request.text, afterInterruption: request.afterInterruption, passages: passages,
                                                      history: promptHistory, config: config, passageScale: scale, includeHistory: includeHistory)
                prompt = built.prompt
                used = built.used
            } else {
                prompt = AssistantPrompt.buildGeneral(question: request.text, afterInterruption: request.afterInterruption,
                                                      history: promptHistory, config: config, includeHistory: includeHistory)
            }
            if !announcedSources || used != state.snapshot.used {
                state.update { $0.used = used }
                // Die Bereiche der Auszüge im Prompt stehen fest. Zu JEDEM Modell (lokal wie Cloud) geht der Prompt erst, wenn die
                // Zeile in der Datei steht.
                if let end = ended(await emitter.noteAreas(used.map(\.area))) { return end }
                if Task.isCancelled { return .cancelled }
                if let end = await ended(output.deliver(.sources(requestId: rid, generation: snapshot.generation, used))) { return end }
                announcedSources = true
            }
            let basis = AssistantGrounding.Basis(passages: used)

            var splitter = AssistantSpeech.Splitter(sentencesOnly: true)
            var seen = 0
            var failure: AssistantFailure?
            var flow = RunEmitter.Flow.next

            func check(_ sentences: [(text: String, speech: String, raw: String)]) async -> RunEmitter.Flow {
                for s in sentences {
                    seen += 1
                    let f: RunEmitter.Flow
                    if kind == .data {
                        f = await emitter.handleData(s.text, basis: basis, titles: titles, clock: clockSentences)
                    } else {
                        f = await emitter.handleGeneral(s.text, speech: s.speech, raw: s.raw, titles: titles, clock: clockSentences,
                                                        knowledge: knowledge)
                    }
                    if f != .next { return f }
                    if emitter.reachedLimit { return .limit }
                }
                return .next
            }

            do {
                for try await piece in model.stream(instructions: instructions, prompt: prompt, maxTokens: config.maxAnswerTokens) {
                    if Task.isCancelled { return .cancelled }
                    state.touch()
                    flow = await check(splitter.feed(piece))
                    if flow != .next { break }
                }
            } catch is CancellationError {
                return .cancelled
            } catch let e as BookTalkModelError {
                switch e {
                case .offline: failure = .offline
                case .unavailable: failure = .noModel
                case .timeout: failure = .timeout
                case .notAllowed: failure = .consentRequired
                case .contextOverflow:
                    if !retried, seen == 0 {
                        retried = true
                        scale = 0.5
                        includeHistory = false
                        continue
                    }
                    failure = .modelFailed
                }
            } catch {
                failure = .modelFailed
            }
            if Task.isCancelled { return .cancelled }
            if flow == .revoked { return .cancelled }
            if flow == .failed { return await finished(.auditUnavailable) }
            if failure == nil, flow == .next {
                // Stromende: der Rest ist der letzte Satz, aber nur mit Satzzeichen; ein abgeschnittener Rest wird verworfen.
                flow = await check(splitter.finish())
                if flow == .revoked { return .cancelled }
                if flow == .failed { return await finished(.auditUnavailable) }
                for _ in 0..<splitter.overlong { emitter.drop(.template) }
                for _ in 0..<splitter.fragments { emitter.drop(.fragment) }
                if seen == 0, splitter.overlong == 0, splitter.fragments == 0 { failure = .modelFailed }
            }
            if failure == nil, flow != .revoked, flow != .limit, emitter.dropped > 0 {
                // Nichts Belegtes blieb übrig: Rückfall gegen die Frage, sonst der feste Satz.
                if kind == .data, emitter.quotes == 0 {
                    let fallback = AssistantGrounding.fallback(question: request.text, basis: basis, taken: emitter.taken)
                    let f: RunEmitter.Flow
                    if let fallback { f = await emitter.emitQuote(fallback) } else { f = await emitter.emitFixed(optIn: snapshot.optIn) }
                    if f == .revoked { return .cancelled }
                    if f == .failed { return await finished(.auditUnavailable) }
                } else if kind == .general, emitter.unsourcedEmitted == 0 {
                    let f = await emitter.emitFixed(optIn: true)
                    if f == .revoked { return .cancelled }
                    if f == .failed { return await finished(.auditUnavailable) }
                }
            }
            return await finished(failure)
        }
    }

    // MARK: Stil natural

    /// Stil `natural` (ADR-0028 E4.2c): eigener Lauf neben `generate`. Eine Anfrage hat bis zu zwei PHASEN, beide am selben Stand
    /// (Generation, Bereiche, Cloud-Zustimmung aus EINEM Lesen), mit denselben Regeln für Widerruf, Abbruch und Schreibfehler; nach einem
    /// Absturz wird nichts fortgesetzt.
    /// - Bestandsfrage: nur Datenphase (Grund `inventory`), das System antwortet aus den Bestandsdaten, kein Modell.
    /// - Direkter Inhaltsbezug (`AssistantOwnData.refers`): nur Datenphase (Grund `naturalSearch`), kein Sprecher.
    /// - Sonst SPRECHER (Rundenzeile `.general`/`naturalTalk`, Bereichszeile der Verlaufsbeiträge im Prompt, leer möglich): antwortet
    ///   direkt (`.conversational`) oder verlangt in einer Zeile `SUCHE: …` eine Suche, mit oder ohne EINEN kurzen Zwischensatz davor
    ///   (der Satz geht sofort als `.conversational` hinaus, die Steuerzeile nie); dann Datenphase.
    /// - Datenphase: Rundenzeile `.data`/`naturalSearch` MIT den zulässigen Abrufbereichen (dauerhaft, bevor der Abruf läuft), Überbrückung
    ///   (nur hier, nur mit `bridge`), Abruf mit kleinem Budget, keine Auszüge → fester Satz; sonst Bereichszeile der verwendeten Auszüge
    ///   und DENKER (`.summarized`, Quellen = genau die Auszüge im Prompt).
    /// Jede Einheit geht über `RunEmitter.emit` (Einordnung dauerhaft, Generationsprüfung, Satzspur). Ein Ereignisstrom
    /// (`AssistantNaturalMailbox`) mit Modellstücken und Weckern, EIN Verbraucher, der nacheinander verarbeitet. Feste Ansagen für eine
    /// langsame Antwort (`AssistantConfig.fillers`) und der Überbrückungssatz (`bridge`) sind standardmäßig aus.
    /// Daneben gehen Fortschrittsereignisse (`AssistantProgress`) an die Denk-Anzeige: nur Anzeige, nie ins Protokoll und nie in den
    /// Verlauf; sie nehmen denselben Weg wie `sources` (Generation, offene Anfrage), nach Abbruch und Widerruf kommt keines mehr.
    private static func generateNatural(
        _ request: AssistantRequest, model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig,
        snapshot: AssistantRelease.Snapshot, release: AssistantRelease, context: AssistantContext, history: [AssistantTurn],
        state: RunState, output: AssistantRunOutput, audit: AssistantAuditLog?, index: AssistantStepIndex, session: String,
        stateLine: AssistantStateLine, run: AssistantRunToken, catchUp: @Sendable () async -> Void,
        bridges: AssistantBridgeRotation, commitNotes: AssistantCommitNotes
    ) async -> Outcome {
        let rid = request.requestId
        let emitter = RunEmitter(rid: rid, snapshot: snapshot, release: release, output: output, state: state,
                                 journal: audit, index: index, started: ContinuousClock().now, canOptIn: false)
        let areas = snapshot.areas
        let now = Date()

        // Der eine Ereignisstrom und die Zeitgeber der Ansagen. Der Hinweis „Äußerung gilt“ kann vor, während oder nach dem Start des
        // Laufs eintreffen (`AssistantCommitNotes`: genau einmal gemeldet). Alles endet mit dem Lauf (Abbruch, Widerruf, Ende).
        let mailbox = AssistantNaturalMailbox()
        // Ohne `fillers` (Standard) gibt es keine Zeitgeber und keine Anmeldung: der Kern gibt keine festen Ansagesätze aus.
        let fillers = AssistantNaturalFillers(firstMs: config.fillers ? config.fillerFirstMs : -1,
                                              secondMs: config.fillers ? config.fillerSecondMs : -1) { mailbox.wake() }
        if config.fillers { commitNotes.attach(rid) { fillers.start(commit: $0) } }
        defer {
            commitNotes.detach(rid)
            fillers.shutdown()
            mailbox.close()
        }
        // EIN Iterator für die ganze Anfrage (ein verworfener Iterator beendet den Strom).
        var events = mailbox.stream.makeAsyncIterator()

        func fail(_ failure: AssistantFailure) -> Outcome {
            .finished(text: "", firstMs: nil, totalMs: emitter.elapsedMs, failure: failure)
        }
        func finished(_ failure: AssistantFailure? = nil) async -> Outcome {
            var f = failure
            // Alle Zeilen der Anfrage stehen in der Datei, bevor `done` hinausgeht; ein Fehler ersetzt jedes andere Ergebnis.
            if let audit, await audit.waitUntilWritten(request: rid) != nil { f = .auditUnavailable }
            return .finished(text: emitter.texts.joined(separator: " "), firstMs: emitter.firstMs, totalMs: emitter.elapsedMs, failure: f)
        }
        /// Ergebnis einer Pflichtzeile: `nil`, wenn sie steht; sonst das Ende des Laufs.
        func ended(_ landing: RunEmitter.Landing) -> Outcome? {
            switch landing {
            case .stands: nil
            case .closed: .cancelled
            case .failed: fail(.auditUnavailable)
            }
        }
        /// Ergebnis des Sendens von `sources`: `nil`, wenn es hinausging; sonst das Ende des Laufs.
        func ended(_ sent: AssistantDelivery) async -> Outcome? {
            switch sent.refusal {
            case .auditFailed?: return await finished(.auditUnavailable)
            case .closed?: return .cancelled
            case nil: return sent.check.ok ? nil : .cancelled
            }
        }
        /// Ergebnis einer Ausgabe: `nil`, wenn die Einheit hinausging; sonst das Ende des Laufs.
        func ended(_ delivery: RunEmitter.Delivery) async -> Outcome? {
            switch delivery {
            case .delivered: nil
            case .revoked: .cancelled
            case .failed: await finished(.auditUnavailable)
            case .limit: await finished()
            }
        }
        func failure(of error: any Error) -> AssistantFailure {
            guard let e = error as? BookTalkModelError else { return .modelFailed }
            switch e {
            case .offline: return .offline
            case .unavailable: return .noModel
            case .timeout: return .timeout
            case .notAllowed: return .consentRequired
            case .contextOverflow: return .modelFailed
            }
        }
        func isOverflow(_ error: any Error) -> Bool { (error as? BookTalkModelError) == .contextOverflow }

        /// Gibt fällige Ansagen für eine langsame Antwort aus, der Reihe nach, im Verbraucher (nie nebenläufig). Ob eine Ansage noch
        /// gilt, entscheidet der Stand JETZT: die erste nur, solange noch keine Einheit hinausging (auch keine Überbrückung), die zweite,
        /// solange noch keine Antworteinheit hinausging. Eine Ansage ist nie Teil des Antworttextes und nie im Verlauf (`.announcement`).
        func drainFillers() async -> Outcome? {
            for n in fillers.takeDue() {
                let text: String
                if n == 1 {
                    guard !emitter.anyEmitted else { continue }
                    text = AssistantNatural.fillerFirstSentence
                } else {
                    guard !emitter.answerEmitted else { continue }
                    text = AssistantNatural.fillerSecondSentence
                }
                if let end = await ended(await emitter.emit(text, speech: text, status: .announcement)) { return end }
            }
            return nil
        }

        enum Handling { case next, stage, abort(Outcome) }
        enum Consumed { case streamEnd, stopped, failed(any Error), abort(Outcome) }

        /// Verarbeitet die Ereignisse einer Stufe der Reihe nach, bis sie endet. `handle` bekommt jedes Stück; `.stage` beendet die
        /// Stufe vorzeitig (ihr Modellstrom wird abgebrochen). Wecker lösen fällige Ansagen aus. Ereignisse älterer Stufen werden
        /// übersprungen. Ein Abbruch des Laufs beendet den Strom.
        func consume(_ stage: Int, _ handle: (String) async -> Handling) async -> Consumed {
            while let event = await events.next() {
                if Task.isCancelled { return .abort(.cancelled) }
                switch event {
                case .wake:
                    if let end = await drainFillers() { return .abort(end) }
                case .piece(let s, let text):
                    guard s == stage else { continue }
                    state.touch()
                    if let end = await drainFillers() { return .abort(end) }
                    switch await handle(text) {
                    case .next: break
                    case .stage:
                        mailbox.cancel(stage: stage)
                        return .stopped
                    case .abort(let o): return .abort(o)
                    }
                case .end(let s):
                    if s == stage { return .streamEnd }
                case .failed(let s, let error):
                    if s == stage { return .failed(error) }
                }
            }
            return .abort(.cancelled)
        }

        enum FreeStage {
            case search(words: String)
            case done(seen: Int, emitted: Int)
            case failed(any Error, seen: Int)
            case abort(Outcome)
        }

        /// Fortschritt für die Denk-Anzeige: nur Anzeige, nie im Protokoll und nie im Verlauf. Er geht durch dasselbe Senden wie `sources`
        /// (Generation, offene Anfrage, kein Schreibfehler): Nach Abbruch und Widerruf kommt keiner mehr.
        func progress(_ p: AssistantProgress) async -> Outcome? {
            await ended(output.deliver(.progress(requestId: rid, generation: snapshot.generation, p)))
        }

        /// Eine Stufe mit freien Sätzen: Satztrenner (nur ganze Sätze), Sätze des Systems und Protokollzeilen werden verworfen, jeder
        /// andere geht mit `status` und `sources` hinaus, höchstens `naturalMaxSentences`. `gated`: der Sprecher; sein Strom wird in
        /// Text und Steuerzeile `SUCHE:` getrennt (`AssistantNatural.SpeakerGate`): Der Text geht als Sätze hinaus, SOBALD sie vollständig
        /// sind (ein Zwischensatz also vor der Steuerzeile), die Steuerzeile nie. `draft`: der Denker; sein Rohtext geht als Fortschritt
        /// `draft` an die Anzeige (höchstens etwa alle 150 ms, am Ende einmal vollständig).
        func runFree(_ stage: Int, status: AssistantSentenceStatus, sources: [AssistantPassage], gated: Bool,
                     draft: Bool = false) async -> FreeStage {
            var gate = AssistantNatural.SpeakerGate()
            var splitter = AssistantSpeech.Splitter(sentencesOnly: true)
            var seen = 0
            var emitted = 0
            var limitHit = false
            var searchRaw: String?
            var tail = ""   // die letzten Zeichen, die in den Satztrenner gingen (offener Satz vor der Steuerzeile?)
            var rawText = ""
            var draftSent = ""
            var draftAt: ContinuousClock.Instant?
            let maxSentences = max(1, config.naturalMaxSentences)

            func sentences(_ list: [(text: String, speech: String, raw: String)], ignoreLimit: Bool = false) async -> Handling {
                for s in list {
                    seen += 1
                    if AssistantNatural.isSystemOnly(s.text) || AssistantNatural.isProtocolLine(s.text) {
                        emitter.drop(.template)
                        continue
                    }
                    if let end = await ended(await emitter.emit(s.text, speech: s.speech, status: status, sources: sources)) {
                        return .abort(end)
                    }
                    emitted += 1
                    if emitted >= maxSentences, !ignoreLimit {
                        limitHit = true
                        return .stage
                    }
                    if emitter.reachedLimit { return .abort(await finished()) }
                }
                return .next
            }
            func feedSplitter(_ text: String) async -> Handling {
                tail = String((tail + text).suffix(8))
                return await sentences(splitter.feed(text))
            }
            /// Die Steuerzeile ist erkannt: Was vor ihr steht, wird jetzt ausgegeben (der Zwischensatz), nicht erst am Zeilenende. Ein
            /// offener Satz (ohne Satzzeichen vor dem Zeilenumbruch) gilt als vollständig. Höchstens EIN Satz steht vor der Steuerzeile:
            /// weitere, die noch im Satztrenner standen, werden verworfen (was schon als direkte Antwort hinausging, bleibt).
            func flushBeforeSearch() async -> Handling {
                var list: [(text: String, speech: String, raw: String)] = []
                let open = !tail.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !AssistantSpeech.Splitter.endsSentence(tail)
                if open { list += splitter.feed(".") }
                list += splitter.finish()
                for _ in 0..<splitter.overlong { emitter.drop(.template) }
                for _ in 0..<splitter.fragments { emitter.drop(.fragment) }
                let first = seen == 0 ? Array(list.prefix(1)) : []
                let keep = first.filter { !AssistantNatural.isFillerInterim($0.text) }
                for _ in 0..<(list.count - keep.count) {
                    seen += 1
                    emitter.drop(.template)
                }
                return await sentences(keep, ignoreLimit: true)
            }
            func sendDraft(force: Bool) async -> Outcome? {
                guard draft, rawText != draftSent else { return nil }
                let at = ContinuousClock.now
                if !force, let last = draftAt, at - last < .milliseconds(150) { return nil }
                draftSent = rawText
                draftAt = at
                return await progress(.draft(rawText))
            }
            func piece(_ text: String) async -> Handling {
                if draft {
                    rawText += text
                    if let end = await sendDraft(force: false) { return .abort(end) }
                }
                guard gated else { return await sentences(splitter.feed(text)) }
                let step = gate.feed(text)
                if !step.speech.isEmpty {
                    switch await feedSplitter(step.speech) {
                    case .next: break
                    case let other: return other
                    }
                }
                guard let search = step.search else { return .next }
                if searchRaw == nil {
                    switch await flushBeforeSearch() {
                    case .next: break
                    case let other: return other
                    }
                }
                searchRaw = search.words
                return search.complete ? .stage : .next
            }

            switch await consume(stage, piece) {
            case .abort(let o):
                return .abort(o)
            case .failed(let e):
                return .failed(e, seen: seen)
            case .stopped:
                if let end = await sendDraft(force: true) { return .abort(end) }
                if let raw = searchRaw { return .search(words: AssistantNatural.searchWords(raw)) }
                return .done(seen: seen, emitted: emitted)
            case .streamEnd:
                if let end = await sendDraft(force: true) { return .abort(end) }
                if gated {
                    let step = gate.finish()
                    if let search = step.search {
                        return .search(words: AssistantNatural.searchWords(search.words))
                    }
                    if !step.speech.isEmpty, case .abort(let o) = await feedSplitter(step.speech) { return .abort(o) }
                }
                if !limitHit {
                    // Stromende: der Rest ist der letzte Satz, aber nur mit Satzzeichen; ein abgeschnittener Rest wird verworfen.
                    if case .abort(let o) = await sentences(splitter.finish()) { return .abort(o) }
                    for _ in 0..<splitter.overlong { emitter.drop(.template) }
                    for _ in 0..<splitter.fragments { emitter.drop(.fragment) }
                }
                return .done(seen: seen, emitted: emitted)
            }
        }

        enum SpeakerEnd { case search(String), end(Outcome) }

        /// Phase Sprecher. Vor dem Modell stehen dauerhaft: Rundenzeile `.general`/`naturalTalk` und die Bereichszeile der Quellen der
        /// Verlaufsbeiträge, die im Prompt stehen (leer möglich; in der Cloud sind es schon nur freigegebene Bereiche).
        func speakerPhase() async -> SpeakerEnd {
            state.update {
                $0.round = .general
                $0.decided = true
                $0.contextReason = .naturalTalk
            }
            var built = AssistantPrompt.buildSpeaker(question: request.text, afterInterruption: request.afterInterruption,
                                                     history: history, config: config)
            var round = AssistantJournalLine(.round, req: rid)
            round.round = .general
            round.reason = .naturalTalk
            round.gen = snapshot.generation
            var lines = [round]
            if let areasLine = emitter.areasLine(built.history.flatMap { $0.sources.map(\.area) }, force: true) { lines.append(areasLine) }
            if audit != nil { await index.reach(rid, .beforeRound) }
            if let end = ended(await emitter.land(all: lines)) { return .end(end) }
            if Task.isCancelled { return .end(.cancelled) }
            if let end = await ended(output.deliver(.sources(requestId: rid, generation: snapshot.generation, []))) { return .end(end) }
            state.touch()

            let instructions = AssistantPrompt.speakerInstructions(config: config, now: now, timeZone: .current)
            if let end = await progress(.speaker) { return .end(end) }
            var retried = false
            while true {
                let stage = mailbox.pump(model, instructions: instructions, prompt: built.prompt, maxTokens: config.maxAnswerTokens)
                switch await runFree(stage, status: .conversational, sources: [], gated: true) {
                case .abort(let o):
                    return .end(o)
                case .search(let words):
                    return .search(words)
                case .done(_, let emitted):
                    if Task.isCancelled { return .end(.cancelled) }
                    if emitted == 0 {
                        // Nichts Verwertbares: gar nichts (Fehler des Modells) oder nur Verworfenes (fester Satz).
                        if emitter.dropped == 0 { return .end(await finished(.modelFailed)) }
                        let s = AssistantNatural.notUnderstoodSentence
                        if let end = await ended(await emitter.emit(s, speech: s, status: .neutral)) { return .end(end) }
                    }
                    return .end(await finished())
                case .failed(let e, let seen):
                    if Task.isCancelled || e is CancellationError { return .end(.cancelled) }
                    if isOverflow(e), !retried, seen == 0 {
                        retried = true
                        built = AssistantPrompt.buildSpeaker(question: request.text, afterInterruption: request.afterInterruption,
                                                             history: history, config: config, includeHistory: false)
                        continue
                    }
                    return .end(await finished(failure(of: e)))
                }
            }
        }

        /// Keine Auszüge: Fortschritt `found` mit leerer Liste, dann der feste Satz; kein Denker.
        func foundNothing() async -> Outcome {
            if let end = await progress(.found(titles: [])) { return end }
            return await nothingFound()
        }

        /// Der feste Satz der Suche ohne Auszüge (`.neutral`); kein Denker.
        func nothingFound() async -> Outcome {
            if let end = await ended(output.deliver(.sources(requestId: rid, generation: snapshot.generation, []))) { return end }
            let s = AssistantNatural.nothingFoundSentence
            if let end = await ended(await emitter.emit(s, speech: s, status: .neutral)) { return end }
            return await finished()
        }

        /// Phase Daten. Vor dem Abruf steht dauerhaft die Rundenzeile `.data`/`naturalSearch` mit den zulässigen Abrufbereichen (dem Stand
        /// dieser Anfrage); der Überbrückungssatz kommt nach ihr und vor dem Abruf, nur hier.
        func dataPhase(words: String) async -> Outcome {
            state.update {
                $0.round = .data
                $0.decided = true
                $0.contextReason = .naturalSearch
            }
            var round = AssistantJournalLine(.round, req: rid)
            round.round = .data
            round.reason = .naturalSearch
            round.areas = AssistantRelease.names(areas)
            round.gen = snapshot.generation
            if audit != nil { await index.reach(rid, .beforeRound) }
            if let end = ended(await emitter.land(all: [round])) { return end }
            if Task.isCancelled { return .cancelled }
            if let end = await progress(.searching(query: words.isEmpty ? request.text : words)) { return end }
            if config.bridge {
                let text = bridges.take()
                if let end = await ended(await emitter.emit(text, speech: text, status: .announcement)) { return end }
            }
            let passages = await knowledge.retrieve(question: AssistantNatural.searchText(words: words, question: request.text),
                                                    previous: history.last?.user, limit: config.naturalPassageLimit,
                                                    maxChars: AssistantPrompt.naturalPassageBudget(config: config), areas: areas)
            if Task.isCancelled { return .cancelled }
            state.touch()
            if let end = await drainFillers() { return end }
            if passages.isEmpty { return await foundNothing() }

            let instructions = AssistantPrompt.thinkerInstructions(config: config, now: now, timeZone: .current)
            var scale = 1.0
            var includeHistory = true
            var announced = false
            var retried = false
            var composing = false
            while true {
                let built = AssistantPrompt.buildThinker(question: request.text, afterInterruption: request.afterInterruption,
                                                         passages: passages, history: history, config: config, passageScale: scale,
                                                         includeHistory: includeHistory)
                if built.used.isEmpty { return await foundNothing() }
                // `found` einmal je Anfrage (beim ersten Prompt); die Wiederholung nach einem Überlauf meldet nichts Neues.
                if !announced, let end = await progress(.found(titles: AssistantNatural.titles(of: built.used))) { return end }
                if !announced || built.used != state.snapshot.used {
                    state.update { $0.used = built.used }
                    // Die Bereiche der Auszüge im Prompt stehen fest. Zu JEDEM Modell (lokal wie Cloud) geht der Prompt erst, wenn die
                    // Zeile in der Datei steht; die Zeile der Phase steht immer (auch wenn der Sprecher dieselben Bereiche schon nannte).
                    if let end = ended(await emitter.land(emitter.areasLine(built.used.map(\.area), force: !announced))) { return end }
                    if Task.isCancelled { return .cancelled }
                    if let end = await ended(output.deliver(.sources(requestId: rid, generation: snapshot.generation, built.used))) {
                        return end
                    }
                    announced = true
                }
                if !composing {
                    composing = true
                    if let end = await progress(.composing) { return end }
                }
                let droppedBefore = emitter.dropped
                let stage = mailbox.pump(model, instructions: instructions, prompt: built.prompt, maxTokens: config.maxAnswerTokens)
                switch await runFree(stage, status: .summarized, sources: built.used, gated: false, draft: true) {
                case .abort(let o):
                    return o
                case .search:
                    return await finished(.modelFailed)
                case .done(_, let emitted):
                    if Task.isCancelled { return .cancelled }
                    if emitted == 0 {
                        if emitter.dropped == droppedBefore { return await finished(.modelFailed) }
                        let s = AssistantNatural.nothingFoundSentence
                        if let end = await ended(await emitter.emit(s, speech: s, status: .neutral)) { return end }
                    }
                    return await finished()
                case .failed(let e, let seen):
                    if Task.isCancelled || e is CancellationError { return .cancelled }
                    if isOverflow(e), !retried, seen == 0 {
                        retried = true
                        scale = 0.5
                        includeHistory = false
                        continue
                    }
                    return await finished(failure(of: e))
                }
            }
        }

        /// Bestandsfrage: Datenphase ohne Modell, das System antwortet aus den Bestandsdaten (wie im Stil `verbatim`).
        func inventoryPhase() async -> Outcome {
            state.update {
                $0.round = .data
                $0.decided = true
                $0.contextReason = .inventory
            }
            var round = AssistantJournalLine(.round, req: rid)
            round.round = .data
            round.reason = .inventory
            round.gen = snapshot.generation
            if audit != nil { await index.reach(rid, .beforeRound) }
            if let end = ended(await emitter.land(all: [round])) { return end }
            if Task.isCancelled { return .cancelled }
            if let end = await ended(output.deliver(.sources(requestId: rid, generation: snapshot.generation, []))) { return end }
            let groups = await knowledge.inventoryGroups(question: request.text, areas: areas)
            if Task.isCancelled { return .cancelled }
            if let end = await drainFillers() { return end }
            let statements = groups.flatMap(\.statements)
            if statements.isEmpty {
                let s = AssistantNatural.nothingFoundSentence
                if let end = await ended(await emitter.emit(s, speech: s, status: .neutral)) { return end }
                return await finished()
            }
            state.update { $0.inventoryAreas = groups.map(\.area) }
            // Die Bereiche der Bestandsangaben stehen in der Datei, bevor die erste Angabe hinausgeht.
            if let end = ended(await emitter.noteAreas(groups.map(\.area))) { return end }
            if Task.isCancelled { return .cancelled }
            let preface = AssistantGrounding.inventoryAnnouncement
            if let end = await ended(await emitter.emit(preface, speech: preface, status: .announcement)) { return end }
            for s in statements {
                if let end = await ended(await emitter.emit(s, speech: emitter.speech(s), status: .inventory)) { return end }
            }
            return await finished()
        }

        // Pflichtzeilen, fail-closed (wie im Stil `verbatim`): Standzeile und `start` stehen dauerhaft in der Datei, bevor ein Modell
        // läuft oder etwas ausgegeben wird; ebenso die Rundenzeile und die Bereichszeile jeder Phase vor ihrem Modell bzw. Abruf.
        state.update { $0.contextBefore = context.mode }
        if let audit {
            await index.reach(rid, .beforeState)
            guard await stateLine.ensure(audit, request: rid) else {
                await index.reach(rid, .stateRefused)
                return fail(.auditUnavailable)
            }
            if Task.isCancelled { return .cancelled }
            let landing = await emitter.land(startLine(rid, session: session, config: config, record: state.snapshot), run: run)
            state.update { $0.journaled = landing == .stands }
            if let end = ended(landing) { return end }
            if Task.isCancelled { return .cancelled }
            await catchUp()
        }
        // Cloud-Ziel ohne Zustimmung (aus dem Stand dieser Anfrage): weder Sprecher noch Abruf noch Denker.
        if config.isCloud, !snapshot.cloudConsent { return fail(.consentRequired) }

        let titles = await knowledge.inventoryTitles(areas: areas)
        if Task.isCancelled { return .cancelled }
        state.touch()

        if AssistantKnowledge.isInventoryQuestion(request.text) { return await inventoryPhase() }
        if AssistantOwnData.refers(request.text, mode: .question, titles: titles) { return await dataPhase(words: "") }
        switch await speakerPhase() {
        case .end(let outcome): return outcome
        case .search(let words): return await dataPhase(words: words)
        }
    }
}
