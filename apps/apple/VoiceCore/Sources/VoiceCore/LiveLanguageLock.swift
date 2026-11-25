// nach Yogurt (MIT), FrigadeHQ — FrigadeHQ/yogurt @76e5595
import Foundation

/// Zustandsautomat; Wiederholungen führt der Aufrufer seriell aus.
public struct LiveLanguageLock {
    public static func equivalentLocale(_ current: String, _ requested: String, explicitRegion: Bool = false) -> Bool {
        let current = Locale(identifier: current), requested = Locale(identifier: requested)
        guard current.language.languageCode == requested.language.languageCode else { return false }
        return !explicitRegion || current.region == requested.region
    }

    private struct Track { var language: String; var candidate: String?; var consecutive = 0 }
    private var tracks: [String: Track] = [:]
    public let fixed: String?
    public init(fixed: String? = nil) { self.fixed = fixed == "auto" ? nil : fixed }
    public func language(track: String) -> String { fixed ?? tracks[track]?.language ?? "auto" }

    public func retry(track: String, language: String, prob: Double, text: String) -> String? {
        guard fixed == nil, let had = tracks[track]?.language else { return nil }
        if LiveHallucination.isGarbled(text) { return "auto" }
        if language != "auto", language != had, prob >= 0.9, LiveHallucination.tokens(text).count >= 2 { return language }
        return nil
    }

    /// Nur die endgültig gelesene Antwort zählt. Leere Wiederholungen erhalten die Sperre.
    public mutating func accept(track: String, language: String, prob: Double, text: String) {
        guard fixed == nil else { return }
        let valid = language != "auto" && !LiveHallucination.isGarbled(text) && LiveHallucination.tokens(text).count >= 2
        guard var state = tracks[track] else {
            if valid && prob >= 0.8 { tracks[track] = Track(language: language) }
            return
        }
        if valid && language != state.language && prob >= 0.9 {
            state.consecutive = state.candidate == language ? state.consecutive + 1 : 1
            state.candidate = language
            if state.consecutive >= 2 { state.language = language; state.candidate = nil; state.consecutive = 0 }
        } else {
            state.candidate = nil; state.consecutive = 0
        }
        tracks[track] = state
    }
}
