import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a „Antworten ohne Quelle“): Satzprüfung VOR der Ausgabe.
// Reine Funktionen, kein Modell, kein Zustand. Die Stützung nutzt dieselben Bausteine wie das Buch-Gespräch
// (`BookTalkCitations.supported`: 60 % Wortdeckung, Polarität, Zahlen); dazu kommt die Namensprüfung (großgeschriebene
// Wörter, die im Beleg fehlen). Die Prüfung ist die Durchsetzung; der Prompt bittet das Modell nur darum.
//
// Wertung eines Satzes:
// - `grounded`: durch einen Auszug (oder die Bestandsliste) gedeckt;
// - `appClaimUnsupported`: nimmt Bezug auf App-Inhalte (Buch, Kapitel, Besprechung, Titel aus dem Bestand, „laut …“,
//   „du hast gesagt …“) oder gibt einen Auszug verfälscht wieder (Zahl, Verneinung, Name), ohne gedeckt zu sein;
// - `unsourced`: sonstiges Allgemeinwissen, auch Empfehlungen;
// - `neutral`: Rückfragen, Grüße, feste Vorlagen, Angebote, Angaben zum eigenen Können, die Uhrzeit.

public enum AssistantGroundingVerdict: Sendable, Equatable {
    case grounded([AssistantPassage])
    case unsourced
    case appClaimUnsupported
    case neutral
}

public enum AssistantGrounding {
    // MARK: Feste Sätze

    public static let unsourcedAnnouncement = "Ohne Quelle:"
    public static let groundedAnnouncement = "Aus deinen Inhalten:"
    /// Ohne Opt-in, wenn von einer Antwort nichts Gesprochenes übrig bleibt.
    public static let nothingFoundSentence = "Dazu finde ich in deinen Inhalten nichts."
    public static let optInHint = "Antworten ohne Quelle kannst du im Menü einschalten."
    /// Mit Opt-in, wenn nichts Belegtes (und nichts Unbelegtes) bleibt.
    public static let nothingGroundedSentence = "Dazu finde ich nichts Belegtes."

    public static func fixedSentence(allowUnsourced: Bool) -> String {
        allowUnsourced ? nothingGroundedSentence : nothingFoundSentence + " " + optInHint
    }

    // MARK: Grundlage

    /// Fenster, gegen die ein Satz geprüft wird: je Auszug jeder Satz mit seinen Nachbarsätzen, der Bestand und die
    /// Uhrzeit als ganzer Text. Einmal je Anfrage bauen.
    public struct Basis: Sendable {
        enum Kind: Sendable { case passage, inventory, clock }
        struct Window: Sendable {
            let text: String
            let passage: AssistantPassage?
            let kind: Kind
        }

        let windows: [Window]
        /// Gefaltete Wortfolgen der Titel (Bestand und Auszüge), ab 5 Zeichen; Titel mit Artikel auch ohne ihn.
        let titles: [[String]]
        /// Stunde und Minute der Uhrzeit („08:05“), für Sätze wie „Es ist 08:05 Uhr.“
        let clockTime: (hour: Int, minute: Int)?
        /// Gefaltete Wörter der Kopfzeilen des Bestands („Bücher: 3 insgesamt“), für Zählangaben.
        let inventoryHeaders: [[String]]

