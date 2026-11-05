import Foundation

// Wo das Vorlesen eines Buchs beginnt (Patrick 05.11.2026): beim ersten
// echten Inhalt statt bei Titelei, Inhaltsverzeichnis, Impressum, ISBN oder
// Widmung. Vorwort und Einleitung zählen als Inhalt, solange `readPreface`
// gilt (Standard).
//
// Gleiche Regeln wie `apps/local-voice/src/lib/reader/bookStart.ts`; beide
// prüfen gegen `apps/local-voice/tests/fixtures/book-start.json`. Offsets
// sind UTF-16-Einheiten wie in `LibraryText`.

public enum BookStart {
    public struct Result: Equatable, Sendable {
        /// Offset, an dem Vorlesen ohne gemerkte Stelle beginnt (0 = vorn).
        public var offset: Int
        /// Ob das Buch ein Vorwort/eine Einleitung vor dem ersten Kapitel hat.
        public var hasPreface: Bool
    }

    /// Schlüssel der Einstellung „Vorwort und Einleitung mitlesen“ (Standard: ja).
    public static let readPrefaceKey = "lva.reader.readPreface"

    /// Startpunkt eines Buchtexts; 0, wenn kein Vorspann erkennbar ist.
    public static func start(_ text: String, readPreface: Bool = true) -> Result {
        let ns = text as NSString
        let secs = sections(paragraphs(ns))
        let norm = secs.map { $0.heading.map(normalizeHeading) }
        let title: String? = secs.first.flatMap { $0.level == 1 ? norm[0] : nil }
        var pending: (start: Int, level: Int)?
        // Unterabschnitte eines übersprungenen Abschnitts bleiben übersprungen.
        var skipBelow: Int?
        var offset = 0, hasPreface = false, found = false
        for (i, s) in secs.enumerated() {
            if let below = skipBelow, s.heading != nil, s.level > below { continue }
            skipBelow = nil
            let kind = classify(s, index: i, count: secs.count, title: title, later: { Set(norm[(i + 1)...].compactMap { $0 }) })
            if kind == .neutral {
                if pending == nil || s.level <= pending!.level { pending = (s.start, s.level) }
                continue
            }
            if kind == .preface { hasPreface = true }
            if kind == .front || (kind == .preface && !readPreface) {
                pending = nil
                if s.heading != nil { skipBelow = s.level }
                continue
            }
            let head: Int? = pending.flatMap { $0.level < s.level ? $0.start : nil }
            if s.heading == nil {
                let at = preambleStart(s)
                if at == -1 { continue }
                offset = at ?? s.start
            } else {
                offset = head ?? s.start
            }
            found = true
            break
        }
        if !found || Double(offset) > Double(ns.length) * maxShare { return Result(offset: 0, hasPreface: hasPreface) }
        return Result(offset: offset, hasPreface: hasPreface)
    }

    // MARK: Regeln

