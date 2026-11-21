import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket K): Vollduplex-Entscheidungslogik, ohne Audio und ohne Uhr.

final class AssistantDuplexTests: XCTestCase {
    private typealias Action = DuplexTurnPolicy.Action
    private typealias Input = DuplexTurnPolicy.Input

    private func feed(_ p: inout DuplexTurnPolicy, _ inputs: Input...) -> [Action] {
        inputs.flatMap { p.feed($0) }
    }

    /// Nutzer spricht `text`, Stille bis zum Äußerungsende; liefert alle Aktionen.
    private func say(_ p: inout DuplexTurnPolicy, _ text: String, at ms: Int) -> [Action] {
        feed(&p, .speechStarted(atMs: ms), .partial(text: text, atMs: ms + 500), .silence(sinceMs: 350, atMs: ms + 850),
             .silence(sinceMs: 600, atMs: ms + 1_100))
    }

    /// Antwort läuft; der erste Satz hat angefangen (`firstEnded`: auch zu Ende gesprochen). Eine Einheit klingt (weiter).
    private func startAnswer(_ p: inout DuplexTurnPolicy, id: String = "r1", sentence: String = "Das Wetter morgen wird sonnig und warm.",
                             startedAt: Int = 2_000, endedAt: Int? = nil) {
        _ = feed(&p, .answerStarted(requestId: id), .unitStarted(atMs: startedAt),
                 .sentenceSpoken(text: sentence, startedAtMs: startedAt, endedAtMs: endedAt))
    }

    /// Die Tests dieser Datei beschreiben die feste Frist von 600 ms; das Ende nach Inhalt prüft `AssistantTurnShapeTests`.
    private func legacyPolicy() -> DuplexTurnPolicy {
        var config = DuplexTurnPolicy.Config()
        config.adaptiveEnd = false
        return DuplexTurnPolicy(config: config)
    }

    private func committed(_ actions: [Action]) -> [(String, Bool, Bool)] {
        actions.compactMap { if case .commit(let t, let m, let i) = $0 { return (t, m, i) } else { return nil } }
    }

