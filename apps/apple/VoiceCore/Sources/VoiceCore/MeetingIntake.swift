import Foundation
#if canImport(CryptoKit)
import CryptoKit
#endif

// Übernahme von Aufnahmen (Epic #247, P2): Eingang im App-Group-Ordner für
// Teilen-Menü und „Öffnen in“, dazu das Stückprotokoll der Watch-Aufnahme.
// Beide quittieren erst nach dauerhaftem Speichern und sind idempotent.

public protocol ContentDigest: Sendable {
    func hex(of url: URL) throws -> String
}

#if canImport(CryptoKit)
public struct SHA256Digest: ContentDigest {
    public init() {}
    public func hex(of url: URL) throws -> String {
        let h = try FileHandle(forReadingFrom: url); defer { try? h.close() }
        var hash = SHA256()
        while let part = try h.read(upToCount: 1 << 20), !part.isEmpty { hash.update(data: part) }
        return hash.finalize().map { String(format: "%02x", $0) }.joined()
    }
}
#endif

public enum IntakeError: Error, Equatable {
    case consentMissing, invalid, tooLarge, conflict(String), digestMismatch, unknownSession
}

enum DurableFiles {
    static func sync(_ url: URL) throws {
        let h = try FileHandle(forUpdating: url); defer { try? h.close() }
        try h.synchronize()
    }
    static func write(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: .atomic)
        try sync(url)
        try syncDirectory(url.deletingLastPathComponent())
    }
    /// Verzeichniseintrag dauerhaft machen (nach Anlegen, Umbenennen, Löschen).
    static func syncDirectory(_ url: URL) throws {
        let fd = open(url.path, O_RDONLY)
        guard fd >= 0 else { throw IntakeError.invalid }
        defer { close(fd) }
        guard fsync(fd) == 0 else { throw IntakeError.invalid }
    }
    static func safeExtension(_ url: URL) -> String? {
        let ext = url.pathExtension.lowercased()
        guard !ext.isEmpty, ext.count <= 10, ext.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { return nil }
        return ext
    }
}

// MARK: - Eingang (Teilen-Menü, „Öffnen in“)

public struct InboxItem: Codable, Equatable, Identifiable, Sendable {
    public enum Source: String, Codable, Sendable { case shareSheet, openIn, shortcut }
    public let id: UUID
    public let source: Source
    public let originalName: String
    public let originalExtension: String
    public let bytes: Int
    public let digest: String
    public let createdAt: Date
    /// Bestätigung „Aufnahme erfolgte mit Zustimmung aller Beteiligten“.
    public let consent: ConsentRecord
}

/// Die Share-Extension kopiert nur (Speichergrenze), die App holt ab und
/// transkribiert. Ein Eintrag verschwindet erst, wenn die Übernahme ins Archiv
/// erfolgreich war.
public struct MeetingInbox: Sendable {
    public let root: URL
    let digest: ContentDigest
    public static let maxBytes = 2 * 1024 * 1024 * 1024

