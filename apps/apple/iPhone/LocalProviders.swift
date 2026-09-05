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
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "de-DE")) else { return try await cpuTranscribe(url) }
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        // Never silently use server recognition or start an asset download.
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { return try await cpuTranscribe(url) }
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
        return try await withTaskCancellationHandler {
            do {
                try Task.checkCancellation()
                let file = try AVAudioFile(forReading: url)
                try await analyzer.start(inputAudioFile: file, finishAfterFile: true)
                let text = try await results.value
                try Task.checkCancellation()
                guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw VoiceError.invalid }
                return text
            } catch { results.cancel(); await analyzer.cancelAndFinishNow(); throw error }
        } onCancel: {
            results.cancel()
            Task { await analyzer.cancelAndFinishNow() }
        }
    }

    static func reply(to transcript: String) async throws -> String {
        guard SystemLanguageModel.default.availability == .available else { return try await cpuReply(transcript) }
        let session = LanguageModelSession(instructions: "Antworte kurz auf Deutsch, höchstens zwei Sätze. Führe keine externen Aktionen aus.")
        let generation = Task {
            let result = try await session.respond(to: String(transcript.prefix(2000)), options: GenerationOptions(maximumResponseTokens: 128))
            try Task.checkCancellation()
            return String(result.content.prefix(500))
        }
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(60)) } catch { return }
            generation.cancel()
        }
        defer { deadline.cancel() }
        return try await withTaskCancellationHandler {
            try await generation.value
        } onCancel: { generation.cancel() }
    }
    private static func cpuTranscribe(_ url: URL) async throws -> String {
        let cancellation = InferenceCancellation()
        return try await withTaskCancellationHandler {
            try await CPULocalProviders.shared.transcribe(url, cancellation: cancellation)
        } onCancel: { cancellation.cancel() }
    }
    private static func cpuReply(_ text: String) async throws -> String {
        let cancellation = InferenceCancellation()
        return try await withTaskCancellationHandler {
            try await CPULocalProviders.shared.reply(to: text, cancellation: cancellation)
        } onCancel: { cancellation.cancel() }
    }

}
