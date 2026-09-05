import Foundation
import SwiftUI
import AVFoundation
import WatchConnectivity
#if os(watchOS)
import WatchKit
#endif

typealias Wire = VoiceEnvelope

@MainActor
final class VoiceModel: NSObject, ObservableObject, WCSessionDelegate, AVAudioRecorderDelegate, AVSpeechSynthesizerDelegate {
    @Published var entries: [Entry] = []
    @Published var status = "Bereit"
    @Published var recording = false
    @Published var reachable = false
    @Published var fixedAnswer = true
    @Published var processing = false
    private var store: DurableStore?
    private var recorder: AVAudioRecorder?
    private let speaker = AVSpeechSynthesizer()
    private var captureStarted = Date()
    private var feedbackMilliseconds = 0.0
    private var speechStarted = Date()
    private var speakingId: UUID?
    private var inFlight = Set<UUID>()
    private var active = false
    private var debugActionsStarted = false
    private var debugReplaySent = false
    private var pendingURL: URL?
    private var requestingPermission = false
    private let encoder = JSONEncoder()

    override init() {
        super.init()
        do {
            let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("VoiceOutbox")
            store = try DurableStore(root: root)
            refresh()
        } catch { status = "Speicher nicht verfügbar – Aufnahme gesperrt" }
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--fixture-capture"), let store {
            do {
                let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                let data = try Data(contentsOf: documents.appendingPathComponent("fixture.m4a"))
                let args = ProcessInfo.processInfo.arguments
                let count = args.firstIndex(of: "--fixture-count").flatMap { args.indices.contains($0 + 1) ? Int(args[$0 + 1]) : nil } ?? 1
                for _ in 0..<max(1, min(count, 100)) { _ = try store.accept(Packet.capture(audio: data)) }
                refresh()
            } catch { status = "Testfixture konnte nicht gespeichert werden" }
        }
        if ProcessInfo.processInfo.arguments.contains("--local-models") { fixedAnswer = false }
        #endif
        #if os(iOS) && DEBUG
        Task {
            let capabilities = await LocalProviders.capabilities()
            let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            try? JSONEncoder().encode(capabilities).write(to: documents.appendingPathComponent("capabilities.json"), options: .atomic)
        }
        #endif
        speaker.delegate = self
        if WCSession.isSupported() { WCSession.default.delegate = self; WCSession.default.activate() }
        NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                if self.recording { self.stop() }
                self.speaker.stopSpeaking(at: .immediate)
                self.status = "Audio unterbrochen – gespeicherte Inhalte bleiben erhalten"
            }
        }
    }
    func scene(active: Bool) {
        self.active = active
        if active {
            retry()
            #if DEBUG
            if !debugActionsStarted {
                debugActionsStarted = true
                let arguments = ProcessInfo.processInfo.arguments
                if arguments.contains("--record-probe") { start() }
                if arguments.contains("--interrupt-playback"), let entry = entries.first(where: { $0.reply != nil }), let reply = entry.reply {
                    speak(reply, id: entry.id)
                    Task {
                        try? await Task.sleep(for: .milliseconds(200))
                        self.stopPlayback()
                    }
                }
            }
            #endif
        }
        else if recording { stop() }
    }
    private func recordDiagnostic(_ code: String) {
        #if DEBUG
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? Data(code.utf8).write(to: documents.appendingPathComponent("last-event.txt"), options: .atomic)
        #endif
    }
    func refresh() {
        do { entries = try store?.entries() ?? [] }
        catch { status = "Verlauf konnte nicht gelesen werden" }
    }
    func start() {
        guard !recording, !requestingPermission, store != nil else { return }
        requestingPermission = true
        speaker.stopSpeaking(at: .immediate)
        let requested = Date()
        AVAudioApplication.requestRecordPermission { allowed in
            Task { @MainActor in
                self.requestingPermission = false
                guard allowed else { self.status = "Mikrofonzugriff verweigert – in Einstellungen erlauben"; self.recordDiagnostic("microphone_denied"); return }
                do {
                    let audio = AVAudioSession.sharedInstance()
                    try audio.setCategory(.playAndRecord, mode: .default)
                    #if os(iOS)
                    try audio.setActive(true)
                    #endif
                    let url = self.store!.root.appendingPathComponent(".recording-" + UUID().uuidString + ".m4a")
                    let recorder = try AVAudioRecorder(url: url, settings: [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16000, AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 32000])
                    recorder.delegate = self
                    guard recorder.record(forDuration: 30) else { throw VoiceError.invalid }
                    self.recorder = recorder; self.pendingURL = url
                    self.captureStarted = requested
                    self.feedbackMilliseconds = Date().timeIntervalSince(requested) * 1000
                    self.recording = true; self.status = "Aufnahme läuft · maximal 30 Sekunden"
                    #if os(watchOS)
                    WKInterfaceDevice.current().play(.start)
                    #endif
                } catch { self.status = "Aufnahme konnte nicht starten" }
            }
        }
    }
    func stop() {
        guard recording else { return }
        recording = false
        recorder?.stop()
        saveRecording()
    }
    private func saveRecording() {
        guard let url = pendingURL, let store else { return }
        pendingURL = nil
        do {
            let packet = Packet.capture(audio: try Data(contentsOf: url))
            let start = Date()
            _ = try store.accept(packet)
            try store.update(packet.sessionId, state: .saved, timing: ("persistence_ms", Date().timeIntervalSince(start) * 1000))
            try store.update(packet.sessionId, state: .saved, timing: ("record_feedback_ms", feedbackMilliseconds))
            status = "gespeichert – Verarbeitung folgt"
            try? FileManager.default.removeItem(at: url)
            #if os(watchOS)
            WKInterfaceDevice.current().play(.success)
            #endif
            refresh(); retry()
        } catch {
            status = "Nicht bestätigt – Audiodatei bleibt zur Wiederherstellung erhalten"
        }
        recorder = nil
    }
    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        Task { @MainActor in
            guard self.recorder === recorder else { return }
            self.recording = false
            if flag { self.saveRecording() }
            else { self.status = "Aufnahme unterbrochen – noch nicht bestätigt" }
        }
    }
    func retry() {
        refresh()
        guard WCSession.isSupported() else { return }
        reachable = WCSession.default.isReachable
        #if os(watchOS)
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--replay-capture"), reachable, !debugReplaySent,
           let entry = entries.last, let packet = try? store?.packet(for: entry),
           let data = try? encoder.encode(Wire(capture: packet)) {
            debugReplaySent = true
            WCSession.default.sendMessageData(data, replyHandler: { response in
                Task { @MainActor in
                    self.receive(response)
                    self.recordDiagnostic(response.isEmpty ? "duplicate_failed" : "duplicate_replayed")
                }
            }, errorHandler: { _ in Task { @MainActor in self.recordDiagnostic("duplicate_transport_failed") } })
        }
        #endif
        for entry in entries.sorted(by: { ($0.state == .saved ? 0 : 1) < ($1.state == .saved ? 0 : 1) }) where entry.state != .answered {
            guard inFlight.isEmpty, let store else { continue }
            do {
                let packet = try store.packet(for: entry)
                let data = try encoder.encode(Wire(capture: packet))
                if reachable {
                    inFlight.insert(entry.id)
                    let start = Date()
                    WCSession.default.sendMessageData(data, replyHandler: { response in
                        Task { @MainActor in
                            self.inFlight.remove(entry.id)
                            self.receive(response)
                            let state = self.entries.first(where: { $0.id == entry.id })?.state ?? .saved
                            try? self.store?.update(entry.id, state: state, timing: ("transfer_roundtrip_ms", Date().timeIntervalSince(start) * 1000))
                            self.refresh()
                            if !response.isEmpty && self.entries.contains(where: { $0.state == .saved }) { self.retry() }
                        }
                    }, errorHandler: { _ in
                        Task { @MainActor in self.inFlight.remove(entry.id); self.queueFile(entry, data: data) }
                    })
                } else { queueFile(entry, data: data) }
            } catch { status = "Gespeicherte Aufnahme konnte nicht übertragen werden" }
        }
        #else
        if active { Task { await processPending() } }
        for entry in entries where entry.reply != nil { sendAnswer(entry) }
        #endif
    }
    private func queueFile(_ entry: Entry, data: Data) {
        guard WCSession.default.activationState == .activated, let store else { return }
        guard !WCSession.default.outstandingFileTransfers.contains(where: { $0.file.metadata?["sessionId"] as? String == entry.id.uuidString }) else { return }
        do {
            let url = store.root.appendingPathComponent(".transfer-" + entry.id.uuidString + ".json")
            try data.write(to: url, options: .atomic)
            WCSession.default.transferFile(url, metadata: ["sessionId": entry.id.uuidString])
            status = "gespeichert – Verarbeitung folgt"
        } catch { status = "Gespeichert – Übertragung erneut versuchen" }
    }
    @discardableResult
    private func receive(_ data: Data) -> Data {
        do {
            let wire = try JSONDecoder().decode(Wire.self, from: data)
            guard wire.schemaVersion == 1 else { throw VoiceError.version }
            guard ["capture", "receipt", "reply"].contains(wire.kind) else { throw VoiceError.invalid }
            guard let store else { throw VoiceError.persistence }
            #if os(iOS)
            if let packet = wire.capture {
                guard wire.kind == "capture", wire.sessionId == packet.sessionId, wire.messageId == packet.messageId else { throw VoiceError.invalid }
                let receipt = try store.accept(packet)
                refresh()
                if let existing = entries.first(where: { $0.id == packet.sessionId }), existing.reply != nil { sendAnswer(existing) }
                if active { Task { await processPending() } }
                return try encoder.encode(Wire(receipt: receipt))
            }
            #else
            if let receipt = wire.receipt {
                guard wire.kind == "receipt", wire.sessionId == receipt.sessionId else { throw VoiceError.invalid }
                guard let entry = entries.first(where: { $0.id == receipt.sessionId }), entry.receipt.messageId == receipt.messageId else { throw VoiceError.invalid }
                if entry.state != .answered {
                    try store.update(entry.id, state: .accepted)
                    status = "Auf iPhone gesichert – Verarbeitung folgt"
                } else { status = "Antwort gespeichert" }
            }
            if let id = wire.sessionId, let reply = wire.reply {
                guard wire.kind == "reply", reply.count <= 500 else { throw VoiceError.invalid }
                let alreadyAnswered = entries.first(where: { $0.id == id })?.reply == reply
                try store.update(id, transcript: wire.transcript, reply: reply, state: .answered)
                refresh()
                if !alreadyAnswered && active && !recording { speak(reply, id: id) }
            }
            #endif
            refresh()
            return Data()
        } catch { status = "Nachricht nicht bestätigt – erneute Zustellung erforderlich"; return Data() }
    }
    func speak(_ text: String, id: UUID) {
        guard !recording else { return }
        speaker.stopSpeaking(at: .immediate)
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
            #if os(iOS)
            try AVAudioSession.sharedInstance().setActive(true)
            #endif
            let utterance = AVSpeechUtterance(string: text)
            utterance.voice = AVSpeechSynthesisVoice(language: "de-DE")
            speakingId = id; speechStarted = Date()
            speaker.speak(utterance); status = "Antwort wird gesprochen"
        } catch { status = "Audio nicht verfügbar – Antwort ist gespeichert" }
    }
    func stopPlayback() { speaker.stopSpeaking(at: .immediate); status = "Wiedergabe gestoppt"; recordDiagnostic("playback_stopped") }
    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) {
        Task { @MainActor in
            if let id = self.speakingId { try? self.store?.update(id, state: .answered, timing: ("tts_start_ms", Date().timeIntervalSince(self.speechStarted) * 1000)); self.refresh() }
        }
    }
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: Error?) { Task { @MainActor in self.retry() } }
    nonisolated func sessionReachabilityDidChange(_ session: WCSession) { Task { @MainActor in self.retry() } }
    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data, replyHandler: @escaping (Data) -> Void) {
        Task { @MainActor in replyHandler(self.receive(messageData)) }
    }
    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data) { Task { @MainActor in self.receive(messageData) } }
    nonisolated func session(_ session: WCSession, didReceive file: WCSessionFile) {
        // WC removes its temporary file as soon as this delegate returns.
        guard let data = try? Data(contentsOf: file.fileURL) else { return }
        Task { @MainActor in
            let receipt = self.receive(data)
            if !receipt.isEmpty { WCSession.default.transferUserInfo(["wire": receipt]) }
        }
    }
    nonisolated func session(_ session: WCSession, didReceiveUserInfo userInfo: [String: Any]) {
        guard let data = userInfo["wire"] as? Data else { return }
        Task { @MainActor in self.receive(data) }
    }
    #if os(iOS)
    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}
    nonisolated func sessionDidDeactivate(_ session: WCSession) { session.activate() }
    private func sendAnswer(_ entry: Entry) {
        guard let reply = entry.reply, let data = try? encoder.encode(Wire(sessionId: entry.id, transcript: entry.transcript, reply: reply)) else { return }
        if reachable { WCSession.default.sendMessageData(data, replyHandler: nil, errorHandler: nil) }
        // The persisted entry is retried on activation/reachability; no polling.
        else if WCSession.default.activationState == .activated,
                !WCSession.default.outstandingUserInfoTransfers.contains(where: { $0.userInfo["id"] as? String == entry.id.uuidString }) {
            WCSession.default.transferUserInfo(["id": entry.id.uuidString, "wire": data])
        }
    }
    func prepareLocalSpeech() {
        Task {
            do { try await LocalProviders.installSpeech(); status = "Deutsche Sprachdateien bereit"; retry() }
            catch { status = "Lokale Spracherkennung auf diesem Gerät nicht verfügbar" }
        }
    }
    private func processPending() async {
        guard !processing, active, let store else { return }
        processing = true; defer { processing = false; refresh() }
        for entry in entries where entry.reply == nil {
            guard active else { return }
            do {
                let started = Date()
                if fixedAnswer {
                    try store.update(entry.id, reply: "Deine Aufnahme ist sicher gespeichert.", state: .answered, timing: ("fixed_reply_ms", Date().timeIntervalSince(started) * 1000))
                } else {
                    let transcript = try await LocalProviders.transcribe(store.audioURL(for: entry.id))
                    try store.update(entry.id, transcript: transcript, state: .deferred, timing: ("stt_ms", Date().timeIntervalSince(started) * 1000))
                    guard active else { return }
                    let modelStart = Date()
                    let reply = try await LocalProviders.reply(to: transcript)
                    try store.update(entry.id, reply: reply, state: .answered, timing: ("generation_ms", Date().timeIntervalSince(modelStart) * 1000))
                }
                refresh()
                if let answer = entries.first(where: { $0.id == entry.id }) { sendAnswer(answer) }
                status = "Antwort gespeichert"
            } catch {
                try? store.update(entry.id, state: .deferred)
                status = "gespeichert – Verarbeitung folgt (lokales Modell/Assets nicht verfügbar)"
            }
        }
    }
    #endif
}