    static let frontWords = [
        "inhalt", "inhaltsverzeichnis", "inhaltsübersicht", "inhaltsangabe", "verzeichnis", "contents",
        "table of contents", "toc", "impressum", "copyright", "copyrightseite", "copyright page",
        "rechtliche hinweise", "titel", "titelseite", "titelblatt", "title", "title page", "cover", "umschlag",
        "frontmatter", "front matter", "widmung", "dedication", "motto", "epigraph", "epigraf",
        "über das buch", "über dieses buch", "zum buch", "das buch", "buch", "about the book", "about this book",
        "über den autor", "über die autorin", "über die autoren", "über die autorinnen", "über den verfasser",
        "zum autor", "zur autorin", "zu den autoren", "der autor", "die autorin", "die autoren", "autor",
        "autorin", "autoren", "about the author", "about the authors", "bibliografische information",
        "bibliographische information", "abbildungsverzeichnis", "tabellenverzeichnis", "abkürzungsverzeichnis",
        "verzeichnis der abbildungen", "list of figures", "list of tables", "weitere titel", "weitere bücher",
        "also by", "praise for", "lob für", "pressestimmen", "stimmen zum buch",
    ]
    static let frontHeading = regex("^(?:" + frontWords.map { $0.replacingOccurrences(of: " ", with: "\\s+") }.joined(separator: "|")
        + ")$")
    // Ganze Überschrift: „Vorwort zur zweiten Auflage“ ja, „Einführung in die Algebra“ nicht.
    static let prefaceHeading = regex("^(?:vorwort|geleitwort|vorrede|vorbemerkung|einleitung|einführung|zur einführung|prolog|prologue|preface|foreword|introduction)(?:\\s+(?:zur|zum|des|der|von|to the|of the|by)\\s.*)?$")
    static let frontBody = [
        regex("\\bISBN(?:-1[03])?\\b"),
        regex("©|\\(c\\)\\s*(?:19|20)\\d\\d|\\bcopyright\\b", [.caseInsensitive]),
        regex("alle rechte vorbehalten|all rights reserved", [.caseInsensitive]),
        regex("deutsche(?:n)?\\s+nationalbibliothek|library of congress", [.caseInsensitive]),
        regex("originalausgabe|originaltitel|original title|erstausgabe|first published", [.caseInsensitive]),
        regex("umschlaggestaltung|covergestaltung|umschlagmotiv|cover design|satz und layout|printed in|gedruckt in", [.caseInsensitive]),
    ]
    static let frontBodyMax = 2500
    /// Unter einer gewöhnlichen Überschrift: nur kurze Abschnitte.
    static let frontSectionMax = 800
    static let genre = regex("^(?:roman|novel|a novel|erzählung|erzählungen|novelle|sachbuch|thriller|krimi|kriminalroman|gedichte|poems|stories)$", [.caseInsensitive])
    static let maxShare = 0.5
    static let pageMarks = regex("\\{\\{S\\. ?\\d{1,5}\\}\\}")
    static let headingLine = regex("^(#{1,6})\\s+(.*)$")
    static let byline = regex("^(?:von|by)\\s+\\p{Lu}")
    static let prose = regex("[.!?…:\"“”»«')]\\s*$")
    static let sentenceBreak = regex("[.!?…]\\s+\\p{Lu}")
    static let paragraphBreak = regex("\\n[ \\t\\r]*\\n")

    enum Kind { case front, preface, content, neutral }
    struct Para { let start: Int; let text: String }
    struct Section { let start: Int; let heading: String?; let level: Int; var body: [Para] }

    static func regex(_ p: String, _ o: NSRegularExpression.Options = []) -> NSRegularExpression {
        // Muster sind fest im Code; ein Fehler hier ist ein Programmierfehler.
        try! NSRegularExpression(pattern: p, options: o)
    }

    static func matches(_ r: NSRegularExpression, _ s: String) -> Bool {
        r.firstMatch(in: s, range: NSRange(location: 0, length: (s as NSString).length)) != nil
    }

    static func replace(_ r: NSRegularExpression, _ s: String, _ with: String) -> String {
        r.stringByReplacingMatches(in: s, range: NSRange(location: 0, length: (s as NSString).length), withTemplate: with)
    }

    /// JS-`trim()`: Leerraum, Zeilenumbrüche und BOM an beiden Enden.
    static let trimSet = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\u{FEFF}"))
    static func trim(_ s: String) -> String { s.trimmingCharacters(in: trimSet) }

    /// Überschrift ohne Markdown, Seitenmarken und Satzzeichen, klein.
    public static func normalizeHeading(_ raw: String) -> String {
        var s = replace(pageMarks, raw, " ")
        s = replace(regex("^#{1,6}\\s+"), s, "")
        s = replace(regex("[*_`]"), s, "")
        s = trim(replace(regex("\\s+"), s, " "))
        s = replace(regex("[\\s.:;,!?…–—-]+$"), s, "")
        s = replace(regex("^[\\s.:;,…–—-]+"), s, "")
        return s.lowercased()
    }

    static func paragraphs(_ ns: NSString) -> [Para] {
        var out: [Para] = []
        var from = 0
        func push(_ end: Int) {
            let raw = ns.substring(with: NSRange(location: from, length: end - from))
            let body = trim(raw)
            // Absätze nur aus Seitenmarken zählen nicht.
            guard !body.isEmpty, !trim(replace(pageMarks, body, "")).isEmpty else { return }
            let lead = (raw as NSString).range(of: body).location
            out.append(Para(start: from + lead, text: body))
        }
        for m in paragraphBreak.matches(in: ns as String, range: NSRange(location: 0, length: ns.length)) {
            push(m.range.location)
            from = m.range.location + m.range.length
        }
        push(ns.length)
        return out
    }

