import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): Ablauf einer Antwort.
// Abruf → Ereignis `sources` → Modellstrom → Satztrenner → `sentence`-Ereignisse → `done`. Muster: `BookTalkSession`
// (Strom, Abbruch, Wächter). Mehrere Anfragen dürfen gleichzeitig laufen (Vorarbeit und Abbruch); keine schreibt in den
// Verlauf, das macht allein `commit`.

/// Gemeinsamer Zustand einer Anfrage zwischen Arbeit, Wächter und Sitzung.
private final class RunState: @unchecked Sendable {
    private let lock = NSLock()
    private var lastProgress = Date()
    private var timedOut = false
    private var passages: [AssistantPassage] = []

    func touch() { lock.lock(); lastProgress = Date(); lock.unlock() }
    var idle: TimeInterval { lock.lock(); defer { lock.unlock() }; return Date().timeIntervalSince(lastProgress) }
    func fireTimeout() { lock.lock(); timedOut = true; lock.unlock() }
    var didTimeOut: Bool { lock.lock(); defer { lock.unlock() }; return timedOut }
    func setUsed(_ p: [AssistantPassage]) { lock.lock(); passages = p; lock.unlock() }
    var used: [AssistantPassage] { lock.lock(); defer { lock.unlock() }; return passages }
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
    private var active: [String: @Sendable () -> Void] = [:]
    private var earlyCancel: Set<String> = []
    private var results: [String: [AssistantPassage]] = [:]
    private var resultOrder: [String] = []
    public private(set) var history: [AssistantTurn]

    public init(model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig, history: [AssistantTurn] = []) {
        self.model = model
        self.knowledge = knowledge
        self.config = config
        self.history = history.count > Self.maxStoredTurns ? Array(history.suffix(Self.maxStoredTurns)) : history
    }

    // MARK: Verlauf

    public func restore(history turns: [AssistantTurn]) {
        history = turns.count > Self.maxStoredTurns ? Array(turns.suffix(Self.maxStoredTurns)) : turns
    }

    public func clearHistory() { history.removeAll() }

    /// Nach `done` oder nach Barge-in (dann `spoken` = tatsächlich Gesprochenes, `interrupted` = true) trägt die Steuerung
    /// den Beitrag ein. Quellen und „Allgemeinwissen“ kommen aus dem Abruf dieser Anfrage.
    public func commit(requestId: String, user: String, spoken: String, interrupted: Bool) {
        let text = user.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        let sources = results.removeValue(forKey: requestId) ?? []
        resultOrder.removeAll { $0 == requestId }
        history.append(AssistantTurn(
            user: text, assistant: spoken.trimmingCharacters(in: .whitespacesAndNewlines), interrupted: interrupted,
            sources: sources, general: sources.isEmpty, model: config.modelLabel, at: Date()))
        if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }
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
        if earlyCancel.remove(rid) != nil {
            out.yield(.failed(requestId: rid, .cancelled))
            return
        }
        let model = self.model, knowledge = self.knowledge, config = self.config, snapshot = history
        let state = RunState()
        let work = Task<Outcome, Never> {
            await Self.generate(request, model: model, knowledge: knowledge, config: config, history: snapshot, state: state, out: out)
        }
        active[rid] = { work.cancel() }
        let timeout = config.timeout
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
        remember(rid, state.used)

        switch outcome {
        case .cancelled:
            out.yield(.failed(requestId: rid, state.didTimeOut ? .timeout : .cancelled))
        case .finished(let text, let firstMs, let totalMs, let failure):
            if Task.isCancelled {
                out.yield(.failed(requestId: rid, .cancelled))
            } else if let failure {
                out.yield(.failed(requestId: rid, failure))
            } else {
                let used = state.used
                out.yield(.done(requestId: rid, AssistantAnswer(text: text, sources: used, general: used.isEmpty,
                                                                firstSentenceMs: firstMs, totalMs: totalMs)))
            }
        }
    }

    private func remember(_ id: String, _ passages: [AssistantPassage]) {
        results[id] = passages
        resultOrder.removeAll { $0 == id }
        resultOrder.append(id)
        while resultOrder.count > 16 { results[resultOrder.removeFirst()] = nil }
    }

    private static func milliseconds(_ d: Duration) -> Int {
        Int(d.components.seconds) * 1000 + Int(d.components.attoseconds / 1_000_000_000_000_000)
    }

    private static func generate(
        _ request: AssistantRequest, model: any BookTalkModel, knowledge: AssistantKnowledge, config: AssistantConfig,
        history: [AssistantTurn], state: RunState, out: AsyncStream<AssistantEvent>.Continuation
    ) async -> Outcome {
        let clock = ContinuousClock()
        let started = clock.now
        let rid = request.requestId
        let budget = AssistantPrompt.budget(config: config)

        let passages = await knowledge.retrieve(question: request.text, previous: history.last?.user, limit: passageLimit, maxChars: budget.passages)
        if Task.isCancelled { return .cancelled }
        let inventory = await knowledge.inventory(maxChars: budget.inventory)
        if Task.isCancelled { return .cancelled }
        state.touch()

        let instructions = AssistantPrompt.instructions(config: config, now: Date(), timeZone: .current)
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
            var sentences: [String] = []
            var firstMs: Int?
            var failure: AssistantFailure?

            do {
                for try await piece in model.stream(instructions: instructions, prompt: built.prompt, maxTokens: config.maxAnswerTokens) {
                    if Task.isCancelled { return .cancelled }
                    state.touch()
                    for s in splitter.feed(piece) {
                        if firstMs == nil { firstMs = milliseconds(started.duration(to: clock.now)) }
                        sentences.append(s.text)
                        out.yield(.sentence(requestId: rid, text: s.text, speech: s.speech))
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
                    if !retried, sentences.isEmpty {
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
                    if firstMs == nil { firstMs = milliseconds(started.duration(to: clock.now)) }
                    sentences.append(s.text)
                    out.yield(.sentence(requestId: rid, text: s.text, speech: s.speech))
                }
                if sentences.isEmpty { failure = .modelFailed }
            }
            return .finished(text: sentences.joined(separator: " "), firstMs: firstMs,
                             totalMs: milliseconds(started.duration(to: clock.now)), failure: failure)
        }
    }
}
