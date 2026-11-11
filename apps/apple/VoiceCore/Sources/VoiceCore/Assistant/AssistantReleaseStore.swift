import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Schreibfolge 8, Klasse A): die Zustandsdatei der Freigaben.
// Sie führt Schalter, Einwilligung, Bereiche und Cloud-Zustimmung und ist die EINZIGE Quelle des Stands beim Start: Aus
// Protokollzeilen wird nie ein Stand abgeleitet, und UserDefaults ist keine Quelle der Freigabe mehr (höchstens einmalig beim
// allerersten Start, nur einschränkend, siehe `AssistantReleaseRecord.narrowed`).
//
// Format: Kopfzeile mit Fassung („LVA-FREIGABEN 1“), eine JSON-Zeile (Änderungszähler `revision` und Inhalt), Prüfzeile
// („crc32 xxxxxxxx“ über die beiden ersten Zeilen). Jede Fassung ist vollständig; die bestehende Datei wird NIE überschrieben.
//
// Austausch (Schreibfolge 8 Schritt „Datei ersetzen“), jeder Teilschritt einzeln fehlerinjizierbar (`AssistantStoreStep`):
// 1. Verzeichnis anlegen, 2. die neue Fassung vollständig in eine eigene Temp-Datei schreiben, 3. die Temp-Datei dauerhaft
// bestätigen (fsync), 4. in einem unteilbaren Schritt an die Stelle der alten setzen (rename), 5. das Verzeichnis dauerhaft
// bestätigen (fsync). Erst nach 5 gilt der Austausch als bestätigt. Scheitert oder bricht ein Schritt ab, ist der Ausgang OFFEN
// (die neue Fassung kann schon liegen, ohne bestätigt zu sein): Die Sitzung macht eine Erweiterung dann in diesem Lauf nicht
// wirksam; beim nächsten Start gilt die Fassung, die lesbar vorliegt.
//
// Fehlerfälle und ihre Absicherung (Paket-Regel „riskante Pakete“):
// - Nebenläufigkeit: Zwei Änderungen überlappen nie; die Sitzung reiht ihre Sicherungsfolgen streng nacheinander
//   (`AssistantChangeTicket`), und der Speicher weist einen überlappenden Austausch ab (`overlaps`, nie nötig, im Test 0).
// - Abbruch mitten im Vorgang: je Teilschritt Fehlerinjektion; nach Abbruch liegt nur die vollständige alte oder die
//   vollständige neue Fassung vor; ein Neustart liest die gültige (Test: Abbruch nach dem rename, vor dem Verzeichnis-fsync).
// - Voller Datenträger / Schreibfehler: scheitert Teilschritt 2, bleibt die alte Datei unberührt, die Temp-Datei wird
//   entfernt; Einschränkungen gelten im Prozess weiter und werden erneut gesichert, Erweiterungen werden nicht wirksam.
//   Eine gescheiterte Einschränkung geht dabei nie verloren: Der Speicher merkt sich den engeren, noch nicht gesicherten Stand
//   (`AssistantReleaseCeiling`: Schnitt aus bestätigtem und gewünschtem Stand, Bereiche UND Zustimmungen, Schalter). Jede spätere
//   Fassung, ob mit oder ohne Sitzung, wird darauf begrenzt, bis ein Austausch bestätigt ist; `record` liefert denselben Stand.
//   Eine gescheiterte Erweiterung wird NICHT gemerkt. Das Gemerkte lebt nur im Prozess: Nach einem Neustart vor der gelungenen
//   Sicherung gilt wieder die Datei (benannte Grenze der ADR), der Hinweis „Freigaben nicht gesichert“ steht im Lauf bis dahin.
// - Änderung ohne Sitzung (`updateAreas`, kein Modell): derselbe Ablauf und dieselben Zeilen wie in der Sitzung, nur ohne Generation
//   (es gibt keine): Einschränkung → Datei → Zeile `release`; Erweiterung → Zeile „beabsichtigt“ → Datei → Zeile „wirksam“ (oder
//   „in diesem Lauf nicht wirksam“, sobald schreibbar). Eigenes Lauf-Token je Speicher, Änderungsnummern fortlaufend je Lauf.
//   Sperrordnung: die Folgen ohne Sitzung laufen streng nacheinander (eigene Reihe, `AssistantChangeTicket`); der Austausch läuft
//   auf der einen Warteschlange des Speichers; auf das Protokoll wird nie unter dem Lock des Speichers gewartet (Lock nur kurz,
//   ohne I/O).
// - Fehlendes oder verändertes Gerät/Dateisystem: fehlende Datei bei vorhandenem Protokoll, unlesbarer Inhalt, falscher
//   Prüfwert oder fremde Fassung sperren (`locked`): kein Bereich, keine Cloud-Zustimmung, bis der Nutzer neu setzt und das
//   gesichert ist. Allererster Start (weder Datei noch Protokoll): Auslieferungsstand, erst gesichert, dann nutzbar.
// - Absturz eines Kindprozesses: Dieses Paket startet keinen Kindprozess. Das Prozessende selbst (Absturz, Beenden) ist durch
//   die Austauschfolge und die Prüfung beim Start abgedeckt.

/// Teilschritte des Austauschs der Zustandsdatei (für Fehlerinjektion und Meldungen).
public enum AssistantStoreStep: String, Sendable, Equatable, CaseIterable {
    case prepare, write, syncFile, rename, syncDirectory
}

