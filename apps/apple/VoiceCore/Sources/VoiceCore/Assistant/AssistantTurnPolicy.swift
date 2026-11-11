import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Rundenart, deterministisch VOR dem Modellaufruf. Reiner Typ ohne Zustand.
//
// `.data` ist der Standard. `.general` nur, wenn alles gilt:
//  1. der Opt-in „Antworten ohne Quelle“ ist an;
//  2. die Frage hat keinen Eigendatenbezug (`AssistantOwnData`, Fragemodus, mit den Titeln aus dem Bestand);
//  3. der Abruf liefert keinen Auszug oberhalb der Relevanzschwelle;
//  4. keine Übernahme aus der Vorrunde: War sie eine Datenrunde, darf die Frage nicht anaphorisch sein (er, sie, es, ihn, ihm,
//     ihr, ihnen, das, dies*, dort, da*, „und“ am Anfang) und muss mindestens drei Inhaltswörter haben.
// Ausnahme: Enthält die Frage ausdrücklich „ohne Quelle“ oder „allgemein“, gilt (1) und (2) genügen; Abruftreffer und
// Vorrunde ändern dann nichts.
//
// Relevanzschwelle (der Abruf liefert nur je Bereich normierte Werte, 1,0 = bester Treffer des Bereichs, das taugt nicht als
// absolute Schwelle): Ein Auszug ist relevant, wenn Titel und Text mindestens zwei verschiedene Inhaltswörter der Frage
// decken (bei einer Frage mit nur einem Inhaltswort: dieses), gedeckt wie in der Belegprüfung (gleiches Wort oder
// gemeinsamer Anfang ab fünf Zeichen). Ein einzelnes zufälliges Wort („Haus“) macht keine Datenrunde; zwei Wörter derselben
// Frage in einem Auszug sind ein Treffer. Eine Frage ganz ohne Inhaltswort kann nichts ausschließen: dann gilt jeder
// Auszug als relevant.

public enum AssistantTurnPolicy {
    public enum Reason: String, Sendable, Equatable {
        /// Opt-in aus: jede Runde ist eine Datenrunde.
        case optedOut
        case ownData
        case relevantPassage
        /// Anschlussfrage an eine Datenrunde.
        case carryOver
        /// Die Frage verlangt ausdrücklich „ohne Quelle“ oder „allgemein“.
        case explicitGeneral
        case general
    }

    public struct Decision: Sendable, Equatable {
        public let kind: AssistantRoundKind
        public let reason: Reason
    }

    /// Mindestzahl verschiedener Inhaltswörter der Frage, die ein Auszug decken muss (kleiner bei weniger Wörtern).
    static let relevantWords = 2
    /// Weniger Inhaltswörter gelten als Anschlussfrage.
    static let minContentWords = 3

    public static func decide(question: String, allowUnsourced: Bool, titles: [String], passages: [AssistantPassage],
                              previous: AssistantRoundKind?) -> Decision {
        guard allowUnsourced else { return Decision(kind: .data, reason: .optedOut) }
        if AssistantOwnData.refers(question, mode: .question, titles: titles) { return Decision(kind: .data, reason: .ownData) }
        if explicitlyGeneral(question) { return Decision(kind: .general, reason: .explicitGeneral) }
        if passages.contains(where: { isRelevant($0, to: question) }) { return Decision(kind: .data, reason: .relevantPassage) }
        if previous == .data, isAnaphoric(question) || BookTalkTokens.contentWords(question).count < minContentWords {
            return Decision(kind: .data, reason: .carryOver)
        }
        return Decision(kind: .general, reason: .general)
    }

    // MARK: Relevanz

    public static func isRelevant(_ passage: AssistantPassage, to question: String) -> Bool {
        let wanted = Set(BookTalkTokens.contentWords(question))
        guard !wanted.isEmpty else { return true }
        let words = BookTalkTokens.foldedWords(passage.title + " " + passage.text)
        let hits = wanted.filter { BookTalkCitations.covered($0, by: words) }.count
        return hits >= min(relevantWords, wanted.count)
    }

    // MARK: Fragearten

    /// „ohne Quelle“ (auch „ohne Quellen“) oder ein Wort mit Anfang „allgemein“.
    static func explicitlyGeneral(_ question: String) -> Bool {
        let words = BookTalkTokens.foldedWords(question)
        if words.contains(where: { $0.hasPrefix("allgemein") }) { return true }
        for i in words.indices.dropLast() where words[i] == "ohne" && words[i + 1].hasPrefix("quelle") { return true }
        return false
    }

    private static let pronouns: Set<String> = ["er", "sie", "es", "ihn", "ihm", "ihr", "ihnen", "das", "dort"]

    /// Anaphorisch: Pronomen, „dies*“, „da*“ oder „und“ am Anfang. Bewusst weit (auch „dass“ zählt, wegen „da*“).
    static func isAnaphoric(_ question: String) -> Bool {
        let words = BookTalkTokens.foldedWords(question)
        if words.first == "und" { return true }
        return words.contains { pronouns.contains($0) || $0.hasPrefix("dies") || $0.hasPrefix("da") }
    }
}
