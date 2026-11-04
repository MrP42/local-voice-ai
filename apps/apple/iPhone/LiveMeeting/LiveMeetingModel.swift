import SwiftUI
import UIKit

/// Laufende Besprechung: Aufnahme, Live-Transkript, Live-Stand, Einwilligung,
/// dauerhafte Sicherung für den Wiederanlauf (Epic #247, P3).
@MainActor
final class LiveMeetingModel: ObservableObject {
    /// `awaitingConsent`: Person kommt hinzu, Aufnahme steht bis zur neuen Bestätigung.
    /// `savingConsent`: Bestätigung wird dauerhaft gesichert; nichts ist bedienbar.
    enum Phase: Equatable { case idle, starting, recording, paused, awaitingConsent, savingConsent, stopping, finished }

    /// Gesichert nach jedem fertigen Segment und jedem Takt (`sitzung.json`).
    struct SessionFile: Codable, Sendable {
        var id: UUID
        var title: String
        var startedAt: Date
        var consent: ConsentLog
        var live: LiveState
        var segments: [LiveSegment]
        var notes: String
        var seconds: Double
        /// Audiostellen, an denen die Aufnahme pausiert oder unterbrochen wurde (Lücken im Gespräch).
        var breaks: [Double]? = nil

        /// Ohne Inhalt: kein Segment, keine Notiz. Nur solche Sitzungen dürfen verworfen werden.
        var isEmpty: Bool { segments.isEmpty && notes.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    @Published private(set) var phase: Phase = .idle
    @Published private(set) var segments: [LiveSegment] = []
    @Published private(set) var volatile = ""
    @Published private(set) var live = LiveState()
    @Published private(set) var seconds: Double = 0
    @Published private(set) var aiStatus = ""
    @Published private(set) var lastTick: Date?
    @Published private(set) var consent = ConsentLog()
    @Published private(set) var hint: String?
    @Published private(set) var breaks: [Double] = []
    @Published var notes = ""
    @Published var title = ""
    @Published var error: String?
    @Published var jumpTo: String?

    private var capture: LiveCapture?
    private var session: SessionFile?
    private var timer: Timer?
    private var beats = 0
    private var tickTask: Task<Void, Never>?
    private var notesSave: Task<Void, Never>?
    private let writer = SessionWriter()
    private var revision = 0
    private var lastTickAudio: Double?
    private let policy = LiveTickPolicy()
    private let finish: (URL, SessionFile) async -> Void

    static let runningRoot = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Laufend", isDirectory: true)
    /// Sitzung, die gerade aufnimmt oder übernommen wird; die Wiederherstellung lässt sie in Ruhe.
    static private(set) var activeId: UUID?

    init(finish: @escaping (URL, SessionFile) async -> Void) { self.finish = finish }

    var aiAllowed: Bool { consent.allows(.aiAnalysis, at: seconds) }

    // MARK: Start, Pause, Person kommt hinzu, Ende

    func start(title: String, consent record: ConsentRecord) async {
        guard phase == .idle, Self.activeId == nil else { return }
        var log = ConsentLog()
        do { try log.append(record) } catch { self.error = "Einwilligung unvollständig. Aufnahme und Transkription, Zweck und Verantwortlichen angeben."; return }
        guard log.canRecord else { self.error = "Einwilligung unvollständig."; return }
        let id = UUID()
        // Vor dem ersten `await`: die Wiederherstellung darf diese Sitzung nicht übernehmen.
        Self.activeId = id
        phase = .starting
        let dir = Self.runningRoot.appendingPathComponent(id.uuidString, isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let capture = LiveCapture(fileURL: dir.appendingPathComponent(Self.fileName(title) + ".caf"))
            capture.onUpdate = { [weak self] u in self?.receive(u) }
            capture.onError = { [weak self] e in self?.error = e.localizedDescription }
            capture.onInterrupted = { [weak self] why in self?.interrupted(why) }
            capture.onLimit = { [weak self] in self?.limitReached() }
            self.consent = log; self.title = title
            session = SessionFile(id: id, title: title, startedAt: Date(), consent: log, live: LiveState(), segments: [], notes: "", seconds: 0)
            // Erst wenn der Nachweis gesichert ist, darf aufgenommen werden.
            try await save()
            self.capture = capture
            try await capture.start()
            phase = .recording
            UIApplication.shared.isIdleTimerDisabled = true
            startTimer()
        } catch {
            self.error = error.localizedDescription
            self.capture = nil
            session = nil
            try? FileManager.default.removeItem(at: dir)
            Self.activeId = nil
            phase = .idle
        }
    }

    func pause() {
        guard phase == .recording else { return }
        capture?.pause(); phase = .paused; markBreak(); checkpoint()
    }

    func resume() {
        guard phase == .paused else { return }
        do { try capture?.resume(); hint = nil; phase = .recording } catch { self.error = error.localizedDescription }
    }

    /// Person kommt hinzu: Aufnahme anhalten. Fortsetzen geht danach nur über eine neue
    /// Bestätigung (`confirmJoin`), nicht über „Weiter“. Ein laufender KI-Takt wird abgebrochen.
    func requestJoin() {
        guard phase == .recording || phase == .paused else { return }
        if phase == .recording { capture?.pause(); markBreak() }
        tickTask?.cancel()
        phase = .awaitingConsent
        hint = "Person kommt hinzu: Aufnahme angehalten, bis die Einwilligung bestätigt ist."
        checkpoint()
    }

    /// Neue Bestätigung ab `audioPosition`; fortgesetzt wird erst nach dauerhaft gesichertem Nachweis.
    func confirmJoin(_ record: ConsentRecord) async {
        guard phase == .awaitingConsent else { return }
        let before = consent
        do { try consent.append(record) } catch { self.error = "Einwilligung unvollständig. Aufnahme bleibt angehalten."; return }
        phase = .savingConsent
        do { try await save() } catch {
            consent = before
            phase = .awaitingConsent
            self.error = "Der Einwilligungsnachweis konnte nicht gespeichert werden. Aufnahme bleibt angehalten."
            return
        }
        phase = .paused
        resume()
    }

    /// Audiostelle für eine neue Einwilligung („Person kommt hinzu“).
    var audioPosition: Double { capture?.seconds ?? seconds }

    func stop() async {
        guard let capture, [.recording, .paused, .awaitingConsent].contains(phase) else { return }
        phase = .stopping
        timer?.invalidate(); timer = nil
        await capture.stop()
        // Laufenden Takt abwarten, damit sein Ergebnis mitgesichert wird.
        await tickTask?.value
        tickTask = nil
        notesSave?.cancel(); notesSave = nil
        seconds = capture.seconds
        do { try await save() } catch { self.error = "Der letzte Stand konnte nicht gesichert werden; die Aufnahme wird trotzdem übernommen." }
        UIApplication.shared.isIdleTimerDisabled = false
        if let session { await finish(capture.fileURL, session) }
        self.capture = nil
        Self.activeId = nil
        phase = .finished
    }

    // MARK: Nutzeraktionen im Live-Stand

    func confirmDone(_ id: String) { live.confirmDone(id); checkpoint() }
    func rejectAnswer(_ id: String) { live.rejectAnswer(id); checkpoint() }

    /// Notizen: gebündelt sichern, nicht bei jedem Tastendruck.
    func saveNotes() {
        notesSave?.cancel()
        notesSave = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            self?.checkpoint()
        }
    }