public enum AssistantStoreError: Error, Sendable, Equatable {
    /// Die Datei ist da, ließ sich aber nicht lesen.
    case readFailed
    /// Kopfzeile, Inhalt oder Prüfwert stimmen nicht (auch eine fremde Fassung): gesperrt.
    case invalid
    /// Die Datei fehlt bei vorhandenem Protokoll: gesperrt.
    case missing
    /// Ein Teilschritt des Austauschs ist gescheitert oder abgebrochen: der Ausgang ist OFFEN.
    case replaceFailed(AssistantStoreStep)
    /// Eine zweite Sicherungsfolge begann, bevor die erste fertig war (Programmierfehler, nie erwartet).
    case overlap
    /// Eine ältere, noch nicht gesicherte Einschränkung begrenzt die Fassung: Was darüber hinausging (eine Erweiterung), gilt nicht.
    /// Die Einschränkung selbst ist mit diesem Austausch gesichert.
    case restrictionPending
}

/// Ergebnis einer Änderung der Bereiche ohne Sitzung (`AssistantReleaseStore.updateAreas`).
public struct AssistantAreaChange: Sendable, Equatable {
    /// Die Bereiche, die jetzt gelten: der bestätigte Stand, begrenzt auf einen engeren, noch nicht gesicherten.
    public var effective: Set<AssistantArea>
    /// Ein Austausch der Zustandsdatei ist gescheitert (Ausgang offen): Eine Einschränkung gilt trotzdem, eine Erweiterung nicht.
    public var storeError: AssistantStoreError?
    /// Eine Zeile des Protokolls ließ sich nicht schreiben („Protokoll nicht schreibbar“). Der gesicherte Stand bleibt.
    public var auditError: AssistantAuditError?
    /// Alles Gewünschte gilt (keine Erweiterung blieb aus).
    public var applied: Bool
}

/// Dateioperationen des Speichers hinter einem kleinen Protokoll: Tests ersetzen sie, um jeden Teilschritt scheitern zu lassen.
public protocol AssistantStoreFileSystem: Sendable {
    /// `nil`: die Datei gibt es nicht. Jeder andere Fehler wird geworfen.
    func read(_ url: URL) throws -> Data?
    func exists(_ url: URL) -> Bool
    func createDirectory(_ url: URL) throws
    /// Legt die Datei NEU an (eigene Datei) und schreibt alle Bytes.
    func writeNew(_ url: URL, _ data: Data) throws
    /// Bestätigt den Inhalt der Datei dauerhaft (fsync).
    func syncFile(_ url: URL) throws
    /// Atomares Umbenennen, ersetzt das Ziel.
    func rename(_ from: URL, to: URL) throws
    /// Bestätigt das Verzeichnis dauerhaft (fsync).
    func syncDirectory(_ url: URL) throws
    /// Best-Effort-Aufräumen.
    func remove(_ url: URL)
}

/// Das echte Dateisystem (POSIX). `F_FULLFSYNC` auf Apple-Plattformen, sonst `fsync`.
public struct AssistantPosixStoreFileSystem: AssistantStoreFileSystem {
    public init() {}

    private static func failure(_ code: Int32 = errno) -> Error {
        NSError(domain: NSPOSIXErrorDomain, code: Int(code))
    }

    private static func fullSync(_ descriptor: Int32) throws {
        #if canImport(Darwin)
        if fcntl(descriptor, F_FULLFSYNC) != -1 { return }
        #endif
        if fsync(descriptor) != 0 { throw failure() }
    }

    public func read(_ url: URL) throws -> Data? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try Data(contentsOf: url)
    }

    public func exists(_ url: URL) -> Bool { FileManager.default.fileExists(atPath: url.path) }

    public func createDirectory(_ url: URL) throws {
        let fm = FileManager.default
        let fresh = !fm.fileExists(atPath: url.path)
        try fm.createDirectory(at: url, withIntermediateDirectories: true)
        if fresh { BookTalkHistoryStore.excludeFromBackup(url) }
    }

    public func writeNew(_ url: URL, _ data: Data) throws {
        let descriptor = open(url.path, O_WRONLY | O_CREAT | O_TRUNC, 0o600)
        guard descriptor >= 0 else { throw Self.failure() }
        defer { close(descriptor) }
        try data.withUnsafeBytes { raw in
            var offset = 0
            while offset < raw.count {
                let written = write(descriptor, raw.baseAddress! + offset, raw.count - offset)
                if written < 0 {
                    if errno == EINTR { continue }
                    throw Self.failure()
                }
                offset += written
            }
        }
        #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
        try? FileManager.default.setAttributes([.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
                                               ofItemAtPath: url.path)
        #endif
    }

    public func syncFile(_ url: URL) throws {
        let descriptor = open(url.path, O_RDONLY)
        guard descriptor >= 0 else { throw Self.failure() }
        defer { close(descriptor) }
        try Self.fullSync(descriptor)
    }

    public func rename(_ from: URL, to: URL) throws {
        if Foundation.rename(from.path, to.path) != 0 { throw Self.failure() }
    }

    public func syncDirectory(_ url: URL) throws {
        let descriptor = open(url.path, O_RDONLY)
        guard descriptor >= 0 else { throw Self.failure() }
        defer { close(descriptor) }
        try Self.fullSync(descriptor)
    }

    public func remove(_ url: URL) { try? FileManager.default.removeItem(at: url) }
}

/// Prüfwert über den Inhalt einer Fassung (CRC-32, IEEE). Er erkennt abgeschnittene und beschädigte Dateien; Schutz gegen
/// Fälschung ist er nicht.
enum AssistantCRC32 {
    private static let table: [UInt32] = (0..<256).map { index in
        var value = UInt32(index)
        for _ in 0..<8 { value = value & 1 != 0 ? 0xEDB8_8320 ^ (value >> 1) : value >> 1 }
        return value
    }

    static func checksum(_ data: Data) -> UInt32 {
        var value: UInt32 = 0xFFFF_FFFF
        for byte in data { value = table[Int((value ^ UInt32(byte)) & 0xFF)] ^ (value >> 8) }
        return value ^ 0xFFFF_FFFF
    }

