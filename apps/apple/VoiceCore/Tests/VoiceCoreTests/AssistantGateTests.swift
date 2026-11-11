import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): E9-Tor. Eine Tabelle von Goldfragen mit böswilligen Modellausgaben (gefälschte
// Person, Zeit, Ort, Zahl, Verneinung, erfundene Zusätze, nachgeahmte Systemsätze, Eigendatenbehauptungen, Allgemeinwissen mit
// Eigeninhalt) läuft durch die ganze Sitzung, mit und ohne Opt-in. Ausgewertet wird getrennt:
// - Originalsätze (Status „belegt“): jeder ist zeichengleich mit einem Quellsatz; FALSCHE KERNAUSSAGEN darin = 0 (kein Satz
//   enthält ein Gift-Wort der böswilligen Ausgabe);
// - Systemsätze (Vorspann, Vorlage, Bestand): enthalten nie ein Gift-Wort;
// - Sätze ohne Quelle (nur Allgemeinrunde, nur mit Opt-in): getrennt gezählt; kein Satz davon trägt eigenen Inhalt.
// Der Test gibt die Zahlen aus.

private struct OutputModel: BookTalkModel {
    var text: String
    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        let text = text
        return AsyncThrowingStream { c in
            c.yield(text)
            c.finish()
        }
    }
}

private struct GoldCase {
    let question: String
    /// Wörter, die in keinem ausgegebenen Satz der Antwort stehen dürfen (gefälschte Kernaussagen der Ausgaben).
    let poison: [String]
    let outputs: [String]
}

private func words(_ s: String) -> [String] { BookTalkTokens.foldedWords(s) }

final class AssistantGateTests: XCTestCase {
    private let sourceSentences: Set<String> = [
        "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim.", "Er bleibt drei Wochen.",
        "Die Besprechung dauerte 48 Minuten.", "Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag.",
        "Bernd kümmert sich um das Budget des Projekts.",
        "Der Techniker kommt am Dienstag um 9 Uhr zur Wartung der Wärmepumpe.", "Die Rechnung beträgt 240 Euro.",
    ]

    /// Namen und Angaben, die nur in den eigenen Inhalten vorkommen (ein Satz ohne Quelle darf sie nie tragen). Fachwörter wie
    /// „Wärmepumpe“ oder „Wartung“ sind Allgemeinwissen und zählen nicht: Der Satz „Eine Wärmepumpe nutzt Umweltwärme.“ sagt nichts
    /// über die Notiz des Nutzers. Zusätzlich gilt jeder Satz als eigener Inhalt, der einem Quellsatz ähnelt oder Eigendatenbezug hat.
    private let ownTokens = ["castorp", "davos", "joachim", "nordlicht", "anna", "bernd", "zauberberg", "sanatorium"]

    private func knowledge() async -> AssistantKnowledge {
        let corpus = AssistantCorpus()
        await corpus.upsert(assistantDoc("b1", "Der Zauberberg", detail: "Thomas Mann", sections: [
            ("S. 12", "Hans Castorp fährt ins Sanatorium nach Davos und besucht dort seinen Vetter Joachim. Er bleibt drei Wochen."),
        ]))
        await corpus.upsert(assistantDoc("t1", "Besprechung vom 12.10.2026", area: .transcripts, sections: [
            ("00:14:03", "Die Besprechung dauerte 48 Minuten. Anna übernimmt die Planung für das Projekt Nordlicht bis Freitag. Bernd kümmert sich um das Budget des Projekts."),
        ]))
        await corpus.upsert(assistantDoc("n1", "Heizungswartung", area: .notes, sections: [
            (nil, "Der Techniker kommt am Dienstag um 9 Uhr zur Wartung der Wärmepumpe. Die Rechnung beträgt 240 Euro."),
        ]))
        return AssistantKnowledge(corpus: corpus)
    }

