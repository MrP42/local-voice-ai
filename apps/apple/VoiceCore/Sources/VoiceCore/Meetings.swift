import Foundation
import CryptoKit
import Darwin

public struct MeetingSegment: Codable, Equatable, Sendable {
    public let index: Int
    public let start: Double
    public let end: Double
    public var text: String
    public var speaker: String?
    public init(index: Int, start: Double, end: Double, text: String, speaker: String? = nil) {
        self.index = index; self.start = start; self.end = end; self.text = text; self.speaker = speaker
    }
}

/// Same content schema as Desktop minutes. Names, owners and deadlines may be unknown.
public struct MeetingMinutes: Codable, Equatable, Sendable {
    public struct Decision: Codable, Equatable, Sendable { public var text: String; public var context: String }
    public struct Task: Codable, Equatable, Sendable {
        public var text: String; public var assignee: String?; public var due: String?
        enum CodingKeys: String, CodingKey { case text, assignee, due }
        public func encode(to encoder: Encoder) throws {
            var box = encoder.container(keyedBy: CodingKeys.self)
            try box.encode(text, forKey: .text); try box.encode(assignee, forKey: .assignee); try box.encode(due, forKey: .due)
        }
    }
    public struct Step: Codable, Equatable, Sendable {
        public var text: String; public var owner: String?
        enum CodingKeys: String, CodingKey { case text, owner }
        public func encode(to encoder: Encoder) throws {
            var box = encoder.container(keyedBy: CodingKeys.self)
            try box.encode(text, forKey: .text); try box.encode(owner, forKey: .owner)
        }
    }
    public struct Recommendation: Codable, Equatable, Sendable { public var text: String; public var reason: String }
    public struct Question: Codable, Equatable, Sendable { public var text: String }
    public var summary: String
    public var scope: String
    public var decisions: [Decision]
    public var tasks: [Task]
    public var next_steps: [Step]
    public var follow_ups: [Recommendation]
    public var open_questions: [Question]
    public init(summary: String, scope: String, decisions: [Decision], tasks: [Task], next_steps: [Step], follow_ups: [Recommendation], open_questions: [Question]) {
        self.summary = summary; self.scope = scope; self.decisions = decisions; self.tasks = tasks
        self.next_steps = next_steps; self.follow_ups = follow_ups; self.open_questions = open_questions
    }
    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= 128 * 1024,
              let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys) == Set(["summary", "scope", "decisions", "tasks", "next_steps", "follow_ups", "open_questions"]) else { throw VoiceError.invalid }
        func bounded(_ value: Any) -> Bool {
            if let text = value as? String { return text.count <= 8192 }
            if let array = value as? [Any] { return array.count <= 100 && array.allSatisfy(bounded) }
            if let dict = value as? [String: Any] { return dict.values.allSatisfy(bounded) }
            return value is NSNull
        }
        guard bounded(object) else { throw VoiceError.invalid }
        let fields: [String: Set<String>] = ["decisions": ["text", "context"], "tasks": ["text", "assignee", "due"], "next_steps": ["text", "owner"], "follow_ups": ["text", "reason"], "open_questions": ["text"]]
        for (key, keys) in fields {
            guard let items = object[key] as? [[String: Any]], items.allSatisfy({ Set($0.keys) == keys }) else { throw VoiceError.invalid }
        }
        return try JSONDecoder().decode(Self.self, from: data)
    }
}

public struct MeetingDocument: Codable, Identifiable, Sendable {
    public let id: UUID
    public let createdAt: Date
    public let originalName: String
    public let originalExtension: String
    public let digest: String
    public let bytes: Int
    public let duration: Double
    public var nextOffset: Double = 0
    public var segments: [MeetingSegment] = []
    public var minutes: MeetingMinutes?
}

