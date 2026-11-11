import Foundation

// Gesprächsmodus „Sprechen“ (Paket K): Index im Arbeitsspeicher über Bücher, Transkripte und Notizen.
// Stücke von ~700 Zeichen mit ~100 Zeichen Überlappung, BM25 über Wortstämme (BookTalkTokens). Nichts davon liegt auf
// Platte. Kompakt gebaut, damit auch ~30 MB Text passen: ein umgekehrter Index (Stamm → Fundstellen) statt einer
// Häufigkeitstabelle je Stück, Stücke nur als Text plus wenige Zahlen.

/// Zerlegt Text in Stücke an Absatz-/Satz-/Wortgrenzen und weist je Stück nach, ob Anfang und Ende an einer Satzgrenze liegen.
enum AssistantChunker {
    static let target = 700
    static let overlap = 100

    /// Ein Stück mit Nachweis der Satzgrenzen. Ohne Nachweis wird sein erster bzw. letzter Satz nie zitiert (er kann
    /// angeschnitten sein).
    struct Piece: Equatable {
        let text: String
        let startsAtBoundary: Bool
        let endsAtBoundary: Bool
    }

    /// Nachgewiesene Grenzen eines Textes: Satzanfänge und -enden (abgeschlossen von der Satzerkennung, die Abkürzungen kennt)
    /// und Absatzgrenzen (Leerzeile). Ein Zeilenumbruch allein beweist nichts (harter Umbruch im Fließtext).
    private struct Marks {
        var starts: [Int] = []
        var ends: [Int] = []
        let startSet: Set<Int>
        let endSet: Set<Int>

        init(_ u: [UInt16], text: String) {
            var starts = Set<Int>()
            var ends = Set<Int>()
            for s in BookTalkSentences.split(text) {
                starts.insert(s.start)
                ends.insert(s.end)
            }
            var i = 0
            while i < u.count {
                guard u[i] == 10 else { i += 1; continue }
                var j = i + 1
                var breaks = 1
                while j < u.count, AssistantChunker.isSpace(u[j]) {
                    if u[j] == 10 { breaks += 1 }
                    j += 1
                }
                if breaks >= 2 {
                    var t = i
                    while t > 0, AssistantChunker.isSpace(u[t - 1]) { t -= 1 }
                    if t > 0 { ends.insert(t) }
                    if j < u.count { starts.insert(j) }
                }
                i = j
            }
            startSet = starts
            endSet = ends
            self.starts = starts.sorted()
            self.ends = ends.sorted()
        }

        static func lastAtMost(_ a: [Int], _ x: Int) -> Int? {
            var lo = 0, hi = a.count
            while lo < hi {
                let m = (lo + hi) / 2
                if a[m] <= x { lo = m + 1 } else { hi = m }
            }
            return lo > 0 ? a[lo - 1] : nil
        }

        static func firstAtLeast(_ a: [Int], _ x: Int) -> Int? {
            var lo = 0, hi = a.count
            while lo < hi {
                let m = (lo + hi) / 2
                if a[m] < x { lo = m + 1 } else { hi = m }
            }
            return lo < a.count ? a[lo] : nil
        }
    }

    /// Teilt einen Abschnitt. Kurze Texte bleiben ein Stück; sonst endet jedes Stück möglichst an einer nachgewiesenen Satz- oder
    /// Absatzgrenze (dann ein Zeilenumbruch, dann eine Wortgrenze), und das nächste beginnt ~`overlap` Zeichen früher, wenn
    /// möglich an einem nachgewiesenen Satzanfang, sonst an einer Wortgrenze. Anfang und Ende des Abschnitts tragen
    /// `startsAtBoundary`/`endsAtBoundary` des Abschnitts, alle inneren Kanten gelten nur mit Nachweis.
    static func split(_ text: String, startsAtBoundary: Bool = false, endsAtBoundary: Bool = false,
                      target: Int = AssistantChunker.target, overlap: Int = AssistantChunker.overlap) -> [Piece] {
        let u = Array(text.utf16)
        let n = u.count
        guard n > 0 else { return [] }
        var first = 0
        while first < n, isSpace(u[first]) { first += 1 }
        var last = n
        while last > first, isSpace(u[last - 1]) { last -= 1 }
        guard last > first else { return [] }
        let marks = Marks(u, text: text)

        func piece(_ s: Int, _ e: Int) -> Piece? {
            var a = s
            while a < e, isSpace(u[a]) { a += 1 }
            var b = e
            while b > a, isSpace(u[b - 1]) { b -= 1 }
            guard b > a else { return nil }
            return Piece(text: String(decoding: u[a..<b], as: UTF16.self),
                         startsAtBoundary: a <= first ? startsAtBoundary : marks.startSet.contains(a),
                         endsAtBoundary: b >= last ? endsAtBoundary : marks.endSet.contains(b))
        }

        if n <= target { return piece(0, n).map { [$0] } ?? [] }
        var out: [Piece] = []
        var s = 0
        while s < n {
            var e = min(s + target, n)
            if e < n { e = boundary(u, marks: marks, from: s, to: e, target: target) }
            if let p = piece(s, e) { out.append(p) }
            if e >= n { break }
            var next = max(e - overlap, s + 1)
            if let p = Marks.firstAtLeast(marks.starts, max(e - 2 * overlap, s + 1)), p < e {
                next = p
            } else {
                while next < e, !isSpace(u[next]) { next += 1 }
                while next < e, isSpace(u[next]) { next += 1 }
            }
            s = next >= e ? e : next
        }
        return out
    }

