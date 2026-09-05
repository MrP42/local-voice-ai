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
    public func accept(_ packet: Packet) throws -> Receipt {
        guard packet.schemaVersion == 1 else { throw VoiceError.version }
        guard packet.kind == "capture", !packet.audio.isEmpty, packet.audio.count <= 1024 * 1024 else { throw VoiceError.invalid }
        let digest = SHA256.hash(data: packet.audio).map { String(format: "%02x", $0) }.joined()
        let current = try entries()
        if let existing = current.first(where: { $0.id == packet.sessionId || $0.receipt.messageId == packet.messageId }) {
            guard existing.id == packet.sessionId, existing.receipt.messageId == packet.messageId, existing.digest == digest else { throw VoiceError.conflict }
            guard try audio(for: existing.id) == packet.audio else { throw VoiceError.persistence }
            return existing.receipt
        }
        let used = try current.reduce(0) { try $0 + audio(for: $1.id).count }
        guard used + packet.audio.count <= limit else { throw VoiceError.full }
        let receipt = Receipt(sessionId: packet.sessionId, messageId: packet.messageId, receiptId: UUID())
        let entry = Entry(receipt: receipt, createdAt: packet.createdAt, digest: digest, state: .saved)
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
    public func update(_ id: UUID, transcript: String? = nil, reply: String? = nil, state: CaptureState, timing: (String, Double)? = nil) throws {
        guard var entry = try entries().first(where: { $0.id == id }) else { throw VoiceError.missing }
        if let transcript { entry.transcript = transcript }
        if let reply {
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
        var envelope = VoiceEnvelope(sessionId: id, transcript: entry.transcript, reply: reply)
        envelope.messageId = entry.replyMessageId!
        return envelope
    }
    public func acceptReply(_ envelope: VoiceEnvelope) throws -> (receipt: Receipt, isNew: Bool) {
        guard envelope.schemaVersion == 1 else { throw VoiceError.version }
        guard envelope.kind == "reply", let id = envelope.sessionId, let text = envelope.reply,
              !text.isEmpty, text.count <= 500, (envelope.transcript?.count ?? 0) <= 16000 else { throw VoiceError.invalid }
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
