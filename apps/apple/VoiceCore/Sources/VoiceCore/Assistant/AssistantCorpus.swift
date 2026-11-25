import Foundation

// Gesprächsmodus „Sprechen“ (Paket K, E4.2b): Index im Arbeitsspeicher über Bücher, Transkripte und Notizen.
// Die Einheit des Index ist der QUELLBLOCK (`AssistantDocument.Section`: Absatz, Notizblock, Segment), ganz und unverändert:
// kein Teilen in Stücke, keine Überlappung, kein Zusammenziehen kurzer Blöcke. Ein Block über `AssistantGrounding.maxBlockChars`
// ist nicht zitierbar und wird nicht indexiert (nie gekürzt oder geteilt); er wird gezählt (`stats`). BM25 über Wortstämme
// (BookTalkTokens). Nichts davon liegt auf Platte. Kompakt gebaut, damit auch ~30 MB Text passen: ein umgekehrter Index
// (Stamm → Fundstellen) statt einer Häufigkeitstabelle je Block.

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
        /// Blöcke über der Maximallänge, die nicht indexiert wurden.
        var tooLong: Int
        /// Zeichen des aufbereiteten Textes aller Abschnitte (für die Obergrenze, `admit`).
        var size: Int
        var alive: Bool
    }

    private struct Chunk {
        let doc: Int32
        let area: UInt8
        let locator: String?
        let text: String
        let length: Int32
        /// Der rohe gespeicherte Block (E4.2b): nur er wird als Zitat ausgegeben; `text` ist der aufbereitete Text für die Suche.
        /// Ist beides bytegleich, teilen sich die Felder denselben Speicher.
        let raw: String?
        /// Die Quellart lässt wörtliche Ausgabe zu UND es gibt einen eindeutigen Rohtext.
        let quotable: Bool
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
    /// Summe der `size` der lebenden Dokumente.
    private var totalSize = 0
    private var areaChunks = [Int](repeating: 0, count: AssistantArea.allCases.count)
    private var areaLength = [Int](repeating: 0, count: AssistantArea.allCases.count)

    public init() {}

    /// Zahl der zitierbaren Blöcke und der Blöcke über der Maximallänge (nicht indexiert), je Bereich oder gesamt (Messung).
    public func stats(area: AssistantArea? = nil) -> (blocks: Int, tooLong: Int) {
        var blocks = 0, tooLong = 0
        for d in docs where d.alive && (area == nil || d.item.area == area) {
            blocks += (d.first..<(d.first + d.count)).filter { chunks[$0].alive }.count
            tooLong += d.tooLong
        }
        return (blocks, tooLong)
    }

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
        var tooLong = 0
        for block in Self.blocks(document.sections) {
            guard block.text.count <= AssistantGrounding.maxBlockChars, block.raw.map(AssistantGrounding.fits(raw:)) ?? true else {
                tooLong += 1
                continue
            }
            let tokens = BookTalkTokens.tokenize(block.text)
            guard !tokens.isEmpty else { continue }
            let id = Int32(chunks.count)
            var tf: [String: Int] = [:]
            for t in tokens { tf[t, default: 0] += 1 }
            for (t, f) in tf { postings[t, default: []].append(Posting(chunk: id, tf: UInt16(min(f, Int(UInt16.max))))) }
            chunks.append(Chunk(doc: Int32(slot), area: UInt8(areaSlot), locator: block.locator, text: block.text,
                                length: Int32(tokens.count), raw: block.raw, quotable: block.quotable, alive: true))
            areaChunks[areaSlot] += 1
            areaLength[areaSlot] += tokens.count
        }
        let size = Self.size(of: document)
        docs.append(Doc(item: document.item, version: document.version, titleStems: Set(BookTalkTokens.tokenize(document.item.title)),
                        first: first, count: chunks.count - first, tooLong: tooLong, size: size, alive: true))
        slots[key] = slot
        totalSize += size
    }

    private static func size(of document: AssistantDocument) -> Int { document.sections.reduce(0) { $0 + $1.text.count } }

    /// Nimmt ein Dokument unter einer Obergrenze für den Gesamttext (Zeichen) auf. `false`, wenn die Obergrenze überschritten würde:
    /// Dann scheidet das Dokument AUS dem Bestand aus (auch ein älterer Stand), damit ein Dokument, das sich nicht neu aufnehmen
    /// lässt, nie mit seinem alten Inhalt zitierbar bleibt (Löschung, geänderte Freigabe oder Lesegrenze wären sonst unwirksam).
    /// Erst danach darf der Aufrufer „bereit“ melden.
    @discardableResult
    public func admit(_ document: AssistantDocument, maxChars: Int) -> Bool {
        let key = Key(area: document.item.area, id: document.item.id)
        let own = slots[key].map { docs[$0].size } ?? 0
        guard totalSize - own + Self.size(of: document) <= maxChars else {
            if let slot = slots[key] {
                drop(slot)
                compactIfNeeded()
            }
            return false
        }
        upsert(document)
        return true
    }

    /// Gesamtgröße der Dokumente im Bestand (Zeichen des aufbereiteten Textes; Messung und Test).
    public func totalChars() -> Int { totalSize }

    /// Ersetzt den Bestand durch genau diese Dokumente (Klasse B: der neue Bestand nach einer Änderung von Inhalten): Was nicht
    /// mehr dabei ist, fällt weg, alles Übrige wird mit seiner Version aufgenommen.
    public func install(_ documents: [AssistantDocument]) {
        let keep = Set(documents.map { Key(area: $0.item.area, id: $0.item.id) })
        for (key, slot) in slots where !keep.contains(key) { drop(slot) }
        for document in documents { upsert(document) }
        compactIfNeeded()
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
    ///
    /// `only` (verknüpfte Kontexte, `AssistantScope`): nur Stücke dieser Elemente. Der Filter wirkt VOR der Begrenzung auf `limit` und
    /// vor der Untergrenze, nicht erst hinterher; Elemente, die nicht im Index stehen, kommen nie vor. Die Punktzahlen rechnen mit den
    /// Zahlen aller gewählten Bereiche (ein Treffer hat mit und ohne Filter denselben Wert). `floorFromAll`: die Untergrenze misst sich
    /// an der besten Punktzahl ALLER Stücke, nicht nur der gefilterten (Modus „zuerst“: ein schwacher Kontexttreffer ist kein Treffer);
    /// ohne sie (Modus „nur“) an der besten gefilterten.
    public func search(_ question: String, previous: String?, areas: Set<AssistantArea>, limit: Int,
                       only: Set<AssistantScope.Entry>? = nil, floorFromAll: Bool = false) -> [AssistantPassage] {
        guard limit > 0, !areas.isEmpty else { return [] }
        let allowed: Set<Int>? = only.map { entries in Set(entries.compactMap { slots[Key(area: $0.area, id: $0.itemId)] }) }
        if let allowed, allowed.isEmpty { return [] }
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
        var top = 0.0
        if let allowed, floorFromAll {
            applyTitleBonus(&scored, terms: Set(weights.keys), selected: selected, allowed: nil)
            top = scored.values.max() ?? 0
            scored = scored.filter { allowed.contains(Int(chunks[Int($0.key)].doc)) }
        } else {
            if let allowed { scored = scored.filter { allowed.contains(Int(chunks[Int($0.key)].doc)) } }
            applyTitleBonus(&scored, terms: Set(weights.keys), selected: selected, allowed: allowed)
            top = scored.values.max() ?? 0
        }
        guard top > 0, !scored.isEmpty else { return [] }
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
                                        isBlock: true, quotable: c.quotable, raw: c.raw))
            if out.count >= limit { break }
        }
        return out
    }

    // MARK: Intern

    /// Nennt die Frage Buch- oder Transkripttitel, bekommen Stücke dieses Elements einen Bonus; hat es keinen Treffer im
    /// Text, rücken seine ersten Stücke nach.
    private func applyTitleBonus(_ scored: inout [Int32: Double], terms: Set<String>, selected: [Bool], allowed: Set<Int>?) {
        var matched: [Int] = []
        for (slot, d) in docs.enumerated() where d.alive && selected[d.item.area.order] && !d.titleStems.isEmpty {
            if let allowed, !allowed.contains(slot) { continue }
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
        var raw: String?
        var quotable: Bool
    }

    /// Die Quellblöcke eines Dokuments: je Abschnitt genau einer. Der aufbereitete Text (Rand-Leerraum entfernt) ist nur die
    /// Grundlage der Suche; der Rohtext des Abschnitts bleibt UNVERÄNDERT (auch Rand-Leerraum, nie getrimmt). Leere Abschnitte
    /// entfallen; ein Rohtext nur aus Leerraum ist keiner. Ein Block über der Maximallänge (aufbereitet oder roh) wird nicht
    /// indexiert.
    private static func blocks(_ sections: [AssistantDocument.Section]) -> [Part] {
        sections.compactMap { section in
            let text = section.text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { return nil }
            var raw = section.raw
            if let r = raw, r.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { raw = nil }
            if let r = raw, r.hasSameBytes(as: text) { raw = text }
            let quotable = section.kind.isQuotable && raw != nil
            return Part(locator: section.locator, text: text, raw: raw, quotable: quotable)
        }
    }

    private func drop(_ slot: Int) {
        let d = docs[slot]
        guard d.alive else { return }
        docs[slot].alive = false
        totalSize -= d.size
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
                newChunks.append(Chunk(doc: Int32(slot), area: c.area, locator: c.locator, text: c.text, length: c.length,
                                       raw: c.raw, quotable: c.quotable, alive: true))
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
