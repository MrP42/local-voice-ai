import Foundation
import XCTest
@testable import VoiceCore

// Auswertung am Gesprächsende (lva#559, Paket G5-Kern): Prompt, Lesen der Rohantwort mit Erdung, Fristauflösung, Kodierung. Die
// gemeinsamen Prüfstände (`apps/shared/assistenten/auswertung-pruefstaende.json`) laufen hier und in
// `scripts/auswertung.test.mjs` des PC mit identischem Ergebnis. Nur erfundene Gespräche.

private func outcomeProfile(_ tree: AssistantJSON, file: StaticString = #filePath, line: UInt = #line) -> AssistantProfile {
    switch AssistantProfile.parse(tree: tree) {
    case .success(let p): return p
    case .failure(let f):
        XCTFail("Profil ungültig: \(f.issues)", file: file, line: line)
        return AssistantProfile(id: "x", name: "X")
    }
}

private func outcomeTurns(_ tree: AssistantJSON?) -> [AssistantOutcomeTurn] {
    (tree?.items ?? []).map { item in
        AssistantOutcomeTurn(role: item["rolle"]?.string == "assistent" ? .assistant : .user, text: item["text"]?.string ?? "")
    }
}

private func outcomeDate(_ tree: AssistantJSON?) -> AssistantOutcomeDate {
    AssistantOutcomeDate(year: tree?["jahr"]?.int ?? 0, month: tree?["monat"]?.int ?? 0, day: tree?["tag"]?.int ?? 0)
}

private func outcomeHints(_ tree: AssistantJSON?) -> [AssistantOutcomeHint] {
    (tree?.items ?? []).map { AssistantOutcomeHint(path: $0["pfad"]?.string ?? "", code: AssistantOutcomeHint.Code(rawValue: $0["code"]?.string ?? "") ?? .type) }
}

/// Rohantwort eines gemeinsamen Falls: `roh`, oder `roh_teile` (Text, `anzahl`-mal wiederholt, hintereinander).
private func outcomeRaw(_ c: AssistantJSON) -> String {
    if let parts = c["roh_teile"]?.items {
        var out = ""
        for part in parts { out += String(repeating: part["text"]?.string ?? "", count: part["anzahl"]?.int ?? 0) }
        return out
    }
    return c["roh"]?.string ?? ""
}

final class AssistantOutcomeTests: XCTestCase {
    private let shared = AssistantSharedFiles.tree("auswertung-pruefstaende.json")
    private let today = AssistantOutcomeDate(year: 2026, month: 11, day: 13)

    private func simpleProfile(fields: AssistantJSON = .array([]), summary: Bool = false, tasks: Bool = false) -> AssistantProfile {
        outcomeProfile(.object([
            AssistantJSONMember("schema", .string("lva.assistent/1")),
            AssistantJSONMember("id", .string("eigen-aaaaaaaaaaaa")),
            AssistantJSONMember("name", .string("T")),
            AssistantJSONMember("auswertung", .object([
                AssistantJSONMember("zusammenfassung", .bool(summary)),
                AssistantJSONMember("aufgaben", .bool(tasks)),
                AssistantJSONMember("felder", fields),
            ])),
        ]))
    }

    private func normalizedText(_ turns: [AssistantOutcomeTurn], _ n: Int) -> [UInt32] {
        AssistantOutcomeText.normalizedScalars(turns[n - 1].text)
    }

    // MARK: Gemeinsame Prüfstände

    func testSharedVolume() {
        XCTAssertGreaterThanOrEqual(shared["auswerten"]?.items.count ?? 0, 12)
        XCTAssertGreaterThanOrEqual(shared["frist"]?.items.count ?? 0, 40)
        XCTAssertGreaterThanOrEqual(shared["prompt"]?.items.count ?? 0, 6)
        var codes = Set<String>()
        for c in shared["auswerten"]?.items ?? [] { for h in c["erwartet"]?["hinweise"]?.items ?? [] { codes.insert(h["code"]?.string ?? "") } }
        for code in AssistantOutcomeHint.Code.allCases { XCTAssertTrue(codes.contains(code.rawValue), code.rawValue) }
    }

