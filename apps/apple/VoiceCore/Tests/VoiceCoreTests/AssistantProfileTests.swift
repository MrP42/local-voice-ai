import Foundation
import XCTest
@testable import VoiceCore

// Assistenten-Profil (lva#555, Paket G1): Prüfung, Kodierung, Platzhalter, Rollenblock, Bereiche und Vorlagen. Die gemeinsamen
// Prüfstände (`apps/shared/assistenten/pruefstaende.json`) laufen hier und in `scripts/assistenten.test.mjs` des PC mit
// identischem Ergebnis. Nur synthetische Daten.

// MARK: - Helfer

/// Vergleicht zwei Texte über ihre UTF-8-Bytes. `XCTAssertEqual` auf `String` vergleicht kanonisch (`é` gleich `e` + U+0301) und
/// würde Unterschiede verschlucken, die Swift und PC auseinanderlaufen ließen.
func assertSameBytes(_ a: String?, _ b: String?, _ message: @autoclosure () -> String = "", file: StaticString = #filePath, line: UInt = #line) {
    if a.map({ Array($0.utf8) }) != b.map({ Array($0.utf8) }) {
        XCTFail("Bytes verschieden (\(message())): \(String(describing: a)) != \(String(describing: b))", file: file, line: line)
    }
}

func assertSameBytes(_ a: [String]?, _ b: [String]?, _ message: @autoclosure () -> String = "", file: StaticString = #filePath, line: UInt = #line) {
    if a.map({ $0.map { Array($0.utf8) } }) != b.map({ $0.map { Array($0.utf8) } }) {
        XCTFail("Bytes verschieden (\(message())): \(String(describing: a)) != \(String(describing: b))", file: file, line: line)
    }
}

enum AssistantSharedFiles {
    /// `apps/shared/assistenten`, ausgehend von dieser Quelldatei (auch über symbolische Verweise eines Scratch-Pakets).
    static let directory: URL = {
        var url = URL(fileURLWithPath: #filePath).resolvingSymlinksInPath()
        for _ in 0..<5 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("shared/assistenten")
    }()

    static func text(_ name: String) -> String {
        guard let data = try? Data(contentsOf: directory.appendingPathComponent(name)), let text = String(data: data, encoding: .utf8) else {
            XCTFail("Datei fehlt: \(name)")
            return ""
        }
        return text
    }

    static func tree(_ name: String) -> AssistantJSON {
        guard let tree = AssistantJSON.parse(text(name)) else {
            XCTFail("kein gültiges JSON: \(name)")
            return .null
        }
        return tree
    }
}

extension AssistantJSON {
    subscript(_ key: String) -> AssistantJSON? { member(key) }
    var string: String? { if case .string(let s) = self { return s }; return nil }
    var bool: Bool? { if case .bool(let b) = self { return b }; return nil }
    var int: Int? { if case .number(let raw) = self { return Int(raw) }; return nil }
    var items: [AssistantJSON] { if case .array(let a) = self { return a }; return [] }
    var strings: [String] { items.compactMap(\.string) }
}

private func issues(_ result: Result<AssistantProfile, AssistantProfileFailure>) -> [AssistantProfileIssue] {
    if case .failure(let failure) = result { return failure.issues }
    return []
}

private func profile(_ tree: AssistantJSON, file: StaticString = #filePath, line: UInt = #line) -> AssistantProfile {
    switch AssistantProfile.parse(tree: tree) {
    case .success(let p): return p
    case .failure(let f):
        XCTFail("Profil ungültig: \(f.issues)", file: file, line: line)
        return AssistantProfile(id: "x", name: "X")
    }
}

private func placeholderValues(_ tree: AssistantJSON) -> AssistantPlaceholderValues {
    // Die Mitgliederliste des JSON-Objekts 1:1 als Paarliste (kein Dictionary: kanonisch gleiche Schlüssel blieben nicht getrennt).
    var extra: [AssistantPlaceholderExtra] = []
    if case .object(let members)? = tree["extra"] {
        extra = members.compactMap { member in member.value.string.map { AssistantPlaceholderExtra(key: member.key, value: $0) } }
    }
    return AssistantPlaceholderValues(
        name: tree["name"]?.string,
        year: tree["jahr"]?.int ?? 0, month: tree["monat"]?.int ?? 0, day: tree["tag"]?.int ?? 0,
        weekday: tree["wochentag"]?.int ?? 0, hour: tree["stunde"]?.int ?? 0, minute: tree["minute"]?.int ?? 0,
        extra: extra
    )
}

private let defaultValues = AssistantPlaceholderValues(year: 2026, month: 1, day: 2, weekday: 5, hour: 9, minute: 0)

private extension String {
    func repeated(_ n: Int) -> String { String(repeating: self, count: n) }
}

/// Wandelt die Sonderformen der Prüfstände (`$zahl`, `$paare`, `$tief`) in einen Profilbaum (siehe `hinweis` in `pruefstaende.json`).
private func buildTree(_ node: AssistantJSON) -> AssistantJSON {
    switch node {
    case .array(let items):
        return .array(items.map(buildTree))
    case .object(let members):
        if members.count == 1 {
            let only = members[0]
            if AssistantExact.equal(only.key, "$zahl"), case .string(let raw) = only.value { return .number(raw) }
            if AssistantExact.equal(only.key, "$paare"), case .array(let pairs) = only.value {
                return .object(pairs.map { AssistantJSONMember($0.items[0].string ?? "", buildTree($0.items[1])) })
            }
            if AssistantExact.equal(only.key, "$tief"), case .number(let raw) = only.value, let levels = Int(raw), levels >= 1 {
                var out = AssistantJSON.array([])
                for _ in 1..<levels { out = .array([out]) }
                return out
            }
        }
        return .object(members.map { AssistantJSONMember($0.key, buildTree($0.value)) })
    default:
        return node
    }
}

final class AssistantProfileTests: XCTestCase {
    private let shared = AssistantSharedFiles.tree("pruefstaende.json")

