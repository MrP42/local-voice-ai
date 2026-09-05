import Foundation
import Speech
import AVFoundation
import FoundationModels

enum LocalProviders {
    static func capabilities() async -> [String: String] {
        var result = ["speechTranscriberAvailable": String(SpeechTranscriber.isAvailable), "foundationModels": String(describing: SystemLanguageModel.default.availability)]
        if let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "de-DE")) {
            result["germanAssets"] = String(describing: await AssetInventory.status(forModules: [SpeechTranscriber(locale: locale, preset: .transcription)]))
        } else { result["germanAssets"] = "unsupported" }
        return result
    }

    static func installSpeech() async throws {
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "de-DE")) else { throw VoiceError.missing }
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) { try await request.downloadAndInstall() }
    }
    static func transcribe(_ url: URL) async throws -> String {
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "de-DE")) else { return try await CPULocalProviders.shared.transcribe(url) }
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        // Never silently use server recognition or start an asset download.
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { return try await CPULocalProviders.shared.transcribe(url) }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let results = Task {
            var text = ""
            for try await result in transcriber.results { text += String(result.text.characters) }
            try Task.checkCancellation()
            return text
        }
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(60)) } catch { return }
            results.cancel()
            await analyzer.cancelAndFinishNow()
        }
        defer { deadline.cancel() }
        do {
            let file = try AVAudioFile(forReading: url)
            try await analyzer.start(inputAudioFile: file, finishAfterFile: true)
            let text = try await results.value
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw VoiceError.invalid }
            return text
        } catch { results.cancel(); await analyzer.cancelAndFinishNow(); throw error }
    }
    static func reply(to transcript: String) async throws -> String {
        guard SystemLanguageModel.default.availability == .available else { return try await CPULocalProviders.shared.reply(to: transcript) }
        let session = LanguageModelSession(instructions: "Antworte kurz auf Deutsch, höchstens zwei Sätze. Führe keine externen Aktionen aus.")
        return String(try await session.respond(to: String(transcript.prefix(2000))).content.prefix(500))
    }
}
