import Foundation
import SwiftUI
import AVFoundation
#if os(watchOS)
import WatchKit
#endif

@MainActor
final class VoiceModel: NSObject, ObservableObject, AVSpeechSynthesizerDelegate {
    @Published var entries: [Entry] = []
    @Published var storageIssues: [StorageIssue] = []
    @Published var status = "Bereit"
    @Published var recording = false
    @Published var reachable = false
    @Published var fixedAnswer = UserDefaults.standard.object(forKey: "fixedAnswer") as? Bool ?? false {
        didSet {
            UserDefaults.standard.set(fixedAnswer, forKey: "fixedAnswer")
            #if os(iOS)
            jobs?.fixedAnswer = fixedAnswer
            #endif
        }
    }
    @Published var processing = false
    #if os(iOS)
    private var jobs: JobProcessor?
    @Published var installedModels: [InstalledModel] = []
    @Published var providerDescription = "Verfügbarkeit wird geprüft"
    @Published var modelMessage = ""
    @Published var installingModel = false
    @Published var sttModel = UserDefaults.standard.string(forKey: "sttModel") ?? "ggml-base.bin" {
        didSet { UserDefaults.standard.set(sttModel, forKey: "sttModel") }
    }
    #endif
    private var store: DurableStore?
    private var transport: VoiceTransport?
    private var capture: CaptureController?
    private let speaker = AVSpeechSynthesizer()
    private var speechStarted = Date()
    private var speakingId: UUID?
    private var currentUtterance: AVSpeechUtterance?
    private var speechAttempts: [ObjectIdentifier: (UUID, Date)] = [:]
    private var active = false
    private var debugActionsStarted = false