    // MARK: Gemeinsame Prüfstände

    func testSharedCasesHaveEnoughVolumeAndEveryIssueCode() {
        let cases = shared["faelle"]?.items ?? []
        XCTAssertGreaterThanOrEqual(cases.count, 40)
        var codes = Set<String>()
        for c in cases { for e in c["erwartet"]?["fehler"]?.items ?? [] { codes.insert(e["code"]?.string ?? "") } }
        for code in AssistantProfileIssue.Code.allCases { XCTAssertTrue(codes.contains(code.rawValue), code.rawValue) }
    }

    func testSharedCases() {
        for c in shared["faelle"]?.items ?? [] {
            let name = c["name"]?.string ?? "?"
            let input = c["eingabe"] ?? .null
            let result: Result<AssistantProfile, AssistantProfileFailure>
            if let raw = input.string { result = AssistantProfile.parse(json: raw) } else { result = AssistantProfile.parse(tree: input) }
            let expected = c["erwartet"]
            if expected?["ok"]?.bool == true {
                guard case .success(let p) = result else { XCTFail("\(name): \(issues(result))"); continue }
                let text = p.encode()
                assertSameBytes(text, expected?["kodiert"]?.string, name)
                // Rundreise: erneut lesen und schreiben ergibt dieselben Bytes.
                guard case .success(let again) = AssistantProfile.parse(json: text) else { XCTFail("\(name): Rundreise"); continue }
                assertSameBytes(again.encode(), text, name)
                XCTAssertEqual(p.validate(), [], name)
            } else {
                let want = (expected?["fehler"]?.items ?? []).map { AssistantProfileIssue(path: $0["pfad"]?.string ?? "", code: AssistantProfileIssue.Code(rawValue: $0["code"]?.string ?? "") ?? .json) }
                XCTAssertEqual(issues(result), want, name)
            }
        }
    }

    func testSharedPlaceholders() {
        let cases = shared["platzhalter"]?.items ?? []
        XCTAssertGreaterThan(cases.count, 30)
        for c in cases {
            let r = AssistantProfileText.resolve(c["vorlage"]?.string ?? "", language: c["sprache"]?.string ?? "de", values: placeholderValues(c["werte"] ?? .null))
            assertSameBytes(r.text, c["erwartet"]?.string, c["name"]?.string ?? "?")
            assertSameBytes(r.unknown, c["unbekannt"]?.strings, c["name"]?.string ?? "?")
        }
    }

    func testCanonicallyEqualExtraKeysStayApartAndTheLastDuplicateWins() {
        let composed = "\u{00e9}"
        let decomposed = "e\u{0301}"
        let base = AssistantPlaceholderValues(year: 2026, month: 1, day: 2, weekday: 5, hour: 9, minute: 0)
        func sample(_ pairs: [(String, String)]) -> AssistantPlaceholderValues {
            var v = base
            v.extra = pairs.map { AssistantPlaceholderExtra(key: $0.0, value: $0.1) }
            return v
        }
        let template = "{{\(composed)}}|{{\(decomposed)}}"
        let both = sample([(composed, "A"), (decomposed, "B")])
        let reversed = sample([(decomposed, "B"), (composed, "A")])
        let result = AssistantProfileText.resolve(template, language: "de", values: both)
        XCTAssertEqual(Array(result.text.utf8), Array("A|B".utf8))
        XCTAssertEqual(result.unknown, [])
        XCTAssertEqual(Array(AssistantProfileText.resolve(template, language: "de", values: reversed).text.utf8), Array("A|B".utf8))
        // Gleiche Zuordnung in anderer Reihenfolge ist gleich; vertauschte Schlüssel sind es nicht.
        XCTAssertEqual(both, reversed)
        XCTAssertNotEqual(both, sample([(composed, "B"), (decomposed, "A")]))
        XCTAssertNotEqual(both, sample([(composed, "A")]))
        // Bei bytegleichem Schlüssel mehrfach gilt der letzte Eintrag; `==` vergleicht die wirksame Zuordnung.
        let twice = sample([("k", "erster"), ("k", "letzter")])
        XCTAssertEqual(Array(AssistantProfileText.resolve("{{k}}", language: "de", values: twice).text.utf8), Array("letzter".utf8))
        XCTAssertEqual(twice, sample([("k", "letzter")]))
        XCTAssertNotEqual(twice, sample([("k", "erster")]))
    }