        public init(passages: [AssistantPassage], inventoryTitles: [String] = [], inventory: String = "", clock: String = "") {
            var windows: [Window] = []
            for p in passages {
                let text = p.text.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { continue }
                // Der Titel gehört zum Fenster: Ein Satz darf die Quelle nennen („im Zauberberg …“, „in der Besprechung vom …“).
                let title = " " + p.title
                let units = Array(text.utf16)
                let sentences = BookTalkSentences.split(text)
                guard !sentences.isEmpty else {
                    windows.append(Window(text: text + title, passage: p, kind: .passage))
                    continue
                }
                for i in sentences.indices {
                    let from = sentences[max(i - 1, 0)].start, to = sentences[min(i + 1, sentences.count - 1)].end
                    windows.append(Window(text: String(decoding: units[from..<to], as: UTF16.self) + title, passage: p, kind: .passage))
                }
            }
            // Bestand: je Bereich ein Fenster (Kopfzeile mit den Titeln darunter). Ein Bereich gehört nicht in das Fenster
            // eines anderen: „drei Transkripte“ wird nicht durch „3 Bücher“ gedeckt. Zählangaben über mehrere Bereiche
            // prüft `statesCount` an den Kopfzeilen.
            var blocks: [[String]] = []
            for line in inventory.split(whereSeparator: { $0.isNewline }).map({ $0.trimmingCharacters(in: .whitespaces) }) where !line.isEmpty {
                if line.hasPrefix("-"), !blocks.isEmpty { blocks[blocks.count - 1].append(line) } else { blocks.append([line]) }
            }
            for block in blocks { windows.append(Window(text: block.joined(separator: "\n"), passage: nil, kind: .inventory)) }
            self.inventoryHeaders = blocks.compactMap(\.first).map { BookTalkTokens.foldedWords($0) }
            let now = clock.trimmingCharacters(in: .whitespacesAndNewlines)
            if !now.isEmpty { windows.append(Window(text: now, passage: nil, kind: .clock)) }
            self.windows = windows
            self.clockTime = Self.time(in: now)
            var seen = Set<[String]>()
            var titleForms: [[String]] = []
            for title in inventoryTitles + passages.map(\.title) {
                var words = BookTalkTokens.foldedWords(title)
                titleForms.append(words)
                if let first = words.first, Self.articles.contains(first) { words.removeFirst(); titleForms.append(words) }
            }
            self.titles = titleForms.filter { $0.joined(separator: " ").count >= 5 && seen.insert($0).inserted }
        }

        private static let articles: Set<String> = ["der", "die", "das", "ein", "eine", "the", "a", "an"]

        /// „…, 08:05 Uhr“ → (8, 5).
        static func time(in text: String) -> (hour: Int, minute: Int)? {
            for token in text.split(whereSeparator: { $0.isWhitespace }) {
                let parts = token.split(separator: ":", omittingEmptySubsequences: false)
                guard parts.count == 2, let h = Int(parts[0]), let m = Int(parts[1].prefix(2)), (0..<24).contains(h), (0..<60).contains(m) else { continue }
                return (h, m)
            }
            return nil
        }
    }

    public static func classify(sentence: String, passages: [AssistantPassage], inventoryTitles: [String],
                                inventory: String = "", clock: String = "") -> AssistantGroundingVerdict {
        classify(sentence: sentence, basis: Basis(passages: passages, inventoryTitles: inventoryTitles, inventory: inventory, clock: clock))
    }

    public static func classify(sentence: String, basis: Basis) -> AssistantGroundingVerdict {
        let text = sentence.trimmingCharacters(in: .whitespacesAndNewlines)
        let words = BookTalkTokens.foldedWords(text)
        guard !words.isEmpty else { return .neutral }
        if isAnnouncement(text) || isTemplate(text) { return .neutral }
        let numbered = !BookTalkCitations.numbers(words).isEmpty
        if isFiller(words, numbered: numbered) || isQuestion(text, words, numbered: numbered)
            || isOffer(words, numbered: numbered) || isOwnAbility(words, numbered: numbered) { return .neutral }

        var covering: [AssistantPassage] = []
        var inventoryHit = false, clockHit = false, near = false
        for w in basis.windows {
            let ignoring = w.kind == .passage ? passageIgnoring : functionWords
            if supported(text, window: w.text, ignoring: ignoring) {
                switch w.kind {
                case .passage:
                    if let p = w.passage, !covering.contains(where: { sameSource($0, p) }) { covering.append(p) }
                case .inventory: inventoryHit = true
                case .clock: clockHit = true
                }
            } else if BookTalkCitations.coverage(text, window: w.text, ignoring: ignoring) >= BookTalkCitations.minCoverage {
                near = true
            }
        }
        if !covering.isEmpty { return .grounded(covering) }
        if inventoryHit { return .grounded([]) }
        if clockHit || statesClockTime(words, basis: basis) { return .neutral }
        if statesCount(words, basis: basis) { return .grounded([]) }
        if near || refersToApp(words, basis: basis) { return .appClaimUnsupported }
        return .unsourced
    }

    /// „Es ist 08:05 Uhr.“: keine Inhaltswörter, aber „Uhr“ und genau Stunde und Minute der Uhrzeit hintereinander.
    private static func statesClockTime(_ words: [String], basis: Basis) -> Bool {
        guard let t = basis.clockTime, words.contains("uhr") else { return false }
        let numbers = words.map { Int($0) }
        return numbers.indices.dropLast().contains { numbers[$0] == t.hour && numbers[$0 + 1] == t.minute }
    }

