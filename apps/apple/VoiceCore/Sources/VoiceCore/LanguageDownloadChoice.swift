/// Eine laufende Zustimmung bleibt bestehen; andere Sprachwahlen warten bis zu ihrem Ende.
public struct LanguageDownloadChoice: Sendable {
    public enum Action: Equatable, Sendable {
        case none, download(String), apply(String), unavailable(String)
    }

    public private(set) var loading: String?
    public private(set) var pendingChoice: String?

    public init() {}

    public mutating func choose(
        _ code: String, installed: Bool, downloadable: Bool
    ) -> Action {
        if let loading {
            if code != loading { pendingChoice = code }
            return .none
        }
        if installed { return .apply(code) }
        guard downloadable else { return .none }
        loading = code
        return .download(code)
    }

    /// Erfolg und Fehler lesen denselben frischen Paketstatus; die letzte Vormerkung gilt.
    public mutating func finished(installed: Set<String>) -> Action {
        guard let loading else { return .none }
        let wanted = pendingChoice ?? loading
        close()
        return installed.contains(wanted) ? .apply(wanted) : .unavailable(wanted)
    }

    public mutating func close() {
        loading = nil
        pendingChoice = nil
    }
}
