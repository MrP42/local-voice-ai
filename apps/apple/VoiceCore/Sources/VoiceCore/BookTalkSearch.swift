import Foundation

// Buch-Gespräch (Epic #307, Paket G4): Tokenisierung, BM25, Retrieval, Budget.

public enum BookTalkTokens {
    static let stop: Set<String> = [
        // de
        "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "eines", "und", "oder", "aber",
        "ist", "sind", "war", "waren", "wird", "werden", "wurde", "hat", "haben", "hatte", "nicht", "kein", "keine",
        "ich", "du", "er", "sie", "es", "wir", "ihr", "mir", "mich", "dir", "dich", "mein", "dein", "sein", "von", "mit",
        "bei", "aus", "auf", "an", "in", "im", "zu", "zum", "zur", "fur", "uber", "um", "als", "wie", "was", "wer", "wo",
        "wann", "warum", "wieso", "welche", "welcher", "welches", "wird", "dass", "da", "dann", "auch", "noch", "nur",
        "so", "sich", "man", "gibt", "diesem", "dieser", "diese", "dieses", "bitte", "mal", "doch", "jetzt", "bisher",
        "erklar", "erklart", "gemeint", "steht", "stelle", "buch",
        "seine", "seinen", "seines", "seiner", "seinem", "ihre", "ihren", "ihrem", "ihrer", "ihres", "meine", "meinen",
        "meines", "meiner", "unser", "wenn", "weil", "denn", "sehr", "dazu", "davon", "dabei", "damit", "darauf", "dafur",
        "dies", "immer", "werden", "wurde", "wurden", "sind", "hatten", "kann", "konnte", "muss", "musste", "soll",
        "sollte", "will", "wollte", "gab", "mehr", "viel", "viele", "alle", "alles", "jeden", "jede", "jedes", "hier",
        "dort", "nach", "vor", "durch", "ohne", "sagt", "sagte", "wurde", "etwa", "also", "oder", "ganz", "eigene",
        "eigenen", "eigener", "andere", "anderen", "anderer",
        // en
        "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "has", "have", "had", "not", "no",
        "i", "you", "he", "she", "it", "we", "they", "me", "my", "your", "his", "her", "its", "of", "with", "at", "from",
        "on", "in", "to", "for", "by", "as", "how", "what", "who", "where", "when", "why", "which", "that", "this", "these",
        "those", "there", "then", "also", "only", "so", "does", "do", "did", "about", "explain", "please", "book",
    ]

    /// Einfache Tokenisierung: klein, Umlaute gefaltet, Buchstaben/Ziffern, leichte Endungskürzung, ohne Stoppwörter.
    public static func tokenize(_ text: String) -> [String] {
        foldedWords(text).compactMap { w in
            guard w.count >= 2, !stop.contains(w) else { return nil }
            let s = stem(w)
            return s.count >= 2 ? s : nil
        }
    }

    /// Wie `BookTalkNormalize.words`, aber ä→a, ö→o, ü→u (Väter ↔ Vater).
    static func foldedWords(_ s: String) -> [String] {
        var out = ""
        for ch in s.lowercased() {
            switch ch {
            case "ä": out += "a"
            case "ö": out += "o"
            case "ü": out += "u"
            case "ß": out += "ss"
            default: out += (ch.isLetter || ch.isNumber) ? String(ch) : " "
            }
        }
        return out.split(separator: " ").map(String.init)
    }

    /// Inhaltswörter für die Belegprüfung: gefaltet, ab 4 Zeichen, nicht in der Stoppwortliste, ohne Stammkürzung.
    static func contentWords(_ s: String) -> [String] {
        foldedWords(s).filter { $0.count >= 4 && !stop.contains($0) }
    }

    static func contentStems(_ s: String) -> Set<String> {
        Set(contentWords(s).map(stem))
    }

    private static let suffixes = ["ern", "en", "er", "es", "e", "n", "s", "ing", "ed"]

    /// Zwei Durchgänge: „Lehrers“ → „Lehrer“ → „Lehr“.
    static func stem(_ w: String) -> String {
        var w = w
        for _ in 0..<2 {
            guard w.count > 5, let suf = suffixes.first(where: { w.hasSuffix($0) && w.count - $0.count >= 4 }) else { break }
            w = String(w.dropLast(suf.count))
        }
        return w
    }
}

