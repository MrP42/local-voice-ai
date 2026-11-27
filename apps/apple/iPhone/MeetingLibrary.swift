import SwiftUI
import AVFoundation
import UniformTypeIdentifiers
import UserNotifications

@MainActor
final class MeetingLibrary: ObservableObject {
    static let shared = MeetingLibrary()
    @Published private(set) var documents: [MeetingDocument] = [] { didSet { searchIndex = Dictionary(uniqueKeysWithValues: documents.map { ($0.id, MeetingListing.searchText($0)) }) } }
    /// Gefalteter Suchtext je Besprechung (lva#837), neu nur bei geändertem Bestand.
    private(set) var searchIndex: [UUID: String] = [:]
    @Published private(set) var status = "Audio oder Video importieren"
    @Published private(set) var running: UUID?
    @Published private(set) var importBusy = false
    @Published var error: String?
    private var archive: MeetingArchive?
    /// Eingang in der App-Gruppe (Share-Extension, „Öffnen in“, Dateiauswahl).
    private var inbox: MeetingInbox?
    private var draining = false
    /// Sitzungen, die gerade übernommen werden (Szenenwechsel kann doppelt auslösen).
    private var importingLive: Set<UUID> = []
    private var worker: Task<Void, Never>?
    private var readyNotifications = MeetingReadyNotifications()
    private var token: InferenceCancellation?
    private var exporter: AVAssetExportSession?
    private var failed = Set<UUID>()
    /// Automatische Wiederholungen je Besprechung nach vorübergehenden Fehlern (lva#960).
    private var retryAttempts: [UUID: Int] = [:]
    private var retryTasks: [UUID: Task<Void, Never>] = [:]
    /// Vorübergehend gescheitert: beim nächsten Öffnen der App sofort wieder aufnehmen (Apple-Modell drosselt im Hintergrund).
    private var transientFailures = Set<UUID>()
    static let retryDelays: [UInt64] = [60, 5 * 60, 15 * 60]
    /// Gerade gelöschte Besprechungen: der Worker nimmt sie nicht mehr auf.
    private var deleting = Set<UUID>()
    private lazy var background: MeetingProcessingRuntime = {
        let runtime = MeetingProcessingRuntime()
        runtime.onExpire = { [weak self] in
            guard let self else { return }
            // Im Vordergrund braucht die Verarbeitung keine Systemfreigabe: weiterrechnen statt abbrechen (Hänger 26.11.).
            if self.foreground { self.background.setForeground(true); return }
            self.worker?.cancel(); self.token?.cancel(); self.exporter?.cancelExport()
            // iOS hat die Fortsetzung beendet: Liste zeigt „pausiert“, bis es wieder läuft.
            self.backgroundPaused = true
        }
        runtime.onResume = { [weak self] deferred in
            Task { @MainActor in
                guard let self else { return }
                if deferred { self.failed.removeAll() }
                await self.refresh(); self.start()
            }
        }
        return runtime
    }()
    func registerBackgroundProcessing() { background.register() }
    func updateBackgroundSetting() { background.updateBackgroundSetting() }
    /// Einstellung „Im Hintergrund weiterverarbeiten“ (lva#935), Standard an.
    static let backgroundSettingKey = "meetingBackgroundProcessing"
    static var backgroundEnabled: Bool { UserDefaults.standard.object(forKey: backgroundSettingKey) as? Bool ?? true }
    /// Die letzte Hintergrund-Fortsetzung wurde von iOS beendet; der Rest folgt am Ladekabel oder beim Öffnen.
    @Published private(set) var backgroundPaused = false
    private var foreground = true
    let modelCatalog = AppModelCatalog.shared
    var modelAccessState: ModelAccessState { modelCatalog.accessState }
    @Published private(set) var localMinutesAvailable: Set<UUID> = []
    private var localMinutesRequested: Set<UUID> = []
    private var loadedModelConsents: Set<UUID> = []
    private var loadingModelConsents: [UUID: Task<Void, Error>] = [:]
    private var modelConsentUpdate: Task<Void, Error>?
    /// Offene Zustimmung für eine externe Auswertung (T3b); der Dialog liest Geltungsbereich und Ziel nur von hier.
    @Published var consentRequest: ModelConsentRequest?

