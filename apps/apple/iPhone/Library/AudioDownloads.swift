import Foundation
import UIKit
import CryptoKit

/// Hörbücher aus dem Konto im Hintergrund laden (Epic #210, P8 Paket 3).
///
/// Eigene Hintergrund-`URLSession`, ein Auftrag je Stück. Das System lädt
/// weiter, wenn die App im Hintergrund ist oder beendet wurde, und weckt sie
/// für jedes angekommene Stück. Angekommene Stücke liegen bis zum Anhängen in
/// `audio/.stuecke/`; angehängt wird streng in Hörreihenfolge (prüfen,
/// entschlüsseln, an `<teil>.part` schreiben). Ein Teil wird erst vollständig
/// in seinen Namen umbenannt. Plan und Zustand: `AudioDownload` (VoiceCore).
@MainActor
final class AudioDownloads: NSObject, ObservableObject, URLSessionDownloadDelegate {
    static let shared = AudioDownloads()
    static let sessionID = "de.localvoice.prototype.audiobook-downloads"

    /// Laufende, pausierte oder fehlgeschlagene Ladevorgänge je Buch.
    @Published private(set) var jobs: [String: AudioDownload.Job] = [:]
    /// Geladene Bytes je Buch (fertige Teile plus angehängte Stücke).
    @Published private(set) var done: [String: Int] = [:]

