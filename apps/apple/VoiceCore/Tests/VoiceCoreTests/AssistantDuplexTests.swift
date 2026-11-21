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
    func testAddingBeforeFirstToneMergesUtterances() {
        var p = legacyPolicy()
        XCTAssertEqual(committed(say(&p, "Wie wird das Wetter morgen", at: 0)).first?.0, "Wie wird das Wetter morgen")
        _ = p.feed(.answerStarted(requestId: "r1"))
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

    func testDelayedWordEndingCommitsOnlyTheFinalText() {
        var p = legacyPolicy()
        var u = AssistantUtterance()
        u.apply("Alles Id", start: 0, end: 1, isFinal: false)
        _ = p.feed(.partial(text: u.text, atMs: 1_000, stableText: u.stable))
        _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
        let quiet = AssistantSilenceGate.reported(
            silenceMs: 1_300, lastLoudAt: 1.4, recognizedUntil: u.latestEnd)
        XCTAssertTrue(committed(p.feed(.silence(sinceMs: quiet, atMs: 2_700))).isEmpty)
        u.apply("Alles Idioten", start: 0, end: 1.4, isFinal: true)
        _ = p.feed(.partial(text: u.text, atMs: 2_800, stableText: u.stable))
        _ = p.feed(.recognition(caughtUp: true, atMs: 2_800))
        let commits = committed(p.feed(.silence(sinceMs: 1_500, atMs: 2_900)))
        XCTAssertEqual(commits.map { $0.0 }, ["Alles Idioten"])
        _ = u.consume(at: 2.9)
        XCTAssertFalse(u.apply("Alles Idioten", start: 0, end: 2.92, isFinal: true))
        XCTAssertTrue(u.isEmpty, "Kein Rest und kein zweiter Versand")
        XCTAssertTrue(committed(p.feed(.silence(sinceMs: 1_600, atMs: 3_000))).isEmpty)
    }

    func testGraceLimitUsesConfirmedTextEvenWithVolatileEnding() {
        for caughtUp in [false, true] {
            var p = legacyPolicy()
            _ = p.feed(.partial(text: "Alles Id", atMs: 1_000, stableText: "Alles"))
            _ = p.feed(.recognition(caughtUp: caughtUp, atMs: 1_100))
            XCTAssertTrue(committed(p.feed(.silence(sinceMs: 3_199, atMs: 4_199))).isEmpty)
            XCTAssertEqual(
                committed(p.feed(.silence(sinceMs: 3_200, atMs: 4_200))).map { $0.0 },
                ["Alles"])
        }
    }

    func testContinuationMergesBeforeToneButStartsNewUtteranceAfterTone() {
        for heard in [false, true] {
            var p = legacyPolicy()
            _ = say(&p, "Meine Frage", at: 0)
            _ = p.feed(.answerStarted(requestId: "r1"))
            if heard { _ = p.feed(.unitStarted(atMs: 1_200)) }
            XCTAssertEqual(p.feed(.partial(text: "und weiter", atMs: 1_300)),
                           [.cancelAnswer(merge: !heard)])
            let result = committed(p.feed(.silence(sinceMs: 600, atMs: 1_900)))
            XCTAssertEqual(result.map { $0.0 }, [heard ? "und weiter" : "Meine Frage und weiter"])
            XCTAssertEqual(result.first?.2, heard)
        }
    }

    func testConsumedCorrectionReplacesPrefixWithoutRepeatingWords() {
        var p = legacyPolicy()
        _ = say(&p, "Alles Id", at: 0)
        _ = p.feed(.answerStarted(requestId: "r1"))
        XCTAssertEqual(p.feed(.correction(text: "Alles Idioten", atMs: 1_200)),
                       [.cancelAnswer(merge: true)])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 600, atMs: 1_800))).map { $0.0 },
                       ["Alles Idioten"], "Auch eine reine Korrektur startet die Antwort neu")
    }

    func testLateFinalCorrectsConsumedWordsAndKeepsAdditionalWordOnce() {
        for heard in [false, true] {
            var p = legacyPolicy()
            var u = AssistantUtterance()
            u.apply("Alles Id", start: 0, end: 1, isFinal: false)
            _ = say(&p, u.text, at: 0)
            XCTAssertEqual(u.consume(at: 1.1), "Alles Id")
            _ = p.feed(.answerStarted(requestId: "r1"))
            if heard { _ = p.feed(.unitStarted(atMs: 1_200)) }

            XCTAssertTrue(u.apply("Alles Idioten bitte", start: 0, end: 1,
                                  isFinal: true))
            XCTAssertEqual(u.correction, "Alles Idioten")
            XCTAssertEqual(u.text, "bitte")
            XCTAssertEqual(p.feed(.correction(text: u.correction ?? "", atMs: 1_300)),
                           heard ? [] : [.cancelAnswer(merge: true)])
            XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 1_300,
                                           stableText: u.stable)), [])
            var actions = p.feed(.silence(sinceMs: 600, atMs: 1_900))
            if heard { actions += p.feed(.unitEnded(atMs: 2_000)) }
            let result = committed(actions)
            XCTAssertEqual(result.map { $0.0 }, [heard ? "bitte" : "Alles Idioten bitte"])
            XCTAssertEqual(result.first?.1, !heard)
            XCTAssertEqual(result.first?.2, heard)
            XCTAssertEqual(u.consume(at: 2), "bitte")
            XCTAssertFalse(u.apply("Alles Idioten bitte", start: 0, end: 1,
                                   isFinal: true))
            XCTAssertTrue(u.isEmpty)
            XCTAssertTrue(committed(p.feed(.silence(sinceMs: 900, atMs: 2_300))).isEmpty)
        }
    }

    func testDeferredTailSurvivesEchoOfTheConfirmedPrefix() {
        for sentence in ["Das Wetter wird sonnig", "Das Wetter wird morgen sonnig"] {
            var p = legacyPolicy()
            var u = AssistantUtterance()
            u.apply("Wie wird das Wetter", start: 0, end: 0.8, isFinal: true)
            u.apply("morgen", start: 0.9, end: 1, isFinal: false)
            _ = p.feed(.partial(text: u.text, atMs: 1_000, stableText: u.stable))
            _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
            XCTAssertEqual(
                committed(p.feed(.silence(sinceMs: 3_200, atMs: 4_200))).map { $0.0 },
                ["Wie wird das Wetter"])
            _ = u.consume(at: 4.2)
            startAnswer(&p, sentence: sentence, startedAt: 4_250)
            XCTAssertTrue(u.apply(sentence, start: 4.25, end: 4.3, isFinal: false))
            XCTAssertNil(u.correction)
            XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 4_300)),
                           [.ignored(reason: DuplexTurnPolicy.reasonEcho)])

            XCTAssertFalse(u.apply("Wie wird das Wetter morgen", start: 0, end: 1,
                                   isFinal: true))
            XCTAssertEqual(u.correction, "Wie wird das Wetter morgen")
            let correction = u.correction ?? ""
            XCTAssertEqual(p.feed(.correction(text: correction, atMs: 4_400)), [])
            XCTAssertEqual(p.feed(.correction(text: correction, atMs: 4_450)), [])
            _ = p.feed(.recognition(caughtUp: true, atMs: 4_500))
            XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 5_000)), [])
            XCTAssertEqual(p.feed(.unitEnded(atMs: 5_100)), [
                .cancelAnswer(merge: false),
                .commit(text: "morgen", merged: false, afterInterruption: true),
            ])
            XCTAssertEqual(p.feed(.correction(text: correction, atMs: 5_200)), [])
            XCTAssertTrue(committed(p.feed(.silence(sinceMs: 900, atMs: 5_400))).isEmpty)
        }
    }

    func testDeferredTailKeepsRepeatedFinalEchoOutAndAllowsRealContinuation() {
        let echo = "Das Wetter wird sonnig"
        for finalEcho in [echo, echo + " und warm"] {
            for continues in [false, true] {
                var p = legacyPolicy()
                var u = AssistantUtterance()
                u.apply("Wie wird das Wetter", start: 0, end: 0.8, isFinal: true)
                u.apply("morgen", start: 0.9, end: 1, isFinal: false)
                _ = p.feed(.partial(text: u.text, atMs: 1_000, stableText: u.stable))
                _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
                XCTAssertEqual(
                    committed(p.feed(.silence(sinceMs: 3_200, atMs: 4_200))).map { $0.0 },
                    ["Wie wird das Wetter"])
                _ = u.consume(at: 4.2)
                startAnswer(&p, sentence: echo + " und warm", startedAt: 4_250)

                XCTAssertTrue(u.apply(echo, start: 4.25, end: 4.3, isFinal: false))
                XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 4_300,
                                               stableText: u.stable)),
                               [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
                XCTAssertFalse(u.apply("Wie wird das Wetter morgen", start: 0, end: 1,
                                       isFinal: true))
                XCTAssertEqual(u.correction, "Wie wird das Wetter morgen")
                XCTAssertEqual(p.feed(.correction(text: u.correction ?? "", atMs: 4_400)), [])
                let visible = p.liveParts(stable: u.stable, volatile: u.volatile)
                XCTAssertTrue(visible.stable.isEmpty && visible.volatile.isEmpty,
                              "Der interne leere Aufruf darf das Echopräfix nicht freigeben")

                // Dasselbe Echo kommt erst nach der alten Final-Korrektur bestätigt zurück.
                XCTAssertTrue(u.apply(finalEcho, start: 4.25, end: 4.5, isFinal: true))
                XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 4_500,
                                               stableText: u.stable)),
                               finalEcho == echo ? [] : [.ignored(
                                   reason: DuplexTurnPolicy.reasonEcho)],
                               "Auch zusätzliche Echowörter dürfen die Antwort nicht abbrechen")
                if continues {
                    // Echter neuer Text hinter dem verbrauchten Echo bleibt erhalten.
                    XCTAssertTrue(u.apply("früh", start: 4.6, end: 4.8, isFinal: true))
                    XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 4_800,
                                                   stableText: u.stable)),
                                   [.cancelAnswer(merge: false)])
                }
                _ = p.feed(.recognition(caughtUp: true, atMs: 4_800))
                let quiet = p.feed(.silence(sinceMs: 600, atMs: 5_400))
                if continues {
                    XCTAssertEqual(quiet, [.commit(text: "morgen früh", merged: false,
                                                   afterInterruption: true)])
                    XCTAssertEqual(p.feed(.unitEnded(atMs: 5_500)), [])
                } else {
                    XCTAssertEqual(quiet, [], "Ein Wort wartet bis zum tatsächlichen Klangende")
                    XCTAssertEqual(p.feed(.unitEnded(atMs: 5_500)), [
                        .cancelAnswer(merge: false),
                        .commit(text: "morgen", merged: false, afterInterruption: true),
                    ])
                }
                XCTAssertTrue(committed(p.feed(.silence(
                    sinceMs: 900, atMs: 5_700))).isEmpty, "Kein zweiter Versand")
            }
        }
    }

    func testDeferredWordEndingSurvivesMatchingSpokenEcho() {
        var p = legacyPolicy()
        var u = AssistantUtterance()
        u.apply("Wie wird das Wetter", start: 0, end: 0.8, isFinal: true)
        u.apply("mor", start: 0.9, end: 1, isFinal: false)
        _ = p.feed(.partial(text: u.text, atMs: 1_000, stableText: u.stable))
        _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
        XCTAssertEqual(
            committed(p.feed(.silence(sinceMs: 3_200, atMs: 4_200))).map { $0.0 },
            ["Wie wird das Wetter"])
        _ = u.consume(at: 4.2)
        startAnswer(&p, sentence: "Morgen wird es sonnig", startedAt: 4_250)
        XCTAssertFalse(u.apply("Wie wird das Wetter morgen", start: 0, end: 1,
                               isFinal: true))
        XCTAssertEqual(u.correction, "Wie wird das Wetter morgen")
        XCTAssertEqual(p.feed(.correction(text: u.correction ?? "", atMs: 4_400)), [])
        _ = p.feed(.recognition(caughtUp: true, atMs: 4_500))
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 5_000)), [])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 5_100)), [
            .cancelAnswer(merge: false),
            .commit(text: "morgen", merged: false, afterInterruption: true),
        ])
        XCTAssertEqual(p.feed(.correction(
            text: "Wie wird das Wetter morgen", atMs: 5_200)), [])

        // Ohne vorgemerkten Rest bleibt dasselbe Wort reines Echo.
        var echo = legacyPolicy()
        _ = say(&echo, "Wie wird das Wetter", at: 0)
        startAnswer(&echo, sentence: "Morgen wird es sonnig", startedAt: 4_250)
        XCTAssertEqual(echo.feed(.correction(
            text: "Wie wird das Wetter morgen", atMs: 4_400)), [])
        XCTAssertEqual(echo.feed(.partial(text: "morgen", atMs: 4_450)),
                       [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        XCTAssertEqual(echo.feed(.unitEnded(atMs: 5_100)), [])
        XCTAssertTrue(committed(echo.feed(.silence(sinceMs: 900, atMs: 5_350))).isEmpty)
    }

    func testFinalEndingAfterGraceContinuesTheConfirmedPrefix() {
        var p = legacyPolicy()
        var u = AssistantUtterance()
        u.apply("Alles", start: 0, end: 0.5, isFinal: true)
        u.apply("Idioten", start: 0.6, end: 1, isFinal: false)
        _ = p.feed(.partial(text: u.text, atMs: 1_000, stableText: u.stable))
        _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
        _ = p.feed(.silence(sinceMs: 3_200, atMs: 4_200))
        _ = u.consume(at: 4.2)
        u.apply("Idioten", start: 0.6, end: 1, isFinal: true)
        XCTAssertEqual(u.correction, "Alles Idioten")
        XCTAssertEqual(p.feed(.correction(text: u.correction ?? "", atMs: 4_300)),
                       [.cancelAnswer(merge: true)])
        _ = p.feed(.recognition(caughtUp: true, atMs: 4_300))
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 600, atMs: 4_900))).map { $0.0 },
                       ["Alles Idioten"])
    }

    func testCorrectionAfterMergedCommitKeepsTheEarlierPrefix() {
        var p = legacyPolicy()
        _ = say(&p, "Meine Frage", at: 0)
        _ = p.feed(.partial(text: "Alles Id", atMs: 1_300))
        _ = p.feed(.silence(sinceMs: 600, atMs: 1_900))
        XCTAssertEqual(p.feed(.correction(text: "Alles Idioten", atMs: 2_000)),
                       [.cancelAnswer(merge: true)])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 600, atMs: 2_600))).map { $0.0 },
                       ["Meine Frage Alles Idioten"])
    }

    func testDeferredFinalTailSurvivesFirstToneAndIsCommittedExactlyOnce() {
        for heard in [false, true] {
            for cumulative in [false, true] {
                var p = legacyPolicy()
                var u = AssistantUtterance()
                u.apply("Alles", start: 0, end: 0.5, isFinal: true)
                u.apply("Idioten", start: 0.6, end: 1, isFinal: false)
                _ = p.feed(.partial(text: u.text, atMs: 1_000, stableText: u.stable))
                _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
                XCTAssertEqual(
                    committed(p.feed(.silence(sinceMs: 3_200, atMs: 4_200))).map { $0.0 },
                    ["Alles"])
                _ = u.consume(at: 4.2)
                _ = p.feed(.answerStarted(requestId: "r1"))
                if heard { _ = p.feed(.unitStarted(atMs: 4_250)) }
                XCTAssertFalse(u.apply(
                    cumulative ? "Alles Idioten" : "Idioten",
                    start: cumulative ? 0 : 0.6, end: 1, isFinal: true))
                XCTAssertEqual(u.correction, "Alles Idioten")
                let correction = u.correction ?? ""
                XCTAssertEqual(p.feed(.correction(text: correction, atMs: 4_300)),
                               heard ? [] : [.cancelAnswer(merge: true)])
                XCTAssertEqual(p.feed(.correction(text: correction, atMs: 4_350)), [],
                               "Kein doppelter Rest")
                _ = p.feed(.recognition(caughtUp: true, atMs: 4_300))
                var actions = p.feed(.silence(sinceMs: 600, atMs: 4_900))
                if heard { actions += p.feed(.unitEnded(atMs: 5_000)) }
                let result = committed(actions)
                XCTAssertEqual(result.map { $0.0 }, [heard ? "Idioten" : "Alles Idioten"])
                XCTAssertEqual(result.first?.1, !heard)
                XCTAssertEqual(result.first?.2, heard)
                XCTAssertEqual(p.feed(.correction(text: correction, atMs: 5_100)), [])
                XCTAssertTrue(committed(p.feed(.silence(sinceMs: 900, atMs: 5_200))).isEmpty)
                XCTAssertTrue(u.isEmpty, "Der Rest kommt ausschließlich über die Korrektur")
            }
        }
    }

    func testConsumedCorrectionDropsTheSameIgnoredPrefixAsTheCommittedQuestion() {
        for ignored in ["ja", "Das Wetter"] {
            var p = legacyPolicy()
            var u = AssistantUtterance()
            _ = say(&p, "Meine Frage", at: 0)
            _ = p.feed(.answerStarted(requestId: "r1"))
            if ignored != "ja" {
                _ = p.feed(.sentenceSpoken(
                    text: ignored, startedAtMs: 1_200, endedAtMs: nil))
            }
            u.apply(ignored, start: 0, end: 0.3, isFinal: false)
            XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 1_300)), [.ignored(
                reason: ignored == "ja"
                    ? DuplexTurnPolicy.reasonBackchannel : DuplexTurnPolicy.reasonEcho)])
            u.apply(ignored + " Alles Id", start: 0, end: 1, isFinal: false)
            XCTAssertEqual(p.feed(.partial(text: u.text, atMs: 1_500)),
                           [.cancelAnswer(merge: true)])
            XCTAssertEqual(
                committed(p.feed(.silence(sinceMs: 600, atMs: 2_100))).map { $0.0 },
                ["Meine Frage Alles Id"])
            _ = u.consume(at: 2.1)
            XCTAssertFalse(u.apply(
                ignored + " Alles Idioten", start: 0, end: 1, isFinal: true))
            XCTAssertEqual(u.correction, ignored + " Alles Idioten")
            let correction = u.correction ?? ""
            XCTAssertEqual(p.feed(.correction(text: correction, atMs: 2_200)),
                           [.cancelAnswer(merge: true)])
            XCTAssertEqual(p.feed(.correction(text: correction, atMs: 2_250)), [])
            XCTAssertEqual(
                committed(p.feed(.silence(sinceMs: 600, atMs: 2_800))).map { $0.0 },
                ["Meine Frage Alles Idioten"], "Ohne ignoriertes Präfix und ohne Doppelung")
        }
    }

    func testDeferredTailStaysBeforeLaterRecognizerWords() {
        var p = legacyPolicy()
        _ = p.feed(.partial(text: "Alles Idioten", atMs: 1_000, stableText: "Alles"))
        _ = p.feed(.recognition(caughtUp: false, atMs: 1_100))
        _ = p.feed(.silence(sinceMs: 3_200, atMs: 4_200))
        _ = p.feed(.answerStarted(requestId: "r1"))
        _ = p.feed(.unitStarted(atMs: 4_250))
        XCTAssertEqual(p.feed(.correction(text: "Alles Idioten", atMs: 4_300)), [])
        XCTAssertEqual(p.feed(.partial(
            text: "und weiter", atMs: 4_400, stableText: "und weiter")),
            [.cancelAnswer(merge: false)])
        _ = p.feed(.recognition(caughtUp: true, atMs: 4_500))
        XCTAssertEqual(p.feed(.silence(sinceMs: 600, atMs: 5_000)),
                       [.commit(text: "Idioten und weiter", merged: false,
                                afterInterruption: true)])
        XCTAssertEqual(p.feed(.correction(text: "Alles Idioten", atMs: 5_100)), [])
        XCTAssertTrue(committed(p.feed(.silence(sinceMs: 900, atMs: 5_300))).isEmpty)
    }

    func testCumulativeEchoCorrectionsStayOutOfTheLiveLine() {
        var p = legacyPolicy()
        startAnswer(&p, sentence: "Das Wetter wird sonnig.", endedAt: 4_000)
        _ = p.feed(.partial(text: "Das Wetter wird sonnig", atMs: 4_200))
        _ = p.feed(.partial(text: "Das Wetter wird sonniger", atMs: 4_300))
        let visible = p.liveParts(stable: "", volatile: "Das Wetter wird sonniger")
        XCTAssertTrue(visible.stable.isEmpty && visible.volatile.isEmpty)
        XCTAssertTrue(committed(p.feed(.silence(sinceMs: 3_200, atMs: 7_500))).isEmpty)
        XCTAssertEqual(p.feed(.correction(text: "Das Wetter wird sonniger", atMs: 7_600)), [])
    }

    func testUnitEndDoesNotCommitWhileRecognitionLags() {
        var p = legacyPolicy()
        _ = say(&p, "Meine Frage", at: 0)
        startAnswer(&p, endedAt: nil)
        _ = p.feed(.partial(text: "Berlin", atMs: 2_500))
        _ = p.feed(.recognition(caughtUp: false, atMs: 2_600))
        _ = p.feed(.silence(sinceMs: 600, atMs: 3_100))
        XCTAssertEqual(p.feed(.unitEnded(atMs: 3_200)), [.cancelAnswer(merge: false)])
        _ = p.feed(.recognition(caughtUp: true, atMs: 3_300))
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: 800, atMs: 3_300))).map { $0.0 },
                       ["Berlin"])
    }

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
