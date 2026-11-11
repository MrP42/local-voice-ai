import Foundation

// Gesprächsmodus „Sprechen“ (Paket K, erweitert in Paket A um „Antworten ohne Quelle“, E4.2a): Ablauf einer Antwort.
// Abruf → Rundenart (`AssistantTurnPolicy`, vor dem Modellaufruf) → Ereignis `sources` → Modellstrom → Satztrenner (nur ganze
// Sätze) → Satzprüfung (`AssistantGrounding.judge`) → `sentence`-Ereignisse → `done`. Muster: `BookTalkSession` (Strom,
// Abbruch, Wächter). Mehrere Anfragen dürfen gleichzeitig laufen (Vorarbeit und Abbruch); keine schreibt in den Verlauf, das
// macht allein `commit`.
//
// Zwei Kanäle je Runde:
// - DATENRUNDE (Standard, mit und ohne Opt-in): Das Modell wählt, das System gibt aus. Je vollständigem Modellsatz rastet der
//   beste Quellsatz der Auszüge ein und wird im Originalwortlaut ausgegeben (Vorspann „Aus deinen Inhalten, ‚Titel‘:“, höchstens
//   drei Zitate, keine Doppelten, danach bricht das Modell ab). Rastet nichts ein, kommt ein deterministischer Rückfall gegen
//   die Frage oder der feste Satz. Bestandsfragen beantwortet das System aus den Bestandsdaten, ohne Modell.
// - ALLGEMEINRUNDE (nur mit Opt-in, Prompt ohne Auszüge und Bestand): Vorlage → neutral, Eigendatenbezug → verworfen, sonst
//   „ohne Quelle“ mit Ansage.
//
// Widerruf: Jede Änderung von Opt-in, Bereichen oder Cloud-Zustimmung erhöht die Freigabe-Generation und bricht alle
// laufenden Antworten ab (Ausgang „widerrufen“). Jedes Ereignis trägt die Generation seines Laufs und wird unmittelbar vor
// dem Senden gegen sie geprüft (`AssistantRelease`).

/// Ein ausgegebener Satz (ohne Ansagen) mit Einordnung und Quellen, für `commit`.
struct AssistantEmitted: Sendable, Equatable {
    let text: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
    let quote: Bool
}

/// Was eine Anfrage hervorgebracht hat; `commit` macht daraus den Beitrag und den Protokolleintrag.
struct AssistantRunRecord: Sendable {
    var round: AssistantRoundKind = .data
    /// Die Rundenart wurde festgelegt (sonst endete die Anfrage davor).
    var decided = false
    /// Freigabe-Generation beim Start.
    var generation = 0
    /// „Ohne Quelle“ war bei Beginn der Anfrage erlaubt.
    var optInAtStart = false
    var used: [AssistantPassage] = []
    var sentences: [AssistantEmitted] = []
    var counts = AssistantSentenceCounts()
    var trace: [AssistantTraceStep] = []
    var traceOverflow = 0
    /// Laufende Nummer der Einordnungen.
    var steps = 0
    /// Ereignis-Nr. (Reihenfolge der `sentence`-Ereignisse) → Index in `trace`; -1: nicht in der Spur.
    var eventSteps: [Int] = []
    var revoked = false
    var cancelled = false
    var failure: AssistantFailure?

    /// Trägt eine Einordnung in die Satzspur ein. Gibt den Index in `trace` zurück; `nil` über der Obergrenze (nur gezählt).
    mutating func note(_ verdict: AssistantTraceVerdict, generation: Int) -> Int? {
        steps += 1
        guard trace.count < AssistantAuditEntry.maxTrace else {
            traceOverflow += 1
            return nil
        }
        trace.append(AssistantTraceStep(n: steps, verdict: verdict, generation: generation))
        return trace.count - 1
    }
}

