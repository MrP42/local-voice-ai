import Foundation

// Buch-Gespräch (Epic #307, Paket G4): Belege `[Q<k>]`, wörtliche Zitate, Sprechfassung (Plan §3.5).
// Das Modell nennt nur die Nummer; Seite, Offsets und Zitattext setzt der Code aus dem Buchtext.

public struct BookTalkMarker: Sendable, Equatable {
    public let ids: [Int]
    public let offset: Int
}

/// Entfernt `[Q<k>]` aus Modellausgabe, auch gestreamt in beliebigen Stücken, und faltet Leerraum.
/// Die Summe der `feed`-Rückgaben plus `finish()` ist der Endtext; `markers` hält die Stellen.
public struct BookTalkDeltaFilter: Sendable {
    private var pending: [Character] = []
    private var heldSpace = false
    private var started = false
    private var emitted = 0
    public private(set) var markers: [BookTalkMarker] = []

    public init() {}

    public static func extract(_ raw: String) -> (text: String, markers: [BookTalkMarker]) {
        var f = BookTalkDeltaFilter()
        var text = f.feed(raw)
        text += f.finish()
        return (text, f.markers)
    }

    public mutating func feed(_ s: String) -> String {
        var out = ""
        for ch in s { out += step(ch) }
        return out
    }

    public mutating func finish() -> String {
        let rest = String(pending)
        pending = []
        heldSpace = false
        return emit(rest)
    }

    private mutating func step(_ ch: Character) -> String {
        if pending.isEmpty {
            if ch == "[" {
                pending = ["["]
                return ""
            }
            return emit(String(ch))
        }
        if ch == "]" {
            let inner = String(pending.dropFirst())
            pending = []
            if let ids = Self.parseIds(inner) {
                markers.append(BookTalkMarker(ids: ids, offset: emitted))
                return ""
            }
            return emit("[" + inner + "]")
        }
        if pending.count >= 24 || !Self.isMarkerChar(ch) {
            let text = String(pending)
            pending = []
            return emit(text) + step(ch)
        }
        pending.append(ch)
        return ""
    }

    private mutating func emit(_ s: String) -> String {
        var out = ""
        for c in s {
            if c.isWhitespace {
                if started { heldSpace = true }
                continue
            }
            if heldSpace, !".,;:!?".contains(c) {
                out += " "
                emitted += 1
            }
            heldSpace = false
            out.append(c)
            emitted += String(c).utf16.count
            started = true
        }
        return out
    }

    private static func isMarkerChar(_ c: Character) -> Bool {
        c == "Q" || c == "q" || c == "," || c == ";" || c == " " || (c.isASCII && c.isNumber)
    }

    private static func parseIds(_ inner: String) -> [Int]? {
        let parts = inner.split(whereSeparator: { $0 == "," || $0 == ";" || $0 == " " })
        guard !parts.isEmpty else { return nil }
        var ids: [Int] = []
        for p in parts {
            guard p.count >= 2, p.count <= 4, p.first == "Q" || p.first == "q", let n = Int(p.dropFirst()) else { return nil }
            ids.append(n)
        }
        return ids
    }
}

public enum BookTalkSentenceKind: Sendable, Equatable {
    /// Buchsatz mit gültigem Beleg und Stützung.
    case cited
    /// „Allgemein:“-Satz (nur explain, höchstens einer): Modellwissen ohne Beleg.
    case general
    /// Feste Nicht-im-Buch-Vorlage.
    case absent
    /// Ohne gültigen oder gestützten Beleg oder mit Spätwort: wird verworfen.
    case dropped
}

/// Alles, was die Prüfung eines Antwortsatzes braucht (Plan §3.5, Runde 3).
struct BookTalkGroundingContext: Sendable {
    let index: BookTalkIndex
    let excerpts: [BookTalkExcerpt]
    let entities: [BookTalkEntity]
    let intent: BookTalkIntent
    let question: String
    /// Stämme von Inhaltswörtern, die im Buch erst hinter der Grenze erstmals vorkommen (nur Bereich „Bis hier“).
    let lateStems: Set<String>
    /// Stämme aller Inhaltswörter des ganzen Buchs.
    let bookStems: Set<String>
}

