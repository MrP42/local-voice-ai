import Foundation

/// One worker under an explicit execution grant, durable failure budget and stage checkpoints.
@MainActor
public final class JobProcessor {
    public var fixedAnswer = false
    public private(set) var active = false
    public private(set) var currentId: UUID?
    public private(set) var message = "Bereit"
    public var isProcessing: Bool { task != nil }
    public var onChange: ((UUID?) -> Void)?
    private let store: DurableStore
    private let transcribe: @Sendable (URL) async throws -> ProcessingOutput
    private let reply: @Sendable (String, [ConversationTurn]) async throws -> ProcessingOutput
    private var task: Task<Void, Never>?
    private var resumeRequested = false
    private var userCancelled = false

    public init(store: DurableStore, transcribe: @escaping @Sendable (URL) async throws -> String,
                reply: @escaping @Sendable (String) async throws -> String) {
        self.store = store
        self.transcribe = { ProcessingOutput(text: try await transcribe($0), model: "Nicht dokumentiert") }
        self.reply = { text, _ in ProcessingOutput(text: try await reply(text), model: "Nicht dokumentiert") }
    }
    public init(store: DurableStore, annotatedTranscribe: @escaping @Sendable (URL) async throws -> ProcessingOutput,
                annotatedReply: @escaping @Sendable (String) async throws -> ProcessingOutput) {
        self.store = store; self.transcribe = annotatedTranscribe; self.reply = { text, _ in try await annotatedReply(text) }
    }
    public init(store: DurableStore, annotatedTranscribe: @escaping @Sendable (URL) async throws -> ProcessingOutput,
                conversationalReply: @escaping @Sendable (String, [ConversationTurn]) async throws -> ProcessingOutput) {
        self.store = store; self.transcribe = annotatedTranscribe; self.reply = conversationalReply
    }
    public func setActive(_ active: Bool) {
        self.active = active
        if active { start() } else { task?.cancel() }
    }
    public func cancelCurrent() {
        userCancelled = true
        message = "Verarbeitung wird abgebrochen – Aufnahme bleibt gespeichert"
        onChange?(currentId)
        task?.cancel()
    }
    public func start() {
        guard active else { return }
        guard task == nil else { resumeRequested = true; return }
        userCancelled = false
        task = Task { [weak self] in
            guard let self else { return }
            await self.drain()
            self.task = nil; self.currentId = nil; self.onChange?(nil)
            if self.resumeRequested && self.active {
                self.resumeRequested = false; self.start()
            }
        }
    }
    private func drain() async {
        do {
            while active {
                try Task.checkCancellation()
                let entries = try store.entries().sorted { $0.createdAt < $1.createdAt }
                let now = Date()
                guard let entry = entries.first(where: { $0.reply == nil && ($0.job ?? JobRecord()).canStart(now: now) }) else {
                    let next = entries.filter { $0.reply == nil && $0.job?.phase == .waiting && ($0.job?.attempts ?? 0) < 3 }
                        .compactMap { $0.job?.nextAttemptAt }.min()
                    guard let next else { return }
                    try await Task.sleep(for: .seconds(max(0.1, next.timeIntervalSinceNow)))
                    continue
                }
                guard try store.beginJob(entry.id) else { continue }
                currentId = entry.id; message = "Aufnahme wird lokal verarbeitet"; onChange?(entry.id)
                do {
                    if fixedAnswer {
                        try store.update(entry.id, reply: "Deine Aufnahme ist sicher gespeichert.", state: .answered, event: ProcessingEvent(operation: "Antwort", model: "Feste Testantwort", completedAt: Date(), durationMS: 0, isAI: false))
                    } else {
                        let text: String
                        if let previous = entry.transcript { text = previous }
                        else {
                            let start = Date()
                            let result = try await transcribe(store.audioURL(for: entry.id))
                            text = result.text
                            try Task.checkCancellation()
                            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw ProcessingFailure.noSpeech }
                            try store.update(entry.id, transcript: text, state: .deferred, timing: ("stt_ms", Date().timeIntervalSince(start) * 1000), event: ProcessingEvent(operation: "Transkription", model: result.model, completedAt: Date(), durationMS: Date().timeIntervalSince(start) * 1000, isAI: result.isAI))
                        }
                        try Task.checkCancellation()
                        try store.setJobPhase(entry.id, phase: .generating)
                        message = "Kurze Antwort wird lokal erzeugt"; onChange?(entry.id)
                        let start = Date()
                        let result = try await reply(text, ConversationContext.previousTurns(for: entry, in: store.entries()))
                        let answer = result.text
                        try Task.checkCancellation()
                        guard !answer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, answer.count <= 500 else { throw ProcessingFailure.invalidOutput }
                        try store.update(entry.id, reply: answer, state: .answered, timing: ("generation_ms", Date().timeIntervalSince(start) * 1000), event: ProcessingEvent(operation: "Antwort", model: result.model, completedAt: Date(), durationMS: Date().timeIntervalSince(start) * 1000, isAI: result.isAI))
                    }
                    message = "Antwort gespeichert"
                } catch {
                    if Task.isCancelled || error is CancellationError {
                        try store.pauseJob(entry.id, userInitiated: userCancelled)
                        message = userCancelled ? "Verarbeitung abgebrochen – Aufnahme bleibt gespeichert" : "gespeichert – Verarbeitung folgt"
                        onChange?(entry.id); return
                    }
                    let reason: JobFailure = (error as? ProcessingFailure) == .noSpeech ? .noSpeech : (error as? ProcessingFailure) == .invalidOutput ? .invalidOutput : .unavailable
                    try store.failJob(entry.id, reason: reason)
                    if reason == .noSpeech, entry.conversationId != nil {
                        // A durable, non-AI outcome uses the existing acknowledged reply delivery,
                        // including Watch retries. It is excluded from context because it has no transcript.
                        try store.update(entry.id, reply: ConversationOutcome.noSpeechMessage, state: .answered,
                                         event: ProcessingEvent(operation: "Keine Sprache", model: "Gesprächssteuerung", completedAt: Date(), durationMS: 0, isAI: false))
                    }
                    message = reason == .noSpeech ? "Keine Sprache erkannt – Aufnahme bleibt gespeichert" : "gespeichert – Verarbeitung folgt"
                }
                onChange?(entry.id); currentId = nil
            }
        } catch {
            if let id = currentId { try? store.pauseJob(id, userInitiated: userCancelled) }
            message = Task.isCancelled ? "gespeichert – Verarbeitung folgt" : "Speicherprüfung erforderlich – Originale bleiben erhalten"
            onChange?(currentId)
        }
    }
}
public enum ProcessingFailure: Error { case noSpeech, invalidOutput, timedOut }