/// Gemeinsamer Zustand einer Anfrage zwischen Arbeit, Wächter und Sitzung.
private final class RunState: @unchecked Sendable {
    private let lock = NSLock()
    private var lastProgress = Date()
    private var timedOut = false
    private var record: AssistantRunRecord

    init(generation: Int, optIn: Bool) {
        record = AssistantRunRecord()
        record.generation = generation
        record.optInAtStart = optIn
    }

    func touch() { lock.lock(); lastProgress = Date(); lock.unlock() }
    var idle: TimeInterval { lock.lock(); defer { lock.unlock() }; return Date().timeIntervalSince(lastProgress) }
    func fireTimeout() { lock.lock(); timedOut = true; lock.unlock() }
    var didTimeOut: Bool { lock.lock(); defer { lock.unlock() }; return timedOut }
    func update(_ body: (inout AssistantRunRecord) -> Void) { lock.lock(); body(&record); lock.unlock() }
    var snapshot: AssistantRunRecord { lock.lock(); defer { lock.unlock() }; return record }
    func markRevoked() { update { $0.revoked = true } }
    func markCancelled() { update { $0.cancelled = true } }
}

/// Rundenart der zuletzt begonnenen Anfrage (für „keine Übernahme aus der Vorrunde“), auch wenn deren Beitrag noch nicht im
/// Verlauf steht.
private final class RoundMemory: @unchecked Sendable {
    private let lock = NSLock()
    private var value: AssistantRoundKind?

    init(_ value: AssistantRoundKind?) { self.value = value }
    var kind: AssistantRoundKind? { lock.lock(); defer { lock.unlock() }; return value }
    func set(_ kind: AssistantRoundKind?) { lock.lock(); value = kind; lock.unlock() }
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

    /// Sendet nur, wenn die Generation noch gilt. `false`: veraltet, nichts gesendet.
    func send(_ event: AssistantEvent) -> Bool {
        release.ifCurrent(generation) { continuation.yield(event) }
    }

    /// Fehler und Abbruch tragen keine Inhalte und gehen immer hinaus.
    func sendUnchecked(_ event: AssistantEvent) { continuation.yield(event) }
}

/// Gibt die Sätze eines Laufs aus und führt Buch: Zähler, Satzspur, Zitate, Vorspann. Nur innerhalb des Laufs benutzt.
private final class RunEmitter {
    enum Flow { case next, enough, revoked }

    let rid: String
    let generation: Int
    let release: AssistantRelease
    let output: AssistantRunOutput
    let state: RunState
    let clock = ContinuousClock()
    let started: ContinuousClock.Instant

    var texts: [String] = []
    var firstMs: Int?
    var lastSource: String?
    var taken = Set<String>()
    var quotes = 0
    var dropped = 0
    var unsourcedEmitted = 0
    var announcedUnsourced = false

    /// Höchstens so viele Zitate je Antwort; danach wird das Modell abgebrochen.
    static let maxQuotes = 3

    init(rid: String, generation: Int, release: AssistantRelease, output: AssistantRunOutput, state: RunState,
         started: ContinuousClock.Instant) {
        self.rid = rid
        self.generation = generation
        self.release = release
        self.output = output
        self.state = state
        self.started = started
    }

    private static func milliseconds(_ d: Duration) -> Int {
        Int(d.components.seconds) * 1000 + Int(d.components.attoseconds / 1_000_000_000_000_000)
    }

    var elapsedMs: Int { Self.milliseconds(started.duration(to: clock.now)) }

    func speech(_ text: String) -> String {
        AssistantSpeech.render(text, lineStart: false)?.speech ?? text
    }

