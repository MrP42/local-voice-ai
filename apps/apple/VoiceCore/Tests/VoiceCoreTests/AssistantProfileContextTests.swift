import Foundation
import XCTest
@testable import VoiceCore

// Verknüpfte Kontexte je Assistent und je Gespräch (lva#557, Epic #554): Filter im vorhandenen Abruf (`AssistantScope`), Modus
// „nur“ und „zuerst“, Verfügbarkeit je Kontext, Kontexte des Gesprächs. Nur erfundene Daten.

/// Eine Quelle mit eigenem Filter im Index (wie die Ablage am iPhone): merkt sich die Kennungen, mit denen sie gefragt wurde.
private final class ContextRecordingSource: AssistantSource, @unchecked Sendable {
    let area: AssistantArea
    private let lock = NSLock()
    private var asked: [Set<String>?] = []
    private let all: [AssistantPassage]
    private let stored: [AssistantItem]

    init(area: AssistantArea, items: [AssistantItem], passages: [AssistantPassage]) {
        self.area = area
        stored = items
        all = passages
    }

    var requests: [Set<String>?] { lock.withLock { asked } }

    func items() async -> [AssistantItem] { stored }

    func search(_ question: String, limit: Int) async -> [AssistantPassage] { await search(question, limit: limit, only: nil) }

    func search(_ question: String, limit: Int, only itemIds: Set<String>?) async -> [AssistantPassage] {
        lock.withLock { asked.append(itemIds) }
        let kept = itemIds.map { ids in all.filter { ids.contains($0.itemId) } } ?? all
        return Array(kept.prefix(limit))
    }
}

private struct ContextNoModel: BookTalkModel {
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { $0.finish() }
    }
}

final class AssistantProfileContextTests: XCTestCase {
    private let everything = Set(AssistantArea.allCases)

    private func ctx(_ kind: AssistantProfile.Context.Kind, _ id: String, _ title: String = "") -> AssistantProfile.Context {
        AssistantProfile.Context(kind: kind, id: id, title: title.isEmpty ? id : title)
    }

    private func only(_ items: [(AssistantArea, String)], folders: Set<String> = []) -> AssistantScope {
        AssistantScope(mode: .only, items: Set(items.map { AssistantScope.Entry(area: $0.0, itemId: $0.1) }), folders: folders)
    }

    private func first(_ items: [(AssistantArea, String)]) -> AssistantScope {
        AssistantScope(mode: .first, items: Set(items.map { AssistantScope.Entry(area: $0.0, itemId: $0.1) }))
    }

    private func ids(_ passages: [AssistantPassage]) -> [String] { passages.map(\.itemId) }

    private func retrieve(_ k: AssistantKnowledge, _ question: String, limit: Int = 6, areas: Set<AssistantArea>? = nil) async -> [AssistantPassage] {
        await k.retrieve(question: question, previous: nil, limit: limit, maxChars: 8_000, areas: areas)
    }

    // MARK: Bestand je Bereich: Kontext und besseres Gegenbeispiel

    private let tomatoes = "Wie ist das mit den Tomaten, der Sonne und dem Gießen?"

    /// Je Bereich ein Element im Kontext (schwächerer Treffer) und ein Gegenbeispiel außerhalb (besserer Treffer).
    private func fourAreas() async -> AssistantCorpus {
        let corpus = AssistantCorpus()
        let weak = "Die Tomaten brauchen viel Sonne und regelmäßiges Gießen im Hochsommer."
        let strong = "Tomaten, Tomaten, Tomaten: Tomaten gedeihen bei Sonne, Tomaten brauchen Gießen, Gießen und nochmals Gießen in der Sonne."
        for area in AssistantArea.allCases {
            await corpus.upsert(assistantDoc("\(area.rawValue)-ctx", "Kontext \(area.rawValue)", area: area, sections: [("A", weak)]))
            await corpus.upsert(assistantDoc("\(area.rawValue)-out", "Fremd \(area.rawValue)", area: area, sections: [("B", strong)]))
        }
        return corpus
    }

    func testOnlyReturnsNothingOutsideTheContextsInEveryArea() async {
        let corpus = await fourAreas()
        let open = AssistantKnowledge(corpus: corpus)
        let baseline = await retrieve(open, tomatoes)
        XCTAssertTrue(baseline.contains { $0.itemId.hasSuffix("-out") }, "Gegenprobe: ohne Kontexte stehen die besseren Fremdtreffer da")
        for area in AssistantArea.allCases {
            let k = AssistantKnowledge(corpus: corpus).withScope(only([(area, "\(area.rawValue)-ctx")]))
            let found = await retrieve(k, tomatoes)
            XCTAssertEqual(ids(found), ["\(area.rawValue)-ctx"], "\(area): \(ids(found))")
            let titles = await k.inventoryTitles()
            XCTAssertEqual(titles, ["Kontext \(area.rawValue)"], "Bestand nur aus dem Kontext")
            let groups = await k.inventoryGroups(question: "Was habe ich gespeichert?")
            XCTAssertEqual(groups.map(\.area), [area], "Bereiche ohne Kontext stehen nicht im Bestand")
            XCTAssertEqual(groups.first?.statements.first, "Du hast \(area.countPhrase(1)).")
        }
        let all = AssistantKnowledge(corpus: corpus).withScope(only(AssistantArea.allCases.map { ($0, "\($0.rawValue)-ctx") }))
        let found = await retrieve(all, tomatoes, limit: 8)
        XCTAssertEqual(Set(ids(found)), Set(AssistantArea.allCases.map { "\($0.rawValue)-ctx" }))
        let titles = await all.inventoryTitles()
        XCTAssertFalse(titles.contains { $0.hasPrefix("Fremd") })
        let counted = await all.inventoryStatements(question: "Welche Bücher habe ich?")
        XCTAssertEqual(counted, ["Du hast ein Buch.", "Neueste Bücher: Kontext books."])
    }

