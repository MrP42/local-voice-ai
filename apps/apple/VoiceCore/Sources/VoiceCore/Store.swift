import Foundation
import CryptoKit
import Darwin

public enum StorageCheckpoint: String, CaseIterable, Sendable { case beforeWrite, fileSynced, captureMetadataSynced, captureRenamed, captureCommitted, replyCommitted, replyAckCommitted, jobStarted, transcriptCommitted, generatedReplyCommitted }

public enum VoiceError: Error { case version, invalid, conflict, full, missing, persistence }
public enum CaptureState: String, Codable, Sendable { case saved, accepted, deferred, answered }
public struct Receipt: Codable, Equatable, Sendable {
    public let sessionId: UUID
    public let messageId: UUID
    public let receiptId: UUID
}
public struct Packet: Codable, Sendable {
    public var schemaVersion = 1
    public var sessionId: UUID
    public var messageId: UUID
    public var kind: String
    public var createdAt: Date
    public struct Payload: Codable, Sendable { public var audio: Data }
    public var payload: Payload
    public var audio: Data { get { payload.audio } set { payload.audio = newValue } }
    public static func capture(audio: Data) -> Packet {
        Packet(sessionId: UUID(), messageId: UUID(), kind: "capture", createdAt: Date(), payload: Payload(audio: audio))
    }
}
public struct Entry: Codable, Identifiable, Sendable {
    public var id: UUID { receipt.sessionId }
    public let receipt: Receipt
    public let createdAt: Date
    public let digest: String
    public var state: CaptureState
    public var transcript: String?
    public var reply: String?
    public var timings: [String: Double] = [:]
    public var replyMessageId: UUID?
    public var replyAcknowledged: Bool?
    public var replyReceipt: Receipt?
    public var replyToWatch: Bool?
    public var job: JobRecord?
    public static func pendingDelivery(in entries: [Entry]) -> [Entry] {
        entries.filter { $0.state != .answered }.sorted {
            let left = $0.state == .saved ? 0 : 1
            let right = $1.state == .saved ? 0 : 1
            if left != right { return left < right }
            if $0.createdAt != $1.createdAt { return $0.createdAt < $1.createdAt }
            return $0.id.uuidString < $1.id.uuidString
        }
    }
}

