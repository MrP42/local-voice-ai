import Foundation

// Gesprächsmodus „Sprechen“ (lva#521): Latenzmessung am Gerät. Gemessen wird je Runde die Zeit vom erkannten Sprachende (oder vom
// Senden per Hand) bis zum ersten sichtbaren Zeichen und bis zum ersten hörbaren Ton, getrennt nach der Klasse der ersten Einheit
// (ADR-0028 E9). Die Messwerte enthalten NUR Zeiten, Klassen und Zähler: keine Texte, Titel, Fragen oder Kennungen von Inhalten.
//
// Aufbau: `AssistantLatencyClock` misst eine Runde (reiner Werttyp, Zeitquelle austauschbar), `AssistantLatencyLog` hält die letzten
// 200 Messungen dauerhaft und rechnet die Auswertung, `AssistantDurationRing` ist der kleine Ringspeicher für die fsync-Dauer des
// Protokollschreibers (siehe `AssistantAuditWriter.durableStats`).

/// Klasse der ersten Einheit einer Antwort.
public enum AssistantLatencyUnitClass: String, Sendable, Codable, Equatable, CaseIterable {
    /// Vorlage oder Ansage ohne Abruf (feste Sätze, Gruß, Uhrzeit).
    case template
    /// Auszug oder Bestandsangabe (Antwort mit Abruf aus den eigenen Inhalten), auch die Ansage davor und Sätze ohne Quelle.
    case content

    public var title: String {
        switch self {
        case .template: "Vorlage/Ansage"
        case .content: "Antwort mit Abruf"
        }
    }

    /// Obergrenze für p95 „erster Ton“ in Millisekunden (warm, lokal): Vorlage 1,2 s, Antwort mit Abruf 4 s.
    public var gateMs: Int {
        switch self {
        case .template: 1_200
        case .content: 4_000
        }
    }

    var gateText: String {
        switch self {
        case .template: "≤ 1,2 s"
        case .content: "≤ 4 s"
        }
    }
}

/// Eine Messung einer Runde. Ohne `firstVisibleMs` und ohne `firstAudioMs` gibt es keine Messung (wird nicht gespeichert).
public struct AssistantLatencySample: Sendable, Codable, Equatable {
    /// Zeitpunkt der Messung.
    public var at: Date
    /// Millisekunden vom Sprachende (Senden) bis zum ersten sichtbaren Zeichen; `nil`: nie etwas angezeigt.
    public var firstVisibleMs: Int?
    /// Millisekunden bis zum ersten hörbaren Ton; `nil`: nie ein Ton (Wiedergabe aus, gestoppt, gescheitert).
    public var firstAudioMs: Int?
    public var unit: AssistantLatencyUnitClass
    /// Erste Runde nach dem Prozessstart.
    public var cold: Bool
    /// Modell auf dem Gerät oder Endpunkt im privaten Netz; Relay über den PC und Cloud sind nicht lokal.
    public var local: Bool
    /// Feste Modellart, nie ein Modellname.
    public var model: AssistantModelKind

    public init(at: Date, firstVisibleMs: Int?, firstAudioMs: Int?, unit: AssistantLatencyUnitClass, cold: Bool, local: Bool,
                model: AssistantModelKind) {
        self.at = at
        self.firstVisibleMs = firstVisibleMs
        self.firstAudioMs = firstAudioMs
        self.unit = unit
        self.cold = cold
        self.local = local
        self.model = model
    }

    /// Hat die Messung überhaupt einen Wert?
    public var hasValue: Bool { firstVisibleMs != nil || firstAudioMs != nil }

    /// Negative Werte (Uhrensprung) werden zu 0.
    var normalized: AssistantLatencySample {
        var copy = self
        copy.firstVisibleMs = firstVisibleMs.map { max(0, $0) }
        copy.firstAudioMs = firstAudioMs.map { max(0, $0) }
        return copy
    }
}

// MARK: - Messvorgang einer Runde