    init() {
        #if DEBUG
        // Vorschau für das Mac-Tor: Zustand nach beendeter Hintergrundverarbeitung (lva#935).
        if ProcessInfo.processInfo.arguments.contains("--meeting-background-paused") { backgroundPaused = true }
        #endif
        do {
            let root = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Meetings")
            archive = try MeetingArchive(root: root)
            // Ohne eingerichtete App-Gruppe (z. B. unsignierter Simulator) ein lokaler Eingang;
            // die Share-Extension meldet dann selbst, dass sie nicht übergeben kann.
            let inboxRoot = MeetingInbox.sharedRoot() ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Eingang")
            let inbox = try MeetingInbox(root: inboxRoot, digest: SHA256Digest())
            inbox.sweepAbandoned()
            self.inbox = inbox
        } catch { self.error = "Aufnahmespeicher konnte nicht geöffnet werden." }
    }
    func scene(active: Bool, inactive: Bool = false) {
        // .inactive liegt noch vor .background; nur hier darf erneut eingereicht werden.
        if active { foreground = true } else if !inactive { foreground = false }
        if inactive, Self.backgroundEnabled, let doc = pendingDocument {
            background.request(doc, queue: pendingDocuments.map(\.id))
            Task { await Self.allowQuietNotifications() }
        }
        background.setForeground(active)
        if active { resumeTransientFailures(); Task { await recoverLive(); await drainInbox(); await refresh(); start() } }
        else if !inactive && !background.allowed {
            worker?.cancel(); token?.cancel(); exporter?.cancelExport()
            if running != nil { status = "gespeichert – Verarbeitung folgt" }
        }
    }
    func refresh() async {
        do { documents = try await archive?.list() ?? [] }
        catch { self.error = "Der Aufnahmespeicher konnte nicht vollständig gelesen werden. Originaldateien bleiben erhalten." }
    }
    func importMedia(_ url: URL) async {
        guard !importBusy, let archive else { return }
        importBusy = true; defer { importBusy = false }
        error = nil
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let startTime = ContinuousClock.now
        do {
            let asset = AVURLAsset(url: url)
            guard !(try await asset.loadTracks(withMediaType: .audio)).isEmpty else { throw VoiceError.invalid }
            let duration = try await asset.load(.duration).seconds
            let doc = try await archive.importMedia(url, duration: duration,
                minutesRouteKey: UserDefaults.standard.string(forKey: ModelSelection.settingsKey(.meetingMinutes)))
            status = "gespeichert – Verarbeitung folgt"
            try await archive.recordTiming(doc.id, phase: "import", milliseconds: elapsed(startTime))
            await refresh(); start()
        } catch {
            self.error = "Import nicht abgeschlossen. Bitte eine lesbare Audio-/Videodatei bis 2 Stunden und 2 GB wählen und freien Speicher prüfen."
        }
    }
    /// Bestätigte Datei in den Eingang kopieren (geprüft, dauerhaft) und gleich übernehmen.
    /// `true` erst nach dem dauerhaften Eingangs-Commit; nur dann darf die Quelle weg.
    @discardableResult
    func deposit(_ url: URL, from source: InboxItem.Source, consent: ConsentRecord) async -> Bool {
        guard let inbox else { self.error = "Der Eingang ist nicht verfügbar."; return false }
        // Nacheinander, nie verworfen: warten, bis ein laufender Import fertig ist.
        while importBusy { try? await Task.sleep(for: .milliseconds(200)) }
        importBusy = true; defer { importBusy = false }
        error = nil
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            _ = try await Task.detached { try inbox.deposit(url, from: source, consent: consent) }.value
        } catch IntakeError.tooLarge {
            self.error = "Die Datei ist größer als 2 GB."; return false
        } catch IntakeError.conflict {
            self.error = "Diese Aufnahme wartet bereits mit einer anderen Einwilligung. Eine frühere Freigabe wird nicht übernommen."; return false
        } catch {
            self.error = "Die Datei konnte nicht gesichert werden. Bitte freien Speicher prüfen."; return false
        }
        await drainInbox()
        return true
    }

    /// Wartende Einträge ins Archiv übernehmen; Transkription und (falls freigegeben)
    /// Auswertung starten danach automatisch. Ein Eintrag verschwindet erst nach
    /// erfolgreicher, dauerhafter Übernahme; die Wiederholung ist idempotent (gleiche Kennung).
    func drainInbox() async {
        guard let inbox, let archive, !draining else { return }
        draining = true; defer { draining = false }
        for item in (try? inbox.pending()) ?? [] {
            do {
                _ = try await inbox.take(item) { url, item in
                    let asset = AVURLAsset(url: url)
                    guard !(try await asset.loadTracks(withMediaType: .audio)).isEmpty else { throw VoiceError.invalid }
                    let duration = try await asset.load(.duration).seconds
                    var log = ConsentLog()
                    try log.append(item.consent)
                    return try await archive.importMedia(url, duration: duration,
                                                         live: MeetingArchive.LiveImport(consent: log, live: nil, segments: [], notes: nil),
                                                         id: item.id, name: item.originalName,
                                                         minutesRouteKey: UserDefaults.standard.string(forKey: ModelSelection.settingsKey(.meetingMinutes)))
                }
                status = "gespeichert – Verarbeitung folgt"
            } catch VoiceError.conflict {
                // Schon im Archiv (Absturz nach der Übernahme, vor dem Aufräumen): nicht doppelt
                // anlegen. Es gilt die beim ersten Mal gespeicherte Einwilligung.
                // Nur bei intaktem Archivbestand UND gleichen Einwilligungsbedingungen als erledigt
                // quittieren; eine abweichende (z. B. KI-)Einwilligung bleibt ein Konflikt.
                let doc = try? await archive.load(item.id)
                let sameTerms = doc?.consent?.records.first.map { $0.sameTerms(as: item.consent) } ?? false
                let intact = (try? await archive.verifyCommitted(item.id, digest: item.digest)) ?? false
                if sameTerms, intact, (try? await inbox.take(item) { _, _ in () }) != nil {
                    self.error = "„\(item.originalName)“ war bereits übernommen. Es gilt die damalige Einwilligung; zum Ändern das Dokument löschen und neu teilen."
                } else if intact, !sameTerms {
                    self.error = "„\(item.originalName)“ liegt bereits mit einer anderen Einwilligung im Archiv. Zum Ändern das Dokument löschen und neu teilen."
                } else {
                    self.error = "„\(item.originalName)“ ist gesichert, konnte aber noch nicht übernommen werden. Nächster Versuch beim nächsten Öffnen."
                }
            } catch IntakeError.busy {
                // Die Extension schreibt gerade an diesem Eintrag: beim nächsten Durchlauf erneut.
                continue
            } catch VoiceError.invalid {
                // Keine lesbare Tonspur oder über 2 Stunden: nicht aufbewahren (Datenminimierung).
                try? inbox.discard(item)
                self.error = "„\(item.originalName)“ hat keine lesbare Tonspur oder ist länger als 2 Stunden und wurde nicht übernommen."
            } catch {
                self.error = "„\(item.originalName)“ ist gesichert, konnte aber noch nicht übernommen werden. Nächster Versuch beim nächsten Öffnen."
            }
        }
        await refresh(); start()
    }

    /// Liegt diese Datei schon dauerhaft im Eingang oder Archiv? (Kennung aus dem Inhalt.)
    /// Dann darf die iOS-Kopie ohne neue Rückfrage weg; ein Neustart importiert nicht doppelt.
    func alreadyHeld(_ url: URL) async -> Bool {
        guard let inbox, let archive else { return false }
        guard let hex = try? await Task.detached(operation: { try SHA256Digest().hex(of: url) }).value else { return false }
        let id = MeetingInbox.itemId(digest: hex)
        // Nur ein intakter, erneut gesicherter Bestand zählt (Original, Größe, voller Digest).
        if let item = (try? inbox.pending())?.first(where: { $0.id == id && $0.digest == hex }) {
            return (try? await Task.detached(operation: { try inbox.verifyCommitted(item) }).value) ?? false
        }
        return (try? await archive.verifyCommitted(id, digest: hex)) ?? false
    }

    /// In der App aufgenommene Besprechung übernehmen: Original, Einwilligungsnachweis
    /// und Live-Stand in einem Schritt ins Archiv. Erst danach wird der Laufend-Ordner
    /// entfernt; scheitert etwas, bleibt er für den nächsten Versuch.
    func importLive(_ url: URL, _ session: LiveMeetingModel.SessionFile) async {
        guard let archive, !importingLive.contains(session.id) else { return }
        importingLive.insert(session.id); defer { importingLive.remove(session.id) }
        let folder = url.deletingLastPathComponent()
        // Ohne gültigen Nachweis wird nichts übernommen; der Ordner bleibt zur Klärung liegen.
        guard session.consent.canRecord else {
            self.error = "Eine Aufnahme ohne vollständigen Einwilligungsnachweis wurde nicht übernommen."; return
        }
        do {
            // Echte Dateidauer archivieren; die Aufnahme selbst endet spätestens bei 2 Stunden.
            let measured = try? await AVURLAsset(url: url).load(.duration).seconds
            let duration = (measured?.isFinite == true && measured! > 0) ? measured! : session.seconds
            guard duration.isFinite, duration > 0.5 else {
                // Nur eine wirklich leere Sitzung wird verworfen, nie eine mit Inhalt.
                if session.isEmpty { try? FileManager.default.removeItem(at: folder) }
                else { self.error = "Die Aufnahme ist gesichert, aber nicht lesbar. Sie bleibt erhalten und wird erneut versucht." }
                return
            }
            let live = MeetingArchive.LiveImport(consent: session.consent, live: session.live, segments: session.segments, notes: session.notes,
                                                 deferredRefinement: session.deferredRefinement, title: session.title,
                                                 participantNames: session.participantNames, startedAt: session.startedAt)
            // Feste Kennung = Sitzungskennung: nach einem Absturz liefert die Wiederholung dasselbe Dokument.
            _ = try await archive.importMedia(url, duration: duration, live: live, id: session.id,
                minutesRouteKey: UserDefaults.standard.string(forKey: ModelSelection.settingsKey(.meetingMinutes)))
            // Erst nach dem synchronisierten Übernehmen wird die Quelle entfernt.
            try? FileManager.default.removeItem(at: folder)
            status = "gespeichert – Verarbeitung folgt"
            await refresh(); start()
        } catch {
            self.error = "Die Aufnahme ist gesichert, konnte aber noch nicht übernommen werden. Sie wird beim nächsten Öffnen erneut übernommen."
        }
    }

    /// Nach Absturz oder Beenden während einer Aufnahme: gesicherte Sitzungen übernehmen.
    func recoverLive() async {
        for (url, session) in LiveMeetingModel.abandoned() { await importLive(url, session) }
    }

    #if DEBUG
    func importProbe() async {
        let args = ProcessInfo.processInfo.arguments
        guard let index = args.firstIndex(of: "--meeting-import-probe"), args.indices.contains(index + 1) else { return }
        let name = args[index + 1]
        guard name == URL(fileURLWithPath: name).lastPathComponent else { return }
        await refresh()
        if documents.contains(where: { $0.originalName == name }) { return }
        let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("MeetingFixtures").appendingPathComponent(name)
        await importMedia(url)
    }
    #endif
    func retry() {
        failed.removeAll(); error = nil
        for task in retryTasks.values { task.cancel() }
        retryTasks.removeAll(); retryAttempts.removeAll(); transientFailures.removeAll()
        start(userInitiated: true)
    }

    /// Vorübergehende Fehler (Antwort unlesbar, Netz, Server, Zeitlimit, Modell nicht erreichbar) setzen sich
    /// nach 1, 5 und 15 Minuten selbst fort. Fehlende Zustimmung, Sperren und zu lange Texte brauchen den Nutzer.
    static func isTransient(_ error: Error) -> Bool {
        guard let failure = error as? ModelFailure else { return true }
        switch failure {
        case .unavailable, .timeout, .stateChanged: return true
        case .provider(let kind): return ![.unauthorized, .notInstalled, .contextTooLong].contains(kind)
        default: return false
        }
    }

    /// Beim Öffnen der App: den geplanten Versuch sofort statt nach Ablauf des Zeitgebers. Das Budget (3 je
    /// Fehlerepisode, bis Erfolg oder „Erneut versuchen“) zählt scheduleRetry; das Öffnen zählt nichts dazu.
    private func resumeTransientFailures() {
        guard !transientFailures.isEmpty else { return }
        for id in transientFailures {
            // Nur ein schon geplanter (und beim Planen gezählter) Versuch wird vorgezogen; ohne ihn ist das Budget aufgebraucht.
            guard let pending = retryTasks.removeValue(forKey: id) else { continue }
            pending.cancel()
            failed.remove(id); transientFailures.remove(id)
        }
        if failed.isEmpty { error = nil }
    }

    /// Plant die nächste automatische Wiederholung; nil, wenn keine mehr folgt.
    private func scheduleRetry(_ id: UUID, after error: Error) -> UInt64? {
        // Ein alter Zeitgeber darf einen späteren, nicht vorübergehenden Fehler nie überschreiben.
        retryTasks.removeValue(forKey: id)?.cancel()
        let attempt = retryAttempts[id, default: 0]
        guard Self.isTransient(error), attempt < Self.retryDelays.count else { return nil }
        let delay = Self.retryDelays[attempt]
        retryAttempts[id] = attempt + 1
        retryTasks[id]?.cancel()
        retryTasks[id] = Task { [weak self] in
            try? await Task.sleep(nanoseconds: delay * 1_000_000_000)
            guard !Task.isCancelled, let self else { return }
            self.retryTasks[id] = nil
            guard self.failed.remove(id) != nil else { return }
            if self.failed.isEmpty { self.error = nil }
            self.start()
        }
        return delay
    }

    /// Verständliche Meldung: was fehlt, was gesichert ist und wie es weitergeht (lva#960).
    static func failureMessage(_ error: Error, transcribed: Bool, retryIn delay: UInt64?) -> String {
        let what: String
        if let failure = error as? ModelFailure {
            switch failure {
            case .provider(.invalidResponse):
                what = "Das Protokoll konnte nicht erstellt werden: Das Modell hat keine lesbare Antwort geliefert."
            case .provider(.network): what = "Das Protokoll konnte nicht erstellt werden: keine Verbindung zum Modell."
            case .provider(.rateLimited):
                what = "Das Protokoll wartet: Das Modell ist gerade ausgelastet oder wird im Hintergrund von iOS gebremst."
            default: what = "Das Protokoll konnte nicht erstellt werden. \(ModelFailure.sanitized(error).localizedDescription)"
            }
        } else {
            what = "Die Verarbeitung auf dem iPhone ist nicht abgeschlossen."
        }
        let kept = transcribed ? " Aufnahme und Transkript sind gespeichert." : " Aufnahme und Fortschritt sind gespeichert."
        let next: String
        if let delay {
            let minutes = max(1, Int(delay / 60))
            next = minutes == 1 ? " Neuer Versuch in einer Minute." : " Neuer Versuch in \(minutes) Minuten."
        } else {
            next = " Tippe auf „Erneut versuchen“ oder erstelle das Protokoll auf dem iPhone."
        }
        return what + kept + next
    }
    /// T3 kann diese Aktion nach einem Fehler anbieten. Nur eine ausdrückliche Aktion wechselt lokal.
    func createMinutesLocally(_ id: UUID) {
        guard localMinutesAvailable.contains(id) else { return }
        retryTasks.removeValue(forKey: id)?.cancel(); transientFailures.remove(id)
        localMinutesRequested.insert(id); localMinutesAvailable.remove(id)
        failed.remove(id); error = nil; start(userInitiated: true)
    }

    /// T3 speichert vor der Freigabe im gemeinsamen Zustand; fehlgeschlagene Sicherung erlaubt kein Senden.
    func recordModelConsent(_ record: ModelConsentRecord, for id: UUID) async throws {
        try await changeModelConsent(record.scope, record: record, for: id)
    }

    func revokeModelConsent(_ scope: ModelConsentScope, for id: UUID) async throws {
        try await changeModelConsent(scope, record: nil, for: id)
    }

    private func changeModelConsent(_ scope: ModelConsentScope, record: ModelConsentRecord?, for id: UUID) async throws {
        guard let archive, scope.subjectId == id.uuidString else { throw ModelFailure.consentMissing }
        let previous = modelConsentUpdate
        let state = modelAccessState
        let update = Task {
            // Ganze Änderungen ordnen, auch über Suspensionen: ein alter Grant darf keinen Widerruf überholen.
            _ = try? await previous?.value
            try await loadModelConsents(id)
            if let record {
                try await archive.updateModelConsent(id, scope: scope, record: record)
                await state.record(record)
            } else {
                // Auch bei einem Speicherfehler gilt der Widerruf sofort für diesen Prozesslauf.
                await state.revoke(scope)
                try await archive.updateModelConsent(id, scope: scope, record: nil)
            }
        }
        modelConsentUpdate = update
        try await update.value
    }

    /// Geltungsbereich und Ziel aus dem gemeinsamen Zustand; scheitert die Auflösung, bleibt nur der Fehlertext.
    private func prepareConsentRequest(_ doc: MeetingDocument) async {
        let choices = await modelCatalog.choices(for: .meetingMinutes)
        guard let key = doc.minutesRouteKey, let choice = choices.first(where: { $0.id == key }), !choice.route.isLocal,
              let resolved = try? await modelAccessState.consentScope(route: choice.route, purpose: .meetingMinutes,
                  subjectId: doc.id.uuidString, dataCategories: Self.minutesDataCategories) else { return }
        consentRequest = ModelConsentRequest(id: doc.id, title: MeetingListing.title(doc), choice: choice, scope: resolved.0, target: resolved.1,
                                             localAvailable: choices.contains { $0.route.isLocal })
    }

    /// Entscheidung aus dem Zustimmungsdialog anwenden. Erst gespeicherte Zustimmung erlaubt das Senden.
    func decide(_ decision: ModelConsentSheet.Decision, for request: ModelConsentRequest) async {
        consentRequest = nil
        switch decision {
        case .grant(let record):
            do { try await recordModelConsent(record, for: request.id); retry() }
            catch { self.error = "Die Zustimmung konnte nicht gespeichert werden. Es wurde nichts gesendet." }
        case .local: createMinutesLocally(request.id)
        case .cancel: break
        }
    }

    /// Dieselben Kategorien wie beim Öffnen der Sitzung; Zustimmung und Senden müssen übereinstimmen.
    static let minutesDataCategories = ["Transkript", "Teilnehmernamen"]

    /// Vor dem Zustimmungsdialog laden. Pro Prozess nur einmal, damit ein Widerruf nie alte Nachweise wiederbelebt.
    func loadModelConsents(_ id: UUID) async throws {
        if let loading = loadingModelConsents[id] { try await loading.value; return }
        guard !loadedModelConsents.contains(id), let archive else { return }
        let state = modelAccessState
        let loading = Task {
            let records = try await archive.load(id).modelConsents ?? []
            for record in records where record.scope.subjectId == id.uuidString { await state.record(record) }
        }
        loadingModelConsents[id] = loading
        defer { loadingModelConsents[id] = nil }
        try await loading.value
        loadedModelConsents.insert(id)
    }
    /// Anzeige-Status für die Liste (lva#837).
    func listingStatus(_ doc: MeetingDocument) -> MeetingListing.Status {
        MeetingListing.status(doc, running: running == doc.id, failed: failed.contains(doc.id))
    }
    /// Eigener Titel; leer stellt den automatischen wieder her.
    func rename(_ id: UUID, to title: String) async {
        guard let archive else { return }
        do { _ = try await archive.rename(id, title: title) }
        catch { self.error = "Der Titel konnte nicht gespeichert werden." }
        await refresh()
    }
    /// Besprechung samt Original löschen. Läuft gerade ihre Verarbeitung, wird sie zuerst sauber beendet.
    func delete(_ id: UUID) async {
        guard let archive, !deleting.contains(id) else { return }
        deleting.insert(id); defer { deleting.remove(id) }
        if running == id, let task = worker {
            task.cancel(); token?.cancel(); exporter?.cancelExport()
            await task.value
        }
        do {
            try await archive.delete(id)
            failed.remove(id); localMinutesAvailable.remove(id); localMinutesRequested.remove(id)
            retryTasks.removeValue(forKey: id)?.cancel(); retryAttempts[id] = nil; transientFailures.remove(id)
            loadedModelConsents.remove(id)
            if consentRequest?.id == id { consentRequest = nil }
            // Export-Kopien der Detailansicht gehören zur Besprechung und gehen mit.
            try? FileManager.default.removeItem(at: FileManager.default.temporaryDirectory.appendingPathComponent("export-" + id.uuidString))
            try? FileManager.default.removeItem(at: Self.workingAudio(id))
        } catch { self.error = "Die Besprechung konnte nicht gelöscht werden." }
        await refresh(); start()
    }
    func original(_ id: UUID) async throws -> URL {
        guard let archive else { throw VoiceError.missing }
        return try await archive.originalURL(id)
    }
    private func start(userInitiated: Bool = false) {
        #if DEBUG
        // Vorschau „Pausiert“: Zustand stehen lassen, keine Verarbeitung starten.
        if ProcessInfo.processInfo.arguments.contains("--meeting-background-paused") { return }
        #endif
        guard background.allowStart(userInitiated: userInitiated) else { return }
        guard worker == nil, let archive else { return }
        guard let doc = pendingDocument else {
            background.finish(success: !documents.contains { $0.minutes == nil && !$0.requiresMinutesConsent })
            return
        }
        // Keine Fortsetzung im Vordergrund einreichen: die Anfrage entsteht erst beim Wechsel nach .inactive (scene).
        guard background.beginWorker() else { return }
        backgroundPaused = false
        background.update(phase: "Besprechung verarbeiten", subtitle: MeetingListing.title(doc),
                          document: doc.id, units: MeetingProcessing.transcription(offset: doc.nextOffset, duration: doc.duration))
        worker = Task {
            let cancellation = InferenceCancellation(); token = cancellation; running = doc.id
            do {
                try await process(doc, archive: archive, cancellation: cancellation)
                // Erfolg beendet die Fehlerepisode.
                retryAttempts[doc.id] = nil; transientFailures.remove(doc.id)
            } catch {
                if !Task.isCancelled {
                    failed.insert(doc.id)
                    let delay = scheduleRetry(doc.id, after: error)
                    if Self.isTransient(error) { transientFailures.insert(doc.id) } else { transientFailures.remove(doc.id) }
                    if error is ModelFailure {
                        localMinutesAvailable.insert(doc.id)
                        if case ModelFailure.consentMissing = error { await prepareConsentRequest(doc) }
                    }
                    self.error = Self.failureMessage(error, transcribed: doc.nextOffset >= doc.duration, retryIn: delay)
                }
                status = "gespeichert – Verarbeitung folgt"
            }
            background.endWorker(doc.id, completed: !Task.isCancelled, success: !failed.contains(doc.id))
            token = nil; exporter = nil; running = nil; worker = nil
            await refresh(); start()
        }
    }
    /// Zwischendatei der Audiospur je Besprechung (Caches, vom System entfernbar).
    static func workingAudio(_ id: UUID) -> URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("MeetingAudio").appendingPathComponent(id.uuidString + ".m4a")
    }
    private var pendingDocument: MeetingDocument? {
        background.nextDocument(in: pendingDocuments)
    }
    private var pendingDocuments: [MeetingDocument] {
        documents.filter { $0.minutes == nil && !$0.requiresMinutesConsent && !failed.contains($0.id) && !deleting.contains($0.id) }
    }
    private func process(_ original: MeetingDocument, archive: MeetingArchive, cancellation: InferenceCancellation) async throws {
        var doc = original
        if doc.nextOffset < doc.duration {
            let source = try await archive.originalURL(doc.id)
            // Fertige Audiospur bleibt bis zum Ende der Transkription liegen; ein Neustart exportiert nicht erneut.
            let working = Self.workingAudio(doc.id)
            if !FileManager.default.fileExists(atPath: working.path) {
                let partial = working.appendingPathExtension("partial")
                try FileManager.default.createDirectory(at: working.deletingLastPathComponent(), withIntermediateDirectories: true)
                // Ein abgebrochener Export wird nie als vollständige Datei verwendet.
                try? FileManager.default.removeItem(at: partial)
                defer { try? FileManager.default.removeItem(at: partial) }
                guard let session = AVAssetExportSession(asset: AVURLAsset(url: source), presetName: AVAssetExportPresetAppleM4A) else { throw VoiceError.invalid }
                exporter = session; status = "Audiospur vorbereiten"
                background.update(phase: "Audiospur vorbereiten", subtitle: MeetingListing.title(doc),
                                  document: doc.id, units: MeetingProcessing.transcription(offset: doc.nextOffset, duration: doc.duration))
                let audioStart = ContinuousClock.now
                try await session.export(to: partial, as: .m4a)
                try Task.checkCancellation()
                try FileManager.default.moveItem(at: partial, to: working)
                try await archive.recordTiming(doc.id, phase: "audio", milliseconds: elapsed(audioStart))
                exporter = nil
            }
            while doc.nextOffset < doc.duration {
                try Task.checkCancellation()
                let startTime = ContinuousClock.now
                let end = min(doc.nextOffset + 30, doc.duration)
                status = "Lokal transkribieren · \(Int(doc.nextOffset / doc.duration * 100)) %"
                background.update(phase: "Lokal transkribieren", subtitle: MeetingListing.title(doc),
                                  document: doc.id, units: MeetingProcessing.transcription(offset: doc.nextOffset, duration: doc.duration))
                let selected = LocalModel.selectedSTTModel()
                let segments = try await CPULocalProviders.shared.transcribeSlice(working, offset: doc.nextOffset, duration: end - doc.nextOffset, cancellation: cancellation, selectedModel: selected)
                try Task.checkCancellation()
                let shifted = segments.map {
                    MeetingSegment(
                        index: $0.index, start: min(end, doc.nextOffset + $0.start),
                        end: min(end, doc.nextOffset + $0.end), text: $0.text,
                        speaker: $0.speaker, language: $0.language
                    )
                }
                try await archive.appendChunk(
                    doc.id, offset: doc.nextOffset, nextOffset: end, segments: shifted,
                    event: ProcessingEvent(
                        operation: "Transkription · Abschnitt", model: LocalModel.label(for: selected),
                        completedAt: Date(), durationMS: elapsed(startTime), isAI: true
                    )
                )
                try await archive.recordTiming(doc.id, phase: "stt", milliseconds: elapsed(startTime))
                doc = try await archive.load(doc.id); await refresh()
            }
            try? FileManager.default.removeItem(at: working)
        }
        try await createLocalMinutes(doc, archive: archive)
        status = "Transkript und Auswertung gespeichert"
        if !foreground { await notifyReady(doc) }
    }

    /// Ohne Rückfrage: vorläufige Mitteilungen landen still in der Mitteilungszentrale (lva#935).
    private static func allowQuietNotifications() async {
        let center = UNUserNotificationCenter.current()
        if await center.notificationSettings().authorizationStatus == .notDetermined {
            _ = try? await center.requestAuthorization(options: [.alert, .provisional])
        }
    }
    /// Nur der Titel, nie Inhalte aus Transkript oder Protokoll.
    private func notifyReady(_ doc: MeetingDocument) async {
        let center = UNUserNotificationCenter.current()
        let status = await center.notificationSettings().authorizationStatus
        guard status == .authorized || status == .provisional else { return }
        guard readyNotifications.claim(doc.id) else { return }
        let content = UNMutableNotificationContent()
        content.title = "Protokoll fertig"
        content.body = MeetingListing.title(doc)
        try? await center.add(UNNotificationRequest(identifier: "meeting-ready-" + doc.id.uuidString, content: content, trigger: nil))
    }

    private func createLocalMinutes(_ doc: MeetingDocument, archive: MeetingArchive) async throws {
        guard !doc.requiresMinutesConsent else { throw VoiceError.invalid }
        guard let consent = doc.consent,
              doc.minutesSegments().contains(where: { consent.allows(.aiAnalysis, from: $0.start, to: $0.end) }) else {
            try await archive.savePipelineMinutes(doc.id, result: MinutesResult(), event: ProcessingEvent(
                operation: "Auswertung ausgelassen · keine KI-Einwilligung", model: "–", completedAt: Date(), durationMS: 0, isAI: false))
            return
        }
        let choices = await modelCatalog.choices(for: .meetingMinutes)
        let forceLocal = localMinutesRequested.contains(doc.id)
        let key = forceLocal ? nil : doc.minutesRouteKey
        let selected = key.flatMap { stored in choices.first { $0.id == stored } }
        // Auch eine gelöschte oder gesperrte ausdrückliche Wahl bleibt ein klarer Fehler.
        if key != nil && selected == nil { throw ModelFailure.unavailable }
        let candidates = try doc.minutesCandidates(choices: choices, forceLocal: forceLocal)
        guard !candidates.isEmpty else { throw ModelFailure.unavailable }
        try await loadModelConsents(doc.id)
        let id = doc.id
        for (index, choice) in candidates.enumerated() {
            let provider = try modelCatalog.provider(for: choice)
            try Task.checkCancellation()
            status = "Auswerten · \(choice.label)"
            background.update(phase: "Auswerten", subtitle: MeetingListing.title(doc), document: doc.id, units: 800)
            let startTime = ContinuousClock.now
            do {
                try await archive.selectMinutesRoute(id, route: choice.route, explicitLocal: forceLocal)
                localMinutesRequested.remove(id)
                let session = try await ModelGate.open(provider, purpose: .meetingMinutes, subjectId: id.uuidString,
                    dataCategories: Self.minutesDataCategories, state: modelAccessState)
                let pipeline = MinutesPipeline(session: session, contextTokens: choice.contextTokens)
                let result = try await pipeline.run(segments: doc.minutesSegments(), consent: consent,
                    checkpoint: forceLocal ? nil : doc.minutesCheckpoint,
                    onProgress: { [weak self] progress in
                        await self?.minutesProgress(progress, model: choice.label)
                    }, onCheckpoint: { checkpoint in
                        try Task.checkCancellation()
                        try await archive.saveMinutesCheckpoint(id, checkpoint: checkpoint)
                    })
                try Task.checkCancellation()
                try await archive.savePipelineMinutes(id, result: result, event: ProcessingEvent(
                    operation: "Auswertung · \(choice.route.isLocal ? "lokal" : "API") · Map-Reduce", model: choice.label,
                    completedAt: Date(), durationMS: elapsed(startTime), isAI: true, route: provider.route.key))
                try await archive.recordTiming(id, phase: "minutes", milliseconds: elapsed(startTime))
                localMinutesAvailable.remove(id); localMinutesRequested.remove(id)
                return
            } catch ModelFailure.unavailable where forceLocal && index + 1 < candidates.count {
                continue // Die ausdrückliche lokale Aktion erlaubt verfügbare lokale Alternativen.
            }
        }
        throw ModelFailure.unavailable
    }

    private func minutesProgress(_ progress: MinutesProgress, model: String) {
        status = "\(model) · \(progress.phase) · Abschnitt \(progress.section) von \(progress.total)"
        if let doc = documents.first(where: { $0.id == running }) {
            background.update(phase: progress.phase, subtitle: MeetingListing.title(doc),
                              document: doc.id, units: MeetingProcessing.minutes(section: progress.section, total: progress.total))
        }
    }
    /// Zerlegt Text an Wortgrenzen in Teile von höchstens `max` Zeichen, ohne etwas auszulassen.
    static func slices(_ text: String, max: Int) -> [String] {
        var out: [String] = [], current = ""
        for word in text.split(separator: " ") {
            var w = Substring(word)
            while w.count > max { if !current.isEmpty { out.append(current); current = "" }; out.append(String(w.prefix(max))); w = w.dropFirst(max) }
            if w.isEmpty { continue }
            if !current.isEmpty, current.count + 1 + w.count > max { out.append(current); current = String(w) }
            else { current = current.isEmpty ? String(w) : current + " " + w }
        }
        if !current.isEmpty { out.append(current) }
        return out
    }

    static func merge(_ a: MeetingMinutes, _ b: MeetingMinutes) -> MeetingMinutes {
        MeetingMinutes(summary: [a.summary, b.summary].filter { !$0.isEmpty }.joined(separator: "\n"),
                       scope: [a.scope, b.scope].filter { !$0.isEmpty }.joined(separator: "\n"),
                       decisions: a.decisions + b.decisions, tasks: a.tasks + b.tasks, next_steps: a.next_steps + b.next_steps,
                       follow_ups: a.follow_ups + b.follow_ups, open_questions: a.open_questions + b.open_questions)
    }

    private func elapsed(_ start: ContinuousClock.Instant) -> Double {
        let duration = start.duration(to: .now).components
        return Double(duration.seconds) * 1000 + Double(duration.attoseconds) / 1e15
    }
}

