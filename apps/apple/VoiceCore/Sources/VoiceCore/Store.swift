import Foundation
import CryptoKit
import Darwin

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
    private let fm = FileManager.default
    public init(root: URL, limit: Int = 64 * 1024 * 1024) throws {
        self.root = root; self.limit = limit
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }
    public func entries() throws -> [Entry] {
        try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .filter { UUID(uuidString: $0.lastPathComponent) != nil }
            .map { try JSONDecoder().decode(Entry.self, from: Data(contentsOf: $0.appendingPathComponent("entry.json"))) }
            .sorted { $0.createdAt > $1.createdAt }
    }
    public func audioURL(for id: UUID) -> URL { root.appendingPathComponent(id.uuidString).appendingPathComponent("audio.m4a") }
    public func audio(for id: UUID) throws -> Data { try Data(contentsOf: audioURL(for: id)) }
    public func packet(for entry: Entry) throws -> Packet {
        Packet(sessionId: entry.id, messageId: entry.receipt.messageId, kind: "capture", createdAt: entry.createdAt, payload: Packet.Payload(audio: try audio(for: entry.id)))
    }
    public func accept(_ packet: Packet, replyToWatch: Bool = false) throws -> Receipt {
        guard packet.schemaVersion == 1 else { throw VoiceError.version }
        guard packet.kind == "capture", !packet.audio.isEmpty, packet.audio.count <= 1024 * 1024 else { throw VoiceError.invalid }
        let digest = SHA256.hash(data: packet.audio).map { String(format: "%02x", $0) }.joined()
        let current = try entries()
        if var existing = current.first(where: { $0.id == packet.sessionId || $0.receipt.messageId == packet.messageId }) {
            guard existing.id == packet.sessionId, existing.receipt.messageId == packet.messageId, existing.digest == digest else { throw VoiceError.conflict }
            guard try audio(for: existing.id) == packet.audio else { throw VoiceError.persistence }
            if replyToWatch && existing.replyToWatch != true { existing.replyToWatch = true; try save(existing) }
            return existing.receipt
        }
        let used = try current.reduce(0) { total, entry in
            let attributes = try fm.attributesOfItem(atPath: audioURL(for: entry.id).path)
            guard let size = attributes[.size] as? NSNumber else { throw VoiceError.persistence }
            return total + size.intValue
        }
        guard used + packet.audio.count <= limit else { throw VoiceError.full }
        let receipt = Receipt(sessionId: packet.sessionId, messageId: packet.messageId, receiptId: UUID())
        let entry = Entry(receipt: receipt, createdAt: packet.createdAt, digest: digest, state: .saved, replyToWatch: replyToWatch)
        let staging = root.appendingPathComponent(".partial-" + UUID().uuidString)
        try fm.createDirectory(at: staging, withIntermediateDirectories: false)
        defer { try? fm.removeItem(at: staging) }
        try write(packet.audio, to: staging.appendingPathComponent("audio.m4a"))
        try write(JSONEncoder().encode(entry), to: staging.appendingPathComponent("entry.json"))
        try syncDirectory(staging)
        try fm.moveItem(at: staging, to: root.appendingPathComponent(packet.sessionId.uuidString))
        try syncDirectory(root)
        return receipt
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
        guard var entry = try entries().first(where: { $0.id == id }) else { throw VoiceError.missing }
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
        if let timing { entry.timings[timing.0] = timing.1 }
        let folder = root.appendingPathComponent(id.uuidString)
        try write(JSONEncoder().encode(entry), to: folder.appendingPathComponent("entry.json"))
        try syncDirectory(folder)
    }
    /// Prepare and persist a stable response identity before handing it to transport.
    public func replyEnvelope(for id: UUID) throws -> VoiceEnvelope {
        guard var entry = try entries().first(where: { $0.id == id }), let reply = entry.reply else { throw VoiceError.missing }
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
        guard var entry = try entries().first(where: { $0.id == id }) else { throw VoiceError.missing }
        if let previous = entry.reply {
            guard previous == text, entry.transcript == envelope.transcript else { throw VoiceError.conflict }
        }
        if let receipt = entry.replyReceipt {
            guard receipt.messageId == envelope.messageId else { throw VoiceError.conflict }
            return (receipt, false)
        }
        let isNew = entry.reply == nil
        let receipt = Receipt(sessionId: id, messageId: envelope.messageId, receiptId: UUID())
        entry.reply = text; entry.transcript = envelope.transcript
        entry.replyMessageId = envelope.messageId; entry.replyReceipt = receipt; entry.state = .answered
        try save(entry)
        return (receipt, isNew)
    }
    public func acknowledgeReply(_ receipt: Receipt) throws {
        guard var entry = try entries().first(where: { $0.id == receipt.sessionId }),
              entry.replyMessageId == receipt.messageId else { throw VoiceError.invalid }
        entry.replyAcknowledged = true
        try save(entry)
    }
    private func save(_ entry: Entry) throws {
        let folder = root.appendingPathComponent(entry.id.uuidString)
        try write(JSONEncoder().encode(entry), to: folder.appendingPathComponent("entry.json"))
        try syncDirectory(folder)
    }
    private func write(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: .atomic)
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.synchronize()
    }
    private func syncDirectory(_ url: URL) throws {
        let descriptor = open(url.path, O_RDONLY)
        guard descriptor >= 0 else { throw VoiceError.persistence }
        defer { close(descriptor) }
        guard fsync(descriptor) == 0 else { throw VoiceError.persistence }
    }
}
