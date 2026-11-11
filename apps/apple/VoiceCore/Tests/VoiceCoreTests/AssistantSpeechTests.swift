import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Satztrenner für die Stimme.

final class AssistantSpeechTests: XCTestCase {
    private func run(_ deltas: [String]) -> [(text: String, speech: String, raw: String)] {
        var s = AssistantSpeech.Splitter()
        var out: [(text: String, speech: String, raw: String)] = []
        for d in deltas { out += s.feed(d) }
        out += s.finish()
        return out
    }

    private func texts(_ input: String) -> [String] { run([input]).map(\.text) }

    /// Jede Zerschneidung der Deltas muss dasselbe ergeben wie der ganze Text auf einmal.
    private func assertChunkInvariant(_ input: String, file: StaticString = #filePath, line: UInt = #line) {
        let whole = run([input]).map { "\($0.text)|\($0.speech)" }
        let chars = run(input.map { String($0) }).map { "\($0.text)|\($0.speech)" }
        XCTAssertEqual(chars, whole, "zeichenweise", file: file, line: line)
        for size in [2, 3, 5, 7, 11] {
            var parts: [String] = []
            var rest = Substring(input)
            while !rest.isEmpty { parts.append(String(rest.prefix(size))); rest = rest.dropFirst(size) }
            XCTAssertEqual(run(parts).map { "\($0.text)|\($0.speech)" }, whole, "Größe \(size)", file: file, line: line)
        }
    }

    func testSplitsSentences() {
        XCTAssertEqual(texts("Das ist gut. Wirklich? Ja! Und weiter."), ["Das ist gut.", "Wirklich?", "Ja!", "Und weiter."])
    }

    func testSentenceWaitsForLookahead() {
        var s = AssistantSpeech.Splitter()
        XCTAssertTrue(s.feed("Das ist gut.").isEmpty)
        XCTAssertEqual(s.feed(" Und").map(\.text), ["Das ist gut."])
        XCTAssertEqual(s.finish().map(\.text), ["Und"])
    }

    func testAbbreviationsDoNotSplit() {
        XCTAssertEqual(texts("Das gilt z. B. für Hunde. Danach geht es weiter."), ["Das gilt z. B. für Hunde.", "Danach geht es weiter."])
        XCTAssertEqual(texts("Dr. Müller sagt das. Ich auch."), ["Dr. Müller sagt das.", "Ich auch."])
        XCTAssertEqual(texts("Das steht auf S. 12 im Buch. Dann weiter."), ["Das steht auf S. 12 im Buch.", "Dann weiter."])
        XCTAssertEqual(texts("Es gibt Katzen, Hunde u. a. Tiere. Weiter."), ["Es gibt Katzen, Hunde u. a. Tiere.", "Weiter."])
    }

    func testDecimalNumbersAndDatesDoNotSplit() {
        XCTAssertEqual(texts("Der Wert liegt bei 3.5 Prozent. Das ist viel."), ["Der Wert liegt bei 3.5 Prozent.", "Das ist viel."])
        XCTAssertEqual(texts("Am 11. November 2026 ist es so. Danach nicht."), ["Am 11. November 2026 ist es so.", "Danach nicht."])
        XCTAssertEqual(texts("Das war im Jahr 2020. Dann kam alles anders."), ["Das war im Jahr 2020.", "Dann kam alles anders."])
    }

    func testEllipsis() {
        XCTAssertEqual(texts("Ich überlege noch… vielleicht doch nicht. Aber gut."), ["Ich überlege noch… vielleicht doch nicht.", "Aber gut."])
        XCTAssertEqual(texts("Das war eine lange Reise... Ich bin froh."), ["Das war eine lange Reise...", "Ich bin froh."])
    }

    func testFirstChunkMayEndAtCommaAfter40Chars() {
        let text = "Das Wetter in Berlin ist heute ziemlich sonnig und warm, aber am Abend wird es kühler."
        let chunks = texts(text)
        XCTAssertEqual(chunks, ["Das Wetter in Berlin ist heute ziemlich sonnig und warm,", "aber am Abend wird es kühler."])
        // Nur das erste Häppchen darf am Komma enden.
        XCTAssertEqual(texts("Erstens kurz. Das Wetter in Berlin ist heute ziemlich sonnig und warm, aber am Abend wird es kühler.").count, 2)
    }

    func testShortCommaDoesNotSplit() {
        XCTAssertEqual(texts("Ja, das stimmt so. Weiter."), ["Ja, das stimmt so.", "Weiter."])
        XCTAssertEqual(texts("Der Wert ist 3,5 und das passt nicht zu den anderen Zahlen hier, ok. Weiter."), ["Der Wert ist 3,5 und das passt nicht zu den anderen Zahlen hier,", "ok.", "Weiter."])
    }

