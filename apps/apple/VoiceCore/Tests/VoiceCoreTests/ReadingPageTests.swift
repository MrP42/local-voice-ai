import XCTest
@testable import VoiceCore

final class ReadingPageTests: XCTestCase {
    // Aus collect.rs, TtsSettings.tsx und pasteLog.ts abgeleitet; keine Nutzerdaten.
    private let desktop = #"{"title":"Probe","state":{"text":"Original","summary":"Kurz","translation":"Text","tab":"original","sourceUrl":"","kiDoc":null,"voices":{"de":"synthetic"},"einfuegungen":[{"zeit":"2026-11-25T00:00:00Z","sha256":"synthetic","roh":"Roh","bereinigt":"Original","regeln":[]}],"future":{"integer":18446744073709551615,"float":1.0,"empty":null,"array":[true,null]}}}"#

    func testDesktopRoundTripPreservesUnknownFieldsAndVersions() throws {
        let raw = Data(desktop.utf8)
        var page = try ReadingPage(id: "page_1", json: raw)
        XCTAssertEqual(try AblageJSON.parse([UInt8](page.json())), try AblageJSON.parse([UInt8](raw)))
        page.text = "Bearbeitet"
        let back = try ReadingPage(id: page.id, json: page.json())
        XCTAssertEqual(back.text, "Bearbeitet")
        XCTAssertEqual(back.state["future"], page.state["future"])
        XCTAssertEqual(back.state["einfuegungen"], page.state["einfuegungen"])
        XCTAssertEqual(back.summary, "Kurz")
        XCTAssertEqual(back.translation, "Text")
    }