    static func hex(_ data: Data) -> String {
        let text = String(checksum(data), radix: 16)
        return String(repeating: "0", count: max(0, 8 - text.count)) + text
    }
}

/// Der gesamte Freigabestand der Klasse A, wie die Zustandsdatei ihn führt: Schalter, Einwilligungsversion, Bereiche je Art des
/// Modells (lokal, Cloud) und die Cloud-Zustimmungen je Ziel (mit den Bereichen, für die zugestimmt wurde).
public struct AssistantReleaseRecord: Codable, Equatable, Sendable {
    public var optIn: Bool
    public var consentVersion: Int
    /// Bereichsnamen, sortiert.
    public var localAreas: [String]
    public var cloudAreas: [String]
    /// Ziel-Kennung → Bereichsnamen, für die zugestimmt wurde (sortiert).
    public var consents: [String: [String]]

    public init(optIn: Bool = false, consentVersion: Int = 0, localAreas: Set<AssistantArea>, cloudAreas: Set<AssistantArea>,
                consents: [String: Set<AssistantArea>] = [:]) {
        self.optIn = optIn
        self.consentVersion = optIn ? consentVersion : 0
        self.localAreas = Self.names(localAreas)
        self.cloudAreas = Self.names(cloudAreas)
        self.consents = consents.mapValues(Self.names)
    }

    /// Auslieferungsstand: Schalter aus, keine Cloud-Zustimmung, Bereiche nach Voreinstellung (lokal alle, Cloud nur Bücher).
    public static let shipped = AssistantReleaseRecord(localAreas: Set(AssistantArea.allCases), cloudAreas: [.books])

    /// Der gesperrte Stand: kein Bereich, keine Zustimmung.
    public static let locked = AssistantReleaseRecord(localAreas: [], cloudAreas: [])

    static func names(_ areas: Set<AssistantArea>) -> [String] { areas.map(\.rawValue).sorted() }

    static func areaSet(_ names: [String]) -> Set<AssistantArea> { Set(names.compactMap(AssistantArea.init(rawValue:))) }

    public func areas(cloud: Bool) -> Set<AssistantArea> { Self.areaSet(cloud ? cloudAreas : localAreas) }

    /// Die Zustimmung zu `target` gilt für genau diese Bereiche (leer: keine).
    public func consentedAreas(_ target: String) -> Set<AssistantArea> { Self.areaSet(consents[target] ?? []) }

    /// Die Ansicht einer Sitzung: Bereiche ihrer Art des Modells; Cloud-Zustimmung nur für ein Cloud-Ziel, dessen Zustimmung die
    /// jetzigen Bereiche umfasst.
    func state(cloud: Bool, target: String?) -> AssistantReleaseState {
        let areas = self.areas(cloud: cloud)
        var consent = false
        if cloud, let target { consent = consentedAreas(target).isSuperset(of: areas) }
        return AssistantReleaseState(optIn: optIn, consentVersion: optIn ? consentVersion : 0, areas: areas, cloudConsent: consent)
    }

    /// Setzt die Ansicht einer Sitzung zurück in den Gesamtstand. Eine ausdrückliche Zustimmung (`consent`) ersetzt den Eintrag
    /// des Ziels durch die jetzigen Bereiche, ein Entzug löscht ihn; sonst bleibt ein Eintrag stehen, der die Bereiche deckt
    /// (weniger Bereiche brauchen keine neue Frage), und fällt weg, sobald die Bereiche über ihn hinausgehen.
    func applying(_ state: AssistantReleaseState, cloud: Bool, target: String?, cause: AssistantReleaseCause) -> AssistantReleaseRecord {
        var next = self
        next.optIn = state.optIn
        next.consentVersion = state.optIn ? state.consentVersion : 0
        if cloud { next.cloudAreas = Self.names(state.areas) } else { next.localAreas = Self.names(state.areas) }
        guard cloud, let target else { return next }
        let covered = consentedAreas(target).isSuperset(of: state.areas)
        if cause == .consent {
            next.consents[target] = state.cloudConsent ? Self.names(state.areas) : nil
        } else if state.cloudConsent, !covered {
            next.consents[target] = Self.names(state.areas)
        } else if !state.cloudConsent, !covered {
            // Mehr Bereiche, als die Zustimmung deckt: Der Eintrag fällt weg. Er darf nicht später wieder aufleben, wenn die
            // Bereiche schrumpfen (Zustimmung gilt nur für den Stand, für den sie erteilt wurde, strengere Lesart).
            next.consents[target] = nil
        }
        return next
    }

    /// Übernahme alter Einstellungen beim allerersten Start: nur ENGER als der Auslieferungsstand (Schnittmenge der Bereiche),
    /// nie eine Zustimmung, nie ein Schalter.
    public func narrowed(localAreas: Set<AssistantArea>?, cloudAreas: Set<AssistantArea>?) -> AssistantReleaseRecord {
        var next = self
        if let localAreas { next.localAreas = Self.names(Self.areaSet(self.localAreas).intersection(localAreas)) }
        if let cloudAreas { next.cloudAreas = Self.names(Self.areaSet(self.cloudAreas).intersection(cloudAreas)) }
        return next
    }
}

/// Der engere, noch nicht gesicherte Stand einer gescheiterten Einschränkung: die Obergrenze, die jede spätere Fassung einhalten
/// muss, bis ein Austausch bestätigt ist. Sie hält nur, was einschränkte (Schalter aus, weniger Bereiche, weniger abgedeckte
/// Bereiche einer Zustimmung): Eine Größe, die nicht eingeschränkt wurde, bleibt ohne Grenze (nil), damit eine unabhängige
/// Änderung nicht mit begrenzt wird. Erweiterungen kommen nie hinein.
struct AssistantReleaseCeiling: Equatable, Sendable {
    /// Der Schalter wurde ausgeschaltet.
    var optInOff = false
    /// Höchstens diese lokalen / Cloud-Bereiche (nil: keine Grenze).
    var local: Set<AssistantArea>?
    var cloud: Set<AssistantArea>?
    /// Höchstens diese Bereiche je Ziel decken seine Zustimmung (leer: keine Zustimmung).
    var consents: [String: Set<AssistantArea>] = [:]