/// Separate from short Watch captures. Commit the original before acknowledging import.
/// One archive actor owns a root; chunks advance using compare-and-swap offsets.
public actor MeetingArchive {
    private let root: URL
    private let quota: Int
    private let fm = FileManager.default
    public init(root: URL, quota: Int = 4 * 1024 * 1024 * 1024) throws {
        guard quota > 0 else { throw VoiceError.invalid }
        self.root = root; self.quota = quota
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }
    public func list() throws -> [MeetingDocument] {
        try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .compactMap { UUID(uuidString: $0.lastPathComponent) }
            .map { try load($0) }.sorted { $0.createdAt > $1.createdAt }
    }
    public func load(_ id: UUID) throws -> MeetingDocument {
        let data = try Data(contentsOf: folder(id).appendingPathComponent("meeting.json"))
        guard data.count <= 32 * 1024 * 1024 else { throw VoiceError.invalid }
        let document = try JSONDecoder().decode(MeetingDocument.self, from: data)
        guard document.id == id else { throw VoiceError.conflict }
        return document
    }
    public func originalURL(_ id: UUID) throws -> URL {
        let doc = try load(id)
        guard doc.originalExtension.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }), doc.originalExtension.count <= 10 else { throw VoiceError.invalid }
        let url = folder(id).appendingPathComponent("original." + doc.originalExtension)
        guard try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey]).isRegularFile == true,
              try url.resourceValues(forKeys: [.isSymbolicLinkKey]).isSymbolicLink != true else { throw VoiceError.persistence }
        return url
    }
    public func importMedia(_ source: URL, duration: Double) throws -> MeetingDocument {
        guard duration.isFinite, duration > 0, duration <= 7200 else { throw VoiceError.invalid }
        let info = try source.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard info.isRegularFile == true, info.isSymbolicLink != true, let bytes = info.fileSize, bytes > 0, bytes <= 2 * 1024 * 1024 * 1024 else { throw VoiceError.invalid }
        // Count actual files, including interrupted imports and damaged metadata.
        var used = 0
        if let walk = fm.enumerator(at: root, includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey]) {
            for case let file as URL in walk where file.lastPathComponent.hasPrefix("original.") {
                used += try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            }
        }
        guard bytes <= quota - used else { throw VoiceError.full }
        let ext = source.pathExtension.lowercased()
        guard !ext.isEmpty, ext.count <= 10, ext.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { throw VoiceError.invalid }
        let id = UUID(), staging = root.appendingPathComponent(".import-" + UUID().uuidString)
        try fm.createDirectory(at: staging, withIntermediateDirectories: false)
        defer { try? fm.removeItem(at: staging) }
        let target = staging.appendingPathComponent("original." + ext)
        try fm.copyItem(at: source, to: target)
        let reader = try FileHandle(forReadingFrom: target)
        defer { try? reader.close() }
        var hash = SHA256(), copied = 0
        while let part = try reader.read(upToCount: 1024 * 1024), !part.isEmpty {
            copied += part.count; guard copied <= bytes else { throw VoiceError.conflict }
            hash.update(data: part)
        }
        guard copied == bytes else { throw VoiceError.conflict }
        try syncFile(target)
        let doc = MeetingDocument(id: id, createdAt: Date(), originalName: String(source.lastPathComponent.prefix(255)), originalExtension: ext,
            digest: hash.finalize().map { String(format: "%02x", $0) }.joined(), bytes: bytes, duration: duration)
        try write(try JSONEncoder().encode(doc), at: staging.appendingPathComponent("meeting.json"))
        try syncDirectory(staging)
        try fm.moveItem(at: staging, to: folder(id))
        try syncDirectory(root)
        return doc
    }
    public func appendChunk(_ id: UUID, offset: Double, nextOffset: Double, segments: [MeetingSegment]) throws {
        var doc = try load(id)
        guard offset == doc.nextOffset else { throw VoiceError.conflict }
        guard offset.isFinite, nextOffset.isFinite, nextOffset > offset, nextOffset <= doc.duration,
              segments.count <= 1000, segments.allSatisfy({ $0.start.isFinite && $0.end.isFinite && $0.start >= offset && $0.end >= $0.start && $0.end <= nextOffset && $0.text.count <= 16384 }) else { throw VoiceError.invalid }
        for segment in segments {
            doc.segments.append(MeetingSegment(index: doc.segments.count, start: segment.start, end: segment.end, text: segment.text, speaker: segment.speaker))
        }
        doc.nextOffset = nextOffset
        try save(doc)
    }
    public func saveMinutes(_ id: UUID, minutes: MeetingMinutes) throws {
        var doc = try load(id)
        guard doc.nextOffset == doc.duration else { throw VoiceError.conflict }
        doc.minutes = try MeetingMinutes.decode(JSONEncoder().encode(minutes))
        try save(doc)
    }
    private func folder(_ id: UUID) -> URL { root.appendingPathComponent(id.uuidString) }
    private func save(_ doc: MeetingDocument) throws { try write(JSONEncoder().encode(doc), at: folder(doc.id).appendingPathComponent("meeting.json")) }
    private func write(_ data: Data, at url: URL) throws {
        guard data.count <= 32 * 1024 * 1024 else { throw VoiceError.full }
        try data.write(to: url, options: .atomic)
        try syncFile(url); try syncDirectory(url.deletingLastPathComponent())
    }
    private func syncFile(_ url: URL) throws {
        let handle = try FileHandle(forWritingTo: url); defer { try? handle.close() }; try handle.synchronize()
    }
    private func syncDirectory(_ url: URL) throws {
        let fd = Darwin.open(url.path, O_RDONLY)
        guard fd >= 0 else { throw VoiceError.persistence }
        defer { Darwin.close(fd) }
        guard fsync(fd) == 0 else { throw VoiceError.persistence }
    }
}

public enum MeetingExport {
    public struct Share: Sendable { public let name: String; public let seconds: Double; public let percent: Double }
    public static func speakerShares(_ segments: [MeetingSegment]) -> [Share] {
        guard segments.contains(where: { !($0.speaker?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) }) else { return [] }
        let groups = Dictionary(grouping: segments.filter { $0.start.isFinite && $0.end.isFinite && $0.start >= 0 && $0.end > $0.start }) { segment in
            let name = segment.speaker?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return name.isEmpty ? "Nicht zugeordnet" : name
        }
        let times = groups.mapValues { items -> Double in
            var end = 0.0, seconds = 0.0
            for item in items.sorted(by: { $0.start < $1.start }) {
                seconds += max(0, item.end - max(item.start, end)); end = max(end, item.end)
            }
            return seconds
        }
        let total = times.values.reduce(0, +)
        guard total > 0 else { return [] }
        return times.keys.sorted().map { Share(name: $0, seconds: times[$0]!, percent: times[$0]! / total * 100) }
    }
    public static func srt(_ segments: [MeetingSegment]) -> String {
        func stamp(_ seconds: Double) -> String {
            let ms = Int((max(0, min(seconds.isFinite ? seconds : 0, 86400)) * 1000).rounded())
            return String(format: "%02d:%02d:%02d,%03d", ms / 3600000, ms / 60000 % 60, ms / 1000 % 60, ms % 1000)
        }
        return segments.enumerated().map { index, item in "\(index + 1)\n\(stamp(item.start)) --> \(stamp(item.end))\n\(item.text)\n" }.joined(separator: "\n")
    }
    public static func html(_ text: String) -> String {
        "<pre>" + text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;") + "</pre>"
    }
}
