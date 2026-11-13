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

    /// Alle Einträge des Verzeichnisses mit ihrem Inhalt (reguläre Dateien), ihrer Art (Verzeichnis) oder ihrem Ziel (Symlink).
    private func snapshot(_ directory: URL? = nil) throws -> [String: String] {
        let base = directory ?? dir!
        let fm = FileManager.default
        var out: [String: String] = [:]
        for name in try fm.contentsOfDirectory(atPath: base.path).sorted() {
            let path = base.appendingPathComponent(name).path
            let type = try fm.attributesOfItem(atPath: path)[.type] as? FileAttributeType
            switch type {
            case .typeSymbolicLink?: out[name] = "symlink:" + (try fm.destinationOfSymbolicLink(atPath: path))
            case .typeDirectory?: out[name] = "dir:" + (try fm.contentsOfDirectory(atPath: path).sorted().joined(separator: ","))
            default: out[name] = "file:" + (try Data(contentsOf: URL(fileURLWithPath: path))).base64EncodedString()
            }
        }
        return out
    }

    func testSaveWritesExactEncodeOutputAndListReadsItBack() throws {
        let s = store()
        let saved = try s.save(own("eigen-aaaaaaaaaaaa", "  Mein Assistent  "))
        XCTAssertEqual(saved.name, "Mein Assistent")
        let data = try Data(contentsOf: dir.appendingPathComponent("eigen-aaaaaaaaaaaa.json"))
        XCTAssertEqual(Array(data), Array(saved.encode().utf8))
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
        XCTAssertEqual(s.list().rejected, [activeRejected], "zu große aktiv.json ist belegt und wird gezeigt")
        XCTAssertEqual(s.list().profiles, [])
    }

    /// Eintrag in `rejected` für eine belegte `aktiv.json`.
    private var activeRejected: AssistantRejectedFile {
        AssistantRejectedFile(fileName: "aktiv.json", issues: [AssistantProfileIssue(path: "", code: .value)])
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
        XCTAssertEqual(Array(second), Array(changed.encode().utf8))
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

    // MARK: `aktiv` ist vorbehalten

    func testAktivIsReservedForSaveDeleteAndSelection() throws {
        let s = store()
        try s.save(own("eigen-aaaaaaaaaaaa"))
        try s.setActive("eigen-aaaaaaaaaaaa")
        let before = try snapshot()
        XCTAssertEqual(Set(before.keys), ["aktiv.json", "eigen-aaaaaaaaaaaa.json"])
        XCTAssertThrowsError(try s.save(own("aktiv"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.delete(id: "aktiv")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.setActive("aktiv")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.setActive("vorlage-gibt-es-nicht")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertNil(s.profile(id: "aktiv"))
        XCTAssertEqual(try snapshot(), before, "nichts verändert")
        XCTAssertEqual(s.activeId(), "eigen-aaaaaaaaaaaa")
        // Auch die Vorlagen bleiben wählbar.
        try s.setActive("alles")
        XCTAssertEqual(s.activeId(), "alles")
    }

    func testAktivJsonIsNeverReadAsAProfileNorOverwrittenByOne() throws {
        let s = store()
        // aktiv.json hält zufällig ein vollständiges Profil mit der Id `aktiv`.
        var profile = AssistantProfile(id: "aktiv", name: "Falsch")
        profile.instructions = "Ich bin kein Profil."
        let bytes = profile.encode()
        try put("aktiv.json", bytes)
        XCTAssertEqual(s.list(), AssistantProfileListing(profiles: [], rejected: [activeRejected]), "aktiv.json zählt nicht als Profil, ist aber belegt")
        XCTAssertNil(s.profile(id: "aktiv"))
        XCTAssertEqual(s.activeId(), "alles")
        XCTAssertEqual(s.active().id, "alles")
        XCTAssertThrowsError(try s.save(profile)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertThrowsError(try s.delete(id: "aktiv")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .reservedId) }
        XCTAssertEqual(Array(try Data(contentsOf: dir.appendingPathComponent("aktiv.json"))), Array(bytes.utf8))
        // Ohne das fremde Dokument schreibt ein Auswahlwechsel nur aktiv.json und fasst kein Profil an.
        try FileManager.default.removeItem(at: dir.appendingPathComponent("aktiv.json"))
        try s.save(own("eigen-aaaaaaaaaaaa"))
        try s.save(own("eigen-bbbbbbbbbbbb"))
        let profiles = try snapshot().filter { $0.key != "aktiv.json" }
        try s.setActive("eigen-aaaaaaaaaaaa")
        try s.setActive("eigen-bbbbbbbbbbbb")
        try s.setActive("vorlage-lernpartner")
        XCTAssertEqual(try snapshot().filter { $0.key != "aktiv.json" }, profiles)
        XCTAssertEqual(s.activeId(), "vorlage-lernpartner")
    }

    // MARK: Abgewiesene Dateien bleiben unberührt

    private func rejectedFiles() -> [(name: String, text: String)] {
        let tooBig = #"{"schema":"lva.assistent/1","id":"eigen-ffffffffffff","name":"G","x":""# + String(repeating: "x", count: AssistantProfileStore.maxFileBytes) + #""}"#
        return [
            ("eigen-aaaaaaaaaaaa.json", "{ nicht json"),
            ("eigen-bbbbbbbbbbbb.json", #"{"schema":"lva.assistent/2","id":"eigen-bbbbbbbbbbbb","name":"N"}"#),
            ("eigen-cccccccccccc.json", #"{"schema":"lva.assistent/1","id":"eigen-dddddddddddd","name":"N"}"#),
            ("eigen-eeeeeeeeeeee.json", #"{"schema":"lva.assistent/1","id":"eigen-eeeeeeeeeeee","name":""}"#),
            ("eigen-ffffffffffff.json", tooBig),
            ("eigen-gggggggggggg.json", #"{"schema":"lva.assistent/1","id":"alles","name":"Geschummelt"}"#),
            ("eigen-hhhhhhhhhhhh.json", ""),
        ]
    }

    func testSaveAndDeleteNeverTouchRejectedFiles() throws {
        let s = store()
        for (name, text) in rejectedFiles() { try put(name, text) }
        let before = try snapshot()
        for (name, _) in rejectedFiles() {
            let id = String(name.dropLast(5))
            XCTAssertThrowsError(try s.save(own(id)), name) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied, name) }
            XCTAssertThrowsError(try s.delete(id: id), name) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied, name) }
            XCTAssertEqual(try snapshot(), before, "\(name): Bytes und Verzeichnis unverändert")
        }
        XCTAssertEqual(s.list().profiles, [])
        XCTAssertEqual(s.list().rejected.count, rejectedFiles().count)
        // Neben den belegten Namen lässt sich weiter speichern, und ein gültiges Profil unter dem Namen der Falschen-Id-Datei.
        XCTAssertNoThrow(try s.save(own("eigen-dddddddddddd")))
        XCTAssertEqual(s.list().profiles.map(\.id), ["eigen-dddddddddddd"])
    }

    func testSaveAndDeleteNeverFollowOrReplaceSymlinks() throws {
        let s = store()
        let outside = FileManager.default.temporaryDirectory.appendingPathComponent("lva-g1-outside-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: outside) }
        var other = AssistantProfile(id: "eigen-aaaaaaaaaaaa", name: "Fremd")
        other.instructions = "Gehört mir."
        let target = outside.appendingPathComponent("ziel.json")
        try Data(other.encode().utf8).write(to: target)
        let link = dir.appendingPathComponent("eigen-aaaaaaaaaaaa.json")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)
        let dangling = dir.appendingPathComponent("eigen-bbbbbbbbbbbb.json")
        try FileManager.default.createSymbolicLink(at: dangling, withDestinationURL: outside.appendingPathComponent("gibt-es-nicht.json"))
        let directoryLink = dir.appendingPathComponent("eigen-cccccccccccc.json")
        try FileManager.default.createSymbolicLink(at: directoryLink, withDestinationURL: outside)
        let before = try snapshot()
        let targetBefore = try Data(contentsOf: target)

        // Ein Symlink gilt als abgewiesen (json) — auch einer auf ein gültiges Profil.
        let rejected = Dictionary(uniqueKeysWithValues: s.list().rejected.map { ($0.fileName, $0.issues.map { "\($0.path):\($0.code.rawValue)" }) })
        XCTAssertEqual(rejected["eigen-aaaaaaaaaaaa.json"], [":json"])
        XCTAssertEqual(rejected["eigen-bbbbbbbbbbbb.json"], [":json"])
        XCTAssertEqual(rejected["eigen-cccccccccccc.json"], [":json"])
        XCTAssertEqual(s.list().profiles, [])
        XCTAssertNil(s.profile(id: "eigen-aaaaaaaaaaaa"))

        for id in ["eigen-aaaaaaaaaaaa", "eigen-bbbbbbbbbbbb", "eigen-cccccccccccc"] {
            XCTAssertThrowsError(try s.save(own(id)), id) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied, id) }
            XCTAssertThrowsError(try s.delete(id: id), id) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound, id) }
            XCTAssertThrowsError(try s.setActive(id), id) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound, id) }
        }
        XCTAssertEqual(try snapshot(), before, "Links und Verzeichnis unverändert")
        XCTAssertEqual(try Data(contentsOf: target), targetBefore, "das Ziel des Links bleibt, wie es war")
        XCTAssertEqual(try snapshot(outside).keys.sorted(), ["ziel.json"], "nichts draußen angelegt")
    }

    func testSaveAndDeleteNeverTouchADirectoryNamedLikeAProfile() throws {
        let s = store()
        let folder = dir.appendingPathComponent("eigen-aaaaaaaaaaaa.json")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        try Data("Inhalt".utf8).write(to: folder.appendingPathComponent("datei.txt"))
        let before = try snapshot()
        XCTAssertThrowsError(try s.save(own("eigen-aaaaaaaaaaaa"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertThrowsError(try s.delete(id: "eigen-aaaaaaaaaaaa")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound) }
        XCTAssertNil(s.profile(id: "eigen-aaaaaaaaaaaa"))
        XCTAssertEqual(s.list(), AssistantProfileListing(profiles: [], rejected: []), "ein Verzeichnis wird ignoriert")
        XCTAssertEqual(try snapshot(), before)
    }

    func testValidOwnFilesStayReplaceableAndDeletable() throws {
        let s = store()
        try s.save(own("eigen-aaaaaaaaaaaa", "Eins"))
        try s.save(own("eigen-aaaaaaaaaaaa", "Zwei"))
        XCTAssertEqual(s.profile(id: "eigen-aaaaaaaaaaaa")?.name, "Zwei")
        try s.delete(id: "eigen-aaaaaaaaaaaa")
        XCTAssertEqual(try snapshot(), [:])
    }

    func testNewOwnIdSkipsDanglingSymlinks() throws {
        let counter = Counter()
        let s = store { counter.next() }
        try FileManager.default.createSymbolicLink(at: dir.appendingPathComponent("eigen-0123456789ab.json"), withDestinationURL: dir.appendingPathComponent("gibt-es-nicht"))
        let id = try s.newOwnId()
        XCTAssertNotEqual(id, "eigen-0123456789ab")
        XCTAssertTrue(id.hasPrefix("eigen-"))
    }

    // MARK: Nur speicherbare Profile werden gespeichert (Prüfung des Baums)

    func testUnreadableExtrasAreNotSavedAndLeaveTheFileUntouched() throws {
        let s = store()
        let id = "eigen-aaaaaaaaaaaa"
        try s.save(own(id, "Gut"))
        let before = try snapshot()
        var nan = own(id, "Schlecht")
        nan.extra = [AssistantJSONMember("x", .number("NaN"))]
        XCTAssertThrowsError(try s.save(nan)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .invalid([AssistantProfileIssue(path: "x", code: .json)])) }
        var deep = AssistantJSON.array([])
        for _ in 0..<70 { deep = .array([deep]) }
        var tooDeep = own(id, "Zu tief")
        tooDeep.extra = [AssistantJSONMember("tief", deep)]
        XCTAssertThrowsError(try s.save(tooDeep)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .invalid([AssistantProfileIssue(path: "tief", code: .json)])) }
        var duplicate = own(id, "Doppelt")
        duplicate.extra = [AssistantJSONMember("k", .null), AssistantJSONMember("k", .bool(true))]
        XCTAssertThrowsError(try s.save(duplicate)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .invalid([AssistantProfileIssue(path: "", code: .json)])) }
        var nested = own(id, "Verschachtelt")
        nested.knowledge.extra = [AssistantJSONMember("neu", .array([.number("1.")]))]
        XCTAssertThrowsError(try s.save(nested)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .invalid([AssistantProfileIssue(path: "wissen.neu", code: .json)])) }
        XCTAssertEqual(try snapshot(), before, "die vorhandene Datei bleibt unverändert")
        XCTAssertEqual(s.profile(id: id)?.name, "Gut")
        // Ein neues Profil mit solchem Feld legt keine Datei an.
        var fresh = own("eigen-bbbbbbbbbbbb")
        fresh.extra = [AssistantJSONMember("x", .number("NaN"))]
        XCTAssertThrowsError(try s.save(fresh))
        XCTAssertEqual(try snapshot(), before)
    }

    /// Grundsatz: Was `save` annimmt, liest `list` wieder, und die Datei ist die bytegleiche zweite Ausgabe.
    func testWhatSaveAcceptsIsReadBackByteIdentical() throws {
        let s = store()
        var p = own("eigen-aaaaaaaaaaaa", "Rund")
        p.extra = [AssistantJSONMember("zahlen", .array([.number("-0.5e+10"), .number("1.50"), .number("0")]))]
        p.behavior.extra = [AssistantJSONMember("neu", .object([AssistantJSONMember("\u{00e9}", .number("1")), AssistantJSONMember("e\u{0301}", .number("2"))]))]
        let saved = try s.save(p)
        let listing = s.list()
        XCTAssertEqual(listing.rejected, [])
        XCTAssertEqual(listing.profiles, [saved])
        let data = try Data(contentsOf: dir.appendingPathComponent("eigen-aaaaaaaaaaaa.json"))
        XCTAssertEqual(Array(data), Array(listing.profiles[0].encode().utf8))
        XCTAssertEqual(Array(data), Array(saved.encode().utf8))
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

    // MARK: Fremde aktiv.json bleibt unberührt

    /// Gültiges vollständiges Profil mit der Id `aktiv` und eindeutigen Anweisungen (Altbestand eines fremden Programms).
    private func foreignActiveBytes() -> [UInt8] {
        var profile = AssistantProfile(id: "aktiv", name: "Fremd")
        profile.instructions = "Eindeutige Anweisung des fremden Programms 7f3a91."
        return Array(profile.encode().utf8)
    }

    func testSetActiveNeverOverwritesAForeignProfileInAktivJson() throws {
        let s = store()
        let bytes = foreignActiveBytes()
        try Data(bytes).write(to: dir.appendingPathComponent("aktiv.json"))
        let before = try snapshot()
        for _ in 0..<2 {
            XCTAssertThrowsError(try s.setActive("alles")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
            XCTAssertEqual(Array(try Data(contentsOf: dir.appendingPathComponent("aktiv.json"))), bytes)
            XCTAssertEqual(try snapshot(), before)
        }
        XCTAssertEqual(s.activeId(), "alles")
        XCTAssertEqual(s.list().rejected, [activeRejected])
        XCTAssertEqual(s.list().profiles, [])
        // Auch ein eigenes Profil lässt sich nicht wählen, solange aktiv.json belegt ist.
        try s.save(own("eigen-aaaaaaaaaaaa"))
        XCTAssertThrowsError(try s.setActive("eigen-aaaaaaaaaaaa")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertEqual(Array(try Data(contentsOf: dir.appendingPathComponent("aktiv.json"))), bytes)
        XCTAssertEqual(s.activeId(), "alles")
    }

    func testSetActiveRefusesEveryOccupiedAktivJsonAndLeavesItUnchanged() throws {
        let s = store()
        let aktiv = dir.appendingPathComponent("aktiv.json")
        let outside = FileManager.default.temporaryDirectory.appendingPathComponent("lva-g1-outside-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: outside, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: outside) }
        let selectionOutside = outside.appendingPathComponent("auswahl.json")
        try Data(#"{"id":"alles"}"#.utf8).write(to: selectionOutside)
        let big = #"{"id":"alles","x":""# + String(repeating: "x", count: AssistantProfileStore.maxFileBytes) + #""}"#
        let texts = [
            #"{"id":"alles","neu":1}"#,
            #"{"neu":"alles"}"#,
            "{}",
            "{ kaputt",
            #"{"id": 5}"#,
            #"{"id": null}"#,
            #"["alles"]"#,
            "",
            big,
        ]
        for text in texts {
            try put("aktiv.json", text)
            let before = try snapshot()
            for _ in 0..<2 {
                XCTAssertThrowsError(try s.setActive("alles"), String(text.prefix(40))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
                XCTAssertEqual(try snapshot(), before, String(text.prefix(40)))
            }
            XCTAssertEqual(s.activeId(), "alles")
            XCTAssertEqual(s.list().rejected, [activeRejected], String(text.prefix(40)))
            try FileManager.default.removeItem(at: aktiv)
        }
        // Symlink (auch auf ein gültiges Auswahldokument), hängender Symlink und Verzeichnis.
        try FileManager.default.createSymbolicLink(at: aktiv, withDestinationURL: selectionOutside)
        var before = try snapshot()
        XCTAssertThrowsError(try s.setActive("alles")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertEqual(try snapshot(), before)
        XCTAssertEqual(try Data(contentsOf: selectionOutside), Data(#"{"id":"alles"}"#.utf8))
        XCTAssertEqual(s.list().rejected, [activeRejected])
        try FileManager.default.removeItem(at: aktiv)
        try FileManager.default.createSymbolicLink(at: aktiv, withDestinationURL: outside.appendingPathComponent("gibt-es-nicht.json"))
        before = try snapshot()
        XCTAssertThrowsError(try s.setActive("alles")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertEqual(try snapshot(), before)
        XCTAssertEqual(s.list().rejected, [activeRejected])
        try FileManager.default.removeItem(at: aktiv)
        try FileManager.default.createDirectory(at: aktiv, withIntermediateDirectories: false)
        try Data("Inhalt".utf8).write(to: aktiv.appendingPathComponent("datei.txt"))
        before = try snapshot()
        XCTAssertThrowsError(try s.setActive("alles")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertEqual(try snapshot(), before)
        XCTAssertEqual(s.list(), AssistantProfileListing(profiles: [], rejected: []), "ein Verzeichnis aktiv.json wird ignoriert")
        XCTAssertEqual(s.activeId(), "alles")
        XCTAssertEqual(try snapshot(outside).keys.sorted(), ["auswahl.json"], "nichts draußen angelegt")
    }

    func testAnOrdinarySelectionDocumentIsReplacedAndAMissingOneIsCreated() throws {
        let s = store()
        XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent("aktiv.json").path))
        try s.setActive("vorlage-lernpartner")
        XCTAssertEqual(try String(contentsOf: dir.appendingPathComponent("aktiv.json"), encoding: .utf8), "{\n  \"id\": \"vorlage-lernpartner\"\n}\n")
        XCTAssertEqual(s.activeId(), "vorlage-lernpartner")
        // Andere Schreibweise, ein Mitglied, auch eine Id ohne Profil: gilt als Auswahldokument und wird ersetzt.
        // (Doppelte Schlüssel fasst der Leser zu einem Mitglied zusammen, der letzte gilt.)
        for text in [#"{ "id" : "alles" }"#, #"{"id":"gibt-es-nicht"}"#, "{\n\t\"id\": \"\"\n}", #"{"id":"x","id":"alles"}"#] {
            try put("aktiv.json", text)
            XCTAssertEqual(s.list(), AssistantProfileListing(profiles: [], rejected: []), "Auswahldokument zeigt nichts Abgewiesenes")
            try s.setActive("alles")
            XCTAssertEqual(try String(contentsOf: dir.appendingPathComponent("aktiv.json"), encoding: .utf8), "{\n  \"id\": \"alles\"\n}\n")
        }
        XCTAssertEqual(s.activeId(), "alles")
    }

    // MARK: Die Auswahl sind die ersten 200 Namen

    /// Ein Verzeichnis unter `dir` mit 201 gültigen Profilen `p000.json` … `p200.json`.
    private func storeWithTwoHundredAndOne(_ label: String) throws -> (AssistantProfileStore, URL) {
        let sub = dir.appendingPathComponent(label)
        try FileManager.default.createDirectory(at: sub, withIntermediateDirectories: true)
        for i in 0...AssistantProfileStore.maxFiles {
            let id = "p" + String(format: "%03d", i)
            try Data(AssistantProfile(id: id, name: "N").encode().utf8).write(to: sub.appendingPathComponent(id + ".json"))
        }
        return (AssistantProfileStore(directory: sub), sub)
    }

    func testAFileBeyondTheFirstTwoHundredNamesIsAbsentForTheStore() throws {
        let (s, sub) = try storeWithTwoHundredAndOne("liste")
        let listing = s.list()
        XCTAssertEqual(listing.profiles.count, AssistantProfileStore.maxFiles)
        XCTAssertEqual(listing.rejected, [AssistantRejectedFile(fileName: "p200.json", issues: [AssistantProfileIssue(path: "", code: .tooMany)])])
        XCTAssertNil(s.profile(id: "p200"))
        XCTAssertNotNil(s.profile(id: "p199"))
        XCTAssertThrowsError(try s.setActive("p200")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .notFound) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: sub.appendingPathComponent("aktiv.json").path), "nichts geschrieben")
        // Zeigt aktiv.json auf die abgewiesene Datei, gilt der Rückfall.
        try Data(#"{"id":"p200"}"#.utf8).write(to: sub.appendingPathComponent("aktiv.json"))
        XCTAssertEqual(s.activeId(), "alles")
        XCTAssertEqual(s.active().id, "alles")
        // Neuanlage ab 200 Namen ist `tooMany`.
        XCTAssertThrowsError(try s.save(own("neu-eins"))) { XCTAssertEqual($0 as? AssistantProfileStoreError, .tooMany) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: sub.appendingPathComponent("neu-eins.json").path))
    }

    func testSaveAndDeleteNeverTouchAFileBeyondTheFirstTwoHundredNames() throws {
        let (saving, savingDir) = try storeWithTwoHundredAndOne("speichern")
        var changed = AssistantProfile(id: "p200", name: "Geändert")
        changed.instructions = "Soll nie ankommen."
        let savingBefore = try snapshot(savingDir)
        XCTAssertThrowsError(try saving.save(changed)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertEqual(try snapshot(savingDir), savingBefore, "p200.json unverändert, nichts angelegt")
        XCTAssertEqual(Array(try Data(contentsOf: savingDir.appendingPathComponent("p200.json"))), Array(AssistantProfile(id: "p200", name: "N").encode().utf8))

        let (deleting, deletingDir) = try storeWithTwoHundredAndOne("loeschen")
        let deletingBefore = try snapshot(deletingDir)
        XCTAssertThrowsError(try deleting.delete(id: "p200")) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
        XCTAssertTrue(FileManager.default.fileExists(atPath: deletingDir.appendingPathComponent("p200.json").path))
        XCTAssertEqual(try snapshot(deletingDir), deletingBefore)
    }

    func testFilesInsideTheSelectionStayChangeableAndDeletingOneAdmitsTheNext() throws {
        let (inner, innerDir) = try storeWithTwoHundredAndOne("innen")
        var changed = AssistantProfile(id: "p199", name: "Geändert")
        changed.instructions = "Kommt an."
        XCTAssertNoThrow(try inner.save(changed))
        XCTAssertEqual(inner.profile(id: "p199")?.name, "Geändert")
        XCTAssertNoThrow(try inner.delete(id: "p199"))
        XCTAssertFalse(FileManager.default.fileExists(atPath: innerDir.appendingPathComponent("p199.json").path))
        // Mit 200 Namen (p000 … p198, p200) gehört p200 jetzt zur Auswahl.
        XCTAssertEqual(inner.list().profiles.count, AssistantProfileStore.maxFiles)
        XCTAssertEqual(inner.list().rejected, [])
        XCTAssertNotNil(inner.profile(id: "p200"))

        let (first, _) = try storeWithTwoHundredAndOne("erstes")
        XCTAssertNil(first.profile(id: "p200"))
        XCTAssertNoThrow(try first.delete(id: "p000"))
        XCTAssertEqual(first.profile(id: "p200")?.id, "p200", "nach dem Löschen von p000 ist p200 in der Auswahl und lesbar")
        var rewritten = AssistantProfile(id: "p200", name: "Neu")
        rewritten.instructions = "Überschrieben."
        XCTAssertNoThrow(try first.save(rewritten))
        XCTAssertEqual(first.profile(id: "p200")?.name, "Neu")
        XCTAssertNoThrow(try first.setActive("p200"))
        XCTAssertEqual(first.activeId(), "p200")
        XCTAssertEqual(first.list().rejected, [])
    }

    // MARK: Kopie einer Vorlage auf eine belegte Id

    func testSavingATemplateCopyUnderAnOccupiedIdIsRefusedRepeatedly() throws {
        let s = store()
        guard let template = AssistantProfileTemplates.template(id: "vorlage-lernpartner"),
              case .success(let copy) = AssistantProfileTemplates.copyAsOwn(template, newId: "eigen-aaaaaaaaaaaa", newName: "Kopie") else {
            return XCTFail("Vorlage kopieren")
        }
        try put("eigen-aaaaaaaaaaaa.json", "{ nicht json")
        let before = try snapshot()
        for _ in 0..<2 {
            XCTAssertThrowsError(try s.save(copy)) { XCTAssertEqual($0 as? AssistantProfileStoreError, .occupied) }
            XCTAssertEqual(try snapshot(), before)
        }
        XCTAssertEqual(try String(contentsOf: dir.appendingPathComponent("eigen-aaaaaaaaaaaa.json"), encoding: .utf8), "{ nicht json")
        // Auf eine freie Id gespeichert, liegt genau die Kopie im Verzeichnis.
        guard case .success(let free) = AssistantProfileTemplates.copyAsOwn(template, newId: "eigen-bbbbbbbbbbbb", newName: "Kopie") else { return XCTFail("zweite Kopie") }
        XCTAssertNoThrow(try s.save(free))
        XCTAssertEqual(s.profile(id: "eigen-bbbbbbbbbbbb")?.name, "Kopie")
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