    func testOnlyFiltersBeforeTheLimitSoManyBetterForeignHitsDoNotCrowdOutTheContext() async {
        let corpus = AssistantCorpus()
        let strong = "Quartalsbericht Umsatz Marge Quartalsbericht Umsatz Marge: Umsatz und Marge im Quartalsbericht, Umsatz Umsatz."
        for i in 0..<40 {
            await corpus.upsert(assistantDoc("f\(i)", "Fremdbuch \(i)", sections: [("S. \(i)", strong)]))
        }
        await corpus.upsert(assistantDoc("ctx", "Mein Buch", sections: [("S. 1", "Im Quartalsbericht steht der Umsatz neben der Marge der Filiale.")]))
        let question = "Was steht im Quartalsbericht zu Umsatz und Marge?"
        let open = AssistantKnowledge(corpus: corpus)
        let baseline = await retrieve(open, question, limit: 3)
        XCTAssertFalse(ids(baseline).contains("ctx"), "Gegenprobe: ohne Filter verdrängen die Fremdtreffer den Kontext: \(ids(baseline))")
        let scoped = await retrieve(open.withScope(only([(.books, "ctx")])), question, limit: 3)
        XCTAssertEqual(ids(scoped), ["ctx"])
    }

    func testOnlyWithAllContextsUnavailableIsEmptyNeverEverything() async {
        let corpus = await fourAreas()
        let gone = AssistantKnowledge(corpus: corpus).withScope(only([(.books, "gibt-es-nicht")]))
        let nothing = await retrieve(gone, tomatoes)
        XCTAssertEqual(nothing, [])
        let titles = await gone.inventoryTitles()
        XCTAssertEqual(titles, [])
        let groups = await gone.inventoryGroups(question: "Was habe ich gespeichert?")
        XCTAssertEqual(groups.count, 0)
        // Nur Arten ohne Abbildung: der Scope ist leer, bei „nur“ bleibt das Ergebnis leer.
        let unsupported = AssistantScope.make(mode: .only, contexts: [ctx(.project, "p1"), ctx(.section, "s1")])
        XCTAssertEqual(unsupported, AssistantScope(mode: .only))
        let k = AssistantKnowledge(corpus: corpus).withScope(unsupported)
        let none = await retrieve(k, tomatoes)
        XCTAssertEqual(none, [])
        // Bei „zuerst“ wirkt ein leerer Scope wie kein Scope.
        let firstEmpty = AssistantKnowledge(corpus: corpus).withScope(AssistantScope.make(mode: .first, contexts: [ctx(.project, "p1")]))
        let open = await retrieve(AssistantKnowledge(corpus: corpus), tomatoes)
        let same = await retrieve(firstEmpty, tomatoes)
        XCTAssertEqual(same, open)
    }

    func testOnlyReachesSourcesWithTheFilterInsideTheirIndex() async {
        let corpus = await fourAreas()
        let item = { (id: String) in AssistantItem(area: .files, id: id, title: "Datei \(id)") }
        let passages = (0..<30).map { i in
            assistantPassage(.files, itemId: "x\(i)", title: "Datei x\(i)", locator: "S. 1", text: "Tomaten Sonne Gießen \(i)", score: 1 / Double(1 + i), whole: false)
        } + [assistantPassage(.files, itemId: "ctx", title: "Datei ctx", locator: "S. 2", text: "Tomaten Sonne Gießen im Kontext", score: 0.001, whole: false)]
        let source = ContextRecordingSource(area: .files, items: [item("ctx")] + (0..<30).map { item("x\($0)") }, passages: passages)
        let k = AssistantKnowledge(corpus: AssistantCorpus(), sources: [source]).withScope(only([(.files, "ctx")]))
        let found = await retrieve(k, tomatoes, limit: 3)
        XCTAssertEqual(ids(found), ["ctx"], "der Filter steht vor der Begrenzung der Quelle")
        XCTAssertEqual(source.requests, [Set(["ctx"])])
        _ = corpus
        // Eine Quelle, die den Filter nicht kennt (Vorgabe des Protokolls), liefert trotzdem nichts Fremdes.
        let plain = AssistantFakeSource(area: .files, stored: [item("ctx"), item("x1")], passages: [
            assistantPassage(.files, itemId: "x1", title: "Datei x1", text: "Tomaten Sonne Gießen", score: 1, whole: false),
            assistantPassage(.files, itemId: "ctx", title: "Datei ctx", text: "Tomaten Sonne Gießen", score: 0.5, whole: false),
        ])
        let kp = AssistantKnowledge(corpus: AssistantCorpus(), sources: [plain]).withScope(only([(.files, "ctx")]))
        let foundPlain = await retrieve(kp, tomatoes)
        XCTAssertEqual(ids(foundPlain), ["ctx"])
    }