private struct MeetingSection<Content: View>: View {
    let title: String
    let content: Content
    init(_ title: String, @ViewBuilder content: () -> Content) { self.title = title; self.content = content() }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.headline)
            content
        }.voiceCard()
    }
}

struct MeetingLibraryView: View {
    @StateObject private var library = MeetingLibrary.shared
    @State private var importer = false
    @State private var recordLive = false
    @ObservedObject private var incoming = IncomingFiles.shared
    /// Datei, die gerade bestätigt wird: aus der Dateiauswahl oder per „Öffnen in“.
    @State private var confirming: PendingImport?
    /// Fehlgeschlagene Übernahmen dieser Sitzung: nicht sofort erneut anbieten (beim nächsten Start wieder).
    @State private var deferred: Set<URL> = []

    /// URLs, die gerade bestätigt, geprüft oder übernommen werden: nie ein zweites Mal anbieten.
    @State private var inFlight: Set<URL> = []

    /// Aus der Dateiauswahl gewählte Dateien. Sie warten wie „Öffnen in“-Kopien in derselben
    /// Reihe und überschreiben nie eine laufende Bestätigung.
    @State private var requests: [PendingImport] = []

    /// Reservierung: die eine Übernahme, die gerade geprüft, bestätigt oder gesichert wird.
    /// Synchron gesetzt, erst nach dem Abschluss (Bestätigen oder Verwerfen) wieder frei.
    @State private var current: PendingImport?
    /// Angezeigter Zustimmungsdialog für eine externe Auswertung.
    @State private var consentShown: ModelConsentRequest?
    @State private var query = ""
    @Environment(\.dynamicTypeSize) private var typeSize
    @State private var filter = MeetingListing.Filter.all
    @State private var renaming: MeetingDocument?
    @State private var renameText = ""
    @State private var deleting: MeetingDocument?

