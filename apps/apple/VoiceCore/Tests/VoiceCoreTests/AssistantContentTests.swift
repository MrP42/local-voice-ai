import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2b, Klasse B und Quellarten):
// - Klasse B: Inhalte und Schutzregeln der Quellen. `contentWillChange()` erhöht die Generation synchron im selben Lock wie
//   Klasse A und stellt den Bestand auf `stale`; `install` bildet ihn neu und meldet ihn. Eine Anfrage liest Generation, Stand und
//   Bestand in EINEM Schritt; bei `stale` kommt nichts aus eigenen Inhalten. Die Zeile mit Ursache „Inhalt“ wird nur eingereiht.
// - Quellarten: zitierbar sind nur Buch, getippte Notiz, importiertes Dokument und Transkript. Erzeugter Text (Protokoll einer
//   Sprachnotiz oder Besprechung, Antworten der App), Mail und Kalender sind es nie; das Tor steht in der Auswahl UND in der
//   Prüfung vor der Ausgabe. Nur synthetische Daten.

private struct EchoModel: BookTalkModel {
    var text: String
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let text = text
        return AsyncThrowingStream { c in
            c.yield(text)
            c.finish()
        }
    }
}

/// Wartepunkt für eine Quelle im Abruf: `arrive` meldet das Eintreffen und wartet, bis `open` ruft (ohne Thread zu blockieren).
private actor Gate {
    private var arrived = false
    private var opened = false
    private var arrivedWaiters: [CheckedContinuation<Void, Never>] = []
    private var openWaiters: [CheckedContinuation<Void, Never>] = []
    private(set) var calls = 0

    func arrive() async {
        calls += 1
        arrived = true
        for w in arrivedWaiters { w.resume() }
        arrivedWaiters = []
        if opened { return }
        await withCheckedContinuation { openWaiters.append($0) }
    }

    func waitUntilArrived() async {
        if arrived { return }
        await withCheckedContinuation { arrivedWaiters.append($0) }
    }

    func open() {
        opened = true
        for w in openWaiters { w.resume() }
        openWaiters = []
    }
}

private struct GatedSource: AssistantSource {
    let area: AssistantArea
    let gate: Gate
    func items() async -> [AssistantItem] { [] }
    func search(_ question: String, limit: Int) async -> [AssistantPassage] {
        await gate.arrive()
        return []
    }
}

/// Quelle, die einen Auszug mit festem Kennzeichen liefert: ein „Block“, dessen Art nicht zitierbar ist, und ein zitierbarer.
private struct FixedSource: AssistantSource {
    let area: AssistantArea
    let passages: [AssistantPassage]
    func items() async -> [AssistantItem] { [] }
    func search(_ question: String, limit: Int) async -> [AssistantPassage] { passages }
}

final class AssistantContentTests: XCTestCase {
    private var dir: URL!
    private let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
    private let block = "Der Eisvogel brütet am Mühlbach in der Uferböschung unter dem Weidenbaum."
    private let question = "Wo brütet der Eisvogel am Mühlbach?"
    private let answer = "Der Eisvogel brütet am Mühlbach."

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-content-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    // MARK: Hilfen

    private func area(of kind: AssistantSourceKind) -> AssistantArea {
        switch kind {
        case .book: .books
        case .typedNote: .notes
        case .importedDocument, .mail, .calendar: .files
        case .transcript, .generated: .transcripts
        }
    }

    private func make(_ knowledge: AssistantKnowledge, audit: AssistantAuditLog? = nil, store: AssistantReleaseStore? = nil) -> AssistantSession {
        AssistantSession(model: EchoModel(text: answer), knowledge: knowledge, config: config, audit: audit, released: true, releaseStore: store)
    }

    private func units(_ s: AssistantSession, _ id: String) async -> [(text: String, quote: Bool, status: AssistantSentenceStatus)] {
        var out: [(String, Bool, AssistantSentenceStatus)] = []
        for await e in s.events(AssistantRequest(requestId: id, text: question)) {
            if case .sentence(_, _, let text, _, let status, _, let quote) = e { out.append((text, quote, status)) }
        }
        _ = await s.commit(requestId: id, user: "x", spoken: "", interrupted: true)
        return out
    }

