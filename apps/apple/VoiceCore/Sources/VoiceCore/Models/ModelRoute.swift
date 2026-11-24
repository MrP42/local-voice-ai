import Foundation

// Gemeinsame Modellwahl (lva#801 T4, lva#804). Reine Typen ohne Apple-Frameworks.
// Schnittstelle festgelegt in https://github.com/MrP42/local-voice-ai/issues/801 (Kommentar „Gemeinsame Modellwahl“).
// Änderungen an den Typen nur nach Kommentar dort.

/// Wo ein Modell rechnet.
public enum ModelRoute: Codable, Hashable, Sendable {
    /// Auf dem Gerät, z. B. `qwen3-4b-instruct-2507-q4_k_m.gguf` oder `apple.foundation`.
    case local(modelId: String)
    /// Eigener oder API-Endpunkt aus den Endpunkt-Einstellungen (Schlüssel nur im Schlüsselbund).
    case endpoint(id: UUID)
    /// Über den gekoppelten PC; `provider` meldet der PC (z. B. `claude-cli`, `codex-cli`, `ollama`).
    case pcRelay(provider: String, model: String)

    /// Stabiler, eindeutiger Schlüssel für Einstellungen und Nachweise. Freie Teile sind
    /// prozentkodiert, damit `/`, `:` und `%` in Anbieter- oder Modellnamen nie zwei Routen gleich machen.
    public var key: String {
        switch self {
        case .local(let id): return "local:\(Self.encode(id))"
        case .endpoint(let id): return "endpoint:\(id.uuidString.lowercased())"
        case .pcRelay(let provider, let model): return "relay:\(Self.encode(provider))/\(Self.encode(model))"
        }
    }

    private static let keySafe = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")

    static func encode(_ part: String) -> String {
        part.addingPercentEncoding(withAllowedCharacters: keySafe) ?? part
    }

    public var isLocal: Bool { if case .local = self { return true }; return false }
}

/// Wofür ein Modell gerufen wird. Bestimmt Grenzen und den Zustimmungstext.
public enum ModelPurpose: String, Codable, CaseIterable, Sendable {
    case meetingMinutes, meetingSummary, liveState, talk, textPrep

    /// Obergrenze der Antwortlänge je Aufruf.
    public var maxOutputTokens: Int {
        switch self {
        case .meetingMinutes: return 4000
        case .meetingSummary: return 1500
        case .liveState: return 600
        case .talk: return 800
        case .textPrep: return 2000
        }
    }

    /// Live-Auswertung während der Aufnahme bleibt immer auf dem Gerät.
    public var localOnly: Bool { self == .liveState }
}

/// Ein wählbares Modell, wie es Katalog und Auswahl zeigen.
public struct ModelChoice: Codable, Hashable, Sendable, Identifiable {
    public let route: ModelRoute
    public let label: String
    /// Anbieter für Zustimmung und Kundenprofil, z. B. „OpenAI“, „Anthropic“; `nil` = eigenes Gerät.
    public let vendor: String?
    /// Nur Anzeige (Gruppe „Im eigenen Netz“). Wird nie geprüft: die Prüfung löst den Endpunkt selbst auf.
    public let ownNetworkHint: Bool
    public let contextTokens: Int
    public let supportsStreaming: Bool
    /// Ort der Verarbeitung für den Zustimmungstext, z. B. „USA“, „EU“.
    public let region: String?

    public var id: String { route.key }
    /// Daten verlassen das Gerät. Abgeleitet aus der Route, nie frei gesetzt (Endpunkt und PC-Relay zählen immer).
    public var isExternal: Bool { !route.isLocal }

    public init(route: ModelRoute, label: String, vendor: String? = nil, ownNetworkHint: Bool = false, contextTokens: Int,
                supportsStreaming: Bool = true, region: String? = nil) {
        self.route = route; self.label = label; self.vendor = vendor; self.ownNetworkHint = ownNetworkHint
        self.contextTokens = contextTokens; self.supportsStreaming = supportsStreaming; self.region = region
    }
}
