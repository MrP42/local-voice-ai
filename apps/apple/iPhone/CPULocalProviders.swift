import Foundation
import AVFoundation
import CryptoKit
import UIKit

/// The serial actor keeps inference off the main thread and limits concurrent model memory.
actor CPULocalProviders {
    static let shared = CPULocalProviders()
    private var verifiedModels: [String: String] = [:]
    /// Gemeinsame Größen-/Hashprüfung vor dem Laden eines Protokollmodells.
    func minutesModelURL(_ name: String) throws -> URL { try modelURL(name) }
    /// Derselbe Actor wie Datei-Transkription; die Bridge sperrt zusätzlich prozessweit.
    func openLiveWhisper(cancellation: InferenceCancellation) async throws -> LiveWhisperSession {
        let selected = LocalModel.selectedSTTModel()
        guard ["ggml-base.bin", "ggml-small.bin", LocalModel.turboName].contains(selected) else { throw VoiceError.invalid }
        let model = try modelURL(selected)
        let allowGPU = await MainActor.run { UIApplication.shared.applicationState == .active }
        try Task.checkCancellation()
        guard let context = lv_whisper_load(model.path, allowGPU ? 1 : 0, cancellation.pointer) else { throw VoiceError.invalid }
        return LiveWhisperSession(context)
    }

    func livePrompt(_ session: LiveWhisperSession, names: [String], emails: [String], glossary: String) -> String {
        LiveRefinement.prompt(names: names, emails: emails, glossary: glossary) {
            Int(lv_whisper_count(session.pointer, $0))
        }
    }

    func refine(_ session: LiveWhisperSession, chunk: LiveChunk, language: String,
                prompt: String, guess: Bool, cancellation: InferenceCancellation) throws -> LiveWhisperResult {
        try Task.checkCancellation()
        var output = [CChar](repeating: 0, count: 65536)
        let status = chunk.samples.withUnsafeBufferPointer { samples in
            lv_whisper_run_with_guess(session.pointer, samples.baseAddress, Int32(samples.count),
                                      language, prompt, &output, Int32(output.count), cancellation.pointer, guess ? 1 : 0)
        }
        try Task.checkCancellation()
        guard status == 0 else { throw VoiceError.invalid }
        return try JSONDecoder().decode(LiveWhisperResult.self, from: Data(String(cString: output).utf8))
    }
    private func modelURL(_ name: String) throws -> URL {
        let folder = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Models")
        let url = folder.appendingPathComponent(name)
        guard FileManager.default.fileExists(atPath: url.path) else { throw VoiceError.missing }
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              let descriptor = LocalModel.all.first(where: { $0.name == name }),
              (attributes[.size] as? NSNumber)?.int64Value == descriptor.bytes else { throw VoiceError.invalid }
        let modified = (attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        let fingerprint = "\(attributes[.systemFileNumber] ?? ""):\(attributes[.size] ?? ""):\(modified)"
        if verifiedModels[name] != fingerprint {
            let input = try FileHandle(forReadingFrom: url)
            defer { try? input.close() }
            var hash = SHA256()
            while let data = try input.read(upToCount: 1024 * 1024), !data.isEmpty {
                try Task.checkCancellation(); hash.update(data: data)
            }
            guard hash.finalize().map({ String(format: "%02x", $0) }).joined() == LocalModel.all.first(where: { $0.name == name })?.sha256 else { throw VoiceError.invalid }
            verifiedModels[name] = fingerprint
        }
        return url
    }
    func annotatedTranscribe(
        _ url: URL, cancellation: InferenceCancellation
    ) async throws -> ProcessingOutput {
        let selected = LocalModel.selectedSTTModel()
        return ProcessingOutput(
            text: try await transcription(
                url, cancellation: cancellation, segmented: false, selectedModel: selected
            ),
            model: LocalModel.label(for: selected)
        )
    }
    func transcribe(_ url: URL, cancellation: InferenceCancellation) async throws -> String {
        try await transcription(url, cancellation: cancellation, segmented: false)
    }
    func transcribeSlice(
        _ url: URL, offset: Double, duration: Double, cancellation: InferenceCancellation,
        selectedModel: String? = nil
    ) async throws -> [MeetingSegment] {
        try Task.checkCancellation()
        let file = try AVAudioFile(forReading: url)
        let rate = file.processingFormat.sampleRate
        guard offset.isFinite, offset >= 0, duration > 0, duration <= 30, rate > 0, rate <= 192000, file.processingFormat.channelCount <= 8 else { throw VoiceError.invalid }
        let position = AVAudioFramePosition((offset * rate).rounded())
        if position >= file.length { return [] }
        let count = min(file.length - position, AVAudioFramePosition((duration * rate).rounded()))
        guard count > 0, let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(count)) else { throw VoiceError.invalid }
        file.framePosition = position; try file.read(into: buffer, frameCount: AVAudioFrameCount(count))
        let clip = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".caf")
        defer { try? FileManager.default.removeItem(at: clip) }
        do {
            let output = try AVAudioFile(forWriting: clip, settings: file.processingFormat.settings)
            try output.write(from: buffer)
        }
        do {
            return try await transcribeSegments(
                clip, cancellation: cancellation, selectedModel: selectedModel
            )
        }
        catch ProcessingFailure.noSpeech { return [] }
    }
    func transcribeSegments(
        _ url: URL, cancellation: InferenceCancellation, selectedModel: String? = nil
    ) async throws -> [MeetingSegment] {
        let json = try await transcription(
            url, cancellation: cancellation, segmented: true, selectedModel: selectedModel
        )
        return try JSONDecoder().decode([MeetingSegment].self, from: Data(json.utf8))
    }
    private func transcription(
        _ url: URL, cancellation: InferenceCancellation, segmented: Bool,
        selectedModel: String? = nil
    ) async throws -> String {
        try Task.checkCancellation()
        let selected = selectedModel ?? LocalModel.selectedSTTModel()
        guard ["ggml-base.bin", "ggml-small.bin", LocalModel.turboName].contains(selected) else {
            throw VoiceError.invalid
        }
        let language = try LocalProviders.sttLanguage()
        let modelName = selected
        let file = try AVAudioFile(forReading: url)
        guard file.length > 0, Double(file.length) / file.processingFormat.sampleRate <= 60,
              let input = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)),
              let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false),
              let converter = AVAudioConverter(from: file.processingFormat, to: format) else { throw VoiceError.invalid }
        try file.read(into: input)
        let capacity = AVAudioFrameCount(ceil(Double(input.frameLength) * 16000 / file.processingFormat.sampleRate)) + 1024
        guard let converted = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { throw VoiceError.invalid }
        var supplied = false
        var error: NSError?
        let result = converter.convert(to: converted, error: &error) { _, state in
            if supplied { state.pointee = .endOfStream; return nil }
            supplied = true; state.pointee = .haveData; return input
        }
        guard error == nil, result != .error, let samples = converted.floatChannelData?[0] else { throw VoiceError.invalid }
        switch AudioSignal.assess(UnsafeBufferPointer(start: samples, count: Int(converted.frameLength))) {
        case .empty, .silent, .stationaryNoise: throw ProcessingFailure.noSpeech
        case .invalid: throw VoiceError.invalid
        case .signal: break
        }
        let model = try modelURL(modelName)
        // Erst nach Datei- und Modellprüfung abfragen; UIKit bleibt auf dem Main Actor.
        let allowGPU = await MainActor.run { UIApplication.shared.applicationState == .active }
        try Task.checkCancellation()
        var output = [CChar](repeating: 0, count: segmented ? 65536 : 16384)
        // Lade-, Erkennungs- und Inferenzphase werden erst am nativen Einstieg markiert.
        let code: Int32
        if segmented {
            code = lv_transcribe_segments_with_options(
                model.path, samples, Int32(converted.frameLength), &output,
                Int32(output.count), cancellation.pointer, language, nil, allowGPU ? 1 : 0
            )
        } else {
            code = lv_transcribe_with_options(
                model.path, samples, Int32(converted.frameLength), &output,
                Int32(output.count), cancellation.pointer, language, nil, allowGPU ? 1 : 0
            )
        }
        try Task.checkCancellation()
        if code == 4 { throw ProcessingFailure.noSpeech }
        guard code == 0 else { throw VoiceError.invalid }
        return String(cString: output).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    func reply(to transcript: String, cancellation: InferenceCancellation, history: [ConversationTurn] = []) throws -> String {
        try Task.checkCancellation()
        let model = try modelURL("qwen2.5-0.5b-instruct-q4_k_m.gguf")
        // Qwen2.5 ChatML: role-delimited prior turns from this conversation only.
        let text = String(transcript.prefix(1200)).replacingOccurrences(of: "<|", with: "< |")
        func safe(_ text: String) -> String { text.replacingOccurrences(of: "<|", with: "< |") }
        let previous = history.map { "<|im_start|>user\n" + safe($0.user) + "<|im_end|>\n<|im_start|>assistant\n" + safe($0.assistant) + "<|im_end|>\n" }.joined()
        let prompt = "<|im_start|>system\nDu bist ein hilfreicher Sprachbegleiter. Antworte kurz auf Deutsch in einem Satz. Du kannst nur Text antworten und Notizen speichern. Behaupte niemals, einen Wecker gestellt, Nachrichten gesendet oder andere externe Aktionen ausgeführt zu haben.<|im_end|>\n\(previous)<|im_start|>user\n\(text)<|im_end|>\n<|im_start|>assistant\n"
        var output = [CChar](repeating: 0, count: 4096)
        inferenceMarker("generating")
        defer { inferenceMarker("idle") }
        let code = lv_generate(model.path, prompt, &output, Int32(output.count), cancellation.pointer)
        try Task.checkCancellation()
        guard code == 0 else { throw VoiceError.invalid }
        return String(String(cString: output).trimmingCharacters(in: .whitespacesAndNewlines).prefix(500))
    }
    private func inferenceMarker(_ phase: String) {
        #if DEBUG
        let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("native-phase.json")
        let value: [String: Any] = ["phase": phase, "at": Date().timeIntervalSince1970]
        if let data = try? JSONSerialization.data(withJSONObject: value) { try? data.write(to: url, options: .atomic) }
        #endif
    }

}
