import XCTest
@testable import VoiceCore

final class BookTalkGemmaVariantTests: XCTestCase {
    func testBothVariantsAreRecognizedByHash() {
        for variant in BookTalkGemmaVariant.all {
            XCTAssertEqual(BookTalkGemmaVariant.identify(bytes: variant.bytes, sha256: variant.sha256), variant)
            XCTAssertEqual(BookTalkGemmaVariant.identify(bytes: variant.bytes, sha256: variant.sha256.uppercased()), variant)
        }
    }

    func testWrongHashOrSizeIsRefused() {
        let variant = BookTalkGemmaVariant.q40
        XCTAssertNil(BookTalkGemmaVariant.identify(bytes: variant.bytes, sha256: String(repeating: "0", count: 64)))
        XCTAssertNil(BookTalkGemmaVariant.identify(bytes: variant.bytes - 1, sha256: variant.sha256))
        XCTAssertNil(BookTalkGemmaVariant.identify(bytes: variant.bytes, sha256: BookTalkGemmaVariant.q4KM.sha256))
    }

    func testFileWithAllowedNameButOtherContentIsRefused() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            .appendingPathExtension("gguf")
        try Data("kein Modell".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertNil(try BookTalkGemmaVariant.identify(file: file))
    }

    func testEvalTupleKeepsQuantizationsApart() {
        XCTAssertNotEqual(BookTalkGemmaVariant.q4KM.evalModel, BookTalkGemmaVariant.q40.evalModel)
        XCTAssertTrue(BookTalkGemmaVariant.q40.evalModel.hasPrefix(BookTalkGemmaVariant.modelId))
    }
}