/// Misst eine Runde: `begin` beim erkannten Sprachende (oder Senden), `markVisible` beim ersten sichtbaren Zeichen, `markAudio` beim
/// ersten hörbaren Ton, `finish` liefert die Messung. Mehrfaches Markieren zählt nur das erste Mal; ein neuer `begin` verwirft eine
/// unfertige Messung; Marken ohne `begin` (oder nach `finish`) werden ignoriert. Die Zeitquelle ist austauschbar (Sekunden, monoton).
public struct AssistantLatencyClock: Sendable {
    /// Monotone Zeit in Sekunden (Systemlaufzeit ohne Schlafphasen-Sprünge der Wanduhr).
    public static let uptime: @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }

    private let now: @Sendable () -> TimeInterval
    private var start: TimeInterval?
    private var visible: TimeInterval?
    private var audio: TimeInterval?
    private var unit: AssistantLatencyUnitClass?

    public init(now: @escaping @Sendable () -> TimeInterval = AssistantLatencyClock.uptime) {
        self.now = now
    }

    /// Läuft eine Messung (begonnen, nicht beendet)?
    public var isRunning: Bool { start != nil }

    /// Beginnt eine Messung; `start` in der Zeitquelle (Standard: jetzt). Eine unfertige Messung wird verworfen.
    public mutating func begin(at start: TimeInterval? = nil) {
        self.start = start ?? now()
        visible = nil
        audio = nil
        unit = nil
    }

    /// Klasse der ersten Einheit; nur der erste Aufruf zählt.
    public mutating func markUnit(_ unit: AssistantLatencyUnitClass) {
        guard start != nil, self.unit == nil else { return }
        self.unit = unit
    }

    /// Erstes sichtbares Zeichen; nur der erste Aufruf zählt.
    public mutating func markVisible(at time: TimeInterval? = nil) {
        guard start != nil, visible == nil else { return }
        visible = time ?? now()
    }

    /// Erster hörbarer Ton; nur der erste Aufruf zählt.
    public mutating func markAudio(at time: TimeInterval? = nil) {
        guard start != nil, audio == nil else { return }
        audio = time ?? now()
    }

    /// Beendet die Messung. `nil`, wenn keine lief oder weder etwas sichtbar noch hörbar wurde (kein Sample ohne Marke).
    public mutating func finish(cold: Bool, local: Bool, model: AssistantModelKind, at date: Date = Date()) -> AssistantLatencySample? {
        guard let start else { return nil }
        let visible = self.visible, audio = self.audio, unit = self.unit
        self.start = nil
        self.visible = nil
        self.audio = nil
        self.unit = nil
        guard visible != nil || audio != nil else { return nil }
        func ms(_ time: TimeInterval?) -> Int? { time.map { max(0, Int(((($0 - start) * 1_000).rounded()))) } }
        return AssistantLatencySample(at: date, firstVisibleMs: ms(visible), firstAudioMs: ms(audio), unit: unit ?? .content, cold: cold,
                                      local: local, model: model)
    }
}

// MARK: - Auswertung

/// Nearest-Rank-Perzentil: der Wert am Rang `ceil(p/100 · n)` der aufsteigend sortierten Werte (kein Mittelwert zweier Werte).
enum AssistantPercentile {
    static func nearestRank<T: Comparable>(_ values: [T], percent: Int) -> T? {
        guard !values.isEmpty else { return nil }
        let sorted = values.sorted()
        let rank = min(sorted.count, max(1, (percent * sorted.count + 99) / 100))
        return sorted[rank - 1]
    }
}

public struct AssistantLatencyPercentiles: Sendable, Equatable {
    public let count: Int
    public let p50: Int
    public let p95: Int

    init?(_ values: [Int]) {
        guard let p50 = AssistantPercentile.nearestRank(values, percent: 50), let p95 = AssistantPercentile.nearestRank(values, percent: 95) else {
            return nil
        }
        count = values.count
        self.p50 = p50
        self.p95 = p95
    }
}

public enum AssistantLatencyVerdict: String, Sendable, Equatable {
    /// Kalt und nicht lokal werden ohne Tor berichtet.
    case noGate
    case met
    case missed
    case tooFew

    public var title: String {
        switch self {
        case .noGate: "kein Tor"
        case .met: "erfüllt"
        case .missed: "verfehlt"
        case .tooFew: "zu wenige Messungen"
        }
    }
}

public struct AssistantLatencyGroup: Sendable, Equatable {
    public let unit: AssistantLatencyUnitClass
    public let cold: Bool
    public let local: Bool
    /// Messungen der Gruppe.
    public let count: Int
    public let visible: AssistantLatencyPercentiles?
    public let audio: AssistantLatencyPercentiles?
    public let verdict: AssistantLatencyVerdict

