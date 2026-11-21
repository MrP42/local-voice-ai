import Foundation

/// Prüfung und kurze, synchrone Übergabe teilen die Sperre mit allen Zustandswechseln.
/// `work` darf weder ins Gate zurückrufen noch auf einen Zustandswechsel warten.
final class AssistantMuteGate: @unchecked Sendable {
    // Der Tap schützt mit derselben Sperre auch seine Ströme und Zeitachse.
    // Diese Zustandswerte dürfen nur unter `lock` gelesen werden.
    let lock = NSLock()
    private(set) var muted = false
    private(set) var live = false
    private(set) var generation = 0

    func configure(muted: Bool, _ work: () -> Void = {}) {
        lock.lock(); defer { lock.unlock() }
        generation &+= 1
        live = false
        self.muted = muted
        work()
    }

    func setMuted(_ value: Bool, _ work: () -> Void = {}) {
        lock.lock(); defer { lock.unlock() }
        if muted != value { generation &+= 1 }
        muted = value
        work()
    }

    func setLive(_ value: Bool) {
        lock.lock(); defer { lock.unlock() }
        if live != value { generation &+= 1 }
        live = value
    }

    /// Merker vor langsamer Vorbereitung; nach jedem Zustandswechsel gilt er nicht mehr.
    func snapshot() -> Int? {
        lock.lock(); defer { lock.unlock() }
        return live && !muted ? generation : nil
    }

    @discardableResult
    func deliver(generation expected: Int? = nil, _ work: () -> Void) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard live, !muted else { return false }
        if let expected, expected != generation { return false }
        work()
        return true
    }
}
