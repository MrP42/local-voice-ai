import Foundation
import XCTest
@testable import VoiceCore

// Buch-Gespräch (G5/G6): Lesegrenze, Verlaufsspeicher, Chat-Rahmen, Satzende, Verlaufsmodell.
// Linux-tauglich; Audio, UI und llama.cpp gehören zur App und sind hier nicht abgedeckt.

final class BookTalkFrontierTests: XCTestCase {
    func testFrontierFieldOnlyRises() {
        var rec = LibraryProgress.recordPosition(nil, to: 1_000, length: 50_000, unit: .chars, now: 1)
        XCTAssertEqual(rec.frontier, 1_000)
        rec = LibraryProgress.recordPosition(rec, to: 3_000, length: 50_000, unit: .chars, now: 2)
        XCTAssertEqual(rec.frontier, 3_000)
        // Zurückblättern senkt die Grenze nicht.
        rec = LibraryProgress.recordPosition(rec, to: 200, length: 50_000, unit: .chars, now: 3)
        XCTAssertEqual(rec.frontier, 3_000)
        XCTAssertEqual(rec.offset, 200)
    }

    func testBigJumpDoesNotRaiseFrontier() {
        var rec = LibraryProgress.recordPosition(nil, to: 1_000, length: 50_000, unit: .chars, now: 1)
        rec = LibraryProgress.recordPosition(rec, to: 40_000, length: 50_000, unit: .chars, now: 2)
        XCTAssertEqual(rec.offset, 40_000)
        XCTAssertEqual(rec.frontier, 1_000, "Sprung ist nicht gelesen")
    }

    func testFirstJumpOnFreshRecordKeepsFrontierAtZero() {
        let jumped = LibraryProgress.recordPosition(nil, to: 40_000, length: 50_000, unit: .chars, now: 1)
        XCTAssertEqual(jumped.frontier, 0, "0 bleibt 0 und wird nicht zu nil")
        XCTAssertEqual(LibraryProgress.reach(jumped), 0, "kein Rueckfall auf den Offset, wenn das Feld da ist")
        XCTAssertNil(LibraryProgress.frontier(text: jumped, audio: nil, map: nil))
        let again = LibraryProgress.recordPosition(jumped, to: 90_000, length: 50_000, unit: .chars, now: 2)
        XCTAssertEqual(again.frontier, 0)
        let merged = LibraryProgress.merge(jumped, again)
        XCTAssertEqual(merged.frontier, 0)
        let heard = LibraryProgress.markHeard(jumped, from: 0, to: 0)
        XCTAssertEqual(heard.frontier, 0)
    }

    func testLegacyRecordKeepsOffsetUntilFirstWrite() {
        let legacy = ProgressRecord(offset: 7_000, length: 50_000, updated: 1)
        XCTAssertEqual(LibraryProgress.reach(legacy), 7_000)
        let next = LibraryProgress.recordPosition(legacy, to: 7_100, length: 50_000, unit: .chars, now: 2)
        XCTAssertEqual(next.frontier, 7_100)
    }

    func testMarkHeardRaisesFrontierToRangeEnd() {
        let base = ProgressRecord(offset: 500, length: 10_000, heard: [[0, 500]], updated: 1, frontier: 500)
        let out = LibraryProgress.markHeard(base, from: 500, to: 6_000)
        XCTAssertEqual(out.frontier, 6_000)
    }

    func testMergeTakesMaximumFromBothSides() {
        let a = ProgressRecord(offset: 10, length: 100, heard: [[0, 40]], updated: 5, frontier: 40)
        let b = ProgressRecord(offset: 90, length: 100, heard: [[0, 10]], updated: 9, frontier: 10)
        XCTAssertEqual(LibraryProgress.merge(a, b).frontier, 40)
        XCTAssertEqual(LibraryProgress.merge(b, a).frontier, 40)
    }

    func testLegacyRecordFallsBackToRangesThenOffset() {
        let ranges = ProgressRecord(offset: 9_000, length: 20_000, heard: [[0, 700]], read: [[0, 1_200]], updated: 1)
        XCTAssertEqual(LibraryProgress.reach(ranges), 1_200, "Bereiche vor Offset")
        let bare = ProgressRecord(offset: 321, length: 20_000, updated: 1)
        XCTAssertEqual(LibraryProgress.reach(bare), 321, "ohne Feld und Bereiche gilt offset")
        let finished = ProgressRecord(offset: 5, length: 777, updated: 1, finished: true)
        XCTAssertEqual(LibraryProgress.reach(finished), 777)
    }