    func time(of segmentId: String) -> Double? { segments.first { $0.id == segmentId }?.start }

    // MARK: Intern

    private func receive(_ u: LiveCapture.Update) {
        if !u.finals.isEmpty { segments += u.finals; volatile = ""; checkpoint() } else { volatile = u.volatile }
    }

    /// Die Aufnahme ist im Aufnahmepfad schon gesperrt; hier nur Zustand und Anzeige.
    private func interrupted(_ why: String) {
        guard phase == .recording else { return }
        capture?.pause()
        phase = .paused
        hint = why
        markBreak()
        checkpoint()
    }

    private func limitReached() {
        guard [.recording, .paused, .awaitingConsent].contains(phase) else { return }
        hint = "Höchstdauer von zwei Stunden erreicht. Die Aufnahme wurde beendet."
        Task { await stop() }
    }

    private func markBreak() {
        let at = capture?.seconds ?? seconds
        if breaks.last != at { breaks.append(at) }
    }

    private func startTimer() {
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.clock() }
        }
    }

    /// Jede Sekunde die Uhr, jede fünfte Sekunde der Takt-Entscheid.
    private func clock() {
        guard let capture else { return }
        seconds = capture.seconds
        beats += 1
        if beats % 5 == 0 { heartbeat() }
    }

    private func heartbeat() {
        guard phase == .recording else { return }
        // Nur Abschnitte, für die zur jeweiligen Audiostelle eine KI-Einwilligung vorliegt.
        let pending = segments.filter { $0.end > live.processedThrough && consent.allows(.aiAnalysis, from: $0.start, to: $0.end) }
        let words = pending.reduce(0) { $0 + $1.text.split(separator: " ").count }
        let decision = policy.decide(now: seconds, lastRun: lastTickAudio, newWords: words, running: tickTask != nil,
                                     thermal: LiveThermal(ProcessInfo.processInfo.thermalState),
                                     lowPower: ProcessInfo.processInfo.isLowPowerModeEnabled,
                                     foreground: UIApplication.shared.applicationState == .active,
                                     aiAllowed: aiAllowed && LiveSummarizer.available)
        switch decision {
        case .paused(let why):
            aiStatus = LiveSummarizer.available ? (why == "KI pausiert (Wärme)" ? "KI pausiert – iPhone zu warm" : why) : "KI nicht verfügbar"
        case .wait: if aiStatus.hasPrefix("KI pausiert") { aiStatus = "" }
        case .run:
            guard !pending.isEmpty else { return }
            let version = consent.version
            tickTask = Task { [weak self] in
                await self?.tick(pending: pending, consentVersion: version)
                self?.tickTask = nil
            }
        }
    }

    private func tick(pending: [LiveSegment], consentVersion: Int) async {
        lastTickAudio = seconds
        // Gezielt ältere Belege: Segmente, auf die offene Einträge verweisen.
        let wanted = Set(live.items.filter { $0.status != .done }.flatMap { $0.evidence.map(\.segmentId) })
        let older = segments.filter { wanted.contains($0.id) && $0.end <= live.processedThrough && consent.allows(.aiAnalysis, from: $0.start, to: $0.end) }.suffix(12)
        do {
            let (delta, prompt) = try await LiveSummarizer.tick(state: live, pending: pending, older: Array(older))
            // Einwilligung inzwischen geändert oder Takt abgebrochen: Ergebnis verwerfen.
            guard !Task.isCancelled, consentVersion == consent.version, aiAllowed else { aiStatus = "KI-Takt verworfen (Einwilligung geändert)"; return }
            _ = live.apply(delta, segments: prompt.segments)
            lastTick = Date(); aiStatus = ""
            checkpoint()
        } catch {
            aiStatus = "KI-Takt übersprungen"
        }
    }

    // MARK: Sicherung (seriell, außerhalb des MainActors)

    private func snapshot() -> (SessionFile, Int, URL)? {
        guard var s = session else { return nil }
        s.live = live; s.segments = segments; s.notes = notes; s.consent = consent; s.title = title
        s.seconds = capture?.seconds ?? seconds
        s.breaks = breaks.isEmpty ? nil : breaks
        session = s
        revision += 1
        return (s, revision, Self.runningRoot.appendingPathComponent(s.id.uuidString).appendingPathComponent("sitzung.json"))
    }

    /// Wartet, bis der aktuelle Stand auf dem Datenträger liegt; Fehler werden weitergegeben.
    private func save() async throws {
        guard let (s, r, url) = snapshot() else { return }
        try await writer.write(s, revision: r, to: url)
    }

    /// Checkpoint ohne Warten. Scheitert er, wird die Aufnahme pausiert statt still weiterzulaufen.
    private func checkpoint() {
        guard let (s, r, url) = snapshot() else { return }
        Task { [weak self, writer] in
            do { try await writer.write(s, revision: r, to: url) } catch { self?.saveFailed() }
        }
    }

    private func saveFailed() {
        error = "Der Stand konnte nicht gesichert werden (Speicher voll?). Die Aufnahme ist pausiert."
        if phase == .recording { capture?.pause(); phase = .paused; markBreak() }
    }

    static func fileName(_ title: String) -> String {
        let cleaned = title.components(separatedBy: CharacterSet(charactersIn: "/\\:?*\"<>|").union(.newlines)).joined(separator: " ")
            .trimmingCharacters(in: .whitespaces)
        return cleaned.isEmpty ? "Besprechung" : String(cleaned.prefix(80))
    }

    /// Abgebrochene Sitzungen (App beendet, Absturz): Audio und Stand liegen vor.
    static func abandoned() -> [(URL, SessionFile)] {
        let fm = FileManager.default
        return ((try? fm.contentsOfDirectory(at: runningRoot, includingPropertiesForKeys: nil)) ?? []).compactMap { dir in
            guard UUID(uuidString: dir.lastPathComponent) != activeId,
                  let data = try? Data(contentsOf: dir.appendingPathComponent("sitzung.json")),
                  let s = try? JSONDecoder().decode(SessionFile.self, from: data),
                  let audio = try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil).first(where: { $0.pathExtension == "caf" }) else { return nil }
            return (audio, s)
        }
    }
}

