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

    // MARK: Flache Dateien gehören zur Hauptfassung des Senders

    private func blob(_ c: Character, file: String? = nil) -> BlobRef {
        BlobRef(blob: String(repeating: c, count: 64), size: 10, chunks: 1, file: file)
    }

    func testFlatFilesBelongToSendersPrimaryAfterMerge() throws {
        let sender = try book(primary: "ed_b")   // Sender: Hauptfassung EN
        let local = try book(primary: "ed_a")    // lokal nach dem Merge: Hauptfassung DE
        let flat = BookEditions.senderPrimary(local: local, incoming: sender)
        XCTAssertEqual(flat, "ed_b")
        let files = BookFiles(text: blob("t"), audio: [blob("a", file: "teil_001.mp3")])
        XCTAssertEqual(files.files(forEdition: "ed_b", flat: flat).audio.count, 1)
        XCTAssertNotNil(files.files(forEdition: "ed_b", flat: flat).text)
        XCTAssertTrue(files.files(forEdition: "ed_a", flat: flat).audio.isEmpty)
        XCTAssertNil(files.files(forEdition: "ed_a", flat: flat).text)
    }

    func testLegacyDeviceFlatFilesGoToLocalPrimary() throws {
        let legacy = try JSONDecoder().decode(LibraryBook.self, from: Data(
            #"{"id":"book_1","title":"Die Verwandlung","authors":[],"added_ms":1}"#.utf8))
        let local = try book(primary: "ed_b")
        XCTAssertEqual(BookEditions.senderPrimary(local: local, incoming: legacy), "ed_b")
        let kept = legacy.keepingEditions(from: local)
        XCTAssertEqual(kept.editions, local.editions)
        XCTAssertEqual(kept.primary, local.primary)
        XCTAssertEqual(BookEditions(book: kept).primaryId, "ed_b")
    }

    func testLegacyOnBothSidesUsesMigratedId() throws {
        let legacy = try JSONDecoder().decode(LibraryBook.self, from: Data(
            #"{"id":"book_1","title":"Die Verwandlung","authors":[],"added_ms":1}"#.utf8))
        XCTAssertEqual(BookEditions.senderPrimary(local: legacy, incoming: legacy), "ed_book_1")
        XCTAssertEqual(BookEditions.senderPrimary(local: nil, incoming: legacy), "ed_book_1")
    }

    func testFlatEditionVermerkSurvivesCoding() throws {
        let meta = LibrarySyncMeta(files: BookFiles(), updated: 1, flatEdition: "ed_b")
        let back = try JSONDecoder().decode(LibrarySyncMeta.self, from: JSONEncoder().encode(meta))
        XCTAssertEqual(back.flatEdition, "ed_b")
    }

    // MARK: Rückgängig stellt die Zielfassung zurück

    func testUndoRestoresTargetBeforeSwitch() {
        let before = ProgressRecord(offset: 800, length: 1000, heard: [[0, 800]], updated: 10, unit: .ms)
        var switched = before
        switched.offset = 300; switched.updated = 20
        let undone = LibraryProgress.undoingSwitch(current: switched, before: before, now: 30)
        XCTAssertEqual(undone?.offset, 800)
        XCTAssertEqual(undone?.heard, [[0, 800]])
        XCTAssertEqual(undone?.updated, 30)
        XCTAssertNil(undone?.finished)
    }

    func testUndoKeepsRangesHeardSinceSwitch() {
        let before = ProgressRecord(offset: 800, length: 1000, heard: [[600, 800]], updated: 10, unit: .ms)
        var now = before
        now.offset = 350; now.heard = [[300, 350], [600, 800]]; now.updated = 20
        let undone = LibraryProgress.undoingSwitch(current: now, before: before, now: 30)
        XCTAssertEqual(undone?.offset, 800)
        XCTAssertEqual(undone?.heard, [[300, 350], [600, 800]])
    }

    func testUndoWithoutPriorRecordKeepsOnlyHeardRanges() {
        let none = ProgressRecord(offset: 300, length: 1000, updated: 20, unit: .ms)
        XCTAssertNil(LibraryProgress.undoingSwitch(current: none, before: nil, now: 30))
        var heard = none
        heard.heard = [[300, 350]]
        XCTAssertEqual(LibraryProgress.undoingSwitch(current: heard, before: nil, now: 30)?.heard, [[300, 350]])
    }

    func testUndoRestoresFinishedFlag() {
        var before = ProgressRecord(offset: 1000, length: 1000, updated: 10, finished: true, unit: .ms)
        before.heard = [[0, 1000]]
        var now = before
        now.finished = nil; now.offset = 300
        XCTAssertEqual(LibraryProgress.undoingSwitch(current: now, before: before, now: 30)?.finished, true)
    }

    // MARK: Empfang von einem Altgerät verliert keine Dateiverweise

    func testLegacyReceiveKeepsAllEditionFileReferences() throws {
        let local = try book(primary: "ed_b")  // lokal: EN Hauptfassung, DE weitere Fassung
        var localFiles = BookFiles(text: blob("1"), audio: [blob("2", file: "ed_b_teil.mp3")])
        localFiles.editions = .object(["ed_a": .object([
            "text": try PassthroughJSON(encoding: blob("3")),
            "audio": try PassthroughJSON(encoding: [blob("4", file: "ed_a__teil.mp3")]),
        ])])
        let legacy = try JSONDecoder().decode(LibraryBook.self, from: Data(
            #"{"id":"book_1","title":"Die Verwandlung","authors":[],"added_ms":1}"#.utf8))
        let incoming = BookFiles(text: blob("5"), audio: [blob("6", file: "neu.mp3")])
        let flat = BookEditions.senderPrimary(local: local, incoming: legacy)
        XCTAssertEqual(flat, "ed_b")

        let merged = localFiles.receivingLegacy(incoming, localFlat: "ed_b", senderFlat: flat)
        XCTAssertEqual(merged.text, incoming.text)
        XCTAssertEqual(merged.audio, incoming.audio)
        XCTAssertEqual(merged.files(forEdition: "ed_a", flat: flat).text, blob("3"))
        XCTAssertEqual(merged.files(forEdition: "ed_a", flat: flat).audio, [blob("4", file: "ed_a__teil.mp3")])
        XCTAssertEqual(merged.files(forEdition: "ed_b", flat: flat).audio, incoming.audio)

        let kept = legacy.keepingEditions(from: local)
        let json = try SyncedBook(book: kept, files: merged, updated: 9).json()
        let raw = try JSONSerialization.jsonObject(with: json) as! [String: Any]
        let editions = (raw["files"] as? [String: Any])?["editions"] as? [String: Any]
        XCTAssertNotNil(editions?["ed_a"])
        XCTAssertNil(editions?["ed_b"])
        XCTAssertNotNil(raw["editions"])
    }

    func testLegacyReceiveMovesOldFlatFilesToTheirEdition() throws {
        let local = BookFiles(text: blob("1"), audio: [blob("2", file: "a.mp3")])
        let merged = local.receivingLegacy(BookFiles(text: blob("5")), localFlat: "ed_a", senderFlat: "ed_b")
        XCTAssertEqual(merged.text, blob("5"))
        XCTAssertEqual(merged.files(forEdition: "ed_a", flat: "ed_b").text, blob("1"))
        XCTAssertEqual(merged.files(forEdition: "ed_a", flat: "ed_b").audio, [blob("2", file: "a.mp3")])
    }
}
