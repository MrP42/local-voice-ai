import Foundation
import CryptoKit

public struct StorageIssue: Identifiable, Sendable {
    public var id: String { url.lastPathComponent }
    public let url: URL
    public let sessionId: UUID?
    public let messageId: UUID?
    public let reason: String
}
public struct StorageInventory: Sendable {
    public var entries: [Entry] = []
    public var issues: [StorageIssue] = []
    public var temporaryBytes = 0
    public var totalBytes = 0
}

extension DurableStore {
    public func invalidateInventory() { cachedInventory = nil }
    func cacheEntry(_ entry: Entry) {
        guard var cached = cachedInventory else { return }
        if let index = cached.entries.firstIndex(where: { $0.id == entry.id }) { cached.entries[index] = entry }
        else { cached.entries.append(entry) }
        cached.entries.sort { $0.createdAt > $1.createdAt }
        cached.issues.removeAll { $0.sessionId == entry.id }
        cachedInventory = cached
    }

    /// Isolation is in-place: damaged directories keep their names, identities and original bytes.
    public func inventory(includeUsage: Bool = true) throws -> StorageInventory {
        if !includeUsage, var cached = cachedInventory {
            cached.totalBytes = 0; cached.temporaryBytes = 0
            return cached
        }
        var result = StorageInventory()
        for url in try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil) {
            let name = url.lastPathComponent
            if let id = UUID(uuidString: name) {
                do {
                    let entry = try readEntry(at: url)
                    if fm.fileExists(atPath: url.appendingPathComponent("identity.json").path) {
                        let receipt = try JSONDecoder().decode(Receipt.self, from: Data(contentsOf: url.appendingPathComponent("identity.json")))
                        guard receipt == entry.receipt else { throw VoiceError.conflict }
                    }
                    result.entries.append(entry)
                } catch {
                    let receipt = try? JSONDecoder().decode(Receipt.self, from: Data(contentsOf: url.appendingPathComponent("identity.json")))
                    result.issues.append(StorageIssue(url: url, sessionId: id, messageId: receipt?.messageId,
                                                      reason: "Verlauf beschädigt – Originaldateien erhalten"))
                }
            } else if name.hasPrefix(".recording-") || name.hasPrefix(".partial-") {
                result.issues.append(StorageIssue(url: url, sessionId: nil, messageId: nil,
                                                  reason: "Unbestätigte Aufnahme – Wiederherstellung erforderlich"))
            }
            if includeUsage {
                let bytes = try diskBytes(at: url)
                result.totalBytes += bytes
                if name.hasPrefix(".transfer-") || name.hasPrefix(".partial-") || name == ".incoming-parts" { result.temporaryBytes += bytes }
            }
        }
        result.entries.sort { $0.createdAt > $1.createdAt }
        result.issues.sort { $0.id < $1.id }
        cachedInventory = result
        return result
    }
    func readEntry(at folder: URL) throws -> Entry {
        guard try fm.attributesOfItem(atPath: folder.path)[.type] as? FileAttributeType == .typeDirectory else { throw VoiceError.persistence }
        let entry = try JSONDecoder().decode(Entry.self, from: Data(contentsOf: folder.appendingPathComponent("entry.json")))
        guard entry.id.uuidString == folder.lastPathComponent || folder.lastPathComponent.hasPrefix(".partial-"),
              !entry.digest.isEmpty else { throw VoiceError.persistence }
        let audio = folder.appendingPathComponent("audio.m4a")
        let attributes = try fm.attributesOfItem(atPath: audio.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              let bytes = attributes[.size] as? NSNumber, bytes.intValue > 0, bytes.intValue <= 1024 * 1024 else { throw VoiceError.persistence }
        let data = try Data(contentsOf: audio)
        guard SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == entry.digest else { throw VoiceError.persistence }
        return entry
    }
    func loadEntry(for id: UUID) throws -> Entry {
        do {
        let folder = root.appendingPathComponent(id.uuidString)
        guard fm.fileExists(atPath: folder.path) else { throw VoiceError.missing }
        let entry = try readEntry(at: folder)
        let identity = folder.appendingPathComponent("identity.json")
        if fm.fileExists(atPath: identity.path) {
            guard try JSONDecoder().decode(Receipt.self, from: Data(contentsOf: identity)) == entry.receipt else { throw VoiceError.conflict }
        }
        return entry
        } catch { invalidateInventory(); throw error }
    }
    func diskBytes(at url: URL) throws -> Int {
        let attributes = try fm.attributesOfItem(atPath: url.path)
        if attributes[.type] as? FileAttributeType == .typeDirectory {
            return try fm.contentsOfDirectory(at: url, includingPropertiesForKeys: nil).reduce(0) { try $0 + diskBytes(at: $1) }
        }
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              let bytes = attributes[.size] as? NSNumber else { throw VoiceError.persistence }
        return bytes.intValue
    }
    /// Additive migration only: never replace a pre-existing identity manifest or metadata.
    public func migrateIdentities() throws {
        for entry in try inventory().entries {
            let folder = root.appendingPathComponent(entry.id.uuidString)
            let url = folder.appendingPathComponent("identity.json")
            if !fm.fileExists(atPath: url.path) {
                try write(JSONEncoder().encode(entry.receipt), to: url)
                try syncDirectory(folder)
            }
        }
    }
    /// A complete transaction can be committed after restart; unidentified bytes remain visible.
    public func recoverStaging() throws {
        defer { invalidateInventory() }
        for stage in try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            where stage.lastPathComponent.hasPrefix(".partial-") {
            guard UUID(uuidString: String(stage.lastPathComponent.dropFirst(9))) != nil,
                  let entry = try? readEntry(at: stage),
                  let audio = try? Data(contentsOf: stage.appendingPathComponent("audio.m4a")),
                  SHA256.hash(data: audio).map({ String(format: "%02x", $0) }).joined() == entry.digest else { continue }
            let destination = root.appendingPathComponent(entry.id.uuidString)
            if fm.fileExists(atPath: destination.path) {
                guard let existing = try? readEntry(at: destination), existing.receipt == entry.receipt,
                      existing.digest == entry.digest, try self.audio(for: entry.id) == audio else { continue }
                try fm.removeItem(at: stage)
            } else {
                // The transaction's durable identity is preserved, not regenerated by accept().
                try write(JSONEncoder().encode(entry.receipt), to: stage.appendingPathComponent("identity.json"))
                try syncDirectory(stage)
                try fm.moveItem(at: stage, to: destination)
            }
            try syncDirectory(root)
        }
    }
    public func temporaryBytes(excludingIncomingParts: Bool = false) throws -> Int {
        try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .filter { $0.lastPathComponent.hasPrefix(".partial-") || $0.lastPathComponent.hasPrefix(".transfer-") || $0.lastPathComponent == ".incoming-parts" && !excludingIncomingParts }
            .reduce(0) { try $0 + diskBytes(at: $1) }
    }
    public func prepareTransfer(for id: UUID, data: Data) throws -> URL {
        guard try entries().contains(where: { $0.id == id }) else { throw VoiceError.missing }
        let url = root.appendingPathComponent(".transfer-" + id.uuidString + ".json")
        // Include old bytes as well: an atomic replacement temporarily needs both copies.
        guard try temporaryBytes() + data.count <= temporaryLimit else { throw VoiceError.full }
        try write(data, to: url)
        try syncDirectory(root)
        return url
    }
}
