import Foundation
import XCTest
@testable import VoiceCore

// Natürlicher Gesprächsmodus, Teil A: Äußerungsende nach Inhalt (`UtteranceShape`, adaptive Stille) und Zurückhalten der
// Antwort, solange der Nutzer spricht (`.holdAnswer` / `.releaseAnswer`). Reine Logik, ohne Audio und ohne Uhr.

final class AssistantTurnShapeTests: XCTestCase {
    private typealias Action = DuplexTurnPolicy.Action
    private typealias Input = DuplexTurnPolicy.Input

    private func feed(_ p: inout DuplexTurnPolicy, _ inputs: Input...) -> [Action] {
        inputs.flatMap { p.feed($0) }
    }

    private let defaults = DuplexTurnPolicy.Config()

    private func scaled(_ ms: Int, _ factor: Double) -> Int { Int((Double(ms) * factor).rounded()) }

    private func committed(_ actions: [Action]) -> [String] {
        actions.compactMap { if case .commit(let t, _, _) = $0 { return t } else { return nil } }
    }

    // MARK: Form der Äußerung

    func testOpenGerman() {
        let samples = [
            "Ich möchte wissen, ob", "Ich habe einen Termin mit", "Sag mir bitte, dass", "Das geht nicht, weil",
            "Wir treffen uns in", "Gib mir bitte den", "Das ist meine", "Also ähm", "Ich suche etwas für",
            "Erzähl mir etwas über", "Ich wollte noch fragen, und", "Ich will", "Das Buch von", "Wie viel kostet das, wenn",
            "Ich überlege,", "Moment…", "Ich bin mir nicht sicher -", "Folgendes:", "Hmm", "Der Termin ist",
        ]
        for text in samples { XCTAssertEqual(UtteranceShape.classify(text), .open, text) }
    }

    func testOpenEnglish() {
        for text in ["I want to know if", "Tell me about the", "I would like bread and", "She said that"] {
            XCTAssertEqual(UtteranceShape.classify(text), .open, text)
        }
    }

    func testShortUtterancesAreOpenExceptKnownOneWordReplies() {
        for text in ["Wie spät", "Wetter morgen", "Berlin", "Danke schön", "Guten Morgen", "Mach weiter", "Wie bitte"] {
            XCTAssertEqual(UtteranceShape.classify(text), .open, text)
        }
        for text in ["ja", "Nein.", "doch", "Danke", "Hallo!", "Stopp", "stop", "weiter", "Genau", "Okay.", "ok", "richtig", "Falsch?"] {
            XCTAssertEqual(UtteranceShape.classify(text), .complete, text)
        }
    }

    func testTwoWordsWithClosingPunctuationAreComplete() {
        for text in ["Wie spät?", "Guten Morgen.", "Und dann?", "Mach weiter.", "Wie bitte?", "Sehr gut!", "Lies vor."] {
            XCTAssertEqual(UtteranceShape.classify(text), .complete, text)
        }
        // Die Regeln zum letzten Wort haben Vorrang.
        for text in ["Ich möchte.", "Und?", "Mit dir und.", "Wie ist das,"] {
            XCTAssertEqual(UtteranceShape.classify(text), .open, text)
        }
    }

    func testSingleWordWithClosingPunctuationIsNeutralUnlessKnownReply() {
        for text in ["Berlin.", "Wetter?", "Morgen!", "Rom."] {
            XCTAssertEqual(UtteranceShape.classify(text), .neutral, text)
        }
        XCTAssertEqual(UtteranceShape.classify("Danke."), .complete)
        XCTAssertEqual(UtteranceShape.classify("Berlin"), .open, "ohne Satzzeichen bleibt ein einzelnes Wort offen")
    }

    func testCompleteGerman() {
        let samples = [
            "Wie wird das Wetter morgen?", "Erzähl mir etwas über Rom.", "Ich möchte eine Notiz anlegen.",
            "Was steht in meiner Besprechung vom Montag?", "Das ist wirklich großartig!",             "Wie spät ist es jetzt?", "Such mir bitte die letzte Sprachnotiz heraus.", "Fass das Kapitel kurz zusammen!",
            "Lies mir das dritte Kapitel vor.", "Mach bitte das Licht an.", "Weißt du, wie spät es ist?", "Was ist das?",
        ]
        for text in samples { XCTAssertEqual(UtteranceShape.classify(text), .complete, text) }
    }

    func testCompleteEnglish() {
        for text in ["What is the weather tomorrow?", "Please read the third chapter.", "That sounds great!"] {
            XCTAssertEqual(UtteranceShape.classify(text), .complete, text)
        }
    }

    func testNeutralWithoutPunctuationAndNaturalEnding() {
        let samples = [
            "Wie wird das Wetter morgen", "Erzähl mir etwas über Rom", "Ich möchte eine Notiz anlegen",
            "Was steht in meiner Besprechung vom Montag", "Lies mir das dritte Kapitel bitte", "Wie spät ist es jetzt",
            "Such mir die letzte Sprachnotiz heraus", "Fass das Kapitel kurz zusammen",
        ]
        for text in samples { XCTAssertEqual(UtteranceShape.classify(text), .neutral, text) }
        for text in ["What is the weather tomorrow", "Please read the third chapter", "Tell me about Rome"] {
            XCTAssertEqual(UtteranceShape.classify(text), .neutral, text)
        }
    }

    func testWeakEndingsAreOpenOnlyWithoutClosingPunctuation() {
        for text in ["Lies mir das dritte Kapitel vor", "Mach bitte das Licht an", "Das Wetter morgen ist", "Was ist das", "Ich weiß nicht, was er kann"] {
            XCTAssertEqual(UtteranceShape.classify(text), .open, text)
        }
    }

    func testOpenWinsOverCompleteAndTrailingSpacesDoNotMatter() {
        for text in ["Ich möchte wissen, ob.", "Das reicht und.", "Ich habe es gesagt, weil?", "Das Buch von!", "Ich möchte wissen, ob  \n"] {
            XCTAssertEqual(UtteranceShape.classify(text), .open, text)
        }
        XCTAssertEqual(UtteranceShape.classify("Wie wird das Wetter morgen?  "), .complete)
    }

    // MARK: Ende nach Inhalt

    func testDefaultsOfConfig() {
        let c = DuplexTurnPolicy.Config()
        XCTAssertTrue(c.adaptiveEnd)
        XCTAssertEqual([c.endCompleteMs, c.endNeutralMs, c.endOpenMs], [900, 1_200, 2_000])
        XCTAssertEqual(c.pauseScale, 1.0)
        XCTAssertEqual([c.holdQuietMs, c.holdNoticeMs], [700, 6_000])
        XCTAssertEqual(c.endAfterMs, 600)
    }

    /// Spricht `text` (Sprachbeginn 0, letztes Teilergebnis bei 500) und liefert die Aktionen bei Stille `since` ms.
    private func commitsAt(_ since: Int, _ text: String, config: DuplexTurnPolicy.Config = .init()) -> Bool {
        var p = DuplexTurnPolicy(config: config)
        _ = feed(&p, .speechStarted(atMs: 0), .partial(text: text, atMs: 500))
        return !committed(p.feed(.silence(sinceMs: since, atMs: 500 + since))).isEmpty
    }

    func testEndsAfterCompleteNeutralOpenSilence() {
        let c = defaults
        XCTAssertFalse(commitsAt(c.endCompleteMs - 1, "Wie wird das Wetter morgen?"))
        XCTAssertTrue(commitsAt(c.endCompleteMs, "Wie wird das Wetter morgen?"))
        XCTAssertFalse(commitsAt(c.endNeutralMs - 1, "Wie wird das Wetter morgen"))
        XCTAssertTrue(commitsAt(c.endNeutralMs, "Wie wird das Wetter morgen"))
        XCTAssertFalse(commitsAt(c.endOpenMs - 1, "Ich möchte wissen, ob"))
        XCTAssertTrue(commitsAt(c.endOpenMs, "Ich möchte wissen, ob"))
        XCTAssertLessThan(c.endCompleteMs, c.endNeutralMs)
        XCTAssertLessThan(c.endNeutralMs, c.endOpenMs)
    }

    func testPauseScaleStretchesAndShortensAllThresholds() {
        let c = defaults
        for (factor, text, base) in [(1.5, "Wie wird das Wetter morgen?", c.endCompleteMs), (1.5, "Wie wird das Wetter morgen", c.endNeutralMs),
                                     (1.5, "Ich möchte wissen, ob", c.endOpenMs), (0.8, "Wie wird das Wetter morgen?", c.endCompleteMs),
                                     (0.8, "Ich möchte wissen, ob", c.endOpenMs)] {
            var config = c
            config.pauseScale = factor
            let limit = scaled(base, factor)
            XCTAssertFalse(commitsAt(limit - 1, text, config: config), "\(factor) \(text)")
            XCTAssertTrue(commitsAt(limit, text, config: config), "\(factor) \(text)")
        }
    }

    func testNonAdaptiveEndUsesEndAfterMsForEveryShape() {
        var c = DuplexTurnPolicy.Config()
        c.adaptiveEnd = false
        for text in ["Ich möchte wissen, ob", "Wie wird das Wetter morgen", "Wie wird das Wetter morgen?"] {
            XCTAssertFalse(commitsAt(599, text, config: c), text)
            XCTAssertTrue(commitsAt(600, text, config: c), text)
        }
        c.endAfterMs = 900
        XCTAssertFalse(commitsAt(899, "Wie wird das Wetter morgen?", config: c))
        XCTAssertTrue(commitsAt(900, "Wie wird das Wetter morgen?", config: c))
    }

