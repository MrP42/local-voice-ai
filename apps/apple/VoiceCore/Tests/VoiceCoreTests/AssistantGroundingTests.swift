import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Satzprüfung vor der Ausgabe. Goldfälle auf Deutsch; nur synthetische Inhalte.

final class AssistantGroundingTests: XCTestCase {
    private let book = AssistantPassage(
        area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12",
        text: "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim. Er bleibt drei Wochen.", score: 1)
    private let meeting = AssistantPassage(
        area: .transcripts, itemId: "t1", title: "Besprechung vom 12.10.2026", locator: "00:14:03",
        text: "Die Besprechung dauerte 48 Minuten. Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.", score: 0.8)
    private let titles = ["Der Zauberberg", "Besprechung vom 12.10.2026", "Quartalsplanung"]
    private let inventory = "Bücher: 3 insgesamt\n- Der Zauberberg (Thomas Mann)\nTranskripte: 1 insgesamt\n- Besprechung vom 12.10.2026 (48 min)"
    private let clock = "Mittwoch, der 11. November 2026, 08:05 Uhr"

    private func classify(_ sentence: String, passages: [AssistantPassage]? = nil) -> AssistantGroundingVerdict {
        AssistantGrounding.classify(sentence: sentence, passages: passages ?? [book, meeting], inventoryTitles: titles,
                                    inventory: inventory, clock: clock)
    }

    private func assertGrounded(_ sentence: String, by expected: [AssistantPassage], file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(classify(sentence), .grounded(expected), sentence, file: file, line: line)
    }

    // MARK: Belegt

    func testSentenceSupportedByPassageIsGroundedWithThatPassage() {
        assertGrounded("Hans Castorp fährt nach Davos, um seinen Vetter Joachim zu besuchen.", by: [book])
        assertGrounded("Die Besprechung dauerte 48 Minuten.", by: [meeting])
        assertGrounded("Anna übernimmt die Planung bis Freitag.", by: [meeting])
    }

    func testSentenceMayNameTheSourceTitleAndKindWithoutItStandingInTheText() {
        assertGrounded("In deinem Buch fährt Castorp nach Davos.", by: [book])
        assertGrounded("Im Zauberberg besucht Hans Castorp seinen Vetter.", by: [book])
        assertGrounded("In der Besprechung vom 12.10.2026 übernimmt Anna die Planung.", by: [meeting])
    }

    func testInventoryStatementsAreGroundedWithoutPassageSources() {
        XCTAssertEqual(classify("Du hast drei Bücher.", passages: []), .grounded([]))
        XCTAssertEqual(classify("Du hast ein Transkript.", passages: []), .grounded([]))
    }

    func testInventoryCountsMustMatchTheirOwnArea() {
        XCTAssertEqual(classify("Du hast drei Bücher und ein Transkript.", passages: []), .grounded([]))
        XCTAssertEqual(classify("Du hast zwei Bücher und ein Transkript.", passages: []), .appClaimUnsupported)
        XCTAssertEqual(classify("Du hast drei Transkripte.", passages: []), .appClaimUnsupported, "„3“ steht bei den Büchern")
        XCTAssertEqual(classify("Du hast ein Transkript.", passages: []), .grounded([]))
        XCTAssertEqual(classify("Du hast ein Buch.", passages: []), .appClaimUnsupported, "Bücher: 3")
        XCTAssertEqual(classify("Du hast 3 Bücher.", passages: []), .grounded([]))
    }

    func testInventoryStatementWithTitleIsGrounded() {
        XCTAssertEqual(classify("Zu deinen Büchern gehört Der Zauberberg von Thomas Mann.", passages: []), .grounded([]))
        XCTAssertEqual(classify("Zu deinen Büchern gehört Der Prozess von Franz Kafka.", passages: []), .appClaimUnsupported)
    }

    func testParaphraseKeepingNamesStaysGrounded() {
        assertGrounded("Joachim ist der Vetter von Castorp.", by: [book])
        assertGrounded("Castorp besucht in Davos seinen Vetter.", by: [book])
        assertGrounded("Er bleibt drei Wochen.", by: [book])
    }

    // MARK: Falsche Behauptungen über App-Inhalte

