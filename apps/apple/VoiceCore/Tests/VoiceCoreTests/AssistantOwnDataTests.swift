import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Eigendatenbezug (zweite Schicht). Frage und Satz, nur synthetische Inhalte.

final class AssistantOwnDataTests: XCTestCase {
    private let titles = ["Der Zauberberg", "Besprechung vom 12.10.2026", "Quartalsplanung"]

    private func sentence(_ s: String, titles: [String] = []) -> Bool { AssistantOwnData.refers(s, mode: .sentence, titles: titles) }
    private func question(_ s: String, titles: [String] = []) -> Bool { AssistantOwnData.refers(s, mode: .question, titles: titles) }

    // MARK: Goldfälle aus der Entscheidung

    func testGoldSentencesReferToOwnData() {
        for s in [
            "Wenn du magst, deine Notiz ist gefälscht.",
            "Deine Notiz ist falsch, ich bin mir nicht sicher.",
            "Deine Mail wurde um 08:05 Uhr gelöscht.",
            "Ihre Termine wurden abgesagt.",
            "Deine Kalendertermine wurden abgesagt.",
            "Deine Korrespondenz wurde gelöscht.",
        ] { XCTAssertTrue(sentence(s), s) }
        XCTAssertTrue(question("Warum ist deine Notiz gefälscht?"))
    }

    func testCyrillicLookalikeLetterCountsAsOwnData() {
        let cyrillicA = "K\u{0430}lender"
        XCTAssertNotEqual(cyrillicA, "Kalender")
        XCTAssertTrue(sentence("Dein \(cyrillicA) wurde gelöscht."))
        // Auch ohne weiteres Merkmal: ein Wort mit fremder Schrift genügt.
        XCTAssertTrue(sentence("Der \(cyrillicA) ist voll."))
        XCTAssertTrue(sentence("Wann ist der T\u{0435}rmin?"))
        XCTAssertTrue(question("Wann ist der T\u{0435}rmin?"))
        XCTAssertTrue(sentence("Das ist ein griechisches Δ."))
        XCTAssertTrue(sentence("Der Kalender zeigt Tage, Wochen und Monate an."), "Kalender ist selbst ein Stamm")
    }

    func testFormatCharactersAndFullwidthLettersDoNotHideAStem() {
        XCTAssertTrue(sentence("Der Ka\u{200B}lender ist voll."))
        XCTAssertTrue(sentence("Die No\u{00AD}tiz liegt dort."))
        XCTAssertTrue(sentence("Der Ｋａｌｅｎｄｅｒ ist voll."))
        XCTAssertTrue(sentence("Die Notiz\u{0301}en sind alt."))
    }

    // MARK: Stämme, Buch

    func testStemsAreMatchedInsideCompounds() {
        for s in ["Das Notizbuch ist leer.", "Die Besprechungsnotizen fehlen.", "Das Postfach läuft über.", "Posteingangsregeln helfen.",
                  "Der Termin liegt morgen.", "Die Mitschrift ist kurz.", "Ein Hörbuch läuft."] {
            XCTAssertTrue(sentence(s), s)
        }
    }

    func testBuchOnlyAsWordWithFormsOrAsWordEnd() {
        XCTAssertTrue(sentence("Das Buch ist spannend."))
        XCTAssertTrue(sentence("Viele Bücher stehen im Regal."))
        XCTAssertTrue(sentence("Ein Lehrbuch hilft beim Lernen."))
        XCTAssertFalse(sentence("Die Buche ist ein Laubbaum."))
        XCTAssertFalse(sentence("Buchstaben sind Zeichen."))
        XCTAssertFalse(sentence("Die Buchhaltung prüft Zahlen."))
    }

    func testGeneralKnowledgeIsNotOwnData() {
        for s in ["Der Mont Blanc ist 4806 Meter hoch.", "Paris ist die Hauptstadt von Frankreich.", "Wasser kocht bei 100 Grad.",
                  "Die Sonne geht im Osten auf.", "Ja.", "Nein."] {
            XCTAssertFalse(sentence(s), s)
        }
        XCTAssertFalse(question("Wie hoch ist der Mont Blanc?"))
    }

    // MARK: Anrede und Besitz

