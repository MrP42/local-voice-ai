import UIKit

/// Gemeinsame Displaysperre: Ein beendeter Grund hebt keinen anderen auf.
@MainActor
final class IdleTimerGuard {
    typealias Reason = IdleReasons.Reason

    static let shared = IdleTimerGuard()

    private var reasons = IdleReasons()

    private init() {}

    func set(_ reason: Reason, active: Bool) {
        if active {
            reasons.insert(reason)
        } else {
            reasons.remove(reason)
        }
        let disabled = !reasons.isEmpty
        if UIApplication.shared.isIdleTimerDisabled != disabled {
            UIApplication.shared.isIdleTimerDisabled = disabled
        }
    }
}
