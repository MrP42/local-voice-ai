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
    @Published var storageIssues: [StorageIssue] = []
    @Published var status = "Bereit"
    @Published var recording = false
    @Published var reachable = false
    @Published var fixedAnswer = UserDefaults.standard.object(forKey: "fixedAnswer") as? Bool ?? true {
        didSet { UserDefaults.standard.set(fixedAnswer, forKey: "fixedAnswer") }
    }
    @Published var processing = false
    private var store: DurableStore?
    private var chunks: ChunkInbox?
    private var recorder: AVAudioRecorder?
    private let speaker = AVSpeechSynthesizer()
    private var captureStarted = Date()
    private var feedbackMilliseconds = 0.0
    private var speechStarted = Date()
    private var speakingId: UUID?
    private var currentUtterance: AVSpeechUtterance?
    private var speechAttempts: [ObjectIdentifier: (UUID, Date)] = [:]
    private var inFlight = Set<UUID>()
    private var inFlightReplies = Set<UUID>()
    private var active = false
    private var debugActionsStarted = false
    private var debugReplaySent = false
    private var pendingURL: URL?
    private var captureStartGate = CaptureStartGate()
    private let encoder = JSONEncoder()

    override init() {
        super.init()
        do {
            let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("VoiceOutbox")
            store = try DurableStore(root: root)
            try store?.recoverStaging()
            try store?.migrateIdentities()
            chunks = try ChunkInbox(root: root.appendingPathComponent(".incoming-parts"))
            recoverRecordings()
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
        NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] notification in
            guard let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
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
        else {
            captureStartGate.sceneBecameInactive()
            if recording { stop() }
        }
    }
    private func recordDiagnostic(_ code: String) {
        #if DEBUG
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        try? Data(code.utf8).write(to: documents.appendingPathComponent("last-event.txt"), options: .atomic)
        #endif
    }
    func refresh() {
        do {
            let inventory = try store?.inventory(includeUsage: false)
            entries = inventory?.entries ?? []
            storageIssues = inventory?.issues.filter { !(recording && $0.url.path == pendingURL?.path) } ?? []
        }
        catch { status = "Verlauf konnte nicht gelesen werden" }
    }
    func start() {
        guard !recording, store != nil, let intent = captureStartGate.begin(active: active) else { return }
        speaker.stopSpeaking(at: .immediate)
        let requested = Date()
        AVAudioApplication.requestRecordPermission { allowed in
            Task { @MainActor in
                switch self.captureStartGate.resolve(intent, allowed: allowed, active: self.active) {
                case .start: break
                case .denied:
                    self.status = "Mikrofonzugriff verweigert – in Einstellungen erlauben"
                    self.recordDiagnostic("microphone_denied")
                    return
                case .cancelled:
                    self.status = "Aufnahme nicht gestartet – zum Sprechen erneut tippen"
                    return
                case .stale: return
                }
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
        do {
            guard try AVAudioFile(forReading: url).length > 0 else { throw VoiceError.invalid }
            let start = Date()
            let receipt = try store.recoverRecording(at: url)
            pendingURL = nil
            try? store.update(receipt.sessionId, state: .saved, timing: ("persistence_ms", Date().timeIntervalSince(start) * 1000))
            try? store.update(receipt.sessionId, state: .saved, timing: ("record_feedback_ms", feedbackMilliseconds))
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
            // Even an interrupted recorder can leave a readable, finalized audio container.
            self.saveRecording()
        }
    }
    private func recoverRecordings() {
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
    func recoverStorage() {
        do { try store?.recoverStaging(); try store?.migrateIdentities(); retry() }
        catch { status = "Wiederherstellung nicht abgeschlossen – Originale bleiben erhalten"; refresh() }
    }
    func retry() {
        recoverRecordings()
        refresh()
        guard WCSession.isSupported() else { return }
        reachable = WCSession.default.isReachable
        if WCSession.default.activationState == .activated {
            try? store?.cleanupTransfers(keeping: Set(WCSession.default.outstandingFileTransfers.map { $0.file.fileURL }))
        }
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
        for entry in Entry.pendingDelivery(in: entries) {
            guard !inFlight.contains(entry.id), let store else { continue }
            if reachable && !inFlight.isEmpty { break }
            do {
                let packet = try store.packet(for: entry)
                let data = try encoder.encode(Wire(capture: packet))
                if reachable && inFlight.isEmpty && data.count > 60000 {
                    inFlight.insert(entry.id)
                    sendChunks(try CaptureChunk.split(packet), position: 0, entry: entry, started: Date())
                } else if reachable && inFlight.isEmpty {
                    inFlight.insert(entry.id)
                    let start = Date()
                    WCSession.default.sendMessageData(data, replyHandler: { response in
                        Task { @MainActor in
                            self.inFlight.remove(entry.id)
                            self.receive(response)
                            let state = self.entries.first(where: { $0.id == entry.id })?.state ?? .saved
                            try? self.store?.update(entry.id, state: state, timing: ("transfer_roundtrip_ms", Date().timeIntervalSince(start) * 1000))
                            self.refresh()
                            let advanced = self.entries.first(where: { $0.id == entry.id }).map { $0.state != .saved } ?? false
                            if !response.isEmpty && advanced && self.entries.contains(where: { $0.state == .saved }) { self.retry() }
                        }
                    }, errorHandler: { _ in
                        Task { @MainActor in self.inFlight.remove(entry.id); self.queuePendingFiles() }
                    })
                } else { queueFile(entry, data: data) }
            } catch { status = "Gespeicherte Aufnahme konnte nicht übertragen werden" }
        }
        #else
        if active { Task { await processPending() } }
        for entry in entries where entry.reply != nil && entry.replyAcknowledged != true { sendAnswer(entry) }
        #endif
    }
    #if os(watchOS)
    private func queuePendingFiles() {
        guard let store else { return }
        for entry in Entry.pendingDelivery(in: entries) where !inFlight.contains(entry.id) {
            if let packet = try? store.packet(for: entry), let data = try? encoder.encode(Wire(capture: packet)) {
                queueFile(entry, data: data)
            }
        }
    }
    private func sendChunks(_ parts: [CaptureChunk], position: Int, entry: Entry, started: Date) {
        guard parts.indices.contains(position) else { inFlight.remove(entry.id); return }
        let part = parts[position]
        do {
            let data = try encoder.encode(Wire(chunk: part))
            WCSession.default.sendMessageData(data, replyHandler: { response in
                Task { @MainActor in
                    do {
                        let wire = try JSONDecoder().decode(Wire.self, from: response)
                        guard wire.schemaVersion == 1, wire.sessionId == entry.id else { throw VoiceError.invalid }
                        if wire.kind == "chunkReceipt", wire.receipt?.messageId == part.messageId, position + 1 < parts.count {
                            self.sendChunks(parts, position: position + 1, entry: entry, started: started)
                        } else if wire.kind == "receipt", wire.receipt?.messageId == entry.receipt.messageId {
                            self.inFlight.remove(entry.id)
                            self.receive(response)
                            let state = self.entries.first(where: { $0.id == entry.id })?.state ?? .saved
                            try self.store?.update(entry.id, state: state, timing: ("transfer_roundtrip_ms", Date().timeIntervalSince(started) * 1000))
                            self.refresh()
                            if self.entries.contains(where: { $0.state == .saved }) { self.retry() }
                        } else { throw VoiceError.invalid }
                    } catch {
                        self.inFlight.remove(entry.id)
                        self.queuePendingFiles()
                        self.status = "gespeichert – Verarbeitung folgt"
                    }
                }
            }, errorHandler: { error in
                Task { @MainActor in
                    self.inFlight.remove(entry.id)
                    self.recordDiagnostic("chunk_transport_error_\((error as NSError).code)")
                    self.queuePendingFiles()
                }
            })
        } catch { inFlight.remove(entry.id); status = "Gespeichert – Übertragung erneut versuchen" }
    }
    #endif
    private func queueFile(_ entry: Entry, data: Data) {
        guard WCSession.default.activationState == .activated, let store else { return }
        guard !WCSession.default.outstandingFileTransfers.contains(where: { $0.file.metadata?["sessionId"] as? String == entry.id.uuidString }) else { return }
        do {
            let url = try store.prepareTransfer(for: entry.id, data: data)
            WCSession.default.transferFile(url, metadata: ["sessionId": entry.id.uuidString])
            status = "gespeichert – Verarbeitung folgt"
        } catch { status = "Gespeichert – Übertragung erneut versuchen" }
    }
    @discardableResult
    private func receive(_ data: Data) -> Data {
        do {
            let wire = try JSONDecoder().decode(Wire.self, from: data)
            guard wire.schemaVersion == 1 else { throw VoiceError.version }
            guard ["capture", "receipt", "reply", "replyReceipt", "captureChunk", "chunkReceipt"].contains(wire.kind) else { throw VoiceError.invalid }
            guard let store else { throw VoiceError.persistence }
            #if os(iOS)
            if wire.kind == "captureChunk", let part = wire.payload.chunk, let chunks {
                guard wire.sessionId == part.sessionId, wire.messageId == part.messageId else { throw VoiceError.invalid }
                if let existing = entries.first(where: { $0.id == part.sessionId }) {
                    guard existing.receipt.messageId == part.captureMessageId, existing.digest == part.digest else { throw VoiceError.conflict }
                    return receive(try encoder.encode(Wire(capture: store.packet(for: existing))))
                }
                if let packet = try chunks.receive(part) {
                    let acknowledgement = receive(try encoder.encode(Wire(capture: packet)))
                    if !acknowledgement.isEmpty { try? chunks.removeCompleted(packet.sessionId) }
                    return acknowledgement
                }
                var acknowledgement = Wire(receipt: Receipt(sessionId: part.sessionId, messageId: part.messageId, receiptId: part.messageId))
                acknowledgement.kind = "chunkReceipt"
                return try encoder.encode(acknowledgement)
            }
            if wire.kind == "replyReceipt", let receipt = wire.receipt {
                guard wire.sessionId == receipt.sessionId else { throw VoiceError.invalid }
                try store.acknowledgeReply(receipt)
                refresh()
                return Data()
            }
            if let packet = wire.capture {
                guard wire.kind == "capture", wire.sessionId == packet.sessionId, wire.messageId == packet.messageId else { throw VoiceError.invalid }
                let receipt = try store.accept(packet, replyToWatch: true)
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
                let acceptance = try store.acceptReply(wire)
                refresh()
                status = "Antwort gespeichert"
                if acceptance.isNew && active && !recording { speak(reply, id: id) }
                var response = Wire(receipt: acceptance.receipt)
                response.kind = "replyReceipt"
                return try encoder.encode(response)
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
            currentUtterance = utterance
            speechAttempts[ObjectIdentifier(utterance)] = (id, speechStarted)
            speaker.speak(utterance); status = "Antwort wird gesprochen"
        } catch { status = "Audio nicht verfügbar – Antwort ist gespeichert" }
    }
    func stopPlayback() { speaker.stopSpeaking(at: .immediate); status = "Wiedergabe gestoppt"; recordDiagnostic("playback_stopped") }
    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) {
        Task { @MainActor in
            if let (id, started) = self.speechAttempts[ObjectIdentifier(utterance)] {
                try? self.store?.update(id, state: .answered, timing: ("tts_start_ms", Date().timeIntervalSince(started) * 1000))
                self.refresh()
            }
        }
    }
    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        Task { @MainActor in self.finishSpeech(utterance, cancelled: false) }
    }
    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        Task { @MainActor in self.finishSpeech(utterance, cancelled: true) }
    }
    private func finishSpeech(_ utterance: AVSpeechUtterance, cancelled: Bool) {
        speechAttempts.removeValue(forKey: ObjectIdentifier(utterance))
        guard currentUtterance === utterance else { return }
        currentUtterance = nil; speakingId = nil
        if !recording { status = cancelled ? "Wiedergabe gestoppt – Antwort bleibt gespeichert" : "Bereit" }
    }
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: Error?) { Task { @MainActor in self.retry() } }
    nonisolated func sessionReachabilityDidChange(_ session: WCSession) { Task { @MainActor in self.retry() } }
    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data, replyHandler: @escaping (Data) -> Void) {
        Task { @MainActor in replyHandler(self.receive(messageData)) }
    }
    nonisolated func session(_ session: WCSession, didReceiveMessageData messageData: Data) {
        Task { @MainActor in
            let acknowledgement = self.receive(messageData)
            if !acknowledgement.isEmpty { WCSession.default.transferUserInfo(["wire": acknowledgement]) }
        }
    }
    nonisolated func session(_ session: WCSession, didFinish fileTransfer: WCSessionFileTransfer, error: Error?) {
        Task { @MainActor in
            // Transport completion only permits deleting the copy, never the confirmed original.
            if WCSession.default.activationState == .activated {
                try? self.store?.cleanupTransfers(keeping: Set(WCSession.default.outstandingFileTransfers.map { $0.file.fileURL }))
            }
        }
    }
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
        Task { @MainActor in
            let acknowledgement = self.receive(data)
            if !acknowledgement.isEmpty { WCSession.default.transferUserInfo(["wire": acknowledgement]) }
        }
    }
    #if os(iOS)
    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}
    nonisolated func sessionDidDeactivate(_ session: WCSession) { session.activate() }
    private func sendAnswer(_ entry: Entry) {
        guard entry.replyToWatch == true, entry.reply != nil, entry.replyAcknowledged != true,
              !inFlightReplies.contains(entry.id), let store else { return }
        do {
            let data = try encoder.encode(store.replyEnvelope(for: entry.id))
            if WCSession.default.isReachable {
                inFlightReplies.insert(entry.id)
                WCSession.default.sendMessageData(data, replyHandler: { response in
                    Task { @MainActor in
                        self.inFlightReplies.remove(entry.id)
                        self.receive(response)
                    }
                }, errorHandler: { _ in
                    Task { @MainActor in
                        self.inFlightReplies.remove(entry.id)
                        self.queueAnswer(entry.id, data: data)
                    }
                })
            } else { queueAnswer(entry.id, data: data) }
        } catch { status = "Antwort gespeichert – Zustellung erneut versuchen" }
    }
    private func queueAnswer(_ id: UUID, data: Data) {
        guard WCSession.default.activationState == .activated,
              !WCSession.default.outstandingUserInfoTransfers.contains(where: { $0.userInfo["id"] as? String == id.uuidString }) else { return }
        WCSession.default.transferUserInfo(["id": id.uuidString, "wire": data])
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
        var attempted = Set<UUID>()
        while let entry = (try? store.entries())?.reversed().first(where: { $0.reply == nil && !attempted.contains($0.id) }) {
            attempted.insert(entry.id)
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
                if let answer = entries.first(where: { $0.id == entry.id }) {
                    if answer.replyToWatch == true { sendAnswer(answer) }
                    else if let reply = answer.reply, active, !recording { speak(reply, id: answer.id) }
                }
                status = "Antwort gespeichert"
            } catch {
                try? store.update(entry.id, state: .deferred)
                status = "gespeichert – Verarbeitung folgt (lokales Modell/Assets nicht verfügbar)"
            }
        }
    }
    #endif
}