    func testAddressForms() {
        XCTAssertTrue(sentence("Das ist dein Plan."))
        XCTAssertTrue(sentence("Das ist euer Plan."))
        XCTAssertTrue(sentence("Das sind eure Pläne."))
        XCTAssertTrue(sentence("Ihre Pläne sind gut."), "großgeschriebenes Ihr*")
        XCTAssertTrue(sentence("Ich danke Ihnen."))
        XCTAssertFalse(sentence("Eva und ihre Schwester kommen."), "kleingeschriebenes ihre ist kein Bezug")
    }

    func testDuWithActivityVerbOrPast() {
        XCTAssertTrue(sentence("Du hast gesagt, es regnet."))
        XCTAssertTrue(sentence("Du warst gestern dort."))
        XCTAssertTrue(sentence("Du wolltest das notieren."))
        XCTAssertTrue(sentence("Du hattest recht."))
        XCTAssertFalse(sentence("Du hast recht."))
        XCTAssertFalse(sentence("Du kannst das lernen."))
    }

    func testQuestionOnlyMarkers() {
        XCTAssertTrue(question("Was steht in meinem Plan?"))
        XCTAssertTrue(question("Was haben wir besprochen?"))
        XCTAssertTrue(question("Was habe ich gestern gesagt?"))
        XCTAssertTrue(question("Wie lautet unsere Planung?"))
        XCTAssertFalse(sentence("Meine Güte, das ist lang."), "im Satz zählt „mein“ nicht")
        XCTAssertFalse(sentence("Ich habe gestern gesagt, dass es regnet."), "im Satz zählt „ich“ nicht")
    }

    // MARK: Titel, Quellenhinweise

    func testTitlesFromInventoryAsWordSequence() {
        XCTAssertTrue(sentence("Der Zauberberg spielt in Davos.", titles: titles))
        XCTAssertTrue(sentence("Im Zauberberg geht es um Davos.", titles: titles), "Titel auch ohne Artikel")
        XCTAssertTrue(sentence("Die Quartalsplanung ist wichtig.", titles: titles))
        XCTAssertTrue(sentence("Quartalsplanung ist wichtig.", titles: titles))
        XCTAssertFalse(sentence("Ein Berg ist hoch.", titles: titles))
        XCTAssertFalse(sentence("Der Zauberer wohnt im Turm.", titles: titles))
        XCTAssertFalse(sentence("Der Zauberberg spielt in Davos.", titles: []), "ohne Titelliste kein Treffer")
    }

    func testLautGemassAndPageWithNumber() {
        XCTAssertTrue(sentence("Laut Wikipedia ist Paris groß."))
        XCTAssertTrue(sentence("Gemäß dem Gesetz gilt das."))
        XCTAssertTrue(sentence("Das steht auf Seite 12."))
        XCTAssertTrue(sentence("Das steht auf Seite zwölf."))
        XCTAssertFalse(sentence("Die Seite ist leer."))
        XCTAssertFalse(sentence("Die Zahl 12 ist gerade."))
    }

    // MARK: Zweite Schicht: zusammengezogene Wortfolge, Leerzeichen, Titel jeder Länge

    /// Der Stamm wird auch gegen die zusammengezogene Wortfolge ohne Leerzeichen geprüft: Ein zerschnittenes Wort verrät sich.
    func testStemsAreAlsoMatchedAgainstTheContractedWordSequence() {
        for s in ["Der Ka lender ist voll.", "Der K a l e n d e r ist voll.", "Der Ka-len-der ist voll.", "Die No tiz liegt dort.",
                  "Der Ter min liegt morgen.", "Die Post ein gangs regeln helfen.", "Das ist ein Bu ch.", "Das Notiz bu ch ist leer.",
                  "Die Ma il kam an."] {
            XCTAssertTrue(sentence(s), s)
            XCTAssertTrue(question(s), s)
        }
    }

    /// Die Zusammenziehung erzeugt Fehlalarme („Oma Ilse“ ergibt „omailse“): gewollt streng, ein Fehlalarm kostet nur eine Antwort.
    func testContractionAcceptsFalseAlarmsOnPurpose() {
        XCTAssertTrue(sentence("Oma Ilse backt einen Kuchen."), "„omailse“ enthält „mail“")
        XCTAssertFalse(sentence("Oma Berta backt einen Kuchen."))
    }

