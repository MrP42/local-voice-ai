import Foundation
import CryptoKit
import Darwin

enum ModelTransferError: LocalizedError {
    case server(Int), size, integrity, space, storage
    var errorDescription: String? {
        switch self {
        case .server(let code): "Downloadserver meldet Fehler \(code). Erneut versuchen."
        case .size: "Download unvollständig oder falsche Dateigröße. Erneut laden."
        case .integrity: "Prüfsumme stimmt nicht. Das Modell wurde nicht freigegeben."
        case .storage: "Modell konnte nicht sicher gespeichert werden."
        case .space: "Zu wenig freier Speicher für Download und sichere Installation."
        }
    }
    static func message(_ error: Error) -> String {
        if let error = error as? ModelTransferError { return error.localizedDescription }
        if let error = error as? URLError {
            switch error.code {
            case .cancelled: return "Download abgebrochen."
            case .notConnectedToInternet, .networkConnectionLost: return "Internetverbindung unterbrochen. Erneut versuchen."
            case .timedOut: return "Downloadserver antwortet nicht rechtzeitig. Erneut versuchen."
            default: return "Netzwerkfehler (\(error.errorCode)). Erneut versuchen."
            }
        }
        if (error as NSError).code == NSFileWriteOutOfSpaceError { return ModelTransferError.space.localizedDescription }
        return "Modelldatei konnte nicht sicher installiert werden. Erneut versuchen."
    }
}

struct LocalModel: Identifiable, Sendable {
    var id: String { name }
    let name: String
    let label: String
    let bytes: Int64
    let sha256: String
    /// Gemma 4 E2B für das Buch-Gespräch (Epic #307, G6). Q4_K_M, Apache-2.0 (Lizenz-Tag des Repos und von google/gemma-4-E2B-it).
    /// Größe und SHA-256 stammen aus den LFS-Metadaten des Hugging-Face-Repos (API `?blobs=true`, 2026-11-05),
    /// die URL ist auf diese Repo-Revision festgelegt, damit sich unter dem Namen nichts ändert.
    /// TODO(Mac): `llama.xcframework` b5046 (setup_native_engines.py) kennt die Architektur `gemma4` noch nicht;
    /// ohne neueres llama.cpp lädt `lv_llama_load` diese Datei nicht (Gespräch fällt dann auf Fundstellen zurück).
    static let gemmaName = "gemma-4-E2B-it-Q4_K_M.gguf"
    static let gemmaRepository = "unsloth/gemma-4-E2B-it-GGUF"
    static let gemmaRevision = "0314792d7f1f7e229411f620751375812bb9faf2"
    var isGemma: Bool { name == Self.gemmaName }
    var downloadURL: URL {
        if isGemma { return URL(string: "https://huggingface.co/\(Self.gemmaRepository)/resolve/\(Self.gemmaRevision)/\(name)")! }
        let repository = name.hasPrefix("ggml-") ? "ggerganov/whisper.cpp" : name.contains("1.5b") ? "Qwen/Qwen2.5-1.5B-Instruct-GGUF" : "Qwen/Qwen2.5-0.5B-Instruct-GGUF"
        return URL(string: "https://huggingface.co/\(repository)/resolve/main/\(name)")!
    }
    static let all = [
        LocalModel(name: "ggml-base.bin", label: "Whisper Base", bytes: 147951465, sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"),
        LocalModel(name: "ggml-small.bin", label: "Whisper Small", bytes: 487601967, sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"),
        LocalModel(name: "qwen2.5-0.5b-instruct-q4_k_m.gguf", label: "Qwen – kurze Antworten", bytes: 491400032, sha256: "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db"),
        LocalModel(name: "qwen2.5-1.5b-instruct-q4_k_m.gguf", label: "Qwen – Auswertungen", bytes: 1117320736, sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e"),
        LocalModel(name: gemmaName, label: "Gemma 4 E2B – Buch-Gespräch", bytes: 3106738272, sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8")
    ]
}
struct InstalledModel: Identifiable, Sendable {
    var id: String { model.id }
    let model: LocalModel
    let installedBytes: Int64
    var installed: Bool { installedBytes == model.bytes }
}
actor ModelLibrary {
    static let shared = ModelLibrary()
    static var folder: URL { FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Models") }
    private let directory: URL
    init(directory: URL = ModelLibrary.folder) { self.directory = directory }
    func download(_ model: LocalModel) async throws -> String {
        let (temporary, response) = try await URLSession.shared.download(from: model.downloadURL)
        defer { try? FileManager.default.removeItem(at: temporary) }
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else { throw ModelTransferError.server((response as? HTTPURLResponse)?.statusCode ?? 0) }
        try Task.checkCancellation()
        return try install(from: temporary)
    }
    func inventory() -> [InstalledModel] {
        LocalModel.all.map { descriptor in
            let size = (try? FileManager.default.attributesOfItem(atPath: directory.appendingPathComponent(descriptor.name).path)[.size] as? NSNumber)?.int64Value ?? 0
            return InstalledModel(model: descriptor, installedBytes: size)
        }
    }
    /// The destination changes only after size and SHA-256 identify an approved model.
    func install(from source: URL, expected: LocalModel? = nil) throws -> String {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let fm = FileManager.default
        let attributes = try fm.attributesOfItem(atPath: source.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular,
              let size = attributes[.size] as? NSNumber,
              LocalModel.all.contains(where: { $0.bytes == size.int64Value }),
              expected == nil || expected?.bytes == size.int64Value else { throw ModelTransferError.size }
        try fm.createDirectory(at: directory, withIntermediateDirectories: true)
        let free = try fm.attributesOfFileSystem(forPath: directory.path)[.systemFreeSize] as? NSNumber
        guard let free, free.int64Value > size.int64Value + 16 * 1024 * 1024 else { throw ModelTransferError.space }
        let temporary = directory.appendingPathComponent(".install-" + UUID().uuidString)
        guard fm.createFile(atPath: temporary.path, contents: nil, attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]) else { throw ModelTransferError.storage }
        defer { try? fm.removeItem(at: temporary) }
        let input = try FileHandle(forReadingFrom: source), output = try FileHandle(forWritingTo: temporary)
        defer { try? input.close(); try? output.close() }
        var hash = SHA256(), count: Int64 = 0
        while let data = try input.read(upToCount: 1024 * 1024), !data.isEmpty {
            try Task.checkCancellation()
            count += Int64(data.count)
            guard count <= size.int64Value else { throw ModelTransferError.size }
            hash.update(data: data); try output.write(contentsOf: data)
        }
        let digest = hash.finalize().map { String(format: "%02x", $0) }.joined()
        guard let model = LocalModel.all.first(where: { $0.bytes == count && $0.sha256 == digest }),
              expected == nil || expected?.id == model.id else { throw ModelTransferError.integrity }
        try output.synchronize(); try output.close()
        guard rename(temporary.path, directory.appendingPathComponent(model.name).path) == 0 else { throw ModelTransferError.storage }
        let fd = open(directory.path, O_RDONLY)
        guard fd >= 0 else { throw ModelTransferError.storage }
        defer { close(fd) }
        guard fsync(fd) == 0 else { throw ModelTransferError.storage }
        return model.label
    }
}