    func testWrongNumberAboutAppContentIsDropped() {
        XCTAssertEqual(classify("Du hast fünf Bücher.", passages: []), .appClaimUnsupported)
        XCTAssertEqual(classify("Im Zauberberg bleibt Castorp sieben Jahre."), .appClaimUnsupported)
    }

    func testDerivationWithNumberNotInExcerptIsNotGrounded() {
        XCTAssertEqual(classify("Die Besprechung dauerte genau 60 Minuten."), .appClaimUnsupported)
        XCTAssertEqual(classify("Die Besprechung dauerte 48 Minuten."), .grounded([meeting]))
    }

    func testSwappedNameOrPlaceIsDroppedEvenWithoutAnyAppWord() {
        // Ohne Namensprüfung läge die Deckung bei 75 % und der Satz ginge durch.
        XCTAssertEqual(classify("Hans Castorp fährt nach Venedig."), .appClaimUnsupported)
        XCTAssertEqual(classify("Joachim besucht seinen Vetter Hans in Davos."), .grounded([book]))
        XCTAssertEqual(classify("Peter übernimmt die Planung bis Freitag."), .appClaimUnsupported)
    }

    func testNegationFlipAgainstExcerptIsDropped() {
        XCTAssertEqual(classify("Hans Castorp fährt nicht nach Davos."), .appClaimUnsupported)
    }

    func testWrongSourceTitleIsNotAttributedToAnotherBook() {
        XCTAssertEqual(classify("Im Prozess fährt Hans Castorp nach Davos."), .appClaimUnsupported)
    }

    // MARK: Erfundene Quellen

    func testMadeUpSourcesAreDropped() {
        XCTAssertEqual(classify("Laut deiner Notiz Urlaubsplanung fährst du im Mai nach Italien."), .appClaimUnsupported)
        XCTAssertEqual(classify("In dem Dokument Quartalsbericht steht, dass der Umsatz gestiegen ist."), .appClaimUnsupported)
        XCTAssertEqual(classify("Du hast gestern notiert, dass der Termin ausfällt."), .appClaimUnsupported)
        XCTAssertEqual(classify("Im Kapitel zwölf wird das Wetter beschrieben."), .appClaimUnsupported)
        XCTAssertEqual(classify("Auf Seite 40 steht etwas über Bergluft."), .appClaimUnsupported)
        XCTAssertEqual(classify("Gemäß dem Protokoll wurde nichts beschlossen."), .appClaimUnsupported)
        XCTAssertEqual(classify("Laut dem Wetterbericht wird es morgen regnen."), .appClaimUnsupported)
    }

    func testTitleFromTheInventoryIsAnAppReferenceAlone() {
        XCTAssertEqual(classify("Die Quartalsplanung enthält viele Zahlen.", passages: []), .appClaimUnsupported)
    }

    func testWithoutAnyExcerptEveryAppReferenceIsDropped() {
        XCTAssertEqual(classify("In deinem Buch fährt Castorp nach Davos.", passages: []), .appClaimUnsupported)
        XCTAssertEqual(classify("In der Besprechung übernimmt Anna die Planung.", passages: []), .appClaimUnsupported)
    }

    // MARK: Unbelegt

    func testGeneralKnowledgeAndRecommendationsAreUnsourced() {
        XCTAssertEqual(classify("Der Mont Blanc ist 4806 Meter hoch."), .unsourced)
        XCTAssertEqual(classify("Ich würde dir empfehlen, vor dem Schlafen nicht mehr zu essen."), .unsourced)
        XCTAssertEqual(classify("Hunde schwitzen vor allem über die Pfoten."), .unsourced)
    }

    // MARK: Neutral

    func testQuestionsGreetingsTemplatesAndAnnouncementsAreNeutral() {
        for s in [
            "Soll ich dir die Stelle vorlesen?", "Möchtest du mehr dazu hören?", "Hallo!", "Gerne.", "Okay, kein Problem.",
            "Dazu finde ich in deinen Inhalten nichts.", "Dazu finde ich nichts Belegtes.", "Dazu steht nichts im Buch.",
            "Antworten ohne Quelle kannst du im Menü einschalten.", "Ohne Quelle:", "Aus deinen Inhalten:",
            "Wenn du magst, erzähle ich dir mehr dazu.", "Sag Bescheid, wenn ich etwas anderes suchen soll.",
            "Ich kann derzeit keine Mails senden.", "Das weiß ich nicht.", "Ich bin mir nicht sicher.",
        ] {
            XCTAssertEqual(classify(s), .neutral, s)
        }
    }

