import Foundation
import AVFoundation
import CryptoKit

/// The serial actor keeps inference off the main thread and limits concurrent model memory.
actor CPULocalProviders {
    static let shared = CPULocalProviders()
    private var verifiedModels: [String: String] = [:]
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
    func transcribe(_ url: URL, cancellation: InferenceCancellation) throws -> String {
        try Task.checkCancellation()
        let selected = UserDefaults.standard.string(forKey: "sttModel") ?? "ggml-base.bin"
        guard ["ggml-base.bin", "ggml-small.bin"].contains(selected) else { throw VoiceError.invalid }
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
        case .empty, .silent: throw ProcessingFailure.noSpeech
        case .invalid: throw VoiceError.invalid
        case .signal: break
        }
        let model = try modelURL(modelName)
        var output = [CChar](repeating: 0, count: 16384)
        inferenceMarker("transcribing")
        defer { inferenceMarker("idle") }
        let code = lv_transcribe(model.path, samples, Int32(converted.frameLength), &output, Int32(output.count), cancellation.pointer)
        try Task.checkCancellation()
        if code == 4 { throw ProcessingFailure.noSpeech }
        guard code == 0 else { throw VoiceError.invalid }
        return String(cString: output).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    func reply(to transcript: String, cancellation: InferenceCancellation) throws -> String {
        try Task.checkCancellation()
        let model = try modelURL("qwen2.5-0.5b-instruct-q4_k_m.gguf")
        // Qwen2.5's documented ChatML template; only the current turn is context.
        let text = String(transcript.prefix(1200)).replacingOccurrences(of: "<|", with: "< |")
        let prompt = "<|im_start|>system\nDu bist ein hilfreicher Sprachbegleiter. Antworte kurz auf Deutsch in einem Satz. Du kannst nur Text antworten und Notizen speichern. Behaupte niemals, einen Wecker gestellt, Nachrichten gesendet oder andere externe Aktionen ausgeführt zu haben.<|im_end|>\n<|im_start|>user\n\(text)<|im_end|>\n<|im_start|>assistant\n"
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
