import Foundation
import SwiftUI
import AVFoundation
#if os(watchOS)
import WatchKit
#endif

private final class AudioInterruptionObservation {
    private let token: NSObjectProtocol
    init(_ handler: @escaping @Sendable (Notification) -> Void) {
        token = NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main, using: handler)
    }
    deinit { NotificationCenter.default.removeObserver(token) }
}

@MainActor
final class VoiceModel: NSObject, ObservableObject, AVSpeechSynthesizerDelegate, AVAudioPlayerDelegate {
    #if os(iOS)
    static let shared = VoiceModel()
    private var backgroundRuntime: PhoneProcessingRuntime?
    #if DEBUG && targetEnvironment(simulator)
    private var backgroundProbeStarted = false
    private var conversationProbeRecorded = false
    #endif
    func registerBackgroundProcessing() {
        do { try backgroundRuntime?.prepareProtectedFiles() }
        catch { recordDiagnostic("background_file_protection_pending") }
        backgroundRuntime?.register()
    }
    #endif
    @Published var entries: [Entry] = []
    @Published var storageIssues: [StorageIssue] = []
    @Published var status = "Bereit"
    @Published var recording = false
    @Published var reachable = false
    @Published var availableVoices: [SpeechVoiceOption] = []
    @Published var selectedVoiceID = UserDefaults.standard.string(forKey: "selectedVoiceID") ?? "" {
        didSet { UserDefaults.standard.set(selectedVoiceID, forKey: "selectedVoiceID") }
    }
    @Published private(set) var previewingVoiceID: String?
    @Published private(set) var playingRecordingId: UUID?
    @Published private(set) var recordingPlaybackPaused = false
    @Published private(set) var recordingPlaybackTime: TimeInterval = 0
    @Published private(set) var recordingPlaybackDuration: TimeInterval = 0
    @Published private(set) var originalPlaybackIssueId: UUID?
    @Published private(set) var originalPlaybackError: String?
    private var originalPlayer: AVAudioPlayer?
    private var originalProgressTask: Task<Void, Never>?
    @Published var autoPlayReplies = UserDefaults.standard.object(forKey: "autoPlayReplies") as? Bool ?? true {
        didSet { UserDefaults.standard.set(autoPlayReplies, forKey: "autoPlayReplies") }
    }
    @Published var handsFreeEnabled = UserDefaults.standard.bool(forKey: "handsFreeEnabled") {
        didSet { UserDefaults.standard.set(handsFreeEnabled, forKey: "handsFreeEnabled"); if !handsFreeEnabled { endConversation() } }
    }
    @Published var microphoneSensitivity = MicrophoneSensitivity(rawValue: UserDefaults.standard.string(forKey: "microphoneSensitivity") ?? "") ?? .balanced {
        didSet { UserDefaults.standard.set(microphoneSensitivity.rawValue, forKey: "microphoneSensitivity"); capture?.sensitivity = microphoneSensitivity }
    }
    @Published var automaticNoiseFloor = UserDefaults.standard.object(forKey: "automaticNoiseFloor") as? Bool ?? true {
        didSet { UserDefaults.standard.set(automaticNoiseFloor, forKey: "automaticNoiseFloor"); capture?.automaticNoiseFloor = automaticNoiseFloor }
    }
    @Published private(set) var conversationRunning = false
    @Published private(set) var conversationId = UserDefaults.standard.string(forKey: "conversationId").flatMap(UUID.init(uuidString:))
    private var awaitingConversationReply: UUID?
    private var resumeConversationTask: Task<Void, Never>?

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
    @Published var preparingSpeech = false
    @Published var speechModelMessage = ""
    @Published var sttModel = UserDefaults.standard.string(forKey: "sttModel") ?? "ggml-base.bin" {
        didSet { UserDefaults.standard.set(sttModel, forKey: "sttModel") }
    }
    #endif
    private var store: DurableStore?
    private var transport: VoiceTransport?
    private var capture: CaptureController?
    private let speaker = AVSpeechSynthesizer()
    private var speechStarted = Date()
    private var lastSpeechStart = Date.distantPast
    private var speakingId: UUID?
    private var currentUtterance: AVSpeechUtterance?
    private var speechAttempts: [ObjectIdentifier: (UUID, Date, AVSpeechUtterance)] = [:]
    private var interruptionObservation: AudioInterruptionObservation?
    private var active = false
    private var debugActionsStarted = false
    #if DEBUG
    private var setupFailureInjected = false
    #endif

