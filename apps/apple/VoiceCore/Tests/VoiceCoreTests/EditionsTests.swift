import XCTest
@testable import VoiceCore

final class EditionsTests: XCTestCase {
    private func book(primary: String? = "ed_a", reading: String = "") throws -> LibraryBook {
        let primaryJSON = primary.map { #""primary":{"edition":"\#($0)","updated":5,"device":"pc"},"# } ?? ""
        let json = """
        {"id":"book_1","title":"Die Verwandlung","authors":["Franz Kafka"],"added_ms":1,"schema":2,\(primaryJSON)\(reading)
         "editions":[{"id":"ed_a","lang":"de","title":"Die Verwandlung","audio_key":"audio:book_1","origin":"import","added_ms":1,"updated":5,"device":"pc"},
                     {"id":"ed_b","lang":"en","title":"The Metamorphosis","audio_key":"audio:book_1:ed_b","origin":"join","added_ms":2,"updated":6,"device":"pc"}]}
        """
        return try JSONDecoder().decode(LibraryBook.self, from: Data(json.utf8))
    }

    func testEffectivePrimaryFollowsRegister() throws {
        XCTAssertEqual(BookEditions(book: try book(primary: "ed_b")).primaryId, "ed_b")
        XCTAssertEqual(BookEditions(book: try book(primary: "ed_zz")).primaryId, "ed_a")
        XCTAssertEqual(BookEditions(book: try book(primary: nil)).primaryId, "ed_a")
    }

    func testHasLanguages() throws {
        XCTAssertTrue(BookEditions(book: try book()).hasLanguages)
    }

    func testSettingReadingIsMonotonicAndLossless() throws {
        let pos = ReadingPositionRef(key: "audio:book_1:ed_b", unit: "ms", offset: 1234)
        let first = try XCTUnwrap(try book().settingReading(edition: "ed_b", position: pos, device: "iphone", now: 100))
        let r1 = try XCTUnwrap(ReadingState(json: try XCTUnwrap(first.reading)))
        XCTAssertEqual(r1.edition, "ed_b"); XCTAssertEqual(r1.updated, 100); XCTAssertEqual(r1.device, "iphone")
        let second = try XCTUnwrap(first.settingReading(edition: "ed_a", position: nil, device: "iphone", now: 50))
        XCTAssertEqual(ReadingState(json: try XCTUnwrap(second.reading))?.updated, 101)
        XCTAssertEqual(second.editions, try book().editions)
    }

    func testSettingReadingRejectsUnknownEdition() throws {
        XCTAssertNil(try book().settingReading(edition: "ed_nope", position: nil, device: "iphone", now: 1))
    }
}