    var isEmpty: Bool { !optInOff && local == nil && cloud == nil && consents.isEmpty }

    init() {}

    /// Der einschränkende Anteil des Übergangs von `from` nach `to`: der Schnitt aus bestätigtem und gewünschtem Stand, nur dort, wo
    /// `to` weniger enthält.
    init(narrowing from: AssistantReleaseRecord, to: AssistantReleaseRecord) {
        optInOff = from.optIn && !to.optIn
        let oldLocal = from.areas(cloud: false), newLocal = to.areas(cloud: false)
        if !newLocal.isSuperset(of: oldLocal) { local = oldLocal.intersection(newLocal) }
        let oldCloud = from.areas(cloud: true), newCloud = to.areas(cloud: true)
        if !newCloud.isSuperset(of: oldCloud) { cloud = oldCloud.intersection(newCloud) }
        for (target, names) in from.consents {
            let had = AssistantReleaseRecord.areaSet(names), now = to.consentedAreas(target)
            if !now.isSuperset(of: had) { consents[target] = had.intersection(now) }
        }
    }

    private static func meet(_ a: Set<AssistantArea>?, _ b: Set<AssistantArea>?) -> Set<AssistantArea>? {
        guard let a else { return b }
        guard let b else { return a }
        return a.intersection(b)
    }

    /// Beide Grenzen zugleich.
    func merged(with other: AssistantReleaseCeiling) -> AssistantReleaseCeiling {
        var out = AssistantReleaseCeiling()
        out.optInOff = optInOff || other.optInOff
        out.local = Self.meet(local, other.local)
        out.cloud = Self.meet(cloud, other.cloud)
        out.consents = consents
        for (target, cap) in other.consents { out.consents[target] = out.consents[target].map { $0.intersection(cap) } ?? cap }
        return out
    }

    /// Die Fassung innerhalb der Grenzen. Ein Eintrag der Zustimmung, der danach nichts mehr deckt, fällt weg.
    func apply(to record: AssistantReleaseRecord) -> AssistantReleaseRecord {
        var out = record
        if optInOff {
            out.optIn = false
            out.consentVersion = 0
        }
        if let local { out.localAreas = AssistantReleaseRecord.names(AssistantReleaseRecord.areaSet(out.localAreas).intersection(local)) }
        if let cloud { out.cloudAreas = AssistantReleaseRecord.names(AssistantReleaseRecord.areaSet(out.cloudAreas).intersection(cloud)) }
        for (target, cap) in consents {
            guard let current = out.consents[target] else { continue }
            let kept = AssistantReleaseRecord.areaSet(current).intersection(cap)
            out.consents[target] = kept.isEmpty ? nil : AssistantReleaseRecord.names(kept)
        }
        return out
    }
}

/// Lesen und Prüfen einer Fassung.
enum AssistantReleaseFile {
    static let header = "LVA-FREIGABEN 1"

    private struct Payload: Codable, Equatable {
        var revision: Int
        var record: AssistantReleaseRecord
    }

    private static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return e
    }

    static func encode(_ record: AssistantReleaseRecord, revision: Int) -> Data? {
        guard let payload = try? encoder().encode(Payload(revision: revision, record: record)) else { return nil }
        var body = Data(header.utf8)
        body.append(10)
        body.append(payload)
        body.append(10)
        var out = body
        out.append(Data("crc32 \(AssistantCRC32.hex(body))\n".utf8))
        return out
    }

    /// `nil`: nicht gültig (leer, abgeschnitten, falscher Prüfwert, fremde Fassung, unlesbarer Inhalt).
    static func decode(_ data: Data) -> (record: AssistantReleaseRecord, revision: Int)? {
        let lines = data.split(separator: 10, omittingEmptySubsequences: false)
        // Genau drei Zeilen und ein abschließender Zeilenumbruch.
        guard lines.count == 4, lines[3].isEmpty, String(decoding: lines[0], as: UTF8.self) == header else { return nil }
        var body = Data(lines[0])
        body.append(10)
        body.append(Data(lines[1]))
        body.append(10)
        guard String(decoding: lines[2], as: UTF8.self) == "crc32 \(AssistantCRC32.hex(body))" else { return nil }
        guard let payload = try? JSONDecoder().decode(Payload.self, from: Data(lines[1])), payload.revision >= 1 else { return nil }
        return (payload.record, payload.revision)
    }
}

/// Der Speicher der Zustandsdatei. Ein Objekt je Verzeichnis und Prozess (der Controller hält es). Alle Austausche laufen auf
/// einer eigenen seriellen Warteschlange, nie im Lock der Freigabe und nie auf einem Aktor.
public final class AssistantReleaseStore: @unchecked Sendable {
    /// Wie der Speicher beim Öffnen gestartet ist.
    public enum Opening: Sendable, Equatable {
        /// Eine gültige Fassung wurde geladen.
        case loaded(revision: Int)
        /// Allererster Start: der Auslieferungsstand wurde als erste Fassung gesichert.
        case firstStart
        /// Gesperrt: Datei fehlt bei vorhandenem Protokoll, ist ungültig oder unlesbar, oder die erste Fassung ließ sich nicht sichern.
        case locked(AssistantStoreError)
    }

    public static let fileName = "freigaben.json"
    static let tempName = "freigaben.neu"

