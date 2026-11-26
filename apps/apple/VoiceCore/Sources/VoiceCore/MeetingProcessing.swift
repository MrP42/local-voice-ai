import Foundation
import CoreFoundation

/// Laufzeitfreigabe ohne App-Frameworks; Checkpoints gehören weiterhin dem Archiv.
public struct MeetingProcessing: Sendable {
    public var foreground = false
    public var deferred = false
    public private(set) var identifier: String?
    private var suspended = false
    private var workerRunning = false
    public private(set) var backgroundEnabled = true
    public var allowed: Bool { !suspended && (foreground || (backgroundEnabled && (deferred || identifier != nil))) }

    public init() {}
    public mutating func submitted(_ identifier: String) { self.identifier = identifier }
    public func started(_ identifier: String) -> Bool { backgroundEnabled && self.identifier == identifier }
    /// Freigaben verfallen sofort; der Worker bleibt bis zu seinem sauberen Ende belegt.
    @discardableResult
    public mutating func setBackgroundEnabled(_ enabled: Bool) -> Bool {
        backgroundEnabled = enabled
        return enabled ? false : finish()
    }
    public mutating func resume() { suspended = false }
    /// Nur ausdrückliche Nutzeraktionen heben die Ablaufsperre ohne neues Systemereignis auf.
    public mutating func allowStart(userInitiated: Bool) -> Bool {
        if userInitiated { resume() }
        return allowed
    }
    public mutating func beginWorker() -> Bool {
        guard allowed, !workerRunning else { return false }
        workerRunning = true
        return true
    }
    public mutating func endWorker() { workerRunning = false }
    @discardableResult
    public mutating func expire(_ identifier: String) -> Bool {
        guard self.identifier == identifier else { return false }
        self.identifier = nil
        suspended = true
        return true
    }
    public mutating func expireDeferred() -> Bool {
        guard deferred else { return false }
        deferred = false; suspended = true
        return true
    }
    @discardableResult
    public mutating func finish() -> Bool {
        guard identifier != nil || deferred else { return false }
        identifier = nil; deferred = false
        return true
    }

    public static func transcription(offset: Double, duration: Double) -> Int64 {
        guard offset.isFinite, duration.isFinite, duration > 0 else { return 0 }
        return Int64(min(1, max(0, offset / duration)) * 800)
    }
    public static func minutes(section: Int, total: Int) -> Int64 {
        guard total > 0 else { return 800 }
        return min(999, 800 + Int64(min(1, max(0, Double(section - 1) / Double(total))) * 200))
    }
    public static func requestGPU(supported: Bool, entitled: Bool) -> Bool { supported && entitled }

    /// Nur das signierte XML-Entitlement des laufenden, dünnen Mach-O zählt, kein Info.plist-Schalter.
    /// Unbekannte Formate (auch reine DER-Entitlements) bleiben ohne GPU-Freigabe.
    public static func hasGPUEntitlement(executable data: Data) -> Bool {
        func word(_ offset: Int, little: Bool = false) -> Int? {
            guard offset >= 0, offset <= data.count - 4 else { return nil }
            let bytes = (0..<4).map { Int(data[data.startIndex + offset + $0]) }
            return little ? bytes[0] | bytes[1] << 8 | bytes[2] << 16 | bytes[3] << 24
                : bytes[3] | bytes[2] << 8 | bytes[1] << 16 | bytes[0] << 24
        }
        guard word(0, little: true) == 0xfeedfacf, let commands = word(16, little: true),
              let commandBytes = word(20, little: true), commandBytes <= data.count - 32,
              commands <= commandBytes / 8 else { return false }
        var cursor = 32
        for _ in 0..<commands {
            guard let command = word(cursor, little: true), let size = word(cursor + 4, little: true),
                  size >= 8, size <= 32 + commandBytes - cursor else { return false }
            defer { cursor += size }
            guard command == 0x1d else { continue } // LC_CODE_SIGNATURE
            guard size >= 16, let start = word(cursor + 8, little: true),
                  let count = word(cursor + 12, little: true), start >= 32 + commandBytes,
                  count <= data.count - start, count >= 12,
                  word(start) == 0xfade0cc0, let length = word(start + 4), length <= count, length >= 12,
                  let slots = word(start + 8), slots <= (length - 12) / 8 else { return false }
            for index in 0..<slots {
                let entry = start + 12 + index * 8
                guard word(entry) == 5 else { continue } // CSSLOT_ENTITLEMENTS
                guard let offset = word(entry + 4), offset >= 12 + slots * 8, offset <= length - 8,
                      word(start + offset) == 0xfade7171, let blobSize = word(start + offset + 4),
                      blobSize >= 8, blobSize <= length - offset else { return false }
                let plist = data.subdata(in: (start + offset + 8)..<(start + offset + blobSize))
                guard let values = (try? PropertyListSerialization.propertyList(from: plist, options: [], format: nil)) as? [String: Any],
                      let value = values["com.apple.developer.background-tasks.continued-processing.gpu"] as? NSNumber,
                      CFGetTypeID(value) == CFBooleanGetTypeID() else { return false }
                return value.boolValue
            }
            return false
        }
        return false
    }
}

/// Vor dem asynchronen Zustellen reservieren, damit ein Protokoll nur einmal meldet.
public struct MeetingReadyNotifications: Sendable {
    private var claimed: Set<UUID> = []
    public init() {}
    public mutating func claim(_ document: UUID) -> Bool { claimed.insert(document).inserted }
}

/// Fester Umfang je Einreichung; neue Dokumente gehören zur nächsten Fortsetzung.
public struct MeetingQueueProgress: Sendable {
    private var units: [UUID: Int64]
    public private(set) var successful = true
    public var total: Int64 { Int64(units.count) * 1000 }
    public var completed: Int64 { units.values.reduce(0, +) }
    public init(documents: [UUID]) {
        units = Dictionary(documents.map { ($0, Int64(0)) }, uniquingKeysWith: max)
    }
    public func contains(_ document: UUID) -> Bool { units[document] != nil }
    public func next(in documents: [UUID]) -> UUID? {
        documents.first(where: contains) ?? documents.first
    }
    public mutating func update(_ document: UUID, units value: Int64) {
        guard let previous = units[document] else { return }
        units[document] = max(previous, min(999, max(0, value)))
    }
    public mutating func complete(_ document: UUID, success: Bool = true) {
        guard units[document] != nil else { return }
        units[document] = 1000
        successful = successful && success
    }
}
