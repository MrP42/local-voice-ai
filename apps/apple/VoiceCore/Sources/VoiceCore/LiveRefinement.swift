import Foundation

public struct LiveRefinementRange: Codable, Equatable, Sendable {
    public let start: Double
    public let end: Double
    public init(start: Double, end: Double) { self.start = start; self.end = end }
}

/// Vorbelegter Ring. Synchronisierung gehört dem Besitzer; der Tap kopiert nur.
public struct LiveAudioRing {
    public struct Packet { public let startFrame: Int64; public let samples: [Float]; public let boundaryBefore: Bool }
    private struct Entry { var start: Int64 = 0; var count = 0; var boundaryBefore = false }
    private var storage: [Float]
    private var entries = [Entry](repeating: Entry(), count: 256)
    private var readIndex = 0
    private var writeIndex = 0
    private var used = 0
    private var first = 0
    private var length = 0
    private var missed: LiveRefinementRange?
    private var needsBoundary = false
    public init(capacity: Int = 16000 * 30) { storage = [Float](repeating: 0, count: max(1, capacity)) }

    public mutating func append(startFrame: Int64, samples: UnsafeBufferPointer<Float>) {
        guard samples.count <= storage.count - used, length < entries.count else {
            let start = Double(startFrame) / 16000, end = Double(startFrame + Int64(samples.count)) / 16000
            missed = LiveRefinementRange(start: min(missed?.start ?? start, start), end: max(missed?.end ?? end, end))
            needsBoundary = true
            return
        }
        for sample in samples { storage[writeIndex] = sample; writeIndex = (writeIndex + 1) % storage.count }
        entries[(first + length) % entries.count] = Entry(start: startFrame, count: samples.count, boundaryBefore: needsBoundary)
        needsBoundary = false
        used += samples.count; length += 1
    }

    public mutating func pop() -> Packet? {
        guard length > 0 else { return nil }
        let entry = entries[first]
        var samples = [Float](repeating: 0, count: entry.count)
        for i in samples.indices { samples[i] = storage[readIndex]; readIndex = (readIndex + 1) % storage.count }
        first = (first + 1) % entries.count; length -= 1; used -= entry.count
        return Packet(startFrame: entry.start, samples: samples, boundaryBefore: entry.boundaryBefore)
    }

    public mutating func takeMissed() -> LiveRefinementRange? {
        defer { missed = nil }
        return missed
    }
}

public enum LiveRefinement {
    public static func preferredSegments(_ segments: [LiveSegment]) -> [LiveSegment] {
        let refined = segments.filter { $0.isFinal && $0.refined }
        return segments.filter { s in
            s.isFinal && (s.refined || !refined.contains { $0.start < s.end && $0.end > s.start })
        }.sorted { $0.start == $1.start ? $0.id < $1.id : $0.start < $1.start }
    }

    /// Der Tokenizer wird vom geladenen Whisper-Modell gestellt; nie schätzen.
    public static func prompt(names: [String], emails: [String], glossary: String, count: (String) -> Int) -> String {
        let freemail: Set<String> = ["gmail", "googlemail", "outlook", "hotmail", "live", "msn", "yahoo", "ymail", "icloud", "me", "mac", "aol", "gmx", "web", "mail", "proton", "protonmail", "pm", "t-online", "freenet", "posteo", "mailbox", "fastmail", "hey", "zoho", "yandex", "qq", "163"]
        let companies = emails.compactMap { email -> String? in
            let parts = email.split(separator: "@", omittingEmptySubsequences: false)
            guard parts.count == 2 else { return nil }
            let domain = parts[1].lowercased().split(separator: ".").map(String.init)
            guard domain.count >= 2, !domain.dropLast().contains(where: { freemail.contains($0) }) else { return nil }
            // Bei mehrteiligen Länder-TLDs den Firmennamen vor co.uk usw. nehmen.
            let index = domain.count >= 3 && ["co", "com", "org", "net"].contains(domain[domain.count - 2]) ? domain.count - 3 : domain.count - 2
            return domain[index]
        }
        let entries = names + companies + glossary.components(separatedBy: CharacterSet.newlines.union(CharacterSet(charactersIn: ",;")))
        var seen = Set<String>(), result = ""
        for entry in entries {
            let clean = entry.components(separatedBy: .controlCharacters).joined(separator: " ").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !clean.isEmpty, seen.insert(clean.lowercased()).inserted else { continue }
            let candidate = result.isEmpty ? clean : result + ", " + clean
            let size = count(candidate)
            if size >= 0 && size <= 200 { result = candidate }
        }
        return result
    }
}

extension LiveState {
    /// Ersetzt nur den abgeschlossenen Audiobereich. Unverfeinerte Nachbarn bleiben.
    /// Ein Beleg kann sich auf mehrere neue Segmente verteilen; keine erfundene Textzuordnung.
    public mutating func replaceRefined(segments: inout [LiveSegment], from: Double, to: Double, lines: [LiveSegment]) {
        guard from.isFinite, to.isFinite, to > from, !lines.isEmpty else { return }
        let removed = Set(segments.filter { !$0.refined && $0.start < to && $0.end > from }.map(\.id))
        segments.removeAll { removed.contains($0.id) }
        segments += lines
        segments = LiveRefinement.preferredSegments(segments)
        transcriptVersion += 1
        let version = transcriptVersion
        func remap(_ evidence: [LiveEvidence]) -> [LiveEvidence] {
            var out: [LiveEvidence] = []
            for old in evidence {
                let affected = removed.contains(LiveSegment.baseId(old.segmentId))
                let targets = segments.filter { s in
                    (affected ? s.refined : s.id == LiveSegment.baseId(old.segmentId)) && s.start < old.end && s.end > old.start
                }
                if targets.isEmpty {
                    // Unaufgelöste Belege behalten ihren alten Versionsnachweis.
                    out.append(old)
                } else {
                    out += targets.map { LiveEvidence(transcriptVersion: version, segmentId: $0.id,
                                                      start: max(old.start, $0.start), end: min(old.end, $0.end)) }
                }
            }
            return Array(Set(out)).sorted { $0.start == $1.start ? $0.segmentId < $1.segmentId : $0.start < $1.start }
        }
        for i in items.indices {
            items[i].evidence = remap(items[i].evidence)
            items[i].answerEvidence = remap(items[i].answerEvidence)
        }
        // Der nächste Takt sieht die korrigierte Stelle erneut.
        processedThrough = min(processedThrough, from)
    }
}
