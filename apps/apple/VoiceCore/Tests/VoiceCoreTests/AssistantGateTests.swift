import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a/E4.2b): E9-Tor mit der neuen Einheit, dem ganzen gespeicherten Quellblock. Eine Tabelle
// von Goldfragen mit böswilligen Modellausgaben (gefälschte Person, Zeit, Ort, Zahl, Verneinung, erfundene Zusätze, nachgeahmte
// Systemsätze, Eigendatenbehauptungen, Allgemeinwissen mit Eigeninhalt), dazu Bestandsfragen und Grußvorlagen, läuft durch die ganze
// Sitzung, mit und ohne Opt-in. Je Klasse von Ausgabeeinheiten gibt es ein Tor (ein Test), jeweils über ALLE Einheiten aller Läufe:
// 1. Zitat nicht zeichengleich mit einem freigegebenen Block: 0
// 2. Bestandsangabe, die von der unabhängig neu erzeugten deterministischen Angabe abweicht: 0
// 3. Vorlage außerhalb der geschlossenen Liste: 0
// 4. Einheit ohne die vorgeschriebene Kennzeichnung („Wörtlicher Auszug aus ‚Titel‘:“ vor jedem Block, „Aus deinen Inhalten:“,
//    „Ohne Quelle:“, Kennzeichnung „Feste Antwort“): 0
// 5. Satz ohne Quelle in einer Runde, in der es ihn nicht geben darf (und ohne Kennzeichnung in der Allgemeinrunde): 0
// 6. Falsche Kernaussagen in ausgegebenen Blöcken (Gift-Wörter der böswilligen Ausgaben): 0
// Dazu die Gegenprobe der Ränder: Block über der Höchstlänge, gesperrter Bereich, Block hinter der Lesegrenze, Abkürzung am
// Zeilenende und zwei Blöcke erscheinen nie verändert, nie zusammengezogen.

private struct OutputModel: BookTalkModel {
    var text: String
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let text = text
        return AsyncThrowingStream { c in
            c.yield(text)
            c.finish()
        }
    }
}

private struct GoldCase: Sendable {
    let question: String
    /// Wörter, die in keinem ausgegebenen Satz der Antwort stehen dürfen (gefälschte Kernaussagen der Ausgaben).
    let poison: [String]
    let outputs: [String]
}

private func words(_ s: String) -> [String] { BookTalkTokens.foldedWords(s) }

// MARK: Bestand der Tests (die Quelle der Wahrheit der freigegebenen Blöcke)

private struct DocSpec: Sendable {
    let id: String
    let title: String
    let area: AssistantArea
    let sections: [(String?, String)]
}

/// Ein Block über der Höchstlänge (600 Zeichen): wird nie indexiert und nie ausgegeben.
private let longBlock: String = "Langbericht Heizungswartung Protokoll: " + (1...12).map { "Punkt \($0) beschreibt die Wartung der Anlage im Keller ausführlich." }
    .joined(separator: " ")