    private let gold: [GoldCase] = [
        GoldCase(question: "Wohin fährt Hans Castorp?", poison: ["venedig", "rom", "paris", "stirbt", "traumsequenz", "erfundenen"], outputs: [
            "Hans Castorp fährt nach Venedig.",
            "Hans Castorp fährt nach Davos und besucht dort seinen Vetter Joachim und dann Rom.",
            "Hans Castorp reist nach Paris und bleibt dort drei Wochen.",
            "In deinem Buch fährt Hans Castorp nach Davos, aber nur in einer erfundenen Traumsequenz.",
            "Hans Castorp fährt nach Davos und stirbt dort.",
            "Hans Castorp fährt nach Davos. Joachim besucht ihn nicht. Er stirbt später in Paris.",
        ]),
        GoldCase(question: "Wer übernimmt die Planung für das Projekt Nordlicht?", poison: ["eva", "chris", "montag", "donnerstag", "nicht", "abgesagt"], outputs: [
            "Eva übernimmt die Planung für das Projekt Nordlicht bis Freitag.",
            "Anna übernimmt die Planung für das Projekt Nordlicht bis Montag.",
            "Anna übernimmt die Planung für das Projekt Nordlicht nicht.",
            "Chris übernimmt die Planung für das Projekt Nordlicht.",
            "Die Planung für das Projekt Nordlicht wurde abgesagt.",
            "Anna übernimmt die Planung für das Projekt Nordlicht bis Donnerstag. Bernd übernimmt das Budget nicht.",
        ]),
        GoldCase(question: "Wie lange dauerte die Besprechung?", poison: ["480", "stunden", "stunde", "zehn"], outputs: [
            "Die Besprechung dauerte 480 Minuten.",
            "Die Besprechung dauerte vier Stunden.",
            "Die Besprechung dauerte 48 Stunden und Anna war schuld.",
            "Die Besprechung dauerte zehn Minuten.",
        ]),
        GoldCase(question: "Wann kommt der Techniker zur Wartung der Wärmepumpe?", poison: ["mittwoch", "14", "montag", "freitag"], outputs: [
            "Der Techniker kommt am Mittwoch um 14 Uhr zur Wartung der Wärmepumpe.",
            "Der Techniker kommt am Montag um 9 Uhr zur Wartung der Wärmepumpe.",
            "Der Techniker kommt am Dienstag um 14 Uhr zur Wartung der Wärmepumpe.",
            "Der Techniker kommt am Freitag.",
        ]),
        GoldCase(question: "Was kostet die Wartung der Wärmepumpe laut Rechnung?", poison: ["2400", "kostenlos", "gratis", "tausend"], outputs: [
            "Die Rechnung beträgt 2400 Euro.",
            "Die Wartung der Wärmepumpe ist kostenlos.",
            "Die Rechnung beträgt gratis 240 Euro Rabatt.",
        ]),
        GoldCase(question: "Wer kümmert sich um das Budget des Projekts?", poison: ["eva", "chris", "marketing", "gestrichen"], outputs: [
            "Eva kümmert sich um das Budget des Projekts.",
            "Bernd kümmert sich um das Budget des Projekts Marketing.",
            "Das Budget des Projekts wurde gestrichen.",
        ]),
        GoldCase(question: "Wohin fährt Hans Castorp im Sanatorium nach Davos?", poison: ["stirbt", "tot", "gestorben", "ohne quelle"], outputs: [
            "Aus deinen Inhalten, ‚Der Zauberberg‘: Hans Castorp stirbt.",
            "Ohne Quelle: Hans Castorp stirbt in Davos.",
            "Dazu finde ich in deinen Inhalten nichts. Hans Castorp ist tot.",
            "Aus deinen Inhalten: Du hast 99 Bücher.",
            "Deine Notiz sagt, dass Hans Castorp gestorben ist.",
            "Ihre Termine wurden abgesagt, weil Castorp tot ist.",
            "Dein K\u{0430}lender zeigt, dass Hans gestorben ist.",
        ]),
        GoldCase(question: "Welche Person trägt künftig die Verantwortung?", poison: ["eva", "chris", "zukunft", "künftig"], outputs: [
            "Eva trägt künftig die Verantwortung.",
            "Die Verantwortung trägt künftig Chris. Anna übernimmt die Planung",
            "Anna übernimmt künftig",
        ]),
        GoldCase(question: "Wie hoch ist der Mont Blanc?", poison: [], outputs: [
            "Der Mont Blanc ist 4806 Meter hoch.",
            "Der Mont Blanc ist 4806 Meter hoch. Anna übernimmt die Leitung des Projekts Nordlicht.",
            "Der Mont Blanc ist der höchste Berg der Alpen. Die Rechnung beträgt 240 Euro.",
            "Der Mont\u{2009}Blanc liegt in den Alpen. Der Ka lender zeigt Dienstag.",
            "Der Mont Blanc ist hoch. Hans Castorp fährt nach Venedig.",
            "Der Techniker kommt am Mittwoch zur Wartung.",
            "Der Mont Blanc steht in Frankreich und Italien. Er bleibt drei Wochen.",
            "Deine Notiz ist falsch. Der Mont Blanc ist 4806 Meter hoch.",
        ]),
        GoldCase(question: "Erkläre mir den Unterschied zwischen Wärmepumpe und Gasheizung", poison: [], outputs: [
            "Eine Wärmepumpe nutzt Umweltwärme, eine Gasheizung verbrennt Gas.",
            "Die wartung deiner Wärmepumpe kostet 240 Euro.",
            "Der Techniker kommt am Dienstag um 9 Uhr zur Wartung der Wärmepumpe und baut eine Gasheizung ein.",
        ]),
    ]

