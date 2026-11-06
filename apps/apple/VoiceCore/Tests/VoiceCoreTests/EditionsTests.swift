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
        // Ohne Text oder Hörbuch gibt es keine (synthetische) Fassung, im Kern ebenso.
        let withText = try JSONDecoder().decode(LibraryBook.self, from: Data(
            #"{"id":"book_1","title":"Die Verwandlung","authors":[],"added_ms":1,"text":{"page_id":"p","origin":"document"}}"#.utf8))
        XCTAssertEqual(BookEditions.senderPrimary(local: withText, incoming: withText), "ed_book_1")
        XCTAssertEqual(BookEditions.senderPrimary(local: nil, incoming: withText), "ed_book_1")
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

    func testLegacyReceiveKeepsOwnFlatFieldsAsFallback() {
        let local = BookFiles(text: blob("1"), audio: [blob("2", file: "a.mp3")])
        let merged = local.receivingLegacy(BookFiles(text: blob("5")), localFlat: "ed_b", senderFlat: "ed_b")
        XCTAssertEqual(merged.text, blob("5"))
        XCTAssertEqual(merged.audio, [blob("2", file: "a.mp3")])
    }

    func testLegacyReceiveMergesMovedFlatFilesFieldwise() throws {
        var local = BookFiles(text: blob("1"))
        local.editions = .object(["ed_a": .object([
            "text": try PassthroughJSON(encoding: blob("3")),
            "cover": try PassthroughJSON(encoding: blob("7")),
            "zukunft": .string("bleibt"),
        ])])
        let merged = local.receivingLegacy(BookFiles(), localFlat: "ed_a", senderFlat: "ed_b")
        let entry = merged.editions?["ed_a"]
        XCTAssertEqual(entry?["text"]?.decoded(BlobRef.self), blob("1"))      // umziehendes Feld gewinnt
        XCTAssertEqual(entry?["cover"]?.decoded(BlobRef.self), blob("7"))     // fehlendes Feld: vorhandener Wert bleibt
        XCTAssertEqual(entry?["zukunft"]?.stringValue, "bleibt")
    }

    func testLegacyReceiveDropsTombstonedEditionFiles() throws {
        var local = BookFiles(text: blob("1"))
        local.editions = .object([
            "ed_a": .object(["text": try PassthroughJSON(encoding: blob("3"))]),
            "ed_tot": .object(["text": try PassthroughJSON(encoding: blob("4"))]),
        ])
        let merged = local.receivingLegacy(BookFiles(text: blob("5")), localFlat: "ed_b", senderFlat: "ed_b", live: ["ed_a", "ed_b"])
        XCTAssertNotNil(merged.editions?["ed_a"])
        XCTAssertNil(merged.editions?["ed_tot"])
        let movedToDead = local.receivingLegacy(BookFiles(text: blob("5")), localFlat: "ed_tot", senderFlat: "ed_b", live: ["ed_a", "ed_b"])
        XCTAssertNil(movedToDead.editions?["ed_tot"])
    }

    func testLegacyReceiveKeepsUnknownFieldsOfSenderEntry() throws {
        var local = BookFiles(text: blob("1"))
        local.editions = .object(["ed_b": .object(["text": try PassthroughJSON(encoding: blob("3")), "zukunft": .string("bleibt")])])
        let merged = local.receivingLegacy(BookFiles(text: blob("5")), localFlat: "ed_b", senderFlat: "ed_b")
        XCTAssertEqual(merged.text, blob("5"))
        XCTAssertEqual(merged.editions?["ed_b"]?["zukunft"]?.stringValue, "bleibt")
        XCTAssertNil(merged.editions?["ed_b"]?["text"])
        var onlyText = BookFiles()
        onlyText.editions = .object(["ed_b": .object(["text": try PassthroughJSON(encoding: blob("3"))])])
        XCTAssertNil(onlyText.receivingLegacy(BookFiles(text: blob("5")), localFlat: "ed_b", senderFlat: "ed_b").editions)
    }

    // MARK: Echo-Schutz

    func testNeedsResendOnlyWithoutNewEditionData() throws {
        let local = try book(primary: "ed_b")
        let files = BookFiles()
        let legacy = try JSONDecoder().decode(LibraryBook.self, from: Data(
            #"{"id":"book_1","title":"Die Verwandlung","authors":[],"added_ms":1}"#.utf8))
        let sent = local.editionFingerprint(files: files)
        XCTAssertFalse(BookEditions.needsResend(local: local, files: files, sentFingerprint: sent, incoming: legacy), "echtes Echo")
        let newer = try XCTUnwrap(local.settingReading(edition: "ed_a", position: nil, device: "iphone", now: 100))
        XCTAssertTrue(BookEditions.needsResend(local: newer, files: files, sentFingerprint: sent, incoming: legacy), "neueres reading")
        XCTAssertTrue(BookEditions.needsResend(local: local, files: files, sentFingerprint: nil, incoming: legacy), "nie gesendet")
        XCTAssertTrue(BookEditions.needsResend(local: local, files: files, sentFingerprint: sent, incoming: local), "kein Altgerät")
    }

    // MARK: Fortschritt beim Empfang heben

    func testLiftedReadingOnlyWhenSelectionChanged() throws {
        let reading = #""reading":{"edition":"ed_b","updated":8,"device":"pc","position":{"key":"audio:book_1:ed_b","unit":"ms","offset":1234}},"#
        let incoming = try book(reading: reading)
        let lift = try XCTUnwrap(BookEditions.liftedReading(local: nil, incoming: incoming))
        XCTAssertEqual(lift.position.offset, 1234); XCTAssertEqual(lift.updated, 8)
        XCTAssertNil(BookEditions.liftedReading(local: incoming, incoming: incoming))
        XCTAssertNil(BookEditions.liftedReading(local: nil, incoming: try book()), "ohne reading")
    }

    func testLiftingOnlyOlderRecords() {
        let pos = ReadingPositionRef(key: "audio:book_1:ed_b", unit: "ms", offset: 1234)
        let old = ProgressRecord(offset: 10, length: 5000, heard: [[0, 10]], updated: 5, finished: true, unit: .ms)
        let lifted = LibraryProgress.lifting(old, to: pos, updated: 8)
        XCTAssertEqual(lifted?.offset, 1234); XCTAssertEqual(lifted?.updated, 8)
        XCTAssertEqual(lifted?.heard, [[0, 10]]); XCTAssertEqual(lifted?.finished, true)
        XCTAssertNil(LibraryProgress.lifting(ProgressRecord(offset: 10, length: 5000, updated: 9), to: pos, updated: 8))
        XCTAssertEqual(LibraryProgress.lifting(nil, to: pos, updated: 8)?.offset, 1234)
    }

    // MARK: Wechsel ohne bekannte Länge

    func testSwitchWithoutTargetLengthStartsAtBeginning() throws {
        let editions = BookEditions(book: try book())
        let unknown = EditionSwitch.plan(editions: editions, from: "ed_a", to: "ed_b", fromMedium: .audio, toMedium: .audio,
                                         fromChapters: nil, toChapters: ChapterList(titles: [], starts: [], length: 0),
                                         toKey: "audio:book_1:ed_b", offset: 100, length: 1000)
        XCTAssertTrue(unknown.fromStart)
        XCTAssertEqual(unknown.position.offset, 0)
        XCTAssertTrue(unknown.notice(languageName: "English").contains("vom Anfang"))
        XCTAssertFalse(unknown.notice(languageName: "English").contains("ungefähr"))
        let known = EditionSwitch.plan(editions: editions, from: "ed_a", to: "ed_b", fromMedium: .audio, toMedium: .audio,
                                       fromChapters: nil, toChapters: ChapterList(titles: [], starts: [], length: 400),
                                       toKey: "audio:book_1:ed_b", offset: 100, length: 1000)
        XCTAssertFalse(known.fromStart)
        XCTAssertEqual(known.position.offset, 40, accuracy: 0.001)
    }

    func testAudioChaptersUseCachedDurationsForMissingTracks() {
        let audio = LibraryAudioVersion(file: "a.mp3", tracks: [
            LibraryAudioTrack(file: "a.mp3", title: "Eins", durationMs: nil),
            LibraryAudioTrack(file: "b.mp3", title: "Zwei", durationMs: nil),
        ])
        XCTAssertTrue(EditionSwitch.audioChapters(audio).starts.isEmpty)
        let list = EditionSwitch.audioChapters(audio, fallbackDurations: [100, 200])
        XCTAssertEqual(list.starts, [0, 100]); XCTAssertEqual(list.length, 300)
    }
}