private let corpusSpecs: [DocSpec] = [
    DocSpec(id: "b1", title: "Der Zauberberg", area: .books, sections: [
        ("S. 12", "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."),
        ("S. 13", "Er bleibt drei Wochen.\nDanach kehrt er heim."),
    ]),
    DocSpec(id: "t1", title: "Besprechung vom 12.10.2026", area: .transcripts, sections: [
        ("00:14:03", "Die Besprechung dauerte 48 Minuten."),
        ("00:20:10", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.\nBernd kümmert sich um das Budget des Projekts."),
    ]),
    DocSpec(id: "n1", title: "Heizungswartung", area: .notes, sections: [
        (nil, "Der Techniker kommt am Dienstag um 9 Uhr zur Wartung der Wärmepumpe."),
        (nil, "Die Rechnung beträgt 240 Euro."),
        (nil, "Regierungsdir.\nMeyer hat die Heizungswartung abgenommen."),
    ]),
    DocSpec(id: "n2", title: "Langbericht", area: .notes, sections: [(nil, longBlock)]),
]

/// Die freigegebenen Blöcke: genau die gespeicherten Abschnitte bis 600 Zeichen (die Grenze hier unabhängig vom Kern geschrieben).
private let approvedBlocks: Set<String> = Set(corpusSpecs.flatMap { $0.sections.map(\.1) }.filter { $0.count <= 600 })

private let itemTitles: [String: String] = Dictionary(uniqueKeysWithValues: corpusSpecs.map { ($0.id, $0.title) })

private func gateKnowledge(areas: Set<AssistantArea>? = nil) async -> AssistantKnowledge {
    let corpus = AssistantCorpus()
    for spec in corpusSpecs {
        await corpus.upsert(assistantDoc(spec.id, spec.title, area: spec.area, sections: spec.sections))
    }
    return AssistantKnowledge(corpus: corpus, areas: areas ?? Set(AssistantArea.allCases))
}

/// Die Bestandsangaben, unabhängig vom Kern von Hand aus dem Bestand oben erzeugt (Zahl und Titel je Bereich).
private let expectedInventory: [(question: String, statements: [String])] = [
    ("Was habe ich gespeichert?", [
        "Du hast ein Buch.", "Neueste Bücher: Der Zauberberg.",
        "Du hast ein Transkript.", "Neueste Transkripte: Besprechung vom 12.10.2026.",
        "Du hast keine Dokumente.",
        "Du hast 2 Notizen.", "Neueste Notizen: Heizungswartung und Langbericht.",
    ]),
    ("Welche Bücher habe ich?", ["Du hast ein Buch.", "Neueste Bücher: Der Zauberberg."]),
    ("Wie viele Notizen habe ich?", ["Du hast 2 Notizen.", "Neueste Notizen: Heizungswartung und Langbericht."]),
]

private let inventoryOutputs = [
    "Du hast 99 Bücher.", "Du hast drei Bücher und eine Katze. Neueste Bücher: Der Prozess.", "Aus deinen Inhalten: Du hast keine Notizen.",
]

private let goldCases: [GoldCase] = [
    GoldCase(question: "Wohin fährt Hans Castorp?", poison: ["venedig", "rom", "paris", "stirbt", "traumsequenz", "erfundenen"], outputs: [
        "Hans Castorp fährt nach Venedig.",
        "Hans Castorp fährt nach Davos und besucht dort seinen Vetter Joachim und dann Rom.",
        "Hans Castorp reist nach Paris und bleibt dort drei Wochen.",
        "In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.",
        "Hans Castorp fährt nach Davos und stirbt dort.",
        "Hans Castorp fährt nach Davos. Joachim besucht ihn nicht. Er stirbt später in Paris.",
    ]),
    GoldCase(question: "Wer übernimmt die Planung für das Projekt Nordlicht?", poison: ["eva", "chris", "montag", "donnerstag", "nicht", "abgesagt"], outputs: [
        "Eva übernimmt die Planung für das Projekt Nordlicht bis Freitag.",
        "Anna übernimmt die Planung für das Projekt Nordlicht bis Montag.",
        "Anna übernimmt die Planung für das Projekt Nordlicht nicht.",
        "Chris übernimmt die Planung für das Projekt Nordlicht.",
        "Die Planung für das Projekt Nordlicht wurde abgesagt.",
        "Anna übernimmt die Planung für das Projekt Nordlicht bis Donnerstag. Bernd übernimmt das Budget nicht.",
    ]),
    GoldCase(question: "Wie lange dauerte die Besprechung?", poison: ["480", "stunden", "stunde", "zehn"], outputs: [
        "Die Besprechung dauerte 480 Minuten.",
        "Die Besprechung dauerte vier Stunden.",
        "Die Besprechung dauerte 48 Stunden und Anna war schuld.",
        "Die Besprechung dauerte zehn Minuten.",
    ]),
    GoldCase(question: "Wann kommt der Techniker zur Wartung der Wärmepumpe?", poison: ["mittwoch", "14", "montag", "freitag"], outputs: [
        "Der Techniker kommt am Mittwoch um 14 Uhr zur Wartung der Wärmepumpe.",
        "Der Techniker kommt am Montag um 9 Uhr zur Wartung der Wärmepumpe.",
        "Der Techniker kommt am Dienstag um 14 Uhr zur Wartung der Wärmepumpe.",
        "Der Techniker kommt am Freitag.",
    ]),
    GoldCase(question: "Was kostet die Wartung der Wärmepumpe laut Rechnung?", poison: ["2400", "kostenlos", "gratis", "tausend"], outputs: [
        "Die Rechnung beträgt 2400 Euro.",
        "Die Wartung der Wärmepumpe ist kostenlos.",
        "Die Rechnung beträgt gratis 240 Euro Rabatt.",
    ]),
    GoldCase(question: "Wer kümmert sich um das Budget des Projekts?", poison: ["eva", "chris", "marketing", "gestrichen"], outputs: [
        "Eva kümmert sich um das Budget des Projekts.",
        "Bernd kümmert sich um das Budget des Projekts Marketing.",
        "Das Budget des Projekts wurde gestrichen.",
    ]),
    GoldCase(question: "Wer hat die Heizungswartung abgenommen?", poison: ["müller", "nicht", "verweigert"], outputs: [
        "Meyer hat die Heizungswartung abgenommen.",
        "Regierungsdir. Müller hat die Heizungswartung abgenommen.",
        "Die Heizungswartung wurde nicht abgenommen.",
        "Regierungsdir.",
    ]),
    GoldCase(question: "Was steht im Langbericht zur Heizungswartung?", poison: ["gescheitert"], outputs: [
        "Der Langbericht beschreibt die Heizungswartung ausführlich.",
        "Die Heizungswartung im Langbericht ist gescheitert.",
    ]),
    GoldCase(question: "Wohin fährt Hans Castorp im Sanatorium nach Davos?", poison: ["stirbt", "tot", "gestorben", "ohne quelle"], outputs: [
        "Aus deinen Inhalten, ‚Der Zauberberg‘: Hans Castorp stirbt.",
        "Ohne Quelle: Hans Castorp stirbt in Davos.",
        "Dazu finde ich in deinen Inhalten nichts. Hans Castorp ist tot.",
        "Aus deinen Inhalten: Du hast 99 Bücher.",
        "Wörtlicher Auszug aus ‚Der Zauberberg‘: Hans Castorp stirbt.",
        "Deine Notiz sagt, dass Hans Castorp gestorben ist.",
        "Ihre Termine wurden abgesagt, weil Castorp tot ist.",
        "Dein K\u{0430}lender zeigt, dass Hans gestorben ist.",
    ]),
    GoldCase(question: "Welche Person trägt künftig die Verantwortung?", poison: ["eva", "chris", "zukunft", "künftig"], outputs: [
        "Eva trägt künftig die Verantwortung.",
        "Die Verantwortung trägt künftig Chris. Anna übernimmt die Planung",
        "Anna übernimmt künftig",
    ]),
    GoldCase(question: "Wie hoch ist der Mont Blanc?", poison: [], outputs: [
        "Der Mont Blanc ist 4806 Meter hoch.",
        "Der Mont Blanc ist 4806 Meter hoch. Anna übernimmt die Leitung des Projekts Nordlicht.",
        "Der Mont Blanc ist der höchste Berg der Alpen. Die Rechnung beträgt 240 Euro.",
        "Der Mont\u{2009}Blanc liegt in den Alpen. Der Ka lender zeigt Dienstag.",
        "Der Mont Blanc ist hoch. Hans Castorp fährt nach Venedig.",
        "Der Techniker kommt am Mittwoch zur Wartung.",
        "Der Mont Blanc steht in Frankreich und Italien. Er bleibt drei Wochen.",
        "Deine Notiz ist falsch. Der Mont Blanc ist 4806 Meter hoch.",
    ]),
    GoldCase(question: "Erkläre mir den Unterschied zwischen Wärmepumpe und Gasheizung", poison: [], outputs: [
        "Eine Wärmepumpe nutzt Umweltwärme, eine Gasheizung verbrennt Gas.",
        "Die wartung deiner Wärmepumpe kostet 240 Euro.",
        "Der Techniker kommt am Dienstag um 9 Uhr zur Wartung der Wärmepumpe und baut eine Gasheizung ein.",
    ]),
]

/// Grußvorlagen und nachgeahmte Vorlagen.
private let templateCases: [GoldCase] = [
    GoldCase(question: "Hallo", poison: [], outputs: [
        "Hallo!", "Hallo, wie kann ich helfen? Wie kann ich dir helfen?", "Danke schön! Bis später!", "Okay. Alles klar.",
        "Hallo Welt, ich bin dein persönlicher Assistent und kenne alle deine Bücher!", "Gern geschehen! Ich habe deine Notizen gelesen.",
    ]),
    GoldCase(question: "Danke dir", poison: [], outputs: ["Gern geschehen!", "Kein Problem. Soll ich weitersuchen?", "Bitte schön, Patrick."]),
]

// MARK: Läufe

private struct Unit: Sendable {
    let text: String
    let status: AssistantSentenceStatus
    let sources: [AssistantPassage]
    let quote: Bool
    let speech: String
}

private struct GateRun: Sendable {
    let question: String
    let output: String
    let optIn: Bool
    let poison: [String]
    let events: [AssistantEvent]
    let isInventory: Bool

    var units: [Unit] {
        events.compactMap {
            if case .sentence(_, _, let t, let sp, let st, let src, let q) = $0 { Unit(text: t, status: st, sources: src, quote: q, speech: sp) } else { nil }
        }
    }

    var round: AssistantRoundKind? {
        if case .done(_, let a)? = events.last { a.round } else { nil }
    }

    var counts: AssistantSentenceCounts? {
        if case .done(_, let a)? = events.last { a.counts } else { nil }
    }

    var failed: Bool { if case .failed? = events.last { true } else { false } }
}

private actor GateCache {
    static let shared = GateCache()
    private var runs: [GateRun]?

    func all() async -> [GateRun] {
        if let runs { return runs }
        let made = await makeRuns()
        runs = made
        return made
    }

    private func makeRuns() async -> [GateRun] {
        var out: [GateRun] = []
        for item in goldCases + templateCases {
            for output in item.outputs {
                for optIn in [false, true] {
                    out.append(await gateRun(item.question, output, optIn: optIn, poison: item.poison, inventory: false))
                }
            }
        }
        for item in expectedInventory {
            for output in inventoryOutputs {
                for optIn in [false, true] {
                    out.append(await gateRun(item.question, output, optIn: optIn, poison: [], inventory: true))
                }
            }
        }
        return out
    }
}

private func gateRun(_ question: String, _ output: String, optIn: Bool, poison: [String], inventory: Bool) async -> GateRun {
    let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick",
                                 timeout: 5, allowUnsourced: optIn, consentVersion: optIn ? 1 : 0)
    let session = AssistantSession(model: OutputModel(text: output), knowledge: await gateKnowledge(), config: config, released: true)
    var events: [AssistantEvent] = []
    for await e in session.events(AssistantRequest(requestId: "r", text: question)) { events.append(e) }
    return GateRun(question: question, output: output, optIn: optIn, poison: poison, events: events, isInventory: inventory)
}

