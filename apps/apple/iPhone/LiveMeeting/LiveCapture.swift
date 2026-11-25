import Foundation
import AVFoundation
import Speech

/// Aufnahme einer Besprechung mit Live-Erkennung (Epic #247, P3).
/// Ein Mikrofonstrom geht als 16-bit-PCM in eine CAF-Datei
/// (unkomprimiert, bleibt nach einem Absturz bis zum letzten Puffer lesbar) und
/// optional in `SpeechAnalyzer`. Die geschriebenen 16-kHz-Puffer werden außerdem
/// in den Whisper-Ring kopiert; beide Erkennungen beziehen sich auf die Dateizeit.
/// Pausen liefern nichts, Analyzer-Neustarts erhalten einen neuen Zeitversatz.
/// Ob Puffer angenommen werden, entscheidet allein `accepting` unter `lock`.
/// `handle` hält `lock` vom Prüfen bis zum Schreiben; Pause, Unterbrechung,
/// Routenwechsel, Konverterwechsel und Rückbau nehmen denselben Lock und warten
/// damit, bis ein laufender Callback fertig ist. Danach wird nichts mehr geschrieben.
// Audiozustand unter lock; Chunker nur auf refinementQueue, Lebenszyklus auf MainActor.
final class LiveCapture: @unchecked Sendable {
    struct Update: Sendable {
        var finals: [LiveSegment] = []
        var volatile: String = ""
    }

    enum CaptureError: LocalizedError {
        case noSpeechAssets, noFormat, microphone
        case invalidLanguage
        var errorDescription: String? {
            switch self {
            case .noSpeechAssets:
                return "Die gewählte Spracherkennung ist auf diesem iPhone nicht installiert. "
                    + "Bitte unter Modelle laden."
            case .noFormat: return "Das Mikrofonformat wird nicht unterstützt."
            case .microphone: return "Kein Zugriff auf das Mikrofon. Bitte in den Einstellungen erlauben."
            case .invalidLanguage: return "Die gewählte Sprache wird nicht unterstützt. Bitte in den Einstellungen prüfen."
            }
        }
    }

    let fileURL: URL
    private let engine = AVAudioEngine()
    private var analyzer: SpeechAnalyzer?
    private var results: Task<Void, Never>?
    private var analyzerFormat: AVAudioFormat?
    private var recognitionLanguage: String?
    private var recognitionGeneration = 0
    private var recognitionOffset: Double = 0
    private var recognitionFailed = false
    @MainActor private var languageHandover: Task<Void, Never>?
    @MainActor private var pendingLanguage: String?
    private var nextAppleID = 0
    private var ending = false
    private let refinementQueue = DispatchQueue(label: "de.localvoice.live-chunker")
    private var refinementTimer: DispatchSourceTimer?
    private var chunker = LiveChunker()
    private var ring = LiveAudioRing()
    /// Nur bei Lautsprecher-Wiedergabe während der Aufnahme einschalten.
    var voiceProcessing = false
    var onChunk: (@MainActor @Sendable (LiveChunk) -> Void)?
    var onRefinementGap: (@MainActor @Sendable (LiveRefinementRange) -> Void)?
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
    var onUpdate: (@MainActor @Sendable (Update) -> Void)?
    var onError: (@MainActor @Sendable (Error) -> Void)?
    /// Anruf, Siri, Routenwechsel, Formatfehler: Aufnahme steht bereits, Grund für die Anzeige.
    var onInterrupted: (@MainActor @Sendable (String) -> Void)?
    /// Höchstdauer erreicht: es wird nichts mehr angenommen.
    var onLimit: (@MainActor @Sendable () -> Void)?

    init(fileURL: URL) { self.fileURL = fileURL }

    /// Geschriebene Audiozeit in Sekunden.
    var seconds: Double { lock.lock(); defer { lock.unlock() }; return Double(writtenFrames) / fileFormat.sampleRate }

