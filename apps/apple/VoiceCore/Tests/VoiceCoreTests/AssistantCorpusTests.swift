import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Index über Bücher, Transkripte und Notizen.

/// Testdokument. `boundaries`: Die Abschnitte sind ganze Einheiten (Satzgrenzen an Anfang und Ende nachgewiesen); mit `false`
/// fehlt der Nachweis, dann werden Randsätze nicht zitiert.
func assistantDoc(_ id: String, _ title: String, area: AssistantArea = .books, version: String = "1", detail: String? = nil,
                  updated: Date? = nil, boundaries: Bool = true, sections: [(String?, String)]) -> AssistantDocument {
    AssistantDocument(item: AssistantItem(area: area, id: id, title: title, detail: detail, updated: updated), version: version,
                      sections: sections.map { .init(locator: $0.0, text: $0.1, startsAtBoundary: boundaries, endsAtBoundary: boundaries) })
}

/// Passage mit nachgewiesenen Satzgrenzen an beiden Rändern (ein ganzer Auszug).
func assistantPassage(_ area: AssistantArea = .books, itemId: String = "b1", title: String = "Buch", locator: String? = nil, text: String,
                      score: Double = 1, whole: Bool = true) -> AssistantPassage {
    AssistantPassage(area: area, itemId: itemId, title: title, locator: locator, text: text, score: score, startsAtBoundary: whole,
                     endsAtBoundary: whole)
}

final class AssistantCorpusTests: XCTestCase {
    private let all = Set(AssistantArea.allCases)

    /// Fließtext mit wiederkehrendem Thema, damit Stücke unterscheidbar bleiben.
    private func prose(_ topic: String, sentences: Int, seed: Int = 0) -> String {
        let fillers = ["Das ist ein ruhiger Absatz über Alltägliches.", "Man sieht viele Menschen auf der Straße gehen.",
                       "Am Abend wird es still im Haus.", "Die Zeit vergeht langsam in dieser Gegend."]
        return (0..<sentences).map { i in i % 5 == 0 ? "Hier geht es um \(topic)." : fillers[(i + seed) % fillers.count] }.joined(separator: " ")
    }

    // MARK: Chunker

    private func split(_ text: String) -> [String] { AssistantChunker.split(text).map(\.text) }

    func testChunkerKeepsShortTextAsOne() {
        XCTAssertEqual(split("Ein kurzer Text."), ["Ein kurzer Text."])
        XCTAssertEqual(split("   \n  "), [])
    }

    /// Satzgrenzen-Nachweis: Anfang und Ende des Abschnitts tragen die Flags des Abschnitts, innere Kanten nur mit Nachweis.
    func testChunkerFlagsBoundariesOnlyWithProof() {
        let short = AssistantChunker.split("Ein kurzer Text.", startsAtBoundary: true, endsAtBoundary: false)
        XCTAssertEqual(short, [.init(text: "Ein kurzer Text.", startsAtBoundary: true, endsAtBoundary: false)])
        let sentence = "Dies ist ein Satz mit einigen Wörtern darin."
        let text = Array(repeating: sentence, count: 60).joined(separator: " ")
        let pieces = AssistantChunker.split(text, startsAtBoundary: true, endsAtBoundary: true)
        XCTAssertGreaterThan(pieces.count, 3)
        XCTAssertTrue(pieces.first!.startsAtBoundary)
        XCTAssertTrue(pieces.last!.endsAtBoundary)
        // Jede Kante, die als Satzgrenze gilt, ist wirklich eine: Anfang mit Großbuchstaben, Ende mit Punkt.
        for p in pieces {
            if p.startsAtBoundary { XCTAssertTrue(p.text.hasPrefix("Dies ist"), p.text) }
            if p.endsAtBoundary { XCTAssertTrue(p.text.hasSuffix("darin."), p.text) }
        }
        // Mit Satzenden als Schnitt und Satzanfängen im Überlappungsfenster sind innere Kanten nachgewiesen.
        for p in pieces.dropLast() { XCTAssertTrue(p.endsAtBoundary, "Schnitt an Satzende: \(p.text.suffix(30))") }
        XCTAssertTrue(pieces.dropFirst().allSatisfy { $0.startsAtBoundary })
        // Ohne Abschnittsnachweis bleiben Anfang und Ende offen.
        let open = AssistantChunker.split(text)
        XCTAssertFalse(open.first!.startsAtBoundary)
        XCTAssertFalse(open.last!.endsAtBoundary)
    }