    public let directory: URL
    let fileSystem: any AssistantStoreFileSystem
    private let queue = DispatchQueue(label: "de.wolffappliedai.localvoice.assistant.freigaben")
    /// Die EINE Ordnung dieses Prozesslaufs (Lock, Generation, Lauf-Token, Standzeile, Änderungszähler, Reihe der Folgen, Register
    /// der Anfragen): Der Speicher besitzt sie, jede Sitzung und jede Änderung ohne Sitzung benutzt sie.
    let clock = AssistantReleaseClock()
    /// Der Lock der Ordnung: derselbe für Generation, Wiedergabesperre, Anfragen und den Stand dieser Datei im Speicher.
    private var lock: NSLock { clock.lock }
    // Hinter `lock`:
    private var confirmed: AssistantReleaseRecord
    private var revision = 0
    private var attempts = 0
    private var lockedState: Bool
    private var blockedState = false
    private var dirtyState = false
    private var running = false
    private var overlapCount = 0
    /// Der engere, noch nicht gesicherte Stand (leer: es gibt keinen).
    private var ceiling = AssistantReleaseCeiling()
    /// Zeilen „in diesem Lauf nicht wirksam“, die noch nicht geschrieben werden konnten.
    private var unwritten: [UnwrittenLine] = []
    public private(set) var opening: Opening

    private struct UnwrittenLine {
        let number: Int
        let record: AssistantReleaseRecord
        let cloud: Bool
    }

    var url: URL { directory.appendingPathComponent(Self.fileName) }
    var tempURL: URL { directory.appendingPathComponent(Self.tempName) }

    /// `protocolURL`: das Protokoll neben der Datei (sein Vorhandensein entscheidet über „allererster Start“). `adopt`: alte
    /// Einstellungen für den allerersten Start (nur einschränkend übernommen).
    public convenience init(directory: URL, protocolURL: URL? = nil, adopt: AssistantReleaseRecord? = nil) {
        self.init(directory: directory, protocolURL: protocolURL ?? directory.appendingPathComponent(AssistantAuditLog.fileName),
                  fileSystem: AssistantPosixStoreFileSystem(), adopt: adopt)
    }

    /// Öffnen entscheidet allein nach der Datei (nie nach Protokollzeilen): gültige Fassung → geladen; Datei fehlt und es gibt
    /// ein Protokoll → gesperrt; Datei fehlt und kein Protokoll → Auslieferungsstand (mit der Übernahme `adopt`), als erste
    /// Fassung gesichert; alles andere → gesperrt.
    init(directory: URL, protocolURL: URL?, fileSystem: any AssistantStoreFileSystem, adopt: AssistantReleaseRecord? = nil) {
        self.directory = directory
        self.fileSystem = fileSystem
        fileSystem.remove(directory.appendingPathComponent(Self.tempName))   // Rest eines abgebrochenen Austauschs
        let url = directory.appendingPathComponent(Self.fileName)
        var opening: Opening
        var record = AssistantReleaseRecord.locked
        var loadedRevision = 0
        var isLocked = true
        do {
            if let data = try fileSystem.read(url) {
                if let loaded = AssistantReleaseFile.decode(data) {
                    record = loaded.record
                    loadedRevision = loaded.revision
                    isLocked = false
                    opening = .loaded(revision: loaded.revision)
                } else {
                    opening = .locked(.invalid)
                }
            } else if let protocolURL, fileSystem.exists(protocolURL) {
                opening = .locked(.missing)
            } else {
                opening = .firstStart
            }
        } catch {
            opening = .locked(.readFailed)
        }
        self.confirmed = record
        self.revision = loadedRevision
        self.attempts = loadedRevision
        self.lockedState = isLocked
        self.opening = opening
        if case .firstStart = opening {
            let first = adopt.map { shipped in AssistantReleaseRecord.shipped.narrowed(localAreas: shipped.areas(cloud: false),
                                                                                         cloudAreas: shipped.areas(cloud: true)) }
                ?? AssistantReleaseRecord.shipped
            if let error = self.replaceNow(first) {
                self.opening = .locked(error)   // nicht gesichert: kein Stand, nichts wird angenommen
            } else {
                self.lockedState = false
            }
        }
    }

    // MARK: Zustand

    /// Der Stand, der gilt: der letzte BESTÄTIGTE (beim gesperrten Start der leere), begrenzt auf einen engeren, noch nicht gesicherten
    /// Stand einer gescheiterten Einschränkung. Eine Sitzung dieses Laufs und die Anzeige starten daher nie weiter als das, was der
    /// Nutzer eingeschränkt hat.
    public var record: AssistantReleaseRecord {
        lock.lock()
        defer { lock.unlock() }
        return ceiling.apply(to: confirmed)
    }

    /// Eine Einschränkung ist nicht gesichert: Der gemerkte engere Stand begrenzt jede spätere Fassung.
    var hasPendingRestriction: Bool {
        lock.lock()
        defer { lock.unlock() }
        return !ceiling.isEmpty
    }

    /// Es gibt etwas nachzuholen: eine nicht gesicherte Einschränkung (gemerkter engerer Stand, gescheiterter Austausch). Ein beim
    /// Start gesperrter Speicher zählt nicht (er braucht die neue Wahl des Nutzers, nicht dieses Nachholen).
    var needsCatchUp: Bool {
        lock.lock()
        defer { lock.unlock() }
        return (!ceiling.isEmpty || dirtyState) && !lockedState
    }

    /// Zeilen „in diesem Lauf nicht wirksam“ (Änderungen ohne Sitzung), die noch nicht geschrieben werden konnten.
    var hasUnwrittenLines: Bool {
        lock.lock()
        defer { lock.unlock() }
        return !unwritten.isEmpty
    }