    func testSharedEvaluateCases() {
        for c in shared["auswerten"]?.items ?? [] {
            let name = c["name"]?.string ?? "?"
            let profile = outcomeProfile(c["profil"] ?? .null)
            let turns = outcomeTurns(c["verlauf"])
            let result = AssistantOutcome.evaluate(
                profile: profile, turns: turns, raw: outcomeRaw(c), today: outcomeDate(c["heute"]),
                created: c["erstellt"]?.string ?? "", truncated: c["gekuerzt"]?.bool ?? false
            )
            let expected = c["erwartet"]
            if expected?["ok"]?.bool != true {
                guard case .failure(let failure) = result else { XCTFail("\(name): Fehler erwartet"); continue }
                XCTAssertEqual(failure.rawValue, expected?["fehler"]?.string, name)
                continue
            }
            guard case .success(let evaluation) = result else { XCTFail("\(name): kein Ergebnis"); continue }
            let text = evaluation.outcome.encode()
            XCTAssertEqual(text, expected?["kodiert"]?.string, name)
            XCTAssertEqual(evaluation.hints, outcomeHints(expected?["hinweise"]), name)
            // Rundreise: erneut lesen und schreiben ergibt dieselben Bytes.
            guard case .success(let again) = AssistantOutcome.parse(json: text) else { XCTFail("\(name): Rundreise"); continue }
            XCTAssertEqual(again.encode(), text, name)
            // Abnahme: kein Wert ohne Beleg im Gespräch.
            for field in evaluation.outcome.fields {
                guard let quote = field.quote, let turn = field.turn else {
                    XCTAssertNil(field.value, name)
                    continue
                }
                XCTAssertTrue(AssistantOutcomeText.contains(normalizedText(turns, turn), AssistantOutcomeText.normalizedScalars(quote)), name)
            }
            for task in evaluation.outcome.tasks {
                XCTAssertTrue(AssistantOutcomeText.contains(normalizedText(turns, task.turn), AssistantOutcomeText.normalizedScalars(task.quote)), name)
            }
        }
    }

    func testSharedDeadlines() {
        for c in shared["frist"]?.items ?? [] {
            let expression = c["ausdruck"]?.string ?? ""
            let d = outcomeDate(c["heute"])
            XCTAssertEqual(AssistantOutcomeDates.deadline(expression, today: d), c["erwartet"]?.string, "\(expression) ab \(d)")
        }
    }

    func testSharedNormalization() {
        for c in shared["normalisierung"]?.items ?? [] {
            let input = c["eingabe"]?.string ?? ""
            let got = AssistantOutcomeText.normalize(input)
            XCTAssertTrue(got.utf8.elementsEqual((c["erwartet"]?.string ?? "").utf8), "\(input) -> \(got)")
        }
    }

    func testSharedParseErrors() {
        for c in shared["lesen_fehler"]?.items ?? [] {
            let input = c["eingabe"] ?? .null
            let text = input.string ?? input.encoded()
            guard case .failure(let failure) = AssistantOutcome.parse(json: text) else { XCTFail(c["name"]?.string ?? "?"); continue }
            let want = (c["erwartet"]?["fehler"]?.items ?? []).map {
                AssistantProfileIssue(path: $0["pfad"]?.string ?? "", code: AssistantProfileIssue.Code(rawValue: $0["code"]?.string ?? "") ?? .json)
            }
            XCTAssertEqual(failure.issues, want, c["name"]?.string ?? "?")
        }
    }

    func testSharedPrompts() {
        for c in shared["prompt"]?.items ?? [] {
            let name = c["name"]?.string ?? "?"
            let p = AssistantOutcome.prompt(
                profile: outcomeProfile(c["profil"] ?? .null), turns: outcomeTurns(c["verlauf"]),
                today: outcomeDate(c["heute"]), budgetChars: c["budget"]?.int ?? 0
            )
            XCTAssertTrue(p.text.utf8.elementsEqual((c["erwartet"]?["text"]?.string ?? "").utf8), name)
            XCTAssertEqual(p.truncated, c["erwartet"]?["gekuerzt"]?.bool, name)
        }
    }

    // MARK: Eigenschaften

