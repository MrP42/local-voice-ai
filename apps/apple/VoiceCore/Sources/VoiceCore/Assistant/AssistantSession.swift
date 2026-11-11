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

/// Ein ausgegebener Satz (ohne Ansagen) mit Einordnung und Quellen, für `commit`.
struct AssistantEmitted: Sendable, Equatable {
    let text: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
    let quote: Bool
    /// Schrittnummer der Einheit im Protokoll (0: unbekannt, nur Tests).
    var step = 0
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
    /// Die Journalzeile `start` ist geschrieben.
    var journaled = false
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


/// Ausgang eines Laufs. Jedes Ereignis geht durch `send`: Die Prüfung der Generation und das Senden geschehen im selben Lock,
/// nach einem Widerruf kommt also nichts Altes mehr heraus.
final class AssistantRunOutput: @unchecked Sendable {
    let release: AssistantRelease
    let generation: Int
    private let continuation: AsyncStream<AssistantEvent>.Continuation

    init(release: AssistantRelease, generation: Int, continuation: AsyncStream<AssistantEvent>.Continuation) {
        self.release = release
        self.generation = generation
        self.continuation = continuation
    }

    /// Sendet nur, wenn die Generation noch gilt (`stale`: nichts gesendet). `onResult` läuft im selben Lock unmittelbar nach dem
    /// Senden, mit `true`, wenn das Ereignis den Strom erreicht hat, und `false`, wenn der Strom schon beendet war (Leser weg:
    /// die Aktion ließ sich nicht auslösen, Ergebnis `failed`). Dort wird die Ergebniszeile des Protokolls eingereiht (kurz,
    /// ohne Wartezeit): Sie steht damit vor jeder Rückmeldung des Controllers und stimmt mit dem tatsächlichen Senden überein.
    /// `onStale`: die Ablehnung wird IM Lock mit der vorgefundenen Generation gemeldet (Zeile „verworfen“ in der Reihenfolge der
    /// Erhöhungen). `refuse`: hat das Protokoll der Anfrage schon einen Schreibfehler, geht nichts hinaus (Ergebnis `failed`).
    func send(_ event: AssistantEvent, onResult: ((Bool) -> Void)? = nil, onStale: ((Int) -> Void)? = nil,
              refuse: (() -> Bool)? = nil) -> AssistantCheck {
        release.ifCurrentAttempt(generation, onStale: onStale) {
            if refuse?() == true { return false }
            let delivered: Bool
            switch continuation.yield(event) {
            case .terminated: delivered = false
            default: delivered = true
            }
            onResult?(delivered)
            return delivered
        }
    }

    /// Fehler und Abbruch tragen keine Inhalte und gehen immer hinaus.
    func sendUnchecked(_ event: AssistantEvent) { continuation.yield(event) }
}

/// Die Standzeile (`state`) dieses Prozesslaufs (EINE je Ordnung, `AssistantReleaseClock.stateLine`): Sie wird von der ersten
/// Sitzung beim Anlegen eingereiht; die erste Anfrage wird erst angenommen, wenn sie in der Datei steht. Misslingt das Schreiben,
/// nimmt keine Sitzung eine Anfrage an (`.auditUnavailable`); die nächste Anfrage versucht es mit dem dann gültigen Stand erneut.
/// Mehrere gleichzeitige Anfragen (auch verschiedener Sitzungen) teilen EINEN Versuch. Der Lock ist ein Blatt: unter ihm wird nur
/// `make` gerufen (liest den Stand im Lock der Ordnung, Reihenfolge StateLine → Ordnung) und eingereiht, nie gewartet.
final class AssistantStateLine: @unchecked Sendable {
    private let lock = NSLock()
    private var make: (@Sendable () -> AssistantJournalLine)?
    private var run: AssistantRunToken?
    private var key = "state"
    private var enqueued = false
    private var confirmed = false
    private var attempt: Task<Bool, Never>?

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

    func ensure(_ audit: AssistantAuditLog) async -> Bool {
        guard let task = startOrJoin(audit) else { return true }
        let ok = await task.value
        finish(task, ok: ok)
        return ok
    }

