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
    case consentMissing, invalid, tooLarge, conflict(String), digestMismatch, unknownSession, busy
}

enum DurableFiles {
    /// Datei oder Verzeichnis bis auf den Datenträger sichern: F_FULLFSYNC auf Apple
    /// (fsync allein leert dort nicht den Laufwerkscache). Auf fsync wird nur zurückgefallen,
    /// wenn das Dateisystem F_FULLFSYNC nicht unterstützt (ENOTSUP, EINVAL, ENOTTY); die
    /// Garantie ist dann schwächer (Laufwerkscache). Jeder andere Fehler wird weitergegeben.
    static func sync(_ url: URL) throws {
        let fd = open(url.path, O_RDONLY)
        guard fd >= 0 else { throw IntakeError.invalid }
        defer { close(fd) }
        guard fullSync(fd) else { throw IntakeError.invalid }
    }

    static func fullSync(_ fd: Int32) -> Bool {
        #if canImport(Darwin)
        if fcntl(fd, F_FULLFSYNC) == 0 { return true }
        guard [ENOTSUP, EINVAL, ENOTTY].contains(errno) else { return false }
        #endif
        return fsync(fd) == 0
    }
    static func write(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: .atomic)
        try sync(url)
        try syncDirectory(url.deletingLastPathComponent())
    }
    /// Verzeichniseintrag dauerhaft machen (nach Anlegen, Umbenennen, Löschen).
    static func syncDirectory(_ url: URL) throws { try sync(url) }

    /// Verzeichniskette anlegen; jedes neu angelegte Verzeichnis wird im Elternverzeichnis
    /// gesichert, damit es nach einem Stromausfall noch existiert.
    static func createDirectoryChain(_ url: URL) throws {
        let fm = FileManager.default
        var missing: [URL] = [], cursor = url.standardizedFileURL
        while !fm.fileExists(atPath: cursor.path), cursor.pathComponents.count > 1 {
            missing.insert(cursor, at: 0); cursor = cursor.deletingLastPathComponent()
        }
        for dir in missing {
            do {
                try fm.createDirectory(at: dir, withIntermediateDirectories: false)
            } catch CocoaError.fileWriteFileExists {
                // App und Extension legen die Kette gleichzeitig an: ein vorhandenes Verzeichnis genügt.
                var isDir: ObjCBool = false
                guard fm.fileExists(atPath: dir.path, isDirectory: &isDir), isDir.boolValue else { throw IntakeError.invalid }
            }
            try sync(dir)
            try sync(dir.deletingLastPathComponent())
        }
    }
    static func safeExtension(_ url: URL) -> String? {
        let ext = url.pathExtension.lowercased()
        guard !ext.isEmpty, ext.count <= 10, ext.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { return nil }
        return ext
    }
}

// MARK: - Eingang (Teilen-Menü, „Öffnen in“)

public struct InboxItem: Codable, Equatable, Identifiable, Sendable {
    public enum Source: String, Codable, Sendable { case shareSheet, openIn, files, shortcut }
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
    /// App-Gruppe von App und Share-Extension; der Eingang liegt in ihrem Container.
    public static let appGroup = "group.de.localvoice.prototype"

