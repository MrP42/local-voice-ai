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
    /// Factual fields are extractive: unsupported names/dates cannot silently become minutes.
    /// Recommendations remain proposals, with an exact source quote as their reason.
    public func validateEvidence(in source: String) throws {
        func normalized(_ value: String) -> String {
            value.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ").lowercased()
        }
        let evidence = normalized(source)
        func quoted(_ value: String) -> Bool { normalized(value).isEmpty || evidence.contains(normalized(value)) }
        func attributed(_ name: String?, text: String) -> Bool { name.map { !normalized($0).isEmpty && Self.containsAttribution($0, in: text) } ?? true }
        guard (evidence.isEmpty || !normalized(summary).isEmpty), summary.split(whereSeparator: { $0.isNewline }).allSatisfy({ quoted(String($0)) }), quoted(scope),
              decisions.allSatisfy({ quoted($0.text) && quoted($0.context) }),
              tasks.allSatisfy({ quoted($0.text) && attributed($0.assignee, text: $0.text) && attributed($0.due, text: $0.text) }),
              next_steps.allSatisfy({ quoted($0.text) && attributed($0.owner, text: $0.text) }),
              open_questions.allSatisfy({ quoted($0.text) }),
              follow_ups.allSatisfy({ !$0.reason.isEmpty && quoted($0.reason) }) else { throw VoiceError.invalid }
    }
    private static func containsAttribution(_ value: String, in text: String) -> Bool {
        let pattern = "(?<![\\p{L}\\p{N}_])" + NSRegularExpression.escapedPattern(for: value) + "(?![\\p{L}\\p{N}_])"
        return text.range(of: pattern, options: [.regularExpression, .caseInsensitive]) != nil
    }
    public static func fromSelection(_ data: Data, source: [String]) throws -> Self {
        struct SelectedTask: Decodable { let index: Int; let assignee: String?; let due: String? }
        struct SelectedStep: Decodable { let index: Int; let owner: String? }
        struct SelectedRecommendation: Decodable { let index: Int; let text: String }
        struct Selection: Decodable {
            let summary: [Int]; let decisions: [Int]; let tasks: [SelectedTask]; let next_steps: [SelectedStep]; let follow_ups: [SelectedRecommendation]; let open_questions: [Int]
        }
        guard data.count <= 32768, source.count <= 1000 else { throw VoiceError.invalid }
        let selected = try JSONDecoder().decode(Selection.self, from: data)
        guard !selected.summary.isEmpty, selected.summary.count <= 2 else { throw VoiceError.invalid }
        func quote(_ index: Int) throws -> String {
            guard source.indices.contains(index) else { throw VoiceError.invalid }
            return source[index].trimmingCharacters(in: .whitespacesAndNewlines)
        }
        func attribution(_ value: String?, in text: String) -> String? {
            guard let value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  Self.containsAttribution(value, in: text) else { return nil }
            return value
        }
        let summary = try Array(Set(selected.summary)).sorted().map(quote).joined(separator: "\n")
        var report = Self(summary: summary, scope: try quote(selected.summary[0]),
            decisions: try Array(Set(selected.decisions)).sorted().map { Decision(text: try quote($0), context: "") },
            tasks: try selected.tasks.map { task in
                let text = try quote(task.index)
                return Task(text: text, assignee: attribution(task.assignee, in: text), due: attribution(task.due, in: text))
            },
            next_steps: try selected.next_steps.map { step in
                let text = try quote(step.index)
                return Step(text: text, owner: attribution(step.owner, in: text))
            },
            follow_ups: try selected.follow_ups.map {
                guard !$0.text.isEmpty, $0.text.count <= 800 else { throw VoiceError.invalid }
                return Recommendation(text: $0.text, reason: try quote($0.index))
            },
            open_questions: try Array(Set(selected.open_questions)).sorted().map { Question(text: try quote($0)) })
        // Conservative repair of category mistakes seen in the German device fixture.
        func unresolved(_ text: String) -> Bool {
            text.contains("?") || text.range(of: "\\b(offen|unklar|ungeklärt|unbekannt)\\b", options: [.regularExpression, .caseInsensitive]) != nil
        }
        func collectiveFuture(_ text: String) -> Bool {
            let value = text.lowercased()
            return value.range(of: "\\b(wir|gemeinsam)\\b", options: .regularExpression) != nil &&
                (value.contains("nächste") || value.contains("morgen") || value.contains("werden") || value.range(of: "\\b(montag|dienstag|mittwoch|donnerstag|freitag|samstag|sonntag)\\b", options: .regularExpression) != nil)
        }
        report.open_questions = report.open_questions.filter { unresolved($0.text) }
        var tasks: [Task] = []
        for task in report.tasks {
            if report.decisions.contains(where: { $0.text == task.text }) { continue }
            if unresolved(task.text) {
                if !report.open_questions.contains(where: { $0.text == task.text }) { report.open_questions.append(Question(text: task.text)) }
            } else if task.assignee == nil && collectiveFuture(task.text) {
                if !report.next_steps.contains(where: { $0.text == task.text }) { report.next_steps.append(Step(text: task.text, owner: nil)) }
            } else { tasks.append(task) }
        }
        report.tasks = tasks
        let priorities = report.decisions.map(\.text) + report.tasks.map(\.text)
        if !priorities.isEmpty && !selected.summary.contains(where: { index in priorities.contains((try? quote(index)) ?? "") }) {
            report.summary = Array(priorities.prefix(2)).joined(separator: "\n")
        }
        if let topic = source.first(where: { $0.localizedCaseInsensitiveContains("besprechen") || $0.localizedCaseInsensitiveContains("geht es um") }) { report.scope = topic }
        try report.validateEvidence(in: source.joined(separator: "\n"))
        return report
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
    public var summaryParts: [MeetingMinutes]?
    public var summarizedSegments: Int?
    public var timings: [String: Double]?
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
    public func appendMinutes(_ id: UUID, from: Int, through: Int, minutes: MeetingMinutes) throws {
        var doc = try load(id)
        guard doc.nextOffset == doc.duration, from == (doc.summarizedSegments ?? 0), through > from, through <= doc.segments.count else { throw VoiceError.conflict }
        let validated = try MeetingMinutes.decode(JSONEncoder().encode(minutes))
        try validated.validateEvidence(in: doc.segments[from..<through].map(\.text).joined(separator: "\n"))
        doc.summaryParts = (doc.summaryParts ?? []) + [validated]
        doc.summarizedSegments = through
        try save(doc)
    }
    public func recordTiming(_ id: UUID, phase: String, milliseconds: Double) throws {
        guard ["import", "audio", "stt", "minutes"].contains(phase), milliseconds.isFinite, milliseconds >= 0 else { throw VoiceError.invalid }
        var doc = try load(id)
        var timings = doc.timings ?? [:]; timings[phase, default: 0] += milliseconds; doc.timings = timings
        try save(doc)
    }
    public func finishMinutes(_ id: UUID) throws {
        var doc = try load(id)
        guard doc.nextOffset == doc.duration, (doc.summarizedSegments ?? 0) == doc.segments.count else { throw VoiceError.conflict }
        let parts = doc.summaryParts ?? []
        func unique<T: Encodable>(_ values: [T]) throws -> [T] {
            var seen = Set<Data>()
            let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
            return try values.filter { seen.insert(try encoder.encode($0)).inserted }
        }
        doc.minutes = MeetingMinutes(summary: parts.map(\.summary).joined(separator: "\n\n"), scope: parts.map(\.scope).joined(separator: "\n"),
            decisions: try unique(parts.flatMap(\.decisions)), tasks: try unique(parts.flatMap(\.tasks)), next_steps: try unique(parts.flatMap(\.next_steps)),
            follow_ups: try unique(parts.flatMap(\.follow_ups)), open_questions: try unique(parts.flatMap(\.open_questions)))
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
    public static func text(_ doc: MeetingDocument) -> String {
        var lines = [doc.originalName, "", "Transkript", ""]
        lines += doc.segments.map { item in
            let speaker = item.speaker.map { "[" + $0 + "] " } ?? ""
            return speaker + item.text.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        if let report = doc.minutes {
            lines += ["", "Zusammenfassung", report.summary, "", "Kontext", report.scope, "", "Entscheidungen"]
            lines += report.decisions.map { "• " + $0.text + " — " + $0.context }
            lines += ["", "Aufgaben"]
            lines += report.tasks.map { "• " + $0.text + " | Zuständig: " + ($0.assignee ?? "offen") + " | Termin: " + ($0.due ?? "offen") }
            lines += ["", "Nächste Schritte"]
            lines += report.next_steps.map { "• " + $0.text + " | " + ($0.owner ?? "offen") }
            lines += ["", "Handlungsempfehlungen"]
            lines += report.follow_ups.map { "• " + $0.text + " — " + $0.reason }
            lines += ["", "Offene Fragen"]
            lines += report.open_questions.map { "• " + $0.text }
        }
        let shares = speakerShares(doc.segments)
        lines += ["", "Redeanteile"]
        lines += shares.isEmpty ? ["Keine belegte Sprecherzuordnung vorhanden."] : shares.map { String(format: "%@: %.1f s (%.1f %%)", $0.name, $0.seconds, $0.percent) }
        return lines.joined(separator: "\n")
    }
    public static func html(_ text: String) -> String {
        "<pre>" + text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;") + "</pre>"
    }
}
