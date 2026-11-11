import Foundation
import Combine

// Gesprächsmodus „Sprechen“ (Paket A): Inhalte für den Assistenten. Ein app-weiter `AssistantCorpus` (nur im Speicher,
// einmal je App-Lauf aufgebaut, danach nur Änderungen über `version`) und die Ablage als eigene Quelle über die
// vorhandene Suche. Bücher, Besprechungen und Sprachnotizen kommen aus den bestehenden Speichern.
// Alles Rechnen (Lesen, Zerlegen, Indexieren) läuft abseits des Hauptthreads, Buch für Buch; der Hauptthread sieht nur
// einen kleinen Schnappschuss und den Fortschritt.
//
// Schutz der Inhalte:
// - Bücher: Ist ein Buch begonnen und nicht fertig (Lese- oder Hörstand da), steht nur der Text bis zur Lesegrenze im Index
//   (nie dahinter); unbegonnene und fertige Bücher stehen ganz drin.
// - Besprechungen: nur Segmente mit `aiPermitted`; das Protokoll nur, wenn es eines gibt (es entsteht nur aus erlaubten Segmenten).
// - Ablage: dieselbe Suche und dieselben Schutzregeln wie die Seite „Ablage“; bei Cloud-Modellen nur Dokumente der Klasse „öffentlich“.
// - Zitate: Die Einheit ist der gespeicherte Block zwischen strukturellen Grenzen der Quelle (`AssistantSections`, Kern): ein
//   Absatz im Buch, ein Textblock in der Notiz, ein Segment der Besprechung, ein Teil des Protokolls. Nichts wird geteilt, gekürzt
//   oder zusammengezogen; zitiert wird nur ein ganzer Block. Die Ablage liefert Fenster aus der Dateisuche, keine Blöcke, und
//   ist deshalb (noch) nicht zitierbar.

@MainActor
final class AssistantSources: ObservableObject {
    struct Progress: Equatable {
        var done: Int
        var total: Int
    }

    static let shared = AssistantSources()

    /// Gemeinsamer Index aller Gespräche dieses App-Laufs.
    let corpus: AssistantCorpus
    /// Läuft die Indexierung, wie weit (Hinweis „Inhalte werden vorbereitet (n von m)“).
    @Published private(set) var progress: Progress?
    /// Zählt jede abgeschlossene Indexierung.
    @Published private(set) var version = 0
    /// Bücher und Besprechungen, die wegen der Obergrenze fehlen.
    @Published private(set) var skipped = 0

    private let indexer: AssistantIndexer
    private let watch = AssistantContentWatch.shared
    private var running: Task<Void, Never>?
    private var rerun = false
    /// Ein Gespräch ist offen: Änderungen der Quellen stoßen die Indexierung an (sonst erst das nächste Öffnen).
    private var active = false
    private var bag: Set<AnyCancellable> = []

    init() {
        AssistantReleaseFiles.start()   // die Ordnung des Prozesses an Protokoll und Meldestelle hängen (idempotent)
        let corpus = AssistantCorpus()
        self.corpus = corpus
        indexer = AssistantIndexer(corpus: corpus)
        watch.setObserver { [weak self] in Task { @MainActor in self?.changeFinished() } }
        // Der Speicher der Notizen (auch auf der Uhr im Einsatz) meldet über diese Weiche: VOR jeder Änderung an Text oder Bestand.
        let meldestelle = watch
        NoteChangeHook.shared.install {
            let ticket = meldestelle.willChange()
            return { meldestelle.changed(ticket) }
        }
        observeStores()
    }

    /// Das Gespräch öffnet: Der Bestand stammt aus einem früheren Lauf und gilt als nicht aktuell (eine Änderung zählt, die Quellen
    /// haben sich vielleicht geändert), bis ein Durchgang nach diesem Moment endet. Die Meldung stößt ihn an.
    func open() {
        active = true
        watch.around {}
    }

    /// Das Gespräch ist zu Ende: keine Durchgänge mehr von selbst.
    func close() { active = false }

    private func changeFinished() {
        guard active else { return }
        refresh()
    }

