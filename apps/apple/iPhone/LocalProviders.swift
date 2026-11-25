import Foundation
import Speech
import AVFoundation
import FoundationModels

enum LocalProviders {
    static func sttLanguage() throws -> String {
        let defaults = UserDefaults.standard
        let language = defaults.string(forKey: "sttLanguage") ?? "auto"
        if language == "auto" { return language }
        // Locale-Kennungen ausdrücklich auf den Whisper-Sprachcode abbilden.
        let code = language.components(separatedBy: CharacterSet(charactersIn: "-_")).first?.lowercased() ?? ""
        guard !code.isEmpty, code != "auto", lv_whisper_language_valid(code) != 0 else {
            throw LiveCapture.CaptureError.invalidLanguage
        }
        return code
    }

    /// Dieselbe Sprachauswahl für Live-Erkennung, Verfügbarkeitsprüfung und Installation.
    static func speechLocale() async throws -> Locale? {
        let language = try sttLanguage()
        let requested = language == "auto" ? Locale.current : Locale(identifier: language)
        return await SpeechTranscriber.supportedLocale(equivalentTo: requested)
    }

    static func capabilities() async -> [String: String] {
        var result = ["speechTranscriberAvailable": String(SpeechTranscriber.isAvailable), "foundationModels": String(describing: SystemLanguageModel.default.availability)]
        do {
            if let locale = try await speechLocale() {
                result["germanAssets"] = String(describing: await AssetInventory.status(forModules: [SpeechTranscriber(locale: locale, preset: .transcription)]))
            } else { result["germanAssets"] = "unsupported" }
        } catch { result["germanAssets"] = "invalid-language" }
        return result
    }

    static func installSpeech() async throws {
        let locale = try await speechLocale()
        guard SpeechTranscriber.isAvailable, let locale else { throw VoiceError.missing }
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) { try await request.downloadAndInstall() }
    }
    static func transcribe(_ url: URL) async throws -> String { try await annotatedTranscribe(url).text }
    static func annotatedTranscribe(_ url: URL) async throws -> ProcessingOutput {
        let locale = try await speechLocale()
        guard SpeechTranscriber.isAvailable, let locale else {
            return try await cpuTranscribeOutput(url)
        }
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        // Never silently use server recognition or start an asset download.
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { return try await cpuTranscribeOutput(url) }
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let results = Task {
            var text = ""
            for try await result in transcriber.results { text += String(result.text.characters) }
            try Task.checkCancellation()
            return text
        }
        let deadlineState = ProcessingDeadline()
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(60)) } catch { return }
            deadlineState.expire(); results.cancel()
            await analyzer.cancelAndFinishNow()
        }
        defer { deadline.cancel() }
        let text = try await withTaskCancellationHandler {
            do {
                try Task.checkCancellation()
                let file = try AVAudioFile(forReading: url)
                try await analyzer.start(inputAudioFile: file, finishAfterFile: true)
                let text = try await results.value
                try deadlineState.check(cancelled: Task.isCancelled)
                guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw ProcessingFailure.noSpeech }
                return text
            } catch {
                results.cancel(); await analyzer.cancelAndFinishNow()
                try deadlineState.check(cancelled: Task.isCancelled); throw error
            }
        } onCancel: {
            results.cancel()
            Task { await analyzer.cancelAndFinishNow() }
        }
        return ProcessingOutput(text: text, model: "Apple SpeechTranscriber (\(locale.identifier))")
    }

    static func reply(to transcript: String) async throws -> String { try await annotatedReply(to: transcript).text }
    static func annotatedReply(to transcript: String, history: [ConversationTurn] = []) async throws -> ProcessingOutput {
        if ResponsePolicy.isActionRequest(transcript) { return ProcessingOutput(text: ResponsePolicy.capabilityReply, model: "Lokale Funktionsregel", isAI: false) }
        guard SystemLanguageModel.default.availability == .available else { return ProcessingOutput(text: ResponsePolicy.safeAnswer(try await cpuReply(transcript, history: history)), model: "Qwen 2.5 0.5B Instruct · Q4_K_M") }
        let session = LanguageModelSession(instructions: "Antworte kurz auf Deutsch, höchstens zwei Sätze. Du kannst nur Text antworten und Notizen speichern. Behaupte niemals, externe Aktionen ausgeführt zu haben.")
        let previous = history.map { "Nutzer: " + $0.user + "\nAssistent: " + $0.assistant }.joined(separator: "\n")
        let prompt = previous.isEmpty ? String(transcript.prefix(2000)) : "Bisheriges Gespräch (nur Kontext):\n" + previous + "\nAktuelle Nachricht:\n" + String(transcript.prefix(2000))
        let generation = Task {
            let result = try await session.respond(to: prompt, options: GenerationOptions(maximumResponseTokens: 128))
            try Task.checkCancellation()
            return String(result.content.prefix(500))
        }
        let deadlineState = ProcessingDeadline()
        let deadline = Task {
            do { try await Task.sleep(for: .seconds(60)) } catch { return }
            deadlineState.expire(); generation.cancel()
        }
        defer { deadline.cancel() }
        let answer = try await withTaskCancellationHandler {
            do {
                let text = try await generation.value
                try deadlineState.check(cancelled: Task.isCancelled); return text
            } catch {
                try deadlineState.check(cancelled: Task.isCancelled); throw error
            }
        } onCancel: { generation.cancel() }
        return ProcessingOutput(text: ResponsePolicy.safeAnswer(answer), model: "Apple Foundation Models · Systemmodell")
    }
    private static func cpuTranscribeOutput(_ url: URL) async throws -> ProcessingOutput {
        let cancellation = InferenceCancellation()
        return try await withTaskCancellationHandler {
            try await CPULocalProviders.shared.annotatedTranscribe(url, cancellation: cancellation)
        } onCancel: { cancellation.cancel() }
    }
    private static func cpuReply(_ text: String, history: [ConversationTurn] = []) async throws -> String {
        let cancellation = InferenceCancellation()
        return try await withTaskCancellationHandler {
            try await CPULocalProviders.shared.reply(to: text, cancellation: cancellation, history: history)
        } onCancel: { cancellation.cancel() }
    }

}
