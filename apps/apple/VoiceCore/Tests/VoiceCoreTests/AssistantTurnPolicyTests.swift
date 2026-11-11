import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Rundenart vor dem Modellaufruf.

final class AssistantTurnPolicyTests: XCTestCase {
    private let book = AssistantPassage(
        area: .books, itemId: "b1", title: "Der Zauberberg", locator: "S. 12",
        text: "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim.", score: 1)
    private let titles = ["Der Zauberberg"]

    private func decide(_ q: String, optIn: Bool = true, passages: [AssistantPassage] = [], previous: AssistantRoundKind? = nil)
        -> AssistantTurnPolicy.Decision {
        AssistantTurnPolicy.decide(question: q, allowUnsourced: optIn, titles: titles, passages: passages, previous: previous)
    }

    func testWithoutOptInEveryRoundIsADataRound() {
        for q in ["Wie hoch ist der Mont Blanc?", "Allgemein: Wie hoch ist der Mont Blanc?", "Hallo"] {
            XCTAssertEqual(decide(q, optIn: false), .init(kind: .data, reason: .optedOut), q)
        }
    }

    func testGeneralOnlyWhenEverythingHolds() {
        XCTAssertEqual(decide("Wie hoch ist der Mont Blanc?"), .init(kind: .general, reason: .general))
        XCTAssertEqual(decide("Wie hoch ist der Mont Blanc?", previous: .general), .init(kind: .general, reason: .general))
    }

    func testOwnDataQuestionIsADataRound() {
        XCTAssertEqual(decide("Was steht in meiner Notiz?"), .init(kind: .data, reason: .ownData))
        XCTAssertEqual(decide("Warum ist deine Notiz gefälscht?"), .init(kind: .data, reason: .ownData))
        XCTAssertEqual(decide("Worum geht es im Zauberberg?"), .init(kind: .data, reason: .ownData), "Titel aus dem Bestand")
        XCTAssertEqual(decide("Wann ist der T\u{0435}rmin?"), .init(kind: .data, reason: .ownData), "kyrillisches е")
    }

    func testRelevantPassageMakesADataRound() {
        XCTAssertEqual(decide("Wohin fährt Hans Castorp?", passages: [book]), .init(kind: .data, reason: .relevantPassage))
    }

    func testOneIncidentalWordIsNotRelevant() {
        // Nur „Davos“ ist gemeinsam: unter der Schwelle von zwei Wörtern.
        XCTAssertEqual(decide("Wie hoch liegt Davos über dem Meer?", passages: [book]), .init(kind: .general, reason: .general))
        // Eine Frage mit nur einem Inhaltswort braucht genau dieses Wort.
        XCTAssertTrue(AssistantTurnPolicy.isRelevant(book, to: "Davos?"))
        XCTAssertFalse(AssistantTurnPolicy.isRelevant(book, to: "Venedig?"))
    }

    func testRelevanceThresholdUsesQuestionContentWordsAndTitle() {
        XCTAssertTrue(AssistantTurnPolicy.isRelevant(book, to: "Was geschieht im Sanatorium von Davos?"))
        XCTAssertTrue(AssistantTurnPolicy.isRelevant(book, to: "Zauberberg Sanatorium"), "Titel zählt mit")
        XCTAssertFalse(AssistantTurnPolicy.isRelevant(book, to: "Wie funktioniert eine Wärmepumpe?"))
        XCTAssertTrue(AssistantTurnPolicy.isRelevant(book, to: "Und?"), "ohne Inhaltswort lässt sich nichts ausschließen")
    }

    func testFollowUpAfterADataRoundStaysData() {
        XCTAssertEqual(decide("Und was macht er dann?", previous: .data), .init(kind: .data, reason: .carryOver))
        XCTAssertEqual(decide("Er hat gekündigt.", previous: .data), .init(kind: .data, reason: .carryOver))
        XCTAssertEqual(decide("Warum?", previous: .data), .init(kind: .data, reason: .carryOver), "unter drei Inhaltswörtern")
        XCTAssertEqual(decide("Und warum eigentlich?", previous: .data), .init(kind: .data, reason: .carryOver))
        XCTAssertEqual(decide("Wann passierte das?", previous: .data), .init(kind: .data, reason: .carryOver))
        XCTAssertEqual(decide("Und was geschah dort?", previous: .data), .init(kind: .data, reason: .carryOver))
    }

    func testIndependentLongQuestionAfterADataRoundMayBeGeneral() {
        XCTAssertEqual(decide("Erkläre mir bitte die Funktionsweise einer Wärmepumpe im Winter", previous: .data),
                       .init(kind: .general, reason: .general))
        XCTAssertEqual(decide("Und warum?", previous: .general), .init(kind: .general, reason: .general), "keine Übernahme aus Allgemeinrunde")
        XCTAssertEqual(decide("Er hat gekündigt.", previous: nil), .init(kind: .general, reason: .general), "ohne Vorrunde")
    }

    func testExplicitGeneralOverridesHitsAndCarryOverButNotOwnData() {
        XCTAssertEqual(decide("Allgemein: Wohin fährt Hans Castorp?", passages: [book], previous: .data),
                       .init(kind: .general, reason: .explicitGeneral))
        XCTAssertEqual(decide("Beantworte das bitte ohne Quelle", previous: .data), .init(kind: .general, reason: .explicitGeneral))
        XCTAssertEqual(decide("Wie funktioniert das allgemeinen gesprochen?", previous: .data), .init(kind: .general, reason: .explicitGeneral))
        XCTAssertEqual(decide("Allgemein gefragt: was steht in meiner Notiz?"), .init(kind: .data, reason: .ownData))
        XCTAssertEqual(decide("Ohne Quelle: erkläre den Zauberberg"), .init(kind: .data, reason: .ownData))
        XCTAssertEqual(decide("Allgemein: Wohin fährt Hans Castorp?", optIn: false, passages: [book]), .init(kind: .data, reason: .optedOut))
    }
}
