import UIKit
import BackgroundTasks

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
final class VoicePhoneDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        VoiceModel.shared.registerBackgroundProcessing()
        return true
    }
}
