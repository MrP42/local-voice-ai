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
    /// `start()` läuft (Berechtigung, Erkennerwahl, Engine): nicht erneut betretbar.
    private var starting = false
    /// Jeder Start und jedes `cancel()` zählt hoch; ein Start, dessen Token veraltet ist, baut sich selbst zurück.
    private var token = 0
    /// `finish()` kam, bevor die Aufnahme lief: wird nach dem Start sofort nachgeholt.
    private var pendingFinish: Bool?

    /// Start, Aufnahme oder Abschluss laufen.
    var isBusy: Bool { starting || running || finishing }

    // MARK: Start

    func start(mode: Mode, languageCode: String?) async throws {
        guard !isBusy else { return }
        token += 1
        let mine = token
        starting = true
        pendingFinish = nil
        defer { starting = false }
        Self.removeStaleRecordings()
        do {
            try await run(mode: mode, languageCode: languageCode, token: mine)
        } catch {
            // Auch bei Abbruch während des Starts: dieser Aufruf besitzt die Ressourcen und räumt sie auf.
            await teardown(finalize: false)
            discardWhisperFile()
            pendingFinish = nil
            throw error
        }
        if let noSpeech = pendingFinish {
            pendingFinish = nil
            starting = false
            finish(noSpeech: noSpeech)
        }
    }

    private func run(mode: Mode, languageCode: String?, token mine: Int) async throws {
        guard await AVAudioApplication.requestRecordPermission() else { throw ListenError.microphone }
        try checkCurrent(mine)
        stable = ""; volatile = ""
        let english = languageCode?.lowercased().hasPrefix("en") == true
        let locale = Locale(identifier: english ? "en-US" : "de-DE")
        var transcriber: SpeechTranscriber?
        if SpeechTranscriber.isAvailable, let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) {
            try checkCurrent(mine)
            let candidate = SpeechTranscriber(locale: supported, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [])
            let status = await AssetInventory.status(forModules: [candidate])
            try checkCurrent(mine)
            if status == .installed { transcriber = candidate }
        }
        if transcriber == nil, !Self.whisperInstalled { throw ListenError.noRecognizer }

        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetooth, .defaultToSpeaker])
        try session.setActive(true)
        // Echo-Unterdrückung und Rauschminderung; ohne sie hörte das Mikrofon die Antwort mit: lieber sauber scheitern.
        do { try engine.inputNode.setVoiceProcessingEnabled(true) } catch { throw ListenError.engine }

        tap.reset(pushToTalk: mode == .pushToTalk)
        tap.onSpeechStarted = { [weak self] in Task { @MainActor in self?.onSpeechStarted?() } }
        tap.onEnd = { [weak self] noSpeech in Task { @MainActor in self?.finish(noSpeech: noSpeech) } }
        if let transcriber { try await startAnalyzer(transcriber) } else { try startWhisperFile() }
        try checkCurrent(mine)
        observe()
        try installTapAndStart()
        // Ein `cancel()` zwischen Engine-Start und hier lässt `start()` zurückbauen.
        try checkCurrent(mine)
    }

    private func checkCurrent(_ mine: Int) throws {
        if token != mine { throw CancellationError() }
    }

    /// Reste einer früheren Sitzung (Absturz, Beenden während der Aufnahme).
    private static func removeStaleRecordings() {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory
        let names = (try? fm.contentsOfDirectory(atPath: dir.path)) ?? []
        for name in names where name.hasPrefix("booktalk-") && name.hasSuffix(".caf") {
            try? fm.removeItem(at: dir.appendingPathComponent(name))
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
        guard tap.install(on: engine) else { throw ListenError.format }
        engine.prepare()
        do { try engine.start() } catch { throw ListenError.engine }
        tap.setAccepting(true)
        running = true
    }

    // MARK: Ende

    /// Äußerung beenden: erkannten Text abholen und `onFinished(.text)` melden. Kommt es während des Starts,
    /// wird es nach dem Start nachgeholt.
    func finish(noSpeech: Bool = false) {
        if starting { pendingFinish = noSpeech; return }
        guard running, !finishing else { return }
        finishing = true
        Task { await complete(noSpeech: noSpeech) }
    }

    /// Verwerfen, nichts melden (Barge-in, Bereichswechsel, Hintergrund).
    func cancel() {
        guard isBusy else { return }
        token += 1
        pendingFinish = nil
        // Während des Starts räumt `start()` selbst auf; beim Abschluss wird nichts mehr gemeldet.
        guard !starting, !finishing else { return }
        finishing = true
        Task {
            await teardown(finalize: false)
            discardWhisperFile()
            finishing = false
        }
    }

    /// Wie `cancel()`, kehrt aber erst zurück, wenn Mikrofon und Sitzung wirklich frei sind (höchstens ~4 s).
    func cancelAndWait() async {
        cancel()
        var waited = 0
        while isBusy, waited < 200 {
            try? await Task.sleep(for: .milliseconds(20))
            waited += 1
        }
    }

    private func complete(noSpeech: Bool) async {
        let mine = token
        let usedAnalyzer = analyzer != nil
        await teardown(finalize: usedAnalyzer && !noSpeech)
        var text = (stable + (volatile.isEmpty ? "" : (stable.isEmpty ? "" : " ") + volatile)).trimmingCharacters(in: .whitespacesAndNewlines)
        if let url = whisperFile, !noSpeech, token == mine {
            text = await Self.whisper(url)
        }
        discardWhisperFile()
        finishing = false
        guard token == mine else { return }
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
        let mine = token
        let partial = (stable + " " + volatile).trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            await teardown(finalize: false)
            discardWhisperFile()
            finishing = false
            guard token == mine else { return }
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
        // Nicht deaktivieren: Antwort (Anton) und Vorlesen brauchen sofort wieder eine Wiedergabe-Sitzung.
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
        try? AVAudioSession.sharedInstance().setActive(true)
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

    /// Wandler für das aktuelle Mikrofonformat und Tap auf der Engine. Absichtlich hier (nicht isoliert), damit der
    /// Rückruf nicht den Main Actor erbt: er läuft auf dem Audio-Thread. `false`, wenn ein Wandler fehlt.
    func install(on engine: AVAudioEngine) -> Bool {
        let node = engine.inputNode
        node.removeTap(onBus: 0)
        let mic = node.outputFormat(forBus: 0)
        guard mic.sampleRate > 0, prepareConverters(from: mic) else { return false }
        node.installTap(onBus: 0, bufferSize: 4096, format: mic) { [self] buffer, _ in handle(buffer) }
        return true
    }

    private func prepareConverters(from mic: AVAudioFormat) -> Bool {
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
