import BackgroundTasks

/// Eine Fortsetzung für die ganze Warteschlange; keine Inhalte in Diagnoseausgaben.
@MainActor
final class MeetingProcessingRuntime {
    static let deferredIdentifier = "de.localvoice.prototype.process-meetings"
    private var access = MeetingProcessing()
    private var continued: BGContinuedProcessingTask?
    private var deferred: BGProcessingTask?
    private var registered = false
    private var phase = "Besprechung verarbeiten"
    private var subtitle = ""
    private var units: Int64 = 0
    var onExpire: (() -> Void)?
    var onResume: ((Bool) -> Void)?
    private lazy var hasGPUEntitlement: Bool = {
        guard let url = Bundle.main.executableURL, let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return false }
        return MeetingProcessing.hasGPUEntitlement(executable: data)
    }()
    var allowed: Bool { access.allowed }

    func register() {
        guard !registered else { return }
        registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: Self.deferredIdentifier, using: .main) { [weak self] task in
            guard let task = task as? BGProcessingTask else { task.setTaskCompleted(success: false); return }
            Task { @MainActor in
                guard let self, self.deferred == nil else { task.setTaskCompleted(success: false); return }
                self.deferred = task; self.access.deferred = true
                task.expirationHandler = { [weak self, weak task] in
                    Task { @MainActor in
                        guard let self, let task, self.deferred === task else { return }
                        self.deferred = nil; self.access.deferred = false
                        self.onExpire?()
                        task.setTaskCompleted(success: false)
                        self.scheduleDeferred()
                    }
                }
                self.onResume?(true)
            }
        }
    }

    func setForeground(_ foreground: Bool) { access.foreground = foreground }

    /// Vor dem Hintergrundwechsel einreichen. Eine erfolgreiche .fail-Anfrage darf weiterlaufen.
    func request(_ doc: MeetingDocument) {
        guard access.foreground, access.identifier == nil, deferred == nil else { return }
        let identifier = "de.localvoice.prototype.meeting." + doc.id.uuidString
        let scheduler = BGTaskScheduler.shared
        guard scheduler.register(forTaskWithIdentifier: identifier, using: .main, launchHandler: { [weak self] task in
            guard let task = task as? BGContinuedProcessingTask else { task.setTaskCompleted(success: false); return }
            Task { @MainActor in self?.run(task) }
        }) else { scheduleDeferred(); return }
        let request = BGContinuedProcessingTaskRequest(identifier: identifier,
            title: "Besprechung verarbeiten", subtitle: MeetingListing.title(doc))
        request.strategy = .fail
        let gpuSupported = BGTaskScheduler.supportedResources.contains(.gpu)
        if MeetingProcessing.requestGPU(supported: gpuSupported, entitled: gpuSupported && hasGPUEntitlement) {
            request.requiredResources = .gpu
        }
        access.submitted(identifier)
        do { try scheduler.submit(request) }
        catch {
            access.expire(identifier)
            scheduler.unregisterTask(withIdentifier: identifier)
            scheduleDeferred()
        }
    }

    private func run(_ task: BGContinuedProcessingTask) {
        guard access.started(task.identifier), continued == nil else { task.setTaskCompleted(success: false); return }
        continued = task
        task.progress.totalUnitCount = 1000
        task.updateTitle(phase, subtitle: subtitle)
        update(phase: phase, subtitle: subtitle, units: units)
        task.expirationHandler = { [weak self, weak task] in
            Task { @MainActor in
                guard let self, let task, self.continued === task, self.access.expire(task.identifier) else { return }
                self.continued = nil
                self.onExpire?()
                task.setTaskCompleted(success: false)
                BGTaskScheduler.shared.unregisterTask(withIdentifier: task.identifier)
                self.scheduleDeferred()
            }
        }
        // Der bereits laufende Worker behält Task, Token und Checkpoint.
        onResume?(false)
    }

    func update(phase: String, subtitle: String, units: Int64) {
        let changed = self.phase != phase || self.subtitle != subtitle
        self.phase = phase; self.subtitle = subtitle; self.units = units
        continued?.progress.completedUnitCount = min(999, max(0, units))
        if changed { continued?.updateTitle(phase, subtitle: subtitle) }
    }

    func finish(success: Bool) {
        let identifier = access.identifier
        access.finish()
        if success { continued?.progress.completedUnitCount = 1000 }
        continued?.setTaskCompleted(success: success); continued = nil
        deferred?.setTaskCompleted(success: success); deferred = nil
        if let identifier {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: identifier)
            BGTaskScheduler.shared.unregisterTask(withIdentifier: identifier)
        }
        if success { BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.deferredIdentifier) }
        else { scheduleDeferred() }
    }

    private func scheduleDeferred() {
        guard registered, deferred == nil else { return }
        let request = BGProcessingTaskRequest(identifier: Self.deferredIdentifier)
        request.requiresExternalPower = true
        request.requiresNetworkConnectivity = false
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.deferredIdentifier)
        do { try BGTaskScheduler.shared.submit(request) }
        catch { /* Der nächste Vordergrundlauf bleibt möglich. */ }
    }
}
