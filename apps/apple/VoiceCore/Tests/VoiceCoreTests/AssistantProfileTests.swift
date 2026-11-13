import Foundation
import XCTest
@testable import VoiceCore

// Assistenten-Profil (lva#555, Paket G1): Prüfung, Kodierung, Platzhalter, Rollenblock, Bereiche und Vorlagen. Die gemeinsamen
// Prüfstände (`apps/shared/assistenten/pruefstaende.json`) laufen hier und in `scripts/assistenten.test.mjs` des PC mit
// identischem Ergebnis. Nur synthetische Daten.

// MARK: - Helfer

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

private func values(_ tree: AssistantJSON) -> AssistantPlaceholderValues {
    var extra: [String: String] = [:]
    if case .object(let members)? = tree["extra"] {
        for member in members { extra[member.key] = member.value.string }
    }
    return AssistantPlaceholderValues(
        name: tree["name"]?.string,
        year: tree["jahr"]?.int ?? 0, month: tree["monat"]?.int ?? 0, day: tree["tag"]?.int ?? 0,
        weekday: tree["wochentag"]?.int ?? 0, hour: tree["stunde"]?.int ?? 0, minute: tree["minute"]?.int ?? 0,
        extra: extra
    )
}

private let defaultValues = AssistantPlaceholderValues(year: 2026, month: 1, day: 2, weekday: 5, hour: 9, minute: 0)

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
                XCTAssertEqual(text, expected?["kodiert"]?.string, name)
                // Rundreise: erneut lesen und schreiben ergibt dieselben Bytes.
                guard case .success(let again) = AssistantProfile.parse(json: text) else { XCTFail("\(name): Rundreise"); continue }
                XCTAssertEqual(again.encode(), text, name)
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
            let r = AssistantProfileText.resolve(c["vorlage"]?.string ?? "", language: c["sprache"]?.string ?? "de", values: values(c["werte"] ?? .null))
            XCTAssertEqual(r.text, c["erwartet"]?.string, c["name"]?.string ?? "?")
            XCTAssertEqual(r.unknown, c["unbekannt"]?.strings, c["name"]?.string ?? "?")
        }
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
            let r = AssistantProfileText.roleBlock(p, values: values(c["werte"] ?? .null), budgetChars: c["budget"]?.int ?? 0)
            XCTAssertEqual(r.text, c["erwartet"]?["text"]?.string, c["name"]?.string ?? "?")
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
        XCTAssertEqual(tree?.encoded(), "{\n  \"a\": true,\n  \"b\": [\n    1,\n    2.50,\n    {\n      \"a\": \"ä\",\n      \"z\": null\n    }\n  ]\n}\n")
        XCTAssertEqual(tree.flatMap { AssistantJSON.parse($0.encoded()) }?.encoded(), tree?.encoded())
    }

    func testCanonicalEquivalentKeysStayDistinct() {
        // "é" (ein Codepunkt) und "e" + U+0301 sind für Swift-Strings gleich, für das Format zwei Schlüssel.
        let tree = AssistantJSON.parse("{\"\u{00e9}\":1,\"e\u{0301}\":2}")
        guard case .object(let members)? = tree else { return XCTFail("kein Objekt") }
        XCTAssertEqual(members.count, 2)
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
        XCTAssertEqual(AssistantProfileTemplates.json + "\n", file, "AssistantProfileTemplates.swift ist veraltet: generate_assistant_templates.py laufen lassen")
        let tree = AssistantSharedFiles.tree("vorlagen.json")
        XCTAssertEqual(tree.encoded(), file)
        for (index, item) in tree.items.enumerated() {
            XCTAssertEqual(AssistantProfileTemplates.all[index].tree().encoded(), item.encoded())
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
        XCTAssertEqual(keys(schema["properties"]), ["anweisungen", "auswertung", "begruessung", "beschreibung", "id", "name", "schema", "schnittstellen", "sprache", "stimme", "verhalten", "wissen"])
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
