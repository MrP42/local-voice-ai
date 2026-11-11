import Foundation

// Gesprächsmodus „Sprechen“ (Paket K, erweitert in Paket A um „Antworten ohne Quelle“, E4.2a): Ablauf einer Antwort.
// Abruf → Ereignis `sources` → Modellstrom → Satztrenner → Satzprüfung (`AssistantGrounding`) → `sentence`-Ereignisse →
// `done`. Muster: `BookTalkSession` (Strom, Abbruch, Wächter). Mehrere Anfragen dürfen gleichzeitig laufen (Vorarbeit und
// Abbruch); keine schreibt in den Verlauf, das macht allein `commit`.
//
// Satzprüfung vor der Ausgabe: Jeder Satz ist belegt (Auszug oder Bestand), unbelegt (Allgemeinwissen), neutral (Rückfrage,
// Gruß, feste Vorlage) oder verworfen. Verworfen wird immer, was sich auf App-Inhalte beruft und nicht gedeckt ist, und ohne
// Opt-in jeder unbelegte Satz. Beim Wechsel zwischen belegten und unbelegten Abschnitten kommt eine hörbare Ansage.

/// Ein ausgegebener Satz (ohne Ansagen) mit Wertung und deckenden Quellen, für `commit`.
struct AssistantEmitted: Sendable, Equatable {
    let text: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
}

/// Was eine Anfrage hervorgebracht hat; `commit` macht daraus den Beitrag.
struct AssistantRunRecord: Sendable {
    var used: [AssistantPassage] = []
    var sentences: [AssistantEmitted] = []
    var counts = AssistantSentenceCounts()
    /// „Ohne Quelle“ war bei Beginn der Anfrage erlaubt.
    var allowedAtStart = false
}

/// Gemeinsamer Zustand einer Anfrage zwischen Arbeit, Wächter und Sitzung.
private final class RunState: @unchecked Sendable {
    private let lock = NSLock()
    private var lastProgress = Date()
    private var timedOut = false
    private var record: AssistantRunRecord

    init(allowedAtStart: Bool) {
        record = AssistantRunRecord()
        record.allowedAtStart = allowedAtStart
    }

    func touch() { lock.lock(); lastProgress = Date(); lock.unlock() }
    var idle: TimeInterval { lock.lock(); defer { lock.unlock() }; return Date().timeIntervalSince(lastProgress) }
    func fireTimeout() { lock.lock(); timedOut = true; lock.unlock() }
    var didTimeOut: Bool { lock.lock(); defer { lock.unlock() }; return timedOut }
    func setUsed(_ p: [AssistantPassage]) { lock.lock(); record.used = p; lock.unlock() }
    var used: [AssistantPassage] { lock.lock(); defer { lock.unlock() }; return record.used }
    func update(_ body: (inout AssistantRunRecord) -> Void) { lock.lock(); body(&record); lock.unlock() }
    var snapshot: AssistantRunRecord { lock.lock(); defer { lock.unlock() }; return record }
}

/// Opt-in „Antworten ohne Quelle“ zur Laufzeit: wird je Satz gelesen, ein Widerruf wirkt auf laufende Ströme sofort.
private final class UnsourcedSwitch: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Bool

    init(_ value: Bool) { self.value = value }
    var isOn: Bool { lock.lock(); defer { lock.unlock() }; return value }
    func set(_ on: Bool) { lock.lock(); value = on; lock.unlock() }
}

/// Satz zur Ausgabe (nach der Prüfung).
private struct Emission {
    let text: String
    let speech: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
}

/// Prüft die Sätze einer Antwort der Reihe nach und führt Buch über Abschnitt, Ansagen und verworfene Sätze.
private struct SentenceGate {
    let basis: AssistantGrounding.Basis
    let knowledge: AssistantKnowledge
    let unsourced: UnsourcedSwitch
    let startAreas: Set<AssistantArea>
    let startAllowed: Bool
    /// Zuletzt ausgegebener Abschnitt: `.grounded` oder `.unsourced`; nil vor dem ersten.
    var section: AssistantSentenceStatus?
    /// Mindestens ein belegter, unbelegter oder fester Aussagesatz kam heraus.
    var substantive = false
    var dropped = 0

