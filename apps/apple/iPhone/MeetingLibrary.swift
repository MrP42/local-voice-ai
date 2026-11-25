import SwiftUI
import AVFoundation
import UniformTypeIdentifiers

@MainActor
final class MeetingLibrary: ObservableObject {
    @Published private(set) var documents: [MeetingDocument] = []
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
    private var token: InferenceCancellation?
    private var exporter: AVAssetExportSession?
    private var failed = Set<UUID>()
    private var active = false
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
    func scene(active: Bool) {
        self.active = active
        if active { Task { await recoverLive(); await drainInbox(); await refresh(); start() } }
        else {
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
                                                 deferredRefinement: session.deferredRefinement)
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
    func retry() { failed.removeAll(); error = nil; start() }
    /// T3 kann diese Aktion nach einem Fehler anbieten. Nur eine ausdrückliche Aktion wechselt lokal.
    func createMinutesLocally(_ id: UUID) {
        guard localMinutesAvailable.contains(id) else { return }
        localMinutesRequested.insert(id); localMinutesAvailable.remove(id)
        failed.remove(id); error = nil; start()
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
        consentRequest = ModelConsentRequest(id: doc.id, title: doc.originalName, choice: choice, scope: resolved.0, target: resolved.1,
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
    func original(_ id: UUID) async throws -> URL {
        guard let archive else { throw VoiceError.missing }
        return try await archive.originalURL(id)
    }
    private func start() {
        guard active, worker == nil, let archive,
              let doc = documents.first(where: { $0.minutes == nil && !$0.requiresMinutesConsent && !failed.contains($0.id) }) else { return }
        worker = Task {
            let cancellation = InferenceCancellation(); token = cancellation; running = doc.id
            do { try await process(doc, archive: archive, cancellation: cancellation) }
            catch {
                if !Task.isCancelled {
                    failed.insert(doc.id)
                    if error is ModelFailure {
                        localMinutesAvailable.insert(doc.id)
                        if case ModelFailure.consentMissing = error { await prepareConsentRequest(doc) }
                        self.error = "\(ModelFailure.sanitized(error).localizedDescription) Original und Fortschritt sind gespeichert. Lokal erstellen ist möglich."
                    } else {
                        self.error = "Die lokale Verarbeitung ist noch nicht abgeschlossen. Original und Fortschritt sind gespeichert. Lokale Modelle prüfen und erneut versuchen."
                    }
                }
                status = "gespeichert – Verarbeitung folgt"
            }
            token = nil; exporter = nil; running = nil; worker = nil
            await refresh(); start()
        }
    }
    private func process(_ original: MeetingDocument, archive: MeetingArchive, cancellation: InferenceCancellation) async throws {
        var doc = original
        if doc.nextOffset < doc.duration {
            let source = try await archive.originalURL(doc.id)
            let working = FileManager.default.temporaryDirectory.appendingPathComponent("meeting-" + doc.id.uuidString + ".m4a")
            // A cancelled conversion is never reused as a complete audio file.
            try? FileManager.default.removeItem(at: working)
            defer { try? FileManager.default.removeItem(at: working) }
            guard let session = AVAssetExportSession(asset: AVURLAsset(url: source), presetName: AVAssetExportPresetAppleM4A) else { throw VoiceError.invalid }
            exporter = session; status = "Audiospur vorbereiten"
            let audioStart = ContinuousClock.now
            try await session.export(to: working, as: .m4a)
            try Task.checkCancellation()
            try await archive.recordTiming(doc.id, phase: "audio", milliseconds: elapsed(audioStart))
            exporter = nil
            while doc.nextOffset < doc.duration {
                try Task.checkCancellation()
                let startTime = ContinuousClock.now
                let end = min(doc.nextOffset + 30, doc.duration)
                status = "Lokal transkribieren · \(Int(doc.nextOffset / doc.duration * 100)) %"
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
        }
        try await createLocalMinutes(doc, archive: archive)
        status = "Transkript und Auswertung gespeichert"
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
    @StateObject private var library = MeetingLibrary()
    @Environment(\.scenePhase) private var phase
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
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    VStack(alignment: .leading, spacing: 10) {
                        Button { recordLive = true } label: {
                            Label("Besprechung aufnehmen", systemImage: "record.circle")
                                .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                        }.buttonStyle(VoicePrimaryButtonStyle(recording: false))
                            .accessibilityIdentifier("recordMeeting")
                        Button { importer = true } label: {
                            Label("Audio oder Video importieren", systemImage: "square.and.arrow.down")
                                .frame(maxWidth: .infinity, minHeight: 44).foregroundStyle(VoicePalette.ink)
                        }.buttonStyle(VoicePrimaryButtonStyle(recording: false))
                            .disabled(library.importBusy).opacity(library.importBusy ? 0.6 : 1)
                            .accessibilityIdentifier("importMeeting")
                        if library.importBusy { ProgressView("Original sichern") }
                        Text(library.status).font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
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
                    }.voiceCard()
                    Text("Anrufaufzeichnungen aus Dateien importieren. Kein Live-Zugriff auf Telefon-, Teams- oder WhatsApp-Anrufe.")
                        .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                    Text("Gespeicherte Transkripte").font(.headline)
                    if library.documents.isEmpty { Text("Noch keine Aufzeichnung").foregroundStyle(VoicePalette.secondaryText) }
                    ForEach(library.documents) { doc in
                        NavigationLink { MeetingDetailView(library: library, original: doc) } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(doc.originalName).lineLimit(2)
                                    Text(doc.requiresMinutesConsent ? "Einwilligungsnachweis fehlt · Bestand erhalten" :
                                         doc.minutes != nil ? "Transkript & Auswertung" : "gespeichert – Verarbeitung folgt")
                                        .font(.caption).foregroundStyle(VoicePalette.secondaryText)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                            }.voiceCard()
                        }.buttonStyle(.plain).accessibilityIdentifier("meetingEntry")
                    }
                }.padding(16)
            }
            .background(VoicePalette.background)
            .navigationTitle("Transkripte").navigationBarTitleDisplayMode(.inline)
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
        .onChange(of: phase, initial: true) { _, value in library.scene(active: value == .active) }
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
    private var doc: MeetingDocument { library.documents.first(where: { $0.id == original.id }) ?? original }
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(doc.originalName).font(.headline).lineLimit(2)
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
        .navigationTitle("Ergebnisse").navigationBarTitleDisplayMode(.inline)
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