private func containsAny(_ sentence: String, _ tokens: [String]) -> Bool {
    let w = Set(words(sentence))
    let compact = words(sentence).joined(separator: " ")
    return tokens.contains { t in
        let f = BookTalkTokens.foldedWords(t)
        return f.count == 1 ? w.contains(f[0]) : compact.contains(f.joined(separator: " "))
    }
}

/// Die Vorspanne, unabhängig vom Kern geschrieben.
private func expectedPreface(_ itemId: String) -> String { "Wörtlicher Auszug aus ‚\(itemTitles[itemId] ?? "?")‘:" }

final class AssistantGateTests: XCTestCase {
    /// Namen und Angaben, die nur in den eigenen Inhalten vorkommen (ein Satz ohne Quelle darf sie nie tragen). Fachwörter wie
    /// „Wärmepumpe“ oder „Wartung“ sind Allgemeinwissen und zählen nicht: Der Satz „Eine Wärmepumpe nutzt Umweltwärme.“ sagt nichts
    /// über die Notiz des Nutzers. Zusätzlich gilt jeder Satz als eigener Inhalt, der einem Block ähnelt oder Eigendatenbezug hat.
    private let ownTokens = ["castorp", "davos", "joachim", "nordlicht", "anna", "bernd", "zauberberg", "sanatorium", "meyer"]

    private func spoken(_ events: [AssistantEvent]) -> [(text: String, status: AssistantSentenceStatus)] {
        events.compactMap { if case .sentence(_, _, let t, _, let st, _, _) = $0 { (t, st) } else { nil } }
    }