    init(basis: AssistantGrounding.Basis, knowledge: AssistantKnowledge, unsourced: UnsourcedSwitch, startAreas: Set<AssistantArea>,
         startAllowed: Bool) {
        self.basis = basis
        self.knowledge = knowledge
        self.unsourced = unsourced
        self.startAreas = startAreas
        self.startAllowed = startAllowed
    }

    private mutating func drop(revoked: Bool, state: RunState) {
        dropped += 1
        state.update { $0.counts.dropped += 1; if revoked { $0.counts.revoked += 1 } }
    }

    private func announcement(_ text: String) -> Emission {
        Emission(text: text, speech: text, status: .announcement, sources: [])
    }

    private func keep(_ e: Emission, state: RunState) -> Emission {
        state.update { record in
            record.sentences.append(AssistantEmitted(text: e.text, status: e.status, sources: e.sources))
            switch e.status {
            case .grounded: record.counts.grounded += 1
            case .unsourced: record.counts.unsourced += 1
            case .neutral: record.counts.neutral += 1
            case .announcement: break
            }
        }
        return e
    }

    mutating func process(text: String, speech: String, state: RunState) async -> [Emission] {
        switch AssistantGrounding.classify(sentence: text, basis: basis) {
        case .appClaimUnsupported:
            drop(revoked: false, state: state)
            return []
        case .neutral:
            if AssistantGrounding.isTemplate(text) { substantive = true }
            return [keep(Emission(text: text, speech: speech, status: .neutral, sources: []), state: state)]
        case .unsourced:
            guard unsourced.isOn else {
                drop(revoked: startAllowed, state: state)
                return []
            }
            var out: [Emission] = []
            if section != .unsourced { out.append(announcement(AssistantGrounding.unsourcedAnnouncement)) }
            section = .unsourced
            substantive = true
            out.append(keep(Emission(text: text, speech: speech, status: .unsourced, sources: []), state: state))
            return out
        case .grounded(let covering):
            // Bereiche können während der Antwort entzogen werden: Quellen aus ihnen decken nichts mehr.
            let areas = await knowledge.areas
            let removed = !startAreas.subtracting(areas).isEmpty
            let sources = covering.filter { areas.contains($0.area) }
            if (covering.isEmpty ? removed : sources.isEmpty) {
                drop(revoked: true, state: state)
                return []
            }
            var out: [Emission] = []
            if section == .unsourced { out.append(announcement(AssistantGrounding.groundedAnnouncement)) }
            section = .grounded
            substantive = true
            out.append(keep(Emission(text: text, speech: speech, status: .grounded, sources: sources), state: state))
            return out
        }
    }

    /// Bleibt von der Antwort nichts Gesprochenes übrig, kommt genau ein fester Satz.
    mutating func finish(state: RunState) -> [Emission] {
        guard !substantive, dropped > 0 else { return [] }
        substantive = true
        let sentence = AssistantGrounding.fixedSentence(allowUnsourced: unsourced.isOn)
        return [keep(Emission(text: sentence, speech: sentence, status: .neutral, sources: []), state: state)]
    }
}