    fileprivate static func isSpace(_ c: UInt16) -> Bool { c == 32 || c == 10 || c == 9 || c == 13 || c == 0xA0 }

    /// Bevorzugt eine nachgewiesene Satz- oder Absatzgrenze, dann einen Zeilenumbruch, dann eine Wortgrenze; sonst harte Kante.
    private static func boundary(_ u: [UInt16], marks: Marks, from s: Int, to e: Int, target: Int) -> Int {
        let lower = s + target / 2
        if let c = Marks.lastAtMost(marks.ends, e), c > lower { return c }
        var k = e
        while k > lower { if u[k - 1] == 10 { return k }; k -= 1 }
        k = e
        while k > lower { if isSpace(u[k - 1]) { return k }; k -= 1 }
        // Nicht mitten in ein Surrogatpaar schneiden.
        if e < u.count, (0xD800...0xDBFF).contains(u[e - 1]) { return e - 1 }
        return e
    }
}

public actor AssistantCorpus {
    private struct Key: Hashable {
        let area: AssistantArea
        let id: String
    }

    private struct Doc {
        var item: AssistantItem
        var version: String
        var titleStems: Set<String>
        var first: Int
        var count: Int
        var alive: Bool
    }

    private struct Chunk {
        let doc: Int32
        let area: UInt8
        let locator: String?
        let text: String
        let length: Int32
        /// Bit 0: Anfang an einer Satzgrenze, Bit 1: Ende an einer Satzgrenze (nachgewiesen).
        let flags: UInt8
        var alive: Bool
    }

    private struct Posting {
        var chunk: Int32
        var tf: UInt16
    }

    private struct Hit {
        var score = 0.0
        var terms = 0
        var rare = false
    }

    /// Höchstens so viele Stücke je Element im Ergebnis, damit mehrere Quellen vorkommen.
    static let perItemCap = 3
    /// Untergrenze relativ zur besten Punktzahl; darunter sind Treffer Rauschen.
    static let relativeFloor = 0.2

    private var docs: [Doc] = []
    private var chunks: [Chunk] = []
    private var postings: [String: [Posting]] = [:]
    private var slots: [Key: Int] = [:]
    private var deadChunks = 0
    private var areaChunks = [Int](repeating: 0, count: AssistantArea.allCases.count)
    private var areaLength = [Int](repeating: 0, count: AssistantArea.allCases.count)

    public init() {}

    public func version(of id: String, area: AssistantArea) -> String? {
        slots[Key(area: area, id: id)].map { docs[$0].version }
    }

    /// Gleiche Version = nichts zu tun.
    public func upsert(_ document: AssistantDocument) {
        let key = Key(area: document.item.area, id: document.item.id)
        if let slot = slots[key] {
            if docs[slot].version == document.version { return }
            drop(slot)
            compactIfNeeded()
        }
        let slot = docs.count
        let first = chunks.count
        let areaSlot = document.item.area.order
        for piece in Self.pieces(document.sections) {
            let tokens = BookTalkTokens.tokenize(piece.text)
            guard !tokens.isEmpty else { continue }
            let id = Int32(chunks.count)
            var tf: [String: Int] = [:]
            for t in tokens { tf[t, default: 0] += 1 }
            for (t, f) in tf { postings[t, default: []].append(Posting(chunk: id, tf: UInt16(min(f, Int(UInt16.max))))) }
            chunks.append(Chunk(doc: Int32(slot), area: UInt8(areaSlot), locator: piece.locator, text: piece.text,
                                length: Int32(tokens.count), flags: (piece.startsAtBoundary ? 1 : 0) | (piece.endsAtBoundary ? 2 : 0),
                                alive: true))
            areaChunks[areaSlot] += 1
            areaLength[areaSlot] += tokens.count
        }
        docs.append(Doc(item: document.item, version: document.version, titleStems: Set(BookTalkTokens.tokenize(document.item.title)),
                        first: first, count: chunks.count - first, alive: true))
        slots[key] = slot
    }

    public func remove(id: String, area: AssistantArea) {
        guard let slot = slots[Key(area: area, id: id)] else { return }
        drop(slot)
        compactIfNeeded()
    }

    public func removeAll(area: AssistantArea) {
        for (key, slot) in slots where key.area == area { drop(slot) }
        compactIfNeeded()
    }

    public func items(area: AssistantArea?) -> [AssistantItem] {
        docs.filter { $0.alive && (area == nil || $0.item.area == area) }
            .map(\.item)
            .sorted { a, b in
                if a.area != b.area { return a.area.order < b.area.order }
                let (x, y) = (a.updated ?? .distantPast, b.updated ?? .distantPast)
                return x != y ? x > y : a.title.localizedCompare(b.title) == .orderedAscending
            }
    }

    public func search(_ question: String, areas: Set<AssistantArea>, limit: Int) -> [AssistantPassage] {
        search(question, previous: nil, areas: areas, limit: limit)
    }

    /// Wie `search`; `previous` (vorige Nutzeräußerung) fließt mit halbem Gewicht ein (kurze Anschlussfragen).
    public func search(_ question: String, previous: String?, areas: Set<AssistantArea>, limit: Int) -> [AssistantPassage] {
        guard limit > 0, !areas.isEmpty else { return [] }
        var weights: [String: Double] = [:]
        for t in BookTalkTokens.tokenize(question) { weights[t] = 1 }
        if let previous { for t in BookTalkTokens.tokenize(previous) where weights[t] == nil { weights[t] = 0.5 } }
        guard !weights.isEmpty else { return [] }

        var selected = [Bool](repeating: false, count: AssistantArea.allCases.count)
        for a in areas { selected[a.order] = true }
        let total = AssistantArea.allCases.reduce(0) { $0 + (selected[$1.order] ? areaChunks[$1.order] : 0) }
        guard total > 0 else { return [] }
        let length = AssistantArea.allCases.reduce(0) { $0 + (selected[$1.order] ? areaLength[$1.order] : 0) }
        let n = Double(total)
        let avg = max(Double(length) / n, 1)
        let rareLimit = max(2, total / 33)

        var hits: [Int32: Hit] = [:]
        for (term, weight) in weights {
            guard let list = postings[term] else { continue }
            var df = 0
            for p in list { let c = chunks[Int(p.chunk)]; if c.alive, selected[Int(c.area)] { df += 1 } }
            guard df > 0 else { continue }
            let idf = log(1 + (n - Double(df) + 0.5) / (Double(df) + 0.5))
            let rare = df <= rareLimit
            for p in list {
                let c = chunks[Int(p.chunk)]
                guard c.alive, selected[Int(c.area)] else { continue }
                let f = Double(p.tf)
                let part = idf * (f * (BookTalkBM25.k1 + 1)) / (f + BookTalkBM25.k1 * (1 - BookTalkBM25.b + BookTalkBM25.b * Double(c.length) / avg))
                var h = hits[p.chunk, default: Hit()]
                h.score += part * weight
                h.terms += 1
                h.rare = h.rare || rare
                hits[p.chunk] = h
            }
        }
        // Bei längeren Fragen genügt ein einzelner häufiger Begriff nicht als Treffer.
        if weights.count >= 3 { hits = hits.filter { $0.value.terms >= 2 || $0.value.rare } }

        var scored = hits.mapValues(\.score)
        applyTitleBonus(&scored, terms: Set(weights.keys), selected: selected)
        guard let top = scored.values.max(), top > 0 else { return [] }
        let floor = top * Self.relativeFloor
        let ranked = scored.filter { $0.value >= floor }.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }

        var perItem: [Int: Int] = [:]
        var out: [AssistantPassage] = []
        for (id, score) in ranked {
            let c = chunks[Int(id)]
            let slot = Int(c.doc)
            if perItem[slot, default: 0] >= Self.perItemCap { continue }
            perItem[slot, default: 0] += 1
            let item = docs[slot].item
            out.append(AssistantPassage(area: item.area, itemId: item.id, title: item.title, locator: c.locator, text: c.text, score: score,
                                        startsAtBoundary: c.flags & 1 != 0, endsAtBoundary: c.flags & 2 != 0))
            if out.count >= limit { break }
        }
        return out
    }

    // MARK: Intern

    /// Nennt die Frage Buch- oder Transkripttitel, bekommen Stücke dieses Elements einen Bonus; hat es keinen Treffer im
    /// Text, rücken seine ersten Stücke nach.
    private func applyTitleBonus(_ scored: inout [Int32: Double], terms: Set<String>, selected: [Bool]) {
        var matched: [Int] = []
        for (slot, d) in docs.enumerated() where d.alive && selected[d.item.area.order] && !d.titleStems.isEmpty {
            let common = d.titleStems.intersection(terms).count
            if common >= 1, Double(common) / Double(d.titleStems.count) >= 0.6 { matched.append(slot) }
        }
        guard !matched.isEmpty else { return }
        let bonus = max(scored.values.max() ?? 0, 1) * 0.5
        for slot in matched {
            let d = docs[slot]
            var present = 0
            for id in d.first..<(d.first + d.count) where chunks[id].alive {
                if let s = scored[Int32(id)] { scored[Int32(id)] = s + bonus; present += 1 }
            }
            if present == 0 {
                for id in d.first..<(d.first + min(d.count, Self.perItemCap)) where chunks[id].alive { scored[Int32(id)] = bonus }
            }
        }
    }

    private struct Part {
        var locator: String?
        var text: String
        var startsAtBoundary: Bool
        var endsAtBoundary: Bool
    }

    /// Stücke aus den Abschnitten. Kurze Abschnitte werden nur zusammengezogen, wenn die Naht nachweislich eine Satzgrenze ist
    /// (der vorige Abschnitt endet und der nächste beginnt an einer): sonst entstünde ein Satz, der in Wahrheit zwei
    /// Bruchstücke verbindet. Abschnitte ohne Nachweis bleiben einzelne, kleine Stücke.
    private static func pieces(_ sections: [AssistantDocument.Section]) -> [Part] {
        var out: [Part] = []
        var pending: Part?
        func flush() {
            if let p = pending { out.append(p) }
            pending = nil
        }
        for section in sections {
            let text = section.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            if text.utf16.count > AssistantChunker.target {
                flush()
                for piece in AssistantChunker.split(text, startsAtBoundary: section.startsAtBoundary, endsAtBoundary: section.endsAtBoundary) {
                    out.append(Part(locator: section.locator, text: piece.text, startsAtBoundary: piece.startsAtBoundary,
                                    endsAtBoundary: piece.endsAtBoundary))
                }
            } else if let p = pending, p.locator == section.locator, p.endsAtBoundary, section.startsAtBoundary,
                      p.text.utf16.count + 1 + text.utf16.count <= AssistantChunker.target {
                pending = Part(locator: p.locator, text: p.text + "\n" + text, startsAtBoundary: p.startsAtBoundary,
                               endsAtBoundary: section.endsAtBoundary)
            } else {
                flush()
                pending = Part(locator: section.locator, text: text, startsAtBoundary: section.startsAtBoundary,
                               endsAtBoundary: section.endsAtBoundary)
            }
        }
        flush()
        return out
    }

    private func drop(_ slot: Int) {
        let d = docs[slot]
        guard d.alive else { return }
        docs[slot].alive = false
        slots[Key(area: d.item.area, id: d.item.id)] = nil
        for id in d.first..<(d.first + d.count) where chunks[id].alive {
            chunks[id].alive = false
            areaChunks[Int(chunks[id].area)] -= 1
            areaLength[Int(chunks[id].area)] -= Int(chunks[id].length)
            deadChunks += 1
        }
    }

    private func compactIfNeeded() {
        if deadChunks > 2_000, deadChunks > chunks.count / 2 { compact() }
    }

    /// Baut die Felder ohne tote Stücke neu auf (Stücke eines Dokuments bleiben zusammenhängend und in Reihenfolge).
    private func compact() {
        var newDocs: [Doc] = []
        var newChunks: [Chunk] = []
        var map = [Int32](repeating: -1, count: chunks.count)
        var newSlots: [Key: Int] = [:]
        for d in docs where d.alive {
            var nd = d
            let slot = newDocs.count
            nd.first = newChunks.count
            for id in d.first..<(d.first + d.count) where chunks[id].alive {
                map[id] = Int32(newChunks.count)
                let c = chunks[id]
                newChunks.append(Chunk(doc: Int32(slot), area: c.area, locator: c.locator, text: c.text, length: c.length, flags: c.flags,
                                       alive: true))
            }
            nd.count = newChunks.count - nd.first
            newDocs.append(nd)
            newSlots[Key(area: d.item.area, id: d.item.id)] = slot
        }
        var newPostings: [String: [Posting]] = [:]
        newPostings.reserveCapacity(postings.count)
        for (term, list) in postings {
            let kept = list.compactMap { p -> Posting? in
                let m = map[Int(p.chunk)]
                return m < 0 ? nil : Posting(chunk: m, tf: p.tf)
            }
            if !kept.isEmpty { newPostings[term] = kept }
        }
        docs = newDocs
        chunks = newChunks
        postings = newPostings
        slots = newSlots
        deadChunks = 0
    }
}
