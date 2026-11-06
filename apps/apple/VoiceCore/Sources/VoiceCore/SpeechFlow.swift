import Foundation

/// Sprachfluss beim Vorlesen (#314, Runde 2): wie lang die Pausen an Komma, Satzende und
/// Absatzende sind. `value` 0 ist zügig, 0,5 Hörbuch-Standard, 1 ruhig; dazwischen stufenlos.
public struct SpeechFlow: Equatable, Sendable {
    public static let brisk = 0.0
    public static let audiobook = 0.5
    public static let calm = 1.0
    public static let storageKey = "reader.flow"

    public let value: Double

    public init(_ value: Double) { self.value = value.isFinite ? min(max(value, 0), 1) : Self.audiobook }

    /// Gespeicherte Wahl, sonst Hörbuch-Standard.
    public static func stored(_ defaults: UserDefaults = .standard) -> SpeechFlow {
        SpeechFlow(defaults.object(forKey: storageKey) as? Double ?? audiobook)
    }

    /// Pausen in ms: Komma, zwischen Sätzen, Absatzende (zügig / Hörbuch / ruhig).
    public var commaMs: Double { lerp(120, 200, 320) }
    public var sentenceMs: Double { lerp(170, 260, 420) }
    public var paragraphMs: Double { lerp(380, 600, 900) }

    public var title: String {
        value < 0.25 ? "Zügig" : value <= 0.75 ? "Hörbuch" : "Ruhig"
    }

    private func lerp(_ a: Double, _ b: Double, _ c: Double) -> Double {
        value <= 0.5 ? a + (b - a) * value * 2 : b + (c - b) * (value - 0.5) * 2
    }

    /// Satzpause = Nachklang dieses Satzes + Vorlauf des nächsten.
    public func shaper(sampleRate: Double) -> SentenceShaper {
        let lead = 30.0
        return SentenceShaper(sampleRate: sampleRate, leadMs: lead, tailMs: max(sentenceMs - lead, 40),
                              paragraphMs: paragraphMs, innerMs: commaMs)
    }
}
