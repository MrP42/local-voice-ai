import BackgroundTasks

/// Eine Fortsetzung für die ganze Warteschlange; keine Inhalte in Diagnoseausgaben.
@MainActor
final class MeetingProcessingRuntime {
    static let deferredIdentifier = "de.localvoice.prototype.process-meetings"
    private var access = MeetingProcessing()
    private var continued: BGContinuedProcessingTask?
    private var deferred: BGProcessingTask?
    private var registered = false
    private var continuedIdentifiers: Set<String> = []
    private var registrationAttempted = false
    private var phase = "Besprechung verarbeiten"
    private var subtitle = ""
    private var queue = MeetingQueueProgress(documents: [])
    /// Zwischenschritte zwischen echten Fortschrittsmeldungen: iOS beendet Fortsetzungen, deren Fortschritt stillsteht.
    private var tick: Int64 = 0
    private var heartbeat: Task<Void, Never>?
    private static let scale: Int64 = 100
    /// Live-Aktivität (Patrick 28.11.): nur kurz groß, dann in der Insel; dort steht die Restdauer. Jede Änderung von
    /// Titel oder Untertitel kann die Anzeige wieder groß zeigen. Darum: kein Phasenname mehr im Titel, die Restdauer in
    /// groben Stufen und höchstens einmal je `titleInterval`; der Fortschritt selbst läuft über `progress`.
    static let title = "Besprechung verarbeiten"
    static let titleInterval: TimeInterval = 60
    private var remaining = MeetingRemainingTime()
    private var shownTitle: String?
    private var lastTitleUpdate = Date.distantPast
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
        if completed { tick = 0 }
        publishProgress()
        refreshRemaining()
    }
    private func publishProgress() {
        guard let continued else { return }
        continued.progress.completedUnitCount = min(queue.total * Self.scale, queue.completed * Self.scale + tick)
    }

    /// Vor dem Hintergrundwechsel einreichen. Eine erfolgreiche .fail-Anfrage darf weiterlaufen.
    func request(_ doc: MeetingDocument, queue documents: [UUID]) {
        guard MeetingLibrary.backgroundEnabled, access.allowed else { return }
        if access.identifier != nil, !queue.contains(doc.id) {
            finish(success: true)
            if !access.foreground { scheduleDeferred() }
        }
        guard access.foreground, access.identifier == nil, deferred == nil else { return }
        let identifier = "de.localvoice.prototype.meeting." + doc.id.uuidString
        let scheduler = BGTaskScheduler.shared
        if !continuedIdentifiers.contains(identifier) {
            guard scheduler.register(forTaskWithIdentifier: identifier, using: .main, launchHandler: { [weak self] task in
                guard let task = task as? BGContinuedProcessingTask else { task.setTaskCompleted(success: false); return }
                Task { @MainActor in self?.run(task) }
            }) else { scheduleDeferred(); return }
            // Auch nach Abschluss oder Abbruch bleibt der konkrete Handler registriert.
            continuedIdentifiers.insert(identifier)
        }
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
        task.progress.totalUnitCount = queue.total * Self.scale
        remaining.reset()
        shownTitle = Self.title; lastTitleUpdate = Date()
        task.updateTitle(Self.title, subtitle: subtitle)
        tick = 0; publishProgress(); refreshRemaining()
        heartbeat?.cancel()
        heartbeat = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                guard let self, self.continued != nil else { return }
                // Nur innerhalb des laufenden Schritts; die nächste echte Meldung liegt immer darüber.
                self.tick = min(self.tick + 1, Self.scale - 1)
                self.publishProgress()
            }
        }
        task.expirationHandler = { [weak self, weak task] in
            Task { @MainActor in
                guard let self, let task, self.continued === task, self.access.expire(task.identifier) else { return }
                self.continued = nil
                self.heartbeat?.cancel(); self.heartbeat = nil
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
        // Nur ein Wechsel der Besprechung ändert den Untertitel sofort; Phasen stehen in der App, nicht in der Insel.
        let nextMeeting = self.subtitle != subtitle
        self.phase = phase; self.subtitle = subtitle
        let before = queue.completed
        queue.update(document, units: units)
        if queue.completed != before { tick = 0 }
        publishProgress()
        refreshRemaining(force: nextMeeting)
    }

    /// Restdauer aus dem echten Fortschritt: in `progress` (System) und grob im Titel, gedrosselt.
    private func refreshRemaining(force: Bool = false) {
        guard let continued else { return }
        let now = Date()
        let seconds = remaining.estimate(completed: queue.completed * Self.scale, total: queue.total * Self.scale, at: now)
        continued.progress.estimatedTimeRemaining = seconds
        let title = MeetingRemainingTime.label(seconds) ?? shownTitle ?? Self.title
        guard force || (title != shownTitle && now.timeIntervalSince(lastTitleUpdate) >= Self.titleInterval) else { return }
        shownTitle = title; lastTitleUpdate = now
        continued.updateTitle(title, subtitle: subtitle)
    }

    func finish(success: Bool) {
        let success = success && queue.successful
        let identifier = access.identifier
        guard access.finish() else { return }
        heartbeat?.cancel(); heartbeat = nil
        if success { continued?.progress.completedUnitCount = queue.total * Self.scale }
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
