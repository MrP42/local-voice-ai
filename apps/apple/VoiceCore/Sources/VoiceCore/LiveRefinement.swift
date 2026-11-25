import Foundation
#if canImport(os)
import os
#endif

public struct LiveRefinementRange: Codable, Equatable, Sendable {
    public let start: Double
    public let end: Double
    public init(start: Double, end: Double) { self.start = start; self.end = end }
}

/// Vorbelegter SPSC-Ring. Ohne Swift-Atomics: Trylock und Verwerfen im Produzenten.
/// Nur der Verbraucher darf warten/allokieren; der Aufnahme-Lock wird nie genommen.
public final class LiveAudioRing: @unchecked Sendable {
    public struct Packet: Sendable { public let startFrame: Int64; public let samples: [Float]; public let boundaryBefore: Bool }
    #if canImport(os)
    private let gate: UnsafeMutablePointer<os_unfair_lock_s>
    #else
    private let gate = NSLock()
    #endif
    private struct Entry { var start: Int64 = 0; var count = 0; var boundaryBefore = false }
    private var storage: [Float]
    private var entries = [Entry](repeating: Entry(), count: 256)
    private var readIndex = 0
    private var writeIndex = 0
    private var used = 0
    private var first = 0
    private var length = 0
    private var missed: LiveRefinementRange?
    // Nur der serielle Produzent; Veröffentlichung unter gate beim nächsten Erfolg/Rückbau.
    private var producerMissed: LiveRefinementRange?
    private var needsBoundary = false
    public init(capacity: Int = 16000 * 30) {
        storage = [Float](repeating: 0, count: max(1, capacity))
        #if canImport(os)
        gate = .allocate(capacity: 1)
        gate.initialize(to: os_unfair_lock_s())
        #endif
    }

    deinit {
        #if canImport(os)
        gate.deinitialize(count: 1); gate.deallocate()
        #endif
    }

    private func tryLock() -> Bool {
        #if canImport(os)
        return os_unfair_lock_trylock(gate)
        #else
        return gate.try()
        #endif
    }

    private func lock() {
        #if canImport(os)
        os_unfair_lock_lock(gate)
        #else
        gate.lock()
        #endif
    }

    private func unlock() {
        #if canImport(os)
        os_unfair_lock_unlock(gate)
        #else
        gate.unlock()
        #endif
    }

    private func discard(startFrame: Int64, count: Int) {
        let start = Double(startFrame) / 16000, end = Double(startFrame + Int64(count)) / 16000
        producerMissed = LiveRefinementRange(start: min(producerMissed?.start ?? start, start),
                                            end: max(producerMissed?.end ?? end, end))
    }

    private func publishMissedLocked() {
        guard let gap = producerMissed else { return }
        missed = LiveRefinementRange(start: min(missed?.start ?? gap.start, gap.start), end: max(missed?.end ?? gap.end, gap.end))
        producerMissed = nil
        needsBoundary = true
    }

    /// Nur nach Stilllegen des Produzenten aufrufen, auch wenn der letzte Tap verworfen wurde.
    public func publishMissed() {
        lock(); defer { unlock() }
        publishMissedLocked()
    }

    public func append(startFrame: Int64, samples: UnsafeBufferPointer<Float>) {
        guard tryLock() else { discard(startFrame: startFrame, count: samples.count); return }
        defer { unlock() }
        publishMissedLocked()
        guard samples.count <= storage.count - used, length < entries.count else {
            discard(startFrame: startFrame, count: samples.count)
            publishMissedLocked()
            return
        }
        for sample in samples { storage[writeIndex] = sample; writeIndex = (writeIndex + 1) % storage.count }
        entries[(first + length) % entries.count] = Entry(start: startFrame, count: samples.count, boundaryBefore: needsBoundary)
        needsBoundary = false
        used += samples.count; length += 1
    }

    public func pop() -> Packet? {
        withConsumerAccess {
            guard length > 0 else { return nil }
            let entry = entries[first]
            var samples = [Float](repeating: 0, count: entry.count)
            for i in samples.indices { samples[i] = storage[readIndex]; readIndex = (readIndex + 1) % storage.count }
            first = (first + 1) % entries.count; length -= 1; used -= entry.count
            return Packet(startFrame: entry.start, samples: samples, boundaryBefore: entry.boundaryBefore)
        }
    }

    public func takeMissed() -> LiveRefinementRange? {
        withConsumerAccess {
            defer { missed = nil }
            return missed
        }
    }

    /// Synchrone Verbraucherarbeit; der Produzent verwirft währenddessen ohne Wartezeit.
    func withConsumerAccess<T>(_ work: () -> T) -> T {
        lock(); defer { unlock() }
        return work()
    }
}

public enum LiveRefinement {
    /// Eine Überlagerung darf keine Lücke oder nur teilweise abgedeckte Rede verbergen.
    public static func fullyCovered(start: Double, end: Double, by lines: [LiveSegment]) -> Bool {
        guard start.isFinite, end.isFinite, end > start else { return false }
        var through = start
        for line in lines.filter({ $0.isFinal && $0.refined }).sorted(by: { $0.start < $1.start }) {
            guard line.start.isFinite, line.end.isFinite, line.end > line.start else { continue }
            if line.start > through { break }
            through = max(through, line.end)
            if through >= end { return true }
        }
        return false
    }

    public static func excluding(_ lines: [LiveSegment], ranges: [LiveRefinementRange]) -> [LiveSegment] {
        lines.filter { line in !ranges.contains { $0.start < line.end && $0.end > line.start } }
    }

    public static func preferredSegments(_ segments: [LiveSegment]) -> [LiveSegment] {
        let refined = segments.filter { $0.isFinal && $0.refined }
        return segments.filter { s in
            s.isFinal && (s.refined || !fullyCovered(start: s.start, end: s.end, by: refined))
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
    /// Überlagert den abgeschlossenen Audiobereich; Originale bleiben als Beleg erhalten.
    /// Ein Beleg kann sich auf mehrere neue Segmente verteilen; keine erfundene Textzuordnung.
    public mutating func replaceRefined(segments: inout [LiveSegment], from: Double, to: Double, lines: [LiveSegment]) {
        guard from.isFinite, to.isFinite, to > from, !lines.isEmpty else { return }
        segments += lines
        segments.sort { $0.start == $1.start ? $0.id < $1.id : $0.start < $1.start }
        transcriptVersion += 1
        let version = transcriptVersion
        func remap(_ evidence: [LiveEvidence]) -> [LiveEvidence] {
            var out: [LiveEvidence] = []
            for old in evidence {
                let targets = segments.filter { s in
                    s.refined && s.start < old.end && s.end > old.start
                }
                if !LiveRefinement.fullyCovered(start: old.start, end: old.end, by: targets) {
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
