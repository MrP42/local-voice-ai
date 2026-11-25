import Foundation
import CryptoKit

/// Ein Datensatz je Seite, atomar einschließlich Sync-Basis und lokaler Aufnahmen.
/// Alle Zugriffe auf dem MainActor; kein Cache, damit ein Neustart denselben Stand sieht.
/// Gelöschte Seiten bleiben als Grabstein mit ihrem Inhalt erhalten (Papierkorb).
@MainActor
public final class ReadingPageStore {
    public let root: URL
    private var accountID: String?

    private struct Record: Codable {
        var json: Data?
        var revision: Int64 = 0
        var syncedHash: String?
        var syncedDeleted = false
        var deleted = false
        var captures: [ReadingCapture] = []
        var imagesID: String?
        var composedText: String?
        var rejectedHash: String?
        var accountID: String?
    }

    public struct Pending: Equatable, Sendable {
        public let id: String
        public let json: Data?
        public let baseRevision: Int64
        public let hash: String
        public var deleted: Bool { json == nil }
    }

    public init(root: URL) throws {
        self.root = root
        do { try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true) }
        catch { throw ReadingPageError.persistence }
    }

    public static func sharedRoot() -> URL? {
        #if canImport(Darwin)
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: MeetingInbox.appGroup)?
            .appendingPathComponent("ReadingPages", isDirectory: true)
        #else
        return nil
        #endif
    }

    /// Revisionen gehören zum Konto, Inhalte zum Gerät. Ein Kontowechsel darf
    /// weder alte Revisionen verwenden noch lokale Seiten entfernen.
    public func useAccount(_ id: String) { accountID = id }

    private func scoped(_ record: Record) -> Record {
        guard record.accountID != accountID else { return record }
        var copy = record
        copy.accountID = accountID; copy.revision = 0; copy.syncedHash = nil
        copy.syncedDeleted = false; copy.rejectedHash = nil
        return copy
    }

    public func pages() throws -> [ReadingPage] {
        try records().compactMap { id, record in
            guard !record.deleted, let json = record.json else { return nil }
            return try ReadingPage(id: id, json: json, maxBytes: AblageJSON.defaultMaxBytes)
        }
    }

    public func save(_ page: ReadingPage) throws {
        let json = page.json()
        _ = try ReadingPage(id: page.id, json: json, maxBytes: AblageJSON.defaultMaxBytes)
        var record = try read(page.id) ?? Record()
        record.json = json; record.deleted = false
        try write(record, page.id)
    }

    public func delete(_ id: String) throws {
        guard var record = try read(id) else { return }
        record.deleted = true
        try write(record, id)
    }

    public func archivedPage(_ id: String) throws -> ReadingPage? {
        guard let r = try read(id), r.deleted, let json = r.json else { return nil }
        return try ReadingPage(id: id, json: json, maxBytes: AblageJSON.defaultMaxBytes)
    }

    public func captures(for id: String) throws -> [ReadingCapture] { try read(id)?.captures ?? [] }

    public func imageURL(named name: String, for id: String) throws -> URL {
        guard ReadingPage.validID(id), Self.safeImageName(name) else { throw ReadingPageError.invalid }
        let imagesID = try read(id)?.imagesID ?? id
        guard ReadingPage.validID(imagesID) else { throw ReadingPageError.invalid }
        return root.appendingPathComponent("Images", isDirectory: true)
            .appendingPathComponent(imagesID, isDirectory: true).appendingPathComponent(name)
    }

    /// S2 legt das Bild vor seinem Metadatenverweis ab. Ohne automatisches Aufräumen:
    /// auch beim Löschen/Ersetzen einer Aufnahme bleiben Originalbilder erhalten.
    public func storeImage(_ data: Data, named name: String, for id: String) throws {
        guard !data.isEmpty, data.count <= TextInbox.maxImageBytes,
              let record = try read(id), !record.deleted else { throw ReadingPageError.invalid }
        let target = try imageURL(named: name, for: id)
        do {
            try FileManager.default.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
            try writeData(data, to: target)
        } catch { throw ReadingPageError.persistence }
    }

    /// `composedText` ist der Text beim letzten ausdrücklich übernommenen Zusammenbau.
    /// S2 muss bei Abweichung eine Wahl anbieten; dieser Speicher überschreibt nie Text.
    public func setCaptures(_ captures: [ReadingCapture], for id: String, composedText: String?) throws {
        guard var record = try read(id), !record.deleted else { throw ReadingPageError.invalid }
        guard Set(captures.map(\.id)).count == captures.count,
              captures.allSatisfy({ Self.safeImageName($0.image) }) else { throw ReadingPageError.invalid }
        record.captures = captures; record.composedText = composedText
        try write(record, id)
    }

    public func requiresCompositionChoice(_ id: String) throws -> Bool {
        guard let record = try read(id), let json = record.json else { throw ReadingPageError.invalid }
        let page = try ReadingPage(id: id, json: json, maxBytes: AblageJSON.defaultMaxBytes)
        return record.composedText != page.text || record.captures.contains { $0.textEdited || $0.pendingRecognition != nil }
    }

    /// Beschädigte Dateien sind zurückgehalten, niemals implizit gelöscht.
    public func pending() throws -> [Pending] {
        try records().compactMap { id, stored in
            let record = scoped(stored)
            let hash = record.deleted ? "" : Self.hash(record.json ?? Data())
            if record.rejectedHash == hash { return nil }
            if record.deleted {
                if record.syncedDeleted || record.revision == 0 { return nil }
            } else {
                guard let json = record.json, json.count <= ReadingPage.maxPlaintext else { return nil }
                if !record.syncedDeleted && record.syncedHash == hash { return nil }
            }
            return Pending(id: id, json: record.deleted ? nil : record.json, baseRevision: record.revision, hash: hash)
        }
    }

    public func oversizedIDs() throws -> [String] {
        try records().filter { !$0.1.deleted && ($0.1.json?.count ?? 0) > ReadingPage.maxPlaintext }.map { $0.0 }
    }

    /// Bestätigt den GESENDETEN Hash, auch wenn inzwischen lokal weitergearbeitet wurde.
    public func acknowledge(_ sent: Pending, revision: Int64) throws {
        try acknowledge(id: sent.id, hash: sent.hash, deleted: sent.deleted, revision: revision)
    }

    public func acknowledge(id: String, hash: String, deleted: Bool, revision: Int64) throws {
        guard let stored = try read(id) else { return }
        var record = scoped(stored)
        guard revision > record.revision else { return }
        record.revision = revision; record.syncedHash = hash; record.syncedDeleted = deleted
        record.rejectedHash = nil
        try write(record, id)
    }

    public func reject(_ sent: Pending) throws {
        try reject(id: sent.id, hash: sent.hash)
    }

    public func reject(id: String, hash: String) throws {
        guard let stored = try read(id) else { return }
        var record = scoped(stored)
        record.rejectedHash = hash
        try write(record, id)
    }

    public func resetRevision(_ id: String) throws {
        guard let stored = try read(id) else { return }
        var record = scoped(stored)
        record.revision = 0; record.syncedHash = nil; record.syncedDeleted = false
        try write(record, id)
    }

    /// Server gewinnt das Original; ungesicherter abweichender Inhalt wird wie
    /// apply_page als neue Seite gesichert. Basis + Inhalt werden zusammen geschrieben.
    public func apply(id: String, json: Data?, revision: Int64, deviceName: String) throws {
        guard revision > 0 else { throw ReadingPageError.invalid }
        var record = scoped(try read(id) ?? Record())
        guard revision > record.revision else { return }
        let incoming = try json.map { try ReadingPage(id: id, json: $0) }
        if !record.deleted, let localJSON = record.json {
            let local = try ReadingPage(id: id, json: localJSON, maxBytes: AblageJSON.defaultMaxBytes)
            let dirty = record.syncedDeleted || record.syncedHash != Self.hash(local.json())
            if dirty && (incoming.map { local.differs(from: $0) } ?? true) {
                // Deterministisch: Ein Abbruch nach der Kopie erzeugt beim Wiederholen
                // keine zweite Kopie. Der Namensraum bleibt Desktop-/Hub-kompatibel.
                let copyID = "page_" + String(Self.hash(Data((id + ":" + Self.hash(local.json()) + ":" + String(revision)).utf8)).prefix(40))
                if try read(copyID) == nil {
                    var copy = try ReadingPage(id: copyID, json: local.json(), maxBytes: AblageJSON.defaultMaxBytes)
                    copy.title = local.title + " (Konflikt von " + deviceName + ")"
                    var saved = Record(json: copy.json())
                    saved.captures = record.captures; saved.composedText = record.composedText
                    // Kopien und Grabsteine teilen die dauerhaft erhaltenen Originalbilder.
                    saved.imagesID = record.imagesID ?? id
                    try write(saved, copyID)
                }
            }
        }
        if let incoming { record.json = incoming.json() }
        record.deleted = incoming == nil
        record.revision = revision; record.syncedDeleted = record.deleted
        record.syncedHash = incoming.map { Self.hash($0.json()) } ?? ""
        record.rejectedHash = nil
        try write(record, id)
    }

    private static func hash(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private func url(_ id: String) throws -> URL {
        guard ReadingPage.validID(id) else { throw ReadingPageError.invalid }
        return root.appendingPathComponent(id + ".json")
    }

    private func read(_ id: String) throws -> Record? {
        let target = try url(id)
        guard FileManager.default.fileExists(atPath: target.path) else { return nil }
        do {
            let record = try JSONDecoder().decode(Record.self, from: Data(contentsOf: target))
            if let json = record.json { _ = try ReadingPage(id: id, json: json, maxBytes: AblageJSON.defaultMaxBytes) }
            else if !record.deleted { throw ReadingPageError.invalid }
            return record
        } catch { throw ReadingPageError.invalid }
    }

    private func write(_ record: Record, _ id: String) throws {
        let target = try url(id)
        do {
            try writeData(JSONEncoder().encode(record), to: target)
        } catch { throw ReadingPageError.persistence }
    }

    private func writeData(_ data: Data, to target: URL) throws {
        #if os(iOS)
        try data.write(to: target, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        #else
        try data.write(to: target, options: .atomic)
        #endif
    }

    private func records() throws -> [(String, Record)] {
        let urls: [URL]
        do { urls = try FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil) }
        catch { throw ReadingPageError.persistence }
        return urls.sorted { $0.lastPathComponent < $1.lastPathComponent }.compactMap { url in
            guard url.pathExtension == "json" else { return nil }
            let id = url.deletingPathExtension().lastPathComponent
            guard ReadingPage.validID(id), let record = try? read(id) else { return nil }
            return (id, record)
        }
    }

    private static func safeImageName(_ name: String) -> Bool {
        !name.isEmpty && name.utf8.count <= 150 && !name.hasPrefix(".") && !name.contains("..")
            && !name.contains("/") && !name.contains("\\") && !name.contains(":")
    }
}