public actor AssistantSession {
    public static let maxStoredTurns = 200
    /// Höchstens so viele Auszüge je Frage.
    static let passageLimit = 8

    private enum Outcome: Sendable {
        case finished(text: String, firstMs: Int?, totalMs: Int, failure: AssistantFailure?)
        case cancelled
    }

    private let model: any BookTalkModel
    private let knowledge: AssistantKnowledge
    private let config: AssistantConfig
    private let unsourcedSwitch: UnsourcedSwitch
    private let audit: AssistantAuditLog?
    private var active: [String: @Sendable () -> Void] = [:]
    private var earlyCancel: Set<String> = []
    private var results: [String: AssistantRunRecord] = [:]
    private var resultOrder: [String] = []
    public private(set) var history: [AssistantTurn]

    public init(model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig, history: [AssistantTurn] = [],
                audit: AssistantAuditLog? = nil) {
        self.model = model
        self.knowledge = knowledge
        self.config = config
        self.unsourcedSwitch = UnsourcedSwitch(config.allowUnsourced)
        self.audit = audit
        self.history = history.count > Self.maxStoredTurns ? Array(history.suffix(Self.maxStoredTurns)) : history
    }

    // MARK: Antworten ohne Quelle

    /// Ob unbelegte Sätze (Allgemeinwissen) gesprochen werden dürfen. Ohne Aktor-Sprung lesbar (Schalter hinter einem Lock).
    public nonisolated var allowsUnsourced: Bool { unsourcedSwitch.isOn }

    /// Schaltet „Antworten ohne Quelle“ um. Aus wirkt sofort und ohne Aktor-Sprung auf laufende Antworten (die Prüfung liest
    /// den Schalter je Satz): weitere unbelegte Sätze werden verworfen, auch wenn der Aufrufer gleich danach weiterläuft.
    public nonisolated func setAllowUnsourced(_ on: Bool) { unsourcedSwitch.set(on) }

    // MARK: Verlauf

    public func restore(history turns: [AssistantTurn]) {
        history = turns.count > Self.maxStoredTurns ? Array(turns.suffix(Self.maxStoredTurns)) : turns
    }

    public func clearHistory() { history.removeAll() }

    /// Cloud-Schutz: Die Beiträge, die als VERLAUF ins Modell dürfen. Lokal alle; bei einem Cloud-Modell nur Beiträge ohne
    /// Quellen oder mit Quellen ausschließlich aus den für die Cloud freigegebenen Bereichen (`knowledge.areas`, dieselben
    /// wie beim Abruf): Gesprochenes aus einem gesperrten Bereich verlässt das Gerät nicht. `history` selbst (Anzeige,
    /// Datei) bleibt vollständig; die Auswahl gilt je Anfrage und folgt damit jeder Änderung der Bereiche.
    static func promptHistory(_ turns: [AssistantTurn], areas: Set<AssistantArea>?) -> [AssistantTurn] {
        guard let areas else { return turns }
        return turns.filter { turn in turn.sources.allSatisfy { areas.contains($0.area) } }
    }

    /// Zuordnung des tatsächlich Gesprochenen zu den ausgegebenen Sätzen (der Reihe nach, Ansagen zählen nicht). Ein Satz, der
    /// nur angesprochen wurde (Unterbrechung), zählt mit seiner Wertung; Gesprochenes ohne Entsprechung gilt als unbelegt.
    static func attribute(spoken: String, record: AssistantRunRecord?) -> [(text: String, status: AssistantSentenceStatus, sources: [AssistantPassage])] {
        var rest = Substring(spoken)
        var out: [(text: String, status: AssistantSentenceStatus, sources: [AssistantPassage])] = []
        for e in record?.sentences ?? [] where e.status != .announcement {
            rest = rest.drop(while: { $0.isWhitespace })
            if rest.isEmpty { break }
            if rest.hasPrefix(e.text) {
                out.append((e.text, e.status, e.sources))
                rest = rest.dropFirst(e.text.count)
            } else if e.text.hasPrefix(rest) {
                out.append((String(rest), e.status, e.sources))
                rest = ""
            }
        }
        rest = rest.drop(while: { $0.isWhitespace })
        if !rest.isEmpty { out.append((String(rest), .unsourced, [])) }
        return out
    }

    private static func distinct(_ passages: [AssistantPassage]) -> [AssistantPassage] {
        var seen = Set<String>()
        return passages.filter { seen.insert("\($0.area.rawValue)/\($0.itemId)/\($0.locator ?? "")").inserted }
    }

    /// Nach `done` oder nach Barge-in (dann `spoken` = tatsächlich Gesprochenes, `interrupted` = true) trägt die Steuerung
    /// den Beitrag ein. Quellen (nur die, die einen gesprochenen Satz decken) und Kennzeichnung „ohne Quelle“ kommen aus der
    /// Prüfung dieser Anfrage. Ohne Nutzertext oder ohne tatsächlich Gesprochenes entsteht kein Beitrag (eine abgebrochene
    /// Frage ohne Antwort bleibt nicht stehen). Schreibt außerdem den Protokolleintrag (ohne Inhalte); `false`, wenn nur
    /// dieser nicht geschrieben werden konnte (der Beitrag steht trotzdem im Verlauf).
    @discardableResult
    public func commit(requestId: String, user: String, spoken: String, interrupted: Bool, revoked: Bool = false) async -> Bool {
        let record = results.removeValue(forKey: requestId)
        resultOrder.removeAll { $0 == requestId }
        let text = user.trimmingCharacters(in: .whitespacesAndNewlines)
        let said = spoken.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !said.isEmpty else { return true }

        let parts = Self.attribute(spoken: said, record: record)
        let sources = Self.distinct(parts.filter { $0.status == .grounded }.flatMap(\.sources))
        let anyUnsourced = parts.contains { $0.status == .unsourced }
        let mixed = anyUnsourced && parts.contains { $0.status != .unsourced }
        var turnParts: [AssistantTurnPart] = []
        if mixed {
            for p in parts {
                let flag = p.status == .unsourced
                if let last = turnParts.last, last.unsourced == flag {
                    turnParts[turnParts.count - 1].text += " " + p.text
                } else {
                    turnParts.append(AssistantTurnPart(text: p.text, unsourced: flag))
                }
            }
        }
        history.append(AssistantTurn(
            user: text, assistant: said, interrupted: interrupted, sources: sources, general: sources.isEmpty,
            unsourced: anyUnsourced, parts: turnParts, model: config.modelLabel, at: Date()))
        if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }

        guard let audit else { return true }
        let wasRevoked = revoked || (record?.counts.revoked ?? 0) > 0
        let entry = AssistantAuditEntry(
            kind: .answer, model: config.modelLabel, cloud: config.isCloud, unsourcedMode: record?.allowedAtStart ?? unsourcedSwitch.isOn,
            consentVersion: config.consentVersion,
            grounded: parts.filter { $0.status == .grounded }.count, unsourced: parts.filter { $0.status == .unsourced }.count,
            neutral: parts.filter { $0.status == .neutral }.count, dropped: record?.counts.dropped ?? 0,
            areas: sources.map(\.area), outcome: wasRevoked ? .revoked : (interrupted ? .interrupted : .completed))
        return await audit.record(entry)
    }

    // MARK: Abbruch

    /// Bricht eine laufende Anfrage ab: nur noch `.failed(.cancelled)`, nichts wird gespeichert. Ein Abbruch, der vor dem
    /// Start der Anfrage eintrifft, wirkt beim Start.
    public func cancel(requestId: String) {
        if let stop = active[requestId] { return stop() }
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
        // Vor der Abbruchprüfung, damit zwischen Prüfung und Eintrag in `active` kein Wartepunkt liegt.
        var allowedAreas: Set<AssistantArea>?
        if self.config.isCloud { allowedAreas = await self.knowledge.areas }
        if earlyCancel.remove(rid) != nil {
            out.yield(.failed(requestId: rid, .cancelled))
            return
        }
        var runConfig = self.config
        runConfig.allowUnsourced = self.unsourcedSwitch.isOn
        let model = self.model, knowledge = self.knowledge, unsourcedSwitch = self.unsourcedSwitch
        let snapshot = Self.promptHistory(history, areas: allowedAreas)
        let state = RunState(allowedAtStart: runConfig.allowUnsourced)
        let work = Task<Outcome, Never> {
            await Self.generate(request, model: model, knowledge: knowledge, config: runConfig, unsourced: unsourcedSwitch,
                                history: snapshot, state: state, out: out)
        }
        active[rid] = { work.cancel() }
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
        active[rid] = nil
        remember(rid, state.snapshot)

        switch outcome {
        case .cancelled:
            out.yield(.failed(requestId: rid, state.didTimeOut ? .timeout : .cancelled))
        case .finished(let text, let firstMs, let totalMs, let failure):
            if Task.isCancelled {
                out.yield(.failed(requestId: rid, .cancelled))
            } else if let failure {
                out.yield(.failed(requestId: rid, failure))
            } else {
                let record = state.snapshot
                let covering = Self.distinct(record.sentences.filter { $0.status == .grounded }.flatMap(\.sources))
                out.yield(.done(requestId: rid, AssistantAnswer(
                    text: text, sources: covering, general: covering.isEmpty, firstSentenceMs: firstMs, totalMs: totalMs,
                    unsourced: record.counts.unsourced > 0, counts: record.counts)))
            }
        }
    }

    private func remember(_ id: String, _ record: AssistantRunRecord) {
        results[id] = record
        resultOrder.removeAll { $0 == id }
        resultOrder.append(id)
        while resultOrder.count > 16 { results[resultOrder.removeFirst()] = nil }
    }

    private static func milliseconds(_ d: Duration) -> Int {
        Int(d.components.seconds) * 1000 + Int(d.components.attoseconds / 1_000_000_000_000_000)
    }

    private static func generate(
        _ request: AssistantRequest, model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig,
        unsourced: UnsourcedSwitch, history: [AssistantTurn], state: RunState, out: AsyncStream<AssistantEvent>.Continuation
    ) async -> Outcome {
        let clock = ContinuousClock()
        let started = clock.now
        let rid = request.requestId
        let budget = AssistantPrompt.budget(config: config)

        // Zuerst die Bereiche zu Beginn: Was danach entzogen wird, darf keinen Satz mehr decken.
        let startAreas = await knowledge.areas
        let passages = await knowledge.retrieve(question: request.text, previous: history.last?.user, limit: passageLimit, maxChars: budget.passages)
        if Task.isCancelled { return .cancelled }
        let inventory = await knowledge.inventory(maxChars: budget.inventory)
        if Task.isCancelled { return .cancelled }
        let titles = await knowledge.inventoryTitles()
        if Task.isCancelled { return .cancelled }
        state.touch()

        let now = Date()
        let instructions = AssistantPrompt.instructions(config: config, now: now, timeZone: .current)
        let clockText = AssistantPrompt.germanDate(now, timeZone: .current)
        func make(scale: Double, includeHistory: Bool) -> (prompt: String, used: [AssistantPassage]) {
            AssistantPrompt.build(question: request.text, afterInterruption: request.afterInterruption, inventory: inventory,
                                  passages: passages, history: history, config: config, passageScale: scale, includeHistory: includeHistory)
        }
        var built = make(scale: 1, includeHistory: true)
        state.setUsed(built.used)
        out.yield(.sources(requestId: rid, built.used))

        var retried = false
        while true {
            var splitter = AssistantSpeech.Splitter()
            var gate = SentenceGate(
                basis: AssistantGrounding.Basis(passages: built.used, inventoryTitles: titles, inventory: inventory, clock: clockText),
                knowledge: knowledge, unsourced: unsourced, startAreas: startAreas, startAllowed: config.allowUnsourced)
            var seen = 0
            var texts: [String] = []
            var firstMs: Int?
            var failure: AssistantFailure?

            func emit(_ items: [Emission]) {
                for e in items {
                    if firstMs == nil { firstMs = milliseconds(started.duration(to: clock.now)) }
                    if e.status != .announcement { texts.append(e.text) }
                    out.yield(.sentence(requestId: rid, text: e.text, speech: e.speech, status: e.status, sources: e.sources))
                }
            }

            do {
                for try await piece in model.stream(instructions: instructions, prompt: built.prompt, maxTokens: config.maxAnswerTokens) {
                    if Task.isCancelled { return .cancelled }
                    state.touch()
                    for s in splitter.feed(piece) {
                        seen += 1
                        let items = await gate.process(text: s.text, speech: s.speech, state: state)
                        if Task.isCancelled { return .cancelled }
                        emit(items)
                    }
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
                        built = make(scale: 0.5, includeHistory: false)
                        if built.used != state.used { out.yield(.sources(requestId: rid, built.used)) }
                        state.setUsed(built.used)
                        continue
                    }
                    failure = .modelFailed
                }
            } catch {
                failure = .modelFailed
            }
            if Task.isCancelled { return .cancelled }
            if failure == nil {
                for s in splitter.finish() {
                    seen += 1
                    let items = await gate.process(text: s.text, speech: s.speech, state: state)
                    if Task.isCancelled { return .cancelled }
                    emit(items)
                }
                if seen == 0 { failure = .modelFailed } else { emit(gate.finish(state: state)) }
            }
            return .finished(text: texts.joined(separator: " "), firstMs: firstMs,
                             totalMs: milliseconds(started.duration(to: clock.now)), failure: failure)
        }
    }
}