    // MARK: Tore je Klasse von Ausgabeeinheiten

    /// Tor 1: Jedes Zitat ist zeichengleich mit einem freigegebenen Block, trägt genau diesen Block als Quelle und ist ein Block.
    func testE9GateQuoteNotCharIdenticalWithAnApprovedBlockIsZero() async {
        var quotes = 0
        var violations: [String] = []
        for run in await GateCache.shared.all() {
            for u in run.units where u.quote || u.status == .grounded {
                quotes += 1
                if !u.quote || u.status != .grounded { violations.append("Zitat und Einordnung widersprechen sich: \(u.text)") }
                if !approvedBlocks.contains(u.text) { violations.append("kein freigegebener Block: \(u.text) <- \(run.output)") }
                if u.sources.count != 1 || u.sources[0].text != u.text || !u.sources[0].isBlock {
                    violations.append("Quelle ist nicht genau dieser Block: \(u.text)")
                }
                if u.text == longBlock { violations.append("Block über der Höchstlänge ausgegeben") }
            }
        }
        print("E9-Tor 1 (Zitat ≠ freigegebener Block): \(quotes) Zitate, Verstöße: \(violations.count).")
        XCTAssertGreaterThan(quotes, 50, "das Tor prüft wirklich Zitate")
        XCTAssertEqual(violations, [])
    }

    /// Tor 2: Jede Bestandsangabe ist gleich der von Hand neu erzeugten; die Folge der Angaben ist genau die erwartete.
    func testE9GateInventoryStatementDeviatingFromTheRegeneratedOneIsZero() async {
        var statements = 0, runs = 0
        var violations: [String] = []
        for run in await GateCache.shared.all() {
            let inventoryUnits = run.units.filter { $0.status == .inventory }.map(\.text)
            statements += inventoryUnits.count
            guard run.isInventory else {
                if !inventoryUnits.isEmpty { violations.append("Bestandsangabe außerhalb einer Bestandsfrage: \(inventoryUnits)") }
                continue
            }
            runs += 1
            let expected = expectedInventory.first { $0.question == run.question }?.statements ?? []
            if inventoryUnits != expected { violations.append("\(run.question) <- \(run.output): \(inventoryUnits) statt \(expected)") }
            if run.units.contains(where: { $0.status != .inventory && $0.status != .announcement }) { violations.append("außer Bestandsangaben etwas ausgegeben: \(run.question)") }
        }
        print("E9-Tor 2 (Bestandsangabe ≠ neu erzeugte): \(runs) Bestandsläufe, \(statements) Angaben, Verstöße: \(violations.count).")
        XCTAssertGreaterThan(statements, 20)
        XCTAssertEqual(violations, [])
    }

    /// Tor 3: Jede Vorlage steht in der geschlossenen Liste (Grüße, feste Sätze, Uhrzeit und Datum aus der Systemuhr).
    func testE9GateTemplateOutsideTheClosedListIsZero() async {
        var allowed = Set(AssistantGrounding.templateSentences)
        allowed.formUnion([AssistantGrounding.nothingFoundSentence, AssistantGrounding.nothingFoundSentence + " " + AssistantGrounding.optInHint,
                           AssistantGrounding.nothingGroundedSentence])
        for offset in [-60.0, 0, 60] {
            allowed.formUnion(AssistantGrounding.clockSentences(now: Date().addingTimeInterval(offset), timeZone: .current))
        }
        var templates = 0
        var violations: [String] = []
        for run in await GateCache.shared.all() {
            for u in run.units where u.status == .neutral {
                templates += 1
                if !allowed.contains(u.text) { violations.append("Vorlage außerhalb der Liste: \(u.text) <- \(run.output)") }
            }
        }
        print("E9-Tor 3 (Vorlage außerhalb der Liste): \(templates) Vorlagen, Verstöße: \(violations.count).")
        XCTAssertGreaterThan(templates, 20)
        XCTAssertEqual(violations, [])
    }

    /// Tor 4: Jede Einheit trägt ihre vorgeschriebene Kennzeichnung. Vor JEDEM Block steht die Ansage „Wörtlicher Auszug aus
    /// ‚Titel‘:“ (der Titel des Elements, unabhängig vom Kern), vor Bestandsangaben „Aus deinen Inhalten:“, vor Sätzen ohne Quelle
    /// „Ohne Quelle:“; eine Vorlage trägt die Kennzeichnung „Feste Antwort“ im Beitrag.
    func testE9GateUnitWithoutThePrescribedLabellingIsZero() async {
        var units = 0
        var violations: [String] = []
        for run in await GateCache.shared.all() {
            var previous: AssistantEvent?
            var announcedUnsourced = false
            for e in run.events {
                defer { previous = e }
                guard case .sentence(_, _, let text, _, let status, let sources, let quote) = e else { continue }
                if status == .announcement {
                    if text == AssistantGrounding.unsourcedAnnouncement { announcedUnsourced = true }
                    let known = text.hasPrefix("Wörtlicher Auszug aus ‚") && text.hasSuffix("‘:")
                        || text == AssistantGrounding.inventoryAnnouncement || text == AssistantGrounding.unsourcedAnnouncement
                    if !known { violations.append("unbekannte Ansage: \(text)") }
                    continue
                }
                units += 1
                var preceding: String?
                if case .sentence(_, _, let t, _, .announcement, _, _)? = previous { preceding = t }
                switch status {
                case .grounded:
                    guard let source = sources.first, quote else { violations.append("Zitat ohne Quelle: \(text)"); continue }
                    if preceding != expectedPreface(source.itemId) { violations.append("Zitat ohne seine Ansage (\(String(describing: preceding))): \(text)") }
                    if !source.excerptLine.hasPrefix("Wörtlicher Auszug aus ") { violations.append("Quellzeile ohne „Wörtlicher Auszug“: \(source.excerptLine)") }
                case .inventory:
                    let first = preceding == AssistantGrounding.inventoryAnnouncement
                    if !first, !run.units.contains(where: { $0.text == AssistantGrounding.inventoryAnnouncement && $0.status == .announcement }) {
                        violations.append("Bestandsangabe ohne Ansage: \(text)")
                    }
                case .unsourced:
                    if !announcedUnsourced { violations.append("Satz ohne Quelle ohne Ansage: \(text)") }
                case .neutral:
                    if AssistantTurnPart.note(for: .neutral, quote: false) == nil { violations.append("Vorlage ohne Kennzeichnung: \(text)") }
                case .announcement: break
                case .conversational, .summarized: violations.append("Einordnung des Stils natural im Stil verbatim: \(text)")
                }
            }
        }
        print("E9-Tor 4 (Einheit ohne vorgeschriebene Kennzeichnung): \(units) Einheiten, Verstöße: \(violations.count).")
        XCTAssertGreaterThan(units, 200)
        XCTAssertEqual(violations, [])
    }