    @MainActor
    func start() async throws {
        ending = false
        let locale = try await LocalProviders.speechLocale()
        guard await AVAudioApplication.requestRecordPermission() else { throw CaptureError.microphone }
        var transcriber: SpeechTranscriber?
        // Nie stillschweigend Assets laden oder Serverkennung nutzen.
        if SpeechTranscriber.isAvailable, let locale {
            let candidate = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [.audioTimeRange])
            if await AssetInventory.status(forModules: [candidate]) == .installed,
               let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [candidate]) {
                recognitionLanguage = locale.identifier
                analyzerFormat = format
                transcriber = candidate
            }
        }

        // Eine Datei je Aufnahme; nach einem Absturz wird sie übernommen, nicht fortgeschrieben.
        guard !FileManager.default.fileExists(atPath: fileURL.path) else { throw CocoaError(.fileWriteFileExists) }
        do { try await startPipeline(transcriber) } catch {
            // Teilstart vollständig zurückbauen, bevor der Aufrufer den Ordner löscht.
            await teardown(finalize: false)
            throw error
        }
    }

    @MainActor
    private func startPipeline(_ transcriber: SpeechTranscriber?) async throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .spokenAudio, options: [.allowBluetooth, .defaultToSpeaker])
        try session.setActive(true)

        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1,
                                       AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false]
        let written = try AVAudioFile(forWriting: fileURL, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: fileURL.path)

        lock.withLock { file = written }
        if let transcriber {
            let (stream, continuation) = AsyncStream<AnalyzerInput>.makeStream()
            lock.withLock { input = continuation }
            let analyzer = SpeechAnalyzer(modules: [transcriber])
            self.analyzer = analyzer
            try await analyzer.start(inputSequence: stream)
            results = Self.consume(transcriber, owner: self, language: recognitionLanguage)
        }

        observe()
        startRefinement()
        try installTapAndStart()
    }

    /// Sperrt synchron, dann steht die Engine. Nach der Rückkehr wird kein Puffer mehr geschrieben.
    func pause() { suspend() }

    private func suspend() {
        lock.lock(); accepting = false; appendBoundaryLocked(); lock.unlock()   // wartet auf einen laufenden Callback
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
    @MainActor
    func stop() async { await teardown(finalize: true) }

    /// Zentraler Rückbau in fester Reihenfolge: sperren, Tap und Engine, Beobachter,
    /// Eingabestrom, Analyzer, Ergebnis-Task (begrenzt), Datei schließen, Audiositzung.
    @MainActor
    private func teardown(finalize: Bool) async {
        ending = true
        lock.withLock { accepting = false }
        // Ein laufender Wechsel sichert seine alten Ergebnisse und Audiolücken zuerst.
        if let languageHandover { _ = await Self.bounded(15) { await languageHandover.value } }
        let continuation = lock.withLock {
            accepting = false
            defer { input = nil }
            return input
        }
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        ring.publishMissed()
        await flushRefinement()
        running = false
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        continuation?.finish()
        // Jeder Schritt ist zeitlich begrenzt; die Aufnahme liegt zu diesem Zeitpunkt schon in der Datei.
        if let analyzer {
            let done = finalize ? await Self.bounded(10) { try? await analyzer.finalizeAndFinishThroughEndOfInput() } : false
            if !done { _ = await Self.bounded(3) { await analyzer.cancelAndFinishNow() } }
        }
        if let results {
            results.cancel()
            _ = await Self.bounded(3) { await results.value }
        }
        results = nil; analyzer = nil
        lock.withLock { file = nil }   // schließt die Datei
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func installTapAndStart() throws {
        engine.inputNode.removeTap(onBus: 0)
        let processing = voiceProcessing && AVAudioSession.sharedInstance().currentRoute.outputs.contains { $0.portType == .builtInSpeaker }
        if engine.inputNode.isVoiceProcessingEnabled != processing { try engine.inputNode.setVoiceProcessingEnabled(processing) }
        let micFormat = engine.inputNode.outputFormat(forBus: 0)
        guard micFormat.sampleRate > 0,
              let f = AVAudioConverter(from: micFormat, to: fileFormat) else { throw CaptureError.noFormat }
        let a = analyzerFormat.flatMap { AVAudioConverter(from: micFormat, to: $0) }
        if analyzerFormat != nil && a == nil { throw CaptureError.noFormat }
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
        lock.lock(); defer { lock.unlock() }
        guard accepting, let file, let f = toFile else { return }
        let remaining = maxFrames - writtenFrames
        guard remaining > 0 else { return limitLocked() }
        guard let forFile = convert(buffer, with: f, to: fileFormat) else {
            // Kein stilles Verwerfen: sperren und melden, die Lücke wird als Unterbrechung sichtbar.
            return failLocked("Audioformat konnte nicht umgewandelt werden. Aufnahme pausiert, zum Fortsetzen „Weiter“ tippen.")
        }
        var forAnalyzer: AVAudioPCMBuffer?
        if input != nil, let format = analyzerFormat {
            guard let converted = convert(buffer, with: toAnalyzer, to: format) else {
                return failLocked("Audioformat konnte nicht umgewandelt werden. Aufnahme pausiert, zum Fortsetzen „Weiter“ tippen.")
            }
            forAnalyzer = converted
        }
        let last = AVAudioFramePosition(forFile.frameLength) >= remaining
        if last {
            forFile.frameLength = AVAudioFrameCount(remaining)
            if let forAnalyzer, let format = analyzerFormat {
                forAnalyzer.frameLength = min(forAnalyzer.frameLength, AVAudioFrameCount(Double(remaining) * format.sampleRate / fileFormat.sampleRate))
            }
        }
        do {
            try file.write(from: forFile)
        } catch {
            return failLocked("Die Aufnahme konnte nicht gespeichert werden (Speicher voll?). Aufnahme pausiert.")
        }
        if let samples = forFile.floatChannelData?[0] {
            ring.append(startFrame: writtenFrames, samples: UnsafeBufferPointer(start: samples, count: Int(forFile.frameLength)))
        }
        writtenFrames += AVAudioFramePosition(forFile.frameLength)
        if let forAnalyzer { input?.yield(AnalyzerInput(buffer: forAnalyzer)) }
        if last { limitLocked() }
    }

    /// Unter `lock`: sperren, Engine auf dem Hauptthread pausieren, Unterbrechung melden.
    private func failLocked(_ why: String) {
        accepting = false
        appendBoundaryLocked()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.engine.pause(); self.running = false
            Task { @MainActor in self.onInterrupted?(why) }
        }
    }

    /// Unter `lock`: Höchstdauer erreicht, einmal melden.
    private func limitLocked() {
        accepting = false
        appendBoundaryLocked()
        guard !limitReported else { return }
        limitReported = true
        Task { @MainActor [weak self] in self?.onLimit?() }
    }

    private func appendBoundaryLocked() {
        ring.append(startFrame: writtenFrames, samples: UnsafeBufferPointer<Float>(start: nil, count: 0))
    }

    private func startRefinement() {
        let timer = DispatchSource.makeTimerSource(queue: refinementQueue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(100))
        timer.setEventHandler { [weak self] in self?.drainRefinement() }
        refinementTimer = timer
        timer.resume()
    }

    /// Eigener Ring-Lock; dessen Verbraucher kann den Tap niemals warten lassen.
    private func drainRefinement() {
        var chunks: [LiveChunk] = []
        while true {
            let packet = ring.pop()
            guard let packet else { break }
            if packet.boundaryBefore {
                chunks += chunker.push(track: "mic", startFrame: packet.startFrame, samples: [])
            }
            chunks += chunker.push(track: "mic", startFrame: packet.startFrame, samples: packet.samples)
        }
        let missed = ring.takeMissed()
        // Bei Überlauf niemals zwei nicht benachbarte Audioteile zusammenlesen.
        if missed != nil { chunks += chunker.flush() }
        let ready = chunks
        DispatchQueue.main.async { [weak self] in
            for chunk in ready { self?.onChunk?(chunk) }
            if let missed { self?.onRefinementGap?(missed) }
        }
    }

    private func flushRefinement() async {
        refinementTimer?.cancel(); refinementTimer = nil
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            refinementQueue.async { [self] in
                drainRefinement()
                let tail = chunker.flush()
                DispatchQueue.main.async { [weak self] in
                    for chunk in tail { self?.onChunk?(chunk) }
                    continuation.resume()
                }
            }
        }
    }

    /// Nur installierte Apple-Assets benutzen. Die Datei läuft während des Wechsels weiter.
    @MainActor
    func followLanguage(_ language: String) async {
        guard !ending else { return }
        pendingLanguage = language
        if let languageHandover { await languageHandover.value; return }
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            defer { self.languageHandover = nil }
            while !self.ending, let language = self.pendingLanguage {
                self.pendingLanguage = nil
                await self.replaceRecognitionLanguage(language)
            }
        }
        languageHandover = task
        await task.value
    }

    @MainActor
    private func replaceRecognitionLanguage(_ language: String) async {
        let selected = UserDefaults.standard.string(forKey: "sttLanguage") ?? "auto"
        let explicitRegion = selected != "auto" && Locale(identifier: selected).region != nil
        let requested = explicitRegion ? selected : language
        if analyzer != nil, let recognitionLanguage,
           LiveLanguageLock.equivalentLocale(recognitionLanguage, requested, explicitRegion: explicitRegion) { return }
        guard !ending, SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: requested)) else { return }
        let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [.audioTimeRange])
        guard await AssetInventory.status(forModules: [transcriber]) == .installed,
              let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]),
              !ending else { return }
        let next = SpeechAnalyzer(modules: [transcriber])
        let (stream, continuation) = AsyncStream<AnalyzerInput>.makeStream()
        do { try await next.start(inputSequence: stream) } catch {
            continuation.finish()
            _ = await Self.bounded(3) { await next.cancelAndFinishNow() }
            return
        }
        let mic = engine.inputNode.outputFormat(forBus: 0)
        guard !ending, let converter = AVAudioConverter(from: mic, to: format) else {
            continuation.finish()
            _ = await Self.bounded(3) { await next.cancelAndFinishNow() }
            return
        }

        let previous = analyzer, previousResults = results
        let previousOffset = recognitionOffset
        // Der Lock trennt ganze Tap-Puffer. Die Datei läuft weiter, der alte Strom endet hier.
        let (previousInput, boundary) = lock.withLock {
            let previousInput = input
            input = nil
            return (previousInput, Double(writtenFrames) / 16000)
        }
        previousInput?.finish()
        var finalized = true
        if let previous {
            finalized = await Self.boundedThrowing(3) { try await previous.finalizeAndFinishThroughEndOfInput() }
        }
        if let previousResults {
            let drained = await Self.bounded(3) { await previousResults.value }
            finalized = finalized && drained
            previousResults.cancel()
        }
        let failed = lock.withLock { recognitionFailed }
        if (!finalized || failed), boundary > previousOffset {
            onRefinementGap?(LiveRefinementRange(start: previousOffset, end: boundary))
        }
        lock.withLock { recognitionGeneration += 1 }
        results = nil; analyzer = nil
        if !finalized, let previous { _ = await Self.bounded(3) { await previous.cancelAndFinishNow() } }
        // Auch bei Stopp/Formatwechsel darf die seit der Grenze geschriebene Rede nicht verschwinden.
        guard !ending else {
            let resumed = seconds
            if resumed > boundary { onRefinementGap?(LiveRefinementRange(start: boundary, end: resumed)) }
            continuation.finish()
            _ = await Self.bounded(3) { await next.cancelAndFinishNow() }
            return
        }
        let currentMic = engine.inputNode.outputFormat(forBus: 0)
        guard let currentConverter = currentMic == mic ? converter : AVAudioConverter(from: currentMic, to: format) else {
            let resumed = seconds
            if resumed > boundary { onRefinementGap?(LiveRefinementRange(start: boundary, end: resumed)) }
            onError?(CaptureError.noFormat)
            continuation.finish()
            _ = await Self.bounded(3) { await next.cancelAndFinishNow() }
            return
        }
        onUpdate?(Update())
        let resumed = lock.withLock {
            recognitionOffset = Double(writtenFrames) / 16000
            recognitionFailed = false
            recognitionLanguage = locale.identifier
            analyzerFormat = format; toAnalyzer = currentConverter; input = continuation
            return recognitionOffset
        }
        analyzer = next
        results = Self.consume(transcriber, owner: self, language: locale.identifier)
        if resumed > boundary { onRefinementGap?(LiveRefinementRange(start: boundary, end: resumed)) }
    }

    private func generationMatches(_ generation: Int) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return generation == recognitionGeneration
    }

    /// Wartet höchstens `seconds` auf `work`; `true`, wenn es rechtzeitig fertig war.
    /// Ein hängender Schritt läuft im Hintergrund weiter, blockiert den Rückbau aber nicht.
    static func bounded(_ seconds: Double, _ work: @escaping @Sendable () async -> Void) async -> Bool {
        let once = Once()
        return await withCheckedContinuation { (c: CheckedContinuation<Bool, Never>) in
            once.set(c)
            Task { await work(); once.resume(true) }
            Task { try? await Task.sleep(for: .seconds(seconds)); once.resume(false) }
        }
    }

    private static func boundedThrowing(_ seconds: Double, _ work: @escaping @Sendable () async throws -> Void) async -> Bool {
        let once = Once()
        return await withCheckedContinuation { (c: CheckedContinuation<Bool, Never>) in
            once.set(c)
            Task {
                do { try await work(); once.resume(true) }
                catch { once.resume(false) }
            }
            Task { try? await Task.sleep(for: .seconds(seconds)); once.resume(false) }
        }
    }

    private final class Once: @unchecked Sendable {
        private let lock = NSLock()
        private var c: CheckedContinuation<Bool, Never>?
        func set(_ c: CheckedContinuation<Bool, Never>) { lock.lock(); self.c = c; lock.unlock() }
        func resume(_ value: Bool) { lock.lock(); let c = self.c; self.c = nil; lock.unlock(); c?.resume(returning: value) }
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
    private static func consume(_ transcriber: SpeechTranscriber, owner: LiveCapture,
                                language: String?) -> Task<Void, Never> {
        let generation = owner.recognitionGeneration
        let offset = owner.recognitionOffset
        return Task { [weak owner] in
            do {
                for try await result in transcriber.results {
                    guard let capture = owner else { return }
                    let (current, id) = capture.lock.withLock {
                        let current = generation == capture.recognitionGeneration
                        let id = capture.nextAppleID
                        if result.isFinal { capture.nextAppleID += 1 }
                        return (current, id)
                    }
                    guard current else { return }
                    let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
                    var update = Update()
                    if result.isFinal {
                        guard !text.isEmpty else { continue }
                        let start = result.range.start.seconds, end = result.range.end.seconds
                        update.finals = [LiveSegment(
                            id: "a\(id)", start: offset + (start.isFinite ? start : 0),
                            end: offset + (end.isFinite ? end : 0), text: text, language: language
                        )]
                    } else {
                        update.volatile = text
                    }
                    let u = update, sink = capture.onUpdate
                    await MainActor.run {
                        guard capture.generationMatches(generation) else { return }
                        sink?(u)
                    }
                }
            } catch {
                guard let capture = owner else { return }
                capture.lock.withLock {
                    if generation == capture.recognitionGeneration { capture.recognitionFailed = true }
                }
                let sink = capture.onError
                await MainActor.run {
                    guard capture.generationMatches(generation) else { return }
                    sink?(error)
                }
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
