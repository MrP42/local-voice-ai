import XCTest
@testable import VoiceCore
import CryptoKit

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

    /// lva#344: Sprachfassungen kennt nur der Desktop; das iPhone muss sie unverändert weiterreichen.
    func testSyncedBookKeepsEditionFieldsFromDesktop() throws {
        let json = """
        {"id":"book_1","title":"Die Verwandlung","authors":["Franz Kafka"],"added_ms":1,"schema":2,
         "work_key":"w1","primary":{"edition":"ed_book_1","updated":5,"device":"pc"},
         "editions":[{"id":"ed_book_1","lang":"de","title":"Die Verwandlung","audio_key":"audio:book_1","origin":"import","added_ms":1,"updated":5,"device":"pc"},
                     {"id":"ed_x","lang":"en","title":"The Metamorphosis","audio_key":"audio:book_1:ed_x","origin":"join","added_ms":2,"updated":6,"device":"pc",
                      "audio":{"file":"ed_x__teil_001.mp3","duration_ms":1000}}],
         "edition_tombstones":[{"id":"ed_y","removed_ms":7}],
         "reading":{"edition":"ed_x","updated":8,"device":"pc","position":{"key":"audio:book_1:ed_x","unit":"ms","offset":1234.5}},
         "alignments":[{"a":"ed_book_1","b":"ed_x","updated":9,"device":"pc","groups":[{"a":[0],"b":[0,1],"confirmed":true}]}],
         "files":{"audio":[],"editions":{"ed_x":{"audio":[{"blob":"\(String(repeating: "c", count: 64))","size":5,"chunks":1,"file":"ed_x__teil_001.mp3"}]}}},
         "updated":42}
        """
        let synced = try SyncedBook.decode(Data(json.utf8))
        XCTAssertEqual(synced.book.schema, 2)
        XCTAssertNotNil(synced.book.editions); XCTAssertNotNil(synced.files.editions)
        let out = try JSONSerialization.jsonObject(with: synced.json()) as! [String: Any]
        let inn = try JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
        for key in ["schema", "work_key", "primary", "editions", "edition_tombstones", "reading", "alignments"] {
            XCTAssertEqual(out[key] as? NSObject, inn[key] as? NSObject, key)
        }
        XCTAssertEqual((out["files"] as? [String: Any])?["editions"] as? NSObject,
                       (inn["files"] as? [String: Any])?["editions"] as? NSObject)
        // Ein Buch ohne Fassungen schreibt keine neuen Schlüssel.
        let plain = try JSONSerialization.jsonObject(with: SyncedBook(book: LibraryBook(id: "b", title: "T", addedMs: 1),
                                                                     files: BookFiles(), updated: 1).json()) as! [String: Any]
        XCTAssertNil(plain["editions"]); XCTAssertNil(plain["schema"]); XCTAssertNil((plain["files"] as? [String: Any])?["editions"])
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

final class CanonicalHashTests: XCTestCase {
    func testNullFieldsDoNotChangeTheHash() {
        // Rust/serde schreibt None als null, Swift lässt das Feld weg.
        XCTAssertEqual(LibrarySyncFormat.canonicalHash(Data(#"{"a":1,"b":null,"c":{"d":null,"e":2}}"#.utf8)),
                       LibrarySyncFormat.canonicalHash(Data(#"{"c":{"e":2},"a":1}"#.utf8)))
    }
}

final class DesktopVectorTests: XCTestCase {
    /// Testvektor des Desktop-Strangs (P5), unabhängig mit libsodium bestätigt.
    func testBlobIdAndChunkMatchDesktop() throws {
        let key = (0..<32).map { UInt8($0) }
        let plain = Array("Hallo Buch".utf8)
        let digest = Data(SHA256.hash(data: Data(plain)))
        let blob = LibrarySyncFormat.blobId(key: key, plaintextSha256: digest)
        XCTAssertEqual(blob, "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322")
        let sealed = try LibrarySyncFormat.sealChunk(key: key, userId: 7, blobId: blob, index: 0, plain: plain, nonce: (0x40...0x57).map { UInt8($0) })
        XCTAssertEqual(sealed.map { String(format: "%02x", $0) }.joined(),
                       "404142434445464748494a4b4c4d4e4f50515253545556579c58691cbfc03b63ec9ccb2edc16b2b3f35766f059626d3c4bbf")
    }

    func testFavoriteSurvivesRoundTrip() throws {
        var book = LibraryBook(id: "book_1", title: "T", addedMs: 1730000000000)
        book.favorite = true
        let data = try SyncedBook(book: book, files: BookFiles(), updated: 5).json()
        XCTAssertTrue(String(decoding: data, as: UTF8.self).contains(#""added_ms":1730000000000"#), "Ganzzahlen ohne .0")
        XCTAssertEqual(try SyncedBook.decode(data).book.favorite, true)
        XCTAssertEqual(try JSONDecoder().decode(LibraryBook.self, from: Data(#"{"id":"a","title":"b"}"#.utf8)).favorite, false)
    }

    func testChunkCountMatchesDesktopAndFitsHosting() {
        XCTAssertEqual(LibrarySyncFormat.chunkCount(size: 0), 1)
        XCTAssertEqual(LibrarySyncFormat.chunkCount(size: LibrarySyncFormat.oneChunkMaxBytes), 1)
        XCTAssertEqual(LibrarySyncFormat.chunkCount(size: LibrarySyncFormat.oneChunkMaxBytes + 1), 2)
        XCTAssertEqual(LibrarySyncFormat.chunkCount(size: 2 * LibrarySyncFormat.chunkPlainBytes), 2)
        XCTAssertEqual(LibrarySyncFormat.chunkCount(size: 2 * LibrarySyncFormat.chunkPlainBytes + 1), 3)
        // Größte Anfrage: Nonce, Tag, base64, etwas JSON; gemessen geht 4,9 MiB.
        let body = (LibrarySyncFormat.oneChunkMaxBytes + 40 + 2) / 3 * 4 + 200
        XCTAssertLessThan(Double(body), 4.9 * 1024 * 1024)
    }
}
