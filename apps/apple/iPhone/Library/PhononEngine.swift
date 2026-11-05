import Foundation

/// Phonon-Stimmen fürs Vorlesen (Neural Engine, alles auf dem Gerät). Das Paket liegt
/// nur als arm64 vor, daher gibt es Phonon nur im Gerätebau (`PHONON`), nicht im
/// Simulator auf Intel-Macs.
///
/// Je Sprache ein eigenes Modell. Die Modelle stehen nicht im Repo und nicht in der App:
/// Die deutschen Gewichte sind CC BY-NC. Sie werden einmalig nach `Documents/PhononDE`
/// bzw. `Documents/PhononEN` gelegt und von dort nach Application Support übernommen.
struct PhononUnavailable: LocalizedError {
    var errorDescription: String? { "Phonon ist nicht verfügbar." }
}

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
    /// Höchstens ein Modell im Speicher; ein laufendes Laden wird geteilt.
    private var loaded: (language: String, tts: Phonon)?
    private var loading_: (language: String, task: Task<Phonon, Error>)?
    private var player: PhononPlayer?
    private var refreshTask: Task<Void, Never>?

    private func dropped(_ language: String) -> URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Phonon\(language.uppercased())", isDirectory: true)
    }

    private func installed(_ language: String) -> URL {
        PhononModels.defaultDirectory.deletingLastPathComponent().appendingPathComponent("models-\(language)", isDirectory: true)
    }

    nonisolated private static func built(_ dir: URL) -> Int? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("bundle.json")),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return (json["built"] as? NSNumber)?.intValue
    }

    nonisolated private static func voiceList(_ dir: URL) -> [String]? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent("bundle.json")),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return json["voices"] as? [String]
    }

    var modelAvailable: Bool { !voices.isEmpty }

    /// Modelle übernehmen und ihre Stimmen zeigen; geladen wird erst beim Sprechen.
    func refresh() async {
        // Gleichzeitige Aufrufe warten auf denselben Lauf, sonst sähen sie noch keine Stimmen.
        if let running = refreshTask { await running.value; return }
        let task = Task { await scanModels() }
        refreshTask = task
        await task.value
        refreshTask = nil
    }

    private func scanModels() async {
        var found: [String: [String]] = [:]
        for language in Self.languages {
            let source = dropped(language), target = installed(language)
            let list = await Task.detached(priority: .userInitiated) { () -> [String]? in
                // Nur eine vollständige, neue Kopie übernehmen, sonst bleibt das Vorhandene samt
                // übersetzten Modellen. `isInstalled` taugt nur für die Quelle: Core ML lässt im
                // Ziel nach dem Übersetzen nicht alle Dateien des Pakets stehen.
                if PhononModels.isInstalled(at: source), Self.built(source) != Self.built(target) {
                    try? PhononModels.install(bundled: source, to: target)
                }
                if Self.built(source) != nil, Self.built(source) == Self.built(target) {
                    try? FileManager.default.removeItem(at: source)  // nicht doppelt belegen
                }
                return Self.voiceList(target)
            }.value
            // „default“ ist bei mehreren Stimmen nur ein Doppel einer davon.
            if let list { found[language] = list.count > 1 ? list.filter { $0 != "default" }.sorted() : list }
        }
        voices = found
        // Vorgabe einmalig: Anton, sobald das Modell da ist; danach gilt die eigene Wahl.
        let defaults = UserDefaults.standard
        if !defaults.bool(forKey: "voice.antonDefault"), found["de"]?.contains("Anton") == true {
            defaults.set(true, forKey: "voice.antonDefault")
            VoiceModel.shared.selectedVoiceID = Self.identifier(language: "de", voice: "Anton")
        }
    }

    /// Beim ersten Mal übersetzt Core ML das Modell für dieses Gerät (einige Sekunden).
    private func engine(_ language: String) async throws -> Phonon {
        if let loaded, loaded.language == language { return loaded.tts }
        if let loading_, loading_.language == language { return try await loading_.task.value }
        if voices.isEmpty { await refresh() }
        let dir = installed(language), lang: Language = language == "de" ? .german : .english
        let task = Task { try await Phonon.load(models: dir, language: lang) }
        loading_ = (language, task)
        loading = true
        defer { loading = false; if loading_?.language == language { loading_ = nil } }
        let tts = try await task.value
        loaded = (language, tts)
        return tts
    }

    /// Text sprechen und zurückkehren, wenn er zu Ende gespielt ist. Abbruch über die Task.
    func speak(_ text: String, voice name: String) async throws {
        guard let parts = Self.parts(name) else { throw PhononUnavailable() }
        let tts = try await engine(parts.language)
        try Task.checkCancellation()
        if tts.voice != parts.voice { try await tts.setVoice(parts.voice) }
        try Task.checkCancellation()
        // Je Äußerung ein neuer Player: Nach Anruf, Aufnahme oder Routenwechsel steht die alte Audio-Engine.
        let player = try PhononPlayer(configureSession: false)
        self.player = player
        var firstChunk: Date?
        var samples = 0
        for try await chunk in tts.stream(text) {
            try Task.checkCancellation()
            player.enqueue(chunk)
            if firstChunk == nil { firstChunk = Date() }
            samples += chunk.count
        }
        // Erzeugt wird schneller als gespielt: auf das Ende der Wiedergabe warten.
        guard let firstChunk else { return }
        let rest = Double(samples) / Phonon.sampleRate - Date().timeIntervalSince(firstChunk) + 0.15
        if rest > 0 { try await Task.sleep(nanoseconds: UInt64(rest * 1_000_000_000)) }
    }

    func silence() { player?.stop(); player = nil }
    #else
    var modelAvailable: Bool { false }
    func refresh() async {}
    func speak(_ text: String, voice name: String) async throws { throw PhononUnavailable() }
    func silence() {}
    #endif
}
