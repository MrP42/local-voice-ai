import Foundation
import XCTest
@testable import VoiceCore

// Auswertung am Gesprächsende (lva#559, Paket G5-Kern): Prompt, Lesen der Rohantwort mit Erdung, Fristauflösung, Kodierung. Die
// gemeinsamen Prüfstände (`apps/shared/assistenten/auswertung-pruefstaende.json`) laufen hier und in
// `scripts/auswertung.test.mjs` des PC mit identischem Ergebnis. Nur erfundene Gespräche.
//
// Zeichenketten werden hier nie über `String ==` oder `XCTAssertEqual` auf `String` verglichen (kanonische Gleichheit: `é` wäre
// gleich `e` + U+0301), sondern über `assertSameBytes` (UTF-8-Bytes, siehe AssistantProfileTests).

private func outcomeProfile(_ tree: AssistantJSON, file: StaticString = #filePath, line: UInt = #line) -> AssistantProfile {
    switch AssistantProfile.parse(tree: tree) {
    case .success(let p): return p
    case .failure(let f):
        XCTFail("Profil ungültig: \(f.issues)", file: file, line: line)
        return AssistantProfile(id: "x", name: "X")
    }
}

private func outcomeSame(_ a: String, _ b: String) -> Bool { a.utf8.elementsEqual(b.utf8) }

/// Teilfolge in Codepunkten (kein `String.contains`).
private func outcomeContains(_ haystack: String, _ needle: String) -> Bool {
    AssistantOutcomeText.contains(AssistantOutcomeText.scalars(haystack), AssistantOutcomeText.scalars(needle))
}

/// Zeilen eines Textes, getrennt an Byte 0x0A.
private func outcomeLines(_ s: String) -> [String] {
    s.utf8.split(separator: 0x0A, omittingEmptySubsequences: false).map { String(decoding: $0, as: UTF8.self) }
}

private func outcomeHasPrefix(_ s: String, _ prefix: String) -> Bool { s.utf8.starts(with: prefix.utf8) }

