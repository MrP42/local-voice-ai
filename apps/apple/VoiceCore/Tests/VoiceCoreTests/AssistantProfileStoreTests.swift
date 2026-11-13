import Foundation
import XCTest
@testable import VoiceCore

// Assistenten-Profil (lva#555, Paket G1): der Speicher der eigenen Assistenten (Verzeichnis, `<id>.json`, `aktiv.json`). Jeder Test
// arbeitet in einem eigenen Temp-Verzeichnis, nie in produktiven Daten. Nur synthetische Daten.

final class AssistantProfileStoreTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("lva-g1-store-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    private func store(_ source: AssistantIdSource? = nil) -> AssistantProfileStore {
        if let source { return AssistantProfileStore(directory: dir, idSource: source) }
        return AssistantProfileStore(directory: dir)
    }

    private func own(_ id: String, _ name: String = "Eigen") -> AssistantProfile {
        var p = AssistantProfile(id: id, name: name)
        p.instructions = "Sei kurz."
        return p
    }

    private func put(_ name: String, _ text: String) throws {
        try Data(text.utf8).write(to: dir.appendingPathComponent(name))
    }

    private func names() -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []).sorted()
    }

    func testSaveWritesExactEncodeOutputAndListReadsItBack() throws {
        let s = store()
        let saved = try s.save(own("eigen-aaaaaaaaaaaa", "  Mein Assistent  "))
        XCTAssertEqual(saved.name, "Mein Assistent")
        let data = try Data(contentsOf: dir.appendingPathComponent("eigen-aaaaaaaaaaaa.json"))
        XCTAssertEqual(String(decoding: data, as: UTF8.self), saved.encode())
        XCTAssertNotEqual(Array(data.prefix(3)), [0xEF, 0xBB, 0xBF], "kein BOM")
        let listing = s.list()
        XCTAssertEqual(listing.profiles, [saved])
        XCTAssertEqual(listing.rejected, [])
    }

    func testListIsSortedByFileNameAndIgnoresOtherFiles() throws {
        let s = store()
        try s.save(own("eigen-bbbbbbbbbbbb", "B"))
        try s.save(own("eigen-aaaaaaaaaaaa", "A"))
        try put("notiz.txt", "keine Datei des Speichers")
        try put("eigen-cccccccccccc.json.bak", "{}")
        try FileManager.default.createDirectory(at: dir.appendingPathComponent("ordner.json"), withIntermediateDirectories: false)
        let listing = s.list()
        XCTAssertEqual(listing.profiles.map(\.id), ["eigen-aaaaaaaaaaaa", "eigen-bbbbbbbbbbbb"])
        XCTAssertEqual(listing.rejected, [])
    }

    func testBrokenFileIsRejectedWithIssuesAndLeftUntouched() throws {
        let s = store()
        try s.save(own("eigen-aaaaaaaaaaaa"))
        try put("kaputt.json", "{ nicht json")
        try put("falsch.json", #"{"schema":"lva.assistent/1","id":"Falsch","name":""}"#)
        try put("neuer.json", #"{"schema":"lva.assistent/2","id":"neuer","name":"N"}"#)
        let listing = s.list()
        XCTAssertEqual(listing.profiles.map(\.id), ["eigen-aaaaaaaaaaaa"])
        let rejected = Dictionary(uniqueKeysWithValues: listing.rejected.map { ($0.fileName, $0.issues.map { "\($0.path):\($0.code.rawValue)" }) })
        XCTAssertEqual(rejected["kaputt.json"], [":json"])
        XCTAssertEqual(rejected["falsch.json"], ["id:muster", "name:laenge"])
        XCTAssertEqual(rejected["neuer.json"], ["schema:schema_neuer"])
        XCTAssertEqual(try String(contentsOf: dir.appendingPathComponent("kaputt.json"), encoding: .utf8), "{ nicht json")
    }

    func testFileNameMustMatchIdAndTemplatesNeverLiveInTheDirectory() throws {
        let s = store()
        try put("anders.json", #"{"schema":"lva.assistent/1","id":"eigen-aaaaaaaaaaaa","name":"N"}"#)
        try put("alles.json", #"{"schema":"lva.assistent/1","id":"alles","name":"Geschummelt"}"#)
        try put("vorlage-x.json", #"{"schema":"lva.assistent/1","id":"vorlage-x","name":"V"}"#)
        let rejected = Dictionary(uniqueKeysWithValues: s.list().rejected.map { ($0.fileName, $0.issues.map { "\($0.path):\($0.code.rawValue)" }) })
        XCTAssertEqual(rejected["anders.json"], ["id:muster"])
        XCTAssertEqual(rejected["alles.json"], ["id:wert"])
        XCTAssertEqual(rejected["vorlage-x.json"], ["id:wert"])
        XCTAssertEqual(s.list().profiles, [])
        XCTAssertEqual(s.profile(id: "alles")?.name, "Alles", "die Vorlage gewinnt, die Datei zählt nicht")
    }

    func testOversizedFileIsRejectedWithoutReading() throws {
        let s = store()
        let big = String(repeating: "x", count: AssistantProfileStore.maxFileBytes + 1)
        try put("gross.json", #"{"schema":"lva.assistent/1","id":"gross","name":"G","x":""# + big + #""}"#)
        let listing = s.list()
        XCTAssertEqual(listing.profiles, [])
        XCTAssertEqual(listing.rejected, [AssistantRejectedFile(fileName: "gross.json", issues: [AssistantProfileIssue(path: "", code: .length)])])
        // genau an der Grenze wird gelesen (hier als gültiges Profil mit fremdem Feld).
        let filler = String(repeating: "y", count: 1000)
        try put("klein.json", #"{"schema":"lva.assistent/1","id":"klein","name":"K","x":"\#(filler)"}"#)
        XCTAssertEqual(s.list().profiles.map(\.id), ["klein"])
    }

    func testTooManyFilesAreRejectedBeyondTwoHundredWithoutReading() throws {
        let s = store()
        for i in 0..<(AssistantProfileStore.maxFiles + 3) {
            let id = "p" + String(format: "%03d", i)
            try put(id + ".json", #"{"schema":"lva.assistent/1","id":""# + id + #"","name":"N"}"#)
        }
        let listing = s.list()
        XCTAssertEqual(listing.profiles.count, AssistantProfileStore.maxFiles)
        XCTAssertEqual(listing.rejected.map(\.fileName), ["p200.json", "p201.json", "p202.json"])
        XCTAssertEqual(listing.rejected.first?.issues, [AssistantProfileIssue(path: "", code: .tooMany)])
        XCTAssertThrowsError(try s.save(own("neu-eins"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .tooMany) }
        // Überschreiben einer vorhandenen Datei bleibt möglich.
        XCTAssertNoThrow(try s.save(own("p000")))
    }

    func testTemplatesCanNeitherBeOverwrittenNorDeleted() throws {
        let s = store()
        XCTAssertThrowsError(try s.save(own("alles"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.save(own("vorlage-buchgespraech"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.save(own("vorlage-neu"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.delete(id: "alles")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.delete(id: "vorlage-lernpartner")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertEqual(names(), [], "nichts geschrieben")
        XCTAssertEqual(s.allProfiles().count, 15)
    }

    func testInvalidProfileIsNotSaved() {
        let s = store()
        var p = own("eigen-aaaaaaaaaaaa")
        p.language = "xx_YY"
        XCTAssertThrowsError(try s.save(p)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .invalid([AssistantProfileIssue(path: "sprache", code: .pattern)])) }
        XCTAssertEqual(names(), [])
    }

    func testPathTraversalIdsAreNeverUsed() {
        let s = store()
        XCTAssertThrowsError(try s.save(own("../draussen")))
        XCTAssertThrowsError(try s.delete(id: "../draussen")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound) }
        XCTAssertNil(s.profile(id: "../draussen"))
        XCTAssertThrowsError(try s.setActive("../draussen"))
    }

    func testDeleteRemovesTheFileOnly() throws {
        let s = store()
        try s.save(own("eigen-aaaaaaaaaaaa"))
        try s.save(own("eigen-bbbbbbbbbbbb"))
        try s.delete(id: "eigen-aaaaaaaaaaaa")
        XCTAssertEqual(names(), ["eigen-bbbbbbbbbbbb.json"])
        XCTAssertThrowsError(try s.delete(id: "eigen-aaaaaaaaaaaa")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound) }
    }

    func testActiveSelectionFallsBackToEverything() throws {
        let s = store()
        XCTAssertEqual(s.activeId(), "alles", "ohne Datei")
        try s.save(own("eigen-aaaaaaaaaaaa", "Eins"))
        try s.setActive("eigen-aaaaaaaaaaaa")
        XCTAssertEqual(s.activeId(), "eigen-aaaaaaaaaaaa")
        XCTAssertEqual(s.active().name, "Eins")
        XCTAssertEqual(try String(contentsOf: dir.appendingPathComponent("aktiv.json"), encoding: .utf8), "{\n  \"id\": \"eigen-aaaaaaaaaaaa\"\n}\n")

        try s.setActive("vorlage-lernpartner")
        XCTAssertEqual(s.active().id, "vorlage-lernpartner")

        try s.setActive("eigen-aaaaaaaaaaaa")
        try s.delete(id: "eigen-aaaaaaaaaaaa")
        XCTAssertEqual(s.activeId(), "alles", "gelöschtes Profil")
        XCTAssertEqual(s.active().id, "alles")

        try put("aktiv.json", "{ kaputt")
        XCTAssertEqual(s.activeId(), "alles", "kein JSON")
        try put("aktiv.json", #"{"id": 5}"#)
        XCTAssertEqual(s.activeId(), "alles", "id keine Zeichenkette")
        try put("aktiv.json", #"{"id": "gibt-es-nicht"}"#)
        XCTAssertEqual(s.activeId(), "alles", "unbekannte Id")
        try put("aktiv.json", "[]")
        XCTAssertEqual(s.activeId(), "alles", "kein Objekt")
        try put("aktiv.json", #"{"id":"eigen-bbbbbbbbbbbb"}"#)
        try put("eigen-bbbbbbbbbbbb.json", #"{"schema":"lva.assistent/1","id":"Kaputt","name":""}"#)
        XCTAssertEqual(s.activeId(), "alles", "Auswahl zeigt auf ungültige Datei")
        XCTAssertThrowsError(try s.setActive("eigen-bbbbbbbbbbbb")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound) }
        XCTAssertThrowsError(try s.setActive("gibt-es-nicht")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound) }
    }

    func testOversizedActiveFileFallsBack() throws {
        let s = store()
        try put("aktiv.json", #"{"id":"alles","x":""# + String(repeating: "x", count: AssistantProfileStore.maxFileBytes) + #""}"#)
        XCTAssertEqual(s.activeId(), "alles")
        XCTAssertEqual(s.list().rejected, [], "aktiv.json ist keine Profildatei")
    }

    func testAtomicReplaceLeavesOneCompleteFileAndNoTempFiles() throws {
        let s = store()
        let id = "eigen-aaaaaaaaaaaa"
        try s.save(own(id, "Erste Fassung"))
        let first = try Data(contentsOf: dir.appendingPathComponent(id + ".json"))
        var changed = own(id, "Zweite Fassung")
        changed.instructions = String(repeating: "Lang. ", count: 1000)
        try s.save(changed)
        let second = try Data(contentsOf: dir.appendingPathComponent(id + ".json"))
        XCTAssertNotEqual(first, second)
        XCTAssertEqual(String(decoding: second, as: UTF8.self), changed.encode())
        XCTAssertEqual(names(), [id + ".json"], "keine Reste einer temporären Datei")
        XCTAssertEqual(s.list().profiles.map(\.name), ["Zweite Fassung"])
        // Viele Ersetzungen hintereinander (auch aus mehreren Threads) hinterlassen immer ein ganzes, gültiges Profil.
        DispatchQueue.concurrentPerform(iterations: 40) { i in
            var p = AssistantProfile(id: id, name: "Fassung \(i)")
            p.instructions = String(repeating: "x", count: 100 * (i + 1))
            _ = try? s.save(p)
        }
        XCTAssertEqual(s.list().rejected, [])
        XCTAssertEqual(s.list().profiles.count, 1)
        XCTAssertEqual(names(), [id + ".json"])
    }

    func testNewOwnIdUsesTheGivenSourceAndSkipsTakenIds() throws {
        let counter = Counter()
        let s = store { counter.next() }
        let first = try s.newOwnId()
        XCTAssertTrue(first.hasPrefix("eigen-"))
        XCTAssertEqual(first.unicodeScalars.count, 18)
        XCTAssertTrue(AssistantProfilePatterns.id(first))
        XCTAssertEqual(first, "eigen-0123456789ab")
        try s.save(own(first))
        let second = try s.newOwnId()
        XCTAssertNotEqual(first, second)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent(second + ".json").path))
        // Der Standard-Erzeuger liefert gültige Ids.
        let random = try store().newOwnId()
        XCTAssertTrue(AssistantProfilePatterns.id(random) && random.hasPrefix("eigen-") && random.count == 18)
    }

    func testMissingDirectoryListsEmptyAndSaveCreatesIt() throws {
        let nested = dir.appendingPathComponent("a/b")
        let s = AssistantProfileStore(directory: nested)
        XCTAssertEqual(s.list(), AssistantProfileListing(profiles: [], rejected: []))
        XCTAssertEqual(s.activeId(), "alles")
        try s.save(own("eigen-aaaaaaaaaaaa"))
        XCTAssertEqual(s.list().profiles.count, 1)
    }
}

private final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> UInt8 {
        lock.withLock {
            defer { value += 1 }
            return UInt8(value % 256)
        }
    }
}