    /// Holt die noch ausstehenden Zeilen der Änderungen ohne Sitzung nach (bei jeder Gelegenheit: Anfrage, Änderung). Der Fehler der
    /// ersten Zeile, die sich weiter nicht schreiben lässt, sonst `nil`.
    @discardableResult
    func flushUnwritten(_ audit: AssistantAuditLog?) async -> AssistantAuditError? {
        var out = AssistantAreaChange(effective: [], storeError: nil, auditError: nil, applied: true)
        await catchUpLines(audit, into: &out)
        return out.auditError
    }

    /// Änderungszähler der zuletzt bestätigten Fassung (0: keine).
    public var confirmedRevision: Int {
        lock.lock()
        defer { lock.unlock() }
        return revision
    }

    /// Die Datei war beim Start ungültig oder fehlte; es gilt kein Bereich und keine Zustimmung, bis der Nutzer neu setzt und das
    /// gesichert ist.
    public var isLocked: Bool {
        lock.lock()
        defer { lock.unlock() }
        return lockedState
    }

    /// Eine Einschränkung ließ sich nicht sichern: für den Rest des Prozesslaufs nichts aus eigenen Inhalten.
    public var isBlocked: Bool {
        lock.lock()
        defer { lock.unlock() }
        return blockedState
    }

    /// Der Stand im Prozess ist enger oder anders als die Datei (ein Austausch steht aus oder ist gescheitert).
    public var isDirty: Bool {
        lock.lock()
        defer { lock.unlock() }
        return dirtyState
    }

    /// Zahl überlappender Sicherungsfolgen (Programmierfehler; Tests erwarten 0).
    public var overlaps: Int {
        lock.lock()
        defer { lock.unlock() }
        return overlapCount
    }

    func markBlocked() {
        lock.lock()
        blockedState = true
        lock.unlock()
    }

    // MARK: Austausch