    private var groups: [(title: String, items: [MeetingDocument])] {
        let visible = library.documents.filter {
            MeetingListing.matches($0, filter: filter)
                && MeetingListing.matches(searchText: library.searchIndex[$0.id] ?? MeetingListing.searchText($0), query: query)
        }
        return MeetingListing.grouped(visible, now: Date(), calendar: .current)
    }
    private func beginRename(_ doc: MeetingDocument) {
        renameText = doc.titleIsUser == true ? (doc.title ?? "") : MeetingListing.title(doc)
        renaming = doc
    }

    /// Nächste Übernahme reservieren. Gewählte Dateien zuerst, dann „Öffnen in“-Kopien.
    private func offerNext() {
        guard current == nil, confirming == nil else { return }
        let next: PendingImport
        if !requests.isEmpty {
            next = requests.removeFirst()
        } else if let url = incoming.pending.first(where: { !deferred.contains($0) && !inFlight.contains($0) }) {
            next = PendingImport(url: url, source: .openIn)
        } else { return }
        current = next
        inFlight.insert(next.url)
        Task {
            // Nach einem Absturz kann die Kopie schon übernommen sein: dann nur aufräumen.
            if next.source == .openIn, await library.alreadyHeld(next.url) {
                incoming.finish(next.url, deleteCopy: true)
                return complete(next)
            }
            // Nach dem await prüfen, ob die Reservierung noch gilt und keine Bestätigung offen ist.
            guard current?.id == next.id, confirming == nil else { return complete(next) }
            confirming = next
        }
    }