    /// Ein harter Schnitt mitten im Wort (Text ohne Satzzeichen) beweist nichts: keine der inneren Kanten gilt.
    func testChunkerHardCutsProveNothing() {
        let pieces = AssistantChunker.split(String(repeating: "wort ", count: 400), startsAtBoundary: true, endsAtBoundary: true)
        XCTAssertGreaterThan(pieces.count, 2)
        for p in pieces.dropFirst() { XCTAssertFalse(p.startsAtBoundary) }
        for p in pieces.dropLast() { XCTAssertFalse(p.endsAtBoundary) }
    }

    /// Eine Abkürzung am Schnitt ist kein Satzende („z. B.“): kein Nachweis.
    func testChunkerAbbreviationIsNoSentenceEnd() {
        let filler = String(repeating: "Das ist ein ganz gewöhnlicher Satz ohne Besonderheit. ", count: 12)
        let text = filler + "Das gilt etwa für Kosten, z. B. Miete und Strom und alles Weitere bis hier hin und weiter und weiter. " + filler
        for p in AssistantChunker.split(text, startsAtBoundary: true, endsAtBoundary: true) where p.endsAtBoundary && !p.text.hasSuffix(".") {
            XCTFail("Ende ohne Satzzeichen als Grenze gemeldet: \(p.text.suffix(30))")
        }
        for p in AssistantChunker.split(text) where p.endsAtBoundary { XCTAssertFalse(p.text.hasSuffix("z."), p.text) }
    }

    /// Absatzgrenzen (Leerzeile) zählen als Nachweis, ein einzelner Zeilenumbruch nicht.
    func testChunkerParagraphBreakProvesBoundaryButSingleNewlineDoesNot() {
        let para = String(repeating: "Wort ", count: 30).trimmingCharacters(in: .whitespaces)
        let blank = AssistantChunker.split(Array(repeating: para, count: 12).joined(separator: "\n\n"))
        XCTAssertGreaterThan(blank.count, 2)
        XCTAssertTrue(blank.dropLast().allSatisfy { $0.endsAtBoundary }, "Schnitt an der Leerzeile")
        XCTAssertTrue(blank.dropFirst().allSatisfy { $0.startsAtBoundary }, "Anfang am Absatzanfang im Überlappungsfenster")
        let single = AssistantChunker.split(Array(repeating: para, count: 12).joined(separator: "\n"))
        XCTAssertGreaterThan(single.count, 2)
        XCTAssertFalse(single.dropLast().contains { $0.endsAtBoundary }, "ein einzelner Zeilenumbruch beweist nichts")
        XCTAssertFalse(single.dropFirst().contains { $0.startsAtBoundary })
    }

    /// Kurze Abschnitte werden nur zusammengezogen, wenn die Naht nachweislich eine Satzgrenze ist.
    func testShortSectionsMergeOnlyAcrossProvenBoundaries() async {
        let proven = AssistantCorpus()
        await proven.upsert(AssistantDocument(item: AssistantItem(area: .notes, id: "n", title: "Notiz"), version: "1", sections: [
            .init(locator: "L", text: "Anna übernimmt die Planung des Projekts.", startsAtBoundary: true, endsAtBoundary: true),
            .init(locator: "L", text: "Bernd kümmert sich um das Budget des Projekts.", startsAtBoundary: true, endsAtBoundary: true),
        ]))
        let merged = await proven.search("Planung Budget Projekt", areas: all, limit: 5)
        XCTAssertEqual(merged.count, 1, "nachgewiesene Naht: ein Stück")
        XCTAssertTrue(merged[0].startsAtBoundary && merged[0].endsAtBoundary)

        let open = AssistantCorpus()
        await open.upsert(AssistantDocument(item: AssistantItem(area: .notes, id: "n", title: "Notiz"), version: "1", sections: [
            .init(locator: "L", text: "Anna übernimmt die Planung des Projekts", startsAtBoundary: true, endsAtBoundary: false),
            .init(locator: "L", text: "Bernd kümmert sich um das Budget des Projekts.", startsAtBoundary: false, endsAtBoundary: true),
        ]))
        let separate = await open.search("Planung Budget Projekt", areas: all, limit: 5)
        XCTAssertEqual(separate.count, 2, "keine nachgewiesene Naht: getrennte Stücke")
        XCTAssertTrue(separate.allSatisfy { !($0.startsAtBoundary && $0.endsAtBoundary) })
    }

