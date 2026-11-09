import XCTest
@testable import VoiceCore

final class AudioOffloadTests: XCTestCase {
    private let a = AudioOffload.AccountFile(blob: String(repeating: "a", count: 64), size: 10)
    private let b = AudioOffload.AccountFile(blob: String(repeating: "b", count: 64), size: 20)

    private func verdict(loggedIn: Bool = true, synced: Bool = true, downloading: Bool = false,
                         local: [Int?], leftovers: Bool = false,
                         account: [AudioOffload.AccountFile], confirmed: Set<String>) -> AudioOffload.Verdict {
        AudioOffload.verdict(loggedIn: loggedIn, synced: synced, downloading: downloading, localSizes: local,
                             hasLeftovers: leftovers, account: account, confirmed: confirmed)
    }

    func testAllowedOnlyWhenEveryPartIsConfirmed() {
        XCTAssertEqual(verdict(local: [10, 20], account: [a, b], confirmed: [a.blob, b.blob]), .allowed)
        XCTAssertEqual(verdict(local: [10, 20], account: [a, b], confirmed: [a.blob]), .unconfirmed(missing: 1))
        XCTAssertEqual(verdict(local: [10, 20], account: [a, b], confirmed: []), .unconfirmed(missing: 2))
    }

    func testOwnImportNotInAccountIsKept() {
        XCTAssertEqual(verdict(synced: false, local: [10], account: [], confirmed: []), .notInAccount)
        XCTAssertEqual(verdict(synced: true, local: [10], account: [], confirmed: []), .notInAccount)
        XCTAssertEqual(verdict(loggedIn: false, local: [10], account: [a], confirmed: [a.blob]), .notLoggedIn)
        XCTAssertTrue(AudioOffload.Verdict.notInAccount.syncHelps)
        XCTAssertFalse(AudioOffload.Verdict.notLoggedIn.syncHelps)
    }

    func testMismatchedPartsAreKept() {
        // Ein Teil mehr auf dem Gerät als im Konto.
        XCTAssertEqual(verdict(local: [10, 20], account: [a], confirmed: [a.blob]), .differs)
        // Lokale Datei anders groß als im Konto (z. B. neu importiert).
        XCTAssertEqual(verdict(local: [11, 20], account: [a, b], confirmed: [a.blob, b.blob]), .differs)
    }

    func testPartialDownloadMayBeRemoved() {
        // Teil 2 fehlt noch, ein .part liegt daneben: alles kommt aus dem Konto.
        XCTAssertEqual(verdict(local: [10, nil], leftovers: true, account: [a, b], confirmed: [a.blob, b.blob]), .allowed)
        XCTAssertEqual(verdict(local: [nil, nil], leftovers: true, account: [a, b], confirmed: [a.blob, b.blob]), .allowed)
    }

    func testNothingLocalAndDownloading() {
        XCTAssertEqual(verdict(local: [nil, nil], account: [a, b], confirmed: [a.blob, b.blob]), .nothingLocal)
        XCTAssertEqual(verdict(downloading: true, local: [10, 20], account: [a, b], confirmed: [a.blob, b.blob]), .downloading)
    }

    func testBlobsToCheckWithoutDuplicates() {
        XCTAssertEqual(AudioOffload.blobsToCheck([a, b, a]), [a.blob, b.blob])
    }

    func testMessagesAreGerman() {
        XCTAssertTrue(AudioOffload.Verdict.unconfirmed(missing: 1).message.hasPrefix("Ein Teil"))
        XCTAssertTrue(AudioOffload.Verdict.unconfirmed(missing: 3).message.hasPrefix("3 Teile"))
        XCTAssertTrue(AudioOffload.Verdict.notInAccount.message.contains("nur auf diesem iPhone"))
    }

    func testRemoveDeletesAudioPartAndStagingButKeepsBook() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("offload-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: dir) }
        let audio = dir.appendingPathComponent("audio", isDirectory: true)
        let staging = dir.appendingPathComponent("audio.neu-1234", isDirectory: true)
        try fm.createDirectory(at: audio, withIntermediateDirectories: true)
        try fm.createDirectory(at: staging, withIntermediateDirectories: true)
        try Data(count: 100).write(to: audio.appendingPathComponent("teil_001.m4a"))
        try Data(count: 30).write(to: audio.appendingPathComponent("teil_002.m4a.part"))
        try Data(count: 7).write(to: staging.appendingPathComponent("teil_001.mp3"))
        for name in ["buch.json", "text.txt", "sync.json"] { try Data(count: 5).write(to: dir.appendingPathComponent(name)) }

        XCTAssertTrue(AudioOffload.hasLeftovers(in: dir))
        XCTAssertEqual(AudioOffload.bytes(at: audio), 130)
        XCTAssertEqual(try AudioOffload.remove(bookDir: dir), 137)
        XCTAssertFalse(fm.fileExists(atPath: audio.path))
        XCTAssertFalse(fm.fileExists(atPath: staging.path))
        for name in ["buch.json", "text.txt", "sync.json"] { XCTAssertTrue(fm.fileExists(atPath: dir.appendingPathComponent(name).path)) }
        XCTAssertFalse(AudioOffload.hasLeftovers(in: dir))
        XCTAssertEqual(try AudioOffload.remove(bookDir: dir), 0)
    }

