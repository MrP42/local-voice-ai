import Foundation

/// Complications navigate only; a URL never starts recording or deletes data.
public enum WatchDestination: String, Hashable, Sendable {
    case speak, history, latest

    public var url: URL { URL(string: "localvoice-watch://" + rawValue)! }

    public init?(url: URL) {
        guard url.scheme == "localvoice-watch", url.user == nil, url.password == nil,
              url.port == nil, url.path.isEmpty, url.query == nil, url.fragment == nil,
              let host = url.host, let value = Self(rawValue: host) else { return nil }
        self = value
    }
}
