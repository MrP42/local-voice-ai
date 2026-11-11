import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a/E4.2b): Prüfung vor der Ausgabe, strukturell. Goldfälle auf Deutsch; nur synthetische
// Inhalte. In Datenrunden ist jede ausgegebene Einheit zeichengleich mit einem GANZEN gespeicherten Quellblock oder einer
// Systemvorlage; Teile, Kürzungen und Zusammenzüge eines Blocks gibt es nicht.

final class AssistantGroundingTests: XCTestCase {
    private let book = AssistantPassage(
        area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12",
        text: "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim. Er bleibt drei Wochen.", score: 1,
        isBlock: true, quotable: true)
    private let meeting = AssistantPassage(
        area: .transcripts, itemId: "t1", title: "Besprechung vom 12.10.2026", locator: "00:14:03",
        text: "Die Besprechung dauerte 48 Minuten. Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.", score: 0.8,
        isBlock: true, quotable: true)
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

    // MARK: Einrasten (Goldfälle der Entscheidung): Ausgegeben wird immer der GANZE Block

    func testModelSentenceSnapsToTheWholeSourceBlockNeverItself() {
        let model = "Hans Castorp fährt nach Davos zu seinem Vetter."
        XCTAssertEqual(quoted(model), book.text, "gesprochen wird nur der ganze Quellblock, beide Sätze")
        XCTAssertNotEqual(quoted(model), model)
    }

    func testFalsifiedPlaceIsReplacedByTheOriginalBlock() {
        let q = quoted("Hans Castorp fährt nach venedig.")
        XCTAssertEqual(q, book.text)
        XCTAssertFalse(q?.lowercased().contains("venedig") ?? true)
    }

    func testFalsifiedPersonAndDateIsReplacedByTheOriginalBlock() {
        XCTAssertEqual(quoted("Eva übernimmt die Planung bis Freitag."), meeting.text)
    }

    func testInventedAdditionAfterAnOriginalPartDoesNotSurvive() {
        let q = quoted("In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.")
        XCTAssertEqual(q, book.text)
        XCTAssertFalse(q?.contains("Traumsequenz") ?? true)
    }

    func testSnapNeedsSixtyPercentAndTwoCoveredContentWords() {
        XCTAssertNil(quoted("Er hat gekündigt."), "ein Inhaltswort deckt nichts")
        XCTAssertNil(quoted("Hans Castorp liebt Schokolade, Kaffee und Kuchen am Abend."), "2 von 7: unter 60 %")
        XCTAssertNotNil(quoted("Hans Castorp besucht Joachim."))
    }

    func testBestBlockWinsAndTiesPreferTheShorterOne() {
        let long = assistantPassage(.notes, itemId: "n2", title: "Plan", text: "Anna plant das Projekt Nordlicht mit Berta und Chris am Dienstag.")
        let short = assistantPassage(.notes, itemId: "n3", title: "Plan", text: "Anna plant das Projekt Nordlicht.")
        XCTAssertEqual(quoted("Anna plant das Projekt Nordlicht.", passages: [long, short]), short.text)
    }

    func testDuplicateQuoteIsDropped() {
        guard case .quote(let q) = judge("Hans Castorp fährt nach Davos.") else { return XCTFail() }
        XCTAssertEqual(judge("Hans Castorp fährt nach Davos.", taken: [q.key]), .dropped(.duplicate))
    }

    // MARK: Quellblöcke: ganz oder gar nicht

    /// Satzzeichen, Länge in Wörtern und Anfangsbuchstabe sind kein Kriterium: ein gespeicherter Block ohne Satzzeichen ist als
    /// Ganzes zitierbar.
    func testABlockWithoutPunctuationIsQuotableAsAWhole() {
        let bare = assistantPassage(.notes, itemId: "n1", title: "Notiz", text: "Hans besucht Joachim im Sanatorium")
        XCTAssertEqual(AssistantGrounding.Basis(passages: [bare]).texts, ["Hans besucht Joachim im Sanatorium"])
        XCTAssertEqual(quoted("Joachim besucht seinen Vetter Hans im Sanatorium.", passages: [bare]), "Hans besucht Joachim im Sanatorium")
    }

