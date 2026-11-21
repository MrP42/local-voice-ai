import Foundation
import NaturalLanguage
import Translation
import CryptoKit

// Live-Übersetzung beim Vorlesen (lva#358, Spez §3): Apples geräteinterne Übersetzung hinter `LiveTranslator`.
// Der Buchinhalt wird auf dem iPhone übersetzt; Apple kann Nutzungs- und Leistungsmetriken ohne Textinhalt erheben.

/// Stand eines Sprachpaars auf diesem iPhone (Spez §3, getrennte Zustände).
enum LiveLanguageState: Equatable {
    case installed
    /// Paket fehlt, kann geladen werden (Systemzustimmung, Netz nötig).
    case downloadable
    case unsupported
    /// Noch nicht geprüft oder Prüfung fehlgeschlagen.
    case unknown
}

enum LiveTranslationSupport {
    /// `zh-hans` → Locale.Language „zh-Hans“ (Schrift großgeschrieben, wie Apple sie erwartet).
    static func language(_ code: String) -> Locale.Language {
        let parts = LiveTranslation.base(code).split(separator: "-").map(String.init)
        guard let first = parts.first else { return Locale.Language(identifier: code) }
        if parts.count > 1 { return Locale.Language(identifier: first + "-" + parts[1].prefix(1).uppercased() + parts[1].dropFirst()) }
        return Locale.Language(identifier: first)
    }

    /// Ob Apple das Paar übersetzen kann und ob das Paket schon auf dem Gerät liegt.
    static func state(from source: String, to target: String) async -> LiveLanguageState {
        guard #available(iOS 26.0, *) else { return .unsupported }
        let status = await LanguageAvailability().status(from: language(source), to: language(target))
        switch status {
        case .installed: return .installed
        case .supported: return .downloadable
        case .unsupported: return .unsupported
        @unknown default: return .unknown
        }
    }

    /// Quellsprache: Angabe am Buch, sonst aus den ersten Absätzen erkannt.
    static func sourceLanguage(declared: String?, text: String) -> String? {
        if let declared, !declared.isEmpty { return LiveTranslation.base(declared) }
        let recognizer = NLLanguageRecognizer()
        recognizer.processString(String(text.prefix(4000)))
        return recognizer.dominantLanguage.map { LiveTranslation.base($0.rawValue) }
    }

    /// Quellhash nach Hashvertrag (Spez §7.3): normalisierter Text (`LiveTranslation.normalized`), SHA-256 hex.
    /// Der Player bildet seine Offsets am selben normalisierten Text.
    static func sourceHash(_ text: String) -> String {
        let t = LiveTranslation.hashInput(text)
        return SHA256.hash(data: Data(t.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}

/// Apple-Übersetzung für eine Sitzung (ein Sprachpaar). Ohne View, nur für installierte Pakete (iOS 26).
@available(iOS 26.0, *)
final class AppleLiveTranslator: LiveTranslator, @unchecked Sendable {
    private let session: TranslationSession

    init(source: String, target: String) {
        session = TranslationSession(installedSource: LiveTranslationSupport.language(source),
                                     target: LiveTranslationSupport.language(target))
    }

    func translate(_ texts: [String], from source: String, to target: String) async throws -> [String] {
        let requests = texts.enumerated().map { TranslationSession.Request(sourceText: $1, clientIdentifier: String($0)) }
        let responses = try await session.translations(from: requests)
        // Fehlende oder überzählige Antworten meldet die Pipeline als falsche Anzahl.
        guard responses.count == texts.count else { return responses.map(\.targetText) }
        var out = responses.map(\.targetText)
        // Zuordnung über die Kennung, nicht über die Reihenfolge.
        for r in responses { if let id = r.clientIdentifier, let i = Int(id), out.indices.contains(i) { out[i] = r.targetText } }
        return out
    }
}