public enum BookTalkCitations {
    public static let maxQuoteChars = 240
    static let minCoverage = 0.6
    static let maxGeneralWords = 30

    // MARK: Wortlisten

    private static let negations: Set<String> = [
        "nicht", "nichts", "nie", "niemals", "ohne", "weder", "niemand", "nirgends",
        "not", "no", "never", "without", "neither", "nothing", "nobody", "none",
    ]

    private static let numberWords: [String: String] = [
        "null": "0", "zwei": "2", "drei": "3", "vier": "4", "funf": "5", "sechs": "6", "sieben": "7", "acht": "8",
        "neun": "9", "zehn": "10", "elf": "11", "zwolf": "12", "dreizehn": "13", "vierzehn": "14", "funfzehn": "15",
        "sechzehn": "16", "siebzehn": "17", "achtzehn": "18", "neunzehn": "19", "zwanzig": "20", "dreissig": "30",
        "vierzig": "40", "funfzig": "50", "sechzig": "60", "siebzig": "70", "achtzig": "80", "neunzig": "90",
        "hundert": "100", "tausend": "1000", "million": "1000000", "millionen": "1000000",
        "two": "2", "three": "3", "four": "4", "five": "5", "six": "6", "seven": "7", "eight": "8", "nine": "9",
        "ten": "10", "eleven": "11", "twelve": "12", "twenty": "20", "thirty": "30", "forty": "40", "fifty": "50",
        "hundred": "100", "thousand": "1000",
    ]

    private static let absentTemplates: Set<String> = Set([
        "Dazu steht bis hier nichts im Buch.", "Dazu steht bis hierhin nichts im Buch.", "Davon steht nichts im Buch.",
        "Dazu steht nichts im Buch.", "Dazu finde ich bis hier nichts.", "Dazu finde ich im Buch nichts.",
        "Bis zu deiner Leseposition steht dazu noch nichts im Buch.",
        "The book says nothing about that so far.", "The book says nothing about that.",
        "There is nothing about that in the book.", "I found nothing about that in the book.",
    ].map { BookTalkNormalize.words($0).joined(separator: " ") })

    // MARK: Satzprüfung

    static func isAbsentTemplate(_ sentence: String) -> Bool {
        absentTemplates.contains(BookTalkNormalize.words(sentence).joined(separator: " "))
    }

    static func hasGeneralPrefix(_ sentence: String) -> Bool {
        let l = sentence.lowercased()
        return l.hasPrefix("allgemein:") || l.hasPrefix("general:")
    }

    static func negationCount(_ words: [String]) -> Int {
        words.reduce(0) { $0 + (negations.contains($1) || ($1.hasPrefix("kein") && $1.count <= 7) ? 1 : 0) }
    }

    static func numbers(_ words: [String]) -> Set<String> {
        var out = Set<String>()
        for w in words {
            if !w.isEmpty, w.allSatisfy({ $0.isASCII && $0.isNumber }) {
                out.insert(String(w.drop(while: { $0 == "0" })).isEmpty ? "0" : String(w.drop(while: { $0 == "0" })))
            } else if let n = numberWords[w] {
                out.insert(n)
            }
        }
        return out
    }

    static func sharedPrefix(_ a: String, _ b: String) -> Int {
        var n = 0
        for (x, y) in zip(a, b) {
            if x != y { break }
            n += 1
        }
        return n
    }

    /// Gleiches Wort oder gemeinsamer Anfang ab 5 Zeichen („Freund“ ~ „Freundes“).
    static func covered(_ word: String, by window: [String]) -> Bool {
        window.contains { $0 == word || sharedPrefix($0, word) >= 5 }
    }

