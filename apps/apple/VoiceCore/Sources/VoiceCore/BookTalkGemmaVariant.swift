import Foundation
#if canImport(CryptoKit)
import CryptoKit

/// Zugelassene Dateien von Gemma 4 E2B. Dieselbe Modell-ID, aber eigene Quantisierung: Die gehört
/// ins Eval-Tupel, damit Ergebnisse getrennt bleiben. Erkannt wird über den Hash, nicht über den Namen.
public struct BookTalkGemmaVariant: Equatable, Sendable {
    public static let modelId = "gemma-4-E2B-it"

    public let quantization: String
    public let fileName: String
    public let bytes: Int64
    public let sha256: String

    /// Standard für neue Downloads.
    public static let q4KM = BookTalkGemmaVariant(quantization: "Q4_K_M", fileName: "gemma-4-E2B-it-Q4_K_M.gguf", bytes: 3106738272,
                                                  sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8")
    /// Vorhandene ältere Datei; wird nicht geladen, aber angenommen.
    public static let q40 = BookTalkGemmaVariant(quantization: "q4_0", fileName: "gemma-4-E2B_q4_0-it.gguf", bytes: 3349516256,
                                                 sha256: "e12dc03b166b654ad8e8a6671f3abcebd4bbf512696efd582b2fb27dde5bd1e6")
    public static let all = [q4KM, q40]

    /// Modell-Feld des Eval-Tupels.
    public var evalModel: String { Self.modelId + "#" + quantization }

    public static func identify(bytes: Int64, sha256: String) -> BookTalkGemmaVariant? {
        all.first { $0.bytes == bytes && $0.sha256 == sha256.lowercased() }
    }

    /// Datei streamend hashen und zuordnen; `nil`: keine zugelassene Fassung.
    public static func identify(file: URL) throws -> BookTalkGemmaVariant? {
        let handle = try FileHandle(forReadingFrom: file)
        defer { try? handle.close() }
        var hash = SHA256()
        var bytes: Int64 = 0
        while let chunk = try handle.read(upToCount: 4 << 20), !chunk.isEmpty {
            hash.update(data: chunk)
            bytes += Int64(chunk.count)
        }
        return identify(bytes: bytes, sha256: hash.finalize().map { String(format: "%02x", $0) }.joined())
    }
}
#endif
