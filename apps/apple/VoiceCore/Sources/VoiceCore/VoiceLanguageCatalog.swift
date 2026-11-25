import Foundation

/// Sprachgruppen ohne Abhängigkeit von den Stimmen des Betriebssystems.
public enum VoiceLanguageGrouping {
    public struct Group<Voice> {
        public let code: String
        public let name: String
        public let voices: [Voice]
    }

    public static func group<Voice>(
        voices: [Voice], readingLanguage: String?, locale: Locale,
        language: (Voice) -> String
    ) -> [Group<Voice>] {
        let reading = readingLanguage.map(LiveTranslation.base)
        let grouped = Dictionary(grouping: voices) { LiveTranslation.base(language($0)) }
        return grouped.keys.filter { !$0.isEmpty && $0 != reading }.map { code in
            Group(code: code, name: locale.localizedString(forIdentifier: code) ?? code,
                  voices: grouped[code] ?? [])
        }.sorted {
            let order = $0.name.compare($1.name, locale: locale)
            return order == .orderedSame ? $0.code < $1.code : order == .orderedAscending
        }
    }
}

/// Geräteangebot normalisieren; die häufigen Sprachen behalten ihre bisherige Reihenfolge.
public enum LiveLanguageCatalog {
    /// Wie `LiveTranslation.base`, trennt aber Chinesisch nach Schrift (lva#816): Apple meldet „zh“ und „zh-TW“
    /// ohne Schriftangabe; ohne diese Zuordnung fielen beide auf ein einziges „Chinesisch“ zusammen.
    public static func liveCode(_ raw: String) -> String {
        let base = LiveTranslation.base(raw)
        guard base == "zh" else { return base }
        let parts = raw.lowercased().split(whereSeparator: { $0 == "-" || $0 == "_" }).dropFirst()
        return parts.contains { ["tw", "hk", "mo"].contains($0) } ? "zh-hant" : "zh-hans"
    }

    public static func offered(
        supportedCodes: [String], locale: Locale
    ) -> [(code: String, name: String)] {
        guard !supportedCodes.isEmpty else { return LiveLanguages.offered }
        let codes = Set(supportedCodes.map(liveCode).filter { !$0.isEmpty })
        let common = LiveLanguages.offered.filter { codes.contains($0.code) }
        let commonCodes = Set(common.map(\.code))
        let rest = codes.subtracting(commonCodes).map { code in
            (code: code, name: locale.localizedString(forIdentifier: code) ?? code)
        }.sorted {
            let order = $0.name.compare($1.name, locale: locale)
            return order == .orderedSame ? $0.code < $1.code : order == .orderedAscending
        }
        return common + rest
    }
}

/// Ausschließlich feste Beispielsätze, niemals Text aus einem Buch.
public enum SampleSentences {
    public static func sentence(for language: String) -> String {
        let code = LiveTranslation.base(language).split(separator: "-").first.map(String.init)
        switch code {
        case "de": return "Guten Tag. So klingt diese Stimme beim Vorlesen."
        case "fr": return "Bonjour. Voici ma voix pour la lecture."
        case "es": return "Hola. Así suena esta voz al leer."
        case "it": return "Buongiorno. Questa è la mia voce per la lettura."
        case "pt": return "Olá. Esta é a minha voz de leitura."
        case "nl": return "Hallo. Zo klinkt deze stem bij het voorlezen."
        case "pl": return "Dzień dobry. Tak brzmi ten głos podczas czytania."
        case "tr": return "Merhaba. Bu ses okurken böyle duyulur."
        case "uk": return "Добрий день. Ось як звучить цей голос під час читання."
        case "ru": return "Здравствуйте. Так звучит этот голос при чтении."
        case "zh": return "你好。这是朗读时的声音。"
        case "ja": return "こんにちは。この声で読み上げます。"
        case "ko": return "안녕하세요. 이 목소리로 읽어 드립니다."
        case "ar": return "مرحباً. هكذا يبدو هذا الصوت عند القراءة."
        case "hi": return "नमस्ते। पढ़ते समय यह आवाज़ ऐसी सुनाई देती है।"
        case "id": return "Halo. Beginilah suara ini saat membaca."
        case "th": return "สวัสดี นี่คือตัวอย่างเสียงอ่าน"
        case "vi": return "Xin chào. Đây là giọng đọc mẫu."
        case "sv": return "Hej. Så här låter den här rösten vid högläsning."
        default: return "Hello. This is how this voice sounds when reading aloud."
        }
    }
}

/// Neue Proben erhalten eine Marke: ein spätes Ende beendet niemals die nächste Probe.
public struct VoicePreviewState: Equatable, Sendable {
    public struct Active: Equatable, Sendable {
        public let identifier: String
        public let token: Int
    }

    public enum Event: Equatable, Sendable {
        case toggle(String)
        case selected, dismissed
        case finished(Int)
    }

    public private(set) var active: Active?
    private var generation = 0

    public init() {}

    public mutating func handle(_ event: Event) {
        switch event {
        case .toggle(let identifier):
            generation += 1
            active = active?.identifier == identifier ? nil
                : Active(identifier: identifier, token: generation)
        case .selected, .dismissed:
            active = nil
        case .finished(let token):
            if active?.token == token { active = nil }
        }
    }
}