    /// Allgemeinsatz: nur bei explain, genau ein Satz je Antwort, ≤ 30 Wörter, keine Buchwörter außer aus der Frage,
    /// keine Namen. Sonst gilt er nicht als `general` (und wird wie ein Buchsatz geprüft).
    static func isAllowedGeneral(_ sentence: String, ctx: BookTalkGroundingContext) -> Bool {
        guard ctx.intent == .explain, hasGeneralPrefix(sentence) else { return false }
        let body = sentence.drop(while: { $0 != ":" }).dropFirst()
        guard sentence.split(whereSeparator: { $0.isWhitespace }).count <= maxGeneralWords else { return false }
        let seq = BookTalkText.foldedSequence(String(body))
        for e in ctx.entities where e.forms.contains(where: { BookTalkText.contains(seq, form: $0) }) { return false }
        let asked = BookTalkTokens.contentStems(ctx.question)
        for stem in BookTalkTokens.contentStems(String(body)) where ctx.bookStems.contains(stem) && !asked.contains(stem) {
            return false
        }
        return true
    }

    /// Prüft einen Satz (Belegtreue). `ids` sind die `[Q<k>]` im Satz, `generalFree`: Allgemeinplatz noch frei.
    static func evaluate(sentence: String, ids: [Int], ctx: BookTalkGroundingContext, generalFree: Bool)
        -> (kind: BookTalkSentenceKind, citations: [BookTalkCitation])
    {
        var result: (kind: BookTalkSentenceKind, citations: [BookTalkCitation]) = (.dropped, [])
        if isAbsentTemplate(sentence) {
            result = (.absent, [])
        } else if generalFree, isAllowedGeneral(sentence, ctx: ctx) {
            result = (.general, [])
        } else {
            let cites = ground(sentence: sentence, ids: ids, ctx: ctx)
            if !cites.isEmpty { result = (.cited, cites) }
        }
        if result.kind != .dropped, !ctx.lateStems.isEmpty,
           !BookTalkTokens.contentStems(sentence).isDisjoint(with: ctx.lateStems) {
            return (.dropped, [])
        }
        return result
    }

    /// Buchsatz: Stützung durch ein Belegsatzfenster (ein Satz ±1 Nachbarsatz im zitierten Stück) –
    /// Wortdeckung, Polarität, Namen, Zahlen. Es genügt ein Fenster; das Zitat ist dessen Mittelsatz.
    static func ground(sentence: String, ids: [Int], ctx: BookTalkGroundingContext) -> [BookTalkCitation] {
        var out: [BookTalkCitation] = []
        var seen = Set<[Int]>()
        for id in ids where id >= 1 && id <= ctx.excerpts.count {
            let chunk = ctx.excerpts[id - 1].chunk
            guard let q = supportedQuote(statement: sentence, chunk: chunk, ctx: ctx) else { continue }
            guard seen.insert([q.start, q.end]).inserted else { continue }
            out.append(BookTalkCitation(q: id, page: ctx.index.page(at: q.start), chapterIndex: chunk.chapterIndex,
                                        chapterTitle: chunk.chapterTitle, start: q.start, end: q.end, quote: q.text))
        }
        return out
    }

    static func supportedQuote(statement: String, chunk: BookTalkChunk, ctx: BookTalkGroundingContext) -> Quote? {
        let units = Array(chunk.text.utf16)
        let sentences = BookTalkSentences.split(chunk.text)
        guard !sentences.isEmpty else { return nil }
        let want = Set(BookTalkTokens.tokenize(statement))
        var best: (score: Double, i: Int)?
        for i in sentences.indices {
            let w0 = sentences[max(i - 1, 0)].start, w1 = sentences[min(i + 1, sentences.count - 1)].end
            let window = String(decoding: units[w0..<w1], as: UTF16.self)
            guard supported(statement, window: window, ctx: ctx) else { continue }
            let got = Set(BookTalkTokens.tokenize(String(decoding: units[sentences[i].start..<sentences[i].end], as: UTF16.self)))
            let sc = want.isEmpty || got.isEmpty ? 0 : Double(want.intersection(got).count) / (Double(want.count) * Double(got.count)).squareRoot()
            if best == nil || sc > best!.score { best = (sc, i) }
        }
        guard let b = best else { return nil }
        return quote(chunk: chunk, units: units, from: sentences[b.i].start, to: sentences[b.i].end)
    }

