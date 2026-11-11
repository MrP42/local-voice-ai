import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Satzprüfung vor der Ausgabe, strukturell. Goldfälle auf Deutsch; nur
// synthetische Inhalte. In Datenrunden ist jeder ausgegebene Satz zeichengleich mit einem Quellsatz oder einer Systemvorlage.

final class AssistantGroundingTests: XCTestCase {
    private let book = AssistantPassage(
        area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12",
        text: "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim. Er bleibt drei Wochen.", score: 1,
        startsAtBoundary: true, endsAtBoundary: true)
    private let meeting = AssistantPassage(
        area: .transcripts, itemId: "t1", title: "Besprechung vom 12.10.2026", locator: "00:14:03",
        text: "Die Besprechung dauerte 48 Minuten. Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.", score: 0.8,
        startsAtBoundary: true, endsAtBoundary: true)
    private let titles = ["Der Zauberberg", "Besprechung vom 12.10.2026", "Quartalsplanung"]
    private let now = Date(timeIntervalSince1970: 1_794_384_300)   // Mittwoch, 11. November 2026, 08:05 Uhr (UTC)
    private var zone: TimeZone { TimeZone(identifier: "UTC")! }
    private var clock: [String] { AssistantGrounding.clockSentences(now: now, timeZone: zone) }

    private func judge(_ s: String, round: AssistantRoundKind = .data, passages: [AssistantPassage]? = nil, optIn: Bool = false,
                       taken: Set<String> = []) -> AssistantGrounding.Verdict {
        AssistantGrounding.judge(s, round: round, basis: .init(passages: passages ?? [book, meeting]), titles: titles, clock: clock,
                                 optIn: optIn, taken: taken)
    }

    private func quoted(_ s: String, passages: [AssistantPassage]? = nil, optIn: Bool = false) -> String? {
        if case .quote(let q) = judge(s, passages: passages, optIn: optIn) { return q.text }
        return nil
    }

    // MARK: Einrasten (Goldfälle der Entscheidung)

    func testModelSentenceSnapsToTheSourceSentenceNeverItself() {
        let single = assistantPassage(.notes, itemId: "n1", title: "Notiz", text: "Hans besucht Joachim.")
        let model = "Joachim besucht seinen Vetter Hans in Davos."
        XCTAssertEqual(quoted(model, passages: [single]), "Hans besucht Joachim.", "gesprochen wird nur der Quellsatz")
        XCTAssertNotEqual(quoted(model, passages: [single]), model)
    }

    /// Die Ausnahme „ganzer Auszug ohne Satzzeichen“ gibt es nicht mehr: zitierbar ist nur, was mit Satzzeichen endet.
    func testSourceWithoutFinalPunctuationIsNeverQuotableEvenAsTheWholePassage() {
        let bare = assistantPassage(.notes, itemId: "n1", title: "Notiz", text: "Hans besucht Joachim")
        XCTAssertNil(quoted("Joachim besucht seinen Vetter Hans in Davos.", passages: [bare]))
        XCTAssertTrue(AssistantGrounding.Basis(passages: [bare]).isEmpty)
        let fragment = assistantPassage(.notes, itemId: "n2", title: "Projekt", text: "Anna für Projekt Nordlicht")
        XCTAssertTrue(AssistantGrounding.Basis(passages: [fragment]).isEmpty, "ein ganzer Auszug ohne Satzende ist nicht zitierbar")
        XCTAssertNil(quoted("Anna übernimmt die Leitung für das Projekt Nordlicht.", passages: [fragment]))
        XCTAssertNil(AssistantGrounding.fallback(question: "Wer macht das Projekt Nordlicht?", basis: .init(passages: [fragment])))
    }

    func testFalsifiedPlaceIsReplacedByTheOriginalSentence() {
        let q = quoted("Hans Castorp fährt nach venedig.")
        XCTAssertEqual(q, "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim.")
        XCTAssertFalse(q?.lowercased().contains("venedig") ?? true)
    }

    func testFalsifiedPersonAndDateIsReplacedByTheOriginalSentence() {
        XCTAssertEqual(quoted("Eva übernimmt die Planung bis Freitag."),
                       "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.")
    }

