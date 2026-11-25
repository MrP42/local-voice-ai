import Foundation

final class LiveWhisperSession: @unchecked Sendable {
    let pointer: OpaquePointer
    init(_ pointer: OpaquePointer) { self.pointer = pointer }
    deinit {
        // Die letzte Referenz kann auf dem MainActor fallen; dort nie auf die native Sperre warten.
        let address = UInt(bitPattern: pointer)
        DispatchQueue.global(qos: .utility).async { lv_whisper_free(OpaquePointer(bitPattern: address)) }
    }
}

struct LiveWhisperResult: Decodable, Sendable {
    struct Line: Decodable, Sendable { let start: Double; let end: Double; let text: String }
    var lines: [Line]
    var language: String
    var prob: Double
    var discardedRanges: [LiveRefinementRange] = []
    private enum CodingKeys: String, CodingKey { case lines, language, prob }
}

/// Begrenzte FIFO; verworfene Bereiche bleiben im Sitzungsnachweis für den Endlauf.
@MainActor
final class LiveRefiner {
    var onRefined: ((LiveRefinementRange, [LiveSegment]) -> Void)?
    var onDeferred: ((LiveRefinementRange) -> Void)?
    var onLanguage: ((String) -> Void)?
    private var session: LiveWhisperSession?
    private var pending: [LiveChunk] = []
    private var languageLock: LiveLanguageLock
    private var prompt = ""
    private var nextID = 0
    private var worker: Task<Void, Never>?
    private let cancellation = InferenceCancellation()
    private var stopped = false
    private var activeRange: LiveRefinementRange?
    private let names: [String]
    private let emails: [String]

    init(language: String, names: [String], emails: [String]) {
        languageLock = LiveLanguageLock(fixed: language)
        self.names = names; self.emails = emails
    }

    func enqueue(_ chunk: LiveChunk) {
        guard !stopped else { deferChunk(chunk); return }
        pending.append(chunk)
        let thermal = LiveThermal(ProcessInfo.processInfo.thermalState)
        if pending.count > 3 || thermal >= .serious {
            for chunk in pending { deferChunk(chunk) }
            pending = []
            return
        }
        if worker == nil {
            worker = Task { [weak self] in
                guard let self else { return }
                await self.drain()
                self.worker = nil
            }
        }
    }

    private func deferChunk(_ chunk: LiveChunk) {
        guard !stopped else { return } // stop hat laufenden und wartenden Bereich bereits gesichert.
        onDeferred?(LiveRefinementRange(start: chunk.start, end: chunk.end))
    }

    private func drain() async {
        defer { if stopped { session = nil } }
        while !pending.isEmpty && !stopped {
            let chunk = pending.removeFirst()
            activeRange = LiveRefinementRange(start: chunk.start, end: chunk.end)
            defer { activeRange = nil }
            guard LiveThermal(ProcessInfo.processInfo.thermalState) < .serious else { deferChunk(chunk); continue }
            do {
                if session == nil {
                    session = try await CPULocalProviders.shared.openLiveWhisper(cancellation: cancellation)
                    guard !stopped, let session else { deferChunk(chunk); return }
                    prompt = await CPULocalProviders.shared.livePrompt(session, names: names, emails: emails,
                        glossary: UserDefaults.standard.string(forKey: "sttGlossary") ?? "")
                }
                guard !stopped, let session else { deferChunk(chunk); return }
                let had = languageLock.language(track: chunk.track)
                var result = try await call(session, chunk, language: had)
                guard !stopped, !Task.isCancelled else { return }
                var readLanguage = had == "auto" ? result.language : had
                if let retry = languageLock.retry(track: chunk.track, language: result.language, prob: result.prob, text: text(result)) {
                    let again = try await call(session, chunk, language: retry)
                    guard !stopped, !Task.isCancelled else { return }
                    if !LiveHallucination.isGarbled(text(again)) {
                        let prob = retry == "auto" ? again.prob : max(result.prob, again.prob)
                        result = again
                        if retry != "auto" { result.language = retry }
                        result.prob = prob
                        readLanguage = retry == "auto" ? again.language : retry
                    } else {
                        // Eine misslungene Gegenlesung darf keinen Wechsel zählen.
                        result.prob = 0
                    }
                }
                guard !stopped, !Task.isCancelled,
                      LiveThermal(ProcessInfo.processInfo.thermalState) < .serious else { deferChunk(chunk); continue }
                languageLock.accept(track: chunk.track, language: result.language, prob: result.prob, text: text(result))
                let locked = languageLock.language(track: chunk.track)
                if locked != had && locked != "auto" { onLanguage?(locked) }
                let candidates = result.lines.compactMap { line -> LiveSegment? in
                    guard line.start.isFinite, line.end.isFinite, line.start >= 0, line.end > line.start,
                          line.start < Double(chunk.samples.count) / 16000 else { return nil }
                    defer { nextID += 1 }
                    return LiveSegment(id: "w\(nextID)", start: chunk.start + line.start,
                                       end: min(chunk.end, chunk.start + line.end), text: line.text,
                                       language: readLanguage, refined: true)
                }
                let lines = LiveRefinement.excluding(candidates, ranges: result.discardedRanges)
                for range in result.discardedRanges { onDeferred?(range) }
                if lines.isEmpty { deferChunk(chunk) }
                else { onRefined?(LiveRefinementRange(start: chunk.start, end: chunk.end), lines) }
            } catch { deferChunk(chunk) }
        }
    }

    private func text(_ result: LiveWhisperResult) -> String { result.lines.map(\.text).joined(separator: " ") }

    private func call(_ session: LiveWhisperSession, _ chunk: LiveChunk, language: String) async throws -> LiveWhisperResult {
        var result = try await CPULocalProviders.shared.refine(session, chunk: chunk, language: language,
            prompt: prompt, guess: languageLock.fixed == nil, cancellation: cancellation)
        let wholeSectionIsEcho = !result.lines.isEmpty && result.lines.allSatisfy { LiveHallucination.isPromptEcho($0.text, prompt: prompt) }
        let nearSilence = LiveHallucination.nearSilence(chunk.samples)
        var discarded: [LiveRefinementRange] = []
        result.lines = result.lines.compactMap { line in
            guard let clean = LiveHallucination.clean(line.text, prompt: prompt, clearMs: chunk.clearMs,
                wholeSectionIsEcho: wholeSectionIsEcho, nearSilence: nearSilence) else {
                let valid = line.start.isFinite && line.end.isFinite && line.start >= 0 && line.end > line.start && chunk.start + line.start < chunk.end
                discarded.append(LiveRefinementRange(start: valid ? chunk.start + line.start : chunk.start,
                    end: valid ? min(chunk.end, chunk.start + line.end) : chunk.end))
                return nil
            }
            return LiveWhisperResult.Line(start: line.start, end: line.end, text: clean)
        }
        result.discardedRanges = discarded
        return result
    }

    func stop() async {
        guard !stopped else { return }
        stopped = true
        cancellation.cancel(); worker?.cancel()
        if let activeRange { onDeferred?(activeRange) }
        for chunk in pending { onDeferred?(LiveRefinementRange(start: chunk.start, end: chunk.end)) }
        pending = []
        // Null Wartezeit auf fremde Actor-Arbeit oder natives Laden ohne Abbruchcallback.
        // Der Worker hält self, Token und Kontext bis drain tatsächlich zurückkehrt.
        if worker == nil { session = nil }
    }
}
