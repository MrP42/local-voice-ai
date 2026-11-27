import UIKit
import BackgroundTasks
import UserNotifications
import os

/// Ereignisprotokoll auf dem Gerät für Fehlersuche (Start, Auswertung): `Documents/Diagnose/ereignisse.log`.
/// Nur Zeiten, Zustände, Fehlerarten und Kurz-IDs, NIE Inhalte (Titel, Transkript, Protokoll). Der Mac liest die Datei
/// aus dem App-Container. Schreiben auf einer eigenen Warteschlange; höchstens 512 KB, danach eine Vorgängerdatei.
enum PhoneDiagnostics {
    private static let queue = DispatchQueue(label: "de.localvoice.diagnose", qos: .utility)
    static let launchStart = ContinuousClock.now
    static let limit = 512 * 1024

    static var directory: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Diagnose", isDirectory: true)
    }

    /// Ein Ereignis mit Zeit seit Prozessstart, App-Zustand, freiem Speicher und Wärmezustand.
    static func note(_ event: String) {
        let uptime = ms(since: launchStart)
        let state: String
        if Thread.isMainThread {
            state = MainActor.assumeIsolated { stateName(UIApplication.shared.applicationState) }
        } else {
            state = "?"
        }
        let free = os_proc_available_memory() / 1_048_576
        let thermal = ProcessInfo.processInfo.thermalState.rawValue
        let stamp = ISO8601DateFormatter().string(from: Date())
        let line = "\(stamp) +\(uptime)ms [\(state) frei=\(free)MB therm=\(thermal)] \(event)\n"
        queue.async { append(line) }
    }

    /// Schritt messen und ab 20 ms protokollieren (Start: schwarzer Bildschirm vor dem ersten Bild).
    @discardableResult
    static func measure<T>(_ name: String, _ body: () throws -> T) rethrows -> T {
        let start = ContinuousClock.now
        let value = try body()
        let took = ms(since: start)
        if took >= 20 { note("dauer \(name) \(took)ms") }
        return value
    }

    static func ms(since start: ContinuousClock.Instant) -> Int {
        let d = ContinuousClock.now - start
        return Int(d.components.seconds) * 1000 + Int(d.components.attoseconds / 1_000_000_000_000_000)
    }

    /// Kurz-ID statt voller Kennung: reicht zum Zuordnen im Protokoll.
    static func short(_ id: UUID) -> String { String(id.uuidString.prefix(8)) }

    private static func stateName(_ state: UIApplication.State) -> String {
        switch state {
        case .active: return "vorn"
        case .inactive: return "inaktiv"
        case .background: return "hinten"
        @unknown default: return "?"
        }
    }

    private static func append(_ line: String) {
        let fm = FileManager.default
        let dir = directory, file = dir.appendingPathComponent("ereignisse.log")
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true,
                                attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        if let size = (try? fm.attributesOfItem(atPath: file.path))?[.size] as? Int, size > limit {
            let old = dir.appendingPathComponent("ereignisse.1.log")
            try? fm.removeItem(at: old)
            try? fm.moveItem(at: file, to: old)
        }
        guard let data = line.data(using: .utf8) else { return }
        if let handle = try? FileHandle(forWritingTo: file) {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: data)
        } else {
            try? data.write(to: file, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        }
    }
}

/// Uses only system-granted runtime. Originals and STT checkpoints outlive each lease.
@MainActor
final class PhoneProcessingRuntime {
    static let taskIdentifier = "de.localvoice.prototype.process-recordings"
    private let worker: JobProcessor
    private let store: DurableStore
    private var access = ProcessingAccess()
    private var lease: UIBackgroundTaskIdentifier = .invalid
    private var scheduledTask: BGProcessingTask?
    private var registered = false
    var onDiagnostic: ((String) -> Void)?

