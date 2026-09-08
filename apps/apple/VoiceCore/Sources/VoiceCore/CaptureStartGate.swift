import Foundation

/// A permission callback belongs to one foreground recording intent, never a later scene.
public struct CaptureStartGate {
    public enum Resolution: Equatable { case start, denied, cancelled, stale }
    private var pending: UUID?
    private var valid = false
    public init() {}
    public mutating func begin(active: Bool) -> UUID? {
        guard active, pending == nil else { return nil }
        let token = UUID()
        pending = token
        valid = true
        return token
    }
    public mutating func sceneBecameInactive() { valid = false }
    public mutating func resolve(_ token: UUID, allowed: Bool, active: Bool) -> Resolution {
        guard pending == token else { return .stale }
        pending = nil
        let canStart = valid && active
        valid = false
        guard allowed else { return .denied }
        return canStart ? .start : .cancelled
    }
}