    /// Das Tor gilt nur warm und lokal.
    public var hasGate: Bool { !cold && local }

    public var title: String { "\(unit.title) · \(cold ? "kalt" : "warm") · \(local ? "lokal" : "nicht lokal")" }
}

public struct AssistantLatencySummary: Sendable, Equatable {
    public let total: Int
    public let groups: [AssistantLatencyGroup]
}

/// Fsync-Dauer des Protokollschreibers (Millisekunden, je Zeilengruppe), über die letzten 200 Gruppen.
public struct AssistantDurableStats: Sendable, Equatable {
    /// Alle gemessenen Gruppen seit dem Start der App.
    public let total: Int
    /// Gruppen im Fenster (höchstens 200).
    public let count: Int
    public let p50Ms: Double?
    public let p95Ms: Double?
    public let maxMs: Double?

    public static let empty = AssistantDurableStats(total: 0, count: 0, p50Ms: nil, p95Ms: nil, maxMs: nil)
}

/// Kleiner Ringspeicher der letzten Dauern in Millisekunden. Eigener kurzer Lock, nie I/O, nie ein Rückruf.
final class AssistantDurationRing: @unchecked Sendable {
    static let capacity = 200

    private let lock = NSLock()
    private var values: [Double] = []
    private var totalCount = 0

    func record(milliseconds: Double) {
        lock.withLock {
            values.append(max(0, milliseconds))
            if values.count > Self.capacity { values.removeFirst(values.count - Self.capacity) }
            totalCount += 1
        }
    }

    func snapshot() -> AssistantDurableStats {
        let (window, total) = lock.withLock { (values, totalCount) }
        return AssistantDurableStats(total: total, count: window.count, p50Ms: AssistantPercentile.nearestRank(window, percent: 50),
                                     p95Ms: AssistantPercentile.nearestRank(window, percent: 95), maxMs: window.max())
    }
}

// MARK: - Speicher

private struct AssistantLatencyFile: Codable {
    var version: Int
    var samples: [AssistantLatencySample]
}

/// Die letzten 200 Messungen, dauerhaft als kleine JSON-Datei im übergebenen Verzeichnis (atomar geschrieben), threadsicher.
/// Eine fehlende, unlesbare oder neuere Datei ergibt einen leeren Start (sie wird beim nächsten Schreiben ersetzt).
public final class AssistantLatencyLog: @unchecked Sendable {
    public static let capacity = 200
    public static let fileVersion = 1
    public static let fileName = "latenz.json"
    /// Unter so vielen Tonmessungen einer Gruppe gibt es kein Urteil.
    public static let minimumForGate = 20

    /// Der Speicher der App: neben dem Verlauf in `Application Support/Assistant`.
    public static let shared = AssistantLatencyLog(directory: AssistantHistoryStore.standardDirectory())

    public let directory: URL
    public var url: URL { directory.appendingPathComponent(Self.fileName) }

    private let lock = NSLock()
    private var stored: [AssistantLatencySample]

    public init(directory: URL) {
        self.directory = directory
        let file = directory.appendingPathComponent(Self.fileName)
        if let data = try? Data(contentsOf: file),
           let decoded = try? Self.decoder().decode(AssistantLatencyFile.self, from: data), decoded.version <= Self.fileVersion {
            stored = Array(decoded.samples.filter(\.hasValue).map(\.normalized).suffix(Self.capacity))
        } else {
            stored = []
        }
    }

    /// Alle Messungen, älteste zuerst.
    public var samples: [AssistantLatencySample] { lock.withLock { stored } }

    /// Fügt eine Messung an (die ältesten über 200 fallen heraus) und schreibt die Datei. Eine Messung ohne Wert wird nicht gespeichert.
    /// Ein Schreibfehler lässt die Messung im Speicher stehen und ist sonst folgenlos (Messwerte sind keine Pflichtdaten).
    public func add(_ sample: AssistantLatencySample) {
        guard sample.hasValue else { return }
        lock.withLock {
            stored.append(sample.normalized)
            if stored.count > Self.capacity { stored.removeFirst(stored.count - Self.capacity) }
            persist()
        }
    }

    /// Löscht alle Messungen und die Datei.
    public func reset() {
        lock.withLock {
            stored.removeAll()
            try? FileManager.default.removeItem(at: url)
        }
    }

