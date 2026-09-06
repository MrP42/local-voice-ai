import Foundation
import UIKit
import UserNotifications

struct ModelDownloadRecord: Codable {
    enum Phase: String, Codable { case downloading, verifying, ready, failed, cancelled }
    var phase: Phase
    var bytes: Int64 = 0
    var taskID: Int?
    var stagedFile: String?
    var message: String = ""
    var notified = false
    var completedAt: Date?
    var completedInBackground: Bool?
    var busy: Bool { phase == .downloading || phase == .verifying }
}

/// URLSession owns network transfers. This object restores status and verifies before announcing readiness.
@MainActor
final class ModelDownloads: NSObject, ObservableObject, URLSessionDownloadDelegate {
    static let shared = ModelDownloads()
    static let sessionID = "de.localvoice.prototype.model-downloads"
    nonisolated static var folder: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("ModelDownloads")
    }
    @Published private(set) var records: [String: ModelDownloadRecord] = [:]
    @Published private(set) var notificationHint = ""
    var onInstalled: (() -> Void)?
    private var verification: [String: Task<Void, Never>] = [:]
    private var lastProgress: [String: Date] = [:]
    private var backgroundCompletion: (() -> Void)?
    private var eventsFinished = false
    private var restoration: Task<Void, Never>?
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.background(withIdentifier: Self.sessionID)
        configuration.isDiscretionary = false
        configuration.sessionSendsLaunchEvents = true
        configuration.waitsForConnectivity = true
        configuration.timeoutIntervalForResource = 7 * 24 * 60 * 60
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: configuration, delegate: self, delegateQueue: .main)
    }()
    override init() {
        super.init()
        if let data = try? Data(contentsOf: Self.folder.appendingPathComponent("status.json")),
           let saved = try? JSONDecoder().decode([String: ModelDownloadRecord].self, from: data) { records = saved }
        restoration = Task { await restore() }
    }
    private func persist() throws {
        try FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true, attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        try JSONEncoder().encode(records).write(to: Self.folder.appendingPathComponent("status.json"), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }
    private func restore() async {
        let tasks = await session.allTasks
        let taskIDs = Set(tasks.map(\.taskIdentifier))
        for task in tasks {
            guard let name = task.taskDescription, LocalModel.all.contains(where: { $0.id == name }), records[name] == nil else { continue }
            records[name] = ModelDownloadRecord(phase: .downloading, bytes: task.countOfBytesReceived, taskID: task.taskIdentifier, message: "Download wieder verbunden")
        }
        let files = (try? FileManager.default.contentsOfDirectory(atPath: Self.folder.path)) ?? []
        for (name, record) in records {
            guard let model = LocalModel.all.first(where: { $0.id == name }) else { continue }
            if record.busy {
                let orphan = record.taskID.flatMap { id in files.first { $0.hasPrefix(name + "." + String(id) + ".") && $0.hasSuffix(".download") } }
                let staged = record.stagedFile ?? orphan
                if let staged, FileManager.default.fileExists(atPath: Self.folder.appendingPathComponent(staged).path) {
                    // Covers a process exit between the delegate's file move and its status commit.
                    records[name]?.phase = .verifying; records[name]?.stagedFile = staged
                    records[name]?.message = "Download gespeichert · Prüfung wird fortgesetzt"
                    try? persist(); verify(model)
                } else if record.phase == .verifying {
                    fail(name, error: ModelTransferError.storage)
                } else if record.taskID.map({ !taskIDs.contains($0) }) ?? true {
                    enqueue(model)
                }
            } else {
                for file in files where file.hasPrefix(name + ".") && file.hasSuffix(".download") {
                    try? FileManager.default.removeItem(at: Self.folder.appendingPathComponent(file))
                }
                if record.phase == .ready, !record.notified { await notifyReady(model) }
            }
        }
        try? persist()
    }
    func start(_ model: LocalModel) {
        Task {
            await restoration?.value
            guard records[model.id]?.busy != true, verification[model.id] == nil else { return }
            enqueue(model)
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            if settings.authorizationStatus == .notDetermined { _ = try? await center.requestAuthorization(options: [.alert, .sound]) }
            let current = await center.notificationSettings()
            notificationHint = current.authorizationStatus == .denied ? "Mitteilungen sind deaktiviert. Der Status bleibt hier sichtbar." : ""
        }
    }
    private func enqueue(_ model: LocalModel) {
        do {
            try FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true)
            let free = try FileManager.default.attributesOfFileSystem(forPath: Self.folder.path)[.systemFreeSize] as? NSNumber
            guard let free, free.int64Value > model.bytes * 2 + 32 * 1024 * 1024 else { throw ModelTransferError.space }
            var request = URLRequest(url: model.downloadURL)
            request.cachePolicy = .reloadIgnoringLocalCacheData
            request.setValue("no-cache", forHTTPHeaderField: "Cache-Control")
            let task = session.downloadTask(with: request)
            task.taskDescription = model.id
            records[model.id] = ModelDownloadRecord(phase: .downloading, taskID: task.taskIdentifier, message: "Verbindung wird hergestellt …")
            do { try persist() } catch { task.cancel(); throw error }
            task.resume()
        } catch { fail(model.id, error: error) }
    }
    func cancel(_ model: LocalModel) {
        let taskID = records[model.id]?.taskID
        verification[model.id]?.cancel()
        records[model.id]?.phase = .cancelled
        records[model.id]?.message = "Abgebrochen – erneut laden möglich"
        try? persist()
        Task { for task in await session.allTasks where task.taskIdentifier == taskID { task.cancel() } }
    }
    func resumeVerification() {
        for (name, record) in records where record.phase == .verifying {
            if let model = LocalModel.all.first(where: { $0.id == name }) { verify(model) }
        }
    }
    func handleBackgroundEvents(_ completion: @escaping () -> Void) {
        backgroundCompletion = completion
        _ = session
        finishBackgroundEventsIfPossible()
    }
    private func finishBackgroundEventsIfPossible() {
        guard eventsFinished, verification.isEmpty, let completion = backgroundCompletion else { return }
        backgroundCompletion = nil; eventsFinished = false; completion()
    }
    nonisolated func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        guard let name = downloadTask.taskDescription else { return }
        let taskID = downloadTask.taskIdentifier
        MainActor.assumeIsolated {
            guard self.records[name]?.taskID == taskID, self.records[name]?.phase == .downloading,
                  Date().timeIntervalSince(self.lastProgress[name] ?? .distantPast) > 0.4 else { return }
            self.lastProgress[name] = Date()
            self.records[name]?.bytes = totalBytesWritten
            self.records[name]?.message = "Wird heruntergeladen"
            try? self.persist()
        }
    }
    nonisolated func urlSession(_ session: URLSession, taskIsWaitingForConnectivity task: URLSessionTask) {
        MainActor.assumeIsolated {
            guard let name = task.taskDescription, records[name]?.taskID == task.taskIdentifier, records[name]?.phase == .downloading else { return }
            records[name]?.message = "Warte auf Internetverbindung"; try? persist()
        }
    }
    nonisolated func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        guard let name = downloadTask.taskDescription, let model = LocalModel.all.first(where: { $0.id == name }) else { return }
        let taskID = downloadTask.taskIdentifier
        do {
            guard let response = downloadTask.response as? HTTPURLResponse, response.statusCode == 200 else {
                throw ModelTransferError.server((downloadTask.response as? HTTPURLResponse)?.statusCode ?? 0)
            }
            let filename = name + "." + String(taskID) + "." + UUID().uuidString + ".download"
            let destination = Self.folder.appendingPathComponent(filename)
            // URLSession deletes its temporary file when this delegate returns. Move synchronously first.
            try FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true, attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
            try FileManager.default.moveItem(at: location, to: destination)
            try FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication], ofItemAtPath: destination.path)
            MainActor.assumeIsolated {
                guard self.records[name]?.taskID == taskID, self.records[name]?.phase == .downloading else {
                    try? FileManager.default.removeItem(at: destination); return
                }
                self.records[name]?.phase = .verifying; self.records[name]?.bytes = model.bytes
                self.records[name]?.stagedFile = filename; self.records[name]?.message = "Download fertig · Integrität wird geprüft"
                do { try self.persist(); self.verify(model) } catch { self.fail(name, error: error) }
            }
        } catch {
            MainActor.assumeIsolated {
                guard self.records[name]?.taskID == taskID, self.records[name]?.phase == .downloading else { return }
                self.fail(name, error: error)
            }
        }
    }
    nonisolated func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        guard let error, let name = task.taskDescription else { return }
        let taskID = task.taskIdentifier
        MainActor.assumeIsolated {
            guard self.records[name]?.taskID == taskID, self.records[name]?.phase == .downloading else { return }
            if (error as? URLError)?.code == .cancelled {
                self.records[name]?.message = "Unterbrochen · wird beim Öffnen fortgesetzt"
                try? self.persist(); return
            }
            self.fail(name, error: error)
        }
    }
    nonisolated func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        MainActor.assumeIsolated { self.eventsFinished = true; self.finishBackgroundEventsIfPossible() }
    }
    private func verify(_ model: LocalModel) {
        guard verification[model.id] == nil, let file = records[model.id]?.stagedFile else { return }
        guard file == URL(fileURLWithPath: file).lastPathComponent, file.hasPrefix(model.id + "."), file.hasSuffix(".download") else {
            fail(model.id, error: ModelTransferError.storage); return
        }
        let source = Self.folder.appendingPathComponent(file)
        let lease = UIApplication.shared.beginBackgroundTask(withName: "Sprachmodell prüfen") { [weak self] in
            Task { @MainActor in self?.verification[model.id]?.cancel() }
        }
        verification[model.id] = Task {
            defer {
                verification[model.id] = nil
                if lease != .invalid { UIApplication.shared.endBackgroundTask(lease) }
                finishBackgroundEventsIfPossible()
            }
            do {
                _ = try await ModelLibrary.shared.install(from: source, expected: model)
                records[model.id]?.phase = .ready; records[model.id]?.message = "Geprüft · bereit zur Verwendung"
                records[model.id]?.completedAt = Date()
                records[model.id]?.completedInBackground = UIApplication.shared.applicationState == .background
                records[model.id]?.stagedFile = nil
                try persist()
                try? FileManager.default.removeItem(at: source)
                onInstalled?()
                await notifyReady(model)
            } catch is CancellationError {
                if records[model.id]?.phase == .cancelled { try? FileManager.default.removeItem(at: source) }
                else { records[model.id]?.message = "Gespeichert · Prüfung wird beim Öffnen fortgesetzt"; try? persist() }
            } catch { fail(model.id, error: error); try? FileManager.default.removeItem(at: source) }
        }
    }
    private func fail(_ name: String, error: Error) {
        var record = records[name] ?? ModelDownloadRecord(phase: .failed)
        record.phase = .failed; record.message = ModelTransferError.message(error)
        records[name] = record; try? persist()
    }
    private func notifyReady(_ model: LocalModel) async {
        guard records[model.id]?.phase == .ready, records[model.id]?.notified == false else { return }
        guard await ModelLibrary.shared.inventory().contains(where: { $0.id == model.id && $0.installed }) else { return }
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional else { return }
        let content = UNMutableNotificationContent()
        content.title = "Sprachmodell bereit"
        content.body = model.label + " wurde heruntergeladen, geprüft und kann jetzt verwendet werden."
        content.sound = .default
        do {
            try await center.add(UNNotificationRequest(identifier: "model-ready-" + model.id, content: content, trigger: nil))
            records[model.id]?.notified = true; try persist()
        } catch { notificationHint = "Modell bereit; Mitteilung konnte nicht zugestellt werden." }
    }
}
