import BackgroundTasks

/// Eine Fortsetzung für die ganze Warteschlange; keine Inhalte in Diagnoseausgaben.
@MainActor
final class MeetingProcessingRuntime {
    static let deferredIdentifier = "de.localvoice.prototype.process-meetings"
    private var access = MeetingProcessing()
    private var continued: BGContinuedProcessingTask?
    private var deferred: BGProcessingTask?
    private var registered = false
    private var continuedRegistered = false
    private var registrationAttempted = false
    private var phase = "Besprechung verarbeiten"
    private var subtitle = ""
    private var queue = MeetingQueueProgress(documents: [])
    var onExpire: (() -> Void)?
    var onResume: ((Bool) -> Void)?
    private lazy var hasGPUEntitlement: Bool = {
        guard let url = Bundle.main.executableURL, let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return false }
        return MeetingProcessing.hasGPUEntitlement(executable: data)
    }()
    var allowed: Bool { access.allowed }

    func register() {
        updateBackgroundSetting()
        guard !registrationAttempted else { return }
        registrationAttempted = true
        continuedRegistered = BGTaskScheduler.shared.register(forTaskWithIdentifier: "de.localvoice.prototype.meeting.*", using: .main) { [weak self] task in
            guard let task = task as? BGContinuedProcessingTask else { task.setTaskCompleted(success: false); return }
            Task { @MainActor in self?.run(task) }
        }
        registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: Self.deferredIdentifier, using: .main) { [weak self] task in
            guard let task = task as? BGProcessingTask else { task.setTaskCompleted(success: false); return }
            Task { @MainActor in
                guard MeetingLibrary.backgroundEnabled else { task.setTaskCompleted(success: false); return }
                guard let self, self.deferred == nil, self.access.identifier == nil else { task.setTaskCompleted(success: false); return }
                self.deferred = task; self.access.deferred = true
                self.queue = MeetingQueueProgress(documents: [])
                task.expirationHandler = { [weak self, weak task] in
                    Task { @MainActor in
                        guard let self, let task, self.deferred === task, self.access.expireDeferred() else { return }
                        self.deferred = nil
                        self.onExpire?()
                        task.setTaskCompleted(success: false)
                        self.scheduleDeferred()
                    }
                }
                self.access.resume()
                self.onResume?(true)
            }
        }
    }

    func updateBackgroundSetting() {
        let identifier = access.identifier
        let revoked = access.setBackgroundEnabled(MeetingLibrary.backgroundEnabled)
        guard !access.backgroundEnabled else { return }
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.deferredIdentifier)
        // Derselbe Abbruchpfad wie beim Ablauf: letzter gespeicherter Checkpoint bleibt erhalten.
        if revoked { onExpire?() }
        continued?.setTaskCompleted(success: false); continued = nil
        deferred?.setTaskCompleted(success: false); deferred = nil
        if let identifier {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: identifier)
        }
    }

    func setForeground(_ foreground: Bool) {
        access.foreground = foreground
        updateBackgroundSetting()
        if foreground { access.resume() }
    }
    func beginWorker() -> Bool { access.beginWorker() }
    func allowStart(userInitiated: Bool) -> Bool {
        updateBackgroundSetting()
        return access.allowStart(userInitiated: userInitiated)
    }
    func nextDocument(in documents: [MeetingDocument]) -> MeetingDocument? {
        guard access.identifier != nil, let id = queue.next(in: documents.map(\.id)) else { return documents.first }
        return documents.first { $0.id == id }
    }
    func endWorker(_ document: UUID, completed: Bool, success: Bool) {
        access.endWorker()
        if completed { queue.complete(document, success: success) }
        continued?.progress.completedUnitCount = queue.completed
    }

    /// Vor dem Hintergrundwechsel einreichen. Eine erfolgreiche .fail-Anfrage darf weiterlaufen.
    func request(_ doc: MeetingDocument, queue documents: [UUID]) {
        guard MeetingLibrary.backgroundEnabled, access.allowed else { return }
        if access.identifier != nil, !queue.contains(doc.id) {
            finish(success: true)
            if !access.foreground { scheduleDeferred() }
        }
        guard access.foreground, access.identifier == nil, deferred == nil else { return }
        guard continuedRegistered else { scheduleDeferred(); return }
        let identifier = "de.localvoice.prototype.meeting." + doc.id.uuidString
        let scheduler = BGTaskScheduler.shared
        let request = BGContinuedProcessingTaskRequest(identifier: identifier,
            title: "Besprechung verarbeiten", subtitle: MeetingListing.title(doc))
        request.strategy = .fail
        let gpuSupported = BGTaskScheduler.supportedResources.contains(.gpu)
        if MeetingProcessing.requestGPU(supported: gpuSupported, entitled: gpuSupported && hasGPUEntitlement) {
            request.requiredResources = .gpu
        }
        queue = MeetingQueueProgress(documents: documents)
        access.submitted(identifier)
        do { try scheduler.submit(request) }
        catch {
            access.finish()
            scheduler.cancel(taskRequestWithIdentifier: identifier)
            scheduleDeferred()
        }
    }

    private func run(_ task: BGContinuedProcessingTask) {
        guard MeetingLibrary.backgroundEnabled else { task.setTaskCompleted(success: false); return }
        guard access.started(task.identifier), continued == nil else { task.setTaskCompleted(success: false); return }
        continued = task
        task.progress.totalUnitCount = queue.total
        task.updateTitle(phase, subtitle: subtitle)
        task.progress.completedUnitCount = queue.completed
        task.expirationHandler = { [weak self, weak task] in
            Task { @MainActor in
                guard let self, let task, self.continued === task, self.access.expire(task.identifier) else { return }
                self.continued = nil
                self.onExpire?()
                task.setTaskCompleted(success: false)
                BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: task.identifier)
                self.scheduleDeferred()
            }
        }
        // Der bereits laufende Worker behält Task, Token und Checkpoint.
        onResume?(false)
    }

    func update(phase: String, subtitle: String, document: UUID, units: Int64) {
        let changed = self.phase != phase || self.subtitle != subtitle
        self.phase = phase; self.subtitle = subtitle
        queue.update(document, units: units)
        continued?.progress.completedUnitCount = queue.completed
        if changed { continued?.updateTitle(phase, subtitle: subtitle) }
    }

    func finish(success: Bool) {
        let success = success && queue.successful
        let identifier = access.identifier
        guard access.finish() else { return }
        if success { continued?.progress.completedUnitCount = queue.total }
        continued?.setTaskCompleted(success: success); continued = nil
        deferred?.setTaskCompleted(success: success); deferred = nil
        if let identifier {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: identifier)
        }
        if success { BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.deferredIdentifier) }
        else { scheduleDeferred() }
    }

    private func scheduleDeferred() {
        guard MeetingLibrary.backgroundEnabled, registered, deferred == nil else { return }
        let request = BGProcessingTaskRequest(identifier: Self.deferredIdentifier)
        request.requiresExternalPower = true
        request.requiresNetworkConnectivity = false
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.deferredIdentifier)
        do { try BGTaskScheduler.shared.submit(request) }
        catch { /* Der nächste Vordergrundlauf bleibt möglich. */ }
    }
}