    /// Verneinung: Parität der Negationen im Antwortsatz gegen die Negationen der best passenden Teilsatz-Einheit
    /// des Fensters (Komma, Semikolon, Doppelpunkt, Satz- und Anführungszeichen trennen). So zählt eine Verneinung
    /// in einem anderen Teilsatz desselben Fensters nicht gegen eine passende Aussage.
    static func clauseNegations(window: String, content: [String]) -> Int {
        var best = (hits: -1, negations: 0)
        for clause in window.split(whereSeparator: { ",;:.!?„“”\"".contains($0) }) {
            let w = BookTalkTokens.foldedWords(String(clause))
            let hits = content.filter { covered($0, by: w) }.count
            if hits > best.hits { best = (hits, negationCount(w)) }
        }
        return best.negations
    }

    static func supported(_ sentence: String, window: String, ctx: BookTalkGroundingContext) -> Bool {
        let answerWords = BookTalkTokens.foldedWords(sentence)
        let windowWords = BookTalkTokens.foldedWords(window)
        let content = BookTalkTokens.contentWords(sentence)
        guard !content.isEmpty else { return false }
        let hits = content.filter { covered($0, by: windowWords) }.count
        guard Double(hits) / Double(content.count) >= minCoverage else { return false }
        guard negationCount(answerWords) % 2 == clauseNegations(window: window, content: content) % 2 else { return false }
        for e in ctx.entities where e.kind != .term {
            let inAnswer = e.forms.contains { BookTalkText.contains(answerWords, form: $0) }
            if inAnswer, !e.forms.contains(where: { BookTalkText.contains(windowWords, form: $0) }) { return false }
        }
        return numbers(answerWords).isSubset(of: numbers(windowWords))
    }

    static func quote(chunk: BookTalkChunk, units: [UInt16], from: Int, to: Int) -> Quote? {
        var to = to
        if to - from > maxQuoteChars {
            var e = from + maxQuoteChars
            while e > from + maxQuoteChars / 2, units[e - 1] != 32 { e -= 1 }
            while e > from, units[e - 1] == 32 || units[e - 1] == 44 { e -= 1 }
            to = e
        }
        guard to > from else { return nil }
        let map = chunk.offsetMap
        return Quote(text: String(decoding: units[from..<to], as: UTF16.self), start: map[from], end: map[to - 1] + 1, window: "")
    }

    // MARK: Zitat

    struct Quote {
        let text: String
        let start: Int
        let end: Int
        /// Belegsatz plus je ein Nachbarsatz.
        let window: String
    }

    /// Ähnlichster Satz (oder zwei benachbarte, ≤ 240 Zeichen) zur Aussage; Offsets im Original.
    static func bestQuote(statement: String, chunk: BookTalkChunk) -> Quote? {
        let units = Array(chunk.text.utf16)
        let sentences = BookTalkSentences.split(chunk.text)
        guard !sentences.isEmpty else { return nil }
        let want = Set(BookTalkTokens.tokenize(statement))
        let sets = sentences.map { s in Set(BookTalkTokens.tokenize(String(decoding: units[s.start..<s.end], as: UTF16.self))) }

        func sim(_ got: Set<String>) -> Double {
            guard !want.isEmpty, !got.isEmpty else { return 0 }
            return Double(want.intersection(got).count) / (Double(want.count) * Double(got.count)).squareRoot()
        }
        var bestIdx = 0, bestScore = -1.0
        for i in sentences.indices {
            let sc = sim(sets[i])
            if sc > bestScore { bestScore = sc; bestIdx = i }
        }
        var lo = bestIdx, hi = bestIdx
        for i in 0..<(sentences.count - 1) {
            let a = sentences[i], b = sentences[i + 1]
            guard b.end - a.start <= maxQuoteChars else { continue }
            let sc = sim(sets[i].union(sets[i + 1]))
            if sc > bestScore + 0.1 { bestScore = sc; lo = i; hi = i + 1 }
        }
        let w0 = sentences[max(lo - 1, 0)].start, w1 = sentences[min(hi + 1, sentences.count - 1)].end
        guard let q = quote(chunk: chunk, units: units, from: sentences[lo].start, to: sentences[hi].end) else { return nil }
        return Quote(text: q.text, start: q.start, end: q.end, window: String(decoding: units[w0..<w1], as: UTF16.self))
    }

    // MARK: Spätwörter