    func testChunkerSizesOverlapAndBoundaries() {
        let sentence = "Dies ist ein Satz mit einigen Wörtern darin."
        let text = Array(repeating: sentence, count: 60).joined(separator: " ")
        let chunks = split(text)
        XCTAssertGreaterThan(chunks.count, 3)
        for c in chunks { XCTAssertLessThanOrEqual(c.utf16.count, 700) }
        for c in chunks.dropLast() { XCTAssertGreaterThanOrEqual(c.utf16.count, 300, "nicht winzig außer dem letzten") }
        // An Satzgrenzen geschnitten: jedes Stück außer dem letzten endet mit Satzzeichen.
        for c in chunks.dropLast() { XCTAssertTrue(c.hasSuffix("."), c) }
        // Überlappung: der Anfang jedes Stücks kommt am Ende des vorigen vor.
        for (a, b) in zip(chunks, chunks.dropFirst()) {
            let head = String(b.prefix(40))
            XCTAssertTrue(a.contains(head), "keine Überlappung zwischen \(a.suffix(60)) und \(head)")
        }
        // Nichts geht verloren: der Text ist aus den Stücken lückenlos rekonstruierbar.
        XCTAssertTrue(chunks.first!.hasPrefix("Dies ist ein Satz"))
        XCTAssertTrue(chunks.last!.hasSuffix("darin."))
    }

    func testChunkerPrefersNewlines() {
        let para = String(repeating: "Wort ", count: 100)
        let text = para + "\n" + para + "\n" + para
        let chunks = split(text)
        XCTAssertLessThanOrEqual(chunks[0].utf16.count, 500, "schneidet am Zeilenende statt bei 700")
        XCTAssertTrue(chunks.dropLast().allSatisfy { $0.hasSuffix("Wort") })
    }

    func testChunkerHandlesTextWithoutSpaces() {
        let text = String(repeating: "a", count: 2_000)
        let chunks = split(text)
        XCTAssertGreaterThan(chunks.count, 2)
        XCTAssertTrue(chunks.allSatisfy { $0.utf16.count <= 700 })
    }

    func testChunkerDoesNotSplitSurrogatePairs() {
        let text = String(repeating: "😀", count: 800)
        for c in split(text) { XCTAssertTrue(c.unicodeScalars.allSatisfy { $0 == "😀" }) }
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
        let long = (0..<30).map { i in
            (i % 2 == 0 ? "Photosynthese wandelt Licht in Energie um, Abschnitt \(i). " : "Abschnitt \(i) handelt von etwas ganz anderem. ") + String(repeating: "Füllwort ", count: 60)
        }.joined(separator: "\n")
        await corpus.upsert(assistantDoc("big", "Biologie", sections: [(nil, long)]))
        await corpus.upsert(assistantDoc("small", "Gartenbuch", sections: [(nil, "Photosynthese passiert in den Blättern deiner Tomaten.")]))
        await corpus.upsert(assistantDoc("auto", "Autos", sections: [(nil, String(repeating: "Motor Getriebe Reifen Bremsen Lenkrad. ", count: 30))]))
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

    func testShortSectionsWithSameLocatorAreMergedOthersAreNot() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("t", "Besprechung", area: .transcripts, sections: [
            ("00:01:00", "Budget Planung Quartal."), ("00:01:00", "Budget Freigabe Quartal."), ("00:02:00", "Budget Kürzung Quartal."),
        ]))
        let hits = await corpus.search("Budget Quartal", areas: all, limit: 10)
        XCTAssertEqual(hits.count, 2)
        XCTAssertEqual(Set(hits.compactMap(\.locator)), ["00:01:00", "00:02:00"])
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
                let text = (0..<8).map { "Runde\(round) Dokument\(i) Satz\($0) Wörtchen " + String(repeating: "füllung ", count: 80) }.joined(separator: "\n")
                docs.append(assistantDoc("d\(i)", "Dokument \(i)", area: .notes, version: "r\(round)", sections: [(nil, text)]))
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
                sections.append(("S. \(p)", "Hier geht es um \(topic) und ihre Folgen für Haus \(b) Seite \(p). " + String(repeating: "Dazu gibt es allerlei zu berichten, was kaum jemand wissen will. ", count: 10)))
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
