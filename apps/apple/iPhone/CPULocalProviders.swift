import Foundation
import AVFoundation
import CryptoKit

/// The serial actor keeps inference off the main thread and limits concurrent model memory.
actor CPULocalProviders {
    static let shared = CPULocalProviders()
    private var verifiedModels = Set<String>()
    private let hashes = [
        "ggml-small.bin": "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        "ggml-base.bin": "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        "qwen2.5-0.5b-instruct-q4_k_m.gguf": "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db"
    ]
    private func modelURL(_ name: String) throws -> URL {
        let folder = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Models")
        let url = folder.appendingPathComponent(name)
        guard FileManager.default.fileExists(atPath: url.path) else { throw VoiceError.missing }
        if !verifiedModels.contains(name) {
            let input = try FileHandle(forReadingFrom: url)
            defer { try? input.close() }
            var hash = SHA256()
            while let data = try input.read(upToCount: 1024 * 1024), !data.isEmpty { hash.update(data: data) }
            guard hash.finalize().map({ String(format: "%02x", $0) }).joined() == hashes[name] else { throw VoiceError.invalid }
            verifiedModels.insert(name)
        }
        return url
    }
    func transcribe(_ url: URL) throws -> String {
        let model: URL
        let small = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Models/ggml-small.bin")
        if FileManager.default.fileExists(atPath: small.path) { model = try modelURL("ggml-small.bin") }
        else { model = try modelURL("ggml-base.bin") }
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
        var output = [CChar](repeating: 0, count: 16384)
        let code = lv_transcribe(model.path, samples, Int32(converted.frameLength), &output, Int32(output.count))
        guard code == 0 else { throw VoiceError.invalid }
        return String(cString: output).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    func reply(to transcript: String) throws -> String {
        let model = try modelURL("qwen2.5-0.5b-instruct-q4_k_m.gguf")
        // Qwen2.5's documented ChatML template; only the current turn is context.
        let text = String(transcript.prefix(1200)).replacingOccurrences(of: "<|", with: "< |")
        let prompt = "<|im_start|>system\nDu bist ein hilfreicher Sprachbegleiter. Antworte kurz auf Deutsch in einem Satz. Führe keine externen Aktionen aus.<|im_end|>\n<|im_start|>user\n\(text)<|im_end|>\n<|im_start|>assistant\n"
        var output = [CChar](repeating: 0, count: 4096)
        let code = lv_generate(model.path, prompt, &output, Int32(output.count))
        guard code == 0 else { throw VoiceError.invalid }
        return String(String(cString: output).trimmingCharacters(in: .whitespacesAndNewlines).prefix(500))
    }
}