/// Okapi BM25 (k1 = 1,2, b = 0,75) über vortokenisierte Dokumente.
public struct BookTalkBM25: Sendable {
    public static let k1 = 1.2
    public static let b = 0.75

    private let tf: [[String: Int]]
    private let lengths: [Double]
    private let avgLength: Double
    private let df: [String: Int]

    public init(documents: [[String]]) {
        var tf: [[String: Int]] = []
        var df: [String: Int] = [:]
        var lengths: [Double] = []
        for doc in documents {
            var m: [String: Int] = [:]
            for t in doc { m[t, default: 0] += 1 }
            for t in m.keys { df[t, default: 0] += 1 }
            tf.append(m)
            lengths.append(Double(doc.count))
        }
        self.tf = tf
        self.df = df
        self.lengths = lengths
        self.avgLength = lengths.isEmpty ? 0 : max(lengths.reduce(0, +) / Double(lengths.count), 1)
    }

    public var count: Int { tf.count }

    public func score(query: [String], document i: Int) -> Double {
        guard i >= 0, i < tf.count else { return 0 }
        let n = Double(tf.count)
        var total = 0.0
        for term in Set(query) {
            guard let f = tf[i][term], let d = df[term] else { continue }
            let idf = log(1 + (n - Double(d) + 0.5) / (Double(d) + 0.5))
            let ff = Double(f)
            total += idf * (ff * (Self.k1 + 1)) / (ff + Self.k1 * (1 - Self.b + Self.b * lengths[i] / avgLength))
        }
        return total
    }

    /// Dokumente mit Punktzahl > 0, beste zuerst (bei Gleichstand frühere zuerst).
    public func rank(query: [String], within candidates: [Int]) -> [(index: Int, score: Double)] {
        candidates
            .map { (index: $0, score: score(query: query, document: $0)) }
            .filter { $0.score > 0 }
            .sorted { $0.score != $1.score ? $0.score > $1.score : $0.index < $1.index }
    }
}

/// Festes Budget je Modellprofil. Zeichenzahlen sind nur Startwerte für die Auswahl; maßgeblich ist die
/// harte Token-Prüfung `fits` vor dem Aufruf.
public struct BookTalkBudget: Sendable, Equatable {
    public let name: String
    public let instructionChars: Int
    public let historyChars: Int
    public let questionChars: Int
    public let excerptChars: Int
    public let answerTokens: Int
    /// Kontextfenster des Modells in Token (Foundation Models: 4096).
    public let contextTokens: Int

    /// Apple Foundation Models: 4096 Token gesamt; Auszüge ≤ 2.600 Token ≈ 7.000 Zeichen (Startwert).
    public static let foundationModels = BookTalkBudget(
        name: "foundation_models", instructionChars: 1_200, historyChars: 1_400, questionChars: 520,
        excerptChars: 7_000, answerTokens: 350, contextTokens: 4_096)
    /// Großes Profil (Gemma 4 E2B über llama.cpp, G6): Auszüge ≤ 6.000 Token ≈ 18.000 Zeichen (Startwert).
    public static let large = BookTalkBudget(
        name: "large", instructionChars: 1_200, historyChars: 3_000, questionChars: 800,
        excerptChars: 18_000, answerTokens: 500, contextTokens: 12_288)

    /// Profil `apple-fm@1` (Plan §11.1): Kontext 4096, Ausgabe 400; zusammen mit `BookTalkByteProfile.appleFM1`.
    public static let appleFM1 = BookTalkBudget(
        name: "apple-fm@1", instructionChars: 1_200, historyChars: 1_400, questionChars: 520,
        excerptChars: 7_000, answerTokens: 400, contextTokens: 4_096)

    /// Profil `relay-cli@1` (Plan §11.3): Abo-Modell über den PC, Auszüge bis ~20.000 Zeichen.
    /// `contextTokens` meldet der PC je Modell; die Ausgabe ist auf 800 Token begrenzt (kurze Antworten).
    public static func relayCLI1(contextTokens: Int) -> BookTalkBudget {
        BookTalkBudget(
            name: "relay-cli@1", instructionChars: 1_200, historyChars: 3_000, questionChars: 800,
            excerptChars: 20_000, answerTokens: 800, contextTokens: max(contextTokens, 0))
    }