    // MARK: zuerst

    func testFirstPutsContextHitsAheadAndKeepsTheBestOutsideHit() async {
        let corpus = AssistantCorpus()
        let q = "Welche Frist gilt für den Antrag auf Förderung der Heizung?"
        await corpus.upsert(assistantDoc("o1", "Fremd eins", sections: [("A", "Die Frist für den Antrag auf Förderung der Heizung endet im März, Frist Antrag Förderung Heizung.")]))
        await corpus.upsert(assistantDoc("o2", "Fremd zwei", sections: [("A", "Antrag auf Förderung: die Heizung und die Frist werden geprüft.")]))
        await corpus.upsert(assistantDoc("o3", "Fremd drei", sections: [("A", "Heizung Förderung Antrag, Frist Frist.")]))
        await corpus.upsert(assistantDoc("c1", "Mein Eins", area: .notes, sections: [("A", "Notiz: Antrag auf Förderung der Heizung, die Frist habe ich vergessen.")]))
        await corpus.upsert(assistantDoc("c2", "Mein Zwei", area: .notes, sections: [("A", "Heizung: Förderung beantragt, Frist steht noch aus.")]))
        let open = AssistantKnowledge(corpus: corpus)
        let baseline = await retrieve(open, q, limit: 3)
        let scoped = open.withScope(first([(.notes, "c1"), (.notes, "c2")]))

        let three = await retrieve(scoped, q, limit: 3)
        XCTAssertEqual(Set(ids(three).prefix(2)), ["c1", "c2"], "Kontexttreffer stehen vor allen anderen: \(ids(three))")
        XCTAssertEqual(ids(three).last, baseline.first?.itemId, "die beste Stelle außerhalb bleibt, als letzte nach den Kontexten: \(ids(three))")
        XCTAssertEqual(three.count, 3)

        let two = await retrieve(scoped, q, limit: 2)
        XCTAssertEqual(two.count, 2)
        XCTAssertEqual(ids(two).filter { $0 == "c1" || $0 == "c2" }.count, 1, "Kontexte belegen höchstens limit − 1 Plätze: \(ids(two))")
        XCTAssertEqual(ids(two).last, baseline.first?.itemId)

        let five = await retrieve(scoped, q, limit: 5)
        XCTAssertEqual(Set(ids(five)), ["c1", "c2", "o1", "o2", "o3"], "danach mit übrigen Treffern aufgefüllt")
        XCTAssertEqual(Set(ids(five)).count, five.count, "ohne Doppelte")
        XCTAssertEqual(Set(ids(five).prefix(2)), ["c1", "c2"])
    }

