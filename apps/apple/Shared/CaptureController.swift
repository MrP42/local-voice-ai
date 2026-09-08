import Foundation
import AVFoundation
#if os(watchOS)
import WatchKit
#endif

/// Owns microphone intent, recording and durable capture completion independently of delivery.
@MainActor
final class CaptureController: NSObject, AVAudioRecorderDelegate {
    var onChange: ((String, Bool) -> Void)?
    var onSaved: ((UUID) -> Void)?
    var onUnavailable: (() -> Void)?
    var automaticTurns = false
    var sensitivity: MicrophoneSensitivity = .balanced
    var automaticNoiseFloor = true
    var conversationId: UUID?
    private var meterTask: Task<Void, Never>?
    private var store: DurableStore?
    private var recorder: AVAudioRecorder?
    private var captureStartGate = CaptureStartGate()
    private var active = false
    private var captureStarted = Date()
    private var feedbackMilliseconds = 0.0
    private var status = "Bereit" { didSet { onChange?(status, recording) } }
    private(set) var recording = false { didSet { onChange?(status, recording) } }
    private(set) var pendingURL: URL?
    init(store: DurableStore) { self.store = store; super.init() }
    func setActive(_ active: Bool) {
        self.active = active
        if !active { captureStartGate.sceneBecameInactive(); if recording { stop() } }
    }
    private func recordDiagnostic(_ code: String) {
        #if DEBUG
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? Data(code.utf8).write(to: documents.appendingPathComponent("last-event.txt"), options: .atomic)
        #endif
    }
    func cancelPendingStart() { captureStartGate.sceneBecameInactive() }
    func start() {
        guard !recording, store != nil, let intent = captureStartGate.begin(active: active) else { return }
        let requested = Date()
        AVAudioApplication.requestRecordPermission { allowed in
            Task { @MainActor in
                switch self.captureStartGate.resolve(intent, allowed: allowed, active: self.active) {
                case .start: break
                case .denied:
                    self.status = "Mikrofonzugriff verweigert – in Einstellungen erlauben"
                    self.recordDiagnostic("microphone_denied"); self.onUnavailable?()
                    return
                case .cancelled:
                    self.status = "Aufnahme nicht gestartet – zum Sprechen erneut tippen"; self.onUnavailable?()
                    return
                case .stale: return
                }
                do {
                    guard try self.store?.canStartRecording() == true else { throw VoiceError.full }
                    let audio = AVAudioSession.sharedInstance()
                    try audio.setCategory(.playAndRecord, mode: .default)
                    #if os(iOS)
                    try audio.setActive(true)
                    #endif
                    let url = self.store!.root.appendingPathComponent(".recording-" + UUID().uuidString + (self.conversationId.map { "_" + $0.uuidString } ?? "") + ".m4a")
                    let recorder = try AVAudioRecorder(url: url, settings: [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16000, AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 32000])
                    recorder.delegate = self
                    recorder.isMeteringEnabled = self.automaticTurns
                    guard recorder.record(forDuration: 30) else { throw VoiceError.invalid }
                    self.recorder = recorder; self.pendingURL = url
                    self.captureStarted = requested
                    self.feedbackMilliseconds = Date().timeIntervalSince(requested) * 1000
                    self.recording = true; self.status = self.automaticTurns ? "Ich höre zu · Sprechpause beendet den Beitrag" : "Aufnahme läuft · maximal 30 Sekunden"
                    if self.automaticTurns { self.monitorSpeech(recorder) }
                    #if os(watchOS)
                    WKInterfaceDevice.current().play(.start)
                    #endif
                } catch VoiceError.full { self.status = "Speicherbudget voll – vorhandene Aufnahmen bleiben erhalten"; self.onUnavailable?() }
                catch { self.status = "Aufnahme konnte nicht starten"; self.onUnavailable?() }
            }
        }
    }
    private func monitorSpeech(_ recorder: AVAudioRecorder) {
        meterTask?.cancel()
        meterTask = Task { [weak self] in
            var detector = VoiceTurnDetector(sensitivity: self?.sensitivity ?? .balanced, automaticNoiseFloor: self?.automaticNoiseFloor ?? true)
            while !Task.isCancelled {
                do { try await Task.sleep(for: .milliseconds(100)) } catch { return }
                guard let self, self.recording, self.recorder === recorder else { return }
                recorder.updateMeters()
                if let end = detector.observe(powerDB: recorder.averagePower(forChannel: 0), elapsed: recorder.currentTime) {
                    self.stop()
                    if end == .noSpeech { self.status = "Keine Sprache erkannt · Aufnahme wird geprüft" }
                    return
                }
            }
        }
    }
    func stop() {
        guard recording else { return }
        meterTask?.cancel(); meterTask = nil
        recording = false
        let finished = Date()
        recorder?.stop()
        saveRecording(finishedAt: finished)
    }
    private func saveRecording(finishedAt: Date = Date()) {
        guard let url = pendingURL, let store else { return }
        do {
            guard try AVAudioFile(forReading: url).length > 0 else { throw VoiceError.invalid }
            let start = Date()
            let receipt = try store.recoverRecording(at: url)
            pendingURL = nil
            try? store.update(receipt.sessionId, state: .saved, timing: ("capture_end_at_ms", finishedAt.timeIntervalSinceReferenceDate * 1000))
            try? store.update(receipt.sessionId, state: .saved, timing: ("persistence_ms", Date().timeIntervalSince(start) * 1000))
            try? store.update(receipt.sessionId, state: .saved, timing: ("record_feedback_ms", feedbackMilliseconds))
            status = "gespeichert – Verarbeitung folgt"
            try? FileManager.default.removeItem(at: url)
            #if os(watchOS)
            WKInterfaceDevice.current().play(.success)
            #endif
            onSaved?(receipt.sessionId)
        } catch {
            store.invalidateInventory()
            onUnavailable?()
            status = "Nicht bestätigt – Audiodatei bleibt zur Wiederherstellung erhalten"
        }
        recorder = nil
    }
    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        Task { @MainActor in
            guard self.recorder === recorder else { return }
            self.meterTask?.cancel(); self.meterTask = nil
            self.recording = false
            // Even an interrupted recorder can leave a readable, finalized audio container.
            self.saveRecording()
        }
    }
    func recoverRecordings() {
        guard let store, let urls = try? FileManager.default.contentsOfDirectory(at: store.root, includingPropertiesForKeys: nil) else { return }
        for url in urls where url.lastPathComponent.hasPrefix(".recording-") && url.pathExtension == "m4a" {
            if recording && url == pendingURL { continue }
            do {
                guard try AVAudioFile(forReading: url).length > 0 else {
                    status = "Ungesicherte Aufnahme ist nicht lesbar – Datei bleibt erhalten"
                    recordDiagnostic("draft_unreadable")
                    continue
                }
                _ = try store.recoverRecording(at: url)
                if pendingURL == url { pendingURL = nil }
                status = "Aufnahme wiederhergestellt – Verarbeitung folgt"
            } catch { status = "Ungesicherte Aufnahme bleibt zur Wiederherstellung erhalten" }
        }
    }
}
