import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a „Antworten ohne Quelle“): Satzprüfung VOR der Ausgabe, strukturell.
// Reine Funktionen, kein Modell, kein Zustand. Wortlisten und Heuristiken sind keine Sicherheitsgrenze mehr; die Grenze ist:
//
// - DATENRUNDE (Standard, mit und ohne Opt-in): Kein Satz des Modells wird je wörtlich ausgegeben. Jeder vollständige
//   Modellsatz „rastet ein“: Der beste Quellsatz der Auszüge nach Wortdeckung (Deckung ≥ 60 % und mindestens zwei gedeckte
//   Inhaltswörter) wird im Originalwortlaut ausgegeben, Quelle = dieser Auszug. Sonst entfällt der Satz. Ausgegeben wird
//   also nur ein Quellsatz, eine Vorlage der festen Liste oder ein Satz des Systems (Uhr, Bestand, feste Sätze).
// - ALLGEMEINRUNDE (nur mit Opt-in, Prompt ohne Auszüge und ohne Bestand): Vorlage → neutral; Eigendatenbezug
//   (`AssistantOwnData`) → verworfen; Ähnlichkeit mit eigenem Inhalt (`resemblesOwnContent`, Abruf mit dem Satz als Anfrage)
//   → verworfen; sonst „ohne Quelle“. Nie grounded. Die Listen und die Ähnlichkeit können nur verbieten, nie erlauben.
// - Zitierbar ist nur ein vollständiger Satz mit Satzzeichen am Ende. Erster und letzter Satz eines Auszugs zählen nur, wenn
//   der Auszug nachweislich an einer Satzgrenze beginnt bzw. endet (`AssistantPassage.startsAtBoundary/endsAtBoundary`).
//
// Vorlagen sind ganze Sätze einer festen Liste; verglichen wird auf Gleichheit nach Normalisierung (klein, ohne
// Satzzeichen). Keine Präfix-, Frage-, Angebots-, Füllwort- oder Könnens-Regeln. Fehl-Vorlagen („Dazu finde ich in deinen
// Inhalten nichts.“) spricht nur das System; Modelltext mit diesem Wortlaut wird verworfen.

public enum AssistantGrounding {
    // MARK: Feste Sätze und Ansagen

    public static let unsourcedAnnouncement = "Ohne Quelle:"
    /// Vorspann vor Bestandsangaben (ohne Titel).
    public static let inventoryAnnouncement = "Aus deinen Inhalten:"
    /// Ohne Opt-in, wenn von einer Antwort nichts Gesprochenes übrig bleibt.
    public static let nothingFoundSentence = "Dazu finde ich in deinen Inhalten nichts."
    public static let optInHint = "Antworten ohne Quelle kannst du im Menü einschalten."
    /// Mit Opt-in, wenn nichts Belegtes (und nichts Unbelegtes) bleibt.
    public static let nothingGroundedSentence = "Dazu finde ich nichts Belegtes."

    public static func fixedSentence(allowUnsourced: Bool) -> String {
        allowUnsourced ? nothingGroundedSentence : nothingFoundSentence + " " + optInHint
    }

    /// Längster Titel im Vorspann (länger: gekürzt mit „…“).
    static let maxPrefaceTitle = 80

    /// Vorspann vor dem Zitat einer Quelle: „Aus deinen Inhalten, ‚Der Zauberberg‘:“ (ein Wortlaut für alle Bereiche).
    public static func quotePreface(for passage: AssistantPassage) -> String {
        var title = passage.title.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        title = String(title.filter { !"‚‘’'“”„\"".contains($0) }).trimmingCharacters(in: .whitespaces)
        if title.isEmpty { title = passage.area.singularLabel }
        if title.count > maxPrefaceTitle { title = String(title.prefix(maxPrefaceTitle - 1)).trimmingCharacters(in: .whitespaces) + "…" }
        return "Aus deinen Inhalten, ‚\(title)‘:"
    }

    static func normalize(_ s: String) -> String { BookTalkNormalize.words(s).joined(separator: " ") }

    // MARK: Vorlagen

    /// Geschlossene Liste: Grüße, Dank, Abschied, Bestätigungen und wenige feste Rückfragen. Sagt nichts über eigene Inhalte.
    /// „Ja.“ und „Nein.“ gehören nicht dazu.
    static let templateSentences: [String] = [
        "Hallo!", "Hi!", "Guten Morgen!", "Guten Tag!", "Guten Abend!", "Gute Nacht!", "Hallo, wie kann ich helfen?",
        "Wie kann ich dir helfen?", "Ich höre dich.", "Ja, ich höre dich.", "Ich bin da.", "Ja, ich bin da.",
        "Danke!", "Danke schön!", "Gern geschehen!", "Gerne!", "Bitte!", "Kein Problem.", "Okay.", "Alles klar.", "Verstanden.",
        "Tschüss!", "Bis später!", "Bis bald!", "Auf Wiedersehen!", "Bis zum nächsten Mal!",
        "Soll ich dir die Stelle vorlesen?", "Möchtest du mehr dazu hören?", "Möchtest du mehr erfahren?",
        "Soll ich weitersuchen?", "Soll ich anders suchen?", "Kannst du das bitte wiederholen?",
        "Das habe ich nicht verstanden.", "Das weiß ich nicht.", "Ich weiß es nicht.",
    ]