    @discardableResult
    func emit(_ text: String, speech: String, status: AssistantSentenceStatus, sources: [AssistantPassage] = [], quote: Bool = false) -> Bool {
        let delivered = output.send(.sentence(requestId: rid, generation: generation, text: text, speech: speech, status: status,
                                              sources: sources, quote: quote))
        let checked = delivered ? generation : release.currentGeneration
        let gen = generation
        let announcement = status == .announcement
        if delivered, firstMs == nil { firstMs = elapsedMs }
        state.update { r in
            let step = r.note(AssistantTraceVerdict(status: status), generation: gen)
            if delivered {
                r.eventSteps.append(step ?? -1)
                switch status {
                case .grounded: r.counts.grounded += 1
                case .inventory: r.counts.inventory += 1
                case .unsourced: r.counts.unsourced += 1
                case .neutral: r.counts.neutral += 1
                case .announcement: break
                }
                if !announcement { r.sentences.append(AssistantEmitted(text: text, status: status, sources: sources, quote: quote)) }
            } else if !announcement {
                r.counts.drop(.revoked)
            }
            if let i = step { r.trace[i].commits.append(AssistantCommitMark(stage: .emit, ok: delivered, generation: checked)) }
        }
        if delivered, !announcement { texts.append(text) }
        return delivered
    }

    func drop(_ reason: AssistantDropReason) {
        dropped += 1
        let gen = generation
        state.update { r in
            r.counts.drop(reason)
            _ = r.note(AssistantTraceVerdict(dropped: reason), generation: gen)
        }
    }

    /// Originalsatz mit Vorspann bei jedem Quellenwechsel (also vor dem ersten Zitat je Quelle und erneut, wenn nach einer
    /// anderen Quelle wieder zu ihr zurückgekehrt wird: sonst wäre ein Zitat der falschen Quelle zugeordnet).
    func emitQuote(_ q: AssistantGrounding.SourceSentence) -> Flow {
        if q.sourceKey != lastSource {
            let preface = AssistantGrounding.quotePreface(for: q.passage)
            guard emit(preface, speech: preface, status: .announcement) else { return .revoked }
            lastSource = q.sourceKey
        }
        guard emit(q.text, speech: speech(q.text), status: .grounded, sources: [q.passage], quote: true) else { return .revoked }
        taken.insert(q.key)
        quotes += 1
        return quotes >= Self.maxQuotes ? .enough : .next
    }

    func emitFixed(optIn: Bool) -> Flow {
        let s = AssistantGrounding.fixedSentence(allowUnsourced: optIn)
        return emit(s, speech: s, status: .neutral) ? .next : .revoked
    }

    /// Prüft einen vollständigen Modellsatz der Datenrunde.
    func handleData(_ text: String, basis: AssistantGrounding.Basis, titles: [String], clock: [String]) -> Flow {
        switch AssistantGrounding.judge(text, round: .data, basis: basis, titles: titles, clock: clock,
                                        optIn: release.isOptedIn, taken: taken) {
        case .quote(let q): return emitQuote(q)
        case .template(let t): return emit(t, speech: t, status: .neutral) ? .next : .revoked
        case .dropped(let reason): drop(reason); return .next
        case .unsourced: drop(.notSnapped); return .next
        }
    }

    /// Prüft einen vollständigen Modellsatz der Allgemeinrunde.
    func handleGeneral(_ text: String, speech: String, titles: [String], clock: [String]) -> Flow {
        switch AssistantGrounding.judge(text, round: .general, basis: AssistantGrounding.Basis(passages: []), titles: titles,
                                        clock: clock, optIn: release.isOptedIn) {
        case .template(let t): return emit(t, speech: t, status: .neutral) ? .next : .revoked
        case .dropped(let reason): drop(reason); return .next
        case .quote: drop(.notSnapped); return .next
        case .unsourced:
            if !announcedUnsourced {
                let a = AssistantGrounding.unsourcedAnnouncement
                guard emit(a, speech: a, status: .announcement) else { return .revoked }
                announcedUnsourced = true
            }
            guard emit(text, speech: speech, status: .unsourced) else { return .revoked }
            unsourcedEmitted += 1
            return .next
        }
    }
}

