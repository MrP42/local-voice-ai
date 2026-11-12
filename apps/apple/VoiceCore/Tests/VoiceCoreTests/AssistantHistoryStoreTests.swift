import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Verlaufsdatei. Nur Verzeichnisse unter dem Temp-Ordner des Tests.

final class AssistantHistoryStoreTests: XCTestCase {
    private var dir: URL!

    override func setUp() {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-history-\(UUID().uuidString)", isDirectory: true)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dir)
    }

    private func turn(_ n: Int) -> AssistantTurn {
        AssistantTurn(
            user: "Frage \(n) mit Umlauten äöüß", assistant: "Antwort \(n)", interrupted: n % 2 == 0,
            sources: n % 3 == 0 ? [AssistantPassage(area: .books, itemId: "b\(n)", title: "Buch", locator: "S. \(n)", text: "Text", score: 0.5)] : [],
            general: n % 3 != 0, model: "Apple", at: Date(timeIntervalSince1970: 1_760_000_000 + Double(n)))
    }

    /// So, wie `save` sie schreibt: Quellen ohne Auszugstext und ohne Wertung.
    private func stored(_ n: Int) -> AssistantTurn { AssistantHistoryStore.minimized(turn(n)) }

    /// Harte Abschaltung des freien Modus: Gespeicherte Beiträge aus Runden ohne Quelle werden beim Laden herausgefiltert (die Datei
    /// bleibt dabei unberührt) und beim nächsten Speichern nicht mehr geschrieben.
    func testStoredTurnsWithoutASourceAreFilteredOnLoadAndGoneAfterTheNextSave() throws {
        let store = AssistantHistoryStore(directory: dir)
        let sourced = AssistantTurn(user: "Frage mit Quelle", assistant: "Antwort", sources: [AssistantPassage(area: .books, itemId: "b1", title: "Buch", locator: "S. 1", text: "Text", score: 1)],
                                    model: "Apple", at: Date(timeIntervalSince1970: 1_760_000_001), round: .data).shown
        let free = AssistantTurn(user: "Frage ohne Quelle", assistant: "Allgemeinwissen", general: true, unsourced: true, model: "Apple",
                                 at: Date(timeIntervalSince1970: 1_760_000_002), round: .general).shown
        try store.save([sourced, free])
        let before = try Data(contentsOf: dir.appendingPathComponent(AssistantHistoryStore.fileName))

        let shown = AssistantTurn.admitted(store.load(), released: AssistantUnsourcedConsent.isReleased)
        XCTAssertEqual(shown.map(\.user), ["Frage mit Quelle"], "weder angezeigt noch im Prompt")
        XCTAssertEqual(try Data(contentsOf: dir.appendingPathComponent(AssistantHistoryStore.fileName)), before,
                       "Laden verändert die Datei nicht: vor dem nächsten Speichern geht nichts verloren")
        XCTAssertEqual(store.load().count, 2)

        try store.save(shown)   // der nächste Speichervorgang schreibt, was angezeigt wird
        XCTAssertEqual(store.load().map(\.user), ["Frage mit Quelle"])
        XCTAssertFalse(String(decoding: try Data(contentsOf: dir.appendingPathComponent(AssistantHistoryStore.fileName)), as: UTF8.self).contains("Allgemeinwissen"))
    }

    func testRoundTrip() throws {
        let store = AssistantHistoryStore(directory: dir)
        XCTAssertEqual(store.load(), [])
        XCTAssertFalse(store.hasAny)
        let turns = (0..<5).map(turn)
        try store.save(turns)
        XCTAssertTrue(store.hasAny)
        XCTAssertEqual(store.load(), turns.map(AssistantHistoryStore.minimized))
        XCTAssertEqual(AssistantHistoryStore(directory: dir).load(), turns.map(AssistantHistoryStore.minimized))
        // Alles außer dem Auszugstext der Quellen bleibt erhalten.
        let loaded = store.load()
        XCTAssertEqual(loaded.map(\.user), turns.map(\.user))
        XCTAssertEqual(loaded.map(\.assistant), turns.map(\.assistant))
        XCTAssertEqual(loaded[3].sources.first?.title, "Buch")
        XCTAssertEqual(loaded[3].sources.first?.locator, "S. 3")
        XCTAssertEqual(loaded[3].sources.first?.area, .books)
        XCTAssertEqual(loaded[3].sources.first?.itemId, "b3")
    }