    private func onQueue<T: Sendable>(_ body: @escaping @Sendable () -> T) async -> T {
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume(returning: body()) }
        }
    }

    /// Reiht eine Folge ohne Sitzung hinter die vorigen (streng nacheinander, in der Reihenfolge der Aufrufe): dieselbe Reihe wie
    /// die Folgen der Sitzungen.
    private func nextTicket() -> AssistantChangeTicket { clock.nextTicket() }

    /// Setzt den Stand einer Sitzung (Ansicht `state`) in den Gesamtstand und ersetzt die Datei absturzatomar. Läuft auf der
    /// eigenen Warteschlange; kehrt zurück, wenn der Austausch bestätigt ist oder gescheitert (`nil`: bestätigt). Gesperrt
    /// startend: Grundlage ist der leere Stand; nach bestätigtem Austausch ist der Speicher entsperrt. Die Fassung wird auf einen
    /// engeren, noch nicht gesicherten Stand begrenzt (`commitNow`).
    func persist(_ state: AssistantReleaseState, cloud: Bool, target: String?, cause: AssistantReleaseCause) async -> AssistantStoreError? {
        await onQueue {
            let wanted = self.record.applying(state, cloud: cloud, target: target, cause: cause)
            return self.commitNow(wanted)
        }
    }

    /// Schreibt `wanted`, begrenzt auf den gemerkten engeren Stand (nur auf der Warteschlange). `nil`: bestätigt und unverändert
    /// geschrieben. `.restrictionPending`: bestätigt, aber `wanted` ging über die noch nicht gesicherte Einschränkung hinaus (die
    /// Erweiterung gilt nicht; die Einschränkung ist damit gesichert).
    /// `onEffective`: läuft IM Lock im Schritt der Bestätigung (nur wenn `wanted` unbegrenzt geschrieben wurde) mit der dort vergebenen
    /// Generation und der bestätigten Fassung: Die Erweiterung wird wirksam, die Generation steigt, die Zeile wird eingereiht.
    private func commitNow(_ wanted: AssistantReleaseRecord,
                           onEffective: ((Int, AssistantReleaseRecord) -> Void)? = nil) -> AssistantStoreError? {
        let limited = lock.withLock { ceiling }.apply(to: wanted)
        let whole = limited == wanted
        if let error = replaceNow(limited, onConfirmed: whole ? onEffective : nil) { return error }
        return whole ? nil : .restrictionPending
    }

    /// Versucht, den gemerkten engeren Stand ohne Sitzung zu sichern („Erneut sichern“), streng nacheinander mit den anderen
    /// Änderungen (gleiche Warteschlange). Auch ohne gemerkten Stand schreibt es die Datei neu. Ein gesperrter Speicher bleibt
    /// gesperrt (er braucht die neue Wahl des Nutzers, nicht diesen Weg). Die Blockade für den Rest des Laufs bleibt in jedem Fall.
    @discardableResult
    public func retrySave() async -> AssistantStoreError? {
        let ticket = nextTicket()
        await ticket.turn()
        defer { ticket.complete() }
        return await onQueue {
            if self.isLocked {
                if case .locked(let reason) = self.opening { return reason }
                return .invalid
            }
            return self.replaceNow(self.record)
        }
    }

    // MARK: Änderung ohne Sitzung

    /// Das Ergebnis des einschränkenden Teils.
    private struct Restriction: Sendable {
        /// Die Änderungsnummer, wenn wirklich eingeschränkt wurde (dann folgt die Zeile `release`), und ihre Generation (im selben
        /// Lock-Schritt vergeben, in dem der engere Stand gilt).
        var number: Int?
        var generation: Int?
        var record: AssistantReleaseRecord
        var error: AssistantStoreError?
        /// Es kommen Bereiche dazu (ein erweiternder Teil folgt).
        var grows: Bool
    }

    private static func setting(_ areas: Set<AssistantArea>, cloud: Bool, covering coverage: Set<AssistantArea>,
                                in record: AssistantReleaseRecord) -> AssistantReleaseRecord {
        var next = record
        if cloud {
            next.cloudAreas = AssistantReleaseRecord.names(areas)
            next.consents = next.consents.filter { AssistantReleaseRecord.areaSet($0.value).isSuperset(of: coverage) }
        } else {
            next.localAreas = AssistantReleaseRecord.names(areas)
        }
        return next
    }

    /// Ändert die Bereiche OHNE Sitzung (es läuft kein Modell): Die Datei ist die einzige Quelle des Stands. Derselbe Ablauf und
    /// dieselben Zeilen wie in der Sitzung (Schreibfolge 8), nur ohne Generation, denn ohne Sitzung gibt es keine:
    /// - Einschränkung (Bereich weg, Zustimmung, die die neuen Bereiche nicht deckt, weg): gilt sofort (der Stand `record` ist enger,
    ///   bevor die Datei ersetzt ist), dann Datei ersetzen, dann Zeile `release` (Lauf, Änderungsnummer, Ursache, neuer Stand).
    ///   Scheitert der Austausch, ist der Speicher blockiert (nichts aus eigenen Inhalten in diesem Lauf), der engere Stand wird
    ///   gemerkt und bei jeder weiteren Gelegenheit erneut gesichert; die Zeile steht trotzdem.
    /// - Erweiterung (Bereich dazu): Zeile „beabsichtigt“ dauerhaft, dann Datei ersetzen (der Commit), dann Zeile „wirksam“. Scheitert
    ///   die Zeile „beabsichtigt“ oder der Austausch, gilt die Erweiterung in diesem Lauf nicht; die Zeile „in diesem Lauf nicht
    ///   wirksam“ steht, sobald schreibbar. Ein blockierter Speicher nimmt keine Erweiterung an (wie die Sitzung).
    /// - Gemischt: erst der einschränkende, dann der erweiternde Teil (zwei Austausche, zwei Änderungsnummern).
    /// Ein Fehler einer Zeile steht in `auditError`; der gesicherte Stand bleibt. Die Folgen laufen streng nacheinander und auf
    /// derselben Warteschlange wie die Austausche der Sitzungen. `audit`: das Protokoll (`nil`: keine Zeilen).
    public func updateAreas(_ areas: Set<AssistantArea>, cloud: Bool, audit: AssistantAuditLog? = nil) async -> AssistantAreaChange {
        let ticket = nextTicket()
        await ticket.turn()
        defer { ticket.complete() }
        var out = AssistantAreaChange(effective: [], storeError: nil, auditError: nil, applied: true)
        await catchUpLines(audit, into: &out)

        let restriction = await onQueue { self.restrictNow(areas, cloud: cloud) }
        out.storeError = restriction.error
        if let number = restriction.number {
            let error = await writeLine(.release, number: number, generation: restriction.generation, record: restriction.record,
                                        cloud: cloud, audit: audit)
            out.auditError = out.auditError ?? error
        }
        if restriction.grows { await expand(areas, cloud: cloud, audit: audit, into: &out) }

        let effective = record.areas(cloud: cloud)
        out.effective = effective
        out.applied = effective == areas
        return out
    }

    /// Der einschränkende Teil (nur auf der Warteschlange): der engere Stand gilt sofort, dann die Datei. Auch ohne neue
    /// Einschränkung wird ein noch nicht gesicherter engerer Stand erneut gesichert.
    private func restrictNow(_ areas: Set<AssistantArea>, cloud: Bool) -> Restriction {
        let (base, pending) = lock.withLock { (ceiling.apply(to: confirmed), !ceiling.isEmpty) }
        let current = base.areas(cloud: cloud)
        let next = Self.setting(current.intersection(areas), cloud: cloud, covering: areas, in: base)
        let narrows = next != base
        let grows = !areas.isSubset(of: current)
        guard narrows || pending else { return Restriction(number: nil, generation: nil, record: base, error: nil, grows: grows) }
        var number: Int?
        var generation: Int?
        if narrows {
            // Der engere Stand gilt, die Generation steigt und laufende Anfragen ALLER Sitzungen enden: ein Schritt im Lock.
            let after: AssistantRaiseAftermath = lock.withLock {
                ceiling = ceiling.merged(with: AssistantReleaseCeiling(narrowing: base, to: next))
                number = clock.nextChangeLocked()
                let g = clock.raiseLocked()
                generation = g
                return clock.drainLocked(g)
            }
            after.finish()
        }
        let error = replaceNow(next)
        if error != nil, narrows { markBlocked() }
        return Restriction(number: number, generation: generation, record: next, error: error, grows: grows)
    }

    private func expand(_ areas: Set<AssistantArea>, cloud: Bool, audit: AssistantAuditLog?, into out: inout AssistantAreaChange) async {
        let (number, intended, blocked) = lock.withLock { () -> (Int, AssistantReleaseRecord, Bool) in
            (clock.nextChangeLocked(), Self.setting(areas, cloud: cloud, covering: areas, in: ceiling.apply(to: confirmed)), blockedState)
        }
        // Ein blockierter Speicher (eine Einschränkung ließ sich nicht sichern) nimmt keine Erweiterung an.
        if blocked { return await refuse(number, intended, cloud: cloud, audit: audit, into: &out) }
        // (1) „Erweiterung beabsichtigt“ dauerhaft; scheitert das, kein Austausch.
        if let error = await writeLine(.intent, number: number, generation: nil, record: intended, cloud: cloud, audit: audit) {
            out.auditError = out.auditError ?? error
            return await refuse(number, intended, cloud: cloud, audit: audit, into: &out)
        }
        // (2) Datei ersetzen: der Commit der Erweiterung. Mit der Bestätigung wird sie wirksam, und im selben Lock-Schritt steigt die
        // Generation (laufende Anfragen ALLER Sitzungen enden) und wird die Zeile „wirksam“ mit dieser Generation eingereiht.
        let error = await onQueue {
            self.commitNow(Self.setting(areas, cloud: cloud, covering: areas, in: self.record)) { generation, confirmed in
                self.enqueueLine(.effective, number: number, generation: generation, record: confirmed, cloud: cloud, audit: audit)
            }
        }
        if let error {
            out.storeError = error
            return await refuse(number, intended, cloud: cloud, audit: audit, into: &out)
        }
        // (3) Die Zeile „wirksam“ steht dauerhaft; ihr Fehler wird gemeldet, die Erweiterung bleibt (gesichert).
        if audit != nil, let written = await audit?.waitForLine(key: clock.key("chg", number)) { out.auditError = out.auditError ?? written }
    }

    /// Die Erweiterung gilt in diesem Lauf nicht: Die Zeile „in diesem Lauf nicht wirksam“ wird geschrieben, sobald es geht.
    private func refuse(_ number: Int, _ intended: AssistantReleaseRecord, cloud: Bool, audit: AssistantAuditLog?,
                        into out: inout AssistantAreaChange) async {
        lock.withLock {
            if !unwritten.contains(where: { $0.number == number }) { unwritten.append(UnwrittenLine(number: number, record: intended, cloud: cloud)) }
        }
        await catchUpLines(audit, into: &out)
    }

    /// Holt Zeilen „in diesem Lauf nicht wirksam“ nach, die noch nicht geschrieben werden konnten.
    private func catchUpLines(_ audit: AssistantAuditLog?, into out: inout AssistantAreaChange) async {
        guard audit != nil else { return }
        for item in lock.withLock({ unwritten }) {
            if let error = await writeLine(.ineffective, number: item.number, generation: nil, record: item.record, cloud: item.cloud, audit: audit) {
                out.auditError = out.auditError ?? error
                continue
            }
            lock.withLock { unwritten.removeAll { $0.number == item.number } }
        }
    }

    /// Reiht eine Zeile der Klasse A ohne Sitzung ein (`release`, `intent`, `effective`, `ineffective`), ohne zu warten: Lauf vom
    /// Token der Ordnung beim Schreiben, Änderungsnummer, Ursache `areas`, der Stand danach, ohne Titel und Inhalte. `generation`:
    /// nur `release` und `effective` (die im Lock vergebene Generation dieser Änderung). Gibt den Fehlerschlüssel zurück (je Lauf und
    /// Änderung eindeutig, derselbe Schlüsselraum wie in den Sitzungen). Darf im Lock der Ordnung aufgerufen werden (nur Einreihen).
    @discardableResult
    private func enqueueLine(_ kind: AssistantJournalLine.Kind, number: Int, generation: Int?, record: AssistantReleaseRecord, cloud: Bool,
                             audit: AssistantAuditLog?) -> String {
        let key = clock.key("chg", number)
        guard let audit else { return key }
        var line = AssistantJournalLine(kind)
        line.at = Date()
        line.chg = number
        line.gen = generation
        line.cause = .areas
        line.areas = AssistantReleaseRecord.names(record.areas(cloud: cloud))
        line.unsourced = record.optIn
        line.consent = record.consentVersion
        line.cloud = cloud
        audit.enqueue(line, key: key, run: clock.run)
        return key
    }

    /// Wie `enqueueLine`, wartet aber, bis die Zeile (dauerhaft) steht.
    private func writeLine(_ kind: AssistantJournalLine.Kind, number: Int, generation: Int?, record: AssistantReleaseRecord, cloud: Bool,
                           audit: AssistantAuditLog?) async -> AssistantAuditError? {
        guard let audit else { return nil }
        let key = enqueueLine(kind, number: number, generation: generation, record: record, cloud: cloud, audit: audit)
        return await audit.waitForLine(key: key)
    }

    /// Der Austausch selbst (blockierend, nur auf der Warteschlange oder beim Öffnen). Gelingt er, ist `record` bestätigt und ein
    /// gemerkter engerer Stand erfüllt (er war Teil der Fassung). Scheitert er, merkt der Speicher, was an `record` einschränkt.
    func replaceNow(_ record: AssistantReleaseRecord, onConfirmed: ((Int, AssistantReleaseRecord) -> Void)? = nil) -> AssistantStoreError? {
        lock.lock()
        if running {
            overlapCount += 1
            lock.unlock()
            return .overlap
        }
        running = true
        attempts += 1
        let number = attempts
        lock.unlock()
        let error = writeVersion(record, revision: number)
        lock.lock()
        running = false
        var after: AssistantRaiseAftermath?
        if error == nil {
            confirmed = record
            revision = number
            lockedState = false
            dirtyState = false
            ceiling = AssistantReleaseCeiling()
            if let onConfirmed {
                let g = clock.raiseLocked()
                onConfirmed(g, record)
                after = clock.drainLocked(g)
            }
        } else {
            dirtyState = true
            ceiling = ceiling.merged(with: AssistantReleaseCeiling(narrowing: confirmed, to: record))
        }
        lock.unlock()
        after?.finish()
        return error
    }

    private func writeVersion(_ record: AssistantReleaseRecord, revision: Int) -> AssistantStoreError? {
        guard let data = AssistantReleaseFile.encode(record, revision: revision) else { return .replaceFailed(.write) }
        var step = AssistantStoreStep.prepare
        do {
            try fileSystem.createDirectory(directory)
            step = .write
            try fileSystem.writeNew(tempURL, data)
            step = .syncFile
            try fileSystem.syncFile(tempURL)
            step = .rename
            try fileSystem.rename(tempURL, to: url)
            step = .syncDirectory
            try fileSystem.syncDirectory(directory)
            return nil
        } catch {
            if step != .syncDirectory { fileSystem.remove(tempURL) }
            return .replaceFailed(step)
        }
    }
}