    /// Meldet jede Änderung der Speicher, aus denen der Bestand entsteht: Bücher und ihre Stände (`PhoneLibrary`), Sprachnotizen
    /// (`VoiceModel`), Schutz und Auswahl der Ablage (`AblageSync`). `@Published` meldet VOR dem Setzen (`willSet`): Der Aufruf
    /// kommt synchron auf dem Thread des Auslösers, die Änderung im Speicher folgt danach; ihr Ende melden wir im nächsten Zug des
    /// Hauptthreads (dann ist sie gesetzt). Der erste Wert beim Abonnieren zählt nicht.
    private func observeStores() {
        let watch = self.watch
        func announce<P: Publisher>(_ publisher: P) where P.Failure == Never {
            publisher.dropFirst().sink { _ in
                let ticket = watch.willChange()
                DispatchQueue.main.async { watch.changed(ticket) }
            }.store(in: &bag)
        }
        announce(PhoneLibrary.shared.$books)
        announce(PhoneLibrary.shared.$progress)
        announce(VoiceModel.shared.$entries)
        announce(AblageSync.shared.$uebersicht)
    }

    /// Stößt die Indexierung an (kehrt sofort zurück). Läuft schon eine, folgt danach genau ein weiterer Durchgang. Die Marke der
    /// Meldestelle wird VOR den Schnappschüssen genommen: Der Durchgang deckt nur Änderungen, die bis dahin beendet waren.
    func refresh() {
        guard running == nil else { rerun = true; return }
        let mark = watch.mark
        let notes = Self.noteJobs()
        let books = Self.bookJobs()
        let root = MeetingDirectory.root
        let indexer = self.indexer
        running = Task { [weak self] in
            let report = await Task.detached(priority: .utility) {
                await indexer.run(notes: notes, books: books, meetingsRoot: root) { done, total in
                    Task { @MainActor in self?.setProgress(done, total) }
                }
            }.value
            self?.finish(report, mark: mark)
        }
    }

    /// Wissen für ein Modell: gleicher Index, die Ablage je nach Cloud-Modell mit eigenen Regeln.
    func knowledge(cloud: Bool, areas: Set<AssistantArea>) async -> AssistantKnowledge {
        AssistantKnowledge(corpus: corpus, sources: [AblageAssistantSource(cloud: cloud)], areas: areas)
    }

    private func setProgress(_ done: Int, _ total: Int) {
        guard running != nil else { return }
        progress = total > 0 && done < total ? Progress(done: done, total: total) : nil
    }

    private func finish(_ report: AssistantIndexer.Report, mark: Int) {
        running = nil
        progress = nil
        skipped = report.skipped
        if !report.cancelled {
            version += 1
            // Der Bestand steht für alle Änderungen bis zur Marke; ein abgebrochener Durchgang meldet nichts (Bestand bleibt `stale`).
            watch.passCompleted(mark: mark)
        }
        if rerun {
            rerun = false
            refresh()
        }
    }

    // MARK: Schnappschüsse (Hauptthread, nur Werte)

    private static func noteJobs() -> [NoteJob] {
        VoiceModel.shared.entries.compactMap { entry in
            // Der gespeicherte Text der Notiz bleibt UNVERÄNDERT (auch Rand-Leerraum): Der Kern schneidet den Rohtext der Blöcke
            // daraus (E4.2b, Blocktreue). Getrimmt wird nur zur Prüfung, ob es etwas gibt.
            let transcript = entry.transcript ?? ""
            let reply = entry.reply?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !transcript.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !reply.isEmpty else { return nil }
            return NoteJob(id: entry.id.uuidString, date: entry.createdAt, transcript: transcript, reply: reply)
        }
    }

    private static func bookJobs() -> [BookJob] {
        let library = PhoneLibrary.shared
        return library.books.map { book in
            let audio = book.audio
            return BookJob(id: book.id, title: book.title, authors: book.authors, url: library.textURL(book),
                           textRecord: library.record(library.textKey(book)), audioRecord: library.record(library.audioKey(book)),
                           sync: audio?.sync ?? [], totalMs: audio?.totalDurationMs, finished: book.finished,
                           added: Date(timeIntervalSince1970: book.addedMs / 1000))
        }
    }
}

private enum MeetingDirectory {
    static var root: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Meetings")
    }
}

// MARK: - Aufträge (Werte, Sendable)

private struct NoteJob: Sendable {
    let id: String
    let date: Date
    let transcript: String
    let reply: String
}

private struct BookJob: Sendable {
    let id: String
    let title: String
    let authors: [String]
    let url: URL
    let textRecord: ProgressRecord?
    let audioRecord: ProgressRecord?
    let sync: [LibrarySyncPoint]
    let totalMs: Double?
    let finished: Bool
    let added: Date
}

// MARK: - Indexierung

