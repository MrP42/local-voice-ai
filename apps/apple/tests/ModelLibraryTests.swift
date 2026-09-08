// Compile this test executable with iPhone/ModelLibrary.swift; pass a verified Qwen 0.5B file.
import Foundation
import CryptoKit

@main struct ModelLibraryTests {
    static func digest(_ url: URL) throws -> String {
        let input = try FileHandle(forReadingFrom: url); defer { try? input.close() }
        var hash = SHA256()
        while let data = try input.read(upToCount: 1024 * 1024), !data.isEmpty { hash.update(data: data) }
        return hash.finalize().map { String(format: "%02x", $0) }.joined()
    }
    static func main() async throws {
        guard CommandLine.arguments.count == 2 else { fatalError("Pass a verified Qwen 0.5B model path") }
        let fixture = URL(fileURLWithPath: CommandLine.arguments[1])
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ModelLibraryTests-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let library = ModelLibrary(directory: root), expected = LocalModel.all[2]
        _ = try await library.install(from: fixture, expected: expected)
        let installed = root.appendingPathComponent(expected.name)
        guard try digest(installed) == expected.sha256 else { fatalError("Verified model was not installed") }
        print("PASS approved model installs with expected SHA-256")
        do {
            _ = try await library.install(from: fixture, expected: LocalModel.all[0])
            fatalError("Wrong requested model accepted")
        } catch ModelTransferError.size { print("PASS wrong requested model is rejected") }
        let corrupt = root.appendingPathComponent("corrupt.gguf")
        try FileManager.default.copyItem(at: fixture, to: corrupt)
        let file = try FileHandle(forWritingTo: corrupt); try file.write(contentsOf: Data([0])); try file.close()
        do {
            _ = try await library.install(from: corrupt, expected: expected)
            fatalError("Corrupt model accepted")
        } catch ModelTransferError.integrity { print("PASS same-size corrupt model is rejected") }
        let task = Task { try await library.install(from: fixture, expected: expected) }
        task.cancel()
        do { _ = try await task.value; fatalError("Cancelled installation completed") }
        catch is CancellationError { print("PASS cancellation stops installation") }
        guard try digest(installed) == expected.sha256 else { fatalError("Existing model changed after failure") }
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: root.path).filter { $0.hasPrefix(".install-") }
        guard leftovers.isEmpty else { fatalError("Uncommitted installation files left behind") }
        print("PASS existing model survives failures and temporary files are cleaned")
    }
}