    override init() {
        super.init()
        do {
            let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("VoiceOutbox")
            #if DEBUG && targetEnvironment(simulator)
            if ProcessInfo.processInfo.arguments.contains("--original-playback-probe") {
                let testStore = try DurableStore(root: root.deletingLastPathComponent().appendingPathComponent("PlaybackUITest-" + UUID().uuidString))
                let url = testStore.root.appendingPathComponent("fixture.m4a")
                do {
                    let format = AVAudioFormat(standardFormatWithSampleRate: 16000, channels: 1)!
                    let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 320000)!
                    buffer.frameLength = buffer.frameCapacity
                    buffer.floatChannelData![0].initialize(repeating: 0, count: Int(buffer.frameLength))
                    let file = try AVAudioFile(forWriting: url, settings: [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 16000, AVNumberOfChannelsKey: 1])
                    try file.write(from: buffer)
                }
                let receipt = try testStore.accept(.capture(audio: Data(contentsOf: url)))
                try testStore.update(receipt.sessionId, transcript: "Synthetische Originalaufnahme", reply: "Testantwort.", state: .answered)
                store = testStore
            } else if ProcessInfo.processInfo.arguments.contains("--conversation-no-speech-probe") || ProcessInfo.processInfo.arguments.contains("--conversation-cycle-probe") {
                store = try DurableStore(root: root.deletingLastPathComponent().appendingPathComponent("ConversationUITest-" + UUID().uuidString))
            } else if ProcessInfo.processInfo.arguments.contains("--background-processing-probe") {
                store = try DurableStore(root: root.deletingLastPathComponent().appendingPathComponent("BackgroundUITest-" + UUID().uuidString))
            } else if ProcessInfo.processInfo.arguments.contains("--transparency-ui-probe") {
                store = try DurableStore(root: root.appendingPathComponent("UITest-Transparency"))
                if try store?.entries().isEmpty == true, let store {
                    let receipt = try store.accept(.capture(audio: Data([1])))
                    try store.update(receipt.sessionId, transcript: "Darstellung prüfen", reply: "## Ergebnis\n**Wichtig:** Formatierter Text.\n- Ein nächster Schritt", state: .answered, event: ProcessingEvent(operation: "Antwort", model: "UI-Testmodell", completedAt: Date(), durationMS: 123, isAI: true))
                }
            } else { store = try DurableStore(root: root) }
            #else
            store = try DurableStore(root: root)
            #endif
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
                    store.invalidateInventory()
                } catch { status = "Recovery-Testfixture konnte nicht angelegt werden" }
            }
            if ProcessInfo.processInfo.arguments.contains("--restore-history-probe"),
               let text = try? String(contentsOf: marker, encoding: .utf8), let id = UUID(uuidString: text) {
                let folder = store.root.appendingPathComponent(id.uuidString)
                if let data = try? Data(contentsOf: folder.appendingPathComponent(".debug-entry-backup")) {
                    try? data.write(to: folder.appendingPathComponent("entry.json"), options: .atomic)
                    store.invalidateInventory()
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
        do { try configureComponents(); try recoverStorageState() }
        catch { status = "Speicherprüfung erforderlich – Wiederherstellung erneut versuchen"; refresh() }
        interruptionObservation = AudioInterruptionObservation { [weak self] notification in
            guard let raw = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  AVAudioSession.InterruptionType(rawValue: raw) == .began else { return }
            Task { @MainActor in
                guard let self else { return }
                self.endConversation()
                self.stopOriginalPlayback()
                if self.recording { self.stop() }
                self.speaker.stopSpeaking(at: .immediate)
                self.status = "Audio unterbrochen – gespeicherte Inhalte bleiben erhalten"
                self.recordDiagnostic("audio_interrupted")
            }
        }
    }
    private func configureComponents() throws {
        if store == nil {
            let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("VoiceOutbox")
            store = try DurableStore(root: root)
        }
        guard let store else { throw VoiceError.persistence }
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--setup-failure-probe"), !setupFailureInjected {
            setupFailureInjected = true; throw VoiceError.persistence
        }
        #endif
        if transport == nil { transport = try VoiceTransport(store: store) }
        if capture == nil { capture = CaptureController(store: store) }
        #if os(iOS)
        ModelDownloads.shared.onInstalled = { [weak self] in Task { await self?.refreshModels() } }
        if jobs == nil {
            jobs = JobProcessor(store: store, annotatedTranscribe: { try await LocalProviders.annotatedTranscribe($0) }, conversationalReply: { try await LocalProviders.annotatedReply(to: $0, history: $1) })
            #if DEBUG && targetEnvironment(simulator)
            if ProcessInfo.processInfo.arguments.contains("--conversation-no-speech-probe") {
                jobs = JobProcessor(store: store, transcribe: { _ in " " }, reply: { _ in "Unexpected generation" })
            } else if ProcessInfo.processInfo.arguments.contains("--conversation-cycle-probe") {
                jobs = JobProcessor(store: store, annotatedTranscribe: { _ in ProcessingOutput(text: "Testfrage", model: "Test-STT") }, annotatedReply: { _ in ProcessingOutput(text: "Testantwort.", model: "Test-LLM") })
            }
            if ProcessInfo.processInfo.arguments.contains("--background-processing-probe") {
                jobs = JobProcessor(store: store, annotatedTranscribe: { _ in
                    try await Task.sleep(for: .seconds(2))
                    return ProcessingOutput(text: "Hintergrund-Testaufnahme", model: "Test-STT")
                }, annotatedReply: { _ in
                    try await Task.sleep(for: .seconds(2))
                    let background = await MainActor.run { UIApplication.shared.applicationState == .background }
                    return ProcessingOutput(text: background ? "Im Hintergrund fertiggestellt" : "Erst im Vordergrund fertiggestellt", model: "Test-LLM")
                })
            }
            #endif
            jobs?.fixedAnswer = fixedAnswer
            jobs?.onChange = { [weak self] id in
                guard let self else { return }
                self.processing = self.jobs?.isProcessing == true
                self.status = self.jobs?.message ?? "Bereit"
                self.refresh()
                if let id, let entry = self.entries.first(where: { $0.id == id }), let reply = entry.reply {
                    if UIApplication.shared.applicationState == .background, entry.timings["background_reply_completed_at_ms"] == nil {
                        try? self.store?.update(entry.id, state: .answered, timing: ("background_reply_completed_at_ms", Date().timeIntervalSince1970 * 1000))
                    }
                    if entry.replyToWatch == true { self.transport?.sendAnswer(entry) }
                    else if self.active && !self.recording { self.handleAnswer(reply, id: id) }
                }
                self.backgroundRuntime?.processingChanged()
            }
            if let jobs {
                backgroundRuntime = PhoneProcessingRuntime(worker: jobs, store: store)
                backgroundRuntime?.onDiagnostic = { [weak self] in self?.recordDiagnostic($0) }
            }
        }
        #endif
        capture?.onChange = { [weak self] status, recording in
            self?.status = status; self?.recording = recording
            #if DEBUG && os(iOS) && targetEnvironment(simulator)
            if let self, recording, !self.conversationProbeRecorded, (ProcessInfo.processInfo.arguments.contains("--conversation-cycle-probe") || ProcessInfo.processInfo.arguments.contains("--conversation-no-speech-probe")) {
                self.conversationProbeRecorded = true
                Task { [weak self] in
                    try? await Task.sleep(for: .seconds(1))
                    guard let self, self.active, self.conversationRunning, self.recording else { return }
                    // Simulate the first speech-end event; real persistence, playback and rearming remain in use.
                    self.capture?.stop()
                }
            }
            #endif
        }
        capture?.onSaved = { [weak self] id in
            guard let self else { return }
            if self.conversationRunning { self.awaitingConversationReply = id }
            self.retry()
        }
        capture?.onUnavailable = { [weak self] in
            self?.conversationRunning = false; self?.resumeConversationTask?.cancel()
        }
        speaker.delegate = self
        transport?.onChange = { [weak self] in self?.refresh() }
        transport?.onStatus = { [weak self] in self?.status = $0 }
        transport?.onReachability = { [weak self] in self?.reachable = $0 }
        transport?.onReply = { [weak self] text, id in
            guard let self, self.active, !self.recording else { return }
            self.handleAnswer(text, id: id)
        }
        #if os(iOS)
        transport?.onCapture = { [weak self] in self?.backgroundRuntime?.receivedCapture() }
        #endif
    }
    private func recoverStorageState() throws {
        try store?.recoverStaging()
        try store?.migrateIdentities()
        #if os(iOS)
        if jobs?.isProcessing != true { try store?.recoverInterruptedJobs() }
        #endif
        recoverRecordings(); refresh()
    }
    func scene(active: Bool) {
        self.active = active
        if !active { endConversation(); stopOriginalPlayback() }
        if active { store?.invalidateInventory() }
        capture?.setActive(active)
        #if os(iOS)
        backgroundRuntime?.setForeground(active)
        #if DEBUG && targetEnvironment(simulator)
        if !active, debugActionsStarted, !backgroundProbeStarted, ProcessInfo.processInfo.arguments.contains("--background-processing-probe") {
            backgroundProbeStarted = true
            Task {
                try? await Task.sleep(for: .seconds(1))
                do {
                    _ = try store?.accept(.capture(audio: Data([1, 2, 3])), replyToWatch: true)
                    backgroundRuntime?.receivedCapture()
                } catch { status = "Hintergrundprobe konnte nicht gespeichert werden" }
            }
        }
        #endif
        #endif
        if active {
            refreshVoices()
            #if os(iOS)
            ModelDownloads.shared.resumeVerification()
            #endif
            retry()
            #if DEBUG
            if !debugActionsStarted {
                debugActionsStarted = true
                let arguments = ProcessInfo.processInfo.arguments
                if arguments.contains("--record-probe") { start() }
                #if os(iOS)
                if let index = arguments.firstIndex(of: "--download-model-probe"), arguments.indices.contains(index + 1),
                   let descriptor = LocalModel.all.first(where: { $0.id == arguments[index + 1] }) { downloadModel(descriptor) }
                if arguments.contains("--cancel-inference-probe") { cancellationProbe(generation: arguments.contains("--cancel-generating")) }
                if arguments.contains("--model-import-probe") { modelImportProbe(invalid: arguments.contains("--invalid-model")) }
                #endif
                if arguments.contains("--interrupt-playback") || arguments.contains("--interruption-notification"), let entry = entries.first(where: { $0.reply != nil }), let reply = entry.reply {
                    let requested = Date()
                    speak(reply, id: entry.id)
                    Task {
                        if arguments.contains("--interruption-notification") {
                            for _ in 0..<100 {
                                if self.lastSpeechStart > requested { break }
                                try? await Task.sleep(for: .milliseconds(100))
                            }
                            guard self.lastSpeechStart > requested else { self.recordDiagnostic("interruption_probe_failed"); return }
                            NotificationCenter.default.post(name: AVAudioSession.interruptionNotification, object: AVAudioSession.sharedInstance(),
                                userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue])
                        } else {
                            try? await Task.sleep(for: .milliseconds(200)); self.stopPlayback()
                        }
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
    func recordingURL(_ id: UUID) -> URL? {
        guard let url = store?.audioURL(for: id), FileManager.default.fileExists(atPath: url.path) else { return nil }
        return url
    }
    func toggleOriginalPlayback(_ id: UUID) {
        previewingVoiceID = nil
        if playingRecordingId == id, let player = originalPlayer {
            if player.isPlaying {
                player.pause(); originalProgressTask?.cancel()
                recordingPlaybackTime = player.currentTime; recordingPlaybackPaused = true
                status = "Originalaufnahme pausiert"
            } else if player.play() {
                recordingPlaybackPaused = false; monitorOriginalPlayback(player)
                status = "Originalaufnahme wird abgespielt"
            } else { originalPlaybackFailed(id, message: "Audio nicht verfügbar – Original bleibt gespeichert") }
            return
        }
        endConversation()
        currentUtterance = nil; speakingId = nil
        speaker.stopSpeaking(at: .immediate)
        stopOriginalPlayback()
        guard let url = recordingURL(id) else { originalPlaybackFailed(id, message: "Originalaufnahme auf diesem Gerät nicht verfügbar"); return }
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
            #if os(iOS)
            try AVAudioSession.sharedInstance().setActive(true)
            #endif
            let player = try AVAudioPlayer(contentsOf: url)
            player.delegate = self
            guard player.play() else { throw VoiceError.invalid }
            originalPlayer = player; playingRecordingId = id
            recordingPlaybackDuration = player.duration; recordingPlaybackTime = player.currentTime
            recordingPlaybackPaused = false; monitorOriginalPlayback(player)
            status = "Originalaufnahme wird abgespielt"
        } catch { originalPlaybackFailed(id, message: "Audio nicht lesbar – Original bleibt gespeichert") }
    }
    private func monitorOriginalPlayback(_ player: AVAudioPlayer) {
        originalProgressTask?.cancel()
        originalProgressTask = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
                guard let self, self.originalPlayer === player else { return }
                self.recordingPlaybackTime = player.currentTime
            }
        }
    }
    private func stopOriginalPlayback() {
        originalProgressTask?.cancel(); originalProgressTask = nil
        originalPlayer?.stop(); originalPlayer = nil; playingRecordingId = nil
        recordingPlaybackPaused = false; recordingPlaybackTime = 0; recordingPlaybackDuration = 0
        originalPlaybackIssueId = nil; originalPlaybackError = nil
    }
    private func originalPlaybackFailed(_ id: UUID, message: String) {
        stopOriginalPlayback(); originalPlaybackIssueId = id; originalPlaybackError = message; status = message
    }
    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor in
            guard self.originalPlayer === player else { return }
            if !flag, let id = self.playingRecordingId { self.originalPlaybackFailed(id, message: "Wiedergabe unterbrochen – Original bleibt gespeichert") }
            else { self.stopOriginalPlayback(); self.status = "Bereit" }
        }
    }
    nonisolated func audioPlayerDecodeErrorDidOccur(_ player: AVAudioPlayer, error: Error?) {
        Task { @MainActor in
            guard self.originalPlayer === player else { return }
            if let id = self.playingRecordingId { self.originalPlaybackFailed(id, message: "Audio nicht lesbar – Original bleibt gespeichert") }
        }
    }
    func start() {
        previewingVoiceID = nil
        stopOriginalPlayback()
        currentUtterance = nil; speakingId = nil
        speaker.stopSpeaking(at: .immediate)
        if handsFreeEnabled {
            if conversationId == nil { newConversation() }
            conversationRunning = true
        }
        capture?.conversationId = handsFreeEnabled ? conversationId : nil
        capture?.automaticTurns = handsFreeEnabled
        capture?.sensitivity = microphoneSensitivity
        capture?.automaticNoiseFloor = automaticNoiseFloor
        capture?.start()
    }
    func newConversation() {
        endConversation()
        speaker.stopSpeaking(at: .immediate)
        conversationId = UUID()
        UserDefaults.standard.set(conversationId?.uuidString, forKey: "conversationId")
    }
    func endConversation() {
        conversationRunning = false; awaitingConversationReply = nil
        resumeConversationTask?.cancel(); resumeConversationTask = nil
        capture?.cancelPendingStart()
        if recording { capture?.stop() }
    }
    func stopConversation() {
        endConversation()
        speaker.stopSpeaking(at: .immediate)
        status = "Gespräch beendet · Verlauf gespeichert"
    }
    private func handleAnswer(_ text: String, id: UUID) {
        guard originalPlayer == nil else { return }
        if let entry = entries.first(where: { $0.id == id }), ConversationOutcome.isNoSpeech(entry) {
            status = ConversationOutcome.noSpeechMessage
            continueConversation(after: id)
        }
        else if autoPlayReplies { speak(text, id: id) }
        else { continueConversation(after: id) }
    }
    private func continueConversation(after id: UUID) {
        guard active, conversationRunning, handsFreeEnabled, awaitingConversationReply == id,
              entries.first(where: { $0.id == id })?.conversationId == conversationId else { return }
        awaitingConversationReply = nil
        resumeConversationTask?.cancel()
        resumeConversationTask = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(350)) } catch { return }
            guard let self, self.active, self.conversationRunning, !self.recording else { return }
            self.capture?.start()
        }
    }
    func stop() { capture?.stop(); refresh() }
    private func recoverRecordings() { capture?.recoverRecordings() }
    func recoverStorage() {
        store?.invalidateInventory()
        do {
            try configureComponents(); try recoverStorageState()
            capture?.setActive(active)
            #if os(iOS)
            backgroundRuntime?.setForeground(active)
            #endif
            retry()
        }
        catch { status = "Wiederherstellung nicht abgeschlossen – Originale bleiben erhalten"; refresh() }
    }
    func retry(forceReload: Bool = false) {
        if forceReload && (capture == nil || transport == nil) { recoverStorage(); return }
        if forceReload { store?.invalidateInventory() }
        recoverRecordings(); refresh(); transport?.retry()
        #if os(iOS)
        if active { jobs?.start() }
        #endif
    }
    func speak(_ text: String, id: UUID) {
        guard !recording else { return }
        previewingVoiceID = nil
        stopOriginalPlayback()
        speaker.stopSpeaking(at: .immediate)
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
            #if os(iOS)
            try AVAudioSession.sharedInstance().setActive(true)
            #endif
            let spoken = (try? AttributedString(markdown: text)).map { String($0.characters) } ?? text
            let utterance = AVSpeechUtterance(string: spoken)
            utterance.voice = AVSpeechSynthesisVoice(identifier: selectedVoiceID) ?? AVSpeechSynthesisVoice(language: "de-DE")
            speakingId = id; speechStarted = Date()
            currentUtterance = utterance
            speechAttempts[ObjectIdentifier(utterance)] = (id, speechStarted, utterance)
            speaker.speak(utterance); status = "Antwort wird gesprochen"
        } catch { endConversation(); status = "Audio nicht verfügbar – Antwort ist gespeichert" }
    }
    func stopPlayback() { endConversation(); stopOriginalPlayback(); previewingVoiceID = nil; speaker.stopSpeaking(at: .immediate); status = "Wiedergabe gestoppt"; recordDiagnostic("playback_stopped") }
    func refreshVoices() {
        availableVoices = AVSpeechSynthesisVoice.speechVoices().map(SpeechVoiceOption.init).sorted {
            if $0.language.hasPrefix("de") != $1.language.hasPrefix("de") { return $0.language.hasPrefix("de") }
            return ($0.language, $0.name, $0.id) < ($1.language, $1.name, $1.id)
        }
    }
    func previewVoice(_ id: String) {
        if previewingVoiceID == id { stopPlayback(); return }
        stopPlayback()
        guard let voice = AVSpeechSynthesisVoice(identifier: id) else { status = "Stimme nicht mehr verfügbar"; refreshVoices(); return }
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
            #if os(iOS)
            try AVAudioSession.sharedInstance().setActive(true)
            #endif
            let utterance = AVSpeechUtterance(string: voice.language.hasPrefix("de") ? "Hallo, so klingt meine Stimme. Ich lese dir deine Antworten vor." : "Hello, this is a preview of my voice.")
            utterance.voice = voice
            currentUtterance = utterance; speakingId = nil; previewingVoiceID = id
            speaker.speak(utterance)
        } catch { previewingVoiceID = nil; status = "Hörprobe konnte nicht gestartet werden" }
    }
    nonisolated func speechSynthesizer(_ synthesizer: AVSpeechSynthesizer, didStart utterance: AVSpeechUtterance) {
        Task { @MainActor in
            if self.currentUtterance === utterance { self.lastSpeechStart = Date() }
            if let (id, started, _) = self.speechAttempts[ObjectIdentifier(utterance)] {
                try? self.store?.update(id, state: .answered, timing: ("tts_start_ms", Date().timeIntervalSince(started) * 1000), event: ProcessingEvent(operation: "Sprachausgabe gestartet", model: "Apple AVSpeechSynthesizer · " + (utterance.voice?.identifier ?? "Systemstimme"), completedAt: Date(), durationMS: Date().timeIntervalSince(started) * 1000, isAI: false))
                if let entry = try? self.store?.entries().first(where: { $0.id == id }), entry.timings["tts_e2e_ms"] == nil,
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
        previewingVoiceID = nil
        let completedId = speakingId
        currentUtterance = nil; speakingId = nil
        if !recording { status = cancelled ? "Wiedergabe gestoppt – Antwort bleibt gespeichert" : "Bereit" }
        if !cancelled, let completedId { continueConversation(after: completedId) }
        else if cancelled { endConversation() }
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
    private func modelImportProbe(invalid: Bool) {
        Task {
            let source = invalid ? ModelLibrary.folder.deletingLastPathComponent().appendingPathComponent("invalid-model-probe.bin") : ModelLibrary.folder.appendingPathComponent("ggml-base.bin")
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
        providerDescription = speech + ". " + ai + "."
    }
    func downloadModel(_ item: LocalModel) {
        ModelDownloads.shared.start(item)
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
            } catch { modelMessage = ModelTransferError.message(error) }
        }
    }
    func prepareLocalSpeech() {
        guard !preparingSpeech else { return }
        preparingSpeech = true; speechModelMessage = "Apple-Sprachdateien werden vorbereitet …"
        Task {
            defer { preparingSpeech = false }
            do { try await LocalProviders.installSpeech(); speechModelMessage = "Deutsche Sprachdateien bereit"; retry() }
            catch { speechModelMessage = "Apple-Sprachdateien konnten nicht vorbereitet werden: " + error.localizedDescription }
        }
    }
    func cancelProcessing() { jobs?.cancelCurrent() }
    func retryProcessing(_ id: UUID) {
        do { try store?.retryJob(id); jobs?.start(); refresh() }
        catch { status = "Aufnahme bleibt gespeichert – Wiederholung nicht gestartet" }
    }
    #endif
}