    /// Jedes Leerzeichen außer U+0020, U+00A0 und U+202F im Rohtext gilt als Bezug; die drei gewöhnlichen nicht.
    func testUnusualBlanksCountAsReferenceButTheOrdinaryOnesDoNot() {
        for blank in ["\u{2009}", "\u{2003}", "\u{2002}", "\u{3000}", "\u{2005}", "\u{200A}", "\u{1680}", "\u{205F}", "\u{2028}",
                      "\u{2029}", "\u{2800}", "\u{3164}", "\u{115F}", "\u{FFA0}"] {
            XCTAssertTrue(sentence("Wasser\(blank)kocht bei 100 Grad."), "U+\(String(blank.unicodeScalars.first!.value, radix: 16))")
            XCTAssertTrue(question("Wie hoch\(blank)ist der Mont Blanc?"))
        }
        for blank in [" ", "\u{00A0}", "\u{202F}", "\n"] {
            XCTAssertFalse(sentence("Wasser\(blank)kocht bei 100 Grad."), "gewöhnlich: U+\(String(blank.unicodeScalars.first!.value, radix: 16))")
        }
        XCTAssertTrue(AssistantOwnData.hasUnusualBlank("a\u{2009}b"))
        XCTAssertFalse(AssistantOwnData.hasUnusualBlank("a b\u{00A0}c\u{202F}d"))
    }

    /// Titel jeder Länge zählen als ganze Wortfolge, ohne Rücksicht auf Groß- und Kleinschreibung.
    func testTitlesOfAnyLengthMatchAsAWholeWordSequenceCaseInsensitively() {
        let long = "Wie ich lernte die Planung des Jahres in sieben kleinen Schritten zu verstehen und dabei Spaß zu haben"
        XCTAssertGreaterThan(long.split(separator: " ").count, 15)
        XCTAssertTrue(sentence("Gestern las jemand \(long.lowercased()) im Zug.", titles: [long]))
        XCTAssertTrue(sentence("\(long.uppercased()) ist ein Titel.", titles: [long]))
        XCTAssertFalse(sentence("Wie ich lernte die Planung des Jahres in sieben kleinen Schritten.", titles: [long]), "nur ein Anfang des Titels")
        XCTAssertTrue(sentence("Im Zauberberg geht es um Davos.", titles: ["DER ZAUBERBERG"]), "Titel in Großbuchstaben, Satz normal")
        XCTAssertTrue(sentence("Wir sprachen über Quartalsplanung und mehr.", titles: ["quartalsplanung"]))
        XCTAssertTrue(sentence("Das Treffen am Dienstag um zehn Uhr im Büro war lang.", titles: ["Das Treffen am Dienstag um zehn Uhr im Büro"]))
    }

    /// Ein Titel nur aus Stoppwörtern („Es“) zählt bei exakter Schreibung und nur außerhalb des Satzanfangs.
    func testTitleOfOnlyStopwordsNeedsExactSpellingAndNoSentenceStart() {
        XCTAssertTrue(sentence("Dann las ich Es noch einmal.", titles: ["Es"]))
        XCTAssertFalse(sentence("Es regnet heute.", titles: ["Es"]), "Satzanfang: Großschreibung ist gewöhnlich")
        XCTAssertFalse(sentence("Dann regnet es heute.", titles: ["Es"]), "falsche Schreibung")
        XCTAssertFalse(sentence("Dann regnet ES heute.", titles: ["Es"]), "falsche Schreibung")
        XCTAssertTrue(sentence("Wir sahen Er Sie gestern.", titles: ["Er Sie"]), "zwei Stoppwörter: exakte Schreibung")
        XCTAssertFalse(sentence("Wir sahen er sie gestern.", titles: ["Er Sie"]))
        XCTAssertTrue(sentence("Wir sahen die welt gestern.", titles: ["Die Welt"]), "ein Titel mit Inhaltswort: ohne Rücksicht auf Groß-/Kleinschreibung")
        XCTAssertFalse(sentence("Es", titles: ["Es"]), "ein Satz nur aus dem Titel steht am Satzanfang")
    }

    func testTitleWithoutArticleAlsoMatchesAndOtherTitlesNeverMatchPartially() {
        XCTAssertTrue(sentence("Zauberberg ist lang.", titles: ["Der Zauberberg"]))
        XCTAssertFalse(sentence("Zauber ist lang.", titles: ["Der Zauberberg"]))
        XCTAssertFalse(sentence("Der Zauberberg ist lang.", titles: ["Zauberberg und mehr"]))
        XCTAssertFalse(sentence("Der Zauberberg ist lang.", titles: ["", " ", "…"]))
    }

    func testEmptyTextIsNotOwnData() {
        XCTAssertFalse(sentence(""))
        XCTAssertFalse(question("   "))
        XCTAssertFalse(sentence("?!"))
    }
}
