import SwiftUI
import UIKit

/// Laufende Besprechung: Aufnahme, Live-Transkript, Live-Stand, Einwilligung,
/// dauerhafte Sicherung für den Wiederanlauf (Epic #247, P3).
@MainActor
final class LiveMeetingModel: ObservableObject {
    enum Phase: Equatable { case idle, recording, paused, stopping, finished }

    /// Gesichert nach jedem fertigen Segment und jedem Takt (`sitzung.json`).
    struct SessionFile: Codable {
        var id: UUID
        var title: String
        var startedAt: Date
        var consent: ConsentLog
        var live: LiveState
        var segments: [LiveSegment]
        var notes: String
        var seconds: Double
    }

    @Published private(set) var phase: Phase = .idle
    @Published private(set) var segments: [LiveSegment] = []
    @Published private(set) var volatile = ""
    @Published private(set) var live = LiveState()
    @Published private(set) var seconds: Double = 0
    @Published private(set) var aiStatus = ""
    @Published private(set) var lastTick: Date?
    @Published private(set) var consent = ConsentLog()
    @Published var notes = ""
    @Published var title = ""
    @Published var error: String?
    @Published var jumpTo: String?

    private var capture: LiveCapture?
    private var session: SessionFile?
    private var timer: Timer?
    private var ticking = false
    private var lastTickAudio: Double?
    private let policy = LiveTickPolicy()
    private let finish: (URL, SessionFile) async -> Void

    static let runningRoot = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Laufend", isDirectory: true)
    /// Sitzung, die gerade aufnimmt; die Wiederherstellung lässt sie in Ruhe.
    static private(set) var activeId: UUID?

    init(finish: @escaping (URL, SessionFile) async -> Void) { self.finish = finish }

    var aiAllowed: Bool { consent.allows(.aiAnalysis, at: seconds) }

    // MARK: Start, Pause, Person kommt hinzu, Ende

    func start(title: String, consent record: ConsentRecord) async {
        guard phase == .idle else { return }
        var log = ConsentLog()
        do { try log.append(record) } catch { self.error = "Einwilligung unvollständig. Bitte Zweck und Verantwortlichen angeben."; return }
        let id = UUID()
        let dir = Self.runningRoot.appendingPathComponent(id.uuidString, isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let name = Self.fileName(title)
            let capture = LiveCapture(fileURL: dir.appendingPathComponent(name + ".caf"))
            capture.onUpdate = { [weak self] u in self?.receive(u) }
            capture.onError = { [weak self] e in self?.error = e.localizedDescription }
            self.consent = log; self.title = title
            session = SessionFile(id: id, title: title, startedAt: Date(), consent: log, live: LiveState(), segments: [], notes: "", seconds: 0)
            persist()
            try await capture.start()
            self.capture = capture
            Self.activeId = id
            phase = .recording
            UIApplication.shared.isIdleTimerDisabled = true
            startTimer()
        } catch {
            self.error = error.localizedDescription
            try? FileManager.default.removeItem(at: dir)
            phase = .idle
        }
    }

    func pause() { capture?.pause(); phase = .paused; persist() }

    func resume() {
        do { try capture?.resume(); phase = .recording } catch { self.error = error.localizedDescription }
    }

    /// Person kommt hinzu: Aufnahme ist pausiert, neue Bestätigung ab dieser Stelle.
    func confirmJoin(_ record: ConsentRecord) {
        do {
            try consent.append(record)
            session?.consent = consent
            persist(); resume()
        } catch { self.error = "Einwilligung unvollständig. Aufnahme bleibt pausiert." }
    }

    func stop() async {
        guard let capture, phase == .recording || phase == .paused else { return }
        phase = .stopping
        timer?.invalidate(); timer = nil
        await capture.stop()
        seconds = capture.seconds
        persist()
        UIApplication.shared.isIdleTimerDisabled = false
        Self.activeId = nil
        if let session { await finish(capture.fileURL, session) }
        self.capture = nil
        phase = .finished
    }

    // MARK: Nutzeraktionen im Live-Stand

    func confirmDone(_ id: String) { live.confirmDone(id); persist() }
    func rejectAnswer(_ id: String) { live.rejectAnswer(id); persist() }

    func saveNotes() { persist() }

    func time(of segmentId: String) -> Double? { segments.first { $0.id == segmentId }?.start }

    // MARK: Intern

    private func receive(_ u: LiveCapture.Update) {
        if !u.finals.isEmpty { segments += u.finals; volatile = ""; persist() } else { volatile = u.volatile }
    }

    private func startTimer() {
        timer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.heartbeat() }
        }
    }

    private func heartbeat() {
        guard let capture else { return }
        seconds = capture.seconds
        let pending = segments.filter { $0.end > live.processedThrough }
        let words = pending.reduce(0) { $0 + $1.text.split(separator: " ").count }
        let decision = policy.decide(now: seconds, lastRun: lastTickAudio, newWords: words, running: ticking,
                                     thermal: LiveThermal(ProcessInfo.processInfo.thermalState),
                                     lowPower: ProcessInfo.processInfo.isLowPowerModeEnabled,
                                     foreground: UIApplication.shared.applicationState == .active,
                                     aiAllowed: aiAllowed && LiveSummarizer.available)
        switch decision {
        case .paused(let why):
            aiStatus = LiveSummarizer.available ? (why == "KI pausiert (Wärme)" ? "KI pausiert – iPhone zu warm" : why) : "KI nicht verfügbar"
        case .wait: if aiStatus.hasPrefix("KI pausiert") { aiStatus = "" }
        case .run: Task { await tick(pending: pending) }
        }
    }

    private func tick(pending: [LiveSegment]) async {
        ticking = true; defer { ticking = false }
        lastTickAudio = seconds
        // Gezielt ältere Belege: Segmente, auf die offene Einträge verweisen.
        let wanted = Set(live.items.filter { $0.status != .done }.flatMap { $0.evidence.map(\.segmentId) })
        let older = segments.filter { wanted.contains($0.id) && $0.end <= live.processedThrough }.suffix(12)
        do {
            let (delta, prompt) = try await LiveSummarizer.tick(state: live, pending: pending, older: Array(older))
            _ = live.apply(delta, segments: prompt.segments)
            lastTick = Date(); aiStatus = ""
            persist()
        } catch {
            aiStatus = "KI-Takt übersprungen"
        }
    }

    private func persist() {
        guard var s = session else { return }
        s.live = live; s.segments = segments; s.notes = notes; s.consent = consent; s.title = title
        s.seconds = capture?.seconds ?? seconds
        session = s
        let url = Self.runningRoot.appendingPathComponent(s.id.uuidString).appendingPathComponent("sitzung.json")
        try? JSONEncoder().encode(s).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
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