    /// Eingangsordner im App-Group-Container; `nil`, wenn die Gruppe nicht eingerichtet ist.
    public static func sharedRoot() -> URL? {
        #if canImport(Darwin)
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)?.appendingPathComponent("Eingang", isDirectory: true)
        #else
        return nil
        #endif
    }

    public init(root: URL, digest: ContentDigest) throws {
        self.root = root; self.digest = digest
        try DurableFiles.createDirectoryChain(root)
    }

    /// Kopiert `source` dauerhaft in den Eingang. Größe, Dateiart (reguläre Datei, kein
    /// Symlink) und Endung werden vor dem Kopieren geprüft; die Kopie entsteht im Staging,
    /// wird per Prüfsumme verglichen, synchronisiert und erst dann per Rename sichtbar.
    /// Reste bleiben bei Fehlern nicht liegen. `name` ist der Anzeigename (Metadatum),
    /// falls die Quelle nur eine Zufallsdatei ist.
    public func deposit(_ source: URL, from kind: InboxItem.Source, consent: ConsentRecord, name: String? = nil) throws -> InboxItem {
        guard consent.isComplete, consent.reason == .import, consent.channel == .import else { throw IntakeError.consentMissing }
        let fm = FileManager.default
        let info = try source.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard info.isRegularFile == true, info.isSymbolicLink != true, let bytes = info.fileSize, bytes > 0 else { throw IntakeError.invalid }
        // Zusätzlich ohne Symlink-Auflösung prüfen (lstat): nur eine echte, reguläre Datei.
        guard (try fm.attributesOfItem(atPath: source.path)[.type] as? FileAttributeType) == .typeRegular else { throw IntakeError.invalid }
        guard bytes <= Self.maxBytes else { throw IntakeError.tooLarge }
        guard let ext = DurableFiles.safeExtension(source) else { throw IntakeError.invalid }
        let hex = try digest.hex(of: source)
        // Kennung aus dem Inhalt: dieselbe Datei hat immer dieselbe Eingangs- und Archivkennung,
        // auch nach einem Absturz zwischen Übernahme und Aufräumen (kein Doppelimport).
        let id = Self.itemId(digest: hex)
        // Prüfen, Reparieren und Veröffentlichen je Kennung prozessübergreifend serialisiert:
        // App und Extension sehen unter der Sperre immer den committeten Stand des anderen.
        return try locked(id) {
            let finalDir = root.appendingPathComponent(id.uuidString)
            let item = InboxItem(id: id, source: kind, originalName: Self.displayName(name, fallback: source.lastPathComponent, ext: ext), originalExtension: ext,
                                 bytes: bytes, digest: hex, createdAt: Date(), consent: consent)
            guard fm.fileExists(atPath: finalDir.path) else {
                // Neu: vollständige Kopie im Staging, dann ein einziger Rename.
                let staging = root.appendingPathComponent(".tmp-" + UUID().uuidString)
                try fm.createDirectory(at: staging, withIntermediateDirectories: false)
                defer { try? fm.removeItem(at: staging) }
                try copyVerified(source, to: staging.appendingPathComponent("original." + ext), digest: hex)
                try DurableFiles.write(try JSONEncoder().encode(item), to: staging.appendingPathComponent("eingang.json"))
                try fm.moveItem(at: staging, to: finalDir)
                try DurableFiles.syncDirectory(root)
                return item
            }
            if let same = Self.read(finalDir) {
                // Zweimal geteilt: nur mit denselben Einwilligungsbedingungen derselbe Eintrag.
                // Eine frühere (z. B. KI-)Freigabe wird nie still übernommen oder überschrieben.
                guard same.consent.sameTerms(as: consent) else { throw IntakeError.conflict("bereits mit anderer Einwilligung im Eingang") }
                // Erfolg nur bei vollständigem Bestand (Metadaten passen zum Inhalt, Original intakt).
                if same.bytes == bytes, same.digest == hex, try verifyCommitted(same) { return same }
            }
            try repair(finalDir, with: item, from: source)
            return item
        }
    }

    /// Reparatur im Eintrag selbst, jeder Schritt einzeln atomar und wiederaufsetzbar; der
    /// Eintragsordner verschwindet nie. (1) Neue Kopie als `.neu-…` schreiben, sichern, prüfen.
    /// (2) Per rename(2) auf `original.<ext>` legen. (3) `eingang.json` atomar ersetzen.
    /// (4) Erst danach Originale mit anderer Endung entfernen. Bricht der Prozess ab, bleibt
    /// entweder der alte Stand (mit unbeschädigtem Altoriginal) oder der neue vollständige Stand;
    /// verwaiste `.neu-…` räumt die nächste Reparatur auf. Scheitert ein Schritt vor (3), sind
    /// Metadaten und Altoriginal unverändert.
    private func repair(_ dir: URL, with item: InboxItem, from source: URL) throws {
        let fm = FileManager.default
        let entries = try fm.contentsOfDirectory(atPath: dir.path)
        for name in entries where name.hasPrefix(".neu-") { try fm.removeItem(at: dir.appendingPathComponent(name)) }
        let fresh = dir.appendingPathComponent(".neu-" + UUID().uuidString)
        defer { try? fm.removeItem(at: fresh) }
        try copyVerified(source, to: fresh, digest: item.digest)
        let target = dir.appendingPathComponent("original." + item.originalExtension)
        guard rename(fresh.path, target.path) == 0 else { throw IntakeError.invalid }
        try DurableFiles.syncDirectory(dir)
        try DurableFiles.write(try JSONEncoder().encode(item), to: dir.appendingPathComponent("eingang.json"))
        for name in entries where name.hasPrefix("original.") && name != target.lastPathComponent {
            try fm.removeItem(at: dir.appendingPathComponent(name))
        }
        try DurableFiles.syncDirectory(dir)
        try DurableFiles.syncDirectory(root)
    }

    /// Kopieren (APFS-Klon, wo möglich), sichern und gegen die Prüfsumme der Quelle vergleichen.
    private func copyVerified(_ source: URL, to target: URL, digest hex: String) throws {
        try FileManager.default.copyItem(at: source, to: target)
        try DurableFiles.sync(target)
        guard try digest.hex(of: target) == hex else { throw IntakeError.digestMismatch }
    }

    /// Prozessübergreifende Sperre je Kennung (`flock` auf `.lock-<id>` im Eingang).
    /// `wait: false` wartet nie (für die App im async-Kontext) und wirft `.busy`, wenn der
    /// andere Prozess die Kennung gerade bearbeitet. Die leeren Sperrdateien bleiben liegen:
    /// Löschen unter Konkurrenz würde die Sperre aufheben.
    func locked<T>(_ id: UUID, wait: Bool = true, _ body: () throws -> T) throws -> T {
        let fd = open(root.appendingPathComponent(".lock-" + id.uuidString).path, O_RDWR | O_CREAT, 0o600)
        guard fd >= 0 else { throw IntakeError.invalid }
        defer { close(fd) }
        while flock(fd, wait ? LOCK_EX : LOCK_EX | LOCK_NB) != 0 {
            if errno == EWOULDBLOCK { throw IntakeError.busy }
            guard errno == EINTR else { throw IntakeError.invalid }
        }
        defer { flock(fd, LOCK_UN) }
        return try body()
    }

    static func read(_ dir: URL) -> InboxItem? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("eingang.json")),
              let item = try? JSONDecoder().decode(InboxItem.self, from: data), item.id.uuidString == dir.lastPathComponent else { return nil }
        return item
    }

    /// Inhaltsabgeleitete Kennung (erste 128 Bit der Prüfsumme, als UUID Version 8 markiert).
    public static func itemId(digest hex: String) -> UUID {
        var bytes = stride(from: 0, to: 32, by: 2).map { i -> UInt8 in
            let a = hex.index(hex.startIndex, offsetBy: i), b = hex.index(a, offsetBy: 2)
            return UInt8(hex[a..<b], radix: 16) ?? 0
        }
        bytes[6] = (bytes[6] & 0x0F) | 0x80
        bytes[8] = (bytes[8] & 0x3F) | 0x80
        return UUID(uuid: (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                           bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]))
    }

    /// Anzeigename ohne Pfadtrenner und Steuerzeichen, höchstens 255 Zeichen, mit Endung.
    static func displayName(_ name: String?, fallback: String, ext: String) -> String {
        let raw = (name?.isEmpty == false ? name! : fallback)
        var clean = String(raw.unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) && !"/\\:".unicodeScalars.contains($0) })
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if clean.isEmpty || clean == "." || clean == ".." { clean = "Aufnahme" }
        if !clean.lowercased().hasSuffix("." + ext) { clean += "." + ext }
        return String(clean.suffix(255))
    }

    /// Wartende Einträge, älteste zuerst. Abgebrochene Kopien (`.tmp-…`) zählen nicht.
    public func pending() throws -> [InboxItem] {
        try FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .filter { UUID(uuidString: $0.lastPathComponent) != nil }
            .compactMap { Self.read($0) }
            .sorted { $0.createdAt < $1.createdAt }
    }

    public func fileURL(_ item: InboxItem) -> URL {
        root.appendingPathComponent(item.id.uuidString).appendingPathComponent("original." + item.originalExtension)
    }

    /// Übernimmt einen Eintrag. `importer` legt ihn dauerhaft ins Archiv; erst
    /// danach wird der Eingang geleert. Wirft `importer`, bleibt der Eintrag liegen.
    ///
    /// Koordination mit der Extension ohne blockierende Sperre im async-Kontext:
    /// (1) Unter der Sperre (ohne Warten) den Eintrag neu lesen, verifizieren und als private
    /// Momentaufnahme `.take-…` klonen; der Importer liest nur diese, Reparaturen oder ein
    /// erneutes Teilen ändern sie nicht. (2) Nach dem Import unter der Sperre neu lesen und nur
    /// löschen, wenn derselbe Inhalt mit denselben Bedingungen vorliegt. Eine inzwischen neu
    /// angelegte Fassung mit anderer Einwilligung bleibt liegen (Konfliktpfad der App).
    public func take<T>(_ item: InboxItem, _ importer: (URL, InboxItem) async throws -> T) async throws -> T {
        let inbox = self
        let (snapshot, current) = try await Task.detached { try inbox.claim(item) }.value
        defer { inbox.dropSnapshot(snapshot) }
        let result = try await importer(snapshot.appendingPathComponent("original." + current.originalExtension), current)
        try await Task.detached { try inbox.release(current) }.value
        return result
    }

    func claim(_ item: InboxItem) throws -> (URL, InboxItem) {
        try locked(item.id, wait: false) {
            let dir = root.appendingPathComponent(item.id.uuidString)
            guard let current = Self.read(dir), current.digest == item.digest, current.consent.sameTerms(as: item.consent),
                  try verifyCommitted(current) else { throw IntakeError.conflict("Eintrag hat sich geändert") }
            let snapshot = root.appendingPathComponent(".take-" + UUID().uuidString)
            try FileManager.default.createDirectory(at: snapshot, withIntermediateDirectories: false)
            do {
                try copyVerified(fileURL(current), to: snapshot.appendingPathComponent("original." + current.originalExtension), digest: current.digest)
            } catch { dropSnapshot(snapshot); throw error }
            return (snapshot, current)
        }
    }

    func release(_ item: InboxItem) throws {
        try locked(item.id, wait: false) {
            let dir = root.appendingPathComponent(item.id.uuidString)
            guard let now = Self.read(dir) else {
                // Schon weg oder unlesbar: nur noch das Entfernen dauerhaft machen.
                if FileManager.default.fileExists(atPath: dir.path) { try FileManager.default.removeItem(at: dir) }
                return try DurableFiles.syncDirectory(root)
            }
            guard now.digest == item.digest, now.consent.sameTerms(as: item.consent) else { return }
            try FileManager.default.removeItem(at: dir)
            try DurableFiles.syncDirectory(root)
        }
    }

    func dropSnapshot(_ url: URL) { try? FileManager.default.removeItem(at: url) }

    /// Eintrag entfernen (unbrauchbarer Inhalt), unter der Sperre der Kennung, ohne zu warten.
    public func discard(_ item: InboxItem) throws {
        try locked(item.id, wait: false) {
            let dir = root.appendingPathComponent(item.id.uuidString)
            guard let now = Self.read(dir), now.digest == item.digest, now.consent.sameTerms(as: item.consent) else { return }
            try FileManager.default.removeItem(at: dir)
            try DurableFiles.syncDirectory(root)
        }
    }

    /// Liegt der Eintrag vollständig vor (Original vorhanden, Größe und Prüfsumme stimmen)?
    /// Sichert ihn dabei erneut, falls der erste Commit vor dem Sync abgebrochen ist.
    public func verifyCommitted(_ item: InboxItem) throws -> Bool {
        let file = fileURL(item)
        guard (try? FileManager.default.attributesOfItem(atPath: file.path)[.type] as? FileAttributeType) == .typeRegular,
              (try file.resourceValues(forKeys: [.fileSizeKey]).fileSize) == item.bytes,
              try digest.hex(of: file) == item.digest else { return false }
        try DurableFiles.sync(file)
        try DurableFiles.sync(file.deletingLastPathComponent().appendingPathComponent("eingang.json"))
        try DurableFiles.syncDirectory(file.deletingLastPathComponent())
        try DurableFiles.syncDirectory(root)
        return true
    }

    /// Mindestalter, ab dem eine Staging-Kopie als abgebrochen gilt. App und Extension
    /// kopieren nebenläufig; eine frische `.tmp-`-Kopie gehört womöglich gerade dem
    /// anderen Prozess und wird nie angefasst (Besitz per Alter, kein gemeinsamer Lock nötig).
    public static let abandonedAfter: TimeInterval = 6 * 3600

    /// Reste abgebrochener Kopien entfernen (beim App-Start), nur wenn älter als `abandonedAfter`.
    public func sweepAbandoned(now: Date = Date()) {
        let fm = FileManager.default
        for url in (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        where url.lastPathComponent.hasPrefix(".tmp-") || url.lastPathComponent.hasPrefix(".take-") {
            let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? now
            if now.timeIntervalSince(modified) > Self.abandonedAfter { try? fm.removeItem(at: url) }
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
    var syncDirectory: @Sendable (URL) throws -> Void = { try DurableFiles.syncDirectory($0) }
    public static let maxChunkBytes = 8 * 1024 * 1024
    public static let maxSeconds: Double = 2 * 3600

    public init(root: URL, digest: ContentDigest) throws {
        self.root = root; self.digest = digest
        try DurableFiles.createDirectoryChain(root)
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
