import Foundation
import AVFoundation
import Speech

/// Hören für das Buch-Gespräch (Epic #307, G5): eine Äußerung je Start.
/// - Erkennung: `SpeechTranscriber`/`SpeechAnalyzer` live über `AVAudioEngine`, nie Server-Erkennung und nie
///   still nachgeladene Assets. Fehlen sie, fällt es auf das vorhandene Whisper zurück (CPU, nach dem Ende der Äußerung).
/// - Audio: `.playAndRecord`, Modus `.voiceChat`. Audio wird nie gespeichert; der Whisper-Rückfall schreibt kurz in eine
///   temporäre Datei, die sofort nach der Erkennung gelöscht wird.
/// - Ende: Freihand nach 800 ms Stille (`BookTalkEndpointer`), Push-to-talk beim Loslassen, jederzeit über `finish()`.
@MainActor
final class BookTalkListener {
    enum Mode { case handsFree, pushToTalk }

    enum Outcome {
        /// Erkannter Text.
        case text(String)
        /// Nichts gesagt oder nichts verstanden.
        case noSpeech
        /// Unterbrochen (Anruf, Routenwechsel); `partial` ist das bisher Erkannte.
        case interrupted(String, partial: String)
    }

    enum ListenError: LocalizedError {
        case microphone, noRecognizer, format, engine
        var errorDescription: String? {
            switch self {
            case .microphone: "Kein Zugriff auf das Mikrofon. Bitte in den Einstellungen erlauben."
            case .noRecognizer: "Keine Spracherkennung verfügbar. Bitte unter Einstellungen die Apple-Sprachdateien oder Whisper laden. Du kannst auch tippen."
            case .format: "Das Mikrofonformat wird nicht unterstützt."
            case .engine: "Das Mikrofon konnte nicht gestartet werden."
            }
        }
    }

    /// Erkannter Text: fertiger Teil und vorläufiger Rest.
    var onText: ((_ stable: String, _ volatile: String) -> Void)?
    var onSpeechStarted: (() -> Void)?
    var onFinished: ((Outcome) -> Void)?

    private let engine = AVAudioEngine()
    private let tap = TapState()
    private var analyzer: SpeechAnalyzer?
    private var results: Task<Void, Never>?
    private var observers: [NSObjectProtocol] = []
    private var whisperFile: URL?
    private var stable = ""
    private var volatile = ""
    private(set) var running = false
    private var finishing = false

    /// Laufende Äußerung läuft oder wird gerade abgeschlossen.
    var isBusy: Bool { running || finishing }

    // MARK: Start

