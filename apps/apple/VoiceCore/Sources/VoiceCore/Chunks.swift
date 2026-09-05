import Foundation
import CryptoKit
import Darwin

public struct CaptureChunk: Codable, Sendable {
    public let schemaVersion: Int
    public let sessionId: UUID
    public let captureMessageId: UUID
    public let createdAt: Date
    public let digest: String
    public let byteCount: Int
    public let index: Int
    public let count: Int
    public var data: Data
    public var messageId: UUID {
        let bytes = Array(SHA256.hash(data: Data("\(captureMessageId.uuidString):\(index)".utf8)))
        return UUID(uuid: (bytes[0],bytes[1],bytes[2],bytes[3],bytes[4],bytes[5],bytes[6],bytes[7],bytes[8],bytes[9],bytes[10],bytes[11],bytes[12],bytes[13],bytes[14],bytes[15]))
    }
    public static func split(_ packet: Packet) throws -> [CaptureChunk] {
        guard packet.schemaVersion == 1, packet.kind == "capture", !packet.audio.isEmpty, packet.audio.count <= 1024 * 1024 else { throw VoiceError.invalid }
        let size = 32768
        let digest = SHA256.hash(data: packet.audio).map { String(format: "%02x", $0) }.joined()
        let count = (packet.audio.count + size - 1) / size
        return (0..<count).map { i in
            CaptureChunk(schemaVersion: 1, sessionId: packet.sessionId, captureMessageId: packet.messageId,
                         createdAt: packet.createdAt, digest: digest, byteCount: packet.audio.count,
                         index: i, count: count, data: packet.audio.subdata(in: i * size..<min((i + 1) * size, packet.audio.count)))
        }
    }
}

/// A partial receipt only confirms a persisted piece. The full capture receipt is
/// emitted by DurableStore after reassembly, hash verification and atomic commit.
public final class ChunkInbox {
    private let root: URL
    private let fm = FileManager.default
    public init(root: URL) throws {
        self.root = root
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
    }
    public func receive(_ part: CaptureChunk) throws -> Packet? {
        guard part.schemaVersion == 1 else { throw VoiceError.version }
        guard part.byteCount > 0, part.byteCount <= 1024 * 1024,
              part.count == (part.byteCount + 32767) / 32768,
              part.index >= 0, part.index < part.count,
              part.data.count == min(32768, part.byteCount - part.index * 32768) else { throw VoiceError.invalid }
        let directory = root.appendingPathComponent(part.sessionId.uuidString)
        try fm.createDirectory(at: directory, withIntermediateDirectories: true)
        let manifestURL = directory.appendingPathComponent("manifest.json")
        if fm.fileExists(atPath: manifestURL.path) {
            let previous = try JSONDecoder().decode(CaptureChunk.self, from: Data(contentsOf: manifestURL))
            guard previous.captureMessageId == part.captureMessageId, previous.createdAt == part.createdAt,
                  previous.digest == part.digest, previous.byteCount == part.byteCount, previous.count == part.count else { throw VoiceError.conflict }
        } else {
            var manifest = part; manifest.data = Data()
            try write(JSONEncoder().encode(manifest), to: manifestURL)
        }
        let url = directory.appendingPathComponent("\(part.index).part")
        if fm.fileExists(atPath: url.path) {
            guard try Data(contentsOf: url) == part.data else { throw VoiceError.conflict }
        } else {
            var used = 0
            if let files = fm.enumerator(at: root, includingPropertiesForKeys: [.fileSizeKey]) {
                for case let file as URL in files { used += try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0 }
            }
            guard used + part.data.count <= 64 * 1024 * 1024 else { throw VoiceError.full }
            try write(part.data, to: url)
        }
        try sync(directory); try sync(root)
        var audio = Data()
        for i in 0..<part.count {
            let file = directory.appendingPathComponent("\(i).part")
            guard fm.fileExists(atPath: file.path) else { return nil }
            audio.append(try Data(contentsOf: file))
        }
        guard audio.count == part.byteCount,
              SHA256.hash(data: audio).map({ String(format: "%02x", $0) }).joined() == part.digest else { throw VoiceError.conflict }
        return Packet(schemaVersion: 1, sessionId: part.sessionId, messageId: part.captureMessageId, kind: "capture", createdAt: part.createdAt, payload: Packet.Payload(audio: audio))
    }
    public func removeCompleted(_ sessionId: UUID) throws {
        let directory = root.appendingPathComponent(sessionId.uuidString)
        if fm.fileExists(atPath: directory.path) { try fm.removeItem(at: directory); try sync(root) }
    }
    private func write(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: .atomic)
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.synchronize()
    }
    private func sync(_ directory: URL) throws {
        let fd = open(directory.path, O_RDONLY)
        guard fd >= 0 else { throw VoiceError.persistence }
        defer { close(fd) }
        guard fsync(fd) == 0 else { throw VoiceError.persistence }
    }
}