/// Baut und pflegt den Index abseits des Hauptthreads. Die Obergrenze für den Gesamttext hält der Index selbst (`admit`).
private actor AssistantIndexer {
    struct Report {
        var skipped = 0
        var cancelled = false
    }

    /// Obergrenze für den Gesamttext im Speicher (Zeichen); Rest wird gemeldet, nicht still weggelassen.
    static let maxChars = 30_000_000

    private let corpus: AssistantCorpus

    init(corpus: AssistantCorpus) { self.corpus = corpus }

    func run(notes: [NoteJob], books: [BookJob], meetingsRoot: URL, progress: @escaping @Sendable (Int, Int) -> Void) async -> Report {
        var report = Report()
        var keep: [AssistantArea: Set<String>] = [:]
        let meetings = Self.meetingFolders(meetingsRoot)
        let total = notes.count + meetings.count + books.count
        var done = 0
        progress(0, total)

        for job in notes {
            if Task.isCancelled { report.cancelled = true; return report }
            let version = "\(job.transcript.utf8.count)|\(job.reply.utf8.count)|\(job.transcript.hashValue ^ job.reply.hashValue)"
            let same = await unchanged(.notes, job.id, version)
            if same {
                keep[.notes, default: []].insert(job.id)
            } else if let doc = Self.noteDocument(job, version: version) {
                keep[.notes, default: []].insert(job.id)
                if !(await put(doc)) { report.skipped += 1 }
            }
            // Ließ sich die geänderte Notiz nicht mehr aufnehmen (kein Block), bleibt sie nicht mit dem alten Text im Bestand.
            done += 1
            progress(done, total)
        }

        let archive = try? MeetingArchive(root: meetingsRoot)
        for (id, folder) in meetings {
            if Task.isCancelled { report.cancelled = true; return report }
            done += 1
            defer { progress(done, total) }
            guard let version = Self.fileStamp(folder.appendingPathComponent("meeting.json")), let archive else { continue }
            let key = id.uuidString
            if await unchanged(.transcripts, key, version) { keep[.transcripts, default: []].insert(key); continue }
            guard let meeting = try? await archive.load(id), let doc = Self.meetingDocument(meeting, version: version) else { continue }
            keep[.transcripts, default: []].insert(key)
            if !(await put(doc)) { report.skipped += 1 }
        }

        for job in books {
            if Task.isCancelled { report.cancelled = true; return report }
            done += 1
            defer { progress(done, total) }
            guard let stamp = Self.fileStamp(job.url) else { continue }
            let fraction = Self.fraction(job)
            let limited = Self.isLimited(job)
            // Bei begrenztem Text zählt der genaue Stand (nicht gerastert): geht er zurück (Fortschritt zurückgesetzt), wird
            // neu indexiert, damit nie mehr Text im Index bleibt, als die Lesegrenze erlaubt.
            let version = limited
                ? "\(stamp)|limit|\(Self.whole(Self.reach(job.textRecord)))|\(Self.whole(Self.reach(job.audioRecord)))|\(job.sync.count)|\(Self.whole(job.totalMs ?? 0))"
                : "\(stamp)|all"
            keep[.books, default: []].insert(job.id)
            if await unchanged(.books, job.id, version) { continue }
            guard let doc = Self.bookDocument(job, fraction: fraction, limited: limited, version: version) else {
                keep[.books]?.remove(job.id)
                continue
            }
            if !(await put(doc)) { report.skipped += 1 }
        }

        // Aufräumen erst nach vollständigem Durchgang: ein abgebrochener Lauf löscht nichts.
        for area in [AssistantArea.notes, .transcripts, .books] {
            for item in await corpus.items(area: area) where keep[area]?.contains(item.id) != true {
                await corpus.remove(id: item.id, area: area)
            }
        }
        return report
    }

    private func unchanged(_ area: AssistantArea, _ id: String, _ version: String) async -> Bool {
        await corpus.version(of: id, area: area) == version
    }

    /// `false`, wenn die Obergrenze überschritten würde. Dann scheidet das Dokument AUS dem Bestand aus (auch ein älterer Stand,
    /// `AssistantCorpus.admit`): Ein Dokument, das sich nicht neu aufnehmen lässt, bleibt nicht mit seinem alten Inhalt zitierbar.
    /// Erst danach darf der Durchgang „bereit“ melden (`AssistantSources.finish`).
    private func put(_ doc: AssistantDocument) async -> Bool {
        await corpus.admit(doc, maxChars: Self.maxChars)
    }

    // MARK: Dateien

    /// „Größe|Änderungszeit“; nil, wenn es die Datei nicht gibt.
    private static func fileStamp(_ url: URL) -> String? {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path) else { return nil }
        let size = (attributes[.size] as? NSNumber)?.int64Value ?? 0
        let time = (attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        return "\(size)|\(time)"
    }

    private static func meetingFolders(_ root: URL) -> [(UUID, URL)] {
        let urls = (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        return urls.compactMap { url in UUID(uuidString: url.lastPathComponent).map { ($0, url) } }
    }

    // MARK: Sprachnotizen

    private static func noteDocument(_ job: NoteJob, version: String) -> AssistantDocument? {
        let when = format(job.date, "dd.MM.yyyy, HH:mm")
        var sections: [AssistantDocument.Section] = []
        sections += AssistantSections.blocks(of: job.transcript, locator: when)
        if !job.reply.isEmpty { sections += AssistantSections.generated(["Antwort der App: " + job.reply], locator: when) }
        guard !sections.isEmpty else { return nil }
        let item = AssistantItem(area: .notes, id: job.id, title: "Sprachnotiz vom \(when)", detail: format(job.date, "dd.MM.yyyy"), updated: job.date)
        return AssistantDocument(item: item, version: version, sections: sections)
    }

    // MARK: Besprechungen

    private static func meetingDocument(_ meeting: MeetingDocument, version: String) -> AssistantDocument? {
        // Ein Block je Segment der Aufnahme. Nicht erlaubte oder leere Segmente werden kein Block; der Kern bestimmt die Blöcke.
        // `line` (mit Sprecher, getrimmt) dient nur der Suche; ausgegeben wird `raw`: das gespeicherte Textfeld des Segments,
        // unverändert (E4.2b, Blocktreue).
        let segments = meeting.segments.sorted(by: { $0.start < $1.start }).map { segment -> AssistantSegment in
            let body = segment.text.trimmingCharacters(in: .whitespacesAndNewlines)
            return AssistantSegment(start: segment.start, line: segment.speaker.map { "\($0): \(body)" } ?? body, raw: segment.text,
                                    usable: meeting.aiPermitted(segment) && !body.isEmpty)
        }
        var sections = AssistantSections.meeting(segments, locator: clock)

        if let minutes = meeting.minutes { sections += AssistantSections.generated(minutesText(minutes), locator: "Protokoll") }
        guard !sections.isEmpty else { return nil }

        let name = URL(fileURLWithPath: meeting.originalName).deletingPathExtension().lastPathComponent
        let date = format(meeting.createdAt, "dd.MM.yyyy")
        let title = name.isEmpty ? "Besprechung vom \(date)" : name
        let minutesLong = max(1, Int((meeting.duration / 60).rounded()))
        let item = AssistantItem(area: .transcripts, id: meeting.id.uuidString, title: title, detail: "\(date) · \(minutesLong) min", updated: meeting.createdAt)
        return AssistantDocument(item: item, version: version, sections: sections)
    }

    /// Die Teile des Protokolls (Zusammenfassung, Beschlüsse, Aufgaben, …); jeder Teil ist ein Block.
    private static func minutesText(_ minutes: MeetingMinutes) -> [String] {
        var parts: [String] = []
        let summary = minutes.summary.trimmingCharacters(in: .whitespacesAndNewlines)
        if !summary.isEmpty { parts.append("Zusammenfassung: " + summary) }
        let decisions = minutes.decisions.map { entry -> String in
            let context = entry.context.trimmingCharacters(in: .whitespacesAndNewlines)
            return "- " + entry.text + (context.isEmpty ? "" : " (\(context))")
        }
        if !decisions.isEmpty { parts.append("Beschlüsse:\n" + decisions.joined(separator: "\n")) }
        let tasks = minutes.tasks.map { task -> String in
            var extra: [String] = []
            if let who = task.assignee, !who.isEmpty { extra.append("zuständig: " + who) }
            if let due = task.due, !due.isEmpty { extra.append("bis " + due) }
            return "- " + task.text + (extra.isEmpty ? "" : " (\(extra.joined(separator: ", ")))")
        }
        if !tasks.isEmpty { parts.append("Aufgaben:\n" + tasks.joined(separator: "\n")) }
        let steps = minutes.next_steps.map { step -> String in
            "- " + step.text + ((step.owner ?? "").isEmpty ? "" : " (\(step.owner ?? ""))")
        }
        if !steps.isEmpty { parts.append("Nächste Schritte:\n" + steps.joined(separator: "\n")) }
        let questions = minutes.open_questions.map { "- " + $0.text }
        if !questions.isEmpty { parts.append("Offene Fragen:\n" + questions.joined(separator: "\n")) }
        return parts
    }

    /// „00:14:03“.
    private static func clock(_ seconds: Double) -> String {
        let total = max(0, Int(seconds.isFinite ? seconds : 0))
        return String(format: "%02d:%02d:%02d", total / 3600, (total % 3600) / 60, total % 60)
    }

    // MARK: Bücher

    /// Anteil 0…1 aus Lesen und Hören; „fertig“ zählt als 1.
    private static func fraction(_ job: BookJob) -> Double {
        if job.finished { return 1 }
        return max(LibraryProgress.fraction(job.textRecord), LibraryProgress.fraction(job.audioRecord))
    }

    /// Begonnen (Lese- oder Hörstand vorhanden) und nicht fertig: dann endet der Text an der Lesegrenze, unabhängig vom Anteil.
    private static func isLimited(_ job: BookJob) -> Bool {
        !job.finished && (reach(job.textRecord) > 0 || reach(job.audioRecord) > 0)
    }

    private static func reach(_ record: ProgressRecord?) -> Double {
        record.map { LibraryProgress.reach($0) } ?? 0
    }

    /// Ganzzahl für Kennungen; nie ein Absturz bei NaN oder unendlich.
    private static func whole(_ value: Double) -> Int {
        value.isFinite ? Int(min(max(value, 0), 1e15)) : 0
    }

    private static func bookDocument(_ job: BookJob, fraction: Double, limited: Bool, version: String) -> AssistantDocument? {
        guard let text = try? String(contentsOf: job.url, encoding: .utf8), !text.isEmpty else { return nil }
        var limit: Int?
        if limited {
            let length = Double(text.utf16.count)
            var map: FollowMap?
            if let total = job.totalMs, total > 0, !job.sync.isEmpty { map = FollowMap(sync: job.sync, textLength: length, totalMs: total) }
            // Ohne Lesegrenze (nur eine Schätzung vom Hörbuch ohne Abgleich): der geschätzte Anteil, nie mehr.
            limit = LibraryProgress.frontier(text: job.textRecord, audio: job.audioRecord, map: map) ?? whole((fraction * length).rounded(.down))
        }
        // Ein Block je Absatz des Buchtextes; ein Absatz hinter der Lesegrenze fällt ganz weg (der Kern, nie gekürzt).
        let sections = AssistantSections.book(BookTalkIndex.blocks(in: text), frontier: limit)
        guard !sections.isEmpty else { return nil }

        var detail = job.authors.joined(separator: ", ")
        if fraction > 0 {
            let percent = "\(Int((fraction * 100).rounded(.down))) % gelesen"
            detail = detail.isEmpty ? percent : detail + " · " + percent
        }
        let item = AssistantItem(area: .books, id: job.id, title: job.title, detail: detail.isEmpty ? nil : detail, updated: job.added)
        return AssistantDocument(item: item, version: version, sections: sections)
    }

    private static func format(_ date: Date, _ pattern: String) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "de_DE")
        formatter.dateFormat = pattern
        return formatter.string(from: date)
    }
}