    /// Auszugstexte (Buch, Transkript, Ablage, Notiz) stehen nie in der Verlaufsdatei, auch nicht als Rohtext.
    func testSourceTextNeverReachesTheFile() throws {
        let store = AssistantHistoryStore(directory: dir)
        let secret = "GEHEIMER-AUSZUG-aus-der-Ablage"
        let sources = AssistantArea.allCases.map {
            AssistantPassage(area: $0, itemId: "i-\($0.rawValue)", title: "Titel \($0.rawValue)", locator: "Stelle", text: secret, score: 0.9)
        }
        try store.save([AssistantTurn(user: "Frage", assistant: "Antwort", sources: sources, general: false, model: "Apple")])
        let raw = try String(contentsOf: store.url, encoding: .utf8)
        XCTAssertFalse(raw.contains(secret), raw)
        let loaded = store.load()
        XCTAssertEqual(loaded.first?.sources.count, 4)
        XCTAssertEqual(loaded.first?.sources.map(\.text), Array(repeating: "", count: 4))
        XCTAssertEqual(loaded.first?.sources.map(\.itemId), sources.map(\.itemId))
        XCTAssertEqual(loaded.first?.sources.map(\.area), sources.map(\.area))
    }

    /// Ein Verlauf mit gekürzten Quellen geht ohne Weiteres wieder in eine Sitzung und in die Anzeige.
    func testStoredTurnsWithoutSourceTextStayUsable() async throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save((0..<5).map(turn))
        let loaded = store.load()
        let chips = loaded.flatMap(\.sources).map { "\($0.area.rawValue)|\($0.title)|\($0.locator ?? "")" }
        XCTAssertEqual(chips, ["books|Buch|S. 0", "books|Buch|S. 3"])
        let built = AssistantPrompt.buildData(question: "Weiter?", afterInterruption: false, passages: [], history: loaded,
                                              config: AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5))
        XCTAssertTrue(built.prompt.contains("Antwort 4"), built.prompt)
        XCTAssertTrue(built.used.isEmpty)
    }

    func testKeepsOnlyLast200() throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save((0..<260).map(turn))
        let loaded = store.load()
        XCTAssertEqual(loaded.count, 200)
        XCTAssertEqual(loaded.first, stored(60))
        XCTAssertEqual(loaded.last, stored(259))
    }

    func testEmptySaveAndClearDeleteFile() throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save([turn(1)])
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url.path))
        try store.save([])
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))
        try store.save([turn(1)])
        store.clear()
        XCTAssertEqual(store.load(), [])
        XCTAssertFalse(store.hasAny)
    }

    /// Suchlauf (d): „Verlauf löschen“ meldet, wenn die Datei bleibt (vorher `try?` ohne Rückmeldung: der Nutzer hielt den Verlauf für
    /// gelöscht, und er war beim nächsten Start wieder da). Ein leerer Verlauf, der sich nicht löschen lässt, ist beim Speichern ein
    /// Fehler. Gegen 4d802f3 NICHT AUSDRÜCKBAR (`clear()` gab nichts zurück, es gab keine Einspeisung).
    func testClearReportsWhenTheFileStaysAndAnEmptySaveThenFails() throws {
        struct Refused: Error {}
        let store = AssistantHistoryStore(directory: dir, remover: { _ in throw Refused() })
        try store.save([turn(1)])
        XCTAssertFalse(store.clear(), "die Datei bleibt: nicht gelöscht")
        XCTAssertTrue(store.hasAny)
        XCTAssertThrowsError(try store.save([])) { XCTAssertEqual($0 as? AssistantHistoryStore.StoreError, .deleteFailed) }
        XCTAssertEqual(store.load().count, 1, "nichts ging verloren")
        // Gibt es keine Datei, gilt sie als gelöscht, auch wenn das Löschen nicht ginge.
        let empty = AssistantHistoryStore(directory: dir.appendingPathComponent("leer", isDirectory: true), remover: { _ in throw Refused() })
        XCTAssertTrue(empty.clear())
        // Mit funktionierendem Löschen: true.
        XCTAssertTrue(AssistantHistoryStore(directory: dir).clear())
        XCTAssertFalse(store.hasAny)
    }

    func testCorruptFileIsSetAsideAndNewerVersionIsLeftAlone() throws {
        let store = AssistantHistoryStore(directory: dir)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data("kaputt".utf8).write(to: store.url)
        XCTAssertEqual(store.load(), [])
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.url.appendingPathExtension("defekt").path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.url.path))

        let future = #"{"version":99,"turns":[]}"#
        try Data(future.utf8).write(to: store.url)
        XCTAssertEqual(store.load(), [])
        XCTAssertEqual(try String(contentsOf: store.url, encoding: .utf8), future, "neuere Fassung bleibt unangetastet")
    }

    /// Verlaufsdatei aus der Zeit vor „Antworten ohne Quelle“: keine Felder `unsourced` und `parts`. Sie lädt, nichts geht
    /// verloren; eine frühere Antwort ohne Auszug gilt als „ohne Quelle“, eine belegte nicht.
    func testOldHistoryFileWithoutUnsourcedAndPartsStillLoads() throws {
        let store = AssistantHistoryStore(directory: dir)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let old = #"""
        {"version":1,"turns":[
         {"user":"Wie hoch ist der Mont Blanc","assistant":"Der Mont Blanc ist 4806 Meter hoch.","interrupted":false,"sources":[],"general":true,"model":"Apple","at":782000000.5},
         {"user":"Wohin fährt Castorp","assistant":"Nach Davos.","interrupted":true,"sources":[{"area":"books","itemId":"b1","title":"Der Zauberberg","locator":"S. 12","text":"","score":0}],"general":false,"model":"Gemma","at":782000100}
        ]}
        """#
        try Data(old.utf8).write(to: store.url)
        let loaded = store.load()
        XCTAssertEqual(loaded.count, 2)
        XCTAssertEqual(loaded[0].assistant, "Der Mont Blanc ist 4806 Meter hoch.")
        XCTAssertTrue(loaded[0].general)
        XCTAssertTrue(loaded[0].unsourced, "alte Antwort ohne Auszug: als ohne Quelle gekennzeichnet")
        XCTAssertEqual(loaded[0].parts, [])
        XCTAssertFalse(loaded[1].unsourced)
        XCTAssertEqual(loaded[1].sources.first?.title, "Der Zauberberg")
        XCTAssertTrue(loaded[1].interrupted)
        XCTAssertEqual(loaded[1].model, "Gemma")
        // Wird der Verlauf neu geschrieben, trägt er die neuen Felder und lädt unverändert.
        try store.save(loaded)
        XCTAssertEqual(store.load(), loaded.map(AssistantHistoryStore.minimized))
    }

    func testUnsourcedFlagAndPartsRoundTrip() throws {
        let store = AssistantHistoryStore(directory: dir)
        let mixed = AssistantTurn(
            user: "Frage", assistant: "Castorp fährt nach Davos. Hunde schwitzen über die Pfoten.",
            sources: [AssistantPassage(area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: "GEHEIM", score: 1)],
            general: false, unsourced: true,
            parts: [AssistantTurnPart(text: "Castorp fährt nach Davos.", unsourced: false), AssistantTurnPart(text: "Hunde schwitzen über die Pfoten.", unsourced: true)],
            model: "Apple", at: Date(timeIntervalSince1970: 1_760_000_000))
        try store.save([mixed])
        let loaded = store.load()
        XCTAssertEqual(loaded, [AssistantHistoryStore.minimized(mixed)])
        XCTAssertTrue(loaded[0].unsourced)
        XCTAssertEqual(loaded[0].parts.map(\.unsourced), [false, true])
        let raw = try String(contentsOf: store.url, encoding: .utf8)
        XCTAssertFalse(raw.contains("GEHEIM"))
    }

    /// Verlaufsdatei aus der Zeit vor der Rundenart und den Zitaten: kein `round`, in `parts` kein `quote`. Sie lädt, die
    /// Beiträge gelten als Datenrunde (nie im Prompt einer Allgemeinrunde), und neu geschrieben bleibt alles lesbar.
    func testOldHistoryFileWithoutRoundAndQuoteLoadsAndCountsAsDataRound() throws {
        let store = AssistantHistoryStore(directory: dir)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let old = #"""
        {"version":1,"turns":[
         {"user":"Frage alt","assistant":"Castorp fährt nach Davos. Hunde schwitzen über die Pfoten.","interrupted":false,
          "sources":[{"area":"books","itemId":"b1","title":"Der Zauberberg","locator":"S. 12","text":"","score":0}],"general":false,"unsourced":true,
          "parts":[{"text":"Castorp fährt nach Davos.","unsourced":false},{"text":"Hunde schwitzen über die Pfoten.","unsourced":true}],
          "model":"Apple","at":782000000}
        ]}
        """#
        try Data(old.utf8).write(to: store.url)
        let before = try Data(contentsOf: store.url)
        let loaded = store.load()
        XCTAssertEqual(loaded.count, 1)
        XCTAssertNil(loaded[0].round)
        XCTAssertEqual(loaded[0].parts.map(\.quote), [false, false])
        XCTAssertEqual(loaded[0].parts.map(\.source), [nil, nil], "alte Abschnitte kennen keine Quellzeile")
        XCTAssertEqual(loaded[0].parts.map(\.unsourced), [false, true])
        XCTAssertEqual(try Data(contentsOf: store.url), before, "Laden verändert die Datei nicht")

        let config = AssistantConfig(modelLabel: "T", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
        XCTAssertTrue(AssistantPrompt.buildData(question: "Weiter?", afterInterruption: false, passages: [], history: loaded, config: config)
            .prompt.contains("Frage alt"))
        XCTAssertFalse(AssistantPrompt.buildGeneral(question: "Weiter?", afterInterruption: false, history: loaded, config: config)
            .contains("Frage alt"), "ein Beitrag ohne Rundenart gilt als Datenrunde")

        try store.save(loaded)
        XCTAssertEqual(store.load(), loaded.map(AssistantHistoryStore.minimized))
        XCTAssertNil(store.load()[0].round)
    }

    func testRoundAndQuotePartsRoundTrip() throws {
        let store = AssistantHistoryStore(directory: dir)
        let quote = "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim."
        let turn = AssistantTurn(
            user: "Frage", assistant: quote,
            sources: [AssistantPassage(area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12", text: "GEHEIM", score: 1)],
            general: false, parts: [AssistantTurnPart(text: quote, unsourced: false, quote: true, source: "Buch · Der Zauberberg · S. 12")],
            model: "Apple", at: Date(timeIntervalSince1970: 1_760_000_000), round: .data)
        let general = AssistantTurn(user: "Wie hoch?", assistant: "Der Mont Blanc ist 4806 Meter hoch.", general: true, unsourced: true,
                                    model: "Apple", at: Date(timeIntervalSince1970: 1_760_000_100), round: .general)
        try store.save([turn, general])
        let loaded = store.load()
        XCTAssertEqual(loaded, [AssistantHistoryStore.minimized(turn), general])
        XCTAssertEqual(loaded.map(\.round), [.data, .general])
        XCTAssertEqual(loaded[0].parts, [AssistantTurnPart(text: quote, unsourced: false, quote: true, source: "Buch · Der Zauberberg · S. 12")])
        let raw = try String(contentsOf: store.url, encoding: .utf8)
        XCTAssertFalse(raw.contains("GEHEIM"), "von der Quelle bleibt nur die Anzeigezeile, nie der Auszug")
    }

    func testSourceLabelPerAreaMatchesTheChipFormat() {
        func passage(_ area: AssistantArea, locator: String?) -> AssistantPassage {
            AssistantPassage(area: area, itemId: "i", title: "Titel", locator: locator, text: "x", score: 1)
        }
        XCTAssertEqual(passage(.books, locator: "S. 12").sourceLabel, "Buch · Titel · S. 12")
        XCTAssertEqual(passage(.books, locator: nil).sourceLabel, "Buch · Titel")
        XCTAssertEqual(passage(.transcripts, locator: "00:14:03").sourceLabel, "Transkript · Titel · 00:14:03")
        XCTAssertEqual(passage(.files, locator: "Seite 2").sourceLabel, "Ablage · Titel")
        XCTAssertEqual(passage(.notes, locator: "12.03.2026").sourceLabel, "Notiz · 12.03.2026")
        XCTAssertEqual(passage(.notes, locator: nil).sourceLabel, "Notiz · Titel")
    }

    /// Ein Abbruch mitten im Schreiben lässt die alte Datei ganz: geschrieben wird atomar (temporäre Datei, dann Umbenennen).
    func testSaveLeavesNoTemporaryFilesAndKeepsTheOldFileWhenTheNewOneCannotBeWritten() throws {
        let store = AssistantHistoryStore(directory: dir)
        try store.save([turn(1)])
        let before = try Data(contentsOf: store.url)
        // Der Ordner wird zur Datei: Schreiben scheitert, die alte Datei (im echten Ordner) bleibt, nichts Halbes liegt herum.
        let blocker = FileManager.default.temporaryDirectory.appendingPathComponent("assistant-history-file-\(UUID().uuidString)")
        try Data("x".utf8).write(to: blocker)
        defer { try? FileManager.default.removeItem(at: blocker) }
        XCTAssertThrowsError(try AssistantHistoryStore(directory: blocker.appendingPathComponent("unter")).save([turn(2)]))
        XCTAssertEqual(try Data(contentsOf: store.url), before)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path), [AssistantHistoryStore.fileName])
    }

    // MARK: Marke je Ausgabeeinheit (Abschnitte mit Quellzeile oder Kennzeichnung)

    /// Ein Abschnitt trägt nach Speichern und Laden Quellzeile oder Kennzeichnung; Beiträge ohne das Feld `note` laden.
    func testPartsKeepTheirSourceLineAndNoteAndOldPartsWithoutNoteLoad() throws {
        let store = AssistantHistoryStore(directory: dir)
        let parts = [
            AssistantTurnPart(text: "Anna plant.", unsourced: false, quote: true, source: "Buch · Beispielbuch · S. 12"),
            AssistantTurnPart(text: "Dazu finde ich nichts.", unsourced: false, note: AssistantTurnPart.templateNote),
            AssistantTurnPart(text: "Du hast zwei Bücher.", unsourced: false, note: AssistantTurnPart.inventoryNote),
        ]
        try store.save([AssistantTurn(user: "Frage", assistant: "x", parts: parts, round: .data)])
        XCTAssertEqual(store.load().first?.parts, parts)
        let old = #"{"version":1,"turns":[{"user":"a","assistant":"b","interrupted":false,"sources":[],"general":false,"parts":[{"text":"t","unsourced":false,"quote":true,"source":"Notiz · Titel"}],"model":"","at":782000000}]}"#
        try Data(old.utf8).write(to: store.url)
        let loaded = store.load()
        XCTAssertEqual(loaded.first?.parts.first?.note, nil)
        XCTAssertEqual(loaded.first?.parts.first?.source, "Notiz · Titel")
    }

    /// Die gemeinsame Zuordnung (Anzeige der laufenden Antwort UND Beitrag): Zitate je ein Abschnitt mit genau einer Quellzeile, Sätze
    /// gleicher Kennzeichnung zusammen, Vorlage und Bestandsangabe nie in einem Abschnitt.
    func testAppendGivesEachQuoteItsOwnPartWithOneSourceLineAndMergesOnlyEqualMarks() {
        func passage(_ title: String, _ locator: String) -> AssistantPassage {
            AssistantPassage(area: .books, itemId: title, title: title, locator: locator, text: "x", score: 1)
        }
        var list: [AssistantTurnPart] = []
        AssistantTurnPart.append(to: &list, text: "Eins.", status: .grounded, quote: true, sources: [passage("A", "S. 1")])
        AssistantTurnPart.append(to: &list, text: "Zwei.", status: .grounded, quote: true, sources: [passage("A", "S. 1")])
        AssistantTurnPart.append(to: &list, text: "Drei.", status: .grounded, quote: true, sources: [passage("B", "S. 2")])
        AssistantTurnPart.append(to: &list, text: "Vorlage eins.", status: .neutral, quote: false, sources: [])
        AssistantTurnPart.append(to: &list, text: "Vorlage zwei.", status: .neutral, quote: false, sources: [])
        AssistantTurnPart.append(to: &list, text: "Bestand.", status: .inventory, quote: false, sources: [])
        XCTAssertEqual(list.map(\.text), ["Eins.", "Zwei.", "Drei.", "Vorlage eins. Vorlage zwei.", "Bestand."])
        XCTAssertEqual(list.map(\.quote), [true, true, true, false, false])
        XCTAssertEqual(list.prefix(3).map(\.source), ["Wörtlicher Auszug aus A · S. 1", "Wörtlicher Auszug aus A · S. 1", "Wörtlicher Auszug aus B · S. 2"])
        XCTAssertEqual(list.suffix(2).map(\.note), [AssistantTurnPart.templateNote, AssistantTurnPart.inventoryNote])
        XCTAssertEqual(list.suffix(2).map(\.source), [nil, nil])
    }

    func testStandardDirectoryIsSeparateFromBookTalk() {
        XCTAssertEqual(AssistantHistoryStore.standardDirectory().lastPathComponent, "Assistant")
        XCTAssertNotEqual(AssistantHistoryStore.standardDirectory(), BookTalkHistoryStore.standardDirectory())
    }
}