    private func ownContent(_ units: [(text: String, quote: Bool, status: AssistantSentenceStatus)]) -> Bool {
        units.contains { $0.text.contains("Eisvogel") }
    }

    private func knowledge(kind: AssistantSourceKind, text: String? = nil) async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("d1", "Beobachtungen Mühlbach", area: area(of: kind), kind: kind, sections: [(nil, text ?? block)]))
        return AssistantKnowledge(corpus: corpus)
    }

    // MARK: Quellarten

    func testOnlyBookTypedNoteImportedDocumentAndTranscriptAreQuotable() {
        let quotable = AssistantSourceKind.allCases.filter(\.isQuotable)
        XCTAssertEqual(Set(quotable), [.book, .typedNote, .importedDocument, .transcript])
        for kind in AssistantSourceKind.allCases {
            XCTAssertEqual(AssistantDocument.Section(locator: nil, text: "Text", kind: kind, raw: "Text").quotable, quotable.contains(kind), "\(kind)")
            XCTAssertFalse(AssistantDocument.Section(locator: nil, text: "Text", kind: kind).quotable, "\(kind): ohne Rohtext nie zitierbar")
        }
        XCTAssertFalse(AssistantSourceKind.generated.isQuotable)
        XCTAssertFalse(AssistantSourceKind.mail.isQuotable)
        XCTAssertFalse(AssistantSourceKind.calendar.isQuotable)
    }

    /// Die Bausteine der Quellen legen die Art fest: Notiztext getippt, Antwort der App und Protokoll erzeugt, Segment Transkript,
    /// Absatz Buch. Eine Quelle, die nichts angibt, kann nicht übersetzen (die Art hat keinen Vorgabewert).
    func testTheBuildersOfTheSourcesSetTheKindOfEveryBlock() {
        XCTAssertEqual(AssistantSections.blocks(of: "Absatz eins.\n\nAbsatz zwei.", locator: nil).map(\.kind), [.typedNote, .typedNote])
        XCTAssertEqual(AssistantSections.generated(["Antwort der App: Hallo"], locator: nil).map(\.kind), [.generated])
        XCTAssertEqual(AssistantSections.meeting([AssistantSegment(start: 1, line: "A: Hallo", raw: "Hallo", usable: true)], locator: { "\($0)" }).map(\.kind), [.transcript])
        XCTAssertEqual(AssistantSections.generated(["Zusammenfassung: x"], locator: "Protokoll").map(\.quotable), [false])
    }

    /// Je Quellart durch die ganze Sitzung: Das Zitat kommt nur bei den vier zitierbaren Arten; bei erzeugtem Text, Mail und Kalender
    /// steht der Text nirgends in der Ausgabe, auch wenn das Modell ihn zeichengleich liefert.
    func testEachSourceKindIsQuotedOnlyWhenItsKindAllowsIt() async {
        for kind in AssistantSourceKind.allCases {
            let s = make(await knowledge(kind: kind))
            let out = await units(s, "r-\(kind.rawValue)")
            if kind.isQuotable {
                XCTAssertTrue(out.contains { $0.quote && $0.text == block }, "\(kind): zitierbar, aber nicht ausgegeben: \(out.map(\.text))")
            } else {
                XCTAssertFalse(ownContent(out), "\(kind): nicht zitierbar, aber ausgegeben: \(out.map(\.text))")
                XCTAssertFalse(out.contains { $0.quote }, "\(kind)")
            }
        }
    }

    /// Das Tor in der Auswahl: Auszüge nicht zitierbarer Art kommen aus dem Abruf, steigen aber nie in die Zitierbasis auf.
    func testTheSelectionNeverTakesAPassageOfANonQuotableKind() async {
        for kind in AssistantSourceKind.allCases {
            let corpus = AssistantCorpus()
            await corpus.upsert(assistantDoc("d1", "Beobachtungen Mühlbach", area: area(of: kind), kind: kind, sections: [(nil, block)]))
            let found = await corpus.search("Eisvogel Mühlbach", areas: Set(AssistantArea.allCases), limit: 5)
            XCTAssertEqual(found.count, 1, "\(kind): der Auszug ist auffindbar (Abruf, Erkennung)")
            XCTAssertEqual(found.first?.quotable, kind.isQuotable, "\(kind)")
            XCTAssertEqual(AssistantGrounding.Basis(passages: found).isEmpty, !kind.isQuotable, "\(kind)")
        }
    }

    /// Das Tor vor der Ausgabe ist unabhängig von der Auswahl: Es lässt nur ganze Blöcke zitierbarer Art zeichengleich durch.
    func testThePreOutputCheckAdmitsOnlyAWholeQuotableBlockWithTheStoredText() {
        func passage(text: String, isBlock: Bool = true, quotable: Bool = true) -> AssistantPassage {
            AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: text, score: 1, isBlock: isBlock, quotable: quotable, raw: text)
        }
        func check(_ q: AssistantPassage, text: String? = nil) -> Bool {
            AssistantGrounding.mayOutput(AssistantGrounding.SourceBlock(text: text ?? q.text, passage: q))
        }
        XCTAssertTrue(check(passage(text: block)))
        XCTAssertFalse(check(passage(text: block, quotable: false)), "nicht zitierbare Art")
        XCTAssertFalse(check(passage(text: block, isBlock: false)), "kein ganzer Block (Fenster, gekürzt)")
        XCTAssertFalse(check(passage(text: block), text: "Der Eisvogel brütet am Mühlbach."), "nicht zeichengleich mit dem gespeicherten Text")
        XCTAssertFalse(check(passage(text: String(repeating: "Wort ", count: 200))), "über der Höchstlänge")
        XCTAssertFalse(check(passage(text: "   ")), "leer")
        // B1: Die Prüfung vergleicht die Bytefolge mit dem ROHEN Block (nicht Zeichenfolgen kanonisch, nicht den aufbereiteten Text).
        let nfc = "Der Eisvogel brütet am Mühlbach."
        let nfd = "Der Eisvogel bru\u{0308}tet am Mu\u{0308}hlbach."
        XCTAssertEqual(nfc, nfd, "kanonisch gleich ...")
        let stored = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: nfc, score: 1, isBlock: true, quotable: true, raw: nfd)
        XCTAssertTrue(check(stored, text: nfd), "die Bytes des Gespeicherten")
        XCTAssertFalse(check(stored, text: nfc), "... aber NFC statt der gespeicherten NFD-Bytes besteht nicht")
        let cr = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: "Alpha\nBeta", score: 1, isBlock: true, quotable: true, raw: "Alpha\rBeta")
        XCTAssertTrue(check(cr, text: "Alpha\rBeta"))
        XCTAssertFalse(check(cr, text: "Alpha\nBeta"), "der aufbereitete Text ist nie die Ausgabe")
        XCTAssertFalse(check(AssistantPassage(area: .notes, itemId: "n", title: "T", text: block, score: 1, isBlock: true, quotable: true), text: block),
                       "ohne Rohtext nie zitierbar, auch nicht in der aufbereiteten Fassung")
        XCTAssertFalse(check(AssistantPassage(area: .notes, itemId: "n", title: "T", text: "x", score: 1, isBlock: true, quotable: true,
                                              raw: " \r\n\t "), text: " \r\n\t "), "nur Leerraum")
        XCTAssertFalse(check(AssistantPassage(area: .notes, itemId: "n", title: "T", text: "x", score: 1, isBlock: true, quotable: true,
                                              raw: String(repeating: "ab\r\n", count: 151)), text: String(repeating: "ab\r\n", count: 151)),
                       "über der Höchstlänge am Rohtext gemessen (604 Einheiten)")
        XCTAssertFalse(AssistantPassage(area: .notes, itemId: "n", title: "T", text: block, score: 1, isBlock: true).quotable, "ohne Angabe nie zitierbar")
        XCTAssertFalse(passage(text: block, quotable: true).clipped(to: 20).isBlock, "ein gekürzter Text ist kein Block mehr")
        XCTAssertTrue(passage(text: block, quotable: true).replacingScoreKeepsFlags)
    }

    /// Eine Quelle, die einen Auszug als Block ausgibt, aber nicht als zitierbar kennzeichnet, kommt nie wörtlich hinaus; mit der Kennzeichnung schon.
    func testASourcePassageClaimedAsBlockButNotQuotableIsNeverOutputAndTheControlIs() async {
        for quotable in [false, true] {
            let passage = AssistantPassage(area: .files, itemId: "f1", title: "Ablagedatei Eisvogel", text: block, score: 1, isBlock: true, quotable: quotable,
                                           raw: block)
            let knowledge = AssistantKnowledge(corpus: AssistantCorpus(), sources: [FixedSource(area: .files, passages: [passage])])
            await knowledge.setAreas([.files])
            let out = await units(make(knowledge), "r\(quotable)")
            XCTAssertEqual(ownContent(out), quotable, "quotable=\(quotable): \(out.map(\.text))")
        }
    }

    // MARK: Klasse B: stale, ready, Generation

    func testContentWillChangeRaisesTheGenerationAtOnceMarksTheCorpusStaleAndInstallMakesItReadyAgain() async {
        let s = make(await knowledge(kind: .typedNote))
        let g = s.releaseGeneration
        XCTAssertTrue(s.isContentReady)
        let epoch = s.contentWillChange()
        XCTAssertEqual(s.releaseGeneration, g + 1, "synchron, ohne Warten")
        XCTAssertFalse(s.isContentReady)
        XCTAssertTrue(s.contentInstalled(epoch: epoch))
        XCTAssertTrue(s.isContentReady)
        XCTAssertEqual(s.releaseGeneration, g + 1, "der neue Bestand erhöht nicht noch einmal")
    }

    func testAStaleCorpusOutputsNothingFromOwnContentUntilTheNewStockIsInstalled() async {
        let corpus = AssistantCorpus()
        let old = assistantDoc("n1", "Notiz Mühlbach", area: .notes, sections: [(nil, block)])
        await corpus.upsert(old)
        let s = make(AssistantKnowledge(corpus: corpus))
        let before = await units(s, "a")
        XCTAssertTrue(ownContent(before), "Gegenprobe: vor der Änderung kommt das Zitat")

        let epoch = s.contentWillChange()
        let during = await units(s, "b")
        XCTAssertFalse(ownContent(during), "stale: nichts aus eigenen Inhalten: \(during.map(\.text))")

        // Der neue Bestand: die Notiz wurde geändert (anderer Text), eine zweite fiel weg.
        let changed = "Der Eisvogel brütet jetzt im Steilufer am Weiher bei der Mühle."
        let ready = await s.install([assistantDoc("n1", "Notiz Mühlbach", area: .notes, version: "2", sections: [(nil, changed)])], epoch: epoch)
        XCTAssertTrue(ready)
        let after = await units(s, "c")
        XCTAssertTrue(after.contains { $0.quote && $0.text == changed }, "der neue Bestand gilt: \(after.map(\.text))")
        XCTAssertFalse(after.contains { $0.text == block }, "der alte Text ist ersetzt, nicht ergänzt")
    }

    func testInstallReplacesTheStockExactlyAndDropsWhatIsNoLongerThere() async {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("a", "A", area: .notes, sections: [(nil, "Alpha Text.")]))
        await corpus.upsert(assistantDoc("b", "B", area: .notes, sections: [(nil, "Beta Text.")]))
        await corpus.upsert(assistantDoc("c", "C", area: .books, sections: [(nil, "Gamma Text.")]))
        await corpus.install([assistantDoc("b", "B", area: .notes, sections: [(nil, "Beta Text.")]),
                              assistantDoc("d", "D", area: .transcripts, sections: [(nil, "Delta Text.")])])
        let ids = await corpus.items(area: nil).map { "\($0.area.rawValue)/\($0.id)" }
        XCTAssertEqual(Set(ids), ["notes/b", "transcripts/d"])
        let found = await corpus.search("Alpha Gamma", areas: Set(AssistantArea.allCases), limit: 5)
        XCTAssertTrue(found.isEmpty)
    }

    /// Kommt der Bestand einer älteren Änderung an, nachdem schon eine neuere begann, bleibt der Bestand `stale`.
    func testAnInstallForAnOlderChangeKeepsTheCorpusStaleWhileANewerOneIsOutstanding() async {
        let s = make(await knowledge(kind: .book))
        let first = s.contentWillChange()
        let second = s.contentWillChange()
        XCTAssertNotEqual(first, second)
        XCTAssertFalse(s.contentInstalled(epoch: first))
        XCTAssertFalse(s.isContentReady)
        XCTAssertTrue(s.contentInstalled(epoch: second))
        XCTAssertTrue(s.isContentReady)
    }

    /// Die Zeile mit Ursache „Inhalt“ wird nur eingereiht: Der Aufruf kehrt auch bei langsamer Platte sofort zurück; die Zeile trägt
    /// Lauf, Generation und Ursache, nichts von Titel oder Inhalt.
    func testTheContentLineIsOnlyEnqueuedNeverAwaitedAndCarriesNoContent() async throws {
        let audit = AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile, appender: { url, data in
            Thread.sleep(forTimeInterval: 0.25)
            try AssistantAuditWriter.appendFile(url, data)
        })
        let s = make(await knowledge(kind: .typedNote), audit: audit)
        let started = Date()
        for _ in 0..<4 { s.contentWillChange() }
        XCTAssertLessThan(Date().timeIntervalSince(started), 0.2, "kein Warten auf die Platte")
        XCTAssertEqual(s.releaseGeneration, 4)
        _ = await audit.flushReleaseLines()
        let raw = (try? String(contentsOf: audit.url, encoding: .utf8)) ?? ""
        let lines = raw.split(separator: "\n").compactMap { (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any] }
        let released = lines.filter { $0["t"] as? String == "release" }
        XCTAssertEqual(released.count, 4)
        XCTAssertEqual(released.compactMap { $0["cause"] as? String }, Array(repeating: "content", count: 4))
        XCTAssertEqual(released.compactMap { $0["gen"] as? Int }, [1, 2, 3, 4])
        XCTAssertTrue(released.allSatisfy { $0["run"] is Int }, "mit Lauf")
        XCTAssertFalse(raw.contains("Eisvogel"))
        XCTAssertFalse(raw.contains("Mühlbach"))
        let entries = await audit.entries().filter { $0.kind == .release }
        XCTAssertEqual(entries.map(\.cause), Array(repeating: .content, count: 4))
    }

    /// Haltepunkt (Stand und Bestand in EINEM Schritt gelesen): Die Anfrage steckt im Abruf, mitten darin ändert sich ein Inhalt. Sie
    /// ist widerrufen und gibt nichts aus. Die nächste Anfrage (Bestand noch `stale`) gibt ebenfalls nichts aus; nach `install` kommt
    /// wieder Inhalt, mit der neuen Generation.
    func testAContentChangeDuringTheRetrievalRevokesTheRunAndAStaleStockOutputsNothing() async {
        let gate = Gate()
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("n1", "Notiz Mühlbach", area: .notes, sections: [(nil, block)]))
        let knowledge = AssistantKnowledge(corpus: corpus, sources: [GatedSource(area: .files, gate: gate)])
        await knowledge.setAreas([.notes, .files])
        let s = make(knowledge)
        let g = s.releaseGeneration
        let text = question
        let first = Task { () -> [AssistantEvent] in
            var out: [AssistantEvent] = []
            for await e in s.events(AssistantRequest(requestId: "r1", text: text)) { out.append(e) }
            return out
        }
        await gate.waitUntilArrived()   // die Anfrage hat (g, Bereiche, Bestand) gelesen und steckt im Abruf der Ablage
        let epoch = s.contentWillChange()
        XCTAssertEqual(s.releaseGeneration, g + 1)
        await gate.open()
        let events = await first.value
        XCTAssertFalse(events.contains { if case .sentence = $0 { true } else { false } }, "nach der Änderung kommt nichts mehr hinaus")
        for e in events { if case .sources(_, let gen, _) = e { XCTAssertEqual(gen, g, "nie die neue Generation mit dem alten Bestand") } }
        _ = await s.commit(requestId: "r1", user: "x", spoken: "", interrupted: true)

        let stale = await units(s, "r2")
        XCTAssertFalse(ownContent(stale), "stale: \(stale.map(\.text))")
        let calls = await gate.calls
        XCTAssertEqual(calls, 1, "die Quelle der Ablage wird bei stale gar nicht erst abgerufen")
        XCTAssertTrue(s.contentInstalled(epoch: epoch))
        let ready = await units(s, "r3")
        XCTAssertTrue(ownContent(ready), "bereit: wieder Inhalt")
    }

    /// Generation und Bestandszustand werden in einem Schritt gelesen: Kein Lesen zeigt eine Generation, die zum Zustand des
    /// Bestands nicht passt (bei reinen Inhaltsänderungen ist die Generation genau die Zahl der Änderungen), auch bei gleichzeitigen
    /// Änderungen und Meldungen.
    func testGenerationAndCorpusStateAreReadInOneStepEvenWhileTheyChange() {
        let release = AssistantRelease(optIn: false)
        let stop = Flag()
        let bad = Flag()
        let readers = DispatchGroup()
        for _ in 0..<2 {
            readers.enter()
            DispatchQueue.global().async {
                while !stop.value {
                    let snap = release.snapshot
                    if snap.generation != snap.contentEpoch { bad.value = true }
                    if snap.corpusReady, snap.contentEpoch != snap.generation { bad.value = true }
                }
                readers.leave()
            }
        }
        let writers = DispatchGroup()
        for _ in 0..<3 {
            writers.enter()
            DispatchQueue.global().async {
                for _ in 0..<300 {
                    let change = release.contentWillChange()
                    release.contentInstalled(epoch: change.epoch)
                }
                writers.leave()
            }
        }
        XCTAssertEqual(writers.wait(timeout: .now() + 20), .success)
        stop.value = true
        XCTAssertEqual(readers.wait(timeout: .now() + 20), .success)
        XCTAssertFalse(bad.value, "eine Generation, die nicht zum Bestandszustand passt, wurde gelesen")
        XCTAssertEqual(release.snapshot.generation, 900)
        XCTAssertEqual(release.snapshot.contentEpoch, 900)
    }

    /// Gemeinsame Ordnung: Änderungen der Klasse A und B aus mehreren Threads erhalten verschiedene Generationen ohne Lücke; jede
    /// steht in genau einer Zeile (`release`, bei Erweiterungen `effective`).
    func testClassAAndClassBChangesShareOneOrderOfGenerationsWithOneLineEach() async throws {
        let audit = AssistantAuditLog(directory: dir, reader: AssistantAuditWriter.readFile)
        let s = make(await knowledge(kind: .book), audit: audit)
        let collected = NSLock()
        nonisolated(unsafe) var pendings: [AssistantReleasePending] = []
        let group = DispatchGroup()
        for t in 0..<4 {
            group.enter()
            DispatchQueue.global().async {
                for i in 0..<50 {
                    if (t + i) % 2 == 0 {
                        s.contentWillChange()
                    } else {
                        let p = s.applyReleaseChange()
                        collected.withLock { pendings.append(p) }
                    }
                }
                group.leave()
            }
        }
        XCTAssertEqual(group.wait(timeout: .now() + 20), .success)
        let all = collected.withLock { pendings }
        // Die Sicherungsfolgen laufen streng in der Reihenfolge der Vergabe; `settle` wartet auf seine Reihe. Die Sammelliste hat
        // nicht dieselbe Reihenfolge wie die Vergabe (zwei Threads), also nebenläufig abarbeiten statt nacheinander in Listenfolge.
        await withTaskGroup(of: Void.self) { tasks in
            for p in all { tasks.addTask { _ = await s.settle(p) } }
        }
        XCTAssertEqual(s.releaseGeneration, 200)
        _ = await audit.flushReleaseLines()
        let raw = (try? String(contentsOf: audit.url, encoding: .utf8)) ?? ""
        let generations = raw.split(separator: "\n").compactMap { (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any] }
            .filter { ["release", "effective"].contains($0["t"] as? String ?? "") }.compactMap { $0["gen"] as? Int }
        XCTAssertEqual(generations.sorted(), Array(1...200), "jede Generation genau einmal, ohne Lücke")
    }
}

private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var on = false
    var value: Bool {
        get { lock.withLock { on } }
        set { lock.withLock { on = newValue } }
    }
}

private extension AssistantPassage {
    /// `replacing(score:)` behält Block und Zitierbarkeit, solange der Text derselbe bleibt.
    var replacingScoreKeepsFlags: Bool {
        let same = replacing(text: text, score: 2)
        return same.isBlock == isBlock && same.quotable == quotable && same.raw == raw
    }
}