    override init() {
        super.init()
        do {
            let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("VoiceOutbox")
            store = try DurableStore(root: root)
            try store?.recoverStaging()
            try store?.migrateIdentities()
            transport = try VoiceTransport(store: store!)
            capture = CaptureController(store: store!)
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
        #if os(iOS)
        if let store {
            let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            let marker = documents.appendingPathComponent("recovery-ui-probe.txt")
            if ProcessInfo.processInfo.arguments.contains("--corrupt-history-probe") {
                do {
                    let receipt = try store.accept(.capture(audio: Data(contentsOf: documents.appendingPathComponent("fixture.m4a"))))
                    let folder = store.root.appendingPathComponent(receipt.sessionId.uuidString)
                    try FileManager.default.copyItem(at: folder.appendingPathComponent("entry.json"), to: folder.appendingPathComponent(".debug-entry-backup"))
                    try Data(receipt.sessionId.uuidString.utf8).write(to: marker, options: .atomic)
                    try Data("synthetic corrupted metadata".utf8).write(to: folder.appendingPathComponent("entry.json"), options: .atomic)
                } catch { status = "Recovery-Testfixture konnte nicht angelegt werden" }
            }
            if ProcessInfo.processInfo.arguments.contains("--restore-history-probe"),
               let text = try? String(contentsOf: marker, encoding: .utf8), let id = UUID(uuidString: text) {
                let folder = store.root.appendingPathComponent(id.uuidString)
                if let data = try? Data(contentsOf: folder.appendingPathComponent(".debug-entry-backup")) {
                    try? data.write(to: folder.appendingPathComponent("entry.json"), options: .atomic)
                }
            }
            refresh()
        }
        #endif
        if ProcessInfo.processInfo.arguments.contains("--local-models") { fixedAnswer = false }
        #if os(iOS)
        if ProcessInfo.processInfo.arguments.contains("--stt-small") { sttModel = "ggml-small.bin" }
        if ProcessInfo.processInfo.arguments.contains("--stt-base") { sttModel = "ggml-base.bin" }
        let arguments = ProcessInfo.processInfo.arguments
        if let index = arguments.firstIndex(of: "--retry-job"), arguments.indices.contains(index + 1),
           let id = UUID(uuidString: arguments[index + 1]) { try? store?.retryJob(id) }
        #endif
        #endif
        #if os(iOS) && DEBUG
        Task {
            let capabilities = await LocalProviders.capabilities()
            let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            try? JSONEncoder().encode(capabilities).write(to: documents.appendingPathComponent("capabilities.json"), options: .atomic)
        }
        #endif
        #if os(iOS)
        if let store {
            do { try store.recoverInterruptedJobs() } catch { status = "Auftragsstatus konnte nicht wiederhergestellt werden" }
            jobs = JobProcessor(store: store, transcribe: { try await LocalProviders.transcribe($0) }, reply: { try await LocalProviders.reply(to: $0) })
            jobs?.fixedAnswer = fixedAnswer
            jobs?.onChange = { [weak self] id in
                guard let self else { return }
                self.processing = self.jobs?.isProcessing == true
                self.status = self.jobs?.message ?? "Bereit"
                self.refresh()
                if let id, let entry = self.entries.first(where: { $0.id == id }), let reply = entry.reply {
                    if entry.replyToWatch == true { self.transport?.sendAnswer(entry) }
                    else if self.active && !self.recording { self.speak(reply, id: id) }
                }
            }
        }
        #endif
        capture?.onChange = { [weak self] status, recording in self?.status = status; self?.recording = recording }
        capture?.onSaved = { [weak self] in self?.retry() }
        speaker.delegate = self
        transport?.onChange = { [weak self] in self?.refresh() }
        transport?.onStatus = { [weak self] in self?.status = $0 }
        transport?.onReachability = { [weak self] in self?.reachable = $0 }
        transport?.onReply = { [weak self] text, id in
            guard let self, self.active, !self.recording else { return }
            self.speak(text, id: id)
        }
        #if os(iOS)
        transport?.onCapture = { [weak self] in self?.jobs?.start() }
        #endif
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
        capture?.setActive(active)
        #if os(iOS)
        jobs?.setActive(active)
        #endif
        if active {
            retry()
            #if DEBUG
            if !debugActionsStarted {
                debugActionsStarted = true
                let arguments = ProcessInfo.processInfo.arguments
                if arguments.contains("--record-probe") { start() }
                #if os(iOS)
                if arguments.contains("--cancel-inference-probe") { cancellationProbe(generation: arguments.contains("--cancel-generating")) }
                if arguments.contains("--model-import-probe") { modelImportProbe() }
                #endif
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
            storageIssues = inventory?.issues.filter { !(recording && $0.url.path == capture?.pendingURL?.path) } ?? []
        }
        catch { status = "Verlauf konnte nicht gelesen werden" }
    }
    func start() { speaker.stopSpeaking(at: .immediate); capture?.start() }
    func stop() { capture?.stop() }
    private func recoverRecordings() { capture?.recoverRecordings() }
    func recoverStorage() {
        do { try store?.recoverStaging(); try store?.migrateIdentities(); retry() }
        catch { status = "Wiederherstellung nicht abgeschlossen – Originale bleiben erhalten"; refresh() }
    }
    func retry() {
        recoverRecordings(); refresh(); transport?.retry()
        #if os(iOS)
        if active { jobs?.start() }
        #endif
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
                if let entry = self.entries.first(where: { $0.id == id }), entry.timings["tts_e2e_ms"] == nil,
                   let origin = entry.timings["capture_end_at_ms"] ?? entry.timings["capture_saved_at_ms"] {
                    let elapsed = Date().timeIntervalSinceReferenceDate * 1000 - origin
                    if elapsed >= 0 { try? self.store?.update(id, state: .answered, timing: ("tts_e2e_ms", elapsed)) }
                }
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
    #if os(iOS)
    #if DEBUG
    private func cancellationProbe(generation: Bool) {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let started = Date().timeIntervalSince1970
        let phase = generation ? "generating" : "transcribing"
        Task {
            func marker() -> [String: Any] {
                guard let data = try? Data(contentsOf: documents.appendingPathComponent("native-phase.json")),
                      let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
                return value
            }
            for _ in 0..<600 {
                let value = marker()
                if value["phase"] as? String == phase, let at = value["at"] as? Double, at > started {
                    try? await Task.sleep(for: .milliseconds(200))
                    guard marker()["at"] as? Double == at, let id = jobs?.currentId else { continue }
                    let requested = Date().timeIntervalSince1970
                    jobs?.cancelCurrent()
                    for _ in 0..<200 {
                        try? await Task.sleep(for: .milliseconds(50))
                        if let entry = entries.first(where: { $0.id == id }), entry.job?.phase == .cancelled {
                            let stopped = marker()["at"] as? Double ?? 0
                            let report: [String: Any] = ["id": id.uuidString, "phase": phase, "cancelled": true,
                                "nativeStoppedAfterRequest": stopped >= requested && marker()["phase"] as? String == "idle",
                                "nativeStartedAt": at, "nativeStoppedAt": stopped, "cancelRequestedAt": requested,
                                "cancellationMilliseconds": (Date().timeIntervalSince1970 - requested) * 1000,
                                "hasReply": entry.reply != nil, "hasTranscript": entry.transcript != nil]
                            if let data = try? JSONSerialization.data(withJSONObject: report, options: .prettyPrinted) {
                                try? data.write(to: documents.appendingPathComponent("cancellation-probe.json"), options: .atomic)
                            }
                            return
                        }
                    }
                    break
                }
                try? await Task.sleep(for: .milliseconds(100))
            }
            recordDiagnostic("cancellation_probe_failed")
        }
    }
    private func modelImportProbe() {
        Task {
            let source = ModelLibrary.folder.appendingPathComponent("ggml-base.bin")
            do { _ = try await ModelLibrary.shared.install(from: source); recordDiagnostic("model_import_verified") }
            catch { recordDiagnostic("model_import_failed") }
        }
    }
    #endif
    func refreshModels() async {
        installedModels = await ModelLibrary.shared.inventory()
        let availability = await LocalProviders.capabilities()
        let speech = availability["speechTranscriberAvailable"] == "true" ? "Apple-Spracherkennung verfügbar" : "Apple-Spracherkennung hier nicht verfügbar"
        let ai = availability["foundationModels"] == "available" ? "Apple-Antwortmodell verfügbar" : "Apple-Antwortmodell hier nicht bereit"
        providerDescription = speech + ". " + ai + ". Für den CPU-Pfad werden ein Whisper-Modell und Qwen benötigt."
    }
    func installModel(_ url: URL) {
        guard !installingModel else { return }
        installingModel = true
        Task {
            defer { installingModel = false }
            do {
                let label = try await ModelLibrary.shared.install(from: url)
                modelMessage = label + " geprüft und installiert"
                await refreshModels()
            } catch { modelMessage = "Installation nicht bestätigt – Modellbestand bitte erneut prüfen" }
        }
    }
    func prepareLocalSpeech() {
        Task {
            do { try await LocalProviders.installSpeech(); status = "Deutsche Sprachdateien bereit"; retry() }
            catch { status = "Lokale Spracherkennung auf diesem Gerät nicht verfügbar" }
        }
    }
    func cancelProcessing() { jobs?.cancelCurrent() }
    func retryProcessing(_ id: UUID) {
        do { try store?.retryJob(id); jobs?.start(); refresh() }
        catch { status = "Aufnahme bleibt gespeichert – Wiederholung nicht gestartet" }
    }
    #endif
}
