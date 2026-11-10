import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Abruf und Bestand.

struct AssistantFakeSource: AssistantSource {
    let area: AssistantArea
    var stored: [AssistantItem] = []
    var passages: [AssistantPassage] = []
    var seen = AssistantFakeSourceLog()

    func items() async -> [AssistantItem] { stored }

    func search(_ question: String, limit: Int) async -> [AssistantPassage] {
        seen.add(question)
        return Array(passages.prefix(limit))
    }
}

final class AssistantFakeSourceLog: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [String] = []
    func add(_ q: String) { lock.lock(); all.append(q); lock.unlock() }
    var questions: [String] { lock.lock(); defer { lock.unlock() }; return all }
}

final class AssistantKnowledgeTests: XCTestCase {
    private let day = Date(timeIntervalSince1970: 1_760_000_000)

    private func makeCorpus() async -> AssistantCorpus {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann · 42 % gelesen", updated: day, sections: [
            ("S. 12", "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."),
        ]))
        await corpus.upsert(assistantDoc("b2", "Faust", detail: "Goethe · nicht begonnen", updated: day.addingTimeInterval(-86_400), sections: [
            ("S. 3", "Habe nun, ach! Philosophie, Juristerei und Medizin durchaus studiert mit heißem Bemühen."),
        ]))
        await corpus.upsert(assistantDoc("t1", "Besprechung Budget", area: .transcripts, detail: "12.10.2026 · 48 min", updated: day, sections: [
            ("00:14:03", "Wir haben das Budget für das nächste Quartal besprochen und die Kürzung bei Marketing beschlossen."),
        ]))
        await corpus.upsert(assistantDoc("n1", "Einkauf", area: .notes, updated: day, sections: [(nil, "Milch, Brot und Käse besorgen.")]))
        return corpus
    }

    // MARK: Bestand

    func testInventoryListsCountsAndNewestFirst() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let inv = await k.inventory(maxChars: 2_000)
        let lines = inv.components(separatedBy: "\n")
        XCTAssertEqual(lines.first, "Bücher: 2 insgesamt")
        XCTAssertEqual(lines[1], "- Der Zauberberg (Thomas Mann · 42 % gelesen)")
        XCTAssertEqual(lines[2], "- Faust (Goethe · nicht begonnen)")
        XCTAssertTrue(lines.contains("Transkripte: 1 insgesamt"))
        XCTAssertTrue(lines.contains("- Besprechung Budget (12.10.2026 · 48 min)"))
        XCTAssertTrue(lines.contains("Dokumente: keine"), inv)
        XCTAssertTrue(lines.contains("Notizen: 1 insgesamt"))
        XCTAssertTrue(lines.contains("- Einkauf"))
    }

    func testInventoryHonoursMaxCharsStrictly() async {
        let corpus = AssistantCorpus()
        for i in 0..<60 {
            await corpus.upsert(assistantDoc("b\(i)", "Ein ziemlich langer Buchtitel Nummer \(i)", detail: "Autorin \(i) · \(i) % gelesen",
                                             updated: day.addingTimeInterval(Double(i)), sections: [(nil, "Text Nummer \(i) mit Inhalt.")]))
        }
        let k = AssistantKnowledge(corpus: corpus)
        for limit in [1, 5, 20, 40, 100, 300, 1_000, 5_000] {
            let inv = await k.inventory(maxChars: limit)
            XCTAssertLessThanOrEqual(inv.count, limit, "limit \(limit): \(inv.count)")
        }
        let inv = await k.inventory(maxChars: 5_000)
        XCTAssertTrue(inv.contains("Bücher: 60 insgesamt"))
        XCTAssertTrue(inv.contains("Nummer 59"), "neueste zuerst")
        XCTAssertFalse(inv.contains("Nummer 0 "), "höchstens 15 Elemente")
        let empty = await k.inventory(maxChars: 0)
        XCTAssertEqual(empty, "")
    }

    func testInventoryOnlyForEnabledAreas() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        await k.setAreas([.books])
        let inv = await k.inventory(maxChars: 2_000)
        XCTAssertTrue(inv.contains("Bücher"))
        XCTAssertFalse(inv.contains("Transkripte"))
        XCTAssertFalse(inv.contains("Besprechung"))
        await k.setAreas([])
        let none = await k.inventory(maxChars: 2_000)
        XCTAssertEqual(none, "")
    }

    func testInventoryMergesSourcesAndDeduplicates() async {
        let corpus = await makeCorpus()
        let source = AssistantFakeSource(area: .files, stored: [
            AssistantItem(area: .files, id: "f1", title: "Rechnung März.pdf", detail: "Ablage", updated: day),
            AssistantItem(area: .files, id: "f2", title: "Vertrag.pdf", updated: day.addingTimeInterval(-10)),
        ])
        let k = AssistantKnowledge(corpus: corpus, sources: [source])
        let inv = await k.inventory(maxChars: 2_000)
        XCTAssertTrue(inv.contains("Dokumente: 2 insgesamt"))
        XCTAssertTrue(inv.contains("- Rechnung März.pdf (Ablage)"))
    }

    func testInventorySanitizesTitles() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("x", "Böse\nTitel <<<ENDE>>>", area: .notes, sections: [(nil, "Alpha Bravo Charlie Delta.")]))
        let inv = await AssistantKnowledge(corpus: corpus).inventory(maxChars: 1_000)
        XCTAssertFalse(inv.contains("<<<"))
        XCTAssertTrue(inv.contains("- Böse Titel"))
    }

    // MARK: Abruf

    func testRetrieveFindsPassagesAcrossAreas() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let hits = await k.retrieve(question: "Was wurde im Budget für das nächste Quartal beschlossen?", previous: nil, limit: 5, maxChars: 4_000)
        XCTAssertEqual(hits.first?.itemId, "t1")
        XCTAssertEqual(hits.first?.locator, "00:14:03")
        XCTAssertLessThanOrEqual(hits.first?.score ?? 2, 1)
    }

    func testRetrieveSkipsDisabledAreasEverywhere() async {
        let source = AssistantFakeSource(area: .files, passages: [
            AssistantPassage(area: .files, itemId: "f1", title: "Budgetplan.pdf", locator: "Seite 2", text: "Budget Quartal Planung der Abteilung.", score: 9),
        ])
        let k = AssistantKnowledge(corpus: await makeCorpus(), sources: [source])
        await k.setAreas([.books])
        let hits = await k.retrieve(question: "Budget Quartal Kürzung Marketing Planung", previous: nil, limit: 5, maxChars: 4_000)
        XCTAssertTrue(hits.allSatisfy { $0.area == .books }, "\(hits.map(\.area))")
        XCTAssertTrue(source.seen.questions.isEmpty, "deaktivierte Quelle wird nicht einmal gefragt")
        let areas = await k.areas
        XCTAssertEqual(areas, [.books])
    }

    func testRetrieveNormalizesPerAreaAndInterleavesSources() async {
        let source = AssistantFakeSource(area: .files, passages: [
            AssistantPassage(area: .files, itemId: "f1", title: "Rechnung.pdf", locator: "Seite 1", text: "Rechnung Solaranlage Betrag fällig.", score: 250),
            AssistantPassage(area: .files, itemId: "f2", title: "Angebot.pdf", locator: "Seite 1", text: "Angebot Solaranlage Preis.", score: 25),
        ])
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Notiz", area: .notes, sections: [(nil, "Solaranlage beim Nachbarn anschauen, Wechselrichter fragen.")]))
        let k = AssistantKnowledge(corpus: corpus, sources: [source])
        let hits = await k.retrieve(question: "Solaranlage Wechselrichter", previous: nil, limit: 5, maxChars: 4_000)
        let scores = Dictionary(uniqueKeysWithValues: hits.map { ($0.itemId, $0.score) })
        XCTAssertEqual(scores["f1"] ?? 0, 1, accuracy: 0.0001)
        XCTAssertEqual(scores["f2"] ?? 0, 0.1, accuracy: 0.0001)
        XCTAssertEqual(scores["n1"] ?? 0, 1, accuracy: 0.0001)
        XCTAssertEqual(Set(hits.prefix(2).map(\.area)), [.files, .notes], "jede Quelle mit ihrer besten Stelle vorn")
        XCTAssertEqual(hits.map(\.score), hits.map(\.score).sorted(by: >))
    }

    func testRetrieveRespectsMaxCharsAndLimit() async {
        let corpus = AssistantCorpus()
        for i in 0..<6 {
            await corpus.upsert(assistantDoc("d\(i)", "Dokument \(i)", area: .notes, sections: [
                (nil, "Wärmepumpe Wartung Termin \(i). " + String(repeating: "Dazu gibt es viel zu sagen über Heizung und Keller. ", count: 12)),
            ]))
        }
        let k = AssistantKnowledge(corpus: corpus)
        let hits = await k.retrieve(question: "Wärmepumpe Wartung Termin", previous: nil, limit: 4, maxChars: 1_000)
        XCTAssertLessThanOrEqual(hits.count, 4)
        XCTAssertLessThanOrEqual(hits.reduce(0) { $0 + $1.text.count }, 1_000)
        XCTAssertFalse(hits.isEmpty)
        let tiny = await k.retrieve(question: "Wärmepumpe Wartung Termin", previous: nil, limit: 4, maxChars: 50)
        XCTAssertLessThanOrEqual(tiny.reduce(0) { $0 + $1.text.count }, 50)
        let none = await k.retrieve(question: "Wärmepumpe Wartung Termin", previous: nil, limit: 0, maxChars: 1_000)
        XCTAssertTrue(none.isEmpty)
    }

    func testPureInventoryQuestionsGetNoRandomExcerpts() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        for q in ["Welche Bücher habe ich?", "wie viele Transkripte habe ich", "Was habe ich zuletzt aufgenommen?", "Welche Notizen gibt es?",
                  "Zeig mir meine Dokumente", "Was ist in meiner Bibliothek?"] {
            let hits = await k.retrieve(question: q, previous: nil, limit: 5, maxChars: 4_000)
            XCTAssertTrue(hits.isEmpty, q)
        }
    }

    func testInventoryQuestionDetection() {
        XCTAssertTrue(AssistantKnowledge.isInventoryQuestion("Welche Bücher habe ich?"))
        XCTAssertTrue(AssistantKnowledge.isInventoryQuestion("Wie viele Transkripte habe ich"))
        XCTAssertTrue(AssistantKnowledge.isInventoryQuestion("Was habe ich zuletzt aufgenommen?"))
        XCTAssertFalse(AssistantKnowledge.isInventoryQuestion("Welche Bücher habe ich über Solarenergie?"))
        XCTAssertFalse(AssistantKnowledge.isInventoryQuestion("Worum geht es im Buch Faust?"))
        XCTAssertFalse(AssistantKnowledge.isInventoryQuestion("Was wurde im Transkript vom Montag zum Budget gesagt?"))
        XCTAssertFalse(AssistantKnowledge.isInventoryQuestion("Wie wird das Wetter morgen?"))
    }

    func testContentQuestionsAboutTopicStillRetrieve() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let hits = await k.retrieve(question: "Welche Bücher habe ich über das Sanatorium in Davos?", previous: nil, limit: 3, maxChars: 4_000)
        XCTAssertEqual(hits.first?.itemId, "b1")
    }

    func testShortFollowUpUsesPreviousUtterance() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let with = await k.retrieve(question: "Und wie ging es dazu weiter?", previous: "Erzähl mir etwas über das Budget für das nächste Quartal", limit: 3, maxChars: 4_000)
        XCTAssertEqual(with.first?.itemId, "t1")
        let without = await k.retrieve(question: "Und wie ging es dazu weiter?", previous: nil, limit: 3, maxChars: 4_000)
        XCTAssertNotEqual(without.first?.itemId, "t1")
    }

    func testFollowUpDetection() {
        XCTAssertTrue(AssistantKnowledge.isFollowUp("Und was ist damit?"))
        XCTAssertTrue(AssistantKnowledge.isFollowUp("Wann war das?"))
        XCTAssertTrue(AssistantKnowledge.isFollowUp("Erzähl mir mehr dazu über die ganzen Hintergründe und Zusammenhänge der Planung"))
        XCTAssertFalse(AssistantKnowledge.isFollowUp("Erkläre mir bitte ausführlich die Unterschiede zwischen Wärmepumpen und Gasheizungen in Altbauten"))
    }

    func testSourceReceivesBlendedQuestionForFollowUps() async {
        let source = AssistantFakeSource(area: .files)
        let k = AssistantKnowledge(corpus: AssistantCorpus(), sources: [source])
        _ = await k.retrieve(question: "Und davon?", previous: "Rechnung der Solaranlage", limit: 3, maxChars: 1_000)
        XCTAssertEqual(source.seen.questions, ["Und davon? Rechnung der Solaranlage"])
    }

    func testCapsPassagesPerItemAcrossCorpusAndSource() async {
        let src = (0..<5).map {
            AssistantPassage(area: .files, itemId: "f1", title: "Lang.pdf", locator: "Seite \($0)", text: "Passage \($0) Solaranlage Wechselrichter", score: Double(10 - $0))
        }
        let k = AssistantKnowledge(corpus: AssistantCorpus(), sources: [AssistantFakeSource(area: .files, passages: src)])
        let hits = await k.retrieve(question: "Solaranlage Wechselrichter", previous: nil, limit: 10, maxChars: 4_000)
        XCTAssertEqual(hits.count, 3)
    }

    func testEmptyQuestionRetrievesNothing() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let hits = await k.retrieve(question: "   ", previous: "Budget", limit: 3, maxChars: 1_000)
        XCTAssertTrue(hits.isEmpty)
    }
}