    func testPromptTruncationKeepsBudgetHeadAndTail() {
        let profile = simpleProfile()
        let turns = (0..<60).map { i in
            AssistantOutcomeTurn(role: i % 2 == 0 ? .user : .assistant, text: "Beitrag Nummer \(i + 1) mit etwas Text zum Füllen der Zeile.")
        }
        let full = AssistantOutcome.prompt(profile: profile, turns: turns, today: today, budgetChars: 1_000_000_000)
        XCTAssertFalse(full.truncated)
        for budget in [1500, 1800, 2200, 3000] {
            let p = AssistantOutcome.prompt(profile: profile, turns: turns, today: today, budgetChars: budget)
            XCTAssertTrue(p.truncated)
            XCTAssertLessThanOrEqual(p.text.unicodeScalars.count, budget)
            let lines = p.text.components(separatedBy: "\n")
            guard let i = lines.firstIndex(where: { $0.hasPrefix("[1] ") }) else { XCTFail("kein Beitrag 1"); continue }
            XCTAssertTrue(lines[i + 1].hasPrefix("[2] "))
            XCTAssertTrue(lines[i + 2].hasPrefix("[… ") && lines[i + 2].hasSuffix(" Beiträge ausgelassen …]"))
            XCTAssertTrue(lines[lines.count - 1].hasPrefix("[60] "))
            let kept = lines.count - (i + 3)
            let omitted = Int(lines[i + 2].dropFirst(3).prefix { $0.isNumber }) ?? -1
            XCTAssertEqual(omitted, 60 - 2 - kept)
            let more = AssistantOutcome.prompt(profile: profile, turns: turns, today: today, budgetChars: budget + 80)
            XCTAssertGreaterThanOrEqual(more.text.components(separatedBy: "\n").count, lines.count)
        }
    }

    func testTranscriptNumbersAndCollapsesWhitespace() {
        XCTAssertEqual(
            AssistantOutcome.transcript([
                AssistantOutcomeTurn(role: .user, text: " a \n b\t c "),
                AssistantOutcomeTurn(role: .assistant, text: ""),
            ]),
            ["[1] Nutzer: a b c", "[2] Assistent:"]
        )
    }

