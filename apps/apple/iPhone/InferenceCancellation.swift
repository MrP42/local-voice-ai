import Foundation

/// One token per invocation. Native callbacks read an atomic flag; lifetime covers the awaited call.
final class InferenceCancellation: @unchecked Sendable {
    let pointer: OpaquePointer
    init() { pointer = lv_cancel_create()! }
    func cancel() { lv_cancel_request(pointer) }
    deinit { lv_cancel_destroy(pointer) }
}