    func testSharedRoleBlocks() {
        let cases = shared["rollenblock"]?.items ?? []
        XCTAssertGreaterThan(cases.count, 15)
        for c in cases {
            let p = profile(.object([
                AssistantJSONMember("schema", .string("lva.assistent/1")), AssistantJSONMember("id", .string("rolle")),
                AssistantJSONMember("name", c["profil"]?["name"] ?? .null),
                AssistantJSONMember("anweisungen", c["profil"]?["anweisungen"] ?? .null),
                AssistantJSONMember("sprache", c["profil"]?["sprache"] ?? .null),
            ]))
            let r = AssistantProfileText.roleBlock(p, values: placeholderValues(c["werte"] ?? .null), budgetChars: c["budget"]?.int ?? 0)
            assertSameBytes(r.text, c["erwartet"]?["text"]?.string, c["name"]?.string ?? "?")
            XCTAssertEqual(r.truncated, c["erwartet"]?["gekuerzt"]?.bool, c["name"]?.string ?? "?")
        }
    }

    func testSharedAreasIphone() {
        let cases = shared["bereiche_iphone"]?.items ?? []
        XCTAssertGreaterThan(cases.count, 8)
        for c in cases {
            let released = Set((c["freigegeben"]?.strings ?? []).compactMap { AssistantArea(rawValue: $0) })
            let got = profile(c["profil"] ?? .null).effectiveAreas(released: released).map(\.rawValue)
            XCTAssertEqual(got, c["erwartet"]?.strings, c["name"]?.string ?? "?")
        }
    }

    func testSharedContextMode() {
        for c in shared["kontextmodus"]?.items ?? [] {
            XCTAssertEqual(profile(c["profil"] ?? .null).effectiveContextMode.rawValue, c["erwartet"]?.string, c["name"]?.string ?? "?")
        }
    }

    // MARK: Eigenschaften

    /// Für jede Teilmenge freigegebener Bereiche gilt: wirksam ⊆ freigegeben (für Profile mit jeder Teilmenge bekannter Schlüssel).
    func testEffectiveAreasNeverExceedRelease() {
        let all = AssistantArea.allCases
        let keys = AssistantProfile.knownAreaKeys
        for keyMask in 0..<(1 << keys.count) {
            for allAreas in [false, true] {
                var p = AssistantProfile(id: "t", name: "T")
                p.knowledge.allAreas = allAreas
                p.knowledge.areas = keys.enumerated().filter { keyMask & (1 << $0.offset) != 0 }.map(\.element)
                for releaseMask in 0..<(1 << all.count) {
                    let released = Set(all.enumerated().filter { releaseMask & (1 << $0.offset) != 0 }.map(\.element))
                    let effective = p.effectiveAreas(released: released)
                    XCTAssertTrue(Set(effective).isSubset(of: released))
                    XCTAssertEqual(Set(effective).count, effective.count)
                    if !allAreas, keyMask == 0 { XCTAssertTrue(effective.isEmpty) }
                    if allAreas { XCTAssertEqual(Set(effective), released) }
                }
            }
        }
    }

    func testReleaseOpensNothingWithoutProfileWish() {
        let p = AssistantProfile(id: "t", name: "T")
        XCTAssertEqual(p.effectiveAreas(released: Set(AssistantArea.allCases)), [])
    }

    // MARK: Kodierung und Fehler

    func testUnknownFieldsSurviveAndTypedFieldsWin() {
        let text = #"{"schema":"lva.assistent/1","id":"x","name":"X","zukunft":{"a":[1,null,true,"t"]},"verhalten":{"neu":1},"wissen":{"neu":2,"kontexte":[{"art":"buch","id":"b","neu":3}]},"auswertung":{"neu":4,"felder":[{"name":"f","typ":"text","neu":5}]}}"#
        guard case .success(var p) = AssistantProfile.parse(json: text) else { return XCTFail("ungültig") }
        let first = p.encode()
        for part in ["\"zukunft\"", "\"neu\": 1", "\"neu\": 2", "\"neu\": 3", "\"neu\": 4", "\"neu\": 5"] { XCTAssertTrue(first.contains(part), part) }
        p.name = "Y"
        p.behavior.style = .literal
        let second = p.encode()
        XCTAssertTrue(second.contains("\"name\": \"Y\"") && second.contains("\"woertlich\""))
        XCTAssertTrue(second.contains("\"neu\": 1"))
    }

    func testOutputFormat() {
        let text = AssistantProfile(id: "a", name: "A").encode()
        XCTAssertTrue(text.hasPrefix("{\n  \"anweisungen\": \"\""))
        XCTAssertTrue(text.hasSuffix("}\n") && !text.hasSuffix("\n\n"))
        XCTAssertFalse(text.contains("\r") || text.contains("\t"))
    }

    func testIssuesAreSortedByPathThenCodeWithoutRepeats() {
        let failure = AssistantProfileFailure([
            AssistantProfileIssue(path: "b", code: .value), AssistantProfileIssue(path: "a[10]", code: .type),
            AssistantProfileIssue(path: "a[2]", code: .type), AssistantProfileIssue(path: "b", code: .length),
            AssistantProfileIssue(path: "b", code: .value),
        ])
        XCTAssertEqual(failure.issues.map { "\($0.path):\($0.code.rawValue)" }, ["a[10]:typ", "a[2]:typ", "b:laenge", "b:wert"])
    }