    /// Stämme von Inhaltswörtern, die erst hinter der Grenze erstmals vorkommen.
    static func lateStems(index: BookTalkIndex, frontier: Int) -> Set<String> {
        var before = Set<String>()
        var all = Set<String>()
        for c in index.chunks {
            all.formUnion(BookTalkTokens.contentStems(c.text))
            if c.start < frontier, let t = c.truncated(atFrontier: frontier) { before.formUnion(BookTalkTokens.contentStems(t.text)) }
        }
        return all.subtracting(before)
    }

    static func bookStems(index: BookTalkIndex) -> Set<String> {
        index.chunks.reduce(into: Set<String>()) { $0.formUnion(BookTalkTokens.contentStems($1.text)) }
    }
}

// MARK: - Sprechfassung

public enum BookTalkSpeech {
    public static let maxQuoteWords = 25

    private static let abbreviationsDe: [(String, String)] = [
        ("z. B.", "zum Beispiel"), ("z.B.", "zum Beispiel"), ("d. h.", "das heißt"), ("d.h.", "das heißt"),
        ("u. a.", "unter anderem"), ("u.a.", "unter anderem"), ("usw.", "und so weiter"), ("bzw.", "beziehungsweise"),
        ("ca.", "circa"), ("vgl.", "vergleiche"), ("ggf.", "gegebenenfalls"), ("evtl.", "eventuell"), ("Nr.", "Nummer"),
        ("S.", "Seite"), ("Dr.", "Doktor"), ("etc.", "und so weiter"),
    ]
    private static let abbreviationsEn: [(String, String)] = [
        ("e.g.", "for example"), ("i.e.", "that is"), ("etc.", "and so on"), ("vs.", "versus"), ("approx.", "approximately"),
        ("No.", "number"), ("Dr.", "Doctor"), ("p.", "page"),
    ]

    /// Was nach den Satz-Sprechfassungen noch gesprochen wird: Rückfrage (locate, mit Beleg) oder „bis Seite N“ (recap).
    /// Der eine gesprochene Beleg hängt schon an der Sprechfassung des ersten belegten Satzes.
    public static func tail(intent: BookTalkIntent, hasCitation: Bool, language: BookTalkLanguage, readingPage: Int?) -> String {
        let de = language == .de
        switch intent {
        case .recap:
            guard let p = readingPage else { return "" }
            return de ? "Das war der Stand bis Seite \(p)." : "That was the story up to page \(p)."
        case .locate:
            guard hasCitation else { return "" }
            return de ? "Soll ich dir die Stelle vorlesen?" : "Shall I read the passage to you?"
        default:
            return ""
        }
    }

    /// „Das steht auf Seite 14: ‚…‘“
    public static func referenceSentence(_ c: BookTalkCitation, language: BookTalkLanguage) -> String {
        let de = language == .de
        let q = spokenQuote(c.quote)
        let place: String
        if let p = c.page {
            place = de ? "Das steht auf Seite \(p):" : "That is on page \(p):"
        } else {
            place = de ? "Das steht im Buch:" : "That is in the book:"
        }
        return "\(place) ‚\(q)‘"
    }

    /// Höchstens 25 Wörter, sonst mit „…“ gekürzt; Anführungszeichen im Zitat raus (die Stimme setzt eigene).
    static func spokenQuote(_ quote: String) -> String {
        let cleaned = String(quote.filter { !"„“”\"«»".contains($0) })
        let words = cleaned.split(whereSeparator: { $0.isWhitespace })
        guard words.count > maxQuoteWords else { return words.joined(separator: " ") }
        var cut = words.prefix(maxQuoteWords).joined(separator: " ")
        while let l = cut.last, ",;:".contains(l) { cut.removeLast() }
        return cut + "…"
    }

    /// Text ohne Markdown und mit ausgeschriebenen Abkürzungen.
    static func plain(_ text: String, language: BookTalkLanguage) -> String {
        var s = String(text.filter { !"*_#`".contains($0) })
        for (a, b) in language == .de ? abbreviationsDe : abbreviationsEn {
            s = s.replacingOccurrences(of: a, with: b)
        }
        return s.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }
}