    func testBulletsHeadingsAndNewlines() {
        let r = run(["# Überschrift\n", "- Erster Punkt\n", "* Zweiter Punkt\n", "• Dritter Punkt\n", "1. Vierter Punkt\n", "Ende."])
        XCTAssertEqual(r.map(\.text), ["Überschrift", "Erster Punkt", "Zweiter Punkt", "Dritter Punkt", "1. Vierter Punkt", "Ende."])
    }

    func testBoldItalicAndCode() {
        let r = run(["Das ist **sehr wichtig** und __auch__ das hier. Noch `code` und _kursiv_ dazu."])
        XCTAssertEqual(r.map(\.text), ["Das ist sehr wichtig und auch das hier.", "Noch code und kursiv dazu."])
        let r2 = run(["**Wichtig.** Das gilt immer."])
        XCTAssertEqual(r2.map(\.text), ["Wichtig.", "Das gilt immer."])
    }

    func testLinksKeepOnlyText() {
        let r = run(["Siehe [die Doku](https://example.com/a?b=c). Mehr dazu später."])
        XCTAssertEqual(r.map(\.text), ["Siehe die Doku.", "Mehr dazu später."])
        XCTAssertEqual(texts("Bild ![Eine Katze](x.png) hier."), ["Bild Eine Katze hier."])
    }

    func testLinkWithSentencePunctuationInsideIsNotSplit() {
        XCTAssertEqual(texts("Siehe [S. 12. Kapitel](https://example.com/a.b). Weiter."), ["Siehe S. 12. Kapitel.", "Weiter."])
    }

    func testBareUrlsStayInTextButNotInSpeech() {
        let r = run(["Mehr unter https://example.com/info gibt es noch."])
        XCTAssertEqual(r.first?.text, "Mehr unter https://example.com/info gibt es noch.")
        XCTAssertEqual(r.first?.speech, "Mehr unter gibt es noch.")
    }

    func testEmojisAreRemoved() {
        let r = run(["Das ist toll 😀👍 und schön ✨. Weiter 🎉!"])
        XCTAssertEqual(r.map(\.text), ["Das ist toll und schön.", "Weiter!"])
        for chunk in r {
            XCTAssertFalse(chunk.text.unicodeScalars.contains { AssistantSpeech.isEmoji($0) })
            XCTAssertFalse(chunk.speech.unicodeScalars.contains { AssistantSpeech.isEmoji($0) })
        }
        XCTAssertEqual(texts("🎉🎉"), [])
    }

    func testSpeechExpandsAbbreviations() {
        let r = run(["Das gilt z. B. für Hunde, d.h. für alle, usw. Siehe S. 12. Dr. Müller kommt. Bus. Ca. 5 Stück."])
        let speech = r.map(\.speech).joined(separator: " ")
        XCTAssertTrue(speech.contains("zum Beispiel"), speech)
        XCTAssertTrue(speech.contains("das heißt"), speech)
        XCTAssertTrue(speech.contains("und so weiter"), speech)
        XCTAssertTrue(speech.contains("Seite 12"), speech)
        XCTAssertTrue(speech.contains("Doktor Müller"), speech)
        XCTAssertTrue(speech.contains("Bus."), "keine Ersetzung mitten im Wort: \(speech)")
        XCTAssertTrue(speech.contains("Circa 5"), speech)
    }

    func testSpeechDecimalComma() {
        let r = run(["Es sind 3.5 Prozent, 1.000 Leute und der 12.10.2026."])
        XCTAssertEqual(r.first?.speech, "Es sind 3,5 Prozent, 1.000 Leute und der 12.10.2026.")
        XCTAssertEqual(r.first?.text, "Es sind 3.5 Prozent, 1.000 Leute und der 12.10.2026.")
    }

    func testSpeechHasNothingUnspeakable() {
        let r = run(["# Titel\n", "> Zitat | Tabelle & mehr → weiter [x] {y} <z> ~ ^ #tag snake_case_name\n", "---\n", "```swift\n", "Ende."])
        for chunk in r {
            for bad in ["*", "`", "#", "[", "]", "{", "}", "<", ">", "|", "→", "~", "^", "_", "\\"] {
                XCTAssertFalse(chunk.speech.contains(bad), "\(bad) in \(chunk.speech)")
            }
            XCTAssertFalse(chunk.speech.isEmpty)
        }
        XCTAssertFalse(r.map(\.text).contains(where: { $0.contains("---") || $0.contains("swift") }))
    }

    func testHorizontalRuleAndFenceAreDropped() {
        XCTAssertEqual(texts("Vorher.\n---\n```\nNachher."), ["Vorher.", "Nachher."])
    }