    public init(root: URL, digest: ContentDigest) throws {
        self.root = root; self.digest = digest
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    public func deposit(_ source: URL, from kind: InboxItem.Source, consent: ConsentRecord) throws -> InboxItem {
        guard consent.isComplete, consent.reason == .import, consent.channel == .import else { throw IntakeError.consentMissing }
        let fm = FileManager.default
        let info = try source.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard info.isRegularFile == true, info.isSymbolicLink != true, let bytes = info.fileSize, bytes > 0 else { throw IntakeError.invalid }
        guard bytes <= Self.maxBytes else { throw IntakeError.tooLarge }
        guard let ext = DurableFiles.safeExtension(source) else { throw IntakeError.invalid }
        let hex = try digest.hex(of: source)
        // Zweimal geteilt: derselbe Eintrag, keine Doppelung.
        if let same = try pending().first(where: { $0.digest == hex && $0.bytes == bytes }) {
            // Auch die Wiederholung bestätigt erst nach gesichertem Verzeichniseintrag.
            try DurableFiles.syncDirectory(root)
            return same
        }
        let item = InboxItem(id: UUID(), source: kind, originalName: String(source.lastPathComponent.prefix(255)), originalExtension: ext,
                             bytes: bytes, digest: hex, createdAt: Date(), consent: consent)
        let staging = root.appendingPathComponent(".tmp-" + item.id.uuidString)
        try fm.createDirectory(at: staging, withIntermediateDirectories: false)
        defer { try? fm.removeItem(at: staging) }
        let target = staging.appendingPathComponent("original." + ext)
        try fm.copyItem(at: source, to: target)
        try DurableFiles.sync(target)
        guard try digest.hex(of: target) == hex else { throw IntakeError.digestMismatch }
        try DurableFiles.write(try JSONEncoder().encode(item), to: staging.appendingPathComponent("eingang.json"))
        try fm.moveItem(at: staging, to: root.appendingPathComponent(item.id.uuidString))
        try DurableFiles.syncDirectory(root)
        return item
    }

    /// Wartende Einträge, älteste zuerst. Abgebrochene Kopien (`.tmp-…`) zählen nicht.
    public func pending() throws -> [InboxItem] {
        try FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .filter { UUID(uuidString: $0.lastPathComponent) != nil }
            .compactMap { dir -> InboxItem? in
                guard let data = try? Data(contentsOf: dir.appendingPathComponent("eingang.json")),
                      let item = try? JSONDecoder().decode(InboxItem.self, from: data), item.id.uuidString == dir.lastPathComponent else { return nil }
                return item
            }
            .sorted { $0.createdAt < $1.createdAt }
    }

    public func fileURL(_ item: InboxItem) -> URL {
        root.appendingPathComponent(item.id.uuidString).appendingPathComponent("original." + item.originalExtension)
    }

    /// Übernimmt einen Eintrag. `importer` legt ihn dauerhaft ins Archiv; erst
    /// danach wird der Eingang geleert. Wirft `importer`, bleibt der Eintrag liegen.
    public func take<T>(_ item: InboxItem, _ importer: (URL, InboxItem) async throws -> T) async throws -> T {
        let result = try await importer(fileURL(item), item)
        try? FileManager.default.removeItem(at: root.appendingPathComponent(item.id.uuidString))
        return result
    }

    /// Reste abgebrochener Kopien entfernen (beim App-Start).
    public func sweepAbandoned() {
        let fm = FileManager.default
        for url in (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? [] where url.lastPathComponent.hasPrefix(".tmp-") {
            try? fm.removeItem(at: url)
        }
    }
}

// MARK: - Watch-Stückprotokoll

/// Kopf eines Aufnahmestücks der Uhr. Stücke werden lückenlos rotiert: das erste
/// Sample eines Stücks folgt unmittelbar auf das letzte des vorigen.
public struct WatchChunkHeader: Codable, Equatable, Sendable {
    public let sessionId: UUID
    public let sequence: Int
    public let firstSample: Int64
    public let sampleCount: Int64
    public let sampleRate: Int
    public let digest: String
    public let isLast: Bool
    /// Nur im ersten Stück: Einwilligung an der Uhr.
    public let consent: ConsentRecord?
    public init(sessionId: UUID, sequence: Int, firstSample: Int64, sampleCount: Int64, sampleRate: Int = 16_000,
                digest: String, isLast: Bool = false, consent: ConsentRecord? = nil) {
        self.sessionId = sessionId; self.sequence = sequence; self.firstSample = firstSample; self.sampleCount = sampleCount
        self.sampleRate = sampleRate; self.digest = digest; self.isLast = isLast; self.consent = consent
    }
    public var start: Double { Double(firstSample) / Double(sampleRate) }
    /// Letzte Sampleposition + 1, überlaufsicher; `nil` bei ungültigem Kopf.
    public var endSample: Int64? {
        guard firstSample >= 0, sampleCount > 0 else { return nil }
        let (sum, overflow) = firstSample.addingReportingOverflow(sampleCount)
        return overflow ? nil : sum
    }
    public var end: Double { endSample.map { Double($0) / Double(sampleRate) } ?? .infinity }
}

public struct WatchChunkReceipt: Codable, Equatable, Sendable {
    public let sessionId: UUID
    public let sequence: Int
    public let digest: String
}

public struct WatchSessionStatus: Equatable, Sendable {
    public var received: [Int]
    public var missingSequences: [Int]
    /// Sample-Lücken oder Überlappungen zwischen aufeinanderfolgenden Stücken (Sekunden).
    public var gaps: [ClosedRange<Double>]
    public var lastReceived: Bool
    /// Erstes Stück beginnt bei Sample 0, alle Stücke haben dieselbe Samplerate, genau ein
    /// Abschlussstück, und es ist das letzte.
    public var startsAtZero = false
    public var uniformRate = false
    public var singleFinalChunk = false
    public var complete: Bool {
        lastReceived && missingSequences.isEmpty && gaps.isEmpty && startsAtZero && uniformRate && singleFinalChunk
    }
}

/// iPhone-Seite: nimmt Stücke an, speichert sie dauerhaft und quittiert erst dann.
public struct WatchRecordingAssembler: Sendable {
    public let root: URL
    let digest: ContentDigest
    /// Verzeichnis-fsync; im Test austauschbar, um Fehler und Reihenfolge zu prüfen.
    var syncDirectory: @Sendable (URL) throws -> Void = DurableFiles.syncDirectory
    public static let maxChunkBytes = 8 * 1024 * 1024
    public static let maxSeconds: Double = 2 * 3600

    public init(root: URL, digest: ContentDigest) throws {
        self.root = root; self.digest = digest
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    func dir(_ id: UUID) -> URL { root.appendingPathComponent(id.uuidString, isDirectory: true) }
    func name(_ seq: Int) -> String { String(format: "%06d", seq) }

    public func receive(_ header: WatchChunkHeader, file: URL) throws -> WatchChunkReceipt {
        let fm = FileManager.default
        guard header.sequence >= 0, header.sequence < 100_000, header.endSample != nil,
              (8_000...48_000).contains(header.sampleRate), header.end <= Self.maxSeconds,
              header.digest.count == 64, header.digest.allSatisfy(\.isHexDigit) else { throw IntakeError.invalid }
        let size = try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size > 0, size <= Self.maxChunkBytes else { throw IntakeError.invalid }
        let receipt = WatchChunkReceipt(sessionId: header.sessionId, sequence: header.sequence, digest: header.digest)
        let folder = dir(header.sessionId)
        let headerURL = folder.appendingPathComponent(name(header.sequence) + ".json")
        // Doppelt zugestellt: gleiche Quittung, nichts neu schreiben.
        let audio = folder.appendingPathComponent(name(header.sequence) + ".m4a")
        if let data = try? Data(contentsOf: headerURL), let stored = try? JSONDecoder().decode(WatchChunkHeader.self, from: data) {
            guard stored == header else { throw IntakeError.conflict("Stück \(header.sequence) mit anderem Inhalt") }
            // Nur quittieren, wenn das Audio wirklich vorliegt und passt; sonst neu schreiben.
            if fm.fileExists(atPath: audio.path), (try? digest.hex(of: audio)) == header.digest {
                // Der erste Versuch kann vor einem fsync abgebrochen sein: alles erneut sichern.
                try DurableFiles.sync(audio)
                try DurableFiles.sync(headerURL)
                try syncDirectory(folder)
                try syncDirectory(root)
                return receipt
            }
            try fm.removeItem(at: headerURL)
        }
        if header.sequence == 0 {
            guard let c = header.consent, c.isComplete, c.channel == .watch, c.reason == .start, c.scopes.contains(.recording) else { throw IntakeError.consentMissing }
        } else {
            guard fm.fileExists(atPath: folder.appendingPathComponent(name(0) + ".json").path) || header.consent == nil else { throw IntakeError.invalid }
        }
        guard try digest.hex(of: file) == header.digest else { throw IntakeError.digestMismatch }
        if !fm.fileExists(atPath: folder.path) { try fm.createDirectory(at: folder, withIntermediateDirectories: true) }
        let tmp = folder.appendingPathComponent(".tmp-" + UUID().uuidString)
        defer { try? fm.removeItem(at: tmp) }
        try fm.copyItem(at: file, to: tmp)
        try DurableFiles.sync(tmp)
        if fm.fileExists(atPath: audio.path) { try fm.removeItem(at: audio) }
        try fm.moveItem(at: tmp, to: audio)
        try syncDirectory(folder)
        // Kopf zuletzt: er macht das Stück gültig.
        try JSONEncoder().encode(header).write(to: headerURL, options: .atomic)
        try DurableFiles.sync(headerURL)
        // Vor jeder Quittung: Sitzungsordner und Wurzel sichern, auch wenn der Ordner schon
        // bestand (ein früherer Versuch kann nach dem Anlegen vor dem fsync gescheitert sein).
        try syncDirectory(folder)
        try syncDirectory(root)
        return receipt
    }

    public func headers(_ id: UUID) throws -> [WatchChunkHeader] {
        let folder = dir(id)
        guard FileManager.default.fileExists(atPath: folder.path) else { throw IntakeError.unknownSession }
        return try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" && !$0.lastPathComponent.hasPrefix(".") }
            .compactMap { try? JSONDecoder().decode(WatchChunkHeader.self, from: Data(contentsOf: $0)) }
            .filter { $0.sessionId == id }
            .sorted { $0.sequence < $1.sequence }
    }

    public func audioURL(_ header: WatchChunkHeader) -> URL {
        dir(header.sessionId).appendingPathComponent(name(header.sequence) + ".m4a")
    }

    public func status(_ id: UUID) throws -> WatchSessionStatus {
        let hs = try headers(id)
        let seqs = hs.map(\.sequence)
        let maxSeq = (hs.first(where: \.isLast)?.sequence ?? seqs.max() ?? -1)
        let have = Set(seqs)
        let missing = maxSeq < 0 ? [] : (0...maxSeq).filter { !have.contains($0) }
        var gaps: [ClosedRange<Double>] = []
        for (a, b) in zip(hs, hs.dropFirst()) where b.sequence == a.sequence + 1 {
            guard let expected = a.endSample else { continue }
            if b.firstSample != expected {
                let x = Double(min(expected, b.firstSample)) / Double(a.sampleRate), y = Double(max(expected, b.firstSample)) / Double(a.sampleRate)
                gaps.append(x...y)
            }
        }
        let finals = hs.filter(\.isLast)
        return WatchSessionStatus(received: seqs, missingSequences: missing, gaps: gaps, lastReceived: !finals.isEmpty,
                                  startsAtZero: hs.first.map { $0.sequence == 0 && $0.firstSample == 0 } ?? false,
                                  uniformRate: Set(hs.map(\.sampleRate)).count == 1,
                                  singleFinalChunk: finals.count == 1 && finals[0].sequence == seqs.max())
    }

    /// Einwilligung der Uhr (aus Stück 0) für den Nachweis am Dokument.
    public func consent(_ id: UUID) throws -> ConsentRecord? { try headers(id).first { $0.sequence == 0 }?.consent }

    /// Lückenlose Stücke ab Anfang, in Reihenfolge: für den Live-Weg (halbes Live).
    public func contiguousPrefix(_ id: UUID) throws -> [WatchChunkHeader] {
        var out: [WatchChunkHeader] = []
        for h in try headers(id) { guard h.sequence == out.count else { break }; out.append(h) }
        return out
    }

    public func discard(_ id: UUID) { try? FileManager.default.removeItem(at: dir(id)) }
}

/// Uhr-Seite: Stücke bleiben, bis eine passende Quittung kommt.
public struct WatchChunkOutbox: Sendable {
    public let root: URL
    public init(root: URL) throws {
        self.root = root
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    func base(_ sessionId: UUID, _ seq: Int) -> URL { root.appendingPathComponent(sessionId.uuidString + "-" + String(format: "%06d", seq)) }

    /// Nimmt ein fertiges Stück auf (Datei wird in den Ausgang verschoben).
    public func enqueue(_ header: WatchChunkHeader, file: URL) throws {
        let b = base(header.sessionId, header.sequence)
        let audio = b.appendingPathExtension("m4a")
        if FileManager.default.fileExists(atPath: audio.path) { try FileManager.default.removeItem(at: audio) }
        try FileManager.default.moveItem(at: file, to: audio)
        try DurableFiles.sync(audio)
        try DurableFiles.write(try JSONEncoder().encode(header), to: b.appendingPathExtension("json"))
    }

    public func pending() -> [(WatchChunkHeader, URL)] {
        ((try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? [])
            .filter { $0.pathExtension == "json" }
            .compactMap { url -> (WatchChunkHeader, URL)? in
                guard let h = try? JSONDecoder().decode(WatchChunkHeader.self, from: Data(contentsOf: url)) else { return nil }
                let audio = url.deletingPathExtension().appendingPathExtension("m4a")
                return FileManager.default.fileExists(atPath: audio.path) ? (h, audio) : nil
            }
            .sorted { ($0.0.sessionId.uuidString, $0.0.sequence) < ($1.0.sessionId.uuidString, $1.0.sequence) }
    }

    /// Löscht nur bei exakt passender Quittung. Gibt zurück, ob gelöscht wurde.
    @discardableResult public func acknowledge(_ r: WatchChunkReceipt) -> Bool {
        let b = base(r.sessionId, r.sequence)
        guard let data = try? Data(contentsOf: b.appendingPathExtension("json")),
              let h = try? JSONDecoder().decode(WatchChunkHeader.self, from: data), h.digest == r.digest else { return false }
        try? FileManager.default.removeItem(at: b.appendingPathExtension("m4a"))
        try? FileManager.default.removeItem(at: b.appendingPathExtension("json"))
        return true
    }
}