/// Access from one serial executor. A directory rename commits audio and metadata together.
/// The prototype retains every accepted audio file; reaching quota rejects new captures.
public final class DurableStore {
    public let root: URL
    private let limit: Int
    let fm = FileManager.default
    var cachedInventory: StorageInventory?
    let temporaryLimit: Int
    let fault: ((StorageCheckpoint) throws -> Void)?
    public init(root: URL, limit: Int = 64 * 1024 * 1024, temporaryLimit: Int = 8 * 1024 * 1024, fault: ((StorageCheckpoint) throws -> Void)? = nil) throws {
        self.root = root; self.limit = limit; self.temporaryLimit = temporaryLimit; self.fault = fault
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }
    public func entries() throws -> [Entry] {
        try inventory(includeUsage: false).entries
    }
    public func canStartRecording() throws -> Bool {
        try committedAudioBytes() + 1024 * 1024 <= limit && diskBytes(at: root) + 1024 * 1024 <= limit + temporaryLimit
    }
    private func committedAudioBytes() throws -> Int {
        try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .filter { UUID(uuidString: $0.lastPathComponent) != nil }
            .reduce(0) { total, folder in
                let audio = folder.appendingPathComponent("audio.m4a")
                // An absent damaged original reserves a full capture; it cannot create free quota.
                return try total + (fm.fileExists(atPath: audio.path) ? diskBytes(at: audio) : 1024 * 1024)
            }
    }
    public func audioURL(for id: UUID) -> URL { root.appendingPathComponent(id.uuidString).appendingPathComponent("audio.m4a") }
    public func audio(for id: UUID) throws -> Data { try Data(contentsOf: audioURL(for: id)) }
    public func packet(for entry: Entry) throws -> Packet {
        let data = try audio(for: entry.id)
        guard SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == entry.digest else {
            invalidateInventory(); throw VoiceError.persistence
        }
        return Packet(sessionId: entry.id, messageId: entry.receipt.messageId, kind: "capture", createdAt: entry.createdAt, payload: Packet.Payload(audio: data))
    }
    public func accept(_ packet: Packet, replyToWatch: Bool = false) throws -> Receipt {
        do {
        guard packet.schemaVersion == 1 else { throw VoiceError.version }
        guard packet.kind == "capture", !packet.audio.isEmpty, packet.audio.count <= 1024 * 1024 else { throw VoiceError.invalid }
        let digest = SHA256.hash(data: packet.audio).map { String(format: "%02x", $0) }.joined()
        let inventory = try inventory(includeUsage: false)
        let current = inventory.entries
        for issue in inventory.issues where issue.sessionId != nil {
            guard issue.sessionId != packet.sessionId, let messageId = issue.messageId,
                  messageId != packet.messageId else { throw VoiceError.persistence }
        }
        if var existing = current.first(where: { $0.id == packet.sessionId || $0.receipt.messageId == packet.messageId }) {
            guard existing.id == packet.sessionId, existing.receipt.messageId == packet.messageId, existing.digest == digest else { throw VoiceError.conflict }
            guard try audio(for: existing.id) == packet.audio else { throw VoiceError.persistence }
            if replyToWatch && existing.replyToWatch != true { existing.replyToWatch = true; try save(existing) }
            return existing.receipt
        }
        // Include damaged entries in quota; corruption must never create free space.
        let used = try committedAudioBytes()
        guard used + packet.audio.count <= limit else { throw VoiceError.full }
        let receipt = Receipt(sessionId: packet.sessionId, messageId: packet.messageId, receiptId: UUID())
        let entry = Entry(receipt: receipt, createdAt: packet.createdAt, digest: digest, state: .saved, timings: ["capture_saved_at_ms": Date().timeIntervalSinceReferenceDate * 1000], replyToWatch: replyToWatch)
        let metadata = try JSONEncoder().encode(entry)
        let identity = try JSONEncoder().encode(receipt)
        guard try temporaryBytes() + packet.audio.count + metadata.count + identity.count <= temporaryLimit else { throw VoiceError.full }
        let staging = root.appendingPathComponent(".partial-" + UUID().uuidString)
        try fm.createDirectory(at: staging, withIntermediateDirectories: false)
        defer { try? fm.removeItem(at: staging) }
        try write(packet.audio, to: staging.appendingPathComponent("audio.m4a"))
        try write(metadata, to: staging.appendingPathComponent("entry.json"))
        try write(identity, to: staging.appendingPathComponent("identity.json"))
        try syncDirectory(staging)
        try fault?(.captureMetadataSynced)
        try fm.moveItem(at: staging, to: root.appendingPathComponent(packet.sessionId.uuidString))
        try fault?(.captureRenamed)
        try syncDirectory(root)
        cacheEntry(entry)
        try fault?(.captureCommitted)
        return receipt
        } catch { invalidateInventory(); throw error }
    }
    /// Draft filenames provide stable identity even if the process exits before removing the source.
    public func recoverRecording(at url: URL) throws -> Receipt {
        let name = url.lastPathComponent
        guard url.standardizedFileURL.deletingLastPathComponent() == root.standardizedFileURL,
              name.hasPrefix(".recording-"), name.hasSuffix(".m4a"),
              let id = UUID(uuidString: String(name.dropFirst(11).dropLast(4))) else { throw VoiceError.invalid }
        let attributes = try fm.attributesOfItem(atPath: url.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular else { throw VoiceError.invalid }
        let packet = Packet(sessionId: id, messageId: id, kind: "capture",
                            createdAt: attributes[.creationDate] as? Date ?? Date(),
                            payload: Packet.Payload(audio: try Data(contentsOf: url)))
        let receipt = try accept(packet)
        // Failure to remove a redundant draft cannot undo its durable acceptance.
        try? fm.removeItem(at: url)
        if !fm.fileExists(atPath: url.path) {
            cachedInventory?.issues.removeAll { $0.url.lastPathComponent == url.lastPathComponent }
        }
        return receipt
    }
    /// Only redundant, completed transfer copies are removed. Original audio remains untouched.
    public func cleanupTransfers(keeping active: Set<URL>) throws {
        let retained = Set(active.map { $0.standardizedFileURL })
        for url in try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil) {
            let name = url.lastPathComponent
            guard name.hasPrefix(".transfer-"), name.hasSuffix(".json"),
                  UUID(uuidString: String(name.dropFirst(10).dropLast(5))) != nil,
                  !retained.contains(url.standardizedFileURL) else { continue }
            try fm.removeItem(at: url)
        }
    }
    public func update(_ id: UUID, transcript: String? = nil, reply: String? = nil, state: CaptureState, timing: (String, Double)? = nil) throws {
        var entry = try loadEntry(for: id)
        if let transcript {
            guard transcript.count <= 16000 else { throw VoiceError.invalid }
            entry.transcript = transcript
        }
        if let reply {
            guard !reply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, reply.count <= 500 else { throw VoiceError.invalid }
            if let previous = entry.reply, previous != reply { throw VoiceError.conflict }
            entry.reply = reply
            if entry.replyMessageId == nil { entry.replyMessageId = UUID() }
        }
        if entry.state != .answered { entry.state = state }
        if entry.reply != nil, entry.state == .answered { entry.job?.phase = .completed }
        if let timing { entry.timings[timing.0] = timing.1 }
        try save(entry)
        if transcript != nil { try fault?(.transcriptCommitted) }
        if reply != nil { try fault?(.generatedReplyCommitted) }
    }
    /// Prepare and persist a stable response identity before handing it to transport.
    public func replyEnvelope(for id: UUID) throws -> VoiceEnvelope {
        var entry = try loadEntry(for: id)
        guard let reply = entry.reply else { throw VoiceError.missing }
        if entry.replyMessageId == nil {
            entry.replyMessageId = UUID()
            try save(entry)
        }
        return VoiceEnvelope(sessionId: id, transcript: entry.transcript, reply: reply, replyMessageId: entry.replyMessageId!)
    }
    public func acceptReply(_ envelope: VoiceEnvelope) throws -> (receipt: Receipt, isNew: Bool) {
        guard envelope.schemaVersion == 1 else { throw VoiceError.version }
        guard envelope.kind == "reply", let id = envelope.sessionId, let text = envelope.reply,
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.count <= 500, (envelope.transcript?.count ?? 0) <= 16000 else { throw VoiceError.invalid }
        var entry = try loadEntry(for: id)
        if let previous = entry.reply {
            guard previous == text, entry.transcript == envelope.transcript else { throw VoiceError.conflict }
        }
        if let receipt = entry.replyReceipt {
            guard receipt.messageId == envelope.messageId else { throw VoiceError.conflict }
            return (receipt, false)
        }
        if let transcript = entry.transcript {
            guard transcript == envelope.transcript else { throw VoiceError.conflict }
        }
        if let issued = entry.replyMessageId {
            guard issued == envelope.messageId else { throw VoiceError.conflict }
        }
        let isNew = entry.reply == nil
        let receipt = Receipt(sessionId: id, messageId: envelope.messageId, receiptId: UUID())
        let receivedAt = Date().timeIntervalSinceReferenceDate * 1000
        entry.timings["reply_received_at_ms"] = receivedAt
        if let start = entry.timings["capture_end_at_ms"] ?? entry.timings["capture_saved_at_ms"], receivedAt >= start {
            entry.timings["reply_e2e_ms"] = receivedAt - start
        }
        entry.reply = text; entry.transcript = envelope.transcript
        entry.replyMessageId = envelope.messageId; entry.replyReceipt = receipt; entry.state = .answered
        try save(entry)
        try fault?(.replyCommitted)
        return (receipt, isNew)
    }
    public func acknowledgeReply(_ receipt: Receipt) throws {
        var entry = try loadEntry(for: receipt.sessionId)
        guard entry.replyMessageId == receipt.messageId else { throw VoiceError.invalid }
        entry.replyAcknowledged = true
        try save(entry)
        try fault?(.replyAckCommitted)
    }
    func save(_ entry: Entry) throws {
        let folder = root.appendingPathComponent(entry.id.uuidString)
        try write(JSONEncoder().encode(entry), to: folder.appendingPathComponent("entry.json"))
        try syncDirectory(folder)
        cacheEntry(entry)
    }
    func write(_ data: Data, to url: URL) throws {
        do {
        try fault?(.beforeWrite)
        try data.write(to: url, options: .atomic)
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.synchronize()
        try fault?(.fileSynced)
        } catch { invalidateInventory(); throw error }
    }
    func syncDirectory(_ url: URL) throws {
        let descriptor = open(url.path, O_RDONLY)
        guard descriptor >= 0 else { invalidateInventory(); throw VoiceError.persistence }
        defer { close(descriptor) }
        guard fsync(descriptor) == 0 else { invalidateInventory(); throw VoiceError.persistence }
    }
}
