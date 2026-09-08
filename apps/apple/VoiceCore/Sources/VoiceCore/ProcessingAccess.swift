/// Execution permission is separate from whether the app has a visible scene.
public struct ProcessingAccess: Sendable {
    public var foreground = false
    public var watchLease = false
    public var scheduledTask = false
    public var allowed: Bool { foreground || watchLease || scheduledTask }
    public init() {}
}
