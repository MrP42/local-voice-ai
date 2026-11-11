import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Index über Bücher, Transkripte und Notizen.

/// Testdokument: jeder Abschnitt ist ein gespeicherter Quellblock.
/// `kind`: Quellart der Abschnitte; ohne Angabe die zitierbare Art des Bereichs (Buch, Transkript, importiertes Dokument, Notiz).
func assistantDoc(_ id: String, _ title: String, area: AssistantArea = .books, version: String = "1", detail: String? = nil,
                  updated: Date? = nil, kind: AssistantSourceKind? = nil, sections: [(String?, String)]) -> AssistantDocument {
    let sourceKind = kind ?? assistantDefaultKind(area)
    return AssistantDocument(item: AssistantItem(area: area, id: id, title: title, detail: detail, updated: updated), version: version,
                             sections: sections.map { .init(locator: $0.0, text: $0.1, kind: sourceKind) })
}

/// Die zitierbare Quellart eines Bereichs.
func assistantDefaultKind(_ area: AssistantArea) -> AssistantSourceKind {
    switch area {
    case .books: .book
    case .transcripts: .transcript
    case .files: .importedDocument
    case .notes: .typedNote
    }
}

/// Passage als ganzer gespeicherter Block (`whole`) oder als Teil eines Blocks (`whole: false`: nicht zitierbar).
func assistantPassage(_ area: AssistantArea = .books, itemId: String = "b1", title: String = "Buch", locator: String? = nil, text: String,
                      score: Double = 1, whole: Bool = true) -> AssistantPassage {
    AssistantPassage(area: area, itemId: itemId, title: title, locator: locator, text: text, score: score, isBlock: whole, quotable: whole)
}

final class AssistantCorpusTests: XCTestCase {
    private let all = Set(AssistantArea.allCases)

    /// Fließtext mit wiederkehrendem Thema, damit Stücke unterscheidbar bleiben.
    private func prose(_ topic: String, sentences: Int, seed: Int = 0) -> String {
        let fillers = ["Das ist ein ruhiger Absatz über Alltägliches.", "Man sieht viele Menschen auf der Straße gehen.",
                       "Am Abend wird es still im Haus.", "Die Zeit vergeht langsam in dieser Gegend."]
        return (0..<sentences).map { i in i % 5 == 0 ? "Hier geht es um \(topic)." : fillers[(i + seed) % fillers.count] }.joined(separator: " ")
    }

    // MARK: Blöcke (ein Abschnitt der Quelle = ein Block im Index, nie geteilt, nie zusammengezogen)

    private func blockTexts(_ passages: [AssistantPassage]) -> [String] { passages.map(\.text) }

    /// Jeder Abschnitt wird EIN Block: Zeilenumbrüche darin bleiben, zwei Blöcke werden nie zusammengezogen.
    func testEachSectionIsOneStoredBlockAndNeverMergedOrSplit() async {
        let corpus = AssistantCorpus()
        let multi = "Dr.\nMeyer hat das Projekt Nordlicht geplant."
        let other = "Berta kümmert sich um das Budget des Projekts Nordlicht."
        await corpus.upsert(assistantDoc("n", "Notiz", area: .notes, sections: [("A", multi), ("B", other)]))
        let found = await corpus.search("Projekt Nordlicht", areas: all, limit: 5)
        XCTAssertEqual(Set(blockTexts(found)), [multi, other], "zwei Blöcke, unverändert, mit Zeilenumbruch")
        XCTAssertTrue(found.allSatisfy(\.isBlock))
        XCTAssertEqual(found.first { $0.text == multi }?.locator, "A")
    }

    /// Ein Block über `maxBlockChars` wird nicht aufgenommen (nie gekürzt, nie geteilt) und ist in der Statistik als zu lang gezählt.
    func testOverlongBlockIsNeverIndexedShortenedOrSplit() async {
        let corpus = AssistantCorpus()
        let long = String(repeating: "Das Projekt Nordlicht wird weiter geplant. ", count: 20).trimmingCharacters(in: .whitespaces)
        XCTAssertGreaterThan(long.count, AssistantGrounding.maxBlockChars)
        let ok = "Das Projekt Nordlicht startet im Mai."
        await corpus.upsert(assistantDoc("n", "Notiz", area: .notes, sections: [(nil, long), (nil, ok)]))
        let found = await corpus.search("Projekt Nordlicht", areas: all, limit: 5)
        XCTAssertEqual(blockTexts(found), [ok], "nur der kurze Block; kein Teil des langen")
        let stats = await corpus.stats(area: .notes)
        XCTAssertEqual(stats.blocks, 1)
        XCTAssertEqual(stats.tooLong, 1)
    }