    public init(name: String, instructionChars: Int, historyChars: Int, questionChars: Int, excerptChars: Int,
                answerTokens: Int, contextTokens: Int) {
        self.name = name
        self.instructionChars = instructionChars
        self.historyChars = historyChars
        self.questionChars = questionChars
        self.excerptChars = excerptChars
        self.answerTokens = answerTokens
        self.contextTokens = contextTokens
    }

    /// Obere Grenze der Prompt-Token. Mit exaktem Zähler dessen Wert, sonst nur mit einem gemessenen
    /// Byte-Profil; ohne beides nil (= nicht prüfbar, Grund `budget`).
    public static func promptTokens(
        instructions: String, prompt: String, counter: (any BookTalkTokenCounter)?, profile: BookTalkByteProfile?
    ) -> Int? {
        if let counter { return counter.count(instructions) + counter.count(prompt) }
        guard let profile else { return nil }
        return instructions.utf8.count + prompt.utf8.count + profile.fixedTokens + 2 * profile.perMessageTokens
    }

    public func fits(instructions: String, prompt: String, counter: (any BookTalkTokenCounter)?, profile: BookTalkByteProfile?) -> Bool {
        guard let t = Self.promptTokens(instructions: instructions, prompt: prompt, counter: counter, profile: profile) else { return false }
        return t + answerTokens <= contextTokens
    }
}

public struct BookTalkExcerpt: Sendable, Equatable {
    /// 1-basiert, in Buchreihenfolge: `[Q<id>]`.
    public let id: Int
    /// Bei Spoilergrenze am `frontier` gekürzt.
    public let chunk: BookTalkChunk
    public let score: Double
    public let isFirstMention: Bool
    /// 0 = wichtigster Auszug (Erstnennung, dann Punktzahl; recap: neuester).
    public let rank: Int
}

public struct BookTalkRetrieval: Sendable {
    public let intent: BookTalkIntent
    /// In Buchreihenfolge, nummeriert Q1…Qk.
    public let excerpts: [BookTalkExcerpt]
    /// Im erlaubten Bereich gibt es keinen Text (Grenze vor dem ersten Stück).
    public let scopeEmpty: Bool
    /// Namen/Varianten mit nachgewiesenem Vorkommen im sichtbaren Text, mit denen die Anfrage erweitert wurde.
    /// Nur intern: erscheint nie in Antwort oder Oberfläche.
    public let expansion: [String]
    public var excerptChars: Int { excerpts.reduce(0) { $0 + $1.chunk.text.utf16.count } }
    /// Beste zuerst (für found_only).
    public var bestFirst: [BookTalkExcerpt] {
        excerpts.sorted { $0.score != $1.score ? $0.score > $1.score : $0.id < $1.id }
    }

    /// Wirft den rangniedrigsten Auszug weg und nummeriert neu (hartes Budget).
    public func removingLowestRank() -> BookTalkRetrieval {
        guard let worst = excerpts.max(by: { $0.rank < $1.rank }) else { return self }
        let rest = excerpts.filter { $0.id != worst.id }
        let renumbered = rest.enumerated().map { n, e in
            BookTalkExcerpt(id: n + 1, chunk: e.chunk, score: e.score, isFirstMention: e.isFirstMention, rank: e.rank)
        }
        return BookTalkRetrieval(intent: intent, excerpts: renumbered, scopeEmpty: scopeEmpty, expansion: expansion)
    }
}

public enum BookTalkRetriever {
    public static let defaultK = 6
    /// Treffer unter 40 % der besten Punktzahl fallen weg (weniger Rauschen im Prompt).
    public static let minRelativeScore = 0.4
    public static let recapChunks = 5
    /// Pro Auszug Kopfzeile und Begrenzer im Prompt.
    static let headerChars = 90