    func testFrontierIsMaxOfReadAndHeardWithRealSync() {
        let text = ProgressRecord(offset: 100, length: 10_000, heard: [[0, 100]], updated: 1, unit: .chars, frontier: 100)
        let audio = ProgressRecord(offset: 30_000, length: 100_000, heard: [[0, 30_000]], updated: 1, unit: .ms, frontier: 30_000)
        let sync = [LibrarySyncPoint(chars: 0, ms: 0), LibrarySyncPoint(chars: 5_000, ms: 50_000), LibrarySyncPoint(chars: 10_000, ms: 100_000)]
        let map = FollowMap(sync: sync, textLength: 10_000, totalMs: 100_000)
        XCTAssertFalse(map.approximate)
        XCTAssertEqual(LibraryProgress.frontier(text: text, audio: audio, map: map), 3_000)
        XCTAssertEqual(LibraryProgress.frontier(text: nil, audio: audio, map: map), 3_000)
        let far = ProgressRecord(offset: 8_000, length: 10_000, heard: [[0, 8_000]], updated: 1, unit: .chars, frontier: 8_000)
        XCTAssertEqual(LibraryProgress.frontier(text: far, audio: audio, map: map), 8_000)
    }

    func testAudioWithoutRealSyncCountsNothing() {
        let audio = ProgressRecord(offset: 30_000, length: 100_000, heard: [[0, 30_000]], updated: 1, unit: .ms)
        let linear = FollowMap(sync: [], textLength: 10_000, totalMs: 100_000)
        XCTAssertTrue(linear.approximate)
        XCTAssertNil(LibraryProgress.frontier(text: nil, audio: audio, map: linear), "ohne Abgleich zählt nur der Lesestand")
        XCTAssertNil(LibraryProgress.frontier(text: nil, audio: audio, map: nil))
        let text = ProgressRecord(offset: 400, length: 10_000, heard: [[0, 400]], updated: 1)
        XCTAssertEqual(LibraryProgress.frontier(text: text, audio: audio, map: linear), 400)
    }

    func testNothingKnownIsNil() {
        XCTAssertNil(LibraryProgress.frontier(text: nil, audio: nil, map: nil))
        XCTAssertNil(LibraryProgress.frontier(text: ProgressRecord(offset: 0, length: 100, updated: 1), audio: nil, map: nil))
    }