    /// Zählangaben („Du hast ein Buch.“, „Du hast drei Bücher und ein Transkript.“): Außer Hauptwörtern, Zahlen und
    /// Funktionswörtern kommt nichts vor. Jede Zahl steht vor ihrem Hauptwort („ein“ zählt als 1) und muss in der Kopfzeile
    /// genau dieses Bereichs stehen.
    private static func statesCount(_ words: [String], basis: Basis) -> Bool {
        let rest = BookTalkTokens.contentWords(words.joined(separator: " ")).filter { !functionWords.contains($0) && !appNouns.contains($0) }
        guard rest.isEmpty else { return false }
        var pending: String?
        var counted = false
        for w in words {
            if w == "ein" || w == "eine" { pending = "1"; continue }
            if let n = BookTalkCitations.numbers([w]).first { pending = n; counted = true; continue }
            guard appNouns.contains(w) else { continue }
            let lines = basis.inventoryHeaders.filter { $0.contains { $0.hasPrefix(w) || ($0.count >= 4 && w.hasPrefix($0)) } }
            guard !lines.isEmpty else { return false }
            if let n = pending {
                guard lines.contains(where: { BookTalkCitations.numbers($0).contains(n) }) else { return false }
                counted = true
            }
            pending = nil
        }
        return counted && pending == nil
    }

    private static func sameSource(_ a: AssistantPassage, _ b: AssistantPassage) -> Bool {
        a.area == b.area && a.itemId == b.itemId && a.locator == b.locator
    }

    // MARK: Stützung

    /// Wortdeckung, Polarität und Zahlen wie im Buch-Gespräch, dazu: Jedes großgeschriebene Wort (Name, Ort, Hauptwort), das
    /// nicht im Beleg steht, verwirft den Satz. So fällt „Castorp fährt nach Venedig“ gegen „… nach Davos“ durch, obwohl die
    /// übrigen Wörter passen.
    private static func supported(_ sentence: String, window: String, ignoring: Set<String>) -> Bool {
        guard BookTalkCitations.supported(sentence, window: window, entities: [], ignoring: ignoring) else { return false }
        let windowWords = BookTalkTokens.foldedWords(window)
        return capitalizedWords(sentence).allSatisfy { w in
            ignoring.contains(w) || BookTalkCitations.covered(w, by: windowWords)
        }
    }

    /// Gefaltete großgeschriebene Wörter ab 4 Zeichen, ohne Stoppwörter und typische Satzanfänge („Danach“, „Heute“).
    static func capitalizedWords(_ sentence: String) -> [String] {
        var tokens: [String] = []
        var current = ""
        for ch in sentence {
            if ch.isLetter || ch.isNumber { current.append(ch); continue }
            if !current.isEmpty { tokens.append(current); current = "" }
        }
        if !current.isEmpty { tokens.append(current) }
        return tokens.compactMap { token in
            guard let first = token.first, first.isLetter, first.isUppercase else { return nil }
            let folded = BookTalkTokens.foldedWords(token).joined()
            guard folded.count >= 4, !BookTalkTokens.stop.contains(folded), !starters.contains(folded) else { return nil }
            return folded
        }
    }

    // MARK: Neutrale Sätze

    static func isAnnouncement(_ sentence: String) -> Bool {
        let flat = BookTalkNormalize.words(sentence).joined(separator: " ")
        return flat == BookTalkNormalize.words(unsourcedAnnouncement).joined(separator: " ")
            || flat == BookTalkNormalize.words(groundedAnnouncement).joined(separator: " ")
    }

    private static let templates: Set<String> = Set([nothingFoundSentence, optInHint, nothingGroundedSentence,
                                                     "Dazu finde ich in deinen Inhalten leider nichts."]
        .map { BookTalkNormalize.words($0).joined(separator: " ") })

    /// Feste Vorlage „nichts gefunden“ (eigene Sätze und die des Buch-Gesprächs): zählt als Aussage der Antwort.
    public static func isTemplate(_ sentence: String) -> Bool {
        templates.contains(BookTalkNormalize.words(sentence).joined(separator: " ")) || BookTalkCitations.isAbsentTemplate(sentence)
    }

    private static let fillers: Set<String> = [
        "hallo", "hi", "hey", "moin", "servus", "tschuss", "danke", "dankeschon", "bitte", "gerne", "gern", "klar", "okay", "ok",
        "genau", "naturlich", "selbstverstandlich", "verstanden", "gut", "prima", "super", "problem", "geschehen", "bescheid",
        "ja", "nein", "aha", "hm", "hmm", "tja", "na", "nun", "oh", "sorry", "entschuldigung", "moment", "augenblick", "guten",
        "morgen", "tag", "abend", "nacht", "richtig", "stimmt", "alles", "schon", "gleich", "sofort", "leider", "schade", "freut",
        "mich", "dich", "kein", "helfen", "hore", "hier", "bin", "da", "gruss", "wiedersehen",
    ]