    func testFirstFillsAllSlotsWithContextsWhenNothingElseMatches() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("c1", "Mein Eins", sections: [("A", "Der Dachs gräbt seinen Bau unter der Hecke.")]))
        await corpus.upsert(assistantDoc("c2", "Mein Zwei", sections: [("A", "Ein Dachs ist nachts unterwegs.")]))
        await corpus.upsert(assistantDoc("o1", "Fremd", sections: [("A", "Der Ofen heizt das Haus.")]))
        let k = AssistantKnowledge(corpus: corpus).withScope(first([(.books, "c1"), (.books, "c2")]))
        let found = await retrieve(k, "Wo gräbt der Dachs seinen Bau?", limit: 2)
        XCTAssertEqual(Set(ids(found)), ["c1", "c2"], "trifft außerhalb nichts, belegen die Kontexte alle Plätze")
    }

    func testFirstWithoutContextHitsIsExactlyTheUnscopedResult() async {
        let corpus = await fourAreas()
        let open = AssistantKnowledge(corpus: corpus)
        let k = open.withScope(first([(.books, "gibt-es-nicht")]))
        let a = await retrieve(open, tomatoes)
        let b = await retrieve(k, tomatoes)
        XCTAssertEqual(a, b)
        let none = await retrieve(open.withScope(nil), tomatoes)
        XCTAssertEqual(a, none)
    }

    func testFirstKeepsInventoryAndProbeUntouched() async {
        let corpus = await fourAreas()
        let open = AssistantKnowledge(corpus: corpus)
        let k = open.withScope(first([(.books, "books-ctx")]))
        let a = await open.inventoryTitles()
        let b = await k.inventoryTitles()
        XCTAssertEqual(a, b)
    }

    /// Kleine feste Auswertung: 10 Fragen über einen erfundenen Bestand mit ähnlichen Texten. „Treffer“: die erwartete Stelle steht unter
    /// den abgerufenen Auszügen. Mit Kontexten (dem erwarteten Element einiger Fragen) darf die Trefferquote nie unter der ohne liegen.
    func testFirstHitRateIsNeverWorseThanWithoutContexts() async {
        let corpus = AssistantCorpus()
        let shared = "Termin Abnahme Protokoll Anlage Bauleiter Prüfung"
        // Sechs ähnliche Besprechungen, in jeder steht „Abnahme“ und „Termin“; nur eine nennt den Keller.
        for i in 0..<6 {
            let extra = i == 4 ? "Keller Dichtung Feuchte" : "Fassade Gerüst Abbau\(i)"
            await corpus.upsert(assistantDoc("m\(i)", "Baubesprechung \(i)", area: .transcripts, sections: [
                ("00:0\(i):00", "\(shared) \(shared) \(extra) Termin Termin Abnahme"),
                ("00:0\(i):30", "Allgemeines zur Baustelle und zur Abnahme, Termin offen."),
            ]))
        }
        // Vier Bücher und drei Notizen mit eigenem Thema.
        let topics = ["Segeln Wind Boje Anleger", "Pilze Wald Sammeln Körbchen", "Schach Eröffnung Läufer Turm", "Orgel Pfeife Register Balg"]
        for (i, t) in topics.enumerated() {
            await corpus.upsert(assistantDoc("b\(i)", "Buch \(i)", sections: [("S. 1", "\(t). Ein Abschnitt über \(t) mit etwas Termin und Abnahme am Rand.")]))
        }
        let notes = ["Einkauf Milch Brot Käse Butter", "Garten Tomaten Gießen Sonne Beet", "Urlaub Bahn Koffer Hotel Strand"]
        for (i, t) in notes.enumerated() {
            await corpus.upsert(assistantDoc("n\(i)", "Notiz \(i)", area: .notes, sections: [(nil, "\(t) und noch ein Termin")]))
        }
        // (Frage, erwartetes Element)
        let evaluation: [(String, String)] = [
            ("Wann ist der Termin für die Abnahme des Kellers mit der Dichtung?", "m4"),
            ("Was wurde zur Abnahme am Termin über die Feuchte im Keller gesagt?", "m4"),
            ("Welche Boje und welcher Anleger beim Segeln?", "b0"),
            ("Wo wächst der Pilz im Wald und wer sammelt?", "b1"),
            ("Wie war die Eröffnung beim Schach mit dem Läufer?", "b2"),
            ("Wie viele Pfeifen hat das Register der Orgel?", "b3"),
            ("Was muss ich einkaufen, Milch und Käse?", "n0"),
            ("Wann muss ich die Tomaten im Beet gießen?", "n1"),
            ("Welches Hotel am Strand und welche Bahn für den Urlaub?", "n2"),
            ("Welcher Termin steht zur Abnahme mit dem Bauleiter, dem Protokoll und der Anlage?", "m0"),
        ]
        let open = AssistantKnowledge(corpus: corpus)
        let scoped = open.withScope(first([(.transcripts, "m4"), (.books, "b0"), (.notes, "n1"), (.transcripts, "m0")]))
        var without = 0
        var with = 0
        for (question, expected) in evaluation {
            let plain = await retrieve(open, question, limit: 3)
            let ctx = await retrieve(scoped, question, limit: 3)
            if ids(plain).contains(expected) { without += 1 }
            if ids(ctx).contains(expected) { with += 1 }
            XCTAssertEqual(ctx.count > 0, plain.count > 0, "\(question)")
        }
        XCTAssertGreaterThanOrEqual(with, without, "mit Kontexten \(with) von \(evaluation.count), ohne \(without)")
        XCTAssertGreaterThanOrEqual(evaluation.count, 8)
    }

    // MARK: Kein Öffnen

    func testAContextNeverOpensAnAreaThatIsNotReleasedOrAboveTheCeiling() async {
        let corpus = await fourAreas()
        let scope = only([(.transcripts, "transcripts-ctx"), (.books, "books-ctx")])
        let capped = AssistantKnowledge(corpus: corpus, ceiling: [.books]).withScope(scope)
        let a = await retrieve(capped, tomatoes, areas: everything)
        XCTAssertEqual(ids(a), ["books-ctx"], "Obergrenze: Besprechungen bleiben zu")
        // Die Freigabe der Anfrage (Sitzung) ist enger: der Kontext in einem nicht freigegebenen Bereich liefert nichts.
        let open = AssistantKnowledge(corpus: corpus).withScope(scope)
        let b = await retrieve(open, tomatoes, areas: [.books])
        XCTAssertEqual(ids(b), ["books-ctx"])
        let c = await retrieve(open, tomatoes, areas: [.notes])
        XCTAssertEqual(c, [])
        let d = await retrieve(open, tomatoes, areas: [])
        XCTAssertEqual(d, [])
        let titles = await open.inventoryTitles(areas: [.books])
        XCTAssertEqual(titles, ["Kontext books"])
        // Dasselbe bei „zuerst“: die Kontexte öffnen nichts.
        let firstScoped = AssistantKnowledge(corpus: corpus).withScope(AssistantScope(mode: .first, items: scope.items))
        let e = await retrieve(firstScoped, tomatoes, areas: [.books])
        XCTAssertTrue(e.allSatisfy { $0.area == .books })
        XCTAssertTrue(e.contains { $0.itemId == "books-ctx" })
    }

    /// Entzug einer Freigabe mitten im Gespräch: Die Sitzung fragt bei jeder Anfrage mit dem dann gültigen Stand; was entzogen ist,
    /// kommt weder im Abruf noch im Bestand vor, mit und ohne Kontext.
    func testRevokingAnAreaMidConversationDeliversNothingFromIt() async {
        let corpus = await fourAreas()
        for mode in [AssistantScope.Mode.only, .first] {
            let scope = AssistantScope(mode: mode, items: [.init(area: .transcripts, itemId: "transcripts-ctx"), .init(area: .books, itemId: "books-ctx")])
            let k = AssistantKnowledge(corpus: corpus).withScope(scope)
            let before = await retrieve(k, tomatoes, areas: [.books, .transcripts])
            XCTAssertTrue(before.contains { $0.area == .transcripts }, "\(mode)")
            let after = await retrieve(k, tomatoes, areas: [.books])
            XCTAssertFalse(after.contains { $0.area == .transcripts }, "\(mode): \(ids(after))")
            let titles = await k.inventoryTitles(areas: [.books])
            XCTAssertFalse(titles.contains { $0.contains("transcripts") })
            let groups = await k.inventoryGroups(question: "Was habe ich gespeichert?", areas: [.books]).map(\.area)
            XCTAssertFalse(groups.contains(.transcripts))
        }
    }

    func testContentBehindTheReadingFrontierIsNeverFoundEvenAsAContext() async {
        let text = "# Kapitel Eins\n\nDie Eule ruft nachts im Wald von Anfang an.\n\n# Kapitel Zwei\n\nDer geheime Schatz liegt unter der alten Eiche vergraben."
        let blocks = BookTalkIndex.blocks(in: text)
        XCTAssertEqual(blocks.count, 2)
        // Die Lesegrenze liegt hinter dem ersten Absatz: der zweite Absatz steht nicht im Index.
        let sections = AssistantSections.book(blocks, frontier: blocks[0].end)
        XCTAssertEqual(sections.count, 1)
        let corpus = AssistantCorpus()
        await corpus.upsert(AssistantDocument(item: AssistantItem(area: .books, id: "buch", title: "Das Waldbuch"), version: "limit", sections: sections))
        let question = "Wo liegt der geheime Schatz unter der Eiche vergraben?"
        for scope in [only([(.books, "buch")]), first([(.books, "buch")])] {
            let k = AssistantKnowledge(corpus: corpus).withScope(scope)
            let found = await retrieve(k, question)
            XCTAssertFalse(found.contains { $0.text.contains("Schatz") }, "\(scope.mode): hinter der Lesegrenze steht nichts im Index")
        }
        let reachable = await retrieve(AssistantKnowledge(corpus: corpus).withScope(only([(.books, "buch")])), "Wer ruft nachts im Wald, die Eule?")
        XCTAssertEqual(ids(reachable), ["buch"])
    }

    func testASessionWithAnOnlyScopeAnswersOnlyFromTheContext() async {
        let corpus = await fourAreas()
        let knowledge = AssistantKnowledge(corpus: corpus).withScope(only([(.books, "books-ctx")]))
        let session = AssistantSession(model: ContextNoModel(), knowledge: knowledge,
                                       config: AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400,
                                                               userName: nil, timeout: 30, style: .verbatim, roleBlock: ""))
        var seen: [String] = []
        for await event in session.events(AssistantRequest(requestId: "k1", text: tomatoes)) {
            switch event {
            case .sources(_, _, let passages): seen += passages.map(\.itemId)
            case .sentence(_, _, _, _, _, let sources, _): seen += sources.map(\.itemId)
            default: break
            }
        }
        XCTAssertFalse(seen.contains { $0.hasSuffix("-out") }, "\(seen)")
        XCTAssertTrue(seen.contains("books-ctx"), "\(seen)")
    }

    // MARK: Abbildung, Modus, Verfügbarkeit

    func testKindsMapToAreasAndUnsupportedKindsHaveNone() {
        XCTAssertEqual(AssistantScope.area(of: .book), .books)
        XCTAssertEqual(AssistantScope.area(of: .meeting), .transcripts)
        XCTAssertEqual(AssistantScope.area(of: .voiceNote), .notes)
        XCTAssertEqual(AssistantScope.area(of: .file), .files)
        XCTAssertEqual(AssistantScope.area(of: .folder), .files)
        for area in AssistantArea.allCases { XCTAssertEqual(AssistantScope.area(of: AssistantScope.kind(of: area)), area) }
        XCTAssertNil(AssistantScope.area(of: .project))
        XCTAssertNil(AssistantScope.area(of: .section))
        let scope = AssistantScope.make(mode: .only, contexts: [ctx(.book, "b1"), ctx(.meeting, "m1"), ctx(.voiceNote, "n1"), ctx(.file, "f1"),
                                                               ctx(.folder, "ord1"), ctx(.project, "p1"), ctx(.section, "s1")])
        XCTAssertEqual(scope?.items, [.init(area: .books, itemId: "b1"), .init(area: .transcripts, itemId: "m1"),
                                      .init(area: .notes, itemId: "n1"), .init(area: .files, itemId: "f1")])
        XCTAssertEqual(scope?.folders, ["ord1"])
        XCTAssertEqual(scope?.mode, .only)
    }

    func testAnEmptyContextListMakesEveryModeBehaveLikeAll() {
        var p = AssistantProfile(id: "eigen-x", name: "X")
        for mode in [AssistantProfile.Knowledge.Mode.all, .first, .only] {
            p.knowledge.mode = mode
            p.knowledge.contexts = []
            XCTAssertEqual(p.effectiveContextMode, .all)
            XCTAssertNil(AssistantScope.make(mode: p.effectiveContextMode, contexts: p.knowledge.contexts))
            XCTAssertNil(AssistantScope.effective(profile: p, conversation: .empty))
        }
        p.knowledge.mode = .all
        p.knowledge.contexts = [ctx(.book, "b1")]
        XCTAssertNil(AssistantScope.effective(profile: p, conversation: .empty), "Modus alles: Kontexte ohne Wirkung")
    }

    func testTheScopeOfAConversationCombinesAssistantAndConversationContexts() {
        var p = AssistantProfile(id: "eigen-x", name: "X")
        p.knowledge.contexts = [ctx(.book, "b1")]
        var chat = AssistantConversationContexts()
        chat.add(kind: .meeting, id: "m1", title: "Besprechung")
        p.knowledge.mode = .only
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: .empty)?.mode, .only)
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: chat)?.items.count, 2)
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: chat)?.mode, .only)
        p.knowledge.mode = .first
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: chat)?.mode, .first)
        p.knowledge.mode = .all
        XCTAssertNil(AssistantScope.effective(profile: p, conversation: .empty))
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: chat)?.mode, .first, "Modus alles: Gesprächskontexte wirken als zuerst")
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: chat)?.items, [.init(area: .transcripts, itemId: "m1")],
                       "Modus alles: die Kontexte des Assistenten bleiben ohne Wirkung, nur die des Gesprächs zählen")
        // Ohne Assistent („Alles“) ebenso.
        XCTAssertEqual(AssistantScope.effective(profile: nil, conversation: chat)?.mode, .first)
        // Der Schalter setzt „nur“ für dieses Gespräch.
        chat.onlyFromThese = true
        XCTAssertEqual(AssistantScope.effective(profile: p, conversation: chat)?.mode, .only)
        XCTAssertEqual(AssistantScope.effective(profile: nil, conversation: chat)?.mode, .only)
        // Ohne irgendeinen Kontext wirkt auch der Schalter nicht.
        XCTAssertNil(AssistantScope.effective(profile: nil, conversation: AssistantConversationContexts(contexts: [], onlyFromThese: true)))
        // Eine unbekannte Art im Gespräch trägt nichts bei.
        let odd = AssistantConversationContexts(contexts: [.init(art: "zukunft", id: "z", title: "Z")])
        XCTAssertNil(odd.contexts[0].context)
        XCTAssertEqual(AssistantScope.effective(profile: nil, conversation: odd), nil)
    }

    func testAvailabilityHasThreeStates() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Buch Eins", sections: [("S. 1", "Text eins.")]))
        await corpus.upsert(assistantDoc("n1", "Notiz", area: .notes, sections: [(nil, "Text.")]))
        let files = [AssistantItem(area: .files, id: "f1", title: "Datei", folders: ["ord2", "ord1"])]
        let k = AssistantKnowledge(corpus: corpus, sources: [AssistantFakeSource(area: .files, stored: files)])
        let contexts = [ctx(.book, "b1"), ctx(.book, "weg"), ctx(.project, "p1"), ctx(.section, "s1"), ctx(.voiceNote, "n1"),
                        ctx(.file, "f1"), ctx(.folder, "ord1"), ctx(.folder, "leer")]
        let all = await k.availability(of: contexts)
        XCTAssertEqual(all, [.available, .gone, .unsupported, .unsupported, .available, .available, .available, .gone])
        // Bereich nicht wirksam: der Inhalt steht nicht im Index der wirksamen Bereiche.
        let books = await k.availability(of: contexts, areas: [.books])
        XCTAssertEqual(books[0], .available)
        XCTAssertEqual(books[4], .gone)
        XCTAssertEqual(books[2], .unsupported, "Art ohne Abbildung bleibt es, welcher Bereich auch gilt")
        let capped = await k.withCeiling([.notes]).availability(of: contexts)
        XCTAssertEqual(capped[0], .gone)
        XCTAssertEqual(capped[4], .available)
    }

    func testIndexedItemsListEffectiveAreasOnlyAndNotTheScopeFilter() async {
        let corpus = await fourAreas()
        let k = AssistantKnowledge(corpus: corpus).withScope(only([(.books, "books-ctx")]))
        let items = await k.indexedItems(areas: [.books, .notes])
        XCTAssertEqual(Set(items.map(\.id)), ["books-ctx", "books-out", "notes-ctx", "notes-out"], "Auswahlblatt: alles, was nutzbar ist")
    }

    // MARK: Ordner der Ablage

    func testFolderContextsCoverFilesInAndBelowTheFolder() async {
        let chain = AssistantScope.folderChain("c", parents: ["a": String?.none, "b": "a", "c": "b", "z": "y", "y": "z"])
        XCTAssertEqual(chain, ["c", "b", "a"])
        XCTAssertEqual(AssistantScope.folderChain("z", parents: ["z": "y", "y": "z"]), ["z", "y"], "Zyklus bricht ab")
        XCTAssertEqual(AssistantScope.folderChain("x", parents: ["a": String?.none]), [])
        XCTAssertEqual(AssistantScope.folderChain("", parents: ["": String?.none]), [])

        let items = [AssistantItem(area: .files, id: "f1", title: "Plan", folders: ["b", "a"]),
                     AssistantItem(area: .files, id: "f2", title: "Brief", folders: ["a"]),
                     AssistantItem(area: .files, id: "f3", title: "Rechnung", folders: ["q"])]
        let passages = items.map { assistantPassage(.files, itemId: $0.id, title: $0.title, text: "Tomaten Sonne Gießen \($0.title)", whole: false) }
        let source = ContextRecordingSource(area: .files, items: items, passages: passages)
        let below = AssistantKnowledge(corpus: AssistantCorpus(), sources: [source]).withScope(only([], folders: ["b"]))
        let found = await retrieve(below, tomatoes)
        XCTAssertEqual(ids(found), ["f1"], "nur Dateien im Ordner b und darunter")
        let above = AssistantKnowledge(corpus: AssistantCorpus(), sources: [source]).withScope(only([], folders: ["a"]))
        let both = await retrieve(above, tomatoes)
        XCTAssertEqual(Set(ids(both)), ["f1", "f2"])
        let titles = await above.inventoryTitles()
        XCTAssertEqual(Set(titles), ["Plan", "Brief"])
        let scope = AssistantScope.make(mode: .only, contexts: [ctx(.folder, "a")])
        XCTAssertEqual(scope?.folders, ["a"])
        // Eine Notiz mit zufällig gleicher Kennung wie ein Ordner gehört nicht dazu.
        XCTAssertFalse(scope!.admits(AssistantItem(area: .notes, id: "a", title: "Notiz", folders: ["a"])))
    }

    // MARK: Kontexte des Gesprächs

    private func tempDir() -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("lva557-\(UUID().uuidString)", isDirectory: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    func testConversationContextsSurviveARestartAndClearWithTheConversation() throws {
        let dir = tempDir()
        let store = AssistantConversationContextStore(directory: dir)
        XCTAssertEqual(store.load(), .empty)
        var chat = AssistantConversationContexts()
        XCTAssertTrue(chat.add(kind: .book, id: "b1", title: "Das Buch"))
        XCTAssertTrue(chat.add(kind: .voiceNote, id: "n1", title: "Notiz"))
        XCTAssertFalse(chat.add(kind: .book, id: "b1", title: "Doppelt"), "ein Verweis nur einmal")
        XCTAssertFalse(chat.add(kind: .file, id: "", title: "Ohne Kennung"))
        chat.onlyFromThese = true
        try store.save(chat)
        // „Neustart“: ein neues Objekt am selben Ort liest denselben Stand.
        XCTAssertEqual(AssistantConversationContextStore(directory: dir).load(), chat)
        // Neben dem Verlauf, nicht im Profil: eigene Datei im selben Verzeichnis, getrennt vom Verlauf.
        XCTAssertEqual(store.url.lastPathComponent, "kontexte.json")
        XCTAssertNotEqual(store.url, AssistantHistoryStore(directory: dir).url)
        try AssistantHistoryStore(directory: dir).save([])
        XCTAssertEqual(store.load(), chat, "der Verlauf rührt die Kontexte nicht an")
        // Es steht nur Art, Kennung und Titel in der Datei (Verweise), kein Inhalt.
        let raw = try String(contentsOf: store.url, encoding: .utf8)
        XCTAssertTrue(raw.contains("Das Buch") && raw.contains("b1"))
        // Neues Gespräch: Datei weg.
        XCTAssertTrue(store.clear())
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))
        XCTAssertEqual(store.load(), .empty)
        XCTAssertTrue(store.clear(), "nichts da: gelöscht")
        // Ein leerer Stand löscht die Datei.
        try store.save(chat)
        try store.save(.empty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))
    }

    func testConversationContextsAreLimitedAndAFailedDeletionIsReported() throws {
        var chat = AssistantConversationContexts()
        for i in 0..<AssistantConversationContexts.maxCount { XCTAssertTrue(chat.add(kind: .book, id: "b\(i)", title: "B")) }
        XCTAssertFalse(chat.add(kind: .book, id: "zu-viel", title: "B"), "höchstens 50 wie im Schema")
        chat.remove(kind: .book, id: "b3")
        XCTAssertEqual(chat.contexts.count, AssistantConversationContexts.maxCount - 1)
        XCTAssertTrue(chat.add(kind: .book, id: "zu-viel", title: "B"))

        struct Stuck: Error {}
        let dir = tempDir()
        let store = AssistantConversationContextStore(directory: dir, remover: { _ in throw Stuck() })
        try store.save(chat)
        XCTAssertFalse(store.clear(), "bleibt eine Datei, sagt clear es")
        XCTAssertThrowsError(try store.save(.empty))
        // Eine unlesbare Datei ergibt einen leeren Stand und bleibt, bis gespeichert wird.
        try Data("{kaputt".utf8).write(to: store.url)
        XCTAssertEqual(store.load(), .empty)
    }

    func testAnOrphanedContextStaysInTheListAndTheConversationRunsOnWithoutIt() async {
        let corpus = await fourAreas()
        var chat = AssistantConversationContexts()
        chat.add(kind: .book, id: "books-ctx", title: "Kontext books")
        chat.add(kind: .book, id: "geloescht", title: "Gelöschtes Buch")
        let k = AssistantKnowledge(corpus: corpus).withScope(AssistantScope.effective(profile: nil, conversation: chat))
        let states = await k.availability(of: chat.contexts.compactMap(\.context))
        XCTAssertEqual(states, [.available, .gone])
        let found = await retrieve(k, tomatoes)
        XCTAssertEqual(ids(found).first, "books-ctx", "der vorhandene Kontext wirkt, der verwaiste stört nicht")
        XCTAssertEqual(chat.contexts.count, 2, "der Verweis bleibt, bis der Nutzer ihn entfernt")
    }

    func testTheCorpusFilterKeepsScoresAndNeverReturnsOtherItems() async {
        let corpus = await fourAreas()
        let everythingNow = Set(AssistantArea.allCases)
        let all = await corpus.search(tomatoes, previous: nil, areas: everythingNow, limit: 20)
        let key = AssistantScope.Entry(area: .books, itemId: "books-ctx")
        let filtered = await corpus.search(tomatoes, previous: nil, areas: everythingNow, limit: 20, only: [key])
        XCTAssertEqual(filtered.map(\.itemId), ["books-ctx"])
        XCTAssertEqual(filtered.first?.score, all.first { $0.itemId == "books-ctx" }?.score, "derselbe Punktwert mit und ohne Filter")
        let missing = await corpus.search(tomatoes, previous: nil, areas: everythingNow, limit: 20,
                                          only: [AssistantScope.Entry(area: .books, itemId: "gibt-es-nicht")])
        XCTAssertEqual(missing, [])
        let none = await corpus.search(tomatoes, previous: nil, areas: everythingNow, limit: 20, only: [])
        XCTAssertEqual(none, [], "ein leerer Filter ist leer, nicht „alles“")
        // Ein Kontext in einem Bereich, der nicht gewählt ist, liefert nichts.
        let other = await corpus.search(tomatoes, previous: nil, areas: [.notes], limit: 20, only: [key])
        XCTAssertEqual(other, [])
    }

    // MARK: Unverändert ohne Scope

    func testWithoutAScopeEverythingIsAsBefore() async {
        let corpus = await fourAreas()
        let plain = AssistantKnowledge(corpus: corpus)
        XCTAssertNil(plain.scope)
        XCTAssertNil(plain.withAreas(everything).scope)
        let withScope = plain.withScope(only([(.books, "books-ctx")]))
        XCTAssertEqual(withScope.withAreas(everything).scope, withScope.scope, "withAreas und withCeiling behalten den Scope")
        XCTAssertEqual(withScope.withCeiling(nil).scope, withScope.scope)
        XCTAssertEqual(withScope.withCeiling([.books]).ceiling, [.books])
        XCTAssertNil(withScope.withScope(nil).scope)
        let probe = await withScope.probe(sentence: "Tomaten Sonne Gießen im Hochsommer", areas: everything)
        let open = await plain.probe(sentence: "Tomaten Sonne Gießen im Hochsommer", areas: everything)
        XCTAssertEqual(probe, open, "die Prüfung von Sätzen ohne Quelle bleibt über alle freigegebenen Bereiche")
        let merged = AssistantKnowledge.merge([assistantPassage(text: "Eins", score: 2), assistantPassage(itemId: "b2", text: "Zwei", score: 1)], limit: 5, maxChars: 100)
        XCTAssertEqual(merged.map(\.text), ["Eins", "Zwei"])
    }
}