    func testNoSpeculationOnOpenUtteranceButOnOthers() {
        var p = DuplexTurnPolicy()
        _ = feed(&p, .speechStarted(atMs: 0), .partial(text: "Ich möchte wissen, ob", atMs: 500))
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 850)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 1_000, atMs: 1_500)), [])
        var q = DuplexTurnPolicy()
        _ = feed(&q, .speechStarted(atMs: 0), .partial(text: "Wie wird das Wetter morgen", atMs: 500))
        XCTAssertEqual(q.feed(.silence(sinceMs: 350, atMs: 850)), [.speculate(text: "Wie wird das Wetter morgen")])
        var r = DuplexTurnPolicy()
        _ = feed(&r, .speechStarted(atMs: 0), .partial(text: "Wie wird das Wetter morgen?", atMs: 500))
        XCTAssertEqual(r.feed(.silence(sinceMs: 350, atMs: 850)), [.speculate(text: "Wie wird das Wetter morgen?")])
    }

    func testThinkingPauseAfterOpenEndingDoesNotEndTheUtterance() {
        var p = DuplexTurnPolicy()
        _ = feed(&p, .speechStarted(atMs: 0), .partial(text: "Ich möchte wissen, ob", atMs: 1_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 1_200, atMs: 2_200)), [], "Denkpause von 1,2 s")
        XCTAssertGreaterThan(defaults.endOpenMs, 1_200)
        _ = feed(&p, .speechStarted(atMs: 2_300), .partial(text: "Ich möchte wissen, ob es morgen regnet", atMs: 3_300))
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endNeutralMs - 1, atMs: 4_399)), [.speculate(text: "Ich möchte wissen, ob es morgen regnet")])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: defaults.endNeutralMs, atMs: 4_400))), ["Ich möchte wissen, ob es morgen regnet"])
    }

    func testSpeculationDiscardedWhenUtteranceContinuesIntoOpenEnding() {
        var p = DuplexTurnPolicy()
        _ = feed(&p, .speechStarted(atMs: 0), .partial(text: "Wie wird das Wetter", atMs: 500))
        XCTAssertEqual(p.feed(.silence(sinceMs: 350, atMs: 850)), [.speculate(text: "Wie wird das Wetter")])
        XCTAssertEqual(p.feed(.partial(text: "Wie wird das Wetter und", atMs: 900)), [.discardSpeculation])
        XCTAssertEqual(p.feed(.silence(sinceMs: 400, atMs: 1_300)), [], "offen: keine neue Vorarbeit")
    }

    func testEmptyUtteranceResetsAtShortestThreshold() {
        // Stopp: der Rest der Äußerung wird geschluckt, bis die Stille lang genug ist.
        var p = DuplexTurnPolicy()
        _ = feed(&p, .partial(text: "Wie wird das Wetter morgen?", atMs: 500), .silence(sinceMs: defaults.endCompleteMs, atMs: 1_400))
        _ = p.feed(.answerStarted(requestId: "r1"))
        XCTAssertEqual(p.feed(.partial(text: "Stopp", atMs: 2_000)), [.cancelAnswer(merge: false), .stopCommand])
        _ = p.feed(.silence(sinceMs: defaults.endCompleteMs, atMs: 3_100))
        XCTAssertEqual(committed(feed(&p, .partial(text: "Nächste Frage bitte jetzt?", atMs: 4_000), .silence(sinceMs: defaults.endCompleteMs, atMs: 4_900))),
                       ["Nächste Frage bitte jetzt?"])
    }

    // MARK: Sprecherwechsel (Zustandsmatrix, Zeilen siehe Vertragskommentar in DuplexTurnPolicy.swift)

    /// Frage gestellt (vollständig, Ende nach `endCompleteMs` Stille, Commit bei 1 400 ms), Modell läuft, noch keine Einheit klingt.
    private func policyWithRunningAnswer(_ config: DuplexTurnPolicy.Config = .init()) -> DuplexTurnPolicy {
        var p = DuplexTurnPolicy(config: config)
        let actions = feed(&p, .speechStarted(atMs: 0), .partial(text: "Wie wird das Wetter morgen?", atMs: 500),
                           .silence(sinceMs: config.endCompleteMs, atMs: 1_400), .answerStarted(requestId: "r1"))
        XCTAssertEqual(committed(actions), ["Wie wird das Wetter morgen?"])
        return p
    }

    /// Eine Überbrückung hat geklungen und ist zu Ende: Lücke, kein Antwortsatz fertig, das Modell arbeitet noch.
    private func policyInGapAfterBridge() -> DuplexTurnPolicy {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Ich schaue kurz nach.", startedAtMs: 1_500, endedAtMs: nil),
                 .unitEnded(atMs: 2_300))
        return p
    }

    /// Ein Antwortsatz ist fertig geklungen, die Einheit ist zu Ende: Lücke (das Modell liefert den nächsten Satz).
    private func policyInGapAfterAnswerSentence() -> DuplexTurnPolicy {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Morgen wird es sonnig.", startedAtMs: 1_500, endedAtMs: 2_800),
                 .unitEnded(atMs: 2_800))
        return p
    }

    /// Eine Antworteinheit klingt gerade (noch nicht zu Ende).
    private func policySounding() -> DuplexTurnPolicy {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Das Wetter morgen wird sonnig.", startedAtMs: 1_500, endedAtMs: nil))
        return p
    }

    /// Wie `policySounding`, aber ein erster Antwortsatz ist schon fertig geklungen und der zweite klingt.
    private func policySoundingSecondSentence() -> DuplexTurnPolicy {
        var p = policyInGapAfterAnswerSentence()
        _ = feed(&p, .unitStarted(atMs: 2_900), .sentenceSpoken(text: "Dazu später mehr.", startedAtMs: 2_900, endedAtMs: nil))
        return p
    }

    // Zeile 1: keine Einheit klingt, 0 relevante Wörter

    func testRow1SpeechBeforeFirstUnitHoldsOnce() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_100)), [], "schon zurückgehalten")
    }

    func testRow1SpeechInEveryGapHolds() {
        var a = policyInGapAfterBridge()
        XCTAssertEqual(a.feed(.speechStarted(atMs: 3_000)), [.holdAnswer], "Lücke nach der Überbrückung")
        var b = policyInGapAfterAnswerSentence()
        XCTAssertEqual(b.feed(.speechStarted(atMs: 3_000)), [.holdAnswer], "Lücke nach einem Antwortsatz")
    }

    func testRow1LevelActiveHoldsAndQuietPlusCaughtUpReleases() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.silence(sinceMs: 300, atMs: 2_400)), [], "Pegel gerade erst gefallen")
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.holdQuietMs - 1, atMs: 2_900)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.holdQuietMs, atMs: 3_000)), [.releaseAnswer], "Stille und Erkennung eingeholt (Anfangswert)")
        XCTAssertEqual(p.feed(.silence(sinceMs: 800, atMs: 3_100)), [], "nur einmal")
    }

    func testRow1QuietButRecognitionBehindHoldsUntilCaughtUp() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_100))
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 3_000)), [], "Stille da, Erkennung noch nicht eingeholt")
        XCTAssertEqual(p.feed(.silence(sinceMs: 900, atMs: 3_200)), [])
        XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 3_300)), [.releaseAnswer], "eingeholt: jetzt gilt die gemeldete Stille")
    }

    func testRow1RecognitionCaughtUpButLevelActiveDoesNotRelease() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_050))
        XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 2_600)), [], "Erkennung eingeholt, aber noch keine Stille")
        XCTAssertEqual(p.feed(.silence(sinceMs: 200, atMs: 2_800)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 3_300)), [.releaseAnswer])
    }

    func testRow1SpeechResumingAfterQuietResetsTheQuiet() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_100))
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 3_000)), [])
        _ = p.feed(.speechStarted(atMs: 3_100))
        XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 3_200)), [], "die frühere Stille ist durch neuen Sprachbeginn ungültig")
    }

    func testRow1StaleSilenceFromBeforeSpeechDoesNotRelease() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.silence(sinceMs: 1_500, atMs: 2_050)), [], "die Stille begann vor dem Sprachbeginn")
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 3_000)), [.releaseAnswer])
    }

    func testRow1RecognitionLagGivesExactlyOneStallAndNeverReleasesByTime() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 900, atMs: 2_900)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 2_900, atMs: 4_900)), [], "Verzug erst \(defaults.recognitionGraceMs - 100) ms")
        XCTAssertEqual(p.feed(.silence(sinceMs: 3_000, atMs: 5_000)), [.holdStalled(reason: .recognition)])
        var rest: [Action] = []
        for at in stride(from: 6_000, through: 20_000, by: 1_000) { rest += p.feed(.silence(sinceMs: at - 2_000, atMs: at)) }
        XCTAssertEqual(rest, [], "weder noch ein Hinweis (auch nicht nach holdNoticeMs) noch eine Freigabe nach Zeit")
        XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 21_000)), [.releaseAnswer])
    }

    func testRow1RecognitionStallNeedsQuietFirst() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 5_500)), [], "Pegel aktiv: kein Erkennungshinweis")
    }

    func testRow1ContinuousNoiseGivesActivityStallOnceAndNeverReleasesByTime() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        var all: [Action] = []
        for at in stride(from: 3_000, through: 12_000, by: 1_000) { all += p.feed(.silence(sinceMs: 100, atMs: at)) }
        XCTAssertEqual(all, [.holdStalled(reason: .activity)], "10 s Pegel ohne Wort: genau ein Hinweis, nie ein Freigeben nach Zeit")
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 13_000)), [], "gehalten wird weiter")
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 14_000)), [.releaseAnswer], "erst die gemeldete Stille gibt frei")
    }

    func testRow1ActivityNoticeComesAtHoldNoticeMsAndOncePerHold() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 2_000 + defaults.holdNoticeMs - 1)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 2_000 + defaults.holdNoticeMs)), [.holdStalled(reason: .activity)])
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 2_000 + defaults.holdNoticeMs + 500)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 9_500)), [.releaseAnswer])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 10_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 10_000 + defaults.holdNoticeMs)), [.holdStalled(reason: .activity)], "neues Halten, neuer Hinweis")
    }

    func testRow1StallNoticeIsAlsoCheckedOnPartialsAndSpeechStarts() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.partial(text: "mhm", atMs: 8_100)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel), .holdStalled(reason: .activity)])
        var q = policyWithRunningAnswer()
        _ = q.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(q.feed(.speechStarted(atMs: 8_000)), [.holdStalled(reason: .activity)])
    }

    func testRow1UnitStartedWhileHoldingReleasesToKeepControllerAndPolicyInStep() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.unitStarted(atMs: 2_100)), [.releaseAnswer])
        XCTAssertEqual(p.feed(.silence(sinceMs: 800, atMs: 3_000)), [])
    }

    func testRow1AnswerFinishedResetAndHalfDuplex() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        _ = p.feed(.answerFinished(requestId: "r1"))
        XCTAssertEqual(p.feed(.silence(sinceMs: 800, atMs: 3_000)), [], "Antwortende beendet das Halten ohne Freigabe")
        XCTAssertEqual(p.feed(.speechStarted(atMs: 4_000)), [], "keine Antwort mehr aktiv")
        var q = policyWithRunningAnswer()
        _ = q.feed(.speechStarted(atMs: 2_000))
        q.reset()
        XCTAssertEqual(q.feed(.silence(sinceMs: 800, atMs: 3_000)), [])
        var c = DuplexTurnPolicy.Config()
        c.bargeIn = false
        var h = policyWithRunningAnswer(c)
        XCTAssertEqual(h.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        XCTAssertEqual(h.feed(.silence(sinceMs: 700, atMs: 3_000)), [.releaseAnswer])
    }

    func testRow1NoHoldWithoutAnswerOrForSpeculationOnly() {
        var p = DuplexTurnPolicy()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 100)), [], "keine Antwort aktiv")
        var q = DuplexTurnPolicy()
        _ = feed(&q, .speechStarted(atMs: 0), .partial(text: "Wie wird das Wetter morgen", atMs: 500), .silence(sinceMs: 350, atMs: 850),
                 .answerStarted(requestId: "spec"))
        XCTAssertEqual(q.feed(.speechStarted(atMs: 900)), [.discardSpeculation], "Vorarbeit ist noch keine Antwort")
    }

    func testRow1HoldTimesAreConfigurable() {
        var c = DuplexTurnPolicy.Config()
        c.holdQuietMs = 400
        c.holdNoticeMs = 900
        c.recognitionGraceMs = 500
        var p = policyWithRunningAnswer(c)
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 400, atMs: 2_500)), [.releaseAnswer])
        var q = policyWithRunningAnswer(c)
        _ = q.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(q.feed(.silence(sinceMs: 100, atMs: 2_900)), [.holdStalled(reason: .activity)])
        var r = policyWithRunningAnswer(c)
        _ = feed(&r, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_000))
        XCTAssertEqual(r.feed(.silence(sinceMs: 500, atMs: 2_600)), [.holdStalled(reason: .recognition)])
    }

    // Zeile 2: keine Einheit klingt, ≥ 1 relevantes Wort

    func testRow2OneWordDuringHoldIsAContinuationWithMerge() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 2_300)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 3_100)), [], "kein Freigeben, die Anfrage ist verworfen")
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 4_400))), ["Wie wird das Wetter morgen? Berlin"])
    }

    func testRow2OneWordWithoutPriorHoldIsAContinuationToo() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 2_300)), [.cancelAnswer(merge: true)], "Schwelle 1 hängt an „keine Einheit klingt“, nicht am Halten")
        var q = policyInGapAfterBridge()
        XCTAssertEqual(q.feed(.partial(text: "Berlin", atMs: 3_000)), [.cancelAnswer(merge: true)], "nur Überbrückung geklungen: Fortsetzung")
    }

    func testRow2OneWordWhileRecognitionIsBehindStillTakesOver() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_100))
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 2_300)), [.cancelAnswer(merge: true)])
    }

    func testRow2TwoWordsDuringHoldMergeAndNeverRelease() {
        var p = policyWithRunningAnswer()
        var all: [Action] = p.feed(.speechStarted(atMs: 2_000))
        all += p.feed(.partial(text: "und in", atMs: 2_300))
        XCTAssertEqual(all, [.holdAnswer, .cancelAnswer(merge: true)])
        let rest = feed(&p, .partial(text: "und in Berlin", atMs: 2_700), .silence(sinceMs: defaults.endNeutralMs, atMs: 3_900))
        XCTAssertEqual(rest, [.commit(text: "Wie wird das Wetter morgen? und in Berlin", merged: true, afterInterruption: false)])
        XCTAssertEqual(p.feed(.silence(sinceMs: 1_500, atMs: 4_300)), [], "danach nie ein spätes Freigeben")
    }

    func testRow2OneWordInTheGapAfterAFinishedSentenceIsAnInterruptionNotAMerge() {
        var p = policyInGapAfterAnswerSentence()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 3_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 3_300)), [.cancelAnswer(merge: false)])
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 5_400)),
                       [.commit(text: "Berlin", merged: false, afterInterruption: true)])
        var q = policyInGapAfterAnswerSentence()
        XCTAssertEqual(q.feed(.partial(text: "Berlin", atMs: 3_300)), [.cancelAnswer(merge: false)], "auch ohne Sprachbeginn-Meldung")
    }

    func testRow2MorgenAfterTheSentenceMorgenWirdEsSonnigIsEchoNotContinuation() {
        var p = policyInGapAfterAnswerSentence()
        XCTAssertEqual(p.feed(.partial(text: "morgen", atMs: 3_300)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
    }

    func testRow2NewOutputOnlyAfterUtteranceEnd() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .partial(text: "und in", atMs: 2_300))
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endNeutralMs - 1, atMs: 3_000)).filter { if case .commit = $0 { return true } else { return false } }, [])
    }

    // Zeile 3: eine Einheit klingt, 0 oder 1 relevantes Wort

    func testRow3OneWordWhileAUnitSoundsWaitsAndTakesOverAtUnitEnded() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [], "keine Haltung, solange etwas klingt")
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 2_200)), [], "ein Wort bei klingender Einheit: Einheit klingt weiter")
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_600)), [.cancelAnswer(merge: true)], "vor dem ersten fertigen Antwortsatz: Fortsetzung")
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 4_700))), ["Wie wird das Wetter morgen? Berlin"])
    }

    func testRow3WordIsEvaluatedAtUnitEndedEvenForTheLastUnitBeforeAnswerFinished() {
        var p = policySoundingSecondSentence()
        _ = feed(&p, .speechStarted(atMs: 3_200), .partial(text: "Berlin", atMs: 3_300))
        XCTAssertEqual(p.feed(.unitEnded(atMs: 3_800)), [.cancelAnswer(merge: false)], "erster Antwortsatz war fertig: Unterbrechung")
        XCTAssertEqual(p.feed(.answerFinished(requestId: "r1")), [])
    }

    func testRow3NoRelevantWordMeansNothingHappensAtUnitEnded() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "mhm", atMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_600)), [], "0 relevante Wörter: weiter, nichts zu tun")
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_700)), [.holdAnswer], "nach dem Ende der Einheit ist wieder Lücke")
    }

    func testRow3EchoWordWhileSoundingIsNoRelevantWord() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "sonnig", atMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_600)), [], "das Echo-Wort bleibt nicht als Fortsetzung zurück")
    }

    func testRow3UnitEndedAfterTakeoverFromAnotherPathIsHarmless() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "und in Berlin", atMs: 2_000)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_050)), [], "Abbruch beendet die Einheit; kein zweiter Abbruch")
    }

    // Zeile 4: eine Einheit klingt, ≥ 2 relevante Wörter

    func testRow4TwoWordsWhileAUnitSoundsStopOutputAtOnce() {
        var a = policySounding()
        XCTAssertEqual(a.feed(.partial(text: "und in Berlin", atMs: 2_000)), [.cancelAnswer(merge: true)], "vor dem ersten fertigen Satz: merge")
        XCTAssertEqual(committed(a.feed(.silence(sinceMs: defaults.endNeutralMs, atMs: 3_300))), ["Wie wird das Wetter morgen? und in Berlin"])
        var b = policySoundingSecondSentence()
        XCTAssertEqual(b.feed(.partial(text: "und in Berlin", atMs: 3_300)), [.cancelAnswer(merge: false)], "danach: Unterbrechung")
        XCTAssertEqual(b.feed(.silence(sinceMs: defaults.endNeutralMs, atMs: 4_600)),
                       [.commit(text: "und in Berlin", merged: false, afterInterruption: true)])
    }

    func testRow4BridgeSoundsAndUserContinuesWithTwoWordsMerges() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Ich schaue kurz nach.", startedAtMs: 1_500, endedAtMs: nil))
        XCTAssertEqual(p.feed(.speechStarted(atMs: 1_800)), [])
        XCTAssertEqual(p.feed(.partial(text: "und in Berlin", atMs: 2_100)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: defaults.endNeutralMs, atMs: 3_400))), ["Wie wird das Wetter morgen? und in Berlin"])
    }

    // Zeile 5: Stoppbefehl in jedem Zustand

    func testRow5StopInEveryStateEndsEverythingWithoutRelease() {
        let states: [(String, () -> DuplexTurnPolicy)] = [
            ("klingt", { self.policySounding() }),
            ("Lücke nach Überbrückung", { self.policyInGapAfterBridge() }),
            ("Lücke nach Antwortsatz", { self.policyInGapAfterAnswerSentence() }),
            ("vor der ersten Einheit", { self.policyWithRunningAnswer() }),
        ]
        for (name, make) in states {
            for hold in [false, true] {
                var p = make()
                if hold, name != "klingt" { _ = p.feed(.speechStarted(atMs: 3_000)) }
                XCTAssertEqual(p.feed(.partial(text: "Stopp", atMs: 3_100)), [.cancelAnswer(merge: false), .stopCommand], "\(name) hold=\(hold)")
                XCTAssertEqual(p.feed(.silence(sinceMs: 800, atMs: 4_000)), [], "\(name): danach nie ein Freigeben")
                XCTAssertEqual(p.feed(.manualRelease(atMs: 4_100)), [], name)
                XCTAssertEqual(p.feed(.unitStarted(atMs: 4_200)), [], "\(name): nichts läuft wieder an")
                XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 4_300)), [], name)
                XCTAssertEqual(p.feed(.silence(sinceMs: 2_000, atMs: 6_000)), [], "\(name): keine neue Anfrage")
            }
        }
    }

    func testRow5StopWordInsideALongSentenceIsNoStopAndMultiWordStopWorks() {
        var p = policyInGapAfterBridge()
        XCTAssertEqual(p.feed(.partial(text: "Moment mal kurz", atMs: 3_000)), [.cancelAnswer(merge: false), .stopCommand])
        var q = policyInGapAfterBridge()
        XCTAssertEqual(q.feed(.partial(text: "warte ich möchte noch etwas ergänzen", atMs: 3_000)), [.cancelAnswer(merge: true)])
    }

    // Zeile 6: Echo und Rückmeldelaut

    func testRow6BackchannelInBothStatesIsIgnored() {
        var sounding = policySounding()
        XCTAssertEqual(sounding.feed(.partial(text: "mhm", atMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        var gap = policyInGapAfterBridge()
        XCTAssertEqual(gap.feed(.partial(text: "mhm", atMs: 3_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)], "Lücke ohne Halten")
        XCTAssertEqual(gap.feed(.silence(sinceMs: 2_000, atMs: 6_000)), [], "nichts bleibt als Äußerung zurück")
        var hold = policyInGapAfterBridge()
        _ = hold.feed(.speechStarted(atMs: 3_000))
        XCTAssertEqual(hold.feed(.partial(text: "mhm", atMs: 3_200)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(hold.feed(.silence(sinceMs: 300, atMs: 3_500)), [], "keine Freigabe vor der Stille")
        XCTAssertEqual(hold.feed(.silence(sinceMs: 700, atMs: 3_900)), [.releaseAnswer])
    }

    func testRow6EchoTailInTheGapIsNoContinuation() {
        var p = policyInGapAfterAnswerSentence()
        XCTAssertEqual(p.feed(.partial(text: "wird es sonnig", atMs: 3_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)], "Nachlauf des gerade beendeten Satzes")
        XCTAssertEqual(p.feed(.partial(text: "sonnig", atMs: 3_100)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)], "auch ein einzelnes Echo-Wort")
        XCTAssertEqual(p.feed(.silence(sinceMs: 2_000, atMs: 5_000)), [], "es bleibt nichts als Äußerung stehen")
    }

    func testRow6EchoDuringHoldIsIgnoredAndReleasedOnlyAfterQuiet() {
        var p = policyInGapAfterBridge()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 3_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.partial(text: "ich schaue kurz nach", atMs: 3_400)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        XCTAssertEqual(p.feed(.partial(text: "schaue", atMs: 3_500)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)], "ein Echo-Wort ist keine Fortsetzung")
        XCTAssertEqual(p.feed(.silence(sinceMs: 300, atMs: 3_900)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 4_200)), [.releaseAnswer])
    }

    // Zeile 7: Handfreigabe

    func testRow7ManualReleaseFreesTheAnswerAndSuppressesFurtherHoldInThisAnswer() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.manualRelease(atMs: 3_000)), [.releaseAnswer])
        XCTAssertEqual(p.feed(.manualRelease(atMs: 3_100)), [], "nur einmal")
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 4_000)), [], "kein zweites Freigeben")
        XCTAssertEqual(p.feed(.speechStarted(atMs: 5_000)), [], "weiteres Geräusch hält in dieser Antwort nicht mehr")
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 20_000)), [], "auch kein Hinweis")
    }

    func testRow7SuppressionSurvivesUnitsAndGapsOfTheSameAnswer() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .manualRelease(atMs: 2_500), .unitStarted(atMs: 2_600), .unitEnded(atMs: 3_500))
        XCTAssertEqual(p.feed(.speechStarted(atMs: 4_000)), [], "Lücke derselben Antwort")
    }

    func testRow7WordsAndStopCommandsStillWorkAfterManualRelease() {
        var a = policyWithRunningAnswer()
        _ = feed(&a, .speechStarted(atMs: 2_000), .manualRelease(atMs: 2_500))
        XCTAssertEqual(a.feed(.partial(text: "Berlin", atMs: 3_000)), [.cancelAnswer(merge: true)], "Zeile 2 gilt weiter")
        var b = policyWithRunningAnswer()
        _ = feed(&b, .speechStarted(atMs: 2_000), .manualRelease(atMs: 2_500))
        XCTAssertEqual(b.feed(.partial(text: "Stopp", atMs: 3_000)), [.cancelAnswer(merge: false), .stopCommand])
    }

    func testRow7NextAnswerHoldsAgain() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .manualRelease(atMs: 2_500), .answerFinished(requestId: "r1"))
        let actions = feed(&p, .speechStarted(atMs: 4_000), .partial(text: "Und wie wird es übermorgen?", atMs: 4_500),
                           .silence(sinceMs: defaults.endCompleteMs, atMs: 5_400), .answerStarted(requestId: "r2"))
        XCTAssertEqual(committed(actions), ["Und wie wird es übermorgen?"])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 6_000)), [.holdAnswer], "mit der nächsten Antwort gilt wieder die normale Regel")
    }

    func testRow7ContinuousNoiseThenStallThenManualReleaseThenNoiseIsIgnoredUntilTheNextAnswer() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 8_100)), [.holdStalled(reason: .activity)])
        XCTAssertEqual(p.feed(.manualRelease(atMs: 8_500)), [.releaseAnswer])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 9_000)), [], "weiteres Geräusch hält nicht mehr")
        _ = feed(&p, .answerFinished(requestId: "r1"))
        let next = feed(&p, .partial(text: "Noch eine Frage bitte?", atMs: 10_000), .silence(sinceMs: defaults.endCompleteMs, atMs: 10_900),
                        .answerStarted(requestId: "r2"))
        XCTAssertEqual(committed(next), ["Noch eine Frage bitte?"])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 12_000)), [.holdAnswer])
    }

    func testRow7ManualReleaseWithoutHoldDoesNothingAndDoesNotSuppress() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.manualRelease(atMs: 3_000)), [])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 3_500)), [.holdAnswer])
    }

    // Schnittstelle ohne Halten

    func testNoHoldOnceUnitSoundsAndLaterGapHoldsAgain() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.unitStarted(atMs: 1_500))
        XCTAssertEqual(p.feed(.speechStarted(atMs: 1_800)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 800, atMs: 3_000)), [], "nichts freizugeben")
        _ = p.feed(.unitEnded(atMs: 3_100))
        XCTAssertEqual(p.feed(.speechStarted(atMs: 3_500)), [.holdAnswer])
    }

    func testResetForgetsRecognitionAndSuppression() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, atMs: 2_100), .manualRelease(atMs: 2_200))
        p.reset()
        let actions = feed(&p, .partial(text: "Wie wird das Wetter morgen?", atMs: 5_000), .silence(sinceMs: defaults.endCompleteMs, atMs: 5_900))
        XCTAssertEqual(committed(actions), ["Wie wird das Wetter morgen?"])
        _ = p.feed(.answerStarted(requestId: "r9"))
        XCTAssertEqual(p.feed(.speechStarted(atMs: 7_000)), [.holdAnswer])
        XCTAssertEqual(p.feed(.silence(sinceMs: 800, atMs: 8_000)), [.releaseAnswer], "Erkennung gilt nach reset wieder als eingeholt")
    }

    // MARK: ADR-0028 E9, Falltabelle „Sprecherwechsel“ (Fall = Zeile der Tabelle, von oben gezählt)

    private func isStop(_ actions: [Action]) -> Bool { actions == [.cancelAnswer(merge: false), .stopCommand] }

    func testFall01GapOrBeforeFirstUnitLevelActiveZeroWordsHoldsAtOnce() {
        var a = policyWithRunningAnswer()
        XCTAssertEqual(a.feed(.speechStarted(atMs: 2_000)), [.holdAnswer], "vor der ersten Einheit, im selben Ereignis")
        var b = policyInGapAfterBridge()
        XCTAssertEqual(b.feed(.speechStarted(atMs: 3_000)), [.holdAnswer], "Lücke")
    }

    func testFall02QuietOf700WithCaughtUpRecognitionReleases() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 699, atMs: 2_899)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 3_000)), [.releaseAnswer])
    }

    func testFall03QuietButRecognitionNotCaughtUpHoldsAndReportsAfter3sOfQuiet() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, following: true, atMs: 2_050))
        var all: [Action] = []
        for at in stride(from: 2_700, through: 4_900, by: 200) { all += p.feed(.silence(sinceMs: at - 2_100, atMs: at)) }
        XCTAssertEqual(all, [], "Stille da, nicht eingeholt: weiter Zurückhalten, vor 3 s keine Meldung")
        XCTAssertEqual(p.feed(.silence(sinceMs: 3_000, atMs: 5_100)), [.holdStalled(reason: .recognition)], "3 s nach Beginn der Stille")
        XCTAssertEqual(p.feed(.silence(sinceMs: 3_200, atMs: 5_300)), [], "einmal")
    }

    func testFall04AlternatingNoiseAndQuietWithFollowingRecognitionHoldsUntilManualControlAt6s() {
        var p = policyWithRunningAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 2_000)), [.holdAnswer])
        var events: [(Int, [Action])] = []
        var t = 2_400
        while t <= 9_600 {
            events.append((t, p.feed(.silence(sinceMs: 400, atMs: t))))
            events.append((t + 400, p.feed(.speechStarted(atMs: t + 400))))
            t += 800
        }
        let nonEmpty = events.filter { !$0.1.isEmpty }
        XCTAssertEqual(nonEmpty.count, 1, "keine Freigabe nach Zeit, genau ein Hinweis")
        XCTAssertEqual(nonEmpty.first?.1, [.holdStalled(reason: .activity)])
        XCTAssertGreaterThanOrEqual(nonEmpty.first?.0 ?? 0, 2_000 + defaults.holdNoticeMs, "Pegelwechsel setzen die Zählung nicht zurück")
        XCTAssertLessThanOrEqual(nonEmpty.first?.0 ?? 0, 2_000 + defaults.holdNoticeMs + 800)
    }

    func testFall05ContinuousNoiseThenManualReleaseStartsAnswerAndNoiseHoldsNoMoreOnlyInThisAnswer() {
        var p = policyWithRunningAnswer()
        _ = p.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 8_000)), [.holdStalled(reason: .activity)])
        XCTAssertEqual(p.feed(.manualRelease(atMs: 8_100)), [.releaseAnswer])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 8_500)), [], "Geräusch hält in dieser Antwort nicht mehr")
        _ = feed(&p, .answerFinished(requestId: "r1"))
        let next = feed(&p, .partial(text: "Noch eine Frage bitte?", atMs: 10_000), .silence(sinceMs: defaults.endCompleteMs, atMs: 10_900),
                        .answerStarted(requestId: "r2"))
        XCTAssertEqual(committed(next), ["Noch eine Frage bitte?"])
        XCTAssertEqual(p.feed(.speechStarted(atMs: 12_000)), [.holdAnswer], "in der nächsten wieder")
    }

    func testFall06ContinuousNoiseWithoutFollowingRecognitionOffersOnlyDiscardAndManualReleaseDoesNothing() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, following: false, atMs: 2_050))
        var early: [Action] = []
        for at in stride(from: 2_500, through: 7_900, by: 400) { early += p.feed(.silence(sinceMs: 100, atMs: at)) }
        XCTAssertEqual(early, [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 8_000)), [.holdStalled(reason: .recognition)], "nach 6 s, nur Verwerfen")
        XCTAssertEqual(p.feed(.manualRelease(atMs: 8_100)), [], "„Antwort jetzt hören“ ist wirkungslos")
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 8_500)), [], "noch immer gehalten, nichts freigegeben")
        XCTAssertTrue(isStop(p.feed(.manualDiscard(atMs: 9_000))), "Verwerfen beendet die Antwort")
    }

    func testFall06bReasonChangesWithinOneHoldAreReportedOnceEach() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, following: false, atMs: 2_050))
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 8_000)), [.holdStalled(reason: .recognition)])
        XCTAssertEqual(p.feed(.recognition(caughtUp: false, following: true, atMs: 8_200)), [.holdStalled(reason: .activity)], "neuer Grund, einmal")
        XCTAssertEqual(p.feed(.silence(sinceMs: 100, atMs: 9_000)), [])
        XCTAssertEqual(p.feed(.manualRelease(atMs: 9_100)), [.releaseAnswer], "folgend: Handfreigabe wirkt")
    }

    func testFall07GapOrHoldExactlyOneRelevantWordIsContinuation() {
        var gap = policyInGapAfterAnswerSentence()
        XCTAssertEqual(gap.feed(.partial(text: "Berlin", atMs: 3_000)), [.cancelAnswer(merge: false)], "nach dem ersten fertigen Satz: unterbrochen, neue Anfrage")
        var hold = policyWithRunningAnswer()
        _ = hold.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(hold.feed(.partial(text: "Berlin", atMs: 2_200)), [.cancelAnswer(merge: true)], "vor dem ersten fertigen Satz: alte Frage plus Wort")
        XCTAssertEqual(committed(hold.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 4_300))), ["Wie wird das Wetter morgen? Berlin"])
    }

    func testFall08UnitSoundsOneRelevantWordUnitEndsThenContinuation() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 2_000)), [], "klingt zu Ende")
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_400)), [.cancelAnswer(merge: true)])
    }

    func testFall09UnitSoundsNoWordLevelActiveUnitEndsHoldsFromUnitEnd() {
        var a = policySounding()
        _ = a.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(a.feed(.unitEnded(atMs: 2_400)), [.holdAnswer], "Pegel aktiv")
        XCTAssertEqual(a.feed(.silence(sinceMs: 700, atMs: 3_300)), [.releaseAnswer], "danach wie Zeile 2: Stille bei eingeholter Erkennung")
        var b = policySounding()
        _ = feed(&b, .speechStarted(atMs: 2_000), .silence(sinceMs: 300, atMs: 2_400), .recognition(caughtUp: false, atMs: 2_450))
        XCTAssertEqual(b.feed(.unitEnded(atMs: 2_500)), [.holdAnswer], "Pegel still, aber Erkennung nicht eingeholt")
        var c = policySounding()
        _ = feed(&c, .speechStarted(atMs: 2_000), .silence(sinceMs: 300, atMs: 2_400))
        XCTAssertEqual(c.feed(.unitEnded(atMs: 2_500)), [], "Pegel still und Erkennung eingeholt: weiter")
        XCTAssertEqual(c.feed(.speechStarted(atMs: 2_900)), [.holdAnswer], "erst neuer Sprachbeginn hält")
    }

    /// Der Hörer meldet je lautem Mikrofonpuffer `silence(sinceMs: 0)`. Das ist laufender Pegel, keine Stille.
    func testFall09cLoudBuffersReportedAsZeroSilenceKeepTheLevelActive() {
        var a = policySounding()
        _ = feed(&a, .speechStarted(atMs: 2_000), .silence(sinceMs: 0, atMs: 2_085), .silence(sinceMs: 0, atMs: 2_170))
        XCTAssertEqual(a.feed(.unitEnded(atMs: 2_400)), [.holdAnswer], "durchgehende Rede: Pegel aktiv trotz silence(0)")
        XCTAssertEqual(a.feed(.silence(sinceMs: 0, atMs: 3_500)), [], "lauter Puffer gibt nie frei")
        XCTAssertEqual(a.feed(.silence(sinceMs: 700, atMs: 4_300)), [.releaseAnswer])
        var b = policySounding()
        _ = feed(&b, .speechStarted(atMs: 2_000), .silence(sinceMs: 300, atMs: 2_400), .silence(sinceMs: 0, atMs: 2_485))
        XCTAssertEqual(b.feed(.unitEnded(atMs: 2_500)), [.holdAnswer], "nach kurzer Pause wieder laut: Pegel aktiv")
    }

    func testFall09bHoldAtUnitEndedIsSuppressedAfterManualRelease() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .manualRelease(atMs: 2_500), .unitStarted(atMs: 2_600), .speechStarted(atMs: 2_700))
        XCTAssertEqual(p.feed(.unitEnded(atMs: 3_000)), [], "Pegelaktivität hält in dieser Antwort nicht mehr zurück")
    }

    func testFall10UnitSoundsTwoRelevantWordsStopOutputAndContinue() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "und in Berlin", atMs: 2_000)), [.cancelAnswer(merge: true)])
    }

    func testFall11FurtherResultsAfterTheContinuationDecisionTriggerNothing() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "und in Berlin", atMs: 2_000)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(p.feed(.partial(text: "und in Berlin bitte", atMs: 2_300)), [], "kein zweites cancelAnswer")
        XCTAssertEqual(p.feed(.partial(text: "und in Berlin bitte das Wetter", atMs: 2_600)), [])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_700)), [])
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: defaults.endNeutralMs, atMs: 3_900))), ["Wie wird das Wetter morgen? und in Berlin bitte das Wetter"])
    }

    func testFall11bContinuationIsNotSortedOutAsEchoLater() {
        var p = policyInGapAfterAnswerSentence()
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 3_000, audioMs: 3_000)), [.cancelAnswer(merge: false)])
        XCTAssertEqual(p.feed(.partial(text: "Berlin wird es sonnig", atMs: 3_400, audioMs: 3_400)), [], "Wörter der alten Antwort in der Fortsetzung bleiben")
    }

    func testFall12EchoTailOfAnEndedUnitIsNoContinuationAndNoRelease() {
        var p = policyInGapAfterAnswerSentence()   // Einheit [1 500, 2 800], Nachlauf mit Audiozeit bis 3 100
        _ = p.feed(.speechStarted(atMs: 3_000))
        XCTAssertEqual(p.feed(.partial(text: "wird es sonnig", atMs: 3_500, audioMs: 3_400)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)],
                       "Sprachbeginn 3 000 liegt im Nachlauf bis 2 800 + \(defaults.echoTailAudioMs)")
        XCTAssertEqual(p.feed(.silence(sinceMs: 300, atMs: 3_700)), [], "keine Freigabe vor der Stille")
        XCTAssertEqual(p.feed(.silence(sinceMs: 700, atMs: 4_100)), [.releaseAnswer])
    }

    func testFall12bTheSameWordsAfterTheEchoTailAreRelevant() {
        var p = policyInGapAfterAnswerSentence()
        _ = p.feed(.speechStarted(atMs: 3_200))
        XCTAssertEqual(p.feed(.partial(text: "wird es sonnig", atMs: 3_800, audioMs: 3_700)), [.cancelAnswer(merge: false)], "Sprachbeginn 3 200 liegt hinter dem Nachlauf")
    }

    func testEchoTailDefaultIs300msAndIsConfigurable() {
        XCTAssertEqual(defaults.echoTailAudioMs, 300)
        var c = DuplexTurnPolicy.Config()
        c.echoTailAudioMs = 1_500
        var p = policyWithRunningAnswer(c)
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Morgen wird es sonnig.", startedAtMs: 1_500, endedAtMs: 2_800), .unitEnded(atMs: 2_800),
                 .speechStarted(atMs: 4_000))
        XCTAssertEqual(p.feed(.partial(text: "wird es sonnig", atMs: 4_500, audioMs: 4_300)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
    }

    func testRealYesHalfASecondAfterAQuestionContainingYesIsNotLostAsEcho() {
        let t = 2_800
        func afterQuestion() -> DuplexTurnPolicy {
            var p = policyWithRunningAnswer()
            _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Möchtest du das, ja oder nein", startedAtMs: 1_500, endedAtMs: t), .unitEnded(atMs: t))
            return p
        }
        // Außerhalb der Antwort: normale Äußerung, die zum Commit führt.
        var out = afterQuestion()
        _ = out.feed(.answerFinished(requestId: "r1"))
        _ = out.feed(.speechStarted(atMs: t + 500))
        XCTAssertEqual(out.feed(.partial(text: "ja", atMs: t + 1_000, audioMs: t + 900)), [], "kein Echo: Sprachbeginn t+500 liegt hinter t+300")
        XCTAssertEqual(committed(out.feed(.silence(sinceMs: defaults.endCompleteMs, atMs: t + 2_000))), ["ja"])
        // In der Antwort ist ein einzelnes „ja“ ein Rückmeldelaut, aber nie Echo.
        var inside = afterQuestion()
        _ = inside.feed(.speechStarted(atMs: t + 500))
        XCTAssertEqual(inside.feed(.partial(text: "ja", atMs: t + 1_000, audioMs: t + 900)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        // Sprachbeginn 200 ms vor dem Klangende: Echo.
        var early = afterQuestion()
        _ = early.feed(.answerFinished(requestId: "r1"))
        _ = early.feed(.speechStarted(atMs: t - 200))
        XCTAssertEqual(early.feed(.partial(text: "ja", atMs: t + 400, audioMs: t + 100)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        var earlyInside = afterQuestion()
        _ = earlyInside.feed(.speechStarted(atMs: t - 200))
        XCTAssertEqual(earlyInside.feed(.partial(text: "ja", atMs: t + 400, audioMs: t + 100)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
    }

    func testFall12cEchoWithoutAudioTimeUsesArrivalTimeAnd8sTail() {
        var p = policyInGapAfterAnswerSentence()
        XCTAssertEqual(p.feed(.partial(text: "wird es sonnig", atMs: 9_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)], "6,2 s nach Klangende: im Zweifel Echo")
        var q = policyInGapAfterAnswerSentence()
        XCTAssertEqual(q.feed(.partial(text: "wird es sonnig", atMs: 11_000)), [.cancelAnswer(merge: false)], "über 8 s: Nutzerrede")
    }

    func testFall12dEchoSegmentIsBuiltOnlyFromUnitsAroundTheUtteranceTime() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Das Wetter morgen wird sonnig.", startedAtMs: 1_500, endedAtMs: 2_800),
                 .unitEnded(atMs: 2_800), .unitStarted(atMs: 20_000), .sentenceSpoken(text: "Dazu später mehr.", startedAtMs: 20_000, endedAtMs: nil))
        _ = p.feed(.speechStarted(atMs: 20_500))
        XCTAssertEqual(p.feed(.partial(text: "das wetter wird sonnig", atMs: 21_000, audioMs: 21_000)), [.cancelAnswer(merge: false)],
                       "die Einheit von 1 500 bis 2 800 liegt weit zurück: kein Echo, zwei Wörter genügen bei klingender Einheit")
    }

    func testFall13BackchannelWhileSoundingAndInTheGapIsConsumed() {
        var s = policySounding()
        XCTAssertEqual(s.feed(.partial(text: "mhm", atMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(s.feed(.unitEnded(atMs: 2_600)), [], "verbraucht: 0 Wörter, Pegel still")
        var g = policyInGapAfterBridge()
        _ = g.feed(.speechStarted(atMs: 3_000))
        XCTAssertEqual(g.feed(.partial(text: "ja genau", atMs: 3_200)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(g.feed(.silence(sinceMs: 700, atMs: 4_000)), [.releaseAnswer], "Freigabe nach Stille")
    }

    func testFall14ClassificationRevisionWarteBitteLaterFourWordsStaysStopAndRestIsDiscarded() {
        var p = policyInGapAfterBridge()
        XCTAssertTrue(isStop(p.feed(.partial(text: "warte bitte", atMs: 3_000))))
        XCTAssertEqual(p.feed(.partial(text: "warte bitte ich möchte noch", atMs: 3_300)), [], "verbindlich, Rest verworfen")
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endNeutralMs, atMs: 5_000)), [], "keine neue Anfrage")
        XCTAssertEqual(committed(feed(&p, .partial(text: "Und jetzt etwas anderes bitte", atMs: 7_000),
                                     .silence(sinceMs: defaults.endNeutralMs, atMs: 8_300))), ["Und jetzt etwas anderes bitte"], "danach ist alles wieder normal")
    }

    func testFall15ClassificationRevisionFirstResultAlreadyFourWordsWithWarteIsNoStop() {
        var p = policyInGapAfterBridge()
        XCTAssertEqual(p.feed(.partial(text: "warte ich möchte noch", atMs: 3_000)), [.cancelAnswer(merge: true)], "relevante Wörter nach Matrix")
        var q = policyInGapAfterBridge()
        _ = q.feed(.partial(text: "ich möchte noch etwas", atMs: 3_000))
        // Kürzt die Erkennung später auf ein Stoppwort, bleibt es bei der verbindlichen Entscheidung (kein Stopp, keine zweite Fortsetzung).
        XCTAssertEqual(q.feed(.partial(text: "stopp", atMs: 3_100)), [])
    }

    func testFall15bFirstResultDecidesEvenIfTheAnswerIsAliveAtThatMoment() {
        var p = policySounding()
        XCTAssertEqual(p.feed(.partial(text: "warte ich möchte noch etwas", atMs: 2_000)), [.cancelAnswer(merge: true)])
        var q = policySounding()
        _ = q.feed(.partial(text: "mhm", atMs: 2_000))   // verbraucht: der kumulative Text „mhm stopp“ ist danach eine neue Äußerung „stopp“
        XCTAssertTrue(isStop(q.feed(.partial(text: "mhm stopp", atMs: 2_200))))
    }

    func testFall16StopSoundingGapAndHoldEndsTheAnswerWithoutRestartOrNewRequest() {
        var holding = policyInGapAfterBridge()
        _ = holding.feed(.speechStarted(atMs: 3_000))
        let states: [(String, DuplexTurnPolicy)] = [("klingt", policySounding()), ("Lücke", policyInGapAfterAnswerSentence()), ("Halten", holding)]
        for (name, start) in states {
            var p = start
            XCTAssertTrue(isStop(p.feed(.partial(text: "Stopp", atMs: 3_100))), name)
            XCTAssertEqual(p.feed(.silence(sinceMs: 900, atMs: 4_200)), [], "\(name): kein Wiederanlauf, keine neue Anfrage")
            XCTAssertEqual(p.feed(.unitStarted(atMs: 4_300)), [], name)
        }
    }

    func testFall17AfterStopManualReleaseLateResultsAndVoiceMessagesAreWithoutEffect() {
        var p = policyInGapAfterBridge()
        _ = p.feed(.speechStarted(atMs: 3_000))
        XCTAssertTrue(isStop(p.feed(.partial(text: "Stopp", atMs: 3_100))))
        XCTAssertEqual(p.feed(.manualRelease(atMs: 3_200)), [])
        XCTAssertEqual(p.feed(.manualDiscard(atMs: 3_250)), [])
        XCTAssertEqual(p.feed(.partial(text: "Stopp bitte hör auf", atMs: 3_300)), [], "späte Ergebnisse derselben Äußerung")
        XCTAssertEqual(p.feed(.unitStarted(atMs: 3_400)), [])
        XCTAssertEqual(p.feed(.unitEnded(atMs: 3_500)), [], "auch kein Halten durch laufenden Pegel")
        XCTAssertEqual(p.feed(.sentenceSpoken(text: "Morgen wird es sonnig.", startedAtMs: 3_400, endedAtMs: 3_900)), [])
        XCTAssertEqual(p.feed(.recognition(caughtUp: false, following: false, atMs: 3_600)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: 900, atMs: 4_600)), [])
        XCTAssertEqual(p.feed(.answerFinished(requestId: "r1")), [])
    }

    func testFall17bNewAnswerStartedMakesUnitMessagesEffectiveAgain() {
        var p = policyInGapAfterBridge()
        XCTAssertTrue(isStop(p.feed(.partial(text: "Stopp", atMs: 3_100))))
        _ = p.feed(.silence(sinceMs: 900, atMs: 4_200))
        let next = feed(&p, .partial(text: "Noch eine Frage bitte?", atMs: 5_000), .silence(sinceMs: defaults.endCompleteMs, atMs: 5_900))
        XCTAssertEqual(committed(next), ["Noch eine Frage bitte?"])
        XCTAssertEqual(p.feed(.unitStarted(atMs: 6_100)), [], "noch vor answerStarted: Meldung zur alten Antwort, zählt nicht")
        XCTAssertEqual(p.feed(.speechStarted(atMs: 6_150)), [.holdAnswer], "es klingt nichts: Halten ist möglich")
        XCTAssertEqual(p.feed(.unitStarted(atMs: 6_200)), [], "weiterhin ignoriert, kein Freigeben")
        _ = p.feed(.answerStarted(requestId: "r2"))
        XCTAssertEqual(p.feed(.unitStarted(atMs: 6_300)), [.releaseAnswer], "nach answerStarted wirken die Meldungen der neuen Antwort wieder")
    }

    func testFall18DiscardWithRecognitionLagEndsTheAnswerWithoutRestart() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .speechStarted(atMs: 2_000), .recognition(caughtUp: false, following: false, atMs: 2_100))
        XCTAssertEqual(p.feed(.silence(sinceMs: 3_000, atMs: 5_000)), [.holdStalled(reason: .recognition)])
        XCTAssertTrue(isStop(p.feed(.manualDiscard(atMs: 5_100))))
        XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 5_200)), [], "kein Wiederanlauf")
        XCTAssertEqual(p.feed(.manualRelease(atMs: 5_300)), [])
        XCTAssertEqual(p.feed(.unitStarted(atMs: 5_400)), [])
    }

    func testFirstTerminatingEventWins() {
        var a = policyWithRunningAnswer()
        _ = a.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(a.feed(.partial(text: "Berlin", atMs: 2_100)), [.cancelAnswer(merge: true)])
        XCTAssertEqual(a.feed(.manualRelease(atMs: 2_200)), [], "nach Fortsetzung wirkungslos")
        XCTAssertEqual(a.feed(.manualDiscard(atMs: 2_250)), [], "auch Verwerfen")
        var b = policyWithRunningAnswer()
        _ = b.feed(.speechStarted(atMs: 2_000))
        XCTAssertEqual(b.feed(.manualRelease(atMs: 2_100)), [.releaseAnswer])
        XCTAssertEqual(b.feed(.manualRelease(atMs: 2_200)), [], "zweite Handfreigabe: nichts")
    }

    // MARK: Wortklassen (ADR E4.2c Punkt 7 c): Rangfolge Stopp > Echo > Rückmeldelaut > relevant

    func testClassStopBeatsEchoWithoutException() {
        for text in ["moment", "stopp", "moment stopp", "halt"] {
            var p = policyWithRunningAnswer()
            _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Moment, stopp, halt: ich suche das heraus.", startedAtMs: 1_500, endedAtMs: nil))
            XCTAssertTrue(isStop(p.feed(.partial(text: text, atMs: 2_000, audioMs: 2_000))), "\(text): auch das Stoppwort der eigenen Einheit ist ein Stoppbefehl")
        }
    }

    func testClassEchoBeatsBackchannel() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Ja, das stimmt.", startedAtMs: 1_500, endedAtMs: nil))
        XCTAssertEqual(p.feed(.partial(text: "ja", atMs: 2_000, audioMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
    }

    func testClassBackchannelOnlyWhileAnAnswerIsActive() {
        var p = DuplexTurnPolicy()
        XCTAssertEqual(committed(feed(&p, .partial(text: "Ja", atMs: 100), .silence(sinceMs: defaults.endCompleteMs, atMs: 1_100))), ["Ja"])
    }

    func testClassStopWordWithMoreThanThreeWordsInTheFirstResultIsNotAStopEvenIfLaterResultsWouldFit() {
        var p = policyInGapAfterBridge()
        XCTAssertEqual(p.feed(.partial(text: "stopp das ist nicht richtig", atMs: 3_000)), [.cancelAnswer(merge: true)])
    }

    func testClassConsumedEchoStartsANewUtteranceThatIsClassifiedAnew() {
        var p = policySounding()
        // 5 Wörter, Echo: verbraucht. Das nächste Ergebnis ist eine neue Äußerung; ihr erstes Ergebnis hat 1 Wort: Stoppbefehl möglich.
        XCTAssertEqual(p.feed(.partial(text: "das wetter morgen wird sonnig", atMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        XCTAssertTrue(isStop(p.feed(.partial(text: "das wetter morgen wird sonnig stopp", atMs: 2_200))), "kumulativ: nur „stopp“ ist die neue Äußerung")
        var q = policySounding()
        _ = q.feed(.partial(text: "das wetter morgen wird sonnig", atMs: 2_000))
        XCTAssertEqual(q.feed(.partial(text: "das wetter morgen wird sonnig Berlin Hamburg", atMs: 2_300)), [.cancelAnswer(merge: true)], "danach folgende Wörter: neu eingestuft")
        var r = policySounding()
        _ = r.feed(.partial(text: "das wetter morgen wird sonnig", atMs: 2_000))
        XCTAssertTrue(isStop(r.feed(.partial(text: "stopp", atMs: 2_300))), "weniger Wörter als das Präfix: neue Erkennersitzung")
    }

    func testClassNormalizationFoldsCaseUmlautsAndPunctuation() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Für Größe und Übung gibt es Äpfel.", startedAtMs: 1_500, endedAtMs: nil))
        XCTAssertEqual(p.feed(.partial(text: "FUR, GROSSE und UBUNG gibt es!", atMs: 2_000, audioMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
        var q = policyWithRunningAnswer()
        XCTAssertTrue(isStop(q.feed(.partial(text: "  Stopp! ", atMs: 2_000))))
        var r = policyWithRunningAnswer()
        XCTAssertTrue(isStop(r.feed(.partial(text: "Moment…", atMs: 2_000))))
    }

    func testPartialWithoutAudioTimeStillCompilesAndKeepsLegacyEcho() {
        XCTAssertEqual(Input.partial(text: "a", atMs: 1), Input.partial(text: "a", atMs: 1, audioMs: nil))
        XCTAssertEqual(Input.recognition(caughtUp: true, atMs: 1), Input.recognition(caughtUp: true, following: true, atMs: 1))
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Das Wetter morgen wird sonnig.", startedAtMs: 1_500, endedAtMs: 2_800))
        XCTAssertEqual(p.feed(.partial(text: "wetter wird sonnig", atMs: 3_000)), [.ignored(reason: DuplexTurnPolicy.reasonEcho)])
    }

    // MARK: Einzelwort bei klingender Einheit bleibt bis zum Klangende (Matrix Zeile 3)

    func testRow3SingleWordSurvivesTheEndOfTheUtteranceAndIsConsumedExactlyOnceAtUnitEnd() {
        var p = policySounding()
        _ = feed(&p, .speechStarted(atMs: 2_000), .partial(text: "Berlin", atMs: 2_200))
        var during: [Action] = []
        for (since, at) in [(500, 2_800), (1_000, 3_300), (2_000, 4_300), (2_600, 4_900)] { during += p.feed(.silence(sinceMs: since, atMs: at)) }
        XCTAssertEqual(during, [], "2 s Stille, die Einheit klingt noch: nichts wird verbraucht, kein Abbruch, kein Commit")
        XCTAssertEqual(p.feed(.unitEnded(atMs: 5_000)),
                       [.cancelAnswer(merge: true), .commit(text: "Wie wird das Wetter morgen? Berlin", merged: true, afterInterruption: false)],
                       "Klangende: Fortsetzung mit dem Wort, die Äußerung war schon zu Ende, also gleich `commit`")
        XCTAssertEqual(p.feed(.unitEnded(atMs: 5_050)), [], "genau einmal")
        XCTAssertEqual(p.feed(.silence(sinceMs: 3_000, atMs: 5_400)), [], "kein zweiter Commit")
    }

    func testRow3SingleWordWithUtteranceNotYetEndedAtUnitEndWaitsForItsEnd() {
        var p = policySounding()
        _ = feed(&p, .speechStarted(atMs: 2_000), .partial(text: "Berlin", atMs: 2_200), .silence(sinceMs: 300, atMs: 2_600))
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_700)), [.cancelAnswer(merge: true)], "Fortsetzung, das Äußerungsende steht noch aus")
        XCTAssertEqual(committed(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 4_400))), ["Wie wird das Wetter morgen? Berlin"])
    }

    // MARK: Verbrauchte Wörter im kumulativen Text (Wortposition)

    func testConsumedBackchannelThenCumulativeTextCountsOnlyTheNewWords() {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .unitStarted(atMs: 1_500), .sentenceSpoken(text: "Das Wetter morgen wird sonnig.", startedAtMs: 1_500, endedAtMs: nil))
        _ = p.feed(.speechStarted(atMs: 1_900))
        XCTAssertEqual(p.feed(.partial(text: "ja", atMs: 2_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(p.feed(.partial(text: "ja Berlin", atMs: 2_300)), [], "nur „Berlin“ ist die neue Äußerung: ein Wort bei klingender Einheit bleibt stehen")
        XCTAssertEqual(p.feed(.unitEnded(atMs: 2_600)), [.cancelAnswer(merge: true)], "und zählt am Klangende, genau einmal")
        var q = policyInGapAfterBridge()
        XCTAssertEqual(q.feed(.partial(text: "ja", atMs: 3_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(q.feed(.partial(text: "ja Berlin", atMs: 3_300)), [.cancelAnswer(merge: true)], "in der Lücke: das neue Wort ist eine Fortsetzung")
        XCTAssertEqual(committed(q.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 5_400))), ["Wie wird das Wetter morgen? Berlin"], "ohne das verbrauchte „ja“")
    }

    func testRecognizerCorrectionOfAConsumedWordMakesNoNewUtterance() {
        var p = policyInGapAfterBridge()
        XCTAssertEqual(p.feed(.partial(text: "ja", atMs: 3_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        XCTAssertEqual(p.feed(.partial(text: "Jahr", atMs: 3_200)), [], "gleiche Position, nichts dahinter: keine neue Äußerung")
        XCTAssertEqual(p.feed(.silence(sinceMs: 2_000, atMs: 5_500)), [], "es bleibt nichts stehen")
        XCTAssertEqual(p.feed(.partial(text: "Jahr heute", atMs: 5_600)), [.cancelAnswer(merge: true)], "ein echtes neues Wort danach zählt wieder")
    }

    func testConsumedPrefixEndsWithTheUtterance() {
        var p = policyInGapAfterBridge()
        XCTAssertEqual(p.feed(.partial(text: "ja", atMs: 3_000)), [.ignored(reason: DuplexTurnPolicy.reasonBackchannel)])
        _ = p.feed(.silence(sinceMs: defaults.endCompleteMs, atMs: 4_000))
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 6_000)), [.cancelAnswer(merge: true)], "neue Äußerung beginnt vorn")
    }

    // MARK: Nachgereichte Antwort (detached): antwortet nicht auf die letzte Äußerung

    /// Die Frage ist beantwortet und abgeschlossen; später kommt ein Suchergebnis als eigene Antwort (`detached`).
    private func policyWithDetachedAnswer() -> DuplexTurnPolicy {
        var p = policyWithRunningAnswer()
        _ = feed(&p, .answerFinished(requestId: "r1"), .answerStarted(requestId: "d1", detached: true))
        return p
    }

    func testDetachedWordBeforeFirstSoundCancelsWithoutMergeAndCommitsOnlyNewWords() {
        var p = policyWithDetachedAnswer()
        _ = p.feed(.speechStarted(atMs: 5_000))
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 5_200)), [.cancelAnswer(merge: false)], "nie ein Zusammenführen mit der alten Frage")
        let end = p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 7_200))
        XCTAssertEqual(end, [.commit(text: "Berlin", merged: false, afterInterruption: false)],
                       "nur die neuen Wörter; nichts hat geklungen, also keine Unterbrechung")
    }

    func testDetachedTwoWordsWhileFirstUnitSoundsCancelsWithoutMergeAsInterruption() {
        var p = policyWithDetachedAnswer()
        _ = feed(&p, .unitStarted(atMs: 5_000), .sentenceSpoken(text: "Ich habe das Ergebnis der Suche gefunden.", startedAtMs: 5_000, endedAtMs: nil))
        _ = p.feed(.speechStarted(atMs: 5_300))
        XCTAssertEqual(p.feed(.partial(text: "Berlin heute", atMs: 5_700)), [.cancelAnswer(merge: false)],
                       "schon vor dem ersten fertigen Satz kein Zusammenführen")
        let end = p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 7_700))
        XCTAssertEqual(end, [.commit(text: "Berlin heute", merged: false, afterInterruption: true)], "ein Teil wurde gehört")
    }

    func testDetachedSingleWordWhileUnitSoundsWaitsForUnitEndThenCancelsWithoutMerge() {
        var p = policyWithDetachedAnswer()
        _ = feed(&p, .unitStarted(atMs: 5_000), .sentenceSpoken(text: "Ich habe das Ergebnis der Suche gefunden.", startedAtMs: 5_000, endedAtMs: nil))
        _ = feed(&p, .speechStarted(atMs: 5_300))
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 5_500)), [], "ein Wort bei klingender Einheit bleibt stehen")
        XCTAssertEqual(p.feed(.unitEnded(atMs: 6_000)), [.cancelAnswer(merge: false)])
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 7_700)),
                       [.commit(text: "Berlin", merged: false, afterInterruption: true)])
    }

    func testDetachedWordInGapAfterHeardUnitCancelsWithoutMergeAsInterruption() {
        var p = policyWithDetachedAnswer()
        _ = feed(&p, .unitStarted(atMs: 5_000), .sentenceSpoken(text: "Einen Moment.", startedAtMs: 5_000, endedAtMs: nil), .unitEnded(atMs: 5_800))
        _ = p.feed(.speechStarted(atMs: 6_000))
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 6_200)), [.cancelAnswer(merge: false)])
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 8_200)),
                       [.commit(text: "Berlin", merged: false, afterInterruption: true)])
    }

    func testDetachedStopWorksAsUsual() {
        var p = policyWithDetachedAnswer()
        _ = p.feed(.speechStarted(atMs: 5_000))
        XCTAssertTrue(isStop(p.feed(.partial(text: "Stopp", atMs: 5_200))))
        var q = policyWithDetachedAnswer()
        _ = feed(&q, .unitStarted(atMs: 5_000), .sentenceSpoken(text: "Ich habe das Ergebnis der Suche gefunden.", startedAtMs: 5_000, endedAtMs: nil))
        _ = q.feed(.speechStarted(atMs: 5_300))
        XCTAssertTrue(isStop(q.feed(.partial(text: "Stopp", atMs: 5_500))))
    }

    func testDetachedHoldsAsUsualWhileUserSpeaksWithoutWords() {
        var p = policyWithDetachedAnswer()
        XCTAssertEqual(p.feed(.speechStarted(atMs: 5_000)), [.holdAnswer], "Zeile 1: Antwort wartet, solange der Nutzer spricht")
        XCTAssertEqual(p.feed(.recognition(caughtUp: true, atMs: 5_100)), [])
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.holdQuietMs, atMs: 5_800)), [.releaseAnswer])
    }

    func testDetachedFlagEndsWithTheAnswerAndNextNormalAnswerMergesAgain() {
        var p = policyWithDetachedAnswer()
        _ = p.feed(.answerFinished(requestId: "d1"))
        _ = feed(&p, .speechStarted(atMs: 6_000), .partial(text: "Wie wird das Wetter übermorgen?", atMs: 6_300),
                 .silence(sinceMs: defaults.endCompleteMs, atMs: 7_200), .answerStarted(requestId: "r2"))
        _ = p.feed(.speechStarted(atMs: 7_400))
        XCTAssertEqual(p.feed(.partial(text: "Berlin", atMs: 7_600)), [.cancelAnswer(merge: true)], "normale Antwort: Zusammenführen wie gehabt")
    }

    func testDetachedDisplacedBeforeFirstSoundDoesNotCarryInterruptionIntoNextAnswer() {
        var p = policyWithDetachedAnswer()
        _ = feed(&p, .speechStarted(atMs: 5_000), .partial(text: "Berlin", atMs: 5_200))
        _ = p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 7_200))
        _ = p.feed(.answerStarted(requestId: "r3"))
        _ = feed(&p, .speechStarted(atMs: 8_000), .partial(text: "Hamburg", atMs: 8_200))
        XCTAssertEqual(p.feed(.silence(sinceMs: defaults.endOpenMs, atMs: 10_200)),
                       [.commit(text: "Berlin Hamburg", merged: true, afterInterruption: false)],
                       "normale Antwort der Frage „Berlin“: Zusammenführen, kein Rest der nachgereichten Antwort")
    }
}