    /// Reservierung freigeben (jeder Abschluss, jede Quelle) und das Nächste anbieten.
    private func complete(_ done: PendingImport) {
        inFlight.remove(done.url)
        if current?.id == done.id { current = nil }
        offerNext()
    }

    struct PendingImport: Identifiable {
        let url: URL
        let source: InboxItem.Source
        var id: URL { url }
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        (typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(spacing: 10)) : AnyLayout(HStackLayout(spacing: 10))) {
                            Button { recordLive = true } label: {
                                Label("Aufnehmen", systemImage: "record.circle")
                                    .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                            }.buttonStyle(VoicePrimaryButtonStyle(recording: false))
                                .accessibilityLabel("Besprechung aufnehmen").accessibilityIdentifier("recordMeeting")
                            Button { importer = true } label: {
                                Label("Importieren", systemImage: "square.and.arrow.down")
                                    .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                            }.buttonStyle(VoicePrimaryButtonStyle(recording: false))
                                .disabled(library.importBusy).opacity(library.importBusy ? 0.6 : 1)
                                .accessibilityLabel("Audio oder Video importieren").accessibilityIdentifier("importMeeting")
                        }
                        if library.importBusy { ProgressView("Original sichern") }
                        if let error = library.error {
                            Text(error).font(.subheadline)
                            if let request = library.consentRequest {
                                Button("Zustimmung für \(request.target.vendor) …", systemImage: "hand.raised") { consentShown = request }
                                    .frame(minHeight: 44).accessibilityIdentifier("minutesConsent")
                            }
                            if let id = library.localMinutesAvailable.first {
                                Button("Auf dem iPhone erstellen", systemImage: "iphone") { library.createMinutesLocally(id) }
                                    .frame(minHeight: 44).accessibilityIdentifier("minutesLocal")
                            }
                            Button("Erneut versuchen", systemImage: "arrow.clockwise") { library.retry() }.frame(minHeight: 44)
                        }
                    }.buttonStyle(.borderless)
                } footer: {
                    Text("Anrufaufzeichnungen aus Dateien importieren. Kein Live-Zugriff auf Telefon-, Teams- oder WhatsApp-Anrufe.")
                }
                if !library.documents.isEmpty {
                    MeetingFilterBar(selection: $filter)
                        .listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
                }
                if library.documents.isEmpty {
                    Section { Text("Noch keine Besprechung. Nimm eine auf oder importiere eine Datei.").foregroundStyle(VoicePalette.secondaryText) }
                } else if groups.isEmpty {
                    Section { Text(query.isEmpty ? "Keine Besprechung in dieser Auswahl." : "Keine Treffer für „\(query)“.").foregroundStyle(VoicePalette.secondaryText) }
                }
                ForEach(groups.map { MeetingGroup(title: $0.title, items: $0.items) }) { group in
                    Section(group.title) {
                        ForEach(group.items) { doc in
                            NavigationLink { MeetingDetailView(library: library, original: doc) } label: {
                                MeetingRow(doc: doc, status: library.listingStatus(doc), paused: library.backgroundPaused,
                                           detail: library.running == doc.id ? library.status : nil)
                            }
                            .accessibilityIdentifier("meetingEntry")
                            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                Button("Löschen", systemImage: "trash", role: .destructive) { deleting = doc }.tint(VoicePalette.danger)
                                Button("Umbenennen", systemImage: "pencil") { beginRename(doc) }.tint(Color(red: 0, green: 0.36, blue: 0.75))
                            }
                            .contextMenu {
                                Button("Umbenennen", systemImage: "pencil") { beginRename(doc) }
                                ShareLink(item: MeetingExport.text(doc), preview: SharePreview(MeetingListing.title(doc))) {
                                    Label("Teilen", systemImage: "square.and.arrow.up")
                                }
                                Button("Löschen", systemImage: "trash", role: .destructive) { deleting = doc }
                            }
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(VoicePalette.background)
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .automatic), prompt: "Titel, Person oder Inhalt")
            .navigationTitle("Transkripte").navigationBarTitleDisplayMode(.inline)
            .alert("Besprechung umbenennen", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
                TextField("Titel", text: $renameText).accessibilityIdentifier("meetingTitleField")
                Button("Abbrechen", role: .cancel) { renaming = nil }
                Button("Sichern") {
                    if let doc = renaming { Task { await library.rename(doc.id, to: renameText) } }
                    renaming = nil
                }
            } message: { Text("Leer lassen für den automatischen Titel.") }
            .confirmationDialog(deleting.map { "„\(MeetingListing.title($0))“ löschen?" } ?? "Löschen?",
                                isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
                Button("Löschen", role: .destructive) {
                    if let doc = deleting { Task { await library.delete(doc.id) } }
                    deleting = nil
                }
            } message: { Text("Original, Transkript und Protokoll werden vom iPhone entfernt. Das lässt sich nicht rückgängig machen.") }
            .sheet(item: $consentShown) { request in
                ModelConsentSheet(choice: request.choice, scope: request.scope, target: request.target,
                                  localAvailable: request.localAvailable) { decision in
                    consentShown = nil
                    Task { await library.decide(decision, for: request) }
                }
            }
            .toolbar { ToolbarItem(placement: .topBarTrailing) { SettingsButton() } }
            .fullScreenCover(isPresented: $recordLive) { LiveMeetingScreen(library: library) }
            .fileImporter(isPresented: $importer, allowedContentTypes: [.audio, .movie], allowsMultipleSelection: false) { result in
                switch result {
                case .success(let urls):
                    if let first = urls.first, !inFlight.contains(first), !requests.contains(where: { $0.url == first }) {
                        requests.append(PendingImport(url: first, source: .files))
                    }
                    offerNext()
                case .failure: library.error = "Datei konnte nicht geöffnet werden."
                }
            }
        }
        .sheet(item: $confirming) { pending in
            ImportConsentView(fileName: pending.url.lastPathComponent, channel: .import, onConfirm: { record in
                confirming = nil
                Task {
                    let saved = await library.deposit(pending.url, from: pending.source, consent: record)
                    // Kopie nur nach dauerhaftem Eingangs-Commit löschen; sonst bleibt sie für den nächsten Versuch.
                    if pending.source == .openIn, saved { incoming.finish(pending.url, deleteCopy: true) }
                    else if pending.source == .openIn { deferred.insert(pending.url) }
                    complete(pending)
                }
            }, onCancel: {
                confirming = nil
                // Ausdrücklich verworfen.
                if pending.source == .openIn { incoming.finish(pending.url, deleteCopy: true) }
                complete(pending)
            }).interactiveDismissDisabled()
        }
        .onAppear { incoming.rescan() }
        .onChange(of: incoming.pending, initial: true) { _, _ in offerNext() }
        .onChange(of: confirming == nil) { _, closed in if closed { offerNext() } }
        #if DEBUG
        .task { await library.importProbe() }
        #endif
    }
}