    func start(mode: Mode, languageCode: String?) async throws {
        guard !isBusy else { return }
        guard await AVAudioApplication.requestRecordPermission() else { throw ListenError.microphone }
        stable = ""; volatile = ""
        let english = languageCode?.lowercased().hasPrefix("en") == true
        let locale = Locale(identifier: english ? "en-US" : "de-DE")
        var transcriber: SpeechTranscriber?
        if SpeechTranscriber.isAvailable, let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) {
            let candidate = SpeechTranscriber(locale: supported, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [])
            if await AssetInventory.status(forModules: [candidate]) == .installed { transcriber = candidate }
        }
        if transcriber == nil, !Self.whisperInstalled { throw ListenError.noRecognizer }

        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetooth, .defaultToSpeaker])
        try session.setActive(true)
        // Echo-Unterdrückung und Rauschminderung; schlägt es fehl, bleibt der Modus `.voiceChat`.
        try? engine.inputNode.setVoiceProcessingEnabled(true)

        tap.reset(pushToTalk: mode == .pushToTalk)
        tap.onSpeechStarted = { [weak self] in Task { @MainActor in self?.onSpeechStarted?() } }
        tap.onEnd = { [weak self] noSpeech in Task { @MainActor in self?.finish(noSpeech: noSpeech) } }
        do {
            if let transcriber { try await startAnalyzer(transcriber) } else { try startWhisperFile() }
            observe()
            try installTapAndStart()
        } catch {
            await teardown(finalize: false)
            discardWhisperFile()
            throw error
        }
    }

    private static var whisperInstalled: Bool {
        let name = UserDefaults.standard.string(forKey: "sttModel") ?? "ggml-base.bin"
        return FileManager.default.fileExists(atPath: ModelLibrary.folder.appendingPathComponent(name).path)
    }

    private func startAnalyzer(_ transcriber: SpeechTranscriber) async throws {
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else { throw ListenError.format }
        let (stream, continuation) = AsyncStream<AnalyzerInput>.makeStream()
        tap.setAnalyzer(input: continuation, format: format)
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        self.analyzer = analyzer
        try await analyzer.start(inputSequence: stream)
        results = Task { [weak self] in
            do {
                for try await result in transcriber.results {
                    let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
                    self?.received(text, final: result.isFinal)
                }
            } catch {
                // Erkennungsfehler: das bisher Erkannte bleibt; das Ende meldet `finish`.
            }
        }
    }

    private func received(_ text: String, final: Bool) {
        if final {
            if !text.isEmpty { stable = stable.isEmpty ? text : stable + " " + text }
            volatile = ""
        } else {
            volatile = text
        }
        onText?(stable, volatile)
    }

    private func startWhisperFile() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("booktalk-" + UUID().uuidString + ".caf")
        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1,
                                       AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false]
        let written = try AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: url.path)
        whisperFile = url
        tap.setFile(written)
    }

    private func discardWhisperFile() {
        if let whisperFile { try? FileManager.default.removeItem(at: whisperFile) }
        whisperFile = nil
    }

    private func installTapAndStart() throws {
        engine.inputNode.removeTap(onBus: 0)
        let micFormat = engine.inputNode.outputFormat(forBus: 0)
        guard micFormat.sampleRate > 0, tap.prepareConverters(from: micFormat) else { throw ListenError.format }
        let tap = self.tap
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: micFormat) { buffer, _ in tap.handle(buffer) }
        engine.prepare()
        do { try engine.start() } catch { throw ListenError.engine }
        tap.setAccepting(true)
        running = true
    }

    // MARK: Ende

    /// Äußerung beenden: erkannten Text abholen und `onFinished(.text)` melden.
    func finish(noSpeech: Bool = false) {
        guard running, !finishing else { return }
        finishing = true
        Task { await complete(noSpeech: noSpeech) }
    }

    /// Verwerfen, nichts melden (Barge-in, Bereichswechsel, Hintergrund).
    func cancel() {
        guard isBusy else { return }
        finishing = true
        Task {
            await teardown(finalize: false)
            discardWhisperFile()
            finishing = false
        }
    }

    private func complete(noSpeech: Bool) async {
        let usedAnalyzer = analyzer != nil
        await teardown(finalize: usedAnalyzer && !noSpeech)
        var text = (stable + (volatile.isEmpty ? "" : (stable.isEmpty ? "" : " ") + volatile)).trimmingCharacters(in: .whitespacesAndNewlines)
        if let url = whisperFile, !noSpeech {
            text = await Self.whisper(url)
        }
        discardWhisperFile()
        finishing = false
        onFinished?(noSpeech || text.isEmpty ? .noSpeech : .text(text))
    }

    private static func whisper(_ url: URL) async -> String {
        let cancellation = InferenceCancellation()
        let text = try? await withTaskCancellationHandler {
            try await CPULocalProviders.shared.transcribe(url, cancellation: cancellation)
        } onCancel: { cancellation.cancel() }
        return (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func interrupted(_ why: String) {
        guard running, !finishing else { return }
        finishing = true
        let partial = (stable + " " + volatile).trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            await teardown(finalize: false)
            discardWhisperFile()
            finishing = false
            onFinished?(.interrupted(why, partial: partial))
        }
    }

    /// Rückbau in fester Reihenfolge: sperren, Tap und Engine, Beobachter, Eingabestrom, Analyzer, Ergebnisse, Sitzung.
    private func teardown(finalize: Bool) async {
        let continuation = tap.stop()
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        running = false
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        continuation?.finish()
        if let analyzer {
            let done = finalize ? await LiveCapture.bounded(3) { try? await analyzer.finalizeAndFinishThroughEndOfInput() } : false
            if !done { _ = await LiveCapture.bounded(2) { await analyzer.cancelAndFinishNow() } }
        }
        if let results {
            if !finalize { results.cancel() }
            _ = await LiveCapture.bounded(2) { await results.value }
            results.cancel()
        }
        results = nil; analyzer = nil
        tap.clear()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func observe() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            MainActor.assumeIsolated { self?.interrupted("Zuhören unterbrochen (Anruf oder Systemton).") }
        })
        observers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] note in
            guard let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                  let reason = AVAudioSession.RouteChangeReason(rawValue: raw),
                  [.oldDeviceUnavailable, .newDeviceAvailable].contains(reason) else { return }
            MainActor.assumeIsolated { self?.interrupted("Mikrofon gewechselt.") }
        })
        observers.append(center.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.interrupted("Audiogerät gewechselt.") }
        })
    }
}