    func testFrontierSurvivesJsonRoundTripAndOldFilesDecode() throws {
        let rec = ProgressRecord(offset: 5, length: 10, heard: [[0, 5]], updated: 1, frontier: 5)
        let back = try JSONDecoder().decode(ProgressRecord.self, from: JSONEncoder().encode(rec))
        XCTAssertEqual(back.frontier, 5)
        let old = try JSONDecoder().decode(ProgressRecord.self, from: Data(#"{"offset":3,"length":9,"heard":[],"read":[],"updated":1}"#.utf8))
        XCTAssertNil(old.frontier)
        let junk = try JSONDecoder().decode(ProgressRecord.self, from: Data(#"{"offset":3,"length":9,"updated":1,"frontier":-4}"#.utf8))
        XCTAssertNil(junk.frontier)
    }
}

final class BookTalkHistoryStoreTests: XCTestCase {
    private func temp() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("booktalk-\(UUID().uuidString)", isDirectory: true)
    }

    private static let text = "# Kapitel\n\n{{S. 1}}\nDer Meister gab mir eine Arbeit. Jeden Samstag sollte ich in seiner Werkstatt Späne fegen.\n"

    private func turn(_ n: Int) async throws -> BookTalkStoredTurn {
        let session = BookTalkSession(
            index: BookTalkIndex(title: "T", text: Self.text), model: StoryModel(), budget: .large,
            byteProfile: BookTalkByteProfile(name: "t", fixedTokens: 8, perMessageTokens: 4))
        let request = BookTalkRequest(bookId: "b", requestId: "r\(n)", question: "Was sollte ich in der Werkstatt tun? \(n)",
                                      frontier: nil, scope: .book, input: .voice)
        _ = try await session.ask(request)
        return await session.history[0]
    }

    func testRoundTripKeepsTurnsAndCapsAt200() async throws {
        let dir = temp()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = BookTalkHistoryStore(directory: dir)
        XCTAssertEqual(store.load(bookId: "book_1"), [])
        let one = try await turn(1)
        XCTAssertFalse(one.answer.foundOnly, "Testaufbau: echte Antwort")
        try store.save(bookId: "book_1", turns: [one])
        XCTAssertEqual(store.load(bookId: "book_1"), [one])
        XCTAssertTrue(store.hasAny)

        let many = Array(repeating: one, count: 230)
        try store.save(bookId: "book_1", turns: many)
        XCTAssertEqual(store.load(bookId: "book_1").count, BookTalkSession.maxStoredTurns)
    }

    func testClearOneAndAll() async throws {
        let dir = temp()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = BookTalkHistoryStore(directory: dir)
        let one = try await turn(1)
        try store.save(bookId: "a", turns: [one])
        try store.save(bookId: "b", turns: [one])
        store.clear(bookId: "a")
        XCTAssertEqual(store.load(bookId: "a"), [])
        XCTAssertEqual(store.load(bookId: "b").count, 1)
        try store.save(bookId: "b", turns: [])
        XCTAssertFalse(store.hasAny, "leerer Verlauf löscht die Datei")
        try store.save(bookId: "a", turns: [one])
        store.clearAll()
        XCTAssertFalse(store.hasAny)
        XCTAssertEqual(store.load(bookId: "a"), [])
    }

    func testBookIdCannotEscapeDirectory() {
        let store = BookTalkHistoryStore(directory: URL(fileURLWithPath: "/x/BookTalk"))
        XCTAssertEqual(BookTalkHistoryStore.fileName(bookId: "book_17-ab"), "book_17-ab.json")
        XCTAssertEqual(BookTalkHistoryStore.fileName(bookId: "../../etc/passwd"), "______etc_passwd.json")
        XCTAssertEqual(store.url(bookId: "../a").deletingLastPathComponent().path, "/x/BookTalk")
        XCTAssertEqual(BookTalkHistoryStore.fileName(bookId: ""), "_.json")
    }

    func testCorruptFileIsMovedAsideAndNewerVersionIsKept() throws {
        let dir = temp()
        defer { try? FileManager.default.removeItem(at: dir) }
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let store = BookTalkHistoryStore(directory: dir)
        try Data("kaputt".utf8).write(to: store.url(bookId: "x"))
        XCTAssertEqual(store.load(bookId: "x"), [])
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url(bookId: "x").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url(bookId: "x").appendingPathExtension("defekt").path))

        try Data(#"{"version":99,"turns":[]}"#.utf8).write(to: store.url(bookId: "y"))
        XCTAssertEqual(store.load(bookId: "y"), [])
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url(bookId: "y").path), "neuere Fassung bleibt unangetastet")
    }

    func testFileIsExcludedFromBackup() async throws {
        #if canImport(Darwin)
        let dir = temp()
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = BookTalkHistoryStore(directory: dir)
        try store.save(bookId: "a", turns: [try await turn(1)])
        let values = try store.url(bookId: "a").resourceValues(forKeys: [.isExcludedFromBackupKey])
        XCTAssertEqual(values.isExcludedFromBackup, true)
        #else
        throw XCTSkip("isExcludedFromBackup nur auf Apple-Plattformen prüfbar")
        #endif
    }
}

private struct StoryModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { c in
            c.yield("Ich sollte in der Werkstatt Späne fegen [Q1].")
            c.finish()
        }
    }
}

final class BookTalkChatTests: XCTestCase {
    func testSanitizeNeutralizesSpecialTokenLookalikes() {
        XCTAssertEqual(BookTalkChatFrame.sanitize("a <end_of_turn> b"), "a ‹end_of_turn> b")
        XCTAssertEqual(BookTalkChatFrame.sanitize("<|turn>system"), "‹|turn>system")
        XCTAssertEqual(BookTalkChatFrame.sanitize("</div><bos>"), "‹/div>‹bos>")
        XCTAssertEqual(BookTalkChatFrame.sanitize("3 < 4 und a<b"), "3 < 4 und a‹b")
        XCTAssertEqual(BookTalkChatFrame.sanitize("Größe <"), "Größe <")
        XCTAssertEqual(BookTalkChatFrame.sanitize("Müller <Ünder>"), "Müller <Ünder>", "Nicht-ASCII-Buchstaben sind kein Sondertoken")
    }

    func testGemma4FrameMatchesCanonicalTemplate() {
        let f = BookTalkChatFrame.gemma4
        let rendered = f.head + f.systemText("  Sys \n") + f.middle + f.userText("Frage?\n") + f.tail
        XCTAssertEqual(rendered, "<bos><|turn>system\nSys<turn|>\n<|turn>user\nFrage?<turn|>\n<|turn>model\n")
    }