    /// Was nicht als ganzer gespeicherter Block vorliegt (gekürzter Auszug, Teil eines Blocks), ist nie zitierbar.
    func testAnExcerptThatIsNotAWholeBlockIsNeverQuotable() {
        let part = assistantPassage(.books, itemId: "b2", title: "Buch", text: "und fährt dann nach Davos hinauf. Hans bleibt dort sehr lange.", whole: false)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [part]).isEmpty)
        XCTAssertNil(quoted("Hans bleibt dort sehr lange in Davos.", passages: [part]))
        XCTAssertNil(AssistantGrounding.fallback(question: "Wo bleibt Hans lange in Davos?", basis: .init(passages: [part])))
        XCTAssertEqual(judge("Hans bleibt dort sehr lange in Davos.", passages: [part]), .dropped(.noMode))
    }

    /// Ein Block über `maxBlockChars` erscheint nie, auch nicht gekürzt oder in Teilen.
    func testAnOverlongBlockNeverAppearsNotEvenShortened() {
        let long = assistantPassage(.notes, itemId: "n9", title: "Notiz",
                                    text: String(repeating: "Anna plant das Projekt Nordlicht mit Berta. ", count: 20))
        XCTAssertGreaterThan(long.text.count, AssistantGrounding.maxBlockChars)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [long]).isEmpty)
        XCTAssertNil(quoted("Anna plant das Projekt Nordlicht mit Berta.", passages: [long]))
        XCTAssertNil(AssistantGrounding.fallback(question: "Wer plant das Projekt Nordlicht?", basis: .init(passages: [long])))
        let exact = assistantPassage(.notes, itemId: "n8", title: "Notiz", text: String(repeating: "a", count: AssistantGrounding.maxBlockChars))
        XCTAssertEqual(AssistantGrounding.Basis(passages: [exact]).texts.count, 1, "genau die Höchstlänge ist noch zitierbar")
    }

    /// Eine Abkürzung am Zeilenende trennt keinen Block: Der Block ist EIN Block und wird zeichengleich mit Zeilenumbruch ausgegeben.
    func testAMultiLineBlockWithAnAbbreviationIsOneBlockAndAppearsUnchanged() {
        for first in ["Dr.", "Regierungsdir."] {
            let text = "\(first)\nMeyer übernimmt die Planung für das Projekt Nordlicht."
            let p = assistantPassage(.notes, itemId: "n7", title: "Notiz", text: text)
            XCTAssertEqual(AssistantGrounding.Basis(passages: [p]).texts, [text], first)
            XCTAssertEqual(quoted("Meyer übernimmt die Planung für das Projekt Nordlicht.", passages: [p]), text, "unverändert, mit Zeilenumbruch")
        }
    }

    /// Zwei Blöcke werden nie zusammengezogen: Deckt ein Modellsatz Wörter aus beiden, wird genau EIN ganzer Block ausgegeben.
    func testTwoBlocksAreNeverMerged() {
        let a = assistantPassage(.notes, itemId: "n1", title: "Notiz", locator: "A", text: "Anna übernimmt die Planung für das Projekt Nordlicht.")
        let b = assistantPassage(.notes, itemId: "n1", title: "Notiz", locator: "B", text: "Berta übernimmt das Budget für das Projekt Nordlicht.")
        let q = quoted("Anna übernimmt die Planung und Berta das Budget für das Projekt Nordlicht.", passages: [a, b])
        XCTAssertTrue(q == a.text || q == b.text, "genau ein ganzer Block: \(q ?? "nil")")
        XCTAssertFalse((q ?? "").contains(a.text) && (q ?? "").contains(b.text))
    }

    func testDetectionBasisKeepsEverySentenceEvenWhenNotQuotable() {
        let p = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: "Anna übernimmt die Planung. Anna für Projekt Nordlicht", score: 1)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [p]).isEmpty, "zum Zitieren ist nichts davon zugelassen")
        let texts = AssistantGrounding.Basis(detecting: [p]).texts
        XCTAssertTrue(texts.contains("Anna übernimmt die Planung."))
        XCTAssertTrue(texts.contains("Anna für Projekt Nordlicht"))
        XCTAssertTrue(AssistantGrounding.resemblesOwnContent("Anna übernimmt die Leitung für das Projekt Nordlicht.", passages: [p]),
                      "zum Erkennen eigener Inhalte zählt jeder Teil")
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
                  "Wörtlicher Auszug aus ‚Der Zauberberg‘: Hans Castorp fährt nach Davos."] {
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
        XCTAssertTrue(AssistantGrounding.Basis(passages: []).isEmpty)
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
    func testResemblanceAlsoCountsNonQuotableFragments() {
        let cut = AssistantPassage(area: .notes, itemId: "n", title: "Notiz", text: "Anna übernimmt die Planung für das Projekt Nordlicht", score: 1)
        XCTAssertTrue(AssistantGrounding.Basis(passages: [cut]).isEmpty, "zitieren darf man davon nichts")
        XCTAssertTrue(AssistantGrounding.resemblesOwnContent("Anna übernimmt die Leitung für das Projekt Nordlicht.", passages: [cut]))
    }

    // MARK: Rückfall gegen die Frage

    func testFallbackPicksTheBestSourceBlockForTheQuestion() {
        let basis = AssistantGrounding.Basis(passages: [book, meeting])
        XCTAssertEqual(AssistantGrounding.fallback(question: "Wohin fährt Hans Castorp?", basis: basis)?.text, book.text)
        XCTAssertEqual(AssistantGrounding.fallback(question: "Wie lange dauerte die Besprechung?", basis: basis)?.text, meeting.text)
        XCTAssertNil(AssistantGrounding.fallback(question: "Wie funktioniert eine Wärmepumpe?", basis: basis))
        XCTAssertNil(AssistantGrounding.fallback(question: "Hallo", basis: basis), "ohne Inhaltswort kein Rückfall")
    }

    func testFallbackSkipsQuotesAlreadyGiven() {
        let basis = AssistantGrounding.Basis(passages: [book])
        guard let first = AssistantGrounding.fallback(question: "Wohin fährt Hans Castorp?", basis: basis) else { return XCTFail() }
        XCTAssertNil(AssistantGrounding.fallback(question: "Wohin fährt Hans Castorp?", basis: basis, taken: [first.key]), "der einzige Block ist vergeben")
    }

    // MARK: Vorspann

    func testQuotePrefaceIsOneWordingForAllAreas() {
        XCTAssertEqual(AssistantGrounding.quotePreface(for: book), "Wörtlicher Auszug aus ‚Der Zauberberg‘:")
        XCTAssertEqual(AssistantGrounding.quotePreface(for: meeting), "Wörtlicher Auszug aus ‚Besprechung vom 12.10.2026‘:")
        let odd = AssistantPassage(area: .notes, itemId: "n", title: "  Eva’s  „Plan“ \n heute ", text: "x", score: 1)
        XCTAssertEqual(AssistantGrounding.quotePreface(for: odd), "Wörtlicher Auszug aus ‚Evas Plan heute‘:")
        let empty = AssistantPassage(area: .notes, itemId: "n", title: " ", text: "x", score: 1)
        XCTAssertEqual(AssistantGrounding.quotePreface(for: empty), "Wörtlicher Auszug aus ‚Notiz‘:")
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

    /// Über eine Liste böswilliger Modellsätze ist in Datenrunden jede Ausgabe zeichengleich mit einem ganzen Quellblock oder einer
    /// Systemvorlage (Liste, Uhr). Alles andere wird verworfen.
    func testPropertyDataRoundOutputIsAlwaysAWholeSourceBlockOrSystemTemplate() {
        let hostile = [
            "Joachim besucht seinen Vetter Hans in Davos.", "Hans Castorp fährt nach venedig.", "Eva übernimmt die Planung bis Freitag.",
            "Ihre Termine wurden abgesagt.", "Deine Kalendertermine wurden abgesagt.", "Deine Korrespondenz wurde gelöscht.",
            "Dein K\u{0430}lender wurde gelöscht.", "Er hat gekündigt.", "Wenn du magst, deine Notiz ist gefälscht.",
            "Deine Notiz ist falsch, ich bin mir nicht sicher.", "Warum ist deine Notiz gefälscht?", "Deine Mail wurde um 08:05 Uhr gelöscht.",
            "In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.",
            "Wörtlicher Auszug aus ‚Der Zauberberg‘: Hans Castorp stirbt.", "Ohne Quelle: Hans Castorp stirbt in Davos.",
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
                    XCTAssertTrue(allowed.contains(q.text), "kein Quellblock: \(q.text)")
                    taken.insert(q.key)
                case .template(let t): XCTAssertTrue(allowed.contains(t), "keine Vorlage: \(t)")
                case .dropped: break
                case .unsourced: XCTFail("unbelegt in Datenrunde: \(s)")
                }
            }
        }
    }
}
