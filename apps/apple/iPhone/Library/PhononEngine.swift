import Foundation

/// Phonon-Stimmen fürs Vorlesen (Neural Engine, alles auf dem Gerät). Das Paket liegt
/// nur als arm64 vor, daher gibt es Phonon nur im Gerätebau (`PHONON`), nicht im
/// Simulator auf Intel-Macs. Die deutschen Gewichte sind CC BY-NC: Sie stehen nicht im
/// Repo und nicht in der App, sondern werden einmalig nach `Documents/PhononDE` gelegt
/// und von dort nach Application Support übernommen.
@MainActor
final class PhononEngine: ObservableObject {
    static let shared = PhononEngine()
    static let voicePrefix = "phonon:"

    @Published private(set) var voices: [String] = []
    @Published private(set) var loading = false

    static func voiceName(_ identifier: String?) -> String? {
        guard let identifier, identifier.hasPrefix(voicePrefix) else { return nil }
        return String(identifier.dropFirst(voicePrefix.count))
    }

    #if PHONON
    private var tts: Phonon?
    private var player: PhononPlayer?

    private var droppedFolder: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("PhononDE", isDirectory: true)
    }

    var modelAvailable: Bool {
        PhononModels.isInstalled() || FileManager.default.fileExists(atPath: droppedFolder.appendingPathComponent("bundle.json").path)
    }

    /// Modell laden; beim ersten Mal übersetzt Core ML es für dieses Gerät (einige Sekunden).
    func prepare() async throws {
        guard tts == nil else { return }
        loading = true; defer { loading = false }
        let dropped = droppedFolder
        let models = try await Task.detached(priority: .userInitiated) { () throws -> URL in
            if FileManager.default.fileExists(atPath: dropped.appendingPathComponent("bundle.json").path) {
                let dir = try PhononModels.install(bundled: dropped)
                try? FileManager.default.removeItem(at: dropped)  // nicht doppelt belegen
                return dir
            }
            return PhononModels.defaultDirectory
        }.value
        let loaded = try await Phonon.load(models: models, language: .german)
        tts = loaded
        player = try PhononPlayer(configureSession: false)
        voices = loaded.voices.sorted()
    }

    /// Text sprechen und zurückkehren, wenn er zu Ende gespielt ist. Abbruch über die Task.
    func speak(_ text: String, voice: String) async throws {
        try await prepare()
        guard let tts, let player else { return }
        if tts.voice != voice { try await tts.setVoice(voice) }
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
    func prepare() async throws {}
    func speak(_ text: String, voice: String) async throws {}
    func silence() {}
    #endif
}