    /// - Parameters:
    ///   - frontier: weitester gelesener/gehörter Stand (UTF-16-Offset im Original), vom Aufrufer.
    ///     Bei `.untilPosition` ohne `frontier` bleibt der Schutz an (frontier = 0, kein Treffer).
    ///   - previousQuestion: letzte Nutzerfrage (Pronomen-Auflösung ohne Modell).
    ///   - entities: Figuren/Orte/Begriffe; nur Namen und Varianten, die in der Frage vorkommen und im sichtbaren
    ///     Text (vor der Grenze) nachgewiesen sind, erweitern die Anfrage. Jede Form wird einzeln geprüft.
    public static func retrieve(
        index: BookTalkIndex, question: String, intent: BookTalkIntent, frontier: Int?, scope: BookTalkScope,
        previousQuestion: String? = nil, entities: [BookTalkEntity] = [], k: Int = defaultK, maxChars: Int
    ) -> BookTalkRetrieval {
        let visible: [BookTalkChunk]
        if scope == .book {
            visible = index.chunks
        } else {
            let f = max(frontier ?? 0, 0)
            visible = index.chunks.compactMap { $0.start < f ? $0.truncated(atFrontier: f) : nil }
        }
        guard !visible.isEmpty else { return BookTalkRetrieval(intent: intent, excerpts: [], scopeEmpty: true, expansion: []) }

        var query = BookTalkTokens.tokenize(question)
        if let p = previousQuestion { query += BookTalkTokens.tokenize(p) }
        let asked = BookTalkText.foldedSequence(question + " " + (previousQuestion ?? ""))
        let visibleWords = visible.map { BookTalkText.foldedSequence($0.text) }
        var expansion: [String] = []
        for e in entities where e.forms.contains(where: { BookTalkText.contains(asked, form: $0) }) {
            for f in e.forms where visibleWords.contains(where: { BookTalkText.contains($0, form: f) }) {
                expansion.append(f)
                query += BookTalkTokens.tokenize(f)
            }
        }
        let bm25 = BookTalkBM25(documents: visible.map { BookTalkTokens.tokenize($0.text) })
        let ranked = bm25.rank(query: query, within: Array(visible.indices))

        var picked: [(i: Int, score: Double, first: Bool)] = []
        if intent == .recap {
            let lastChapter = visible[visible.count - 1].chapterIndex
            var take = Array(visible.indices.filter { visible[$0].chapterIndex == lastChapter }.suffix(recapChunks))
            if take.count < 2, let before = visible.indices.last(where: { visible[$0].chapterIndex != lastChapter }) {
                take.insert(before, at: 0)
            }
            picked = take.map { (i: $0, score: 0, first: false) }
        } else {
            let top = ranked.first?.score ?? 0
            picked = ranked.filter { $0.score >= top * minRelativeScore }.prefix(k).map { (i: $0.index, score: $0.score, first: false) }
            if intent == .locate, let earliest = ranked.map(\.index).min() {
                if let n = picked.firstIndex(where: { $0.i == earliest }) {
                    picked[n].first = true
                } else {
                    let s = ranked.first(where: { $0.index == earliest })?.score ?? 0
                    if picked.count >= k { picked.removeLast() }
                    picked.append((i: earliest, score: s, first: true))
                }
            }
        }

        let priority: [(i: Int, score: Double, first: Bool)] = intent == .recap
            ? picked.reversed()
            : picked.sorted { a, b in
                if a.first != b.first { return a.first }
                return a.score != b.score ? a.score > b.score : a.i < b.i
            }
        var kept: [(i: Int, score: Double, first: Bool, rank: Int)] = []
        var used = 0
        for (rank, p) in priority.enumerated() {
            let cost = visible[p.i].text.utf16.count + headerChars
            if used + cost > maxChars { continue }
            used += cost
            kept.append((p.i, p.score, p.first, rank))
        }
        let excerpts = kept.sorted { $0.i < $1.i }.enumerated().map { n, p in
            BookTalkExcerpt(id: n + 1, chunk: visible[p.i], score: p.score, isFirstMention: p.first, rank: p.rank)
        }
        return BookTalkRetrieval(intent: intent, excerpts: excerpts, scopeEmpty: false, expansion: expansion)
    }
}
