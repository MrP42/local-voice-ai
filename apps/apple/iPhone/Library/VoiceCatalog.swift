import Foundation
import AVFoundation
import Combine

/// Nur Werttypen verlassen den Hintergrundauftrag; AVFoundation-Objekte bleiben dort.
struct CatalogVoice: Identifiable, Equatable, Sendable {
    enum Quality: Int, Equatable, Sendable {
        case `default`, enhanced, premium

        var label: String {
            switch self {
            case .default: return "Standard"
            case .enhanced: return "Erweitert"
            case .premium: return "Premium"
            }
        }
    }

    let id: String
    let name: String
    let language: String
    let quality: Quality
    let isPersonal: Bool

    var identifier: String { id }
}

struct VoiceCatalogSnapshot: Equatable, Sendable {
    let voices: [CatalogVoice]

    init(voices: [CatalogVoice] = []) {
        self.voices = voices.sorted { $0.id < $1.id }
    }

    func voice(id: String) -> CatalogVoice? { voices.first { $0.id == id } }

    func voices(forLanguagePrefix prefix: String) -> [CatalogVoice] {
        voices.filter { $0.language.hasPrefix(prefix) && !$0.isPersonal }.sorted {
            if $0.quality != $1.quality { return $0.quality.rawValue > $1.quality.rawValue }
            return ($0.name, $0.id) < ($1.name, $1.id)
        }
    }

    func hasVoice(lang: String) -> Bool {
        voices.contains { $0.language.hasPrefix(lang) }
    }

    var personalVoices: [CatalogVoice] { voices.filter(\.isPersonal) }

    func bestVoice(lang: String) -> CatalogVoice? {
        voices(forLanguagePrefix: lang).first
    }

    /// Eine gespeicherte Wahl bleibt auch dann bestehen, wenn ihre Stimme gerade fehlt.
    func automaticVoice(lang: String, hasOwnChoice: Bool) -> CatalogVoice? {
        hasOwnChoice ? nil : bestVoice(lang: lang)
    }

    func shouldPublish(replacing old: Self) -> Bool { self != old }
}

/// Generationen schützen auch vor einem bereits laufenden, nicht abbrechbaren Systemaufruf.
struct VoiceCatalogDebounce: Sendable {
    private(set) var generation: UInt64 = 0

    mutating func request() -> UInt64 {
        generation &+= 1
        return generation
    }

    func accepts(_ request: UInt64, cancelled: Bool) -> Bool {
        !cancelled && request == generation
    }
}

@MainActor
final class VoiceCatalog: ObservableObject {
    static let shared = VoiceCatalog()
    @Published private(set) var snapshot = VoiceCatalogSnapshot()
    private var debounce = VoiceCatalogDebounce()
    private var refreshTask: Task<Void, Never>?
    private var observation: AnyCancellable?

    init() {
        observation = NotificationCenter.default.publisher(
            for: AVSpeechSynthesizer.availableVoicesDidChangeNotification
        ).sink { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
        refresh()
    }

    func refresh() {
        refreshTask?.cancel()
        let request = debounce.request()
        refreshTask = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 500_000_000) } catch { return }
            guard let self, self.debounce.accepts(request, cancelled: Task.isCancelled) else {
                return
            }
            let loader = Task.detached(priority: .utility) {
                VoiceCatalogSnapshot(voices: AVSpeechSynthesisVoice.speechVoices().map { voice in
                    let quality: CatalogVoice.Quality
                    switch voice.quality {
                    case .premium: quality = .premium
                    case .enhanced: quality = .enhanced
                    default: quality = .default
                    }
                    return CatalogVoice(
                        id: voice.identifier, name: voice.name, language: voice.language,
                        quality: quality, isPersonal: voice.voiceTraits.contains(.isPersonalVoice)
                    )
                })
            }
            let fresh = await loader.value
            guard self.debounce.accepts(request, cancelled: Task.isCancelled) else { return }
            if fresh.shouldPublish(replacing: self.snapshot) { self.snapshot = fresh }
        }
    }
}