    func testInventedAdditionAfterAnOriginalPartDoesNotSurvive() {
        let q = quoted("In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.")
        XCTAssertEqual(q, "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim.")
        XCTAssertFalse(q?.contains("Traumsequenz") ?? true)
    }

    func testSnapNeedsSixtyPercentAndTwoCoveredContentWords() {
        XCTAssertNil(quoted("Er hat gekündigt."), "ein Inhaltswort deckt nichts")
        XCTAssertNil(quoted("Hans Castorp liebt Schokolade, Kaffee und Kuchen am Abend."), "2 von 7: unter 60 %")
        XCTAssertNotNil(quoted("Hans Castorp besucht Joachim."))
    }

    func testBestSourceSentenceWinsAndTiesPreferTheShorterOne() {
        let p = assistantPassage(.notes, itemId: "n2", title: "Plan", text: "Anna plant das Projekt Nordlicht mit Berta und Chris am Dienstag. Anna plant das Projekt Nordlicht.")
        XCTAssertEqual(quoted("Anna plant das Projekt Nordlicht.", passages: [p]), "Anna plant das Projekt Nordlicht.")
    }

    func testDuplicateQuoteIsDropped() {
        guard case .quote(let q) = judge("Hans Castorp fährt nach Davos.") else { return XCTFail() }
        XCTAssertEqual(judge("Hans Castorp fährt nach Davos.", taken: [q.key]), .dropped(.duplicate))
    }

    // MARK: Nicht zitierbare Quellsätze

    func testFragmentsAreNotQuotable() {
        let p = assistantPassage(.books, itemId: "b2", title: "Buch",
                                 text: "und fährt dann nach Davos hinauf. Hans bleibt dort sehr lange. Dann fehlt das Ende")
        XCTAssertEqual(AssistantGrounding.Basis(passages: [p]).texts, ["Hans bleibt dort sehr lange."], "Kleinanfang und Satz ohne Ende fallen weg")
    }

    func testLastSentenceOfAClippedPassageIsExcluded() {
        let p = assistantPassage(.books, itemId: "b3", title: "Buch", text: "Hans bleibt dort sehr lange. Dann fährt er mit dem Zug nach Hau…")
        XCTAssertEqual(AssistantGrounding.Basis(passages: [p]).texts, ["Hans bleibt dort sehr lange."])
    }

