import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a „Antworten ohne Quelle“, E4.2b Quellblöcke): Prüfung VOR der Ausgabe, strukturell.
// Reine Funktionen, kein Modell, kein Zustand. Wortlisten und Heuristiken sind keine Sicherheitsgrenze mehr; die Grenze ist:
//
// - DATENRUNDE (Standard, mit und ohne Opt-in): Kein Satz des Modells wird je wörtlich ausgegeben. Jeder vollständige
//   Modellsatz „rastet ein“: Der beste QUELLBLOCK der Auszüge nach Wortdeckung (Deckung ≥ 60 % und mindestens zwei gedeckte
//   Inhaltswörter) wird GANZ und unverändert ausgegeben, Quelle = dieser Auszug, davor die Ansage „Wörtlicher Auszug aus
//   ‚Titel‘:“. Sonst entfällt der Satz. Ausgegeben wird also nur ein ganzer Quellblock, eine Vorlage der festen Liste oder ein
//   Satz des Systems (Uhr, Bestand, feste Sätze).
// - Quellblock = der gespeicherte Text zwischen strukturellen Grenzen der Quelle (`AssistantSections`): Absatz, Notizblock,
//   Segment. Innerhalb eines Blocks wird weder getrennt noch gekürzt, mehrere Blöcke werden nicht zusammengezogen, ein Block über
//   `maxBlockChars` erscheint nie. Strukturelle Grenzen garantieren keine Satzgrenzen: der Auszug kann ein Fragment sein oder
//   Zusammenhang auslassen; die Kennzeichnung „Wörtlicher Auszug“ deckt das. Satzzeichen sind kein Kriterium.
// - ALLGEMEINRUNDE (nur mit Opt-in, Prompt ohne Auszüge und ohne Bestand): Vorlage → neutral; Eigendatenbezug
//   (`AssistantOwnData`) → verworfen; Ähnlichkeit mit eigenem Inhalt (`resemblesOwnContent`, Abruf mit dem Satz als Anfrage)
//   → verworfen; sonst „ohne Quelle“. Nie grounded. Die Listen und die Ähnlichkeit können nur verbieten, nie erlauben.
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

    /// `hint`: Der Hinweis auf den Schalter „Antworten ohne Quelle“ nur, wenn es ihn gibt (Funktion freigeschaltet); in der
    /// Produktfassung (`AssistantUnsourcedConsent.isReleased == false`) verwiese er auf etwas, das es nicht gibt.
    public static func fixedSentence(allowUnsourced: Bool, hint: Bool = true) -> String {
        allowUnsourced ? nothingGroundedSentence : nothingFoundSentence + (hint ? " " + optInHint : "")
    }

    /// Längster Titel im Vorspann (länger: gekürzt mit „…“).
    static let maxPrefaceTitle = 80

    /// Feste Erklärung für die Anzeige (leerer Zustand und Hinweisbereich): Was in einer Antwort steht, ist wörtlich und kann ein
    /// Ausschnitt sein.
    public static let excerptNotice = "Antworten sind wörtliche Auszüge aus deinen Inhalten. Ein Auszug kann unvollständig sein oder Zusammenhang auslassen."

    /// Vorspann VOR JEDEM Block: „Wörtlicher Auszug aus ‚Der Zauberberg‘:“ (ein Wortlaut für alle Bereiche).
    public static func quotePreface(for passage: AssistantPassage) -> String {
        var title = passage.title.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        title = String(title.filter { !"‚‘’'“”„\"".contains($0) }).trimmingCharacters(in: .whitespaces)
        if title.isEmpty { title = passage.area.singularLabel }
        if title.count > maxPrefaceTitle { title = String(title.prefix(maxPrefaceTitle - 1)).trimmingCharacters(in: .whitespaces) + "…" }
        return "\(AssistantPassage.excerptPrefix)‚\(title)‘:"
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
            || normalize(sentence).hasPrefix(normalize(AssistantPassage.excerptPrefix))
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

    // MARK: Quellblöcke

    /// Ein ganzer Quellblock im Originalwortlaut, zeichengleich mit dem gespeicherten Block (Zeilenumbrüche bleiben).
    public struct SourceBlock: Sendable, Equatable {
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
        /// Quelle (Bereich und Element).
        public var sourceKey: String { "\(passage.area.rawValue)/\(passage.itemId)" }
    }

    /// Längster Block, der ausgegeben wird; ein längerer ist nicht zitierbar und wird nie gekürzt oder geteilt.
    public static let maxBlockChars = 600

    /// Die zitierbaren Quellblöcke aller Auszüge in Auszug-Reihenfolge: genau die Auszüge, die ein ganzer gespeicherter Block sind
    /// (`AssistantPassage.isBlock`, nicht gekürzt) und nicht über `maxBlockChars` liegen. Es gibt keine Nähte, keine Satzprüfung
    /// und keine Randflags: der Block gilt ganz oder gar nicht.
    public struct Basis: Sendable {
        let blocks: [SourceBlock]

        public init(passages: [AssistantPassage]) {
            blocks = passages.compactMap { p in
                AssistantGrounding.isQuotable(p) ? SourceBlock(text: p.text, passage: p) : nil
            }
        }

        /// Erkennungsbasis (nicht zum Zitieren): ALLE Sätze der Auszüge und die ganzen Texte. Dient nur dazu, einen Satz ohne
        /// Quelle zu verwerfen, der einem eigenen Inhalt ähnelt.
        init(detecting passages: [AssistantPassage]) {
            blocks = passages.flatMap { p in AssistantGrounding.detectionTexts(of: p.text).map { SourceBlock(text: $0, passage: p) } }
        }

        public var isEmpty: Bool { blocks.isEmpty }
        public var texts: [String] { blocks.map(\.text) }
    }

    /// Alle Sätze eines Textes (Zeilen, dann Satzgrenzen) mit zusammengezogenem Leerraum, plus der ganze Text: nur zum
    /// Verbieten (Erkennungsbasis), nie zum Zitieren.
    static func detectionTexts(of text: String) -> [String] {
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
        let whole = text.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        if !whole.isEmpty, !out.contains(whole) { out.append(whole) }
        return out
    }

    /// Zitierbar: ein ganzer gespeicherter Block, nicht leer, höchstens `maxBlockChars` lang. Satzzeichen, Länge in Wörtern und
    /// Anfangsbuchstabe spielen keine Rolle („Milch kaufen“ ist als Block zitierbar).
    static func isQuotable(_ p: AssistantPassage) -> Bool {
        p.isBlock && !p.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && p.text.count <= maxBlockChars
    }

    // MARK: Einrasten

    /// Wörter, die beim Einrasten nicht als Inhalt zählen: Verweise auf die Quelle und Funktionswörter. Sie beeinflussen nur
    /// die Auswahl, nie die Sicherheit (ausgegeben wird immer der ganze Quellblock).
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

    /// Der beste Quellblock zu einem Modellsatz: Deckung der Inhaltswörter des Modellsatzes ≥ `BookTalkCitations.minCoverage` (60 %)
    /// und mindestens zwei gedeckte Inhaltswörter. Gleichstand: mehr gedeckte Wörter, dann der kürzere Quellblock (Präzision),
    /// dann die frühere Stelle. `nil`, wenn keiner einrastet.
    public static func snap(_ sentence: String, basis: Basis) -> SourceBlock? {
        let content = Array(Set(BookTalkTokens.contentWords(sentence).filter { !ignoring.contains($0) }))
        guard content.count >= 2 else { return nil }
        var best: (SourceBlock, (share: Double, covered: Int, precision: Double))?
        for q in basis.blocks {
            let hits = covered(content, by: q.words)
            let share = Double(hits) / Double(content.count)
            guard hits >= 2, share >= BookTalkCitations.minCoverage else { continue }
            let score = (share: share, covered: hits, precision: Double(hits) / Double(max(q.content.count, 1)))
            if best == nil || better(score, than: best!.1) { best = (q, score) }
        }
        return best?.0
    }

    /// Rückfall, wenn nichts einrastet: der beste Quellblock gegen die FRAGE (mindestens zwei gedeckte Inhaltswörter oder
    /// mindestens 50 % der Inhaltswörter der Frage). Sätze aus `taken` (schon ausgegeben) zählen nicht.
    public static func fallback(question: String, basis: Basis, taken: Set<String> = []) -> SourceBlock? {
        let content = Array(Set(BookTalkTokens.contentWords(question).filter { !ignoring.contains($0) }))
        guard !content.isEmpty else { return nil }
        var best: (SourceBlock, (share: Double, covered: Int, precision: Double))?
        for q in basis.blocks where !taken.contains(q.key) {
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
    /// irgendeinen Satz der Auszüge (auch Randsätze), wird der Satz verworfen. Kann nur verbieten.
    public static func resemblesOwnContent(_ sentence: String, passages: [AssistantPassage]) -> Bool {
        guard !passages.isEmpty else { return false }
        return snap(sentence, basis: Basis(detecting: passages)) != nil
    }

    // MARK: Urteil über einen Modellsatz

    public enum Verdict: Sendable, Equatable {
        /// Datenrunde: der ganze Quellblock, der statt des Modellsatzes ausgegeben wird.
        case quote(SourceBlock)
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