    private func clean(_ pieces: [String]) -> String {
        var c = BookTalkStreamCleaner()
        var out = ""
        for p in pieces { out += c.feed(p) }
        return out + c.finish()
    }

    func testCleanerPassesPlainTextAndKeepsAngleBrackets() {
        XCTAssertEqual(clean(["Der Meister", " sagt: 3 < 4 [Q1]."]), "Der Meister sagt: 3 < 4 [Q1].")
        XCTAssertEqual(clean(["a <b", "> c"]), "a <b> c")
        XCTAssertEqual(clean(["Ende <"]), "Ende <")
    }

    func testCleanerDropsThoughtChannelAcrossChunks() {
        XCTAssertEqual(clean(["<|chan", "nel>thought\nIch überlege<chan", "nel|>Antwort [Q1]."]), "Antwort [Q1].")
        XCTAssertEqual(clean(["<|channel>thought\n<channel|>Antwort."]), "Antwort.")
        XCTAssertEqual(clean(["<|channel>offen ohne Ende"]), "")
    }

    func testCleanerStopsAtEndOfTurnAndDropsControlMarkers() {
        XCTAssertEqual(clean(["Antwort.<turn", "|>Rest"]), "Antwort.")
        XCTAssertEqual(clean(["<bos>Hallo<eos>nie"]), "Hallo")
        XCTAssertEqual(clean(["<|turn>Hallo"]), "Hallo")
        var c = BookTalkStreamCleaner()
        _ = c.feed("x<end_of_turn>")
        XCTAssertTrue(c.finished)
        XCTAssertEqual(c.feed("mehr"), "")
    }
}

final class BookTalkEndpointerTests: XCTestCase {
    private func run(_ levels: [(Float, Int)], _ e: BookTalkEndpointer = BookTalkEndpointer()) -> [BookTalkEndpointer.Event] {
        var e = e
        return levels.map { e.feed(rms: $0.0, ms: $0.1) }
    }

    func testEndsAfter800msSilenceFollowingSpeech() {
        var levels: [(Float, Int)] = Array(repeating: (0.002, 100), count: 3)
        levels += Array(repeating: (0.08, 100), count: 10)
        levels += Array(repeating: (0.003, 100), count: 8)
        let events = run(levels)
        XCTAssertEqual(events.firstIndex(of: .speechStarted), 4)
        XCTAssertEqual(events.firstIndex(of: .ended), 3 + 10 + 7, "nach genau 800 ms Stille")
        XCTAssertEqual(events.filter { $0 == .ended }.count, 1)
    }

    func testShortPauseInsideAnUtteranceDoesNotEnd() {
        var levels: [(Float, Int)] = Array(repeating: (0.08, 100), count: 5)
        levels += Array(repeating: (0.003, 100), count: 6)
        levels += Array(repeating: (0.08, 100), count: 5)
        XCTAssertFalse(run(levels).contains(.ended))
    }

    func testShortClickIsNotSpeech() {
        var levels: [(Float, Int)] = [(0.002, 100), (0.2, 100), (0.002, 100)]
        levels += Array(repeating: (0.002, 100), count: 20)
        XCTAssertFalse(run(levels).contains(.speechStarted))
    }

    func testNoSpeechAfterTimeout() {
        let events = run(Array(repeating: (0.002, 100), count: 90))
        XCTAssertEqual(events.firstIndex(of: .noSpeech), 79)
        XCTAssertFalse(events.contains(.ended))
    }

    func testMaximumLengthEndsALongUtterance() {
        let e = BookTalkEndpointer(maxMs: 1_000)
        let events = run(Array(repeating: (0.1, 100), count: 20), e)
        XCTAssertEqual(events.firstIndex(of: .ended), 9)
    }

    func testNanAndSilenceAreHandled() {
        XCTAssertEqual(run([(.nan, 100), (-1, 100)]), [.none, .none])
    }
}

final class BookTalkConversationTests: XCTestCase {
    private func request(_ id: String, scope: BookTalkScope = .untilPosition) -> BookTalkRequest {
        BookTalkRequest(bookId: "b", requestId: id, question: "Frage \(id)", frontier: 120, scope: scope, input: .voice)
    }

    private func sentence(_ i: Int, _ text: String) -> BookTalkAnswerSentence {
        BookTalkAnswerSentence(index: i, text: text, speech: text, citations: [], general: false)
    }