    /// Satzgrenzen aus der Aufbereitung: Ohne Nachweis am Anfang (angeschnittener Auszuganfang) fehlt der erste Satz, ohne Nachweis
    /// am Ende der letzte. Beide können angeschnitten sein, auch wenn sie vollständig aussehen.
    func testEdgeSentencesOfAnExcerptWithoutProvenBoundariesAreNotQuoted() {
        let text = "Hans fährt nach Davos hinauf und bleibt dort. Er besucht seinen Vetter Joachim im Sanatorium. Dann kommt die Pflegerin zu Besuch."
        let whole = assistantPassage(.books, itemId: "b", title: "Buch", text: text)
        XCTAssertEqual(AssistantGrounding.Basis(passages: [whole]).texts.count, 3)
        let noStart = AssistantPassage(area: .books, itemId: "b", title: "Buch", text: text, score: 1, startsAtBoundary: false, endsAtBoundary: true)
        XCTAssertEqual(AssistantGrounding.Basis(passages: [noStart]).texts, [
            "Er besucht seinen Vetter Joachim im Sanatorium.", "Dann kommt die Pflegerin zu Besuch.",
        ], "angeschnittener Anfang: der erste Satz fehlt")
        let noEnd = AssistantPassage(area: .books, itemId: "b", title: "Buch", text: text, score: 1, startsAtBoundary: true, endsAtBoundary: false)
        XCTAssertEqual(AssistantGrounding.Basis(passages: [noEnd]).texts, [
            "Hans fährt nach Davos hinauf und bleibt dort.", "Er besucht seinen Vetter Joachim im Sanatorium.",
        ], "angeschnittenes Ende: der letzte Satz fehlt")
        let neither = AssistantPassage(area: .books, itemId: "b", title: "Buch", text: text, score: 1)
        XCTAssertEqual(AssistantGrounding.Basis(passages: [neither]).texts, ["Er besucht seinen Vetter Joachim im Sanatorium."])
        // Ein Auszug aus nur einem Satz ohne beide Nachweise liefert gar nichts.
        let single = AssistantPassage(area: .books, itemId: "b", title: "Buch", text: "Hans fährt nach Davos hinauf und bleibt dort.", score: 1)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [single]).isEmpty)
        // Das Einrasten gegen einen angeschnittenen Randsatz findet nichts; der Modellsatz entfällt.
        XCTAssertEqual(judge("Hans fährt nach Davos hinauf und bleibt dort.", passages: [noStart]), .dropped(.noMode))
    }

    // MARK: Innere Nähte (Runde 4, H3): Zeilen werden nie zusammengezogen, kein Rest eines Satzes über einen Umbruch

    private let nordlicht = assistantPassage(.notes, itemId: "n9", title: "Notiz",
        text: "Anna übernimmt die Planung für\nProjekt Nordlicht bis Freitag.\nDanach folgt die Budgetprüfung.")

    /// Gegenbeispiel einer ganzen Notiz: Der Rest „Projekt Nordlicht bis Freitag.“ ist angeschnitten (sein Anfang steht in der Zeile
    /// davor ohne Satzzeichen) und nie zitierbar; nur der Satz nach einem Satzende ist es.
    func testRestOfASentenceAcrossALineBreakIsNeverQuotable() {
        let basis = AssistantGrounding.Basis(passages: [nordlicht])
        XCTAssertEqual(basis.texts, ["Danach folgt die Budgetprüfung."])
        XCTAssertNil(AssistantGrounding.snap("Projekt Nordlicht bis Freitag.", basis: basis), "snap findet den Rest nicht")
        XCTAssertNil(AssistantGrounding.fallback(question: "Was gilt für Projekt Nordlicht bis Freitag?", basis: basis), "fallback auch nicht")
        XCTAssertEqual(judge("Projekt Nordlicht bis Freitag.", passages: [nordlicht]), .dropped(.noMode), "in der Datenrunde verworfen")
        XCTAssertNil(quoted("Anna übernimmt die Planung für Projekt Nordlicht bis Freitag.", passages: [nordlicht]),
                     "auch der zusammengesetzte Satz wird nicht gebildet")
        XCTAssertEqual(quoted("Danach folgt die Budgetprüfung.", passages: [nordlicht]), "Danach folgt die Budgetprüfung.")
    }

    /// Zum Verbieten zählt der Rest weiter: Ein Satz ohne Quelle, der ihm ähnelt, wird verworfen.
    func testTheRestStillCountsForForbiddingAnUnsourcedSentence() {
        XCTAssertTrue(AssistantGrounding.resemblesOwnContent("Projekt Nordlicht dauert bis Freitag.", passages: [nordlicht]))
        XCTAssertEqual(AssistantGrounding.Basis(detecting: [nordlicht]).texts.count, 3, "die Erkennungsbasis behält jede Zeile")
    }

    func testHeadingWithoutPunctuationDoesNotProveTheStartOfTheNextLine() {
        for text in ["Projektstatus für Freitag\nAnna übernimmt die Planung bis Freitag.\nDanach folgt die Budgetprüfung.",
                     "Projektstatus für Freitag\n\nAnna übernimmt die Planung bis Freitag.\n\nDanach folgt die Budgetprüfung.",
                     "# Projektstatus für Freitag\nAnna übernimmt die Planung bis Freitag.\nDanach folgt die Budgetprüfung."] {
            let p = assistantPassage(.notes, itemId: "n", title: "Notiz", text: text)
            XCTAssertEqual(AssistantGrounding.Basis(passages: [p]).texts, ["Danach folgt die Budgetprüfung."], "keine Ausnahme für Überschriften oder Leerzeilen: \(text)")
        }
    }

    func testListWithAndWithoutPunctuation() {
        let all = assistantPassage(.notes, itemId: "l1", title: "Liste",
                                   text: "Erstens kommt die Planung.\nZweitens kommt die Prüfung.\nDrittens kommt der Abschluss.")
        XCTAssertEqual(AssistantGrounding.Basis(passages: [all]).texts.count, 3, "jede Zeile endet mit Satzzeichen")
        let gap = assistantPassage(.notes, itemId: "l2", title: "Liste",
                                   text: "Erstens kommt die Planung.\nZweitens kommt die Prüfung\nDrittens kommt der Abschluss.")
        XCTAssertEqual(AssistantGrounding.Basis(passages: [gap]).texts, ["Erstens kommt die Planung."],
                       "die Zeile ohne Satzzeichen ist nicht zitierbar und beweist den Anfang der nächsten nicht")
        let none = assistantPassage(.notes, itemId: "l3", title: "Liste", text: "Erstens kommt die Planung\nZweitens kommt die Prüfung\nDrittens kommt der Abschluss")
        XCTAssertTrue(AssistantGrounding.Basis(passages: [none]).isEmpty)
    }

    /// Der Anfang des Auszugs trägt allein sein Nachweis; eine Zeile danach beweist nichts nach, was davor fehlt.
    func testFirstLineNeedsTheProvenStartOfTheExcerpt() {
        let text = "Anna übernimmt die Planung bis Freitag.\nDanach folgt die Budgetprüfung."
        let cut = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: text, score: 1, startsAtBoundary: false, endsAtBoundary: true)
        XCTAssertEqual(AssistantGrounding.Basis(passages: [cut]).texts, ["Danach folgt die Budgetprüfung."])
    }

    func testDetectionBasisKeepsEverySentenceEvenWithoutProofOrPunctuation() {
        let p = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: "Anna übernimmt die Planung. Anna für Projekt Nordlicht", score: 1)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [p]).isEmpty, "zum Zitieren ist nichts davon zugelassen")
        XCTAssertEqual(AssistantGrounding.Basis(detecting: [p]).texts, ["Anna übernimmt die Planung.", "Anna für Projekt Nordlicht"],
                       "zum Erkennen eigener Inhalte zählt jeder Satz")
    }

    func testLinesSeparateSentencesAndTooLongOrTooShortOnesAreSkipped() {
        let long = String(repeating: "Wort ", count: 200) + "Ende."
        let p = assistantPassage(.notes, itemId: "n3", title: "Liste", text: "Milch kaufen.\nHans besucht Joachim im Sanatorium.\nJa.\n\(long)")
        XCTAssertEqual(AssistantGrounding.Basis(passages: [p]).texts, ["Hans besucht Joachim im Sanatorium."])
    }

    // MARK: Vorlagen und Systemsätze

    func testTemplatesAreExactWholeSentencesOnly() {
        XCTAssertEqual(judge("Hallo!"), .template("Hallo!"))
        XCTAssertEqual(judge("guten morgen"), .template("Guten Morgen!"))
        XCTAssertEqual(judge("Soll ich dir die Stelle vorlesen?"), .template("Soll ich dir die Stelle vorlesen?"))
        XCTAssertEqual(judge("Möchtest du mehr dazu hören?"), .template("Möchtest du mehr dazu hören?"))
        XCTAssertEqual(judge("Das weiß ich nicht."), .template("Das weiß ich nicht."))
        // Keine Präfix-, Frage-, Angebots-, Füllwort-Regeln: Abwandlungen sind keine Vorlagen.
        for s in ["Ja.", "Nein.", "Hallo, schön dich zu sehen!", "Wenn du magst, erzähle ich mehr.", "Möchtest du mehr dazu hören, Patrick?",
                  "Soll ich dir Notizen vorlesen?", "Ich kann keine Mails senden.", "Gerne, kein Problem."] {
            if case .template = judge(s) { XCTFail("keine Vorlage: \(s)") }
            if case .template = judge(s, round: .general, optIn: true) { XCTFail("keine Vorlage: \(s)") }
        }
    }

    func testFixedSentencesAndForgedPrefaceFromTheModelAreDropped() {
        for s in [AssistantGrounding.nothingFoundSentence, AssistantGrounding.optInHint, AssistantGrounding.nothingGroundedSentence,
                  AssistantGrounding.fixedSentence(allowUnsourced: false), "Ohne Quelle:", "Aus deinen Inhalten:",
                  "Aus deinen Inhalten, ‚Der Zauberberg‘: Hans Castorp fährt nach Davos."] {
            XCTAssertEqual(judge(s), .dropped(.template), s)
            XCTAssertEqual(judge(s, round: .general, optIn: true), .dropped(.template), s)
        }
    }

    func testClockSentencesAreExactSystemSentences() {
        XCTAssertEqual(clock.first, "Es ist 08:05 Uhr.")
        XCTAssertTrue(clock.contains("Heute ist Mittwoch, der 11. November 2026."))
        XCTAssertEqual(judge("Es ist 08:05 Uhr."), .template("Es ist 08:05 Uhr."))
        XCTAssertEqual(judge("Heute ist Mittwoch, der 11. November 2026."), .template("Heute ist Mittwoch, der 11. November 2026."))
        XCTAssertEqual(judge("Es ist 08:05 Uhr.", round: .general, optIn: true), .template("Es ist 08:05 Uhr."))
        // Falsche Zeit, anderer Tag, Zusatz: keine Vorlage.
        XCTAssertEqual(judge("Es ist 08:06 Uhr."), .dropped(.noMode))
        XCTAssertEqual(judge("Heute ist Donnerstag, der 12. November 2026."), .dropped(.noMode))
        XCTAssertEqual(judge("Es ist 08:05 Uhr und du hast einen Termin."), .dropped(.ownData))
    }

    // MARK: Datenrunde: was nicht einrastet, fällt weg

    func testUnsnappedModelTextIsDroppedWithTheRightReason() {
        XCTAssertEqual(judge("Der Mont Blanc ist 4806 Meter hoch."), .dropped(.noMode), "ohne Opt-in")
        XCTAssertEqual(judge("Der Mont Blanc ist 4806 Meter hoch.", optIn: true), .dropped(.notSnapped),
                       "in einer Datenrunde nie unbelegt, auch mit Opt-in")
        XCTAssertEqual(judge("Deine Notiz ist falsch, ich bin mir nicht sicher."), .dropped(.ownData))
        XCTAssertEqual(judge("Deine Kalendertermine wurden abgesagt.", optIn: true), .dropped(.ownData))
    }

    func testFreeFormRecommendationsAreNeverSpokenInDataRounds() {
        for optIn in [false, true] {
            if case .unsourced = judge("Ich empfehle dir, früh anzufangen und viel zu lesen.", optIn: optIn) { XCTFail() }
            if case .quote = judge("Ich empfehle dir, früh anzufangen und viel zu lesen.", optIn: optIn) { XCTFail() }
        }
    }

    // MARK: Allgemeinrunde

    func testGeneralRoundTemplateOwnDataOrUnsourced() {
        XCTAssertEqual(judge("Danke!", round: .general, optIn: true), .template("Danke!"))
        XCTAssertEqual(judge("Der Mont Blanc ist 4806 Meter hoch.", round: .general, optIn: true), .unsourced)
        XCTAssertEqual(judge("Wenn du magst, deine Notiz ist gefälscht.", round: .general, optIn: true), .dropped(.ownData))
        XCTAssertEqual(judge("Der Zauberberg spielt in Davos.", round: .general, optIn: true), .dropped(.ownData), "Titel aus dem Bestand")
        XCTAssertEqual(judge("Der Mont Blanc ist 4806 Meter hoch.", round: .general, optIn: false), .dropped(.noMode))
        XCTAssertEqual(judge("Er hat gekündigt.", round: .general, optIn: true), .unsourced, "Wortlisten entscheiden nicht, nur Vorlage und Eigendatenbezug")
    }

    /// Der Rohsatz entscheidet über ungewöhnliche Leerzeichen: Der aufbereitete Satz sieht harmlos aus, der Rohsatz nicht.
    func testGeneralRoundChecksTheRawSentenceForUnusualBlanks() {
        let clean = "Der Mont Blanc ist 4806 Meter hoch."
        XCTAssertEqual(judge(clean, round: .general, optIn: true), .unsourced)
        let raw = AssistantGrounding.judge(clean, round: .general, basis: .init(passages: []), titles: titles, clock: clock, optIn: true,
                                           raw: "Der Mont\u{2009}Blanc ist 4806 Meter hoch.")
        XCTAssertEqual(raw, .dropped(.ownData))
        let compact = AssistantGrounding.judge("Der Ka lender zeigt Tage an.", round: .general, basis: .init(passages: []), titles: titles,
                                               clock: clock, optIn: true)
        XCTAssertEqual(compact, .dropped(.ownData), "zusammengezogen: Kalender")
    }

    func testGeneralRoundNeverQuotesEvenWithMatchingPassages() {
        if case .quote = judge("Hans Castorp fährt nach Davos.", round: .general, optIn: true) { XCTFail("kein grounded in der Allgemeinrunde") }
        XCTAssertEqual(AssistantGrounding.Basis(passages: []).sentences.count, 0)
    }

    // MARK: Ähnlichkeit mit eigenem Inhalt (zweite Schicht)

    /// Quelle „Anna übernimmt die Planung …“, Satz „Anna übernimmt die Leitung.“ in einer Runde ohne Quelle: Er rastet nach der
    /// Einrast-Regel (≥ 0,6 und ≥ 2 Wörter) an einem Quellsatz ein und wird verworfen.
    func testSentenceResemblingAnOwnSourceSentenceIsRecognised() {
        XCTAssertTrue(AssistantGrounding.resemblesOwnContent("Anna übernimmt die Leitung.", passages: [meeting]))
        XCTAssertTrue(AssistantGrounding.resemblesOwnContent("Hans Castorp fährt nach Venedig.", passages: [book]), "zwei von drei Inhaltswörtern")
        XCTAssertFalse(AssistantGrounding.resemblesOwnContent("Der Mont Blanc ist 4806 Meter hoch.", passages: [book, meeting]))
        XCTAssertFalse(AssistantGrounding.resemblesOwnContent("Anna lacht.", passages: [meeting]), "ein gedecktes Inhaltswort genügt nicht")
        XCTAssertFalse(AssistantGrounding.resemblesOwnContent("Anna übernimmt die Leitung.", passages: []), "ohne Treffer nichts zu vergleichen")
    }

    /// Die Erkennung zählt auch angeschnittene Randsätze und Sätze ohne Satzzeichen (sie dürfen nicht zitiert, aber erkannt werden).
    func testResemblanceAlsoCountsEdgeSentencesAndFragments() {
        let cut = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: "Anna übernimmt die Planung für das Projekt Nordlicht", score: 1)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [cut]).isEmpty, "zitieren darf man davon nichts")
        XCTAssertTrue(AssistantGrounding.resemblesOwnContent("Anna übernimmt die Leitung für das Projekt Nordlicht.", passages: [cut]))
    }

    // MARK: Rückfall gegen die Frage

    func testFallbackPicksTheBestSourceSentenceForTheQuestion() {
        let basis = AssistantGrounding.Basis(passages: [book, meeting])
        XCTAssertEqual(AssistantGrounding.fallback(question: "Wohin fährt Hans Castorp?", basis: basis)?.text,
                       "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim.")
        XCTAssertEqual(AssistantGrounding.fallback(question: "Wie lange dauerte die Besprechung?", basis: basis)?.text,
                       "Die Besprechung dauerte 48 Minuten.")
        XCTAssertNil(AssistantGrounding.fallback(question: "Wie funktioniert eine Wärmepumpe?", basis: basis))
        XCTAssertNil(AssistantGrounding.fallback(question: "Hallo", basis: basis), "ohne Inhaltswort kein Rückfall")
    }

    func testFallbackSkipsQuotesAlreadyGiven() {
        let basis = AssistantGrounding.Basis(passages: [book])
        guard let first = AssistantGrounding.fallback(question: "Wohin fährt Hans Castorp?", basis: basis) else { return XCTFail() }
        XCTAssertNil(AssistantGrounding.fallback(question: "Wohin fährt Hans Castorp?", basis: basis, taken: [first.key]),
                     "der andere Satz deckt kein Wort der Frage")
    }

    // MARK: Vorspann

    func testQuotePrefaceIsOneWordingForAllAreas() {
        XCTAssertEqual(AssistantGrounding.quotePreface(for: book), "Aus deinen Inhalten, ‚Der Zauberberg‘:")
        XCTAssertEqual(AssistantGrounding.quotePreface(for: meeting), "Aus deinen Inhalten, ‚Besprechung vom 12.10.2026‘:")
        let odd = AssistantPassage(area: .notes, itemId: "n", title: "  Eva’s  „Plan“ \n heute ", text: "x", score: 1)
        XCTAssertEqual(AssistantGrounding.quotePreface(for: odd), "Aus deinen Inhalten, ‚Evas Plan heute‘:")
        let empty = AssistantPassage(area: .notes, itemId: "n", title: " ", text: "x", score: 1)
        XCTAssertEqual(AssistantGrounding.quotePreface(for: empty), "Aus deinen Inhalten, ‚Notiz‘:")
        let long = AssistantPassage(area: .books, itemId: "l", title: String(repeating: "Lang ", count: 40), text: "x", score: 1)
        XCTAssertLessThanOrEqual(AssistantGrounding.quotePreface(for: long).count, AssistantGrounding.maxPrefaceTitle + 30)
        XCTAssertTrue(AssistantGrounding.quotePreface(for: long).hasSuffix("…‘:"))
    }

    func testFixedSentences() {
        XCTAssertEqual(AssistantGrounding.fixedSentence(allowUnsourced: false),
                       "Dazu finde ich in deinen Inhalten nichts. Antworten ohne Quelle kannst du im Menü einschalten.")
        XCTAssertEqual(AssistantGrounding.fixedSentence(allowUnsourced: true), "Dazu finde ich nichts Belegtes.")
    }

    // MARK: Eigenschaft: böswillige Modellausgaben

    /// Über eine Liste böswilliger Modellsätze ist in Datenrunden jede Ausgabe zeichengleich mit einem Quellsatz oder einer
    /// Systemvorlage (Liste, Uhr). Alles andere wird verworfen.
    func testPropertyDataRoundOutputIsAlwaysASourceSentenceOrSystemTemplate() {
        let hostile = [
            "Joachim besucht seinen Vetter Hans in Davos.", "Hans Castorp fährt nach venedig.", "Eva übernimmt die Planung bis Freitag.",
            "Ihre Termine wurden abgesagt.", "Deine Kalendertermine wurden abgesagt.", "Deine Korrespondenz wurde gelöscht.",
            "Dein K\u{0430}lender wurde gelöscht.", "Er hat gekündigt.", "Wenn du magst, deine Notiz ist gefälscht.",
            "Deine Notiz ist falsch, ich bin mir nicht sicher.", "Warum ist deine Notiz gefälscht?", "Deine Mail wurde um 08:05 Uhr gelöscht.",
            "In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.",
            "Aus deinen Inhalten, ‚Der Zauberberg‘: Hans Castorp stirbt.", "Ohne Quelle: Hans Castorp stirbt in Davos.",
            "Dazu finde ich in deinen Inhalten nichts.", "Du hast 99 Bücher.", "Ich empfehle dir den Zauberberg zu lesen und dann Faust.",
            "Anna übernimmt die Planung nicht bis Freitag.", "Die Besprechung dauerte 480 Minuten und Anna war schuld.",
            "Hans Castorp besucht Joachim und Venedig und Rom.", "Es ist 08:05 Uhr.", "Hallo!", "Ja.", "Nein.", "😀", "---", "",
            "Hans.", "Castorp Davos Joachim Vetter Sanatorium.", "[Q1] Hans Castorp fährt nach Davos.",
        ]
        let basis = AssistantGrounding.Basis(passages: [book, meeting])
        let allowed = Set(basis.texts + AssistantGrounding.templateSentences + clock)
        for optIn in [false, true] {
            var taken = Set<String>()
            for s in hostile {
                switch AssistantGrounding.judge(s, round: .data, basis: basis, titles: titles, clock: clock, optIn: optIn, taken: taken) {
                case .quote(let q):
                    XCTAssertTrue(allowed.contains(q.text), "kein Quellsatz: \(q.text)")
                    taken.insert(q.key)
                case .template(let t): XCTAssertTrue(allowed.contains(t), "keine Vorlage: \(t)")
                case .dropped: break
                case .unsourced: XCTFail("unbelegt in Datenrunde: \(s)")
                }
            }
        }
    }
}