    func testBlockquote() {
        XCTAssertEqual(texts("> Ein Zitat hier.\nUnd weiter."), ["Ein Zitat hier.", "Und weiter."])
    }

    func testNewlineEndsChunkWithoutPunctuation() {
        XCTAssertEqual(texts("Erste Zeile\nZweite Zeile\n\nDritte"), ["Erste Zeile", "Zweite Zeile", "Dritte"])
    }

    func testVeryLongTextWithoutEndIsCut() {
        let text = String(repeating: "wort ", count: 200)
        let r = run([text])
        XCTAssertGreaterThan(r.count, 1)
        XCTAssertTrue(r.allSatisfy { $0.text.count <= AssistantSpeech.Splitter.maxChunk })
        XCTAssertEqual(r.map(\.text).joined(separator: " "), text.trimmingCharacters(in: .whitespaces))
    }

    func testFinishFlushesRemainderOnce() {
        var s = AssistantSpeech.Splitter()
        XCTAssertTrue(s.feed("Noch ein Rest ohne Ende").isEmpty)
        XCTAssertEqual(s.finish().map(\.text), ["Noch ein Rest ohne Ende"])
        XCTAssertTrue(s.finish().isEmpty)
    }

    func testChunkInvariance() {
        assertChunkInvariant("Das gilt z. B. für Hunde. Dr. Müller sagt: Am 11. November 2026 ist es 3.5 Grad, das ist wenig. Siehe [Link](https://example.com).")
        assertChunkInvariant("# Titel\n- **Fett** und `code`\n1. Eins\n\nSchluss… Ende! Wirklich?")
        assertChunkInvariant("Das Wetter in Berlin ist heute ziemlich sonnig und warm, aber am Abend wird es kühler. 😀 Tschüss.")
    }

    // MARK: Nur ganze Sätze (Prüfung vor der Ausgabe, Paket A)

    private func wholeSentences(_ deltas: [String]) -> (sentences: [String], overlong: Int, fragments: Int) {
        var s = AssistantSpeech.Splitter(sentencesOnly: true)
        var out: [String] = []
        for d in deltas { out += s.feed(d).map(\.text) }
        out += s.finish().map(\.text)
        return (out, s.overlong, s.fragments)
    }

    /// F: Die Prüfung sieht nur vollständige Sätze, nie Komma-Häppchen.
    func testSentencesOnlyNeverCutsAtCommasOrAfterTheFirstChars() {
        let long = "In deinem Buch fährt Hans Castorp nach Davos, um dort seinen Vetter zu besuchen; er bleibt drei Wochen, aber nur in einer erfundenen Traumsequenz."
        XCTAssertEqual(wholeSentences([long]).sentences, [long])
        // Der klassische Modus würde bei einem langen ersten Satz am Komma teilen, dieser nicht.
        XCTAssertGreaterThan(run([long]).count, 1)
        XCTAssertEqual(wholeSentences(long.map { String($0) }).sentences, [long])
    }

    func testSentencesOnlyReleasesNothingBeforeTheSentenceEnd() {
        var s = AssistantSpeech.Splitter(sentencesOnly: true)
        XCTAssertTrue(s.feed("Hans Castorp fährt nach Davos, ").isEmpty)
        XCTAssertTrue(s.feed("aber nur in einer erfundenen ").isEmpty)
        XCTAssertTrue(s.feed("Traumsequenz.").isEmpty, "das Satzzeichen am Puffer-Ende wartet auf das Folgezeichen oder das Stromende")
        XCTAssertEqual(s.feed(" Danach").map(\.text), ["Hans Castorp fährt nach Davos, aber nur in einer erfundenen Traumsequenz."])
        XCTAssertTrue(s.finish().isEmpty, "„Danach“ hat kein Satzzeichen: abgeschnittener Rest, nie geliefert")
        XCTAssertEqual(s.fragments, 1)
    }

    func testSentencesOnlyTreatsNewlinesAsWhitespace() {
        XCTAssertEqual(wholeSentences(["Erste Zeile\nZweite Zeile\n\nDritte. Vierte"]).sentences, ["Erste Zeile Zweite Zeile Dritte."])
        XCTAssertEqual(wholeSentences(["Erste Zeile\nZweite Zeile\n\nDritte. Vierte"]).fragments, 1)
        XCTAssertEqual(wholeSentences(["Ein Satz.\nNoch ein Satz.\n"]).sentences, ["Ein Satz.", "Noch ein Satz."])
    }

    func testSentencesOnlyKeepsAbbreviationsDatesAndDecimalsInOneSentence() {
        XCTAssertEqual(wholeSentences(["Dr. Müller kam am 11. November 2026 um 3.5 Uhr, z. B. nach Davos. Dann ging er."]).sentences,
                       ["Doktor Müller kam am 11. November 2026 um 3.5 Uhr, z. B. nach Davos.".replacingOccurrences(of: "Doktor", with: "Dr."),
                        "Dann ging er."])
    }

