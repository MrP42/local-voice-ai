import XCTest
@testable import VoiceCore

final class SpeechTextTests: XCTestCase {
    func testGermanAbbreviationsAreSpokenOut() {
        XCTAssertEqual(SpeechText.spoken("Obst, z. B. Äpfel, d.h. gesund.", language: "de"),
                       "Obst, zum Beispiel Äpfel, das heißt gesund.")
        XCTAssertEqual(SpeechText.spoken("Siehe S. 12 bzw. Nr. 3.", language: "de"), "Siehe Seite 12 beziehungsweise Nummer 3.")
        XCTAssertEqual(SpeechText.spoken("Nr.5 auf S.7", language: "de"), "Nummer 5 auf Seite 7")
        XCTAssertEqual(SpeechText.spoken("Dr. Meier kam.", language: "de"), "Doktor Meier kam.")
    }

    func testAbbreviationAtTheEndKeepsTheFullStop() {
        XCTAssertEqual(SpeechText.spoken("Äpfel, Birnen usw.", language: "de"), "Äpfel, Birnen und so weiter.")
    }

    func testOnlyWholeWords() {
        XCTAssertEqual(SpeechText.spoken("Der Stand. Ein Weg.", language: "de"), "Der Stand. Ein Weg.")
        XCTAssertEqual(SpeechText.spoken("Seite S. ohne Zahl", language: "de"), "Seite S. ohne Zahl")
    }

    func testEnglishAndUnknownLanguage() {
        XCTAssertEqual(SpeechText.spoken("Fruit, e.g. apples.", language: "en"), "Fruit, for example apples.")
        XCTAssertEqual(SpeechText.spoken("z. B. bleibt", language: "fr"), "z. B. bleibt")
    }

    func testFlowPresetsAreOrdered() {
        let brisk = SpeechFlow(SpeechFlow.brisk), book = SpeechFlow(SpeechFlow.audiobook), calm = SpeechFlow(SpeechFlow.calm)
        XCTAssertLessThan(brisk.sentenceMs, book.sentenceMs)
        XCTAssertLessThan(book.sentenceMs, calm.sentenceMs)
        XCTAssertLessThan(book.commaMs, book.sentenceMs)
        XCTAssertLessThan(book.sentenceMs, book.paragraphMs)
        XCTAssertEqual(SpeechFlow(7).value, 1)
    }
}