    // Nur unter `lock`.
    private func persist() {
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let data = try Self.encoder().encode(AssistantLatencyFile(version: Self.fileVersion, samples: stored))
            #if os(iOS) || os(watchOS) || os(tvOS) || os(visionOS)
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            #else
            try data.write(to: url, options: .atomic)
            #endif
            BookTalkHistoryStore.excludeFromBackup(url)
        } catch {
            // Der Speicher bleibt, die Datei nicht: folgenlos.
        }
    }

    private static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return e
    }

    private static func decoder() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }

    // MARK: Auswertung

    /// Je Gruppe (Klasse × warm/kalt × lokal/nicht lokal) Anzahl, p50 und p95 (Nearest-Rank) für sichtbar und Ton, dazu das Tor. Nur
    /// nicht leere Gruppen, in fester Reihenfolge (Klasse, warm vor kalt, lokal vor nicht lokal). Das Tor gilt nur warm und lokal und
    /// stützt sich auf p95 des ersten Tons; unter 20 Tonmessungen in der Gruppe heißt es „zu wenige Messungen“.
    public func summary() -> AssistantLatencySummary {
        let all = samples
        var groups: [AssistantLatencyGroup] = []
        for unit in AssistantLatencyUnitClass.allCases {
            for cold in [false, true] {
                for local in [true, false] {
                    let members = all.filter { $0.unit == unit && $0.cold == cold && $0.local == local }
                    guard !members.isEmpty else { continue }
                    let audio = AssistantLatencyPercentiles(members.compactMap(\.firstAudioMs))
                    groups.append(AssistantLatencyGroup(
                        unit: unit, cold: cold, local: local, count: members.count,
                        visible: AssistantLatencyPercentiles(members.compactMap(\.firstVisibleMs)), audio: audio,
                        verdict: Self.verdict(unit: unit, cold: cold, local: local, audio: audio)))
                }
            }
        }
        return AssistantLatencySummary(total: all.count, groups: groups)
    }

    private static func verdict(unit: AssistantLatencyUnitClass, cold: Bool, local: Bool, audio: AssistantLatencyPercentiles?) -> AssistantLatencyVerdict {
        guard !cold, local else { return .noGate }
        guard let audio, audio.count >= minimumForGate else { return .tooFew }
        return audio.p95 <= unit.gateMs ? .met : .missed
    }

    /// Eine einfache Texttabelle zum Kopieren (deutsch, nur feste Bezeichnungen und Zahlen). `durable`: die fsync-Dauer des Protokolls.
    public func summaryText(durable: AssistantDurableStats? = nil) -> String {
        let summary = summary()
        var lines = ["Messwerte Gespräch (letzte \(Self.capacity) Runden, Zeiten ab erkanntem Sprachende)"]
        if summary.groups.isEmpty {
            lines.append("Noch keine Messungen.")
        } else {
            lines.append("Gruppe | Anzahl | Sichtbar p50 | Sichtbar p95 | Ton p50 | Ton p95 | Tor erster Ton p95 | Stand")
            for group in summary.groups {
                let gate = group.hasGate ? group.unit.gateText : "–"
                lines.append([group.title, String(group.count), Self.cell(group.visible?.p50), Self.cell(group.visible?.p95),
                              Self.cell(group.audio?.p50), Self.cell(group.audio?.p95), gate, group.verdict.title].joined(separator: " | "))
            }
            lines.append("Gesamt: \(summary.total) Messungen. Tor nur warm und lokal, ab \(Self.minimumForGate) Tonmessungen je Gruppe.")
        }
        if let durable {
            if durable.count == 0 {
                lines.append("Protokoll-fsync: noch keine Messung.")
            } else {
                lines.append("Protokoll-fsync (letzte \(durable.count) Gruppen): p50 \(Self.cell(durable.p50Ms)), p95 \(Self.cell(durable.p95Ms)), Maximum \(Self.cell(durable.maxMs))")
            }
        }
        return lines.joined(separator: "\n")
    }

    /// Anzeige eines Wertes in Millisekunden; „–“ ohne Wert.
    static func cell(_ ms: Int?) -> String { ms.map { "\($0) ms" } ?? "–" }

    static func cell(_ ms: Double?) -> String {
        guard let ms else { return "–" }
        return ms < 10 ? String(format: "%.1f ms", ms).replacingOccurrences(of: ".", with: ",") : "\(Int(ms.rounded())) ms"
    }
}
