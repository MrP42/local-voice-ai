import Foundation
import AVFoundation
import Speech

/// Aufnahme einer Besprechung mit Live-Erkennung (Epic #247, P3).
/// Ein Mikrofonstrom geht in zwei Richtungen: als 16-bit-PCM in eine CAF-Datei
/// (unkomprimiert, bleibt nach einem Absturz bis zum letzten Puffer lesbar) und
/// in `SpeechAnalyzer`. Beide bekommen denselben Puffer oder keinen, damit die
/// Zeitachse der Ergebnisse der Dateizeit entspricht. Pausen liefern nichts.
/// Ob Puffer angenommen werden, entscheidet allein `accepting` unter `lock`:
/// Pause, Unterbrechung, Routenwechsel und Formatfehler sperren synchron.
final class LiveCapture {
    struct Update {
        var finals: [LiveSegment] = []
        var volatile: String = ""
    }

    enum CaptureError: LocalizedError {
        case noSpeechAssets, noFormat, microphone
        var errorDescription: String? {
            switch self {
            case .noSpeechAssets: return "Die deutsche Spracherkennung ist auf diesem iPhone nicht installiert. Bitte unter Modelle laden."
            case .noFormat: return "Das Mikrofonformat wird nicht unterstützt."
            case .microphone: return "Kein Zugriff auf das Mikrofon. Bitte in den Einstellungen erlauben."
            }
        }
    }