public actor AssistantSession {
    public static let maxStoredTurns = 200
    /// Höchstens so viele Auszüge je Frage.
    static let passageLimit = 8
    /// Wie lange `commit` auf den geordneten Abschluss eines Laufs wartet (hängendes Modell).
    static let commitWait: Duration = .seconds(2)

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
    private let rounds: RoundMemory
    private let ledger = AssistantOutputLedger()
    private let audit: AssistantAuditLog?
    private var active: [String: ActiveRun] = [:]
    private var earlyCancel: Set<String> = []
    private var results: [String: AssistantRunRecord] = [:]
    private var resultOrder: [String] = []
    public private(set) var history: [AssistantTurn]

    public init(model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig, history: [AssistantTurn] = [],
                audit: AssistantAuditLog? = nil) {
        self.model = model
        self.knowledge = knowledge
        self.config = config
        self.release = AssistantRelease(optIn: config.allowUnsourced)
        let kept = history.count > Self.maxStoredTurns ? Array(history.suffix(Self.maxStoredTurns)) : history
        self.rounds = RoundMemory(Self.lastRound(of: kept))
        self.audit = audit
        self.history = kept
    }

    private static func lastRound(of turns: [AssistantTurn]) -> AssistantRoundKind? {
        turns.last.map { $0.round ?? .data }
    }

    // MARK: Freigabe

    /// Ob unbelegte Sätze (Allgemeinwissen) gesprochen werden dürfen. Ohne Aktor-Sprung lesbar (hinter einem Lock).
    public nonisolated var allowsUnsourced: Bool { release.isOptedIn }

    /// Aktuelle Freigabe-Generation (monoton). Ohne Aktor-Sprung lesbar; der Controller prüft sie vor Anzeige und Audiostart.
    public nonisolated var releaseGeneration: Int { release.currentGeneration }

    /// Schaltet „Antworten ohne Quelle“ um. Jede Änderung erhöht die Generation und bricht alle laufenden Antworten ab (Ausgang
    /// „widerrufen“), ohne Aktor-Sprung und sofort. Gibt die Generation danach zurück (für den Protokolleintrag).
    @discardableResult
    public nonisolated func setAllowUnsourced(_ on: Bool) -> Int { release.setOptIn(on) }

    /// Eine Freigabe hat sich geändert (Cloud-Zustimmung, oder Bereiche außerhalb von `setAreas`): Generation hoch, laufende
    /// Antworten abbrechen. Gibt die neue Generation zurück.
    @discardableResult
    public nonisolated func releaseDidChange() -> Int { release.bump() }

    /// Ändert die Bereiche und die Generation. Erst die Generation (laufende Antworten enden sofort), dann die Bereiche, dann
    /// noch einmal die Generation: Eine Anfrage, die dazwischen mit alten Bereichen begann, wird ebenfalls widerrufen.
    @discardableResult
    public func setAreas(_ areas: Set<AssistantArea>) async -> Int {
        release.bump()
        await knowledge.setAreas(areas)
        return release.bump()
    }

    /// Der Controller meldet einen Ausgabe-Commit (Anzeige, Audiostart) eines Ereignisses (`index` = Nummer in der Reihenfolge
    /// der empfangenen `sentence`-Ereignisse dieser Anfrage), mit der dabei geprüften Generation. `ok`: ausgeführt; sonst
    /// verworfen. Die Meldung geht in die Satzspur des Protokolleintrags.
    public nonisolated func noteCommit(requestId: String, index: Int, stage: AssistantCommitStage, ok: Bool, generation: Int) {
        ledger.add(requestId, .init(index: index, mark: AssistantCommitMark(stage: stage, ok: ok, generation: generation)))
    }

    // MARK: Verlauf

    public func restore(history turns: [AssistantTurn]) {
        history = turns.count > Self.maxStoredTurns ? Array(turns.suffix(Self.maxStoredTurns)) : turns
        rounds.set(Self.lastRound(of: history))
    }

    public func clearHistory() {
        history.removeAll()
        rounds.set(nil)
    }

    /// Cloud-Schutz: Die Beiträge, die als VERLAUF ins Modell dürfen. Lokal alle; bei einem Cloud-Modell nur Beiträge ohne
    /// Quellen oder mit Quellen ausschließlich aus den für die Cloud freigegebenen Bereichen (`knowledge.areas`, dieselben
    /// wie beim Abruf): Gesprochenes aus einem gesperrten Bereich verlässt das Gerät nicht. `history` selbst (Anzeige,
    /// Datei) bleibt vollständig; die Auswahl gilt je Anfrage und folgt damit jeder Änderung der Bereiche.
    static func promptHistory(_ turns: [AssistantTurn], areas: Set<AssistantArea>?) -> [AssistantTurn] {
        guard let areas else { return turns }
        return turns.filter { turn in turn.sources.allSatisfy { areas.contains($0.area) } }
    }

    /// Zuordnung des tatsächlich Gesprochenen zu den ausgegebenen Sätzen (der Reihe nach, Ansagen zählen nicht). Ein Satz, der
    /// nur angesprochen wurde (Unterbrechung), zählt mit seiner Einordnung. Gesprochenes ohne Entsprechung (kommt bei einem
    /// vollständigen Lauf nicht vor) wird nie als „ohne Quelle“ gewertet, sondern als Vorlage ohne Quelle.
    static func attribute(spoken: String, record: AssistantRunRecord?)
        -> [(text: String, status: AssistantSentenceStatus, sources: [AssistantPassage], quote: Bool)] {
        var rest = Substring(spoken)
        var out: [(text: String, status: AssistantSentenceStatus, sources: [AssistantPassage], quote: Bool)] = []
        for e in record?.sentences ?? [] where e.status != .announcement {
            rest = rest.drop(while: { $0.isWhitespace })
            if rest.isEmpty { break }
            if rest.hasPrefix(e.text) {
                out.append((e.text, e.status, e.sources, e.quote))
                rest = rest.dropFirst(e.text.count)
            } else if e.text.hasPrefix(rest) {
                out.append((String(rest), e.status, e.sources, e.quote))
                rest = ""
            }
        }
        rest = rest.drop(while: { $0.isWhitespace })
        if !rest.isEmpty { out.append((String(rest), .neutral, [], false)) }
        return out
    }

    private static func distinct(_ passages: [AssistantPassage]) -> [AssistantPassage] {
        var seen = Set<String>()
        return passages.filter { seen.insert("\($0.area.rawValue)/\($0.itemId)/\($0.locator ?? "")").inserted }
    }

    /// Wartet (begrenzt) auf den geordneten Abschluss des Laufs: erst Abbruch, dann Endstand.
    private func settle(_ requestId: String) async -> AssistantRunRecord? {
        guard let run = active[requestId] else { return results.removeValue(forKey: requestId) }
        run.state.markCancelled()
        run.stop()
        let deadline = ContinuousClock.now.advanced(by: Self.commitWait)
        while active[requestId] != nil, ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
        if active[requestId] != nil { return run.state.snapshot }   // hängendes Modell: Stand bis hierher
        return results.removeValue(forKey: requestId)
    }

    /// Nach `done` oder nach Barge-in (dann `spoken` = tatsächlich Gesprochenes, `interrupted` = true) trägt die Steuerung
    /// den Beitrag ein. Für JEDE Anfrage entsteht genau ein Protokolleintrag, auch ohne gesprochenen Text (Fehler, Abbruch,
    /// Widerruf); einen Beitrag im Verlauf gibt es nur mit Nutzertext, mit tatsächlich Gesprochenem, mit bekanntem Lauf und
    /// bei noch gültiger Freigabe-Generation (der Verlaufs-Commit wird erneut geprüft). Läuft die Anfrage noch, bricht `commit`
    /// sie ab und wartet auf ihren Endstand, bevor es schreibt: kein verspäteter Stand, keine falsche Zuordnung. Quellen
    /// (nur die, die einen gesprochenen Satz decken) kommen aus der Prüfung dieser Anfrage. `false`, wenn nur das Protokoll
    /// nicht geschrieben werden konnte (der Beitrag steht trotzdem im Verlauf).
    @discardableResult
    public func commit(requestId: String, user: String, spoken: String, interrupted: Bool, revoked: Bool = false) async -> Bool {
        let record = await settle(requestId)
        let notes = ledger.take(requestId)
        guard var record else { return true }   // unbekannte oder schon abgeschlossene Anfrage
        let text = user.trimmingCharacters(in: .whitespacesAndNewlines)
        let said = spoken.trimmingCharacters(in: .whitespacesAndNewlines)

        let current = release.currentGeneration
        var historyMark: AssistantCommitMark?
        var sources: [AssistantPassage] = []
        if !text.isEmpty, !said.isEmpty {
            let valid = record.generation == current && !revoked && !record.revoked
            historyMark = AssistantCommitMark(stage: .history, ok: valid, generation: current)
            if valid {
                let parts = Self.attribute(spoken: said, record: record)
                sources = Self.distinct(parts.filter { $0.status == .grounded }.flatMap(\.sources))
                let anyUnsourced = parts.contains { $0.status == .unsourced }
                let anyQuote = parts.contains { $0.quote }
                var turnParts: [AssistantTurnPart] = []
                if anyQuote || (anyUnsourced && parts.contains { $0.status != .unsourced }) {
                    for p in parts {
                        let flag = p.status == .unsourced
                        if !p.quote, let last = turnParts.last, !last.quote, last.unsourced == flag {
                            turnParts[turnParts.count - 1].text += " " + p.text
                        } else {
                            turnParts.append(AssistantTurnPart(text: p.text, unsourced: flag, quote: p.quote,
                                                               source: p.quote ? p.sources.first?.sourceLabel : nil))
                        }
                    }
                }
                history.append(AssistantTurn(
                    user: text, assistant: said, interrupted: interrupted, sources: sources, general: sources.isEmpty,
                    unsourced: anyUnsourced, parts: turnParts, model: config.modelLabel, at: Date(),
                    round: record.decided ? record.round : nil))
                if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }
            }
        }

        guard let audit else { return true }
        for n in notes where n.index >= 0 && n.index < record.eventSteps.count {
            let step = record.eventSteps[n.index]
            if step >= 0 { record.trace[step].commits.append(n.mark) }
        }
        let outcome: AssistantAuditEntry.Outcome
        if revoked || record.revoked || record.failure == .revoked || historyMark?.ok == false {
            outcome = .revoked   // auch ein fertiger Lauf, dessen Verlaufs-Commit die geänderte Freigabe verwirft
        } else if let failure = record.failure {
            outcome = failure == .cancelled ? .interrupted : .failed
        } else {
            outcome = interrupted || record.cancelled ? .interrupted : .completed
        }
        let areas = Self.distinct(record.sentences.filter { $0.status == .grounded }.flatMap(\.sources)).map(\.area)
        let entry = AssistantAuditEntry(
            kind: .answer, model: config.modelLabel, cloud: config.isCloud, unsourcedMode: record.optInAtStart,
            consentVersion: config.consentVersion, counts: record.counts, areas: areas, outcome: outcome,
            round: record.decided ? record.round : nil, generation: record.generation, trace: record.trace,
            traceOverflow: record.traceOverflow, history: historyMark)
        return await audit.record(entry)
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
        // Zuerst die Freigabe, dann die Bereiche: Ändert sich eine davon dazwischen, ist die Generation veraltet und der Lauf
        // wird beim Anmelden widerrufen. Vor der Abbruchprüfung, damit zwischen Prüfung und Eintrag in `active` kein
        // Wartepunkt liegt.
        let snapshot = self.release.snapshot
        var allowedAreas: Set<AssistantArea>?
        if self.config.isCloud { allowedAreas = await self.knowledge.areas }
        if earlyCancel.remove(rid) != nil {
            out.yield(.failed(requestId: rid, .cancelled))
            if let audit {
                await audit.record(AssistantAuditEntry(
                    kind: .answer, model: config.modelLabel, cloud: config.isCloud, unsourcedMode: snapshot.optIn,
                    consentVersion: config.consentVersion, outcome: .interrupted, generation: snapshot.generation))
            }
            return
        }
        var runConfig = self.config
        runConfig.allowUnsourced = snapshot.optIn
        let model = self.model, knowledge = self.knowledge, release = self.release, rounds = self.rounds
        let promptHistory = Self.promptHistory(history, areas: allowedAreas)
        let state = RunState(generation: snapshot.generation, optIn: snapshot.optIn)
        let output = AssistantRunOutput(release: release, generation: snapshot.generation, continuation: out)
        let work = Task<Outcome, Never> {
            await Self.generate(request, model: model, knowledge: knowledge, config: runConfig, snapshot: snapshot, release: release,
                                rounds: rounds, history: promptHistory, state: state, output: output)
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
        release.unregister(rid)

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
        remember(rid, state.snapshot)
        active[rid] = nil

        if let failure {
            output.sendUnchecked(.failed(requestId: rid, failure))
        } else if case .finished(let text, let firstMs, let totalMs, _) = outcome {
            let record = state.snapshot
            let covering = Self.distinct(record.sentences.filter { $0.status == .grounded }.flatMap(\.sources))
            let done = AssistantAnswer(
                text: text, sources: covering, general: covering.isEmpty, firstSentenceMs: firstMs, totalMs: totalMs,
                unsourced: record.counts.unsourced > 0, counts: record.counts, round: record.round, generation: record.generation)
            if !output.send(.done(requestId: rid, done)) {
                state.markRevoked()
                state.update { $0.failure = .revoked }
                remember(rid, state.snapshot)
                output.sendUnchecked(.failed(requestId: rid, .revoked))
            }
        }
    }

    private func remember(_ id: String, _ record: AssistantRunRecord) {
        results[id] = record
        resultOrder.removeAll { $0 == id }
        resultOrder.append(id)
        while resultOrder.count > 16 { results[resultOrder.removeFirst()] = nil }
    }

    private static func generate(
        _ request: AssistantRequest, model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig,
        snapshot: AssistantRelease.Snapshot, release: AssistantRelease, rounds: RoundMemory, history: [AssistantTurn],
        state: RunState, output: AssistantRunOutput
    ) async -> Outcome {
        let rid = request.requestId
        let budget = AssistantPrompt.budget(config: config)
        let emitter = RunEmitter(rid: rid, generation: snapshot.generation, release: release, output: output, state: state,
                                 started: ContinuousClock().now)
        func finished(_ failure: AssistantFailure? = nil) -> Outcome {
            .finished(text: emitter.texts.joined(separator: " "), firstMs: emitter.firstMs, totalMs: emitter.elapsedMs, failure: failure)
        }

        let passages = await knowledge.retrieve(question: request.text, previous: history.last?.user, limit: passageLimit, maxChars: budget.passages)
        if Task.isCancelled { return .cancelled }
        let titles = await knowledge.inventoryTitles()
        if Task.isCancelled { return .cancelled }
        state.touch()

        // Rundenart, deterministisch vor dem Modellaufruf. Bestandsfragen sind immer Datenrunden.
        let inventoryQuestion = AssistantKnowledge.isInventoryQuestion(request.text)
        let kind: AssistantRoundKind = inventoryQuestion ? .data
            : AssistantTurnPolicy.decide(question: request.text, allowUnsourced: snapshot.optIn, titles: titles, passages: passages,
                                         previous: rounds.kind).kind
        rounds.set(kind)
        state.update { $0.round = kind; $0.decided = true }

        let now = Date()
        let clockSentences = AssistantGrounding.clockSentences(now: now, timeZone: .current)

        if inventoryQuestion {
            guard output.send(.sources(requestId: rid, generation: snapshot.generation, [])) else { return .cancelled }
            let statements = await knowledge.inventoryStatements(question: request.text)
            if Task.isCancelled { return .cancelled }
            if statements.isEmpty {
                if emitter.emitFixed(optIn: snapshot.optIn) == .revoked { return .cancelled }
                return finished()
            }
            let preface = AssistantGrounding.inventoryAnnouncement
            guard emitter.emit(preface, speech: preface, status: .announcement) else { return .cancelled }
            for s in statements {
                guard emitter.emit(s, speech: emitter.speech(s), status: .inventory) else { return .cancelled }
            }
            return finished()
        }

        let instructions = kind == .data
            ? AssistantPrompt.dataInstructions(config: config, now: now, timeZone: .current)
            : AssistantPrompt.generalInstructions(config: config, now: now, timeZone: .current)
        var scale = 1.0
        var includeHistory = true
        var announcedSources = false
        var retried = false

        while true {
            var prompt: String
            var used: [AssistantPassage] = []
            if kind == .data {
                let built = AssistantPrompt.buildData(question: request.text, afterInterruption: request.afterInterruption, passages: passages,
                                                      history: history, config: config, passageScale: scale, includeHistory: includeHistory)
                prompt = built.prompt
                used = built.used
            } else {
                prompt = AssistantPrompt.buildGeneral(question: request.text, afterInterruption: request.afterInterruption,
                                                      history: history, config: config, includeHistory: includeHistory)
            }
            if !announcedSources || used != state.snapshot.used {
                state.update { $0.used = used }
                guard output.send(.sources(requestId: rid, generation: snapshot.generation, used)) else { return .cancelled }
                announcedSources = true
            }
            let basis = AssistantGrounding.Basis(passages: used)

            var splitter = AssistantSpeech.Splitter(sentencesOnly: true)
            var seen = 0
            var failure: AssistantFailure?
            var flow = RunEmitter.Flow.next

            func check(_ sentences: [(text: String, speech: String)]) -> RunEmitter.Flow {
                for s in sentences {
                    seen += 1
                    let f = kind == .data
                        ? emitter.handleData(s.text, basis: basis, titles: titles, clock: clockSentences)
                        : emitter.handleGeneral(s.text, speech: s.speech, titles: titles, clock: clockSentences)
                    if f != .next { return f }
                }
                return .next
            }

            do {
                for try await piece in model.stream(instructions: instructions, prompt: prompt, maxTokens: config.maxAnswerTokens) {
                    if Task.isCancelled { return .cancelled }
                    state.touch()
                    flow = check(splitter.feed(piece))
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
            if failure == nil, flow == .next {
                // Stromende: der Rest ist der letzte Satz.
                flow = check(splitter.finish())
                if flow == .revoked { return .cancelled }
                for _ in 0..<splitter.overlong { emitter.drop(.template) }
                if seen == 0, splitter.overlong == 0 { failure = .modelFailed }
            }
            if failure == nil, flow != .revoked, emitter.dropped > 0 {
                // Nichts Belegtes blieb übrig: Rückfall gegen die Frage, sonst der feste Satz.
                if kind == .data, emitter.quotes == 0 {
                    let fallback = AssistantGrounding.fallback(question: request.text, basis: basis, taken: emitter.taken)
                    let f = fallback.map { emitter.emitQuote($0) } ?? emitter.emitFixed(optIn: snapshot.optIn)
                    if f == .revoked { return .cancelled }
                } else if kind == .general, emitter.unsourcedEmitted == 0 {
                    if emitter.emitFixed(optIn: true) == .revoked { return .cancelled }
                }
            }
            return finished(failure)
        }
    }
}
