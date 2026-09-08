import Foundation

/// Distinguish a provider deadline from cancellation requested by its caller.
public final class ProcessingDeadline: @unchecked Sendable {
    private let lock = NSLock()
    private var expired = false
    public init() {}
    public func expire() { lock.lock(); expired = true; lock.unlock() }
    public func check(cancelled: Bool = false) throws {
        if cancelled { throw CancellationError() }
        lock.lock(); let timedOut = expired; lock.unlock()
        if timedOut { throw ProcessingFailure.timedOut }
    }
}