    let fileURL: URL
    private let engine = AVAudioEngine()
    private var analyzer: SpeechAnalyzer?
    private var results: Task<Void, Never>?
    private var analyzerFormat: AVAudioFormat?
    private let fileFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)!
    private var observers: [NSObjectProtocol] = []
    private(set) var running = false

    // Vom Audio-Thread benutzt: nur unter `lock` lesen oder setzen.
    private let lock = NSLock()
    private var file: AVAudioFile?
    private var input: AsyncStream<AnalyzerInput>.Continuation?
    private var toAnalyzer: AVAudioConverter?
    private var toFile: AVAudioConverter?
    private var writtenFrames: AVAudioFramePosition = 0
    private var accepting = false
    private var limitReported = false

    /// Höchstdauer, im Aufnahmepfad über die geschriebenen Frames durchgesetzt.
    static let maxSeconds: Double = 2 * 60 * 60 - 0.5
    private var maxFrames: AVAudioFramePosition { AVAudioFramePosition(Self.maxSeconds * fileFormat.sampleRate) }

    /// Neue finalisierte Segmente und der vorläufige Rest (nur Anzeige).
    var onUpdate: (@MainActor (Update) -> Void)?
    var onError: (@MainActor (Error) -> Void)?
    /// Anruf, Siri, Routenwechsel, Formatfehler: Aufnahme steht bereits, Grund für die Anzeige.
    var onInterrupted: (@MainActor (String) -> Void)?
    /// Höchstdauer erreicht: es wird nichts mehr angenommen.
    var onLimit: (@MainActor () -> Void)?

    init(fileURL: URL) { self.fileURL = fileURL }

    /// Geschriebene Audiozeit in Sekunden.
    var seconds: Double { lock.lock(); defer { lock.unlock() }; return Double(writtenFrames) / fileFormat.sampleRate }

    func start() async throws {
        guard await AVAudioApplication.requestRecordPermission() else { throw CaptureError.microphone }
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "de-DE")) else { throw CaptureError.noSpeechAssets }
        let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [.audioTimeRange])
        // Nie stillschweigend Assets laden oder Serverkennung nutzen.
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { throw CaptureError.noSpeechAssets }
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else { throw CaptureError.noFormat }
        analyzerFormat = format

        // Eine Datei je Aufnahme; nach einem Absturz wird sie übernommen, nicht fortgeschrieben.
        guard !FileManager.default.fileExists(atPath: fileURL.path) else { throw CocoaError(.fileWriteFileExists) }
        do { try await startPipeline(transcriber) } catch {
            // Teilstart vollständig zurückbauen, bevor der Aufrufer den Ordner löscht.
            await teardown(finalize: false)
            throw error
        }
    }

    private func startPipeline(_ transcriber: SpeechTranscriber) async throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .spokenAudio, options: [.allowBluetooth, .defaultToSpeaker])
        try session.setActive(true)

        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1,
                                       AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false]
        let written = try AVAudioFile(forWriting: fileURL, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: fileURL.path)

        let (stream, continuation) = AsyncStream<AnalyzerInput>.makeStream()
        lock.lock(); file = written; input = continuation; lock.unlock()
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        self.analyzer = analyzer
        try await analyzer.start(inputSequence: stream)
        results = Self.consume(transcriber, owner: self)

        observe()
        try installTapAndStart()
    }

    /// Sperrt synchron, dann steht die Engine. Nach der Rückkehr wird kein Puffer mehr geschrieben.
    func pause() { suspend() }

    private func suspend() {
        lock.lock(); accepting = false; lock.unlock()
        engine.pause()
        running = false
    }

    /// Fortsetzen; nach Routenwechsel mit neuem Mikrofonformat. Angenommen wird erst
    /// nach erfolgreicher Neukonfiguration.
    func resume() throws {
        lock.lock(); let full = writtenFrames >= maxFrames; lock.unlock()
        guard !full else { return }
        try AVAudioSession.sharedInstance().setActive(true)
        try installTapAndStart()
    }

    /// Beendet Aufnahme und Erkennung; wartet, bis alle Ergebnisse finalisiert sind.
    func stop() async { await teardown(finalize: true) }

    /// Zentraler Rückbau in fester Reihenfolge: sperren, Tap und Engine, Beobachter,
    /// Eingabestrom, Analyzer, Ergebnis-Task (begrenzt), Datei schließen, Audiositzung.
    private func teardown(finalize: Bool) async {
        lock.lock(); accepting = false; let continuation = input; input = nil; lock.unlock()
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        running = false
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        continuation?.finish()
        if finalize { try? await analyzer?.finalizeAndFinishThroughEndOfInput() } else { await analyzer?.cancelAndFinishNow() }
        if let results {
            // Hängt die Erkennung, wird der Task nach 10 s beendet; die Aufnahme ist dann schon gesichert.
            let guardTask = Task { try? await Task.sleep(for: .seconds(10)); results.cancel() }
            await results.value
            guardTask.cancel()
        }
        results = nil; analyzer = nil
        lock.lock(); file = nil; lock.unlock()   // schließt die Datei
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func installTapAndStart() throws {
        guard let format = analyzerFormat else { throw CaptureError.noFormat }
        engine.inputNode.removeTap(onBus: 0)
        let micFormat = engine.inputNode.outputFormat(forBus: 0)
        guard micFormat.sampleRate > 0, let a = AVAudioConverter(from: micFormat, to: format),
              let f = AVAudioConverter(from: micFormat, to: fileFormat) else { throw CaptureError.noFormat }
        lock.lock(); toAnalyzer = a; toFile = f; lock.unlock()
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: micFormat) { [weak self] buffer, _ in self?.handle(buffer) }
        engine.prepare()
        try engine.start()
        lock.lock(); accepting = true; lock.unlock()
        running = true
    }

    private func observe() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
            guard let self, let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            self.suspend()
            Task { @MainActor in self.onInterrupted?("Aufnahme unterbrochen (Anruf oder Systemton). Zum Fortsetzen „Weiter“ tippen.") }
        })
        observers.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { [weak self] note in
            guard let self, self.running, let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
                  let reason = AVAudioSession.RouteChangeReason(rawValue: raw),
                  [.oldDeviceUnavailable, .newDeviceAvailable, .categoryChange, .override, .routeConfigurationChange].contains(reason) else { return }
            self.suspend()
            Task { @MainActor in self.onInterrupted?("Mikrofon gewechselt. Zum Fortsetzen „Weiter“ tippen.") }
        })
        observers.append(center.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
            guard let self else { return }
            self.suspend()
            Task { @MainActor in self.onInterrupted?("Audiogerät gewechselt. Zum Fortsetzen „Weiter“ tippen.") }
        })
    }

    private func handle(_ buffer: AVAudioPCMBuffer) {
        lock.lock()
        guard accepting, let file, let input, let f = toFile, let a = toAnalyzer, let format = analyzerFormat else { lock.unlock(); return }
        let remaining = maxFrames - writtenFrames
        lock.unlock()
        guard remaining > 0 else { return reportLimit() }
        guard let forFile = convert(buffer, with: f, to: fileFormat), let forAnalyzer = convert(buffer, with: a, to: format) else {
            // Kein stilles Verwerfen: sperren und melden, die Lücke wird als Unterbrechung sichtbar.
            lock.lock(); accepting = false; lock.unlock()
            DispatchQueue.main.async { [weak self] in
                self?.engine.pause(); self?.running = false
                Task { @MainActor in self?.onInterrupted?("Audioformat konnte nicht umgewandelt werden. Aufnahme pausiert, zum Fortsetzen „Weiter“ tippen.") }
            }
            return
        }
        let last = AVAudioFramePosition(forFile.frameLength) >= remaining
        if last {
            forFile.frameLength = AVAudioFrameCount(remaining)
            forAnalyzer.frameLength = min(forAnalyzer.frameLength, AVAudioFrameCount(Double(remaining) * format.sampleRate / fileFormat.sampleRate))
        }
        do {
            try file.write(from: forFile)
            lock.lock(); writtenFrames += AVAudioFramePosition(forFile.frameLength); lock.unlock()
            input.yield(AnalyzerInput(buffer: forAnalyzer))
            if last { reportLimit() }
        } catch {
            lock.lock(); accepting = false; lock.unlock()
            Task { @MainActor in self.onError?(error) }
        }
    }

    private func reportLimit() {
        lock.lock(); accepting = false; let first = !limitReported; limitReported = true; lock.unlock()
        guard first else { return }
        Task { @MainActor [weak self] in self?.onLimit?() }
    }

    private func convert(_ buffer: AVAudioPCMBuffer, with converter: AVAudioConverter?, to format: AVAudioFormat) -> AVAudioPCMBuffer? {
        guard let converter else { return nil }
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

    /// Liest die Ergebnisse, ohne die Aufnahme festzuhalten: `owner` nur schwach,
    /// der Task endet mit dem Strom oder spätestens im Rückbau.
    private static func consume(_ transcriber: SpeechTranscriber, owner: LiveCapture) -> Task<Void, Never> {
        Task { [weak owner] in
            var next = 0
            do {
                for try await result in transcriber.results {
                    guard let capture = owner else { return }
                    let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
                    var update = Update()
                    if result.isFinal {
                        guard !text.isEmpty else { continue }
                        let start = result.range.start.seconds, end = result.range.end.seconds
                        update.finals = [LiveSegment(id: "a\(next)", start: start.isFinite ? start : 0, end: end.isFinite ? end : start, text: text)]
                        next += 1
                    } else {
                        update.volatile = text
                    }
                    let u = update, sink = capture.onUpdate
                    await MainActor.run { sink?(u) }
                }
            } catch {
                let sink = owner?.onError
                await MainActor.run { sink?(error) }
            }
        }
    }
}

extension LiveThermal {
    init(_ state: ProcessInfo.ThermalState) {
        switch state {
        case .nominal: self = .nominal
        case .fair: self = .fair
        case .serious: self = .serious
        case .critical: self = .critical
        @unknown default: self = .serious
        }
    }
}
