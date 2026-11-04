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
}