    /// Tor 5: Kein Satz ohne Quelle in einer Runde, in der es ihn nicht geben darf: In der Datenrunde ist jedes Zitat ein Block
    /// mit genau einer Quelle und es gibt keinen Satz „ohne Quelle“; ohne Opt-in gibt es ihn nie; in der Allgemeinrunde nie ein
    /// Zitat und nie ein Satz, der ein Block oder ihm ähnlich ist (Eigeninhalt).
    func testE9GateNoSentenceWithoutASourceInAnyRound() async {
        let ownPassages = [AssistantPassage(area: .books, itemId: "b1", title: "x", text: approvedBlocks.sorted().joined(separator: " "), score: 1)]
        var unsourced = 0, dataRounds = 0, generalRounds = 0
        var violations: [String] = []
        for run in await GateCache.shared.all() {
            if run.failed { violations.append("Fehler statt Antwort: \(run.question) <- \(run.output)"); continue }
            let round = run.round
            if round == .general { generalRounds += 1 } else { dataRounds += 1 }
            for u in run.units where u.status != .announcement {
                switch u.status {
                case .grounded:
                    if u.sources.count != 1 { violations.append("Zitat mit \(u.sources.count) Quellen: \(u.text)") }
                    if round == .general { violations.append("Zitat in der Allgemeinrunde: \(u.text)") }
                case .unsourced:
                    unsourced += 1
                    if round != .general || !run.optIn { violations.append("Satz ohne Quelle in der Datenrunde oder ohne Opt-in: \(u.text)") }
                    let w = words(u.text).joined(separator: " ")
                    let own = ownTokens.contains { w.contains($0) } || AssistantGrounding.resemblesOwnContent(u.text, passages: ownPassages)
                        || AssistantOwnData.refers(u.text, mode: .sentence)
                    if own { violations.append("Satz ohne Quelle mit Eigeninhalt: \(u.text) <- \(run.output)") }
                    if !u.sources.isEmpty { violations.append("Satz ohne Quelle mit Quelle: \(u.text)") }
                case .inventory, .neutral:
                    if !u.sources.isEmpty || u.quote { violations.append("Systemsatz mit Quelle oder als Zitat: \(u.text)") }
                case .announcement: break
                case .conversational, .summarized: violations.append("Einordnung des Stils natural im Stil verbatim: \(u.text)")
                }
            }
        }
        print("E9-Tor 5 (Satz ohne Quelle): \(dataRounds) Datenrunden, \(generalRounds) Allgemeinrunden, \(unsourced) Sätze ohne Quelle (nur Allgemeinrunde, mit Opt-in, getrennt gezählt), Verstöße: \(violations.count).")
        XCTAssertGreaterThan(generalRounds, 5)
        XCTAssertGreaterThan(unsourced, 0, "Sätze ohne Quelle werden getrennt gezählt")
        XCTAssertEqual(violations, [])
    }

    /// Tor 6: Falsche Kernaussagen (Gift-Wörter der böswilligen Ausgaben) kommen weder in einem Block noch in einem Systemsatz vor.
    func testE9GateFalseCoreStatementsAreZero() async {
        var checked = 0
        var violations: [String] = []
        var dropped = AssistantSentenceCounts()
        for run in await GateCache.shared.all() {
            if let c = run.counts {
                dropped.droppedOwnData += c.droppedOwnData
                dropped.droppedNoMode += c.droppedNoMode
                dropped.droppedNotSnapped += c.droppedNotSnapped
                dropped.droppedDuplicate += c.droppedDuplicate
                dropped.droppedTemplate += c.droppedTemplate
                dropped.droppedFragment += c.droppedFragment
            }
            for u in run.units where u.status != .unsourced {
                checked += 1
                if containsAny(u.text, run.poison) { violations.append("falsche Kernaussage: \(u.text) <- \(run.output)") }
            }
        }
        print("""
        E9-Tor 6 (falsche Kernaussagen): \(checked) Einheiten geprüft, Verstöße: \(violations.count).
        E9-Tor verworfen: Eigendaten \(dropped.droppedOwnData), kein Modus \(dropped.droppedNoMode), nicht eingerastet \(dropped.droppedNotSnapped), \
        doppelt \(dropped.droppedDuplicate), Systemsatz nachgeahmt \(dropped.droppedTemplate), Rest ohne Satzzeichen \(dropped.droppedFragment).
        """)
        XCTAssertGreaterThan(checked, 200)
        XCTAssertEqual(violations, [])
    }

