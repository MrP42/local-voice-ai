import Foundation
import AVFoundation
import Speech

/// Aufnahme einer Besprechung mit Live-Erkennung (Epic #247, P3).
/// Ein Mikrofonstrom geht in zwei Richtungen: als AAC in eine CAF-Datei (bleibt
/// auch nach einem Absturz lesbar) und in `SpeechAnalyzer`. Die Zeitachse der
/// Ergebnisse ist die geschriebene Audiozeit; Pausen schreiben und analysieren nichts.
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
    private var file: AVAudioFile?
    private var analyzer: SpeechAnalyzer?
    private var transcriber: SpeechTranscriber?
    private var input: AsyncStream<AnalyzerInput>.Continuation?
    private var results: Task<Void, Never>?
    private var toAnalyzer: AVAudioConverter?
    private var toFile: AVAudioConverter?
    private var analyzerFormat: AVAudioFormat?
    private let fileFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)!
    private let lock = NSLock()
    private var writtenFrames: AVAudioFramePosition = 0
    private var nextIndex: Int
    private(set) var running = false

    /// Liefert neue finalisierte Segmente und den vorläufigen Rest (nur Anzeige).
    var onUpdate: (@MainActor (Update) -> Void)?
    var onError: (@MainActor (Error) -> Void)?

    init(fileURL: URL) { self.fileURL = fileURL; self.nextIndex = 0 }

    /// Geschriebene Audiozeit in Sekunden.
    var seconds: Double { lock.lock(); defer { lock.unlock() }; return Double(writtenFrames) / fileFormat.sampleRate }

    static func requestPermission() async -> Bool { await AVAudioApplication.requestRecordPermission() }

    func start() async throws {
        guard await Self.requestPermission() else { throw CaptureError.microphone }
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "de-DE")) else { throw CaptureError.noSpeechAssets }
        let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [.audioTimeRange])
        // Nie stillschweigend Assets laden oder Serverkennung nutzen.
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { throw CaptureError.noSpeechAssets }
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else { throw CaptureError.noFormat }
        self.transcriber = transcriber; analyzerFormat = format

        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .spokenAudio, options: [.allowBluetooth, .defaultToSpeaker])
        try session.setActive(true)

        // AAC in CAF: bleibt bis zum letzten geschriebenen Paket lesbar.
        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 32_000]
        // Eine Datei je Aufnahme; nach einem Absturz wird sie übernommen, nicht fortgeschrieben.
        guard !FileManager.default.fileExists(atPath: fileURL.path) else { throw CocoaError(.fileWriteFileExists) }
        file = try AVAudioFile(forWriting: fileURL, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: fileURL.path)

        let (stream, continuation) = AsyncStream<AnalyzerInput>.makeStream()
        input = continuation
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        self.analyzer = analyzer
        try await analyzer.start(inputSequence: stream)
        results = Task { [weak self] in await self?.consume(transcriber) }

        let micFormat = engine.inputNode.outputFormat(forBus: 0)
        toAnalyzer = AVAudioConverter(from: micFormat, to: format)
        toFile = AVAudioConverter(from: micFormat, to: fileFormat)
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: micFormat) { [weak self] buffer, _ in self?.handle(buffer) }
        engine.prepare()
        try engine.start()
        running = true
    }

    func pause() { engine.pause(); running = false }

    func resume() throws { try engine.start(); running = true }

    /// Beendet Aufnahme und Erkennung; wartet, bis alle Ergebnisse finalisiert sind.
    func stop() async {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        running = false
        input?.finish()
        try? await analyzer?.finalizeAndFinishThroughEndOfInput()
        await results?.value
        file = nil   // schließt die Datei
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func handle(_ buffer: AVAudioPCMBuffer) {
        if let out = convert(buffer, with: toFile, to: fileFormat) {
            do {
                try file?.write(from: out)
                lock.lock(); writtenFrames += AVAudioFramePosition(out.frameLength); lock.unlock()
            } catch { Task { @MainActor in self.onError?(error) } }
        }
        if let format = analyzerFormat, let out = convert(buffer, with: toAnalyzer, to: format) {
            input?.yield(AnalyzerInput(buffer: out))
        }
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

    private func consume(_ transcriber: SpeechTranscriber) async {
        do {
            for try await result in transcriber.results {
                let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
                var update = Update()
                if result.isFinal {
                    guard !text.isEmpty else { continue }
                    let start = result.range.start.seconds, end = result.range.end.seconds
                    // Zeitachse = geschriebene Audiozeit (Pausen liefern keine Puffer).
                    let id = "a\(nextIndex)"; nextIndex += 1
                    update.finals = [LiveSegment(id: id, start: start.isFinite ? start : 0, end: end.isFinite ? end : start, text: text)]
                } else {
                    update.volatile = text
                }
                await MainActor.run { self.onUpdate?(update) }
            }
        } catch {
            await MainActor.run { self.onError?(error) }
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