/// Schreibt `sitzung.json` seriell und dauerhaft: temporäre Datei schreiben und per
/// fsync sichern, umbenennen, dann das Verzeichnis per fsync sichern. Ältere Stände
/// (niedrigere Revision) überschreiben nie neuere.
actor SessionWriter {
    private var written = 0
    func write(_ s: LiveMeetingModel.SessionFile, revision: Int, to url: URL) throws {
        guard revision > written else { return }
        let data = try JSONEncoder().encode(s)
        let dir = url.deletingLastPathComponent()
        let tmp = dir.appendingPathComponent(".sitzung-\(revision).tmp")
        defer { try? FileManager.default.removeItem(at: tmp) }
        try data.write(to: tmp, options: [.completeFileProtectionUntilFirstUserAuthentication])
        try Self.fullSync(tmp)
        guard rename(tmp.path, url.path) == 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }
        try Self.fullSync(dir)
        written = revision
    }

    /// F_FULLFSYNC: bis auf den Datenträger, nicht nur in den Laufwerkscache.
    private static func fullSync(_ url: URL) throws {
        let fd = open(url.path, O_RDONLY)
        guard fd >= 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }
        defer { close(fd) }
        guard fcntl(fd, F_FULLFSYNC) == 0 || fsync(fd) == 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }
    }
}