    // (a)
    func testNormalUtteranceSpeculatesAt350AndCommitsAt600() {
        var p = legacyPolicy()
        XCTAssertEqual(feed(&p, .speechStarted(atMs: 1_000), .partial(text: "Wie wird das Wetter", atMs: 1_400)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 200, atMs: 1_600)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 1_750)), [.speculate(text: "Wie wird das Wetter")])
        XCTAssertEqual(p.feed(.silence(sinceMs: 400, atMs: 1_800)), [], "Vorarbeit nur einmal")
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 2_000)), [.commit(text: "Wie wird das Wetter", merged: false, afterInterruption: false)])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 2_100)), [], "nur ein commit")
    }

    func testCommitWithoutPriorSpeculationWhenSilenceJumps() {
        var p = legacyPolicy()
        _ = feed(&p, .partial(text: "Hallo zusammen", atMs: 100))
        XCTAssertEqual(p.feed(.silence(sinceMs: 650, atMs: 800)), [.commit(text: "Hallo zusammen", merged: false, afterInterruption: false)])
    }

    // (b)
    func testContinuingAfterSpeculationDiscardsIt() {
        var p = legacyPolicy()
        _ = feed(&p, .speechStarted(atMs: 0), .partial(text: "Wie wird das Wetter", atMs: 1_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 1_350)), [.speculate(text: "Wie wird das Wetter")])
        _ = p.feed(.answerStarted(requestId: "spec"))
        XCTAssertEqual(p.feed(.partial(text: "Wie wird das Wetter morgen", atMs: 1_400)), [.discardSpeculation])
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 1_800)), [.speculate(text: "Wie wird das Wetter morgen")])
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 2_050)), [.commit(text: "Wie wird das Wetter morgen", merged: false, afterInterruption: false)])
    }

    func testSpeechStartedAloneDiscardsSpeculationOnce() {
        var p = legacyPolicy()
        _ = feed(&p, .partial(text: "Wie wird das Wetter", atMs: 1_000), .silence(sinceMs: 350, atMs: 1_350))
        XCTAssertEqual(p.feed(.speechStarted(atMs: 1_400)), [.discardSpeculation])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 1_450)), [])
    }

    func testSameTextRefinementKeepsSpeculation() {
        var p = legacyPolicy()
        _ = feed(&p, .partial(text: "wie wird das wetter", atMs: 1_000), .silence(sinceMs: 350, atMs: 1_350))
        XCTAssertEqual(p.feed(.partial(text: "Wie wird das Wetter?", atMs: 1_400)), [], "nur Schreibweise geändert")
    }

    // (c)
    func testAddingBeforeFirstSentenceEndsMergesUtterances() {
        var p = legacyPolicy()
        XCTAssertEqual(committed(say(&p, "Wie wird das Wetter morgen", at: 0)).first?.0, "Wie wird das Wetter morgen")
        startAnswer(&p, startedAt: 2_000, endedAt: nil)
        XCTAssertEqual(p.feed(.partial(text: "und auch in Berlin", atMs: 2_500)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(p.feed(.partial(text: "und auch in Berlin bitte", atMs: 2_900)), [], "kein zweiter Abbruch")
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 3_300)), [.speculate(text: "Wie wird das Wetter morgen und auch in Berlin bitte")])
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 3_550)),
                       [.commit(text: "Wie wird das Wetter morgen und auch in Berlin bitte", merged: true, afterInterruption: false)])
    }

    func testAddingWhileThinkingMerges() {
        var p = legacyPolicy()
        _ = say(&p, "Erzähl mir etwas über Rom", at: 0)
        _ = p.feed(.answerStarted(requestId: "r1"))
        XCTAssertEqual(p.feed(.partial(text: "genauer über das Kolosseum", atMs: 1_800)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 600, atMs: 2_500))).first?.0, "Erzähl mir etwas über Rom genauer über das Kolosseum")
    }

    func testMergeChainsAcrossSeveralAdditions() {
        var p = legacyPolicy()
        _ = say(&p, "Eins", at: 0)
        _ = p.feed(.answerStarted(requestId: "r1"))
        XCTAssertEqual(p.feed(.partial(text: "zwei drei", atMs: 1_800)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 600, atMs: 2_500))).first?.0, "Eins zwei drei")
        _ = p.feed(.answerStarted(requestId: "r2"))
        XCTAssertEqual(p.feed(.partial(text: "vier fünf", atMs: 3_000)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 600, atMs: 3_700))).first?.0, "Eins zwei drei vier fünf")
    }

    // (d)
    func testBargeInAfterFirstSentenceCancelsAndMarksInterruption() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, startedAt: 2_000, endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "und wie ist es in Berlin", atMs: 4_500)), [.cancelAnswer(merge: false)])
        XCTAssertEqual(p.feed(.answerFinished(requestId: "r1")), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 5_300)),
                       [.commit(text: "und wie ist es in Berlin", merged: false, afterInterruption: true)])
        // Danach ist wieder alles normal.
        _ = p.feed(.answerStarted(requestId: "r2"))
        _ = p.feed(.answerFinished(requestId: "r2"))
        XCTAssertEqual(committed(say(&p, "Danke dir", at: 9_000)).first.map { [$0.1, $0.2] }, [false, false])
    }

    func testBargeInNeedsTwoWords() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 4_500)), [], "ein Wort: noch nichts")
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 4_850)), [], "keine Vorarbeit, solange die Antwort läuft")
        XCTAssertEqual(p.feed(.partial(text: "Berlin bitte", atMs: 4_900)), [.cancelAnswer(merge: false)])
    }

    // (e)
    func testEchoOfSpokenSentencesIsIgnored() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, sentence: "Das Wetter morgen wird sonnig und warm.", startedAt: 2_000, endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "das wetter morgen wird sonnig", atMs: 4_500)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        XCTAssertEqual(p.feed(.partial(text: "Das Wetter morgen wird sonnig und wärm", atMs: 4_700)).first, .ignored(reason: DuplexTurnPolicy.reasonEcho),
                       "60 % reichen: 5 von 6 Wörtern")
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 5_400)), [], "ignorierter Text wird nie zur Äußerung")
    }

    func testEchoComparesFoldedWords() {
        var p = legacyPolicy()
        _ = say(&p, "Sag etwas", at: 0)
        startAnswer(&p, sentence: "Für Größe und Übung gibt es Äpfel.", startedAt: 2_000, endedAt: nil)
        XCTAssertEqual(p.feed(.partial(text: "FUR GROSSE und UBUNG gibt es", atMs: 3_000)).first, .ignored(reason: DuplexTurnPolicy.reasonEcho))
    }

    func testOldSpokenSentencesDoNotCountAsEcho() {
        var p = legacyPolicy()
        _ = say(&p, "Sag etwas", at: 0)
        _ = p.feed(.answerStarted(requestId: "r1"))
        _ = p.feed(.sentenceSpoken(text: "Das Wetter morgen wird sonnig.", startedAtMs: 1_000, endedAtMs: 3_000))
        _ = p.feed(.sentenceSpoken(text: "Es bleibt trocken.", startedAtMs: 20_000, endedAtMs: nil))
        // 12 s nach dem ersten Satz (Fenster 8 s) und inhaltlich nicht im jüngeren Satz.
        XCTAssertEqual(p.feed(.partial(text: "das wetter morgen wird sonnig", atMs: 15_000)), [.cancelAnswer(merge: false)])
    }

    func testRealSpeechWithLowOverlapIsNoEcho() {
        var p = legacyPolicy()
        _ = say(&p, "Sag etwas", at: 0)
        startAnswer(&p, sentence: "Das Wetter morgen wird sonnig und warm.", startedAt: 2_000, endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "Moment das Wetter interessiert mich gar nicht", atMs: 4_500)), [.cancelAnswer(merge: false)])
    }

    // (f)
    func testBackchannelDuringAnswerIsIgnored() {
        for word in ["mhm", "ja", "okay", "aha", "genau", "Ja", "Okay."] {
            var p = legacyPolicy()
            _ = say(&p, "Erzähl mir etwas", at: 0)
            startAnswer(&p, sentence: "Eine ganz andere Geschichte beginnt hier.", endedAt: 4_000)
            XCTAssertEqual(p.feed(.partial(text: word, atMs: 4_500)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)], word)
            XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 5_300)), [], word)
        }
    }

    func testYesOutsideAnswerIsNormalUtterance() {
        var p = legacyPolicy()
        let actions = say(&p, "Ja", at: 0)
        XCTAssertEqual(committed(actions).first?.0, "Ja")
    }

    // (g)
    func testStopCommandCancelsWithoutCommit() {
        for word in ["Stopp", "halt", "Warte", "Moment", "stopp bitte", "Moment mal kurz"] {
            var p = legacyPolicy()
            _ = say(&p, "Erzähl mir etwas", at: 0)
            startAnswer(&p, sentence: "Eine ganz andere Geschichte beginnt hier.", endedAt: nil)
            XCTAssertEqual(p.feed(.partial(text: word, atMs: 3_000)), [.cancelAnswer(merge: false), .stopCommand], word)
            XCTAssertEqual(p.feed(.partial(text: word + " jetzt", atMs: 3_200)), [], "Rest derselben Äußerung")
            XCTAssertEqual(committed(p.feed(.silence(sinceMs: 700, atMs: 4_000))).count, 0, word)
            // Neue Äußerung danach ist normal.
            XCTAssertEqual(committed(say(&p, "Und jetzt etwas anderes", at: 6_000)).first?.0, "Und jetzt etwas anderes")
        }
    }

    func testStopWordInsideLongSentenceIsNoStop() {
        var p = legacyPolicy()
        _ = say(&p, "Erzähl mir etwas", at: 0)
        startAnswer(&p, sentence: "Eine ganz andere Geschichte beginnt hier.", endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "warte ich möchte noch etwas ergänzen", atMs: 4_500)), [.cancelAnswer(merge: false)])
    }

    func testStopWordOutsideAnswerIsNormalUtterance() {
        var p = legacyPolicy()
        XCTAssertEqual(committed(say(&p, "Stopp", at: 0)).first?.0, "Stopp")
    }

    // (h)
    func testNoSpeculationWhenDisabled() {
        var cfg = DuplexTurnPolicy.Config()
        cfg.speculate = false
        cfg.adaptiveEnd = false
        var p = DuplexTurnPolicy(config: cfg)
        _ = feed(&p, .partial(text: "Wie wird das Wetter", atMs: 1_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 1_350)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 500, atMs: 1_500)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 1_600)), [.commit(text: "Wie wird das Wetter", merged: false, afterInterruption: false)])
    }

    // (i)
    func testHalfDuplexIgnoresSpeechDuringAnswer() {
        var cfg = DuplexTurnPolicy.Config()
        cfg.bargeIn = false
        cfg.adaptiveEnd = false
        var p = DuplexTurnPolicy(config: cfg)
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "und wie ist es in Berlin", atMs: 4_500)), [.ignored(reason: DuplexTurnPolicy.reasonHalfDuplex)])
        XCTAssertEqual(p.feed(.partial(text: "Stopp", atMs: 4_600)), [.ignored(reason: DuplexTurnPolicy.reasonHalfDuplex)])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 5_300)), [])
        _ = p.feed(.answerFinished(requestId: "r1"))
        XCTAssertEqual(committed(say(&p, "Jetzt bitte Berlin", at: 8_000)).first?.0, "Jetzt bitte Berlin")
    }

    // (j)
    func testEmptyOrWhitespaceUtteranceNeverCommits() {
        var p = legacyPolicy()
        XCTAssertEqual(feed(&p, .speechStarted(atMs: 0), .partial(text: "", atMs: 100), .silence(sinceMs: 700, atMs: 800)), [])
        XCTAssertEqual(feed(&p, .partial(text: "   \n ", atMs: 900), .silence(sinceMs: 350, atMs: 1_250), .silence(sinceMs: 700, atMs: 1_600)), [])
        XCTAssertEqual(feed(&p, .partial(text: "...", atMs: 1_700), .silence(sinceMs: 700, atMs: 2_400)), [], "nur Satzzeichen")
    }

    // (k)
    func testSingleWordInterruptionCountsOnlyAtUtteranceEnd() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 4_500)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 4_850)), [])
        // Die Einheit klingt noch: das Wort bleibt stehen und wird erst am Klangende verbraucht (Zustandsmatrix Zeile 3).
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 5_100)), [])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 5_200)),
                       [.cancelAnswer(merge: false), .commit(text: "Berlin", merged: false, afterInterruption: true)])
    }

    func testSingleWordBeforeFirstSentenceEndsMergesAtUtteranceEnd() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter", at: 0)
        _ = p.feed(.answerStarted(requestId: "r1"))
        // Zustandsmatrix Zeile 2: Klingt keine Einheit, ist schon ein relevantes Wort eine Fortsetzung (früher erst am Äußerungsende).
        XCTAssertEqual(p.feed(.partial(text: "morgen", atMs: 2_000)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 2_700)),
                       [.commit(text: "Wie wird das Wetter morgen", merged: true, afterInterruption: false)])
    }

    func testSingleWordThatEchoesIsStillIgnored() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, sentence: "Das Wetter morgen wird sonnig.", endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "sonnig", atMs: 4_300)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
    }

    func testSingleWordThatEndsWhileAnswerFinishedIsNormalUtterance() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, sentence: "Das Wetter morgen wird sonnig.", endedAt: 4_000)
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 4_500)), [])
        _ = p.feed(.answerFinished(requestId: "r1"))
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 5_100)), [.commit(text: "Berlin", merged: false, afterInterruption: false)])
    }

    // Weiteres

    func testEchoTailRightAfterAnswerIsIgnoredButLaterSpeechIsNot() {
        var p = legacyPolicy()
        _ = say(&p, "Wie wird das Wetter morgen", at: 0)
        startAnswer(&p, sentence: "Das Wetter morgen wird sonnig und warm.", startedAt: 2_000, endedAt: 4_000)
        _ = p.feed(.answerFinished(requestId: "r1"))
        XCTAssertEqual(p.feed(.partial(text: "wetter morgen wird sonnig", atMs: 4_600)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        _ = p.feed(.silence(sinceMs: 700, atMs: 5_300))
        XCTAssertEqual(p.feed(.partial(text: "wetter morgen wird sonnig", atMs: 9_000)), [], "lange nach dem Satz ist es Nutzerrede")
    }

    func testLateAnswerFinishedOfDiscardedRunDoesNotEndNewAnswer() {
        var p = legacyPolicy()
        _ = feed(&p, .partial(text: "Wie wird das Wetter", atMs: 1_000), .silence(sinceMs: 350, atMs: 1_350), .answerStarted(requestId: "spec1"))
        XCTAssertEqual(p.feed(.partial(text: "Wie wird das Wetter morgen", atMs: 1_400)), [.discardSpeculation])
        _ = p.feed(.silence(sinceMs: 650, atMs: 2_100))      // commit
        _ = p.feed(.answerStarted(requestId: "r2"))
        _ = p.feed(.answerFinished(requestId: "spec1"))      // verspätet
        _ = p.feed(.sentenceSpoken(text: "Es wird sonnig.", startedAtMs: 3_000, endedAtMs: 4_500))
        XCTAssertEqual(p.feed(.partial(text: "und wie ist es in Berlin", atMs: 5_000)), [.cancelAnswer(merge: false)])
    }

    func testSpeculativeRunPromotedAtCommitStaysActive() {
        var p = legacyPolicy()
        _ = feed(&p, .partial(text: "Wie wird das Wetter", atMs: 1_000), .silence(sinceMs: 350, atMs: 1_350), .answerStarted(requestId: "spec1"))
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 1_600)), [.commit(text: "Wie wird das Wetter", merged: false, afterInterruption: false)])
        // Vorarbeit wird zur Antwort: Ergänzung vor dem ersten Satz führt zusammen.
        XCTAssertEqual(p.feed(.partial(text: "morgen in Berlin", atMs: 2_000)), [.cancelAnswer(merge: true)])
    }

    func testStopThenNewUtteranceAfterStopWithoutSilenceEventsStillWorks() {
        var p = legacyPolicy()
        _ = say(&p, "Erzähl", at: 0)
        startAnswer(&p, endedAt: nil)
        XCTAssertEqual(p.feed(.partial(text: "Stopp", atMs: 3_000)), [.cancelAnswer(merge: false), .stopCommand])
        _ = p.feed(.silence(sinceMs: 650, atMs: 3_700))
        XCTAssertEqual(committed(say(&p, "Nächste Frage bitte", at: 5_000)).first?.0, "Nächste Frage bitte")
    }

    func testResetForgetsEverything() {
        var p = legacyPolicy()
        _ = say(&p, "Eins", at: 0)
        startAnswer(&p, endedAt: 4_000)
        p.reset()
        XCTAssertEqual(p.feed(.partial(text: "und wie ist es in Berlin", atMs: 4_500)), [], "keine Antwort mehr aktiv")
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 650, atMs: 5_200))).first.map { [$0.1, $0.2] }, [false, false])
    }

    func testPunctuationAndCaseDoNotMatterForEcho() {
        var p = legacyPolicy()
        _ = say(&p, "Sag etwas", at: 0)
        startAnswer(&p, sentence: "Zeit, Geld & Muße: das reicht!", startedAt: 2_000, endedAt: 3_000)
        XCTAssertEqual(p.feed(.partial(text: "ZEIT GELD MUSSE das reicht", atMs: 3_200)).first, .ignored(reason: DuplexTurnPolicy.reasonEcho))
    }
}
