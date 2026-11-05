import XCTest
@testable import VoiceCore

final class AudioDownloadTests: XCTestCase {
    private let chunk = 4
    private typealias Key = AudioDownload.ChunkKey
    private func job() -> AudioDownload.Job {
        AudioDownload.Job(bookId: "book_1", parts: [
            .init(blob: "a", size: 10, chunks: 3, file: "teil_001.m4a"),  // 4 + 4 + 2
            .init(blob: "b", size: 4, chunks: 1, file: "teil_002.m4a"),
        ])
    }
    private let fresh = [AudioDownload.PartState(complete: false, partialBytes: 0), .init(complete: false, partialBytes: 0)]

    func testMissingChunksInListeningOrderWithinWindow() {
        let all = AudioDownload.missingChunks(job(), states: fresh, staged: [], inFlight: [], chunkPlainBytes: chunk)
        XCTAssertEqual(all, [Key(part: 0, index: 0), Key(part: 0, index: 1), Key(part: 0, index: 2), Key(part: 1, index: 0)])
        let limited = AudioDownload.missingChunks(job(), states: fresh, staged: [Key(part: 0, index: 1)],
                                                  inFlight: [Key(part: 0, index: 0)], chunkPlainBytes: chunk, limit: 3)
        XCTAssertEqual(limited, [Key(part: 0, index: 2)])
        XCTAssertEqual(AudioDownload.missingChunks(job(), states: fresh, staged: [], inFlight: [Key(part: 0, index: 0)],
                                                   chunkPlainBytes: chunk, limit: 1), [])
    }

    func testResumesAfterPartialAndSkipsCompleteParts() {
        let states = [AudioDownload.PartState(complete: false, partialBytes: 8), .init(complete: true, partialBytes: 0)]
        XCTAssertEqual(AudioDownload.missingChunks(job(), states: states, staged: [], inFlight: [], chunkPlainBytes: chunk),
                       [Key(part: 0, index: 2)])
        XCTAssertEqual(AudioDownload.doneBytes(job(), states: states), 12)
        XCTAssertFalse(AudioDownload.isComplete(job(), states: states))
        XCTAssertTrue(AudioDownload.isComplete(job(), states: [.init(complete: true, partialBytes: 0), .init(complete: true, partialBytes: 0)]))
    }

    func testPartialBytesBeyondChunkBoundaryCountWholeChunksOnly() {
        // 6 Bytes = 1 volles Stück + Rest: Stück 1 wird neu geschrieben (truncate).
        XCTAssertEqual(AudioDownload.nextChunk(job().parts[0], .init(complete: false, partialBytes: 6), chunkPlainBytes: chunk), 1)
        XCTAssertNil(AudioDownload.nextChunk(job().parts[0], .init(complete: true, partialBytes: 0), chunkPlainBytes: chunk))
    }

    func testTornSingleChunkLargerThanChunkSizeIsFetchedAgain() {
        let part = AudioDownload.Part(blob: "c", size: 7, chunks: 1, file: "teil_003.m4a")
        XCTAssertEqual(AudioDownload.nextChunk(part, .init(complete: false, partialBytes: 5), chunkPlainBytes: chunk), 0)
        XCTAssertNil(AudioDownload.nextChunk(part, .init(complete: false, partialBytes: 7), chunkPlainBytes: chunk))
    }

    func testLegacyBlobBetweenOneChunkLimitsAsksForSecondChunkOnlyIfNeeded() {
        let size = LibrarySyncFormat.oneChunkMaxBytes + 1000
        let plain = LibrarySyncFormat.chunkPlainBytes
        let part = AudioDownload.Part(blob: "d", size: size, chunks: 2, file: "teil_004.m4a")
        let job = AudioDownload.Job(bookId: "b", parts: [part])
        func missing(_ partial: Int) -> [Int] {
            AudioDownload.missingChunks(job, states: [.init(complete: false, partialBytes: partial)], staged: [], inFlight: [], chunkPlainBytes: plain).map(\.index)
        }
        XCTAssertEqual(missing(0), [0], "nicht gleich Stück 1 anfordern: Am Hub gibt es vielleicht nur eins")
        XCTAssertEqual(missing(size), [], "Alt-Blob: ein Stück reicht")
        XCTAssertEqual(missing(plain), [1], "neues Format: Stück 0 war 3 MiB")
        XCTAssertEqual(missing(plain + 5), [0], "abgerissen: von vorn")
        let legacyRef = AudioDownload.Part(blob: "e", size: LibrarySyncFormat.legacyOneChunkMaxBytes, chunks: 1, file: "teil_005.m4a")
        XCTAssertEqual(AudioDownload.nextChunk(legacyRef, .init(complete: false, partialBytes: 0), chunkPlainBytes: plain), 0)
        XCTAssertNil(AudioDownload.nextChunk(legacyRef, .init(complete: false, partialBytes: legacyRef.size), chunkPlainBytes: plain))
    }

    func testSpaceProblem() {
        let fmt: (Int64) -> String = { "\($0) B" }
        XCTAssertNil(AudioDownload.spaceProblem(remainingBytes: 100, freeBytes: nil, format: fmt))
        XCTAssertNil(AudioDownload.spaceProblem(remainingBytes: 100, freeBytes: Int64(AudioDownload.reserveBytes) + 100, format: fmt))
        let msg = AudioDownload.spaceProblem(remainingBytes: 100, freeBytes: 50, format: fmt)
        XCTAssertEqual(msg?.hasPrefix("Zu wenig Speicher: Das Hörbuch braucht noch 100 B, frei sind 50 B."), true)
    }

    func testTaskAndStagedNamesRoundTrip() {
        let key = Key(part: 2, index: 17)
        let desc = AudioDownload.taskDescription(bookId: "book_1_abc", key)
        XCTAssertEqual(AudioDownload.parseTask(desc)?.bookId, "book_1_abc")
        XCTAssertEqual(AudioDownload.parseTask(desc)?.key, key)
        XCTAssertNil(AudioDownload.parseTask("qwen-0.5b"))
        XCTAssertNil(AudioDownload.parseTask(nil))
        XCTAssertEqual(AudioDownload.parseStaged(AudioDownload.stagedName(key)), key)
        XCTAssertNil(AudioDownload.parseStaged("teil_001.m4a.part"))
    }

    func testJobCodableKeepsPhaseAndCellular() throws {
        var j = job(); j.phase = .paused; j.allowCellular = true; j.message = "x"
        let back = try JSONDecoder().decode(AudioDownload.Job.self, from: JSONEncoder().encode(j))
        XCTAssertEqual(back, j)
        XCTAssertEqual(back.totalBytes, 14)
    }
}