    /// Nur Füllwörter und Stoppwörter („Okay.“, „Gerne, kein Problem.“, „Ich bin da.“) und keine Zahl: keine Aussage.
    private static func isFiller(_ words: [String], numbered: Bool) -> Bool {
        !numbered && words.allSatisfy { BookTalkTokens.stop.contains($0) || fillers.contains($0) }
    }

    /// Kurze Rückfrage („Soll ich dir die Stelle vorlesen?“). Lange Fragen, Fragen mit Zahl und Fragen mit „dass“-Satz
    /// prüfen wie Aussagen, damit sich eine Behauptung nicht als Frage tarnt („Wusstest du, dass …?“).
    private static func isQuestion(_ text: String, _ words: [String], numbered: Bool) -> Bool {
        (text.hasSuffix("?") || text.hasSuffix("？")) && words.count <= 16 && !numbered && !words.contains("dass")
    }

    private static let offerStarts = [
        "sag bescheid", "sag mir bescheid", "sag einfach", "frag mich", "frag gerne", "frag gern", "wenn du magst", "wenn du willst",
        "wenn du mochtest", "falls du magst", "falls du mochtest", "falls du willst", "wenn du mehr", "falls du mehr", "ich kann dir gern",
        "ich kann dir gerne", "ich kann dir mehr", "ich erzahl dir gern", "ich erzahle dir gern", "gern erzahle ich", "gerne erzahle ich",
    ]

    /// Angebote zur Vertiefung („Wenn du magst, erzähle ich mehr.“): behaupten nichts. Mit Zahl nicht.
    private static func isOffer(_ words: [String], numbered: Bool) -> Bool {
        guard !numbered else { return false }
        let flat = words.joined(separator: " ")
        return offerStarts.contains { flat == $0 || flat.hasPrefix($0 + " ") }
    }

    private static let negators: Set<String> = ["nicht", "nichts", "kein", "keine", "keinen", "keinem", "keiner"]
    private static let humility = ["ich weiss nicht", "das weiss ich nicht", "ich bin nicht sicher", "ich bin mir nicht sicher",
                                   "ich bin unsicher", "da bin ich unsicher", "das weiss ich leider nicht"]

    /// Angaben zum eigenen Können („Ich kann keine Mails senden.“, „Das weiß ich nicht.“): sagen nichts über die Welt.
    /// Kurz, ohne Zahl, und die Verneinung steht gleich hinter „kann“.
    private static func isOwnAbility(_ words: [String], numbered: Bool) -> Bool {
        guard !numbered, words.count <= 12 else { return false }
        let flat = " " + words.joined(separator: " ") + " "
        if humility.contains(where: { flat.contains(" \($0) ") }) { return true }
        guard let i = words.firstIndex(of: "kann") else { return false }
        let byMe = (i > 0 && words[i - 1] == "ich") || (i + 1 < words.count && words[i + 1] == "ich")
        return byMe && words[(i + 1)..<min(i + 5, words.count)].contains { negators.contains($0) }
    }

    // MARK: Bezug auf App-Inhalte

    /// Gefaltete Wörter, die auf Inhalte der App zeigen. Bewusst weit: Ein Satz darüber, der nicht gedeckt ist, fällt weg.
    static let appNouns: Set<String> = [
        "buch", "buches", "bucher", "buchern", "buchs", "horbuch", "horbucher", "roman", "romane", "kapitel", "kapiteln",
        "besprechung", "besprechungen", "meeting", "meetings", "protokoll", "protokolle", "protokolls", "transkript",
        "transkripte", "transkripten", "transkripts", "mitschrift", "mitschriften", "notiz", "notizen", "sprachnotiz",
        "sprachnotizen", "ablage", "dokument", "dokumente", "dokumenten", "dokuments", "datei", "dateien", "aufnahme",
        "aufnahmen", "bibliothek", "inhalte", "inhalten",
    ]

    /// Mail und Kalender (ADR-0028 E4.2a Nr. 3): Bezug auf eigene Daten nur zusammen mit „du“, „dein …“ oder „euer …“
    /// („Du hast morgen einen Termin.“). Ohne diesen Bezug bleibt es Allgemeinwissen („Ein Termin beim Notar dauert …“).
    static let ownDataNouns: Set<String> = [
        "mail", "mails", "email", "emails", "postfach", "posteingang", "kalender", "kalenders", "termin", "termine", "terminen",
        "termins",
    ]