    /// H3: Ein Rest ohne Satzzeichen am Stromende ist ein abgeschnittener Satz: er wird nie geliefert, nur gezählt.
    func testSentencesOnlyDropsAndCountsAnUnterminatedRemainderAtStreamEnd() {
        var s = AssistantSpeech.Splitter(sentencesOnly: true)
        XCTAssertTrue(s.feed("Noch ein Rest ohne Ende").isEmpty)
        XCTAssertTrue(s.finish().isEmpty)
        XCTAssertEqual(s.fragments, 1)
        XCTAssertTrue(s.finish().isEmpty)
        XCTAssertEqual(s.fragments, 1, "ein zweites Finish zählt nichts doppelt")
        let r = wholeSentences(["Anna übernimmt künftig"])
        XCTAssertEqual(r.sentences, [])
        XCTAssertEqual(r.fragments, 1)
        // Satzzeichen mit schließenden Zeichen und Leerraum dahinter zählt als Ende.
        XCTAssertEqual(wholeSentences(["Er sagte: „Das stimmt.“  "]).sentences, ["Er sagte: „Das stimmt.“"])
        XCTAssertEqual(wholeSentences(["Das stimmt (wirklich!)"]).sentences, ["Das stimmt (wirklich!)"], "die Klammer schließt, das Satzzeichen steht davor: Ende")
        XCTAssertEqual(wholeSentences(["Wirklich?"]).sentences, ["Wirklich?"])
        XCTAssertEqual(wholeSentences(["Und dann…"]).sentences, ["Und dann…"])
        // Nichts Lesbares ist kein Fragment.
        for junk in ["---", "😀", "   \n"] {
            let j = wholeSentences([junk])
            XCTAssertEqual(j.sentences, [])
            XCTAssertEqual(j.fragments, 0, junk)
        }
    }

    /// Der Rohsatz kommt unverändert mit (für die Prüfung auf ungewöhnliche Leerzeichen); `text` ist aufbereitet.
    func testSentencesOnlyKeepsTheRawSentence() {
        var s = AssistantSpeech.Splitter(sentencesOnly: true)
        let raw = "Das \u{2009}steht im Ka\u{2003}lender."
        let got = s.feed(raw + " Danach.") + s.finish()
        XCTAssertEqual(got.first?.raw, raw)
        XCTAssertEqual(got.first?.text, "Das steht im Ka lender.", "aufbereitet: Leerraum zusammengezogen")
    }

    func testSentencesOnlyChunkInvariance() {
        let input = "Das gilt z. B. für Hunde. Dr. Müller sagt: Am 11. November 2026 ist es 3.5 Grad, das ist wenig. Siehe [Link](https://example.com). Ende… Wirklich?\nUnd weiter."
        let whole = wholeSentences([input]).sentences
        XCTAssertEqual(wholeSentences(input.map { String($0) }).sentences, whole)
        for size in [2, 3, 5, 7, 11] {
            var parts: [String] = []
            var rest = Substring(input)
            while !rest.isEmpty { parts.append(String(rest.prefix(size))); rest = rest.dropFirst(size) }
            XCTAssertEqual(wholeSentences(parts).sentences, whole, "Größe \(size)")
        }
        XCTAssertEqual(whole.count, 5, "\(whole)")
    }

    /// Ein Satz ohne Ende über der Obergrenze wird verworfen und gezählt, samt Rest bis zum Satzende; danach geht es normal weiter.
    func testSentencesOnlyOverlongWithoutEndIsDroppedCountedAndSkippedToTheSentenceEnd() {
        let r = wholeSentences([String(repeating: "wort ", count: 450), " und Rest. Danach ein neuer Satz."])
        XCTAssertEqual(r.sentences, ["Danach ein neuer Satz."])
        XCTAssertEqual(r.overlong, 1)
        XCTAssertEqual(r.fragments, 0)
    }

    func testSentencesOnlyOverlongWithEndIsDroppedAndCounted() {
        let r = wholeSentences([String(repeating: "wort ", count: 450) + "Ende. Und noch einer"])
        XCTAssertEqual(r.sentences, [], "„Und noch einer“ endet ohne Satzzeichen")
        XCTAssertEqual(r.overlong, 1)
        XCTAssertEqual(r.fragments, 1)
        XCTAssertLessThanOrEqual(AssistantSpeech.Splitter.maxSentence, 2_000)
    }

    func testUmlautsAndSharpSStayIntact() {
        XCTAssertEqual(texts("Größe, Übung und Straße sind schön. Äpfel auch."), ["Größe, Übung und Straße sind schön.", "Äpfel auch."])
    }
}