private func outcomeTurns(_ tree: AssistantJSON?) -> [AssistantOutcomeTurn] {
    (tree?.items ?? []).map { item in
        AssistantOutcomeTurn(role: outcomeSame(item["rolle"]?.string ?? "", "assistent") ? .assistant : .user, text: item["text"]?.string ?? "")
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

/// Ganzzahliger 32-Bit-Zufallserzeuger (Linearer Kongruenzgenerator, Zahlen nach Numerical Recipes): `&*` und `&+` rechnen modulo
/// 2^32. Der Node-Test rechnet mit `Math.imul` und `>>> 0` dieselbe Folge.
private struct OutcomeRandom {
    var state: UInt32

    init(seed: UInt32) { state = seed }

    mutating func next(_ n: Int) -> Int {
        state = state &* 1_664_525 &+ 1_013_904_223
        return Int((state >> 16) % UInt32(n))
    }
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
        XCTAssertGreaterThanOrEqual(shared["prompt"]?.items.count ?? 0, 10)
        XCTAssertGreaterThanOrEqual(shared["lesen_ok"]?.items.count ?? 0, 40)
        XCTAssertGreaterThanOrEqual(shared["fristen_in"]?.items.count ?? 0, 20)
        var codes = Set<AssistantExactKey>()
        for c in shared["auswerten"]?.items ?? [] { for h in c["erwartet"]?["hinweise"]?.items ?? [] { codes.insert(AssistantExactKey(h["code"]?.string ?? "")) } }
        for code in AssistantOutcomeHint.Code.allCases { XCTAssertTrue(codes.contains(AssistantExactKey(code.rawValue)), code.rawValue) }
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
                assertSameBytes(failure.rawValue, expected?["fehler"]?.string, name)
                continue
            }
            guard case .success(let evaluation) = result else { XCTFail("\(name): kein Ergebnis"); continue }
            let text = evaluation.outcome.encode()
            assertSameBytes(text, expected?["kodiert"]?.string, name)
            XCTAssertTrue(evaluation.hints == outcomeHints(expected?["hinweise"]), name)
            // Rundreise: erneut lesen und schreiben ergibt dieselben Bytes.
            guard case .success(let again) = AssistantOutcome.parse(json: text) else { XCTFail("\(name): Rundreise"); continue }
            assertSameBytes(again.encode(), text, name)
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
                XCTAssertEqual(task.due == nil, task.dueTurn == nil, name)
                if let dueTurn = task.dueTurn {
                    XCTAssertTrue(AssistantOutcomeText.contains(normalizedText(turns, dueTurn), AssistantOutcomeText.normalizedScalars(task.dueText)), name)
                }
            }
            // Kein Hinweis-Pfad trägt Inhalt: nur Namen des Formats und Indizes.
            for hint in evaluation.hints { XCTAssertTrue(isFormatPath(hint.path), "\(name): \(hint.path)") }
        }
    }

    /// Pfade bestehen nur aus Namen des Formats und Indizes.
    private func isFormatPath(_ path: String) -> Bool {
        let b = Array(path.utf8)
        if b.isEmpty { return true }
        if outcomeSame(path, "zusammenfassung") { return true }
        var rest = b[...]
        var matched = false
        for root in ["felder", "aufgaben", "offene_fragen"] where rest.starts(with: root.utf8) {
            rest = rest.dropFirst(root.utf8.count)
            matched = true
            break
        }
        guard matched else { return false }
        if rest.first == 0x5B {
            guard let close = rest.firstIndex(of: 0x5D), close > rest.startIndex + 1,
                  rest[(rest.startIndex + 1)..<close].allSatisfy({ (0x30...0x39).contains($0) }) else { return false }
            rest = rest[(close + 1)...]
        }
        if rest.isEmpty { return true }
        guard rest.first == 0x2E else { return false }
        let leaf = String(decoding: rest.dropFirst(), as: UTF8.self)
        return ["wert", "beleg", "beitrag", "status", "text", "frist_text"].contains { outcomeSame($0, leaf) }
    }

    func testSharedDeadlines() {
        for c in shared["frist"]?.items ?? [] {
            let expression = c["ausdruck"]?.string ?? ""
            let d = outcomeDate(c["heute"])
            assertSameBytes(AssistantOutcomeDates.deadline(expression, today: d), c["erwartet"]?.string, "\(expression) ab \(d)")
        }
    }

    func testSharedNormalization() {
        for c in shared["normalisierung"]?.items ?? [] {
            let input = c["eingabe"]?.string ?? ""
            let got = AssistantOutcomeText.normalize(input)
            XCTAssertTrue(got.utf8.elementsEqual((c["erwartet"]?.string ?? "").utf8), "\(input) -> \(got)")
        }
    }

    func testSharedReadableResults() {
        for c in shared["lesen_ok"]?.items ?? [] {
            let name = c["name"]?.string ?? "?"
            let text = c["kodiert"]?.string ?? ""
            guard case .success(let outcome) = AssistantOutcome.parse(json: text) else { XCTFail("\(name): lesbar erwartet"); continue }
            assertSameBytes(outcome.encode(), text, name)
        }
    }

    func testSharedDeadlinesInText() {
        for c in shared["fristen_in"]?.items ?? [] {
            let text = c["text"]?.string ?? ""
            let d = outcomeDate(c["heute"])
            let expected = c["erwartet"]
            assertSameBytes(AssistantOutcomeDates.deadlines(in: text, today: d), expected?["tage"]?.strings ?? [], text)
            let scalars = AssistantOutcomeText.normalizedScalars(text)
            assertSameBytes(AssistantOutcomeDates.contextDeadline(scalars, start: 0, end: scalars.count, today: d), expected?["deckung"]?.string, text)
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
            let lines = outcomeLines(p.text)
            guard let i = lines.firstIndex(where: { outcomeHasPrefix($0, "[1] ") }) else { XCTFail("kein Beitrag 1"); continue }
            XCTAssertTrue(outcomeHasPrefix(lines[i + 1], "[2] "))
            XCTAssertTrue(outcomeHasPrefix(lines[i + 2], "[… ") && outcomeContains(lines[i + 2], " Beiträge ausgelassen …]"))
            XCTAssertTrue(outcomeHasPrefix(lines[lines.count - 1], "[60] "))
            let kept = lines.count - (i + 3)
            let omitted = Int(lines[i + 2].dropFirst(3).prefix { $0.isNumber }) ?? -1
            XCTAssertEqual(omitted, 60 - 2 - kept)
            let more = AssistantOutcome.prompt(profile: profile, turns: turns, today: today, budgetChars: budget + 80)
            XCTAssertGreaterThanOrEqual(outcomeLines(more.text).count, lines.count)
        }
    }

    func testTranscriptNumbersAndCollapsesWhitespace() {
        assertSameBytes(
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
        var generator = OutcomeRandom(seed: 7)
        func random(_ n: Int) -> Int { generator.next(n) }
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
        XCTAssertFalse(outcomeContains(dump, "geheim"))
    }

    func testDeadlinesInSentences() {
        assertSameBytes(AssistantOutcomeDates.deadlines(in: "Machen wir am 14.11., dann melde ich mich übermorgen.", today: today), ["2026-11-14", "2026-11-15"])
    }

    func testWeekdayAndCalendarArithmetic() {
        XCTAssertEqual(today.weekday, 5)
        assertSameBytes(AssistantOutcomeDate(dayNumber: AssistantOutcomeDate(year: 2028, month: 2, day: 29).dayNumber).formatted, "2028-02-29")
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
        assertSameBytes(evaluation.outcome.fields[0].value, "Alpha")
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
        assertSameBytes(evaluation.outcome.fields[0].value, "Omega")
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
        assertSameBytes(failure.rawValue, "json")
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
            assertSameBytes(f.rawValue, "json")
        }
        XCTAssertLessThan(t, 30)
    }

    func testManyBalancedInvalidObjectsAreNotQuadratic() {
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        let t = seconds {
            let r = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: String(repeating: "{,}", count: 80_000), today: today)
            guard case .failure(let f) = r else { XCTFail("json erwartet"); return }
            assertSameBytes(f.rawValue, "json")
        }
        XCTAssertLessThan(t, 30)
        let t2 = seconds {
            let raw = String(repeating: "{,}", count: 80_000) + "{\"offene_fragen\":[\"y\"]}"
            guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: raw, today: today) else {
                XCTFail("das echte Objekt muss gelten")
                return
            }
            assertSameBytes(evaluation.outcome.openQuestions, ["y"])
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
                assertSameBytes(f.rawValue, "json")
            }
        }
        XCTAssertLessThan(t, 60)
    }

    func testStripPrefix() {
        typealias G = AssistantOutcomeGrounding
        for (input, text, turn) in [
            ("[3] Nutzer: Projekt Alpha", "Projekt Alpha", 3 as Int?), ("Nutzer: Projekt Alpha", "Projekt Alpha", nil),
            ("assistent:Projekt", "Projekt", nil), ("[12] User: x", "x", 12), ("Assistant : x", "x", nil), ("[7] nur Nummer", "nur Nummer", 7),
        ] {
            let r = G.stripPrefix(input)
            assertSameBytes(r.text, text, input)
            XCTAssertEqual(r.turn, turn, input)
        }
        for same in ["Projekt Alpha", "Chef: Projekt", "[x] Nutzer: a", "[] Nutzer: a", "[1234567] a", "Nutzerin: a"] {
            let r = G.stripPrefix(same)
            XCTAssertTrue(r.text.utf8.elementsEqual(same.utf8) && r.turn == nil, same)
        }
    }

    // MARK: Invarianten (Blocker: jedes Ergebnisfeld hält seine Längen- und Formatregel)

    private func assertRoundTrip(_ outcome: AssistantOutcome, _ what: String, file: StaticString = #filePath, line: UInt = #line) {
        let text = outcome.encode()
        guard case .success(let again) = AssistantOutcome.parse(json: text) else {
            XCTFail("\(what): nicht lesbar", file: file, line: line)
            return
        }
        assertSameBytes(again.encode(), text, what, file: file, line: line)
        XCTAssertTrue(again == outcome, what, file: file, line: line)
    }

    /// Jede Beitragsnummer des Ergebnisses liegt im Gespräch (1 bis Anzahl der Beiträge).
    private func assertTurnNumbers(_ outcome: AssistantOutcome, _ count: Int, _ what: String, file: StaticString = #filePath, line: UInt = #line) {
        for f in outcome.fields {
            if let turn = f.turn { XCTAssertTrue(turn >= 1 && turn <= count, "\(what): Feld", file: file, line: line) }
        }
        for t in outcome.tasks {
            XCTAssertTrue(t.turn >= 1 && t.turn <= count, "\(what): Aufgabe", file: file, line: line)
            if let due = t.dueTurn { XCTAssertTrue(due >= 1 && due <= count, "\(what): Frist", file: file, line: line) }
        }
    }

    func testRandomGeneratorGivesTheSameSequenceAsTheNodeTest() {
        func sequence(_ seed: UInt32) -> [Int] {
            var g = OutcomeRandom(seed: seed)
            return (0..<12).map { _ in g.next(1000) }
        }
        XCTAssertEqual(sequence(7), [648, 866, 140, 750, 233, 84, 657, 480, 507, 483, 753, 816])
        XCTAssertEqual(sequence(11), [750, 108, 24, 918, 871, 484, 579, 36, 792, 7, 623, 727])
    }

    func testInvariantRoundTripForAllSharedCases() {
        var counted = 0
        for c in shared["auswerten"]?.items ?? [] {
            let result = AssistantOutcome.evaluate(
                profile: outcomeProfile(c["profil"] ?? .null), turns: outcomeTurns(c["verlauf"]), raw: outcomeRaw(c),
                today: outcomeDate(c["heute"]), created: c["erstellt"]?.string ?? "", truncated: c["gekuerzt"]?.bool ?? false
            )
            guard case .success(let evaluation) = result else { continue }
            assertRoundTrip(evaluation.outcome, c["name"]?.string ?? "?")
            assertTurnNumbers(evaluation.outcome, outcomeTurns(c["verlauf"]).count, c["name"]?.string ?? "?")
            counted += 1
        }
        XCTAssertGreaterThanOrEqual(counted, 60)
    }

    private func profileField(_ name: String, _ kind: String, options: [String] = []) -> AssistantJSON {
        .object([
            AssistantJSONMember("name", .string(name)), AssistantJSONMember("typ", .string(kind)),
            AssistantJSONMember("optionen", .array(options.map { .string($0) })),
        ])
    }

    func testInvariantRoundTripWithExtremeValues() {
        let profile = simpleProfile(
            fields: .array([
                profileField("a", "text"), profileField("b", "zahl"), profileField("c", "datum"),
                profileField("d", "auswahl", options: ["ja", "nein"]), profileField("e", "zahl"),
            ]),
            summary: true, tasks: true
        )
        let turns = [
            AssistantOutcomeTurn(role: .user, text: "Es geht um 5 Dinge, 0001 Stück, ja gerne."),
            AssistantOutcomeTurn(role: .assistant, text: "Gut, übernächsten Montag oder Montag in zwei Wochen?"),
            AssistantOutcomeTurn(role: .user, text: "Hallo, morgen ist das Jahr 2027 und eins."),
        ]
        func long(_ s: String, _ n: Int) -> String { String(repeating: s, count: n) }
        let values = [
            "5", "0001", "-0", "00.50", "1e5", "NaN", "eins", "ja", "Jahr", "morgen", "2026-11-14", "2026-02-30",
            "übernächsten Montag", "Montag in zwei Wochen", "12,5", "1.250,50", "", "  ",
            long("0", 500) + "1", long("0", 501) + "1", long("9", 600), "-" + long("0", 499) + "5", long("1", 501),
            long("ä", 501), "x" + long(" ", 600) + "y", long("😀", 520), "-" + long("0", 700),
        ]
        let quotes = [
            "Es geht um 5 Dinge", "0001 Stück", "ja gerne", "übernächsten Montag", "Montag in zwei Wochen", "Hallo", "eins",
            "morgen ist das Jahr 2027", "erfunden", "", "[3] Nutzer: Hallo", "Assistent: Gut", long("Hallo ", 200),
        ]
        let dueTexts = ["Montag", "morgen", "übernächsten Montag", "heute", "", long("x", 150), "in zwei Wochen", "Jahr"]
        let longTexts = ["t", long("Satz eins. ", 100), long("ä", 400), long("😀", 1300), "  ", ""]
        let created = ["", "heute", "2026-02-30T10:00:00Z", "2026-11-13T10:00:00Z", "2026-11-13T25:00:00Z"]
        var generator = OutcomeRandom(seed: 11)
        func random(_ n: Int) -> Int { generator.next(n) }
        func pick<T>(_ list: [T]) -> T { list[random(list.count)] }
        func object(_ members: [(String, AssistantJSON)]) -> AssistantJSON { .object(members.map { AssistantJSONMember($0.0, $0.1) }) }
        let statuses: [String?] = ["klar", "unklar", "nicht_genannt", nil, "?"]
        let turnValues: [AssistantJSON] = [.integer(1), .integer(2), .integer(3), .integer(4), .integer(0), .null, .string("2")]
        for run in 0..<600 {
            let fields = ["a", "b", "c", "d", "e"].map { name -> AssistantJSON in
                let status = pick(statuses)
                var members: [(String, AssistantJSON)] = [("name", .string(name)), ("wert", .string(pick(values))), ("beleg", .string(pick(quotes))), ("beitrag", pick(turnValues))]
                if let status { members.append(("status", .string(status))) }
                return object(members)
            }
            let tasks = (0..<random(4)).map { _ in
                object([("text", .string(pick(longTexts))), ("frist_text", .string(pick(dueTexts))), ("beleg", .string(pick(quotes))), ("beitrag", .integer(pick([1, 2, 3, 9])))])
            }
            let raw = object([
                ("felder", .array(fields)), ("aufgaben", .array(tasks)), ("zusammenfassung", .string(pick(longTexts))),
                ("offene_fragen", .array([.string(pick(longTexts)), .string(pick(longTexts))])),
            ]).encoded()
            guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: turns, raw: raw, today: today, created: pick(created), truncated: random(2) == 0) else {
                XCTFail("kein Ergebnis")
                return
            }
            checkLengths(evaluation.outcome)
            assertRoundTrip(evaluation.outcome, "Lauf \(run)")
            assertTurnNumbers(evaluation.outcome, turns.count, "Lauf \(run)")
            for hint in evaluation.hints { XCTAssertTrue(isFormatPath(hint.path), hint.path) }
        }
    }

    func testFiveHundredZerosAndOneIsTheCanonicalNumberOne() {
        let profile = simpleProfile(fields: .array([profileField("n", "zahl")]))
        let raw = AssistantJSON.object([AssistantJSONMember("felder", .array([.object([
            AssistantJSONMember("name", .string("n")), AssistantJSONMember("status", .string("klar")),
            AssistantJSONMember("wert", .string(String(repeating: "0", count: 500) + "1")), AssistantJSONMember("beleg", .string("eins")),
            AssistantJSONMember("beitrag", .integer(1)),
        ])]))]).encoded()
        guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: [AssistantOutcomeTurn(role: .user, text: "eins")], raw: raw, today: today) else {
            XCTFail("kein Ergebnis")
            return
        }
        assertSameBytes(evaluation.outcome.fields[0].value, "1")
        XCTAssertEqual(evaluation.outcome.fields[0].status, .clear)
        assertRoundTrip(evaluation.outcome, "500 Nullen")
    }

    func testCanonicalNumbers() {
        typealias T = AssistantOutcomeText
        for (input, canonical) in [("0001", "1"), ("000", "0"), ("00.50", "0.50"), ("-0", "0"), ("-0.0", "0.0"), ("-0.50", "-0.50"), ("1.250,50", "1250.50"), ("001.250,5", "1250.5"), ("zwei", "2")] {
            assertSameBytes(T.readNumber(input)?.canonical, canonical, input)
        }
        for ok in ["0", "-1", "12.50", "0.50", "0.0", "1250.5", "-0.50"] { XCTAssertTrue(T.isCanonicalNumber(ok), ok) }
        for bad in ["01", "-0", "-0.00", "1.", ".5", "+1", "", "NaN", "1e5", "1,5", "--1", " 1", "00.5"] { XCTAssertFalse(T.isCanonicalNumber(bad), bad) }
    }

    func testHintPathsNeverCarryAnUnknownKey() {
        let profile = outcomeProfile(.object([
            AssistantJSONMember("schema", .string("lva.assistent/1")), AssistantJSONMember("id", .string("eigen-aaaaaaaaaaaa")),
            AssistantJSONMember("name", .string("T")),
            AssistantJSONMember("auswertung", .object([
                AssistantJSONMember("aufgaben", .bool(true)),
                AssistantJSONMember("felder", .array([profileField("iban_de00", "text")])),
            ])),
        ]))
        let secrets = [("token_abc123", AssistantJSON.integer(1)), ("passwort", .integer(2)), ("iban_de00", .integer(3)), ("Zusatz", .integer(4))]
        func withSecrets(_ members: [(String, AssistantJSON)]) -> AssistantJSON { .object((members + secrets).map { AssistantJSONMember($0.0, $0.1) }) }
        let raw = withSecrets([
            ("felder", .array([
                withSecrets([("name", .string("iban_de00")), ("status", .string("klar")), ("wert", .string("A")), ("beleg", .string("A")), ("beitrag", .integer(1))]),
                .object([AssistantJSONMember("name", .string("token_abc123"))]),
            ])),
            ("aufgaben", .array([withSecrets([("text", .string("t")), ("beleg", .string("A")), ("beitrag", .integer(1))])])),
        ]).encoded()
        guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: profile, turns: [AssistantOutcomeTurn(role: .user, text: "A")], raw: raw, today: today) else {
            XCTFail("kein Ergebnis")
            return
        }
        let dump = evaluation.hints.map { $0.path + "|" + $0.code.rawValue }.joined(separator: ",")
        for word in ["token_abc123", "passwort", "iban_de00", "Zusatz"] { XCTAssertFalse(outcomeContains(dump, word), word) }
        let want = [("", "unbekannt"), ("aufgaben[0]", "unbekannt"), ("felder[0]", "unbekannt"), ("felder[1]", "unbekannt")]
        XCTAssertEqual(evaluation.hints.count, want.count)
        for (hint, expected) in zip(evaluation.hints, want) {
            assertSameBytes(hint.path, expected.0)
            assertSameBytes(hint.code.rawValue, expected.1)
        }
    }

    func testPromptHasNoLineSeparatorsAndQuotesInOptionsAreSoftened() {
        let profile = outcomeProfile(.object([
            AssistantJSONMember("schema", .string("lva.assistent/1")), AssistantJSONMember("id", .string("eigen-aaaaaaaaaaaa")),
            AssistantJSONMember("name", .string("T")),
            AssistantJSONMember("auswertung", .object([AssistantJSONMember("felder", .array([.object([
                AssistantJSONMember("name", .string("a")), AssistantJSONMember("beschreibung", .string("x\u{2028}y\u{85}z\u{2029}w\rv")),
                AssistantJSONMember("typ", .string("auswahl")), AssistantJSONMember("optionen", .array([.string("a\"b"), .string("c\u{2028}d")])),
            ])]))])),
        ]))
        let p = AssistantOutcome.prompt(
            profile: profile, turns: [AssistantOutcomeTurn(role: .user, text: "1\u{2028}2\u{85}3\u{2029}4\r5\n6")], today: today, budgetChars: 100_000
        )
        let lines = outcomeLines(p.text)
        assertSameBytes(lines[lines.count - 1], "[1] Nutzer: 1 2 3 4 5 6")
        for c in [0x2028, 0x85, 0x2029, 0x0D] as [UInt32] { XCTAssertFalse(p.text.unicodeScalars.contains { $0.value == c }, "U+\(String(c, radix: 16))") }
        XCTAssertTrue(outcomeContains(p.text, "\"a'b\", \"c d\""))
    }

    func testCreatedIsOnlyKeptWhenItIsATimestamp() {
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        for (input, expected) in [
            ("2026-11-13T10:00:00Z", "2026-11-13T10:00:00Z"), ("2026-11-13T10:00:00", ""), ("2026-02-30T10:00:00Z", ""),
            ("2026-11-13T24:00:00Z", ""), ("heute", ""), ("", ""),
        ] {
            guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: "{\"offene_fragen\":[]}", today: today, created: input) else {
                XCTFail("kein Ergebnis")
                return
            }
            assertSameBytes(evaluation.outcome.created, expected, input)
            assertRoundTrip(evaluation.outcome, input)
        }
    }

    // MARK: Byte-genauer Vergleich

    func testComposedAndDecomposedTextAreDifferent() {
        let composed = "\u{E9}"
        let decomposed = "e\u{301}"
        XCTAssertTrue(composed == decomposed, "Swift setzt kanonische Gleichheit an; darum prüfen die Tests über Bytes")
        XCTAssertFalse(outcomeSame(composed, decomposed))
        func outcome(_ quote: String) -> AssistantOutcome {
            AssistantOutcome(
                assistantId: "eigen-aaaaaaaaaaaa",
                fields: [AssistantOutcome.Field(name: "f", kind: .text, status: .unclear, value: quote, quote: quote, turn: 1)],
                tasks: [AssistantOutcome.Task(text: quote, dueText: quote, quote: quote, turn: 1)],
                openQuestions: [quote]
            )
        }
        let a = outcome(composed)
        let b = outcome(decomposed)
        XCTAssertTrue(a == a)
        XCTAssertFalse(a == b)
        XCTAssertFalse(a.fields[0] == b.fields[0])
        XCTAssertFalse(a.tasks[0] == b.tasks[0])
        XCTAssertFalse(outcomeSame(a.encode(), b.encode()))
        assertRoundTrip(a, "é")
        assertRoundTrip(b, "e + U+0301")
        XCTAssertFalse(AssistantOutcomeHint(path: "a\u{E9}", code: .type) == AssistantOutcomeHint(path: "ae\u{301}", code: .type))
        XCTAssertFalse(AssistantOutcomeTurn(role: .user, text: composed) == AssistantOutcomeTurn(role: .user, text: decomposed))
        let pa = AssistantOutcome.prompt(profile: simpleProfile(), turns: [AssistantOutcomeTurn(role: .user, text: composed)], today: today, budgetChars: 100_000)
        let pb = AssistantOutcome.prompt(profile: simpleProfile(), turns: [AssistantOutcomeTurn(role: .user, text: decomposed)], today: today, budgetChars: 100_000)
        XCTAssertTrue(pa == pa)
        XCTAssertFalse(pa == pb, "der Prompt vergleicht byteweise")
    }

    func testOccurrencesAtWordBoundaries() {
        typealias T = AssistantOutcomeText
        func spots(_ haystack: String, _ needle: String, _ limit: Int) -> [Int] {
            T.occurrences(of: T.scalars(needle), in: T.scalars(haystack), limit: limit)
        }
        XCTAssertEqual(spots("ja jahr ja", "ja", 10), [0, 8])
        XCTAssertEqual(spots("jahr", "ja", 10), [])
        XCTAssertEqual(spots("a a a", "a", 2), [0, 2])
        XCTAssertEqual(spots("12,5 liter", "12", 5), [0])
        XCTAssertEqual(spots("x+5 und +5", "+5", 5), [1, 8])
        XCTAssertEqual(spots("a😀b 😀", "😀", 5), [4])
        XCTAssertEqual(spots("abc", "", 5), [])
    }

    func testManyHintsAreDeduplicatedWithoutQuadraticWork() {
        let turns = [AssistantOutcomeTurn(role: .user, text: "Hallo")]
        let entries = (0..<3000).map { _ in AssistantJSON.integer(1) }
        let raw = AssistantJSON.object([AssistantJSONMember("offene_fragen", .array(entries))]).encoded()
        let t = seconds {
            guard case .success(let evaluation) = AssistantOutcome.evaluate(profile: simpleProfile(), turns: turns, raw: raw, today: today) else {
                XCTFail("kein Ergebnis")
                return
            }
            XCTAssertEqual(evaluation.hints.count, 3000)
        }
        XCTAssertLessThan(t, 30)
    }
}