    /// Verben, mit denen der Assistent dem Nutzer eigene Äußerungen oder Handlungen zuschreibt („du hast gesagt …“).
    private static let userActs: Set<String> = [
        "gesagt", "notiert", "erwahnt", "aufgenommen", "gelesen", "geschrieben", "besprochen", "gespeichert", "diktiert",
        "hochgeladen", "importiert", "angelegt", "gefragt", "gesprochen", "erzahlt", "sagtest", "erwahntest", "notiertest",
        "schriebst", "fragtest", "markiert", "gemerkt", "vorgemerkt",
    ]

    private static let sourceArticles: Set<String> = [
        "dem", "der", "den", "des", "deinem", "deiner", "deinen", "deine", "dein", "meinem", "meiner", "meinen", "einem", "einer",
        "diesem", "dieser", "ihrem", "ihrer",
    ]

    /// Wörter, die nicht als eigene Inhalte zählen, wenn sie fehlen: Funktionswörter, die `contentWords` nicht kennt.
    static let passageIgnoring = functionWords.union(appNouns)

    /// Funktionswörter und ausgeschriebene Zahlwörter (Zahlen prüft `BookTalkCitations.numbers` eigens: „drei“ ~ „3“).
    static let functionWords: Set<String> = baseFunctionWords.union(BookTalkCitations.numberWords.keys)

    private static let baseFunctionWords: Set<String> = [
        "hast", "habe", "habt", "bist", "seid", "kannst", "wirst", "musst", "sollst", "willst", "gerade", "leider", "gerne", "gern",
        "genau", "wirklich", "ubrigens", "ausserdem", "zudem", "deine", "deinen", "deinem", "deiner", "deines", "eure", "euer",
        "laut", "gemass", "gesagt", "notiert", "erwahnt", "geschrieben", "gelesen", "aufgenommen", "besprochen", "gespeichert",
    ]

    /// Sätze, die typisch groß beginnen, ohne einen Namen zu meinen („Danach …“, „Heute …“).
    private static let starters: Set<String> = [
        "danach", "anschliessend", "ausserdem", "zudem", "leider", "ubrigens", "naturlich", "ebenfalls", "allerdings", "jedoch",
        "deshalb", "daher", "dennoch", "trotzdem", "vielleicht", "wahrscheinlich", "vermutlich", "offenbar", "eigentlich",
        "zunachst", "schliesslich", "spater", "gestern", "heute", "morgen", "bisher", "jetzt", "zuerst", "laut", "insgesamt",
        "genau", "gerne", "aktuell", "derzeit", "momentan", "grundsatzlich", "generell", "meist", "oft", "manchmal",
        "beispielsweise", "hierzu", "hierbei", "damals", "anfangs", "zuletzt", "zusammengefasst", "erstens", "zweitens",
        "drittens", "ebenso", "gleichzeitig", "inzwischen", "mittlerweile", "immerhin", "tatsachlich", "wirklich", "sicher",
        "sicherlich", "bestimmt", "konkret", "kurzum", "gemass", "dabei", "dafur", "daruber", "darin", "davon", "dazu",
    ]

    private static func refersToApp(_ words: [String], basis: Basis) -> Bool {
        if words.contains(where: { appNouns.contains($0) }) { return true }
        if words.contains("du"), words.contains(where: { userActs.contains($0) }) { return true }
        if words.contains(where: { ownDataNouns.contains($0) }),
           words.contains(where: { $0 == "du" || $0 == "dir" || $0 == "dich" || $0.hasPrefix("dein") || $0 == "euer" || $0.hasPrefix("eure") }) {
            return true
        }
        if words.contains("gemass") { return true }
        if let i = words.firstIndex(of: "laut"), i + 1 < words.count {
            let next = words[i + 1]
            if i == 0 ? !["ist", "war", "wird", "sind", "waren"].contains(next) : sourceArticles.contains(next) { return true }
        }
        if words.contains(where: { $0 == "seite" || $0 == "seiten" }), !BookTalkCitations.numbers(words).isEmpty { return true }
        return basis.titles.contains { contains(words, sequence: $0) }
    }

    private static func contains(_ words: [String], sequence: [String]) -> Bool {
        guard !sequence.isEmpty, words.count >= sequence.count else { return false }
        for i in 0...(words.count - sequence.count) where Array(words[i..<(i + sequence.count)]) == sequence { return true }
        return false
    }
}
