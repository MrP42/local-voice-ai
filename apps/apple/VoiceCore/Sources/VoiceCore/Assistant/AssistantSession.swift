import Foundation

// Gesprächsmodus „Sprechen“ (Paket K, erweitert in Paket A um „Antworten ohne Quelle“, E4.2a): Ablauf einer Antwort.
// Abruf → Rundenart (`AssistantContext`, vor dem Modellaufruf) → Protokollzeile `start` → Ereignis `sources` → Modellstrom →
// Satztrenner (nur ganze Sätze, Reste ohne Satzzeichen werden verworfen) → Satzprüfung (`AssistantGrounding.judge`) →
// `sentence`-Ereignisse → `done`. Muster: `BookTalkSession` (Strom, Abbruch, Wächter). Mehrere Anfragen dürfen gleichzeitig
// laufen (Vorarbeit und Abbruch); keine schreibt in den Verlauf, das macht allein `commit`.
//
// Zwei Kanäle je Runde:
// - DATENRUNDE (Standard, mit und ohne Opt-in): Das Modell wählt, das System gibt aus. Je vollständigem Modellsatz rastet der
//   beste Quellsatz der Auszüge ein und wird im Originalwortlaut ausgegeben (Vorspann „Aus deinen Inhalten, ‚Titel‘:“, höchstens
//   drei Zitate, keine Doppelten, danach bricht das Modell ab). Rastet nichts ein, kommt ein deterministischer Rückfall gegen
//   die Frage oder der feste Satz. Bestandsfragen beantwortet das System aus den Bestandsdaten, ohne Modell.
// - ALLGEMEINRUNDE (nur mit Opt-in und nur im Zustand `.free`, Prompt ohne Auszüge und Bestand): Vorlage → neutral,
//   Eigendatenbezug oder Ähnlichkeit mit eigenem Inhalt → verworfen, sonst „ohne Quelle“ mit Ansage.
//
// Widerruf: Jede Änderung von Opt-in, Bereichen, Cloud-Zustimmung oder des Zustands (`resumeFree`) erhöht die Freigabe-
// Generation, bricht alle angemeldeten Läufe ab (Ausgang „widerrufen“) und ruft den Beobachter (Controller: Wiedergabe stoppen,
// Warteschlange leeren). Ein Lauf bleibt bis `commit` angemeldet. Jedes Ereignis trägt die Generation seines Laufs und wird
// unmittelbar vor dem Senden gegen sie geprüft; der Verlaufseintrag entsteht im selben Lock (`AssistantRelease`).

/// Ein ausgegebener Satz (ohne Ansagen) mit Einordnung und Quellen, für `commit`.
struct AssistantEmitted: Sendable, Equatable {
    let text: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
    let quote: Bool
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
    var counts = AssistantSentenceCounts()
    /// Bereiche der Bestandsangaben (gehen mit ins Protokoll).
    var inventoryAreas: [AssistantArea] = []
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

    /// Sendet nur, wenn die Generation noch gilt. `false`: veraltet, nichts gesendet. `before` läuft im selben Lock unmittelbar
    /// vor dem Senden (kurz, ohne Wartezeit): so steht die Protokollzeile des Kerns vor jeder Rückmeldung des Controllers.
    func send(_ event: AssistantEvent, before: () -> Void = {}) -> Bool {
        release.ifCurrent(generation) {
            before()
            continuation.yield(event)
        }
    }

    /// Fehler und Abbruch tragen keine Inhalte und gehen immer hinaus.
    func sendUnchecked(_ event: AssistantEvent) { continuation.yield(event) }
}

/// Gibt die Sätze eines Laufs aus und führt Buch: Zähler, Zitate, Vorspann und die Protokollzeilen `step` und `commit`.
/// Nur innerhalb des Laufs benutzt.
private final class RunEmitter {
    enum Flow { case next, enough, revoked, limit }

    let rid: String
    let generation: Int
    let release: AssistantRelease
    let output: AssistantRunOutput
    let state: RunState
    let journal: AssistantAuditLog?
    let index: AssistantStepIndex
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
    /// Die Schrittgrenze ist erreicht: die Antwort endet.
    var reachedLimit = false

    /// Höchstens so viele Zitate je Antwort; danach wird das Modell abgebrochen.
    static let maxQuotes = 3