    func testValidateAfterEditingTypedFields() {
        var p = AssistantProfile(id: "gut", name: "Gut")
        XCTAssertEqual(p.validate(), [])
        p.id = "Böse"
        p.behavior.silenceFollowUpSeconds = 3
        p.knowledge.areas = ["a", "a"]
        XCTAssertEqual(p.validate().map { "\($0.path):\($0.code.rawValue)" }, ["id:muster", "verhalten.nachfrage_stille_s:wert", "wissen.bereiche[1]:doppelt"])
    }

    func testParseDataRejectsInvalidUTF8() {
        let result = AssistantProfile.parse(data: Data([0x7B, 0xFF, 0x7D]))
        XCTAssertEqual(issues(result), [AssistantProfileIssue(path: "", code: .json)])
    }

    func testJSONTreeRoundTripAndKeyOrder() {
        let tree = AssistantJSON.parse(#"{"b":[1,2.50,{"z":null,"a":"ä"}],"a":true}"#)
        assertSameBytes(tree?.encoded(), "{\n  \"a\": true,\n  \"b\": [\n    1,\n    2.50,\n    {\n      \"a\": \"ä\",\n      \"z\": null\n    }\n  ]\n}\n")
        assertSameBytes(tree.flatMap { AssistantJSON.parse($0.encoded()) }?.encoded(), tree?.encoded())
    }

    func testCanonicalEquivalentKeysStayDistinct() {
        // "é" (ein Codepunkt) und "e" + U+0301 sind für Swift-Strings gleich, für das Format zwei Schlüssel.
        let tree = AssistantJSON.parse("{\"\u{00e9}\":1,\"e\u{0301}\":2}")
        guard case .object(let members)? = tree else { return XCTFail("kein Objekt") }
        XCTAssertEqual(members.count, 2)
    }

    func testEqualityIsExactNotCanonical() {
        let composed = "\u{00e9}"
        let decomposed = "e\u{0301}"
        XCTAssertTrue(composed == decomposed, "Swift vergleicht kanonisch; genau das darf der Kern nicht übernehmen")
        XCTAssertNotEqual(AssistantJSON.string(composed), .string(decomposed))
        XCTAssertNotEqual(AssistantJSON.number("1"), .string("1"))
        XCTAssertNotEqual(AssistantJSONMember(composed, .null), AssistantJSONMember(decomposed, .null))
        XCTAssertNotEqual(AssistantJSON.array([.string(composed)]), .array([.string(decomposed)]))
        XCTAssertNotEqual(AssistantJSON.object([AssistantJSONMember(composed, .null)]), .object([AssistantJSONMember(decomposed, .null)]))
        XCTAssertEqual(AssistantJSON.object([AssistantJSONMember(composed, .null)]), .object([AssistantJSONMember(composed, .null)]))
        // Die Reihenfolge der Mitglieder eines Objekts zählt nicht (geschrieben wird sortiert), die einer Liste schon.
        let a = AssistantJSONMember("a", .number("1")), b = AssistantJSONMember("b", .null)
        XCTAssertEqual(AssistantJSON.object([a, b]), .object([b, a]))
        XCTAssertNotEqual(AssistantJSON.object([a, b]), .object([a, AssistantJSONMember("b", .bool(false))]))
        XCTAssertNotEqual(AssistantJSON.object([a]), .object([a, b]))
        XCTAssertNotEqual(AssistantJSON.array([.null, .bool(true)]), .array([.bool(true), .null]))
        func profile(_ edit: (inout AssistantProfile) -> Void, _ other: (inout AssistantProfile) -> Void) -> Bool {
            var a = AssistantProfile(id: "a", name: "A")
            var b = AssistantProfile(id: "a", name: "A")
            edit(&a)
            other(&b)
            return a == b
        }
        XCTAssertTrue(profile({ _ in }, { _ in }))
        XCTAssertFalse(profile({ $0.name = composed }, { $0.name = decomposed }))
        XCTAssertFalse(profile({ $0.descriptionText = composed }, { $0.descriptionText = decomposed }))
        XCTAssertFalse(profile({ $0.instructions = composed }, { $0.instructions = decomposed }))
        XCTAssertFalse(profile({ $0.greeting = composed }, { $0.greeting = decomposed }))
        XCTAssertFalse(profile({ $0.voice = composed }, { $0.voice = decomposed }))
        XCTAssertFalse(profile({ $0.voice = composed }, { $0.voice = nil }))
        XCTAssertFalse(profile({ $0.interfaces = [composed] }, { $0.interfaces = [decomposed] }))
        XCTAssertFalse(profile({ $0.knowledge.areas = [composed] }, { $0.knowledge.areas = [decomposed] }))
        XCTAssertFalse(profile({ $0.knowledge.contexts = [.init(kind: .book, id: composed)] }, { $0.knowledge.contexts = [.init(kind: .book, id: decomposed)] }))
        XCTAssertFalse(profile({ $0.knowledge.contexts = [.init(kind: .book, id: "k", title: composed)] }, { $0.knowledge.contexts = [.init(kind: .book, id: "k", title: decomposed)] }))
        XCTAssertFalse(profile({ $0.evaluation.fields = [.init(name: "f", options: [composed])] }, { $0.evaluation.fields = [.init(name: "f", options: [decomposed])] }))
        XCTAssertFalse(profile({ $0.evaluation.fields = [.init(name: "f", descriptionText: composed)] }, { $0.evaluation.fields = [.init(name: "f", descriptionText: decomposed)] }))
        XCTAssertFalse(profile({ $0.extra = [AssistantJSONMember(composed, .null)] }, { $0.extra = [AssistantJSONMember(decomposed, .null)] }))
        XCTAssertFalse(profile({ $0.behavior.extra = [AssistantJSONMember("x", .string(composed))] }, { $0.behavior.extra = [AssistantJSONMember("x", .string(decomposed))] }))
        XCTAssertFalse(profile({ $0.knowledge.extra = [AssistantJSONMember("x", .string(composed))] }, { $0.knowledge.extra = [AssistantJSONMember("x", .string(decomposed))] }))
        XCTAssertFalse(profile({ $0.evaluation.extra = [AssistantJSONMember("x", .string(composed))] }, { $0.evaluation.extra = [AssistantJSONMember("x", .string(decomposed))] }))
        XCTAssertTrue(profile({ $0.name = composed }, { $0.name = composed }))
        // Platzhalterwerte und Ergebnisse
        let base = AssistantPlaceholderValues(year: 2026, month: 1, day: 2, weekday: 5, hour: 9, minute: 0)
        func sample(_ name: String?, _ pairs: [(String, String)]) -> AssistantPlaceholderValues {
            var v = base
            v.name = name
            v.extra = pairs.map { AssistantPlaceholderExtra(key: $0.0, value: $0.1) }
            return v
        }
        XCTAssertNotEqual(sample(composed, []), sample(decomposed, []))
        XCTAssertNotEqual(sample(nil, [(composed, "x")]), sample(nil, [(decomposed, "x")]))
        XCTAssertNotEqual(sample(nil, [("k", composed)]), sample(nil, [("k", decomposed)]))
        XCTAssertEqual(sample(nil, [(composed, "x")]), sample(nil, [(composed, "x")]))
        XCTAssertNotEqual(AssistantPlaceholderResult(text: composed, unknown: []), AssistantPlaceholderResult(text: decomposed, unknown: []))
        XCTAssertNotEqual(AssistantPlaceholderResult(text: "", unknown: [composed]), AssistantPlaceholderResult(text: "", unknown: [decomposed]))
        XCTAssertNotEqual(AssistantRoleBlock(text: composed, truncated: false), AssistantRoleBlock(text: decomposed, truncated: false))
        XCTAssertNotEqual(AssistantRejectedFile(fileName: composed + ".json", issues: []), AssistantRejectedFile(fileName: decomposed + ".json", issues: []))
    }

    func testCanonicalEquivalentDuplicatesAreNotDuplicates() {
        // Bereichsschlüssel und Feldnamen sind ASCII; ein Akzent ist ein Musterfehler, nie ein Duplikat.
        var p = AssistantProfile(id: "a", name: "A")
        p.knowledge.areas = ["\u{00e9}", "e\u{0301}"]
        XCTAssertEqual(p.validate().map { "\($0.path):\($0.code.rawValue)" }, ["wissen.bereiche[0]:muster", "wissen.bereiche[1]:muster"])
        // Optionen: kein Duplikat-Test, beide bleiben byteweise erhalten.
        var q = AssistantProfile(id: "a", name: "A")
        q.evaluation.fields = [AssistantProfile.Field(name: "w", kind: .choice, options: ["\u{00e9}", "e\u{0301}"])]
        XCTAssertEqual(q.validate(), [])
        guard case .success(let again) = AssistantProfile.parse(json: q.encode()) else { return XCTFail("Rundreise") }
        XCTAssertEqual(again.evaluation.fields[0].options.map { Array($0.utf8) }, [Array("\u{00e9}".utf8), Array("e\u{0301}".utf8)])
    }

    // MARK: Ids, Gerätenamen, vorbehaltene Ids

    func testSharedIds() {
        let cases = shared["ids"]?.items ?? []
        XCTAssertGreaterThan(cases.count, 60)
        var sources: [AssistantProfile] = []
        sources.append(AssistantProfileTemplates.all[1])
        for c in cases {
            let id = c["id"]?.string ?? ""
            let tree = AssistantJSON.object([
                AssistantJSONMember("schema", .string("lva.assistent/1")), AssistantJSONMember("id", .string(id)),
                AssistantJSONMember("name", .string("N")),
            ])
            let result = AssistantProfile.parse(tree: tree)
            let expected = c["erwartet"]?.string ?? "?"
            if expected == "ok" {
                guard case .success = result else { XCTFail("Id \(id.debugDescription): \(issues(result))"); continue }
            } else {
                XCTAssertEqual(issues(result), [AssistantProfileIssue(path: "id", code: AssistantProfileIssue.Code(rawValue: expected) ?? .json)], id.debugDescription)
            }
            XCTAssertEqual(AssistantProfileTemplates.isReservedId(id), c["vorbehalten"]?.bool, id.debugDescription)
            let copy = AssistantProfileTemplates.copyAsOwn(sources[0], newId: id, newName: "Kopie")
            if expected == "ok", c["vorbehalten"]?.bool == false {
                guard case .success = copy else { XCTFail("Kopie \(id.debugDescription)"); continue }
            } else if expected == "ok" {
                XCTAssertEqual(issues(copy), [AssistantProfileIssue(path: "id", code: .value)], id.debugDescription)
            } else {
                XCTAssertTrue(issues(copy).contains(AssistantProfileIssue(path: "id", code: AssistantProfileIssue.Code(rawValue: expected) ?? .json)), id.debugDescription)
            }
        }
    }

    func testDeviceNamesAreExactlyTheWindowsOnes() {
        var expected = Set(["con", "prn", "aux", "nul"])
        for n in 0..<10 { expected.insert("com\(n)"); expected.insert("lpt\(n)") }
        var candidates = Array(expected) + ["con1", "com10", "nul-1", "console", "co", ""]
        let alphabet = Array("conprauxnlmt019-")
        for a in alphabet { for b in alphabet { for c in alphabet {
            let base = String([a, b, c])
            candidates += [base, base + "0", base + "9"]
        } } }
        for k in candidates { XCTAssertEqual(AssistantProfilePatterns.isDeviceName(k), expected.contains(k), k) }
    }

    func testCopyOnAktivIsRejectedAsReserved() {
        let source = AssistantProfileTemplates.all[1]
        XCTAssertEqual(issues(AssistantProfileTemplates.copyAsOwn(source, newId: "aktiv", newName: "X")), [AssistantProfileIssue(path: "id", code: .value)])
        XCTAssertTrue(AssistantProfileTemplates.isReservedId("aktiv"))
        XCTAssertFalse(AssistantProfileTemplates.isReservedId("aktiv-2"))
        XCTAssertFalse(AssistantProfileTemplates.isReservedId("Aktiv"))
        XCTAssertFalse(AssistantProfileTemplates.isTemplate("aktiv"))
    }

    // MARK: Prüfung des Baums

    func testSharedTrees() {
        let cases = shared["baeume"]?.items ?? []
        XCTAssertGreaterThan(cases.count, 25)
        for c in cases {
            let name = c["name"]?.string ?? "?"
            let result = AssistantProfile.parse(tree: buildTree(c["baum"] ?? .null))
            let expected = c["erwartet"]
            if expected?["ok"]?.bool == true {
                guard case .success(let p) = result else { XCTFail("\(name): \(issues(result))"); continue }
                let text = p.encode()
                assertSameBytes(text, expected?["kodiert"]?.string, name)
                guard case .success(let again) = AssistantProfile.parse(json: text) else { XCTFail("\(name): nicht lesbar"); continue }
                assertSameBytes(again.encode(), text, name)
                XCTAssertEqual(p.validate(), [], name)
            } else {
                let want = (expected?["fehler"]?.items ?? []).map { AssistantProfileIssue(path: $0["pfad"]?.string ?? "", code: AssistantProfileIssue.Code(rawValue: $0["code"]?.string ?? "") ?? .json) }
                XCTAssertEqual(issues(result), want, name)
            }
        }
    }

    /// Grundsatz: Was die Prüfung besteht, schreibt `encode` und liest `parse` wieder, bytegleich. Jeder Zahlentext bis Länge 5.
    func testEveryNumberTextEitherFailsOrRoundTrips() throws {
        let grammar = try NSRegularExpression(pattern: "^-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?$")
        let alphabet = ["0", "1", "-", "+", ".", "e", "E", "x", " "]
        var texts = [""]
        var previous = [""]
        for _ in 1...5 {
            previous = previous.flatMap { t in alphabet.map { t + $0 } }
            texts += previous
        }
        XCTAssertGreaterThan(texts.count, 60000)
        for raw in texts {
            let tree = AssistantJSON.object([
                AssistantJSONMember("schema", .string("lva.assistent/1")), AssistantJSONMember("id", .string("a")),
                AssistantJSONMember("name", .string("A")), AssistantJSONMember("x", .array([.number(raw)])),
            ])
            let valid = grammar.firstMatch(in: raw, range: NSRange(raw.startIndex..., in: raw)) != nil
            let result = AssistantProfile.parse(tree: tree)
            if valid {
                guard case .success(let p) = result else { XCTFail("gültige Zahl abgewiesen: \(raw.debugDescription)"); continue }
                let text = p.encode()
                guard case .success(let again) = AssistantProfile.parse(json: text) else { XCTFail("nicht lesbar: \(raw.debugDescription)"); continue }
                XCTAssertEqual(Array(again.encode().utf8), Array(text.utf8), raw.debugDescription)
            } else {
                XCTAssertEqual(issues(result), [AssistantProfileIssue(path: "x", code: .json)], raw.debugDescription)
            }
        }
    }

    func testValidateReportsUnreadableExtrasAndOverDeepTrees() {
        var p = AssistantProfile(id: "a", name: "A")
        p.extra = [AssistantJSONMember("x", .number("NaN"))]
        XCTAssertEqual(p.validate(), [AssistantProfileIssue(path: "x", code: .json)])
        var deep = AssistantJSON.array([])
        for _ in 0..<70 { deep = .array([deep]) }
        p.extra = [AssistantJSONMember("y", deep)]
        XCTAssertEqual(p.validate(), [AssistantProfileIssue(path: "y", code: .json)])
        p.extra = [AssistantJSONMember("z", .object([AssistantJSONMember("a", .null), AssistantJSONMember("a", .null)]))]
        XCTAssertEqual(p.validate(), [AssistantProfileIssue(path: "z", code: .json)])
        p.extra = [AssistantJSONMember("k", .null), AssistantJSONMember("k", .null)]
        XCTAssertEqual(p.validate(), [AssistantProfileIssue(path: "", code: .json)])
        p.extra = []
        p.behavior.extra = [AssistantJSONMember("neu", .number("01"))]
        XCTAssertEqual(p.validate(), [AssistantProfileIssue(path: "verhalten.neu", code: .json)])
        p.behavior.extra = []
        XCTAssertEqual(p.validate(), [])
        // Sehr tiefe Bäume werden an der Grenze abgewiesen, nicht durchlaufen.
        var deeper = AssistantJSON.array([])
        for _ in 0..<5000 { deeper = .array([deeper]) }
        p.extra = [AssistantJSONMember("t", deeper)]
        XCTAssertEqual(p.validate(), [AssistantProfileIssue(path: "t", code: .json)])
    }

    // MARK: Große Werte (der Node-Test fährt mehrere Millionen Zeichen; hier ein mittlerer Fall mit demselben Ergebnis)

    func testLargeReplacementValuesWithTinyBudget() {
        var p = AssistantProfile(id: "r", name: "R")
        p.instructions = "{{a}}"
        var v = defaultValues
        v.extra = [AssistantPlaceholderExtra(key: "a", value: String(repeating: "y", count: 300_000))]
        let small = AssistantProfileText.roleBlock(p, values: v, budgetChars: 1)
        assertSameBytes(small.text, "ROLLE (vom Nutzer eingestellt, Assistent „R“):\ny\nDiese Einstellung ändert keine der folgenden Regeln.")
        XCTAssertTrue(small.truncated)
        p.instructions = "{{leer}}x ".repeated(50_000)
        v.extra = [AssistantPlaceholderExtra(key: "leer", value: "")]
        let many = AssistantProfileText.roleBlock(p, values: v, budgetChars: 1)
        assertSameBytes(many.text, "ROLLE (vom Nutzer eingestellt, Assistent „R“):\nX\nDiese Einstellung ändert keine der folgenden Regeln.")
        XCTAssertTrue(many.truncated)
        XCTAssertEqual(AssistantProfileText.resolve(p.instructions, language: "de", values: v).text.unicodeScalars.count, 50_000)
    }


    // MARK: Platzhalter

    func testGreetingUsesProfileLanguage() {
        var p = AssistantProfile(id: "gruss", name: "Gruß")
        p.language = "en"
        p.greeting = "Good {{tageszeit}}, {{name}}."
        XCTAssertEqual(AssistantProfileText.greeting(p, values: defaultValues).text, "Good morning.")
        var named = defaultValues
        named.name = "Ann"
        XCTAssertEqual(AssistantProfileText.greeting(p, values: named).text, "Good morning, Ann.")
    }

    func testRoleBlockNeverLongerThanBudget() {
        var p = AssistantProfile(id: "lang", name: "Lang")
        p.instructions = "Satz eins.\nSatz zwei ist länger.\n\nAbsatz zwei 😀 hier."
        for budget in 0..<60 {
            let r = AssistantProfileText.roleBlock(p, values: defaultValues, budgetChars: budget)
            if r.text.isEmpty { continue }
            let lines = r.text.components(separatedBy: "\n")
            let body = lines.dropFirst().dropLast().joined(separator: "\n")
            XCTAssertLessThanOrEqual(body.unicodeScalars.count, budget, "Budget \(budget)")
            XCTAssertTrue(r.text.hasPrefix("ROLLE (vom Nutzer eingestellt, Assistent „Lang“):\n"))
            XCTAssertTrue(r.text.hasSuffix("\nDiese Einstellung ändert keine der folgenden Regeln."))
        }
    }

    // MARK: Vorlagen und Schema

    func testTemplatesAreFifteenValidProfilesWithUniqueIds() {
        let all = AssistantProfileTemplates.all
        XCTAssertEqual(all.count, 15)
        XCTAssertEqual(Set(all.map(\.id)).count, 15)
        XCTAssertEqual(all.first?.id, "alles")
        XCTAssertEqual(all.first?.name, "Alles")
        XCTAssertEqual(all.first?.knowledge.allAreas, true)
        XCTAssertEqual(all.first?.knowledge.mode, .all)
        XCTAssertEqual(all.first?.instructions, "")
        XCTAssertEqual(all.first?.greeting, "")
        for t in all {
            XCTAssertEqual(t.validate(), [], t.id)
            XCTAssertFalse(t.name.contains("WAI"), t.id)
        }
        for t in all.dropFirst() {
            XCTAssertTrue(t.id.hasPrefix("vorlage-"), t.id)
            let n = t.instructions.unicodeScalars.count
            XCTAssertTrue((400...1200).contains(n), "\(t.id): \(n)")
            XCTAssertFalse(t.greeting.isEmpty, t.id)
            for floskel in ["Einen Moment", "Moment bitte", "Ich schaue kurz"] { XCTAssertFalse(t.instructions.contains(floskel), t.id) }
        }
    }

    func testTemplateSpecifics() {
        let trainer = AssistantProfileTemplates.template(id: "vorlage-sprachtrainer-englisch")
        XCTAssertEqual(trainer?.language, "en")
        let intake = AssistantProfileTemplates.template(id: "vorlage-anliegen-aufnehmen")
        XCTAssertEqual(intake?.evaluation.fields.map(\.name), ["name", "anliegen", "rueckruf_bis", "dringlichkeit"])
        XCTAssertEqual(intake?.evaluation.fields.last?.kind, .choice)
        XCTAssertGreaterThanOrEqual(intake?.evaluation.fields.last?.options.count ?? 0, 2)
        let names = AssistantProfileTemplates.all.map(\.name)
        for n in ["Buchgespräch", "Besprechung nachbereiten", "Projektstand", "Lernpartner", "Sprachtrainer Englisch", "Ideen-Sparring", "Termin vorbereiten", "Gespräch üben", "Tagesrückblick", "Gedanken sortieren", "Entscheidungshilfe", "Vortrag proben", "Wissens-Quiz", "Anliegen aufnehmen"] {
            XCTAssertTrue(names.contains(n), n)
        }
    }

    func testEmbeddedTemplatesMatchSharedFileAndAreCanonical() {
        let file = AssistantSharedFiles.text("vorlagen.json")
        assertSameBytes(AssistantProfileTemplates.json + "\n", file, "AssistantProfileTemplates.swift ist veraltet: generate_assistant_templates.py laufen lassen")
        let tree = AssistantSharedFiles.tree("vorlagen.json")
        assertSameBytes(tree.encoded(), file)
        for (index, item) in tree.items.enumerated() {
            assertSameBytes(AssistantProfileTemplates.all[index].tree().encoded(), item.encoded())
        }
    }

    func testTemplatesAreReadOnlyAndCopyIsChecked() {
        XCTAssertTrue(AssistantProfileTemplates.isTemplate("alles"))
        XCTAssertTrue(AssistantProfileTemplates.isReservedId("vorlage-neu"))
        XCTAssertFalse(AssistantProfileTemplates.isReservedId("eigen-abc123def456"))
        let source = AssistantProfileTemplates.all[1]
        guard case .success(let copy) = AssistantProfileTemplates.copyAsOwn(source, newId: "eigen-abc123def456", newName: "  Mein Buch  ") else { return XCTFail("Kopie") }
        XCTAssertEqual(copy.id, "eigen-abc123def456")
        XCTAssertEqual(copy.name, "Mein Buch")
        XCTAssertEqual(copy.instructions, source.instructions)
        XCTAssertEqual(AssistantProfileTemplates.all[1].id, "vorlage-buchgespraech")
        XCTAssertEqual(issues(AssistantProfileTemplates.copyAsOwn(source, newId: "alles", newName: "X")), [AssistantProfileIssue(path: "id", code: .value)])
        XCTAssertEqual(issues(AssistantProfileTemplates.copyAsOwn(source, newId: "Böse", newName: "")).map { "\($0.path):\($0.code.rawValue)" }, ["id:muster", "name:laenge"])
    }

    func testSchemaFileIsValidJSONWithAllFields() {
        let schema = AssistantSharedFiles.tree("assistent.schema.json")
        XCTAssertEqual(schema["$schema"]?.string, "https://json-schema.org/draft/2020-12/schema")
        func keys(_ node: AssistantJSON?) -> [String] {
            guard case .object(let members)? = node else { return [] }
            return members.map(\.key).sorted()
        }
        XCTAssertEqual(keys(schema["properties"]), ["anweisungen", "aussprache", "auswertung", "begruessung", "beschreibung", "id", "name", "schema", "schnittstellen", "sprache", "stimme", "verhalten", "wissen"])
        XCTAssertEqual(schema["required"]?.strings.sorted(), ["id", "name", "schema"])
        XCTAssertEqual(keys(schema["properties"]?["verhalten"]?["properties"]), ["nachfrage_stille_s", "sprechpause", "stil", "unterbrechen"])
        XCTAssertEqual(keys(schema["properties"]?["wissen"]?["properties"]), ["alle_bereiche", "bereiche", "kontexte", "modus"])
        XCTAssertEqual(keys(schema["properties"]?["auswertung"]?["properties"]), ["aufgaben", "felder", "zusammenfassung"])
        XCTAssertEqual(schema["properties"]?["schema"]?["const"]?.string, "lva.assistent/1")
    }

    func testReadmeMentionsStorageFormat() {
        let readme = AssistantSharedFiles.text("README.md")
        for word in ["lva.assistent/1", "Speicherformat", "aktiv.json", "Auslegung"] { XCTAssertTrue(readme.contains(word), word) }
    }
}