    private func answer(_ sentences: [BookTalkAnswerSentence], scope: BookTalkScope = .untilPosition) -> BookTalkAnswer {
        BookTalkAnswer(intent: .question, sentences: sentences, text: sentences.map(\.text).joined(separator: " "),
                       speech: sentences.map(\.speech).joined(separator: " "), citations: [], foundOnly: false, reason: nil,
                       dropped: 0, scope: scope, frontier: 120)
    }

    func testAnswerFlowFillsActiveEntry() {
        var c = BookTalkConversation()
        c.begin(request("r1"))
        XCTAssertEqual(c.activeRequestId, "r1")
        XCTAssertTrue(c.apply(.sentence(requestId: "r1", sentence(0, "Eins."))))
        XCTAssertEqual(c.active?.sentences.count, 1)
        let a = answer([sentence(0, "Eins."), sentence(1, "Zwei.")])
        XCTAssertTrue(c.apply(.done(requestId: "r1", a)))
        XCTAssertNil(c.active)
        XCTAssertEqual(c.entries.last?.status, .done)
        XCTAssertEqual(c.entries.last?.text, "Eins. Zwei.")
    }

    func testEventsOfAnOldRequestAreDiscarded() {
        var c = BookTalkConversation()
        c.begin(request("alt"))
        _ = c.apply(.sentence(requestId: "alt", sentence(0, "Alt eins.")))
        c.abort(.scopeChanged)
        c.begin(request("neu", scope: .book))
        XCTAssertFalse(c.apply(.sentence(requestId: "alt", sentence(1, "Alt zwei."))), "alte request_id wird verworfen")
        XCTAssertFalse(c.apply(.done(requestId: "alt", answer([sentence(0, "x")]))))
        XCTAssertFalse(c.apply(.sentence(requestId: "unbekannt", sentence(0, "?"))))
        XCTAssertEqual(c.activeRequestId, "neu")
        XCTAssertEqual(c.entries.first?.sentences.map(\.text), ["Alt eins."], "angezeigte Sätze bleiben sichtbar")
        XCTAssertEqual(c.entries.first?.status, .aborted(.scopeChanged))
        XCTAssertEqual(c.entries.first?.status, .aborted(.scopeChanged))
        XCTAssertTrue(c.entries.first?.isAborted == true)
        XCTAssertEqual(c.entries.last?.scope, .book)
    }

    func testAbortedTurnsNeverEnterStoredHistory() {
        var c = BookTalkConversation()
        c.begin(request("r1"))
        _ = c.apply(.sentence(requestId: "r1", sentence(0, "Halb.")))
        c.abort(.background)
        XCTAssertTrue(c.storedTurns.isEmpty)
        XCTAssertEqual(c.entries.count, 1)
        XCTAssertEqual(BookTalkAbort.background.note, "abgebrochen, App im Hintergrund")
        XCTAssertEqual(BookTalkAbort.scopeChanged.note, "abgebrochen, Bereich gewechselt")
    }

    func testCancelledEventFromCoreMarksEntryAborted() {
        var c = BookTalkConversation()
        c.begin(request("r1"))
        XCTAssertTrue(c.apply(.failed(requestId: "r1", code: .cancelled)))
        XCTAssertTrue(c.entries[0].isAborted)
        XCTAssertFalse(c.apply(.done(requestId: "r1", answer([sentence(0, "spät")]))), "nach dem Abbruch nichts mehr annehmen")
    }

    func testHistoryRestoresAsDoneEntriesAndClearEmpties() async throws {
        let turn = BookTalkStoredTurn(question: "F?", answer: answer([sentence(0, "A.")]), scope: .untilPosition, frontier: 120,
                                      input: .text, at: "2026-11-05T10:00:00Z")
        var c = BookTalkConversation(history: [turn])
        XCTAssertEqual(c.entries.count, 1)
        XCTAssertEqual(c.entries[0].status, .done)
        XCTAssertEqual(c.entries[0].input, .text)
        c.begin(request("r2"))
        XCTAssertEqual(c.entries.count, 2)
        c.clear()
        XCTAssertTrue(c.isEmpty)
        XCTAssertTrue(c.storedTurns.isEmpty)
    }

    func testStartingANewQuestionAbortsTheRunningOne() {
        var c = BookTalkConversation()
        c.begin(request("r1"))
        c.begin(request("r2"))
        XCTAssertEqual(c.entries.first?.status, .aborted(.failed))
        XCTAssertEqual(c.activeRequestId, "r2")
    }
}