private struct MeetingDetailView: View {
    @ObservedObject var library: MeetingLibrary
    let original: MeetingDocument
    @State private var files: [URL] = []
    @State private var originalURL: URL?
    @State private var copied = false
    @State private var exportError = false
    @State private var tab = 0
    @State private var renaming = false
    @State private var renameText = ""
    @State private var confirmDelete = false
    @Environment(\.dismiss) private var dismiss
    private var doc: MeetingDocument { library.documents.first(where: { $0.id == original.id }) ?? original }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    MeetingHeader(doc: doc)
                    HStack {
                        Button(copied ? "Kopiert" : "Kopieren", systemImage: copied ? "checkmark" : "doc.on.doc") {
                            let text = MeetingExport.text(doc)
                            UIPasteboard.general.items = [[UTType.utf8PlainText.identifier: text, UTType.html.identifier: Data(MeetingExport.html(text).utf8)]]
                            copied = true
                        }.frame(minHeight: 44)
                        Spacer()
                        Menu("Exportieren", systemImage: "square.and.arrow.up") {
                            ForEach(files, id: \.self) { file in ShareLink(file.pathExtension.uppercased(), item: file) }
                            if let originalURL { ShareLink("Originaldatei", item: originalURL) }
                        }.disabled(files.isEmpty).frame(minHeight: 44)
                    }.buttonStyle(.borderless)
                    if exportError { Text("Export konnte nicht vorbereitet werden.").font(.caption).foregroundStyle(VoicePalette.secondaryText) }
                }.voiceCard()
                ProcessingDisclosure(events: doc.processingEvents ?? [])
                Picker("Ergebnisansicht", selection: $tab) {
                    Text("Auswertung").tag(0)
                    Text("Transkript").tag(1)
                }.pickerStyle(.segmented)
                if tab == 0 { analysis }
                else {
                    if doc.segments.isEmpty { Text("gespeichert – Verarbeitung folgt").foregroundStyle(VoicePalette.secondaryText) }
                    ForEach(doc.segments, id: \.index) { item in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(Duration.seconds(item.start).formatted(.time(pattern: .minuteSecond))).font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            Text(item.text).textSelection(.enabled)
                        }.voiceCard()
                    }
                    Text("Keine belegte Sprecherzuordnung: Für gemischte Aufnahmen werden keine Redeanteile geschätzt.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                }
                DisclosureGroup("Verarbeitungszeiten") {
                    ForEach([("import", "Import"), ("audio", "Audiospur"), ("stt", "Transkription"), ("minutes", "Auswertung")], id: \.0) { key, label in
                        LabeledContent(label, value: String(format: "%.1f s", (doc.timings?[key] ?? 0) / 1000))
                    }.font(.caption)
                }.font(.subheadline).voiceCard()
            }.padding(16)
        }
        .background(VoicePalette.background)
        .navigationTitle(MeetingListing.title(doc)).navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu("Verwalten", systemImage: "ellipsis.circle") {
                    Button("Umbenennen", systemImage: "pencil") {
                        renameText = doc.titleIsUser == true ? (doc.title ?? "") : MeetingListing.title(doc); renaming = true
                    }
                    Button("Löschen", systemImage: "trash", role: .destructive) { confirmDelete = true }
                }.accessibilityIdentifier("meetingManage")
            }
        }
        .alert("Besprechung umbenennen", isPresented: $renaming) {
            TextField("Titel", text: $renameText)
            Button("Abbrechen", role: .cancel) {}
            Button("Sichern") { let id = doc.id, text = renameText; Task { await library.rename(id, to: text) } }
        } message: { Text("Leer lassen für den automatischen Titel.") }
        .confirmationDialog("„\(MeetingListing.title(doc))“ löschen?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Löschen", role: .destructive) {
                let id = doc.id
                dismiss()
                Task { await library.delete(id) }
            }
        } message: { Text("Original, Transkript und Protokoll werden vom iPhone entfernt. Das lässt sich nicht rückgängig machen.") }
        .task(id: "\(doc.nextOffset):\(doc.summarizedSegments ?? 0):\(doc.minutes != nil)") { await prepareExports() }
    }
    @ViewBuilder private var analysis: some View {
        if let report = doc.minutes {
            MeetingSection("Zusammenfassung") { Text(report.summary.isEmpty ? "Keine Sprache erkannt." : report.summary).textSelection(.enabled) }
            if !report.scope.isEmpty && !report.summary.contains(report.scope) {
                MeetingSection("Kontext") { Text(report.scope).textSelection(.enabled) }
            }
            if !report.decisions.isEmpty {
                MeetingSection("Entscheidungen") { ForEach(Array(report.decisions.enumerated()), id: \.offset) { _, item in Text(item.text + (item.context.isEmpty ? "" : "\n" + item.context)).textSelection(.enabled) } }
            }
            if !report.tasks.isEmpty {
                MeetingSection("Aufgaben") { ForEach(Array(report.tasks.enumerated()), id: \.offset) { _, item in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.text).textSelection(.enabled)
                        Text("\(item.assignee ?? "Zuständig: offen") · \(item.due ?? "Termin: offen")").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    }
                } }
            }
            if !report.next_steps.isEmpty {
                MeetingSection("Nächste Schritte") { ForEach(Array(report.next_steps.enumerated()), id: \.offset) { _, item in Text(item.text + (item.owner.map { " · " + $0 } ?? "")).textSelection(.enabled) } }
            }
            if !report.follow_ups.isEmpty {
                MeetingSection("Handlungsempfehlungen") { ForEach(Array(report.follow_ups.enumerated()), id: \.offset) { _, item in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.text).textSelection(.enabled)
                        Text("Bezug: " + item.reason).font(.caption).foregroundStyle(VoicePalette.secondaryText).textSelection(.enabled)
                    }
                } }
            }
            if !report.open_questions.isEmpty {
                MeetingSection("Offene Fragen") { ForEach(Array(report.open_questions.enumerated()), id: \.offset) { _, item in Text(item.text).textSelection(.enabled) } }
            }
            Text("Aus belegten Originalstellen erstellt. Handlungsempfehlungen sind Vorschläge. Das Transkript bleibt zur Prüfung erhalten.").font(.caption).foregroundStyle(VoicePalette.secondaryText)
        } else if doc.requiresMinutesConsent {
            Text("Einwilligungsnachweis fehlt. Vorhandene Teilauswertungen bleiben erhalten; keine neue KI-Auswertung.")
                .foregroundStyle(VoicePalette.secondaryText).voiceCard()
            ForEach(Array((doc.summaryParts ?? []).enumerated()), id: \.offset) { _, part in
                MeetingSection("Gespeicherte Teilauswertung") { Text(part.summary).textSelection(.enabled) }
            }
        } else { Text("gespeichert – Verarbeitung folgt").foregroundStyle(VoicePalette.secondaryText).voiceCard() }
    }
    private func prepareExports() async {
        copied = false; files = []; exportError = false
        do {
            let folder = FileManager.default.temporaryDirectory.appendingPathComponent("export-" + doc.id.uuidString)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let text = MeetingExport.text(doc)
            let values: [(String, Data)] = [("txt", Data(text.utf8)), ("html", Data(MeetingExport.html(text).utf8)), ("srt", Data(MeetingExport.srt(doc.segments).utf8)), ("json", try JSONEncoder().encode(doc))]
            var result: [URL] = []
            for (ext, data) in values {
                let url = folder.appendingPathComponent("Ergebnisse." + ext)
                try data.write(to: url, options: .atomic); result.append(url)
            }
            let original = try await library.original(doc.id)
            try Task.checkCancellation()
            originalURL = original; files = result; exportError = false
        } catch { if !Task.isCancelled { exportError = true } }
    }
}

/// Offene Zustimmung zu einer externen Protokoll-Auswertung (lva#801 T3b). Keine Inhalte, nur Bezug und Ziel.
struct ModelConsentRequest: Identifiable {
    let id: UUID
    let title: String
    let choice: ModelChoice
    let scope: ModelConsentScope
    let target: ModelResolvedTarget
    let localAvailable: Bool
}