// MARK: - Ablage

/// Die Ablage als Quelle: dieselbe Suche und dieselben Schutzregeln wie die Seite „Ablage“ (Index-Tor, Papierkorb,
/// angehaltene und nur lokale Dateien bleiben draußen). Ein Cloud-Modell bekommt nur Dokumente der Klasse „öffentlich“:
/// das iPhone kennt die Cloud-Freigabe der Ablage nicht (sie liegt nur am PC), also gilt die strengste Lesart.
struct AblageAssistantSource: AssistantSource {
    let cloud: Bool

    var area: AssistantArea { .files }

    func items() async -> [AssistantItem] {
        let files = await MainActor.run { AblageSync.shared.uebersicht?.dateien ?? [] }
        return files.filter { allowed($0) && $0.textBereit }.map { file in
            AssistantItem(area: .files, id: file.id, title: file.name, detail: file.typ?.uppercased(), updated: nil)
        }
    }

    func search(_ question: String, limit: Int) async -> [AssistantPassage] {
        guard limit > 0 else { return [] }
        let found = await AblageSync.shared.suchen(question)
        // Ablage-Auszüge sind Fenster aus der Dateisuche, keine gespeicherten Blöcke (Standard `isBlock: false`): Sie dienen dem
        // Abruf und der Erkennung, werden aber nie wörtlich ausgegeben.
        var out: [AssistantPassage] = []
        for (rank, result) in found.filter({ allowed($0.eintrag) }).enumerated() {
            let score = 1.0 / Double(1 + rank)
            for place in result.stellen {
                out.append(AssistantPassage(area: .files, itemId: result.eintrag.id, title: result.eintrag.name,
                                            locator: AblageAnker.beschreibung(place.anker), text: place.auszug, score: score))
            }
        }
        return Array(out.prefix(limit))
    }

    private func allowed(_ file: AblageDateiEintrag) -> Bool {
        guard !file.imKorb, file.zustand != .angehalten, file.zustand != .nurGeraet else { return false }
        return cloud ? file.klasse == .public : true
    }
}