    init(worker: JobProcessor, store: DurableStore) {
        self.worker = worker; self.store = store
    }
    /// Preserve encryption while allowing work after the first unlock since reboot.
    func prepareProtectedFiles() throws {
        for root in [store.root, ModelLibrary.folder] where FileManager.default.fileExists(atPath: root.path) {
            try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: root.path)
            if let files = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil) {
                for case let url as URL in files {
                    try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: url.path)
                }
            }
        }
    }
    func register() {
        guard !registered else { return }
        registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: Self.taskIdentifier, using: .main) { [weak self] task in
            guard let processing = task as? BGProcessingTask else { task.setTaskCompleted(success: false); return }
            Task { @MainActor in self?.run(processing) }
        }
        if !registered { onDiagnostic?("background_registration_failed") }
    }
    private var hasPendingWork: Bool {
        guard let entries = try? store.entries() else { return false }
        return entries.contains { entry in
            guard entry.reply == nil else { return false }
            let job = entry.job ?? JobRecord()
            return job.phase != .failed && job.phase != .cancelled && job.attempts < 3
        }
    }
    func setForeground(_ foreground: Bool) {
        access.foreground = foreground
        if foreground {
            worker.setActive(true)
            endLease()
        } else {
            if hasPendingWork || worker.isProcessing { acquireLease() }
            worker.setActive(access.allowed)
            scheduleDeferred()
        }
    }
    func receivedCapture() {
        if !access.foreground && hasPendingWork { acquireLease() }
        worker.setActive(access.allowed)
        if !access.foreground { scheduleDeferred() }
    }
    func processingChanged() {
        guard !worker.isProcessing else { return }
        if let task = scheduledTask {
            scheduledTask = nil; access.scheduledTask = false
            task.setTaskCompleted(success: !hasPendingWork)
            onDiagnostic?("background_scheduled_finished")
        }
        endLease()
        // Do not start another empty worker from its completion callback.
        if !access.allowed { worker.setActive(false) }
        if hasPendingWork { scheduleDeferred() }
        else { BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.taskIdentifier) }
    }
    private func acquireLease() {
        guard lease == .invalid, scheduledTask == nil else { return }
        lease = UIApplication.shared.beginBackgroundTask(withName: "Watch-Aufnahme verarbeiten") { [weak self] in
            Task { @MainActor in self?.expireLease() }
        }
        access.watchLease = lease != .invalid
        onDiagnostic?(access.watchLease ? "background_lease_started" : "background_lease_denied")
    }
    private func expireLease() {
        access.watchLease = false
        worker.setActive(access.allowed)
        // Persist the interruption even if native inference has not returned yet.
        if !access.allowed, let id = worker.currentId { try? store.pauseJob(id) }
        endLease()
        scheduleDeferred()
        onDiagnostic?("background_lease_expired_saved")
    }
    private func endLease() {
        let previous = lease
        lease = .invalid; access.watchLease = false
        if previous != .invalid { UIApplication.shared.endBackgroundTask(previous) }
    }
    private func run(_ task: BGProcessingTask) {
        scheduledTask = task; access.scheduledTask = true
        endLease()
        task.expirationHandler = { [weak self, weak task] in
            Task { @MainActor in
                guard let self, let task, self.scheduledTask === task else { return }
                self.scheduledTask = nil; self.access.scheduledTask = false
                self.worker.setActive(self.access.allowed)
                if !self.access.allowed, let id = self.worker.currentId { try? self.store.pauseJob(id) }
                task.setTaskCompleted(success: false)
                self.scheduleDeferred()
                self.onDiagnostic?("background_scheduled_expired_saved")
            }
        }
        do { if !worker.isProcessing { try store.recoverInterruptedJobs() } }
        catch { task.setTaskCompleted(success: false); scheduledTask = nil; access.scheduledTask = false; return }
        onDiagnostic?("background_scheduled_started")
        worker.setActive(true)
    }
    private func scheduleDeferred() {
        guard registered, hasPendingWork, scheduledTask == nil else { return }
        let request = BGProcessingTaskRequest(identifier: Self.taskIdentifier)
        request.requiresNetworkConnectivity = false
        request.requiresExternalPower = false
        // iOS chooses when to run; this is not a real-time timer.
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.taskIdentifier)
        do { try BGTaskScheduler.shared.submit(request) }
        catch { onDiagnostic?("background_schedule_unavailable_saved") }
    }
    #if DEBUG && targetEnvironment(simulator)
    func expireForTesting() { expireLease() }
    #endif
}

@MainActor
final class VoicePhoneDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    private var gpuObservers: [NSObjectProtocol] = []
    private func observeGPULifecycle(_ application: UIApplication) {
        guard gpuObservers.isEmpty else { return }
        lv_set_gpu_allowed(0)
        let center = NotificationCenter.default
        // Synchron auf dem meldenden Thread sperren, ohne Task-/Main-Queue-Verzögerung.
        for name in [UIApplication.willResignActiveNotification, UIApplication.didEnterBackgroundNotification] {
            gpuObservers.append(center.addObserver(forName: name, object: nil, queue: nil) { _ in
                lv_set_gpu_allowed(0)
            })
        }
        gpuObservers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: nil) { _ in
            lv_set_gpu_allowed(1)
        })
        lv_set_gpu_allowed(application.applicationState == .active ? 1 : 0)
    }
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        PhoneDiagnostics.note("start")
        observeGPULifecycle(application)
        // Jeder Schritt läuft vor dem ersten Bild: gemessen, weil der Start mehrere Sekunden schwarz blieb (27.11.).
        PhoneDiagnostics.measure("VoiceModel") { VoiceModel.shared.registerBackgroundProcessing() }
        PhoneDiagnostics.measure("MeetingLibrary") { MeetingLibrary.shared.registerBackgroundProcessing() }
        PhoneDiagnostics.measure("ModelDownloads") { _ = ModelDownloads.shared }
        PhoneDiagnostics.measure("AudioDownloads") { _ = AudioDownloads.shared }  // Hörbuch-Downloads im Hintergrund wieder zuordnen
        // Assistent: Ordnung des Prozesses mit Protokoll und Meldestelle der Quellen verbinden (ohne Gespräch)
        PhoneDiagnostics.measure("Assistent") { AssistantReleaseFiles.start() }
        PhoneDiagnostics.note("start fertig")
        UNUserNotificationCenter.current().delegate = self
        return true
    }
    func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String, completionHandler: @escaping () -> Void) {
        switch identifier {
        case ModelDownloads.sessionID: ModelDownloads.shared.handleBackgroundEvents(completionHandler)
        case AudioDownloads.sessionID: AudioDownloads.shared.handleBackgroundEvents(completionHandler)
        default: completionHandler()
        }
    }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([]) // Foreground progress and readiness are already visible beside the model.
    }
}
