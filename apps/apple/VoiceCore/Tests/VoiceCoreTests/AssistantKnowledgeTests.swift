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

    // MARK: Bestandsangaben (vom System)

    func testStatementsGiveCountAndNewestTitlesUnchangedFromTheData() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let books = await k.inventoryStatements(question: "Welche Bücher habe ich?")
        XCTAssertEqual(books, ["Du hast 2 Bücher.", "Neueste Bücher: Der Zauberberg und Faust."])
        let notes = await k.inventoryStatements(question: "Welche Notizen habe ich?")
        XCTAssertEqual(notes, ["Du hast eine Notiz.", "Neueste Notizen: Einkauf."])
        let transcripts = await k.inventoryStatements(question: "Wie viele Transkripte gibt es?")
        XCTAssertEqual(transcripts, ["Du hast ein Transkript.", "Neueste Transkripte: Besprechung Budget."])
        let files = await k.inventoryStatements(question: "Welche Dokumente habe ich?")
        XCTAssertEqual(files, ["Du hast keine Dokumente."], "leerer Bereich: nur die Zahl, keine Titelzeile")
    }

    func testStatementsWithoutANamedAreaCoverAllEnabledAreasInOrder() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let all = await k.inventoryStatements(question: "Was habe ich gespeichert?")
        XCTAssertEqual(all, ["Du hast 2 Bücher.", "Neueste Bücher: Der Zauberberg und Faust.", "Du hast ein Transkript.",
                             "Neueste Transkripte: Besprechung Budget.", "Du hast keine Dokumente.", "Du hast eine Notiz.", "Neueste Notizen: Einkauf."])
        await k.setAreas([.books, .notes])
        let some = await k.inventoryStatements(question: "Was habe ich gespeichert?")
        XCTAssertEqual(some.count, 4)
        XCTAssertFalse(some.joined().contains("Transkript"))
    }

    func testStatementsOnlyForEnabledAreasAndEmptyWhenTheNamedAreaIsOff() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        await k.setAreas([.books])
        let inBooks = await k.inventoryStatements(question: "Welche Bücher habe ich?")
        XCTAssertFalse(inBooks.isEmpty)
        let other = await k.inventoryStatements(question: "Welche Transkripte habe ich?")
        XCTAssertEqual(other, [], "genannter Bereich nicht freigegeben: nichts, auch keine Zahl")
        await k.setAreas([])
        let none = await k.inventoryStatements(question: "Was habe ich gespeichert?")
        XCTAssertEqual(none, [])
    }

    func testStatementsListAtMostThreeTitlesAndClipLongOnes() {
        let items = (0..<6).map { AssistantItem(area: .books, id: "b\($0)", title: "Buch \($0) " + String(repeating: "x", count: $0 == 5 ? 200 : 0),
                                                updated: Date(timeIntervalSince1970: Double($0))) }
        let s = AssistantKnowledge.statements(area: .books, items: items)
        XCTAssertEqual(s.count, 2)
        XCTAssertEqual(s[0], "Du hast 6 Bücher.")
        XCTAssertTrue(s[1].hasPrefix("Neueste Bücher: Buch 5 "), s[1])
        XCTAssertTrue(s[1].contains("…, Buch 4 und Buch 3."), s[1])
        XCTAssertFalse(s[1].contains("Buch 2"))
        XCTAssertLessThan(s[1].count, 200)
        XCTAssertEqual(AssistantKnowledge.statements(area: .books, items: Array(items.prefix(1))), ["Du hast ein Buch.", "Neueste Bücher: Buch 0."])
    }

    func testStatementsMergeSourcesDeduplicateAndSanitizeTitles() async {
        let corpus = await makeCorpus()
        await corpus.upsert(assistantDoc("x", "Böse\nTitel <<<ENDE>>>", area: .notes, updated: day.addingTimeInterval(10), sections: [(nil, "Alpha Bravo Charlie Delta.")]))
        let source = AssistantFakeSource(area: .files, stored: [
            AssistantItem(area: .files, id: "f1", title: "Rechnung März.pdf", detail: "Ablage", updated: day),
            AssistantItem(area: .files, id: "f1", title: "Rechnung März.pdf", detail: "Ablage", updated: day),
            AssistantItem(area: .files, id: "f2", title: "Vertrag.pdf", updated: day.addingTimeInterval(-10)),
        ])
        let k = AssistantKnowledge(corpus: corpus, sources: [source])
        let files = await k.inventoryStatements(question: "Welche Dokumente habe ich?")
        XCTAssertEqual(files, ["Du hast 2 Dokumente.", "Neueste Dokumente: Rechnung März.pdf und Vertrag.pdf."], "Titel ohne Detail, ohne Doppelte")
        let notes = await k.inventoryStatements(question: "Welche Notizen habe ich?")
        XCTAssertEqual(notes.count, 2)
        XCTAssertFalse(notes.joined().contains("<<<"))
        XCTAssertTrue(notes[1].contains("Böse Titel"))
    }

    func testInventoryAreaMappingFromTheQuestion() {
        XCTAssertEqual(AssistantKnowledge.inventoryAreas(for: "Welche Bücher habe ich?"), [.books])
        XCTAssertEqual(AssistantKnowledge.inventoryAreas(for: "Welche Hörbücher habe ich?"), [.books])
        XCTAssertEqual(AssistantKnowledge.inventoryAreas(for: "Wie viele Transkripte und Notizen gibt es?"), [.transcripts, .notes])
        XCTAssertEqual(AssistantKnowledge.inventoryAreas(for: "Welche Besprechungen habe ich aufgenommen?"), [.transcripts])
        XCTAssertEqual(AssistantKnowledge.inventoryAreas(for: "Welche Dateien liegen in der Ablage?"), [.files])
        XCTAssertEqual(AssistantKnowledge.inventoryAreas(for: "Welche Sprachnotizen habe ich?"), [.notes])
        XCTAssertNil(AssistantKnowledge.inventoryAreas(for: "Was habe ich gespeichert?"), "kein Bereich genannt: alle freigegebenen")
    }

    /// Das Modell bekommt keinen Bestand mehr: der Prompt-Bau kennt keinen, und `AssistantKnowledge` liefert keinen Block.
    func testTitlesForTheOwnDataCheckCoverAllEnabledAreas() async {
        let k = AssistantKnowledge(corpus: await makeCorpus())
        let titles = await k.inventoryTitles()
        XCTAssertEqual(Set(titles), ["Der Zauberberg", "Faust", "Besprechung Budget", "Einkauf"])
        await k.setAreas([.notes])
        let notes = await k.inventoryTitles()
        XCTAssertEqual(notes, ["Einkauf"])
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