/// Alles, was der Audio-Thread anfasst: unter `lock`, ohne Main-Actor-Bezug (wie `LiveCapture`).
private final class TapState: @unchecked Sendable {
    /// Obergrenze je Äußerung bei Push-to-talk; Whisper verarbeitet höchstens 60 s.
    static let maxPushMs = 55_000

    var onSpeechStarted: (@Sendable () -> Void)?
    var onEnd: (@Sendable (_ noSpeech: Bool) -> Void)?

    private let lock = NSLock()
    private let fileFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)!
    private var accepting = false
    private var pushToTalk = false
    private var endpointer = BookTalkEndpointer()
    private var input: AsyncStream<AnalyzerInput>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var toAnalyzer: AVAudioConverter?
    private var toFile: AVAudioConverter?
    private var file: AVAudioFile?
    private var elapsedMs = 0
    private var endSent = false

    func reset(pushToTalk: Bool) {
        lock.lock(); defer { lock.unlock() }
        self.pushToTalk = pushToTalk
        endpointer = BookTalkEndpointer()
        elapsedMs = 0
        endSent = false
    }

    func setAnalyzer(input: AsyncStream<AnalyzerInput>.Continuation, format: AVAudioFormat) {
        lock.lock(); defer { lock.unlock() }
        self.input = input
        analyzerFormat = format
    }

    func setFile(_ file: AVAudioFile) {
        lock.lock(); defer { lock.unlock() }
        self.file = file
    }

    /// Wandler für das aktuelle Mikrofonformat; `false`, wenn einer fehlt.
    func prepareConverters(from mic: AVAudioFormat) -> Bool {
        lock.lock(); defer { lock.unlock() }
        if let analyzerFormat { toAnalyzer = AVAudioConverter(from: mic, to: analyzerFormat) }
        toFile = AVAudioConverter(from: mic, to: fileFormat)
        return (input == nil || toAnalyzer != nil) && (file == nil || toFile != nil)
    }

    func setAccepting(_ value: Bool) {
        lock.lock(); accepting = value; lock.unlock()
    }

    /// Sperrt (wartet auf einen laufenden Puffer) und gibt den Eingabestrom zum Schließen zurück.
    func stop() -> AsyncStream<AnalyzerInput>.Continuation? {
        lock.lock(); defer { lock.unlock() }
        accepting = false
        let continuation = input
        input = nil
        return continuation
    }

    func clear() {
        lock.lock(); defer { lock.unlock() }
        file = nil; toAnalyzer = nil; toFile = nil; analyzerFormat = nil; input = nil
    }

    func handle(_ buffer: AVAudioPCMBuffer) {
        let frames = Int(buffer.frameLength)
        let rate = buffer.format.sampleRate
        guard frames > 0, rate > 0 else { return }
        let ms = max(1, Int(Double(frames) / rate * 1000))
        let level = Self.rms(buffer)

        lock.lock()
        guard accepting else { lock.unlock(); return }
        var event = endpointer.feed(rms: level, ms: ms)
        elapsedMs += ms
        if pushToTalk {
            // Beim Halten entscheidet das Loslassen; nur die Höchstdauer beendet von selbst.
            if event != .speechStarted { event = .none }
            if elapsedMs >= Self.maxPushMs { event = .ended }
        }
        if let a = toAnalyzer, let format = analyzerFormat, let input, let converted = Self.convert(buffer, with: a, to: format) {
            input.yield(AnalyzerInput(buffer: converted))
        } else if let file, let f = toFile, let converted = Self.convert(buffer, with: f, to: fileFormat) {
            try? file.write(from: converted)
        }
        let ending = (event == .ended || event == .noSpeech) && !endSent
        if ending { endSent = true }
        let started = event == .speechStarted
        lock.unlock()

        if started { onSpeechStarted?() }
        if ending { onEnd?(event == .noSpeech) }
    }

    private static func rms(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData, buffer.frameLength > 0 else { return 0 }
        let count = Int(buffer.frameLength)
        let channel = data[0]
        var sum: Float = 0
        for i in 0..<count { sum += channel[i] * channel[i] }
        return (sum / Float(count)).squareRoot()
    }

    private static func convert(_ buffer: AVAudioPCMBuffer, with converter: AVAudioConverter, to format: AVAudioFormat) -> AVAudioPCMBuffer? {
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        var fed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if fed { status.pointee = .noDataNow; return nil }
            fed = true; status.pointee = .haveData; return buffer
        }
        return error == nil && out.frameLength > 0 ? out : nil
    }
}