    /// `nil`: schon bestätigt (oder nie begonnen). Sonst der (geteilte) Versuch; der erste reiht die Zeile bei Bedarf neu ein.
    private func startOrJoin(_ audit: AssistantAuditLog) -> Task<Bool, Never>? {
        lock.lock()
        defer { lock.unlock() }
        guard let make, let run else { return nil }
        if confirmed { return nil }
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
        } else if attempt == task {
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
    enum Delivery { case delivered, revoked, failed }

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

    /// Nummeriert einen Schritt und reiht seine Zeile ein (nur Einreihen; das Warten macht der Aufrufer).
    private func step(_ verdict: AssistantTraceVerdict) -> Int {
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

    private func commitLine(_ n: Int, _ mark: AssistantCommitMark) -> AssistantJournalLine {
        var l = AssistantJournalLine(.commit, req: rid)
        l.n = n
        l.mark = mark
        return l
    }

    /// Gibt eine Einheit aus. Reihenfolge (Schreibfolge E4.2b): Einordnung (`step`) einreihen, dauerhaft schreiben lassen (das
    /// deckt auch die Ergebniszeilen der vorigen Einheiten und jeden früheren Fehler der Anfrage), erst dann den Ausgabe-Commit
    /// prüfen und senden. Misslingt das Schreiben, wird die Einheit NICHT ausgegeben.
    func emit(_ text: String, speech: String, status: AssistantSentenceStatus, sources: [AssistantPassage] = [],
              quote: Bool = false) async -> Delivery {
        let announcement = status == .announcement
        let n = step(AssistantTraceVerdict(status: status))
        if let journal, await journal.waitUntilWritten(request: rid) != nil {
            // Nichts wurde ausgegeben: das Ergebnis ist „verworfen“, nicht „unbekannt“.
            release.inSection { found in
                journal.enqueue(commitLine(n, AssistantCommitMark(stage: .emit, ok: false, generation: found)))
            }
            if !announcement { state.update { $0.counts.drop(.revoked) } }
            return .failed
        }
        // Der Schritt steht im Index, BEVOR das Ereignis hinausgeht: Eine schnelle Rückmeldung des Controllers läuft nie ins Leere.
        index.append(rid, step: n)
        let gen = generation
        var refusedByAudit = false
        let check = output.send(.sentence(requestId: rid, generation: gen, text: text, speech: speech, status: status,
                                          sources: sources, quote: quote), onResult: { delivered in
            self.journal?.enqueue(self.commitLine(n, AssistantCommitMark(stage: .emit, ok: delivered, generation: gen, failed: !delivered)))
        }, onStale: { found in
            self.journal?.enqueue(self.commitLine(n, AssistantCommitMark(stage: .emit, ok: false, generation: found)))
        }, refuse: {
            // Ein Schreibfehler zwischen dem Warten und dem Senden (klebrig): nichts geht hinaus.
            guard let journal = self.journal, journal.hasFailed(request: self.rid) else { return false }
            refusedByAudit = true
            self.journal?.enqueue(self.commitLine(n, AssistantCommitMark(stage: .emit, ok: false, generation: gen, failed: true)))
            return true
        })
        if !check.ok { index.removeLast(rid) }
        if refusedByAudit {
            if !announcement { state.update { $0.counts.drop(.revoked) } }
            return .failed
        }
        let delivered = check.ok
        if delivered, firstMs == nil { firstMs = elapsedMs }
        state.update { r in
            if delivered {
                switch status {
                case .grounded: r.counts.grounded += 1
                case .inventory: r.counts.inventory += 1
                case .unsourced: r.counts.unsourced += 1
                case .neutral: r.counts.neutral += 1
                case .announcement: break
                }
                r.outputSteps.append(n)
                if !announcement { r.sentences.append(AssistantEmitted(text: text, status: status, sources: sources, quote: quote, step: n)) }
            } else if !announcement {
                r.counts.drop(.revoked)
            }
        }
        if delivered, !announcement { texts.append(text) }
        return delivered ? .delivered : .revoked
    }

    private func flow(_ d: Delivery) -> Flow {
        switch d {
        case .delivered: .next
        case .revoked: .revoked
        case .failed: .failed
        }
    }

    /// Schreibt die Bereiche der Quellen, sobald sie feststehen (Zeile `areas`), nicht erst im `end`: Ein Prozessende danach
    /// verliert sie nicht. Nur Neues wird geschrieben, und die Zeile steht dauerhaft in der Datei, wenn die Methode zurückkehrt.
    /// `false`: die Pflichtzeile ließ sich nicht schreiben (kein Modell, keine Ausgabe).
    func noteAreas(_ areas: [AssistantArea]) async -> Bool {
        let names = Set(areas.map(\.rawValue))
        var fresh = false
        state.update { r in
            guard !names.subtracting(r.areasWritten).isEmpty else { return }
            r.areasWritten.formUnion(names)
            fresh = true
        }
        guard let journal else { return true }
        if fresh {
            var l = AssistantJournalLine(.areas, req: rid)
            l.areas = names.sorted()
            l.gen = generation
            journal.enqueue(l)
        }
        return await journal.waitUntilWritten(request: rid) == nil
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
        let second = await emit(q.text, speech: speech(q.text), status: .grounded, sources: [q.passage], quote: true)
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

/// Änderungsnummern von Erweiterungen, die in diesem Lauf nicht wirksam wurden und deren Zeile „Erweiterung in diesem Lauf nicht
/// wirksam“ noch nicht geschrieben werden konnte: Sie wird bei der nächsten Gelegenheit nachgeholt.
final class AssistantPendingChanges: @unchecked Sendable {
    struct Item: Sendable, Equatable {
        let number: Int
        let cause: AssistantReleaseCause
        let state: AssistantReleaseState
    }

    private let lock = NSLock()
    private var items: [Item] = []

    func add(_ item: Item) {
        lock.lock()
        if !items.contains(where: { $0.number == item.number }) { items.append(item) }
        lock.unlock()
    }

    func remove(_ number: Int) {
        lock.lock()
        items.removeAll { $0.number == number }
        lock.unlock()
    }

    var all: [Item] {
        lock.lock()
        defer { lock.unlock() }
        return items
    }
}

public actor AssistantSession {
    public static let maxStoredTurns = 200
    /// Höchstens so viele Auszüge je Frage.
    static let passageLimit = 8
    /// Wie lange `commit` auf den geordneten Abschluss eines Laufs wartet (hängendes Modell).
    static let commitWait: Duration = .seconds(2)
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

    /// Wohin eine Änderung der Klasse A gesichert wird: die Zustandsdatei (mit der Art des Modells und dem Cloud-Ziel dieser Sitzung)
    /// oder, ohne Speicher (Tests der Freigabe allein), nirgends.
    private struct Persistence: Sendable {
        let store: AssistantReleaseStore?
        let cloud: Bool
        let target: String?

        /// `nil`: bestätigt (oder kein Speicher). Sonst der Fehler: Der Ausgang des Austauschs ist OFFEN.
        func save(_ view: AssistantReleaseState, cause: AssistantReleaseCause) async -> AssistantStoreError? {
            guard let store else { return nil }
            return await store.persist(view, cloud: cloud, target: target, cause: cause)
        }
    }

    private let model: any BookTalkModel
    private let knowledge: AssistantKnowledge
    private let config: AssistantConfig
    private let release: AssistantRelease
    private let context: AssistantContext
    private let released: Bool
    private let stepIndex = AssistantStepIndex()
    private let sessionId = UUID().uuidString
    private let audit: AssistantAuditLog?
    private let stateLine: AssistantStateLine
    private let runToken: AssistantRunToken
    private nonisolated var clock: AssistantReleaseClock { release.clock }
    private let persistence: Persistence
    private let ineffective = AssistantPendingChanges()
    private var active: [String: ActiveRun] = [:]
    private var earlyCancel: Set<String> = []
    private var results: [String: RunState] = [:]
    private var resultOrder: [String] = []
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
         cloudTarget: String? = nil) {
        // Die eine Ordnung dieses Prozesslaufs: mit Speicher die seine (alle Sitzungen und Änderungen ohne Sitzung teilen Lock,
        // Generation, Lauf-Token, Standzeile, Zähler und Reihe), ohne Speicher eine eigene.
        let clock = releaseStore?.clock ?? AssistantReleaseClock()
        let run = clock.run
        let initial: AssistantReleaseState
        if let releaseStore {
            var fromFile = releaseStore.record.state(cloud: config.isCloud, target: cloudTarget)
            if !released { fromFile.optIn = false; fromFile.consentVersion = 0 }
            initial = fromFile
        } else {
            initial = AssistantReleaseState(optIn: config.allowUnsourced && released, consentVersion: config.consentVersion,
                                            areas: knowledge.currentAreas, cloudConsent: cloudConsent)
        }
        let optIn = initial.optIn
        let release = AssistantRelease(optIn: optIn, consentVersion: initial.consentVersion, areas: initial.areas,
                                       cloudConsent: initial.cloudConsent, locked: releaseStore?.isLocked ?? false,
                                       blocked: releaseStore?.isBlocked ?? false, clock: clock,
                                       onIncrease: Self.increaseWriter(audit: audit, config: config, run: run))
        self.runToken = run
        self.persistence = Persistence(store: releaseStore, cloud: config.isCloud, target: cloudTarget)
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

    /// Eine Zeile der Klasse A (`release`, `intent`, `effective`, `ineffective`): Lauf kommt vom Token beim Schreiben; Änderungsnummer,
    /// Ursache und Stand (ohne Titel und Inhalte). Die Generation steht nur in Zeilen, in denen sie für genau diese Änderung vergeben
    /// wurde (`release`, `effective`), nie in `intent` und `ineffective`.
    private static func classALine(_ kind: AssistantJournalLine.Kind, change: Int?, generation: Int?, cause: AssistantReleaseCause,
                                   state: AssistantReleaseState, config: AssistantConfig) -> AssistantJournalLine {
        var line = AssistantJournalLine(kind)
        line.at = Date()
        line.gen = generation
        line.chg = change
        line.cause = cause
        line.areas = AssistantRelease.names(state.areas)
        line.unsourced = state.optIn
        line.consent = state.consentVersion
        line.granted = state.cloudConsent
        line.model = AssistantModelKind.classify(label: config.modelLabel)
        line.cloud = config.isCloud
        return line
    }

    /// Die Zeile zu einer Erhöhung, die im Lock der Freigabe geschieht: „Erweiterung wirksam“, Änderung eines Inhalts (Ursache
    /// „Inhalt“) und Blockade. Wird IM Lock an der Stelle der Erhöhung gerufen (kurz, nur Einreihen, kein I/O). Einschränkungen
    /// schreiben ihre Zeile erst nach der Zustandsdatei (`settle`). Ohne Protokoll gibt es keine Zeile.
    private static func increaseWriter(audit: AssistantAuditLog?, config: AssistantConfig, run: AssistantRunToken)
        -> (@Sendable (AssistantRelease.Increase) -> Void)? {
        guard let audit else { return nil }
        return { increase in
            let kind: AssistantJournalLine.Kind = increase.kind == .effective ? .effective : .release
            let line = classALine(kind, change: increase.change, generation: increase.generation, cause: increase.cause,
                                  state: increase.state, config: config)
            audit.enqueue(line, key: increase.key, run: run)
        }
    }

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

    /// Was `settle` über die Teile einer Änderung sammelt.
    private struct SettleState: Sendable {
        var error: AssistantAuditError?
        var storeError: AssistantStoreError?
        var effective = true
        var generation = 0
    }

    /// Sichert die Teile der Änderung der Reihe nach (streng nacheinander mit den Folgen anderer Änderungen) und gibt den Stand
    /// zurück. Jede Folge wartet auf ihre Reihe in der Reihenfolge des Anwendens (`apply…`): Wer mehrere Änderungen angewendet hat,
    /// wickelt sie in dieser Reihenfolge ab (der Controller: eine Kette in der Reihenfolge des Anwendens) oder nebenläufig; ein
    /// späteres `settle` vor dem früheren, das nie läuft, wartete ewig. Ein verworfenes `AssistantReleasePending` gibt seine Reihe frei. Ein Fehler steht in `error` (Zeile des Protokolls) oder `storeError` (Zustandsdatei, Ausgang offen); eine
    /// Einschränkung gilt trotzdem, eine Erweiterung nur nach bestätigtem Austausch.
    public nonisolated func settle(_ pending: AssistantReleasePending) async -> AssistantReleaseResult {
        let applied = pending.applied
        var out = SettleState()
        out.generation = applied.generation
        for step in applied.steps {
            await step.ticket.turn()
            await settleStep(step, into: &out)
            step.ticket.complete()
        }
        let expands = applied.expands || pending.optInExpansion || pending.resume
        var done = out.effective
        if done, pending.optInExpansion { context.optInChanged(true) }
        if done, pending.resume { done = context.resumeFree(optIn: release.isOptedIn) }
        if applied.changed { await knowledge.setAreas(release.snapshot.areas) }   // Spiegel für Aufrufer ohne Sitzung
        return AssistantReleaseResult(generation: out.generation, changed: applied.changed, effective: !expands || done, error: out.error,
                                      storeError: out.storeError)
    }

    private nonisolated func settleStep(_ step: AssistantRelease.Step, into out: inout SettleState) async {
        await catchUpInSequence()
        switch step.kind {
        case .event:
            out.generation = step.generation
            await writeLine(.release, step: step, generation: step.generation, state: step.delta, into: &out)
        case .restriction:
            out.generation = step.generation
            if let error = await persistence.save(release.viewForSave(step), cause: step.cause) {
                // Der Ausgang des Austauschs ist offen: Die Einschränkung gilt im Prozess, ist aber nicht als gesichert bestätigt.
                out.storeError = error
                release.block()
                persistence.store?.markBlocked()
            } else {
                release.clearLocked()
            }
            await writeLine(.release, step: step, generation: step.generation, state: step.delta, into: &out)
        case .expansion:
            await settleExpansion(step, into: &out)
        }
    }

    private nonisolated func settleExpansion(_ step: AssistantRelease.Step, into out: inout SettleState) async {
        let intended = release.viewForSave(step)
        // (1) „Erweiterung beabsichtigt“, ohne Generation, dauerhaft. Scheitert das, wird die Erweiterung nicht wirksam und nicht gesichert.
        // Eine blockierte Sitzung (Einschränkung nicht gesichert) lehnt nie ab, ohne die Sicherung neu zu versuchen: `catchUpInSequence`
        // lief schon zu Beginn dieser Folge; hier noch einmal, falls inzwischen schreibbar.
        if release.isBlocked {
            await secureOutstandingRestriction()
            return await notEffective(step, state: intended, into: &out)
        }
        var wrote = true
        await writeLine(.intent, step: step, generation: nil, state: intended, into: &out, ok: &wrote)
        guard wrote else { return await notEffective(step, state: intended, into: &out) }
        // (2) Zustandsdatei ersetzen: das ist der Commit der Erweiterung. Scheitert oder bricht er ab, ist der Ausgang offen.
        if let error = await persistence.save(intended, cause: step.cause) {
            out.storeError = error
            return await notEffective(step, state: intended, into: &out)
        }
        // (3) Erst jetzt wirksam im Prozess: Stand, Generation und Zeile „wirksam“ in einem unteilbaren Schritt.
        guard let generation = release.becomeEffective(step) else { return await notEffective(step, state: intended, into: &out) }
        out.generation = generation
        // (4) Die Zeile „wirksam“ steht dauerhaft; ihr Fehler wird gemeldet, die Erweiterung bleibt wirksam (sie ist gesichert).
        if let error = await audit?.waitForLine(key: step.key) { out.error = out.error ?? error }
    }

    /// Die Erweiterung wird in diesem Lauf nicht wirksam: Zeile „Erweiterung in diesem Lauf nicht wirksam“, sobald geschrieben werden kann.
    private nonisolated func notEffective(_ step: AssistantRelease.Step, state: AssistantReleaseState, into out: inout SettleState) async {
        out.effective = false
        release.abandon(step)
        let item = AssistantPendingChanges.Item(number: step.number, cause: step.cause, state: state)
        ineffective.add(item)
        await flushPendingIneffective()
    }

    /// Nachholen bei jeder Gelegenheit (ADR, Fehlerpfade der Einschränkung): Die nicht gesicherte Einschränkung wird erneut gesichert
    /// und die Zeilen „in diesem Lauf nicht wirksam“, die noch fehlen, werden geschrieben. Gerufen am Anfang jeder Sicherungsfolge
    /// und bei jeder Anfrage (nach der gesicherten `start`-Zeile). Nur innerhalb der Reihe der Folgen aufrufen (`catchUp()` nimmt eine
    /// Karte).
    private nonisolated func catchUpInSequence() async {
        await secureOutstandingRestriction()
        await flushPendingIneffective()
        if let audit { await persistence.store?.flushUnwritten(audit) }
    }

    /// Sichert eine noch nicht gesicherte Einschränkung erneut (der gemerkte engere Stand begrenzt die Fassung). Die Blockade für
    /// den Rest des Laufs bleibt in jedem Fall.
    private nonisolated func secureOutstandingRestriction() async {
        guard let store = persistence.store, store.needsCatchUp else { return }
        if await persistence.save(release.snapshot.state, cause: .other) == nil { release.clearLocked() }
    }

    /// Es gibt etwas nachzuholen (billige Vorprüfung ohne Warten).
    private nonisolated var needsCatchUp: Bool {
        (persistence.store?.needsCatchUp ?? false) || (persistence.store?.hasUnwrittenLines ?? false) || !ineffective.all.isEmpty
    }

    /// Holt nach, was bei einer früheren Gelegenheit nicht ging: eine nicht gesicherte Einschränkung erneut sichern, fehlende Zeilen
    /// „in diesem Lauf nicht wirksam“ schreiben. Läuft bei jeder Anfrage (nach ihrer gesicherten `start`-Zeile) und bei jeder
    /// Änderung (`settle`); streng nacheinander mit den Folgen der Änderungen (eine Karte der Reihe), und nur, wenn etwas aussteht.
    public nonisolated func catchUp() async {
        guard needsCatchUp else { return }
        let ticket = release.enqueueTicket()
        await ticket.turn()
        defer { ticket.complete() }
        await catchUpInSequence()
    }

    /// Holt Zeilen „in diesem Lauf nicht wirksam“ nach, die noch nicht geschrieben werden konnten.
    private nonisolated func flushPendingIneffective() async {
        guard let audit else { return }
        for item in ineffective.all {
            let line = Self.classALine(.ineffective, change: item.number, generation: nil, cause: item.cause, state: item.state, config: config)
            let key = clock.key("chg", item.number)
            audit.enqueue(line, key: key, run: runToken)
            if await audit.waitForLine(key: key) == nil { ineffective.remove(item.number) }
        }
    }

    private nonisolated func writeLine(_ kind: AssistantJournalLine.Kind, step: AssistantRelease.Step, generation: Int?,
                                       state: AssistantReleaseState, into out: inout SettleState) async {
        var ok = true
        await writeLine(kind, step: step, generation: generation, state: state, into: &out, ok: &ok)
    }

    private nonisolated func writeLine(_ kind: AssistantJournalLine.Kind, step: AssistantRelease.Step, generation: Int?,
                                       state: AssistantReleaseState, into out: inout SettleState, ok: inout Bool) async {
        guard let audit else { return }
        let line = Self.classALine(kind, change: step.number, generation: generation, cause: step.cause, state: state, config: config)
        audit.enqueue(line, key: step.key, run: runToken)
        if let error = await audit.waitForLine(key: step.key) {
            out.error = out.error ?? error
            ok = false
        }
    }

    /// Versucht die Sicherung des jetzigen Stands erneut (nach einem gescheiterten Austausch der Zustandsdatei). Läuft streng
    /// nacheinander mit den anderen Änderungen. Auch ohne Fehler schreibt es die Datei neu. Die Blockade für den Rest des Laufs
    /// bleibt in jedem Fall bestehen.
    @discardableResult
    public nonisolated func retrySave() async -> AssistantStoreError? {
        let ticket = release.enqueueTicket()
        await ticket.turn()
        defer { ticket.complete() }
        let error = await persistence.save(release.snapshot.state, cause: .other)
        if error == nil { release.clearLocked() }
        return error
    }

    /// Eine Einschränkung ließ sich nicht sichern oder die Datei steht noch nicht fest: nicht bestätigt.
    public nonisolated var needsSave: Bool { persistence.store?.isDirty ?? false }

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
        // Prüfung steht IM Lock der Ordnung (vor `body`), nicht davor.
        var blocked = false
        let check = release.ifCurrent(generation, onStale: { found in
            // Die Ablehnung wird im Lock eingereiht, mit der vorgefundenen Generation: nie hinter der Zeile einer späteren Änderung.
            self.enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: stage, ok: false, generation: found))
        }) {
            if audit?.hasFailed(request: requestId) == true {
                blocked = true
                enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: stage, ok: false, generation: generation, failed: true))
                return
            }
            body()
            if reportPass { enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: stage, ok: true, generation: generation)) }
        }
        return AssistantCommitResult(ok: check.ok && !blocked, generation: check.generation,
                                     auditFailed: blocked || (audit?.hasFailed(request: requestId) ?? false))
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
        let check = release.ifCurrentAttempt(generation, onStale: { found in
            self.enqueueMark(requestId: requestId, step: step, mark: AssistantCommitMark(stage: .audio, ok: false, generation: found))
        }) {
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
        return AssistantCommitResult(ok: check.ok, failed: check.failed && !blocked, generation: check.generation,
                                     auditFailed: blocked || (audit?.hasFailed(request: requestId) ?? false))
    }

    /// Nur für Tests der Commits ohne Lauf: macht der Sitzung eine Anfrage bekannt, als hätte dieser Prozesslauf ein Ereignis
    /// ausgegeben (sonst gelten Commits für unbekannte Anfragen nie).
    nonisolated func admitForTesting(requestId: String) { stepIndex.append(requestId, step: 0) }

    /// Meldet das Ergebnis eines Ausgabe-Commits, das NICHT im Lock lief (der Controller hat gar nichts ausgelöst, z. B. weil
    /// die Stimme die Generation schon verworfen hatte): Die Zeile trägt die übergebene Generation. Ohne Warten. `failed`: die
    /// Aktion ließ sich nicht auslösen, obwohl nichts widerrufen war (Marke „fehlgeschlagen“, nie „ausgeführt“).
    public nonisolated func noteCommit(requestId: String, index: Int, stage: AssistantCommitStage, ok: Bool, generation: Int,
                                       failed: Bool = false) {
        let step = stepIndex.step(requestId, index: index)
        // Die Zeile wird im Lock der Ordnung eingereiht (Reihenfolge der Erhöhungen); eine Ablehnung trägt die dort vorgefundene
        // Generation (nie die mitgebrachte, die inzwischen veraltet sein kann).
        release.inSection { found in
            let mark = ok || failed
                ? AssistantCommitMark(stage: stage, ok: ok, generation: generation, failed: failed)
                : AssistantCommitMark(stage: stage, ok: false, generation: max(generation, found))
            enqueueMark(requestId: requestId, step: step, mark: mark)
        }
    }

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

    /// Zuordnung des tatsächlich Gesprochenen zu den ausgegebenen Sätzen (der Reihe nach, Ansagen zählen nicht). Ein Satz, der
    /// nur angesprochen wurde (Unterbrechung), zählt mit seiner Einordnung. Gesprochenes ohne Entsprechung (kommt bei einem
    /// vollständigen Lauf nicht vor) wird nie als „ohne Quelle“ gewertet, sondern als Vorlage ohne Quelle.
    static func attribute(spoken: String, record: AssistantRunRecord?)
        -> [(text: String, status: AssistantSentenceStatus, sources: [AssistantPassage], quote: Bool, step: Int?)] {
        var rest = Substring(spoken)
        var out: [(text: String, status: AssistantSentenceStatus, sources: [AssistantPassage], quote: Bool, step: Int?)] = []
        for e in record?.sentences ?? [] where e.status != .announcement {
            rest = rest.drop(while: { $0.isWhitespace })
            if rest.isEmpty { break }
            if rest.hasPrefix(e.text) {
                out.append((e.text, e.status, e.sources, e.quote, e.step > 0 ? e.step : nil))
                rest = rest.dropFirst(e.text.count)
            } else if e.text.hasPrefix(rest) {
                out.append((String(rest), e.status, e.sources, e.quote, e.step > 0 ? e.step : nil))
                rest = ""
            }
        }
        rest = rest.drop(while: { $0.isWhitespace })
        if !rest.isEmpty { out.append((String(rest), .neutral, [], false, nil)) }
        return out
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
        let deadline = ContinuousClock.now.advanced(by: Self.commitWait)
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
        return l
    }

    /// Nach `done` oder nach Barge-in (dann `spoken` = tatsächlich Gesprochenes, `interrupted` = true) trägt die Steuerung
    /// den Beitrag ein. Für JEDE Anfrage entsteht ein abgeschlossenes Protokoll (`start` … `end`), auch ohne gesprochenen Text
    /// (Fehler, Abbruch, Widerruf); einen Beitrag im Verlauf gibt es nur mit Nutzertext, mit tatsächlich Gesprochenem, mit
    /// bekanntem Lauf und bei noch gültiger Freigabe-Generation: Die Prüfung, das Anhängen UND die Ergebniszeile
    /// (`h+<Generation>`) geschehen im selben Lock wie die Erhöhung der Generation; eine Ablehnung trägt die dabei gefundene
    /// Generation. Läuft die Anfrage noch, bricht `commit` sie ab und wartet auf ihren Endstand, bevor es schreibt. Quellen (nur
    /// die, die einen gesprochenen Satz decken) kommen aus der Prüfung dieser Anfrage. Mit dem Commit endet die Anmeldung des
    /// Laufs für den Widerruf. `false`, wenn nur das Protokoll nicht geschrieben werden konnte (der Beitrag steht trotzdem im
    /// Verlauf); die Anfrage endet dann mit Ausgang „Fehler“.
    @discardableResult
    public func commit(requestId: String, user: String, spoken: String, interrupted: Bool, revoked: Bool = false) async -> Bool {
        guard let state = await settle(requestId) else { return true }   // unbekannte oder schon abgeschlossene Anfrage
        release.unregister(requestId)
        let record = state.snapshot
        let text = user.trimmingCharacters(in: .whitespacesAndNewlines)
        let said = spoken.trimmingCharacters(in: .whitespacesAndNewlines)

        // `start` zuerst (war es nicht schon geschrieben): Die Zeilen der Anfrage stehen in der Reihenfolge start, …, h, end.
        if let audit, !record.journaled { audit.enqueue(Self.startLine(requestId, session: sessionId, config: config, record: record), run: runToken) }

        var historyRejected = false
        // Ergebnis des Verlaufs JE EINHEIT (Runde 7, B4): Jede ausgegebene Einheit bekommt ihre Marke `h+g` (steht im Beitrag) oder `h-g`
        // (nicht im Beitrag: nicht gesprochen, abgelehnt oder gar kein Beitrag), in demselben Lock-Schritt wie das Anhängen.
        let outputSteps = record.outputSteps
        if text.isEmpty || said.isEmpty {
            release.inSection { found in
                enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: [], generation: found)
            }
        }
        if !text.isEmpty, !said.isEmpty {
            let parts = Self.attribute(spoken: said, record: record)
            let included = Set(parts.compactMap(\.step))
            let sources = Self.distinct(parts.filter { $0.status == .grounded }.flatMap(\.sources))
            let anyUnsourced = parts.contains { $0.status == .unsourced }
            // Abschnitte (Marke je Satz): sobald ein Zitat oder ein Satz mit Kennzeichnung (feste Antwort, Bestandsangabe) vorkommt
            // oder Sätze ohne Quelle mit anderen gemischt sind; ein rein unbelegter Beitrag bleibt ohne Abschnitte.
            var turnParts: [AssistantTurnPart] = []
            let marked = parts.contains { $0.quote || AssistantTurnPart.note(for: $0.status, quote: $0.quote) != nil }
            if marked || (anyUnsourced && parts.contains { $0.status != .unsourced }) {
                for p in parts {
                    AssistantTurnPart.append(to: &turnParts, text: p.text, status: p.status, quote: p.quote, sources: p.sources)
                }
            }
            let turn = AssistantTurn(
                user: text, assistant: said, interrupted: interrupted, sources: sources, general: sources.isEmpty,
                unsourced: anyUnsourced, parts: turnParts, model: config.modelLabel, at: Date(),
                round: record.decided ? record.round : nil)
            // Prüfung der Generation, Anhängen und Ergebniszeile im selben Lock wie die Erhöhung der Generation: Ein Widerruf
            // dazwischen ist ausgeschlossen, die Marke trägt genau die geprüfte Generation.
            if !revoked, !record.revoked {
                let check = release.ifCurrent(record.generation, onStale: { found in
                    self.enqueueMark(requestId: requestId, step: 0, mark: AssistantCommitMark(stage: .history, ok: false, generation: found))
                    self.enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: [], generation: found)
                }) {
                    history.append(turn)
                    if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }
                    enqueueMark(requestId: requestId, step: 0, mark: AssistantCommitMark(stage: .history, ok: true, generation: record.generation))
                    enqueueHistoryMarks(requestId: requestId, steps: outputSteps, included: included, generation: record.generation)
                }
                historyRejected = !check.ok
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
        let outcome: AssistantAuditEntry.Outcome
        if writeFailed || record.failure == .auditUnavailable {
            outcome = .failed   // ein Schreibfehler geht dem Widerruf voran (Schreibfolge 7: nichts wird ausgegeben, Ausgang „Fehler“)
        } else if revoked || record.revoked || record.failure == .revoked || historyRejected {
            outcome = .revoked   // auch ein fertiger Lauf, dessen Verlaufs-Commit die geänderte Freigabe verwirft
        } else if let failure = record.failure {
            outcome = failure == .cancelled ? .interrupted : .failed
        } else if record.limited {
            outcome = .limited
        } else {
            outcome = interrupted || record.cancelled ? .interrupted : .completed
        }
        var end = AssistantJournalLine(.end, req: requestId)
        end.at = Date()
        end.outcome = outcome
        end.level = AssistantAuditLevel.of(record.counts)
        let covering = Self.distinct(record.sentences.filter { $0.status == .grounded }.flatMap(\.sources)).map(\.area)
        end.areas = Set((covering + record.inventoryAreas).map(\.rawValue)).sorted()
        audit.enqueue(end)
        let tail = await audit.flush(request: requestId)
        return !writeFailed && tail == nil
    }

    // MARK: Abbruch

    /// Bricht eine laufende Anfrage ab: nur noch `.failed(.cancelled)`, nichts wird gespeichert. Ein Abbruch, der vor dem
    /// Start der Anfrage eintrifft, wirkt beim Start.
    public func cancel(requestId: String) {
        if let run = active[requestId] {
            run.state.markCancelled()
            return run.stop()
        }
        if earlyCancel.count > 64 { earlyCancel.removeAll() }
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
        if earlyCancel.remove(rid) != nil {
            out.yield(.failed(requestId: rid, .cancelled))
            if let audit {
                var record = AssistantRunRecord()
                record.generation = snapshot.generation
                record.optInAtStart = snapshot.optIn
                record.consentVersion = snapshot.consentVersion
                audit.enqueue(Self.startLine(rid, session: sessionId, config: config, record: record), run: runToken)
                var end = AssistantJournalLine(.end, req: rid)
                end.at = Date()
                end.outcome = .interrupted
                end.level = .s0
                end.areas = []
                audit.enqueue(end)
                _ = await audit.flush(request: rid)
            }
            return
        }
        var runConfig = self.config
        runConfig.allowUnsourced = snapshot.optIn
        let model = self.model, knowledge = self.knowledge, release = self.release, context = self.context
        let audit = self.audit, index = self.stepIndex, canOptIn = self.released, stateLine = self.stateLine, runToken = self.runToken
        let promptHistory = Self.promptHistory(history, areas: allowedAreas)
        let state = RunState(snapshot: snapshot)
        let output = AssistantRunOutput(release: release, generation: snapshot.generation, continuation: out)
        let work = Task<Outcome, Never> {
            await Self.generate(request, model: model, knowledge: knowledge, config: runConfig, snapshot: snapshot, release: release,
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
        remember(rid, state)
        active[rid] = nil

        if let failure {
            output.sendUnchecked(.failed(requestId: rid, failure))
        } else if case .finished(let text, let firstMs, let totalMs, _) = outcome {
            let record = state.snapshot
            let covering = Self.distinct(record.sentences.filter { $0.status == .grounded }.flatMap(\.sources))
            let done = AssistantAnswer(
                text: text, sources: covering, general: covering.isEmpty, firstSentenceMs: firstMs, totalMs: totalMs,
                unsourced: record.counts.unsourced > 0, counts: record.counts, round: record.round, generation: record.generation)
            if !output.send(.done(requestId: rid, done)).ok {
                state.markRevoked()
                state.update { $0.failure = .revoked }
                output.sendUnchecked(.failed(requestId: rid, .revoked))
            }
        }
    }

    private func remember(_ id: String, _ state: RunState) {
        results[id] = state
        resultOrder.removeAll { $0 == id }
        resultOrder.append(id)
        while resultOrder.count > Self.maxRemembered {
            let old = resultOrder.removeFirst()
            results[old] = nil
            release.unregister(old)
        }
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

        // Pflichtzeilen, fail-closed: Die Standzeile des Öffnens, `start` (vor Abruf und Bestandstiteln) und später `round` und
        // `areas` stehen DAUERHAFT in der Datei, bevor das Modell läuft oder etwas ausgegeben wird (`await` auf das Schreiben,
        // nicht nur Einreihen), bei JEDEM Modell. Ein Fehler: kein Modell, keine Ausgabe, Ausgang „Fehler“. Ein Prozessende
        // nach `start` hinterlässt die Anfrage; das nächste Öffnen schließt sie als unvollständig.
        state.update { $0.contextBefore = context.mode }
        if let audit {
            guard await stateLine.ensure(audit) else { return fail(.auditUnavailable) }
            if Task.isCancelled { return .cancelled }
            let started = await audit.begin(startLine(rid, session: session, config: config, record: state.snapshot), run: run)
            state.update { $0.journaled = started }
            guard started else { return fail(.auditUnavailable) }
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
        if let audit {
            var l = AssistantJournalLine(.round, req: rid)
            l.round = kind
            l.reason = decision.reason
            l.gen = snapshot.generation
            audit.enqueue(l)
            if await audit.waitUntilWritten(request: rid) != nil { return fail(.auditUnavailable) }
            if Task.isCancelled { return .cancelled }
        }

        let now = Date()
        let clockSentences = AssistantGrounding.clockSentences(now: now, timeZone: .current)

        if inventoryQuestion {
            guard output.send(.sources(requestId: rid, generation: snapshot.generation, [])).ok else { return .cancelled }
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
            guard await emitter.noteAreas(groups.map(\.area)) else { return fail(.auditUnavailable) }
            if Task.isCancelled { return .cancelled }
            let preface = AssistantGrounding.inventoryAnnouncement
            switch await emitter.emit(preface, speech: preface, status: .announcement) {
            case .revoked: return .cancelled
            case .failed: return await finished(.auditUnavailable)
            case .delivered: break
            }
            for s in statements {
                switch await emitter.emit(s, speech: emitter.speech(s), status: .inventory) {
                case .revoked: return .cancelled
                case .failed: return await finished(.auditUnavailable)
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
                guard await emitter.noteAreas(used.map(\.area)) else { return fail(.auditUnavailable) }
                if Task.isCancelled { return .cancelled }
                guard output.send(.sources(requestId: rid, generation: snapshot.generation, used)).ok else { return .cancelled }
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
}
