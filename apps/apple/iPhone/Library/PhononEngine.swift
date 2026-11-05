import Foundation

/// Phonon-Stimmen fürs Vorlesen (Neural Engine, alles auf dem Gerät). Das Paket liegt
/// nur als arm64 vor, daher gibt es Phonon nur im Gerätebau (`PHONON`), nicht im
/// Simulator auf Intel-Macs.
///
/// Je Sprache ein eigenes Modell. Die Modelle stehen nicht im Repo und nicht in der App:
/// Die deutschen Gewichte sind CC BY-NC. Sie werden einmalig nach `Documents/PhononDE`
/// bzw. `Documents/PhononEN` gelegt und von dort nach Application Support übernommen.
@MainActor
final class PhononEngine: ObservableObject {
    static let shared = PhononEngine()
    static let voicePrefix = "phonon:"
    static let languages = ["de", "en"]

    /// Stimmen je Sprache, aus der Beschreibung der Modelle gelesen.
    @Published private(set) var voices: [String: [String]] = [:]
    @Published private(set) var loading = false

    /// „phonon:de:Nele“ → „de:Nele“; alles andere ist keine Phonon-Stimme.
    static func voiceName(_ identifier: String?) -> String? {
        guard let identifier, identifier.hasPrefix(voicePrefix) else { return nil }
        let rest = String(identifier.dropFirst(voicePrefix.count))
        return rest.contains(":") ? rest : nil
    }

    static func identifier(language: String, voice: String) -> String { "\(voicePrefix)\(language):\(voice)" }

    static func parts(_ name: String) -> (language: String, voice: String)? {
        guard let i = name.firstIndex(of: ":") else { return nil }
        return (String(name[..<i]), String(name[name.index(after: i)...]))
    }

    static func displayName(_ name: String) -> String { parts(name)?.voice ?? name }

    static func sectionTitle(_ language: String) -> String { language == "de" ? "Phonon Deutsch" : "Phonon Englisch" }

    #if PHONON
    private var loaded: [String: Phonon] = [:]
    private var player: PhononPlayer?

    private func dropped(_ language: String) -> URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Phonon\(language.uppercased())", isDirectory: true)
    }

    private func installed(_ language: String) -> URL {
        PhononModels.defaultDirectory.deletingLastPathComponent().appendingPathComponent("models-\(language)", isDirectory: true)
    }

    nonisolated private static func voiceList(_ dir: URL) -> [String]? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("bundle.json")),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return json["voices"] as? [String]
    }

    var modelAvailable: Bool { !voices.isEmpty }

    /// Modelle übernehmen und ihre Stimmen zeigen; geladen wird erst beim Sprechen.
    func refresh() async {
        var found: [String: [String]] = [:]
        for language in Self.languages {
            let source = dropped(language), target = installed(language)
            let list = await Task.detached(priority: .userInitiated) { () -> [String]? in
                if Self.voiceList(source) != nil, (try? PhononModels.install(bundled: source, to: target)) != nil {
                    try? FileManager.default.removeItem(at: source)  // nicht doppelt belegen
                }
                return Self.voiceList(target)
            }.value
            // „default“ ist bei mehreren Stimmen nur ein Doppel einer davon.
            if let list { found[language] = list.count > 1 ? list.filter { $0 != "default" }.sorted() : list }
        }
        voices = found
    }

    /// Beim ersten Mal übersetzt Core ML das Modell für dieses Gerät (einige Sekunden).
    private func engine(_ language: String) async throws -> Phonon {
        if let tts = loaded[language] { return tts }
        loading = true; defer { loading = false }
        if voices.isEmpty { await refresh() }
        let tts = try await Phonon.load(models: installed(language), language: language == "de" ? .german : .english)
        loaded[language] = tts
        if player == nil { player = try PhononPlayer(configureSession: false) }
        return tts
    }

    /// Text sprechen und zurückkehren, wenn er zu Ende gespielt ist. Abbruch über die Task.
    func speak(_ text: String, voice name: String) async throws {
        guard let parts = Self.parts(name) else { throw CancellationError() }
        let tts = try await engine(parts.language)
        guard let player else { return }
        if tts.voice != parts.voice { try await tts.setVoice(parts.voice) }
        let started = Date()
        var samples = 0
        for try await chunk in tts.stream(text) {
            try Task.checkCancellation()
            player.enqueue(chunk)
            samples += chunk.count
        }
        // Erzeugt wird schneller als gespielt: auf das Ende der Wiedergabe warten.
        let rest = Double(samples) / Phonon.sampleRate - Date().timeIntervalSince(started)
        if rest > 0 { try await Task.sleep(nanoseconds: UInt64(rest * 1_000_000_000)) }
    }

    func silence() { player?.stop() }
    #else
    var modelAvailable: Bool { false }
    func refresh() async {}
    func speak(_ text: String, voice name: String) async throws {}
    func silence() {}
    #endif
}