    private let library: PhoneLibrary
    /// Laufende Aufträge je Buch: Auftragsnummer → Stück. Rückmeldungen von
    /// Aufträgen, die hier nicht (mehr) stehen, ändern nichts an der Buchführung.
    private var inFlight: [String: [Int: AudioDownload.ChunkKey]] = [:]
    /// Fehlversuche je Stück (`<buch>|<teil>|<stück>`); erst nach dreien schlägt das Laden fehl.
    /// Je laufendem Auftrag; endet mit ihm, damit ein späteres Laden wieder zurückfallen darf.
    private var legacyFallbacks: [String: AudioDownload.LegacyFallback] = [:]
    private var speed = AudioDownload.SpeedTracker()
    private var retries: [String: Int] = [:]
    private var assembling: Set<String> = []
    private var waiters: [String: [CheckedContinuation<Bool, Never>]] = [:]
    private var backgroundCompletion: (() -> Void)?
    private var eventsFinished = false
    private var restoration: Task<Void, Never>?
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.background(withIdentifier: Self.sessionID)
        configuration.isDiscretionary = false
        configuration.sessionSendsLaunchEvents = true
        configuration.timeoutIntervalForResource = 7 * 24 * 60 * 60
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.httpMaximumConnectionsPerHost = 4
        return URLSession(configuration: configuration, delegate: self, delegateQueue: .main)
    }()

    init(library: PhoneLibrary? = nil) {
        self.library = library ?? .shared
        super.init()
        if let data = try? Data(contentsOf: stateURL),
           let saved = try? JSONDecoder().decode([String: AudioDownload.Job].self, from: data) { jobs = saved }
        for id in jobs.keys { refreshDone(id) }
        restoration = Task { await restore() }
    }

    // MARK: Abfragen

    /// Läuft (oder wartet auf Netz)? Dann darf nichts am Hörbuch gelöscht werden.
    func isActive(_ id: String) -> Bool { jobs[id]?.phase == .running }

    /// Läuft, aber „Nur im WLAN“ hält es gerade an.
    func waitsForWifi(_ id: String) -> Bool {
        let sync = LibrarySync.shared
        guard let job = jobs[id], job.phase == .running, !job.allowCellular else { return false }
        return sync.wifiOnly && sync.expensiveNetwork
    }

    /// Bis der Ladevorgang fertig ist (`true`) oder pausiert/fehlschlägt (`false`).
    func waitUntilDone(_ id: String) async -> Bool {
        guard jobs[id]?.phase == .running else { return jobs[id] == nil && isLocal(id) }
        return await withCheckedContinuation { waiters[id, default: []].append($0) }
    }

    private func isLocal(_ id: String) -> Bool { library.books.first { $0.id == id }.map { library.hasAudio($0) } ?? false }

    // MARK: Steuern

    /// Laden starten oder fortsetzen. Prüft vorher den freien Speicher.
    func start(_ book: LibraryBook, allowCellular: Bool? = nil) {
        guard LibrarySync.shared.account != nil else { return }
        let refs = library.meta(book).files.audio
        guard !refs.isEmpty, !library.hasAudio(book) else { return }
        var previous = jobs[book.id]
        let parts = library.audioParts(refs)
        // Konto hat die Dateien ersetzt: Weder alte Dateien noch angefangene Teile dürfen zählen.
        library.removeStaleAudio(book)
        if let stale = previous, stale.parts != parts {
            discard(book.id)
            for part in stale.parts { try? FileManager.default.removeItem(at: audioDir(book.id).appendingPathComponent(part.file).appendingPathExtension("part")) }
            previous = nil
        }
        if previous?.phase == .running, allowCellular == nil { assemble(book.id); return }
        var job = AudioDownload.Job(bookId: book.id, parts: parts, allowCellular: allowCellular ?? previous?.allowCellular ?? false)
        let remaining = job.totalBytes - AudioDownload.doneBytes(job, states: states(job))
        if let problem = AudioDownload.spaceProblem(remainingBytes: remaining, freeBytes: library.freeBytes(), format: LibraryBytes.text) {
            job.phase = .failed; job.message = problem
            jobs[book.id] = job; persist(); resolve(book.id, false)
            return
        }
        do {
            try FileManager.default.createDirectory(at: stagingDir(book.id), withIntermediateDirectories: true,
                                                    attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
            PhoneLibrary.excludeFromBackup(library.audioDir(book))
        } catch {
            job.phase = .failed; job.message = "Der Ordner für das Hörbuch lässt sich nicht anlegen: \(error.localizedDescription)"
            jobs[book.id] = job; persist(); resolve(book.id, false)
            return
        }
        jobs[book.id] = job
        persist()
        refreshDone(book.id)
        // Mit anderer Netzregel neu einreihen: laufende Aufträge vorher abbrechen.
        let ruleChanged = previous?.allowCellular != job.allowCellular
        let id = book.id
        Task {
            await restoration?.value
            if ruleChanged { cancelTasks(id) }
            assemble(id)
        }
    }

    func pause(_ id: String) {
        guard jobs[id]?.phase == .running else { return }
        speed.reset(id)
        jobs[id]?.phase = .paused; jobs[id]?.message = nil
        persist(); resolve(id, false)
        cancelTasks(id)
    }

    /// Für dieses Buch auch über Mobilfunk laden.
    func useCellular(_ book: LibraryBook) { start(book, allowCellular: true) }

    /// Vom Nutzer abgebrochen: Auftrag verwerfen und alles Geladene löschen.
    /// Geladen wird nur, was im Konto liegt; das Hörbuch bleibt dort erhalten.
    func cancel(_ book: LibraryBook) {
        discard(book.id)
        _ = try? AudioOffload.remove(bookDir: library.dir(book))
        library.reload()
    }

    /// Ladevorgang verwerfen (Hörbuch wird entfernt oder Buch gelöscht).
    func discard(_ id: String) {
        speed.reset(id)
        legacyFallbacks[id] = nil
        guard jobs[id] != nil || inFlight[id] != nil else { return }
        jobs[id] = nil; done[id] = nil
        persist(); resolve(id, false)
        cancelTasks(id)
        if let book = library.books.first(where: { $0.id == id }) {
            try? FileManager.default.removeItem(at: library.audioDir(book).appendingPathComponent(AudioDownload.stagingFolder))
        }
    }

    /// „Nur im WLAN“ wurde umgestellt: laufende Aufträge mit der neuen Regel neu einreihen.
    func networkRuleChanged() {
        for (id, job) in jobs where job.phase == .running && !job.allowCellular {
            cancelTasks(id)
            pump(id)
        }
    }

    // MARK: Ablauf

    /// Teile, deren `.part` schon vollständig ist (App beendet zwischen letztem
    /// Stück und Umbenennen), fertigstellen.
    private func finalizeParts(_ job: AudioDownload.Job) {
        let dir = audioDir(job.bookId)
        for part in job.parts {
            let target = dir.appendingPathComponent(part.file), partial = target.appendingPathExtension("part")
            guard LibrarySync.fileSize(target) != part.size, LibrarySync.fileSize(partial) == part.size else { continue }
            _ = try? FileManager.default.removeItem(at: target)
            if (try? FileManager.default.moveItem(at: partial, to: target)) != nil {
                LibrarySync.shared.markConfirmed(part.blob)
                library.noteLocalAudio(bookId: job.bookId, file: part.file, blob: part.blob)
            }
        }
    }

    /// Schon angehängt (oder Teil fertig): ein solches Stück wird nicht mehr gebraucht.
    private func alreadyAppended(_ key: AudioDownload.ChunkKey, _ job: AudioDownload.Job, _ states: [AudioDownload.PartState]) -> Bool {
        guard key.part < job.parts.count, key.part < states.count else { return true }
        guard let next = AudioDownload.nextChunk(job.parts[key.part], states[key.part], chunkPlainBytes: LibrarySyncFormat.chunkPlainBytes) else { return true }
        return key.index < next
    }

    private func states(_ job: AudioDownload.Job) -> [AudioDownload.PartState] {
        let dir = audioDir(job.bookId)
        return job.parts.map { part in
            let target = dir.appendingPathComponent(part.file)
            let complete = LibrarySync.fileSize(target) == part.size
            let partial = LibrarySync.fileSize(target.appendingPathExtension("part")) ?? 0
            return AudioDownload.PartState(complete: complete, partialBytes: complete ? 0 : partial)
        }
    }

    private func staged(_ id: String) -> Set<AudioDownload.ChunkKey> {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: stagingDir(id).path)) ?? []
        return Set(names.compactMap(AudioDownload.parseStaged))
    }

    /// Fehlende Stücke einreihen (höchstens `AudioDownload.window` unterwegs).
    private func pump(_ id: String) {
        guard let job = jobs[id], job.phase == .running else { return }
        guard library.books.contains(where: { $0.id == id }) else { discard(id); return }
        guard let acc = LibrarySync.shared.account else { fail(id, "Nicht angemeldet. Bitte unter „Konto & Sync“ anmelden."); return }
        finalizeParts(job)
        let current = states(job)
        if AudioDownload.isComplete(job, states: current) { finish(id); return }
        // Überholte Stücke (doppelt geladen, Abbruch nach dem Anhängen) belegen sonst das Fenster.
        var waiting = staged(id)
        for key in waiting where alreadyAppended(key, job, current) {
            try? FileManager.default.removeItem(at: stagingDir(id).appendingPathComponent(AudioDownload.stagedName(key)))
            waiting.remove(key)
        }
        let keys = AudioDownload.missingChunks(job, states: current, staged: waiting, inFlight: Set((inFlight[id] ?? [:]).values),
                                               chunkPlainBytes: LibrarySyncFormat.chunkPlainBytes)
        let cellular = job.allowCellular || !LibrarySync.shared.wifiOnly
        for key in keys {
            let part = job.parts[key.part]
            guard let url = URL(string: acc.url + "/api/v1/voice/blobs/\(part.blob)/chunks/\(key.index)") else { fail(id, "Ungültige Server-Adresse."); return }
            var request = URLRequest(url: url)
            request.httpMethod = "GET"
            request.cachePolicy = .reloadIgnoringLocalCacheData
            request.allowsExpensiveNetworkAccess = cellular
            request.allowsConstrainedNetworkAccess = cellular
            request.setValue("application/json", forHTTPHeaderField: "Accept")
            request.setValue("Bearer \(acc.token)", forHTTPHeaderField: "Authorization")
            let task = session.downloadTask(with: request)
            task.taskDescription = AudioDownload.taskDescription(bookId: id, key)
            inFlight[id, default: [:]][task.taskIdentifier] = key
            task.resume()
        }
    }

    /// Angekommene Stücke in Hörreihenfolge anhängen, dann nachladen.
    private func assemble(_ id: String) {
        guard !assembling.contains(id), let job = jobs[id], job.phase == .running else { return }
        guard let acc = LibrarySync.shared.account else { fail(id, "Nicht angemeldet. Bitte unter „Konto & Sync“ anmelden."); return }
        assembling.insert(id)
        finalizeParts(job)
        let lease = Lease()
        lease.id = UIApplication.shared.beginBackgroundTask(withName: "Hörbuch zusammensetzen") {
            // Zeit abgelaufen: Stand ist auf der Platte, es geht beim nächsten Wecken weiter.
            MainActor.assumeIsolated { lease.end() }  // wird auf dem Hauptthread gerufen
        }
        Task {
            defer {
                assembling.remove(id)
                lease.end()
                finishBackgroundEventsIfPossible()
            }
            let key = [UInt8](acc.key), userId = acc.userId, chunk = LibrarySyncFormat.chunkPlainBytes
            let dir = audioDir(id), staging = stagingDir(id)
            while let job = jobs[id], job.phase == .running {
                let current = states(job)
                // Erstes Teil, das noch Stücke braucht, und sein nächstes Stück.
                guard let p = current.indices.first(where: { !current[$0].complete }),
                      let index = AudioDownload.nextChunk(job.parts[p], current[p], chunkPlainBytes: chunk) else { break }
                let part = job.parts[p]
                let ck = AudioDownload.ChunkKey(part: p, index: index)
                let file = staging.appendingPathComponent(AudioDownload.stagedName(ck))
                guard FileManager.default.fileExists(atPath: file.path) else { break }
                let target = dir.appendingPathComponent(part.file)
                do {
                    let size = try await Task.detached(priority: .utility) {
                        try Self.append(staged: file, to: target.appendingPathExtension("part"), key: key, userId: userId,
                                        blob: part.blob, index: index, chunkPlainBytes: chunk)
                    }.value
                    if index == part.chunks - 1 || size == part.size {
                        guard size == part.size else {
                            try? FileManager.default.removeItem(at: target.appendingPathExtension("part"))
                            throw LibrarySync.SyncError.corrupt("Größe von \(part.file)")
                        }
                        _ = try? FileManager.default.removeItem(at: target)
                        try FileManager.default.moveItem(at: target.appendingPathExtension("part"), to: target)
                        LibrarySync.shared.markConfirmed(part.blob)
                        library.noteLocalAudio(bookId: id, file: part.file, blob: part.blob)
                    }
                } catch {
                    try? FileManager.default.removeItem(at: file)
                    fail(id, "Ein Stück kam beschädigt an. Bereits geladene Teile bleiben erhalten, es geht an derselben Stelle weiter.")
                    return
                }
                refreshDone(id)
            }
            refreshDone(id)
            if let job = jobs[id], job.phase == .running, AudioDownload.isComplete(job, states: states(job)) { finish(id) }
            else { pump(id) }
        }
    }

    /// Ein Stück prüfen, entschlüsseln und an seine Stelle in `<teil>.part` schreiben. Liefert die neue Größe.
    nonisolated private static func append(staged: URL, to part: URL, key: [UInt8], userId: Int64, blob: String,
                                           index: Int, chunkPlainBytes: Int) throws -> Int {
        defer { try? FileManager.default.removeItem(at: staged) }
        let data = try Data(contentsOf: staged)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let raw = (json["data"] as? String).flatMap({ Data(base64Encoded: $0) }) else { throw LibrarySync.SyncError.corrupt("Antwort") }
        guard SHA256.hash(data: raw).map({ String(format: "%02x", $0) }).joined() == json["sha256"] as? String else {
            throw LibrarySync.SyncError.corrupt("Prüfsumme")
        }
        let plain = try LibrarySyncFormat.openChunk(key: key, userId: userId, blobId: blob, index: index, sealed: [UInt8](raw))
        let fm = FileManager.default
        if !fm.fileExists(atPath: part.path) {
            fm.createFile(atPath: part.path, contents: nil, attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        }
        let h = try FileHandle(forWritingTo: part); defer { try? h.close() }
        try h.truncate(atOffset: UInt64(index * chunkPlainBytes))
        try h.seekToEnd()
        try h.write(contentsOf: plain)
        return index * chunkPlainBytes + plain.count
    }

    /// Restzeit für das Regal; misst ab Beginn des laufenden Ladens.
    func remainingSeconds(_ id: String) -> TimeInterval? {
        guard let job = jobs[id], job.phase == .running, let bytes = done[id] else { return nil }
        return speed.remaining(id, doneBytes: bytes, totalBytes: job.totalBytes, now: Date())
    }

    private func finish(_ id: String) {
        speed.reset(id)
        legacyFallbacks[id] = nil
        jobs[id] = nil; done[id] = nil
        inFlight[id] = nil
        try? FileManager.default.removeItem(at: stagingDir(id))
        persist()
        library.reload()
        resolve(id, true)
    }

    private func fail(_ id: String, _ message: String) {
        speed.reset(id)
        legacyFallbacks[id] = nil
        guard jobs[id] != nil else { return }
        jobs[id]?.phase = .failed; jobs[id]?.message = message
        persist(); resolve(id, false)
        Task { await cancelTasks(id) }
    }

    private func resolve(_ id: String, _ ok: Bool) {
        let list = waiters.removeValue(forKey: id) ?? []
        for w in list { w.resume(returning: ok) }
    }

    /// Laufende Aufträge des Buchs abbrechen. Die Buchführung wird sofort geleert;
    /// späte Rückmeldungen dieser Aufträge fallen danach ins Leere.
    private func cancelTasks(_ id: String) {
        let ids = Set((inFlight[id] ?? [:]).keys)
        inFlight[id] = nil
        session.getAllTasks { tasks in
            for task in tasks where ids.contains(task.taskIdentifier) { task.cancel() }
        }
    }

    private func refreshDone(_ id: String) {
        guard let job = jobs[id] else { done[id] = nil; return }
        let bytes = AudioDownload.doneBytes(job, states: states(job))
        done[id] = bytes
        speed.note(id, phase: job.phase, bytes: bytes, now: Date())
    }

    /// Nach einem Neustart: laufende Aufträge wieder zuordnen, liegengebliebene Stücke anhängen.
    private func restore() async {
        let tasks = await session.allTasks
        for task in tasks where task.state == .running || task.state == .suspended {
            guard let parsed = AudioDownload.parseTask(task.taskDescription) else { task.cancel(); continue }
            let id = parsed.bookId, key = parsed.key
            guard let job = jobs[id], job.phase == .running else { task.cancel(); continue }
            // Inzwischen angekommen oder angehängt: nicht als laufend zählen.
            let arrived = FileManager.default.fileExists(atPath: stagingDir(id).appendingPathComponent(AudioDownload.stagedName(key)).path)
            if arrived || alreadyAppended(key, job, states(job)) { continue }
            inFlight[id, default: [:]][task.taskIdentifier] = key
        }
        for (id, job) in jobs where job.phase == .running { assemble(id) }
    }

    // MARK: Pfade

    private var stateURL: URL { library.root.appendingPathComponent("hoerbuch-downloads.json") }
    private func audioDir(_ id: String) -> URL { library.root.appendingPathComponent(id, isDirectory: true).appendingPathComponent(AudioOffload.audioFolder, isDirectory: true) }
    private func stagingDir(_ id: String) -> URL { audioDir(id).appendingPathComponent(AudioDownload.stagingFolder, isDirectory: true) }

    private func persist() {
        try? JSONEncoder().encode(jobs).write(to: stateURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    // MARK: Hintergrund-Ereignisse

    func handleBackgroundEvents(_ completion: @escaping () -> Void) {
        backgroundCompletion = completion
        _ = session
        finishBackgroundEventsIfPossible()
    }

    private func finishBackgroundEventsIfPossible() {
        guard eventsFinished, assembling.isEmpty, let completion = backgroundCompletion else { return }
        backgroundCompletion = nil; eventsFinished = false; completion()
    }

    // MARK: URLSessionDownloadDelegate (Hauptwarteschlange)

    nonisolated func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        let status = (downloadTask.response as? HTTPURLResponse)?.statusCode ?? 0
        let description = downloadTask.taskDescription, taskID = downloadTask.taskIdentifier
        let finalURL = downloadTask.response?.url
        // Die Datei verschwindet, sobald diese Methode zurückkehrt: synchron verschieben.
        MainActor.assumeIsolated { self.received(description, taskID: taskID, status: status, file: location, finalURL: finalURL) }
    }

    private func received(_ description: String?, taskID: Int, status: Int, file: URL, finalURL: URL?) {
        guard let parsed = AudioDownload.parseTask(description) else { return }
        let id = parsed.bookId, key = parsed.key
        inFlight[id]?[taskID] = nil
        guard let job = jobs[id], job.phase == .running else { return }
        // Buch inzwischen gelöscht (auch von einem anderen Gerät): Ordner nicht neu anlegen.
        guard library.books.contains(where: { $0.id == id }) else { discard(id); return }
        // Hintergrund-Sessions folgen Umleitungen, ohne zu fragen: Wenigstens das Ergebnis verwerfen.
        if let finalURL, let hub = LibrarySync.shared.account.flatMap({ URL(string: $0.url) }), !AudioDownload.sameOrigin(finalURL, hub) {
            fail(id, "Der Server hat auf eine andere Adresse umgeleitet. Das Laden wurde abgebrochen.")
            return
        }
        if status == 401 { fail(id, "Die Anmeldung ist abgelaufen. Bitte unter „Konto & Sync“ neu anmelden."); return }
        let name = AudioDownload.taskDescription(bookId: id, key)
        if status == 404, job.parts.indices.contains(key.part),
           legacyFallbacks[id, default: .init()].shouldRestart(job.parts[key.part], index: key.index, task: name) {
            _ = try? FileManager.default.removeItem(at: audioDir(id).appendingPathComponent(job.parts[key.part].file).appendingPathExtension("part"))
            pump(id)
            return
        }
        guard status == 200 else {
            retry(id, key, "Der Server antwortet mit Fehler \(status). Bereits geladene Teile bleiben erhalten, es geht an derselben Stelle weiter.")
            return
        }
        retries[name] = nil
        // Doppelt geladen (z. B. nach einem Neustart): schon angehängt, nicht noch einmal ablegen.
        guard !alreadyAppended(key, job, states(job)) else { assemble(id); return }
        let target = stagingDir(id).appendingPathComponent(AudioDownload.stagedName(key))
        do {
            try FileManager.default.createDirectory(at: stagingDir(id), withIntermediateDirectories: true)
            _ = try? FileManager.default.removeItem(at: target)
            try FileManager.default.moveItem(at: file, to: target)
        } catch {
            fail(id, "Das Hörbuch lässt sich nicht speichern: \(error.localizedDescription)")
            return
        }
        assemble(id)
    }

    nonisolated func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        let description = task.taskDescription, taskID = task.taskIdentifier
        let cancelled = (error as? URLError)?.code == .cancelled
        let failed = error != nil
        MainActor.assumeIsolated {
            guard failed, let parsed = AudioDownload.parseTask(description) else { return }
            let id = parsed.bookId, key = parsed.key
            // Nur Aufträge, die noch in der Buchführung stehen (nicht abgebrochene alte).
            guard inFlight[id]?.removeValue(forKey: taskID) != nil, !cancelled, jobs[id]?.phase == .running else { return }
            retry(id, key, "Der Server ist gerade nicht erreichbar. Bereits geladene Teile bleiben erhalten, es geht an derselben Stelle weiter.")
        }
    }

    /// Ein Stück bis zu dreimal neu anfordern (2, 4, 8 s), dann gilt das Laden als fehlgeschlagen.
    private func retry(_ id: String, _ key: AudioDownload.ChunkKey, _ message: String) {
        let name = AudioDownload.taskDescription(bookId: id, key)
        let attempt = (retries[name] ?? 0) + 1
        guard attempt <= 3 else { retries[name] = nil; fail(id, message); return }
        retries[name] = attempt
        Task {
            try? await Task.sleep(nanoseconds: UInt64(1 << attempt) * 1_000_000_000)
            pump(id)
        }
    }

    nonisolated func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        MainActor.assumeIsolated { self.eventsFinished = true; self.finishBackgroundEventsIfPossible() }
    }
}

/// Hintergrundzeit für das Zusammensetzen; genau einmal beenden.
@MainActor
private final class Lease {
    var id: UIBackgroundTaskIdentifier = .invalid
    func end() {
        guard id != .invalid else { return }
        UIApplication.shared.endBackgroundTask(id); id = .invalid
    }
}
