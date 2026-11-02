import XCTest
@testable import VoiceCore

final class LibrarySyncModelTests: XCTestCase {
    func testSyncedBookRoundTripKeepsDesktopKeysAndFiles() throws {
        var book = LibraryBook(id: "book_1", title: "Die Verwandlung", authors: ["Franz Kafka"], addedMs: 1)
        book.audio = LibraryAudioVersion(file: "teil_001.mp3", durationMs: 1000)
        let files = BookFiles(text: BlobRef(blob: String(repeating: "a", count: 64), size: 10, chunks: 1),
                              audio: [BlobRef(blob: String(repeating: "b", count: 64), size: 20, chunks: 1, file: "teil_001.mp3")])
        let data = try SyncedBook(book: book, files: files, updated: 42).json()
        let raw = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        XCTAssertNotNil(raw["added_ms"]); XCTAssertNil(raw["cover_path"])
        let back = try SyncedBook.decode(data)
        XCTAssertEqual(back.book, book); XCTAssertEqual(back.files, files); XCTAssertEqual(back.updated, 42)
    }

    func testObjectIdsFitHubRules() {
        let id = LibrarySyncFormat.progressObjectId("text:page_1:original")
        XCTAssertEqual(id.count, 42)
        XCTAssertNotNil(id.range(of: "^[A-Za-z0-9_-]{1,64}$", options: .regularExpression))
        XCTAssertNotEqual(id, LibrarySyncFormat.progressObjectId("audio:book_1"))
    }

    func testChunkSealIsBoundToBlobAndIndex() throws {
        let key = [UInt8](repeating: 9, count: 32)
        let blob = LibrarySyncFormat.blobId(key: key, plaintextSha256: Data(repeating: 1, count: 32))
        XCTAssertEqual(blob.count, 64)
        let sealed = try LibrarySyncFormat.sealChunk(key: key, userId: 7, blobId: blob, index: 3, plain: Array("Teil".utf8))
        XCTAssertEqual(try LibrarySyncFormat.openChunk(key: key, userId: 7, blobId: blob, index: 3, sealed: sealed), Array("Teil".utf8))
        XCTAssertThrowsError(try LibrarySyncFormat.openChunk(key: key, userId: 7, blobId: blob, index: 4, sealed: sealed))
        XCTAssertThrowsError(try LibrarySyncFormat.openChunk(key: key, userId: 8, blobId: blob, index: 3, sealed: sealed))
    }

    func testCanonicalHashIgnoresKeyOrder() {
        XCTAssertEqual(LibrarySyncFormat.canonicalHash(Data(#"{"a":1,"b":[2,3]}"#.utf8)),
                       LibrarySyncFormat.canonicalHash(Data(#"{"b":[2,3],"a":1}"#.utf8)))
    }

    func testDecodesHubPullResponse() throws {
        let json = #"{"objects":[{"collection":"book","object_id":"book_1","revision":2,"payload":"x","key_id":"deadbeef","deleted":false,"device_id":"d","server_seq":5}],"cursor":5,"more":false,"server_time":"2026-11-02"}"#
        let r = try JSONDecoder().decode(HubPullResponse.self, from: Data(json.utf8))
        XCTAssertEqual(r.objects.first?.serverSeq, 5); XCTAssertEqual(r.cursor, 5)
    }
}