    init(rid: String, generation: Int, release: AssistantRelease, output: AssistantRunOutput, state: RunState,
         journal: AssistantAuditLog?, index: AssistantStepIndex, started: ContinuousClock.Instant) {
        self.rid = rid
        self.generation = generation
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

    /// Nummeriert einen Schritt und schreibt seine Zeile.
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

    private func commitLine(_ n: Int, ok: Bool, generation g: Int) -> AssistantJournalLine {
        var l = AssistantJournalLine(.commit, req: rid)
        l.n = n
        l.mark = AssistantCommitMark(stage: .emit, ok: ok, generation: g)
        return l
    }

    @discardableResult
    func emit(_ text: String, speech: String, status: AssistantSentenceStatus, sources: [AssistantPassage] = [], quote: Bool = false) -> Bool {
        let announcement = status == .announcement
        let n = step(AssistantTraceVerdict(status: status))
        // Der Schritt steht im Index, BEVOR das Ereignis hinausgeht: Eine schnelle Rückmeldung des Controllers läuft nie ins Leere.
        index.append(rid, step: n)
        let gen = generation
        let delivered = output.send(.sentence(requestId: rid, generation: gen, text: text, speech: speech, status: status,
                                              sources: sources, quote: quote)) {
            journal?.enqueue(commitLine(n, ok: true, generation: gen))
        }
        if !delivered {
            index.removeLast(rid)
            journal?.enqueue(commitLine(n, ok: false, generation: release.currentGeneration))
        }
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
                if !announcement { r.sentences.append(AssistantEmitted(text: text, status: status, sources: sources, quote: quote)) }
            } else if !announcement {
                r.counts.drop(.revoked)
            }
        }
        if delivered, !announcement { texts.append(text) }
        return delivered
    }

    func drop(_ reason: AssistantDropReason) {
        dropped += 1
        _ = step(AssistantTraceVerdict(dropped: reason))
        state.update { $0.counts.drop(reason) }
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

    /// Prüft einen vollständigen Modellsatz der Allgemeinrunde. Ein Satz ohne Quelle wird zusätzlich gegen die eigenen Inhalte
    /// geprüft: Abruf mit dem Satz als Anfrage; rastet er an einem Quellsatz ein, wird er verworfen.
    func handleGeneral(_ text: String, speech: String, raw: String, titles: [String], clock: [String],
                       knowledge: AssistantKnowledge) async -> Flow {
        switch AssistantGrounding.judge(text, round: .general, basis: AssistantGrounding.Basis(passages: []), titles: titles,
                                        clock: clock, optIn: release.isOptedIn, raw: raw) {
        case .template(let t): return emit(t, speech: t, status: .neutral) ? .next : .revoked
        case .dropped(let reason): drop(reason); return .next
        case .quote: drop(.notSnapped); return .next
        case .unsourced:
            let probed = await knowledge.probe(sentence: text)
            if AssistantGrounding.resemblesOwnContent(text, passages: probed) { drop(.ownData); return .next }
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
    private let stepIndex = AssistantStepIndex()
    private let sessionId = UUID().uuidString
    private let audit: AssistantAuditLog?
    private var active: [String: ActiveRun] = [:]
    private var earlyCancel: Set<String> = []
    private var results: [String: RunState] = [:]
    private var resultOrder: [String] = []
    public private(set) var history: [AssistantTurn]

    /// `released`: Freischaltung der Funktion „Antworten ohne Quelle“ (`AssistantUnsourcedConsent.isReleased`); bei `false`
    /// verhält sich die Sitzung wie mit ausgeschaltetem Opt-in.
    public init(model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig, history: [AssistantTurn] = [],
                audit: AssistantAuditLog? = nil, released: Bool = AssistantUnsourcedConsent.isReleased) {
        let optIn = config.allowUnsourced && released
        self.model = model
        self.knowledge = knowledge
        self.config = config
        self.released = released
        self.release = AssistantRelease(optIn: optIn, consentVersion: config.consentVersion)
        let kept = history.count > Self.maxStoredTurns ? Array(history.suffix(Self.maxStoredTurns)) : history
        self.context = AssistantContext(optIn: optIn, history: kept)
        self.audit = audit
        self.history = kept
    }

    // MARK: Freigabe

    /// Ob unbelegte Sätze (Allgemeinwissen) gesprochen werden dürfen. Ohne Aktor-Sprung lesbar (hinter einem Lock).
    public nonisolated var allowsUnsourced: Bool { release.isOptedIn }

    /// Die Funktion „Antworten ohne Quelle“ ist freigeschaltet (sonst gibt es weder Schalter noch Knopf).
    public nonisolated var isReleased: Bool { released }

    /// Aktuelle Freigabe-Generation (monoton). Ohne Aktor-Sprung lesbar; der Controller prüft sie vor Anzeige und Audiostart.
    public nonisolated var releaseGeneration: Int { release.currentGeneration }

    /// Zustand des Datenkontexts: `.data` (jede Runde antwortet aus den Inhalten) oder `.free` (Runden ohne Quelle möglich).
    public nonisolated var contextMode: AssistantContext.Mode { context.mode }

    /// Der Knopf „Ohne Quelle fortsetzen“ ist sinnvoll: Opt-in an, freigeschaltet und der Zustand `.data`.
    public nonisolated var canResumeFree: Bool { released && release.isOptedIn && context.mode == .data }

    /// Zahl der angemeldeten Läufe (ein Lauf bleibt bis `commit` angemeldet; Tests).
    nonisolated var registeredRunCount: Int { release.registeredCount }

    /// Der Beobachter wird nach JEDER Erhöhung der Generation gerufen (auf dem Thread des Auslösers, ohne Aktor-Sprung): Der
    /// Controller stoppt dann die Wiedergabe und leert die Warteschlange, unabhängig von Quellen und vom Modellende.
    public nonisolated func setReleaseObserver(_ observer: (@Sendable (Int) -> Void)?) { release.setObserver(observer) }

    /// Schaltet „Antworten ohne Quelle“ um. Jede Änderung erhöht die Generation und bricht alle laufenden Antworten ab (Ausgang
    /// „widerrufen“), ohne Aktor-Sprung und sofort. Ein Einschalten setzt den Zustand auf `.free`, aber nur, wenn das Gespräch
    /// noch keine Datenrunde hatte. `consentVersion`: Fassung des Einwilligungstextes (sonst die der Konfiguration); sie wird
    /// bei jeder neuen Anfrage aus der Freigabe gelesen. Gibt die Generation danach zurück (für das Protokoll).
    @discardableResult
    public nonisolated func setAllowUnsourced(_ on: Bool, consentVersion: Int? = nil) -> Int {
        let effective = on && released
        let before = release.isOptedIn
        let generation = release.setOptIn(effective, consentVersion: consentVersion ?? config.consentVersion)
        if before != effective { context.optInChanged(effective) }
        return generation
    }

    /// „Ohne Quelle fortsetzen“: ausdrückliche Nutzerhandlung, die den Zustand `.data` aufhebt. Erhöht die Generation. `false`,
    /// wenn nichts zu tun war (nicht freigeschaltet, Opt-in aus oder schon `.free`).
    @discardableResult
    public nonisolated func resumeFree() -> Bool {
        guard released, context.resumeFree(optIn: release.isOptedIn) else { return false }
        release.bump()
        return true
    }

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
    /// verworfen. Die Meldung geht als Zeile `commit` ins Journal, ohne Warten und ohne I/O im Aufrufer.
    public nonisolated func noteCommit(requestId: String, index: Int, stage: AssistantCommitStage, ok: Bool, generation: Int) {
        guard let audit, let n = stepIndex.step(requestId, index: index) else { return }
        var line = AssistantJournalLine(.commit, req: requestId)
        line.n = n
        line.mark = AssistantCommitMark(stage: stage, ok: ok, generation: generation)
        audit.enqueue(line)
    }

    // MARK: Verlauf

    public func restore(history turns: [AssistantTurn]) {
        history = turns.count > Self.maxStoredTurns ? Array(turns.suffix(Self.maxStoredTurns)) : turns
        context.reset(optIn: release.isOptedIn, history: history)
    }

    public func clearHistory() {
        history.removeAll()
        context.reset(optIn: release.isOptedIn, history: [])
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
    /// bekanntem Lauf und bei noch gültiger Freigabe-Generation: Die Prüfung und das Anhängen geschehen im selben Lock wie die
    /// Erhöhung der Generation. Läuft die Anfrage noch, bricht `commit` sie ab und wartet auf ihren Endstand, bevor es
    /// schreibt. Quellen (nur die, die einen gesprochenen Satz decken) kommen aus der Prüfung dieser Anfrage. Mit dem Commit
    /// endet die Anmeldung des Laufs für den Widerruf. `false`, wenn nur das Protokoll nicht geschrieben werden konnte (der
    /// Beitrag steht trotzdem im Verlauf).
    @discardableResult
    public func commit(requestId: String, user: String, spoken: String, interrupted: Bool, revoked: Bool = false) async -> Bool {
        guard let state = await settle(requestId) else { return true }   // unbekannte oder schon abgeschlossene Anfrage
        release.unregister(requestId)
        let record = state.snapshot
        let text = user.trimmingCharacters(in: .whitespacesAndNewlines)
        let said = spoken.trimmingCharacters(in: .whitespacesAndNewlines)

        var historyMark: AssistantCommitMark?
        if !text.isEmpty, !said.isEmpty {
            let parts = Self.attribute(spoken: said, record: record)
            let sources = Self.distinct(parts.filter { $0.status == .grounded }.flatMap(\.sources))
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
            let turn = AssistantTurn(
                user: text, assistant: said, interrupted: interrupted, sources: sources, general: sources.isEmpty,
                unsourced: anyUnsourced, parts: turnParts, model: config.modelLabel, at: Date(),
                round: record.decided ? record.round : nil)
            // Prüfung der Generation und Anhängen im selben Lock wie `bump`: Ein Widerruf dazwischen ist ausgeschlossen.
            var appended = false
            if !revoked, !record.revoked {
                appended = release.ifCurrent(record.generation) {
                    history.append(turn)
                    if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }
                }
            }
            historyMark = AssistantCommitMark(stage: .history, ok: appended, generation: release.currentGeneration)
        }

        guard let audit else { return true }
        if !record.journaled { audit.enqueue(Self.startLine(requestId, session: sessionId, config: config, record: record)) }
        if let mark = historyMark {
            var line = AssistantJournalLine(.commit, req: requestId)
            line.n = 0
            line.mark = mark
            audit.enqueue(line)
        }
        let outcome: AssistantAuditEntry.Outcome
        if revoked || record.revoked || record.failure == .revoked || historyMark?.ok == false {
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
        return await audit.flush(request: requestId) == nil
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
                var record = AssistantRunRecord()
                record.generation = snapshot.generation
                record.optInAtStart = snapshot.optIn
                record.consentVersion = snapshot.consentVersion
                audit.enqueue(Self.startLine(rid, session: sessionId, config: config, record: record))
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
        let audit = self.audit, index = self.stepIndex
        let promptHistory = Self.promptHistory(history, areas: allowedAreas)
        let state = RunState(snapshot: snapshot)
        let output = AssistantRunOutput(release: release, generation: snapshot.generation, continuation: out)
        let work = Task<Outcome, Never> {
            await Self.generate(request, model: model, knowledge: knowledge, config: runConfig, snapshot: snapshot, release: release,
                                context: context, history: promptHistory, state: state, output: output, audit: audit,
                                index: index, session: self.sessionId)
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
            if !output.send(.done(requestId: rid, done)) {
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
        state: RunState, output: AssistantRunOutput, audit: AssistantAuditLog?, index: AssistantStepIndex, session: String
    ) async -> Outcome {
        let rid = request.requestId
        let budget = AssistantPrompt.budget(config: config)
        let emitter = RunEmitter(rid: rid, generation: snapshot.generation, release: release, output: output, state: state,
                                 journal: audit, index: index, started: ContinuousClock().now)
        func finished(_ failure: AssistantFailure? = nil) -> Outcome {
            .finished(text: emitter.texts.joined(separator: " "), firstMs: emitter.firstMs, totalMs: emitter.elapsedMs, failure: failure)
        }

        let passages = await knowledge.retrieve(question: request.text, previous: history.last?.user, limit: passageLimit, maxChars: budget.passages)
        if Task.isCancelled { return .cancelled }
        let titles = await knowledge.inventoryTitles()
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
        // `start` steht in der Datei, bevor das Modell läuft und bevor etwas ausgegeben wird. Ein Fehler wird für den Abschluss
        // der Anfrage gemerkt (`commit` meldet ihn), die Antwort läuft trotzdem.
        if let audit {
            _ = await audit.begin(startLine(rid, session: session, config: config, record: state.snapshot))
            state.update { $0.journaled = true }
            if Task.isCancelled { return .cancelled }
        }

        let now = Date()
        let clockSentences = AssistantGrounding.clockSentences(now: now, timeZone: .current)

        if inventoryQuestion {
            guard output.send(.sources(requestId: rid, generation: snapshot.generation, [])) else { return .cancelled }
            let groups = await knowledge.inventoryGroups(question: request.text)
            if Task.isCancelled { return .cancelled }
            let statements = groups.flatMap(\.statements)
            if statements.isEmpty {
                if emitter.emitFixed(optIn: snapshot.optIn) == .revoked { return .cancelled }
                return finished()
            }
            state.update { $0.inventoryAreas = groups.map(\.area) }
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
                guard output.send(.sources(requestId: rid, generation: snapshot.generation, used)) else { return .cancelled }
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
                        f = emitter.handleData(s.text, basis: basis, titles: titles, clock: clockSentences)
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
            if failure == nil, flow == .next {
                // Stromende: der Rest ist der letzte Satz, aber nur mit Satzzeichen; ein abgeschnittener Rest wird verworfen.
                flow = await check(splitter.finish())
                if flow == .revoked { return .cancelled }
                for _ in 0..<splitter.overlong { emitter.drop(.template) }
                for _ in 0..<splitter.fragments { emitter.drop(.fragment) }
                if seen == 0, splitter.overlong == 0, splitter.fragments == 0 { failure = .modelFailed }
            }
            if failure == nil, flow != .revoked, flow != .limit, emitter.dropped > 0 {
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
