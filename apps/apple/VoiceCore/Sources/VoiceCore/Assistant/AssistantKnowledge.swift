import Foundation

// Gesprächsmodus „Sprechen“ (Paket K, E4.2a in Paket A): Abruf und Bestand über die freigegebenen Bereiche.
// Deterministisch (BM25), keine Werkzeugaufrufe des Modells. Bereiche, die nicht in `areas` stehen, kommen weder im
// Bestand noch im Abruf vor. Eine Anfrage der Sitzung übergibt ihre Bereiche aus dem Freigabezustand (`areas:`), gelesen mit der
// Generation aus einem Lock (Runde 8, R3: der eine Stand der Ordnung). Dieses Objekt hält KEINEN veränderlichen Stand der Bereiche:
// `areas` ist der feste Anfangswert (Aufrufer ohne Angabe: Anzeige, Tests; Sitzung ohne Zustandsdatei: ihr Anfangsstand) und wird nach
// dem Anlegen nie mehr gesetzt. Der Bestand kommt nicht mehr in den Prompt: Bestandsangaben (Zahl je Bereich, Titel) erzeugt das
// System aus diesen Daten (`inventoryStatements`), nie das Modell.

public actor AssistantKnowledge {
    private let corpus: AssistantCorpus
    private let sources: [any AssistantSource]
    /// Der feste Anfangswert der Bereiche (nie verändert; die Freigabe lebt in der Ordnung, nicht hier).
    public nonisolated let areas: Set<AssistantArea>
    /// Obergrenze der Bereiche (lva#556, Assistenten-Profil): nie mehr als diese, was immer die Sitzung als freigegeben übergibt. Fest wie
    /// `areas`, nie verändert; `nil`: keine Obergrenze (alle bisherigen Aufrufer). Wirkt in Abruf, Bestand und Prüfung gleichermaßen.
    /// Nötig, weil die Sitzung ihre Bereiche aus der Freigabe (Zustandsdatei) liest und `areas` nur der Anfangswert ist: Das Profil
    /// schränkt die Freigabe ein, es öffnet nie etwas und ändert die Freigabe selbst nicht.
    public nonisolated let ceiling: Set<AssistantArea>?
    /// Verknüpfte Kontexte (lva#557): `nil` (alle bisherigen Aufrufer): kein Filter, Verhalten wie bisher. Fest wie `ceiling`, nie
    /// verändert. `only`: Abruf, Bestand und Auszüge nur aus den Kontexten; `first`: Treffer aus den Kontexten zuerst. Der Filter wirkt
    /// im vorhandenen Abruf, innerhalb der wirksamen Bereiche; er öffnet nie etwas, was Freigabe, Obergrenze oder Index nicht liefern.
    /// Die Prüfung von Sätzen ohne Quelle (`probe`) ignoriert ihn: sie bleibt über alle freigegebenen Bereiche (die strengere Seite).
    public nonisolated let scope: AssistantScope?

    /// Höchstens so viele Auszüge je Element im Gesamtergebnis (wie im Index).
    static let perItemCap = 3
    /// Ein Auszug unter dieser Länge wird nicht mehr gekürzt aufgenommen, sondern weggelassen.
    static let minClipChars = 200

    public init(corpus: AssistantCorpus, sources: [any AssistantSource] = [], areas: Set<AssistantArea> = Set(AssistantArea.allCases),
                ceiling: Set<AssistantArea>? = nil, scope: AssistantScope? = nil) {
        self.corpus = corpus
        self.sources = sources
        self.areas = areas
        self.ceiling = ceiling
        self.scope = scope
    }

    /// Baut den Bestand neu (Klasse B): ersetzt den Index durch genau diese Dokumente.
    public func install(_ documents: [AssistantDocument]) async {
        await corpus.install(documents)
    }

    /// Dasselbe Wissen (Index und Quellen) mit anderen Anfangsbereichen: ein neues Objekt, nie eine Änderung dieses.
    public nonisolated func withAreas(_ areas: Set<AssistantArea>) -> AssistantKnowledge {
        AssistantKnowledge(corpus: corpus, sources: sources, areas: areas, ceiling: ceiling, scope: scope)
    }

    /// Dasselbe Wissen mit anderer Obergrenze (`nil`: keine); die Anfangsbereiche bleiben.
    public nonisolated func withCeiling(_ ceiling: Set<AssistantArea>?) -> AssistantKnowledge {
        AssistantKnowledge(corpus: corpus, sources: sources, areas: areas, ceiling: ceiling, scope: scope)
    }

    /// Dasselbe Wissen mit anderen verknüpften Kontexten (`nil`: keine); Bereiche und Obergrenze bleiben.
    public nonisolated func withScope(_ scope: AssistantScope?) -> AssistantKnowledge {
        AssistantKnowledge(corpus: corpus, sources: sources, areas: areas, ceiling: ceiling, scope: scope)
    }

    /// Der Anfangswert der Bereiche, ohne Aktor-Sprung (Anfangsstand einer Sitzung ohne Zustandsdatei), höchstens die Obergrenze.
    public nonisolated var currentAreas: Set<AssistantArea> { capped(areas) }

    /// Die Bereiche, mit denen eine Anfrage wirklich arbeitet: die übergebenen (sonst die Anfangsbereiche), höchstens die Obergrenze.
    nonisolated func capped(_ requested: Set<AssistantArea>?) -> Set<AssistantArea> {
        let base = requested ?? areas
        guard let ceiling else { return base }
        return base.intersection(ceiling)
    }

    // MARK: Bestand

    /// Höchstens so viele Titel für die Prüfung „nimmt ein Satz Bezug auf einen Titel aus dem Bestand“.
    static let titleCap = 400

    /// Titel aller Elemente der freigegebenen Bereiche, ohne Doppelte (für die Eigendatenprüfung von Frage und Satz).
    public func inventoryTitles(areas: Set<AssistantArea>? = nil) async -> [String] {
        let active = capped(areas)
        var seen = Set<String>()
        var out: [String] = []
        for area in AssistantArea.allCases where active.contains(area) {
            for item in await scopedItems(in: area) {
                let title = Self.oneLine(item.title)
                guard !title.isEmpty, seen.insert(title).inserted else { continue }
                out.append(title)
                if out.count >= Self.titleCap { return out }
            }
        }
        return out
    }

    private func items(in area: AssistantArea) async -> [AssistantItem] {
        var byId: [String: AssistantItem] = [:]
        for item in await corpus.items(area: area) { byId[item.id] = item }
        for source in sources where source.area == area {
            for item in await source.items() where item.area == area { byId[item.id] = item }
        }
        return Array(byId.values)
    }

    /// Wie `items(in:)`, im Modus „nur“ nur die Elemente der Kontexte (Bestand ausschließlich aus ihnen). Sonst unverändert.
    private func scopedItems(in area: AssistantArea) async -> [AssistantItem] {
        let all = await items(in: area)
        guard let scope, scope.mode == .only else { return all }
        return all.filter { scope.admits($0) }
    }

    /// Die Elemente des Index in den wirksamen Bereichen (Anfrage, sonst Anfangsbereiche, höchstens die Obergrenze), ohne Filter der
    /// Kontexte: Grundlage für Verfügbarkeit und Auswahl (Anhänge-Blatt). Nach Bereich, dann neueste zuerst, dann Titel.
    public func indexedItems(areas: Set<AssistantArea>? = nil) async -> [AssistantItem] {
        let active = capped(areas)
        var out: [AssistantItem] = []
        for area in AssistantArea.allCases where active.contains(area) {
            out += await items(in: area).sorted { a, b in
                let (x, y) = (a.updated ?? .distantPast, b.updated ?? .distantPast)
                return x != y ? x > y : a.title.localizedCompare(b.title) == .orderedAscending
            }
        }
        return out
    }

    /// Verfügbarkeit je Kontext gegen den Index der wirksamen Bereiche (`AssistantScope.availability`).
    public func availability(of contexts: [AssistantProfile.Context], areas: Set<AssistantArea>? = nil) async -> [AssistantContextAvailability] {
        AssistantScope.availability(of: contexts, in: await indexedItems(areas: areas))
    }

    private static func oneLine(_ s: String) -> String {
        BookTalkPrompt.neutralize(s).split(whereSeparator: { $0.isNewline || $0 == "\t" }).joined(separator: " ")
            .trimmingCharacters(in: .whitespaces)
    }

    /// Titel unverändert; nur Zeilenumbrüche und Tabulatoren werden zu Leerzeichen.
    private static func flat(_ s: String) -> String {
        s.split(whereSeparator: { $0.isNewline || $0 == "\t" }).joined(separator: " ").trimmingCharacters(in: .whitespaces)
    }

    // MARK: Bestandsangaben (vom System, nie vom Modell)

    /// Höchstens so viele Titel je Bereich in einer Bestandsangabe.
    static let statementTitles = 3

    /// Bereiche, nach denen eine Bestandsfrage fragt (Hauptwörter wie „Bücher“, „Transkripte“, „Notizen“, „Dokumente“);
    /// `nil`, wenn keiner genannt ist („Was habe ich gespeichert?“): dann alle freigegebenen.
    static func inventoryAreas(for question: String) -> Set<AssistantArea>? {
        var found = Set<AssistantArea>()
        for w in BookTalkTokens.foldedWords(question) {
            if ["buch", "buches", "bucher", "buchern", "horbuch", "horbucher", "bibliothek"].contains(w) { found.insert(.books) }
            if ["transkript", "besprechung", "meeting", "aufnahme", "aufgenommen", "protokoll", "mitschrift"].contains(where: { w.hasPrefix($0) }) {
                found.insert(.transcripts)
            }
            if ["dokument", "datei", "ablage"].contains(where: { w.hasPrefix($0) }) { found.insert(.files) }
            if ["notiz", "sprachnotiz"].contains(where: { w.hasPrefix($0) }) { found.insert(.notes) }
        }
        return found.isEmpty ? nil : found
    }

    /// „Du hast drei Bücher.“, „Du hast keine Notizen.“, dazu „Neueste Bücher: A, B und C.“ Zahl und Titel stehen unverändert
    /// und ungekürzt aus den Bestandsdaten im Satz (nur Zeilenumbrüche werden zu Leerzeichen, damit es ein Satz bleibt), sonst
    /// nichts (keine Herleitung, keine Empfehlung).
    static func statements(area: AssistantArea, items: [AssistantItem]) -> [String] {
        var out = ["Du hast \(area.countPhrase(items.count))."]
        guard !items.isEmpty else { return out }
        let sorted = items.sorted { a, b in
            let (x, y) = (a.updated ?? .distantPast, b.updated ?? .distantPast)
            return x != y ? x > y : a.title.localizedCompare(b.title) == .orderedAscending
        }
        let titles = sorted.prefix(statementTitles).map { flat($0.title) }.filter { !$0.isEmpty }
        guard !titles.isEmpty else { return out }
        let list = titles.count == 1 ? titles[0] : titles.dropLast().joined(separator: ", ") + " und " + titles[titles.count - 1]
        out.append("Neueste \(area.inventoryLabel): \(list).")
        return out
    }

    /// Bestandsangaben zu einer Bestandsfrage je Bereich, nur aus den freigegebenen Bereichen. Leer, wenn kein genannter Bereich
    /// freigegeben ist. Die Bereiche gehen ins Protokoll.
    public func inventoryGroups(question: String, areas: Set<AssistantArea>? = nil) async -> [(area: AssistantArea, statements: [String])] {
        let wanted = Self.inventoryAreas(for: question)
        let active = capped(areas)
        var out: [(area: AssistantArea, statements: [String])] = []
        for area in AssistantArea.allCases where active.contains(area) && (wanted?.contains(area) ?? true) {
            let items = await scopedItems(in: area)
            // Im Modus „nur“ steht ein Bereich ohne Kontext nicht im Bestand (wie ein nicht freigegebener Bereich).
            if let scope, scope.mode == .only, items.isEmpty { continue }
            out.append((area, Self.statements(area: area, items: items)))
        }
        return out
    }

    public func inventoryStatements(question: String) async -> [String] {
        await inventoryGroups(question: question).flatMap(\.statements)
    }

    /// Abruf für die Prüfung eines Satzes ohne Quelle: der SATZ ist die Anfrage, über alle freigegebenen Bereiche, ohne Vorfrage
    /// und ohne die Sonderbehandlung von Bestandsfragen, mit großzügiger Obergrenze. Nur zum Verwerfen, nie zum Zitieren.
    public func probe(sentence: String, areas: Set<AssistantArea>? = nil) async -> [AssistantPassage] {
        let active = capped(areas)
        let trimmed = sentence.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !active.isEmpty else { return [] }
        var found = await corpus.search(trimmed, previous: nil, areas: active, limit: Self.probeLimit)
        for source in sources where active.contains(source.area) {
            found += (await source.search(trimmed, limit: Self.probeLimit)).filter { $0.area == source.area && active.contains($0.area) }
        }
        return found
    }

    static let probeLimit = 16

    // MARK: Abruf

    /// Abruf: aktuelle Frage, bei kurzen Anschlussfragen zusätzlich die vorige Nutzeräußerung (halbes Gewicht).
    /// Scores sind je Bereich auf 0…1 normiert (beste Stelle des Bereichs = 1), damit Bereiche vergleichbar werden.
    /// Mit `scope` (verknüpfte Kontexte): `only` ruft ausschließlich aus den Kontexten ab (der Filter wirkt im Index, VOR `limit`),
    /// `first` stellt Treffer aus den Kontexten voran und behält die beste Stelle außerhalb (`mergeFirst`).
    public func retrieve(question: String, previous: String?, limit: Int, maxChars: Int,
                         areas: Set<AssistantArea>? = nil) async -> [AssistantPassage] {
        let active = capped(areas)
        guard limit > 0, maxChars > 0, !active.isEmpty else { return [] }
        let trimmed = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !Self.isInventoryQuestion(trimmed) else { return [] }
        let prev = Self.isFollowUp(trimmed) ? previous?.trimmingCharacters(in: .whitespacesAndNewlines) : nil
        let blended = [trimmed, prev].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
        let each = limit * 2

        guard let scope else {
            var found = await corpus.search(trimmed, previous: prev, areas: active, limit: each)
            for source in sources where active.contains(source.area) {
                found += (await source.search(blended, limit: each)).filter { $0.area == source.area && active.contains($0.area) }
            }
            return Self.merge(found, limit: limit, maxChars: maxChars)
        }

        let allowed = await allowedIds(scope, areas: active)
        let any = allowed.values.contains { !$0.isEmpty }
        switch scope.mode {
        case .only:
            guard any else { return [] }
            let found = await search(trimmed, previous: prev, blended: blended, areas: active, limit: each, allowed: allowed, floorFromAll: false)
            return Self.merge(found, limit: limit, maxChars: maxChars)
        case .first:
            var found = await search(trimmed, previous: prev, blended: blended, areas: active, limit: each, allowed: nil, floorFromAll: false)
            guard any else { return Self.merge(found, limit: limit, maxChars: maxChars) }
            found += await search(trimmed, previous: prev, blended: blended, areas: active, limit: each, allowed: allowed, floorFromAll: true)
            return Self.mergeFirst(found, inside: allowed, limit: limit, maxChars: maxChars)
        }
    }

    /// Kennungen der Elemente je Bereich, die der Scope in den wirksamen Bereichen umfasst und die im Index oder in einer Quelle stehen.
    private func allowedIds(_ scope: AssistantScope, areas active: Set<AssistantArea>) async -> [AssistantArea: Set<String>] {
        var out: [AssistantArea: Set<String>] = [:]
        for area in AssistantArea.allCases where active.contains(area) {
            out[area] = Set(await items(in: area).filter { scope.admits($0) }.map(\.id))
        }
        return out
    }

    /// Ein Abruf über Index und Quellen; mit `allowed` nur in diesen Elementen (der Filter steht vor der Begrenzung).
    private func search(_ question: String, previous: String?, blended: String, areas active: Set<AssistantArea>, limit: Int,
                        allowed: [AssistantArea: Set<String>]?, floorFromAll: Bool) async -> [AssistantPassage] {
        var entries: Set<AssistantScope.Entry>?
        if let allowed {
            entries = Set(allowed.flatMap { area, ids in ids.map { AssistantScope.Entry(area: area, itemId: $0) } })
        }
        var found = await corpus.search(question, previous: previous, areas: active, limit: limit, only: entries, floorFromAll: floorFromAll)
        for source in sources where active.contains(source.area) {
            let ids = allowed.map { $0[source.area] ?? [] }
            if let ids, ids.isEmpty { continue }
            let passages = await source.search(blended, limit: limit, only: ids)
            found += passages.filter { $0.area == source.area && active.contains($0.area) && (ids?.contains($0.itemId) ?? true) }
        }
        return found
    }

    /// Normalisiert je Bereich, ordnet (Punktzahl, dann Bereichsreihenfolge, dann Eingangsreihenfolge), begrenzt je Element,
    /// Anzahl und Zeichen.
    static func merge(_ passages: [AssistantPassage], limit: Int, maxChars: Int) -> [AssistantPassage] {
        var picker = Picker(maxChars: maxChars)
        var out: [AssistantPassage] = []
        for p in rank(passages) {
            guard out.count < limit, picker.remaining > 0 else { break }
            if let taken = picker.offer(p) { out.append(taken) }
        }
        return out
    }

    /// Modus „zuerst“: zuerst die Treffer aus den Kontexten (`inside`: Kennungen je Bereich), danach die beste Stelle außerhalb, danach
    /// die übrigen Treffer in der Reihenfolge der Punktzahl, ohne Doppelte. Die beste Stelle außerhalb bleibt immer erhalten, wenn es
    /// eine gibt: Sie wird zuerst eingeplant (auch ihre Zeichen), die Kontexte belegen höchstens `limit − 1` Plätze und den Rest des
    /// Zeichenbudgets. Ohne Treffer aus den Kontexten ist das Ergebnis genau das von `merge`.
    static func mergeFirst(_ passages: [AssistantPassage], inside: [AssistantArea: Set<String>], limit: Int, maxChars: Int) -> [AssistantPassage] {
        let ranked = rank(passages)
        func inScope(_ p: AssistantPassage) -> Bool { inside[p.area]?.contains(p.itemId) ?? false }
        guard ranked.contains(where: inScope) else { return merge(passages, limit: limit, maxChars: maxChars) }

        var picker = Picker(maxChars: maxChars)
        let best = ranked.first { !inScope($0) }.flatMap { picker.offer($0) }
        let slots = max(limit - (best == nil ? 0 : 1), 0)
        var first: [AssistantPassage] = []
        for p in ranked where inScope(p) {
            guard first.count < slots, picker.remaining > 0 else { break }
            if let taken = picker.offer(p) { first.append(taken) }
        }
        var rest: [AssistantPassage] = []
        let used = first.count + (best == nil ? 0 : 1)
        for p in ranked {
            guard used + rest.count < limit, picker.remaining > 0 else { break }
            if let taken = picker.offer(p) { rest.append(taken) }
        }
        return first + (best.map { [$0] } ?? []) + rest
    }

    /// Normalisierte Punktzahl je Bereich (beste Stelle des Bereichs = 1), geordnet: Punktzahl, Bereichsreihenfolge, Eingang.
    private static func rank(_ passages: [AssistantPassage]) -> [AssistantPassage] {
        var top: [AssistantArea: Double] = [:]
        for p in passages { top[p.area] = max(top[p.area] ?? 0, p.score) }
        let normalized: [(order: Int, p: AssistantPassage)] = passages.enumerated().map { i, p in
            let t = top[p.area] ?? 0
            let score = t > 0 ? max(0, p.score) / t : 0.5
            return (i, p.replacing(text: p.text, score: score))
        }
        return normalized.sorted { a, b in
            if a.p.score != b.p.score { return a.p.score > b.p.score }
            if a.p.area != b.p.area { return a.p.area.order < b.p.area.order }
            return a.order < b.order
        }.map(\.p)
    }

    /// Nimmt Auszüge in der angebotenen Reihenfolge auf: höchstens `perItemCap` je Element, keine Doppelten, im Zeichenbudget (ein zu
    /// langer Auszug wird gekürzt, wenn mindestens `minClipChars` bleiben, sonst weggelassen).
    private struct Picker {
        var remaining: Int
        private var perItem: [String: Int] = [:]
        private var seen = Set<String>()

        init(maxChars: Int) { remaining = maxChars }

        mutating func offer(_ p: AssistantPassage) -> AssistantPassage? {
            let key = "\(p.area.rawValue)/\(p.itemId)"
            let dedupe = "\(key)/\(p.locator ?? "")/\(p.text.prefix(80))"
            guard !seen.contains(dedupe), perItem[key, default: 0] < AssistantKnowledge.perItemCap else { return nil }
            var text = p.text
            if text.count > remaining {
                guard remaining >= AssistantKnowledge.minClipChars else { return nil }
                text = BookTalkPrompt.clip(text, to: remaining)
            }
            seen.insert(dedupe)
            perItem[key, default: 0] += 1
            remaining -= text.count
            return p.replacing(text: text)
        }
    }

    // MARK: Fragearten

    private static let inventoryNouns: Set<String> = [
        "buch", "bucher", "buchern", "transkript", "transkripte", "transkripten", "besprechung", "besprechungen",
        "aufnahme", "aufnahmen", "aufgenommen", "notiz", "notizen", "dokument", "dokumente", "dokumenten", "datei", "dateien",
        "ablage", "bibliothek", "sprachnotiz", "sprachnotizen", "gespeichert", "vorhanden", "hörbuch", "hörbücher", "horbuch", "horbucher",
    ]

    private static let inventoryFiller: Set<String> = [
        "habe", "haben", "hast", "zuletzt", "neueste", "neuesten", "neuester", "neuestes", "letzte", "letzten", "letzter", "letztes",
        "liste", "aufzahlen", "zeig", "zeige", "nenn", "nenne", "gesamt", "insgesamt", "anzahl", "meine", "meinen", "meiner", "meinem",
        "eigentlich", "grad", "gerade", "zuerst", "kurzlich", "neulich", "derzeit", "aktuell", "aktuelle", "aktuellen", "kennst",
        "kennen", "lesen", "gelesen", "angefangen", "begonnen", "wieviele", "gibt", "welche", "welchen", "viele", "sage", "sag",
        "alles", "alle", "uberhaupt", "noch", "schon", "bisher", "habt", "hatte", "hatten", "app", "inhalte", "inhalten", "lokal",
        "ubersicht", "uberblick", "bestand", "folgende", "uns", "mir", "kurz",
    ]

    /// Reine Bestandsfragen („welche Bücher habe ich“, „wie viele Transkripte“, „was habe ich zuletzt aufgenommen“): kein
    /// Abruf, der Bestand im Prompt beantwortet sie. Bleibt ein inhaltliches Wort übrig, ist es keine.
    static func isInventoryQuestion(_ question: String) -> Bool {
        let words = BookTalkTokens.foldedWords(question)
        guard words.contains(where: { inventoryNouns.contains($0) }) else { return false }
        return BookTalkTokens.contentWords(question).allSatisfy { inventoryNouns.contains($0) || inventoryFiller.contains($0) }
    }

    private static let pronouns: Set<String> = [
        "das", "dieses", "dieser", "diese", "diesem", "dazu", "davon", "darin", "daran", "darauf", "damit", "dabei", "daraus",
        "dafur", "daruber", "dort", "er", "sie", "ihn", "ihm",
    ]

    /// Kurze Anschlussfrage: höchstens 4 Inhaltswörter, oder mit Pronomen („das“, „dazu“ …) bis 8 Inhaltswörter.
    static func isFollowUp(_ question: String) -> Bool {
        let content = BookTalkTokens.contentWords(question).count
        if content <= 4 { return true }
        return content <= 8 && BookTalkTokens.foldedWords(question).contains { pronouns.contains($0) }
    }
}