    private static let templateIndex: [String: String] = {
        var map: [String: String] = [:]
        for t in templateSentences { map[normalize(t)] = t }
        return map
    }()

    /// Die Vorlage (in ihrem festen Wortlaut), wenn `sentence` ganz einer entspricht.
    public static func template(matching sentence: String) -> String? { templateIndex[normalize(sentence)] }

    private static let failIndex: Set<String> = {
        var set: Set<String> = [nothingFoundSentence, optInHint, nothingGroundedSentence, "Dazu finde ich in deinen Inhalten leider nichts.",
                                nothingFoundSentence + " " + optInHint].reduce(into: Set<String>()) { $0.insert(normalize($1)) }
        set.insert(normalize(unsourcedAnnouncement))
        set.insert(normalize(inventoryAnnouncement))
        return set
    }()

    /// Fehl-Vorlagen und Ansagen: spricht nur das System. Modelltext mit diesem Wortlaut wird verworfen.
    public static func isSystemOnly(_ sentence: String) -> Bool {
        failIndex.contains(normalize(sentence)) || BookTalkCitations.isAbsentTemplate(sentence)
            || normalize(sentence).hasPrefix(normalize("Aus deinen Inhalten"))
    }

    // MARK: Uhrzeit und Datum

    /// Genau die Sätze, die das System aus der Uhr erzeugt („Es ist 08:05 Uhr.“, „Heute ist Mittwoch, der 11. November 2026.“
    /// und wenige Varianten). Modelltext gilt nur bei Gleichheit als Vorlage.
    public static func clockSentences(now: Date, timeZone: TimeZone) -> [String] {
        let parts = AssistantPrompt.germanDate(now, timeZone: timeZone).components(separatedBy: ", ")
        guard parts.count == 3 else { return [] }
        let (weekday, date, time) = (parts[0], parts[1], parts[2])
        return [
            "Es ist \(time).", "Es ist jetzt \(time).",
            "Heute ist \(weekday), \(date).", "Heute ist \(weekday).", "Heute ist \(date).",
            "Es ist \(weekday), \(date), \(time).",
        ]
    }

    /// Der Wortlaut aus `expected`, wenn `sentence` ihm nach Normalisierung gleicht.
    public static func clockSentence(matching sentence: String, expected: [String]) -> String? {
        let n = normalize(sentence)
        return expected.first { normalize($0) == n }
    }

    // MARK: Quellsätze

    /// Ein vollständiger Satz eines Auszugs im Originalwortlaut (Leerraum zusammengezogen).
    public struct SourceSentence: Sendable, Equatable {
        public let text: String
        public let passage: AssistantPassage
        let words: [String]
        let content: [String]

        init(text: String, passage: AssistantPassage) {
            self.text = text
            self.passage = passage
            self.words = BookTalkTokens.foldedWords(text)
            self.content = BookTalkTokens.contentWords(text).filter { !AssistantGrounding.ignoring.contains($0) }
        }

        /// Schlüssel zum Erkennen doppelter Zitate.
        public var key: String { "\(passage.area.rawValue)/\(passage.itemId)/\(passage.locator ?? "")/\(text)" }
        /// Quelle (Bereich und Element), für den Vorspann.
        public var sourceKey: String { "\(passage.area.rawValue)/\(passage.itemId)" }
    }

    /// Längster Quellsatz, der ausgegeben wird; längere werden nicht zitiert (ein gekürzter wäre ein Fragment).
    static let maxQuoteChars = 600

    /// Die zitierbaren Quellsätze aller Auszüge in Auszug-Reihenfolge. Nur vollständige Sätze: Beginn mit Großbuchstaben, Ziffer
    /// oder Anführungszeichen, Ende mit Satzzeichen (keine Ausnahme, auch nicht für einen Auszug aus nur einem Satz),
    /// mindestens drei Wörter. Der erste Satz eines Auszugs fehlt, wenn der Auszug nicht nachweislich an einer Satzgrenze
    /// beginnt, der letzte, wenn er nicht nachweislich an einer endet oder gekürzt wurde (Ende „…“): beide können angeschnitten
    /// sein. Zeilenumbrüche trennen Sätze.
    public struct Basis: Sendable {
        let sentences: [SourceSentence]