    func testGroundingNeverCreatesAValueWithoutQuote() {
        func field(_ name: String, _ kind: String, options: [String] = []) -> AssistantJSON {
            .object([
                AssistantJSONMember("name", .string(name)), AssistantJSONMember("typ", .string(kind)),
                AssistantJSONMember("optionen", .array(options.map { .string($0) })),
            ])
        }
        let profile = simpleProfile(
            fields: .array([field("a", "text"), field("b", "zahl"), field("c", "datum"), field("d", "auswahl", options: ["x", "y"])]),
            summary: true, tasks: true
        )
        let turns = [
            AssistantOutcomeTurn(role: .user, text: "Es geht um 5 Dinge bis morgen."),
            AssistantOutcomeTurn(role: .assistant, text: "Gut."),
        ]
        let values = ["5", "x", "2026-11-14", "fünf", "morgen", "99", "y", ""]
        let quotes = ["5 Dinge", "bis morgen", "Gut.", "erfunden", "", "Es geht"]
        let stati = ["klar", "unklar", "nicht_genannt"]
        var state = 7
        func random(_ n: Int) -> Int {
            state = (state * 1_103_515_245 + 12345) % 2_147_483_648
            return state % n
        }
        func object(_ members: [(String, AssistantJSON)]) -> AssistantJSON { .object(members.map { AssistantJSONMember($0.0, $0.1) }) }
        for _ in 0..<300 {
            let fields = ["a", "b", "c", "d"].map { name in
                object([
                    ("name", .string(name)), ("status", .string(stati[random(3)])), ("wert", .string(values[random(values.count)])),
                    ("beleg", .string(quotes[random(quotes.count)])), ("beitrag", .integer(random(4))),
                ])
            }
            let tasks = [object([("text", .string("t")), ("frist_text", .string("morgen")), ("beleg", .string(quotes[random(quotes.count)])), ("beitrag", .integer(1))])]
            let raw = object([("felder", .array(fields)), ("aufgaben", .array(tasks))]).encoded()
            guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: turns, raw: raw, today: today) else {
                XCTFail("kein Ergebnis")
                return
            }
            for f in evaluation.outcome.fields {
                if f.value != nil {
                    guard let quote = f.quote, let turn = f.turn else { XCTFail("Wert ohne Beleg"); continue }
                    XCTAssertTrue(AssistantOutcomeText.contains(normalizedText(turns, turn), AssistantOutcomeText.normalizedScalars(quote)))
                }
                if f.status == .clear { XCTAssertNotNil(f.value) }
                if f.status == .notMentioned { XCTAssertTrue(f.value == nil && f.quote == nil && f.turn == nil) }
            }
            for t in evaluation.outcome.tasks {
                XCTAssertTrue(AssistantOutcomeText.contains(normalizedText(turns, t.turn), AssistantOutcomeText.normalizedScalars(t.quote)))
            }
        }
    }

    func testHintsCarryOnlyPathAndCode() {
        let profile = simpleProfile()
        let secret = "GEHEIMER-SCHLUESSEL-TEXT"
        let raw = AssistantJSON.object([
            AssistantJSONMember(secret, .integer(1)),
            AssistantJSONMember(secret.lowercased(), .integer(2)),
            AssistantJSONMember("felder", .array([.object([AssistantJSONMember("name", .string(secret))])])),
        ]).encoded()
        guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: [AssistantOutcomeTurn(role: .user, text: "Hallo")], raw: raw, today: today) else {
            XCTFail("kein Ergebnis")
            return
        }
        let dump = evaluation.hints.map { $0.path + $0.code.rawValue }.joined().lowercased()
        XCTAssertFalse(dump.contains("geheim"))
    }

    func testDeadlinesInSentences() {
        XCTAssertEqual(AssistantOutcomeDates.deadlines(in: "Machen wir am 14.11. oder übermorgen.", today: today), ["2026-11-14", "2026-11-15"])
    }

    func testWeekdayAndCalendarArithmetic() {
        XCTAssertEqual(today.weekday, 5)
        XCTAssertEqual(AssistantOutcomeDate(dayNumber: AssistantOutcomeDate(year: 2028, month: 2, day: 29).dayNumber).formatted, "2028-02-29")
        XCTAssertEqual(AssistantOutcomeDate(year: 1970, month: 1, day: 1).dayNumber, 0)
        XCTAssertEqual(AssistantOutcomeDate(year: 2027, month: 1, day: 4).weekday, 1)
    }

    // MARK: Grenzen und große Eingaben

    private func fieldProfile() -> AssistantProfile {
        simpleProfile(
            fields: .array([.object([AssistantJSONMember("name", .string("projekt")), AssistantJSONMember("typ", .string("text"))])]),
            summary: true, tasks: true
        )
    }

    private func checkLengths(_ outcome: AssistantOutcome, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertLessThanOrEqual(outcome.summary.unicodeScalars.count, 1200, file: file, line: line)
        XCTAssertLessThanOrEqual(outcome.tasks.count, 20, file: file, line: line)
        for t in outcome.tasks {
            XCTAssertLessThanOrEqual(t.text.unicodeScalars.count, 300, file: file, line: line)
            XCTAssertLessThanOrEqual(t.quote.unicodeScalars.count, 500, file: file, line: line)
        }
        for f in outcome.fields {
            XCTAssertLessThanOrEqual(f.value?.unicodeScalars.count ?? 0, 500, file: file, line: line)
            XCTAssertLessThanOrEqual(f.quote?.unicodeScalars.count ?? 0, 500, file: file, line: line)
        }
        for q in outcome.openQuestions { XCTAssertLessThanOrEqual(q.unicodeScalars.count, 300, file: file, line: line) }
    }

    /// Wanduhrzeit eines Blocks. Die Grenzen in den Tests sind bewusst weit (30 bzw. 60 s): Sie fangen nur quadratisches Verhalten ab
    /// (das Minuten bräuchte) und hängen nicht von der Last des Prüfrechners ab.
    private func seconds(_ block: () -> Void) -> Double {
        let start = Date()
        block()
        return Date().timeIntervalSince(start)
    }

    func testHugeSingleTurn() {
        let huge = String(repeating: "Es geht um das Projekt Alpha. ", count: 100_000)
        let turns = [AssistantOutcomeTurn(role: .user, text: huge), AssistantOutcomeTurn(role: .assistant, text: "Verstanden.")]
        let profile = fieldProfile()
        let whole = AssistantOutcome.prompt(profile: profile, turns: turns, today: today, budgetChars: 20_000)
        XCTAssertFalse(whole.truncated)
        let middle = [
            AssistantOutcomeTurn(role: .user, text: "a"), AssistantOutcomeTurn(role: .assistant, text: "b"),
            AssistantOutcomeTurn(role: .user, text: huge), AssistantOutcomeTurn(role: .assistant, text: "c"),
        ]
        let p = AssistantOutcome.prompt(profile: profile, turns: middle, today: today, budgetChars: 20_000)
        XCTAssertTrue(p.truncated)
        XCTAssertLessThanOrEqual(p.text.unicodeScalars.count, 20_000)
        let raw = AssistantJSON.object([
            AssistantJSONMember("zusammenfassung", .string(String(repeating: "x", count: 5000))),
            AssistantJSONMember("felder", .array([.object([
                AssistantJSONMember("name", .string("projekt")), AssistantJSONMember("status", .string("klar")),
                AssistantJSONMember("wert", .string("Alpha")), AssistantJSONMember("beleg", .string("projekt alpha")),
                AssistantJSONMember("beitrag", .integer(1)),
            ])])),
        ]).encoded()
        guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: turns, raw: raw, today: today) else {
            XCTFail("kein Ergebnis")
            return
        }
        checkLengths(evaluation.outcome)
        XCTAssertEqual(evaluation.outcome.fields[0].value, "Alpha")
        XCTAssertEqual(evaluation.outcome.fields[0].turn, 1)
        XCTAssertGreaterThan(AssistantOutcomeText.normalizedScalars(huge).count, 2_000_000)
        XCTAssertLessThanOrEqual(AssistantOutcomeDates.deadlines(in: String(repeating: "am 14.11. ", count: 500_000), today: today).count, 1024)
        XCTAssertNil(AssistantOutcomeDates.deadline(String(repeating: "morgen ", count: 500_000), today: today))
    }

    func testManyTurns() {
        let n = 100_000
        let turns = (0..<n).map { AssistantOutcomeTurn(role: $0 % 2 == 0 ? .user : .assistant, text: $0 == n - 1 ? "Projekt Omega" : "ja") }
        let profile = fieldProfile()
        let p = AssistantOutcome.prompt(profile: profile, turns: turns, today: today, budgetChars: 20_000)
        XCTAssertTrue(p.truncated)
        XCTAssertLessThanOrEqual(p.text.unicodeScalars.count, 20_000)
        XCTAssertEqual(AssistantOutcome.transcript(turns).count, n)
        let raw = AssistantJSON.object([AssistantJSONMember("felder", .array([.object([
            AssistantJSONMember("name", .string("projekt")), AssistantJSONMember("status", .string("klar")),
            AssistantJSONMember("wert", .string("Omega")), AssistantJSONMember("beleg", .string("Projekt Omega")),
            AssistantJSONMember("beitrag", .integer(n)),
        ])]))]).encoded()
        guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: turns, raw: raw, today: today) else {
            XCTFail("kein Ergebnis")
            return
        }
        checkLengths(evaluation.outcome)
        XCTAssertEqual(evaluation.outcome.fields[0].turn, n)
        XCTAssertEqual(evaluation.outcome.fields[0].value, "Omega")
    }

    func testVeryLongQuoteAndValueWithinRawLimit() {
        let long = String(repeating: "wort ", count: 5000)
        let turns = [AssistantOutcomeTurn(role: .user, text: "Anfang \(long) Ende")]
        let s = AssistantJSON.string(long)
        let raw = AssistantJSON.object([
            AssistantJSONMember("zusammenfassung", s),
            AssistantJSONMember("felder", .array([.object([
                AssistantJSONMember("name", .string("projekt")), AssistantJSONMember("status", .string("klar")),
                AssistantJSONMember("wert", s), AssistantJSONMember("beleg", s), AssistantJSONMember("beitrag", .integer(1)),
            ])])),
            AssistantJSONMember("aufgaben", .array([.object([
                AssistantJSONMember("text", s), AssistantJSONMember("frist_text", s), AssistantJSONMember("beleg", s),
                AssistantJSONMember("beitrag", .integer(1)),
            ])])),
            AssistantJSONMember("offene_fragen", .array([s])),
        ]).encoded()
        XCTAssertLessThan(raw.unicodeScalars.count, AssistantOutcome.maxRaw)
        guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: fieldProfile(), turns: turns, raw: raw, today: today) else {
            XCTFail("kein Ergebnis")
            return
        }
        checkLengths(evaluation.outcome)
        XCTAssertFalse(evaluation.outcome.encode().isEmpty)
    }

    func testRawLimitCountsScalarsAndEndsWithJson() {
        let ok = "{\"offene_fragen\":[\"Wer?\"]}" + String(repeating: " ", count: AssistantOutcome.maxRaw - 26)
        XCTAssertEqual(ok.unicodeScalars.count, AssistantOutcome.maxRaw)
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        guard case .success = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: ok, today: today) else {
            XCTFail("an der Grenze muss gelesen werden")
            return
        }
        guard case .failure(let failure) = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: ok + " ", today: today) else {
            XCTFail("darüber muss json kommen")
            return
        }
        XCTAssertEqual(failure.rawValue, "json")
        let t = seconds {
            let r = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: String(repeating: "{", count: 5_000_000), today: today)
            if case .success = r { XCTFail("json erwartet") }
        }
        XCTAssertLessThan(t, 30)
    }

    func testManyOpeningBracesAreNotQuadratic() {
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        let t = seconds {
            let r = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: String(repeating: "{", count: 200_000), today: today)
            guard case .failure(let f) = r else { XCTFail("json erwartet"); return }
            XCTAssertEqual(f.rawValue, "json")
        }
        XCTAssertLessThan(t, 30)
    }

    func testManyBalancedInvalidObjectsAreNotQuadratic() {
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        let t = seconds {
            let r = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: String(repeating: "{,}", count: 80_000), today: today)
            guard case .failure(let f) = r else { XCTFail("json erwartet"); return }
            XCTAssertEqual(f.rawValue, "json")
        }
        XCTAssertLessThan(t, 30)
        let t2 = seconds {
            let raw = String(repeating: "{,}", count: 80_000) + "{\"offene_fragen\":[\"y\"]}"
            guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: raw, today: today) else {
                XCTFail("das echte Objekt muss gelten")
                return
            }
            XCTAssertEqual(evaluation.outcome.openQuestions, ["y"])
        }
        XCTAssertLessThan(t2, 30)
    }

    func testNestedAndInterleavedShapesEndAsJson() {
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        let shapes = [
            String(repeating: "{\"a\":", count: 40_000), String(repeating: "{\"a\":[", count: 40_000), String(repeating: "{\"", count: 60_000),
            "{\"x\":\"" + String(repeating: "{", count: 100_000),
            String(repeating: "{ ", count: 16) + String(repeating: "{,}", count: 50_000),
        ]
        let t = seconds {
            for raw in shapes {
                guard case .failure(let f) = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: raw, today: today) else {
                    XCTFail("json erwartet")
                    continue
                }
                XCTAssertEqual(f.rawValue, "json")
            }
        }
        XCTAssertLessThan(t, 60)
    }

    func testStripPrefix() {
        typealias G = AssistantOutcomeGrounding
        XCTAssertTrue(G.stripPrefix("[3] Nutzer: Projekt Alpha").text == "Projekt Alpha" && G.stripPrefix("[3] Nutzer: Projekt Alpha").turn == 3)
        XCTAssertTrue(G.stripPrefix("Nutzer: Projekt Alpha").text == "Projekt Alpha" && G.stripPrefix("Nutzer: Projekt Alpha").turn == nil)
        XCTAssertTrue(G.stripPrefix("assistent:Projekt").text == "Projekt" && G.stripPrefix("assistent:Projekt").turn == nil)
        XCTAssertTrue(G.stripPrefix("[12] User: x").text == "x" && G.stripPrefix("[12] User: x").turn == 12)
        XCTAssertTrue(G.stripPrefix("Assistant : x").text == "x" && G.stripPrefix("Assistant : x").turn == nil)
        XCTAssertTrue(G.stripPrefix("[7] nur Nummer").text == "nur Nummer" && G.stripPrefix("[7] nur Nummer").turn == 7)
        for same in ["Projekt Alpha", "Chef: Projekt", "[x] Nutzer: a", "[] Nutzer: a", "[1234567] a", "Nutzerin: a"] {
            let r = G.stripPrefix(same)
            XCTAssertTrue(r.text.utf8.elementsEqual(same.utf8) && r.turn == nil, same)
        }
    }
}