    /// Entfernen und Abbrechen eines Downloads löschen nur `audio/`: das Cover
    /// (`cover.jpg` + `cover.blob` aus dem Konto) bleibt im Buchordner.
    func testRemoveKeepsCoverAndDropsDownloadPieces() throws {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("offload-\(UUID().uuidString)", isDirectory: true)
        defer { try? fm.removeItem(at: dir) }
        let audio = dir.appendingPathComponent(AudioOffload.audioFolder, isDirectory: true)
        let pieces = audio.appendingPathComponent(AudioDownload.stagingFolder, isDirectory: true)
        try fm.createDirectory(at: pieces, withIntermediateDirectories: true)
        try Data(count: 64).write(to: audio.appendingPathComponent("teil_001.m4a"))
        try Data(count: 16).write(to: pieces.appendingPathComponent(AudioDownload.stagedName(.init(part: 0, index: 3))))
        let cover = Data([0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3])
        try cover.write(to: dir.appendingPathComponent("cover.jpg"))
        try Data("abc".utf8).write(to: dir.appendingPathComponent("cover.blob"))

        XCTAssertEqual(try AudioOffload.remove(bookDir: dir), 80)
        XCTAssertFalse(fm.fileExists(atPath: audio.path))
        XCTAssertEqual(try Data(contentsOf: dir.appendingPathComponent("cover.jpg")), cover)
        XCTAssertEqual(try String(contentsOf: dir.appendingPathComponent("cover.blob"), encoding: .utf8), "abc")
    }
}

final class AudioStorageTests: XCTestCase {
    private func item(_ id: String, _ bytes: Int, fraction: Double = 0, finished: Bool = false,
                      partial: Bool = false, removable: Bool = true) -> AudioStorage.Item {
        AudioStorage.Item(id: id, title: id, bytes: bytes, fraction: fraction, finished: finished,
                          lastUsedMs: nil, partial: partial, removable: removable)
    }

    func testSortedBySizeThenTitle() {
        let list = AudioStorage.sorted([item("b", 10), item("a", 10), item("c", 500)])
        XCTAssertEqual(list.map(\.id), ["c", "a", "b"])
        XCTAssertEqual(AudioStorage.total(list), 520)
    }

    func testFinishedRemovableSkipsLocalOnlyAndUnfinished() {
        let list = [item("fertig", 10, finished: true), item("eigen", 99, finished: true, removable: false),
                    item("laufend", 50, fraction: 0.4), item("gross", 80, finished: true)]
        XCTAssertEqual(AudioStorage.finishedRemovable(list).map(\.id), ["gross", "fertig"])
    }

    func testStandInWords() {
        XCTAssertEqual(item("x", 1, finished: true).stand, "Fertig gehört")
        XCTAssertEqual(item("x", 1, fraction: 0.454).stand, "45\u{00A0}% gehört")
        XCTAssertEqual(item("x", 1).stand, "Noch nicht gehört")
        XCTAssertEqual(item("x", 1, partial: true).stand, "Download unvollständig")
        XCTAssertEqual(item("x", 1, fraction: 0.2, partial: true).stand, "20\u{00A0}% gehört · Download unvollständig")
    }

    func testSummary() {
        XCTAssertEqual(AudioStorage.summary(removed: 1, freedText: "1 GB", kept: 0), "1 Hörbuch entfernt, 1 GB frei.")
        XCTAssertEqual(AudioStorage.summary(removed: 3, freedText: "2 GB", kept: 1),
                       "3 Hörbücher entfernt, 2 GB frei. 1 Hörbuch bleibt, weil das Konto es nicht als vollständig bestätigt hat.")
        XCTAssertEqual(AudioStorage.summary(removed: 0, freedText: "0 KB", kept: 2),
                       "Nichts entfernt. 2 Hörbücher bleiben, weil das Konto sie nicht als vollständig bestätigt hat.")
    }
}