        public init(passages: [AssistantPassage]) {
            var out: [SourceSentence] = []
            for p in passages {
                var units = AssistantGrounding.sourceTexts(of: p.text)
                if !p.startsAtBoundary, !units.isEmpty { units.removeFirst() }
                let clipped = p.text.trimmingCharacters(in: .whitespacesAndNewlines).hasSuffix("…")
                if !p.endsAtBoundary || clipped, !units.isEmpty { units.removeLast() }
                out += units.filter { AssistantGrounding.isQuotable($0) }.map { SourceSentence(text: $0, passage: p) }
            }
            sentences = out
        }

        /// Erkennungsbasis (nicht zum Zitieren): ALLE Sätze der Auszüge, auch Randsätze und Sätze ohne Satzzeichen. Dient nur
        /// dazu, einen Satz ohne Quelle zu verwerfen, der einem eigenen Inhalt ähnelt.
        init(detecting passages: [AssistantPassage]) {
            sentences = passages.flatMap { p in AssistantGrounding.sourceTexts(of: p.text).map { SourceSentence(text: $0, passage: p) } }
        }

        public var isEmpty: Bool { sentences.isEmpty }
        public var texts: [String] { sentences.map(\.text) }
    }

    /// Alle Sätze eines Textes (Zeilen, dann Satzgrenzen) mit zusammengezogenem Leerraum, auch die nicht zitierbaren, damit der
    /// letzte erkennbar bleibt.
    static func sourceTexts(of text: String) -> [String] {
        var out: [String] = []
        for line in text.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard !trimmed.isEmpty else { continue }
            let units = Array(trimmed.utf16)
            for s in BookTalkSentences.split(trimmed) {
                let raw = String(decoding: units[s.start..<s.end], as: UTF16.self)
                let sentence = raw.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
                if !sentence.isEmpty { out.append(sentence) }
            }
        }
        return out
    }

    private static let openers: Set<Character> = ["„", "“", "‚", "‘", "»", "«", "\"", "'", "(", "‹", "–", "—"]
    private static let closers: Set<Character> = ["“", "”", "‘", "’", "\"", "'", "»", "«", ")", "›", "]"]

    static func isQuotable(_ s: String) -> Bool {
        guard s.count >= 12, s.count <= maxQuoteChars, s.split(separator: " ").count >= 3, let first = s.first else { return false }
        guard first.isUppercase || first.isNumber || openers.contains(first) else { return false }
        var tail = Substring(s)
        while let last = tail.last, closers.contains(last) { tail = tail.dropLast() }
        guard let end = tail.last else { return false }
        return ".!?…".contains(end)
    }

    // MARK: Einrasten

    /// Wörter, die beim Einrasten nicht als Inhalt zählen: Verweise auf die Quelle und Funktionswörter. Sie beeinflussen nur
    /// die Auswahl, nie die Sicherheit (ausgegeben wird immer der Quellsatz).
    static let ignoring: Set<String> = baseFunctionWords.union(appNouns)

    private static let baseFunctionWords: Set<String> = [
        "hast", "habe", "habt", "bist", "seid", "kannst", "wirst", "musst", "sollst", "willst", "gerade", "leider", "gerne", "gern",
        "genau", "wirklich", "ubrigens", "ausserdem", "zudem", "deine", "deinen", "deinem", "deiner", "deines", "eure", "euer",
        "laut", "gemass", "gesagt", "notiert", "erwahnt", "geschrieben", "gelesen", "aufgenommen", "besprochen", "gespeichert",
    ]

    static let appNouns: Set<String> = [
        "buch", "buches", "bucher", "buchern", "buchs", "horbuch", "horbucher", "roman", "romane", "kapitel", "kapiteln",
        "besprechung", "besprechungen", "meeting", "meetings", "protokoll", "protokolle", "protokolls", "transkript",
        "transkripte", "transkripten", "transkripts", "mitschrift", "mitschriften", "notiz", "notizen", "sprachnotiz",
        "sprachnotizen", "ablage", "dokument", "dokumente", "dokumenten", "dokuments", "datei", "dateien", "aufnahme",
        "aufnahmen", "bibliothek", "inhalte", "inhalten",
    ]

    /// Wie viele Inhaltswörter `content` im Satz gedeckt sind.
    private static func covered(_ content: [String], by words: [String]) -> Int {
        content.filter { BookTalkCitations.covered($0, by: words) }.count
    }

    private static func better(_ a: (share: Double, covered: Int, precision: Double), than b: (share: Double, covered: Int, precision: Double)) -> Bool {
        if a.share != b.share { return a.share > b.share }
        if a.covered != b.covered { return a.covered > b.covered }
        return a.precision > b.precision
    }

    /// Der beste Quellsatz zu einem Modellsatz: Deckung der Inhaltswörter des Modellsatzes ≥ `BookTalkCitations.minCoverage` (60 %)
    /// und mindestens zwei gedeckte Inhaltswörter. Gleichstand: mehr gedeckte Wörter, dann der kürzere Quellsatz (Präzision),
    /// dann die frühere Stelle. `nil`, wenn keiner einrastet.
    public static func snap(_ sentence: String, basis: Basis) -> SourceSentence? {
        let content = Array(Set(BookTalkTokens.contentWords(sentence).filter { !ignoring.contains($0) }))
        guard content.count >= 2 else { return nil }
        var best: (SourceSentence, (share: Double, covered: Int, precision: Double))?
        for q in basis.sentences {
            let hits = covered(content, by: q.words)
            let share = Double(hits) / Double(content.count)
            guard hits >= 2, share >= BookTalkCitations.minCoverage else { continue }
            let score = (share: share, covered: hits, precision: Double(hits) / Double(max(q.content.count, 1)))
            if best == nil || better(score, than: best!.1) { best = (q, score) }
        }
        return best?.0
    }

    /// Rückfall, wenn nichts einrastet: der beste Quellsatz gegen die FRAGE (mindestens zwei gedeckte Inhaltswörter oder
    /// mindestens 50 % der Inhaltswörter der Frage). Sätze aus `taken` (schon ausgegeben) zählen nicht.
    public static func fallback(question: String, basis: Basis, taken: Set<String> = []) -> SourceSentence? {
        let content = Array(Set(BookTalkTokens.contentWords(question).filter { !ignoring.contains($0) }))
        guard !content.isEmpty else { return nil }
        var best: (SourceSentence, (share: Double, covered: Int, precision: Double))?
        for q in basis.sentences where !taken.contains(q.key) {
            let hits = covered(content, by: q.words)
            let share = Double(hits) / Double(content.count)
            guard hits >= 1, hits >= 2 || share >= 0.5 else { continue }
            let score = (share: share, covered: hits, precision: Double(hits) / Double(max(q.content.count, 1)))
            if best == nil || better(score, than: best!.1) { best = (q, score) }
        }
        return best?.0
    }

    /// Ähnelt ein Satz ohne Quelle einem Satz aus den eigenen Inhalten? `passages` sind die Treffer eines Abrufs mit dem SATZ als
    /// Anfrage über die freigegebenen Bereiche. Gilt die Einrast-Regel (Deckung ≥ 60 % und mindestens zwei Inhaltswörter) für
    /// irgendeinen Quellsatz (auch Randsätze), wird der Satz verworfen. Kann nur verbieten.
    public static func resemblesOwnContent(_ sentence: String, passages: [AssistantPassage]) -> Bool {
        guard !passages.isEmpty else { return false }
        return snap(sentence, basis: Basis(detecting: passages)) != nil
    }

    // MARK: Urteil über einen Modellsatz

    public enum Verdict: Sendable, Equatable {
        /// Datenrunde: der Quellsatz, der statt des Modellsatzes ausgegeben wird.
        case quote(SourceSentence)
        /// Vorlage in ihrem festen Wortlaut (Liste oder Uhr).
        case template(String)
        /// Allgemeinrunde: Satz ohne Quelle.
        case unsourced
        case dropped(AssistantDropReason)
    }

    /// Urteil über einen vollständigen Modellsatz (Satzzeichen oder Stromende, Markdown schon entfernt).
    /// - `taken`: Schlüssel der in dieser Antwort schon ausgegebenen Zitate.
    /// - `titles`: Titel des Bestands (für den Eigendatenbezug).
    /// - `optIn`: Opt-in jetzt (nicht bei Beginn).
    /// - `raw`: der Satz, wie er im Strom stand (für die Prüfung auf ungewöhnliche Leerzeichen); sonst `sentence`.
    public static func judge(_ sentence: String, round: AssistantRoundKind, basis: Basis, titles: [String], clock: [String],
                             optIn: Bool, taken: Set<String> = [], raw: String? = nil) -> Verdict {
        if let t = template(matching: sentence) { return .template(t) }
        if let c = clockSentence(matching: sentence, expected: clock) { return .template(c) }
        if isSystemOnly(sentence) { return .dropped(.template) }
        switch round {
        case .data:
            if let q = snap(sentence, basis: basis) { return taken.contains(q.key) ? .dropped(.duplicate) : .quote(q) }
            if AssistantOwnData.refers(sentence, mode: .sentence, titles: titles) { return .dropped(.ownData) }
            return .dropped(optIn ? .notSnapped : .noMode)
        case .general:
            if AssistantOwnData.refers(raw ?? sentence, mode: .sentence, titles: titles) { return .dropped(.ownData) }
            return optIn ? .unsourced : .dropped(.noMode)
        }
    }
}
