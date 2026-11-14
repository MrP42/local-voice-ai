/// Unabhängige Gründe, das Display wach zu halten. Entfernen betrifft nur den eigenen Grund.
public struct IdleReasons: Sendable, Equatable {
    public enum Reason: Hashable, Sendable {
        case reader
        case liveMeeting
    }

    private var reasons: Set<Reason> = []

    public init() {}

    public mutating func insert(_ reason: Reason) {
        reasons.insert(reason)
    }

    public mutating func remove(_ reason: Reason) {
        reasons.remove(reason)
    }

    public var isEmpty: Bool { reasons.isEmpty }
    public var needsAwake: Bool { !isEmpty }
}

/// Nur der sichtbare Vollbild-Player hält beim Lesen oder Abspielen das Display wach.
public func readerReasonActive(
    presented: Bool,
    sceneActive: Bool,
    surface: PlayerSurface,
    playing: Bool
) -> Bool {
    presented && sceneActive && (surface == .read || surface == .readListen || playing)
}