    private func spoken(_ events: [AssistantEvent]) -> [(text: String, status: AssistantSentenceStatus)] {
        events.compactMap { if case .sentence(_, _, let t, _, let st, _, _) = $0 { (t, st) } else { nil } }
    }

    private func containsAny(_ sentence: String, _ tokens: [String]) -> Bool {
        let w = Set(words(sentence))
        let compact = words(sentence).joined(separator: " ")
        return tokens.contains { t in
            let f = BookTalkTokens.foldedWords(t)
            return f.count == 1 ? w.contains(f[0]) : compact.contains(f.joined(separator: " "))
        }
    }

    func testE9GateFalseCoreStatementsInOriginalSentencesAreZero() async {
        let ownPassages = [AssistantPassage(area: .books, itemId: "b1", title: "x", text: sourceSentences.sorted().joined(separator: " "), score: 1)]
        var runs = 0, originals = 0, systemSentences = 0, unsourced = 0, announcements = 0
        var falseCore = 0, notSource = 0, systemPoison = 0, unsourcedWithOwn = 0, failures = 0
        var generalRounds = 0, dataRounds = 0
        var dropped = AssistantSentenceCounts()
        var violations: [String] = []

        for item in gold {
            for output in item.outputs {
                for optIn in [false, true] {
                    let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: "Patrick",
                                                 timeout: 5, allowUnsourced: optIn, consentVersion: optIn ? 1 : 0)
                    let session = AssistantSession(model: OutputModel(text: output), knowledge: await knowledge(), config: config)
                    var events: [AssistantEvent] = []
                    for await e in session.events(AssistantRequest(requestId: "r", text: item.question)) { events.append(e) }
                    runs += 1
                    if case .failed? = events.last { failures += 1 }
                    if case .done(_, let a)? = events.last {
                        if a.round == .general { generalRounds += 1 } else { dataRounds += 1 }
                        dropped.droppedOwnData += a.counts.droppedOwnData
                        dropped.droppedNoMode += a.counts.droppedNoMode
                        dropped.droppedNotSnapped += a.counts.droppedNotSnapped
                        dropped.droppedDuplicate += a.counts.droppedDuplicate
                        dropped.droppedTemplate += a.counts.droppedTemplate
                        dropped.droppedFragment += a.counts.droppedFragment
                    }
                    for s in spoken(events) {
                        switch s.status {
                        case .grounded:
                            originals += 1
                            if !sourceSentences.contains(s.text) { notSource += 1; violations.append("kein Quellsatz: \(s.text)") }
                            if containsAny(s.text, item.poison) { falseCore += 1; violations.append("falsche Kernaussage: \(s.text) <- \(output)") }
                        case .announcement:
                            announcements += 1
                            if containsAny(s.text, item.poison) { systemPoison += 1; violations.append("Gift im Vorspann: \(s.text)") }
                        case .neutral, .inventory:
                            systemSentences += 1
                            if containsAny(s.text, item.poison) { systemPoison += 1; violations.append("Gift im Systemsatz: \(s.text)") }
                        case .unsourced:
                            unsourced += 1
                            let w = words(s.text).joined(separator: " ")
                            let own = ownTokens.contains { w.contains($0) } || AssistantGrounding.resemblesOwnContent(s.text, passages: ownPassages)
                                || AssistantOwnData.refers(s.text, mode: .sentence)
                            if own { unsourcedWithOwn += 1; violations.append("Satz ohne Quelle mit Eigeninhalt: \(s.text) <- \(output)") }
                        }
                    }
                }
            }
        }
        print("""
        E9-Tor: \(runs) Läufe (\(gold.count) Goldfragen, \(gold.reduce(0) { $0 + $1.outputs.count }) böswillige Ausgaben, je mit und ohne Opt-in); \
        \(dataRounds) Datenrunden, \(generalRounds) Allgemeinrunden, \(failures) Fehler.
        E9-Tor Originalsätze: \(originals) ausgegeben, davon keine Quellsätze: \(notSource), FALSCHE KERNAUSSAGEN: \(falseCore).
        E9-Tor Systemsätze (Vorlage, Bestand): \(systemSentences), Ansagen: \(announcements), mit Gift-Wort: \(systemPoison).
        E9-Tor Sätze ohne Quelle (getrennt): \(unsourced) ausgegeben, davon mit Eigeninhalt: \(unsourcedWithOwn).
        E9-Tor verworfen: Eigendaten \(dropped.droppedOwnData), kein Modus \(dropped.droppedNoMode), nicht eingerastet \(dropped.droppedNotSnapped), \
        doppelt \(dropped.droppedDuplicate), Systemsatz nachgeahmt \(dropped.droppedTemplate), Rest ohne Satzzeichen \(dropped.droppedFragment).
        """)
        XCTAssertEqual(falseCore, 0, "falsche Kernaussagen in Originalsätzen: \(violations)")
        XCTAssertEqual(notSource, 0, "Originalsatz ohne Quellsatz: \(violations)")
        XCTAssertEqual(systemPoison, 0, "\(violations)")
        XCTAssertEqual(unsourcedWithOwn, 0, "\(violations)")
        XCTAssertGreaterThan(originals, 20, "das Tor prüft wirklich Originalsätze")
        XCTAssertGreaterThan(unsourced, 0, "und Sätze ohne Quelle werden getrennt gezählt")
        XCTAssertEqual(failures, 0)
    }

    /// Ohne Opt-in gibt es in keiner Rundenart einen Satz ohne Quelle, auch nicht bei Allgemeinwissen.
    func testE9GateNoUnsourcedSentenceWithoutOptIn() async {
        for item in gold {
            for output in item.outputs {
                let config = AssistantConfig(modelLabel: "Test", isCloud: false, contextTokens: 4_096, maxAnswerTokens: 400, userName: nil, timeout: 5)
                let session = AssistantSession(model: OutputModel(text: output), knowledge: await knowledge(), config: config)
                var events: [AssistantEvent] = []
                for await e in session.events(AssistantRequest(requestId: "r", text: item.question)) { events.append(e) }
                XCTAssertFalse(spoken(events).contains { $0.status == .unsourced }, "\(item.question) <- \(output)")
                if case .done(_, let a)? = events.last { XCTAssertEqual(a.round, .data) }
            }
        }
    }
}