    func testClockStatementIsNeutralButAWrongDayIsNot() {
        XCTAssertEqual(classify("Heute ist Mittwoch, der 11. November 2026."), .neutral)
        XCTAssertEqual(classify("Es ist 08:05 Uhr."), .neutral)
        XCTAssertNotEqual(classify("Heute ist Donnerstag, der 12. November 2026."), .neutral)
    }

    func testQuestionsCannotSmuggleClaims() {
        XCTAssertEqual(classify("Wusstest du, dass Castorp in deinem Buch 40 Jahre in Davos bleibt und dort alle Freunde verliert?"),
                       .appClaimUnsupported)
        XCTAssertEqual(classify("Stimmt es, dass der Mont Blanc 4806 Meter hoch ist?"), .unsourced)
        // Auch kurz und ohne Zahl: Ein „dass“-Satz in der Frage wird wie eine Aussage geprüft.
        XCTAssertNotEqual(classify("Wusstest du, dass Castorp nach Venedig fährt?"), .neutral)
        XCTAssertEqual(classify("Wusstest du, dass Hans Castorp in deinem Buch nach Venedig fährt?"), .appClaimUnsupported)
        XCTAssertEqual(classify("Wusstest du, dass Hunde über die Pfoten schwitzen?"), .unsourced)
    }

    func testMailAndCalendarClaimsAboutOwnDataAreDropped() {
        XCTAssertEqual(classify("Du hast morgen einen Termin beim Zahnarzt."), .appClaimUnsupported)
        XCTAssertEqual(classify("In deinem Kalender steht am Freitag eine Reise."), .appClaimUnsupported)
        XCTAssertEqual(classify("Deine letzte Mail kam von der Stadtverwaltung."), .appClaimUnsupported)
        // Ohne Bezug auf den Nutzer bleibt es Allgemeinwissen.
        XCTAssertEqual(classify("Ein Termin beim Notar dauert meist eine halbe Stunde."), .unsourced)
    }

    func testOfferAndAbilityRulesDoNotWaveThroughClaims() {
        XCTAssertEqual(classify("Ich kann dir sagen, dass Hunde 3 Mägen haben."), .unsourced)
        XCTAssertEqual(classify("Wenn du Fieber hast, solltest du zum Arzt gehen."), .unsourced)
    }

    func testEmptyAndWhitespaceAreNeutral() {
        XCTAssertEqual(classify("   "), .neutral)
        XCTAssertEqual(classify(""), .neutral)
    }

    // MARK: Hilfsfunktionen

    func testFixedSentencesAndTemplates() {
        XCTAssertEqual(AssistantGrounding.fixedSentence(allowUnsourced: false),
                       "Dazu finde ich in deinen Inhalten nichts. Antworten ohne Quelle kannst du im Menü einschalten.")
        XCTAssertEqual(AssistantGrounding.fixedSentence(allowUnsourced: true), "Dazu finde ich nichts Belegtes.")
        XCTAssertTrue(AssistantGrounding.isTemplate("Dazu finde ich in deinen Inhalten nichts."))
        XCTAssertTrue(AssistantGrounding.isTemplate("The book says nothing about that."))
        XCTAssertFalse(AssistantGrounding.isTemplate("Der Mont Blanc ist hoch."))
    }

    func testPreparedBasisGivesTheSameVerdictAsTheConvenienceCall() {
        let basis = AssistantGrounding.Basis(passages: [book, meeting], inventoryTitles: titles, inventory: inventory, clock: clock)
        for s in ["Hans Castorp fährt nach Venedig.", "Die Besprechung dauerte 48 Minuten.", "Der Mont Blanc ist 4806 Meter hoch."] {
            XCTAssertEqual(AssistantGrounding.classify(sentence: s, basis: basis), classify(s), s)
        }
    }
}