    func testNullStateAndMissingFieldsAreNotInvented() throws {
        let page = try ReadingPage(id: "page_2", json: Data(#"{"title":"Leer","state":null,"future":42}"#.utf8))
        XCTAssertEqual(page.state, .null)
        XCTAssertEqual(page.text, "")
        XCTAssertEqual(try AblageJSON.parse([UInt8](page.json()))["future"], .uint(42))
    }

    func testEssenceMatchesDesktopIncludingMissingVersusNull() throws {
        let a = try ReadingPage(id: "page_1", json: Data(#"{"title":"T","state":{"text":"x","tab":"original"}}"#.utf8))
        var b = try ReadingPage(id: "page_1", json: Data(#"{"title":"T","state":{"text":"x","tab":"summary","sourceUrl":"u"}}"#.utf8))
        XCTAssertFalse(a.differs(from: b))
        b.translation = "Neu"
        XCTAssertTrue(a.differs(from: b))
        b = try ReadingPage(id: "page_1", json: Data(#"{"title":"T","state":{"text":"x","summary":null}}"#.utf8))
        XCTAssertFalse(a.differs(from: b))
    }

    func testPathsAndPayloadLimits() throws {
        for id in ["../page_1", "page_a/b", "page_a_b", "page_", "page_ä"] {
            XCTAssertThrowsError(try ReadingPage(id: id, title: "T"))
        }
        XCTAssertNoThrow(try ReadingPage(id: "page_A19", title: "T"))
        XCTAssertEqual(ReadingPage.maxPlaintext, 3 * 1024 * 1024)
        XCTAssertThrowsError(try ReadingPage(id: "page_1", json: Data(repeating: 32, count: ReadingPage.maxPlaintext + 1)))
    }

    func testPagePayloadUsesExistingEncryptionAndCannotBecomeBook() throws {
        let page = try ReadingPage(id: "page_1", json: Data(desktop.utf8))
        let key = [UInt8](repeating: 7, count: 32)
        let aad = SyncCrypto.aad(userId: 1, collection: ReadingPage.collection, objectId: page.id)
        let sealed = try SyncCrypto.seal(key: key, aad: aad, plaintext: [UInt8](page.json()), nonce: [UInt8](repeating: 3, count: 24))
        let opened = try SyncCrypto.open(key: key, aad: aad, sealedBase64: sealed)
        XCTAssertEqual(try ReadingPage(id: page.id, json: Data(opened)), page)
        XCTAssertThrowsError(try SyncCrypto.open(key: key,
            aad: SyncCrypto.aad(userId: 1, collection: "book", objectId: page.id), sealedBase64: sealed))
    }
}

@MainActor
final class ReadingPageStoreTests: XCTestCase {
    private func store() throws -> ReadingPageStore {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return try ReadingPageStore(root: root)
    }

    private func page(_ text: String = "Original") throws -> ReadingPage {
        var p = try ReadingPage(id: "page_1", title: "Probe")
        p.text = text
        return p
    }

    func testTwoDevicesConflictCopyAndEchoSurviveRestart() throws {
        let a = try store(), b = try store()
        try a.save(page())
        let first = try XCTUnwrap(a.pending().first)
        try a.acknowledge(first, revision: 1)
        try b.apply(id: first.id, json: first.json, revision: 1, deviceName: "B")
        XCTAssertTrue(try b.pending().isEmpty)
        try a.save(page("A")); try b.save(page("B"))
        let sent = try XCTUnwrap(a.pending().first)
        try a.acknowledge(sent, revision: 2)
        try b.apply(id: sent.id, json: sent.json, revision: 2, deviceName: "B")
        let pages = try b.pages()
        XCTAssertEqual(pages.count, 2)
        XCTAssertEqual(pages.first { $0.id == "page_1" }?.text, "A")
        XCTAssertEqual(pages.first { $0.id != "page_1" }?.text, "B")
        XCTAssertEqual(pages.first { $0.id != "page_1" }?.title, "Probe (Konflikt von B)")
        try b.apply(id: sent.id, json: sent.json, revision: 2, deviceName: "B")
        let reopened = try ReadingPageStore(root: b.root)
        XCTAssertEqual(try reopened.pages().count, 2)
        XCTAssertEqual(try reopened.pending().count, 1) // Nur die Konfliktkopie.
    }

    func testLateAcknowledgementDoesNotSwallowNewEdit() throws {
        let s = try store()
        try s.save(page())
        let sent = try XCTUnwrap(s.pending().first)
        try s.save(page("Während der Anfrage"))
        try s.acknowledge(sent, revision: 1)
        XCTAssertEqual(try s.pending().count, 1)
        XCTAssertEqual(try s.pending().first?.baseRevision, 1)
    }

    func testMissingServerObjectCanRetryFromZeroWithoutLosingText() throws {
        let s = try store()
        try s.save(page())
        try s.acknowledge(XCTUnwrap(s.pending().first), revision: 4)
        XCTAssertTrue(try s.pending().isEmpty)
        try s.resetRevision("page_1")
        XCTAssertEqual(try s.pending().first?.baseRevision, 0)
        XCTAssertEqual(try s.pages().first?.text, "Original")
    }

    func testSameEssenceDoesNotCreateConflictForDifferentTab() throws {
        let s = try store()
        var local = try page()
        try s.save(local)
        local.setStateField("tab", .string("summary"))
        try s.apply(id: local.id, json: local.json(), revision: 1, deviceName: "A")
        XCTAssertEqual(try s.pages().count, 1)
        XCTAssertTrue(try s.pending().isEmpty)
    }

    func testAccountSwitchDoesNotReuseRevisionOrDeleteLocalPages() throws {
        let s = try store()
        s.useAccount("accountA")
        try s.save(page())
        try s.acknowledge(XCTUnwrap(s.pending().first), revision: 9)
        XCTAssertTrue(try s.pending().isEmpty)
        s.useAccount("accountB")
        XCTAssertEqual(try s.pending().first?.baseRevision, 0)
        try s.apply(id: "page_1", json: page("B").json(), revision: 1, deviceName: "A")
        XCTAssertEqual(try s.pages().count, 2)
        XCTAssertEqual(try s.pages().first { $0.id == "page_1" }?.text, "B")
        XCTAssertEqual(try s.pages().first { $0.id != "page_1" }?.text, "Original")
        let reopened = try ReadingPageStore(root: s.root)
        reopened.useAccount("accountB")
        XCTAssertEqual(try reopened.pending().count, 1)
    }

    func testTombstoneRetainsCapturesAndDoesNotResurrectOnReplay() throws {
        let a = try store(), b = try store()
        try a.save(page())
        let first = try XCTUnwrap(a.pending().first)
        try a.acknowledge(first, revision: 1)
        try b.apply(id: first.id, json: first.json, revision: 1, deviceName: "B")
        let capture = ReadingCapture(image: "probe.jpg", recognizedText: "Original", source: "photo", createdMs: 1)
        try b.setCaptures([capture], for: "page_1", composedText: "Original")
        try a.delete("page_1")
        let tomb = try XCTUnwrap(a.pending().first)
        XCTAssertTrue(tomb.deleted)
        try b.apply(id: tomb.id, json: nil, revision: 2, deviceName: "B")
        try b.apply(id: first.id, json: first.json, revision: 1, deviceName: "B")
        XCTAssertTrue(try b.pages().isEmpty)
        XCTAssertTrue(try b.pending().isEmpty)
        XCTAssertEqual(try b.archivedPage("page_1")?.text, "Original")
        XCTAssertEqual(try b.captures(for: "page_1"), [capture])
        try a.acknowledge(tomb, revision: 2)
        XCTAssertTrue(try a.pending().isEmpty)
    }

    func testDirtyTextAgainstRemoteDeletionIsCopied() throws {
        let s = try store()
        try s.apply(id: "page_1", json: page().json(), revision: 1, deviceName: "A")
        try s.save(page("Ungesichert"))
        try s.apply(id: "page_1", json: nil, revision: 2, deviceName: "A")
        XCTAssertEqual(try s.pages().map(\.text), ["Ungesichert"])
        XCTAssertNotEqual(try s.pages().first?.id, "page_1")
    }

    func testRetryAfterConflictCopyWasCommittedDoesNotDuplicateIt() throws {
        let s = try store()
        try s.save(page("Lokal"))
        let target = s.root.appendingPathComponent("page_1.json")
        let before = try Data(contentsOf: target)
        let incoming = try page("Remote").json()
        try s.apply(id: "page_1", json: incoming, revision: 1, deviceName: "A")
        // Simuliert Abbruch zwischen dem Schreiben der Kopie und dem Original.
        try before.write(to: target, options: .atomic)
        try s.apply(id: "page_1", json: incoming, revision: 1, deviceName: "A")
        XCTAssertEqual(try s.pages().count, 2)
        XCTAssertEqual(try s.pages().first { $0.id != "page_1" }?.text, "Lokal")
    }

    func testFailedSaveKeepsOtherPagesAndProducesNoTombstone() throws {
        let s = try store()
        try s.save(page())
        // Ein Ordner statt der Zieldatei erzwingt einen echten Schreibfehler.
        try FileManager.default.createDirectory(at: s.root.appendingPathComponent("page_2.json"), withIntermediateDirectories: true)
        XCTAssertThrowsError(try s.save(ReadingPage(id: "page_2", title: "Neu")))
        XCTAssertEqual(try s.pages().map(\.text), ["Original"])
        XCTAssertFalse(try XCTUnwrap(s.pending().first).deleted)
    }

    func testImagesStayLocalAndSurviveDeletion() throws {
        let s = try store()
        try s.save(page())
        let bytes = Data([0, 1, 2, 3]) // Synthetische Bytes, keine Kamera erforderlich.
        try s.storeImage(bytes, named: "probe.jpg", for: "page_1")
        let image = try s.imageURL(named: "probe.jpg", for: "page_1")
        XCTAssertEqual(try Data(contentsOf: image), bytes)
        XCTAssertThrowsError(try s.storeImage(bytes, named: "../probe.jpg", for: "page_1"))
        try s.delete("page_1")
        XCTAssertEqual(try Data(contentsOf: image), bytes)
    }

    func testConflictImagesSurviveRestartAndDeletionIncludingRemoteTombstone() throws {
        for deleted in [false, true] {
            let s = try store()
            try s.apply(id: "page_1", json: page().json(), revision: 1, deviceName: "A")
            try s.save(page("Lokal"))
            let capture = ReadingCapture(id: "c1", image: "probe.jpg", recognizedText: "Lokal", source: "photo", createdMs: 1)
            let bytes = Data([0, 1, 2, 3])
            try s.storeImage(bytes, named: capture.image, for: "page_1")
            try s.setCaptures([capture], for: "page_1", composedText: "Lokal")
            let originalImage = try s.imageURL(named: capture.image, for: "page_1")
            try s.apply(id: "page_1", json: deleted ? nil : page("Remote").json(), revision: 2, deviceName: "A")
            let reopened = try ReadingPageStore(root: s.root)
            let copy = try XCTUnwrap(reopened.pages().first { $0.id != "page_1" })
            XCTAssertEqual(try reopened.captures(for: copy.id), [capture])
            XCTAssertFalse(try reopened.requiresCompositionChoice(copy.id))
            XCTAssertEqual(try Data(contentsOf: reopened.imageURL(named: capture.image, for: copy.id)), bytes)
            XCTAssertEqual(try reopened.imageURL(named: capture.image, for: copy.id), originalImage)
            // Auch eine weitere Konfliktgeneration muss den ursprünglichen Ordner behalten.
            try reopened.save(ReadingPage(id: copy.id, title: "Weiter bearbeitet"))
            try reopened.apply(id: copy.id, json: nil, revision: 1, deviceName: "A")
            let next = try XCTUnwrap(reopened.pages().first { $0.id != "page_1" && $0.id != copy.id })
            try reopened.delete("page_1")
            try reopened.delete(next.id)
            let archived = try ReadingPageStore(root: s.root)
            for id in ["page_1", copy.id, next.id] {
                XCTAssertNotNil(try archived.archivedPage(id))
                XCTAssertEqual(try Data(contentsOf: archived.imageURL(named: capture.image, for: id)), bytes)
            }
        }
    }

    func testSaveRejectsUnreadableSizeWithoutReplacingRecord() throws {
        let s = try store()
        try s.save(page())
        let target = s.root.appendingPathComponent("page_1.json")
        let before = try Data(contentsOf: target)
        var invalid = try page()
        invalid.text = String(repeating: "x", count: AblageJSON.defaultMaxBytes + 1)
        XCTAssertThrowsError(try s.save(invalid))
        XCTAssertEqual(try Data(contentsOf: target), before)
        XCTAssertEqual(try ReadingPageStore(root: s.root).pages().map(\.text), ["Original"])
    }

    func testSaveRejectsUnreadableNestingWithoutReplacingOrCreatingRecord() throws {
        let s = try store()
        try s.save(page())
        let target = s.root.appendingPathComponent("page_1.json")
        let before = try Data(contentsOf: target)
        var nested: AblageJSON = .null
        for _ in 0...AblageJSON.maxDepth { nested = .array([nested]) }
        for id in ["page_1", "page_2"] {
            var invalid = try ReadingPage(id: id, title: "Probe")
            invalid.setStateField("future", nested)
            XCTAssertThrowsError(try s.save(invalid))
        }
        XCTAssertEqual(try Data(contentsOf: target), before)
        XCTAssertFalse(FileManager.default.fileExists(atPath: s.root.appendingPathComponent("page_2.json").path))
        XCTAssertEqual(try ReadingPageStore(root: s.root).pages().map(\.text), ["Original"])
    }

    func testCorruptFileIsHeldAndCannotTurnIntoDeletion() throws {
        let s = try store()
        try s.save(page())
        try Data("defekt".utf8).write(to: s.root.appendingPathComponent("page_2.json"))
        XCTAssertEqual(try s.pages().count, 1)
        XCTAssertEqual(try s.pending().count, 1)
        XCTAssertThrowsError(try s.apply(id: "page_2", json: page().json(), revision: 1, deviceName: "A"))
        XCTAssertEqual(try Data(contentsOf: s.root.appendingPathComponent("page_2.json")), Data("defekt".utf8))
    }

    func testLocalMetadataSurvivesDesktopOverwriteAndNeverEntersPayload() throws {
        let s = try store()
        try s.save(page())
        let capture = ReadingCapture(id: "c1", image: "c1.jpg", recognizedText: "OCR", source: "camera", createdMs: 1)
        try s.setCaptures([capture], for: "page_1", composedText: "Original")
        try s.apply(id: "page_1", json: page("Desktop").json(), revision: 1, deviceName: "A")
        let reopened = try ReadingPageStore(root: s.root)
        XCTAssertEqual(try reopened.captures(for: "page_1"), [capture])
        XCTAssertTrue(try reopened.requiresCompositionChoice("page_1"))
        XCTAssertFalse(String(decoding: try XCTUnwrap(reopened.pages().first).json(), as: UTF8.self).contains("c1.jpg"))
        XCTAssertEqual(ReadingCapture.compose([capture, capture]), "OCR\n\nOCR")
    }

    func testCompositionChoiceAndUnsafeImageReference() throws {
        let s = try store()
        try s.save(page())
        var capture = ReadingCapture(image: "probe.jpg", recognizedText: "Original", source: "camera", createdMs: 1)
        try s.setCaptures([capture], for: "page_1", composedText: "Original")
        XCTAssertFalse(try s.requiresCompositionChoice("page_1"))
        capture.pendingRecognition = "Neue Erkennung"
        try s.setCaptures([capture], for: "page_1", composedText: "Original")
        XCTAssertTrue(try s.requiresCompositionChoice("page_1"))
        XCTAssertEqual(try s.pages().first?.text, "Original")
        capture.image = "../fremd.jpg"
        XCTAssertThrowsError(try s.setCaptures([capture], for: "page_1", composedText: nil))
        XCTAssertEqual(try s.captures(for: "page_1").first?.image, "probe.jpg")
    }

    func testRejectionAndOversizeDoNotLoopAndChangedContentRetries() throws {
        let s = try store()
        try s.save(page())
        let sent = try XCTUnwrap(s.pending().first)
        try s.reject(sent)
        XCTAssertTrue(try s.pending().isEmpty)
        try s.save(page("Neu"))
        XCTAssertEqual(try s.pending().count, 1)
        var large = try page()
        large.text = String(repeating: "ä", count: ReadingPage.maxPlaintext / 2)
        try s.save(large)
        XCTAssertTrue(try s.pending().isEmpty)
        XCTAssertEqual(try s.oversizedIDs(), ["page_1"])
    }
}