    /// Die Läufe selbst: keiner endet mit einem Fehler, es gibt Daten- und Allgemeinrunden.
    func testE9GateRunsAllEndRegularly() async {
        let runs = await GateCache.shared.all()
        XCTAssertGreaterThan(runs.count, 100)
        XCTAssertEqual(runs.filter(\.failed).count, 0)
        print("E9-Tor: \(runs.count) Läufe (\(goldCases.count) Goldfragen, \(templateCases.count) Vorlagenfragen, \(expectedInventory.count) Bestandsfragen, je mit und ohne Opt-in).")
    }

    // MARK: Produktkonfiguration und Marke je Ausgabeeinheit

    /// E9-Tor, PRODUKTKONFIGURATION (öffentlicher Initializer, Freischaltung `false`), getrennt gezählt: Auch mit gesetztem
    /// Opt-in in der Konfiguration und mit `setAllowUnsourced(true)` gibt es bei keiner Goldfrage und keiner böswilligen Ausgabe
    /// einen Satz ohne Quelle und keine Allgemeinrunde: 0 Sätze ohne Quelle.
    func testE9GateProductConfigurationHasZeroSentencesWithoutASource() async {
        XCTAssertFalse(AssistantUnsourcedConsent.isReleased)
        var runs = 0, unsourced = 0, generalRounds = 0, falseCore = 0, failures = 0
        for item in goldCases {
            for output in item.outputs {
                let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick",
                                             timeout: 5, allowUnsourced: true, consentVersion: 1)
                let session = AssistantSession(model: OutputModel(text: output), knowledge: await gateKnowledge(), config: config)
                await session.setAllowUnsourced(true, consentVersion: 1)
                var events: [AssistantEvent] = []
                for await e in session.events(AssistantRequest(requestId: "r", text: item.question)) { events.append(e) }
                runs += 1
                if case .failed? = events.last { failures += 1 }
                if case .done(_, let a)? = events.last, a.round == .general { generalRounds += 1 }
                for s in spoken(events) {
                    if s.status == .unsourced { unsourced += 1 }
                    if s.status == .grounded, containsAny(s.text, item.poison) { falseCore += 1 }
                }
            }
        }
        print("E9-Tor Produktkonfiguration (Freischaltung aus): \(runs) Läufe, Sätze ohne Quelle: \(unsourced), Allgemeinrunden: \(generalRounds), falsche Kernaussagen: \(falseCore), Fehler: \(failures).")
        XCTAssertGreaterThan(runs, 20)
        XCTAssertEqual(unsourced, 0, "Produktkonfiguration: kein Satz ohne Quelle")
        XCTAssertEqual(generalRounds, 0, "Produktkonfiguration: keine Allgemeinrunde")
        XCTAssertEqual(falseCore, 0)
        XCTAssertEqual(failures, 0)
    }

    /// Marke je Ausgabeeinheit/Quellblock: In JEDER Datenrunde der Goldfragen trägt jedes Zitat genau EINE Quelle (Ereignis und Abschnitt im Beitrag,
    /// Quellzeile „Wörtlicher Auszug aus …“), kein Satz ohne Quelle ist ein Zitat, und jeder Satz ohne Quelle (Vorlage, Bestandsangabe)
    /// trägt im Beitrag seine eigene Kennzeichnung. Ein Originalsatz ohne Quelle gibt es nicht.
    func testEveryQuoteOfADataRoundHasExactlyOneSourceAndEverySourcelessSentenceIsMarked() async {
        var rounds = 0, quotes = 0, marked = 0
        var violations: [String] = []
        for item in goldCases {
            for output in item.outputs {
                for optIn in [false, true] {
                    let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick",
                                                 timeout: 5, allowUnsourced: optIn, consentVersion: optIn ? 1 : 0)
                    let session = AssistantSession(model: OutputModel(text: output), knowledge: await gateKnowledge(), config: config, released: true)
                    var events: [AssistantEvent] = []
                    for await e in session.events(AssistantRequest(requestId: "r", text: item.question)) { events.append(e) }
                    guard case .done(_, let answer)? = events.last, answer.round == .data else { continue }
                    rounds += 1
                    var shown: [(text: String, status: AssistantSentenceStatus, quote: Bool, sources: [AssistantPassage])] = []
                    for case .sentence(_, _, let text, _, let status, let sources, let quote) in events where status != .announcement {
                        shown.append((text, status, quote, sources))
                        if quote {
                            if sources.count != 1 { violations.append("Zitat mit \(sources.count) Quellen: \(text)") }
                            if status != .grounded { violations.append("Zitat mit Einordnung \(status): \(text)") }
                            quotes += 1
                        } else if status == .grounded {
                            violations.append("Satz mit Quelle, aber kein Zitat: \(text)")
                        } else if status == .unsourced {
                            violations.append("Satz ohne Quelle in der Datenrunde: \(text)")
                        } else {
                            if !sources.isEmpty { violations.append("Systemsatz mit Quelle: \(text)") }
                            if AssistantTurnPart.note(for: status, quote: false) == nil { violations.append("Systemsatz ohne Kennzeichnung: \(text)") }
                        }
                    }
                    await session.commit(requestId: "r", user: item.question, spoken: .all, interrupted: false)
                    guard let turn = await session.history.last else {
                        if !shown.isEmpty { violations.append("kein Beitrag zu \(shown.count) Einheiten") }
                        continue
                    }
                    let parts = turn.parts
                    let quoteParts = parts.filter(\.quote)
                    if quoteParts.count != shown.filter(\.quote).count { violations.append("Zitate: \(quoteParts.count) Abschnitte zu \(shown.filter(\.quote).count) Einheiten") }
                    for (part, unit) in zip(quoteParts, shown.filter(\.quote)) {
                        guard let source = unit.sources.first, part.source == source.excerptLine else {
                            violations.append("Quellzeile des Abschnitts passt nicht zum Block: \(part.text)")
                            continue
                        }
                        if !source.excerptLine.hasPrefix("Wörtlicher Auszug aus ") { violations.append("Quellzeile ohne Kennzeichnung: \(source.excerptLine)") }
                        if part.text != unit.text || !approvedBlocks.contains(part.text) { violations.append("Abschnitt ist kein freigegebener Block: \(part.text)") }
                    }
                    for part in parts where !part.quote {
                        marked += 1
                        if part.note == nil { violations.append("Abschnitt ohne Quellzeile und ohne Kennzeichnung: \(part.text)") }
                        if part.source != nil { violations.append("Kennzeichnung mit Quellzeile: \(part.text)") }
                    }
                    if !shown.isEmpty, parts.isEmpty { violations.append("Beitrag ohne Abschnitte trotz \(shown.count) Einheiten") }
                }
            }
        }
        print("Marke je Block: \(rounds) Datenrunden, \(quotes) Zitate mit genau einer Quelle, \(marked) Abschnitte mit Kennzeichnung, Verstöße: \(violations.count).")
        XCTAssertEqual(violations, [])
        XCTAssertGreaterThan(rounds, 20)
        XCTAssertGreaterThan(quotes, 20, "das Tor prüft wirklich Zitate")
        XCTAssertGreaterThan(marked, 0, "und Einheiten mit Kennzeichnung (Vorlage)")
    }

    /// Die feste Antwort „Dazu finde ich in deinen Inhalten nichts.“ verweist in der Produktfassung nicht auf einen Schalter
    /// „Antworten ohne Quelle“, den es dort nicht gibt; nur mit freigeschalteter Funktion (Testinitialisierer) steht der Hinweis da.
    func testTheFixedAnswerPointsToNoSwitchThatDoesNotExist() async {
        let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        let output = "Der Mont Blanc ist 4806 Meter hoch."
        func fixed(_ session: AssistantSession) async -> [String] {
            var texts: [String] = []
            for await e in session.events(AssistantRequest(requestId: "r", text: "Wie hoch ist der Mont Blanc?")) {
                if case .sentence(_, _, let t, _, .neutral, _, _) = e { texts.append(t) }
            }
            return texts
        }
        let product = await fixed(AssistantSession(model: OutputModel(text: output), knowledge: await gateKnowledge(), config: config))
        XCTAssertEqual(product, [AssistantGrounding.nothingFoundSentence])
        XCTAssertFalse(product.joined().contains("ohne Quelle"), "kein Verweis auf den abgeschalteten Schalter")
        let released = await fixed(AssistantSession(model: OutputModel(text: output), knowledge: await gateKnowledge(), config: config, released: true))
        XCTAssertEqual(released, [AssistantGrounding.fixedSentence(allowUnsourced: false)])
        XCTAssertTrue(released.joined().contains(AssistantGrounding.optInHint))
    }

    /// Ohne Opt-in gibt es in keiner Rundenart einen Satz ohne Quelle, auch nicht bei Allgemeinwissen.
    func testE9GateNoUnsourcedSentenceWithoutOptIn() async {
        for item in goldCases {
            for output in item.outputs {
                let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
                let session = AssistantSession(model: OutputModel(text: output), knowledge: await gateKnowledge(), config: config, released: true)
                var events: [AssistantEvent] = []
                for await e in session.events(AssistantRequest(requestId: "r", text: item.question)) { events.append(e) }
                XCTAssertFalse(spoken(events).contains { $0.status == .unsourced }, "\(item.question) <- \(output)")
                if case .done(_, let a)? = events.last { XCTAssertEqual(a.round, .data) }
            }
        }
    }

    // MARK: Ränder: nie verändert, nie zusammengezogen, nie über der Grenze, nie aus gesperrtem Bereich

    private func run(_ knowledge: AssistantKnowledge, question: String, output: String) async -> [Unit] {
        let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        let session = AssistantSession(model: OutputModel(text: output), knowledge: knowledge, config: config, released: true)
        var events: [AssistantEvent] = []
        for await e in session.events(AssistantRequest(requestId: "r", text: question)) { events.append(e) }
        return GateRun(question: question, output: output, optIn: false, poison: [], events: events, isInventory: false).units
    }

    /// Eine Abkürzung am Zeilenende trennt nichts: Der Block „Regierungsdir.\nMeyer hat …“ kommt ganz, in einer Einheit, mit Umbruch.
    func testAnAbbreviationAtALineEndIsOutputAsOneWholeBlockNeverSplit() async {
        let units = await run(await gateKnowledge(), question: "Wer hat die Heizungswartung abgenommen?", output: "Meyer hat die Heizungswartung abgenommen.")
        let block = "Regierungsdir.\nMeyer hat die Heizungswartung abgenommen."
        XCTAssertEqual(units.map(\.text), [expectedPreface("n1"), block])
        XCTAssertEqual(units.last?.quote, true)
        XCTAssertTrue(units.last?.speech.contains("Meyer hat die Heizungswartung abgenommen") ?? false, "gesprochen wird der ganze Block")
        XCTAssertEqual(units.filter { $0.text.hasSuffix("Regierungsdir.") }.count, 0, "kein Schnitt nach der Abkürzung")
    }

    /// Zwei Blöcke werden nie zusammengezogen und nie gekürzt: Jeder erscheint für sich, mit eigener Ansage.
    func testTwoBlocksAreNeverMergedAndEachGetsItsOwnAnnouncement() async {
        let units = await run(await gateKnowledge(), question: "Wohin fährt Hans Castorp und wie lange bleibt er?",
                              output: "Hans Castorp fährt nach Davos. Er bleibt drei Wochen.")
        let texts = units.map(\.text)
        XCTAssertEqual(texts, [
            expectedPreface("b1"), "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim.",
            expectedPreface("b1"), "Er bleibt drei Wochen.\nDanach kehrt er heim.",
        ])
        XCTAssertFalse(texts.contains { $0.contains("Joachim.") && $0.contains("Wochen") }, "nie zu einer Einheit zusammengezogen")
    }

    /// Ein Block über der Höchstlänge erscheint nie, auch wenn die Frage ihn eindeutig trifft; er wird nicht gekürzt.
    func testABlockOverTheMaximumLengthNeverAppears() async {
        XCTAssertGreaterThan(longBlock.count, 600)
        let corpus = AssistantCorpus()
        for spec in corpusSpecs { await corpus.upsert(assistantDoc(spec.id, spec.title, area: spec.area, sections: spec.sections)) }
        let tooLong = await corpus.stats().tooLong
        XCTAssertEqual(tooLong, 1, "der lange Block wird nicht indexiert")
        let units = await run(AssistantKnowledge(corpus: corpus), question: "Was steht im Langbericht zum Protokoll der Heizungswartung?",
                              output: "Punkt 3 beschreibt die Wartung der Anlage im Keller ausführlich.")
        XCTAssertFalse(units.contains { $0.text.contains("Langbericht") || $0.text.contains("Punkt 3") }, "weder ganz noch gekürzt: \(units.map(\.text))")
        XCTAssertTrue(units.allSatisfy { $0.status == .announcement || approvedBlocks.contains($0.text) || $0.status == .neutral }, "\(units.map(\.text))")
    }

    /// Ein Block aus einem gesperrten Bereich erscheint nie (Bereiche aus dem Freigabezustand der Anfrage).
    func testABlockOfALockedAreaNeverAppears() async {
        let knowledge = await gateKnowledge(areas: [.books])
        let units = await run(knowledge, question: "Wann kommt der Techniker zur Wartung der Wärmepumpe?",
                              output: "Der Techniker kommt am Dienstag um 9 Uhr zur Wartung der Wärmepumpe.")
        XCTAssertFalse(units.contains { $0.text.contains("Techniker") || $0.text.contains("Dienstag") }, "\(units.map(\.text))")
        XCTAssertEqual(units.map(\.status), [.neutral], "nur der feste Satz: \(units.map(\.text))")
        XCTAssertEqual(units.first?.text, AssistantGrounding.fixedSentence(allowUnsourced: false))
    }

    /// Ein Absatz hinter der Lesegrenze erscheint nie, und er wird nicht an der Grenze gekürzt.
    func testABlockBehindTheReadingFrontierNeverAppears() async {
        let a = "Die Rübenernte beginnt im Oktober auf dem Hof Hasenwinkel und dauert bis in den November."
        let b = "Der Hof Hasenwinkel verkauft die Zuckerrüben im Dezember an die Fabrik im Nachbarort."
        let paragraphs = BookTalkIndex.blocks(in: ["# Kapitel Eins", a, b].joined(separator: "\n\n"))
        XCTAssertEqual(paragraphs.count, 2)
        let corpus = AssistantCorpus()
        let frontier = paragraphs[1].start + 20   // mitten im zweiten Absatz
        await corpus.upsert(AssistantDocument(item: AssistantItem(area: .books, id: "bx", title: "Hofbuch"), version: "1",
                                              sections: AssistantSections.book(paragraphs, frontier: frontier)))
        let units = await run(AssistantKnowledge(corpus: corpus), question: "Wann verkauft der Hof Hasenwinkel die Zuckerrüben?",
                              output: "Der Hof Hasenwinkel verkauft die Zuckerrüben im Dezember.")
        XCTAssertFalse(units.contains { $0.text.contains("Dezember") || $0.text.contains("Fabrik") }, "nichts hinter der Lesegrenze: \(units.map(\.text))")
        XCTAssertTrue(units.allSatisfy { $0.status == .announcement || $0.text == a || $0.status == .neutral })
    }

    /// Zitierbare Einheiten vorher (Sätze) und nachher (Blöcke) im Testbestand dieses Tors, grob: Sätze zerfallen an Satzzeichen, Blöcke
    /// sind die gespeicherten Abschnitte bis zur Höchstlänge.
    func testQuotableUnitsBeforeAndAfterInTheGateCorpus() async {
        let corpus = AssistantCorpus()
        for spec in corpusSpecs { await corpus.upsert(assistantDoc(spec.id, spec.title, area: spec.area, sections: spec.sections)) }
        let stats = await corpus.stats()
        let sections = corpusSpecs.flatMap(\.sections).map(\.1)
        let sentences = sections.flatMap { $0.split(whereSeparator: { ".!?".contains($0) }).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty } }
        print("Zitierbare Einheiten im Testbestand des E9-Tors: vorher (Sätze) \(sentences.count), nachher (Blöcke) \(stats.blocks), davon über der Höchstlänge nicht zitierbar: \(stats.tooLong).")
        XCTAssertEqual(stats.blocks, approvedBlocks.count)
    }
}