    static func sections(_ paras: [Para]) -> [Section] {
        var out: [Section] = []
        for p in paras {
            let first = p.text.components(separatedBy: "\n")[0]
            let probe = trim(replace(pageMarks, first, "")) as NSString
            if let h = headingLine.firstMatch(in: probe as String, range: NSRange(location: 0, length: probe.length)) {
                let f = first as NSString
                var sec = Section(start: p.start, heading: probe.substring(with: h.range(at: 2)), level: h.range(at: 1).length, body: [])
                let pt = p.text as NSString
                let rest = trim(pt.substring(from: f.length))
                if !rest.isEmpty {
                    let at = pt.range(of: rest, range: NSRange(location: f.length, length: pt.length - f.length)).location
                    sec.body.append(Para(start: p.start + at, text: rest))
                }
                out.append(sec)
                continue
            }
            if out.isEmpty { out.append(Section(start: p.start, heading: nil, level: 0, body: [])) }
            out[out.count - 1].body.append(p)
        }
        return out
    }

    static func bodyText(_ s: Section) -> String { s.body.map(\.text).joined(separator: "\n") }

    static func lines(_ s: Section) -> [String] {
        s.body.flatMap { $0.text.components(separatedBy: "\n") }
            .map { $0.hasSuffix("\r") ? String($0.dropLast()) : $0 }
            .map { trim(replace(pageMarks, $0, "")) }
            .filter { !$0.isEmpty }
    }

    /// Indizes der Impressum-Muster im Text (0 = ISBN).
    static func frontHits(_ text: String) -> Set<Int> {
        Set(frontBody.indices.filter { matches(frontBody[$0], text) })
    }

    /// Klar Impressum: eine ISBN oder zwei verschiedene Merkmale.
    static func strongFront(_ hits: Set<Int>) -> Bool { hits.contains(0) || hits.count >= 2 }

    static func looksLikeFront(_ text: String, max: Int) -> Bool {
        (text as NSString).length <= max && strongFront(frontHits(text))
    }

    /// Inhaltsverzeichnis ohne Überschrift: die meisten Zeilen sind spätere Überschriften.
    static func looksLikeToc(_ s: Section, _ laterSet: () -> Set<String>) -> Bool {
        let ls = lines(s)
        guard ls.count >= 3 else { return false }
        let later = laterSet()
        let hits = ls.filter { l in
            let n = normalizeHeading(replace(regex("[\\s.·…]*\\d{1,4}$"), replace(regex("^[-*•]\\s+"), l, ""), ""))
            return !n.isEmpty && later.contains(n)
        }.count
        return hits * 2 >= ls.count
    }

    /// Titelei: Überschrift mit nur kurzen Zeilen darunter („von …“, „Roman“, Verlag).
    static func looksLikeTitlePage(_ s: Section) -> Bool {
        guard s.heading != nil, s.level == 1, !s.body.isEmpty else { return false }
        // Ohne Autor- oder Genrezeile ist ein kurzer Abschnitt ein kurzes Kapitel.
        let marked = lines(s).contains { matches(byline, $0) || matches(genre, $0) }
        return marked && (bodyText(s) as NSString).length <= 400
            && s.body.allSatisfy { ($0.text as NSString).length <= 200 && !matches(sentenceBreak, $0.text) }
    }

    static func classify(_ s: Section, index: Int, count: Int, title: String?, later: () -> Set<String>) -> Kind {
        let text = bodyText(s)
        if let heading = s.heading {
            let h = normalizeHeading(heading)
            if matches(frontHeading, h) { return .front }
            if let title, h == title, index > 0 { return .front }
            if matches(prefaceHeading, h) { return .preface }
            if count > 1, looksLikeTitlePage(s) { return .front }
            if s.body.isEmpty { return .neutral }
            if looksLikeFront(text, max: frontSectionMax) { return .front }
            return looksLikeToc(s, later) ? .front : .content
        }
        if looksLikeFront(text, max: frontBodyMax) || looksLikeToc(s, later) { return .front }
        return .content
    }

    /// Vorspann ohne Überschriften (PDF): Offset des ersten Fließtexts nach einem
    /// klaren Impressum, nil (nichts überspringen) oder -1 (nur Impressum).
    static func preambleStart(_ s: Section) -> Int? {
        var lastFront = -1
        var hits = Set<Int>()
        for (i, p) in s.body.enumerated() {
            let len = (p.text as NSString).length
            if len > 400, matches(prose, p.text) { break }
            let h = len <= frontBodyMax ? frontHits(p.text) : []
            if h.isEmpty { continue }
            hits.formUnion(h)
            lastFront = i
        }
        guard lastFront >= 0, strongFront(hits) else { return nil }
        return lastFront + 1 < s.body.count ? s.body[lastFront + 1].start : -1
    }
}
