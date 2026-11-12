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

    /// Höchstens so viele Auszüge je Element im Gesamtergebnis (wie im Index).
    static let perItemCap = 3
    /// Ein Auszug unter dieser Länge wird nicht mehr gekürzt aufgenommen, sondern weggelassen.
    static let minClipChars = 200

    public init(corpus: AssistantCorpus, sources: [any AssistantSource] = [], areas: Set<AssistantArea> = Set(AssistantArea.allCases)) {
        self.corpus = corpus
        self.sources = sources
        self.areas = areas
    }

    /// Baut den Bestand neu (Klasse B): ersetzt den Index durch genau diese Dokumente.
    public func install(_ documents: [AssistantDocument]) async {
        await corpus.install(documents)
    }

    /// Dasselbe Wissen (Index und Quellen) mit anderen Anfangsbereichen: ein neues Objekt, nie eine Änderung dieses.
    public nonisolated func withAreas(_ areas: Set<AssistantArea>) -> AssistantKnowledge {
        AssistantKnowledge(corpus: corpus, sources: sources, areas: areas)
    }

    /// Der Anfangswert der Bereiche, ohne Aktor-Sprung (Anfangsstand einer Sitzung ohne Zustandsdatei).
    public nonisolated var currentAreas: Set<AssistantArea> { areas }

    // MARK: Bestand

    /// Höchstens so viele Titel für die Prüfung „nimmt ein Satz Bezug auf einen Titel aus dem Bestand“.
    static let titleCap = 400

    /// Titel aller Elemente der freigegebenen Bereiche, ohne Doppelte (für die Eigendatenprüfung von Frage und Satz).
    public func inventoryTitles(areas: Set<AssistantArea>? = nil) async -> [String] {
        let active = areas ?? self.areas
        var seen = Set<String>()
        var out: [String] = []
        for area in AssistantArea.allCases where active.contains(area) {
            for item in await items(in: area) {
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
        let active = areas ?? self.areas
        var out: [(area: AssistantArea, statements: [String])] = []
        for area in AssistantArea.allCases where active.contains(area) && (wanted?.contains(area) ?? true) {
            out.append((area, Self.statements(area: area, items: await items(in: area))))
        }
        return out
    }

    public func inventoryStatements(question: String) async -> [String] {
        await inventoryGroups(question: question).flatMap(\.statements)
    }

    /// Abruf für die Prüfung eines Satzes ohne Quelle: der SATZ ist die Anfrage, über alle freigegebenen Bereiche, ohne Vorfrage
    /// und ohne die Sonderbehandlung von Bestandsfragen, mit großzügiger Obergrenze. Nur zum Verwerfen, nie zum Zitieren.
    public func probe(sentence: String, areas: Set<AssistantArea>? = nil) async -> [AssistantPassage] {
        let active = areas ?? self.areas
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
    public func retrieve(question: String, previous: String?, limit: Int, maxChars: Int,
                         areas: Set<AssistantArea>? = nil) async -> [AssistantPassage] {
        let active = areas ?? self.areas
        guard limit > 0, maxChars > 0, !active.isEmpty else { return [] }
        let trimmed = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !Self.isInventoryQuestion(trimmed) else { return [] }
        let prev = Self.isFollowUp(trimmed) ? previous?.trimmingCharacters(in: .whitespacesAndNewlines) : nil
        let blended = [trimmed, prev].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
        let each = limit * 2

        var found = await corpus.search(trimmed, previous: prev, areas: active, limit: each)
        for source in sources where active.contains(source.area) {
            found += (await source.search(blended, limit: each)).filter { $0.area == source.area && active.contains($0.area) }
        }
        return Self.merge(found, limit: limit, maxChars: maxChars)
    }

    /// Normalisiert je Bereich, ordnet (Punktzahl, dann Bereichsreihenfolge, dann Eingangsreihenfolge), begrenzt je Element,
    /// Anzahl und Zeichen.
    static func merge(_ passages: [AssistantPassage], limit: Int, maxChars: Int) -> [AssistantPassage] {
        var top: [AssistantArea: Double] = [:]
        for p in passages { top[p.area] = max(top[p.area] ?? 0, p.score) }
        let normalized: [(order: Int, p: AssistantPassage)] = passages.enumerated().map { i, p in
            let t = top[p.area] ?? 0
            let score = t > 0 ? max(0, p.score) / t : 0.5
            return (i, p.replacing(text: p.text, score: score))
        }
        let ranked = normalized.sorted { a, b in
            if a.p.score != b.p.score { return a.p.score > b.p.score }
            if a.p.area != b.p.area { return a.p.area.order < b.p.area.order }
            return a.order < b.order
        }
        var perItem: [String: Int] = [:]
        var seen = Set<String>()
        var remaining = maxChars
        var out: [AssistantPassage] = []
        for (_, p) in ranked {
            guard out.count < limit, remaining > 0 else { break }
            let key = "\(p.area.rawValue)/\(p.itemId)"
            let dedupe = "\(key)/\(p.locator ?? "")/\(p.text.prefix(80))"
            guard !seen.contains(dedupe), perItem[key, default: 0] < perItemCap else { continue }
            var text = p.text
            if text.count > remaining {
                guard remaining >= minClipChars else { continue }
                text = BookTalkPrompt.clip(text, to: remaining)
            }
            seen.insert(dedupe)
            perItem[key, default: 0] += 1
            remaining -= text.count
            out.append(p.replacing(text: text))
        }
        return out
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