    func testEmptySectionsAreSkippedAndEdgesTrimmedOnly() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n", "Notiz", area: .notes, sections: [(nil, "  \n "), (nil, "  Ein Block über Beispielburg.  ")]))
        let found = await corpus.search("Beispielburg", areas: all, limit: 5)
        XCTAssertEqual(blockTexts(found), ["Ein Block über Beispielburg."])
    }

    // MARK: Index

    func testSearchFindsPassageWithLocatorAndItem() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", sections: [
            ("S. 1", prose("Hans Castorp und die Reise nach Davos", sentences: 8)),
            ("S. 2", prose("die Heilung der Tuberkulose im Sanatorium", sentences: 8, seed: 1)),
        ]))
        let hits = await corpus.search("Wie läuft die Heilung der Tuberkulose im Sanatorium ab?", areas: all, limit: 5)
        XCTAssertFalse(hits.isEmpty)
        XCTAssertEqual(hits.first?.itemId, "b1")
        XCTAssertEqual(hits.first?.locator, "S. 2")
        XCTAssertEqual(hits.first?.title, "Der Zauberberg")
        XCTAssertEqual(hits.first?.area, .books)
        XCTAssertGreaterThan(hits.first?.score ?? 0, 0)
    }

    func testStemmingAndUmlautsMatch() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Familie", area: .notes, sections: [(nil, "Meine Väter und Großväter waren Müller am Fluss.")]))
        await corpus.upsert(assistantDoc("n2", "Einkauf", area: .notes, sections: [(nil, "Brot, Milch und Käse besorgen.")]))
        let hits = await corpus.search("Was war mein Vater von Beruf? Müller?", areas: all, limit: 3)
        XCTAssertEqual(hits.first?.itemId, "n1")
    }

    func testSameVersionUpsertIsNoOp() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Buch", version: "v1", sections: [(nil, "Alpha Bravo Charlie Delta Echo Foxtrot.")]))
        await corpus.upsert(assistantDoc("b1", "Buch", version: "v1", sections: [(nil, "Zulu Yankee Xray Whiskey Victor Uniform.")]))
        let old = await corpus.search("Alpha Bravo Charlie", areas: all, limit: 3)
        let new = await corpus.search("Zulu Yankee Xray", areas: all, limit: 3)
        XCTAssertEqual(old.count, 1)
        XCTAssertTrue(new.isEmpty)
        let version = await corpus.version(of: "b1", area: .books)
        XCTAssertEqual(version, "v1")
    }

    func testNewVersionReplacesContent() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Buch", version: "v1", sections: [(nil, "Alpha Bravo Charlie Delta Echo Foxtrot.")]))
        await corpus.upsert(assistantDoc("b1", "Buch", version: "v2", sections: [(nil, "Zulu Yankee Xray Whiskey Victor Uniform.")]))
        let old = await corpus.search("Alpha Bravo Charlie", areas: all, limit: 3)
        let new = await corpus.search("Zulu Yankee Xray", areas: all, limit: 3)
        XCTAssertTrue(old.isEmpty)
        XCTAssertEqual(new.count, 1)
        let version = await corpus.version(of: "b1", area: .books)
        XCTAssertEqual(version, "v2")
        let items = await corpus.items(area: .books)
        XCTAssertEqual(items.count, 1)
    }

    func testRemoveAndRemoveAll() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Eins", sections: [(nil, "Alpha Bravo Charlie Delta.")]))
        await corpus.upsert(assistantDoc("b2", "Zwei", sections: [(nil, "Alpha Bravo Charlie Echo.")]))
        await corpus.upsert(assistantDoc("t1", "Drei", area: .transcripts, sections: [(nil, "Alpha Bravo Charlie Foxtrot.")]))
        await corpus.remove(id: "b1", area: .books)
        var ids = Set(await corpus.search("Alpha Bravo Charlie", areas: all, limit: 10).map(\.itemId))
        XCTAssertEqual(ids, ["b2", "t1"])
        let gone = await corpus.version(of: "b1", area: .books)
        XCTAssertNil(gone)
        await corpus.removeAll(area: .books)
        ids = Set(await corpus.search("Alpha Bravo Charlie", areas: all, limit: 10).map(\.itemId))
        XCTAssertEqual(ids, ["t1"])
        let items = await corpus.items(area: nil)
        XCTAssertEqual(items.map(\.id), ["t1"])
        await corpus.remove(id: "unbekannt", area: .notes)
    }

    func testAreasFilterSearch() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Buch", sections: [(nil, "Solaranlage Wechselrichter Speicher Dach.")]))
        await corpus.upsert(assistantDoc("t1", "Termin", area: .transcripts, sections: [("00:01:00", "Solaranlage Wechselrichter Speicher Förderung.")]))
        let books = await corpus.search("Solaranlage Wechselrichter", areas: [.books], limit: 10)
        XCTAssertEqual(books.map(\.area), [.books])
        let none = await corpus.search("Solaranlage Wechselrichter", areas: [.notes], limit: 10)
        XCTAssertTrue(none.isEmpty)
        let empty = await corpus.search("Solaranlage Wechselrichter", areas: [], limit: 10)
        XCTAssertTrue(empty.isEmpty)
    }

    func testAtMostThreePassagesPerItemSoSeveralSourcesAppear() async {
        let corpus = AssistantCorpus()
        let long: [(String?, String)] = (0..<30).map { i in
            (nil, (i % 2 == 0 ? "Photosynthese wandelt Licht in Energie um, Abschnitt \(i). " : "Abschnitt \(i) handelt von etwas ganz anderem. ") + String(repeating: "Füllwort ", count: 20))
        }
        await corpus.upsert(assistantDoc("big", "Biologie", sections: long))
        await corpus.upsert(assistantDoc("small", "Gartenbuch", sections: [(nil, "Photosynthese passiert in den Blättern deiner Tomaten.")]))
        await corpus.upsert(assistantDoc("auto", "Autos", sections: [(nil, String(repeating: "Motor Getriebe Reifen Bremsen Lenkrad. ", count: 10))]))
        let hits = await corpus.search("Photosynthese", areas: all, limit: 8)
        XCTAssertEqual(hits.filter { $0.itemId == "big" }.count, 3)
        XCTAssertTrue(hits.contains { $0.itemId == "small" })
    }

    func testTitleMentionedInQuestionBoostsThatItem() async {
        let corpus = AssistantCorpus()
        let filler = "Der Held reist durch das Land und trifft viele Freunde auf dem Weg."
        await corpus.upsert(assistantDoc("faust", "Faust", sections: [("S. 1", filler + " Ein Gelehrter zweifelt am Wissen.")]))
        await corpus.upsert(assistantDoc("other", "Reiseführer", sections: [("S. 1", filler + " Ein Gelehrter zweifelt am Wissen der Reisenden und am Land.")]))
        let plain = await corpus.search("Was zweifelt der Gelehrte am Wissen?", areas: all, limit: 2)
        let boosted = await corpus.search("Was zweifelt der Gelehrte am Wissen in Faust?", areas: all, limit: 2)
        XCTAssertEqual(boosted.first?.itemId, "faust")
        XCTAssertNotNil(plain.first)
    }

    func testTitleOnlyQuestionStillReturnsFirstPassages() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("proz", "Der Prozess", sections: [
            ("S. 1", "Jemand musste Josef K. verleumdet haben, denn ohne dass er etwas Böses getan hätte, wurde er eines Morgens verhaftet."),
            ("S. 9", "Ein späterer Abschnitt über Gericht und Dachboden."),
        ]))
        await corpus.upsert(assistantDoc("x", "Anderes", sections: [(nil, "Etwas völlig anderes über Gärten.")]))
        let hits = await corpus.search("Worum geht es in dem Prozess?", areas: all, limit: 3)
        XCTAssertEqual(hits.first?.itemId, "proz")
        XCTAssertFalse(hits.contains { $0.itemId == "x" })
    }

    func testWeakSingleCommonWordMatchesAreDroppedForLongerQuestions() async {
        let corpus = AssistantCorpus()
        for i in 0..<40 {
            await corpus.upsert(assistantDoc("d\(i)", "Doc \(i)", area: .notes, sections: [(nil, "Gespräch Nummer \(i) über Alltag und Arbeit im Büro.")]))
        }
        let hits = await corpus.search("Erkläre die Quantenverschränkung beim Gespräch zwischen Photonen", areas: all, limit: 5)
        XCTAssertTrue(hits.isEmpty, "\(hits.map(\.itemId))")
    }

    func testPreviousQuestionHelpsShortFollowUps() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("a", "Heizung", area: .notes, sections: [(nil, "Die Wärmepumpe im Keller braucht eine neue Wartung, Termin im März.")]))
        await corpus.upsert(assistantDoc("b", "Auto", area: .notes, sections: [(nil, "Der Termin für den Reifenwechsel ist im Oktober beim Händler.")]))
        let without = await corpus.search("Und wann ist der Termin?", previous: nil, areas: all, limit: 2)
        let with = await corpus.search("Und wann ist der Termin?", previous: "Was ist mit der Wärmepumpe im Keller?", areas: all, limit: 2)
        XCTAssertEqual(with.first?.itemId, "a")
        XCTAssertEqual(Set(without.map(\.itemId)), ["a", "b"])
    }

    /// Auch kurze Blöcke mit demselben Ort werden NIE zusammengezogen: jeder Abschnitt bleibt ein eigener Block.
    func testShortSectionsWithSameLocatorStayDistinctBlocks() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("t", "Besprechung", area: .transcripts, sections: [
            ("00:01:00", "Budget Planung Quartal."), ("00:01:00", "Budget Freigabe Quartal."), ("00:02:00", "Budget Kürzung Quartal."),
        ]))
        let hits = await corpus.search("Budget Quartal", areas: all, limit: 10)
        XCTAssertEqual(hits.count, 3)
        XCTAssertEqual(Set(blockTexts(hits)), ["Budget Planung Quartal.", "Budget Freigabe Quartal.", "Budget Kürzung Quartal."])
    }

    func testItemsAreSortedNewestFirst() async {
        let corpus = AssistantCorpus()
        let d = Date(timeIntervalSince1970: 1_700_000_000)
        await corpus.upsert(assistantDoc("old", "Alt", area: .notes, updated: d, sections: [(nil, "Alpha Bravo Charlie.")]))
        await corpus.upsert(assistantDoc("new", "Neu", area: .notes, updated: d.addingTimeInterval(1_000), sections: [(nil, "Delta Echo Foxtrot.")]))
        await corpus.upsert(assistantDoc("undated", "Ohne", area: .notes, sections: [(nil, "Golf Hotel India.")]))
        let ids = await corpus.items(area: .notes).map(\.id)
        XCTAssertEqual(ids, ["new", "old", "undated"])
    }

    func testEmptyQuestionAndEmptyCorpus() async {
        let corpus = AssistantCorpus()
        let a = await corpus.search("irgendwas Interessantes hier", areas: all, limit: 5)
        XCTAssertTrue(a.isEmpty)
        await corpus.upsert(assistantDoc("b", "Buch", sections: [(nil, "Alpha Bravo Charlie.")]))
        let b = await corpus.search("   ", areas: all, limit: 5)
        XCTAssertTrue(b.isEmpty)
        let c = await corpus.search("Alpha", areas: all, limit: 0)
        XCTAssertTrue(c.isEmpty)
        // Dokument ohne indexierbaren Text bleibt als Element im Bestand.
        await corpus.upsert(assistantDoc("leer", "Leer", sections: [(nil, "   ")]))
        let items = await corpus.items(area: .books)
        XCTAssertEqual(Set(items.map(\.id)), ["b", "leer"])
    }

    func testCompactionKeepsResultsCorrect() async {
        let corpus = AssistantCorpus()
        // Genug Wechsel, dass mehrfach verdichtet wird (> 2 000 tote Stücke).
        for round in 0..<12 {
            var docs: [AssistantDocument] = []
            for i in 0..<30 {
                let blocks: [(String?, String)] = (0..<8).map { (nil, "Runde\(round) Dokument\(i) Satz\($0) Wörtchen " + String(repeating: "füllung ", count: 20)) }
                docs.append(assistantDoc("d\(i)", "Dokument \(i)", area: .notes, version: "r\(round)", sections: blocks))
            }
            for d in docs { await corpus.upsert(d) }
        }
        let hits = await corpus.search("Runde11 Dokument7 Satz3", areas: all, limit: 5)
        XCTAssertEqual(hits.first?.itemId, "d7")
        XCTAssertTrue(hits.first?.text.contains("Runde11") ?? false)
        let stale = await corpus.search("Runde3 Runde4 Runde5", areas: all, limit: 5)
        XCTAssertTrue(stale.isEmpty)
        let items = await corpus.items(area: .notes)
        XCTAssertEqual(items.count, 30)
    }

    // MARK: Leistung

    func testSearchOver2000ChunksIsFast() async {
        let corpus = AssistantCorpus()
        let topics = ["Photosynthese", "Zinsen", "Wärmepumpe", "Kubernetes", "Marathon", "Impfung", "Romanik", "Schach"]
        for b in 0..<40 {
            var sections: [(String?, String)] = []
            for p in 0..<50 {
                let topic = topics[(b + p) % topics.count]
                sections.append(("S. \(p)", "Hier geht es um \(topic) und ihre Folgen für Haus \(b) Seite \(p). " + String(repeating: "Dazu gibt es allerlei zu berichten, was kaum jemand wissen will. ", count: 6)))
            }
            await corpus.upsert(assistantDoc("b\(b)", "Buch \(b)", sections: sections))
        }
        let count = await corpus.items(area: .books).count
        XCTAssertEqual(count, 40)
        let start = ContinuousClock.now
        var results: [AssistantPassage] = []
        for _ in 0..<10 { results = await corpus.search("Was steht über Kubernetes und die Folgen für Haus 7?", areas: all, limit: 6) }
        let ms = start.duration(to: .now).components.attoseconds / 1_000_000_000_000_000 + start.duration(to: .now).components.seconds * 1000
        XCTAssertFalse(results.isEmpty)
        XCTAssertLessThan(Double(ms) / 10, 50, "eine Suche über 2 000 Stücke dauert \(Double(ms) / 10) ms")
    }
}
