import Foundation

// Buch-Gespräch (Epic #307, Paket G4): Ablauf einer Frage (Plan §3, Auflagen aus den Codex-Runden).
// Absicht → Suche (mit Spoilergrenze) → Auszüge → Budgetprüfung → Modell → satzweise Prüfung → Antwort.

public struct BookTalkFailure: Error, Equatable {
    public let code: BookTalkCode
}

private final class TimeoutFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var set = false
    func fire() { lock.lock(); set = true; lock.unlock() }
    var fired: Bool { lock.lock(); defer { lock.unlock() }; return set }
}

public actor BookTalkSession {
    public static let maxStoredTurns = 200

    private let index: BookTalkIndex
    private let model: (any BookTalkModel)?
    private let budget: BookTalkBudget
    private let entities: [BookTalkEntity]
    private let counter: (any BookTalkTokenCounter)?
    private let profile: BookTalkByteProfile?
    private let timeout: TimeInterval
    private let bookStems: Set<String>
    private var active: [String: @Sendable () -> Void] = [:]
    public private(set) var history: [BookTalkStoredTurn]

    /// - Parameters:
    ///   - tokenCounter: exakter Zähler des Modells; ohne ihn und ohne `byteProfile` ist nichts prüfbar
    ///     und jede Frage endet als found_only mit Grund `budget`.
    ///   - timeout: Zeitrahmen für die Modellantwort (iOS 45 s, Desktop 30 s).
    ///   - history: wiederhergestellter Verlauf (z. B. aus `Application Support/BookTalk`).
    public init(
        index: BookTalkIndex, model: (any BookTalkModel)?, budget: BookTalkBudget = .foundationModels,
        entities: [BookTalkEntity] = [], tokenCounter: (any BookTalkTokenCounter)? = nil,
        byteProfile: BookTalkByteProfile? = nil, timeout: TimeInterval = 45, history: [BookTalkStoredTurn] = []
    ) {
        self.index = index
        self.model = model
        self.budget = budget
        self.entities = entities
        self.counter = tokenCounter
        self.profile = byteProfile
        self.timeout = timeout
        self.history = history
        self.bookStems = BookTalkCitations.bookStems(index: index)
    }

    public func clearHistory() { history.removeAll() }

    /// Setzt den Verlauf auf einen Stand der Oberflaeche zurueck (hoechstens `maxStoredTurns`). Die App ruft das nach einem
    /// Abbruch, damit ein Wechsel, den die Anzeige verworfen hat, nicht doch im Prompt oder auf der Platte landet.
    public func restore(history turns: [BookTalkStoredTurn]) {
        history = turns.count > Self.maxStoredTurns ? Array(turns.suffix(Self.maxStoredTurns)) : turns
    }

    /// Bricht eine laufende Frage ab: nur noch `.failed(.cancelled)`, nichts wird gespeichert.
    public func cancel(requestId: String) { active[requestId]?() }

    /// Strom der Ereignisse; endet mit `.done` oder `.failed`.
    public nonisolated func events(_ request: BookTalkRequest) -> AsyncStream<BookTalkEvent> {
        AsyncStream { continuation in
            let task = Task { await self.run(request, out: continuation) }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    public nonisolated func events(_ question: String, frontier: Int?, scope: BookTalkScope) -> AsyncStream<BookTalkEvent> {
        events(BookTalkRequest(bookId: "", requestId: UUID().uuidString, question: question, frontier: frontier, scope: scope, input: .text))
    }

    /// Bequem: wartet auf die Gesamtantwort.
    public func ask(_ request: BookTalkRequest) async throws -> BookTalkAnswer {
        for await e in events(request) {
            switch e {
            case .done(_, let a): return a
            case .failed(_, let c): throw BookTalkFailure(code: c)
            case .sentence: continue
            }
        }
        throw BookTalkFailure(code: .cancelled)
    }

    public func ask(_ question: String, frontier: Int?, scope: BookTalkScope) async throws -> BookTalkAnswer {
        try await ask(BookTalkRequest(bookId: "", requestId: UUID().uuidString, question: question, frontier: frontier, scope: scope, input: .text))
    }

    // MARK: Ablauf

    private enum Outcome: Sendable {
        case finished(Collector, BookTalkCode?)
        case cancelled
    }

    private func run(_ request: BookTalkRequest, out: AsyncStream<BookTalkEvent>.Continuation) async {
        defer { out.finish() }
        // Schnappschuss: Bereich und Grenze gelten für diese Anfrage unverändert.
        let question = request.question
        let scope = request.scope
        let language = BookTalkIntents.language(of: question)
        let intent = BookTalkIntents.detect(question)
        let eligible = BookTalkPrompt.eligibleHistory(history.map(\.promptTurn), scope: scope, frontier: request.frontier ?? 0)
        let retrieval = BookTalkRetriever.retrieve(
            index: index, question: question, intent: intent, frontier: request.frontier, scope: scope,
            previousQuestion: eligible.last?.question, entities: entities, maxChars: budget.excerptChars)
        let readingPage: Int? = request.frontier.flatMap { index.page(at: max($0 - 1, 0)) } ?? retrieval.excerpts.last?.chunk.page
        let ctx = Context(request: request, intent: intent, language: language, readingPage: readingPage, index: index)

        if scope == .untilPosition, request.frontier == nil {
            return finish(Fallback.notice(ctx, reason: .noFrontier, language == .de
                ? "Ich kenne deine Leseposition noch nicht." : "I do not know your reading position yet."), ctx, out)
        }
        if retrieval.excerpts.isEmpty {
            return finish(Fallback.notice(ctx, reason: .noHits, language == .de
                ? (scope == .untilPosition ? "Dazu finde ich bis hier nichts." : "Dazu finde ich im Buch nichts.")
                : (scope == .untilPosition ? "I find nothing about that so far." : "I find nothing about that in the book.")), ctx, out)
        }
        guard let model else { return finish(Fallback.foundOnly(ctx, retrieval, reason: .noModel), ctx, out) }
        guard let fit = BookTalkPrompt.fit(
            title: index.title, question: question, retrieval: retrieval, history: history.map(\.promptTurn),
            language: language, scope: scope, frontier: request.frontier ?? 0, budget: budget, counter: counter, profile: profile)
        else { return finish(Fallback.foundOnly(ctx, retrieval, reason: .budget), ctx, out) }

        let grounding = BookTalkGroundingContext(
            index: index, excerpts: fit.retrieval.excerpts, entities: entities, intent: intent, question: question,
            lateStems: scope == .untilPosition ? BookTalkCitations.lateStems(index: index, frontier: request.frontier ?? 0) : [],
            bookStems: bookStems)
        let rid = request.requestId
        let parts = fit.parts
        let tokens = budget.answerTokens

        let work = Task<Outcome, Never> {
            var collector = Collector(grounding: grounding, language: language, requestId: rid)
            var failure: BookTalkCode?
            do {
                for try await piece in model.stream(instructions: parts.instructions, prompt: parts.prompt, maxTokens: tokens) {
                    if Task.isCancelled { return .cancelled }
                    for e in collector.feed(piece) { out.yield(e) }
                }
            } catch is CancellationError {
                return .cancelled
            } catch let e as BookTalkModelError {
                switch e {
                case .offline: failure = .offline
                case .unavailable: failure = .noModel
                case .contextOverflow: failure = .budget
                }
            } catch {
                failure = .llmFailed
            }
            return Task.isCancelled ? .cancelled : .finished(collector, failure)
        }
        active[rid] = { work.cancel() }
        let flag = TimeoutFlag()
        let seconds = timeout
        let watchdog = Task {
            try? await Task.sleep(nanoseconds: UInt64(max(seconds, 0) * 1_000_000_000))
            if !Task.isCancelled {
                flag.fire()
                work.cancel()
            }
        }
        let outcome = await withTaskCancellationHandler { await work.value } onCancel: { work.cancel() }
        watchdog.cancel()
        active[rid] = nil

        var collector: Collector
        var failure: BookTalkCode?
        switch outcome {
        case .finished(let c, let f):
            collector = c
            failure = f
        case .cancelled:
            guard flag.fired else {
                out.yield(.failed(requestId: rid, code: .cancelled))
                return
            }
            collector = Collector(grounding: grounding, language: language, requestId: rid)
            failure = .timeout
        }
        if Task.isCancelled {
            out.yield(.failed(requestId: rid, code: .cancelled))
            return
        }
        for e in collector.finish() { out.yield(e) }

        guard collector.hasAnchor else {
            return finish(Fallback.foundOnly(ctx, retrieval, reason: failure, dropped: collector.dropped), ctx, out)
        }
        let answer = collector.answer(ctx)
        if !answer.violations(index: index).isEmpty {
            return finish(Fallback.foundOnly(ctx, retrieval, reason: .llmFailed, dropped: collector.dropped), ctx, out)
        }
        finish(answer, ctx, out)
    }

    private func finish(_ f: Fallback.Result, _ ctx: Context, _ out: AsyncStream<BookTalkEvent>.Continuation) {
        for e in f.events { out.yield(e) }
        finish(f.answer, ctx, out)
    }

    private func finish(_ answer: BookTalkAnswer, _ ctx: Context, _ out: AsyncStream<BookTalkEvent>.Continuation) {
        let at = ISO8601DateFormatter().string(from: Date())
        history.append(BookTalkStoredTurn(question: ctx.request.question, answer: answer, scope: ctx.request.scope,
                                          frontier: answer.frontier, input: ctx.request.input, at: at))
        if history.count > Self.maxStoredTurns { history.removeFirst(history.count - Self.maxStoredTurns) }
        out.yield(.done(requestId: ctx.request.requestId, answer))
    }
}

struct Context {
    let request: BookTalkRequest
    let intent: BookTalkIntent
    let language: BookTalkLanguage
    let readingPage: Int?
    let index: BookTalkIndex
    var scope: BookTalkScope { request.scope }
    var frontier: Int { request.frontier ?? 0 }
}

/// Sammelt den Modellstrom, prüft Satz für Satz und liefert Ereignisse.
struct Collector: Sendable {
    let grounding: BookTalkGroundingContext
    let language: BookTalkLanguage
    let requestId: String
    private var filter = BookTalkDeltaFilter()
    private var text = ""
    private var done = 0
    private var kept: [BookTalkAnswerSentence] = []
    private var held: [(text: String, citations: [BookTalkCitation], general: Bool)] = []
    private var generalKept = 0
    private var citedKept = 0
    private(set) var dropped = 0
    private(set) var hasAnchor = false

    init(grounding: BookTalkGroundingContext, language: BookTalkLanguage, requestId: String) {
        self.grounding = grounding
        self.language = language
        self.requestId = requestId
    }

    mutating func feed(_ piece: String) -> [BookTalkEvent] {
        text += filter.feed(piece)
        return handle(final: false)
    }

    mutating func finish() -> [BookTalkEvent] {
        text += filter.finish()
        return handle(final: true)
    }

    private mutating func handle(final: Bool) -> [BookTalkEvent] {
        let units = Array(text.utf16)
        let sentences = BookTalkSentences.split(text)
        let limit = final ? sentences.count : max(sentences.count - 1, 0)
        var events: [BookTalkEvent] = []
        while done < limit {
            let s = sentences[done]
            done += 1
            let sentence = String(decoding: units[s.start..<s.end], as: UTF16.self).trimmingCharacters(in: .whitespaces)
            let ids = filter.markers.filter { $0.offset > s.start && $0.offset <= s.end }.flatMap(\.ids)
            let g = BookTalkCitations.evaluate(sentence: sentence, ids: ids, ctx: grounding, generalFree: generalKept == 0)
            switch g.kind {
            case .dropped:
                dropped += 1
            case .general:
                generalKept += 1
                if hasAnchor { events.append(emit(sentence, [], general: true)) } else { held.append((sentence, [], true)) }
            case .cited, .absent:
                hasAnchor = true
                for h in held { events.append(emit(h.text, h.citations, general: h.general)) }
                held.removeAll()
                events.append(emit(sentence, g.citations, general: false))
            }
        }
        return events
    }

    private mutating func emit(_ sentence: String, _ citations: [BookTalkCitation], general: Bool) -> BookTalkEvent {
        var speech = BookTalkSpeech.plain(sentence, language: language)
        if general {
            let prefix = language == .de ? "Allgemein:" : "General:"
            if speech.hasPrefix(prefix) { speech = (language == .de ? "Allgemein gilt:" : "In general:") + speech.dropFirst(prefix.count) }
        } else if !citations.isEmpty {
            citedKept += 1
            if citedKept == 1, grounding.intent != .recap, let c = citations.first {
                speech += " " + BookTalkSpeech.referenceSentence(c, language: language)
            }
        }
        let s = BookTalkAnswerSentence(index: kept.count, text: sentence, speech: speech, citations: citations, general: general)
        kept.append(s)
        return .sentence(requestId: requestId, s)
    }

    func answer(_ ctx: Context) -> BookTalkAnswer {
        let cites = kept.flatMap(\.citations)
        var speech = kept.map(\.speech).joined(separator: " ")
        let tail = BookTalkSpeech.tail(intent: ctx.intent, hasCitation: !cites.isEmpty, language: ctx.language, readingPage: ctx.readingPage)
        if !tail.isEmpty { speech += (speech.isEmpty ? "" : " ") + tail }
        return BookTalkAnswer(
            intent: ctx.intent, sentences: kept, text: kept.map(\.text).joined(separator: " "), speech: speech, citations: cites,
            foundOnly: false, reason: nil, dropped: dropped, scope: ctx.scope, frontier: ctx.frontier)
    }
}

/// Antworten ohne Modell: die zwei besten Fundstellen oder ein Hinweis; nie ein leerer Fehler.
enum Fallback {
    struct Result {
        let events: [BookTalkEvent]
        let answer: BookTalkAnswer
    }

    static func notice(_ ctx: Context, reason: BookTalkCode, _ text: String) -> Result {
        var speech = BookTalkSpeech.plain(text, language: ctx.language)
        var shown = text
        if ctx.scope == .untilPosition {
            let offer = ctx.language == .de ? "Soll ich im ganzen Buch suchen?" : "Shall I search the whole book?"
            shown += " " + offer
            speech += " " + offer
        }
        let s = BookTalkAnswerSentence(index: 0, text: shown, speech: speech, citations: [], general: false)
        let answer = BookTalkAnswer(intent: ctx.intent, sentences: [s], text: shown, speech: speech, citations: [], foundOnly: true,
                                    reason: reason, dropped: 0, scope: ctx.scope, frontier: ctx.frontier)
        return Result(events: [.sentence(requestId: ctx.request.requestId, s)], answer: answer)
    }

    static func foundOnly(_ ctx: Context, _ retrieval: BookTalkRetrieval, reason: BookTalkCode?, dropped: Int = 0) -> Result {
        let de = ctx.language == .de
        var sentences: [BookTalkAnswerSentence] = []
        var cites: [BookTalkCitation] = []
        for (n, e) in retrieval.bestFirst.prefix(2).enumerated() {
            guard let q = BookTalkCitations.bestQuote(statement: ctx.request.question, chunk: e.chunk) else { continue }
            let lead = n == 0 ? (de ? "Dazu steht etwas" : "There is something about it") : (de ? "Außerdem" : "Also")
            let page = ctx.index.page(at: q.start)
            let place = page.map { de ? " auf Seite \($0)" : " on page \($0)" } ?? (de ? " im Buch" : " in the book")
            let c = BookTalkCitation(q: e.id, page: page, chapterIndex: e.chunk.chapterIndex, chapterTitle: e.chunk.chapterTitle,
                                     start: q.start, end: q.end, quote: q.text)
            sentences.append(BookTalkAnswerSentence(
                index: sentences.count, text: "\(lead)\(place): „\(q.text)“",
                speech: "\(lead)\(place): ‚\(BookTalkSpeech.spokenQuote(q.text))‘", citations: [c], general: false))
            cites.append(c)
        }
        guard !sentences.isEmpty else {
            return notice(ctx, reason: reason ?? .noHits, de ? "Dazu finde ich im Buch nichts." : "I find nothing about that in the book.")
        }
        var speech = sentences.map(\.speech).joined(separator: " ")
        if ctx.intent == .locate { speech += de ? " Soll ich dir die Stelle vorlesen?" : " Shall I read the passage to you?" }
        let answer = BookTalkAnswer(
            intent: ctx.intent, sentences: sentences, text: sentences.map(\.text).joined(separator: " "), speech: speech,
            citations: cites, foundOnly: true, reason: reason, dropped: dropped, scope: ctx.scope, frontier: ctx.frontier)
        return Result(events: sentences.map { .sentence(requestId: ctx.request.requestId, $0) }, answer: answer)
    }
}

// MARK: - Feldübergreifende Regeln (§7a), zur Laufzeit

extension BookTalkAnswer {
    /// Regeln, die das JSON-Schema nicht ausdrücken kann. Leer = gültig.
    /// Im Kern führt eine Verletzung zu found_only mit Grund `llm_failed`.
    public func violations(index: BookTalkIndex) -> [String] {
        var v: [String] = []
        if reason != nil, !foundOnly { v.append("reason ohne found_only") }
        if foundOnly, sentences.contains(where: \.general) { v.append("found_only mit general-Satz") }
        if sentences.contains(where: { $0.general && !$0.citations.isEmpty }) { v.append("general-Satz mit Beleg") }
        if !sentences.isEmpty {
            if sentences.map(\.text).joined(separator: " ") != text { v.append("text weicht von den Sätzen ab") }
            if sentences.flatMap(\.citations) != citations { v.append("citations weichen von den Sätzen ab") }
            if sentences.enumerated().contains(where: { $0.offset != $0.element.index }) { v.append("Satzindex nicht fortlaufend") }
        }
        for c in citations {
            guard c.start >= 0, c.end > c.start, c.end <= index.sourceLength else {
                v.append("Beleg außerhalb des Textes")
                continue
            }
            if c.quote.isEmpty || c.quote.utf16.count > BookTalkCitations.maxQuoteChars { v.append("Zitatlänge") }
            if scope == .untilPosition, c.end > frontier { v.append("Beleg hinter der Grenze") }
            guard let chunk = index.chunks.first(where: { $0.start <= c.start && c.end <= $0.end }) else {
                v.append("Beleg liegt in keinem Stück")
                continue
            }
            if !chunk.text.contains(c.quote) { v.append("Zitat nicht im Stück") }
        }
        return v
    }
}
